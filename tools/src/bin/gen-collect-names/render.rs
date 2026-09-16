//! Emit: the whole of `desert-core/src/collect.rs`.
//!
//! The generator owns the whole file - enum, rows, both lookups and every test -
//! so `collect.rs` can be regenerated straight over itself. That was not true
//! before: the hand-written "Tried and rejected" note on `family_by_name` and
//! the `non_gather_records_stay_out` test used to be dropped on every run, and
//! the file's header said as much. They are emitted here now ([`HAND_NOTE`] and
//! [`HAND_TEST`]); anything else hand-added to `collect.rs` still dies on the
//! next run, so add it here instead.

use crate::spec::FamilySpec;
use crate::Rows;

/// Hand-written parts the generator used to drop. They record findings the
/// table cannot, so they live here rather than in the generated file.
const HAND_NOTE: &[&str] = &[
    "/// Exact record names only. Tried and rejected (2026-09-06): treating the",
    "/// felled-tree chunks (`log_*`) as Logging. The pickup event is ignored for",
    "/// them; the chunk actor even survives having its `firewood_*` materials cut",
    "/// out. Only the materials are gather nodes.",
];

const HAND_TEST: &[&str] = &[
    "    #[test]",
    "    fn non_gather_records_stay_out() {",
    "        assert_eq!(family_by_name(\"log_1002_index07\"), None);",
    "        assert_eq!(family_by_name(\"gimmick_tree_cd_crop_apple_02_collect\"), None);",
    "        assert_eq!(family_by_name(\"item_basic_onehand\"), None);",
    "        assert_eq!(family_by_name(\"firewood_1002_index07\"), Some(Family::Logging));",
    "    }",
];

/// `text` as `/// ` lines, hard-wrapped, with no trailing whitespace.
fn wrap_doc(text: &str) -> Vec<String> {
    const WIDTH: usize = 76;
    let mut out = Vec::new();
    let mut line = String::from("///");
    for word in text.split_whitespace() {
        if line != "///" && line.chars().count() + 1 + word.chars().count() > WIDTH {
            out.push(std::mem::replace(&mut line, String::from("///")));
        }
        line.push(' ');
        line.push_str(word);
    }
    out.push(line);
    out
}

/// A record map's entries as `(key, name)` pairs, sorted like the table.
fn by_name(records: &[crate::spec::Record]) -> Vec<(u32, &str)> {
    let mut v: Vec<(u32, &str)> = records.iter().map(|r| (r.key, r.name.as_str())).collect();
    v.sort_by_key(|(_, n)| n.to_lowercase());
    v
}

fn indent(lines: Vec<String>) -> impl Iterator<Item = String> {
    lines.into_iter().map(|l| format!("    {l}"))
}

/// The module header, the enum, the rows, both lookups and the fixed tests.
fn render(rows: &Rows, order: &[String], spec: &[FamilySpec]) -> Vec<String> {
    let mut out: Vec<String> = [
        "//! Gather-node records of Crimson Desert build 25116796, generated from the",
        "//! Desert Gatherer DMM pack (gimmickinfo records patched by that pack) plus",
        "//! tools/extra-families.json (the records DMM has no module for).",
        "//! `(record key, record name, family)`. Regenerate with tools/target/x86_64-unknown-linux-gnu/release/gen-collect-names.",
        "//!",
        "//! That generator rewrites this whole file and it is now safe to run straight",
        "//! over it: it owns the enum, the rows, both lookups AND the tests below,",
        "//! including the `family_by_name` note and `non_gather_records_stay_out`,",
        "//! which earlier versions dropped. Anything hand-added HERE still dies on the",
        "//! next run - add it to the generator instead.",
        "",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub enum Family {",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    for (i, family) in order.iter().enumerate() {
        let note = spec
            .iter()
            .find(|f| &f.name == family && !f.note.is_empty())
            .map(|f| f.note.as_str());
        match note {
            Some(note) => {
                // A noted variant is set off by a blank line on each side, so
                // the doc comment reads as belonging to one variant and not to
                // the run of bare ones above it.
                if i != 0 {
                    out.push(String::new());
                }
                out.extend(indent(wrap_doc(note)));
                out.push(format!("    {family},"));
                if i + 1 < order.len() {
                    out.push(String::new());
                }
            }
            None => out.push(format!("    {family},")),
        }
    }

    out.extend(
        ["}", "", "pub const COLLECT_RECORDS: &[(u32, &str, Family)] = &["]
            .iter()
            .map(|s| (*s).to_string()),
    );
    for (name, key, family) in rows.sorted_by_name() {
        out.push(format!("    ({key}, \"{name}\", Family::{family}),"));
    }
    out.extend(
        [
            "];",
            "",
            "/// Family of a gather record by its key, if it is one.",
            "pub fn family_by_key(key: u32) -> Option<Family> {",
            "    COLLECT_RECORDS.iter().find(|(k, _, _)| *k == key).map(|(_, _, f)| *f)",
            "}",
            "",
            "/// Family of a gather record by its name (case-insensitive), if it is one.",
        ]
        .iter()
        .map(|s| (*s).to_string()),
    );
    out.extend(HAND_NOTE.iter().map(|s| (*s).to_string()));
    out.extend(
        [
            "pub fn family_by_name(name: &str) -> Option<Family> {",
            "    COLLECT_RECORDS.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name)).map(|(_, _, f)| *f)",
            "}",
            "",
            "#[cfg(test)]",
            "mod tests {",
            "    use super::*;",
        ]
        .iter()
        .map(|s| (*s).to_string()),
    );
    out.extend(HAND_TEST.iter().map(|s| (*s).to_string()));
    out.extend(
        [
            "",
            "    #[test]",
            "    fn known_records() {",
            "        assert_eq!(family_by_name(\"peony_01\"), Some(Family::Foraging));",
            "        assert_eq!(family_by_key(17020006), Some(Family::Foraging));",
            "        assert_eq!(family_by_name(\"ore_copper_01\"), Some(Family::Ore));",
            "        assert_eq!(family_by_name(\"gimmick_gate_metal_lattice_01_dungeon\"), None);",
        ]
        .iter()
        .map(|s| (*s).to_string()),
    );
    out.push(format!("        assert_eq!(COLLECT_RECORDS.len(), {});", rows.count()));
    out.push("    }".to_string());
    out
}

/// Two tests per `extra-families.json` entry.
///
/// The first says its rows are in the table under the right family and that the
/// family's total is what the DMM pack plus this entry add up to. The second is
/// the more valuable one: every `records_not_enabled` record must resolve to NO
/// family, by key and by name. Those are the records a table edit was measured
/// not to reach, and a row for one would be work the log reports and the player
/// never sees - which is exactly what shipped once.
fn render_extra_tests(spec: &[FamilySpec], rows: &Rows) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for family in spec {
        let name = &family.name;
        let lower = name.to_lowercase();
        let total = rows.count_in_family(name);
        let items: Vec<u32> = {
            let mut v: Vec<u32> = family.records.iter().flat_map(|r| r.items.clone()).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let items = items.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
        out.push(String::new());
        out.push("    /// Rows from tools/extra-families.json, not from the DMM pack.".to_string());
        out.extend(indent(wrap_doc(&format!("Items paid: {items}."))));
        out.push("    #[test]".to_string());
        out.push(format!("    fn {lower}_extra_records_are_present() {{"));
        // A named slice rather than an array literal in the `for` head:
        // clippy's single_element_loop fires on the latter when an entry has
        // exactly one record, and this must be clean at any count.
        out.push("        let extra: &[(u32, &str)] = &[".to_string());
        for (key, rname) in by_name(&family.records) {
            out.push(format!("            ({key}, \"{rname}\"),"));
        }
        out.extend(
            [
                "        ];".to_string(),
                "        for &(key, name) in extra {".to_string(),
                format!("            assert_eq!(family_by_key(key), Some(Family::{name}), \"{{name}}\");"),
                format!("            assert_eq!(family_by_name(name), Some(Family::{name}), \"{{name}}\");"),
                "        }".to_string(),
                "        assert_eq!(".to_string(),
                format!("            COLLECT_RECORDS.iter().filter(|(_, _, f)| *f == Family::{name}).count(),"),
                format!("            {total}"),
                "        );".to_string(),
                "    }".to_string(),
            ],
        );
        let skipped = by_name(&family.records_not_enabled);
        if skipped.is_empty() {
            continue;
        }
        out.extend(
            [
                "",
                "    /// `records_not_enabled` in tools/extra-families.json: records a table",
                "    /// edit was measured or argued not to reach - they hand the player a",
                "    /// pre-built item instance and never read their output block (see",
                "    /// docs/findings-water-wells-2026-09-12.md section 11), or they are",
                "    /// not pickups at all; each record's `why` in that file says which.",
                "    /// They must stay OUT of the table, not sit in it inert: a row here",
                "    /// would be an edit the log reports and the player never sees.",
                "    #[test]",
            ]
            .iter()
            .map(|s| (*s).to_string()),
        );
        out.push(format!("    fn {lower}_records_not_enabled_stay_out() {{"));
        out.push("        let out: &[(u32, &str)] = &[".to_string());
        for (key, rname) in skipped {
            out.push(format!("            ({key}, \"{rname}\"),"));
        }
        out.extend(
            [
                "        ];",
                "        for &(key, name) in out {",
                "            assert_eq!(family_by_key(key), None, \"{name}\");",
                "            assert_eq!(family_by_name(name), None, \"{name}\");",
                "        }",
                "    }",
            ]
            .iter()
            .map(|s| (*s).to_string()),
        );
    }
    out.push("}".to_string());
    out
}

/// The whole file, newline-terminated.
pub fn collect_rs(rows: &Rows, order: &[String], spec: &[FamilySpec]) -> String {
    let mut text = render(rows, order, spec);
    text.extend(render_extra_tests(spec, rows));
    let mut s = text.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_doc_hard_wraps_at_76_columns() {
        let lines = wrap_doc(&"word ".repeat(40));
        assert!(lines.iter().all(|l| l.starts_with("///")), "{lines:?}");
        assert!(lines.iter().all(|l| l.chars().count() <= 76), "{lines:?}");
        assert!(lines.len() > 1);
        // No trailing whitespace: a generated file that fails `cargo fmt --check`
        // is a diff nobody can read.
        assert!(lines.iter().all(|l| l.trim_end() == l), "{lines:?}");
    }

    #[test]
    fn wrap_doc_keeps_a_word_that_does_not_fit_on_its_own_line() {
        let long = "x".repeat(200);
        let lines = wrap_doc(&format!("short {long}"));
        assert_eq!(lines, vec!["/// short".to_string(), format!("/// {long}")]);
    }

    #[test]
    fn by_name_sorts_case_insensitively() {
        let recs = vec![
            crate::spec::Record { key: 2, name: "beta".into(), items: vec![1], why: String::new() },
            crate::spec::Record { key: 1, name: "Alpha".into(), items: vec![1], why: String::new() },
        ];
        assert_eq!(by_name(&recs), vec![(1, "Alpha"), (2, "beta")]);
    }
}
