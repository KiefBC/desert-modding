//! The ini dialect every plugin reads: `Key=Value`, one per line, `;` or `#`
//! comments, `[Section]` headers ignored (each mod has one file, so sections
//! are decoration). Values are interpreted by the caller, which owns its own
//! key list; this module only tokenises and supplies the shared vocabulary.

/// One meaningful line of an ini file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line<'a> {
    /// `key`, `value`, both already trimmed.
    Pair(&'a str, &'a str),
    /// A line that is not a comment, a section, or a `key=value`. The string is
    /// a ready-to-log warning naming the line number.
    Bad(String),
}

/// Walk the file. Blank lines, comments and section headers are dropped;
/// everything else comes back as a [`Line`] in file order, so a caller can
/// interleave its own warnings with the syntax ones.
pub fn lines(text: &str) -> impl Iterator<Item = Line<'_>> {
    text.lines().enumerate().filter_map(|(lineno, raw)| {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
            return None;
        }
        Some(match line.split_once('=') {
            Some((k, v)) => Line::Pair(k.trim(), v.trim()),
            None => Line::Bad(format!("line {}: no '=': {raw:?}", lineno + 1)),
        })
    })
}

/// The truthy spellings accepted in a value position.
pub fn parse_bool(v: &str) -> bool {
    matches!(v, "1" | "true" | "yes" | "on")
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
    if let [c] = n.as_bytes() {
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(*c as u16);
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
    fn skips_noise_and_reports_bad_lines() {
        let got: Vec<_> = lines("; c\n\n[Section]\n Enabled = 1 \n# c\nJunk\nA=b=c\n").collect();
        assert_eq!(got[0], Line::Pair("Enabled", "1"));
        assert_eq!(got[2], Line::Pair("A", "b=c"), "only the first '=' splits");
        assert!(matches!(&got[1], Line::Bad(w) if w.starts_with("line 6: no '='")));
        assert_eq!(got.len(), 3);
    }

    #[test]
    fn bools() {
        assert!(parse_bool("1") && parse_bool("on") && parse_bool("yes") && parse_bool("true"));
        assert!(!parse_bool("0") && !parse_bool("True") && !parse_bool(""));
    }
}
