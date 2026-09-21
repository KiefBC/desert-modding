//! The clean-body walk: re-derive every stored `items` list from the bytes.
//!
//! `items` in `extra-families.json` is derived data, and this module verifies it
//! rather than trusting it. When DMM's clean table body is present, every record
//! in `records` and `records_not_enabled` is looked up by key in the record
//! walk: its name must match, the item ids across the output blocks it owns must
//! equal the stored `items`, and a record in `records` must own at least one
//! block (a row nothing can multiply is a mistake). Any mismatch is a hard
//! error, because a game update that changed the table is exactly what that
//! should catch. When the body is absent the stored rows are used and a banner
//! says they were NOT verified. Either way `collect.rs` regenerates, and
//! pointing the table path at something that does not exist is how that second
//! branch gets tested deliberately.
//!
//! The walk re-implements `desert_core::gimmick::output_lists` + `block_ok`
//! (greedy, non-overlapping, item id at block+1) and attributes each list to its
//! record with the echo discriminator from
//! `docs/findings/2026-09-12-water-wells.md` sections 7 and 8: nested string
//! fields use the same `u32 len, bytes, NUL` shape as record names, so a
//! backwards scan for a header finds false positives freely, and the test that
//! works is that a real record echoes its own `u32` key immediately before a
//! later digits-only id sub-field. [`CALIBRATION`] is what proves the walk still
//! resolves; all three anchors must match or nothing is written.

use std::collections::{BTreeSet, HashMap};

use anyhow::{bail, Result};
use regex::bytes::Regex;

use crate::spec::{check_money_rule, py_ints, py_strs, q, FamilySpec};

/// `desert_core::gimmick`'s block layout. `ITEM_AT` is 1, not 5: bytes +5..+8
/// are zero padding (findings-water-wells section 5).
const BLOCK: usize = 68;
const ITEM_AT: usize = 1;
const MIN_AT: usize = 42;
const MAX_AT: usize = 50;
const MAX_QTY: u64 = 100_000;

/// Table-wide totals and three (list offset -> record, record-relative offset)
/// anchors the record walk must reproduce. `firewood_0001` is the load-bearing
/// one: DMM patches file offset 12843255 = 12841210 + 1999 + 4 + MIN_AT, so a
/// walk that puts this list anywhere else is not calibrated. Without the echo
/// test the first two resolve to `NatureBuffTrigger` and `UnnamedTrigger_0`,
/// both with the bogus key 16777216. The well and the pot are anchors because
/// they are the two records the walk was first proved on; that the pot is no
/// longer a row changes nothing about whether the walk resolves it.
pub struct Calibration {
    pub lists: usize,
    pub blocks: usize,
    pub records: usize,
    /// `(list offset, record key, record name, record-relative offset)`
    pub anchors: &'static [(usize, u32, &'static str, usize)],
}

pub const CALIBRATION: Calibration = Calibration {
    lists: 573,
    blocks: 896,
    records: 13412,
    anchors: &[
        (12843209, 1002971, "firewood_0001", 1999),
        (4373870, 1001081, "gimmick_well_0001_parts01", 965),
        (1049316, 21030076, "Background_Breakable_66", 1600),
    ],
};

fn u32_at(buf: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(buf[o..o + 4].try_into().expect("4 bytes"))
}

fn u64_at(buf: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(buf[o..o + 8].try_into().expect("8 bytes"))
}

/// `b[5..9] == b[64..68]` is kept for fidelity even though both ranges are zero
/// on every real block (findings section 5); the flag, the `FF FF` and
/// `1 <= min <= max` are what make it selective.
fn block_ok(buf: &[u8], at: usize) -> bool {
    if at + BLOCK > buf.len() {
        return false;
    }
    if buf[at] != 1 || buf[at + 58] != 0xFF || buf[at + 59] != 0xFF {
        return false;
    }
    if buf[at + 5..at + 9] != buf[at + 64..at + 68] {
        return false;
    }
    let (lo, hi) = (u64_at(buf, at + MIN_AT), u64_at(buf, at + MAX_AT));
    1 <= lo && lo <= hi && hi <= MAX_QTY
}

/// Every resource-output list in the table body, as `(offset, block count)`.
///
/// A direct port of `desert_core::gimmick::output_lists`: a `u32` count of
/// 1..=64 followed by that many valid blocks, walked greedily and
/// non-overlapping.
pub fn output_lists(buf: &[u8]) -> Vec<(usize, usize)> {
    let n = buf.len();
    let mut lists = Vec::new();
    let mut i = 0usize;
    while i + 4 <= n {
        let count = u32_at(buf, i) as usize;
        if (1..=64).contains(&count)
            && i + 4 + count * BLOCK <= n
            && (0..count).all(|k| block_ok(buf, i + 4 + k * BLOCK))
        {
            lists.push((i, count));
            i += 4 + count * BLOCK;
        } else {
            i += 1;
        }
    }
    lists
}

/// One record header: `(offset of its length prefix, key, name)`.
pub struct Header {
    pub off: usize,
    pub key: u32,
    pub name: String,
}

/// Every real record header, sorted by offset.
///
/// Nested string fields share the record name's `u32 len, bytes, NUL` shape, so
/// shape alone is not enough. The discriminator that works: a real record echoes
/// its own `u32` key immediately before a later digits-only id sub-field. See
/// findings section 7.
pub fn record_headers(buf: &[u8]) -> Vec<Header> {
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
    pub hdrs: Vec<Header>,
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
    let lists = output_lists(buf);
    let hdrs = record_headers(buf);
    let blocks: usize = lists.iter().map(|&(_, c)| c).sum();
    for (what, got, want) in [
        ("lists", lists.len(), CALIBRATION.lists),
        ("blocks", blocks, CALIBRATION.blocks),
        ("records", hdrs.len(), CALIBRATION.records),
    ] {
        if got != want {
            bail!(
                "calibration failed: {got} {what}, expected {want}. The table \
                 changed or the walk broke; do not trust the walk. See \
                 docs/findings/2026-09-12-water-wells.md section 8."
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
                 expected {} (key {key}, rel +{rel})",
                q(&h.name),
                h.key,
                off - h.off,
                q(name)
            );
        }
    }
    Ok(w)
}

/// `{header offset: (sorted item ids, list count)}` over every output list.
pub fn what_records_pay(buf: &[u8], w: &Walk) -> HashMap<usize, (Vec<u32>, usize)> {
    let mut paid: HashMap<usize, (BTreeSet<u32>, usize)> = HashMap::new();
    for &(off, count) in &w.lists {
        let h = w.owner(off);
        let entry = paid.entry(h.off).or_default();
        for k in 0..count {
            entry.0.insert(u32_at(buf, off + 4 + k * BLOCK + ITEM_AT));
        }
        entry.1 += 1;
    }
    paid.into_iter()
        .map(|(off, (items, n))| (off, (items.into_iter().collect(), n)))
        .collect()
}

/// Re-derive every stored record's `items` from the clean body.
///
/// A drift is fatal: the stored items are derived data, and a game update
/// changing the table is exactly what this should catch. Returns the lines to
/// print once the file has been written.
pub fn verify(spec: &[FamilySpec], buf: &[u8], table: &str) -> Result<Vec<String>> {
    let w = walk(buf)?;
    let paid = what_records_pay(buf, &w);
    let mut by_key: HashMap<u32, Vec<&Header>> = HashMap::new();
    for h in &w.hdrs {
        by_key.entry(h.key).or_default().push(h);
    }
    let mut out = Vec::new();
    for family in spec {
        for (field, records, must_pay) in family.fields() {
            for rec in records {
                let location = format!("{}.{field}: record {} {}", family.name, rec.key, q(&rec.name));
                let all = by_key.get(&rec.key).map_or(&[][..], Vec::as_slice);
                let hits: Vec<&&Header> = all.iter().filter(|h| h.name == rec.name).collect();
                if hits.len() != 1 {
                    let mut others: Vec<String> = all.iter().map(|h| h.name.clone()).collect();
                    others.sort();
                    others.dedup();
                    let others = if others.is_empty() { "none".to_string() } else { py_strs(&others) };
                    bail!(
                        "{location}: {} record headers in the clean body, expected \
                         exactly 1 (headers with that key: {others}). The table \
                         changed, or the row is wrong.",
                        hits.len()
                    );
                }
                let empty = (Vec::new(), 0usize);
                let (items, nlists) = paid.get(&hits[0].off).unwrap_or(&empty);
                if must_pay && *nlists == 0 {
                    bail!("{location}: owns no resource-output block; nothing to multiply");
                }
                check_money_rule(&location, &rec.name, &family.name, items, "the clean body")?;
                if *items != rec.items {
                    bail!(
                        "{location}: the stored items do not match the clean body.\n  \
                         stored: {}\n  in the table: {} over {nlists} output list(s)\n\
                         Fix the json or work out what changed in the game's table first.",
                        py_ints(&rec.items),
                        py_ints(items)
                    );
                }
            }
            out.push(format!(
                "verified {}.{field}: {} records against {table}",
                family.name,
                records.len()
            ));
        }
    }
    Ok(out)
}

/// The banner printed when the clean body is not on this machine.
pub fn absent_banner(table: &str) -> Vec<String> {
    vec![
        String::new(),
        "!".repeat(72),
        format!("! {table} is ABSENT."),
        "! The records in tools/extra-families.json were NOT verified against".to_string(),
        "! the table body; their stored rows were used as-is. That is fine for".to_string(),
        "! a routine regenerate and NOT fine after a game update - re-run this".to_string(),
        "! on a machine that has the clean body before trusting the result.".to_string(),
        "!".repeat(72),
        String::new(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One synthetic output list: a `u32` count then `count` blocks paying
    /// `item`, with the min/max the real blocks carry.
    fn list(count: usize, item: u32) -> Vec<u8> {
        let mut v = (count as u32).to_le_bytes().to_vec();
        for _ in 0..count {
            let mut b = vec![0u8; BLOCK];
            b[0] = 1;
            b[ITEM_AT..ITEM_AT + 4].copy_from_slice(&item.to_le_bytes());
            b[MIN_AT..MIN_AT + 8].copy_from_slice(&5u64.to_le_bytes());
            b[MAX_AT..MAX_AT + 8].copy_from_slice(&5u64.to_le_bytes());
            b[58] = 0xFF;
            b[59] = 0xFF;
            v.extend_from_slice(&b);
        }
        v
    }

    #[test]
    fn output_lists_are_greedy_and_non_overlapping() {
        let mut buf = vec![0u8; 16];
        buf.extend_from_slice(&list(2, 22008));
        buf.extend_from_slice(&[0u8; 8]);
        let found = output_lists(&buf);
        assert_eq!(found, vec![(16, 2)]);
    }

    #[test]
    fn a_block_needs_its_flag_and_its_ff_ff() {
        let mut buf = vec![0u8; 8];
        buf.extend_from_slice(&list(1, 1));
        let at = 12;
        assert!(block_ok(&buf, at));
        buf[at] = 0;
        assert!(!block_ok(&buf, at), "flag byte is what makes the block selective");
        buf[at] = 1;
        buf[at + 58] = 0;
        assert!(!block_ok(&buf, at));
    }

    #[test]
    fn a_block_needs_one_le_min_le_max_le_max_qty() {
        let mut buf = vec![0u8; 8];
        buf.extend_from_slice(&list(1, 1));
        let at = 12;
        buf[at + MIN_AT..at + MIN_AT + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(!block_ok(&buf, at), "a minimum of 0 is not a payout");
        buf[at + MIN_AT..at + MIN_AT + 8].copy_from_slice(&9u64.to_le_bytes());
        assert!(!block_ok(&buf, at), "min above max is not a range");
        buf[at + MAX_AT..at + MAX_AT + 8].copy_from_slice(&(MAX_QTY + 1).to_le_bytes());
        assert!(!block_ok(&buf, at));
    }

    #[test]
    fn the_absent_banner_names_the_path_it_looked_for() {
        let b = absent_banner("/nowhere/clean.bin");
        assert!(b.iter().any(|l| l.contains("/nowhere/clean.bin is ABSENT.")));
        assert!(b.iter().any(|l| l.contains("NOT verified")));
    }
}
