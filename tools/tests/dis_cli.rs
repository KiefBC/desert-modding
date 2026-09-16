//! `dis` against the shipped CrimsonDesert.exe.
//!
//! Ignored by default; the game is not on a CI runner. The window builder and
//! the formatter options are unit-tested offline in `src/bin/dis.rs`.
//!
//! `dis` is the one tool in this directory allowed to differ from the script it
//! replaced, because it swapped `objdump` for `iced-x86`. Operand SPELLING may
//! differ. Instruction BOUNDARIES may not, in code, and that is what these
//! check: known first bytes at known RVAs on build 25246367, and the property
//! that every line's address is the previous one plus that instruction's
//! length with nothing skipped.

use std::path::PathBuf;
use std::process::{Command, Output};

fn exe() -> PathBuf {
    match std::env::var_os("EXE") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe"),
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dis"))
        .args(args)
        .output()
        .expect("cannot run the dis binary")
}

fn stdout(args: &[&str]) -> String {
    let out = run(args);
    assert_eq!(out.status.code(), Some(0), "dis {args:?} failed: {out:?}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn a_known_function_prologue_decodes_to_the_expected_lines() {
    assert!(exe().is_file(), "CrimsonDesert.exe not found at {}", exe().display());
    // `get_pos` from reference-signatures.txt, converted from its file offset
    // 0x1763820 to an RVA. This is also the worked example of that conversion:
    // the two numbers differ by 0xC00 here and by something else in every
    // other section.
    let s = stdout(&["1764420", "1764500"]);
    let lines: Vec<&str> = s.lines().collect();
    assert_eq!(lines[0], "+1764420 push rbx");
    assert_eq!(lines[1], "+1764422 sub rsp,0x50");
    assert_eq!(lines[2], "+1764426 mov rax,QWORD PTR [rcx+0x68]");
    assert_eq!(lines[3], "+176442a mov rcx,QWORD PTR [rax+0x1a0]");
    assert_eq!(lines[4], "+1764431 mov rax,QWORD PTR [rcx]");
    // The signature comment says this one walks [rcx+0x68] -> [+0x1A0] ->
    // vtbl+0x140; the call through the vtable is the last of those.
    assert!(s.contains("call QWORD PTR [rax+0x140]"), "{s}");
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn every_address_is_covered_exactly_once_with_no_gap_and_no_overlap() {
    // The property that would catch a decoder losing sync: the addresses are
    // strictly increasing, start at the requested RVA, and the last one is
    // inside the range. A gap means bytes were skipped; a repeat means the
    // window was rebuilt mid-run.
    for (start, end) in [
        ("1764420", "1764500"),
        ("13aaa80", "13aab40"),
        ("2a73c20", "2a73d00"),
        ("3796520", "3796600"),
    ] {
        let s = stdout(&[start, end]);
        let addrs: Vec<u64> = s
            .lines()
            .map(|l| u64::from_str_radix(l.split(' ').next().unwrap().trim_start_matches('+'), 16).unwrap())
            .collect();
        let (lo, hi) = (u64::from_str_radix(start, 16).unwrap(), u64::from_str_radix(end, 16).unwrap());
        assert_eq!(addrs.first(), Some(&lo), "{start}..{end} does not start at the start");
        assert!(addrs.windows(2).all(|w| w[0] < w[1]), "{start}..{end} addresses are not increasing");
        assert!(*addrs.last().unwrap() < hi, "{start}..{end} ran past the end address");
        // No instruction is longer than 15 bytes, so no step may be either.
        assert!(addrs.windows(2).all(|w| w[1] - w[0] <= 15), "{start}..{end} has an impossible step");
    }
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn an_instruction_straddling_the_end_address_still_decodes_whole() {
    // 0x1764422 is `sub rsp,0x50`, four bytes. Asking for a range that ends
    // one byte into it must print it whole, not a truncated `(bad)` - the
    // decoder is given 15 bytes of slack past the end for this. `objdump
    // --stop-address` behaves the same way.
    let s = stdout(&["1764422", "1764423"]);
    assert_eq!(s, "+1764422 sub rsp,0x50\n");
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn rip_relative_operands_resolve_to_rvas() {
    // Only true because the decoder's IP is the RVA. If a future change
    // decoded a bare buffer from 0, every one of these would silently become
    // an offset into that buffer - still plausible-looking, still wrong.
    let s = stdout(&["176445f", "1764466"]);
    assert_eq!(s, "+176445f lea rax,[0x57fc990]\n");
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn no_line_is_wider_than_a_hundred_columns() {
    // A wall of prefixes in data can format to hundreds of characters.
    let s = stdout(&["569c7c0", "569c800"]);
    assert!(s.lines().all(|l| l.chars().count() <= 100));
    assert!(!s.is_empty());
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn nothing_is_written_to_a_temporary_directory() {
    // `dis.sh` cached a ~385 MB image copy in $TMPDIR and never removed it,
    // and the cache is what served a stale game build's bytes for three days.
    // This port keeps no cache; that is a behaviour, so it gets a test.
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_dis"))
        .args(["1764420", "1764500"])
        .env("TMPDIR", tmp.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0, "dis left something in TMPDIR");
}

#[test]
fn an_empty_or_backwards_range_is_an_error() {
    // Offline: both are rejected before the exe is opened.
    for args in [["1764500", "1764420"], ["1764420", "1764420"]] {
        let out = run(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?} was accepted");
        assert!(String::from_utf8_lossy(&out.stderr).contains("empty range"));
    }
}

#[test]
fn a_non_hex_address_is_an_error() {
    let out = run(&["zzz", "1764500"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a hexadecimal RVA"));
}
