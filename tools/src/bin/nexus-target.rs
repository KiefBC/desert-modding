//! Turn a release tag into the Nexus Mods file it publishes to.
//!
//! ```text
//! nexus-target desert-looter-v0.1.2
//! ```
//!
//! Prints `key=value` lines for `$GITHUB_OUTPUT`; the `nexus` job in
//! .github/workflows/release.yml appends them and reads them back as step
//! outputs:
//!
//! ```text
//! publish         "true" or "false" - false means this package has no Nexus
//!                 target configured, and the job skips the rest of its steps
//! version         0.1.2, from the tag
//! zip             DesertLooter-0.1.2.zip, the release's one package zip
//! display_name    what the file is called on the Nexus page
//! category        main / optional / miscellaneous, where the file sits
//! update_mod_version  whether this file's version becomes the page's version
//! changelog       whether to append the CHANGELOG entry to the page's changelog
//! game_domain     crimsondesert
//! game_scoped_id  the number in the mod page URL
//! file_id         the Nexus file (update group) to add a version to
//! ```
//!
//! The mapping lives in `tools/nexus-targets.json`, which says where the two
//! IDs come from. Everything here is a lookup and a format check - no network
//! calls, so a misconfigured target fails in a second instead of half way
//! through an upload.
//!
//! The tag's version is NOT re-checked against `Cargo.toml` here: release.yml
//! already ran `release-notes`, which exits 1 on a mismatch, before anything
//! was built. This runs after that.
//!
//! Ported from `tools/nexus-target.py`, reading the same JSON unchanged. The
//! key names and the spelling of every value are what the workflow's `if:`
//! conditions and the upload action's inputs are written against, so stdout is
//! byte-for-byte what the Python printed.

use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;
use serde_json::Value;

use desert_tools::paths;

/// What the Nexus API accepts for a mod file version (openapi.yaml,
/// `CreateModFileVersionRequest`). Checked here so a bad name is a red step
/// before the upload rather than a 422 after the bytes are already in S3.
const VERSION_RE: &str = r"^[a-zA-Z0-9.-]{1,50}$";
const NAME_RE: &str = r"^[a-zA-Z0-9 _'().-]{1,50}$";
const CATEGORIES: [&str; 3] = ["main", "optional", "miscellaneous"];

/// The file name alone, because that is what the messages quote.
const TARGETS_NAME: &str = "nexus-targets.json";

#[derive(Parser)]
#[command(
    name = "nexus-target",
    about = "Turn a release tag into the Nexus Mods file it publishes to."
)]
struct Cli {
    tag: String,
}

/// Die the way the Python did: the message on stderr, status 1, no `Error:`
/// prefix. This runs as a workflow step whose log is the only place anyone
/// reads it.
fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

/// Python's `repr`, because the "names no package" message quoted its tag with
/// `{tag!r}` and that message is what a release engineer greps for.
fn py_repr(s: &str) -> String {
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

/// A JSON value as Python's f-string would have written it: a string without
/// its quotes, anything else as JSON. The two IDs are strings in the file
/// today, and this keeps a hand-edit that drops the quotes from silently
/// emitting `file_id="7934145"` with the quotes baked into the output.
fn plain(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_string)
}

/// JSON truthiness, Python's rules: absent, null, false, 0 and "" are all the
/// empty-`file_id` opt-out.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `desert-gatherer-dmm-v1.1` -> ("desert-gatherer-dmm", "1.1"). Longest prefix
/// wins, or `desert-gatherer-dmm-v1.1` would be read as the gatherer's tag with
/// a version of `dmm-v1.1`.
fn parse_tag<'a>(tag: &str, prefixes: &[&'a str]) -> (&'a str, String) {
    let mut by_length = prefixes.to_vec();
    by_length.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for prefix in by_length {
        if let Some(version) = tag.strip_prefix(&format!("{prefix}-v")) {
            return (prefix, version.to_string());
        }
    }
    fail(&format!(
        "nexus-target: tag {} names no package in {TARGETS_NAME}",
        py_repr(tag)
    ));
}

/// `str.format(version=...)` for the one field these templates use.
fn substitute_version(template: &str, version: &str) -> String {
    template.replace("{version}", version)
}

fn field<'a>(target: &'a Value, prefix: &str, key: &str) -> &'a Value {
    target.get(key).unwrap_or_else(|| {
        fail(&format!(
            "nexus-target: target {prefix} in {TARGETS_NAME} has no {key}"
        ))
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = paths::repo_root()?;
    let targets_path: &Path = &root.join("tools").join(TARGETS_NAME);

    let text = std::fs::read_to_string(targets_path)
        .with_context(|| format!("cannot read {}", targets_path.display()))?;
    let config: Value = serde_json::from_str(&text)
        .with_context(|| format!("cannot parse {}", targets_path.display()))?;
    let targets = config
        .get("targets")
        .and_then(Value::as_object)
        .with_context(|| format!("no `targets` object in {}", targets_path.display()))?;
    let domain = config
        .get("game_domain")
        .map(plain)
        .with_context(|| format!("no `game_domain` in {}", targets_path.display()))?;

    // `serde_json` is built with preserve_order, so this is the order the file
    // lists its targets in - which is what the "N targets set" message below
    // reports, and what the Python's dict iteration gave.
    let prefixes: Vec<&str> = targets.keys().map(String::as_str).collect();
    let (prefix, version) = parse_tag(&cli.tag, &prefixes);
    let target = &targets[prefix];

    // One mod page holds all four files, so exactly one of them may own the
    // page's version field. Catch a second claimant here rather than watching
    // two releases overwrite each other on the site.
    let owners: Vec<&str> = targets
        .iter()
        .filter(|(_, t)| truthy(t.get("update_mod_version")))
        .map(|(name, _)| name.as_str())
        .collect();
    if owners.len() > 1 {
        fail(&format!(
            "nexus-target: {} targets set update_mod_version ({}); at most one may.",
            owners.len(),
            owners.join(", ")
        ));
    }

    // An unconfigured package is not an error: the tag still cut a GitHub
    // release, and this is how a package opts out of Nexus entirely.
    if !truthy(target.get("file_id")) || !truthy(target.get("game_scoped_id")) {
        println!("publish=false");
        // stderr, not stdout: stdout is redirected into $GITHUB_OUTPUT, and a
        // workflow command has to reach the log to be seen.
        eprintln!(
            "::notice::{prefix} has no Nexus target in {TARGETS_NAME}, so nothing was published to Nexus."
        );
        return Ok(());
    }

    let zip_name = substitute_version(&plain(field(target, prefix, "zip")), &version);
    let display_name = substitute_version(&plain(field(target, prefix, "display_name")), &version);

    let category = plain(field(target, prefix, "category"));
    if !CATEGORIES.contains(&category.as_str()) {
        fail(&format!(
            "nexus-target: category {} is not one of {}",
            py_repr(&category),
            CATEGORIES.join(", ")
        ));
    }
    if !Regex::new(VERSION_RE).unwrap().is_match(&version) {
        fail(&format!(
            "nexus-target: version {} is not accepted by Nexus ({VERSION_RE})",
            py_repr(&version)
        ));
    }
    if !Regex::new(NAME_RE).unwrap().is_match(&display_name) {
        fail(&format!(
            "nexus-target: display name {} is not accepted by Nexus ({NAME_RE})",
            py_repr(&display_name)
        ));
    }

    for (key, value) in [
        ("publish", "true".to_string()),
        ("version", version),
        ("zip", zip_name),
        ("display_name", display_name),
        ("category", category),
        (
            "update_mod_version",
            truthy(target.get("update_mod_version")).to_string(),
        ),
        ("changelog", truthy(target.get("changelog")).to_string()),
        ("game_domain", domain),
        ("game_scoped_id", plain(field(target, prefix, "game_scoped_id"))),
        ("file_id", plain(field(target, prefix, "file_id"))),
    ] {
        println!("{key}={value}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_prefix_wins() {
        let prefixes = ["desert-tooling", "desert-gatherer", "desert-gatherer-dmm"];
        assert_eq!(
            parse_tag("desert-gatherer-dmm-v1.1", &prefixes),
            ("desert-gatherer-dmm", "1.1".to_string())
        );
        assert_eq!(
            parse_tag("desert-tooling-v0.6.0", &prefixes),
            ("desert-tooling", "0.6.0".to_string())
        );
    }

    #[test]
    fn an_empty_file_id_is_the_opt_out_and_not_a_zero() {
        assert!(!truthy(Some(&Value::String(String::new()))));
        assert!(!truthy(None));
        assert!(!truthy(Some(&Value::Null)));
        assert!(truthy(Some(&Value::String("7934145".to_string()))));
        assert!(truthy(Some(&Value::Bool(true))));
        assert!(!truthy(Some(&Value::Bool(false))));
    }

    #[test]
    fn the_version_and_name_patterns_reject_what_nexus_rejects() {
        let version = Regex::new(VERSION_RE).unwrap();
        assert!(version.is_match("0.6.0"));
        assert!(!version.is_match("0.6.0 beta"));
        assert!(!version.is_match("0.6.0\n"));
        let name = Regex::new(NAME_RE).unwrap();
        assert!(name.is_match("Desert Tooling (ASI)"));
        assert!(!name.is_match("Desert Tooling [ASI]"));
        assert!(!name.is_match(&"x".repeat(51)));
    }

    #[test]
    fn only_the_version_field_is_substituted() {
        assert_eq!(
            substitute_version("DesertTooling-{version}.zip", "0.6.0"),
            "DesertTooling-0.6.0.zip"
        );
        assert_eq!(
            substitute_version("Desert Tooling (ASI)", "0.6.0"),
            "Desert Tooling (ASI)"
        );
    }
}
