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
pub use desert_core::{collect, ini, log, pattern, pe, rtti, trampoline};
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
    use crate::{events, game, hook, log};

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
                    crate::log!("[event] steal check NOT resolved: ground items will all be treated as owned");
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
        unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
    }

    fn host_exe_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

    fn load_config() -> Config {
        let path = log::exe_dir().join(crate::INI_NAME);
        match std::fs::read_to_string(&path) {
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

    /// Runs on its own thread for the life of the process.
    unsafe extern "system" fn main_thread(_param: *mut c_void) -> u32 {
        log::init(crate::LOG_NAME);
        crate::log!("Desert Looter {} loaded, pid {}", crate::VERSION, GetCurrentProcessId());
        let cfg = load_config();
        crate::log!(
            "[ini] Enabled={} Debug={} ScanRange={} GatherRange={} AutoGather={} GatherUnarmed={} GatherItems={} GatherGear={} BagTab={} StackLimit={} GatherInterval={} NodeCooldown={} KeyToggle=0x{:02X} KeyScan=0x{:02X} KeyGather=0x{:02X} KeyRecord=0x{:02X}",
            cfg.enabled as u8, cfg.debug as u8, cfg.scan_range, cfg.gather_range, cfg.auto_gather as u8,
            cfg.gather_unarmed as u8, cfg.gather_items as u8, cfg.gather_gear as u8,
            cfg.bag_tab.map(|t| t.to_string()).unwrap_or_else(|| "auto".into()), cfg.stack_limit, cfg.gather_interval_ms, cfg.node_cooldown_ms, cfg.key_toggle, cfg.key_scan, cfg.key_gather, cfg.key_record
        );
        if !cfg.enabled {
            crate::log!("Enabled=0, staying idle");
            return 0;
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
        MessageBeep(MB_OK);

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
        loop {
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
            if k_toggle.pressed() {
                let on = gatherer.toggle();
                crate::log!("[key] auto-gather {}", if on { "ON" } else { "OFF" });
                MessageBeep(MB_OK);
            }
            if k_record.pressed() {
                MessageBeep(MB_OK);
                if !recorder {
                    crate::log!("[record] enqueue hook not installed");
                } else if events::toggle_recording() {
                    crate::log!("[record] ON: logging every event the game queues (cap {})", events::RECORD_CAP);
                }
            }
            if k_gather.pressed() {
                MessageBeep(MB_OK);
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
            if k_scan.pressed() {
                MessageBeep(MB_OK);
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
