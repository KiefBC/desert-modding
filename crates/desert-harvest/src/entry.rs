//! The one harvest thread: read `[Harvest]`, install every hook of both
//! halves, then run the auto-loot loop and watch the ini for the rest of the
//! session.
//!
//! Built from the two old subsystems' `start()`s, which ran on two threads.
//! The order inside is what kept them on two: the yield hooks go in first,
//! with nothing waiting in front of them, because the game reads the
//! gimmickinfo table while it is still loading and the record loader's
//! prologue can only be patched before any thread runs it. Auto-loot's
//! signature scan and its three hooks come next, and its 20-second boot grace
//! is a deadline inside the loop, not a wait in front of anything.

use crate::config::{self, Config};
use crate::gatherer;
use crate::hotkey::Hotkey;
use crate::log;
use crate::looter::autogather::Gatherer;
use crate::looter::entry::{
    beep, install_enqueue_hook, install_sweep_hook, load_yields, publish_position, resolve_descriptor,
    resolve_event_api, save_yields, tid_now,
};
use crate::looter::{events, game, hunting};
use crate::module::MainModule;

/// How often this thread stats `DesertTooling.ini` for a modified time
/// change. The overlay's edit is expected to be picked up within about a
/// second, and this is the whole budget for that: the stat is cheap and no
/// hook and no game thread ever waits on it.
const RELOAD_POLL_SECS: u64 = 1;

/// The ini's last-modified time, or `None` if it cannot be stat'd
/// (missing, permissions, mid-write on some filesystems).
fn ini_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Read and parse the ini, with the modification time the parsed text
/// belongs to. `None` means "do not act on this": missing, unreadable,
/// empty, or written while it was being read - the modified time is taken
/// before and after the read and has to agree, so an overlay rewriting the
/// file in place is not caught half way. The caller keeps the config it
/// already has and tries the same file again on the next poll.
fn read_config(path: &std::path::Path) -> Option<(std::time::SystemTime, Config, Vec<String>)> {
    let before = ini_mtime(path)?;
    let text = std::fs::read_to_string(path).ok()?;
    if text.trim().is_empty() || ini_mtime(path)? != before {
        return None;
    }
    let (cfg, warnings) = config::parse(&text);
    Some((before, cfg, warnings))
}

/// Startup read: every failure is defaults, with the reason logged.
fn load_config(path: &std::path::Path) -> (Option<std::time::SystemTime>, Config) {
    match read_config(path) {
        Some((mtime, cfg, warnings)) => {
            for w in warnings {
                crate::log!("[ini] {w}");
            }
            (Some(mtime), cfg)
        }
        None => {
            crate::log!("[ini] {} missing, empty or unreadable, using defaults", path.display());
            (None, Config::default())
        }
    }
}

/// The startup lines that say what the yield half will and will not do with
/// the config it was given.
fn log_yield_intent(cfg: &Config) {
    let y = &cfg.gatherer;
    if y.all_vanilla() {
        crate::log!("[ini] every family is at 1x: the hook will read records and write nothing");
    }
    if y.dry_run {
        crate::log!("[ini] DryRun=1: the log shows what the yield multipliers would change, nothing is written");
    }
    // Said here as well as by the hook when it decides: this line is at the
    // top of the log where the ini is summarised, the hook's is ~9 s later
    // beside the stream size it decided on - and the ini can change between
    // the two, so the hook reads the live values again rather than trusting
    // this one.
    if y.dump_table && y.dry_run {
        crate::log!(
            "[ini] DumpTable=1: the gimmickinfo table body will be written beside the log on first load"
        );
    } else if y.dump_table {
        crate::log!("[ini] DumpTable=1 ignored: needs DryRun=1 so the copy is vanilla");
    }
}

/// Run Desert Harvest for the life of the process. `desert-tooling` calls this
/// on a thread of its own, straight after `log::init`, the ini seeding and the
/// migration, and it never returns except when the main module cannot be
/// found.
///
/// Nothing may be inserted ahead of the two yield-hook installs below beyond
/// reading the ini: there is no boot grace for them.
pub fn start() {
    crate::log!(
        "Desert Harvest {} starting, {} gather records known, settings from [{}]",
        crate::VERSION,
        crate::known_records(),
        config::INI_SECTION
    );
    // The one ini this subsystem reads, named by its own schema so the menu,
    // the seeded file and this poll can never disagree about it.
    let ini_path = log::exe_dir().join(config::schema().ini);
    let (mut ini_seen, mut cfg) = load_config(&ini_path);
    crate::log!("[ini] {}", config::ini_summary(&cfg));
    if !cfg.enabled() {
        // This does not return: every prologue can only be patched now, while
        // the game is still loading and no thread is executing it, so the
        // hooks go in regardless and `Enabled` gates what they do instead.
        // That is what lets the ini turn harvest back on without a restart.
        // The yield hook still reads every record, because those bytes carry
        // the vanilla yields `hook::reapply` needs to turn anything up later.
        crate::log!(
            "Enabled=0: every hook is installed, but auto-loot and the hotkeys stay idle and the \
             yield hook reads records and writes nothing, until the ini says otherwise"
        );
    }
    log_yield_intent(&cfg);

    // The yield hook reads this; publish before it can possibly fire.
    gatherer::config::LIVE.publish(&cfg.gatherer);
    events::set_log_received(cfg.enabled() && cfg.looter.log_received);
    hunting::set_multiplier(if cfg.enabled() { cfg.looter.hunting } else { 1 });

    // Once for both halves; `events::set_module` still wants its own copy below.
    let Some(module) = MainModule::locate() else {
        crate::log!("could not locate the main module; giving up");
        return;
    };
    crate::log!("[module] base=0x{:X} size=0x{:X}", module.base, module.size);

    // The yield hooks first. The catch hook is independent of the loader
    // hook: it patches a different function for a different lever (the catch
    // count, `docs/reference-internals.md` section 17), so one failing is no
    // reason to skip the other. Both go in regardless of `Enabled`, because
    // there is no later moment at which patching code is safer;
    // `gatherer::config::LIVE` is what gates the behaviour.
    let t0 = std::time::Instant::now();
    let yield_hooked = gatherer::entry::install_loader_hook(&module);
    let catching = gatherer::catch::install_catch_hook(&module);
    crate::log!("[hook] resolve+install took {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
    if !yield_hooked && !catching {
        // Used to be the end of the yield thread. Now it is one line, and the
        // auto-loot half carries on below.
        crate::log!("[hook] no yield hook installed; the multipliers are off and the game's yields are untouched");
    }

    // Auto-loot: the signature walk, then its three hooks, installed right
    // away for the same reason as the two above.
    let t0 = std::time::Instant::now();
    let anchors = game::resolve(&module);
    crate::log!(
        "[sig] {} found, {} missing, {} actor-manager vtable(s), {:.0} ms",
        anchors.hits.len(),
        anchors.missing.len(),
        anchors.actor_manager_vtables.len(),
        t0.elapsed().as_secs_f64() * 1000.0
    );
    let hooked = install_sweep_hook(&module, &anchors);
    let recorder = install_enqueue_hook(&module, &anchors);
    // The Hunting multiplier's hook, installed regardless of what the key is
    // set to: the callback reads the live value on every grant.
    hunting::install(&module, &anchors);
    let api_ok = resolve_event_api(&module, &anchors);
    if let Some(m2) = MainModule::locate() {
        events::set_module(m2);
    }
    if cfg.enabled() {
        // The "hooks are in" acknowledgement. Not sounded when the ini says
        // harvest is off; nothing will happen until it says 1.
        beep();
    }

    let mut auto = Gatherer::new(&cfg.looter);
    load_yields();
    // Give the game its loading phase before we start walking heap pointers.
    let mut world: Option<game::World> = None;
    let boot_grace = std::time::Duration::from_secs(20);
    let mut census_done = false;
    let mut next_world_try = std::time::Instant::now() + boot_grace;
    let mut k_toggle = Hotkey::new(cfg.looter.key_toggle);
    let mut k_scan = Hotkey::new(cfg.looter.key_scan);
    let mut k_gather = Hotkey::new(cfg.looter.key_gather);
    let mut k_record = Hotkey::new(cfg.looter.key_record);
    let mut descriptor_ok = false;
    let mut hook_reported = false;
    let ini_poll = std::time::Duration::from_secs(RELOAD_POLL_SECS);
    let mut next_ini_poll = std::time::Instant::now() + ini_poll;
    let summary_every = std::time::Duration::from_secs(gatherer::entry::SUMMARY_SECS);
    let mut last_summary_at = std::time::Instant::now();
    let mut last_counts = gatherer::hook::Counters::default();
    let mut last_catch: Option<String> = None;
    loop {
        // Once a second, and above the `Enabled` gate on purpose: the yield
        // half's work here has to keep running under `Enabled=0` - that is
        // what puts vanilla numbers back - and the ini is watched here and
        // nowhere else. This is the plugin thread, never a hook and never the
        // game thread. A stat a second, and a read only when the file moved.
        if std::time::Instant::now() >= next_ini_poll {
            next_ini_poll = std::time::Instant::now() + ini_poll;
            // First: the table copy is 22 MB of heap held until this runs,
            // and the log line is what says the file is there.
            gatherer::entry::write_pending_dump();
            if ini_mtime(&ini_path).is_some_and(|t| Some(t) != ini_seen) {
                // A failed read leaves `ini_seen` alone, so the next poll
                // tries the same file again.
                if let Some((mtime, new_cfg, warnings)) = read_config(&ini_path) {
                    ini_seen = Some(mtime);
                    // The watermark moves either way, but the work below only
                    // runs when **this section** changed: one ini carries
                    // every subsystem, so the overlay's write of an
                    // `[Overlay]` or `[Dispatch]` key moves the modified time
                    // of the file this thread watches. Identical text also
                    // means identical warnings, already logged.
                    if new_cfg != cfg {
                        let old = std::mem::replace(&mut cfg, new_cfg);
                        for w in warnings {
                            crate::log!("[ini] {w}");
                        }
                        crate::log!("[ini] reloaded: {}", config::ini_summary(&cfg));
                        if old.looter != cfg.looter {
                            let (o, n) = (&old.looter, &cfg.looter);
                            events::set_log_received(n.enabled && n.log_received);
                            hunting::set_multiplier(if n.enabled { n.hunting } else { 1 });
                            auto.apply(n);
                            // Rebuilt only when the binding actually moved: a
                            // new poller starts with "not held", which would
                            // fire once for a key that happens to be down.
                            if o.key_toggle != n.key_toggle {
                                k_toggle = Hotkey::new(n.key_toggle);
                            }
                            if o.key_scan != n.key_scan {
                                k_scan = Hotkey::new(n.key_scan);
                            }
                            if o.key_gather != n.key_gather {
                                k_gather = Hotkey::new(n.key_gather);
                            }
                            if o.key_record != n.key_record {
                                k_record = Hotkey::new(n.key_record);
                            }
                            if o.enabled != n.enabled {
                                crate::log!(
                                    "[ini] Enabled={}: automatic gathering and the hotkeys are {}, and the yields are {} from the next gather (the hooks stay where they are)",
                                    n.enabled as u8,
                                    if n.enabled { "back on" } else { "off" },
                                    if n.enabled { "multiplied again" } else { "vanilla" }
                                );
                            }
                        }
                        // Only when a yield key moved: an auto-loot slider
                        // must never cost a re-apply pass over game memory.
                        if old.gatherer != cfg.gatherer {
                            gatherer::entry::apply_reload(&cfg.gatherer);
                        }
                    }
                }
            }
            if last_summary_at.elapsed() >= summary_every {
                last_summary_at = std::time::Instant::now();
                gatherer::entry::log_counters(&mut last_counts);
                gatherer::catch::log_counters(&mut last_catch);
            }
        }
        // Polled even while disabled, so a press made with Enabled=0 is
        // consumed rather than fired the moment it goes back to 1.
        let (press_toggle, press_record, press_gather, press_scan) =
            (k_toggle.pressed(), k_record.pressed(), k_gather.pressed(), k_scan.pressed());
        // Above the `Enabled` gate, and deliberately: the overlay's
        // coordinate readout is fed from here, and a readout that goes blank
        // because somebody switched auto-gathering off would be reporting on
        // this subsystem rather than on the player. Finding the world is the
        // same work either way and it is done once.
        if world.is_none() && std::time::Instant::now() >= next_world_try {
            world = game::find_world(&module, &anchors);
            if world.is_none() {
                next_world_try = std::time::Instant::now() + std::time::Duration::from_secs(3);
            }
        }
        publish_position(world.as_ref());
        if !cfg.enabled() {
            std::thread::sleep(std::time::Duration::from_millis(30));
            continue;
        }
        if world.is_some() && api_ok && !descriptor_ok {
            // Same cadence as the world retry: the table exists once the game
            // is past loading, which the world being found implies.
            if let Some(api) = events::api() {
                descriptor_ok = resolve_descriptor(&module, &api);
                if !descriptor_ok {
                    next_world_try = std::time::Instant::now() + std::time::Duration::from_secs(3);
                    std::thread::sleep(std::time::Duration::from_secs(3));
                }
            }
        }
        if hooked && !hook_reported && events::game_thread_id() != 0 {
            hook_reported = true;
            crate::log!("[hook] game thread id {} (main thread here is {})", events::game_thread_id(), tid_now());
        }
        if press_toggle {
            let on = auto.toggle();
            crate::log!("[key] auto-gather {}", if on { "ON" } else { "OFF" });
            beep();
        }
        if press_record {
            beep();
            if !recorder {
                crate::log!("[record] enqueue hook not installed");
            } else if events::toggle_recording() {
                crate::log!(
                    "[record] ON: logging what the game queues (cap {} lines, {} per descriptor)",
                    crate::looter::recorder::RECORD_CAP,
                    crate::looter::recorder::RECORD_PER_DESC
                );
            }
        }
        if press_gather {
            beep();
            if !hooked {
                crate::log!("[gather] no sweep hook; cannot send on the game thread");
            } else {
                match &world {
                    Some(w) => auto.manual(&module, w),
                    None => crate::log!("[gather] actor manager not found yet"),
                }
            }
        }
        if hooked {
            if let Some(w) = &world {
                auto.tick(&module, w);
            }
        }
        if events::take_yield_dirty() {
            save_yields();
        }
        if press_scan {
            beep();
            match &world {
                Some(w) => {
                    let l = &cfg.looter;
                    if l.debug && !census_done {
                        census_done = true;
                        game::census(&module, w);
                        game::dump_maps(&module, w);
                    }
                    let t = std::time::Instant::now();
                    game::survey(&module, w, l.scan_range, l.survey_lines as usize, l.debug);
                    crate::log!("[survey] done in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
                }
                None => crate::log!("[survey] actor manager not found yet (retrying every 3 s)"),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}
