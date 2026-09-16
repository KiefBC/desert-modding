//! `fieldnames` against the shipped exe.
//!
//! Everything here needs `CrimsonDesert.exe`, which is 363 MB and is not in the
//! repository, so every test is `#[ignore]`d and a bare `cargo test` passes on
//! a machine that has never installed the game. Run them with
//!
//!     cargo test --test fieldnames -- --ignored
//!
//! and `EXE=/path/to/CrimsonDesert.exe` if the game is not in the usual Steam
//! library. They are the only place the tool's two counts and its
//! offset-to-RVA arithmetic get checked against real data.

use std::path::{Path, PathBuf};
use std::process::Command;

fn exe() -> PathBuf {
    match std::env::var_os("EXE") {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(
            "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe",
        ),
    }
}

/// A throwaway repo root, so the inventory lands in the temp directory instead
/// of over the one in the working tree.
fn fake_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("justfile"), "").unwrap();
    std::fs::write(dir.path().join("flake.nix"), "").unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_fieldnames"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the tool should start")
}

#[test]
#[ignore = "needs the game exe"]
fn a_rebuild_passes_every_self_check() {
    let dir = fake_root();
    let out = run(dir.path(), &["--exe", exe().to_str().unwrap()]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(
        !text.contains("FAIL"),
        "a self-check moved; explain it before trusting the inventory:\n{text}"
    );
    assert!(
        text.contains("4675 (class, field) pairs, expected 4675"),
        "{text}"
    );
    assert!(
        text.contains("536 distinct classes, expected 536"),
        "{text}"
    );
    assert!(dir.path().join("analysis/fieldnames.json").is_file());
}

/// The point of carrying the RVA: `xrefs` takes one and `sigscan` prints the
/// other, so the conversion has to be right or the whole output is a trap. The
/// oracle is the section table read straight out of the file here, independent
/// of `desert_tools::pe`.
#[test]
#[ignore = "needs the game exe"]
fn every_rva_matches_an_independent_section_walk() {
    let dir = fake_root();
    run(dir.path(), &["--exe", exe().to_str().unwrap()]);
    let text = std::fs::read_to_string(dir.path().join("analysis/fieldnames.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();

    let head = std::fs::read(exe()).unwrap();
    let u16at = |o: usize| u16::from_le_bytes([head[o], head[o + 1]]) as usize;
    let u32at = |o: usize| u32::from_le_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
    let pe = u32at(0x3c) as usize;
    let nsec = u16at(pe + 6);
    let table = pe + 24 + u16at(pe + 20);
    // (raw_offset, raw_offset + raw_size, virtual_address)
    let secs: Vec<(u32, u32, u32)> = (0..nsec)
        .map(|i| {
            let e = table + i * 40;
            let va = u32at(e + 12);
            let rs = u32at(e + 16);
            let ro = u32at(e + 20);
            (ro, ro + rs, va)
        })
        .collect();

    let pairs = doc["pairs"].as_array().unwrap();
    assert_eq!(pairs.len(), 4675);
    for p in pairs {
        let off = p["message_off"].as_u64().unwrap() as u32;
        let want = secs
            .iter()
            .find(|(lo, hi, _)| *lo <= off && off < *hi)
            .map(|(lo, _, va)| va + (off - lo));
        assert_eq!(
            p["message_rva"].as_u64().map(|v| v as u32),
            want,
            "offset {off:#x} converted wrong"
        );
    }
}

/// Every anchored relation in one place: the two classes the gatherer writes,
/// and that `dropTagNameHash` is on those two and nowhere else.
#[test]
#[ignore = "needs the game exe"]
fn the_drop_tag_hash_is_on_exactly_two_classes() {
    let dir = fake_root();
    run(dir.path(), &["--exe", exe().to_str().unwrap()]);
    let out = run(dir.path(), &["--field", "dropTagNameHash"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.starts_with("_dropTagNameHash appears on 2 class(es):"),
        "{text}"
    );
    assert!(text.contains("DropInfoData"), "{text}");
    assert!(text.contains("DropSetInfo"), "{text}");
}

/// A lookup reads the JSON back rather than re-walking 363 MB; a run with no
/// exe at all must still answer once the inventory exists.
#[test]
#[ignore = "needs the game exe"]
fn a_lookup_answers_from_the_json_without_the_exe() {
    let dir = fake_root();
    run(dir.path(), &["--exe", exe().to_str().unwrap()]);
    let out = run(dir.path(), &["--exe", "/nonexistent", "GimmickInfo"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("GimmickInfo  (211 fields)"), "{text}");
}

/// A missing exe with no inventory to fall back on is the one failure a user
/// can act on, so it exits 1 with the hint and nothing else.
#[test]
fn a_missing_exe_and_no_inventory_exits_one() {
    let dir = fake_root();
    let out = run(dir.path(), &["--exe", "/nonexistent/CrimsonDesert.exe"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim_end(),
        "/nonexistent/CrimsonDesert.exe not found (pass --exe)"
    );
}

/// A truncated inventory must be rebuilt, not half-answered. Without an exe to
/// rebuild from, that shows up as the same exit-1 hint rather than a parse
/// error or an empty result.
#[test]
fn a_corrupt_inventory_is_not_trusted() {
    let dir = fake_root();
    std::fs::create_dir_all(dir.path().join("analysis")).unwrap();
    std::fs::write(dir.path().join("analysis/fieldnames.json"), "{\"pairs\": [").unwrap();
    let out = run(dir.path(), &["--exe", "/nonexistent", "--list"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("not found (pass --exe)"));
}

/// A hand-written inventory drives the lookup modes, so their formatting is
/// checked without the game. The column widths are what make the output
/// greppable and they are load-bearing.
#[test]
fn the_lookup_modes_format_from_a_written_inventory() {
    let dir = fake_root();
    std::fs::create_dir_all(dir.path().join("analysis")).unwrap();
    std::fs::write(
        dir.path().join("analysis/fieldnames.json"),
        r#"{"pairs": [
 {"class": "DropSetInfo", "field": "dropTagNameHash", "message_off": 90682704, "message_rva": 90685776},
 {"class": "DropSetInfo", "field": "key", "message_off": 90682600, "message_rva": null},
 {"class": "GimmickInfo", "field": "dropSetInfoList", "message_off": 90000000, "message_rva": 90003072}
]}"#,
    )
    .unwrap();

    let text = |args: &[&str]| String::from_utf8(run(dir.path(), args).stdout).unwrap();

    assert_eq!(text(&["--list"]), "   2  DropSetInfo\n   1  GimmickInfo\n");
    assert_eq!(
        text(&["--field", "dropTagNameHash"]),
        "_dropTagNameHash appears on 1 class(es):\n  \
         DropSetInfo                                      msg @ file 0x567B550\n"
    );
    assert_eq!(
        text(&["--grep", "drop"]),
        "DropSetInfo._dropTagNameHash\nDropSetInfo._key\n\
         GimmickInfo._dropSetInfoList\n3 match(es)\n"
    );
    // A pair with no RVA prints the file offset alone, rather than an RVA of 0
    // that would send someone to the wrong address.
    assert_eq!(
        text(&["DropSetInfo"]),
        "\nDropSetInfo  (2 fields)\n  \
         _dropTagNameHash                                  msg @ file 0x567B550  rva 0x567C150\n  \
         _key                                              msg @ file 0x567B4E8\n"
    );
    assert_eq!(text(&["Nope"]), "no class matching 'Nope'\n");
    assert_eq!(
        text(&["--field", "nope"]),
        "no field named 'nope' (try --grep)\n"
    );
}

/// The inventory belongs to the repository, not to the caller's cwd. Anyone can
/// run this from `tools/` or from a `docs/` subdirectory while chasing a field
/// name, and it must read and write the one `analysis/fieldnames.json` either
/// way rather than scattering copies or silently finding none.
#[test]
fn the_inventory_path_does_not_depend_on_the_working_directory() {
    let dir = fake_root();
    std::fs::create_dir_all(dir.path().join("analysis")).unwrap();
    std::fs::create_dir_all(dir.path().join("tools/src/bin")).unwrap();
    std::fs::write(
        dir.path().join("analysis/fieldnames.json"),
        r#"{"pairs": [{"class": "DropSetInfo", "field": "key",
                      "message_off": 1, "message_rva": 2}]}"#,
    )
    .unwrap();

    for cwd in [
        dir.path().to_path_buf(),
        dir.path().join("analysis"),
        dir.path().join("tools/src/bin"),
    ] {
        let out = run(&cwd, &["--list"]);
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "   1  DropSetInfo\n",
            "from {}",
            cwd.display()
        );
    }
}
