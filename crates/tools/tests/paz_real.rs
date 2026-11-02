//! `desert_tools::paz` and `desert_tools::staticinfo` against the installed
//! game's archives.
//!
//! Ignored by default: the game is not on a CI runner. `just tools-check-game`
//! runs every ignored test in this workspace, this file's with them.
//!
//! The oracle is `analysis/tables/`, the 274 static-info files the Python
//! reader (`analysis/pamt.py`, `lz4block.py`) extracted on build 26361232:
//! every one must come out of the archive byte for byte.

use std::path::PathBuf;

use desert_tools::paths;
use desert_tools::paz::{Archive, DEFAULT_GROUP};
use desert_tools::records;
use desert_tools::staticinfo::{self, BODY_EXT, HEADER_EXT, TABLE_DIR};

fn archive() -> Archive {
    let dir = paths::game_dir();
    Archive::open(&dir).unwrap_or_else(|e| panic!("no game archives at {}: {e:#}", dir.display()))
}

fn tables_dir() -> PathBuf {
    paths::repo_root().unwrap().join("analysis/tables")
}

#[test]
#[ignore = "needs the installed game's archives and analysis/tables/"]
fn every_static_info_file_matches_the_extracted_copy() {
    let a = archive();
    let g = a.group(DEFAULT_GROUP).unwrap();
    let oracle = tables_dir();
    let mut n = 0;
    for e in g.entries() {
        if e.dir != TABLE_DIR || !(e.name.ends_with(BODY_EXT) || e.name.ends_with(HEADER_EXT)) {
            continue;
        }
        let got = g.read(e).unwrap_or_else(|err| panic!("{}: {err:#}", e.path()));
        let want = std::fs::read(oracle.join(&e.name)).unwrap_or_else(|err| panic!("{}: {err}", e.name));
        assert!(got == want, "{} differs from analysis/tables/{}", e.path(), e.name);
        n += 1;
    }
    assert_eq!(n, 274);
}

#[test]
#[ignore = "needs the installed game's archives"]
fn gimmickinfo_entries_are_what_the_listing_says() {
    let a = archive();
    let (group, body) = a.find(&staticinfo::archive_path("gimmickinfo", BODY_EXT)).unwrap();
    assert_eq!(group, "0008");
    assert_eq!((body.flags, body.size, body.comp_size, body.paz()), (0x0802_0001, 17_621_227, 1_360_890, 1));
    assert_eq!((body.compression(), body.encryption()), (2, 0));
    let (_, header) = a.find(&staticinfo::archive_path("gimmickinfo", HEADER_EXT)).unwrap();
    assert_eq!((header.flags, header.size, header.comp_size), (0x0900_0001, 113_682, 113_682));
    assert_eq!(header.compression(), 0);
}

#[test]
#[ignore = "needs the installed game's archives"]
fn gimmickinfo_splits_into_its_records() {
    let a = archive();
    let t = staticinfo::read_from_archive(&a, "gimmickinfo").unwrap();
    let recs = t.split().unwrap();
    assert_eq!(recs.len(), 14210);
    // Record 0's own header names the key the offsets table gives it.
    let shape = records::probe(&t.body).expect("record header shape");
    let h = records::header_at(&t.body, recs[0].offset, shape).expect("record 0 header");
    assert_eq!(u64::from(h.key), recs[0].key);
    assert_eq!(recs.last().unwrap().end, t.body.len());

    // The extracted copy reads the same.
    let d = staticinfo::read_from_dir(&tables_dir(), "gimmickinfo").unwrap();
    assert!(d.body == t.body && d.header == t.header);
}

#[test]
#[ignore = "needs the installed game's archives"]
fn an_encrypted_entry_is_refused() {
    let a = archive();
    let err = a.read("gamedata/gamestring.binarystring").unwrap_err();
    assert!(err.to_string().contains("encrypted"), "{err:#}");
}

#[test]
#[ignore = "needs analysis/tables/"]
fn every_extracted_table_splits() {
    let dir = tables_dir();
    let mut n = 0;
    let mut failed = Vec::new();
    for de in std::fs::read_dir(&dir).unwrap() {
        let name = de.unwrap().file_name().to_string_lossy().into_owned();
        let Some(table) = name.strip_suffix(BODY_EXT) else { continue };
        let t = staticinfo::read_from_dir(&dir, table).unwrap();
        match t.split() {
            Ok(_) => n += 1,
            Err(e) => failed.push(format!("{table}: {e:#}")),
        }
    }
    assert!(failed.is_empty(), "{} of {} tables do not split:\n{}", failed.len(), n + failed.len(), failed.join("\n"));
    assert_eq!(n, 137);
}
