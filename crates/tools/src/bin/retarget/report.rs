//! `retarget report`: triage, census and headers as one markdown checklist.
//!
//! This is the first thing to run after a game update, and its output is the
//! skeleton of the findings note (`docs/findings/<date>-build-<id>-retarget.md`):
//! what died, what the template looks like now, and the constants in
//! `crates/desert-core/src/gimmick.rs` and `crates/tools/src/bin/retarget/expect.rs` that
//! have to move. Exit 1 when anything on the checklist is not clean.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

use crate::{census, peinfo, triage, Exe};

/// Returns `true` when every non-optional signature hits once, no accessor
/// tail was rejected, the `gimmickinfo` accessor is reached by a getter with
/// one slot and has a loader call whose target opens with `LOADER_PROLOGUE`,
/// its stream fill parses (the hook reads the stream at those offsets), no
/// accessor's getters disagree on a slot and no table name has two, every
/// getter reads the record count and the entries pointer at the same
/// manager offsets (the entry stride is per table), every name a site passes
/// is in the list, and the appmanifest build is the pinned one. A loader target
/// without the 15-byte prologue elsewhere is not a failure: most loaders
/// open with a different spill sequence on 26636972 (and on every build
/// some tables call a loader of another shape), and the plugin only hooks
/// the `gimmickinfo` one.
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

    writeln!(out, "## Accessor chain\n\n```text")?;
    census::print(out, &c, false, false)?;
    writeln!(out, "```\n")?;

    writeln!(out, "## Constants to check\n")?;
    writeln!(out, "| constant | where | measured |")?;
    writeln!(out, "| --- | --- | --- |")?;
    let hc = c.head_counts();
    writeln!(out, "| accessor heads (`ACCESSORS`) | `gimmick.rs`, `gimmick_real.rs`, `expect.rs` | {} = {} + {} + {} + {} |", c.sites.len(), hc[0], hc[1], hc[2], hc[3])?;
    let tails: Vec<String> = c.tails().iter().map(|(k, v)| format!("{k} x{v}")).collect();
    writeln!(out, "| accessor tail | `gimmick.rs` | {} |", tails.join(", "))?;
    let offs: Vec<String> = c.head_offsets().iter().map(|(k, v)| format!("{k:#x} x{v}")).collect();
    writeln!(out, "| head offset into the accessor | `gimmick.rs` | {} |", offs.join(", "))?;
    let regs: Vec<String> = c.getter_regs().iter().map(|((s, r), v)| format!("mov {s} / cmp {r} x{v}")).collect();
    let dists: Vec<String> = c.getter_call_dists().iter().map(|(k, v)| format!("{k:#x} x{v}")).collect();
    writeln!(
        out,
        "| getter slot load | `gimmick.rs`, `expect.rs` | {} getters; {}; slot load -> call {}; {} accessor(s) without a getter, {} slot disagreement(s), {} name(s) without one slot |",
        c.getters.len(),
        regs.join(", "),
        dists.join(", "),
        c.sites_without_getter().len(),
        c.slot_disagreements().len(),
        c.names_without_one_slot().len()
    )?;
    let mgr: Vec<String> = c.manager_layouts().iter().map(|(m, v)| format!("{} x{v}", m.label())).collect();
    writeln!(out, "| manager offsets | `manager.rs`, `gimmick.rs` (hook), `expect.rs` | {} |", mgr.join("; "))?;
    let hd: Vec<String> = c.loader_head_dist_hist().iter().map(|(k, v)| format!("{k:#x} x{v}")).collect();
    match c.loader_dist_range() {
        Some((lo, hi)) => writeln!(
            out,
            "| `B_WINDOW` | `gimmick.rs` | {} of {} sites have a loader call, head+{}; {lo:#x}..{hi:#x} past the accessor's call; suggested {:#x} |",
            c.loaders_found(),
            c.sites.len(),
            hd.join(", "),
            c.suggested_b_window().unwrap_or(0)
        )?,
        None => writeln!(out, "| `B_WINDOW` | `gimmick.rs` | NO loader call found at any site |")?,
    }
    let pro: Vec<String> = c.loader_prologues().iter().map(|(p, v)| format!("`{}` x{v}", p.label())).collect();
    writeln!(
        out,
        "| `LOADER_PROLOGUE` ({} bytes) | `gimmick.rs` | present at {} of {} loader targets; {} |",
        census::LOADER_PROLOGUE.len(),
        c.loaders_with_prologue(),
        c.loaders_found(),
        pro.join(", ")
    )?;
    let opt = |v: Option<u32>| v.map(|x| format!("+{x:#x}")).unwrap_or_else(|| "none".into());
    match c.site_named("gimmickinfo") {
        Some(g) => writeln!(
            out,
            "| `gimmickinfo` | `gimmick_real.rs`, `expect.rs` | getter slot load {}, slot {}, accessor `{}` head `+{:#x}`, loader call `{}` -> loader `{}` ({}) |",
            c.getters_of("gimmickinfo").iter().map(|x| format!("`+{:#x}`", x.at)).collect::<Vec<_>>().join(", "),
            opt(g.slot()),
            opt(g.fn_start),
            g.at,
            opt(g.loader.as_ref().map(|l| l.call_at)),
            opt(g.loader.as_ref().map(|l| l.target)),
            match &g.loader {
                Some(l) if l.prologue_ok => "prologue matches".to_string(),
                Some(l) => format!("prologue `{}`", l.prologue.label()),
                None => "no loader".to_string(),
            }
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
        "| `static-info-names.txt` | `crates/tools/` | {} names; {} unseen by any site; {} site(s) pass an unlisted name |",
        c.names_total,
        c.names_unseen.len(),
        c.unknown_names().len()
    )?;
    writeln!(
        out,
        "| `expect::BUILD` / `paths::CALIBRATED_BUILD` | `retarget/expect.rs`, `crates/tools/src/paths.rs` | {} / {} (appmanifest says {build}) |",
        crate::expect::BUILD,
        desert_tools::paths::CALIBRATED_BUILD
    )?;
    writeln!(out, "| `SizeOfImage` | `crates/desert-dispatch/src/node.rs` banking gates | {:#x} |", exe.size_of_image)?;

    let gimmick_ok = c
        .site_named("gimmickinfo")
        .is_some_and(|g| g.slot().is_some() && g.loader.as_ref().is_some_and(|l| l.prologue_ok));
    let mut why = Vec::new();
    if !dead.is_empty() {
        why.push(format!("{} signature(s) not exactly once", dead.len()));
    }
    if !c.rejected_tails().is_empty() {
        why.push(format!("{} accessor tail(s) of an unknown shape", c.rejected_tails().len()));
    }
    if !gimmick_ok {
        why.push("the gimmickinfo accessor has no getter with one slot, or no loader call opening with LOADER_PROLOGUE".to_string());
    }
    if !c.slot_disagreements().is_empty() || !c.names_without_one_slot().is_empty() {
        why.push(format!(
            "{} accessor(s) whose getters disagree on the slot, {} name(s) without exactly one slot",
            c.slot_disagreements().len(),
            c.names_without_one_slot().len()
        ));
    }
    if c.manager_heads().len() != 1 || c.manager_heads().keys().any(|(_, e)| e.is_none()) {
        why.push(format!(
            "the getters do not agree on one manager count/entries pair ({} seen, or entries unparsed)",
            c.manager_heads().len()
        ));
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
