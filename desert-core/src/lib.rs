//! Shared plumbing for the Crimson Desert ASI plugins.
//!
//! This crate ships nothing on its own; it is linked into `desert-looter` and
//! `desert-gatherer`. It holds exactly the things that must not exist twice:
//! the crash-safe logger, the guarded game-memory reads, the inline
//! trampoline hook, PE/RTTI/pattern scanning, the ini reader, the settings
//! schema the overlay draws every mod's menu from, and the gather record table
//! both mods classify nodes with.
//!
//! Module split, same rule as the plugins: modules that touch the Win32 API
//! are `#[cfg(windows)]`, everything else compiles (and is unit tested)
//! natively on Linux, so `cargo test --target x86_64-unknown-linux-gnu` works.

pub mod collect;
pub mod creature;
pub mod gimmick;
pub mod ini;
pub mod log;
pub mod pattern;
pub mod pe;
pub mod rtti;
pub mod schema;
pub mod trampoline;

#[cfg(windows)]
pub mod hook;
#[cfg(windows)]
pub mod hotkey;
#[cfg(windows)]
pub mod module;
#[cfg(windows)]
pub mod safe;
