//! The one-time rewrite of a pre-0.10 `DesertTooling.ini`: its `[Looter]` and
//! `[Gatherer]` sections become one `[Harvest]` section, where `[Looter]` was.
//!
//! Before 0.10 the two halves of this subsystem were two subsystems with a
//! section each, and both sections had an `Enabled` and a `Debug`. The merged
//! section has one of each, so the two old values are folded into one:
//!
//! - `Enabled` is 1 only if it was 1 in **both** old sections (a missing key
//!   counts as its default, 1). Nothing starts doing something the player had
//!   switched off; when the two disagreed, a comment above the migrated line
//!   says what each was and what to set, and the log says it once.
//! - `Debug` is 1 if it was 1 in **either**: it only adds log lines.
//!
//! Everything else travels with its line: the player's values, comments and
//! hand-added keys, the spelling and spacing of every key, and the file's line
//! ending. Lines outside the two old sections are not touched at all. The
//! comment run directly above each old header (the `; ====` banner of the
//! shipped template) is dropped, and the first one comes back above
//! `[Harvest]` with the old section's name replaced.
//!
//! [`migrate`] is pure and always compiled, so it is tested natively;
//! [`migrate_file`] is the read, the call and the atomic write
//! (`desert_core::ini::write_atomically`, the same swap the overlay uses).
//! `desert-tooling` runs it once per launch, right after seeding the ini and
//! before any subsystem thread exists, so nothing reads the file mid-rewrite.

use std::io;
use std::path::Path;

use desert_core::ini::{self, Line};

use crate::config::INI_SECTION;

/// The two section names 0.9 and earlier used.
const OLD_LOOTER: &str = "Looter";
const OLD_GATHERER: &str = "Gatherer";

/// What [`migrate`] made of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Migration {
    /// Nothing to do: no old section, or `[Harvest]` alone.
    Unchanged,
    /// `[Harvest]` is there **and** so is at least one old section, named here
    /// as the file spells them. Not rewritten: the player has already got a
    /// `[Harvest]` they may have edited, and only it is read.
    StaleOldSections(Vec<String>),
    /// The old sections were merged; this is the new file text.
    Rewritten(String),
}

/// One line of the file: its body (no terminator) and the terminator it
/// carried, which is `""` only for a last line that had none.
#[derive(Debug, Clone)]
struct Row {
    body: String,
    eol: &'static str,
}

/// One `[Header]` and what belongs to it.
struct Block {
    /// The comment run directly above the header, back to a blank line or a
    /// non-comment line: a banner, or a note about the section.
    above: Vec<Row>,
    header: Row,
    /// The header's trimmed name, the way `ini::entries` reports it.
    name: String,
    /// Everything after the header up to the next block's `above`.
    body: Vec<Row>,
}

/// Split a raw line into its body and its terminator. A copy of the overlay's
/// `rewrite::split_eol`, which this has to agree with.
fn split_eol(raw: &str) -> (&str, &'static str) {
    match raw.strip_suffix('\n') {
        Some(b) => match b.strip_suffix('\r') {
            Some(b) => (b, "\r\n"),
            None => (b, "\n"),
        },
        None => (raw, ""),
    }
}

/// The section this line names, if it is a header. The same rule as
/// `desert_core::ini::entries`: anything opening with `[` is a header, a
/// missing `]` and all, and the name is what sits between the brackets once
/// trimmed.
fn header_name(body: &str) -> Option<&str> {
    let rest = body.trim().strip_prefix('[')?;
    Some(rest.strip_suffix(']').unwrap_or(rest).trim())
}

fn is_comment(body: &str) -> bool {
    let t = body.trim_start();
    t.starts_with(';') || t.starts_with('#')
}

fn is_blank(body: &str) -> bool {
    body.trim().is_empty()
}

fn is_old(name: &str) -> bool {
    name.eq_ignore_ascii_case(OLD_LOOTER) || name.eq_ignore_ascii_case(OLD_GATHERER)
}

/// The key of a `key=value` line, trimmed, or `None` for anything else.
fn pair_key(body: &str) -> Option<&str> {
    if is_blank(body) || is_comment(body) || header_name(body).is_some() {
        return None;
    }
    body.split_once('=').map(|(k, _)| k.trim())
}

/// `body` with its value replaced, keeping the key's spelling and the spacing
/// on both sides of the `=`.
fn with_value(body: &str, value: &str) -> String {
    match body.split_once('=') {
        Some((key, after)) => {
            let gap_len = after.len() - after.trim_start().len();
            let gap = after.get(..gap_len).unwrap_or("");
            format!("{key}={gap}{value}")
        }
        None => body.to_string(),
    }
}

/// The last value of `key` in `section`, read the way the old parsers read it
/// (`ini::lines_in_section`, a repeated header continuing the same section).
fn last_bool(text: &str, section: &str, key: &str) -> Option<bool> {
    ini::lines_in_section(text, section)
        .filter_map(|l| match l {
            Line::Pair(k, v) if k.eq_ignore_ascii_case(key) => Some(ini::parse_bool(v)),
            _ => None,
        })
        .last()
}

/// Cut the file into the rows before the first header and one [`Block`] per
/// header.
fn blocks(text: &str) -> (Vec<Row>, Vec<Block>) {
    let mut preamble: Vec<Row> = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    for raw in text.split_inclusive('\n') {
        let (body, eol) = split_eol(raw);
        let row = Row { body: body.to_string(), eol };
        let Some(name) = header_name(body) else {
            match blocks.last_mut() {
                Some(b) => b.body.push(row),
                None => preamble.push(row),
            }
            continue;
        };
        // The comment run directly above this header moves from wherever it
        // was (the previous block's tail, or the preamble's) to this block.
        let tail = match blocks.last_mut() {
            Some(b) => &mut b.body,
            None => &mut preamble,
        };
        let keep = tail.iter().rposition(|r| !is_comment(&r.body)).map_or(0, |i| i + 1);
        let above = tail.split_off(keep);
        blocks.push(Block { above, header: row, name: name.to_string(), body: Vec::new() });
    }
    (preamble, blocks)
}

/// Fold a pre-0.10 file's `[Looter]` and `[Gatherer]` sections into one
/// `[Harvest]` section. Pure; see the module docs for the rules.
pub fn migrate(text: &str) -> Migration {
    let (preamble, blocks) = blocks(text);
    let has_harvest = blocks.iter().any(|b| b.name.eq_ignore_ascii_case(INI_SECTION));
    let mut stale: Vec<String> = Vec::new();
    for b in blocks.iter().filter(|b| is_old(&b.name)) {
        if !stale.iter().any(|s| s.eq_ignore_ascii_case(&b.name)) {
            stale.push(b.name.clone());
        }
    }
    if stale.is_empty() {
        return Migration::Unchanged;
    }
    if has_harvest {
        return Migration::StaleOldSections(stale);
    }

    let newline: &'static str = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let looter_on = last_bool(text, OLD_LOOTER, "Enabled").unwrap_or(true);
    let gatherer_on = last_bool(text, OLD_GATHERER, "Enabled").unwrap_or(true);
    let enabled = looter_on && gatherer_on;
    let debug = last_bool(text, OLD_LOOTER, "Debug").unwrap_or(false)
        || last_bool(text, OLD_GATHERER, "Debug").unwrap_or(false);

    // The merged body: every [Looter] body, then every [Gatherer] body, in
    // file order within each.
    let mut body: Vec<Row> = Vec::new();
    for want in [OLD_LOOTER, OLD_GATHERER] {
        for b in blocks.iter().filter(|b| b.name.eq_ignore_ascii_case(want)) {
            body.extend(b.body.iter().cloned());
        }
    }
    let note = (looter_on != gatherer_on).then(|| disagreement_note(looter_on, gatherer_on));
    let body = fold_shared_keys(body, enabled, debug, note.as_deref(), newline);

    let mut out: Vec<Row> = preamble;
    let mut merged_done = false;
    for b in &blocks {
        if !is_old(&b.name) {
            out.extend(b.above.iter().cloned());
            out.push(b.header.clone());
            out.extend(b.body.iter().cloned());
            continue;
        }
        if merged_done {
            continue;
        }
        merged_done = true;
        let mut merged: Vec<Row> = b
            .above
            .iter()
            .map(|r| Row { body: rename_old(&r.body), eol: r.eol })
            .collect();
        let header_eol = if b.header.eol.is_empty() { newline } else { b.header.eol };
        merged.push(Row { body: format!("[{INI_SECTION}]"), eol: header_eol });
        merged.extend(body.iter().cloned());
        out.extend(collapse_blank_runs(merged));
    }

    // A row moved away from the end of the file may have no terminator; give
    // every row but the last one a line ending, so nothing joins up.
    let last = out.len().saturating_sub(1);
    let mut text_out = String::with_capacity(text.len() + 256);
    for (i, row) in out.iter().enumerate() {
        text_out.push_str(&row.body);
        text_out.push_str(if row.eol.is_empty() && i != last { newline } else { row.eol });
    }
    Migration::Rewritten(text_out)
}

/// `[Looter]` and `[Gatherer]` in a banner line become `[Harvest]`, in any
/// case the player wrote them.
fn rename_old(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(open) = rest.find('[') {
        let (before, from) = rest.split_at(open);
        out.push_str(before);
        let close = from.find(']');
        match close.and_then(|c| from.get(1..c)) {
            Some(name) if is_old(name.trim()) => {
                out.push('[');
                out.push_str(INI_SECTION);
                out.push(']');
                rest = close.and_then(|c| from.get(c + 1..)).unwrap_or("");
            }
            _ => {
                out.push('[');
                rest = from.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// How the first line of the note above a migrated `Enabled` starts.
const NOTE_PREFIX: &str = "; Migrated from 0.9:";

/// The note [`migrate`] left above `Enabled` because the two old sections
/// disagreed about it, as one line for the log, or `None` when they agreed.
pub fn enabled_note(migrated: &str) -> Option<String> {
    let first = migrated.lines().find(|l| l.trim_start().starts_with(NOTE_PREFIX))?;
    let first = first.trim_start().trim_start_matches(';').trim();
    Some(format!("{first} Enabled is now 0; the comment above it in the file says what to set."))
}

/// The comment put above the migrated `Enabled` when the two old sections
/// disagreed about it, so the player finds out where the file is read, not
/// only in the log.
fn disagreement_note(looter_on: bool, gatherer_on: bool) -> Vec<String> {
    let (l, g) = (u8::from(looter_on), u8::from(gatherer_on));
    let mut lines = vec![
        format!("{NOTE_PREFIX} [Looter] had Enabled={l} and [Gatherer] had Enabled={g}."),
        "; One Enabled now covers both, and it is 0 so that nothing runs that was off.".to_string(),
    ];
    if gatherer_on {
        lines.push("; For the yield multipliers without auto-loot, set Enabled=1 and keep".to_string());
        lines.push("; AutoGather=0.".to_string());
    } else {
        lines.push("; For auto-loot back, set Enabled=1; the multipliers then apply too, so".to_string());
        lines.push("; set them to 1 if you want vanilla yields.".to_string());
    }
    lines
}

/// Keep the first `Enabled=` and `Debug=` lines of the merged body with their
/// values rewritten, drop every later copy (and the comment run directly above
/// it, which describes a line that is gone), and insert either one after the
/// header if the body had none.
fn fold_shared_keys(
    rows: Vec<Row>,
    enabled: bool,
    debug: bool,
    note: Option<&[String]>,
    newline: &'static str,
) -> Vec<Row> {
    let shared: [(&str, bool); 2] = [("Enabled", enabled), ("Debug", debug)];
    let mut seen = [false; 2];
    let mut out: Vec<Row> = Vec::with_capacity(rows.len() + 8);
    for row in rows {
        let slot = pair_key(&row.body)
            .and_then(|k| shared.iter().position(|(name, _)| k.eq_ignore_ascii_case(name)));
        let Some(slot) = slot else {
            out.push(row);
            continue;
        };
        let (Some(done), Some(&(name, value))) = (seen.get_mut(slot), shared.get(slot)) else {
            out.push(row);
            continue;
        };
        if *done {
            let keep = out.iter().rposition(|r| !is_comment(&r.body)).map_or(0, |i| i + 1);
            out.truncate(keep);
            continue;
        }
        *done = true;
        if name == "Enabled" {
            for line in note.unwrap_or(&[]) {
                out.push(Row { body: line.clone(), eol: newline });
            }
        }
        let eol = if row.eol.is_empty() { newline } else { row.eol };
        out.push(Row { body: with_value(&row.body, if value { "1" } else { "0" }), eol });
    }

    // A key neither old section had goes right after the header and a blank
    // line, `Enabled` first.
    let mut missing: Vec<Row> = Vec::new();
    for (i, (name, value)) in shared.iter().enumerate() {
        if seen.get(i).copied().unwrap_or(true) {
            continue;
        }
        if *name == "Enabled" {
            for line in note.unwrap_or(&[]) {
                missing.push(Row { body: line.clone(), eol: newline });
            }
        }
        missing.push(Row { body: format!("{name}={}", u8::from(*value)), eol: newline });
    }
    if !missing.is_empty() {
        let mut head: Vec<Row> = vec![Row { body: String::new(), eol: newline }];
        head.extend(missing);
        head.extend(out);
        out = head;
    }
    out
}

/// Two or more blank rows in a row become one. Applied to the merged block
/// only, where the seams between the two old bodies and the dropped lines
/// leave them.
fn collapse_blank_runs(rows: Vec<Row>) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::with_capacity(rows.len());
    for row in rows {
        let blank = is_blank(&row.body);
        if blank && out.last().is_some_and(|r| is_blank(&r.body)) {
            continue;
        }
        out.push(row);
    }
    out
}

/// Read `path`, migrate it, and on [`Migration::Rewritten`] write the result
/// back through a temporary file and a rename. A missing file is
/// [`Migration::Unchanged`]: there is nothing to migrate.
pub fn migrate_file(path: &Path) -> io::Result<Migration> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Migration::Unchanged),
        Err(e) => return Err(e),
    };
    let m = migrate(&text);
    if let Migration::Rewritten(new) = &m {
        ini::write_atomically(path, new)?;
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    /// The `DesertTooling.ini` 0.9.0 shipped, byte for byte: the file most
    /// players will have when they upgrade.
    const PRE_MERGE: &str = include_str!("../tests/fixtures/pre-merge.ini");

    fn rewritten(text: &str) -> String {
        match migrate(text) {
            Migration::Rewritten(t) => t,
            other => panic!("expected a rewrite, got {other:?}"),
        }
    }

    /// Every header in `text`, in file order.
    fn headers(text: &str) -> Vec<String> {
        text.lines().filter_map(header_name).map(str::to_string).collect()
    }

    fn count_key(text: &str, key: &str) -> usize {
        text.lines().filter(|l| pair_key(l).is_some_and(|k| k.eq_ignore_ascii_case(key))).count()
    }

    #[test]
    fn the_pre_merge_template_becomes_harvest_at_its_defaults() {
        let out = rewritten(PRE_MERGE);
        assert_eq!(headers(&out), ["Harvest", "Overlay", "Dispatch"]);
        let (cfg, w) = config::parse(&out);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg, config::Config::default());
        // Every key of both old sections is under [Harvest], once. Keys added
        // after 0.9.0 were never in that file, and the migration does not
        // invent them: they read as their defaults, which `cfg` above proves.
        const ADDED_SINCE_0_9_0: &[&str] = &["GatherCritters", "Critters"];
        for field in config::schema().fields {
            let n = ini::lines_in_section(&out, "Harvest")
                .filter(|l| matches!(l, Line::Pair(k, _) if k.eq_ignore_ascii_case(&field.key)))
                .count();
            let want = usize::from(!ADDED_SINCE_0_9_0.contains(&field.key.as_str()));
            assert_eq!(n, want, "{}", field.key);
        }
    }

    #[test]
    fn exactly_one_enabled_and_debug_remain_with_the_looters_comment_above() {
        let out = rewritten(PRE_MERGE);
        let harvest: Vec<&str> = {
            let start = out.find("\n[Harvest]\n").unwrap_or_else(|| panic!("no [Harvest]"));
            let end = out.find("\n[Overlay]\n").unwrap_or(out.len());
            out[start..end].lines().collect()
        };
        let at = |key: &str| harvest.iter().position(|l| pair_key(l) == Some(key));
        assert_eq!(harvest.iter().filter(|l| pair_key(l) == Some("Enabled")).count(), 1);
        assert_eq!(harvest.iter().filter(|l| pair_key(l) == Some("Debug")).count(), 1);
        let enabled = at("Enabled").unwrap_or_else(|| panic!("no Enabled"));
        // The comment run directly above it is the [Looter] one.
        let above: Vec<&str> =
            harvest[..enabled].iter().rev().take_while(|l| is_comment(l)).copied().collect();
        let first = above.last().copied().unwrap_or_default();
        assert!(first.starts_with("; Master switch. 0 = the looter is idle"), "{above:?}");
        assert_eq!(harvest.iter().filter(|l| l.starts_with("; Master switch")).count(), 1);
        // The gatherer's Debug comment went with its line.
        assert!(!out.contains("; 1 = also log the records that are NOT gather nodes"));
        // Enabled and Debug come before every other key, as in [Looter].
        assert!(at("Enabled") < at("Debug") && at("Debug") < at("LogReceived"));
        assert!(at("KeyRecord") < at("DryRun") && at("Fish").is_some());
    }

    #[test]
    fn the_banners_go_and_the_first_comes_back_renamed() {
        let out = rewritten(PRE_MERGE);
        assert!(out.contains("; [Harvest] - auto-loot\n"), "the [Looter] banner, renamed");
        assert!(!out.contains("; [Looter] - auto-loot"));
        assert!(!out.contains("; [Gatherer] - gathering yield multipliers"));
        assert!(
            !out.contains("; A changed multiplier takes effect on the NEXT GATHER"),
            "the [Gatherer] banner is dropped whole"
        );
        // The renamed banner sits directly above the header.
        assert!(out.contains(
            "; [Harvest] - auto-loot\n; ===========================================================================\n[Harvest]\n"
        ));
        assert!(!out.contains("\n\n\n"), "no doubled blank lines at the seams");
    }

    #[test]
    fn the_preamble_overlay_and_dispatch_are_byte_identical() {
        let out = rewritten(PRE_MERGE);
        let banner = "; ===========================================================================\n; [Looter]";
        let pre = PRE_MERGE.find(banner).unwrap_or_else(|| panic!("no [Looter] banner"));
        assert!(out.starts_with(&PRE_MERGE[..pre]), "everything above the first banner is untouched");
        let overlay = "; ===========================================================================\n; [Overlay]";
        let tail_in = PRE_MERGE.find(overlay).unwrap_or_else(|| panic!("no [Overlay] banner"));
        let tail_out = out.find(overlay).unwrap_or_else(|| panic!("the [Overlay] banner is gone"));
        assert_eq!(&out[tail_out..], &PRE_MERGE[tail_in..]);
    }

    /// `Enabled` is an AND over the two old sections, `Debug` an OR, over all
    /// four combinations of each.
    #[test]
    fn enabled_is_and_and_debug_is_or() {
        for l in [false, true] {
            for g in [false, true] {
                let text = format!(
                    "[Looter]\nEnabled={}\nDebug={}\nAutoGather=1\n\n[Gatherer]\nEnabled={}\nDebug={}\nForaging=3\n",
                    u8::from(l),
                    u8::from(l),
                    u8::from(g),
                    u8::from(g)
                );
                let out = rewritten(&text);
                let (cfg, w) = config::parse(&out);
                assert!(w.is_empty(), "{w:?}\n{out}");
                assert_eq!(cfg.enabled(), l && g, "{text}");
                assert_eq!(cfg.debug(), l || g, "{text}");
                assert!(cfg.looter.auto_gather && cfg.gatherer.foraging == 3, "{out}");
                assert_eq!(count_key(&out, "Enabled"), 1, "{out}");
                assert_eq!(count_key(&out, "Debug"), 1, "{out}");
                // The note appears exactly when the two disagreed, above Enabled.
                let noted = out.contains(NOTE_PREFIX);
                assert_eq!(noted, l != g, "{out}");
                assert_eq!(enabled_note(&out).is_some(), l != g);
                if noted {
                    let lines: Vec<&str> = out.lines().collect();
                    let e = lines.iter().position(|x| pair_key(x) == Some("Enabled")).unwrap_or(0);
                    let n = lines.iter().position(|x| x.starts_with(NOTE_PREFIX)).unwrap_or(usize::MAX);
                    assert!(n < e, "{out}");
                    let want = if g { "AutoGather=0" } else { "vanilla yields" };
                    assert!(out.contains(want), "{out}");
                }
            }
        }
    }

    #[test]
    fn missing_keys_fall_back_to_their_defaults() {
        // No Enabled anywhere: both count as 1, so it is 1. No Debug: 0.
        let out = rewritten("[Looter]\nAutoGather=1\n[Gatherer]\nOre=2\n");
        let (cfg, w) = config::parse(&out);
        assert!(w.is_empty(), "{w:?}");
        assert!(cfg.enabled() && !cfg.debug());
        assert!(out.starts_with("[Harvest]\n\nEnabled=1\nDebug=0\n"), "{out}");
        // Only one side says 0: the missing one counts as 1, so they
        // disagree, and the result is 0 with the note.
        let out = rewritten("[Looter]\nAutoGather=1\n[Gatherer]\nEnabled=0\n");
        let (cfg, _) = config::parse(&out);
        assert!(!cfg.enabled());
        assert!(out.contains(NOTE_PREFIX), "{out}");
        // A Debug only the gatherer had is kept, and moved nowhere.
        let out = rewritten("[Looter]\nEnabled=1\n[Gatherer]\nDebug=1\n");
        let (cfg, _) = config::parse(&out);
        assert!(cfg.debug());
        assert_eq!(count_key(&out, "Debug"), 1, "{out}");
    }

    #[test]
    fn a_file_with_only_one_old_section_is_migrated_too() {
        let out = rewritten("; top\n\n[Looter]\nEnabled=0\nGatherOre=0\n");
        assert_eq!(headers(&out), ["Harvest"]);
        let (cfg, w) = config::parse(&out);
        assert!(w.is_empty(), "{w:?}");
        assert!(!cfg.enabled() && !cfg.looter.gather_ore);
        assert!(out.contains(NOTE_PREFIX), "Looter 0 against a missing (1) Gatherer: {out}");

        let out = rewritten("[Overlay]\nEnabled=1\n\n[Gatherer]\nMining=4\nDryRun=1\n");
        assert_eq!(headers(&out), ["Overlay", "Harvest"]);
        let (cfg, w) = config::parse(&out);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg.gatherer.mining, 4);
        assert!(cfg.gatherer.dry_run && cfg.enabled());
    }

    #[test]
    fn crlf_is_kept_including_on_inserted_lines() {
        let input = PRE_MERGE.replace('\n', "\r\n");
        let out = rewritten(&input);
        assert!(!out.replace("\r\n", "").contains('\n'), "a bare LF crept in");
        let (cfg, w) = config::parse(&out);
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(cfg, config::Config::default());
        // Inserted lines (the header, missing keys, the note) are CRLF too.
        let out = rewritten("[Looter]\r\nEnabled=0\r\n[Gatherer]\r\nOre=2\r\n");
        assert!(out.contains("[Harvest]\r\n"), "{out:?}");
        assert!(out.contains("Debug=0\r\n"), "{out:?}");
        assert!(out.contains(&format!("{NOTE_PREFIX} [Looter] had Enabled=0")), "{out:?}");
        assert!(!out.replace("\r\n", "").contains('\n'), "{out:?}");
    }

    #[test]
    fn spelling_and_spacing_of_the_kept_lines_survive() {
        let out = rewritten("[looter]\nENABLED = 1\n  debug=0\n[GATHERER]\nenabled=1\nDebug =1\n");
        assert!(out.contains("ENABLED = 1\n"), "{out}");
        assert!(out.contains("  debug=1\n"), "the OR, written into the looter's spelling: {out}");
        assert_eq!(count_key(&out, "debug"), 1, "{out}");
    }

    #[test]
    fn player_comments_and_hand_added_keys_travel_with_their_lines() {
        let text = "[Looter]\nEnabled=1\n; my own note\nMyKey=7\nGatherOre=0\n\n[Gatherer]\nEnabled=1\n; why 3\nForaging=3\n";
        let out = rewritten(text);
        assert!(out.contains("; my own note\nMyKey=7\nGatherOre=0\n"), "{out}");
        assert!(out.contains("; why 3\nForaging=3\n"), "{out}");
        // The hand-added key is now under [Harvest], where it is unknown;
        // that is the parser's business to warn about, not the migration's.
        let (_, w) = config::parse(&out);
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn migrating_twice_changes_nothing_the_second_time() {
        let once = rewritten(PRE_MERGE);
        assert_eq!(migrate(&once), Migration::Unchanged);
        for text in ["[Looter]\nEnabled=0\n", "[Gatherer]\r\nOre=2\r\n", "[Looter]\n[Gatherer]\n"] {
            let once = rewritten(text);
            assert_eq!(migrate(&once), Migration::Unchanged, "{once:?}");
        }
    }

    #[test]
    fn harvest_beside_an_old_section_is_reported_and_left_alone() {
        assert_eq!(
            migrate("[Harvest]\nEnabled=1\n[Looter]\nEnabled=0\n[gatherer]\n"),
            Migration::StaleOldSections(vec!["Looter".to_string(), "gatherer".to_string()])
        );
        assert_eq!(
            migrate("[Looter]\n[Harvest]\n[Looter]\n"),
            Migration::StaleOldSections(vec!["Looter".to_string()])
        );
        assert_eq!(migrate("[Harvest]\nEnabled=1\n"), Migration::Unchanged);
        assert_eq!(migrate("[Overlay]\nEnabled=1\n"), Migration::Unchanged);
        assert_eq!(migrate(""), Migration::Unchanged);
    }

    #[test]
    fn junk_never_panics() {
        for text in [
            "[Looter",
            "[Looter]",
            "[Gatherer]Enabled=0",
            "=\n[Looter]\n=\n[Gatherer]\n=",
            "[Looter]\nEnabled=0",
            "[]\n[\n]\n[Looter]]\n",
            "; [Looter]\n[Looter]\n; [Gatherer] [x [Looter\n[Gatherer]\n",
            "\r\n\r\n[Looter]\r",
            "[Looter]\n\u{e9}=\u{fc}\n[Gatherer]\n\u{1F600}[\n",
        ] {
            let m = migrate(text);
            if let Migration::Rewritten(out) = m {
                let _ = config::parse(&out);
                assert_ne!(migrate(&out), Migration::Rewritten(out.clone()), "{text:?}");
            }
        }
        // An unterminated last line in an old section does not get the next
        // line glued onto it.
        let out = rewritten("[Looter]\nGatherOre=0");
        assert!(out.contains("GatherOre=0"), "{out:?}");
        let out = rewritten("[Gatherer]\nOre=2\n[Overlay]\nTheme=x");
        assert!(out.ends_with("[Overlay]\nTheme=x"), "{out:?}");
        let out = rewritten("[Overlay]\nTheme=x\n[Looter]\nGatherOre=0");
        assert!(out.ends_with("GatherOre=0"), "{out:?}");
    }

    /// Prints what the 0.9.0 template becomes, with `[Gatherer] Enabled=0`
    /// so the note shows. `cargo test -- --ignored --nocapture show_migration`.
    #[test]
    #[ignore]
    fn show_migration() {
        println!("{}", rewritten(&PRE_MERGE.replace("launch.\nEnabled=1\n", "launch.\nEnabled=0\n")));
    }

    #[test]
    fn migrate_file_rewrites_in_place_and_leaves_no_temporary() {
        let dir = std::env::temp_dir().join(format!("desert-harvest-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("DesertTooling.ini");
        std::fs::write(&path, PRE_MERGE).unwrap();
        let m = migrate_file(&path).unwrap();
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(m, Migration::Rewritten(on_disk.clone()));
        assert!(!desert_core::ini::temp_path(&path).exists(), "the staging file is gone");
        // The second launch finds nothing to do.
        assert_eq!(migrate_file(&path).unwrap(), Migration::Unchanged);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), on_disk);
        // A missing file is nothing to migrate, not an error.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(migrate_file(&path).unwrap(), Migration::Unchanged);
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
