//! The auto-loot half's keys in the `[Harvest]` section of `DesertTooling.ini`
//! beside the game exe. Missing file, missing section or missing key =>
//! defaults.
//!
//! The section itself is walked once, by `crate::config::parse`, which owns
//! the two keys both halves read (`Enabled`, `Debug`) and hands every other
//! key to [`Config::apply_key`] here and then to the yield half's. The ini
//! tokeniser, the truthy spellings and the virtual-key name table are shared in
//! `desert_core::ini`.

use desert_core::collect::Family;
use desert_core::ini;
use desert_core::schema::{Field, Kind, Preset, Tab};

use crate::config::{beside, f, on, under};

/// The two millisecond keys' accepted range, in one place: `apply_key` refuses
/// a value outside it and [`fields`] hands the same bounds to the menu, so the
/// overlay can never write a number this parser would throw away.
pub const MS_RANGE: (u32, u32) = (100, 60_000);

/// `StackLimit`'s accepted range, used by `apply_key` and [`fields`] alike.
pub const STACK_LIMIT_RANGE: (u32, u32) = (10, 1_000_000);
/// `SurveyLines`' accepted range. The floor is where a listing stops being
/// one; the ceiling is a guard on the shared log, not a recommendation.
pub const SURVEY_LINES_RANGE: (u32, u32) = (16, 2_000);
/// `Hunting`'s range. Deliberately the same `1..=100` as every yield
/// multiplier: the two halves' sliders mean the same thing to a player, and
/// a different ceiling here would only be a second number to remember. The
/// floor is 1 rather than 0 for the same reason it is there - a 0 would zero
/// out a carcass rather than leave it alone.
pub const HUNTING_RANGE: (u32, u32) = (1, 100);
/// `LootLogMinTier`'s range: `0` is the vanilla item log, `1..=5` a minimum
/// `ItemInfo` tier, `6` above every tier (no line but money, if that is on).
pub const LOOT_LOG_TIER_RANGE: (u32, u32) = (0, 6);

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
pub(crate) const BAG_TAB_MENU_MAX: i64 = 15;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub debug: bool,
    /// Log every item the game hands the player, whether or not the plugin
    /// caused the pickup. For measuring gathering yields; capped per session.
    pub log_received: bool,
    /// Survey radius in game metres.
    pub scan_range: f32,
    /// How many actor lines one F11 survey may print. It was a hard-coded 64
    /// until 2026-09-13, and because the listing was distance-sorted that
    /// meant "the 64 nearest actors" - a window a few metres wide with a heap
    /// of smashed pottery underfoot, which silently cut the very actors the
    /// survey exists to show and cost three investigations
    /// (`docs/src/dev/open-questions.md`, "Survey truncation"). The listing
    /// is now ordered kind-first (gather nodes, then items, then the rest)
    /// and says how many lines it cut, so this is a budget on the shared
    /// log, not a filter on what is seen. The kind census at the foot of the
    /// survey was never truncated and still is not.
    pub survey_lines: u32,
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
    /// `gather_gear` are for ground items. All four on = the old behaviour;
    /// the fifth family, `gather_money`, is the exception and says why.
    pub gather_foraging: bool,
    pub gather_logging: bool,
    pub gather_mining: bool,
    pub gather_ore: bool,
    /// The fifth family, and the only one that starts off. The placed coin
    /// props (`gimmick_item_common_coin_0001`, key 1000183, and its siblings)
    /// surveyed as `Kind::Inert` on 2026-09-13: they carry neither an
    /// interaction object nor an instance object, and they were classified
    /// that way only because no family covered them. With `Family::Money`
    /// they classify `Kind::Unarmed` instead, so `gather_unarmed` - on by
    /// default - plus this switch would forge a pickup at a coin exactly the
    /// way the looter already forges one at an ore dropping. That has never
    /// been tried in game, which is why the default is 0; 0 also keeps the
    /// upgrade behaviour-preserving, which `VERSIONING.md` requires of a
    /// MINOR. This is only the auto-loot switch: how much a coin pays is the
    /// yield half's `Money` multiplier, which applies to hand pickups too.
    pub gather_money: bool,
    /// Catch insects within `GatherRange`. Not a gather family: it is a
    /// different game event (`TrocTrPushCharacterToInventoryOnceTimer`) sent
    /// at a different kind of actor, so it has its own switch.
    pub gather_bugs: bool,
    /// Catch fish within `GatherRange`. The same game event as `gather_bugs`
    /// at the same kind of actor, told apart only by the creature's class
    /// byte (`actors::catch_class`), so it gets its own switch rather than
    /// riding on that one.
    pub gather_fish: bool,
    /// Catch the small animals that go into the bag (lizards, small birds,
    /// rats, frogs, ...) within `GatherRange`. The same game event as
    /// `gather_bugs`; a creature is a critter only when its species record
    /// says so (`desert_core::species_table`, `actors::catch_class`), never
    /// by class byte, so ducks and raccoons are never one.
    pub gather_critters: bool,
    /// Loot animal carcasses within `GatherRange`, and the third switch that
    /// starts **off**. Skinning is a third game event again
    /// (`TrocTrProcessLootingDeadDropOnceTimer`, a 7-byte payload carrying
    /// only the carcass's entity id) whose handler grants the loot in the same
    /// tick with no animation, no distance check and nothing to duplicate -
    /// the granted rows are erased from the carcass, so a second event at the
    /// same corpse gives nothing
    /// (`docs/findings/2026-09-15-skinning.md` sections 2 and 3). It is off by
    /// default for the `gather_money` reason and no other: forging this event
    /// has never been tried in game, and a MINOR may not change behaviour the
    /// player did not ask for.
    pub gather_carcass: bool,
    /// Search dead hostile people and monsters within `GatherRange` (the
    /// species table's `Body` class: bandits, criminals, monsters, bosses)
    /// with the same dead-drop event `gather_carcass` sends. Off by default.
    /// A body whose owner id is set (a quest trigger, or owned by someone) is
    /// skipped, the same gate the game's own loot interaction applies.
    pub gather_bodies: bool,
    /// Also search the bodies of people who were not enemies (the species
    /// table's `FriendlyBody` class: citizens, traders, mercenaries; the
    /// white-marker bodies). Needs `gather_bodies` too. Off by default.
    pub gather_friendly_bodies: bool,
    /// Yield multiplier for looted carcasses, `1..=100`, `1` = vanilla.
    ///
    /// The one lever in this section that changes an *amount* rather than what
    /// the looter aims at, and it lives in this half rather than the yield
    /// half because what it multiplies is a corpse being looted, not a node
    /// being gathered. It is also not a gather family and could not be one: carcass
    /// loot is not in `gimmickinfo`, so there is no record for the gatherer's
    /// table edit to reach (`docs/findings/2026-09-15-hunting-multiplier.md`).
    ///
    /// **Independent of [`Config::gather_carcass`]**, which is a targeting
    /// switch and not a yield one. `crate::looter::hunting` hooks the game's own loot
    /// grant, so this reaches a carcass however it was looted - by the gather
    /// key with `GatherCarcass=1`, or by skinning one by hand. The gate is the
    /// loot method, not who asked: only a grant whose mask includes method 0,
    /// the search/skin method, and only the rows inside it that carry that same
    /// method bit.
    ///
    /// Default 1 for the `GatherMoney` reason: this changes how much the player
    /// gets rather than saving them trips, which is theirs to decide.
    pub hunting: u32,
    /// The HUD side item log's minimum item tier, `0..=6`; `0` = every line
    /// (vanilla), `6` = none. `crate::looter::itemlog` hooks the handler that
    /// builds each "Item xN" line and drops the ones below this; see
    /// `crate::looter::logfilter` for the policy. Nothing else is touched:
    /// the item still lands in the bag and quests still see it.
    pub loot_log_min_tier: u8,
    /// With a minimum tier set, still show currency (`itemType` 32: coins,
    /// camp funds) whatever its tier. Default on.
    pub loot_log_money: bool,
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
            survey_lines: 200,
            gather_range: 6.0,
            auto_gather: false,
            gather_unarmed: true,
            gather_items: true,
            gather_gear: false,
            gather_foraging: true,
            gather_logging: true,
            gather_mining: true,
            gather_ore: true,
            gather_money: false,
            gather_bugs: true,
            gather_fish: true,
            gather_critters: true,
            gather_carcass: false,
            gather_bodies: false,
            gather_friendly_bodies: false,
            hunting: 1,
            loot_log_min_tier: 0,
            loot_log_money: true,
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

/// The water well's bucket, `gimmick_well_0001_parts01`. It is a `Foraging`
/// record and the multiplier reaches it like any other; the **looter** must
/// never send a `PickUpItem` at it. Tried in game on 2026-09-13, build
/// 25246367 (`analysis/logs/DesertTooling-2026-09-13-well-coin-bugs.log`,
/// `t=197`): F9 at a well forged the pickup, the player received `22008 x5`,
/// the bucket actor was gone 0.1 s later and **the well never produced another
/// one** - the game's own crank sequence is what respawns the bucket, and a
/// pickup delivered from outside that sequence leaves the well permanently
/// empty. That is worse than the duplication exploit the earlier comment
/// worried about; it is a broken world object. So this is a record-level
/// refusal, not a family switch: `GatherForaging` still means what it says
/// for every bush, and the well is the one Foraging record it never covers.
///
/// How it was reachable at all (two F11 surveys, same log's successor
/// `DesertTooling-2026-09-13-well-x15-x30.log`, `t=272` and `t=319`): the
/// bucket actor is **always present** - it classifies `Unarmed` while the
/// well is empty and `Gather` once the bucket is raised full. So it is
/// `GatherUnarmed=1`, the default, that put an *empty* well in front of F9,
/// and turning that off would only have narrowed the damage to full ones.
/// The refusal is by record, so neither switch matters for it.
pub const WELL_BUCKET_RECORD_KEY: u32 = 1001081;
/// The same record by name, for the case where the header's key did not read:
/// `actors::node_identity` falls back to the name to find the family, so the
/// refusal has to fall back the same way or an unreadable key would let the
/// one record this exists for straight through.
pub const WELL_BUCKET_RECORD_NAME: &str = "gimmick_well_0001_parts01";

/// Whether the looter must refuse to forge a pickup at this record, whatever
/// the family switches say. Either identifier is enough on its own. The set is
/// one record today and is kept as a predicate so a second multi-step gimmick
/// has somewhere to go that is not a new `if` in `nearest_gather`.
pub fn never_forge_pickup(record_key: Option<u32>, record_name: Option<&str>) -> bool {
    record_key == Some(WELL_BUCKET_RECORD_KEY) || record_name == Some(WELL_BUCKET_RECORD_NAME)
}

impl Config {
    /// Whether gather nodes of this family are wanted. Called with the typed
    /// family from `desert_core::collect`, before anything stringifies it.
    /// [`never_forge_pickup`] is checked **before** this, on the record key,
    /// and wins: the water well is a `Foraging` record this switch does not
    /// reach, and the constant's doc comment says why.
    pub fn allows_family(&self, f: Family) -> bool {
        match f {
            // Foraging includes the water well (`gimmick_well_0001_parts01`,
            // key 1001081) for the *multiplier*; for the looter that record
            // is refused one step earlier by `never_forge_pickup`. The reason
            // the two disagree is in that constant's doc comment - a forged
            // pickup at the bucket breaks the well - and the yield multiplier
            // is independent of all of it: it edits the record's block at
            // load time and never consults the looter.
            Family::Foraging => self.gather_foraging,
            Family::Logging => self.gather_logging,
            Family::Mining => self.gather_mining,
            Family::Ore => self.gather_ore,
            // The one family that is off unless asked for: a forged pickup at
            // a coin prop has never been tried in game. See `gather_money`.
            Family::Money => self.gather_money,
        }
    }
}

// ---------------------------------------------------------------------------
// The menu schema
// ---------------------------------------------------------------------------

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

/// The label above the preset row on the Harvest tab.
pub(crate) fn presets_label() -> String {
    "Presets (what auto-loot picks up):".to_string()
}

/// The preset buttons: each sets the four gather families plus `GatherItems`
/// and nothing else. No balance lever (`GatherMoney`, `Hunting`, any yield
/// multiplier) is ever set by a button.
pub(crate) fn presets() -> Vec<Preset> {
    vec![
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
    ]
}

/// This half's fields for the `[Harvest]` menu schema (`crate::config::schema`),
/// in menu order, with the labels, ranges and help the menu shows. `Enabled`
/// and `Debug` are not here: both halves read them, so the root owns them.
///
/// `LogReceived`, `SurveyLines` and `BagTab` are marked [`Tab::Debug`] so the
/// menu draws them on its shared Debug tab rather than on the Harvest page;
/// the four hotkeys are marked [`Tab::Settings`] for the same reason, so that
/// every key a player binds is in one place. They used to be left out as
/// "diagnostics and a game-build detail", which stopped being tenable once the
/// schema became what the plugin writes its own ini from
/// (`schema::create_ini_if_missing_all`): a key that is not named here is
/// missing from the generated file as well as from the menu, and a player
/// working from that file would never find out it existed.
///
/// Every default is `Config::default()` and every range is the const
/// [`Config::apply_key`] itself uses, so the menu can only produce values this
/// parser accepts. Both facts are tested in `crate::config`.
pub(crate) fn fields() -> Vec<Field> {
    let d = Config::default();
    vec![
        f(
            "AutoGather",
            "Auto gather",
            Kind::Bool { default: d.auto_gather },
            "1 = automatic gathering is on from the start (the toggle key flips it).",
        ),
        under(
            "Gather nodes:",
            f(
                "GatherForaging",
                "Foraging",
                Kind::Bool { default: d.gather_foraging },
                "Plants, fruit, berries, mushrooms, crops. Never the water well: taking its \
                 bucket with a forged pickup leaves the well empty for good, so the plugin \
                 refuses that one record whatever this is set to.",
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
        beside(f(
            "GatherMoney",
            "Money",
            Kind::Bool { default: d.gather_money },
            "0 by default. 1 = also pick up the coins lying in the world (the placed coin props). \
             Untested in game: the pickup is forged the same way as at an ore chunk. This is the \
             auto-loot switch; the amount is the Money multiplier below.",
        )),
        beside(f(
            "GatherUnarmed",
            "Unarmed nodes",
            Kind::Bool { default: d.gather_unarmed },
            "1 = also target the nodes the game has not armed with interaction data, which is what lets auto mode mine a whole vein.",
        )),
        under(
            "Loose items:",
            f(
                "GatherItems",
                "Ground items",
                Kind::Bool { default: d.gather_items },
                "1 = also pick up basic items lying on the ground (ore chunks and other item_basic_* drops).",
            ),
        ),
        beside(f(
            "GatherGear",
            "Dropped gear",
            Kind::Bool { default: d.gather_gear },
            "1 = dropped weapons and armour (item_basic_equip_*) are picked up too. Each one costs a bag slot.",
        )),
        under(
            "Creatures:",
            f(
                "GatherBugs",
                "Insects",
                Kind::Bool { default: d.gather_bugs },
                "1 = catch insects within GatherRange; the game's steal check still applies.",
            ),
        ),
        beside(f(
            "GatherFish",
            "Fish",
            Kind::Bool { default: d.gather_fish },
            "1 = catch fish within GatherRange by hand, the same event as insects; the steal check still applies.",
        )),
        beside(f(
            "GatherCritters",
            "Critters",
            Kind::Bool { default: d.gather_critters },
            "1 = catch the small animals that go into the bag (lizards, small birds, rats, frogs, \
             toads, salamanders...) within GatherRange, the same event as insects. Animals you \
             carry and skin, like ducks and raccoons, are never caught.",
        )),
        under(
            "Bodies:",
            f(
                "GatherCarcass",
                "Animal carcasses",
                Kind::Bool { default: d.gather_carcass },
                "0 by default. 1 = loot animal carcasses within GatherRange, without the skinning \
                 animation. A third game event again, granting the same loot the animation would; \
                 a corpse already looted gives nothing. Animals only, not people. 0 by default: opt in.",
            ),
        ),
        beside(f(
            "GatherBodies",
            "Enemies",
            Kind::Bool { default: d.gather_bodies },
            "0 by default. 1 = search dead people and monsters for loot (bandits, soldiers, \
             bosses) within GatherRange with the gather key, the same event as skinning. Bodies \
             with a quest trigger are skipped.",
        )),
        beside(f(
            "GatherFriendlyBodies",
            "Friendly",
            Kind::Bool { default: d.gather_friendly_bodies },
            "0 by default. 1 = also search the bodies of people who were not enemies (citizens, \
             traders, mercenaries: the white-marker bodies). Needs GatherBodies=1. Off by default.",
        )),
        f(
            "Hunting",
            "Hunting",
            Kind::Int {
                default: i64::from(HUNTING_RANGE.0),
                min: i64::from(HUNTING_RANGE.0),
                max: i64::from(HUNTING_RANGE.1),
                step: 1,
                slider: true,
                format: Some("%dx".to_string()),
            },
            "How much a looted carcass gives, 1..100. 1 = vanilla. Works however you loot the \
             carcass - by the gather key with GatherCarcass=1, or by skinning one by hand - \
             because it hooks the game's own grant rather than this plugin's send. It scales \
             the amounts the game has already rolled for that one corpse, so nothing in any \
             table is edited and no other kind of loot in the game is touched. Like the \
             Money multiplier, it ships at 1 on purpose - it changes how much you get rather \
             than saving you trips.",
        ),
        under(
            "Item log:",
            f(
                "LootLogMinTier",
                "Minimum tier",
                Kind::Int {
                    default: i64::from(d.loot_log_min_tier),
                    min: i64::from(LOOT_LOG_TIER_RANGE.0),
                    max: i64::from(LOOT_LOG_TIER_RANGE.1),
                    step: 1,
                    slider: true,
                    format: None,
                },
                "Which pickups get an \"Item xN\" line in the item log at the side of the screen. \
                 0 = every line, as the game does. 1..5 = only items of at least that tier. \
                 6 = no lines at all (except money, below). Only the line is hidden: the item \
                 still goes into your bag and quests still count it. Takes effect at once.",
            ),
        ),
        f(
            "LootLogMoney",
            "Always show money",
            Kind::Bool { default: d.loot_log_money },
            "1 = coins and camp funds always get their line, whatever the minimum tier says.",
        ),
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
        // The four hotkeys live on the menu's shared Settings tab, under
        // this section's own title, so every key a player binds is in one
        // place. No "Keys:" heading any more: on that tab the section
        // title is the group header.
        on(Tab::Settings,
            key("KeyToggle", "Toggle auto gather", d.key_toggle,
                "Turns automatic gathering on and off. Avoid keys the game already uses.")),
        on(Tab::Settings,
            key("KeyScan", "Survey", d.key_scan,
                "Writes the nodes and items in scan range to the log.")),
        on(Tab::Settings,
            key("KeyGather", "Gather nearest", d.key_gather,
                "Gather the nearest node: one node per press, whatever the auto state.")),
        on(Tab::Settings,
            key("KeyRecord", "Record events (debug)", d.key_record,
                &format!(
                    "Toggles logging the events the game queues: up to {} lines per event type \
                     and {} in all, then it stops by itself and writes a census of everything \
                     it saw.",
                    crate::looter::recorder::RECORD_PER_DESC,
                    crate::looter::recorder::RECORD_CAP
                ))),
        // The diagnostics go to the shared Debug tab, where the section
        // title is the group header. `Debug` itself is the root's (both
        // halves read it) and comes first on that tab, so `LogReceived`
        // is drawn beside it; `SurveyLines` follows, then `BagTab`.
        on(Tab::Debug, beside(f(
            "LogReceived",
            "Log received items",
            Kind::Bool { default: d.log_received },
            "Logs every item the game hands the player as [recv] item <key> x<count>, whether or not this plugin caused it, capped at 500 a session. This is how a gathering yield is actually measured. Coin props are the exception: picking one up writes no line, so measure money with the bag count an F11 survey prints.",
        ))),
        on(
            Tab::Debug,
            f(
                "SurveyLines",
                "Survey lines",
                Kind::Int {
                    default: i64::from(d.survey_lines),
                    min: i64::from(SURVEY_LINES_RANGE.0),
                    max: i64::from(SURVEY_LINES_RANGE.1),
                    step: 16,
                    slider: false,
                    format: None,
                },
                "How many actor lines one F11 survey prints. Gather nodes and items are listed \
                 first, so raising this only ever adds the less interesting actors; the survey \
                 says how many it cut.",
            ),
        ),
        on(
            Tab::Debug,
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
            "Which inventory tab counts as the bag for the full check. 1 was right on build 25116796; -1 means auto, i.e. whichever tab has the largest capacity.",
            ),
        ),
    ]
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
/// `apply_key` reads it with - the two can never disagree about what F10 is.
fn key(key_name: &str, label: &str, vk: u16, help: &str) -> Field {
    let default = ini::key_names()
        .iter()
        .find(|n| ini::vk_from_name(n) == Some(vk))
        .map_or_else(String::new, |n| (*n).to_string());
    f(key_name, label, Kind::Key { default }, help)
}

impl Config {
    /// Apply one `key=value` from the `[Harvest]` section if it is one of this
    /// half's keys; `false` means "not mine", and the root parser then offers
    /// it to the yield half. `lower` is `k` in ASCII lower case. A bad value is
    /// pushed onto `warnings` and keeps the current value: the config always
    /// stays usable. `Enabled` and `Debug` never get here; the root owns them.
    pub(crate) fn apply_key(&mut self, lower: &str, k: &str, v: &str, warnings: &mut Vec<String>) -> bool {
        match lower {
            "logreceived" => self.log_received = ini::parse_bool(v),
            "scanrange" => match v.parse::<f32>() {
                Ok(f) if f > 0.0 => self.scan_range = f,
                _ => warnings.push(format!("ScanRange: bad value {v:?}, keeping {}", self.scan_range)),
            },
            "autogather" => self.auto_gather = ini::parse_bool(v),
            "gatherunarmed" => self.gather_unarmed = ini::parse_bool(v),
            "gatheritems" => self.gather_items = ini::parse_bool(v),
            "gathergear" => self.gather_gear = ini::parse_bool(v),
            "gatherforaging" => self.gather_foraging = ini::parse_bool(v),
            "gatherlogging" => self.gather_logging = ini::parse_bool(v),
            "gathermining" => self.gather_mining = ini::parse_bool(v),
            "gatherore" => self.gather_ore = ini::parse_bool(v),
            "gathermoney" => self.gather_money = ini::parse_bool(v),
            "gatherbugs" => self.gather_bugs = ini::parse_bool(v),
            "gatherfish" => self.gather_fish = ini::parse_bool(v),
            "gathercritters" => self.gather_critters = ini::parse_bool(v),
            "gathercarcass" => self.gather_carcass = ini::parse_bool(v),
            "gatherbodies" => self.gather_bodies = ini::parse_bool(v),
            "gatherfriendlybodies" => self.gather_friendly_bodies = ini::parse_bool(v),
            "hunting" => match v.parse::<u32>() {
                Ok(n) if (HUNTING_RANGE.0..=HUNTING_RANGE.1).contains(&n) => self.hunting = n,
                _ => warnings.push(format!(
                    "Hunting: bad value {v:?} ({}..{}), keeping {}",
                    HUNTING_RANGE.0, HUNTING_RANGE.1, self.hunting
                )),
            },
            "lootlogmintier" => match v.parse::<u8>() {
                Ok(n) if (LOOT_LOG_TIER_RANGE.0..=LOOT_LOG_TIER_RANGE.1).contains(&u32::from(n)) => {
                    self.loot_log_min_tier = n
                }
                _ => warnings.push(format!(
                    "LootLogMinTier: bad value {v:?} ({}..{}), keeping {}",
                    LOOT_LOG_TIER_RANGE.0, LOOT_LOG_TIER_RANGE.1, self.loot_log_min_tier
                )),
            },
            "lootlogmoney" => self.loot_log_money = ini::parse_bool(v),
            "surveylines" => match v.parse::<u32>() {
                Ok(n) if (SURVEY_LINES_RANGE.0..=SURVEY_LINES_RANGE.1).contains(&n) => self.survey_lines = n,
                _ => warnings.push(format!("SurveyLines: bad value {v:?}, keeping {}", self.survey_lines)),
            },
            "stacklimit" => match v.parse::<u32>() {
                Ok(n) if (STACK_LIMIT_RANGE.0..=STACK_LIMIT_RANGE.1).contains(&n) => self.stack_limit = n,
                _ => warnings.push(format!("StackLimit: bad value {v:?}, keeping {}", self.stack_limit)),
            },
            "bagtab" => match v.parse::<i16>() {
                Ok(id) if id >= 0 => self.bag_tab = Some(id),
                Ok(_) => self.bag_tab = None,
                Err(_) => warnings.push(format!("BagTab: bad value {v:?}, keeping auto")),
            },
            "gatherinterval" | "nodecooldown" => match v.parse::<u32>() {
                Ok(ms) if (MS_RANGE.0..=MS_RANGE.1).contains(&ms) => {
                    if k.eq_ignore_ascii_case("gatherinterval") {
                        self.gather_interval_ms = ms;
                    } else {
                        self.node_cooldown_ms = ms;
                    }
                }
                _ => warnings.push(format!(
                    "{k}: bad value {v:?} ({}..{} ms), keeping default",
                    MS_RANGE.0, MS_RANGE.1
                )),
            },
            "gatherrange" => match v.parse::<f32>() {
                Ok(f) if f > 0.0 && f <= GATHER_RANGE_MAX => self.gather_range = f,
                _ => warnings.push(format!("GatherRange: bad value {v:?}, keeping {}", self.gather_range)),
            },
            "keytoggle" | "keyscan" | "keygather" | "keyrecord" => match ini::vk_from_name(v) {
                Some(vk) if k.eq_ignore_ascii_case("keytoggle") => self.key_toggle = vk,
                Some(vk) if k.eq_ignore_ascii_case("keyscan") => self.key_scan = vk,
                Some(vk) if k.eq_ignore_ascii_case("keyrecord") => self.key_record = vk,
                Some(vk) => self.key_gather = vk,
                None => warnings.push(format!("{k}: unknown key name {v:?}, keeping previous")),
            },
            _ => return false,
        }
        true
    }

    /// `Enabled=1 Debug=0 LogReceived=0 ...`, this half's part of the `[ini]`
    /// line the harvest thread logs at startup and on every reload.
    pub fn summary(&self) -> String {
        let cfg = self;
        format!(
            "Enabled={} Debug={} LogReceived={} ScanRange={} SurveyLines={} GatherRange={} AutoGather={} GatherUnarmed={} GatherItems={} GatherGear={} GatherForaging={} GatherLogging={} GatherMining={} GatherOre={} GatherMoney={} GatherBugs={} GatherFish={} GatherCritters={} GatherCarcass={} GatherBodies={} GatherFriendlyBodies={} Hunting={} LootLogMinTier={} LootLogMoney={} BagTab={} StackLimit={} GatherInterval={} NodeCooldown={} KeyToggle=0x{:02X} KeyScan=0x{:02X} KeyGather=0x{:02X} KeyRecord=0x{:02X}",
            cfg.enabled as u8, cfg.debug as u8, cfg.log_received as u8, cfg.scan_range, cfg.survey_lines, cfg.gather_range, cfg.auto_gather as u8,
            cfg.gather_unarmed as u8, cfg.gather_items as u8, cfg.gather_gear as u8,
            cfg.gather_foraging as u8, cfg.gather_logging as u8, cfg.gather_mining as u8, cfg.gather_ore as u8, cfg.gather_money as u8, cfg.gather_bugs as u8, cfg.gather_fish as u8, cfg.gather_critters as u8, cfg.gather_carcass as u8, cfg.gather_bodies as u8, cfg.gather_friendly_bodies as u8, cfg.hunting, cfg.loot_log_min_tier, cfg.loot_log_money as u8,
            cfg.bag_tab.map(|t| t.to_string()).unwrap_or_else(|| "auto".into()), cfg.stack_limit, cfg.gather_interval_ms, cfg.node_cooldown_ms,
            cfg.key_toggle, cfg.key_scan, cfg.key_gather, cfg.key_record
        )
    }
}
