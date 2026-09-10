//! The `[Overlay]` section of `DesertTooling.ini`. Missing file, missing
//! section or missing key => defaults.
//!
//! Same shape as `desert_looter::config` and `desert_gatherer::config`:
//! [`parse`] returns the config plus ready-to-log warnings, a bad value never
//! replaces the default, the tokeniser and the virtual-key name table come from
//! `desert_core::ini`, and [`schema`] declares the same keys for the menu -
//! which is how the overlay's own settings get a section of the menu beside the
//! two subsystems it draws.
//!
//! Every subsystem shares one file, and `Enabled` and `Debug` exist under all
//! of their headers, so reading is scoped to [`INI_SECTION`] through
//! [`ini::lines_in_section`].
//!
//! These settings are read **once**, at startup: changing the menu key, the
//! font or the master switch from inside the menu it draws makes no sense, and
//! the graphics hook is installed long before a later edit could be noticed.

use desert_core::ini::{self, Line};
use desert_core::schema::{Field, Kind, Section};

/// The one ini every subsystem shares, beside the game exe.
pub const INI_NAME: &str = "DesertTooling.ini";

/// The `[Header]` inside it that these settings live under.
pub const INI_SECTION: &str = "Overlay";

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
    /// Menu font height in pixels at [`Config::scale`] `1.0`, in
    /// `FONT_SIZE_MIN..=FONT_SIZE_MAX`. What actually reaches the rasteriser
    /// is `font_size * scale`.
    pub font_size: f32,
    /// The `Font` value exactly as the ini spelled it. [`Config::font_choice`]
    /// turns it into a [`FontChoice`]; empty means imgui's built-in font.
    pub font: String,
    /// Paper white in nits: how bright plain white is drawn on an HDR
    /// swapchain. Ignored entirely in SDR. In `HDR_MIN..=HDR_MAX`.
    pub hdr_brightness: f32,
    /// Which colour space the menu's pixels are encoded for.
    /// [`ColorSpace::Auto`] follows what the swapchain says.
    pub color_space: ColorSpace,
    /// The colour theme, by its `name` in [`crate::themes::ALL`]. Always a
    /// name that exists: an unknown one is replaced by the default with a
    /// warning at parse time.
    pub theme: &'static crate::theme::Theme,
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

/// Smallest and largest `FontSize` accepted from the ini, in pixels. Under 8
/// nothing is legible at any scale; over 72 a single line of the menu is
/// taller than a 4K screen can usefully spare.
pub const FONT_SIZE_MIN: f32 = 8.0;
pub const FONT_SIZE_MAX: f32 = 72.0;

/// `FontSize` when the ini names none. imgui's own font is 13 px and was the
/// whole reason the menu was hard to read on a 4K display.
pub const DEFAULT_FONT_SIZE: f32 = 20.0;

/// `Font` when the ini names none: Segoe UI, which every supported Windows
/// ships in its Fonts directory.
pub const DEFAULT_FONT: &str = "segoeui.ttf";

/// What a `Font` value asks for, once the optional `:N` face suffix has been
/// split off.
///
/// Only string work happens here so it is unit-tested natively on Linux; the
/// Windows half of the overlay is the only thing that looks a name up in the
/// Fonts directory or opens a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontChoice {
    /// An empty `Font` value: draw with imgui's built-in bitmap font.
    BuiltIn,
    /// A bare file name (`segoeui.ttf`), to be resolved against the Windows
    /// Fonts directory.
    Name {
        /// The file name, with no directory part and no `:N` suffix.
        file: String,
        /// Face index inside a `.ttc` collection; 0 for a plain `.ttf`.
        face: u32,
    },
    /// A value carrying a directory separator (`C:\fonts\mine.ttf`), opened as
    /// it stands.
    Path {
        /// The path, with the `:N` suffix removed.
        path: String,
        /// Face index inside a `.ttc` collection.
        face: u32,
    },
}

impl FontChoice {
    /// Split a `Font` value into a target and a face index.
    ///
    /// The `:N` suffix is only taken as a face index when everything after the
    /// **last** colon is decimal digits, which is what keeps a drive letter in
    /// `C:\Windows\Fonts\cambria.ttc` from being mistaken for one.
    pub fn parse(value: &str) -> FontChoice {
        let value = value.trim();
        let (target, face) = split_face(value);
        let target = target.trim();
        if target.is_empty() {
            return FontChoice::BuiltIn;
        }
        if target.contains('\\') || target.contains('/') {
            FontChoice::Path { path: target.to_string(), face }
        } else {
            FontChoice::Name { file: target.to_string(), face }
        }
    }

    /// The face index this choice selects, 0 for the built-in font.
    pub fn face(&self) -> u32 {
        match self {
            FontChoice::BuiltIn => 0,
            FontChoice::Name { face, .. } | FontChoice::Path { face, .. } => *face,
        }
    }
}

/// `("cambria.ttc:1")` => `("cambria.ttc", 1)`, `("C:\\f.ttf")` =>
/// `("C:\\f.ttf", 0)`.
fn split_face(value: &str) -> (&str, u32) {
    let Some((head, tail)) = value.rsplit_once(':') else {
        return (value, 0);
    };
    if head.is_empty() || tail.is_empty() || !tail.bytes().all(|b| b.is_ascii_digit()) {
        return (value, 0);
    }
    match tail.parse::<u32>() {
        Ok(n) => (head, n),
        // Digits that do not fit a u32 are not a face index anybody meant.
        Err(_) => (value, 0),
    }
}

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
            font_size: DEFAULT_FONT_SIZE,
            font: DEFAULT_FONT.to_string(),
            hdr_brightness: DEFAULT_HDR_BRIGHTNESS,
            color_space: ColorSpace::Auto,
            theme: default_theme(),
        }
    }
}

impl Config {
    /// The `Font` value as a resolved intent. Cheap, and called once.
    pub fn font_choice(&self) -> FontChoice {
        FontChoice::parse(&self.font)
    }
}

/// The theme the ini gets when it names none.
fn default_theme() -> &'static crate::theme::Theme {
    // DEFAULT is checked against ALL by the theme tests, and ALL is never
    // empty, so the fallbacks here can only fire on a broken build.
    crate::theme::Theme::by_name(crate::themes::DEFAULT)
        .or_else(|| crate::themes::ALL.first().copied())
        .unwrap_or(&crate::themes::classic::THEME)
}

/// Parse ini text. Unknown keys and bad values are reported back so they can
/// be logged; the config always comes back usable.
pub fn parse(text: &str) -> (Config, Vec<String>) {
    let mut cfg = Config::default();
    let mut warnings = Vec::new();
    // Only this subsystem's own section: `Enabled` under `[Looter]` is not
    // ours, and a key we do not know about there is not our unknown key either.
    for line in ini::lines_in_section(text, INI_SECTION) {
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
            "fontsize" => match v.trim().parse::<f32>() {
                Ok(f) if (FONT_SIZE_MIN..=FONT_SIZE_MAX).contains(&f) => cfg.font_size = f,
                _ => warnings.push(format!(
                    "FontSize: bad value {v:?} ({FONT_SIZE_MIN}..{FONT_SIZE_MAX} px), keeping {}",
                    cfg.font_size
                )),
            },
            // Every value is legal here: an empty one asks for the built-in
            // font and anything else is a file name or a path, which only the
            // Windows side can succeed or fail at opening. It says so in the
            // log when it fails, and falls back to the built-in font.
            "font" => cfg.font = v.trim().to_string(),
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
            "theme" => match crate::theme::Theme::by_name(v) {
                Some(t) => cfg.theme = t,
                None => warnings.push(format!(
                    "Theme: unknown value {v:?} ({}), keeping {}",
                    crate::themes::ALL.iter().map(|t| t.name).collect::<Vec<_>>().join(", "),
                    cfg.theme.name
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


// ---------------------------------------------------------------------------
// The menu's description of these same keys
// ---------------------------------------------------------------------------

/// One field, with its help text; nothing here has a heading or shares a row.
fn f(key: &str, label: &str, kind: Kind, help: &str) -> Field {
    Field {
        key: key.to_string(),
        label: label.to_string(),
        kind,
        heading: None,
        same_line: false,
        help: Some(help.to_string()),
    }
}

/// The same field, drawn on the row above's line.
fn beside(mut field: Field) -> Field {
    field.same_line = true;
    field
}

/// The same field, under a dim heading.
fn under(heading: &str, mut field: Field) -> Field {
    field.heading = Some(heading.to_string());
    field
}

/// The overlay's own settings, for the overlay's own menu section.
///
/// `desert-tooling` collects this alongside the looter's and the gatherer's and
/// hands all three to [`crate::start`], so the menu that edits the other two
/// subsystems edits itself as well. Every key here is read **once**, at
/// startup, so a change made in the menu is on disk immediately and on screen
/// at the next launch - which the section's `notice` says out loud.
///
/// `Font` is deliberately **not** here. Its value is a file name, an absolute
/// path or an empty string, optionally suffixed `:N` for a `.ttc` face, and
/// [`Kind`] has no way to say that; a combo of guessed file names would show
/// the wrong value for a hand-written path and overwrite it on the next edit.
/// It stays a hand-edited key that [`parse`] still reads and that the overlay's
/// rewriter leaves alone.
pub fn schema() -> Section {
    let d = Config::default();
    Section {
        title: "Desert Overlay".to_string(),
        ini: INI_NAME.to_string(),
        ini_section: INI_SECTION.to_string(),
        // One .asi holds every subsystem now, so there is no module whose
        // absence could grey this section out.
        module: None,
        // Last: this is the menu's own settings, below the two mods the menu
        // exists to configure.
        order: 30,
        notice: Some(
            "These are read once, at startup: a change here is saved now and takes effect at the \
             next launch."
                .to_string(),
        ),
        presets_label: None,
        presets: Vec::new(),
        fields: vec![
            f(
                "Enabled",
                "Enabled",
                Kind::Bool { default: d.enabled },
                "Master switch. 0 = the overlay installs no graphics hook at all and the game \
                 renders exactly as it would without it. Use this rather than deleting anything \
                 while chasing a crash.",
            ),
            beside(f(
                "ShowOnStart",
                "Open at startup",
                Kind::Bool { default: d.show_on_start },
                "1 = the menu is already open at the first frame instead of waiting for the key.",
            )),
            f(
                "KeyMenu",
                "Menu key",
                Kind::Key { default: key_name(d.key_menu) },
                "Shows and hides the menu. Avoid keys the game uses, and the ones Desert Looter \
                 already has.",
            ),
            under(
                "Size and font:",
                f(
                    "Scale",
                    "Scale",
                    Kind::Float {
                        default: d.scale,
                        // 0 is "follow the Windows display scaling", so the
                        // slider has to reach it. Between 0 and SCALE_MIN the
                        // value is not accepted and automatic is what you get,
                        // which is what the help text says.
                        min: 0.0,
                        max: SCALE_MAX,
                        format: Some("%.2f".to_string()),
                    },
                    "Size of the menu: fonts, spacing and the window. 0 follows the Windows \
                     display scaling (125% gives 1.25); anything below 0.5 is treated as 0.",
                ),
            ),
            f(
                "FontSize",
                "Font size (px)",
                Kind::Float {
                    default: d.font_size,
                    min: FONT_SIZE_MIN,
                    max: FONT_SIZE_MAX,
                    format: Some("%.0f px".to_string()),
                },
                "Height of the menu's text in pixels before Scale is applied. The font is \
                 rasterised at that size rather than blown up, so raising this makes the text \
                 sharper, not blockier.",
            ),
            under(
                "Colour:",
                f(
                    "Theme",
                    "Theme",
                    Kind::Choice { default: d.theme.name.to_string(), options: theme_names() },
                    "The menu's colour theme. The picker at the top of the window changes it \
                     live; this key is what makes the choice stick.",
                ),
            ),
            f(
                "ColorSpace",
                "Colour space",
                Kind::Choice {
                    default: d.color_space.as_str().to_string(),
                    options: COLOR_SPACES.iter().map(|s| (*s).to_string()).collect(),
                },
                "What the menu's pixels are encoded for. Leave it on auto; the forced values are \
                 for when the swapchain's own answer is wrong and the menu comes out washed out \
                 or oversaturated.",
            ),
            f(
                "HdrBrightness",
                "HDR paper white (nits)",
                Kind::Float {
                    default: d.hdr_brightness,
                    min: HDR_MIN,
                    max: HDR_MAX,
                    format: Some("%.0f nits".to_string()),
                },
                "How bright the menu's white is on an HDR display. 203 is the broadcast \
                 reference. Ignored entirely on an SDR display.",
            ),
            under(
                "Diagnostics:",
                f(
                    "Debug",
                    "Debug",
                    Kind::Bool { default: d.debug },
                    "Verbose log: every ini write and reload, plus hudhook's own per-frame debug \
                     and trace lines. That is a lot of writing, and it is what a \"the game will \
                     not start\" or \"the menu never appears\" report needs.",
                ),
            ),
        ],
    }
}

/// The `ColorSpace` spellings, in the order the combo lists them. The same
/// four [`ColorSpace::parse`] accepts.
const COLOR_SPACES: [&str; 4] = ["auto", "sdr", "hdr10", "scrgb"];

/// Every theme's ini name, for the picker.
fn theme_names() -> Vec<String> {
    crate::themes::ALL.iter().map(|t| t.name.to_string()).collect()
}

/// The key name a virtual-key code is written as, which is what the schema's
/// default has to be. A code with no name cannot happen for a compiled-in
/// default, and `INSERT` is a safe answer if it ever did.
fn key_name(vk: u16) -> String {
    ini::key_names()
        .iter()
        .find(|n| ini::vk_from_name(n) == Some(vk))
        .map_or_else(|| "INSERT".to_string(), |n| (*n).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse `body` as the `[Overlay]` section of the shared ini, which is the
    /// only place `parse` reads.
    fn in_section(body: &str) -> (Config, Vec<String>) {
        parse(&format!("[Overlay]\n{body}"))
    }

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
        assert_eq!(c.font_size, 20.0);
        assert_eq!(c.font, "segoeui.ttf");
        assert_eq!(c.font_choice(), FontChoice::Name { file: "segoeui.ttf".into(), face: 0 });
    }

    #[test]
    fn parses_every_key() {
        let (c, w) = parse(
            "; c\n[Overlay]\nEnabled=0\nDebug=1\nKeyMenu=F4\nShowOnStart=yes\nScale=1.5\n\
             FontSize=28\nFont=georgia.ttf\nHdrBrightness=400\nColorSpace=hdr10\n",
        );
        assert!(!c.enabled);
        assert!(c.debug);
        assert!(c.show_on_start);
        assert_eq!(c.scale, 1.5);
        assert_eq!(c.key_menu, 0x73);
        assert_eq!(c.hdr_brightness, 400.0);
        assert_eq!(c.color_space, ColorSpace::Hdr10);
        assert_eq!(c.font_size, 28.0);
        assert_eq!(c.font, "georgia.ttf");
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn font_size_out_of_range_is_refused_with_a_warning() {
        for bad in ["0", "7.9", "73", "-20", "nope", ""] {
            let (c, w) = in_section(&format!("FontSize={bad}\n"));
            assert_eq!(c.font_size, DEFAULT_FONT_SIZE, "{bad}");
            assert_eq!(w.len(), 1, "{bad}: {w:?}");
            assert!(w[0].starts_with("FontSize: bad value"), "{w:?}");
        }
        for good in ["8", "20", "72", " 24.5 "] {
            let (_, w) = in_section(&format!("FontSize={good}\n"));
            assert!(w.is_empty(), "{good}: {w:?}");
        }
    }

    #[test]
    fn font_takes_a_name_a_path_or_nothing() {
        for (text, want) in [
            ("segoeui.ttf", FontChoice::Name { file: "segoeui.ttf".into(), face: 0 }),
            ("  georgia.ttf  ", FontChoice::Name { file: "georgia.ttf".into(), face: 0 }),
            ("", FontChoice::BuiltIn),
            ("   ", FontChoice::BuiltIn),
            (
                "C:\\Windows\\Fonts\\constan.ttf",
                FontChoice::Path { path: "C:\\Windows\\Fonts\\constan.ttf".into(), face: 0 },
            ),
            ("/usr/share/fonts/x.ttf", FontChoice::Path { path: "/usr/share/fonts/x.ttf".into(), face: 0 }),
        ] {
            let (c, w) = in_section(&format!("Font={text}\n"));
            assert_eq!(c.font_choice(), want, "{text:?}");
            assert!(w.is_empty(), "{text:?}: {w:?}");
        }
    }

    #[test]
    fn a_trailing_colon_number_is_a_ttc_face_index() {
        assert_eq!(
            FontChoice::parse("cambria.ttc:0"),
            FontChoice::Name { file: "cambria.ttc".into(), face: 0 }
        );
        assert_eq!(
            FontChoice::parse("cambria.ttc:1"),
            FontChoice::Name { file: "cambria.ttc".into(), face: 1 }
        );
        assert_eq!(FontChoice::parse("cambria.ttc:1").face(), 1);
        // A drive letter is not a face index, and neither is a non-numeric
        // suffix: both stay part of the name.
        assert_eq!(
            FontChoice::parse("C:\\Windows\\Fonts\\cambria.ttc"),
            FontChoice::Path { path: "C:\\Windows\\Fonts\\cambria.ttc".into(), face: 0 }
        );
        assert_eq!(
            FontChoice::parse("C:\\Windows\\Fonts\\cambria.ttc:2"),
            FontChoice::Path { path: "C:\\Windows\\Fonts\\cambria.ttc".into(), face: 2 }
        );
        assert_eq!(
            FontChoice::parse("weird:name.ttf"),
            FontChoice::Name { file: "weird:name.ttf".into(), face: 0 }
        );
        assert_eq!(
            FontChoice::parse("huge.ttc:99999999999999999999"),
            FontChoice::Name { file: "huge.ttc:99999999999999999999".into(), face: 0 }
        );
        assert_eq!(FontChoice::parse(":3"), FontChoice::Name { file: ":3".into(), face: 0 });
        assert_eq!(FontChoice::BuiltIn.face(), 0);
    }

    #[test]
    fn hdr_brightness_out_of_range_is_refused_with_a_warning() {
        for bad in ["0", "79", "1001", "nope", ""] {
            let (c, w) = in_section(&format!("HdrBrightness={bad}\n"));
            assert_eq!(c.hdr_brightness, DEFAULT_HDR_BRIGHTNESS, "{bad}");
            assert_eq!(w.len(), 1, "{bad}: {w:?}");
            assert!(w[0].starts_with("HdrBrightness: bad value"), "{w:?}");
        }
        for good in ["80", "203", "1000", " 250 "] {
            let (_, w) = in_section(&format!("HdrBrightness={good}\n"));
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
            let (c, w) = in_section(&format!("ColorSpace={text}\n"));
            assert_eq!(c.color_space, want, "{text}");
            assert!(w.is_empty(), "{text}: {w:?}");
            assert_eq!(ColorSpace::parse(want.as_str()), Some(want));
        }
    }

    #[test]
    fn an_unknown_color_space_is_auto_with_a_warning() {
        let (c, w) = in_section("ColorSpace=hdr\n");
        assert_eq!(c.color_space, ColorSpace::Auto);
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].starts_with("ColorSpace: unknown value"), "{w:?}");
    }

    #[test]
    fn scale_out_of_range_is_refused_with_a_warning() {
        let (c, w) = in_section("Scale=9\n");
        assert_eq!(c.scale, 0.0);
        assert_eq!(w.len(), 1, "{w:?}");
        let (c, w) = in_section("Scale=0\n");
        assert_eq!(c.scale, 0.0);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn case_insensitive_keys() {
        let (c, w) = in_section("ENABLED=0\nkeymenu=home\n");
        assert!(!c.enabled);
        assert_eq!(c.key_menu, 0x24);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn bad_values_keep_the_default_and_warn() {
        let (c, w) = in_section("KeyMenu=nope\nJunk=1\nnoequals\n");
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
    fn only_this_subsystems_section_is_read() {
        // The hazard the section scoping exists for: `Enabled` and `Debug`
        // live under all three headers of the one file.
        let (c, w) = parse(
            "Enabled=0\nDebug=1\n\n[Looter]\nEnabled=0\nDebug=1\nBagTab=1\n\n\
             [Gatherer]\nEnabled=0\nDebug=1\n\n[Overlay]\nEnabled=1\nDebug=0\nFontSize=28\n",
        );
        assert!(c.enabled, "the looter's Enabled=0 is not ours");
        assert!(!c.debug, "nor its Debug=1");
        assert_eq!(c.font_size, 28.0);
        assert!(w.is_empty(), "another section's keys are not our unknown keys: {w:?}");
    }

    #[test]
    fn a_file_without_our_section_is_the_default() {
        let (c, w) = parse("[Looter]\nEnabled=0\nScanRange=40\n");
        assert_eq!(c, Config::default());
        assert!(w.is_empty(), "{w:?}");
    }

    // -- the schema the menu draws these same keys from -------------------

    #[test]
    fn the_schema_names_the_shared_ini_and_this_sections_header() {
        let s = schema();
        assert_eq!(s.ini, "DesertTooling.ini");
        assert_eq!(s.ini_section, "Overlay");
        assert_eq!(s.module, None, "one .asi: there is no module to be missing");
        assert!(s.field("Font").is_none(), "Font is free text; no Kind can express it");
    }

    #[test]
    fn every_schema_field_is_a_key_this_parser_knows() {
        for field in &schema().fields {
            let (_, w) = in_section(&format!("{}={}\n", field.key, field.kind.default_text()));
            assert!(w.is_empty(), "{}: {w:?}", field.key);
        }
    }

    #[test]
    fn the_defaults_the_schema_seeds_are_the_defaults_this_parser_reads() {
        // The one agreement that matters: what `desert-tooling` writes into a
        // missing ini has to parse back to exactly `Config::default()`.
        let text = desert_core::schema::render_ini_defaults(&schema(), "Desert Tooling");
        assert!(text.contains("[Overlay]"), "{text}");
        let (c, w) = parse(&text);
        assert!(w.is_empty(), "the seeded ini has a line this parser rejects: {w:?}");
        assert_eq!(c, Config::default(), "the seeded ini must state the defaults");
    }

    #[test]
    fn every_schema_default_is_a_value_its_own_field_accepts() {
        for field in &schema().fields {
            let text = field.kind.default_text();
            assert_eq!(
                field.kind.normalize(&text).as_deref(),
                Some(text.as_str()),
                "{} default {text:?}",
                field.key
            );
        }
    }
}
