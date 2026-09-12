//! The dispatch subsystem's thread: find the `FactionNode` manager by content,
//! wait for the game to map its table, then watch the records it parses and log
//! what is in them each time a batch appears.
//!
//! **Nothing in this file writes to the game**, and that is now a statement
//! about this file rather than about the subsystem. There is no hook, no code
//! patch and no call into a game function anywhere here; every foreign read
//! goes through `safe`, which turns an unmapped page into `None` rather than
//! into a crash to desktop. The walks below - the mission dump, the reward dump
//! and every raw hex line - are read-only exactly as they always were, and a
//! wrong offset in them prints a wrong number and nothing else.
//!
//! The writes live in [`crate::apply`], which this file **drives**: the watch
//! loop decides when a pass is worth running and hands it the levers, and
//! `apply` owns every `safe::write` and every guard in front of one. Keeping
//! the two apart is what lets the diagnostic half stay something that cannot
//! damage anything, whatever the ini says.
//!
//! The pass repeats, because a one-shot pass races the game. The manager
//! reports its full record count as soon as the table is mapped, but the object
//! array at `+0x58` is filled in later. The first build dumped once, at about
//! 15 s, and found 1119 records with 1119 null slots; the next session had all
//! 1119 filled before 12 s. They arrive **all at once**, not one at a time, so
//! this is a race against a step in the game's own loading and not lazy
//! per-record parsing - a distinction worth keeping straight, because only the
//! second would have justified a loader hook. Watching costs nothing and wins
//! the race either way.
//!
//! The offsets it reads live in [`crate::node::parsed`] and came out of
//! decompilation - **except two, which the first real dump corrected.** The
//! mission length is not `entry+0xC0`, which reads a constant `1`; it is the
//! middle step's `+0x08`, a `u32` in tenths of an hour, matched against DMM's
//! 47-value ground truth. `node.rs`'s module header carries the argument.
//!
//! Every line this file writes is counted against one per-pass budget - the
//! `MaxLines` the ini sets - raw hex dumps included, and a line is only written
//! for a mission or a reward row that has not been logged before. Both rules
//! exist for the same reason: the log is one file shared with the looter and
//! the gatherer's hook, every write takes the process-wide lock and reopens the
//! file, and a watch loop that re-logged its whole census every time four more
//! rows appeared would be the loudest thing in the process for no new
//! information at all.
//!
//! A second pass rides the same loop: the missions name `dropsetinfo` rows at
//! `+0xA8` / `+0xAA`, and those rows hold the items and amounts a mission pays.
//! `DumpRewards` says whether they are *printed*; the `Rewards` lever is what
//! decides whether they are *edited*, and it reads them either way. Those rows
//! load lazily and separately from the faction nodes, so they get their own
//! settle-then-act bookkeeping in [`Watch`], and the row set comes **only**
//! from the operation entries - see the reward module comment further down for
//! why that is not negotiable. There are two of those sets, and which one a pass
//! is handed is load-bearing: the dump reads every row a mission named, the
//! write pass is given only the rows of missions it could identify
//! ([`Watch::named`] and [`Watch::acted`]).
//!
//! Four clocks share the one loop, which is why it is worth naming them:
//! the ini is stat'd every [`RELOAD_POLL_MS`]; the record array is counted
//! every [`WATCH_MS`]; a **dump** happens when the loaded count has settled and
//! grown - or has grown at all, when the pass that follows it in the same tick
//! is going to write, so that the census in the dump is a census of vanilla
//! records rather than of this plugin's own output; an **apply pass** happens
//! when the loaded count has settled and
//! either it or the levers have moved since the last one. The last of those is
//! the rule `CLAUDE.md` states in as many words - `DesertTooling.ini` carries
//! four sections, so a `[Looter]` slider being dragged moves this file's
//! modified time once a second, and without a comparison against what is
//! already applied that drag would run a full pass over every mission and every
//! reward row per second.

use std::collections::BTreeSet;

use crate::apply;
use crate::config::{self, Config, Levers};
use crate::gate::Gate;
use crate::module::MainModule;
use crate::node::parsed::dropset;
use crate::node::{self, parsed, Cond, DropsetEntry, DropsetRow, Operation, RewardSummary, Summary};
use crate::{gimmick, log, manager, safe};

/// How often the manager slot is checked while waiting for the table.
const POLL_MS: u64 = 500;
/// How long to wait for it before giving up with one line. The table is read
/// while the level loads; five minutes covers a slow disk and a long shader
/// warm-up with room to spare, and giving up costs nothing but the dump.
const WAIT_SECS: u64 = 300;
/// How many distinct condition keys the histogram line names before it
/// summarises the tail. Every key is still counted.
const HISTOGRAM_KEYS: usize = 250;

/// How often the loaded-object array is counted once the manager exists.
///
/// The record count is published before the records themselves are, so the
/// count alone is not a readiness signal. Verified in game on build 25116796:
/// one session had 1119 records and 1119 null slots at 15.3 s, the next had all
/// 1119 filled by 11.8 s. Polling is what makes the subsystem independent of
/// where in that window it happens to start.
const WATCH_MS: u64 = 2000;
/// Re-dump once the loaded-record count has grown by at least this much since
/// the last dump. Observed behaviour is one burst filling every slot, but the
/// floor costs nothing and stops a table that did trickle in from writing one
/// dump per record.
const REDUMP_GROWTH: usize = 8;
/// How many polls the count must hold still before a dump is written, so a
/// burst is dumped once it has finished rather than part-way through.
const STABLE_POLLS: u32 = 2;
/// Re-dump the reward rows once at least this many more of them have loaded
/// since the last reward dump.
///
/// The `dropsetinfo` rows are lazier than the faction nodes: a row is parsed
/// when something in the game needs it, so the 219 rows the missions name
/// trickle in over a session rather than arriving in one burst. The floor is
/// smaller than [`REDUMP_GROWTH`] because a trickle is the expected case here,
/// and it is not 1 because [`STABLE_POLLS`] alone would then write a dump for
/// every single row that appeared.
const REWARD_REDUMP_GROWTH: usize = 4;
/// How often `DesertTooling.ini` is stat'd for a change.
///
/// The same one-second budget the looter and the gatherer give the overlay's
/// write, so a slider dragged in the menu reaches every subsystem at the same
/// speed. The stat is cheap; what a change costs is a pass, and that is what
/// the comparison against the already-applied levers is for.
const RELOAD_POLL_MS: u64 = 1000;

/// Read the `[Dispatch]` section, or the defaults if there is nothing to read.
fn load_config(path: &std::path::Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (cfg, warnings) = config::parse(&text);
            for w in warnings {
                crate::log!("[ini] {w}");
            }
            cfg
        }
        Err(_) => {
            crate::log!("[ini] {} not found, using defaults", path.display());
            Config::default()
        }
    }
}

/// The manager object the `FactionNode` table was parsed into, once the game
/// has actually loaded it, or `None` after [`WAIT_SECS`].
///
/// Both halves have to be true before the walk is worth starting: the slot
/// holds a plausible pointer *and* the object behind it reports a non-zero
/// record count. A manager that exists with a count of zero is one the loader
/// has allocated but not filled.
fn wait_for_manager(slot: usize) -> Option<(usize, u32)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(WAIT_SECS);
    let mut said_waiting = false;
    loop {
        if let Some(mgr) = safe::read_ptr(slot) {
            if node::plausible(mgr) {
                if let Some(count) = safe::read::<u32>(mgr + parsed::MGR_COUNT) {
                    if count > 0 {
                        return Some((mgr, count));
                    }
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        if !said_waiting {
            said_waiting = true;
            crate::log!("waiting for the FactionNode table to load (up to {WAIT_SECS}s)");
        }
        std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
    }
}

/// The per-pass ceiling on log lines that `MaxLines` sets, and the one place
/// that ceiling is enforced.
///
/// **Every** line a pass writes goes through [`Budget::take`]: mission lines,
/// reward row and entry lines, and every raw hex line under `DumpRaw`. That was
/// not always so - the raw dump used to be outside the count and guarded with
/// `<=` rather than `<`, which made `DumpRaw` unbounded under `LogRecords=0`,
/// unbounded under `MaxLines=0`, and unbounded again once the counter stopped
/// moving - and `config.rs` documents `MaxLines` as exactly this protection, so
/// it has to be true.
pub(crate) struct Budget {
    used: u32,
    max: u32,
    /// Set the first time a line was refused, so the `[cap]` line is written
    /// when the cap actually bit and not merely because a pass logged fewer
    /// lines than it read (which is the normal case once the already-logged
    /// records are being skipped).
    capped: bool,
}

impl Budget {
    pub(crate) fn new(max: u32) -> Self {
        Budget { used: 0, max, capped: false }
    }

    /// Claim one line, or refuse it and remember that the cap bit.
    pub(crate) fn take(&mut self) -> bool {
        if self.used < self.max {
            self.used += 1;
            true
        } else {
            self.capped = true;
            false
        }
    }
}

/// Read one operation entry whole, or `None` if any field of it would not
/// read. Partial entries are dropped rather than reported: a half-read entry
/// is indistinguishable from a wrong offset, and this subsystem exists to tell
/// those apart.
///
/// `raw` is the entry's own [`parsed::OP_STRIDE`] bytes, already read as one
/// block by the caller: the scalar fields are decoded from them by
/// [`node::decode_operation`], which is pure and unit-tested natively, so the
/// offsets are exercised on this machine by the same code that runs in the game
/// process. The step list and the condition list are pointers the entry only
/// names, so those are still followed here.
fn read_operation(
    node_key: u32,
    index: usize,
    entry: usize,
    raw: &[u8],
    sum: &mut Summary,
) -> Option<Operation> {
    // The `dropsetinfo` row indices in here are what the reward pass reads, and
    // the only source it is allowed to take a row from: `dropsetinfo` is the
    // game-wide drop table, so a row set gathered any other way would be
    // measuring (and, one day, multiplying) every chest in the game.
    let f = node::decode_operation(raw)?;

    // The step list, which is where the mission's duration actually lives. A
    // list that will not read is not a reason to drop the entry - the fields
    // above still say something - so it degrades to 0 steps and a 0 duration.
    let (steps, duration_tenths) = read_steps(entry);
    let conds = read_conditions(entry, sum);

    Some(Operation {
        node_key,
        index,
        key: f.key,
        group: f.group,
        duration_tenths,
        min_operators: f.min_operators,
        max_operators: f.max_operators,
        combat_power: f.combat_power,
        steps,
        flag_c0: f.flag_c0,
        reward1: f.reward1,
        reward2: f.reward2,
        skill_req: f.skill_req,
        skill_stat: f.skill_stat,
        conds,
    })
}

/// `(step count, duration in tenths of an hour)` for one entry; the duration
/// is `0` when the list is absent, unreadable or shorter than two steps.
///
/// The duration is **the middle step's** `+0x08` and nothing else. Every entry
/// carries exactly three steps, one per operation state
/// (`cpp-operation-state-goto`, `-working`, `-leave`), and only the middle
/// "working" one has a non-zero value there; the first version of this
/// function summed all three and called the total a "cost", which reached the
/// same number by accident and hid what the field was. Reading `step[1]`
/// specifically is what keeps a log line honest if a future entry ever puts a
/// number on another step.
fn read_steps(entry: usize) -> (usize, u32) {
    let (Some(ptr), Some(n)) = (
        safe::read_ptr(entry + parsed::OP_STEPS),
        safe::read::<u32>(entry + parsed::OP_STEP_COUNT),
    ) else {
        return (0, 0);
    };
    if !node::plausible(ptr) {
        return (0, 0);
    }
    let n = (n as usize).min(node::MAX_STEPS);
    // Count the steps that actually read, so a truncated list still reports a
    // truthful step count.
    let mut seen = 0usize;
    for i in 0..n {
        // `n` is capped at MAX_STEPS and `ptr` at well below the top of the
        // address space by `plausible`, so this cannot wrap.
        if safe::read::<u32>(ptr + i * parsed::STEP_STRIDE + parsed::STEP_DURATION).is_none() {
            break;
        }
        seen += 1;
    }
    // `parsed::DURATION_STEP` is the second of the three; a list too short to
    // have one reports no duration rather than borrowing another step's value.
    let duration = if seen > parsed::DURATION_STEP {
        safe::read::<u32>(ptr + parsed::DURATION_STEP * parsed::STEP_STRIDE + parsed::STEP_DURATION)
            .unwrap_or(0)
    } else {
        0
    };
    (seen, duration)
}

/// The entry's condition list. **This is the output the whole subsystem is
/// for**: `docs/findings-dispatch-2026-09-10.md` could not settle which
/// `conditioninfo` records appear here from the exe alone.
fn read_conditions(entry: usize, sum: &mut Summary) -> Vec<Cond> {
    let mut conds = Vec::new();
    let (Some(ptr), Some(n)) = (
        safe::read_ptr(entry + parsed::OP_CONDS),
        safe::read::<u32>(entry + parsed::OP_COND_COUNT),
    ) else {
        sum.skips.cond_read += 1;
        return conds;
    };
    if n == 0 {
        return conds;
    }
    if !node::plausible(ptr) {
        sum.skips.cond_read += 1;
        return conds;
    }
    let n = (n as usize).min(node::MAX_CONDS);
    for i in 0..n {
        let at = ptr + i * parsed::COND_STRIDE;
        let (Some(key), Some(show), Some(fail)) = (
            safe::read::<u16>(at + parsed::COND_KEY),
            safe::read::<u16>(at + parsed::COND_SHOW),
            safe::read::<u16>(at + parsed::COND_FAIL),
        ) else {
            sum.skips.cond_read += 1;
            break;
        };
        conds.push(Cond { key, show, fail });
    }
    conds
}

// ---------------------------------------------------------------------------
// The reward rows.
//
// A dispatch mission's payout lives in the `dropsetinfo` table, not in the
// mission: the operation entry carries two `u16` row indices (`+0xA8`, `+0xAA`)
// and the items and amounts are over there. Every offset used below is
// decompilation-only - `docs/reference-internals.md` section 20.17 says in as
// many words that nothing in that layout has been read in game - so this pass
// is what settles them, and the `rawdrop` / `rawdropentry` lines under
// `DumpRaw` are what settles them *offline* if a named field turns out to be
// at the wrong place.
//
// Two constraints on this code, both from section 20.17, and neither is
// stylistic:
//
//   1. The rows walked come **only** from operation entries. `dropsetinfo` is
//      the game-wide drop table; a pass that enumerated it would be reporting
//      on every chest and carcass in the game, and the multiplier this dump is
//      groundwork for would silently become a global loot mod.
//   2. Rows load **lazily**. A null row slot is the normal reading, not an
//      error, which is why the pass repeats and counts them rather than
//      complaining.
//
// And, as everywhere else here: nothing below writes. Every read is a `safe`
// read of our own process.
// ---------------------------------------------------------------------------

/// Read one entry of a `dropsetinfo` row, or `None` if its bytes would not
/// read. Same rule as [`read_operation`]: a half-read entry is
/// indistinguishable from a wrong offset, so it is dropped and counted, and the
/// bytes are read as one block and decoded by the pure, natively tested
/// [`node::decode_entry`].
fn read_dropset_entry(index: usize, at: usize) -> Option<DropsetEntry> {
    let mut raw = [0u8; dropset::ENTRY_RAW];
    if !safe::read_into(at, &mut raw) {
        return None;
    }
    node::decode_entry(index, &raw)
}

/// Read one `dropsetinfo` record whole: its head, then every entry it points
/// at. `None` when the head itself would not read.
///
/// The entry array at `+0x28` is an array of **pointers** to entries, not an
/// array of inline entries, so each entry costs one more dereference than an
/// operation entry does - and each of those pointers is checked before it is
/// followed. A row whose entries would not read still comes back, with the
/// count it declared beside the entries that actually read: that difference is
/// itself a finding.
fn read_dropset_row(row: u16, obj: usize, sum: &mut RewardSummary) -> Option<DropsetRow> {
    let draws = safe::read::<i32>(obj + dropset::ROW_DRAWS)?;
    let declared_entries = safe::read::<u32>(obj + dropset::ROW_ENTRY_COUNT)?;
    let no_drop_ppm = safe::read::<i64>(obj + dropset::ROW_NO_DROP_PPM)?;

    let declared = declared_entries as usize;
    if declared > node::MAX_DROP_ENTRIES {
        sum.entries_capped += 1;
    }
    let n = declared.min(node::MAX_DROP_ENTRIES);
    let mut entries = Vec::new();
    if n > 0 {
        match safe::read_ptr(obj + dropset::ROW_ENTRIES) {
            Some(arr) if node::plausible(arr) => {
                for i in 0..n {
                    // `n` is capped at MAX_DROP_ENTRIES and `arr` well below
                    // the top of the address space by `plausible`, so the
                    // arithmetic cannot wrap.
                    let Some(at) = safe::read_ptr(arr + i * dropset::ENTRY_PTR_STRIDE) else {
                        sum.entry_read_fail += 1;
                        break;
                    };
                    if !node::plausible(at) {
                        sum.entry_read_fail += 1;
                        continue;
                    }
                    match read_dropset_entry(i, at) {
                        Some(e) => entries.push(e),
                        None => sum.entry_read_fail += 1,
                    }
                }
            }
            // A row that declares entries and has no readable array to hold
            // them is exactly what a wrong `+0x28` or `+0x30` looks like.
            _ => sum.entry_read_fail += 1,
        }
    }
    Some(DropsetRow { row, draws, declared_entries, no_drop_ppm, entries })
}

/// The hex dump of one reward row: its head, then each entry it points at,
/// every line of it counted against the pass's [`Budget`].
///
/// **This is the part that survives a wrong offset.** Every named field above
/// is a hypothesis; these bytes are not, and they are what lets the layout be
/// re-read offline without another game launch. Do not trim it down to the
/// fields we think we understand - that is the mistake `rawstep` exists to
/// remember.
fn dump_raw_row(row: u16, obj: usize, entries: usize, budget: &mut Budget) {
    let mut head = [0u8; dropset::ROW_RAW];
    if budget.take() && safe::read_into(obj, &mut head) {
        crate::log!("rawdrop row={row} {}", node::hex_line(&head));
    }
    let Some(arr) = safe::read_ptr(obj + dropset::ROW_ENTRIES) else {
        return;
    };
    if !node::plausible(arr) {
        return;
    }
    let mut raw = [0u8; dropset::ENTRY_RAW];
    for i in 0..entries.min(node::MAX_DROP_ENTRIES) {
        // One row can be 60 entries, and there are 219 rows: without a cap
        // check of its own this loop was the one place `MaxLines` did not
        // reach at all.
        if !budget.take() {
            return;
        }
        if let Some(at) = safe::read_ptr(arr + i * dropset::ENTRY_PTR_STRIDE) {
            if node::plausible(at) && safe::read_into(at, &mut raw) {
                // **The address is part of the dump, not decoration.** One 0x70
                // entry blob is byte-identical under rows 14665 and 14715, and
                // 79 of its 112 bytes are zero, so identical content cannot say
                // whether the game shares one entry object between two rows -
                // which is the question that decides whether a remember table
                // keyed on `(row, index)` can poison its own vanilla
                // (`crate::apply`'s module header). The write path no longer
                // depends on the answer, but one capture of these lines settles
                // it outright: two rows printing the same `at` is the proof.
                crate::log!(
                    "rawdropentry row={row} idx={i} at=0x{at:X} {}",
                    node::hex_line(&raw)
                );
            }
        }
    }
}

/// `(record count, record-object array)` of the `dropsetinfo` manager, or
/// `None` while it is not there yet. The same manager shape every static-info
/// table has, read through the same two offsets.
pub(crate) fn dropset_manager(slot: usize) -> Option<(usize, usize)> {
    let (count, records) = manager::view(slot)?;
    Some((count as usize, records))
}

/// How many of `rows` the game has actually parsed, as one pointer read each.
///
/// Deliberately cheap, exactly like [`count_loaded`]: it runs every
/// [`WATCH_MS`] for the life of the process, and 219 pointer reads cost
/// nothing measurable.
fn count_rows_loaded(slot: usize, rows: &[u16]) -> usize {
    let Some((count, records)) = dropset_manager(slot) else {
        return 0;
    };
    // The same cheap reading as `count_loaded`, and the same reason for
    // `manager::record` rather than `manager::slot` over there.
    rows.iter()
        .map(|&row| usize::from(row))
        .filter(|&idx| idx < count && manager::record(records, idx).is_some())
        .count()
}

/// Read every reward row in `rows`, and log the ones that have not been logged
/// before.
///
/// `rows` is the union of the missions' `+0xA8` and `+0xAA`, and nothing else
/// ever gets into it - see the module comment above.
///
/// `logged` carries across passes and holds the rows whose lines are already in
/// the log. A pass re-reads and re-counts everything, because that is what the
/// summary and the verdict are for, but it writes lines only for what is new:
/// the reward rows arrive a few at a time over a whole session, and re-printing
/// 219 rows and 725 entries every time four more appear would take the shared
/// log lock thousands of times to say what it already said.
fn dump_rewards(
    slot: usize,
    rows: &[u16],
    cfg: &Config,
    logged: &mut BTreeSet<u16>,
) -> RewardSummary {
    let mut sum = RewardSummary::new(rows.len());
    let Some((count, records)) = dropset_manager(slot) else {
        crate::log!(
            "[rewards] the dropsetinfo manager is not mapped yet (slot 0x{slot:X}); the reward \
             rows will be read on a later pass"
        );
        return sum;
    };

    let mut budget = Budget::new(cfg.max_lines);
    for &row in rows {
        let idx = usize::from(row);
        if idx >= count {
            // Not a lazy row: an index past the end of the table, which is
            // what a `+0xA8` that is not a row index would look like.
            sum.rows_out_of_range += 1;
            continue;
        }
        let obj = match manager::slot(records, idx) {
            manager::Slot::Loaded(p) => p,
            manager::Slot::Empty => {
                // Normal: rows are parsed on demand.
                sum.rows_null += 1;
                if cfg.debug {
                    crate::log!("[rewards] row {row} is not loaded yet");
                }
                continue;
            }
            manager::Slot::Unreadable => {
                sum.rows_unreadable += 1;
                continue;
            }
        };
        let Some(r) = read_dropset_row(row, obj, &mut sum) else {
            sum.rows_unreadable += 1;
            continue;
        };
        // A row already logged on an earlier pass is read and counted again -
        // that is what the summary and the verdict are for - and only its lines
        // are skipped.
        let fresh = !logged.contains(&row);
        if fresh && cfg.log_records && budget.take() {
            crate::log!("{}", r.log_line());
            for e in &r.entries {
                if !budget.take() {
                    break;
                }
                crate::log!("{}", e.log_line(row));
            }
        }
        if fresh && cfg.dump_raw {
            dump_raw_row(row, obj, r.entries.len(), &mut budget);
        }
        // Remembered as logged only while the budget has refused nothing: a row
        // whose lines the cap ate is left for the next pass rather than being
        // silently marked done and never printed at all.
        if fresh && !budget.capped {
            logged.insert(row);
        }
        sum.row(&r);
    }
    if budget.capped {
        crate::log!(
            "[cap] reward lines stopped at MaxLines={} ({} written this pass); the summary below \
             covers every row that was read",
            cfg.max_lines,
            budget.used
        );
    }
    sum
}

/// One reward pass, plus the two lines that summarise it.
fn reward_pass(
    slot: usize,
    rows: &[u16],
    cfg: &Config,
    logged: &mut BTreeSet<u16>,
) -> RewardSummary {
    let sum = dump_rewards(slot, rows, cfg, logged);
    crate::log!("[rewards] {}", sum.counts_line());
    crate::log!("[rewards] {}", sum.verdict());
    sum
}

/// Walk every record of the manager once, logging as `cfg` asks.
///
/// `records` and `count` are one snapshot of the manager, taken by the caller
/// through [`manager_view`] so that the count and the array it indexes are
/// always read together. `logged` holds the missions already in the log, keyed
/// by `(node key, operation key)`, so a re-walk after eight more records
/// appeared writes lines for those eight and nothing else - the summary,
/// histogram and verdict below still cover every mission read.
fn walk(
    records: usize,
    count: usize,
    cfg: &Config,
    logged: &mut BTreeSet<(u32, u32)>,
) -> Summary {
    let mut sum = Summary::new();
    let mut budget = Budget::new(cfg.max_lines);
    let mut total_ops = 0usize;

    for idx in 0..count {
        let obj = match manager::slot(records, idx) {
            manager::Slot::Loaded(p) => p,
            manager::Slot::Empty => {
                sum.record(false);
                sum.skips.unloaded += 1;
                continue;
            }
            manager::Slot::Unreadable => {
                sum.record(false);
                sum.skips.record_read += 1;
                continue;
            }
        };
        let (Some(node_key), Some(op_count)) = (
            safe::read::<u32>(obj + parsed::REC_KEY),
            safe::read::<u32>(obj + parsed::REC_OP_COUNT),
        ) else {
            sum.record(false);
            sum.skips.record_read += 1;
            continue;
        };
        sum.record(true);

        let ops_ptr = safe::read_ptr(obj + parsed::REC_OPS).unwrap_or(0);
        if op_count == 0 || ops_ptr == 0 {
            sum.skips.no_ops += 1;
            if cfg.debug {
                crate::log!("node key={node_key} idx={idx} has no missions");
            }
            continue;
        }
        if !node::plausible(ops_ptr) {
            sum.skips.record_read += 1;
            continue;
        }
        sum.record_with_ops();
        let n = (op_count as usize).min(node::MAX_OPS);
        if n < op_count as usize {
            sum.skips.ops_capped += 1;
        }

        for i in 0..n {
            // The bound that actually holds. Per-record and per-table caps
            // multiply out to a million entries and a dozen guarded reads
            // each; one misread `u32` would wedge this thread for the rest of
            // the session, and there would be nothing in the log to say why.
            if total_ops >= node::MAX_TOTAL_OPS {
                sum.skips.ops_budget += 1;
                crate::log!(
                    "[cap] stopped after {} operation entries (MAX_TOTAL_OPS); a vanilla table \
                     holds 936, so a count somewhere is being misread - the summary below covers \
                     only what was walked",
                    node::MAX_TOTAL_OPS
                );
                return finish_walk(sum, &budget, cfg);
            }
            total_ops += 1;

            let entry = ops_ptr + i * parsed::OP_STRIDE;
            // One block read of the whole entry: it is what the raw dump needs
            // anyway, and it is what lets the field decode be pure and tested
            // natively rather than being a list of guarded reads nothing but
            // the game can exercise.
            let mut raw = [0u8; parsed::OP_STRIDE];
            if !safe::read_into(entry, &mut raw) {
                sum.skips.op_read += 1;
                continue;
            }
            let Some(op) = read_operation(node_key, i, entry, &raw, &mut sum) else {
                sum.skips.op_read += 1;
                continue;
            };
            let fresh = !logged.contains(&(node_key, op.key));
            if fresh && cfg.log_records && budget.take() {
                crate::log!("{}", op.log_line());
            }
            if fresh && cfg.dump_raw && budget.take() {
                crate::log!("raw node={node_key} idx={i} {}", node::hex_line(&raw));
                // The step list too, unchanged: dumping these three 0x30-byte
                // blocks is how the duration was found (the middle step's
                // `+0x08`, everything else zero), and it is how the next
                // unknown field will be. Do not trim it down to the fields we
                // now understand.
                if let (Some(ptr), Some(n)) = (
                    safe::read_ptr(entry + parsed::OP_STEPS),
                    safe::read::<u32>(entry + parsed::OP_STEP_COUNT),
                ) {
                    let n = (n as usize).min(node::MAX_STEPS);
                    if node::plausible(ptr) && n > 0 {
                        let mut step = [0u8; parsed::STEP_STRIDE];
                        for k in 0..n {
                            if !budget.take() {
                                break;
                            }
                            // A step that will not read ends the list, exactly
                            // as `read_steps` treats it: continuing would print
                            // whatever the previous iteration left in the
                            // buffer as if it were step k.
                            if !safe::read_into(ptr + k * parsed::STEP_STRIDE, &mut step) {
                                break;
                            }
                            crate::log!(
                                "rawstep node={node_key} idx={i} step={k} {}",
                                node::hex_line(&step)
                            );
                        }
                    }
                }
            }
            // Same rule as the reward rows: a mission is remembered as logged
            // only while the budget has refused nothing, so a mission the cap
            // ate is printed on the next pass instead of being lost.
            if fresh && !budget.capped {
                logged.insert((node_key, op.key));
            }
            sum.operation(&op);
        }
    }

    finish_walk(sum, &budget, cfg)
}

/// The one `[cap]` line a walk may owe, and the summary it returns either way.
fn finish_walk(sum: Summary, budget: &Budget, cfg: &Config) -> Summary {
    if budget.capped {
        crate::log!(
            "[cap] mission lines stopped at MaxLines={} ({} written this pass); the summary below \
             covers every mission that was read",
            cfg.max_lines,
            budget.used
        );
    }
    sum
}


/// `Enabled=1 LogRecords=1 ... Speed=1 Rewards=1 ...`, shared between the
/// startup line and the reload loop's `[ini] reloaded:` line so both read the
/// same way and one `grep` finds either.
fn ini_summary(cfg: &Config) -> String {
    format!(
        "Enabled={} LogRecords={} MaxLines={} Debug={} DumpRaw={} DumpRewards={} \
         Speed={} Rewards={} NoSkillRequirement={} AnyOperatorCount={} DryRun={}",
        cfg.enabled as u8,
        cfg.log_records as u8,
        cfg.max_lines,
        cfg.debug as u8,
        cfg.dump_raw as u8,
        cfg.dump_rewards as u8,
        cfg.speed,
        cfg.rewards,
        cfg.no_skill_requirement as u8,
        cfg.any_operator_count as u8,
        cfg.dry_run as u8
    )
}

/// The ini's last-modified time, or `None` if it cannot be stat'd (missing,
/// permissions, mid-write on some filesystems). Used only to notice a change
/// cheaply; the reload below still re-reads and re-parses on top.
///
/// **`None` is a reading, not an absence of one.** A deleted ini stats as `None`
/// for ever after, and [`watch`] compares this value for *inequality* rather than
/// requiring it to be `Some`: the file going away is a change, and the change it
/// means is "every lever back to its default, which is vanilla".
fn ini_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Wait for the `FactionNode` table, then watch it for the life of the process.
///
/// **This does not return.** It ends in [`watch`], which loops for ever:
/// records and reward rows keep being parsed as the player moves around and
/// opens the faction map, so there is no point at which the subsystem could say
/// it has seen everything. The only ways out are before that - `Enabled=0`, a
/// module or manager slot that will not resolve, or the table never loading
/// inside [`WAIT_SECS`].
///
/// `desert-tooling` calls this on a thread of its own. It has no hook to
/// install, so unlike the gatherer it has no reason to be first and nothing
/// before it on its thread matters; and unlike the looter it has no boot grace,
/// because it waits on the data itself rather than on the clock.
pub fn start() {
    // The one source of truth for which file this subsystem reads is the
    // schema it hands the menu: the overlay writes the keys back into
    // `Section::ini`, so the poll below has to watch that same file rather
    // than a second constant that could drift from it.
    let ini_name = config::schema().ini;
    let ini_path = log::exe_dir().join(&ini_name);
    crate::log!(
        "started ({}), settings from [{}] of {ini_name}",
        crate::VERSION,
        config::INI_SECTION
    );
    let cfg = load_config(&ini_path);
    crate::log!("[ini] {}", ini_summary(&cfg));
    // The hook-free claim, restated where a log reader will see it: everything
    // this subsystem writes is a field of a parsed static-info record, and
    // those are re-read from disk every launch.
    crate::log!(
        "no hook is installed and no game code is patched; the settings edit parsed records only, \
         which the game re-reads from disk every launch"
    );
    if !cfg.enabled {
        // Nothing has been applied, so there is nothing to put back: this is
        // the one case where returning early is still the whole story. Flipping
        // `Enabled` to 0 *while the game runs* is a different thing entirely -
        // the loop below stays alive and reverts.
        crate::log!(
            "Enabled=0: nothing is read, nothing is written and nothing is logged this session"
        );
        return;
    }
    // The write path reads its levers from here, and so does every log line
    // that names them. Published before anything can run a pass.
    config::LIVE.publish(&cfg);
    if cfg.dry_run {
        crate::log!("[ini] DryRun=1: the log shows what would change, nothing is written");
    }
    if cfg.levers().is_vanilla() {
        crate::log!(
            "[ini] every lever is at vanilla: the pass reads and remembers every mission and \
             reward row, and writes nothing"
        );
    }

    let Some(module) = MainModule::locate() else {
        crate::log!("could not locate the main module; giving up");
        return;
    };

    // The two bytes that decide whether anything this subsystem writes can ever
    // be banked into the save, read once and **only** read: see
    // `node::globals`, which is the one place here that names an address instead
    // of finding it by content, and says why that is acceptable for these two.
    // `docs/reference-internals.md` section 20.18 calls them the most
    // consequential open question in the feature, and the reason the README's
    // `AnyOperatorCount` warning has to hedge twice; nothing acts on either
    // value, and one capture of this line settles the hedging.
    crate::log!(
        "{}",
        node::globals::banking_gate_line(
            safe::read::<u8>(module.base + node::globals::DEFERRED_REWARD_GATE_RVA),
            safe::read::<u8>(module.base + node::globals::SURPLUS_WORKER_GATE_RVA),
        )
    );

    // By content, never by address: the 0.2 milestone rule in VERSIONING.md.
    // `FactionNode` is reachable only through the indirect accessor template,
    // which is what `resolve_manager_slot_based` adds over the resolver the
    // looter and the gatherer use.
    //
    // Both tables are resolved through one scan. Finding the 149 template
    // copies costs one pass per encoding over the whole 363 MB image, and the
    // table name is only consulted afterwards - so the two lookups below were
    // scanning the same image for the same four patterns twice, eight passes
    // and 0.88 to 1.81 s of startup measured over six launches. `AccessorSites`
    // is those four passes, kept.
    let sites = match gimmick::AccessorSites::scan(module.bytes(), Some(module.base)) {
        Ok(sites) => sites,
        Err(e) => {
            crate::log!("the accessor template could not be scanned: {e}; nothing to do");
            return;
        }
    };
    let slot_rva =
        match sites.manager_slot(module.bytes(), gimmick::FACTION_NODE_TABLE) {
            Ok(rva) => rva,
            Err(e) => {
                crate::log!("the FactionNode manager slot was NOT found: {e}; nothing to do");
                return;
            }
        };
    let slot = module.base + slot_rva;
    crate::log!("FactionNode manager slot at +0x{slot_rva:X} (0x{slot:X})");

    // The reward rows live in a second table, resolved the same way and for
    // the same reason: by content, never by address. It is resolved
    // unconditionally now, because `DumpRewards` only says whether the rows are
    // *printed* - the `Rewards` lever needs them whatever that key says, and it
    // can be turned on mid-session. A failure here costs the reward half and
    // nothing else: the mission walk does not depend on it.
    let dropset_slot =
        match sites.manager_slot(module.bytes(), gimmick::DROPSET_TABLE) {
            Ok(rva) => {
                crate::log!("dropsetinfo manager slot at +0x{rva:X} (0x{:X})", module.base + rva);
                Some(module.base + rva)
            }
            Err(e) => {
                crate::log!(
                    "[rewards] the dropsetinfo manager slot was NOT found: {e}; the missions are \
                     still walked, their reward rows are neither dumped nor multiplied"
                );
                None
            }
        };
    if !cfg.dump_rewards {
        crate::log!(
            "[rewards] DumpRewards=0: the reward rows are not dumped. They are still read and \
             multiplied whenever Rewards is above 1 - there is no way to multiply a row without \
             reading it."
        );
    }

    let Some((mgr, count)) = wait_for_manager(slot) else {
        crate::log!(
            "the FactionNode table was still not loaded after {WAIT_SECS}s; giving up. Nothing \
             was written either way - no pass ever ran."
        );
        return;
    };
    crate::log!("manager 0x{mgr:X}, {count} records");
    // `count` is not passed on: `watch` re-reads it, and the array it indexes,
    // on every poll. It is only logged here.
    watch(mgr, cfg, &ini_path, dropset_slot);
}

/// `(record count, record-object array)` of the `FactionNode` manager as they
/// are **right now**, or `None` if either would not read.
///
/// Read together, and re-read on every poll rather than cached at startup. The
/// count published when the table first appears is not the count for the rest
/// of the session - records get registered later - and a cached array pointer
/// with a fresh count is the worst of both. `desert_gatherer::hook::reapply`
/// re-reads its manager the same way and for the same reason.
fn manager_view(mgr: usize) -> Option<(usize, usize)> {
    let (count, records) = manager::view_of(mgr)?;
    Some(((count as usize).min(node::MAX_RECORDS), records))
}

/// Count the record slots the game has actually parsed something into.
///
/// Deliberately cheap: one pointer read per slot and no field reads at all, so
/// it can run every [`WATCH_MS`] for the life of the process without costing
/// anything measurable.
fn count_loaded(count: usize, records: usize) -> usize {
    // `manager::record`, not `manager::slot`: this counts non-null slots and
    // never uses one as a base, so it deliberately does not apply the
    // `plausible` filter the walks do. Flattening the two would change what this
    // clock reads on a slot holding a non-null value no pointer check would
    // accept - which is exactly the reading that must not move silently.
    (0..count).filter(|&idx| manager::record(records, idx).is_some()).count()
}

/// Everything the watch loop carries from one poll to the next.
///
/// Two independent settle-then-act state machines - one for the faction node
/// records, one for the `dropsetinfo` rows, because those arrive on completely
/// different schedules - and, for each, two things that can trigger work: a
/// **dump**, which is read-only and gated on the table having grown, and an
/// **apply pass**, which writes and is gated on the table or the levers having
/// moved since the last one. The two [`Gate`]s are the whole of that second
/// gate, and without them a `[Looter]` slider drag would run a full pass every
/// second. What is in a gate's key, and why a pass that did not finish is not
/// memoised at all, is argued in [`crate::gate`] - both of those were bugs that
/// left the game non-vanilla while the ini said vanilla.
struct Watch {
    /// Loaded record count at the previous poll, and how many polls it has
    /// held still. `usize::MAX` = no poll has happened yet.
    last: usize,
    stable: u32,
    /// Loaded record count at the last **dump**.
    dumped_at: usize,
    /// The missions already written to the log, by `(node key, operation key)`.
    logged_ops: BTreeSet<(u32, u32)>,
    /// Every `dropsetinfo` row a mission has named, from the diagnostic walk and
    /// from the apply pass alike - and from nothing else, ever: `dropsetinfo` is
    /// the game-wide drop table. **This set is read, never written**: it is what
    /// the dump prints and what the loaded-row clock counts.
    named: BTreeSet<u16>,
    /// The rows of missions the apply pass actually **identified**, and the only
    /// set the write path is ever handed.
    ///
    /// A subset of [`Self::named`], and the difference is the point. The dump
    /// reads whatever a mission's `+0xA8` says so that a wrong stride is visible
    /// in the log; the write path may only touch a row named by a mission whose
    /// key the remember table accepted and whose values were in band. Before the
    /// two sets were separated, the diagnostic walk fed the same set the reward
    /// writes came out of, so an unidentified mission could still put a row in
    /// front of a write.
    acted: BTreeSet<u16>,
    /// True once a row has been refused for the [`node::MAX_DROP_ROWS`] cap, so
    /// the log says it exactly once.
    rows_over_cap: bool,
    /// The reward rows already written to the log.
    logged_rows: BTreeSet<u16>,
    rows_last: usize,
    rows_stable: u32,
    rows_dumped_at: usize,
    rows_dumped_of: usize,
    /// Loaded record count, `DryRun` and levers at the last **clean** mission
    /// apply pass, plus whether anything non-vanilla is still written.
    gate: Gate,
    /// The same, for the reward rows.
    rows_gate: Gate,
}

impl Watch {
    fn new() -> Self {
        Watch {
            last: usize::MAX,
            stable: 0,
            dumped_at: 0,
            logged_ops: BTreeSet::new(),
            named: BTreeSet::new(),
            acted: BTreeSet::new(),
            rows_over_cap: false,
            logged_rows: BTreeSet::new(),
            rows_last: usize::MAX,
            rows_stable: 0,
            rows_dumped_at: 0,
            rows_dumped_of: 0,
            gate: Gate::new(),
            rows_gate: Gate::new(),
        }
    }

    /// One poll of the faction node records: count them, settle, then dump
    /// and/or apply if either is owed.
    fn missions_tick(&mut self, mgr: usize, cfg: &Config) {
        // Fresh every poll, count and array together: records registered after
        // the table first appeared are exactly what this loop exists to catch.
        let Some((count, records)) = manager_view(mgr) else {
            return;
        };
        let loaded = count_loaded(count, records);
        if loaded == self.last {
            self.stable = self.stable.saturating_add(1);
        } else {
            if self.last != usize::MAX && cfg.debug {
                crate::log!("[watch] {loaded} of {count} records parsed");
            }
            self.last = loaded;
            self.stable = 0;
        }
        // Nothing parsed yet is nothing to read and nothing to put back.
        if loaded == 0 {
            return;
        }
        let lev = config::LIVE.levers();
        // **A revert does not wait for the table to settle.** An apply reads
        // first-sight vanilla values, so it wants a table the game has finished
        // filling; a revert writes values already remembered and checks each
        // field before it touches it, so it is correct on a half-filled table by
        // construction. Making the player wait out a settling burst - which a
        // loading screen can keep restarting - to get vanilla back is the wrong
        // trade.
        let revert = lev.is_vanilla() && self.gate.dirty();
        let settled = self.stable >= STABLE_POLLS;
        if !settled && !revert {
            return;
        }
        // The dump runs before the write pass, always. A mission is logged the
        // first time it is seen, and that is the pass that also remembers its
        // vanilla values, so the log's per-mission line records the record as
        // the game parsed it rather than as this plugin left it. It stays behind
        // the settle gate: it is read-only, so nothing about it is urgent.
        //
        // **The growth floor comes off while a pass is going to write.** The
        // dump carries the census and the verdict, and those are a layout check
        // over whatever is in the records *now*: a `Speed` of 20 turns a vanilla
        // 20 into a 1, below `node::DURATION_LO`, and a dump that ran after the
        // write would report LOOKS WRONG about this plugin's own output. Dumping
        // every record that appeared in the same tick that is about to write to
        // it is what keeps the census a vanilla one. When nothing is going to be
        // written - every lever vanilla, or `DryRun` - there is nothing to get
        // in front of, and `REDUMP_GROWTH` goes back to being what it is for:
        // not writing one dump per record if the table ever trickles in.
        let writing = !lev.is_vanilla() && !cfg.dry_run;
        let grown_enough = loaded >= self.dumped_at.saturating_add(REDUMP_GROWTH);
        if settled && loaded > self.dumped_at && (writing || grown_enough) {
            self.dump_missions(records, count, cfg, loaded);
        }
        // The rule from CLAUDE.md: nothing moved, nothing to do. `loaded` is
        // this subsystem's own table and `lev` is its own section, so neither
        // moves when somebody else's key does. `DryRun` is in the key too, and
        // the outcome decides whether the pass is memoised at all.
        if !self.gate.owed(loaded, cfg.dry_run, &lev) {
            return;
        }
        let outcome = self.apply_missions(records, count, cfg, &lev);
        self.gate.record(loaded, cfg.dry_run, lev, outcome);
    }

    /// The read-only dump, unchanged: walk, log what is new, summarise.
    fn dump_missions(&mut self, records: usize, count: usize, cfg: &Config, loaded: usize) {
        let t0 = std::time::Instant::now();
        let sum = walk(records, count, cfg, &mut self.logged_ops);
        crate::log!("[sum] {}", sum.counts_line());
        crate::log!("[sum] condition keys: {}", sum.histogram_line(HISTOGRAM_KEYS));
        // The skill census, beside the condition one: `+0xDA` is the hard gate
        // `NoSkillRequirement` clears, and `+0xD8` is the reward-bonus stat two
        // bytes away from it that the write path must leave alone.
        crate::log!("[sum] skills: {}", sum.skill_line(HISTOGRAM_KEYS));
        if !sum.skips.is_empty() {
            crate::log!("[sum] skipped: {}", sum.skips.summary());
        }
        crate::log!("[sum] {}", sum.verdict());
        // The one case the ordering above cannot fix: records that appeared,
        // were written to, and are only being dumped on a later pass. The census
        // then describes the table as this plugin left it, and a reader who is
        // not told that will go looking for a moved offset.
        if self.gate.dirty() {
            crate::log!("[sum] {}", Summary::written_to_caveat());
        }
        // The reward census, whether or not the rows themselves are being
        // dumped: it comes out of the missions just walked, it is one line, and
        // it states the vanilla figures beside the measured ones so a wrong
        // count is visible at a glance.
        crate::log!("[rewards] {}", sum.reward_request_line());
        // Named, not acted on: these rows come from every mission the walk could
        // read, with no identity guard in front of them, so they may be dumped
        // and counted and must not reach a write. The write path's rows come
        // from `apply::missions` alone.
        self.name_rows(sum.reward_rows());
        crate::log!(
            "dump finished in {:.0} ms ({} of {} records parsed so far); the dump itself writes \
             nothing to the game",
            t0.elapsed().as_secs_f64() * 1000.0,
            loaded,
            count
        );
        self.dumped_at = loaded;
    }

    /// The write pass over the missions, and the two lines it owes the log.
    ///
    /// Returns what the pass managed, because the caller's memo must not record
    /// a pass that refused a write or stopped early: the field it could not
    /// write is still carrying the previous value, and on a revert that value is
    /// the non-vanilla one.
    fn apply_missions(
        &mut self,
        records: usize,
        count: usize,
        cfg: &Config,
        lev: &Levers,
    ) -> crate::gate::Outcome {
        let dry = cfg.dry_run;
        let mut budget = Budget::new(cfg.max_lines);
        let t0 = std::time::Instant::now();
        let (counts, rows) = apply::missions(records, count, lev, dry, cfg.debug, &mut budget);
        // The row set the reward pass is allowed to touch comes from here and
        // from nowhere else: these are the rows of missions this pass identified.
        self.act_on_rows(rows);
        crate::log!("{}", counts.summary(lev, dry));
        if let Some(why) = counts.warning() {
            crate::log!("[apply] WARN {why}");
        }
        if budget.capped {
            crate::log!(
                "[cap] per-change lines stopped at MaxLines={} ({} written this pass); every \
                 change was still made and the summary above covers all of them",
                cfg.max_lines,
                budget.used
            );
        }
        if cfg.debug {
            crate::log!(
                "[apply] mission pass took {:.0} ms",
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
        counts.outcome()
    }

    /// Add rows the **diagnostic walk** saw a mission name. Read-only: they are
    /// dumped and counted, never written to.
    fn name_rows<I: IntoIterator<Item = u16>>(&mut self, rows: I) {
        for row in rows {
            if !node::insert_capped(&mut self.named, row) {
                self.say_over_cap();
            }
        }
    }

    /// Add rows an **identified** mission named. These go into both sets: the
    /// write path takes [`Self::acted`] and the dump takes [`Self::named`].
    fn act_on_rows<I: IntoIterator<Item = u16>>(&mut self, rows: I) {
        for row in rows {
            let in_named = node::insert_capped(&mut self.named, row);
            let in_acted = node::insert_capped(&mut self.acted, row);
            if !in_named || !in_acted {
                self.say_over_cap();
            }
        }
    }

    /// The one line the [`node::MAX_DROP_ROWS`] cap is worth, said once.
    ///
    /// The cap bites where a row is inserted, not where the set is read, and
    /// that is what makes the acted-on set stable: a row that got in stays in,
    /// so nothing already multiplied can drift out of the window a revert walks.
    /// (219 rows on build 25116796, so nothing is anywhere near this.)
    fn say_over_cap(&mut self) {
        if self.rows_over_cap {
            return;
        }
        self.rows_over_cap = true;
        crate::log!(
            "[rewards] the missions have named more than {} distinct dropsetinfo rows; the extra \
             ones are ignored rather than swapped in, so nothing already multiplied can drift out \
             of reach of a revert",
            node::MAX_DROP_ROWS
        );
    }

    /// One poll of the `dropsetinfo` rows the missions have named. They load
    /// lazily and one at a time, so they get their own settle-then-act clock.
    fn rewards_tick(&mut self, slot: usize, cfg: &Config) {
        if self.named.is_empty() {
            return;
        }
        let lev = config::LIVE.levers();
        // Nothing to dump and nothing to multiply, and nothing was ever
        // applied that a revert would have to undo: do not even count the rows.
        // `dirty` rather than "a pass has run": a dry pass wrote nothing, so it
        // leaves nothing for a revert to find.
        if !cfg.dump_rewards && lev.rewards <= 1 && !self.rows_gate.dirty() {
            return;
        }
        // Two sets, and which one a pass gets is not interchangeable: the dump
        // and the loaded-row clock read every row a mission named, the write pass
        // is handed only the rows of missions it identified.
        let rows: Vec<u16> = self.named.iter().copied().collect();
        let live = count_rows_loaded(slot, &rows);
        if live == self.rows_last {
            self.rows_stable = self.rows_stable.saturating_add(1);
        } else {
            if self.rows_last != usize::MAX && cfg.debug {
                crate::log!("[rewards] {live} of {} named rows parsed", rows.len());
            }
            self.rows_last = live;
            self.rows_stable = 0;
        }
        // Same rule the missions get: an apply waits for the rows to settle,
        // a revert does not. Reward rows load one at a time and on demand, so
        // "settled" here can be a long way off.
        let revert = lev.is_vanilla() && self.rows_gate.dirty();
        let settled = self.rows_stable >= STABLE_POLLS;
        if live == 0 || (!settled && !revert) {
            return;
        }
        // The dump: the first rows to appear at all, more rows than last time,
        // or a mission walk that named rows nobody had asked for before - and
        // **any** newly loaded row at all while a pass is about to write to one,
        // for the reason `missions_tick` gives at length: both halves of the
        // amount pair are fields the `Rewards` lever writes, so a census taken
        // after the write describes this plugin's output rather than the layout.
        let writing = lev.rewards > 1 && !cfg.dry_run;
        let first = self.rows_dumped_at == 0;
        let more_rows = live >= self.rows_dumped_at.saturating_add(REWARD_REDUMP_GROWTH);
        let more_asked = rows.len() != self.rows_dumped_of;
        let fresh_row = writing && live > self.rows_dumped_at;
        if settled && cfg.dump_rewards && (first || more_rows || more_asked || fresh_row) {
            let t0 = std::time::Instant::now();
            let sum = reward_pass(slot, &rows, cfg, &mut self.logged_rows);
            if self.rows_gate.dirty() {
                // Rows that loaded, were multiplied, and are only being dumped
                // now: the amounts above are `vanilla * Rewards`.
                crate::log!("[rewards] {}", Summary::written_to_caveat());
            }
            crate::log!(
                "reward dump finished in {:.0} ms ({} of {} named rows parsed so far); the dump \
                 itself writes nothing to the game",
                t0.elapsed().as_secs_f64() * 1000.0,
                sum.rows_loaded,
                rows.len()
            );
            self.rows_dumped_at = sum.rows_loaded.max(live);
            self.rows_dumped_of = rows.len();
        }
        if !self.rows_gate.owed(live, cfg.dry_run, &lev) {
            return;
        }
        // The write pass, over the identified rows alone. Empty until the mission
        // pass has identified a mission that names one, which is also the only
        // state in which there is nothing a revert could owe.
        let acted: Vec<u16> = self.acted.iter().copied().collect();
        if acted.is_empty() {
            return;
        }
        let dry = cfg.dry_run;
        let mut budget = Budget::new(cfg.max_lines);
        let counts = apply::rewards(slot, &acted, &lev, dry, cfg.debug, &mut budget);
        crate::log!("{}", counts.summary(&lev, dry));
        if let Some(why) = counts.warning() {
            crate::log!("[apply] WARN {why}");
        }
        if budget.capped {
            crate::log!(
                "[cap] per-change lines stopped at MaxLines={} ({} written this pass); every \
                 change was still made and the summary above covers all of them",
                cfg.max_lines,
                budget.used
            );
        }
        self.rows_gate.record(live, dry, lev, counts.outcome());
    }
}

/// Watch the object array as the game fills it, apply the levers to what turns
/// up, and re-read the ini for the rest of the session.
///
/// This never returns. Records keep appearing as the player moves around and
/// opens the faction map, so there is no point at which the subsystem could say
/// it has seen everything - the honest thing is to keep watching, keep the
/// missions in step with the ini, and let the log show it happening.
///
/// The loop ticks once a second because that is the budget the overlay's write
/// is given everywhere else in this plugin. The record and reward work is the
/// slower of the two clocks and runs every [`WATCH_MS`] - except immediately
/// after an ini change, which forces one, so a menu edit does not have to wait
/// out the rest of a watch interval.
fn watch(mgr: usize, cfg: Config, ini_path: &std::path::Path, dropset_slot: Option<usize>) {
    let Some((count, _)) = manager_view(mgr) else {
        crate::log!("the manager's record array at +0x{:X} would not read", parsed::MGR_RECORDS);
        return;
    };
    crate::log!(
        "watching {count} record slots every {:.0}s; records load on demand, so the first pass \
         comes when the game parses some (opening the faction map is what does it). Only missions \
         and reward rows that have not been logged before are printed on a later pass.",
        WATCH_MS as f64 / 1000.0
    );

    let mut w = Watch::new();
    // What this subsystem is already running on. The modified time says the
    // *file* changed; this says whether **this subsystem's section** did.
    let mut live = cfg;
    let mut last_ini_mtime = ini_mtime(ini_path);
    let mut last_watch = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(RELOAD_POLL_MS));

        let mut changed = false;
        let mtime = ini_mtime(ini_path);
        // `None` is a change like any other. The condition here used to be
        // `mtime.is_some() && mtime != last`, and a **deleted** ini stats as
        // `None`: no reload ran, nothing was put back, and `Speed=4` stayed
        // applied with no file left in the world saying so. Deleting the settings
        // file now does what it looks like it does.
        if mtime != last_ini_mtime {
            match std::fs::read_to_string(ini_path) {
                Ok(text) => {
                    // Only advance the watermark on a successful read: if the
                    // file is mid-write this same change is retried next tick
                    // instead of being missed.
                    last_ini_mtime = mtime;
                    let (cfg, warnings) = config::parse(&text);
                    // A `[Looter]`, `[Gatherer]` or `[Overlay]` edit moves this
                    // file too. Comparing against what is already live is what
                    // keeps a slider drag somewhere else from costing a pass a
                    // second.
                    if cfg != live {
                        for warning in &warnings {
                            crate::log!("[ini] {warning}");
                        }
                        crate::log!("[ini] reloaded: {}", ini_summary(&cfg));
                        if !cfg.enabled {
                            crate::log!(
                                "[ini] Enabled=0: every mission and reward row this session \
                                 changed is being put back to vanilla"
                            );
                        }
                        config::LIVE.publish(&cfg);
                        live = cfg;
                        changed = true;
                    }
                }
                Err(_) if mtime.is_none() => {
                    // The file is gone. Fall back to the defaults, which is what
                    // `start` does when there is no ini to read in the first
                    // place - and every lever of the defaults is vanilla, so the
                    // pass this schedules puts the table back.
                    last_ini_mtime = None;
                    let cfg = Config::default();
                    crate::log!(
                        "[ini] {} is gone; falling back to the defaults, whose every lever is \
                         vanilla, so anything written this session is being put back",
                        ini_path.display()
                    );
                    if cfg != live {
                        crate::log!("[ini] reloaded: {}", ini_summary(&cfg));
                        config::LIVE.publish(&cfg);
                        live = cfg;
                        changed = true;
                    }
                }
                // Readable a moment ago and not now, and the file is still
                // there: mid-write, or briefly locked. The watermark has not
                // moved, so the next tick tries the same change again.
                Err(_) => {}
            }
        }

        if !changed && last_watch.elapsed() < std::time::Duration::from_millis(WATCH_MS) {
            continue;
        }
        last_watch = std::time::Instant::now();
        w.missions_tick(mgr, &live);
        if let Some(slot) = dropset_slot {
            w.rewards_tick(slot, &live);
        }
    }
}
