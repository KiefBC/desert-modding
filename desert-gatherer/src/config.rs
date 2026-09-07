//! `DesertGatherer.ini` beside the game exe. Missing file or key => defaults.
//!
//! Only Desert Gatherer's own keys live here; the ini tokeniser and the truthy
//! spellings are shared in `desert_core::ini`. Same shape as
//! `desert_looter::config`: `parse` returns the config plus ready-to-log
//! warnings, and a bad value never replaces the default.
//!
//! The four multiplier keys are the four independent gather families of
//! `desert_core::collect::Family`, and they carry the vocabulary the DMM pack
//! used (`dmm-pack/README.md`): Foraging, Logging, Mining and Ore Nodes are
//! separate internal families, and setting one does not touch the others.

use desert_core::collect::Family;
use desert_core::ini::{self, Line};

/// Multipliers below this are meaningless (0 would zero out every yield) and
/// above it are almost certainly a typo, so both are refused.
pub const MULT_MIN: u32 = 1;
pub const MULT_MAX: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Master switch. 0 = load, log, and install no hook at all.
    pub enabled: bool,
    /// 1 = the hook logs the edits it would make and writes nothing.
    pub dry_run: bool,
    /// 1 = also log every record the loader hands us that is not a gather
    /// record (13,600 of them), capped by the hook so the log stays finite.
    pub debug: bool,
    /// Yield multiplier for `Family::Foraging` (plants, fruit, mushrooms).
    pub foraging: u32,
    /// Yield multiplier for `Family::Logging` (`firewood_*`).
    pub logging: u32,
    /// Yield multiplier for `Family::Mining` (`collect_mine`: rocks, veins,
    /// breakable stalactites).
    pub mining: u32,
    /// Yield multiplier for `Family::Ore` (`collect_ore`: `ore_*` deposits,
    /// sulfur stone, collectible stalactites). Separate from Mining.
    pub ore: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            dry_run: false,
            debug: false,
            foraging: 1,
            logging: 1,
            mining: 1,
            ore: 1,
        }
    }
}

impl Config {
    /// The configured multiplier for a gather family. `1` means "leave the
    /// record alone", and the hook treats it as a reason to bail early.
    pub fn multiplier(&self, family: Family) -> u32 {
        match family {
            Family::Foraging => self.foraging,
            Family::Logging => self.logging,
            Family::Mining => self.mining,
            Family::Ore => self.ore,
        }
    }

    /// True if no family is multiplied, i.e. the hook would never write.
    pub fn all_vanilla(&self) -> bool {
        self.foraging <= 1 && self.logging <= 1 && self.mining <= 1 && self.ore <= 1
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
            "dryrun" => cfg.dry_run = ini::parse_bool(v),
            "debug" => cfg.debug = ini::parse_bool(v),
            "foraging" | "logging" | "mining" | "ore" => {
                let slot: &mut u32 = match k.to_ascii_lowercase().as_str() {
                    "foraging" => &mut cfg.foraging,
                    "logging" => &mut cfg.logging,
                    "mining" => &mut cfg.mining,
                    _ => &mut cfg.ore,
                };
                match v.parse::<u32>() {
                    Ok(n) if (MULT_MIN..=MULT_MAX).contains(&n) => *slot = n,
                    _ => warnings.push(format!(
                        "{k}: bad value {v:?} ({MULT_MIN}..{MULT_MAX}), keeping {slot}"
                    )),
                }
            }
            _ => warnings.push(format!("unknown key {k:?}")),
        }
    }
    (cfg, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_vanilla() {
        let c = Config::default();
        assert!(c.enabled);
        assert!(!c.dry_run);
        assert!(!c.debug);
        assert!(c.all_vanilla());
        for f in [Family::Foraging, Family::Logging, Family::Mining, Family::Ore] {
            assert_eq!(c.multiplier(f), 1);
        }
    }

    #[test]
    fn parses_every_key() {
        let (c, w) = parse(
            "; comment\n[DesertGatherer]\nEnabled=1\nDryRun=yes\nDebug=on\n\
             Foraging=10\nLogging=2\nMining=5\nOre=100\n",
        );
        assert!(c.enabled);
        assert!(c.dry_run);
        assert!(c.debug);
        assert_eq!(c.multiplier(Family::Foraging), 10);
        assert_eq!(c.multiplier(Family::Logging), 2);
        assert_eq!(c.multiplier(Family::Mining), 5);
        assert_eq!(c.multiplier(Family::Ore), 100);
        assert!(!c.all_vanilla());
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn case_insensitive_keys() {
        let (c, w) = parse("ENABLED=0\nforaging=3\nOrE=4\n");
        assert!(!c.enabled);
        assert_eq!(c.foraging, 3);
        assert_eq!(c.ore, 4);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn bad_values_keep_the_default_and_warn() {
        let d = Config::default();
        let (c, w) = parse("Foraging=0\nLogging=101\nMining=abc\nOre=-2\nJunk=1\nnoequals\n");
        assert_eq!(c.foraging, d.foraging);
        assert_eq!(c.logging, d.logging);
        assert_eq!(c.mining, d.mining);
        assert_eq!(c.ore, d.ore);
        assert!(c.all_vanilla());
        // four bad multipliers, one unknown key, one syntax error
        assert_eq!(w.len(), 6, "{w:?}");
        assert!(w[0].starts_with("Foraging: bad value"), "{w:?}");
    }

    #[test]
    fn range_edges() {
        let (c, w) = parse("Foraging=1\nLogging=100\n");
        assert_eq!(c.foraging, MULT_MIN);
        assert_eq!(c.logging, MULT_MAX);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn empty_text_is_the_default() {
        let (c, w) = parse("");
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
    }
}
