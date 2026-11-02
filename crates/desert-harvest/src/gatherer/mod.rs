//! The gathering yield half of Desert Harvest: what used to be the
//! `desert-gatherer` crate.
//!
//! It hooks the game's gimmickinfo record loader and multiplies the minimum
//! and maximum yield scalars in the raw table bytes of each gather record, in
//! the instant between the loader being entered and the deserializer reading
//! the record. Five independent families - Foraging, Logging, Mining, Ore
//! Nodes and Money - each get their own multiplier from the `[Harvest]`
//! section of `DesertTooling.ini`. The first four are the DMM pack's own
//! families; the one record the pack never had is the water well, which sits
//! in Foraging (water drawn from a well is gathered out of the world like
//! everything else there) and comes from `crates/tools/extra-families.json`. Money is
//! this plugin's own family and the odd one out: the three coin and silver bar
//! props placed in the world read an output block like any other node, so the
//! same edit reaches them, but what it multiplies is money rather than a
//! material - it is the first balance lever here, and so it ships at 1.
//!
//! This replaces the DMM JSON pack in `desert-gatherer-dmm/`, which patched
//! the same scalars on disk; **the pack and DMM's built-in gathering
//! multiplier must be unmounted, or yields multiply twice.**
//!
//! Two further multipliers, Bugs and Fish, scale the creatures the player
//! catches by hand. Those are not gimmick records and have no yields in any
//! table - the amount is an immediate in the game's code - so that one is a
//! second, separate hook that replaces the immediate with whatever the ini
//! says (`catch.rs`, `docs/reference-internals.md` section 17).
//!
//! Nothing is hard-coded to an address: the loader is found by content
//! (`desert_core::gimmick::resolve_record_loader`), the output blocks by
//! their 68-byte signature and the catch site by a 21-byte signature that
//! hits once. Every step that fails logs and leaves vanilla yields behind.
//!
//! The thread that installs both hooks and watches the ini is the one harvest
//! thread, `crate::start`; `entry` holds this half's pieces of it. The rules
//! desert-core enforces hold here too: no panics (`panic = "abort"`) and every
//! foreign read through `desert_core::safe`.

// Same split as desert-core: anything that talks to the game or to Win32 is
// #[cfg(windows)], the rest links natively on Linux so `cargo test
// --target x86_64-unknown-linux-gnu` runs the unit tests here.
//
// `hook` here is this half's own module (the record-loader callback), and it
// re-exports `desert_core::hook::{install, hex}`, so `crate::gatherer::hook::`
// names both. The crate root's `crate::hook` is the core module, which the
// auto-loot half uses; this half only ever names its own.
pub mod config;
pub mod dump;
pub mod remember;

#[cfg(windows)]
pub mod catch;
#[cfg(windows)]
pub mod hook;

/// The helpers the one harvest thread (`crate::entry`) calls for this half:
/// the loader hook install, the table dump, the reload pass.
#[cfg(windows)]
pub(crate) mod entry;
