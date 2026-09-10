//! Colour themes for the menu, as plain data.
//!
//! A theme is a name, a handful of style numbers and a list of colour
//! overrides on top of Dear ImGui's stock dark palette. Nothing in here knows
//! about imgui: [`Role`] mirrors imgui's `StyleColor` slot for slot so the
//! Windows-only side ([`crate::ui`]) can map one to the other with a match,
//! and this module keeps compiling and unit-testing on Linux. The themes
//! themselves live one per file under [`crate::themes`].
//!
//! Colours are straight sRGB `[r, g, b, a]` in `0.0..=1.0`, exactly what imgui
//! wants; the HDR conversion happens later, in the pixel shader, and a theme
//! never has to think about it.

/// One colour, sRGB, non-premultiplied, each channel `0.0..=1.0`.
pub type Rgba = [f32; 4];

/// A colour from a hex triplet and an alpha: `rgb(0xC8A951, 1.0)` is gold.
pub const fn rgb(hex: u32, alpha: f32) -> Rgba {
    [
        ((hex >> 16) & 0xFF) as f32 / 255.0,
        ((hex >> 8) & 0xFF) as f32 / 255.0,
        (hex & 0xFF) as f32 / 255.0,
        alpha,
    ]
}

/// The imgui style slot a colour goes into. Same names, same meaning as
/// imgui's `StyleColor`; the doc line on each says what it paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Ordinary text.
    Text,
    /// Text inside a disabled section (a plugin that is not installed).
    TextDisabled,
    /// The window's background.
    WindowBg,
    /// Child window background (unused today).
    ChildBg,
    /// Popup and combo dropdown background.
    PopupBg,
    /// Window and frame borders.
    Border,
    /// Border shadow (normally transparent).
    BorderShadow,
    /// Background of checkboxes, sliders, text inputs.
    FrameBg,
    /// ... while hovered.
    FrameBgHovered,
    /// ... while held or focused.
    FrameBgActive,
    /// Title bar of an unfocused window.
    TitleBg,
    /// Title bar of the focused window.
    TitleBgActive,
    /// Title bar of a collapsed window.
    TitleBgCollapsed,
    /// Menu bar background (unused today).
    MenuBarBg,
    /// Scrollbar track.
    ScrollbarBg,
    /// Scrollbar thumb.
    ScrollbarGrab,
    /// ... while hovered.
    ScrollbarGrabHovered,
    /// ... while dragged.
    ScrollbarGrabActive,
    /// The tick inside a checked checkbox.
    CheckMark,
    /// Slider knob.
    SliderGrab,
    /// Slider knob while dragged.
    SliderGrabActive,
    /// Buttons (the presets row, the +/- on number fields).
    Button,
    /// ... while hovered.
    ButtonHovered,
    /// ... while pressed.
    ButtonActive,
    /// Collapsing header bars (the "Desert Looter" / "Desert Gatherer" rows).
    Header,
    /// ... while hovered.
    HeaderHovered,
    /// ... while pressed.
    HeaderActive,
    /// Separator lines.
    Separator,
    /// ... while hovered.
    SeparatorHovered,
    /// ... while dragged.
    SeparatorActive,
    /// The resize grip in the window corner.
    ResizeGrip,
    /// ... while hovered.
    ResizeGripHovered,
    /// ... while dragged.
    ResizeGripActive,
    /// Tabs (unused today).
    Tab,
    /// Tabs while hovered (unused today).
    TabHovered,
    /// The active tab (unused today).
    TabActive,
    /// Tabs in an unfocused window (unused today).
    TabUnfocused,
    /// The active tab in an unfocused window (unused today).
    TabUnfocusedActive,
    /// Plot lines (unused today).
    PlotLines,
    /// Plot lines while hovered (unused today).
    PlotLinesHovered,
    /// Histogram bars (unused today).
    PlotHistogram,
    /// Histogram bars while hovered (unused today).
    PlotHistogramHovered,
    /// Table header background (unused today).
    TableHeaderBg,
    /// Strong table borders (unused today).
    TableBorderStrong,
    /// Light table borders (unused today).
    TableBorderLight,
    /// Table row background (unused today).
    TableRowBg,
    /// Alternate table row background (unused today).
    TableRowBgAlt,
    /// Selection highlight in text inputs.
    TextSelectedBg,
    /// Drag and drop target (unused today).
    DragDropTarget,
    /// Keyboard navigation highlight.
    NavHighlight,
    /// Window-switching highlight.
    NavWindowingHighlight,
    /// Dimming behind window switching.
    NavWindowingDimBg,
    /// Dimming behind a modal.
    ModalWindowDimBg,
}

/// A complete look for the menu.
///
/// Every field is a plain value so a theme can be a `const`; `colors` is a
/// function only because a `Vec` cannot be built in one.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// The ini spelling (`Theme=crimson`), lowercase, no spaces. Also the log
    /// spelling.
    pub name: &'static str,
    /// What the picker shows.
    pub title: &'static str,
    /// One line under the picker saying what the theme is going for.
    pub blurb: &'static str,
    /// The overlay's own dimmed explanatory text (hints, family labels).
    pub dim: Rgba,
    /// The overlay's own failure line under a section.
    pub error: Rgba,
    /// Corner rounding of the window, in unscaled pixels.
    pub window_rounding: f32,
    /// Corner rounding of frames (checkboxes, sliders, inputs, buttons).
    pub frame_rounding: f32,
    /// Corner rounding of slider knobs.
    pub grab_rounding: f32,
    /// Window border thickness; `0.0` for none.
    pub window_border: f32,
    /// Frame border thickness; `0.0` for none.
    pub frame_border: f32,
    /// Colour overrides applied on top of imgui's stock dark palette. A role
    /// that is not listed keeps the stock colour, so a theme only has to name
    /// what it changes.
    pub colors: fn() -> Vec<(Role, Rgba)>,
}

/// Themes are compared by name: two themes with the same name are the same
/// entry in [`crate::themes::ALL`], and a function pointer comparison would
/// say nothing useful anyway.
impl PartialEq for Theme {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Theme {
    /// The theme called `name` (case-insensitive), if there is one.
    pub fn by_name(name: &str) -> Option<&'static Theme> {
        let wanted = name.trim().to_ascii_lowercase();
        crate::themes::ALL.iter().copied().find(|t| t.name == wanted)
    }

    /// Position of this theme in [`crate::themes::ALL`], for the picker.
    pub fn index(&self) -> usize {
        crate::themes::ALL.iter().position(|t| t.name == self.name).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::{ALL, DEFAULT};

    #[test]
    fn rgb_unpacks_channels() {
        assert_eq!(rgb(0xFF0000, 1.0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(rgb(0x000000, 0.5), [0.0, 0.0, 0.0, 0.5]);
        let g = rgb(0xC8A951, 1.0);
        assert!((g[0] - 200.0 / 255.0).abs() < 1e-6);
        assert!((g[1] - 169.0 / 255.0).abs() < 1e-6);
        assert!((g[2] - 81.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn default_theme_exists_and_names_are_unique() {
        assert!(Theme::by_name(DEFAULT).is_some(), "DEFAULT {DEFAULT:?} not in ALL");
        for (i, a) in ALL.iter().enumerate() {
            assert_eq!(a.name, a.name.trim().to_ascii_lowercase(), "{}: name must be lowercase", a.name);
            assert!(!a.name.contains(' '), "{}: name must have no spaces", a.name);
            assert!(!a.title.is_empty() && !a.blurb.is_empty(), "{}: title and blurb required", a.name);
            for b in ALL.iter().skip(i + 1) {
                assert_ne!(a.name, b.name, "duplicate theme name");
            }
            assert_eq!(a.index(), i);
        }
    }

    #[test]
    fn every_colour_is_in_range() {
        let ok = |c: &Rgba| c.iter().all(|v| (0.0..=1.0).contains(v));
        for t in ALL {
            assert!(ok(&t.dim), "{}: dim out of range", t.name);
            assert!(ok(&t.error), "{}: error out of range", t.name);
            let colors = (t.colors)();
            for (role, c) in &colors {
                assert!(ok(c), "{}: {role:?} out of range: {c:?}", t.name);
            }
            for (i, (a, _)) in colors.iter().enumerate() {
                for (b, _) in colors.iter().skip(i + 1) {
                    assert_ne!(a, b, "{}: {a:?} listed twice", t.name);
                }
            }
            for v in [t.window_rounding, t.frame_rounding, t.grab_rounding, t.window_border, t.frame_border] {
                assert!((0.0..=32.0).contains(&v), "{}: style number {v} out of range", t.name);
            }
        }
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(Theme::by_name(" Classic ").map(|t| t.name), Some("classic"));
        assert!(Theme::by_name("no-such-theme").is_none());
    }
}
