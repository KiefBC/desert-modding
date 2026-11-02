//! `retarget triage`: which signatures still identify one place, and what is
//! left of the ones that do not.
//!
//! A dead signature is rarely dead all the way through. A recompile that moved
//! one register or one displacement leaves the bytes before the change and the
//! bytes after it intact, and the longest prefix and suffix that still hit say
//! where the change is and, usually, where the function went. Hit count is
//! monotone in length (a longer prefix can only match where a shorter one
//! does), so both are a binary search over length, a handful of scans each.
//!
//! Uniqueness is counted with the overlapping finder, as the plugin's matcher
//! counts it (`desert_tools::pattern` module docs).

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use rayon::prelude::*;

use desert_tools::pattern::Pattern;

use crate::sigfile::{self, Sig};
use crate::Exe;

/// RVAs printed per surviving prefix or suffix.
const SHOW: usize = 8;
/// Hits collected for a surviving prefix or suffix before it is reported as
/// "at least this many".
const CAP: usize = 10_000;

/// What is left of a dead signature at one end.
#[derive(Debug, Clone)]
pub struct Remnant {
    /// How many bytes of the signature still hit.
    pub len: usize,
    /// Where, capped at [`CAP`].
    pub hits: Vec<usize>,
    /// The start of the whole signature each hit implies (the hit itself for
    /// a prefix, the hit minus the cut-off bytes for a suffix).
    pub starts: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub sig: Sig,
    pub hits: Vec<usize>,
    pub prefix: Option<Remnant>,
    pub suffix: Option<Remnant>,
}

impl Row {
    pub fn status(&self) -> &'static str {
        match self.hits.len() {
            1 => "ok",
            0 if self.sig.optional => "dead (optional)",
            0 => "DEAD",
            _ => "AMBIGUOUS",
        }
    }
}

/// Whether any of `p` hits. A cut with no fixed byte left matches everywhere,
/// and counts as hitting: that keeps the predicate monotone.
fn hits_any(img: &[u8], p: &Pattern) -> bool {
    !p.has_fixed() || !p.find_overlapping_capped(img, 1).is_empty()
}

/// The longest cut (prefix or suffix, by `cut`) with at least one hit.
fn longest(img: &[u8], full: &Pattern, cut: impl Fn(usize) -> Pattern) -> Option<usize> {
    // hits_any(cut(lo)) holds, hits_any(cut(hi)) does not: the full pattern
    // is dead when this is called.
    let (mut lo, mut hi) = (0usize, full.len());
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if hits_any(img, &cut(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    // A remnant of all wildcards says nothing.
    (lo > 0 && cut(lo).has_fixed()).then_some(lo)
}

fn remnant(img: &[u8], full: &Pattern, prefix: bool) -> Option<Remnant> {
    let len = if prefix {
        longest(img, full, |n| full.prefix(n))?
    } else {
        longest(img, full, |n| full.suffix(n))?
    };
    let p = if prefix { full.prefix(len) } else { full.suffix(len) };
    let hits = p.find_overlapping_capped(img, CAP);
    let back = if prefix { 0 } else { full.len() - len };
    let starts = hits.iter().filter_map(|h| h.checked_sub(back)).collect();
    Some(Remnant { len, hits, starts })
}

pub fn triage(exe: &Exe, sigs: Vec<Sig>) -> Vec<Row> {
    let img = &exe.image;
    sigs.into_par_iter()
        .map(|sig| {
            let hits = sig.pattern.find_all_overlapping(img);
            let (prefix, suffix) = if hits.is_empty() {
                (remnant(img, &sig.pattern, true), remnant(img, &sig.pattern, false))
            } else {
                (None, None)
            };
            Row { sig, hits, prefix, suffix }
        })
        .collect()
}

pub fn run(exe: &Exe, path: &Path) -> Result<Vec<Row>> {
    Ok(triage(exe, sigfile::read(path)?))
}

fn rvas(v: &[usize]) -> String {
    let shown: Vec<String> = v.iter().take(SHOW).map(|h| format!("+{h:#x}")).collect();
    let more = if v.len() > SHOW { format!(" (+{} more)", v.len() - SHOW) } else { String::new() };
    format!("{}{more}", shown.join(" "))
}

fn count(r: &Remnant) -> String {
    if r.hits.len() >= CAP {
        format!(">={CAP}")
    } else {
        r.hits.len().to_string()
    }
}

pub fn print(out: &mut impl Write, exe: &Exe, rows: &[Row], tsv: bool) -> Result<()> {
    if tsv {
        writeln!(out, "name\tline\tlen\thits\tstatus\trvas\tprefix_len\tprefix_hits\tprefix_starts\tsuffix_len\tsuffix_hits\tsuffix_starts")?;
        for r in rows {
            let rem = |x: &Option<Remnant>| match x {
                Some(x) => format!(
                    "{}\t{}\t{}",
                    x.len,
                    count(x),
                    x.starts.iter().take(SHOW).map(|h| format!("{h:#x}")).collect::<Vec<_>>().join(",")
                ),
                None => "\t\t".to_string(),
            };
            writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                r.sig.name,
                r.sig.line,
                r.sig.pattern.len(),
                r.hits.len(),
                r.status(),
                r.hits.iter().take(SHOW).map(|h| format!("{h:#x}")).collect::<Vec<_>>().join(","),
                rem(&r.prefix),
                rem(&r.suffix)
            )?;
        }
        return Ok(());
    }
    writeln!(out, "triage: {} signature(s) against {}", rows.len(), exe.path.display())?;
    for r in rows {
        let at = match r.hits.first() {
            Some(&h) if r.hits.len() == 1 => format!("+{h:#x} (file {:#x})", exe.rva_to_offset(h as u32).unwrap_or(0)),
            _ if r.hits.len() > 1 => rvas(&r.hits),
            _ => format!("({} bytes)", r.sig.pattern.len()),
        };
        writeln!(out, "  {:<16} {:>3}  {:<22} {at}", r.status(), r.hits.len(), r.sig.name)?;
        if let Some(p) = &r.prefix {
            writeln!(
                out,
                "        longest prefix {:>2}/{} bytes, {} hit(s): {}",
                p.len,
                r.sig.pattern.len(),
                count(p),
                rvas(&p.hits)
            )?;
            writeln!(out, "          {}", r.sig.pattern.prefix(p.len))?;
        }
        if let Some(s) = &r.suffix {
            writeln!(
                out,
                "        longest suffix {:>2}/{} bytes, {} hit(s): {}",
                s.len,
                r.sig.pattern.len(),
                count(s),
                rvas(&s.hits)
            )?;
            writeln!(out, "          {}", r.sig.pattern.suffix(s.len))?;
            writeln!(out, "          implied signature start {}", rvas(&s.starts))?;
        }
        if r.hits.is_empty() && r.prefix.is_none() && r.suffix.is_none() {
            writeln!(out, "        nothing of it hits at either end")?;
        }
    }
    let ok = rows.iter().filter(|r| r.hits.len() == 1).count();
    let dead = rows.iter().filter(|r| r.hits.is_empty()).count();
    let amb = rows.iter().filter(|r| r.hits.len() > 1).count();
    writeln!(out, "summary: {ok} ok, {dead} dead, {amb} ambiguous")?;
    Ok(())
}
