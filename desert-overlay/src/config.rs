//! `DesertOverlay.ini` beside the game exe. Missing file or key => defaults.
//!
//! Same shape as `desert_looter::config` and `desert_gatherer::config`:
//! `parse` returns the config plus ready-to-log warnings, a bad value never
//! replaces the default, and the tokeniser and the virtual-key name table come
//! from `desert_core::ini`.
//!
//! This file is the overlay's own settings only. The two files the menu edits
//! are described by [`crate::model`]; the overlay never rereads its own ini
//! while the game runs, because changing the menu key or the master switch
//! from inside the menu it draws makes no sense.

use desert_core::ini::{self, Line};

/// `Insert`. The default has to be a key the game does not use and that a
/// laptop keyboard actually has, which rules out most of F1..F12 (Desert
/// Looter already owns F7/F9/F10/F11).
pub const DEFAULT_KEY_MENU: u16 = 0x2D;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Master switch. 0 = the plugin loads, logs one line and installs no
    /// graphics hook at all, so the game renders exactly as it would without
    /// the `.asi`.
    pub enabled: bool,
    /// 1 = verbose log: every ini write, every reload, and hudhook's own INFO
    /// messages as well as its warnings and errors.
    pub debug: bool,
    /// Virtual-key code that shows and hides the menu.
    pub key_menu: u16,
    /// 1 = the menu is already open when the game reaches the first frame.
    pub show_on_start: bool,
    /// UI scale. `0.0` = automatic from the Windows display scaling (125% =>
    /// 1.25); otherwise a fixed factor in `SCALE_MIN..=SCALE_MAX`.
    pub scale: f32,
    /// Paper white in nits: how bright plain white is drawn on an HDR
    /// swapchain. Ignored entirely in SDR. In `HDR_MIN..=HDR_MAX`.
    pub hdr_brightness: f32,
    /// Which colour space the menu's pixels are encoded for.
    /// [`ColorSpace::Auto`] follows what the swapchain says.
    pub color_space: ColorSpace,
}

/// Smallest and largest fixed `Scale` accepted from the ini.
pub const SCALE_MIN: f32 = 0.5;
pub const SCALE_MAX: f32 = 4.0;

/// Smallest and largest `HdrBrightness` accepted from the ini. 80 is the scRGB
/// unit; past ~1000 the menu is brighter than anything the game draws.
pub const HDR_MIN: f32 = 80.0;
pub const HDR_MAX: f32 = 1000.0;

/// The ITU-R BT.2408 reference level for diffuse white, and what ReShade's own
/// overlay uses (`HdrOverlayBrightness=203`).
pub const DEFAULT_HDR_BRIGHTNESS: f32 = 203.0;

/// The `ColorSpace` key: what the menu's pixels are encoded for.
///
/// The game presents through an HDR10 (PQ) swapchain, and sRGB colours written
/// into one unconverted come out blown out. [`ColorSpace::Auto`] reads the
/// swapchain's own colour space and is right unless that detection is wrong,
/// which is the only reason the other three exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorSpace {
    /// Follow the swapchain. The default.
    #[default]
    Auto,
    /// Force sRGB passthrough (what every SDR display wants).
    Sdr,
    /// Force HDR10: BT.2020 primaries, PQ transfer.
    Hdr10,
    /// Force scRGB: linear BT.709, `1.0` = 80 nits.
    ScRgb,
}

impl ColorSpace {
    /// The ini spelling, which is also how the startup log prints it.
    pub fn as_str(self) -> &'static str {
        match self {
            ColorSpace::Auto => "auto",
            ColorSpace::Sdr => "sdr",
            ColorSpace::Hdr10 => "hdr10",
            ColorSpace::ScRgb => "scrgb",
        }
    }

    /// Parse an ini value. Case-insensitive; `None` for anything unknown.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(ColorSpace::Auto),
            "sdr" => Some(ColorSpace::Sdr),
            "hdr10" => Some(ColorSpace::Hdr10),
            "scrgb" => Some(ColorSpace::ScRgb),
            _ => None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            debug: false,
            key_menu: DEFAULT_KEY_MENU,
            show_on_start: false,
            scale: 0.0,
            hdr_brightness: DEFAULT_HDR_BRIGHTNESS,
            color_space: ColorSpace::Auto,
        }
    }
}

/// Parse ini text. Unknown keys and bad values are reported back so they can
/// be logged; the config always comes back usable.
pub fn parse(text: &str) -> (Config, Vec<String>) {
    let mut cfg = Config::default();
    let mut warnings = Vec::new();
    for line in ini::lines(text) {
        let (k, v) = match line {
            Line::Pair(k, v) => (k, v),
            Line::Bad(w) => {
                warnings.push(w);
                continue;
            }
        };
        match k.to_ascii_lowercase().as_str() {
            "enabled" => cfg.enabled = ini::parse_bool(v),
            "debug" => cfg.debug = ini::parse_bool(v),
            "showonstart" => cfg.show_on_start = ini::parse_bool(v),
            "scale" => match v.trim().parse::<f32>() {
                Ok(f) if f == 0.0 || (SCALE_MIN..=SCALE_MAX).contains(&f) => cfg.scale = f,
                _ => warnings.push(format!(
                    "Scale: bad value {v:?} (0 = auto, or {SCALE_MIN}..{SCALE_MAX}), keeping {}",
                    cfg.scale
                )),
            },
            "hdrbrightness" => match v.trim().parse::<f32>() {
                Ok(f) if (HDR_MIN..=HDR_MAX).contains(&f) => cfg.hdr_brightness = f,
                _ => warnings.push(format!(
                    "HdrBrightness: bad value {v:?} ({HDR_MIN}..{HDR_MAX} nits), keeping {}",
                    cfg.hdr_brightness
                )),
            },
            "colorspace" => match ColorSpace::parse(v) {
                Some(cs) => cfg.color_space = cs,
                None => warnings.push(format!(
                    "ColorSpace: unknown value {v:?} (auto, sdr, hdr10, scrgb), keeping {}",
                    cfg.color_space.as_str()
                )),
            },
            "keymenu" => match ini::vk_from_name(v) {
                Some(vk) => cfg.key_menu = vk,
                None => warnings
                    .push(format!("KeyMenu: unknown key name {v:?}, keeping 0x{:02X}", cfg.key_menu)),
            },
            _ => warnings.push(format!("unknown key {k:?}")),
        }
    }
    (cfg, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert!(c.enabled);
        assert!(!c.debug);
        assert!(!c.show_on_start);
        assert_eq!(c.scale, 0.0, "auto");
        assert_eq!(c.key_menu, 0x2D, "Insert");
        assert_eq!(ini::vk_from_name("Insert"), Some(DEFAULT_KEY_MENU));
        assert_eq!(c.hdr_brightness, 203.0);
        assert_eq!(c.color_space, ColorSpace::Auto);
    }

    #[test]
    fn parses_every_key() {
        let (c, w) = parse(
            "; c\n[DesertOverlay]\nEnabled=0\nDebug=1\nKeyMenu=F4\nShowOnStart=yes\nScale=1.5\n\
             HdrBrightness=400\nColorSpace=hdr10\n",
        );
        assert!(!c.enabled);
        assert!(c.debug);
        assert!(c.show_on_start);
        assert_eq!(c.scale, 1.5);
        assert_eq!(c.key_menu, 0x73);
        assert_eq!(c.hdr_brightness, 400.0);
        assert_eq!(c.color_space, ColorSpace::Hdr10);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn hdr_brightness_out_of_range_is_refused_with_a_warning() {
        for bad in ["0", "79", "1001", "nope", ""] {
            let (c, w) = parse(&format!("HdrBrightness={bad}\n"));
            assert_eq!(c.hdr_brightness, DEFAULT_HDR_BRIGHTNESS, "{bad}");
            assert_eq!(w.len(), 1, "{bad}: {w:?}");
            assert!(w[0].starts_with("HdrBrightness: bad value"), "{w:?}");
        }
        for good in ["80", "203", "1000", " 250 "] {
            let (_, w) = parse(&format!("HdrBrightness={good}\n"));
            assert!(w.is_empty(), "{good}: {w:?}");
        }
    }

    #[test]
    fn color_space_takes_the_four_names_in_any_case() {
        for (text, want) in [
            ("auto", ColorSpace::Auto),
            ("SDR", ColorSpace::Sdr),
            ("Hdr10", ColorSpace::Hdr10),
            (" scrgb ", ColorSpace::ScRgb),
        ] {
            let (c, w) = parse(&format!("ColorSpace={text}\n"));
            assert_eq!(c.color_space, want, "{text}");
            assert!(w.is_empty(), "{text}: {w:?}");
            assert_eq!(ColorSpace::parse(want.as_str()), Some(want));
        }
    }

    #[test]
    fn an_unknown_color_space_is_auto_with_a_warning() {
        let (c, w) = parse("ColorSpace=hdr\n");
        assert_eq!(c.color_space, ColorSpace::Auto);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].starts_with("ColorSpace: unknown value"), "{w:?}");
    }

    #[test]
    fn scale_out_of_range_is_refused_with_a_warning() {
        let (c, w) = parse("Scale=9\n");
        assert_eq!(c.scale, 0.0);
        assert_eq!(w.len(), 1, "{w:?}");
        let (c, w) = parse("Scale=0\n");
        assert_eq!(c.scale, 0.0);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn case_insensitive_keys() {
        let (c, w) = parse("ENABLED=0\nkeymenu=home\n");
        assert!(!c.enabled);
        assert_eq!(c.key_menu, 0x24);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn bad_values_keep_the_default_and_warn() {
        let (c, w) = parse("KeyMenu=nope\nJunk=1\nnoequals\n");
        assert_eq!(c.key_menu, DEFAULT_KEY_MENU);
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w[0].starts_with("KeyMenu: unknown key name"), "{w:?}");
    }

    #[test]
    fn empty_text_is_the_default() {
        let (c, w) = parse("");
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
    }

    #[test]
    fn the_shipped_template_parses_clean_and_is_the_default() {
        let (c, w) = parse(include_str!("../DesertOverlay.ini"));
        assert!(w.is_empty(), "the shipped DesertOverlay.ini has a bad line: {w:?}");
        assert_eq!(c, Config::default(), "the shipped ini must state the defaults");
    }
}
