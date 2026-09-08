//! Desert Looter - gathering auto-loot for Crimson Desert, as an ASI plugin.
//!
//! Current stage: **first write**. On load it resolves the game-side anchors
//! (byte signatures, RTTI), hooks the per-frame sweep function to get a
//! callback on the game thread, and resolves the PickUpItem event descriptor.
//! The gather hotkey forges one PickUpItem event for the nearest gather node
//! and enqueues it from inside the sweep hook.

// The shared plumbing lives in desert-core. Re-exported under the names this
// crate has always used, so `crate::log!`, `crate::safe::read`, `crate::pe`,
// ... keep resolving inside every module here. `log` names both a module and
// the exported macro; one `use` brings in both.
pub use desert_core::{collect, ini, log, pattern, pe, rtti, schema, trampoline};
#[cfg(windows)]
pub use desert_core::{hook, hotkey, module, safe};

// Desert Looter's own modules. Same split as desert-core: anything that talks
// to the game or to Win32 is #[cfg(windows)], the rest links natively on Linux
// so `cargo test` runs there.
pub mod config;
pub mod payload;

#[cfg(windows)]
pub mod actors;
#[cfg(windows)]
pub mod events;
#[cfg(windows)]
pub mod game;
#[cfg(windows)]
pub mod gatherer;
#[cfg(windows)]
pub mod tables;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INI_NAME: &str = "DesertLooter.ini";
/// Written beside the game exe. Named here, not in desert-core, so each
/// plugin gets its own file.
pub const LOG_NAME: &str = "DesertLooter.log";

/// Only this process is the game. The ASI loader (winmm.dll) also gets pulled
/// into helper processes started from bin64 (crashpad_handler.exe), and each
/// of those would otherwise run its own copy of us.
pub const GAME_EXE: &str = "CrimsonDesert.exe";

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
    use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows_sys::Win32::System::Threading::{CreateThread, GetCurrentProcessId};
    use windows_sys::Win32::UI::WindowsAndMessaging::MB_OK;

    use crate::config::{self, Config};
    use crate::hotkey::Hotkey;
    use crate::module::MainModule;
    use crate::gatherer::Gatherer;
    use crate::{events, game, hook, log, schema};

    /// The `area_sweep` signature hits 15 bytes into the function; the
    /// function starts with three 5-byte `mov [rsp+x],reg` spills, which are
    /// exactly the bytes the hook steals (build 25116796, verified by
    /// disassembly; the reference mod uses the same 0xF).
    const SWEEP_HIT_OFFSET: usize = 0xF;
    const SWEEP_STOLEN: usize = 0xF;
    /// `mov [rsp+0x10],rbx; mov [rsp+0x18],rsi; mov [rsp+0x20],rdi`
    const SWEEP_PROLOGUE: [u8; 15] = [
        0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7C, 0x24, 0x20,
    ];

    /// `enqueue` starts `mov [rsp+8],rbx; push rdi; sub rsp,0x20; mov rbx,[rdx+0x38]`:
    /// 14 bytes, the count the reference mod steals too. Byte 12 is the
    /// ModRM the signature wildcards.
    const ENQUEUE_STOLEN: usize = 0xE;
    const ENQUEUE_PROLOGUE: [u8; 14] = [
        0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0x5A, 0x38,
    ];

    /// Observer hook on the game's enqueue, for the record key.
    fn install_enqueue_hook(module: &MainModule, anchors: &game::Anchors) -> bool {
        let Some(target) = anchors.get("enqueue") else {
            crate::log!("[hook] enqueue signature missing; no recorder");
            return false;
        };
        let mut have = [0u8; 14];
        if !crate::safe::read_into(target, &mut have)
            || have[..12] != ENQUEUE_PROLOGUE[..12]
            || have[13] != ENQUEUE_PROLOGUE[13]
        {
            crate::log!("[hook] enqueue prologue is {} not the expected bytes; NOT hooking", hook::hex(&have));
            return false;
        }
        // SAFETY: `target` is the unique `enqueue` signature hit inside the
        // main module and the 14 bytes there were just read back and matched
        // against `ENQUEUE_PROLOGUE`, so the `ENQUEUE_STOLEN` (0xE) bytes the
        // stub copies really are whole, position-independent instructions.
        // This runs during load, before any thread executes that prologue, and
        // `events::on_enqueue` has the four-integer-argument shape the stub
        // calls it with.
        match unsafe { hook::install(target, ENQUEUE_STOLEN, events::on_enqueue) } {
            Ok(h) => {
                crate::log!("[hook] enqueue +0x{:X} -> stub 0x{:X}; original bytes: {}", module.rva(h.target), h.stub, hook::hex(&h.original));
                true
            }
            Err(e) => {
                crate::log!("[hook] enqueue install FAILED: {e}");
                false
            }
        }
    }

    /// Patch the sweep function's prologue. Done right after signature
    /// resolution, while the game is still loading and no thread runs it.
    fn install_sweep_hook(module: &MainModule, anchors: &game::Anchors) -> bool {
        let Some(hit) = anchors.get("area_sweep+0xF") else {
            crate::log!("[hook] area_sweep signature missing; no game-thread callback");
            return false;
        };
        let target = hit - SWEEP_HIT_OFFSET;
        let mut have = [0u8; 15];
        if !crate::safe::read_into(target, &mut have) || have != SWEEP_PROLOGUE {
            crate::log!(
                "[hook] area_sweep prologue at +0x{:X} is {} not the expected spills; NOT hooking",
                module.rva(target), hook::hex(&have)
            );
            return false;
        }
        // SAFETY: `target` is the `area_sweep` signature hit backed up by
        // `SWEEP_HIT_OFFSET` and the 15 bytes there were just read back and
        // matched against `SWEEP_PROLOGUE`, so the `SWEEP_STOLEN` (0xF) bytes
        // the stub copies are the three whole, position-independent register
        // spills. This runs during load, before any thread executes that
        // prologue, and `events::on_sweep` has the four-integer-argument shape
        // the stub calls it with.
        match unsafe { hook::install(target, SWEEP_STOLEN, events::on_sweep) } {
            Ok(h) => {
                crate::log!(
                    "[hook] area_sweep +0x{:X} -> stub 0x{:X}; original bytes: {}",
                    module.rva(h.target), h.stub, hook::hex(&h.original)
                );
                true
            }
            Err(e) => {
                crate::log!("[hook] area_sweep install FAILED: {e}");
                false
            }
        }
    }

    fn resolve_event_api(module: &MainModule, anchors: &game::Anchors) -> bool {
        match events::resolve(module, anchors) {
            Ok(api) => {
                crate::log!(
                    "[event] prepare=+0x{:X} desc_by_id=+0x{:X} alloc_event=+0x{:X} enqueue=+0x{:X} queue_slot=+0x{:X} desc_mask=+0x{:X}",
                    module.rva(api.prepare), module.rva(api.desc_by_id), module.rva(api.alloc_event),
                    module.rva(api.enqueue), module.rva(api.queue_slot), module.rva(api.desc_mask)
                );
                if api.steal_check != 0 && api.steal_ctx != 0 {
                    crate::log!("[event] steal_check=+0x{:X} ctx=+0x{:X}", module.rva(api.steal_check), module.rva(api.steal_ctx));
                } else {
                    crate::log!("[event] steal check NOT resolved: every pickup (gather nodes and ground items) will be treated as owned and skipped");
                }
                events::set_api(api);
                true
            }
            Err(e) => {
                crate::log!("[event] api resolve FAILED: {e}");
                false
            }
        }
    }

    /// Walk the descriptor table once the game has built it (after the boot grace).
    fn resolve_descriptor(module: &MainModule, api: &events::EventApi) -> bool {
        let t = std::time::Instant::now();
        match events::find_descriptor(module, api, events::PICKUP_DESCRIPTOR) {
            Ok(d) => {
                let note = if d.id == events::PICKUP_ID_EXPECTED && d.payload_size as usize == events::PICKUP_PAYLOAD_SIZE {
                    "as expected"
                } else {
                    "DIFFERS from the reference mod's 2057/13"
                };
                crate::log!(
                    "[event] {} id={} payload={} dispatch={} desc=0x{:X} ({note}, {:.0} ms)",
                    d.name, d.id, d.payload_size, d.dispatch, d.ptr, t.elapsed().as_secs_f64() * 1000.0
                );
                events::set_descriptor(d);
                match events::find_descriptor(module, api, events::HANDLE_GAME_EVENT) {
                    Ok(h) => {
                        crate::log!("[event] {} id={} payload={} (yield learning armed)", h.name, h.id, h.payload_size);
                        events::set_handle_descriptor(h.ptr);
                    }
                    Err(e) => crate::log!("[event] {}: {e}; yields will not be learned", events::HANDLE_GAME_EVENT),
                }
                // The catch event is optional: without it insects are simply
                // not caught, and gathering carries on exactly as before.
                match events::find_descriptor(module, api, events::CATCH_DESCRIPTOR) {
                    Ok(c) => {
                        let note = if c.id == events::CATCH_ID_EXPECTED
                            && c.payload_size as usize == events::CATCH_PAYLOAD_SIZE
                        {
                            "as expected"
                        } else {
                            "DIFFERS from the recorded 2048/8"
                        };
                        crate::log!(
                            "[event] {} id={} payload={} dispatch={} ({note})",
                            c.name, c.id, c.payload_size, c.dispatch
                        );
                        events::set_catch_descriptor(c);
                    }
                    Err(e) => crate::log!("[event] {}: {e}; bugs will not be caught", events::CATCH_DESCRIPTOR),
                }
                true
            }
            Err(e) => {
                crate::log!("[event] descriptor: {e} ({:.0} ms)", t.elapsed().as_secs_f64() * 1000.0);
                false
            }
        }
    }

    /// `DesertLooter.yields` beside the log: one `record=item` pair per line.
    fn yields_path() -> std::path::PathBuf {
        log::exe_dir().join("DesertLooter.yields")
    }

    /// Files without this header come from a build that also learned from
    /// ground items (wrong: their record is generic) and are ignored.
    const YIELDS_HEADER: &str = "# desert-looter yields v2";

    fn load_yields() {
        let Ok(text) = std::fs::read_to_string(yields_path()) else { return };
        if text.lines().next().map(str::trim) != Some(YIELDS_HEADER) {
            crate::log!("[yield] ignoring an old-format DesertLooter.yields; it will be rewritten");
            return;
        }
        let pairs: Vec<(u16, u32, u32)> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .filter_map(|l| {
                let (r, rest) = l.trim().split_once('=')?;
                let (i, c) = rest.split_once(',').unwrap_or((rest, "1"));
                Some((r.trim().parse().ok()?, i.trim().parse().ok()?, c.trim().parse().unwrap_or(1)))
            })
            .collect();
        crate::log!("[yield] {} learned record->item pairs loaded", pairs.len());
        events::load_yields(pairs);
    }

    fn save_yields() {
        let mut body = format!("{YIELDS_HEADER}\n");
        body.extend(events::yields_snapshot().iter().map(|(r, i, c)| format!("{r}={i},{c}\n")));
        // Best effort; the file is a cache and is rebuilt by playing.
        let _ = std::fs::write(yields_path(), body);
    }

    fn tid_now() -> u32 {
        // SAFETY: `GetCurrentThreadId` takes no arguments and only reads the
        // calling thread's own TEB; it is sound to call from any thread.
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
    }

    /// The plugin's only UI: an audible acknowledgement of a hotkey.
    fn beep() {
        // SAFETY: `MessageBeep` takes a plain sound-type flag, borrows nothing
        // of ours and has no thread affinity; it is sound to call from the
        // plugin thread at any time.
        unsafe { MessageBeep(MB_OK) };
    }

    fn host_exe_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    /// How often the plugin thread stats `DesertLooter.ini` for a modified
    /// time change. The overlay plugin's edit is expected to be picked up
    /// within about a second, and this is the whole budget for that: the stat
    /// is cheap and no hook and no game thread ever waits on it.
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

    /// `[ini] Enabled=1 Debug=0 ...`, shared between the startup summary and
    /// the reload loop's `[ini] reloaded: ...` line so both read the same way.
    fn ini_summary(cfg: &Config) -> String {
        format!(
            "Enabled={} Debug={} LogReceived={} ScanRange={} GatherRange={} AutoGather={} GatherUnarmed={} GatherItems={} GatherGear={} GatherForaging={} GatherLogging={} GatherMining={} GatherOre={} GatherBugs={} BagTab={} StackLimit={} GatherInterval={} NodeCooldown={} KeyToggle=0x{:02X} KeyScan=0x{:02X} KeyGather=0x{:02X} KeyRecord=0x{:02X}",
            cfg.enabled as u8, cfg.debug as u8, cfg.log_received as u8, cfg.scan_range, cfg.gather_range, cfg.auto_gather as u8,
            cfg.gather_unarmed as u8, cfg.gather_items as u8, cfg.gather_gear as u8,
            cfg.gather_foraging as u8, cfg.gather_logging as u8, cfg.gather_mining as u8, cfg.gather_ore as u8, cfg.gather_bugs as u8,
            cfg.bag_tab.map(|t| t.to_string()).unwrap_or_else(|| "auto".into()), cfg.stack_limit, cfg.gather_interval_ms, cfg.node_cooldown_ms,
            cfg.key_toggle, cfg.key_scan, cfg.key_gather, cfg.key_record
        )
    }

    /// Write `DesertLooter.overlay.ini` beside the exe, so Desert Overlay can
    /// draw this plugin's settings without knowing anything about it.
    ///
    /// Regenerated at every launch, and only actually written when the text
    /// differs, so a matching file costs one read. A failure is a WARN and
    /// nothing else: the overlay simply does not offer this section, and the
    /// plugin itself is unaffected - the ini stays the only thing it reads.
    ///
    /// On the plugin's own thread, after `log::init`, never in `DllMain`.
    fn write_schema() {
        let section = config::schema();
        let name = section.schema_file_name();
        let banner = format!(
            "{name} - written by Desert Looter {} every time the game starts.\n\
             It tells Desert Overlay what {} contains and how to draw it. Editing\n\
             this file has no effect: the plugin regenerates it at the next launch.\n\
             Change the settings in {} instead.",
            crate::VERSION,
            crate::INI_NAME,
            crate::INI_NAME
        );
        match schema::write_beside(&log::exe_dir(), &section, &banner) {
            schema::Written::Written => crate::log!("[schema] wrote {name}"),
            schema::Written::Unchanged => crate::log!("[schema] {name} is current"),
            schema::Written::Failed(why) => {
                crate::log!("[schema] WARN could not write {name}: {why}")
            }
        }
    }

    /// Create `DesertLooter.ini` beside the exe, at every key's default, when
    /// the file is not there at all.
    ///
    /// This is the fallback for someone who dropped only the `.asi` into
    /// `bin64` without the commented template the release zip ships: instead
    /// of having nothing to edit, they get a bare file with every key present.
    /// An existing ini is never read, rewritten or replaced - the create is
    /// `create_new`, so a file that appears in the race window wins and the
    /// user's own settings can never be clobbered.
    ///
    /// A failure is a WARN and nothing else. `Config::default()` already
    /// covers a missing ini, so the plugin behaves identically either way;
    /// the only thing lost is the file to edit.
    ///
    /// On the plugin's own thread, after `log::init`, never in `DllMain`.
    fn write_default_ini() {
        let banner = format!(
            "{} was not found, so Desert Looter {} created it with every\n\
             key at its default. Edit it here or from Desert Overlay's in-game menu; it\n\
             is re-read while the game runs, once a second.\n\
             \n\
             The copy that ships in the release zip has a comment explaining every key.\n\
             This one is bare. Nothing here is regenerated: your edits survive, and a key\n\
             you add by hand is left alone.",
            crate::INI_NAME,
            crate::VERSION
        );
        match schema::create_ini_if_missing(&log::exe_dir(), &config::schema(), &banner) {
            schema::Written::Written => crate::log!(
                "[ini] {} was missing, so it was created with every key at its default",
                crate::INI_NAME
            ),
            // The overwhelmingly common case: the file is there. `load_config`
            // is about to log every value, so a line saying so is pure noise.
            schema::Written::Unchanged => {}
            schema::Written::Failed(why) => crate::log!(
                "[ini] WARN could not create {}: {why}; the defaults are in effect",
                crate::INI_NAME
            ),
        }
    }

    /// Runs on its own thread for the life of the process.
    unsafe extern "system" fn main_thread(_param: *mut c_void) -> u32 {
        log::init(crate::LOG_NAME);
        // SAFETY: `GetCurrentProcessId` takes no arguments and only reads this
        // process's own PEB; it is sound to call from any thread.
        let pid = unsafe { GetCurrentProcessId() };
        crate::log!("Desert Looter {} loaded, pid {}", crate::VERSION, pid);
        let ini_path = log::exe_dir().join(crate::INI_NAME);
        // Before the read, not after: on a first run `load_config` then reads
        // the file that was just written, which also stamps `ini_seen` from a
        // file that exists so the once-a-second reload poll does not fire
        // immediately on an ini it has never seen.
        write_default_ini();
        let (mut ini_seen, mut cfg) = load_config(&ini_path);
        crate::log!("[ini] {}", ini_summary(&cfg));
        write_schema();
        events::set_log_received(cfg.enabled && cfg.log_received);
        if !cfg.enabled {
            // This no longer returns: the two prologues can only be patched
            // here, while the game is still loading and no thread is executing
            // them, so the hooks go in regardless and `Enabled` gates every
            // action in the loop below instead. That is what lets the ini turn
            // the plugin back on without a restart. Nothing is sent, nothing
            // is scanned and nothing is logged until it says 1.
            crate::log!("Enabled=0: the hooks are installed but the plugin stays idle until the ini says otherwise");
        }

        let Some(module) = MainModule::locate() else {
            crate::log!("could not locate the main module; giving up");
            return 0;
        };
        crate::log!("[module] base=0x{:X} size=0x{:X}", module.base, module.size);
        let t0 = std::time::Instant::now();
        let anchors = game::resolve(&module);
        crate::log!(
            "[sig] {} found, {} missing, {} actor-manager vtable(s), {:.0} ms",
            anchors.hits.len(), anchors.missing.len(), anchors.actor_manager_vtables.len(),
            t0.elapsed().as_secs_f64() * 1000.0
        );
        let hooked = install_sweep_hook(&module, &anchors);
        let recorder = install_enqueue_hook(&module, &anchors);
        let api_ok = resolve_event_api(&module, &anchors);
        if let Some(m2) = MainModule::locate() {
            events::set_module(m2);
        }
        if cfg.enabled {
            // The "hooks are in" acknowledgement. Not sounded when the ini
            // says the plugin is off; nothing will happen until it says 1.
            beep();
        }

        let mut gatherer = Gatherer::new(&cfg);
        load_yields();
        // Give the game its loading phase before we start walking heap pointers.
        let mut world: Option<game::World> = None;
        let boot_grace = std::time::Duration::from_secs(20);
        let mut census_done = false;
        let mut next_world_try = std::time::Instant::now() + boot_grace;
        let mut k_toggle = Hotkey::new(cfg.key_toggle);
        let mut k_scan = Hotkey::new(cfg.key_scan);
        let mut k_gather = Hotkey::new(cfg.key_gather);
        let mut k_record = Hotkey::new(cfg.key_record);
        let mut descriptor_ok = false;
        let mut hook_reported = false;
        let ini_poll = std::time::Duration::from_secs(RELOAD_POLL_SECS);
        let mut next_ini_poll = std::time::Instant::now() + ini_poll;
        loop {
            // The ini is watched here and nowhere else: this is the plugin
            // thread, never a hook and never the game thread. A stat a second,
            // and a read only when the file has moved.
            if std::time::Instant::now() >= next_ini_poll {
                next_ini_poll = std::time::Instant::now() + ini_poll;
                if ini_mtime(&ini_path).is_some_and(|t| Some(t) != ini_seen) {
                    // A failed read leaves `ini_seen` alone, so the next poll
                    // tries the same file again.
                    if let Some((mtime, new_cfg, warnings)) = read_config(&ini_path) {
                        ini_seen = Some(mtime);
                        let old = std::mem::replace(&mut cfg, new_cfg);
                        for w in warnings {
                            crate::log!("[ini] {w}");
                        }
                        crate::log!("[ini] reloaded: {}", ini_summary(&cfg));
                        events::set_log_received(cfg.enabled && cfg.log_received);
                        gatherer.apply(&cfg);
                        // Rebuilt only when the binding actually moved: a new
                        // poller starts with "not held", which would fire once
                        // for a key that happens to be down right now.
                        if old.key_toggle != cfg.key_toggle {
                            k_toggle = Hotkey::new(cfg.key_toggle);
                        }
                        if old.key_scan != cfg.key_scan {
                            k_scan = Hotkey::new(cfg.key_scan);
                        }
                        if old.key_gather != cfg.key_gather {
                            k_gather = Hotkey::new(cfg.key_gather);
                        }
                        if old.key_record != cfg.key_record {
                            k_record = Hotkey::new(cfg.key_record);
                        }
                        if old.enabled != cfg.enabled {
                            crate::log!(
                                "[ini] Enabled={}: automatic gathering and the hotkeys are {} (the hooks stay where they are)",
                                cfg.enabled as u8, if cfg.enabled { "back on" } else { "off" }
                            );
                        }
                    }
                }
            }
            // Polled even while disabled, so a press made with Enabled=0 is
            // consumed rather than fired the moment it goes back to 1.
            let (press_toggle, press_record, press_gather, press_scan) =
                (k_toggle.pressed(), k_record.pressed(), k_gather.pressed(), k_scan.pressed());
            if !cfg.enabled {
                std::thread::sleep(std::time::Duration::from_millis(30));
                continue;
            }
            if world.is_none() && std::time::Instant::now() >= next_world_try {
                world = game::find_world(&module, &anchors);
                if world.is_none() {
                    next_world_try = std::time::Instant::now() + std::time::Duration::from_secs(3);
                }
            }
            if world.is_some() && api_ok && !descriptor_ok {
                // Same cadence as the world retry: the table exists once the
                // game is past loading, which the world being found implies.
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
                let on = gatherer.toggle();
                crate::log!("[key] auto-gather {}", if on { "ON" } else { "OFF" });
                beep();
            }
            if press_record {
                beep();
                if !recorder {
                    crate::log!("[record] enqueue hook not installed");
                } else if events::toggle_recording() {
                    crate::log!("[record] ON: logging every event the game queues (cap {})", events::RECORD_CAP);
                }
            }
            if press_gather {
                beep();
                if !hooked {
                    crate::log!("[gather] no sweep hook; cannot send on the game thread");
                } else {
                    match &world {
                        Some(w) => gatherer.manual(&module, w),
                        None => crate::log!("[gather] actor manager not found yet"),
                    }
                }
            }
            if hooked {
                if let Some(w) = &world {
                    gatherer.tick(&module, w);
                }
            }
            if events::take_yield_dirty() {
                save_yields();
            }
            if press_scan {
                beep();
                match &world {
                    Some(w) => {
                        if cfg.debug && !census_done {
                            census_done = true;
                            game::census(&module, w);
                            game::dump_maps(&module, w);
                        }
                        let t = std::time::Instant::now();
                        game::survey(&module, w, cfg.scan_range, 64, cfg.debug);
                        crate::log!("[survey] done in {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
                    }
                    None => crate::log!("[survey] actor manager not found yet (retrying every 3 s)"),
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }

    #[no_mangle]
    pub extern "system" fn DllMain(hinst: HMODULE, reason: u32, _reserved: *mut c_void) -> BOOL {
        if reason == DLL_PROCESS_ATTACH {
            // SAFETY: `hinst` is the handle the loader passed for this very
            // module, so it is a live HMODULE; the call only clears this
            // module's thread-attach notifications and is the documented thing
            // to do first under the loader lock.
            unsafe {
                DisableThreadLibraryCalls(hinst);
            }
            // No file I/O inside DllMain (loader lock), and none at all in
            // helper processes such as crashpad_handler.exe.
            let host = host_exe_name();
            if !host.eq_ignore_ascii_case(crate::GAME_EXE) {
                log::disable();
                return TRUE;
            }
            // SAFETY: every pointer argument is null except the entry point,
            // which is a `'static` function in this module; `main_thread`
            // ignores its parameter, so passing null is correct. An .asi is
            // never unloaded, so the thread cannot outlive its own code, and
            // creating a thread is one of the few things permitted while the
            // loader lock is held - it does not run until DllMain returns.
            unsafe {
                // Never do real work inside DllMain itself; hand off to a thread.
                CreateThread(
                    core::ptr::null(),
                    0,
                    Some(main_thread),
                    core::ptr::null(),
                    0,
                    core::ptr::null_mut(),
                );
            }
        }
        TRUE
    }
}
