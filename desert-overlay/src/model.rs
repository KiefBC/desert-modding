//! The overlay's own typed picture of the two plugins' ini files.
//!
//! The overlay never talks to Desert Looter or Desert Gatherer: **the ini file
//! beside the game exe is the whole contract**, and the file on disk is the
//! source of truth. So this module has to hold its own copy of the two key
//! lists and their defaults rather than linking the plugin crates - each of
//! those is a `cdylib` exporting its own `DllMain`, and two `DllMain`s cannot
//! live in one DLL.
//!
//! **The defaults below are copied from `desert-looter/src/config.rs` and
//! `desert-gatherer/src/config.rs` (`Config::default()`).** They exist only so
//! that a key missing from the file shows the same value in the menu that the
//! plugin will actually use; if a plugin's default changes, change it here in
//! the same commit. The same goes for the accepted ranges, which the widgets
//! clamp to so the overlay can never write a value its plugin would reject.
//!
//! Keys the overlay does not know about (`BagTab`, `LogReceived`, `KeyToggle`,
//! ...) are deliberately ignored on read and never written: [`crate::rewrite`]
//! only touches the lines whose key it was given, so hand-edited keys survive.

use desert_core::ini::{self, Line};

/// A model that maps onto one ini file.
pub trait IniModel: Sized + Clone + PartialEq + Default {
    /// The file's name beside the game exe.
    const FILE_NAME: &'static str;

    /// Comment block put at the top when the overlay has to CREATE the file,
    /// because the user deleted it or never unzipped it. It is deliberately
    /// short: the mod's own shipped ini is the documented one, and this exists
    /// only so a from-nothing file is not an anonymous list of numbers. The
    /// overlay never rewrites this header on a file that already exists.
    const HEADER: &'static str;

    /// Read a model out of ini text. Unknown keys are ignored; a value outside
    /// the plugin's accepted range keeps the default, exactly as the plugin
    /// itself would.
    fn parse(text: &str) -> Self;

    /// The keys this model owns, with their values formatted for the file.
    /// [`crate::rewrite::rewrite`] replaces exactly these keys and leaves the
    /// rest of the file alone.
    fn pairs(&self) -> Vec<(&'static str, String)>;
}

/// `1` / `0`, the spelling the shipped ini templates use.
fn flag(b: bool) -> String {
    if b {
        "1".to_string()
    } else {
        "0".to_string()
    }
}

/// `40` rather than `40.0`, and `6.5` when it has to be. Both parse as `f32`.
fn num(f: f32) -> String {
    if f.fract() == 0.0 {
        format!("{:.0}", f)
    } else {
        format!("{:.1}", f)
    }
}

// ---------------------------------------------------------------------------
// Desert Looter
// ---------------------------------------------------------------------------

/// Accepted ranges, mirroring `desert_looter::config::parse`. The widgets use
/// these as their bounds, so a value the menu produces always survives the
/// plugin's own parse.
pub const SCAN_RANGE: (f32, f32) = (1.0, 200.0);
pub const GATHER_RANGE: (f32, f32) = (1.0, 50.0);
pub const MS_RANGE: (i32, i32) = (100, 60_000);
pub const STACK_LIMIT_RANGE: (i32, i32) = (10, 1_000_000);

/// The subset of `DesertLooter.ini` the menu edits.
#[derive(Debug, Clone, PartialEq)]
pub struct LooterModel {
    pub enabled: bool,
    pub auto_gather: bool,
    pub gather_foraging: bool,
    pub gather_logging: bool,
    pub gather_mining: bool,
    pub gather_ore: bool,
    pub gather_items: bool,
    pub gather_gear: bool,
    pub gather_unarmed: bool,
    pub scan_range: f32,
    pub gather_range: f32,
    pub gather_interval_ms: i32,
    pub node_cooldown_ms: i32,
    pub stack_limit: i32,
}

impl Default for LooterModel {
    /// Copied from `desert-looter/src/config.rs`, `impl Default for Config`.
    fn default() -> Self {
        LooterModel {
            enabled: true,
            auto_gather: false,
            gather_foraging: true,
            gather_logging: true,
            gather_mining: true,
            gather_ore: true,
            gather_items: true,
            gather_gear: false,
            gather_unarmed: true,
            scan_range: 40.0,
            gather_range: 6.0,
            gather_interval_ms: 500,
            node_cooldown_ms: 8000,
            stack_limit: 999,
        }
    }
}

impl IniModel for LooterModel {
    const FILE_NAME: &'static str = "DesertLooter.ini";
    const HEADER: &'static str = "\
; DesertLooter.ini, created by Desert Overlay because the file was missing.
; The keys below are the ones the overlay's menu edits, at Desert Looter's own
; defaults. The mod's shipped DesertLooter.ini documents these and several more
; (BagTab, LogReceived, Debug, KeyToggle/KeyScan/KeyGather/KeyRecord); anything
; you add by hand is preserved - the overlay only ever rewrites its own keys.
[DesertLooter]
";

    fn parse(text: &str) -> Self {
        let mut m = LooterModel::default();
        for line in ini::lines(text) {
            let Line::Pair(k, v) = line else { continue };
            match k.to_ascii_lowercase().as_str() {
                "enabled" => m.enabled = ini::parse_bool(v),
                "autogather" => m.auto_gather = ini::parse_bool(v),
                "gatherforaging" => m.gather_foraging = ini::parse_bool(v),
                "gatherlogging" => m.gather_logging = ini::parse_bool(v),
                "gathermining" => m.gather_mining = ini::parse_bool(v),
                "gatherore" => m.gather_ore = ini::parse_bool(v),
                "gatheritems" => m.gather_items = ini::parse_bool(v),
                "gathergear" => m.gather_gear = ini::parse_bool(v),
                "gatherunarmed" => m.gather_unarmed = ini::parse_bool(v),
                "scanrange" => m.scan_range = clamp_f32(v, SCAN_RANGE, m.scan_range),
                "gatherrange" => m.gather_range = clamp_f32(v, GATHER_RANGE, m.gather_range),
                "gatherinterval" => {
                    m.gather_interval_ms = clamp_i32(v, MS_RANGE, m.gather_interval_ms)
                }
                "nodecooldown" => m.node_cooldown_ms = clamp_i32(v, MS_RANGE, m.node_cooldown_ms),
                "stacklimit" => m.stack_limit = clamp_i32(v, STACK_LIMIT_RANGE, m.stack_limit),
                // Every other key belongs to the plugin alone (BagTab,
                // LogReceived, Debug, the four Key* bindings). Not shown, not
                // written, not disturbed.
                _ => {}
            }
        }
        m
    }

    fn pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Enabled", flag(self.enabled)),
            ("AutoGather", flag(self.auto_gather)),
            ("GatherForaging", flag(self.gather_foraging)),
            ("GatherLogging", flag(self.gather_logging)),
            ("GatherMining", flag(self.gather_mining)),
            ("GatherOre", flag(self.gather_ore)),
            ("GatherItems", flag(self.gather_items)),
            ("GatherGear", flag(self.gather_gear)),
            ("GatherUnarmed", flag(self.gather_unarmed)),
            ("ScanRange", num(self.scan_range)),
            ("GatherRange", num(self.gather_range)),
            ("GatherInterval", self.gather_interval_ms.to_string()),
            ("NodeCooldown", self.node_cooldown_ms.to_string()),
            ("StackLimit", self.stack_limit.to_string()),
        ]
    }
}

// ---------------------------------------------------------------------------
// Desert Gatherer
// ---------------------------------------------------------------------------

/// `desert_gatherer::config::{MULT_MIN, MULT_MAX}`: a multiplier outside this
/// is refused by the plugin and keeps its previous value, so the sliders stop
/// here rather than writing something that would be silently ignored.
pub const MULT_RANGE: (i32, i32) = (1, 100);

/// The subset of `DesertGatherer.ini` the menu edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GathererModel {
    pub enabled: bool,
    pub dry_run: bool,
    pub foraging: i32,
    pub logging: i32,
    pub mining: i32,
    pub ore: i32,
}

impl Default for GathererModel {
    /// Copied from `desert-gatherer/src/config.rs`, `impl Default for Config`.
    fn default() -> Self {
        GathererModel { enabled: true, dry_run: false, foraging: 1, logging: 1, mining: 1, ore: 1 }
    }
}

impl IniModel for GathererModel {
    const FILE_NAME: &'static str = "DesertGatherer.ini";
    const HEADER: &'static str = "\
; DesertGatherer.ini, created by Desert Overlay because the file was missing.
; The keys below are the ones the overlay's menu edits, at Desert Gatherer's own
; defaults (every family at 1x, i.e. vanilla yields). The mod's shipped
; DesertGatherer.ini documents them, the Debug key and the DMM conflict; anything
; you add by hand is preserved - the overlay only ever rewrites its own keys.
[DesertGatherer]
";

    fn parse(text: &str) -> Self {
        let mut m = GathererModel::default();
        for line in ini::lines(text) {
            let Line::Pair(k, v) = line else { continue };
            match k.to_ascii_lowercase().as_str() {
                "enabled" => m.enabled = ini::parse_bool(v),
                "dryrun" => m.dry_run = ini::parse_bool(v),
                "foraging" => m.foraging = clamp_i32(v, MULT_RANGE, m.foraging),
                "logging" => m.logging = clamp_i32(v, MULT_RANGE, m.logging),
                "mining" => m.mining = clamp_i32(v, MULT_RANGE, m.mining),
                "ore" => m.ore = clamp_i32(v, MULT_RANGE, m.ore),
                _ => {}
            }
        }
        m
    }

    fn pairs(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Enabled", flag(self.enabled)),
            ("DryRun", flag(self.dry_run)),
            ("Foraging", self.foraging.to_string()),
            ("Logging", self.logging.to_string()),
            ("Mining", self.mining.to_string()),
            ("Ore", self.ore.to_string()),
        ]
    }
}

// ---------------------------------------------------------------------------

/// Parse `v` and keep it only if it lands inside the plugin's accepted range;
/// otherwise leave `fallback` in place, which is what the plugin does too.
fn clamp_i32(v: &str, (lo, hi): (i32, i32), fallback: i32) -> i32 {
    match v.parse::<i32>() {
        Ok(n) if (lo..=hi).contains(&n) => n,
        _ => fallback,
    }
}

fn clamp_f32(v: &str, (lo, hi): (f32, f32), fallback: f32) -> f32 {
    match v.parse::<f32>() {
        Ok(n) if n.is_finite() && n >= lo && n <= hi => n,
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looter_defaults_match_the_plugin() {
        // Mirrors desert-looter/src/config.rs Config::default(). If this test
        // has to change, the plugin's default changed and the README and the
        // shipped template need the same edit.
        let m = LooterModel::default();
        assert!(m.enabled && !m.auto_gather && m.gather_unarmed);
        assert!(m.gather_foraging && m.gather_logging && m.gather_mining && m.gather_ore);
        assert!(m.gather_items && !m.gather_gear);
        assert_eq!(m.scan_range, 40.0);
        assert_eq!(m.gather_range, 6.0);
        assert_eq!(m.gather_interval_ms, 500);
        assert_eq!(m.node_cooldown_ms, 8000);
        assert_eq!(m.stack_limit, 999);
    }

    #[test]
    fn gatherer_defaults_match_the_plugin() {
        let m = GathererModel::default();
        assert!(m.enabled && !m.dry_run);
        assert_eq!((m.foraging, m.logging, m.mining, m.ore), (1, 1, 1, 1));
    }

    #[test]
    fn a_created_file_reads_back_as_what_was_written() {
        // The from-nothing path: HEADER + pairs must parse back to the model
        // that produced it, or a created file would not say what the menu shows.
        let l = LooterModel::default();
        let body: String = l.pairs().iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        assert_eq!(LooterModel::parse(&format!("{}{body}", LooterModel::HEADER)), l);
        let g = GathererModel { enabled: true, dry_run: false, foraging: 3, logging: 1, mining: 1, ore: 1 };
        let body: String = g.pairs().iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        assert_eq!(GathererModel::parse(&format!("{}{body}", GathererModel::HEADER)), g);
    }

    #[test]
    fn looter_parses_the_keys_it_owns_and_ignores_the_rest() {
        let m = LooterModel::parse(
            "; c\n[DesertLooter]\nEnabled=0\nAutoGather=yes\nGatherLogging=0\n\
             ScanRange=25.5\nGatherRange=4\nGatherInterval=250\nNodeCooldown=12000\n\
             StackLimit=500\nBagTab=1\nKeyToggle=F10\nLogReceived=1\nDebug=1\n",
        );
        assert!(!m.enabled);
        assert!(m.auto_gather);
        assert!(!m.gather_logging);
        assert!(m.gather_foraging, "an untouched family keeps its default");
        assert_eq!(m.scan_range, 25.5);
        assert_eq!(m.gather_range, 4.0);
        assert_eq!(m.gather_interval_ms, 250);
        assert_eq!(m.node_cooldown_ms, 12_000);
        assert_eq!(m.stack_limit, 500);
    }

    #[test]
    fn out_of_range_values_keep_the_default() {
        let d = LooterModel::default();
        let m = LooterModel::parse(
            "ScanRange=0\nGatherRange=80\nGatherInterval=5\nNodeCooldown=99999\nStackLimit=1\n",
        );
        assert_eq!(m.scan_range, d.scan_range);
        assert_eq!(m.gather_range, d.gather_range);
        assert_eq!(m.gather_interval_ms, d.gather_interval_ms);
        assert_eq!(m.node_cooldown_ms, d.node_cooldown_ms);
        assert_eq!(m.stack_limit, d.stack_limit);

        let g = GathererModel::parse("Foraging=0\nLogging=101\nMining=x\n");
        assert_eq!((g.foraging, g.logging, g.mining), (1, 1, 1));
    }

    #[test]
    fn gatherer_parses_its_keys() {
        let g = GathererModel::parse("[DesertGatherer]\nEnabled=1\nDryRun=1\nForaging=10\nOre=100\n");
        assert!(g.enabled && g.dry_run);
        assert_eq!((g.foraging, g.ore), (10, 100));
        assert_eq!((g.logging, g.mining), (1, 1), "untouched multipliers stay vanilla");
    }

    #[test]
    fn pairs_round_trip_through_parse() {
        let l = LooterModel {
            gather_gear: true,
            scan_range: 12.5,
            stack_limit: 4242,
            ..LooterModel::default()
        };
        let text: String =
            l.pairs().iter().map(|(k, v)| format!("{k}={v}\n")).collect::<Vec<_>>().concat();
        assert_eq!(LooterModel::parse(&text), l);

        let g = GathererModel { enabled: false, dry_run: true, foraging: 7, logging: 2, mining: 3, ore: 4 };
        let text: String =
            g.pairs().iter().map(|(k, v)| format!("{k}={v}\n")).collect::<Vec<_>>().concat();
        assert_eq!(GathererModel::parse(&text), g);
    }

    #[test]
    fn floats_are_written_without_a_pointless_decimal() {
        assert_eq!(num(40.0), "40");
        assert_eq!(num(6.5), "6.5");
    }
}
