//! Turn a release tag into a release title and release notes.
//!
//! ```text
//! release-notes desert-tooling-v0.3.0              notes, as markdown, on stdout
//! release-notes desert-tooling-v0.3.0 --title      just the title
//! release-notes TAG --sums dist/SHA256SUMS         notes plus a table of the attached files
//! release-notes desert-tooling-v0.3.0 --changelog  just the changelog entry
//! release-notes desert-tooling-v0.3.0 --package    just the package name
//! ```
//!
//! The tag names one package and its version (VERSIONING.md step 6):
//!
//! ```text
//! desert-tooling-v<semver>       Desert Tooling       version from desert-tooling/Cargo.toml
//! desert-gatherer-dmm-v<x.y>     Desert Gatherer DMM  version from desert-gatherer-dmm/dmm_pack.json
//! ```
//!
//! The version in the tag must equal the one in that source of truth, or this
//! exits 1: a release built from a commit whose `Cargo.toml` disagrees with its
//! tag would print one number on the log's first line and carry another on the
//! release page. The release workflow runs this before it builds anything.
//!
//! The notes are the package's own changelog entry for that version (the DMM
//! pack keeps its history in its README's "What Version" section), then, with
//! `--sums`, the SHA256 of every file attached to the release.
//!
//! `--changelog` prints that entry and nothing else. It is what the release
//! workflow sends to the mod's Nexus Mods page, where the file list and the
//! checksum table would be noise: a Nexus page carries one package, and its
//! Files tab already shows what is attached.
//!
//! Ported from `tools/release-notes.py`. `.github/workflows/release.yml` pipes
//! this into `gh release create` and into `$GITHUB_OUTPUT`, so stdout is
//! byte-for-byte what the Python printed, down to the blank line before the
//! Files heading, and the two refusals still exit 1 with the same wording.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;

use desert_tools::paths;

const SEMVER: &str = r"\d+\.\d+\.\d+";
const PACK_VERSION: &str = r"\d+\.\d+";

/// Where a package's version really lives. The tag only claims a version; this
/// is what it is checked against.
enum Source {
    /// The `[package]` version of a crate in the workspace.
    Cargo(&'static str),
    /// `desert-gatherer-dmm/dmm_pack.json`. The pack is not a crate.
    Pack,
}

/// Which heading in the history file holds the entry for a version.
enum Heading {
    /// Keep a Changelog: one `## [x.y.z]` heading per release.
    PerVersion,
    /// The DMM pack's README has one standing section instead, rewritten in
    /// place each time the pack is rebuilt.
    Fixed(&'static str),
}

struct Package {
    /// Tag prefix.
    prefix: &'static str,
    /// Display name, which with the version becomes the release title.
    name: &'static str,
    /// What the version in the tag is allowed to look like.
    version_pattern: &'static str,
    source: Source,
    history: &'static str,
    heading: Heading,
}

static PACKAGES: [Package; 2] = [
    Package {
        prefix: "desert-tooling",
        name: "Desert Tooling",
        version_pattern: SEMVER,
        source: Source::Cargo("desert-tooling"),
        history: "desert-tooling/CHANGELOG.md",
        heading: Heading::PerVersion,
    },
    Package {
        prefix: "desert-gatherer-dmm",
        name: "Desert Gatherer DMM pack",
        version_pattern: PACK_VERSION,
        source: Source::Pack,
        history: "desert-gatherer-dmm/README.md",
        heading: Heading::Fixed("## What Version"),
    },
];

#[derive(Parser)]
#[command(
    name = "release-notes",
    about = "Turn a release tag into a release title and release notes."
)]
struct Cli {
    tag: String,
    /// print only the release title
    #[arg(long)]
    title: bool,
    /// print only the package name, for the dist tool
    #[arg(long)]
    package: bool,
    /// print only the changelog entry, for Nexus Mods
    #[arg(long)]
    changelog: bool,
    /// dist/SHA256SUMS, to list the attached files
    #[arg(long, value_name = "SUMS")]
    sums: Option<PathBuf>,
}

/// Die the way the Python did: the message on stderr, status 1, no `Error:`
/// prefix. The release workflow surfaces these verbatim, and a release engineer
/// reading a red step is the whole audience.
fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

/// Python's `repr` of a string, because these messages quoted their inputs with
/// `{x!r}` and the workflow logs from before the port are still the reference.
fn py_repr(s: &str) -> String {
    // Python prefers single quotes, switching to double only when the string
    // contains a single quote and no double quote.
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python's `re.escape`, whose output ends up inside an error message.
///
/// Since 3.7 it escapes only the characters below, so `re.escape("0.6.0")` is
/// `0\.6\.0` and not `0\.6\.0` with the digits escaped too. The difference is
/// visible: the "no heading matching ..." message prints the pattern.
fn py_re_escape(s: &str) -> String {
    const SPECIAL: &str = "()[]{}?*+-|^$\\.&~# \t\n\r\x0b\x0c";
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if SPECIAL.contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The body of a manifest's first `[section]`, which is `[package]`.
///
/// Stopping at the next header matters: `[dependencies]` entries carry
/// `version = "..."` too, and the first one in the file is not the package's.
fn first_section(text: &str) -> &str {
    let mut starts = text
        .match_indices('[')
        .filter(|(i, _)| *i == 0 || text.as_bytes()[i - 1] == b'\n')
        .map(|(i, _)| i + 1);
    let Some(begin) = starts.next() else {
        return "";
    };
    &text[begin..starts.next().map_or(text.len(), |next| next - 1)]
}

fn cargo_version(root: &Path, crate_name: &str) -> Result<String> {
    let manifest = root.join(crate_name).join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("cannot read {}", manifest.display()))?;
    let version = Regex::new(&format!(r#"(?m)^version\s*=\s*"({SEMVER})""#)).unwrap();
    match version.captures(first_section(&text)) {
        Some(found) => Ok(found[1].to_string()),
        None => fail(&format!(
            "release-notes: no package version in {crate_name}/Cargo.toml"
        )),
    }
}

fn pack_version(root: &Path) -> Result<String> {
    let manifest = root.join("desert-gatherer-dmm").join("dmm_pack.json");
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("cannot read {}", manifest.display()))?;
    let usable = Regex::new(&format!(r"\A{PACK_VERSION}\z")).unwrap();
    let version = serde_json::from_str::<serde_json::Value>(&text)
        .with_context(|| format!("cannot parse {}", manifest.display()))?
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    match version {
        Some(v) if usable.is_match(&v) => Ok(v),
        _ => fail(&format!(
            "release-notes: no version in {}",
            manifest.display()
        )),
    }
}

/// `desert-gatherer-dmm-v1.1` -> (the pack, "1.1"). Longest prefix wins.
fn parse_tag(tag: &str) -> (&'static Package, String) {
    let mut by_length: Vec<&Package> = PACKAGES.iter().collect();
    by_length.sort_by_key(|p| std::cmp::Reverse(p.prefix.len()));
    for package in by_length {
        let pattern = Regex::new(&format!(
            r"\A{}-v({})\z",
            regex::escape(package.prefix),
            package.version_pattern
        ))
        .unwrap();
        if let Some(found) = pattern.captures(tag) {
            return (package, found[1].to_string());
        }
    }
    let expected = PACKAGES
        .iter()
        .map(|p| format!("{}-v<version>", p.prefix))
        .collect::<Vec<_>>()
        .join(", ");
    fail(&format!(
        "release-notes: tag {} is not one of {expected}",
        py_repr(tag)
    ));
}

/// The body under the first `## ` heading matching `heading`, up to the next `## `.
fn section(root: &Path, history: &str, heading: &str) -> Result<String> {
    let path = root.join(history);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    // Anchored, like Python's `re.match`: a `## [0.3.0]` heading must start the
    // line, so a changelog bullet that mentions one does not open a section.
    let anchored = Regex::new(&format!(r"\A(?:{heading})")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let Some(start) = lines.iter().position(|line| anchored.is_match(line)) else {
        fail(&format!(
            "release-notes: no heading matching {} in {history}",
            py_repr(heading)
        ));
    };
    let body: Vec<&str> = lines[start + 1..]
        .iter()
        .take_while(|line| !line.starts_with("## "))
        .copied()
        .collect();
    let entry = body.join("\n").trim().to_string();
    if entry.is_empty() {
        fail(&format!(
            "release-notes: empty section under {} in {history}",
            py_repr(heading)
        ));
    }
    Ok(entry)
}

fn sums_table(sums: &Path) -> Result<String> {
    let text =
        std::fs::read_to_string(sums).with_context(|| format!("cannot read {}", sums.display()))?;
    let mut rows = vec!["| File | SHA256 |".to_string(), "| --- | --- |".to_string()];
    for line in text.lines() {
        // `sha256sum` writes digest, two spaces, name. Anything else here means
        // the file is not the one `sha256sum --check` was going to read.
        let (digest, name) = line.split_once("  ").unwrap_or((line, ""));
        if digest.is_empty() || name.is_empty() {
            fail(&format!(
                "release-notes: unreadable line in {}: {}",
                sums.display(),
                py_repr(line)
            ));
        }
        rows.push(format!("| `{name}` | `{digest}` |"));
    }
    let name = sums
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    rows.push(format!(
        "| `{name}` | the list above, for `sha256sum --check` |"
    ));
    Ok(rows.join("\n"))
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = paths::repo_root()?;

    let (package, tagged) = parse_tag(&cli.tag);

    let actual = match package.source {
        Source::Cargo(crate_name) => cargo_version(&root, crate_name)?,
        Source::Pack => pack_version(&root)?,
    };
    if tagged != actual {
        fail(&format!(
            "release-notes: tag {} says {tagged}, but the source says {actual}. \
             Bump the version (VERSIONING.md step 1) or fix the tag; never release the mismatch.",
            cli.tag
        ));
    }

    if cli.package {
        println!("{}", package.prefix);
        return Ok(());
    }

    let title = format!("{} {tagged}", package.name);
    if cli.title {
        println!("{title}");
        return Ok(());
    }

    let heading = match package.heading {
        Heading::PerVersion => format!(r"## \[{}\]", py_re_escape(&tagged)),
        Heading::Fixed(fixed) => fixed.to_string(),
    };
    let entry = section(&root, package.history, &heading)?;
    if cli.changelog {
        println!("{entry}");
        return Ok(());
    }

    let mut parts = vec![
        entry,
        String::new(),
        "## Files".to_string(),
        String::new(),
        format!(
            "{title} only, built from the tagged commit. The other packages in this repository \
             are versioned separately and each has its own tag and its own releases."
        ),
    ];
    if let Some(sums) = &cli.sums {
        parts.push(String::new());
        parts.push(sums_table(sums)?);
    }
    println!("{}", parts.join("\n"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repr_matches_python_for_the_strings_these_messages_quote() {
        assert_eq!(py_repr("desert-tooling-v9.9.9"), "'desert-tooling-v9.9.9'");
        // The heading pattern is full of backslashes, and repr doubles them.
        assert_eq!(py_repr(r"## \[0\.6\.0\]"), r"'## \\[0\\.6\\.0\\]'");
        assert_eq!(py_repr("it's"), "\"it's\"");
    }

    #[test]
    fn re_escape_leaves_digits_alone() {
        assert_eq!(py_re_escape("0.6.0"), r"0\.6\.0");
        assert_eq!(py_re_escape("1.1"), r"1\.1");
    }

    #[test]
    fn first_section_stops_before_the_dependencies() {
        let manifest =
            "[package]\nversion = \"0.6.0\"\n\n[dependencies]\nanyhow = { version = \"1.2.3\" }\n";
        assert!(first_section(manifest).contains("0.6.0"));
        assert!(!first_section(manifest).contains("1.2.3"));
    }

    #[test]
    fn the_longest_tag_prefix_wins() {
        // "desert-gatherer-dmm" would be unreachable if a shorter prefix were
        // allowed to match first; the pack's x.y version is the giveaway.
        let (package, version) = parse_tag("desert-gatherer-dmm-v1.1");
        assert_eq!(package.prefix, "desert-gatherer-dmm");
        assert_eq!(version, "1.1");
        let (package, version) = parse_tag("desert-tooling-v0.6.0");
        assert_eq!(package.prefix, "desert-tooling");
        assert_eq!(version, "0.6.0");
    }

    #[test]
    fn sums_table_names_the_sums_file_itself_last() {
        let dir = tempfile::tempdir().unwrap();
        let sums = dir.path().join("SHA256SUMS");
        std::fs::write(&sums, "abc123  DesertTooling-0.6.0.zip\n").unwrap();
        assert_eq!(
            sums_table(&sums).unwrap(),
            "| File | SHA256 |\n| --- | --- |\n| `DesertTooling-0.6.0.zip` | `abc123` |\n\
             | `SHA256SUMS` | the list above, for `sha256sum --check` |"
        );
    }
}
