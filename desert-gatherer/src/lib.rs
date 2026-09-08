//! Desert Gatherer - gathering yield multiplier for Crimson Desert, as an ASI
//! plugin.
//!
//! It hooks the game's gimmickinfo record loader and multiplies the minimum
//! and maximum yield scalars in the raw table bytes of each gather record, in
//! the instant between the loader being entered and the deserializer reading
//! the record. Four independent families - Foraging, Logging, Mining and Ore
//! Nodes - each get their own multiplier from `DesertGatherer.ini`. This
//! replaces the DMM JSON pack in `desert-gatherer-dmm/`, which patched the
//! same scalars on disk; **the pack and DMM's built-in gathering multiplier
//! must be unmounted, or yields multiply twice.**
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
pub use desert_core::{collect, gimmick, ini, log, pattern, pe, rtti, schema, trampoline};
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
    use crate::{hook, log, safe, schema};

    /// Seconds between two counter summaries in the log, and only when a
    /// counter moved since the last one. Record loading is a burst at level
    /// load, so this settles into silence.
    const SUMMARY_SECS: u64 = 60;

    /// How often the main thread stats `DesertGatherer.ini` for a modified
    /// time change. The overlay plugin's edit is expected to be picked up
    /// within about a second, and this is the whole budget for that: the
    /// stat is cheap and the hook never blocks on it either way.
    const RELOAD_POLL_SECS: u64 = 1;

    fn host_exe_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    }

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
            "Enabled={} DryRun={} Debug={} Foraging={} Logging={} Mining={} Ore={}",
            cfg.enabled as u8,
            cfg.dry_run as u8,
            cfg.debug as u8,
            cfg.foraging,
            cfg.logging,
            cfg.mining,
            cfg.ore
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

    /// Write `DesertGatherer.overlay.ini` beside the exe, so Desert Overlay
    /// can draw this plugin's settings without knowing anything about it.
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
            "{name} - written by Desert Gatherer {} every time the game starts.\n\
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

    /// Runs on its own thread for the life of the process.
    ///
    /// There is no boot grace here, unlike Desert Looter: the gimmickinfo
    /// table is read during loading, so the hook has to be in place before the
    /// game gets that far. That is also why it is safe to patch the prologue -
    /// no thread is executing it yet.
    unsafe extern "system" fn main_thread(_param: *mut c_void) -> u32 {
        log::init(crate::LOG_NAME);
        // SAFETY: `GetCurrentProcessId` takes no arguments and only reads this
        // process's own PEB; it is sound to call from any thread.
        let pid = unsafe { GetCurrentProcessId() };
        crate::log!(
            "Desert Gatherer {} loaded, pid {}, {} gather records known",
            crate::VERSION,
            pid,
            crate::known_records()
        );
        let ini_path = log::exe_dir().join(crate::INI_NAME);
        let cfg = load_config(&ini_path);
        crate::log!("[ini] {}", ini_summary(&cfg));
        write_schema();
        if !cfg.enabled {
            // The hook is installed anyway and `LiveConfig::enabled` gates
            // it per call. Installing it later, when the ini flips `Enabled`
            // on, is not an option: patching the loader prologue is only
            // safe now, while the game is still loading and no thread can be
            // executing those 12 bytes. Runtime toggling therefore needs the
            // hook present from the start.
            crate::log!("Enabled=0: hook will be installed but stays a pass-through until the ini says otherwise");
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

        // From here the thread does two things on a ~1s tick, forever: watch
        // the ini for a modified time change (the overlay plugin's write) and
        // re-publish it live, and print a counters summary every
        // `SUMMARY_SECS`. All the per-record work still happens on the game
        // threads; this thread never touches game memory.
        let mut last_ini_mtime = ini_mtime(&ini_path);
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
                        for w in &warnings {
                            crate::log!("[ini] {w}");
                        }
                        crate::log!("[ini] reloaded: {}", ini_summary(&cfg));
                        config::LIVE.publish(&cfg);
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
            if !host_exe_name().eq_ignore_ascii_case(crate::GAME_EXE) {
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

#[cfg(test)]
mod tests {
    #[test]
    fn core_is_linked() {
        assert!(super::known_records() > 200, "the gather record table came from desert-core");
    }
}
