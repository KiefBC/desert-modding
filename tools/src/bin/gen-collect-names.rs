//! Regenerate `desert-core/src/collect.rs`.
//!
//! Usage: `gen-collect-names [pack-dir]`
//! Default pack dir: `desert-gatherer-dmm/`, the copy of the pack kept in this
//! repo. Unlike the python this replaces it does not have to be run from the
//! workspace root: every default path hangs off `desert_tools::paths::repo_root`.
//!
//! Two sources, one output. The generator owns the whole file - enum, rows, both
//! lookups and every test - so `collect.rs` can be regenerated straight over
//! itself; `render` says what that used to drop.
//!
//! 1. The DMM pack (`desert-gatherer-dmm/* - 2X.json`). Its four modules give
//!    the four families the pack itself multiplies: Foraging, Logging, Mining,
//!    Ore. Each `changes` entry already carries `entry` (record name) and
//!    `record_key`.
//!
//! 2. `tools/extra-families.json` - records the DMM pack has no module for,
//!    added to a family by hand. Today that is the water well
//!    `gimmick_well_0001_parts01`, which joins `Foraging` (water drawn from a
//!    well is gathered out of the world like everything else there), and the
//!    three placed money props, which are the `Money` family on their own. An
//!    entry may name an existing DMM family, in which case its records extend
//!    that family and its note is appended to the variant's doc comment, or a
//!    new one, in which case the generator emits the variant. Either way a
//!    hand-added row in `collect.rs` dies on the next regenerate; this input is
//!    how the generator owns it. `spec` documents its format and the reasons.

// `src/bin/<name>.rs` is a crate root but NOT a module directory root: rustc
// looks for `mod spec` at `src/bin/spec.rs`, which would be a second binary.
// `#[path]` puts the parts in `src/bin/gen-collect-names/` where they belong,
// without turning this into a `src/bin/gen-collect-names/main.rs` directory
// target that the other tools here do not use.
#[path = "gen-collect-names/render.rs"]
mod render;
#[path = "gen-collect-names/spec.rs"]
mod spec;
#[path = "gen-collect-names/table.rs"]
mod table;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;
use desert_tools::paths;
use serde_json::Value;

use spec::q;

/// The four DMM modules, in the order their families are emitted.
const DMM_FAMILIES: &[(&str, &str)] = &[
    ("Foraging", "Foraging"),
    ("Logging", "Logging"),
    ("Mining", "Mining"),
    ("Ore Nodes", "Ore"),
];

#[derive(Parser)]
#[command(about = "Regenerate desert-core/src/collect.rs from the DMM pack and tools/extra-families.json")]
struct Cli {
    /// The DMM pack directory [default: <repo>/desert-gatherer-dmm]
    pack_dir: Option<PathBuf>,
    /// The hand-added records [default: <repo>/tools/extra-families.json]
    #[arg(long, value_name = "FILE")]
    extras: Option<PathBuf>,
    /// Where to write [default: <repo>/desert-core/src/collect.rs]
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// DMM's clean gimmickinfo table body; pointing this at a path that does
    /// not exist is how the unverified branch gets exercised deliberately
    /// [default: $CD_DMM_TABLE]
    #[arg(long, value_name = "FILE")]
    table: Option<PathBuf>,
}

/// The rows of `COLLECT_RECORDS`: record name -> (key, family).
///
/// Insertion-ordered on purpose. The emitted order is by lowercased name, and
/// the sort that produces it is stable, so two names that differ only in case
/// would come out in the order they were added - DMM pack first, then the
/// extras in `extra-families.json` order.
pub struct Rows {
    order: Vec<(String, u32, String)>,
    index: HashMap<String, usize>,
}

impl Rows {
    fn new() -> Self {
        Rows { order: Vec::new(), index: HashMap::new() }
    }

    /// Insert unless the name is already a row; the first module to claim a
    /// record name wins, which is what the python's `setdefault` did.
    fn set_default(&mut self, name: &str, key: u32, family: &str) {
        if self.index.contains_key(name) {
            return;
        }
        self.index.insert(name.to_string(), self.order.len());
        self.order.push((name.to_string(), key, family.to_string()));
    }

    fn family_of(&self, name: &str) -> Option<&str> {
        self.index.get(name).map(|&i| self.order[i].2.as_str())
    }

    /// The name already using `key`, if any. Two families for one key would
    /// make `family_by_key` answer whichever row it hit first.
    fn name_using_key(&self, key: u32) -> Option<&str> {
        self.order.iter().find(|(_, k, _)| *k == key).map(|(n, _, _)| n.as_str())
    }

    /// `count`, not `len`: clippy asks a type with `len` for an `is_empty`,
    /// and a `Rows` that is empty is a bug the generator has already died on.
    pub fn count(&self) -> usize {
        self.order.len()
    }

    pub fn count_in_family(&self, family: &str) -> usize {
        self.order.iter().filter(|(_, _, f)| f == family).count()
    }

    /// The rows in emitted order: by lowercased name, stably.
    pub fn sorted_by_name(&self) -> Vec<(&str, u32, &str)> {
        let mut v: Vec<(&str, u32, &str)> =
            self.order.iter().map(|(n, k, f)| (n.as_str(), *k, f.as_str())).collect();
        v.sort_by_key(|(n, _, _)| n.to_lowercase());
        v
    }
}

/// Source 1: the DMM pack. Only the `2X` module of each family is read; the 5X
/// and 10X modules cover the same records.
fn read_dmm(base: &Path) -> Result<Rows> {
    let fam: HashMap<&str, &str> = DMM_FAMILIES.iter().copied().collect();
    let mut files: Vec<PathBuf> = fs::read_dir(base)
        .with_context(|| format!("cannot read the pack directory {}", base.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(" - 2X.json"))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("no '* - 2X.json' modules under {}", q(&base.display().to_string()));
    }
    let mut rows = Rows::new();
    for path in &files {
        let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        let cat = stem.split(" - ").next().unwrap_or_default();
        let Some(family) = fam.get(cat) else {
            bail!("{}: unknown DMM module {}", path.display(), q(cat));
        };
        let text = fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let d: Value = serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid json", path.display()))?;
        let Some(groups) = d.get("patches").and_then(Value::as_array) else {
            bail!("{}: no `patches` array; this is not a DMM module", path.display());
        };
        for group in groups {
            let Some(changes) = group.get("changes").and_then(Value::as_array) else {
                bail!("{}: a patch group with no `changes`", path.display());
            };
            for c in changes {
                let (Some(entry), Some(key)) = (
                    c.get("entry").and_then(Value::as_str),
                    c.get("record_key").and_then(Value::as_u64),
                ) else {
                    bail!("{}: a change with no `entry`/`record_key`", path.display());
                };
                rows.set_default(entry, key as u32, family);
            }
        }
    }
    Ok(rows)
}

fn run(cli: &Cli) -> Result<()> {
    let root = paths::repo_root()?;
    let pack = cli.pack_dir.clone().unwrap_or_else(|| root.join("desert-gatherer-dmm"));
    let extras_path = cli.extras.clone().unwrap_or_else(|| root.join("tools/extra-families.json"));
    let out_path = cli.out.clone().unwrap_or_else(|| root.join("desert-core/src/collect.rs"));
    let table_path = cli.table.clone().unwrap_or_else(paths::dmm_table);

    let mut rows = read_dmm(&pack)?;
    let dmm_count = rows.count();

    // The path is quoted into the messages the way the python quoted it, so
    // that a diagnostic from either generator reads the same.
    let extras_name = extras_path.display().to_string();
    if !extras_path.exists() {
        bail!("{extras_name} is missing; it is the source of every non-DMM row");
    }
    let extras_text = fs::read_to_string(&extras_path)
        .with_context(|| format!("cannot read {extras_name}"))?;
    let spec = spec::read_extras(&extras_name, &extras_text)?;

    // Verification comes before the file is touched: a mismatch must leave
    // `collect.rs` exactly as it was, not half-rewritten from bad rows.
    let table_name = table_path.display().to_string();
    let banner = match fs::read(&table_path) {
        Ok(buf) => table::verify(&spec, &buf, &table_name)?,
        Err(_) => table::absent_banner(&table_name),
    };

    for family in &spec {
        // A record cannot be both a DMM row and a record the pack was measured
        // not to reach: one of the two statements is false.
        for rec in &family.records_not_enabled {
            if rows.family_of(&rec.name).is_some() || rows.name_using_key(rec.key).is_some() {
                bail!(
                    "{}: {} (key {}) is listed as not enabled but is a DMM pack \
                     record. One of the two is wrong; decide which.",
                    family.name,
                    q(&rec.name),
                    rec.key
                );
            }
        }
        for rec in &family.records {
            if let Some(existing) = rows.family_of(&rec.name) {
                bail!(
                    "{}: record {} (key {}) is already a {existing} record from the \
                     DMM pack. A record has one family; decide which.",
                    family.name,
                    q(&rec.name),
                    rec.key
                );
            }
            if let Some(clash) = rows.name_using_key(rec.key) {
                bail!("{}: key {} is already used by {}", family.name, rec.key, q(clash));
            }
            rows.set_default(&rec.name, rec.key, &family.name);
        }
    }

    let mut order: Vec<String> = DMM_FAMILIES.iter().map(|(_, rust)| (*rust).to_string()).collect();
    let mut extra: Vec<String> = spec
        .iter()
        .map(|f| f.name.clone())
        .filter(|f| !order.contains(f))
        .collect();
    extra.sort();
    order.extend(extra);

    let text = render::collect_rs(&rows, &order, &spec);
    if let Some(dir) = out_path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    fs::write(&out_path, text)
        .with_context(|| format!("cannot write {}", out_path.display()))?;

    println!(
        "records: {} (DMM {dmm_count} + extras {})",
        rows.count(),
        rows.count() - dmm_count
    );
    for family in &spec {
        println!(
            "{}: +{} records, {} recorded as not enabled, {} candidate items deferred",
            family.name,
            family.records.len(),
            family.records_not_enabled.len(),
            family.candidates_not_enabled
        );
    }
    for line in banner {
        println!("{line}");
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Same prefix the python used, so a script grepping either of them
            // keeps working: `gen-collect-names: <what went wrong>`.
            eprintln!("gen-collect-names: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_keep_the_first_module_to_claim_a_name() {
        let mut rows = Rows::new();
        rows.set_default("a", 1, "Foraging");
        rows.set_default("a", 2, "Logging");
        assert_eq!(rows.family_of("a"), Some("Foraging"));
        assert_eq!(rows.count(), 1);
    }

    #[test]
    fn rows_sort_case_insensitively_and_stably() {
        let mut rows = Rows::new();
        rows.set_default("Zebra", 1, "Foraging");
        rows.set_default("apple", 2, "Logging");
        rows.set_default("Apple_2", 3, "Mining");
        let got: Vec<&str> = rows.sorted_by_name().into_iter().map(|(n, _, _)| n).collect();
        assert_eq!(got, vec!["apple", "Apple_2", "Zebra"]);
        assert_eq!(rows.count_in_family("Foraging"), 1);
    }

    #[test]
    fn a_key_can_only_belong_to_one_row() {
        let mut rows = Rows::new();
        rows.set_default("a", 7, "Foraging");
        assert_eq!(rows.name_using_key(7), Some("a"));
        assert_eq!(rows.name_using_key(8), None);
    }
}
