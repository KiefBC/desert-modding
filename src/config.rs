//! `DesertLooter.ini` beside the game exe. Missing file or key => defaults.

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

/// Key names accepted in the ini, matching the reference mod's vocabulary.
pub fn vk_from_name(name: &str) -> Option<u16> {
    let n = name.trim().to_ascii_uppercase();
    if let Some(f) = n.strip_prefix('F') {
        if let Ok(k) = f.parse::<u16>() {
            if (1..=24).contains(&k) {
                return Some(0x6F + k);
            }
        }
    }
    if n.len() == 1 {
        let c = n.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u16);
        }
    }
    if let Some(d) = n.strip_prefix("NUM") {
        if let Ok(k) = d.parse::<u16>() {
            if k <= 9 {
                return Some(0x60 + k);
            }
        }
    }
    Some(match n.as_str() {
        "NUMMULT" => 0x6A,
        "NUMPLUS" => 0x6B,
        "NUMMINUS" => 0x6D,
        "NUMDOT" => 0x6E,
        "NUMDIV" => 0x6F,
        "HOME" => 0x24,
        "END" => 0x23,
        "INSERT" => 0x2D,
        "DELETE" => 0x2E,
        "PAGEUP" => 0x21,
        "PAGEDOWN" => 0x22,
        "TAB" => 0x09,
        "SPACE" => 0x20,
        "BACKSPACE" => 0x08,
        "SCROLLLOCK" => 0x91,
        "PAUSE" => 0x13,
        "MOUSE3" => 0x04,
        "MOUSE4" => 0x05,
        "MOUSE5" => 0x06,
        _ => return None,
    })
}

/// Parse ini text. Unknown keys are reported back so they can be logged.
pub fn parse(text: &str) -> (Config, Vec<String>) {
    let mut cfg = Config::default();
    let mut warnings = Vec::new();
    for (lineno, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            warnings.push(format!("line {}: no '=': {raw:?}", lineno + 1));
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        let bool_of = |v: &str| matches!(v, "1" | "true" | "yes" | "on");
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
            "keytoggle" | "keyscan" | "keygather" | "keyrecord" => match vk_from_name(v) {
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
    fn key_names() {
        assert_eq!(vk_from_name("F1"), Some(0x70));
        assert_eq!(vk_from_name("f24"), Some(0x87));
        assert_eq!(vk_from_name("F25"), None);
        assert_eq!(vk_from_name("A"), Some(0x41));
        assert_eq!(vk_from_name("7"), Some(0x37));
        assert_eq!(vk_from_name("NUM0"), Some(0x60));
        assert_eq!(vk_from_name("NUMPLUS"), Some(0x6B));
        assert_eq!(vk_from_name("MOUSE5"), Some(0x06));
        assert_eq!(vk_from_name("bogus"), None);
    }

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
