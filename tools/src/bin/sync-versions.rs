//! Sync the version numbers in the docs with the crates' `Cargo.toml`.
//!
//! `Cargo.toml` is the single source of truth (VERSIONING.md says so); this
//! keeps the tables and the tag examples in the markdown from drifting away
//! from it.
//!
//! ```text
//! sync-versions            rewrite the docs in place
//! sync-versions --check    exit 1 if a doc is stale, changing nothing
//! ```
//!
//! Three things get rewritten. In every file listed in `DOCS`:
//!
//!   1. a table row whose first cell names a crate -> the cell holding a bare
//!      version number is set to that crate's version
//!   2. any `<crate>-v<version>` string anywhere -> the crate's version
//!
//! and in the shipping crate's own `README.md`, additionally:
//!
//!   3. the `**Version <version>**` line near the top -> that crate's version
//!
//! Only one crate ships now: desert-tooling, which builds `DesertTooling.asi`.
//! desert-looter, desert-gatherer, desert-overlay and desert-dispatch are
//! internal libraries linked into it - versioned for their own sake, never
//! tagged, no README of their own - so they are in `CRATES` (their numbers
//! still appear in the docs' tables) but not in `SHIPPING`.
//!
//! The DMM pack is not a crate and has its own source of truth,
//! `dmm_pack.json`, which `release-notes` reads for the
//! `desert-gatherer-dmm-v<x.y>` tag. Its twelve module files each repeat that
//! version twice, so:
//!
//!   4. every `"version": "<x.y>"` in `desert-gatherer-dmm/*.json` except the
//!      manifest itself -> the manifest's version
//!
//! Finally, one thing is checked but never written, because only a human can
//! write it: the shipping crate's `CHANGELOG.md` must carry a `## [<version>]`
//! heading for the version in its `Cargo.toml`. VERSIONING.md step 3 and
//! CLAUDE.md both say the entry lands in the same commit as the bump; without
//! this check nothing notices a missing entry until the release workflow
//! builds the notes, which is after the tag has been pushed.
//!
//! Ported from `tools/sync-versions.py`. Its output is consumed by `just
//! check-versions` and by both GitHub workflows, so the exit codes and every
//! line of the drift report are reproduced exactly; only the `--help` text is
//! new. One thing the port does NOT reproduce is the Python's newline
//! translation - see `sync_pack_module`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use regex::{Captures, Regex};

use desert_tools::paths;

const CRATES: [&str; 6] = [
    "desert-tooling",
    "desert-looter",
    "desert-gatherer",
    "desert-overlay",
    "desert-dispatch",
    "desert-core",
];
const DOCS: [&str; 2] = ["README.md", "VERSIONING.md"];
/// Crates that ship a plugin and carry a `**Version x.y.z**` line in their README.
const SHIPPING: [&str; 1] = ["desert-tooling"];

/// The DMM pack: a directory of JSON, versioned x.y, not a crate.
const PACK_DIR: &str = "desert-gatherer-dmm";
const PACK_MANIFEST: &str = "dmm_pack.json";

const SEMVER: &str = r"\d+\.\d+\.\d+";
/// A version *value* inside a JSON string: digits and dots, so the substitution
/// cannot touch some future `"version": "unreleased"` by accident.
const JSON_VERSION: &str = r"\d[\d.]*";

#[derive(Parser)]
#[command(
    name = "sync-versions",
    about = "Sync the version numbers in the docs with the crates' Cargo.toml."
)]
struct Cli {
    /// Exit 1 if a doc is stale, changing nothing.
    #[arg(long)]
    check: bool,
}

/// Die the way the Python did: the message on stderr, status 1, no `Error:`
/// prefix. These are the diagnostics a person is meant to read and act on, and
/// CI logs from before the port are still the reference for what they say.
fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

/// The compiled substitutions, built once. `sync` runs over every line of every
/// doc, and rebuilding these per line is the difference between instant and
/// noticeable on a repo this size.
struct Rewrites {
    /// A whole table cell that is nothing but a version number. `\s` swallows
    /// the trailing newline of a last cell, exactly as the Python's
    /// `re.fullmatch` did, and `trim` below puts it back.
    bare_version: Regex,
    /// One `<crate>-v<semver>` matcher per crate, in `CRATES` order.
    tag_refs: Vec<Regex>,
    /// The `**Version x.y.z**` line at the top of a shipping crate's README.
    version_line: Regex,
    /// A `"version": "x.y"` value in a DMM module file.
    json_version: Regex,
}

impl Rewrites {
    fn new() -> Self {
        Self {
            bare_version: Regex::new(&format!(r"\A\s*{SEMVER}\s*\z")).unwrap(),
            tag_refs: CRATES
                .iter()
                .map(|c| Regex::new(&format!("{}-v{SEMVER}", regex::escape(c))).unwrap())
                .collect(),
            version_line: Regex::new(&format!(r"(?m)^(\*\*Version ){SEMVER}(\*\*)")).unwrap(),
            json_version: Regex::new(&format!(r#"("version"\s*:\s*"){JSON_VERSION}(")"#)).unwrap(),
        }
    }
}

/// Read `version` from the `[package]` section of each crate's `Cargo.toml`.
fn crate_versions(root: &Path) -> Result<Vec<(&'static str, String)>> {
    let version = Regex::new(&format!(r#"(?m)^version\s*=\s*"({SEMVER})""#)).unwrap();
    let mut versions = Vec::with_capacity(CRATES.len());
    for name in CRATES {
        let manifest = root.join(name).join("Cargo.toml");
        let text = std::fs::read_to_string(&manifest)
            .with_context(|| format!("cannot read {}", manifest.display()))?;
        let Some(found) = version.captures(first_section(&text)) else {
            fail(&format!(
                "sync-versions: no package version in {}",
                manifest.display()
            ));
        };
        versions.push((name, found[1].to_string()));
    }
    Ok(versions)
}

/// The body of the manifest's first `[section]`, which is `[package]`.
///
/// Stopping at the next section header is the whole point: `[dependencies]`
/// entries carry `version = "..."` too, and a plain search for the first
/// `version =` in the file would happily return a dependency's.
fn first_section(text: &str) -> &str {
    let mut starts = text
        .match_indices('[')
        .filter(|(i, _)| *i == 0 || text.as_bytes()[i - 1] == b'\n')
        .map(|(i, _)| i + 1);
    let Some(begin) = starts.next() else {
        return "";
    };
    // A header's `[` is consumed, matching Python's `re.split(r"^\[")`, which
    // dropped the separator. Nothing downstream looks at the section's name.
    &text[begin..starts.next().map_or(text.len(), |next| next - 1)]
}

fn version_of<'a>(versions: &'a [(&str, String)], crate_name: &str) -> &'a str {
    versions
        .iter()
        .find(|(name, _)| *name == crate_name)
        .map(|(_, version)| version.as_str())
        .expect("crate_versions covers every name in CRATES")
}

fn sync(text: &str, versions: &[(&str, String)], re: &Rewrites) -> String {
    let mut out = String::with_capacity(text.len());
    for raw in text.split_inclusive('\n') {
        let mut line = raw.to_string();

        // Longest name wins: "desert-looter" is not a substring of another
        // crate today, but a future "desert-looter-cli" would be ambiguous.
        // Strictly longer, so that a line naming two crates of equal name
        // length picks the first one in CRATES, as Python's `max` did.
        let mut named: Option<&str> = None;
        for candidate in CRATES {
            let longer = named.is_none_or(|best: &str| candidate.len() > best.len());
            if longer && line.contains(candidate) {
                named = Some(candidate);
            }
        }

        if let Some(crate_name) = named {
            if line.trim_start().starts_with('|') {
                let version = version_of(versions, crate_name);
                let cells: Vec<String> = line
                    .split('|')
                    .map(|cell| {
                        if re.bare_version.is_match(cell) {
                            // Only the number is replaced; the cell's padding
                            // is what keeps the markdown table aligned.
                            let body = cell.trim();
                            let lead = &cell[..cell.len() - cell.trim_start().len()];
                            let trail = &cell[lead.len() + body.len()..];
                            format!("{lead}{version}{trail}")
                        } else {
                            cell.to_string()
                        }
                    })
                    .collect();
                line = cells.join("|");
            }
        }

        for (tag_ref, (_, version)) in re.tag_refs.iter().zip(versions) {
            let replacement = tag_ref
                .replace_all(&line, |caps: &Captures| {
                    // The match is `<crate>-v<semver>`; rebuild it around the
                    // new number rather than expanding `$`, which a version
                    // string will never contain but a crate name might grow.
                    let matched = &caps[0];
                    let cut = matched.rfind("-v").expect("the pattern contains -v") + 2;
                    format!("{}{version}", &matched[..cut])
                })
                .into_owned();
            line = replacement;
        }
        out.push_str(&line);
    }
    out
}

/// A shipping crate's README: the general rules plus its own Version line.
fn sync_readme(text: &str, crate_name: &str, versions: &[(&str, String)], re: &Rewrites) -> String {
    let text = sync(text, versions, re);
    let version = version_of(versions, crate_name);
    re.version_line
        .replacen(&text, 1, |caps: &Captures| {
            format!("{}{version}{}", &caps[1], &caps[2])
        })
        .into_owned()
}

/// The DMM pack's version, from its own manifest.
fn pack_version(root: &Path) -> Result<String> {
    let manifest = root.join(PACK_DIR).join(PACK_MANIFEST);
    let text = std::fs::read_to_string(&manifest)
        .with_context(|| format!("cannot read {}", manifest.display()))?;
    let usable = Regex::new(&format!(r"\A{JSON_VERSION}\z")).unwrap();
    let version = serde_json::from_str::<serde_json::Value>(&text)
        .with_context(|| format!("cannot parse {}", manifest.display()))?
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    match version {
        Some(v) if usable.is_match(&v) => Ok(v),
        _ => fail(&format!(
            "sync-versions: no usable version in {}",
            manifest.display()
        )),
    }
}

/// Every version value in a DMM module file follows the manifest.
///
/// There are two per file, the module's own and the one repeated inside
/// `modinfo`, and both are the pack's version.
///
/// The substitution is over the file's bytes, not over its lines, and that is
/// deliberate: the twelve module files are CRLF in git, every other file this
/// tool touches is LF. The Python read them through universal newlines and
/// wrote them back with `\n`, so bumping the pack turned a two-line change per
/// file into a 6470-line one and hid the real edit. Nothing consuming these
/// files cares which ending they use; the review of the commit that bumps them
/// does.
fn sync_pack_module(text: &str, version: &str, re: &Rewrites) -> String {
    re.json_version
        .replace_all(text, |caps: &Captures| {
            format!("{}{version}{}", &caps[1], &caps[2])
        })
        .into_owned()
}

/// Shipping crates whose CHANGELOG has no heading for their own version.
fn missing_changelog_entries(root: &Path, versions: &[(&str, String)]) -> Result<Vec<String>> {
    let mut missing = Vec::new();
    for crate_name in SHIPPING {
        let path = root.join(crate_name).join("CHANGELOG.md");
        let version = version_of(versions, crate_name);
        let heading = Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(version))).unwrap();
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if !heading.is_match(&text) {
            missing.push(format!(
                "{crate_name}/CHANGELOG.md has no `## [{version}]` entry"
            ));
        }
    }
    Ok(missing)
}

/// The pack's module files, in the order `sorted(PACK_DIR.glob("*.json"))` gave
/// them, so the drift report lists them the same way it always has.
fn pack_modules(root: &Path) -> Result<Vec<PathBuf>> {
    let dir = root.join(PACK_DIR);
    let mut modules: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| path.file_name().is_some_and(|name| name != PACK_MANIFEST))
        .collect();
    modules.sort();
    Ok(modules)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = paths::repo_root()?;
    let re = Rewrites::new();
    let versions = crate_versions(&root)?;
    let mut stale: Vec<String> = Vec::new();

    let mut targets: Vec<(String, PathBuf, Option<&str>)> = DOCS
        .iter()
        .map(|name| ((*name).to_string(), root.join(name), None))
        .collect();
    targets.extend(SHIPPING.iter().map(|c| {
        (
            format!("{c}/README.md"),
            root.join(c).join("README.md"),
            Some(*c),
        )
    }));

    for (name, path, crate_name) in targets {
        let before = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let after = match crate_name {
            Some(c) => sync_readme(&before, c, &versions, &re),
            None => sync(&before, &versions, &re),
        };
        if before == after {
            continue;
        }
        stale.push(name);
        if !cli.check {
            std::fs::write(&path, after)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }
    }

    let pack = pack_version(&root)?;
    for path in pack_modules(&root)? {
        let before = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let after = sync_pack_module(&before, &pack, &re);
        if before == after {
            continue;
        }
        stale.push(
            path.strip_prefix(&root)
                .unwrap_or(&path)
                .display()
                .to_string(),
        );
        if !cli.check {
            std::fs::write(&path, after)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }
    }

    let mut listed = versions
        .iter()
        .map(|(c, v)| format!("{c} {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    listed += &format!(", DMM pack {pack}");

    // Reported whether or not we are in --check mode: a missing entry cannot be
    // written for you, so `just sync-versions` must not exit 0 pretending it is
    // all done.
    let missing = missing_changelog_entries(&root, &versions)?;
    if !missing.is_empty() {
        for line in &missing {
            eprintln!("sync-versions: {line}");
        }
        eprintln!(
            "sync-versions: write the entry in the same commit as the bump (VERSIONING.md step 3)"
        );
    }

    if cli.check && !stale.is_empty() {
        eprintln!("sync-versions: out of date: {}", stale.join(", "));
        eprintln!("sync-versions: the sources of truth say {listed}");
        eprintln!("sync-versions: run `just sync-versions` and commit the result");
        std::process::exit(1);
    }
    if !stale.is_empty() {
        println!("sync-versions: updated {} ({listed})", stale.join(", "));
    } else if missing.is_empty() {
        println!("sync-versions: already in sync ({listed})");
    }
    std::process::exit(i32::from(!missing.is_empty()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn versions() -> Vec<(&'static str, String)> {
        CRATES
            .iter()
            .enumerate()
            .map(|(i, c)| (*c, format!("1.{i}.0")))
            .collect()
    }

    #[test]
    fn first_section_stops_at_the_next_header() {
        let manifest = "# a comment\n[package]\nname = \"x\"\nversion = \"0.1.0\"\n\n[dependencies]\nanyhow = { version = \"9.9.9\" }\n";
        let package = first_section(manifest);
        assert!(package.starts_with("package]"));
        assert!(package.contains("version = \"0.1.0\""));
        assert!(!package.contains("9.9.9"));
    }

    #[test]
    fn table_cell_keeps_its_padding() {
        let re = Rewrites::new();
        let line = "| `desert-core` | 0.0.1    | nothing | never |\n";
        let out = sync(line, &versions(), &re);
        assert_eq!(out, "| `desert-core` | 1.5.0    | nothing | never |\n");
    }

    #[test]
    fn longest_crate_name_wins_and_ties_go_to_the_first() {
        let re = Rewrites::new();
        // Both desert-gatherer and desert-dispatch are 15 characters; CRATES
        // lists the gatherer first, so its version is the one written.
        let line = "| desert-gatherer + desert-dispatch | 0.0.1 |\n";
        assert_eq!(
            sync(line, &versions(), &re),
            "| desert-gatherer + desert-dispatch | 1.2.0 |\n"
        );
    }

    #[test]
    fn tag_examples_are_rewritten_anywhere_not_just_in_tables() {
        let re = Rewrites::new();
        let text = "git push origin desert-tooling-v0.1.0\n";
        assert_eq!(
            sync(text, &versions(), &re),
            "git push origin desert-tooling-v1.0.0\n"
        );
    }

    #[test]
    fn a_dmm_tag_is_not_a_gatherer_tag() {
        let re = Rewrites::new();
        // desert-gatherer is a prefix of desert-gatherer-dmm, but the pack is
        // versioned x.y and is not a crate: the semver pattern must not reach
        // into its tag and leave `desert-gatherer-dmm-v1.2.0` behind.
        let text = "desert-gatherer-dmm-v1.1\n";
        assert_eq!(sync(text, &versions(), &re), text);
    }

    #[test]
    fn readme_version_line_is_rewritten_once() {
        let re = Rewrites::new();
        let text = "# x\n\n**Version 0.0.1**, for the game.\n\n**Version 0.0.1** again.\n";
        let out = sync_readme(text, "desert-tooling", &versions(), &re);
        assert_eq!(
            out,
            "# x\n\n**Version 1.0.0**, for the game.\n\n**Version 0.0.1** again.\n"
        );
    }

    #[test]
    fn both_versions_in_a_pack_module_follow_the_manifest() {
        let re = Rewrites::new();
        let text = "{\n  \"version\": \"1.0\",\n  \"modinfo\": { \"version\": \"1.0\" }\n}\n";
        assert_eq!(
            sync_pack_module(text, "1.1", &re),
            "{\n  \"version\": \"1.1\",\n  \"modinfo\": { \"version\": \"1.1\" }\n}\n"
        );
    }

    #[test]
    fn a_non_numeric_version_value_is_left_alone() {
        let re = Rewrites::new();
        let text = "{ \"version\": \"unreleased\" }\n";
        assert_eq!(sync_pack_module(text, "1.1", &re), text);
    }

    #[test]
    fn a_file_with_no_trailing_newline_survives_the_round_trip() {
        let re = Rewrites::new();
        let text = "| desert-core | 1.5.0 |";
        assert_eq!(sync(text, &versions(), &re), text);
    }
}
