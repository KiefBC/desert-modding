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

/// RVA of `FUN_1403856b0` on Steam build 25116796.
const LOADER_RVA: usize = 0x3856b0;

/// RVA of the `mov r8d,1` inside `FUN_142a73c20` on Steam build 25116796
/// (`docs/reference-internals.md` section 17.4).
const CATCH_RVA: usize = 0x2a74151;

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
