//! Automatic gathering: pacing, a per-node cooldown list, and feedback on
//! whether a node went away after we sent for it. Runs on the plugin thread;
//! the only thing it hands to the game thread is a `PickupRequest`.

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::events;
use crate::game::{self, Scene, World};
use crate::module::MainModule;
use crate::actors;

struct Done {
    eid: u32,
    name: String,
    sent: Instant,
}

pub struct Gatherer {
    pub auto: bool,
    range: f32,
    unarmed: bool,
    items: bool,
    gear: bool,
    bag_tab: Option<i16>,
    stack_limit: u32,
    /// item key -> item record index, filled lazily (a table scan each).
    item_index_cache: Vec<(u32, u16)>,
    /// Logged once per fill so a full bag does not spam the log.
    bag_full_reported: bool,
    /// The last refusal reason logged at a full bag; a different one (a
    /// different node type in reach) is logged too, a repeat is not.
    bag_full_reason: String,
    /// Record indices whose declared outputs have been logged once, so the
    /// `[yield]` line appears the first time a node type is looked at.
    yield_logged: Vec<u16>,
    interval: Duration,
    cooldown: Duration,
    done: Vec<Done>,
    last_send: Option<Instant>,
    last_review: Option<Instant>,
    last_idle_log: Option<Instant>,
    pub sent: u32,
    pub gathered: u32,
    /// Sends whose target was still standing when the cooldown ran out.
    consecutive_failures: u32,
}

/// A parked request older than this means the sweep hook is not firing.
const STALE_REQUEST: Duration = Duration::from_secs(2);
/// How often auto mode says "nothing in range" while idle.
const IDLE_LOG_EVERY: Duration = Duration::from_secs(30);
/// While nodes are outstanding, check on them this often so the "gone after"
/// time is a measurement rather than the send interval.
const REVIEW_EVERY: Duration = Duration::from_millis(100);
/// This many expired sends in a row means the game is refusing pickups
/// (a full bag is the usual reason); auto mode stops rather than loop.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

impl Gatherer {
    pub fn new(cfg: &Config) -> Self {
        Gatherer {
            auto: cfg.auto_gather,
            range: cfg.gather_range,
            unarmed: cfg.gather_unarmed,
            items: cfg.gather_items,
            gear: cfg.gather_gear,
            bag_tab: cfg.bag_tab,
            stack_limit: cfg.stack_limit,
            item_index_cache: Vec::new(),
            bag_full_reported: false,
            bag_full_reason: String::new(),
            yield_logged: Vec::new(),
            interval: Duration::from_millis(cfg.gather_interval_ms as u64),
            cooldown: Duration::from_millis(cfg.node_cooldown_ms as u64),
            done: Vec::new(),
            last_send: None,
            last_review: None,
            last_idle_log: None,
            sent: 0,
            gathered: 0,
            consecutive_failures: 0,
        }
    }

    pub fn toggle(&mut self) -> bool {
        self.auto = !self.auto;
        self.auto
    }

    fn item_index(&mut self, m: &MainModule, key: u32) -> Option<u16> {
        if let Some((_, i)) = self.item_index_cache.iter().find(|(k, _)| *k == key) {
            return Some(*i);
        }
        let i = crate::tables::item_index_by_key(m, key)?;
        self.item_index_cache.push((key, i));
        Some(i)
    }

    /// What a node can pay out, as `(item key, count)` per distinct item.
    ///
    /// Preferred source is the constructed gimmick record, which declares every
    /// output block up front; a gather pays out **one** block, so per item the
    /// largest `max` is the margin the bag has to have. Falls back to what was
    /// learned by watching a pickup (`events::yield_of`) when the record is
    /// unreadable, and takes the larger of the two when both are known.
    fn node_yields(&mut self, m: &MainModule, target: &game::GatherTarget) -> Result<Vec<(u32, u32)>, String> {
        let drops = crate::tables::gimmick_record(m, target.record)
            .and_then(crate::tables::gimmick_record_drops);
        let Some(drops) = drops else {
            return match events::yield_of(target.record) {
                Some(pair) => Ok(vec![pair]),
                None => Err("yield unknown: record outputs unreadable and not learned yet".into()),
            };
        };
        if !self.yield_logged.contains(&target.record) {
            self.yield_logged.push(target.record);
            let list: Vec<String> = drops
                .iter()
                .map(|d| format!("item {} x{}-{}", d.item, d.min, d.max))
                .collect();
            crate::log!(
                "[yield] record {} {} declares {} outputs: {}",
                target.record, target.name, drops.len(), list.join(", ")
            );
        }
        let mut out: Vec<(u32, u32)> = Vec::new();
        for d in &drops {
            let count = d.max.min(u32::MAX as u64) as u32;
            match out.iter_mut().find(|(item, _)| *item == d.item) {
                Some(e) => e.1 = e.1.max(count),
                None => out.push((d.item, count)),
            }
        }
        // A learned pair refines the margin; one the record does not list is
        // kept as well, so a disagreement makes the rule stricter, not looser.
        if let Some((item, count)) = events::yield_of(target.record) {
            match out.iter_mut().find(|(i, _)| *i == item) {
                Some(e) => e.1 = e.1.max(count),
                None => out.push((item, count)),
            }
        }
        if out.is_empty() {
            return Err("yield unknown: record outputs unreadable and not learned yet".into());
        }
        Ok(out)
    }

    /// The game's own interaction UI refuses when the bag is full, but the
    /// forged event bypasses that UI and the gather path does not check on
    /// the server side (133/132 was observed). So we check first. A full bag
    /// still accepts a pickup that stacks onto an existing stack, which the
    /// game allows: we take that only when the node's yields are known
    /// ([`Self::node_yields`]) and *every* item it can pay out is already in
    /// the bag as a real stack (count >= 2 proves it stacks) whose result
    /// stays under `StackLimit`.
    fn bag_has_room(&mut self, m: &MainModule, sc: &Scene, target: &game::GatherTarget, why: &str) -> bool {
        let Some(tabs) = sc.tabs.as_ref() else {
            if !self.bag_full_reported {
                crate::log!("[gather] {why}: inventory unreadable; sending without a bag check");
            }
            return true;
        };
        let Some(bag) = actors::bag_tab(tabs, self.bag_tab) else {
            if !self.bag_full_reported {
                crate::log!("[gather] {why}: no bag tab among [{}]; sending without a bag check", game::tabs_summary(tabs));
            }
            return true;
        };
        if bag.free() > 0 {
            if self.bag_full_reported {
                crate::log!("[gather] bag has room again ({}/{} in tab {})", bag.used, bag.max, bag.id);
                self.bag_full_reported = false;
            }
            return true;
        }
        // Full: can it stack?
        let verdict: Result<String, String> = (|| {
            if target.mode != crate::payload::PickupMode::Gather {
                return Err("ground items need a free slot (their contents are per instance)".into());
            }
            let yields = self.node_yields(m, target)?;
            let slots = actors::tab_slots(&bag).ok_or("bag slots unreadable")?;
            let mut notes: Vec<String> = Vec::new();
            for (item, count) in yields {
                let idx = self.item_index(m, item).ok_or_else(|| format!("item {item} not in the item table"))?;
                let stack = slots
                    .iter()
                    .filter(|s| s.item_index == idx)
                    .max_by_key(|s| s.count)
                    .ok_or_else(|| format!("no stack of item {item} in the bag"))?;
                if stack.count < 2 {
                    return Err(format!("item {item} is in the bag as a single, not proven stackable"));
                }
                let after = stack.count + count as i64;
                if after > self.stack_limit as i64 {
                    return Err(format!("stack of item {item} is {} and +{count} would pass StackLimit {}", stack.count, self.stack_limit));
                }
                notes.push(format!("stacks onto item {item} ({} -> {after})", stack.count));
            }
            Ok(notes.join(", "))
        })();
        match verdict {
            Ok(note) => {
                crate::log!("[gather] {why}: bag full ({}/{}) but {note}", bag.used, bag.max);
                true
            }
            Err(reason) => {
                if !self.bag_full_reported || self.bag_full_reason != reason {
                    self.bag_full_reported = true;
                    crate::log!(
                        "[gather] {why}: bag full ({}/{} in tab {}): {} {reason}; not sending",
                        bag.used, bag.max, bag.id, target.name
                    );
                    self.bag_full_reason = reason;
                }
                false
            }
        }
    }

    fn ready_to_send(&self) -> Result<(), &'static str> {
        if events::descriptor().is_none() {
            return Err("descriptor not resolved");
        }
        if events::game_thread_id() == 0 {
            return Err("sweep hook has not fired yet");
        }
        if events::has_pending() {
            return Err("previous request still pending");
        }
        Ok(())
    }

    /// Drop a request the hook never drained, so auto mode does not stall.
    fn reap_stale(&mut self) {
        if let Some(r) = events::drop_stale(STALE_REQUEST) {
            crate::log!(
                "[gather] request for eid={:08X} dropped: sweep hook did not drain it in {:?} (sweep calls {})",
                r.target_eid, STALE_REQUEST, events::sweep_calls()
            );
            self.done.retain(|d| d.eid != r.target_eid);
        }
    }

    /// Look at every node we sent for: gone or no longer `Gather` counts as
    /// gathered; still there after the cooldown becomes eligible again.
    fn review(&mut self, m: &MainModule, sc: &Scene) {
        let mut i = 0;
        while i < self.done.len() {
            let d = &self.done[i];
            let age = d.sent.elapsed();
            let still_gather = sc.actor(d.eid).is_some_and(|a| {
                matches!(actors::classify(m, a, sc.player), actors::Kind::Gather | actors::Kind::Unarmed | actors::Kind::Item)
            });
            if !still_gather {
                self.gathered += 1;
                self.consecutive_failures = 0;
                crate::log!(
                    "[gather] {} eid={:08X} gone after {:.1} s (gathered {} of {} sent)",
                    d.name, d.eid, age.as_secs_f64(), self.gathered, self.sent
                );
                self.done.swap_remove(i);
            } else if events::is_owned(d.eid) {
                self.done.swap_remove(i);
            } else if age > self.cooldown {
                self.consecutive_failures += 1;
                crate::log!(
                    "[gather] {} eid={:08X} still there after {:.1} s; eligible again ({} failed in a row)",
                    d.name, d.eid, age.as_secs_f64(), self.consecutive_failures
                );
                self.done.swap_remove(i);
                if self.auto && self.consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    self.auto = false;
                    self.consecutive_failures = 0;
                    crate::log!(
                        "[gather] {MAX_CONSECUTIVE_FAILURES} pickups in a row were refused: bag full? auto-gather OFF; press KeyToggle to resume"
                    );
                }
            } else {
                i += 1;
            }
        }
    }

    fn send(&mut self, target: &game::GatherTarget, why: &str) {
        let req = events::PickupRequest {
            target_eid: target.eid,
            record: target.record,
            mode: target.mode,
            target_actor: target.actor,
            player_actor: target.player_actor,
            player_eid: target.player_eid,
            route: target.route,
            flag: 0,
        };
        if events::request(req) {
            self.sent += 1;
            self.last_send = Some(Instant::now());
            self.done.push(Done { eid: target.eid, name: target.name.clone(), sent: Instant::now() });
            crate::log!(
                "[gather] {why}: {} ({}{}) eid={:08X} at {:.1} m -> request #{} parked",
                target.name, target.family, if target.armed { "" } else { ", UNARMED" }, target.eid, target.dist, self.sent
            );
        } else {
            crate::log!("[gather] {why}: a request is still pending; ignored");
        }
    }

    /// F9: one node per press, whatever the auto state.
    pub fn manual(&mut self, m: &MainModule, w: &World) {
        self.reap_stale();
        let sc = match game::scene(m, w) {
            Ok(s) => s,
            Err(e) => {
                crate::log!("[gather] manual: {e}");
                return;
            }
        };
        self.last_review = Some(Instant::now());
        self.review(m, &sc);
        if let Err(e) = self.ready_to_send() {
            crate::log!("[gather] manual: {e}; not sending");
            return;
        }
        let skip = |eid: u32| self.done.iter().any(|d| d.eid == eid) || events::is_owned(eid);
        match game::nearest_gather(m, &sc, self.range, self.unarmed, self.items, self.gear, &skip) {
            Ok(t) => {
                if self.bag_has_room(m, &sc, &t, "manual") {
                    self.send(&t, "manual");
                }
            }
            Err(e) => crate::log!("[gather] manual: {e} (excluding {} on cooldown)", self.done.len()),
        }
    }

    /// Called every loop iteration; does nothing unless auto mode is on and
    /// the interval has elapsed.
    pub fn tick(&mut self, m: &MainModule, w: &World) {
        let send_due = self.auto && !self.last_send.is_some_and(|t| t.elapsed() < self.interval);
        let review_due = !self.done.is_empty() && !self.last_review.is_some_and(|t| t.elapsed() < REVIEW_EVERY);
        if !send_due && !review_due {
            return;
        }
        self.reap_stale();
        if send_due && self.ready_to_send().is_err() && !review_due {
            return;
        }
        let Ok(sc) = game::scene(m, w) else { return };
        self.last_review = Some(Instant::now());
        self.review(m, &sc);
        if !send_due || self.ready_to_send().is_err() {
            return;
        }
        let skip = |eid: u32| self.done.iter().any(|d| d.eid == eid) || events::is_owned(eid);
        match game::nearest_gather(m, &sc, self.range, self.unarmed, self.items, self.gear, &skip) {
            Ok(t) => {
                if self.bag_has_room(m, &sc, &t, "auto") {
                    self.send(&t, "auto");
                } else {
                    // Keep the cadence; the "room again" line shows when it clears.
                    self.last_send = Some(Instant::now());
                }
            }
            Err(_) => {
                // Nothing eligible: keep the cadence but stay quiet.
                self.last_send = Some(Instant::now());
                if self.last_idle_log.map_or(true, |t| t.elapsed() > IDLE_LOG_EVERY) && !self.done.is_empty() {
                    self.last_idle_log = Some(Instant::now());
                    crate::log!("[gather] auto: nothing eligible in {:.1} m ({} on cooldown)", self.range, self.done.len());
                }
            }
        }
    }
}
