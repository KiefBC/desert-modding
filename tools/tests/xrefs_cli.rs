//! `xrefs` against the shipped CrimsonDesert.exe.
//!
//! Ignored by default; the game is not on a CI runner. The scanners themselves
//! are unit-tested offline over hand-built fixtures in `src/bin/xrefs/scan.rs`,
//! including the rel32 and RIP-relative arithmetic where an off-by-one hides.
//! These are the known answers on the real binary, recorded on build 25477059
//! so that a game update that moves them says so out loud.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Addresses on build 25477059, chosen to cover every branch of the report.
///
/// Re-derived from 25246367 by what each one IS, not by where it was: the
/// functions through their `reference-signatures.txt` signatures (`sigscan`),
/// the literal through its bytes, and `NEITHER` from the exception directory.
/// Every count came out the same as on 25246367. Only `NEITHER` is a fresh
/// pick: its old address was no named function there was anything to follow.
const CODE_XREFS: &str = "17fa220"; // `get_pos`: six calls, no cells (25246367: 1764420)
const MANY_XREFS: &str = "1439060"; // `alloc_event`, hot: 65 calls (25246367: 13aaa80)
const ONE_XREF: &str = "387bda0"; // `area_sweep`, reached exactly once (25246367: 3796520)
/// A real function with neither - see below. 25246367 used 2a73c20, which on
/// 25477059 is inside the body of the function at 0x2A73AC0 (`.pdata`
/// 0x2A73AC0..0x2A73D39). This one is a `.pdata` function start
/// (0x2AFFBF0..0x2AFFF48) behind int3 padding with a full prologue, and the
/// only int3-padded start in 0x2A00000..0x2C00000 that no rel32, RIP-relative
/// operand or 8-byte VA anywhere in the image reaches (a numpy pass over the
/// loaded image, then this tool and its `--selfcheck` to confirm).
const NEITHER: &str = "2affbf0";
/// `SetAdditionalCollectDropRate`, the short-name literal (25246367: 569C7C0).
const POINTER_ONLY: &str = "57AFC28";

fn exe() -> PathBuf {
    match std::env::var_os("EXE") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe"),
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xrefs"))
        .args(args)
        .output()
        .expect("cannot run the xrefs binary")
}

fn stdout(args: &[&str]) -> String {
    let out = run(args);
    assert_eq!(out.status.code(), Some(0), "xrefs {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Every known answer on build 25477059, in ONE test.
///
/// Deliberately not six tests: each run holds the 385 MB image, and six of them
/// in parallel is enough to take this machine into ENOMEM while anything else
/// is building. Splitting them looked tidier and failed for reasons that had
/// nothing to do with the code.
#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn the_known_answers_on_this_build() {
    assert!(exe().is_file(), "CrimsonDesert.exe not found at {}", exe().display());

    // A function entry: six calls reach it and nothing holds its address.
    let s = stdout(&[CODE_XREFS]);
    // The caller at get_pos-0x117 is the same site on both builds: +0x1764309
    // on 25246367, +0x17FA109 on 25477059.
    assert!(s.contains("6 code reference(s) and 0 pointer cell(s) to +0x17FA220"), "{s}");
    assert_eq!(s.lines().filter(|l| l.starts_with("call ")).count(), 6);
    assert!(s.contains("call           at +0x17FA109"), "{s}");

    // The rest of the spread: many, exactly one, and none at all.
    assert!(stdout(&[MANY_XREFS]).contains("65 code reference(s) and 0 pointer cell(s)"));
    assert!(stdout(&[ONE_XREF]).contains("1 code reference(s) and 0 pointer cell(s)"));

    // Nothing pointing here is not a failure and not an empty report: the
    // summary line is always printed, because "nothing" is an answer.
    assert_eq!(
        stdout(&[NEITHER]),
        "0 code reference(s) and 0 pointer cell(s) to +0x2AFFBF0\n"
    );

    // `SetAdditionalCollectDropRate`, the worked example in tools/README.md.
    // A scanner that reported only code references would print two coincidental
    // `lea`s into the middle of a string and nothing that identifies the
    // address at all. The pointer cell is the whole answer. Build 25477059;
    // on 25246367 the cell was +0x5861BF8 holding VA 0x14569C7C0.
    let s = stdout(&[POINTER_ONLY]);
    assert!(
        s.contains("ptr cell       at +0x59792A8  (holds VA 0x1457AFC28)"),
        "the pointer cell is gone:\n{s}"
    );
    assert!(s.contains("2 code reference(s) and 1 pointer cell(s) to +0x57AFC28"), "{s}");

    // An explicit exe path is accepted, and a flag is never mistaken for one:
    // `xrefs <rva> --selfcheck` must not try to open a file called
    // `--selfcheck`. The Python filters `--` arguments out of the positional
    // list for exactly that; so does the port.
    assert_eq!(stdout(&[ONE_XREF, &exe().display().to_string()]), stdout(&[ONE_XREF]));
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn the_fast_scan_agrees_with_the_byte_loop() {
    // What `--selfcheck` is for: after changing the scanner, and after a game
    // update when a result looks wrong.
    for a in [CODE_XREFS, POINTER_ONLY, NEITHER] {
        let out = run(&[a, "--selfcheck"]);
        assert_eq!(out.status.code(), Some(0), "selfcheck failed for {a}");
        let s = String::from_utf8_lossy(&out.stdout);
        assert!(s.starts_with("selfcheck ok: fast scan agrees with the byte loop on "), "{s}");
    }
}

#[test]
fn with_no_arguments_it_prints_the_usage_and_exits_two() {
    // Offline: it never opens the exe on this path. Exit 2, not 0 and not 1,
    // because `xrefs` in a pipeline should be distinguishable from a scan that
    // found nothing (0) and from one that failed (1).
    let out = run(&[]);
    assert_eq!(out.status.code(), Some(2));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.starts_with("Find references to an RVA"), "{s}");
    assert!(s.contains("pointer cells"), "the pointer-cell warning is the point: {s}");
}

#[test]
fn a_bad_address_is_an_error_not_a_scan_of_zero() {
    let out = run(&["not-hex"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a hexadecimal RVA"));
}
