//! The walk that produces the dataset both artifacts are rendered from.
//!
//! The shape of the JSON is load-bearing: `analysis/items.json` is read back by
//! every lookup, and the default build must stay byte-for-byte what it has
//! always been. Everything a `--loose` build adds is behind the `loose` flag
//! for exactly that reason - a wider walk must not be able to move the file the
//! plugin's own population is recorded in.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde_json::{Map, Value};

use super::checks::run_checks;
use super::infer::{infer_names, Named, Res, CONFIDENCE_DOC};
use super::table::*;

/// One resource-output block, attributed to a record. Never serialised as it
/// stands: the JSON carries these folded up per record key.
///
/// The Python's per-block dict also held `block_offset`, `list_items` and
/// `pad`, and nothing ever read any of the three: the fold to `recs_by_key`
/// drops them. `list_items` in particular is **promised by the generated doc**
/// ("the `sources` entries in `analysis/items.json` carry `list_items`") and
/// has never actually been in the file. The doc text is reproduced verbatim
/// because these artifacts are diffed byte-for-byte, but the claim is wrong and
/// is flagged rather than quietly fixed - fixing it moves `analysis/items.json`,
/// which is a decision, not a port.
pub struct Src {
    pub record_key: u32,
    pub record_name: String,
    pub min: u64,
    pub max: u64,
    pub family: Option<String>,
    pub record_offset: usize,
    pub block_rel: usize,
    /// `"shipped"` / `"loose-only"`, and `""` on a default build, which carries
    /// no visibility at all because every block it sees is visible.
    pub visibility: &'static str,
    pub entry_key: u32,
}

pub struct LooseEntry {
    pub item: u64,
    pub min: u64,
    pub max: u64,
    pub entry_key: u32,
    pub pad: u32,
    pub implausible_id: bool,
}

pub struct LooseList {
    pub list_offset: usize,
    pub record_key: u32,
    pub record_name: String,
    pub family: Option<String>,
    pub blocks: u32,
    pub block_rel: usize,
    pub entries: Vec<LooseEntry>,
}

impl LooseList {
    fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("list_offset".into(), self.list_offset.into());
        m.insert("record_key".into(), self.record_key.into());
        m.insert("record_name".into(), self.record_name.clone().into());
        m.insert("family".into(), opt_str(&self.family));
        m.insert("blocks".into(), self.blocks.into());
        m.insert("block_rel".into(), self.block_rel.into());
        let entries: Vec<Value> = self
            .entries
            .iter()
            .map(|e| {
                let mut x = Map::new();
                x.insert("item".into(), e.item.into());
                x.insert("min".into(), e.min.into());
                x.insert("max".into(), e.max.into());
                x.insert("entry_key".into(), e.entry_key.into());
                x.insert("pad".into(), e.pad.into());
                x.insert("implausible_id".into(), e.implausible_id.into());
                Value::Object(x)
            })
            .collect();
        m.insert("entries".into(), entries.into());
        Value::Object(m)
    }
}

fn opt_str(s: &Option<String>) -> Value {
    match s {
        Some(v) => Value::String(v.clone()),
        None => Value::Null,
    }
}

/// Walk the body and assemble the whole dataset. `quiet` suppresses the
/// self-check lines, which a lookup that happens to miss its cache does not
/// want on stdout in front of its answer.
pub fn build(
    table_path: &Path,
    collect_rs: &Path,
    detector: Detector,
    quiet: bool,
    out: &mut dyn std::io::Write,
) -> Result<Value> {
    let loose = detector == Detector::Loose;
    let t = Table::open(table_path)?;
    let lists = t.output_lists(detector);
    // A loose build walks the body twice, because "which of these lists can the
    // plugin actually see?" is the whole question it exists to answer. The
    // shipped offsets are the answer, and they are also what the strict-subset
    // self-check compares against.
    let shipped_offsets: HashSet<usize> = if loose {
        t.output_lists(Detector::Shipped)
            .iter()
            .map(|(lo, _)| *lo)
            .collect()
    } else {
        lists.iter().map(|(lo, _)| *lo).collect()
    };
    let recs = t.records();
    let starts: Vec<usize> = recs.iter().map(|r| r.0).collect();
    let blocks: u64 = lists.iter().map(|(_, c)| *c as u64).sum();

    let owner = |off: usize| -> Option<&(usize, u32, String)> {
        let i = starts.partition_point(|&s| s <= off);
        if i == 0 {
            None
        } else {
            Some(&recs[i - 1])
        }
    };

    let collect = read_collect(collect_rs)?;
    let mut sources: BTreeMap<u64, Vec<Src>> = BTreeMap::new();
    let mut loose_only_lists: Vec<LooseList> = Vec::new();
    let mut unattributed = 0u64;
    for &(lo, c) in &lists {
        let Some((hdr, key, name)) = owner(lo) else {
            unattributed += 1;
            continue;
        };
        let (hdr, key, name) = (*hdr, *key, name.as_str());
        let fam = collect.get(&key).map(|(_, f)| f.clone());
        let vis = if shipped_offsets.contains(&lo) {
            "shipped"
        } else {
            "loose-only"
        };
        let mut rows: Vec<(u64, u32, u32, u64, u64)> = Vec::new();
        for k in 0..c as usize {
            let bo = lo + 4 + k * BLOCK;
            let item = t.u32(bo + ITEM_AT) as u64;
            let entry_key = t.u32(bo + ENTRY_KEY_AT);
            let pad = t.u32(bo + PAD_AT);
            let src = Src {
                record_key: key,
                record_name: name.to_string(),
                min: t.u64(bo + MIN_AT),
                max: t.u64(bo + MAX_AT),
                family: fam.clone(),
                record_offset: hdr,
                block_rel: bo - hdr,
                // Only a loose build carries these, so the default outputs stay
                // byte-for-byte what they have always been.
                visibility: if loose { vis } else { "" },
                entry_key,
            };
            if loose {
                rows.push((item, entry_key, pad, src.min, src.max));
            }
            sources.entry(item).or_default().push(src);
        }
        if loose && vis == "loose-only" {
            loose_only_lists.push(LooseList {
                list_offset: lo,
                record_key: key,
                record_name: name.to_string(),
                family: fam.clone(),
                blocks: c,
                block_rel: lo - hdr,
                entries: rows
                    .iter()
                    .map(|&(i, ek, pad, mn, mx)| LooseEntry {
                        item: i,
                        min: mn,
                        max: mx,
                        entry_key: ek,
                        pad,
                        implausible_id: i > MAX_PLAUSIBLE_ITEM_ID,
                    })
                    .collect(),
            });
        }
    }

    let res = Res::default();
    let names_only: BTreeMap<u64, Vec<String>> = sources
        .iter()
        .map(|(k, v)| (*k, v.iter().map(|s| s.record_name.clone()).collect()))
        .collect();
    let named = infer_names(&res, &names_only);
    // The name inference weighs a token by how many *different* items mention
    // it, so a wider corpus can hand an id a different name: with the chests in
    // scope, item 53 stops being "itembox". Re-running the inference over the
    // shipped sources alone is what lets an entry say which name the default
    // doc gives it, instead of quietly disagreeing with that file.
    let shipped_named: HashMap<u64, Named> = if loose {
        let only_shipped: BTreeMap<u64, Vec<String>> = sources
            .iter()
            .map(|(k, v)| {
                (
                    *k,
                    v.iter()
                        .filter(|s| s.visibility == "shipped")
                        .map(|s| s.record_name.clone())
                        .collect::<Vec<String>>(),
                )
            })
            .filter(|(_, v)| !v.is_empty())
            .collect();
        infer_names(&res, &only_shipped)
    } else {
        HashMap::new()
    };

    let mut items: BTreeMap<u64, Map<String, Value>> = BTreeMap::new();
    for (&item, srcs) in &sources {
        let fams: BTreeSet<&str> = srcs
            .iter()
            .filter_map(|s| s.family.as_deref())
            .collect();
        // Insertion-ordered fold by record key: the JSON's `sources` array is
        // this, sorted by name, and a stable sort keeps first-seen order for
        // records that share a name.
        let mut order: Vec<u32> = Vec::new();
        let mut by_key: HashMap<u32, Map<String, Value>> = HashMap::new();
        for s in srcs {
            let e = by_key.entry(s.record_key).or_insert_with(|| {
                order.push(s.record_key);
                let mut m = Map::new();
                m.insert("key".into(), s.record_key.into());
                m.insert("name".into(), s.record_name.clone().into());
                m.insert("family".into(), opt_str(&s.family));
                m.insert("min".into(), s.min.into());
                m.insert("max".into(), s.max.into());
                m.insert("blocks".into(), Value::from(0u64));
                m.insert("record_offset".into(), s.record_offset.into());
                m.insert("block_rel".into(), Value::Array(Vec::new()));
                m
            });
            let b = e["blocks"].as_u64().unwrap() + 1;
            e["blocks"] = b.into();
            let mn = e["min"].as_u64().unwrap().min(s.min);
            e["min"] = mn.into();
            let mx = e["max"].as_u64().unwrap().max(s.max);
            e["max"] = mx.into();
            e["block_rel"]
                .as_array_mut()
                .unwrap()
                .push(s.block_rel.into());
            if loose {
                e.entry("visibility")
                    .or_insert_with(|| Value::from(s.visibility));
                let ks = e
                    .entry("entry_keys")
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .unwrap();
                let v = Value::from(s.entry_key);
                if !ks.contains(&v) {
                    ks.push(v);
                }
            }
        }
        let mut src_vals: Vec<Map<String, Value>> =
            order.iter().map(|k| by_key.remove(k).unwrap()).collect();
        src_vals.sort_by_key(|m| m["name"].as_str().unwrap().to_lowercase());

        let n = &named[&item];
        let mut info = Map::new();
        info.insert("name".into(), n.name.clone().into());
        info.insert("confidence".into(), n.confidence.into());
        info.insert("named_by".into(), n.named_by.clone().into());
        info.insert("agreeing_records".into(), n.agreeing.clone().into());
        info.insert("conflicting_records".into(), n.conflicting.clone().into());
        info.insert(
            "note".into(),
            match n.note {
                Some(s) => Value::from(s),
                None => Value::Null,
            },
        );
        info.insert("id".into(), item.into());
        info.insert("blocks".into(), srcs.len().into());
        info.insert("records".into(), src_vals.len().into());
        info.insert(
            "min".into(),
            srcs.iter().map(|s| s.min).min().unwrap().into(),
        );
        info.insert(
            "max".into(),
            srcs.iter().map(|s| s.max).max().unwrap().into(),
        );
        info.insert(
            "fixed_amount".into(),
            srcs.iter().all(|s| s.min == s.max).into(),
        );
        info.insert(
            "families".into(),
            fams.iter().map(|f| Value::from(*f)).collect::<Vec<_>>().into(),
        );
        info.insert("classified".into(), (!fams.is_empty()).into());
        info.insert(
            "sources".into(),
            src_vals.into_iter().map(Value::Object).collect::<Vec<_>>().into(),
        );
        if loose {
            let shipped_blocks = srcs.iter().filter(|s| s.visibility == "shipped").count();
            // An item is "shipped" as soon as **one** block of it is in the
            // population the plugin sees: the gatherer would reach it there.
            // "loose-only" means no block of it is, i.e. the mod is blind to
            // this id entirely.
            info.insert(
                "visibility".into(),
                if shipped_blocks > 0 { "shipped" } else { "loose-only" }.into(),
            );
            info.insert("shipped_blocks".into(), shipped_blocks.into());
            info.insert(
                "loose_only_blocks".into(),
                (srcs.len() - shipped_blocks).into(),
            );
            info.insert(
                "implausible_id".into(),
                (item > MAX_PLAUSIBLE_ITEM_ID).into(),
            );
            if let Some(other) = shipped_named.get(&item).map(|x| x.name.as_str()) {
                if other != n.name {
                    info.insert("name_in_default_doc".into(), other.into());
                }
            }
        }
        items.insert(item, info);
    }

    // An inferred name landing on two ids is the interesting case, not a bug:
    // the game gives the same substance a different id per source (foraged
    // ginseng vs the trade prop). Make it visible on both entries.
    let mut by_name: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for (id, info) in &items {
        by_name
            .entry(info["name"].as_str().unwrap().to_string())
            .or_default()
            .push(*id);
    }
    for ids in by_name.values() {
        if ids.len() > 1 {
            for &i in ids {
                let others: Vec<Value> =
                    ids.iter().filter(|&&x| x != i).map(|&x| Value::from(x)).collect();
                items.get_mut(&i).unwrap().insert("name_shared_with".into(), others.into());
            }
        }
    }

    // One extra honesty rule, and it applies to loose-only items only - both
    // because the default outputs must not move and because this is the shape
    // the loose-only records actually have. `Action_dig_01` yields two ids and
    // `gimmick_Dig_land_0001` five; the inference calls all seven "action_dig"
    // or "dig_land", each `single` because nothing contradicts it. Nothing
    // contradicts it because nothing else mentions them at all. A name one
    // record hands to several different ids names **the source**, not the
    // goods, so it is a guess whatever the agreement count says.
    if loose {
        for info in items.values_mut() {
            let shared = info.get("name_shared_with").map(|v| v.as_array().unwrap().len());
            let conf = info["confidence"].as_str().unwrap().to_string();
            if info["visibility"] == "loose-only"
                && shared.is_some_and(|n| n > 0)
                && conf != "curated"
                && conf != "guess"
            {
                let nshared = shared.unwrap();
                info.insert("confidence_before_loose_rule".into(), conf.into());
                info["confidence"] = "guess".into();
                info.insert(
                    "confidence_downgraded".into(),
                    format!(
                        "the records naming this id name {nshared} other id{} the same way, \
                         so the name is the source and not the goods",
                        if nshared > 1 { "s" } else { "" }
                    )
                    .into(),
                );
            }
        }
    }

    let checks = run_checks(
        &t,
        &lists,
        blocks,
        recs.len(),
        &items,
        &recs,
        &starts,
        detector,
        &shipped_offsets,
        &loose_only_lists,
    );

    let covered_blocks: u64 = lists
        .iter()
        .filter(|&&(lo, _)| owner(lo).is_some_and(|h| collect.contains_key(&h.1)))
        .map(|&(_, c)| c as u64)
        .sum();
    let covered_records = lists
        .iter()
        .filter_map(|&(lo, _)| owner(lo))
        .filter(|h| collect.contains_key(&h.1))
        .map(|h| h.1)
        .collect::<HashSet<u32>>()
        .len();
    let records_with_output = lists
        .iter()
        .filter_map(|&(lo, _)| owner(lo))
        .map(|h| h.1)
        .collect::<HashSet<u32>>()
        .len();

    let mut confidence_levels = Map::new();
    for (k, v) in CONFIDENCE_DOC {
        confidence_levels.insert((*k).into(), (*v).into());
    }
    let mut totals = Map::new();
    totals.insert("output_lists".into(), lists.len().into());
    totals.insert("blocks".into(), blocks.into());
    totals.insert("records_in_table".into(), recs.len().into());
    totals.insert("records_with_output".into(), records_with_output.into());
    totals.insert("distinct_items".into(), items.len().into());
    totals.insert("unattributed_lists".into(), unattributed.into());
    totals.insert("blocks_covered_by_collect_rs".into(), covered_blocks.into());
    totals.insert("records_covered_by_collect_rs".into(), covered_records.into());
    totals.insert(
        "items_classified".into(),
        items.values().filter(|i| i["classified"] == true).count().into(),
    );
    totals.insert(
        "items_unclassified".into(),
        items.values().filter(|i| i["classified"] == false).count().into(),
    );

    let mut data = Map::new();
    data.insert("source".into(), table_path.display().to_string().into());
    data.insert("source_bytes".into(), t.b.len().into());
    data.insert("generator".into(), super::GENERATOR.into());
    data.insert(
        "method".into(),
        "Blocks found by the desert_core::gimmick::output_lists detector; \
         item id read at block+1 (echoed at +60). Blocks attributed to \
         records by the key-echo discriminator. Item NAMES are INFERRED \
         from the names of the records that yield them - the game's own \
         item names live in the .paz archives and are not read here."
            .into(),
    );
    data.insert("confidence_levels".into(), Value::Object(confidence_levels));
    data.insert("totals".into(), Value::Object(totals));
    data.insert(
        "checks".into(),
        checks.iter().map(|s| Value::from(s.as_str())).collect::<Vec<_>>().into(),
    );
    data.insert(
        "items".into(),
        items.values().cloned().map(Value::Object).collect::<Vec<_>>().into(),
    );

    if loose {
        // Everything below is loose-only, and so is every extra field on an
        // item and on a source. The default build writes none of it, which is
        // what keeps analysis/items.json byte-identical to what it always was.
        let lo_items = items.values().filter(|i| i["visibility"] == "loose-only").count();
        let bad: Vec<u64> = items
            .iter()
            .filter(|(_, i)| i["implausible_id"] == true)
            .map(|(id, _)| *id)
            .collect();
        let mut det = Map::new();
        det.insert("name".into(), detector.name().into());
        det.insert("summary".into(), detector.summary().into());
        det.insert("pad_clause".into(), detector.pad_clause().into());
        data.insert("detector".into(), Value::Object(det));
        data.insert(
            "warning".into(),
            "LOOSE POPULATION. This file is the wider walk: the pad clause of \
             desert_core::gimmick::block_ok is dropped, so it contains content \
             the plugin CANNOT SEE. Any item or source marked \
             visibility=loose-only is invisible to the gatherer and no \
             multiplier reaches it. It is also less trustworthy than \
             analysis/items.json: see implausible_item_ids below, and \
             docs/reference-items-loose.md for the caveats in full."
                .into(),
        );
        let tot = data["totals"].as_object_mut().unwrap();
        tot.insert("shipped_lists".into(), shipped_offsets.len().into());
        tot.insert("loose_only_lists".into(), loose_only_lists.len().into());
        tot.insert(
            "loose_only_blocks".into(),
            loose_only_lists.iter().map(|l| l.blocks as u64).sum::<u64>().into(),
        );
        tot.insert("items_shipped_visible".into(), (items.len() - lo_items).into());
        tot.insert("items_loose_only".into(), lo_items.into());
        tot.insert("implausible_item_ids".into(), bad.len().into());

        let mut homes: BTreeSet<&str> = BTreeSet::new();
        for id in &bad {
            for s in items[id]["sources"].as_array().unwrap() {
                homes.insert(s["name"].as_str().unwrap());
            }
        }
        let mut imp = Map::new();
        imp.insert(
            "ids".into(),
            bad.iter().map(|&i| Value::from(i)).collect::<Vec<_>>().into(),
        );
        imp.insert(
            "note".into(),
            format!(
                "Every id in the shipped population is eight digits or fewer \
                 (largest 10021720). These are past {} and are reported, not \
                 filtered: they mean the list they sit in is being mis-parsed. \
                 Every one of them is inside {}, so the damage is known to be \
                 local - but it is proof the loose walk is not uniformly \
                 right, and no id that only this walk can see should be used \
                 without checking the record it came from.",
                super::commas(MAX_PLAUSIBLE_ITEM_ID),
                super::checks::IMPLAUSIBLE_HOME
            )
            .into(),
        );
        imp.insert(
            "records".into(),
            homes.iter().map(|s| Value::from(*s)).collect::<Vec<_>>().into(),
        );
        data.insert("implausible_item_ids".into(), Value::Object(imp));
        data.insert(
            "loose_confidence_rule".into(),
            "A loose-only item whose inferred name is shared with another id \
             is forced to `guess`, whatever its agreement count: a name one \
             record hands to several ids names the source and not the goods. \
             The pre-rule level is kept as confidence_before_loose_rule. This \
             rule is not applied to the shipped population, which is why \
             analysis/items.json is unchanged by it."
                .into(),
        );
        let mut ll: Vec<&LooseList> = loose_only_lists.iter().collect();
        ll.sort_by_key(|l| l.record_name.to_lowercase());
        data.insert(
            "loose_only_lists".into(),
            ll.iter().map(|l| l.to_value()).collect::<Vec<_>>().into(),
        );
    }

    if !quiet {
        for line in &checks {
            writeln!(out, "{line}")?;
        }
    }
    Ok(Value::Object(data))
}
