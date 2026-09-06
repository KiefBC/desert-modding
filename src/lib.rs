//! Desert Looter - gathering auto-loot for Crimson Desert, as an ASI plugin.
//!
//! Current stage: **read-only observer**. On load it resolves the game-side
//! anchors (byte signatures, RTTI) and logs them. Hotkeys toggle and request
//! a survey. Nothing writes to game memory yet.

pub mod collect;
pub mod config;
pub mod log;
pub mod pattern;
pub mod pe;
pub mod rtti;

#[cfg(windows)]
pub mod actors;
#[cfg(windows)]
pub mod game;
#[cfg(windows)]
pub mod hotkey;
#[cfg(windows)]
pub mod module;
#[cfg(windows)]
pub mod safe;
#[cfg(windows)]
pub mod tables;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INI_NAME: &str = "DesertLooter.ini";

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
    use crate::{game, log};

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
        log::init();
        crate::log!("Desert Looter {} loaded, pid {}", crate::VERSION, GetCurrentProcessId());
        let cfg = load_config();
        crate::log!(
            "[ini] Enabled={} Debug={} ScanRange={} KeyToggle=0x{:02X} KeyScan=0x{:02X}",
            cfg.enabled as u8, cfg.debug as u8, cfg.scan_range, cfg.key_toggle, cfg.key_scan
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
        MessageBeep(MB_OK);

        let mut enabled = true;
        // Give the game its loading phase before we start walking heap pointers.
        let mut world: Option<game::World> = None;
        let boot_grace = std::time::Duration::from_secs(20);
        let mut census_done = false;
        let mut next_world_try = std::time::Instant::now() + boot_grace;
        let mut k_toggle = Hotkey::new(cfg.key_toggle);
        let mut k_scan = Hotkey::new(cfg.key_scan);
        loop {
            if world.is_none() && std::time::Instant::now() >= next_world_try {
                world = game::find_world(&module, &anchors);
                if world.is_none() {
                    next_world_try = std::time::Instant::now() + std::time::Duration::from_secs(3);
                }
            }
            if k_toggle.pressed() {
                enabled = !enabled;
                crate::log!("[key] auto-loot {}", if enabled { "ON" } else { "OFF" });
                MessageBeep(MB_OK);
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
