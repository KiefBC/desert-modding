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
    interval: Duration,
    cooldown: Duration,
    done: Vec<Done>,
    last_send: Option<Instant>,
    last_review: Option<Instant>,
    last_idle_log: Option<Instant>,
    pub sent: u32,
    pub gathered: u32,
}

/// A parked request older than this means the sweep hook is not firing.
const STALE_REQUEST: Duration = Duration::from_secs(2);
/// How often auto mode says "nothing in range" while idle.
const IDLE_LOG_EVERY: Duration = Duration::from_secs(30);
/// While nodes are outstanding, check on them this often so the "gone after"
/// time is a measurement rather than the send interval.
const REVIEW_EVERY: Duration = Duration::from_millis(100);

impl Gatherer {
    pub fn new(cfg: &Config) -> Self {
        Gatherer {
            auto: cfg.auto_gather,
            range: cfg.gather_range,
            interval: Duration::from_millis(cfg.gather_interval_ms as u64),
            cooldown: Duration::from_millis(cfg.node_cooldown_ms as u64),
            done: Vec::new(),
            last_send: None,
            last_review: None,
            last_idle_log: None,
            sent: 0,
            gathered: 0,
        }
    }

    pub fn toggle(&mut self) -> bool {
        self.auto = !self.auto;
        self.auto
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
            let still_gather = sc
                .actor(d.eid)
                .is_some_and(|a| actors::classify(m, a, sc.player) == actors::Kind::Gather);
            if !still_gather {
                self.gathered += 1;
                crate::log!(
                    "[gather] {} eid={:08X} gone after {:.1} s (gathered {} of {} sent)",
                    d.name, d.eid, age.as_secs_f64(), self.gathered, self.sent
                );
                self.done.swap_remove(i);
            } else if age > self.cooldown {
                crate::log!(
                    "[gather] {} eid={:08X} still there after {:.1} s; eligible again",
                    d.name, d.eid, age.as_secs_f64()
                );
                self.done.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn send(&mut self, target: &game::GatherTarget, why: &str) {
        let req = events::PickupRequest {
            target_eid: target.eid,
            player_eid: target.player_eid,
            route: target.route,
            flag: 0,
        };
        if events::request(req) {
            self.sent += 1;
            self.last_send = Some(Instant::now());
            self.done.push(Done { eid: target.eid, name: target.name.clone(), sent: Instant::now() });
            crate::log!(
                "[gather] {why}: {} ({:?}) eid={:08X} at {:.1} m -> request #{} parked",
                target.name, target.family, target.eid, target.dist, self.sent
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
        let skip = |eid: u32| self.done.iter().any(|d| d.eid == eid);
        match game::nearest_gather(m, &sc, self.range, &skip) {
            Ok(t) => self.send(&t, "manual"),
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
        let skip = |eid: u32| self.done.iter().any(|d| d.eid == eid);
        match game::nearest_gather(m, &sc, self.range, &skip) {
            Ok(t) => self.send(&t, "auto"),
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
