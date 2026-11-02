//! Desert Harvest - the auto-loot and gathering-yield subsystem of
//! `DesertTooling.asi`.
//!
//! Two halves that act on the same in-game system (gather nodes, ground items,
//! creatures caught by hand) and used to be two crates:
//!
//! - [`looter`]: auto-loot. Forges the game's own `PickUpItem` event for the
//!   nearest gather node from inside a per-frame sweep hook, plus the hotkeys,
//!   the survey and the `Hunting` multiplier.
//! - [`gatherer`]: the yield multipliers. Hooks the gimmickinfo record loader
//!   and multiplies each gather record's yield scalars before the game parses
//!   it, plus the catch-count patch for `Bugs` and `Fish`.
//!
//! One `[Harvest]` section of `DesertTooling.ini` ([`config`]), one `[harvest]`
//! log tag, one menu tab and one thread ([`start`]). `desert-tooling` owns the
//! `DllMain`, the host-exe gate, the log file and the shared ini.

// The shared plumbing lives in desert-core, re-exported under the names this
// workspace has always used, so `crate::log!`, `crate::safe::read`,
// `crate::module::MainModule` keep resolving inside every module of both
// halves. `log` names both a module and the exported macro; one `use` brings
// in both. `hook` is the core module: the auto-loot half installs through it,
// and the yield half names its own `crate::gatherer::hook` instead.
pub use desert_core::{collect, gimmick, ini, log, pattern, pe, rtti, schema, telemetry, trampoline};
#[cfg(windows)]
pub use desert_core::{hook, hotkey, module, safe};

pub mod config;
pub mod gatherer;
pub mod looter;
pub mod migrate;
/// Which `characterinfo` rows are fish, by name; read by both halves.
#[cfg(windows)]
pub mod species;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The tag every line this crate logs carries in the one shared
/// `DesertTooling.log`.
///
/// `desert_core::log!` expands to `log::write_tagged(crate::LOG_TAG, ...)`, and
/// `crate::` in a `macro_rules!` body resolves at the **call site's** crate, so
/// this one line is what makes every `crate::log!` in both halves compile and
/// come out as `[harvest]`. Nothing else names it.
pub const LOG_TAG: &str = "harvest";

/// How many gather records `desert_core::collect` knows about (279: the DMM
/// pack's 275, plus the water well, plus the three money props). The ceiling
/// on the yield half's own per-record log lines.
pub fn known_records() -> usize {
    collect::COLLECT_RECORDS.len()
}

/// The harvest thread. Never returns; `desert-tooling` calls it on a thread
/// of its own.
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
