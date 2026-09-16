//! Bulk-export Ghidra evidence for the game functions this project cares about.
//!
//! Walks the call graph outward from a set of anchor functions and writes one
//! Markdown file per function containing its signature, decompilation,
//! disassembly, callers and callees. The result is a local, greppable snapshot
//! of what the Windows Ghidra knows, so a session can answer "who else touches
//! this" with `grep` instead of a few dozen MCP round-trips.
//!
//! ```text
//! evidence                                  # depth 1 from the default anchors
//! evidence --depth 2 --max-functions 3000
//! evidence --anchor 1403856b0 --depth 2 --out /tmp/loader
//! ```
//!
//! Default anchors are every game-exe `FUN_1xxxxxxxx` named in `docs/*.md` or
//! already dumped in `analysis/*.c` - i.e. everything we have ever written
//! about. Addresses outside the game module ([0x140000000, 0x160000000)) are
//! dropped, so the dead CDLoot.asi names at 0x180000000 do not become anchors.
//! Both directories are found through `paths::repo_root()`, not through the
//! working directory: `just evidence` runs this from the repo root and a
//! session runs it from wherever it happens to be, and the anchor set has to
//! be the same either way.
//!
//! Needs the Windows Ghidra running with the GhidraMCP extension listening (the
//! same server the `ghidra` MCP bridge talks to; this speaks its HTTP API
//! directly, so it works whether or not the MCP client is connected). The host
//! is probed on 127.0.0.1 then the WSL default gateway, matching
//! `~/.local/bin/ghidra-mcp-bridge-win`.
//!
//! Output is a snapshot of one game build and goes stale on an update. It is
//! regenerable and belongs beside `analysis/` in .gitignore, never in a commit.
//! Re-run after an update and diff the two trees: the functions whose
//! decompilation moved are the candidate breakage list.
//!
//! Resumable: a function whose file already exists is not re-fetched (its graph
//! edges are read back out of the file's metadata line). Pass --force to redo.
//!
//! Ported from `tools/evidence.py`. Every byte it wrote - the per-function
//! files, `index.tsv`, `graph.json` and the tree's README - was what the Python
//! wrote, so that an existing tree could be topped up by either one and the two
//! diffed against each other. Four strings were held at the Python's spelling
//! for exactly that long: the two in-band notices, now `[evidence: ...]`, and
//! the two references in the generated README, now `just evidence`. The Python
//! is gone and those anchors went with it, so a tree exported before the port
//! differs from one exported after in those four places and nowhere else.

#[path = "evidence/ghidra.rs"]
mod ghidra;
#[path = "evidence/meta.rs"]
mod meta;
#[path = "evidence/parse.rs"]
mod parse;
#[path = "evidence/record.rs"]
mod record;
#[path = "evidence/render.rs"]
mod render;
#[path = "evidence/tree.rs"]
mod tree;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;

use ghidra::Ghidra;
use meta::Meta;
use record::Limits;

#[derive(Parser)]
#[command(
    name = "evidence",
    about = "Export a bounded Ghidra evidence tree around anchor functions."
)]
struct Cli {
    /// anchor address in hex (repeatable); default: every FUN_ named in docs/ or analysis/
    #[arg(long = "anchor", value_name = "ADDR")]
    anchor: Vec<String>,

    /// file of anchor addresses, one hex address per line ('#' comments and
    /// blank lines ignored); combines with --anchor. Use it when the anchor set
    /// is a few hundred addresses and will not fit on a command line.
    #[arg(long, value_name = "PATH")]
    anchors_file: Option<PathBuf>,

    /// call-graph hops from the anchors
    ///
    /// `allow_negative_numbers` because argparse took this as a plain `int`:
    /// `--depth -1` walks nothing at all and just rebuilds `index.tsv` and
    /// `graph.json` from the tree on disk, and without this clap rejects the
    /// `-1` as an unknown flag.
    #[arg(long, default_value_t = 1, allow_negative_numbers = true)]
    depth: i64,

    /// hard cap on functions written
    #[arg(long, default_value_t = 1500)]
    max_functions: usize,

    /// do not expand through a function with more than N callers; its callers
    /// are still recorded. This is what stops the walk drowning in hub
    /// functions like operator new.
    #[arg(long, value_name = "N", default_value_t = 40)]
    max_callers_expand: usize,

    /// skip a function whose body spans more than N bytes. Ghidra's
    /// auto-analysis glues runs of unanalysed bytes into single bogus
    /// 'functions' - one here spans 222MB and reports 14994 callees - and both
    /// its disassembly and its callee list poison the walk.
    #[arg(long, value_name = "N", default_value = "0x100000", value_parser = py_int)]
    max_body_bytes: u64,

    /// truncate a decompilation or disassembly longer than N bytes. A backstop
    /// for the above.
    #[arg(long, value_name = "N", default_value = "2097152", value_parser = py_int)]
    max_section_bytes: u64,

    /// max caller references fetched per function
    #[arg(long, default_value_t = 200)]
    caller_limit: usize,

    /// parallel requests
    #[arg(long, default_value_t = 4)]
    jobs: usize,

    /// per-request seconds
    #[arg(long, default_value_t = 180)]
    timeout: u64,

    /// output directory [default: <repo>/evidence]
    #[arg(long)]
    out: Option<PathBuf>,

    /// e.g. http://172.23.144.1:8081 (default: probe)
    #[arg(long)]
    host: Option<String>,

    /// GhidraMCP port [default: 8081, or $GHIDRA_MCP_PORT]
    #[arg(long)]
    port: Option<u32>,

    /// re-fetch existing files
    #[arg(long)]
    force: bool,

    /// print the anchor set and exit without contacting Ghidra
    #[arg(long)]
    dry_run: bool,
}

/// argparse took these two as `int(x, 0)`, so `0x100000` and `1048576` are both
/// spellings of the same default and both appear in this project's notes.
fn py_int(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let (radix, digits) = match t.get(..2).map(str::to_ascii_lowercase).as_deref() {
        Some("0x") => (16, &t[2..]),
        Some("0o") => (8, &t[2..]),
        Some("0b") => (2, &t[2..]),
        _ => (10, t),
    };
    let cleaned: String = digits.chars().filter(|c| *c != '_').collect();
    u64::from_str_radix(&cleaned, radix).map_err(|e| format!("invalid int value '{s}': {e}"))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("evidence: {e:#}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let root = desert_tools::paths::repo_root()?;

    let mut given: Vec<String> = cli.anchor.clone();
    if let Some(file) = &cli.anchors_file {
        match std::fs::read_to_string(file) {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.split('#').next().unwrap_or("").trim();
                    if !line.is_empty() {
                        given.push(line.to_string());
                    }
                }
            }
            Err(e) => {
                eprintln!("evidence: cannot read {}: {e}", file.display());
                return Ok(ExitCode::from(1));
            }
        }
    }

    let mut anchors = Vec::new();
    if given.is_empty() {
        anchors = parse::default_anchors(&root);
    } else {
        for a in &given {
            match parse::py_hex(a) {
                Some(va) => anchors.push(va),
                None => {
                    eprintln!("evidence: bad anchor address: {a}");
                    return Ok(ExitCode::from(1));
                }
            }
        }
    }
    // Not de-duplicated, only filtered: a repeated --anchor is counted twice in
    // the "N anchor(s)" line and listed twice in graph.json, as it was in the
    // Python. Only the frontier is de-duplicated, which is where it matters.
    anchors.retain(|a| parse::in_module(*a));
    if anchors.is_empty() {
        eprintln!("evidence: no anchors (nothing in docs/ or analysis/, and none given)");
        return Ok(ExitCode::from(1));
    }
    eprintln!(
        "evidence: {} anchor(s), depth {}, cap {}",
        anchors.len(),
        cli.depth,
        cli.max_functions
    );
    if cli.dry_run {
        let mut out = String::new();
        for a in &anchors {
            out.push_str(&format!("{a:x}\n"));
        }
        print!("{out}");
        return Ok(ExitCode::SUCCESS);
    }

    let port = match cli.port {
        Some(p) => p,
        None => std::env::var("GHIDRA_MCP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8081),
    };
    let host_arg = cli.host.clone().or_else(|| std::env::var("GHIDRA_MCP_HOST").ok());
    let Some(host) = ghidra::discover_host(host_arg.as_deref(), port) else {
        eprintln!(
            "evidence: no GhidraMCP server answering on port {port} \
             (127.0.0.1 or the default gateway). Is Ghidra running?"
        );
        return Ok(ExitCode::from(1));
    };
    eprintln!("evidence: using {host}");

    let out = cli.out.clone().unwrap_or_else(|| root.join("evidence"));
    let fndir = out.join("fn");
    std::fs::create_dir_all(&fndir)
        .with_context(|| format!("creating {}", fndir.display()))?;
    let g = Ghidra::new(host, Duration::from_secs(cli.timeout));
    let limits = Limits {
        caller_limit: cli.caller_limit,
        max_body: cli.max_body_bytes,
        max_section: usize::try_from(cli.max_section_bytes).unwrap_or(usize::MAX),
    };

    let anchor_set: HashSet<u64> = anchors.iter().copied().collect();
    let mut nodes: HashMap<u64, Option<Meta>> = HashMap::new();
    let mut frontier = dedup(anchors.clone());
    let started = Instant::now();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(cli.jobs.max(1))
        .build()
        .context("building the request thread pool")?;

    for depth in 0..=cli.depth {
        frontier.retain(|va| !nodes.contains_key(va));
        if frontier.is_empty() {
            break;
        }
        // The cap is hard, not advisory. Depth 1 from the default anchors lands
        // around 1500 functions; depth 2 does not, and without a cap the first
        // person to try it fills a disk.
        let room = cli.max_functions as i64 - nodes.len() as i64;
        if room <= 0 {
            eprintln!("evidence: cap of {} reached, stopping", cli.max_functions);
            break;
        }
        if frontier.len() as i64 > room {
            eprintln!(
                "evidence: depth {depth} has {} functions, taking {room} to stay under the cap",
                frontier.len()
            );
            frontier.truncate(room as usize);
        }
        eprintln!("evidence: depth {depth}: {} function(s)", frontier.len());

        let total = frontier.len();
        // Progress is counted in frontier order, not completion order, and is
        // reported from inside the workers. Both halves of that are needed: the
        // Python's `ex.map` yielded in submission order, so counting by
        // completion would make the numbers depend on which of --jobs requests
        // came back first; and reporting from an ordered loop after the level
        // finished would print a 1500-function level's progress in one burst at
        // the end, twenty-five minutes after it stopped being useful. Each
        // worker drains whatever prefix of the level is now complete, under the
        // lock, so the lines stay in order and cannot interleave.
        let progress: Mutex<(Vec<Option<bool>>, usize)> = Mutex::new((vec![None; total], 0));
        let results: Vec<Result<(u64, Option<Meta>)>> = pool.install(|| {
            frontier
                .par_iter()
                .enumerate()
                .map(|(i, &va)| {
                    let got = work(&g, &fndir, va, depth, cli.force, &anchor_set, limits);
                    let was_function = matches!(got, Ok((_, Some(_))));
                    let mut p = progress.lock().expect("no worker panics under this lock");
                    p.0[i] = Some(was_function);
                    let mut at = p.1;
                    while at < total && p.0[at].is_some() {
                        let done = at + 1;
                        let reported = p.0[at] == Some(true);
                        at = done;
                        // The Python `continue`d past this line for an address
                        // that turned out not to be a function, so a level
                        // whose 25th or last address is not a function prints
                        // no line for it - and a level ending in one prints no
                        // final progress line at all. Reproduced rather than
                        // fixed, because the trees have to diff clean while
                        // both implementations exist; see the port notes.
                        if reported && (done.is_multiple_of(25) || done == total) {
                            eprintln!(
                                "  {done}/{total} ({} requests, {} failed)",
                                g.calls(),
                                g.failures()
                            );
                        }
                    }
                    p.1 = at;
                    drop(p);
                    got
                })
                .collect()
        });
        for r in results {
            let (va, m) = r?;
            // A `None` is remembered too: it means Ghidra says there is no
            // function at that address, and without the entry the next level
            // would ask again for every caller that reaches it.
            nodes.insert(va, m);
        }

        if depth == cli.depth {
            break;
        }
        let mut next = Vec::new();
        for va in &frontier {
            let Some(Some(m)) = nodes.get(va) else { continue };
            let caller_entries: Vec<u64> =
                if m.callers.len() > cli.max_callers_expand || m.callers_truncated {
                    // A hub. Its callers are recorded in its own file; expanding
                    // through them is what turns a walk into the whole program -
                    // depth 2 finds `operator new` and the walk becomes all
                    // 250k functions.
                    Vec::new()
                } else {
                    m.callers.iter().filter_map(|c| parse::entry_of(&c.name)).collect()
                };
            for t in m.callees.iter().chain(&m.tailcalls).chain(&caller_entries) {
                if !nodes.contains_key(t) {
                    next.push(*t);
                }
            }
        }
        frontier = dedup(next);
    }

    // Built from every file in the tree, not just this run's walk.
    let mut real: BTreeMap<u64, Meta> = BTreeMap::new();
    for path in md_files(&fndir)? {
        if let Some(m) = meta::read_meta(&path) {
            real.insert(m.va, m);
        }
    }
    let mut walked = 0usize;
    let mut missing = 0usize;
    for m in nodes.values() {
        match m {
            // This run wins on anything it refreshed.
            Some(m) => {
                walked += 1;
                real.insert(m.va, m.clone());
            }
            None => missing += 1,
        }
    }
    let today = tree::today();
    tree::write_tree(
        &real,
        &tree::Summary {
            out: &out,
            anchors: &anchors,
            anchor_set: &anchor_set,
            walked,
            depth: cli.depth,
            max_functions: cli.max_functions,
            today: &today,
        },
    )?;

    eprintln!(
        "evidence: {walked} function(s) this run, {} in {} ({missing} address(es) were not \
         functions, {} requests, {} failed, {:.0}s)",
        real.len(),
        out.display(),
        g.calls(),
        g.failures(),
        started.elapsed().as_secs_f64()
    );
    Ok(ExitCode::SUCCESS)
}

/// One function: cached, fetched, or not a function at all.
fn work(
    g: &Ghidra,
    fndir: &std::path::Path,
    va: u64,
    depth: i64,
    force: bool,
    anchors: &HashSet<u64>,
    limits: Limits,
) -> Result<(u64, Option<Meta>)> {
    let path = fndir.join(format!("{va:x}.md"));
    if path.exists() && !force {
        // The whole point of the metadata line: the edges come back without a
        // request, so a re-run after an interruption costs seconds. A file
        // whose line is unreadable falls through and is re-fetched.
        if let Some(m) = meta::read_meta(&path) {
            return Ok((va, Some(m)));
        }
    }
    let Some(rec) = record::fetch_function(g, va, limits) else {
        return Ok((va, None));
    };
    let note = if anchors.contains(&va) { " (anchor)" } else { "" };
    // Written to a sibling and renamed: a run killed halfway through a 2 MiB
    // disassembly would otherwise leave a file whose metadata line is intact
    // and whose body is half there, and the next run would trust it.
    let tmp = fndir.join(format!("{va:x}.md.tmp"));
    std::fs::write(&tmp, render::render(&rec, depth, note))
        .with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("renaming to {}", path.display()))?;
    Ok((va, Some(rec.meta(depth))))
}

/// First-seen order, no repeats - Python's `list(dict.fromkeys(xs))`.
fn dedup(xs: Vec<u64>) -> Vec<u64> {
    let mut seen = HashSet::new();
    xs.into_iter().filter(|x| seen.insert(*x)).collect()
}

fn md_files(dir: &std::path::Path) -> Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("listing {}", dir.display()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        // `*.md`, so the `.md.tmp` a killed run may have left behind is not
        // mistaken for an exported function.
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    out.sort();
    Ok(out)
}
