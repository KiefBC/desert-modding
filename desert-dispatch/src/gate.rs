//! The gate in front of an apply pass: has anything moved since the last one,
//! and did that last one actually finish?
//!
//! # Why this is not just "compare the levers"
//!
//! The rule from `CLAUDE.md` is that a subsystem does nothing when nothing it
//! owns has moved, so a `[Looter]` slider drag does not cost a full pass a
//! second. That memo is cheap to get wrong in a way that leaves the game
//! permanently non-vanilla while the ini says vanilla, and two ways of getting
//! it wrong were shipped here before this module existed:
//!
//! 1. **`DryRun` was not part of the key.** A dry pass computes everything and
//!    writes nothing, so memoising it as "the table now holds these levers" is
//!    a lie. `Speed=4` (real), `DryRun=1`, `Speed=1` (dry - memo now records
//!    vanilla, nothing written), `DryRun=0` (memo matches, early return) leaves
//!    every mission at a quarter length for the rest of the session, and
//!    `Enabled=0` cannot rescue it because that also collapses to
//!    [`Levers::VANILLA`], which the memo already holds. `dry` is therefore
//!    part of the key: a repeated dry tick still early-returns, and flipping
//!    `DryRun` **either way** forces exactly one real pass.
//! 2. **A pass that refused a write was memoised as if it had succeeded.** A
//!    `safe::write` refusal leaves the field at its previous, non-vanilla value
//!    and the memo then says vanilla, so every later tick early-returns and
//!    that mission never comes back. Same for a pass that stopped early on
//!    `node::MAX_TOTAL_OPS`. [`Gate::record`] takes an [`Outcome`] and
//!    **clears** the memo on anything but a clean pass, so the next tick runs
//!    again whatever the levers do in the meantime.
//!
//! Not every skip clears the memo, and the distinction is deliberate.
//! `skip_range` (a first-sight value outside the band a vanilla one lives in)
//! and `skip_foreign` (a field holding something this plugin could not have
//! written) are **persistent** conditions: the offset is not the field, or
//! somebody else owns it. Re-walking the whole table every two seconds would
//! not change either answer, and the pass already says so in its warning line.
//! `skip_write` and the early stop are **transient** - a page that went away, a
//! table that had grown past a sanity cap - and those are exactly the ones a
//! later pass can fix.
//!
//! # `dirty`, and what a revert is owed
//!
//! The gate also remembers whether this plugin has written anything
//! non-vanilla that a revert would have to undo. That is not the same question
//! as "is the memo non-vanilla": the memo is cleared by an unclean pass, and an
//! unclean pass is precisely one that may have left half its writes in place.
//! [`Gate::dirty`] stays true from the first field a real, non-vanilla pass
//! writes until a **clean** vanilla pass has put everything back, and it is
//! what lets the caller run a revert without waiting for the record table to
//! settle: a revert writes remembered vanilla, which is safe on a half-filled
//! table by construction.
//!
//! Pure and always compiled, so it is unit tested natively on Linux.

use crate::config::Levers;

/// What a finished pass is remembered by.
///
/// All three fields are the key. `loaded` is this subsystem's own record count,
/// `levers` its own section of the ini, and `dry` the one setting that decides
/// whether a pass means anything at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memo {
    /// Records (or reward rows) the game had parsed when the pass ran.
    pub loaded: usize,
    /// `DryRun` as it was for that pass.
    pub dry: bool,
    /// The levers that pass ran with.
    pub levers: Levers,
}

/// What a pass did, as far as the gate is concerned.
///
/// Deliberately two numbers rather than the whole count struct: the gate must
/// not grow an opinion about the twelve individual skip counters, and the two
/// callers have different count types anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// True when nothing transient stopped the pass: no write was refused and
    /// it did not stop early. A clean pass is the only kind worth memoising.
    pub clean: bool,
    /// Scalars actually written. Always `0` under `DryRun`, which is what keeps
    /// a dry pass from ever marking the gate dirty.
    pub wrote: usize,
}

impl Outcome {
    /// A pass that finished with nothing left to do and wrote nothing.
    pub const CLEAN: Outcome = Outcome { clean: true, wrote: 0 };
}

/// The memo for one apply pass - one for the missions, one for the reward rows,
/// because those load on completely different schedules.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Gate {
    memo: Option<Memo>,
    dirty: bool,
}

impl Gate {
    /// A gate that has never seen a pass.
    pub const fn new() -> Self {
        Gate { memo: None, dirty: false }
    }

    /// True when a pass is owed: nothing has been memoised, or something in the
    /// key has moved since it was.
    pub fn owed(&self, loaded: usize, dry: bool, lev: &Levers) -> bool {
        self.memo != Some(Memo { loaded, dry, levers: *lev })
    }

    /// Record what a pass did.
    ///
    /// A clean pass is memoised. Anything else **clears** the memo rather than
    /// leaving the old one in place: a stale memo would match again the moment
    /// the levers happened to come back to what it holds, and early-return over
    /// a table that pass had only half written.
    pub fn record(&mut self, loaded: usize, dry: bool, lev: Levers, outcome: Outcome) {
        self.memo =
            if outcome.clean { Some(Memo { loaded, dry, levers: lev }) } else { None };
        if dry {
            // A dry pass wrote nothing, so it cannot have made the game dirty
            // and it cannot have cleaned it either.
            return;
        }
        if lev.is_vanilla() {
            // Only a *clean* revert may declare the game vanilla again: an
            // unclean one is exactly the pass that left some field behind.
            if outcome.clean {
                self.dirty = false;
            }
        } else if outcome.wrote > 0 {
            self.dirty = true;
        }
    }

    /// True when this plugin has written something non-vanilla that no clean
    /// revert has put back yet.
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// What the last clean pass was memoised as, for tests and for a caller
    /// that wants to log it.
    pub fn memo(&self) -> Option<Memo> {
        self.memo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lev(speed: u32, rewards: u32) -> Levers {
        Levers { speed, rewards, clear_skill: false, any_operators: false }
    }

    /// The baseline the memo exists for: nothing moved, nothing to do.
    #[test]
    fn a_repeated_tick_with_nothing_moving_is_owed_nothing() {
        let mut g = Gate::new();
        assert!(g.owed(936, false, &lev(4, 1)), "no pass has run yet");
        g.record(936, false, lev(4, 1), Outcome { clean: true, wrote: 2808 });
        assert!(!g.owed(936, false, &lev(4, 1)));
        // Any one of the three moving is a pass.
        assert!(g.owed(937, false, &lev(4, 1)), "a record appeared");
        assert!(g.owed(936, true, &lev(4, 1)), "DryRun was turned on");
        assert!(g.owed(936, false, &lev(2, 1)), "Speed moved");
    }

    /// BLOCKING 1, the proven sequence. Four ini edits that used to leave every
    /// mission at `vanilla / 4` for the rest of the session while the ini read
    /// `Speed=1 DryRun=0`.
    #[test]
    fn flipping_dry_run_always_forces_one_real_pass() {
        let mut g = Gate::new();
        // 1. Speed=4, DryRun=0: a real pass. Durations are now vanilla/4.
        assert!(g.owed(936, false, &lev(4, 1)));
        g.record(936, false, lev(4, 1), Outcome { clean: true, wrote: 936 });
        assert!(g.dirty(), "the game is non-vanilla now");
        // 2. DryRun=1: the key moved, so a dry pass runs and writes nothing.
        assert!(g.owed(936, true, &lev(4, 1)));
        g.record(936, true, lev(4, 1), Outcome { clean: true, wrote: 0 });
        assert!(g.dirty(), "a dry pass cannot clean the game");
        // 3. Speed=1 while still dry: memoised as a *dry* vanilla pass.
        assert!(g.owed(936, true, &lev(1, 1)));
        g.record(936, true, lev(1, 1), Outcome { clean: true, wrote: 0 });
        assert!(g.dirty(), "still nothing has been written back");
        // 4. DryRun=0: this is the tick that used to early-return.
        assert!(
            g.owed(936, false, &lev(1, 1)),
            "the revert must run: the memo holds a dry pass, not a written one"
        );
        g.record(936, false, lev(1, 1), Outcome { clean: true, wrote: 936 });
        assert!(!g.dirty(), "a clean revert put it back");
        assert!(!g.owed(936, false, &lev(1, 1)), "and now there is nothing left to do");
    }

    /// The mirror of the same bug: a legitimate apply that was memoised while
    /// dry and then never performed.
    #[test]
    fn a_lever_set_while_dry_is_applied_when_dry_run_comes_off() {
        let mut g = Gate::new();
        g.record(936, true, Levers::VANILLA, Outcome::CLEAN);
        // Speed=4 while DryRun=1: computed, logged, not written.
        assert!(g.owed(936, true, &lev(4, 1)));
        g.record(936, true, lev(4, 1), Outcome { clean: true, wrote: 0 });
        assert!(!g.dirty());
        // DryRun=0 with the same levers has to actually apply them.
        assert!(g.owed(936, false, &lev(4, 1)), "the apply was never performed");
    }

    /// BLOCKING 2: a pass that refused a write is not memoised, so the next
    /// tick tries again instead of early-returning over a half-written table.
    #[test]
    fn a_refused_write_is_not_memoised() {
        let mut g = Gate::new();
        g.record(936, false, lev(4, 1), Outcome { clean: true, wrote: 936 });
        // A revert in which one write was refused: the memo must not say
        // "vanilla", because one mission is still at a quarter length.
        g.record(936, false, Levers::VANILLA, Outcome { clean: false, wrote: 935 });
        assert_eq!(g.memo(), None, "an unclean pass memoises nothing at all");
        assert!(g.owed(936, false, &Levers::VANILLA), "and the revert is owed again");
        assert!(g.dirty(), "the refused field is still ours to put back");
        // The retry succeeds.
        g.record(936, false, Levers::VANILLA, Outcome { clean: true, wrote: 1 });
        assert!(!g.dirty());
        assert!(!g.owed(936, false, &Levers::VANILLA));
    }

    /// The same, for a pass that stopped early on `MAX_TOTAL_OPS`: `clean` is
    /// the one bit the gate looks at, whichever transient reason set it.
    #[test]
    fn a_pass_that_stopped_early_is_not_memoised() {
        let mut g = Gate::new();
        g.record(4096, false, lev(4, 1), Outcome { clean: false, wrote: 8192 });
        assert_eq!(g.memo(), None);
        assert!(g.owed(4096, false, &lev(4, 1)));
        assert!(g.dirty(), "8192 fields were written and none of them reverted");
    }

    /// An unclean pass must not leave a memo that could match again later: this
    /// is the sequence that would otherwise skip a revert entirely.
    #[test]
    fn an_unclean_pass_cannot_leave_a_memo_that_matches_later() {
        let mut g = Gate::new();
        g.record(936, false, Levers::VANILLA, Outcome::CLEAN);
        // Speed=4 with one write refused. Under a memo that survived, going
        // back to Speed=1 would have matched the vanilla memo and returned.
        g.record(936, false, lev(4, 1), Outcome { clean: false, wrote: 900 });
        assert!(g.owed(936, false, &Levers::VANILLA), "the revert still has to run");
        assert!(g.dirty());
    }

    /// `dirty` is what tells the caller a revert is owed even before the record
    /// table has settled.
    #[test]
    fn dirty_tracks_whether_a_revert_has_anything_to_do() {
        let mut g = Gate::new();
        assert!(!g.dirty(), "a fresh session has written nothing");
        // A non-vanilla pass that wrote nothing (every field already right)
        // does not make the game dirty on its own account.
        g.record(936, false, lev(4, 1), Outcome { clean: true, wrote: 0 });
        assert!(!g.dirty());
        g.record(936, false, lev(4, 1), Outcome { clean: true, wrote: 12 });
        assert!(g.dirty());
        // Enabled=0 reaches the pass as VANILLA, and a clean one clears it.
        g.record(936, false, Levers::VANILLA, Outcome { clean: true, wrote: 12 });
        assert!(!g.dirty());
    }
}
