//! The terminal side: the cache, and one printer per lookup mode.
//!
//! Every line here is width-for-width what `tools/items.py` printed. The
//! padding is not decoration - `items --list | grep ...` is documented usage
//! and people have column positions in their fingers.

use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::table::{Detector, MAX_PLAUSIBLE_ITEM_ID};
use super::{av, bv, commas, fams_of, sv, truthy, uv, Args};

/// The cache for the population `args` asked for, or a fresh walk.
///
/// Each detector has its own file, so `--loose` can never read - or write - the
/// default one. **A cache whose detector does not match is treated as no cache
/// at all rather than as an answer**: answering a `--loose` question out of
/// `analysis/items.json` would silently hide every loose-only id, which is the
/// one thing the flag exists to show.
pub fn load(args: &Args, json_out: &Path, collect_rs: &Path, out: &mut dyn Write) -> Result<Value> {
    let want_loose = args.detector() == Detector::Loose;
    if !args.rescan && json_out.exists() {
        if let Ok(text) = std::fs::read_to_string(json_out) {
            if let Ok(d) = serde_json::from_str::<Value>(&text) {
                if d.get("detector").is_some() == want_loose {
                    return Ok(d);
                }
            }
        }
    }
    super::dataset::build(&args.table, collect_rs, args.detector(), true, out)
}

pub fn fmt_amount(it: &Value) -> String {
    let (mn, mx) = (uv(it, "min"), uv(it, "max"));
    if mn == mx {
        format!("{mn}")
    } else {
        format!("{mn}..{mx}")
    }
}

/// The padded visibility column for a terminal line, **empty string and no
/// padding at all** for the default population - every item there is visible
/// by construction, and the default CLI output must not move.
pub fn vis_tag(it: &Value) -> String {
    match it.get("visibility").and_then(|v| v.as_str()) {
        Some("loose-only") => {
            format!("LOOSE-ONLY{}", if bv(it, "implausible_id") { "! " } else { "  " })
        }
        Some("shipped") => "shipped     ".to_string(),
        _ => String::new(),
    }
}

pub fn show(it: &Value, w: &mut dyn Write) -> Result<()> {
    writeln!(
        w,
        "item {}  {}   [{}] {}{}",
        uv(it, "id"),
        sv(it, "name"),
        sv(it, "confidence"),
        fmt_amount(it),
        if bv(it, "fixed_amount") { "  (fixed)" } else { "" }
    )?;
    match it.get("visibility").and_then(|v| v.as_str()) {
        Some("loose-only") => {
            writeln!(
                w,
                "  VISIBILITY  loose-only: NO block of this item is in the \
                 population the mod sees."
            )?;
            writeln!(
                w,
                "              No multiplier reaches it. \
                 It is absent from analysis/items.json."
            )?;
        }
        Some("shipped") => {
            writeln!(
                w,
                "  visibility  shipped ({} of {} blocks; {} only the loose walk sees)",
                uv(it, "shipped_blocks"),
                uv(it, "blocks"),
                uv(it, "loose_only_blocks")
            )?;
        }
        _ => {}
    }
    if bv(it, "implausible_id") {
        writeln!(
            w,
            "  \u{26a0} ID       past {}: the list this came from is being \
             mis-parsed. Do not use this id.",
            commas(MAX_PLAUSIBLE_ITEM_ID)
        )?;
    }
    writeln!(
        w,
        "  family      {}",
        fams_of(it).unwrap_or_else(|| "unclassified".into())
    )?;
    writeln!(
        w,
        "  named by    {}  ({} agreeing, {} conflicting)",
        sv(it, "named_by"),
        av(it, "agreeing_records").len(),
        av(it, "conflicting_records").len()
    )?;
    if truthy(it, "confidence_downgraded") {
        writeln!(
            w,
            "  downgraded  from {}: {}",
            sv(it, "confidence_before_loose_rule"),
            sv(it, "confidence_downgraded")
        )?;
    }
    if truthy(it, "name_in_default_doc") {
        writeln!(
            w,
            "  NB          docs/reference-items.md calls this id '{}': \
             the inference sees a wider corpus here",
            sv(it, "name_in_default_doc")
        )?;
    }
    if truthy(it, "note") {
        writeln!(w, "  note        {}", sv(it, "note"))?;
    }
    if truthy(it, "name_shared_with") {
        writeln!(
            w,
            "  same name   {}",
            av(it, "name_shared_with")
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )?;
    }
    writeln!(
        w,
        "  {} blocks in {} records:",
        uv(it, "blocks"),
        uv(it, "records")
    )?;
    for s in av(it, "sources") {
        let mut rels: Vec<u64> = av(s, "block_rel").iter().map(|v| v.as_u64().unwrap()).collect();
        rels.sort_unstable();
        let rel = rels.iter().map(|r| format!("+{r}")).collect::<Vec<_>>().join(",");
        let extra = if s.get("visibility").is_some() {
            let keys = av(s, "entry_keys")
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(",");
            if sv(s, "visibility") == "loose-only" {
                format!("  [{}, +64={keys}]", sv(s, "visibility"))
            } else {
                format!("  [{}]", sv(s, "visibility"))
            }
        } else {
            String::new()
        };
        writeln!(
            w,
            "    {:>9}  {:<58} {}..{:<6} {:<9} rel {rel}{extra}",
            uv(s, "key"),
            sv(s, "name"),
            uv(s, "min"),
            uv(s, "max"),
            s.get("family").and_then(|v| v.as_str()).unwrap_or("-"),
        )?;
    }
    Ok(())
}

/// `--list`, `--unclassified`, `--family`, `--record` and the free-text query,
/// in the order `main()` checks them.
pub fn query(args: &Args, d: &Value, w: &mut dyn Write) -> Result<()> {
    let items = d["items"].as_array().unwrap();
    let loose = args.loose;

    if args.list {
        for it in items {
            writeln!(
                w,
                "{:>9}  {:<26} {:<10} {:<8} {}{}",
                uv(it, "id"),
                sv(it, "name"),
                fmt_amount(it),
                sv(it, "confidence"),
                vis_tag(it),
                fams_of(it).unwrap_or_else(|| "-".into())
            )?;
        }
        return Ok(());
    }
    if args.unclassified {
        let mut rows: Vec<&Value> = items.iter().filter(|i| !bv(i, "classified")).collect();
        rows.sort_by_key(|i| -(uv(i, "blocks") as i64));
        for it in &rows {
            writeln!(
                w,
                "{:>9}  {:<26} {:<10} {:<8} {}{:>3} blocks  {}",
                uv(it, "id"),
                sv(it, "name"),
                fmt_amount(it),
                sv(it, "confidence"),
                vis_tag(it),
                uv(it, "blocks"),
                sv(&av(it, "sources")[0], "name")
            )?;
        }
        writeln!(
            w,
            "{} unclassified items{}",
            rows.len(),
            if loose {
                format!(
                    ", {} of them loose-only",
                    rows.iter().filter(|i| sv(i, "visibility") == "loose-only").count()
                )
            } else {
                String::new()
            }
        )?;
        return Ok(());
    }
    if let Some(family) = &args.family {
        let f = family.to_lowercase();
        let mut rows: Vec<&Value> = items
            .iter()
            .filter(|i| av(i, "families").iter().any(|x| x.as_str().unwrap().to_lowercase() == f))
            .collect();
        if rows.is_empty() {
            let mut fams: Vec<&str> = items
                .iter()
                .flat_map(|i| av(i, "families"))
                .map(|x| x.as_str().unwrap())
                .collect();
            fams.sort_unstable();
            fams.dedup();
            return Err(anyhow!(
                "no Family '{family}'; known: {}",
                fams.join(", ")
            ));
        }
        rows.sort_by_key(|i| sv(i, "name"));
        for it in &rows {
            writeln!(
                w,
                "{:>9}  {:<26} {:<10} {:<8} {}{} records",
                uv(it, "id"),
                sv(it, "name"),
                fmt_amount(it),
                sv(it, "confidence"),
                vis_tag(it),
                uv(it, "records")
            )?;
        }
        writeln!(w, "{} items in {family}", rows.len())?;
        return Ok(());
    }
    if let Some(record) = &args.record {
        let q = record.to_lowercase();
        let mut hits: Vec<(&Value, &Value)> = items
            .iter()
            .flat_map(|it| av(it, "sources").iter().map(move |s| (it, s)))
            .filter(|(_, s)| {
                sv(s, "name").to_lowercase().contains(&q) || q == uv(s, "key").to_string()
            })
            .collect();
        if hits.is_empty() {
            return Err(anyhow!(
                "no record matching '{record}' yields anything{}",
                if loose {
                    ""
                } else {
                    ". The mod's detector may not see it: try --loose."
                }
            ));
        }
        hits.sort_by_key(|(_, s)| sv(s, "name").to_lowercase());
        for (it, s) in &hits {
            let tag = if s.get("visibility").and_then(|v| v.as_str()) == Some("loose-only") {
                format!(
                    "   LOOSE-ONLY, +64={}",
                    av(s, "entry_keys").iter().map(|k| k.to_string()).collect::<Vec<_>>().join(",")
                )
            } else {
                String::new()
            };
            writeln!(
                w,
                "{} (key {}, {}) -> item {} {} [{}] {}..{}{tag}",
                sv(s, "name"),
                uv(s, "key"),
                s.get("family").and_then(|v| v.as_str()).unwrap_or("unclassified"),
                uv(it, "id"),
                sv(it, "name"),
                sv(it, "confidence"),
                uv(s, "min"),
                uv(s, "max")
            )?;
        }
        return Ok(());
    }

    let q = args.query.as_deref().unwrap();
    if !q.is_empty() && q.bytes().all(|c| c.is_ascii_digit()) {
        if let Ok(n) = q.parse::<u64>() {
            if let Some(it) = items.iter().find(|i| uv(i, "id") == n) {
                return show(it, w);
            }
        }
    }
    let ql = q.to_lowercase();
    let hits: Vec<&Value> = items
        .iter()
        .filter(|i| {
            sv(i, "name").to_lowercase().contains(&ql)
                || av(i, "sources").iter().any(|s| sv(s, "name").to_lowercase().contains(&ql))
        })
        .collect();
    if hits.is_empty() {
        return Err(anyhow!(
            "nothing matching '{q}'. `--list` prints every item.{}",
            if loose {
                ""
            } else {
                " The mod's detector may not see it: try --loose."
            }
        ));
    }
    if hits.len() == 1 {
        return show(hits[0], w);
    }
    let exact: Vec<&&Value> = hits.iter().filter(|i| sv(i, "name").to_lowercase() == ql).collect();
    if exact.len() == 1 {
        show(exact[0], w)?;
        writeln!(
            w,
            "\n({} other partial matches; `{} --list | grep {q}` for all)",
            hits.len() - 1,
            super::GENERATOR_BASENAME
        )?;
        return Ok(());
    }
    writeln!(w, "{} matches for '{q}':", hits.len())?;
    let mut sorted = hits.clone();
    sorted.sort_by(|a, b| (sv(a, "name"), uv(a, "id")).cmp(&(sv(b, "name"), uv(b, "id"))));
    for it in sorted {
        writeln!(
            w,
            "{:>9}  {:<26} {:<10} {:<8} {}{:<9} {}",
            uv(it, "id"),
            sv(it, "name"),
            fmt_amount(it),
            sv(it, "confidence"),
            vis_tag(it),
            fams_of(it).unwrap_or_else(|| "-".into()),
            sv(&av(it, "sources")[0], "name")
        )?;
    }
    Ok(())
}
