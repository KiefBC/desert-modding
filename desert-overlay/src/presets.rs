//! The preset buttons on the menu's top row.
//!
//! A preset is a one-click answer to "what do I want auto-loot to pick up",
//! and nothing else. It sets Desert Looter's four gather-family switches and
//! turns `GatherItems` on (a preset that leaves the ground items behind would
//! surprise everybody), and it deliberately leaves every other setting alone:
//! ranges, timings, `AutoGather`, `GatherGear`, `GatherUnarmed`, and the whole
//! of Desert Gatherer. Somebody who has tuned their ranges can still press a
//! preset without losing that work.

use crate::model::LooterModel;

/// The four buttons, in the order they are drawn.
pub const ALL: [Preset; 4] =
    [Preset::Everything, Preset::PlantsOnly, Preset::WoodOnly, Preset::RockAndOre];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// All four families.
    Everything,
    /// Foraging: plants, fruit, berries, mushrooms, crops.
    PlantsOnly,
    /// Logging: firewood from felled trees.
    WoodOnly,
    /// Mining and Ore Nodes together - they look alike in game but are
    /// separate families in the game's data, so a "rock" preset needs both.
    RockAndOre,
}

impl Preset {
    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            Preset::Everything => "Everything",
            Preset::PlantsOnly => "Plants only",
            Preset::WoodOnly => "Wood only",
            Preset::RockAndOre => "Rock and ore only",
        }
    }

    /// One line of explanation, shown as the button's tooltip.
    pub fn hint(self) -> &'static str {
        match self {
            Preset::Everything => "All four gather families, plus ground items.",
            Preset::PlantsOnly => "Foraging only: plants, fruit, berries, mushrooms, crops.",
            Preset::WoodOnly => "Logging only: firewood from felled trees.",
            Preset::RockAndOre => "Mining and Ore Nodes: rocks, veins, ore deposits.",
        }
    }

    /// `(foraging, logging, mining, ore)`.
    fn families(self) -> (bool, bool, bool, bool) {
        match self {
            Preset::Everything => (true, true, true, true),
            Preset::PlantsOnly => (true, false, false, false),
            Preset::WoodOnly => (false, true, false, false),
            Preset::RockAndOre => (false, false, true, true),
        }
    }

    /// Apply to a looter model in place. Touches five fields and nothing else.
    pub fn apply(self, m: &mut LooterModel) {
        let (f, l, mi, o) = self.families();
        m.gather_foraging = f;
        m.gather_logging = l;
        m.gather_mining = mi;
        m.gather_ore = o;
        m.gather_items = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_preset_sets_its_families_and_turns_items_on() {
        for (p, want) in [
            (Preset::Everything, (true, true, true, true)),
            (Preset::PlantsOnly, (true, false, false, false)),
            (Preset::WoodOnly, (false, true, false, false)),
            (Preset::RockAndOre, (false, false, true, true)),
        ] {
            let mut m = LooterModel { gather_items: false, ..LooterModel::default() };
            p.apply(&mut m);
            assert_eq!(
                (m.gather_foraging, m.gather_logging, m.gather_mining, m.gather_ore),
                want,
                "{p:?}"
            );
            assert!(m.gather_items, "{p:?} turns GatherItems on");
        }
    }

    #[test]
    fn a_preset_changes_nothing_else() {
        // The whole point: somebody who tuned their ranges keeps them.
        let tuned = LooterModel {
            enabled: false,
            auto_gather: true,
            gather_gear: true,
            gather_unarmed: false,
            scan_range: 123.0,
            gather_range: 12.5,
            gather_interval_ms: 250,
            node_cooldown_ms: 30_000,
            stack_limit: 5000,
            ..LooterModel::default()
        };
        for p in ALL {
            let mut m = tuned.clone();
            p.apply(&mut m);
            assert_eq!(m.enabled, tuned.enabled);
            assert_eq!(m.auto_gather, tuned.auto_gather);
            assert_eq!(m.gather_gear, tuned.gather_gear);
            assert_eq!(m.gather_unarmed, tuned.gather_unarmed);
            assert_eq!(m.scan_range, tuned.scan_range);
            assert_eq!(m.gather_range, tuned.gather_range);
            assert_eq!(m.gather_interval_ms, tuned.gather_interval_ms);
            assert_eq!(m.node_cooldown_ms, tuned.node_cooldown_ms);
            assert_eq!(m.stack_limit, tuned.stack_limit);
        }
    }

    #[test]
    fn everything_is_the_looter_default_family_set() {
        let mut m = LooterModel { gather_foraging: false, ..LooterModel::default() };
        Preset::Everything.apply(&mut m);
        assert_eq!(m, LooterModel::default());
    }

    #[test]
    fn labels_are_distinct() {
        let mut labels: Vec<_> = ALL.iter().map(|p| p.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), ALL.len());
    }
}
