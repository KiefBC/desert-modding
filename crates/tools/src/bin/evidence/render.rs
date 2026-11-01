//! One function, as the Markdown file a later session greps.

use crate::record::Record;

/// The exported file's text, metadata line first.
pub fn render(rec: &Record, depth: i64, anchor_note: &str) -> String {
    let meta = rec.meta(depth).to_json();
    if rec.oversized != 0 {
        // The stub is a full paragraph on purpose. Someone grepping the tree
        // and landing on a 200 MB "function" needs to be told it is an
        // artefact of auto-analysis, not a real function that failed to
        // export, or the next thing they do is raise --max-section-bytes and
        // re-export 9.9 GB of unanalysed bytes.
        return [
            format!("<!-- evidence 1 {meta} -->"),
            format!("# {} @ 0x{:x} (SKIPPED)", rec.name, rec.va),
            String::new(),
            format!(
                "Body `{}` spans {} bytes. That is not a",
                rec.body,
                commas(rec.oversized)
            ),
            "function: Ghidra's auto-analysis glues runs of unanalysed bytes into one".to_string(),
            "oversized entry. Nothing was exported and it contributes no call edges.".to_string(),
            "Raise --max-body-bytes to export it anyway.".to_string(),
            String::new(),
        ]
        .join("\n");
    }
    let mut lines = vec![
        format!("<!-- evidence 1 {meta} -->"),
        format!("# {} @ 0x{:x}", rec.name, rec.va),
        String::new(),
        format!(
            "- Signature: `{}`",
            if rec.signature.is_empty() { "unknown" } else { &rec.signature }
        ),
        format!("- Body: `{}`", if rec.body.is_empty() { "unknown" } else { &rec.body }),
        format!("- Reached at depth {depth}{anchor_note}"),
        format!(
            "- {} caller reference(s){}, {} direct callee(s)",
            rec.callers.len(),
            if rec.callers_truncated { " (TRUNCATED)" } else { "" },
            rec.callees.len()
        ),
        String::new(),
        "## Callers".to_string(),
        String::new(),
    ];
    if rec.callers.is_empty() {
        lines.push("- none found".to_string());
    } else {
        for c in &rec.callers {
            let name = if c.name.is_empty() { "?" } else { &c.name };
            lines.push(format!("- `0x{:x}` in `{name}` [{}]", c.site, c.kind));
        }
    }
    if !rec.tailcalls.is_empty() {
        // Kept in their own section rather than merged into the callees: a
        // reader asking "what does this call" wants both, but a reader asking
        // "why is this function in the tree at all" needs to see that it was
        // reached through a thunk.
        lines.extend([String::new(), "## Tail calls / thunk targets".to_string(), String::new()]);
        lines.extend(rec.tailcalls.iter().map(|c| format!("- `0x{c:x}` (`FUN_{c:x}`)")));
    }
    lines.extend([String::new(), "## Callees (direct calls only)".to_string(), String::new()]);
    if rec.callees.is_empty() {
        lines.push("- none found".to_string());
    } else {
        lines.extend(rec.callees.iter().map(|c| format!("- `0x{c:x}` (`FUN_{c:x}`)")));
    }
    lines.extend([
        String::new(),
        "## Decompilation".to_string(),
        String::new(),
        "```c".to_string(),
        rec.decompile.trim().to_string(),
        "```".to_string(),
        String::new(),
        "## Disassembly".to_string(),
        String::new(),
        "```asm".to_string(),
        rec.asm.trim().to_string(),
        "```".to_string(),
        String::new(),
    ]);
    lines.join("\n")
}

/// Python's `{:,}`: thousands separators, for the one number a human reads as
/// a size rather than an address.
fn commas(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
