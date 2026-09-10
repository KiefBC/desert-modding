//! In-place, comment-preserving ini rewriting **inside one `[Section]`**, and
//! the atomic swap that puts the result on disk.
//!
//! The overlay is not the owner of the file it edits: the user wrote it, the
//! mod ships it with a page of explanation, and a hand-added key the overlay
//! knows nothing about must survive being edited from the menu. So the file is
//! never regenerated from the model. [`rewrite`] walks the existing text line
//! by line and changes only the value on the lines whose key it was handed,
//! inserting the keys it never found. Comments, blank lines, `[Section]`
//! headers, key order, key spelling, the spacing around the `=` and the file's
//! line ending all come out the way they went in.
//!
//! **The section is the whole point.** `DesertTooling.ini` holds `[Looter]`,
//! `[Gatherer]` and `[Overlay]` in one file, `Enabled` exists in all three and
//! `Debug` in two, so a key match on its own would write the looter's checkbox
//! into the gatherer's settings. Every match here is therefore gated on the
//! header the line falls under, matched the way [`desert_core::ini::entries`]
//! reports it (bracket-stripped, trimmed, ASCII case-insensitive); pairs that
//! sit before any header at all belong to no section and are never touched. A
//! key the section is missing is inserted **under that section's header**, not
//! at the end of the file, and a section that is not in the file at all is
//! appended with its header first.
//!
//! [`write_atomically`] writes `<name>.ini.tmp` beside the target and renames
//! it over the original. `std::fs::rename` on Windows is `MoveFileEx` with
//! `MOVEFILE_REPLACE_EXISTING`, so the swap replaces the file rather than
//! failing, and a subsystem that stats the ini one second later either sees the
//! whole old file or the whole new one - never a half-written one.

use std::path::{Path, PathBuf};

/// One line of the file as it will be written back: its body (no terminator)
/// and the terminator it carried, which is `""` only for a last line that had
/// none.
struct Row {
    body: String,
    eol: &'static str,
}

/// Rewrite `text` so that every key in `updates` carries its given value
/// **within `section`**, leaving every other section, and everything before
/// the first header, exactly as it was.
///
/// Matching is case-insensitive on the key, as the subsystems' own parsers
/// are. A key that appears more than once inside the section has every
/// occurrence updated (the parsers take the last one, so leaving an earlier
/// line stale would make the file disagree with itself). Keys the section does
/// not have are inserted after its last `key=value` line - or, when the file
/// has no such header, appended as a fresh `[section]` block.
pub fn rewrite(text: &str, section: &str, updates: &[(&str, String)]) -> String {
    let want = section.trim();
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut seen = vec![false; updates.len()];
    let mut rows: Vec<Row> = Vec::new();
    let mut inside = false;
    let mut found = false;
    // Where a key the section is missing would go: just past the section's
    // header until it has a pair of its own, then just past its last pair, so
    // a comment or a blank line that introduces the *next* section is not
    // separated from it.
    let mut insert_at: Option<usize> = None;

    // `split_inclusive` keeps each line's own terminator, so a file with mixed
    // or missing endings is reproduced exactly except where a value changed.
    for raw in text.split_inclusive('\n') {
        let (body, eol) = split_eol(raw);
        if let Some(name) = header_name(body) {
            inside = name.eq_ignore_ascii_case(want);
            rows.push(Row { body: body.to_string(), eol });
            if inside {
                // A repeated header continues the same section, exactly as
                // `ini::lines_in_section` reads it, so this only moves the
                // insertion point forward.
                found = true;
                insert_at = Some(rows.len());
            }
            continue;
        }
        let out = if inside { rewrite_line(body, updates, &mut seen) } else { body.to_string() };
        let is_pair = inside && is_pair_line(body);
        rows.push(Row { body: out, eol });
        if is_pair {
            insert_at = Some(rows.len());
        }
    }

    let missing: Vec<_> =
        updates.iter().zip(&seen).filter(|(_, s)| !**s).map(|(kv, _)| kv).collect();
    if !missing.is_empty() {
        let mut new_rows: Vec<Row> = Vec::with_capacity(missing.len() + 1);
        if !found {
            // No such header anywhere: the block goes at the end of the file,
            // after a blank line if there is anything to separate it from.
            if !rows.is_empty() {
                new_rows.push(Row { body: String::new(), eol: newline });
            }
            new_rows.push(Row { body: format!("[{want}]"), eol: newline });
        }
        for (k, v) in missing {
            new_rows.push(Row { body: format!("{k}={v}"), eol: newline });
        }
        let at = if found { insert_at.unwrap_or(rows.len()) } else { rows.len() };
        let at = at.min(rows.len());
        // A file whose last line had no terminator gets one, or the first
        // inserted key would land on the end of it.
        if let Some(prev) = at.checked_sub(1).and_then(|i| rows.get_mut(i)) {
            if prev.eol.is_empty() {
                prev.eol = newline;
            }
        }
        let tail = rows.split_off(at);
        rows.extend(new_rows);
        rows.extend(tail);
    }

    let mut out = String::with_capacity(text.len() + 64);
    for row in &rows {
        out.push_str(&row.body);
        out.push_str(row.eol);
    }
    out
}

/// Split a raw line into its body and its terminator.
fn split_eol(raw: &str) -> (&str, &'static str) {
    match raw.strip_suffix('\n') {
        Some(b) => match b.strip_suffix('\r') {
            Some(b) => (b, "\r\n"),
            None => (b, "\n"),
        },
        None => (raw, ""),
    }
}

/// The section this line names, if it is a header.
///
/// Deliberately the same rule as [`desert_core::ini::entries`], which is what
/// the subsystems read the file back with: anything opening with `[` is a
/// header, a missing `]` and all, and the name is what sits between the
/// brackets once trimmed. The two must agree, or the overlay would write under
/// a header the reader does not believe in.
fn header_name(body: &str) -> Option<&str> {
    let rest = body.trim().strip_prefix('[')?;
    Some(rest.strip_suffix(']').unwrap_or(rest).trim())
}

/// Is this a `key=value` line, rather than a blank, a comment or a header?
/// Only these move the insertion point, so a trailing comment block keeps the
/// section it introduces.
fn is_pair_line(body: &str) -> bool {
    let trimmed = body.trim_start();
    !trimmed.is_empty()
        && !trimmed.starts_with(';')
        && !trimmed.starts_with('#')
        && !trimmed.starts_with('[')
        && body.contains('=')
}

/// One line, terminator already stripped and already known to be inside the
/// section. Returns it unchanged unless it is a `key=value` whose key is in
/// `updates`.
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

/// Where [`write_atomically`] stages the new content: `DesertTooling.ini` ->
/// `DesertTooling.ini.tmp`, in the same directory so the rename is a
/// same-volume move rather than a copy.
pub fn temp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Write `contents` to `path` through a temporary file and a rename.
///
/// The subsystems poll the ini's modified time roughly once a second and
/// re-read it when it moves; a truncate-then-write would give them a window in
/// which the file is empty and every key falls back to its default. The rename
/// makes the change one indivisible step. A failure leaves the original
/// untouched and the temporary file removed.
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
    use desert_core::ini::{self, Line};

    fn up(pairs: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        pairs.iter().map(|(k, v)| (*k, (*v).to_string())).collect()
    }

    /// The shape of the real file: three subsystems, `Enabled` in all three
    /// and `Debug` in two.
    const SHARED: &str = "\
; Desert Tooling
[Looter]
; why Enabled matters
Enabled=1
Debug=0
GatherOre=1

[Gatherer]
Enabled=1
Foraging=2

[Overlay]
Enabled=1
Debug=0
Theme=banner
";

    /// What `ini::lines_in_section` - the reader every subsystem uses - makes
    /// of one section of `text`.
    fn read(text: &str, section: &str) -> Vec<(String, String)> {
        ini::lines_in_section(text, section)
            .filter_map(|l| match l {
                Line::Pair(k, v) => Some((k.to_string(), v.to_string())),
                Line::Bad(_) => None,
            })
            .collect()
    }

    fn value(text: &str, section: &str, key: &str) -> Option<String> {
        read(text, section).into_iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    // -- the hazard: one file, three sections ------------------------------

    #[test]
    fn a_write_lands_in_its_own_section_and_nowhere_else() {
        let after = rewrite(SHARED, "Gatherer", &up(&[("Enabled", "0"), ("Foraging", "5")]));
        assert_eq!(value(&after, "Gatherer", "Enabled").as_deref(), Some("0"));
        assert_eq!(value(&after, "Gatherer", "Foraging").as_deref(), Some("5"));
        assert_eq!(value(&after, "Looter", "Enabled").as_deref(), Some("1"), "the looter is untouched");
        assert_eq!(value(&after, "Overlay", "Enabled").as_deref(), Some("1"), "so is the overlay");
        // And nothing else moved at all.
        assert_eq!(read(&after, "Looter"), read(SHARED, "Looter"));
        assert_eq!(read(&after, "Overlay"), read(SHARED, "Overlay"));
    }

    #[test]
    fn every_section_can_be_written_in_turn_without_disturbing_the_others() {
        let mut text = SHARED.to_string();
        for (section, key) in [("Looter", "Debug"), ("Gatherer", "Enabled"), ("Overlay", "Debug")] {
            text = rewrite(&text, section, &up(&[(key, "1")]));
        }
        assert_eq!(value(&text, "Looter", "Debug").as_deref(), Some("1"));
        assert_eq!(value(&text, "Looter", "Enabled").as_deref(), Some("1"));
        assert_eq!(value(&text, "Gatherer", "Enabled").as_deref(), Some("1"));
        assert_eq!(value(&text, "Overlay", "Debug").as_deref(), Some("1"));
        assert_eq!(value(&text, "Overlay", "Enabled").as_deref(), Some("1"));
        assert_eq!(value(&text, "Gatherer", "Foraging").as_deref(), Some("2"));
        assert!(text.contains("; why Enabled matters"), "the comments are still there");
    }

    #[test]
    fn a_key_missing_from_its_section_is_inserted_under_that_header() {
        let after = rewrite(SHARED, "Gatherer", &up(&[("Fish", "3")]));
        assert_eq!(value(&after, "Gatherer", "Fish").as_deref(), Some("3"));
        assert_eq!(value(&after, "Looter", "Fish"), None, "it did not land in the looter");
        assert_eq!(value(&after, "Overlay", "Fish"), None, "nor at the end of the file");
        assert_eq!(
            after,
            "; Desert Tooling\n[Looter]\n; why Enabled matters\nEnabled=1\nDebug=0\nGatherOre=1\n\n\
             [Gatherer]\nEnabled=1\nForaging=2\nFish=3\n\n\
             [Overlay]\nEnabled=1\nDebug=0\nTheme=banner\n"
        );
    }

    #[test]
    fn a_key_missing_from_the_last_section_stays_inside_it() {
        let after = rewrite(SHARED, "Overlay", &up(&[("Scale", "1.5")]));
        assert_eq!(value(&after, "Overlay", "Scale").as_deref(), Some("1.5"));
        assert!(after.ends_with("Theme=banner\nScale=1.5\n"), "{after}");
    }

    #[test]
    fn a_section_the_file_does_not_have_is_appended_with_its_header() {
        let after = rewrite(SHARED, "Gatherer2", &up(&[("Enabled", "0")]));
        assert_eq!(value(&after, "Gatherer2", "Enabled").as_deref(), Some("0"));
        assert_eq!(value(&after, "Gatherer", "Enabled").as_deref(), Some("1"));
        assert!(after.ends_with("\n[Gatherer2]\nEnabled=0\n"), "{after}");
    }

    #[test]
    fn a_key_before_any_header_is_not_mistaken_for_ours() {
        // A stray pair at the top of the file belongs to no section, which is
        // exactly what `ini::lines_in_section` says of it.
        let before = "Enabled=1\nDebug=1\n\n[Overlay]\nEnabled=1\n";
        let after = rewrite(before, "Overlay", &up(&[("Enabled", "0"), ("Debug", "1")]));
        assert!(after.starts_with("Enabled=1\nDebug=1\n"), "the orphan lines are untouched: {after}");
        assert_eq!(value(&after, "Overlay", "Enabled").as_deref(), Some("0"));
        assert_eq!(value(&after, "Overlay", "Debug").as_deref(), Some("1"), "inserted, not stolen");
        assert!(after.ends_with("[Overlay]\nEnabled=0\nDebug=1\n"), "{after}");
    }

    #[test]
    fn the_sections_may_come_in_any_order() {
        let before = "[Overlay]\nEnabled=1\n\n[Gatherer]\nEnabled=1\n\n[Looter]\nEnabled=1\n";
        let after = rewrite(before, "Looter", &up(&[("Enabled", "0")]));
        assert_eq!(after, "[Overlay]\nEnabled=1\n\n[Gatherer]\nEnabled=1\n\n[Looter]\nEnabled=0\n");
    }

    #[test]
    fn a_header_that_repeats_continues_the_same_section() {
        // The reader treats a second `[Looter]` as more of the same section,
        // so the writer has to as well - and the missing key goes after the
        // last pair of the last block.
        let before = "[Looter]\nEnabled=1\n\n[Gatherer]\nEnabled=1\n\n[Looter]\nDebug=1\n";
        let after = rewrite(before, "Looter", &up(&[("Enabled", "0"), ("Debug", "0"), ("New", "7")]));
        assert_eq!(
            after,
            "[Looter]\nEnabled=0\n\n[Gatherer]\nEnabled=1\n\n[Looter]\nDebug=0\nNew=7\n"
        );
        assert_eq!(value(&after, "Gatherer", "Enabled").as_deref(), Some("1"));
    }

    #[test]
    fn the_header_match_is_case_insensitive_and_ignores_spacing() {
        let before = "[ gatherer ]\nEnabled=1\n";
        let after = rewrite(before, "Gatherer", &up(&[("Enabled", "0")]));
        assert_eq!(after, "[ gatherer ]\nEnabled=0\n");
    }

    #[test]
    fn a_comment_that_introduces_the_next_section_keeps_its_place() {
        let before = "[Looter]\nEnabled=1\n\n; the gatherer's own settings\n[Gatherer]\nEnabled=1\n";
        let after = rewrite(before, "Looter", &up(&[("Enabled", "1"), ("Debug", "0")]));
        assert_eq!(
            after,
            "[Looter]\nEnabled=1\nDebug=0\n\n; the gatherer's own settings\n[Gatherer]\nEnabled=1\n"
        );
    }

    // -- everything the single-file rewriter already guaranteed -------------

    #[test]
    fn replaces_a_value_and_keeps_everything_else() {
        let before = "; a comment\n[Looter]\n\n; why Enabled matters\nEnabled=1\nDebug=0\n";
        let after = rewrite(before, "Looter", &up(&[("Enabled", "0")]));
        assert_eq!(after, "; a comment\n[Looter]\n\n; why Enabled matters\nEnabled=0\nDebug=0\n");
    }

    #[test]
    fn key_match_is_case_insensitive_and_the_spelling_is_kept() {
        let after = rewrite(
            "[Looter]\nenabled=1\nGATHERITEMS=0\n",
            "Looter",
            &up(&[("Enabled", "0"), ("GatherItems", "1")]),
        );
        assert_eq!(
            after,
            "[Looter]\nenabled=0\nGATHERITEMS=1\n",
            "the user's capitalisation survives"
        );
    }

    #[test]
    fn spacing_around_the_equals_is_preserved() {
        let after = rewrite(
            "[Looter]\n  Enabled = 1  \nScanRange=40\n",
            "Looter",
            &up(&[("Enabled", "0"), ("ScanRange", "55")]),
        );
        assert_eq!(after, "[Looter]\n  Enabled = 0\nScanRange=55\n");
    }

    #[test]
    fn appends_after_a_file_with_no_trailing_newline() {
        let after = rewrite("[Looter]\nEnabled=1", "Looter", &up(&[("GatherOre", "0")]));
        assert_eq!(after, "[Looter]\nEnabled=1\nGatherOre=0\n");
    }

    #[test]
    fn crlf_is_preserved_including_on_inserted_keys() {
        let before = "; c\r\n[Looter]\r\nEnabled=1\r\n";
        let after = rewrite(before, "Looter", &up(&[("Enabled", "0"), ("GatherOre", "1")]));
        assert_eq!(after, "; c\r\n[Looter]\r\nEnabled=0\r\nGatherOre=1\r\n");
    }

    #[test]
    fn comments_that_look_like_pairs_are_left_alone() {
        let before = "[Looter]\n; Enabled=1 is the default\n# Enabled=9\nEnabled=1\n";
        let after = rewrite(before, "Looter", &up(&[("Enabled", "0")]));
        assert_eq!(after, "[Looter]\n; Enabled=1 is the default\n# Enabled=9\nEnabled=0\n");
    }

    #[test]
    fn every_occurrence_of_a_duplicated_key_is_updated() {
        // The subsystems' parsers take the last line, so a stale earlier line
        // would make the file disagree with what the menu shows.
        let after = rewrite(
            "[Looter]\nEnabled=1\nDebug=0\nEnabled=1\n",
            "Looter",
            &up(&[("Enabled", "0")]),
        );
        assert_eq!(after, "[Looter]\nEnabled=0\nDebug=0\nEnabled=0\n");
    }

    #[test]
    fn a_key_the_overlay_does_not_own_is_untouched() {
        let before = "[Looter]\nBagTab=1\nKeyToggle=F10\nEnabled=1\n";
        let after = rewrite(before, "Looter", &up(&[("Enabled", "0")]));
        assert_eq!(after, "[Looter]\nBagTab=1\nKeyToggle=F10\nEnabled=0\n");
    }

    #[test]
    fn an_empty_file_becomes_a_header_and_the_keys() {
        assert_eq!(rewrite("", "Overlay", &up(&[("Enabled", "1")])), "[Overlay]\nEnabled=1\n");
    }

    #[test]
    fn rewriting_is_idempotent() {
        let updates = up(&[("Enabled", "0"), ("Ore", "5")]);
        let once = rewrite("[Looter]\n; c\nEnabled=1\n", "Looter", &updates);
        let twice = rewrite(&once, "Looter", &updates);
        assert_eq!(once, twice);
    }

    #[test]
    fn temp_path_sits_beside_the_target() {
        let p = Path::new("/game/bin64/DesertTooling.ini");
        assert_eq!(temp_path(p), Path::new("/game/bin64/DesertTooling.ini.tmp"));
    }

    #[test]
    fn write_atomically_replaces_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("desert-overlay-rewrite-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("DesertTooling.ini");
        std::fs::write(&path, "Enabled=1\n").unwrap();
        write_atomically(&path, "Enabled=0\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "Enabled=0\n");
        assert!(!temp_path(&path).exists(), "the staging file is gone after the rename");
        std::fs::remove_dir_all(&dir).ok();
    }
}
