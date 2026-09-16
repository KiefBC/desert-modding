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
//! Ported from `tools/sigscan.py`; stdout is byte-for-byte the same, including
//! the `:3d`/`:4d`/`:48s` column widths and the thousands separators in the
//! size line, because that is the only way to tell the port did not change an
//! answer.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use memchr::memmem;
use rayon::prelude::*;

use desert_tools::paths;

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

/// One IDA-style signature: the bytes, plus which of them are wildcards.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pattern {
    bytes: Vec<u8>,
    /// `false` at a `??` position. Kept as a parallel vector rather than an
    /// `Option<u8>` per byte so the verify loop stays a pair of slice reads.
    fixed: Vec<bool>,
}

impl Pattern {
    /// Parse `"48 89 5C 24 ?? 4C"`. Returns `None` for anything that is not a
    /// whole line of two-hex-digit tokens and `??`, which is how comment and
    /// prose lines in `reference-signatures.txt` are skipped.
    fn parse(line: &str) -> Option<Pattern> {
        let mut bytes = Vec::new();
        let mut fixed = Vec::new();
        for tok in line.split(' ') {
            if tok == "??" {
                bytes.push(0);
                fixed.push(false);
            } else if tok.len() == 2 && tok.bytes().all(|b| b.is_ascii_hexdigit()) {
                bytes.push(u8::from_str_radix(tok, 16).ok()?);
                fixed.push(true);
            } else {
                return None;
            }
        }
        // The Python's SIG_RE needs at least two tokens, so a bare `48` on a
        // line is prose, not a signature. Keep that: a one-byte "signature"
        // would hit millions of times and drown the report.
        (bytes.len() >= 2).then_some(Pattern { bytes, fixed })
    }

    /// A plain literal, for the name lists.
    fn literal(bytes: &[u8]) -> Pattern {
        Pattern { bytes: bytes.to_vec(), fixed: vec![true; bytes.len()] }
    }

    /// The longest run of non-wildcard bytes, as (start, bytes).
    ///
    /// This is the run handed to `memmem`, and picking the LONGEST one rather
    /// than the first matters: several of these signatures start `48 89 5C 24`,
    /// which occurs by the hundred thousand in 375 MB, while the six-byte run
    /// in their middle occurs a handful of times. Scanning for the first run
    /// works and is thirty times slower.
    fn anchor(&self) -> (usize, &[u8]) {
        let (mut best_at, mut best_len) = (0usize, 0usize);
        let mut i = 0;
        while i < self.fixed.len() {
            if !self.fixed[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < self.fixed.len() && self.fixed[i] {
                i += 1;
            }
            if i - start > best_len {
                best_at = start;
                best_len = i - start;
            }
        }
        (best_at, &self.bytes[best_at..best_at + best_len])
    }

    fn matches_at(&self, hay: &[u8], at: usize) -> bool {
        let Some(win) = hay.get(at..at + self.bytes.len()) else {
            return false;
        };
        win.iter()
            .zip(self.bytes.iter().zip(self.fixed.iter()))
            .all(|(&h, (&b, &f))| !f || h == b)
    }

    /// Every offset the pattern matches at, NON-OVERLAPPING and left to right.
    ///
    /// Non-overlapping because the Python used `re.finditer`, which skips past
    /// a match before looking for the next one. For these signatures it makes
    /// no difference; for the NUL-delimited name check it very much does, and
    /// the two matchers share this function so the rule cannot drift apart
    /// between them.
    fn find_all(&self, hay: &[u8]) -> Vec<usize> {
        let (anchor_at, anchor) = self.anchor();
        if anchor.is_empty() {
            // An all-wildcard pattern matches everywhere; the data files have
            // no such line and a scan of 375 MB of hits helps nobody.
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut next = 0usize; // first offset a match is still allowed to start at
        let mut from = 0usize;
        while let Some(rel) = memmem::find(&hay[from..], anchor) {
            let pos = from + rel;
            // Step the anchor search by one, not by the anchor length: two
            // occurrences of the anchor can overlap even when two matches of
            // the whole pattern cannot, and `find_iter` would skip the second.
            from = pos + 1;
            let Some(start) = pos.checked_sub(anchor_at) else { continue };
            if start < next {
                continue;
            }
            if self.matches_at(hay, start) {
                out.push(start);
                next = start + self.bytes.len();
            }
        }
        out
    }
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
    fn parses_ida_style_signatures_and_rejects_prose() {
        let p = Pattern::parse("48 89 ?? 24").unwrap();
        assert_eq!(p.bytes, vec![0x48, 0x89, 0x00, 0x24]);
        assert_eq!(p.fixed, vec![true, true, false, true]);

        // Everything in reference-signatures.txt that is not a signature.
        assert!(Pattern::parse("alloc_event - function start").is_none());
        assert!(Pattern::parse("48").is_none(), "one token is not a signature");
        assert!(Pattern::parse("4 8 89").is_none(), "single hex digits are not bytes");
        assert!(Pattern::parse("48 8G").is_none());
        assert!(Pattern::parse("48  89").is_none(), "a double space is an empty token");
    }

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
        // the number of signatures the file had until desert-looter's hunting
        // hook added a tenth - a legitimate change that failed a test about
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
    fn anchor_is_the_longest_literal_run_not_the_first() {
        let p = Pattern::parse("48 89 ?? 57 48 83 EC 20 ?? BA").unwrap();
        let (at, a) = p.anchor();
        assert_eq!(at, 3);
        assert_eq!(a, &[0x57, 0x48, 0x83, 0xEC, 0x20]);

        // Leading wildcards must not push the anchor offset negative when the
        // match start is computed back from an anchor hit.
        let p = Pattern::parse("?? ?? 90 90 90").unwrap();
        assert_eq!(p.anchor(), (2, &[0x90, 0x90, 0x90][..]));
        let hay = [0x90u8, 0x90, 0x90, 0x00, 0x11, 0x90, 0x90, 0x90];
        // The run at offset 0 has no room for the two wildcard bytes in front
        // of it, so it is NOT a match; the run at 5 is, starting at 3.
        assert_eq!(p.find_all(&hay), vec![3]);
    }

    #[test]
    fn wildcards_match_any_byte_and_fixed_bytes_do_not() {
        let p = Pattern::parse("AA ?? CC").unwrap();
        let hay = [0xAA, 0x00, 0xCC, 0xAA, 0xFF, 0xCC, 0xAA, 0xFF, 0xCD];
        assert_eq!(p.find_all(&hay), vec![0, 3]);
    }

    #[test]
    fn a_match_running_off_the_end_is_not_a_match() {
        // The off-by-one that hides: the anchor is present, the rest of the
        // pattern is past the end of the buffer.
        let p = Pattern::parse("AA BB CC DD").unwrap();
        assert_eq!(p.find_all(&[0xAA, 0xBB, 0xCC]), Vec::<usize>::new());
        assert_eq!(p.find_all(&[0xAA, 0xBB, 0xCC, 0xDD]), vec![0]);
        // ... and at the very last possible offset it IS a match.
        assert_eq!(p.find_all(&[0x00, 0xAA, 0xBB, 0xCC, 0xDD]), vec![1]);
    }

    #[test]
    fn matches_do_not_overlap() {
        // `re.finditer` semantics: after a match, the search resumes at its
        // end. `ABAB` contains `ABA` twice if overlaps count and once if not.
        let p = Pattern::parse("41 42 41").unwrap();
        assert_eq!(p.find_all(b"ABABA"), vec![0]);
        assert_eq!(p.find_all(b"ABABABABA"), vec![0, 4]);
    }

    #[test]
    fn overlapping_anchor_runs_do_not_hide_a_match() {
        // The anchor `AA AA` occurs at offsets 3 AND 4 - overlapping - and only
        // the second one has the pattern's `01` two bytes in front of it.
        // Stepping the anchor search by the anchor's length would resume at 5,
        // never see the occurrence at 4, and report no match at all.
        let p = Pattern::parse("01 ?? AA AA").unwrap();
        assert_eq!(p.anchor(), (2, &[0xAA, 0xAA][..]));
        assert_eq!(p.find_all(&[0x00, 0x00, 0x01, 0xAA, 0xAA, 0xAA, 0x00]), vec![2]);
    }

    #[test]
    fn nul_delimited_is_not_the_same_as_substring() {
        // The `iteminfo` case from tools/README.md, in miniature: a string
        // table holding `trademarketiteminfo` and `iteminfo` back to back,
        // sharing the terminator between them.
        let table = b"\0trademarketiteminfo\0iteminfo\0";
        let substring = Pattern::literal(b"iteminfo").find_all(table);
        assert_eq!(substring.len(), 2, "substring finds it inside the longer name too");

        let mut delim = b"iteminfo".to_vec();
        delim.push(0);
        let nul: Vec<usize> = Pattern::literal(&delim)
            .find_all(table)
            .into_iter()
            .filter(|&s| s > 0 && table[s - 1] == 0)
            .collect();
        assert_eq!(nul.len(), 1, "NUL-delimited finds only the standalone name");
        assert_eq!(nul[0], 21);
    }

    #[test]
    fn a_name_at_offset_zero_is_not_counted() {
        // There is no leading NUL to test, and the check must not read
        // `data[-1]`. The Python guards with `x.start() > 0` for exactly this.
        let table = b"iteminfo\0";
        let mut delim = b"iteminfo".to_vec();
        delim.push(0);
        let nul: Vec<usize> = Pattern::literal(&delim)
            .find_all(table)
            .into_iter()
            .filter(|&s| s > 0 && table[s - 1] == 0)
            .collect();
        assert!(nul.is_empty());
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
