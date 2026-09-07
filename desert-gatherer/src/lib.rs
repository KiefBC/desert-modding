//! Desert Gatherer - gathering yield multiplier for Crimson Desert, as an ASI
//! plugin.
//!
//! It hooks the game's gimmickinfo record loader and multiplies the minimum
//! and maximum yield scalars in the raw table bytes of each gather record, in
//! the instant between the loader being entered and the deserializer reading
//! the record. Four independent families - Foraging, Logging, Mining and Ore
//! Nodes - each get their own multiplier from `DesertGatherer.ini`. This
//! replaces the DMM JSON pack in `dmm-pack/`, which patched the same scalars
//! on disk; **the pack and DMM's built-in gathering multiplier must be
//! unmounted, or yields multiply twice.**
//!
//! Nothing is hard-coded to an address: the loader is found by content
//! (`desert_core::gimmick::resolve_record_loader`) and the output blocks by
//! their 68-byte signature. Every step that fails logs and leaves vanilla
//! yields behind.
//!
//! The rules desert-core enforces hold here too: no panics
//! (`panic = "abort"`), every foreign read through `desert_core::safe`, no
//! file I/O from `DllMain`, and nothing runs unless the host process is the
//! game.

// The shared plumbing lives in desert-core, re-exported under the names this
// workspace has always used, so `crate::log!`, `crate::safe::read`,
// `crate::module::MainModule` keep resolving inside every module here. `log`
// names both a module and the exported macro; one `use` brings in both.
// `hook` is deliberately NOT re-exported: this crate has its own `hook`
// module, which re-exports `desert_core::hook::{install, hex}` so that
// `crate::hook::install` and `crate::hook::on_record_load` both resolve.
pub use desert_core::{collect, gimmick, ini, log, pattern, pe, rtti, trampoline};
#[cfg(windows)]
pub use desert_core::{module, safe};

// Same split as desert-core: anything that talks to the game or to Win32 is
// #[cfg(windows)], the rest links natively on Linux so `cargo test
// --target x86_64-unknown-linux-gnu` runs the unit tests here.
pub mod config;

#[cfg(windows)]
pub mod hook;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INI_NAME: &str = "DesertGatherer.ini";
/// Written beside the game exe. Named here, not in desert-core, so each
/// plugin gets its own file.
pub const LOG_NAME: &str = "DesertGatherer.log";

/// Only this process is the game. The ASI loader (winmm.dll) also gets pulled
/// into helper processes started from bin64 (crashpad_handler.exe), and each
/// of those would otherwise run its own copy of us.
pub const GAME_EXE: &str = "CrimsonDesert.exe";

/// How many gather records `desert_core::collect` knows about (275 on build
/// 25116796). The ceiling on the plugin's own per-record log lines.
pub fn known_records() -> usize {
    collect::COLLECT_RECORDS.len()
}

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows_sys::Win32::System::Threading::{CreateThread, GetCurrentProcessId};

    use crate::config::{self, Config};
    use crate::gimmick;
    use crate::module::MainModule;
    use crate::{hook, log, safe};

    /// Seconds between two counter summaries in the log, and only when a
    /// counter moved since the last one. Record loading is a burst at level
    /// load, so this settles into silence.
    const SUMMARY_SECS: u64 = 60;

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

    /// Runs on its own thread for the life of the process.
    ///
    /// There is no boot grace here, unlike Desert Looter: the gimmickinfo
    /// table is read during loading, so the hook has to be in place before the
    /// game gets that far. That is also why it is safe to patch the prologue -
    /// no thread is executing it yet.
    unsafe extern "system" fn main_thread(_param: *mut c_void) -> u32 {
        log::init(crate::LOG_NAME);
        crate::log!(
            "Desert Gatherer {} loaded, pid {}, {} gather records known",
            crate::VERSION,
            GetCurrentProcessId(),
            crate::known_records()
        );
        let cfg = load_config();
        crate::log!(
            "[ini] Enabled={} DryRun={} Debug={} Foraging={} Logging={} Mining={} Ore={}",
            cfg.enabled as u8,
            cfg.dry_run as u8,
            cfg.debug as u8,
            cfg.foraging,
            cfg.logging,
            cfg.mining,
            cfg.ore
        );
        if !cfg.enabled {
            crate::log!("Enabled=0, staying idle");
            return 0;
        }
        if cfg.all_vanilla() {
            crate::log!("[ini] every family is at 1x: the hook will read records and write nothing");
        }
        if cfg.dry_run {
            crate::log!("[ini] DryRun=1: the log shows what would change, nothing is written");
        }
        // The hook reads this; set before the hook can possibly fire.
        hook::set_config(cfg);

        let Some(module) = MainModule::locate() else {
            crate::log!("could not locate the main module; giving up");
            return 0;
        };
        crate::log!("[module] base=0x{:X} size=0x{:X}", module.base, module.size);

        let t0 = std::time::Instant::now();
        let hooked = install_loader_hook(&module);
        crate::log!("[hook] resolve+install took {:.0} ms", t0.elapsed().as_secs_f64() * 1000.0);
        if !hooked {
            crate::log!("no hook installed; the plugin is idle and the game is untouched");
            return 0;
        }

        // Summaries only. All the per-record work happens on the game threads.
        let mut last = (0u64, 0u64, 0u64);
        loop {
            std::thread::sleep(std::time::Duration::from_secs(SUMMARY_SECS));
            let now = hook::counters();
            if now != last {
                crate::log!(
                    "[stat] records seen {}, gather records patched {}, scalars written {}",
                    now.0,
                    now.1,
                    now.2
                );
                last = now;
            }
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
            if !host_exe_name().eq_ignore_ascii_case(crate::GAME_EXE) {
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

#[cfg(test)]
mod tests {
    #[test]
    fn core_is_linked() {
        assert!(super::known_records() > 200, "the gather record table came from desert-core");
    }
}
