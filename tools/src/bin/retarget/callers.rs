//! `retarget callers <rva>`: every reference to an address, grouped by the
//! function it is in.
//!
//! `xrefs` lists references one per line; for a function with sixty callers
//! that is a page of offsets with no structure. Grouped by enclosing function
//! it becomes a short list of functions, each with its first 24 bytes and an
//! auto-wildcarded pattern, which is what a signature for "the caller of X"
//! is cut from. The enclosing function comes from the exception directory
//! when it covers the site, from the prologue heuristic otherwise, and each
//! row says which.
//!
//! A `REX` RIP-relative instruction is reported twice by the scanner (once at
//! the REX byte, once at the opcode; see `xrefs`); here it counts once.

use std::collections::BTreeMap;
use std::io::Write;

use anyhow::Result;

use desert_tools::funcs::StartSource;
use desert_tools::scan::{self, Xref};

use crate::calls::pattern_at;
use crate::{hex_bytes, Exe};

/// Bytes of each calling function printed and cut into a pattern.
const HEAD: usize = 24;

pub struct Group {
    pub start: Option<u32>,
    pub source: StartSource,
    pub refs: Vec<Xref>,
    pub head: Vec<u8>,
    pub pattern: String,
    pub count: usize,
}

/// One reference per instruction: the `lea32` half of a `lea`/`lea32` pair
/// at adjacent offsets is dropped.
pub fn dedup_rex(mut hits: Vec<Xref>) -> Vec<Xref> {
    hits.sort();
    let mut out: Vec<Xref> = Vec::with_capacity(hits.len());
    for h in hits {
        if let Some(prev) = out.last() {
            if prev.at + 1 == h.at && h.kind.contains("32") && !prev.kind.contains("32") {
                continue;
            }
        }
        out.push(h);
    }
    out
}

pub fn collect(exe: &Exe, rva: u32) -> (Vec<Group>, usize) {
    let hits = dedup_rex(scan::fast_scan(&exe.image, rva as u64));
    let cells = scan::pointer_cells(&exe.image, exe.base + rva as u64).len();
    let mut by: BTreeMap<(Option<u32>, u8), (StartSource, Vec<Xref>)> = BTreeMap::new();
    for h in hits {
        let (start, src) = exe.function_start(h.at as u32);
        let key = (start, matches!(src, StartSource::Heuristic) as u8);
        by.entry(key).or_insert_with(|| (src, Vec::new())).1.push(h);
    }
    let groups = by
        .into_iter()
        .map(|((start, _), (source, refs))| {
            let (head, pattern, count) = match start {
                Some(s) => {
                    let (p, c) = pattern_at(exe, s, HEAD);
                    (exe.bytes(s, HEAD).to_vec(), p, c)
                }
                None => (Vec::new(), String::new(), 0),
            };
            Group { start, source, refs, head, pattern, count }
        })
        .collect();
    (groups, cells)
}

pub fn run(out: &mut impl Write, exe: &Exe, rva: u32, tsv: bool) -> Result<()> {
    let (groups, cells) = collect(exe, rva);
    let refs: usize = groups.iter().map(|g| g.refs.len()).sum();
    let pdata = groups.iter().filter(|g| g.source == StartSource::Pdata).count();
    if tsv {
        writeln!(out, "function\tsource\trefs\tsites\tbytes\tpattern\tcount")?;
        for g in &groups {
            let sites: Vec<String> = g.refs.iter().map(|x| format!("{:#x}:{}", x.at, x.kind)).collect();
            writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                g.start.map(|s| format!("{s:#x}")).unwrap_or_else(|| "?".into()),
                g.source.label(),
                g.refs.len(),
                sites.join(","),
                hex_bytes(&g.head),
                g.pattern,
                g.count
            )?;
        }
        return Ok(());
    }
    writeln!(out, "callers of +{rva:#x} ({}):", exe.pdata_status(rva))?;
    for g in &groups {
        let start = g.start.map(|s| format!("+{s:#x}")).unwrap_or_else(|| "?".into());
        writeln!(out, "  {start} [{}]  {} ref(s)", g.source.label(), g.refs.len())?;
        for x in &g.refs {
            let off = g.start.map(|s| format!(" (+{:#x})", x.at as u32 - s.min(x.at as u32))).unwrap_or_default();
            writeln!(out, "      {:<14} at +{:#x}{off}", x.kind, x.at)?;
        }
        if !g.head.is_empty() {
            writeln!(out, "      bytes   {}", hex_bytes(&g.head))?;
            writeln!(out, "      pattern {}  ({} hit(s) exe-wide)", g.pattern, g.count)?;
        }
    }
    writeln!(
        out,
        "{refs} code reference(s) in {} function(s) ({pdata} from pdata, {} by heuristic), {cells} pointer cell(s)",
        groups.len(),
        groups.len() - pdata
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rex_pair_counts_once_and_separate_hits_stay() {
        let x = |at, kind| Xref { at, kind, target: 0 };
        let v = dedup_rex(vec![x(0x10, "lea"), x(0x11, "lea32"), x(0x20, "call"), x(0x30, "lea32")]);
        let kinds: Vec<&str> = v.iter().map(|x| x.kind).collect();
        assert_eq!(kinds, ["lea", "call", "lea32"]);
    }
}
