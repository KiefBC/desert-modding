//! The menu: hudhook's render loop, the key that shows it, and the input
//! blocking that keeps the game from reacting while it is on screen.
//!
//! Everything the window does is a read or a write of a mod's ini file
//! through [`crate::store`]. There is no other channel to the plugins, and
//! this module never touches game memory, calls a game function, or holds a
//! lock a render thread could block on.
//!
//! Nothing below names a mod, a key or a range. The sections, their widgets
//! and their presets all arrive in [`crate::start`], one
//! [`desert_core::schema::Section`] per subsystem: this module is a renderer
//! for [`desert_core::schema::Field`] and nothing more.
//!
//! Three things about running inside somebody else's frame:
//!
//! * hudhook calls [`ImguiRenderLoop::message_filter`] *before*
//!   [`ImguiRenderLoop::before_render`] each frame, so a toggle noticed in
//!   `before_render` starts blocking input on the next frame. One frame of
//!   game input on the way in and out is not worth a second mechanism.
//! * The game hides the hardware cursor and clips it to the window, so imgui
//!   has to draw its own (`io.mouse_draw_cursor`, only while the hardware
//!   cursor is really hidden) and the clip rectangle has to be released, or
//!   the pointer cannot reach the window's edges. Releasing it here is only
//!   the opening move: the game re-clips and re-centres the cursor every
//!   frame, so what actually keeps the pointer free is [`crate::cursor`],
//!   which hooks `ClipCursor` and `SetCursorPos` and neuters them for as long
//!   as [`crate::cursor::set_menu_open`] says the menu is up.
//! * `is_any_item_active` is what tells the store that a slider has been let
//!   go, which is what turns a drag into four writes a second instead of one
//!   per frame.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::Instant;

use hudhook::{ImguiRenderLoop, MessageFilter};
use imgui::{
    Condition, Context, FontAtlas, FontConfig, FontId, FontSource, Io, Style, StyleColor,
    TextureId, TreeNodeFlags, Ui,
};

use desert_core::hotkey::Hotkey;
use desert_core::ini;
use desert_core::schema::{Kind, Section};
use desert_core::telemetry;

use crate::config::{ColorSpace, Config, FontChoice};
use crate::dynmodel::{self, DynModel, SectionEntry};
use crate::logo;
use crate::store::Flushed;
use crate::theme::{Role, Theme};
use crate::themes;

/// How often each section's plugin DLL is looked up in the process.
const PLUGIN_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// What the window says when it was handed no sections at all, which can only
/// happen to a build that forgot to pass them.
const NO_SECTIONS: &str = "No settings to show: this build of Desert Tooling handed the menu no \
                           sections at all.";

/// The label above a section's preset buttons when its schema does not give
/// one of its own.
const PRESETS_LABEL: &str = "Presets:";

/// Widest bound handed to a float slider. Dear ImGui aborts on a slider range
/// wider than half `f32`'s, and the bounds come out of a text file some other
/// mod wrote, so they are clamped to something no settings menu can need. A
/// value is still checked against the schema's own range before it is written.
const SLIDER_LIMIT: f32 = 1.0e9;

/// The largest font file the overlay will pull into the game's address space.
/// Every Windows system font is a couple of megabytes at most (`cambria.ttc`,
/// the biggest of the ones the ini suggests, is about 3 MB); the cap only
/// exists so a `Font` pointing at something that is not a font cannot cost the
/// game hundreds of megabytes before stb_truetype rejects it.
const FONT_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// What the log calls the font when there is no file behind it.
const BUILT_IN_FONT: &str = "built-in ProggyClean";

/// The widest the position readout ever gets, and the string the row is laid
/// out against. Never displayed.
const READOUT_WIDEST: &str = "X -000000.0   Y -000000.0   Z -000000.0";

/// What the readout says when no position has been published lately: the world
/// is not up yet, the player is on a load screen, or the subsystem that
/// publishes it is not running in this build.
const NO_POSITION: &str = "position unavailable";

/// The readout's tooltip. It says which axis is which because the answer is
/// not the intuitive one.
const POSITION_HELP: &str = "The player character's position in the game world. Y is altitude: X \
                             and Z are the two that place you on the map.";

/// The narrowest the theme combo is allowed to get before it stops making room
/// for the readout beside it.
const COMBO_MIN_WIDTH: f32 = 80.0;

/// The name on the window and in the header, in one place so the two cannot
/// drift apart. It is also the imgui window id, so changing it resets a
/// window position remembered within a session.
const TITLE: &str = "Desert Tooling";

/// The header title's face, as a multiple of the menu font's size. Big enough
/// to read as a heading beside the logo, small enough that it still fits
/// inside the logo's height without the row growing.
const TITLE_FONT_RATIO: f32 = 1.6;

/// The logo's drawn size, square, as a multiple of the menu font's size. The
/// embedded picture is 128 px and the DX12 sampler is linear, so at 40-70
/// physical pixels this is a clean shrink of a sharp image.
const LOGO_RATIO: f32 = 2.2;

pub struct Overlay {
    cfg: Config,
    /// One section per subsystem, in display order, fixed for the life of the
    /// process. Each owns a store on the one shared ini, scoped to its own
    /// `[Header]`.
    sections: Vec<SectionEntry>,
    /// When each section's `Module` was last looked up in the process. A
    /// section that names one and whose module is absent is drawn greyed out
    /// with a "not installed" note: its settings would still be written, but
    /// nothing would read them. Every subsystem ships in one `.asi` now, so
    /// none of them names a module and the poll is a no-op - the mechanism is
    /// kept for a section that comes from somewhere else.
    last_plugin_poll: Option<Instant>,
    /// UI scale applied once in [`ImguiRenderLoop::initialize`]: fonts, style
    /// paddings and the window's own geometry. From `Scale` in the ini, or the
    /// Windows display scaling when that is 0.
    scale: f32,
    /// The menu font's TTF/TTC bytes, read once in [`Overlay::new`]. `None`
    /// means imgui's built-in font: either `Font` was empty or the file could
    /// not be read, which was logged at the time.
    ///
    /// imgui copies these into the atlas, so it is not *ownership* that keeps
    /// them here - it is that [`ImguiRenderLoop::initialize`] runs again on a
    /// fresh context after a swapchain reset and has to add the font a second
    /// time, and re-reading a file on a render thread is exactly what this
    /// plugin must not do.
    font_data: Option<Vec<u8>>,
    /// What the startup line calls the font: the file name, plus `:N` when a
    /// `.ttc` face was picked, or [`BUILT_IN_FONT`].
    font_label: String,
    /// Face index inside a `.ttc` collection, 0 for everything else.
    font_face: u32,
    /// `FontSize` from the ini, in pixels before [`Overlay::scale`].
    font_size: f32,
    /// Where the bigger face the header title is drawn in sits in the atlas:
    /// an *index*, not a [`FontId`], because a `FontId` is a raw `*const
    /// Font` and hudhook requires the render loop to be `Send + Sync`. The
    /// index is resolved back to an id inside the frame, where the atlas is
    /// there to be asked.
    ///
    /// `None` until [`ImguiRenderLoop::initialize`] has run; the title then
    /// falls back to the menu font, which is imgui's default.
    title_font: Option<usize>,
    /// The logo, uploaded to the renderer in
    /// [`ImguiRenderLoop::initialize`]. `None` means the upload failed, which
    /// was logged at the time; the header then draws its text alone.
    ///
    /// Not cached across swapchain resets: `initialize` runs again on a fresh
    /// engine whose texture heap knows nothing about the old id, so the
    /// picture is uploaded once per pipeline.
    logo: Option<TextureId>,
    /// The colour theme in force, from `Theme` in the ini until the picker
    /// changes it.
    theme: &'static Theme,
    /// A theme chosen in the picker this frame. `render` has no access to
    /// the imgui style, so it is applied in the next `before_render`.
    pending_theme: Option<&'static Theme>,
    /// The window's size on the last frame and when it last changed, so a
    /// resize is logged once it has settled rather than on every frame of the
    /// drag. That log line is how a good default size gets chosen.
    window_size: [f32; 2],
    window_resized_at: Option<Instant>,
    visible: bool,
    menu_key: Hotkey,
}

impl Overlay {
    /// Builds one store per section, reads the ini they share and the menu
    /// font once. Called from `desert-tooling`'s own thread, never from
    /// `DllMain` and never from a render thread.
    pub fn new(cfg: Config, sections: Vec<Section>) -> Self {
        let dir = desert_core::log::exe_dir();
        let now = Instant::now();
        let sections = crate::dynmodel::entries(&dir, sections, now);
        for entry in &sections {
            crate::log!(
                "[schema] [{}] {}, {} fields, {} presets",
                entry.ini_section(),
                entry.title(),
                entry.section().fields.len(),
                entry.section().presets.len()
            );
        }
        if sections.is_empty() {
            crate::log!("[schema] no sections were handed to the menu; it is empty");
        }
        let visible = cfg.show_on_start;
        // The cursor hooks are installed later, on the plugin thread, but the
        // flag they read is set here so a menu that opens with the game is
        // already holding the pointer.
        crate::cursor::set_menu_open(visible);
        let scale = if cfg.scale > 0.0 { cfg.scale } else { Self::system_scale() };
        let theme = cfg.theme;
        let choice = cfg.font_choice();
        let (font_data, font_label) = Self::load_font(&choice);
        let font_face = if font_data.is_some() { choice.face() } else { 0 };
        let font_size = cfg.font_size;
        Overlay {
            theme,
            pending_theme: None,
            window_size: [0.0, 0.0],
            window_resized_at: None,
            menu_key: Hotkey::new(cfg.key_menu),
            sections,
            last_plugin_poll: None,
            font_data,
            font_label,
            font_face,
            font_size,
            title_font: None,
            logo: None,
            cfg,
            scale,
            visible,
        }
    }

    /// `%WINDIR%\Fonts`. `GetWindowsDirectoryW` first because it is the
    /// authoritative answer and needs no environment; `WINDIR` is the fallback
    /// for the case where it somehow fails. Never a hard-coded `C:`.
    fn windows_fonts_dir() -> Option<PathBuf> {
        use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
        let mut buf = [0u16; 260];
        // SAFETY: `buf` is a live, writable array of `buf.len()` u16s on this
        // stack frame and that same count is what the call is permitted to
        // write; GetWindowsDirectoryW only writes into it and returns the
        // number of characters written, or 0 on failure (or a required size
        // larger than the buffer, which the length check below rejects).
        let n = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        let dir = match buf.get(..n).filter(|_| n > 0 && n <= buf.len()) {
            Some(s) => PathBuf::from(String::from_utf16_lossy(s)),
            None => PathBuf::from(std::env::var_os("WINDIR")?),
        };
        Some(dir.join("Fonts"))
    }

    /// Read a font file, refusing anything over [`FONT_MAX_BYTES`]. The error
    /// is a string because it only ever goes to the log.
    fn read_font_file(path: &Path) -> Result<Vec<u8>, String> {
        let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if len > FONT_MAX_BYTES {
            return Err(format!("{len} bytes is over the {FONT_MAX_BYTES} byte limit"));
        }
        std::fs::read(path).map_err(|e| e.to_string())
    }

    /// Turn the ini's `Font` into bytes plus the name to log. A missing or
    /// unreadable file is a WARN and the built-in font, never a failure: the
    /// menu still draws.
    fn load_font(choice: &FontChoice) -> (Option<Vec<u8>>, String) {
        let (path, shown) = match choice {
            FontChoice::BuiltIn => return (None, BUILT_IN_FONT.to_string()),
            FontChoice::Path { path, face } => (PathBuf::from(path), Self::font_label(path, *face)),
            FontChoice::Name { file, face } => {
                let Some(dir) = Self::windows_fonts_dir() else {
                    crate::log!(
                        "[menu] WARN font {file}: the Windows Fonts directory could not be \
                         found; using the {BUILT_IN_FONT} font"
                    );
                    return (None, BUILT_IN_FONT.to_string());
                };
                (dir.join(file), Self::font_label(file, *face))
            }
        };
        match Self::read_font_file(&path) {
            Ok(bytes) => (Some(bytes), shown),
            Err(why) => {
                crate::log!(
                    "[menu] WARN font {}: {why}; using the {BUILT_IN_FONT} font",
                    path.display()
                );
                (None, BUILT_IN_FONT.to_string())
            }
        }
    }

    /// `segoeui.ttf`, or `cambria.ttc:1` when a collection face was picked.
    fn font_label(name: &str, face: u32) -> String {
        if face == 0 {
            name.to_string()
        } else {
            format!("{name}:{face}")
        }
    }

    /// The menu font's final size in pixels: `FontSize` from the ini through
    /// [`Overlay::scale`]. The one place that decides it, so the atlas, the
    /// log line and the header's geometry cannot disagree.
    fn font_px(&self) -> f32 {
        (self.font_size * self.scale).round().max(1.0)
    }

    /// Add the menu font to a freshly built atlas at `px` pixels, and return
    /// the id imgui gave it.
    ///
    /// Called from [`ImguiRenderLoop::initialize`], which hudhook runs before
    /// it builds and uploads the atlas, so this is the font that gets drawn.
    /// The first face added to an atlas is imgui's default, which is why the
    /// menu font goes in before the bigger title face.
    fn add_font(&self, fonts: &mut FontAtlas, px: f32) -> FontId {
        let config = FontConfig {
            size_pixels: px,
            // 2x horizontal oversampling is what makes small text on an LCD
            // look sharp rather than smeared; vertical oversampling costs
            // atlas area and buys almost nothing.
            oversample_h: 2,
            oversample_v: 1,
            ..FontConfig::default()
        };
        match &self.font_data {
            Some(bytes) => {
                let id = fonts.add_font(&[FontSource::TtfData {
                    data: bytes,
                    size_pixels: px,
                    config: Some(config),
                }]);
                if self.font_face > 0 {
                    Self::select_ttc_face(fonts, self.font_face);
                }
                id
            }
            None => fonts.add_font(&[FontSource::DefaultFontData { config: Some(config) }]),
        }
    }

    /// Point the font that was just added at face `face` of a `.ttc`
    /// collection.
    ///
    /// imgui-rs 0.12's `FontConfig` has no `font_no`, so the only way to reach
    /// Dear ImGui's `ImFontConfig::FontNo` is the raw config the atlas pushed a
    /// moment ago. That is safe to edit here because the atlas is not built
    /// until hudhook's `setup_fonts`, which runs after `initialize` returns.
    fn select_ttc_face(fonts: &mut FontAtlas, face: u32) {
        use imgui::internal::RawCast as _;
        let face = i32::try_from(face).unwrap_or(0);
        // SAFETY: `FontAtlas` declares `RawCast<sys::ImFontAtlas>`, so the two
        // have the same layout and `raw_mut` is a valid reborrow of the live
        // atlas for this scope. `ConfigData` is Dear ImGui's own vector of the
        // configs added so far, and `add_font` pushed one immediately before
        // this call; `Data` is only dereferenced after `Size` is checked to be
        // positive and the pointer non-null, at index `Size - 1`, which is
        // that config. One `i32` field is written and nothing is read back.
        unsafe {
            let raw = fonts.raw_mut();
            let n = raw.ConfigData.Size;
            if n > 0 && !raw.ConfigData.Data.is_null() {
                (*raw.ConfigData.Data.add(n as usize - 1)).FontNo = face;
            }
        }
    }

    /// The Windows display scaling as a factor (125% = 1.25), clamped to a
    /// sane range. The game is per-monitor DPI aware, so its back buffer is in
    /// physical pixels and imgui's default 13 px font is tiny on a 4K screen;
    /// following the user's own scaling setting is the least surprising size.
    fn system_scale() -> f32 {
        // SAFETY: GetDpiForSystem takes no arguments, reads only system state
        // and is callable from any thread; it returns 0 on failure.
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForSystem() };
        if dpi == 0 {
            return 1.0;
        }
        (dpi as f32 / 96.0).clamp(1.0, 4.0)
    }

    /// Paint `theme` over imgui's stock dark palette. `scale` is the same
    /// factor `scale_all_sizes` was given, so the theme's rounding numbers are
    /// in the same unscaled pixels as everything else in the file.
    fn apply_theme(style: &mut Style, theme: &Theme, scale: f32) {
        style.use_dark_colors();
        for (role, color) in (theme.colors)() {
            style[Self::style_color(role)] = color;
        }
        style.window_rounding = theme.window_rounding * scale;
        style.frame_rounding = theme.frame_rounding * scale;
        style.grab_rounding = theme.grab_rounding * scale;
        style.window_border_size = theme.window_border;
        style.frame_border_size = theme.frame_border;
    }

    /// The imgui slot for a theme role. Spelled out rather than cast so the
    /// theme module never has to depend on imgui's numbering.
    fn style_color(role: Role) -> StyleColor {
        match role {
            Role::Text => StyleColor::Text,
            Role::TextDisabled => StyleColor::TextDisabled,
            Role::WindowBg => StyleColor::WindowBg,
            Role::ChildBg => StyleColor::ChildBg,
            Role::PopupBg => StyleColor::PopupBg,
            Role::Border => StyleColor::Border,
            Role::BorderShadow => StyleColor::BorderShadow,
            Role::FrameBg => StyleColor::FrameBg,
            Role::FrameBgHovered => StyleColor::FrameBgHovered,
            Role::FrameBgActive => StyleColor::FrameBgActive,
            Role::TitleBg => StyleColor::TitleBg,
            Role::TitleBgActive => StyleColor::TitleBgActive,
            Role::TitleBgCollapsed => StyleColor::TitleBgCollapsed,
            Role::MenuBarBg => StyleColor::MenuBarBg,
            Role::ScrollbarBg => StyleColor::ScrollbarBg,
            Role::ScrollbarGrab => StyleColor::ScrollbarGrab,
            Role::ScrollbarGrabHovered => StyleColor::ScrollbarGrabHovered,
            Role::ScrollbarGrabActive => StyleColor::ScrollbarGrabActive,
            Role::CheckMark => StyleColor::CheckMark,
            Role::SliderGrab => StyleColor::SliderGrab,
            Role::SliderGrabActive => StyleColor::SliderGrabActive,
            Role::Button => StyleColor::Button,
            Role::ButtonHovered => StyleColor::ButtonHovered,
            Role::ButtonActive => StyleColor::ButtonActive,
            Role::Header => StyleColor::Header,
            Role::HeaderHovered => StyleColor::HeaderHovered,
            Role::HeaderActive => StyleColor::HeaderActive,
            Role::Separator => StyleColor::Separator,
            Role::SeparatorHovered => StyleColor::SeparatorHovered,
            Role::SeparatorActive => StyleColor::SeparatorActive,
            Role::ResizeGrip => StyleColor::ResizeGrip,
            Role::ResizeGripHovered => StyleColor::ResizeGripHovered,
            Role::ResizeGripActive => StyleColor::ResizeGripActive,
            Role::Tab => StyleColor::Tab,
            Role::TabHovered => StyleColor::TabHovered,
            Role::TabActive => StyleColor::TabActive,
            Role::TabUnfocused => StyleColor::TabUnfocused,
            Role::TabUnfocusedActive => StyleColor::TabUnfocusedActive,
            Role::PlotLines => StyleColor::PlotLines,
            Role::PlotLinesHovered => StyleColor::PlotLinesHovered,
            Role::PlotHistogram => StyleColor::PlotHistogram,
            Role::PlotHistogramHovered => StyleColor::PlotHistogramHovered,
            Role::TableHeaderBg => StyleColor::TableHeaderBg,
            Role::TableBorderStrong => StyleColor::TableBorderStrong,
            Role::TableBorderLight => StyleColor::TableBorderLight,
            Role::TableRowBg => StyleColor::TableRowBg,
            Role::TableRowBgAlt => StyleColor::TableRowBgAlt,
            Role::TextSelectedBg => StyleColor::TextSelectedBg,
            Role::DragDropTarget => StyleColor::DragDropTarget,
            Role::NavHighlight => StyleColor::NavHighlight,
            Role::NavWindowingHighlight => StyleColor::NavWindowingHighlight,
            Role::NavWindowingDimBg => StyleColor::NavWindowingDimBg,
            Role::ModalWindowDimBg => StyleColor::ModalWindowDimBg,
        }
    }

    /// The ini's `ColorSpace` as hudhook's own override. The two enums are
    /// deliberately separate: `config` compiles and unit-tests on Linux, where
    /// hudhook does not build at all.
    fn color_space_override(value: ColorSpace) -> hudhook::output::ColorSpaceOverride {
        match value {
            ColorSpace::Auto => hudhook::output::ColorSpaceOverride::Auto,
            ColorSpace::Sdr => hudhook::output::ColorSpaceOverride::Sdr,
            ColorSpace::Hdr10 => hudhook::output::ColorSpaceOverride::Hdr10,
            ColorSpace::ScRgb => hudhook::output::ColorSpaceOverride::ScRgb,
        }
    }

    /// Release the cursor's confining rectangle on the frame the menu opens.
    ///
    /// The game hides the cursor and calls `ClipCursor` to pin it inside the
    /// window (and, in camera control, to a single point). imgui gets its
    /// mouse position from the raw-input deltas hudhook feeds it, but hudhook
    /// also assigns it the absolute position out of every `WM_MOUSEMOVE`, so
    /// a pinned cursor snaps the pointer back as fast as the deltas move it.
    ///
    /// This call alone would fix nothing - the game re-clips on the next
    /// frame. It is [`crate::cursor`]'s hook on `ClipCursor` that keeps the
    /// clip off, and its hook on `SetCursorPos` that keeps the game from
    /// re-centring the pointer; this releases whatever clip was already in
    /// force when the menu came up, which the hook has no way to undo. The
    /// clip is never restored here: the game establishes its own again as soon
    /// as the menu closes and the hooks stand aside.
    fn unclip_cursor() {
        // SAFETY: ClipCursor(NULL) is the documented way to release the
        // cursor's confining rectangle. Null is the argument's own "no
        // rectangle" value, so no memory of ours is read, and the call is
        // valid from any thread of the process that owns the foreground
        // window.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::ClipCursor(core::ptr::null());
        }
    }

    /// Whether Windows is currently showing a hardware cursor for this
    /// process. In the open world the game hides it, and imgui has to draw
    /// its own; in the game's own menus it is visible, and drawing a second
    /// one on top puts two cursors on screen (seen on the first in-game run).
    fn hardware_cursor_visible() -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorInfo, CURSORINFO, CURSOR_SHOWING};
        let mut info = CURSORINFO {
            cbSize: core::mem::size_of::<CURSORINFO>() as u32,
            flags: 0,
            hCursor: core::ptr::null_mut(),
            ptScreenPos: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
        };
        // SAFETY: `info` is a properly sized, initialised CURSORINFO on our
        // own stack and lives for the whole call; GetCursorInfo only writes
        // into it. A failed call leaves `flags` at 0, which reads as hidden.
        let ok = unsafe { GetCursorInfo(&mut info) } != 0;
        ok && (info.flags & CURSOR_SHOWING) != 0
    }

    fn toggle(&mut self) {
        self.visible = !self.visible;
        crate::cursor::set_menu_open(self.visible);
        if self.visible {
            Self::unclip_cursor();
        }
        if self.cfg.debug {
            crate::log!("[menu] {}", if self.visible { "shown" } else { "hidden" });
        }
    }

    /// One frame's worth of file watching, for both stores.
    /// Is a DLL of this file name loaded in the game process? Asking the
    /// loader is the honest check: a file sitting in bin64 that failed to load
    /// (wrong build, missing loader) is not installed in any useful sense.
    fn module_loaded(name: &str) -> bool {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 buffer that outlives the
        // call; GetModuleHandleW only reads it and returns null when no such
        // module is loaded, without taking a reference.
        let h = unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(wide.as_ptr()) };
        !h.is_null()
    }

    /// Once a second: which of the sections' plugins are actually present.
    /// A section whose schema names no `Module` is always treated as loaded.
    fn poll_plugins(&mut self, now: Instant) {
        if self.last_plugin_poll.is_some_and(|t| now.duration_since(t) < PLUGIN_POLL) {
            return;
        }
        self.last_plugin_poll = Some(now);
        for entry in &mut self.sections {
            // Cloned rather than borrowed so the flag beside it can be written
            // in the same breath; it is one small string a second.
            let Some(module) = entry.store.model.section.module.clone() else { continue };
            let loaded = Self::module_loaded(&module);
            if loaded != entry.loaded {
                crate::log!("[menu] {module} is {}", if loaded { "loaded" } else { "not loaded" });
            }
            entry.loaded = loaded;
        }
    }

    /// One frame's worth of watching: the shared ini, once per section. Each
    /// store stats the same file and re-reads only its own `[Header]` out of
    /// it, which is a stat per section per second and nothing next to a frame.
    fn poll_files(&mut self, now: Instant) {
        let debug = self.cfg.debug;
        for entry in &mut self.sections {
            if entry.store.poll(now) && debug {
                crate::log!(
                    "[ini] {} changed on disk, [{}] refreshed",
                    entry.store.file_name(),
                    entry.ini_section()
                );
            }
        }
    }

    /// End of frame: write back whatever the widgets changed.
    fn flush_files(&mut self, now: Instant, released: bool) {
        let debug = self.cfg.debug;
        for entry in &mut self.sections {
            let outcome = entry.store.flush(now, released);
            let name = entry.store.file_name();
            match outcome {
                Some(Flushed::Wrote) if debug => {
                    crate::log!(
                        "[ini] wrote [{}] into {name}",
                        entry.store.model.section.ini_section
                    );
                }
                Some(Flushed::Created) => {
                    crate::log!("[ini] {name} was missing; created it");
                }
                Some(Flushed::Failed) => {
                    // The store already put the reason in its status line; log
                    // it once here so a bug report carries it too.
                    let status = entry.store.status.as_deref().unwrap_or(name);
                    crate::log!("[ini] {status}");
                }
                _ => {}
            }
        }
    }

    /// The banner at the top of the window: the goblin, then the name in the
    /// bigger face, then the rule that separates them from the settings.
    ///
    /// The image is square at [`LOGO_RATIO`] times the menu font size, so the
    /// whole row scales with `FontSize` and `Scale` and never needs a pixel
    /// figure of its own. imgui puts the text baseline at the top of the row
    /// after a `same_line`, so the cursor is nudged down by half the leftover
    /// height to centre the words against the picture; the title is drawn in
    /// the normal text colour, which is what makes it gold on the banner
    /// theme rather than the muted grey used for explanatory lines.
    ///
    /// With no texture there is nothing to centre against and the title is
    /// simply drawn where the cursor already is.
    fn header(&self, ui: &Ui) {
        let h = (LOGO_RATIO * self.font_px()).round();
        // The group is what makes the row as tall as the *image* even though
        // the text was nudged down inside it: moving the cursor by hand ends
        // imgui's line-height bookkeeping, so without this the separator
        // underneath would ride up over the goblin's feet.
        ui.group(|| {
            let mut top = ui.cursor_pos()[1];
            if let Some(tex) = self.logo {
                imgui::Image::new(tex, [h, h]).build(ui);
                ui.same_line();
                top = ui.cursor_pos()[1];
            }
            // `push_font` panics on an id this atlas does not hold, and a
            // panic here is a crash to desktop, so the index is resolved
            // through the live atlas and anything unexpected simply leaves
            // the default face in place.
            let _face = self
                .title_font
                .and_then(|i| ui.fonts().fonts().get(i).copied())
                .map(|id| ui.push_font(id));
            let text_h = ui.calc_text_size(TITLE)[1];
            if self.logo.is_some() && text_h < h {
                ui.set_cursor_pos([ui.cursor_pos()[0], top + (h - text_h) * 0.5]);
            }
            ui.text(TITLE);
        });
    }
}

impl ImguiRenderLoop for Overlay {
    fn initialize(&mut self, ctx: &mut Context, rc: &mut dyn hudhook::RenderContext) {
        // imgui otherwise writes an `imgui.ini` of window positions into the
        // process's working directory - which for a Steam game is somebody
        // else's folder - on every window move. The overlay has exactly one
        // window and sets its own position, so there is nothing to remember.
        ctx.set_ini_filename(None);
        // The font is rasterised at its final size instead of being blown up
        // from imgui's 13 px bitmap face, so the global font scale stays at
        // 1.0. Anything else here would stretch an already correctly sized
        // atlas.
        ctx.io_mut().font_global_scale = 1.0;
        // Applied once: the style scale compounds if repeated, and hudhook
        // calls this exactly once per pipeline (a swapchain reset rebuilds the
        // context and calls it again, on fresh defaults).
        ctx.style_mut().scale_all_sizes(self.scale);
        // Before hudhook's `setup_fonts` builds and uploads the atlas, which
        // is the whole reason the font is added from here.
        let px = self.font_px();
        self.add_font(ctx.fonts(), px);
        // A second face, same font file, for the header title. It is added
        // after the menu font on purpose: imgui draws everything in the first
        // face an atlas was given unless a `push_font` says otherwise, and
        // this one is only ever pushed around the title. `add_font` appends,
        // so its index is however many faces the atlas held before it.
        let before = ctx.fonts().fonts().len();
        self.add_font(ctx.fonts(), (px * TITLE_FONT_RATIO).round().max(1.0));
        self.title_font = Some(before);
        // The theme goes on after the scaling, so its rounding is applied in
        // scaled pixels exactly once, and again from `before_render` whenever
        // the picker changes it.
        Self::apply_theme(ctx.style_mut(), self.theme, self.scale);
        // The output encoding, applied here for the same reason: this is the
        // one place that runs once per pipeline, and hudhook reads both values
        // out of its own atomics on every frame after it. They are what stop
        // an sRGB menu from being written raw into the game's HDR10 swapchain,
        // which is what made it blown out and oversaturated.
        hudhook::output::set_paper_white_nits(self.cfg.hdr_brightness);
        hudhook::output::set_color_space_override(Self::color_space_override(self.cfg.color_space));
        crate::log!(
            "[menu] imgui context initialised, scale {:.2}, HDR paper white {:.0} nits, \
             colour space {}, theme {}",
            self.scale,
            self.cfg.hdr_brightness,
            self.cfg.color_space.as_str(),
            self.theme.name
        );
        crate::log!(
            "[menu] font: {} {:.0} px (FontSize {} x scale {:.2})",
            self.font_label, px, self.font_size, self.scale
        );
        // Re-uploaded every time, never carried over: after a swapchain reset
        // this runs again against a brand new engine whose texture heap has
        // never heard of the previous id.
        self.logo = match rc.load_texture(logo::RGBA, logo::WIDTH, logo::HEIGHT) {
            Ok(id) => Some(id),
            // A menu with no picture in the corner is a cosmetic loss and
            // nothing more, so this is a warning and the frame goes on.
            Err(e) => {
                crate::log!(
                    "[menu] WARN the logo texture could not be uploaded: {e:?}; the header \
                     shows its title only"
                );
                None
            }
        };
    }

    fn before_render(&mut self, ctx: &mut Context, _rc: &mut dyn hudhook::RenderContext) {
        if self.menu_key.pressed() {
            self.toggle();
        }
        if let Some(theme) = self.pending_theme.take() {
            self.theme = theme;
            Self::apply_theme(ctx.style_mut(), theme, self.scale);
            crate::log!(
                "[menu] theme: {} (Theme={} under [Overlay] in the ini keeps it)",
                theme.title, theme.name
            );
        }
        // While the menu is up imgui draws a cursor only if Windows is not
        // already showing one: the game hides the hardware cursor in the open
        // world but shows it in its own menus, and this must not add a second.
        // Checked every frame because the game flips it whenever it likes.
        ctx.io_mut().mouse_draw_cursor = self.visible && !Self::hardware_cursor_visible();

        let now = Instant::now();
        if !self.visible {
            // `render` returns early while the menu is hidden, so this is the
            // only place a write can happen once it is closed - and an edit
            // made in the last 250 ms before closing is exactly the one that
            // would otherwise be sitting in the debounce, unwritten.
            self.flush_files(now, true);
        }
        self.poll_files(now);
        self.poll_plugins(now);
    }

    fn render(&mut self, ui: &mut Ui) {
        if !self.visible {
            return;
        }
        let mut window_size = self.window_size;

        ui.window(TITLE)
            .position([40.0 * self.scale, 40.0 * self.scale], Condition::FirstUseEver)
            // Measured in game on 2026-09-08 (the `[menu] window size` log
            // line at scale 1.25): the size at which everything fits with no
            // scrollbar. Unscaled units, the same as the constraints below.
            .size([575.0 * self.scale, 770.0 * self.scale], Condition::FirstUseEver)
            .size_constraints([360.0 * self.scale, 120.0 * self.scale], [900.0 * self.scale, 1200.0 * self.scale])
            .build(|| {
                let t = self.theme;
                self.header(ui);
                ui.separator();
                self.pending_theme = theme_picker(ui, t, position_reserve(ui));
                position_readout(ui, t);
                ui.separator();
                ui.text_colored(t.dim, "Changes are saved to the ini as you make them.");
                ui.separator();

                // One collapsible section per subsystem, in the order they
                // sorted into. Nothing in here knows which mod it is drawing.
                if self.sections.is_empty() {
                    // Wrapped, not `text_colored`: it is the longest line in
                    // the window and would otherwise put a horizontal
                    // scrollbar under an otherwise empty menu.
                    let _dim = ui.push_style_color(StyleColor::Text, t.dim);
                    ui.text_wrapped(NO_SECTIONS);
                }
                for entry in &mut self.sections {
                    draw_section(ui, t, entry);
                }
                window_size = ui.window_size();
            });
        self.note_window_size(window_size, Instant::now());

        // Nothing held means a drag just ended, which is the moment a pending
        // edit should reach disk immediately rather than waiting out the
        // debounce.
        let released = !ui.is_any_item_active();
        self.flush_files(Instant::now(), released);
    }

    fn message_filter(&self, _io: &Io) -> MessageFilter {
        if self.visible {
            // Keyboard, mouse and WM_INPUT: while the menu is up, typing a
            // stack limit must not also swing the sword, and dragging a slider
            // must not turn the camera. Window messages are deliberately NOT
            // blocked - alt-tab, resize and close still have to work.
            MessageFilter::InputAll
        } else {
            MessageFilter::empty()
        }
    }
}

/// Header text for a section. The `##` suffix is the section's own ini
/// `[Header]`, which keeps imgui's widget id stable when the visible text
/// changes, so the section does not collapse or re-open the moment a module
/// appears or disappears - and two subsystems that happen to share a title
/// still get one id each. It has to be the header and not the file name: every
/// section names the same file now.
fn section_title(title: &str, ini_section: &str, loaded: bool) -> String {
    if loaded {
        format!("{title}##{ini_section}")
    } else {
        format!("{title} (not installed)##{ini_section}")
    }
}

/// How long a window size has to hold still before it is logged.
const RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

impl Overlay {
    /// Log the window's size once a resize has settled, in pixels and in the
    /// unscaled units `render` passes to `.size()`, so a size that looks
    /// right in game can be copied straight into the code as the default.
    fn note_window_size(&mut self, size: [f32; 2], now: Instant) {
        if size != self.window_size {
            self.window_size = size;
            self.window_resized_at = Some(now);
            return;
        }
        if self.window_resized_at.is_some_and(|at| now.duration_since(at) >= RESIZE_SETTLE) {
            self.window_resized_at = None;
            let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
            crate::log!(
                "[menu] window size {:.0}x{:.0} px = {:.0}x{:.0} at scale 1.0 (scale {:.2})",
                size[0],
                size[1],
                size[0] / scale,
                size[1] / scale,
                self.scale
            );
        }
    }
}

/// The theme picker at the top of the window. Returns the newly chosen theme
/// on the frame the choice is made, which the caller applies next frame.
///
/// `reserve` is how much room to leave on its right for [`position_readout`],
/// which shares this row. Left to itself an imgui combo takes about 65% of the
/// window, which at the default width leaves the coordinates nowhere to go, so
/// the width is set here rather than fought over afterwards.
fn theme_picker(ui: &Ui, current: &Theme, reserve: f32) -> Option<&'static Theme> {
    let mut index = current.index();
    let label = ui.calc_text_size("Theme")[0] + ui.clone_style().item_inner_spacing[0];
    let width = ui.content_region_avail()[0] - label - reserve;
    // A window dragged narrow enough would ask for a negative width, which
    // imgui reads as "measured back from the right edge" and turns into a
    // combo wider than the window rather than a smaller one. Below the floor
    // the picker keeps its default size and the readout gets squeezed instead.
    if width >= COMBO_MIN_WIDTH {
        ui.set_next_item_width(width);
    }
    let changed = ui.combo("Theme", &mut index, themes::ALL, |t| Cow::Borrowed(t.title));
    if !changed {
        return None;
    }
    themes::ALL.get(index).copied().filter(|t| *t != current)
}

/// The player's position, right-aligned on the theme picker's row.
///
/// The overlay reads no game memory (`crate` docs): these coordinates come
/// from [`desert_core::telemetry`], where the looter publishes them from its
/// own thread about thirty times a second. Nothing here knows how they were
/// obtained, and a build whose looter never started simply shows
/// [`NO_POSITION`] forever.
///
/// `y` is altitude, which is why the readout names all three axes instead of
/// implying that the first two are the map.
fn position_readout(ui: &Ui, t: &Theme) {
    let text = match telemetry::read() {
        Some(p) => format!("X {:.1}   Y {:.1}   Z {:.1}", p.x, p.y, p.z),
        None => NO_POSITION.to_string(),
    };
    ui.same_line();
    // imgui puts the cursor at the top of the row after a `same_line`, so the
    // text would sit against the combo's upper edge; half the frame padding
    // is the difference between a text line's height and a widget's.
    let row_top = ui.cursor_pos()[1];
    let right = ui.content_region_max()[0];
    let x = (right - ui.calc_text_size(&text)[0]).max(ui.cursor_pos()[0]);
    ui.set_cursor_pos([x, row_top + ui.clone_style().frame_padding[1]]);
    ui.text_colored(t.dim, &text);
    if ui.is_item_hovered() {
        ui.tooltip_text(POSITION_HELP);
    }
}

/// How wide to keep the right-hand end of the theme picker's row free.
///
/// Measured off a worst case rather than off the live text: the numbers change
/// every frame, and sizing the combo from them would make it twitch as the
/// player walks. Six digits and a sign is wider than the map.
fn position_reserve(ui: &Ui) -> f32 {
    ui.calc_text_size(READOUT_WIDEST)[0] + ui.clone_style().item_spacing[0]
}

/// One dim line explaining why a section is greyed out. Nothing when the
/// plugin is there, or when the schema names no module at all.
fn not_installed_line(ui: &Ui, t: &Theme, module: Option<&str>, loaded: bool) {
    if loaded {
        return;
    }
    if let Some(asi) = module {
        ui.text_colored(
            t.dim,
            format!("{asi} is not loaded in the game; these settings would go unread."),
        );
    }
}

/// One mod's whole section: the header, its presets, its fields, its notice
/// and its status line, all of it out of the schema.
///
/// The section is drawn even when its plugin is missing, but disabled: the
/// settings are still on disk and still worth looking at, and a greyed-out
/// section with a reason under it is a better answer than an empty window.
fn draw_section(ui: &Ui, t: &Theme, entry: &mut SectionEntry) {
    let loaded = entry.loaded;
    let title = section_title(entry.title(), entry.ini_section(), loaded);
    if !ui.collapsing_header(&title, TreeNodeFlags::DEFAULT_OPEN) {
        return;
    }
    not_installed_line(ui, t, entry.module(), loaded);
    let _disabled = ui.begin_disabled(!loaded);

    // Split borrow: the schema is read while the values are written, which is
    // why `DynModel` keeps them as two fields rather than behind a method.
    let DynModel { section, values } = &mut entry.store.model;
    let mut changed = false;
    if !section.presets.is_empty() {
        changed |= presets_row(ui, t, section, values);
        ui.separator();
    }
    changed |= fields(ui, t, section, values);
    if let Some(notice) = &section.notice {
        ui.text_colored(t.dim, notice);
    }

    if changed {
        entry.store.touch();
    }
    status_line(ui, t, entry.store.status.as_deref());
}

/// The preset buttons, two to a row: a label like "Rock and ore only" is long
/// enough that four across would run off the edge of the window at its default
/// width.
fn presets_row(ui: &Ui, t: &Theme, section: &Section, values: &mut [String]) -> bool {
    let mut changed = false;
    ui.text_colored(t.dim, section.presets_label.as_deref().unwrap_or(PRESETS_LABEL));
    for (i, preset) in section.presets.iter().enumerate() {
        if i % 2 == 1 {
            ui.same_line();
        }
        // The id carries the section's header and the preset's position, so
        // two subsystems may both have an "Everything" button.
        if ui.button(format!("{}##{}.preset{i}", preset.label, section.ini_section)) {
            changed |= dynmodel::apply_preset(section, values, preset);
        }
        if let Some(hint) = &preset.hint {
            if ui.is_item_hovered() {
                ui.tooltip_text(hint);
            }
        }
    }
    changed
}

/// Every field of the schema, in the order the schema lists them.
fn fields(ui: &Ui, t: &Theme, section: &Section, values: &mut [String]) -> bool {
    let mut changed = false;
    for (i, field) in section.fields.iter().enumerate() {
        let Some(slot) = values.get_mut(i) else { continue };
        if let Some(heading) = &field.heading {
            ui.text_colored(t.dim, heading);
        }
        if field.same_line {
            ui.same_line();
        }
        // `##<header>.<key>` so two sections can show the same label - which
        // `Enabled` and `Debug` do - and so a relabelled field keeps its widget
        // state. The file name would not do it: all three share one file.
        let label = format!("{}##{}.{}", field.label, section.ini_section, field.key);
        changed |= widget(ui, &label, &field.kind, slot);
        if let Some(help) = &field.help {
            if ui.is_item_hovered() {
                ui.tooltip_text(help);
            }
        }
    }
    changed
}

/// One field's widget, chosen by its kind. `slot` is the value in its written
/// spelling and is only replaced when the field accepts the new value, so a
/// widget can never put something in the ini that the plugin would reject.
fn widget(ui: &Ui, label: &str, kind: &Kind, slot: &mut String) -> bool {
    match kind {
        Kind::Bool { .. } => {
            let mut on = dynmodel::as_bool(slot.as_str());
            if !ui.checkbox(label, &mut on) {
                return false;
            }
            *slot = if on { "1" } else { "0" }.to_string();
            true
        }
        Kind::Int { min, max, step, slider, format, .. } => {
            // imgui's integer widgets are i32; a schema bound too wide for one
            // saturates, and `normalize` still holds the real range.
            let (lo, hi) = (dynmodel::i32_of(*min), dynmodel::i32_of(*max));
            let mut value = dynmodel::i32_of(dynmodel::as_int(kind, slot.as_str()));
            let edited = if *slider {
                let s = ui.slider_config(label, lo, hi);
                match format {
                    Some(f) => s.display_format(f).build(&mut value),
                    None => s.build(&mut value),
                }
            } else {
                int_input(ui, label, &mut value, (lo, hi), dynmodel::i32_of(*step).max(1))
            };
            // Ctrl+click on a slider types a number straight in, and imgui
            // does not clamp that unless asked, so clamp before writing.
            edited && set_text(kind, slot, &value.clamp(lo, hi).to_string())
        }
        Kind::Float { min, max, format, .. } => {
            let (lo, hi) = (min.max(-SLIDER_LIMIT), max.min(SLIDER_LIMIT));
            let mut value = dynmodel::as_float(kind, slot.as_str());
            let s = ui.slider_config(label, lo, hi);
            let edited = match format {
                Some(f) => s.display_format(f).build(&mut value),
                None => s.build(&mut value),
            };
            edited && set_text(kind, slot, &format!("{}", value.clamp(lo, hi)))
        }
        Kind::Choice { options, .. } => {
            let mut index = dynmodel::option_index(options, slot.as_str());
            if !ui.combo(label, &mut index, options, |o| Cow::Borrowed(o.as_str())) {
                return false;
            }
            match options.get(index) {
                Some(picked) => set_text(kind, slot, picked),
                None => false,
            }
        }
        Kind::Key { .. } => {
            // The picker is every name the ini parser accepts, so a binding
            // chosen here always reads back.
            let names = ini::key_names();
            let mut index = dynmodel::key_index(slot.as_str());
            if !ui.combo(label, &mut index, names, |n| Cow::Borrowed(*n)) {
                return false;
            }
            match names.get(index) {
                Some(picked) => set_text(kind, slot, picked),
                None => false,
            }
        }
    }
}

/// Put `raw` in `slot` if the field accepts it, in the field's own written
/// spelling. `false` leaves the old value alone and reports no change.
fn set_text(kind: &Kind, slot: &mut String, raw: &str) -> bool {
    match kind.normalize(raw) {
        Some(text) => {
            *slot = text;
            true
        }
        None => false,
    }
}

/// An `input_int` that only reports a change once the field is committed
/// (focus lost or Enter), and clamps to the plugin's accepted range at that
/// point. Reporting per keystroke would write "1" on the way to typing
/// "1000", and the plugin would reject it.
fn int_input(ui: &Ui, label: &str, value: &mut i32, (lo, hi): (i32, i32), step: i32) -> bool {
    let edited = ui.input_int(label, value).step(step).step_fast(step * 10).build();
    // Two ways to finish: typing then leaving the field (deactivated after
    // edit), or clicking the +/- buttons, which edit without ever leaving the
    // field focused. Neither commits mid-keystroke.
    if ui.is_item_deactivated_after_edit() || (edited && !ui.is_item_active()) {
        *value = (*value).clamp(lo, hi);
        return true;
    }
    false
}

/// The red failure line under a section, or nothing at all when the last read
/// and write both worked.
fn status_line(ui: &Ui, t: &Theme, status: Option<&str>) {
    if let Some(msg) = status {
        ui.text_colored(t.error, msg);
    }
}
