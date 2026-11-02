//! Resource-output blocks in a `gimmickinfo` record, found exactly the way the
//! plugin finds the ones it multiplies.
//!
//! A port of `desert_core::gimmick::{block_ok, output_lists}`, both equality
//! clauses included, plus the loose variant (the pad clause dropped) that
//! `items --loose` and `gen-collect-names` use to say what the plugin cannot
//! see. `crates/tools/` does not depend on `desert-core`, deliberately, so this
//! is the one copy the tools share: `gen-collect-names` decides which records
//! are rows with it, `items` walks the whole body with it, and `dmm-pack`
//! writes the pack's edits from it. Keep it in step with
//! `crates/desert-core/src/gimmick.rs`; `just test-game`'s pack oracle
//! (`multiply_reproduces_the_dmm_pack_edits`) is what fails when the two part.
//!
//! ```text
//! list    u32 count (1..=64), then count x block
//! block   68 bytes: +0 u8 flag = 1, +1 u32 item id, +5 u32 pad,
//!         +42 u64 minimum, +50 u64 maximum, +58 FF FF,
//!         +60 u32 item id again, +64 u32 (the list entry's key field)
//! ```

/// Bytes per block.
pub const BLOCK: usize = 68;
/// The item id. It is `+1`, not `+5`: bytes `+5..+8` are zero padding
/// (`docs/findings/2026-09-12-water-wells.md` section 5).
pub const ITEM_AT: usize = 1;
/// The item id's echo.
pub const ITEM_TAIL_AT: usize = 60;
pub const PAD_AT: usize = ITEM_AT + 4;
pub const PAD_TAIL_AT: usize = ITEM_TAIL_AT + 4;
/// The `u64` minimum and maximum.
pub const MIN_AT: usize = 42;
pub const MAX_AT: usize = 50;
/// The largest list count the detector accepts.
pub const MAX_COUNT: usize = 64;
/// The largest quantity the detector accepts.
pub const MAX_QTY: u64 = 100_000;

/// Which of the two block detectors a walk uses. A named choice, not a flag.
///
/// Both check the block's shape - flag byte 1 at `+0`, `FF FF` at `+58`, a
/// `u64` min/max pair at `+42`/`+50` inside `1..=MAX_QTY` - and both require
/// the two copies of the item id at [`ITEM_AT`] and [`ITEM_TAIL_AT`] to agree.
/// They differ in one clause:
///
/// * `Shipped` also requires [`PAD_AT`] to equal [`PAD_TAIL_AT`], exactly as
///   `desert_core::gimmick::block_ok` does. **This is the population the
///   plugin sees**, and the only one whose yields the gatherer multiplies.
/// * `Loose` drops that clause and keeps everything else. Blocks only it finds
///   carry a list-entry key at `+64`, and the plugin never edits them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Detector {
    Shipped,
    Loose,
}

impl Detector {
    pub fn name(self) -> &'static str {
        match self {
            Detector::Shipped => "shipped",
            Detector::Loose => "loose",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Detector::Shipped => {
                "both equality clauses, exactly desert_core::gimmick::block_ok - the \
                 population the plugin sees and the gatherer multiplies"
            }
            Detector::Loose => {
                "the item-id clause only; the pad clause dropped. Finds everything the \
                 shipped detector finds plus 16 lists the mod cannot see"
            }
        }
    }

    /// Whether `+5` must equal `+64`. The whole difference between the two.
    pub fn pad_clause(self) -> bool {
        matches!(self, Detector::Shipped)
    }
}

fn u32_at(buf: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(buf: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(buf.get(o..o.checked_add(8)?)?.try_into().ok()?))
}

/// A well-formed output block at `at`, per `d`: flag 1, `FF FF` at `+58`, the
/// item id equal at `+1` and `+60`, (Shipped only) the pads at `+5` and `+64`
/// equal, and `1 <= min <= max <= MAX_QTY`.
pub fn block_ok(buf: &[u8], at: usize, d: Detector) -> bool {
    let Some(b) = at.checked_add(BLOCK).and_then(|end| buf.get(at..end)) else {
        return false;
    };
    if b[0] != 1 || b[58] != 0xFF || b[59] != 0xFF {
        return false;
    }
    if b[ITEM_AT..ITEM_AT + 4] != b[ITEM_TAIL_AT..ITEM_TAIL_AT + 4] {
        return false;
    }
    if d.pad_clause() && b[PAD_AT..PAD_AT + 4] != b[PAD_TAIL_AT..PAD_TAIL_AT + 4] {
        return false;
    }
    let (Some(lo), Some(hi)) = (u64_at(b, MIN_AT), u64_at(b, MAX_AT)) else {
        return false;
    };
    1 <= lo && lo <= hi && hi <= MAX_QTY
}

/// Every resource-output list in `buf`, as `(offset of its count, block
/// count)`: a `u32` count of 1..=[`MAX_COUNT`] followed by that many valid
/// blocks, walked greedily and non-overlapping, exactly as
/// `desert_core::gimmick::output_lists`.
pub fn output_lists(buf: &[u8], d: Detector) -> Vec<(usize, usize)> {
    let n = buf.len();
    let mut lists = Vec::new();
    let mut i = 0usize;
    while i + 4 <= n {
        let count = u32_at(buf, i).unwrap_or(0) as usize;
        if (1..=MAX_COUNT).contains(&count)
            && i + 4 + count * BLOCK <= n
            && (0..count).all(|k| block_ok(buf, i + 4 + k * BLOCK, d))
        {
            lists.push((i, count));
            i += 4 + count * BLOCK;
        } else {
            i += 1;
        }
    }
    lists
}

/// One block of a record, located: which list and which block it is (both
/// 1-based, in walk order) and where it starts in the buffer walked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub list: usize,
    pub block: usize,
    pub at: usize,
}

impl Block {
    pub fn min_at(&self) -> usize {
        self.at + MIN_AT
    }

    pub fn max_at(&self) -> usize {
        self.at + MAX_AT
    }
}

/// Every block [`output_lists`] finds in `buf`, in order.
pub fn blocks(buf: &[u8], d: Detector) -> Vec<Block> {
    output_lists(buf, d)
        .into_iter()
        .enumerate()
        .flat_map(|(li, (off, count))| {
            (0..count).map(move |k| Block { list: li + 1, block: k + 1, at: off + 4 + k * BLOCK })
        })
        .collect()
}

/// What one record pays: its sorted, distinct item ids and how many lists
/// they came from.
pub fn record_pays(rec: &[u8], d: Detector) -> (Vec<u32>, usize) {
    let lists = output_lists(rec, d);
    let mut items: Vec<u32> = lists
        .iter()
        .flat_map(|&(off, count)| (0..count).map(move |k| off + 4 + k * BLOCK + ITEM_AT))
        .filter_map(|o| u32_at(rec, o))
        .collect();
    items.sort_unstable();
    items.dedup();
    (items, lists.len())
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
            b[ITEM_TAIL_AT..ITEM_TAIL_AT + 4].copy_from_slice(&item.to_le_bytes());
            v.extend_from_slice(&b);
        }
        v
    }

    #[test]
    fn output_lists_are_greedy_and_non_overlapping() {
        let mut buf = vec![0u8; 16];
        buf.extend_from_slice(&list(2, 22008));
        buf.extend_from_slice(&[0u8; 8]);
        let found = output_lists(&buf, Detector::Shipped);
        assert_eq!(found, vec![(16, 2)]);
    }

    #[test]
    fn a_block_needs_its_flag_and_its_ff_ff() {
        let mut buf = vec![0u8; 8];
        buf.extend_from_slice(&list(1, 1));
        let at = 12;
        assert!(block_ok(&buf, at, Detector::Shipped));
        buf[at] = 0;
        assert!(!block_ok(&buf, at, Detector::Shipped), "flag byte is what makes the block selective");
        buf[at] = 1;
        buf[at + 58] = 0;
        assert!(!block_ok(&buf, at, Detector::Shipped));
    }

    #[test]
    fn a_block_needs_one_le_min_le_max_le_max_qty() {
        let mut buf = vec![0u8; 8];
        buf.extend_from_slice(&list(1, 1));
        let at = 12;
        buf[at + MIN_AT..at + MIN_AT + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(!block_ok(&buf, at, Detector::Shipped), "a minimum of 0 is not a payout");
        buf[at + MIN_AT..at + MIN_AT + 8].copy_from_slice(&9u64.to_le_bytes());
        assert!(!block_ok(&buf, at, Detector::Shipped), "min above max is not a range");
        buf[at + MAX_AT..at + MAX_AT + 8].copy_from_slice(&(MAX_QTY + 1).to_le_bytes());
        assert!(!block_ok(&buf, at, Detector::Shipped));
    }

    #[test]
    fn the_item_echo_and_the_pad_clause_are_what_the_plugin_checks() {
        let mut buf = vec![0u8; 8];
        buf.extend_from_slice(&list(1, 22008));
        let at = 12;
        buf[at + ITEM_TAIL_AT] ^= 1;
        assert!(!block_ok(&buf, at, Detector::Loose), "the item echo must agree");
        buf[at + ITEM_TAIL_AT] ^= 1;
        buf[at + PAD_TAIL_AT] = 7;
        assert!(!block_ok(&buf, at, Detector::Shipped), "a list-entry key is invisible to the plugin");
        assert!(block_ok(&buf, at, Detector::Loose));
    }

    #[test]
    fn a_record_pays_its_distinct_items() {
        let mut rec = vec![0u8; 5];
        rec.extend_from_slice(&list(2, 9));
        rec.extend_from_slice(&[0u8; 3]);
        rec.extend_from_slice(&list(1, 4));
        assert_eq!(record_pays(&rec, Detector::Shipped), (vec![4, 9], 2));
        assert_eq!(record_pays(&[0u8; 10], Detector::Shipped), (vec![], 0));
    }

    #[test]
    fn blocks_are_numbered_by_list_then_block_from_one() {
        let mut rec = vec![0u8; 5];
        rec.extend_from_slice(&list(2, 9));
        rec.extend_from_slice(&[0u8; 3]);
        let second = rec.len();
        rec.extend_from_slice(&list(1, 4));
        let got = blocks(&rec, Detector::Shipped);
        assert_eq!(
            got,
            vec![
                Block { list: 1, block: 1, at: 9 },
                Block { list: 1, block: 2, at: 9 + BLOCK },
                Block { list: 2, block: 1, at: second + 4 },
            ]
        );
        assert_eq!((got[0].min_at(), got[0].max_at()), (9 + MIN_AT, 9 + MAX_AT));
    }
}
