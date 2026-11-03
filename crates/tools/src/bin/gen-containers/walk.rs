//! `gimmickinfo` from the record start through `tags`, and the head of
//! `interactioninfo`: the two layouts `gen-containers` reads.
//!
//! A port of the first part of `analysis/gimmick_tail.py`'s `walk()` (the
//! record deserialiser `0x14170A4E0`, read in the same stream order through
//! the same sub-readers) and of its `interaction_flags()`. Only what the two
//! container classes need is ported: the interaction overrides (whose
//! condition trees must be walked to get past them) and the tag list. The
//! walk stops after `tags`, so the record-end check `gimmick_tail.checks()`
//! makes is not available here; three checks stand in for it, on every one
//! of the 14210 records of build 26361232:
//!
//! - `gimmickName`, the localised string right after the overrides and the
//!   property list, carries the table key `(record key << 32) | 0x200`, so a
//!   single byte misread anywhere in the overrides fails the record;
//! - every optional-field present byte read on the way is 0 or 1 (the Python
//!   walk accepts any non-zero byte; this one refuses);
//! - the byte after `tags`, `triggerVolumeGroupDataList`'s present byte, is 0
//!   or 1 as well.
//!
//! A condition subtype or kind the walk does not know fails the record
//! instead of being misread, as in the Python.

/// A walk failure, said in words. The Python's `Stop`.
pub type Stop = String;

/// `gimmickName`'s localised-string column.
pub const GIMMICK_NAME_COLUMN: u64 = 0x200;

/// Cursor over one record's bytes; every method reads what the named game
/// reader reads.
pub struct R<'a> {
    d: &'a [u8],
    pub o: usize,
}

impl<'a> R<'a> {
    pub fn new(d: &'a [u8], o: usize) -> Self {
        Self { d, o }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Stop> {
        let end = self.o.checked_add(n).ok_or("offset overflow")?;
        let b = self.d.get(self.o..end).ok_or_else(|| format!("read of {n} bytes at {} past the record end", self.o))?;
        self.o = end;
        Ok(b)
    }

    pub fn u8(&mut self) -> Result<u8, Stop> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16, Stop> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().map_err(|_| "u16")?))
    }

    pub fn u32(&mut self) -> Result<u32, Stop> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| "u32")?))
    }

    pub fn u64(&mut self) -> Result<u64, Stop> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| "u64")?))
    }

    pub fn skip(&mut self, n: usize) -> Result<(), Stop> {
        self.take(n).map(|_| ())
    }

    /// `0x1601490` (and `0x1601780`, which hashes it): `u32` length, bytes.
    pub fn s(&mut self) -> Result<&'a [u8], Stop> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    /// `0x149F270`, a localised string: `u8 kind`, `u64 (recordKey << 32 |
    /// column)`, then a string (the key in decimal). Returns the table key.
    pub fn ls(&mut self) -> Result<u64, Stop> {
        self.skip(1)?;
        let k = self.u64()?;
        self.s()?;
        Ok(k)
    }

    /// The `u8` present byte an optional-pointer reader puts first. Strict:
    /// anything but 0 or 1 is a misread.
    pub fn opt(&mut self) -> Result<bool, Stop> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            b => Err(format!("present byte {b} at {}", self.o - 1)),
        }
    }

    /// A list count, held to the Python's plausibility bound.
    pub fn n(&mut self) -> Result<usize, Stop> {
        let v = self.u32()?;
        if v > 100_000 {
            return Err(format!("implausible count {v}"));
        }
        Ok(v as usize)
    }
}

/// One field a condition leaf's `+0x80` reader takes.
#[derive(Clone, Copy)]
enum F {
    U8,
    U16,
    U32,
    I64,
    S,
    /// `0x141735960`: 16 single bytes.
    B16,
}

/// Condition leaf subtype -> (`+0x80` reader's fields, whether `+0x98` is
/// the shared `0x142412E20` reader). `gimmick_tail.py` `COND`, which names
/// each subtype's RTTI class and reader address. `0xE2` has a reader of its
/// own ([`cond_node`]).
fn cond_fields(st: u16) -> Option<(&'static [F], bool)> {
    use F::*;
    Some(match st {
        0x002 => (&[], false),            // CheckNone
        0x01F => (&[B16, U8], true),      // CheckActionAttribute
        0x024 => (&[U32, U32, U8], true), // CheckKnowledge
        0x037 => (&[U32, U32, U32], true), // CheckEquipType
        0x066 => (&[U32], true),          // CheckGimmickState
        0x075 => (&[S], true),            // CheckCatcheeGimmickTag
        0x082 => (&[U32], false),         // SpecialModeKey
        0x09E => (&[U32, U8, U8, U8], true), // CheckFriendlyLevel
        0x0BC => (&[U32, U32, U8], true), // GetGimmickVariable
        0x0D9 => (&[U8], true),           // IsDockingChild
        0x0E0 => (&[U8], true),           // IsFullGimmickAttachment
        0x0ED => (&[U32], true),          // CheckSpawnReason
        0x104 => (&[], true),             // IsCrimeTarget
        0x107 => (&[U32], true),          // CheckBuffTag
        0x10E => (&[U32], false),         // Macro
        0x14D => (&[U32, U32, U8, U8], true), // DockingChildTagCount
        0x16B => (&[], true),             // CheckHireMercenary
        0x027 => (&[U32, I64, U16, U16], true), // CheckHaveItem
        0x02F => (&[], true),             // CheckBattle
        0x048 => (&[U32], true),          // CompleteMission
        0x08F => (&[U8], true),           // CheckTargetToPushInventory
        0x0B0 => (&[U32, U8], true),      // CheckAttachedGimmickCount
        0x0E2 => (&[], true),             // CheckGimmickTargetCount (own reader)
        0x14E => (&[], true),             // HasBagDocking
        0x159 => (&[U8], true),           // CheckFlip
        0x15F => (&[U32], true),          // IsGimmickPlacedByHousing
        0x17A => (&[U8], true),           // CheckTargetDropListToPushInventory
        0x1A0 => (&[], true),             // IsMiniGamePlayable
        0x008 => (&[], true),             // IsNightTime
        0x00C => (&[U32], true),          // CheckCharacterKey
        0x01B => (&[], true),             // IsFocusActor
        0x0D4 => (&[U32, U8], true),      // CheckElementalMaterialStateFail
        _ => return None,
    })
}

/// The condition-tree factory `0x142631AF0`: `u8 kind`; 0 and 1 two
/// children, 2 one, 3 a leaf (`u16 subtype`, the subtype's `+0x80` reader,
/// then its `+0x98` reader: `u8 present` + string, 8 bytes, three `u8`).
fn cond_node(r: &mut R, depth: usize) -> Result<(), Stop> {
    if depth > 64 {
        return Err("condition tree deeper than 64".into());
    }
    match r.u8()? {
        0 | 1 => {
            cond_node(r, depth + 1)?;
            cond_node(r, depth + 1)
        }
        2 => cond_node(r, depth + 1),
        3 => {
            let st = r.u16()?;
            let (fields, s98) = cond_fields(st).ok_or_else(|| format!("condition subtype 0x{st:X}"))?;
            if st == 0xE2 {
                // 0x14244EA80: 0x14248EA70 (u8 present + 0x1425D8090: u8,
                // opt_cond, string, string), u8, string, u16.
                if r.opt()? {
                    r.u8()?;
                    opt_cond(r, depth + 1)?;
                    r.s()?;
                    r.s()?;
                }
                r.u8()?;
                r.s()?;
                r.u16()?;
            } else {
                for f in fields {
                    match f {
                        F::U8 => r.skip(1)?,
                        F::U16 => r.skip(2)?,
                        F::U32 => r.skip(4)?,
                        F::I64 => r.skip(8)?,
                        F::S => {
                            r.s()?;
                        }
                        F::B16 => r.skip(16)?,
                    }
                }
            }
            if s98 && r.opt()? {
                r.s()?;
                r.skip(8 + 3)?;
            }
            Ok(())
        }
        k => Err(format!("condition kind {k}")),
    }
}

/// `0x14172C060 -> 0x142490850`: `u8 present`, `u8 present`, a condition
/// tree, then three `u8`.
fn opt_cond(r: &mut R, depth: usize) -> Result<(), Stop> {
    if !r.opt()? || !r.opt()? {
        return Ok(());
    }
    cond_node(r, depth)?;
    r.skip(3)
}

/// `0x141713840`, one interaction override: `u32 interaction`, a localised
/// string, `u32`, a list of `(string, u32)`, a list of (four strings, 36
/// bytes), a condition list (`opt_cond, opt_cond`, 11 bytes), a list of 28-byte
/// elements, a `u32` key list, `u32`, `u32 dropset`, five `u8`. Returns the
/// interaction key.
fn interaction_override(r: &mut R) -> Result<u32, Stop> {
    let interaction = r.u32()?;
    r.ls()?;
    r.u32()?;
    for _ in 0..r.n()? {
        r.s()?;
        r.u32()?;
    }
    for _ in 0..r.n()? {
        for _ in 0..4 {
            r.s()?;
        }
        r.skip(4 + 12 + 8 + 12)?;
    }
    for _ in 0..r.n()? {
        opt_cond(r, 0)?;
        opt_cond(r, 0)?;
        r.skip(1 + 4 + 4 + 1 + 1)?;
    }
    let n = r.n()?;
    r.skip(28 * n)?;
    let n = r.n()?;
    r.skip(4 * n)?;
    r.u32()?;
    r.u32()?;
    r.skip(5)?;
    Ok(interaction)
}

/// What `gen-containers` needs out of a `gimmickinfo` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gimmick {
    /// The interaction key of every present override, in order.
    pub interactions: Vec<u32>,
    pub tags: Vec<String>,
}

/// Walk a `NameFirst` `gimmickinfo` record (`u32 name_len, name, u32 key,
/// ...`) from its key through `tags`: `u32 key`, `u8 isBlocked`,
/// `prefabPath`, the interaction overrides (`u8 present`, `u32 n` x (`u8
/// present` + override)), `propertyList` (`u32` keys), `gimmickName`
/// (localised, checked), `devMemo`, `u64 logoutTimeAfterBreak`,
/// `remoteCatchable` (`u8 present` + 14 bytes), `chartParameters` (`u8
/// present` + `u32 n` x two strings), `tags` (`u8 present` + `u32 n` x
/// string), then the next present byte is checked.
pub fn gimmick(rec: &[u8], key: u32) -> Result<Gimmick, Stop> {
    let name_len = u32::from_le_bytes(rec.get(0..4).ok_or("no name length")?.try_into().map_err(|_| "name length")?);
    let mut r = R::new(rec, 4 + name_len as usize);
    let k = r.u32()?;
    if k != key {
        return Err(format!("body key {k} is not the header's {key}"));
    }
    r.u8()?; // isBlocked
    r.s()?; // prefabPath
    let mut interactions = Vec::new();
    if r.opt()? {
        for _ in 0..r.n()? {
            if r.opt()? {
                interactions.push(interaction_override(&mut r)?);
            }
        }
    }
    let n = r.n()?; // propertyList
    r.skip(4 * n)?;
    let lk = r.ls()?; // gimmickName
    if lk != (u64::from(key) << 32) | GIMMICK_NAME_COLUMN {
        return Err(format!("gimmickName table key {lk:#x} is not ({key} << 32) | {GIMMICK_NAME_COLUMN:#x}"));
    }
    r.s()?; // devMemo
    r.u64()?; // logoutTimeAfterBreak
    if r.opt()? {
        r.skip(12 + 2)?; // remoteCatchable
    }
    if r.opt()? {
        for _ in 0..r.n()? {
            r.s()?;
            r.s()?; // chartParameters
        }
    }
    let mut tags = Vec::new();
    if r.opt()? {
        for _ in 0..r.n()? {
            let t = r.s()?;
            tags.push(String::from_utf8(t.to_vec()).map_err(|_| "a tag is not UTF-8")?);
        }
    }
    // triggerVolumeGroupDataList's present byte: not walked, only checked.
    match r.u8()? {
        0 | 1 => {}
        b => return Err(format!("the byte after tags reads {b}, not a present byte")),
    }
    Ok(Gimmick { interactions, tags })
}

/// `interactioninfo`'s 24 one-byte head fields (`gimmick_tail.py`
/// `INTER_FLAGS`): `checkLootableField` is the sixth.
pub const CHECK_LOOTABLE_FIELD: usize = 5;
pub const INTER_FLAG_BYTES: usize = 24;

/// An `interactioninfo` record's name and `checkLootableField`: `u32
/// name_len, name, u32 key`, the 24 flag bytes, `u32 rewardDropSetInfo`. The
/// flag must be 0 or 1 and the key must repeat the header's.
pub fn interaction(rec: &[u8], key: u32) -> Result<(String, bool), Stop> {
    let mut r = R::new(rec, 0);
    let name = r.s()?;
    if !name.iter().all(|c| (0x20..=0x7E).contains(c)) {
        return Err("the name is not printable ASCII".into());
    }
    let name = String::from_utf8_lossy(name).into_owned();
    if r.u32()? != key {
        return Err("the body key is not the header's".into());
    }
    let flags = r.take(INTER_FLAG_BYTES)?;
    r.u32()?; // rewardDropSetInfo
    match flags.get(CHECK_LOOTABLE_FIELD) {
        Some(0) => Ok((name, false)),
        Some(1) => Ok((name, true)),
        Some(b) => Err(format!("checkLootableField reads {b}, not a bool")),
        None => Err("no checkLootableField".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(v: u32) -> [u8; 4] {
        v.to_le_bytes()
    }

    fn s(out: &mut Vec<u8>, t: &[u8]) {
        out.extend_from_slice(&le(t.len() as u32));
        out.extend_from_slice(t);
    }

    fn ls(out: &mut Vec<u8>, key: u64) {
        out.push(0);
        out.extend_from_slice(&key.to_le_bytes());
        s(out, b"0");
    }

    /// One override naming `interaction`, with one condition whose tree is
    /// `and(CheckGimmickState, not(IsCrimeTarget))`.
    fn override_bytes(interaction: u32) -> Vec<u8> {
        let mut o = le(interaction).to_vec();
        ls(&mut o, 7);
        o.extend_from_slice(&le(0));
        o.extend_from_slice(&le(0)); // (string, u32) list
        o.extend_from_slice(&le(0)); // (4 strings, 36) list
        o.extend_from_slice(&le(1)); // one condition
        o.extend_from_slice(&[1, 1, 1]); // present, present, and
        o.extend_from_slice(&[3]);
        o.extend_from_slice(&0x066u16.to_le_bytes());
        o.extend_from_slice(&le(42));
        o.push(0); // no +0x98
        o.extend_from_slice(&[2, 3]);
        o.extend_from_slice(&0x104u16.to_le_bytes());
        o.push(1); // a +0x98: string, 11 bytes
        s(&mut o, b"x");
        o.extend_from_slice(&[0; 11]);
        o.extend_from_slice(&[0; 3]); // opt_cond tail
        o.push(0); // second opt_cond absent
        o.extend_from_slice(&[0; 11]);
        o.extend_from_slice(&le(0)); // 28-byte list
        o.extend_from_slice(&le(0)); // key list
        o.extend_from_slice(&le(0));
        o.extend_from_slice(&le(0));
        o.extend_from_slice(&[0; 5]);
        o
    }

    fn record(key: u32, interaction: u32, tags: &[&str], name_key: u64) -> Vec<u8> {
        let mut r = Vec::new();
        s(&mut r, b"basket");
        r.extend_from_slice(&le(key));
        r.push(0);
        s(&mut r, b"path");
        r.push(1);
        r.extend_from_slice(&le(1));
        r.push(1);
        r.extend_from_slice(&override_bytes(interaction));
        r.extend_from_slice(&le(0)); // propertyList
        ls(&mut r, name_key);
        s(&mut r, b"");
        r.extend_from_slice(&[0; 8]);
        r.push(0); // remoteCatchable
        r.push(0); // chartParameters
        r.push(1);
        r.extend_from_slice(&le(tags.len() as u32));
        for t in tags {
            s(&mut r, t.as_bytes());
        }
        r.push(0); // triggerVolumeGroupDataList absent
        r
    }

    #[test]
    fn the_walk_reads_overrides_and_tags() {
        let r = record(1005717, 10028, &["catch_itemcontainer", "x"], (1005717u64 << 32) | 0x200);
        let g = gimmick(&r, 1005717).unwrap();
        assert_eq!(g.interactions, vec![10028]);
        assert_eq!(g.tags, vec!["catch_itemcontainer".to_string(), "x".to_string()]);
    }

    #[test]
    fn a_misaligned_walk_is_refused() {
        // The gimmickName key of another record.
        let r = record(1005717, 10028, &[], (1005718u64 << 32) | 0x200);
        assert!(gimmick(&r, 1005717).is_err());
        // The header's key differs.
        let r = record(1005717, 10028, &[], (1005717u64 << 32) | 0x200);
        assert!(gimmick(&r, 1005718).is_err());
        // Truncated.
        assert!(gimmick(&r[..r.len() - 1], 1005717).is_err());
        // A present byte that is not a bool.
        let mut bad = r.clone();
        let last = bad.len() - 1;
        bad[last] = 2;
        assert!(gimmick(&bad, 1005717).is_err());
    }

    #[test]
    fn an_unknown_condition_subtype_is_refused() {
        let mut r = Vec::new();
        r.extend_from_slice(&[3]);
        r.extend_from_slice(&0x0FFu16.to_le_bytes());
        assert!(cond_node(&mut R::new(&r, 0), 0).is_err());
        assert!(cond_node(&mut R::new(&[9], 0), 0).is_err());
    }

    #[test]
    fn the_interaction_head_reads_the_loot_flag() {
        let mut r = Vec::new();
        s(&mut r, b"Gimmick_PickUp");
        r.extend_from_slice(&le(5));
        let mut flags = [0u8; INTER_FLAG_BYTES];
        flags[CHECK_LOOTABLE_FIELD] = 1;
        r.extend_from_slice(&flags);
        r.extend_from_slice(&le(0));
        assert_eq!(interaction(&r, 5), Ok(("Gimmick_PickUp".to_string(), true)));
        assert!(interaction(&r, 6).is_err());
        let mut bad = r.clone();
        bad[4 + 14 + 4 + CHECK_LOOTABLE_FIELD] = 2;
        assert!(interaction(&bad, 5).is_err());
    }
}
