//! Name lookup in the game's `*InfoManager` data tables, following the
//! traversal the reference mod uses (`FUN_1800151f0`, whose argument is the
//! global slot, so `*slot` is the manager object):
//!   manager+8 = u32 count (< 0x40000); manager+0x58 = record**;
//!   record+8 -> ptr -> ptr -> NUL-terminated name (3+ printable chars).
//!
//! The two global pointer slots this file needs, `iteminfo` and `gimmickinfo`,
//! are not constants here. [`resolve_slots`] asks
//! [`desert_core::gimmick::resolve_manager_slot`] for each of them once at
//! startup, which finds the `mov rbx,[rip+disp]` in front of the accessor copy
//! that names the table, so nothing in this file is a build-specific address.
//!
//! Neither is the layout of a parsed *gimmick* record: 26361232 moved its key,
//! name and output list, so the `gimmick_record_*` readers go through a layout
//! and a name offset picked once per session from the loaded records
//! ([`maybe_probe`]). The parsed *item* record's key and name are picked
//! the same way ([`maybe_probe_items`]), against the item ids the gimmick
//! records pay out.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use desert_core::gimmick::{ItemLayout, LayoutVote, NameAt, NameVote, ParsedLayout};
use desert_core::manager::{self, Slot};

use crate::module::MainModule;
use crate::safe;

/// RVAs of the two manager slots, filled in once by [`resolve_slots`]. Empty
/// means the scan failed for that table and every lookup through it declines.
static ITEM_INFO_SLOT: OnceLock<usize> = OnceLock::new();
static GIMMICK_INFO_SLOT: OnceLock<usize> = OnceLock::new();
/// The same slot as an absolute address (`base + rva`), for the record walks
/// that pick the parsed layout ([`maybe_probe`]); the accessors they serve
/// are handed a record pointer and no module.
static GIMMICK_SLOT_ADDR: OnceLock<usize> = OnceLock::new();
/// The `iteminfo` slot as an absolute address, for [`maybe_probe_items`].
static ITEM_SLOT_ADDR: OnceLock<usize> = OnceLock::new();

fn item_slot() -> Option<usize> {
    ITEM_INFO_SLOT.get().copied()
}

fn gimmick_slot() -> Option<usize> {
    GIMMICK_INFO_SLOT.get().copied()
}

/// Resolve both slots by content and log one line each, in the style of
/// `game::resolve`'s `[sig]` lines. Called once at startup; a failure for one
/// table leaves the other working.
pub fn resolve_slots(m: &MainModule) {
    // Both slots come out of one scan. Finding the accessor template's copies
    // does not depend on which table is being asked about - the name is
    // consulted only afterwards - so a resolve per table searched the whole
    // 363 MB image twice over for the same two encodings. Measured 0.53-0.65 s
    // per-call against 0.25-0.28 s shared, 2.1-2.4x.
    //
    // `None`, not `m.base`: this subsystem wants two tables that a `lea`
    // encoding names, and a base would put the two indirect encodings into the
    // scan as well - twice the passes to reach the same two answers. The base
    // belongs to the scan rather than to each lookup precisely so that this
    // call site can decline it.
    let sites = match desert_core::gimmick::AccessorSites::scan(m.bytes(), None) {
        Ok(sites) => sites,
        Err(e) => {
            crate::log!("[slot] the accessor template could not be scanned: {e}");
            return;
        }
    };
    for (name, table, cell) in [
        ("iteminfo", desert_core::gimmick::ITEM_TABLE, &ITEM_INFO_SLOT),
        ("gimmickinfo", desert_core::gimmick::GIMMICK_TABLE, &GIMMICK_INFO_SLOT),
    ] {
        match sites.manager_slot(m.bytes(), table) {
            Ok(rva) => {
                crate::log!("[slot] {name:<22} = +0x{rva:X}");
                // Runs once, so a second `set` cannot happen; ignoring it is
                // still the only sane answer if it ever did.
                let _ = cell.set(rva);
                let abs = if name == "gimmickinfo" { &GIMMICK_SLOT_ADDR } else { &ITEM_SLOT_ADDR };
                if let Some(addr) = m.base.checked_add(rva) {
                    let _ = abs.set(addr);
                }
            }
            Err(e) => crate::log!("[slot] {name:<22} NOT FOUND: {e}"),
        }
    }
}

pub fn manager_at(m: &MainModule, slot_rva: usize) -> Option<usize> {
    safe::read_ptr(m.base + slot_rva)
}

pub fn record_count(m: &MainModule, slot_rva: usize) -> Option<u32> {
    let inner = manager_at(m, slot_rva)?;
    let n: u32 = safe::read(inner + 8)?;
    (n > 0 && n < 0x40000).then_some(n)
}

/// The name of record `id` (an index) of the table at `slot_rva`, for the
/// debug listing `game::dump_table_records`. The two tables this file knows
/// read through their latched layouts ([`item_record_name`],
/// [`gimmick_record_name`]); any other table keeps the generic
/// `**(rec+0x08)`, which is what `iteminfo` used through 25477059 and is not
/// known to hold for any table on 26361232 - the listing prints the raw
/// qwords beside it, so a wrong guess reads `?`, never misleads.
pub fn name(m: &MainModule, slot_rva: usize, id: u32) -> Option<String> {
    let inner = manager_at(m, slot_rva)?;
    let n: u32 = safe::read(inner + 8)?;
    if n == 0 || n >= 0x40000 || id >= n {
        return None;
    }
    let arr = safe::read_ptr(inner + 0x58)?;
    let rec = safe::read_ptr(arr + id as usize * 8)?;
    if item_slot() == Some(slot_rva) {
        return item_record_name(rec);
    }
    if gimmick_slot() == Some(slot_rva) {
        return gimmick_record_name(rec);
    }
    let p = safe::read_ptr(rec + 8)?;
    let s = safe::read_ptr(p)?;
    safe::read_cstr(s, 64).filter(|s| s.len() >= 3)
}

/// key -> record pointer for every record in a table whose key is at `key_off`.
pub fn key_index(m: &MainModule, slot_rva: usize, key_off: usize) -> std::collections::HashMap<u32, usize> {
    let mut out = std::collections::HashMap::new();
    let Some(mgr) = manager_at(m, slot_rva) else { return out };
    let n: u32 = safe::read(mgr + 8).unwrap_or(0);
    let Some(arr) = safe::read_ptr(mgr + 0x58) else { return out };
    if n == 0 || n >= 0x40000 || !safe::readable(arr, 8 * n as usize) {
        return out;
    }
    for i in 0..n as usize {
        let Some(rec) = safe::read_ptr(arr + i * 8) else { continue };
        if let Some(k) = safe::read::<u32>(rec + key_off) {
            out.entry(k).or_insert(rec);
        }
    }
    out
}

/// Candidate name strings behind a gimmick record's +0x10 pointer.
pub fn gimmick_name_candidates(rec: usize) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(p) = safe::read_ptr(rec + 0x10) {
        if let Some(s) = safe::read_cstr(p, 64) { out.push(format!("p:{s}")); }
        if let Some(q) = safe::read_ptr(p) {
            if let Some(s) = safe::read_cstr(q, 64) { out.push(format!("*p:{s}")); }
            if let Some(r) = safe::read_ptr(q) {
                if let Some(s) = safe::read_cstr(r, 64) { out.push(format!("**p:{s}")); }
            }
        }
    }
    out
}

/// Name by the two record layouts seen so far:
///   A (iteminfo):    record+0x08 -> ptr -> ptr -> chars
///   B (gimmickinfo): record+0x10 -> ptr -> ptr -> chars   (`**(rec+0x10)`)
pub fn record_name_any(rec: usize) -> Option<String> {
    for off in [0x08usize, 0x10] {
        if let Some(p) = safe::read_ptr(rec + off) {
            if let Some(q) = safe::read_ptr(p) {
                if let Some(s) = safe::read_cstr(q, 64).filter(|s| s.len() >= 3) {
                    return Some(s);
                }
            }
        }
    }
    None
}

/// Try every InfoManager the census knows for `id`, by index and by key at
/// +0 / +8. Logs each hit.
pub fn probe_all_tables(_m: &MainModule, census: &[(usize, usize, String)], id: u32) {
    for (rva, obj, class) in census {
        if !class.ends_with("InfoManager") {
            continue;
        }
        let Some(count) = safe::read::<u32>(obj + 8) else { continue };
        if count == 0 || count > 0x40000 {
            continue;
        }
        let Some(arr) = safe::read_ptr(obj + 0x58) else { continue };
        if !safe::readable(arr, 8 * count as usize) {
            continue;
        }
        if id < count {
            if let Some(rec) = safe::read_ptr(arr + id as usize * 8) {
                if let Some(n) = record_name_any(rec) {
                    crate::log!("[probe] {class} (+0x{rva:X}) count={count}: [index {id}] = {n}");
                }
            }
        }
        let mut hits = 0;
        for i in 0..count as usize {
            let Some(rec) = safe::read_ptr(arr + i * 8) else { continue };
            for koff in [0usize, 8] {
                if safe::read::<u32>(rec + koff) == Some(id) {
                    let n = record_name_any(rec).unwrap_or_else(|| "?".into());
                    crate::log!("[probe] {class} (+0x{rva:X}) count={count}: key@+0x{koff:X} == {id} at [{i}] = {n}");
                    hits += 1;
                }
            }
            if hits >= 3 { break; }
        }
    }
}

/// Every record pointer of a table, keyed by pointer -> (table label, index).
pub fn record_pointer_set(
    census: &[(usize, usize, String)],
    classes: &[&str],
) -> std::collections::HashMap<usize, (String, usize)> {
    let mut out = std::collections::HashMap::new();
    for (_, obj, class) in census {
        if !classes.iter().any(|c| c == class) {
            continue;
        }
        let Some(count) = safe::read::<u32>(obj + 8) else { continue };
        if count == 0 || count > 0x40000 {
            continue;
        }
        let Some(arr) = safe::read_ptr(obj + 0x58) else { continue };
        if !safe::readable(arr, 8 * count as usize) {
            continue;
        }
        for i in 0..count as usize {
            if let Some(rec) = safe::read_ptr(arr + i * 8) {
                out.insert(rec, (class.clone(), i));
            }
        }
    }
    out
}

/// key -> [(table, index)] over the given tables, trying keys at +0 and +8.
pub fn key_index_multi(
    census: &[(usize, usize, String)],
    classes: &[&str],
) -> std::collections::HashMap<u32, Vec<(String, usize, usize)>> {
    let mut out: std::collections::HashMap<u32, Vec<(String, usize, usize)>> = std::collections::HashMap::new();
    for (_, obj, class) in census {
        if !classes.iter().any(|c| c == class) {
            continue;
        }
        let Some(count) = safe::read::<u32>(obj + 8) else { continue };
        if count == 0 || count > 0x40000 {
            continue;
        }
        let Some(arr) = safe::read_ptr(obj + 0x58) else { continue };
        if !safe::readable(arr, 8 * count as usize) {
            continue;
        }
        for i in 0..count as usize {
            let Some(rec) = safe::read_ptr(arr + i * 8) else { continue };
            for koff in [0usize, 8] {
                if let Some(k) = safe::read::<u32>(rec + koff) {
                    if k > 0xFFFF && k != 0xFFFF_FFFF {
                        out.entry(k).or_default().push((class.clone(), i, rec));
                    }
                }
            }
        }
    }
    out
}

/// Gimmick record by index (the u16 the game stores on the component).
/// Records are lazily loaded by the game; a null slot means "not loaded yet".
pub fn gimmick_record(m: &MainModule, index: u16) -> Option<usize> {
    let mgr = manager_at(m, gimmick_slot()?)?;
    let count: u32 = safe::read(mgr + 8)?;
    if index as u32 >= count {
        return None;
    }
    let arr = safe::read_ptr(mgr + 0x58)?;
    safe::read_ptr(arr + index as usize * 8)
}

/// The record's `u32` key, at the latched layout's `key`: `+0x08` through
/// 25477059 ([`ParsedLayout::OLD`]), `+0x10` on 26361232
/// ([`ParsedLayout::NEW`]). Until a layout latches it reads the old one.
pub fn gimmick_record_key(rec: usize) -> Option<u32> {
    safe::read(rec.checked_add(record_layout().0.key)?)
}

/// The record's name, through the latched [`NameAt`]: `**(rec+0x10)` through
/// 25477059, wherever [`maybe_probe`] found it on a later build. `None` when
/// the layout moved and no name offset has been found (the one WARN line
/// says so), which leaves ground items unclassified but never misnamed.
pub fn gimmick_record_name(rec: usize) -> Option<String> {
    read_name(rec, record_layout().1?)
}

fn read_name(rec: usize, at: NameAt) -> Option<String> {
    desert_core::gimmick::read_name_at(rec, at, &read_bytes)
}

// ---------------------------------------------------------------------------
// The parsed gimmick record layout, latched once per session
// ---------------------------------------------------------------------------
//
// Build 26361232 moved the parsed record object (`ParsedLayout::NEW`): the
// key from `+0x08` to `+0x10`, where the name pointer used to be, and the
// output list from an inline `{ptr, u32 size, u32 cap}` at `+0x278` to a
// pointer at `+0x128` to that header. Read through the old offsets every
// node's key was wrong and its name unreadable, so no node had a family and
// auto-loot found no target. The yield half probes the same move from the
// records it remembered at load time (`gatherer::hook::parsed_layout`); the
// looter has no such samples, so it picks among the known layouts by walking
// the manager's loaded records against the gather keys `collect` knows, and
// then finds the name by matching `collect`'s names.

/// The layout, once at least [`MIN_AGREE`] records agreed on it.
static LAYOUT: OnceLock<ParsedLayout> = OnceLock::new();
/// The name's place, once at least [`MIN_AGREE`] records agreed on it.
static NAME_AT: OnceLock<NameAt> = OnceLock::new();

/// Bookkeeping for the probe; only the unlatched path takes the lock, and
/// with `try_lock`, so the accessors never wait on it.
struct ProbeState {
    last: Option<Instant>,
    layout_misses: u32,
    name_misses: u32,
    layout_warned: bool,
    name_warned: bool,
}

static PROBE: Mutex<ProbeState> = Mutex::new(ProbeState {
    last: None,
    layout_misses: 0,
    name_misses: 0,
    layout_warned: false,
    name_warned: false,
});

/// Records that must agree before anything latches.
const MIN_AGREE: usize = 3;
/// A walk stops early once a candidate has this many votes.
const ENOUGH: usize = 16;
/// The cap the manager's record count is held to, as everywhere in this file.
const MAX_RECORDS: usize = 0x40000;
/// How often an unlatched probe walks the table.
const PROBE_EVERY: Duration = Duration::from_secs(5);
/// ... and after its WARN line has been printed.
const PROBE_EVERY_AFTER_WARN: Duration = Duration::from_secs(60);
/// Walks over loaded records that found nothing before the WARN line: the
/// table is still filling in for the first few.
const WARN_AFTER: u32 = 3;

/// A guarded read of `len` bytes at `addr`, for the pure readers in
/// `desert_core::gimmick`.
fn read_bytes(addr: usize, len: usize) -> Option<Vec<u8>> {
    if !manager::plausible(addr) {
        return None;
    }
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, 0);
    safe::read_into(addr, &mut v).then_some(v)
}

/// The layout and the name's place to read records through now: the latched
/// ones, else [`ParsedLayout::OLD`] and, while the layout is unlatched or
/// old, [`NameAt::OLD`] - what this file read before 26361232. Probes
/// (rate-limited) while either is unlatched.
fn record_layout() -> (ParsedLayout, Option<NameAt>) {
    if let (Some(l), Some(n)) = (LAYOUT.get(), NAME_AT.get()) {
        return (*l, Some(*n));
    }
    maybe_probe();
    let layout = LAYOUT.get().copied();
    let name = NAME_AT
        .get()
        .copied()
        .or_else(|| layout.is_none_or(|l| l == ParsedLayout::OLD).then_some(NameAt::OLD));
    (layout.unwrap_or(ParsedLayout::OLD), name)
}

/// One rate-limited walk of the gimmick manager's loaded records: vote for a
/// layout while none is latched, then for the name's place under it. Nothing
/// latches on a walk that found fewer than [`MIN_AGREE`] agreeing records -
/// the table may still be loading - so a later call tries again.
fn maybe_probe() {
    let Ok(mut st) = PROBE.try_lock() else { return };
    let every = if st.layout_warned || st.name_warned { PROBE_EVERY_AFTER_WARN } else { PROBE_EVERY };
    if st.last.is_some_and(|t| t.elapsed() < every) {
        return;
    }
    st.last = Some(Instant::now());
    let Some(&slot) = GIMMICK_SLOT_ADDR.get() else { return };
    let Some((count, records)) = manager::view(slot) else { return };
    let n = (count as usize).min(MAX_RECORDS);

    if LAYOUT.get().is_none() {
        let mut vote = LayoutVote::default();
        let mut loaded = 0usize;
        for i in 0..n {
            let Slot::Loaded(rec) = manager::slot(records, i) else { continue };
            loaded += 1;
            vote.observe(rec, |k| crate::collect::family_by_key(k).is_some(), &read_bytes);
            if vote.best() >= ENOUGH {
                break;
            }
        }
        match vote.verdict(MIN_AGREE) {
            Some(l) => {
                let _ = LAYOUT.set(l);
                crate::log!(
                    "[yield] parsed layout: {} (gather records agreeing per known layout {:?}, {loaded} loaded records walked)",
                    l.describe(),
                    vote.hits()
                );
            }
            None if loaded > 0 => {
                st.layout_misses += 1;
                if st.layout_misses >= WARN_AFTER && !st.layout_warned {
                    st.layout_warned = true;
                    crate::log!(
                        "[yield] WARN no known parsed gimmick layout reads {MIN_AGREE} gather records ({loaded} loaded of {count}; votes per known layout {:?}); reading the old one ({}), so gather nodes and ground items may not be recognised",
                        vote.hits(),
                        ParsedLayout::OLD.describe()
                    );
                }
            }
            None => {}
        }
    }

    let Some(&layout) = LAYOUT.get() else { return };
    if NAME_AT.get().is_some() {
        return;
    }
    let mut vote = NameVote::default();
    let mut named = 0usize;
    for i in 0..n {
        let Slot::Loaded(rec) = manager::slot(records, i) else { continue };
        let Some(key) = rec.checked_add(layout.key).and_then(safe::read::<u32>) else { continue };
        let Some(&(_, name, _)) = crate::collect::COLLECT_RECORDS.iter().find(|(k, _, _)| *k == key) else {
            continue;
        };
        named += 1;
        vote.observe(rec, name, &read_bytes);
        if vote.best() >= ENOUGH {
            break;
        }
    }
    match vote.verdict(MIN_AGREE) {
        Some(at) => {
            let _ = NAME_AT.set(at);
            crate::log!("[yield] record name at {} ({named} gather records checked)", at.describe());
        }
        None if named > 0 => {
            st.name_misses += 1;
            if st.name_misses >= WARN_AFTER && !st.name_warned {
                st.name_warned = true;
                crate::log!(
                    "[yield] WARN record name offset not found ({named} gather records checked, offsets 0..0x{:x} under ** and *); ground items cannot be classified",
                    desert_core::gimmick::NAME_PROBE_END
                );
            }
        }
        None => {}
    }
}

/// Item record by index (the u16 an inventory slot stores at +0x08).
pub fn item_record(m: &MainModule, index: u16) -> Option<usize> {
    let mgr = manager_at(m, item_slot()?)?;
    let count: u32 = safe::read(mgr + 8)?;
    if index as u32 >= count {
        return None;
    }
    let arr = safe::read_ptr(mgr + 0x58)?;
    safe::read_ptr(arr + index as usize * 8)
}

/// The default inventory tab id of an item record, `i16 @0x428` through
/// 25477059 (read by the game's cross-tab item counter `FUN_14207F770`, -1 =
/// none). **Unverified on 26361232**: the item record's key and name moved
/// there, so this offset probably did too, and nothing validates it. It
/// feeds only the F11 survey's `tab=` display, which says so; do not build
/// on it before it is re-read.
pub const ITEM_TAB_OFF: usize = 0x428;

/// The item record's `u32` key, through the latched [`ItemLayout`]: `@0`
/// through 25477059, wherever [`maybe_probe_items`] found it on a later
/// build. Until a layout latches it reads the old one.
pub fn item_record_key(rec: usize) -> Option<u32> {
    safe::read(rec.checked_add(item_layout().key)?)
}

/// The item record's name, through the latched [`ItemLayout`]
/// (`**(rec+0x08)` through 25477059).
pub fn item_record_name(rec: usize) -> Option<String> {
    read_name(rec, item_layout().name)
}

/// See [`ITEM_TAB_OFF`]: the pre-26361232 offset, unverified.
pub fn item_record_tab(rec: usize) -> Option<i16> {
    safe::read(rec.checked_add(ITEM_TAB_OFF)?)
}

/// Item record index for a key, by scanning the table (a few thousand reads;
/// call rarely and cache). Reads each key through the latched [`ItemLayout`].
pub fn item_index_by_key(m: &MainModule, key: u32) -> Option<u16> {
    let key_off = item_layout().key;
    let mgr = manager_at(m, item_slot()?)?;
    let count: u32 = safe::read(mgr + 8)?;
    if count == 0 || count >= 0x40000 {
        return None;
    }
    let arr = safe::read_ptr(mgr + 0x58)?;
    (0..count)
        .find(|&i| {
            safe::read_ptr(arr + i as usize * 8)
                .and_then(|rec| rec.checked_add(key_off))
                .and_then(safe::read::<u32>)
                == Some(key)
        })
        .and_then(|i| u16::try_from(i).ok())
}

// ---------------------------------------------------------------------------
// The parsed item record layout, latched once per session
// ---------------------------------------------------------------------------
//
// Through 25477059 an item record was `u32 key @0, **(rec+0x08)` name. On
// 26361232 that reading gave heap-pointer halves for keys (`key=313907456`)
// and no names, which is the shape of the gimmick record's move but is not
// proven to be the same move, so nothing here assumes it: the key offset is
// where several item ids that the loaded gather records pay out are found as
// keys, unique over the table, and the name is the place most records read an
// identifier from, settled by the money record (`Money_Copper`, key 1) when
// two places qualify (`desert_core::gimmick::probe_item_layout`).

/// The layout, once a probe chose one.
static ITEM_LAYOUT: OnceLock<ItemLayout> = OnceLock::new();

struct ItemProbeState {
    last: Option<Instant>,
    misses: u32,
    warned: bool,
}

/// Bookkeeping for the item probe; taken with `try_lock` only.
static ITEM_PROBE: Mutex<ItemProbeState> = Mutex::new(ItemProbeState { last: None, misses: 0, warned: false });

/// Known item ids collected from the gather records before the item probe
/// stops looking for more.
const KNOWN_ITEMS_ENOUGH: usize = 64;

/// The item layout to read through now: the latched one, else
/// [`ItemLayout::OLD`]. Probes (rate-limited, never on the game thread) while
/// unlatched.
fn item_layout() -> ItemLayout {
    if let Some(l) = ITEM_LAYOUT.get() {
        return *l;
    }
    maybe_probe_items();
    ITEM_LAYOUT.get().copied().unwrap_or(ItemLayout::OLD)
}

/// The item ids the loaded gather records pay out, read through the latched
/// gimmick layout (`parsed_outputs`). Empty until that layout latches: an
/// output list read through a wrong layout proves nothing.
fn known_item_ids() -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    // Give the gimmick probe its chance first; it is rate-limited itself.
    let _ = record_layout();
    let Some(&layout) = LAYOUT.get() else { return out };
    let Some(&slot) = GIMMICK_SLOT_ADDR.get() else { return out };
    let Some((count, records)) = manager::view(slot) else { return out };
    for i in 0..(count as usize).min(MAX_RECORDS) {
        let Slot::Loaded(rec) = manager::slot(records, i) else { continue };
        let Some(key) = rec.checked_add(layout.key).and_then(safe::read::<u32>) else { continue };
        if crate::collect::family_by_key(key).is_none() {
            continue;
        }
        if let Some(outputs) = desert_core::gimmick::parsed_outputs(rec, &layout, &read_bytes) {
            out.extend(outputs.iter().map(|o| o.item));
        }
        if out.len() >= KNOWN_ITEMS_ENOUGH {
            break;
        }
    }
    out
}

/// One rate-limited walk of the item manager's loaded records, latching
/// [`ITEM_LAYOUT`] when `probe_item_layout` chooses one. Skipped on the game
/// thread (the skin flush reaches the item readers from the sweep hook), so
/// a full-table walk never lands inside a game frame.
fn maybe_probe_items() {
    if crate::looter::events::game_thread_id() != 0 {
        // SAFETY: `GetCurrentThreadId` takes no arguments and only reads the
        // calling thread's own TEB; it is sound to call from any thread.
        let tid = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        if tid == crate::looter::events::game_thread_id() {
            return;
        }
    }
    let Ok(mut st) = ITEM_PROBE.try_lock() else { return };
    let every = if st.warned { PROBE_EVERY_AFTER_WARN } else { PROBE_EVERY };
    if st.last.is_some_and(|t| t.elapsed() < every) {
        return;
    }
    st.last = Some(Instant::now());
    let Some(&slot) = ITEM_SLOT_ADDR.get() else { return };
    let Some((count, records)) = manager::view(slot) else { return };
    let objs: Vec<usize> = (0..(count as usize).min(MAX_RECORDS))
        .filter_map(|i| match manager::slot(records, i) {
            Slot::Loaded(rec) => Some(rec),
            _ => None,
        })
        .collect();
    if objs.is_empty() {
        return;
    }
    let known = known_item_ids();
    let line = match desert_core::gimmick::probe_item_layout(&objs, &known, &read_bytes) {
        Ok(l) => {
            let _ = ITEM_LAYOUT.set(l);
            Some(format!(
                "[item] record layout: {} ({} loaded item records, {} known item ids from gather records)",
                l.describe(),
                objs.len(),
                known.len()
            ))
        }
        Err(why) => {
            st.misses += 1;
            (st.misses >= WARN_AFTER && !st.warned).then(|| {
                st.warned = true;
                format!(
                    "[item] WARN no item record layout validates ({why}); reading the old one ({}), so bag names and keys, the full-bag check and [skin] names may be wrong",
                    ItemLayout::OLD.describe()
                )
            })
        }
    };
    drop(st);
    if let Some(line) = line {
        crate::log!("{line}");
    }
}

/// Resource outputs of a constructed gimmick record, from the record
/// deserializer (`FUN_141472070` on build 25116796, `FUN_1414711e0` on
/// 25246367). Through 25477059 one vector inline at `record+0x278`, `{ptr
/// data @0, u32 size @8, u32 cap @0xC}`; on 26361232 a pointer at
/// `record+0x128` to that same header. Either way 16-byte elements `{Block*
/// @0, u32 @8}` whose `+8` is zero on gather records, and a block with `u64
/// min @0x20`, `u64 max @0x28` and the item id as the low dword of `u64
/// @0x68`. Read through the latched [`ParsedLayout`]
/// (`desert_core::gimmick::parsed_outputs`).
/// One resource-output block of a gimmick record: a gather pays out exactly one
/// of these, rolled between `min` and `max`. Both are already multiplied if
/// the yield half patched the raw bytes before deserialization.
pub struct DropOutput {
    pub item: u32,
    pub min: u64,
    pub max: u64,
}

/// Every output block of a constructed gimmick record, in vector order.
///
/// Any layout surprise (a bad count, an unreadable element, a nonzero element
/// item that disagrees with its block, an implausible quantity) returns `None` for
/// the whole record rather than a half-right list. `None` is not an error: the
/// caller falls back to the learned-yield path (`events::yield_of`).
pub fn gimmick_record_drops(rec: usize) -> Option<Vec<DropOutput>> {
    let layout = record_layout().0;
    let outputs = desert_core::gimmick::parsed_outputs(rec, &layout, &read_bytes)?;
    Some(outputs.into_iter().map(|o| DropOutput { item: o.item, min: o.min, max: o.max }).collect())
}
