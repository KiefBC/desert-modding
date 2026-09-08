//! Resolution of game-side anchors (byte signatures, RTTI vtables) and the
//! read-only survey. Nothing here touches game state.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::actors::{self, CatchClass, Vec3};
use crate::config::Config;
use crate::module::MainModule;
use crate::pattern::{Found, Pattern};
use crate::rtti;

/// Signatures lifted from the reference mod. Names are ours; the comment says
/// what the hit is.
pub const SIGNATURES: &[(&str, &str)] = &[
    ("alloc_event", "48 89 5C 24 ?? 4C 89 44 24 ?? 57 48 83 EC 20 8B ?? BA ?? ?? 00 00"),
    ("enqueue", "48 89 5C 24 08 57 48 83 EC 20 48 8B ?? 38 65 48 8B 04 25 58 00 00 00"),
    ("area_sweep+0xF", "55 41 54 41 55 41 56 41 57 48 8B EC 48 83 EC 50 C5 F8 29 74 24 40 4C 8B ?? 48 8B ?? 48 8B ?? 44 8B"),
    ("own_check_site", "48 8B 89 20 01 00 00 E8 ?? ?? ?? ?? 84 C0 74 04 B3 02"),
    ("get_pos", "40 53 48 83 EC 50 48 8B 41 68 48 8B 88 ?? 01 00 00 48 8B 01"),
    ("desc_mask+queue_site", "E8 ?? ?? ?? ?? 44 8B 05 ?? ?? ?? ?? 0F B7 54 24 ?? E8 ?? ?? ?? ?? 4C 8B 25"),
    ("interaction_fn", "88 54 24 10 48 89 4C 24 08 53 55 56 57 41 54 41 55 41 56 41 57 48 83 EC 58 49 8B ?? 44 0F B6 ?? 4C 8B ??"),
    ("category_fn", "48 89 5C 24 18 88 54 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8B EC 48 81 EC ?? ?? ?? ?? 41 8B D9 4D 8B F0 0F B6 F2 4C 8B F9"),
];

pub const ACTOR_MANAGER_RTTI: &str = ".?AVClientActorManager@pa@@";

#[derive(Debug, Default)]
pub struct Anchors {
    /// name -> absolute VA of the signature hit (not adjusted for +0xF etc.).
    pub hits: Vec<(&'static str, usize)>,
    pub missing: Vec<&'static str>,
    pub actor_manager_vtables: Vec<u64>,
}

impl Anchors {
    pub fn get(&self, name: &str) -> Option<usize> {
        self.hits.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
    }
}

/// Scan the main module for everything we know how to find, logging as we go.
pub fn resolve(m: &MainModule) -> Anchors {
    let img = m.bytes();
    let mut a = Anchors::default();
    for (name, text) in SIGNATURES {
        // The signatures above are built in and all parse; a typo in one is a
        // skipped anchor, never a panic in the game process.
        let Some(pat) = Pattern::parse(text) else {
            crate::log!("[sig] {name:<22} UNPARSEABLE");
            a.missing.push(name);
            continue;
        };
        match pat.find_unique(img) {
            Found::Unique(off) => {
                crate::log!("[sig] {name:<22} = +0x{off:X}");
                a.hits.push((name, m.base + off));
            }
            Found::None => {
                crate::log!("[sig] {name:<22} NOT FOUND");
                a.missing.push(name);
            }
            Found::Ambiguous(n) => {
                crate::log!("[sig] {name:<22} AMBIGUOUS ({n}+ hits)");
                a.missing.push(name);
            }
        }
    }
    a.actor_manager_vtables = rtti::vtables_for_class(img, m.base as u64, ACTOR_MANAGER_RTTI);
    match a.actor_manager_vtables.as_slice() {
        [] => crate::log!("[rtti] {ACTOR_MANAGER_RTTI}: no vtable found"),
        v => {
            for vt in v {
                crate::log!("[rtti] {ACTOR_MANAGER_RTTI} vtable = +0x{:X}", vt - m.base as u64);
            }
        }
    }
    a
}

pub struct World {
    pub manager_slot: usize,
    pub manager: usize,
}

/// Find the actor manager through its global pointer. Retried by the caller.
pub fn find_world(m: &MainModule, a: &Anchors) -> Option<World> {
    let (slot, obj) = actors::find_manager(m, &a.actor_manager_vtables)?;
    crate::log!("[world] actor manager at 0x{obj:X} via global +0x{:X}", m.rva(slot));
    Some(World { manager_slot: slot, manager: obj })
}

fn class_of(m: &MainModule, obj: usize) -> String {
    actors::rtti_name(m, obj).map(|n| actors::short_name(&n)).unwrap_or_else(|| "?".into())
}

/// Inert actors closer than this get a detail line in the survey.
pub const INERT_DETAIL_RANGE: f32 = 10.0;

/// One survey line for an actor with no gimmick record: the class, the type
/// byte, the two `ClientStatusActorComponent` bytes, the interaction category
/// (`status+0x5A`, what `FUN_1429DB730` switches on) and the components. This
/// is the evidence the catch rule is refined from.
fn catch_diagnostic_line(
    m: &MainModule,
    a: usize,
    d: f32,
    eid: u32,
    pos: &Vec3,
    kind: actors::Kind,
) {
    let ty = match actors::type_byte(a) {
        Some(t) => format!("{t:02X}"),
        None => "?".to_string(),
    };
    let st = match actors::status_bytes(m, a) {
        Some(s) => format!("{:02X}/{:02X}", s.kind, s.flag),
        None => "?".to_string(),
    };
    let cat = category_text(m, a);
    let comps: Vec<String> = actors::component_names(m, a)
        .into_iter()
        .map(|(_, n)| actors::component_label(&actors::short_name(&n)))
        .collect();
    crate::log!(
        "  {d:6.1} m  {kind:<9?} eid={eid:08X} ({:7.1} {:7.1} {:7.1}) {} type={ty} cat={cat} status={st} comps=[{}]",
        pos.x, pos.y, pos.z, class_of(m, a), comps.join(" ")
    );
}

/// `actors::interaction_category` as two hex digits, or `?` when the status
/// component is not there or not readable.
pub fn category_text(m: &MainModule, actor: usize) -> String {
    match actors::interaction_category(m, actor) {
        Some(c) => format!("{c:02X}"),
        None => "?".to_string(),
    }
}

/// Log everything within `range` metres of the player, classified the way
/// the reference mod does it. Read-only. Debug=1 adds the structural dumps.
pub fn survey(m: &MainModule, w: &World, range: f32, max_lines: usize, debug: bool) {
    let manager = actors::find_manager_current(w).unwrap_or(w.manager);
    let Some(c) = crate::safe::read_ptr(manager + actors::CONTAINER_OFF) else {
        crate::log!("[survey] container pointer unreadable");
        return;
    };
    let Some(map) = actors::eid_map(c) else {
        let b = c + actors::EIDMAP_OFF;
        crate::log!(
            "[survey] eid map rejected: count={:?} cap={:?} keys={:?} values={:?}",
            crate::safe::read::<u32>(b + 8), crate::safe::read::<u32>(b + 0xC),
            crate::safe::read::<u64>(b + 0x10).map(|v| format!("0x{v:X}")),
            crate::safe::read::<u64>(b + 0x18).map(|v| format!("0x{v:X}"))
        );
        return;
    };
    let entries = map.entries();
    let Some(player) = actors::player_actor(manager) else {
        crate::log!("[survey] player actor not at manager+0x50");
        return;
    };
    let Some(ppos) = actors::actor_position(player) else {
        crate::log!("[survey] player has no readable position");
        return;
    };
    crate::log!(
        "[survey] {} actors ({} readable); player eid={:08X} at ({:.1} {:.1} {:.1})",
        map.count, entries.len(), actors::actor_eid(player).unwrap_or(0), ppos.x, ppos.y, ppos.z
    );
    match actors::inventory_tabs(player) {
        Some(tabs) => {
            crate::log!("[survey] inventory tabs (id:used/max): {}", tabs_summary(&tabs));
            if let Some(bag) = actors::bag_tab(&tabs, Some(1)) {
                match actors::tab_slots(&bag) {
                    Some(slots) => {
                        let shown: Vec<String> = slots
                            .iter()
                            .take(if debug { 200 } else { 12 })
                            .map(|sl| {
                                let rec = crate::tables::item_record(m, sl.item_index);
                                let key = rec.and_then(crate::tables::item_record_key).unwrap_or(0);
                                let name = rec.and_then(crate::tables::item_record_name).unwrap_or_else(|| "?".into());
                                let tab = rec.and_then(crate::tables::item_record_tab).unwrap_or(-2);
                                format!("[{}] {name} key={key} x{} tab={tab}", sl.slot, sl.count)
                            })
                            .collect();
                        crate::log!("[survey] bag: {} stacks; {}", slots.len(), shown.join("; "));
                        if debug {
                            dump_item_records(m, &tabs, &slots);
                        }
                    }
                    None => crate::log!("[survey] bag slots unreadable (tab ptr 0x{:X})", bag.ptr),
                }
            }
        }
        None => crate::log!("[survey] inventory not readable via player+0x68->+0xB8"),
    }
    let mut near: Vec<(f32, usize, u32, Vec3)> = entries
        .iter()
        .filter(|(_, a)| *a != player)
        .filter_map(|&(eid, a)| {
            let pos = actors::actor_position(a)?;
            let d = pos.dist(&ppos);
            (d <= range).then_some((d, a, eid, pos))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let mut shown = 0usize;
    for (d, a, eid, pos) in &near {
        let kind = actors::classify(m, *a, player);
        *counts.entry(format!("{kind:?}")).or_default() += 1;
        if shown >= max_lines {
            continue;
        }
        shown += 1;
        match kind {
            actors::Kind::Gather | actors::Kind::Item | actors::Kind::Interactable | actors::Kind::Unarmed => {
                let id = actors::node_identity(m, *a);
                let (rec, key, name, fam) = match &id {
                    Some(n) => (
                        n.index.to_string(),
                        n.key.map(|k| k.to_string()).unwrap_or_else(|| "-".into()),
                        n.name.clone().unwrap_or_else(|| "?".into()),
                        n.family.map(|f| format!("{f:?}")).unwrap_or_else(|| "-".into()),
                    ),
                    None => ("-".into(), "-".into(), "?".into(), "-".into()),
                };
                let st = match actors::status_bytes(m, *a) {
                    Some(s) => format!(" status={:02X}/{:02X}{}", s.kind, s.flag, if actors::is_owned_or_special(s) { " OWNED" } else { "" }),
                    None => " status=?".to_string(),
                };
                crate::log!(
                    "  {d:6.1} m  {kind:<12?} eid={eid:08X} ({:7.1} {:7.1} {:7.1}) rec={rec} key={key} name={name} family={fam}{st}",
                    pos.x, pos.y, pos.z
                );
            }
            actors::Kind::Inert if *d <= INERT_DETAIL_RANGE => {
                // Close-by inert gimmicks are what droppings look like; show
                // what record they carry even when it is not a gather record.
                let id = actors::node_identity(m, *a);
                let (rec, key, name) = match &id {
                    Some(n) => (
                        n.index.to_string(),
                        n.key.map(|k| k.to_string()).unwrap_or_else(|| "-".into()),
                        n.name.clone().unwrap_or_else(|| "?".into()),
                    ),
                    None => ("-".into(), "-".into(), "-".into()),
                };
                let comps: Vec<String> = actors::component_names(m, *a)
                    .into_iter()
                    .map(|(_, n)| actors::component_label(&actors::short_name(&n)))
                    .collect();
                crate::log!(
                    "  {d:6.1} m  {kind:<12?} eid={eid:08X} ({:7.1} {:7.1} {:7.1}) rec={rec} key={key} name={name} gimmick={} comps=[{}]",
                    pos.x, pos.y, pos.z, actors::interaction_words(m, *a), comps.join(" ")
                );
                if debug {
                    dump_gimmick_raw(m, *a);
                }
            }
            actors::Kind::Inert if !debug => {}
            // Characters, insects and everything unclassified: show the bytes
            // the catch rule keys on (`actors::catch_class`, section 15) so
            // insects and fish can be told apart from birds, NPCs and animals
            // - `cat=` is the class byte that decides it. `Catchable` is
            // named here rather than left to `_` precisely because it is the
            // line that says whether the rule picked the right actor.
            actors::Kind::Character | actors::Kind::Catchable | actors::Kind::Other => {
                catch_diagnostic_line(m, *a, *d, *eid, pos, kind);
            }
            _ => {
                catch_diagnostic_line(m, *a, *d, *eid, pos, kind);
            }
        }
    }
    let summary: Vec<String> = counts.iter().map(|(k, v)| format!("{k}={v}")).collect();
    crate::log!("[survey] within {range:.0} m: {}", summary.join(" "));
    if debug {
        if let Some((_, a, eid, _)) = near.iter().find(|(_, a, _, _)| actors::classify(m, *a, player) == actors::Kind::Gather) {
            crate::log!("[debug] strings of gather node eid={eid:08X}");
            dump_all_strings(m, *a);
            dump_gimmick_raw(m, *a);
        }
    }
}

/// One-off structural dump: what the manager points at, and every global
/// object the exe knows by class. Read-only.
pub fn census(m: &MainModule, w: &World) {
    let manager = actors::find_manager_current(w).unwrap_or(w.manager);
    crate::log!("[census] manager 0x{manager:X} fields with RTTI targets:");
    for (off, p, class) in actors::object_fields(m, manager, actors::MANAGER_SIZE) {
        crate::log!("[census]   manager+0x{off:03X} -> 0x{p:X} {class}");
    }
    if let Some(c) = crate::safe::read_ptr(manager + actors::CONTAINER_OFF) {
        crate::log!("[census] container 0x{c:X} ({}) fields with RTTI targets:", class_of(m, c));
        for (off, p, class) in actors::object_fields(m, c, actors::CONTAINER_SCAN) {
            crate::log!("[census]   container+0x{off:03X} -> 0x{p:X} {class}");
        }
    }
    let globals = actors::global_object_census(m);
    crate::log!("[census] {} distinct RTTI classes referenced from data sections:", globals.len());
    for (rva, p, class) in globals {
        crate::log!("[census]   +0x{rva:X} -> 0x{p:X} {class}");
    }
}

/// Dump a `{ptr p0; u32 a; u32 b; ptr p2; ptr p3}` structure and the first
/// qwords behind p0 and p2, annotated as actors where they look like actors.
fn dump_map(m: &MainModule, label: &str, base: usize) {
    use crate::safe;
    let (Some(p0), Some(a), Some(b)) = (
        safe::read::<u64>(base),
        safe::read::<u32>(base + 8),
        safe::read::<u32>(base + 0xC),
    ) else {
        return;
    };
    let p2: u64 = safe::read(base + 0x10).unwrap_or(0);
    let p3: u64 = safe::read(base + 0x18).unwrap_or(0);
    crate::log!("[map] {label}: p0=0x{p0:X} a={a} b={} p2=0x{p2:X} p3=0x{p3:X}", b as i32);
    for (nm, p) in [("p0", p0 as usize), ("p2", p2 as usize)] {
        if p < 0x10000 || !safe::readable(p, 8 * 6) {
            continue;
        }
        let mut cells = Vec::new();
        for i in 0..6 {
            let v: u64 = safe::read(p + i * 8).unwrap_or(0);
            let q = v as usize;
            let desc = if q >= 0x10000 && safe::readable(q, 0x100) {
                match (actors::actor_eid(q), actors::rtti_name(m, q)) {
                    (Some(e), Some(n)) => format!("{e:08X}:{}", actors::short_name(&n)),
                    (_, Some(n)) => format!("obj:{}", actors::short_name(&n)),
                    _ => "mem".into(),
                }
            } else {
                "-".into()
            };
            cells.push(format!("{v:X}({desc})"));
        }
        crate::log!("[map]   {nm}[0..6] = {}", cells.join(" "));
    }
}

/// Raw structural dump of the container and manager map/list structures.
pub fn dump_maps(m: &MainModule, w: &World) {
    let manager = actors::find_manager_current(w).unwrap_or(w.manager);
    if let Some(c) = crate::safe::read_ptr(manager + actors::CONTAINER_OFF) {
        dump_map(m, "ctr+0x08", c + 0x08);
        let mut off = 0x88;
        while off + 0x20 <= actors::CONTAINER_SCAN {
            dump_map(m, &format!("ctr+0x{off:X}"), c + off);
            off += 0x20;
        }
    }
    for off in [0x128usize, 0x130, 0x138, 0x140, 0x148, 0x150, 0x158, 0x180, 0x190, 0x438] {
        dump_map(m, &format!("mgr+0x{off:X}"), manager + off);
    }
}

fn qwords(addr: usize, n: usize) -> String {
    (0..n)
        .map(|i| crate::safe::read::<u64>(addr + i * 8).map(|v| format!("{v:X}")).unwrap_or_else(|| "?".into()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Raw look at the first records of a data table and at a gimmick component's
/// two side objects, to pin down where the record key lives.
pub fn dump_table_records(m: &MainModule, label: &str, slot_rva: usize, n: usize) {
    use crate::safe;
    let Some(mgr) = crate::tables::manager_at(m, slot_rva) else { return };
    let count: u32 = safe::read(mgr + 8).unwrap_or(0);
    let Some(arr) = safe::read_ptr(mgr + 0x58) else { return };
    crate::log!("[table] {label}: manager=0x{mgr:X} count={count} records=0x{arr:X}; manager qwords: {}", qwords(mgr, 14));
    for i in 0..n {
        let Some(rec) = safe::read_ptr(arr + i * 8) else { continue };
        let name = crate::tables::name(m, slot_rva, i as u32).unwrap_or_else(|| "?".into());
        crate::log!("[table]   [{i}] rec=0x{rec:X} name={name}: {}", qwords(rec, 10));
    }
}

pub fn dump_gimmick_raw(m: &MainModule, actor: usize) {
    use crate::safe;
    let Some((off, _)) = actors::component_names(m, actor).into_iter().find(|(_, n)| n.contains("ClientGimmickActorComponent")) else { return };
    let Some(sub) = safe::read_ptr(actor + 0x68) else { return };
    let Some(comp) = safe::read_ptr(sub + off) else { return };
    crate::log!("           gimmick comp=0x{comp:X} +0xC0..+0xF0: {}", qwords(comp + 0xC0, 6));
    for o in [0xC0usize, 0xE0] {
        if let Some(p) = safe::read_ptr(comp + o) {
            crate::log!("           comp+0x{o:X} -> 0x{p:X}: {} {}", qwords(p, 6),
                actors::rtti_name(m, p).map(|n| actors::short_name(&n)).unwrap_or_default());
        }
    }
}

fn hexascii(addr: usize, n: usize) -> String {
    let mut hex = String::new();
    let mut asc = String::new();
    for i in 0..n {
        match crate::safe::read::<u8>(addr + i) {
            Some(b) => {
                hex.push_str(&format!("{b:02X} "));
                asc.push(if (0x20..0x7F).contains(&b) { b as char } else { '.' });
            }
            None => {
                hex.push_str("?? ");
                asc.push('?');
            }
        }
    }
    format!("{hex}|{asc}|")
}

/// Look inside an eid-map value object for anything that is an actor.
pub fn dump_value_object(m: &MainModule, key: u32, obj: usize) {
    crate::log!("[val] key={key:08X} obj=0x{obj:X}: {}", qwords(obj, 12));
    for off in (0..0x80).step_by(8) {
        if let Some(p) = crate::safe::read_ptr(obj + off) {
            if let Some(n) = actors::rtti_name(m, p) {
                let eid = actors::actor_eid(p).map(|e| format!("{e:08X}")).unwrap_or_else(|| "-".into());
                crate::log!("[val]   +0x{off:02X} -> 0x{p:X} {} eid={eid}", actors::short_name(&n));
            }
        }
    }
}

pub fn dump_record_bytes(m: &MainModule, label: &str, slot_rva: usize, idx: usize) {
    let Some(mgr) = crate::tables::manager_at(m, slot_rva) else { return };
    let Some(arr) = crate::safe::read_ptr(mgr + 0x58) else { return };
    let Some(rec) = crate::safe::read_ptr(arr + idx * 8) else { return };
    for line in 0..4 {
        crate::log!("[table] {label}[{idx}] +0x{:02X}: {}", line * 32, hexascii(rec + line * 32, 32));
    }
}

/// Check every u32 in a gimmick node's side objects against the gimmick key index.
pub fn match_gimmick_keys(m: &MainModule, actor: usize, index: &std::collections::HashMap<u32, usize>) {
    use crate::safe;
    let Some((off, _)) = actors::component_names(m, actor).into_iter().find(|(_, n)| n.contains("ClientGimmickActorComponent")) else { return };
    let Some(sub) = safe::read_ptr(actor + 0x68) else { return };
    let Some(comp) = safe::read_ptr(sub + off) else { return };
    let mut places: Vec<(String, usize)> = vec![("comp".into(), comp)];
    for o in [0xC0usize, 0xE0] {
        if let Some(p) = safe::read_ptr(comp + o) {
            places.push((format!("comp+0x{o:X}->"), p));
        }
    }
    for (label, base) in places {
        for i in 0..64 {
            let Some(v) = safe::read::<u32>(base + i * 4) else { break };
            if v == 0 || v == 0xFFFF_FFFF { continue; }
            if let Some(&rec) = index.get(&v) {
                crate::log!(
                    "           key match: {label}+0x{:X} = {v} -> rec 0x{rec:X} names={:?}",
                    i * 4, crate::tables::gimmick_name_candidates(rec)
                );
            }
        }
    }
}

/// The object behind `+0xE0 -> [1]` on a gimmick node: class, qwords, and any
/// name reachable by the known record layouts.
pub fn dump_type_object(m: &MainModule, actor: usize) {
    use crate::safe;
    let Some((off, _)) = actors::component_names(m, actor).into_iter().find(|(_, n)| n.contains("ClientGimmickActorComponent")) else { return };
    let Some(sub) = safe::read_ptr(actor + 0x68) else { return };
    let Some(comp) = safe::read_ptr(sub + off) else { return };
    let Some(t) = safe::read_ptr(comp + 0xE0) else { return };
    let Some(obj) = safe::read_ptr(t + 8) else { return };
    let class = actors::rtti_name(m, obj).map(|n| actors::short_name(&n)).unwrap_or_else(|| "?".into());
    crate::log!("           type obj 0x{obj:X} ({class}): {}", qwords(obj, 12));
    crate::log!("           type obj name candidates: {:?} / +0x08 chain {:?}",
        crate::tables::record_name_any(obj),
        safe::read_ptr(obj + 8).and_then(|p| actors::rtti_name(m, p)).map(|n| actors::short_name(&n)));
    for o in (0..0x60).step_by(8) {
        if let Some(p) = safe::read_ptr(obj + o) {
            if let Some(s) = safe::read_cstr(p, 48).filter(|s| s.len() >= 4) {
                crate::log!("           type obj +0x{o:X} -> str {s:?}");
            }
            if let Some(n) = actors::rtti_name(m, p) {
                crate::log!("           type obj +0x{o:X} -> {}", actors::short_name(&n));
            }
        }
    }
}

/// Scan an object's qwords for pointers into known table records or to strings.
fn scan_links(
    m: &MainModule,
    label: &str,
    base: usize,
    size: usize,
    records: &std::collections::HashMap<usize, (String, usize)>,
) {
    use crate::safe;
    for off in (0..size).step_by(8) {
        let Some(p) = safe::read_ptr(base + off) else { continue };
        if let Some((table, idx)) = records.get(&p) {
            let name = crate::tables::record_name_any(p).unwrap_or_else(|| "?".into());
            crate::log!("           link {label}+0x{off:X} -> {table}[{idx}] {name}");
            continue;
        }
        if m.contains(p) {
            continue;
        }
        if let Some(s) = safe::read_cstr(p, 48) {
            if s.len() >= 4 && s.chars().any(|c| c.is_ascii_alphabetic()) {
                crate::log!("           str  {label}+0x{off:X} -> {s:?}");
            }
        }
    }
}

/// Depth-2: for every pointer field of `base`, scan the target for record pointers.
fn scan_links_deep(
    m: &MainModule,
    label: &str,
    base: usize,
    size: usize,
    records: &std::collections::HashMap<usize, (String, usize)>,
) {
    use crate::safe;
    for off in (0..size).step_by(8) {
        let Some(p) = safe::read_ptr(base + off) else { continue };
        if m.contains(p) || !safe::readable(p, 0x100) {
            continue;
        }
        for o2 in (0..0x100).step_by(8) {
            let Some(q) = safe::read_ptr(p + o2) else { continue };
            if let Some((table, idx)) = records.get(&q) {
                let name = crate::tables::record_name_any(q).unwrap_or_else(|| "?".into());
                crate::log!("           link {label}+0x{off:X}->+0x{o2:X} -> {table}[{idx}] {name}");
            }
        }
    }
}

/// Where does a node point at its definition? Check actor, sub-object, every
/// component, and one level below each of them.
pub fn find_record_links(
    m: &MainModule,
    actor: usize,
    records: &std::collections::HashMap<usize, (String, usize)>,
) {
    use crate::safe;
    scan_links(m, "actor", actor, 0x300, records);
    scan_links_deep(m, "actor", actor, 0x300, records);
    let Some(sub) = safe::read_ptr(actor + 0x68) else { return };
    scan_links(m, "sub", sub, 0x400, records);
    scan_links_deep(m, "sub", sub, 0x400, records);
    for (off, name) in actors::component_names(m, actor) {
        let Some(comp) = safe::read_ptr(sub + off) else { continue };
        let label = actors::component_label(&actors::short_name(&name));
        let size = if name.contains("Gimmick") { 0x600 } else { 0x300 };
        scan_links(m, &label, comp, size, records);
        scan_links_deep(m, &label, comp, size, records);
        if name.contains("Gimmick") {
            for o in [0x420usize, 0x428, 0x5A8] {
                let Some(p) = safe::read_ptr(comp + o) else {
                    crate::log!("           gimmick+0x{o:X} = null");
                    continue;
                };
                let class = actors::rtti_name(m, p).map(|n| actors::short_name(&n)).unwrap_or_else(|| "?".into());
                let rec = records.get(&p).map(|(t, i)| format!(" [{t}[{i}]]")).unwrap_or_default();
                crate::log!("           gimmick+0x{o:X} -> 0x{p:X} ({class}){rec}: {}", qwords(p, 10));
                scan_links(m, &format!("gimmick+0x{o:X}->"), p, 0x200, records);
                scan_links_deep(m, &format!("gimmick+0x{o:X}->"), p, 0x100, records);
            }
        }
    }
}

/// Scan u32 fields of a node's objects for values that are record keys.
pub fn find_key_links(
    m: &MainModule,
    actor: usize,
    keys: &std::collections::HashMap<u32, Vec<(String, usize, usize)>>,
) {
    use crate::safe;
    let mut places: Vec<(String, usize, usize)> = vec![("actor".into(), actor, 0x300)];
    if let Some(sub) = safe::read_ptr(actor + 0x68) {
        places.push(("sub".into(), sub, 0x400));
        for (off, name) in actors::component_names(m, actor) {
            if let Some(comp) = safe::read_ptr(sub + off) {
                let label = actors::component_label(&actors::short_name(&name));
                let size = if name.contains("Gimmick") { 0x600 } else { 0x300 };
                places.push((label.clone(), comp, size));
                if name.contains("Gimmick") {
                    for o in [0xC0usize, 0xE0, 0x420, 0x428] {
                        if let Some(p) = safe::read_ptr(comp + o) {
                            places.push((format!("{label}+0x{o:X}->"), p, 0x80));
                        }
                    }
                }
            }
        }
    }
    for (label, base, size) in places {
        for off in (0..size).step_by(4) {
            let Some(v) = safe::read::<u32>(base + off) else { break };
            if let Some(hits) = keys.get(&v) {
                let desc: Vec<String> = hits
                    .iter()
                    .take(4)
                    .map(|(t, i, rec)| format!("{t}[{i}]={}", crate::tables::record_name_any(*rec).unwrap_or_else(|| "?".into())))
                    .collect();
                crate::log!("           key  {label}+0x{off:X} = {v} -> {}", desc.join(" | "));
            }
        }
    }
}

/// Look at the ServerActorManager: its fields, and whether it has an eid map
/// holding the given entity id.
pub fn server_side(m: &MainModule, census: &[(usize, usize, String)], eid: u32) {
    use crate::safe;
    let Some((_, srv, _)) = census.iter().find(|(_, _, c)| c == "ServerActorManager") else {
        crate::log!("[server] no ServerActorManager in census");
        return;
    };
    crate::log!("[server] ServerActorManager at 0x{srv:X}");
    for (off, p, class) in actors::object_fields(m, *srv, 0x600) {
        crate::log!("[server]   +0x{off:03X} -> 0x{p:X} {class}");
    }
    // Same container/map shape as the client side?
    for coff in [0x08usize, 0x10, 0x18, 0x20] {
        let Some(c) = safe::read_ptr(srv + coff) else { continue };
        if !safe::readable(c, 0x100) {
            continue;
        }
        let Some(map) = actors::eid_map(c) else { continue };
        let entries = map.entries();
        crate::log!("[server]   container via +0x{coff:X}: eid map count={} readable={}", map.count, entries.len());
        if let Some((_, a)) = entries.iter().find(|(e, _)| *e == eid) {
            let class = actors::rtti_name(m, *a).map(|n| actors::short_name(&n)).unwrap_or_else(|| "?".into());
            crate::log!("[server]   eid {eid:08X} -> server actor 0x{a:X} {class}: {}", qwords(*a, 12));
            for (off, name) in actors::component_names(m, *a) {
                crate::log!("[server]     comp +0x{off:02X} {}", actors::short_name(&name));
            }
            for off in (0..0x200).step_by(8) {
                if let Some(p) = safe::read_ptr(*a + off) {
                    if let Some(s) = safe::read_cstr(p, 48).filter(|s| s.len() >= 4) {
                        crate::log!("[server]     str +0x{off:X} -> {s:?}");
                    }
                    if let Some(n) = actors::rtti_name(m, p) {
                        crate::log!("[server]     +0x{off:X} -> {}", actors::short_name(&n));
                    }
                }
            }
        }
    }
}

/// Every string reachable one pointer away from the actor, its sub-object and components.
pub fn dump_all_strings(m: &MainModule, actor: usize) {
    use crate::safe;
    let mut places: Vec<(String, usize, usize)> = vec![("actor".into(), actor, 0x300)];
    if let Some(sub) = safe::read_ptr(actor + 0x68) {
        places.push(("sub".into(), sub, 0x400));
        for (off, name) in actors::component_names(m, actor) {
            if let Some(comp) = safe::read_ptr(sub + off) {
                let size = if name.contains("Gimmick") { 0x600 } else { 0x300 };
                places.push((actors::component_label(&actors::short_name(&name)), comp, size));
            }
        }
    }
    for (label, base, size) in places {
        for off in (0..size).step_by(8) {
            let Some(p) = safe::read_ptr(base + off) else { continue };
            if m.contains(p) { continue; }
            if let Some(s) = safe::read_cstr(p, 200) {
                if s.len() >= 6 && s.chars().filter(|c| c.is_ascii_alphabetic()).count() >= 4 {
                    crate::log!("           str {label}+0x{off:X} -> {s:?}");
                }
            }
        }
    }
}

/// Hunt for gimmickinfo record keys (the collect families use 17,0xx,xxx keys,
/// e.g. peony_01 = 17020006) anywhere in a node's objects, scanning further
/// than before.
pub fn hunt_keys(m: &MainModule, actor: usize, keys: &std::collections::HashMap<u32, Vec<(String, usize, usize)>>) {
    use crate::safe;
    let mut places: Vec<(String, usize, usize)> = vec![("actor".into(), actor, 0x800)];
    if let Some(sub) = safe::read_ptr(actor + 0x68) {
        places.push(("sub".into(), sub, 0x1000));
        for (off, name) in actors::component_names(m, actor) {
            if let Some(comp) = safe::read_ptr(sub + off) {
                let label = actors::component_label(&actors::short_name(&name));
                places.push((label.clone(), comp, 0x1000));
                if name.contains("Gimmick") {
                    for o in [0xC0usize, 0xE0, 0x420, 0x428, 0x5A8] {
                        if let Some(p) = safe::read_ptr(comp + o) {
                            places.push((format!("{label}+0x{o:X}->"), p, 0x200));
                            if o == 0xE0 {
                                if let Some(q) = safe::read_ptr(p + 8) {
                                    places.push((format!("{label}+0xE0->[1]->"), q, 0x200));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    for (label, base, size) in places {
        for off in (0..size).step_by(4) {
            let Some(v) = safe::read::<u32>(base + off) else { break };
            let in_collect_range = (17_000_000..17_999_999).contains(&v);
            if let Some(hits) = keys.get(&v).filter(|h| h.iter().any(|(t, _, _)| t == "GimmickInfoManager")) {
                let desc: Vec<String> = hits.iter().filter(|(t, _, _)| t == "GimmickInfoManager").take(2)
                    .map(|(t, i, rec)| format!("{t}[{i}]={}", crate::tables::record_name_any(*rec).unwrap_or_else(|| "?".into()))).collect();
                crate::log!("[hunt]   {label}+0x{off:X} = {v} -> {}", desc.join(" | "));
            } else if in_collect_range {
                crate::log!("[hunt]   {label}+0x{off:X} = {v} (collect-range, no record)");
            }
        }
    }
}

/// What the gather hotkey found: the nearest `Gather` node in range plus the
/// two player values the event carries.
#[derive(Debug, Clone)]
pub struct GatherTarget {
    pub eid: u32,
    pub record: u16,
    pub actor: usize,
    pub player_actor: usize,
    pub dist: f32,
    pub name: String,
    /// Gather family, or "Item" for a ground item.
    pub family: String,
    pub mode: crate::payload::PickupMode,
    /// Carries the game's interaction object (`Gather`) rather than `Unarmed`.
    pub armed: bool,
    pub player_eid: u32,
    pub route: u32,
}

/// The game's own event builder (`FUN_1426B21A0`) fills `ev+0x50` with
/// `actor+0x60` (the eid) and `ev+0x58` ("route") with `actor+0x58` of the
/// actor it is given. The reference mod, built for an older build, read the
/// route at `player+0x90`; that value is only logged here for comparison.
pub const PLAYER_ROUTE_OFF: usize = 0x58;
pub const PLAYER_ROUTE_MOD_OFF: usize = 0x90;

pub fn player_route(player: usize) -> Option<u32> {
    crate::safe::read(player + PLAYER_ROUTE_OFF)
}

/// One enumeration of the world, shared by everything a tick needs.
pub struct Scene {
    pub player: usize,
    pub player_eid: u32,
    pub route: u32,
    pub ppos: Vec3,
    pub entries: Vec<(u32, usize)>,
    /// None when the inventory could not be read this tick.
    pub tabs: Option<Vec<actors::InventoryTab>>,
}

pub fn tabs_summary(tabs: &[actors::InventoryTab]) -> String {
    tabs.iter().map(|t| format!("{}:{}/{}", t.id, t.used, t.max)).collect::<Vec<_>>().join(" ")
}

impl Scene {
    pub fn actor(&self, eid: u32) -> Option<usize> {
        self.entries.iter().find(|(e, _)| *e == eid).map(|(_, a)| *a)
    }
}

pub fn scene(m: &MainModule, w: &World) -> Result<Scene, String> {
    let _ = m;
    let manager = actors::find_manager_current(w).unwrap_or(w.manager);
    let c = crate::safe::read_ptr(manager + actors::CONTAINER_OFF).ok_or("container pointer unreadable")?;
    let map = actors::eid_map(c).ok_or("eid map rejected")?;
    let player = actors::player_actor(manager).ok_or("player actor not at manager+0x50")?;
    let player_eid = actors::actor_eid(player).ok_or("player eid unreadable")?;
    let route = player_route(player).ok_or("player+0x58 unreadable")?;
    let ppos = actors::actor_position(player).ok_or("player position unreadable")?;
    let tabs = actors::inventory_tabs(player);
    Ok(Scene { player, player_eid, route, ppos, entries: map.entries(), tabs })
}

/// One bit per [`actors::interaction_category`] value (256 of them in four
/// words), set the first time a catchable creature of that class is passed
/// over as unknown. A catch candidate is looked at every gather tick, so
/// without this the log would carry the same line several times a second;
/// with it each unrecognised class says its piece once per session. Four
/// atomics, no allocation and no lock on the path that reads it.
static UNKNOWN_CATCH_CATEGORIES: [AtomicU64; 4] =
    [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// Claim the right to log category `c`: true exactly once per value per
/// session. `c >> 6` is 0..=3 for any `u8`, so the `get` never misses; it is
/// a `get` rather than an index because indexing is denied in shipped code.
fn first_sighting_of(c: u8) -> bool {
    let bit = 1u64 << (c & 63);
    match UNKNOWN_CATCH_CATEGORIES.get(usize::from(c >> 6)) {
        Some(word) => word.fetch_or(bit, Ordering::Relaxed) & bit == 0,
        None => false,
    }
}

/// Nearest node within `GatherRange` metres that classifies as `Gather`
/// (interaction object present and the gimmick record is a
/// Foraging/Logging/Mining/Ore record), skipping eids for which `skip` is
/// true. Ground items (`GatherItems`) and catchable creatures (`GatherBugs`
/// for insects, `GatherFish` for fish) are candidates too; a creature comes
/// back with `mode: Catch`, which is a different game event entirely (see
/// `payload::catch_payload`). `cfg` is the live config, so what counts as a
/// candidate follows the ini without this having to be told twice. Read-only.
pub fn nearest_gather(
    m: &MainModule,
    sc: &Scene,
    cfg: &Config,
    skip: &dyn Fn(u32) -> bool,
) -> Result<GatherTarget, String> {
    let range = cfg.gather_range;
    let unarmed = cfg.gather_unarmed;
    // (dist, eid, actor, pos, kind) for every gather-record node in range.
    let mut nodes: Vec<(f32, u32, usize, Vec3, actors::Kind)> = Vec::new();
    for &(eid, a) in &sc.entries {
        if a == sc.player {
            continue;
        }
        let Some(pos) = actors::actor_position(a) else { continue };
        let d = pos.dist(&sc.ppos);
        if d > range {
            continue;
        }
        let kind = actors::classify(m, a, sc.player);
        if matches!(
            kind,
            actors::Kind::Gather
                | actors::Kind::Unarmed
                | actors::Kind::Item
                | actors::Kind::Catchable
        ) {
            // Shop goods, quest items, decoration: the reference mod's first
            // rejection, and the difference between an ore chunk and a cup on
            // a merchant's table (both are `item_basic_*`). Unreadable status
            // counts as owned for items; nodes have no owner.
            if kind == actors::Kind::Item {
                match actors::status_bytes(m, a) {
                    Some(st) if !actors::is_owned_or_special(st) => {}
                    _ => continue,
                }
            }
            nodes.push((d, eid, a, pos, kind));
        }
    }
    nodes.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut unarmed_seen = 0usize;
    // Nodes passed over only because their family is switched off. Creatures
    // skipped by `GatherBugs=0` or `GatherFish=0` are counted here too: to the
    // player it is the same kind of "you switched that off", and the summary
    // reads the same. A creature of an unknown class is *not* counted, because
    // nothing the player can set would have taken it.
    let mut family_off_seen = 0usize;
    for &(d, eid, a, pos, kind) in &nodes {
        // Catchable creatures carry no gimmick record, so they are decided
        // before `node_identity` (which returns None for them) rather than
        // after. `Kind::Catchable` only says the actor is shaped like a
        // creature the game lets you catch; the class byte says whether it is
        // one we have ever actually caught, and birds in flight and several
        // unidentified species pass the first test but fail the second.
        if kind == actors::Kind::Catchable {
            let (what, family) = match actors::catch_class(m, a) {
                Some(CatchClass::Bug) if !cfg.gather_bugs => {
                    family_off_seen += 1;
                    continue;
                }
                Some(CatchClass::Fish) if !cfg.gather_fish => {
                    family_off_seen += 1;
                    continue;
                }
                Some(CatchClass::Bug) => ("bug", "Bug"),
                Some(CatchClass::Fish) => ("fish", "Fish"),
                Some(CatchClass::Unknown(c)) => {
                    if first_sighting_of(c) {
                        crate::log!(
                            "[gather] catchable creature cat={c:02X} at {d:.0} m is not a known bug/fish class; skipped (catch one by hand with F7 recording to add it)"
                        );
                    }
                    continue;
                }
                // Classified `Catchable` a moment ago and unreadable now: the
                // actor went away mid-pass. Nothing to say about it.
                None => continue,
            };
            if skip(eid) {
                continue;
            }
            return Ok(GatherTarget {
                eid,
                // No gimmick record: 0xFFFF is the same "none" the game's own
                // record index uses, and nothing downstream looks it up
                // (yield learning is Gather-only).
                record: 0xFFFF,
                actor: a,
                player_actor: sc.player,
                dist: d,
                name: format!("{what} cat={}", category_text(m, a)),
                family: family.to_string(),
                mode: crate::payload::PickupMode::Catch,
                armed: true,
                player_eid: sc.player_eid,
                route: sc.route,
            });
        }
        if kind == actors::Kind::Unarmed {
            unarmed_seen += 1;
            if !unarmed {
                continue;
            }
            // An armed node at the same spot means this is its twin: use the armed one.
            if nodes.iter().any(|n| n.4 == actors::Kind::Gather && n.3.dist(&pos) < 0.1) {
                continue;
            }
        }
        if skip(eid) {
            continue;
        }
        let Some(id) = actors::node_identity(m, a) else { continue };
        let name = id.name.clone().unwrap_or_else(|| format!("rec{}", id.index));
        let (family, mode) = if kind == actors::Kind::Item {
            if !cfg.gather_items || !actors::is_basic_item_record(&name) {
                continue;
            }
            if !cfg.gather_gear && actors::is_gear_item_record(&name) {
                continue;
            }
            ("Item".to_string(), crate::payload::PickupMode::Item)
        } else {
            let Some(f) = id.family else { continue };
            // Filtered on the typed family, before it becomes a string.
            if !cfg.allows_family(f) {
                family_off_seen += 1;
                continue;
            }
            (format!("{f:?}"), crate::payload::PickupMode::Gather)
        };
        return Ok(GatherTarget {
            eid,
            record: id.index,
            actor: a,
            player_actor: sc.player,
            dist: d,
            name,
            family,
            mode,
            armed: kind == actors::Kind::Gather,
            player_eid: sc.player_eid,
            route: sc.route,
        });
    }
    if unarmed_seen > 0 && !unarmed {
        return Err(format!("no armed Gather node within {range:.1} m ({unarmed_seen} unarmed; GatherUnarmed=1 to try them)"));
    }
    if family_off_seen > 0 {
        return Err(format!("no Gather node within {range:.1} m ({family_off_seen} skipped by the Gather<Family>/GatherBugs/GatherFish switches)"));
    }
    Err(format!("no Gather node within {range:.1} m"))
}

/// Debug: raw item records for a few bag stacks and a few equipment-tab
/// entries, to locate the stack-limit field by comparison (a material should
/// show a large limit where gear shows 1).
pub fn dump_item_records(m: &MainModule, tabs: &[actors::InventoryTab], bag: &[actors::InventorySlot]) {
    let mut picks: Vec<(String, actors::InventorySlot)> = bag.iter().take(4).map(|s| ("bag".to_string(), *s)).collect();
    for t in tabs.iter().filter(|t| t.id != 1 && t.used > 0).take(3) {
        if let Some(slots) = actors::tab_slots(t) {
            for s in slots.iter().take(2) {
                picks.push((format!("tab{}", t.id), *s));
            }
        }
    }
    for (label, sl) in picks {
        let Some(rec) = crate::tables::item_record(m, sl.item_index) else { continue };
        let name = crate::tables::item_record_name(rec).unwrap_or_else(|| "?".into());
        crate::log!("[itemrec] {label} slot {} {name} x{} rec=0x{rec:X} idx={}", sl.slot, sl.count, sl.item_index);
        for line in 0..(0x480 / 32) {
            crate::log!("[itemrec]   +0x{:03X}: {}", line * 32, hexascii(rec + line * 32, 32));
        }
    }
}
