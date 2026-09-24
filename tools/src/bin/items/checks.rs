//! The self-checks every run prints.
//!
//! Never fatal: a `FAIL` **is** the report, and it means the table or the walk
//! moved. Nothing downstream should be trusted until a `FAIL` is explained -
//! not the doc, not the JSON, and not a family decision taken off either.

use std::collections::{BTreeSet, HashSet};

use serde_json::{Map, Value};

use super::dataset::LooseList;
use super::table::{Detector, Table, BLOCK, PAD_AT};

// Expected results. The scan prints a FAIL line for any of these that moves;
// all five were confirmed three independent ways in findings section 9, on
// build 25246367. Measured again on `paths::CALIBRATED_BUILD` (25477059), the
// body `gen-collect-names`' CALIBRATION is taken on: 573 / 896 / 13412 became
// 572 / 895 / 13447 (`itembox_11`, key 1012375, lost its only block; 35
// records were added, none with blocks). The item count did not move.
pub const EXPECT_LISTS: usize = 572;
pub const EXPECT_BLOCKS: u64 = 895;
pub const EXPECT_ITEMS: usize = 215;
pub const EXPECT_RECORDS: usize = 13447;
// The same three for the loose walk, plus what must stay true *between* the two
// populations: every shipped list is also a loose list, and the extras are
// exactly the blocks with a nonzero entry key. Build 25246367 was 589 / 1038;
// the lost `itembox_11` block was in both populations, so the loose-only
// counts below did not move.
pub const EXPECT_LOOSE_LISTS: usize = 588;
pub const EXPECT_LOOSE_BLOCKS: u64 = 1037;
pub const EXPECT_LOOSE_ITEMS: usize = 311;
pub const EXPECT_LOOSE_ONLY_LISTS: usize = EXPECT_LOOSE_LISTS - EXPECT_LISTS; // 16
pub const EXPECT_LOOSE_ONLY_BLOCKS: u64 = EXPECT_LOOSE_BLOCKS - EXPECT_BLOCKS; // 142
// Nine ids in `391518521..391518546`, all nine inside the one list. They are
// the reason the loose population is reported as less trustworthy than the
// default.
pub const EXPECT_IMPLAUSIBLE_IDS: usize = 9;
pub const IMPLAUSIBLE_HOME: &str = "gimmick_item_dropset_treasurebox_01";
// 101 ids occur in the 142 loose-only blocks. Five of them also occur in the
// shipped population, so 96 items are ones the mod is blind to outright. Five
// out of 101 is not corroboration and is not meant to read as any: chests yield
// different things from gather nodes, so a low overlap is exactly what a
// correct walk would produce too. It is recorded because a *move* in it is
// worth noticing.
pub const EXPECT_LOOSE_ONLY_ITEMS: usize = 96;
pub const EXPECT_SHARED_IDS: &[u64] = &[1, 53, 75001, 1001597, 1001957];

/// `(list offset, record name, record-relative offset)`, three anchors the
/// attribution has to keep hitting. The first matches the Logging pack's own
/// patch offset (12921585 = list + 4 + 42). Build 25246367 had the lists at
/// 12843209, 4373870 and 1049316; the relative offsets have not moved.
pub const ANCHORS: &[(usize, &str, usize)] = &[
    (12921539, "firewood_0001", 1999),
    (4382086, "gimmick_well_0001_parts01", 965),
    (1051437, "Background_Breakable_66", 1600),
];

/// Item id -> the inferred name the doc confirmed by two agreeing records each.
/// Checked, not trusted: a disagreement means attribution is off. The order is
/// the order the failures are reported in, so keep it.
pub const ANCHOR_NAMES: &[(u64, &str)] = &[
    (1000648, "salt"),
    (1000602, "sugar"),
    (1000646, "flour"),
    (1000608, "pepper"),
    (1000667, "cheese"),
    (1000668, "fishmeat"),
    (1000666, "ginseng"),
    (1000603, "honey"),
    (1000647, "wine"),
    (740001, "leather"),
    (757006, "peony"),
    (710001, "firewood"),
    (720004, "copper"),
    (756802, "weed_sophora"),
];

/// Python's `repr()` of a list of ints, which is what the check lines print.
fn repr_ints(v: &[u64]) -> String {
    format!(
        "[{}]",
        v.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
    )
}

/// Python's `repr()` of a list of str. These names are ASCII with no quotes in
/// them, so the simple form is the right one.
fn repr_strs(v: &[&str]) -> String {
    format!(
        "[{}]",
        v.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(", ")
    )
}

/// Self-checks against the numbers findings section 9 confirmed. Never fatal:
/// a FAIL is the report, and it means the table or the walk moved.
///
/// The loose walk gets the same treatment - its own three counts, plus the two
/// relations that make it a *superset* of the shipped one rather than a
/// different answer to the same question.
#[allow(clippy::too_many_arguments)]
pub fn run_checks(
    t: &Table,
    lists: &[(usize, u32)],
    blocks: u64,
    n_records: usize,
    items: &std::collections::BTreeMap<u64, Map<String, Value>>,
    recs: &[(usize, u32, String)],
    starts: &[usize],
    detector: Detector,
    shipped_offsets: &HashSet<usize>,
    loose_only_lists: &[LooseList],
) -> Vec<String> {
    let loose = detector == Detector::Loose;
    let mut out: Vec<String> = Vec::new();

    macro_rules! chk {
        ($label:expr, $got:expr, $want:expr) => {{
            let (got, want) = ($got, $want);
            let ok = got == want;
            out.push(format!(
                "{} {}: {}{}",
                if ok { "ok  " } else { "FAIL" },
                $label,
                got,
                if ok { String::new() } else { format!(" (expected {want})") }
            ));
        }};
    }
    macro_rules! note {
        ($ok:expr, $label:expr, $detail:expr) => {
            out.push(format!(
                "{} {}: {}",
                if $ok { "ok  " } else { "FAIL" },
                $label,
                $detail
            ))
        };
    }

    if loose {
        // Only the loose run announces its detector: the default run's check
        // lines are copied verbatim into docs/reference-items.md, and that file
        // is meant to stay byte-identical to what it has always been.
        out.push(format!(
            "--   detector: {} ({})",
            detector.name(),
            detector.summary()
        ));
    }
    chk!(
        "output lists",
        lists.len(),
        if loose { EXPECT_LOOSE_LISTS } else { EXPECT_LISTS }
    );
    chk!(
        "blocks",
        blocks,
        if loose { EXPECT_LOOSE_BLOCKS } else { EXPECT_BLOCKS }
    );
    chk!(
        "distinct item ids",
        items.len(),
        if loose { EXPECT_LOOSE_ITEMS } else { EXPECT_ITEMS }
    );
    chk!("records", n_records, EXPECT_RECORDS);

    if loose {
        let offs: HashSet<usize> = lists.iter().map(|(lo, _)| *lo).collect();
        let missing: BTreeSet<usize> = shipped_offsets.difference(&offs).copied().collect();
        note!(
            missing.is_empty() && shipped_offsets.len() == EXPECT_LISTS,
            "shipped lists are a strict subset",
            format!(
                "{}/{} shipped lists all present{}",
                shipped_offsets.len(),
                offs.len(),
                match missing.iter().next() {
                    None => String::new(),
                    Some(first) => format!("; {} MISSING, first at {first}", missing.len()),
                }
            )
        );
        chk!("loose-only lists", loose_only_lists.len(), EXPECT_LOOSE_ONLY_LISTS);
        chk!(
            "loose-only blocks",
            loose_only_lists.iter().map(|l| l.blocks as u64).sum::<u64>(),
            EXPECT_LOOSE_ONLY_BLOCKS
        );
        // The discriminator itself: every extra block carries a nonzero entry
        // key at +64 and a zero pad at +5. If that ever stops holding, the two
        // populations differ for some other reason and nothing below is safe.
        let ents: Vec<&super::dataset::LooseEntry> =
            loose_only_lists.iter().flat_map(|l| l.entries.iter()).collect();
        let nz = ents.iter().filter(|e| e.entry_key != 0).count();
        let distinct: HashSet<u32> = ents.iter().map(|e| e.entry_key).collect();
        note!(
            nz == ents.len() && ents.len() as u64 == EXPECT_LOOSE_ONLY_BLOCKS,
            "every loose-only block has a nonzero +64",
            format!("{nz}/{}, {} distinct", ents.len(), distinct.len())
        );
        let pads = lists
            .iter()
            .flat_map(|&(lo, c)| (0..c as usize).map(move |k| lo + 4 + k * BLOCK))
            .filter(|&bo| t.u32(bo + PAD_AT) == 0)
            .count() as u64;
        note!(
            pads == blocks,
            "+5 is zero on every block of both populations",
            format!("{pads}/{blocks}")
        );
        chk!(
            "items only this walk can see",
            items.values().filter(|e| e["visibility"] == "loose-only").count(),
            EXPECT_LOOSE_ONLY_ITEMS
        );
        let lo_ids: BTreeSet<u64> = loose_only_lists
            .iter()
            .flat_map(|l| l.entries.iter().map(|e| e.item))
            .collect();
        let shared: Vec<u64> = lo_ids
            .iter()
            .copied()
            .filter(|i| items[i]["visibility"] == "shipped")
            .collect();
        note!(
            shared == EXPECT_SHARED_IDS,
            "ids in loose-only blocks that the mod also sees",
            format!(
                "{} of {}: {}{}",
                shared.len(),
                lo_ids.len(),
                repr_ints(&shared),
                if shared == EXPECT_SHARED_IDS {
                    String::new()
                } else {
                    format!(" (expected {})", repr_ints(EXPECT_SHARED_IDS))
                }
            )
        );
        let bad: Vec<u64> = items
            .iter()
            .filter(|(_, e)| e.get("implausible_id").is_some_and(|v| v == true))
            .map(|(i, _)| *i)
            .collect();
        let homes: Vec<&str> = bad
            .iter()
            .flat_map(|i| items[i]["sources"].as_array().unwrap())
            .map(|s| s["name"].as_str().unwrap())
            .collect::<BTreeSet<&str>>()
            .into_iter()
            .collect();
        let homes_ok = homes == [IMPLAUSIBLE_HOME];
        note!(
            bad.len() == EXPECT_IMPLAUSIBLE_IDS && homes_ok,
            "implausible item ids are confined to one list",
            format!(
                "{} ids in {}{}",
                bad.len(),
                if homes.is_empty() {
                    repr_strs(&["none"])
                } else {
                    repr_strs(&homes)
                },
                if bad.len() != EXPECT_IMPLAUSIBLE_IDS || !homes_ok {
                    format!(" (expected {EXPECT_IMPLAUSIBLE_IDS} in {IMPLAUSIBLE_HOME})")
                } else {
                    String::new()
                }
            )
        );
    }

    for &(off, name, rel) in ANCHORS {
        let i = starts.partition_point(|&s| s <= off);
        let got = if i == 0 {
            "unattributed".to_string()
        } else {
            let h = &recs[i - 1];
            format!("{} rel +{}", h.2, off - h.0)
        };
        let want = format!("{name} rel +{rel}");
        out.push(format!(
            "{} anchor list@{off}: {got}{}",
            if got == want { "ok  " } else { "FAIL" },
            if got == want {
                String::new()
            } else {
                format!(" (expected {want})")
            }
        ));
    }

    let mut bad: Vec<String> = Vec::new();
    for &(item, want) in ANCHOR_NAMES {
        let got = items.get(&item).and_then(|i| i["name"].as_str());
        if got != Some(want) {
            bad.push(format!(
                "{item} -> {} (expected '{want}')",
                match got {
                    Some(g) => format!("'{g}'"),
                    None => "None".to_string(),
                }
            ));
        }
    }
    out.push(format!(
        "{} anchor names: {}/{} agree{}",
        if bad.is_empty() { "ok  " } else { "FAIL" },
        ANCHOR_NAMES.len() - bad.len(),
        ANCHOR_NAMES.len(),
        if bad.is_empty() {
            String::new()
        } else {
            format!("; {}", bad.join(", "))
        }
    ));
    out
}
