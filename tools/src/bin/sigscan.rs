//! Scan CrimsonDesert.exe for byte signatures and class/event names.
//!
//! ```text
//! sigscan [exe] [sigfile]
//! ```
//!
//! Defaults: the Steam install on /mnt/f and tools/reference-signatures.txt.
//! Signature lines are IDA-style: "48 8B ?? 20 01 00 00" (?? = wildcard).
//! Prints hit counts and FILE OFFSETS; a good anchor hits exactly once.
//!
//! Also checks every static-info type name in static-info-names.txt: each must
//! still occur exactly once as a NUL-delimited literal. A name that vanishes or
//! doubles after a game update means the type registry moved, which is an
//! earlier and louder warning than a byte signature going stale.
//!
//! Two things about this tool are load-bearing and easy to undo:
//!
//!   * **The offsets printed here are file offsets, not RVAs.** `xrefs` takes
//!     an RVA. Handing a number from this output straight to `xrefs` reports
//!     zero references, which looks like a bug and is not; `tools/README.md`
//!     says so at length. Convert through the PE section table
//!     (`desert_tools::pe`) first.
//!   * **Nothing here selects a section by name.** It does not need to - the
//!     scan is over the raw file - but if that ever changes, note that this
//!     exe's section names are scrambled and the 80 MB one holding the code is
//!     called `.idata`. See `tests/pe_real_exe.rs`.
//!
//! The matcher is `desert_tools::pattern`, shared with `retarget` since
//! 2026-11-01; it was private to this file before and moved unchanged, its
//! edge-case tests with it.
//!
//! Ported from `tools/sigscan.py`; stdout is byte-for-byte the same, including
//! the `:3d`/`:4d`/`:48s` column widths and the thousands separators in the
//! size line, because that is the only way to tell the port did not change an
//! answer.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use rayon::prelude::*;

use desert_tools::paths;
use desert_tools::pattern::Pattern;

/// Class, RTTI and event-name literals worth eyeballing after a game update.
///
/// These are a plain SUBSTRING check, unlike the static-info list below, and
/// the counts reflect that: `iteminfo` reports 2 because it also occurs inside
/// `trademarketiteminfo`. That is not a bug in either matcher - the two ask
/// different questions - and the difference is exactly why the static-info
/// check delimits on NUL.
const NAMES: [&str; 13] = [
    ".?AVClientActorManager@pa@@",
    "ClientActorManager",
    "ClientGimmickActorComponent",
    "ClientStatusActorComponent",
    "ClientAiActorComponent",
    "CameraManager",
    "TrocTrProcessLootingDeadDropOnceTimer",
    "TrocTrProcessPickUpItemOnceTimer",
    "TrocTrPushCharacterToInventoryOnceTimer",
    "TrocTrInteractionDoStepDoInteractionOnceTimer",
    "TrocTrStealItemByFrameEventOnceTimer",
    "gimmickinfo",
    "iteminfo",
];

#[derive(Parser)]
#[command(about = "Scan CrimsonDesert.exe for byte signatures and class/event names")]
struct Cli {
    /// The exe to scan (default: the Steam install, or $EXE / $CD_BIN64).
    exe: Option<PathBuf>,
    /// The signature list (default: tools/reference-signatures.txt).
    sigfile: Option<PathBuf>,
}

/// `375511960` -> `375,511,960`, the way Python's `{:,}` writes it.
fn with_commas(n: usize) -> String {
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

/// Lines of a data file that are neither blank nor comments.
///
/// `#` is tested on the RAW line, not the trimmed one, exactly as the Python
/// does. It reaches the same answer by a different route - an indented `#`
/// line survives this filter and is then rejected for not parsing - and
/// keeping the route identical is what stops a future data file with indented
/// comments from quietly meaning two different things to the two tools.
fn data_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| l.trim())
        .collect()
}

/// Every static-info type name must still occur exactly once, NUL-delimited.
///
/// Substring matching is not good enough here: several names contain each
/// other (`FactionInfo` inside `FactionInfoManager`, `iteminfo` inside
/// `trademarketiteminfo`), so the check is for a NUL on both sides. Adjacent
/// strings in a string table SHARE a terminator, so only the trailing NUL is
/// part of the pattern and the leading one is tested by hand - a pattern with
/// a NUL at both ends cannot match two adjacent names, because neither the
/// regex this was ported from nor `find_all` above will overlap two matches.
fn check_static_info_names(namefile: &std::path::Path, data: &[u8]) -> Result<()> {
    if !namefile.exists() {
        let name = namefile.file_name().unwrap_or_default().to_string_lossy();
        println!("  ({name} not found, skipped)");
        return Ok(());
    }
    let text = std::fs::read_to_string(namefile)
        .with_context(|| format!("cannot read {}", namefile.display()))?;
    let names = data_lines(&text);

    let counts: Vec<usize> = names
        .par_iter()
        .map(|n| {
            let mut pat = n.as_bytes().to_vec();
            pat.push(0);
            Pattern::literal(&pat)
                .find_all(data)
                .into_iter()
                .filter(|&s| s > 0 && data[s - 1] == 0)
                .count()
        })
        .collect();

    let once = counts.iter().filter(|&&c| c == 1).count();
    println!("{once:4}/{} type names still occur exactly once", names.len());
    for (n, &c) in names.iter().zip(counts.iter()) {
        if c == 0 {
            println!("       MISSING  {n}");
        }
    }
    for (n, &c) in names.iter().zip(counts.iter()) {
        if c > 1 {
            println!("       {c} HITS  {n}");
        }
    }
    if counts.iter().all(|&c| c == 1) {
        println!("       the static-info type registry is unchanged");
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // The Python resolves both data files relative to ITS OWN directory
    // (`Path(__file__).parent`), never to the cwd, so that `just sigscan` from
    // the repo root and a hand-run from anywhere read the same files. A
    // compiled binary has no `__file__`, so `repo_root()` - the nearest
    // ancestor holding both `justfile` and `flake.nix` - stands in for it, and
    // the compiled-in manifest directory (which IS `tools/`) is the last
    // resort. A bare relative `tools/` would have silently depended on the
    // cwd, which is the one thing the Python was careful not to do.
    let tools = paths::repo_root()
        .map(|r| r.join("tools"))
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let exe = cli.exe.unwrap_or_else(paths::game_exe);
    let sigfile = cli.sigfile.unwrap_or_else(|| tools.join("reference-signatures.txt"));
    let namefile = tools.join("static-info-names.txt");

    let sigtext = std::fs::read_to_string(&sigfile)
        .with_context(|| format!("cannot read the signature list {}", sigfile.display()))?;
    let sigs: Vec<(&str, Pattern)> = data_lines(&sigtext)
        .into_iter()
        .filter_map(|l| Pattern::parse(l).map(|p| (l, p)))
        .collect();

    // Read rather than mmap. The Python mmaps to keep 375 MB off the heap;
    // here the whole scan is one pass per pattern over a shared &[u8] and the
    // allocation is a single up-front read, which is simpler and no slower.
    let data = std::fs::read(&exe)
        .with_context(|| format!("cannot read the game exe {}", exe.display()))?;

    println!("{}", exe.display());
    println!("  size {} bytes", with_commas(data.len()));
    println!();
    println!("--- signatures ---");
    let sig_lines: Vec<String> = sigs
        .par_iter()
        .map(|(text, pat)| {
            let hits = pat.find_all(&data);
            let where_ = if hits.is_empty() {
                String::new()
            } else {
                let first: Vec<String> =
                    hits.iter().take(5).map(|h| format!("{h:#x}")).collect();
                format!("  @ {}", first.join(", "))
            };
            format!("{:3}  {text}{where_}", hits.len())
        })
        .collect();
    for l in sig_lines {
        println!("{l}");
    }

    println!();
    println!("--- names ---");
    let name_lines: Vec<String> = NAMES
        .par_iter()
        .map(|n| {
            let hits = Pattern::literal(n.as_bytes()).find_all(&data);
            let first = match hits.first() {
                Some(h) => format!("{h:#x}"),
                None => "-".to_string(),
            };
            format!("{:4}  {:48} first @ {first}", hits.len(), n)
        })
        .collect();
    for l in name_lines {
        println!("{l}");
    }

    println!();
    println!("--- static-info type names ---");
    check_static_info_names(&namefile, &data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_of_the_real_signature_file_parses() {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("reference-signatures.txt"),
        )
        .unwrap();
        let lines = data_lines(&text);
        let sigs: Vec<Pattern> = lines.iter().filter_map(|l| Pattern::parse(l)).collect();

        // Every data line must parse, and the count is deliberately NOT pinned
        // to a literal. An earlier version asserted "nine", which is exactly
        // the number of signatures the file had until the looter's hunting
        // hook (now in desert-harvest) added a tenth - a legitimate change that failed a test about
        // the parser. Comparing against the file's own line count keeps the
        // property the test is actually for (nothing is silently dropped) and
        // is strictly stronger: a pinned total still passes if one line stops
        // parsing while another is added.
        assert_eq!(
            sigs.len(),
            lines.len(),
            "{} of {} lines in reference-signatures.txt did not parse",
            lines.len() - sigs.len(),
            lines.len()
        );
        assert!(!sigs.is_empty(), "the signature file cannot be empty");
        // The `??` in the middle of the first one must survive parsing; a
        // tokeniser that dropped wildcards would still parse, and would then
        // match nothing.
        assert!(sigs.iter().any(|p| p.fixed.contains(&false)));
    }

    #[test]
    fn data_lines_drops_comments_and_blanks_only() {
        let text = "# a comment\n\n  \nAIMemoryInfo\n  Indented  \n#not a name\n";
        assert_eq!(data_lines(text), vec!["AIMemoryInfo", "Indented"]);
    }

    #[test]
    fn thousands_separators_match_pythons_format() {
        assert_eq!(with_commas(0), "0");
        assert_eq!(with_commas(999), "999");
        assert_eq!(with_commas(1000), "1,000");
        assert_eq!(with_commas(375_511_960), "375,511,960");
    }
}
