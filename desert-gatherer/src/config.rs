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
use desert_core::schema::{Field, Kind, Section};

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

// ---------------------------------------------------------------------------
// The menu schema
// ---------------------------------------------------------------------------

/// The `.asi` the overlay looks for before it lets this section be edited.
const MODULE: &str = "DesertGatherer.asi";

/// One field with no heading, on its own row.
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

/// One of the four family multipliers: an identical `1..=100` slider, so the
/// four differ only in their key, label and help.
fn mult(key: &str, help: &str) -> Field {
    f(
        key,
        key,
        Kind::Int {
            default: i64::from(MULT_MIN),
            min: i64::from(MULT_MIN),
            max: i64::from(MULT_MAX),
            step: 1,
            slider: true,
            format: Some("%dx".to_string()),
        },
        help,
    )
}

/// What Desert Overlay draws for `DesertGatherer.ini`: the keys, in menu
/// order, with the labels, ranges and help the menu shows. Written to
/// `DesertGatherer.overlay.ini` at every launch (see `lib.rs`); the overlay
/// reads that file and needs no knowledge of this plugin at all.
///
/// `Debug` is here too, at the bottom under `Diagnostics:`. It used to be left
/// out as a diagnostic that costs 400 log lines, which stopped being tenable
/// once this schema became what the plugin writes its own ini from
/// (`schema::create_ini_if_missing`): a key that is not named here is missing
/// from the generated file as well as from the menu, and a player working from
/// that file would never find out it existed. `DryRun` stays where it is, near
/// the top: it is the one diagnostic a player genuinely reaches for.
///
/// Every default is `Config::default()` (vanilla yields, which the shipped
/// template now matches) and the multiplier range is
/// [`MULT_MIN`]..=[`MULT_MAX`], the same consts `parse` enforces. Both facts
/// are tested below.
pub fn schema() -> Section {
    let d = Config::default();
    Section {
        title: "Desert Gatherer".to_string(),
        ini: crate::INI_NAME.to_string(),
        module: Some(MODULE.to_string()),
        // After Desert Looter's 10.
        order: 20,
        notice: Some(
            "Takes effect on records the game loads next; already-loaded ones keep their yields."
                .to_string(),
        ),
        presets_label: None,
        presets: Vec::new(),
        fields: vec![
            f(
                "Enabled",
                "Enabled",
                Kind::Bool { default: d.enabled },
                "Master switch. 0 = the record-loader hook is installed but a pass-through: it reads and writes nothing.",
            ),
            Field {
                same_line: true,
                ..f(
                    "DryRun",
                    "Dry run",
                    Kind::Bool { default: d.dry_run },
                    "Log what would change and write nothing to the game.",
                )
            },
            Field {
                heading: Some("Yield multipliers:".to_string()),
                ..mult("Foraging", "Plants, fruit, berries, mushrooms, crops. 82 records.")
            },
            mult("Logging", "Firewood cut from felled trees (firewood_*). 141 records."),
            mult(
                "Mining",
                "The collect_mine family: mine_* rocks and veins, breakable stalactites. 36 records.",
            ),
            mult(
                "Ore",
                "The collect_ore family: ore_* deposits and sulfur stone, separate from Mining. 16 records.",
            ),
            Field {
                heading: Some("Diagnostics:".to_string()),
                ..f(
                    "Debug",
                    "Debug",
                    Kind::Bool { default: d.debug },
                    "Also log the records that are not gather nodes. The table holds about 13,875 of them, so the hook caps this at 400 lines; useful only when a family looks like it is missing and you want to see what the loader is handing us.",
                )
            },
        ],
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

    // -----------------------------------------------------------------------
    // The menu schema
    //
    // The schema is read by a different program (Desert Overlay) that writes
    // this plugin's ini back. These tests are the contract: every key the
    // schema names is one `parse` accepts, every default it declares is the
    // one `parse` would have produced anyway, and the multiplier range it
    // hands the menu is the range `parse` enforces.
    // -----------------------------------------------------------------------

    use desert_core::schema as sch;

    #[test]
    fn schema_defaults_parse_back_to_the_default_config() {
        let (cfg, w) = parse(&sch::render_ini_defaults(&schema(), ""));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg, Config::default());
        assert!(cfg.all_vanilla(), "the menu opens at vanilla yields; raising one is the player's call");
    }

    #[test]
    fn schema_survives_a_render_and_parse_round_trip() {
        let want = schema();
        let text = sch::render(&want, "written by the test");
        let (got, w) = sch::parse(&text).expect("the rendered schema must parse");
        assert!(w.is_empty(), "{w:?}\n{text}");
        assert_eq!(got, want, "{text}");
    }

    #[test]
    fn parse_accepts_every_key_the_schema_names() {
        for field in &schema().fields {
            let text = format!("{}={}\n", field.key, field.kind.default_text());
            let (_, w) = parse(&text);
            assert!(w.is_empty(), "{}: {w:?}", field.key);
        }
    }

    #[test]
    fn the_schema_names_the_files_this_crate_ships() {
        let s = schema();
        assert_eq!(s.ini, crate::INI_NAME);
        assert_eq!(s.module.as_deref(), Some("DesertGatherer.asi"));
        assert_eq!(s.schema_file_name(), "DesertGatherer.overlay.ini");
        assert!(s.presets.is_empty(), "there is nothing to preset: four independent numbers");
        assert!(s.notice.is_some(), "the section says when a change takes effect");
    }

    #[test]
    fn schema_ranges_are_the_ones_parse_enforces() {
        let s = schema();
        for key in ["Foraging", "Logging", "Mining", "Ore"] {
            match s.field(key).map(|f| f.kind.clone()) {
                Some(Kind::Int { min, max, default, slider, .. }) => {
                    assert_eq!(min, i64::from(MULT_MIN), "{key}");
                    assert_eq!(max, i64::from(MULT_MAX), "{key}");
                    assert_eq!(default, i64::from(MULT_MIN), "{key}");
                    assert!(slider, "{key} is drawn as a slider");
                }
                other => panic!("{key} is not an int field: {other:?}"),
            }
        }
        // Both ends survive `parse`, one past either end does not: the
        // slider's stops are the real stops.
        let (c, w) = parse("Foraging=1\nLogging=100\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!((c.foraging, c.logging), (MULT_MIN, MULT_MAX));
        let (c, w) = parse("Foraging=0\nLogging=101\n");
        assert_eq!(w.len(), 2, "{w:?}");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn debug_is_in_the_schema_under_diagnostics_and_dry_run_is_not() {
        let s = schema();
        let debug = s.field("Debug").unwrap_or_else(|| panic!("the menu must offer Debug"));
        assert_eq!(debug.heading.as_deref(), Some("Diagnostics:"));
        assert_eq!(debug.kind, Kind::Bool { default: false });
        assert_eq!(debug.kind.default_text(), "0");
        // Last in the section: the heading groups the diagnostics at the end.
        assert_eq!(s.fields.last().map(|f| f.key.as_str()), Some("Debug"));

        // `DryRun` stays where it was, near the top and un-headed: it is the
        // diagnostic a player reaches for after a game update.
        let dry = s.field("DryRun").unwrap_or_else(|| panic!("no DryRun"));
        assert_eq!(dry.heading, None);
        assert_eq!(s.fields.iter().position(|f| f.key == "DryRun"), Some(1));

        let (c, w) = parse("Debug=1\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(c.debug);
    }

    /// Prints the rendered schema. `cargo test -- --ignored --nocapture
    /// show_schema` is how the file's exact text gets read by a human.
    #[test]
    #[ignore]
    fn show_schema() {
        println!("{}", sch::render(&schema(), ""));
    }
}
