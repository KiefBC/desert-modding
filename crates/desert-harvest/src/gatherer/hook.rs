//! The gimmickinfo record-loader hook: multiply the yield scalars in the raw
//! table bytes just before the game parses each record.
//!
//! ## The game side (build 25246367, from Ghidra)
//!
//! `FUN_140385cd0(mgr, status, idx, stream)` (`FUN_14038ba40`, `+0x38BA40`,
//! on build 25477059), its RVA resolved at runtime by
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
//! Stream (StreamLayout::NEW on 26361232; StreamLayout::OLD through 25477059)
//!   +0x20  ptr   buffer: the raw table body, byte-identical to the archive's (+0x10)
//!                gimmickinfo.staticinfobody
//!   +0x28  u32   size                                                     (+0x18)
//!   +0x2c  u32   cursor: at hook time, the start of the record being     (+0x1c)
//!                loaded
//!
//! Record  through 25477059 (HeaderShape::KeyFirst):
//!           u32 key, u32 name_len, name, 0x00, ...fields...
//!         on 26361232 (HeaderShape::NameFirst):
//!           u32 name_len, name, u32 key, 0x00, u32 path_len, path, u8 flag, ...fields...
//! ```
//!
//! The header shape is latched from the first record that validates as
//! exactly one shape and logged once (`[gimmick] record header: ...`). On the
//! first 26361232 launch the old parser rejected every record without a word,
//! as the old stream offsets had before it; so every silent drop is now
//! counted on the `[stat]` line, and a path that eats every record says so
//! once with a `WARN`.
//!
//! ## The stream layout is parsed, not assumed
//!
//! The stream is an object the accessor builds on its own stack just before
//! the loader call, and on 26361232 it grew two zeroed qwords in front of the
//! buffer pointer, moving all three fields by 0x10. With the old offsets
//! hard-coded the hook read a zero buffer and a zero size on every call, so
//! every record failed the size check and the whole half was silently inert
//! for that build, dump included. So the offsets are now read off the
//! accessor's own fill sequence at install time
//! (`desert_core::gimmick::AccessorSites::stream_layout`) and handed to
//! [`set_stream_layout`] before the hook goes in. If that parse fails, the
//! first call probes the known layouts against the live stream instead
//! ([`desert_core::gimmick::choose_layout`]: a non-null buffer, a size in
//! range, a cursor equal to the offsets table's entry), and if neither
//! validates the hook stays inert for the session and says so once.
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
//! callback hands every gather record's vanilla blocks to [`crate::gatherer::remember`]
//! on the way past, whatever the ini says at the time, and `reapply` (on the
//! plugin's own thread, after a change) walks those and rewrites the
//! *parsed* objects to vanilla times the current multiplier. That is the one
//! place this plugin writes into a game object rather than into bytes the
//! game has not read yet, so every step of it is checked against what was
//! remembered before anything is written.
//!
//! ## The table dump
//!
//! `[Harvest] DumpTable=1` (with `DryRun=1`) has the first call copy the
//! whole buffer - the table body, `size` bytes from the stream's buffer
//! pointer - and hand it to
//! the plugin's own thread, which writes it beside the log as
//! [`crate::gatherer::dump::FILE_NAME`], a diagnostic copy of what the game
//! loaded (the offline tools read the game's archives). The copy has to be taken here for the same reason the
//! patch does: the buffer is gone a couple of seconds after the last load.
//! The decision itself is [`crate::gatherer::dump::dump_decision`], pure and tested
//! natively; see [`PENDING_DUMP`] for the hand-off.
//!
//! ## Thread safety
//!
//! The hook runs on whichever game thread loads records, and several threads
//! may be inside it at once for different indices. There is no shared mutable
//! state here beyond atomics - the counters below, the manager pointer, the
//! lock-free table in `crate::gatherer::remember` this writes and the plugin thread
//! reads, and, in `crate::gatherer::config`, the
//! [`LiveConfig`](crate::gatherer::config::LiveConfig) the ini-reload loop on the
//! plugin's main thread publishes to and this hook only ever reads - with one
//! deliberate exception: [`PENDING_DUMP`], a `Mutex` the hook touches at most
//! once per process and only with `try_lock`; its doc comment says why that is
//! not the lock the rule is about. Every write from the callback lands in the
//! heap-buffer bytes of the record this call was handed, through
//! `safe::write`. No game function is ever called, from the callback or from
//! [`reapply`].
//!
//! Every foreign read goes through `desert_core::safe`, so an unmapped page
//! is a `None` and a silent return, never a fault. A failure at any step
//! leaves the record vanilla; that is always the correct fallback.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use desert_core::collect::{self, Family};
use desert_core::gimmick::{self, Edit, HeaderShape, StreamLayout, StreamReading};
use desert_core::manager;
use desert_core::safe;

use crate::gatherer::config::LIVE;
use crate::gatherer::dump::{self, DumpDecision};
use crate::gatherer::remember;

// Re-exported so `crate::gatherer::hook::` names both the installer and the callback,
// the way the auto-loot half uses `crate::hook` (the core module) directly.
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
/// Records whose raw bytes could not be read out of the stream: no layout,
/// a failed read, a cursor that disagrees with the offsets table, an end that
/// makes no sense. Records already loaded (the loader's own early-out) are
/// not counted: nothing is wrong with those.
static STREAM_UNREAD: AtomicU64 = AtomicU64::new(0);
/// Records whose raw bytes were read.
static STREAM_READ: AtomicU64 = AtomicU64::new(0);
/// Records read whose header did not parse in the latched shape.
static HEADERS_UNPARSED: AtomicU64 = AtomicU64::new(0);
/// Records read whose header parsed.
static HEADERS_PARSED: AtomicU64 = AtomicU64::new(0);
/// Records whose key `collect` knows as a gather record.
static RECORDS_RECOGNISED: AtomicU64 = AtomicU64::new(0);
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
/// The "no record stream read" warning, once.
static NO_STREAM_WARNED: AtomicBool = AtomicBool::new(false);
/// The "no record header parsed" warning, once.
static NO_HEADER_WARNED: AtomicBool = AtomicBool::new(false);
/// Records seen before the hook checks its own silent paths without waiting
/// for the plugin thread's `[stat]` line.
const SILENT_CHECK_AT: u64 = 1000;

/// The record header shape this table uses, latched from the first record
/// whose header validates as exactly one shape
/// ([`gimmick::detect_header_shape`]), then held to for every other record:
/// one table has one shape, and a record that fails it is counted, not
/// re-detected. Unset until a record has validated.
static HEADER_SHAPE: OnceLock<HeaderShape> = OnceLock::new();

/// Where the stream object keeps the buffer, size and cursor on this build.
/// Set by [`set_stream_layout`] before the hook is installed, or by the
/// first-call probe; unset means every call returns without touching
/// anything. A `OnceLock` because it is written once and then only read, and
/// its `get` is a plain atomic load on the game threads that read it.
static STREAM: OnceLock<StreamLayout> = OnceLock::new();

/// The table body the first call copied under `DumpTable=1`, waiting for the
/// plugin's own thread to write it out ([`take_pending_dump`], polled from
/// `entry.rs`'s ~1 s reload tick). The file write cannot happen here: 22 MB
/// of disk IO on a game thread in the middle of the loading screen is exactly
/// the stall this hook is built never to cause.
///
/// **Why a `Mutex` here, when the rule in this file is no locks on a game
/// thread.** The rule exists because a lock a game thread can *wait on* turns
/// the plugin thread's scheduling into a hitch in the game, and a lock taken
/// per record would be contended by the several threads loading records at
/// once. Neither applies to this one:
///
/// - the hook touches it **at most once per process**: only from the
///   [`first_call`], which `FIRST_CALL_LOGGED` lets
///   exactly one call into, and only when a dump was decided on;
/// - both sides take it with **`try_lock`** only, so neither ever waits. If
///   the plugin thread happened to be holding it in that instant - an empty
///   check that lasts nanoseconds, once a second - the hook drops the copy and
///   logs it rather than blocking, and the cost is one launch without a file;
/// - there is nothing to be lock-free *about*: a hand-off of one `Vec` from
///   one producer to one consumer, once. An atomic pointer would need a
///   `Box::into_raw` and a matching `from_raw` on the other thread for no gain.
///
/// The log line the hook writes a moment earlier already takes the log's own
/// `Mutex`, so this is not the first lock a game thread sees in this file; it
/// is the only one this file adds.
static PENDING_DUMP: Mutex<Option<Vec<u8>>> = Mutex::new(None);

/// The dumped table body, if the hook has copied one the plugin thread has not
/// collected yet. Never blocks: `None` both when there is nothing and in the
/// one instant the hook might be storing into the slot, which the next tick
/// picks up instead.
pub fn take_pending_dump() -> Option<Vec<u8>> {
    PENDING_DUMP.try_lock().ok()?.take()
}

/// `DumpTable=1`: copy the whole table body out of the stream buffer and park
/// it in [`PENDING_DUMP`]. Called once, from [`first_call`], on a game
/// thread.
///
/// Reads the live `DumpTable` and `DryRun` rather than the startup ones: the
/// ini can change in the ~9 s between launch and the first load, and what
/// matters is whether the hook is writing *now*. That is also why the
/// "ignored: needs DryRun=1" line is logged again here although the startup
/// summary already said it - this one is the decision actually taken, printed
/// beside the `first call` line whose size it was taken for.
///
/// Only the first call copies. Under `DryRun=1` every call leaves the buffer
/// vanilla, so any call's copy would do, but the first is the one moment the
/// buffer is certain to hold the table the game has just read, and taking the
/// copy there keeps `DumpTable` off the per-record path entirely: after the
/// first call nothing here is so much as loaded.
fn take_dump(buf: usize, size: usize) {
    match dump::dump_decision(LIVE.dump_table(), LIVE.dry_run(), size) {
        DumpDecision::Skip => {}
        DumpDecision::Refuse(why) => crate::log!("[dump] {why}"),
        DumpDecision::Take => {
            // `try_reserve_exact` rather than `vec![0; size]`: an allocation
            // failure there is an abort (panic = "abort"), on a game thread,
            // for a diagnostic. Here it is a log line.
            let mut bytes = Vec::new();
            if bytes.try_reserve_exact(size).is_err() {
                crate::log!("[dump] table NOT dumped: could not allocate {size} bytes");
                return;
            }
            bytes.resize(size, 0);
            if !safe::read_into(buf, &mut bytes) {
                crate::log!(
                    "[dump] table NOT dumped: the guarded read of 0x{size:X} bytes at 0x{buf:X} failed"
                );
                return;
            }
            match PENDING_DUMP.try_lock() {
                Ok(mut slot) => *slot = Some(bytes),
                Err(_) => crate::log!(
                    "[dump] table NOT dumped: the hand-off to the plugin thread was busy; \
                     try again next launch"
                ),
            }
        }
    }
}

/// Keys already warned about for a name mismatch. A fixed table of atomics
/// rather than a `Mutex<HashSet>`: the callback must not take a lock a game
/// thread could contend on, and 275 gather records fit comfortably.
const WARN_SLOTS: usize = 256;
static NAME_WARNED: [AtomicU32; WARN_SLOTS] = [const { AtomicU32::new(0) }; WARN_SLOTS];

/// `(records seen, gather records patched, scalars written)`.
pub fn counters() -> Counters {
    let r = |a: &AtomicU64| a.load(Ordering::Relaxed);
    Counters {
        seen: r(&RECORDS_SEEN),
        stream_unread: r(&STREAM_UNREAD),
        stream_read: r(&STREAM_READ),
        headers_unparsed: r(&HEADERS_UNPARSED),
        headers_parsed: r(&HEADERS_PARSED),
        recognised: r(&RECORDS_RECOGNISED),
        patched: r(&RECORDS_PATCHED),
        written: r(&EDITS_WRITTEN),
    }
}

/// Everything the hook counted, for the `[stat]` line. `stream_read` and
/// `headers_parsed` are not printed; they are what [`check_silent_paths`]
/// tests against zero.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub seen: u64,
    pub stream_unread: u64,
    pub stream_read: u64,
    pub headers_unparsed: u64,
    pub headers_parsed: u64,
    pub recognised: u64,
    pub patched: u64,
    pub written: u64,
}

impl Counters {
    /// The `[stat]` line's body.
    pub fn line(&self) -> String {
        format!(
            "records seen {}, stream unread {}, headers unparsed {}, gather records recognised {}, \
             patched {}, scalars written {}",
            self.seen, self.stream_unread, self.headers_unparsed, self.recognised, self.patched, self.written
        )
    }
}

/// Say once, loudly, when a whole silent path has eaten every record: no
/// stream read at all, or streams read and not one header parsed. Each of
/// those is a format change that leaves yields vanilla and, before this, said
/// nothing (two of them on 26361232 alone). Called by the plugin thread with
/// every `[stat]` line and by the hook itself once [`SILENT_CHECK_AT`] records
/// have gone past.
pub fn check_silent_paths() {
    let c = counters();
    if c.seen == 0 {
        return;
    }
    if c.stream_read == 0 && c.stream_unread > 0 && !NO_STREAM_WARNED.swap(true, Ordering::Relaxed) {
        crate::log!(
            "[gimmick] WARN no record stream read in {} records: the stream layout changed; yields stay vanilla",
            c.seen
        );
    }
    if c.stream_read > 0 && c.headers_parsed == 0 && !NO_HEADER_WARNED.swap(true, Ordering::Relaxed) {
        crate::log!(
            "[gimmick] WARN no record header parsed in {} records: the record format changed; yields stay vanilla",
            c.seen
        );
    }
}

/// The header of `bytes` in the latched shape, latching (and logging) it from
/// the first record that validates as exactly one shape.
fn record_header(bytes: &[u8]) -> Option<gimmick::RecordHeader> {
    if let Some(&shape) = HEADER_SHAPE.get() {
        return gimmick::parse_header_as(bytes, shape);
    }
    let h = gimmick::parse_header(bytes)?;
    if HEADER_SHAPE.set(h.shape).is_ok() {
        let what = match h.shape {
            HeaderShape::NameFirst => "name first, key after it (26361232 shape)",
            HeaderShape::KeyFirst => "key first (25477059 shape)",
        };
        crate::log!("[gimmick] record header: {what}");
    }
    // Another thread may have latched first; hold this record to that shape.
    match HEADER_SHAPE.get() {
        Some(&s) if s != h.shape => gimmick::parse_header_as(bytes, s),
        _ => Some(h),
    }
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
    collect::COLLECT_RECORDS.iter().find(|(k, _, _, _)| *k == key).map(|(_, n, _, _)| *n)
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

/// Hand the hook the stream layout parsed from the accessor's fill sequence.
/// Called once, before the hook is installed; `false` if a layout was already
/// set (which nothing does, so it would be a second install).
pub fn set_stream_layout(l: StreamLayout) -> bool {
    STREAM.set(l).is_ok()
}

/// `buffer +0x20 size +0x28 cursor +0x2c`, the way every log line names a
/// layout.
pub fn describe_layout(l: &StreamLayout) -> String {
    format!("buffer +0x{:x} size +0x{:x} cursor +0x{:x}", l.buf, l.size, l.cursor)
}

/// The first call's one-off work, on whichever game thread got there first:
/// settle the stream layout if install time could not (the probe), print the
/// raw values a log from a new build is read for, and take the table dump.
///
/// The probe is here because only a live stream can validate a layout, and
/// here only, so exactly one thread runs it; every other call returns without
/// touching anything until [`STREAM`] is set, and for good once the probe has
/// failed.
fn first_call(mgr: usize, count: usize, idx: usize, stream: usize) {
    let entry = safe::read_ptr(mgr + 0x28).and_then(|t| safe::read::<u32>(t + idx * 8 + 4));
    let layout = match STREAM.get() {
        Some(l) => Some(*l),
        None => probe_layout(stream, entry),
    };
    let Some(l) = layout else {
        crate::log!(
            "[gimmick] first call: mgr=0x{mgr:X} count={count} idx={idx} stream=0x{stream:X} \
             stream layout unknown table[idx]=0x{:X}",
            entry.unwrap_or(0)
        );
        return;
    };
    let buf = safe::read_ptr(stream + l.buf as usize).unwrap_or(0);
    let size = safe::read::<u32>(stream + l.size as usize).unwrap_or(0);
    let cursor = safe::read::<u32>(stream + l.cursor as usize).unwrap_or(0);
    crate::log!(
        "[gimmick] first call: mgr=0x{mgr:X} count={count} idx={idx} stream=0x{stream:X} \
         buf=0x{buf:X} size=0x{size:X} cursor=0x{cursor:X} table[idx]=0x{:X} ({})",
        entry.unwrap_or(0),
        describe_layout(&l)
    );
    // A size that would not read is the 0 above, which the decision
    // refuses, and a buffer pointer that would not read is a 0 that
    // `safe::read_into` refuses: neither guesses.
    take_dump(buf, size as usize);
}

/// Try every known layout against the live stream and keep the one that
/// validates; see [`gimmick::choose_layout`]. Logs which won and why, or that
/// none did, once (it is only reached from [`first_call`]).
fn probe_layout(stream: usize, entry: Option<u32>) -> Option<StreamLayout> {
    let readings = StreamLayout::KNOWN.map(|l| StreamReading {
        layout: l,
        buf: safe::read_ptr(stream + l.buf as usize),
        size: safe::read::<u32>(stream + l.size as usize),
        cursor: safe::read::<u32>(stream + l.cursor as usize),
    });
    let chosen = match entry {
        Some(expected) => gimmick::choose_layout(&readings, expected, dump::MAX_DUMP as u32),
        None => Err("the offsets table entry for this record is unreadable".to_string()),
    };
    match chosen {
        Ok((l, why)) => {
            // The only writer left: install time did not set one, or this
            // would not be running.
            let _ = STREAM.set(l);
            crate::log!("[gimmick] stream layout: {} (probed on the first call: {why})", describe_layout(&l));
            Some(l)
        }
        Err(why) => {
            crate::log!(
                "[gimmick] stream layout unknown on this build: not patching anything and not \
                 dumping ({why})"
            );
            None
        }
    }
}

/// Read the record's raw bytes out of the stream buffer.
///
/// Returns `None` for every reason the record must be left alone: no stream
/// layout, a read that failed, an index out of range, a record already loaded,
/// or an offsets table that disagrees with the stream cursor.
fn record_bytes(mgr: usize, idx: usize, stream: usize) -> Option<(usize, Vec<u8>)> {
    // The offsets are `desert_core::manager`'s, the reads are not: this function
    // must read the count *before* the first-call diagnostic below and the
    // records array *after* it, and it wants the `records[idx] == 0` reading
    // rather than the object, so `manager::view_of` and `manager::slot` would
    // both change what happens here. Only the two strides are shared.
    let unread = || {
        STREAM_UNREAD.fetch_add(1, Ordering::Relaxed);
        None
    };
    let Some(count) = safe::read::<u32>(mgr + manager::MGR_COUNT) else { return unread() };
    let count = count as usize;
    if !FIRST_CALL_LOGGED.swap(true, Ordering::Relaxed) {
        first_call(mgr, count, idx, stream);
    }
    // Unset while the first call is still probing on another thread, and for
    // the whole session when no layout validated: either way, touch nothing.
    let Some(&layout) = STREAM.get() else { return unread() };
    if idx >= count {
        return unread();
    }
    // The two callers only reach the loader when the slot is null. If it is
    // not, the game will not parse anything and a patch would be pointless
    // (and, since the buffer may hold a different record's bytes, wrong).
    // Not a failure, so not counted.
    let Some(records) = safe::read_ptr(mgr + manager::MGR_RECORDS) else { return unread() };
    match safe::read::<usize>(records + idx * 8) {
        Some(0) => {}
        Some(_) => return None,
        None => return unread(),
    }
    match stream_record(mgr, idx, count, stream, layout) {
        Some(r) => {
            STREAM_READ.fetch_add(1, Ordering::Relaxed);
            Some(r)
        }
        None => unread(),
    }
}

/// The record's bytes out of the stream buffer at `layout`, checked against
/// the offsets table; `None` (counted as unread by the caller) on any doubt.
fn stream_record(
    mgr: usize,
    idx: usize,
    count: usize,
    stream: usize,
    layout: StreamLayout,
) -> Option<(usize, Vec<u8>)> {
    let buf = safe::read_ptr(stream + layout.buf as usize)?;
    let size = safe::read::<u32>(stream + layout.size as usize)? as usize;
    let cursor = safe::read::<u32>(stream + layout.cursor as usize)? as usize;
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
    if RECORDS_SEEN.fetch_add(1, Ordering::Relaxed) + 1 == SILENT_CHECK_AT {
        check_silent_paths();
    }
    // The manager is a process singleton and this is the only place its
    // address is ever seen. `reapply` needs it after the load pass is over.
    MANAGER.store(mgr, Ordering::Relaxed);

    let Some((at, bytes)) = record_bytes(mgr, idx, stream) else { return };
    let Some(header) = record_header(&bytes) else {
        HEADERS_UNPARSED.fetch_add(1, Ordering::Relaxed);
        return;
    };
    HEADERS_PARSED.fetch_add(1, Ordering::Relaxed);

    let Some(family) = collect::family_by_key(header.key) else {
        if LIVE.debug() && DEBUG_LINES.fetch_add(1, Ordering::Relaxed) < DEBUG_CAP {
            crate::log!(
                "[gimmick] idx {idx} key {} {:?}: not a gather record",
                header.key, header.name
            );
        }
        return;
    };
    RECORDS_RECOGNISED.fetch_add(1, Ordering::Relaxed);

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

/// Where the yields lived in a *parsed* record through 25477059
/// (`gimmick::ParsedLayout::OLD`), as opposed to the raw bytes the callback
/// edits. Established by decompile and disassembly of build 25116796 and
/// written up in `docs/reference-internals.md` section 16. Since 26361232 the
/// layout a pass uses is the probed one ([`parsed_layout`]), which tries this
/// one first.
///
/// ```text
/// mgr+0x08    u32   record_count
/// mgr+0x58    ptr   array of record pointers (null = not loaded)
/// rec+0x08    u32   key, the same one the raw record header carries
/// rec+0x278   ptr   output list data: entries of 16 bytes
/// rec+0x280   u32   entry count
/// entry+0x00  ptr   block object (0x70 bytes), null when the disk flag was 0
/// entry+0x08  u32   unnamed (raw block +64); ZERO on gather records
/// block+0x0C  u32   DropInfoData._dropTagNameHash (raw block +22); the
///                   collect-drop-rate tag, nonzero on every gather block
/// block+0x20  u64   MIN   (raw block +42)
/// block+0x28  u64   MAX   (raw block +50)
/// block+0x68  u32   item id (raw block +1, echoed at +60)
/// ```
///
/// `rec+0x288` holds one further optional block of the same type with no
/// count. The raw scanner has never touched it, so neither does this: the two
/// paths must produce the same yields or a session's numbers would depend on
/// when the ini was last edited.
// The manager pair itself is not here: `mgr+0x08` and `mgr+0x58` are
// `desert_core::manager`'s MGR_COUNT and MGR_RECORDS, which `desert_dispatch`
// reads the faction-node and dropset managers through as well. One
// definition, so a stride cannot be right in one subsystem and wrong in the
// other.
//
// The rest of the table above was `mod parsed`'s constants through 0.10.0 and
// is `gimmick::ParsedLayout::OLD` now. Two notes from it stay true:
//
// - the entry's key field (`entry+0x08` with 16-byte entries) is zero on
//   every gather record, measured over all 896 output blocks in the clean
//   body (build 25246367), so it identifies nothing and is checked against zero, never against
//   an item id. The 142 blocks that carry a nonzero value there are chests,
//   dig sites and dungeon loot (`docs/reference-internals.md` section 16);
// - the block's item id was corrected on 2026-09-12 from `0x6C`, always zero,
//   to `0x68`: the block parser takes eight bytes from disk `+1` into
//   `block+0x68`, so the low dword is the item id and the high one is disk
//   `+5`, a pad.
//
// On 26361232 the key is no longer at `+0x08` on any remembered record, so the
// layout is probed once per session by [`parsed_layout`].
///
/// The parsed record layout this session's re-apply passes use, probed on the
/// first pass that has a manager and loaded remembered records.
static PARSED: OnceLock<gimmick::ParsedLayout> = OnceLock::new();
/// The probe ran and did not converge: no re-apply this session.
static PARSED_FAILED: AtomicBool = AtomicBool::new(false);
/// Keys the `Debug=1` dump of the first pass prints: the well, plus the first
/// other remembered record.
const DUMP_KEY: u32 = 1_001_081;
/// Bytes of a parsed object the `Debug=1` dump prints.
const DUMP_OBJ: usize = 0x300;
/// Bytes of an entry and of its block the dump prints.
const DUMP_BLOCK: usize = 0x80;

/// A guarded read of `len` bytes at `addr`, for the probe.
fn read_bytes(addr: usize, len: usize) -> Option<Vec<u8>> {
    if !plausible(addr) {
        return None;
    }
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, 0);
    safe::read_into(addr, &mut v).then_some(v)
}

/// `bytes` as `[live] [<what>] +0x..` lines, 16 bytes each.
fn hex_lines(what: &str, bytes: &[u8]) {
    for (i, row) in bytes.chunks(16).enumerate() {
        crate::log!("[live] [{what}] +0x{:03x}  {}", i * 16, hex(row));
    }
}

/// The `Debug=1` dump: the well and one more remembered record's object, and
/// their first list entry and its block. With a layout the list is the
/// layout's. Without one (the probe failed) the pointer chain to the first
/// remembered block's item id is searched for instead
/// ([`gimmick::find_item_paths`]) and every path printed, with the windows
/// along the first one: that path is the layout, offsets included, which is
/// what an object dump alone could not show on 26361232. What a layout is
/// read off when the probe fails.
fn dump_objects(samples: &[gimmick::ParsedSample], layout: Option<&gimmick::ParsedLayout>) {
    let well = samples.iter().find(|s| s.key == DUMP_KEY);
    let other = samples.iter().find(|s| s.key != DUMP_KEY);
    for s in well.into_iter().chain(other) {
        crate::log!(
            "[live] [obj] key {} ({}) at 0x{:X}, {} remembered block(s) {:?}",
            s.key,
            table_name(s.key).unwrap_or("?"),
            s.obj,
            s.blocks.len(),
            s.blocks
        );
        match read_bytes(s.obj, DUMP_OBJ) {
            Some(b) => hex_lines("obj", &b),
            None => crate::log!("[live] [obj] unreadable"),
        }
        let Some(l) = layout else {
            find_and_dump(s);
            continue;
        };
        let entry = list_of(s.obj, l).map(|(_, p)| p).filter(|&p| plausible(p));
        let Some(entry) = entry else {
            crate::log!("[live] [entry] no plausible list under {}", l.describe());
            continue;
        };
        if let Some(b) = read_bytes(entry, DUMP_BLOCK) {
            hex_lines("entry", &b);
        }
        let block = safe::read::<usize>(entry).filter(|&p| plausible(p));
        if let Some(b) = block.and_then(|p| read_bytes(p, DUMP_BLOCK)) {
            hex_lines("block", &b);
        }
    }
}

/// `(entry count, entries pointer)` of the record object at `obj` under
/// `l`: read from the object itself for an inline list header, or from the
/// header the object points at (`ParsedLayout::head`, 26361232). `None` when a
/// read fails or the header pointer is not plausible; the entries pointer is
/// handed back raw, null included, for the caller's own checks.
fn list_of(obj: usize, l: &gimmick::ParsedLayout) -> Option<(u32, usize)> {
    let base = match l.head {
        None => obj,
        Some(h) => safe::read::<usize>(obj + h).filter(|&p| plausible(p))?,
    };
    Some((safe::read::<u32>(base + l.count)?, safe::read::<usize>(base + l.list)?))
}

/// Depth, window and hit bounds of the `Debug=1` item search.
const FIND_DEPTH: usize = 3;
const FIND_NODES: usize = 4000;
const FIND_HITS: usize = 8;

/// Search from `s`'s object for its first remembered block's item id and log
/// every pointer path that reaches it, then the windows the first path
/// passes through (the last two: the entry and the block).
fn find_and_dump(s: &gimmick::ParsedSample) {
    let Some(&(item, min, max)) = s.blocks.first() else { return };
    let paths = gimmick::find_item_paths(s.obj, item, read_bytes, FIND_DEPTH, FIND_NODES, FIND_HITS);
    if paths.is_empty() {
        crate::log!(
            "[live] [find] key {} item {item}: not reachable within {FIND_DEPTH} pointers ({FIND_NODES} windows)",
            s.key
        );
        return;
    }
    for p in &paths {
        let mut text = String::from("obj");
        for (i, o) in p.iter().enumerate() {
            text.push_str(&format!("{}+0x{o:x}", if i == 0 { "" } else { " -> " }));
        }
        crate::log!("[live] [find] key {} item {item} (vanilla {min}..{max}): {text}", s.key);
    }
    // Walk the first path, keeping the windows it passes through.
    let Some(first) = paths.first() else { return };
    let mut addr = s.obj;
    let mut trail = vec![addr];
    for &o in first.iter().take(first.len().saturating_sub(1)) {
        let Some(next) = safe::read::<usize>(addr + o).filter(|&p| plausible(p)) else { return };
        addr = next;
        trail.push(addr);
    }
    let n = trail.len();
    for (i, &a) in trail.iter().enumerate().skip(1) {
        let what = if i + 1 == n { "block" } else { "entry" };
        crate::log!("[live] [{what}] at 0x{a:X}");
        if let Some(b) = read_bytes(a, DUMP_BLOCK) {
            hex_lines(what, &b);
        }
    }
}

/// The parsed layout for this pass: the latched one, or the probe's answer on
/// the first pass with loaded remembered records (logged once, either way).
/// `None` once the probe has failed: the pass is skipped, and the load path
/// still multiplies every record the game loads later.
fn parsed_layout(records: usize, count: usize, debug: bool) -> Option<gimmick::ParsedLayout> {
    if let Some(&l) = PARSED.get() {
        return Some(l);
    }
    if PARSED_FAILED.load(Ordering::Relaxed) {
        return None;
    }
    let samples: Vec<gimmick::ParsedSample> = remember::snapshot()
        .into_iter()
        .filter(|r| (r.idx as usize) < count)
        .filter_map(|r| match manager::slot(records, r.idx as usize) {
            manager::Slot::Loaded(obj) => Some(gimmick::ParsedSample { key: r.key, obj, blocks: r.blocks }),
            _ => None,
        })
        .collect();
    if samples.is_empty() {
        // Nothing loaded to probe against yet; the pass has nothing to do
        // either, so the old layout costs nothing and nothing is latched.
        return Some(gimmick::ParsedLayout::OLD);
    }
    match gimmick::probe_parsed_layout(&samples, read_bytes) {
        Ok(l) => {
            crate::log!(
                "[live] parsed layout: {} (probed against {} remembered records)",
                l.describe(),
                samples.len()
            );
            if debug {
                dump_objects(&samples, Some(&l));
            }
            Some(*PARSED.get_or_init(|| l))
        }
        Err(why) => {
            PARSED_FAILED.store(true, Ordering::Relaxed);
            crate::log!(
                "[live] WARN parsed layout not found ({why}); sliders have no effect until the next \
                 load, the hook still multiplies at load"
            );
            if debug {
                dump_objects(&samples, None);
            }
            None
        }
    }
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
    /// True when the parsed-layout probe did not converge this session:
    /// nothing was attempted.
    pub layout_unknown: bool,
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
    /// The block's current min/max is not one whole multiple of the
    /// remembered vanilla pair.
    pub block_value: usize,
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
            layout_unknown: false,
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
            block_value: 0,
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
        if self.layout_unknown {
            return Some(
                "the parsed record layout is unknown on this build (see the parsed layout line); \
                 nothing was re-applied"
                    .to_string(),
            );
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
            (self.block_value, "yield not a multiple of vanilla"),
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
/// expressions there can overflow, which `safe::read` would refuse anyway,
/// but only after the addition had already happened.
///
/// `desert_core::manager::plausible` under the name this file has always used.
/// It was byte-identical to `desert_dispatch`'s copy, and a bound that differed
/// between two subsystems walking the same managers would be a bug in whichever
/// one was looser.
use desert_core::manager::plausible;

/// The multiplier a remembered record should be at right now: its family's
/// live value, or 1 when `Enabled=0` or the key is not a gather record after
/// all. Never 0: that would zero the record's yields.
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
/// the vanilla blocks `crate::gatherer::remember` holds are the only fixed point, and
/// scaling what is already there would compound every edit. A block is only
/// written when the record's key and the block's item id both still match what
/// was remembered, so a record the game has replaced or a layout that moved is
/// skipped rather than corrupted. Every read is a guarded `safe::read`, every
/// write a guarded `safe::write::<u64>`, and no game function is called.
pub fn reapply() -> Outcome {
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
    let Some(layout) = parsed_layout(records, count, debug) else {
        out.layout_unknown = true;
        return out;
    };

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
        match safe::read::<u32>(obj + layout.key) {
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
        let Some((n, data)) = list_of(obj, &layout) else {
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
            let entry = data + i * layout.entry;
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
            // A layout without an entry key field reads it as the zero it
            // would have been.
            let entry_item = match layout.entry_item {
                Some(o) => safe::read::<u32>(entry + o),
                None => Some(0),
            };
            let (Some(entry_item), Some(block_item)) = (entry_item, safe::read::<u32>(block + layout.block_item))
            else {
                out.blocks_skipped += 1;
                out.block_read += 1;
                continue;
            };
            // The block's item id has to be the one the raw block carried, or
            // this is not the block we remembered. `entry_item` is not a second
            // copy of it - it is the list entry's own key field, zero on every
            // gather record (see the notes above `PARSED`) - so it is checked
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
                (safe::read::<u64>(block + layout.block_min), safe::read::<u64>(block + layout.block_max))
            else {
                out.blocks_skipped += 1;
                out.block_read += 1;
                continue;
            };
            // What sits there now must be the remembered vanilla times one
            // whole multiplier (the load-time one, or a previous pass's): a
            // field that reads anything else is not this block's yield, and
            // writing to it would corrupt whatever it is.
            if gimmick::vanilla_factor(cur_min, cur_max, min, max).is_none() {
                out.blocks_skipped += 1;
                out.block_value += 1;
                continue;
            }
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
                let ok = usize::from(safe::write(block + layout.block_min, want_min))
                    + usize::from(safe::write(block + layout.block_max, want_max));
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
