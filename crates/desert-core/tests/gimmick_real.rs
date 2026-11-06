//! Runs only on request:
//! `cargo test --release --target x86_64-unknown-linux-gnu -p desert-core -- --ignored --nocapture`
//!
//! Needs the Steam install at the path below, and for the four table tests
//! the game's own `gimmickinfo` table as `just extract-tables` writes it into
//! `analysis/tables/` (or `CD_TABLES`). When the table is absent those four
//! print a skip notice and pass without proving anything, rather than failing
//! for a file nobody deleted. Every other test here needs only the exe.
//! Same shape as `crates/desert-harvest/tests/game_exe.rs`: these are the tests to run
//! after a game update, because they are what break first.

use std::collections::{BTreeMap, BTreeSet};

use desert_core::creature::{CATCH_BYTES, CATCH_SITE, CATCH_STOLEN};
use desert_core::gimmick::{self, LOADER_PROLOGUE, LOADER_STOLEN};
use desert_core::pattern::{Found, Pattern};
use desert_core::pe;

const EXE: &str = "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe";
/// The directory of extracted static-info tables: `CD_TABLES`, else
/// `<repo>/analysis/tables`, where `just extract-tables` writes them out of
/// the game's archives. A `desert-core` test cannot depend on `desert_tools`
/// (a separate workspace), so the archive is not read here directly.
fn tables_dir() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("CD_TABLES") {
        return std::path::PathBuf::from(p);
    }
    std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../analysis/tables"))
}

/// The `gimmickinfo` body, byte-identical to what the game loads.
fn table_path() -> std::path::PathBuf {
    tables_dir().join("gimmickinfo.staticinfobody")
}

/// The offsets table beside it: a `u16` count, then `count` x `{u32 key,
/// u32 body offset}` (the shape `crates/tools/src/staticinfo.rs` probes for
/// `gimmickinfo`).
fn header_path() -> std::path::PathBuf {
    tables_dir().join("gimmickinfo.staticinfoheader")
}

/// `key -> (start, end)` of every record, from the header, or why not.
fn record_ranges(header: &[u8], body_len: usize) -> Result<BTreeMap<u32, (usize, usize)>, String> {
    let count = header.get(..2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes).ok_or("no count")? as usize;
    if header.len() != 2 + count * 8 {
        return Err(format!("{} header bytes for {count} records of 8", header.len()));
    }
    let entries: Vec<(u32, usize)> = (0..count)
        .filter_map(|i| Some((u32_at(header, 2 + i * 8)?, u32_at(header, 6 + i * 8)? as usize)))
        .collect();
    let mut out = BTreeMap::new();
    for (i, &(key, start)) in entries.iter().enumerate() {
        let end = entries.get(i + 1).map_or(body_len, |e| e.1);
        if !(start < end && end <= body_len) {
            return Err(format!("record {key}: {start}..{end}"));
        }
        if out.insert(key, (start, end)).is_some() {
            return Err(format!("key {key} twice"));
        }
    }
    Ok(out)
}

/// The loud notice a table test prints instead of passing quietly.
fn skipped(what: &str) {
    let bar = "=".repeat(76);
    println!("\n{bar}");
    println!("!! SKIPPED: {what} DID NOT RUN. THIS TEST PROVED NOTHING. !!");
    println!("{bar}");
    println!("missing fixture: {}", table_path().display());
    println!();
    println!("Run `just extract-tables` to write the game's static-info tables into");
    println!("analysis/tables/ (CD_TABLES overrides the directory). Read this as a hole");
    println!("in coverage, not a pass.");
    println!("{bar}\n");
}

/// RVA of the `gimmickinfo` record loader on Steam build 26480954 (`0x3D8FA0`
/// on 26423880, `0x3D89A0` on 26361232, 26356046 and 26345091, where it was re-derived; was
/// `0x38BA40`, `FUN_14038ba40`, on 25477059, `0x385CD0` on 25246367 and
/// `FUN_1403856b0` on the build before that; the function is the same, the
/// address is not).
const LOADER_RVA: usize = 0x3D9280;

/// RVAs of the two `*InfoManager` global pointer slots on Steam build 26480954
/// (`0x71BEE70` and `0x71BEE98` on 26423880, `0x71B7BD0` and `0x71B7BF8` on 26361232 and 26356046, `0x71B6C20` and `0x71B6C48` on 26345091, `0x6D69AA8` and `0x6D69AC8`
/// on 25477059, `0x6C2E2E8` and `0x6C2E308` on 25246367).
/// These are the oracle for `gimmick::resolve_manager_slot`, and are the values
/// `crates/desert-harvest/src/looter/tables.rs` used to carry as hard-coded constants before
/// the slots were resolved by content at startup.
const ITEM_INFO_SLOT_RVA: usize = 0x71C1080;
const GIMMICK_INFO_SLOT_RVA: usize = 0x71C10A8;

/// RVA of the `mov r8d,1` inside the catch-count function on Steam build
/// 26480954, file offset `0x2DDBFD8` (was `0x2DDBF68`, file `0x2DDB368`, on
/// 26423880, `0x2DD6728`, file `0x2DD5B28`, on
/// 26361232, `0x2DD6778`, file `0x2DD5B78`, on
/// 26356046, `0x2DD6448`, file `0x2DD5848`, on 26345091, `0x2B42FB1` on 25477059, `0x2A75891`
/// on 25246367, inside `FUN_142a73c20` on the build before that;
/// `docs/reference-internals.md` section 17.4).
const CATCH_RVA: usize = 0x2DDCBD8;

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
// The archive table: records, output blocks and `multiply`
// ---------------------------------------------------------------------------

/// A record is identified by its key and its name.
type Record = (u32, String);

/// Find every wanted record in one pass over the table, by content alone (the
/// header offsets table is not consulted, so this is an independent check on
/// it): the table's header shape is probed off its first record
/// (`gimmick::detect_header_shape`), every offset is parsed strictly in that
/// shape only, and each wanted `(key, name)` must hit exactly once. Scanning in
/// both shapes would not do: a strict `KeyFirst` parse finds 36248 false
/// headers in the 26361232 `NameFirst` body. Returns each record's first
/// header byte.
fn locate_records(table: &[u8], wanted: &BTreeSet<Record>) -> Result<BTreeMap<Record, usize>, String> {
    let shape = gimmick::detect_header_shape(table)
        .ok_or("the table's first record validates as neither header shape (or as both)")?;
    let mut hits: BTreeMap<Record, Vec<usize>> = BTreeMap::new();
    for p in 0..table.len() {
        let Some(rest) = table.get(p..) else { break };
        if let Some(h) = gimmick::parse_header_as(rest, shape) {
            let k = (h.key, h.name);
            if wanted.contains(&k) {
                hits.entry(k).or_default().push(p);
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
        return Err(format!("{} records not in this table exactly once:\n{}", bad.len(), bad.join("\n")));
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

/// Every independently found min/max field in a record: a `u32` count of
/// 1..=64 at any offset, followed by that many 68-byte blocks with flag 1 at
/// `+0` and `FF FF` at `+58`, the minimum at `+42` and the maximum at `+50`.
/// Looser than `gimmick::block_ok` on purpose (no range check, no item or pad
/// clause, every start tried), so an edit `multiply` makes anywhere but a
/// block's two scalars cannot hide in it.
fn block_scalar_offsets(rec: &[u8]) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for at in 0..rec.len() {
        let Some(count) = u32_at(rec, at) else { break };
        if !(1..=64).contains(&count) {
            continue;
        }
        let blocks: Vec<usize> = (0..count as usize).map(|k| at + 4 + k * 68).collect();
        let shaped = blocks.iter().all(|&b| {
            rec.get(b) == Some(&1) && rec.get(b + 58) == Some(&0xFF) && rec.get(b + 59) == Some(&0xFF) && b + 68 <= rec.len()
        });
        if shaped {
            for b in blocks {
                out.insert(b + 42);
                out.insert(b + 50);
            }
        }
    }
    out
}

/// `gimmick::multiply` at 2 against every `collect::COLLECT_RECORDS` row, on
/// the game's own table: each row gets at least one edit, every edit lands on
/// a block's min or max field (found by [`block_scalar_offsets`], not by
/// `multiply`'s own detector), reads the vanilla `u64` that is really there,
/// and doubles it. The rows are derived from this same archive table, so this
/// is the check that every row is one the hook can actually multiply.
///
/// Records are cut out of the body by the archive's own offsets table, the
/// same split the game's manager keeps at `mgr+0x28`, and each slice is
/// exactly what the record-loader hook is handed.
#[test]
#[ignore]
fn multiply_doubles_every_collect_record_from_the_archive_table() {
    let (Ok(table), Ok(header)) = (std::fs::read(table_path()), std::fs::read(header_path())) else {
        skipped("THE COLLECT-ROW MULTIPLY CHECK");
        return;
    };
    println!("table: {} ({} bytes)", table_path().display(), table.len());
    let ranges = record_ranges(&header, table.len()).expect("the header splits the body");
    assert_eq!(ranges.len(), 14210, "records on builds 26361232, 26423880 and 26480954");
    let shape = gimmick::detect_header_shape(&table).expect("the first record has one header shape");

    let (mut records, mut blocks, mut scalars, mut most) = (0usize, 0usize, 0usize, 0usize);
    for &(key, name, family, display) in desert_core::collect::COLLECT_RECORDS {
        let &(start, end) = ranges.get(&key).unwrap_or_else(|| panic!("{name} ({key}) is not in the table"));
        let rec = &table[start..end];
        let h = gimmick::parse_header_as(rec, shape).unwrap_or_else(|| panic!("{name}: no header"));
        assert_eq!((h.key, h.name.as_str()), (key, name), "the row names the record at its key");
        assert!(!display.is_empty(), "{name}: no display name");

        let edits = gimmick::multiply(rec, 2);
        assert!(!edits.is_empty(), "{name} ({family:?}): multiply finds nothing to edit");
        let fields = block_scalar_offsets(rec);
        for e in &edits {
            assert!(fields.contains(&e.offset), "{name}: edit at +{} is not a block's min or max", e.offset);
            let vanilla = u64::from_le_bytes(rec[e.offset..e.offset + 8].try_into().unwrap());
            assert_eq!(e.old, vanilla, "{name}: +{} old", e.offset);
            assert_eq!(e.new, 2 * e.old, "{name}: +{} new", e.offset);
        }
        assert_eq!(edits.len() % 2, 0, "{name}: min and max in pairs");
        records += 1;
        blocks += edits.len() / 2;
        scalars += edits.len();
        most = most.max(edits.len() / 2);
    }
    println!("{records} records, {blocks} blocks, {scalars} scalars doubled; at most {most} blocks in one record");
    // Build 26361232, measured 2026-11-02 against the archive's table; the same
    // on 26423880 and 26480954.
    assert_eq!((records, blocks, scalars), (294, 606, 1212));
    // `desert_harvest::gatherer::remember` keeps at most 8 blocks a record.
    assert!(most <= 8, "a row has {most} blocks");
}

// ---------------------------------------------------------------------------
// The DMM pack against `multiply`
// ---------------------------------------------------------------------------

const PACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../desert-gatherer-dmm");

/// Minimal JSON field reader: no serde, no new dependencies. The pack files are
/// machine-written by `dmm-pack` (`crates/tools/src/bin/dmm-pack.rs`), so every
/// change object has its fields in the same order (`offset`, `record_key`,
/// `record_rel_offset`, `entry`, `rel_offset`, `label`, `original`, `patched`)
/// and plain string searching is enough.
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

/// A `u64` written as 16 little-endian hex digits, the pack's `original` and
/// `patched`.
fn le_hex(s: &str) -> Option<u64> {
    if s.len() != 16 {
        return None;
    }
    let mut b = [0u8; 8];
    for (i, byte) in b.iter_mut().enumerate() {
        *byte = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(u64::from_le_bytes(b))
}

/// One pack family: `(key, name) -> record-relative offset -> (old, new)`.
type PackFamily = BTreeMap<Record, BTreeMap<usize, (u64, u64)>>;

/// Every change of one module, by record. `None` if a change object does not
/// have its fields in the documented order, or a value does not parse.
///
/// `record_rel_offset`, not `offset`: it counts from the record's `u32` key,
/// which `multiply`'s own `Edit::offset` reaches by adding the key's position in
/// the record, so the two sets compare without trusting any absolute address.
fn read_pack_family(text: &str) -> Option<PackFamily> {
    let mut out: PackFamily = BTreeMap::new();
    let mut cur = 0usize;
    while let Some((_, at)) = number_after(text, "\"offset\":", cur) {
        let (key, at) = number_after(text, "\"record_key\":", at)?;
        let (rel, at) = number_after(text, "\"record_rel_offset\":", at)?;
        let (entry, at) = string_after(text, "\"entry\":", at)?;
        let (old, at) = string_after(text, "\"original\":", at)?;
        let (new, at) = string_after(text, "\"patched\":", at)?;
        let prev = out.entry((u32::try_from(key).ok()?, entry)).or_default().insert(rel as usize, (le_hex(&old)?, le_hex(&new)?));
        if prev.is_some() {
            return None;
        }
        cur = at;
    }
    Some(out)
}

/// The DMM pack is exactly what `gimmick::multiply` does, record by record, on
/// the game's own table.
///
/// For every change in `desert-gatherer-dmm/* - 2X.json`, the record named by
/// `record_key` and `entry` is cut out of the body by the archive's offsets
/// table, `multiply(rec, 2)` is run over it, and the two must agree exactly:
/// the same record-relative offsets (no edit missing, none extra) with the same
/// old and new values. That is the property the pack exists to have - a DMM user
/// and a plugin user get the same nodes, the same blocks and the same numbers -
/// and the check `dmm-pack` cannot make about itself, because it walks the
/// blocks with the tools' port of the detector and this runs the plugin's.
///
/// Before regenerating after a game update this says whether the block format
/// still holds on the new build; after it, that the regenerated pack and the
/// plugin agree. Same fixture and skip contract as the multiply check above.
#[test]
#[ignore]
fn multiply_reproduces_the_dmm_pack_edits() {
    let (Ok(table), Ok(header)) = (std::fs::read(table_path()), std::fs::read(header_path())) else {
        skipped("THE DMM PACK ORACLE");
        return;
    };
    println!("table: {} ({} bytes)", table_path().display(), table.len());
    let ranges = record_ranges(&header, table.len()).expect("the header splits the body");
    let shape = gimmick::detect_header_shape(&table).expect("the first record has one header shape");

    // A repo file, generated by `dmm-pack`. If it cannot be read or does not
    // parse, the pack is broken: a real failure, not a missing-fixture skip.
    let mut names: Vec<String> = std::fs::read_dir(PACK)
        .expect("desert-gatherer-dmm present")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(" - 2X.json"))
        .collect();
    names.sort();
    assert_eq!(names.len(), 4, "expected four 2X modules, got {names:?}");

    let (mut records, mut blocks, mut scalars) = (0usize, 0usize, 0usize);
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for n in &names {
        let text = std::fs::read_to_string(format!("{PACK}/{n}")).expect("pack json readable");
        let family = read_pack_family(&text).unwrap_or_else(|| panic!("{n}: change fields not in the documented order"));
        let mut offsets = 0usize;
        for ((key, name), pack) in &family {
            assert!(seen.insert(*key), "{n}: record {key} is in two modules");
            let &(start, end) = ranges.get(key).unwrap_or_else(|| panic!("{n}: {name} ({key}) is not in the table"));
            let rec = &table[start..end];
            let h = gimmick::parse_header_as(rec, shape).unwrap_or_else(|| panic!("{n}: {name}: no header"));
            assert_eq!((h.key, h.name.as_str()), (*key, name.as_str()), "{n}: the pack names the record at its key");
            // Where the `u32` key sits in the record: first on `KeyFirst`,
            // after the length-prefixed name on `NameFirst`.
            let key_off = match shape {
                gimmick::HeaderShape::KeyFirst => 0,
                gimmick::HeaderShape::NameFirst => 4 + h.name.len(),
            };
            let got: BTreeMap<usize, (u64, u64)> = gimmick::multiply(rec, 2)
                .iter()
                .map(|e| (e.offset - key_off, (e.old, e.new)))
                .collect();
            if got != *pack {
                let (g, p): (BTreeSet<_>, BTreeSet<_>) = (got.iter().collect(), pack.iter().collect());
                panic!(
                    "{n}: record {key} {name:?}: the pack has {:?} that multiply does not, multiply has {:?} that the pack does not",
                    p.difference(&g).collect::<Vec<_>>(),
                    g.difference(&p).collect::<Vec<_>>()
                );
            }
            offsets += pack.len();
        }
        println!("{n:<22} {:>3} records, {:>4} blocks, {:>4} u64 offsets: exact match", family.len(), offsets / 2, offsets);
        records += family.len();
        scalars += offsets;
        blocks += offsets / 2;
    }
    println!("total: {records} records, {blocks} blocks, {scalars} u64 offsets");
    // Pack 1.4 on build 26361232, measured 2026-11-02, and pack 1.5 on 26423880
    // and 26480954 with the same offsets and values: every non-Money row of
    // collect.rs (the three Money rows and their three blocks are the plugin's
    // alone).
    assert_eq!((records, blocks, scalars), (291, 603, 1206));
}

/// Where the item id is, against the table itself.
///
/// `ITEM_AT` and `ITEM_TAIL_AT` were 4 too high until 2026-09-12 - `+5` and
/// `+64`, the two zero pads - so `OutputBlock::item` read 0 for every block in
/// the game, `desert_harvest::gatherer::remember` stored 0, and `hook::reapply`'s
/// "is this the block I remembered" cross-check could never match. `multiply`
/// writes only min/max and so was never affected, which is why the multiply
/// oracle passed throughout and nothing else noticed.
///
/// Nothing synthetic can catch that class of mistake: the unit tests built their
/// blocks with the same wrong offsets the code read them at. This is the test
/// that needs the real bytes, and it asserts two things - `peony_01`'s first
/// block reads back the item id `docs/reference-internals.md` section 13.1
/// records for it, and no block anywhere in the body has an item id of 0, which
/// is what every single one of them had before the fix.
///
/// Same fixture and the same skip contract as the multiply check above: the
/// table's absence is a hole in coverage and says so, not a failure.
#[test]
#[ignore]
fn output_blocks_read_the_item_id_the_table_actually_carries() {
    let Ok(table) = std::fs::read(table_path()) else {
        skipped("THE ITEM-OFFSET CHECK (`gimmick::ITEM_AT` / `ITEM_TAIL_AT` against real bytes)");
        return;
    };
    println!("table: {} bytes", table.len());

    // Locate one record by content, and bound the slice generously; the first
    // block read is certainly this record's.
    let peony: Record = (17_020_006, "peony_01".to_owned());
    let wanted: BTreeSet<Record> = [peony.clone()].into_iter().collect();
    let start = locate_records(&table, &wanted)
        .expect("peony_01 is in this table exactly once")
        .get(&peony)
        .copied()
        .expect("located");
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

/// The installed build's table body is a `NameFirst` table: its first
/// record validates as that shape and no other, a strict scan in that shape
/// finds 14210 headers with 14210 distinct keys (one per record the game's
/// manager counts), and the well and the 26361232 example record (both
/// unmoved on 26423880 and 26480954) sit where
/// they always have. On 25477059 and before the same body was `KeyFirst`; the
/// rule for which shape a table uses is what the gatherer latches in game.
///
/// Same skip contract as the multiply check: no table, no proof, and it says so.
#[test]
#[ignore]
fn the_table_body_is_name_first_with_one_header_per_record() {
    let Ok(table) = std::fs::read(table_path()) else {
        skipped("THE HEADER-SHAPE CHECK");
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
// they live here rather than in `crates/desert-harvest/tests/game_exe.rs`, where they
// were first written. `just test-game` runs both files.
// ---------------------------------------------------------------------------

/// The two tables that are reachable only through the **indirect** accessor
/// template (`mov r8,[rip+cell]`, the cell holding the name's VA), resolved by
/// `gimmick::resolve_manager_slot_based`. `FactionNode` is the one
/// `desert-dispatch` walks. RVAs on build 26480954 (`0x71C1938` and
/// `0x71BEED0` on 26423880, `0x71BA698` and
/// `0x71B7C30` on 26356046 and 26361232, `0x71B96E8` and
/// `0x71B6C80` on 26345091); on 25477059 they were
/// `0x6D6BE70` and `0x6D69AF0`, and on 25246367 `0x6C30308` and `0x6C2E330`,
/// the values the offline census in `docs/findings/2026-09-10-dispatch.md`
/// records.
const PTR_TEMPLATE_SLOTS: &[(&[u8], usize)] =
    &[(gimmick::FACTION_NODE_TABLE, 0x71C3B58), (b"Skill", 0x71C10E0)];

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

/// Names the accessors pass that `crates/tools/static-info-names.txt` (as extracted on
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
/// Values are build 26480954's. On 26423880 the rows read, in this order,
/// `(0x71BEE98, 0x3D8FA0)`, `(0x71BEE70, 0x3D7EB0)`, `(0x71C1938, 0x4D5740)`,
/// `(0x71BEED0, 0x3D9260)`, `(0x71C3EF8, 0x551AF0)`, `(0x71C72F8, 0x73E750)`,
/// `(0x71BEEC8, 0x3D86F0)`. On 26361232, the same as 26356046, the rows
/// read, in this order, `(0x71B7BF8, 0x3D89A0)`, `(0x71B7BD0, 0x3D78B0)`,
/// `(0x71BA698, 0x4D4ED0)`, `(0x71B7C30, 0x3D8C60)`, `(0x71BCC70, 0x5511D0)`,
/// `(0x71C0040, 0x73DE10)`, `(0x71B7C28, 0x3D80F0)`. Those loaders were
/// 26345091's too; its slots were
/// `0x71B6C48`, `0x71B6C20`, `0x71B96E8`, `0x71B6C80`, `0x71BBCC0`,
/// `0x71BF090` and `0x71B6C78`. On 25477059 the rows read, in the same order:
/// `(0x6D69AC8, 0x38BA40)`, `(0x6D69AA8, 0x38AE70)`, `(0x6D6BE70, 0x433880)`,
/// `(0x6D69AF0, 0x38CF30)`, `(0x6D6E428, 0x4A6760)`, `(0x6D71E60, 0x671370)`,
/// `(0x6D69AE8, 0x584C70)`.
const TABLES: &[(&[u8], usize, usize)] = &[
    (gimmick::GIMMICK_TABLE, 0x71C10A8, 0x3D9280),
    (gimmick::ITEM_TABLE, 0x71C1080, 0x3D8190),
    (gimmick::FACTION_NODE_TABLE, 0x71C3B58, 0x4D5DA0),
    (b"Skill", 0x71C10E0, 0x3D9540),
    (gimmick::DROPSET_TABLE, 0x71C6110, 0x552150),
    (gimmick::BUFF_TABLE, 0x71C9500, 0x73EDB0),
    (gimmick::STATUS_TABLE, 0x71C10D8, 0x3D89D0),
];

/// The `gimmickinfo` accessor on 26480954: its head, its slot load, and the
/// distance from the head to the loader call's `E8`. On 26423880 the head was
/// `0x3DDFA8` and the slot load `0x3DDF75`; on 26361232 (and 26356046
/// and 26345091) the head was `0x3DD9A8` and the slot load `0x3DD975`, the
/// same 0x9C to the `E8`.
const GIMMICK_HEAD: usize = 0x3DE288;
const GIMMICK_SLOT_LOAD: usize = 0x3DE255;
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

const NAMES_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/tools/static-info-names.txt");

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
    let text = std::fs::read_to_string(NAMES_FILE).expect("crates/tools/static-info-names.txt readable");
    let listed: BTreeSet<String> = listed_names(&text).iter().map(|n| n.to_lowercase()).collect();
    let unlisted: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|n| {
            !listed.contains(&n.to_lowercase()) && !listed.contains(&format!("{n}info").to_lowercase())
        })
        .collect();
    println!("unlisted names    = {unlisted:?}");
    // A subset, not an equality: `crates/tools/static-info-names.txt` is maintained
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
    // Build 26480954 values; on 26423880 the dropset slot was 0x71C3EF8, on
    // 26361232 and 26356046
    // 0x71BCC70, on 26345091 0x71BBCC0, and on
    // 25477059 they were 0x38BA40, 0x6D69AC8, 0x6D69AA8 and 0x6D6E428.
    assert_eq!(gimmick::resolve_record_loader(&img, base), Ok(base + LOADER_RVA));
    // And so are the two slots the looter resolves without an image base.
    assert_eq!(
        gimmick::resolve_manager_slot(&img, gimmick::GIMMICK_TABLE),
        Ok(GIMMICK_INFO_SLOT_RVA)
    );
    assert_eq!(gimmick::resolve_manager_slot(&img, gimmick::ITEM_TABLE), Ok(ITEM_INFO_SLOT_RVA));
    assert_eq!(gimmick::resolve_manager_slot(&img, gimmick::DROPSET_TABLE), Ok(0x71C6110));
}
