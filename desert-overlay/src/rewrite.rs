//! In-place, comment-preserving ini rewriting, and the atomic swap that puts
//! the result on disk.
//!
//! The overlay is not the owner of the files it edits: the user wrote them,
//! the mods ship them with a page of explanation each, and a hand-added key
//! the overlay knows nothing about must survive being edited from the menu.
//! So the file is never regenerated from the model. [`rewrite`] walks the
//! existing text line by line and changes only the value on the lines whose
//! key it was handed, appending the keys it never found. Comments, blank
//! lines, `[Section]` headers, key order, key spelling, the spacing around the
//! `=` and the file's line ending all come out the way they went in.
//!
//! [`write_atomically`] writes `<name>.ini.tmp` beside the target and renames
//! it over the original. `std::fs::rename` on Windows is `MoveFileEx` with
//! `MOVEFILE_REPLACE_EXISTING`, so the swap replaces the file rather than
//! failing, and a plugin that stats the ini one second later either sees the
//! whole old file or the whole new one - never a half-written one.

use std::path::{Path, PathBuf};

/// Rewrite `text` so that every key in `updates` carries its given value.
///
/// Matching is case-insensitive on the key, as the plugins' own parsers are.
/// A key that appears more than once has every occurrence updated (the parsers
/// take the last one, so leaving an earlier line stale would make the file
/// disagree with itself). Keys not present anywhere are appended at the end.
pub fn rewrite(text: &str, updates: &[(&str, String)]) -> String {
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut seen = vec![false; updates.len()];
    let mut out = String::with_capacity(text.len() + 64);

    // `split_inclusive` keeps each line's own terminator, so a file with mixed
    // or missing endings is reproduced exactly except where a value changed.
    for raw in text.split_inclusive('\n') {
        let (body, eol) = match raw.strip_suffix('\n') {
            Some(b) => match b.strip_suffix('\r') {
                Some(b) => (b, "\r\n"),
                None => (b, "\n"),
            },
            None => (raw, ""),
        };
        out.push_str(&rewrite_line(body, updates, &mut seen));
        out.push_str(eol);
    }

    let missing: Vec<_> =
        updates.iter().zip(&seen).filter(|(_, s)| !**s).map(|(kv, _)| kv).collect();
    if !missing.is_empty() {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str(newline);
        }
        for (k, v) in missing {
            out.push_str(k);
            out.push('=');
            out.push_str(v);
            out.push_str(newline);
        }
    }
    out
}

/// One line, terminator already stripped. Returns it unchanged unless it is a
/// `key=value` whose key is in `updates`.
fn rewrite_line(body: &str, updates: &[(&str, String)], seen: &mut [bool]) -> String {
    let trimmed = body.trim_start();
    if trimmed.is_empty()
        || trimmed.starts_with(';')
        || trimmed.starts_with('#')
        || trimmed.starts_with('[')
    {
        return body.to_string();
    }
    let Some(eq) = body.find('=') else { return body.to_string() };
    let Some(key) = body.get(..eq) else { return body.to_string() };
    let Some(after) = body.get(eq + 1..) else { return body.to_string() };

    let Some(idx) = updates.iter().position(|(k, _)| k.eq_ignore_ascii_case(key.trim())) else {
        return body.to_string();
    };
    if let Some(flag) = seen.get_mut(idx) {
        *flag = true;
    }
    let Some((_, value)) = updates.get(idx) else { return body.to_string() };

    // Keep the key exactly as the user spelled it, keep whatever spacing sat
    // between the `=` and the old value, replace only the value itself.
    let gap_len = after.len() - after.trim_start().len();
    let gap = after.get(..gap_len).unwrap_or("");
    format!("{key}={gap}{value}")
}

/// Where [`write_atomically`] stages the new content: `DesertLooter.ini` ->
/// `DesertLooter.ini.tmp`, in the same directory so the rename is a same-volume
/// move rather than a copy.
pub fn temp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Write `contents` to `path` through a temporary file and a rename.
///
/// The plugins poll the ini's modified time roughly once a second and re-read
/// it when it moves; a truncate-then-write would give them a window in which
/// the file is empty and every key falls back to its default. The rename makes
/// the change one indivisible step. A failure leaves the original untouched
/// and the temporary file removed.
pub fn write_atomically(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = temp_path(path);
    std::fs::write(&tmp, contents)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(k, v)| (*k, (*v).to_string())).collect()
    }

    #[test]
    fn replaces_a_value_and_keeps_everything_else() {
        let before = "; a comment\n[DesertLooter]\n\n; why Enabled matters\nEnabled=1\nDebug=0\n";
        let after = rewrite(before, &up(&[("Enabled", "0")]));
        assert_eq!(
            after,
            "; a comment\n[DesertLooter]\n\n; why Enabled matters\nEnabled=0\nDebug=0\n"
        );
    }

    #[test]
    fn key_match_is_case_insensitive_and_the_spelling_is_kept() {
        let after = rewrite("enabled=1\nGATHERITEMS=0\n", &up(&[("Enabled", "0"), ("GatherItems", "1")]));
        assert_eq!(after, "enabled=0\nGATHERITEMS=1\n", "the user's capitalisation survives");
    }

    #[test]
    fn spacing_around_the_equals_is_preserved() {
        let after = rewrite("  Enabled = 1  \nScanRange=40\n", &up(&[("Enabled", "0"), ("ScanRange", "55")]));
        assert_eq!(after, "  Enabled = 0\nScanRange=55\n");
    }

    #[test]
    fn missing_keys_are_appended_at_the_end() {
        let after = rewrite("[DesertLooter]\nEnabled=1\n", &up(&[("Enabled", "1"), ("GatherOre", "0")]));
        assert_eq!(after, "[DesertLooter]\nEnabled=1\nGatherOre=0\n");
    }

    #[test]
    fn appends_after_a_file_with_no_trailing_newline() {
        let after = rewrite("Enabled=1", &up(&[("GatherOre", "0")]));
        assert_eq!(after, "Enabled=1\nGatherOre=0\n");
    }

    #[test]
    fn crlf_is_preserved_including_on_appended_keys() {
        let before = "; c\r\n[DesertLooter]\r\nEnabled=1\r\n";
        let after = rewrite(before, &up(&[("Enabled", "0"), ("GatherOre", "1")]));
        assert_eq!(after, "; c\r\n[DesertLooter]\r\nEnabled=0\r\nGatherOre=1\r\n");
    }

    #[test]
    fn comments_and_sections_that_look_like_pairs_are_left_alone() {
        let before = "; Enabled=1 is the default\n# Enabled=9\n[Enabled=7]\nEnabled=1\n";
        let after = rewrite(before, &up(&[("Enabled", "0")]));
        assert_eq!(before.replace("Enabled=1\n", "Enabled=0\n"), after);
        assert!(after.contains("; Enabled=1 is the default"), "the comment kept its example");
    }

    #[test]
    fn every_occurrence_of_a_duplicated_key_is_updated() {
        // The plugins' parsers take the last line, so a stale earlier line
        // would make the file disagree with what the menu shows.
        let after = rewrite("Enabled=1\nDebug=0\nEnabled=1\n", &up(&[("Enabled", "0")]));
        assert_eq!(after, "Enabled=0\nDebug=0\nEnabled=0\n");
    }

    #[test]
    fn a_key_the_overlay_does_not_own_is_untouched() {
        let before = "BagTab=1\nKeyToggle=F10\nEnabled=1\n";
        let after = rewrite(before, &up(&[("Enabled", "0")]));
        assert_eq!(after, "BagTab=1\nKeyToggle=F10\nEnabled=0\n");
    }

    #[test]
    fn an_empty_file_becomes_just_the_keys() {
        assert_eq!(rewrite("", &up(&[("Enabled", "1")])), "Enabled=1\n");
    }

    #[test]
    fn rewriting_is_idempotent() {
        let once = rewrite("[X]\n; c\nEnabled=1\n", &up(&[("Enabled", "0"), ("Ore", "5")]));
        let twice = rewrite(&once, &up(&[("Enabled", "0"), ("Ore", "5")]));
        assert_eq!(once, twice);
    }

    #[test]
    fn temp_path_sits_beside_the_target() {
        let p = Path::new("/game/bin64/DesertLooter.ini");
        assert_eq!(temp_path(p), Path::new("/game/bin64/DesertLooter.ini.tmp"));
    }

    #[test]
    fn write_atomically_replaces_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("desert-overlay-rewrite-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("DesertLooter.ini");
        std::fs::write(&path, "Enabled=1\n").unwrap();
        write_atomically(&path, "Enabled=0\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Enabled=0\n");
        assert!(!temp_path(&path).exists(), "the staging file is gone after the rename");
        std::fs::remove_dir_all(&dir).ok();
    }
}
