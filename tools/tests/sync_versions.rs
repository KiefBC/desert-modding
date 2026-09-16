//! End-to-end tests for the `sync-versions` binary.
//!
//! Every one of these runs the real binary against a synthetic repository in a
//! temp directory, because what the tool is for is its exit code and its
//! report: `just check-versions` and both GitHub workflows read nothing else.
//! A unit test of the substitution would not have caught the two things that
//! actually matter here - that a missing changelog entry fails in BOTH modes,
//! and that `--check` never writes.

use std::path::Path;
use std::process::{Command, Output};

const CRATES: [(&str, &str); 6] = [
    ("desert-tooling", "0.6.0"),
    ("desert-looter", "0.3.1"),
    ("desert-gatherer", "0.3.1"),
    ("desert-overlay", "0.3.0"),
    ("desert-dispatch", "0.3.0"),
    ("desert-core", "0.7.0"),
];

/// The smallest tree `sync-versions` will run against: the two markers
/// `paths::repo_root` looks for, one manifest per crate, the docs it rewrites
/// and the DMM pack.
fn fixture(root: &Path) {
    let write = |rel: &str, body: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    write("justfile", "");
    write("flake.nix", "");

    for (crate_name, version) in CRATES {
        write(
            &format!("{crate_name}/Cargo.toml"),
            // A dependency carrying its own `version =` is the trap the
            // [package]-section-only search exists for.
            &format!(
                "[package]\nname = \"{crate_name}\"\nversion = \"{version}\"\n\n\
                 [dependencies]\nanyhow = {{ version = \"9.9.9\" }}\n"
            ),
        );
    }

    write(
        "README.md",
        "| Crate | Version | What |\n| --- | --- | --- |\n\
         | [desert-tooling](desert-tooling) | 0.6.0 | the plugin |\n",
    );
    write(
        "VERSIONING.md",
        "| `desert-core` | 0.7.0 | nothing | never |\n\n\
         Then: `git push origin desert-tooling-v0.6.0`\n",
    );
    write(
        "desert-tooling/README.md",
        "# Desert Tooling\n\n**Version 0.6.0**, for the game.\n",
    );
    write(
        "desert-tooling/CHANGELOG.md",
        "# Changelog\n\n## [0.6.0] - 2026-09-13\n\n- something\n",
    );

    write("desert-gatherer-dmm/dmm_pack.json", "{\n  \"version\": \"1.1\"\n}\n");
    // CRLF, exactly as the twelve real module files are in git.
    write(
        "desert-gatherer-dmm/Foraging - 2X.json",
        "{\r\n  \"version\": \"1.1\",\r\n  \"modinfo\": {\r\n    \"version\": \"1.1\"\r\n  }\r\n}\r\n",
    );
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sync-versions"))
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
fn a_tree_in_sync_says_so_and_exits_zero() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let out = run(dir.path(), &["--check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "sync-versions: already in sync (desert-tooling 0.6.0, desert-looter 0.3.1, \
         desert-gatherer 0.3.1, desert-overlay 0.3.0, desert-dispatch 0.3.0, \
         desert-core 0.7.0, DMM pack 1.1)\n"
    );
}

#[test]
fn check_names_every_stale_file_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let readme = dir.path().join("README.md");
    let before = std::fs::read_to_string(&readme).unwrap();
    std::fs::write(&readme, before.replace("0.6.0", "0.1.0")).unwrap();

    let out = run(dir.path(), &["--check"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("sync-versions: out of date: README.md"), "{err}");
    assert!(err.contains("run `just sync-versions` and commit the result"), "{err}");
    // The whole point of --check: CI must not leave a dirty tree behind.
    assert_eq!(
        std::fs::read_to_string(&readme).unwrap(),
        before.replace("0.6.0", "0.1.0")
    );
}

#[test]
fn the_rewrite_fixes_the_table_the_version_line_and_the_tag_example() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    for (rel, from, to) in [
        ("README.md", "0.6.0", "0.1.0"),
        ("VERSIONING.md", "0.7.0", "0.1.0"),
        ("VERSIONING.md", "desert-tooling-v0.6.0", "desert-tooling-v0.1.0"),
        ("desert-tooling/README.md", "**Version 0.6.0**", "**Version 0.1.0**"),
    ] {
        let path = dir.path().join(rel);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace(from, to)).unwrap();
    }

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("sync-versions: updated README.md, VERSIONING.md, desert-tooling/README.md"));

    assert!(std::fs::read_to_string(dir.path().join("README.md"))
        .unwrap()
        .contains("| 0.6.0 |"));
    assert!(std::fs::read_to_string(dir.path().join("VERSIONING.md"))
        .unwrap()
        .contains("desert-tooling-v0.6.0"));
    assert!(std::fs::read_to_string(dir.path().join("desert-tooling/README.md"))
        .unwrap()
        .contains("**Version 0.6.0**"));

    // Idempotent: a second run has nothing to do.
    let again = run(dir.path(), &[]);
    assert!(stdout(&again).contains("already in sync"));
}

#[test]
fn a_pack_bump_rewrites_both_versions_and_keeps_the_files_crlf() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    let manifest = dir.path().join("desert-gatherer-dmm/dmm_pack.json");
    std::fs::write(&manifest, "{\n  \"version\": \"1.2\"\n}\n").unwrap();

    let out = run(dir.path(), &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("DMM pack 1.2"));

    let module = std::fs::read_to_string(dir.path().join("desert-gatherer-dmm/Foraging - 2X.json")).unwrap();
    // Both the module's own version and the one inside modinfo.
    assert_eq!(module.matches("\"1.2\"").count(), 2);
    // And the line endings the file came with. The Python read these through
    // universal newlines and wrote them back as LF, turning a two-line edit
    // into a whole-file rewrite; that is the one deliberate difference here.
    assert_eq!(module.matches("\r\n").count(), 6);
}

#[test]
fn a_bump_with_no_changelog_entry_fails_in_both_modes() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    // VERSIONING.md step 3: the entry lands in the same commit as the bump.
    // Nothing else notices a missing one until the tag has been pushed, which
    // is too late, so both modes have to go red - `just sync-versions` in
    // particular must not exit 0 having "finished".
    let manifest = dir.path().join("desert-tooling/Cargo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, text.replace("0.6.0", "0.7.0")).unwrap();

    for args in [vec!["--check"], vec![]] {
        let out = run(dir.path(), &args);
        assert_eq!(out.status.code(), Some(1), "args {args:?}");
        assert!(
            stderr(&out).contains("desert-tooling/CHANGELOG.md has no `## [0.7.0]` entry"),
            "args {args:?}: {}",
            stderr(&out)
        );
        assert!(stderr(&out).contains("(VERSIONING.md step 3)"));
    }
}

#[test]
fn a_manifest_with_no_package_version_is_named() {
    let dir = tempfile::tempdir().unwrap();
    fixture(dir.path());
    std::fs::write(
        dir.path().join("desert-core/Cargo.toml"),
        "[package]\nname = \"desert-core\"\n",
    )
    .unwrap();
    let out = run(dir.path(), &["--check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("sync-versions: no package version in"));
    assert!(stderr(&out).contains("desert-core/Cargo.toml"));
}
