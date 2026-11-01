//! The auto-loot half of Desert Harvest: what used to be the `desert-looter`
//! crate.
//!
//! [`crate::start`] resolves the game-side anchors (byte signatures, RTTI)
//! through [`entry`], hooks the per-frame sweep function to get a callback on
//! the game thread, and resolves the PickUpItem event descriptor. The gather
//! hotkey forges one PickUpItem event for the nearest gather node and enqueues
//! it from inside the sweep hook.
//!
//! The shared plumbing (`crate::log!`, `crate::safe`, `crate::module`, ...) is
//! re-exported at the crate root, so every module here keeps naming it the way
//! it always did. Only this half's own modules moved, to `crate::looter::`.

// Same split as desert-core: anything that talks to the game or to Win32 is
// #[cfg(windows)], the rest links natively on Linux so `cargo test` runs there.
pub mod config;
pub mod payload;
pub mod recorder;

#[cfg(windows)]
pub mod actors;
/// The auto-gather pacing policy. It was `gatherer.rs` while this was its own
/// crate; renamed so that `crate::gatherer` means only the yield half.
#[cfg(windows)]
pub mod autogather;
#[cfg(windows)]
pub mod events;
#[cfg(windows)]
pub mod game;
#[cfg(windows)]
pub mod hunting;
#[cfg(windows)]
pub mod tables;

/// The helpers the one harvest thread (`crate::entry`) calls for this half:
/// the hook installs, the event API, the yields cache, the config reads.
#[cfg(windows)]
pub(crate) mod entry;
