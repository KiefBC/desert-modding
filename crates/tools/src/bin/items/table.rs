//! The `gimmickinfo` table body, and the two walks over it.
//!
//! Everything here is a port of the retired `items.py`'s `Table` class and the
//! constants above it, and the reasoning in those comments is the part worth
//! keeping. `docs/findings/2026-09-12-water-wells.md` sections 5, 7 and 8 are
//! the source; two of the facts recorded below each cost an earlier
//! investigation a wrong answer.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};

// Block layout: `desert_tools::outputs`, the tools' one port of
// desert_core::gimmick's. ITEM_AT is the corrected +1; the constant said 5
// until 2026-09-12, which is four bytes of zero padding (findings section 5).
pub use desert_tools::outputs::{BLOCK, ITEM_AT, MAX_AT, MIN_AT, PAD_AT, PAD_TAIL_AT};

/// `PAD_TAIL_AT` is the second half of `block_ok`'s pad clause, and calling it
/// a pad is only true of the population that clause admits. What the
/// deserialiser says it is: `FUN_141a37180`, the block parser, consumes exactly
/// `+0..+63` and never reads these four bytes. The list loop `FUN_1414a7cc0`
/// reads them after each block and stores them at `entry+0x08` of its 0x10
/// -stride entries, so `+64` is **the list entry's own key field** - zero on
/// every gather record, populated on the records only `--loose` can see. It is
/// not a pad, not the high half of a wide item id, and not the variant tag at
/// `+9` (both populations carry tags 0 and 4). `PAD_AT` (`+5`) really is zero
/// on all 1037 blocks of both populations, so that half of the clause is the
/// vacuous one.
pub const ENTRY_KEY_AT: usize = PAD_TAIL_AT;

/// Every id in the shipped population is eight digits or fewer, and the largest
/// is 10021720 (`goldbar`); 205 of the 215 are six or seven. The bound is the
/// next order of magnitude up from that, so it accuses nothing the plugin
/// already trusts. Anything past it is **reported, never dropped**: an
/// out-of-range id means the list it sits in is being mis-parsed, and saying so
/// is worth more than hiding it.
pub const MAX_PLAUSIBLE_ITEM_ID: u64 = 99_999_999;

// Record-attribution tuning, from findings section 7/8. Changing any of these
// invalidates the three calibration anchors in `checks.rs`.
const MIN_RECORD_KEY: u32 = 100_000;
const ECHO_WINDOW: usize = 6000;
const ECHO_LOOKAHEAD: usize = 400;
const BAD_NAME_CHARS: &[u8] = b"/.\\ <>=\"'\t";

/// Which of the two block detectors a walk uses. A named choice, not a flag.
///
/// Both check the block's shape - flag byte 1 at `+0`, `FF FF` at `+58`, a
/// `u64` min/max pair at `+42`/`+50` inside `1..=MAX_QTY` - and both require
/// the two copies of the item id at `ITEM_AT` and `ITEM_TAIL_AT` to agree.
/// They differ in one clause:
///
/// * `Shipped` also requires `PAD_AT` to equal `PAD_TAIL_AT`, exactly as
///   `desert_core::gimmick::block_ok` does. **This is the population the
///   plugin sees**, and the only one whose yields the gatherer multiplies.
/// * `Loose` drops that clause and keeps everything else.
///
/// Dropping it grows the walk from 572 lists / 895 blocks / 215 items to
/// 588 / 1037 / 311 on build 25477059 (573 / 896 and 589 / 1038 on 25246367).
/// The 572 are a strict subset of the 588; the 16 extra lists differ only in
/// carrying a nonzero `u32` at `PAD_TAIL_AT`, which the deserialiser says is
/// the list entry's key field (see [`ENTRY_KEY_AT`]) rather than a pad.
/// **None of the 16 is in a `collect.rs` row** (rows pay shipped blocks) -
/// they are chests, dig sites and dungeon loot:
/// `Temple_Chest_01`, `dff_chest_24`, `gimmick_item_dropset_treasurebox_01`,
/// `clawmachine_capsule_01`, `Action_dig_01`, `gimmick_Dig_land_0001`, the
/// `gimmick_abyssone_bridge_gate_*` set and `gimmick_marni_teleportation_*`.
///
/// (An earlier revision of this docstring said the extra lists sit in records
/// like `player`. They do not. `player` is a **false header**: it occurs 1267
/// times, each preceded by the `u32` 1, which is a nested string field and not
/// a record key. The claim came from a comment in `crates/desert-core/src/gimmick.rs`
/// that has since been corrected; the key-echo discriminator in
/// [`Table::records`] is what rules such headers out. See
/// `docs/findings/2026-09-12-water-wells.md` section 7.)
pub use desert_tools::outputs::Detector;

/// The table body, with the two walks findings section 8 records.
///
/// The Python's `Table` carried the [`Detector`] it was opened with as a
/// default and every walk took an optional override. Here every walk takes the
/// detector outright: one loaded body is walked both ways on a `--loose` build,
/// because it has to know which of its lists the shipped detector would also
/// have found, and a required argument is a stronger guarantee than a default
/// nobody may rely on.
pub struct Table {
    pub b: Vec<u8>,
}

impl Table {
    /// The `gimmickinfo` body, and a description of where it came from: out
    /// of `<tables>/gimmickinfo.staticinfobody` when a directory of extracted
    /// copies is given, else out of the installed game's archives.
    pub fn load(tables: Option<&Path>) -> Result<(Table, String)> {
        use desert_tools::staticinfo::{archive_path, BODY_EXT};
        const TABLE: &str = "gimmickinfo";
        // One flat message, because `main` prints it the way the Python's
        // `die()` did: `items: <what went wrong>`, hint on the second line.
        let hint = "pass --tables DIR (extracted copies, `just extract-tables`) or set \
                    CD_GAME_DIR to the install holding the archives.";
        match tables {
            Some(dir) => {
                let path = dir.join(format!("{TABLE}{BODY_EXT}"));
                let b = std::fs::read(&path)
                    .map_err(|e| anyhow!("cannot read the gimmickinfo table body {}: {e}\n  {hint}", path.display()))?;
                Ok((Table { b }, path.display().to_string()))
            }
            None => {
                let game = desert_tools::paths::game_dir();
                let inner = archive_path(TABLE, BODY_EXT);
                let b = desert_tools::paz::Archive::open(&game)
                    .and_then(|a| a.read(&inner))
                    .map_err(|e| {
                        anyhow!("cannot read the gimmickinfo table body out of the archives under {}: {e:#}\n  {hint}", game.display())
                    })?;
                Ok((Table { b }, format!("{inner} in the archives under {}", game.display())))
            }
        }
    }

    #[inline]
    pub fn u32(&self, o: usize) -> u32 {
        u32::from_le_bytes(self.b[o..o + 4].try_into().unwrap())
    }

    #[inline]
    pub fn u64(&self, o: usize) -> u64 {
        u64::from_le_bytes(self.b[o..o + 8].try_into().unwrap())
    }

    /// Is there a well-formed output block at `at`, per `d`?
    ///
    /// Under [`Detector::Shipped`] this is a faithful port of
    /// `desert_core::gimmick::block_ok`, both equality clauses included: the
    /// two copies of the item id must agree, and so must `PAD_AT` and
    /// `PAD_TAIL_AT`. Under [`Detector::Loose`] the second clause is dropped
    /// and nothing else changes. Which clause buys what, and why `PAD_TAIL_AT`
    /// is not really a pad, is on [`Detector`] and [`ENTRY_KEY_AT`]. The walk
    /// itself calls `desert_tools::outputs` directly; this is what the tests
    /// below pin the two detectors with.
    #[cfg(test)]
    pub fn block_ok(&self, at: usize, d: Detector) -> bool {
        desert_tools::outputs::block_ok(&self.b, at, d)
    }

    /// Greedy, non-overlapping walk for `u32 count` + count blocks.
    pub fn output_lists(&self, d: Detector) -> Vec<(usize, u32)> {
        desert_tools::outputs::output_lists(&self.b, d).into_iter().map(|(o, c)| (o, c as u32)).collect()
    }

    /// Every length-prefixed, NUL-terminated printable string in the body,
    /// as `(offset, text)` in ascending offset order.
    ///
    /// The Python walks maximal runs of printable bytes with the regex
    /// `[\x20-\x7E]{3,120}` and tries every start inside a match except the
    /// last two. This is the same walk written out: a run is chopped into
    /// 120-byte matches the way a greedy regex would chop it, so the set of
    /// candidate offsets is identical. Offsets come out strictly increasing
    /// (the last candidate of a match is at least three bytes before the next
    /// match can start), so the result needs no sort and can hold no
    /// duplicates.
    pub fn strings(&self) -> Vec<(usize, String)> {
        let b = &self.b;
        let n = b.len();
        let mut found: Vec<(usize, String)> = Vec::new();
        let mut i = 0usize;
        while i < n {
            if !(0x20..=0x7E).contains(&b[i]) {
                i += 1;
                continue;
            }
            // Maximal run of printable bytes: [i, end).
            let mut end = i;
            while end < n && (0x20..=0x7E).contains(&b[end]) {
                end += 1;
            }
            let mut p = i;
            while end - p >= 3 {
                let mlen = (end - p).min(120);
                // `range(len(g) - 2)`: the last two starts of a match are
                // never tried, because the regex needed three bytes to match.
                for o in p..p + mlen.saturating_sub(2) {
                    if o < 8 {
                        continue;
                    }
                    let ln = self.u32(o - 4) as usize;
                    // The Python writes `o + ln > n` and then indexes
                    // `b[o + ln]`, which raises IndexError when a candidate
                    // ends exactly at the end of the body. Nothing in the
                    // shipped table reaches it, and a string with no room for
                    // its NUL is not a string, so `>=` is both the fix and
                    // behaviour-identical everywhere Python survives.
                    if ln < 3 || o + ln >= n || b[o + ln] != 0 {
                        continue;
                    }
                    let cand = &b[o..o + ln];
                    if cand.iter().all(|&c| (0x20..=0x7E).contains(&c)) {
                        found.push((o, String::from_utf8_lossy(cand).into_owned()));
                    }
                }
                p += mlen;
            }
            i = end;
        }
        found
    }

    /// `(header offset, key, name)` for every record, echo-discriminated.
    ///
    /// The echo is what separates a record header from a nested string field:
    /// nested fields use the same `u32 len, bytes, NUL` shape, so a plain
    /// backwards scan finds `NatureBuffTrigger` with the bogus key 16777216
    /// where `firewood_0001` should be. A real record repeats its own key just
    /// before a later digits-only id sub-field.
    ///
    /// On a `NameFirst` body (26361232 on) the strict header scan in
    /// `desert_tools::records` replaces all of that: every record's header
    /// validates and nothing else does. The shape is probed off the first
    /// record.
    pub fn records(&self) -> Vec<(usize, u32, String)> {
        use desert_tools::records::{name_first_headers, probe, HeaderShape};
        if probe(&self.b) == Some(HeaderShape::NameFirst) {
            return name_first_headers(&self.b).into_iter().map(|h| (h.off, h.key, h.name)).collect();
        }
        let strs = self.strings();
        let mut out = Vec::new();
        for (i, (o, name)) in strs.iter().enumerate() {
            let (o, name) = (*o, name.as_str());
            if is_digits(name) || name.bytes().any(|c| BAD_NAME_CHARS.contains(&c)) {
                continue;
            }
            let key = self.u32(o - 8);
            if key < MIN_RECORD_KEY {
                continue;
            }
            // `bisect_right(soff, o)` is `i + 1`: offsets are unique and sorted.
            let hi = (i + 1 + ECHO_LOOKAHEAD).min(strs.len());
            for (o2, s2) in &strs[i + 1..hi] {
                if o2 - o > ECHO_WINDOW {
                    break;
                }
                if is_digits(s2) && self.u32(o2 - 8) == key {
                    out.push((o - 8, key, name.to_string()));
                    break;
                }
            }
        }
        out.sort();
        out
    }
}

/// Python's `str.isdigit()` for the ASCII-only strings this walk produces.
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}

// ------------------------------------------------------------------ collect.rs

/// `record key -> (record name, Family)` from the generated key table
/// (`desert_tools::collect_rows`).
pub fn read_collect(path: &Path) -> Result<HashMap<u32, (String, String)>> {
    Ok(desert_tools::collect_rows::read(path)?.into_iter().map(|r| (r.key, (r.name, r.family))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use desert_tools::outputs::{ITEM_TAIL_AT, MAX_QTY};

    /// A 68-byte output block. The named offsets are the point of the test:
    /// the item id lives at `+1` and is echoed at `+60`, `+5` is the only real
    /// pad, and `+64` is the list entry's own key field.
    fn block(item: u32, mn: u64, mx: u64, pad: u32, entry_key: u32) -> [u8; BLOCK] {
        let mut b = [0u8; BLOCK];
        b[0] = 1;
        b[ITEM_AT..ITEM_AT + 4].copy_from_slice(&item.to_le_bytes());
        b[PAD_AT..PAD_AT + 4].copy_from_slice(&pad.to_le_bytes());
        b[MIN_AT..MIN_AT + 8].copy_from_slice(&mn.to_le_bytes());
        b[MAX_AT..MAX_AT + 8].copy_from_slice(&mx.to_le_bytes());
        b[58] = 0xFF;
        b[59] = 0xFF;
        b[ITEM_TAIL_AT..ITEM_TAIL_AT + 4].copy_from_slice(&item.to_le_bytes());
        b[PAD_TAIL_AT..PAD_TAIL_AT + 4].copy_from_slice(&entry_key.to_le_bytes());
        b
    }

    fn table(b: Vec<u8>) -> Table {
        Table { b }
    }

    #[test]
    fn a_well_formed_block_passes_both_detectors() {
        let t = table(block(1001, 1, 5, 0, 0).to_vec());
        assert!(t.block_ok(0, Detector::Shipped));
        assert!(t.block_ok(0, Detector::Loose));
    }

    /// The whole difference between the two populations, in one assertion.
    /// `+64` is not a pad: it is the list entry's key, zero on every gather
    /// record and nonzero on the 142 blocks only `--loose` sees, and that is
    /// exactly what the shipped detector's second equality clause excludes.
    #[test]
    fn a_nonzero_entry_key_is_what_the_pad_clause_excludes() {
        let t = table(block(1001, 1, 5, 0, 77).to_vec());
        assert!(!t.block_ok(0, Detector::Shipped));
        assert!(t.block_ok(0, Detector::Loose));
    }

    /// The clause is an *equality*, not a zero test. A block with `+5` and
    /// `+64` both nonzero and equal is admitted by the shipped detector, which
    /// is why calling either offset "the pad" is only true of the population
    /// the clause happens to admit.
    #[test]
    fn the_pad_clause_is_equality_not_zeroness() {
        let t = table(block(1001, 1, 5, 9, 9).to_vec());
        assert!(t.block_ok(0, Detector::Shipped));
    }

    #[test]
    fn block_shape_rejections() {
        // The item id must agree with its echo at +60.
        let mut b = block(1001, 1, 5, 0, 0);
        b[ITEM_TAIL_AT] = 0xEE;
        assert!(!table(b.to_vec()).block_ok(0, Detector::Shipped));
        assert!(!table(b.to_vec()).block_ok(0, Detector::Loose));

        for mutate in [
            |b: &mut [u8; BLOCK]| b[0] = 0,      // flag byte
            |b: &mut [u8; BLOCK]| b[58] = 0,     // the FF FF marker
            |b: &mut [u8; BLOCK]| b[59] = 0,
        ] {
            let mut b = block(1001, 1, 5, 0, 0);
            mutate(&mut b);
            assert!(!table(b.to_vec()).block_ok(0, Detector::Loose));
        }
        // min must be at least 1, must not exceed max, and max must be inside
        // MAX_QTY - the three clauses that keep arbitrary bytes out.
        assert!(!table(block(1001, 0, 5, 0, 0).to_vec()).block_ok(0, Detector::Loose));
        assert!(!table(block(1001, 6, 5, 0, 0).to_vec()).block_ok(0, Detector::Loose));
        assert!(!table(block(1001, 1, MAX_QTY + 1, 0, 0).to_vec()).block_ok(0, Detector::Loose));
        // A truncated body has no block at all, however good the prefix looks.
        let short = block(1001, 1, 5, 0, 0)[..BLOCK - 1].to_vec();
        assert!(!table(short).block_ok(0, Detector::Loose));
    }

    #[test]
    fn output_lists_walks_greedily_and_does_not_overlap() {
        let mut b = Vec::new();
        b.extend_from_slice(&2u32.to_le_bytes());
        b.extend_from_slice(&block(1001, 1, 5, 0, 0));
        b.extend_from_slice(&block(1002, 2, 2, 0, 0));
        let second = b.len();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&block(1003, 3, 9, 0, 0));
        let t = table(b);
        assert_eq!(t.output_lists(Detector::Shipped), vec![(0, 2), (second, 1)]);
    }

    /// A record header, as the body writes one: `u32 key, u32 len, bytes, NUL`.
    fn rec(out: &mut Vec<u8>, key: u32, name: &str) -> usize {
        out.extend_from_slice(&key.to_le_bytes());
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        let at = out.len();
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        at
    }

    /// The key echo, which is the whole reason `records()` is not a backwards
    /// scan. Nested string fields use the same `u32 len, bytes, NUL` shape as a
    /// record name, so the naive version picks up `NatureBuffTrigger` carrying
    /// the bogus key 16777216 where `firewood_0001` should be. A real record
    /// repeats its own key just before a later digits-only id sub-field, and
    /// nothing else does.
    #[test]
    fn the_key_echo_rules_out_nested_string_fields() {
        let mut b = vec![0u8; 16];
        rec(&mut b, 500001, "firewood_0001");
        rec(&mut b, 500001, "500001"); // the echo: same key, digits-only name
        rec(&mut b, 16777216, "NatureBuffTrigger"); // a nested field, no echo
        let t = table(b);
        let recs = t.records();
        assert!(
            recs.iter().any(|(_, k, n)| *k == 500001 && n == "firewood_0001"),
            "the echoed record must be found: {recs:?}"
        );
        assert!(
            !recs.iter().any(|(_, _, n)| n == "NatureBuffTrigger"),
            "an unechoed nested string must not become a record: {recs:?}"
        );
    }

    /// The echo has to be *after* the name. A digits field before it is some
    /// other record's, and attributing a block to it would put the block in the
    /// wrong record entirely.
    #[test]
    fn an_echo_before_the_name_does_not_count() {
        let mut b = vec![0u8; 16];
        rec(&mut b, 700002, "700002");
        rec(&mut b, 700002, "lonely_record_01");
        let t = table(b);
        assert!(!t.records().iter().any(|(_, _, n)| n == "lonely_record_01"));
    }

    #[test]
    fn strings_needs_a_length_prefix_and_a_terminator() {
        // Printable bytes with no `u32 len` in front and no NUL behind are not
        // a string, however readable they look.
        let t = table(b"\x00\x00\x00\x00\x00\x00\x00\x00plain text".to_vec());
        assert!(t.strings().is_empty());
    }
}
