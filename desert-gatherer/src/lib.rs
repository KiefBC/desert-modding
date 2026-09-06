//! Desert Gatherer - gathering yield multiplier for Crimson Desert, as an ASI
//! plugin.
//!
//! **Stub.** This crate exists to hold the workspace layout open and to prove
//! the desert-core dependency links into a second cdylib; it installs no hooks
//! and touches no game memory yet. The yield hook lands next.
//!
//! The rules it will be built under are the ones desert-core already enforces:
//! no panics (`panic = "abort"`), every foreign read through
//! `desert_core::safe`, no file I/O from `DllMain`, and nothing runs unless
//! the host process is the game.

/// Only this process is the game. The ASI loader (winmm.dll) also gets pulled
/// into helper processes started from bin64 (crashpad_handler.exe), and each
/// of those would otherwise run its own copy of us.
pub const GAME_EXE: &str = "CrimsonDesert.exe";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const INI_NAME: &str = "DesertGatherer.ini";
pub const LOG_NAME: &str = "DesertGatherer.log";

/// Exercises the desert-core dependency: the gather records this mod will
/// multiply yields for are the same table Desert Looter classifies nodes with.
pub fn known_records() -> usize {
    desert_core::collect::COLLECT_RECORDS.len()
}

#[cfg(windows)]
mod entry {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::{BOOL, HMODULE, TRUE};
    use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
    use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

    use desert_core::log;

    fn host_exe_name() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
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
            }
            // Nothing else yet: the stub deliberately starts no thread.
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
