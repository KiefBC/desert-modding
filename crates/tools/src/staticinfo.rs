//! Static-info tables as the archive stores them: one `<table>.staticinfobody`
//! (the raw record stream the record-loader hook patches) and one
//! `<table>.staticinfoheader` (the offsets table the hook sees at `mgr+0x28`).
//!
//! ```text
//! header   count, then count x { key, u32 body offset }
//! ```
//!
//! `analysis/staticinfo.py` knew one shape of that: a `u16` or `u32` count and
//! a `u32` key. The 137 tables of build 26361232 use more. The count is a `u16`
//! on 114 of them, a `u32` on 22 and a `u8` on one (`gamestartinfo`); the key is
//! a `u32` on 88, a `u16` on 40, a `u8` on 7, a `u64` on one
//! (`characterappearanceindexinfo`) and 12 bytes on one (`aieventtableinfo`).
//! [`split`] tries every count width and key width and keeps the one whose
//! count makes the header length exact with offsets strictly ascending from 0
//! and inside the body. For a given count width only one key width can fit
//! (the count fixes the entry width); across count widths a `u16` count is
//! preferred, then `u32`, then `u8`, which settles the one table that fits two
//! ways (`autospawnfilterinfo`, 7 bytes: a `u16` count and a `u8` key, the
//! shape `gamestartinfo`'s `u8` key also has).

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::paz::Archive;

/// Where group `0008` keeps the tables.
pub const TABLE_DIR: &str = "gamedata/binarystaticinfo__/bin";
pub const BODY_EXT: &str = ".staticinfobody";
pub const HEADER_EXT: &str = ".staticinfoheader";

/// One record of a table: its index, key and byte range in the body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub index: usize,
    /// The key's first (up to) 8 bytes, little-endian, zero-extended.
    pub key: u64,
    /// Bytes 8..12 of a 12-byte key, else 0.
    pub key_hi: u32,
    pub offset: usize,
    pub end: usize,
}

/// How a header is laid out: the width of its count and of each key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub count_width: usize,
    pub key_width: usize,
}

/// Count widths in order of preference; see the module comment.
const COUNT_WIDTHS: [usize; 3] = [2, 4, 1];
const KEY_WIDTHS: [usize; 5] = [4, 2, 1, 8, 12];

/// A table's two files.
#[derive(Debug, Clone)]
pub struct Table {
    pub body: Vec<u8>,
    pub header: Vec<u8>,
}

impl Table {
    pub fn split(&self) -> Result<Vec<Record>> {
        split(&self.header, &self.body)
    }
}

fn le(b: &[u8]) -> u64 {
    b.iter().rev().fold(0, |v, &c| (v << 8) | u64::from(c))
}

/// The records for `shape`, if the header fits it.
fn records(header: &[u8], body_len: usize, shape: Shape) -> Option<Vec<Record>> {
    let Shape { count_width: start, key_width: kw } = shape;
    let w = kw + 4;
    let rest = header.len().checked_sub(start)?;
    if rest % w != 0 {
        return None;
    }
    let n = rest / w;
    if n == 0 || le(header.get(..start)?) != n as u64 {
        return None;
    }
    let mut recs = Vec::with_capacity(n);
    for i in 0..n {
        let e = header.get(start + w * i..start + w * (i + 1))?;
        let key = e.get(..kw.min(8))?;
        let hi = e.get(8.min(kw)..kw)?;
        let offset = le(e.get(kw..)?) as usize;
        recs.push(Record { index: i, key: le(key), key_hi: le(hi) as u32, offset, end: 0 });
    }
    let ascending = recs.windows(2).all(|p| p[0].offset < p[1].offset);
    if !ascending || recs[0].offset != 0 || recs[n - 1].offset >= body_len {
        return None;
    }
    for i in 0..n {
        recs[i].end = recs.get(i + 1).map_or(body_len, |r| r.offset);
    }
    Some(recs)
}

/// Split a table body into records by its header, and say which header shape
/// it used.
pub fn split_shape(header: &[u8], body: &[u8]) -> Result<(Shape, Vec<Record>)> {
    for count_width in COUNT_WIDTHS {
        for key_width in KEY_WIDTHS {
            let shape = Shape { count_width, key_width };
            if let Some(recs) = records(header, body.len(), shape) {
                return Ok((shape, recs));
            }
        }
    }
    bail!(
        "header shape not recognised ({} header bytes, {} body bytes): no count width (u16, u32, u8) and key width (1, 2, 4, 8, 12) fits",
        header.len(),
        body.len()
    )
}

/// Split a table body into records by its header.
pub fn split(header: &[u8], body: &[u8]) -> Result<Vec<Record>> {
    split_shape(header, body).map(|(_, r)| r)
}

/// The archive path of one of `table`'s files, `ext` being [`BODY_EXT`] or
/// [`HEADER_EXT`].
pub fn archive_path(table: &str, ext: &str) -> String {
    format!("{TABLE_DIR}/{table}{ext}")
}

/// Read `table` out of the archive.
pub fn read_from_archive(archive: &Archive, table: &str) -> Result<Table> {
    Ok(Table {
        body: archive.read(&archive_path(table, BODY_EXT))?,
        header: archive.read(&archive_path(table, HEADER_EXT))?,
    })
}

/// Read `table` from a directory of extracted copies (`<dir>/<table>.staticinfobody`).
pub fn read_from_dir(dir: &Path, table: &str) -> Result<Table> {
    let read = |ext: &str| {
        let p = dir.join(format!("{table}{ext}"));
        std::fs::read(&p).with_context(|| format!("cannot read {}", p.display()))
    };
    Ok(Table { body: read(BODY_EXT)?, header: read(HEADER_EXT)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(count_width: usize, key_width: usize, ents: &[(u128, u32)]) -> Vec<u8> {
        let mut h = (ents.len() as u64).to_le_bytes()[..count_width].to_vec();
        for (k, o) in ents {
            h.extend_from_slice(&k.to_le_bytes()[..key_width]);
            h.extend_from_slice(&o.to_le_bytes());
        }
        h
    }

    #[test]
    fn both_count_widths_split() {
        let body = vec![0u8; 30];
        let ents = [(7, 0), (9, 10), (11, 25)];
        for w in [2, 4] {
            let (shape, recs) = split_shape(&header(w, 4, &ents), &body).unwrap();
            assert_eq!(shape, Shape { count_width: w, key_width: 4 });
            assert_eq!(recs.len(), 3);
            assert_eq!(recs[0], Record { index: 0, key: 7, key_hi: 0, offset: 0, end: 10 });
            assert_eq!(recs[2], Record { index: 2, key: 11, key_hi: 0, offset: 25, end: 30 });
        }
    }

    #[test]
    fn other_key_widths_split() {
        let body = vec![0u8; 30];
        for kw in [1, 2, 8] {
            let (shape, recs) = split_shape(&header(2, kw, &[(0x40, 0), (0x41, 12)]), &body).unwrap();
            assert_eq!(shape.key_width, kw);
            assert_eq!((recs[1].key, recs[1].offset, recs[1].end), (0x41, 12, 30));
        }
        let big = 0x0102_0304_0506_0708_090A_0B0C_u128;
        let recs = split(&header(2, 12, &[(big, 0)]), &body).unwrap();
        assert_eq!((recs[0].key, recs[0].key_hi), (0x0506_0708_090A_0B0C, 0x0102_0304));
        // gamestartinfo's header exactly: a u8 count and a u8 key.
        let (shape, _) = split_shape(&[1, 0x40, 0, 0, 0, 0], &body).unwrap();
        assert_eq!(shape, Shape { count_width: 1, key_width: 1 });
        // autospawnfilterinfo's: fits u8/u16 and u16/u8; the u16 count wins.
        let (shape, recs) = split_shape(&[1, 0, 0x40, 0, 0, 0, 0], &body).unwrap();
        assert_eq!(shape, Shape { count_width: 2, key_width: 1 });
        assert_eq!(recs[0].key, 0x40);
    }

    #[test]
    fn bad_headers_are_errors() {
        let body = vec![0u8; 30];
        // Not ascending, not starting at 0, last offset past the body.
        assert!(split(&header(4, 4, &[(1, 0), (2, 10), (3, 10)]), &body).is_err());
        assert!(split(&header(4, 4, &[(1, 4), (2, 10)]), &body).is_err());
        assert!(split(&header(2, 4, &[(1, 0), (2, 30)]), &body).is_err());
        // A count that disagrees with the length, and a truncated header.
        let mut h = header(4, 4, &[(1, 0), (2, 10)]);
        h[0] = 3;
        assert!(split(&h, &body).is_err());
        assert!(split(&[1], &body).is_err());
        assert!(split(&[], &body).is_err());
    }
}
