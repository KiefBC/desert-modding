//! The two walks over the `gimmickinfo` body: the calibrated global one, and
//! the per-record one the rows are built from.
//!
//! **Per record** (`record_pays`): `desert_tools::outputs`, the tools' one port
//! of `desert_core::gimmick::output_lists` and `block_ok` (both equality
//! clauses included, and the walk `dmm-pack` writes the pack from), run over
//! one record's bytes as the archive header splits them. That is exactly what the plugin's
//! record-loader hook sees and multiplies, so a record this walk finds nothing
//! in is a record no multiplier reaches, and it is never a row. The loose
//! variant (the pad clause dropped, as `items --loose` has it) is walked too,
//! only so the report can say what the plugin cannot see.
//!
//! **Global** (`walk`): the same detector over the whole body, greedy and
//! non-overlapping, with each list attributed to the header that precedes it.
//! It is kept for [`CALIBRATION`] alone: its counts and three anchors are what
//! prove the body is the one the rule was measured on, and nothing is written
//! unless they reproduce. On 26361232 both walks find the same 581 lists: a
//! list never crosses a record boundary, because every header in between would
//! have to read as blocks.
//!
//! On a `KeyFirst` body (25477059 and earlier) the global walk attributes lists
//! with the echo discriminator from `docs/findings/2026-09-12-water-wells.md`
//! sections 7 and 8; nothing else here supports that shape any more.

use anyhow::{bail, Result};
use regex::bytes::Regex;

use crate::spec::q;
pub use desert_tools::outputs::{output_lists, record_pays, Detector};

/// Table-wide totals and three (list offset -> record, record-relative offset)
/// anchors the global walk must reproduce. `firewood_0001` is the load-bearing
/// one: its list at body offset 10136473 is 1796 bytes into the record on
/// 26361232 (1999 on 25477059 and 25246367), so a walk that puts this list
/// anywhere else is not calibrated. On a `KeyFirst` body without the echo test
/// the first two resolve to `NatureBuffTrigger` and `UnnamedTrigger_0`, both
/// with the bogus key 16777216. The well and the pot are anchors because they
/// are the two records the walk was first proved on; that the pot is not a row
/// changes nothing about whether the walk resolves it.
pub struct Calibration {
    /// The game build whose table body these numbers were taken on.
    pub build: &'static str,
    pub lists: usize,
    pub blocks: usize,
    pub records: usize,
    /// `(list offset, record key, record name, record-relative offset)`
    pub anchors: &'static [(usize, u32, &'static str, usize)],
}

/// One calibration, not one per build. The calibration belongs to the table
/// body, not the exe: the old body no longer exists anywhere to walk, so an
/// old entry could never be exercised, and keying on the appmanifest would
/// pick the wrong entry whenever `--tables` points at an older extract.
/// After an update, re-measure and replace these numbers.
///
/// Build 25246367 was 573 lists / 896 blocks / 13412 records, anchors at
/// 12843209, 4373870 and 1049316 (same keys, same relative offsets). 25477059
/// added 35 records, none with blocks, and `itembox_11` (key 1012375, never a
/// row) lost its only output block: 572 / 895 / 13447, anchors at 12921539,
/// 4382086 and 1051437, relative 1999, 965 and 1600.
///
/// 26361232 is the first `NameFirst` table (`desert_tools::records`): 581
/// lists / 904 blocks / 14210 records, every record found by the strict
/// header scan rather than the echo test. The relative offsets moved because
/// the record bodies did: the lists sit 120 to 320 bytes nearer their record's
/// start than on 25477059, and the header itself grew a prefab path.
pub const CALIBRATION: Calibration = Calibration {
    build: desert_tools::paths::CALIBRATED_BUILD,
    lists: 581,
    blocks: 904,
    records: 14210,
    anchors: &[
        (10136473, 1002971, "firewood_0001", 1796),
        (3374134, 1001081, "gimmick_well_0001_parts01", 731),
        (807359, 21030076, "Background_Breakable_66", 1393),
    ],
};

/// Which build the calibration was taken on, and the installed one if the
/// appmanifest says, for the refusal message.
fn builds() -> String {
    match desert_tools::paths::build_id() {
        Some(b) => format!("calibrated on build {}, installed build is {b}", CALIBRATION.build),
        None => format!("calibrated on build {}, installed build unknown", CALIBRATION.build),
    }
}

fn u32_at(buf: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(buf[o..o + 4].try_into().expect("4 bytes"))
}

/// One record header: `(offset of its length prefix, key, name)`.
pub struct Header {
    pub off: usize,
    pub key: u32,
    pub name: String,
}

/// Every real record header, sorted by offset.
///
/// On a `NameFirst` body (26361232 on, `desert_tools::records`) the header
/// validates strictly and nothing else in the body does, so the walk is that
/// scan. On a `KeyFirst` body nested string fields share the record name's
/// `u32 len, bytes, NUL` shape, so shape alone is not enough. The
/// discriminator that works there: a real record echoes its own `u32` key
/// immediately before a later digits-only id sub-field. See findings section
/// 7. The shape is probed off the body's first record.
pub fn record_headers(buf: &[u8]) -> Vec<Header> {
    use desert_tools::records::{name_first_headers, probe, HeaderShape};
    if probe(buf) == Some(HeaderShape::NameFirst) {
        return name_first_headers(buf).into_iter().map(|h| Header { off: h.off, key: h.key, name: h.name }).collect();
    }
    let n = buf.len();
    // Printable runs, capped at 120 bytes each. The cap is not cosmetic: a run
    // longer than 120 is matched as back-to-back 120-byte chunks, and the
    // `len - 2` below then skips the last two offsets of every chunk. Any
    // rewrite of this scan has to keep that or the header set shifts.
    let runs = Regex::new(r"[\x20-\x7E]{3,120}").expect("static regex");
    // Offsets come out ascending, so this vector is already the sorted
    // `soff` the lookahead below bisects on; it is never re-sorted.
    let mut strs: Vec<(usize, usize)> = Vec::new(); // (offset, length)
    for m in runs.find_iter(buf) {
        let run = m.as_bytes();
        for rel in 0..run.len().saturating_sub(2) {
            let o = m.start() + rel;
            if o < 8 {
                continue;
            }
            let length = u32_at(buf, o - 4) as usize;
            // `o + length < n`, not `<=`: the python this ports read
            // `buf[o + length]` after checking only `<=` and would have raised
            // an IndexError on a body whose very last byte ended a string.
            if length < 3 || o + length >= n || buf[o + length] != 0 {
                continue;
            }
            if buf[o..o + length].iter().all(|c| (0x20..=0x7E).contains(c)) {
                strs.push((o, length));
            }
        }
    }
    let bad = b"/.\\ <>=\"'\t";
    let mut hdrs = Vec::new();
    for (i, &(o, length)) in strs.iter().enumerate() {
        let name = &buf[o..o + length];
        if name.iter().all(u8::is_ascii_digit) || name.iter().any(|c| bad.contains(c)) {
            continue;
        }
        let key = u32_at(buf, o - 8);
        if key < 100_000 {
            // key == 1 and friends are nested fields
            continue;
        }
        // The echo test: a digits-only sub-field within 6000 bytes (and 400
        // string candidates) that repeats this record's own key.
        for &(o2, len2) in strs.iter().skip(i + 1).take(400) {
            if o2 - o > 6000 {
                break;
            }
            if buf[o2..o2 + len2].iter().all(u8::is_ascii_digit) && u32_at(buf, o2 - 8) == key {
                hdrs.push(Header {
                    off: o - 8,
                    key,
                    name: String::from_utf8_lossy(name).into_owned(),
                });
                break;
            }
        }
    }
    hdrs
}

/// The calibrated walk: lists, headers, and the header starts to attribute by.
pub struct Walk {
    pub lists: Vec<(usize, usize)>,
    hdrs: Vec<Header>,
    starts: Vec<usize>,
}

impl Walk {
    /// Index of the header that owns `off`.
    ///
    /// An offset below the first header wraps to the LAST header, because the
    /// python this ports indexed `hdrs[-1]` there. It cannot happen on the real
    /// body - the first list is well past the first header - and reproducing it
    /// is cheaper than arguing about a branch neither generator has taken.
    fn owner(&self, off: usize) -> &Header {
        let i = self.starts.partition_point(|&s| s <= off);
        if i == 0 {
            &self.hdrs[self.hdrs.len() - 1]
        } else {
            &self.hdrs[i - 1]
        }
    }
}

/// `(lists, headers, owner)` for the table body - calibrated, or an error.
///
/// Refuses to answer unless [`CALIBRATION`] reproduces exactly, because an
/// uncalibrated record walk mis-spans boundaries silently and that is the
/// failure mode this whole method exists to avoid.
pub fn walk(buf: &[u8]) -> Result<Walk> {
    let lists = output_lists(buf, Detector::Shipped);
    let hdrs = record_headers(buf);
    let blocks: usize = lists.iter().map(|&(_, c)| c).sum();
    for (what, got, want) in [
        ("lists", lists.len(), CALIBRATION.lists),
        ("blocks", blocks, CALIBRATION.blocks),
        ("records", hdrs.len(), CALIBRATION.records),
    ] {
        if got != want {
            bail!(
                "calibration failed: {got} {what}, expected {want} ({}). The \
                 table changed or the walk broke; do not trust the walk. See \
                 docs/findings/2026-09-12-water-wells.md section 8.",
                builds()
            );
        }
    }
    let starts = hdrs.iter().map(|h| h.off).collect();
    let w = Walk { lists, hdrs, starts };
    for &(off, key, name, rel) in CALIBRATION.anchors {
        let h = w.owner(off);
        if (h.key, h.name.as_str(), off - h.off) != (key, name, rel) {
            bail!(
                "calibration anchor {off} resolved to {} (key {}, rel +{}), \
                 expected {} (key {key}, rel +{rel}) ({})",
                q(&h.name),
                h.key,
                off - h.off,
                q(name),
                builds()
            );
        }
    }
    Ok(w)
}
