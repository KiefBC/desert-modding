//! What counts as a function, a call, a caller: the parsers over GhidraMCP's
//! plain-text responses and over our own source tree.
//!
//! Everything here is pure: no I/O beyond reading `docs/` and `analysis/` for
//! the default anchor set, and no network. That is deliberate - the walk in
//! `main` is the only part that needs a live Ghidra, so every rule about which
//! edges the walk may follow is testable without one.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

/// The game module: base 0x140000000, image a little under 0x18000000 bytes.
/// Anything outside this is not CrimsonDesert.exe (0x180000000 was CDLoot.asi).
pub const MOD_LO: u64 = 0x1_4000_0000;
pub const MOD_HI: u64 = 0x1_6000_0000;

pub fn in_module(va: u64) -> bool {
    (MOD_LO..MOD_HI).contains(&va)
}

/// `\bFUN_<9+ hex>\b` - a Ghidra auto-name encodes its own entry point.
pub fn fun_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\bFUN_([0-9a-fA-F]{9,})\b").unwrap())
}

/// A whole disassembly line that is nothing but a direct call. The anchoring
/// is the point: `CALL qword ptr [...]` has more after the mnemonic and so
/// never matches, which is how indirect calls stay out of the call graph.
fn call_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^\s*([0-9a-f]+):\s+CALL\s+0x([0-9a-f]+)\s*$").unwrap())
}

fn jmp_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^\s*([0-9a-f]+):\s+JMP\s+0x([0-9a-f]+)\s*$").unwrap())
}

fn xref_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^From ([0-9a-f]+)(?: in (\S+))? \[(\w+)\]").unwrap())
}

/// `// ==== FUN_x ====` - the header a dump in `analysis/` puts above a body.
fn analysis_header_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?m)^// ==== (FUN_[0-9a-fA-F]+)").unwrap())
}

/// One caller reference: the call site, the function containing it, the kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Caller {
    pub site: u64,
    pub name: String,
    pub kind: String,
}

/// Python's `int(s, 16)`, minus the bignum.
///
/// `None` means "Python would not have produced an in-module address": either
/// it is not hex at all, or it is wider than 64 bits, and every such value is
/// far above `MOD_HI` so the caller's `in_module` check would have dropped it
/// anyway. The one behaviour difference is a signed literal (`-140000000`),
/// which the Python parsed and then silently dropped as out-of-module and this
/// rejects as unparsable; see the port notes.
pub fn py_hex(s: &str) -> Option<u64> {
    let t = s.trim();
    let t = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    let cleaned: String = t.chars().filter(|c| *c != '_').collect();
    if cleaned.is_empty() || !cleaned.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u64::try_from(u128::from_str_radix(&cleaned, 16).ok()?).ok()
}

/// Direct `CALL 0x...` targets, in first-seen order.
///
/// Indirect calls (`CALL qword ptr [...]`) are deliberately not followed: the
/// target is a runtime value and the address in the brackets is a slot, not a
/// function.
pub fn parse_callees(asm: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for c in call_re().captures_iter(asm) {
        let Some(va) = py_hex(&c[2]) else { continue };
        if in_module(va) && !out.contains(&va) {
            out.push(va);
        }
    }
    out
}

/// Unconditional JMPs that leave the function: tail calls and thunk stubs.
///
/// This exe is full of 5-byte `jmp` thunks - cold-code layout puts a stub at
/// 0x1417xxxxx that jumps to the real body at 0x14cxxxxxx - so a walk that
/// follows only CALL dead-ends at every one of them and never reaches the
/// implementation. In the first depth-2 tree 16% of functions had such a jump
/// and 1254 distinct targets were missing entirely, including 122 of the 149
/// `initStatic()` bodies. Jumps that stay inside [lo, hi] are the function's
/// own branches and loops, and are not followed.
pub fn parse_tailcalls(asm: &str, lo: u64, hi: u64) -> Vec<u64> {
    let mut out = Vec::new();
    for c in jmp_re().captures_iter(asm) {
        let Some(va) = py_hex(&c[2]) else { continue };
        if in_module(va) && !(lo <= va && va <= hi) && !out.contains(&va) {
            out.push(va);
        }
    }
    out
}

/// `"1403856b0 - 14038595e"` -> (0x1403856b0, 0x14038595e); (0, 0) if unparsable.
pub fn parse_body(body: &str) -> (u64, u64) {
    let Some((a, b)) = body.split_once('-') else {
        return (0, 0);
    };
    match (py_hex(a), py_hex(b)) {
        (Some(lo), Some(hi)) => (lo, hi),
        _ => (0, 0),
    }
}

/// (site, containing-function-name, kind) triples from an xrefs_to dump.
pub fn parse_callers(xrefs: &str) -> Vec<Caller> {
    xref_re()
        .captures_iter(xrefs)
        .map(|c| Caller {
            // The regex only admits `[0-9a-f]+`, so this parses unless Ghidra
            // emitted a site address wider than 64 bits, which would be a bug
            // in Ghidra rather than something to carry into the graph.
            site: py_hex(&c[1]).unwrap_or(0),
            name: c.get(2).map_or(String::new(), |m| m.as_str().to_string()),
            kind: c[3].to_string(),
        })
        .collect()
}

/// A `FUN_<hex>` name encodes its own entry point. Others we cannot resolve.
pub fn entry_of(name: &str) -> Option<u64> {
    let m = fun_re().captures(name)?;
    // Python used `fullmatch`: the name has to be exactly the FUN_ token, not
    // merely contain one, or `sub_140385000_thunk_FUN_140386000` would be
    // taken for the function it mentions.
    if m.get(0)?.as_str().len() != name.len() {
        return None;
    }
    let va = py_hex(&m[1])?;
    in_module(va).then_some(va)
}

/// Every in-module FUN_ address named in docs/ or already dumped in analysis/.
pub fn default_anchors(root: &Path) -> Vec<u64> {
    let mut found: Vec<u64> = Vec::new();
    let sources: [(&str, &str, Option<&Regex>); 2] = [
        // Prose: every FUN_ we bothered to name in a findings or reference doc.
        ("docs", "md", None),
        // Dumps: only the `// ==== FUN_x ====` headers. The FUN_ names inside a
        // decompiled body are its callees, and depth 1 reaches those anyway;
        // taking them as anchors makes 100 anchors into 600.
        ("analysis", "c", Some(analysis_header_re())),
    ];
    for (dir, ext, header_re) in sources {
        for path in sorted_files(&root.join(dir), ext) {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            let text = match header_re {
                Some(re) => re
                    .captures_iter(&text)
                    .map(|c| c[1].to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => text.into_owned(),
            };
            for c in fun_re().captures_iter(&text) {
                if let Some(va) = py_hex(&c[1]) {
                    if in_module(va) && !found.contains(&va) {
                        found.push(va);
                    }
                }
            }
        }
    }
    found.sort_unstable();
    found
}

fn sorted_files(dir: &Path, ext: &str) -> Vec<std::path::PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<_> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    out.sort();
    out
}
