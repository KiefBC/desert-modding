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
    PICKUP_PAYLOAD_SIZE, SKIN_DESCRIPTOR, SKIN_ID_EXPECTED, SKIN_PAYLOAD_SIZE,
};
use crate::payload::{catch_payload, hex, pickup_payload, skin_payload, PickupMode};
use crate::recorder::record_action;
/// Re-exported so `events::RECORD_CAP` keeps resolving; the constant itself
/// moved to [`crate::recorder`] when the cap policy did.
pub use crate::recorder::RECORD_CAP;

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
    /// Species class byte for a carcass, 0 otherwise. Only the skin/`[recv]`
    /// pairing reads it.
    pub cat: u8,
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
///
/// `ty` is the target's type byte, and it decides how much of this applies:
/// the owner record is read **only** by the 4/5/6 branch, while the status
/// component and the transform are read by every non-gimmick type - type 3
/// included, which is what the Firefly Colony catches as (section 15.5). So a
/// type-3 target is checked for the two the game will actually touch and not
/// held to a field its branch never reads.
fn creature_preflight(target: usize, ty: u8) -> Result<(), String> {
    let sub = safe::read_ptr(target + 0x68).ok_or("creature has no sub-object at +0x68")?;
    // `read_ptr` already rejects a null or unreadable slot; `readable` then
    // covers the fields the branch indexes out to.
    if (4..=6).contains(&ty)
        && safe::read_ptr(sub + 0x118).is_none_or(|owner| !safe::readable(owner, 0x30))
    {
        return Err("creature has no readable owner record at sub+0x118; not asking the steal check".into());
    }
    if safe::read_ptr(sub + 0x20).is_none_or(|status| !safe::readable(status, 0x340)) {
        return Err("creature status component unreadable".into());
    }
    // The transform: 0x538 because `FUN_1428330b0` hands `tf+0x4D8` to
    // `FUN_142af1640`, which reads fields at `+0x10`, `+0x50` and `+0x58` from
    // there - so the mapped range has to run to `tf+0x538`, where the old bound
    // of `0x4D8` stopped before the first of them.
    //
    // What is deliberately **not** checked is a pointer chain out of
    // `tf+0x4D8`. It was, for one build, on the reading that `FUN_142af1640`
    // dereferences `*(tf+0x4D8)` and then `*(P+8)`; that reading was wrong.
    // `tf+0x4D8` is the address of a struct embedded in the transform, not a
    // pointer to one, so `*(tf+0x4D8)` is that struct's first field and
    // chasing it lands on whatever that field happens to hold. In game the
    // check refused **9 of 16** carcasses - every one of them skinnable, three
    // refusals in a row switched auto-gather off, and the player was left with
    // an ini that said `AutoGather=1` and a looter that had stopped. The
    // readability bound below is the real guard, and it refused nothing in the
    // same session.
    if safe::read_ptr(sub + 0x1A0).is_none_or(|tf| !safe::readable(tf, 0x538)) {
        return Err("creature transform unreadable".into());
    }
    Ok(())
}

/// [`creature_preflight`] gated on the target's type byte, which is what
/// decides whether any of it applies: type 7 is a gimmick and reaches none of
/// those reads, and a type byte that will not read at all is left alone
/// deliberately - in [`would_steal`] an `Err` here costs the eid its whole
/// session, because the caller marks it owned, and the readability checks
/// standing directly in front of the game function do not go away either way.
fn preflight_unless_gimmick(target: usize) -> Result<(), String> {
    match desert_core::creature::type_byte(target) {
        Some(7) | None => Ok(()),
        Some(ty) => creature_preflight(target, ty),
    }
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
/// 7 dereferences the owner record, the status component and the transform
/// with no null check of its own, so [`creature_preflight`] verifies what it
/// will touch first (section 15). Gimmick targets are unaffected: nothing
/// about the type-7 path changed.
/// Game thread only. `Err` means "could not ask": treated as owned.
unsafe fn would_steal(api: &EventApi, player: usize, target: usize) -> Result<bool, String> {
    if api.steal_check == 0 || api.steal_ctx == 0 {
        return Err("steal check not resolved".into());
    }
    // Every type but the gimmick branch's 7 reaches the reads this checks.
    // The owner record belongs to the character branch's own test (`cVar5 == 4
    // || (byte)(cVar5 - 5) < 2`, types 4/5/6) and `creature_preflight` gates
    // on that itself; the status and transform reads after it are shared, so a
    // type-3 `Catch` target - the Firefly Colony, section 15.5 - needs them
    // checked exactly as a type-6 one does. An unreadable type byte is left
    // alone deliberately: `Err` here marks the eid owned for the whole
    // session, and the readability checks below still stand in front of the
    // call.
    preflight_unless_gimmick(target)?;
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
/// `TrocTrProcessLootingDeadDropOnceTimer`, the carcass-skinning event.
/// Resolved separately and optional in exactly the same way: without it
/// carcasses are simply not skinned and everything else carries on.
static SKIN_DESCRIPTOR_SLOT: OnceLock<Descriptor> = OnceLock::new();
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

pub fn set_skin_descriptor(d: Descriptor) {
    let _ = SKIN_DESCRIPTOR_SLOT.set(d);
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
/// The carcass a `Skin` event was last sent at, and the items that have arrived
/// since. Skinning has no equivalent of the gather path's record id to learn
/// against - a carcass's drops come off `characterinfo`, not a gimmick record -
/// so this exists for the log rather than for the yields cache: it turns a
/// scatter of `[recv]` lines into "this species gave these items".
///
/// It is a **correlation, not attribution**. The game reports every item the
/// player receives through one event, whatever produced it, so anything picked
/// up by hand inside the window lands here too. Getting that wrong by eye is
/// what made a copper pack and an apple look like carcass loot once; the log
/// line says "within Ns" so nobody reads it as proof.
static SKIN_PENDING: Mutex<Option<SkinYield>> = Mutex::new(None);

struct SkinYield {
    eid: u32,
    cat: u8,
    at: std::time::Instant,
    /// `(item key, count)`, merged by key in arrival order.
    items: Vec<(u32, u32)>,
}
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

/// Add one arrival to the open carcass window, if there is one.
fn note_skin_received(item: u32, count: u32) {
    let mut g = SKIN_PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(p) = g.as_mut() else { return };
    if p.at.elapsed() > YIELD_PAIR_WINDOW {
        return;
    }
    match p.items.iter_mut().find(|(k, _)| *k == item) {
        Some(e) => e.1 = e.1.saturating_add(count),
        None => p.items.push((item, count)),
    }
}

/// Print and clear the open carcass window once it has closed. Called from the
/// gatherer's tick, so the line lands a moment after the loot does rather than
/// waiting for the next carcass.
///
/// Resolves each key to a name the way the survey's bag listing does, through
/// `tables::item_index_by_key`; a key that will not resolve prints bare.
pub fn flush_skin_yield() {
    flush_skin_yield_inner(false);
}

/// [`flush_skin_yield`], but `force` prints a window that has not closed yet.
///
/// The send path forces it. Auto mode skins one carcass per `GatherInterval` -
/// 500 ms by default, well inside the 2 s pairing window - so waiting for the
/// window to close before printing meant a run of carcasses clobbered each
/// other and only the last one was ever logged: 12 skins produced 5 lines in
/// game. Forcing loses nothing, because the items arrive 1-3 ms after the event
/// and the next send is hundreds of milliseconds later. The line reports the
/// window it actually had rather than the cap, so a forced flush says
/// "within 0.5s" and cannot be mistaken for a full one.
fn flush_skin_yield_inner(force: bool) {
    let done = {
        let mut g = SKIN_PENDING.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_ref() {
            Some(p) if force || p.at.elapsed() > YIELD_PAIR_WINDOW => g.take(),
            _ => None,
        }
    };
    let Some(p) = done else { return };
    let secs = format!("{:.1}", p.at.elapsed().as_secs_f32().min(YIELD_PAIR_WINDOW.as_secs_f32()));
    if p.items.is_empty() {
        crate::log!(
            "[skin] carcass cat={:02X} eid={:08X} gave nothing within {secs}s",
            p.cat, p.eid
        );
        return;
    }
    let named: Vec<String> = match MODULE.get() {
        Some(m) => p
            .items
            .iter()
            .map(|&(key, n)| match crate::tables::item_index_by_key(m, key)
                .and_then(|i| crate::tables::item_record(m, i))
                .and_then(crate::tables::item_record_name)
            {
                Some(name) => format!("{name} key={key} x{n}"),
                None => format!("key={key} x{n}"),
            })
            .collect(),
        None => p.items.iter().map(|&(key, n)| format!("key={key} x{n}")).collect(),
    };
    crate::log!(
        "[skin] carcass cat={:02X} eid={:08X} gave (within {secs}s): {}",
        p.cat, p.eid, named.join("; ")
    );
}

fn note_received(item: u32, count: u32) {
    let sent = LAST_SENT.lock().unwrap_or_else(|e| e.into_inner()).take();
    let Some((record, when)) = sent else { return };
    if when.elapsed() > YIELD_PAIR_WINDOW {
        return;
    }
    // Decide under the lock, log after dropping it. `crate::log!` takes the
    // logger's own mutex, which all four subsystems in this one plugin share,
    // and this runs on whatever game thread queued the event - so holding
    // `YIELDS` across it nests two process-wide locks in an order nothing else
    // guarantees. Same rule the recorder's `log_census` follows.
    let learned = {
        let mut y = YIELDS.lock().unwrap_or_else(|e| e.into_inner());
        match y.iter_mut().find(|(r, _, _)| *r == record) {
            Some(e) => {
                // Keep the largest count seen: the stacking rule uses it as a margin.
                if e.1 == item && count > e.2 {
                    e.2 = count;
                    YIELD_DIRTY.store(true, Ordering::Release);
                }
                false
            }
            None => {
                y.push((record, item, count));
                YIELD_DIRTY.store(true, Ordering::Release);
                true
            }
        }
    };
    if learned {
        crate::log!("[yield] record {record} gives item {item} (x{count}); learned");
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
    note_skin_received(item, count);
}

pub fn descriptor() -> Option<&'static Descriptor> {
    DESCRIPTOR.get()
}

pub fn catch_descriptor() -> Option<&'static Descriptor> {
    CATCH_DESCRIPTOR_SLOT.get()
}

pub fn skin_descriptor() -> Option<&'static Descriptor> {
    SKIN_DESCRIPTOR_SLOT.get()
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
        if r.mode == PickupMode::Skin {
            // **Before** the event goes out, not after. The grant runs on
            // another thread and its items have been seen arriving 1-3 ms after
            // the enqueue, so a window opened afterwards is a race the grant
            // sometimes wins: the items land with nothing open and are dropped.
            // In game that silently lost 3 of 11 carcasses' drops, and the
            // misses were indistinguishable from the hits - same species, same
            // dead-drop state before and after - because the difference was
            // thread timing, not anything about the carcass.
            //
            // Flushing the previous window here too keeps back-to-back skins
            // from pooling into one line; forced, because at one skin per
            // `GatherInterval` the previous window has not closed yet.
            flush_skin_yield_inner(true);
            *SKIN_PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(SkinYield {
                eid: r.target_eid,
                cat: r.cat,
                at: std::time::Instant::now(),
                items: Vec::new(),
            });
        }
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
                    PickupMode::Skin => "ProcessLootingDeadDrop",
                    PickupMode::Gather | PickupMode::Item => "PickUpItem",
                };
                crate::log!("[event] enqueued {what} ({:?}) for eid={:08X}: event 0x{ev:X}", r.mode, r.target_eid);
            }
            Err(e) => {
                // The window was opened before the attempt, so close it again:
                // nothing was queued, and leaving it would charge the next
                // carcass's drops to this one.
                if r.mode == PickupMode::Skin {
                    *SKIN_PENDING.lock().unwrap_or_else(|e| e.into_inner()) = None;
                }
                crate::log!("[event] NOT sent for eid={:08X}: {e}", r.target_eid);
            }
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
    // Three descriptors, three payload layouts: neither a catch nor a skinning
    // is a `PickUpItem` with a different mode byte, each is its own event
    // (section 15, and `docs/findings/2026-09-15-skinning.md` section 3).
    let (desc, want) = match r.mode {
        PickupMode::Catch => (
            CATCH_DESCRIPTOR_SLOT.get().ok_or("catch descriptor not resolved")?,
            CATCH_PAYLOAD_SIZE,
        ),
        PickupMode::Skin => (
            SKIN_DESCRIPTOR_SLOT.get().ok_or("skin descriptor not resolved")?,
            SKIN_PAYLOAD_SIZE,
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
    // Every request but a `Skin`, gather nodes included: the game's own
    // interaction handler asks this for every target with no category filter
    // (section 14).
    //
    // **`Skin` deliberately does not ask it.** This is the one behavioural
    // divergence between skinning and the other three modes, it was decided
    // rather than defaulted, and there are three reasons for it
    // (`docs/findings/2026-09-15-skinning.md` sections 3 and 7):
    //
    // 1. A carcass cannot be stolen in game. Skinning is not a take-or-steal
    //    interaction and no ownership prompt is ever shown at one, so there is
    //    no vanilla behaviour here for this check to preserve.
    // 2. The game's own carcass path does not make this call either. It uses a
    //    different gate entirely - `FUN_142a75360` reads the owner id at
    //    `*(*(target+0x68)+0x118)+0x20`, reaches it only when
    //    `status+0x273 != 0`, and refuses only when that id is not `-1` - so
    //    asking the generic mode-7 take-or-steal check here would hold
    //    skinning to a test vanilla never applies to it.
    // 3. The failure arms of this call are permanent. Both `Ok(true)` and
    //    `Err` run `mark_owned`, which lasts for the session, so a carcass
    //    whose owner record or status component happened to read oddly once
    //    would stay unskinnable until the game restarts.
    //
    // What is **not** skipped is the memory safety. `preflight_unless_gimmick`
    // still runs, because the dead-drop handler dereferences the very fields
    // `creature_preflight` proves readable, and with no null check of its own:
    // `FUN_142ab8140` reads `*(u16*)(*(*(carcass+0x68)+0x20)+0x30)` and
    // `FUN_1428330b0` reads `*(*(actor+0x68)+0x1A0)+0x4D8`, items 2 and 3 of
    // that document's crash surface. A fault on either is a fault on the game
    // thread, which is a crash to desktop. It is the steal *call* that is
    // skipped here, never the reads that make it safe to point an event at a
    // creature at all.
    match r.mode {
        // Not `preflight_unless_gimmick`: its `None => Ok(())` leniency is
        // justified only inside `would_steal`, where an `Err` would cost the
        // eid its whole session through `mark_owned` and where readability
        // checks still stand between it and the game call. Neither holds here
        // - `Skin` never marks anything owned, and nothing stands after this
        // point except building the event and queueing it. An unreadable type
        // byte is `*(actor+0x88)` failing, which is itself item 4 of the
        // findings' crash surface (a stale owner back-pointer mid-despawn), so
        // it is the strongest signal to refuse on, not to wave through: the
        // carcass was classified on the plugin thread and may have despawned
        // before this sweep. `Some(7)` cannot occur, because `Kind::Carcass`
        // requires type 3 or 6.
        PickupMode::Skin => match desert_core::creature::type_byte(r.target_actor) {
            Some(ty) => creature_preflight(r.target_actor, ty)?,
            None => return Err("carcass type byte unreadable (despawned?); skipped".into()),
        },
        PickupMode::Gather | PickupMode::Item | PickupMode::Catch => {
            // SAFETY: `would_steal` requires the game thread, which
            // `send_pickup` itself requires and the sweep hook - its only
            // caller - provides.
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
            PickupMode::Skin => safe::write_into(buf, &skin_payload(desc.id, r.target_eid)),
            PickupMode::Gather | PickupMode::Item => {
                match pickup_payload(desc.id, r.target_eid, r.mode) {
                    Some(p) => safe::write_into(buf, &p),
                    // Unreachable: `pickup_payload` refuses only `Catch` and
                    // `Skin`, which the two arms above took. Treated as a
                    // write failure rather than asserted, because this must
                    // never panic.
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
/// Lines **logged** this session, not events observed. Every read and write is
/// made with `RECORD_SEEN` held, which is what makes the cap exact: the
/// read-decide-charge in `tally_locked` is then one indivisible step, so exactly
/// one event can be the one that fills the cap. `Relaxed` is enough for the same
/// reason - the mutex supplies the ordering.
static RECORDED: AtomicU32 = AtomicU32::new(0);
/// One row per descriptor *pointer* seen while recording.
static RECORD_SEEN: Mutex<Vec<DescTally>> = Mutex::new(Vec::new());
const RECORD_PAYLOAD_MAX: usize = 48;

/// One descriptor's tally. Keyed on the descriptor pointer, not on the
/// resolved name: descriptor objects are per-class singletons, so the pointer
/// is a stable identity, and keying on it means `actors::rtti_name` (several
/// `ReadProcessMemory` calls) runs once per descriptor instead of once per
/// event. The hot path is then a scan of this short vector.
struct DescTally {
    desc: usize,
    /// Resolved lazily, on first sight of `desc`.
    name: String,
    /// Every event seen for this descriptor, logged or not.
    observed: u32,
    /// Lines actually logged for it.
    logged: u32,
}

/// Run `f` with `RECORD_SEEN` held, and drop the lock before returning.
///
/// Nothing inside `f` may log or read game memory: `crate::log!` takes the
/// process-wide logger mutex shared with every other subsystem, and
/// `actors::rtti_name` is several syscalls. Both belong outside this.
fn with_seen<T>(f: impl FnOnce(&mut Vec<DescTally>) -> T) -> T {
    let mut seen = RECORD_SEEN.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut seen)
}

/// One observed event's bookkeeping, all of it under the lock: tally the
/// observation, ask [`record_action`] what to do, and charge the logged counts
/// if it says to log.
///
/// `name` is the freshly resolved name to insert when `desc` has no row yet;
/// pass `None` to only look for an existing row. Returns `None` exactly when
/// `desc` is absent and `name` was `None`, which is the caller's cue to
/// resolve the name with the lock down and come back.
fn tally_locked(seen: &mut Vec<DescTally>, desc: usize, name: Option<String>) -> Option<Tallied> {
    if !seen.iter().any(|t| t.desc == desc) {
        // Another thread may have inserted this same pointer while the lock
        // was down for the name resolution; then `name` is simply dropped.
        seen.push(DescTally { desc, name: name?, observed: 0, logged: 0 });
    }
    let total = RECORDED.load(Ordering::Relaxed);
    let t = seen.iter_mut().find(|t| t.desc == desc)?;
    t.observed = t.observed.saturating_add(1);
    let action = record_action(t.logged, total);
    let name = if action.log {
        t.logged = t.logged.saturating_add(1);
        RECORDED.store(total.saturating_add(1), Ordering::Relaxed);
        Some(t.name.clone())
    } else {
        // The flood's common case: tallied and dropped, so not even the name
        // is cloned.
        None
    };
    Some(Tallied { stop: action.stop, n: total, name })
}

/// The result of [`tally_locked`].
struct Tallied {
    /// This event filled the global cap: print the census and stop. Never set
    /// unless `name` is `Some`, because [`record_action`] never stops without
    /// logging.
    stop: bool,
    /// The `#n` for the log line: the 0-based index of this line among the
    /// lines logged since recording was switched on.
    n: u32,
    /// The descriptor's short name, cloned only when the line is logged. This
    /// is also how "log this one" reaches the caller - `Some` *is* the
    /// decision, so the bit is carried once rather than in two places that
    /// could disagree.
    name: Option<String>,
}

/// One session's census: total lines logged, then a row per descriptor.
type Census = (u32, Vec<(String, u32, u32)>);

/// Claim the end of a recording session and take its census, or `None` if it
/// was already over.
///
/// The claim and the snapshot happen together under `RECORD_SEEN`, and
/// [`start_session`] clears under the same lock, which is what makes the census
/// print exactly once. Both endings race: the cap ends a session from a game
/// thread while F7 ends it from the plugin thread, and a plain load-then-store
/// of `RECORDING` let both decide they were the one ending it - printing the
/// census twice, or clearing the tallies between the winner's claim and its
/// snapshot so the census came out empty. The `swap` settles it: whoever turns
/// `RECORDING` off owns the census, and the loser gets `None` and prints
/// nothing.
///
/// `only_if_capped` is the cap path's extra condition; see
/// [`end_capped_session`].
fn claim_session_end(only_if_capped: bool) -> Option<Census> {
    with_seen(|seen| {
        // Under the lock, so it cannot be a reset that lands mid-decision.
        if only_if_capped && RECORDED.load(Ordering::Relaxed) < RECORD_CAP {
            return None;
        }
        if !RECORDING.swap(false, Ordering::AcqRel) {
            return None;
        }
        let rows = seen.iter().map(|t| (t.name.clone(), t.observed, t.logged)).collect();
        Some((RECORDED.load(Ordering::Relaxed), rows))
    })
}

/// The user's F7: end whatever session is running.
fn end_session() -> Option<Census> {
    claim_session_end(false)
}

/// The cap: end the session only if the flag still carries the one that filled
/// it.
///
/// The extra condition guards a stale decision. A thread that was told to stop
/// drops the lock, reads eight event fields and writes its own log line before
/// it gets here; if F7 turned recording off and on again inside that window, the
/// bare `swap` would kill the *new* session and print its near-empty census.
/// Requiring `RECORDED >= RECORD_CAP` rules that out, because [`start_session`]
/// zeroes the counter under the same lock. Reaching the window needs two
/// keypresses inside a few microseconds - the hotkey poll alone is 30 ms - so
/// this is belt and braces, not a bug being papered over.
fn end_capped_session() -> Option<Census> {
    claim_session_end(true)
}

/// Start a fresh session: clear the tallies, then arm the hook.
///
/// The clear is sequenced before the `Release` store, so any `on_enqueue` that
/// passes the `Acquire` gate sees zeroed counters. It shares `RECORD_SEEN` with
/// [`end_session`], so it cannot interleave with a census snapshot.
fn start_session() {
    with_seen(|seen| {
        seen.clear();
        RECORDED.store(0, Ordering::Relaxed);
    });
    RECORDING.store(true, Ordering::Release);
}

/// Print a census. `why` labels it so the two endings are told apart.
///
/// Must be called with `RECORD_SEEN` **not** held - it takes the logger's own
/// mutex, which every subsystem in this one plugin shares, and the two are
/// never nested. That is why [`end_session`] hands the rows over instead of
/// letting this reach for them.
fn log_census(why: &str, (logged, rows): Census) {
    let observed = rows.iter().map(|r| r.1).fold(0u32, u32::saturating_add);
    crate::log!(
        "[record] {why}: {observed} events observed, {logged} logged, {} distinct descriptors",
        rows.len()
    );
    for (name, observed, logged) in rows {
        if observed == logged {
            crate::log!("[record]   {observed:5} x {name}");
        } else {
            crate::log!("[record]   {observed:5} x {name} ({logged} logged)");
        }
    }
}

/// Toggle recording; returns the new state. Ending a session - by this or by
/// the cap - prints its census.
pub fn toggle_recording() -> bool {
    match end_session() {
        Some(census) => {
            log_census("OFF", census);
            false
        }
        None => {
            start_session();
            true
        }
    }
}

/// Callback on the game's `enqueue(queue, ev, desc, flag)`. Runs on whatever
/// thread queues the event, before the original. Must never panic.
///
/// While recording, every event is tallied; at most
/// [`RECORD_PER_DESC`](crate::recorder::RECORD_PER_DESC) lines per descriptor
/// and [`RECORD_CAP`] lines in all are logged.
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
    // Several game threads run this at once (tids 33884 and 59080 carried 370
    // and 830 of one live log's 1200 recorded events; 36116 in that log is the
    // plugin's own thread and never gets here), so: tally, decide, drop the lock,
    // and only then log. The lock is never held across `crate::log!` (whose
    // mutex is shared with every other subsystem in this plugin) nor across
    // `actors::rtti_name` (several `ReadProcessMemory` calls), which is why
    // the miss below drops the lock to resolve the name and comes back.
    let t = match with_seen(|seen| tally_locked(seen, desc, None)) {
        Some(t) => t,
        None => {
            let name = MODULE
                .get()
                .and_then(|m| actors::rtti_name(m, desc))
                .map(|s| actors::short_name(&s))
                .unwrap_or_else(|| format!("desc@0x{desc:X}"));
            match with_seen(|seen| tally_locked(seen, desc, Some(name))) {
                Some(t) => t,
                // Cannot happen: `tally_locked` inserts when given a name.
                // Returning beats asserting in a callback that must not panic.
                None => return,
            }
        }
    };
    let (Some(name), n) = (t.name, t.n) else {
        // Tallied, not logged: this descriptor has had its share of lines.
        return;
    };
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
    if t.stop {
        // Fires exactly once: the deciding read-and-charge of `RECORDED`
        // happened under `RECORD_SEEN`, so exactly one event can be the one
        // that filled the cap.
        // The census has to be printed here, not left for the user's next F7:
        // that press sees `RECORDING` already off, so it reads as "start" and
        // would clear these tallies without printing them. `end_capped_session`
        // also settles the race against an F7 landing in this same instant.
        if let Some(census) = end_capped_session() {
            crate::log!("[record] cap of {RECORD_CAP} logged lines reached; stopping");
            log_census("CAPPED", census);
        }
    }
}
