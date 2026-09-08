//! The menu: hudhook's render loop, the key that shows it, and the input
//! blocking that keeps the game from reacting while it is on screen.
//!
//! Everything the window does is a read or a write of the two ini files
//! through [`crate::store`]. There is no other channel to the plugins, and
//! this module never touches game memory, calls a game function, or holds a
//! lock a render thread could block on.
//!
//! Three things about running inside somebody else's frame:
//!
//! * hudhook calls [`ImguiRenderLoop::message_filter`] *before*
//!   [`ImguiRenderLoop::before_render`] each frame, so a toggle noticed in
//!   `before_render` starts blocking input on the next frame. One frame of
//!   game input on the way in and out is not worth a second mechanism.
//! * The game hides the hardware cursor and clips it to the window, so imgui
//!   has to draw its own (`io.mouse_draw_cursor`, only while the hardware
//!   cursor is really hidden) and the clip rectangle has
//!   to be released, or the pointer cannot reach the window's edges.
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

use crate::config::{ColorSpace, Config, FontChoice};
use crate::logo;
use crate::model::{
    GathererModel, LooterModel, GATHER_RANGE, MS_RANGE, MULT_RANGE, SCAN_RANGE, STACK_LIMIT_RANGE,
};
use crate::presets;
use crate::store::{Flushed, Store};
use crate::theme::{Role, Theme};
use crate::themes;

/// How often the two plugin DLLs are looked up in the process.
const PLUGIN_POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// The largest font file the overlay will pull into the game's address space.
/// Every Windows system font is a couple of megabytes at most (`cambria.ttc`,
/// the biggest of the ones the ini suggests, is about 3 MB); the cap only
/// exists so a `Font` pointing at something that is not a font cannot cost the
/// game hundreds of megabytes before stb_truetype rejects it.
const FONT_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// What the log calls the font when there is no file behind it.
const BUILT_IN_FONT: &str = "built-in ProggyClean";

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
    /// Whether `DesertLooter.asi` / `DesertGatherer.asi` are loaded in this
    /// process, refreshed every [`PLUGIN_POLL`]. A section whose plugin is
    /// absent is drawn greyed out with a "not installed" note: its ini would
    /// still be written, but nothing would read it.
    looter_loaded: bool,
    gatherer_loaded: bool,
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
    looter: Store<LooterModel>,
    gatherer: Store<GathererModel>,
}

impl Overlay {
    /// Reads both ini files and the menu font once. Called from the plugin's
    /// own thread, never from `DllMain` and never from a render thread.
    pub fn new(cfg: Config) -> Self {
        let dir = desert_core::log::exe_dir();
        let now = Instant::now();
        let visible = cfg.show_on_start;
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
            looter: Store::new(&dir, now),
            gatherer: Store::new(&dir, now),
            looter_loaded: false,
            gatherer_loaded: false,
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
                    desert_core::log::write(&format!(
                        "[menu] WARN font {file}: the Windows Fonts directory could not be found; \
                         using the {BUILT_IN_FONT} font"
                    ));
                    return (None, BUILT_IN_FONT.to_string());
                };
                (dir.join(file), Self::font_label(file, *face))
            }
        };
        match Self::read_font_file(&path) {
            Ok(bytes) => (Some(bytes), shown),
            Err(why) => {
                desert_core::log::write(&format!(
                    "[menu] WARN font {}: {why}; using the {BUILT_IN_FONT} font",
                    path.display()
                ));
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

    /// The game hides the cursor and calls `ClipCursor` to pin it inside the
    /// window (and, in first-person-style camera control, to a single point).
    /// imgui gets its mouse position from the raw-input deltas hudhook feeds
    /// it, but a clipped cursor still cannot travel, so release the clip when
    /// the menu opens. The game re-establishes it on its own the next time it
    /// wants to; that is why this is only done on the opening edge and never
    /// undone here.
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
        if self.visible {
            Self::unclip_cursor();
        }
        if self.cfg.debug {
            desert_core::log::write(&format!(
                "[menu] {}",
                if self.visible { "shown" } else { "hidden" }
            ));
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

    /// Once a second: which plugins are actually present.
    fn poll_plugins(&mut self, now: Instant) {
        if self.last_plugin_poll.is_some_and(|t| now.duration_since(t) < PLUGIN_POLL) {
            return;
        }
        self.last_plugin_poll = Some(now);
        let (l, g) = (Self::module_loaded("DesertLooter.asi"), Self::module_loaded("DesertGatherer.asi"));
        if (l, g) != (self.looter_loaded, self.gatherer_loaded) {
            desert_core::log::write(&format!(
                "[menu] plugins loaded: DesertLooter.asi={} DesertGatherer.asi={}",
                l as u8, g as u8
            ));
        }
        self.looter_loaded = l;
        self.gatherer_loaded = g;
    }

    fn poll_files(&mut self, now: Instant) {
        if self.looter.poll(now) && self.cfg.debug {
            desert_core::log::write("[ini] DesertLooter.ini changed on disk, menu refreshed");
        }
        if self.gatherer.poll(now) && self.cfg.debug {
            desert_core::log::write("[ini] DesertGatherer.ini changed on disk, menu refreshed");
        }
    }

    /// End of frame: write back whatever the widgets changed.
    fn flush_files(&mut self, now: Instant, released: bool) {
        for (name, outcome) in [
            ("DesertLooter.ini", self.looter.flush(now, released)),
            ("DesertGatherer.ini", self.gatherer.flush(now, released)),
        ] {
            match outcome {
                Some(Flushed::Wrote) if self.cfg.debug => {
                    desert_core::log::write(&format!("[ini] wrote {name}"));
                }
                Some(Flushed::Created) => {
                    desert_core::log::write(&format!("[ini] {name} was missing; created it"));
                }
                Some(Flushed::Failed) => {
                    // The store already put the reason in its status line; log
                    // it once here so a bug report carries it too.
                    let status = if name.contains("Looter") {
                        self.looter.status.as_deref()
                    } else {
                        self.gatherer.status.as_deref()
                    };
                    desert_core::log::write(&format!("[ini] {}", status.unwrap_or(name)));
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
        desert_core::log::write(&format!(
            "[menu] imgui context initialised, scale {:.2}, HDR paper white {:.0} nits, colour \
             space {}, theme {}",
            self.scale,
            self.cfg.hdr_brightness,
            self.cfg.color_space.as_str(),
            self.theme.name
        ));
        desert_core::log::write(&format!(
            "[menu] font: {} {:.0} px (FontSize {} x scale {:.2})",
            self.font_label, px, self.font_size, self.scale
        ));
        // Re-uploaded every time, never carried over: after a swapchain reset
        // this runs again against a brand new engine whose texture heap has
        // never heard of the previous id.
        self.logo = match rc.load_texture(logo::RGBA, logo::WIDTH, logo::HEIGHT) {
            Ok(id) => Some(id),
            // A menu with no picture in the corner is a cosmetic loss and
            // nothing more, so this is a warning and the frame goes on.
            Err(e) => {
                desert_core::log::write(&format!(
                    "[menu] WARN the logo texture could not be uploaded: {e:?}; the header shows \
                     its title only"
                ));
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
            desert_core::log::write(&format!("[menu] theme: {} (Theme={} in DesertOverlay.ini keeps it)", theme.title, theme.name));
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
        let mut changed_looter = false;
        let mut changed_gatherer = false;
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
                self.pending_theme = theme_picker(ui, t);
                ui.separator();
                ui.text_colored(t.dim, "Changes are saved to the ini files as you make them.");
                ui.separator();

                if ui.collapsing_header(section_title("Desert Looter", self.looter_loaded), TreeNodeFlags::DEFAULT_OPEN) {
                    not_installed_line(ui, t, "DesertLooter.asi", self.looter_loaded);
                    let _d = ui.begin_disabled(!self.looter_loaded);
                    changed_looter |= presets_row(ui, t, &mut self.looter.model);
                    ui.separator();
                    changed_looter |= looter_section(ui, t, &mut self.looter.model);
                    status_line(ui, t, self.looter.status.as_deref());
                }
                if ui.collapsing_header(section_title("Desert Gatherer", self.gatherer_loaded), TreeNodeFlags::DEFAULT_OPEN) {
                    not_installed_line(ui, t, "DesertGatherer.asi", self.gatherer_loaded);
                    let _d = ui.begin_disabled(!self.gatherer_loaded);
                    changed_gatherer |= gatherer_section(ui, t, &mut self.gatherer.model);
                    status_line(ui, t, self.gatherer.status.as_deref());
                }
                window_size = ui.window_size();
            });
        self.note_window_size(window_size, Instant::now());

        if changed_looter {
            self.looter.touch();
        }
        if changed_gatherer {
            self.gatherer.touch();
        }
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

/// The preset buttons, two to a row: "Rock and ore only" is long enough that
/// four across would run off the edge of the window at its default width.
/// Header text for a plugin section. The `##` suffix keeps imgui's widget id
/// stable when the visible text changes, so the section does not collapse or
/// re-open the moment a plugin appears or disappears.
fn section_title(name: &str, loaded: bool) -> String {
    if loaded {
        format!("{name}##{name}")
    } else {
        format!("{name} (not installed)##{name}")
    }
}

/// One dim line explaining why a section is greyed out. Nothing when the
/// plugin is there.
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
            desert_core::log::write(&format!(
                "[menu] window size {:.0}x{:.0} px = {:.0}x{:.0} at scale 1.0 (scale {:.2})",
                size[0],
                size[1],
                size[0] / scale,
                size[1] / scale,
                self.scale
            ));
        }
    }
}

/// The theme picker at the top of the window. Returns the newly chosen theme
/// on the frame the choice is made, which the caller applies next frame.
fn theme_picker(ui: &Ui, current: &Theme) -> Option<&'static Theme> {
    let mut index = current.index();
    let changed = ui.combo("Theme", &mut index, themes::ALL, |t| Cow::Borrowed(t.title));
    // The blurb only shows as a tooltip: a line of prose under the picker
    // was clutter once a theme had been chosen.
    if ui.is_item_hovered() {
        ui.tooltip_text(current.blurb);
    }
    if !changed {
        return None;
    }
    themes::ALL.get(index).copied().filter(|t| *t != current)
}

fn not_installed_line(ui: &Ui, t: &Theme, asi: &str, loaded: bool) {
    if !loaded {
        ui.text_colored(t.dim, format!("{asi} is not loaded in the game; these settings would go unread."));
    }
}

fn presets_row(ui: &Ui, t: &Theme, m: &mut LooterModel) -> bool {
    let mut changed = false;
    ui.text_colored(t.dim, "Presets (what auto-loot picks up):");
    for (i, p) in presets::ALL.iter().enumerate() {
        if i % 2 == 1 {
            ui.same_line();
        }
        if ui.button(p.label()) {
            p.apply(m);
            changed = true;
        }
        if ui.is_item_hovered() {
            ui.tooltip_text(p.hint());
        }
    }
    changed
}

fn looter_section(ui: &Ui, t: &Theme, m: &mut LooterModel) -> bool {
    let mut changed = false;

    changed |= ui.checkbox("Enabled##looter", &mut m.enabled);
    changed |= ui.checkbox("Auto gather", &mut m.auto_gather);

    ui.text_colored(t.dim, "Gather families:");
    changed |= ui.checkbox("Foraging##looter", &mut m.gather_foraging);
    ui.same_line();
    changed |= ui.checkbox("Logging##looter", &mut m.gather_logging);
    ui.same_line();
    changed |= ui.checkbox("Mining##looter", &mut m.gather_mining);
    ui.same_line();
    changed |= ui.checkbox("Ore##looter", &mut m.gather_ore);

    changed |= ui.checkbox("Ground items", &mut m.gather_items);
    ui.same_line();
    changed |= ui.checkbox("Dropped gear", &mut m.gather_gear);
    ui.same_line();
    changed |= ui.checkbox("Unarmed nodes", &mut m.gather_unarmed);

    changed |= ui
        .slider_config("Scan range", SCAN_RANGE.0, SCAN_RANGE.1)
        .display_format("%.0f m")
        .build(&mut m.scan_range);
    changed |= ui
        .slider_config("Gather range", GATHER_RANGE.0, GATHER_RANGE.1)
        .display_format("%.1f m")
        .build(&mut m.gather_range);

    changed |= int_input(ui, "Gather interval (ms)", &mut m.gather_interval_ms, MS_RANGE, 50);
    changed |= int_input(ui, "Node cooldown (ms)", &mut m.node_cooldown_ms, MS_RANGE, 500);
    changed |= int_input(ui, "Stack limit", &mut m.stack_limit, STACK_LIMIT_RANGE, 10);

    changed
}

fn gatherer_section(ui: &Ui, t: &Theme, m: &mut GathererModel) -> bool {
    let mut changed = false;

    changed |= ui.checkbox("Enabled##gatherer", &mut m.enabled);
    ui.same_line();
    changed |= ui.checkbox("Dry run", &mut m.dry_run);
    if ui.is_item_hovered() {
        ui.tooltip_text("Log what would change and write nothing to the game.");
    }

    ui.text_colored(t.dim, "Yield multipliers:");
    for (label, slot) in [
        ("Foraging##gatherer", &mut m.foraging),
        ("Logging##gatherer", &mut m.logging),
        ("Mining##gatherer", &mut m.mining),
        ("Ore##gatherer", &mut m.ore),
    ] {
        changed |= ui.slider_config(label, MULT_RANGE.0, MULT_RANGE.1).display_format("%dx").build(slot);
    }
    ui.text_colored(
        t.dim,
        "Takes effect on records the game loads next; already-loaded ones keep their yields.",
    );

    changed
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
