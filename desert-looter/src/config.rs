//! `DesertLooter.ini` beside the game exe. Missing file or key => defaults.
//!
//! Only Desert Looter's own keys live here; the ini tokeniser, the truthy
//! spellings and the virtual-key name table are shared in `desert_core::ini`.

use desert_core::collect::Family;
use desert_core::ini::{self, Line};
use desert_core::schema::{Field, Kind, Preset, Section};

/// The two millisecond keys' accepted range, in one place: `parse` refuses a
/// value outside it and [`schema`] hands the same bounds to the menu, so the
/// overlay can never write a number this parser would throw away.
pub const MS_RANGE: (u32, u32) = (100, 60_000);

/// `StackLimit`'s accepted range, used by `parse` and [`schema`] alike.
pub const STACK_LIMIT_RANGE: (u32, u32) = (10, 1_000_000);

/// The furthest `GatherRange` may reach. The parser accepts anything above 0
/// up to this; the menu's slider starts at [`GATHER_RANGE_MENU_MIN`] because a
/// slider needs a bottom end and sub-metre gathering is not useful.
pub const GATHER_RANGE_MAX: f32 = 50.0;
pub const GATHER_RANGE_MENU_MIN: f32 = 1.0;

/// `ScanRange`'s slider bounds. The parser accepts any positive radius - a
/// hand-edited 500 still works - so these are the menu's ends, not a limit.
pub const SCAN_RANGE_MENU: (f32, f32) = (1.0, 200.0);

/// The top of `BagTab`'s menu input. `actors::MAX_INVENTORY_TABS` is 64 as a
/// sanity ceiling on what the game hands back, but a real character carries a
/// handful of tabs, so the menu stops well short of it - and that const cannot
/// be named from here anyway: this file is always compiled and `actors` is
/// `#[cfg(windows)]`. The parser has no upper bound of its own; a hand-edited
/// larger id still works.
const BAG_TAB_MENU_MAX: i64 = 15;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub debug: bool,
    /// Log every item the game hands the player, whether or not the plugin
    /// caused the pickup. For measuring gathering yields; capped per session.
    pub log_received: bool,
    /// Survey radius in game metres.
    pub scan_range: f32,
    /// Radius within which a gather node is picked, in game metres.
    pub gather_range: f32,
    /// Start with automatic gathering on.
    pub auto_gather: bool,
    /// Also target nodes the game has not armed with an interaction object.
    pub gather_unarmed: bool,
    /// Pick up basic ground items (ore chunks, `item_basic_*` records).
    pub gather_items: bool,
    /// Also pick up dropped gear (`item_basic_equip_*`).
    pub gather_gear: bool,
    /// Per-family switches for gather nodes, the same gate `gather_items` and
    /// `gather_gear` are for ground items. All four on = the old behaviour.
    pub gather_foraging: bool,
    pub gather_logging: bool,
    pub gather_mining: bool,
    pub gather_ore: bool,
    /// Assumed per-stack ceiling used only when the bag is full: a pickup that
    /// would push an existing stack past this is refused.
    pub stack_limit: u32,
    /// Inventory tab id treated as the bag (tab 1 on build 25116796, the one
    /// the HUD shows as n/132); None = the largest tab.
    pub bag_tab: Option<i16>,
    /// Minimum time between two automatic sends.
    pub gather_interval_ms: u32,
    /// After sending for a node, leave it alone this long before retrying.
    pub node_cooldown_ms: u32,
    /// Virtual-key codes.
    pub key_toggle: u16,
    pub key_scan: u16,
    pub key_gather: u16,
    pub key_record: u16,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            debug: false,
            log_received: false,
            scan_range: 40.0,
            gather_range: 6.0,
            auto_gather: false,
            gather_unarmed: true,
            gather_items: true,
            gather_gear: false,
            gather_foraging: true,
            gather_logging: true,
            gather_mining: true,
            gather_ore: true,
            bag_tab: Some(1),
            stack_limit: 999,
            gather_interval_ms: 500,
            node_cooldown_ms: 8000,
            key_toggle: 0x79, // F10
            key_scan: 0x7A,   // F11
            key_gather: 0x78, // F9
            key_record: 0x76, // F7
        }
    }
}

impl Config {
    /// Whether gather nodes of this family are wanted. Called with the typed
    /// family from `desert_core::collect`, before anything stringifies it.
    pub fn allows_family(&self, f: Family) -> bool {
        match f {
            Family::Foraging => self.gather_foraging,
            Family::Logging => self.gather_logging,
            Family::Mining => self.gather_mining,
            Family::Ore => self.gather_ore,
        }
    }
}

// ---------------------------------------------------------------------------
// The menu schema
// ---------------------------------------------------------------------------

/// The `.asi` the overlay looks for before it lets this section be edited.
const MODULE: &str = "DesertLooter.asi";

/// One field with no heading, on its own row. The three decorations the
/// builders below add are the exceptions, so the common case stays one line.
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

/// Draw a dim `heading` line before this field.
fn under(heading: &str, mut field: Field) -> Field {
    field.heading = Some(heading.to_string());
    field
}

/// Draw this field on the same row as the one before it.
fn beside(mut field: Field) -> Field {
    field.same_line = true;
    field
}

/// A preset button: the four gather families plus `GatherItems`, and nothing
/// else. Someone who has tuned their ranges keeps them.
fn preset(label: &str, hint: &str, families: (bool, bool, bool, bool)) -> Preset {
    let (foraging, logging, mining, ore) = families;
    let flag = |b: bool| if b { "1" } else { "0" }.to_string();
    Preset {
        label: label.to_string(),
        hint: Some(hint.to_string()),
        set: vec![
            ("GatherForaging".to_string(), flag(foraging)),
            ("GatherLogging".to_string(), flag(logging)),
            ("GatherMining".to_string(), flag(mining)),
            ("GatherOre".to_string(), flag(ore)),
            ("GatherItems".to_string(), "1".to_string()),
        ],
    }
}

/// What Desert Overlay draws for `DesertLooter.ini`: the keys, in menu order,
/// with the labels, ranges and help the menu shows. Written to
/// `DesertLooter.overlay.ini` at every launch (see `lib.rs`); the overlay reads
/// that file and needs no knowledge of this plugin at all.
///
/// `Debug`, `LogReceived` and `BagTab` are here too, grouped at the bottom
/// under `Diagnostics:`. They used to be left out as "diagnostics and a
/// game-build detail", which stopped being tenable once this schema became
/// what the plugin writes its own ini from (`schema::create_ini_if_missing`):
/// a key that is not named here is missing from the generated file as well as
/// from the menu, and a player working from that file would never find out it
/// existed.
///
/// Every default is `Config::default()` and every range is the const `parse`
/// itself uses, so the menu can only produce values this parser accepts. Both
/// facts are tested below.
pub fn schema() -> Section {
    let d = Config::default();
    Section {
        title: "Desert Looter".to_string(),
        ini: crate::INI_NAME.to_string(),
        module: Some(MODULE.to_string()),
        // Before Desert Gatherer's 20: the looter is the mod most people came
        // for, and its section is the one that should be at the top.
        order: 10,
        notice: None,
        presets_label: Some("Presets (what auto-loot picks up):".to_string()),
        presets: vec![
            preset(
                "Everything",
                "All four gather families, plus ground items.",
                (true, true, true, true),
            ),
            preset(
                "Plants only",
                "Foraging only: plants, fruit, berries, mushrooms, crops.",
                (true, false, false, false),
            ),
            preset(
                "Wood only",
                "Logging only: firewood from felled trees.",
                (false, true, false, false),
            ),
            preset(
                "Rock and ore only",
                "Mining and Ore Nodes: rocks, veins, ore deposits.",
                (false, false, true, true),
            ),
        ],
        fields: vec![
            f(
                "Enabled",
                "Enabled",
                Kind::Bool { default: d.enabled },
                "Master switch. 0 = the plugin is idle: no gathering, no hotkeys, nothing logged.",
            ),
            f(
                "AutoGather",
                "Auto gather",
                Kind::Bool { default: d.auto_gather },
                "1 = automatic gathering is on from the start (the toggle key flips it).",
            ),
            under(
                "Gather families:",
                f(
                    "GatherForaging",
                    "Foraging",
                    Kind::Bool { default: d.gather_foraging },
                    "Plants, fruit, berries, mushrooms, crops.",
                ),
            ),
            beside(f(
                "GatherLogging",
                "Logging",
                Kind::Bool { default: d.gather_logging },
                "Firewood cut from felled trees (firewood_*).",
            )),
            beside(f(
                "GatherMining",
                "Mining",
                Kind::Bool { default: d.gather_mining },
                "The collect_mine family: mine_* rocks and veins, breakable stalactites.",
            )),
            beside(f(
                "GatherOre",
                "Ore",
                Kind::Bool { default: d.gather_ore },
                "The collect_ore family: ore_* deposits and sulfur stone, separate from Mining.",
            )),
            f(
                "GatherItems",
                "Ground items",
                Kind::Bool { default: d.gather_items },
                "1 = also pick up basic items lying on the ground (ore chunks and other item_basic_* drops).",
            ),
            beside(f(
                "GatherGear",
                "Dropped gear",
                Kind::Bool { default: d.gather_gear },
                "1 = dropped weapons and armour (item_basic_equip_*) are picked up too. Each one costs a bag slot.",
            )),
            beside(f(
                "GatherUnarmed",
                "Unarmed nodes",
                Kind::Bool { default: d.gather_unarmed },
                "1 = also target the nodes the game has not armed with interaction data, which is what lets auto mode mine a whole vein.",
            )),
            f(
                "ScanRange",
                "Scan range",
                Kind::Float {
                    default: d.scan_range,
                    min: SCAN_RANGE_MENU.0,
                    max: SCAN_RANGE_MENU.1,
                    format: Some("%.0f m".to_string()),
                },
                "Survey radius in game metres.",
            ),
            f(
                "GatherRange",
                "Gather range",
                Kind::Float {
                    default: d.gather_range,
                    min: GATHER_RANGE_MENU_MIN,
                    max: GATHER_RANGE_MAX,
                    format: Some("%.1f m".to_string()),
                },
                "The gather key and auto mode pick the nearest node within this many metres. 6 m is confirmed to work.",
            ),
            f(
                "GatherInterval",
                "Gather interval (ms)",
                ms(i64::from(d.gather_interval_ms), 50),
                "Milliseconds between two automatic sends. Nodes and chunks vanish within about 100 ms of a send.",
            ),
            f(
                "NodeCooldown",
                "Node cooldown (ms)",
                ms(i64::from(d.node_cooldown_ms), 500),
                "After sending for a node, leave it alone this long before trying it again.",
            ),
            f(
                "StackLimit",
                "Stack limit",
                Kind::Int {
                    default: i64::from(d.stack_limit),
                    min: i64::from(STACK_LIMIT_RANGE.0),
                    max: i64::from(STACK_LIMIT_RANGE.1),
                    step: 10,
                    slider: false,
                    format: None,
                },
                "When the bag is full a node whose yield is already stacked is still gathered, unless the stack would pass this ceiling.",
            ),
            under(
                "Keys:",
                key("KeyToggle", "Toggle auto gather", d.key_toggle,
                    "Turns automatic gathering on and off. Avoid keys the game already uses."),
            ),
            key("KeyScan", "Survey", d.key_scan,
                "Writes the nodes and items in scan range to the log."),
            key("KeyGather", "Gather nearest", d.key_gather,
                "Gather the nearest node: one node per press, whatever the auto state."),
            key("KeyRecord", "Record events (debug)", d.key_record,
                "Toggles recording of every event the game queues, capped at 300."),
            under(
                "Diagnostics:",
                f(
                    "Debug",
                    "Debug",
                    Kind::Bool { default: d.debug },
                    "The very verbose survey: the first Survey press after this is on dumps hundreds of lines to the log. For working out why something is not picked up, not for playing.",
                ),
            ),
            beside(f(
                "LogReceived",
                "Log received items",
                Kind::Bool { default: d.log_received },
                "Logs every item the game hands the player as [recv] item <key> x<count>, whether or not this plugin caused it, capped at 500 a session. This is how a gathering yield is actually measured.",
            )),
            f(
                "BagTab",
                "Bag tab",
                Kind::Int {
                    default: i64::from(d.bag_tab.unwrap_or(-1)),
                    min: -1,
                    max: BAG_TAB_MENU_MAX,
                    step: 1,
                    slider: false,
                    format: None,
                },
                "Which inventory tab counts as the bag for the full check. 1 is the right answer on build 25116796; -1 means auto, i.e. whichever tab has the largest capacity.",
            ),
        ],
    }
}

/// One of the two millisecond inputs: same range, different step.
fn ms(default: i64, step: i64) -> Kind {
    Kind::Int {
        default,
        min: i64::from(MS_RANGE.0),
        max: i64::from(MS_RANGE.1),
        step,
        slider: false,
        format: None,
    }
}

/// One of the four hotkeys. The default is a virtual-key code in `Config`, and
/// the schema wants the name, so it is spelled back through the same table
/// `parse` reads it with - the two can never disagree about what F10 is.
fn key(key_name: &str, label: &str, vk: u16, help: &str) -> Field {
    let default = ini::key_names()
        .iter()
        .find(|n| ini::vk_from_name(n) == Some(vk))
        .map_or_else(String::new, |n| (*n).to_string());
    f(key_name, label, Kind::Key { default }, help)
}

/// Parse ini text. Unknown keys are reported back so they can be logged.
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
        let bool_of = ini::parse_bool;
        match k.to_ascii_lowercase().as_str() {
            "enabled" => cfg.enabled = bool_of(v),
            "debug" => cfg.debug = bool_of(v),
            "logreceived" => cfg.log_received = bool_of(v),
            "scanrange" => match v.parse::<f32>() {
                Ok(f) if f > 0.0 => cfg.scan_range = f,
                _ => warnings.push(format!("ScanRange: bad value {v:?}, keeping {}", cfg.scan_range)),
            },
            "autogather" => cfg.auto_gather = bool_of(v),
            "gatherunarmed" => cfg.gather_unarmed = bool_of(v),
            "gatheritems" => cfg.gather_items = bool_of(v),
            "gathergear" => cfg.gather_gear = bool_of(v),
            "gatherforaging" => cfg.gather_foraging = bool_of(v),
            "gatherlogging" => cfg.gather_logging = bool_of(v),
            "gathermining" => cfg.gather_mining = bool_of(v),
            "gatherore" => cfg.gather_ore = bool_of(v),
            "stacklimit" => match v.parse::<u32>() {
                Ok(n) if (STACK_LIMIT_RANGE.0..=STACK_LIMIT_RANGE.1).contains(&n) => cfg.stack_limit = n,
                _ => warnings.push(format!("StackLimit: bad value {v:?}, keeping {}", cfg.stack_limit)),
            },
            "bagtab" => match v.parse::<i16>() {
                Ok(id) if id >= 0 => cfg.bag_tab = Some(id),
                Ok(_) => cfg.bag_tab = None,
                Err(_) => warnings.push(format!("BagTab: bad value {v:?}, keeping auto")),
            },
            "gatherinterval" | "nodecooldown" => match v.parse::<u32>() {
                Ok(ms) if (MS_RANGE.0..=MS_RANGE.1).contains(&ms) => {
                    if k.eq_ignore_ascii_case("gatherinterval") {
                        cfg.gather_interval_ms = ms;
                    } else {
                        cfg.node_cooldown_ms = ms;
                    }
                }
                _ => warnings.push(format!(
                    "{k}: bad value {v:?} ({}..{} ms), keeping default",
                    MS_RANGE.0, MS_RANGE.1
                )),
            },
            "gatherrange" => match v.parse::<f32>() {
                Ok(f) if f > 0.0 && f <= GATHER_RANGE_MAX => cfg.gather_range = f,
                _ => warnings.push(format!("GatherRange: bad value {v:?}, keeping {}", cfg.gather_range)),
            },
            "keytoggle" | "keyscan" | "keygather" | "keyrecord" => match ini::vk_from_name(v) {
                Some(vk) if k.eq_ignore_ascii_case("keytoggle") => cfg.key_toggle = vk,
                Some(vk) if k.eq_ignore_ascii_case("keyscan") => cfg.key_scan = vk,
                Some(vk) if k.eq_ignore_ascii_case("keyrecord") => cfg.key_record = vk,
                Some(vk) => cfg.key_gather = vk,
                None => warnings.push(format!("{k}: unknown key name {v:?}, keeping previous")),
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
    fn parses_and_warns() {
        let (c, w) = parse("; c\n[DesertLooter]\nEnabled=0\nDebug=1\nLogReceived=1\nScanRange=25.5\nGatherRange=4\nAutoGather=1\nGatherInterval=250\nNodeCooldown=5\nKeyToggle=F5\nKeyScan=nope\nKeyGather=F8\nJunk=1\n");
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
        let (c, w) = parse("GatherForaging=1\nGatherLogging=0\nGatherMining=no\nGatherOre=on\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(c.allows_family(Family::Foraging));
        assert!(!c.allows_family(Family::Logging));
        assert!(!c.allows_family(Family::Mining));
        assert!(c.allows_family(Family::Ore));
    }

    // -----------------------------------------------------------------------
    // The menu schema
    //
    // The schema is read by a different program (Desert Overlay) that writes
    // this plugin's ini back. These tests are the contract: every key the
    // schema names is one `parse` accepts, every default it declares is the
    // one `parse` would have produced anyway, and every range it hands the
    // menu is the range `parse` enforces. Break one and the menu starts
    // writing values this plugin silently throws away.
    // -----------------------------------------------------------------------

    use desert_core::schema as sch;

    #[test]
    fn schema_defaults_parse_back_to_the_default_config() {
        let (cfg, w) = parse(&sch::render_ini_defaults(&schema(), ""));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg, Config::default());
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
            let (cfg, w) = parse(
                &p.set.iter().map(|(k, v)| format!("{k}={v}\n")).collect::<String>(),
            );
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
            let text: String = p.set.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
            let (c, _) = parse(&text);
            assert_eq!(
                (c.gather_foraging, c.gather_logging, c.gather_mining, c.gather_ore),
                want,
                "{label}"
            );
        }
    }

    #[test]
    fn the_schema_names_the_files_this_crate_ships() {
        let s = schema();
        assert_eq!(s.ini, crate::INI_NAME);
        assert_eq!(s.module.as_deref(), Some("DesertLooter.asi"));
        assert_eq!(s.schema_file_name(), "DesertLooter.overlay.ini");
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

        // A value at either end of every int range is one `parse` keeps, and
        // one past it is one it refuses: the menu's stops are the real stops.
        let (c, w) = parse("GatherInterval=100\nNodeCooldown=60000\nStackLimit=1000000\n");
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.gather_interval_ms, MS_RANGE.0);
        assert_eq!(c.node_cooldown_ms, MS_RANGE.1);
        assert_eq!(c.stack_limit, STACK_LIMIT_RANGE.1);
        let (c, w) = parse("GatherInterval=99\nNodeCooldown=60001\nStackLimit=9\nGatherRange=50.1\n");
        assert_eq!(w.len(), 4, "{w:?}");
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
        // The heading is what groups the three at the bottom of the section.
        assert_eq!(
            s.field("Debug").and_then(|f| f.heading.clone()),
            Some("Diagnostics:".to_string())
        );

        // `BagTab` opens at the default tab, and the bottom of its range is
        // the "auto" the parser turns into `None`: a value the menu can
        // actually produce, written and read back through the same two paths
        // the overlay uses.
        let field = s.field("BagTab").unwrap_or_else(|| panic!("no BagTab"));
        assert_eq!(field.kind.default_text(), "1");
        let auto = field.kind.normalize("-1").unwrap_or_else(|| panic!("-1 is out of range"));
        assert_eq!(auto, "-1");
        let (c, w) = parse(&format!("BagTab={auto}\n"));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.bag_tab, None);
        // One past the auto end is not something the menu can write.
        assert_eq!(field.kind.normalize("-2"), None);
        assert_eq!(field.kind.normalize(&(BAG_TAB_MENU_MAX + 1).to_string()), None);

        let (c, w) = parse("Debug=1\nLogReceived=1\nBagTab=0\n");
        assert!(w.is_empty(), "{w:?}");
        assert!(c.debug && c.log_received);
        assert_eq!(c.bag_tab, Some(0));
    }

    /// Prints the rendered schema. `cargo test -- --ignored --nocapture
    /// show_schema` is how the file's exact text gets read by a human.
    #[test]
    #[ignore]
    fn show_schema() {
        println!("{}", sch::render(&schema(), ""));
    }
}
