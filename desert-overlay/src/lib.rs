//! Desert Overlay - an in-game settings menu for the Crimson Desert mods, as
//! an ASI plugin.
//!
//! It draws a Dear ImGui window inside the game's own DirectX 12 frame (via
//! the `hudhook` crate) and uses it to edit `DesertLooter.ini` and
//! `DesertGatherer.ini` while the game runs. Desert Looter and Desert Gatherer
//! re-read their ini about once a second, so a checkbox ticked here takes
//! effect in a second or so without a restart.
//!
//! **The ini files are the whole contract.** The overlay never calls into the
//! other two plugins, shares no memory with them and does not care whether
//! they are installed at all; the file beside the game exe is the source of
//! truth, and a hand edit made in Notepad shows up in the menu just as fast as
//! a click in the menu shows up on disk. That is also why the key lists and
//! defaults are duplicated in [`model`] rather than imported: each plugin is a
//! `cdylib` exporting its own `DllMain`, so they cannot be linked together.
//!
//! It touches no game memory at all - no signatures, no RVAs, nothing to
//! rebase after a game update. What can break on an update is the graphics
//! API: hudhook hooks the DXGI swapchain, and if the game ever stops
//! presenting through DX12 the menu stops drawing. Nothing else is affected;
//! a failed hook is logged and the plugin goes quiet.
//!
//! The workspace rules hold here as everywhere: no panics
//! (`panic = "abort"`), no file I/O from `DllMain`, and nothing runs unless
//! the host process is the game.

// The shared plumbing, re-exported under the names this workspace uses so
// `crate::log!` and `crate::ini` resolve inside every module here. This crate
// has no use for the memory-reading half of desert-core: it never reads a
// foreign address.
pub use desert_core::{ini, log};
#[cfg(windows)]
pub use desert_core::hotkey;

// The pure half: the ini model, the comment-preserving rewrite, the presets,
// the file store and the embedded logo bytes. All of it links and unit-tests
// natively on Linux, which is where every rule about what the overlay writes
// is actually verified.
pub mod config;
pub mod logo;
pub mod model;
pub mod presets;
pub mod rewrite;
pub mod store;
pub mod theme;
pub mod themes;

// The Windows half: the hudhook render loop and the tracing bridge. hudhook
// and imgui do not build for x86_64-unknown-linux-gnu (imgui-sys compiles the
// Dear ImGui C++ sources for the target), so both stay behind the cfg.
#[cfg(windows)]
pub mod trace;
#[cfg(windows)]
pub mod ui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INI_NAME: &str = "DesertOverlay.ini";
/// Written beside the game exe. Named here, not in desert-core, so each plugin
/// gets its own file.
pub const LOG_NAME: &str = "DesertOverlay.log";

/// Only this process is the game. The ASI loader (winmm.dll) also gets pulled
/// into helper processes started from bin64 (crashpad_handler.exe), and each
/// of those would otherwise run its own copy of us.
pub const GAME_EXE: &str = "CrimsonDesert.exe";

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
    use windows_sys::Win32::System::Threading::{CreateThread, GetCurrentProcessId};

    use hudhook::hooks::dx12::ImguiDx12Hooks;
    use hudhook::Hudhook;

    use crate::config::{self, Config};
    use crate::log;
    use crate::ui::Overlay;
    use crate::{trace, INI_NAME};

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

    /// Runs on its own thread for the life of the process.
    ///
    /// There is no boot grace and no retry loop. hudhook's DX12 hook is a
    /// MinHook patch of `IDXGISwapChain::Present`/`ResizeBuffers` and the
    /// command-queue `ExecuteCommandLists`, all of which it reaches through a
    /// throwaway device it creates itself, so it does not need the game's
    /// swapchain to exist yet. Once `apply()` has returned, this thread has
    /// nothing left to do: every later frame runs `Overlay` on the game's own
    /// render thread.
    unsafe extern "system" fn main_thread(param: *mut c_void) -> u32 {
        log::init(crate::LOG_NAME);
        // SAFETY: `GetCurrentProcessId` takes no arguments and only reads this
        // process's own PEB; it is sound to call from any thread.
        let pid = unsafe { GetCurrentProcessId() };
        crate::log!("Desert Overlay {} loaded, pid {}", crate::VERSION, pid);

        let ini_path = log::exe_dir().join(INI_NAME);
        let cfg = load_config(&ini_path);
        crate::log!(
            "[ini] Enabled={} Debug={} KeyMenu=0x{:02X} ShowOnStart={} Scale={} FontSize={} \
             Font={} HdrBrightness={} ColorSpace={} Theme={}",
            cfg.enabled as u8,
            cfg.debug as u8,
            cfg.key_menu,
            cfg.show_on_start as u8,
            cfg.scale,
            cfg.font_size,
            cfg.font,
            cfg.hdr_brightness,
            cfg.color_space.as_str(),
            cfg.theme.name
        );
        if !cfg.enabled {
            crate::log!("Enabled=0: no graphics hook is installed, the game renders untouched");
            return 0;
        }

        // Before anything hudhook does, so its own diagnosis of a failed hook
        // lands in our log rather than nowhere.
        if !trace::install(cfg.debug) {
            crate::log!("[hudhook] a tracing subscriber was already installed; its log is not ours");
        }

        let debug = cfg.debug;
        let overlay = Overlay::new(cfg);
        crate::log!("[ini] read DesertLooter.ini and DesertGatherer.ini");

        // hudhook wants the HINSTANCE from its own `windows` crate; `param` is
        // the HMODULE DllMain was handed, passed through as a plain pointer
        // because the two crates' handle types are unrelated.
        let hmodule = hudhook::windows::Win32::Foundation::HINSTANCE(param);

        let t0 = std::time::Instant::now();
        match Hudhook::builder().with::<ImguiDx12Hooks>(overlay).with_hmodule(hmodule).build().apply()
        {
            Ok(()) => crate::log!(
                "[hudhook] DX12 hooks applied in {:.0} ms; press the menu key in game",
                t0.elapsed().as_secs_f64() * 1000.0
            ),
            // Deliberately NOT `hudhook::eject()`: unloading our own DLL out
            // from under the loader is a far bigger risk than a plugin that
            // sits there doing nothing, and the game must survive this.
            Err(e) => crate::log!(
                "[hudhook] applying the DX12 hooks FAILED: {e:?}; no menu, the game is untouched"
            ),
        }
        if debug {
            crate::log!("[hudhook] setup thread finished");
        }
        0
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
            // SAFETY: the entry point is a `'static` function in this module
            // and the parameter it receives is this module's own HMODULE,
            // which `main_thread` only passes on to hudhook as an HINSTANCE.
            // An .asi is never unloaded, so the thread cannot outlive its own
            // code, and creating a thread is one of the few things permitted
            // while the loader lock is held - it does not run until DllMain
            // returns.
            unsafe {
                // Never do real work inside DllMain itself; hand off to a thread.
                CreateThread(
                    core::ptr::null(),
                    0,
                    Some(main_thread),
                    hinst.cast(),
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
    use super::*;
    use model::IniModel as _;

    #[test]
    fn the_three_file_names_agree_with_the_shipped_layout() {
        assert_eq!(INI_NAME, "DesertOverlay.ini");
        assert_eq!(LOG_NAME, "DesertOverlay.log");
        assert_eq!(model::LooterModel::FILE_NAME, "DesertLooter.ini");
        assert_eq!(model::GathererModel::FILE_NAME, "DesertGatherer.ini");
    }
}
