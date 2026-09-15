//! Read-only view of the game's actor manager and its actors.
//!
//! Layout knowledge:
//! - The manager singleton is reachable through a global pointer in one of the
//!   exe's writable data sections; the object starts with the
//!   `ClientActorManager` vtable and is 0x498 bytes.
//! - The per-frame sweep walks `Actor** items` at +0x438 with `u32 count` at +0x440.
//!   Other `{u32 count; u32 capacity; Actor** items}` headers inside the object
//!   are transient work lists.
//! - `actor+0x60` is the entity id (u32; the player's starts with 0xA0).
//! - `actor+0x68 -> +0x1A0` is the transform; position floats at +0xB4/+0xB8/+0xBC,
//!   plus an attachment offset at +0xEC/+0xF0/+0xF4 when the u32 at +0xC8 is
//!   neither 0 nor 0xFFFFFFFF.
//!
//! What a creature *is* - the actor type byte and the bug/fish class lists -
//! is not here: it lives in `desert_core::creature`, because Desert
//! Gatherer's catch-count hook has to agree with this file about it. What
//! stays here is how the looter *reaches* those bytes on an arbitrary actor
//! (the `ClientStatusActorComponent` is found by RTTI name, not by a fixed
//! slot) and what it does with the answer.

use desert_core::creature::{self, CatchClass};

use crate::module::MainModule;
use crate::safe;

pub const MAX_ACTORS: usize = 4000;
/// Manager object size from its constructor's allocation (build 25116796).
pub const MANAGER_SIZE: usize = 0x498;
/// Offsets of the per-frame sweep list (from the single caller of `area_sweep`).
pub const SWEEP_ITEMS_OFF: usize = 0x438;
pub const SWEEP_COUNT_OFF: usize = 0x440;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub fn dist(&self, o: &Vec3) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2) + (self.z - o.z).powi(2)).sqrt()
    }
}

/// Scan writable data sections for a pointer to an object whose first qword is
/// one of `vtables`. Returns (slot address, object address).
pub fn find_manager(m: &MainModule, vtables: &[u64]) -> Option<(usize, usize)> {
    let img = m.bytes();
    for s in m.headers.sections.iter().filter(|s| s.is_data()) {
        let start = s.virtual_address as usize;
        let end = (start + s.virtual_size.max(s.raw_size) as usize).min(img.len());
        let mut off = start & !7;
        while off + 8 <= end {
            // `end` is clamped to `img.len()`, so the loop condition already
            // proves this is in range; bail out rather than index blindly.
            let Some(word) = img.get(off..off + 8).and_then(|b| <[u8; 8]>::try_from(b).ok()) else {
                break;
            };
            let p = usize::from_le_bytes(word);
            // Heap objects live outside the module; skip nulls and self-references.
            if (0x10000..0x7FFF_FFFF_FFFF).contains(&p) && !m.contains(p) {
                if let Some(vt) = safe::read::<u64>(p) {
                    if vtables.contains(&vt) {
                        return Some((m.base + off, p));
                    }
                }
            }
            off += 8;
        }
    }
    None
}

/// Re-read the global slot; the manager can be re-created on loading screens.
pub fn find_manager_current(w: &crate::game::World) -> Option<usize> {
    safe::read_ptr(w.manager_slot).filter(|&p| safe::readable(p, MANAGER_SIZE))
}

#[derive(Debug, Clone, Copy)]
pub struct ActorList {
    pub header_off: usize,
    pub count: u32,
    pub capacity: u32,
    pub items: usize,
}

impl ActorList {
    /// Non-null, readable actor pointers, at most MAX_ACTORS.
    pub fn actors(&self) -> Vec<usize> {
        let n = (self.count as usize).min(MAX_ACTORS);
        (0..n)
            .filter_map(|i| safe::read_ptr(self.items + i * 8))
            .filter(|&a| safe::readable(a, 0x100))
            .collect()
    }
}

/// Container object hanging off the manager (build 25116796): `ClientActorContainer`.
pub const CONTAINER_OFF: usize = 0x008;
/// The player actor hangs directly off the manager; it is not in the eid map.
pub const PLAYER_OFF: usize = 0x050;

pub fn player_actor(manager: usize) -> Option<usize> {
    safe::read_ptr(manager + PLAYER_OFF)
        .filter(|&a| safe::readable(a, 0x100) && actor_eid(a).is_some_and(is_player_eid))
}
/// How much of the container to scan for list headers.
pub const CONTAINER_SCAN: usize = 0x800;

/// Every `{u32 count; u32 capacity; Actor** items}`-shaped header inside `obj[..size]`.
pub fn candidate_lists_in(obj: usize, size: usize) -> Vec<ActorList> {
    let mut out = Vec::new();
    for off in (0x08..size.saturating_sub(0x10)).step_by(8) {
        let (Some(count), Some(capacity)) =
            (safe::read::<u32>(obj + off), safe::read::<u32>(obj + off + 4))
        else {
            continue;
        };
        let Some(items) = safe::read_ptr(obj + off + 8) else { continue };
        if count == 0 || capacity == 0 || count > capacity || capacity > 0x10000 {
            continue;
        }
        if !safe::readable(items, 8 * count as usize) {
            continue;
        }
        out.push(ActorList { header_off: off, count, capacity, items });
    }
    out
}

/// Lists inside the manager itself.
pub fn candidate_lists(manager: usize) -> Vec<ActorList> {
    candidate_lists_in(manager, MANAGER_SIZE)
}

/// Lists inside the `ClientActorContainer` the manager points at.
pub fn container_lists(manager: usize) -> Option<(usize, Vec<ActorList>)> {
    let c = safe::read_ptr(manager + CONTAINER_OFF)?;
    if !safe::readable(c, 0x40) {
        return None;
    }
    Some((c, candidate_lists_in(c, CONTAINER_SCAN)))
}

/// A candidate evaluated once: its readable actors and whether the player is among them.
pub struct Evaluated {
    pub source: &'static str,
    pub list: ActorList,
    pub actors: Vec<usize>,
    pub has_player: bool,
}

pub fn evaluate(source: &'static str, lists: Vec<ActorList>) -> Vec<Evaluated> {
    lists
        .into_iter()
        .map(|list| {
            let actors = list.actors();
            let has_player = actors.iter().any(|&a| actor_eid(a).is_some_and(is_player_eid));
            Evaluated { source, list, actors, has_player }
        })
        .collect()
}

pub fn evaluate_lists(manager: usize) -> Vec<Evaluated> {
    let mut out = evaluate("mgr", candidate_lists(manager));
    if let Some((_, lists)) = container_lists(manager) {
        out.extend(evaluate("ctr", lists));
    }
    out
}

/// The real actor list: contains the player, and among those the most entries.
pub fn pick_actor_list(evaluated: Vec<Evaluated>) -> Option<Evaluated> {
    evaluated
        .into_iter()
        .max_by_key(|e| (e.has_player as u32, e.actors.len() as u32, e.list.capacity))
}

pub struct RawItem {
    pub value: u64,
    pub readable: bool,
    pub eid: Option<u32>,
    pub class: Option<String>,
    pub sub28_class: Option<String>,
}

/// Raw qwords at the start of the item array, with what they look like.
pub fn raw_items(m: &MainModule, list: &ActorList, n: usize) -> Vec<RawItem> {
    (0..n.min(list.count as usize))
        .filter_map(|i| {
            let value: u64 = safe::read(list.items + i * 8)?;
            let p = value as usize;
            let readable = p != 0 && safe::readable(p, 0x100);
            let (eid, class, sub28_class) = if readable {
                (
                    actor_eid(p),
                    rtti_name(m, p).map(|s| short_name(&s)),
                    safe::read_ptr(p + 0x28).and_then(|s| rtti_name(m, s)).map(|s| short_name(&s)),
                )
            } else {
                (None, None, None)
            };
            Some(RawItem { value, readable, eid, class, sub28_class })
        })
        .collect()
}

pub fn actor_eid(actor: usize) -> Option<u32> {
    safe::read(actor + 0x60)
}

pub fn is_player_eid(eid: u32) -> bool {
    eid & 0xFF00_0000 == 0xA000_0000
}

pub fn actor_position(actor: usize) -> Option<Vec3> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let tf = safe::read_ptr(sub + 0x1A0)?;
    if !safe::readable(tf, 0xF8) {
        return None;
    }
    let mut p = Vec3 {
        x: safe::read(tf + 0xB4)?,
        y: safe::read(tf + 0xB8)?,
        z: safe::read(tf + 0xBC)?,
    };
    let attach: u32 = safe::read(tf + 0xC8)?;
    if attach != 0 && attach != 0xFFFF_FFFF {
        let (ox, oy, oz): (f32, f32, f32) =
            (safe::read(tf + 0xEC)?, safe::read(tf + 0xF0)?, safe::read(tf + 0xF4)?);
        if ox.is_finite() && oy.is_finite() && oz.is_finite() {
            p.x += ox;
            p.y += oy;
            p.z += oz;
        }
    }
    if p.x.is_finite() && p.y.is_finite() && p.z.is_finite() {
        Some(p)
    } else {
        None
    }
}

/// MSVC RTTI class name of a live object, if its vtable is inside the module.
pub fn rtti_name(m: &MainModule, obj: usize) -> Option<String> {
    let vt = safe::read_ptr(obj)?;
    if !m.contains(vt) {
        return None;
    }
    let col = safe::read_ptr(vt - 8)?;
    if !m.contains(col) || safe::read::<u32>(col)? != 1 {
        return None;
    }
    let td_rva: u32 = safe::read(col + 0xC)?;
    let name = m.base + td_rva as usize + 0x10;
    if !m.contains(name) {
        return None;
    }
    let s = safe::read_cstr(name, 96)?;
    if s.starts_with(".?A") {
        Some(s)
    } else {
        None
    }
}

/// RTTI names of the component objects hanging off `actor+0x68` at +0..+0x80.
pub fn component_names(m: &MainModule, actor: usize) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    if let Some(sub) = safe::read_ptr(actor + 0x68) {
        for off in (0..0x88).step_by(8) {
            if let Some(c) = safe::read_ptr(sub + off) {
                if let Some(n) = rtti_name(m, c) {
                    out.push((off, n));
                }
            }
        }
    }
    out
}

/// `ClientGimmickActorComponent` -> `Gimmick`, for compact survey lines.
pub fn component_label(short: &str) -> String {
    short
        .trim_start_matches("Client")
        .trim_end_matches("ActorComponent")
        .trim_end_matches("Component")
        .to_string()
}

/// Turn `.?AVClientGimmickActorComponent@pa@@` into `ClientGimmickActorComponent`.
pub fn short_name(mangled: &str) -> String {
    mangled
        .trim_start_matches(".?AV")
        .trim_start_matches(".?AU")
        .split('@')
        .next()
        .unwrap_or(mangled)
        .to_string()
}

/// Every writable-section global that points at a heap object with an RTTI
/// class name: (slot RVA, object, class). Deduplicated by class, first slot kept.
pub fn global_object_census(m: &MainModule) -> Vec<(usize, usize, String)> {
    let img = m.bytes();
    let mut out: Vec<(usize, usize, String)> = Vec::new();
    for s in m.headers.sections.iter().filter(|s| s.is_data()) {
        let start = s.virtual_address as usize;
        let end = (start + s.virtual_size.max(s.raw_size) as usize).min(img.len());
        let mut off = start & !7;
        while off + 8 <= end {
            // `end` is clamped to `img.len()`, so the loop condition already
            // proves this is in range; bail out rather than index blindly.
            let Some(word) = img.get(off..off + 8).and_then(|b| <[u8; 8]>::try_from(b).ok()) else {
                break;
            };
            let p = usize::from_le_bytes(word);
            if (0x10000..0x7FFF_FFFF_FFFF).contains(&p) && !m.contains(p) {
                if let Some(name) = rtti_name(m, p) {
                    let short = short_name(&name);
                    if !out.iter().any(|(_, _, n)| *n == short) {
                        out.push((off, p, short));
                    }
                }
            }
            off += 8;
        }
    }
    out
}

/// Pointer fields of an object that reference RTTI-named objects: (offset, target, class).
pub fn object_fields(m: &MainModule, obj: usize, size: usize) -> Vec<(usize, usize, String)> {
    (0..size)
        .step_by(8)
        .filter_map(|off| {
            let p = safe::read_ptr(obj + off)?;
            let name = rtti_name(m, p)?;
            Some((off, p, short_name(&name)))
        })
        .collect()
}

/// What the reference mod reads off a `ClientGimmickActorComponent`.
#[derive(Debug, Default, Clone)]
pub struct GimmickInfo {
    pub comp_off: usize,
    /// `comp+0xC0 -> u32`; 0xFFFFFFFF means "no instance".
    pub instance: Option<u32>,
    /// `comp+0xE0 -> u16` record id into gimmickinfo/iteminfo.
    pub record_id: Option<u16>,
    /// `comp+0xE0 -> byte +5`.
    pub kind: Option<u8>,
}

pub fn gimmick_info(m: &MainModule, actor: usize) -> Option<GimmickInfo> {
    let (off, _) = component_names(m, actor)
        .into_iter()
        .find(|(_, n)| n.contains("ClientGimmickActorComponent"))?;
    let sub = safe::read_ptr(actor + 0x68)?;
    let comp = safe::read_ptr(sub + off)?;
    let mut g = GimmickInfo { comp_off: off, ..Default::default() };
    if let Some(p) = safe::read_ptr(comp + 0xC0) {
        g.instance = safe::read(p);
    }
    if let Some(p) = safe::read_ptr(comp + 0xE0) {
        g.record_id = safe::read(p);
        g.kind = safe::read(p + 5);
    }
    Some(g)
}

/// The entity-id -> actor map inside `ClientActorContainer` (build 25116796):
/// `{u32 ?, u32 ?, u32 count, u32 buckets, Key* keys, Slot** values}` at +0x88.
/// The value array has `count` slot pointers; the key array is a hash chain
/// (its length is not `count`) and is not used for enumeration.
pub const EIDMAP_OFF: usize = 0x88;

#[derive(Debug, Clone, Copy)]
pub struct EidMap {
    pub count: u32,
    pub capacity: u32,
    pub keys: usize,
    pub values: usize,
}

pub fn eid_map(container: usize) -> Option<EidMap> {
    let base = container + EIDMAP_OFF;
    let count: u32 = safe::read(base + 0x8)?;
    let capacity: u32 = safe::read(base + 0xC)?;
    let keys = safe::read_ptr(base + 0x10)?;
    let values = safe::read_ptr(base + 0x18)?;
    // `capacity` is the key hash's bucket count, not an upper bound on `count`
    // (207 entries with 167 was seen live), and it grows with session uptime:
    // four F11 presses ~90 minutes in read 74997, 75283, 78305 and 78942 while
    // `count` stayed 2394 throughout. It is therefore **not** range-checked -
    // it once was, against 0x10000, which silently shut the whole looter down
    // an hour into every session, because `eid_map` is the single gate in front
    // of `game::scene`. Nothing enumerates by it either; `entries()` walks the
    // value array by `count` alone, so the count's own bound below, the value
    // array's readability, and the per-slot checks in `entries()` are the real
    // safety gate. `capacity` is still read and kept on the struct to document
    // the header's shape and to give the regression test something to assert;
    // the survey's rejection diagnostic that made this findable reads the same
    // word straight out of game memory, not off this struct.
    if count == 0 || count > 0x10000 {
        return None;
    }
    if !safe::readable(values, 8 * count as usize) {
        return None;
    }
    Some(EidMap { count, capacity, keys, values })
}

/// Inside each value slot: `+0x04 u32 eid`, `+0x08 Actor*`.
pub const SLOT_EID_OFF: usize = 0x04;
pub const SLOT_ACTOR_OFF: usize = 0x08;

impl EidMap {
    /// (eid from the slot, actor pointer from the slot), readable only.
    /// The key array is a hash chain and is not index-aligned with the values.
    pub fn entries(&self) -> Vec<(u32, usize)> {
        (0..self.count as usize)
            .filter_map(|i| {
                let slot = safe::read_ptr(self.values + i * 8)?;
                let eid: u32 = safe::read(slot + SLOT_EID_OFF)?;
                let actor = safe::read_ptr(slot + SLOT_ACTOR_OFF)?;
                safe::readable(actor, 0x100).then_some((eid, actor))
            })
            .collect()
    }
}

/// Resource path of a gimmick node, present once the game has filled in the
/// node's interaction data: `Catch component + 0xE0 -> chars` (also mirrored
/// at `Attack + 0x70`).
pub const CATCH_PATH_OFF: usize = 0xE0;
pub const ATTACK_PATH_OFF: usize = 0x70;

pub fn node_path(m: &MainModule, actor: usize) -> Option<String> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let comps = component_names(m, actor);
    for (needle, off) in [("ClientCatchActorComponent", CATCH_PATH_OFF), ("ClientAttackActorComponent", ATTACK_PATH_OFF)] {
        if let Some((coff, _)) = comps.iter().find(|(_, n)| n.contains(needle)) {
            if let Some(comp) = safe::read_ptr(sub + coff) {
                if let Some(p) = safe::read_ptr(comp + off) {
                    if let Some(s) = safe::read_cstr(p, 256) {
                        if s.starts_with('/') {
                            return Some(s);
                        }
                    }
                }
            }
        }
    }
    None
}

/// Node name the way the reference mod reads it (`FUN_180011090`):
/// `gimmick component + 0x68 -> ptr -> ptr -> chars` (4..63 printable chars).
pub const GIMMICK_NAME_OFF: usize = 0x68;

pub fn node_name(m: &MainModule, actor: usize) -> Option<String> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let (off, _) = component_names(m, actor)
        .into_iter()
        .find(|(_, n)| n.contains("ClientGimmickActorComponent"))?;
    let comp = safe::read_ptr(sub + off)?;
    let p1 = safe::read_ptr(comp + GIMMICK_NAME_OFF)?;
    // try both depths; the mod uses two dereferences
    if let Some(p2) = safe::read_ptr(p1) {
        if let Some(s) = safe::read_cstr(p2, 64).filter(|s| s.len() >= 4) {
            return Some(s);
        }
    }
    safe::read_cstr(p1, 64).filter(|s| s.len() >= 4)
}

/// What a nearby actor is, by the reference mod's structural rules
/// (`FUN_180013f10`), names not required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Player,
    Character,
    /// A creature the game lets the player catch by hand: an insect, a fish,
    /// or something whose class is not yet known. See [`catch_class`] for the
    /// rule, its evidence, and which classes are actually taken - this kind
    /// only says "the game shapes it like a catchable creature", which is why
    /// the survey lists all of them.
    Catchable,
    /// A dead animal the player can skin: the same actor shape as
    /// [`Kind::Catchable`] with the dead flag set. See
    /// [`is_skinnable_carcass`] for the rule and the live readings behind it.
    /// Targeted only when `[Looter] GatherCarcass=1`, which is off by
    /// default; the survey lists one either way.
    Carcass,
    /// A dead animal with nothing left to give: the drops were rolled and
    /// every row has been granted. Never a candidate, whatever `GatherCarcass`
    /// says, and kept as its own kind rather than folded into
    /// [`Kind::Character`] so that `review` retires it and no corpse can ever
    /// reach `catch_class`. See [`carcass_has_drops`].
    CarcassEmpty,
    /// Gimmick with the +0xE0 interaction object whose record is a known
    /// gather record (Foraging/Logging/Mining/Ore/Money).
    Gather,
    /// Gimmick with an interaction object but some other record (gate, puzzle...).
    Interactable,
    /// An item on the ground: the +0xC0 instance object, or an `item_*`
    /// gimmick record (ore chunks are `item_basic_onehand` with neither object).
    Item,
    /// Gimmick with an instance object that references the player: equipment.
    Equipment,
    /// Gimmick with neither object but a known gather record
    /// (Foraging/Logging/Mining/Ore/Money): a node the game has not armed
    /// with an interaction object (ore droppings beyond the one in focus).
    /// The reference mod's AutoArm exists for these.
    ///
    /// The placed coin props land here too, and only here: surveyed on
    /// 2026-09-13 they carry neither an interaction object nor an instance
    /// object, so before `Family::Money` existed they fell through to
    /// [`Kind::Inert`] for want of a family rather than for any structural
    /// reason. `Config::gather_money` is what decides whether one is
    /// actually targeted.
    Unarmed,
    /// Gimmick with neither object and no gather record: the empty twin of a
    /// node, scenery, or not yet loaded.
    Inert,
    Other,
}

impl Kind {
    /// Is an actor of this kind something the gatherer will point an event
    /// at? This is the **one** definition of the candidate set, because two
    /// places need to agree about it and used to each carry their own copy:
    /// `game::nearest_gather` builds the candidate list from it, and
    /// `gatherer::Gatherer::review` asks it again to decide whether a target
    /// it already sent for has gone. A kind that is a candidate in the first
    /// and not the second is a target that is skinned or gathered and then
    /// never counted as done; the reverse is a target reviewed forever. They
    /// cannot drift while they both call this.
    ///
    /// `skin_carcasses` is `[Looter] GatherCarcass`, and it is an argument
    /// rather than a lookup because this module knows nothing about the ini
    /// and should not start to. With it `false` a [`Kind::Carcass`] is not a
    /// candidate **at all** - not a candidate that is then refused - which is
    /// what keeps the default-off switch from touching any later decision.
    ///
    /// Exhaustive on purpose, like `game::survey_rank`: a new [`Kind`] must
    /// say whether the gatherer aims at it.
    /// Every variant, so a test can assert a decision was recorded for each.
    /// An exhaustive `match` forces a new variant to get an *arm*; only this
    /// forces it to get an assertion, and an arm nobody asserted on is how a
    /// wrong default ships without anyone noticing.
    pub const ALL: [Kind; 12] = [
        Kind::Player,
        Kind::Character,
        Kind::Catchable,
        Kind::Carcass,
        Kind::CarcassEmpty,
        Kind::Gather,
        Kind::Interactable,
        Kind::Item,
        Kind::Equipment,
        Kind::Unarmed,
        Kind::Inert,
        Kind::Other,
    ];

    /// Whether the gatherer may aim an event at this kind. The one definition:
    /// `game::nearest_gather` picks targets with it and `gatherer::review` asks
    /// whether a target it already sent at is still one, and the two drifting
    /// apart is how a target gets sent at and then never retired.
    pub fn is_gather_candidate(self, skin_carcasses: bool) -> bool {
        match self {
            Kind::Gather | Kind::Unarmed | Kind::Item | Kind::Catchable => true,
            Kind::Carcass => skin_carcasses,
            // An emptied carcass is never a target. Firing at one grants
            // nothing, and counting the refusal is what used to switch
            // auto-gather off after three corpses.
            Kind::CarcassEmpty => false,
            Kind::Player
            | Kind::Character
            | Kind::Interactable
            | Kind::Equipment
            | Kind::Inert
            | Kind::Other => false,
        }
    }
}

/// Does the actor's sub-object reference the player actor (first 0x200 bytes)?
/// The reference mod uses this to recognise the player's own equipment.
pub fn refers_to_player(actor: usize, player: usize) -> bool {
    // The reference mod scans the actor object itself (first 0x200 bytes) for
    // the player actor pointer or the player's sub-object pointer.
    let player_sub = safe::read_ptr(player + 0x68);
    (0..0x200).step_by(8).any(|off| {
        let v = safe::read_ptr(actor + off);
        v == Some(player) || (player_sub.is_some() && v == player_sub)
    })
}

/// Entity id this actor is attached to (`transform+0xC8`), if any. The
/// reference mod calls this the owner: equipment is attached to the player.
pub fn attached_to(actor: usize) -> Option<u32> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let tf = safe::read_ptr(sub + 0x1A0)?;
    let v: u32 = safe::read(tf + 0xC8)?;
    (v != 0 && v != 0xFFFF_FFFF).then_some(v)
}

pub fn classify(m: &MainModule, actor: usize, player: usize) -> Kind {
    if actor == player {
        return Kind::Player;
    }
    if let Some(peid) = actor_eid(player) {
        if attached_to(actor) == Some(peid) {
            return Kind::Equipment;
        }
    }
    let comps = component_names(m, actor);
    let has = |needle: &str| comps.iter().any(|(_, n)| n.contains(needle));
    if has("ClientAiActorComponent") || has("ClientCharacterControlActorComponent") {
        // The dead test comes **first**, and the order is the whole point.
        // `catch_class` reads the status kind byte (+0x2C8) and ignores the
        // dead flag beside it, so a corpse satisfies every test it makes and
        // classifies `Catchable` - and `Kind::Catchable` is in the gatherer's
        // candidate set, so until this arm existed F9 beside a dead animal
        // sent a *catch* event at it. Answering `Carcass` here both fixes
        // that and supplies the skinning target, which is why the two land
        // together. The reference mod ordered its own rule the same way:
        // its corpse test (`docs/reference-internals.md` line 326, "dead flag
        // and (category 0x0C or has AI)") ran after the living-creature
        // rejection, never before it.
        //
        // Every dead animal answers here, skinnable or not. Letting an emptied
        // carcass fall through to `catch_class` was a real defect: an emptied
        // corpse still reads type 3/6 with status kind 0, so `catch_class`
        // answered `Some` and the corpse came back as `Catchable` - which is a
        // candidate whatever `GatherCarcass` says. That resurrected the very
        // bug this arm exists to fix (a *catch* event sent at a dead animal,
        // for any corpse whose category is a known class - `B01003D9` was
        // `type=03 cat=23`), and it also defeated the retire path: `review`
        // saw `still_gather` stay true, so the carcass was never retired, came
        // back every `NodeCooldown`, and three rounds of that switched
        // auto-gather off with a "bag full?" line about a bag that was fine.
        // Two kinds, decided here, keep both out of reach.
        //
        // The flag is read through the component list `classify` has already
        // walked. Going through `status_bytes` would re-walk it, and this arm
        // sits in front of `catch_class`, which walks it twice more - the
        // per-creature RTTI passes are the expensive part of a tick that
        // classifies every actor in range.
        //
        // The category byte is asked too, and only to rule people out. Type 3
        // carries NPCs as well as animals (`creature::CATCHABLE_TYPES`: "the
        // type byte alone never decides anything"), and a dead NPC has a
        // dead-drop component like any corpse, so the type gate on its own
        // would have the looter stripping bodies - which is enemy loot, and
        // outside what the README promises. `creature::NPC_CLASSES` is a
        // blacklist rather than a whitelist because the species this actually
        // works on read 0xD9/0x75/0x44/0xD7, classes no recorded catch has
        // ever shown, so a whitelist would refuse all of them.
        if creature::is_catchable_type(actor)
            && status_off(&comps)
                .and_then(|off| status_bytes_at(actor, off))
                .is_some_and(|st| st.flag == DEAD_FLAG)
            && !status_off(&comps)
                .and_then(|off| category_at(actor, off))
                .is_some_and(creature::is_npc_class)
        {
            return if carcass_has_drops(actor) { Kind::Carcass } else { Kind::CarcassEmpty };
        }
        return if catch_class(m, actor).is_some() { Kind::Catchable } else { Kind::Character };
    }
    if !has("ClientGimmickActorComponent") {
        return Kind::Other;
    }
    let gi = gimmick_info(m, actor);
    let has_inter = gi.as_ref().is_some_and(|g| g.record_id.is_some());
    let has_inst = gi.as_ref().is_some_and(|g| g.instance.is_some_and(|i| i != 0xFFFF_FFFF));
    if refers_to_player(actor, player) {
        return Kind::Equipment;
    }
    if has_inter {
        match node_identity(m, actor).and_then(|n| n.family) {
            Some(_) => Kind::Gather,
            None => Kind::Interactable,
        }
    } else if has_inst {
        Kind::Item
    } else {
        match node_identity(m, actor) {
            Some(n) if n.family.is_some() => Kind::Unarmed,
            Some(n) if n.name.as_deref().is_some_and(is_ground_item_record) => Kind::Item,
            _ => Kind::Inert,
        }
    }
}

/// Gimmick records that stand for a physical item lying on the ground.
pub fn is_ground_item_record(name: &str) -> bool {
    name.starts_with("item_")
}

/// The subset we are willing to pick up: generic drops such as ore chunks,
/// never quest or special items.
pub fn is_basic_item_record(name: &str) -> bool {
    name.starts_with("item_basic")
}

/// Dropped gear (`item_basic_equip_*`, e.g. a sword lying in the grass).
/// Outside the gathering scope; taken only when `GatherGear=1`.
pub fn is_gear_item_record(name: &str) -> bool {
    name.starts_with("item_basic_equip")
}

/// Compact view of the +0xE0 interaction object: words 0..5 as hex.
pub fn interaction_words(m: &MainModule, actor: usize) -> String {
    let Some(sub) = safe::read_ptr(actor + 0x68) else { return "-".into() };
    let Some((off, _)) = component_names(m, actor).into_iter().find(|(_, n)| n.contains("ClientGimmickActorComponent")) else { return "-".into() };
    let Some(comp) = safe::read_ptr(sub + off) else { return "-".into() };
    let Some(p) = safe::read_ptr(comp + 0xE0) else { return "-".into() };
    (0..6)
        .map(|i| safe::read::<u64>(p + i * 8).map(|v| format!("{v:X}")).unwrap_or_else(|| "?".into()))
        .collect::<Vec<_>>()
        .join(",")
}

/// The gimmick record index the game keeps on the component (`comp+0x48`,
/// u16; 0xFFFF = none). Found via the game's own accessor `FUN_140382240`,
/// which every ClientGimmickActorComponent method calls with `this+0x48`.
pub const GIMMICK_RECORD_INDEX_OFF: usize = 0x48;

pub fn gimmick_record_index(m: &MainModule, actor: usize) -> Option<u16> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let (off, _) = component_names(m, actor)
        .into_iter()
        .find(|(_, n)| n.contains("ClientGimmickActorComponent"))?;
    let comp = safe::read_ptr(sub + off)?;
    let idx: u16 = safe::read(comp + GIMMICK_RECORD_INDEX_OFF)?;
    (idx != 0xFFFF).then_some(idx)
}

/// What the node is, by its gimmick record.
#[derive(Debug, Clone)]
pub struct NodeIdentity {
    pub index: u16,
    pub key: Option<u32>,
    pub name: Option<String>,
    pub family: Option<crate::collect::Family>,
}

pub fn node_identity(m: &MainModule, actor: usize) -> Option<NodeIdentity> {
    let index = gimmick_record_index(m, actor)?;
    let rec = crate::tables::gimmick_record(m, index);
    let key = rec.and_then(crate::tables::gimmick_record_key);
    let name = rec.and_then(crate::tables::gimmick_record_name);
    let family = key
        .and_then(crate::collect::family_by_key)
        .or_else(|| name.as_deref().and_then(crate::collect::family_by_name));
    Some(NodeIdentity { index, key, name, family })
}

/// The player's inventory, the way the game's own condition evaluators reach
/// it (`FUN_142073710` then `FUN_1421B1940`, build 25116796):
/// `inv = *(actor->sub + 0xB8)`; `inv+0x18` is an array of tab pointers with
/// the count at `inv+0x20`; each tab has `i16 id @+0x10`, `i16 used @+0x12`,
/// `i16 max @+0x14`. The game computes free slots as `max - used` per tab.
pub const INVENTORY_OFF: usize = 0xB8;
pub const INVENTORY_TABS_OFF: usize = 0x18;
pub const INVENTORY_TAB_COUNT_OFF: usize = 0x20;
const MAX_INVENTORY_TABS: u32 = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryTab {
    pub ptr: usize,
    pub id: i16,
    pub used: i16,
    pub max: i16,
}

/// One occupied slot of a tab. Layout from the game's per-tab item counter
/// (`FUN_14234E1F0`): `tab+0x00` slot array, `tab+0x08` low i16 = slot
/// capacity, slots are 200 bytes: `i16 item record index @+0x08` (-1 = empty),
/// `i64 count @+0x10`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventorySlot {
    pub slot: u16,
    pub item_index: u16,
    pub count: i64,
}

pub const SLOT_STRIDE: usize = 200;
const MAX_SLOTS: i16 = 2000;

pub fn tab_slots(tab: &InventoryTab) -> Option<Vec<InventorySlot>> {
    let arr = safe::read_ptr(tab.ptr)?;
    let cap: i16 = safe::read(tab.ptr + 8)?;
    if cap <= 0 || cap > MAX_SLOTS {
        return None;
    }
    let mut out = Vec::new();
    for i in 0..cap as usize {
        let s = arr + i * SLOT_STRIDE;
        let idx: u16 = safe::read(s + 8)?;
        if idx == 0xFFFF {
            continue;
        }
        let count: i64 = safe::read(s + 0x10)?;
        if count <= 0 {
            continue;
        }
        out.push(InventorySlot { slot: i as u16, item_index: idx, count });
    }
    Some(out)
}

impl InventoryTab {
    pub fn free(&self) -> i32 {
        self.max as i32 - self.used as i32
    }
}

pub fn inventory_tabs(actor: usize) -> Option<Vec<InventoryTab>> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let inv = safe::read_ptr(sub + INVENTORY_OFF)?;
    let count: u32 = safe::read(inv + INVENTORY_TAB_COUNT_OFF)?;
    if count == 0 || count > MAX_INVENTORY_TABS {
        return None;
    }
    let arr = safe::read_ptr(inv + INVENTORY_TABS_OFF)?;
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let tab = safe::read_ptr(arr + i * 8)?;
        let id: i16 = safe::read(tab + 0x10)?;
        let used: i16 = safe::read(tab + 0x12)?;
        let max: i16 = safe::read(tab + 0x14)?;
        out.push(InventoryTab { ptr: tab, id, used, max });
    }
    Some(out)
}

/// The tab we treat as "the bag": an explicit id, or the one with the
/// largest capacity.
pub fn bag_tab(tabs: &[InventoryTab], wanted: Option<i16>) -> Option<InventoryTab> {
    match wanted {
        Some(id) => tabs.iter().copied().find(|t| t.id == id),
        None => tabs.iter().copied().filter(|t| t.max > 0).max_by_key(|t| t.max),
    }
}

/// Bytes the reference mod reads off `ClientStatusActorComponent` to reject
/// things that are not loot:
/// `+0x2C8`: 1 = quest item, 0x0F = shop goods, 0x11 = decoration;
/// `+0x273`: **the alive/dead flag, 0 alive and 1 dead** - see
/// [`is_skinnable_carcass`] for the live readings that settled it. The
/// reference mod read this byte as "6 = catchable creature", which it is not.
pub const STATUS_KIND_OFF: usize = 0x2C8;
pub const STATUS_FLAG_OFF: usize = 0x273;
/// The value [`STATUS_FLAG_OFF`] carries on a dead actor. 0 is alive.
pub const DEAD_FLAG: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusBytes {
    pub kind: u8,
    pub flag: u8,
}

pub fn status_bytes(m: &MainModule, actor: usize) -> Option<StatusBytes> {
    status_bytes_at(actor, status_off(&component_names(m, actor))?)
}

/// The `ClientStatusActorComponent`'s slot offset inside the sub-object, out
/// of a component list already walked. Split out so a caller holding one does
/// not pay for a second RTTI pass over the same actor.
pub fn status_off(comps: &[(usize, String)]) -> Option<usize> {
    comps.iter().find(|(_, n)| n.contains("ClientStatusActorComponent")).map(|(off, _)| *off)
}

/// [`interaction_category`] once the component's slot is known, for a caller
/// that already holds the component list and should not pay for a second RTTI
/// pass to read one more byte out of the same object.
pub fn category_at(actor: usize, off: usize) -> Option<u8> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let comp = safe::read_ptr(sub + off)?;
    safe::read(comp + STATUS_CATEGORY_OFF)
}

/// [`status_bytes`] once the component's slot is known.
pub fn status_bytes_at(actor: usize, off: usize) -> Option<StatusBytes> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let comp = safe::read_ptr(sub + off)?;
    Some(StatusBytes { kind: safe::read(comp + STATUS_KIND_OFF)?, flag: safe::read(comp + STATUS_FLAG_OFF)? })
}

/// Shop goods, quest items and decoration are never picked up.
pub fn is_owned_or_special(st: StatusBytes) -> bool {
    matches!(st.kind, 1 | 0x0F | 0x11)
}

/// What this plugin makes of a would-be [`Kind::Character`] that the build
/// says is a creature the player can catch by hand.
///
/// `Unknown(c)` is a creature that passes the type-and-status gate but whose
/// [`interaction_category`] byte has never been seen on a creature we have
/// actually caught (`desert_core::creature` holds the two lists and the
/// evidence for them). It is **never targeted**, and it is a variant rather
/// than a `None` because the looter has something to say about it: the class
/// is reported once per value per session so the lists can be grown from a
/// real log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Catchable {
    Known(CatchClass),
    Unknown(u8),
}

impl Catchable {
    /// The looter's view of an [`interaction_category`] byte: the shared
    /// class when it is one a recorded catch has shown, `Unknown` otherwise.
    /// Nothing here may invent a class - see `desert_core::creature`.
    fn of_category(cat: u8) -> Self {
        match creature::catch_class(cat) {
            Some(c) => Catchable::Known(c),
            None => Catchable::Unknown(cat),
        }
    }
}

/// Classify a catchable creature by its [`interaction_category`] byte.
///
/// `None` means the actor is not a catch candidate at all: its type byte is
/// not one of `creature::CATCHABLE_TYPES`, its `ClientStatusActorComponent`
/// kind byte (+0x2C8) is not 0, or the category byte is not readable. That
/// trio is what
/// [`Kind::Catchable`] means, so the survey still lists every actor this
/// returns `Some` for, whichever class comes back.
///
/// The class is what decides whether the plugin takes it, and it comes from
/// `desert_core::creature::catch_class` so that Desert Gatherer's catch-count
/// hook cannot disagree about what a fish is. The same surveys that produced
/// those lists also showed, all reading `type=06 status=00/00` and so all
/// `Kind::Catchable`:
///
/// - `cat=20` actors 9-25 m *above* the player (y 543-559 against the
///   player's 535): birds in flight, never caught by hand.
/// - `cat=2C`, `cat=44`, `cat=65` at ground and water level: unknown species,
///   never caught by hand.
///
/// The type byte alone is therefore not enough, and the category byte alone
/// is not either (type-05 characters at 37 m read `cat=80`, `8C` and `90`):
/// both gates are needed. Nor does the type byte say what a creature *is*:
/// type 3 carries the NPCs that read `cat=33`, `66`, `71` and `21` **and**
/// the Firefly Colony, which reads the insect class `cat=80` and is caught by
/// hand with the same event as every other insect, which is why `creature`
/// holds a list of types rather than a single value. See
/// `docs/reference-internals.md` section 15.5.
pub fn catch_class(m: &MainModule, actor: usize) -> Option<Catchable> {
    if !creature::is_catchable_type(actor) {
        return None;
    }
    if !status_bytes(m, actor).is_some_and(|s| s.kind == 0) {
        return None;
    }
    let cat = interaction_category(m, actor)?;
    Some(Catchable::of_category(cat))
}

/// Is this actor a carcass the player could skin - a dead animal?
///
/// Two bytes, both of which the mod already reads: the actor type byte is one
/// of `creature::CATCHABLE_TYPES` (3 or 6), and the
/// `ClientStatusActorComponent` flag byte at [`STATUS_FLAG_OFF`] (`+0x273`)
/// is 1. **That flag is the alive/dead bit: 0 alive, 1 dead.**
///
/// Established live on build 25246367 (`docs/findings-skinning-2026-09-15.md`
/// section 7), from three F11 surveys at 224 s / 266 s / 288 s with eight
/// carcasses skinned by hand between 234 s and 283 s. Actor `B01003D9` is the
/// one surveyed on both sides of its own death: it reads `status=00/00` at
/// 224 s and again at 266 s while alive, and `00/01` at 288 s, after being
/// killed and skinned at 283 s. Across every actor surveyed that session all
/// eight skinned carcasses read `flag=01`, and of the 138 actors reading
/// `flag=00` exactly one was ever a carcass - that same one, before it died.
/// `flag=01` appeared only on actors this module classifies `Catchable`,
/// never on `Inert`, `Equipment` or `Character`. The carcass type bytes were
/// `06` (seven of them) and `03` (one), both already in `CATCHABLE_TYPES`.
///
/// The game corroborates it statically: its own carcass gate
/// (`FUN_142a75360`) branches on `status+0x273` before it ever looks at the
/// owner id.
///
/// The type gate is what keeps this to **animals**. Types 4 and 5 - human and
/// NPC corpses - are deliberately outside it: enemy loot is not in this mod's
/// stated scope, and a dead-drop event is as happy to strip a body as a deer.
///
/// An unreadable actor answers `false`, the same way `is_catchable_type` and
/// `catch_class` do: when the mod cannot tell, it leaves the thing alone.
/// Nothing new is read here either - `+0x273` sits inside the range
/// `events::creature_preflight` already proves readable before a send - so
/// this retires the health words at `+0x2CC`/`+0x2D0`/`+0x2D4` and the
/// handler's own `*(*(actor+0x68)+0xC8)` corpse test as things anyone has to
/// decode.
///
/// It also retires a stale note. `docs/reference-internals.md` line 320
/// records the reference mod's catch rule as `status+0x273 == 6` and calls it
/// stale because "+0x273 reads 0 on a confirmed insect". It read 0 because
/// the insect was **alive**: the byte is not a class marker at all.
pub fn is_skinnable_carcass(m: &MainModule, actor: usize) -> bool {
    is_dead_animal(m, actor) && carcass_has_drops(actor)
}

/// A dead animal: the type gate and the dead flag, and nothing about whether
/// anything is left on it. `classify` needs the two halves separately, because
/// an emptied carcass must still be answered as a corpse rather than fall
/// through to `catch_class` - see the comment at that call site for what went
/// wrong when one did. `classify` itself does not call this: it holds the
/// component list already and reads the flag through [`status_bytes_at`] to
/// avoid a second RTTI pass.
pub fn is_dead_animal(m: &MainModule, actor: usize) -> bool {
    creature::is_catchable_type(actor)
        && status_bytes(m, actor).is_some_and(|st| st.flag == DEAD_FLAG)
        && !interaction_category(m, actor).is_some_and(creature::is_npc_class)
}

/// The dead-drop component, `*(*(actor+0x68)+0xC8)`. Non-null is the game's
/// own test for "this corpse can be looted": the dead-drop handler
/// (`FUN_142b62d90`) reads exactly this and does nothing at all when it is
/// null.
pub const DEAD_DROP_COMP_OFF: usize = 0xC8;
/// `comp+0x6C`: a byte latched to 1 the first time the drops are rolled
/// (`FUN_142ab7dd0`, `if (*(char *)(comp + 0x6c) == '\0') { ... = 1; }`).
pub const DROPS_ROLLED_OFF: usize = 0x6C;
/// `comp+0x50`: the live count of the **item** rows at `comp+0x48`, 0x18 bytes
/// each, `{u32 flags, u16 item index @+8, i64 count @+0x10}`. These are the
/// leather/meat/bone rows: `FUN_141e93920` appends them while walking
/// `characterinfo rec+0x2B8`, `FUN_142ab7dd0` stores the list here through
/// `FUN_14043b830`, and the grant (`FUN_142ab6f20`) builds its item stacks
/// from them - `FUN_1423507b0(obj, row+8, row+0x10)` - then subtracts through
/// `FUN_1420903e0`, which deletes a row once its count drops below 1 and
/// decrements this word.
///
/// This is the list that matters, and reading [`DROP_ROWS_OFF`] *instead* of
/// it was a defect: a rolled-but-ungranted carcass reads `item_rows > 0` with
/// `set_rows == 0`, so testing only the other list called it empty.
pub const ITEM_ROWS_OFF: usize = 0x50;
/// `comp+0x60`: the live count of the 0x40-byte **drop-set** rows at
/// `comp+0x58`, which `FUN_141e93700` rolls out of `characterinfo rec+0x280`
/// and stamps with each entry's loot-method mask. `FUN_142ab6f20` erases only
/// the rows whose mask matches the method in play (`FUN_1420905e0`), so rows
/// for other methods can linger here on an otherwise-empty carcass - a known
/// caveat of using this count, and the reason the item list above is checked
/// too rather than instead.
///
/// It is also where a failed inventory add puts refused stacks **back**
/// (`FUN_142090560`), which is what makes a carcass skinned into a full bag
/// stay a target.
pub const DROP_ROWS_OFF: usize = 0x60;
/// `comp+0x58`: the base of those 0x40-byte drop-set rows.
pub const DROP_ROWS_PTR_OFF: usize = 0x58;
/// Stride of a drop-set row.
pub const DROP_ROW_STRIDE: usize = 0x40;
/// The loot-method mask skinning is granted under.
///
/// `FUN_142ab5270` builds the mask it hands the grant as
/// `1 << (*param_5 & 0x1f)`, `param_5` being the loot method, and the dead-drop
/// chain passes method 0 - so the mask is exactly 1. `FUN_142ab6f20` then walks
/// the drop-set rows and, for each, `if ((*row & *mask) == 0)` **skips it and
/// leaves it in place**; only a matching row is granted and erased. A row whose
/// mask lacks this bit is therefore loot for some other method and can never be
/// taken by skinning, however many times the event is fired.
///
/// A refused stack that the game puts back carries mask word 1
/// (`FUN_141e93920` writes it for item-derived stacks), so it matches and is
/// correctly still counted as loot waiting.
pub const SKIN_METHOD_MASK: u32 = 1;
/// How many drop-set rows are worth walking. The count comes out of game
/// memory, so it is bounded rather than trusted; no carcass seen in game has
/// carried more than four.
const DROP_ROWS_MAX: u32 = 64;

/// Whether a carcass still has anything to give.
///
/// This is what keeps a skinned carcass from being skinned again forever. A
/// carcass **stays in the world after it is looted**, with its type byte and
/// its dead flag unchanged - `B01003D9` was skinned at 283 s and still
/// surveyed `status=00/01` at 288 s - so the dead flag alone cannot tell an
/// untouched corpse from an emptied one. Without this test the gatherer's
/// `review` would hand the same carcass back every `NodeCooldown`, the game
/// would grant nothing each time (the handler is idempotent), and three
/// rounds of that would trip `MAX_CONSECUTIVE_FAILURES` and switch auto-gather
/// off with a "bag full?" line that had nothing to do with the bag.
///
/// Emptied means **rolled and now empty on both lists**: the latch at
/// [`DROPS_ROLLED_OFF`] is set and *neither* [`ITEM_ROWS_OFF`] nor
/// [`DROP_ROWS_OFF`] still counts a row. All three words matter. An untouched
/// carcass has not rolled yet, so both counts are legitimately zero and only
/// the latch separates it from an empty one. The item rows are what the grant
/// turns into inventory stacks, so they are the ones that say there is loot
/// left. And when the inventory add fails part-way the game puts the refused
/// stacks **back** - onto the drop-set list, not the item list - so a carcass
/// skinned into a full bag keeps a non-zero count there and stays a target,
/// which is exactly right.
///
/// One known hole, unobserved across the 14 carcasses skinned in game on
/// 2026-09-15 (classes 0xD9, 0x75, 0x44, 0xD7, every one of which went to
/// `1/0/0`): the grant erases only the drop-set rows whose mask matches the
/// method in play, so a species carrying rows for some other method would keep
/// a non-zero [`DROP_ROWS_OFF`] forever and stay a `Carcass` after being
/// emptied. Closing it means testing each row's mask rather than the count,
/// which needs the mask bit confirmed against the exe first.
pub fn carcass_has_drops(actor: usize) -> bool {
    match dead_drop_state(actor) {
        // Not rolled yet: the counts are legitimately zero and the latch is
        // the only thing that separates an untouched carcass from an empty one.
        Some(dd) if dd.rolled == 0 => true,
        // Rolled: item rows are always ours, but a drop-set row only counts if
        // its mask says skinning can take it. Counting every drop-set row was a
        // real bug, seen in game: class 0x82 sits at `1/0/4` and 0xD7 at
        // `1/0/3` after being fully skinned, so both stayed `Carcass`, came back
        // every `NodeCooldown`, and three in a row switched auto-gather off with
        // a "bag full?" line about a bag that was fine.
        Some(dd) => dd.item_rows > 0 || dd.grantable_set_rows > 0,
        // No component, or it would not read: the game itself would do nothing
        // with this corpse, so neither do we.
        None => false,
    }
}

/// The dead-drop component's three interesting words.
#[derive(Debug, Clone, Copy)]
pub struct DeadDrop {
    /// `comp+0x6C`, latched to 1 the first time the drops are rolled.
    pub rolled: u8,
    /// `comp+0x50`, the live count of 0x18-byte **item** rows at `comp+0x48`.
    pub item_rows: u32,
    /// `comp+0x60`, the live count of 0x40-byte **drop-set** rows at `comp+0x58`.
    pub set_rows: u32,
    /// How many of those rows skinning could actually take: their mask word
    /// intersects [`SKIN_METHOD_MASK`]. This is the number that decides whether
    /// a carcass still has loot, not `set_rows` - see [`carcass_has_drops`].
    pub grantable_set_rows: u32,
}

/// Read the dead-drop component's state, or `None` when there is no readable
/// component. Printed by the survey for every dead animal, which is what makes
/// the two counts diagnosable from a log instead of guessable.
pub fn dead_drop_state(actor: usize) -> Option<DeadDrop> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let comp = safe::read_ptr(sub + DEAD_DROP_COMP_OFF)?;
    let set_rows: u32 = safe::read(comp + DROP_ROWS_OFF)?;
    let mut grantable_set_rows = 0;
    if let Some(rows) = safe::read_ptr(comp + DROP_ROWS_PTR_OFF) {
        for i in 0..set_rows.min(DROP_ROWS_MAX) {
            let mask: u32 = safe::read(rows + i as usize * DROP_ROW_STRIDE).unwrap_or(0);
            if mask & SKIN_METHOD_MASK != 0 {
                grantable_set_rows += 1;
            }
        }
    }
    Some(DeadDrop {
        rolled: safe::read(comp + DROPS_ROLLED_OFF)?,
        item_rows: safe::read(comp + ITEM_ROWS_OFF)?,
        set_rows,
        grantable_set_rows,
    })
}


/// The interaction category byte the game's own interaction handler switches
/// on (`FUN_1429DB730`, build 25116796: `*(status + 0x5A)`), read off
/// `ClientStatusActorComponent`. Diagnostic only for now; it is printed in
/// the survey so the catch rule above can be sharpened from real logs.
pub const STATUS_CATEGORY_OFF: usize = 0x5A;

pub fn interaction_category(m: &MainModule, actor: usize) -> Option<u8> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let (off, _) = component_names(m, actor)
        .into_iter()
        .find(|(_, n)| n.contains("ClientStatusActorComponent"))?;
    let comp = safe::read_ptr(sub + off)?;
    safe::read(comp + STATUS_CATEGORY_OFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the *shared* class bytes are, and that no other byte maps to one,
    /// is asserted in `desert_core::creature` next to the lists themselves.
    /// What this file owes is the other half of the rule: a category byte the
    /// core does not recognise must come back as `Unknown`, never as a class,
    /// because `game::nearest_gather` targets `Known` and only reports
    /// `Unknown`. A creature we have never watched being caught is not taken.
    #[test]
    fn an_unrecognised_category_is_never_a_class() {
        // 0x20 is the birds-in-flight class of section 15.5; the rest are the
        // unidentified species the same surveys turned up, plus the ends.
        for cat in [0x00, 0x20, 0x2C, 0x44, 0x65, 0xFF] {
            assert_eq!(Catchable::of_category(cat), Catchable::Unknown(cat), "0x{cat:02X}");
        }
    }

    /// The candidate set is the one thing `game::nearest_gather` and
    /// `gatherer::Gatherer::review` must agree on, and it is pure logic, so
    /// it is the part of the skinning change that can honestly be tested
    /// without the game. What matters is the two edges: a `Carcass` is a
    /// candidate exactly when `GatherCarcass` is on, and nothing else in the
    /// enum changes answer when that switch moves.
    #[test]
    fn a_carcass_is_a_candidate_only_while_gathercarcass_is_on() {
        assert!(Kind::Carcass.is_gather_candidate(true));
        assert!(!Kind::Carcass.is_gather_candidate(false));
        for k in [Kind::Gather, Kind::Unarmed, Kind::Item, Kind::Catchable] {
            assert!(k.is_gather_candidate(false), "{k:?} is a candidate regardless of the switch");
            assert!(k.is_gather_candidate(true), "{k:?} is a candidate regardless of the switch");
        }
        // Scenery, the player, live characters, his own equipment and the
        // interactables are never aimed at, and turning skinning on must not
        // sweep any of them in.
        // An emptied carcass belongs in this list and not beside `Carcass`:
        // firing at one grants nothing, and it was counting those refusals
        // that switched auto-gather off after three corpses. `GatherCarcass=1`
        // must not bring it back.
        for k in [
            Kind::Inert,
            Kind::Player,
            Kind::Character,
            Kind::Equipment,
            Kind::Interactable,
            Kind::Other,
            Kind::CarcassEmpty,
        ] {
            assert!(!k.is_gather_candidate(true), "{k:?} must never be a candidate");
            assert!(!k.is_gather_candidate(false), "{k:?} must never be a candidate");
        }
    }

    /// Every `Kind` has to appear in the test above, or a variant added later
    /// gets no decision recorded for it. `is_gather_candidate` is exhaustive so
    /// the compiler forces the *match* arm; nothing forces the assertion, and
    /// the arm without the assertion is how a wrong default ships quietly.
    #[test]
    fn every_kind_is_covered_by_the_candidate_test() {
        let covered = [
            Kind::Gather,
            Kind::Unarmed,
            Kind::Item,
            Kind::Catchable,
            Kind::Carcass,
            Kind::CarcassEmpty,
            Kind::Inert,
            Kind::Player,
            Kind::Character,
            Kind::Equipment,
            Kind::Interactable,
            Kind::Other,
        ];
        assert_eq!(covered.len(), Kind::ALL.len());
        for k in Kind::ALL {
            assert!(covered.contains(&k), "{k:?} is not named in the candidate test");
        }
    }

    #[test]
    fn the_recorded_categories_carry_the_core_class_through() {
        assert_eq!(Catchable::of_category(0x80), Catchable::Known(CatchClass::Bug));
        assert_eq!(Catchable::of_category(0x23), Catchable::Known(CatchClass::Fish));
        assert_eq!(Catchable::of_category(0x83), Catchable::Known(CatchClass::Fish));
    }

    /// [`eid_map`] used to also reject any map whose `capacity` exceeded
    /// 0x10000, and `capacity` is the key hash's bucket count, which grows with
    /// session uptime: on build 25246367, ~90 minutes into a session, it read
    /// **78942** with `count` at 2394, so every call returned `None` and the
    /// whole looter went quiet - `game::scene` is gated on this one function,
    /// and the AutoGather tick swallows its error. 78942 is that live reading.
    ///
    /// This whole module is `#[cfg(windows)]`, so the test runs under
    /// `just test-win` and is not compiled natively at all. That is what makes
    /// it work: `safe::read` reads *our own* process through
    /// `ReadProcessMemory`, so a map built in local buffers and handed over by
    /// address is a genuine map as far as `eid_map` can tell.
    #[test]
    fn a_grown_bucket_count_does_not_reject_the_eid_map() {
        /// The bucket count observed live when the old check rejected the map.
        const CAPACITY: u32 = 78942;
        const EID: u32 = 0xB000_0123;

        // Every buffer below stays in scope to the end of the test: the map is
        // nothing but addresses, so one dropped early would leave the
        // assertions reading freed memory and the test a coin flip. These are
        // stack arrays, and a Win64 user-mode stack sits far above the 0x10000
        // floor `safe::read_into` enforces.
        let actor = [0u8; 0x100];
        let actor_addr = actor.as_ptr() as usize;

        let mut slot = [0u8; SLOT_ACTOR_OFF + 8];
        slot[SLOT_EID_OFF..SLOT_EID_OFF + 4].copy_from_slice(&EID.to_le_bytes());
        slot[SLOT_ACTOR_OFF..SLOT_ACTOR_OFF + 8].copy_from_slice(&actor_addr.to_le_bytes());

        let values = [slot.as_ptr() as usize];
        // The key array is a hash chain nothing enumerates; it only has to be a
        // non-null pointer for `read_ptr` to hand it back.
        let keys = [0usize; 1];

        let mut container = [0u8; EIDMAP_OFF + 0x20];
        let b = EIDMAP_OFF;
        container[b + 0x8..b + 0xC].copy_from_slice(&1u32.to_le_bytes());
        container[b + 0xC..b + 0x10].copy_from_slice(&CAPACITY.to_le_bytes());
        container[b + 0x10..b + 0x18].copy_from_slice(&(keys.as_ptr() as usize).to_le_bytes());
        container[b + 0x18..b + 0x20].copy_from_slice(&(values.as_ptr() as usize).to_le_bytes());

        let map = eid_map(container.as_ptr() as usize).expect("a bucket count of 78942 is fine");
        assert_eq!(map.count, 1);
        assert_eq!(map.capacity, CAPACITY);
        assert_eq!(map.entries(), vec![(EID, actor_addr)]);
    }
}
