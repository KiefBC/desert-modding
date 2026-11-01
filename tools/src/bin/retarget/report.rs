//! `retarget report`: triage, census and headers as one markdown checklist.
//!
//! This is the first thing to run after a game update, and its output is the
//! skeleton of the findings note (`docs/findings/<date>-build-<id>-retarget.md`):
//! what died, what the template looks like now, and the constants in
//! `desert-core/src/gimmick.rs` and `tools/src/bin/retarget/expect.rs` that
//! have to move. Exit 1 when anything on the checklist is not clean.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::{census, peinfo, triage, Exe};

/// Returns `true` when every non-optional signature hits once, no accessor
/// tail was rejected, the `gimmickinfo` site has a slot load and a loader
/// call whose target opens with the loader prologue, its stream fill parses
/// (the hook reads the stream at those offsets), every name a site passes
/// is in the list, and the appmanifest build is the pinned one. A loader
/// target without the prologue elsewhere is not a failure: two tables
/// (`KeyMap`, `AutoSpawnFilterInfo`) call a loader of another shape on every
/// build measured, and the plugin only hooks the `gimmickinfo` one.
pub fn run(out: &mut impl Write, exe: &Exe, sigs: &Path, names: &Path) -> Result<bool> {
    let rows = triage::run(exe, sigs)?;
    let c = census::measure(exe, 0x400, names)?;
    let build = desert_tools::paths::build_id().unwrap_or_else(|| "?".into());

    writeln!(out, "# retarget report: build {build}\n")?;
    writeln!(out, "Exe `{}`, calibrated build {}.\n", exe.path.display(), crate::expect::BUILD)?;

    writeln!(out, "## Headers\n\n```text")?;
    peinfo::print(out, exe, false)?;
    writeln!(out, "```\n")?;

    writeln!(out, "## Signatures\n")?;
    writeln!(out, "| signature | status | hits | where |")?;
    writeln!(out, "| --- | --- | --- | --- |")?;
    for r in &rows {
        let where_ = match r.hits.as_slice() {
            [h] => format!("`+{h:#x}` (file `{:#x}`)", exe.rva_to_offset(*h as u32).unwrap_or(0)),
            [] => match (&r.prefix, &r.suffix) {
                (Some(p), _) => format!("prefix {}/{} bytes still hits {} time(s)", p.len, r.sig.pattern.len(), p.hits.len()),
                (None, Some(s)) => format!("suffix {}/{} bytes still hits {} time(s)", s.len, r.sig.pattern.len(), s.hits.len()),
                (None, None) => "nothing of it hits".to_string(),
            },
            many => format!("{} hits", many.len()),
        };
        writeln!(out, "| `{}` | {} | {} | {where_} |", r.sig.name, r.status(), r.hits.len())?;
    }
    let dead: Vec<&str> = rows.iter().filter(|r| r.hits.len() != 1 && !r.sig.optional).map(|r| r.sig.name.as_str()).collect();
    writeln!(out)?;
    if dead.is_empty() {
        writeln!(out, "Every required signature hits once.\n")?;
    } else {
        writeln!(out, "To re-derive: {}. Run `retarget triage` for the remnants, then `calls`, `callers` and `prologue` on what is left.\n", dead.join(", "))?;
    }

    writeln!(out, "## Accessor template\n\n```text")?;
    census::print(out, &c, false, false)?;
    writeln!(out, "```\n")?;

    writeln!(out, "## Constants to check\n")?;
    writeln!(out, "| constant | where | measured |")?;
    writeln!(out, "| --- | --- | --- |")?;
    let hc = c.head_counts();
    writeln!(out, "| `ACCESSORS` census | `gimmick_real.rs`, `expect.rs` | {} = {} + {} + {} + {} |", c.sites.len(), hc[0], hc[1], hc[2], hc[3])?;
    let tails: Vec<String> = c.tails().iter().map(|(k, v)| format!("{k} x{v}")).collect();
    writeln!(out, "| `parse_accessor_tail` shapes | `gimmick.rs` | {} |", tails.join(", "))?;
    let slots: Vec<String> = c.slot_hist().iter().map(|(k, v)| format!("{k:#x} x{v}")).collect();
    let regs: Vec<String> = c.slot_regs().iter().map(|(k, v)| format!("{k} x{v}")).collect();
    writeln!(out, "| `SLOT_WINDOW` / `is_slot_cmp` | `gimmick.rs` | distances {}; registers {} |", slots.join(", "), regs.join(", "))?;
    match c.loader_dist_range() {
        Some((lo, hi)) => writeln!(
            out,
            "| `B_WINDOW` | `gimmick.rs` | {} of {} sites have a loader call, {lo:#x}..{hi:#x} past the accessor's call; suggested {:#x} |",
            c.loaders_found(),
            c.sites.len(),
            c.suggested_b_window().unwrap_or(0)
        )?,
        None => writeln!(out, "| `B_WINDOW` | `gimmick.rs` | NO loader call found at any site |")?,
    }
    writeln!(out, "| `LOADER_PROLOGUE` | `gimmick.rs` | present at {} of {} loader targets |", c.loaders_with_prologue(), c.loaders_found())?;
    match c.site_named("gimmickinfo") {
        Some(g) => writeln!(
            out,
            "| `gimmickinfo` | `gimmick_real.rs`, `expect.rs` | accessor `+{:#x}`, slot load `{}`, loader call `{}` -> loader `{}` |",
            g.at,
            g.slot.as_ref().map(|s| format!("+{:#x}", s.at)).unwrap_or_else(|| "none".into()),
            g.loader.as_ref().map(|l| format!("+{:#x}", l.call_at)).unwrap_or_else(|| "none".into()),
            g.loader.as_ref().map(|l| format!("+{:#x}", l.target)).unwrap_or_else(|| "none".into())
        )?,
        None => writeln!(out, "| `gimmickinfo` | `gimmick_real.rs` | NO accessor passes the name |")?,
    }
    let gimmick_stream = c.site_named("gimmickinfo").and_then(|g| g.stream);
    writeln!(
        out,
        "| `StreamLayout` | `hook.rs` (parsed at install), `gimmick_real.rs`, `expect.rs` | gimmickinfo {}; {} |",
        gimmick_stream.map(|l| l.label()).unwrap_or_else(|| "NOT PARSED".into()),
        c.stream_summary()
    )?;
    writeln!(
        out,
        "| `static-info-names.txt` | `tools/` | {} names; {} unseen by any site; {} site(s) pass an unlisted name |",
        c.names_total,
        c.names_unseen.len(),
        c.unknown_names().len()
    )?;
    writeln!(
        out,
        "| `expect::BUILD` / `paths::CALIBRATED_BUILD` | `retarget/expect.rs`, `tools/src/paths.rs` | {} / {} (appmanifest says {build}) |",
        crate::expect::BUILD,
        desert_tools::paths::CALIBRATED_BUILD
    )?;
    writeln!(out, "| `SizeOfImage` | `desert-dispatch/src/node.rs` banking gates | {:#x} |", exe.size_of_image)?;

    let gimmick_ok = c
        .site_named("gimmickinfo")
        .is_some_and(|g| g.slot.is_some() && g.loader.as_ref().is_some_and(|l| l.prologue_ok));
    let mut why = Vec::new();
    if !dead.is_empty() {
        why.push(format!("{} signature(s) not exactly once", dead.len()));
    }
    if !c.rejected_tails().is_empty() {
        why.push(format!("{} accessor tail(s) of an unknown shape", c.rejected_tails().len()));
    }
    if !gimmick_ok {
        why.push("the gimmickinfo site has no slot load or no loader call with the prologue".to_string());
    }
    if gimmick_ok && gimmick_stream.is_none() {
        why.push(
            "the gimmickinfo site's stream fill does not parse (the hook will probe the known layouts at \
             runtime; teach parse_stream_fill the new codegen)"
                .to_string(),
        );
    }
    if !c.unknown_names().is_empty() {
        why.push(format!("{} site(s) pass a name not in the list", c.unknown_names().len()));
    }
    if build != crate::expect::BUILD {
        why.push(format!("appmanifest build {build} is not the pinned {}", crate::expect::BUILD));
    }
    if why.is_empty() {
        writeln!(out, "\nVerdict: clean")?;
    } else {
        writeln!(out, "\nVerdict: work to do: {}", why.join("; "))?;
    }
    Ok(why.is_empty())
}
