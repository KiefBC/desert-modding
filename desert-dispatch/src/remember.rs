//! The vanilla values of every dispatch mission and reward entry this
//! subsystem has ever looked at, kept so that a multiplier changed mid-session
//! - or turned off - has something true to go back to.
//!
//! # Why the write path cannot work without this
//!
//! Both tables this subsystem edits are **parsed static-info records**. The
//! game reads them off disk once, during loading, and never again; nothing
//! here is ever serialised (`docs/reference-internals.md` section 20.18). So
//! the moment the pass writes `vanilla / Speed` into a mission, the vanilla
//! number exists nowhere in the process any more, and dividing the multiplier
//! back out of what is left is **not** the same thing - it is lossy the first
//! time `Speed` does not divide the duration exactly. This table is where the
//! original is kept instead, recorded on the way past, from the same read the
//! pass was about to act on.
//!
//! **Vanilla is recorded the first time a key is seen and never overwritten.**
//! That one rule is what makes the whole write path idempotent by
//! construction: every later pass computes what it wants from the *original*
//! number rather than from whatever the last pass left behind, so applying
//! twice is applying once, and reverting is just another apply with the levers
//! at 1.
//!
//! # Identity, and the trap it avoids
//!
//! `record+0x08` looks like a `FactionNodeKey` and is not one: it is the low
//! half of a heap pointer, 8-aligned on 2808/2808 samples, 2115 distinct
//! values over those 2808 reads, and different on every pass
//! (`docs/reference-internals.md` section 20.19). **A remember table keyed on
//! it is worthless**, and that is not a hypothetical - it is the obvious
//! design, and it is wrong. Missions are keyed on the **operation key** at
//! `entry+0x60` ([`crate::node::parsed::OP_KEY`]): 936 distinct values over
//! the same capture, stable across all three sessions of it. Reward entries
//! are keyed on `(dropset row, entry index)`, which is a position in a table
//! the game indexes directly.
//!
//! One operation key turns up at several addresses, because operations are
//! shared across node handles - 2808 entry reads for 936 distinct keys. That
//! is exactly why the key rather than the address is the identity: every copy
//! is written from the one remembered vanilla, and a second copy that is
//! already carrying the wanted value simply reads as unchanged.
//!
//! # Shape
//!
//! Written by this subsystem's own thread and readable from anywhere, so it is
//! the same shape `desert_gatherer::remember` uses: a fixed open-addressed
//! table of atomics, probed linearly from `key % MAX`, with no allocation
//! after startup, no locks and nothing ever freed. **Deliberately not generic**
//! over that shape: these two tables and the gatherer's share a ~25-line probe
//! and differ in key packing, field sets and atomic types, so a `Table<K, V>`
//! would need a trait over atomic storage to buy back about four lines each. A slot is claimed with one
//! `compare_exchange` on its key; the fields are filled in afterwards and
//! `ready` is stored last, with `Release`, so a reader that sees `ready` with
//! `Acquire` sees the whole slot.
//!
//! The free marker is [`FREE`] (`u32::MAX`) rather than the `0` the gatherer
//! uses, because `(row 0, entry 0)` packs to `0` and is a perfectly ordinary
//! reward entry. A key equal to `FREE` is refused instead, which costs the one
//! identity nothing can name anyway (`0xFFFF` is the "no row" sentinel).
//!
//! Overflow is a **refusal, not a truncation**, and refusing is what keeps the
//! write path safe: a mission or entry that does not fit is never remembered,
//! and [`crate::apply`] never writes to a field whose vanilla it does not
//! hold. The pass counts those refusals itself (`skip_no_room`) and says so
//! once in its warning line.
//!
//! Pure and always compiled, so it is unit tested natively on Linux.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};

/// Slots for missions, keyed by operation key. A vanilla table holds 936; this
/// is comfortable headroom for a game update that adds some, and it keeps the
/// linear probe short.
pub const MAX_MISSIONS: usize = 2_048;
/// Slots for reward entries, keyed by `(dropset row, entry index)`. The 219
/// rows the dispatch missions name hold 725 entries between them.
pub const MAX_ENTRIES: usize = 2_048;

/// The empty-slot marker. Not `0`: `(row 0, entry 0)` is a real reward entry
/// identity and packs to `0`, so the marker has to be a value no real key can
/// take. `0xFFFF` is the "no row" sentinel, so no requested row is `0xFFFF`
/// and no packed key is `FREE`.
pub const FREE: u32 = u32::MAX;

// ---------------------------------------------------------------------------
// Missions
// ---------------------------------------------------------------------------

/// One dispatch mission as it was before this plugin touched it.
///
/// `skill_req` and `min_operators` are stored whether or not they are set:
/// "this mission had no skill requirement" is exactly as much a thing to
/// restore as a requirement is, and `0xFFFF` is what says it
/// ([`crate::node::parsed::SKILL_NONE`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mission {
    /// `entry+0x60`, the FactionOperationKey. The identity, and the only field
    /// of the record that is one.
    pub key: u32,
    /// `steps[1]+0x08`, the mission length in tenths of an hour.
    pub duration_tenths: u32,
    /// `entry+0xDA`, the required `Skill` index, or
    /// [`crate::node::parsed::SKILL_NONE`].
    pub skill_req: u16,
    /// `entry+0xC4`, the minimum operator count.
    pub min_operators: u32,
}

struct MissionSlot {
    key: AtomicU32,
    duration_tenths: AtomicU32,
    skill_req: AtomicU32,
    min_operators: AtomicU32,
    /// Stored last, released; until it is set the slot's fields mean nothing.
    ready: AtomicBool,
}

impl MissionSlot {
    const fn new() -> Self {
        MissionSlot {
            key: AtomicU32::new(FREE),
            duration_tenths: AtomicU32::new(0),
            skill_req: AtomicU32::new(0),
            min_operators: AtomicU32::new(0),
            ready: AtomicBool::new(false),
        }
    }

    /// The slot's contents, or `None` while it is claimed but not yet
    /// published - the state a reader can catch a writer in, half way through
    /// [`Missions::remember`].
    fn read(&self) -> Option<Mission> {
        if !self.ready.load(Ordering::Acquire) {
            return None;
        }
        Some(Mission {
            key: self.key.load(Ordering::Relaxed),
            duration_tenths: self.duration_tenths.load(Ordering::Relaxed),
            // Stored from a `u16`, so the truncation back is exact.
            skill_req: self.skill_req.load(Ordering::Relaxed) as u16,
            min_operators: self.min_operators.load(Ordering::Relaxed),
        })
    }
}

/// The mission table. There is one `static` instance behind [`missions`]; the
/// type is public so the tests can drive a fresh one instead of the
/// process-wide table they cannot reset.
pub struct Missions {
    slots: [MissionSlot; MAX_MISSIONS],
    refused: AtomicU64,
}

impl Missions {
    pub const fn new() -> Self {
        Missions { slots: [const { MissionSlot::new() }; MAX_MISSIONS], refused: AtomicU64::new(0) }
    }

    /// Remember `m` if its key is new, and hand back **the vanilla values that
    /// are remembered for that key**: `m`'s own on a first sight, whatever was
    /// stored before on every sight after that.
    ///
    /// This is the whole contract of the module in one signature. The caller
    /// computes what it wants to write from the returned values, never from
    /// the ones it just read out of the game, so a second pass over a mission
    /// this plugin has already shortened scales the *original* duration rather
    /// than the shortened one.
    ///
    /// `None` means nothing is remembered - an unusable key, or a full table -
    /// and the caller **must not write**: there would be no vanilla to put
    /// back. The caller counts both, as
    /// [`crate::apply::MissionCounts::skip_no_room`]; the `refused` tally in
    /// here is a cross-check the tests read and nothing else does.
    pub fn remember(&self, m: Mission) -> Option<Mission> {
        if m.key == FREE {
            self.refused.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let start = (m.key as usize) % MAX_MISSIONS;
        for i in 0..MAX_MISSIONS {
            // `(start + i) % MAX_MISSIONS` is always in range; a miss would
            // only move on to the next probe, which is what the loop does.
            let Some(slot) = self.slots.get((start + i) % MAX_MISSIONS) else { continue };
            match slot.key.compare_exchange(FREE, m.key, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    slot.duration_tenths.store(m.duration_tenths, Ordering::Relaxed);
                    slot.skill_req.store(u32::from(m.skill_req), Ordering::Relaxed);
                    slot.min_operators.store(m.min_operators, Ordering::Relaxed);
                    // Last, and released: everything above must be visible to
                    // a reader that sees this.
                    slot.ready.store(true, Ordering::Release);
                    return Some(m);
                }
                // Already ours: the stored vanilla wins, always. This is the
                // line that makes re-applying idempotent.
                Err(v) if v == m.key => return slot.read(),
                Err(_) => continue,
            }
        }
        self.refused.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// What is remembered for `key`, or `None` if nothing is.
    ///
    /// The probe stops at the first free slot: nothing is ever removed from
    /// this table, so a free slot along the probe path means the key was never
    /// claimed.
    pub fn get(&self, key: u32) -> Option<Mission> {
        if key == FREE {
            return None;
        }
        let start = (key as usize) % MAX_MISSIONS;
        for i in 0..MAX_MISSIONS {
            let Some(slot) = self.slots.get((start + i) % MAX_MISSIONS) else { continue };
            match slot.key.load(Ordering::Acquire) {
                k if k == key => return slot.read(),
                k if k == FREE => return None,
                _ => continue,
            }
        }
        None
    }

    /// How many missions are remembered and readable right now.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.ready.load(Ordering::Acquire)).count()
    }

    /// True before the first mission has been seen. Kept beside
    /// [`Self::len`], which clippy's `len_without_is_empty` requires.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Missions {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Reward entries
// ---------------------------------------------------------------------------

/// One `dropsetinfo` reward entry as it was before this plugin touched it.
///
/// **Both halves of the amount pair.** `min <= max` holds on 725/725 vanilla
/// entries and the two differ on 136 of them, so a table that remembered only
/// the minimum could not put the other 136 back
/// (`docs/reference-internals.md` section 20.22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reward {
    /// The `dropsetinfo` manager row, straight out of an operation's
    /// [`crate::node::parsed::OP_REWARD_1`] or `_2`.
    pub row: u16,
    /// Position in that row's entry-pointer array.
    pub index: u16,
    /// `entry+0x20`, the fewest of the item a roll pays.
    pub amount_min: i64,
    /// `entry+0x28`, the most of it.
    pub amount_max: i64,
    /// `entry+0x38`, the `iteminfo` row. Not edited - remembered so the pass
    /// can check it has the entry it thinks it has before writing a byte.
    pub item_row: u32,
}

impl Reward {
    /// `(row, index)` packed into the one `u32` the table keys on. `row` is a
    /// `u16` and `index` is capped well inside one, so the pack is lossless
    /// and reversible.
    ///
    /// Private: the packing is this table's business, and the two public ways
    /// in ([`Rewards::remember`] and [`Rewards::get`]) both take the pair.
    fn id(&self) -> u32 {
        (u32::from(self.row) << 16) | u32::from(self.index)
    }
}

struct RewardSlot {
    key: AtomicU32,
    amount_min: AtomicI64,
    amount_max: AtomicI64,
    item_row: AtomicU32,
    ready: AtomicBool,
}

impl RewardSlot {
    const fn new() -> Self {
        RewardSlot {
            key: AtomicU32::new(FREE),
            amount_min: AtomicI64::new(0),
            amount_max: AtomicI64::new(0),
            item_row: AtomicU32::new(0),
            ready: AtomicBool::new(false),
        }
    }

    fn read(&self) -> Option<Reward> {
        if !self.ready.load(Ordering::Acquire) {
            return None;
        }
        let key = self.key.load(Ordering::Relaxed);
        Some(Reward {
            row: (key >> 16) as u16,
            index: (key & 0xFFFF) as u16,
            amount_min: self.amount_min.load(Ordering::Relaxed),
            amount_max: self.amount_max.load(Ordering::Relaxed),
            item_row: self.item_row.load(Ordering::Relaxed),
        })
    }
}

/// The reward-entry table, keyed on `(row, entry index)`. Same shape and same
/// rules as [`Missions`].
pub struct Rewards {
    slots: [RewardSlot; MAX_ENTRIES],
    refused: AtomicU64,
}

impl Rewards {
    pub const fn new() -> Self {
        Rewards { slots: [const { RewardSlot::new() }; MAX_ENTRIES], refused: AtomicU64::new(0) }
    }

    /// Remember `r` if `(row, index)` is new, and hand back the vanilla
    /// amounts remembered for it. See [`Missions::remember`]: the contract,
    /// the refusals and the reason vanilla never gets overwritten are all the
    /// same.
    pub fn remember(&self, r: Reward) -> Option<Reward> {
        let key = r.id();
        if key == FREE {
            self.refused.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        let start = (key as usize) % MAX_ENTRIES;
        for i in 0..MAX_ENTRIES {
            let Some(slot) = self.slots.get((start + i) % MAX_ENTRIES) else { continue };
            match slot.key.compare_exchange(FREE, key, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => {
                    slot.amount_min.store(r.amount_min, Ordering::Relaxed);
                    slot.amount_max.store(r.amount_max, Ordering::Relaxed);
                    slot.item_row.store(r.item_row, Ordering::Relaxed);
                    slot.ready.store(true, Ordering::Release);
                    return Some(r);
                }
                Err(v) if v == key => return slot.read(),
                Err(_) => continue,
            }
        }
        self.refused.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// What is remembered for `(row, index)`, or `None` if nothing is.
    pub fn get(&self, row: u16, index: u16) -> Option<Reward> {
        let key = (u32::from(row) << 16) | u32::from(index);
        if key == FREE {
            return None;
        }
        let start = (key as usize) % MAX_ENTRIES;
        for i in 0..MAX_ENTRIES {
            let Some(slot) = self.slots.get((start + i) % MAX_ENTRIES) else { continue };
            match slot.key.load(Ordering::Acquire) {
                k if k == key => return slot.read(),
                k if k == FREE => return None,
                _ => continue,
            }
        }
        None
    }

    /// How many reward entries are remembered and readable right now.
    ///
    /// Private, unlike [`Missions::len`]: nothing outside this module counts
    /// reward entries - the pass reports what it walked, not what is
    /// remembered - so the only public shape of the count is
    /// [`Self::is_empty`].
    fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.ready.load(Ordering::Acquire)).count()
    }

    /// True before the first reward entry has been read.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Rewards {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// The process-wide tables
// ---------------------------------------------------------------------------

static MISSIONS: Missions = Missions::new();
static REWARDS: Rewards = Rewards::new();

/// The one mission table, written and read by this subsystem's thread.
pub fn missions() -> &'static Missions {
    &MISSIONS
}

/// The one reward-entry table.
pub fn rewards() -> &'static Rewards {
    &REWARDS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Levers;
    use crate::node;

    /// `0xFFFF`, the parsed spelling of "no skill requirement".
    const SKILL_NONE_SPELLING: u16 = crate::node::parsed::SKILL_NONE;

    /// The refusal tally, which no shipped caller reads: the pass counts its
    /// own refusals (`apply::MissionCounts::skip_no_room`), so the field stays
    /// here as the cross-check these tests make of it and has no accessor.
    fn refusals(n: &AtomicU64) -> u64 {
        n.load(Ordering::Relaxed)
    }

    fn m(key: u32, dur: u32, skill: u16, ops: u32) -> Mission {
        Mission { key, duration_tenths: dur, skill_req: skill, min_operators: ops }
    }

    fn r(row: u16, index: u16, min: i64, max: i64, item: u32) -> Reward {
        Reward { row, index, amount_min: min, amount_max: max, item_row: item }
    }

    #[test]
    fn a_mission_goes_in_and_comes_back_out() {
        let t = Missions::new();
        assert!(t.is_empty());
        assert_eq!(t.remember(m(17030001, 160, 61, 5)), Some(m(17030001, 160, 61, 5)));
        assert_eq!(t.len(), 1);
        assert_eq!(t.get(17030001), Some(m(17030001, 160, 61, 5)));
        assert_eq!(t.get(17030002), None, "a key nobody stored");
        assert_eq!(refusals(&t.refused), 0);
    }

    /// The rule the whole write path rests on: a second sight of a key hands
    /// back the **first** reading, not the second. Without it, a pass over a
    /// mission this plugin has already shortened would remember the shortened
    /// duration as vanilla and shorten it again on every pass.
    #[test]
    fn vanilla_is_first_sight_and_is_never_overwritten() {
        let t = Missions::new();
        assert_eq!(t.remember(m(7, 160, 61, 5)), Some(m(7, 160, 61, 5)));
        // What a later pass reads out of the game after the write: 40, no
        // skill requirement, one operator. Every field of it is discarded.
        assert_eq!(t.remember(m(7, 40, 0xFFFF, 1)), Some(m(7, 160, 61, 5)));
        assert_eq!(t.remember(m(7, 10, 0xFFFF, 1)), Some(m(7, 160, 61, 5)));
        assert_eq!(t.len(), 1, "a re-sight must not claim a second slot");
        assert_eq!(t.get(7), Some(m(7, 160, 61, 5)));
    }

    #[test]
    fn many_missions_coexist_including_probe_collisions() {
        let t = Missions::new();
        for n in 0..64u32 {
            // Keys that collide in the probe (same modulo) as well as ones
            // that do not.
            let key = 1 + n * u32::try_from(MAX_MISSIONS).unwrap_or(1);
            assert_eq!(t.remember(m(key, 20 + n, n as u16, n)), Some(m(key, 20 + n, n as u16, n)));
        }
        assert_eq!(t.len(), 64);
        for n in 0..64u32 {
            let key = 1 + n * u32::try_from(MAX_MISSIONS).unwrap_or(1);
            assert_eq!(t.get(key), Some(m(key, 20 + n, n as u16, n)));
        }
    }

    #[test]
    fn a_full_mission_table_refuses_rather_than_evicting() {
        let t = Missions::new();
        for n in 0..MAX_MISSIONS {
            let key = u32::try_from(n).unwrap_or(0);
            assert!(t.remember(m(key, 20, 0xFFFF, 1)).is_some(), "slot {n}");
        }
        assert_eq!(t.len(), MAX_MISSIONS);
        assert_eq!(t.remember(m(0xDEAD_BEEF, 20, 0xFFFF, 1)), None, "no slot left to claim");
        assert_eq!(refusals(&t.refused), 1);
        // Nothing was evicted to make room for it.
        assert_eq!(t.len(), MAX_MISSIONS);
        assert_eq!(t.get(0), Some(m(0, 20, 0xFFFF, 1)));
        // And a key already in the full table still resolves.
        assert_eq!(t.remember(m(5, 999, 0, 0)), Some(m(5, 20, 0xFFFF, 1)));
    }

    #[test]
    fn the_free_marker_is_not_a_key() {
        let t = Missions::new();
        assert_eq!(t.remember(m(FREE, 20, 0xFFFF, 1)), None);
        assert_eq!(refusals(&t.refused), 1);
        assert!(t.is_empty());
        assert_eq!(t.get(FREE), None);
    }

    #[test]
    fn a_claimed_but_unpublished_slot_is_invisible() {
        let t = Missions::new();
        if let Some(slot) = t.slots.first() {
            slot.key.store(1234, Ordering::Relaxed);
        }
        assert_eq!(t.len(), 0, "a claimed but unready slot is not remembered");
        assert!(t.is_empty());
        assert_eq!(t.get(1234), None);
        // The claiming key can still finish.
        assert_eq!(t.remember(m(1234, 80, 0xFFFF, 2)), Some(m(1234, 80, 0xFFFF, 2)));
        assert_eq!(t.get(1234), Some(m(1234, 80, 0xFFFF, 2)));
    }

    #[test]
    fn a_reward_entry_goes_in_and_comes_back_out() {
        let t = Rewards::new();
        assert!(t.is_empty());
        assert_eq!(t.remember(r(6244, 3, 2, 3, 1701)), Some(r(6244, 3, 2, 3, 1701)));
        assert_eq!(t.get(6244, 3), Some(r(6244, 3, 2, 3, 1701)));
        assert_eq!(t.get(6244, 4), None);
        assert_eq!(t.get(6245, 3), None);
    }

    /// `(row 0, entry 0)` packs to `0`, which is why the free marker is not
    /// `0`. A table that used `0` would refuse this entry, or worse, treat the
    /// slot as free and hand its vanilla to somebody else.
    #[test]
    fn row_zero_entry_zero_is_a_real_identity() {
        let t = Rewards::new();
        assert_eq!(r(0, 0, 1, 1, 9).id(), 0);
        assert_eq!(t.remember(r(0, 0, 5, 9, 42)), Some(r(0, 0, 5, 9, 42)));
        assert_eq!(t.len(), 1);
        assert_eq!(t.get(0, 0), Some(r(0, 0, 5, 9, 42)));
    }

    #[test]
    fn reward_vanilla_is_first_sight_and_both_halves_survive() {
        let t = Rewards::new();
        assert_eq!(t.remember(r(100, 1, 2, 3, 77)), Some(r(100, 1, 2, 3, 77)));
        // What a later pass reads after a x3 multiply.
        assert_eq!(t.remember(r(100, 1, 6, 9, 77)), Some(r(100, 1, 2, 3, 77)));
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn reward_ids_do_not_collide_across_rows_and_indices() {
        let t = Rewards::new();
        for row in 0..8u16 {
            for idx in 0..8u16 {
                let e = r(row, idx, i64::from(row) + 1, i64::from(idx) + 1, 1);
                assert_eq!(t.remember(e), Some(e), "row {row} idx {idx}");
            }
        }
        assert_eq!(t.len(), 64);
        for row in 0..8u16 {
            for idx in 0..8u16 {
                assert_eq!(
                    t.get(row, idx),
                    Some(r(row, idx, i64::from(row) + 1, i64::from(idx) + 1, 1))
                );
            }
        }
    }

    #[test]
    fn a_full_reward_table_refuses() {
        let t = Rewards::new();
        for n in 0..MAX_ENTRIES {
            let row = u16::try_from(n / 64).unwrap_or(0);
            let idx = u16::try_from(n % 64).unwrap_or(0);
            assert!(t.remember(r(row, idx, 1, 1, 1)).is_some(), "slot {n}");
        }
        assert_eq!(t.len(), MAX_ENTRIES);
        assert_eq!(t.remember(r(9000, 1, 1, 1, 1)), None);
        assert_eq!(refusals(&t.refused), 1);
    }

    // -----------------------------------------------------------------------
    // The round trip, through the real decision functions.
    //
    // `field` is what would be sitting in the game's memory and each step is
    // what `crate::apply` does to it: remember what is there, ask
    // [`node::plan_mission`] / [`node::plan_entry`] what to write, write it.
    // The plan is the same code the live pass runs, so the `want == 0` locks and
    // the `derived_*` guards are exercised here rather than restated - the
    // helper these tests used to share re-implemented the arithmetic *without*
    // either, which left the whole guard chain untested in this crate.
    // -----------------------------------------------------------------------

    /// One pass over one mission. Returns the new contents of the field and how
    /// many of its three fields the plan refused.
    fn pass(t: &Missions, field: Mission, lev: &Levers) -> (Mission, u8) {
        let van = t.remember(field).unwrap_or(field);
        let plan = node::plan_mission(van, field, lev);
        let after = Mission {
            key: field.key,
            duration_tenths: plan.duration.unwrap_or(field.duration_tenths),
            skill_req: plan.skill.unwrap_or(field.skill_req),
            min_operators: plan.min_operators.unwrap_or(field.min_operators),
        };
        assert_eq!(
            plan.writes() + usize::from(plan.refused),
            [
                after.duration_tenths != field.duration_tenths,
                after.skill_req != field.skill_req,
                after.min_operators != field.min_operators,
            ]
            .iter()
            .filter(|moved| **moved)
            .count()
                + usize::from(plan.refused),
            "a plan that says it wrote N fields moved N fields"
        );
        (after, plan.refused)
    }

    /// The same, for a field whose every value this plugin could have written
    /// itself: the plan must refuse nothing. Every step of every round trip
    /// below is in that position, which is the property being tested - a revert
    /// that a guard refused would be a mission stuck at a shortened duration.
    fn ok(t: &Missions, field: Mission, lev: &Levers) -> Mission {
        let (after, refused) = pass(t, field, lev);
        assert_eq!(refused, 0, "{field:?} under {} was refused", lev.summary());
        after
    }

    #[test]
    fn a_mission_survives_apply_then_revert() {
        let t = Missions::new();
        let on = Levers { speed: 4, rewards: 1, clear_skill: true, any_operators: true };
        let vanilla = m(900, 160, 61, 5);

        // Pass 1: first sight, and every lever lands.
        let field = ok(&t, vanilla, &on);
        assert_eq!(field, m(900, 40, 0xFFFF, 1));
        // Pass 2: the same levers over the already-written field. Idempotent -
        // the duration is not divided a second time, which is exactly what a
        // table that re-derived vanilla from the field would get wrong. The plan
        // asks for nothing at all here, and that is the assertion.
        let (again, refused) = pass(&t, field, &on);
        assert_eq!((again, refused), (field, 0), "a second pass writes nothing");
        // Pass 3: the levers move. Still computed from 160, not from 40.
        let faster = Levers { speed: 2, ..on };
        let field = ok(&t, field, &faster);
        assert_eq!(field, m(900, 80, 0xFFFF, 1));
        // Pass 4: revert. Every field is back to what the game parsed.
        let field = ok(&t, field, &Levers::VANILLA);
        assert_eq!(field, vanilla);
        // And a revert of a revert is a no-op, not a second write.
        assert_eq!(pass(&t, field, &Levers::VANILLA), (vanilla, 0));
        assert_eq!(t.len(), 1, "five passes, one slot");
    }

    /// The two vanilla values the `want == 0` locks nearly made unrevertable: a
    /// mission whose requirement is already "none" spelled as index `0`, and one
    /// whose minimum headcount is `0` (which `node::OPERATORS_LO` says out loud
    /// is a legal thing to find). Neither occurs on build 25116796, but both are
    /// legal, and a guard that refuses the one value a revert needs is worse
    /// than no guard. This is the case the old fixture contained and could not
    /// see, because the test it was in never wrote anything back.
    #[test]
    fn a_vanilla_zero_still_reverts_to_zero() {
        let t = Missions::new();
        let on = Levers { speed: 4, rewards: 1, clear_skill: true, any_operators: true };
        for vanilla in [m(910, 10, 0, 3), m(911, 20, 61, 0), m(912, 4320, 0, 0)] {
            let field = ok(&t, vanilla, &on);
            // Cleared and opened up: 0xFFFF and 1, never 0 in either field.
            assert_eq!(field.skill_req, SKILL_NONE_SPELLING);
            assert_eq!(field.min_operators, 1);
            // Idempotent, re-scaled from the remembered vanilla, then back to
            // the zeros the game parsed - which is the write the guards have to
            // let through.
            assert_eq!(pass(&t, field, &on), (field, 0));
            let field = ok(&t, field, &Levers { speed: 2, ..on });
            assert_eq!(field.duration_tenths, vanilla.duration_tenths / 2);
            let field = ok(&t, field, &Levers::VANILLA);
            assert_eq!(field, vanilla, "a vanilla zero must come back");
        }
    }

    /// A pass that only got half way - one write refused, or the levers turned
    /// off between two fields - still reverts whole, because what is put back
    /// comes from the table rather than from the field beside it.
    #[test]
    fn a_partly_applied_mission_still_reverts_whole() {
        let t = Missions::new();
        let on = Levers { speed: 4, rewards: 1, clear_skill: true, any_operators: true };
        let vanilla = m(901, 240, 57, 8);
        let van = t.remember(vanilla).unwrap_or(vanilla);
        // Only the duration made it: the other two writes were refused.
        let field = Mission { duration_tenths: on.duration(van.duration_tenths), ..vanilla };
        assert_eq!(field, m(901, 60, 57, 8));
        assert_eq!(ok(&t, field, &Levers::VANILLA), vanilla);
        // And the other way round: the duration refused, the two flags landed.
        let field = Mission {
            skill_req: on.skill_req(van.skill_req),
            min_operators: on.min_operators(van.min_operators),
            ..vanilla
        };
        assert_eq!(field, m(901, 240, 0xFFFF, 1));
        assert_eq!(ok(&t, field, &Levers::VANILLA), vanilla);
    }

    /// A value neither vanilla nor anything this plugin could have written from
    /// it is somebody else's memory: the plan refuses the field and the field
    /// keeps whatever it had. Nothing in the crate exercised this before,
    /// because the helper that stood in for the pass had no guards in it.
    #[test]
    fn a_foreign_value_is_refused_rather_than_overwritten() {
        let t = Missions::new();
        let on = Levers { speed: 4, rewards: 1, clear_skill: true, any_operators: true };
        let vanilla = m(902, 160, 61, 5);
        assert_eq!(t.remember(vanilla), Some(vanilla));
        // Longer than vanilla (nothing here lengthens a mission), a skill index
        // that is neither the vanilla one nor "none", and a headcount that is
        // neither the vanilla one nor 1.
        let foreign = m(902, 999, 62, 9);
        let (after, refused) = pass(&t, foreign, &on);
        assert_eq!(refused, 3, "all three fields are foreign");
        assert_eq!(after, foreign, "a refused field is left exactly as it was");
        // A revert refuses them too: the plugin does not own these values and
        // putting "vanilla" into them would be inventing a record.
        let (after, refused) = pass(&t, foreign, &Levers::VANILLA);
        assert_eq!((after, refused), (foreign, 3));
    }

    /// One reward entry's current contents, as `node::plan_entry` reads them.
    /// `kind` is what decides whether the entry is an item drop at all.
    fn as_entry(field: Reward, kind: u8) -> node::DropsetEntry {
        node::DropsetEntry {
            index: usize::from(field.index),
            item_row: field.item_row,
            amount_min: field.amount_min,
            amount_max: field.amount_max,
            weight: 1,
            kind,
            conds: [0xFFFF; 3],
        }
    }

    /// One pass over one reward entry: remember, plan, write both halves or
    /// neither.
    fn reward_pass(t: &Rewards, field: Reward, kind: u8, lev: &Levers) -> (Reward, node::RewardPlan) {
        let van = t.remember(field).unwrap_or(field);
        let plan = node::plan_entry(van, &as_entry(field, kind), lev);
        let after = match plan {
            node::RewardPlan::Write { want_min, want_max } => {
                Reward { amount_min: want_min, amount_max: want_max, ..field }
            }
            _ => field,
        };
        assert!(after.amount_min <= after.amount_max, "the range must not invert: {after:?}");
        (after, plan)
    }

    #[test]
    fn a_reward_entry_survives_apply_then_revert() {
        let t = Rewards::new();
        let x3 = Levers { speed: 1, rewards: 3, clear_skill: false, any_operators: false };
        // One of the 136 entries whose two halves differ - the ones a one-sided
        // multiplier would invert - and one of the 589 where they agree.
        for vanilla in [r(6244, 7, 2, 3, 1701), r(6245, 0, 7, 7, 1702)] {
            let (scaled, plan) = reward_pass(&t, vanilla, 0, &x3);
            assert_eq!(
                plan,
                node::RewardPlan::Write {
                    want_min: vanilla.amount_min * 3,
                    want_max: vanilla.amount_max * 3
                }
            );
            // Idempotent: the second pass is told there is nothing to do, not
            // handed 18..27.
            let (again, plan) = reward_pass(&t, scaled, 0, &x3);
            assert_eq!((again, plan), (scaled, node::RewardPlan::Unchanged));
            // Re-scaled from the remembered vanilla, not from what is there.
            let (bigger, _) = reward_pass(&t, scaled, 0, &Levers { rewards: 10, ..x3 });
            assert_eq!(
                (bigger.amount_min, bigger.amount_max),
                (vanilla.amount_min * 10, vanilla.amount_max * 10),
                "x10 of vanilla, not x10 of x3"
            );
            // And back, exactly.
            let (reverted, _) = reward_pass(&t, bigger, 0, &Levers::VANILLA);
            assert_eq!(reverted, vanilla);
            assert_eq!(reward_pass(&t, reverted, 0, &Levers::VANILLA).1, node::RewardPlan::Unchanged);
        }
    }

    /// `kind == 13` is not an item drop: 34 of 725 vanilla entries, all paying
    /// `0`, whose `+0x38` is not an item row at all. No lever touches them.
    #[test]
    fn a_kind_13_entry_is_never_scaled() {
        let t = Rewards::new();
        let x3 = Levers { speed: 1, rewards: 3, clear_skill: false, any_operators: false };
        let vanilla = r(6244, 9, 0, 0, 2_000_000);
        for lev in [x3, Levers::VANILLA] {
            let (after, plan) = reward_pass(&t, vanilla, 13, &lev);
            assert_eq!(plan, node::RewardPlan::NotAnItemDrop);
            assert_eq!(after, vanilla, "a non-item-drop is left alone");
        }
    }

    /// An entry that has stopped paying the item it was paying is not the entry
    /// that was remembered, whatever its amounts say.
    #[test]
    fn an_entry_that_changed_item_is_not_written() {
        let t = Rewards::new();
        let x3 = Levers { speed: 1, rewards: 3, clear_skill: false, any_operators: false };
        let vanilla = r(6244, 11, 2, 3, 1701);
        assert_eq!(t.remember(vanilla), Some(vanilla));
        let moved = Reward { item_row: 1702, ..vanilla };
        let (after, plan) = reward_pass(&t, moved, 0, &x3);
        assert_eq!(plan, node::RewardPlan::ItemMismatch);
        assert_eq!(after, moved);
    }

    /// An amount that is not a whole multiple of the remembered vanilla is not
    /// ours, so neither half is written - which is what keeps the pair ordered.
    #[test]
    fn a_foreign_amount_is_refused_for_both_halves() {
        let t = Rewards::new();
        let x3 = Levers { speed: 1, rewards: 3, clear_skill: false, any_operators: false };
        let vanilla = r(6244, 13, 2, 3, 1701);
        assert_eq!(t.remember(vanilla), Some(vanilla));
        // 5..9 is ordered and in band, and is not 2..3 times anything: the
        // minimum is not a whole multiple of the remembered 2.
        let foreign = Reward { amount_min: 5, amount_max: 9, ..vanilla };
        let (after, plan) = reward_pass(&t, foreign, 0, &x3);
        assert_eq!(plan, node::RewardPlan::Refused);
        assert_eq!(after, foreign, "refused means left exactly as found");
        assert_eq!(
            reward_pass(&t, Reward { amount_max: 7, ..vanilla }, 0, &Levers::VANILLA).1,
            node::RewardPlan::Refused,
            "a revert refuses a foreign amount too"
        );
    }

    #[test]
    fn the_static_tables_are_the_same_tables() {
        assert_eq!(missions().remember(m(0xC0FFEE, 240, 0xFFFF, 3)).map(|v| v.duration_tenths), Some(240));
        assert_eq!(missions().get(0xC0FFEE).map(|v| v.duration_tenths), Some(240));
        assert!(!missions().is_empty());
        assert_eq!(rewards().remember(r(0xBEEF, 2, 4, 7, 3)).map(|v| v.amount_max), Some(7));
        assert_eq!(rewards().get(0xBEEF, 2).map(|v| v.amount_min), Some(4));
    }
}
