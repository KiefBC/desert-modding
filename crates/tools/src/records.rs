//! Record headers in a raw `gimmickinfo` table body, in both shapes the game
//! has used.
//!
//! ```text
//! KeyFirst   u32 key, u32 name_len, name, 0x00                          through 25477059
//! NameFirst  u32 name_len, name, u32 key, 0x00, u32 path_len, path,     26361232
//!            u8 flag (0 or 1)
//! ```
//!
//! **Mirrors `desert_core::gimmick::{HeaderShape, parse_header_as}`**; `crates/tools/`
//! does not depend on `desert-core`, deliberately, and
//! `crates/desert-core/tests/gimmick_real.rs` asserts the same 14210 records off the
//! same table body.
//!
//! The two shapes are told apart by probing the table's first record, never by
//! scanning for either: a strict `KeyFirst` scan of the 26361232 body finds
//! 36248 false headers, because every `NameFirst` path and nested string has a
//! `u32 len` in front of it. A `NameFirst` scan is exact (every record's header
//! validates and nothing else's does: 14210 hits, 14210 distinct keys), so on
//! a `NameFirst` body the header walk is that scan and nothing heuristic.

/// Which header layout a table body uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderShape {
    KeyFirst,
    NameFirst,
}

/// One record header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// Where the record starts (its first header byte).
    pub off: usize,
    pub key: u32,
    /// Where the `u32` key sits: `off` for `KeyFirst`, `off + 4 + name_len`
    /// for `NameFirst`.
    pub key_at: usize,
    pub name: String,
    /// One past the header's last byte (the NUL, or the flag byte): where the
    /// record's fields start.
    pub body: usize,
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn printable(b: &[u8], at: usize, len: usize) -> Option<String> {
    let s = b.get(at..at.checked_add(len)?)?;
    s.iter().all(|c| (0x20..=0x7E).contains(c)).then(|| String::from_utf8_lossy(s).into_owned())
}

/// The header at `o` in exactly `shape`, validated strictly (the same rules
/// as `gimmick::parse_header_as`).
pub fn header_at(b: &[u8], o: usize, shape: HeaderShape) -> Option<Header> {
    match shape {
        HeaderShape::KeyFirst => {
            let key = u32_at(b, o)?;
            let len = u32_at(b, o + 4)? as usize;
            if !(1..=255).contains(&len) {
                return None;
            }
            let name = printable(b, o + 8, len)?;
            (*b.get(o + 8 + len)? == 0).then_some(Header { off: o, key, key_at: o, name, body: o + 9 + len })
        }
        HeaderShape::NameFirst => {
            let len = u32_at(b, o)? as usize;
            if !(1..=255).contains(&len) {
                return None;
            }
            let name = printable(b, o + 4, len)?;
            let k = o + 4 + len;
            let key = u32_at(b, k)?;
            if *b.get(k + 4)? != 0 {
                return None;
            }
            let plen = u32_at(b, k + 5)? as usize;
            if !(1..=255).contains(&plen) {
                return None;
            }
            let path = printable(b, k + 9, plen)?;
            if !path.starts_with('/') || *b.get(k + 9 + plen)? > 1 {
                return None;
            }
            Some(Header { off: o, key, key_at: k, name, body: k + 10 + plen })
        }
    }
}

/// The shape of the table body's first record: the one shape offset 0
/// validates as, `None` when neither or both do.
pub fn probe(b: &[u8]) -> Option<HeaderShape> {
    let kf = header_at(b, 0, HeaderShape::KeyFirst).is_some();
    let nf = header_at(b, 0, HeaderShape::NameFirst).is_some();
    match (kf, nf) {
        (true, false) => Some(HeaderShape::KeyFirst),
        (false, true) => Some(HeaderShape::NameFirst),
        _ => None,
    }
}

/// Every `NameFirst` header in the body, ascending: every offset tried, so
/// nothing depends on having walked the previous record correctly.
pub fn name_first_headers(b: &[u8]) -> Vec<Header> {
    (0..b.len()).filter_map(|o| header_at(b, o, HeaderShape::NameFirst)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nf(name: &str, key: u32, path: &str, flag: u8) -> Vec<u8> {
        let mut r = (name.len() as u32).to_le_bytes().to_vec();
        r.extend_from_slice(name.as_bytes());
        r.extend_from_slice(&key.to_le_bytes());
        r.push(0);
        r.extend_from_slice(&(path.len() as u32).to_le_bytes());
        r.extend_from_slice(path.as_bytes());
        r.push(flag);
        r
    }

    fn kf(key: u32, name: &str) -> Vec<u8> {
        let mut r = key.to_le_bytes().to_vec();
        r.extend_from_slice(&(name.len() as u32).to_le_bytes());
        r.extend_from_slice(name.as_bytes());
        r.push(0);
        r
    }

    #[test]
    fn both_shapes_parse_and_probe() {
        let mut t = nf("gimmick_well_0001_parts01", 1001081, "/object/cd_gimmick/00_common/well/x.prefab", 1);
        let len = t.len();
        t.extend_from_slice(&[0x02, 0, 0, 0]);
        let h = header_at(&t, 0, HeaderShape::NameFirst).unwrap();
        assert_eq!((h.key, h.key_at, h.body), (1001081, 4 + 25, len));
        assert_eq!(probe(&t), Some(HeaderShape::NameFirst));
        let old = kf(1001081, "gimmick_well_0001_parts01");
        assert_eq!(probe(&old), Some(HeaderShape::KeyFirst));
        assert_eq!(header_at(&old, 0, HeaderShape::KeyFirst).unwrap().body, old.len());
        assert_eq!(probe(&[0xFF; 16]), None);
    }

    #[test]
    fn the_name_first_walk_finds_each_record_once() {
        let mut t = nf("a_one", 10, "/object/a", 0);
        t.extend_from_slice(&[0x90; 7]);
        let second = t.len();
        t.extend(nf("b_two", 20, "/character/b", 1));
        let hs = name_first_headers(&t);
        assert_eq!(hs.iter().map(|h| (h.off, h.key)).collect::<Vec<_>>(), vec![(0, 10), (second, 20)]);
        // A path without its leading slash, or a flag of 2, is not a header.
        assert!(header_at(&nf("x", 1, "object", 0), 0, HeaderShape::NameFirst).is_none());
        assert!(header_at(&nf("x", 1, "/o", 2), 0, HeaderShape::NameFirst).is_none());
    }
}
