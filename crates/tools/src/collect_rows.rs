//! The rows of `crates/desert-core/src/collect.rs`'s `COLLECT_RECORDS`, read
//! back out of the generated source.
//!
//! `gen-collect-names` writes that file; `items` attributes blocks with it,
//! `gen-collect-names` diffs against the copy it is about to replace, and
//! `dmm-pack` builds the DMM pack's modules from it. One parser for all three.
//!
//! A row is `(key, "name", Family::X, "display")`. The display name is
//! optional so a three-field row from before it existed still reads.

use std::path::Path;

use anyhow::{ensure, Context, Result};
use regex::Regex;

/// One `COLLECT_RECORDS` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectRow {
    pub key: u32,
    pub name: String,
    /// The `Family` variant's name, `Foraging` for `Family::Foraging`.
    pub family: String,
    pub display: Option<String>,
}

/// Every row in `text`, in file order. Lines that are not rows are skipped.
pub fn parse(text: &str) -> Vec<CollectRow> {
    let row = Regex::new(r#"^\s*\((\d+),\s*"([^"]+)",\s*Family::(\w+)(?:,\s*"([^"]*)")?[,)]"#).expect("static regex");
    text.lines()
        .filter_map(|l| row.captures(l))
        .filter_map(|m| {
            Some(CollectRow {
                key: m[1].parse().ok()?,
                name: m[2].to_string(),
                family: m[3].to_string(),
                display: m.get(4).map(|d| d.as_str().to_string()),
            })
        })
        .collect()
}

/// Every row of the file at `path`; an error if it cannot be read or holds
/// no row at all.
pub fn read(path: &Path) -> Result<Vec<CollectRow>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let rows = parse(&text);
    ensure!(!rows.is_empty(), "no COLLECT_RECORDS rows parsed out of {}", path.display());
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_row_shapes_read() {
        let text = "    (1, \"a\", Family::Foraging),\n    (2, \"b\", Family::Logging, \"Timber\"),\n    (3, \"c\")\n  // (4, \"d\", Family::Ore, \"x\"),\n";
        let rows = parse(text);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], CollectRow { key: 1, name: "a".into(), family: "Foraging".into(), display: None });
        assert_eq!(rows[1].display.as_deref(), Some("Timber"));
        assert_eq!(rows[1].family, "Logging");
    }

    #[test]
    fn an_empty_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("collect.rs");
        std::fs::write(&p, "pub enum Family {}\n").unwrap();
        assert!(read(&p).is_err());
        assert!(read(&dir.path().join("none.rs")).is_err());
    }
}
