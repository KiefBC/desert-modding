//! End-to-end tests for the `dmm-rebase` binary.
//!
//! Each runs the real binary inside a synthetic repository in a temp
//! directory: a two-module pack (`Logging - 5X`, `Mining - 2X`, the pair the
//! report's mixed-selection test needs) over a hand-built table whose records
//! have moved since the pack was written. What matters about this tool is what
//! it leaves on disk - the rewritten offsets, CRLF, and above all that a
//! failure part way through leaves NOTHING rewritten - so that is what these
//! look at.
//!
//! The last test is `#[ignore]`d: it rebases a copy of the real pack onto the
//! real clean table and demands that nothing changes, which holds whenever the
//! committed pack is current for the table on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};

use desert_tools::paths;

const BIN: &str = env!("CARGO_BIN_EXE_dmm-rebase");
const BLOCK: usize = 68;

/// A 68-byte resource-output block.
fn block(item: u32, min: u64, max: u64) -> Vec<u8> {
    let mut b = vec![0u8; BLOCK];
    b[0] = 1;
    b[5..9].copy_from_slice(&item.to_le_bytes());
    b[42..50].copy_from_slice(&min.to_le_bytes());
    b[50..58].copy_from_slice(&max.to_le_bytes());
    b[58..60].copy_from_slice(&[0xff, 0xff]);
    b[64..68].copy_from_slice(&item.to_le_bytes());
    b
}

/// One record as the fixture lays it out.
struct Record {
    key: u32,
    name: &'static str,
    /// Where the record's key sits in the NEW table.
    at: usize,
    /// Where its output list sits relative to the key, new and old.
    list_now: usize,
    list_then: usize,
    /// (item, min, max) per block.
    blocks: Vec<(u32, u64, u64)>,
}

fn records() -> Vec<Record> {
    vec![
        // Moved in the table, list unchanged inside the record.
        Record { key: 17_030_001, name: "mine_a", at: 5000, list_now: 200, list_then: 200, blocks: vec![(10, 1, 2), (11, 3, 3)] },
        // Moved in the table AND its list shifted 24 bytes inside the record.
        Record { key: 18_000_001, name: "tree_b", at: 9000, list_now: 124, list_then: 100, blocks: vec![(20, 2, 4)] },
    ]
}

fn table_bytes() -> Vec<u8> {
    let mut t = vec![0u8; 16_384];
    for r in records() {
        t[r.at..r.at + 4].copy_from_slice(&r.key.to_le_bytes());
        t[r.at + 4..r.at + 8].copy_from_slice(&(r.name.len() as u32).to_le_bytes());
        t[r.at + 8..r.at + 8 + r.name.len()].copy_from_slice(r.name.as_bytes());
        let list = r.at + r.list_now;
        t[list..list + 4].copy_from_slice(&(r.blocks.len() as u32).to_le_bytes());
        for (i, &(item, mn, mx)) in r.blocks.iter().enumerate() {
            let b = list + 4 + i * BLOCK;
            t[b..b + BLOCK].copy_from_slice(&block(item, mn, mx));
        }
    }
    t
}

/// The changes a module held for `r` before the update: offsets from the
/// OLD layout, where every record sat 1000 bytes earlier.
fn changes(r: &Record, category: &str, mult: u64) -> Vec<Value> {
    let mut out = Vec::new();
    for (i, &(_, mn, mx)) in r.blocks.iter().enumerate() {
        for (kind, at, v) in [("minimum", 42, mn), ("maximum", 50, mx)] {
            let rel = r.list_then + 4 + i * BLOCK + at;
            out.push(json!({
                "offset": r.at - 1000 + rel,
                "record_key": r.key,
                "record_rel_offset": rel,
                "entry": r.name,
                "rel_offset": rel - 8 - r.name.len(),
                "label": format!("{category}: {} output 1.{} {kind} ({v} -> {})", r.name, i + 1, v * mult),
                "original": hex(v),
                "patched": hex(v * mult),
            }));
        }
    }
    out
}

fn hex(v: u64) -> String {
    v.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect()
}

fn module(category: &str, mult: u64, r: &Record) -> Value {
    let name = format!("{category} - {mult}X");
    json!({
        "name": name,
        "version": "1.1",
        "game_build": "100",
        "multiplier": mult,
        "category": category,
        "modinfo": { "name": name, "version": "1.1" },
        "coverage": { "records": 1, "resource_outputs": r.blocks.len(), "scalar_patches": r.blocks.len() * 2 },
        "patches": [{ "game_file": "gamedata/gimmickinfo.pabgb", "changes": changes(r, category, mult) }],
    })
}

fn crlf_json(v: &Value) -> String {
    (serde_json::to_string_pretty(v).unwrap() + "\n").replace('\n', "\r\n")
}

/// A repo `paths::repo_root` accepts, holding the pack and the table.
fn fixture(root: &Path) -> PathBuf {
    fs::write(root.join("justfile"), "").unwrap();
    fs::write(root.join("flake.nix"), "").unwrap();
    let pack = root.join("desert-gatherer-dmm");
    fs::create_dir(&pack).unwrap();
    fs::write(pack.join("dmm_pack.json"), "{\"version\": \"1.1\"}\n").unwrap();
    fs::write(pack.join("VERIFICATION.txt"), "old\r\n").unwrap();
    let rs = records();
    fs::write(pack.join("Logging - 5X.json"), crlf_json(&module("Logging", 5, &rs[1]))).unwrap();
    fs::write(pack.join("Mining - 2X.json"), crlf_json(&module("Mining", 2, &rs[0]))).unwrap();
    let table = root.join("table.bin");
    fs::write(&table, table_bytes()).unwrap();
    table
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(root)
        // Never the real install's appmanifest, whatever this machine has.
        .env("CD_APPMANIFEST", root.join("no-such-appmanifest.acf"))
        .output()
        .unwrap()
}

fn snapshot(pack: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = fs::read_dir(pack)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (p.file_name().unwrap().to_string_lossy().into_owned(), fs::read(&p).unwrap())
        })
        .collect();
    files.sort();
    files
}

#[test]
fn a_rebase_rewrites_the_offsets_and_the_report() {
    let dir = tempfile::tempdir().unwrap();
    let table = fixture(dir.path());
    let out = run(dir.path(), &["--table", table.to_str().unwrap(), "--build", "200"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}{stdout}", String::from_utf8_lossy(&out.stderr));

    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].starts_with("table: 16384 bytes sha256="), "{stdout}");
    assert!(lines[0].ends_with(" build=200"), "{stdout}");
    assert_eq!(
        &lines[1..],
        [
            "  Logging - 5X.json: 2 patches verified; output lists: 0 unchanged, 1 shifted inside record",
            "  Mining - 2X.json: 4 patches verified; output lists: 1 unchanged, 0 shifted inside record",
            "disjoint: True | result: PASS",
        ]
    );

    let pack = dir.path().join("desert-gatherer-dmm");
    let rs = records();
    for (file, r) in [("Logging - 5X.json", &rs[1]), ("Mining - 2X.json", &rs[0])] {
        let text = fs::read_to_string(pack.join(file)).unwrap();
        assert!(!text.replace("\r\n", "").contains('\n'), "{file} is not CRLF");
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["game_build"], "200");
        // Nothing but the offsets moved: the same key order, the same labels.
        assert_eq!(v["version"], "1.1");
        for (i, c) in v["patches"][0]["changes"].as_array().unwrap().iter().enumerate() {
            let at = if i % 2 == 0 { 42 } else { 50 };
            let rel = r.list_now + 4 + (i / 2) * BLOCK + at;
            assert_eq!(c["offset"], r.at + rel, "{file} change {i}");
            assert_eq!(c["record_rel_offset"], rel, "{file} change {i}");
            assert_eq!(c["rel_offset"], rel - 8 - r.name.len(), "{file} change {i}");
        }
        // A key order check that does not trust serde_json to report it.
        assert!(text.find("\"offset\"").unwrap() < text.find("\"record_key\"").unwrap());
    }

    let report = fs::read_to_string(pack.join("VERIFICATION.txt")).unwrap();
    let body = report.strip_prefix("Automated package verification\r\n\r\n").expect("header");
    let v: Value = serde_json::from_str(body).unwrap();
    assert_eq!(v["game_build"], "200");
    assert_eq!(v["module_count"], 2);
    assert_eq!(v["categories"]["Mining"]["scalar_patches"], 4);
    assert_eq!(v["mixed_selection_test"]["scalar_patches"], 6);
    assert_eq!(v["result"], "PASS");
    let names: Vec<&str> = v["options"].as_array().unwrap().iter().map(|o| o["option"].as_str().unwrap()).collect();
    assert_eq!(names, ["Logging - 5X", "Mining - 2X"]);
}

#[test]
fn a_dry_run_prints_the_report_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let table = fixture(dir.path());
    let pack = dir.path().join("desert-gatherer-dmm");
    let before = snapshot(&pack);
    let out = run(dir.path(), &["--table", table.to_str().unwrap(), "--build", "200", "--dry-run"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Automated package verification\n\n{\n  \"game_build\": \"200\","), "{stdout}");
    assert!(stdout.ends_with("disjoint: True | result: PASS\n"), "{stdout}");
    assert_eq!(snapshot(&pack), before);
}

/// The reason the tool verifies everything before writing anything: the
/// first module (by name) rebases cleanly, the second cannot, and the first
/// must NOT have been rewritten.
#[test]
fn a_failure_in_a_later_module_leaves_every_file_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let table = fixture(dir.path());
    let pack = dir.path().join("desert-gatherer-dmm");
    // Mining's second block no longer holds the vanilla maximum it expects.
    let mut bytes = fs::read(&table).unwrap();
    let r = &records()[0];
    let max = r.at + r.list_now + 4 + BLOCK + 50;
    bytes[max] = 9;
    fs::write(&table, bytes).unwrap();

    let before = snapshot(&pack);
    let out = run(dir.path(), &["--table", table.to_str().unwrap(), "--build", "200"]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("dmm-rebase: Mining - 2X.json: record 17030001 (mine_a) group 1: 0 candidate lists"),
        "{stderr}"
    );
    // Byte for byte, and no temporary left beside them either.
    assert_eq!(snapshot(&pack), before, "a file was written despite the failure");
}

#[test]
fn no_build_id_is_a_clear_error_not_a_guess() {
    let dir = tempfile::tempdir().unwrap();
    let table = fixture(dir.path());
    let out = run(dir.path(), &["--table", table.to_str().unwrap()]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.starts_with("dmm-rebase: no game build id: pass --build"), "{stderr}");
}

/// The real pack against the real table: with the committed pack current for
/// that table, rebasing it again must reproduce every file byte for byte -
/// offsets, formatting, CRLF and the report. This is the port's acceptance
/// test against the Python's own output.
#[test]
#[ignore = "needs the clean gimmickinfo table the committed pack was built from"]
fn rebasing_the_real_pack_onto_its_own_table_changes_nothing() {
    let real_root = paths::repo_root().unwrap();
    let real_pack = real_root.join("desert-gatherer-dmm");
    let table = paths::dmm_table();
    assert!(table.exists(), "{} is absent", table.display());

    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("justfile"), "").unwrap();
    fs::write(dir.path().join("flake.nix"), "").unwrap();
    let pack = dir.path().join("desert-gatherer-dmm");
    fs::create_dir(&pack).unwrap();
    for (name, bytes) in snapshot(&real_pack) {
        fs::write(pack.join(name), bytes).unwrap();
    }
    let committed: Value =
        serde_json::from_str(&fs::read_to_string(real_pack.join("Mining - 2X.json")).unwrap()).unwrap();
    let build = committed["game_build"].as_str().unwrap().to_string();

    let before = snapshot(&pack);
    let out = run(dir.path(), &["--table", table.to_str().unwrap(), "--build", &build]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let after = snapshot(&pack);
    for ((name, a), (_, b)) in before.iter().zip(&after) {
        assert!(a == b, "{name} changed on a rebase onto its own table");
    }
}
