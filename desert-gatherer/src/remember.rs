//! The vanilla yields of every gather record the loader has handed us, kept so
//! a multiplier changed mid-session has something to act on.
//!
//! The game reads its gimmickinfo table exactly once, about nine seconds after
//! launch, and never calls the loader again for a slot it has already filled
//! (`docs/reference-internals.md` section 16). The hook's edit therefore lands
//! once and the raw bytes are freed a couple of seconds later, so by the time
//! the ini changes the vanilla numbers exist nowhere: the parsed object holds
//! whatever multiplier was in force at boot, and dividing it back out is not
//! the same thing. This table is where they are kept instead — recorded on the
//! way past, from the same bytes the hook was about to edit.
//!
//! # Shape
//!
//! Written by the hook on whichever game threads load records, read by the
//! plugin's main thread. That rules out a `Mutex` (a game thread must never
//! block on us) and a `HashMap` (no allocation on a game thread), so it is a
//! fixed open-addressed table of atomics, probed linearly from `key %
//! MAX_RECORDS` exactly like `hook::first_warning_for`. `desert_dispatch`'s two
//! tables are the same shape and are **deliberately not shared with this one**:
//! the probe is ~25 lines, and the key packing, the field sets and the atomic
//! types all differ, so a generic table would need a trait over atomic storage
//! to save about four lines in each of the three. A slot is claimed with
//! one `compare_exchange` on its key; the fields are filled in afterwards and
//! `ready` is stored last, with `Release`, so a reader that sees `ready` with
//! `Acquire` sees the whole slot. Nothing is ever freed: the table lives as
//! long as the process, which is exactly as long as the records do.
//!
//! [`MAX_RECORDS`] is comfortably above the 276 gather records `collect` knows
//! about (275 from the DMM pack plus the water well), and [`MAX_BLOCKS`]
//! above the 4 blocks the biggest vanilla record has.
//! Both are refusals, not truncations: a record that does not fit is simply not
//! remembered, and the live path leaves it alone rather than half-rewriting it.
//!
//! Pure and always compiled, so it is unit tested natively on Linux.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use desert_core::gimmick::OutputBlock;

/// Slots in the table. 275 gather records are known on build 25116796; this
/// leaves room for a table that grows and keeps the linear probe short.
pub const MAX_RECORDS: usize = 512;
/// Output blocks kept per record. Vanilla records have 1, 3 or 4.
pub const MAX_BLOCKS: usize = 8;

/// One remembered output block: what the raw bytes said before any multiplier.
struct Block {
    item: AtomicU32,
    min: AtomicU64,
    max: AtomicU64,
}

impl Block {
    const fn new() -> Self {
        Block { item: AtomicU32::new(0), min: AtomicU64::new(0), max: AtomicU64::new(0) }
    }
}

/// One record. `key == 0` means free; the key is also the claim, so the
/// `compare_exchange` that sets it is what decides which thread fills the slot.
struct Slot {
    key: AtomicU32,
    idx: AtomicU32,
    nblocks: AtomicU32,
    blocks: [Block; MAX_BLOCKS],
    /// Set last, released; until it is set the slot's fields mean nothing.
    ready: AtomicBool,
}

impl Slot {
    const fn new() -> Self {
        Slot {
            key: AtomicU32::new(0),
            idx: AtomicU32::new(0),
            nblocks: AtomicU32::new(0),
            blocks: [const { Block::new() }; MAX_BLOCKS],
            ready: AtomicBool::new(false),
        }
    }
}

/// A record as it was on disk: its key, the index the loader used, and the
/// vanilla `(item, min, max)` of each of its output blocks in block order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    pub key: u32,
    pub idx: u16,
    pub blocks: Vec<(u32, u64, u64)>,
}

/// The table itself. There is one `static` instance; the type is public so the
/// tests can drive a fresh one instead of the process-wide table they cannot
/// reset.
pub struct Table {
    slots: [Slot; MAX_RECORDS],
}

impl Table {
    pub const fn new() -> Self {
        Table { slots: [const { Slot::new() }; MAX_RECORDS] }
    }

    /// Record `key`'s vanilla blocks, or say why not.
    ///
    /// `false` means nothing was stored: a key of 0, no blocks, more blocks
    /// than [`MAX_BLOCKS`], or a full table. All four are worth nothing more
    /// than a lost live re-apply for that one record, so none of them is worth
    /// a game thread doing anything about.
    ///
    /// A key that is already here is rewritten in place rather than claiming a
    /// second slot. The lazy loader can re-read a record whose slot was
    /// emptied, and it hands us the same vanilla bytes it did the first time,
    /// so the rewrite stores the values that are already there; a reader
    /// racing it sees one or the other and they are equal.
    pub fn remember(&self, key: u32, idx: u16, blocks: &[OutputBlock]) -> bool {
        if key == 0 || blocks.is_empty() || blocks.len() > MAX_BLOCKS {
            return false;
        }
        let start = (key as usize) % MAX_RECORDS;
        for i in 0..MAX_RECORDS {
            // `(start + i) % MAX_RECORDS` is always in range; a miss would just
            // move on to the next probe, which is what the loop does anyway.
            let Some(slot) = self.slots.get((start + i) % MAX_RECORDS) else { continue };
            let ours = match slot.key.compare_exchange(
                0,
                key,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => true,
                Err(v) => v == key,
            };
            if !ours {
                continue;
            }
            slot.idx.store(u32::from(idx), Ordering::Relaxed);
            slot.nblocks.store(blocks.len() as u32, Ordering::Relaxed);
            for (b, src) in slot.blocks.iter().zip(blocks) {
                b.item.store(src.item, Ordering::Relaxed);
                b.min.store(src.min, Ordering::Relaxed);
                b.max.store(src.max, Ordering::Relaxed);
            }
            // Last, and released: everything above must be visible to a reader
            // that sees this.
            slot.ready.store(true, Ordering::Release);
            return true;
        }
        false
    }

    /// Every record remembered so far, in table order (which is key order
    /// modulo the probe, i.e. nothing meaningful — the caller does not care).
    ///
    /// Allocates, so it is for the plugin's own thread only. A slot being
    /// filled in right now is simply not in the result; the next call gets it.
    pub fn snapshot(&self) -> Vec<Remembered> {
        let mut out = Vec::new();
        for slot in &self.slots {
            if !slot.ready.load(Ordering::Acquire) {
                continue;
            }
            let n = (slot.nblocks.load(Ordering::Relaxed) as usize).min(MAX_BLOCKS);
            let blocks = slot
                .blocks
                .iter()
                .take(n)
                .map(|b| {
                    (
                        b.item.load(Ordering::Relaxed),
                        b.min.load(Ordering::Relaxed),
                        b.max.load(Ordering::Relaxed),
                    )
                })
                .collect();
            out.push(Remembered {
                key: slot.key.load(Ordering::Relaxed),
                idx: slot.idx.load(Ordering::Relaxed) as u16,
                blocks,
            });
        }
        out
    }

    /// How many records are remembered and readable right now.
    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.ready.load(Ordering::Acquire)).count()
    }

    /// True before the first record has been through the loader.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

/// The process-wide table: written by the hook, read by the reload loop.
static TABLE: Table = Table::new();

/// Record a gather record's vanilla blocks. See [`Table::remember`].
pub fn remember(key: u32, idx: u16, blocks: &[OutputBlock]) -> bool {
    TABLE.remember(key, idx, blocks)
}

/// Every record remembered so far. See [`Table::snapshot`].
pub fn snapshot() -> Vec<Remembered> {
    TABLE.snapshot()
}

/// How many records are remembered.
pub fn len() -> usize {
    TABLE.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(specs: &[(u32, u64, u64)]) -> Vec<OutputBlock> {
        specs
            .iter()
            .enumerate()
            .map(|(n, &(item, min, max))| OutputBlock { offset: n * 68, item, min, max })
            .collect()
    }

    fn only(t: &Table, key: u32) -> Remembered {
        let mut hits: Vec<Remembered> =
            t.snapshot().into_iter().filter(|r| r.key == key).collect();
        assert_eq!(hits.len(), 1, "exactly one slot per key");
        hits.remove(0)
    }

    #[test]
    fn a_record_goes_in_and_comes_back_out() {
        let t = Table::new();
        assert!(t.is_empty());
        assert!(t.remember(17030001, 42, &blocks(&[(101, 1, 3), (102, 2, 8)])));
        assert_eq!(t.len(), 1);
        assert_eq!(
            only(&t, 17030001),
            Remembered { key: 17030001, idx: 42, blocks: vec![(101, 1, 3), (102, 2, 8)] }
        );
    }

    #[test]
    fn many_records_coexist() {
        let t = Table::new();
        // Keys that collide in the probe (same modulo) as well as ones that do not.
        for n in 0..64u32 {
            let key = 1 + n * u32::try_from(MAX_RECORDS).unwrap_or(1);
            assert!(t.remember(key, n as u16, &blocks(&[(n, u64::from(n) + 1, 8)])), "{key}");
        }
        assert_eq!(t.len(), 64);
        for n in 0..64u32 {
            let key = 1 + n * u32::try_from(MAX_RECORDS).unwrap_or(1);
            let r = only(&t, key);
            assert_eq!(r.idx, n as u16);
            assert_eq!(r.blocks, vec![(n, u64::from(n) + 1, 8)]);
        }
    }

    #[test]
    fn the_same_key_is_rewritten_in_place() {
        let t = Table::new();
        assert!(t.remember(7, 1, &blocks(&[(9, 1, 2)])));
        assert!(t.remember(7, 1, &blocks(&[(9, 1, 2)])));
        assert_eq!(t.len(), 1, "a reload must not claim a second slot");
        // And a genuinely different reading replaces the old one whole.
        assert!(t.remember(7, 5, &blocks(&[(9, 3, 4), (10, 1, 1)])));
        assert_eq!(t.len(), 1);
        assert_eq!(only(&t, 7), Remembered { key: 7, idx: 5, blocks: vec![(9, 3, 4), (10, 1, 1)] });
    }

    #[test]
    fn a_full_table_refuses() {
        let t = Table::new();
        for n in 0..MAX_RECORDS {
            let key = u32::try_from(n + 1).unwrap_or(1);
            assert!(t.remember(key, 0, &blocks(&[(1, 1, 1)])), "slot {n}");
        }
        assert_eq!(t.len(), MAX_RECORDS);
        assert!(!t.remember(0xDEAD, 0, &blocks(&[(1, 1, 1)])), "no slot left to claim");
        // A key already in the full table is still rewritable.
        assert!(t.remember(1, 3, &blocks(&[(1, 2, 2)])));
        assert_eq!(only(&t, 1).idx, 3);
    }

    #[test]
    fn nonsense_is_refused_rather_than_stored() {
        let t = Table::new();
        assert!(!t.remember(0, 0, &blocks(&[(1, 1, 1)])), "key 0 is the free marker");
        assert!(!t.remember(5, 0, &[]), "a record with no output blocks");
        let too_many = blocks(&[(1, 1, 1); MAX_BLOCKS + 1]);
        assert!(!t.remember(5, 0, &too_many), "more blocks than a slot holds");
        assert!(t.is_empty());
        // Exactly MAX_BLOCKS is fine.
        assert!(t.remember(5, 0, &blocks(&[(1, 1, 1); MAX_BLOCKS])));
        assert_eq!(only(&t, 5).blocks.len(), MAX_BLOCKS);
    }

    #[test]
    fn snapshot_only_returns_ready_slots() {
        let t = Table::new();
        assert!(t.snapshot().is_empty());
        // A slot claimed but not yet published is invisible: this is the state
        // a reader can catch a game thread in, half way through `remember`.
        if let Some(slot) = t.slots.first() {
            slot.key.store(1234, Ordering::Relaxed);
        }
        assert!(t.snapshot().is_empty(), "a claimed but unready slot is not remembered");
        assert_eq!(t.len(), 0);
        assert!(t.remember(1234, 8, &blocks(&[(1, 1, 1)])), "the claiming key can still finish");
        assert_eq!(only(&t, 1234).idx, 8);
    }

    #[test]
    fn the_static_table_is_the_same_table() {
        assert!(remember(0xC0FFEE, 11, &blocks(&[(3, 2, 4)])));
        assert!(len() >= 1);
        let mine = snapshot().into_iter().find(|r| r.key == 0xC0FFEE);
        assert_eq!(mine.map(|r| r.blocks), Some(vec![(3, 2, 4)]));
    }
}
