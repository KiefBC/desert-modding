//! The species store: how each `characterinfo` row is caught, by name.
//!
//! The class bytes do not say what a creature is. The Rice Fish carries the
//! insect class byte 0x80 on type 6, and so does the raccoon (row 6684 on
//! 26361232), which is carried and skinned, never bagged
//! (`desert_core::creature::BUG_CLASSES`). The species record does say: the
//! generated table `desert_core::species_table` files every `Animal_*`
//! record as a `Bug`, `Fish` or `Critter` bag catch or as `NotHandCatchable`,
//! and every other record with the `Dead_Loot` interaction as a `Body`
//! (bandits, criminals, monsters) or, when its `isEnableFriendly` flag is
//! set, a `FriendlyBody` (citizens, traders, mercenaries).
//! Rows shift between game builds (3935 on 25477059, 3966 on 26361232), so
//! the row is never the key: the record's **name** is. This module reads
//! every loaded `characterinfo` record's name once, on the plugin thread,
//! looks it up in that table and stores one byte per row. Both halves then
//! ask one atomic byte through [`class`]: the looter's catch classifier and
//! the catch-count hook, which runs on a game thread and may do nothing more
//! than that.
//!
//! The record's name place is not assumed - the parsed layouts of
//! `gimmickinfo` and `iteminfo` both moved on 26361232 - and there is no known
//! name to check it against, so it is the place that reads a plausible
//! identifier from most sampled records (`gimmick::probe_record_name`),
//! latched once and logged as `[species] record name at ...`.
//!
//! Everything here is plugin-thread work, after the boot grace, rate-limited,
//! and every read goes through `safe`. The table loads on demand, so the walk
//! is repeated (only over rows it has not named yet) when more rows are
//! loaded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use desert_core::creature::{SpeciesBits, SpeciesClass, SpeciesClasses, SPECIES_ROWS};
use desert_core::species_table::species_class;
use desert_core::gimmick::{self, NameAt, NAME_PROBE_END};
use desert_core::manager::{self, Slot};

use crate::looter::tables::{character_slot_addr, read_bytes};

/// Each row's class, by its name. Set by the plugin thread, read by anyone.
static CLASSES: SpeciesClasses = SpeciesClasses::new();
/// Rows whose name has been read and judged, whether the table knows the
/// name or not; the walk skips them.
static EXAMINED: SpeciesBits = SpeciesBits::new();
/// True once the first walk under a latched name place has finished.
static BUILT: AtomicBool = AtomicBool::new(false);
/// Where a `characterinfo` record keeps its name, once probed.
static NAME_AT: OnceLock<NameAt> = OnceLock::new();

/// Species rows seen in the live 26361232 log (2026-11-02): the Rice Fish
/// (3966, class 0x80), the bugs (3887, 3888, 3895, 3902, 3916), the fish
/// (4031 class 0x23, 4036 class 0x65) and the Raccoon (6684,
/// `Animal_Raccoon_Wild_30042`, class 0x80, `NotHandCatchable`: the class
/// byte alone took it for a bug). Their names and classes are logged once
/// each, so the next log shows what the table is matching against.
/// Build-specific on purpose: these are only ever printed, never decided on.
const LOG_ROWS: [u16; 9] = [3966, 3887, 3888, 3895, 3902, 3916, 4031, 4036, 6684];

/// Records that must read an identifier at the chosen name place.
const MIN_NAMED: usize = 8;
/// How often an unlatched probe walks the table ...
const PROBE_EVERY: Duration = Duration::from_secs(5);
/// ... and after its WARN line has been printed.
const PROBE_EVERY_AFTER_WARN: Duration = Duration::from_secs(60);
/// Probes over loaded records that found nothing before the WARN line.
const WARN_AFTER: u32 = 3;
/// How often the loaded count is checked for rows not named yet.
const WALK_EVERY: Duration = Duration::from_secs(10);
/// The manager's record count is held to this, as everywhere else.
const MAX_RECORDS: u32 = 0x40000;

struct State {
    last_probe: Option<Instant>,
    last_walk: Option<Instant>,
    misses: u32,
    warned: bool,
    /// Loaded rows at the last walk; a walk with no more is skipped.
    walked_loaded: usize,
    /// Bit `i` set once `LOG_ROWS[i]` has been logged.
    rows_logged: u16,
    /// The counts the last `[species] table:` line printed.
    last_counts: Option<Counts>,
}

/// Bookkeeping, plugin thread only, taken with `try_lock`.
static STATE: Mutex<State> = Mutex::new(State {
    last_probe: None,
    last_walk: None,
    misses: 0,
    warned: false,
    walked_loaded: 0,
    rows_logged: 0,
    last_counts: None,
});

/// The `[species] table:` line's numbers.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Counts {
    bug: usize,
    fish: usize,
    critter: usize,
    not_catchable: usize,
    body: usize,
    friendly_body: usize,
    /// Rows whose name was read and is not in the species table: people,
    /// mounts, props without loot, and any species newer than the table.
    unknown: usize,
}

/// How `row`'s species is caught, by its record's name. One atomic load: safe
/// on the game thread, inside a hook. `None` for a name the species table
/// does not hold and for every row until the walk has read it, which the
/// classifiers (`creature::catch_class_by_species`) answer with the class
/// bytes, exactly as before the table existed.
pub fn class(row: u16) -> Option<SpeciesClass> {
    CLASSES.get(row)
}

/// The word the log lines use for a row's species class.
pub fn class_name(class: Option<SpeciesClass>) -> &'static str {
    match class {
        Some(SpeciesClass::Bug) => "bug",
        Some(SpeciesClass::Fish) => "fish",
        Some(SpeciesClass::Critter) => "critter",
        Some(SpeciesClass::NotHandCatchable) => "not-catchable",
        Some(SpeciesClass::Body) => "body",
        Some(SpeciesClass::FriendlyBody) => "friendly-body",
        None => "unknown",
    }
}

/// Has the first walk finished? Before it, [`class`] answers `None` for
/// every row and the classifiers behave exactly as they did without it.
pub fn built() -> bool {
    BUILT.load(Ordering::Relaxed)
}

/// True on the game thread (the sweep hook's), where nothing here may read a
/// string.
fn on_game_thread() -> bool {
    let game = crate::looter::events::game_thread_id();
    if game == 0 {
        return false;
    }
    // SAFETY: `GetCurrentThreadId` takes no arguments and only reads the
    // calling thread's own TEB; it is sound to call from any thread.
    let tid = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    tid == game
}

/// The record object of `row`, if the game has loaded it.
fn record(row: u16) -> Option<usize> {
    let (count, records) = manager::view(character_slot_addr()?)?;
    if u32::from(row) >= count.min(MAX_RECORDS) {
        return None;
    }
    match manager::slot(records, usize::from(row)) {
        Slot::Loaded(rec) => Some(rec),
        _ => None,
    }
}

/// The species name of `row`, through the latched name place. `None` before
/// the place is latched, for a row not loaded, and always on the game thread.
/// For the F11 survey and the log; nothing decides on it.
pub fn name(row: u16) -> Option<String> {
    let at = *NAME_AT.get()?;
    if on_game_thread() {
        return None;
    }
    gimmick::read_name_at(record(row)?, at, &read_bytes)
}

/// `(row, record)` for every loaded `characterinfo` record, rows past the
/// `u16` range left out (a species row cannot name them).
fn loaded() -> Option<(u32, Vec<(u16, usize)>)> {
    let (count, records) = manager::view(character_slot_addr()?)?;
    if count == 0 || count >= MAX_RECORDS {
        return None;
    }
    let n = (count as usize).min(SPECIES_ROWS);
    let mut out = Vec::new();
    for i in 0..n {
        let Slot::Loaded(rec) = manager::slot(records, i) else { continue };
        let Ok(row) = u16::try_from(i) else { break };
        out.push((row, rec));
    }
    Some((count, out))
}

/// Every string any candidate name place reads off `rec`, for the WARN line
/// that says no place was chosen: the next log then shows where the name is.
fn raw_candidates(rec: usize) -> String {
    let mut out = Vec::new();
    for offset in (0..NAME_PROBE_END).step_by(8) {
        for derefs in [2u8, 1] {
            let at = NameAt { offset, derefs };
            if let Some(s) = gimmick::read_name_at(rec, at, &read_bytes) {
                out.push(format!("{}={s:?}", at.describe()));
            }
        }
    }
    if out.is_empty() {
        "nothing".to_string()
    } else {
        out.join(" ")
    }
}

/// Probe the name place once, rate-limited. Returns the log line to print.
fn probe(st: &mut State, now: Instant) -> Option<String> {
    let every = if st.warned { PROBE_EVERY_AFTER_WARN } else { PROBE_EVERY };
    if st.last_probe.is_some_and(|t| now.duration_since(t) < every) {
        return None;
    }
    st.last_probe = Some(now);
    let (count, rows) = loaded()?;
    if rows.is_empty() {
        return None;
    }
    let recs: Vec<usize> = rows.iter().map(|&(_, r)| r).collect();
    match gimmick::probe_record_name(&recs, MIN_NAMED, &read_bytes) {
        Ok((at, tied)) => {
            let _ = NAME_AT.set(at);
            let tie = if tied.is_empty() {
                String::new()
            } else {
                let t: Vec<String> = tied.iter().map(NameAt::describe).collect();
                format!("; tied with {}, lowest offset taken", t.join(", "))
            };
            Some(format!(
                "[species] record name at {} ({} loaded of {count} characterinfo records{tie})",
                at.describe(),
                rows.len()
            ))
        }
        Err(why) => {
            st.misses += 1;
            if st.misses < WARN_AFTER || st.warned {
                return None;
            }
            st.warned = true;
            // A row from the live log if one is loaded, else the first.
            let sample = LOG_ROWS
                .iter()
                .find_map(|&r| rows.iter().find(|&&(row, _)| row == r).copied())
                .or_else(|| rows.first().copied());
            let raw = sample
                .map(|(row, rec)| format!("; row {row} reads {}", raw_candidates(rec)))
                .unwrap_or_default();
            Some(format!(
                "[species] WARN characterinfo record name not found ({why}){raw}; catches are classified by their class bytes alone"
            ))
        }
    }
}

/// Name every loaded row not named yet and store its class. Returns the log
/// lines to print.
fn walk(st: &mut State, at: NameAt, now: Instant) -> Vec<String> {
    let mut lines = Vec::new();
    if st.last_walk.is_some_and(|t| now.duration_since(t) < WALK_EVERY) {
        return lines;
    }
    st.last_walk = Some(now);
    let Some((count, rows)) = loaded() else { return lines };
    if built() && rows.len() <= st.walked_loaded {
        return lines;
    }
    let first = !built();
    for &(row, rec) in &rows {
        if EXAMINED.contains(row) {
            continue;
        }
        let Some(name) = gimmick::read_name_at(rec, at, &read_bytes) else { continue };
        EXAMINED.set(row);
        if let Some(c) = species_class(&name) {
            CLASSES.set(row, c);
        }
    }
    st.walked_loaded = rows.len();
    BUILT.store(true, Ordering::Relaxed);

    let [bug, fish, critter, not_catchable, body, friendly_body] = CLASSES.counts();
    let unknown = EXAMINED.count().saturating_sub(bug + fish + critter + not_catchable + body + friendly_body);
    let counts = Counts { bug, fish, critter, not_catchable, body, friendly_body, unknown };
    if st.last_counts != Some(counts) {
        st.last_counts = Some(counts);
        lines.push(format!(
            "[species] table: {bug} bug, {fish} fish, {critter} critter, {not_catchable} not catchable, \
             {body} body, {friendly_body} friendly body, {unknown} unknown ({} of {count} characterinfo records loaded)",
            rows.len()
        ));
    }

    let mut waiting = Vec::new();
    for (i, &want) in LOG_ROWS.iter().enumerate() {
        let bit = 1u16 << i;
        if st.rows_logged & bit != 0 {
            continue;
        }
        let name = rows
            .iter()
            .find(|&&(row, _)| row == want)
            .and_then(|&(_, rec)| gimmick::read_name_at(rec, at, &read_bytes));
        match name {
            Some(n) => {
                st.rows_logged |= bit;
                lines.push(format!("[species] row {want} = {n} ({})", class_name(class(want))));
            }
            None if first => waiting.push(want.to_string()),
            None => {}
        }
    }
    if !waiting.is_empty() {
        lines.push(format!(
            "[species] rows not loaded yet: {} (each is logged when it loads)",
            waiting.join(", ")
        ));
    }
    lines
}

/// One step of the species work: probe the name place until it latches, then
/// fill and grow the class store. Called once a second by the harvest thread
/// after the boot grace; everything inside is rate-limited, and it does
/// nothing without the `characterinfo` slot.
pub fn tick() {
    if character_slot_addr().is_none() || on_game_thread() {
        return;
    }
    let Ok(mut st) = STATE.try_lock() else { return };
    let now = Instant::now();
    let mut lines = Vec::new();
    if NAME_AT.get().is_none() {
        lines.extend(probe(&mut st, now));
    }
    if let Some(&at) = NAME_AT.get() {
        lines.extend(walk(&mut st, at, now));
    }
    drop(st);
    for line in lines {
        crate::log!("{line}");
    }
}
