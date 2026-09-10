//! Desert Overlay - the in-game settings menu of Desert Tooling.
//!
//! It draws a Dear ImGui window inside the game's own DirectX 12 frame (via
//! the `hudhook` crate) and uses it to edit `DesertTooling.ini` while the game
//! runs. Each subsystem re-reads its own section of that file about once a
//! second, so a checkbox ticked here takes effect in a second or so without a
//! restart.
//!
//! **The ini is the whole contract.** The overlay never calls into the other
//! subsystems and shares no memory with them; the file beside the game exe is
//! the source of truth, and a hand edit made in Notepad shows up in the menu
//! just as fast as a click in the menu shows up on disk.
//!
//! **It knows nothing about any particular mod.** There is no list of keys,
//! defaults or ranges in this crate. Each subsystem declares its settings as a
//! [`desert_core::schema::Section`] in its own `config.rs`, and `desert-tooling`
//! hands all of them to [`start`] at startup: [`dynmodel`] holds one model per
//! section, [`store`] watches and writes the one file, and [`ui`] draws
//! whatever the sections describe, one collapsible section each. A new
//! subsystem appears in the menu by passing another `Section` in - this crate
//! needs no change.
//!
//! Because all three sections sit in **one** file, in which `Enabled` exists
//! under every header, every read is scoped through
//! [`desert_core::ini::lines_in_section`] and every write through
//! [`rewrite::rewrite`], which edits only the lines under the section's own
//! `[Header]`.
//!
//! It touches no game memory at all - no signatures, no RVAs, nothing to
//! rebase after a game update. What can break on an update is the graphics
//! API: hudhook hooks the DXGI swapchain, and if the game ever stops
//! presenting through DX12 the menu stops drawing. Nothing else is affected;
//! a failed hook is logged and the subsystem goes quiet.
//!
//! The workspace rules hold here as everywhere: no panics
//! (`panic = "abort"`), and nothing here runs until `desert-tooling`'s own
//! thread calls [`start`], long after `DllMain` has returned.

// The shared plumbing, re-exported under the names this workspace uses so
// `crate::log!` and `crate::ini` resolve inside every module here. This crate
// has no use for the memory-reading half of desert-core: it never reads a
// foreign address.
pub use desert_core::{ini, log};
#[cfg(windows)]
pub use desert_core::hotkey;

/// What `crate::log!` tags this subsystem's lines with in the one shared log.
/// The macro takes it from the calling crate's root, which is why it has to be
/// declared here.
pub const LOG_TAG: &str = "overlay";

// The pure half: the overlay's own settings, the schema-driven model, the
// comment-preserving rewrite, the file store and the embedded logo bytes. All
// of it links and unit-tests natively on Linux, which is where every rule
// about what the overlay writes is actually verified.
pub mod config;
pub mod dynmodel;
pub mod logo;
pub mod rewrite;
pub mod store;
pub mod theme;
pub mod themes;

// The Windows half: the hudhook render loop, the tracing bridge and the two
// user32 hooks that keep the game from pinning the mouse pointer while the
// menu is open. hudhook and imgui do not build for x86_64-unknown-linux-gnu
// (imgui-sys compiles the Dear ImGui C++ sources for the target), so all three
// stay behind the cfg.
#[cfg(windows)]
pub mod cursor;
#[cfg(windows)]
pub mod trace;
#[cfg(windows)]
pub mod ui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Install the menu. Called once, from `desert-tooling`'s own thread.
///
/// `hmodule` is the `HMODULE` `DllMain` was handed, passed through as a plain
/// pointer because hudhook wants the `HINSTANCE` of its own `windows` crate and
/// the two handle types are unrelated. `sections` is every subsystem's
/// settings, this crate's [`config::schema`] included, in whatever order the
/// caller collected them - the menu sorts them by `(order, title)` itself.
///
/// Everything here is what the old `main_thread` did after `log::init`: the
/// ini, the `Enabled=0` early return, the tracing bridge, the render loop and
/// the cursor hooks. It **returns** once the hooks are applied, unlike the
/// other two subsystems' `start`: from then on the menu runs on the game's own
/// render thread, inside hudhook's hook of `Present`.
///
/// There is no boot grace and no retry loop. hudhook's DX12 hook is a MinHook
/// patch of `IDXGISwapChain::Present`/`ResizeBuffers` and the command-queue
/// `ExecuteCommandLists`, all of which it reaches through a throwaway device it
/// creates itself, so it does not need the game's swapchain to exist yet.
#[cfg(windows)]
pub fn start(hmodule: *mut core::ffi::c_void, sections: Vec<desert_core::schema::Section>) {
    use hudhook::hooks::dx12::ImguiDx12Hooks;
    use hudhook::Hudhook;

    use crate::ui::Overlay;

    let ini_path = log::exe_dir().join(config::INI_NAME);
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
        return;
    }

    // Before anything hudhook does, so its own diagnosis of a failed hook
    // lands in our log rather than nowhere.
    if !trace::install(cfg.debug) {
        crate::log!("[hudhook] a tracing subscriber was already installed; its log is not ours");
    }

    let debug = cfg.debug;
    let overlay = Overlay::new(cfg, sections);

    let hmodule = hudhook::windows::Win32::Foundation::HINSTANCE(hmodule);

    let t0 = std::time::Instant::now();
    match Hudhook::builder().with::<ImguiDx12Hooks>(overlay).with_hmodule(hmodule).build().apply() {
        Ok(()) => {
            crate::log!(
                "[hudhook] DX12 hooks applied in {:.0} ms; press the menu key in game",
                t0.elapsed().as_secs_f64() * 1000.0
            );
            // Only now: `Hudhook::builder()` is what initialises MinHook and
            // `apply()` is what flushes its enable queue, so the cursor hooks
            // have to come after it. `install` logs its own outcome; a failure
            // only costs the pointer its freedom while the menu is open.
            crate::cursor::install();
        }
        // Deliberately NOT `hudhook::eject()`: unloading the DLL out from
        // under the loader is a far bigger risk than a menu that sits there
        // doing nothing, and the game must survive this.
        Err(e) => crate::log!(
            "[hudhook] applying the DX12 hooks FAILED: {e:?}; no menu, the game is untouched"
        ),
    }
    if debug {
        crate::log!("[hudhook] setup thread finished");
    }
}

/// Read the `[Overlay]` section of the shared ini. A missing file is not an
/// error: `desert-tooling` seeds it at startup, and the defaults cover the case
/// where that failed.
#[cfg(windows)]
fn load_config(path: &std::path::Path) -> config::Config {
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
            config::Config::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_names_the_one_shared_ini_and_its_own_section_of_it() {
        // This crate no longer names a file of its own: one ini, one log, and
        // the only thing it decides is which header inside that ini is its.
        let section = config::schema();
        assert_eq!(section.ini, config::INI_NAME);
        assert_eq!(section.ini, "DesertTooling.ini");
        assert_eq!(section.ini_section, config::INI_SECTION);
        assert_eq!(LOG_TAG, "overlay");
    }
}
