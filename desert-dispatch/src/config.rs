//! The `[Dispatch]` section of `DesertTooling.ini`, beside the game exe.
//! Missing file, missing section or missing key => defaults.
//!
//! Same shape as `desert_gatherer::config`: `parse` walks only this
//! subsystem's own section through [`ini::lines_in_section`], returns the
//! config plus ready-to-log warnings, and a bad value never replaces the
//! default. `Enabled` and `Debug` exist under other headers too and mean
//! something else there, which is exactly why the section scoping is not
//! optional.
//!
//! [`LiveConfig`] and the once-a-second reload loop in `scan.rs` are the same
//! pair the gatherer has, and for the same reason: this subsystem writes to the
//! game now, so a changed multiplier has to reach the records the game has
//! already parsed, and a multiplier set back to 1 has to put them back. The
//! rule that comes with that pair is not optional either - **compare the
//! section you just parsed against the values you are already running on and do
//! nothing when it did not move.** `DesertTooling.ini` carries four sections,
//! so a `[Looter]` slider being dragged moves this file's modified time once a
//! second, and a pass over every mission and every reward row per drag is what
//! that rule exists to prevent.
//!
//! [`Levers`] is the part of the config the write path actually consults, and
//! it is where `Enabled=0` is turned into "every lever at vanilla" so that
//! reverting is the same code path as applying rather than a special case.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use desert_core::ini::{self, Line};
use desert_core::schema::{Field, Kind, Section};

use crate::node::parsed::SKILL_NONE;

/// Bounds on `MaxLines`. 0 is legal and means "no per-operation lines at all",
/// which is the same thing `LogRecords=0` says and is accepted rather than
/// warned about. The ceiling is generous: the whole census is a few thousand
/// lines at most, and a player who wants all of it should be able to ask.
pub const LINES_MIN: u32 = 0;
pub const LINES_MAX: u32 = 20_000;

/// Bounds on `Speed`. `1` is vanilla and is the only value that writes
/// nothing; `0` would be a division by zero and is refused rather than
/// clamped, so a typo keeps the default instead of quietly meaning something.
/// The ceiling is `desert_gatherer`'s `MULT_MAX`, deliberately: the two
/// subsystems' multipliers should read alike, and 100 is already far past
/// where a dispatch mission stops being one (96 h at x100 is 57 minutes).
pub const SPEED_MIN: u32 = 1;
pub const SPEED_MAX: u32 = 100;

/// Bounds on `Rewards`, the same band and for the same reasons.
pub const REWARDS_MIN: u32 = 1;
pub const REWARDS_MAX: u32 = 100;

/// The shortest a mission may be made: **one tenth of an hour, six minutes**.
///
/// `vanilla / Speed` is integer division, so a 2.0 h mission at `Speed=100`
/// would land on `0`, and a duration of zero is not a faster mission - it is a
/// field the game's own tick compares against and a value no vanilla record
/// has. The floor keeps every written duration inside the shape the table
/// already has: a positive number of tenths of an hour.
pub const MIN_DURATION_TENTHS: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Master switch. 0 = the pass never runs and the game is not read at all.
    pub enabled: bool,
    /// 1 = write one line per dispatch operation: its node, key, group,
    /// duration (raw tenths of an hour and the hours beside them), operator
    /// counts, combat power, step count, `+0xC0`, the skill pair and the
    /// condition keys. The summary, the histograms and the verdict are printed
    /// either way: they are the point, and they are a handful of lines.
    ///
    /// A mission is written once. A later pass, after more records have been
    /// parsed, logs the missions that are new to it and nothing else - the
    /// summary still covers them all.
    pub log_records: bool,
    /// Ceiling on the lines **one pass** writes, so a table that turned out to
    /// be far bigger than expected cannot fill the log.
    ///
    /// It covers every line a pass writes, not only the mission ones: the
    /// reward row and entry lines, and every raw hex line under
    /// [`Self::dump_raw`], are counted against the same budget. `scan::Budget`
    /// is the one place that is enforced, and it is enforced for all of them
    /// because the raw dump used to escape it entirely.
    pub max_lines: u32,
    /// 1 = also log the records that carry no operations at all, and the
    /// reason behind every skipped record or entry.
    pub debug: bool,
    /// 1 = log every operation entry's raw 0x120 bytes as hex.
    ///
    /// For finding a field whose offset we do not know yet. The named offsets
    /// in [`crate::node::parsed`] were recovered by decompilation, and this is
    /// what corrected two of them: `entry+0xC0` reads a constant `1` and is
    /// not the duration, which lives in the middle step of the step list as a
    /// `u32` in tenths of an hour. Dumping the bytes and looking is how that
    /// was settled and how the next unknown field will be. Off by default: it
    /// is one long line per mission.
    pub dump_raw: bool,
    /// 1 = also walk the **reward rows** the missions name.
    ///
    /// A dispatch mission's payout is not in its own record: the operation
    /// entry carries two `u16` row indices into the `dropsetinfo` table
    /// ([`crate::node::parsed::OP_REWARD_1`] and `_2`), and the items and
    /// amounts live there. Those rows were the last layer of the study never
    /// read at runtime until a vanilla capture read all 219 of them on
    /// 2026-09-10, correcting three things about their layout and leaving four
    /// offsets still reading one flat value each;
    /// [`crate::node::parsed::dropset`] says which is which. It stays **on** by
    /// default because those four are still open, and because the row set is
    /// what a reward multiplier would inherit.
    ///
    /// It reads **only** the rows the dispatch operations name, never the
    /// `dropsetinfo` table at large: that table is the game-wide drop table,
    /// and a pass that enumerated it would be measuring every chest and
    /// carcass in the game rather than the 219 rows a dispatch reward mod
    /// would ever touch.
    ///
    /// It is a **diagnostic** switch, not the reward lever's master switch:
    /// with `DumpRewards=0` and [`Self::rewards`] above 1 the rows are still
    /// read and still multiplied, they are just not printed. There is no way
    /// to multiply a row without reading it.
    pub dump_rewards: bool,

    // -- the levers: the keys that write to the game --
    /// Mission duration divisor. `1` = vanilla; `4` means a mission takes a
    /// quarter as long, floored at [`MIN_DURATION_TENTHS`].
    pub speed: u32,
    /// Reward amount multiplier, applied to **both** halves of every
    /// `dropsetinfo` amount pair the missions name. `1` = vanilla.
    ///
    /// It is **not** subject to the game's 10x percent clamp
    /// (`docs/reference-internals.md` section 20.17): the clamp sits on the
    /// reward *percent*, and this scales the base amount the percent is later
    /// applied to, which is upstream of it. `Rewards=50` really is fifty
    /// times.
    pub rewards: u32,
    /// 1 = clear the hard skill gate on the 147 missions that carry one, by
    /// writing [`SKILL_NONE`] - **never `0`**, which is a real `Skill` index
    /// and would make the mission permanently unstartable
    /// (`docs/reference-internals.md` section 20.16).
    pub no_skill_requirement: bool,
    /// 1 = write `1` to every mission's minimum operator count. Again **never
    /// `0`**: `0` means "commit every worker the node has" and is stricter
    /// than `1`, not laxer.
    pub any_operator_count: bool,
    /// 1 = log every change that would be made and write nothing. The same
    /// meaning the `[Gatherer]` key of this name has.
    pub dry_run: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            log_records: true,
            max_lines: 4000,
            debug: false,
            dump_raw: false,
            dump_rewards: true,
            // Every lever ships at vanilla, so installing the plugin changes
            // nothing about dispatch until the player asks for it.
            speed: 1,
            rewards: 1,
            no_skill_requirement: false,
            any_operator_count: false,
            dry_run: false,
        }
    }
}

impl Config {
    /// The levers the write path consults, with `Enabled=0` collapsed to
    /// vanilla.
    ///
    /// This is the one place the master switch is turned into arithmetic, and
    /// it is why there is no separate "revert" code path anywhere:
    /// `Enabled=0`, `Speed=1` and `Rewards=1` all reach [`crate::apply`] as
    /// [`Levers::VANILLA`], and a pass under vanilla levers writes the
    /// remembered original back into whatever is currently parsed.
    pub fn levers(&self) -> Levers {
        if !self.enabled {
            return Levers::VANILLA;
        }
        Levers {
            speed: self.speed,
            rewards: self.rewards,
            clear_skill: self.no_skill_requirement,
            any_operators: self.any_operator_count,
        }
    }
}

// ---------------------------------------------------------------------------
// The levers
// ---------------------------------------------------------------------------

/// What the write path is being asked to make each field say, separated from
/// the config it came out of.
///
/// Every method here is total arithmetic over a **vanilla** value that
/// `crate::remember` handed back - never over what was just read out of the
/// game. That distinction is the whole design: reading the current value and
/// scaling it again would compound on every pass and would round differently
/// each time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Levers {
    pub speed: u32,
    pub rewards: u32,
    pub clear_skill: bool,
    pub any_operators: bool,
}

impl Levers {
    /// Every lever off. What `Enabled=0` reaches the write path as, and what a
    /// revert pass runs with.
    pub const VANILLA: Levers =
        Levers { speed: 1, rewards: 1, clear_skill: false, any_operators: false };

    /// True when nothing would be changed from vanilla. A pass still runs -
    /// that is how anything already written gets put back - but the log calls
    /// it a revert rather than an apply.
    pub fn is_vanilla(&self) -> bool {
        *self == Levers::VANILLA
    }

    /// The duration to write, in tenths of an hour, given the vanilla one.
    ///
    /// Integer division with a floor at [`MIN_DURATION_TENTHS`]. `Speed <= 1`
    /// returns the vanilla number untouched rather than dividing by it, so a
    /// `0` that somehow got past [`parse`] can never divide.
    pub fn duration(&self, vanilla: u32) -> u32 {
        if self.speed <= 1 {
            return vanilla;
        }
        (vanilla / self.speed).max(MIN_DURATION_TENTHS)
    }

    /// The amount to write, given the vanilla one. Used for **both** halves of
    /// the pair, which is what keeps `min <= max` true across the write: the
    /// same non-negative factor applied to both ends of a range preserves its
    /// ordering.
    ///
    /// Saturating, like `desert_gatherer`'s: inside the amount band a vanilla
    /// entry has to be in before it is touched at all, `i64` cannot overflow
    /// here, and saturating instead of wrapping is the correct answer for the
    /// case that says the band check was wrong.
    pub fn amount(&self, vanilla: i64) -> i64 {
        if self.rewards <= 1 {
            return vanilla;
        }
        vanilla.saturating_mul(i64::from(self.rewards))
    }

    /// The required-skill index to write, given the vanilla one.
    ///
    /// **`0xFFFF`, never `0`.** `FUN_1416f09a0` tests `== -1` exactly, and
    /// index `0` is a real row of the `Skill` table that no worker holds, so
    /// `0` here is not "no requirement" - it is a requirement nobody can ever
    /// meet, and the mission never starts again
    /// (`docs/reference-internals.md` section 20.16). DMM writes `0` and is
    /// right to: it patches the *raw* file, where `0` is the key sentinel the
    /// deserialiser turns into `-1`. The raw and parsed spellings of "absent"
    /// are different values, and this function is the parsed one.
    pub fn skill_req(&self, vanilla: u16) -> u16 {
        if self.clear_skill {
            SKILL_NONE
        } else {
            vanilla
        }
    }

    /// The minimum operator count to write, given the vanilla one.
    ///
    /// **`1`, never `0`.** `0` makes the game demand every worker the node
    /// has, so writing it to "remove the requirement" tightens it
    /// (`docs/reference-internals.md` section 20.16, the same trap as
    /// [`Self::skill_req`] in the other direction).
    pub fn min_operators(&self, vanilla: u32) -> u32 {
        if self.any_operators {
            1
        } else {
            vanilla
        }
    }

    /// `Speed=4 Rewards=3 NoSkillRequirement=1 AnyOperatorCount=0`, for the
    /// pass's summary line. All four are always named, so one `grep` over a
    /// log finds what a pass was running with whether or not it was doing
    /// anything.
    pub fn summary(&self) -> String {
        format!(
            "Speed={} Rewards={} NoSkillRequirement={} AnyOperatorCount={}",
            self.speed, self.rewards, self.clear_skill as u8, self.any_operators as u8
        )
    }
}

// ---------------------------------------------------------------------------
// "Is this value one of ours?" - the guard in front of every write
// ---------------------------------------------------------------------------
//
// A field is only ever written when what is in it right now is either the
// vanilla value we remembered or a value this plugin could itself have
// produced from that vanilla under *some* setting of the levers. Anything else
// means the address is not the field we think it is - a record re-parsed into
// a different shape, a stale pointer, a game update that moved an offset - and
// the pass skips it and counts it instead of writing.
//
// The reason it is "under some setting" and not "under the current setting" is
// live editing: dragging `Speed` from 4 to 2 leaves every mission carrying a
// quarter-length duration that the new levers would never produce, and a check
// against the current levers alone would refuse to touch any of them ever
// again.

/// Could `cur` be a duration this plugin wrote from `vanilla`?
///
/// Any `Speed` in `1..=SPEED_MAX` produces something in
/// `MIN_DURATION_TENTHS..=vanilla`, so that band is the answer. It is loose by
/// design: what it rejects is a value *longer* than vanilla or a zero, both of
/// which say the field is not what we think it is.
pub fn derived_duration(vanilla: u32, cur: u32) -> bool {
    cur == vanilla || (cur >= MIN_DURATION_TENTHS && cur <= vanilla)
}

/// Could `cur` be an amount this plugin wrote from `vanilla`? Only an exact
/// whole multiple of it, from 1 up to [`REWARDS_MAX`].
pub fn derived_amount(vanilla: i64, cur: i64) -> bool {
    if cur == vanilla {
        return true;
    }
    if vanilla <= 0 {
        // Nothing is ever multiplied out of a non-positive vanilla amount: the
        // band check refuses those entries before they are remembered.
        return false;
    }
    cur >= vanilla && cur % vanilla == 0 && cur / vanilla <= i64::from(REWARDS_MAX)
}

/// Could `cur` be a required-skill index this plugin wrote from `vanilla`?
/// Only the vanilla value itself or the "none" sentinel - and **never `0`**,
/// which this plugin does not write and must not learn to tolerate.
pub fn derived_skill_req(vanilla: u16, cur: u16) -> bool {
    cur == vanilla || cur == SKILL_NONE
}

/// Could `cur` be a minimum operator count this plugin wrote from `vanilla`?
/// Only the vanilla value itself or `1`.
pub fn derived_min_operators(vanilla: u32, cur: u32) -> bool {
    cur == vanilla || cur == 1
}

// ---------------------------------------------------------------------------
// The live mirror
// ---------------------------------------------------------------------------

/// Lock-free mirror of [`Config`], published by this subsystem's thread after
/// every ini change it detects.
///
/// The same shape `desert_gatherer::config::LiveConfig` has. It matters less
/// here - the only reader is the thread that publishes it, because this
/// subsystem installs no hook and so has no game thread reading its settings -
/// but the two subsystems' configs should read alike, and having one place
/// that always holds the values a pass is currently running with is what lets
/// a log line, a future overlay readback or a second reader see them without
/// re-reading the file.
///
/// `Ordering::Relaxed` throughout, for the same reason: nothing here
/// synchronises with any other memory access, only the eventual visibility of
/// a new value.
pub struct LiveConfig {
    enabled: AtomicBool,
    log_records: AtomicBool,
    max_lines: AtomicU32,
    debug: AtomicBool,
    dump_raw: AtomicBool,
    dump_rewards: AtomicBool,
    speed: AtomicU32,
    rewards: AtomicU32,
    no_skill_requirement: AtomicBool,
    any_operator_count: AtomicBool,
    dry_run: AtomicBool,
}

impl LiveConfig {
    /// Starts equal to `Config::default()`, so a reader that runs before the
    /// first `publish` sees the shipped defaults rather than zeroes.
    pub const fn new() -> Self {
        LiveConfig {
            enabled: AtomicBool::new(true),
            log_records: AtomicBool::new(true),
            max_lines: AtomicU32::new(4000),
            debug: AtomicBool::new(false),
            dump_raw: AtomicBool::new(false),
            dump_rewards: AtomicBool::new(true),
            speed: AtomicU32::new(1),
            rewards: AtomicU32::new(1),
            no_skill_requirement: AtomicBool::new(false),
            any_operator_count: AtomicBool::new(false),
            dry_run: AtomicBool::new(false),
        }
    }

    /// Publish a freshly parsed config. Called from this subsystem's thread
    /// only, at startup and once per detected change of its own section.
    pub fn publish(&self, cfg: &Config) {
        self.enabled.store(cfg.enabled, Ordering::Relaxed);
        self.log_records.store(cfg.log_records, Ordering::Relaxed);
        self.max_lines.store(cfg.max_lines, Ordering::Relaxed);
        self.debug.store(cfg.debug, Ordering::Relaxed);
        self.dump_raw.store(cfg.dump_raw, Ordering::Relaxed);
        self.dump_rewards.store(cfg.dump_rewards, Ordering::Relaxed);
        self.speed.store(cfg.speed, Ordering::Relaxed);
        self.rewards.store(cfg.rewards, Ordering::Relaxed);
        self.no_skill_requirement.store(cfg.no_skill_requirement, Ordering::Relaxed);
        self.any_operator_count.store(cfg.any_operator_count, Ordering::Relaxed);
        self.dry_run.store(cfg.dry_run, Ordering::Relaxed);
    }

    /// Snapshot the live values as a plain [`Config`].
    pub fn load(&self) -> Config {
        Config {
            enabled: self.enabled.load(Ordering::Relaxed),
            log_records: self.log_records.load(Ordering::Relaxed),
            max_lines: self.max_lines.load(Ordering::Relaxed),
            debug: self.debug.load(Ordering::Relaxed),
            dump_raw: self.dump_raw.load(Ordering::Relaxed),
            dump_rewards: self.dump_rewards.load(Ordering::Relaxed),
            speed: self.speed.load(Ordering::Relaxed),
            rewards: self.rewards.load(Ordering::Relaxed),
            no_skill_requirement: self.no_skill_requirement.load(Ordering::Relaxed),
            any_operator_count: self.any_operator_count.load(Ordering::Relaxed),
            dry_run: self.dry_run.load(Ordering::Relaxed),
        }
    }

    /// The levers a pass should run with right now. See [`Config::levers`].
    pub fn levers(&self) -> Levers {
        self.load().levers()
    }

    /// `true` = log every change and write nothing.
    pub fn dry_run(&self) -> bool {
        self.dry_run.load(Ordering::Relaxed)
    }
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// The live config this subsystem is running on.
pub static LIVE: LiveConfig = LiveConfig::new();

// ---------------------------------------------------------------------------
// The menu schema
// ---------------------------------------------------------------------------

/// The `[Header]` this subsystem owns inside the shared ini. Named once here
/// and used by [`schema`], by [`parse`] and by `scan::start`'s log line, so
/// there is nothing to keep in step.
pub const INI_SECTION: &str = "Dispatch";

/// The one ini every subsystem shares. Only [`schema`] names it; `scan` takes
/// the path from `Section::ini`.
const INI_FILE: &str = "DesertTooling.ini";

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

/// What Desert Overlay draws for the `[Dispatch]` section of
/// `DesertTooling.ini`. Handed to the overlay at startup by `desert-tooling`,
/// which also seeds the ini from it.
///
/// Order 40: after the looter's 10 and the gatherer's 20 as asked, and after
/// the overlay's 30 as well, because 30 was already taken - `Order` is what
/// puts the menu's sections in order and two sections sharing a number would
/// leave their relative position to the title tiebreak. Last in the menu,
/// which is where a diagnostic belongs anyway.
pub fn schema() -> Section {
    let d = Config::default();
    Section {
        title: "Dispatch Missions".to_string(),
        ini: INI_FILE.to_string(),
        ini_section: INI_SECTION.to_string(),
        // One .asi, always loaded: there is no module whose absence could
        // grey this section out.
        module: None,
        order: 40,
        notice: Some(
            "These settings edit tables the game re-reads from disk every time it starts, so they \
             are not saved into your game: set them back and the missions go back, and removing \
             the plugin leaves nothing to undo. The reward multiplier banks nothing either - a \
             reward already waiting on you is worked out from the table at the moment it lands, so \
             set Rewards back to 1 and it pays vanilla. Nor does anything else on this game build. \
             The game has code to bank a percentage figure when a mission FINISHES, part of it a \
             bonus for sending more workers than the mission needed - the one thing 'Any number of \
             workers' could have inflated - but both switches that code sits behind are off on \
             build 25116796: no mission defers a payout, and the figure ignores the worker count. \
             The plugin reads both at startup and says so on the [banking] line of the log; check \
             it after a game update, because an update can flip them. Faster missions and no skill \
             requirement are nowhere in that figure at all. The dump below stays read-only \
             whatever the levers say."
                .to_string(),
        ),
        presets_label: None,
        presets: Vec::new(),
        fields: vec![
            f(
                "Enabled",
                "Enabled",
                Kind::Bool { default: d.enabled },
                "Master switch. Turned off while the game runs, it puts every mission and reward it has changed back to vanilla, and turning it on again re-applies them. Turned off before launch, the pass never runs and nothing in the game is read at all - and that lasts the session: ticking this back on does nothing until the next launch, because there is no pass left to wake up.",
            ),
            Field {
                heading: Some("Missions:".to_string()),
                ..f(
                    "Speed",
                    "Faster missions",
                    Kind::Int {
                        default: i64::from(d.speed),
                        min: i64::from(SPEED_MIN),
                        max: i64::from(SPEED_MAX),
                        step: 1,
                        slider: true,
                        format: Some("%dx".to_string()),
                    },
                    "Divide every dispatch mission's duration by this. 1 = vanilla (2h to 96h); 4 makes a 16h mission take 4h. Never goes below 6 minutes. This reaches missions already under way, not only the ones you have not sent yet.",
                )
            },
            f(
                "NoSkillRequirement",
                "No skill requirement",
                Kind::Bool { default: d.no_skill_requirement },
                "Clear the skill a mission demands of an assigned worker, on the 147 missions that demand one. The bonus a mission pays for a skilled worker is a different field and is left alone. Checked when a mission starts, so one already out keeps running - but a repeating mission checks again each time it restarts, and stops with an error once this is off.",
            ),
            f(
                "AnyOperatorCount",
                "Any number of workers",
                Kind::Bool { default: d.any_operator_count },
                "Let every mission start with a single worker, whatever it asks for. 693 of the game's 936 missions already ask for one, so this changes the other 243. Checked at start, with the same repeating-mission caveat as the skill requirement. This is the setting the notice above is about.",
            ),
            Field {
                heading: Some("Rewards:".to_string()),
                ..f(
                    "Rewards",
                    "Reward multiplier",
                    Kind::Int {
                        default: i64::from(d.rewards),
                        min: i64::from(REWARDS_MIN),
                        max: i64::from(REWARDS_MAX),
                        step: 1,
                        slider: true,
                        format: Some("%dx".to_string()),
                    },
                    "Multiply how much of each item a mission pays. 1 = vanilla. Only the reward rows dispatch missions name are touched, never the game's wider drop table. The game reads the amounts at the moment items land, so this reaches rewards already waiting to be paid - and, the other way round, banks nothing: set it back to 1 and anything still waiting pays vanilla.",
                )
            },
            f(
                "LogRecords",
                "Log each mission",
                Kind::Bool { default: d.log_records },
                "One line per dispatch mission: its node, key, group, duration in hours, operator counts and condition keys. The summary and the condition histogram are printed either way.",
            ),
            f(
                "MaxLines",
                "Line limit",
                Kind::Int {
                    default: i64::from(d.max_lines),
                    min: i64::from(LINES_MIN),
                    max: i64::from(LINES_MAX),
                    step: 50,
                    slider: false,
                    format: None,
                },
                "Ceiling on the lines one pass writes - mission lines, reward lines, raw hex lines and the per-change lines Debug and DryRun add - so an unexpectedly large table cannot fill the log. The summaries are never capped.",
            ),
            Field {
                heading: Some("Diagnostics:".to_string()),
                ..f(
                    "DryRun",
                    "Dry run",
                    Kind::Bool { default: d.dry_run },
                    "Show, do not touch. Every change the settings above ask for is logged and nothing is written to the game.",
                )
            },
            f(
                "Debug",
                "Debug",
                Kind::Bool { default: d.debug },
                "Also log the faction nodes that carry no missions, every change written one line at a time, and the reason behind every record or entry the pass skipped. Useful only when the numbers look wrong.",
            ),
            f(
                "DumpRaw",
                "Dump raw bytes",
                Kind::Bool { default: d.dump_raw },
                "Log every mission's raw 0x120 bytes as hex, and the raw bytes of every reward row and reward entry when those are dumped too, for locating a field whose offset is not known yet. One long line each.",
            ),
            f(
                "DumpRewards",
                "Dump reward rows",
                Kind::Bool { default: d.dump_rewards },
                "Dump the reward rows the missions name into the log: the items and amounts a mission pays. A diagnostic only - the reward multiplier above reads and edits those rows whether or not this is on.",
            ),
        ],
    }
}

/// Parse the `[Dispatch]` section of the shared ini. Unknown keys and bad
/// values *inside that section* are reported back so they can be logged; the
/// config always comes back usable.
///
/// Keys under another subsystem's header, and keys before any header at all,
/// are skipped without a warning: they are not this subsystem's to complain
/// about, and `Enabled` under `[Looter]` is a different setting entirely.
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
        match k.to_ascii_lowercase().as_str() {
            "enabled" => cfg.enabled = ini::parse_bool(v),
            "logrecords" => cfg.log_records = ini::parse_bool(v),
            "debug" => cfg.debug = ini::parse_bool(v),
            "dumpraw" => cfg.dump_raw = ini::parse_bool(v),
            "dumprewards" => cfg.dump_rewards = ini::parse_bool(v),
            "noskillrequirement" => cfg.no_skill_requirement = ini::parse_bool(v),
            "anyoperatorcount" => cfg.any_operator_count = ini::parse_bool(v),
            "dryrun" => cfg.dry_run = ini::parse_bool(v),
            "maxlines" => match v.parse::<u32>() {
                Ok(n) if (LINES_MIN..=LINES_MAX).contains(&n) => cfg.max_lines = n,
                _ => warnings.push(format!(
                    "{k}: bad value {v:?} ({LINES_MIN}..{LINES_MAX}), keeping {}",
                    cfg.max_lines
                )),
            },
            // Both levers refuse a value outside the band rather than clamping
            // it: `Speed=0` is a division by zero and `Rewards=0` would zero
            // out the table, and neither is a thing anyone meant to ask for.
            "speed" => match v.parse::<u32>() {
                Ok(n) if (SPEED_MIN..=SPEED_MAX).contains(&n) => cfg.speed = n,
                _ => warnings.push(format!(
                    "{k}: bad value {v:?} ({SPEED_MIN}..{SPEED_MAX}), keeping {}",
                    cfg.speed
                )),
            },
            "rewards" => match v.parse::<u32>() {
                Ok(n) if (REWARDS_MIN..=REWARDS_MAX).contains(&n) => cfg.rewards = n,
                _ => warnings.push(format!(
                    "{k}: bad value {v:?} ({REWARDS_MIN}..{REWARDS_MAX}), keeping {}",
                    cfg.rewards
                )),
            },
            _ => warnings.push(format!("unknown key {k:?}")),
        }
    }
    (cfg, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use desert_core::schema as sch;

    #[test]
    fn defaults_are_a_capped_dump_of_everything() {
        let c = Config::default();
        assert!(c.enabled);
        assert!(c.log_records);
        assert_eq!(c.max_lines, 4000);
        assert!(!c.debug);
        assert!(!c.dump_raw);
        // On by default: the reward rows are the one layer nothing has read at
        // runtime yet, and confirming them is what the next launch is for.
        assert!(c.dump_rewards);
    }

    /// The install-and-nothing-happens property. Every lever ships at vanilla,
    /// so a fresh install changes no dispatch mission until the player asks.
    #[test]
    fn every_lever_ships_at_vanilla() {
        let c = Config::default();
        assert_eq!(c.speed, SPEED_MIN);
        assert_eq!(c.rewards, REWARDS_MIN);
        assert!(!c.no_skill_requirement);
        assert!(!c.any_operator_count);
        assert!(!c.dry_run);
        assert!(c.levers().is_vanilla(), "the default config writes nothing");
    }

    #[test]
    fn parses_every_key() {
        let (c, w) = parse(
            "; comment\n[Dispatch]\nEnabled=0\nLogRecords=no\nMaxLines=1234\nDebug=on\n\
             DumpRaw=1\nDumpRewards=0\nSpeed=4\nRewards=3\nNoSkillRequirement=1\n\
             AnyOperatorCount=yes\nDryRun=1\n",
        );
        assert!(!c.enabled);
        assert!(!c.log_records);
        assert_eq!(c.max_lines, 1234);
        assert!(c.debug);
        assert!(c.dump_raw);
        assert!(!c.dump_rewards);
        assert_eq!(c.speed, 4);
        assert_eq!(c.rewards, 3);
        assert!(c.no_skill_requirement);
        assert!(c.any_operator_count);
        assert!(c.dry_run);
        assert!(w.is_empty(), "{w:?}");
    }

    /// `Enabled=0` is not a separate revert path: it is the levers reading
    /// vanilla, which is what makes a revert pass the same code as an apply
    /// pass.
    #[test]
    fn enabled_zero_collapses_every_lever_to_vanilla() {
        let (c, w) = parse(
            "[Dispatch]\nEnabled=0\nSpeed=8\nRewards=9\nNoSkillRequirement=1\nAnyOperatorCount=1\n",
        );
        assert!(w.is_empty(), "{w:?}");
        // The values are still parsed and still in the file...
        assert_eq!(c.speed, 8);
        assert!(c.no_skill_requirement);
        // ...and the write path is handed vanilla anyway.
        assert_eq!(c.levers(), Levers::VANILLA);
        assert!(c.levers().is_vanilla());
    }

    #[test]
    fn a_lever_out_of_band_keeps_the_default_and_warns() {
        let d = Config::default();
        // 0 is the dangerous one in both: a division by zero, and a table of
        // zeroed rewards.
        let (c, w) = parse("[Dispatch]\nSpeed=0\nRewards=0\n");
        assert_eq!(c.speed, d.speed);
        assert_eq!(c.rewards, d.rewards);
        assert_eq!(w.len(), 2, "{w:?}");
        let (c, w) = parse(&format!(
            "[Dispatch]\nSpeed={}\nRewards={}\n",
            SPEED_MAX + 1,
            REWARDS_MAX + 1
        ));
        assert_eq!(c.speed, d.speed);
        assert_eq!(c.rewards, d.rewards);
        assert_eq!(w.len(), 2, "{w:?}");
        // The edges themselves are fine.
        let (c, w) = parse(&format!("[Dispatch]\nSpeed={SPEED_MAX}\nRewards={REWARDS_MAX}\n"));
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(c.speed, SPEED_MAX);
        assert_eq!(c.rewards, REWARDS_MAX);
    }

    // -----------------------------------------------------------------------
    // The arithmetic. This is what actually goes into the game's memory, so it
    // is tested here rather than only through the pass that writes it.
    // -----------------------------------------------------------------------

    fn levers(speed: u32, rewards: u32, clear_skill: bool, any_operators: bool) -> Levers {
        Levers { speed, rewards, clear_skill, any_operators }
    }

    #[test]
    fn vanilla_levers_change_nothing_at_all() {
        let v = Levers::VANILLA;
        for tenths in [10u32, 20, 160, 960, 4320] {
            assert_eq!(v.duration(tenths), tenths);
        }
        for amount in [1i64, 2, 3, 1000, 1_000_000] {
            assert_eq!(v.amount(amount), amount);
        }
        assert_eq!(v.skill_req(61), 61);
        assert_eq!(v.skill_req(SKILL_NONE), SKILL_NONE);
        assert_eq!(v.min_operators(8), 8);
    }

    #[test]
    fn speed_divides_the_duration_and_floors_at_six_minutes() {
        let l = levers(4, 1, false, false);
        assert_eq!(l.duration(160), 40, "16.0h at x4 is 4.0h");
        assert_eq!(l.duration(960), 240, "96.0h at x4 is 24.0h");
        // Integer division, truncating: 2.0h at x3 is 0.6h, not 0.66h.
        assert_eq!(levers(3, 1, false, false).duration(20), 6);
        // The floor. Vanilla's shortest mission is 20 tenths; nothing may
        // reach zero, whatever the divisor.
        assert_eq!(levers(SPEED_MAX, 1, false, false).duration(20), MIN_DURATION_TENTHS);
        assert_eq!(levers(SPEED_MAX, 1, false, false).duration(10), MIN_DURATION_TENTHS);
        assert_eq!(levers(SPEED_MAX, 1, false, false).duration(4320), 43);
        // A duration is never lengthened, and never left at zero.
        for tenths in [10u32, 20, 55, 160, 961, 4320] {
            for speed in 1..=SPEED_MAX {
                let got = levers(speed, 1, false, false).duration(tenths);
                assert!(got >= MIN_DURATION_TENTHS, "{tenths} / {speed} = {got}");
                assert!(got <= tenths, "{tenths} / {speed} = {got} is longer than vanilla");
            }
        }
    }

    #[test]
    fn rewards_scales_both_halves_and_keeps_min_below_max() {
        let l = levers(1, 3, false, false);
        // The 136 entries whose halves differ are the whole reason both are
        // scaled: 2..3 must become 6..9, never 6..3.
        assert_eq!((l.amount(2), l.amount(3)), (6, 9));
        assert_eq!((l.amount(1), l.amount(1)), (3, 3));
        for (min, max) in [(1i64, 1i64), (2, 3), (3, 5), (1, 1000), (999_999, 1_000_000)] {
            for rewards in 1..=REWARDS_MAX {
                let l = levers(1, rewards, false, false);
                let (wmin, wmax) = (l.amount(min), l.amount(max));
                assert!(wmin <= wmax, "{min}..{max} x{rewards} inverted to {wmin}..{wmax}");
                assert!(wmin >= min && wmax >= max);
            }
        }
    }

    /// The `0` trap, both halves of it. `docs/reference-internals.md` section
    /// 20.16: writing `0` to either of these is stricter than the value it
    /// replaces, and for the skill index it is permanent.
    #[test]
    fn clearing_a_requirement_never_writes_zero() {
        let l = levers(1, 1, true, true);
        assert_eq!(l.skill_req(61), SKILL_NONE);
        assert_eq!(l.skill_req(SKILL_NONE), SKILL_NONE);
        assert_eq!(l.min_operators(8), 1);
        assert_eq!(l.min_operators(1), 1);
        // Over every vanilla value either field is known to take, and over
        // every setting of the levers, neither ever produces a zero.
        for vanilla in [0u16, 1, 57, 61, 73, SKILL_NONE] {
            for clear in [false, true] {
                let got = levers(1, 1, clear, false).skill_req(vanilla);
                assert!(got == vanilla || got == SKILL_NONE, "{vanilla} -> {got}");
                assert!(clear != (got == SKILL_NONE) || got != 0 || vanilla == 0);
            }
        }
        for vanilla in [1u32, 2, 3, 5, 8, 10, 64] {
            for any in [false, true] {
                let got = levers(1, 1, false, any).min_operators(vanilla);
                assert_ne!(got, 0, "{vanilla} -> 0 would demand every worker the node has");
            }
        }
        // The one case that would be a catastrophe: clearing must not turn a
        // real skill index into index 0.
        assert_ne!(levers(1, 1, true, false).skill_req(61), 0);
    }

    /// Apply, then apply again, then re-scale, then revert: every field comes
    /// back to the number it started at, and nothing along the way is refused.
    ///
    /// This is the property the whole write path exists to keep, and it is
    /// driven through [`crate::node::plan_mission`] and
    /// [`crate::node::plan_entry`] - the same decisions `crate::apply` makes,
    /// with the `want == 0` locks and the `derived_*` guards in them - rather
    /// than through the lever arithmetic alone. The test that used to be here
    /// called the arithmetic, **discarded** every applied value and then asserted
    /// that `Levers::VANILLA` was the identity over the vanilla inputs: a
    /// restatement of `is_vanilla`, which is why its own `(10, 0, 3)` fixture -
    /// the skill-index-`0` case the `want == 0` guard used to refuse for ever -
    /// could not fail it.
    #[test]
    fn apply_then_revert_returns_every_field_to_vanilla() {
        use crate::node::{plan_entry, plan_mission, DropsetEntry, RewardPlan};
        use crate::remember::{Mission, Reward};

        /// One pass over one mission field-set: plan, then write what the plan
        /// asked for. Panics if the plan refused anything - every value here is
        /// one this plugin produced itself, so a refusal is the bug.
        fn step(van: Mission, cur: Mission, lev: &Levers) -> Mission {
            let p = plan_mission(van, cur, lev);
            assert_eq!(p.refused, 0, "{cur:?} under {} was refused", lev.summary());
            Mission {
                key: van.key,
                duration_tenths: p.duration.unwrap_or(cur.duration_tenths),
                skill_req: p.skill.unwrap_or(cur.skill_req),
                min_operators: p.min_operators.unwrap_or(cur.min_operators),
            }
        }

        /// The same for one reward entry. `kind` decides whether it is an item
        /// drop at all.
        fn reward_step(van: Reward, cur: Reward, kind: u8, lev: &Levers) -> (Reward, RewardPlan) {
            let e = DropsetEntry {
                index: usize::from(cur.index),
                item_row: cur.item_row,
                amount_min: cur.amount_min,
                amount_max: cur.amount_max,
                weight: 1,
                kind,
                conds: [0xFFFF; 3],
            };
            let plan = plan_entry(van, &e, lev);
            let after = match plan {
                RewardPlan::Write { want_min, want_max } => {
                    Reward { amount_min: want_min, amount_max: want_max, ..cur }
                }
                // Unchanged, refused, not an item drop, wrong item: the field
                // keeps what it had.
                _ => cur,
            };
            assert!(after.amount_min <= after.amount_max, "{after:?} inverted");
            (after, plan)
        }

        // The last two are the cases the two `want == 0` locks nearly broke: a
        // vanilla skill requirement of index `0` and a vanilla minimum headcount
        // of `0`, both legal and neither present on build 25116796.
        let missions: [(u32, u16, u32); 7] = [
            (20, SKILL_NONE, 1),
            (160, 61, 5),
            (240, 57, 8),
            (540, SKILL_NONE, 2),
            (960, 73, 10),
            (10, 0, 3),
            (4320, 61, 0),
        ];
        // 136 of 725 live entries have differing halves; 589 do not. Both shapes
        // are here, and `kind` 13 - not an item drop - is below.
        let amounts: [(i64, i64); 5] = [(1, 1), (2, 3), (3, 5), (7, 7), (1, 1000)];
        let off = Levers::VANILLA;

        for speed in [1u32, 2, 3, 4, 7, 100] {
            for rewards in [1u32, 2, 3, 10, 100] {
                for clear in [false, true] {
                    for any in [false, true] {
                        let on = levers(speed, rewards, clear, any);
                        for &(dur, skill, ops) in &missions {
                            let van = Mission {
                                key: 1,
                                duration_tenths: dur,
                                skill_req: skill,
                                min_operators: ops,
                            };
                            // Apply, then apply again: the second pass must find
                            // nothing left to do, which is what idempotence is.
                            let applied = step(van, van, &on);
                            assert_eq!(step(van, applied, &on), applied, "not idempotent");
                            // Re-scale from the remembered vanilla, never from
                            // what the first pass left in the field: at Speed=2
                            // the duration is half of *vanilla*, not half of the
                            // already-divided number sitting in the field.
                            let re = levers(2, rewards, clear, any);
                            let half = step(van, applied, &re);
                            assert_eq!(
                                half.duration_tenths,
                                re.duration(dur),
                                "re-scaled from the field rather than from vanilla"
                            );
                            // Revert, from whatever is there now.
                            assert_eq!(step(van, half, &off), van, "revert from {half:?}");
                            assert_eq!(step(van, applied, &off), van, "revert from {applied:?}");
                        }
                        for &(min, max) in &amounts {
                            let van = Reward {
                                row: 6244,
                                index: 1,
                                amount_min: min,
                                amount_max: max,
                                item_row: 1701,
                            };
                            let (applied, _) = reward_step(van, van, 0, &on);
                            assert_eq!((applied.amount_min, applied.amount_max), (on.amount(min), on.amount(max)));
                            assert_eq!(reward_step(van, applied, 0, &on).0, applied, "not idempotent");
                            let (bigger, _) = reward_step(van, applied, 0, &levers(speed, 7, clear, any));
                            assert_eq!((bigger.amount_min, bigger.amount_max), (min * 7, max * 7));
                            assert_eq!(reward_step(van, bigger, 0, &off).0, van, "revert from {bigger:?}");
                            assert_eq!(reward_step(van, applied, 0, &off).0, van);

                            // A `kind == 13` entry is not an item drop: no lever
                            // reaches it, in either direction.
                            let (after, plan) = reward_step(van, van, 13, &on);
                            assert_eq!(plan, RewardPlan::NotAnItemDrop);
                            assert_eq!(after, van);
                        }
                    }
                }
            }
        }
    }

    /// A revert has to accept whatever a *partly* applied pass left behind:
    /// the guard in front of every write is what decides that, and it must say
    /// yes to every value the apply arithmetic can produce.
    #[test]
    fn every_value_the_levers_produce_is_recognised_as_ours() {
        for vanilla in [10u32, 20, 160, 540, 960, 4320] {
            assert!(derived_duration(vanilla, vanilla), "vanilla itself");
            for speed in 1..=SPEED_MAX {
                let written = levers(speed, 1, false, false).duration(vanilla);
                assert!(derived_duration(vanilla, written), "{vanilla} / {speed} = {written}");
            }
            // What is refused: a longer duration, and a zero.
            assert!(!derived_duration(vanilla, vanilla + 1));
            assert!(!derived_duration(vanilla, 0));
        }
        for vanilla in [1i64, 2, 3, 7, 1000, 1_000_000] {
            assert!(derived_amount(vanilla, vanilla));
            for rewards in 1..=REWARDS_MAX {
                let written = levers(1, rewards, false, false).amount(vanilla);
                assert!(derived_amount(vanilla, written), "{vanilla} x{rewards} = {written}");
            }
            assert!(!derived_amount(vanilla, vanilla.saturating_mul(101)), "past the ceiling");
            assert!(!derived_amount(vanilla, 0), "nothing this plugin writes is zero");
        }
        // A stray value nothing here could have written.
        assert!(!derived_amount(3, 7));
        assert!(!derived_amount(0, 5), "a non-positive vanilla is never a base");
        for vanilla in [0u16, 61, SKILL_NONE] {
            assert!(derived_skill_req(vanilla, vanilla));
            assert!(derived_skill_req(vanilla, SKILL_NONE));
        }
        assert!(!derived_skill_req(61, 0), "0 is the one value this must never accept");
        assert!(!derived_skill_req(61, 62));
        for vanilla in [1u32, 5, 8, 10] {
            assert!(derived_min_operators(vanilla, vanilla));
            assert!(derived_min_operators(vanilla, 1));
        }
        assert!(!derived_min_operators(8, 0));
        assert!(!derived_min_operators(8, 9));
    }

    #[test]
    fn the_lever_summary_always_names_all_four() {
        assert_eq!(
            levers(4, 3, true, false).summary(),
            "Speed=4 Rewards=3 NoSkillRequirement=1 AnyOperatorCount=0"
        );
        assert_eq!(
            Levers::VANILLA.summary(),
            "Speed=1 Rewards=1 NoSkillRequirement=0 AnyOperatorCount=0"
        );
    }

    #[test]
    fn the_live_mirror_round_trips_a_config() {
        let live = LiveConfig::new();
        assert_eq!(live.load(), Config::default(), "a mirror nobody published is the default");
        let cfg = Config {
            enabled: true,
            log_records: false,
            max_lines: 77,
            debug: true,
            dump_raw: true,
            dump_rewards: false,
            speed: 6,
            rewards: 9,
            no_skill_requirement: true,
            any_operator_count: true,
            dry_run: true,
        };
        live.publish(&cfg);
        assert_eq!(live.load(), cfg);
        assert_eq!(live.levers(), cfg.levers());
        assert!(live.dry_run());
        // And the master switch reaches the levers through the mirror too.
        live.publish(&Config { enabled: false, ..cfg });
        assert_eq!(live.levers(), Levers::VANILLA);
    }

    #[test]
    fn case_insensitive_keys() {
        let (c, w) = parse("[dIsPaTcH]\nENABLED=0\nmaxlines=0\n");
        assert!(!c.enabled);
        assert_eq!(c.max_lines, LINES_MIN);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn bad_values_keep_the_default_and_warn() {
        let d = Config::default();
        let (c, w) = parse("[Dispatch]\nMaxLines=abc\nMaxLines=99999\nJunk=1\nnoequals\n");
        assert_eq!(c.max_lines, d.max_lines);
        // two bad values, one unknown key, one syntax error
        assert_eq!(w.len(), 4, "{w:?}");
        assert!(w[0].starts_with("MaxLines: bad value"), "{w:?}");
    }

    #[test]
    fn range_edges() {
        let (c, w) = parse(&format!("[Dispatch]\nMaxLines={LINES_MAX}\n"));
        assert_eq!(c.max_lines, LINES_MAX);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn empty_text_is_the_default() {
        let (c, w) = parse("");
        assert_eq!(c, Config::default());
        assert!(w.is_empty());
    }

    /// The whole point of the section scoping: `Enabled` and `Debug` mean
    /// something different in every subsystem.
    #[test]
    fn keys_under_another_subsystems_header_are_ignored_silently() {
        let (c, w) = parse(
            "[Looter]\nEnabled=0\nDebug=1\nMaxLines=7\n[Dispatch]\nMaxLines=9\n[Overlay]\nScale=2.0\n",
        );
        assert!(w.is_empty(), "another section's keys are not ours to warn about: {w:?}");
        assert_eq!(c.max_lines, 9);
        assert!(c.enabled, "[Looter] Enabled=0 must not disable this subsystem");
        assert!(!c.debug);
    }

    // -----------------------------------------------------------------------
    // The menu schema. The project convention: every key the schema names is
    // one `parse` accepts, and every default it declares is the one `parse`
    // would have produced anyway. Nothing on the overlay side checks this - it
    // never sees `Config` at all - so this is the only thing keeping the two
    // halves in step.
    // -----------------------------------------------------------------------

    #[test]
    fn schema_defaults_parse_back_to_the_default_config() {
        let text = sch::render_ini_defaults(&schema(), "");
        assert!(text.starts_with("[Dispatch]\n"), "{text}");
        let (cfg, w) = parse(&text);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg, Config::default());
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
    fn the_schema_names_the_one_shared_ini_and_this_subsystems_section() {
        let s = schema();
        assert_eq!(s.ini, "DesertTooling.ini");
        assert_eq!(s.ini_section, INI_SECTION);
        assert_eq!(s.ini_section, "Dispatch");
        assert_eq!(s.module, None);
        assert_eq!(s.order, 40, "after the gatherer's 20 and the overlay's 30");
        assert!(s.presets.is_empty());
        // What a setting can and cannot leave behind has to be stated where a
        // player will read it. On build 25116796 the answer is "nothing" - both
        // banking gates read 0 - but the notice still has to say so, because
        // silence reads as "nobody checked".
        let notice = s.notice.clone().unwrap_or_default();
        assert!(notice.contains("re-reads"), "the tables are not saved: {notice}");
        assert!(notice.contains("FINISHES"), "the banked payout must be called out: {notice}");
        assert!(!notice.contains("Read-only"), "it is not read-only any more: {notice}");
    }

    #[test]
    fn schema_ranges_are_the_ones_parse_enforces() {
        let s = schema();
        match s.field("MaxLines").map(|f| f.kind.clone()) {
            Some(Kind::Int { min, max, default, .. }) => {
                assert_eq!(min, i64::from(LINES_MIN));
                assert_eq!(max, i64::from(LINES_MAX));
                assert_eq!(default, i64::from(Config::default().max_lines));
            }
            other => panic!("MaxLines is not an int field: {other:?}"),
        }
        let (c, w) = parse(&format!("[Dispatch]\nMaxLines={}\n", LINES_MAX + 1));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(c, Config::default());

        for (key, min, max, default) in [
            ("Speed", SPEED_MIN, SPEED_MAX, Config::default().speed),
            ("Rewards", REWARDS_MIN, REWARDS_MAX, Config::default().rewards),
        ] {
            match s.field(key).map(|f| f.kind.clone()) {
                Some(Kind::Int { min: lo, max: hi, default: d, slider, .. }) => {
                    assert_eq!(lo, i64::from(min), "{key}");
                    assert_eq!(hi, i64::from(max), "{key}");
                    assert_eq!(d, i64::from(default), "{key}");
                    assert!(slider, "{key} is a slider like the gatherer's multipliers");
                }
                other => panic!("{key} is not an int field: {other:?}"),
            }
            let (c, w) = parse(&format!("[Dispatch]\n{key}={}\n", max + 1));
            assert_eq!(w.len(), 1, "{key}: {w:?}");
            assert_eq!(c, Config::default(), "{key}");
        }
    }

    /// Prints this section as it is seeded into `DesertTooling.ini`.
    #[test]
    #[ignore]
    fn show_schema() {
        println!("{}", sch::render_ini_defaults(&schema(), ""));
    }
}
