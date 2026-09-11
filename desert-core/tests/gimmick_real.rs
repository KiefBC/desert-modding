//! Runs only on request:
//! `cargo test --release --target x86_64-unknown-linux-gnu -p desert-core -- --ignored --nocapture`
//!
//! Needs the Steam install and DMM's table backup mounted at the paths below.
//! Same shape as `desert-looter/tests/game_exe.rs`: these are the tests to run
//! after a game update, because they are what break first.

use std::collections::{BTreeMap, BTreeSet};

use desert_core::creature::{CATCH_BYTES, CATCH_SITE, CATCH_STOLEN};
use desert_core::gimmick::{self, LOADER_PROLOGUE, LOADER_STOLEN};
use desert_core::pattern::{Found, Pattern};
use desert_core::pe;

const EXE: &str = "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe";
const TABLE: &str = "/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin";
const PACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../desert-gatherer-dmm");

/// RVA of the `gimmickinfo` record loader on Steam build 25246367
/// (`FUN_1403856b0` on the build before it; the function is the same, the
/// address is not).
const LOADER_RVA: usize = 0x385cd0;

/// RVAs of the two `*InfoManager` global pointer slots on Steam build 25246367.
/// These are the oracle for `gimmick::resolve_manager_slot`, and are the values
/// `desert-looter/src/tables.rs` used to carry as hard-coded constants before
/// the slots were resolved by content at startup.
const ITEM_INFO_SLOT_RVA: usize = 0x6C2E2E8;
const GIMMICK_INFO_SLOT_RVA: usize = 0x6C2E308;

/// RVA of the `mov r8d,1` inside the catch-count function on Steam build
/// 25246367 (`FUN_142a73c20` on the build before it;
/// `docs/reference-internals.md` section 17.4).
const CATCH_RVA: usize = 0x2a75891;

#[test]
#[ignore]
fn resolves_the_record_loader_in_the_game_exe() {
    let file = std::fs::read(EXE).expect("game exe present");
    let h = pe::parse(&file).expect("PE32+ headers");
    let img = pe::file_to_image(&file).expect("image layout");
    let base = h.image_base as usize;

    let loader = gimmick::resolve_record_loader(&img, base).expect("loader resolved");
    println!("image_base    = 0x{base:X}");
    println!("record loader = 0x{loader:X} (rva 0x{:X})", loader - base);
    assert_eq!(loader, base + LOADER_RVA);

    let prologue = img.get(LOADER_RVA..LOADER_RVA + LOADER_STOLEN).expect("prologue in image");
    println!("prologue      = {}", hex(prologue));
    assert_eq!(prologue, &LOADER_PROLOGUE[..]);
}

/// Desert Looter's two info-manager slots, resolved by content instead of by
/// the two constants the plugin used to carry. Each is the `mov rbx,[rip+disp]`
/// a short way before the accessor copy that names the table.
#[test]
#[ignore]
fn resolves_the_info_manager_slots_in_the_game_exe() {
    let file = std::fs::read(EXE).expect("game exe present");
    let img = pe::file_to_image(&file).expect("image layout");

    let item = gimmick::resolve_manager_slot(&img, gimmick::ITEM_TABLE);
    show("iteminfo", &item);
    assert_eq!(item, Ok(ITEM_INFO_SLOT_RVA));

    let gim = gimmick::resolve_manager_slot(&img, gimmick::GIMMICK_TABLE);
    show("gimmickinfo", &gim);
    assert_eq!(gim, Ok(GIMMICK_INFO_SLOT_RVA));
}

/// Desert Gatherer's second hook: the catch count for insects and fish.
///
/// Unlike the record loader, this one is a patch on the game's *code*, so the
/// signature and the bytes it overwrites are the whole safety argument. Both
/// are checked here: the site must be found exactly once, and the 13 bytes
/// there must be the `mov r8d,1` + `lea rdx,[rbp+0x1d0]` the stub replaces
/// and replays. If either fails after a game update, the plugin refuses to
/// patch at runtime and only the Bugs/Fish multipliers stop working.
#[test]
#[ignore]
fn finds_the_catch_count_site_in_the_game_exe() {
    let file = std::fs::read(EXE).expect("game exe present");
    let img = pe::file_to_image(&file).expect("image layout");

    let pat = Pattern::parse(CATCH_SITE).expect("the catch-site pattern parses");
    let at = match pat.find_unique(&img) {
        Found::Unique(at) => at,
        other => panic!("catch site: {other:?}"),
    };
    println!("catch count   = rva 0x{at:X} (0x{:X})", at + 0x1_4000_0000usize);
    assert_eq!(at, CATCH_RVA);

    let stolen = img.get(at..at + CATCH_STOLEN).expect("stolen bytes in image");
    println!("stolen bytes  = {}", hex(stolen));
    assert_eq!(stolen, &CATCH_BYTES[..]);
}

/// The `[slot]` line Desert Looter logs, printed here for the same reason.
fn show(table: &str, got: &Result<usize, String>) {
    match got {
        Ok(rva) => println!("[slot] {table:<22} = +0x{rva:X}"),
        Err(e) => println!("[slot] {table:<22} NOT FOUND: {e}"),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X} ")).collect::<String>().trim_end().to_string()
}

// ---------------------------------------------------------------------------
// Pack cross-check
// ---------------------------------------------------------------------------

/// Minimal JSON field reader: no serde, no new dependencies. The pack files are
/// machine-written by `desert-gatherer-dmm/rebase.py`, so every change object
/// has its fields in the same order (`offset`, `record_key`,
/// `record_rel_offset`, `entry`, ...) and plain string searching is enough.
fn number_after(text: &str, key: &str, from: usize) -> Option<(u64, usize)> {
    let at = text.get(from..)?.find(key)? + from + key.len();
    let rest = text.get(at..)?;
    let digits: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    Some((digits.parse().ok()?, at))
}

fn string_after(text: &str, key: &str, from: usize) -> Option<(String, usize)> {
    let at = text.get(from..)?.find(key)? + from + key.len();
    let rest = text.get(at..)?;
    let open = rest.find('"')? + 1;
    let close = rest.get(open..)?.find('"')? + open;
    Some((rest.get(open..close)?.to_string(), at + close))
}

/// A record is identified by its key and its name, the way the pack does it.
type Record = (u32, String);
/// One pack family: which offsets it patches in each record.
type Family = BTreeMap<Record, BTreeSet<usize>>;

/// `(record key, record name) -> the offsets that family patches in it`.
/// `None` if a change object does not have its fields in that documented order.
fn read_family(text: &str) -> Option<Family> {
    let mut out: Family = BTreeMap::new();
    let mut cur = 0usize;
    while let Some((offset, at)) = number_after(text, "\"offset\":", cur) {
        let (key, at) = number_after(text, "\"record_key\":", at)?;
        let (entry, at) = string_after(text, "\"entry\":", at)?;
        out.entry((key as u32, entry)).or_default().insert(offset as usize);
        cur = at;
    }
    Some(out)
}

/// Find every wanted record in one pass over the table, the same rule as
/// `rebase.py`'s `locate_records`: `u32 key`, `u32 len` equal to the name
/// length, the name, a NUL — and exactly one hit per record.
fn locate_records(
    table: &[u8],
    wanted: &BTreeSet<Record>,
) -> Result<BTreeMap<Record, usize>, String> {
    let mut hits: BTreeMap<Record, Vec<usize>> = BTreeMap::new();
    for p in 0..table.len() {
        let Some(rest) = table.get(p..) else { break };
        if let Some(h) = gimmick::parse_header(rest) {
            let k = (h.key, h.name);
            if wanted.contains(&k) {
                hits.entry(k).or_default().push(p);
            }
        }
    }
    let mut out = BTreeMap::new();
    for k in wanted {
        match hits.get(k).map(Vec::as_slice) {
            Some([one]) => {
                out.insert(k.clone(), *one);
            }
            other => return Err(format!("record {} {:?}: expected 1 hit, got {other:?}", k.0, k.1)),
        }
    }
    Ok(out)
}

#[test]
#[ignore]
fn multiply_reproduces_the_dmm_pack_edits() {
    let table = std::fs::read(TABLE).expect("clean gimmickinfo body present");
    println!("table: {} bytes", table.len());
    assert_eq!(table.len(), 22_325_302, "not the build 25116796 clean table");

    let mut families: Vec<(String, Family)> = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(PACK)
        .expect("desert-gatherer-dmm present")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with("2X.json"))
        .collect();
    names.sort();
    assert_eq!(names.len(), 4, "expected four 2X families, got {names:?}");
    for n in names {
        let text = std::fs::read_to_string(format!("{PACK}/{n}")).expect("pack json readable");
        let family = read_family(&text).expect("pack json fields in the documented order");
        families.push((n, family));
    }

    // One pass over the 22 MB table finds every record all four families name.
    let wanted: BTreeSet<Record> =
        families.iter().flat_map(|(_, f)| f.keys().cloned()).collect();
    let starts = locate_records(&table, &wanted).expect("every packed record located exactly once");
    let mut sorted: Vec<usize> = starts.values().copied().collect();
    sorted.sort_unstable();
    println!("records located: {} (all unique)", starts.len());

    let mut grand_offsets = 0usize;
    let mut grand_records = 0usize;
    for (name, family) in &families {
        let mut offsets = 0usize;
        for (rec, pack) in family {
            let start = *starts.get(rec).expect("record located");
            // The record ends where the next known record begins; 64 KiB is the
            // fallback for the last one (the largest gather record is far
            // smaller than that).
            let next = sorted.iter().copied().find(|&s| s > start).unwrap_or(usize::MAX);
            let end = next.min(start + 65536).min(table.len());
            let slice = table.get(start..end).expect("record slice");

            let last = *pack.iter().next_back().expect("family patches this record");
            let got: BTreeSet<usize> = gimmick::multiply(slice, 2)
                .iter()
                .map(|e| start + e.offset)
                .filter(|&o| o <= last)
                .collect();
            assert_eq!(
                got,
                *pack,
                "{name}: record {} {:?} at {start}: missing {:?}, extra {:?}",
                rec.0,
                rec.1,
                pack.difference(&got).collect::<Vec<_>>(),
                got.difference(pack).collect::<Vec<_>>()
            );
            // Every patch is a min/max pair, and the pack doubles vanilla 1..=8.
            for e in gimmick::multiply(slice, 2) {
                if pack.contains(&(start + e.offset)) {
                    assert_eq!(e.new, e.old * 2);
                    assert!((1..=8).contains(&e.old), "vanilla yield {} out of range", e.old);
                }
            }
            offsets += pack.len();
        }
        println!(
            "{name:<20} {:>3} records, {:>4} blocks, {:>4} u64 offsets — exact match",
            family.len(),
            offsets / 2,
            offsets
        );
        grand_offsets += offsets;
        grand_records += family.len();
    }
    println!(
        "total: {grand_records} records, {} blocks, {grand_offsets} u64 offsets",
        grand_offsets / 2
    );
    assert_eq!(grand_offsets, 1174, "pack should carry 1174 scalar patches");
    assert_eq!(grand_offsets / 2, 587, "587 output blocks");
    assert_eq!(starts.len(), 275, "275 gather records");
}

// ---------------------------------------------------------------------------
// The accessor census and the tables resolved through it.
//
// These three touch `desert_core::gimmick` and nothing else - two of the
// tables they name are `desert-dispatch`'s, none of them is the looter's - so
// they live here rather than in `desert-looter/tests/game_exe.rs`, where they
// were first written. `just test-game` runs both files.
// ---------------------------------------------------------------------------

/// The two tables that are reachable only through the **indirect** accessor
/// template (`mov r8,[rip+cell]`, the cell holding the name's VA), resolved by
/// `gimmick::resolve_manager_slot_based`. RVAs from the offline census in
/// `docs/findings-dispatch-2026-09-10.md`; `FactionNode` is the one
/// `desert-dispatch` walks.
const PTR_TEMPLATE_SLOTS: &[(&[u8], usize)] =
    &[(gimmick::FACTION_NODE_TABLE, 0x6C30308), (b"Skill", 0x6C2E330)];

#[test]
#[ignore]
fn indirect_template_tables_resolve_uniquely() {
    let file = std::fs::read(EXE).expect("game exe present");
    let h = pe::parse(&file).expect("PE32+ headers");
    let img = pe::file_to_image(&file).expect("image layout");
    let base = h.image_base as usize;

    for (table, want) in PTR_TEMPLATE_SLOTS {
        let name = String::from_utf8_lossy(table);
        match gimmick::resolve_manager_slot_based(&img, base, table) {
            Ok(slot) => {
                println!("{name:<12} manager slot = +0x{slot:X}");
                assert_eq!(slot, *want, "{name}");
            }
            Err(e) => panic!("{name}: {e}"),
        }
        // These two are exactly the ones the direct-template resolver cannot
        // see; if that ever changes, the second template stopped being needed.
        assert!(
            gimmick::resolve_manager_slot(&img, table).is_err(),
            "{name} is reachable through the direct template after all"
        );
    }

    // And the new resolver is a superset: every table the old one finds, it
    // finds too, with the same answer.
    for table in [gimmick::GIMMICK_TABLE, gimmick::ITEM_TABLE] {
        assert_eq!(
            gimmick::resolve_manager_slot_based(&img, base, table),
            gimmick::resolve_manager_slot(&img, table),
            "{}",
            String::from_utf8_lossy(table)
        );
    }
}

/// The accessor template in all four encodings the compiler emitted, with the
/// copy count each has in build 25246367. `desert_core::gimmick` carries the
/// same four privately; they are spelled out again here on purpose, so this
/// test is an oracle for that scan rather than a mirror of it.
///
/// `45 33 C9` and `45 31 C9` are the same `xor r9d,r9d`; `lea r8,[rip+name]`
/// names the table directly and `mov r8,[rip+cell]` through a pointer cell
/// holding the name's VA.
const ACCESSOR_ENCODINGS: &[(&str, &str, usize)] = &[
    ("45 33 C9 + lea r8", "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8", 98),
    ("45 33 C9 + mov r8", "45 33 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8", 13),
    ("45 31 C9 + lea r8", "45 31 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8", 31),
    ("45 31 C9 + mov r8", "45 31 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8", 7),
];

/// The number of static-info types `docs/reference-internals.md` section 19.2
/// enumerates. The four encodings above sum to exactly this, which is what makes
/// the census provably complete: one accessor per type, no fifth encoding left
/// to find. Scanning only the first of the four reached 111 of them.
const STATIC_INFO_TYPES: usize = 149;

/// The call to the record loader, in its two encodings of `mov rcx,rbx`.
///
/// Unlike [`ACCESSOR_ENCODINGS`], these two do **not** have to sum to
/// [`STATIC_INFO_TYPES`] and never did: the template only fixes which arguments
/// reach the call, not how the compiler schedules the instructions that set
/// them up, so a copy whose run-up was ordered differently is simply not
/// matched. 118 of the 149 accessors are covered here on build 25246367 and 120
/// were on 25116796 — the `48 89 D9` encoding lost two sites, all of its
/// remaining 28 living in the cold-code region above `+0x8000000`.
///
/// **That drop was investigated, not waved through** (2026-09-11, build
/// 25246367). It is instruction scheduling, not a deleted code path: all 149
/// accessors still make an `E8` call to a function opening with
/// [`gimmick::LOADER_PROLOGUE`] within `B_WINDOW` of the accessor site, the 31
/// uncovered ones included. Nothing here is load-bearing for the gatherer
/// either — `docs/reference-internals.md` section 19.6 rests the hook's
/// completeness on the record deserializer having exactly **one** reference in
/// the whole image, not on how many accessors this byte template happens to
/// match, and that single reference still holds on this build (one `call` at
/// `+0x385D9F`, inside the hooked loader), as does the count of nine code
/// references to the `"gimmickinfo"` string. So this stays what it always was:
/// a canary that the accessor template still looks the way the resolvers
/// assume, whose number moves when the compiler reshuffles and means something
/// only when it moves *a lot*.
const LOADER_CALL_ENCODINGS: &[(&str, &str, usize)] = &[
    ("48 8B CB", "4C 8D 4C 24 30 44 0F B7 C7 48 8D 54 24 70 48 8B CB E8", 90),
    ("48 89 D9", "4C 8D 4C 24 30 44 0F B7 C7 48 8D 54 24 70 48 89 D9 E8", 28),
];

/// `(table, manager slot RVA, record loader RVA)` for every table the workspace
/// resolves by name, one per accessor encoding and then some. `dropsetinfo` is
/// the dispatch-mission reward table and a `45 31 C9 + lea` copy — the encoding
/// the scan was blind to before 2026-09-10.
const TABLES: &[(&[u8], usize, usize)] = &[
    (gimmick::GIMMICK_TABLE, 0x6C2E308, 0x385CD0),
    (gimmick::ITEM_TABLE, 0x6C2E2E8, 0x385100),
    (gimmick::FACTION_NODE_TABLE, 0x6C30308, 0x3C1F30),
    (b"Skill", 0x6C2E330, 0x3871C0),
    (gimmick::DROPSET_TABLE, 0x6C328A8, 0x437E70),
];

#[test]
#[ignore]
fn accessor_census_covers_every_static_info_type() {
    let file = std::fs::read(EXE).expect("game exe present");
    let img = pe::file_to_image(&file).expect("image layout");

    let mut total = 0usize;
    for (label, text, want) in ACCESSOR_ENCODINGS {
        let p = Pattern::parse(text).expect("accessor pattern");
        let n = p.find_all(&img, 4096).len();
        println!("{label:<20} = {n:3} copies");
        assert_eq!(n, *want, "{label}");
        total += n;
    }
    assert_eq!(total, STATIC_INFO_TYPES, "the census no longer covers every type");

    let mut calls = 0usize;
    for (label, text, want) in LOADER_CALL_ENCODINGS {
        let p = Pattern::parse(text).expect("loader-call pattern");
        let n = p.find_all(&img, 4096).len();
        println!("loader call {label:<8} = {n:3} sites");
        assert_eq!(n, *want, "{label}");
        calls += n;
    }
    assert_eq!(calls, 118);
}

#[test]
#[ignore]
fn every_named_table_resolves_to_its_documented_slot_and_loader() {
    let file = std::fs::read(EXE).expect("game exe present");
    let h = pe::parse(&file).expect("PE32+ headers");
    let img = pe::file_to_image(&file).expect("image layout");
    let base = h.image_base as usize;

    // The name that reaches the reward table occurs once, which is what lets an
    // accessor be identified by it at all.
    let once = img.windows(12).filter(|w| *w == b"dropsetinfo\0").count();
    assert_eq!(once, 1, "\"dropsetinfo\" is no longer a unique string");

    for (table, slot, loader) in TABLES {
        let name = String::from_utf8_lossy(table);
        assert_eq!(
            gimmick::resolve_manager_slot_based(&img, base, table),
            Ok(*slot),
            "{name} manager slot"
        );
        assert_eq!(
            gimmick::resolve_record_loader_for(&img, base, table),
            Ok(base + *loader),
            "{name} record loader"
        );
        // Whatever it resolved to is a loader: same prologue every time, which
        // is also the check the gatherer makes before patching one.
        assert_eq!(
            img.get(*loader..*loader + gimmick::LOADER_STOLEN),
            Some(&gimmick::LOADER_PROLOGUE[..]),
            "{name} loader prologue"
        );
        println!("{name:<12} slot = +0x{slot:X}  loader = +0x{loader:X}");
    }

    // The gatherer's table-less entry point is unchanged by the widening.
    assert_eq!(gimmick::resolve_record_loader(&img, base), Ok(base + 0x385CD0));
    // And so are the two slots Desert Looter resolves without an image base.
    assert_eq!(gimmick::resolve_manager_slot(&img, gimmick::GIMMICK_TABLE), Ok(0x6C2E308));
    assert_eq!(gimmick::resolve_manager_slot(&img, gimmick::ITEM_TABLE), Ok(0x6C2E2E8));
    assert_eq!(gimmick::resolve_manager_slot(&img, gimmick::DROPSET_TABLE), Ok(0x6C328A8));
}
