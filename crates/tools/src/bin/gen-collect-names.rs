//! Regenerate `crates/desert-core/src/collect.rs` from the game's own tables.
//!
//! Usage: `gen-collect-names [--tables DIR] [--strings FILE] [--extras FILE] [--out FILE]`
//!
//! Inputs, all read before anything is written:
//!
//! 1. Four static-info tables, `gimmickinfo`, `iteminfo`, `interactioninfo`
//!    and `knowledgeinfo`: out of the installed game's archives
//!    (`desert_tools::paz`, `paths::game_dir`) by default, or from a directory
//!    of extracted copies (`--tables`, what `just extract-tables` writes into
//!    `analysis/tables/`). The `gimmickinfo` body must reproduce
//!    `table::CALIBRATION` or nothing is written.
//! 2. The family rule, `classify::RULES`, applied to every record that pays
//!    at least one output block the plugin can multiply (`table::record_pays`).
//! 3. `crates/tools/extra-families.json`: the overrides, records the rule does
//!    not pick that belong to a family (and the `Money` family, which no rule
//!    makes), and `records_not_enabled`, records that are never rows. `spec`
//!    documents its format.
//! 4. The English record names, `analysis/strings/gimmick.eng.tsv` (`key`,
//!    `column`, `text`; column `0x200` is the name). Every row needs one.
//!
//! The generator owns the whole output file - enum, rows, both lookups and
//! every test - so `collect.rs` can be regenerated straight over itself;
//! `render` says what that used to drop. Every default path hangs off
//! `desert_tools::paths::repo_root`, never off the working directory.
//!
//! It prints the rows per family and what changed against the `collect.rs` it
//! is about to replace: every record added or removed, with the decoded fields
//! that put it there.

// `src/bin/<name>.rs` is a crate root but NOT a module directory root: rustc
// looks for `mod spec` at `src/bin/spec.rs`, which would be a second binary.
// `#[path]` puts the parts in `src/bin/gen-collect-names/` where they belong,
// without turning this into a `src/bin/gen-collect-names/main.rs` directory
// target that the other tools here do not use.
#[path = "gen-collect-names/classify.rs"]
mod classify;
#[path = "gen-collect-names/render.rs"]
mod render;
#[path = "gen-collect-names/spec.rs"]
mod spec;
#[path = "gen-collect-names/table.rs"]
mod table;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;
use desert_tools::paths;
use desert_tools::paz::Archive;
use desert_tools::records::{header_at, HeaderShape};
use desert_tools::staticinfo::{self, Table};

use classify::{Facts, Lookups};
use spec::{check_money_rule, py_ints, q, FamilySpec};
use table::Detector;

/// The tables read, in the order they are read.
const GIMMICK: &str = "gimmickinfo";
const ITEM: &str = "iteminfo";
const INTERACTION: &str = "interactioninfo";
const KNOWLEDGE: &str = "knowledgeinfo";

/// The column of `gimmick.eng.tsv` that holds a record's display name.
const NAME_COLUMN: &str = "0x200";

#[derive(Parser)]
#[command(about = "Regenerate crates/desert-core/src/collect.rs from the game's archive tables and crates/tools/extra-families.json")]
struct Cli {
    /// A directory of extracted tables (`<table>.staticinfobody` and
    /// `.staticinfoheader`, as `just extract-tables` writes) to read instead of
    /// the game's archives [default: the archives under $CD_GAME_DIR, else
    /// the parent of $CD_BIN64]
    #[arg(long, value_name = "DIR")]
    tables: Option<PathBuf>,
    /// The English record names [default: <repo>/analysis/strings/gimmick.eng.tsv]
    #[arg(long, value_name = "FILE")]
    strings: Option<PathBuf>,
    /// The overrides [default: <repo>/crates/tools/extra-families.json]
    #[arg(long, value_name = "FILE")]
    extras: Option<PathBuf>,
    /// Where to write [default: <repo>/crates/desert-core/src/collect.rs]
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
}

/// The rows of `COLLECT_RECORDS`.
///
/// Insertion-ordered on purpose. The emitted order is by lowercased name, and
/// the sort that produces it is stable, so two names that differ only in case
/// come out in the order they were added: rule rows in table order, then the
/// overrides in `extra-families.json` order.
pub struct Rows {
    order: Vec<Row>,
}

/// One row: what is emitted, plus where it came from for the report.
#[derive(Clone)]
pub struct Row {
    pub name: String,
    pub key: u32,
    pub family: String,
    pub display: String,
    pub source: &'static str,
}

impl Rows {
    fn new() -> Self {
        Rows { order: Vec::new() }
    }

    fn push(&mut self, row: Row) {
        self.order.push(row);
    }

    /// `count`, not `len`: clippy asks a type with `len` for an `is_empty`,
    /// and a `Rows` that is empty is a bug the generator has already died on.
    pub fn count(&self) -> usize {
        self.order.len()
    }

    pub fn count_in_family(&self, family: &str) -> usize {
        self.order.iter().filter(|r| r.family == family).count()
    }

    /// The rows in emitted order: by lowercased name, stably.
    pub fn sorted_by_name(&self) -> Vec<&Row> {
        let mut v: Vec<&Row> = self.order.iter().collect();
        v.sort_by_key(|r| r.name.to_lowercase());
        v
    }
}

/// One `gimmickinfo` record, decoded.
struct Gimmick {
    key: u32,
    name: String,
    /// What the plugin's detector finds in it.
    items: Vec<u32>,
    lists: usize,
    /// Lists only the loose detector finds, which the plugin never edits.
    loose_only: usize,
    facts: Facts,
}

/// Where the four tables come from.
enum Source {
    Archive(Archive),
    Dir(PathBuf),
}

impl Source {
    fn read(&self, table: &str) -> Result<Table> {
        match self {
            Source::Archive(a) => staticinfo::read_from_archive(a, table)
                .with_context(|| format!("cannot read {table} out of the archives under {}", a.game_dir.display())),
            Source::Dir(d) => staticinfo::read_from_dir(d, table)
                .with_context(|| format!("cannot read {table} from {}", d.display())),
        }
    }

    fn describe(&self) -> String {
        match self {
            Source::Archive(a) => format!("the archives under {}", a.game_dir.display()),
            Source::Dir(d) => d.display().to_string(),
        }
    }
}

/// Every record of a table as `(key, bytes)`, keys as `u32`.
fn records_of<'a>(name: &str, t: &'a Table) -> Result<Vec<(u32, &'a [u8])>> {
    let recs = t.split().with_context(|| format!("{name}: the header does not split the body"))?;
    recs.iter()
        .map(|r| {
            let key = u32::try_from(r.key).with_context(|| format!("{name}: key {} is wider than u32", r.key))?;
            let bytes = t.body.get(r.offset..r.end).with_context(|| format!("{name}: record {} out of range", r.index))?;
            Ok((key, bytes))
        })
        .collect()
}

/// The three lookup tables the rule reads.
fn lookups(src: &Source) -> Result<Lookups> {
    let mut lk = Lookups::default();
    let t = src.read(INTERACTION)?;
    for (key, rec) in records_of(INTERACTION, &t)? {
        let Some(name) = classify::record_name(rec, key) else {
            bail!("{INTERACTION}: record {key} has no readable name");
        };
        lk.interactions.insert(key, name);
    }
    for name in [classify::COLLECT, classify::SCOOP, classify::PICK_UP] {
        let n = lk.interactions.values().filter(|v| *v == name).count();
        if n != 1 {
            bail!("{INTERACTION}: {} records named {}, expected exactly 1", n, q(name));
        }
    }
    let t = src.read(ITEM)?;
    for (key, rec) in records_of(ITEM, &t)? {
        if let Some(m) = classify::item_material(rec, key) {
            lk.materials.insert(key, m);
        }
    }
    let t = src.read(KNOWLEDGE)?;
    lk.knowledge = records_of(KNOWLEDGE, &t)?.into_iter().map(|(k, _)| k).collect();
    Ok(lk)
}

/// `key -> display name` from the strings tsv.
fn display_names(path: &Path) -> Result<HashMap<u32, String>> {
    let text = fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut out = HashMap::new();
    for line in text.lines().skip(1) {
        let mut f = line.splitn(3, '\t');
        let (Some(key), Some(col), Some(name)) = (f.next(), f.next(), f.next()) else { continue };
        if col != NAME_COLUMN {
            continue;
        }
        let Ok(key) = key.parse::<u32>() else { continue };
        if !name.trim().is_empty() {
            out.insert(key, name.to_string());
        }
    }
    Ok(out)
}

/// The rows of a `collect.rs` already on disk: `key -> (name, family)`.
/// Reads both the three-field and the four-field row.
fn previous_rows(path: &Path) -> BTreeMap<u32, (String, String)> {
    let Ok(text) = fs::read_to_string(path) else { return BTreeMap::new() };
    desert_tools::collect_rows::parse(&text).into_iter().map(|r| (r.key, (r.name, r.family))).collect()
}

fn run(cli: &Cli) -> Result<()> {
    let root = paths::repo_root()?;
    let extras_path = cli.extras.clone().unwrap_or_else(|| paths::crate_dir_in(&root, "tools").join("extra-families.json"));
    let out_path = cli.out.clone().unwrap_or_else(|| paths::crate_dir_in(&root, "desert-core").join("src").join("collect.rs"));
    let strings_path = cli.strings.clone().unwrap_or_else(|| root.join("analysis").join("strings").join("gimmick.eng.tsv"));

    let extras_name = extras_path.display().to_string();
    let extras_text = fs::read_to_string(&extras_path).with_context(|| format!("cannot read {extras_name}"))?;
    let spec = spec::read_extras(&extras_name, &extras_text)?;

    let src = match &cli.tables {
        Some(d) => Source::Dir(d.clone()),
        None => Source::Archive(Archive::open(&paths::game_dir())?),
    };

    // The body first, and its calibration before anything is decoded: an
    // uncalibrated body is a different table from the one the rule was
    // measured on, and nothing read out of it is to be trusted.
    let gimmick = src.read(GIMMICK)?;
    let walk = table::walk(&gimmick.body)?;
    let lk = lookups(&src)?;

    let mut all: Vec<Gimmick> = Vec::new();
    for (key, rec) in records_of(GIMMICK, &gimmick)? {
        let Some(h) = header_at(rec, 0, HeaderShape::NameFirst) else {
            bail!("{GIMMICK}: record {key} does not start with a NameFirst header");
        };
        if h.key != key {
            bail!("{GIMMICK}: record {} has key {} in its header and {key} in the table header", q(&h.name), h.key);
        }
        let (items, lists) = table::record_pays(rec, Detector::Shipped);
        let loose = table::record_pays(rec, Detector::Loose).1;
        let facts = Facts::of(rec, &items, &lk);
        if let Some(k) = facts.knowledge.filter(|&k| k != 0 && !items.is_empty()) {
            if !lk.knowledge.contains(&k) {
                bail!("{GIMMICK}: {} names knowledgeInfo {k}, which {KNOWLEDGE} does not have", q(&h.name));
            }
        }
        all.push(Gimmick { key, name: h.name, items, lists, loose_only: loose.saturating_sub(lists), facts });
    }
    let record_lists: usize = all.iter().map(|g| g.lists).sum();
    let loose_lists: usize = record_lists + all.iter().map(|g| g.loose_only).sum::<usize>();
    let by_key: HashMap<u32, &Gimmick> = all.iter().map(|g| (g.key, g)).collect();

    verify_overrides(&spec, &by_key)?;

    // The rule.
    let mut ruled: BTreeMap<u32, &'static str> = BTreeMap::new();
    let mut loose_only_matches: Vec<(&Gimmick, Vec<&'static str>)> = Vec::new();
    for g in &all {
        let fams = classify::families(&g.facts);
        if g.items.is_empty() {
            if !fams.is_empty() && g.loose_only > 0 {
                loose_only_matches.push((g, fams));
            }
            continue;
        }
        match fams.as_slice() {
            [] => {}
            [one] => {
                ruled.insert(g.key, one);
            }
            many => bail!(
                "{} (key {}) meets the rule of {} families at once ({}): {}. Decide which before \
                 regenerating.",
                q(&g.name),
                g.key,
                many.len(),
                many.join(", "),
                g.facts.describe()
            ),
        }
    }

    // Exclusions, then overrides.
    let excluded: HashSet<u32> = spec.iter().flat_map(|f| f.records_not_enabled.iter().map(|r| r.key)).collect();
    let mut excluded_hits: Vec<(&Gimmick, &str)> = Vec::new();
    for g in &all {
        if excluded.contains(&g.key) {
            if let Some(f) = ruled.remove(&g.key) {
                excluded_hits.push((g, f));
            }
        }
    }
    let mut overrides: Vec<(u32, String)> = Vec::new();
    for family in &spec {
        for rec in &family.records {
            if excluded.contains(&rec.key) {
                bail!(
                    "{}: {} (key {}) is an override and also in records_not_enabled. One of the two is wrong.",
                    family.name,
                    q(&rec.name),
                    rec.key
                );
            }
            if let Some(f) = ruled.get(&rec.key) {
                let g = by_key[&rec.key];
                if *f == family.name {
                    bail!(
                        "{}: {} (key {}) is an override, but the rule already makes it {f} ({}). \
                         Remove the override.",
                        family.name,
                        q(&rec.name),
                        rec.key,
                        g.facts.describe()
                    );
                }
                bail!(
                    "{}: {} (key {}) is an override, but the rule makes it {f} ({}). A record has \
                     one family; decide which.",
                    family.name,
                    q(&rec.name),
                    rec.key,
                    g.facts.describe()
                );
            }
            if let Some((_, f)) = overrides.iter().find(|(k, _)| *k == rec.key) {
                bail!("{}: {} (key {}) is already an override in {f}", family.name, q(&rec.name), rec.key);
            }
            overrides.push((rec.key, family.name.clone()));
        }
    }

    // Display names, and the rows.
    let names = display_names(&strings_path)?;
    let mut rows = Rows::new();
    let mut nameless: Vec<String> = Vec::new();
    let entries = all
        .iter()
        .filter_map(|g| ruled.get(&g.key).map(|f| (g, f.to_string(), "rule")))
        .chain(overrides.iter().map(|(k, f)| (by_key[k], f.clone(), "override")));
    for (g, family, source) in entries {
        check_money_rule(&format!("{family}: record {} {}", g.key, q(&g.name)), &g.name, &family, &g.items, "the table")?;
        let Some(display) = names.get(&g.key) else {
            nameless.push(format!("{} {} ({family})", g.key, q(&g.name)));
            continue;
        };
        rows.push(Row { name: g.name.clone(), key: g.key, family, display: display.clone(), source });
    }
    if !nameless.is_empty() {
        bail!(
            "{} row(s) have no display name (column {NAME_COLUMN}) in {}:\n  {}",
            nameless.len(),
            strings_path.display(),
            nameless.join("\n  ")
        );
    }

    let mut order: Vec<String> = classify::RULES.iter().map(|r| r.family.to_string()).collect();
    let mut extra: Vec<String> = spec.iter().map(|f| f.name.clone()).filter(|f| !order.contains(f)).collect();
    extra.sort();
    order.extend(extra);

    let previous = previous_rows(&out_path);
    let text = render::collect_rs(&rows, &order, &spec);
    if let Some(dir) = out_path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    }
    fs::write(&out_path, text).with_context(|| format!("cannot write {}", out_path.display()))?;

    // The report.
    println!("source: {}", src.describe());
    println!(
        "calibrated: {} records, {} lists, {} blocks (global walk, build {})",
        table::CALIBRATION.records,
        walk.lists.len(),
        table::CALIBRATION.blocks,
        table::CALIBRATION.build
    );
    println!(
        "per-record walk: {record_lists} lists the plugin multiplies ({} lists with the loose detector, \
         whose extra {} only `items --loose` sees); global walk: {}",
        loose_lists,
        loose_lists - record_lists,
        walk.lists.len()
    );
    let per_family: Vec<String> =
        order.iter().map(|f| format!("{f} {}", rows.count_in_family(f))).collect();
    println!("rows: {} = {}", rows.count(), per_family.join(", "));
    for family in &spec {
        println!(
            "{}: {} override(s), {} recorded as not enabled, {} candidate item(s) deferred",
            family.name,
            family.records.len(),
            family.records_not_enabled.len(),
            family.candidates_not_enabled
        );
    }
    for (g, f) in &excluded_hits {
        println!("excluded by records_not_enabled: {} {} (rule: {f}; {})", g.key, g.name, g.facts.describe());
    }
    for (g, fams) in &loose_only_matches {
        println!(
            "not a row, pays only blocks the plugin does not edit: {} {} (rule: {}; {} loose-only list(s))",
            g.key,
            g.name,
            fams.join("/"),
            g.loose_only
        );
    }
    let now: BTreeMap<u32, &Row> = rows.order.iter().map(|r| (r.key, r)).collect();
    let fields = |key: u32| {
        by_key.get(&key).map_or_else(
            || "not in the table".to_string(),
            |g| format!("{}; items {}", g.facts.describe(), py_ints(&g.items)),
        )
    };
    let added: Vec<&&Row> = now.iter().filter(|(k, _)| !previous.contains_key(k)).map(|(_, r)| r).collect();
    let removed: Vec<(&u32, &(String, String))> = previous.iter().filter(|(k, _)| !now.contains_key(k)).collect();
    let moved: Vec<(&&Row, &String)> = now
        .iter()
        .filter_map(|(k, r)| previous.get(k).filter(|(_, f)| *f != r.family).map(|(_, f)| (r, f)))
        .collect();
    println!(
        "against the previous {}: {} rows before, {} added, {} removed, {} moved family",
        out_path.display(),
        previous.len(),
        added.len(),
        removed.len(),
        moved.len()
    );
    for r in added {
        println!("  + {} {} {} by {} \"{}\": {}", r.key, r.name, r.family, r.source, r.display, fields(r.key));
    }
    for (k, (n, f)) in removed {
        println!("  - {k} {n} {f}: {}", fields(*k));
    }
    for (r, was) in moved {
        println!("  ~ {} {} {was} -> {} by {}: {}", r.key, r.name, r.family, r.source, fields(r.key));
    }
    Ok(())
}

/// Every override and every exclusion against the table: the record exists
/// under that key and name, its stored items are what the plugin's detector
/// finds, an override pays at least one block, and the money rule holds on
/// what the table says. All before anything is written.
fn verify_overrides(spec: &[FamilySpec], by_key: &HashMap<u32, &Gimmick>) -> Result<()> {
    for family in spec {
        for (field, records, must_pay) in family.fields() {
            for rec in records {
                let location = format!("{}.{field}: record {} {}", family.name, rec.key, q(&rec.name));
                let Some(g) = by_key.get(&rec.key) else {
                    bail!("{location}: no record with that key in {GIMMICK}. The table changed, or the entry is wrong.");
                };
                if g.name != rec.name {
                    bail!("{location}: the record with that key is {} in {GIMMICK}", q(&g.name));
                }
                if must_pay && g.items.is_empty() {
                    bail!("{location}: owns no resource-output block the plugin edits; nothing to multiply");
                }
                check_money_rule(&location, &rec.name, &family.name, &g.items, "the table")?;
                if g.items != rec.items {
                    bail!(
                        "{location}: the stored items do not match the table.\n  \
                         stored: {}\n  in the table: {} over {} output list(s)\n\
                         Fix the json or work out what changed in the game's table first.",
                        py_ints(&rec.items),
                        py_ints(&g.items),
                        g.lists
                    );
                }
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // `gen-collect-names: <what went wrong>`, the prefix scripts grep.
            eprintln!("gen-collect-names: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, key: u32, family: &str) -> Row {
        Row { name: name.into(), key, family: family.into(), display: "x".into(), source: "rule" }
    }

    #[test]
    fn rows_sort_case_insensitively_and_stably() {
        let mut rows = Rows::new();
        rows.push(row("Zebra", 1, "Foraging"));
        rows.push(row("apple", 2, "Logging"));
        rows.push(row("Apple_2", 3, "Mining"));
        let got: Vec<&str> = rows.sorted_by_name().into_iter().map(|r| r.name.as_str()).collect();
        assert_eq!(got, vec!["apple", "Apple_2", "Zebra"]);
        assert_eq!(rows.count_in_family("Foraging"), 1);
    }

    #[test]
    fn previous_rows_read_both_row_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("collect.rs");
        fs::write(
            &p,
            "    (1, \"a\", Family::Foraging),\n    (2, \"b\", Family::Logging, \"Timber\"),\n    (3, \"c\")\n",
        )
        .unwrap();
        let got = previous_rows(&p);
        assert_eq!(got.len(), 2);
        assert_eq!(got[&2], ("b".to_string(), "Logging".to_string()));
        assert!(previous_rows(&dir.path().join("none.rs")).is_empty());
    }

    #[test]
    fn display_names_keep_column_0x200_only() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.tsv");
        fs::write(&p, "key\tcolumn\ttext\n7\t0x200\tPeony\n7\t0x19200\tother\n8\t0x200\t \nx\t0x200\tbad\n").unwrap();
        let got = display_names(&p).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[&7], "Peony");
    }
}
