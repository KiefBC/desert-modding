//! Fetching one function: four requests, or one when the "function" is a lie.

use crate::ghidra::Ghidra;
use crate::meta::Meta;
use crate::parse::{self, Caller};

/// Everything the exported file says about one function.
pub struct Record {
    pub va: u64,
    pub name: String,
    pub signature: String,
    pub body: String,
    pub callees: Vec<u64>,
    pub tailcalls: Vec<u64>,
    pub callers: Vec<Caller>,
    pub callers_truncated: bool,
    /// Body span in bytes when it is over `--max-body-bytes`, else 0.
    pub oversized: u64,
    pub asm: String,
    pub decompile: String,
}

impl Record {
    /// The graph facts about this function, at the depth it was reached.
    pub fn meta(&self, depth: i64) -> Meta {
        Meta {
            va: self.va,
            name: self.name.clone(),
            depth: Some(depth),
            callees: self.callees.clone(),
            tailcalls: self.tailcalls.clone(),
            callers: self.callers.clone(),
            callers_truncated: self.callers_truncated,
            oversized: self.oversized,
        }
    }
}

/// The section and body limits, which travel together everywhere.
#[derive(Clone, Copy)]
pub struct Limits {
    pub caller_limit: usize,
    pub max_body: u64,
    pub max_section: usize,
}

/// Truncate an over-long section, saying so in-band rather than silently.
///
/// The counts are characters, not bytes, because the Python's `len()` over a
/// decoded `str` was; the message says "bytes" in both. For the disassembly
/// this measures, the two are the same number.
pub fn clip(text: &str, limit: usize, what: &str) -> String {
    let len = text.chars().count();
    if len <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit).collect();
    format!(
        "{head}\n\n[evidence: {what} truncated at {limit} bytes of {len}; \
         re-fetch with --max-section-bytes to get more]\n"
    )
}

/// `None` means "Ghidra says there is no function here" - the walk records that
/// so it does not come back, and the address gets no file.
pub fn fetch_function(g: &Ghidra, va: u64, lim: Limits) -> Option<Record> {
    let a = format!("{va:x}");
    let info = g.get("get_function_by_address", &[("address", &a)]);
    if info.starts_with("No function") {
        return None;
    }
    let (mut name, mut signature, mut body) = (String::new(), String::new(), String::new());
    for line in info.lines() {
        if let Some(rest) = line.strip_prefix("Function: ") {
            name = rest.split(" at ").next().unwrap_or("").trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Signature: ") {
            signature = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("Body: ") {
            body = rest.trim().to_string();
        }
    }
    let named = if name.is_empty() { format!("FUN_{a}") } else { name };
    let (lo, hi) = parse::parse_body(&body);
    if hi > lo && hi - lo > lim.max_body {
        // Not a real function. Record it so the walk does not retry it, and
        // contribute nothing: its callee list is noise, not a call graph.
        // Ghidra's auto-analysis glues runs of unanalysed bytes into one
        // oversized entry; 41 of them once accounted for 9.9 GB of a 9.4 GB
        // tree, and their bogus callee lists inflated the walk far more than
        // any real code did.
        return Some(Record {
            va,
            name: named,
            signature,
            body,
            callees: Vec::new(),
            tailcalls: Vec::new(),
            callers: Vec::new(),
            callers_truncated: false,
            oversized: hi - lo,
            asm: String::new(),
            decompile: String::new(),
        });
    }
    let asm = g.get("disassemble_function", &[("address", &a)]);
    let dec = g.get("decompile_function_by_address", &[("address", &a)]);
    let xrefs = g.get("xrefs_to", &[("address", &a), ("limit", &lim.caller_limit.to_string())]);
    let callers = parse::parse_callers(&xrefs);
    Some(Record {
        va,
        name: named,
        signature,
        body,
        callees: parse::parse_callees(&asm),
        tailcalls: parse::parse_tailcalls(&asm, lo, hi),
        // A caller list that came back full is a list that was cut off. The
        // walk treats it as a hub and expands through none of it, because the
        // callers it cannot see are exactly the ones that would matter.
        callers_truncated: callers.len() >= lim.caller_limit,
        callers,
        oversized: 0,
        asm: clip(&asm, lim.max_section, "disassembly"),
        decompile: clip(&dec, lim.max_section, "decompilation"),
    })
}
