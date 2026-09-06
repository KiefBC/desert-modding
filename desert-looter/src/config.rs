//! `DesertLooter.ini` beside the game exe. Missing file or key => defaults.
//!
//! Only Desert Looter's own keys live here; the ini tokeniser, the truthy
//! spellings and the virtual-key name table are shared in `desert_core::ini`.

use desert_core::ini::{self, Line};

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub enabled: bool,
    pub debug: bool,
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
            scan_range: 40.0,
            gather_range: 6.0,
            auto_gather: false,
            gather_unarmed: true,
            gather_items: true,
            gather_gear: false,
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
            "scanrange" => match v.parse::<f32>() {
                Ok(f) if f > 0.0 => cfg.scan_range = f,
                _ => warnings.push(format!("ScanRange: bad value {v:?}, keeping {}", cfg.scan_range)),
            },
            "autogather" => cfg.auto_gather = bool_of(v),
            "gatherunarmed" => cfg.gather_unarmed = bool_of(v),
            "gatheritems" => cfg.gather_items = bool_of(v),
            "gathergear" => cfg.gather_gear = bool_of(v),
            "stacklimit" => match v.parse::<u32>() {
                Ok(n) if (10..=1_000_000).contains(&n) => cfg.stack_limit = n,
                _ => warnings.push(format!("StackLimit: bad value {v:?}, keeping {}", cfg.stack_limit)),
            },
            "bagtab" => match v.parse::<i16>() {
                Ok(id) if id >= 0 => cfg.bag_tab = Some(id),
                Ok(_) => cfg.bag_tab = None,
                Err(_) => warnings.push(format!("BagTab: bad value {v:?}, keeping auto")),
            },
            "gatherinterval" | "nodecooldown" => match v.parse::<u32>() {
                Ok(ms) if (100..=60_000).contains(&ms) => {
                    if k.eq_ignore_ascii_case("gatherinterval") {
                        cfg.gather_interval_ms = ms;
                    } else {
                        cfg.node_cooldown_ms = ms;
                    }
                }
                _ => warnings.push(format!("{k}: bad value {v:?} (100..60000 ms), keeping default")),
            },
            "gatherrange" => match v.parse::<f32>() {
                Ok(f) if f > 0.0 && f <= 50.0 => cfg.gather_range = f,
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
        let (c, w) = parse("; c\n[DesertLooter]\nEnabled=0\nDebug=1\nScanRange=25.5\nGatherRange=4\nAutoGather=1\nGatherInterval=250\nNodeCooldown=5\nKeyToggle=F5\nKeyScan=nope\nKeyGather=F8\nJunk=1\n");
        assert!(!c.enabled);
        assert!(c.debug);
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
}
