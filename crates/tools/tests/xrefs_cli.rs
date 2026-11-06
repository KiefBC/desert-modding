//! `xrefs` against the shipped CrimsonDesert.exe.
//!
//! Ignored by default; the game is not on a CI runner. The scanners themselves
//! are unit-tested offline over hand-built fixtures in `src/scan.rs`,
//! including the rel32 and RIP-relative arithmetic where an off-by-one hides.
//! These are the known answers on the real binary, recorded on build 26480954
//! so that a game update that moves them says so out loud.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Addresses on build 26480954, chosen to cover every branch of the report.
///
/// Re-derived from 25477059 by what each one IS, not by where it was: the
/// functions through their `reference-signatures.txt` signatures (`sigscan`,
/// converted to RVAs with `retarget`), the literal through its bytes, and
/// `NEITHER` from the exception directory. `get_pos` and `area_sweep` kept
/// their counts; `alloc_event` went from 65 calls to 68. The values were
/// first derived on 26345091 and re-pinned the same day to the 26356046
/// hotfix, which moved only `area_sweep`, `NEITHER` and the literal. On
/// 26361232 the three functions and `NEITHER` were all 0x50 lower than on
/// 26356046 and the literal did not move. On 26423880 every one moved, each
/// by its own amount, and the counts held; 26480954 moved them all again
/// (`NEITHER` is the same function, 0xc50 on) with the same counts.
const CODE_XREFS: &str = "19399b0"; // `get_pos`: six calls, no cells (26423880: 1938d70, 26361232: 19375c0, 26356046: 1937610, 25477059: 17fa220)
const MANY_XREFS: &str = "161ccb0"; // `alloc_event`, hot: 68 calls (26423880: 161c490, 26361232: 161af80, 26356046: 161afd0, 25477059: 1439060, 65)
const ONE_XREF: &str = "3b5a4a0"; // `area_sweep`, reached exactly once (26423880: 3b596f0, 26361232: 3b53c90, 26356046: 3b53d60, 25477059: 387bda0)
/// A real function with neither. 25477059 used 2affbf0, 26345091 2af99b0, 26356046 2af9ce0
/// (mid-instruction on 26361232, so the pin follows the function), 26361232
/// 2af9c90, 26423880 2aff360. This one is a `.pdata` function start behind
/// int3 padding with a full prologue, one of four such starts in
/// 0x2A00000..0x2C00000 on 26480954 (five on 26423880, seven on 26361232) that no rel32, RIP-relative operand or 8-byte VA
/// anywhere in the image reaches (one Python pass over the loaded image, then
/// this tool and its `--selfcheck` to confirm).
const NEITHER: &str = "2afffb0";
/// `SetAdditionalCollectDropRate`, the short-name literal (26423880: 5B5DD88, 26361232: 5B58050, 25477059: 57AFC28).
const POINTER_ONLY: &str = "5B5FE28";

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

/// Every known answer on build 26480954, in ONE test.
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
    // The caller at get_pos-0x117 on 25246367 (+0x1764309) and 25477059
    // (+0x17FA109) is at get_pos-0x140 on 26356046 (+0x19374D0) and on
    // 26361232 (+0x1937480), on 26423880 (+0x1938C30), and on 26480954,
    // +0x1939870.
    assert!(s.contains("6 code reference(s) and 0 pointer cell(s) to +0x19399B0"), "{s}");
    assert_eq!(s.lines().filter(|l| l.starts_with("call ")).count(), 6);
    assert!(s.contains("call           at +0x1939870"), "{s}");

    // The rest of the spread: many, exactly one, and none at all.
    assert!(stdout(&[MANY_XREFS]).contains("68 code reference(s) and 0 pointer cell(s)"));
    assert!(stdout(&[ONE_XREF]).contains("1 code reference(s) and 0 pointer cell(s)"));

    // Nothing pointing here is not a failure and not an empty report: the
    // summary line is always printed, because "nothing" is an answer.
    assert_eq!(
        stdout(&[NEITHER]),
        "0 code reference(s) and 0 pointer cell(s) to +0x2AFFFB0\n"
    );

    // `SetAdditionalCollectDropRate`, the worked example in crates/tools/README.md.
    // A scanner that reported only code references would print two coincidental
    // `lea`s into the middle of a string and nothing that identifies the
    // address at all. The pointer cell is the whole answer. Build 26480954; on
    // 26423880 the literal was +0x5B5DD88 with its cell at +0x5D34398; on
    // 26356046 and 26361232 the literal was +0x5B58050 with its cell at
    // +0x5D2E048; on 26345091 the literal was +0x5B57050 with its cell at +0x5D2D1F8; on 25477059 the cell was +0x59792A8 holding VA 0x1457AFC28, and on
    // 25246367 +0x5861BF8 holding VA 0x14569C7C0.
    let s = stdout(&[POINTER_ONLY]);
    assert!(
        s.contains("ptr cell       at +0x5D358A8  (holds VA 0x145B5FE28)"),
        "the pointer cell is gone:\n{s}"
    );
    assert!(s.contains("2 code reference(s) and 1 pointer cell(s) to +0x5B5FE28"), "{s}");

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
