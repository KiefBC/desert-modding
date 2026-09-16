//! End-to-end tests for the `release-notes` binary.
//!
//! `.github/workflows/release.yml` pipes this into `gh release create` and into
//! `$GITHUB_OUTPUT`, so the tests are over stdout and the exit code rather than
//! over any internal function. The two refusals are the important ones: both
//! run before anything is built, and a tag is a release - once it is pushed the
//! only fix is a second tag.

use std::path::Path;
use std::process::{Command, Output};

fn fixture(root: &Path) {
    let write = |rel: &str, body: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    write("justfile", "");
    write("flake.nix", "");
    write(
        "desert-tooling/Cargo.toml",
        "[package]\nname = \"desert-tooling\"\nversion = \"0.6.0\"\n\n\
         [dependencies]\nanyhow = { version = \"9.9.9\" }\n",
    );
    write(
        "desert-tooling/CHANGELOG.md",
        "# Changelog\n\nSome preamble that mentions ## [0.6.0] mid-line.\n\n\
         ## [0.6.0] - 2026-09-13\n\n### Added\n\n- a thing\n\n## [0.5.0] - 2026-09-01\n\n- an older thing\n",
    );
    write("desert-gatherer-dmm/dmm_pack.json", "{\n  \"version\": \"1.1\"\n}\n");
    write(
        "desert-gatherer-dmm/README.md",
        "# The pack\n\n## Installation\n\nUnzip it.\n\n## What Version\n\nBuilt against build 25116796.\n\n## Verified coverage\n\n- lots\n",
    );
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_release-notes"))
        .args(args)
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
fn the_title_and_the_package_name_are_what_the_workflow_asks_for() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    // `--package` feeds the `dist` tool, so it is the tag prefix and nothing else.
    let out = run(dir.path(), &["desert-tooling-v0.6.0", "--package"]);
    assert_eq!(stdout(&out), "desert-tooling\n");
    let out = run(dir.path(), &["desert-tooling-v0.6.0", "--title"]);
    assert_eq!(stdout(&out), "Desert Tooling 0.6.0\n");
    let out = run(dir.path(), &["desert-gatherer-dmm-v1.1", "--package"]);
    assert_eq!(stdout(&out), "desert-gatherer-dmm\n");
    let out = run(dir.path(), &["desert-gatherer-dmm-v1.1", "--title"]);
    assert_eq!(stdout(&out), "Desert Gatherer DMM pack 1.1\n");
}

#[test]
fn the_changelog_entry_stops_at_the_next_heading() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = run(dir.path(), &["desert-tooling-v0.6.0", "--changelog"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    // Nothing from 0.5.0, and nothing from the preamble: the heading match is
    // anchored, so a `## [0.6.0]` inside a sentence does not open the section.
    assert_eq!(stdout(&out), "### Added\n\n- a thing\n");

    // The pack keeps its history in one standing README section instead.
    let out = run(dir.path(), &["desert-gatherer-dmm-v1.1", "--changelog"]);
    assert_eq!(stdout(&out), "Built against build 25116796.\n");
}

#[test]
fn changelog_mode_carries_no_file_list_and_no_checksums() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let sums = dir.path().join("SHA256SUMS");
    std::fs::write(&sums, "abc123  DesertTooling-0.6.0.zip\n").unwrap();
    // What goes to the Nexus page: the entry alone. The page's Files tab
    // already lists what is attached, so the table would be noise there.
    let out = run(
        dir.path(),
        &["desert-tooling-v0.6.0", "--changelog", "--sums", sums.to_str().unwrap()],
    );
    assert_eq!(stdout(&out), "### Added\n\n- a thing\n");
    assert!(!stdout(&out).contains("## Files"));
}

#[test]
fn the_release_body_is_the_entry_then_the_files_then_the_sums() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = run(dir.path(), &["desert-tooling-v0.6.0"]);
    assert_eq!(
        stdout(&out),
        "### Added\n\n- a thing\n\n## Files\n\n\
         Desert Tooling 0.6.0 only, built from the tagged commit. The other packages in this \
         repository are versioned separately and each has its own tag and its own releases.\n"
    );

    let sums = dir.path().join("SHA256SUMS");
    std::fs::write(&sums, "abc123  DesertTooling-0.6.0.zip\ndef456  other.zip\n").unwrap();
    let out = run(dir.path(), &["desert-tooling-v0.6.0", "--sums", sums.to_str().unwrap()]);
    assert!(stdout(&out).ends_with(
        "| File | SHA256 |\n| --- | --- |\n\
         | `DesertTooling-0.6.0.zip` | `abc123` |\n| `other.zip` | `def456` |\n\
         | `SHA256SUMS` | the list above, for `sha256sum --check` |\n"
    ), "{}", stdout(&out));
}

#[test]
fn a_tag_that_disagrees_with_the_source_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    // The release page would say one number and the log's first line another.
    let out = run(dir.path(), &["desert-tooling-v0.9.9", "--title"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("tag desert-tooling-v0.9.9 says 0.9.9, but the source says 0.6.0"));
    assert!(stderr(&out).contains("never release the mismatch"));

    let out = run(dir.path(), &["desert-gatherer-dmm-v9.9", "--title"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("but the source says 1.1"));
}

#[test]
fn a_version_with_no_changelog_heading_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let manifest = dir.path().join("desert-tooling/Cargo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replace("0.6.0", "0.7.0")).unwrap();
    // This is why a package cannot be released while its changelog still says
    // `## [Unreleased]`.
    let out = run(dir.path(), &["desert-tooling-v0.7.0"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("no heading matching"), "{}", stderr(&out));
    assert!(stderr(&out).contains("desert-tooling/CHANGELOG.md"));
    // But the modes that never open the changelog still work, which is what
    // `--package` is for: `dist` runs before the notes are built.
    assert_eq!(run(dir.path(), &["desert-tooling-v0.7.0", "--package"]).status.code(), Some(0));
}

#[test]
fn an_empty_section_is_refused_too() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("desert-gatherer-dmm/README.md"),
        "# The pack\n\n## What Version\n\n## Verified coverage\n\n- lots\n",
    )
    .unwrap();
    let out = run(dir.path(), &["desert-gatherer-dmm-v1.1"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("empty section under"), "{}", stderr(&out));
}

#[test]
fn a_tag_naming_no_package_lists_the_ones_that_exist() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    for tag in ["v1.0.0", "desert-looter-v0.1.2", "desert-tooling-v0.6", "desert-gatherer-dmm-v1.1.0"] {
        let out = run(dir.path(), &[tag]);
        assert_eq!(out.status.code(), Some(1), "{tag}");
        assert!(
            stderr(&out).contains("is not one of desert-tooling-v<version>, desert-gatherer-dmm-v<version>"),
            "{tag}: {}",
            stderr(&out)
        );
    }
}

#[test]
fn the_tree_is_found_from_any_working_directory() {
    // `just` runs these from the repo root and a person runs them from
    // wherever they are; both must read the same sources of truth.
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let deep = dir.path().join("desert-gatherer-dmm");
    for cwd in [dir.path(), &deep] {
        let out = run(cwd, &["desert-tooling-v0.6.0", "--title"]);
        assert_eq!(stdout(&out), "Desert Tooling 0.6.0\n", "cwd {}", cwd.display());
    }
}
