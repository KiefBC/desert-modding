//! The item-log filter's policy: which "Item xN" lines the HUD's side item
//! log is allowed to show. Pure and always compiled, so the decision and the
//! counters are unit tested natively; the hook that asks is
//! `crate::looter::itemlog`, which is Windows-only.
//!
//! Three keys in `[Harvest]` drive it, read live through [`LIVE`]:
//!
//! * `LootLogMinTier`, `0..=6`. `0` is vanilla (every line, and the hook does
//!   no reads at all); `1..=5` shows only items whose parsed `ItemInfo`
//!   `itemTier` is at least that; `6` is above every tier, so no line shows.
//! * `LootLogMoney`, on by default: an item whose `itemType` is the currency
//!   group ([`MONEY_ITEM_TYPE`]) always shows, whatever the tier says.
//! * `LootLogQuest`, on by default: a quest or key item always shows,
//!   whatever the tier says. Which items those are is the generated key list
//!   `desert_core::quest_items` (the game's own inventory groups); the hook
//!   gets a record **index**, not a key, so the plugin thread turns that list
//!   into a bit per index once the item layout has latched ([`QuestMap`],
//!   built by `crate::looter::itemlog::tick`). Until it has, the answer is
//!   "not known" and the tier decides, as it did before the key existed.
//!
//! **Every failed read shows the line.** A record that would not read, or a
//! tier outside `0..=TIER_MAX` (which is what a shifted offset reads as), is
//! a line the player sees: a broken layout costs the filter, never the log.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};

use crate::looter::config::{Config, LOOT_LOG_TIER_RANGE};

/// `itemType`, `u8`, in a parsed `ItemInfo` record (26361232,
/// `analysis/field-offsets-26361232.md`).
pub const ITEM_TYPE_OFF: usize = 0x37;
/// `itemTier`, `u8`, in a parsed `ItemInfo` record (26361232).
pub const ITEM_TIER_OFF: usize = 0x39;

/// `ItemInfo.itemType` of the currency group (Copper, Camp Funds, ...).
pub const MONEY_ITEM_TYPE: u8 = 32;

/// The highest `ItemInfo.itemTier` the game uses (`0..=5`). A larger byte is
/// not a tier this build writes, so it is read as "unreadable".
pub const TIER_MAX: u8 = 5;

/// The two fields of a parsed `ItemInfo` record the filter needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemFacts {
    pub item_type: u8,
    pub tier: u8,
}

/// What [`decide`] concluded, and why. Only [`Decision::Hidden`] hides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// `LootLogMinTier=0`: the vanilla log, decided without a read.
    Vanilla,
    /// The record or one of its fields did not read, or read implausibly.
    Unreadable,
    /// Currency, and `LootLogMoney=1`.
    Money,
    /// A quest or key item ([`QuestMap`]), and `LootLogQuest=1`.
    Quest,
    /// The item's tier is at or above the minimum.
    Tier,
    /// Below the minimum tier: the line is dropped.
    Hidden,
}

impl Decision {
    /// Whether the game's handler should run and the line appear.
    pub fn shows(self) -> bool {
        !matches!(self, Decision::Hidden)
    }

    /// The word the debug line prints.
    pub fn word(self) -> &'static str {
        if self.shows() { "shown" } else { "hidden" }
    }
}

/// The filter's whole policy. `facts` is `None` when the record would not
/// read; a tier above [`TIER_MAX`] is treated the same way. `quest` is the
/// record's [`QuestMap`] bit, `None` while the map is not built: the tier
/// decides then. `quest_switch` is `LootLogQuest`.
///
/// In order: tier 0 is vanilla; a failed read shows; money under
/// `LootLogMoney`; a quest item under `LootLogQuest`; the tier floor; else
/// hidden.
pub fn decide(min_tier: u8, money: bool, quest_switch: bool, quest: Option<bool>, facts: Option<ItemFacts>) -> Decision {
    if min_tier == 0 {
        return Decision::Vanilla;
    }
    let Some(f) = facts.filter(|f| f.tier <= TIER_MAX) else {
        return Decision::Unreadable;
    };
    if money && f.item_type == MONEY_ITEM_TYPE {
        Decision::Money
    } else if quest_switch && quest == Some(true) {
        Decision::Quest
    } else if f.tier >= min_tier {
        Decision::Tier
    } else {
        Decision::Hidden
    }
}

/// The live settings the hook reads on every call. Atomics rather than a
/// copy parked anywhere: the callback runs on a game thread inside the game's
/// own UI code, may not lock, and a line decided on one tick's older value is
/// a line at the previous setting. `Relaxed` throughout for that reason.
pub struct Live {
    min_tier: AtomicU8,
    money: AtomicBool,
    quest: AtomicBool,
    debug: AtomicBool,
}

pub static LIVE: Live = Live::new();

impl Live {
    const fn new() -> Self {
        Live {
            min_tier: AtomicU8::new(0),
            money: AtomicBool::new(true),
            quest: AtomicBool::new(true),
            debug: AtomicBool::new(false),
        }
    }

    /// Publish this half's config. Called from the harvest thread at startup
    /// and on every reload; `Enabled=0` publishes the vanilla log.
    pub fn publish(&self, cfg: &Config) {
        let tier = if cfg.enabled { cfg.loot_log_min_tier } else { 0 };
        // Clamped here too: this is the last gate in front of the decision,
        // and a value `apply_key` would refuse costs nothing to hold.
        let tier = u32::from(tier).clamp(LOOT_LOG_TIER_RANGE.0, LOOT_LOG_TIER_RANGE.1);
        self.min_tier.store(u8::try_from(tier).unwrap_or(0), Ordering::Relaxed);
        self.money.store(cfg.loot_log_money, Ordering::Relaxed);
        self.quest.store(cfg.loot_log_quest, Ordering::Relaxed);
        self.debug.store(cfg.debug, Ordering::Relaxed);
    }

    pub fn min_tier(&self) -> u8 {
        self.min_tier.load(Ordering::Relaxed)
    }

    pub fn money(&self) -> bool {
        self.money.load(Ordering::Relaxed)
    }

    pub fn quest(&self) -> bool {
        self.quest.load(Ordering::Relaxed)
    }

    pub fn debug(&self) -> bool {
        self.debug.load(Ordering::Relaxed)
    }
}

/// Item records the quest map can hold: 16384, against 7064 `iteminfo`
/// records on 26361232. A bigger table is refused whole (one log line, and
/// the map never becomes ready) rather than half-marked.
pub const QUEST_MAP_RECORDS: usize = 1 << 14;
const QUEST_WORDS: usize = QUEST_MAP_RECORDS / 64;

/// One bit per `iteminfo` record index: is it a quest or key item. Set by the
/// plugin thread, read by the item-log hook with one atomic load; nothing on
/// the game thread ever builds it. `examined` remembers which indices the
/// build has read, so a later walk over records the game loaded since reads
/// only those.
pub struct QuestMap {
    quest: [AtomicU64; QUEST_WORDS],
    examined: [AtomicU64; QUEST_WORDS],
    ready: AtomicBool,
}

pub static QUEST_MAP: QuestMap = QuestMap::new();

fn bit(words: &[AtomicU64], idx: usize) -> bool {
    words.get(idx >> 6).is_some_and(|w| w.load(Ordering::Relaxed) & (1u64 << (idx & 63)) != 0)
}

fn set_bit(words: &[AtomicU64], idx: usize) {
    if let Some(w) = words.get(idx >> 6) {
        w.fetch_or(1u64 << (idx & 63), Ordering::Relaxed);
    }
}

impl QuestMap {
    pub const fn new() -> Self {
        QuestMap {
            quest: [const { AtomicU64::new(0) }; QUEST_WORDS],
            examined: [const { AtomicU64::new(0) }; QUEST_WORDS],
            ready: AtomicBool::new(false),
        }
    }

    /// The record's bit for [`decide`]: `None` until the first build has
    /// finished, then whether `idx` was marked. An index past the map is not
    /// a quest item (the build refuses a table that would have one).
    pub fn get(&self, idx: u16) -> Option<bool> {
        self.ready.load(Ordering::Relaxed).then(|| bit(&self.quest, usize::from(idx)))
    }

    /// Has record `idx` been read by a build?
    pub fn examined(&self, idx: usize) -> bool {
        bit(&self.examined, idx)
    }

    /// Record what a build read off record `idx`. Out-of-range indices are
    /// ignored; the build checks the record count first.
    pub fn record(&self, idx: usize, is_quest: bool) {
        if is_quest {
            set_bit(&self.quest, idx);
        }
        set_bit(&self.examined, idx);
    }

    /// Publish the map to the hook. Called after the first walk.
    pub fn set_ready(&self) {
        self.ready.store(true, Ordering::Relaxed);
    }

    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// How many records are marked.
    pub fn marked(&self) -> usize {
        self.quest.iter().map(|w| w.load(Ordering::Relaxed).count_ones() as usize).sum()
    }
}

impl Default for QuestMap {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a build that marked `marked` records out of a key list of
/// `table_keys` should warn: under half of them found, once every record is
/// loaded, means the key offset is wrong or the list is from another build.
pub fn quest_map_suspicious(marked: usize, table_keys: usize) -> bool {
    marked.saturating_mul(2) < table_keys
}

/// Per-session counters for the `[itemlog] stat:` line. `shown` counts every
/// line that went through, so `unreadable` (shown because a read failed) and
/// `quest` (shown because `LootLogQuest` kept it) are parts of it, not beside
/// it: `seen = shown + hidden`.
pub struct Counters {
    seen: AtomicU64,
    shown: AtomicU64,
    hidden: AtomicU64,
    unreadable: AtomicU64,
    quest: AtomicU64,
}

pub static COUNTERS: Counters = Counters::new();

impl Counters {
    const fn new() -> Self {
        Counters {
            seen: AtomicU64::new(0),
            shown: AtomicU64::new(0),
            hidden: AtomicU64::new(0),
            unreadable: AtomicU64::new(0),
            quest: AtomicU64::new(0),
        }
    }

    /// Count one decision.
    pub fn record(&self, d: Decision) {
        self.seen.fetch_add(1, Ordering::Relaxed);
        if d.shows() {
            self.shown.fetch_add(1, Ordering::Relaxed);
        } else {
            self.hidden.fetch_add(1, Ordering::Relaxed);
        }
        match d {
            Decision::Unreadable => {
                self.unreadable.fetch_add(1, Ordering::Relaxed);
            }
            Decision::Quest => {
                self.quest.fetch_add(1, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// The counters as one line body, or `None` before the first line.
    pub fn stat_line(&self) -> Option<String> {
        let r = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let seen = r(&self.seen);
        (seen > 0).then(|| {
            format!(
                "seen {seen}, shown {}, hidden {}, unreadable {}, quest {}",
                r(&self.shown),
                r(&self.hidden),
                r(&self.unreadable),
                r(&self.quest)
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(item_type: u8, tier: u8) -> Option<ItemFacts> {
        Some(ItemFacts { item_type, tier })
    }

    /// `decide` as it was before `LootLogQuest`: the quest map not built.
    fn decide(min_tier: u8, money: bool, facts: Option<ItemFacts>) -> Decision {
        super::decide(min_tier, money, true, None, facts)
    }

    #[test]
    fn zero_is_vanilla_whatever_the_item() {
        assert_eq!(decide(0, false, None), Decision::Vanilla);
        assert_eq!(decide(0, false, item(1, 0)), Decision::Vanilla);
        assert!(decide(0, true, item(MONEY_ITEM_TYPE, 9)).shows());
    }

    #[test]
    fn a_failed_or_implausible_read_shows_the_line() {
        assert_eq!(decide(6, false, None), Decision::Unreadable);
        assert_eq!(decide(3, false, item(1, TIER_MAX + 1)), Decision::Unreadable);
        assert_eq!(decide(3, true, item(MONEY_ITEM_TYPE, 0xFF)), Decision::Unreadable);
        assert!(Decision::Unreadable.shows());
    }

    #[test]
    fn the_tier_floor_is_inclusive() {
        assert_eq!(decide(3, false, item(1, 2)), Decision::Hidden);
        assert_eq!(decide(3, false, item(1, 3)), Decision::Tier);
        assert_eq!(decide(3, false, item(1, 5)), Decision::Tier);
        assert_eq!(decide(1, false, item(1, 0)), Decision::Hidden);
    }

    #[test]
    fn six_hides_every_item_but_money() {
        for tier in 0..=TIER_MAX {
            assert_eq!(decide(6, false, item(1, tier)), Decision::Hidden, "tier {tier}");
            assert_eq!(decide(6, true, item(MONEY_ITEM_TYPE, tier)), Decision::Money, "tier {tier}");
            assert_eq!(decide(6, false, item(MONEY_ITEM_TYPE, tier)), Decision::Hidden, "tier {tier}");
        }
    }

    #[test]
    fn the_counters_add_up() {
        let c = Counters::new();
        assert_eq!(c.stat_line(), None);
        for d in [
            Decision::Vanilla,
            Decision::Hidden,
            Decision::Unreadable,
            Decision::Money,
            Decision::Hidden,
            Decision::Quest,
        ] {
            c.record(d);
        }
        assert_eq!(c.stat_line().as_deref(), Some("seen 6, shown 4, hidden 2, unreadable 1, quest 1"));
    }

    #[test]
    fn disabled_publishes_the_vanilla_log() {
        let live = Live::new();
        let mut cfg = Config { loot_log_min_tier: 4, loot_log_money: false, loot_log_quest: false, ..Config::default() };
        live.publish(&cfg);
        assert_eq!((live.min_tier(), live.money(), live.quest()), (4, false, false));
        cfg.enabled = false;
        live.publish(&cfg);
        assert_eq!(live.min_tier(), 0, "Enabled=0 must leave the log alone");
    }

    #[test]
    fn quest_items_show_under_the_switch_once_the_map_is_built() {
        use super::decide as d;
        // Kept whatever the tier, including 6.
        for min in 1..=6 {
            assert_eq!(d(min, false, true, Some(true), item(1, 0)), Decision::Quest, "min {min}");
            assert!(Decision::Quest.shows());
        }
        // The switch off, or the bit clear, leaves it to the tier.
        assert_eq!(d(3, false, false, Some(true), item(1, 0)), Decision::Hidden);
        assert_eq!(d(3, false, true, Some(false), item(1, 0)), Decision::Hidden);
        assert_eq!(d(3, false, true, Some(false), item(1, 4)), Decision::Tier);
        // Not built yet: the tier decides, as before the key existed.
        assert_eq!(d(3, false, true, None, item(1, 0)), Decision::Hidden);
        assert_eq!(d(3, false, true, None, item(1, 3)), Decision::Tier);
        // Tier 0 is vanilla and a failed read shows, ahead of the quest bit.
        assert_eq!(d(0, false, true, Some(true), item(1, 0)), Decision::Vanilla);
        assert_eq!(d(3, false, true, Some(true), None), Decision::Unreadable);
        assert_eq!(d(3, false, true, Some(true), item(1, TIER_MAX + 1)), Decision::Unreadable);
        // Money is checked first, so a currency item is Money either way.
        assert_eq!(d(6, true, true, Some(true), item(MONEY_ITEM_TYPE, 0)), Decision::Money);
        assert_eq!(d(6, false, true, Some(true), item(MONEY_ITEM_TYPE, 0)), Decision::Quest);
    }

    #[test]
    fn the_quest_map_answers_none_until_ready() {
        let m = QuestMap::new();
        m.record(7, true);
        m.record(8, false);
        assert_eq!(m.get(7), None);
        assert!(m.examined(7) && m.examined(8) && !m.examined(9));
        m.set_ready();
        assert_eq!((m.get(7), m.get(8), m.get(9)), (Some(true), Some(false), Some(false)));
        assert_eq!(m.marked(), 1);
        // Past the end: ignored on write, false on read, never a panic.
        m.record(QUEST_MAP_RECORDS, true);
        m.record(usize::MAX, true);
        assert_eq!(m.marked(), 1);
        assert_eq!(m.get(u16::MAX), Some(false));
        const { assert!(QUEST_MAP_RECORDS >= 2 * 7064) };
    }

    #[test]
    fn a_map_under_half_the_table_is_suspicious() {
        assert!(!quest_map_suspicious(1194, 1194));
        assert!(!quest_map_suspicious(597, 1194));
        assert!(quest_map_suspicious(596, 1194));
        assert!(quest_map_suspicious(0, 1194));
    }
}
