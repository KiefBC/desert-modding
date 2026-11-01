//! `retarget calls <rva> <len>`: every `E8`/`E9` target in a range.
//!
//! The question it answers is "which function is the call at +0x11 of this
//! site", for every call in a window at once, with enough beside each target
//! to cut a signature for it: whether the exception directory has it as a
//! function start, its first bytes, those bytes auto-wildcarded, and how many
//! times that pattern occurs in the whole image. `--callers` adds how many
//! distinct functions call each target, one more pass for all of them
//! together; a hub like `alloc_event` and a one-caller helper look nothing
//! alike there.
//!
//! A target that is an import thunk (`jmp [rip+slot]` into the IAT) is printed
//! by its import name instead.

use std::collections::{BTreeSet, HashSet};
use std::io::Write;

use anyhow::Result;

use desert_tools::disasm::{self, Insn};
use desert_tools::scan;

use crate::{hex_bytes, Exe};

pub struct Target {
    pub site: u32,
    pub kind: &'static str,
    pub target: u32,
    pub status: String,
    pub import: Option<String>,
    pub head: Vec<u8>,
    pub pattern: String,
    pub count: usize,
    pub callers: Option<(usize, usize)>,
}

/// The import name of a `jmp [rip+slot]` thunk at `rva`, if it is one.
pub fn import_thunk(exe: &Exe, rva: u32) -> Option<String> {
    let i = disasm::decode_range(&exe.image, rva, 1);
    let i = i.first()?;
    if i.bytes.starts_with(&[0xFF, 0x25]) || i.bytes.starts_with(&[0x48, 0xFF, 0x25]) {
        return exe.imports.get(&i.rip_target()?).cloned();
    }
    None
}

/// The first `n` bytes at `rva` as an auto-wildcarded pattern, and its
/// exe-wide hit count.
pub fn pattern_at(exe: &Exe, rva: u32, n: usize) -> (String, usize) {
    let insns: Vec<Insn> = disasm::decode_range(&exe.image, rva, n);
    let p = disasm::auto_wildcard(&insns, n);
    let count = if p.has_fixed() { p.find_all_overlapping(&exe.image).len() } else { 0 };
    (p.to_string(), count)
}

/// Distinct functions among a list of reference sites.
pub fn distinct_functions(exe: &Exe, sites: &[usize]) -> usize {
    let starts: BTreeSet<Option<u32>> = sites.iter().map(|&s| exe.function_start(s as u32).0).collect();
    starts.len()
}

pub fn collect(exe: &Exe, rva: u32, len: usize, n: usize, callers: bool) -> Vec<Target> {
    let insns = disasm::decode_range(&exe.image, rva, len);
    let mut out: Vec<Target> = insns
        .iter()
        .filter(|i| i.is_call_rel32() || i.is_jmp_rel32())
        .filter_map(|i| {
            let target = i.rel_target()?;
            let import = import_thunk(exe, target);
            let (pattern, count) = pattern_at(exe, target, n);
            Some(Target {
                site: i.rva,
                kind: if i.is_call_rel32() { "call" } else { "jmp" },
                target,
                status: exe.pdata_status(target),
                import,
                head: exe.bytes(target, n).to_vec(),
                pattern,
                count,
                callers: None,
            })
        })
        .collect();
    if callers {
        let set: HashSet<u64> = out.iter().map(|t| t.target as u64).collect();
        let index = scan::rel32_index(&exe.image, &set);
        for t in &mut out {
            let sites = index.get(&(t.target as u64)).cloned().unwrap_or_default();
            t.callers = Some((sites.len(), distinct_functions(exe, &sites)));
        }
    }
    out
}

pub fn run(out: &mut impl Write, exe: &Exe, rva: u32, len: usize, n: usize, callers: bool, tsv: bool) -> Result<()> {
    let rows = collect(exe, rva, len, n, callers);
    if tsv {
        writeln!(out, "site\tkind\ttarget\tstatus\timport\tbytes\tpattern\tcount\tcall_sites\tcaller_fns")?;
        for t in &rows {
            let (cs, cf) = t.callers.map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or_default();
            writeln!(
                out,
                "{:#x}\t{}\t{:#x}\t{}\t{}\t{}\t{}\t{}\t{cs}\t{cf}",
                t.site,
                t.kind,
                t.target,
                t.status,
                t.import.clone().unwrap_or_default(),
                hex_bytes(&t.head),
                t.pattern,
                t.count
            )?;
        }
        return Ok(());
    }
    writeln!(out, "calls in +{rva:#x}..+{:#x}: {} rel32 call/jmp target(s)", rva as usize + len, rows.len())?;
    for t in &rows {
        let off = t.site - rva;
        write!(out, "  +{:#x} (+{off:#x}) {:<4} -> +{:#x}  {}", t.site, t.kind, t.target, t.status)?;
        if let Some((cs, cf)) = t.callers {
            write!(out, ", {cs} call site(s) in {cf} function(s)")?;
        }
        writeln!(out)?;
        if let Some(name) = &t.import {
            writeln!(out, "        import thunk: {name}")?;
            continue;
        }
        writeln!(out, "        bytes   {}", hex_bytes(&t.head))?;
        writeln!(out, "        pattern {}  ({} hit(s) exe-wide)", t.pattern, t.count)?;
    }
    Ok(())
}
