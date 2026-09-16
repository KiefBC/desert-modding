//! The `dist` binary, end to end.
//!
//! Nearly everything here is `#[ignore]`d, and for a blunter reason than the
//! other ignored tests in this directory: `dist` DELETES `dist/` before it
//! writes. A test that ran on a bare `cargo test` would throw away the
//! archives somebody was halfway through uploading. The two that do run are
//! the ones with no side effects at all - and one of them exists precisely to
//! prove that the unknown-package path is side-effect-free, because that is
//! the path a mistyped release tag takes.
//!
//! What the ignored ones check is what a release depends on and nothing else
//! can see:
//!
//!   * two runs give byte-identical archives, including with `TZ` set west of
//!     UTC - the exact case the DIST_EPOCH comment in `dist/archive.rs` is
//!     about, and the one a 1980 stamp underflows in;
//!   * the entry order, names, stamps and modes are the ones `dist.sh` wrote,
//!     because DMM reads `dmm_pack.json` only at the zip root and a reordered
//!     or renamed entry is a broken install, not a cosmetic change;
//!   * `SHA256SUMS` is what `sha256sum --check --strict` parses, and says what
//!     coreutils says. The release workflow runs that check; a format this
//!     tool alone agrees with would fail after the tag is already pushed.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Every test here spends the same `dist/`, and `cargo test` runs them on
/// threads. Without this they wipe each other's output halfway through and
/// fail in ways that look like non-determinism in the tool - which is the one
/// property most of them exist to measure. Poisoning is ignored deliberately:
/// a failed test still leaves `dist/` in a state the next one rebuilds from
/// scratch anyway.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn root() -> PathBuf {
    desert_tools::paths::repo_root().unwrap()
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dist"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("cannot run dist")
}

fn run(args: &[&str]) -> Output {
    run_in(&root(), args)
}

/// The names in `dist/`, sorted, or `None` when there is no `dist/` at all.
fn dist_listing() -> Option<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(root().join("dist"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Some(names)
}

fn pack_zip() -> PathBuf {
    let version: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("desert-gatherer-dmm/dmm_pack.json")).unwrap(),
    )
    .unwrap();
    let version = version["version"].as_str().unwrap().to_string();
    root().join(format!("dist/DesertGatherer-DMM-{version}.zip"))
}

#[test]
fn an_unknown_package_is_rejected_before_anything_is_deleted() {
    let _dist = exclusive();
    // A mistyped tag reaching the release workflow must not cost the previous
    // build. `dist.sh` validated first for the same reason; this is the test
    // that would notice the order being swapped.
    let before = dist_listing();
    let out = run(&["desert-tooling-typo"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown package 'desert-tooling-typo'"), "stderr was {err:?}");
    assert!(
        err.contains("desert-tooling") && err.contains("desert-gatherer-dmm"),
        "the message must name the valid packages: {err:?}"
    );
    assert_eq!(dist_listing(), before, "dist/ changed on the error path");
}

#[test]
fn the_pack_ships_exactly_twelve_modules() {
    // The sanity gate `dist.sh` carried, checked against the repository rather
    // than through a build: the pack is twelve multipliers, and a half-landed
    // thirteenth (or a deleted twelfth) must fail the release rather than ship
    // a pack DMM will show with a hole in it.
    let src = root().join("desert-gatherer-dmm");
    let modules: Vec<String> = std::fs::read_dir(&src)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(" - ") && n.ends_with("X.json"))
        .collect();
    assert_eq!(modules.len(), 12, "found {modules:?}");
}

#[test]
#[ignore = "wipes and rebuilds dist/"]
fn the_pack_zip_is_reproducible_and_ignores_the_local_timezone() {
    let _dist = exclusive();
    assert!(run(&["desert-gatherer-dmm"]).status.success());
    let first = std::fs::read(pack_zip()).unwrap();

    assert!(run(&["desert-gatherer-dmm"]).status.success());
    let second = std::fs::read(pack_zip()).unwrap();
    assert_eq!(first, second, "two runs disagreed");

    // West of UTC is where a DOS timestamp built from a 1980 epoch underflows,
    // and where the shell version silently stamped a different date. The
    // archive must not be able to tell.
    let out = Command::new(env!("CARGO_BIN_EXE_dist"))
        .arg("desert-gatherer-dmm")
        .current_dir(root())
        .env("TZ", "America/Los_Angeles")
        .output()
        .unwrap();
    assert!(out.status.success());
    let west = std::fs::read(pack_zip()).unwrap();
    assert_eq!(first, west, "TZ reached the archive");
}

#[test]
#[ignore = "wipes and rebuilds dist/"]
fn the_pack_zip_has_the_layout_dmm_needs() {
    let _dist = exclusive();
    assert!(run(&["desert-gatherer-dmm"]).status.success());
    let file = std::fs::File::open(pack_zip()).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();

    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    // Flat, manifest first: DMM reads a pack's title and description only when
    // dmm_pack.json is at the zip ROOT, so a wrapper folder would leave the
    // pack card showing the folder name instead.
    assert_eq!(names[0], "dmm_pack.json");
    assert!(names.iter().all(|n| !n.contains('/')), "not flat: {names:?}");
    assert_eq!(names.len(), 16, "{names:?}");
    // Twelve modules in byte order, then the docs and the rebaser.
    assert_eq!(&names[13..], ["README.md", "VERIFICATION.txt", "rebase.py"]);
    let modules = &names[1..13];
    assert!(modules.windows(2).all(|w| w[0] < w[1]), "modules out of order: {modules:?}");

    for i in 0..zip.len() {
        let entry = zip.by_index(i).unwrap();
        let t = entry.last_modified().unwrap();
        assert_eq!(
            (t.year(), t.month(), t.day(), t.hour(), t.minute(), t.second()),
            (2020, 1, 1, 0, 0, 0),
            "{} carries a stamp that is not DIST_EPOCH",
            entry.name()
        );
        // 0o644, as `install -m 644` gave every staged file. The umask must
        // not reach the archive.
        assert_eq!(
            entry.unix_mode().map(|m| m & 0o777),
            Some(0o644),
            "{} has the wrong mode",
            entry.name()
        );
    }
}

#[test]
#[ignore = "wipes and rebuilds dist/"]
fn sha256sums_is_what_coreutils_and_the_release_workflow_expect() {
    let _dist = exclusive();
    assert!(run(&["desert-gatherer-dmm"]).status.success());
    let dist = root().join("dist");
    let sums = std::fs::read_to_string(dist.join("SHA256SUMS")).unwrap();

    // `<64 hex>  <bare name>` - two spaces, and no `./` prefix, which is what
    // the shell version's `sed 's| \./| |'` was for. The workflow runs
    // `sha256sum --check --strict` from inside dist/, so a prefixed name is a
    // different path from the one being verified.
    for line in sums.lines() {
        let (digest, name) = line.split_once("  ").expect("two spaces separate the fields");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(!name.starts_with("./"), "{line:?}");
        assert!(dist.join(name).is_file(), "{name} is not in dist/");
    }

    // And the digests are right, checked against the implementation everyone
    // else's `sha256sum --check` will use.
    let out = Command::new("sha256sum")
        .arg("DesertGatherer-DMM-1.1.zip")
        .current_dir(&dist)
        .output()
        .expect("coreutils sha256sum");
    assert!(out.status.success());
    let coreutils = String::from_utf8_lossy(&out.stdout);
    assert!(
        sums.lines().any(|l| l == coreutils.trim_end()),
        "ours:\n{sums}\ncoreutils:\n{coreutils}"
    );

    let check = Command::new("sha256sum")
        .args(["--check", "--strict", "SHA256SUMS"])
        .current_dir(&dist)
        .status()
        .expect("coreutils sha256sum");
    assert!(check.success(), "sha256sum --check --strict rejected our file");
}

#[test]
#[ignore = "wipes and rebuilds dist/"]
fn the_working_directory_does_not_decide_which_dist_is_deleted() {
    let _dist = exclusive();
    // The justfile and the release workflow run this from the repo root, but
    // nothing enforces that, and a cwd-relative `dist` would delete or create
    // the wrong directory. Every path goes through `paths::repo_root()`.
    let elsewhere = tempfile::tempdir().unwrap();
    let out = run_in(elsewhere.path(), &["desert-gatherer-dmm"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(pack_zip().is_file(), "the zip did not land in the repo's dist/");
    assert!(
        !elsewhere.path().join("dist").exists(),
        "a stray dist/ was created beside the working directory"
    );
}

#[test]
#[ignore = "wipes and rebuilds dist/, and needs `just build` for the Windows DLL"]
fn the_plugin_zip_is_flat_and_carries_the_whole_install() {
    let _dist = exclusive();
    let out = run(&["desert-tooling"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let version = std::fs::read_to_string(root().join("desert-tooling/Cargo.toml"))
        .unwrap()
        .lines()
        .find(|l| l.starts_with("version"))
        .and_then(|l| l.split('"').nth(1).map(str::to_string))
        .unwrap();
    let path = root().join(format!("dist/DesertTooling-{version}.zip"));

    let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().to_string())
        .collect();
    // FLAT and in this order: extracting straight into bin64 must leave the
    // .asi next to winmm.dll with nothing to move afterwards, and DMM accepts
    // the archive because it sees the .asi immediately.
    assert_eq!(
        names,
        [
            "DesertTooling.asi",
            "DesertTooling.ini",
            "README.md",
            "CHANGELOG.md",
            "LICENSE"
        ]
    );

    // The .asi is the built DLL under its shipped name - the rename the shell
    // version needed a staging directory for.
    let built = std::fs::read(root().join("target/x86_64-pc-windows-gnu/release/desert_tooling.dll"))
        .expect("no built desert_tooling.dll - run `just build` first");
    let mut packed = Vec::new();
    std::io::copy(&mut zip.by_name("DesertTooling.asi").unwrap(), &mut packed).unwrap();
    assert_eq!(packed, built, "the packed .asi is not the built DLL");

    // Only this package's zip: a release tagged for one package must not
    // publish the other one's stale version number over new bytes.
    assert_eq!(
        dist_listing().unwrap(),
        [format!("DesertTooling-{version}.zip"), "SHA256SUMS".to_string()]
    );
}
