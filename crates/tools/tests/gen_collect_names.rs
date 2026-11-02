//! The generator against its own output.
//!
//! `crates/desert-core/src/collect.rs` is committed and kept in sync, so the honest
//! test of this tool is that it reproduces that file byte for byte, from the
//! game's archives and from the extracted copies in `analysis/tables/`.
//! Everything that needs the real tables is `#[ignore]`d (`just
//! tools-check-game` runs them), because a bare `cargo test` has to pass on a
//! machine with no game installed. The offline tests are the refusals a
//! synthetic table can drive.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use desert_tools::paths;

const BIN: &str = env!("CARGO_BIN_EXE_gen-collect-names");
/// The water well's output list in the calibrated body (`table.rs`
/// `CALIBRATION`'s second anchor): 3374134 on 26361232, 4382086 on 25477059.
const WELL_LIST: usize = 3374134;
/// The four tables the generator reads.
const TABLES: [&str; 4] = ["gimmickinfo", "iteminfo", "interactioninfo", "knowledgeinfo"];

fn committed_collect_rs() -> String {
    let path = paths::repo_root().expect("inside the repo").join("crates/desert-core/src/collect.rs");
    fs::read_to_string(&path).expect("collect.rs is committed")
}

/// `<repo>/analysis/tables`, the extracted copies; every caller is `#[ignore]`d.
fn real_tables() -> PathBuf {
    let dir = paths::repo_root().unwrap().join("analysis/tables");
    assert!(
        dir.join("gimmickinfo.staticinfobody").is_file(),
        "{} has no tables; run `just extract-tables`. This test is #[ignore]d for exactly that reason",
        dir.display()
    );
    dir
}

/// A copy of the four real tables in `dir`, with `edit` applied to the
/// `gimmickinfo` body.
fn copy_tables(dir: &Path, edit: impl FnOnce(&mut Vec<u8>)) {
    let src = real_tables();
    for t in TABLES {
        for ext in [".staticinfobody", ".staticinfoheader"] {
            let name = format!("{t}{ext}");
            fs::copy(src.join(&name), dir.join(&name)).unwrap();
        }
    }
    let body = dir.join("gimmickinfo.staticinfobody");
    let mut b = fs::read(&body).unwrap();
    edit(&mut b);
    fs::write(&body, b).unwrap();
}

/// Point the well's one block at `item`, both copies (`+1` and its echo at
/// `+60`), so the block stays one the detector admits and the walk still
/// calibrates.
fn set_well_item(body: &mut [u8], item: u32) {
    let block = WELL_LIST + 4;
    assert_eq!(u32::from_le_bytes(body[block + 1..block + 5].try_into().unwrap()), 22008);
    body[block + 1..block + 5].copy_from_slice(&item.to_le_bytes());
    body[block + 60..block + 64].copy_from_slice(&item.to_le_bytes());
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
#[ignore = "needs the installed game's archives"]
fn the_archives_regenerate_collect_rs_byte_for_byte() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("collect.rs");
    let o = run(&["--out".as_ref(), out.as_os_str()]);
    assert!(o.status.success(), "{}", stderr(&o));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("source: the archives under"), "{stdout}");
    assert!(stdout.contains("calibrated: 14210 records, 581 lists, 904 blocks"), "{stdout}");
    assert_eq!(fs::read_to_string(&out).unwrap(), committed_collect_rs());
}

#[test]
#[ignore = "needs analysis/tables (just extract-tables)"]
fn the_extracted_tables_regenerate_the_same_file() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("collect.rs");
    let o = run(&["--tables".as_ref(), real_tables().as_os_str(), "--out".as_ref(), out.as_os_str()]);
    assert!(o.status.success(), "{}", stderr(&o));
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("rows: 294 = Foraging 97, Logging 142, Mining 36, Ore 16, Money 3"), "{stdout}");
    assert_eq!(fs::read_to_string(&out).unwrap(), committed_collect_rs());
    // Run over its own output, it reports no change.
    let o = run(&["--tables".as_ref(), real_tables().as_os_str(), "--out".as_ref(), out.as_os_str()]);
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("294 rows before, 0 added, 0 removed, 0 moved family"), "{stdout}");
}

/// The table is the authority on what an override pays, and a disagreement is
/// a refusal rather than a rewrite: a game update that changed the table is
/// exactly what this is here to catch.
#[test]
#[ignore = "needs analysis/tables (just extract-tables)"]
fn a_table_that_disagrees_with_an_override_refuses_to_write() {
    let tmp = tempfile::tempdir().unwrap();
    copy_tables(tmp.path(), |b| set_well_item(b, 48879));
    let out = tmp.path().join("collect.rs");
    let o = run(&["--tables".as_ref(), tmp.path().as_os_str(), "--out".as_ref(), out.as_os_str()]);
    assert!(!o.status.success(), "a drifted table must not be written from");
    let err = stderr(&o);
    assert!(err.contains("the stored items do not match the table"), "{err}");
    assert!(err.contains("in the table: [48879] over 1 output list(s)"), "{err}");
    assert!(!out.exists(), "nothing may be written when the table disagrees");
}

/// The gathering half of the two-way money rule, checked against the table
/// rather than against the json: a record that starts paying item 1 must not
/// quietly become a Foraging row, or a yield slider becomes an economy lever.
#[test]
#[ignore = "needs analysis/tables (just extract-tables)"]
fn money_in_the_table_cannot_ride_into_a_gathering_family() {
    let tmp = tempfile::tempdir().unwrap();
    copy_tables(tmp.path(), |b| set_well_item(b, 1));
    let out = tmp.path().join("collect.rs");
    let o = run(&["--tables".as_ref(), tmp.path().as_os_str(), "--out".as_ref(), out.as_os_str()]);
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(err.contains("which is MONEY"), "{err}");
    assert!(err.contains("(the table)"), "{err}");
    assert!(!out.exists());
}

/// An override the rule already covers is noise that would hide the day the
/// rule stops covering it, and one naming a record the table does not have is
/// a typo or a game update. Both are refusals.
#[test]
#[ignore = "needs analysis/tables (just extract-tables)"]
fn redundant_and_missing_overrides_are_refused() {
    let root = paths::repo_root().unwrap();
    let extras: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("crates/tools/extra-families.json")).unwrap()).unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("collect.rs");
    for (key, rec, want) in [
        ("17020006", serde_json::json!({"name": "peony_01", "items": [757006], "why": "x"}), "the rule already makes it Foraging"),
        ("999", serde_json::json!({"name": "no_such_record", "items": [5], "why": "x"}), "no record with that key in gimmickinfo"),
    ] {
        let mut e = extras.clone();
        e["Foraging"]["records"][key] = rec;
        let path = tmp.path().join("extras.json");
        fs::write(&path, serde_json::to_string_pretty(&e).unwrap()).unwrap();
        let o = run(&[
            "--tables".as_ref(),
            real_tables().as_os_str(),
            "--extras".as_ref(),
            path.as_os_str(),
            "--out".as_ref(),
            out.as_os_str(),
        ]);
        assert!(!o.status.success(), "{key}");
        assert!(stderr(&o).contains(want), "{}", stderr(&o));
        assert!(!out.exists());
    }
}

/// Every row carries its display name, and a row without one is an error
/// naming the row, never an empty string in the table.
#[test]
#[ignore = "needs analysis/tables and analysis/strings"]
fn a_row_with_no_display_name_is_an_error() {
    let root = paths::repo_root().unwrap();
    let text = fs::read_to_string(root.join("analysis/strings/gimmick.eng.tsv")).unwrap();
    let kept: String = text.lines().filter(|l| !l.starts_with("1001081\t")).map(|l| format!("{l}\n")).collect();
    let tmp = tempfile::tempdir().unwrap();
    let strings = tmp.path().join("names.tsv");
    fs::write(&strings, kept).unwrap();
    let out = tmp.path().join("collect.rs");
    let o = run(&[
        "--tables".as_ref(),
        real_tables().as_os_str(),
        "--strings".as_ref(),
        strings.as_os_str(),
        "--out".as_ref(),
        out.as_os_str(),
    ]);
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(err.contains("have no display name"), "{err}");
    assert!(err.contains("1001081 'gimmick_well_0001_parts01' (Foraging)"), "{err}");
    assert!(!out.exists());
}

/// Every default path hangs off `desert_tools::paths::repo_root`, never off the
/// working directory: a cwd-relative default would write `collect.rs`
/// somewhere else entirely, or miss the overrides and the names.
#[test]
#[ignore = "needs analysis/tables (just extract-tables)"]
fn the_defaults_do_not_depend_on_the_working_directory() {
    let committed = committed_collect_rs();
    let root = paths::repo_root().unwrap();
    let tables = real_tables();
    let tmp = tempfile::tempdir().unwrap();
    // The repo root (how it is really invoked), a subdirectory of it, and a
    // directory outside the checkout altogether.
    for cwd in [root.clone(), paths::crate_dir_in(&root, "tools"), tmp.path().to_path_buf()] {
        let out = tmp.path().join(format!("collect-{}.rs", cwd.components().count()));
        let o = Command::new(BIN)
            .current_dir(&cwd)
            .args(["--tables".as_ref(), tables.as_os_str(), "--out".as_ref(), out.as_os_str()])
            .output()
            .unwrap();
        assert!(o.status.success(), "from {}: {}", cwd.display(), stderr(&o));
        assert_eq!(fs::read_to_string(&out).unwrap(), committed, "from {}", cwd.display());
    }
}

/// A body that does not reproduce the calibration is a refusal before
/// anything else is decoded: an uncalibrated table is not the one the rule was
/// measured on. A synthetic one drives it offline.
#[test]
fn a_table_that_does_not_calibrate_refuses_to_write() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("gimmickinfo.staticinfobody"), vec![0u8; 4096]).unwrap();
    fs::write(tmp.path().join("gimmickinfo.staticinfoheader"), [1u8, 0, 7, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    let out = tmp.path().join("collect.rs");
    let o = run(&["--tables".as_ref(), tmp.path().as_os_str(), "--out".as_ref(), out.as_os_str()]);
    assert!(!o.status.success());
    let err = stderr(&o);
    assert!(err.contains("calibration failed:"), "{err}");
    // The refusal names the build it was calibrated on, so after an update it
    // says which side moved.
    let calibrated = format!("calibrated on build {}", paths::CALIBRATED_BUILD);
    assert!(err.contains(&calibrated), "{err}");
    assert!(!out.exists());
}

#[test]
fn a_missing_tables_directory_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("collect.rs");
    let missing = tmp.path().join("no-tables-here");
    let o = run(&["--tables".as_ref(), missing.as_os_str(), "--out".as_ref(), out.as_os_str()]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("cannot read gimmickinfo from"), "{}", stderr(&o));
    assert!(!out.exists());
}
