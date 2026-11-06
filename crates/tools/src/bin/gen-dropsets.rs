//! Generate `crates/desert-core/src/dropset_table.rs`: the `dropsetinfo` rows
//! the `[Harvest]` `Pouches` and `Breakables` levers multiply, by dropset
//! key.
//!
//! Usage: `gen-dropsets [--tables DIR] [--out FILE] [--build ID]`
//!
//! Reads the game's own archive tables (`desert_tools::paz`,
//! `paths::game_dir`) or a directory of extracted copies (`--tables`, what
//! `just extract-tables` writes into `analysis/tables/`), and every record of
//! every table it reads is walked to its last byte (`gimmickinfo` through
//! `desert_tools::gimmick_walk::full`, the rest through `walks`) or nothing is
//! written:
//!
//! - **Pouch** rows: the dropset of an `itemuseinfo` RandomBox record (kind
//!   2, `ItemUseData_RandomBox`) that some `iteminfo` record's
//!   `_itemUseInfoList` names: what an item pays when it is opened (FINDINGS
//!   2.6). The coin pouches are five of them; the rest are seed packs, chest
//!   items, recipe boxes and the like, all of them "a box you open".
//! - **Breakable** rows: the `dropSetInfoList` dropsets of the `gimmickinfo`
//!   records `analysis/container_catalog.py` calls "Breakable container": a
//!   non-`BreakNone` `breakableObjectInfo`, a loot list, no lootable
//!   interaction, not collect-tagged, not a gather node and no earlier class
//!   (puzzle box and part, chest, furniture and personal storage, furniture
//!   drop, item container). Barrels, crates, pots, sacks, smashed or thrown.
//!
//! **A row any other source also pays from is refused**, and goes in neither
//! group: another gimmick class (a chest, a ground item, ...), an
//! interaction's `rewardDropSetInfo`, an interaction override, a gimmick's
//! `productData`, a crafting recipe (`multichangeinfo`), a quest or mission
//! (`missioninfo`), a dispatch operation, faction research or a religion
//! block cost (`factionnode`), or the other group. Multiplying a shared row
//! would multiply that source too. The sources not walked here (the
//! character tables' loot and pockets, quest dialogue rewards, stage and
//! sub-level rewards, mini-games) are named in the generated header; on
//! 26361232 and 26423880 the catalogue database (`analysis/catalog_db.py`, which does walk
//! the characters and quests) finds none of the emitted rows shared with them.
//!
//! Each emitted row carries what the plugin checks the parsed row against
//! before it writes: its archive index (the manager's record index), its
//! declared entry count and the sum of `min + max` over its item entries.

#[path = "gen-dropsets/render.rs"]
mod render;
#[path = "gen-dropsets/walks.rs"]
mod walks;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;
use desert_tools::gimmick_walk::{self as gw, DROP_KIND_ITEM};
use desert_tools::paz::Archive;
use desert_tools::staticinfo::{self, Table};
use desert_tools::{collect_rows, paths};

const DROPSET: &str = "dropsetinfo";
const GIMMICK: &str = "gimmickinfo";
const INTERACTION: &str = "interactioninfo";
const BREAKABLE: &str = "breakableobjectinfo";
const ITEM_USE: &str = "itemuseinfo";
const ITEM: &str = "iteminfo";
const MULTICHANGE: &str = "multichangeinfo";
const MISSION: &str = "missioninfo";
const FACTION_NODE: &str = "factionnode";

/// `container_catalog.py`'s take verbs, matched as prefixes.
const TAKE: [&str; 6] =
    ["Gimmick_PickUp", "ItemHot_Collect", "ItemCold_Collect", "Item_Collect_Reward", "Ship_PickUp", "ReadMemory_Paper_Collect"];
const CHEST: &str = "Chest_Open";
const FURNITURE: &str = "Furniture_Open";
const PERSONAL: &str = "Furniture_Examine";
const PUZZLE: &str = "Gimmick_Examine_PuzzleBox";
const PUZZLE_PART: [&str; 2] = ["Gimmick_PushDown_Camera", "Gimmick_Move_Camera"];
const TAG_FURNITURE_DROP: &str = "furniture_drop";
const TAG_ITEM_CONTAINER: &str = "catch_itemcontainer";
/// Row-name prefixes that make a loot list a destructible drop rather than a
/// container (`container_catalog.py` `destructible_rows`).
const DESTRUCTIBLE_ROWS: [&str; 3] = ["DropSet_Collect_", "DropSet_TreeWeapon", "Item("];
/// `breakableObjectInfo` names that do not make a record breakable.
const NOT_BREAKABLE: [&str; 2] = ["BreakNone", "Unbreakable"];

/// The `container_catalog.py` class that pays its `dropSetInfoList` to the
/// `Breakables` lever.
const BREAKABLE_CLASS: &str = "Breakable container";
/// The source label of a RandomBox use.
const POUCH_SOURCE: &str = "pouch";

/// The checks made before writing, and emitted as tests: `(dropset key, row
/// name, group)`, `None` for a row that must be absent. The pouches are
/// FINDINGS 2.6's five; the refused ones are shared on 26361232.
pub const SPOT_CHECKS: &[(u32, &str, Option<Group>)] = &[
    (400005, "DropSet_Money_Copper_100_500", Some(Group::Pouch)),
    (400006, "DropSet_Money_Copper_10000", Some(Group::Pouch)),
    (400007, "DropSet_Money_Copper_20000", Some(Group::Pouch)),
    (1016612, "DropSet_Money_Copper_100000", Some(Group::Pouch)),
    (100137, "DropSet_Pinball_Bag", Some(Group::Pouch)),
    (100000, "DropSet_BoxBarrel_01", Some(Group::Breakable)),
    (110000, "DropSet_BoxBarrel_02", Some(Group::Breakable)),
    (100001, "DropSet_Pot_01", Some(Group::Breakable)),
    // Shared with chests.
    (110001, "DropSet_BoxBarrel_03", None),
    // Shared with crafting.
    (160100, "DropSet_Arrow_Package", None),
];

#[derive(Parser)]
#[command(about = "Generate crates/desert-core/src/dropset_table.rs from the game's static-info tables")]
struct Cli {
    /// A directory of extracted tables (`<table>.staticinfobody` and
    /// `.staticinfoheader`, as `just extract-tables` writes) to read instead of
    /// the game's archives [default: the archives under $CD_GAME_DIR, else
    /// the parent of $CD_BIN64]
    #[arg(long, value_name = "DIR")]
    tables: Option<PathBuf>,
    /// Where to write [default: <repo>/crates/desert-core/src/dropset_table.rs]
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// The game build named in the header [default: the appmanifest's, else
    /// the calibrated build]
    #[arg(long, value_name = "ID")]
    build: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Pouch,
    Breakable,
}

impl Group {
    pub fn variant(self) -> &'static str {
        match self {
            Group::Pouch => "Pouch",
            Group::Breakable => "Breakable",
        }
    }

    /// The source label this group's own referrers carry.
    fn source(self) -> &'static str {
        match self {
            Group::Pouch => POUCH_SOURCE,
            Group::Breakable => "gimmick: Breakable container",
        }
    }
}

/// One emitted row.
pub struct Row {
    pub key: u32,
    pub index: u16,
    pub name: String,
    pub group: Group,
    /// The declared entry count (the parsed row's `+0x28`).
    pub entries: usize,
    /// Item (kind 0) entries.
    pub items: usize,
    /// `sum(min + max)` over the item entries.
    pub amounts: i64,
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

/// One record: its header index and key, and its bytes.
struct Rec {
    index: usize,
    key: u32,
    bytes: Vec<u8>,
}

fn records_of(src: &Source, name: &str) -> Result<Vec<Rec>> {
    let t = src.read(name)?;
    let recs = t.split().with_context(|| format!("{name}: the header does not split the body"))?;
    recs.iter()
        .map(|r| {
            let key = u32::try_from(r.key).with_context(|| format!("{name}: key {} is wider than u32", r.key))?;
            let bytes = t.body.get(r.offset..r.end).with_context(|| format!("{name}: record {} out of range", r.index))?;
            Ok(Rec { index: r.index, key, bytes: bytes.to_vec() })
        })
        .collect()
}

/// A record's name: `u32 len`, bytes.
fn record_name(rec: &[u8]) -> String {
    let n = rec.get(0..4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).unwrap_or(0) as usize;
    rec.get(4..4 + n).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default()
}

/// Walk every record of a table with `f`, all or nothing.
fn walk_all<T>(name: &str, recs: &[Rec], mut f: impl FnMut(&Rec) -> Result<T, gw::Stop>) -> Result<Vec<T>> {
    let mut out = Vec::with_capacity(recs.len());
    let mut fails = Vec::new();
    for r in recs {
        match f(r) {
            Ok(v) => out.push(v),
            Err(e) => fails.push(format!("{} {}: {e}", r.key, record_name(&r.bytes))),
        }
    }
    if !fails.is_empty() {
        let shown: Vec<&str> = fails.iter().take(10).map(String::as_str).collect();
        bail!(
            "{name}: {} of {} records fail the walk; nothing written. First ones:\n  {}",
            fails.len(),
            recs.len(),
            shown.join("\n  ")
        );
    }
    Ok(out)
}

/// The one `ItemInfo` layout every `iteminfo` record walks under. None or
/// both is a refusal: the walk cannot tell which build the table came from.
fn item_layout(recs: &[Rec]) -> Result<walks::ItemLayout> {
    let fits: Vec<walks::ItemLayout> = walks::ItemLayout::ALL
        .into_iter()
        .filter(|l| recs.iter().all(|r| walks::item_use_list(&r.bytes, r.key, *l).is_ok()))
        .collect();
    match fits.as_slice() {
        [one] => Ok(*one),
        [] => {
            // Report the failures of the newest layout, the likelier one.
            walk_all(ITEM, recs, |r| walks::item_use_list(&r.bytes, r.key, walks::ItemLayout::ALL[0]))?;
            bail!("{ITEM}: no ItemInfo layout walks every record")
        }
        _ => bail!("{ITEM}: every record walks under more than one ItemInfo layout ({fits:?}); nothing written"),
    }
}

/// The facts the class rule reads off one gimmick record.
pub struct GimmickFacts<'a> {
    pub name: &'a str,
    pub inames: Vec<&'a str>,
    pub lootable: Vec<bool>,
    pub tags: &'a [String],
    pub gather: bool,
    pub loot: bool,
    pub destructible_rows: bool,
    pub breakable: &'a str,
}

/// `container_catalog.py`'s `rule()`, first rule met. Returns the class name.
pub fn class_of(g: &GimmickFacts) -> &'static str {
    let loot: Vec<&str> = g.inames.iter().zip(&g.lootable).filter(|(_, l)| **l).map(|(n, _)| *n).collect();
    let first = g.inames.first().copied().unwrap_or("");
    let take = loot.iter().any(|n| TAKE.iter().any(|t| n.starts_with(t)));
    let has = |t: &str| g.tags.iter().any(|x| x == t);
    let collect_tag = g.tags.iter().any(|t| t.starts_with("collect"));
    let brk = !g.breakable.is_empty() && !NOT_BREAKABLE.contains(&g.breakable);
    let any_inter = !g.inames.is_empty();
    if g.inames.contains(&PUZZLE) {
        return "Treasure/puzzle box";
    }
    if PUZZLE_PART.contains(&first) {
        return "Puzzle-box part";
    }
    if loot.iter().any(|n| n.contains(CHEST)) {
        return "Chest";
    }
    if g.inames.contains(&FURNITURE) {
        return "Furniture storage";
    }
    if g.inames.contains(&PERSONAL) {
        return "Personal storage";
    }
    if take && has(TAG_FURNITURE_DROP) {
        return "Furniture drop";
    }
    if take && has(TAG_ITEM_CONTAINER) {
        return "Item container";
    }
    if g.gather {
        return "Gather node";
    }
    if g.loot && loot.is_empty() {
        if collect_tag || g.destructible_rows {
            return "Destructible drop";
        }
        if brk {
            return BREAKABLE_CLASS;
        }
    }
    if brk && !g.loot && loot.is_empty() && !any_inter {
        return "Breakable, no loot";
    }
    if take {
        return "Ground item";
    }
    if !any_inter && g.name.to_lowercase().starts_with("item_") {
        return "Ground item";
    }
    if first.starts_with("Gimmick_Catch") {
        return "Carry prop";
    }
    if any_inter {
        return "Other interactable";
    }
    if brk {
        return "Breakable, no loot";
    }
    "Scenery"
}

/// Every referrer of every dropset key: `key -> source label -> count`.
#[derive(Default)]
struct Refs {
    by_key: HashMap<u32, BTreeMap<String, usize>>,
    /// References to a key `dropsetinfo` does not have, per source.
    dangling: BTreeMap<String, usize>,
}

impl Refs {
    fn add(&mut self, known: &HashMap<u32, usize>, key: u32, source: &str) {
        if key == 0 || key == u32::MAX {
            return;
        }
        if !known.contains_key(&key) {
            *self.dangling.entry(source.to_string()).or_insert(0) += 1;
            return;
        }
        *self.by_key.entry(key).or_default().entry(source.to_string()).or_insert(0) += 1;
    }

    fn sources(&self, key: u32) -> Vec<(&str, usize)> {
        self.by_key.get(&key).map(|m| m.iter().map(|(s, n)| (s.as_str(), *n)).collect()).unwrap_or_default()
    }
}

fn run(cli: &Cli) -> Result<()> {
    let root = paths::repo_root()?;
    let core_src = paths::crate_dir_in(&root, "desert-core").join("src");
    let out_path = cli.out.clone().unwrap_or_else(|| core_src.join("dropset_table.rs"));
    let build = cli.build.clone().or_else(paths::build_id).unwrap_or_else(|| paths::CALIBRATED_BUILD.to_string());
    let src = match &cli.tables {
        Some(d) => Source::Dir(d.clone()),
        None => Source::Archive(Archive::open(&paths::game_dir())?),
    };

    // dropsetinfo: key -> header index, and every row walked to its end.
    let ds_recs = records_of(&src, DROPSET)?;
    let ds_rows = walk_all(DROPSET, &ds_recs, |r| walks::dropset(&r.bytes, r.key))?;
    let index_of: HashMap<u32, usize> = ds_recs.iter().map(|r| (r.key, r.index)).collect();
    if index_of.len() != ds_recs.len() {
        bail!("{DROPSET}: a key occurs twice; nothing written");
    }
    let ds_name: HashMap<u32, String> = ds_recs.iter().map(|r| (r.key, record_name(&r.bytes))).collect();
    let ds_by_key: HashMap<u32, &walks::Dropset> = ds_recs.iter().map(|r| r.key).zip(ds_rows.iter()).collect();

    // breakableobjectinfo: u16 key -> name.
    let brk_recs = records_of(&src, BREAKABLE)?;
    let brk_names = walk_all(BREAKABLE, &brk_recs, |r| {
        let n = u32::from_le_bytes(r.bytes.get(0..4).ok_or("no name")?.try_into().map_err(|_| "name")?) as usize;
        let k = r.bytes.get(4 + n..6 + n).ok_or("no key")?;
        if u32::from(u16::from_le_bytes([k[0], k[1]])) != r.key {
            return Err("the body key is not the header's".to_string());
        }
        if r.bytes.len() != 4 + n + 2 + 6 {
            return Err(format!("{} bytes, not name + u16 + six u8", r.bytes.len()));
        }
        Ok((r.key, record_name(&r.bytes)))
    })?;
    let brk_name: HashMap<u32, String> = brk_names.into_iter().collect();

    // interactioninfo heads.
    let inter_recs = records_of(&src, INTERACTION)?;
    let inters = walk_all(INTERACTION, &inter_recs, |r| gw::interaction_head(&r.bytes, r.key))?;
    let inter: HashMap<u32, gw::Interaction> = inter_recs.iter().map(|r| r.key).zip(inters).collect();

    let mut refs = Refs::default();
    for i in inter.values() {
        refs.add(&index_of, i.reward_dropset, "interaction rewardDropSetInfo");
    }

    // gimmickinfo, walked to the end and classed.
    let gather: BTreeSet<u32> = collect_rows::read(&core_src.join("collect.rs"))?.iter().map(|r| r.key).collect();
    let gim_recs = records_of(&src, GIMMICK)?;
    let gims = walk_all(GIMMICK, &gim_recs, |r| gw::full(&r.bytes, r.key))?;
    let mut classes: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut breakable_gimmicks = 0usize;
    let mut fails = Vec::new();
    for (r, g) in gim_recs.iter().zip(&gims) {
        let name = record_name(&r.bytes);
        let mut inames = Vec::with_capacity(g.head.interactions.len());
        let mut lootable = Vec::with_capacity(g.head.interactions.len());
        for k in &g.head.interactions {
            match inter.get(k) {
                Some(i) => {
                    inames.push(i.name.as_str());
                    lootable.push(i.lootable);
                }
                None => fails.push(format!("{} {name}: override interaction {k} is not an {INTERACTION} record", r.key)),
            }
        }
        let breakable = match brk_name.get(&u32::from(g.breakable)) {
            Some(n) => n.as_str(),
            None => {
                fails.push(format!("{} {name}: breakableObjectInfo {} is not a {BREAKABLE} record", r.key, g.breakable));
                ""
            }
        };
        let row_names: Vec<&str> = g.dropsets.iter().filter_map(|d| ds_name.get(d)).map(String::as_str).collect();
        let facts = GimmickFacts {
            name: &name,
            inames,
            lootable,
            tags: &g.head.tags,
            gather: gather.contains(&r.key),
            loot: !g.dropsets.is_empty() || g.drop_items > 0,
            destructible_rows: !row_names.is_empty()
                && row_names.iter().all(|n| DESTRUCTIBLE_ROWS.iter().any(|p| n.starts_with(p))),
            breakable,
        };
        let class = class_of(&facts);
        *classes.entry(class).or_insert(0) += 1;
        if class == BREAKABLE_CLASS && !g.dropsets.is_empty() {
            breakable_gimmicks += 1;
        }
        let label = format!("gimmick: {class}");
        for d in &g.dropsets {
            refs.add(&index_of, *d, &label);
        }
        for d in &g.head.override_dropsets {
            refs.add(&index_of, *d, "gimmick interaction override");
        }
        if let Some(d) = g.product_dropset {
            refs.add(&index_of, d, "gimmick productData");
        }
    }
    if !fails.is_empty() {
        bail!("{GIMMICK}: {} references do not resolve; nothing written:\n  {}", fails.len(), fails.join("\n  "));
    }

    // itemuseinfo RandomBox records, and the items that use them.
    let use_recs = records_of(&src, ITEM_USE)?;
    let uses = walk_all(ITEM_USE, &use_recs, |r| walks::item_use(&r.bytes, r.key))?;
    let random_box: HashMap<u32, u32> = use_recs
        .iter()
        .zip(&uses)
        .filter_map(|(r, (kind, d))| (*kind == walks::RANDOM_BOX).then_some((r.key, d.unwrap_or(0))))
        .collect();
    let item_recs = records_of(&src, ITEM)?;
    let layout = item_layout(&item_recs)?;
    let use_lists = walk_all(ITEM, &item_recs, |r| walks::item_use_list(&r.bytes, r.key, layout))?;
    let used: BTreeSet<u32> = use_lists.iter().flatten().copied().collect();
    let mut pouch_items = 0usize;
    for l in &use_lists {
        pouch_items += usize::from(l.iter().any(|u| random_box.get(u).is_some_and(|d| *d != 0)));
    }
    let mut pouch_candidates: BTreeSet<u32> = BTreeSet::new();
    for (u, d) in &random_box {
        refs.add(&index_of, *d, POUCH_SOURCE);
        if used.contains(u) && index_of.contains_key(d) {
            pouch_candidates.insert(*d);
        }
    }

    // The other referrers.
    let mc_recs = records_of(&src, MULTICHANGE)?;
    for l in walk_all(MULTICHANGE, &mc_recs, |r| walks::multichange(&r.bytes, r.key))? {
        for d in l {
            refs.add(&index_of, d, "crafting (multichangeinfo)");
        }
    }
    let mi_recs = records_of(&src, MISSION)?;
    for l in walk_all(MISSION, &mi_recs, |r| walks::mission(&r.bytes, r.key))? {
        for d in l {
            refs.add(&index_of, d, "quest/mission (missioninfo)");
        }
    }
    let fn_recs = records_of(&src, FACTION_NODE)?;
    for f in walk_all(FACTION_NODE, &fn_recs, |r| walks::factionnode(&r.bytes, r.key))? {
        for d in f.dispatch {
            refs.add(&index_of, d, "dispatch operation (factionnode)");
        }
        for d in f.research {
            refs.add(&index_of, d, "faction research (factionnode)");
        }
        for d in f.block_cost {
            refs.add(&index_of, d, "religion block cost (factionnode)");
        }
    }

    // The two groups, and what is refused.
    let breakable_candidates: BTreeSet<u32> = refs
        .by_key
        .iter()
        .filter(|(_, m)| m.contains_key(Group::Breakable.source()))
        .map(|(k, _)| *k)
        .collect();
    let mut rows: Vec<Row> = Vec::new();
    let mut refused: Vec<(u32, Group, String)> = Vec::new();
    for (group, cands) in [(Group::Pouch, &pouch_candidates), (Group::Breakable, &breakable_candidates)] {
        for &key in cands {
            let others: Vec<String> = refs
                .sources(key)
                .into_iter()
                .filter(|(s, _)| *s != group.source())
                .map(|(s, n)| format!("{s} x{n}"))
                .collect();
            if !others.is_empty() {
                refused.push((key, group, others.join(", ")));
                continue;
            }
            let ds = ds_by_key.get(&key).with_context(|| format!("dropset {key} vanished"))?;
            let index = index_of.get(&key).copied().unwrap_or(usize::MAX);
            let index = u16::try_from(index).with_context(|| format!("dropset {key}: index {index} wider than u16"))?;
            let items: Vec<&gw::DropEntry> = ds.entries.iter().filter(|e| e.kind == DROP_KIND_ITEM).collect();
            if ds.entries.len() != ds.declared {
                refused.push((key, group, "an absent entry in the row (the parsed list would hold a gap)".into()));
                continue;
            }
            let weights: i64 = ds.entries.iter().map(|e| e.weight).sum();
            if weights != ds.total {
                refused.push((key, group, format!("entry weights sum to {weights}, totalDropRate is {}", ds.total)));
                continue;
            }
            if items.is_empty() {
                refused.push((key, group, "no item entry: nothing to multiply".into()));
                continue;
            }
            rows.push(Row {
                key,
                index,
                name: ds_name.get(&key).cloned().unwrap_or_default(),
                group,
                entries: ds.declared,
                items: items.len(),
                amounts: items.iter().map(|e| e.min + e.max).sum(),
            });
        }
    }
    rows.sort_by_key(|r| r.key);
    refused.sort();
    if let Some(w) = rows.windows(2).find(|w| w[0].key == w[1].key) {
        bail!("dropset {} landed in two groups; nothing written", w[0].key);
    }

    let group_of = |k: u32| rows.iter().find(|r| r.key == k).map(|r| r.group);
    let wrong: Vec<String> = SPOT_CHECKS
        .iter()
        .filter(|(k, n, want)| group_of(*k) != *want || ds_name.get(k).map(String::as_str) != Some(*n))
        .map(|(k, n, want)| format!("{k} {n}: {:?} named {:?}, expected {want:?}", group_of(*k), ds_name.get(k)))
        .collect();
    if !wrong.is_empty() {
        bail!("spot checks fail; nothing written:\n  {}", wrong.join("\n  "));
    }

    let count = |g: Group| rows.iter().filter(|r| r.group == g).count();
    let counts = render::Counts {
        pouch: count(Group::Pouch),
        breakable: count(Group::Breakable),
        refused_pouch: refused.iter().filter(|r| r.1 == Group::Pouch).count(),
        refused_breakable: refused.iter().filter(|r| r.1 == Group::Breakable).count(),
        pouch_candidates: pouch_candidates.len(),
        breakable_candidates: breakable_candidates.len(),
        pouch_items,
        breakable_gimmicks,
    };
    let previous = fs::read_to_string(&out_path).map(|t| render::previous_rows(&t)).unwrap_or_default();
    let text = render::dropset_table_rs(&rows, &refused, &ds_name, &counts, &build);
    fs::write(&out_path, text).with_context(|| format!("cannot write {}", out_path.display()))?;

    // The report.
    println!("source: {}", src.describe());
    println!("build: {build}");
    println!(
        "records walked to the end: {} {DROPSET}, {} {GIMMICK}, {} {INTERACTION} heads, {} {BREAKABLE}, \
         {} {ITEM_USE} ({} RandomBox), {} {ITEM} (to _itemUseInfoList), {} {MULTICHANGE}, {} {MISSION}, \
         {} {FACTION_NODE}",
        ds_recs.len(),
        gim_recs.len(),
        inter_recs.len(),
        brk_recs.len(),
        use_recs.len(),
        random_box.len(),
        item_recs.len(),
        mc_recs.len(),
        mi_recs.len(),
        fn_recs.len()
    );
    println!("{ITEM} layout: {layout:?} (the only one every record walks under)");
    let cls: Vec<String> = classes.iter().map(|(c, n)| format!("{c} {n}")).collect();
    println!("gimmick classes: {}", cls.join(", "));
    println!(
        "candidates: {} pouch rows ({} items open one), {} breakable rows ({} breakable containers with a dropSetInfoList)",
        counts.pouch_candidates, counts.pouch_items, counts.breakable_candidates, counts.breakable_gimmicks
    );
    println!(
        "rows: {} = Pouch {} ({} item entries), Breakable {} ({} item entries)",
        rows.len(),
        counts.pouch,
        rows.iter().filter(|r| r.group == Group::Pouch).map(|r| r.items).sum::<usize>(),
        counts.breakable,
        rows.iter().filter(|r| r.group == Group::Breakable).map(|r| r.items).sum::<usize>()
    );
    println!("refused: {}", refused.len());
    for (k, g, why) in &refused {
        println!("  refused {k} {} as {}: {why}", ds_name.get(k).map(String::as_str).unwrap_or("?"), g.variant());
    }
    for (s, n) in &refs.dangling {
        println!("note: {n} {s} references name no {DROPSET} key");
    }
    let now: BTreeMap<u32, &str> = rows.iter().map(|r| (r.key, r.group.variant())).collect();
    let added: Vec<String> =
        now.iter().filter(|(k, _)| !previous.contains_key(k)).map(|(k, c)| format!("  + {k} {c}")).collect();
    let removed: Vec<String> =
        previous.iter().filter(|(k, _)| !now.contains_key(k)).map(|(k, c)| format!("  - {k} {c}")).collect();
    let moved: Vec<String> = now
        .iter()
        .filter_map(|(k, c)| previous.get(k).filter(|p| p != c).map(|p| format!("  ~ {k} {p} -> {c}")))
        .collect();
    println!(
        "against the previous {}: {} rows before, {} added, {} removed, {} moved group",
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
            eprintln!("gen-dropsets: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(inames: Vec<&'a str>, lootable: Vec<bool>, tags: &'a [String], brk: &'a str) -> GimmickFacts<'a> {
        GimmickFacts {
            name: "barrel",
            inames,
            lootable,
            tags,
            gather: false,
            loot: true,
            destructible_rows: false,
            breakable: brk,
        }
    }

    #[test]
    fn a_breakable_with_loot_and_no_loot_verb_is_a_breakable_container() {
        let none: Vec<String> = Vec::new();
        assert_eq!(class_of(&facts(vec![], vec![], &none, "BreakAttack")), BREAKABLE_CLASS);
        // A carry interaction is not a loot verb.
        assert_eq!(class_of(&facts(vec!["Gimmick_Catch"], vec![false], &none, "BreakAttack")), BREAKABLE_CLASS);
        // BreakNone is not breakable.
        assert_eq!(class_of(&facts(vec![], vec![], &none, "BreakNone")), "Scenery");
        // A lootable chest verb takes it first.
        assert_eq!(class_of(&facts(vec!["Chest_Open"], vec![true], &none, "BreakAttack")), "Chest");
        // A collect tag makes it a destructible drop.
        let tags = vec!["collect_tree".to_string()];
        assert_eq!(class_of(&facts(vec![], vec![], &tags, "BreakAttack")), "Destructible drop");
        // A gather node is not a container.
        let mut g = facts(vec![], vec![], &none, "BreakAttack");
        g.gather = true;
        assert_eq!(class_of(&g), "Gather node");
        // No loot: not a container.
        let mut g = facts(vec![], vec![], &none, "BreakAttack");
        g.loot = false;
        assert_eq!(class_of(&g), "Breakable, no loot");
    }
}
