//! Runs only on request:
//! `cargo test --release --target x86_64-unknown-linux-gnu -p desert-core -- --ignored --nocapture`
//!
//! Needs the Steam install at the path below. A clean `gimmickinfo` body is
//! needed by the pack cross-check alone, and *any* build's copy of one does:
//! that test compares record-relative coordinates and so is pinned to no game
//! build at all, unlike the RVA constants further down. The body is DMM's
//! artifact, not this repo's, so when it is absent that one test prints a skip
//! notice and passes without proving anything, rather than failing for a missing
//! file nobody deleted. Every other test here needs only the exe.
//! Same shape as `desert-harvest/tests/game_exe.rs`: these are the tests to run
//! after a game update, because they are what break first.

use std::collections::{BTreeMap, BTreeSet};

use desert_core::creature::{CATCH_BYTES, CATCH_SITE, CATCH_STOLEN};
use desert_core::gimmick::{self, LOADER_PROLOGUE, LOADER_STOLEN};
use desert_core::pattern::{Found, Pattern};
use desert_core::pe;

const EXE: &str = "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe";
/// The plugin's own copy of the clean table body, written beside its log by one
/// launch with `[Harvest] DumpTable=1` and `DryRun=1` (desert-tooling 0.9.0, under
/// `[Gatherer]` before 0.10.0).
const TABLE_DUMP: &str =
    "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/DesertTooling.gimmickinfo.bin";
/// DMM's extracted copy of the same bytes, which DMM deletes and restores at will.
const DMM_TABLE: &str = "/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin";

/// The clean `gimmickinfo` body, by the same three-step rule the tools and the
/// justfile apply (`tools/src/paths.rs::dmm_table`): `CD_DMM_TABLE` if set, else
/// the plugin's dump if it exists, else DMM's backup. Until 2026-09-24 this was
/// the DMM path alone, and with that file gone both fixture tests below passed
/// by skipping for two weeks while their banners went unread in a green run -
/// which is exactly the failure the banners describe. A `desert-core` test
/// cannot depend on `desert_tools` (a separate workspace), so the rule is
/// repeated here rather than imported; keep the two in step.
fn table_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("CD_DMM_TABLE") {
        return std::path::PathBuf::from(p);
    }
    let dump = std::path::PathBuf::from(TABLE_DUMP);
    if dump.is_file() {
        return dump;
    }
    std::path::PathBuf::from(DMM_TABLE)
}
const PACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../desert-gatherer-dmm");
/// The pack's own generated record of what it was built against. Only the build
/// number is read from it, and only to print it: a hard-coded `22_325_302` table
/// length and a hard-coded build number in an assertion message used to live
/// here, and both were a second copy of a fact the pack states about itself.
const VERIFICATION: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/../desert-gatherer-dmm/VERIFICATION.txt");
/// Steam's own record of the build that is installed, which is necessarily the
/// fixture's build as well: both the plugin's dump and DMM's extract come out of
/// the *installed* game, so no other build's body can turn up. Printed beside the
/// pack's build every run as context and nothing more - the cross-check compares
/// record-relative coordinates and does not care whether the two agree - but
/// when it does fail, whether they diverged is the first thing a reader needs.
const APPMANIFEST: &str = "/mnt/f/SteamLibrary/steamapps/appmanifest_3321460.acf";

/// RVA of the `gimmickinfo` record loader on Steam build 26361232, the same as
/// on 26356046 and on 26345091, where it was re-derived (was
/// `0x38BA40`, `FUN_14038ba40`, on 25477059, `0x385CD0` on 25246367 and
/// `FUN_1403856b0` on the build before that; the function is the same, the
/// address is not).
const LOADER_RVA: usize = 0x3D89A0;

/// RVAs of the two `*InfoManager` global pointer slots on Steam build 26361232,
/// the same as on 26356046 (were `0x71B6C20` and `0x71B6C48` on 26345091, `0x6D69AA8` and `0x6D69AC8`
/// on 25477059, `0x6C2E2E8` and `0x6C2E308` on 25246367).
/// These are the oracle for `gimmick::resolve_manager_slot`, and are the values
/// `desert-harvest/src/looter/tables.rs` used to carry as hard-coded constants before
/// the slots were resolved by content at startup.
const ITEM_INFO_SLOT_RVA: usize = 0x71B7BD0;
const GIMMICK_INFO_SLOT_RVA: usize = 0x71B7BF8;

/// RVA of the `mov r8d,1` inside the catch-count function on Steam build
/// 26361232, file offset `0x2DD5B28` (was `0x2DD6778`, file `0x2DD5B78`, on
/// 26356046, `0x2DD6448`, file `0x2DD5848`, on 26345091, `0x2B42FB1` on 25477059, `0x2A75891`
/// on 25246367, inside `FUN_142a73c20` on the build before that;
/// `docs/reference-internals.md` section 17.4).
const CATCH_RVA: usize = 0x2DD6728;

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

    // The stream layout the hook reads, parsed from the same accessor, both
    // through the full scan and through the lea-only scan the gatherer makes.
    let stream = gimmick::resolve_stream_layout(&img, base);
    println!("stream layout = {stream:X?}");
    assert_eq!(stream, Ok(STREAM_LAYOUT));
    let sites = gimmick::AccessorSites::scan(&img, None).expect("patterns parse");
    assert_eq!(sites.stream_layout(&img, gimmick::GIMMICK_TABLE), Ok(STREAM_LAYOUT));
}

/// The looter's two info-manager slots, resolved by content instead of by
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

/// The gatherer's second hook: the catch count for insects and fish.
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

/// The `[slot]` line the looter logs, printed here for the same reason.
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
/// machine-written by `dmm-rebase` (`tools/src/bin/dmm-rebase.rs`), so every change object
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
/// One pack family: which **record-relative** offsets it patches in each record.
type Family = BTreeMap<Record, BTreeSet<usize>>;

/// `(record key, record name) -> the record-relative offsets that family patches
/// in it`. `None` if a change object does not have its fields in that documented
/// order.
///
/// `record_rel_offset`, not `offset`: the absolute one is build-specific, while
/// `offset - record_rel_offset` is the same for every change in a record (key
/// 1005631: 12517954-1758 == 12517962-1766 == 12516196), so the relative one is
/// a coordinate that survives a game update. It is counted from the record's
/// `u32` key - exactly where `locate_records` points, and exactly where
/// `gimmick::multiply` counts its own `Edit::offset` from - so the two sets
/// compare with no arithmetic and no absolute address anywhere in the test.
fn read_family(text: &str) -> Option<Family> {
    let mut out: Family = BTreeMap::new();
    let mut cur = 0usize;
    // `offset` is read only to step onto this change object: the documented field
    // order is `offset`, `record_key`, `record_rel_offset`, `entry`, so the three
    // wanted fields all lie after it and before the next object's `offset`.
    while let Some((_, at)) = number_after(text, "\"offset\":", cur) {
        let (key, at) = number_after(text, "\"record_key\":", at)?;
        let (rel, at) = number_after(text, "\"record_rel_offset\":", at)?;
        let (entry, at) = string_after(text, "\"entry\":", at)?;
        out.entry((key as u32, entry)).or_default().insert(rel as usize);
        cur = at;
    }
    Some(out)
}

/// Where a located record sits: its first header byte, and its `u32` key
/// (which the pack's `record_rel_offset` counts from) relative to that.
#[derive(Debug, Clone, Copy)]
struct Located {
    start: usize,
    key_off: usize,
}

/// Find every wanted record in one pass over the table, the same rule as
/// `dmm-rebase`'s `locate_records`: the table's header shape is probed off its
/// first record (`gimmick::detect_header_shape`), every offset is parsed
/// strictly in that shape only, and each wanted `(key, name)` must hit exactly
/// once. Scanning in both shapes would not do: a strict `KeyFirst` parse finds
/// 36248 false headers in the 26361232 `NameFirst` body.
///
/// Every record that is not found exactly once is reported, not just the first:
/// on a build newer than the pack's, a record legitimately renamed, removed or
/// duplicated is the finding, and the caller turns the whole list into the
/// answer to "is the published pack still safe for DMM users on this build".
fn locate_records(
    table: &[u8],
    wanted: &BTreeSet<Record>,
) -> Result<BTreeMap<Record, Located>, String> {
    let shape = gimmick::detect_header_shape(table)
        .ok_or("the table's first record validates as neither header shape (or as both)")?;
    let mut hits: BTreeMap<Record, Vec<Located>> = BTreeMap::new();
    for p in 0..table.len() {
        let Some(rest) = table.get(p..) else { break };
        if let Some(h) = gimmick::parse_header_as(rest, shape) {
            let key_off = match shape {
                gimmick::HeaderShape::KeyFirst => 0,
                gimmick::HeaderShape::NameFirst => 4 + h.name.len(),
            };
            let k = (h.key, h.name);
            if wanted.contains(&k) {
                hits.entry(k).or_default().push(Located { start: p, key_off });
            }
        }
    }
    let mut out = BTreeMap::new();
    let mut bad: Vec<String> = Vec::new();
    for k in wanted {
        match hits.get(k).map(Vec::as_slice) {
            Some([one]) => {
                out.insert(k.clone(), *one);
            }
            Some(many) => bad.push(format!("  key {} {:?}: {} hits, expected 1", k.0, k.1, many.len())),
            None => bad.push(format!("  key {} {:?}: not in this table", k.0, k.1)),
        }
    }
    if !bad.is_empty() {
        return Err(format!(
            "{} of the {} records the pack names are not in this table exactly once:\n{}",
            bad.len(),
            wanted.len(),
            bad.join("\n")
        ));
    }
    Ok(out)
}

/// One shared scan answers exactly what a scan per call answers, on the real
/// exe.
///
/// `AccessorSites` exists because finding the 154 template copies does not
/// depend on which table is being asked about, so `desert-dispatch` can resolve
/// `FactionNode` and `dropsetinfo` from four passes over the 363 MB image
/// instead of eight - measured 2.0x, 1.0 s down to 0.49 s. The synthetic-image
/// test in `gimmick.rs` proves the equivalence on a five-site fixture; this
/// proves it on the 154 real ones, which is where a filtering mistake that the
/// fixture is too small to expose would show up.
#[test]
#[ignore]
fn one_shared_scan_answers_what_a_scan_per_call_answers() {
    let file = std::fs::read(EXE).expect("game exe present");
    let img = pe::file_to_image(&file).expect("image layout");
    let base = pe::parse(&file).expect("PE32+ headers").image_base as usize;
    let sites =
        gimmick::AccessorSites::scan(&img, Some(base)).expect("the patterns are well formed");

    for table in [
        gimmick::GIMMICK_TABLE,
        gimmick::ITEM_TABLE,
        gimmick::DROPSET_TABLE,
        gimmick::FACTION_NODE_TABLE,
        b"Skill",
    ] {
        let name = String::from_utf8_lossy(table);
        assert_eq!(
            sites.manager_slot(&img, table),
            gimmick::resolve_manager_slot_based(&img, base, table),
            "manager slot disagrees for {name}"
        );
        assert_eq!(
            sites.record_loader(&img, base, table),
            gimmick::resolve_record_loader_for(&img, base, table),
            "record loader disagrees for {name}"
        );
        println!("{name:<12} agrees: slot {:X?}", sites.manager_slot(&img, table));
    }

    // The base-less scan the looter makes: two encodings searched instead of
    // four, and the same answers for the two tables it wants.
    let lea = gimmick::AccessorSites::scan(&img, None).expect("the patterns are well formed");
    for table in [gimmick::GIMMICK_TABLE, gimmick::ITEM_TABLE] {
        assert_eq!(
            lea.manager_slot(&img, table),
            gimmick::resolve_manager_slot(&img, table),
            "base-less manager slot disagrees for {}",
            String::from_utf8_lossy(table)
        );
    }
}

/// The only oracle that `gimmick::multiply` reproduces the released DMM pack's
/// published edits: all 1174 of them, by offset and by value, recomputed from a
/// clean `gimmickinfo` body.
///
/// Nothing in here is pinned to a game build. Records are found by key and name
/// rather than by address, and the offsets compared are the pack's
/// `record_rel_offset` against `multiply`'s own record-relative `Edit::offset`,
/// so any build's clean table answers the question and the pack never needs
/// rebasing for this test's sake. That is the whole point: the body the pack was
/// built against (build 25116796) can never be obtained again - no backup
/// survives anywhere and DMM only ever extracts from the installed build - so
/// the `table.len() == 22_325_302` this used to assert, and the SHA-256 pin that
/// briefly replaced it, were both checks against a dead artifact that would
/// have failed on every future fixture with nothing actually wrong.
///
/// The two halves fail for different reasons and say which: `locate_records`
/// failing means the published pack is stale for some record on this build,
/// while an offset or value mismatch means `multiply` and the pack disagree
/// about the record layout they are both reading.
#[test]
#[ignore]
fn multiply_reproduces_the_dmm_pack_edits() {
    // A repo file, generated by `dmm-rebase` (tools/). If it cannot be read
    // or does not name what it should, the pack is broken - that is a real
    // failure, not a missing-fixture skip.
    let verification = std::fs::read_to_string(VERIFICATION)
        .expect("desert-gatherer-dmm/VERIFICATION.txt readable");
    let (pack_build, _) = string_after(&verification, "\"game_build\":", 0)
        .expect("VERIFICATION.txt names game_build");

    // Context, not a check. The comparison below is build-independent, so these
    // two are allowed to differ and nothing gates on them. They are printed
    // unconditionally because when something here does fail, whether the builds
    // diverged is what separates "rebase the pack" from "a real regression in
    // `multiply`". The fixture's build is necessarily the installed one: DMM
    // extracts the clean table from the installed game and from nowhere else.
    let installed = std::fs::read_to_string(APPMANIFEST)
        .ok()
        .and_then(|acf| string_after(&acf, "\"buildid\"", 0).map(|(v, _)| v))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("pack built against game build {pack_build}; installed game build {installed}");

    // The clean body comes from a game launch (the plugin's dump) or from DMM,
    // and nothing in this repo can create one offline. Its absence is therefore
    // not a regression and must not read as one - but a quiet `return` would
    // look exactly like a pass in `--nocapture` output, so say plainly that
    // nothing was proved.
    let bar = "=".repeat(76);
    let table_path = table_path();
    let Ok(table) = std::fs::read(&table_path) else {
        println!("\n{bar}");
        println!("!! SKIPPED: THE DMM PACK ORACLE DID NOT RUN. THIS TEST PROVED NOTHING. !!");
        println!("{bar}");
        println!("missing fixture: {}", table_path.display());
        println!();
        println!("`gimmick::multiply` was NOT checked against the pack's 1174 published");
        println!("scalar edits on this run. Read this as a hole in coverage, not a pass.");
        println!();
        println!("To get one: set `[Harvest] DumpTable=1` and `DryRun=1` in DesertTooling.ini");
        println!("(the menu's Debug tab), launch the game once, and the plugin writes");
        println!("{TABLE_DUMP}");
        println!("about ten seconds in. Then set both back to 0. CD_DMM_TABLE overrides the");
        println!("path; DMM's backup copy at {DMM_TABLE} is the fallback.");
        println!();
        println!("ANY build's clean body will do, the installed build included. This test");
        println!("compares record-relative coordinates and never an address, so it needs");
        println!("no particular build's table and the pack needs no rebasing for it.");
        println!("{bar}\n");
        return;
    };
    println!("table: {} ({} bytes)", table_path.display(), table.len());

    // No digest and no length assertion: there is nothing left to pin either to.
    // All that is required of the table is that it be a clean one, and a patched
    // one fails loudly below on `e.new == e.old * 2` rather than passing quietly.
    println!("table: {} bytes", table.len());

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
    let starts = match locate_records(&table, &wanted) {
        Ok(s) => s,
        // Not a fixture problem and not a `multiply` problem: the pack names a
        // record this table does not hold exactly once, which means the
        // *published* pack is stale for that record - DMM would fail to locate
        // it, or worse locate the wrong one. This failing is the answer to "is
        // the released pack still safe on this build", so the message names
        // every record involved rather than only the first.
        Err(e) => panic!(
            "\n{bar}\nthe published DMM pack is stale against this table.\n{e}\n\
             Records the pack patches were renamed, removed or duplicated between\n\
             build {pack_build} (the pack's) and build {installed} (this table's).\n\
             The pack needs rebasing with `just dmm-rebase` before it is\n\
             safe for DMM users on that build. `gimmick::multiply` is not implicated.\n{bar}"
        ),
    };
    let mut sorted: Vec<usize> = starts.values().map(|l| l.start).collect();
    sorted.sort_unstable();
    println!("records located: {} (all unique)", starts.len());

    let mut grand_offsets = 0usize;
    let mut grand_records = 0usize;
    for (name, family) in &families {
        let mut offsets = 0usize;
        for (rec, pack) in family {
            let Located { start, key_off } = *starts.get(rec).expect("record located");
            // The record ends where the next known record begins; 64 KiB is the
            // fallback for the last one (the largest gather record is far
            // smaller than that).
            let next = sorted.iter().copied().find(|&s| s > start).unwrap_or(usize::MAX);
            let end = next.min(start + 65536).min(table.len());
            let slice = table.get(start..end).expect("record slice");

            // The pack counts from the record's `u32` key, `multiply` from the
            // slice start (the record's first header byte); on a `KeyFirst`
            // table the two are the same byte, on a `NameFirst` table the key
            // is `4 + name_len` in. `last` trims the edits `multiply` finds
            // past the end of what this family published - the slice can run on
            // into the next record's tail when `sorted` has no closer bound.
            let last = *pack.iter().next_back().expect("family patches this record");
            let got: BTreeSet<usize> = gimmick::multiply(slice, 2)
                .iter()
                .filter_map(|e| e.offset.checked_sub(key_off))
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
                if e.offset.checked_sub(key_off).is_some_and(|o| pack.contains(&o)) {
                    assert_eq!(e.new, e.old * 2);
                    assert!((1..=8).contains(&e.old), "vanilla yield {} out of range", e.old);
                }
            }
            offsets += pack.len();
        }
        println!(
            "{name:<20} {:>3} records, {:>4} blocks, {:>4} u64 offsets: exact match",
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

/// Where the item id is, against the table itself.
///
/// `ITEM_AT` and `ITEM_TAIL_AT` were 4 too high until 2026-09-12 - `+5` and
/// `+64`, the two zero pads - so `OutputBlock::item` read 0 for every block in
/// the game, `desert_harvest::gatherer::remember` stored 0, and `hook::reapply`'s
/// "is this the block I remembered" cross-check could never match. `multiply`
/// writes only min/max and so was never affected, which is why the pack oracle
/// above passed throughout and nothing else noticed.
///
/// Nothing synthetic can catch that class of mistake: the unit tests built their
/// blocks with the same wrong offsets the code read them at. This is the test
/// that needs the real bytes, and it asserts two things - `peony_01`'s first
/// block reads back the item id `docs/reference-internals.md` section 13.1
/// records for it, and no block anywhere in the body has an item id of 0, which
/// is what every single one of them had before the fix.
///
/// Same fixture and the same skip contract as the pack oracle: the clean body is
/// DMM's artifact, so its absence is a hole in coverage and says so, not a
/// failure.
#[test]
#[ignore]
fn output_blocks_read_the_item_id_the_table_actually_carries() {
    let bar = "=".repeat(76);
    let table_path = table_path();
    let Ok(table) = std::fs::read(&table_path) else {
        println!("\n{bar}");
        println!("!! SKIPPED: THE ITEM-OFFSET CHECK DID NOT RUN. IT PROVED NOTHING. !!");
        println!("{bar}");
        println!("missing fixture: {}", table_path.display());
        println!();
        println!("`gimmick::ITEM_AT` / `ITEM_TAIL_AT` were NOT checked against real table");
        println!("bytes on this run. They were wrong by 4 once and only the fixture can");
        println!("say so. Read this as a hole in coverage, not a pass.");
        println!();
        println!("`multiply_reproduces_the_dmm_pack_edits` above prints how to get a clean");
        println!("body; the same file serves both tests and any build's copy will do.");
        println!("{bar}\n");
        return;
    };
    println!("table: {} bytes", table.len());

    // Locate one record the same way the pack oracle does, and bound the slice
    // with the next record so the blocks read are certainly this record's.
    let peony: Record = (17_020_006, "peony_01".to_owned());
    let wanted: BTreeSet<Record> = [peony.clone()].into_iter().collect();
    let start = locate_records(&table, &wanted)
        .expect("peony_01 is in this table exactly once")
        .get(&peony)
        .expect("located")
        .start;
    let end = (start + 65536).min(table.len());
    let slice = table.get(start..end).expect("record slice");
    let first = *gimmick::output_blocks(slice).first().expect("peony_01 has output blocks");
    println!(
        "peony_01 at {start}: first block +{} item {} min {} max {}",
        first.offset, first.item, first.min, first.max
    );
    // Section 13.1 learned 757006 from the game itself, through the pickup
    // event - an origin entirely outside this table - so it is a real oracle and
    // not a second reading of the same bytes.
    assert_eq!(first.item, 757_006, "peony_01's first output block is Peony");
    assert_eq!((first.min, first.max), (4, 7), "and its vanilla yield is 4..=7");

    // The whole body, which is the part no synthetic block can stand in for: on
    // the old constants every one of these would have been 0.
    let all = gimmick::output_blocks(&table);
    let lists = gimmick::output_lists(&table).len();
    println!("whole body: {lists} output lists, {} blocks", all.len());
    assert!(all.len() > 800, "expected the whole body's blocks, got {}", all.len());
    let zero = all.iter().filter(|b| b.item == 0).count();
    assert_eq!(zero, 0, "{zero} of {} blocks have item id 0", all.len());
    // The two copies agree and the two pads are zero, which is the layout claim
    // the constants encode, stated over every block rather than over one.
    for b in &all {
        let head = u32_at(&table, b.offset + gimmick::ITEM_AT).expect("item id in the table");
        let tail = u32_at(&table, b.offset + gimmick::ITEM_TAIL_AT).expect("item echo");
        assert_eq!((head, tail), (b.item, b.item), "block at {}", b.offset);
        assert_eq!(u32_at(&table, b.offset + gimmick::PAD_AT), Some(0), "pad at {}", b.offset);
        assert_eq!(
            u32_at(&table, b.offset + gimmick::PAD_TAIL_AT),
            Some(0),
            "tail pad at {}",
            b.offset
        );
    }
    println!("all {} blocks: item at +1 == item at +60 != 0, both pads zero", all.len());
}

/// The table body the installed build dumps is a `NameFirst` table: its first
/// record validates as that shape and no other, a strict scan in that shape
/// finds 14210 headers with 14210 distinct keys (one per record the game's
/// manager counts), and the well and the 26361232 example record sit where the
/// dump put them. On 25477059 and before the same body was `KeyFirst`; the
/// rule for which shape a table uses is what the gatherer latches in game.
///
/// Same skip contract as the pack oracle: no dump, no proof, and it says so.
#[test]
#[ignore]
fn the_table_body_is_name_first_with_one_header_per_record() {
    let bar = "=".repeat(76);
    let table_path = table_path();
    let Ok(table) = std::fs::read(&table_path) else {
        println!("\n{bar}");
        println!("!! SKIPPED: THE HEADER-SHAPE CHECK DID NOT RUN. IT PROVED NOTHING. !!");
        println!("missing fixture: {} (DMM's backup may be gone; dump one from the game)", table_path.display());
        println!("{bar}\n");
        return;
    };
    let shape = gimmick::detect_header_shape(&table);
    println!("table: {} bytes, first record {shape:?}", table.len());
    assert_eq!(shape, Some(gimmick::HeaderShape::NameFirst));
    let headers: Vec<(usize, gimmick::RecordHeader)> = (0..table.len())
        .filter_map(|p| Some((p, gimmick::parse_header_as(table.get(p..)?, gimmick::HeaderShape::NameFirst)?)))
        .collect();
    let keys: BTreeSet<u32> = headers.iter().map(|(_, h)| h.key).collect();
    // `body` counts from the header's own start, which is `p` in the table.
    let flags: BTreeMap<u8, usize> = headers.iter().fold(BTreeMap::new(), |mut m, (p, h)| {
        *m.entry(table[p + h.body - 1]).or_default() += 1;
        m
    });
    println!("NameFirst headers: {} ({} distinct keys), flags {flags:?}", headers.len(), keys.len());
    assert_eq!(headers.len(), 14210);
    assert_eq!(keys.len(), 14210);
    assert_eq!(flags, BTreeMap::from([(0, 11511), (1, 2699)]));
    let (first, h0) = &headers[0];
    assert_eq!((*first, h0.key, h0.name.as_str()), (0, 1003051, "operator_gimmickcomplete_00"));
    let well = headers.iter().find(|(_, h)| h.key == 1001081).expect("the well");
    assert_eq!((well.0, well.1.name.as_str()), (0x33795b, "gimmick_well_0001_parts01"));
    // Every path is rooted; exactly one is not a `/object` path.
    let odd: Vec<&str> = headers
        .iter()
        .filter_map(|(_, h)| h.path.as_deref())
        .filter(|p| !p.starts_with("/object"))
        .collect();
    assert_eq!(odd, ["/character/prefab/6_object/tools/cd_gimmick_item_flowers_01.prefab"]);
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)?.try_into().ok().map(u32::from_le_bytes)
}

// ---------------------------------------------------------------------------
// The accessor census and the tables resolved through it.
//
// These three touch `desert_core::gimmick` and nothing else - two of the
// tables they name are `desert-dispatch`'s, none of them is the looter's - so
// they live here rather than in `desert-harvest/tests/game_exe.rs`, where they
// were first written. `just test-game` runs both files.
// ---------------------------------------------------------------------------

/// The two tables that are reachable only through the **indirect** accessor
/// template (`mov r8,[rip+cell]`, the cell holding the name's VA), resolved by
/// `gimmick::resolve_manager_slot_based`. `FactionNode` is the one
/// `desert-dispatch` walks. RVAs on builds 26356046 and 26361232 (`0x71B96E8` and
/// `0x71B6C80` on 26345091); on 25477059 they were
/// `0x6D6BE70` and `0x6D69AF0`, and on 25246367 `0x6C30308` and `0x6C2E330`,
/// the values the offline census in `docs/findings/2026-09-10-dispatch.md`
/// records.
const PTR_TEMPLATE_SLOTS: &[(&[u8], usize)] =
    &[(gimmick::FACTION_NODE_TABLE, 0x71BA698), (b"Skill", 0x71B7C30)];

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

/// The accessor head in all four encodings the compiler emitted, with the
/// copy count each has in builds 26345091, 26356046 and 26361232 (98, 13, 31 and 7 on
/// 25477059).
/// `desert_core::gimmick` carries the same four privately; they are spelled
/// out again here on purpose, so this test is an oracle for that scan rather
/// than a mirror of it.
///
/// `45 33 C9` and `45 31 C9` are the same `xor r9d,r9d`; `lea r8,[rip+name]`
/// names the table directly and `mov r8,[rip+cell]` through a pointer cell
/// holding the name's VA. The head stops at `mov rdx,[rbx+0x10]`; the tail
/// after it is `lea rcx,[r11+0x10]` on every copy of this build.
const ACCESSOR_ENCODINGS: &[(&str, &str, usize)] = &[
    ("45 33 C9 + lea r8", "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10", 132),
    ("45 33 C9 + mov r8", "45 33 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10", 20),
    ("45 31 C9 + lea r8", "45 31 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10", 2),
    ("45 31 C9 + mov r8", "45 31 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10", 0),
];

/// The number of accessor copies, one per static-info table, each passing a
/// different name: 149 through 25477059, 154 on 26345091, 26356046 and 26361232.
const ACCESSOR_COPIES: usize = 154;

/// The tail every copy has on 26345091: `lea rcx,[r11+0x10]; call`.
const R11_TAIL: [u8; 5] = [0x49, 0x8D, 0x4B, 0x10, 0xE8];

/// The call to the record loader, counted exe-wide in the shapes 26345091,
/// 26356046 and 26361232 use. On 26345091 the second row read 1 (`tribeinfo`'s,
/// whose accessor was obfuscated code on 26356046, where the row read 0); on
/// 26361232 it reads 2, `characterinfo`'s and `tribeinfo`'s. A canary, not a census: these count every match in the image, not
/// only those inside an accessor, and the template fixes which arguments reach
/// the call, not how the compiler schedules the instructions that set them up.
/// On 25477059 the two old shapes (`movzx r8d,di` inside the run-up and a disp8
/// `lea rdx`) matched 90 and 30; both are 0 now.
const LOADER_CALL_ENCODINGS: &[(&str, &str, usize)] = &[
    ("disp32 + 48 8B CB", "4C 8D 4C 24 30 48 8D 94 24 ?? ?? 00 00 48 8B CB E8", 127),
    ("disp32 + 48 89 D9", "4C 8D 4C 24 30 48 8D 94 24 ?? ?? 00 00 48 89 D9 E8", 2),
    ("mov r8d + disp32 + 48 8B CB", "4C 8D 4C 24 30 44 8B ?? 48 8D 94 24 ?? ?? 00 00 48 8B CB E8", 25),
    ("old movzx + disp8 + 48 8B CB", "4C 8D 4C 24 30 44 0F B7 C7 48 8D 54 24 70 48 8B CB E8", 0),
    ("old movzx + disp8 + 48 89 D9", "4C 8D 4C 24 30 44 0F B7 C7 48 8D 54 24 70 48 89 D9 E8", 0),
];

/// Accessors whose loader call `AccessorSites::census` does not resolve on
/// 26361232, and why. Both have a loader call of the template's shape, but it
/// lands on a `jmp` thunk rather than on a function opening with
/// `LOADER_PROLOGUE`. Neither is a table this workspace reads. Sorted by name.
///
/// On 26356046 `characterinfo` and `tribeinfo` were here too, as `no loader
/// call`: their accessors were obfuscated code with no call of the template's
/// shape at all (`tribeinfo`'s was the one `48 89 D9` call on 26345091, which
/// had only the first three rows). On 26361232 both have the call again, with
/// the `48 89 D9` encoding and no index load.
const NO_LOADER: &[(&str, &str)] = &[
    ("AutoSpawnFilterInfo", "does not open with LOADER_PROLOGUE"),
    ("KeyMap", "does not open with LOADER_PROLOGUE"),
];

/// Names the accessors pass that `tools/static-info-names.txt` (as extracted on
/// 25116796) does not list, even allowing for its `<name>Info` class names and
/// its case: the five tables new on 26345091, and `KeyMap`, whose registry
/// name is `KeyMapSettingListInfo`.
const UNLISTED_NAMES: &[&str] = &[
    "HousingObjectGroupInfo",
    "KeyMap",
    "ManagementNeedsSynergyInfo",
    "NpcNameInfo",
    "managementbuffinfo",
    "statchangeInfo",
];

/// `(table, manager slot RVA, record loader RVA)` for every table the workspace
/// resolves by name, one per accessor encoding and then some. `dropsetinfo` is
/// the dispatch-mission reward table and a `45 33 C9 + lea` copy on this build
/// (`45 31 C9 + lea` on 25477059).
///
/// Values are build 26361232's, the same as 26356046's. The loaders are 26345091's too; its slots were
/// `0x71B6C48`, `0x71B6C20`, `0x71B96E8`, `0x71B6C80`, `0x71BBCC0`,
/// `0x71BF090` and `0x71B6C78`. On 25477059 the rows read, in the same order:
/// `(0x6D69AC8, 0x38BA40)`, `(0x6D69AA8, 0x38AE70)`, `(0x6D6BE70, 0x433880)`,
/// `(0x6D69AF0, 0x38CF30)`, `(0x6D6E428, 0x4A6760)`, `(0x6D71E60, 0x671370)`,
/// `(0x6D69AE8, 0x584C70)`.
const TABLES: &[(&[u8], usize, usize)] = &[
    (gimmick::GIMMICK_TABLE, 0x71B7BF8, 0x3D89A0),
    (gimmick::ITEM_TABLE, 0x71B7BD0, 0x3D78B0),
    (gimmick::FACTION_NODE_TABLE, 0x71BA698, 0x4D4ED0),
    (b"Skill", 0x71B7C30, 0x3D8C60),
    (gimmick::DROPSET_TABLE, 0x71BCC70, 0x5511D0),
    (gimmick::BUFF_TABLE, 0x71C0040, 0x73DE10),
    (gimmick::STATUS_TABLE, 0x71B7C28, 0x3D80F0),
];

/// The `gimmickinfo` accessor on 26361232 (and 26356046 and 26345091): its head, its slot load, and the
/// distance from the head to the loader call's `E8`.
const GIMMICK_HEAD: usize = 0x3DD9A8;
const GIMMICK_SLOT_LOAD: usize = 0x3DD975;
const GIMMICK_LOADER_E8_FROM_HEAD: usize = 0x9C;

/// The stream layout the `gimmickinfo` accessor fills in before its loader
/// call on 26361232, as `gimmick::parse_stream_fill` reads it: buffer `+0x20`,
/// size `+0x28`, cursor `+0x2c`. Through 25477059 it was `(0x10, 0x18, 0x1c)`
/// (`StreamLayout::OLD`), the offsets `desert-harvest`'s hook hard-coded until
/// a dry run on 26361232 read a zero buffer and a zero size there.
const STREAM_LAYOUT: gimmick::StreamLayout = gimmick::StreamLayout::NEW;

fn c_str(img: &[u8], rva: usize) -> Option<&[u8]> {
    let rest = img.get(rva..)?;
    rest.get(..rest.iter().position(|&b| b == 0)?)
}

const NAMES_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tools/static-info-names.txt");

fn listed_names(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore]
fn accessor_census_covers_every_static_info_type() {
    let file = std::fs::read(EXE).expect("game exe present");
    let img = pe::file_to_image(&file).expect("image layout");
    let base = pe::parse(&file).expect("PE32+ headers").image_base as usize;

    // The oracle: the four heads counted with plain patterns, and every hit's
    // tail checked by hand.
    let mut total = 0usize;
    let mut want_counts = [0usize; 4];
    for (i, (label, text, want)) in ACCESSOR_ENCODINGS.iter().enumerate() {
        let p = Pattern::parse(text).expect("accessor pattern");
        let hits = p.find_all(&img, 4096);
        println!("{label:<20} = {:3} copies", hits.len());
        assert_eq!(hits.len(), *want, "{label}");
        for h in &hits {
            assert_eq!(
                img.get(h + gimmick::A_HEAD_LEN..h + gimmick::A_HEAD_LEN + 5),
                Some(&R11_TAIL[..]),
                "{label} at +0x{h:X}: tail"
            );
        }
        want_counts[i] = *want;
        total += hits.len();
    }
    assert_eq!(total, ACCESSOR_COPIES, "the census no longer covers every table");

    // The module's scan agrees, and recognises every tail.
    let sites = gimmick::AccessorSites::scan(&img, Some(base)).expect("patterns parse");
    assert_eq!(sites.counts(), want_counts);
    assert_eq!(sites.rejected(), &[] as &[usize], "heads with an unknown tail");
    let census = sites.census(&img).expect("same image");
    assert_eq!(census.len(), ACCESSOR_COPIES);

    let mut tails: BTreeMap<String, usize> = BTreeMap::new();
    let mut slot_dist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut slot_dist_first: BTreeMap<usize, usize> = BTreeMap::new();
    let mut shapes: BTreeMap<String, usize> = BTreeMap::new();
    let mut call_dist: BTreeMap<usize, usize> = BTreeMap::new();
    let mut no_loader: Vec<(String, String)> = Vec::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut streams: BTreeMap<String, usize> = BTreeMap::new();
    let mut no_stream: Vec<String> = Vec::new();
    for r in &census {
        *tails.entry(format!("{:?}", r.site.tail)).or_default() += 1;
        let name = String::from_utf8_lossy(
            c_str(&img, r.name_rva.expect("every name readable with a base")).expect("NUL"),
        )
        .into_owned();
        assert!(names.insert(name.clone()), "{name} passed by two accessors");

        let (load, _) = r.slot.clone().unwrap_or_else(|e| panic!("{name}: slot: {e}"));
        *slot_dist.entry(r.site.at - load).or_default() += 1;
        if r.encoding == 0 {
            *slot_dist_first.entry(r.site.at - load).or_default() += 1;
        }
        // `cmp edx,[rbx+0x8]` on every copy of this build.
        assert_eq!(img.get(load + 7..load + 10), Some(&[0x3B, 0x53, 0x08][..]), "{name}: cmp");

        match &r.loader {
            Ok((b, call, target)) => {
                *shapes.entry(format!("{:?}/{}/{}", call.index, call.rdx_disp32, call.rcx_89)).or_default() += 1;
                *call_dist.entry(b - r.site.call_at).or_default() += 1;
                assert_eq!(
                    img.get(*target..*target + LOADER_STOLEN),
                    Some(&LOADER_PROLOGUE[..]),
                    "{name}: loader prologue"
                );
            }
            Err(e) => no_loader.push((name.clone(), e.clone())),
        }

        match r.stream {
            Some(l) => *streams.entry(format!("{:X}/{:X}/{:X}", l.buf, l.size, l.cursor)).or_default() += 1,
            None => no_stream.push(name.clone()),
        }

        if name == "gimmickinfo" {
            assert_eq!(r.stream, Some(STREAM_LAYOUT), "gimmickinfo's stream fill");
            assert_eq!(r.site.at, GIMMICK_HEAD);
            assert_eq!(load, GIMMICK_SLOT_LOAD);
            let (b, call, target) = r.loader.clone().expect("gimmickinfo loader call");
            assert_eq!(b + call.call_at - r.site.at, GIMMICK_LOADER_E8_FROM_HEAD);
            assert_eq!(target, LOADER_RVA);
        }
    }
    println!("tails             = {tails:?}");
    println!("slot distance     = {:X?} (45 33 C9 + lea only: {:X?})", slot_dist, slot_dist_first);
    println!("loader-call shape = {shapes:?}");
    println!("call distance     = {call_dist:X?}");
    println!("no loader         = {no_loader:?}");
    no_stream.sort();
    println!("stream fill       = {streams:?} (no fill parsed: {no_stream:?})");

    assert_eq!(tails, BTreeMap::from([("R11Disp8(16)".to_owned(), ACCESSOR_COPIES)]));
    assert_eq!(slot_dist, BTreeMap::from([(0x33, 130), (0x3B, 24)]));
    assert_eq!(slot_dist_first, BTreeMap::from([(0x33, 110), (0x3B, 22)]));
    assert_eq!(
        shapes,
        BTreeMap::from([
            ("Mov/true/false".to_owned(), 25),
            ("None/true/false".to_owned(), 125),
            ("None/true/true".to_owned(), 2),
        ])
    );
    assert_eq!(shapes.values().sum::<usize>(), ACCESSOR_COPIES - NO_LOADER.len());
    // The widest observed distance is what `gimmick::B_WINDOW` is set from.
    let widest = *call_dist.keys().next_back().expect("some loader calls");
    assert!(widest < gimmick::B_WINDOW, "widest 0x{widest:X} vs B_WINDOW");
    assert_eq!(widest, 0x7E);
    // Every site with a resolved loader call fills the stream the same way;
    // the two without one are the only two whose fill is not read.
    assert_eq!(
        streams,
        BTreeMap::from([("20/28/2C".to_owned(), ACCESSOR_COPIES - NO_LOADER.len())])
    );
    assert_eq!(no_stream, NO_LOADER.iter().map(|(n, _)| n.to_string()).collect::<Vec<_>>());
    no_loader.sort();
    assert_eq!(no_loader.len(), NO_LOADER.len(), "{no_loader:?}");
    for ((got_name, got_err), (want_name, want_err)) in no_loader.iter().zip(NO_LOADER) {
        assert_eq!(got_name, want_name);
        assert!(got_err.contains(want_err), "{got_name}: {got_err}");
    }

    // Every name passed is one the registry list carries, as a class name or
    // a lowercase table name, except the tables added since it was extracted.
    let text = std::fs::read_to_string(NAMES_FILE).expect("tools/static-info-names.txt readable");
    let listed: BTreeSet<String> = listed_names(&text).iter().map(|n| n.to_lowercase()).collect();
    let unlisted: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| {
            !listed.contains(&n.to_lowercase()) && !listed.contains(&format!("{n}info").to_lowercase())
        })
        .collect();
    println!("unlisted names    = {unlisted:?}");
    // A subset, not an equality: `tools/static-info-names.txt` is maintained
    // separately and may gain these names, but no name outside this list may
    // turn up unlisted without someone looking at it.
    for n in &unlisted {
        assert!(UNLISTED_NAMES.contains(n), "{n} is passed by an accessor and not in the names list");
    }

    // The exe-wide loader-call canary.
    for (label, text, want) in LOADER_CALL_ENCODINGS {
        let p = Pattern::parse(text).expect("loader-call pattern");
        let n = p.find_all(&img, 4096).len();
        println!("loader call {label:<30} = {n:3} sites");
        assert_eq!(n, *want, "{label}");
    }
}

#[test]
#[ignore]
fn every_named_table_resolves_to_its_documented_slot_and_loader() {
    let file = std::fs::read(EXE).expect("game exe present");
    let h = pe::parse(&file).expect("PE32+ headers");
    let img = pe::file_to_image(&file).expect("image layout");
    let base = h.image_base as usize;

    // Each of these names occurs exactly once as a NUL-delimited literal, which
    // is what lets an accessor be identified by the name it passes at all. The
    // reward table is here because it was the first one resolved this way; the
    // buff and stat tables because the census in `desert-dispatch` now is.
    // NUL on both sides: on 26345091 `"buffinfo\0"` is also the tail of the
    // new `"managementbuffinfo\0"`, which is a different literal.
    for want in [&b"\0dropsetinfo\0"[..], b"\0buffinfo\0", b"\0statusinfo\0"] {
        let once = img.windows(want.len()).filter(|w| *w == want).count();
        assert_eq!(once, 1, "{:?} is no longer a unique string", String::from_utf8_lossy(want));
    }

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
    // Build 26361232 values, the same as 26356046's; on 26345091 the dropset slot was 0x71BBCC0, and on
    // 25477059 they were 0x38BA40, 0x6D69AC8, 0x6D69AA8 and 0x6D6E428.
    assert_eq!(gimmick::resolve_record_loader(&img, base), Ok(base + LOADER_RVA));
    // And so are the two slots the looter resolves without an image base.
    assert_eq!(
        gimmick::resolve_manager_slot(&img, gimmick::GIMMICK_TABLE),
        Ok(GIMMICK_INFO_SLOT_RVA)
    );
    assert_eq!(gimmick::resolve_manager_slot(&img, gimmick::ITEM_TABLE), Ok(ITEM_INFO_SLOT_RVA));
    assert_eq!(gimmick::resolve_manager_slot(&img, gimmick::DROPSET_TABLE), Ok(0x71BCC70));
}
