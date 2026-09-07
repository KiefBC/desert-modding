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
}

/// Smallest and largest fixed `Scale` accepted from the ini.
pub const SCALE_MIN: f32 = 0.5;
pub const SCALE_MAX: f32 = 4.0;

impl Default for Config {
    fn default() -> Self {
        Config { enabled: true, debug: false, key_menu: DEFAULT_KEY_MENU, show_on_start: false, scale: 0.0 }
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
    }

    #[test]
    fn parses_every_key() {
        let (c, w) = parse("; c\n[DesertOverlay]\nEnabled=0\nDebug=1\nKeyMenu=F4\nShowOnStart=yes\nScale=1.5\n");
        assert!(!c.enabled);
        assert!(c.debug);
        assert!(c.show_on_start);
        assert_eq!(c.scale, 1.5);
        assert_eq!(c.key_menu, 0x73);
        assert!(w.is_empty(), "{w:?}");
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
