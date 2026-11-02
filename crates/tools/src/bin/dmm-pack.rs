//! Generate the Desert Gatherer DMM pack from the game's own `gimmickinfo`
//! table and the rows of `collect.rs`.
//!
//! ```text
//! dmm-pack                      archive table, appmanifest build, the real pack
//! dmm-pack --tables DIR         extracted copies (`just extract-tables`) instead
//! dmm-pack --build ID           override the build id
//! dmm-pack --version X.Y        stamp a version other than dmm_pack.json's
//! dmm-pack --dry-run            verify everything, print the report, write nothing
//! ```
//!
//! It replaces `dmm-rebase`, which carried the published modules across a game
//! update by searching for each list the pack already had. This builds them
//! instead, from the same two inputs the plugin multiplies from:
//!
//!   * **The records** are the `Foraging`, `Logging`, `Mining` and `Ore` rows
//!     of `crates/desert-core/src/collect.rs` (`desert_tools::collect_rows`),
//!     which `gen-collect-names` derives from the game's own records. `Money`
//!     rows are the plugin's alone and never in the pack.
//!   * **The bytes** are the `gimmickinfo` table body out of the game's
//!     archives (`paz`, `staticinfo`), split into records by its own header.
//!     Each row's record is found by key and its header name must be the
//!     row's name. Its blocks are exactly what `desert_tools::outputs` finds
//!     with the shipped detector, which is the plugin's walk, so a DMM user
//!     and a plugin user get the same records and the same blocks multiplied.
//!     A row whose record has no block is an error.
//!
//! Every block gives two changes, the `u64` minimum at `+42` and the maximum
//! at `+50`, each `original * rate`. Four categories times rates 2, 5 and 10
//! make the twelve modules, written in the shape the pack has always had:
//! `offset` is absolute in the table body, `record_rel_offset` counts from the
//! record's `u32` key, `rel_offset` from the header's last byte (the flag byte
//! on a `NameFirst` table, the name's NUL on a `KeyFirst` one), and the label's
//! `output G.B` is list `G` of the record and block `B` of that list, both
//! 1-based. Changes are in offset order. The module files and
//! `VERIFICATION.txt` are CRLF (they ship to Windows users), `dmm_pack.json`
//! is only read.
//!
//! **All or nothing.** Every module and the whole report are built in memory
//! first; two categories editing the same byte is an error (one rate per
//! category is only safe to mix while they are disjoint), and only then is
//! anything written, through temporary files renamed into place. A failure
//! leaves the pack exactly as it was.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use clap::Parser;
use serde_json::{json, Value};

use desert_tools::collect_rows::{self, CollectRow};
use desert_tools::outputs::{self, Detector};
use desert_tools::paths;
use desert_tools::paz::Archive;
use desert_tools::records::{header_at, probe};
use desert_tools::sha256::hex_digest;
use desert_tools::staticinfo::{self, Table};

/// The pack, relative to the repo root.
const PACK_DIR: &str = "desert-gatherer-dmm";
/// The pack manifest DMM reads. It sits beside the modules but is not one.
const PACK_MANIFEST: &str = "dmm_pack.json";
const VERIFICATION: &str = "VERIFICATION.txt";
const VERIFICATION_HEADER: &str = "Automated package verification";
const TABLE: &str = "gimmickinfo";
const GAME_FILE: &str = "gamedata/gimmickinfo.pabgb";
const AUTHOR: &str = "KiefBC";
const GAME: &str = "Crimson Desert Enhanced";

/// `collect.rs` family -> the pack's category, which names the module files
/// and heads every label. `Money` is not here on purpose.
const CATEGORIES: [(&str, &str); 4] =
    [("Foraging", "Foraging"), ("Logging", "Logging"), ("Mining", "Mining"), ("Ore", "Ore Nodes")];

/// One module per category per rate.
const RATES: [u64; 3] = [2, 5, 10];

/// The report's `mixed_selection_test`: one rate from one category plus a
/// different rate from another, applied together, exactly as a user mixing
/// modules in DMM would.
const MIXED: [(&str, u64); 2] = [("Mining", 2), ("Logging", 5)];

#[derive(Parser)]
#[command(
    name = "dmm-pack",
    about = "Generate desert-gatherer-dmm/ from the game's gimmickinfo table and collect.rs."
)]
struct Cli {
    /// A directory of extracted tables (`gimmickinfo.staticinfobody` and
    /// `.staticinfoheader`, as `just extract-tables` writes) [default: the
    /// archives under $CD_GAME_DIR, else the parent of $CD_BIN64]
    #[arg(long, value_name = "DIR")]
    tables: Option<PathBuf>,
    /// The rows [default: <repo>/crates/desert-core/src/collect.rs]
    #[arg(long, value_name = "FILE")]
    collect: Option<PathBuf>,
    /// The game build the table came from [default: the buildid in the Steam
    /// appmanifest, $CD_APPMANIFEST]
    #[arg(long, value_name = "ID")]
    build: Option<String>,
    /// The pack directory [default: <repo>/desert-gatherer-dmm]
    #[arg(long, value_name = "DIR")]
    pack_dir: Option<PathBuf>,
    /// The version stamped into every module [default: dmm_pack.json's]
    #[arg(long, value_name = "X.Y")]
    version: Option<String>,
    /// Build and verify everything and print the report; write nothing.
    #[arg(long)]
    dry_run: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("dmm-pack: {e:#}");
        std::process::exit(1);
    }
}

/// One scalar edit, as it lands in the table.
#[derive(Debug, Clone)]
struct Change {
    offset: usize,
    record_key: u32,
    record_rel_offset: usize,
    entry: String,
    rel_offset: usize,
    list: usize,
    block: usize,
    is_min: bool,
    original: u64,
}

/// What one category patches, rate-independent.
struct Category {
    name: &'static str,
    records: usize,
    blocks: usize,
    changes: Vec<Change>,
}

/// One module, ready to write.
struct Module {
    file: String,
    name: String,
    category: &'static str,
    rate: u64,
    records: usize,
    blocks: usize,
    /// `(offset, patched bytes)`, for the report's digests.
    edits: Vec<(usize, [u8; 8])>,
    text: String,
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let root = paths::repo_root()?;
    let pack = cli.pack_dir.clone().unwrap_or_else(|| root.join(PACK_DIR));
    let collect = cli.collect.clone().unwrap_or_else(|| paths::crate_dir_in(&root, "desert-core").join("src").join("collect.rs"));

    let build = match cli.build.clone().or_else(paths::build_id) {
        Some(b) => b,
        None => bail!(
            "no game build id: pass --build, or point CD_APPMANIFEST at the Steam appmanifest \
             (looked for a buildid in {})",
            paths::appmanifest().display()
        ),
    };
    let version = match cli.version.clone() {
        Some(v) => v,
        None => manifest_version(&pack)?,
    };

    let (table, source) = load_table(cli.tables.as_deref())?;
    let table_sha = hex_digest(&table.body);
    println!("table: {source}: {} bytes sha256={table_sha} build={build}", table.body.len());

    let rows = collect_rows::read(&collect)?;
    let categories = categories(&table, &rows)?;
    let previous = previous_coverage(&pack);
    for c in &categories {
        let was = match previous.get(c.name) {
            Some((r, b)) => format!(" (was {r} records, {b} outputs)"),
            None => String::new(),
        };
        println!(
            "  {}: {} records, {} outputs, {} scalar edits per rate option{was}",
            c.name,
            c.records,
            c.blocks,
            c.changes.len()
        );
    }

    let mut modules = Vec::new();
    for c in &categories {
        for rate in RATES {
            modules.push(module(c, rate, &build, &version)?);
        }
    }
    // Byte order of file name: the order the report's options have always had.
    modules.sort_by(|a, b| a.file.cmp(&b.file));

    let (report, disjoint) = build_report(&modules, &categories, &table.body, &table_sha, &build)?;
    let text = verification_text(&report);
    if cli.dry_run {
        println!("{text}");
    }
    if !disjoint {
        println!("disjoint: False | result: FAIL");
        bail!("two categories edit the same bytes, so their rates cannot be mixed; nothing written");
    }
    if cli.dry_run {
        println!("disjoint: True | result: PASS (dry run, nothing written)");
        return Ok(());
    }
    let mut outputs: Vec<(PathBuf, String)> = modules.iter().map(|m| (pack.join(&m.file), crlf(&m.text))).collect();
    outputs.push((pack.join(VERIFICATION), crlf(&text)));
    write_all_or_nothing(&outputs)?;
    println!("wrote {} modules and {VERIFICATION} into {}", modules.len(), pack.display());
    println!("disjoint: True | result: PASS");
    Ok(())
}

/// The pack's version, from its own manifest.
fn manifest_version(pack: &Path) -> Result<String> {
    let path = pack.join(PACK_MANIFEST);
    let text = fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
    let v: Value = serde_json::from_str(&text).with_context(|| format!("{} is not valid JSON", path.display()))?;
    v["version"]
        .as_str()
        .map(str::to_string)
        .with_context(|| format!("{}: no string \"version\" (or pass --version)", path.display()))
}

fn load_table(tables: Option<&Path>) -> Result<(Table, String)> {
    let hint = "pass --tables DIR (extracted copies, `just extract-tables`) or set CD_GAME_DIR \
                to the install holding the archives";
    match tables {
        Some(dir) => {
            let t = staticinfo::read_from_dir(dir, TABLE).with_context(|| hint.to_string())?;
            Ok((t, format!("{}/{TABLE}", dir.display())))
        }
        None => {
            let game = paths::game_dir();
            let t = Archive::open(&game)
                .and_then(|a| staticinfo::read_from_archive(&a, TABLE))
                .with_context(|| format!("cannot read {TABLE} out of the archives under {}; {hint}", game.display()))?;
            Ok((t, format!("{TABLE} in the archives under {}", game.display())))
        }
    }
}

/// Every category's changes, verified against the table.
fn categories(table: &Table, rows: &[CollectRow]) -> Result<Vec<Category>> {
    let body = &table.body;
    let records = table.split().with_context(|| format!("{TABLE}: the header does not split the body"))?;
    let mut by_key: HashMap<u64, staticinfo::Record> = HashMap::with_capacity(records.len());
    for r in &records {
        ensure!(by_key.insert(r.key, *r).is_none(), "{TABLE}: key {} appears twice in the header", r.key);
    }
    let Some(shape) = probe(body) else {
        bail!("{TABLE}: the first record validates as neither header shape (or as both)");
    };

    let mut out = Vec::with_capacity(CATEGORIES.len());
    for (family, name) in CATEGORIES {
        let mine: Vec<&CollectRow> = rows.iter().filter(|r| r.family == family).collect();
        ensure!(!mine.is_empty(), "collect.rs has no Family::{family} row");
        let (mut changes, mut blocks) = (Vec::new(), 0usize);
        for row in &mine {
            let Some(rec) = by_key.get(&u64::from(row.key)) else {
                bail!("{family} row {} '{}': no record with that key in {TABLE}", row.key, row.name);
            };
            let Some(h) = header_at(body, rec.offset, shape) else {
                bail!("{family} row {} '{}': the record at {} has no {shape:?} header", row.key, row.name, rec.offset);
            };
            ensure!(
                h.key == row.key && h.name == row.name,
                "{family} row {} '{}': the record at that key is {} '{}'",
                row.key,
                row.name,
                h.key,
                h.name
            );
            let slice = &body[rec.offset..rec.end];
            let found = outputs::blocks(slice, Detector::Shipped);
            ensure!(!found.is_empty(), "{family} row {} '{}': no output block the plugin multiplies", row.key, row.name);
            blocks += found.len();
            for b in found {
                for (is_min, at) in [(true, b.min_at()), (false, b.max_at())] {
                    let offset = rec.offset + at;
                    let original = u64::from_le_bytes(body[offset..offset + 8].try_into().unwrap());
                    changes.push(Change {
                        offset,
                        record_key: row.key,
                        record_rel_offset: offset - h.key_at,
                        entry: row.name.clone(),
                        rel_offset: offset - (h.body - 1),
                        list: b.list,
                        block: b.block,
                        is_min,
                        original,
                    });
                }
            }
        }
        changes.sort_by_key(|c| c.offset);
        out.push(Category { name, records: mine.len(), blocks, changes });
    }
    Ok(out)
}

/// The published coverage of each category, out of its 2X module, if there
/// is one to read: what the run prints as "was".
fn previous_coverage(pack: &Path) -> HashMap<&'static str, (u64, u64)> {
    let mut out = HashMap::new();
    for (_, name) in CATEGORIES {
        let Ok(text) = fs::read_to_string(pack.join(format!("{name} - 2X.json"))) else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
        if let (Some(r), Some(b)) = (v["coverage"]["records"].as_u64(), v["coverage"]["resource_outputs"].as_u64()) {
            out.insert(name, (r, b));
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One module's JSON, in the pack's key order.
fn module(c: &Category, rate: u64, build: &str, version: &str) -> Result<Module> {
    let name = format!("{} - {rate}X", c.name);
    let lower = c.name.to_lowercase();
    let mut changes = Vec::with_capacity(c.changes.len());
    let mut edits = Vec::with_capacity(c.changes.len());
    for ch in &c.changes {
        let Some(patched) = ch.original.checked_mul(rate) else {
            bail!("{}: record {} block {}.{}: {} * {rate} overflows a u64", c.name, ch.record_key, ch.list, ch.block, ch.original);
        };
        let which = if ch.is_min { "minimum" } else { "maximum" };
        changes.push(json!({
            "offset": ch.offset,
            "record_key": ch.record_key,
            "record_rel_offset": ch.record_rel_offset,
            "entry": ch.entry,
            "rel_offset": ch.rel_offset,
            "label": format!("{}: {} output {}.{} {which} ({} -> {patched})", c.name, ch.entry, ch.list, ch.block, ch.original),
            "original": hex(&ch.original.to_le_bytes()),
            "patched": hex(&patched.to_le_bytes()),
        }));
        edits.push((ch.offset, patched.to_le_bytes()));
    }
    let json = json!({
        "name": name,
        "version": version,
        "author": AUTHOR,
        "description": format!(
            "Multiplies {lower} node minimum and maximum yields by {rate}. Select no more than one {} rate in DMM.",
            c.name
        ),
        "game": GAME,
        "game_build": build,
        "multiplier": rate,
        "category": c.name,
        "allow_partial_apply": true,
        "modinfo": {
            "name": name,
            "version": version,
            "author": AUTHOR,
            "description": format!("{rate}x yields for {lower} nodes."),
            "allow_partial_apply": true,
        },
        "coverage": {
            "records": c.records,
            "resource_outputs": c.blocks,
            "scalar_patches": c.changes.len(),
        },
        "patches": [
            {
                "game_file": GAME_FILE,
                "changes": changes,
            }
        ],
    });
    Ok(Module {
        file: format!("{name}.json"),
        name,
        category: c.name,
        rate,
        records: c.records,
        blocks: c.blocks,
        edits,
        text: serde_json::to_string_pretty(&json).unwrap() + "\n",
    })
}

/// The table with some edits applied, as a digest: what DMM's mounted file
/// would hash to with those modules enabled.
fn simulate<'a>(table: &[u8], edits: impl IntoIterator<Item = &'a (usize, [u8; 8])>) -> Result<String> {
    let mut out = table.to_vec();
    for (offset, patched) in edits {
        let Some(dest) = out.get_mut(*offset..offset + patched.len()) else {
            bail!("an edit at {offset} runs past the end of the table");
        };
        dest.copy_from_slice(patched);
    }
    Ok(hex_digest(&out))
}

/// `VERIFICATION.txt`'s JSON, and whether the categories are disjoint.
fn build_report(
    modules: &[Module],
    categories: &[Category],
    table: &[u8],
    table_sha: &str,
    build: &str,
) -> Result<(Value, bool)> {
    let mut options = Vec::with_capacity(modules.len());
    for m in modules {
        options.push(json!({
            "option": m.name,
            "records": m.records,
            "resource_outputs": m.blocks,
            "scalar_patches": m.edits.len(),
            "simulated_patched_sha256": simulate(table, &m.edits)?,
            "result": "PASS",
        }));
    }

    let offsets: Vec<(&str, HashSet<usize>)> =
        categories.iter().map(|c| (c.name, c.changes.iter().map(|ch| ch.offset).collect())).collect();
    for (name, set) in &offsets {
        let c = categories.iter().find(|c| c.name == *name).unwrap();
        ensure!(set.len() == c.changes.len(), "{name}: two changes at the same offset");
    }
    let disjoint =
        offsets.iter().enumerate().all(|(i, (_, a))| offsets[i + 1..].iter().all(|(_, b)| a.is_disjoint(b)));

    let mut mixed: Vec<&(usize, [u8; 8])> = Vec::new();
    for (cat, rate) in MIXED {
        let Some(m) = modules.iter().find(|m| m.category == cat && m.rate == rate) else {
            bail!("the mixed selection test needs a {cat} {rate}X module");
        };
        mixed.extend(&m.edits);
    }

    let mut cats = serde_json::Map::new();
    for c in categories {
        cats.insert(
            c.name.to_string(),
            json!({
                "records": c.records,
                "resource_outputs": c.blocks,
                "scalar_patches": c.changes.len(),
            }),
        );
    }
    let selections: serde_json::Map<String, Value> = MIXED.iter().map(|(c, m)| (c.to_string(), json!(m))).collect();
    let report = json!({
        "game_build": build,
        "source_gimmickinfo_sha256": table_sha,
        "module_count": modules.len(),
        "categories": cats,
        "options": options,
        "category_offset_sets_disjoint": disjoint,
        "mixed_selection_test": {
            "selections": selections,
            "scalar_patches": mixed.len(),
            "simulated_patched_sha256": simulate(table, mixed)?,
            "result": "PASS",
        },
        "result": if disjoint { "PASS" } else { "FAIL" },
    });
    Ok((report, disjoint))
}

/// `VERIFICATION.txt`, LF, every non-ASCII character escaped as `\uXXXX`
/// (the shape the Python that first wrote it had).
fn verification_text(report: &Value) -> String {
    format!("{VERIFICATION_HEADER}\n\n{}\n", ascii_escape(&serde_json::to_string_pretty(report).unwrap()))
}

/// Every non-ASCII character as `\uXXXX`, lowercase, astral ones as a
/// surrogate pair. Outside a string JSON is pure ASCII, so this can run over
/// the serialised text as a whole.
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

/// CRLF: the pack's module files and report are CRLF in git because they
/// ship to Windows users. JSON output carries no literal newline inside a
/// string, so every `\n` is a line end.
fn crlf(text: &str) -> String {
    text.replace('\n', "\r\n")
}

/// Write every file or none. Each goes to a temporary sibling first; only once
/// all of them are on disk are they renamed over the originals. A failure
/// while writing removes the temporaries and leaves the pack untouched.
fn write_all_or_nothing(outputs: &[(PathBuf, String)]) -> Result<()> {
    let temp = |p: &Path| {
        let mut name = p.file_name().unwrap_or_default().to_os_string();
        name.push(".dmm-pack.tmp");
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

    /// The report's header and blank line survive a parse and a rewrite.
    #[test]
    fn verification_text_has_its_header() {
        let text = verification_text(&json!({"result": "PASS"}));
        assert_eq!(text, "Automated package verification\n\n{\n  \"result\": \"PASS\"\n}\n");
    }

    /// `json.dumps("é𝄞")` with the default `ensure_ascii=True`.
    #[test]
    fn ascii_escape_matches_python() {
        assert_eq!(ascii_escape("\"é𝄞\""), "\"\\u00e9\\ud834\\udd1e\"");
    }

    #[test]
    fn hex_is_lowercase_little_endian() {
        assert_eq!(hex(&10u64.to_le_bytes()), "0a00000000000000");
        assert_eq!(crlf("a\nb\n"), "a\r\nb\r\n");
    }
}
