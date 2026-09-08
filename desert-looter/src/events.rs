//! The write path: forging the game's own PickUpItem event and pushing it
//! onto the game's event queue from the game thread.
//!
//! Everything here mirrors what the game's own event builder does
//! (`FUN_1426B21A0`, build 25116796):
//!
//! ```text
//! prepare();                                   // static-init guard, no args
//! desc = descriptor_by_id(_, id, desc_mask);   // hash lookup, returns 0 if masked out
//! ev   = alloc_event(_, payload_size);         // 0x80-byte object, payload buffer at +0x70
//! ev+0x30=1  +0x40=0  +0x48=0  +0x50=actor eid  +0x54=0  +0x58=actor route (actor+0x58)
//! ev+0x60=desc  +0x68=size(u16)  +0x78=1;  memcpy(*(ev+0x70), payload, size)
//! enqueue(*queue, ev, desc, flag);
//! ```
//!
//! The allocator and enqueue both consult thread-local state (`gs:[0x58]`),
//! so the whole sequence runs inside the sweep hook on the game thread. The
//! worker thread only parks a request; the hook drains it.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::actors;
use crate::module::MainModule;
use crate::safe;

pub use crate::payload::{
    CATCH_DESCRIPTOR, CATCH_ID_EXPECTED, CATCH_PAYLOAD_SIZE, PICKUP_DESCRIPTOR, PICKUP_ID_EXPECTED,
    PICKUP_PAYLOAD_SIZE,
};
use crate::payload::{catch_payload, hex, pickup_payload, PickupMode};

pub const MAX_DESCRIPTOR_ID: u32 = 0x1FFF;
/// Event object size from the allocator (`FUN_1413A9790`: alloc(0x80, 0x10)).
pub const EVENT_SIZE: usize = 0x80;

type PrepareFn = unsafe extern "system" fn() -> usize;
type DescByIdFn = unsafe extern "system" fn(usize, u32, u32) -> usize;
type AllocEventFn = unsafe extern "system" fn(usize, u32) -> usize;
type EnqueueFn = unsafe extern "system" fn(usize, usize, usize, u8);
/// `bool is_steal(acting_comp, player_actor, target_actor, ctx, u8 mode)`.
type StealCheckFn = unsafe extern "system" fn(usize, usize, usize, usize, u32) -> u8;

/// Game-side addresses resolved from the signatures (absolute VAs).
#[derive(Debug, Clone, Copy)]
pub struct EventApi {
    pub prepare: usize,
    pub desc_by_id: usize,
    pub alloc_event: usize,
    pub enqueue: usize,
    /// Global slot holding the event queue pointer.
    pub queue_slot: usize,
    /// Global u32 passed as the descriptor lookup mask.
    pub desc_mask: usize,
    /// The game's "would this be stealing?" function (`FUN_14251BA50`) and
    /// the static its caller passes as the fourth argument. 0 if unresolved.
    pub steal_check: usize,
    pub steal_ctx: usize,
}

/// Decode the four things the `desc_mask+queue` call site yields:
/// `E8` at +0 (prepare), `44 8B 05`
/// at +5 (desc_mask), `E8` at +0x11 (descriptor_by_id), `4C 8B 25` at +0x16
/// (queue slot).
pub fn resolve(m: &MainModule, anchors: &crate::game::Anchors) -> Result<EventApi, String> {
    let site = anchors.get("desc_mask+queue_site").ok_or("desc_mask+queue_site not found")?;
    let alloc_event = anchors.get("alloc_event").ok_or("alloc_event not found")?;
    let enqueue = anchors.get("enqueue").ok_or("enqueue not found")?;
    let img = m.bytes();
    let rel = |ins_off: usize, disp_off: usize, ins_len: usize| -> Result<usize, String> {
        let at = m.rva(site) + ins_off + disp_off;
        let d: [u8; 4] = img
            .get(at..at + 4)
            .and_then(|b| b.try_into().ok())
            .ok_or("site outside image")?;
        let disp = i32::from_le_bytes(d) as i64;
        let target = (site as i64 + ins_off as i64 + ins_len as i64 + disp) as usize;
        if !m.contains(target) {
            return Err(format!("rel target 0x{target:X} outside module"));
        }
        Ok(target)
    };
    // own_check_site: `48 8B 89 20 01 00 00 | E8 rel32 | 84 C0 74 04 B3 02`;
    // 0xE bytes before it, `4C 8D 0D rel32` loads the r9 argument.
    let (steal_check, steal_ctx) = match anchors.get("own_check_site") {
        Some(site) => {
            let base = m.rva(site);
            let at = |o: usize| -> Option<i32> {
                let b = img.get(base.wrapping_add(o)..base.wrapping_add(o) + 4)?;
                Some(i32::from_le_bytes(<[u8; 4]>::try_from(b).ok()?))
            };
            let f = at(8).map(|d| (site as i64 + 12 + d as i64) as usize).filter(|t| m.contains(*t));
            let lea_ok = img.get(base - 0xE..base - 0xB) == Some(&[0x4C, 0x8D, 0x0D]);
            let c = if lea_ok {
                at(0usize.wrapping_sub(0xB)).map(|d| (site as i64 - 0xE + 7 + d as i64) as usize).filter(|t| m.contains(*t))
            } else {
                None
            };
            (f.unwrap_or(0), c.unwrap_or(0))
        }
        None => (0, 0),
    };
    Ok(EventApi {
        prepare: rel(0, 1, 5)?,
        desc_mask: rel(5, 3, 7)?,
        desc_by_id: rel(0x11, 1, 5)?,
        queue_slot: rel(0x16, 3, 7)?,
        alloc_event,
        enqueue,
        steal_check,
        steal_ctx,
    })
}

#[derive(Debug, Clone)]
pub struct Descriptor {
    pub id: u16,
    pub ptr: usize,
    pub payload_size: u16,
    /// `desc+0x1C`: 1 = routed to a worker queue, 0 = handled inline (from enqueue's decompile).
    pub dispatch: u32,
    pub name: String,
}

fn desc_mask(api: &EventApi) -> u32 {
    match safe::read::<u32>(api.desc_mask) {
        Some(0) | None => 0xFFFF_FFFF,
        Some(v) => v,
    }
}

/// Walk descriptor ids 1..=0x1FFF through the game's lookup and return the one
/// whose RTTI class name is `name`. The lookup (`FUN_1413AB9F0`) is a plain
/// read-only hash-table probe over globals, so it is safe to call from our
/// thread once the game has built the table.
pub fn find_descriptor(m: &MainModule, api: &EventApi, name: &str) -> Result<Descriptor, String> {
    // SAFETY: `api.desc_by_id` was decoded by `resolve` from the `E8` at
    // `desc_mask+queue_site`+0x11 and rejected unless it lands inside the main
    // module, so it is the entry point of the game's own
    // `descriptor_by_id(_, id, mask)`; `DescByIdFn` is that function's
    // prototype as the call site uses it.
    let f: DescByIdFn = unsafe { core::mem::transmute(api.desc_by_id) };
    let mask = desc_mask(api);
    let mut seen = 0u32;
    for id in 1..=MAX_DESCRIPTOR_ID {
        // SAFETY: `FUN_1413AB9F0` is a read-only probe of a global hash table
        // that touches no thread-local state, so the plugin thread may call it
        // once the game has built the table - which the caller establishes by
        // only resolving descriptors after the world exists.
        let d = unsafe { f(0, id, mask) };
        if d == 0 {
            continue;
        }
        seen += 1;
        let Some(full) = actors::rtti_name(m, d) else { continue };
        let short = actors::short_name(&full);
        if short == name {
            let payload_size: u16 = safe::read(d + 0x18).ok_or("descriptor+0x18 unreadable")?;
            let dispatch: u32 = safe::read(d + 0x1C).unwrap_or(u32::MAX);
            crate::log!("[event] descriptor walk: {seen} live ids up to {id} (mask=0x{mask:X})");
            return Ok(Descriptor { id: id as u16, ptr: d, payload_size, dispatch, name: short });
        }
    }
    Err(format!("{name} not found among {seen} descriptors (mask=0x{mask:X})"))
}

/// What the hotkey parks for the game thread.
#[derive(Debug, Clone, Copy)]
pub struct PickupRequest {
    pub target_eid: u32,
    /// Gimmick record index of the target, for yield learning.
    pub record: u16,
    pub mode: PickupMode,
    /// Actor pointers for the ownership check.
    pub target_actor: usize,
    pub player_actor: usize,
    pub player_eid: u32,
    pub route: u32,
    /// `enqueue` flag byte (the game's builder passes its caller's choice).
    pub flag: u8,
}

static API: OnceLock<EventApi> = OnceLock::new();
/// Eids the game said would be stealing; skipped for the rest of the session.
static OWNED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn is_owned(eid: u32) -> bool {
    OWNED.lock().unwrap_or_else(|e| e.into_inner()).contains(&eid)
}

fn mark_owned(eid: u32) {
    let mut o = OWNED.lock().unwrap_or_else(|e| e.into_inner());
    if !o.contains(&eid) {
        o.push(eid);
    }
}

/// Everything `FUN_14251BA50`'s character branch dereferences without a null
/// check, verified before we hand it a creature (section 14 and section 15 of
/// `docs/reference-internals.md`). From its decompile, build 25116796:
///
/// ```text
/// cVar5 = *(char *)(*(longlong *)(param_3 + 0x88) + 1);
/// if ((cVar5 == 4) || ((byte)(cVar5 - 5U) < 2)) {                  // types 4, 5, 6
///     lVar6 = *(longlong *)(*(longlong *)(param_3 + 0x68) + 0x118); // owner record
///     if ((*(char *)(lVar6 + 0x2c) != '\0') ||
///         (*(int *)(*(longlong *)(param_1 + 8) + 0x60) == *(int *)(lVar6 + 0x18))) { return false; }
/// }
/// // then, for every non-gimmick type:
/// lVar6 = *(longlong *)(*(longlong *)(param_3 + 0x68) + 0x20);     // status component
/// iVar1 = *(int *)(lVar6 + 0x300);  ...  FUN_1416f8b90(lVar6 + 0x2e8, param_3)
/// ```
///
/// The owner record at `sub+0x118` is read straight through, and the status
/// component is indexed out to `+0x300`/`+0x2E8`; the shared tail then reads
/// the transform at `sub+0x1A0` out to `+0x4C8`/`+0x4D0`. Until `PickupMode::Catch`
/// existed every target was a gimmick (type byte 7) and none of this ran. A
/// null or unmapped one of them would fault on the game thread, which is a
/// crash to desktop, so we check each before asking rather than after.
fn creature_preflight(target: usize) -> Result<(), String> {
    let sub = safe::read_ptr(target + 0x68).ok_or("creature has no sub-object at +0x68")?;
    // `read_ptr` already rejects a null or unreadable slot; `readable` then
    // covers the fields the branch indexes out to.
    if safe::read_ptr(sub + 0x118).is_none_or(|owner| !safe::readable(owner, 0x30)) {
        return Err("creature has no readable owner record at sub+0x118; not asking the steal check".into());
    }
    if safe::read_ptr(sub + 0x20).is_none_or(|status| !safe::readable(status, 0x340)) {
        return Err("creature status component unreadable".into());
    }
    if safe::read_ptr(sub + 0x1A0).is_none_or(|tf| !safe::readable(tf, 0x4D8)) {
        return Err("creature transform unreadable".into());
    }
    Ok(())
}

/// Ask the game whether picking `target` up would be stealing, exactly as
/// its interaction code does (`FUN_1429DB730`): the acting actor is
/// `*(*(player+0xA0)+0xD0)`, the first argument its component at
/// `sub+0x120`, then `FUN_14251BA50(comp, player, target, ctx, 7)`.
/// Asked for every pickup, gather nodes as much as ground items: the game's
/// handler makes this call for every interaction target with no category
/// filter, and mode 7 has its own branch for gimmick actors.
///
/// A `Catch` request is the first target that is **not** a gimmick, and mode
/// 7's character branch (type byte 4/5/6) dereferences the owner record, the
/// status component and the transform with no null check of its own, so
/// [`creature_preflight`] verifies all three first (section 15). Gimmick
/// targets are unaffected: nothing about the type-7 path changed.
/// Game thread only. `Err` means "could not ask": treated as owned.
unsafe fn would_steal(api: &EventApi, player: usize, target: usize) -> Result<bool, String> {
    if api.steal_check == 0 || api.steal_ctx == 0 {
        return Err("steal check not resolved".into());
    }
    // Types 4, 5 and 6 are the character branch's own test (`cVar5 == 4 ||
    // (byte)(cVar5 - 5) < 2`).
    if matches!(actors::type_byte(target), Some(4..=6)) {
        creature_preflight(target)?;
    }
    let link = safe::read_ptr(player + 0xA0).ok_or("player+0xA0 is null")?;
    let acting = safe::read_ptr(link + 0xD0).ok_or("acting actor is null")?;
    let sub = safe::read_ptr(acting + 0x68).ok_or("acting actor has no sub-object")?;
    let comp = safe::read_ptr(sub + 0x120).ok_or("acting actor has no +0x120 component")?;
    if !safe::readable(target, 0x100) || !safe::readable(comp, 0x40) {
        return Err("target or component unreadable".into());
    }
    // SAFETY: `api.steal_check` is the `E8` target decoded from the
    // `own_check_site` signature and confirmed to lie inside the main module,
    // so it is the game's own `FUN_14251BA50`; `StealCheckFn` is the prototype
    // that call site uses.
    let f: StealCheckFn = unsafe { core::mem::transmute(api.steal_check) };
    // SAFETY: the same call the game's interaction code makes at
    // `own_check_site`, with the same five arguments. `comp`, `player` and
    // `target` were each read back through `safe` and checked readable just
    // above, `api.steal_ctx` is the module-resident static its caller passes,
    // and this runs on the game thread as this function's contract requires.
    Ok(unsafe { f(comp, player, target, api.steal_ctx, 7) } != 0)
}
static MODULE: OnceLock<MainModule> = OnceLock::new();

pub fn set_module(m: MainModule) {
    let _ = MODULE.set(m);
}
static DESCRIPTOR: OnceLock<Descriptor> = OnceLock::new();
/// `TrocTrPushCharacterToInventoryOnceTimer`, the catch event. Resolved
/// separately and optional: gathering works without it.
static CATCH_DESCRIPTOR_SLOT: OnceLock<Descriptor> = OnceLock::new();
static PENDING: Mutex<Option<(PickupRequest, std::time::Instant)>> = Mutex::new(None);
static PENDING_FLAG: AtomicBool = AtomicBool::new(false);
static SWEEP_CALLS: AtomicU64 = AtomicU64::new(0);
static GAME_TID: AtomicU32 = AtomicU32::new(0);
static SENT: AtomicU32 = AtomicU32::new(0);

pub fn set_api(api: EventApi) {
    let _ = API.set(api);
}

pub fn api() -> Option<EventApi> {
    API.get().copied()
}

pub fn set_descriptor(d: Descriptor) {
    let _ = DESCRIPTOR.set(d);
}

pub fn set_catch_descriptor(d: Descriptor) {
    let _ = CATCH_DESCRIPTOR_SLOT.set(d);
}

// ---------------------------------------------------------------------------
// Yield learning. After a pickup the game queues
// `TrocTrHandleGameEventOnceTimer` (115 bytes) whose payload, for an item
// received, has zeros at 11..19, the item key (u32) at 19, a 2 at 23 and the
// count (u32) at 27 (observed live: copper chunk 720004 x1, vein x10). We
// pair it with the node sent in the previous two seconds and remember
// record -> item key, so the stacking rule can be applied before a send.

pub const HANDLE_GAME_EVENT: &str = "TrocTrHandleGameEventOnceTimer";
static HANDLE_DESC: OnceLock<usize> = OnceLock::new();
/// (record, when); only gather nodes are recorded: a ground item's record is
/// the generic `item_basic_*` and says nothing about its contents.
static LAST_SENT: Mutex<Option<(u16, std::time::Instant)>> = Mutex::new(None);
static YIELDS: Mutex<Vec<(u16, u32, u32)>> = Mutex::new(Vec::new());
static YIELD_DIRTY: AtomicBool = AtomicBool::new(false);
const YIELD_PAIR_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);
/// `LogReceived`: log every item the game hands the player, whether or not we
/// asked for it. Read on the game thread inside the callback, so it is an
/// atomic and the counter that caps it is one too.
static LOG_RECEIVED: AtomicBool = AtomicBool::new(false);
static RECV_LOGGED: AtomicU32 = AtomicU32::new(0);
/// Ceiling on `[recv]` lines per session; the log is append-only.
const RECV_LOG_CAP: u32 = 500;

pub fn set_log_received(on: bool) {
    LOG_RECEIVED.store(on, Ordering::Release);
}

pub fn set_handle_descriptor(ptr: usize) {
    let _ = HANDLE_DESC.set(ptr);
}

/// (item key, count received last time) for a gimmick record, if learned.
pub fn yield_of(record: u16) -> Option<(u32, u32)> {
    YIELDS.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(r, _, _)| *r == record).map(|(_, k, c)| (*k, *c))
}

pub fn yields_snapshot() -> Vec<(u16, u32, u32)> {
    YIELDS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn load_yields(pairs: Vec<(u16, u32, u32)>) {
    *YIELDS.lock().unwrap_or_else(|e| e.into_inner()) = pairs;
}

/// True once after something new was learned; the caller persists.
pub fn take_yield_dirty() -> bool {
    YIELD_DIRTY.swap(false, Ordering::AcqRel)
}

fn note_received(item: u32, count: u32) {
    let sent = LAST_SENT.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some((record, when)) = sent else { return };
    if when.elapsed() > YIELD_PAIR_WINDOW {
        return;
    }
    let mut y = YIELDS.lock().unwrap_or_else(|e| e.into_inner());
    match y.iter_mut().find(|(r, _, _)| *r == record) {
        Some(e) => {
            // Keep the largest count seen: the stacking rule uses it as a margin.
            if e.1 == item && count > e.2 {
                e.2 = count;
                YIELD_DIRTY.store(true, Ordering::Release);
            }
        }
        None => {
            y.push((record, item, count));
            YIELD_DIRTY.store(true, Ordering::Release);
            crate::log!("[yield] record {record} gives item {item} (x{count}); learned");
        }
    }
}

fn watch_received(desc: usize, ev: usize) {
    if HANDLE_DESC.get().copied() != Some(desc) {
        return;
    }
    let size: u16 = safe::read(ev + 0x68).unwrap_or(0);
    if size < 40 {
        return;
    }
    let Some(buf) = safe::read_ptr(ev + 0x70) else { return };
    let mut p = [0u8; 40];
    if !safe::read_into(buf, &mut p) {
        return;
    }
    if p[11..19].iter().any(|&b| b != 0) {
        return;
    }
    let item = u32::from_le_bytes([p[19], p[20], p[21], p[22]]);
    let kind = u32::from_le_bytes([p[23], p[24], p[25], p[26]]);
    let count = u32::from_le_bytes([p[27], p[28], p[29], p[30]]);
    if kind != 2 || item == 0 || count == 0 || count > 10_000 {
        return;
    }
    if LOG_RECEIVED.load(Ordering::Acquire) {
        let bump = |n: u32| (n < RECV_LOG_CAP).then_some(n + 1);
        if RECV_LOGGED.fetch_update(Ordering::Relaxed, Ordering::Relaxed, bump).is_ok() {
            crate::log!("[recv] item {item} x{count}");
        }
    }
    note_received(item, count);
}

pub fn descriptor() -> Option<&'static Descriptor> {
    DESCRIPTOR.get()
}

pub fn catch_descriptor() -> Option<&'static Descriptor> {
    CATCH_DESCRIPTOR_SLOT.get()
}

pub fn sweep_calls() -> u64 {
    SWEEP_CALLS.load(Ordering::Relaxed)
}

pub fn game_thread_id() -> u32 {
    GAME_TID.load(Ordering::Relaxed)
}

pub fn sent_count() -> u32 {
    SENT.load(Ordering::Relaxed)
}

/// Park one request for the game thread. Returns false if one is already waiting.
pub fn request(r: PickupRequest) -> bool {
    let mut g = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        return false;
    }
    *g = Some((r, std::time::Instant::now()));
    PENDING_FLAG.store(true, Ordering::Release);
    true
}

pub fn has_pending() -> bool {
    PENDING_FLAG.load(Ordering::Acquire)
}

/// Take back a request the hook has not drained within `max_age` (the sweep
/// stopped firing: loading screen, audio off, menu). Returns what was dropped.
pub fn drop_stale(max_age: std::time::Duration) -> Option<PickupRequest> {
    let mut g = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    match *g {
        Some((r, t)) if t.elapsed() > max_age => {
            *g = None;
            PENDING_FLAG.store(false, Ordering::Release);
            Some(r)
        }
        _ => None,
    }
}

/// The sweep hook callback. Runs on the game thread once per audio emitter
/// per frame, so the no-work path is one atomic load. Must never panic.
///
/// # Safety
///
/// Only the trampoline installed on `area_sweep` may call this, with the
/// game's own `rcx`/`rdx` for that call. `this` and `item` are treated as
/// untrusted addresses and read through `safe` only; the game function
/// pointers this reaches (the steal check, the event API) are called with the
/// arguments the game itself uses at this site.
pub unsafe extern "system" fn on_sweep(this: usize, item: usize, _r8: usize, _r9: usize) {
    let n = SWEEP_CALLS.fetch_add(1, Ordering::Relaxed);
    if n == 0 {
        // SAFETY: `GetCurrentThreadId` takes no arguments and only reads the
        // calling thread's own TEB; it is sound to call from any thread.
        let tid = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
        GAME_TID.store(tid, Ordering::Relaxed);
        crate::log!("[hook] first sweep callback on tid {tid}: this=0x{this:X} item=0x{item:X}");
    }
    if !PENDING_FLAG.load(Ordering::Acquire) {
        return;
    }
    let req = {
        let mut g = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        PENDING_FLAG.store(false, Ordering::Release);
        g.take().map(|(r, _)| r)
    };
    if let Some(r) = req {
        // SAFETY: `send_pickup` may only run on the game thread, which is
        // exactly where the `area_sweep` trampoline calls this callback from;
        // `r` is plain data the plugin thread parked and we took ownership of.
        match unsafe { send_pickup(&r) } {
            Ok(ev) => {
                SENT.fetch_add(1, Ordering::Relaxed);
                if r.mode == PickupMode::Gather {
                    *LAST_SENT.lock().unwrap_or_else(|e| e.into_inner()) = Some((r.record, std::time::Instant::now()));
                }
                let what = match r.mode {
                    PickupMode::Catch => "PushCharacterToInventory",
                    PickupMode::Gather | PickupMode::Item => "PickUpItem",
                };
                crate::log!("[event] enqueued {what} ({:?}) for eid={:08X}: event 0x{ev:X}", r.mode, r.target_eid);
            }
            Err(e) => crate::log!("[event] NOT sent for eid={:08X}: {e}", r.target_eid),
        }
    }
}

/// Read-only replica of the lookup `enqueue` does for a routed event
/// (`FUN_1413AACB0`): `router = *(queue+0x38)`; table pointer at
/// `router+0x38` (route bits 28..29 == 0) or `router+0x40`; entry =
/// `(*table)[route & 0xFFFFF]` with 0x10-byte stride, **no bounds check**.
/// We refuse to send if any step is unreadable, since the game thread would
/// fault on it. A null entry is the game's own "no such route" error and is
/// harmless (the event is freed), so it is only reported.
pub fn route_entry(queue: usize, route: u32) -> Result<Option<usize>, String> {
    if route == 0 {
        return Ok(None);
    }
    let router = safe::read_ptr(queue + 0x38).ok_or("queue+0x38 (router) unreadable")?;
    let sel = (route >> 28) & 3;
    let table_pp = router + if sel == 0 { 0x38 } else { 0x40 };
    let table_p = safe::read_ptr(table_pp).ok_or(format!("router+0x{:X} unreadable", table_pp - router))?;
    let table = safe::read_ptr(table_p).ok_or("route table pointer unreadable")?;
    let slot = table + (route & 0xF_FFFF) as usize * 0x10;
    let entry: u64 = safe::read(slot).ok_or(format!("route slot 0x{slot:X} unreadable"))?;
    Ok((entry != 0).then_some(entry as usize))
}

/// Build and enqueue one gather event. Game thread only.
unsafe fn send_pickup(r: &PickupRequest) -> Result<usize, String> {
    let api = API.get().ok_or("event api not resolved")?;
    // Two descriptors, two payload layouts: a catch is its own event, not a
    // `PickUpItem` with a different mode byte (section 15).
    let (desc, want) = match r.mode {
        PickupMode::Catch => (
            CATCH_DESCRIPTOR_SLOT.get().ok_or("catch descriptor not resolved")?,
            CATCH_PAYLOAD_SIZE,
        ),
        PickupMode::Gather | PickupMode::Item => (
            DESCRIPTOR.get().ok_or("descriptor not resolved")?,
            PICKUP_PAYLOAD_SIZE,
        ),
    };
    let size = desc.payload_size as usize;
    if size != want {
        return Err(format!("payload size {size} != {want}; refusing"));
    }
    // Every request, gather nodes included: the game's own interaction handler
    // asks this for every target with no category filter (section 14).
    // SAFETY: `would_steal` requires the game thread, which `send_pickup`
    // itself requires and the sweep hook - its only caller - provides.
    match unsafe { would_steal(api, r.player_actor, r.target_actor) } {
        Ok(false) => {}
        Ok(true) => {
            mark_owned(r.target_eid);
            return Err("the game says taking this would be stealing; skipped".into());
        }
        Err(e) => {
            mark_owned(r.target_eid);
            return Err(format!("ownership unknown ({e}); skipped"));
        }
    }
    let queue = safe::read_ptr(api.queue_slot).ok_or("queue global is null/unreadable")?;
    if !safe::readable(queue, 0x40) {
        return Err(format!("queue object 0x{queue:X} unreadable"));
    }
    if !safe::readable(desc.ptr, 0x20) {
        return Err("descriptor object vanished".into());
    }
    match route_entry(queue, r.route)? {
        Some(e) => crate::log!("[event] route 0x{:X} -> entry 0x{e:X}", r.route),
        None if r.route == 0 => crate::log!("[event] route 0: enqueue skips the route lookup"),
        None => crate::log!("[event] route 0x{:X} has a NULL entry: the game will reject this event", r.route),
    }

    // SAFETY: `api.prepare` is the `E8` target at `desc_mask+queue_site`+0,
    // decoded by `resolve` and rejected unless it lies inside the main module;
    // the game's builder calls it with no arguments, which is `PrepareFn`.
    let prepare: PrepareFn = unsafe { core::mem::transmute(api.prepare) };
    // SAFETY: `api.alloc_event` is the unique hit of the `alloc_event`
    // signature in the main module, i.e. `FUN_1413A9790(_, size)`, which is
    // what `AllocEventFn` describes.
    let alloc: AllocEventFn = unsafe { core::mem::transmute(api.alloc_event) };
    // SAFETY: `api.enqueue` is the unique hit of the `enqueue` signature in the
    // main module, i.e. `FUN_1413AACB0(queue, ev, desc, flag)`, which is what
    // `EnqueueFn` describes.
    let enqueue: EnqueueFn = unsafe { core::mem::transmute(api.enqueue) };

    // SAFETY: the game's own static-init guard, argument-less and idempotent.
    // It reads thread-local state at gs:[0x58], so it runs here on the game
    // thread, exactly as the game's event builder calls it.
    unsafe { prepare() };
    // SAFETY: the game's 0x80-byte event allocator, called with the same
    // (null, payload_size) arguments its builder uses. It too consults
    // gs:[0x58], which is why `send_pickup` is game-thread only.
    let ev = unsafe { alloc(0, desc.payload_size as u32) };
    if ev == 0 || !safe::readable(ev, EVENT_SIZE) {
        return Err(format!("alloc_event returned 0x{ev:X}"));
    }
    let buf = safe::read_ptr(ev + 0x70).ok_or("event has no payload buffer at +0x70")?;
    if !safe::readable(buf, size) {
        return Err(format!("payload buffer 0x{buf:X} unreadable"));
    }
    let ok = safe::write::<u32>(ev + 0x30, 1)
        && safe::write::<u32>(ev + 0x40, 0)
        && safe::write::<u64>(ev + 0x48, 0)
        && safe::write::<u32>(ev + 0x50, r.player_eid)
        && safe::write::<u32>(ev + 0x54, 0)
        && safe::write::<u32>(ev + 0x58, r.route)
        && safe::write::<u64>(ev + 0x60, desc.ptr as u64)
        && safe::write::<u16>(ev + 0x68, desc.payload_size)
        && safe::write::<u8>(ev + 0x78, 1);
    let ok = ok
        && match r.mode {
            PickupMode::Catch => safe::write_into(buf, &catch_payload(desc.id, r.target_eid)),
            PickupMode::Gather | PickupMode::Item => {
                match pickup_payload(desc.id, r.target_eid, r.mode) {
                    Some(p) => safe::write_into(buf, &p),
                    // Unreachable: `pickup_payload` only refuses `Catch`,
                    // which the arm above took. Treated as a write failure
                    // rather than asserted, because this must never panic.
                    None => false,
                }
            }
        };
    if !ok {
        // The event is the game's memory now; leaking one 0x80-byte object
        // beats calling a destructor we have not verified.
        return Err(format!("writing event fields at 0x{ev:X} failed; event leaked"));
    }
    let mut hdr = [0u8; 0x50];
    // Sized to this event's payload, not to the 13-byte one: reading 13 bytes
    // out of an 8-byte buffer would log five bytes of somebody else's heap.
    let mut back = vec![0u8; size];
    safe::read_into(ev + 0x30, &mut hdr);
    safe::read_into(buf, &mut back);
    crate::log!(
        "[event] ev=0x{ev:X} desc=0x{:X} id={} mode={:?} player={:08X} route=0x{:X} flag={} queue=0x{queue:X}",
        desc.ptr, desc.id, r.mode, r.player_eid, r.route, r.flag
    );
    crate::log!("[event]   ev+0x30..: {}", hex(&hdr));
    crate::log!("[event]   payload : {}", hex(&back));
    // SAFETY: `ev` came from the game's own allocator above and every field
    // the game reads was written through `safe::write` and verified; `queue`
    // and `desc.ptr` were both checked readable at their full sizes. This is
    // the call the game's builder makes at this point, on the game thread.
    unsafe { enqueue(queue, ev, desc.ptr, r.flag) };
    Ok(ev)
}


// ---------------------------------------------------------------------------
// Recorder: an observer hook on `enqueue` that logs what the game itself
// queues (the reference mod's F7 feature). Off unless toggled; capped.

static RECORDING: AtomicBool = AtomicBool::new(false);
static RECORDED: AtomicU32 = AtomicU32::new(0);
static RECORD_SEEN: Mutex<Vec<(String, u32)>> = Mutex::new(Vec::new());
pub const RECORD_CAP: u32 = 300;
const RECORD_PAYLOAD_MAX: usize = 48;

/// Toggle recording; returns the new state. Turning it off prints a summary.
pub fn toggle_recording() -> bool {
    let on = !RECORDING.load(Ordering::Relaxed);
    if on {
        RECORDED.store(0, Ordering::Relaxed);
        RECORD_SEEN.lock().unwrap_or_else(|e| e.into_inner()).clear();
        RECORDING.store(true, Ordering::Release);
    } else {
        RECORDING.store(false, Ordering::Release);
        let seen = RECORD_SEEN.lock().unwrap_or_else(|e| e.into_inner());
        crate::log!("[record] OFF: {} events, {} distinct descriptors", RECORDED.load(Ordering::Relaxed), seen.len());
        for (name, n) in seen.iter() {
            crate::log!("[record]   {n:5} x {name}");
        }
    }
    on
}

/// Callback on the game's `enqueue(queue, ev, desc, flag)`. Runs on whatever
/// thread queues the event, before the original. Must never panic.
///
/// # Safety
///
/// Only the trampoline installed on `enqueue` may call this, with the game's
/// own four arguments. `ev` and `desc` are treated as untrusted addresses and
/// read through `safe` only; nothing is written to game memory here.
pub unsafe extern "system" fn on_enqueue(_queue: usize, ev: usize, desc: usize, flag: usize) {
    watch_received(desc, ev);
    if !RECORDING.load(Ordering::Acquire) {
        return;
    }
    let n = RECORDED.fetch_add(1, Ordering::Relaxed);
    if n >= RECORD_CAP {
        if n == RECORD_CAP {
            crate::log!("[record] cap of {RECORD_CAP} events reached; stopping");
            RECORDING.store(false, Ordering::Release);
        }
        return;
    }
    let name = MODULE
        .get()
        .and_then(|m| actors::rtti_name(m, desc))
        .map(|s| actors::short_name(&s))
        .unwrap_or_else(|| format!("desc@0x{desc:X}"));
    {
        let mut seen = RECORD_SEEN.lock().unwrap_or_else(|e| e.into_inner());
        match seen.iter_mut().find(|(k, _)| *k == name) {
            Some(e) => e.1 += 1,
            None => seen.push((name.clone(), 1)),
        }
    }
    let f30: u32 = safe::read(ev + 0x30).unwrap_or(0);
    let f40: u32 = safe::read(ev + 0x40).unwrap_or(0);
    let f48: u64 = safe::read(ev + 0x48).unwrap_or(0);
    let eid: u32 = safe::read(ev + 0x50).unwrap_or(0);
    let f54: u32 = safe::read(ev + 0x54).unwrap_or(0);
    let route: u32 = safe::read(ev + 0x58).unwrap_or(0);
    let size: u16 = safe::read(ev + 0x68).unwrap_or(0);
    let f78: u8 = safe::read(ev + 0x78).unwrap_or(0);
    let mut payload = vec![0u8; (size as usize).min(RECORD_PAYLOAD_MAX)];
    let buf = safe::read_ptr(ev + 0x70).unwrap_or(0);
    if buf == 0 || !safe::read_into(buf, &mut payload) {
        payload.clear();
    }
    let dispatch: u32 = safe::read(desc + 0x1C).unwrap_or(u32::MAX);
    // SAFETY: `GetCurrentThreadId` takes no arguments and only reads the
    // calling thread's own TEB; it is sound on whatever thread the game
    // happens to be queueing an event from.
    let tid = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    crate::log!(
        "[record] #{n} {name} tid={tid} +30={f30} +40={f40} +48=0x{f48:X} eid={eid:08X} +54={f54} route=0x{route:X} size={size} +78={f78} flag={} disp={dispatch}: {}",
        flag & 0xFF, hex(&payload)
    );
}
