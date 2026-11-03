//! Generate `crates/desert-core/src/species_table.rs`: every creature species
//! by how it is caught.
//!
//! Usage: `gen-species [--tables DIR] [--out FILE] [--build ID]`
//!
//! Reads `characterinfo`, `interactioninfo`, `tribeinfo` and
//! `charactergroupinfo`, plus the key sets of `knowledgeinfo`, `iteminfo`,
//! `npcinfo`, `mercenaryinfo`, `vehicleinfo`, `regioninfo` and `jobinfo` that
//! every character and group record's references must resolve into, out of the
//! installed game's archives (`desert_tools::paz`, `paths::game_dir`) or a
//! directory of extracted copies (`--tables`, what `just extract-tables`
//! writes into `analysis/tables/`). The decoding is a port of
//! `analysis/items_catalog.py`'s `tribes()` and `characters()` and
//! `analysis/npc_catalog.py`'s `groups()` (`decode`).
//!
//! All or nothing: every `characterinfo`, `tribeinfo` and `charactergroupinfo`
//! record must pass its layout check, `Insect_Catch` must name exactly one
//! interaction, the insect and fish tribe groups exactly one record each,
//! every bag catch must have a level-2 tribe group, every
//! `CharacterGroup_Ally_*` group must be one of the seven known, and no
//! character may sit in two of them (`Ally_Guard` or `Ally_Mercenary` beside
//! `Ally_NPC` is the one pairing the game uses, and counts as the former), or
//! nothing is written.
//!
//! The rule (decided, do not change it here without changing the plugin's
//! expectations):
//!
//! - a **bag catch** is a record whose interaction list holds `Insect_Catch`
//!   and whose `convertItemInfo` is not 0. Its class is its tribe chain's
//!   level-2 group (index 2 of root -> leaf), matched by tribe KEY: the insect
//!   group -> `Bug`, the fish group -> `Fish`, any other -> `Critter`. The two
//!   group keys are found here, once, by their Korean editor names.
//! - every other record whose name starts with `Animal_` (any case) is
//!   `NotHandCatchable`.
//! - every record not named `Animal_*` whose interaction list holds
//!   `Dead_Loot` is a body: a person or monster whose corpse is searched. Its
//!   class comes from the spawn ally group it is a member of
//!   (`CharacterGroup_Ally_*`, membership by the group's
//!   `characterInfoList`; ENEMIES.md section 1), which is the game's own
//!   hostility signal:
//!   - a member of `Ally_Player`, or a record whose `nonePlayerSubType` is 3
//!     (playable: Kliff, Damiane, Oongka, Nairah and their doubles), is
//!     `Protected`: never searched, caught or skinned;
//!   - `Ally_NPC`, `Ally_Guard` or `Ally_Mercenary` is a `FriendlyBody`;
//!   - `Ally_Enemy` or `Ally_Aniaml_Wild` (sic) is a `Body`;
//!   - in no ally group (or `Ally_Aniaml_Domestic`), the `isEnableFriendly`
//!     flag decides: 1 is a `FriendlyBody`, 0 a `Body`. That flag means "can
//!     be befriended", not "is not hostile", which is why it is only the
//!     fallback.
//!
//!   `Animal_*` records with `Dead_Loot` (spiders, wyverns) stay animals.
//!
//! It prints the counts, the two group keys, every `Critter` with its group,
//! and what changed against the file it replaces.

#[path = "gen-species/decode.rs"]
mod decode;
#[path = "gen-species/render.rs"]
mod render;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;
use desert_tools::paths;
use desert_tools::paz::Archive;
use desert_tools::staticinfo::{self, Table};

const CHARACTER: &str = "characterinfo";
const INTERACTION: &str = "interactioninfo";
const TRIBE: &str = "tribeinfo";
const KNOWLEDGE: &str = "knowledgeinfo";
const ITEM: &str = "iteminfo";
const NPC: &str = "npcinfo";
const MERCENARY: &str = "mercenaryinfo";
const VEHICLE: &str = "vehicleinfo";
const CHARACTER_GROUP: &str = "charactergroupinfo";
const REGION: &str = "regioninfo";
const JOB: &str = "jobinfo";

/// The prefix of the spawn ally groups' names.
const ALLY_PREFIX: &str = "CharacterGroup_Ally_";
/// `nonePlayerSubType`'s value on a playable character.
const PLAYABLE: u8 = 3;

/// The interaction that puts a creature in the bag.
const INSECT_CATCH: &str = "Insect_Catch";
/// The interaction that searches a dead body.
const DEAD_LOOT: &str = "Dead_Loot";
/// The tribe groups' editor names: insect group, fish group.
const INSECT_GROUP: &str = "곤충 그룹";
const FISH_GROUP: &str = "어류 그룹";
/// The tribe chain depth (root = 0) whose group decides a bag catch's class.
const GROUP_LEVEL: usize = 2;

/// The checks the generator makes before writing, and emits as a test.
pub const SPOT_CHECKS: &[(&str, Option<Class>)] = &[
    ("Animal_Raccoon_Wild_30042", Some(Class::NotHandCatchable)),
    ("Animal_Duck_Wild_30020", Some(Class::NotHandCatchable)),
    ("Riding_Iguana_1", None),
    ("Animal_Iguana_Wild_30069", Some(Class::Critter)),
    ("Animal_Insect_Lobster", Some(Class::Fish)),
    ("Animal_Insect_Sea_Slater", Some(Class::Fish)),
    ("Animal_RiceFish", Some(Class::Fish)),
    ("Animal_Sparrow_Wild_30071", Some(Class::Critter)),
    ("Animal_Insect_GrassHopper", Some(Class::Bug)),
    ("NHM_Bleed_Bandits_OneHandAxe_54140", Some(Class::Body)),
    ("NHM_Bleed_Bandits_OneHandSword_54109", Some(Class::Body)),
    ("NHM_Wandering_Mercenary_OneHandMusket_03", Some(Class::FriendlyBody)),
    ("Kliff", Some(Class::Protected)),
    ("Damian", Some(Class::Protected)),
    ("Oongka", Some(Class::Protected)),
    ("Nairah", Some(Class::Protected)),
    ("NHM_Unique_Ziane_TwoHandSword_411", Some(Class::FriendlyBody)),
    ("NHM_Bastion_Henchman_OneHandSword_1", Some(Class::Body)),
    ("NHM_Corpse_3469", Some(Class::Body)),
    ("NHM_Citizen_3053", Some(Class::FriendlyBody)),
    ("Animal_Crocodile_Wild_30052", Some(Class::NotHandCatchable)),
];

#[derive(Parser)]
#[command(about = "Generate crates/desert-core/src/species_table.rs from the game's characterinfo, interactioninfo and tribeinfo")]
struct Cli {
    /// A directory of extracted tables (`<table>.staticinfobody` and
    /// `.staticinfoheader`, as `just extract-tables` writes) to read instead of
    /// the game's archives [default: the archives under $CD_GAME_DIR, else
    /// the parent of $CD_BIN64]
    #[arg(long, value_name = "DIR")]
    tables: Option<PathBuf>,
    /// Where to write [default: <repo>/crates/desert-core/src/species_table.rs]
    #[arg(long, value_name = "FILE")]
    out: Option<PathBuf>,
    /// The game build named in the header [default: the appmanifest's, else
    /// the calibrated build]
    #[arg(long, value_name = "ID")]
    build: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Bug,
    Fish,
    Critter,
    NotHandCatchable,
    Body,
    FriendlyBody,
    Protected,
}

impl Class {
    pub fn variant(self) -> &'static str {
        match self {
            Class::Bug => "Bug",
            Class::Fish => "Fish",
            Class::Critter => "Critter",
            Class::NotHandCatchable => "NotHandCatchable",
            Class::Body => "Body",
            Class::FriendlyBody => "FriendlyBody",
            Class::Protected => "Protected",
        }
    }
}

/// A spawn ally group (`CharacterGroup_Ally_<suffix>`), the game's
/// hostility signal (ENEMIES.md section 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ally {
    Player,
    Npc,
    Guard,
    Mercenary,
    Enemy,
    AnimalWild,
    AnimalDomestic,
}

impl Ally {
    /// The seven groups by name suffix; any other `CharacterGroup_Ally_*`
    /// name is refused.
    const ALL: [(&'static str, Ally); 7] = [
        ("Player", Ally::Player),
        ("NPC", Ally::Npc),
        ("Guard", Ally::Guard),
        ("Mercenary", Ally::Mercenary),
        ("Enemy", Ally::Enemy),
        ("Aniaml_Wild", Ally::AnimalWild),
        ("Aniaml_Domestic", Ally::AnimalDomestic),
    ];

    fn from_suffix(s: &str) -> Option<Ally> {
        Self::ALL.iter().find(|(n, _)| *n == s).map(|&(_, a)| a)
    }

    fn suffix(self) -> &'static str {
        Self::ALL.iter().find(|(_, a)| *a == self).map_or("?", |(n, _)| n)
    }
}

/// One character's ally groups as the game lists them, down to the one that
/// counts: `Ally_Guard` and `Ally_Mercenary` always come with `Ally_NPC`
/// (508 and 94 records on 26361232), and they are the narrower of the two.
/// `None` when two groups are left, which the generator refuses.
pub fn ally_of(groups: &[Ally]) -> Option<Option<Ally>> {
    let mut v: Vec<Ally> = groups.to_vec();
    v.sort();
    v.dedup();
    if v.len() == 2 && v.contains(&Ally::Npc) && (v.contains(&Ally::Guard) || v.contains(&Ally::Mercenary)) {
        v.retain(|a| *a != Ally::Npc);
    }
    match v.as_slice() {
        [] => Some(None),
        [a] => Some(Some(*a)),
        _ => None,
    }
}

/// The class of a body record (not `Animal_*`, with `Dead_Loot`): see the
/// module comment.
pub fn body_class(ally: Option<Ally>, none_player_sub_type: u8, enable_friendly: u8) -> Class {
    if ally == Some(Ally::Player) || none_player_sub_type == PLAYABLE {
        return Class::Protected;
    }
    match ally {
        Some(Ally::Npc | Ally::Guard | Ally::Mercenary) => Class::FriendlyBody,
        Some(Ally::Enemy | Ally::AnimalWild) => Class::Body,
        _ if enable_friendly == 1 => Class::FriendlyBody,
        _ => Class::Body,
    }
}

/// One emitted row, plus what the report says about it.
pub struct Species {
    pub name: String,
    pub class: Class,
    /// The level-2 tribe group key, for a bag catch.
    pub group: Option<u32>,
    pub convert_item: u32,
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

/// Every record of a table as `(key, bytes)`. The header's key, which equals
/// the body's on every table read here (`decode::record_start` checks it).
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

fn keys_of(src: &Source, name: &str) -> Result<HashSet<u32>> {
    Ok(records_of(name, &src.read(name)?)?.into_iter().map(|(k, _)| k).collect())
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
        cli.out.clone().unwrap_or_else(|| paths::crate_dir_in(&root, "desert-core").join("src").join("species_table.rs"));
    let build = cli.build.clone().or_else(paths::build_id).unwrap_or_else(|| paths::CALIBRATED_BUILD.to_string());
    let src = match &cli.tables {
        Some(d) => Source::Dir(d.clone()),
        None => Source::Archive(Archive::open(&paths::game_dir())?),
    };

    // interactioninfo: key -> name.
    let mut interactions: HashMap<u32, String> = HashMap::new();
    for (key, rec) in records_of(INTERACTION, &src.read(INTERACTION)?)? {
        let Some((name, _)) = decode::record_start(&rec, key) else {
            bail!("{INTERACTION}: record {key} has no readable name");
        };
        interactions.insert(key, name);
    }
    let catch: Vec<u32> = interactions.iter().filter(|(_, n)| *n == INSECT_CATCH).map(|(&k, _)| k).collect();
    let [insect_catch] = catch.as_slice() else {
        bail!("{INTERACTION}: {} records named {INSECT_CATCH}, expected exactly 1", catch.len());
    };
    let insect_catch = *insect_catch;
    let dead: Vec<u32> = interactions.iter().filter(|(_, n)| *n == DEAD_LOOT).map(|(&k, _)| k).collect();
    let [dead_loot] = dead.as_slice() else {
        bail!("{INTERACTION}: {} records named {DEAD_LOOT}, expected exactly 1", dead.len());
    };
    let dead_loot = *dead_loot;

    // tribeinfo: key -> (name, parent), every parent resolving.
    let tribe_recs = records_of(TRIBE, &src.read(TRIBE)?)?;
    let mut tribes: HashMap<u32, decode::Tribe> = HashMap::new();
    let mut fails = Vec::new();
    for (key, rec) in &tribe_recs {
        match decode::tribe(rec, *key) {
            Ok(t) => {
                tribes.insert(*key, t);
            }
            Err(e) => fails.push(format!("{key}: {e}")),
        }
    }
    for (key, t) in &tribes {
        if t.parent != 0 && !tribes.contains_key(&t.parent) {
            fails.push(format!("{key} {:?}: parent {} is not a tribe", t.name, t.parent));
        }
    }
    refuse(TRIBE, &fails, tribe_recs.len())?;
    let group_key = |name: &str| -> Result<u32> {
        let ks: Vec<u32> = tribes.iter().filter(|(_, t)| t.name == name).map(|(&k, _)| k).collect();
        match ks.as_slice() {
            [k] => Ok(*k),
            _ => bail!("{TRIBE}: {} records named {name:?}, expected exactly 1", ks.len()),
        }
    };
    let insect_group = group_key(INSECT_GROUP)?;
    let fish_group = group_key(FISH_GROUP)?;
    // root -> leaf, by key.
    let chain = |mut k: u32| -> Result<Vec<u32>> {
        let mut p = Vec::new();
        while let Some(t) = tribes.get(&k) {
            if p.contains(&k) {
                bail!("{TRIBE}: the parent chain through {k} loops");
            }
            p.push(k);
            k = t.parent;
        }
        p.reverse();
        Ok(p)
    };

    // characterinfo.
    let refs = decode::Refs {
        knowledge: keys_of(&src, KNOWLEDGE)?,
        items: keys_of(&src, ITEM)?,
        npcs: keys_of(&src, NPC)?,
        mercenaries: keys_of(&src, MERCENARY)?,
        vehicles: keys_of(&src, VEHICLE)?,
        interactions: interactions.keys().copied().collect(),
    };
    let char_recs = records_of(CHARACTER, &src.read(CHARACTER)?)?;
    let mut chars = Vec::with_capacity(char_recs.len());
    let mut fails = Vec::new();
    for (key, rec) in &char_recs {
        match decode::character(rec, *key, &refs) {
            Ok(c) => chars.push(c),
            Err(e) => fails.push(format!("{key}: {e}")),
        }
    }
    refuse(CHARACTER, &fails, char_recs.len())?;

    // charactergroupinfo: every character's spawn ally group.
    let group_refs = decode::GroupRefs {
        characters: chars.iter().map(|c| c.key).collect(),
        tribes: tribes.keys().copied().collect(),
        regions: keys_of(&src, REGION)?,
        jobs: keys_of(&src, JOB)?,
    };
    let group_recs = records_of(CHARACTER_GROUP, &src.read(CHARACTER_GROUP)?)?;
    let mut groups = Vec::with_capacity(group_recs.len());
    let mut fails = Vec::new();
    for (key, rec) in &group_recs {
        match decode::character_group(rec, *key, &group_refs) {
            Ok(g) => groups.push(g),
            Err(e) => fails.push(format!("{key}: {e}")),
        }
    }
    refuse(CHARACTER_GROUP, &fails, group_recs.len())?;
    let mut memberships: HashMap<u32, Vec<Ally>> = HashMap::new();
    let mut ally_groups: BTreeMap<Ally, usize> = BTreeMap::new();
    for g in &groups {
        let Some(suffix) = g.name.strip_prefix(ALLY_PREFIX) else { continue };
        let Some(ally) = Ally::from_suffix(suffix) else {
            bail!("{CHARACTER_GROUP}: {} is an ally group this generator does not know; nothing written", g.name);
        };
        if ally_groups.insert(ally, g.characters.len()).is_some() {
            bail!("{CHARACTER_GROUP}: two groups named {}; nothing written", g.name);
        }
        for &c in &g.characters {
            memberships.entry(c).or_default().push(ally);
        }
    }
    let mut ally: HashMap<u32, Ally> = HashMap::new();
    let mut doubles = Vec::new();
    for (&c, gs) in &memberships {
        match ally_of(gs) {
            Some(Some(a)) => {
                ally.insert(c, a);
            }
            Some(None) => {}
            None => doubles.push(format!(
                "{c}: {}",
                gs.iter().map(|a| format!("{ALLY_PREFIX}{}", a.suffix())).collect::<Vec<_>>().join(" + ")
            )),
        }
    }
    if !doubles.is_empty() {
        doubles.sort();
        bail!(
            "{CHARACTER_GROUP}: {} characters sit in two ally groups; nothing written. First ones:\n  {}",
            doubles.len(),
            doubles.iter().take(10).cloned().collect::<Vec<_>>().join("\n  ")
        );
    }

    // The rule.
    let mut rows: Vec<Species> = Vec::new();
    // (ally group, isEnableFriendly's class, the class now) -> records.
    let mut body_moves: BTreeMap<(&str, &str, &str), usize> = BTreeMap::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for c in &chars {
        if !seen.insert(c.name.as_str()) {
            bail!("{CHARACTER}: the name {} occurs more than once; the table is by name", c.name);
        }
        if c.interactions.contains(&insect_catch) && c.convert_item != 0 {
            let path = chain(c.tribe)?;
            let Some(&group) = path.get(GROUP_LEVEL) else {
                bail!(
                    "{CHARACTER}: bag catch {} ({}) has tribe {} whose chain has no level-{GROUP_LEVEL} group ({:?})",
                    c.name,
                    c.key,
                    c.tribe,
                    path
                );
            };
            let class = if group == insect_group {
                Class::Bug
            } else if group == fish_group {
                Class::Fish
            } else {
                Class::Critter
            };
            rows.push(Species { name: c.name.clone(), class, group: Some(group), convert_item: c.convert_item });
        } else if c.name.to_ascii_lowercase().starts_with("animal_") {
            rows.push(Species { name: c.name.clone(), class: Class::NotHandCatchable, group: None, convert_item: 0 });
        } else if c.interactions.contains(&dead_loot) {
            let a = ally.get(&c.key).copied();
            let class = body_class(a, c.none_player_sub_type, c.enable_friendly);
            let old = if c.enable_friendly == 1 { Class::FriendlyBody } else { Class::Body };
            *body_moves.entry((a.map_or("none", Ally::suffix), old.variant(), class.variant())).or_insert(0) += 1;
            rows.push(Species { name: c.name.clone(), class, group: None, convert_item: 0 });
        }
    }
    rows.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));

    let class_of = |name: &str| rows.iter().find(|r| r.name == name).map(|r| r.class);
    let wrong: Vec<String> = SPOT_CHECKS
        .iter()
        .filter(|(n, want)| class_of(n) != *want)
        .map(|(n, want)| format!("{n}: {:?}, expected {want:?}", class_of(n)))
        .collect();
    if !wrong.is_empty() {
        bail!("spot checks fail; nothing written:\n  {}", wrong.join("\n  "));
    }

    let count = |c: Class| rows.iter().filter(|r| r.class == c).count();
    let counts = render::Counts {
        bug: count(Class::Bug),
        fish: count(Class::Fish),
        critter: count(Class::Critter),
        not_hand_catchable: count(Class::NotHandCatchable),
        body: count(Class::Body),
        friendly_body: count(Class::FriendlyBody),
        protected: count(Class::Protected),
    };
    let previous = fs::read_to_string(&out_path).map(|t| render::previous_rows(&t)).unwrap_or_default();
    let text = render::species_table_rs(&rows, &counts, &build);
    fs::write(&out_path, text).with_context(|| format!("cannot write {}", out_path.display()))?;

    // The report.
    println!("source: {}", src.describe());
    println!("build: {build}");
    println!(
        "records: {} {CHARACTER}, {} {TRIBE}, {} {INTERACTION}, {} {CHARACTER_GROUP}; every one passes the layout check",
        char_recs.len(),
        tribe_recs.len(),
        interactions.len(),
        group_recs.len()
    );
    println!(
        "ally groups (members): {}; {} characters in one",
        ally_groups.iter().map(|(a, n)| format!("{} {n}", a.suffix())).collect::<Vec<_>>().join(", "),
        ally.len()
    );
    println!("{INSECT_CATCH}: interaction key {insect_catch}; {DEAD_LOOT}: interaction key {dead_loot}");
    println!("tribe groups: insect {INSECT_GROUP} = {insect_group}, fish {FISH_GROUP} = {fish_group}");
    let not_animal: Vec<&str> = rows
        .iter()
        .filter(|r| {
            !matches!(r.class, Class::NotHandCatchable | Class::Body | Class::FriendlyBody | Class::Protected)
                && !r.name.to_ascii_lowercase().starts_with("animal_")
        })
        .map(|r| r.name.as_str())
        .collect();
    println!(
        "rows: {} = Bug {}, Fish {}, Critter {} ({} bag catches), NotHandCatchable {}, Body {}, FriendlyBody {}, Protected {}",
        rows.len(),
        counts.bug,
        counts.fish,
        counts.critter,
        counts.bug + counts.fish + counts.critter,
        counts.not_hand_catchable,
        counts.body,
        counts.friendly_body,
        counts.protected
    );
    println!(
        "bodies: {} (ally group, by isEnableFriendly -> now: records)",
        counts.body + counts.friendly_body + counts.protected
    );
    for ((a, old, new), n) in &body_moves {
        println!("  {a:<16} {old:<12} -> {new:<12} {n}");
    }
    let protected: Vec<&str> = rows.iter().filter(|r| r.class == Class::Protected).map(|r| r.name.as_str()).collect();
    println!("Protected: {}", protected.join(", "));
    let animal_dead_loot: Vec<&str> = chars
        .iter()
        .filter(|c| c.interactions.contains(&dead_loot) && c.name.to_ascii_lowercase().starts_with("animal_"))
        .map(|c| c.name.as_str())
        .collect();
    println!(
        "Animal_* records with {DEAD_LOOT}, kept as animals: {}: {}",
        animal_dead_loot.len(),
        animal_dead_loot.join(", ")
    );
    if !not_animal.is_empty() {
        println!("bag catches not named Animal_*: {}", not_animal.join(", "));
    }
    let mut by_group: BTreeMap<String, Vec<&Species>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.class == Class::Critter) {
        let g = r.group.unwrap_or(0);
        let gname = tribes.get(&g).map_or("?", |t| t.name.as_str());
        by_group.entry(format!("{g} {gname}")).or_default().push(r);
    }
    for (g, rs) in &by_group {
        println!("Critter group {g}: {}", rs.len());
        for r in rs {
            println!("  {} (item {})", r.name, r.convert_item);
        }
    }
    let now: BTreeMap<&str, &str> = rows.iter().map(|r| (r.name.as_str(), r.class.variant())).collect();
    let added: Vec<String> =
        now.iter().filter(|(n, _)| !previous.contains_key(**n)).map(|(n, c)| format!("  + {n} {c}")).collect();
    let removed: Vec<String> =
        previous.iter().filter(|(n, _)| !now.contains_key(n.as_str())).map(|(n, c)| format!("  - {n} {c}")).collect();
    let moved: Vec<String> = now
        .iter()
        .filter_map(|(n, c)| previous.get(*n).filter(|p| p != c).map(|p| format!("  ~ {n} {p} -> {c}")))
        .collect();
    println!(
        "against the previous {}: {} rows before, {} added, {} removed, {} moved class",
        out_path.display(),
        previous.len(),
        added.len(),
        removed.len(),
        moved.len()
    );
    // A first run adds every row; listing a thousand of them says nothing,
    // and neither does listing thousands of new rows of one class.
    if !previous.is_empty() && added.len() + removed.len() + moved.len() <= 200 {
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
            eprintln!("gen-species: {err:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_and_mercenary_beside_npc_count_as_themselves() {
        assert_eq!(ally_of(&[]), Some(None));
        assert_eq!(ally_of(&[Ally::Enemy]), Some(Some(Ally::Enemy)));
        assert_eq!(ally_of(&[Ally::Npc, Ally::Guard]), Some(Some(Ally::Guard)));
        assert_eq!(ally_of(&[Ally::Mercenary, Ally::Npc]), Some(Some(Ally::Mercenary)));
        assert_eq!(ally_of(&[Ally::Npc, Ally::Npc]), Some(Some(Ally::Npc)));
        // Any other pair is refused.
        assert_eq!(ally_of(&[Ally::Npc, Ally::Enemy]), None);
        assert_eq!(ally_of(&[Ally::Guard, Ally::Mercenary]), None);
        assert_eq!(ally_of(&[Ally::Npc, Ally::Guard, Ally::Mercenary]), None);
        assert_eq!(ally_of(&[Ally::Player, Ally::Enemy]), None);
    }

    #[test]
    fn the_body_rule() {
        // Playable wins over every group, and over the flag.
        for a in [None, Some(Ally::Enemy), Some(Ally::Npc), Some(Ally::Player)] {
            for f in [0, 1] {
                assert_eq!(body_class(a, PLAYABLE, f), Class::Protected, "{a:?} {f}");
            }
        }
        for f in [0, 1] {
            assert_eq!(body_class(Some(Ally::Player), 0, f), Class::Protected);
            assert_eq!(body_class(Some(Ally::Npc), 0, f), Class::FriendlyBody);
            assert_eq!(body_class(Some(Ally::Guard), 1, f), Class::FriendlyBody);
            assert_eq!(body_class(Some(Ally::Mercenary), 0, f), Class::FriendlyBody);
            assert_eq!(body_class(Some(Ally::Enemy), 0, f), Class::Body);
            assert_eq!(body_class(Some(Ally::AnimalWild), 1, f), Class::Body);
        }
        // No group: the flag.
        assert_eq!(body_class(None, 0, 0), Class::Body);
        assert_eq!(body_class(None, 1, 1), Class::FriendlyBody);
        assert_eq!(body_class(Some(Ally::AnimalDomestic), 0, 1), Class::FriendlyBody);
    }
}
