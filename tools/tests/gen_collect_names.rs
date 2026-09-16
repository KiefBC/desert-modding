//! The generator against its own output.
//!
//! `desert-core/src/collect.rs` is committed and kept in sync, so the honest
//! test of this tool is that it reproduces that file byte for byte. Both paths
//! are tested: with DMM's clean table body, where every stored `items` list is
//! re-derived from the bytes first, and without it, where the banner says so.
//! Everything that needs the body is `#[ignore]`d, because a bare `cargo test`
//! has to pass on a machine that has never run DMM.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use desert_tools::paths;

const BIN: &str = env!("CARGO_BIN_EXE_gen-collect-names");

fn committed_collect_rs() -> (PathBuf, String) {
    let path = paths::repo_root().expect("inside the repo").join("desert-core/src/collect.rs");
    let text = fs::read_to_string(&path).expect("collect.rs is committed");
    (path, text)
}

/// The table body, or `None` - every caller of this is `#[ignore]`d.
fn clean_body() -> PathBuf {
    let table = paths::dmm_table();
    assert!(
        table.exists(),
        "{} is absent; this test is #[ignore]d for exactly that reason",
        table.display()
    );
    table
}

#[test]
#[ignore = "needs DMM's clean gimmickinfo table body"]
fn the_verified_path_regenerates_collect_rs_byte_for_byte() {
    let (_, committed) = committed_collect_rs();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("collect.rs");
    let run = Command::new(BIN)
        .args(["--table".as_ref(), clean_body().as_os_str(), "--out".as_ref(), out.as_os_str()])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("verified Foraging.records:"), "{stdout}");
    assert!(stdout.contains("verified Money.records_not_enabled:"), "{stdout}");
    assert_eq!(fs::read_to_string(&out).unwrap(), committed);
}

#[test]
fn the_unverified_path_still_regenerates_and_says_so() {
    let (_, committed) = committed_collect_rs();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("collect.rs");
    let missing = tmp.path().join("no-such-clean-body.bin");
    let run = Command::new(BIN)
        .args(["--table".as_ref(), missing.as_os_str(), "--out".as_ref(), out.as_os_str()])
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(stdout.contains("is ABSENT."), "{stdout}");
    assert!(stdout.contains("were NOT verified against"), "{stdout}");
    assert!(!stdout.contains("verified Foraging.records:"), "{stdout}");
    // The banner is a warning, not a refusal: the file still regenerates, and
    // it is the same file, because the stored rows are what it is built from
    // either way.
    assert_eq!(fs::read_to_string(&out).unwrap(), committed);
}

/// The clean body is the authority on what a record pays, and a disagreement is
/// a refusal rather than a rewrite: a game update that changed the table is
/// exactly what this is here to catch, and half-rewriting `collect.rs` from
/// rows nobody has checked is the outcome worth avoiding.
#[test]
#[ignore = "needs DMM's clean gimmickinfo table body"]
fn a_body_that_disagrees_refuses_to_write() {
    let tmp = tempfile::tempdir().unwrap();
    let table = tmp.path().join("corrupt.bin");
    let mut body = fs::read(clean_body()).unwrap();
    // The water well's output list is at 4373870 (the CALIBRATION anchor) and
    // the item id sits at +4 for the count, +1 for ITEM_AT. Changing it makes
    // the body say the well pays something the json does not claim, without
    // disturbing the block flags the walk calibrates on.
    let item = 4373870 + 4 + 1;
    assert_eq!(u32::from_le_bytes(body[item..item + 4].try_into().unwrap()), 22008);
    body[item..item + 4].copy_from_slice(&48879u32.to_le_bytes());
    fs::write(&table, &body).unwrap();

    let out = tmp.path().join("collect.rs");
    let run = Command::new(BIN)
        .args(["--table".as_ref(), table.as_os_str(), "--out".as_ref(), out.as_os_str()])
        .output()
        .unwrap();
    assert!(!run.status.success(), "a drifted body must not be written from");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("the stored items do not match the clean body"), "{stderr}");
    assert!(stderr.contains("in the table: [48879] over 1 output list(s)"), "{stderr}");
    assert!(!out.exists(), "nothing may be written when the body disagrees");
}

/// The gathering half of the two-way money rule, checked against the body
/// rather than against the json: a record that starts paying item 1 must not
/// quietly become a Foraging row, or a yield slider becomes an economy lever.
#[test]
#[ignore = "needs DMM's clean gimmickinfo table body"]
fn money_in_the_body_cannot_ride_into_a_gathering_family() {
    let tmp = tempfile::tempdir().unwrap();
    let table = tmp.path().join("money.bin");
    let mut body = fs::read(clean_body()).unwrap();
    let item = 4373870 + 4 + 1;
    body[item..item + 4].copy_from_slice(&1u32.to_le_bytes());
    fs::write(&table, &body).unwrap();

    let out = tmp.path().join("collect.rs");
    let run = Command::new(BIN)
        .args(["--table".as_ref(), table.as_os_str(), "--out".as_ref(), out.as_os_str()])
        .output()
        .unwrap();
    assert!(!run.status.success());
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("which is MONEY"), "{stderr}");
    assert!(stderr.contains("(the clean body)"), "{stderr}");
    assert!(!out.exists());
}

/// A calibration that no longer reproduces is a refusal too: an uncalibrated
/// record walk mis-spans boundaries silently, which is the failure mode the
/// whole method exists to avoid.
#[test]
#[ignore = "needs DMM's clean gimmickinfo table body"]
fn a_body_that_does_not_calibrate_refuses_to_write() {
    let tmp = tempfile::tempdir().unwrap();
    let table = tmp.path().join("short.bin");
    let body = fs::read(clean_body()).unwrap();
    fs::write(&table, &body[..body.len() / 2]).unwrap();

    let out = tmp.path().join("collect.rs");
    let run = Command::new(BIN)
        .args(["--table".as_ref(), table.as_os_str(), "--out".as_ref(), out.as_os_str()])
        .output()
        .unwrap();
    assert!(!run.status.success());
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("calibration failed:"), "{stderr}");
    assert!(!out.exists());
}

/// The pack is the other input, and a pack directory with no `2X` module is a
/// mistake worth naming rather than an empty enum.
#[test]
fn an_empty_pack_directory_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("collect.rs");
    let run = Command::new(BIN)
        .args([tmp.path().as_os_str(), "--out".as_ref(), out.as_os_str()])
        .output()
        .unwrap();
    assert!(!run.status.success());
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("no '* - 2X.json' modules under"), "{stderr}");
    assert!(!out.exists());
}

/// Every default path hangs off `desert_tools::paths::repo_root`, never off the
/// working directory. The tools are invoked with the repo root as the cwd, but a
/// cwd-relative default would write `collect.rs` somewhere else entirely - or,
/// worse, find no `* - 2X.json` modules and generate from an empty pack without
/// anyone noticing which of the two happened.
#[test]
fn the_defaults_do_not_depend_on_the_working_directory() {
    let (_, committed) = committed_collect_rs();
    let root = paths::repo_root().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("no-such-clean-body.bin");
    // The repo root (how it is really invoked), a subdirectory of it, and a
    // directory outside the checkout altogether.
    for cwd in [root.clone(), root.join("tools"), tmp.path().to_path_buf()] {
        let out = tmp.path().join(format!("collect-{}.rs", cwd.components().count()));
        let run = Command::new(BIN)
            .current_dir(&cwd)
            .args(["--table".as_ref(), missing.as_os_str(), "--out".as_ref(), out.as_os_str()])
            .output()
            .unwrap();
        assert!(run.status.success(), "from {}: {}", cwd.display(), String::from_utf8_lossy(&run.stderr));
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(stdout.contains("records: 279 (DMM 275 + extras 4)"), "from {}: {stdout}", cwd.display());
        assert_eq!(fs::read_to_string(&out).unwrap(), committed, "from {}", cwd.display());
    }
}
