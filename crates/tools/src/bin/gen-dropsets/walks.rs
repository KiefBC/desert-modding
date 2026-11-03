//! The record layouts `gen-dropsets` reads besides `gimmickinfo`, each walked
//! to the record end in the order of the 26361232 deserialiser and failed
//! when the walk does not end on the last byte.
//!
//! Ports of `analysis/reward_tables.py` (`item_uses`, `item_use_lists`,
//! `multichange`, `mission`, `factionnode`) and of `analysis/gimmick_tail.py`
//! `dropsets()`, FINDINGS 2.6, 2.7 and 7.8. Every walk starts at the record's
//! `u32 key`, just past its `u32 name_len, name`, and checks the key against
//! the header's.

use desert_tools::gimmick_walk::{self as gw, Stop, R};

/// A cursor at `rec`'s key, after checking it repeats `key`.
fn at_key(rec: &[u8], key: u32) -> Result<R<'_>, Stop> {
    let len = u32::from_le_bytes(rec.get(0..4).ok_or("no name length")?.try_into().map_err(|_| "name length")?);
    let mut r = R::new(rec, 4 + len as usize);
    let k = r.u32()?;
    if k != key {
        return Err(format!("body key {k} is not the header's {key}"));
    }
    Ok(r)
}

fn end(r: &R, rec: &[u8]) -> Result<(), Stop> {
    if r.at_end() {
        Ok(())
    } else {
        Err(format!("the walk ends at {} of a {}-byte record", r.o, rec.len()))
    }
}

/// One `dropsetinfo` row, walked to its end (FINDINGS 2.7): `u8 isBlocked`,
/// `u8 isEqualPercent`, `u8 dropRollType`, `u16 neeSlotCount`, `u32
/// dropTagNameHash`, `u32 dropRollCount`, `u32 n` x (`u8 present` + drop
/// entry), `i64 needWeight`, `i64 totalDropRate`, `i64 rollDiceSkipRate`, `u32
/// dropConditionString`, string `originalString`.
pub struct Dropset {
    /// The declared entry count, absent entries included: what the parsed
    /// row's `+0x28` holds.
    pub declared: usize,
    /// The present entries, in order.
    pub entries: Vec<gw::DropEntry>,
    /// `totalDropRate`, the parsed `+0x38` the dispatch write path checks the
    /// entry weights against.
    pub total: i64,
}

pub fn dropset(rec: &[u8], key: u32) -> Result<Dropset, Stop> {
    let mut r = at_key(rec, key)?;
    r.skip(3 + 2 + 4 + 4)?;
    let declared = r.n()?;
    let mut entries = Vec::with_capacity(declared);
    for _ in 0..declared {
        if let Some(e) = gw::opt_drop(&mut r)? {
            entries.push(e);
        }
    }
    r.i64()?;
    let total = r.i64()?;
    r.i64()?;
    r.u32()?;
    r.s()?;
    end(&r, rec)?;
    Ok(Dropset { declared, entries, total })
}

/// The `itemuseinfo` kind byte of `ItemUseData_RandomBox` (FINDINGS 2.1, 2.6).
pub const RANDOM_BOX: u8 = 2;

/// An `itemuseinfo` record (`0x14170F0A0`): `u8 isBlocked`, `u8 kind`, then
/// the kind's reader. Only kind 2 (RandomBox, `0x142611210`) is walked, as in
/// the Python: the `ItemUseData` base (four `u8`, `u32`, `u32`, `u8`, `u8`,
/// `u32`, a list of (`u32`, `u8`, `u32`)), `u8 present` + `u32 DropSetKey`,
/// `u8 present` + a drop entry, `u8`, `u32`, `u8`, to the record end.
/// Returns `(kind, dropset)`.
pub fn item_use(rec: &[u8], key: u32) -> Result<(u8, Option<u32>), Stop> {
    let mut r = at_key(rec, key)?;
    r.u8()?;
    let kind = r.u8()?;
    if kind != RANDOM_BOX {
        return Ok((kind, None));
    }
    r.skip(4 + 4 + 4 + 1 + 1 + 4)?;
    r.each(|r| r.skip(4 + 1 + 4))?;
    let dropset = if r.opt()? { Some(r.u32()?) } else { None };
    if r.opt()? {
        gw::opt_drop(&mut r)?;
    }
    r.skip(1 + 4 + 1)?;
    end(&r, rec)?;
    Ok((kind, dropset))
}

/// `iteminfo`'s `_itemUseInfoList` (`+0x100`), FINDINGS 2.6 and
/// `reward_tables.py` `item_use_lists()`: 124 bytes past the key, a `u8` (and
/// four more bytes unless it is `0x12`), 32 bytes, `itemName` (a localised
/// string whose table key must be `(key << 32) | 0x70`, the check that the
/// offsets are right), a localised string, an optional one, two strings, two
/// `u32`, `occupiedEquipSlotDataList`, `itemTagList`, then the list.
pub fn item_use_list(rec: &[u8], key: u32) -> Result<Vec<u32>, Stop> {
    let mut r = at_key(rec, key)?;
    r.o = r.o - 4 + 124;
    if r.u8()? != 0x12 {
        r.skip(4)?;
    }
    r.skip(32)?;
    let nk = r.ls()?;
    if nk != (u64::from(key) << 32) | 0x70 {
        return Err(format!("itemName table key {nk:#x} is not ({key} << 32) | 0x70"));
    }
    r.ls()?;
    if r.opt()? {
        r.ls()?;
    }
    r.s()?;
    r.s()?;
    r.skip(8)?;
    r.oeach(|r| {
        r.u32()?;
        let n = r.n()?;
        r.skip(n)
    })?;
    r.oeach(|r| r.skip(4))?;
    let mut uses = Vec::new();
    r.oeach(|r| {
        uses.push(r.u32()?);
        Ok(())
    })?;
    Ok(uses)
}

/// `FixedMaterialData` `0x14171C380`: `u32` item, gimmick, character, `i64`
/// count, `i64` coupon count, `u16` enchant level.
const FIXED_MATERIAL: usize = 4 * 3 + 8 * 2 + 2;

/// A `multichangeinfo` record (`0x14171C4F0`), to the end. Returns its
/// `resultDropInfoList` and `additionalDropInfoList` dropsets.
pub fn multichange(rec: &[u8], key: u32) -> Result<Vec<u32>, Stop> {
    let mut r = at_key(rec, key)?;
    r.skip(7 + 4 * 3 + 4 + 4 + 2)?;
    r.s()?; // craftTagName
    r.each(|r| {
        r.u32()?;
        r.ls().map(|_| ())
    })?;
    r.each(|r| r.skip(FIXED_MATERIAL))?;
    r.each(|r| r.skip(2 + 8 + 2))?;
    r.each(|r| r.s().map(|_| ()))?;
    for _ in 0..3 {
        r.ls()?;
    }
    let mut out = r.keys()?;
    out.extend(r.keys()?);
    end(&r, rec)?;
    Ok(out)
}

/// A `missioninfo` record (`0x141712860`), to the end. Returns the dropsets
/// of its `rewardList` and of every `resultDataList` element.
pub fn mission(rec: &[u8], key: u32) -> Result<Vec<u32>, Stop> {
    let mut r = at_key(rec, key)?;
    // 15 u8, repeatCondition, parentMission, two u16, displaySubMission,
    // completeTime (u16), parentQuest, two u32.
    r.skip(15 + 4 + 4 + 2 + 2 + 4 + 2 + 4 + 4 + 4)?;
    r.keys()?; // subMissionList
    r.each(|r| r.skip(4 * 5 + 5))?; // executeStageList
    r.each(|r| r.skip(4 * 4 + 3))?; // branchMissionList
    r.keys()?;
    r.keys()?;
    r.keys()?;
    if r.opt()? {
        // triggerVolumeData 0x142524FB0.
        r.u8()?;
        r.skip(40)?;
        r.s()?;
        r.s()?;
        r.skip(1 + 24 + 2)?;
    }
    r.each(|r| r.skip(8))?; // knowledgeDescList
    let mut out = r.keys()?; // +0xB0 rewardList
    // +0xC0 resultDataList (MissionResultData 0x141712360): u8, tags, u32
    // dropSetInfo, seven u32, three u8.
    r.each(|r| {
        r.u8()?;
        r.keys()?;
        out.push(r.u32()?);
        r.skip(4 * 7 + 3)
    })?;
    // +0xD0 uiDesc.
    r.skip(24)?;
    r.keys()?;
    r.keys()?;
    r.skip(8 + 12 + 4 + 6 + 2)?;
    for _ in 0..4 {
        r.ls()?;
    }
    // +0x1A0 missionFunctionList.
    r.each(|r| {
        r.u8()?;
        r.s()?;
        r.s()?;
        r.skip(4 + 4 + 2 + 1)?;
        r.s()?;
        r.keys()?;
        r.keys()?;
        r.keys()?;
        r.skip(8 + 1)
    })?;
    // +0x1B0 challengeEventList.
    r.each(|r| {
        r.skip(1 + 4 * 3)?;
        r.keys()?;
        r.skip(4 + 2 + 4 * 3 + 1 + 4 * 3)?;
        r.s()?;
        r.skip(4 + 1 + 4 + 1)
    })?;
    r.keys()?; // sameNameSubMissionList
    end(&r, rec)?;
    Ok(out)
}

/// `0x1417069E0`: list `u8`, `u32`, `u32`, `u8`.
fn active_state(r: &mut R) -> Result<(), Stop> {
    let n = r.n()?;
    r.skip(n + 4 + 4 + 1)
}

/// `0x1416F6430`: `u32`, `u8`, `u32`, `u32`.
const COND_OPTION: usize = 13;

/// What a `factionnode` record pays from: the dispatch operations' two reward
/// sets, the research rewards, and `religionBlockCostList` (a cost, but a
/// dropset row all the same).
#[derive(Default)]
pub struct FactionNode {
    pub dispatch: Vec<u32>,
    pub research: Vec<u32>,
    pub block_cost: Vec<u32>,
}

/// A `factionnode` record (`FactionNodeInfo` `0x141707990`), to the end.
pub fn factionnode(rec: &[u8], key: u32) -> Result<FactionNode, Stop> {
    let mut r = at_key(rec, key)?;
    let mut out = FactionNode::default();
    // 5 u8, knockDownCondition, bitMapColorKey (2), four u32, two u16, u32,
    // three u32, worldPosition (12), observeData (28).
    r.skip(5 + 4 + 2 + 16 + 4 + 4 + 12 + 12 + 28)?;
    r.s()?; // memo
    r.keys()?;
    r.keys()?;
    r.each(|r| {
        r.skip(4 + 8)?;
        active_state(r)
    })?; // applySkillDataList
    r.each(|r| {
        r.skip(4 + 4 + 8 + 8)?;
        active_state(r)
    })?; // resourceItemList
    r.keys()?; // revivalStageInfoList
    r.each(|r| {
        r.skip(8)?;
        let n = r.n()?;
        r.skip(12 * n)
    })?; // wayPointDataList
    // +0xC0 factionScheduleInfoList (FactionScheduleInfo 0x141706D00).
    r.each(|r| {
        r.u8()?;
        r.each(|r| {
            r.skip(1 + 4 + 4 + 12 + 4 + 1)?;
            let n = r.n()?;
            r.skip(8 * n)
        })?;
        active_state(r)?;
        r.each(|r| r.skip(12))?; // deliveringItemList
        r.keys()?;
        r.skip(12 + 2 + 4)?;
        r.ls()?; // operationName
        r.skip(2)?; // operationGroup
        r.each(|r| r.skip(FIXED_MATERIAL))?;
        out.dispatch.push(r.u32()?); // rewardDropSetInfo
        out.dispatch.push(r.u32()?); // rewardDropMoney
        r.skip(4 + 4 + 16)?;
        r.s()?;
        r.skip(4 + 4 + 8 + 12)?;
        r.each(|r| r.skip(COND_OPTION))?;
        r.each(|r| r.skip(12))?;
        r.skip(3)?;
        for _ in 0..4 {
            r.s()?;
        }
        Ok(())
    })?;
    // +0xD0 researchDataList: u32 key, then 0x141707560.
    r.each(|r| {
        r.skip(4 + 4)?;
        r.ls()?;
        r.ls()?;
        r.u32()?;
        out.research.push(r.u32()?);
        r.u32()?;
        r.each(|r| r.skip(COND_OPTION))?;
        r.each(|r| r.skip(8))?;
        r.each(|r| r.skip(4 + 24))?;
        r.skip(4)
    })?;
    // +0xE0 .. +0x1A0: 13 event lists.
    for _ in 0..13 {
        r.each(|r| {
            r.skip(1 + 4 * 7 + 2)?;
            gw::member(r)
        })?;
    }
    out.block_cost = r.keys()?; // +0x1B0 religionBlockCostList
    let n = r.n()?;
    r.skip(2 * n)?; // religionEffectRegionInfoList
    r.each(|r| {
        r.skip(4 * 3 + 3 + 4 * 2)?;
        r.keys()?;
        r.skip(8)
    })?; // factionBlockadingDataList
    r.each(|r| r.skip(4 + 4 + 1))?; // factionConquerDataList
    end(&r, rec)?;
    Ok(out)
}
