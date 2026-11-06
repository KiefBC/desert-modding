//! Generate `crates/desert-core/src/quest_items.rs`: the item keys the
//! item-log filter (`[Harvest] LootLogQuest`) always shows.
//!
//! Usage: `gen-quest-items [--tables DIR] [--out FILE] [--build ID]`
//!
//! Reads `itemgroupinfo` and `iteminfo` out of the installed game's archives
//! (`desert_tools::paz`, `paths::game_dir`) or a directory of extracted copies
//! (`--tables`, what `just extract-tables` writes into `analysis/tables/`).
//! The decoding is a port of `analysis/items_catalog.py`'s `itemgroups()`.
//!
//! ## The inventory group path
//!
//! An `itemgroupinfo` record is a node of the inventory's group tree: `u16
//! key`, six one-byte flags, `u16 orderIndex`, `u32 iconPath`, the localised
//! `groupName`, `u32 n` + `n` `u16` sub-group keys, `u32 n` + `n` `u32` item
//! keys, and `u32 n` + `n` bytes (`categoryTypeList`). Every record must
//! decode to its end, every sub-group must be a group and every item an
//! `iteminfo` key, or nothing is written.
//!
//! A group's parent is the group whose sub-group list names it; a group
//! listed by two parents takes the one **later in record order** (the Python
//! builds the map with a dict comprehension, and the last write wins). The
//! group's path is its parent chain, root first. An item sits in several
//! groups (6859 of 7064 on 26361232 and 26423880, 6864 of 7069 on 26480954); its path is the **longest** of its
//! groups' paths, the first in record order on a tie (`max(paths, key=len)`).
//!
//! The Python joins display names (`Equipment > Key Item > Book`), which come
//! out of the encrypted `.paloc` tables the archive reader cannot open. This
//! port matches the groups' editor names instead, which are the same nodes:
//!
//! | display path | editor path |
//! | --- | --- |
//! | `Equipment > Key Item ...` | `ItemGroup_Equip / ItemGroup_ETC_Quest / ...` |
//! | `Item > Keepsake > Quest Item` | `ItemGroup_Main / ItemGroup_ETC_Quest_Memory / ItemGroup_ETC_Quest_Normal` |
//! | `Item > Keepsake > Quest Reward` | `ItemGroup_Main / ItemGroup_ETC_Quest_Memory / ItemGroup_Collection_Quest_Reward` |
//! | `Item > Document > Quest Reward` | `ItemGroup_Main / ItemGroup_ETC_Document / ItemGroup_Collection_Quest_Reward_Document` |
//! | `Item > Key > Key` | `ItemGroup_Main / ItemGroup_ETC_Key / <any child>` (`_Key_Quest`, `_Key_Normal`, `_Key_Cube`) |
//!
//! On 26361232, 26423880 and 26480954 that is 1106 + 37 + 3 + 1 + 47 = 1194 items, the same set
//! `analysis/db/catalog.db`'s `items_items.inventory_group_path` gives.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;
use desert_tools::paths;
use desert_tools::paz::Archive;
use desert_tools::staticinfo::{self, Table};

const ITEM: &str = "iteminfo";
const ITEM_GROUP: &str = "itemgroupinfo";

/// One counted family of groups: its label in the header and the report, and
/// the editor path it matches.
pub struct Family {
    pub label: &'static str,
    pub display: &'static str,
    /// The path's head, root first.
    pub head: &'static [&'static str],
    /// `true`: the path must be exactly `head`, plus one more group when
    /// `any_child`. `false`: `head` is a prefix and anything below counts.
    pub exact: bool,
    /// With `exact`: one further group, any of them, below `head`.
    pub any_child: bool,
}

pub const FAMILIES: &[Family] = &[
    Family {
        label: "key_item",
        display: "Equipment > Key Item (and below)",
        head: &["ItemGroup_Equip", "ItemGroup_ETC_Quest"],
        exact: false,
        any_child: false,
    },
    Family {
        label: "quest_item",
        display: "Item > Keepsake > Quest Item",
        head: &["ItemGroup_Main", "ItemGroup_ETC_Quest_Memory", "ItemGroup_ETC_Quest_Normal"],
        exact: true,
        any_child: false,
    },
    Family {
        label: "keepsake_quest_reward",
        display: "Item > Keepsake > Quest Reward",
        head: &["ItemGroup_Main", "ItemGroup_ETC_Quest_Memory", "ItemGroup_Collection_Quest_Reward"],
        exact: true,
        any_child: false,
    },
    Family {
        label: "document_quest_reward",
        display: "Item > Document > Quest Reward",
        head: &["ItemGroup_Main", "ItemGroup_ETC_Document", "ItemGroup_Collection_Quest_Reward_Document"],
        exact: true,
        any_child: false,
    },
    Family {
        label: "key",
        display: "Item > Key > Key",
        head: &["ItemGroup_Main", "ItemGroup_ETC_Key"],
        exact: true,
        any_child: true,
    },
];

/// The checks made before writing, and emitted as a test: `(item key, editor
/// name, counted)`.
pub const SPOT_CHECKS: &[(u32, &str, bool)] = &[
    (30041, "Home_Key", true),
    (1000271, "Visione_Chip_Witch_Kidnapped_Memory", true),
    (1000061, "Book_Investigation_GoldleafGuildhouse", true),
    (801001, "Permit_Her_Pass", true),
    (70010, "Old_Wood_Doll_II", true),
    (1000151, "Quest_Crowman_Key_I", true),
    (1001309, "Quest_TrollCube_I", true),
    (1000859, "Paper_ReventineMonastery_Document", true),
    (1004388, "Item_Egg_KuKu", true),
    (1001245, "Item_dragonfly", false),
    (757006, "Peony", false),
    (1, "Money_Copper", false),
];

/// Which family an editor path belongs to, if any.
pub fn family_of(path: &[&str]) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| {
        let Some(rest) = path.strip_prefix(f.head) else { return false };
        match (f.exact, f.any_child) {
            (false, _) => true,
            (true, false) => rest.is_empty(),
            (true, true) => rest.len() == 1,
        }
    })
}

#[derive(Parser)]
#[command(about = "Generate crates/desert-core/src/quest_items.rs from the game's itemgroupinfo and iteminfo")]
struct Cli {
    /// A directory of extracted tables (`<table>.staticinfobody` and
    /// `.staticinfoheader`, as `just extract-tables` writes) to read instead of
    /// the game's archives [default: the archives under $CD_GAME_DIR, else
    /// the parent of $CD_BIN64]
    #[arg(long, value_name = "DIR")]
    tables: Option<PathBuf>,
    /// Where to write [default: <repo>/crates/desert-core/src/quest_items.rs]
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// The game build named in the header [default: the appmanifest's, else
    /// the calibrated build]
    #[arg(long, value_name = "ID")]
    build: Option<String>,
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

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o.checked_add(8)?)?.try_into().ok()?))
}

/// The record's editor name and where the body key starts: `u32 len, name`
/// (26361232's name-first header). Not held to ASCII: a few `iteminfo` names
/// are not (`1000186`); the key repeated after the name is the check.
fn name_of(rec: &[u8]) -> Option<(String, usize)> {
    let len = u32_at(rec, 0)? as usize;
    let name = rec.get(4..4usize.checked_add(len)?)?;
    Some((String::from_utf8_lossy(name).into_owned(), 4 + len))
}

/// One decoded `itemgroupinfo` record.
pub struct Group {
    pub key: u16,
    pub name: String,
    pub subs: Vec<u16>,
    pub items: Vec<u32>,
}

/// Decode one `itemgroupinfo` record to its end; see the module comment.
pub fn decode_group(rec: &[u8], header_key: u64) -> Result<Group, String> {
    let (name, mut o) = name_of(rec).ok_or("no readable name")?;
    let key = u16_at(rec, o).ok_or("no key")?;
    if u64::from(key) != header_key {
        return Err(format!("body key {key} is not the header's {header_key}"));
    }
    o += 2 + 6 + 2 + 4; // key, six flags, orderIndex, iconPath
    // groupName: u8 kind, u64 (record key << 32 | column), u32 len, digits.
    let lk = u64_at(rec, o + 1).ok_or("groupName cut short")?;
    if lk >> 32 != u64::from(key) {
        return Err(format!("groupName names record {} not {key}", lk >> 32));
    }
    let n = u32_at(rec, o + 9).ok_or("groupName cut short")? as usize;
    o += 13 + n;
    let n = u32_at(rec, o).ok_or("no sub-group count")? as usize;
    o += 4;
    let subs = (0..n).map(|i| u16_at(rec, o + 2 * i)).collect::<Option<Vec<_>>>().ok_or("sub-group list cut short")?;
    o += 2 * n;
    let n = u32_at(rec, o).ok_or("no item count")? as usize;
    o += 4;
    let items = (0..n).map(|i| u32_at(rec, o + 4 * i)).collect::<Option<Vec<_>>>().ok_or("item list cut short")?;
    o += 4 * n;
    let n = u32_at(rec, o).ok_or("no categoryTypeList count")? as usize;
    o += 4 + n;
    if o != rec.len() {
        return Err(format!("decodes to {o}, the record is {} bytes", rec.len()));
    }
    Ok(Group { key, name, subs, items })
}

/// Every record of a table as `(header key, bytes)`.
fn records_of(name: &str, t: &Table) -> Result<Vec<(u64, Vec<u8>)>> {
    let recs = t.split().with_context(|| format!("{name}: the header does not split the body"))?;
    recs.iter()
        .map(|r| {
            let bytes = t.body.get(r.offset..r.end).with_context(|| format!("{name}: record {} out of range", r.index))?;
            Ok((r.key, bytes.to_vec()))
        })
        .collect()
}

/// The previous file's keys, for the diff the report prints.
fn previous_keys(text: &str) -> BTreeSet<u32> {
    let Some(start) = text.find("pub const QUEST_ITEMS: &[u32] = &[") else { return BTreeSet::new() };
    let Some(body) = text.get(start..) else { return BTreeSet::new() };
    let Some(end) = body.find("];") else { return BTreeSet::new() };
    body.get(..end)
        .unwrap_or("")
        .lines()
        .skip(1)
        .flat_map(|l| l.split(','))
        .filter_map(|t| t.trim().parse().ok())
        .collect()
}

fn render(keys: &[u32], counts: &[(&Family, usize)], items_total: usize, build: &str) -> String {
    let mut out = String::new();
    let mut l = |s: &str| {
        out.push_str(s);
        out.push('\n');
    };
    l(&format!("//! Quest and key items of Crimson Desert build {build}: the items the"));
    l("//! item-log filter always shows under `[Harvest] LootLogQuest=1`.");
    l("//!");
    l("//! Generated by crates/tools/src/bin/gen-quest-items.rs (`just gen-quest-items`)");
    l("//! from the game's own archive tables, itemgroupinfo and iteminfo. Do not");
    l("//! edit by hand; rerun `just gen-quest-items`.");
    l("//!");
    l("//! The rule. An item's inventory group path is the longest parent chain of");
    l("//! the itemgroupinfo groups that list it (a group named by two parents takes");
    l("//! the later one in record order). The item counts when that path, by the");
    l("//! groups' editor names, is one of:");
    l("//!");
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    for (f, n) in counts {
        let shape = match (f.exact, f.any_child) {
            (false, _) => " / ...".to_string(),
            (true, false) => String::new(),
            (true, true) => " / <any child>".to_string(),
        };
        l(&format!("//! - {} ({n}): `{}{shape}`", f.display, f.head.join(" / ")));
    }
    l("//!");
    l(&format!("//! {total} of {items_total} iteminfo records on build {build}."));
    l("");
    l("/// Every counted item key, ascending, so [`contains`] can binary-search it.");
    l("pub const QUEST_ITEMS: &[u32] = &[");
    for chunk in keys.chunks(8) {
        let row: Vec<String> = chunk.iter().map(u32::to_string).collect();
        l(&format!("    {},", row.join(", ")));
    }
    l("];");
    l("");
    l("/// Whether the item `key` is a quest or key item.");
    l("pub fn contains(key: u32) -> bool {");
    l("    QUEST_ITEMS.binary_search(&key).is_ok()");
    l("}");
    l("");
    l("#[cfg(test)]");
    l("mod tests {");
    l("    use super::*;");
    l("");
    l("    #[test]");
    l("    fn sorted_and_unique() {");
    l("        assert!(QUEST_ITEMS.windows(2).all(|w| w[0] < w[1]));");
    l("    }");
    l("");
    l(&format!("    /// The count on build {build}."));
    l("    #[test]");
    l("    fn pinned_count() {");
    l(&format!("        assert_eq!(QUEST_ITEMS.len(), {total});"));
    l("    }");
    l("");
    l("    #[test]");
    l("    fn spot_keys() {");
    for (key, name, want) in SPOT_CHECKS {
        let not = if *want { "" } else { "!" };
        l(&format!("        assert!({not}contains({key}), \"{name}\");"));
    }
    l("    }");
    l("}");
    out
}

fn run(cli: &Cli) -> Result<()> {
    let root = paths::repo_root()?;
    let out_path =
        cli.out.clone().unwrap_or_else(|| paths::crate_dir_in(&root, "desert-core").join("src").join("quest_items.rs"));
    let build = cli.build.clone().or_else(paths::build_id).unwrap_or_else(|| paths::CALIBRATED_BUILD.to_string());
    let src = match &cli.tables {
        Some(d) => Source::Dir(d.clone()),
        None => Source::Archive(Archive::open(&paths::game_dir())?),
    };

    // iteminfo: key -> editor name, every record's body key equal to the header's.
    let item_recs = records_of(ITEM, &src.read(ITEM)?)?;
    let mut items: HashMap<u32, String> = HashMap::new();
    let mut item_order: Vec<u32> = Vec::with_capacity(item_recs.len());
    for (hk, rec) in &item_recs {
        let key = u32::try_from(*hk).with_context(|| format!("{ITEM}: key {hk} is wider than u32"))?;
        let Some((name, o)) = name_of(rec) else { bail!("{ITEM}: record {key} has no readable name") };
        if u32_at(rec, o) != Some(key) {
            bail!("{ITEM}: record {key} ({name}) does not repeat its key after the name");
        }
        if items.insert(key, name).is_some() {
            bail!("{ITEM}: key {key} occurs twice");
        }
        item_order.push(key);
    }

    // itemgroupinfo, all or nothing.
    let group_recs = records_of(ITEM_GROUP, &src.read(ITEM_GROUP)?)?;
    let mut groups: Vec<Group> = Vec::with_capacity(group_recs.len());
    let mut fails = Vec::new();
    for (hk, rec) in &group_recs {
        match decode_group(rec, *hk) {
            Ok(g) => groups.push(g),
            Err(e) => fails.push(format!("{hk}: {e}")),
        }
    }
    let index: HashMap<u16, usize> = groups.iter().enumerate().map(|(i, g)| (g.key, i)).collect();
    if index.len() != groups.len() {
        fails.push("two groups share a key".into());
    }
    for g in &groups {
        if let Some(s) = g.subs.iter().find(|s| !index.contains_key(s)) {
            fails.push(format!("{} {}: sub-group {s} is not a group", g.key, g.name));
        }
        if let Some(i) = g.items.iter().find(|i| !items.contains_key(i)) {
            fails.push(format!("{} {}: item {i} is not an iteminfo key", g.key, g.name));
        }
    }
    if !fails.is_empty() {
        bail!(
            "{ITEM_GROUP}: {} of {} records fail the layout check; nothing written. First ones:\n  {}",
            fails.len(),
            group_recs.len(),
            fails.iter().take(10).cloned().collect::<Vec<_>>().join("\n  ")
        );
    }

    // parent: the later lister wins, as in the Python.
    let mut parent: HashMap<usize, usize> = HashMap::new();
    for (gi, g) in groups.iter().enumerate() {
        for s in &g.subs {
            if let Some(&si) = index.get(s) {
                parent.insert(si, gi);
            }
        }
    }
    let path = |mut g: usize| -> Vec<usize> {
        let mut p = Vec::new();
        while !p.contains(&g) {
            p.push(g);
            match parent.get(&g) {
                Some(&up) => g = up,
                None => break,
            }
        }
        p.reverse();
        p
    };
    // member: item -> groups listing it, in record order.
    let mut member: HashMap<u32, Vec<usize>> = HashMap::new();
    for (gi, g) in groups.iter().enumerate() {
        for &i in &g.items {
            member.entry(i).or_default().push(gi);
        }
    }

    let mut keys: Vec<u32> = Vec::new();
    let mut per_family: BTreeMap<&str, usize> = BTreeMap::new();
    let mut counted: HashSet<u32> = HashSet::new();
    for &key in &item_order {
        let Some(gs) = member.get(&key) else { continue };
        // First longest, as Python's max(key=len).
        let mut best: Vec<usize> = Vec::new();
        for &g in gs {
            let p = path(g);
            if p.len() > best.len() {
                best = p;
            }
        }
        let names: Vec<&str> = best.iter().filter_map(|&g| groups.get(g)).map(|g| g.name.as_str()).collect();
        if let Some(f) = family_of(&names) {
            keys.push(key);
            counted.insert(key);
            *per_family.entry(f.label).or_insert(0) += 1;
        }
    }
    keys.sort_unstable();
    keys.dedup();

    let wrong: Vec<String> = SPOT_CHECKS
        .iter()
        .filter_map(|&(key, name, want)| {
            let have = items.get(&key);
            if have.map(String::as_str) != Some(name) {
                return Some(format!("{key}: named {have:?} in {ITEM}, expected {name}"));
            }
            (counted.contains(&key) != want).then(|| format!("{key} {name}: counted {}, expected {want}", !want))
        })
        .collect();
    if !wrong.is_empty() {
        bail!("spot checks fail; nothing written:\n  {}", wrong.join("\n  "));
    }

    let counts: Vec<(&Family, usize)> =
        FAMILIES.iter().map(|f| (f, per_family.get(f.label).copied().unwrap_or(0))).collect();
    let previous = fs::read_to_string(&out_path).map(|t| previous_keys(&t)).unwrap_or_default();
    let text = render(&keys, &counts, items.len(), &build);
    fs::write(&out_path, text).with_context(|| format!("cannot write {}", out_path.display()))?;

    println!("source: {}", src.describe());
    println!("build: {build}");
    println!(
        "records: {} {ITEM}, {} {ITEM_GROUP}; every group decodes to its end and every reference resolves",
        items.len(),
        groups.len()
    );
    println!("items in more than one group: {}", member.values().filter(|g| g.len() > 1).count());
    for (f, n) in &counts {
        println!("  {:<40} {n}", f.display);
    }
    println!("quest items: {} written to {}", keys.len(), out_path.display());
    let now: BTreeSet<u32> = keys.iter().copied().collect();
    let added: Vec<&u32> = now.difference(&previous).collect();
    let removed: Vec<&u32> = previous.difference(&now).collect();
    println!(
        "against the previous file: {} keys before, {} added, {} removed",
        previous.len(),
        added.len(),
        removed.len()
    );
    if !previous.is_empty() && added.len() + removed.len() <= 200 {
        for k in added {
            println!("  + {k} {}", items.get(k).map_or("?", String::as_str));
        }
        for k in removed {
            println!("  - {k}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("gen-quest-items: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_family_rule() {
        let f = |p: &[&str]| family_of(p).map(|f| f.label);
        assert_eq!(f(&["ItemGroup_Equip", "ItemGroup_ETC_Quest"]), Some("key_item"));
        assert_eq!(f(&["ItemGroup_Equip", "ItemGroup_ETC_Quest", "ItemGroup_ETC_Book_Quest"]), Some("key_item"));
        assert_eq!(f(&["ItemGroup_Equip", "ItemGroup_ETC_Quest_Equip"]), None);
        assert_eq!(f(&["ItemGroup_Main", "ItemGroup_ETC_Key"]), None);
        assert_eq!(f(&["ItemGroup_Main", "ItemGroup_ETC_Key", "ItemGroup_ETC_Key_Cube"]), Some("key"));
        assert_eq!(f(&["ItemGroup_Main", "ItemGroup_ETC_Key", "a", "b"]), None);
        assert_eq!(
            f(&["ItemGroup_Main", "ItemGroup_ETC_Quest_Memory", "ItemGroup_ETC_Quest_Normal"]),
            Some("quest_item")
        );
        assert_eq!(f(&["ItemGroup_Main", "ItemGroup_ETC_Quest_Memory", "ItemGroup_ETC_Customize_Kliff"]), None);
        assert_eq!(f(&["ItemGroup_Main", "ItemGroup_ETC_Quest_Memory"]), None);
    }

    #[test]
    fn a_group_record_decodes_to_its_end() {
        let mut r = Vec::new();
        r.extend_from_slice(&4u32.to_le_bytes());
        r.extend_from_slice(b"Grp1");
        r.extend_from_slice(&17077u16.to_le_bytes());
        r.extend_from_slice(&[0; 6]);
        r.extend_from_slice(&3u16.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes());
        r.push(0);
        r.extend_from_slice(&((17077u64 << 32) | 0x80).to_le_bytes());
        r.extend_from_slice(&2u32.to_le_bytes());
        r.extend_from_slice(b"77");
        r.extend_from_slice(&1u32.to_le_bytes());
        r.extend_from_slice(&16970u16.to_le_bytes());
        r.extend_from_slice(&2u32.to_le_bytes());
        r.extend_from_slice(&30041u32.to_le_bytes());
        r.extend_from_slice(&1u32.to_le_bytes());
        r.extend_from_slice(&1u32.to_le_bytes());
        r.push(7);
        let g = decode_group(&r, 17077).unwrap();
        assert_eq!((g.key, g.name.as_str(), g.subs, g.items), (17077, "Grp1", vec![16970], vec![30041, 1]));
        assert!(decode_group(&r, 17078).is_err());
        r.push(0);
        assert!(decode_group(&r, 17077).is_err(), "a trailing byte is refused");
    }

    #[test]
    fn previous_keys_round_trips_render() {
        let fams: Vec<(&Family, usize)> = FAMILIES.iter().map(|f| (f, 1)).collect();
        let keys = [1u32, 5, 9, 30041, 1000151, 1000859, 1001309, 1004388, 2000000];
        let text = render(&keys, &fams, 9, "1");
        assert_eq!(previous_keys(&text), keys.iter().copied().collect());
    }
}
