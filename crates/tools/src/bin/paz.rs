//! Read the game's `.pamt`/`.paz` archives offline.
//!
//! ```text
//! paz list [--group G] [--prefix P]     tsv: group, path, size, comp_size, flags
//! paz cat <path> -o FILE                one decoded file
//! paz extract-tables [--out DIR]        every static-info body and header
//! ```
//!
//! `--game-dir` on every subcommand, default `CD_GAME_DIR` or `bin64/..`.
//! Encrypted entries are refused, never written as ciphertext; the format and
//! the flags are in `desert_tools::paz` and `analysis/FINDINGS.md` section 7.

use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use desert_tools::paths;
use desert_tools::paz::{Archive, DEFAULT_GROUP};
use desert_tools::staticinfo::{BODY_EXT, HEADER_EXT, TABLE_DIR};

#[derive(Parser)]
#[command(about = "Read the game's .pamt/.paz archives offline")]
struct Cli {
    /// The game's install directory (holding `<group>/0.pamt`).
    #[arg(long, global = true)]
    game_dir: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List entries as tsv: group, path, size, comp_size, flags (`0x` then upper-case hex, as `analysis/paz-listing-*.tsv` has it).
    List {
        /// Only this group (default: every group that parses).
        #[arg(long)]
        group: Option<String>,
        /// Only paths starting with this.
        #[arg(long)]
        prefix: Option<String>,
    },
    /// Write one decoded file.
    Cat {
        /// The full archive path, `dir/name`.
        path: String,
        /// Where to write it.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Extract every static-info body and header from the archive.
    ExtractTables {
        /// Default `<repo>/analysis/tables`.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let game_dir = cli.game_dir.unwrap_or_else(paths::game_dir);
    let archive = Archive::open(&game_dir)?;
    match cli.cmd {
        Cmd::List { group, prefix } => {
            let names: Vec<String> = match group {
                Some(g) => vec![g],
                None => archive.group_names().to_vec(),
            };
            let stdout = std::io::stdout();
            let mut w = BufWriter::new(stdout.lock());
            for name in &names {
                let g = match archive.group(name) {
                    Ok(g) => g,
                    Err(e) if names.len() > 1 => {
                        eprintln!("skipping group {name}: {e:#}");
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                for (file, disk, indexed) in &g.oversize {
                    eprintln!("note: {} is {disk} bytes, the index says {indexed} (entries all lie inside)", file.display());
                }
                for e in g.entries() {
                    let path = e.path();
                    if prefix.as_deref().is_some_and(|p| !path.starts_with(p)) {
                        continue;
                    }
                    writeln!(w, "{name}\t{path}\t{}\t{}\t0x{:X}", e.size, e.comp_size, e.flags)?;
                }
            }
            w.flush()?;
        }
        Cmd::Cat { path, out } => {
            let bytes = archive.read(&path)?;
            std::fs::write(&out, &bytes).with_context(|| format!("cannot write {}", out.display()))?;
            eprintln!("{path}: {} bytes -> {}", bytes.len(), out.display());
        }
        Cmd::ExtractTables { out } => {
            let out = match out {
                Some(o) => o,
                None => paths::repo_root()?.join("analysis/tables"),
            };
            std::fs::create_dir_all(&out).with_context(|| format!("cannot create {}", out.display()))?;
            let started = Instant::now();
            let g = archive.group(DEFAULT_GROUP)?;
            let mut count = 0usize;
            let mut bytes = 0usize;
            for e in g.entries() {
                if e.dir != TABLE_DIR || !(e.name.ends_with(BODY_EXT) || e.name.ends_with(HEADER_EXT)) {
                    continue;
                }
                let data = g.read(e)?;
                let file = out.join(&e.name);
                std::fs::write(&file, &data).with_context(|| format!("cannot write {}", file.display()))?;
                count += 1;
                bytes += data.len();
            }
            println!(
                "extracted {count} static-info files ({bytes} bytes) from group {DEFAULT_GROUP} into {} in {:.2}s",
                out.display(),
                started.elapsed().as_secs_f64()
            );
        }
    }
    Ok(())
}
