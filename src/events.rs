//! The write path: forging the game's own PickUpItem event and pushing it
//! onto the game's event queue from the game thread.
//!
//! Everything here mirrors what the game's own event builder does
//! (`FUN_1426B21A0`, build 25116796, decompiled in analysis/game-events.c):
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

pub use crate::payload::{PICKUP_DESCRIPTOR, PICKUP_ID_EXPECTED, PICKUP_PAYLOAD_SIZE};
use crate::payload::{hex, pickup_payload, PickupMode};

pub const MAX_DESCRIPTOR_ID: u32 = 0x1FFF;
/// Event object size from the allocator (`FUN_1413A9790`: alloc(0x80, 0x10)).
pub const EVENT_SIZE: usize = 0x80;

type PrepareFn = unsafe extern "system" fn() -> usize;
type DescByIdFn = unsafe extern "system" fn(usize, u32, u32) -> usize;
type AllocEventFn = unsafe extern "system" fn(usize, u32) -> usize;
type EnqueueFn = unsafe extern "system" fn(usize, usize, usize, u8);

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
}

/// Decode the four things the `desc_mask+queue` call site yields
/// (docs/cdloot-internals.md section 2): `E8` at +0 (prepare), `44 8B 05`
/// at +5 (desc_mask), `E8` at +0x11 (descriptor_by_id), `4C 8B 25` at +0x16
/// (queue slot).
pub fn resolve(m: &MainModule, anchors: &crate::game::Anchors) -> Result<EventApi, String> {
    let site = anchors.get("desc_mask+queue_site").ok_or("desc_mask+queue_site not found")?;
    let alloc_event = anchors.get("alloc_event").ok_or("alloc_event not found")?;
    let enqueue = anchors.get("enqueue").ok_or("enqueue not found")?;
    let img = m.bytes();
    let rel = |ins_off: usize, disp_off: usize, ins_len: usize| -> Result<usize, String> {
        let at = m.rva(site) + ins_off + disp_off;
        let d = img.get(at..at + 4).ok_or("site outside image")?;
        let disp = i32::from_le_bytes([d[0], d[1], d[2], d[3]]) as i64;
        let target = (site as i64 + ins_off as i64 + ins_len as i64 + disp) as usize;
        if !m.contains(target) {
            return Err(format!("rel target 0x{target:X} outside module"));
        }
        Ok(target)
    };
    Ok(EventApi {
        prepare: rel(0, 1, 5)?,
        desc_mask: rel(5, 3, 7)?,
        desc_by_id: rel(0x11, 1, 5)?,
        queue_slot: rel(0x16, 3, 7)?,
        alloc_event,
        enqueue,
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
    let f: DescByIdFn = unsafe { core::mem::transmute(api.desc_by_id) };
    let mask = desc_mask(api);
    let mut seen = 0u32;
    for id in 1..=MAX_DESCRIPTOR_ID {
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
    pub mode: PickupMode,
    pub player_eid: u32,
    pub route: u32,
    /// `enqueue` flag byte (the game's builder passes its caller's choice).
    pub flag: u8,
}

static API: OnceLock<EventApi> = OnceLock::new();
static MODULE: OnceLock<MainModule> = OnceLock::new();

pub fn set_module(m: MainModule) {
    let _ = MODULE.set(m);
}
static DESCRIPTOR: OnceLock<Descriptor> = OnceLock::new();
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

pub fn descriptor() -> Option<&'static Descriptor> {
    DESCRIPTOR.get()
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
pub unsafe extern "system" fn on_sweep(this: usize, item: usize, _r8: usize, _r9: usize) {
    let n = SWEEP_CALLS.fetch_add(1, Ordering::Relaxed);
    if n == 0 {
        let tid = windows_sys::Win32::System::Threading::GetCurrentThreadId();
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
        match send_pickup(&r) {
            Ok(ev) => {
                SENT.fetch_add(1, Ordering::Relaxed);
                crate::log!("[event] enqueued PickUpItem ({:?}) for eid={:08X}: event 0x{ev:X}", r.mode, r.target_eid);
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
    let desc = DESCRIPTOR.get().ok_or("descriptor not resolved")?;
    let size = desc.payload_size as usize;
    if size != PICKUP_PAYLOAD_SIZE {
        return Err(format!("payload size {size} != {PICKUP_PAYLOAD_SIZE}; refusing"));
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

    let prepare: PrepareFn = core::mem::transmute(api.prepare);
    let alloc: AllocEventFn = core::mem::transmute(api.alloc_event);
    let enqueue: EnqueueFn = core::mem::transmute(api.enqueue);

    prepare();
    let ev = alloc(0, desc.payload_size as u32);
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
    let payload = pickup_payload(desc.id, r.target_eid, r.mode);
    let ok = ok && safe::write_into(buf, &payload);
    if !ok {
        // The event is the game's memory now; leaking one 0x80-byte object
        // beats calling a destructor we have not verified.
        return Err(format!("writing event fields at 0x{ev:X} failed; event leaked"));
    }
    let mut hdr = [0u8; 0x50];
    let mut back = [0u8; PICKUP_PAYLOAD_SIZE];
    safe::read_into(ev + 0x30, &mut hdr);
    safe::read_into(buf, &mut back);
    crate::log!(
        "[event] ev=0x{ev:X} desc=0x{:X} id={} mode={:?} player={:08X} route=0x{:X} flag={} queue=0x{queue:X}",
        desc.ptr, desc.id, r.mode, r.player_eid, r.route, r.flag
    );
    crate::log!("[event]   ev+0x30..: {}", hex(&hdr));
    crate::log!("[event]   payload : {}", hex(&back));
    enqueue(queue, ev, desc.ptr, r.flag);
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
pub unsafe extern "system" fn on_enqueue(_queue: usize, ev: usize, desc: usize, flag: usize) {
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
    crate::log!(
        "[record] #{n} {name} tid={} +30={f30} +40={f40} +48=0x{f48:X} eid={eid:08X} +54={f54} route=0x{route:X} size={size} +78={f78} flag={} disp={dispatch}: {}",
        windows_sys::Win32::System::Threading::GetCurrentThreadId(), flag & 0xFF, hex(&payload)
    );
}
