//! End-to-end tests for the `nexus-target` binary.
//!
//! Every assertion here is on the exact bytes of stdout, because those lines
//! are appended to `$GITHUB_OUTPUT` and read back as step outputs: the `nexus`
//! job compares `publish` and `changelog` against the string `'true'` in its
//! `if:` expressions, and hands the rest to the upload action as inputs. A key
//! renamed or a boolean spelled `True` would not fail here or there - the step
//! would just quietly not run, or upload with the wrong name.

use std::path::Path;
use std::process::{Command, Output};

const TARGETS: &str = r#"{
  "game_domain": "crimsondesert",
  "targets": {
    "desert-tooling": {
      "game_scoped_id": "3369",
      "file_id": "7934145",
      "zip": "DesertTooling-{version}.zip",
      "display_name": "Desert Tooling (ASI)",
      "category": "main",
      "update_mod_version": true,
      "changelog": true
    },
    "desert-gatherer-dmm": {
      "game_scoped_id": "3369",
      "file_id": "7924209",
      "zip": "DesertGatherer-DMM-{version}.zip",
      "display_name": "Desert Gatherer (DMM)",
      "category": "main",
      "update_mod_version": false,
      "changelog": true
    }
  }
}
"#;

fn fixture(root: &Path, targets: &str) {
    std::fs::write(root.join("justfile"), "").unwrap();
    std::fs::write(root.join("flake.nix"), "").unwrap();
    std::fs::create_dir_all(root.join("tools")).unwrap();
    std::fs::write(root.join("tools/nexus-targets.json"), targets).unwrap();
}

fn run(root: &Path, tag: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nexus-target"))
        .arg(tag)
        .current_dir(root)
        .output()
        .expect("the binary runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8(out.stderr.clone()).unwrap()
}

#[test]
fn every_key_the_workflow_reads_is_emitted_in_order() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), TARGETS);
    let out = run(dir.path(), "desert-tooling-v0.6.0");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "publish=true\n\
         version=0.6.0\n\
         zip=DesertTooling-0.6.0.zip\n\
         display_name=Desert Tooling (ASI)\n\
         category=main\n\
         update_mod_version=true\n\
         changelog=true\n\
         game_domain=crimsondesert\n\
         game_scoped_id=3369\n\
         file_id=7934145\n"
    );
}

#[test]
fn the_pack_gets_its_own_file_and_does_not_own_the_pages_version() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), TARGETS);
    let out = run(dir.path(), "desert-gatherer-dmm-v1.1");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // Longest prefix wins, or this would be the gatherer's tag with a version
    // of "dmm-v1.1". And the pack's x.y number must not drag the page's
    // version backwards, so update_mod_version is false while changelog - which
    // is additive and keyed by version string - stays true.
    assert!(stdout(&out).contains("zip=DesertGatherer-DMM-1.1.zip\n"));
    assert!(stdout(&out).contains("update_mod_version=false\n"));
    assert!(stdout(&out).contains("changelog=true\n"));
    assert!(stdout(&out).contains("file_id=7924209\n"));
}

#[test]
fn an_empty_file_id_is_the_documented_opt_out_and_not_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), &TARGETS.replace("\"7924209\"", "\"\""));
    let out = run(dir.path(), "desert-gatherer-dmm-v1.1");
    // The tag still cut a GitHub release; this is how a package opts out of
    // Nexus entirely, so it exits 0 and the upload steps skip.
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "publish=false\n");
    // The notice goes to stderr because stdout is $GITHUB_OUTPUT, and a
    // workflow command has to reach the log to be seen.
    assert_eq!(
        stderr(&out),
        "::notice::desert-gatherer-dmm has no Nexus target in nexus-targets.json, \
         so nothing was published to Nexus.\n"
    );
}

#[test]
fn two_claimants_for_the_pages_version_field_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), &TARGETS.replace("\"update_mod_version\": false", "\"update_mod_version\": true"));
    // One page, two files. Two releases overwriting each other's version on the
    // site is much harder to notice than a red step here.
    let out = run(dir.path(), "desert-tooling-v0.6.0");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "nexus-target: 2 targets set update_mod_version (desert-tooling, desert-gatherer-dmm); \
         at most one may.\n"
    );
}

#[test]
fn a_name_or_version_the_api_would_reject_fails_before_the_upload() {
    let dir = tempfile::tempdir().unwrap();
    // A 422 from Nexus arrives after the bytes are already in S3; this is the
    // same check, a second in and free.
    fixture(dir.path(), &TARGETS.replace("Desert Tooling (ASI)", "Desert Tooling [ASI]"));
    let out = run(dir.path(), "desert-tooling-v0.6.0");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("display name 'Desert Tooling [ASI]' is not accepted by Nexus"));

    fixture(dir.path(), &TARGETS.replace("\"main\"", "\"Main\""));
    let out = run(dir.path(), "desert-tooling-v0.6.0");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("category 'Main' is not one of main, optional, miscellaneous"));

    fixture(dir.path(), TARGETS);
    let out = run(dir.path(), "desert-tooling-v");
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("version '' is not accepted by Nexus"));
}

#[test]
fn a_tag_naming_no_configured_package_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), TARGETS);
    let out = run(dir.path(), "desert-looter-v0.1.2");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        "nexus-target: tag 'desert-looter-v0.1.2' names no package in nexus-targets.json\n"
    );
}

#[test]
fn the_targets_file_is_found_from_any_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path(), TARGETS);
    let out = run(&dir.path().join("tools"), "desert-tooling-v0.6.0");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("publish=true\n"));
}
