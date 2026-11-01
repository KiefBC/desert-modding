//! The `check-imports` binary, end to end.
//!
//! `pe_imports.rs` already covers the import reader itself. What is left to
//! check here is the part CI depends on and a unit test cannot see: the exit
//! codes. `just ci` and the CI workflow only ever look at the status, so a
//! version of this tool that printed the right complaint and still exited 0
//! would let libstdc++-6.dll back into the shipped .asi without anyone
//! noticing - which is the exact failure the tool exists to prevent.
//!
//! The 2-versus-1 split matters for the same reason: an empty `allowed_imports`
//! in the justfile would otherwise look like a clean pass of an empty
//! allowlist.

use std::path::PathBuf;
use std::process::{Command, Output};

/// The justfile's `allowed_imports`, read from the justfile rather than copied,
/// so this test cannot drift away from what `just check-imports` actually runs.
fn allowlist() -> Vec<String> {
    let root = desert_tools::paths::repo_root().unwrap();
    let justfile = std::fs::read_to_string(root.join("justfile")).unwrap();
    let line = justfile
        .lines()
        .find(|l| l.starts_with("allowed_imports"))
        .expect("justfile has no allowed_imports");
    let value = line.split('"').nth(1).expect("allowed_imports is not a quoted string");
    value.split_whitespace().map(str::to_string).collect()
}

fn run(args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_check-imports"))
        .args(args)
        .output()
        .expect("cannot run check-imports")
}

fn built_plugin() -> Option<PathBuf> {
    let p = desert_tools::paths::repo_root()
        .ok()?
        .join("target/x86_64-pc-windows-gnu/release/desert_tooling.dll");
    p.is_file().then_some(p)
}

#[test]
fn an_empty_allowlist_is_a_caller_bug_not_a_pass() {
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2), "no allowlist must exit 2, not 0 or 1");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("no allowlist given"), "stderr was {err:?}");
}

#[test]
fn the_justfile_allowlist_is_read_as_a_space_separated_list() {
    // Not a tautology: the justfile passes `{{allowed_imports}}` unquoted, so
    // the tool receives one argument per name. If that variable were ever
    // rewritten as a comma-separated string the split would silently yield one
    // long "name" that matches nothing.
    let allowed = allowlist();
    assert!(allowed.len() > 5, "suspiciously short allowlist: {allowed:?}");
    assert!(allowed.iter().any(|n| n == "kernel32"));
    for name in &allowed {
        assert_eq!(*name, name.to_ascii_lowercase(), "{name} is not lowercase");
        assert!(!name.ends_with(".dll"), "{name} kept its suffix");
    }
}

#[test]
#[ignore = "needs `just build` to have produced the Windows DLL"]
fn the_shipped_plugin_passes_the_justfile_allowlist() {
    assert!(built_plugin().is_some(), "no built desert_tooling.dll - run `just build` first");
    let out = run(&allowlist());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "stdout {stdout:?} stderr {stderr:?}");
    assert!(stdout.contains("desert_tooling.dll ok ("), "stdout was {stdout:?}");
}

#[test]
#[ignore = "needs `just build` to have produced the Windows DLL"]
fn an_import_off_the_list_fails_and_is_named() {
    assert!(built_plugin().is_some(), "no built desert_tooling.dll - run `just build` first");
    // kernel32 alone: whatever else the plugin links, the rest must be
    // reported. A tool that only counted offenders, or only reported the
    // first, would leave a CI log that does not say what to fix.
    let out = run(&["kernel32".to_string()]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("imports non-system DLL(s):"), "stderr was {err:?}");
    assert!(err.contains(" ntdll"), "ntdll should have been named: {err:?}");
    assert!(!err.contains(" kernel32"), "kernel32 was allowed: {err:?}");
}
