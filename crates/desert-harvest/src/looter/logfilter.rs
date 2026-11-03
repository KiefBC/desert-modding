//! The item-log filter's policy: which "Item xN" lines the HUD's side item
//! log is allowed to show. Pure and always compiled, so the decision and the
//! counters are unit tested natively; the hook that asks is
//! `crate::looter::itemlog`, which is Windows-only.
//!
//! Two keys in `[Harvest]` drive it, read live through [`LIVE`]:
//!
//! * `LootLogMinTier`, `0..=6`. `0` is vanilla (every line, and the hook does
//!   no reads at all); `1..=5` shows only items whose parsed `ItemInfo`
//!   `itemTier` is at least that; `6` is above every tier, so no line shows.
//! * `LootLogMoney`, on by default: an item whose `itemType` is the currency
//!   group ([`MONEY_ITEM_TYPE`]) always shows, whatever the tier says.
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
/// read; a tier above [`TIER_MAX`] is treated the same way.
pub fn decide(min_tier: u8, money: bool, facts: Option<ItemFacts>) -> Decision {
    if min_tier == 0 {
        return Decision::Vanilla;
    }
    let Some(f) = facts.filter(|f| f.tier <= TIER_MAX) else {
        return Decision::Unreadable;
    };
    if money && f.item_type == MONEY_ITEM_TYPE {
        Decision::Money
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
    debug: AtomicBool,
}

pub static LIVE: Live =
    Live { min_tier: AtomicU8::new(0), money: AtomicBool::new(true), debug: AtomicBool::new(false) };

impl Live {
    /// Publish this half's config. Called from the harvest thread at startup
    /// and on every reload; `Enabled=0` publishes the vanilla log.
    pub fn publish(&self, cfg: &Config) {
        let tier = if cfg.enabled { cfg.loot_log_min_tier } else { 0 };
        // Clamped here too: this is the last gate in front of the decision,
        // and a value `apply_key` would refuse costs nothing to hold.
        let tier = u32::from(tier).clamp(LOOT_LOG_TIER_RANGE.0, LOOT_LOG_TIER_RANGE.1);
        self.min_tier.store(u8::try_from(tier).unwrap_or(0), Ordering::Relaxed);
        self.money.store(cfg.loot_log_money, Ordering::Relaxed);
        self.debug.store(cfg.debug, Ordering::Relaxed);
    }

    pub fn min_tier(&self) -> u8 {
        self.min_tier.load(Ordering::Relaxed)
    }

    pub fn money(&self) -> bool {
        self.money.load(Ordering::Relaxed)
    }

    pub fn debug(&self) -> bool {
        self.debug.load(Ordering::Relaxed)
    }
}

/// Per-session counters for the `[itemlog] stat:` line. `shown` counts every
/// line that went through, so `unreadable` (shown because a read failed) is a
/// part of it, not beside it: `seen = shown + hidden`.
pub struct Counters {
    seen: AtomicU64,
    shown: AtomicU64,
    hidden: AtomicU64,
    unreadable: AtomicU64,
}

pub static COUNTERS: Counters = Counters {
    seen: AtomicU64::new(0),
    shown: AtomicU64::new(0),
    hidden: AtomicU64::new(0),
    unreadable: AtomicU64::new(0),
};

impl Counters {
    /// Count one decision.
    pub fn record(&self, d: Decision) {
        self.seen.fetch_add(1, Ordering::Relaxed);
        if d.shows() {
            self.shown.fetch_add(1, Ordering::Relaxed);
        } else {
            self.hidden.fetch_add(1, Ordering::Relaxed);
        }
        if d == Decision::Unreadable {
            self.unreadable.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The counters as one line body, or `None` before the first line.
    pub fn stat_line(&self) -> Option<String> {
        let r = |a: &AtomicU64| a.load(Ordering::Relaxed);
        let seen = r(&self.seen);
        (seen > 0).then(|| {
            format!(
                "seen {seen}, shown {}, hidden {}, unreadable {}",
                r(&self.shown),
                r(&self.hidden),
                r(&self.unreadable)
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
        let c = Counters {
            seen: AtomicU64::new(0),
            shown: AtomicU64::new(0),
            hidden: AtomicU64::new(0),
            unreadable: AtomicU64::new(0),
        };
        assert_eq!(c.stat_line(), None);
        for d in [Decision::Vanilla, Decision::Hidden, Decision::Unreadable, Decision::Money, Decision::Hidden] {
            c.record(d);
        }
        assert_eq!(c.stat_line().as_deref(), Some("seen 5, shown 3, hidden 2, unreadable 1"));
    }

    #[test]
    fn disabled_publishes_the_vanilla_log() {
        let live = Live { min_tier: AtomicU8::new(0), money: AtomicBool::new(true), debug: AtomicBool::new(false) };
        let mut cfg = Config { loot_log_min_tier: 4, loot_log_money: false, ..Config::default() };
        live.publish(&cfg);
        assert_eq!((live.min_tier(), live.money()), (4, false));
        cfg.enabled = false;
        live.publish(&cfg);
        assert_eq!(live.min_tier(), 0, "Enabled=0 must leave the log alone");
    }
}
