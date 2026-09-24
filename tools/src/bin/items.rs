//! Item cross-reference for the gimmickinfo resource-output blocks.
//!
//! ```text
//! items                     # rebuild both outputs
//! items 22008               # what is item 22008?
//! items salt                # find an item by inferred name
//! items --record peony_01   # what does a record yield?
//! items --family Foraging   # every item of one family
//! items --unclassified      # items no Family covers yet
//! items --list              # one line per item
//! items --loose             # ditto, wider detector, own files
//! ```
//!
//! Outputs, all four gitignored and machine-generated:
//!
//! ```text
//! analysis/items.json             one entry per item id, machine-readable
//! docs/reference-items.md         the human cross-reference
//! analysis/items-loose.json       the same, `--loose` (SEPARATE FILES: a
//! docs/reference-items-loose.md   loose run can never clobber the default)
//! ```
//!
//! Reads the clean `gimmickinfo` table body - the plugin's own dump
//! (`<bin64>/DesertTooling.gimmickinfo.bin`, written under `[Gatherer]
//! DumpTable=1`) when it exists, else the copy DMM writes out
//! (`/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin`); `paths::dmm_table` has
//! the rule. It carries record-relative offsets only and so needs no rebasing
//! for a game update. Also reads `desert-core/src/collect.rs` for the current
//! `Family` of each record.
//!
//! Method, and its limits, because the doc states them and this is where they
//! are implemented (`docs/findings/2026-09-12-water-wells.md` sections 3, 5, 7
//! and 8):
//!
//!   * Blocks are found by the detector `desert_core::gimmick::output_lists`
//!     uses: a `u32 count` followed by `count` well-formed 68-byte blocks,
//!     walked greedily and non-overlapping. The item id is read at
//!     **block+1**, not +5.
//!   * A block is attributed to its record by the echo discriminator: a real
//!     record header echoes its own `u32` key immediately before a later
//!     digits-only id sub-field. A naive backwards scan finds nested string
//!     fields instead (`key = 16777216` is the usual false positive) and
//!     mis-attributes.
//!   * **Item names are inferred from the names of the records that yield
//!     them.** Nothing here reads the game's own item names: those live in the
//!     `.paz` archives and are not extracted. An inferred name is a
//!     hypothesis, and the confidence field says how much of one.
//!   * There are **two detectors** and the choice is named, never implied (see
//!     `Detector`). `shipped` is the plugin's own, both equality clauses
//!     included, and is the default; `--loose` drops the pad clause and finds
//!     589 lists / 1038 blocks / 311 items instead of 573 / 896 / 215. The
//!     extra content is real - chests, dig sites, dungeon loot - but it is
//!     **invisible to the mod**, and it is not uniformly trustworthy: read the
//!     caveats at the top of `docs/reference-items-loose.md` before using an
//!     id from it.
//!
//! Paths are resolved through `desert_tools::paths::repo_root()`, not against
//! the working directory: the tools are run from the repository root by `just`
//! and by hand from wherever the shell happens to be, and both must write the
//! same four files.

// `src/bin/items.rs` is a crate root, so its module directory is `src/bin/`
// itself, not `src/bin/items/` - a bare `mod table;` would look for
// `src/bin/table.rs` and collide with every other tool's helpers. `#[path]`
// puts the submodules where they belong, one directory per binary.
#[path = "items/checks.rs"]
mod checks;
#[path = "items/dataset.rs"]
mod dataset;
#[path = "items/doc.rs"]
mod doc;
#[path = "items/infer.rs"]
mod infer;
#[path = "items/lookup.rs"]
mod lookup;
#[path = "items/table.rs"]
mod table;

use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Parser;
use serde::Serialize;
use serde_json::Value;

use checks::IMPLAUSIBLE_HOME;
use table::Detector;

/// What the JSON records as having written it, and what the generated docs tell
/// a reader to re-run: the built binary's path from the repo root, the same
/// spelling `.github/workflows/release.yml` uses to reach these tools.
///
/// Through the port these named `tools/items.py`, so the four artifacts stayed
/// byte-identical to the Python's and a diff proved the port. That parity was
/// met and verified, and the script has since been deleted, so the anchor is
/// gone with it; there is no `just` recipe for this tool, hence the full path.
pub const GENERATOR: &str = "tools/target/x86_64-unknown-linux-gnu/release/items";
/// The `argv[0]` this prints its errors under.
pub const GENERATOR_BASENAME: &str = "items";

#[derive(Parser, Debug)]
#[command(
    name = "items",
    about = "Item cross-reference for the gimmickinfo output blocks.",
    after_help = "With no arguments, rebuilds analysis/items.json and \
                  docs/reference-items.md. With --loose and no query, rebuilds \
                  analysis/items-loose.json and docs/reference-items-loose.md \
                  instead; the two pairs are never written by the same run."
)]
pub struct Args {
    /// an item id, or part of an inferred or record name
    pub query: Option<String>,
    /// what the records matching NAME yield
    #[arg(long, value_name = "NAME")]
    pub record: Option<String>,
    /// every item of one collect.rs Family
    #[arg(long, value_name = "FAMILY")]
    pub family: Option<String>,
    /// items no Family covers
    #[arg(long)]
    pub unclassified: bool,
    /// one line per item
    #[arg(long)]
    pub list: bool,
    /// use the loose detector (item-id clause only, no pad clause): 589 lists /
    /// 1038 blocks / 311 items instead of 573 / 896 / 215. Reads and writes its
    /// OWN files, analysis/items-loose.json and docs/reference-items-loose.md,
    /// and marks everything the mod cannot see. Less trustworthy than the
    /// default - read the caveats it prints.
    #[arg(long)]
    pub loose: bool,
    /// re-walk the table instead of reading the cache for the detector in use
    #[arg(long)]
    pub rescan: bool,
    /// clean gimmickinfo body ($CD_DMM_TABLE, else the plugin's dump in
    /// bin64 if present, else DMM's backup copy)
    #[arg(long, default_value_os_t = desert_tools::paths::dmm_table())]
    pub table: PathBuf,
    #[arg(long = "json")]
    pub json_out: Option<PathBuf>,
    #[arg(long = "doc")]
    pub doc_out: Option<PathBuf>,
}

impl Args {
    /// The detector decides the files. Nothing else in the tool picks an output
    /// path, so a loose run cannot reach the default artifacts by any route
    /// except an explicit `--json`/`--doc` from the caller.
    pub fn detector(&self) -> Detector {
        if self.loose {
            Detector::Loose
        } else {
            Detector::Shipped
        }
    }

    /// Python treats `""` as "not given" for all three of these, because an
    /// empty string is falsy; a port that used `Option::is_some` would make
    /// `items --record ''` rebuild the artifacts instead of searching.
    fn normalise(&mut self) {
        for f in [&mut self.query, &mut self.record, &mut self.family] {
            if f.as_deref() == Some("") {
                *f = None;
            }
        }
    }
}

// ------------------------------------------------------------------- helpers

/// Python's `f"{n:,}"`.
pub fn commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn sv<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

pub fn uv(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64()).unwrap_or(0)
}

pub fn bv(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}

pub fn av<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[])
}

/// Python's truthiness for the optional fields: absent, `null`, `""` and `[]`
/// are all "no". Several of these keys exist only on a loose build.
pub fn truthy(v: &Value, k: &str) -> bool {
    match v.get(k) {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Bool(b)) => *b,
        _ => true,
    }
}

/// `"/".join(it["families"])`, or `None` when the item is in no family.
pub fn fams_of(it: &Value) -> Option<String> {
    let f = av(it, "families");
    if f.is_empty() {
        None
    } else {
        Some(f.iter().map(|x| x.as_str().unwrap()).collect::<Vec<_>>().join("/"))
    }
}

// ---------------------------------------------------------------------- main

fn main() {
    let mut args = Args::parse();
    args.normalise();
    let stdout = std::io::stdout();
    let mut w = BufWriter::new(stdout.lock());
    let r = run(&args, &mut w).and_then(|()| Ok(w.flush()?));
    match r {
        Ok(()) => {}
        Err(e) => {
            // `--list | grep ...` is documented usage, so a closed pipe is
            // normal and must not print a diagnostic. The Python restored the
            // default SIGPIPE disposition for this; without libc the nearest
            // equivalent is to swallow the write error the ignored signal
            // turns into.
            if is_broken_pipe(&e) {
                std::process::exit(0);
            }
            eprintln!("{GENERATOR_BASENAME}: {e}");
            std::process::exit(1);
        }
    }
}

fn is_broken_pipe(e: &anyhow::Error) -> bool {
    e.chain().any(|c| {
        c.downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
    })
}

fn run(args: &Args, w: &mut dyn Write) -> Result<()> {
    // The repository root, not the working directory: `just` runs these from
    // the root and a human runs them from wherever they are, and both have to
    // write the same four files and read the same collect.rs.
    let root = desert_tools::paths::repo_root()?;
    let collect_rs = root.join("desert-core").join("src").join("collect.rs");
    let (default_json, default_doc) = artifact_paths(&root, args.loose);
    let json_out = args.json_out.clone().unwrap_or(default_json);
    let doc_out = args.doc_out.clone().unwrap_or(default_doc);

    let querying = args.query.is_some()
        || args.record.is_some()
        || args.family.is_some()
        || args.unclassified
        || args.list;
    if !querying {
        let d = dataset::build(&args.table, &collect_rs, args.detector(), false, w)?;
        if let Some(p) = json_out.parent() {
            std::fs::create_dir_all(p)?;
        }
        if let Some(p) = doc_out.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::write(&json_out, dump_json(&d)?)?;
        std::fs::write(&doc_out, doc::render_doc(&d))?;
        let t = &d["totals"];
        writeln!(
            w,
            "wrote {} and {}: {} items, {} blocks, {} unclassified",
            rel(&json_out, &root),
            rel(&doc_out, &root),
            uv(t, "distinct_items"),
            uv(t, "blocks"),
            uv(t, "items_unclassified")
        )?;
        if args.loose {
            writeln!(
                w,
                "LOOSE population: {} of those items are INVISIBLE to the mod \
                 ({} blocks in {} lists the plugin's detector rejects),",
                uv(t, "items_loose_only"),
                uv(t, "loose_only_blocks"),
                uv(t, "loose_only_lists")
            )?;
            writeln!(
                w,
                "and {} item ids are implausible (all inside {IMPLAUSIBLE_HOME}, \
                 which is therefore being mis-parsed).",
                uv(t, "implausible_item_ids")
            )?;
            writeln!(
                w,
                "The mod's own view is analysis/items.json, untouched by this run."
            )?;
        }
        return Ok(());
    }

    let d = lookup::load(args, &json_out, &collect_rs, w)?;
    if args.loose {
        let t = &d["totals"];
        eprintln!(
            "[LOOSE population: {} items, of which {} are INVISIBLE to the mod. \
             Lines marked LOOSE-ONLY are ones",
            uv(t, "distinct_items"),
            uv(t, "items_loose_only")
        );
        eprintln!(
            " no multiplier reaches; a trailing `!` means the id itself is \
             implausible. Drop --loose for the mod's own view.]"
        );
    }
    lookup::query(args, &d, w)
}

/// The `(json, doc)` pair a run of this detector owns.
///
/// **The two pairs never touch.** A `--loose` run must never be able to
/// overwrite the population the plugin actually sees, because that is the one
/// anything downstream acts on, and a default run must not clobber the wider
/// walk's record either. This function is the only place an output path is
/// chosen, so there is no route from `--loose` to the default artifacts except
/// an explicit `--json`/`--doc` from the caller.
pub fn artifact_paths(root: &Path, loose: bool) -> (PathBuf, PathBuf) {
    if loose {
        (
            root.join("analysis").join("items-loose.json"),
            root.join("docs").join("reference-items-loose.md"),
        )
    } else {
        (
            root.join("analysis").join("items.json"),
            root.join("docs").join("reference-items.md"),
        )
    }
}

/// `json.dumps(d, indent=1) + "\n"`: one space of indent, and a trailing
/// newline. The key order is the order `build` inserted them in, which is what
/// `serde_json`'s `preserve_order` feature buys and what makes the output
/// diffable against the Python's.
fn dump_json(d: &Value) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    d.serialize(&mut ser)?;
    buf.push(b'\n');
    Ok(buf)
}

/// `Path.relative_to(ROOT)`, falling back to the full path for an output the
/// caller pointed outside the checkout with `--json`/`--doc` (which is where
/// the Python raised instead).
fn rel(p: &Path, root: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commas_matches_pythons_thousands_separator() {
        assert_eq!(commas(0), "0");
        assert_eq!(commas(999), "999");
        assert_eq!(commas(1000), "1,000");
        assert_eq!(commas(13412), "13,412");
        assert_eq!(commas(99_999_999), "99,999,999");
        assert_eq!(commas(22_325_302), "22,325,302");
    }

    /// The two artifact pairs never touch. A loose run cannot name the default
    /// files and a default run cannot name the loose ones, so neither can
    /// overwrite the other however the tool is invoked.
    #[test]
    fn the_two_detectors_artifact_pairs_are_disjoint() {
        let root = Path::new("/repo");
        let (dj, dd) = artifact_paths(root, false);
        let (lj, ld) = artifact_paths(root, true);
        let all = [&dj, &dd, &lj, &ld];
        for (i, a) in all.iter().enumerate() {
            for b in all.iter().skip(i + 1) {
                assert_ne!(a, b, "two artifacts share a path: {a:?}");
            }
        }
        assert!(dj.ends_with("analysis/items.json"));
        assert!(dd.ends_with("docs/reference-items.md"));
        assert!(lj.ends_with("analysis/items-loose.json"));
        assert!(ld.ends_with("docs/reference-items-loose.md"));
    }

    /// Python's truthiness, which decides whether an optional field prints at
    /// all. Getting this wrong prints `note` lines with nothing after them.
    #[test]
    fn truthy_follows_pythons_rules_for_the_optional_fields() {
        let v = serde_json::json!({
            "null": null, "empty_str": "", "str": "x",
            "empty_arr": [], "arr": [1], "no": false, "yes": true,
        });
        for k in ["null", "empty_str", "empty_arr", "no", "absent"] {
            assert!(!truthy(&v, k), "{k} should be falsy");
        }
        for k in ["str", "arr", "yes"] {
            assert!(truthy(&v, k), "{k} should be truthy");
        }
    }

    /// `""` is falsy in Python, so `items --record ''` rebuilds the artifacts
    /// rather than searching for a record named nothing.
    #[test]
    fn empty_string_arguments_count_as_absent() {
        let mut a = Args {
            query: Some(String::new()),
            record: Some(String::new()),
            family: Some(String::new()),
            unclassified: false,
            list: false,
            loose: false,
            rescan: false,
            table: PathBuf::from("/x"),
            json_out: None,
            doc_out: None,
        };
        a.normalise();
        assert!(a.query.is_none() && a.record.is_none() && a.family.is_none());
    }
}
