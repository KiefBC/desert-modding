//! Name lookup in the game's `*InfoManager` data tables, following the
//! traversal the reference mod uses (`FUN_1800151f0`, whose argument is the
//! global slot, so `*slot` is the manager object):
//!   manager+8 = u32 count (< 0x40000); manager+0x58 = record**;
//!   record+8 -> ptr -> ptr -> NUL-terminated name (3+ printable chars).
//!
//! The two global pointer slots this file needs — `iteminfo` and `gimmickinfo`
//! — are not constants here. [`resolve_slots`] asks
//! [`desert_core::gimmick::resolve_manager_slot`] for each of them once at
//! startup, which finds the `mov rbx,[rip+disp]` in front of the accessor copy
//! that names the table, so nothing in this file is a build-specific address.

use std::sync::OnceLock;

use crate::module::MainModule;
use crate::safe;

/// RVAs of the two manager slots, filled in once by [`resolve_slots`]. Empty
/// means the scan failed for that table and every lookup through it declines.
static ITEM_INFO_SLOT: OnceLock<usize> = OnceLock::new();
static GIMMICK_INFO_SLOT: OnceLock<usize> = OnceLock::new();

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
    for (name, table, cell) in [
        ("iteminfo", desert_core::gimmick::ITEM_TABLE, &ITEM_INFO_SLOT),
        ("gimmickinfo", desert_core::gimmick::GIMMICK_TABLE, &GIMMICK_INFO_SLOT),
    ] {
        match desert_core::gimmick::resolve_manager_slot(m.bytes(), table) {
            Ok(rva) => {
                crate::log!("[slot] {name:<22} = +0x{rva:X}");
                // Runs once, so a second `set` cannot happen; ignoring it is
                // still the only sane answer if it ever did.
                let _ = cell.set(rva);
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

pub fn name(m: &MainModule, slot_rva: usize, id: u32) -> Option<String> {
    let inner = manager_at(m, slot_rva)?;
    let n: u32 = safe::read(inner + 8)?;
    if n == 0 || n >= 0x40000 || id >= n {
        return None;
    }
    let arr = safe::read_ptr(inner + 0x58)?;
    let rec = safe::read_ptr(arr + id as usize * 8)?;
    let p = safe::read_ptr(rec + 8)?;
    let s = safe::read_ptr(p)?;
    safe::read_cstr(s, 64).filter(|s| s.len() >= 3)
}

/// Gimmick records: `+0x08 u32 key`, `+0x10 ptr` (name path under investigation).
pub const GIMMICK_KEY_OFF: usize = 0x08;

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

pub fn gimmick_record_key(rec: usize) -> Option<u32> {
    safe::read(rec + GIMMICK_KEY_OFF)
}

/// `**(rec+0x10)`
pub fn gimmick_record_name(rec: usize) -> Option<String> {
    let p = safe::read_ptr(rec + 0x10)?;
    let q = safe::read_ptr(p)?;
    safe::read_cstr(q, 64).filter(|s| s.len() >= 3)
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

/// Item records: `u32 key @0`, name `**(rec+0x08)`, and the default inventory
/// tab id as `i16 @0x428` (read by the game's cross-tab item counter
/// `FUN_14207F770`, -1 = none).
pub const ITEM_TAB_OFF: usize = 0x428;

pub fn item_record_key(rec: usize) -> Option<u32> {
    safe::read(rec)
}

pub fn item_record_name(rec: usize) -> Option<String> {
    let p = safe::read_ptr(rec + 8)?;
    let q = safe::read_ptr(p)?;
    safe::read_cstr(q, 64).filter(|s| s.len() >= 3)
}

pub fn item_record_tab(rec: usize) -> Option<i16> {
    safe::read(rec + ITEM_TAB_OFF)
}

/// Item record index for a key, by scanning the table (a few thousand reads;
/// call rarely and cache).
pub fn item_index_by_key(m: &MainModule, key: u32) -> Option<u16> {
    let mgr = manager_at(m, item_slot()?)?;
    let count: u32 = safe::read(mgr + 8)?;
    if count == 0 || count >= 0x40000 {
        return None;
    }
    let arr = safe::read_ptr(mgr + 0x58)?;
    (0..count).find(|&i| {
        safe::read_ptr(arr + i as usize * 8).and_then(safe::read::<u32>) == Some(key)
    }).map(|i| i as u16)
}

/// Resource outputs of a constructed gimmick record, from the record
/// deserializer - `FUN_141472070` on build 25116796, `FUN_1414711e0` on
/// 25246367, and every offset below survived that move untouched: one vector at
/// `record+0x278` — `{ptr data @0, u32 size @8, u32 cap @0xC}`, elements of 16
/// bytes `{Block* @0, u32 item id @8}`; the block (112 bytes) has `u64 min @0x20`,
/// `u64 max @0x28` and the item id again as the high dword of `u64 @0x68`.
pub const GIMMICK_DROPS_OFF: usize = 0x278;

/// One resource-output block of a gimmick record: a gather pays out exactly one
/// of these, rolled between `min` and `max`. Both are already multiplied if
/// Desert Gatherer patched the raw bytes before deserialization.
pub struct DropOutput {
    pub item: u32,
    pub min: u64,
    pub max: u64,
}

/// Every output block of a constructed gimmick record, in vector order.
///
/// Any layout surprise — a bad count, an unreadable element, a block whose item
/// id does not match the element's, an implausible quantity — returns `None` for
/// the whole record rather than a half-right list. `None` is not an error: the
/// caller falls back to the learned-yield path (`events::yield_of`).
pub fn gimmick_record_drops(rec: usize) -> Option<Vec<DropOutput>> {
    let data = safe::read_ptr(rec + GIMMICK_DROPS_OFF)?;
    let size: u32 = safe::read(rec + GIMMICK_DROPS_OFF + 0x08)?;
    let cap: u32 = safe::read(rec + GIMMICK_DROPS_OFF + 0x0C)?;
    if size == 0 || size > cap || size > desert_core::gimmick::MAX_COUNT {
        return None;
    }
    let n = size as usize;
    if !safe::readable(data, n * 16) {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let elem = data + i * 16;
        let item: u32 = safe::read(elem + 8)?;
        if item == 0 {
            return None;
        }
        let block = safe::read_ptr(elem)?;
        let min: u64 = safe::read(block + 0x20)?;
        let max: u64 = safe::read(block + 0x28)?;
        let block_item: u32 = safe::read(block + 0x6C)?;
        if block_item != item || min == 0 || min > max || max > desert_core::gimmick::MAX_QTY {
            return None;
        }
        out.push(DropOutput { item, min, max });
    }
    Some(out)
}
