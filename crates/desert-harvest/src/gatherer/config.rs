//! The yield half's keys in the `[Harvest]` section of `DesertTooling.ini`,
//! beside the game exe. Missing file, missing section or missing key =>
//! defaults.
//!
//! The section is walked once, by `crate::config::parse`, which owns the two
//! keys both halves read (`Enabled`, `Debug`) and offers every other key to the
//! auto-loot half first and then to [`Config::apply_key`] here. `DryRun` and
//! `DumpTable` are this half's alone: they scope to the yield multipliers and
//! the table dump, and auto-loot never reads them. The ini tokeniser and the
//! truthy spellings are still shared in `desert_core::ini`. A bad value never
//! replaces the default.
//!
//! The family multiplier keys are the independent gather families of
//! `desert_core::collect::Family`. Four of the five are read off the game's
//! own records (the family rule in
//! `crates/tools/src/bin/gen-collect-names/classify.rs`): Foraging, Logging,
//! Mining and Ore Nodes are separate internal families, and setting one does
//! not touch the others. `Foraging` also covers records no field marks, the
//! water well among them - see the field's doc comment below.
//!
//! `Money` is the fifth family and the odd one out: no offset pack ever had it,
//! and what it multiplies is money rather than a material. Mechanically it is
//! the same lever as the other four - the placed coin and silver bar props
//! read an output block like any other node, so the same table edit reaches
//! them - but it is the first *balance* lever this subsystem has, because it
//! changes how fast the player gets money rather than saving them trips. Its
//! default 1 is therefore deliberate, and the help says so. It multiplies the
//! money lying in the world and nothing else: not enemy drops, not quest
//! rewards, not coin pouches, not chests, not shop prices, not dispatch
//! rewards.
//!
//! `Bugs` and `Fish` are a different lever entirely. Creatures caught by hand
//! are not gimmick records, so there is nothing in a table to multiply; the
//! count is an immediate in the game's code (`docs/reference-internals.md`
//! section 17) and the plugin patches it. They share the multiplier range and
//! the parsing of the family keys, and nothing else - hence
//! [`Config::catch_multiplier`] beside [`Config::multiplier`] rather than two
//! more `Family` arms.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use desert_core::collect::Family;
use desert_core::creature::CatchClass;
use desert_core::ini;
use desert_core::schema::{Field, Kind, Tab};

use crate::config::{f, on};

/// Multipliers below this are meaningless (0 would zero out every yield) and
/// above it are almost certainly a typo, so both are refused.
pub const MULT_MIN: u32 = 1;
pub const MULT_MAX: u32 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Master switch. 0 = the hook is still installed and still reads every
    /// record, but writes nothing; anything already multiplied is put back to
    /// vanilla on the next re-apply pass.
    pub enabled: bool,
    /// 1 = the hook logs the edits it would make and writes nothing.
    pub dry_run: bool,
    /// 1 = also log every record the loader hands us that is not a gather
    /// record (13,600 of them), capped by the hook so the log stays finite.
    pub debug: bool,
    /// 1 = on the loader's first call, copy the whole gimmickinfo table body
    /// out of its stream buffer and write it to
    /// [`crate::gatherer::dump::FILE_NAME`] beside the log, once per session: a
    /// diagnostic copy of the loader buffer, which the offline tools no longer
    /// need (they read the game's archives). **Needs `dry_run`**: several game threads multiply records
    /// in that same buffer while the copy is read, so only a hook that writes
    /// nothing guarantees the file is vanilla; without `DryRun=1` this is
    /// logged as ignored and nothing is copied. Read once, at the first
    /// load, so a change after that waits for the next launch.
    pub dump_table: bool,
    /// Yield multiplier for `Family::Foraging` (plants, fruit, mushrooms,
    /// crops, and the water drawn from a water well - `gimmick_well_0001_parts01`,
    /// an override, since no field of the record marks it. Its block is a fixed
    /// `5..5`, so a 3 pays exactly 15 - and on 2026-09-13 it did, with the bag
    /// count logged either side of the take (`Water x33` -> `x48` at
    /// `Foraging=3`, `analysis/logs/DesertTooling-2026-09-13-well-x15-x30.log`).
    /// Raising the slider *after* the bucket is full still counts: the amount
    /// is rolled when the water is taken, not when the bucket fills).
    pub foraging: u32,
    /// Yield multiplier for `Family::Logging` (`firewood_*`).
    pub logging: u32,
    /// Yield multiplier for `Family::Mining` (`collect_mine`: rocks, veins,
    /// breakable stalactites).
    pub mining: u32,
    /// Yield multiplier for `Family::Ore` (`collect_ore`: `ore_*` deposits,
    /// sulfur stone, collectible stalactites). Separate from Mining.
    pub ore: u32,
    /// Yield multiplier for `Family::Money`: the three money props placed in
    /// the world - `gimmick_item_common_coin_0001` (10..15),
    /// `gimmick_item_common_coin_0002` (100..150) and
    /// `gimmick_item_common_silverbar_0001` (a fixed 2500..2500), all paying
    /// item 1, the game's one money item. This is the money lying in the world
    /// and nothing else: not enemy drops, not quest rewards, not coin pouches,
    /// not chests, not shop prices, not dispatch rewards. The default is 1 on
    /// purpose - unlike the other four this changes how fast the player gets
    /// money, which is a balance decision rather than a convenience one.
    /// CONFIRMED IN GAME 2026-09-13, build 25246367: three vanilla hand
    /// pickups of `coin_0001` paid item 1 x15, x14, x15, inside its 10..15
    /// block, and one pickup at `Money=3` paid `x30` (a roll of 10, times 3)
    /// against a bag delta of exactly +30 between two F11 surveys - written
    /// by the live re-apply pass, not the load-time hook, since the slider
    /// moved after the table had loaded.
    pub money: u32,
    /// Count multiplier for insects caught by hand
    /// (`desert_core::creature::CatchClass::Bug`). A code patch, not a table
    /// edit: see the module docs.
    pub bugs: u32,
    /// Count multiplier for fish caught by hand
    /// (`desert_core::creature::CatchClass::Fish`). Same patch as `bugs`.
    pub fish: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            dry_run: false,
            debug: false,
            dump_table: false,
            foraging: 1,
            logging: 1,
            mining: 1,
            ore: 1,
            money: 1,
            bugs: 1,
            fish: 1,
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
            Family::Money => self.money,
        }
    }

    /// The configured multiplier for a creature caught by hand. Deliberately
    /// separate from [`Config::multiplier`]: bugs and fish are not gather
    /// families, they are not in `desert_core::collect`, and they are applied
    /// by a code patch on the catch count rather than by editing a record.
    pub fn catch_multiplier(&self, class: CatchClass) -> u32 {
        match class {
            CatchClass::Bug => self.bugs,
            CatchClass::Fish => self.fish,
        }
    }

    /// True if no gather family is multiplied, i.e. the record-loader hook
    /// would never write. `Money` counts: its records are ordinary gimmick
    /// records with ordinary output blocks, so raising it gives the hook work
    /// to do exactly as raising `Ore` does. Says nothing about `Bugs`/`Fish`,
    /// which are not families and never touch a record.
    pub fn all_vanilla(&self) -> bool {
        self.foraging <= 1
            && self.logging <= 1
            && self.mining <= 1
            && self.ore <= 1
            && self.money <= 1
    }
}

/// Lock-free mirror of [`Config`] the hook reads on every record load.
///
/// The overlay subsystem edits the `[Harvest]` section of `DesertTooling.ini`
/// beside the game exe while it runs; the harvest thread notices (see
/// `crate::entry`'s reload loop), re-parses with `crate::config::parse` and calls
/// [`LiveConfig::publish`]. The hook itself
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
    dump_table: AtomicBool,
    foraging: AtomicU32,
    logging: AtomicU32,
    mining: AtomicU32,
    ore: AtomicU32,
    money: AtomicU32,
    bugs: AtomicU32,
    fish: AtomicU32,
}

impl LiveConfig {
    /// Starts equal to `Config::default()` so a hook installed before the
    /// first `publish` behaves exactly like the pre-live-reload plugin.
    pub const fn new() -> Self {
        LiveConfig {
            enabled: AtomicBool::new(true),
            dry_run: AtomicBool::new(false),
            debug: AtomicBool::new(false),
            dump_table: AtomicBool::new(false),
            foraging: AtomicU32::new(1),
            logging: AtomicU32::new(1),
            mining: AtomicU32::new(1),
            ore: AtomicU32::new(1),
            money: AtomicU32::new(1),
            bugs: AtomicU32::new(1),
            fish: AtomicU32::new(1),
        }
    }

    /// Publish a freshly parsed config for the hook to pick up. Called from
    /// the plugin's main thread only (startup, and once per detected ini
    /// change); the hook only ever reads.
    pub fn publish(&self, cfg: &Config) {
        self.enabled.store(cfg.enabled, Ordering::Relaxed);
        self.dry_run.store(cfg.dry_run, Ordering::Relaxed);
        self.debug.store(cfg.debug, Ordering::Relaxed);
        self.dump_table.store(cfg.dump_table, Ordering::Relaxed);
        self.foraging.store(cfg.foraging, Ordering::Relaxed);
        self.logging.store(cfg.logging, Ordering::Relaxed);
        self.mining.store(cfg.mining, Ordering::Relaxed);
        self.ore.store(cfg.ore, Ordering::Relaxed);
        self.money.store(cfg.money, Ordering::Relaxed);
        self.bugs.store(cfg.bugs, Ordering::Relaxed);
        self.fish.store(cfg.fish, Ordering::Relaxed);
    }

    /// Snapshot the live values as a plain `Config`, e.g. for a log line.
    pub fn load(&self) -> Config {
        Config {
            enabled: self.enabled.load(Ordering::Relaxed),
            dry_run: self.dry_run.load(Ordering::Relaxed),
            debug: self.debug.load(Ordering::Relaxed),
            dump_table: self.dump_table.load(Ordering::Relaxed),
            foraging: self.foraging.load(Ordering::Relaxed),
            logging: self.logging.load(Ordering::Relaxed),
            mining: self.mining.load(Ordering::Relaxed),
            ore: self.ore.load(Ordering::Relaxed),
            money: self.money.load(Ordering::Relaxed),
            bugs: self.bugs.load(Ordering::Relaxed),
            fish: self.fish.load(Ordering::Relaxed),
        }
    }

    /// Master switch. `false` means the hook reads as usual and writes
    /// nothing, and `hook::reapply` puts the loaded records back to vanilla.
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

    /// `true` = copy the table body on the loader's first call, if `DryRun`
    /// is on too. Read by the hook exactly once, inside its first-call block,
    /// so it costs the per-record path nothing at all.
    pub fn dump_table(&self) -> bool {
        self.dump_table.load(Ordering::Relaxed)
    }

    /// The live multiplier for a gather family. Safe to call from the hook on
    /// every record: one atomic load, no allocation, no lock.
    pub fn multiplier(&self, family: Family) -> u32 {
        match family {
            Family::Foraging => self.foraging.load(Ordering::Relaxed),
            Family::Logging => self.logging.load(Ordering::Relaxed),
            Family::Mining => self.mining.load(Ordering::Relaxed),
            Family::Ore => self.ore.load(Ordering::Relaxed),
            Family::Money => self.money.load(Ordering::Relaxed),
        }
    }

    /// The live multiplier for a creature caught by hand. Read from the catch
    /// hook on the game thread, once per catch: one atomic load, no
    /// allocation, no lock, so an ini change lands on the very next catch
    /// with no re-apply pass of any kind.
    pub fn catch_multiplier(&self, class: CatchClass) -> u32 {
        match class {
            CatchClass::Bug => self.bugs.load(Ordering::Relaxed),
            CatchClass::Fish => self.fish.load(Ordering::Relaxed),
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

/// One of the family multipliers: an identical `1..=100` slider, so they
/// differ only in their key, label and help.
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

/// The line the menu shows on the Harvest tab about when a yield change
/// lands.
pub(crate) fn notice() -> String {
    "The yield multipliers are applied to the gather records the game has already loaded, so a \
     change to one takes effect on the next gather. Bugs and Fish are read as the creature is \
     caught, so they take effect on the next catch."
        .to_string()
}

/// This half's fields for the `[Harvest]` menu schema (`crate::config::schema`),
/// in menu order. `Enabled` and `Debug` are not here: both halves read them, so
/// the root owns them.
///
/// `DryRun` and `DumpTable` are marked [`Tab::Debug`], so the menu draws them on
/// its shared Debug tab rather than among the sliders: whichever half a player
/// is chasing, the switches a bug report needs are in one place. `DumpTable` is
/// a one-launch tool for whoever runs the offline tools, not a setting a player
/// plays with, and it only works with `DryRun` ticked beside it.
///
/// Every default is `Config::default()` (vanilla yields, which the shipped
/// template matches) and the multiplier range is [`MULT_MIN`]..=[`MULT_MAX`],
/// the same consts [`Config::apply_key`] enforces. Both facts are tested in
/// `crate::config`.
pub(crate) fn fields() -> Vec<Field> {
    let d = Config::default();
    vec![
        // On the shared Debug tab, after the auto-loot half's diagnostics,
        // and on a row of its own: the field before it there is BagTab,
        // which has nothing to do with it.
        on(
            Tab::Debug,
            f(
                "DryRun",
                "Dry run",
                Kind::Bool { default: d.dry_run },
                "Yield multipliers only: log what the gather-record and catch-count edits would be and write nothing to the game. Auto-loot is not affected.",
            ),
        ),
        Field {
            heading: Some("Yield multipliers:".to_string()),
            ..mult(
                "Foraging",
                "Plants, fruit, berries, mushrooms, crops, and the water you draw from a water \
                 well. 97 records. A well pays a fixed amount, so at 3 a well that gave 5 gives \
                 exactly 15.",
            )
        },
        mult("Logging", "Firewood cut from felled trees (firewood_*). 142 records."),
        mult(
            "Mining",
            "The collect_mine family: mine_* rocks and veins, breakable stalactites. 36 records.",
        ),
        mult(
            "Ore",
            "The collect_ore family: ore_* deposits and sulfur stone, separate from Mining. 16 records.",
        ),
        mult(
            "Money",
            "The placed coin and silver bar props, 3 records, all paying the game's one money \
             item. This multiplies money lying in the world and nothing else - not enemy drops, \
             not quest rewards, not coin pouches, not chests, not shop prices, not dispatch \
             rewards. The default is 1 on purpose: this one changes how fast you get money \
             rather than saving you trips, so it is yours to raise. Copper, silver and gold are \
             display units over one count, so a multiplied pickup can show up as a different \
             denomination than the vanilla one did.",
        ),
        mult(
            "Bugs",
            "Insects caught by hand. One code patch on the catch count, not a table edit.",
        ),
        mult("Fish", "Fish caught by hand at the water's edge. Same patch as Bugs."),
        // Beside DryRun on the shared Debug tab (the field before it on
        // that tab), which is where it belongs: it does nothing unless that
        // box is ticked too, and the help says so.
        on(
            Tab::Debug,
            Field {
                same_line: true,
                ..f(
                    "DumpTable",
                    "Dump table",
                    Kind::Bool { default: d.dump_table },
                    "Write the game's vanilla gimmickinfo table body to DesertTooling.gimmickinfo.bin beside the log on the next launch. Needs DryRun=1, so nothing in the copy is multiplied; without it this is ignored. About 22 MB, written once per session. It is a diagnostic copy of the loader buffer; the offline tools read the game's archives instead.",
                )
            },
        ),
    ]
}

impl Config {
    /// Apply one `key=value` from the `[Harvest]` section if it is one of this
    /// half's keys; `false` means "not mine". `lower` is `k` in ASCII lower
    /// case. A bad value is pushed onto `warnings` and keeps the current value.
    /// `Enabled` and `Debug` never get here; the root owns them.
    pub(crate) fn apply_key(&mut self, lower: &str, k: &str, v: &str, warnings: &mut Vec<String>) -> bool {
        match lower {
            "dryrun" => self.dry_run = ini::parse_bool(v),
            "dumptable" => self.dump_table = ini::parse_bool(v),
            "foraging" | "logging" | "mining" | "ore" | "money" | "bugs" | "fish" => {
                let slot: &mut u32 = match lower {
                    "foraging" => &mut self.foraging,
                    "logging" => &mut self.logging,
                    "mining" => &mut self.mining,
                    "money" => &mut self.money,
                    "bugs" => &mut self.bugs,
                    "fish" => &mut self.fish,
                    _ => &mut self.ore,
                };
                match v.parse::<u32>() {
                    Ok(n) if (MULT_MIN..=MULT_MAX).contains(&n) => *slot = n,
                    _ => warnings.push(format!(
                        "{k}: bad value {v:?} ({MULT_MIN}..{MULT_MAX}), keeping {slot}"
                    )),
                }
            }
            _ => return false,
        }
        true
    }

    /// `DryRun=0 DumpTable=0 Foraging=1 ...`, this half's part of the `[ini]`
    /// line. `Enabled` and `Debug` are left out: the auto-loot half's part
    /// already says them, and they are one key each.
    pub fn summary(&self) -> String {
        format!(
            "DryRun={} DumpTable={} Foraging={} Logging={} Mining={} Ore={} Money={} Bugs={} Fish={}",
            self.dry_run as u8,
            self.dump_table as u8,
            self.foraging,
            self.logging,
            self.mining,
            self.ore,
            self.money,
            self.bugs,
            self.fish
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The parse tests live with the root parser in `crate::config`; what is
    // left here is the config's own defaults and the lock-free mirror the hook
    // reads.

    #[test]
    fn defaults_are_vanilla() {
        let c = Config::default();
        assert!(c.enabled);
        assert!(!c.dry_run);
        assert!(!c.debug);
        assert!(!c.dump_table, "a 22 MB file is written only when asked for");
        assert!(c.all_vanilla());
        for f in [Family::Foraging, Family::Logging, Family::Mining, Family::Ore, Family::Money] {
            assert_eq!(c.multiplier(f), 1);
        }
        for k in [CatchClass::Bug, CatchClass::Fish] {
            assert_eq!(c.catch_multiplier(k), 1);
        }
    }

    #[test]
    fn live_config_starts_equal_to_default() {
        let live = LiveConfig::new();
        assert_eq!(live.load(), Config::default());
        assert!(live.enabled());
        assert!(!live.dry_run());
        assert!(!live.debug());
        assert!(!live.dump_table());
        for f in [Family::Foraging, Family::Logging, Family::Mining, Family::Ore, Family::Money] {
            assert_eq!(live.multiplier(f), 1);
        }
        for k in [CatchClass::Bug, CatchClass::Fish] {
            assert_eq!(live.catch_multiplier(k), 1);
        }
    }

    #[test]
    fn live_config_publish_round_trips() {
        let live = LiveConfig::new();
        let (cfg, w) = crate::config::parse(
            "[Harvest]\nEnabled=0\nDryRun=1\nDebug=1\nDumpTable=1\nForaging=10\nLogging=2\n\
             Mining=5\nOre=100\nMoney=7\nBugs=4\nFish=9\n",
        );
        let cfg = cfg.gatherer;
        assert!(w.is_empty(), "{w:?}");
        live.publish(&cfg);
        assert_eq!(live.load(), cfg);
        assert!(!live.enabled());
        assert!(live.dry_run());
        assert!(live.debug());
        assert!(live.dump_table());
        assert_eq!(live.multiplier(Family::Foraging), 10);
        assert_eq!(live.multiplier(Family::Logging), 2);
        assert_eq!(live.multiplier(Family::Mining), 5);
        assert_eq!(live.multiplier(Family::Ore), 100);
        assert_eq!(live.multiplier(Family::Money), 7);
        assert_eq!(live.catch_multiplier(CatchClass::Bug), 4);
        assert_eq!(live.catch_multiplier(CatchClass::Fish), 9);
    }

    #[test]
    fn live_config_publish_overwrites_previous_values() {
        let live = LiveConfig::new();
        live.publish(&Config {
            enabled: false,
            dry_run: true,
            debug: true,
            dump_table: true,
            foraging: 50,
            ..Config::default()
        });
        assert!(!live.enabled());
        assert!(live.dump_table());
        assert_eq!(live.multiplier(Family::Foraging), 50);

        // A later publish fully replaces the previous snapshot, which is what
        // the reload loop relies on: every field in the new ini wins, not
        // just the ones that changed.
        live.publish(&Config::default());
        assert_eq!(live.load(), Config::default());
    }
}
