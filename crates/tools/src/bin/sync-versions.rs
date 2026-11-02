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
//!   2. any `<crate>-v<version>` string anywhere -> the crate's version, and
//!      any `desert-gatherer-dmm-v<x.y>` string -> the DMM pack's version from
//!      `dmm_pack.json` (digits only, so a `<x.y>` placeholder is left alone)
//!
//! and in the shipping crate's own `README.md`, additionally:
//!
//!   3. the `**Version <version>**` line near the top -> that crate's version
//!
//! Only one crate ships now: desert-tooling, which builds `DesertTooling.asi`.
//! desert-harvest, desert-overlay, desert-dispatch and desert-core are
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
//! The same goes for the game build the plugin is verified on, which three
//! files state and none of them can be generated from the others:
//! `paths::CALIBRATED_BUILD`, the `Steam build **N**` line in the shipping
//! crate's README, and the `build` of the `desert-tooling` card in
//! `mod-status.toml` (the live status card on the Nexus page, which flips to
//! "untested" when Steam's build moves past it). All three must agree, so a
//! re-target that forgets the card fails `just check-versions`.
//!
//! Ported from the retired `sync-versions.py`. Its output is consumed by `just
//! check-versions` and by both GitHub workflows, so the exit codes and every
//! line of the drift report are reproduced exactly; only the `--help` text is
//! new. One thing the port does NOT reproduce is the Python's newline
//! translation - see `sync_pack_module`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use regex::{Captures, Regex};

use desert_tools::paths;

const CRATES: [&str; 5] = [
    "desert-tooling",
    "desert-harvest",
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
/// The pack's release tag prefix: `desert-gatherer-dmm-v<x.y>`.
const PACK_TAG: &str = "desert-gatherer-dmm";

/// The mod-status card config, and the `[[mod]]` in it that tracks the plugin.
const STATUS_CONFIG: &str = "mod-status.toml";
const STATUS_KEY: &str = "desert-tooling";

const SEMVER: &str = r"\d+\.\d+\.\d+";
/// The pack's x.y version inside its tag. Digits only, so the `<x.y>` and
/// `<version>` placeholders VERSIONING.md writes are never matched.
const PACK_VERSION: &str = r"\d+\.\d+";
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
    /// The DMM pack's `desert-gatherer-dmm-v<x.y>` tag.
    pack_tag: Regex,
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
            pack_tag: Regex::new(&format!("{}-v{PACK_VERSION}", regex::escape(PACK_TAG))).unwrap(),
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
        let manifest = paths::crate_dir_in(root, name).join("Cargo.toml");
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

/// Rebuild a `<name>-v<old>` match around a new version, cutting at the last
/// `-v` rather than expanding `$`, which a version string will never contain
/// but a name might grow.
fn retag(matched: &str, version: &str) -> String {
    let cut = matched.rfind("-v").expect("the pattern contains -v") + 2;
    format!("{}{version}", &matched[..cut])
}

fn sync(text: &str, versions: &[(&str, String)], pack: &str, re: &Rewrites) -> String {
    let mut out = String::with_capacity(text.len());
    for raw in text.split_inclusive('\n') {
        let mut line = raw.to_string();

        // Longest name wins: "desert-harvest" is not a substring of another
        // crate today, but a future "desert-harvest-cli" would be ambiguous.
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
                .replace_all(&line, |caps: &Captures| retag(&caps[0], version))
                .into_owned();
            line = replacement;
        }
        line = re
            .pack_tag
            .replace_all(&line, |caps: &Captures| retag(&caps[0], pack))
            .into_owned();
        out.push_str(&line);
    }
    out
}

/// A shipping crate's README: the general rules plus its own Version line.
fn sync_readme(
    text: &str,
    crate_name: &str,
    versions: &[(&str, String)],
    pack: &str,
    re: &Rewrites,
) -> String {
    let text = sync(text, versions, pack, re);
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
        let path = paths::crate_dir_in(root, crate_name).join("CHANGELOG.md");
        let version = version_of(versions, crate_name);
        let heading = Regex::new(&format!(r"(?m)^## \[{}\]", regex::escape(version))).unwrap();
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if !heading.is_match(&text) {
            missing.push(format!(
                "{}/{crate_name}/CHANGELOG.md has no `## [{version}]` entry",
                paths::CRATES_DIR
            ));
        }
    }
    Ok(missing)
}

/// The `build = "N"` of the `[[mod]]` table whose `key` is `key`, read line by
/// line rather than through a TOML parser: the file is ours and this is the
/// one value needed from it.
fn status_card_build(text: &str, key: &str) -> Option<String> {
    let value = |line: &str, name: &str| -> Option<String> {
        let rest = line.trim().strip_prefix(name)?.trim_start().strip_prefix('=')?;
        let rest = rest.trim_start().strip_prefix('"')?;
        rest.split_once('"').map(|(v, _)| v.to_string())
    };
    for table in text.split("[[mod]]").skip(1) {
        // A later [section] ends the table.
        let body = table.split("\n[").next().unwrap_or(table);
        if body.lines().any(|l| value(l, "key").as_deref() == Some(key)) {
            return body.lines().find_map(|l| value(l, "build"));
        }
    }
    None
}

/// The `Steam build **N**` line in a shipping crate's README.
fn readme_build(text: &str) -> Option<String> {
    let re = Regex::new(r"Steam build \*\*(\d+)\*\*").unwrap();
    re.captures(text).map(|c| c[1].to_string())
}

/// Disagreements between the three places that name the verified game build.
fn build_drift(root: &Path) -> Result<Vec<String>> {
    let want = paths::CALIBRATED_BUILD;
    let mut drift = Vec::new();

    let config = root.join(STATUS_CONFIG);
    let text = std::fs::read_to_string(&config)
        .with_context(|| format!("cannot read {}", config.display()))?;
    match status_card_build(&text, STATUS_KEY) {
        Some(b) if b == want => {}
        Some(b) => drift.push(format!(
            "{STATUS_CONFIG}: the `{STATUS_KEY}` card says build {b}, paths::CALIBRATED_BUILD is {want}"
        )),
        None => drift.push(format!(
            "{STATUS_CONFIG}: no [[mod]] with key = \"{STATUS_KEY}\" and a build"
        )),
    }

    for crate_name in SHIPPING {
        let path = paths::crate_dir_in(root, crate_name).join("README.md");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let name = format!("{}/{crate_name}/README.md", paths::CRATES_DIR);
        match readme_build(&text) {
            Some(b) if b == want => {}
            Some(b) => drift.push(format!(
                "{name}: says Steam build {b}, paths::CALIBRATED_BUILD is {want}"
            )),
            None => drift.push(format!("{name}: no `Steam build **N**` line")),
        }
    }
    Ok(drift)
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
    // Read before the docs loop: the docs carry the pack's tag as well as
    // the module files carrying its version.
    let pack = pack_version(&root)?;
    let mut stale: Vec<String> = Vec::new();

    let mut targets: Vec<(String, PathBuf, Option<&str>)> = DOCS
        .iter()
        .map(|name| ((*name).to_string(), root.join(name), None))
        .collect();
    targets.extend(SHIPPING.iter().map(|c| {
        (
            format!("{}/{c}/README.md", paths::CRATES_DIR),
            paths::crate_dir_in(&root, c).join("README.md"),
            Some(*c),
        )
    }));

    for (name, path, crate_name) in targets {
        let before = std::fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let after = match crate_name {
            Some(c) => sync_readme(&before, c, &versions, &pack, &re),
            None => sync(&before, &versions, &pack, &re),
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

    // Checked, never written, like the CHANGELOG: which build the plugin was
    // verified on is a fact only a human can state.
    let drift = build_drift(&root)?;
    if !drift.is_empty() {
        for line in &drift {
            eprintln!("sync-versions: {line}");
        }
        eprintln!(
            "sync-versions: the verified game build must agree everywhere; update {STATUS_CONFIG}'s build and date in the same commit as the README build line"
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
    } else if missing.is_empty() && drift.is_empty() {
        println!("sync-versions: already in sync ({listed})");
    }
    std::process::exit(i32::from(!missing.is_empty() || !drift.is_empty()));
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
        let out = sync(line, &versions(), "1.1", &re);
        assert_eq!(out, "| `desert-core` | 1.4.0    | nothing | never |\n");
    }

    #[test]
    fn longest_crate_name_wins_and_ties_go_to_the_first() {
        let re = Rewrites::new();
        // Both desert-harvest and desert-overlay are 14 characters; CRATES
        // lists the harvest crate first, so its version is the one written.
        let line = "| desert-overlay + desert-harvest | 0.0.1 |\n";
        assert_eq!(
            sync(line, &versions(), "1.1", &re),
            "| desert-overlay + desert-harvest | 1.1.0 |\n"
        );
    }

    #[test]
    fn tag_examples_are_rewritten_anywhere_not_just_in_tables() {
        let re = Rewrites::new();
        let text = "git push origin desert-tooling-v0.1.0\n";
        assert_eq!(
            sync(text, &versions(), "1.1", &re),
            "git push origin desert-tooling-v1.0.0\n"
        );
    }

    #[test]
    fn a_dmm_tag_is_not_a_crate_tag() {
        let re = Rewrites::new();
        // The pack is versioned x.y and is not a crate: no crate's semver
        // pattern may reach into its tag and leave `desert-gatherer-dmm-v1.2.0`
        // behind. (It used to share a prefix with the desert-gatherer crate,
        // now merged into desert-harvest; the tag must still pass untouched.)
        let text = "desert-gatherer-dmm-v1.1\n";
        assert_eq!(sync(text, &versions(), "1.1", &re), text);
    }

    #[test]
    fn a_dmm_tag_follows_the_pack_and_a_placeholder_does_not() {
        let re = Rewrites::new();
        let text = "[desert-gatherer-dmm-v1.0](x/tag/desert-gatherer-dmm-v1.0)\n\
                    tagged `desert-gatherer-dmm-v<x.y>`\n";
        assert_eq!(
            sync(text, &versions(), "1.3", &re),
            "[desert-gatherer-dmm-v1.3](x/tag/desert-gatherer-dmm-v1.3)\n\
             tagged `desert-gatherer-dmm-v<x.y>`\n"
        );
    }

    #[test]
    fn readme_version_line_is_rewritten_once() {
        let re = Rewrites::new();
        let text = "# x\n\n**Version 0.0.1**, for the game.\n\n**Version 0.0.1** again.\n";
        let out = sync_readme(text, "desert-tooling", &versions(), "1.1", &re);
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
    fn status_card_build_reads_the_right_mod() {
        let text = "[source]\nkind = \"steam\"\n\n\
                    [[mod]]\nkey = \"other\"\nbuild = \"1\"\n\n\
                    [[mod]]\nkey   = \"desert-tooling\"  # comment\n\
                    label = \"Desert Tooling\"\nbuild = \"26361232\"   # verified\n\n\
                    [render]\nbuild = \"9\"\n";
        assert_eq!(
            status_card_build(text, "desert-tooling").as_deref(),
            Some("26361232")
        );
        assert_eq!(status_card_build(text, "other").as_deref(), Some("1"));
        assert_eq!(status_card_build(text, "missing"), None);
        // A table without a build does not borrow the next section's.
        let no_build = "[[mod]]\nkey = \"desert-tooling\"\n[render]\nbuild = \"9\"\n";
        assert_eq!(status_card_build(no_build, "desert-tooling"), None);
    }

    #[test]
    fn readme_build_line() {
        let text = "**Version 0.10.0**, for Crimson Desert Enhanced, Steam build **26361232**.\n";
        assert_eq!(readme_build(text).as_deref(), Some("26361232"));
        assert_eq!(readme_build("Steam build 26361232"), None);
    }

    #[test]
    fn the_status_card_names_the_calibrated_build() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        assert_eq!(build_drift(&root).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn a_file_with_no_trailing_newline_survives_the_round_trip() {
        let re = Rewrites::new();
        let text = "| desert-core | 1.4.0 |";
        assert_eq!(sync(text, &versions(), "1.1", &re), text);
    }
}
