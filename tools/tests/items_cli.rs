//! End-to-end checks on the `items` binary.
//!
//! The offline ones build a tiny synthetic table body, so a bare `cargo test`
//! passes on a machine that has never seen DMM. The ones that need the real
//! 22 MB body are `#[ignore]`d and say so; run them with
//! `cargo test -- --ignored`.
//!
//! Every run here passes `--json`/`--doc` into a temporary directory. Nothing
//! in this file may write `analysis/` or `docs/`: a test that clobbers the
//! artifacts a human is reading is worse than no test.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

const BLOCK: usize = 68;

/// A 68-byte output block. `entry_key` is the `+64` field - zero on a block the
/// shipped detector admits, nonzero on one only `--loose` sees.
fn block(item: u32, mn: u64, mx: u64, entry_key: u32) -> Vec<u8> {
    let mut b = vec![0u8; BLOCK];
    b[0] = 1;
    b[1..5].copy_from_slice(&item.to_le_bytes());
    b[42..50].copy_from_slice(&mn.to_le_bytes());
    b[50..58].copy_from_slice(&mx.to_le_bytes());
    b[58] = 0xFF;
    b[59] = 0xFF;
    b[60..64].copy_from_slice(&item.to_le_bytes());
    b[64..68].copy_from_slice(&entry_key.to_le_bytes());
    b
}

/// `u32 key, u32 len, name, NUL`, then the digits-only echo that makes the
/// walk accept it as a record header rather than a nested string field.
fn record(out: &mut Vec<u8>, key: u32, name: &str) {
    for s in [name.to_string(), key.to_string()] {
        out.extend_from_slice(&key.to_le_bytes());
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s.as_bytes());
        out.push(0);
    }
}

/// Two records: one whose single block the plugin can see, one whose block
/// carries a nonzero `+64` and so exists only for the loose detector.
fn fixture() -> Vec<u8> {
    let mut b = vec![0u8; 16];
    record(&mut b, 500001, "synthetic_peony_01");
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&block(757006, 1, 3, 0));
    record(&mut b, 500002, "synthetic_chest_01");
    b.extend_from_slice(&1u32.to_le_bytes());
    b.extend_from_slice(&block(919293, 1, 1, 77));
    b
}

fn items(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_items"))
        .args(args)
        .output()
        .expect("failed to run the items binary")
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("stdout is not UTF-8")
}

fn read_json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// The two artifact pairs never touch, and a cache whose detector does not
/// match what was asked for is ignored rather than answered from.
///
/// This is a correctness property, not a nicety: answering a `--loose` question
/// out of `analysis/items.json` silently hides every loose-only id, which is
/// the one thing the flag exists to show, and answering a default question out
/// of the loose file invents ids the plugin cannot reach.
#[test]
fn a_cache_built_by_the_other_detector_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let table = dir.path().join("table.bin");
    std::fs::write(&table, fixture()).unwrap();
    let t = table.to_str().unwrap();
    let sj = dir.path().join("shipped.json");
    let lj = dir.path().join("loose.json");
    let sd = dir.path().join("shipped.md");
    let ld = dir.path().join("loose.md");
    let (sjs, ljs) = (sj.to_str().unwrap(), lj.to_str().unwrap());

    let o = items(&["--table", t, "--json", sjs, "--doc", sd.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = items(&["--loose", "--table", t, "--json", ljs, "--doc", ld.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));

    // The detector block is what a loaded cache is identified by, and only the
    // loose build writes one.
    let shipped = read_json(&sj);
    let loose = read_json(&lj);
    assert!(shipped.get("detector").is_none());
    assert_eq!(loose["detector"]["name"], "loose");
    // The synthetic body is built so the two populations differ: the chest's
    // block carries a nonzero `+64`.
    assert_eq!(shipped["totals"]["distinct_items"], 1);
    assert_eq!(loose["totals"]["distinct_items"], 2);

    // Plant the shipped file where a loose run reads its cache. The loose
    // lookup must walk the table again rather than answer out of it, which it
    // proves by knowing about the loose-only id at all.
    std::fs::copy(&sj, &lj).unwrap();
    let o = items(&["--loose", "--table", t, "--json", ljs, "--list"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(
        stdout(&o).contains("LOOSE-ONLY"),
        "a loose lookup answered out of a shipped cache: {}",
        stdout(&o)
    );

    // And the other way: a default lookup handed the loose file must not
    // report ids the plugin cannot see.
    std::fs::copy(&lj, &sj).unwrap();
    // (`lj` currently holds the shipped copy, so rebuild the loose one first.)
    let o = items(&["--loose", "--rescan", "--table", t, "--json", ljs, "--doc", ld.to_str().unwrap()]);
    assert!(o.status.success());
    std::fs::copy(&lj, &sj).unwrap();
    let o = items(&["--table", t, "--json", sjs, "--list"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = stdout(&o);
    assert!(!out.contains("LOOSE-ONLY"), "default lookup leaked loose rows: {out}");
    assert!(!out.contains("919293"), "default lookup leaked a loose-only id: {out}");
    assert_eq!(out.lines().count(), 1, "default population is one item: {out}");
}

/// A loose run writes the loose pair and touches neither default artifact.
#[test]
fn a_loose_rebuild_leaves_the_default_pair_alone() {
    let dir = tempfile::tempdir().unwrap();
    let table = dir.path().join("table.bin");
    std::fs::write(&table, fixture()).unwrap();
    let t = table.to_str().unwrap();
    let sj = dir.path().join("shipped.json");
    let sd = dir.path().join("shipped.md");
    let lj = dir.path().join("loose.json");
    let ld = dir.path().join("loose.md");

    items(&["--table", t, "--json", sj.to_str().unwrap(), "--doc", sd.to_str().unwrap()]);
    let before = (std::fs::read(&sj).unwrap(), std::fs::read(&sd).unwrap());
    items(&["--loose", "--table", t, "--json", lj.to_str().unwrap(), "--doc", ld.to_str().unwrap()]);
    let after = (std::fs::read(&sj).unwrap(), std::fs::read(&sd).unwrap());
    assert_eq!(before, after, "a loose run modified the default artifacts");
    assert!(lj.exists() && ld.exists());
}

/// An unreadable table is a user error, not a panic, and the message says what
/// to do about it.
#[test]
fn a_missing_table_is_reported_not_panicked() {
    let o = items(&["--table", "/nonexistent/gimmickinfo.bin", "--rescan", "--list"]);
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("cannot read the clean table body"), "{err}");
    assert!(err.contains("--table"), "{err}");
}

// ----------------------------------------------- needs DMM's real table body

fn real_table() -> Option<String> {
    let p = desert_tools::paths::dmm_table();
    p.is_file().then(|| p.to_str().unwrap().to_string())
}

/// The five numbers findings section 9 confirmed three independent ways. A
/// `FAIL` here means the table or the walk moved and nothing downstream should
/// be trusted until it is explained.
#[test]
#[ignore = "needs DMM's clean gimmickinfo body"]
fn the_shipped_walk_still_finds_573_896_215() {
    let Some(t) = real_table() else { return };
    let dir = tempfile::tempdir().unwrap();
    let j = dir.path().join("items.json");
    let o = items(&[
        "--table", &t,
        "--json", j.to_str().unwrap(),
        "--doc", dir.path().join("doc.md").to_str().unwrap(),
    ]);
    let out = stdout(&o);
    assert!(!out.contains("FAIL"), "{out}");
    let d = read_json(&j);
    assert_eq!(d["totals"]["output_lists"], 573);
    assert_eq!(d["totals"]["blocks"], 896);
    assert_eq!(d["totals"]["distinct_items"], 215);
    assert_eq!(d["totals"]["records_in_table"], 13412);
}

/// The loose walk is a *superset*, not a different answer: its own three counts
/// plus the relations that tie it to the shipped one.
#[test]
#[ignore = "needs DMM's clean gimmickinfo body"]
fn the_loose_walk_is_a_superset_of_the_shipped_one() {
    let Some(t) = real_table() else { return };
    let dir = tempfile::tempdir().unwrap();
    let j = dir.path().join("items-loose.json");
    let o = items(&[
        "--loose",
        "--table", &t,
        "--json", j.to_str().unwrap(),
        "--doc", dir.path().join("doc.md").to_str().unwrap(),
    ]);
    let out = stdout(&o);
    assert!(!out.contains("FAIL"), "{out}");
    let d = read_json(&j);
    assert_eq!(d["totals"]["output_lists"], 589);
    assert_eq!(d["totals"]["blocks"], 1038);
    assert_eq!(d["totals"]["distinct_items"], 311);
    assert_eq!(d["totals"]["shipped_lists"], 573);
    assert_eq!(d["totals"]["loose_only_lists"], 16);
    assert_eq!(d["totals"]["loose_only_blocks"], 142);
    assert_eq!(d["totals"]["items_loose_only"], 96);
    // The nine nine-digit ids, all inside the one mis-parsed list. They are
    // flagged, never filtered: the flag is the finding.
    assert_eq!(d["totals"]["implausible_item_ids"], 9);
    assert_eq!(
        d["implausible_item_ids"]["records"],
        serde_json::json!(["gimmick_item_dropset_treasurebox_01"])
    );
}

/// Item `1` is money and `22008` is water, both by evidence the name inference
/// cannot see, and both keep their reason in the output.
#[test]
#[ignore = "needs DMM's clean gimmickinfo body"]
fn the_curated_ids_keep_their_names_and_reasons() {
    let Some(t) = real_table() else { return };
    let o = items(&["--table", &t, "--rescan", "22008"]);
    let out = stdout(&o);
    assert!(out.starts_with("item 22008  water   [curated]"), "{out}");
    assert!(out.contains("GIMMICK_WATER_PICKUP"), "{out}");
    let o = items(&["--table", &t, "--rescan", "1"]);
    let out = stdout(&o);
    assert!(out.starts_with("item 1  money   [curated]"), "{out}");
    assert!(out.contains("must stay out of any gather family"), "{out}");
}
