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
    // `capacity` is not an upper bound on `count` (207 entries with 167 was
    // seen live; it is probably the bucket count of the key hash), so only
    // the count and the value array's readability are checked.
    if count == 0 || count > 0x10000 || capacity > 0x10000 {
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
    /// Gimmick with the +0xE0 interaction object whose record is a known
    /// gather record (Foraging/Logging/Mining/Ore).
    Gather,
    /// Gimmick with an interaction object but some other record (gate, puzzle...).
    Interactable,
    /// An item on the ground: the +0xC0 instance object, or an `item_*`
    /// gimmick record (ore chunks are `item_basic_onehand` with neither object).
    Item,
    /// Gimmick with an instance object that references the player: equipment.
    Equipment,
    /// Gimmick with neither object but a known gather record: a node the game
    /// has not armed with an interaction object (ore droppings beyond the one
    /// in focus). The reference mod's AutoArm exists for these.
    Unarmed,
    /// Gimmick with neither object and no gather record: the empty twin of a
    /// node, scenery, or not yet loaded.
    Inert,
    Other,
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
/// `+0x273`: 6 = catchable creature.
pub const STATUS_KIND_OFF: usize = 0x2C8;
pub const STATUS_FLAG_OFF: usize = 0x273;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusBytes {
    pub kind: u8,
    pub flag: u8,
}

pub fn status_bytes(m: &MainModule, actor: usize) -> Option<StatusBytes> {
    let sub = safe::read_ptr(actor + 0x68)?;
    let (off, _) = component_names(m, actor)
        .into_iter()
        .find(|(_, n)| n.contains("ClientStatusActorComponent"))?;
    let comp = safe::read_ptr(sub + off)?;
    Some(StatusBytes { kind: safe::read(comp + STATUS_KIND_OFF)?, flag: safe::read(comp + STATUS_FLAG_OFF)? })
}

/// Shop goods, quest items and decoration are never picked up.
pub fn is_owned_or_special(st: StatusBytes) -> bool {
    matches!(st.kind, 1 | 0x0F | 0x11)
}

/// The actor's type byte: `*(actor+0x88) -> byte @1`.
///
/// The reference mod reads it as its per-actor "flag byte +0x2E"
/// (`FUN_18000e560`, `docs/reference-internals.md` section 10.1) and treats
/// 6 as "catchable"; its catch rule (section 10, rule 2) pairs that with a
/// category byte in {5, 9} and `ClientStatusActorComponent+0x273 == 6`.
/// The game's own steal check (`FUN_14251BA50`, section 14) switches on the
/// same byte: 4/5/6 are characters, 7 is a gimmick.
pub const TYPE_OBJECT_OFF: usize = 0x88;
pub const TYPE_BYTE_OFF: usize = 1;

pub fn type_byte(actor: usize) -> Option<u8> {
    let p = safe::read_ptr(actor + TYPE_OBJECT_OFF)?;
    safe::read(p + TYPE_BYTE_OFF)
}

/// Type byte of a creature the player can catch by hand (an insect).
///
/// Recorded live on build 25116796 (2026-09-08) by surveying the world and
/// then catching the very actors the survey had just listed, with the F7
/// event recorder running. All three insects read the same way:
///
/// ```text
/// Character eid=B01002C3 ClientNormalInGameActor type=06 status=00/00
///   comps=[Status EquipSlot CharacterControl Vehicle Detect Ai Effect
///          FrameEvent Catch RemoteCatch Attack]
/// ```
///
/// and each catch queued one `TrocTrPushCharacterToInventoryOnceTimer`
/// naming that eid. Two other type-06 characters standing further off read
/// `status` kind 0x20 and 0x1A; NPCs and horses read type 05 or 03.
pub const CATCHABLE_TYPE: u8 = 6;

/// Would-be [`Kind::Character`] that this build says is a creature the player
/// can catch by hand, and which kind of creature it is.
///
/// `Unknown(c)` is a creature that passes the type-and-status gate but whose
/// [`interaction_category`] byte has never been seen on a creature we have
/// actually caught. It is **never targeted**: the two lists below are exactly
/// the categories observed on successful hand catches so far, nothing more,
/// and a class that is not on them is reported once per session and skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchClass {
    Bug,
    Fish,
    Unknown(u8),
}

/// [`interaction_category`] values seen on insects that were caught by hand.
///
/// Six catches in the first verified field session (build 25116796,
/// 2026-09-08) all read 0x80, across four item ids (1001254, 1000680,
/// 1001238, 1001245). This is a list of what has been seen caught, not a
/// range the game defines: an insect reading anything else would classify as
/// [`CatchClass::Unknown`] and be skipped until a catch is recorded for it.
pub const BUG_CATEGORIES: &[u8] = &[0x80];

/// [`interaction_category`] values seen on fish that were caught by hand.
///
/// Fish are taken with the **same** event as insects
/// (`TrocTrPushCharacterToInventoryOnceTimer`, 8-byte payload). Four manual
/// catches at a lake on build 25116796 (2026-09-08), each surveyed with F11
/// immediately before and recorded with F7, read `type=06 status=00/00` and:
///
/// ```text
/// eid=B0100550  cat=83   -> [recv] item 29817 x1
/// eid=B01005D8  cat=23   -> [recv] item 29805 x1
/// eid=B01005AB  cat=23   -> [recv] item 29804 x1
/// eid=B010068C  cat=83   -> [recv] item 29817 x1
/// ```
///
/// As with [`BUG_CATEGORIES`], this is the set observed caught, not a set the
/// game declares.
pub const FISH_CATEGORIES: &[u8] = &[0x23, 0x83];

/// Classify a catchable creature by its [`interaction_category`] byte.
///
/// `None` means the actor is not a catch candidate at all: its [`type_byte`]
/// is not [`CATCHABLE_TYPE`], its `ClientStatusActorComponent` kind byte
/// (+0x2C8) is not 0, or the category byte is not readable. That trio is what
/// [`Kind::Catchable`] means, so the survey still lists every actor this
/// returns `Some` for, whichever class comes back.
///
/// The class is what decides whether the plugin takes it. The same surveys
/// that produced the tables above also showed, all reading
/// `type=06 status=00/00` and so all `Kind::Catchable`:
///
/// - `cat=20` actors 9-25 m *above* the player (y 543-559 against the
///   player's 535): birds in flight, never caught by hand.
/// - `cat=2C`, `cat=44`, `cat=65` at ground and water level: unknown species,
///   never caught by hand.
///
/// The type byte alone is therefore not enough, and the category byte alone
/// is not either (type-05 characters at 37 m read `cat=80`, `8C` and `90`,
/// and type-03 NPCs read `cat=33`, `66`, `71`, `21`): both gates are needed.
/// See `docs/reference-internals.md` section 15.5.
pub fn catch_class(m: &MainModule, actor: usize) -> Option<CatchClass> {
    if type_byte(actor) != Some(CATCHABLE_TYPE) {
        return None;
    }
    if !status_bytes(m, actor).is_some_and(|s| s.kind == 0) {
        return None;
    }
    let cat = interaction_category(m, actor)?;
    Some(if BUG_CATEGORIES.contains(&cat) {
        CatchClass::Bug
    } else if FISH_CATEGORIES.contains(&cat) {
        CatchClass::Fish
    } else {
        CatchClass::Unknown(cat)
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
