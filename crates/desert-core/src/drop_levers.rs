//! A one-slot mailbox between two subsystem threads: three numbers one side
//! publishes and the other reads, plus a generation that says whether anything
//! has been published yet and whether it moved since the reader last looked.
//!
//! It exists because the `Pouches` / `Breakables` settings and the code that
//! acts on them live in different crates. The keys belong to `[Harvest]`
//! (they sit beside the other loot multipliers in the menu), and the write
//! path stays in `desert-dispatch`, which already holds the one accessor scan
//! that can reach `dropsetinfo`. Harvest publishes here at startup and on every
//! reload that moves one of the values; dispatch reads it on its own poll.
//!
//! Plumbing, not policy: nothing here knows what a pouch is, what `1` means,
//! or what to do before the first publish. The reader decides all of that.
//!
//! The whole slot is one `AtomicU64`, so a reader never sees half of one
//! publish and half of another, with no lock and nothing that can block:
//!
//! | bits    | field                                                      |
//! | ------- | ---------------------------------------------------------- |
//! | 0..16   | `pouches`, saturated to `u16::MAX`                         |
//! | 16..32  | `breakables`, saturated to `u16::MAX`                      |
//! | 32      | `dry_run`                                                  |
//! | 33..64  | generation: 0 = never published, then 1, 2, ... (skips 0)  |
//!
//! The saturation is a representation limit, not a range check: the
//! publisher's own parser bounds the values far below it.

use std::sync::atomic::{AtomicU64, Ordering};

/// What one side publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levers {
    pub pouches: u32,
    pub breakables: u32,
    pub dry_run: bool,
}

/// One read of the mailbox: the values and the generation they were
/// published under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    pub levers: Levers,
    /// Never 0: a snapshot exists only once something was published.
    pub generation: u32,
}

const VALUE_BITS: u32 = 16;
const VALUE_MASK: u64 = (1 << VALUE_BITS) - 1;
const DRY_BIT: u64 = 1 << 32;
const GEN_SHIFT: u32 = 33;
/// The generation field is 31 bits wide.
const GEN_MASK: u64 = (1 << 31) - 1;

fn pack(l: Levers, generation: u32) -> u64 {
    let p = u64::from(u16::try_from(l.pouches).unwrap_or(u16::MAX));
    let b = u64::from(u16::try_from(l.breakables).unwrap_or(u16::MAX));
    let d = if l.dry_run { DRY_BIT } else { 0 };
    p | (b << VALUE_BITS) | d | ((u64::from(generation) & GEN_MASK) << GEN_SHIFT)
}

fn generation_of(word: u64) -> u32 {
    u32::try_from((word >> GEN_SHIFT) & GEN_MASK).unwrap_or(0)
}

fn unpack(word: u64) -> Option<Snapshot> {
    let generation = generation_of(word);
    if generation == 0 {
        return None;
    }
    let pouches = u32::try_from(word & VALUE_MASK).unwrap_or(0);
    let breakables = u32::try_from((word >> VALUE_BITS) & VALUE_MASK).unwrap_or(0);
    Some(Snapshot {
        levers: Levers { pouches, breakables, dry_run: word & DRY_BIT != 0 },
        generation,
    })
}

/// The next generation after `g`, wrapping inside the 31-bit field and
/// skipping 0, which is reserved for "never published".
fn next_generation(g: u32) -> u32 {
    let n = (u64::from(g) + 1) & GEN_MASK;
    if n == 0 {
        1
    } else {
        u32::try_from(n).unwrap_or(1)
    }
}

/// The slot itself. [`MAILBOX`] is the one the plugin uses; tests make their
/// own.
pub struct Mailbox(AtomicU64);

impl Mailbox {
    /// Empty: [`Mailbox::load`] returns `None` until the first publish.
    pub const fn new() -> Self {
        Mailbox(AtomicU64::new(0))
    }

    /// Store `levers` under the next generation and return that generation.
    /// Every publish bumps it, even one that repeats the previous values; a
    /// publisher that wants to be quiet compares before it calls.
    pub fn publish(&self, levers: Levers) -> u32 {
        let mut generation = 0;
        // `fetch_update` with a closure that always returns `Some` cannot
        // fail; the `Err` arm only exists in its signature.
        let _ = self.0.fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
            generation = next_generation(generation_of(old));
            Some(pack(levers, generation))
        });
        generation
    }

    /// The last publish, or `None` if there has been none.
    pub fn load(&self) -> Option<Snapshot> {
        unpack(self.0.load(Ordering::Acquire))
    }

    /// The current generation, 0 before the first publish. One load, for a
    /// reader that only wants to know whether to look again.
    pub fn generation(&self) -> u32 {
        generation_of(self.0.load(Ordering::Acquire))
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

/// The plugin's one mailbox for the `Pouches` / `Breakables` hand-off.
pub static MAILBOX: Mailbox = Mailbox::new();

/// [`Mailbox::publish`] on [`MAILBOX`].
pub fn publish(levers: Levers) -> u32 {
    MAILBOX.publish(levers)
}

/// [`Mailbox::load`] on [`MAILBOX`].
pub fn load() -> Option<Snapshot> {
    MAILBOX.load()
}

/// [`Mailbox::generation`] on [`MAILBOX`].
pub fn generation() -> u32 {
    MAILBOX.generation()
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Levers = Levers { pouches: 3, breakables: 7, dry_run: true };

    #[test]
    fn empty_until_the_first_publish() {
        let m = Mailbox::new();
        assert_eq!(m.load(), None);
        assert_eq!(m.generation(), 0);
    }

    #[test]
    fn a_publish_round_trips_and_bumps_the_generation() {
        let m = Mailbox::new();
        assert_eq!(m.publish(L), 1);
        assert_eq!(m.load(), Some(Snapshot { levers: L, generation: 1 }));
        let l2 = Levers { pouches: 1, breakables: 100, dry_run: false };
        assert_eq!(m.publish(l2), 2);
        assert_eq!(m.load(), Some(Snapshot { levers: l2, generation: 2 }));
        // The same values again are still a new generation.
        assert_eq!(m.publish(l2), 3);
        assert_eq!(m.generation(), 3);
    }

    #[test]
    fn values_saturate_and_never_bleed_into_their_neighbours() {
        let m = Mailbox::new();
        m.publish(Levers { pouches: u32::MAX, breakables: 0, dry_run: false });
        let s = m.load().map(|s| s.levers);
        assert_eq!(s, Some(Levers { pouches: u32::from(u16::MAX), breakables: 0, dry_run: false }));
        m.publish(Levers { pouches: 0, breakables: 70_000, dry_run: true });
        let s = m.load().map(|s| s.levers);
        assert_eq!(s, Some(Levers { pouches: 0, breakables: u32::from(u16::MAX), dry_run: true }));
    }

    #[test]
    fn the_generation_wraps_without_landing_on_zero() {
        assert_eq!(next_generation(0), 1);
        assert_eq!(next_generation(41), 42);
        let top = u32::try_from(GEN_MASK).unwrap_or(0);
        assert_eq!(next_generation(top), 1);
        // A packed word at the top of the field still unpacks to it.
        let s = unpack(pack(L, top));
        assert_eq!(s, Some(Snapshot { levers: L, generation: top }));
    }

    #[test]
    fn concurrent_readers_never_see_a_torn_publish() {
        let m = std::sync::Arc::new(Mailbox::new());
        let w = std::sync::Arc::clone(&m);
        let writer = std::thread::spawn(move || {
            for i in 1..5000u32 {
                // pouches and breakables always equal, dry_run their parity.
                w.publish(Levers { pouches: i % 100, breakables: i % 100, dry_run: i % 2 == 0 });
            }
        });
        for _ in 0..20_000 {
            if let Some(s) = m.load() {
                assert_eq!(s.levers.pouches, s.levers.breakables, "{s:?}");
                assert_eq!(s.levers.dry_run, s.generation % 2 == 0, "{s:?}");
            }
        }
        writer.join().unwrap();
    }
}
