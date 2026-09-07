//! The gimmickinfo record-loader hook: multiply the yield scalars in the raw
//! table bytes just before the game parses each record.
//!
//! ## The game side (build 25116796, from Ghidra)
//!
//! `FUN_1403856b0(mgr, status, idx, stream)` — RVA resolved at runtime by
//! `desert_core::gimmick::resolve_record_loader`, never hard-coded. Two
//! callers reach it (the per-record accessor and a load-everything path) and
//! both only call it when the record is *not* loaded yet
//! (`mgr->records[idx] == null`). It allocates the record object and then
//! calls the deserializer, which reads the record sequentially out of the
//! stream. Our hook runs before that, while the bytes are still raw.
//!
//! `idx` arrives in `r8w`: the upper 48 bits are garbage, so it is masked
//! with `0xFFFF` exactly as the game does (`(param_3 & 0xffff) * 8`).
//!
//! ```text
//! Manager (mgr)
//!   +0x08  u32   record_count
//!   +0x28  ptr   offsets table, 8-byte entries, u32 file_offset at entry+4
//!   +0x58  ptr   array of record_count object pointers (null = not loaded)
//!
//! Stream
//!   +0x10  ptr   buffer: the raw table body, byte-identical to DMM's
//!                gimmickinfo_pabgb_clean.bin
//!   +0x18  u32   size
//!   +0x1c  u32   cursor: at hook time, the start of the record being loaded
//!
//! Record  u32 key, u32 name_len, name, 0x00, ...fields...
//! ```
//!
//! Records are contiguous in index order, so entry[idx+1] ends record idx and
//! the last record ends at the stream size.
//!
//! ## Why the patch happens here and not once
//!
//! The stream buffer is a heap allocation the game frees roughly two seconds
//! after the last load and reallocates on demand. Patching it once would be
//! undone by the next reload; patching per record inside the hook is the only
//! shape that stays correct. It also means nothing is ever cached across
//! calls.
//!
//! ## Thread safety
//!
//! The hook runs on whichever game thread loads records, and several threads
//! may be inside it at once for different indices. There is no shared mutable
//! state here beyond atomics and the [`Config`] in a `OnceLock`. Every write
//! lands in the heap-buffer bytes of the record this call was handed, through
//! `safe::write`. No game function is ever called.
//!
//! Every foreign read goes through `desert_core::safe`, so an unmapped page
//! is a `None` and a silent return, never a fault. A failure at any step
//! leaves the record vanilla; that is always the correct fallback.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

use desert_core::collect;
use desert_core::gimmick::{self, Edit};
use desert_core::safe;

use crate::config::Config;

// Re-exported so `crate::hook::` names both the installer and the callback,
// the way `desert-looter` uses `desert_core::hook` directly.
pub use desert_core::hook::{hex, install, Hook};

/// A single record can never be this big; anything larger means the offsets
/// table is not what we think it is, and we leave the record alone.
pub const MAX_RECORD: usize = 4 * 1024 * 1024;

/// Ceiling on `Debug=1` lines for non-gather records. The table holds ~13,875
/// records, of which 275 are gather records; without a cap the log would be
/// tens of thousands of lines per session.
pub const DEBUG_CAP: u32 = 400;

/// How many blocks are spelled out in a per-record log line before it is
/// truncated. The largest gather record has a handful of output blocks.
const DETAIL_BLOCKS: usize = 8;

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Records the hook was entered for (gather and non-gather alike).
static RECORDS_SEEN: AtomicU64 = AtomicU64::new(0);
/// Gather records whose yields were multiplied (or, in `DryRun`, would have been).
static RECORDS_PATCHED: AtomicU64 = AtomicU64::new(0);
/// Individual u64 scalars written.
static EDITS_WRITTEN: AtomicU64 = AtomicU64::new(0);

/// The "stream cursor is not where the offsets table says" complaint, once.
static CURSOR_MISMATCH_LOGGED: AtomicBool = AtomicBool::new(false);
/// The "record end does not make sense" complaint, once.
static BAD_END_LOGGED: AtomicBool = AtomicBool::new(false);
/// One line with the raw manager/stream values the first time the hook fires,
/// so a log from a new build shows whether the layout still holds.
static FIRST_CALL_LOGGED: AtomicBool = AtomicBool::new(false);
/// Non-gather `Debug` lines emitted so far.
static DEBUG_LINES: AtomicU32 = AtomicU32::new(0);

/// Keys already warned about for a name mismatch. A fixed table of atomics
/// rather than a `Mutex<HashSet>`: the callback must not take a lock a game
/// thread could contend on, and 275 gather records fit comfortably.
const WARN_SLOTS: usize = 256;
static NAME_WARNED: [AtomicU32; WARN_SLOTS] = [const { AtomicU32::new(0) }; WARN_SLOTS];

/// Hand the hook its settings. Called once from `main_thread` before the hook
/// is installed, so the callback always sees a config.
pub fn set_config(cfg: Config) {
    let _ = CONFIG.set(cfg);
}

fn config() -> &'static Config {
    CONFIG.get_or_init(Config::default)
}

/// `(records seen, gather records patched, scalars written)`.
pub fn counters() -> (u64, u64, u64) {
    (
        RECORDS_SEEN.load(Ordering::Relaxed),
        RECORDS_PATCHED.load(Ordering::Relaxed),
        EDITS_WRITTEN.load(Ordering::Relaxed),
    )
}

/// True the first time this key is passed. Best effort: a full table simply
/// stops reporting rather than growing or blocking.
fn first_warning_for(key: u32) -> bool {
    if key == 0 {
        return false;
    }
    let start = (key as usize) % WARN_SLOTS;
    for i in 0..WARN_SLOTS {
        let slot = &NAME_WARNED[(start + i) % WARN_SLOTS];
        match slot.compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(v) if v == key => return false,
            Err(_) => continue,
        }
    }
    false
}

/// The name `desert_core::collect` has on file for a key.
fn table_name(key: u32) -> Option<&'static str> {
    collect::COLLECT_RECORDS.iter().find(|(k, _, _)| *k == key).map(|(_, n, _)| *n)
}

/// `"3->6/5->10, 1->2/2->4"`: one `min->min/max->max` group per output block.
/// `multiply` emits the two scalars of a block adjacent and in offset order,
/// so pairing consecutive edits reconstructs the blocks.
fn describe(edits: &[Edit]) -> String {
    let mut out = String::new();
    for (n, pair) in edits.chunks(2).enumerate() {
        if n == DETAIL_BLOCKS {
            out.push_str(&format!(", +{} more", edits.chunks(2).count() - DETAIL_BLOCKS));
            break;
        }
        if n > 0 {
            out.push_str(", ");
        }
        match pair {
            [lo, hi] => out.push_str(&format!("{}->{}/{}->{}", lo.old, lo.new, hi.old, hi.new)),
            [only] => out.push_str(&format!("{}->{}", only.old, only.new)),
            _ => {}
        }
    }
    out
}

/// Read the record's raw bytes out of the stream buffer.
///
/// Returns `None` for every reason the record must be left alone: a read that
/// failed, an index out of range, a record already loaded, or an offsets table
/// that disagrees with the stream cursor.
fn record_bytes(mgr: usize, idx: usize, stream: usize) -> Option<(usize, Vec<u8>)> {
    let count = safe::read::<u32>(mgr + 0x08)? as usize;
    if !FIRST_CALL_LOGGED.swap(true, Ordering::Relaxed) {
        let size = safe::read::<u32>(stream + 0x18).unwrap_or(0);
        let cursor = safe::read::<u32>(stream + 0x1C).unwrap_or(0);
        let entry = safe::read_ptr(mgr + 0x28)
            .and_then(|t| safe::read::<u32>(t + idx * 8 + 4))
            .unwrap_or(0);
        crate::log!(
            "[gimmick] first call: mgr=0x{mgr:X} count={count} idx={idx} stream=0x{stream:X} \
             buf=0x{:X} size=0x{size:X} cursor=0x{cursor:X} table[idx]=0x{entry:X}",
            safe::read_ptr(stream + 0x10).unwrap_or(0)
        );
    }
    if idx >= count {
        return None;
    }
    // The two callers only reach the loader when the slot is null. If it is
    // not, the game will not parse anything and a patch would be pointless
    // (and, since the buffer may hold a different record's bytes, wrong).
    let records = safe::read_ptr(mgr + 0x58)?;
    if safe::read::<usize>(records + idx * 8)? != 0 {
        return None;
    }

    let buf = safe::read_ptr(stream + 0x10)?;
    let size = safe::read::<u32>(stream + 0x18)? as usize;
    let cursor = safe::read::<u32>(stream + 0x1C)? as usize;
    if cursor >= size {
        return None;
    }

    // entry[idx].file_offset must be exactly where the stream is parked. If it
    // is not, our model of the manager is wrong on this build: say so once and
    // never guess at a record boundary.
    let offsets = safe::read_ptr(mgr + 0x28)?;
    let expected = safe::read::<u32>(offsets + idx * 8 + 4)? as usize;
    if expected != cursor {
        if !CURSOR_MISMATCH_LOGGED.swap(true, Ordering::Relaxed) {
            crate::log!(
                "[gimmick] idx {idx}: stream cursor 0x{cursor:X} != offsets table 0x{expected:X}; \
                 not patching anything (logged once)"
            );
        }
        return None;
    }

    let end = if idx + 1 < count {
        safe::read::<u32>(offsets + (idx + 1) * 8 + 4)? as usize
    } else {
        size
    };
    if !(cursor < end && end <= size && end - cursor <= MAX_RECORD) {
        if !BAD_END_LOGGED.swap(true, Ordering::Relaxed) {
            crate::log!(
                "[gimmick] idx {idx}: record end 0x{end:X} not in (0x{cursor:X}, 0x{size:X}]; \
                 not patching this record (logged once)"
            );
        }
        return None;
    }

    let mut bytes = vec![0u8; end - cursor];
    if !safe::read_into(buf + cursor, &mut bytes) {
        return None;
    }
    Some((buf + cursor, bytes))
}

/// Installed on `FUN_1403856b0`. Runs before the deserializer, on a game thread.
///
/// # Safety
/// Called from the trampoline stub with the hooked function's first four
/// integer arguments. It only reads through `safe` and only writes into the
/// stream buffer bytes of the record it was handed.
pub unsafe extern "system" fn on_record_load(mgr: usize, status: usize, idx: usize, stream: usize) {
    // `status` is the loader's out-parameter; we neither read nor touch it.
    let _ = status;
    // r8w: the upper bits are whatever was in r8, exactly as the game masks it.
    let idx = idx & 0xFFFF;
    RECORDS_SEEN.fetch_add(1, Ordering::Relaxed);

    let cfg = config();
    let Some((at, bytes)) = record_bytes(mgr, idx, stream) else { return };
    let Some(header) = gimmick::parse_header(&bytes) else { return };

    let Some(family) = collect::family_by_key(header.key) else {
        if cfg.debug && DEBUG_LINES.fetch_add(1, Ordering::Relaxed) < DEBUG_CAP {
            crate::log!(
                "[gimmick] idx {idx} key {} {:?}: not a gather record",
                header.key, header.name
            );
        }
        return;
    };

    // A key whose name moved means the table was regenerated under us. Worth
    // knowing, but the key is what identifies the record, so carry on.
    if let Some(expected) = table_name(header.key) {
        if !expected.eq_ignore_ascii_case(&header.name) && first_warning_for(header.key) {
            crate::log!(
                "[gimmick] key {}: collect table says {expected:?}, the game says {:?}; continuing",
                header.key, header.name
            );
        }
    }

    let mult = cfg.multiplier(family);
    if mult <= 1 {
        return;
    }

    let edits = gimmick::multiply(&bytes, mult);
    if edits.is_empty() {
        crate::log!(
            "[gimmick] {} (key {}, {family:?}) x{mult}: no output lists",
            header.name, header.key
        );
        return;
    }
    let blocks = edits.len().div_ceil(2);
    let detail = describe(&edits);

    if cfg.dry_run {
        RECORDS_PATCHED.fetch_add(1, Ordering::Relaxed);
        crate::log!(
            "[dry] {} key={} {family:?} x{mult} blocks={blocks} would write {}: {detail}",
            header.name,
            header.key,
            edits.len()
        );
        return;
    }

    let mut applied = 0usize;
    for e in &edits {
        if safe::write(at + e.offset, e.new) {
            applied += 1;
        }
    }
    RECORDS_PATCHED.fetch_add(1, Ordering::Relaxed);
    EDITS_WRITTEN.fetch_add(applied as u64, Ordering::Relaxed);
    crate::log!(
        "[gimmick] {} key={} {family:?} x{mult} blocks={blocks} applied {applied}/{}: {detail}",
        header.name,
        header.key,
        edits.len()
    );
}
