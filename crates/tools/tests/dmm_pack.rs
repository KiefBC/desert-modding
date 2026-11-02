//! `dmm-pack` against a synthetic table and a synthetic `collect.rs`, and (one
//! `#[ignore]`d test) against the real game: regenerating the pack into a copy
//! of `desert-gatherer-dmm/` must reproduce the committed files byte for byte.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

use desert_tools::paths;

const BIN: &str = env!("CARGO_BIN_EXE_dmm-pack");

/// One 68-byte output block paying `item`, `min..=max`.
fn block(item: u32, min: u64, max: u64) -> Vec<u8> {
    let mut b = vec![0u8; 68];
    b[0] = 1;
    b[1..5].copy_from_slice(&item.to_le_bytes());
    b[42..50].copy_from_slice(&min.to_le_bytes());
    b[50..58].copy_from_slice(&max.to_le_bytes());
    b[58] = 0xFF;
    b[59] = 0xFF;
    b[60..64].copy_from_slice(&item.to_le_bytes());
    b
}

fn list(blocks: &[(u32, u64, u64)]) -> Vec<u8> {
    let mut v = (blocks.len() as u32).to_le_bytes().to_vec();
    for &(item, min, max) in blocks {
        v.extend(block(item, min, max));
    }
    v
}

/// A `NameFirst` record (26361232's shape): the header, then `lists`, each
/// behind a few bytes of padding.
fn record(name: &str, key: u32, lists: &[Vec<u8>]) -> Vec<u8> {
    let path = format!("/object/{name}.prefab");
    let mut r = (name.len() as u32).to_le_bytes().to_vec();
    r.extend_from_slice(name.as_bytes());
    r.extend_from_slice(&key.to_le_bytes());
    r.push(0);
    r.extend_from_slice(&(path.len() as u32).to_le_bytes());
    r.extend_from_slice(path.as_bytes());
    r.push(1);
    for l in lists {
        r.extend_from_slice(&[0xEE; 7]);
        r.extend_from_slice(l);
    }
    r.extend_from_slice(&[0xEE; 5]);
    r
}

/// Where a record's key and its body sit, for the offset assertions.
struct Placed {
    start: usize,
    key_at: usize,
    body: usize,
}

/// Six records, one per family plus a `Money` row and a record no row names.
/// Returns where each one landed, by name.
fn write_tables(dir: &Path) -> BTreeMap<&'static str, Placed> {
    let recs: [(&str, u32, Vec<Vec<u8>>); 6] = [
        ("peony_01", 100, vec![list(&[(7, 1, 3), (8, 2, 2)])]),
        ("firewood_x", 200, vec![list(&[(9, 1, 1)])]),
        ("mine_x", 300, vec![list(&[(10, 1, 2)]), list(&[(11, 4, 6)])]),
        ("ore_x", 400, vec![list(&[(12, 1, 1)])]),
        ("coin_x", 500, vec![list(&[(1, 5, 5)])]),
        ("rock_x", 600, vec![list(&[(13, 1, 1)])]),
    ];
    let (mut body, mut header, mut placed) = (Vec::new(), (recs.len() as u16).to_le_bytes().to_vec(), BTreeMap::new());
    for (name, key, lists) in recs {
        let start = body.len();
        header.extend_from_slice(&key.to_le_bytes());
        header.extend_from_slice(&(start as u32).to_le_bytes());
        let path_len = format!("/object/{name}.prefab").len();
        let key_at = start + 4 + name.len();
        placed.insert(name, Placed { start, key_at, body: key_at + 10 + path_len });
        body.extend(record(name, key, &lists));
    }
    fs::write(dir.join("gimmickinfo.staticinfobody"), body).unwrap();
    fs::write(dir.join("gimmickinfo.staticinfoheader"), header).unwrap();
    placed
}

const COLLECT: &str = "pub const COLLECT_RECORDS: &[(u32, &str, Family, &str)] = &[\n    \
    (500, \"coin_x\", Family::Money, \"Coin\"),\n    \
    (200, \"firewood_x\", Family::Logging, \"Timber\"),\n    \
    (300, \"mine_x\", Family::Mining, \"Stone\"),\n    \
    (400, \"ore_x\", Family::Ore, \"Ore\"),\n    \
    (100, \"peony_01\", Family::Foraging, \"Peony\"),\n];\n";

const MANIFEST: &str = "{\n  \"title\": \"The Desert Gatherer\",\n  \"version\": \"9.9\"\n}\n";

struct Fixture {
    _tmp: tempfile::TempDir,
    tables: PathBuf,
    collect: PathBuf,
    pack: PathBuf,
    placed: BTreeMap<&'static str, Placed>,
}

fn fixture(collect: &str) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let (tables, pack) = (tmp.path().join("tables"), tmp.path().join("pack"));
    fs::create_dir_all(&tables).unwrap();
    fs::create_dir_all(&pack).unwrap();
    let placed = write_tables(&tables);
    let collect_path = tmp.path().join("collect.rs");
    fs::write(&collect_path, collect).unwrap();
    fs::write(pack.join("dmm_pack.json"), MANIFEST).unwrap();
    Fixture { _tmp: tmp, tables, collect: collect_path, pack, placed }
}

fn run(f: &Fixture, extra: &[&str]) -> Output {
    Command::new(BIN)
        .arg("--tables")
        .arg(&f.tables)
        .arg("--collect")
        .arg(&f.collect)
        .arg("--pack-dir")
        .arg(&f.pack)
        .args(extra)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// Every file in `dir` with its bytes.
fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap()))
        .collect()
}

fn module(f: &Fixture, name: &str) -> (String, Value) {
    let raw = fs::read_to_string(f.pack.join(name)).unwrap();
    let v = serde_json::from_str(&raw).unwrap();
    (raw, v)
}

#[test]
fn twelve_modules_in_the_pack_shape() {
    let f = fixture(COLLECT);
    let o = run(&f, &["--build", "123"]);
    assert!(o.status.success(), "{}", text(&o));

    let files: Vec<String> = snapshot(&f.pack).into_keys().collect();
    let mut want: Vec<String> = ["Foraging", "Logging", "Mining", "Ore Nodes"]
        .iter()
        .flat_map(|c| [2, 5, 10].map(|r| format!("{c} - {r}X.json")))
        .collect();
    want.extend(["VERIFICATION.txt".to_string(), "dmm_pack.json".to_string()]);
    want.sort();
    assert_eq!(files, want);

    let (raw, m) = module(&f, "Mining - 5X.json");
    assert!(raw.ends_with("}\r\n") && !raw.replace("\r\n", "").contains('\n'), "CRLF throughout");
    let keys: Vec<&str> = m.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "name", "version", "author", "description", "game", "game_build", "multiplier", "category",
            "allow_partial_apply", "modinfo", "coverage", "patches"
        ]
    );
    assert_eq!(m["name"], "Mining - 5X");
    assert_eq!(m["version"], "9.9");
    assert_eq!(m["modinfo"]["version"], "9.9");
    assert_eq!(m["game_build"], "123");
    assert_eq!(m["multiplier"], 5);
    assert_eq!(
        m["description"],
        "Multiplies mining node minimum and maximum yields by 5. Select no more than one Mining rate in DMM."
    );
    assert_eq!(m["modinfo"]["description"], "5x yields for mining nodes.");
    assert_eq!(m["coverage"], serde_json::json!({"records": 1, "resource_outputs": 2, "scalar_patches": 4}));
    assert_eq!(m["patches"][0]["game_file"], "gamedata/gimmickinfo.pabgb");

    // Both lists of mine_x, the minimum and the maximum of each, in offset order.
    let changes = m["patches"][0]["changes"].as_array().unwrap();
    let p = &f.placed["mine_x"];
    let labels: Vec<&str> = changes.iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert_eq!(
        labels,
        [
            "Mining: mine_x output 1.1 minimum (1 -> 5)",
            "Mining: mine_x output 1.1 maximum (2 -> 10)",
            "Mining: mine_x output 2.1 minimum (4 -> 20)",
            "Mining: mine_x output 2.1 maximum (6 -> 30)",
        ]
    );
    let first_list = p.body + 7;
    assert_eq!(changes[0]["offset"], first_list + 4 + 42);
    assert_eq!(changes[1]["offset"], first_list + 4 + 50);
    for c in changes {
        let keys: Vec<&str> = c.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["offset", "record_key", "record_rel_offset", "entry", "rel_offset", "label", "original", "patched"]);
        let off = c["offset"].as_u64().unwrap() as usize;
        assert_eq!(c["record_key"], 300);
        assert_eq!(c["entry"], "mine_x");
        assert_eq!(c["record_rel_offset"].as_u64().unwrap() as usize, off - p.key_at);
        assert_eq!(c["rel_offset"].as_u64().unwrap() as usize, off - (p.body - 1));
        assert!(off > p.start);
    }
    assert_eq!(changes[2]["original"], "0400000000000000");
    assert_eq!(changes[2]["patched"], "1400000000000000");

    // The two-block list of the Foraging row, at 10X: both blocks, both fields.
    let (_, m) = module(&f, "Foraging - 10X.json");
    let changes = m["patches"][0]["changes"].as_array().unwrap();
    let got: Vec<(&str, &str)> =
        changes.iter().map(|c| (c["original"].as_str().unwrap(), c["patched"].as_str().unwrap())).collect();
    assert_eq!(
        got,
        [
            ("0100000000000000", "0a00000000000000"),
            ("0300000000000000", "1e00000000000000"),
            ("0200000000000000", "1400000000000000"),
            ("0200000000000000", "1400000000000000"),
        ]
    );
    assert_eq!(changes[3]["label"], "Foraging: peony_01 output 1.2 maximum (2 -> 20)");

    // Ore rows are the "Ore Nodes" category; Money and unnamed records are nowhere.
    let (_, m) = module(&f, "Ore Nodes - 2X.json");
    assert_eq!(m["category"], "Ore Nodes");
    assert_eq!(m["modinfo"]["description"], "2x yields for ore nodes nodes.");
    for (name, _) in snapshot(&f.pack) {
        let raw = fs::read_to_string(f.pack.join(&name)).unwrap();
        assert!(!raw.contains("coin_x") && !raw.contains("rock_x"), "{name}");
    }

    let v = fs::read_to_string(f.pack.join("VERIFICATION.txt")).unwrap();
    assert!(v.starts_with("Automated package verification\r\n\r\n{\r\n"));
    let report: Value = serde_json::from_str(v.split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(report["module_count"], 12);
    assert_eq!(report["game_build"], "123");
    assert_eq!(report["category_offset_sets_disjoint"], true);
    assert_eq!(report["mixed_selection_test"]["scalar_patches"], 4 + 2);
    assert_eq!(report["options"][0]["option"], "Foraging - 10X");
    assert_eq!(report["result"], "PASS");
    let body = fs::read(f.tables.join("gimmickinfo.staticinfobody")).unwrap();
    assert_eq!(report["source_gimmickinfo_sha256"], desert_tools::sha256::hex_digest(&body));

    // A second run changes nothing.
    let before = snapshot(&f.pack);
    let o = run(&f, &["--build", "123"]);
    assert!(o.status.success(), "{}", text(&o));
    assert_eq!(snapshot(&f.pack), before);
}

#[test]
fn a_dry_run_writes_nothing() {
    let f = fixture(COLLECT);
    let o = run(&f, &["--build", "123", "--dry-run"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("Automated package verification"), "the report is printed");
    assert_eq!(snapshot(&f.pack).into_keys().collect::<Vec<_>>(), ["dmm_pack.json"]);
}

/// Each of these fails, and leaves an already generated pack exactly as it was.
#[test]
fn an_error_writes_nothing() {
    let f = fixture(COLLECT);
    assert!(run(&f, &["--build", "123"]).status.success());
    let before = snapshot(&f.pack);

    let cases: [(&str, &str); 4] = [
        ("(300, \"mine_x\", Family::Mining", "(300, \"mine_y\", Family::Mining"),
        ("(300, \"mine_x\", Family::Mining", "(301, \"mine_x\", Family::Mining"),
        // Two categories editing the same bytes: their rates could not be mixed.
        ("(400, \"ore_x\", Family::Ore", "(300, \"mine_x\", Family::Ore"),
        // A record whose bytes hold no block the plugin multiplies.
        ("(400, \"ore_x\", Family::Ore", "(400, \"ore_x\", Family::Ore, \"Ore\"),\n    (600, \"rock_x\", Family::Ore"),
    ];
    for (from, to) in cases {
        let bad = COLLECT.replace(from, to);
        assert_ne!(bad, COLLECT);
        fs::write(&f.collect, &bad).unwrap();
        if to.contains("rock_x") {
            // Make rock_x pay nothing: zero its block's flag byte.
            let path = f.tables.join("gimmickinfo.staticinfobody");
            let mut body = fs::read(&path).unwrap();
            let rock = &f.placed["rock_x"];
            body[rock.body + 7 + 4] = 0;
            fs::write(&path, body).unwrap();
        }
        let o = run(&f, &["--build", "456", "--version", "1.0"]);
        assert!(!o.status.success(), "{to}: should fail\n{}", text(&o));
        if to == "(300, \"mine_x\", Family::Ore" {
            assert!(text(&o).contains("same bytes"), "{}", text(&o));
        }
        assert_eq!(snapshot(&f.pack), before, "{to}: the pack changed");
    }
    let err = text(&run(&f, &["--build", "456"]));
    assert!(err.contains("rock_x") && err.contains("no output block"), "{err}");
}

#[test]
fn no_build_id_is_an_error() {
    let f = fixture(COLLECT);
    let o = Command::new(BIN)
        .env("CD_APPMANIFEST", f.pack.join("no-such.acf"))
        .arg("--tables")
        .arg(&f.tables)
        .arg("--collect")
        .arg(&f.collect)
        .arg("--pack-dir")
        .arg(&f.pack)
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(text(&o).contains("no game build id"), "{}", text(&o));
    assert_eq!(snapshot(&f.pack).len(), 1);
}

/// The committed pack is exactly what the generator writes from the
/// installed game's archives and the committed `collect.rs`.
#[test]
#[ignore = "needs the installed game's archives and appmanifest"]
fn regenerating_reproduces_the_committed_pack_byte_for_byte() {
    let pack = paths::repo_root().unwrap().join("desert-gatherer-dmm");
    let tmp = tempfile::tempdir().unwrap();
    for (name, bytes) in snapshot(&pack) {
        fs::write(tmp.path().join(name), bytes).unwrap();
    }
    let o = Command::new(BIN).arg("--pack-dir").arg(tmp.path()).output().unwrap();
    assert!(o.status.success(), "{}", text(&o));
    let (want, got) = (snapshot(&pack), snapshot(tmp.path()));
    assert_eq!(want.keys().collect::<Vec<_>>(), got.keys().collect::<Vec<_>>());
    for (name, bytes) in &want {
        assert!(got[name] == *bytes, "{name} differs from the committed copy; run `just dmm-pack` and diff");
    }
}
