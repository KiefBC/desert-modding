//! The two record layouts `gen-species` reads: `tribeinfo` and `characterinfo`.
//!
//! A port of `analysis/items_catalog.py`'s `tribes()` and `characters()`,
//! verified on build 26361232 (every one of the 383 tribe records and the 7315
//! character records passes the check below there). Both tables use the
//! `NameFirst` record start: `u32 name_len, name, u32 key`; the key repeats the
//! header's and the fields follow it.

use std::collections::HashSet;

fn u8_at(b: &[u8], o: usize) -> Option<u8> {
    b.get(o).copied()
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

/// The record's name and where its fields start (just past the `u32 key`),
/// when the name is printable ASCII and the key after it is `key`.
pub fn record_start(rec: &[u8], key: u32) -> Option<(String, usize)> {
    let len = u32_at(rec, 0)? as usize;
    let name = rec.get(4..4usize.checked_add(len)?)?;
    if u32_at(rec, 4 + len)? != key || !name.iter().all(|c| (0x20..=0x7E).contains(c)) {
        return None;
    }
    Some((String::from_utf8_lossy(name).into_owned(), 4 + len + 4))
}

/// A plain string (reader `0x1601490`): `u32 len`, bytes. Returns the text and
/// the offset past it.
fn pstr(b: &[u8], o: usize) -> Option<(String, usize)> {
    let n = u32_at(b, o)? as usize;
    let s = b.get(o + 4..(o + 4).checked_add(n)?)?;
    Some((String::from_utf8(s.to_vec()).ok()?, o + 4 + n))
}

/// A localised string (reader `0x149F270`): `u8 kind, u64 table key = (record
/// key << 32) | column, u32 len`, the key in decimal. Returns the table key and
/// the offset past it.
fn lstr(b: &[u8], o: usize) -> Option<(u64, usize)> {
    u8_at(b, o)?;
    let key = u64_at(b, o + 1)?;
    let n = u32_at(b, o + 9)? as usize;
    b.get(o + 13..(o + 13).checked_add(n)?)?;
    Some((key, o + 13 + n))
}

/// One `tribeinfo` record.
pub struct Tribe {
    /// The editor name, Korean (`곤충 그룹`, ...).
    pub name: String,
    /// `parentTribeInfo`, 0 at a root.
    pub parent: u32,
}

/// `TribeInfo`: `u32 key`, 17 one-byte fields (`isBlocked` .. `isVehicle`),
/// `activityPresetInfo`, `parentTribeInfo` (`u32` at fields + 21), ...; the
/// editor name, a plain string (empty on some), at fields + 69 on every
/// record. The caller
/// checks the parent resolves.
pub fn tribe(rec: &[u8], key: u32) -> Result<Tribe, String> {
    let (_, f) = record_start(rec, key).ok_or("no NameFirst record start")?;
    // items_catalog.py counts from the key (`body`), the fields start 4 later.
    let b = f - 4;
    let parent = u32_at(rec, b + 25).ok_or("parentTribeInfo past the record end")?;
    let (name, _) = pstr(rec, b + 73).ok_or("no editor name at body+73")?;
    if name.chars().any(char::is_control) {
        return Err(format!("editor name {name:?} is not printable"));
    }
    Ok(Tribe { name, parent })
}

/// `characterinfo`'s fixed run after the flags, in order, with each field's
/// width from its reader (`items_catalog.py` `CHAR_LAY`).
pub const CHAR_LAY: &[(&str, usize)] = &[
    ("dailyRoutineInfo", 4),
    ("interactionCategoryGroupInfo", 2),
    ("activityPresetInfo", 4),
    ("statChangeInfoWrapper", 4),
    ("detectReactionInfo", 4),
    ("knowledgeInfo", 4),
    ("uiMapTextureInfo", 4),
    ("talkTreeInfo", 2),
    ("maxAggroCount", 2),
    ("jobInfo", 2),
    ("callVehicleGimmickInfo", 4),
    ("breakableObjectInfo", 2),
    ("convertItemInfo", 4),
    ("dialogVoiceInfo", 2),
    ("tribeInfoWrapper", 4),
    ("memo", 4),
    ("lowerActionChartPackageGroupName", 4),
    ("upperActionChartPackageGroupName", 4),
    ("characterTribeAndGenderString", 4),
    ("skeletonVariationName", 4),
    ("factionInfo", 4),
    ("characterPrefabPath", 4),
    ("socketFileName", 4),
    ("skeletonName", 4),
    ("uiDisplayLevelIconTexture", 4),
    ("projectileInfoPackage", 4),
    ("characterGamePlayDataName", 4),
    ("appearanceName", 4),
    ("uiIconPath", 4),
    ("playerIndex", 2),
    ("ownedMercenaryCharacterInfo", 4),
    ("uiPortraitPath", 4),
    ("symbolImage", 4),
    ("category", 4),
    ("equipInfo", 4),
    ("vehicleInfo", 2),
    ("personalityInfo", 2),
    ("npcInfo", 4),
    ("mercenaryInfo", 1),
    ("empoweredOverlayColor", 4),
    ("characterWeight", 4),
    ("subZoneTagHash", 4),
    ("empoweredOverlayColorRatio", 4),
    ("grownLevel", 4),
    ("gamePlayTriggerZone", 4),
    ("baseMaterialKeyOverride", 4),
    ("armorMaterialKeyOverride", 4),
    ("characterPauseType", 4),
    ("aiScriptPathHash", 4),
    ("tribeEffectHash", 4),
    ("farmBreedingCoolTime", 4),
    ("aiScriptPathFocusHash", 4),
    ("interactionDistance", 4),
    ("defaultActionActionIndex", 4),
];

/// The 64 one-byte flags (`isBlocked` .. `isTeleportable`) and the 4 bytes
/// after them that read 0 on all but one record.
const CHAR_FLAG_BYTES: usize = 68;
/// `isEnableFriendly`'s index among the 64 flags (`items_catalog.py`
/// `CHAR_FLAGS`): 1 on people who were not enemies (citizens, traders,
/// mercenaries; the white map marker), 0 on bandits and monsters.
const IS_ENABLE_FRIENDLY: usize = 39;
/// `teleportSocketName`, `reviveSocketName`.
const CHAR_SOCKETS: usize = 8;
/// `characterAge`, `playerLockOnType`, `characterTier`, `gender`,
/// `playerTargetableType`.
const CHAR_SMALL: usize = 5;
/// The column of the `characterName` localised string.
const NAME_COLUMN: u64 = 0x30;

/// The key sets a character record's references must resolve into.
pub struct Refs {
    pub knowledge: HashSet<u32>,
    pub items: HashSet<u32>,
    pub npcs: HashSet<u32>,
    pub mercenaries: HashSet<u32>,
    pub vehicles: HashSet<u32>,
    pub interactions: HashSet<u32>,
}

/// What `gen-species` needs out of a `characterinfo` record.
pub struct Character {
    pub key: u32,
    pub name: String,
    pub convert_item: u32,
    pub tribe: u32,
    pub interactions: Vec<u32>,
    /// The `isEnableFriendly` flag byte.
    pub enable_friendly: u8,
}

fn field(v: &[u32], name: &str) -> u32 {
    CHAR_LAY.iter().position(|(f, _)| *f == name).and_then(|i| v.get(i).copied()).unwrap_or(0)
}

/// `CharacterInfo`: `u32 key`, the flags, the sockets, the five small fields,
/// [`CHAR_LAY`], `visioningData` (`u8 kind`; kind 0 is followed by a `u32`),
/// `vanishTickCount`, `terrainRegionAutoSpawnInfo`, eight `u32`,
/// `characterName` (localised, column 0x30), `characterDesc`,
/// `characterEditName` (plain), three `u64`. The record passes when the name's
/// table key is `(key << 32) | 0x30` and `knowledgeInfo`, `convertItemInfo`,
/// `npcInfo`, `mercenaryInfo` and `vehicleInfo` each resolve (or are empty).
///
/// The interaction list is found by shape after that: a `u32` count of 1..=40
/// `interactioninfo` keys that repeats at once
/// (`interactionInfoListWithRecursiveChild`); failing that, the first such
/// list with a key of at least 1000; failing that, none.
pub fn character(rec: &[u8], key: u32, refs: &Refs) -> Result<Character, String> {
    let (name, f) = record_start(rec, key).ok_or("no NameFirst record start")?;
    let enable_friendly = u8_at(rec, f + IS_ENABLE_FRIENDLY).ok_or("isEnableFriendly past the record end")?;
    if enable_friendly > 1 {
        return Err(format!("isEnableFriendly reads {enable_friendly}, not a bool"));
    }
    let mut o = f + CHAR_FLAG_BYTES + CHAR_SOCKETS + CHAR_SMALL;
    let mut v = Vec::with_capacity(CHAR_LAY.len());
    for (fname, w) in CHAR_LAY {
        let x = match w {
            4 => u32_at(rec, o),
            2 => u16_at(rec, o).map(u32::from),
            _ => u8_at(rec, o).map(u32::from),
        };
        v.push(x.ok_or_else(|| format!("{fname} past the record end"))?);
        o += w;
    }
    let vk = u8_at(rec, o).ok_or("visioningData past the record end")?;
    o += 1;
    if vk == 0 {
        o += 4;
    }
    o += 8 + 32;
    let (lk, o2) = lstr(rec, o).ok_or("no characterName")?;
    if lk >> 32 != u64::from(key) || lk & 0xFFFF_FFFF != NAME_COLUMN {
        return Err(format!("characterName table key {lk:#x} is not ({key} << 32) | {NAME_COLUMN:#x}"));
    }
    let resolves = |x: u32, set: &HashSet<u32>, empty: &[u32]| empty.contains(&x) || set.contains(&x);
    let checks = [
        ("knowledgeInfo", resolves(field(&v, "knowledgeInfo"), &refs.knowledge, &[0])),
        ("convertItemInfo", resolves(field(&v, "convertItemInfo"), &refs.items, &[0])),
        ("npcInfo", resolves(field(&v, "npcInfo"), &refs.npcs, &[0])),
        ("mercenaryInfo", resolves(field(&v, "mercenaryInfo"), &refs.mercenaries, &[0, 0xFF])),
        ("vehicleInfo", resolves(field(&v, "vehicleInfo"), &refs.vehicles, &[0, 0xFFFF])),
    ];
    if let Some((what, _)) = checks.iter().find(|(_, ok)| !ok) {
        return Err(format!("{what} does not resolve"));
    }
    let (_, o3) = lstr(rec, o2).ok_or("no characterDesc")?;
    let (_, o4) = pstr(rec, o3).ok_or("no characterEditName")?;
    // The three u64 after the edit name must be there.
    let rest = rec.get(o4 + 24..).ok_or("the three u64 after characterEditName run past the record end")?;
    Ok(Character {
        key,
        name,
        convert_item: field(&v, "convertItemInfo"),
        tribe: field(&v, "tribeInfoWrapper"),
        interactions: interaction_list(rest, &refs.interactions),
        enable_friendly,
    })
}

/// The interaction list, located by shape; see [`character`].
fn interaction_list(rest: &[u8], known: &HashSet<u32>) -> Vec<u32> {
    let mut lists: Vec<(usize, Vec<u32>)> = Vec::new();
    for p in 0..rest.len().saturating_sub(4) {
        let Some(m) = u32_at(rest, p) else { break };
        let m = m as usize;
        if !(1..=40).contains(&m) || p + 4 + 4 * m > rest.len() {
            continue;
        }
        let ks: Option<Vec<u32>> = (0..m).map(|i| u32_at(rest, p + 4 + 4 * i)).collect();
        if let Some(ks) = ks.filter(|ks| ks.iter().all(|x| known.contains(x))) {
            lists.push((p, ks));
        }
    }
    for w in lists.windows(2) {
        let ((p1, k1), (p2, k2)) = (&w[0], &w[1]);
        if k1 == k2 && *p2 == p1 + 4 + 4 * k1.len() {
            return k1.clone();
        }
    }
    lists.into_iter().find(|(_, k)| k.iter().any(|&x| x >= 1000)).map(|(_, k)| k).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }

    #[test]
    fn record_start_checks_the_key() {
        let mut r = le(3).to_vec();
        r.extend_from_slice(b"abc");
        r.extend_from_slice(&le(77));
        assert_eq!(record_start(&r, 77), Some(("abc".to_string(), 11)));
        assert_eq!(record_start(&r, 78), None);
    }

    #[test]
    fn the_repeated_list_wins_over_an_earlier_one() {
        let known: HashSet<u32> = [5, 1000, 1001].into_iter().collect();
        let mut rest = vec![0xEE; 3];
        // An early single list with a big key, then the repeated pair.
        rest.extend_from_slice(&le(1));
        rest.extend_from_slice(&le(1001));
        for _ in 0..2 {
            rest.extend_from_slice(&le(2));
            rest.extend_from_slice(&le(5));
            rest.extend_from_slice(&le(1000));
        }
        rest.extend_from_slice(&[0xEE; 4]);
        assert_eq!(interaction_list(&rest, &known), vec![5, 1000]);
    }

    #[test]
    fn without_a_repeat_the_first_list_with_a_big_key() {
        let known: HashSet<u32> = [5, 1000].into_iter().collect();
        let mut rest = vec![0xEE; 2];
        rest.extend_from_slice(&le(1));
        rest.extend_from_slice(&le(5));
        rest.extend_from_slice(&[0xEE; 3]);
        rest.extend_from_slice(&le(1));
        rest.extend_from_slice(&le(1000));
        rest.extend_from_slice(&[0xEE; 4]);
        assert_eq!(interaction_list(&rest, &known), vec![1000]);
        assert!(interaction_list(&[0xEE; 3], &known).is_empty());
    }
}
