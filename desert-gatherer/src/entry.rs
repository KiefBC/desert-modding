//! The gatherer subsystem's thread: read the ini, install the two hooks, then
//! watch the ini for the rest of the session.
//!
//! This used to be this crate's `DllMain` plus its `main_thread`. `DllMain`,
//! the host-exe gate, the `CreateThread` and the `log::init` are
//! `desert-tooling`'s now, and so is seeding `DesertTooling.ini`; what is left
//! is [`start`], which is the old thread body verbatim from just after
//! `log::init` onward. The hook installs still happen at the very top of it,
//! with nothing in front of them - see [`start`].

use crate::config::{self, Config};
use crate::gimmick;
use crate::module::MainModule;
use crate::{catch, hook, log, safe};

/// Seconds between two counter summaries in the log, and only when a
/// counter moved since the last one. Record loading is a burst at level
/// load, so this settles into silence.
const SUMMARY_SECS: u64 = 60;

/// How often this subsystem's thread stats `DesertTooling.ini` for a modified
/// time change. The overlay's edit is expected to be picked up within about a
/// second, and this is the whole budget for that: the stat is cheap and the
/// hook never blocks on it either way.
const RELOAD_POLL_SECS: u64 = 1;

fn load_config(path: &std::path::Path) -> Config {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (cfg, warnings) = config::parse(&text);
            for w in warnings {
                crate::log!("[ini] {w}");
            }
            cfg
        }
        Err(_) => {
            crate::log!("[ini] {} not found, using defaults", path.display());
            Config::default()
        }
    }
}

/// `[ini] Enabled=1 DryRun=0 ...`, shared between the startup summary and
/// the reload loop's `[ini] reloaded: ...` line so both read the same way.
fn ini_summary(cfg: &Config) -> String {
    format!(
        "Enabled={} DryRun={} Debug={} Foraging={} Logging={} Mining={} Ore={} \
         Bugs={} Fish={}",
        cfg.enabled as u8,
        cfg.dry_run as u8,
        cfg.debug as u8,
        cfg.foraging,
        cfg.logging,
        cfg.mining,
        cfg.ore,
        cfg.bugs,
        cfg.fish
    )
}

/// The multipliers a re-apply pass just used, for its summary line. At
/// `Enabled=0` every family is effectively 1x - the pass puts the records
/// back to vanilla - and the line says so rather than naming the values
/// sitting unused in the file.
fn live_summary(cfg: &Config) -> String {
    if !cfg.enabled {
        return "Foraging=1 Logging=1 Mining=1 Ore=1 (Enabled=0)".to_string();
    }
    format!(
        "Foraging={} Logging={} Mining={} Ore={}",
        cfg.foraging, cfg.logging, cfg.mining, cfg.ore
    )
}

/// The ini's last-modified time, or `None` if it cannot be stat'd (missing,
/// permissions, mid-write on some filesystems). Used only to notice a
/// change cheaply; the reload loop still re-reads and re-parses on top.
fn ini_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Find the loader, check its prologue really is the 12 bytes we are
/// about to overwrite, and patch it. A refusal here is a plugin that does
/// nothing; a wrong patch is a crash, so every doubt means refusing.
fn install_loader_hook(module: &MainModule) -> bool {
    let target = match gimmick::resolve_record_loader(module.bytes(), module.base) {
        Ok(t) => t,
        Err(e) => {
            crate::log!("[gimmick] record loader NOT found: {e}; yields stay vanilla");
            return false;
        }
    };
    crate::log!("[gimmick] record loader at +0x{:X} (0x{target:X})", module.rva(target));

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

/// Everything a genuinely changed `[Gatherer]` section costs: its warnings, the
/// summary line, the publish, and the re-apply pass that rewrites every record
/// the game has already parsed.
///
/// Split out of the reload loop so the loop can skip it whole. `DesertTooling.ini`
/// carries all three subsystems now, so the overlay's write of a `[Looter]` or an
/// `[Overlay]` key moves the modified time of the file this thread watches -
/// before the merge nothing outside this subsystem could touch it. Running the
/// pass below on somebody else's edit would be a few thousand reads and writes
/// into game memory, plus a log line a second while a slider is being dragged,
/// for numbers that did not move.
fn publish_reload(cfg: &Config, warnings: &[String]) {
    for w in warnings {
        crate::log!("[ini] {w}");
    }
    crate::log!("[ini] reloaded: {}", ini_summary(cfg));
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

/// Run the gathering yield multiplier for the life of the process.
///
/// `desert-tooling` calls this on a thread of its own, straight after
/// `log::init` and the ini seeding, and it never returns except on the two
/// give-up paths below. Everything here is what the old `DesertGatherer.asi`
/// did on its own thread; only the entry point moved.
///
/// There is no boot grace here, unlike Desert Looter: the gimmickinfo
/// table is read during loading, so the hook has to be in place before the
/// game gets that far. That is also why it is safe to patch the prologue -
/// no thread is executing it yet. Nothing may be inserted ahead of the two
/// installs below.
pub fn start() {
    // The one source of truth for which file this subsystem reads is the
    // schema it hands the menu: the overlay writes the keys back into
    // `Section::ini`, so the poll below has to watch that same file rather
    // than a second constant that could drift from it.
    let ini_name = config::schema().ini;
    let ini_path = log::exe_dir().join(&ini_name);
    crate::log!(
        "started ({}), {} gather records known, settings from [{}] of {ini_name}",
        crate::VERSION,
        crate::known_records(),
        config::INI_SECTION
    );
    let cfg = load_config(&ini_path);
    crate::log!("[ini] {}", ini_summary(&cfg));
    if !cfg.enabled {
        // The hook is installed anyway and `LiveConfig::enabled` gates
        // the writing per call. Installing it later, when the ini flips
        // `Enabled` on, is not an option: patching the loader prologue is
        // only safe now, while the game is still loading and no thread
        // can be executing those 12 bytes. It still reads every record,
        // because those bytes carry the vanilla yields `hook::reapply`
        // needs to turn anything up later in the session.
        crate::log!(
            "Enabled=0: the hook is installed and reads records, but writes nothing until the ini says otherwise"
        );
    }
    if cfg.all_vanilla() {
        crate::log!("[ini] every family is at 1x: the hook will read records and write nothing");
    }
    if cfg.dry_run {
        crate::log!("[ini] DryRun=1: the log shows what would change, nothing is written");
    }
    // The hook reads this; publish before the hook can possibly fire.
    config::LIVE.publish(&cfg);

    let Some(module) = MainModule::locate() else {
        crate::log!("could not locate the main module; giving up");
        return;
    };
    crate::log!("[module] base=0x{:X} size=0x{:X}", module.base, module.size);

    let t0 = std::time::Instant::now();
    let hooked = install_loader_hook(&module);
    // Independent of the loader hook: it patches a different function for
    // a different lever (the catch count, `docs/reference-internals.md`
    // section 17), so one failing is no reason to skip the other. Also
    // installed regardless of `Enabled`, for the same reason as the
    // loader hook - `LiveConfig` is what gates the behaviour, because
    // there is no later moment at which patching code is safer.
    let catching = catch::install_catch_hook(&module);
    crate::log!("[hook] resolve+install took {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
    if !hooked && !catching {
        crate::log!("no hook installed; this subsystem is idle and the game is untouched");
        return;
    }

    // From here the thread does two things on a ~1s tick, forever: watch
    // the ini for a modified time change (the overlay's write) and
    // re-publish it live, and print a counters summary every
    // `SUMMARY_SECS`. All the per-record work still happens on the game
    // threads; this thread never touches game memory.
    let mut last_ini_mtime = ini_mtime(&ini_path);
    // What `LIVE` is already publishing. The modified time says the *file*
    // changed; this says whether **this subsystem's section** did.
    let mut live = cfg;
    let mut last_summary_at = std::time::Instant::now();
    let mut last_counts = (0u64, 0u64, 0u64);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(RELOAD_POLL_SECS));

        let mtime = ini_mtime(&ini_path);
        if mtime.is_some() && mtime != last_ini_mtime {
            match std::fs::read_to_string(&ini_path) {
                Ok(text) => {
                    // Only advance the watermark on a successful read: if
                    // the file is mid-write (or briefly missing) this same
                    // change is retried next tick instead of being missed.
                    last_ini_mtime = mtime;
                    let (cfg, warnings) = config::parse(&text);
                    // A `[Looter]` or `[Overlay]` edit moves this file too;
                    // see `publish_reload`.
                    if cfg != live {
                        publish_reload(&cfg, &warnings);
                        live = cfg;
                    }
                }
                Err(_) => {
                    // Keep the live values; try again next tick.
                }
            }
        }

        if last_summary_at.elapsed() >= std::time::Duration::from_secs(SUMMARY_SECS) {
            last_summary_at = std::time::Instant::now();
            let now = hook::counters();
            if now != last_counts {
                crate::log!(
                    "[stat] records seen {}, gather records patched {}, scalars written {}",
                    now.0,
                    now.1,
                    now.2
                );
                last_counts = now;
            }
        }
    }
}
