//! Desert Dispatch - the dispatch-mission subsystem of `DesertTooling.asi`.
//!
//! It started as a read-only dump and it is not one any more. What it does now:
//! it watches the `FactionNodeInfo` records the game parses, and the
//! `dropsetinfo` reward rows those records name, logs what is in them, and
//! **edits five fields of them** to whatever the `[Dispatch]` section of
//! `DesertTooling.ini` asks for - shorter missions, bigger rewards, no skill
//! gate, any number of workers: four settings over five fields, because
//! `Rewards` writes both halves of a min/max pair. The diagnostic half is
//! unchanged and is still read-only; the write half is [`apply`], and this
//! paragraph is the whole of what it may claim.
//!
//! **What it still does not do**, and these are structural rather than a
//! promise:
//!
//! * **No hook, no trampoline, no code patch.** This crate does not re-export
//!   `hook` or `trampoline` and does not depend on anything that does, so the
//!   code to patch a prologue is not reachable from here by any spelling.
//!   Everything runs on this subsystem's own watch loop, which is possible only
//!   because the records it edits are *parsed* objects that live for the
//!   session - unlike the gatherer's raw stream bytes, which exist for one
//!   instant per record and are what forced a loader hook there.
//! * **Nothing is written to save data.** Every address it writes is a field of
//!   a parsed static-info record. Those tables are re-read from disk on every
//!   launch and are never serialised (`docs/reference-internals.md` section
//!   20.18), so removing the plugin restores vanilla at the next launch with
//!   nothing to clean up. What the *game* may bank on its own, out of a field
//!   this subsystem wrote, is the exception below - a value the game computed,
//!   not a write of ours.
//! * **Nothing outside dispatch is touched.** The reward rows come only from
//!   the `+0xA8` / `+0xAA` of an operation entry. `dropsetinfo` is the
//!   game-wide drop table, and a row set gathered any other way would make this
//!   a global loot mod by accident.
//!
//! **The one exception is inert on build 25116796, and it is still worth
//! carrying.** `Rewards` banks nothing: both payout branches re-read the
//! `dropsetinfo` amounts from the live table at the moment items are granted, so
//! a parked reward pays at whatever `Rewards` says *when it lands* (section
//! 20.18). The lever that *could* leave something behind is
//! **`AnyOperatorCount`**. What a completed mission stores is a single `i64`
//! percent, and the only plugin-written field feeding it is `entry+0xC4`: the
//! percent carries a surplus-worker term `max(0, workersSent - entry+0xC4)`, so
//! with `+0xC4` forced to `1` a full crew would look like a surplus crew and the
//! stored percent would be inflated - and it would pay out later even after the
//! plugin was gone.
//!
//! **Both gates in front of that are off.** Measured 2026-09-11 on build
//! 25116796, one launch: the deferred-reward gate (`0x6BC6AA8`) reads `0`, so no
//! completed mission defers a payout and nothing is appended to the stored-reward
//! list at all; the surplus-worker gate (`0x6BA07C8`) also reads `0`, so the
//! stored percent is the flat base and `entry+0xC4` never enters the arithmetic.
//! Nothing this subsystem writes reaches save data on this build. The paragraph
//! stays because the code path is real and both gates are plain data bytes a game
//! update could flip - `0x6BA07C8`'s own compiled initialiser writes `1` and
//! something zeroes it before the game runs, so neither value is structural:
//! `node::globals::banking_gate_line` prints them at
//! startup on the `[banking]` line, and that line - not this comment - is the
//! check after an update. Were they both on, the exposure would be at most the
//! 142 of 936 operations with `entry+0x118 != 0`, bounded to missions completed
//! under the forced minimum, self-clearing as the list drains, and not save
//! corruption. `Speed` and `NoSkillRequirement` appear nowhere in that percent
//! either way.
//!
//! # The four levers, over five fields
//!
//! Four settings, five fields: `Rewards` writes both halves of a min/max pair,
//! and a write that scaled one half would invert 136 of the 725 entries. All of
//! them are fields of parsed records, all were confirmed in game, and two carry
//! a trap that is written out at length where they are declared:
//!
//! | Lever | Field | Note |
//! | --- | --- | --- |
//! | `Speed` | `steps[1]+0x08`, `u32` tenths of an hour | floored at one tenth; reaches missions already under way (section 20.4) |
//! | `Rewards` | dropset `entry+0x20` **and** `+0x28`, `i64` | both halves, or 136 entries invert; retroactive to parked rewards |
//! | `NoSkillRequirement` | `entry+0xDA`, `i16` | write `0xFFFF`; `0` makes a mission unstartable. Start-time gate |
//! | `AnyOperatorCount` | `entry+0xC4`, `u32` | write `1`; `0` is stricter, not laxer. Start-time gate, **and the one field a save could keep if the banking gates were on** |
//!
//! The two start-time gates are checked when a mission starts, so a mission
//! already running keeps running under them; a **repeating** mission re-checks on
//! every restart, which is why removing the plugin stops a repeating operation
//! with a handled error rather than letting it continue.
//!
//! Reverting is not a separate path: `Enabled=0`, or any lever back at its
//! vanilla value, reaches [`apply`] as [`config::Levers::VANILLA`], and a pass
//! under vanilla levers writes the remembered original back into whatever is
//! currently parsed. [`remember`] is what makes that exact - it holds each
//! record's vanilla values from the first time the pass ever saw it, and never
//! overwrites them.
//!
//! # Why it exists
//!
//! `docs/findings-dispatch-2026-09-10.md` is a static-analysis study of the
//! game's "dispatch missions" - **FactionOperation** internally. A dispatch
//! mission is not its own table: it is a 0x120-byte sub-record inside a
//! `FactionNodeInfo` record, and the study recovered its field offsets from
//! the decompiled record copy, the validator and the UI. Two things it could
//! not settle from the exe alone - whether those offsets were right, and which
//! `conditioninfo` records actually appear in an operation's condition list -
//! were settled by one launch with the dump enabled, and the corrections that
//! came back (the duration is not `+0xC0`; the reward amount is a pair; `kind
//! 13` is not an item drop) are why the write path could be built at all. The
//! dump is still here, still on by default, and still what a future offset
//! question gets answered with.
//!
//! # The reward rows
//!
//! A mission's payout is not in the mission. The operation entry carries two
//! `u16` row indices into the **`dropsetinfo`** table (`+0xA8` and `+0xAA`,
//! `0xFFFF` = none) and the items and amounts live over there; a vanilla table
//! names 219 distinct rows across 936 missions, holding 725 entries.
//! [`node::parsed::dropset`] carries what each offset now stands on, and the
//! four still marked UNCONFIRMED - none of which anything writes to.
//!
//! One of those offsets earns its keep twice over: `row+0x48` holds the sum of
//! that row's entry weights, exactly, on 219/219 rows. Nothing here edits a
//! weight, so the invariant survives every write this subsystem makes, and
//! [`apply`] asserts it before touching a row - a free check that the pointer
//! being walked really is a dropset record.
//!
//! # Why it needs no hook
//!
//! The gatherer hooks the record loader because the raw stream buffer it edits
//! is freed seconds after loading, so there is exactly one instant per record
//! in which to write. The *parsed* objects this subsystem edits live for the
//! session, so a plain pass on the plugin's own thread over `manager+0x58` can
//! reach every record the loader ever produced, and can reach it again a minute
//! later when the ini changes. That is why this carries near-zero crash risk:
//! no prologue is patched, no game function is called, and every foreign read
//! and write goes through `desert_core::safe`, which faults into a `None` or a
//! `false` instead of into a crash.
//!
//! The reward rows are lazier than the faction nodes - a `dropsetinfo` row is
//! parsed when something in the game first needs it - so the pass repeats and
//! picks up the rows that have appeared since, which is again a poll rather
//! than a hook, and again costs nothing but a pointer read per row.
//!
//! This crate is an rlib with no `DllMain` of its own: `desert-tooling` owns
//! the entry point, the host-exe gate, the log file and the shared ini, and
//! calls [`start`] on a thread of its own.

// The shared plumbing lives in desert-core, re-exported under the names this
// workspace has always used, so `crate::log!`, `crate::safe::read` and
// `crate::module::MainModule` keep resolving inside every module here. `log`
// names both a module and the exported macro; one `use` brings in both.
// `manager` is the static-info record-manager plumbing - the two manager
// offsets, the pointer check and the record-slot walk - which the gatherer
// reads the same tables through; it lives in `desert-core` because this was the
// third copy of it.
// `hook` and `trampoline` are deliberately absent, and now that this subsystem
// writes to the game they matter more rather than less: it installs no hook and
// patches no code, and a name that does not resolve is a cheaper guard against
// one being added by habit than a comment is. `safe` is here because every
// foreign read and write goes through it - that is the rule it exists for, not
// an exception to this one. By the same argument, only the modules this crate
// actually uses are named here: an absent name is a guard only while the list
// is the list of what is used.
pub use desert_core::{gimmick, ini, log, manager, schema};
#[cfg(windows)]
pub use desert_core::{module, safe};

// Same split as every other crate here: anything that talks to the game or to
// Win32 is #[cfg(windows)], the rest links natively on Linux so `cargo test
// --target x86_64-unknown-linux-gnu` runs the unit tests.
pub mod config;
// The apply-pass memo. Pure state, so the sequences that used to leave the game
// non-vanilla while the ini said vanilla (a dry pass memoised as if it had
// written; a pass that refused a write memoised as if it had not) are unit
// tested natively rather than discovered in a log.
pub mod gate;
pub mod node;
pub mod remember;

#[cfg(windows)]
mod apply;
#[cfg(windows)]
mod scan;
#[cfg(windows)]
pub use scan::start;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What this subsystem's log lines are tagged with in the one shared
/// `DesertTooling.log`.
///
/// `desert_core::log!` expands to `log::write_tagged(crate::LOG_TAG, ...)`, and
/// `crate::` in a `macro_rules!` body resolves at the **call site's** crate, so
/// this one line is what makes every `crate::log!` in this crate compile and
/// come out as `[dispatch]`. Nothing else names it.
pub const LOG_TAG: &str = "dispatch";

#[cfg(test)]
mod tests {
    #[test]
    fn core_is_linked() {
        // Both tables this subsystem walks are found through desert-core's
        // gimmick resolver, so the link is not incidental.
        assert_eq!(super::gimmick::FACTION_NODE_TABLE, b"FactionNode");
        assert_eq!(super::gimmick::DROPSET_TABLE, b"dropsetinfo");
    }
}
