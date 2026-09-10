//! The player's world position, published by one subsystem for another to draw.
//!
//! The overlay's contract is that it never touches game memory: no signatures,
//! no RVAs, nothing to rebase after a game update (`desert-overlay/src/lib.rs`).
//! A coordinate readout would break that if the menu resolved the actor manager
//! for itself, and the ini - the only other channel between subsystems - is a
//! file on disk, which is no place for a value that moves thirty times a
//! second. So the looter, which already holds the manager and the player actor,
//! publishes here and the overlay reads here. Neither one learns anything about
//! the other.
//!
//! This only works because all three subsystems link into one `.asi` (0.3.0
//! onwards): these statics are one instance in one module. Three separate DLLs
//! would each get their own copy and the readout would sit at "unavailable"
//! forever.
//!
//! # Shape
//!
//! One writer (the looter's plugin thread, every 30 ms) and one reader (the
//! overlay's render thread, every frame), which rules out a `Mutex`: a render
//! thread must never block on us, exactly as a game thread must never block on
//! the gatherer's [`crate::gimmick`] hook. Three floats plus a timestamp do not
//! fit in one atomic, so this is a seqlock - the writer bumps an odd sequence
//! number, writes, then bumps it even again, and a reader that sees an odd
//! number or a number that moved under it tries again. The reader's retries are
//! bounded: a frame that cannot get a clean sample draws the last thing it had
//! rather than spinning inside somebody else's present call.
//!
//! Nothing here is allocated, locked or dropped, so it is pure and always
//! compiled, and the tests below run natively on Linux.

use std::sync::atomic::{fence, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A point in the game's world, in the game's own units.
///
/// **`y` is altitude**, not a map coordinate: `docs/reference-internals.md`
/// section 9 has birds surveyed at y 543-559 against a player standing at y
/// 535. The two numbers that answer "where am I on the map" are `x` and `z`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// How long a published position is worth drawing.
///
/// Generous against the publisher's 30 ms cadence on purpose: its loop has a
/// three-second sleep in the branch that waits for the pickup descriptor, and a
/// readout that blinks out during it would be reporting on the looter's
/// internals rather than on the player. Anything past this is a publisher that
/// has stopped - a subsystem that never started, or a world that went away.
pub const FRESH: Duration = Duration::from_secs(4);

/// [`FRESH`] in the units [`AT_MS`] is kept in.
fn fresh_ms() -> u64 {
    u64::try_from(FRESH.as_millis()).unwrap_or(u64::MAX)
}

/// Reader attempts before giving up. The writer holds the sequence odd for the
/// four relaxed stores between its two bumps, so one retry is already more than
/// this ever needs; the bound exists so a render thread cannot spin.
const MAX_TRIES: u32 = 8;

/// Even means the fields are stable, odd means a write is in progress.
static SEQ: AtomicU64 = AtomicU64::new(0);
static X: AtomicU32 = AtomicU32::new(0);
static Y: AtomicU32 = AtomicU32::new(0);
static Z: AtomicU32 = AtomicU32::new(0);
/// Milliseconds since [`epoch`] at the last publish. Zero means "no position":
/// the world is not up, or the player actor was unreadable this tick.
static AT_MS: AtomicU64 = AtomicU64::new(0);

/// The process-wide zero for [`AT_MS`]. `Instant` cannot live in an atomic, so
/// ages are kept as milliseconds from the first call to this.
fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Now, in the same units [`AT_MS`] holds. Saturating: a process would have to
/// run for half a billion years to reach the ceiling, and the alternative is a
/// cast that wraps.
pub fn now_ms() -> u64 {
    u64::try_from(epoch().elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Publish the player's position, or `None` when there isn't one to publish.
///
/// Called from the publishing subsystem's own thread and from nowhere else:
/// this is a single-writer seqlock, and a second writer would let two odd
/// bumps interleave into a sequence number that looks stable while the fields
/// are half written.
pub fn publish(p: Option<Position>) {
    let s = SEQ.load(Ordering::Relaxed);
    // Odd: a reader that arrives now retries instead of reading torn fields.
    SEQ.store(s.wrapping_add(1), Ordering::Relaxed);
    fence(Ordering::Release);
    if let Some(p) = p {
        X.store(p.x.to_bits(), Ordering::Relaxed);
        Y.store(p.y.to_bits(), Ordering::Relaxed);
        Z.store(p.z.to_bits(), Ordering::Relaxed);
        // Never zero: that is the "no position" marker, and a publish in the
        // first millisecond of the process would otherwise read as one.
        AT_MS.store(now_ms().max(1), Ordering::Relaxed);
    } else {
        AT_MS.store(0, Ordering::Relaxed);
    }
    SEQ.store(s.wrapping_add(2), Ordering::Release);
}

/// The last published position, if there is one and it is still fresh.
pub fn read() -> Option<Position> {
    read_as_of(now_ms())
}

/// [`read`] against a caller-supplied clock, which is what makes staleness
/// testable without sleeping.
pub fn read_as_of(now: u64) -> Option<Position> {
    for _ in 0..MAX_TRIES {
        let before = SEQ.load(Ordering::Acquire);
        if !before.is_multiple_of(2) {
            // A write is in progress; let it finish.
            std::hint::spin_loop();
            continue;
        }
        let (x, y, z, at) = (
            X.load(Ordering::Relaxed),
            Y.load(Ordering::Relaxed),
            Z.load(Ordering::Relaxed),
            AT_MS.load(Ordering::Relaxed),
        );
        fence(Ordering::Acquire);
        if SEQ.load(Ordering::Relaxed) != before {
            // The fields moved under us; nothing read above is trustworthy.
            continue;
        }
        if at == 0 || now.saturating_sub(at) > fresh_ms() {
            return None;
        }
        let p = Position { x: f32::from_bits(x), y: f32::from_bits(y), z: f32::from_bits(z) };
        // A NaN here would come from a game read that passed `is_finite` and
        // then went through `to_bits`, which cannot happen - but the readout
        // formats whatever it is handed, and "NaN" on screen is worse than a
        // blank.
        return (p.x.is_finite() && p.y.is_finite() && p.z.is_finite()).then_some(p);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // One process, one set of statics, and cargo runs tests on a thread pool:
    // a second `#[test]` in here would publish underneath this one. Everything
    // this module has to say therefore gets asserted in one test.
    #[test]
    fn publish_read_roundtrip_and_staleness() {
        let p = Position { x: 1234.5, y: 535.0, z: -2100.25 };
        publish(Some(p));
        let at = AT_MS.load(Ordering::Relaxed);
        assert_ne!(at, 0, "a published position is never stamped zero");
        assert!(SEQ.load(Ordering::Relaxed).is_multiple_of(2), "the sequence rests even");
        assert_eq!(read_as_of(at), Some(p));

        // Still fresh at the boundary, gone past it.
        assert_eq!(read_as_of(at + fresh_ms()), Some(p));
        assert_eq!(read_as_of(at + fresh_ms() + 1), None);

        // An explicit clear reads as absent however recent it is.
        publish(None);
        assert_eq!(read_as_of(now_ms()), None);
        assert!(SEQ.load(Ordering::Relaxed).is_multiple_of(2));

        // And publishing again brings it back.
        publish(Some(p));
        assert_eq!(read_as_of(AT_MS.load(Ordering::Relaxed)), Some(p));

        // A non-finite value never reaches the caller, however it got in.
        X.store(f32::NAN.to_bits(), Ordering::Relaxed);
        assert_eq!(read_as_of(AT_MS.load(Ordering::Relaxed)), None);
    }
}
