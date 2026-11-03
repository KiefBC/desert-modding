//! The `[Harvest]` section of `DesertTooling.ini`, beside the game exe: one
//! section for both halves. Missing file, missing section or missing key =>
//! defaults.
//!
//! [`parse`] walks the section once through [`ini::lines_in_section`]. The two
//! keys both halves read, `Enabled` and `Debug`, are matched here and written
//! into both halves' configs; every other key is offered to the auto-loot half
//! and then to the yield half, and is reported as unknown only when neither
//! claims it. No key belongs to both halves once those two are lifted out, and
//! a test below proves it.
//!
//! Anything under another header is not this subsystem's business and is not
//! even warned about - including the pre-0.10 `[Looter]` and `[Gatherer]`
//! sections, which `crate::migrate` folds into this one on first launch.

use desert_core::ini::{self, Line};
use desert_core::schema::{Field, Kind, Section, Tab};

use crate::{gatherer, looter};

/// The one ini every subsystem shares and the overlay writes. Only [`schema`]
/// names it; the harvest thread takes the path from `Section::ini`, so the file
/// the menu writes and the file the reload loop watches cannot drift apart.
pub const INI_FILE: &str = "DesertTooling.ini";

/// The `[Header]` this subsystem owns inside [`INI_FILE`]. One constant for
/// both ends: [`schema`] declares it, [`parse`] scopes to it.
pub const INI_SECTION: &str = "Harvest";

/// Both halves' settings, as one value: what the harvest thread keeps, compares
/// on every reload and hands to each half.
///
/// `PartialEq` only, not `Eq`: the auto-loot half has `f32` ranges. Each half
/// keeps its own `enabled` and `debug`, so neither half's code had to change
/// how it reads them; [`parse`] writes the same value into both.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub looter: looter::config::Config,
    pub gatherer: gatherer::config::Config,
}

impl Config {
    /// The master switch both halves obey.
    pub fn enabled(&self) -> bool {
        self.looter.enabled
    }

    /// The verbose-logging switch both halves obey.
    pub fn debug(&self) -> bool {
        self.looter.debug
    }
}

/// `Enabled=1 Debug=0 ... DryRun=0 DumpTable=0 Foraging=1 ... Breakables=1`: the
/// `[ini]` line the harvest thread logs at startup and on every reload, the
/// auto-loot half's keys first and then the yield half's.
pub fn ini_summary(cfg: &Config) -> String {
    format!("{} {}", cfg.looter.summary(), cfg.gatherer.summary())
}

/// Parse the `[Harvest]` section of the shared ini. Unknown keys and bad values
/// *inside that section* are reported back so they can be logged; the config
/// always comes back usable.
///
/// Keys under another subsystem's header, and keys before any header at all,
/// are skipped without a warning: `Enabled` under `[Overlay]` is a different
/// setting entirely.
pub fn parse(text: &str) -> (Config, Vec<String>) {
    let mut cfg = Config::default();
    let mut warnings = Vec::new();
    for line in ini::lines_in_section(text, INI_SECTION) {
        let (k, v) = match line {
            Line::Pair(k, v) => (k, v),
            Line::Bad(w) => {
                warnings.push(w);
                continue;
            }
        };
        let lower = k.to_ascii_lowercase();
        match lower.as_str() {
            "enabled" => {
                let on = ini::parse_bool(v);
                cfg.looter.enabled = on;
                cfg.gatherer.enabled = on;
            }
            "debug" => {
                let on = ini::parse_bool(v);
                cfg.looter.debug = on;
                cfg.gatherer.debug = on;
            }
            _ => {
                let claimed = cfg.looter.apply_key(&lower, k, v, &mut warnings)
                    || cfg.gatherer.apply_key(&lower, k, v, &mut warnings);
                if !claimed {
                    warnings.push(format!("unknown key {k:?}"));
                }
            }
        }
    }
    (cfg, warnings)
}

// ---------------------------------------------------------------------------
// The menu schema
// ---------------------------------------------------------------------------

/// One field with no heading, on its own row, on this section's own tab. The
/// decorations the builders below add are the exceptions, so the common case
/// stays one line. Shared by both halves' `fields()`.
pub(crate) fn f(key: &str, label: &str, kind: Kind, help: &str) -> Field {
    Field {
        key: key.to_string(),
        label: label.to_string(),
        kind,
        heading: None,
        same_line: false,
        help: Some(help.to_string()),
        tab: Tab::Section,
    }
}

/// Draw a dim `heading` line before this field.
pub(crate) fn under(heading: &str, mut field: Field) -> Field {
    field.heading = Some(heading.to_string());
    field
}

/// Draw this field on the same row as the one before it **on the same tab**.
pub(crate) fn beside(mut field: Field) -> Field {
    field.same_line = true;
    field
}

/// Draw this field on one of the menu's shared tabs instead of the Harvest
/// tab: the hotkeys belong with every other key a player binds, and the
/// diagnostics with every other switch a bug report asks for.
pub(crate) fn on(tab: Tab, mut field: Field) -> Field {
    field.tab = tab;
    field
}

/// What Desert Overlay draws for the `[Harvest]` section of
/// `DesertTooling.ini`: the keys, in menu order, with the labels, ranges and
/// help the menu shows. `desert-tooling` hands this to the overlay at startup
/// and seeds the ini from it when the file is missing.
///
/// The overlay draws a tab's fields in this list's order, and `same_line` binds
/// to the previous field on the same tab, so the order below is what makes the
/// three tabs come out as:
///
/// - Harvest: `Enabled`, `AutoGather`, four auto-loot rows ("Gather nodes:"
///   with unarmed, "Loose items:", "Creatures:", "Bodies:"), `Hunting`, "Item log:" `LootLogMinTier` |
///   `LootLogMoney`, `ScanRange`,
///   `GatherRange`, `GatherInterval`, `NodeCooldown`, `StackLimit`, then
///   "Yield multipliers:" `Foraging` .. `Critters`, then "Pouches and
///   breakables:" `Pouches`, `Breakables` (parsed here, written by
///   `desert-dispatch`).
/// - Settings: the four hotkeys.
/// - Debug: `Debug` | `LogReceived`, `SurveyLines`, `BagTab`,
///   `DryRun` | `DumpTable`.
///
/// `Debug` therefore sits right after `Enabled` in the list: it is on another
/// tab, so the Harvest page does not see it, and being first on the Debug tab
/// is what puts `LogReceived` beside it.
///
/// Every default is `Config::default()` and every range is the one the halves'
/// `apply_key` enforces. Both facts are tested below.
pub fn schema() -> Section {
    let d = Config::default();
    let mut fields = vec![
        f(
            "Enabled",
            "Enabled",
            Kind::Bool { default: d.enabled() },
            "Master switch for the whole section. 0 = idle: no automatic gathering, no hotkeys, \
             and vanilla yields (anything already multiplied is put back). Every hook stays \
             installed, so setting it back to 1 needs no restart.",
        ),
        on(
            Tab::Debug,
            f(
                "Debug",
                "Debug",
                Kind::Bool { default: d.debug() },
                "Verbose logging for both halves. The first Survey press after this is on dumps \
                 hundreds of lines to the log; the yield hook also logs the records that are not \
                 gather nodes (the table holds nearly 14,000 of them, so it caps this at 400 \
                 lines), and one line per creature caught, with its type, class and species - \
                 which is how a fish the plugin passes over gets named. For working out why \
                 something is not picked up or multiplied, not for playing.",
            ),
        ),
    ];
    fields.extend(looter::config::fields());
    fields.extend(gatherer::config::fields());
    Section {
        title: "Desert Harvest".to_string(),
        ini: INI_FILE.to_string(),
        ini_section: INI_SECTION.to_string(),
        // One .asi, always loaded: there is no separate module whose presence
        // could gate this section.
        module: None,
        // First: auto-loot is the mod most people came for.
        order: 10,
        notice: Some(gatherer::config::notice()),
        presets_label: Some(looter::config::presets_label()),
        presets: looter::config::presets(),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // The menu schema
    //
    // The schema is read by a different subsystem (Desert Overlay) that writes
    // this section of the shared ini back, and by `desert-tooling`, which seeds
    // that ini from it. These tests are the contract: every key the schema
    // names is one `parse` accepts, every default it declares is the one
    // `parse` would have produced anyway, and every range it hands the menu is
    // the range `parse` enforces. Nothing on the overlay side checks any of
    // this - it never sees `Config` at all - so this is the only thing keeping
    // the two in step.
    // -----------------------------------------------------------------------

    use desert_core::schema as sch;

    /// The seeded file's own text, parsed back. `render_ini_defaults` emits
    /// `[Harvest]` as the header, which is exactly the header `parse` scopes
    /// itself to, so the round trip covers the section wiring as well as the
    /// defaults.
    #[test]
    fn schema_defaults_parse_back_to_the_default_config() {
        let text = sch::render_ini_defaults(&schema(), "");
        assert!(text.starts_with("[Harvest]\n"), "{text}");
        let (cfg, w) = parse(&text);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg, Config::default());
        assert!(
            cfg.gatherer.all_vanilla(),
            "the menu opens at vanilla yields; raising one is the player's call"
        );
    }

    #[test]
    fn parse_accepts_every_key_the_schema_names() {
        for field in &schema().fields {
            let text = format!("[{INI_SECTION}]\n{}={}\n", field.key, field.kind.default_text());
            let (_, w) = parse(&text);
            assert!(w.is_empty(), "{}: {w:?}", field.key);
        }
    }

    #[test]
    fn the_schema_names_the_one_shared_ini_and_this_section() {
        let s = schema();
        assert_eq!(s.ini, INI_FILE);
        assert_eq!(s.ini, "DesertTooling.ini");
        assert_eq!(s.ini_section, INI_SECTION);
        assert_eq!(s.ini_section, "Harvest");
        assert_eq!(s.title, "Desert Harvest");
        assert_eq!(s.order, 10);
        // One .asi carries every subsystem, so there is nothing left for the
        // overlay's "is it loaded?" gate to ask about.
        assert_eq!(s.module, None);
        assert_eq!(s.presets.len(), 4, "the auto-loot half's four presets");
        assert!(s.presets_label.is_some());
        assert!(s.notice.is_some(), "the section says when a yield change takes effect");
    }

    /// The whole point of the section scoping: another subsystem's `Enabled`
    /// is not this one's, and a key that only exists elsewhere is not an
    /// unknown key here. That includes the pre-0.10 `[Looter]` and
    /// `[Gatherer]` headers: until `crate::migrate` has folded them into
    /// `[Harvest]`, a file that has only those is read as every default,
    /// silently.
    #[test]
    fn only_the_harvest_section_is_read() {
        let text = "\
Foraging=9
[Looter]
Enabled=0
GatherLogging=0

[Gatherer]
Enabled=0
Foraging=4

[Harvest]
GatherOre=0
Mining=3

[Overlay]
Enabled=0
Scale=1.5
";
        let (c, w) = parse(text);
        assert!(w.is_empty(), "{w:?}");
        assert!(c.enabled(), "the [Looter], [Gatherer] and [Overlay] Enabled=0 must not reach us");
        assert!(c.looter.gather_logging, "only the [Harvest] keys count");
        assert!(!c.looter.gather_ore);
        assert_eq!(c.gatherer.foraging, 1, "neither the stray pair nor [Gatherer] counts");
        assert_eq!(c.gatherer.mining, 3);

        // Only the old headers: every default, and nothing to warn about.
        let (c, w) = parse("[Looter]\nEnabled=0\nGatherLogging=0\n[Gatherer]\nForaging=4\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c, Config::default());

        // The header match is ASCII case-insensitive, like desert-core's.
        let (c, w) = parse("[harvest]\nEnabled=0\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(!c.enabled());
    }

    #[test]
    fn keys_under_another_subsystems_header_are_ignored_silently() {
        let (c, w) = parse(
            "[Overlay]\nEnabled=0\nForaging=50\nKeyToggle=F7\n\
             [Harvest]\nForaging=3\n\
             [Dispatch]\nEnabled=0\nDryRun=1\nDebug=1\n",
        );
        assert!(w.is_empty(), "another section's keys are not ours to warn about: {w:?}");
        assert_eq!(c.gatherer.foraging, 3, "only the [Harvest] Foraging counts");
        assert!(c.enabled(), "[Overlay] Enabled=0 must not disable harvest");
        assert!(!c.gatherer.dry_run && !c.debug(), "[Dispatch] DryRun/Debug are not ours");
        assert_eq!(c.looter.key_toggle, Config::default().looter.key_toggle);
    }

    /// `Enabled` and `Debug` are one key each now, read by both halves: the
    /// root parser writes the same value into both, whichever value it is.
    #[test]
    fn enabled_and_debug_reach_both_halves() {
        for (enabled, debug) in [(false, false), (false, true), (true, false), (true, true)] {
            let text = format!("[Harvest]\nEnabled={}\nDebug={}\n", enabled as u8, debug as u8);
            let (c, w) = parse(&text);
            assert!(w.is_empty(), "{w:?}");
            assert_eq!((c.looter.enabled, c.gatherer.enabled), (enabled, enabled), "{text}");
            assert_eq!((c.looter.debug, c.gatherer.debug), (debug, debug), "{text}");
            assert_eq!((c.enabled(), c.debug()), (enabled, debug));
        }
        // The last one wins, the way every other key does.
        let (c, _) = parse("[Harvest]\nEnabled=0\nEnabled=1\n");
        assert!(c.looter.enabled && c.gatherer.enabled);
        // The defaults agree, so `Config::default()` is one consistent value.
        let d = Config::default();
        assert_eq!(d.looter.enabled, d.gatherer.enabled);
        assert_eq!(d.looter.debug, d.gatherer.debug);
    }

    /// Every key belongs to exactly one place: the root (`Enabled`, `Debug`)
    /// or one half. A key both halves claimed would be applied by the
    /// auto-loot half only, silently, because the root offers it there first.
    #[test]
    fn the_two_halves_share_no_key() {
        let mut lc = looter::config::Config::default();
        let mut gc = gatherer::config::Config::default();
        let mut w = Vec::new();
        for field in schema().fields {
            let k = field.key.as_str();
            let v = field.kind.default_text();
            let lower = k.to_ascii_lowercase();
            let by_looter = lc.apply_key(&lower, k, &v, &mut w);
            let by_gatherer = gc.apply_key(&lower, k, &v, &mut w);
            if matches!(k, "Enabled" | "Debug") {
                assert!(!by_looter && !by_gatherer, "{k} is the root's, not a half's");
            } else {
                assert!(by_looter != by_gatherer, "{k}: looter {by_looter}, gatherer {by_gatherer}");
            }
        }
        assert!(w.is_empty(), "{w:?}");
        // The halves' own field lists agree with what they claim.
        for field in looter::config::fields() {
            let lower = field.key.to_ascii_lowercase();
            assert!(!gc.apply_key(&lower, &field.key, "1", &mut w), "{}", field.key);
        }
        for field in gatherer::config::fields() {
            let lower = field.key.to_ascii_lowercase();
            assert!(!lc.apply_key(&lower, &field.key, "1", &mut w), "{}", field.key);
        }
    }

    /// One tab's fields in draw order, each with whether it is drawn beside
    /// the one before it.
    fn tab(s: &Section, tab: Tab) -> Vec<(String, bool)> {
        s.fields.iter().filter(|f| f.tab == tab).map(|f| (f.key.clone(), f.same_line)).collect()
    }

    fn expect(keys: &[&str]) -> Vec<(String, bool)> {
        keys.iter()
            .map(|k| match k.strip_prefix('|') {
                Some(k) => (k.to_string(), true),
                None => (k.to_string(), false),
            })
            .collect()
    }

    /// The merged order, pinned tab by tab: what used to be two sections' tests
    /// (`debug_and_dry_run_are_on_the_shared_debug_tab_...` and
    /// `bugs_and_fish_are_the_last_two_yield_multipliers`). `|` marks a field
    /// drawn beside the one before it on the same tab.
    #[test]
    fn the_merged_fields_land_on_their_tabs_in_this_order() {
        let s = schema();
        assert_eq!(
            tab(&s, Tab::Section),
            expect(&[
                "Enabled",
                "AutoGather",
                "GatherForaging",
                "|GatherLogging",
                "|GatherMining",
                "|GatherOre",
                "|GatherMoney",
                "|GatherUnarmed",
                "GatherItems",
                "|GatherGear",
                "GatherBugs",
                "|GatherFish",
                "|GatherCritters",
                "GatherCarcass",
                "|GatherBodies",
                "|GatherFriendlyBodies",
                "Hunting",
                "LootLogMinTier",
                "LootLogMoney",
                "ScanRange",
                "GatherRange",
                "GatherInterval",
                "NodeCooldown",
                "StackLimit",
                "Foraging",
                "Logging",
                "Mining",
                "Ore",
                "Money",
                "Bugs",
                "Fish",
                "Critters",
                "Pouches",
                "Breakables",
            ])
        );
        assert_eq!(tab(&s, Tab::Settings), expect(&["KeyToggle", "KeyScan", "KeyGather", "KeyRecord"]));
        assert_eq!(
            tab(&s, Tab::Debug),
            expect(&["Debug", "|LogReceived", "SurveyLines", "BagTab", "DryRun", "|DumpTable"])
        );

        // The headings: four auto-loot rows (nodes, loose items, creatures,
        // bodies), and one list of yield multipliers that Bugs, Fish and
        // Critters continue rather than start.
        let heading = |key: &str| s.field(key).and_then(|f| f.heading.clone());
        assert_eq!(heading("GatherForaging").as_deref(), Some("Gather nodes:"));
        assert_eq!(heading("GatherItems").as_deref(), Some("Loose items:"));
        assert_eq!(heading("GatherBugs").as_deref(), Some("Creatures:"));
        assert_eq!(heading("GatherCarcass").as_deref(), Some("Bodies:"));
        assert_eq!(heading("Foraging").as_deref(), Some("Yield multipliers:"));
        assert_eq!(heading("Pouches").as_deref(), Some("Pouches and breakables:"));
        for key in ["Logging", "Mining", "Ore", "Money", "Bugs", "Fish", "Critters", "Breakables", "Debug", "DryRun", "DumpTable"] {
            assert_eq!(heading(key), None, "{key}");
        }

        // The Debug tab switches.
        for key in ["Debug", "DryRun", "DumpTable"] {
            let f = s.field(key).unwrap_or_else(|| panic!("the menu must offer {key}"));
            assert_eq!(f.kind, Kind::Bool { default: false }, "{key}");
            assert_eq!(f.kind.default_text(), "0");
        }
        // `DryRun` scopes to the yields, and its help has to say so now that
        // it sits in a section that also auto-loots.
        let dry = s.field("DryRun").and_then(|f| f.help.clone()).unwrap_or_default();
        assert!(dry.starts_with("Yield multipliers only"), "{dry}");
        // `DumpTable` only works with DryRun ticked, so it sits beside it, and
        // its help has to say so, name the file, and say it is once per launch.
        let dump = s.field("DumpTable").and_then(|f| f.help.clone()).unwrap_or_default();
        for needed in ["DryRun=1", "DesertTooling.gimmickinfo.bin", "next launch", "once per session"] {
            assert!(dump.contains(needed), "the help must say {needed:?}: {dump}");
        }
        assert!(dump.contains(crate::gatherer::dump::FILE_NAME), "{dump}");
        assert_eq!(s.fields.last().map(|f| f.key.as_str()), Some("DumpTable"));

        // The notice has to say when a catch multiplier lands, because it is
        // not the "next gather" the yield sliders talk about.
        let notice = s.notice.clone().unwrap_or_default();
        assert!(notice.contains("next gather") && notice.contains("next catch"), "{notice}");
        assert!(notice.len() <= 160, "one or two lines in the menu, not a paragraph: {notice}");
    }

    /// Where the menu draws each key. The four hotkeys go on the shared
    /// Settings tab beside every other binding, the diagnostics on the shared
    /// Debug tab, and everything else stays on the Harvest tab - which is also
    /// what keeps that tab from disappearing, because a section with nothing
    /// left on [`Tab::Section`] gets no tab at all.
    #[test]
    fn the_keys_and_the_diagnostics_are_on_the_shared_tabs() {
        let s = schema();
        let at = |key: &str| s.field(key).map(|f| f.tab);
        for key in ["KeyToggle", "KeyScan", "KeyGather", "KeyRecord"] {
            assert_eq!(at(key), Some(Tab::Settings), "{key}");
        }
        let debug = ["Debug", "LogReceived", "BagTab", "SurveyLines", "DryRun", "DumpTable"];
        for key in debug {
            assert_eq!(at(key), Some(Tab::Debug), "{key}");
        }
        for field in &s.fields {
            if field.key.starts_with("Key") || debug.contains(&field.key.as_str()) {
                continue;
            }
            assert_eq!(field.tab, Tab::Section, "{} belongs on the Harvest tab", field.key);
        }
        for key in ["Foraging", "Logging", "Mining", "Ore", "Money", "Bugs", "Fish", "Critters", "Pouches", "Breakables"] {
            assert_eq!(at(key), Some(Tab::Section), "{key}");
        }
    }

    /// The `[ini]` line names `Enabled` and `Debug` once each, then every
    /// other key of both halves.
    #[test]
    fn the_ini_summary_names_the_shared_keys_once() {
        let line = ini_summary(&Config::default());
        assert!(line.starts_with("Enabled=1 Debug=0 "), "{line}");
        for key in ["Enabled=", "Debug=", "DryRun=", "DumpTable="] {
            assert_eq!(line.matches(&format!(" {key}")).count() + usize::from(line.starts_with(key)), 1, "{key} in {line}");
        }
        assert!(line.ends_with("Bugs=1 Fish=1 Critters=1 Pouches=1 Breakables=1"), "{line}");
    }

    /// Prints this section as it is seeded into `DesertTooling.ini`.
    /// `cargo test -- --ignored --nocapture show_schema` is how the exact text
    /// gets read by a human.
    #[test]
    #[ignore]
    fn show_schema() {
        println!("{}", sch::render_ini_defaults(&schema(), ""));
    }

    /// The auto-loot half's parse tests, through the root parser. `parse` here
    /// is the root's with the `.looter` half picked out, so the bodies read the
    /// way they did when the half had a parser of its own.
    mod looter_half {
        use crate::config::{schema, INI_SECTION};
        use crate::looter::config::*;
        use desert_core::collect::Family;
        use desert_core::ini;
        use desert_core::schema::{Kind, Tab};

        fn parse(text: &str) -> (Config, Vec<String>) {
            let (c, w) = crate::config::parse(text);
            (c.looter, w)
        }

        /// The parser only ever sees the shared ini, so every test feeds it a
        /// `[Harvest]` section. This is the one place the header is spelled.
        fn sectioned(body: &str) -> String {
            format!("[{INI_SECTION}]\n{body}")
        }

        #[test]
        fn parses_and_warns() {
            let (c, w) = parse("; c\n[Harvest]\nEnabled=0\nDebug=1\nLogReceived=1\nScanRange=25.5\nGatherRange=4\nAutoGather=1\nGatherInterval=250\nNodeCooldown=5\nKeyToggle=F5\nKeyScan=nope\nKeyGather=F8\nJunk=1\n");
            assert!(!c.enabled);
            assert!(c.debug);
            assert!(c.log_received);
            assert!(!Config::default().log_received);
            assert_eq!(c.scan_range, 25.5);
            assert_eq!(c.key_toggle, 0x74);
            assert_eq!(c.key_scan, Config::default().key_scan);
            assert_eq!(c.key_gather, 0x77);
            assert_eq!(c.gather_range, 4.0);
            assert!(c.auto_gather);
            assert_eq!(c.gather_interval_ms, 250);
            assert_eq!(c.node_cooldown_ms, Config::default().node_cooldown_ms);
            assert_eq!(w.len(), 3, "{w:?}");
        }

        #[test]
        fn family_switches_default_on_and_parse() {
            let d = Config::default();
            assert!(d.gather_foraging && d.gather_logging && d.gather_mining && d.gather_ore);
            for f in [Family::Foraging, Family::Logging, Family::Mining, Family::Ore] {
                assert!(d.allows_family(f), "{f:?} should be on by default");
            }
            // Money is the fifth family and the only one that starts off, so it
            // is not in the loop above; it has its own test below.
            assert!(!d.gather_money);
            assert!(!d.allows_family(Family::Money), "Money is off until it is asked for");
            // The water well is a Foraging record and rides on `GatherForaging` -
            // see `allows_family`.
            assert_eq!(desert_core::collect::family_by_key(1001081), Some(Family::Foraging));
            // - and yet the looter must refuse it, by key, before the family
            // switch is even consulted: a forged pickup at the bucket breaks the
            // well (see `WELL_BUCKET_RECORD_KEY`). Both halves are asserted so a
            // change to either side has to come here and argue.
            assert_eq!(WELL_BUCKET_RECORD_KEY, 1001081);
            assert_eq!(
                desert_core::collect::family_by_name(WELL_BUCKET_RECORD_NAME),
                Some(Family::Foraging),
                "the name constant must be the record collect.rs knows"
            );
            // Key alone, name alone, both, and the unreadable-key case.
            assert!(never_forge_pickup(Some(WELL_BUCKET_RECORD_KEY), None));
            assert!(never_forge_pickup(None, Some(WELL_BUCKET_RECORD_NAME)));
            assert!(never_forge_pickup(Some(WELL_BUCKET_RECORD_KEY), Some(WELL_BUCKET_RECORD_NAME)));
            assert!(never_forge_pickup(None, Some("gimmick_well_0001_parts01")));
            // The crank, its neighbours, firewood and money are all let through,
            // and a record with neither identifier readable is not refused (it
            // is not a gather node either - `family` would be None).
            assert!(!never_forge_pickup(None, Some("gimmick_well_0001_parts02")));
            for key in [1001080, 1001082, 1002971, 1] {
                assert!(!never_forge_pickup(Some(key), None), "{key} must not be refused");
            }
            assert!(!never_forge_pickup(None, None));
            let help = schema().field("GatherForaging").and_then(|f| f.help.clone()).unwrap_or_default();
            assert!(help.contains("water well"), "the Foraging switch has to say it excludes the well: {help}");
            assert!(
                schema().field("GatherIngredients").is_none(),
                "Ingredients never shipped, so the menu must not offer a switch for it"
            );
            let (c, w) = parse(&sectioned("GatherForaging=1\nGatherLogging=0\nGatherMining=no\nGatherOre=on\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.allows_family(Family::Foraging));
            assert!(!c.allows_family(Family::Logging));
            assert!(!c.allows_family(Family::Mining));
            assert!(c.allows_family(Family::Ore));
            // The four say nothing about the fifth: this file set none of them.
            assert!(!c.allows_family(Family::Money));
        }

        /// `GatherBugs` and `GatherFish` are not gather families - they are one
        /// different game event at a different kind of actor, with the creature's
        /// class byte deciding which switch applies - so neither has an
        /// `allows_family` arm and no preset touches either. What they do share
        /// with the family switches is the shape: on by default, a plain bool in
        /// the ini, and named by the schema.
        #[test]
        fn gather_bugs_defaults_on_and_parses_like_the_family_switches() {
            assert!(Config::default().gather_bugs);
            let (c, w) = parse(&sectioned("GatherBugs=0\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_bugs);
            let (c, w) = parse(&sectioned("GatherBugs=on\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_bugs);
            assert!(schema().field("GatherBugs").is_some(), "the menu must offer GatherBugs");
            // No preset sets it: a preset only chooses what to gather, and every
            // one of them is about node families and ground items.
            for p in &schema().presets {
                assert!(!p.set.iter().any(|(k, _)| k == "GatherBugs"), "{}", p.label);
            }
        }

        /// The same, for `GatherFish`. It is a sibling of `GatherBugs` in every
        /// respect (see `actors::catch_class`), so it is held to the same shape.
        #[test]
        fn gather_fish_defaults_on_and_parses_like_gather_bugs() {
            assert!(Config::default().gather_fish);
            let (c, w) = parse(&sectioned("GatherFish=0\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_fish);
            let (c, w) = parse(&sectioned("GatherFish=on\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_fish);
            assert!(schema().field("GatherFish").is_some(), "the menu must offer GatherFish");
            for p in &schema().presets {
                assert!(!p.set.iter().any(|(k, _)| k == "GatherFish"), "{}", p.label);
            }
            // The two switches are independent: turning insects off must not take
            // fish with it, and vice versa.
            let (c, w) = parse(&sectioned("GatherBugs=0\nGatherFish=1\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_bugs && c.gather_fish);
        }

        /// The same, for `GatherCritters`: the small animals the species table
        /// files as bag catches outside the insect and fish groups. A sibling
        /// of `GatherBugs` and `GatherFish`, held to the same shape, and
        /// independent of both.
        #[test]
        fn gather_critters_defaults_on_and_parses_like_gather_bugs() {
            assert!(Config::default().gather_critters);
            let (c, w) = parse(&sectioned("GatherCritters=0\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_critters);
            let (c, w) = parse(&sectioned("GatherCritters=on\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_critters);
            assert!(schema().field("GatherCritters").is_some(), "the menu must offer GatherCritters");
            for p in &schema().presets {
                assert!(!p.set.iter().any(|(k, _)| k == "GatherCritters"), "{}", p.label);
            }
            let (c, w) = parse(&sectioned("GatherBugs=0\nGatherFish=0\nGatherCritters=1\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_bugs && !c.gather_fish && c.gather_critters);
            let (c, w) = parse(&sectioned("GatherCritters=0\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_bugs && c.gather_fish && !c.gather_critters);
        }

        /// `GatherMoney` is a family switch like the four above - it has an
        /// `allows_family` arm and a `Family::Money` behind it - but it is the one
        /// that starts **off**, so it is held to the opposite default. The coin
        /// props carry neither an interaction object nor an instance object, so
        /// they classify `Unarmed`, and `GatherUnarmed=1` (the default) plus this
        /// would forge a pickup at one the same way as at an ore dropping. Nobody
        /// has tried that in game yet, and a MINOR may not change behaviour.
        #[test]
        fn gather_money_defaults_off_and_follows_the_family_switch() {
            assert!(!Config::default().gather_money);
            let (c, w) = parse(&sectioned("GatherMoney=1\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_money);
            assert!(c.allows_family(Family::Money), "the switch is what allows_family reads");
            let (c, w) = parse(&sectioned("GatherMoney=off\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_money && !c.allows_family(Family::Money));
            assert!(schema().field("GatherMoney").is_some(), "the menu must offer GatherMoney");
            // No preset names it, the same as GatherBugs and GatherFish: a preset
            // sets the four gather families plus GatherItems and nothing else, so an
            // untested switch can never be turned on by pressing a button.
            for p in &schema().presets {
                assert!(!p.set.iter().any(|(k, _)| k == "GatherMoney"), "{}", p.label);
            }
            // The help has to say both halves: that it is untested, and that the
            // amount is not this key's business.
            let help = schema().field("GatherMoney").and_then(|f| f.help.clone()).unwrap_or_default();
            assert!(help.contains("Untested in game"), "{help}");
            assert!(help.contains("the Money multiplier below"), "{help}");
        }

        /// `GatherCarcass` is the third switch that starts **off**, and it is
        /// neither a gather family nor a catch: skinning is its own game event at
        /// its own kind of actor (a dead one), so it has no `allows_family` arm
        /// and no preset names it. The default is 0 for the `GatherMoney` reason -
        /// forging this event has never been tried in game, and a MINOR may not
        /// change behaviour - and the help has to say so, because the ini and the
        /// menu are where a player finds that out.
        #[test]
        fn gather_carcass_defaults_off_and_parses_like_gather_money() {
            assert!(!Config::default().gather_carcass);
            let (c, w) = parse(&sectioned("GatherCarcass=1\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_carcass);
            let (c, w) = parse(&sectioned("GatherCarcass=on\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_carcass);
            let (c, w) = parse(&sectioned("GatherCarcass=off\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_carcass);
            assert!(schema().field("GatherCarcass").is_some(), "the menu must offer GatherCarcass");
            // No preset names it, the same as GatherBugs, GatherFish and
            // GatherMoney: a preset chooses node families and ground items, so an
            // untested switch can never be turned on by pressing a button.
            for p in &schema().presets {
                assert!(!p.set.iter().any(|(k, _)| k == "GatherCarcass"), "{}", p.label);
            }
            // The help has to say what it does and that it starts off.
            let help = schema().field("GatherCarcass").and_then(|f| f.help.clone()).unwrap_or_default();
            assert!(help.contains("0 by default"), "{help}");
            assert!(help.contains("carcasses within GatherRange"), "{help}");
            assert!(help.contains("without the skinning animation"), "{help}");
            // It says nothing about the other switches, and they say nothing about
            // it: turning insects and fish off must not take skinning with them.
            let (c, w) = parse(&sectioned("GatherBugs=0\nGatherFish=0\nGatherCarcass=1\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(!c.gather_bugs && !c.gather_fish && c.gather_carcass);
        }

        /// `GatherBodies` and `GatherFriendlyBodies` start **off**, parse like
        /// `GatherCarcass`, are offered by the menu, are named by no preset, and
        /// are independent of `GatherCarcass` in both directions.
        #[test]
        fn gather_bodies_default_off_and_parse_like_gather_carcass() {
            let d = Config::default();
            assert!(!d.gather_bodies && !d.gather_friendly_bodies);
            let (c, w) = parse(&sectioned("GatherBodies=1
"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_bodies && !c.gather_friendly_bodies && !c.gather_carcass);
            let (c, w) = parse(&sectioned("GatherBodies=on
GatherFriendlyBodies=yes
"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_bodies && c.gather_friendly_bodies);
            let (c, w) = parse(&sectioned("GatherCarcass=1
GatherBodies=off
"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.gather_carcass && !c.gather_bodies);
            for key in ["GatherBodies", "GatherFriendlyBodies"] {
                assert!(schema().field(key).is_some(), "the menu must offer {key}");
                for p in &schema().presets {
                    assert!(!p.set.iter().any(|(k, _)| k == key), "{}", p.label);
                }
                let help = schema().field(key).and_then(|f| f.help.clone()).unwrap_or_default();
                assert!(help.contains("0 by default"), "{key}: {help}");
            }
            let help = schema().field("GatherBodies").and_then(|f| f.help.clone()).unwrap_or_default();
            assert!(help.contains("quest trigger are skipped"), "{help}");
            let help = schema().field("GatherFriendlyBodies").and_then(|f| f.help.clone()).unwrap_or_default();
            assert!(help.contains("Needs GatherBodies=1"), "{help}");
        }

        /// `Hunting` is the one key in this section that is an amount rather than
        /// a switch, so it is held to the multiplier rules the yield
        /// multiplier sliders are: the same `1..=100`, a default of 1, and a bad value that
        /// warns and keeps the default rather than replacing it.
        #[test]
        fn hunting_defaults_to_vanilla_and_refuses_a_value_outside_its_range() {
            assert_eq!(Config::default().hunting, 1, "the default must be vanilla");
            let (c, w) = parse(&sectioned("Hunting=5\n"));
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.hunting, 5);
            // Both ends of the range are accepted.
            for n in [HUNTING_RANGE.0, HUNTING_RANGE.1] {
                let (c, w) = parse(&sectioned(&format!("Hunting={n}\n")));
                assert!(w.is_empty(), "{w:?}");
                assert_eq!(c.hunting, n);
            }
            // 0 would zero a carcass out rather than leave it alone, which is why
            // the floor is 1 and not 0.
            for bad in ["0", "101", "-1", "x", ""] {
                let (c, w) = parse(&sectioned(&format!("Hunting={bad}\n")));
                assert_eq!(c.hunting, 1, "{bad:?} must keep the default");
                assert_eq!(w.len(), 1, "{bad:?} must warn: {w:?}");
            }
        }

        /// The menu must offer `Hunting` over the range `parse` enforces - the
        /// overlay writes this same ini, so a slider wider than the parser would
        /// write values the parser then throws away - and no preset may name it,
        /// for the reason no preset names `GatherMoney`: it is a balance lever,
        /// and pressing a button must never raise how much the player is given.
        #[test]
        fn hunting_is_on_the_menu_over_the_range_parse_enforces_and_in_no_preset() {
            let section = schema();
            let field = section.field("Hunting").expect("the menu must offer Hunting");
            match field.kind {
                Kind::Int { min, max, default, slider, .. } => {
                    assert_eq!(min, i64::from(HUNTING_RANGE.0));
                    assert_eq!(max, i64::from(HUNTING_RANGE.1));
                    assert_eq!(default, i64::from(Config::default().hunting));
                    assert!(slider, "the other multipliers are sliders and so is this");
                }
                ref k => panic!("Hunting must be an Int, not {k:?}"),
            }
            for p in &section.presets {
                assert!(!p.set.iter().any(|(k, _)| k == "Hunting"), "{}", p.label);
            }
            // The help has to say the two things a player cannot infer: which key
            // it does and does not depend on, and that it is not a table edit
            // reaching the rest of the game's loot.
            let help = field.help.clone().unwrap_or_default();
            assert!(help.contains("GatherCarcass=1"), "{help}");
            assert!(help.contains("nothing in any table is edited"), "{help}");
            // It must not claim to need GatherCarcass: it does not, and a player
            // who read that would leave the key at 1 believing it was inert.
            assert!(!help.contains("Needs GatherCarcass"), "{help}");
        }

        /// The item-log filter's two keys: vanilla by default (every line,
        /// money on), both ends of the tier range accepted, anything past
        /// them warned about and refused, and the menu offering exactly the
        /// range `parse` enforces. No preset may set either.
        #[test]
        fn loot_log_keys_default_to_vanilla_and_hold_their_range() {
            let d = Config::default();
            assert_eq!((d.loot_log_min_tier, d.loot_log_money), (0, true));
            let (c, w) = parse(&sectioned("LootLogMinTier=3\nLootLogMoney=0\n"));
            assert!(w.is_empty(), "{w:?}");
            assert_eq!((c.loot_log_min_tier, c.loot_log_money), (3, false));
            for n in [LOOT_LOG_TIER_RANGE.0, LOOT_LOG_TIER_RANGE.1] {
                let (c, w) = parse(&sectioned(&format!("LootLogMinTier={n}\n")));
                assert!(w.is_empty(), "{w:?}");
                assert_eq!(u32::from(c.loot_log_min_tier), n);
            }
            for bad in ["7", "-1", "256", "x", ""] {
                let (c, w) = parse(&sectioned(&format!("LootLogMinTier={bad}\n")));
                assert_eq!(c.loot_log_min_tier, 0, "{bad:?} must keep the default");
                assert_eq!(w.len(), 1, "{bad:?} must warn: {w:?}");
            }
            let section = schema();
            match section.field("LootLogMinTier").map(|f| f.kind.clone()) {
                Some(Kind::Int { min, max, default, slider, .. }) => {
                    assert_eq!((min, max), (i64::from(LOOT_LOG_TIER_RANGE.0), i64::from(LOOT_LOG_TIER_RANGE.1)));
                    assert_eq!(default, 0);
                    assert!(slider);
                }
                other => panic!("LootLogMinTier must be an Int, not {other:?}"),
            }
            assert!(matches!(
                section.field("LootLogMoney").map(|f| f.kind.clone()),
                Some(Kind::Bool { default: true })
            ));
            for p in &section.presets {
                assert!(!p.set.iter().any(|(k, _)| k.starts_with("LootLog")), "{}", p.label);
            }
        }

        #[test]
        fn every_preset_names_a_field_and_sets_a_value_it_accepts() {
            let s = schema();
            assert_eq!(s.presets.len(), 4);
            for p in &s.presets {
                for (k, v) in &p.set {
                    let field = s.field(k).unwrap_or_else(|| panic!("{}: no field {k}", p.label));
                    assert_eq!(
                        field.kind.normalize(v).as_deref(),
                        Some(v.as_str()),
                        "{}: {k}={v} is not what {k} would be written as",
                        p.label
                    );
                }
                let (cfg, w) = parse(&sectioned(
                    &p.set.iter().map(|(k, v)| format!("{k}={v}\n")).collect::<String>(),
                ));
                assert!(w.is_empty(), "{}: {w:?}", p.label);
                assert!(cfg.gather_items, "{}: every preset turns GatherItems on", p.label);
            }
        }

        #[test]
        fn presets_set_the_families_they_are_named_for() {
            let s = schema();
            for (label, want) in [
                ("Everything", (true, true, true, true)),
                ("Plants only", (true, false, false, false)),
                ("Wood only", (false, true, false, false)),
                ("Rock and ore only", (false, false, true, true)),
            ] {
                let p = s.presets.iter().find(|p| p.label == label).unwrap_or_else(|| panic!("{label}"));
                let text = sectioned(&p.set.iter().map(|(k, v)| format!("{k}={v}\n")).collect::<String>());
                let (c, _) = parse(&text);
                assert_eq!(
                    (c.gather_foraging, c.gather_logging, c.gather_mining, c.gather_ore),
                    want,
                    "{label}"
                );
            }
        }

        #[test]
        fn schema_ranges_are_the_ones_parse_enforces() {
            let s = schema();
            let int = |key: &str| match s.field(key).map(|f| f.kind.clone()) {
                Some(Kind::Int { min, max, step, .. }) => (min, max, step),
                other => panic!("{key} is not an int field: {other:?}"),
            };
            assert_eq!(int("GatherInterval"), (i64::from(MS_RANGE.0), i64::from(MS_RANGE.1), 50));
            assert_eq!(int("NodeCooldown"), (i64::from(MS_RANGE.0), i64::from(MS_RANGE.1), 500));
            assert_eq!(
                int("StackLimit"),
                (i64::from(STACK_LIMIT_RANGE.0), i64::from(STACK_LIMIT_RANGE.1), 10)
            );

            match s.field("GatherRange").map(|f| f.kind.clone()) {
                Some(Kind::Float { min, max, .. }) => {
                    assert_eq!((min, max), (GATHER_RANGE_MENU_MIN, GATHER_RANGE_MAX));
                }
                other => panic!("GatherRange is not a float field: {other:?}"),
            }
            match s.field("ScanRange").map(|f| f.kind.clone()) {
                Some(Kind::Float { min, max, .. }) => {
                    assert_eq!((min, max), SCAN_RANGE_MENU);
                }
                other => panic!("ScanRange is not a float field: {other:?}"),
            }
            match s.field("SurveyLines").map(|f| f.kind.clone()) {
                Some(Kind::Int { min, max, default, .. }) => {
                    assert_eq!((min, max), (i64::from(SURVEY_LINES_RANGE.0), i64::from(SURVEY_LINES_RANGE.1)));
                    assert_eq!(default, 200, "the old hard-coded 64 must not come back as the default");
                }
                other => panic!("SurveyLines is not an int field: {other:?}"),
            }

            // A value at either end of every int range is one `parse` keeps, and
            // one past it is one it refuses: the menu's stops are the real stops.
            let (c, w) = parse(&sectioned("GatherInterval=100\nNodeCooldown=60000\nStackLimit=1000000\nSurveyLines=2000\n"));
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.gather_interval_ms, MS_RANGE.0);
            assert_eq!(c.node_cooldown_ms, MS_RANGE.1);
            assert_eq!(c.stack_limit, STACK_LIMIT_RANGE.1);
            assert_eq!(c.survey_lines, SURVEY_LINES_RANGE.1);
            let (c, w) = parse(&sectioned("SurveyLines=16\n"));
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.survey_lines, SURVEY_LINES_RANGE.0);
            let (c, w) = parse(&sectioned("GatherInterval=99\nNodeCooldown=60001\nStackLimit=9\nGatherRange=50.1\nSurveyLines=15\n"));
            assert_eq!(w.len(), 5, "{w:?}");
            assert_eq!(c, Config::default());
        }

        #[test]
        fn the_key_fields_default_to_the_configs_own_bindings() {
            let s = schema();
            let d = Config::default();
            for (key, vk) in [
                ("KeyToggle", d.key_toggle),
                ("KeyScan", d.key_scan),
                ("KeyGather", d.key_gather),
                ("KeyRecord", d.key_record),
            ] {
                let field = s.field(key).unwrap_or_else(|| panic!("no {key}"));
                let text = field.kind.default_text();
                assert_eq!(ini::vk_from_name(&text), Some(vk), "{key}={text}");
            }
            assert_eq!(s.field("KeyToggle").map(|f| f.kind.default_text()), Some("F10".to_string()));
        }

        #[test]
        fn the_diagnostics_keys_are_in_the_schema_and_bag_tab_reaches_auto() {
            let s = schema();
            for key in ["Debug", "LogReceived", "BagTab"] {
                assert!(s.field(key).is_some(), "the menu must offer {key}");
            }
            // What groups them is the tab now, not a heading: the menu draws the
            // section's title above the group on its shared Debug tab.
            assert_eq!(s.field("Debug").map(|f| f.tab), Some(Tab::Debug));
            assert_eq!(s.field("Debug").and_then(|f| f.heading.clone()), None);

            // `BagTab` opens at the default tab, and the bottom of its range is
            // the "auto" the parser turns into `None`: a value the menu can
            // actually produce, written and read back through the same two paths
            // the overlay uses.
            let field = s.field("BagTab").unwrap_or_else(|| panic!("no BagTab"));
            assert_eq!(field.kind.default_text(), "1");
            let auto = field.kind.normalize("-1").unwrap_or_else(|| panic!("-1 is out of range"));
            assert_eq!(auto, "-1");
            let (c, w) = parse(&sectioned(&format!("BagTab={auto}\n")));
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.bag_tab, None);
            // One past the auto end is not something the menu can write.
            assert_eq!(field.kind.normalize("-2"), None);
            assert_eq!(field.kind.normalize(&(BAG_TAB_MENU_MAX + 1).to_string()), None);

            let (c, w) = parse(&sectioned("Debug=1\nLogReceived=1\nBagTab=0\n"));
            assert!(w.is_empty(), "{w:?}");
            assert!(c.debug && c.log_received);
            assert_eq!(c.bag_tab, Some(0));
        }
    }

    /// The yield half's parse tests, through the root parser, the same way.
    mod gatherer_half {
        use crate::config::schema;
        use crate::gatherer::config::*;
        use desert_core::collect::Family;
        use desert_core::creature::CatchClass;
        use desert_core::schema::Kind;

        fn parse(text: &str) -> (Config, Vec<String>) {
            let (c, w) = crate::config::parse(text);
            (c.gatherer, w)
        }

        #[test]
        fn parses_every_key() {
            let (c, w) = parse(
                "; comment\n[Harvest]\nEnabled=1\nDryRun=yes\nDebug=on\nDumpTable=true\n\
                 Foraging=10\nLogging=2\nMining=5\nOre=100\nMoney=4\nBugs=3\nFish=7\nCritters=5\n\
                 Pouches=6\nBreakables=9\n",
            );
            assert!(c.enabled);
            assert!(c.dry_run);
            assert!(c.debug);
            assert!(c.dump_table);
            assert_eq!(c.multiplier(Family::Foraging), 10);
            assert_eq!(c.multiplier(Family::Logging), 2);
            assert_eq!(c.multiplier(Family::Mining), 5);
            assert_eq!(c.multiplier(Family::Ore), 100);
            assert_eq!(c.multiplier(Family::Money), 4);
            assert_eq!(c.catch_multiplier(CatchClass::Bug), 3);
            assert_eq!(c.catch_multiplier(CatchClass::Fish), 7);
            assert_eq!(c.catch_multiplier(CatchClass::Critter), 5);
            assert_eq!((c.pouches, c.breakables), (6, 9));
            assert!(!c.all_vanilla());
            assert!(w.is_empty(), "{w:?}");
        }

        #[test]
        fn bugs_and_fish_parse_like_the_family_keys() {
            let d = Config::default();
            // Same case-insensitivity, same range, same "keep the default and
            // warn" on a bad value as Foraging..Ore.
            let (c, w) = parse("[Harvest]\nBUGS=2\nfIsH=100\ncRiTtErS=3\n");
            assert!(w.is_empty(), "{w:?}");
            assert_eq!((c.bugs, c.fish, c.critters), (2, MULT_MAX, 3));

            let (c, w) = parse("[Harvest]\nBugs=0\nFish=101\nCritters=0\n");
            assert_eq!(w.len(), 3, "{w:?}");
            assert_eq!((c.bugs, c.fish, c.critters), (d.bugs, d.fish, d.critters));

            // They are not gather families, so they never make `all_vanilla`
            // false: the record-loader hook has nothing to do for them.
            let (c, w) = parse("[Harvest]\nBugs=10\nFish=10\nCritters=10\n");
            assert!(w.is_empty(), "{w:?}");
            assert!(c.all_vanilla(), "Bugs/Fish/Critters never touch a gimmick record");
        }

        /// `Pouches` and `Breakables` parse like every other multiplier here
        /// (and are handed to dispatch, which is tested with the mailbox).
        #[test]
        fn pouches_and_breakables_parse_like_the_family_keys() {
            let d = Config::default();
            let (c, w) = parse("[Harvest]\npOuChEs=2\nBREAKABLES=100\n");
            assert!(w.is_empty(), "{w:?}");
            assert_eq!((c.pouches, c.breakables), (2, MULT_MAX));
            assert!(c.all_vanilla(), "they never touch a gimmick record either");
            let (c, w) = parse("[Harvest]\nPouches=0\nBreakables=101\n");
            assert_eq!(w.len(), 2, "{w:?}");
            assert_eq!((c.pouches, c.breakables), (d.pouches, d.breakables));
            // Under another header they are not ours: [Dispatch] is where they
            // used to be, and `crate::migrate::move_drop_keys` moves them.
            let (c, w) = parse("[Dispatch]\nPouches=5\n[Harvest]\n");
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.pouches, 1);
        }

        /// The water well is a `Foraging` record, so the `Foraging` key is what
        /// reaches it - there is no separate well key, and an `Ingredients` key
        /// (a family that never shipped) is an unknown key like any other.
        #[test]
        fn the_well_is_reached_through_foraging() {
            assert_eq!(
                desert_core::collect::family_by_name("gimmick_well_0001_parts01"),
                Some(Family::Foraging)
            );
            let (c, w) = parse("[Harvest]\nForaging=3\n");
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(c.multiplier(Family::Foraging), 3);
            assert!(!c.all_vanilla());

            let (c, w) = parse("[Harvest]\nIngredients=3\n");
            assert_eq!(c, Config::default());
            assert_eq!(w.len(), 1, "{w:?}");
            assert!(w[0].starts_with("unknown key"), "{w:?}");
        }

        #[test]
        fn case_insensitive_keys() {
            let (c, w) = parse("[hArVeSt]\nENABLED=0\nforaging=3\nOrE=4\ndUmPtAbLe=1\n");
            assert!(!c.enabled);
            assert!(c.dump_table);
            assert_eq!(c.foraging, 3);
            assert_eq!(c.ore, 4);
            assert!(w.is_empty(), "{w:?}");
        }

        #[test]
        fn bad_values_keep_the_default_and_warn() {
            let d = Config::default();
            let (c, w) =
                parse("[Harvest]\nForaging=0\nLogging=101\nMining=abc\nOre=-2\nJunk=1\nnoequals\n");
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
            let (c, w) = parse("[Harvest]\nForaging=1\nLogging=100\n");
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
        fn schema_ranges_are_the_ones_parse_enforces() {
            let s = schema();
            for key in ["Foraging", "Logging", "Mining", "Ore", "Money", "Bugs", "Fish", "Critters", "Pouches", "Breakables"] {
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
            let (c, w) = parse("[Harvest]\nForaging=1\nLogging=100\n");
            assert!(w.is_empty(), "{w:?}");
            assert_eq!((c.foraging, c.logging), (MULT_MIN, MULT_MAX));
            let (c, w) = parse("[Harvest]\nForaging=0\nLogging=101\n");
            assert_eq!(w.len(), 2, "{w:?}");
            assert_eq!(c, Config::default());
        }

        /// A player reading "plants, fruit, berries, mushrooms, crops" would not
        /// guess that water is in there, so the Foraging help has to say it covers
        /// the water well - and there must be no separate well or `Ingredients`
        /// field for them to go looking for instead.
        #[test]
        fn the_foraging_help_names_the_water_well() {
            let s = schema();
            let help = s
                .field("Foraging")
                .and_then(|f| f.help.clone())
                .unwrap_or_else(|| panic!("the menu must offer Foraging, with help"));
            assert!(help.contains("water well"), "{help}");
            assert!(s.field("Ingredients").is_none(), "Ingredients never shipped; it must not be offered");
        }

        /// The one balance lever in the section, and the one whose help has to be
        /// read before it is moved: a player who reads "money multiplier" will
        /// assume it covers everything that pays money, so the help has to name
        /// what it does not reach, and it has to say the 1 is a decision rather
        /// than an oversight.
        #[test]
        fn the_money_help_says_what_it_multiplies_and_what_it_does_not() {
            let s = schema();
            let f = s
                .field("Money")
                .unwrap_or_else(|| panic!("the menu must offer Money, with help"));
            let help = f.help.clone().unwrap_or_default();
            assert!(help.contains("money lying in the world"), "{help}");
            for missed in ["enemy drops", "quest rewards", "pouches", "chests", "shop prices"] {
                assert!(help.contains(missed), "the help must say it does not touch {missed}: {help}");
            }
            assert!(help.contains("default is 1"), "the default has to read as deliberate: {help}");
            assert_eq!(f.kind.default_text(), "1");

            // A family like the other four, drawn in the same list: no heading of
            // its own, and between Ore and the two catch multipliers.
            assert_eq!(f.heading, None, "Money continues the Yield multipliers: group");
            assert!(!f.same_line);
            let at = |key: &str| s.fields.iter().position(|f| f.key == key);
            assert!(at("Ore") < at("Money") && at("Money") < at("Bugs"));
        }
    }
}
