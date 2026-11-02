//! The family rule: which gather family a `gimmickinfo` record belongs to,
//! read off the record itself and the tables it points into.
//!
//! Every decode here is from the 26361232 deserializers (GimmickInfo
//! `0x170A52A`..`0x170C555`, ItemInfo from `0x1718196`) and was checked over
//! the whole table (`analysis/FINDINGS.md` section 7.2). Each one is a small
//! function over a byte slice that returns `None` rather than reading out of
//! bounds, so a record the decode does not fit is simply not classified.
//!
//! ```text
//! gimmickinfo record (NameFirst)
//!   u32 name_len, name, u32 key, u8 0, u32 path_len, path
//!   gimmickInteractionOverrideDataList   u8 present, u32 count,
//!                                        per element u8 present + InteractionOverrideData,
//!                                        whose first field is u32 originInteractionInfo
//!   ...
//!   92 one-byte fields                   collectFilter_Dev is index 75, knowledgeExtractType 76
//!   u64 respawnTimeSeconds
//!   elementalMaterialInfoList            u32 count, count x 8 bytes
//!   3 optional-pointer flags             1 byte each, 0 = absent
//!   tail, 21 bytes                       u32 defaultSpawnReasonHash, u32 additionalHeightOnCatched,
//!                                        u32 saveOption, u32 knowledgeInfo, 5 bools
//! iteminfo record
//!   u32 name_len, name, u32 key, ...     materialMatchInfo is the u32 at key + 80
//! ```
//!
//! The `u8` that `desert_tools::records` reads as the header's trailing flag is
//! the interaction list's `present` byte: the header ends at the path.
//!
//! The rule is [`RULES`], one row per family, and a record that two rows claim
//! is an error rather than a precedence: on 26361232 no record is claimed
//! twice, and if a game update changes that, a person should decide.

use std::collections::{HashMap, HashSet};

use desert_tools::records::{header_at, HeaderShape};

/// The interactions the rule names, by their `interactioninfo` record names.
/// The numeric keys are resolved from that table on every run, never written
/// down here.
pub const COLLECT: &str = "Gimmick_Collect";
pub const SCOOP: &str = "Gimmick_Scoop";
pub const PICK_UP: &str = "Gimmick_PickUp";

/// `materialMatchInfo` is a `lookup3` hash of the material's name with this
/// seed.
pub const MATERIAL_SEED: u32 = 0xC5EDE;
pub const WOOD: &str = "wood";

/// Materials worth naming in the report. Every ore and stone item carries
/// `0xAA24F6B9`, whose name is not known.
const NAMED_MATERIALS: &[&str] = &["wood", "grass"];

/// The fixed record tail: four `u32`s and five bools.
const TAIL: usize = 21;
/// `knowledgeInfo` counted back from the record's end.
const KNOWLEDGE_FROM_END: usize = 9;
/// The three optional-pointer flags before the tail.
const FLAGS: usize = 3;
/// Bytes in the one-byte field run, and where the two fields sit in it.
const RUN: usize = 92;
const COLLECT_FILTER_AT: usize = 75;
const EXTRACT_TYPE_AT: usize = 76;
/// `respawnTimeSeconds`, between the run and the material list.
const RESPAWN: usize = 8;
/// Element size of `elementalMaterialInfoList`, and the most elements the
/// backward walk will try. The smallest count that fits is taken: on 26361232
/// 85 records fit more than one, and on the four of them that pay anything
/// every fit reads the same `collectFilter_Dev`.
const ELEMENT: usize = 8;
const MAX_ELEMENTS: usize = 255;
/// Where `materialMatchInfo` sits, counted from the item record's key.
const MATERIAL_FROM_KEY: usize = 80;

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

/// Bob Jenkins' `lookup3` `hashlittle`, as the game hashes material names.
/// A port of `analysis/lookup3.py`.
pub fn hashlittle(data: &[u8], initval: u32) -> u32 {
    fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(4);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(6);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(8);
        *b = b.wrapping_add(*a);
        *a = a.wrapping_sub(*c);
        *a ^= c.rotate_left(16);
        *c = c.wrapping_add(*b);
        *b = b.wrapping_sub(*a);
        *b ^= a.rotate_left(19);
        *a = a.wrapping_add(*c);
        *c = c.wrapping_sub(*b);
        *c ^= b.rotate_left(4);
        *b = b.wrapping_add(*a);
    }
    fn final_mix(mut a: u32, mut b: u32, mut c: u32) -> u32 {
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(14));
        a ^= c;
        a = a.wrapping_sub(c.rotate_left(11));
        b ^= a;
        b = b.wrapping_sub(a.rotate_left(25));
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(16));
        a ^= c;
        a = a.wrapping_sub(c.rotate_left(4));
        b ^= a;
        b = b.wrapping_sub(a.rotate_left(14));
        c ^= b;
        c = c.wrapping_sub(b.rotate_left(24));
        c
    }
    let word = |chunk: &[u8]| -> u32 {
        let mut w = [0u8; 4];
        for (d, s) in w.iter_mut().zip(chunk) {
            *d = *s;
        }
        u32::from_le_bytes(w)
    };
    let init = 0xDEAD_BEEFu32.wrapping_add(data.len() as u32).wrapping_add(initval);
    let (mut a, mut b, mut c) = (init, init, init);
    let mut rest = data;
    while rest.len() > 12 {
        a = a.wrapping_add(word(&rest[0..4]));
        b = b.wrapping_add(word(&rest[4..8]));
        c = c.wrapping_add(word(&rest[8..12]));
        mix(&mut a, &mut b, &mut c);
        rest = &rest[12..];
    }
    if rest.is_empty() {
        return c;
    }
    // The tail, zero-padded: the same as masking the over-read words.
    a = a.wrapping_add(word(rest.get(0..rest.len().min(4)).unwrap_or(&[])));
    if rest.len() > 4 {
        b = b.wrapping_add(word(rest.get(4..rest.len().min(8)).unwrap_or(&[])));
    }
    if rest.len() > 8 {
        c = c.wrapping_add(word(rest.get(8..rest.len()).unwrap_or(&[])));
    }
    final_mix(a, b, c)
}

/// `materialMatchInfo` for a material name.
pub fn material_hash(name: &str) -> u32 {
    hashlittle(name.as_bytes(), MATERIAL_SEED)
}

/// A material hash as a name where one is known, else as hex.
pub fn material_name(hash: u32) -> String {
    NAMED_MATERIALS
        .iter()
        .find(|n| material_hash(n) == hash)
        .map_or_else(|| format!("{hash:#010X}"), |n| (*n).to_string())
}

/// Where a `NameFirst` record's fields start: the interaction list's
/// `present` byte, one before `desert_tools::records`' `body`.
pub fn fields_at(rec: &[u8]) -> Option<usize> {
    header_at(rec, 0, HeaderShape::NameFirst)?.body.checked_sub(1)
}

/// `originInteractionInfo` of the first element of
/// `gimmickInteractionOverrideDataList`: `None` when the list is absent, empty,
/// or its first element is.
pub fn first_interaction(rec: &[u8]) -> Option<u32> {
    let at = fields_at(rec)?;
    if *rec.get(at)? != 1 {
        return None;
    }
    if u32_at(rec, at + 1)? == 0 {
        return None;
    }
    if *rec.get(at + 5)? != 1 {
        return None;
    }
    u32_at(rec, at + 6)
}

/// `knowledgeInfo`, the `u32` key nine bytes from the record's end.
pub fn knowledge(rec: &[u8]) -> Option<u32> {
    if rec.len() < TAIL {
        return None;
    }
    u32_at(rec, rec.len().checked_sub(KNOWLEDGE_FROM_END)?)
}

/// `(collectFilter_Dev, knowledgeExtractType)`, reached by walking back from
/// the tail: the three optional flags must all be 0 (a set one means a nested
/// struct of unknown length sits there), then `elementalMaterialInfoList`
/// (the smallest element count whose `u32` count fits), then the `u64`
/// respawn time, then the 92-byte run. The run has to start after the
/// record's header.
pub fn tail_run(rec: &[u8]) -> Option<(u8, u8)> {
    let floor = fields_at(rec)?;
    let flags = rec.len().checked_sub(TAIL + FLAGS)?;
    if rec.get(flags..flags + FLAGS)?.iter().any(|&f| f != 0) {
        return None;
    }
    for n in 0..=MAX_ELEMENTS {
        let count_at = flags.checked_sub(4 + n * ELEMENT)?;
        if u32_at(rec, count_at)? as usize != n {
            continue;
        }
        let run = count_at.checked_sub(RESPAWN + RUN)?;
        if run < floor {
            return None;
        }
        return Some((*rec.get(run + COLLECT_FILTER_AT)?, *rec.get(run + EXTRACT_TYPE_AT)?));
    }
    None
}

/// `materialMatchInfo` of one `iteminfo` record: the `u32` 80 bytes past its
/// key, which follows the name. The key is checked against `key`.
pub fn item_material(rec: &[u8], key: u32) -> Option<u32> {
    let name_len = u32_at(rec, 0)? as usize;
    let key_at = 4usize.checked_add(name_len)?;
    if u32_at(rec, key_at)? != key {
        return None;
    }
    u32_at(rec, key_at.checked_add(MATERIAL_FROM_KEY)?)
}

/// The name of a `NameFirst`-style record in `interactioninfo` (`u32 len,
/// name, u32 key`), with its key checked.
pub fn record_name(rec: &[u8], key: u32) -> Option<String> {
    let len = u32_at(rec, 0)? as usize;
    let name = rec.get(4..4usize.checked_add(len)?)?;
    if u32_at(rec, 4 + len)? != key || !name.iter().all(|c| (0x20..=0x7E).contains(c)) {
        return None;
    }
    Some(String::from_utf8_lossy(name).into_owned())
}

/// What the other three tables say, keyed for the rule.
#[derive(Default)]
pub struct Lookups {
    /// `interactioninfo`: key -> record name.
    pub interactions: HashMap<u32, String>,
    /// `iteminfo`: key -> `materialMatchInfo`.
    pub materials: HashMap<u32, u32>,
    /// `knowledgeinfo`: every key.
    pub knowledge: HashSet<u32>,
}

/// Everything decoded off one record that the rule reads, kept for the
/// report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub interaction: Option<String>,
    pub knowledge: Option<u32>,
    pub collect_filter: Option<u8>,
    pub extract_type: Option<u8>,
    /// `materialMatchInfo` of every paid item, in item order; `None` for an
    /// item `iteminfo` does not have.
    pub materials: Vec<Option<u32>>,
}

impl Facts {
    pub fn of(rec: &[u8], items: &[u32], lk: &Lookups) -> Facts {
        let run = tail_run(rec);
        Facts {
            interaction: first_interaction(rec).and_then(|k| lk.interactions.get(&k).cloned()),
            knowledge: knowledge(rec),
            collect_filter: run.map(|r| r.0),
            extract_type: run.map(|r| r.1),
            materials: items.iter().map(|i| lk.materials.get(i).copied()).collect(),
        }
    }

    fn is(&self, interaction: &str) -> bool {
        self.interaction.as_deref() == Some(interaction)
    }

    /// Every paid item is wood. False for a record that pays nothing.
    fn all_wood(&self) -> bool {
        let wood = material_hash(WOOD);
        !self.materials.is_empty() && self.materials.iter().all(|m| *m == Some(wood))
    }

    /// One line for the report.
    pub fn describe(&self) -> String {
        let opt = |v: Option<String>| v.unwrap_or_else(|| "?".to_string());
        let mats: Vec<String> =
            self.materials.iter().map(|m| m.map_or_else(|| "?".to_string(), material_name)).collect();
        format!(
            "interaction={} knowledge={} collectFilter_Dev={} knowledgeExtractType={} materials=[{}]",
            self.interaction.as_deref().unwrap_or("none"),
            opt(self.knowledge.map(|k| k.to_string())),
            opt(self.collect_filter.map(|v| v.to_string())),
            opt(self.extract_type.map(|v| v.to_string())),
            mats.join(", ")
        )
    }
}

/// One family's rule: its name, the sentence the generated doc comment
/// carries, and the test.
pub struct Rule {
    pub family: &'static str,
    pub text: &'static str,
    pub test: fn(&Facts) -> bool,
}

/// The four gather families, in the order they are emitted.
pub const RULES: &[Rule] = &[
    Rule {
        family: "Foraging",
        text: "Rule: the record's first interaction override is `Gimmick_Collect` or `Gimmick_Scoop`.",
        test: |f| f.is(COLLECT) || f.is(SCOOP),
    },
    Rule {
        family: "Logging",
        text: "Rule: `Gimmick_PickUp`, no `knowledgeInfo`, and every item the record pays has the `wood` material.",
        test: |f| f.is(PICK_UP) && f.knowledge == Some(0) && f.all_wood(),
    },
    Rule {
        family: "Mining",
        text: "Rule: `collectFilter_Dev == 2`.",
        test: |f| f.collect_filter == Some(2),
    },
    Rule {
        family: "Ore",
        text: "Rule: `Gimmick_PickUp` with a non-zero `knowledgeInfo`.",
        test: |f| f.is(PICK_UP) && f.knowledge.is_some_and(|k| k != 0),
    },
];

/// Every family whose rule the record meets. The caller decides what more
/// than one means (it is an error).
pub fn families(f: &Facts) -> Vec<&'static str> {
    RULES.iter().filter(|r| (r.test)(f)).map(|r| r.family).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const WOOD_HASH: u32 = 0x850B_B20C;

    /// A `NameFirst` record: header, then `fields`, then `tail_body` bytes and
    /// the fixed 21-byte tail carrying `knowledge`.
    fn record(name: &str, key: u32, fields: &[u8], before_tail: &[u8], knowledge: u32) -> Vec<u8> {
        let mut r = (name.len() as u32).to_le_bytes().to_vec();
        r.extend_from_slice(name.as_bytes());
        r.extend_from_slice(&key.to_le_bytes());
        r.push(0);
        let path = "/object/x.prefab";
        r.extend_from_slice(&(path.len() as u32).to_le_bytes());
        r.extend_from_slice(path.as_bytes());
        r.extend_from_slice(fields);
        r.extend_from_slice(before_tail);
        r.extend_from_slice(&[0u8; 12]);
        r.extend_from_slice(&knowledge.to_le_bytes());
        r.extend_from_slice(&[0, 1, 0, 0, 0]);
        r
    }

    /// The interaction list with one present element naming `key`.
    fn interaction(key: u32) -> Vec<u8> {
        let mut v = vec![1];
        v.extend_from_slice(&1u32.to_le_bytes());
        v.push(1);
        v.extend_from_slice(&key.to_le_bytes());
        v.extend_from_slice(&[0u8; 16]);
        v
    }

    /// The run, the respawn time, `elements` 8-byte elements and the three
    /// flags, ending where the tail starts.
    fn run(collect_filter: u8, elements: usize, flags: [u8; 3]) -> Vec<u8> {
        let mut v = vec![0u8; RUN];
        v[COLLECT_FILTER_AT] = collect_filter;
        v[EXTRACT_TYPE_AT] = 1;
        v.extend_from_slice(&3600u64.to_le_bytes());
        v.extend_from_slice(&(elements as u32).to_le_bytes());
        v.extend(std::iter::repeat_n(0xAB, elements * ELEMENT));
        v.extend_from_slice(&flags);
        v
    }

    #[test]
    fn lookup3_hashes_the_material_names() {
        assert_eq!(material_hash("wood"), WOOD_HASH);
        assert_eq!(material_hash("grass"), 0xBC9C_F63D);
        assert_eq!(material_name(WOOD_HASH), "wood");
        assert_eq!(material_name(0xAA24_F6B9), "0xAA24F6B9");
        // Lengths past one 12-byte round and exactly on one.
        assert_eq!(hashlittle(b"", 0), 0xDEAD_BEEF);
        assert_ne!(hashlittle(b"0123456789ab", 0), hashlittle(b"0123456789abc", 0));
    }

    #[test]
    fn the_first_interaction_is_read_after_the_path() {
        let r = record("peony_01", 17020006, &interaction(10028), &run(0, 0, [0; 3]), 0);
        assert_eq!(first_interaction(&r), Some(10028));
        // An absent list, and a list whose first element is absent.
        let r = record("x", 1, &[0, 7, 0, 0, 0], &run(0, 0, [0; 3]), 0);
        assert_eq!(first_interaction(&r), None);
        let mut f = interaction(10028);
        f[5] = 0;
        assert_eq!(first_interaction(&record("x", 1, &f, &[], 0)), None);
        assert_eq!(first_interaction(&[1, 0, 0]), None);
    }

    #[test]
    fn the_tail_gives_knowledge_and_the_run_gives_the_collect_filter() {
        let r = record("mine_iron", 7, &interaction(1), &run(2, 0, [0; 3]), 17045);
        assert_eq!(knowledge(&r), Some(17045));
        assert_eq!(tail_run(&r), Some((2, 1)));
        // Elements in the material list are walked over by their count.
        let r = record("mine_iron", 7, &interaction(1), &run(2, 3, [0; 3]), 0);
        assert_eq!(tail_run(&r), Some((2, 1)));
        // A set optional flag stops the walk.
        let r = record("x", 7, &interaction(1), &run(2, 0, [0, 1, 0]), 0);
        assert_eq!(tail_run(&r), None);
        // A run that would start inside the header is not a run.
        let r = record("x", 7, &[0], &[0u8; 20], 0);
        assert_eq!(tail_run(&r), None);
        assert_eq!(knowledge(&[0u8; 20]), None);
    }

    #[test]
    fn the_item_material_sits_80_bytes_past_the_key() {
        let mut rec = 6u32.to_le_bytes().to_vec();
        rec.extend_from_slice(b"Timber");
        rec.extend_from_slice(&710001u32.to_le_bytes());
        rec.extend_from_slice(&[0u8; MATERIAL_FROM_KEY - 4]);
        rec.extend_from_slice(&WOOD_HASH.to_le_bytes());
        assert_eq!(item_material(&rec, 710001), Some(WOOD_HASH));
        assert_eq!(item_material(&rec, 710002), None, "the key is checked");
        assert_eq!(item_material(&rec[..rec.len() - 1], 710001), None);
        assert_eq!(record_name(&rec, 710001).as_deref(), Some("Timber"));
    }

    fn facts(interaction: Option<&str>, knowledge: u32, cf: u8, materials: &[u32]) -> Facts {
        Facts {
            interaction: interaction.map(str::to_string),
            knowledge: Some(knowledge),
            collect_filter: Some(cf),
            extract_type: Some(0),
            materials: materials.iter().map(|m| Some(*m)).collect(),
        }
    }

    #[test]
    fn the_rule_names_one_family_per_record() {
        assert_eq!(families(&facts(Some(COLLECT), 0, 0, &[1])), ["Foraging"]);
        assert_eq!(families(&facts(Some(SCOOP), 5, 0, &[1])), ["Foraging"]);
        assert_eq!(families(&facts(Some(PICK_UP), 0, 0, &[WOOD_HASH, WOOD_HASH])), ["Logging"]);
        assert_eq!(families(&facts(Some(PICK_UP), 17045, 0, &[1])), ["Ore"]);
        assert_eq!(families(&facts(None, 0, 2, &[1])), ["Mining"]);
        // Not every item wood, no interaction, the well's interaction.
        assert!(families(&facts(Some(PICK_UP), 0, 0, &[WOOD_HASH, 1])).is_empty());
        assert!(families(&facts(None, 0, 0, &[WOOD_HASH])).is_empty());
        assert!(families(&facts(Some("Gimmick_Fetch_Water"), 0, 0, &[1])).is_empty());
        // A record that pays nothing is never Logging.
        assert!(families(&facts(Some(PICK_UP), 0, 0, &[])).is_empty());
        // Two rules at once is reported, not resolved.
        assert_eq!(families(&facts(Some(PICK_UP), 9, 2, &[1])), ["Mining", "Ore"]);
    }

    #[test]
    fn facts_come_off_the_record_and_the_lookups() {
        let lk = Lookups {
            interactions: [(1000004, PICK_UP.to_string())].into_iter().collect(),
            materials: [(710001, WOOD_HASH)].into_iter().collect(),
            knowledge: HashSet::new(),
        };
        let r = record("firewood_0001", 1002971, &interaction(1000004), &run(0, 0, [0; 3]), 0);
        let f = Facts::of(&r, &[710001], &lk);
        assert_eq!(families(&f), ["Logging"]);
        assert!(f.describe().contains("interaction=Gimmick_PickUp"), "{}", f.describe());
        assert!(f.describe().contains("materials=[wood]"), "{}", f.describe());
        let f = Facts::of(&r, &[999], &lk);
        assert!(families(&f).is_empty(), "an item iteminfo lacks is not wood");
    }
}
