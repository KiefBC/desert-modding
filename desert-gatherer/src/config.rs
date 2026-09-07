//! `DesertGatherer.ini` beside the game exe. Missing file or key => defaults.
//!
//! Only Desert Gatherer's own keys live here; the ini tokeniser and the truthy
//! spellings are shared in `desert_core::ini`. Same shape as
//! `desert_looter::config`: `parse` returns the config plus ready-to-log
//! warnings, and a bad value never replaces the default.
//!
//! The four multiplier keys are the four independent gather families of
//! `desert_core::collect::Family`, and they carry the vocabulary the DMM pack
//! used (`desert-gatherer-dmm/README.md`): Foraging, Logging, Mining and Ore
//! Nodes are separate internal families, and setting one does not touch the
//! others.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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

/// Lock-free mirror of [`Config`] the hook reads on every record load.
///
/// The overlay plugin edits `DesertGatherer.ini` beside the game exe while it
/// runs; the plugin's main thread notices (see `lib.rs`'s reload loop),
/// re-parses with [`parse`] and calls [`LiveConfig::publish`]. The hook itself
/// never touches a `Mutex` or does file I/O - it only ever loads these
/// atomics, so a config change is visible to the very next record the loader
/// hands us, with no allocation and nothing that can block a game thread.
///
/// `Ordering::Relaxed` throughout: nothing here synchronises with any other
/// memory access, so there is no ordering to preserve, only the eventual
/// visibility of a new value. A reader that sees, say, a new multiplier
/// alongside a still-old `Enabled` for one call is at most one record behind;
/// it settles on the next.
pub struct LiveConfig {
    enabled: AtomicBool,
    dry_run: AtomicBool,
    debug: AtomicBool,
    foraging: AtomicU32,
    logging: AtomicU32,
    mining: AtomicU32,
    ore: AtomicU32,
}

impl LiveConfig {
    /// Starts equal to `Config::default()` so a hook installed before the
    /// first `publish` behaves exactly like the pre-live-reload plugin.
    pub const fn new() -> Self {
        LiveConfig {
            enabled: AtomicBool::new(true),
            dry_run: AtomicBool::new(false),
            debug: AtomicBool::new(false),
            foraging: AtomicU32::new(1),
            logging: AtomicU32::new(1),
            mining: AtomicU32::new(1),
            ore: AtomicU32::new(1),
        }
    }

    /// Publish a freshly parsed config for the hook to pick up. Called from
    /// the plugin's main thread only (startup, and once per detected ini
    /// change); the hook only ever reads.
    pub fn publish(&self, cfg: &Config) {
        self.enabled.store(cfg.enabled, Ordering::Relaxed);
        self.dry_run.store(cfg.dry_run, Ordering::Relaxed);
        self.debug.store(cfg.debug, Ordering::Relaxed);
        self.foraging.store(cfg.foraging, Ordering::Relaxed);
        self.logging.store(cfg.logging, Ordering::Relaxed);
        self.mining.store(cfg.mining, Ordering::Relaxed);
        self.ore.store(cfg.ore, Ordering::Relaxed);
    }

    /// Snapshot the live values as a plain `Config`, e.g. for a log line.
    pub fn load(&self) -> Config {
        Config {
            enabled: self.enabled.load(Ordering::Relaxed),
            dry_run: self.dry_run.load(Ordering::Relaxed),
            debug: self.debug.load(Ordering::Relaxed),
            foraging: self.foraging.load(Ordering::Relaxed),
            logging: self.logging.load(Ordering::Relaxed),
            mining: self.mining.load(Ordering::Relaxed),
            ore: self.ore.load(Ordering::Relaxed),
        }
    }

    /// Master switch. `false` means the hook must read and write nothing.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// `true` = the hook logs the edits it would make and writes nothing.
    pub fn dry_run(&self) -> bool {
        self.dry_run.load(Ordering::Relaxed)
    }

    /// `true` = also log non-gather records, capped by the hook.
    pub fn debug(&self) -> bool {
        self.debug.load(Ordering::Relaxed)
    }

    /// The live multiplier for a gather family. Safe to call from the hook on
    /// every record: one atomic load, no allocation, no lock.
    pub fn multiplier(&self, family: Family) -> u32 {
        match family {
            Family::Foraging => self.foraging.load(Ordering::Relaxed),
            Family::Logging => self.logging.load(Ordering::Relaxed),
            Family::Mining => self.mining.load(Ordering::Relaxed),
            Family::Ore => self.ore.load(Ordering::Relaxed),
        }
    }
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// The live, hot-reloadable config the hook reads. Published by the plugin's
/// main thread (startup, and its ~1s ini-reload loop); read only by the hook.
pub static LIVE: LiveConfig = LiveConfig::new();

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

    #[test]
    fn live_config_starts_equal_to_default() {
        let live = LiveConfig::new();
        assert_eq!(live.load(), Config::default());
        assert!(live.enabled());
        assert!(!live.dry_run());
        assert!(!live.debug());
        for f in [Family::Foraging, Family::Logging, Family::Mining, Family::Ore] {
            assert_eq!(live.multiplier(f), 1);
        }
    }

    #[test]
    fn live_config_publish_round_trips() {
        let live = LiveConfig::new();
        let (cfg, w) = parse(
            "Enabled=0\nDryRun=1\nDebug=1\nForaging=10\nLogging=2\nMining=5\nOre=100\n",
        );
        assert!(w.is_empty(), "{w:?}");
        live.publish(&cfg);
        assert_eq!(live.load(), cfg);
        assert!(!live.enabled());
        assert!(live.dry_run());
        assert!(live.debug());
        assert_eq!(live.multiplier(Family::Foraging), 10);
        assert_eq!(live.multiplier(Family::Logging), 2);
        assert_eq!(live.multiplier(Family::Mining), 5);
        assert_eq!(live.multiplier(Family::Ore), 100);
    }

    #[test]
    fn live_config_publish_overwrites_previous_values() {
        let live = LiveConfig::new();
        live.publish(&Config { enabled: false, dry_run: true, debug: true, foraging: 50, ..Config::default() });
        assert!(!live.enabled());
        assert_eq!(live.multiplier(Family::Foraging), 50);

        // A later publish fully replaces the previous snapshot, which is what
        // the reload loop relies on: every field in the new ini wins, not
        // just the ones that changed.
        live.publish(&Config::default());
        assert_eq!(live.load(), Config::default());
    }
}
