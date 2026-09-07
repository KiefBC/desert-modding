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

use std::time::Instant;

use hudhook::{ImguiRenderLoop, MessageFilter};
use imgui::{Condition, Context, Io, TreeNodeFlags, Ui};

use desert_core::hotkey::Hotkey;

use crate::config::Config;
use crate::model::{
    GathererModel, LooterModel, GATHER_RANGE, MS_RANGE, MULT_RANGE, SCAN_RANGE, STACK_LIMIT_RANGE,
};
use crate::presets;
use crate::store::{Flushed, Store};

/// Red, for the one-line failure message under a section.
const RED: [f32; 4] = [1.0, 0.35, 0.35, 1.0];
/// Dimmed, for the explanatory lines.
const DIM: [f32; 4] = [0.65, 0.65, 0.65, 1.0];

/// How often the two plugin DLLs are looked up in the process.
const PLUGIN_POLL: std::time::Duration = std::time::Duration::from_secs(1);

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
    visible: bool,
    menu_key: Hotkey,
    looter: Store<LooterModel>,
    gatherer: Store<GathererModel>,
}

impl Overlay {
    /// Reads both ini files once. Called from the plugin's own thread, never
    /// from `DllMain`.
    pub fn new(cfg: Config) -> Self {
        let dir = desert_core::log::exe_dir();
        let now = Instant::now();
        let visible = cfg.show_on_start;
        let scale = if cfg.scale > 0.0 { cfg.scale } else { Self::system_scale() };
        Overlay {
            menu_key: Hotkey::new(cfg.key_menu),
            looter: Store::new(&dir, now),
            gatherer: Store::new(&dir, now),
            looter_loaded: false,
            gatherer_loaded: false,
            last_plugin_poll: None,
            cfg,
            scale,
            visible,
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
}

impl ImguiRenderLoop for Overlay {
    fn initialize(&mut self, ctx: &mut Context, _rc: &mut dyn hudhook::RenderContext) {
        // imgui otherwise writes an `imgui.ini` of window positions into the
        // process's working directory - which for a Steam game is somebody
        // else's folder - on every window move. The overlay has exactly one
        // window and sets its own position, so there is nothing to remember.
        ctx.set_ini_filename(None);
        // Applied once: the style scale compounds if repeated, and hudhook
        // calls this exactly once per pipeline (a swapchain reset rebuilds the
        // context and calls it again, on fresh defaults).
        ctx.io_mut().font_global_scale = self.scale;
        ctx.style_mut().scale_all_sizes(self.scale);
        desert_core::log::write(&format!("[menu] imgui context initialised, scale {:.2}", self.scale));
    }

    fn before_render(&mut self, ctx: &mut Context, _rc: &mut dyn hudhook::RenderContext) {
        if self.menu_key.pressed() {
            self.toggle();
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

        ui.window("Desert Mods")
            .position([40.0 * self.scale, 40.0 * self.scale], Condition::FirstUseEver)
            .size([460.0 * self.scale, 0.0], Condition::FirstUseEver)
            .size_constraints([360.0 * self.scale, 120.0 * self.scale], [900.0 * self.scale, 1200.0 * self.scale])
            .build(|| {
                ui.text_colored(DIM, "Changes are saved to the ini files as you make them.");
                ui.separator();

                {
                    let _d = ui.begin_disabled(!self.looter_loaded);
                    changed_looter |= presets_row(ui, &mut self.looter.model);
                }
                ui.separator();

                if ui.collapsing_header(section_title("Desert Looter", self.looter_loaded), TreeNodeFlags::DEFAULT_OPEN) {
                    not_installed_line(ui, "DesertLooter.asi", self.looter_loaded);
                    let _d = ui.begin_disabled(!self.looter_loaded);
                    changed_looter |= looter_section(ui, &mut self.looter.model);
                    status_line(ui, self.looter.status.as_deref());
                }
                if ui.collapsing_header(section_title("Desert Gatherer", self.gatherer_loaded), TreeNodeFlags::DEFAULT_OPEN) {
                    not_installed_line(ui, "DesertGatherer.asi", self.gatherer_loaded);
                    let _d = ui.begin_disabled(!self.gatherer_loaded);
                    changed_gatherer |= gatherer_section(ui, &mut self.gatherer.model);
                    status_line(ui, self.gatherer.status.as_deref());
                }
            });

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
fn not_installed_line(ui: &Ui, asi: &str, loaded: bool) {
    if !loaded {
        ui.text_colored(DIM, format!("{asi} is not loaded in the game; these settings would go unread."));
    }
}

fn presets_row(ui: &Ui, m: &mut LooterModel) -> bool {
    let mut changed = false;
    ui.text_colored(DIM, "Presets (what auto-loot picks up):");
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

fn looter_section(ui: &Ui, m: &mut LooterModel) -> bool {
    let mut changed = false;

    changed |= ui.checkbox("Enabled##looter", &mut m.enabled);
    changed |= ui.checkbox("Auto gather", &mut m.auto_gather);

    ui.text_colored(DIM, "Gather families:");
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

fn gatherer_section(ui: &Ui, m: &mut GathererModel) -> bool {
    let mut changed = false;

    changed |= ui.checkbox("Enabled##gatherer", &mut m.enabled);
    ui.same_line();
    changed |= ui.checkbox("Dry run", &mut m.dry_run);
    if ui.is_item_hovered() {
        ui.tooltip_text("Log what would change and write nothing to the game.");
    }

    ui.text_colored(DIM, "Yield multipliers:");
    for (label, slot) in [
        ("Foraging##gatherer", &mut m.foraging),
        ("Logging##gatherer", &mut m.logging),
        ("Mining##gatherer", &mut m.mining),
        ("Ore##gatherer", &mut m.ore),
    ] {
        changed |= ui.slider_config(label, MULT_RANGE.0, MULT_RANGE.1).display_format("%dx").build(slot);
    }
    ui.text_colored(
        DIM,
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
fn status_line(ui: &Ui, status: Option<&str>) {
    if let Some(msg) = status {
        ui.text_colored(RED, msg);
    }
}
