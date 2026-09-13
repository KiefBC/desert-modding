//! The gimmickinfo record-loader hook: multiply the yield scalars in the raw
//! table bytes just before the game parses each record.
//!
//! ## The game side (build 25246367, from Ghidra)
//!
//! `FUN_140385cd0(mgr, status, idx, stream)` — RVA resolved at runtime by
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
//! ## The live path
//!
//! The loader runs once per session (the game's preload pass, ~9 s after
//! launch, `docs/reference-internals.md` section 16), so an ini change after
//! that has no record left to intercept. [`reapply`] is the answer: the
//! callback hands every gather record's vanilla blocks to [`crate::remember`]
//! on the way past, whatever the ini says at the time, and `reapply` — on the
//! plugin's own thread, after a change — walks those and rewrites the
//! *parsed* objects to vanilla times the current multiplier. That is the one
//! place this plugin writes into a game object rather than into bytes the
//! game has not read yet, so every step of it is checked against what was
//! remembered before anything is written.
//!
//! ## Thread safety
//!
//! The hook runs on whichever game thread loads records, and several threads
//! may be inside it at once for different indices. There is no shared mutable
//! state here beyond atomics: the counters below, the manager pointer, the
//! lock-free table in `crate::remember` this writes and the plugin thread
//! reads, and, in `crate::config`, the
//! [`LiveConfig`](crate::config::LiveConfig) the ini-reload loop on the
//! plugin's main thread publishes to and this hook only ever reads. Every
//! write from the callback lands in the heap-buffer bytes of the record this
//! call was handed, through `safe::write`. No game function is ever called,
//! from the callback or from [`reapply`].
//!
//! Every foreign read goes through `desert_core::safe`, so an unmapped page
//! is a `None` and a silent return, never a fault. A failure at any step
//! leaves the record vanilla; that is always the correct fallback.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use desert_core::collect::{self, Family};
use desert_core::gimmick::{self, Edit};
use desert_core::manager;
use desert_core::safe;

use crate::config::LIVE;
use crate::remember;

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

/// The record manager the loader was last called with. A process singleton
/// (one gimmickinfo table), stored on every call so [`reapply`] can find the
/// parsed records long after the load pass is over. 0 = the hook has never
/// fired, so there is nothing loaded to re-apply to.
static MANAGER: AtomicUsize = AtomicUsize::new(0);

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
        // `(start + i) % WARN_SLOTS` is always in range; a miss simply moves
        // on to the next probe, which is what the loop does anyway.
        let Some(slot) = NAME_WARNED.get((start + i) % WARN_SLOTS) else { continue };
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
    // The offsets are `desert_core::manager`'s, the reads are not: this function
    // must read the count *before* the first-call diagnostic below and the
    // records array *after* it, and it wants the `records[idx] == 0` reading
    // rather than the object, so `manager::view_of` and `manager::slot` would
    // both change what happens here. Only the two strides are shared.
    let count = safe::read::<u32>(mgr + manager::MGR_COUNT)? as usize;
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
    let records = safe::read_ptr(mgr + manager::MGR_RECORDS)?;
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

/// Installed on `FUN_140385cd0`. Runs before the deserializer, on a game thread.
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
    // The manager is a process singleton and this is the only place its
    // address is ever seen. `reapply` needs it after the load pass is over.
    MANAGER.store(mgr, Ordering::Relaxed);

    let Some((at, bytes)) = record_bytes(mgr, idx, stream) else { return };
    let Some(header) = gimmick::parse_header(&bytes) else { return };

    let Some(family) = collect::family_by_key(header.key) else {
        if LIVE.debug() && DEBUG_LINES.fetch_add(1, Ordering::Relaxed) < DEBUG_CAP {
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

    // Before any gate, and for every gather record: these bytes are the only
    // place the vanilla yields exist, they are freed a couple of seconds after
    // the load pass, and `reapply` cannot scale a number it never saw. That
    // holds even at `Enabled=0` and 1x - both of those can be turned up later,
    // and this is the only chance to record what to turn up from.
    remember::remember(header.key, idx as u16, &gimmick::output_blocks(&bytes));

    // `Enabled=0` writes nothing. The reads above already happened; they cost
    // one guarded copy of a record and are what keeps a later `Enabled=1` (or
    // a raised multiplier) able to do anything at all. Installing the hook
    // later is not an option either: the loader prologue can only be patched
    // before the table starts loading, so `LiveConfig::enabled` is the only
    // thing deciding whether this writes.
    if !LIVE.enabled() {
        return;
    }

    let mult = LIVE.multiplier(family);
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

    if LIVE.dry_run() {
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

// ---------------------------------------------------------------------------
// The live path: re-apply the current multipliers to the already parsed records
// ---------------------------------------------------------------------------

/// Where the yields live in a *parsed* record, as opposed to the raw bytes the
/// callback edits. Established by decompile and disassembly of build 25116796
/// and written up in `docs/reference-internals.md` section 16.
///
/// ```text
/// mgr+0x08    u32   record_count
/// mgr+0x58    ptr   array of record pointers (null = not loaded)
/// rec+0x08    u32   key, the same one the raw record header carries
/// rec+0x278   ptr   output list data: entries of 16 bytes
/// rec+0x280   u32   entry count
/// entry+0x00  ptr   block object (0x70 bytes), null when the disk flag was 0
/// entry+0x08  u32   _dropTagNameHash (raw block +64); ZERO on gather records
/// block+0x20  u64   MIN   (raw block +42)
/// block+0x28  u64   MAX   (raw block +50)
/// block+0x68  u32   item id (raw block +1, echoed at +60)
/// ```
///
/// `rec+0x288` holds one further optional block of the same type with no
/// count. The raw scanner has never touched it, so neither does this: the two
/// paths must produce the same yields or a session's numbers would depend on
/// when the ini was last edited.
mod parsed {
    // The manager pair itself is not here: `mgr+0x08` and `mgr+0x58` are
    // `desert_core::manager`'s MGR_COUNT and MGR_RECORDS, which
    // `desert_dispatch` reads the faction-node and dropset managers through as
    // well. One definition, so a stride cannot be right in one subsystem and
    // wrong in the other.
    pub const REC_KEY: usize = 0x08;
    pub const REC_LIST: usize = 0x278;
    pub const REC_COUNT: usize = 0x280;
    /// Bytes per list entry.
    pub const ENTRY: usize = 16;
    /// The list entry's own key field, which `FUN_1414a7cc0` reads as the four
    /// bytes *after* each 64-byte block body — disk `+64`.
    ///
    /// **It is zero on every gather record**, measured over all 896 output
    /// blocks in DMM's clean table body, so it identifies nothing and must not
    /// be compared against an item id. It is still read, as a structural
    /// canary: the 142 blocks in the table that do carry a nonzero value here
    /// are chests, dig sites and dungeon loot, and a gather record growing one
    /// means the record shape moved. See `docs/reference-internals.md` §16.
    pub const ENTRY_ITEM: usize = 8;
    pub const BLOCK_MIN: usize = 0x20;
    pub const BLOCK_MAX: usize = 0x28;
    /// The block's item id.
    ///
    /// **Corrected 2026-09-12 from `0x6C`, which is always zero.** The block
    /// parser's first read (`FUN_141a37180`) takes eight bytes from disk `+1`
    /// into `block+0x68`, so the low dword at `0x68` is the item id (disk `+1`)
    /// and the high dword at `0x6C` is disk `+5` — a pad that is zero on all
    /// 1038 blocks in the table. §16 named `0x6C` the item id from the same
    /// off-by-four reading that put the disk-side item at `+5` instead of `+1`;
    /// both are fixed together, and neither can be tested natively because
    /// these are offsets into memory the game parsed.
    pub const BLOCK_ITEM: usize = 0x68;
}

/// What one [`reapply`] pass did, for the single summary line its caller logs.
///
/// The skip counters are the point of the struct: every one of them is a place
/// where the parsed layout above stopped matching what we remembered, which is
/// the first thing a game update breaks. A pass that skips nothing prints one
/// line; a pass that skips anything prints the reasons too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// False when the hook has never fired, so nothing is loaded yet.
    pub manager_known: bool,
    /// True when the manager itself would not read: the pointer is stale or
    /// the layout moved, and nothing at all was attempted.
    pub manager_unreadable: bool,
    /// Remembered records walked.
    pub visited: usize,
    /// Records at least one scalar was written for (would have been, in DryRun).
    pub rewritten: usize,
    /// Records already carrying exactly the wanted numbers.
    pub unchanged: usize,
    /// Records nothing was attempted on; the sum of the five reasons below.
    pub skipped: usize,
    /// `u64` yields written (would have been, in DryRun).
    pub scalars: usize,
    /// Blocks skipped inside records that were otherwise fine.
    pub blocks_skipped: usize,

    /// The remembered index is past the manager's record count.
    pub skip_index: usize,
    /// The record's slot is still null: the game never loaded it.
    pub skip_unloaded: usize,
    /// The object at that slot carries a different key than we remembered.
    pub skip_key: usize,
    /// The parsed list has a different number of entries than the raw record had.
    pub skip_count: usize,
    /// The record has no output list pointer at all.
    pub skip_list: usize,
    /// A guarded read of the record failed (an unmapped page, a stale pointer).
    pub skip_read: usize,

    /// The list entry's block pointer is null (the disk flag byte was 0).
    pub block_null: usize,
    /// The block's item id is not the one the raw block had, or the list
    /// entry's key field is not the zero every gather record carries there.
    pub block_item: usize,
    /// A guarded read of the block failed.
    pub block_read: usize,
    /// The write itself failed, having read the same address a moment earlier.
    pub block_write: usize,
}

impl Outcome {
    /// The hook has never fired: no manager, nothing loaded, nothing to say.
    pub const NOT_LOADED: Self = Self::empty(false);

    const fn empty(manager_known: bool) -> Self {
        Outcome {
            manager_known,
            manager_unreadable: false,
            visited: 0,
            rewritten: 0,
            unchanged: 0,
            skipped: 0,
            scalars: 0,
            blocks_skipped: 0,
            skip_index: 0,
            skip_unloaded: 0,
            skip_key: 0,
            skip_count: 0,
            skip_list: 0,
            skip_read: 0,
            block_null: 0,
            block_item: 0,
            block_read: 0,
            block_write: 0,
        }
    }

    /// `"82 records rewritten, 193 unchanged, 0 skipped; 644 scalars written"`.
    pub fn summary(&self) -> String {
        format!(
            "{} records rewritten, {} unchanged, {} skipped; {} scalars written",
            self.rewritten, self.unchanged, self.skipped, self.scalars
        )
    }

    /// The reasons anything was skipped, or `None` when nothing was.
    ///
    /// This is the line that says a game update moved the parsed layout: in a
    /// healthy session every remembered record is loaded, keyed and shaped
    /// exactly as it was on disk, so nothing is skipped at all.
    pub fn warning(&self) -> Option<String> {
        if self.manager_unreadable {
            return Some("the record manager is unreadable; nothing was re-applied".to_string());
        }
        if self.skipped == 0 && self.blocks_skipped == 0 {
            return None;
        }
        let mut why: Vec<String> = Vec::new();
        for (n, what) in [
            (self.skip_index, "index past the record count"),
            (self.skip_unloaded, "not loaded"),
            (self.skip_key, "key mismatch"),
            (self.skip_count, "block count mismatch"),
            (self.skip_list, "no output list"),
            (self.skip_read, "unreadable record"),
            (self.block_null, "null block"),
            (self.block_item, "item mismatch"),
            (self.block_read, "unreadable block"),
            (self.block_write, "write refused"),
        ] {
            if n > 0 {
                why.push(format!("{what} {n}"));
            }
        }
        Some(format!(
            "{} of {} records and {} blocks skipped ({}); the parsed record layout may have \
             moved in this game build",
            self.skipped,
            self.visited,
            self.blocks_skipped,
            why.join(", ")
        ))
    }
}

/// A pointer the game could really have handed us: non-null, inside user
/// address space, and far enough below the top that adding a struct offset to
/// it cannot wrap a `usize`. Everything [`reapply`] reads out of the game goes
/// through this before it is used as a base, so none of the `base + offset`
/// expressions there can overflow — which `safe::read` would refuse anyway,
/// but only after the addition had already happened.
///
/// `desert_core::manager::plausible` under the name this file has always used.
/// It was byte-identical to `desert_dispatch`'s copy, and a bound that differed
/// between two subsystems walking the same managers would be a bug in whichever
/// one was looser.
use desert_core::manager::plausible;

/// The multiplier a remembered record should be at right now: its family's
/// live value, or 1 when `Enabled=0` or the key is not a gather record after
/// all. Never 0 — that would zero the record's yields.
fn wanted_multiplier(key: u32, enabled: bool) -> (Option<Family>, u64) {
    let family = collect::family_by_key(key);
    match family {
        Some(f) if enabled => (family, u64::from(LIVE.multiplier(f).max(1))),
        _ => (family, 1),
    }
}

/// Rewrite the already parsed records so their yields are the vanilla ones
/// times the multiplier the ini says *now*.
///
/// Called on the plugin's main thread after an ini change, never from the
/// callback. The game reads its gimmickinfo table once per session, so without
/// this a change made while playing has nothing to act on until the next
/// launch; with it, the numbers move on the next gather.
///
/// Every value written is `remembered × current`, never `current × something`:
/// the vanilla blocks `crate::remember` holds are the only fixed point, and
/// scaling what is already there would compound every edit. A block is only
/// written when the record's key and the block's item id both still match what
/// was remembered, so a record the game has replaced or a layout that moved is
/// skipped rather than corrupted. Every read is a guarded `safe::read`, every
/// write a guarded `safe::write::<u64>`, and no game function is called.
pub fn reapply() -> Outcome {
    use parsed::*;

    let mgr = MANAGER.load(Ordering::Relaxed);
    if mgr == 0 {
        // The hook has not fired yet: the table is not loaded, and the load
        // path will apply the current multipliers when it is.
        return Outcome::NOT_LOADED;
    }
    let mut out = Outcome::empty(true);
    if !plausible(mgr) {
        out.manager_unreadable = true;
        return out;
    }
    let enabled = LIVE.enabled();
    let dry = LIVE.dry_run();
    let debug = LIVE.debug();

    // `manager::view_of` is the same three steps this block used to spell out:
    // read the `u32` count at MGR_COUNT, read the array pointer at MGR_RECORDS,
    // and require it to be `plausible` - so `records + idx * 8`, with a `u16`
    // index adding at most 0x7FFF8, cannot wrap. Every failure lands on the same
    // `manager_unreadable` flag it did before. The one difference is that a
    // count that will not read no longer attempts the array read beside it,
    // which nothing can observe: `safe::read` has no effect but its answer.
    let Some((count, records)) = manager::view_of(mgr) else {
        out.manager_unreadable = true;
        return out;
    };
    let count = count as usize;

    for rec in remember::snapshot() {
        out.visited += 1;
        let idx = rec.idx as usize;
        if idx >= count {
            out.skipped += 1;
            out.skip_index += 1;
            continue;
        }
        // The same three readings this block used to spell out, in the same
        // order - read, null, `plausible` - and charged to the same two
        // counters. `manager::slot` is where `desert_dispatch` walks its two
        // managers through as well.
        let obj = match manager::slot(records, idx) {
            manager::Slot::Loaded(p) => p,
            manager::Slot::Empty => {
                // The slot was never filled: the lazy loader will fill it
                // through the hook, which applies the current multiplier itself.
                out.skipped += 1;
                out.skip_unloaded += 1;
                continue;
            }
            manager::Slot::Unreadable => {
                out.skipped += 1;
                out.skip_read += 1;
                continue;
            }
        };
        match safe::read::<u32>(obj + REC_KEY) {
            Some(k) if k == rec.key => {}
            Some(_) => {
                out.skipped += 1;
                out.skip_key += 1;
                continue;
            }
            None => {
                out.skipped += 1;
                out.skip_read += 1;
                continue;
            }
        }
        let (Some(n), Some(data)) =
            (safe::read::<u32>(obj + REC_COUNT), safe::read::<usize>(obj + REC_LIST))
        else {
            out.skipped += 1;
            out.skip_read += 1;
            continue;
        };
        if data == 0 {
            out.skipped += 1;
            out.skip_list += 1;
            continue;
        }
        if !plausible(data) {
            out.skipped += 1;
            out.skip_read += 1;
            continue;
        }
        if n as usize != rec.blocks.len() {
            // The parsed list is not the list we read off the disk bytes, so
            // entry i is not block i and nothing here can be trusted.
            out.skipped += 1;
            out.skip_count += 1;
            continue;
        }

        let (family, mult) = wanted_multiplier(rec.key, enabled);
        let mut wrote = 0usize;
        let mut detail = String::new();
        let mut shown = 0usize;

        for (i, &(item, min, max)) in rec.blocks.iter().enumerate() {
            let entry = data + i * ENTRY;
            let Some(block) = safe::read::<usize>(entry) else {
                out.blocks_skipped += 1;
                out.block_read += 1;
                continue;
            };
            if block == 0 {
                out.blocks_skipped += 1;
                out.block_null += 1;
                continue;
            }
            if !plausible(block) {
                out.blocks_skipped += 1;
                out.block_read += 1;
                continue;
            }
            let (Some(entry_item), Some(block_item)) =
                (safe::read::<u32>(entry + ENTRY_ITEM), safe::read::<u32>(block + BLOCK_ITEM))
            else {
                out.blocks_skipped += 1;
                out.block_read += 1;
                continue;
            };
            // The block's item id has to be the one the raw block carried, or
            // this is not the block we remembered. `entry_item` is not a second
            // copy of it - it is the list entry's own key field, zero on every
            // gather record (see `parsed::ENTRY_ITEM`) - so it is checked
            // against the zero it should be rather than against `item`.
            // Comparing it to `item` is what the pre-2026-09-12 code did, and
            // it only ever passed because `item` was itself being read from a
            // pad and was also zero.
            if block_item != item || entry_item != 0 {
                out.blocks_skipped += 1;
                out.block_item += 1;
                continue;
            }
            // The same saturating arithmetic `gimmick::multiply` does, so the
            // live path and the load path produce identical numbers.
            let (want_min, want_max) = (min.saturating_mul(mult), max.saturating_mul(mult));
            let (Some(cur_min), Some(cur_max)) =
                (safe::read::<u64>(block + BLOCK_MIN), safe::read::<u64>(block + BLOCK_MAX))
            else {
                out.blocks_skipped += 1;
                out.block_read += 1;
                continue;
            };
            if cur_min == want_min && cur_max == want_max {
                continue;
            }
            if !dry {
                // Both writes are attempted: the pair was just read, so a
                // refusal here means the page went away between the two, and
                // half a block is still better reported than retried.
                //
                // `desert_dispatch::apply` counts its own min/max pair the same
                // way (`usize::from(write(..)) + ..`, then `ok < 2`) and the
                // idiom is deliberately not shared: that one is *ordered* by
                // `node::amount_write_order` and stops on the first refusal,
                // this one is unordered and attempts both, so a helper would
                // take the difference as a parameter to save two lines.
                let ok = usize::from(safe::write(block + BLOCK_MIN, want_min))
                    + usize::from(safe::write(block + BLOCK_MAX, want_max));
                if ok < 2 {
                    out.blocks_skipped += 1;
                    out.block_write += 1;
                }
                if ok == 0 {
                    continue;
                }
                wrote += ok;
            } else {
                wrote += 2;
            }
            if shown < DETAIL_BLOCKS {
                if shown > 0 {
                    detail.push_str(", ");
                }
                detail.push_str(&format!("{cur_min}->{want_min}/{cur_max}->{want_max}"));
            }
            shown += 1;
        }

        if wrote == 0 {
            out.unchanged += 1;
            continue;
        }
        out.rewritten += 1;
        out.scalars += wrote;
        if !dry {
            EDITS_WRITTEN.fetch_add(wrote as u64, Ordering::Relaxed);
        }
        if debug {
            if shown > DETAIL_BLOCKS {
                detail.push_str(&format!(", +{} more", shown - DETAIL_BLOCKS));
            }
            let name = table_name(rec.key).unwrap_or("?");
            let fam = family.map_or_else(|| "?".to_string(), |f| format!("{f:?}"));
            crate::log!(
                "[{}] {name} key={} {fam} x{mult} blocks={} {} {wrote}: {detail}",
                if dry { "dry" } else { "live" },
                rec.key,
                rec.blocks.len(),
                if dry { "would write" } else { "wrote" }
            );
        }
    }
    out
}
