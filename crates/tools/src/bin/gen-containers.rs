//! Generate `crates/desert-core/src/container_table.rs`: every take-able
//! container by gimmick key.
//!
//! Usage: `gen-containers [--tables DIR] [--out FILE] [--build ID]`
//!
//! Reads `gimmickinfo` and `interactioninfo` out of the installed game's
//! archives (`desert_tools::paz`, `paths::game_dir`) or a directory of
//! extracted copies (`--tables`, what `just extract-tables` writes into
//! `analysis/tables/`). Each gimmick record is walked from its key through its
//! tag list (`walk`, a port of the first part of `analysis/gimmick_tail.py`'s
//! `walk()`), and each interaction's `checkLootableField` read off its head
//! (`gimmick_tail.py` `interaction_flags()`).
//!
//! All or nothing: every `interactioninfo` record must pass its head check,
//! every `gimmickinfo` record must walk to its tags with the checks `walk`
//! names, every override's interaction key must resolve, and the spot checks
//! must hold, or nothing is written.
//!
//! The rule is `analysis/container_catalog.py`'s, first rule met, for its
//! first seven classes; the first five only take a record out of the two
//! emitted here:
//!
//! 1. `Gimmick_Examine_PuzzleBox` among the interactions: a puzzle box.
//! 2. first interaction `Gimmick_PushDown_Camera` or `Gimmick_Move_Camera`: a
//!    puzzle-box part.
//! 3. a lootable interaction (`checkLootableField` 1) whose name contains
//!    `Chest_Open`: a chest.
//! 4. `Furniture_Open` among the interactions: furniture storage.
//! 5. `Furniture_Examine` among the interactions: personal storage.
//! 6. a lootable interaction whose name starts with a take verb ([`TAKE`]) and
//!    the tag `furniture_drop`: **`FurnitureDrop`**.
//! 7. a lootable take interaction and the tag `catch_itemcontainer`:
//!    **`ItemContainer`**.
//!
//! It prints the counts, the records the earlier classes took that carry one
//! of the two tags, and what changed against the file it replaces.

#[path = "gen-containers/render.rs"]
mod render;
#[path = "gen-containers/walk.rs"]
mod walk;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;
use desert_tools::paths;
use desert_tools::paz::Archive;
use desert_tools::staticinfo::{self, Table};

const GIMMICK: &str = "gimmickinfo";
const INTERACTION: &str = "interactioninfo";

/// The take verbs (`container_catalog.py` `TAKE`), matched as prefixes.
const TAKE: [&str; 6] =
    ["Gimmick_PickUp", "ItemHot_Collect", "ItemCold_Collect", "Item_Collect_Reward", "Ship_PickUp", "ReadMemory_Paper_Collect"];
const CHEST: &str = "Chest_Open";
const FURNITURE: &str = "Furniture_Open";
const PERSONAL: &str = "Furniture_Examine";
const PUZZLE: &str = "Gimmick_Examine_PuzzleBox";
const PUZZLE_PART: [&str; 2] = ["Gimmick_PushDown_Camera", "Gimmick_Move_Camera"];
const TAG_FURNITURE_DROP: &str = "furniture_drop";
const TAG_ITEM_CONTAINER: &str = "catch_itemcontainer";

/// The checks the generator makes before writing, and emits as a test:
/// `(gimmick key, record name, class)`. The negatives are one record of each
/// class `container_catalog.py` keeps out (classes.tsv on 26361232).
pub const SPOT_CHECKS: &[(u32, &str, Option<Class>)] = &[
    (1005717, "gimmick_box_donation_reward_coin_01", Some(Class::ItemContainer)),
    (1004116, "gimmick_breakable_sack_06", Some(Class::ItemContainer)),
    (1003953, "in_shops_hppotionwoodbox_01", Some(Class::ItemContainer)),
    (1000155, "Background_Breakable_Wood_209", Some(Class::ItemContainer)),
    (1004949, "gimmick_item_dropset_cabinet_01", Some(Class::FurnitureDrop)),
    (1006434, "gimmick_item_dropset_armor_fabric_03", Some(Class::FurnitureDrop)),
    (1000425, "treasurebox_small_01", None),
    (1006143, "gimmick_in_dff_cabinet_27_parts01", None),
    (1000022, "Collection_Prop_Trophy_0001", None),
    (1000132, "pot_0013", None),
    (1000285, "basecamp_warehouse_box_01_cover", None),
    (1000838, "gimmick_treasurebox_padlock_safe_01", None),
];

#[derive(Parser)]
#[command(about = "Generate crates/desert-core/src/container_table.rs from the game's gimmickinfo and interactioninfo")]
struct Cli {
    /// A directory of extracted tables (`<table>.staticinfobody` and
    /// `.staticinfoheader`, as `just extract-tables` writes) to read instead of
    /// the game's archives [default: the archives under $CD_GAME_DIR, else
    /// the parent of $CD_BIN64]
    #[arg(long, value_name = "DIR")]
    tables: Option<PathBuf>,
    /// Where to write [default: <repo>/crates/desert-core/src/container_table.rs]
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// The game build named in the header [default: the appmanifest's, else
    /// the calibrated build]
    #[arg(long, value_name = "ID")]
    build: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    ItemContainer,
    FurnitureDrop,
}

impl Class {
    pub fn variant(self) -> &'static str {
        match self {
            Class::ItemContainer => "ItemContainer",
            Class::FurnitureDrop => "FurnitureDrop",
        }
    }
}

/// One emitted row.
pub struct Container {
    pub key: u32,
    pub name: String,
    pub class: Class,
}

/// What the rule makes of one record: one of the two classes, an earlier
/// class that keeps it out (named, for the report), or nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Emit(Class),
    TakenEarlier(&'static str),
    None,
}

/// The rule, over a record's interaction names (in override order), which of
/// them are lootable, and its tags. See the module comment.
pub fn rule(names: &[&str], lootable: &[bool], tags: &[String]) -> Verdict {
    let loot: Vec<&str> = names.iter().zip(lootable).filter(|(_, l)| **l).map(|(n, _)| *n).collect();
    let first = names.first().copied().unwrap_or("");
    if names.contains(&PUZZLE) {
        return Verdict::TakenEarlier("Treasure/puzzle box");
    }
    if PUZZLE_PART.contains(&first) {
        return Verdict::TakenEarlier("Puzzle-box part");
    }
    if loot.iter().any(|n| n.contains(CHEST)) {
        return Verdict::TakenEarlier("Chest");
    }
    if names.contains(&FURNITURE) {
        return Verdict::TakenEarlier("Furniture storage");
    }
    if names.contains(&PERSONAL) {
        return Verdict::TakenEarlier("Personal storage");
    }
    let take = loot.iter().any(|n| TAKE.iter().any(|t| n.starts_with(t)));
    let has = |t: &str| tags.iter().any(|x| x == t);
    if take && has(TAG_FURNITURE_DROP) {
        Verdict::Emit(Class::FurnitureDrop)
    } else if take && has(TAG_ITEM_CONTAINER) {
        Verdict::Emit(Class::ItemContainer)
    } else {
        Verdict::None
    }
}

enum Source {
    Archive(Archive),
    Dir(PathBuf),
}

impl Source {
    fn read(&self, table: &str) -> Result<Table> {
        match self {
            Source::Archive(a) => staticinfo::read_from_archive(a, table)
                .with_context(|| format!("cannot read {table} out of the archives under {}", a.game_dir.display())),
            Source::Dir(d) => staticinfo::read_from_dir(d, table)
                .with_context(|| format!("cannot read {table} from {}", d.display())),
        }
    }

    fn describe(&self) -> String {
        match self {
            Source::Archive(a) => format!("the archives under {}", a.game_dir.display()),
            Source::Dir(d) => d.display().to_string(),
        }
    }
}

/// Every record of a table as `(key, bytes)`.
fn records_of(name: &str, t: &Table) -> Result<Vec<(u32, Vec<u8>)>> {
    let recs = t.split().with_context(|| format!("{name}: the header does not split the body"))?;
    recs.iter()
        .map(|r| {
            let key = u32::try_from(r.key).with_context(|| format!("{name}: key {} is wider than u32", r.key))?;
            let bytes = t.body.get(r.offset..r.end).with_context(|| format!("{name}: record {} out of range", r.index))?;
            Ok((key, bytes.to_vec()))
        })
        .collect()
}

/// A `NameFirst` record's name: `u32 len`, bytes.
fn record_name(rec: &[u8]) -> Option<String> {
    let n = u32::from_le_bytes(rec.get(0..4)?.try_into().ok()?) as usize;
    Some(String::from_utf8_lossy(rec.get(4..4 + n)?).into_owned())
}

/// Report up to a handful of failures and bail, the all-or-nothing check.
fn refuse(what: &str, fails: &[String], total: usize) -> Result<()> {
    if fails.is_empty() {
        return Ok(());
    }
    let shown: Vec<&str> = fails.iter().take(10).map(String::as_str).collect();
    bail!(
        "{what}: {} of {total} records fail the layout check; nothing written. First ones:\n  {}",
        fails.len(),
        shown.join("\n  ")
    )
}

fn run(cli: &Cli) -> Result<()> {
    let root = paths::repo_root()?;
    let out_path =
        cli.out.clone().unwrap_or_else(|| paths::crate_dir_in(&root, "desert-core").join("src").join("container_table.rs"));
    let build = cli.build.clone().or_else(paths::build_id).unwrap_or_else(|| paths::CALIBRATED_BUILD.to_string());
    let src = match &cli.tables {
        Some(d) => Source::Dir(d.clone()),
        None => Source::Archive(Archive::open(&paths::game_dir())?),
    };

    // interactioninfo: key -> (name, checkLootableField).
    let inter_recs = records_of(INTERACTION, &src.read(INTERACTION)?)?;
    let mut interactions: HashMap<u32, (String, bool)> = HashMap::new();
    let mut fails = Vec::new();
    for (key, rec) in &inter_recs {
        match walk::interaction(rec, *key) {
            Ok(v) => {
                interactions.insert(*key, v);
            }
            Err(e) => fails.push(format!("{key}: {e}")),
        }
    }
    refuse(INTERACTION, &fails, inter_recs.len())?;
    let lootable = interactions.values().filter(|(_, l)| *l).count();

    // gimmickinfo, walked to its tags.
    let gim_recs = records_of(GIMMICK, &src.read(GIMMICK)?)?;
    let mut fails = Vec::new();
    let mut rows: Vec<Container> = Vec::new();
    let mut earlier: BTreeMap<(&'static str, &'static str), Vec<String>> = BTreeMap::new();
    let mut tagged_no_take: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut overrides = 0usize;
    let mut with_tags = 0usize;
    for (key, rec) in &gim_recs {
        let g = match walk::gimmick(rec, *key) {
            Ok(g) => g,
            Err(e) => {
                fails.push(format!("{key}: {e}"));
                continue;
            }
        };
        overrides += g.interactions.len();
        with_tags += usize::from(!g.tags.is_empty());
        let mut names = Vec::with_capacity(g.interactions.len());
        let mut loot = Vec::with_capacity(g.interactions.len());
        for k in &g.interactions {
            match interactions.get(k) {
                Some((n, l)) => {
                    names.push(n.as_str());
                    loot.push(*l);
                }
                None => fails.push(format!("{key}: override interaction {k} is not an {INTERACTION} record")),
            }
        }
        if names.len() != g.interactions.len() {
            continue;
        }
        let name = record_name(rec).unwrap_or_default();
        let tag = [TAG_FURNITURE_DROP, TAG_ITEM_CONTAINER].into_iter().find(|t| g.tags.iter().any(|x| x == t));
        match rule(&names, &loot, &g.tags) {
            Verdict::Emit(class) => rows.push(Container { key: *key, name, class }),
            Verdict::TakenEarlier(by) => {
                if let Some(t) = tag {
                    earlier.entry((t, by)).or_default().push(name);
                }
            }
            Verdict::None => {
                if let Some(t) = tag {
                    *tagged_no_take.entry(t).or_insert(0) += 1;
                }
            }
        }
    }
    refuse(GIMMICK, &fails, gim_recs.len())?;
    rows.sort_by_key(|r| r.key);
    if let Some(w) = rows.windows(2).find(|w| w[0].key == w[1].key) {
        bail!("{GIMMICK}: key {} occurs twice; nothing written", w[0].key);
    }

    let class_of = |k: u32| rows.iter().find(|r| r.key == k).map(|r| r.class);
    let name_of = |k: u32| gim_recs.iter().find(|(x, _)| *x == k).and_then(|(_, r)| record_name(r));
    let wrong: Vec<String> = SPOT_CHECKS
        .iter()
        .filter(|(k, n, want)| class_of(*k) != *want || name_of(*k).as_deref() != Some(*n))
        .map(|(k, n, want)| format!("{k} {n}: {:?} named {:?}, expected {want:?}", class_of(*k), name_of(*k)))
        .collect();
    if !wrong.is_empty() {
        bail!("spot checks fail; nothing written:\n  {}", wrong.join("\n  "));
    }

    let count = |c: Class| rows.iter().filter(|r| r.class == c).count();
    let counts =
        render::Counts { item_container: count(Class::ItemContainer), furniture_drop: count(Class::FurnitureDrop) };
    let previous = fs::read_to_string(&out_path).map(|t| render::previous_rows(&t)).unwrap_or_default();
    let text = render::container_table_rs(&rows, &counts, &build);
    fs::write(&out_path, text).with_context(|| format!("cannot write {}", out_path.display()))?;

    // The report.
    println!("source: {}", src.describe());
    println!("build: {build}");
    println!(
        "records: {} {GIMMICK} (walked to their tags; {overrides} interaction overrides, {with_tags} with tags), \
         {} {INTERACTION} ({lootable} with checkLootableField 1); every one passes the layout check",
        gim_recs.len(),
        inter_recs.len()
    );
    println!(
        "rows: {} = ItemContainer {}, FurnitureDrop {}",
        rows.len(),
        counts.item_container,
        counts.furniture_drop
    );
    for ((tag, by), names) in &earlier {
        println!("tag {tag} on {} records an earlier class took ({by}): {}", names.len(), names.join(", "));
    }
    for (tag, n) in &tagged_no_take {
        println!("tag {tag} on {n} records with no lootable take interaction: not emitted");
    }
    let now: BTreeMap<u32, &str> = rows.iter().map(|r| (r.key, r.class.variant())).collect();
    let added: Vec<String> =
        now.iter().filter(|(k, _)| !previous.contains_key(k)).map(|(k, c)| format!("  + {k} {c}")).collect();
    let removed: Vec<String> =
        previous.iter().filter(|(k, _)| !now.contains_key(k)).map(|(k, c)| format!("  - {k} {c}")).collect();
    let moved: Vec<String> = now
        .iter()
        .filter_map(|(k, c)| previous.get(k).filter(|p| p != c).map(|p| format!("  ~ {k} {p} -> {c}")))
        .collect();
    println!(
        "against the previous {}: {} rows before, {} added, {} removed, {} moved class",
        out_path.display(),
        previous.len(),
        added.len(),
        removed.len(),
        moved.len()
    );
    if !previous.is_empty() {
        for l in added.iter().chain(&removed).chain(&moved) {
            println!("{l}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gen-containers: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(t: &[&str]) -> Vec<String> {
        t.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_take_and_a_tag_make_a_container() {
        let pick = ["Gimmick_PickUp", "Gimmick_PickUp_Thief"];
        let l = [true, true];
        assert_eq!(rule(&pick, &l, &tags(&["catch_itemcontainer"])), Verdict::Emit(Class::ItemContainer));
        assert_eq!(rule(&pick, &l, &tags(&["furniture_drop"])), Verdict::Emit(Class::FurnitureDrop));
        // furniture_drop is decided first.
        assert_eq!(
            rule(&pick, &l, &tags(&["catch_itemcontainer", "furniture_drop"])),
            Verdict::Emit(Class::FurnitureDrop)
        );
        // No tag, or a take verb that is not lootable: not a container.
        assert_eq!(rule(&pick, &l, &tags(&["x"])), Verdict::None);
        assert_eq!(rule(&pick, &[false, false], &tags(&["catch_itemcontainer"])), Verdict::None);
        assert_eq!(rule(&["Gimmick_Catch"], &[true], &tags(&["catch_itemcontainer"])), Verdict::None);
    }

    #[test]
    fn the_earlier_classes_keep_a_record_out() {
        let t = tags(&["catch_itemcontainer"]);
        assert_eq!(
            rule(&["Gimmick_PickUp", PUZZLE], &[true, false], &t),
            Verdict::TakenEarlier("Treasure/puzzle box")
        );
        assert_eq!(
            rule(&["Gimmick_PushDown_Camera", "Gimmick_PickUp"], &[false, true], &t),
            Verdict::TakenEarlier("Puzzle-box part")
        );
        assert_eq!(rule(&["Gimmick_PickUp", "Chest_Open_360"], &[true, true], &t), Verdict::TakenEarlier("Chest"));
        // A Chest_Open that is not lootable does not make a chest.
        assert_eq!(rule(&["Gimmick_PickUp", "Chest_Open"], &[true, false], &t), Verdict::Emit(Class::ItemContainer));
        assert_eq!(
            rule(&["Gimmick_PickUp", FURNITURE], &[true, false], &t),
            Verdict::TakenEarlier("Furniture storage")
        );
        assert_eq!(
            rule(&["Gimmick_PickUp", PERSONAL], &[true, false], &t),
            Verdict::TakenEarlier("Personal storage")
        );
    }
}
