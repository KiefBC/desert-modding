//! The yield half's pieces of the one harvest thread (`crate::entry`): find
//! and hook the record loader, write the table dump, re-apply a changed
//! config to the records already parsed, and the counters line.
//!
//! This used to be the old `desert-gatherer` crate's `start()` plus helpers.
//! The thread itself is `crate::entry::start` now, and it installs these hooks
//! before anything else, with nothing in front of them: the gimmickinfo table
//! is read during loading, so there is no boot grace for this half.

use crate::gatherer::config::{self, Config};
use crate::gatherer::{dump, hook};
use crate::gimmick;
use crate::module::MainModule;
use crate::{log, safe};

/// Seconds between two counter summaries in the log, and only when a
/// counter moved since the last one. Record loading is a burst at level
/// load, so this settles into silence.
pub(crate) const SUMMARY_SECS: u64 = 60;

/// The multipliers a re-apply pass just used, for its summary line. At
/// `Enabled=0` every family is effectively 1x - the pass puts the records
/// back to vanilla - and the line says so rather than naming the values
/// sitting unused in the file.
fn live_summary(cfg: &Config) -> String {
    if !cfg.enabled {
        return "Foraging=1 Logging=1 Mining=1 Ore=1 Money=1 (Enabled=0)".to_string();
    }
    format!(
        "Foraging={} Logging={} Mining={} Ore={} Money={}",
        cfg.foraging, cfg.logging, cfg.mining, cfg.ore, cfg.money
    )
}

/// `DesertTooling.gimmickinfo.bin` beside the log, where `DumpTable=1` writes
/// the table body and where the justfile and `tools/src/paths.rs` look for it
/// first. The same `exe_dir()` the log and the looter's yields cache use.
fn dump_path() -> std::path::PathBuf {
    log::exe_dir().join(dump::FILE_NAME)
}

/// Write the table body the hook copied, if it has copied one since the last
/// tick. On this thread, never on the game thread that took the copy: 22 MB of
/// disk IO is not something to do in the middle of the game's loading.
///
/// `std::fs::write` truncates, so a file from an earlier launch is replaced
/// whole rather than appended to - and the copy is taken once per session, so
/// this runs at most once per launch.
pub(crate) fn write_pending_dump() {
    let Some(bytes) = hook::take_pending_dump() else { return };
    let path = dump_path();
    match std::fs::write(&path, &bytes) {
        Ok(()) => {
            crate::log!("[dump] gimmickinfo table body: {} bytes -> {}", bytes.len(), path.display())
        }
        Err(e) => crate::log!("[dump] write FAILED: {}: {e}", path.display()),
    }
}

/// Find the record loader, cheapest search first.
///
/// `gimmickinfo` is named by one of the two `lea` encodings on every build seen
/// so far, and those two are half the scan: `AccessorSites::scan(img, None)`
/// searches them alone, 0.25 s against 0.50 s over build 25246367's 363 MB
/// image. The base is still passed to [`AccessorSites::record_loader`], which
/// wants it only as the addend - the base the cheap scan declines is the one
/// that reads a name out of a pointer cell, and `gimmickinfo`'s name is not in
/// one.
///
/// **This subsystem is the one that cannot afford the other half.** The game
/// reads the gimmickinfo table during loading, so there is no boot grace here
/// and the hook has to be installed before that happens; every 0.25 s is margin
/// on a deadline the other subsystems do not have.
///
/// Betting on an encoding is exactly what this project does not do - the old
/// single-encoding scan reached 111 of the 149 tables and that was a bug worth
/// fixing - so it is a bet with a fallback. A build that emitted this accessor
/// as a `mov r8` instead costs one extra scan and one line in the log, not a
/// vanilla session, and the line is what says the assumption stopped holding.
///
/// The stream layout comes off the same sites, so it costs no second scan:
/// the accessor that names `gimmickinfo` is also the code that fills the
/// stream the hook reads ([`gimmick::AccessorSites::stream_layout`]).
fn resolve_loader(module: &MainModule) -> Option<(usize, Result<gimmick::StreamLayout, String>)> {
    let img = module.bytes();
    let found = |sites: gimmick::AccessorSites| {
        sites
            .record_loader(img, module.base, gimmick::GIMMICK_TABLE)
            .map(|t| (t, sites.stream_layout(img, gimmick::GIMMICK_TABLE)))
    };
    match gimmick::AccessorSites::scan(img, None).and_then(found) {
        Ok(r) => return Some(r),
        Err(e) => crate::log!(
            "[gimmick] the record loader is not reachable through the lea encodings ({e}); \
             searching all four"
        ),
    }
    match gimmick::AccessorSites::scan(img, Some(module.base)).and_then(found) {
        Ok(r) => Some(r),
        Err(e) => {
            crate::log!("[gimmick] record loader NOT found: {e}; yields stay vanilla");
            None
        }
    }
}

/// Find the loader, check its prologue really is the 12 bytes we are
/// about to overwrite, and patch it. A refusal here is a plugin that does
/// nothing; a wrong patch is a crash, so every doubt means refusing.
pub(crate) fn install_loader_hook(module: &MainModule) -> bool {
    let Some((target, stream)) = resolve_loader(module) else {
        return false;
    };
    crate::log!("[gimmick] record loader at +0x{:X} (0x{target:X})", module.rva(target));
    // Before the hook goes in, so the first call already reads the right
    // offsets. A fill that does not parse is not a refusal: the first call
    // probes the known layouts against the live stream instead.
    match stream {
        Ok(l) => {
            hook::set_stream_layout(l);
            crate::log!(
                "[gimmick] stream layout: {} (parsed from the accessor's fill sequence)",
                hook::describe_layout(&l)
            );
        }
        Err(e) => crate::log!(
            "[gimmick] stream layout not parsed ({e}); probing the known layouts on the first call"
        ),
    }

    let mut have = [0u8; gimmick::LOADER_STOLEN];
    if !safe::read_into(target, &mut have) {
        crate::log!("[hook] loader prologue at +0x{:X} unreadable; NOT hooking", module.rva(target));
        return false;
    }
    if have[..] != gimmick::LOADER_PROLOGUE[..] {
        crate::log!(
            "[hook] loader prologue is {} not {}; NOT hooking",
            hook::hex(&have),
            hook::hex(&gimmick::LOADER_PROLOGUE)
        );
        return false;
    }

    // SAFETY: `target` came from `resolve_record_loader`, which finds the
    // function by content inside the running image, and its first
    // `LOADER_STOLEN` bytes were just read back and compared byte for byte
    // with `LOADER_PROLOGUE`, so what the stub copies is that exact whole,
    // position-independent prologue. This runs before the gimmickinfo table
    // is loaded, so no thread is executing those bytes, and
    // `hook::on_record_load` has the four-integer-argument shape the stub
    // calls it with.
    match unsafe { hook::install(target, gimmick::LOADER_STOLEN, hook::on_record_load) } {
        Ok(h) => {
            crate::log!(
                "[hook] record loader +0x{:X} -> stub 0x{:X}; original bytes: {}",
                module.rva(h.target),
                h.stub,
                hook::hex(&h.original)
            );
            true
        }
        Err(e) => {
            crate::log!("[hook] record loader install FAILED: {e}");
            false
        }
    }
}

/// Everything a genuinely changed yield config costs: the publish, and the
/// re-apply pass that rewrites every record the game has already parsed.
///
/// Called by the harvest thread only when this half's config moved, never
/// for an auto-loot key: the overlay's write of any `[Harvest]` key (or of
/// another section's) moves the modified time of the file the thread watches,
/// and running the pass below on that would be a few thousand reads and writes
/// into game memory, plus a log line a second while a slider is being dragged,
/// for numbers that did not move. The warnings and the `[ini] reloaded:` line
/// are the caller's, said once for the whole section.
pub(crate) fn apply_reload(cfg: &Config) {
    config::LIVE.publish(cfg);
    // The game read its gimmickinfo table once, seconds after launch, and
    // will not read it again; the only way a change reaches this session is
    // by rewriting the records it already parsed. Not done at startup:
    // there, the load path itself is about to apply the very same numbers.
    let outcome = hook::reapply();
    if outcome.manager_known {
        crate::log!(
            "{} {}: {}",
            if cfg.dry_run { "[dry] would re-apply" } else { "[live] re-applied" },
            live_summary(cfg),
            outcome.summary()
        );
        if let Some(why) = outcome.warning() {
            crate::log!("[live] WARN {why}");
        }
    }
}

/// The `[stat]` counters line, when a counter moved since the last one.
/// `last` is what the previous line said. Record loading is a burst at level
/// load, so this settles into silence.
pub(crate) fn log_counters(last: &mut hook::Counters) {
    let now = hook::counters();
    if now != *last {
        crate::log!("[stat] {}", now.line());
        hook::check_silent_paths();
        *last = now;
    }
}
