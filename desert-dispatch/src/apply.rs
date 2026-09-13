//! The write path: turn the `[Dispatch]` levers into edits of the parsed
//! records, and turn them back off again.
//!
//! # What this writes, and what it does not
//!
//! Five addresses, and nothing else in the process:
//!
//! | Address | Type | Lever |
//! | --- | --- | --- |
//! | `steps[1] + 0x08` | `u32` | `Speed` - the mission length, in tenths of an hour |
//! | `entry + 0xDA` | `u16` | `NoSkillRequirement` - `0xFFFF`, **never** `0` |
//! | `entry + 0xC4` | `u32` | `AnyOperatorCount` - `1`, **never** `0` |
//! | `dropset entry + 0x20` | `i64` | `Rewards` - the amount minimum |
//! | `dropset entry + 0x28` | `i64` | `Rewards` - the amount maximum |
//!
//! Every one of them is a field of a **parsed static-info record**. Those
//! records are re-parsed from disk on every launch and are **never serialised**
//! (`docs/reference-internals.md` section 20.18), so removing the plugin
//! restores vanilla with nothing to undo. No code is patched, no hook is
//! installed, no game function is called, and nothing here ever touches save
//! data. The one thing that outlives a revert is not written by this file at
//! all: a mission that *completes* while `Rewards` is above 1 has its inflated
//! payout percent banked in the save by the game itself, and that percent still
//! pays out later. The section notice and the README say so; this module cannot
//! do anything about it beyond not being surprising.
//!
//! Two fields deliberately **not** written, both two bytes from one that is:
//!
//! * `entry+0xD8`, the work-stat a mission pays a bonus for. Clearing it
//!   alongside `+0xDA` silently deletes a reward nobody asked to lose, with
//!   nothing in the UI to show it happened.
//! * `entry+0x10` and the dropset weights. Editing a weight would break the
//!   `row+0x48` weight-sum invariant this module *asserts* before every reward
//!   write - the invariant is only free because nothing here touches a weight.
//!
//! # The shape, and why it is the gatherer's
//!
//! `desert_gatherer::hook::reapply` is the model, and the resemblance is not
//! decoration. Three rules come with it:
//!
//! 1. **Vanilla is remembered on first sight and never re-derived.** What is
//!    written is always `f(vanilla)`, never `f(what is in the field now)`, so a
//!    pass is idempotent and a revert is exact. [`crate::remember`] enforces it.
//! 2. **A pass that cannot identify what it is looking at does not write.**
//!    Every skip has a counter, and the counters are the log line that says a
//!    game update moved something. A refusal costs a mission its multiplier; a
//!    wrong write costs a table.
//! 3. **Reverting is applying with the levers at vanilla.** There is no second
//!    code path, so a revert cannot rot while the apply path is maintained.
//!
//! # Every guard, and the write it stands in front of
//!
//! * The record slot must be non-null and [`node::plausible`]; the operation
//!   array pointer and count likewise.
//! * The operation entry's own `0x120` bytes must read in one block.
//! * The **operation key** at `+0x60` is the identity. `record+0x08` is not a
//!   key - it is the low half of a heap pointer that changes between passes -
//!   and a table keyed on it would be worthless (section 20.19).
//! * On a **first sight** the values read are the vanilla ones, so they have to
//!   look like it: the duration inside `node::DURATION_LO..=DURATION_HI`, the
//!   operator count inside `OPERATORS_LO..=OPERATORS_HI`, the amounts inside
//!   `AMOUNT_LO..=AMOUNT_HI` with `min <= max`. Out of band means the offset is
//!   not the field, so nothing is remembered and nothing is written.
//! * The step list must be present, long enough to have a middle step, and
//!   **shaped like a step list**: every step but the middle one reads `0` at
//!   `+0x08`, on all 936 vanilla missions and after any write of ours.
//! * Before any write, what is in the field now must be either the remembered
//!   vanilla or a value this plugin could itself have produced from it
//!   (`config::derived_*`). Anything else is somebody else's memory. That
//!   decision, and every other one that needs no address - the bands, both
//!   `want == 0` locks, the `kind == 13` skip - is [`node::plan_mission`] and
//!   [`node::plan_entry`], so it is unit tested on the dev machine rather than
//!   only reachable through a live game. This file is reads, then a plan, then
//!   the writes and the counters.
//! * A reward row's entry weights must sum to `row+0x48`, exactly, or the row
//!   is skipped whole. That is one loop the pass is already doing.
//! * A reward entry's remembered item row must still be the item row in the
//!   record, and `kind == 13` entries are skipped: they are not item drops.
//! * **One entry object is written at most once per pass.** The remember table
//!   is keyed on `(row, entry index)`, so if two rows ever pointed at the same
//!   entry object the second key would read the first key's already-multiplied
//!   value as its own vanilla and write `v * N * N`, and a revert would then
//!   restore `v * N` for ever. Whether the game ever shares an entry object is
//!   not known - one 0x70 blob is byte-identical under rows 14665 and 14715,
//!   but 79 of its 112 bytes are zero, so identical content is not proof - and
//!   a per-pass set of entry **addresses** makes the question moot instead of
//!   answering it. The `rawdropentry` dump line carries the address so a future
//!   capture can settle it outright.
//! * The two halves of an amount pair are written in the order that cannot be
//!   observed inverted ([`node::amount_write_order`]), and the second half is
//!   not written at all if the first was refused.
//! * **A mission's reward rows only enter the acted-on set once the mission has
//!   been identified.** [`apply_mission`] hands its `+0xA8` / `+0xAA` back only
//!   after the key has been accepted and the bands have passed; an entry read at
//!   a wrong stride therefore cannot put a row in front of the reward pass at
//!   all, rather than relying on the weight-sum check over there to catch it.
//! * **The row set is capped where rows are inserted**, by
//!   [`node::insert_capped`]. It only ever grows, so a read-time `take(N)` would
//!   let a smaller row index arriving later evict an already-multiplied larger
//!   one into a window no pass visits and no revert reaches. Rows refused by the
//!   cap are counted ([`MissionCounts::rows_over_cap`]) rather than dropped
//!   silently.
//!
//! Every foreign read and write goes through `desert_core::safe`, which reads
//! and writes our own process through `ReadProcessMemory`/`WriteProcessMemory`,
//! so an unmapped page is a `false` and a counted skip rather than a fault.

use std::collections::BTreeSet;

use crate::config::Levers;
use crate::gate::Outcome;
use crate::node::parsed::dropset;
use crate::node::{self, parsed};
use crate::remember::{self, Mission, Reward};
use crate::{manager, safe};
use crate::scan::{dropset_manager, Budget};

/// What a pass is doing, for the tag at the front of its log lines.
///
/// Three words for one code path: the levers decide which. `[revert]` is not a
/// different pass, it is this pass with [`Levers::VANILLA`], and saying so in
/// the log is what makes a revert legible in a session's worth of lines.
fn tag(lev: &Levers, dry: bool) -> &'static str {
    if dry {
        "[dry]"
    } else if lev.is_vanilla() {
        "[revert]"
    } else {
        "[apply]"
    }
}

/// `Speed=4 Rewards=1 ...`, or `back to vanilla` when every lever is off.
fn what(lev: &Levers) -> String {
    if lev.is_vanilla() {
        "back to vanilla".to_string()
    } else {
        lev.summary()
    }
}

/// `written` / `would be written`, so a `DryRun` line cannot be misread as a
/// record of something that happened.
fn verb(dry: bool) -> &'static str {
    if dry {
        "would be written"
    } else {
        "written"
    }
}

/// The three things every write in this module needs and none of them owns.
struct Ctx<'a> {
    lev: &'a Levers,
    dry: bool,
    debug: bool,
    /// The same per-pass line budget `MaxLines` sets for the diagnostic dump:
    /// the per-change lines below are lines a pass writes, so they are counted
    /// against it like every other kind.
    budget: &'a mut Budget,
}

impl Ctx<'_> {
    /// Write one scalar, unless this is a dry run. `true` means the field now
    /// holds `v` (or would).
    fn put<T: Copy>(&self, at: usize, v: T) -> bool {
        self.dry || safe::write(at, v)
    }

    /// Log one per-change line, if `Debug` or `DryRun` asked for them and the
    /// budget still has room.
    ///
    /// The line arrives as a **closure**, not a `&str`: a `format!` argument
    /// would be built on every changed field of every pass and then dropped
    /// unread under the default `Debug=0 DryRun=0`, or once the budget is spent,
    /// which is the allocation `Budget` exists to keep out of a pass. Nothing is
    /// formatted unless the line is going to be written.
    fn note(&mut self, line: impl FnOnce() -> String, dry_tag: &'static str) {
        if (self.debug || self.dry) && self.budget.take() {
            crate::log!("{dry_tag} {}", line());
        }
    }
}

// ---------------------------------------------------------------------------
// Missions
// ---------------------------------------------------------------------------

/// What one mission pass did. Every skip counter is a place where the parsed
/// layout stopped matching what was remembered, which is the first thing a
/// game update breaks and the first thing to read when a number looks wrong.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissionCounts {
    /// Operation entries walked. Larger than the number of distinct missions:
    /// operations are shared across node handles, so a vanilla table walks
    /// 2808 entries carrying 936 distinct keys.
    pub visited: usize,
    /// Distinct operation keys remembered, over the whole session.
    pub known: usize,
    /// Entries at least one field was written for (would have been, in DryRun).
    pub changed: usize,
    /// Entries already carrying exactly the wanted values.
    pub unchanged: usize,
    /// Entries nothing was written for and nothing *could* be: the three
    /// identity reasons below, plus an entry every single field of which was
    /// refused.
    ///
    /// That last case used to land on [`Self::unchanged`], which made a table
    /// this plugin could not write one byte of report "936 unchanged" - the one
    /// number that says nothing is wrong. An entry is only unchanged when
    /// nothing needed writing, never when everything was refused.
    pub skipped: usize,
    /// Individual scalars written (would have been, in DryRun).
    pub fields: usize,
    /// Fields skipped inside entries that were otherwise fine: the sum of the
    /// four reasons after those three.
    pub fields_skipped: usize,

    /// A guarded read of a record, an entry or a field failed.
    pub skip_read: usize,
    /// The operation key was one [`crate::remember`] cannot store.
    pub skip_key: usize,
    /// The remember table is full, so there is no vanilla to write back later.
    pub skip_no_room: usize,

    /// The step list is missing, too short, or not shaped like a step list.
    pub skip_steps: usize,
    /// A first-sight value was outside the band a vanilla one lives in, so the
    /// offset is not the field.
    pub skip_range: usize,
    /// What is in the field now is neither vanilla nor anything this plugin
    /// could have written from it.
    pub skip_foreign: usize,
    /// The write itself was refused, having read the same address a moment ago.
    pub skip_write: usize,
    /// Reward rows an identified mission named that did not fit inside
    /// [`node::MAX_DROP_ROWS`], counting repeats.
    ///
    /// The cap is applied where the row is inserted
    /// ([`node::insert_capped`]), so the rows this pass hands back are a
    /// **stable** set rather than the smallest 4096 of a growing one. A row
    /// refused here is never written to and so never needs reverting; a row
    /// evicted from a read-time window would have been both.
    pub rows_over_cap: usize,
    /// The pass stopped early on [`node::MAX_TOTAL_OPS`].
    pub stopped_early: bool,
}

impl MissionCounts {
    /// What [`crate::gate`] needs out of a finished pass.
    ///
    /// `clean` names the two **transient** failures and only those. A refused
    /// `safe::write` leaves the field at its previous value - which on a revert
    /// is still non-vanilla - and the early stop on [`node::MAX_TOTAL_OPS`]
    /// leaves every entry past the cap untouched; both can succeed on the next
    /// tick, so neither may be memoised as a finished pass.
    ///
    /// `skip_range` and `skip_foreign` deliberately do **not** count. Those say
    /// the offset is not the field, or the field is somebody else's - persistent
    /// conditions that re-walking the table every two seconds cannot change.
    /// The pass's warning line is what reports them.
    pub fn outcome(&self) -> Outcome {
        Outcome { clean: self.skip_write == 0 && !self.stopped_early, wrote: self.fields }
    }

    /// The one summary line a pass logs.
    pub fn summary(&self, lev: &Levers, dry: bool) -> String {
        format!(
            "{} missions {}: {} entries visited ({} missions known), {} changed, {} unchanged, \
             {} skipped; {} fields {}",
            tag(lev, dry),
            what(lev),
            self.visited,
            self.known,
            self.changed,
            self.unchanged,
            self.skipped,
            self.fields,
            verb(dry)
        )
    }

    /// The reasons anything was skipped, or `None` when nothing was. In a
    /// healthy session every mission is identified, in band and written, so
    /// this line does not appear at all.
    pub fn warning(&self) -> Option<String> {
        if self.skipped == 0
            && self.fields_skipped == 0
            && self.rows_over_cap == 0
            && !self.stopped_early
        {
            return None;
        }
        let mut why: Vec<String> = Vec::new();
        for (n, name) in [
            (self.skip_read, "unreadable"),
            (self.skip_key, "unusable key"),
            (self.skip_no_room, "remember table full"),
            (self.skip_steps, "step list wrong shape"),
            (self.skip_range, "vanilla value out of band"),
            (self.skip_foreign, "field is not ours to write"),
            (self.skip_write, "write refused"),
            (self.rows_over_cap, "reward rows past the cap"),
        ] {
            if n > 0 {
                why.push(format!("{name} {n}"));
            }
        }
        if self.stopped_early {
            why.push(format!("stopped at MAX_TOTAL_OPS={}", node::MAX_TOTAL_OPS));
        }
        Some(format!(
            "{} of {} mission entries and {} fields were left alone ({}); the parsed record \
             layout may have moved in this game build",
            self.skipped,
            self.visited,
            self.fields_skipped,
            why.join(", ")
        ))
    }
}

/// `(step list, readable step count)` for one operation entry, or `None` when
/// the list is absent or implausible.
fn step_list(entry: usize) -> Option<(usize, usize)> {
    let ptr = safe::read_ptr(entry + parsed::OP_STEPS)?;
    let n = safe::read::<u32>(entry + parsed::OP_STEP_COUNT)?;
    if !node::plausible(ptr) {
        return None;
    }
    Some((ptr, (n as usize).min(node::MAX_STEPS)))
}

/// The address of the mission's duration, once the step list has been checked
/// for the shape a step list has.
///
/// Three steps per entry on all 936 vanilla missions - goto, working, leave -
/// and only the middle one carries a number at `+0x08`. Steps 0 and 2 reading
/// `0` is therefore a free identity check on the list, and it survives our own
/// write: shortening the middle step does not put a number on the other two.
fn duration_at(entry: usize) -> Option<usize> {
    let (ptr, n) = step_list(entry)?;
    if n <= parsed::DURATION_STEP {
        return None;
    }
    for i in 0..n {
        let at = ptr + i * parsed::STEP_STRIDE + parsed::STEP_DURATION;
        let v = safe::read::<u32>(at)?;
        if i != parsed::DURATION_STEP && v != 0 {
            // Some other step carries a duration. That is not the list this
            // code knows how to edit.
            return None;
        }
    }
    Some(ptr + parsed::DURATION_STEP * parsed::STEP_STRIDE + parsed::STEP_DURATION)
}

/// The vanilla values for `key`, remembering this reading if it is the first
/// sight of it.
///
/// A first sight is the only moment the band checks apply, because it is the
/// only moment the values being read are vanilla: on every later pass the
/// duration may already be a quarter of what it was, which is the point.
fn mission_vanilla(now: Mission, c: &mut MissionCounts) -> Option<Mission> {
    if let Some(v) = remember::missions().get(now.key) {
        return Some(v);
    }
    if !node::vanilla_mission_in_band(now) {
        c.skipped += 1;
        c.skip_range += 1;
        return None;
    }
    match remember::missions().remember(now) {
        Some(v) => Some(v),
        None => {
            c.skipped += 1;
            c.skip_no_room += 1;
            None
        }
    }
}

/// Apply the three mission levers to one operation entry.
///
/// Returns the entry's two `dropsetinfo` row indices, **and only for an entry
/// this pass managed to identify**: one whose key [`crate::remember`] accepted
/// and whose first-sight values were inside the bands a vanilla record's are.
/// `None` for everything else, including an entry that was identified long ago
/// but would not read today.
///
/// That return value is the whole of the reward pass's input, and the gate on it
/// is not tidiness. The caller used to harvest both rows from every entry
/// `decode_operation` merely succeeded on, **before** a single identity, band or
/// step-shape guard had run - so a record read at a wrong stride after a game
/// update would inject rows into the set the reward pass writes to, and the
/// weight-sum and item-row checks over there are all that would have stood
/// between that and a write into the game-wide drop table.
fn apply_mission(
    entry: usize,
    raw: &[u8],
    ctx: &mut Ctx,
    c: &mut MissionCounts,
) -> Option<(u16, u16)> {
    let Some(f) = node::decode_operation(raw) else {
        c.skipped += 1;
        c.skip_read += 1;
        return None;
    };
    if f.key == remember::FREE {
        c.skipped += 1;
        c.skip_key += 1;
        return None;
    }
    // The duration is the one lever whose field is not inside these bytes, and
    // an entry whose step list will not read cannot be remembered at all: there
    // would be no vanilla duration to put back later.
    let Some(dur_at) = duration_at(entry) else {
        c.skipped += 1;
        c.skip_steps += 1;
        return None;
    };
    let Some(dur_now) = safe::read::<u32>(dur_at) else {
        c.skipped += 1;
        c.skip_read += 1;
        return None;
    };

    let now = Mission {
        key: f.key,
        duration_tenths: dur_now,
        skill_req: f.skill_req,
        min_operators: f.min_operators,
    };
    let van = mission_vanilla(now, c)?;
    // Identified from here on: the key is one the remember table holds and the
    // vanilla values behind it are this entry's own. The rows are safe to hand
    // back whatever the three writes below do.
    let rows = (f.reward1, f.reward2);

    // What the levers ask for, decided by [`node::plan_mission`] over plain
    // values: every band check, every `derived_*` predicate and both `want == 0`
    // guards are in there, which is what makes them natively unit tested. What
    // is left here is the part that needs a live address - the writes - and the
    // counters.
    let plan = node::plan_mission(van, now, ctx.lev);
    // Fields the plan refused, as opposed to fields that needed nothing.
    // `wrote == 0` alone cannot tell those apart, and reporting a wholly refused
    // entry as "unchanged" hides the one thing the counters exist to show.
    let mut refused = usize::from(plan.refused);
    c.fields_skipped += usize::from(plan.refused);
    c.skip_foreign += usize::from(plan.refused);

    // The per-change detail is built only when something will read it: `note`
    // drops the line unless `Debug` or `DryRun` asked for one, and a `format!`
    // evaluated in order to be thrown away is exactly the cost `Budget` exists
    // to keep out of a pass.
    let verbose = ctx.debug || ctx.dry;
    let mut wrote = 0usize;
    let mut detail = String::new();

    // 1. Duration: steps[1] + 0x08, u32, tenths of an hour.
    if let Some(want) = plan.duration {
        if ctx.put(dur_at, want) {
            wrote += 1;
            if verbose {
                detail.push_str(&format!(" dur {dur_now}->{want}"));
            }
        } else {
            c.fields_skipped += 1;
            refused += 1;
            c.skip_write += 1;
        }
    }

    // 2. Required skill: entry + 0xDA, u16, 0xFFFF to clear. `entry + 0xD8`,
    //    two bytes below it, is the reward-bonus stat and is never touched.
    if let Some(want) = plan.skill {
        if ctx.put(entry + parsed::OP_SKILL_REQ, want) {
            wrote += 1;
            if verbose {
                detail.push_str(&format!(
                    " skill {}->{}",
                    node::row_text(f.skill_req),
                    node::row_text(want)
                ));
            }
        } else {
            c.fields_skipped += 1;
            refused += 1;
            c.skip_write += 1;
        }
    }

    // 3. Minimum operators: entry + 0xC4, u32, 1 to open up.
    if let Some(want) = plan.min_operators {
        if ctx.put(entry + parsed::OP_MIN_OPERATORS, want) {
            wrote += 1;
            if verbose {
                detail.push_str(&format!(" minops {}->{}", f.min_operators, want));
            }
        } else {
            c.fields_skipped += 1;
            refused += 1;
            c.skip_write += 1;
        }
    }

    if wrote == 0 {
        // Nothing written, and the two reasons for that are not the same thing:
        // every field already said what it should (unchanged), or every field
        // that did not was refused (skipped). Counting the second as the first
        // is how a table nothing could be written to reported "936 unchanged".
        if refused > 0 {
            c.skipped += 1;
        } else {
            c.unchanged += 1;
        }
        return Some(rows);
    }
    c.changed += 1;
    c.fields += wrote;
    let t = tag(ctx.lev, ctx.dry);
    ctx.note(|| format!("op key={}{detail}", f.key), t);
    Some(rows)
}

/// Walk every loaded `FactionNode` record and apply the mission levers to
/// every operation entry in it, returning what happened and the set of
/// `dropsetinfo` rows those missions name.
///
/// **The row set comes from here and nowhere else.** `dropsetinfo` is the
/// game-wide drop table: a reward multiplier that took its rows from anywhere
/// but an operation entry's `+0xA8` / `+0xAA` would silently become a global
/// loot mod (`docs/reference-internals.md` section 20.17).
pub fn missions(
    records: usize,
    count: usize,
    lev: &Levers,
    dry: bool,
    debug: bool,
    budget: &mut Budget,
) -> (MissionCounts, BTreeSet<u16>) {
    let mut c = MissionCounts::default();
    let mut rows: BTreeSet<u16> = BTreeSet::new();
    let mut ctx = Ctx { lev, dry, debug, budget };

    for idx in 0..count.min(node::MAX_RECORDS) {
        let obj = match manager::slot(records, idx) {
            manager::Slot::Loaded(p) => p,
            // A null slot is the normal reading for a record the game has not
            // parsed yet, not a failure. It is not counted as a skip because
            // there is nothing there to skip.
            manager::Slot::Empty => continue,
            manager::Slot::Unreadable => {
                c.skip_read += 1;
                continue;
            }
        };
        let Some(op_count) = safe::read::<u32>(obj + parsed::REC_OP_COUNT) else {
            c.skip_read += 1;
            continue;
        };
        let ops_ptr = safe::read_ptr(obj + parsed::REC_OPS).unwrap_or(0);
        if op_count == 0 || ops_ptr == 0 {
            continue;
        }
        if !node::plausible(ops_ptr) {
            c.skip_read += 1;
            continue;
        }
        for i in 0..(op_count as usize).min(node::MAX_OPS) {
            // The bound that actually holds: the per-record and per-table caps
            // multiply out to a million entries, and one misread `u32` would
            // otherwise wedge this thread inside the game's process.
            if c.visited >= node::MAX_TOTAL_OPS {
                c.stopped_early = true;
                c.known = remember::missions().len();
                return (c, rows);
            }
            c.visited += 1;
            let entry = ops_ptr + i * parsed::OP_STRIDE;
            let mut raw = [0u8; parsed::OP_STRIDE];
            if !safe::read_into(entry, &mut raw) {
                c.skipped += 1;
                c.skip_read += 1;
                continue;
            }
            // The rows come back from `apply_mission` and only for an entry it
            // identified, which is why there is no second `decode_operation`
            // here any more: this loop used to harvest both row indices from
            // every entry that merely decoded, ahead of every guard.
            if let Some((r1, r2)) = apply_mission(entry, &raw, &mut ctx, &mut c) {
                for row in [r1, r2] {
                    if row != parsed::REWARD_NONE && !node::insert_capped(&mut rows, row) {
                        c.rows_over_cap += 1;
                    }
                }
            }
        }
    }
    c.known = remember::missions().len();
    (c, rows)
}

// ---------------------------------------------------------------------------
// Reward amounts
// ---------------------------------------------------------------------------

/// What one reward pass did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewardCounts {
    /// Rows the missions named.
    pub rows_requested: usize,
    /// Of those, rows that were loaded and walked whole.
    pub rows_walked: usize,
    /// Rows the game has not parsed yet. Normal: dropset rows load on demand.
    pub rows_null: usize,
    /// Rows nothing was attempted on: out of range, unreadable, an entry that
    /// would not read, or a weight sum that did not match.
    pub rows_skipped: usize,
    /// Entries inside the walked rows.
    pub visited: usize,
    /// Entries both amounts were written for (would have been, in DryRun).
    pub changed: usize,
    /// Entries already carrying the wanted amounts.
    pub unchanged: usize,
    /// `kind == 13` entries. Not a failure: they are not item drops, they all
    /// pay `0`, and their `+0x38` is not an item row.
    pub not_item_drops: usize,
    /// Entries nothing was attempted on.
    pub skipped: usize,
    /// `i64` amounts written (would have been, in DryRun).
    pub fields: usize,

    /// The row's entry weights did not sum to `row+0x48`.
    pub skip_weights: usize,
    /// A guarded read failed somewhere in the row.
    pub skip_read: usize,
    /// The row declared an entry count no reward row has
    /// ([`node::drop_entry_count`]), or an entry index too wide for the
    /// remember table's key.
    ///
    /// This is the counter a game update moves, and until `read_row` learned to
    /// say *why* it failed it could never move at all: a moved count read as
    /// "unreadable" and an unmapped page read as "unreadable" were one number.
    pub skip_shape: usize,
    /// The row index is past the end of the `dropsetinfo` table. Kept apart from
    /// [`Self::skip_shape`]: that one says a row's own count is wrong, this one
    /// says `entry+0xA8` is not a row index.
    pub skip_row_index: usize,
    /// The remember table is full.
    pub skip_no_room: usize,
    /// The entry's item row is not the one that was remembered for it.
    pub skip_item: usize,
    /// A first-sight amount was outside the band a vanilla one lives in.
    pub skip_range: usize,
    /// What is in the field now is not something this plugin could have
    /// written from the remembered vanilla.
    pub skip_foreign: usize,
    /// The write itself was refused.
    pub skip_write: usize,
    /// An entry object this pass had already written through another row. See
    /// the module header: the remember table is keyed on `(row, entry index)`,
    /// so writing one object twice in a pass would square the multiplier and
    /// poison the vanilla it keeps for the second key.
    pub skip_shared: usize,
}

impl RewardCounts {
    /// What [`crate::gate`] needs out of a finished pass. Same rule as
    /// [`MissionCounts::outcome`]: only a refused write is transient.
    pub fn outcome(&self) -> Outcome {
        Outcome { clean: self.skip_write == 0, wrote: self.fields }
    }

    /// The one summary line a reward pass logs.
    pub fn summary(&self, lev: &Levers, dry: bool) -> String {
        format!(
            "{} rewards {}: {} of {} rows walked ({} not loaded, {} skipped), {} entries, \
             {} changed, {} unchanged, {} not item drops, {} skipped; {} amounts {}",
            tag(lev, dry),
            what(lev),
            self.rows_walked,
            self.rows_requested,
            self.rows_null,
            self.rows_skipped,
            self.visited,
            self.changed,
            self.unchanged,
            self.not_item_drops,
            self.skipped,
            self.fields,
            verb(dry)
        )
    }

    /// The reasons anything was skipped, or `None` when nothing was. A row
    /// that has not loaded yet is not a reason: those are expected, and the
    /// next pass picks them up.
    pub fn warning(&self) -> Option<String> {
        // `skip_write` gets its own term: half a pair written counts as a
        // change rather than a skip, so it would otherwise be the one failure
        // that never reached the log.
        if self.rows_skipped == 0 && self.skipped == 0 && self.skip_write == 0 {
            return None;
        }
        let mut why: Vec<String> = Vec::new();
        for (n, name) in [
            (self.skip_weights, "weight sum mismatch"),
            (self.skip_read, "unreadable"),
            (self.skip_shape, "impossible entry count"),
            (self.skip_row_index, "row index past the table"),
            (self.skip_no_room, "remember table full"),
            (self.skip_item, "item row mismatch"),
            (self.skip_range, "vanilla amount out of band"),
            (self.skip_foreign, "amount is not ours to write"),
            (self.skip_write, "write refused"),
            (self.skip_shared, "entry shared by another row"),
        ] {
            if n > 0 {
                why.push(format!("{name} {n}"));
            }
        }
        Some(format!(
            "{} of {} reward rows and {} entries were left alone ({}); the parsed dropsetinfo \
             layout may have moved in this game build",
            self.rows_skipped,
            self.rows_requested,
            self.skipped,
            why.join(", ")
        ))
    }
}

/// Why one `dropsetinfo` row could not be read whole.
///
/// Two variants rather than a `None`, and the distinction is the whole point of
/// the type. A row whose declared entry count is outside the band a real reward
/// row's lives in is **the** reading a game update produces, and it used to be
/// charged to the "unreadable" counter alongside a page that had gone away -
/// which left [`RewardCounts::skip_shape`], the counter that exists for exactly
/// this, unable to fire at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowError {
    /// A guarded read failed: the row head, the entry array, or one entry.
    /// Transient as often as not - a page can go away mid-pass.
    Unreadable,
    /// The row declared an entry count no reward row has
    /// ([`node::drop_entry_count`]). Persistent: it says `record+0x30` is not a
    /// count in this build.
    Shape,
}

/// One `dropsetinfo` row read whole: its entry addresses and their decoded
/// contents, or why it could not be.
///
/// All or nothing on purpose. The weight-sum check below is only an invariant
/// if it is summed over *every* entry the row declares, so a row with one
/// unreadable entry is a row that cannot be checked, and a row that cannot be
/// checked is not written to.
fn read_row(obj: usize) -> Result<Vec<(usize, node::DropsetEntry)>, RowError> {
    let declared =
        safe::read::<u32>(obj + dropset::ROW_ENTRY_COUNT).ok_or(RowError::Unreadable)?;
    let declared = node::drop_entry_count(declared).ok_or(RowError::Shape)?;
    let arr = safe::read_ptr(obj + dropset::ROW_ENTRIES).ok_or(RowError::Unreadable)?;
    if !node::plausible(arr) {
        return Err(RowError::Unreadable);
    }
    let mut out = Vec::with_capacity(declared);
    for i in 0..declared {
        // `declared` is capped at DROP_ENTRIES_HI and `arr` well below the top
        // of the address space by `plausible`: the arithmetic cannot wrap.
        let at = safe::read_ptr(arr + i * dropset::ENTRY_PTR_STRIDE).ok_or(RowError::Unreadable)?;
        if !node::plausible(at) {
            return Err(RowError::Unreadable);
        }
        let mut raw = [0u8; dropset::ENTRY_RAW];
        if !safe::read_into(at, &mut raw) {
            return Err(RowError::Unreadable);
        }
        out.push((at, node::decode_entry(i, &raw).ok_or(RowError::Unreadable)?));
    }
    Ok(out)
}

/// The free integrity check: the row's cached weight total at `+0x48` equals
/// the sum of its entries' weights, on 219/219 vanilla rows.
///
/// Nothing here ever edits a weight, so this must still hold after a write
/// pass - which is exactly what makes it assertable. A mismatch says the
/// pointer being walked is not a dropset record, or the row moved under the
/// reader, and it is caught before a byte is written.
fn weights_agree(obj: usize, entries: &[(usize, node::DropsetEntry)]) -> bool {
    let Some(cached) = safe::read::<i64>(obj + dropset::ROW_WEIGHT_SUM) else {
        return false;
    };
    let mut sum: i64 = 0;
    for (_, e) in entries {
        sum = sum.saturating_add(e.weight);
    }
    sum == cached
}

/// The vanilla amounts for one entry, remembering this reading if it is the
/// first sight of it. Same first-sight rule as [`mission_vanilla`].
fn reward_vanilla(now: Reward, c: &mut RewardCounts) -> Option<Reward> {
    if let Some(v) = remember::rewards().get(now.row, now.index) {
        return Some(v);
    }
    if !node::vanilla_reward_in_band(now) {
        c.skipped += 1;
        c.skip_range += 1;
        return None;
    }
    match remember::rewards().remember(now) {
        Some(v) => Some(v),
        None => {
            c.skipped += 1;
            c.skip_no_room += 1;
            None
        }
    }
}

/// Apply the `Rewards` lever to one entry of one row.
fn apply_entry(row: u16, at: usize, e: &node::DropsetEntry, ctx: &mut Ctx, c: &mut RewardCounts) {
    // Not an item drop: 34 of 725 vanilla entries, exactly the ones paying `0`
    // and exactly the ones whose `+0x38` is too wide for an item row. Scaling
    // zero is harmless; reading them as items is not.
    if !e.is_item_drop() {
        c.not_item_drops += 1;
        return;
    }
    let Ok(index) = u16::try_from(e.index) else {
        c.skipped += 1;
        c.skip_shape += 1;
        return;
    };
    let now = Reward {
        row,
        index,
        amount_min: e.amount_min,
        amount_max: e.amount_max,
        item_row: e.item_row,
    };
    let Some(van) = reward_vanilla(now, c) else { return };

    // What the lever asks for, decided by [`node::plan_entry`] over plain
    // values. The item-row check, the `kind == 13` skip and the `derived_amount`
    // guard on both halves are all in there and all natively unit tested; what
    // is left here is the pair of writes and the counters.
    let (want_min, want_max) = match node::plan_entry(van, e, ctx.lev) {
        // Unreachable: the guard above this function's `reward_vanilla` call
        // already returned for a non-item-drop, and it has to stay there - a
        // `kind == 13` entry's `0` amounts are out of band on purpose, so
        // remembering one first would report it as a vanilla value that looks
        // wrong. Counted the same way either way.
        node::RewardPlan::NotAnItemDrop => {
            c.not_item_drops += 1;
            return;
        }
        // The entry is not paying the item it was paying when its vanilla
        // amounts were recorded, so this is not the entry we remembered.
        node::RewardPlan::ItemMismatch => {
            c.skipped += 1;
            c.skip_item += 1;
            return;
        }
        node::RewardPlan::Unchanged => {
            c.unchanged += 1;
            return;
        }
        node::RewardPlan::Refused => {
            c.skipped += 1;
            c.skip_foreign += 1;
            return;
        }
        node::RewardPlan::Write { want_min, want_max } => (want_min, want_max),
    };
    // Two stores, so a game thread rolling a reward between them sees an
    // intermediate pair whatever we do. `node::amount_write_order` picks the
    // order whose intermediate is *wider* than both end states rather than
    // inverted, and the `break` is the other half of the same argument: once the
    // first store has been refused, the second one alone is exactly the
    // inversion the order exists to prevent.
    //
    // `usize::from(write(..)) + ..` / `ok < 2` is the same idiom
    // `desert_gatherer::hook::reapply` uses over its own min/max pair, and it is
    // deliberately not shared: the two loops differ in their order (this one is
    // ordered by `amount_write_order` and stops on a refusal, the gatherer's is
    // unordered and attempts both) and a helper would have to take that
    // difference as a parameter to save two lines.
    let mut ok = 0usize;
    for (off, v) in node::amount_write_order(e.amount_max, want_min, want_max) {
        if !ctx.put(at + off, v) {
            break;
        }
        ok += 1;
    }
    if ok < 2 {
        // The pair was read a moment ago, so a refusal means the page went
        // away between the read and the write. The next pass fixes it: the
        // gate does not memoise a pass that reports one.
        c.skip_write += 1;
    }
    if ok == 0 {
        c.skipped += 1;
        return;
    }
    c.changed += 1;
    c.fields += ok;
    let t = tag(ctx.lev, ctx.dry);
    ctx.note(
        || {
            format!(
                "drop row={row} idx={} item={} amount {}..{} -> {want_min}..{want_max}",
                e.index, e.item_row, e.amount_min, e.amount_max
            )
        },
        t,
    );
}

/// Apply the `Rewards` lever to every entry of every row in `rows`.
///
/// `rows` is the union of the missions' `+0xA8` and `+0xAA`, produced by
/// [`missions`] and by nothing else, so this can never reach a drop table row
/// no dispatch mission pays from.
pub fn rewards(
    slot: usize,
    rows: &[u16],
    lev: &Levers,
    dry: bool,
    debug: bool,
    budget: &mut Budget,
) -> RewardCounts {
    // What this pass will actually visit, not what it was handed: the caller
    // caps `requested` at [`node::MAX_DROP_ROWS`] when it inserts, so these are
    // equal today, and counting the untruncated set would claim rows the pass
    // never looked at.
    let walked = rows.len().min(node::MAX_DROP_ROWS);
    let mut c = RewardCounts { rows_requested: walked, ..RewardCounts::default() };
    // Entry object addresses already written this pass. See the module header:
    // two rows pointing at one entry object would make the second row square
    // the multiplier and remember the result as vanilla.
    let mut written: BTreeSet<usize> = BTreeSet::new();
    let mut ctx = Ctx { lev, dry, debug, budget };
    let Some((count, records)) = dropset_manager(slot) else {
        c.rows_skipped = walked;
        c.skip_read = walked;
        return c;
    };

    for &row in rows.iter().take(node::MAX_DROP_ROWS) {
        let idx = usize::from(row);
        if idx >= count {
            c.rows_skipped += 1;
            c.skip_row_index += 1;
            continue;
        }
        let obj = match manager::slot(records, idx) {
            manager::Slot::Loaded(p) => p,
            manager::Slot::Empty => {
                // Normal: dropset rows are parsed when the game first needs one.
                c.rows_null += 1;
                continue;
            }
            manager::Slot::Unreadable => {
                c.rows_skipped += 1;
                c.skip_read += 1;
                continue;
            }
        };
        let entries = match read_row(obj) {
            Ok(e) => e,
            Err(RowError::Unreadable) => {
                c.rows_skipped += 1;
                c.skip_read += 1;
                continue;
            }
            Err(RowError::Shape) => {
                // The reading a game update gives: the row is there and reads
                // fine, and says it holds a number of entries no reward row
                // holds. Nothing is written to it, and the warning line names
                // this reason specifically.
                c.rows_skipped += 1;
                c.skip_shape += 1;
                continue;
            }
        };
        if !weights_agree(obj, &entries) {
            c.rows_skipped += 1;
            c.skip_weights += 1;
            continue;
        }
        c.rows_walked += 1;
        for (at, e) in &entries {
            c.visited += 1;
            if !written.insert(*at) {
                c.skipped += 1;
                c.skip_shared += 1;
                continue;
            }
            apply_entry(row, *at, e, &mut ctx, &mut c);
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lev(speed: u32, rewards: u32) -> Levers {
        Levers { speed, rewards, clear_skill: false, any_operators: false }
    }

    #[test]
    fn the_tag_says_which_of_the_three_a_pass_is() {
        assert_eq!(tag(&lev(4, 1), false), "[apply]");
        assert_eq!(tag(&Levers::VANILLA, false), "[revert]");
        assert_eq!(tag(&Levers::VANILLA, true), "[dry]");
        assert_eq!(tag(&lev(4, 1), true), "[dry]", "a dry run is never called an apply");
        assert_eq!(what(&Levers::VANILLA), "back to vanilla");
        assert!(what(&lev(4, 3)).starts_with("Speed=4 Rewards=3"));
    }

    #[test]
    fn a_clean_pass_says_nothing_beyond_its_summary() {
        let c = MissionCounts { visited: 2808, known: 936, changed: 936, unchanged: 1872, ..MissionCounts::default() };
        assert_eq!(c.warning(), None);
        let line = c.summary(&lev(4, 1), false);
        assert!(line.starts_with("[apply] missions Speed=4"), "{line}");
        assert!(line.contains("2808 entries visited (936 missions known)"), "{line}");
        assert!(line.ends_with("fields written"), "{line}");
        // A revert names itself, and a dry run never claims a write happened.
        assert!(c.summary(&Levers::VANILLA, false).starts_with("[revert] missions back to vanilla"));
        assert!(c.summary(&lev(4, 1), true).ends_with("fields would be written"));
    }

    #[test]
    fn a_pass_that_skipped_something_says_why() {
        let c = MissionCounts {
            visited: 10,
            skipped: 3,
            skip_key: 1,
            skip_range: 2,
            fields_skipped: 4,
            skip_foreign: 4,
            ..MissionCounts::default()
        };
        let w = c.warning().unwrap_or_default();
        assert!(w.contains("3 of 10 mission entries"), "{w}");
        assert!(w.contains("unusable key 1"), "{w}");
        assert!(w.contains("vanilla value out of band 2"), "{w}");
        assert!(w.contains("field is not ours to write 4"), "{w}");
        assert!(!w.contains("unreadable"), "counters that did not move are not named: {w}");
    }

    #[test]
    fn reward_summaries_read_the_same_way() {
        let c = RewardCounts {
            rows_requested: 219,
            rows_walked: 219,
            visited: 725,
            changed: 691,
            not_item_drops: 34,
            fields: 1382,
            ..RewardCounts::default()
        };
        assert_eq!(c.warning(), None);
        let line = c.summary(&lev(1, 3), false);
        assert!(line.starts_with("[apply] rewards Speed=1 Rewards=3"), "{line}");
        assert!(line.contains("219 of 219 rows walked"), "{line}");
        assert!(line.contains("34 not item drops"), "{line}");
        assert!(line.ends_with("1382 amounts written"), "{line}");
    }

    /// A row whose weights do not add up is a row the pass must not write to,
    /// and the warning has to name that reason specifically: it is the one
    /// that says the pointer being walked is not a dropset record at all.
    #[test]
    fn a_weight_sum_mismatch_is_named_in_the_warning() {
        let c = RewardCounts {
            rows_requested: 219,
            rows_walked: 218,
            rows_skipped: 1,
            skip_weights: 1,
            ..RewardCounts::default()
        };
        let w = c.warning().unwrap_or_default();
        assert!(w.contains("weight sum mismatch 1"), "{w}");
        assert!(w.contains("1 of 219 reward rows"), "{w}");
    }

    /// Rows that have not loaded yet are the normal reading, not a problem.
    #[test]
    fn unloaded_rows_are_not_a_warning() {
        let c = RewardCounts { rows_requested: 219, rows_walked: 12, rows_null: 207, ..RewardCounts::default() };
        assert_eq!(c.warning(), None);
        assert!(c.summary(&lev(1, 2), false).contains("207 not loaded"));
    }

    /// BLOCKING 7: an entry whose every field was refused is **not**
    /// "unchanged". The two readings differ by exactly the thing the counters
    /// exist to show, and a table nothing could be written to used to report
    /// "936 unchanged" - the one number that says all is well.
    #[test]
    fn a_wholly_refused_entry_is_skipped_and_not_called_unchanged() {
        // What the pass produces for 936 missions it could not write one field
        // of: every entry skipped, and the reason beside it.
        let refused = MissionCounts {
            visited: 2808,
            known: 936,
            skipped: 936,
            fields_skipped: 936,
            skip_foreign: 936,
            ..MissionCounts::default()
        };
        assert_eq!(refused.unchanged, 0, "nothing here was unchanged; it was refused");
        let line = refused.summary(&lev(4, 1), false);
        assert!(line.contains("0 changed, 0 unchanged, 936 skipped"), "{line}");
        let w = refused.warning().unwrap_or_default();
        assert!(w.contains("936 of 2808 mission entries"), "{w}");
        assert!(w.contains("field is not ours to write 936"), "{w}");

        // The honest "nothing needed doing" reading still exists and is still
        // silent: no skips, no refusals, no warning.
        let quiet = MissionCounts {
            visited: 2808,
            known: 936,
            unchanged: 2808,
            ..MissionCounts::default()
        };
        assert_eq!(quiet.warning(), None);
        assert!(quiet.summary(&lev(4, 1), false).contains("2808 unchanged"));
    }

    /// BLOCKING 3: rows the cap refused are counted and named, because the
    /// alternative is a pass that quietly acts on some of a mission's rewards
    /// and not others with nothing in the log to say so.
    #[test]
    fn rows_refused_by_the_cap_are_named_in_the_warning() {
        let c = MissionCounts {
            visited: 2808,
            known: 936,
            changed: 936,
            unchanged: 1872,
            rows_over_cap: 7,
            ..MissionCounts::default()
        };
        let w = c.warning().unwrap_or_default();
        assert!(w.contains("reward rows past the cap 7"), "{w}");
        // It is a warning all on its own: no entry and no field was skipped.
        assert_eq!(c.skipped, 0);
        assert_eq!(c.fields_skipped, 0);
    }

    /// BLOCKING 6: "the entry count moved" and "the row would not read" are two
    /// findings, and `skip_shape` is the one a game update produces. It used to
    /// be unreachable, because `read_row` answered `None` to both questions.
    #[test]
    fn a_moved_entry_count_lands_on_its_own_counter() {
        let c = RewardCounts {
            rows_requested: 219,
            rows_walked: 0,
            rows_skipped: 219,
            skip_shape: 219,
            ..RewardCounts::default()
        };
        let w = c.warning().unwrap_or_default();
        assert!(w.contains("impossible entry count 219"), "{w}");
        assert!(!w.contains("unreadable"), "a moved count is not an unreadable row: {w}");
        // And the other way round: a page that went away is still "unreadable".
        let c = RewardCounts {
            rows_requested: 219,
            rows_skipped: 1,
            skip_read: 1,
            ..RewardCounts::default()
        };
        let w = c.warning().unwrap_or_default();
        assert!(w.contains("unreadable 1"), "{w}");
        assert!(!w.contains("impossible entry count"), "{w}");
    }

    /// The band `read_row` enforces is [`node::drop_entry_count`]'s, and it is
    /// narrower than the dump's read cap on purpose - so a row the dump prints
    /// can be a row the write path refuses, and the refusal is now audible.
    #[test]
    fn the_write_path_bands_entry_counts_more_tightly_than_the_dump_caps_them() {
        assert_eq!(node::drop_entry_count(60), Some(60));
        assert_eq!(node::drop_entry_count(0), None);
        assert_eq!(node::drop_entry_count(node::DROP_ENTRIES_HI as u32 + 1), None);
        const { assert!(node::DROP_ENTRIES_HI < node::MAX_DROP_ENTRIES) };
    }

    /// A row index past the end of the table is not the same finding as a row
    /// whose own entry count is wrong, and it gets its own word.
    #[test]
    fn a_row_index_past_the_table_is_its_own_reason() {
        let c = RewardCounts {
            rows_requested: 219,
            rows_skipped: 2,
            skip_row_index: 2,
            ..RewardCounts::default()
        };
        let w = c.warning().unwrap_or_default();
        assert!(w.contains("row index past the table 2"), "{w}");
        assert!(!w.contains("impossible entry count"), "{w}");
    }

    /// BLOCKING 1: an entry object reached through two rows is written once.
    /// The remember table is keyed on `(row, entry index)`, so the second key
    /// would read the first key's already-multiplied amounts as its own vanilla
    /// and write `v * N * N`, and a revert would then put `v * N` back for ever.
    /// The per-pass set of entry addresses makes that impossible whether or not
    /// the game ever shares an object.
    #[test]
    fn one_entry_address_is_acted_on_once_per_pass() {
        let mut written: BTreeSet<usize> = BTreeSet::new();
        // The same object under two different rows, which is the whole hazard.
        assert!(written.insert(0x1234_5000), "first sight of the object");
        assert!(!written.insert(0x1234_5000), "the second row must be refused");
        // A different object in the same pass is unaffected.
        assert!(written.insert(0x1234_5070));
        assert_eq!(written.len(), 2);

        // And the refusal is reported under its own name rather than vanishing.
        let c = RewardCounts {
            rows_requested: 219,
            rows_walked: 219,
            visited: 725,
            changed: 724,
            skipped: 1,
            skip_shared: 1,
            ..RewardCounts::default()
        };
        let w = c.warning().unwrap_or_default();
        assert!(w.contains("entry shared by another row 1"), "{w}");
    }

}
