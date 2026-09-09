//! The ini dialect every plugin reads: `Key=Value`, one per line, `;` or `#`
//! comments, `[Section]` headers. Three readers over the same tokeniser:
//! [`entries`] yields headers and pairs alike, [`lines`] drops the headers and
//! reads the whole file flat, and [`lines_in_section`] reads one `[Section]` of
//! it. Every subsystem now shares one ini, and `Enabled`/`Debug`/`DryRun`
//! collide across them, so a subsystem reads its own settings through
//! [`lines_in_section`]. Values are interpreted by the caller, which owns its
//! own key list; this module only tokenises and supplies the shared vocabulary.

/// One meaningful line of an ini file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line<'a> {
    /// `key`, `value`, both already trimmed.
    Pair(&'a str, &'a str),
    /// A line that is not a comment, a section, or a `key=value`. The string is
    /// a ready-to-log warning naming the line number.
    Bad(String),
}

/// One meaningful line of an ini file, sections included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry<'a> {
    /// A `[Section]` header, brackets stripped and trimmed. A line that opens
    /// with `[` but never closes it is still a header: that keeps [`lines`],
    /// which drops headers, from suddenly reporting one as a bad line.
    Section(&'a str),
    /// `key`, `value`, both already trimmed.
    Pair(&'a str, &'a str),
    /// A line that is not a comment, a section, or a `key=value`. The string is
    /// a ready-to-log warning naming the line number.
    Bad(String),
}

/// Walk the file. Blank lines and comments are dropped; everything else comes
/// back as an [`Entry`] in file order, so a caller can interleave its own
/// warnings with the syntax ones.
pub fn entries(text: &str) -> impl Iterator<Item = Entry<'_>> {
    text.lines().enumerate().filter_map(|(lineno, raw)| {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            return None;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.strip_suffix(']').unwrap_or(rest);
            return Some(Entry::Section(name.trim()));
        }
        Some(match line.split_once('=') {
            Some((k, v)) => Entry::Pair(k.trim(), v.trim()),
            None => Entry::Bad(format!("line {}: no '=': {raw:?}", lineno + 1)),
        })
    })
}

/// Walk the file. Blank lines, comments and section headers are dropped;
/// everything else comes back as a [`Line`] in file order, so a caller can
/// interleave its own warnings with the syntax ones.
pub fn lines(text: &str) -> impl Iterator<Item = Line<'_>> {
    entries(text).filter_map(|e| match e {
        Entry::Section(_) => None,
        Entry::Pair(k, v) => Some(Line::Pair(k, v)),
        Entry::Bad(w) => Some(Line::Bad(w)),
    })
}

/// Walk one `[section]` of the file: the [`Line`]s that follow that header,
/// stopping at the next one. The header is matched ASCII case-insensitively
/// and after trimming, as [`entries`] reports it.
///
/// Pairs before any header belong to no section and are skipped, and so is a
/// bad line outside the section: a subsystem reading `[Gatherer]` should not
/// warn about junk that belongs to somebody else. A header that repeats
/// **continues** the same section rather than starting a second one, so a
/// player who pastes a `[Looter]` block at the end of the file still gets the
/// keys they wrote.
pub fn lines_in_section<'a>(text: &'a str, section: &str) -> impl Iterator<Item = Line<'a>> + 'a {
    // Owned, so the returned iterator borrows only `text`: the caller's section
    // name may be a temporary.
    let want = section.trim().to_string();
    let mut inside = false;
    entries(text).filter_map(move |e| match e {
        Entry::Section(name) => {
            inside = name.eq_ignore_ascii_case(&want);
            None
        }
        Entry::Pair(k, v) => inside.then_some(Line::Pair(k, v)),
        Entry::Bad(why) => inside.then_some(Line::Bad(why)),
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

/// Every name [`vk_from_name`] accepts, in picker order: `F1`..`F24`, `A`..`Z`,
/// `0`..`9`, `NUM0`..`NUM9`, then the named keys in the order `vk_from_name`
/// lists them. The `key_names_all_resolve` test keeps this list and that
/// function from drifting apart.
const KEY_NAMES: &[&str] = &[
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15",
    "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24", //
    "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S",
    "T", "U", "V", "W", "X", "Y", "Z", //
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", //
    "NUM0", "NUM1", "NUM2", "NUM3", "NUM4", "NUM5", "NUM6", "NUM7", "NUM8", "NUM9", //
    "NUMMULT", "NUMPLUS", "NUMMINUS", "NUMDOT", "NUMDIV", "HOME", "END", "INSERT", "DELETE",
    "PAGEUP", "PAGEDOWN", "TAB", "SPACE", "BACKSPACE", "SCROLLLOCK", "PAUSE", "MOUSE3", "MOUSE4",
    "MOUSE5",
];

/// Every name [`vk_from_name`] accepts, for a picker.
pub fn key_names() -> &'static [&'static str] {
    KEY_NAMES
}

/// The canonical spelling (upper-case, as [`key_names`] lists it) of a key name
/// [`vk_from_name`] accepts. `None` for anything it rejects.
pub fn canonical_key_name(name: &str) -> Option<String> {
    let n = name.trim().to_ascii_uppercase();
    let vk = vk_from_name(&n)?;
    // An accepted but unusual spelling (`F09`) canonicalises to the listed name
    // with the same virtual-key code; every code in the list is unique.
    let listed = KEY_NAMES.iter().find(|s| s.eq_ignore_ascii_case(&n));
    let listed = listed.or_else(|| KEY_NAMES.iter().find(|s| vk_from_name(s) == Some(vk)));
    Some(listed.map_or(n, |s| (*s).to_string()))
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

    /// The picker list is exactly what the parser accepts: every name resolves,
    /// no name repeats, no virtual-key code repeats, and the count matches the
    /// generated families plus one entry per named arm of `vk_from_name` -
    /// counted out of this file's own source, so adding an arm without adding
    /// the name fails here.
    #[test]
    fn key_names_all_resolve() {
        let mut codes = std::collections::HashSet::new();
        for name in super::key_names() {
            let vk = vk_from_name(name).unwrap_or_else(|| panic!("{name} does not resolve"));
            assert!(codes.insert(vk), "{name} repeats virtual-key 0x{vk:02X}");
            assert_eq!(canonical_key_name(&name.to_ascii_lowercase()).as_deref(), Some(*name));
        }
        let arms = include_str!("ini.rs")
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with('"') && l.contains("=> 0x"))
            .count();
        assert_eq!(super::key_names().len(), 24 + 26 + 10 + 10 + arms);
    }

    #[test]
    fn canonical_names() {
        assert_eq!(canonical_key_name(" f10 ").as_deref(), Some("F10"));
        assert_eq!(canonical_key_name("F09").as_deref(), Some("F9"), "an odd spelling is fixed");
        assert_eq!(canonical_key_name("mouse4").as_deref(), Some("MOUSE4"));
        assert_eq!(canonical_key_name("nope"), None);
        assert_eq!(canonical_key_name(""), None);
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
    fn entries_report_sections() {
        let got: Vec<_> = entries("; c\n[ overlay ]\nA=1\n[preset:Two]\nJunk\n[open\n").collect();
        assert_eq!(got[0], Entry::Section("overlay"));
        assert_eq!(got[1], Entry::Pair("A", "1"));
        assert_eq!(got[2], Entry::Section("preset:Two"));
        assert!(matches!(&got[3], Entry::Bad(w) if w.starts_with("line 5: no '='")));
        assert_eq!(got[4], Entry::Section("open"), "an unclosed header is still a header");
        assert_eq!(got.len(), 5);
    }

    /// `lines` is `entries` with the headers dropped, and nothing else.
    #[test]
    fn lines_agree_with_entries() {
        let text = "[a]\nk=v\n[b\nbad\n; c\n=v\n";
        let from_entries: Vec<_> = entries(text)
            .filter_map(|e| match e {
                Entry::Section(_) => None,
                Entry::Pair(k, v) => Some(Line::Pair(k, v)),
                Entry::Bad(w) => Some(Line::Bad(w)),
            })
            .collect();
        assert_eq!(lines(text).collect::<Vec<_>>(), from_entries);
    }

    #[test]
    fn reads_only_the_named_section() {
        let text = "\
Stray=1
[Looter]
Enabled=1
[Gatherer]
Enabled=0
Multiplier=2
[Overlay]
Enabled=1
";
        let got: Vec<_> = lines_in_section(text, "Gatherer").collect();
        assert_eq!(got, vec![Line::Pair("Enabled", "0"), Line::Pair("Multiplier", "2")]);
        assert_eq!(
            lines_in_section(text, "Looter").collect::<Vec<_>>(),
            vec![Line::Pair("Enabled", "1")],
            "a key that appears in two sections reads its own section's value"
        );
        assert!(
            lines_in_section(text, "Nope").next().is_none(),
            "a section the file does not have is empty, not an error"
        );
        assert!(
            lines_in_section(text, "").next().is_none(),
            "a pair before any header belongs to no section"
        );
    }

    #[test]
    fn section_headers_match_case_insensitively() {
        let text = "[ looter ]\nEnabled=1\n";
        for name in ["Looter", "looter", "LOOTER", " Looter "] {
            assert_eq!(
                lines_in_section(text, name).collect::<Vec<_>>(),
                vec![Line::Pair("Enabled", "1")],
                "{name:?}"
            );
        }
    }

    /// A player who pastes a second `[Looter]` block at the end of the file
    /// gets both halves; the later header continues the same section.
    #[test]
    fn a_repeated_header_continues_the_section() {
        let text = "[Looter]\nA=1\n[Gatherer]\nB=2\n[looter]\nC=3\n";
        assert_eq!(
            lines_in_section(text, "Looter").collect::<Vec<_>>(),
            vec![Line::Pair("A", "1"), Line::Pair("C", "3")]
        );
    }

    /// Bad lines are reported inside the section and ignored outside it: a
    /// subsystem should not warn about junk that belongs to somebody else.
    #[test]
    fn bad_lines_are_reported_only_inside_the_section() {
        let text = "junk before\n[Looter]\nmine\n[Gatherer]\ntheirs\n";
        let got: Vec<_> = lines_in_section(text, "Looter").collect();
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(matches!(&got[0], Line::Bad(w) if w.starts_with("line 3: no '='")), "{got:?}");
    }

    #[test]
    fn bools() {
        assert!(parse_bool("1") && parse_bool("on") && parse_bool("yes") && parse_bool("true"));
        assert!(!parse_bool("0") && !parse_bool("True") && !parse_bool(""));
    }
}
