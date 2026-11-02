//! `reference-signatures.txt`, read with names.
//!
//! `sigscan` reads the file as bare pattern lines. `triage` needs to say WHICH
//! signature died, so it also reads the comment block above each pattern: the
//! first comment line of the form `# <name> - <description>` names the next
//! pattern line, as every entry in the file does today (`# alloc_event      -
//! function start; ...`). A pattern with no such line is named by its line
//! number. A name line containing `[optional]` marks a signature whose loss
//! does not fail `triage`: one the plugin scans for and can do without.

use std::path::Path;

use anyhow::{Context, Result};

use desert_tools::pattern::Pattern;

#[derive(Debug, Clone)]
pub struct Sig {
    pub name: String,
    pub line: usize,
    pub pattern: Pattern,
    pub optional: bool,
}

/// The name a `# name - description` comment line gives, if it is one.
///
/// A deeply indented comment is a continuation of the one above, never a
/// name, however many ` - ` it holds.
fn name_of(comment: &str) -> Option<(String, bool)> {
    let after = comment.strip_prefix('#')?;
    if after.starts_with("  ") {
        return None;
    }
    let body = after.trim();
    let (name, _) = body.split_once(" - ")?;
    let name = name.trim();
    if name.is_empty() || name.contains(':') || name.len() > 40 {
        return None;
    }
    Some((name.to_string(), comment.contains("[optional]")))
}

pub fn parse(text: &str) -> Vec<Sig> {
    let mut out = Vec::new();
    let mut pending: Option<(String, bool)> = None;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if raw.starts_with('#') {
            if let Some(n) = name_of(raw) {
                pending = Some(n);
            }
            continue;
        }
        let Some(pattern) = Pattern::parse(line) else { continue };
        let (name, optional) = pending.take().unwrap_or_else(|| (format!("line {}", i + 1), false));
        out.push(Sig { name, line: i + 1, pattern, optional });
    }
    out
}

pub fn read(path: &Path) -> Result<Vec<Sig>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the signature list {}", path.display()))?;
    Ok(parse(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_the_comment_above() {
        let text = "# Header prose: not a name\n\
                    # alloc_event      - function start; returns\n\
                    48 89 5C 24 ?? 57\n\
                    # desc_mask+queue  - call site; E8@+0 -> x\n\
                    #                    continuation - not a name either\n\
                    E8 ?? ?? ?? ?? 44\n\
                    90 90\n\
                    # category fn      - [optional] scanned, never consumed\n\
                    48 89 5C 24 18\n";
        let s = parse(text);
        let names: Vec<&str> = s.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["alloc_event", "desc_mask+queue", "line 7", "category fn"]);
        assert_eq!(s.iter().filter(|x| x.optional).count(), 1);
        assert!(s[3].optional);
        assert_eq!(s[1].line, 6);
    }

    #[test]
    fn the_real_file_names_every_signature() {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("reference-signatures.txt"),
        )
        .unwrap();
        let s = parse(&text);
        assert!(!s.is_empty());
        for x in &s {
            assert!(!x.name.starts_with("line "), "line {} has no `# name - ...` comment", x.line);
        }
    }
}
