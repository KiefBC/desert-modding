//! Desert Gatherer - the gathering yield multiplier subsystem of
//! `DesertTooling.asi`.
//!
//! It hooks the game's gimmickinfo record loader and multiplies the minimum
//! and maximum yield scalars in the raw table bytes of each gather record, in
//! the instant between the loader being entered and the deserializer reading
//! the record. Four independent families - Foraging, Logging, Mining and Ore
//! Nodes - each get their own multiplier from the `[Gatherer]` section of
//! `DesertTooling.ini`. They are the DMM pack's own families; the one record
//! the pack never had is the water well, which sits in Foraging (water drawn
//! from a well is gathered out of the world like everything else there) and
//! comes from `tools/extra-families.json`. This replaces the DMM JSON pack in
//! `desert-gatherer-dmm/`, which patched the same scalars on disk; **the pack
//! and DMM's built-in gathering multiplier must be unmounted, or yields
//! multiply twice.**
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
//! This crate is an rlib with no `DllMain` of its own: `desert-tooling` owns
//! the entry point, the host-exe gate, the log file and the shared ini, and
//! calls [`start`] on a thread of its own. The rules desert-core enforces hold
//! here too: no panics (`panic = "abort"`) and every foreign read through
//! `desert_core::safe`.

// The shared plumbing lives in desert-core, re-exported under the names this
// workspace has always used, so `crate::log!`, `crate::safe::read`,
// `crate::module::MainModule` keep resolving inside every module here. `log`
// names both a module and the exported macro; one `use` brings in both.
// `hook` is deliberately NOT re-exported: this crate has its own `hook`
// module, which re-exports `desert_core::hook::{install, hex}` so that
// `crate::hook::install` and `crate::hook::on_record_load` both resolve.
pub use desert_core::{collect, gimmick, ini, log, pattern, pe, rtti, schema, trampoline};
#[cfg(windows)]
pub use desert_core::{module, safe};

// Same split as desert-core: anything that talks to the game or to Win32 is
// #[cfg(windows)], the rest links natively on Linux so `cargo test
// --target x86_64-unknown-linux-gnu` runs the unit tests here.
pub mod config;
pub mod remember;

#[cfg(windows)]
pub mod catch;
#[cfg(windows)]
pub mod hook;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What this subsystem's log lines are tagged with in the one shared
/// `DesertTooling.log`.
///
/// `desert_core::log!` expands to `log::write_tagged(crate::LOG_TAG, ...)`, and
/// `crate::` in a `macro_rules!` body resolves at the **call site's** crate, so
/// this one line is what makes every `crate::log!` in this crate compile and
/// come out as `[gatherer]`. Nothing else names it.
pub const LOG_TAG: &str = "gatherer";

/// How many gather records `desert_core::collect` knows about (276: the DMM
/// pack's 275 plus the water well). The ceiling on the plugin's own per-record
/// log lines.
pub fn known_records() -> usize {
    collect::COLLECT_RECORDS.len()
}

#[cfg(windows)]
mod entry;
#[cfg(windows)]
pub use entry::start;

#[cfg(test)]
mod tests {
    #[test]
    fn core_is_linked() {
        assert!(super::known_records() > 200, "the gather record table came from desert-core");
    }
}
