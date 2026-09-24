//! Rebase the Desert Gatherer DMM pack onto a new game build's clean table.
//!
//! ```text
//! dmm-rebase                          table and build id from the defaults
//! dmm-rebase --table FILE --build ID  either or both given explicitly
//! dmm-rebase --dry-run                verify everything, print the report, write nothing
//! ```
//!
//! The table defaults to `paths::dmm_table()`: `CD_DMM_TABLE`, else the
//! plugin's own dump `<bin64>/DesertTooling.gimmickinfo.bin` (one launch with
//! `[Gatherer] DumpTable=1` and `DryRun=1` writes it), else DMM's backup copy.
//! The build id defaults to the one in the Steam appmanifest
//! (`paths::build_id()`, `CD_APPMANIFEST`). There is no fallback for the build:
//! a pack stamped with a guessed build is worse than no pack.
//!
//! Every change in a module stores `record_key`, `entry` and
//! `record_rel_offset` (bytes from the record's `u32` key). Records move
//! between builds, and a resource-output list can shift inside its record, but
//! each output block is a fixed 68-byte structure (`u8` flag = 1, the item id
//! as a `u32` at +5, `u64` min at +42, `u64` max at +50, `FF FF` at +58, the
//! item id again at +64) behind a `u32` count. So for every record this finds
//! the record by key and name, finds each output list by that signature near
//! where it used to be, checks the vanilla bytes are still the ones the pack
//! expects, and rewrites the three offsets of every change. Then it
//! regenerates `VERIFICATION.txt`: the table's digest, the digest of the table
//! with each module applied, and the checks that the four categories edit
//! disjoint bytes (so one rate per category can be mixed freely).
//!
//! Ported from `desert-gatherer-dmm/rebase.py`, which shipped inside the pack
//! zip. The stdout lines, the rewritten JSON and `VERIFICATION.txt` are what
//! the Python produced, byte for byte - the module files are diffed in review
//! on every rebase, and formatting churn would bury the offsets that actually
//! moved. Three things are deliberately different:
//!
//!   * **All or nothing.** The Python rebased and wrote one module at a time,
//!     so a record it could not resolve in module seven left modules one to
//!     six rebased and the rest not: a pack half on one build and half on
//!     another, which DMM would happily mount. This verifies every module and
//!     builds the whole report before anything is written, and then writes
//!     through temporary files renamed into place, so a failure leaves the
//!     pack exactly as it was.
//!   * **An overlap is a failure.** The Python wrote `"result": "FAIL"` into
//!     the report when two categories edited the same byte, and still exited
//!     0 with every file rewritten. That is now an error and nothing is
//!     written (a dry run still prints the report, so the overlap can be
//!     inspected).
//!   * **Records are located in one pass** over the table for every record of
//!     every module, instead of one full scan per record per module - 275
//!     records times twelve modules was over three thousand scans of a 22 MB
//!     buffer. The scan also looks at every byte position, where the Python's
//!     `re.finditer` skipped matches overlapping a previous one; a record
//!     header can never overlap another, so the answer is the same.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use clap::Parser;
use regex::Regex;
use serde_json::{json, Value};

use desert_tools::paths;
use desert_tools::sha256::hex_digest;

/// The pack, relative to the repo root.
const PACK_DIR: &str = "desert-gatherer-dmm";
/// The pack manifest DMM reads. It sits beside the modules but is not one.
const PACK_MANIFEST: &str = "dmm_pack.json";
const VERIFICATION: &str = "VERIFICATION.txt";
const VERIFICATION_HEADER: &str = "Automated package verification";

/// Bytes per resource-output block.
const BLOCK: usize = 68;
/// Where the `u64` minimum and maximum sit inside a block.
const MIN_AT: usize = 42;
const MAX_AT: usize = 50;
/// How far either side of a list's old record-relative position to look for
/// it in the new table. On every update so far lists have moved inside their
/// record by far less than this, or not at all. A rebase that fails on one or
/// two specific records after `just test-game`'s pack oracle has passed is the
/// sign that one moved further: widen this before suspecting the format.
const SEARCH_WINDOW: i64 = 1024;

/// What a change's label says about it: `... output <group>.<block>
/// <minimum|maximum> ...`, for example `Mining: mine_bluestone output 1.2
/// maximum (1 -> 2)`. The block number is 1-based within its list.
const LABEL: &str = r" output (\d+)\.(\d+) (minimum|maximum) ";

/// The report's `mixed_selection_test`: one rate from one category plus a
/// different rate from another, applied together, exactly as a user mixing
/// modules in DMM would.
const MIXED: [(&str, u64); 2] = [("Mining", 2), ("Logging", 5)];

#[derive(Parser)]
#[command(
    name = "dmm-rebase",
    about = "Rebase desert-gatherer-dmm/*.json onto a new game build's clean gimmickinfo table."
)]
struct Cli {
    /// The clean gimmickinfo table body [default: $CD_DMM_TABLE, else
    /// <bin64>/DesertTooling.gimmickinfo.bin if the plugin has dumped it, else
    /// DMM's backup copy]
    #[arg(long, value_name = "FILE")]
    table: Option<PathBuf>,
    /// The game build the table came from [default: the buildid in the Steam
    /// appmanifest, $CD_APPMANIFEST]
    #[arg(long, value_name = "ID")]
    build: Option<String>,
    /// Verify every module and print the report; write nothing.
    #[arg(long)]
    dry_run: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("dmm-rebase: {e:#}");
        std::process::exit(1);
    }
}

/// One scalar edit, as it lands in the table: the report's digests are over
/// the table with these applied.
struct Edit {
    offset: usize,
    patched: Vec<u8>,
}

/// A module after its offsets have been rebased in memory.
struct Module {
    path: PathBuf,
    json: Value,
    name: String,
    category: String,
    multiplier: u64,
    edits: Vec<Edit>,
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = paths::repo_root()?;
    let pack = root.join(PACK_DIR);

    let build = match cli.build.clone().or_else(paths::build_id) {
        Some(b) => b,
        None => bail!(
            "no game build id: pass --build, or point CD_APPMANIFEST at the Steam \
             appmanifest (looked for a buildid in {})",
            paths::appmanifest().display()
        ),
    };
    let table_path = cli.table.clone().unwrap_or_else(paths::dmm_table);
    let table = fs::read(&table_path).with_context(|| {
        format!(
            "cannot read the clean table {} (launch the game once with [Gatherer] \
             DumpTable=1 and DryRun=1, set CD_DMM_TABLE, or pass --table)",
            table_path.display()
        )
    })?;
    let table_sha = hex_digest(&table);
    println!("table: {} bytes sha256={table_sha} build={build}", table.len());

    let files = pack_modules(&pack)?;
    let mut loaded = Vec::with_capacity(files.len());
    for path in files {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let json: Value = serde_json::from_str(&text)
            .with_context(|| format!("{} is not valid JSON", path.display()))?;
        loaded.push((path, json));
    }

    let recs = locate_records(&table, &wanted_records(&loaded)?)?;
    let label = Regex::new(LABEL).unwrap();
    let mut modules = Vec::with_capacity(loaded.len());
    for (path, mut json) in loaded {
        let file = file_name(&path);
        let edits = rebase_module(&file, &mut json, &table, &recs, &label)?;
        json["game_build"] = Value::String(build.clone());
        modules.push(Module {
            name: str_field(&json, "name", &file)?.to_string(),
            category: str_field(&json, "category", &file)?.to_string(),
            multiplier: json["multiplier"]
                .as_u64()
                .with_context(|| format!("{file}: no integer multiplier"))?,
            path,
            json,
            edits,
        });
    }

    let (report, disjoint) = build_report(&modules, &table, &table_sha, &build)?;
    let text = verification_text(&report);
    if cli.dry_run {
        println!("{text}");
    }
    let result = if disjoint { "PASS" } else { "FAIL" };
    if !disjoint {
        // Printed before failing so the line reads as it always has; the
        // Python went on to write everything regardless.
        println!("disjoint: False | result: {result}");
        bail!("two categories edit the same bytes, so their rates cannot be mixed; nothing written");
    }
    if !cli.dry_run {
        let mut outputs: Vec<(PathBuf, String)> = modules
            .iter()
            .map(|m| (m.path.clone(), crlf(&module_text(&m.json))))
            .collect();
        outputs.push((pack.join(VERIFICATION), crlf(&text)));
        write_all_or_nothing(&outputs)?;
    }
    println!("disjoint: True | result: {result}");
    Ok(())
}

/// The pack's module files: every `*.json` but the manifest, in byte order of
/// file name, which is the order `sorted(glob("*.json"))` gave the Python and
/// therefore the order of the report's `options`.
fn pack_modules(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut modules: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("cannot read the pack directory {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|entry| entry.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .filter(|p| p.file_name().is_some_and(|n| n != PACK_MANIFEST))
        .collect();
    modules.sort();
    ensure!(!modules.is_empty(), "no module JSON in {}", dir.display());
    Ok(modules)
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

fn str_field<'a>(v: &'a Value, key: &str, file: &str) -> Result<&'a str> {
    v[key].as_str().with_context(|| format!("{file}: no string \"{key}\""))
}

/// The `changes` array of every patch group in a module.
fn groups<'a>(module: &'a Value, file: &str) -> Result<Vec<&'a Vec<Value>>> {
    let patches = module["patches"]
        .as_array()
        .with_context(|| format!("{file}: no \"patches\" array"))?;
    patches
        .iter()
        .map(|g| {
            g["changes"]
                .as_array()
                .with_context(|| format!("{file}: a patch group has no \"changes\" array"))
        })
        .collect()
}

/// A change's `record_key`, which the table stores as a `u32`.
fn record_key(change: &Value, file: &str) -> Result<u32> {
    change["record_key"]
        .as_u64()
        .and_then(|k| u32::try_from(k).ok())
        .with_context(|| format!("{file}: a change has no u32 \"record_key\""))
}

/// Every (key, name) record any module edits, first-seen order.
fn wanted_records(modules: &[(PathBuf, Value)]) -> Result<Vec<(u32, String)>> {
    let mut seen = HashSet::new();
    let mut wanted = Vec::new();
    for (path, json) in modules {
        let file = file_name(path);
        for changes in groups(json, &file)? {
            for c in changes {
                let key = record_key(c, &file)?;
                let entry = str_field(c, "entry", &file)?.to_string();
                if seen.insert((key, entry.clone())) {
                    wanted.push((key, entry));
                }
            }
        }
    }
    Ok(wanted)
}

fn read_u32(table: &[u8], at: usize) -> Option<u32> {
    let bytes = table.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(bytes.try_into().unwrap()))
}

/// Map each wanted (key, name) to the absolute offset of its record's `u32`
/// key. A record starts `u32 key, u32 len, name, NUL`, where `len` is the
/// name's length without the NUL; exactly one such header must exist.
fn locate_records(table: &[u8], wanted: &[(u32, String)]) -> Result<HashMap<(u32, String), usize>> {
    let mut by_key: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, (key, _)) in wanted.iter().enumerate() {
        by_key.entry(*key).or_default().push(i);
    }
    let mut hits: Vec<Vec<usize>> = vec![Vec::new(); wanted.len()];
    for p in 0..table.len().saturating_sub(3) {
        let Some(candidates) = by_key.get(&read_u32(table, p).unwrap()) else {
            continue;
        };
        for &i in candidates {
            let name = wanted[i].1.as_bytes();
            if read_u32(table, p + 4) != Some(name.len() as u32) {
                continue;
            }
            let start = p + 8;
            if table.get(start..start + name.len()) == Some(name)
                && table.get(start + name.len()) == Some(&0)
            {
                hits[i].push(p);
            }
        }
    }
    let mut found = HashMap::with_capacity(wanted.len());
    for ((key, name), hit) in wanted.iter().zip(hits) {
        if hit.len() != 1 {
            bail!("record {key} '{name}': expected 1 hit, got {hit:?}");
        }
        found.insert((*key, name.clone()), hit[0]);
    }
    Ok(found)
}

fn unhex(s: &str, what: &str) -> Result<Vec<u8>> {
    ensure!(s.len().is_multiple_of(2) && s.is_ascii(), "{what}: {s:?} is not hex bytes");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).with_context(|| format!("{what}: {s:?} is not hex bytes")))
        .collect()
}

/// A little-endian integer of up to 16 bytes. The block's values are `u64`s,
/// so `u128` holds any of them times any multiplier the pack uses without the
/// overflow the Python's big integers never had to think about.
fn le_value(bytes: &[u8]) -> Option<u128> {
    if bytes.len() > 16 {
        return None;
    }
    Some(bytes.iter().rev().fold(0u128, |acc, &b| (acc << 8) | u128::from(b)))
}

/// One output list the changes of a patch group point into.
struct List {
    key: u32,
    entry: String,
    group: u64,
    /// Where the list's `u32` count was, relative to the record's key.
    old_list: i64,
    /// Block number (1-based) -> the vanilla (minimum, maximum) bytes.
    expect: BTreeMap<usize, [Option<Vec<u8>>; 2]>,
    /// (index into `changes`, block number, offset of the value in the block).
    members: Vec<(usize, usize, usize)>,
}

/// Rebase one module's changes in place and return its edits in file order.
fn rebase_module(
    file: &str,
    module: &mut Value,
    table: &[u8],
    recs: &HashMap<(u32, String), usize>,
    label: &Regex,
) -> Result<Vec<Edit>> {
    let mult = module["multiplier"]
        .as_u64()
        .with_context(|| format!("{file}: no integer multiplier"))?;
    let group_count = groups(module, file)?.len();
    let mut edits = Vec::new();
    for gi in 0..group_count {
        let changes = module["patches"][gi]["changes"].as_array_mut().unwrap();
        let mut lists: Vec<List> = Vec::new();
        let mut index: HashMap<(u32, u64), usize> = HashMap::new();
        let mut originals = Vec::with_capacity(changes.len());

        for (ci, c) in changes.iter().enumerate() {
            let text = str_field(c, "label", file)?;
            let Some(m) = label.captures(text) else {
                bail!("{file}: unparsable label '{text}'");
            };
            let group: u64 = m[1].parse().with_context(|| format!("{file}: {text}: group"))?;
            let n: usize = m[2].parse().with_context(|| format!("{file}: {text}: block"))?;
            ensure!(n >= 1, "{file}: {text}: block numbers start at 1");
            let is_min = &m[3] == "minimum";
            let original = unhex(str_field(c, "original", file)?, file)?;
            let patched = unhex(str_field(c, "patched", file)?, file)?;
            let good = match (le_value(&original), le_value(&patched)) {
                (Some(o), Some(p)) => o.checked_mul(u128::from(mult)) == Some(p),
                _ => false,
            };
            ensure!(good, "{file}: {text}: bad multiplier");
            let at = if is_min { MIN_AT } else { MAX_AT };
            let rel = c["record_rel_offset"]
                .as_i64()
                .with_context(|| format!("{file}: {text}: no integer record_rel_offset"))?;
            let old_list = rel - at as i64 - 4 - ((n - 1) * BLOCK) as i64;
            let key = record_key(c, file)?;
            let li = *index.entry((key, group)).or_insert_with(|| {
                lists.push(List {
                    key,
                    entry: c["entry"].as_str().unwrap_or_default().to_string(),
                    group,
                    old_list,
                    expect: BTreeMap::new(),
                    members: Vec::new(),
                });
                lists.len() - 1
            });
            let list = &mut lists[li];
            ensure!(list.old_list == old_list, "{file}: {text}: inconsistent list offset");
            list.expect.entry(n).or_default()[usize::from(!is_min)] = Some(original.clone());
            list.members.push((ci, n, at));
            originals.push((original, patched));
        }

        let (mut stable, mut moved) = (0, 0);
        for list in &lists {
            let count = *list.expect.keys().next_back().unwrap();
            let complete = list.expect.keys().copied().eq(1..=count)
                && list.expect.values().all(|v| v.iter().all(Option::is_some));
            ensure!(
                complete,
                "{file}: record {} group {}: incomplete coverage",
                list.key,
                list.group
            );
            let expect: BTreeMap<usize, (&[u8], &[u8])> = list
                .expect
                .iter()
                .map(|(&n, [mn, mx])| (n, (mn.as_deref().unwrap(), mx.as_deref().unwrap())))
                .collect();
            let keypos = recs[&(list.key, list.entry.clone())];
            let hits = find_output_list(table, keypos, list.old_list, count, &expect);
            if hits.len() != 1 {
                bail!(
                    "{file}: record {} ({}) group {}: {} candidate lists {hits:?}",
                    list.key,
                    list.entry,
                    list.group,
                    hits.len()
                );
            }
            let q = hits[0];
            if q as i64 - keypos as i64 == list.old_list {
                stable += 1;
            } else {
                moved += 1;
            }
            for &(ci, n, at) in &list.members {
                let new_off = q + 4 + (n - 1) * BLOCK + at;
                let original = &originals[ci].0;
                // Guaranteed by the signature match; checked anyway, because
                // an offset written without it is a silent corruption in a
                // file other people mount.
                ensure!(
                    table.get(new_off..new_off + original.len()) == Some(original.as_slice()),
                    "{file}: record {} block {n}: the bytes at {new_off} are not the vanilla value",
                    list.key
                );
                let c = &mut changes[ci];
                // The name's length in bytes, as the record header stores it.
                // The Python took `len()` of the str, which counts characters;
                // every record name is ASCII, so the two agree.
                let name_len = c["entry"].as_str().unwrap_or_default().len();
                c["offset"] = json!(new_off);
                c["record_rel_offset"] = json!(new_off - keypos);
                c["rel_offset"] = json!(new_off as i64 - (keypos + 8 + name_len) as i64);
            }
        }
        for (ci, (_, patched)) in originals.into_iter().enumerate() {
            let offset = changes[ci]["offset"].as_u64().unwrap() as usize;
            edits.push(Edit { offset, patched });
        }
        println!(
            "  {file}: {} patches verified; output lists: {stable} unchanged, {moved} shifted inside record",
            changes.len()
        );
    }
    Ok(edits)
}

/// Absolute offsets of every `u32`-count-prefixed output list within
/// [`SEARCH_WINDOW`] of `keypos + old_list` whose blocks carry exactly the
/// vanilla (minimum, maximum) pairs in `expect`. Never before the record's
/// name length field: a list cannot start inside the record's own key.
fn find_output_list(
    table: &[u8],
    keypos: usize,
    old_list: i64,
    count: usize,
    expect: &BTreeMap<usize, (&[u8], &[u8])>,
) -> Vec<usize> {
    let centre = keypos as i64 + old_list;
    let lo = (keypos as i64 + 8).max(centre - SEARCH_WINDOW).max(0);
    let hi = (centre + SEARCH_WINDOW).min(table.len() as i64);
    let mut hits = Vec::new();
    for q in lo..hi.max(lo) {
        let q = q as usize;
        if read_u32(table, q).map(|c| c as usize) != Some(count) {
            continue;
        }
        let matches = expect.iter().all(|(&n, &(mn, mx))| {
            let b = q + 4 + (n - 1) * BLOCK;
            let Some(block) = table.get(b..b + BLOCK) else {
                return false;
            };
            block[0] == 1
                && &block[MIN_AT..MIN_AT + 8] == mn
                && &block[MAX_AT..MAX_AT + 8] == mx
                && block[58..60] == [0xff, 0xff]
                && block[5..9] == block[64..68]
        });
        if matches {
            hits.push(q);
        }
    }
    hits
}

/// The table with some edits applied, as a digest: what DMM's mounted file
/// would hash to with those modules enabled.
fn simulate<'a>(table: &[u8], edits: impl IntoIterator<Item = &'a Edit>) -> Result<String> {
    let mut out = table.to_vec();
    for e in edits {
        let Some(dest) = out.get_mut(e.offset..e.offset + e.patched.len()) else {
            bail!("an edit at {} runs past the end of the table", e.offset);
        };
        dest.copy_from_slice(&e.patched);
    }
    Ok(hex_digest(&out))
}

/// `VERIFICATION.txt`'s JSON, and whether the categories are disjoint.
fn build_report(modules: &[Module], table: &[u8], table_sha: &str, build: &str) -> Result<(Value, bool)> {
    // Category -> its modules by multiplier, both in first-seen order. The
    // Python's dict silently let a second module with the same category and
    // multiplier replace the first; that is a broken pack, so it is an error.
    let mut by_cat: Vec<(&str, Vec<(u64, &Module)>)> = Vec::new();
    for m in modules {
        let slot = match by_cat.iter().position(|(c, _)| *c == m.category) {
            Some(i) => &mut by_cat[i].1,
            None => {
                by_cat.push((&m.category, Vec::new()));
                &mut by_cat.last_mut().unwrap().1
            }
        };
        ensure!(
            slot.iter().all(|(mult, _)| *mult != m.multiplier),
            "two modules are {} {}X",
            m.category,
            m.multiplier
        );
        slot.push((m.multiplier, m));
    }

    let mut options = Vec::with_capacity(modules.len());
    for m in modules {
        let coverage = &m.json["coverage"];
        options.push(json!({
            "option": m.name,
            "records": coverage["records"],
            "resource_outputs": coverage["resource_outputs"],
            "scalar_patches": m.edits.len(),
            "simulated_patched_sha256": simulate(table, &m.edits)?,
            "result": "PASS",
        }));
    }

    // A category's offsets are the same for every rate, so the first module
    // of each stands for its category.
    let cat_offsets: Vec<(&str, HashSet<usize>)> = by_cat
        .iter()
        .map(|(cat, mods)| (*cat, mods[0].1.edits.iter().map(|e| e.offset).collect()))
        .collect();
    let disjoint = cat_offsets.iter().enumerate().all(|(i, (_, a))| {
        cat_offsets[i + 1..].iter().all(|(_, b)| a.is_disjoint(b))
    });

    let mut mixed: Vec<&Edit> = Vec::new();
    for (cat, mult) in MIXED {
        let Some(m) = by_cat
            .iter()
            .find(|(c, _)| *c == cat)
            .and_then(|(_, mods)| mods.iter().find(|(x, _)| *x == mult))
        else {
            bail!("the mixed selection test needs a {cat} {mult}X module");
        };
        mixed.extend(&m.1.edits);
    }

    let mut categories = serde_json::Map::new();
    for (cat, offsets) in &cat_offsets {
        let first = options
            .iter()
            .find(|o| o["option"].as_str().is_some_and(|n| n.starts_with(cat)))
            .with_context(|| format!("no option named after category {cat}"))?;
        categories.insert(
            cat.to_string(),
            json!({
                "records": first["records"],
                "resource_outputs": first["resource_outputs"],
                "scalar_patches": offsets.len(),
            }),
        );
    }
    let selections: serde_json::Map<String, Value> =
        MIXED.iter().map(|(c, m)| (c.to_string(), json!(m))).collect();
    let result = if disjoint { "PASS" } else { "FAIL" };

    let report = json!({
        "game_build": build,
        "source_gimmickinfo_sha256": table_sha,
        "module_count": modules.len(),
        "categories": categories,
        "options": options,
        "category_offset_sets_disjoint": disjoint,
        "mixed_selection_test": {
            "selections": selections,
            "scalar_patches": mixed.len(),
            "simulated_patched_sha256": simulate(table, mixed)?,
            "result": "PASS",
        },
        "result": result,
    });
    Ok((report, disjoint))
}

/// A module file's text, LF: Python's `json.dumps(obj, indent=2,
/// ensure_ascii=False)` plus a newline. serde_json's pretty printer is the
/// same format - two-space indent, `": "` and a bare `,` at line ends, `[]` and
/// `{}` for empty containers, only `"`, `\` and control characters escaped -
/// and `preserve_order` keeps every key where the file had it. The
/// `every_module_round_trips_byte_for_byte` test is what holds it to that.
fn module_text(module: &Value) -> String {
    serde_json::to_string_pretty(module).unwrap() + "\n"
}

/// `VERIFICATION.txt`, LF. The Python dumped the report with the default
/// `ensure_ascii=True`, so anything outside ASCII (a module name, one day) is
/// written as `\uXXXX`.
fn verification_text(report: &Value) -> String {
    format!(
        "{VERIFICATION_HEADER}\n\n{}\n",
        ascii_escape(&serde_json::to_string_pretty(report).unwrap())
    )
}

/// Python's `ensure_ascii`: every non-ASCII character as `\uXXXX`, lowercase,
/// astral ones as a surrogate pair. Outside a string JSON is pure ASCII, so
/// this can run over the serialised text as a whole.
fn ascii_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch);
            continue;
        }
        let mut units = [0u16; 2];
        for unit in ch.encode_utf16(&mut units) {
            out.push_str(&format!("\\u{unit:04x}"));
        }
    }
    out
}

/// CRLF, as the Python wrote through `newline="\r\n"`: the pack's files are
/// CRLF in git because they ship to Windows users. JSON output carries no
/// literal newline inside a string, so every `\n` is a line end.
fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

/// Write every file or none. Each goes to a temporary sibling first; only once
/// all of them are on disk are they renamed over the originals. A failure
/// while writing removes the temporaries and leaves the pack untouched. A
/// rename is atomic, and the renames are the only step left that could fail
/// part way - on one filesystem, with every file already written, that takes
/// a disk being pulled.
fn write_all_or_nothing(outputs: &[(PathBuf, String)]) -> Result<()> {
    let temp = |p: &Path| {
        let mut name = p.file_name().unwrap_or_default().to_os_string();
        name.push(".dmm-rebase.tmp");
        p.with_file_name(name)
    };
    let mut written = Vec::with_capacity(outputs.len());
    for (path, text) in outputs {
        let t = temp(path);
        if let Err(e) = fs::write(&t, text) {
            for w in written.iter().chain([&t]) {
                let _ = fs::remove_file(w);
            }
            return Err(e).with_context(|| format!("cannot write {}; nothing changed", t.display()));
        }
        written.push(t);
    }
    for ((path, _), t) in outputs.iter().zip(&written) {
        fs::rename(t, path).with_context(|| format!("cannot replace {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_dir() -> PathBuf {
        paths::repo_root().unwrap().join(PACK_DIR)
    }

    /// The fidelity claim, against the real files: every module, parsed and
    /// written straight back with nothing changed, is the file it came from.
    /// If serde_json and Python's `json.dumps` ever disagree on anything the
    /// pack contains - an escape, a number, key order - this fails before a
    /// rebase turns it into a thousand-line diff.
    #[test]
    fn every_module_round_trips_byte_for_byte() {
        let modules = pack_modules(&pack_dir()).unwrap();
        assert_eq!(modules.len(), 12, "{modules:?}");
        for path in modules {
            let on_disk = fs::read_to_string(&path).unwrap();
            let value: Value = serde_json::from_str(&on_disk).unwrap();
            let written = crlf(&module_text(&value));
            assert!(!written.replace("\r\n", "").contains('\n'), "a bare LF");
            assert_eq!(
                written.replace("\r\n", "\n"),
                on_disk.replace("\r\n", "\n"),
                "{} does not round-trip",
                path.display()
            );
        }
    }

    /// The same for the report, including its header and blank line.
    #[test]
    fn verification_round_trips_byte_for_byte() {
        let on_disk = fs::read_to_string(pack_dir().join(VERIFICATION)).unwrap();
        let normalised = on_disk.replace("\r\n", "\n");
        let body = normalised
            .strip_prefix(&format!("{VERIFICATION_HEADER}\n\n"))
            .expect("the header and a blank line");
        let report: Value = serde_json::from_str(body).unwrap();
        assert_eq!(verification_text(&report), normalised);
    }

    /// What `json.dumps(..., ensure_ascii=False)` prints for the characters
    /// that could plausibly turn up in a label: arrows are not escaped, nor
    /// is non-ASCII, nor DEL; control characters are, in lowercase `\u00XX`
    /// except the five with short forms.
    #[test]
    fn string_escaping_matches_python() {
        let v = json!({"label": "a -> b é \u{1} \t \u{7f} / \"q\" \\"});
        assert_eq!(
            serde_json::to_string_pretty(&v).unwrap(),
            "{\n  \"label\": \"a -> b é \\u0001 \\t \u{7f} / \\\"q\\\" \\\\\"\n}"
        );
        assert_eq!(serde_json::to_string_pretty(&json!({"a": [], "b": {}})).unwrap(), "{\n  \"a\": [],\n  \"b\": {}\n}");
    }

    /// `json.dumps("é𝄞")` with the default `ensure_ascii=True`.
    #[test]
    fn ascii_escape_matches_python() {
        assert_eq!(ascii_escape("\"é𝄞\""), "\"\\u00e9\\ud834\\udd1e\"");
    }

    #[test]
    fn labels_parse_as_the_python_regex_did() {
        let re = Regex::new(LABEL).unwrap();
        let m = re.captures("Mining: mine_bluestone output 1.12 maximum (1 -> 2)").unwrap();
        assert_eq!((&m[1], &m[2], &m[3]), ("1", "12", "maximum"));
        assert!(re.captures("Mining: mine_bluestone output 1.1 max (1 -> 2)").is_none());
    }

    /// A block carrying `item` with the given minimum and maximum.
    fn block(item: u32, min: u64, max: u64) -> Vec<u8> {
        let mut b = vec![0u8; BLOCK];
        b[0] = 1;
        b[5..9].copy_from_slice(&item.to_le_bytes());
        b[MIN_AT..MIN_AT + 8].copy_from_slice(&min.to_le_bytes());
        b[MAX_AT..MAX_AT + 8].copy_from_slice(&max.to_le_bytes());
        b[58..60].copy_from_slice(&[0xff, 0xff]);
        b[64..68].copy_from_slice(&item.to_le_bytes());
        b
    }

    fn list(blocks: &[Vec<u8>]) -> Vec<u8> {
        let mut out = (blocks.len() as u32).to_le_bytes().to_vec();
        for b in blocks {
            out.extend(b);
        }
        out
    }

    #[test]
    fn the_list_is_found_where_it_moved_to_and_only_there() {
        let one = 1u64.to_le_bytes();
        let three = 3u64.to_le_bytes();
        let expect: BTreeMap<usize, (&[u8], &[u8])> =
            [(1, (&one[..], &three[..])), (2, (&one[..], &one[..]))].into();
        let blocks = [block(7, 1, 3), block(8, 1, 1)];

        let mut table = vec![0u8; 4096];
        table[1000 + 300..1000 + 300 + 140].copy_from_slice(&list(&blocks));
        // It used to be 40 bytes earlier in its record.
        assert_eq!(find_output_list(&table, 1000, 260, 2, &expect), vec![1300]);

        // Too far away is not found at all.
        assert!(find_output_list(&table, 1000, 260 + 2000, 2, &expect).is_empty());

        // A second identical list in the window is ambiguous, and reported as such.
        table[1000 + 600..1000 + 600 + 140].copy_from_slice(&list(&blocks));
        assert_eq!(find_output_list(&table, 1000, 260, 2, &expect), vec![1300, 1600]);

        // A block whose trailing item id disagrees with its leading one is not one.
        let mut table = vec![0u8; 4096];
        let mut bad = blocks.clone();
        bad[1][64] ^= 1;
        table[1300..1440].copy_from_slice(&list(&bad));
        assert!(find_output_list(&table, 1000, 300, 2, &expect).is_empty());
    }

    #[test]
    fn records_are_found_by_key_and_name_and_must_be_unique() {
        let mut table = vec![0u8; 256];
        let header = |t: &mut Vec<u8>, at: usize, key: u32, name: &str| {
            t[at..at + 4].copy_from_slice(&key.to_le_bytes());
            t[at + 4..at + 8].copy_from_slice(&(name.len() as u32).to_le_bytes());
            t[at + 8..at + 8 + name.len()].copy_from_slice(name.as_bytes());
        };
        header(&mut table, 10, 42, "mine_a");
        // Same key, different name: not a hit for mine_a.
        header(&mut table, 100, 42, "mine_b");
        let wanted = vec![(42, "mine_a".to_string())];
        assert_eq!(locate_records(&table, &wanted).unwrap()[&wanted[0]], 10);

        header(&mut table, 150, 42, "mine_a");
        let err = locate_records(&table, &wanted).unwrap_err().to_string();
        assert_eq!(err, "record 42 'mine_a': expected 1 hit, got [10, 150]");
    }

    #[test]
    fn multiplier_check_uses_little_endian_values() {
        assert_eq!(le_value(&[0x02, 0x01]), Some(0x0102));
        assert_eq!(le_value(&[0; 17]), None);
    }
}
