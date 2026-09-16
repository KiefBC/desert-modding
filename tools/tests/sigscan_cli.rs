//! `sigscan` against the shipped CrimsonDesert.exe.
//!
//! Ignored by default: the game is not on a CI runner, for the same reason
//! `tests/pe_real_exe.rs` is ignored and `just sigscan` is not in `just ci`.
//! `cargo test -- --ignored` runs them.
//!
//! The matcher's own edge cases are unit-tested offline inside
//! `src/bin/sigscan.rs`. What is left for here is the thing a fixture cannot
//! check: that the signatures still bind to exactly one place in the REAL
//! binary, which is the entire point of the tool.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn exe() -> PathBuf {
    match std::env::var_os("EXE") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe"),
    }
}

/// Run `sigscan` with `cwd` as the working directory.
fn run_in(cwd: &Path) -> (String, Option<i32>) {
    let out = Command::new(env!("CARGO_BIN_EXE_sigscan"))
        .current_dir(cwd)
        .output()
        .expect("cannot run the sigscan binary");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code())
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn every_signature_still_hits_exactly_once() {
    assert!(exe().is_file(), "CrimsonDesert.exe not found at {}", exe().display());
    let (stdout, code) = run_in(&repo_root());
    assert_eq!(code, Some(0));

    // The signature block is the lines between `--- signatures ---` and the
    // blank line before `--- names ---`; each begins with a right-aligned hit
    // count. More than one hit means the pattern is no longer unique and a
    // plugin resolving it at load could bind to the wrong function - which is
    // a crash in the game, found by a player, not by a test.
    let block = stdout
        .split("--- signatures ---")
        .nth(1)
        .and_then(|s| s.split("--- names ---").next())
        .expect("no signature block in the output");
    let sigs: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();

    // Every signature in the file must be reported - counted from the file
    // itself, never pinned to a literal. This assertion used to say `9`, which
    // was right until desert-looter's hunting hook added a tenth, and then a
    // legitimate signature addition failed a test about uniqueness. Reading
    // the file keeps the property that matters: a pattern the scanner silently
    // dropped never gets its hit count checked at all.
    let text = std::fs::read_to_string(repo_root().join("tools/reference-signatures.txt"))
        .expect("cannot read reference-signatures.txt");
    let expected = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .count();
    assert_eq!(sigs.len(), expected, "{expected} signatures in the file, {} reported", sigs.len());
    for line in sigs {
        let count: usize = line[..3].trim().parse().unwrap_or_else(|_| panic!("no count in {line:?}"));
        assert_eq!(count, 1, "signature is no longer unique: {line}");
        assert!(line.contains(" @ 0x"), "a hit must report where: {line}");
    }
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn the_static_info_registry_is_unchanged() {
    let (stdout, _) = run_in(&repo_root());
    assert!(
        stdout.contains("216/216 type names still occur exactly once"),
        "static-info name check changed:\n{}",
        stdout.split("--- static-info type names ---").nth(1).unwrap_or("(missing)")
    );
    assert!(stdout.contains("the static-info type registry is unchanged"));
    assert!(!stdout.contains("MISSING"));
    assert!(!stdout.contains("HITS"));
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn a_substring_name_and_a_nul_delimited_one_answer_differently() {
    // `iteminfo` occurs twice as a substring, because `trademarketiteminfo`
    // contains it, and once as a NUL-delimited literal. Both numbers are in
    // the output at the same time, and they are supposed to disagree. This is
    // the check that stops someone "fixing" one matcher into the other.
    let (stdout, _) = run_in(&repo_root());
    let line = stdout
        .lines()
        .find(|l| l.contains("iteminfo ") && l.contains("first @"))
        .expect("no iteminfo line");
    assert_eq!(line[..4].trim(), "2", "substring hits: {line}");
    assert!(stdout.contains("216/216"), "and all 216 are NUL-unique");
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn the_data_files_are_found_from_any_working_directory() {
    // `just sigscan` runs the binary FROM THE REPO ROOT, never from `tools/`.
    // A relative `tools/reference-signatures.txt` would work there and nowhere
    // else; the lookup goes through `paths::repo_root()` for that reason. Run
    // it from three directories and require identical output.
    let root = run_in(&repo_root()).0;
    let tools = run_in(Path::new(env!("CARGO_MANIFEST_DIR"))).0;
    assert_eq!(root, tools, "output differs between the repo root and tools/");

    // And from outside the checkout entirely, where `repo_root()` falls back
    // to the compiled-in manifest directory.
    let outside = run_in(Path::new("/")).0;
    assert_eq!(root, outside, "output differs when run from outside the repo");
    assert!(root.contains("--- static-info type names ---"));
}
