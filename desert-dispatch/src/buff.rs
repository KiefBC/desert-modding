//! Pure logic for the **buff/stat census**: the offsets of the parsed
//! `buffinfo` and `statusinfo` records, the seven `BuffData` subclasses worth
//! looking at, the decoders that turn a block of record bytes into those
//! fields, the one-line rendering of each finding, and the accumulator whose
//! `verdict()` says whether the layout still looks right.
//!
//! Nothing here touches the game: it is a byte-offset table plus arithmetic
//! over bytes somebody else read, so it compiles and is unit tested natively on
//! Linux. `census.rs` is the guarded reads and the log calls, and is the only
//! half that needs a live process - the same split `node.rs` and `scan.rs`
//! already have, and for the same reason: the judgement about whether an offset
//! is right must be testable on this machine.
//!
//! **The census is read-only and structurally so.** There is no `safe::write`,
//! no hook, no patch and no call into a game function anywhere in this module
//! or in `census.rs`; a wrong offset here prints a wrong number and nothing
//! else.
//!
//! # Where the offsets come from
//!
//! Every offset below was read off the game's own deserializers on build
//! **25246367**, which name each field in a Korean per-field error message as
//! they read it (`docs/reference-internals.md` section 19.9 is the method):
//!
//! | what | function |
//! | --- | --- |
//! | `BuffInfo` record | `FUN_141468e30` |
//! | `StatusInfo` record | `FUN_141482d40` |
//! | `BuffData` factory (the base fields, and the null entry) | `FUN_141e7e4c0` |
//! | the by-kind constructor switch (kind byte `<= 0x7A`, 122 kinds) | `FUN_141e7a1a0` |
//!
//! The subclass fields come from each class's own vtable slot 10 deserializer,
//! cited where they are declared.
//!
//! # Why the class is read from RTTI and not from a field
//!
//! The kind byte that `FUN_141e7a1a0` switches on comes off the **stream**, and
//! nothing says it is kept in the object it constructs. So the only reliable
//! way to know what a `buffDataList` entry points at is the object's own
//! vtable, which is what [`class_from_mangled`] is the tail of. [`Interest`]
//! carries the kind byte anyway, because a kind that stops agreeing with the
//! class its RTTI names is worth seeing in the log.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// Offsets inside the objects the game parsed `buffinfo` and `statusinfo` into.
///
/// They are **parsed-object** offsets - the objects the record loader produced -
/// not offsets into the raw table bytes, the same distinction
/// [`crate::node::parsed`] and `desert_gatherer::hook`'s own `mod parsed` make.
pub mod parsed {
    /// One parsed `buffinfo` record: the head the census reads as one block.
    ///
    /// Every field is CONFIRMED from the deserializer `FUN_141468e30`, which
    /// names each one in its own error message as it reads it.
    pub mod buff {
        /// Bytes of the head read as one block. Everything named below is
        /// inside it, so one guarded read answers the whole record.
        pub const RAW: usize = 0x48;

        /// `u32 _key` at `record+0x00`: the BuffInfoKey.
        pub const KEY: usize = 0x00;
        /// `ptr _stringKey` at `record+0x08`, pointing at a string object
        /// ([`super::string`]). This is the buff's name in the log.
        pub const STRING_KEY: usize = 0x08;
        /// `u8 _isBlocked` at `record+0x10`.
        pub const IS_BLOCKED: usize = 0x10;
        /// `ptr _buffDataList` at `record+0x18`: the entry array, one
        /// [`super::entry::STRIDE`] per entry.
        pub const LIST: usize = 0x18;
        /// `u32` entry count at `record+0x20`.
        pub const COUNT: usize = 0x20;
        /// `u32` entry capacity at `record+0x24`. Read and printed nowhere, but
        /// named here because it is what tells a misread count from a real one.
        pub const CAPACITY: usize = 0x24;
        /// `u32 _minLevel` at `record+0x28`.
        pub const MIN_LEVEL: usize = 0x28;
        /// `u32 _maxLevel` at `record+0x2C`.
        pub const MAX_LEVEL: usize = 0x2C;
        /// `ptr _sequencerFileName` at `record+0x30`, a string object.
        pub const SEQUENCER: usize = 0x30;
        /// `u8 _buffLevelCalculateType` at `record+0x38`.
        pub const LEVEL_CALC_TYPE: usize = 0x38;
        /// `u16 _uiTemplateName` at `record+0x3A`.
        pub const UI_TEMPLATE: usize = 0x3A;
        /// `u16 _uiComponentName` at `record+0x3C`.
        pub const UI_COMPONENT: usize = 0x3C;
        /// `u16 _elementalStatusInfo` at `record+0x3E`: a `statusinfo` row
        /// index, `0xFFFF` for none.
        pub const ELEMENTAL_STATUS: usize = 0x3E;
        /// `u8 _isUseSkillInfoPatternDescription` at `record+0x40`.
        pub const USE_SKILL_DESC: usize = 0x40;
        /// `u8 _useCountingByGlobalTimer` at `record+0x41`.
        pub const USE_COUNTING: usize = 0x41;
    }

    /// One entry of a record's `_buffDataList`.
    pub mod entry {
        /// Bytes per entry.
        pub const STRIDE: usize = 0x10;
        /// A `u32` the factory `FUN_141e7e4c0` reads from the stream just
        /// **before** the BuffData object it then constructs.
        ///
        /// PLAUSIBLE, not confirmed: the buff level this entry applies at.
        /// `BuffInfo` carries a `_minLevel`/`_maxLevel` pair, which is what
        /// makes a per-entry level the obvious reading, but nothing in the
        /// deserializer names it. It is printed as `lvl=` and called
        /// `entry_u32` in the code, so the log stays readable while the name
        /// stays honest.
        pub const U32: usize = 0x00;
        /// Pointer to the entry's BuffData object at `entry+0x08`.
        ///
        /// **Null is a normal reading**: the factory writes null when the
        /// stream's presence byte is 1, so a null entry is the table saying
        /// "no data here" rather than a failed read.
        pub const OBJ: usize = 0x08;
    }

    /// One BuffData object: the base fields every subclass has, and the
    /// subclass fields of the seven classes the census decodes.
    ///
    /// The object is polymorphic, so the census reads [`RAW`] bytes as one
    /// block and decodes whichever tail the RTTI class calls for.
    pub mod data {
        /// Bytes read as one block. `0xC0` covers the base fields and the
        /// longest subclass tail (`VaryStatBuffData` ends at `+0xB9`), and it
        /// is the whole of what the raw dump prints.
        pub const RAW: usize = 0xC0;

        /// The object's vtable pointer, at `+0x00`. The census reads the class
        /// name through it; see [`super::super::class_from_mangled`].
        pub const VTABLE: usize = 0x00;

        // -- the base fields the factory `FUN_141e7e4c0` writes --
        //
        // None of these has a name yet: the factory's error messages cover the
        // fields it *validates*, not every field it writes. They are printed
        // by offset on every interest line (`base=[...]`) precisely because
        // they are unnamed - one capture of them beside a known buff is how
        // the next one gets a name.

        /// Four bytes between the vtable and `+0x0C` that the factory does not
        /// read off the stream. `+0x08` is the **kind byte**, CONFIRMED on
        /// 2026-09-13 over two kinds: every kind-2 object reads `2` there and
        /// every kind-3 object reads `3`, so the constructor stores what the
        /// stream's kind byte selected. `+0x09` reads `2` and `+0x0A` reads
        /// `1` on both; `+0x0B` varies and looks uninitialised. Printed first
        /// in `base=[...]` as `8= 9= a= b=`.
        pub const B08: usize = 0x08;
        pub const B09: usize = 0x09;
        pub const B0A: usize = 0x0A;
        pub const B0B: usize = 0x0B;
        /// `u32` at `+0x0C`.
        pub const B0C: usize = 0x0C;
        /// `u32` at `+0x10`.
        pub const B10: usize = 0x10;
        /// `u8` at `+0x14`.
        pub const B14: usize = 0x14;
        /// `u8` at `+0x15`.
        pub const B15: usize = 0x15;
        /// `i64` at `+0x18`.
        pub const B18: usize = 0x18;
        /// `i64` at `+0x20`.
        pub const B20: usize = 0x20;
        /// `i64` at `+0x28`.
        pub const B28: usize = 0x28;
        /// `ptr` to a string object at `+0x30`.
        pub const STRING: usize = 0x30;
        /// `u16` row index at `+0x38`, into a table this census has not
        /// identified. `0xFFFF` = none.
        pub const B38: usize = 0x38;
        /// `u8` at `+0x3A`.
        pub const B3A: usize = 0x3A;
        /// `u16` at `+0x3C`.
        pub const B3C: usize = 0x3C;
        /// `u16` at `+0x3E`.
        pub const B3E: usize = 0x3E;
        /// `u16` at `+0x40`.
        pub const B40: usize = 0x40;
        /// `u16` at `+0x42`.
        pub const B42: usize = 0x42;
        /// `u8` at `+0x44`.
        pub const B44: usize = 0x44;
        /// `u8` at `+0x45`.
        pub const B45: usize = 0x45;
        /// `u32` at `+0x48`.
        pub const B48: usize = 0x48;
        /// `u32` at `+0x4C`.
        pub const B4C: usize = 0x4C;
        /// `u32` at `+0x50`.
        pub const B50: usize = 0x50;
        /// `u32` at `+0x54`.
        pub const B54: usize = 0x54;
        /// `u16` at `+0x58`.
        pub const B58: usize = 0x58;
        /// `u16` at `+0x5A`.
        pub const B5A: usize = 0x5A;
        /// A 16-byte list (ptr/count/capacity) at `+0x60`.
        pub const LIST60: usize = 0x60;
        /// A second 16-byte list at `+0x70`.
        pub const LIST70: usize = 0x70;
        /// `u32` at `+0x80`.
        pub const B80: usize = 0x80;
        /// `u8` at `+0x84`.
        pub const B84: usize = 0x84;
        /// `u32` at `+0x88`.
        pub const B88: usize = 0x88;

        /// Where the subclass fields start. Every one of the seven classes
        /// below writes its own fields from here on, which is why the base
        /// block stops at `+0x88` and the raw dump keeps going to [`RAW`].
        pub const SUB: usize = 0x90;

        /// `VaryCollectDropRateBuffData` (kind 2), `FUN_141e80350`: `i32` key
        /// at `+0x90`.
        pub const COLLECT_KEY: usize = 0x90;
        /// `u32` between the key and the value. Not read by the slot-10
        /// deserializer, and the second launch showed why it looked like data
        /// on the first: the same entries read different values across two
        /// launches and 11 of 54 read zero, so it is **uninitialised padding**
        /// (CONFIRMED 2026-09-13). Still printed as `u94=` so a build where it
        /// starts holding something is visible.
        pub const COLLECT_U94: usize = 0x94;
        /// The same class's `i64` value at `+0x98`.
        pub const COLLECT_VALUE: usize = 0x98;

        /// Kinds 3, 4 and 7 (`FUN_141e80da0`, `FUN_141e80fe0`, `FUN_141e82920`)
        /// all start with a `statusinfo` row index read by `FUN_141491bc0`, the
        /// same reader kind 5 uses; then an `i64` at `+0x98`; then kind 4 reads
        /// one byte and kind 7 one more `i64` at `+0xA0`.
        pub const STATIC_STAT_ROW: usize = 0x90;
        pub const STATIC_STAT_V98: usize = 0x98;
        pub const STATIC_STAT_LEVEL_A0: usize = 0xA0;
        pub const STATIC_STAT_RATE_A0: usize = 0xA0;

        /// `VaryStatBuffData` (kind 5), `FUN_141e81580`: `u16` **statusinfo row
        /// index** at `+0x90`, `0xFFFF` for none.
        ///
        /// It is a row index and not a stat enum: the reader `FUN_141491bc0`
        /// looks this value up in the `statusinfo` manager. That is what lets
        /// the census say which of these entries touches the money drop rate -
        /// it compares this against the row the name `AddMoneyDropRate`
        /// resolved to, rather than against a guessed constant.
        pub const VARY_STAT_ROW: usize = 0x90;
        /// `i64` at `+0x98`.
        pub const VARY_STAT_V98: usize = 0x98;
        /// `i64` at `+0xA0`.
        pub const VARY_STAT_A0: usize = 0xA0;
        /// `i64` at `+0xA8`.
        pub const VARY_STAT_A8: usize = 0xA8;
        /// `u32` at `+0xB0`.
        pub const VARY_STAT_B0: usize = 0xB0;
        /// `u8` at `+0xB8`.
        pub const VARY_STAT_B8: usize = 0xB8;

        /// `VaryStatRateBuffData` (kind 8), `FUN_141e82ab0`: `u8` at `+0x90`.
        ///
        /// **PLAUSIBLE** that this is a position in the game's own 19-name stat
        /// table rather than a `statusinfo` row index: it is a byte, and the
        /// row indices are words. The census prints it raw and histograms it
        /// for exactly that reason - one capture of the histogram settles it.
        pub const VARY_STAT_RATE_B90: usize = 0x90;
        /// `u8` at `+0x91`.
        pub const VARY_STAT_RATE_B91: usize = 0x91;
        /// `i64` at `+0x98`.
        pub const VARY_STAT_RATE_V98: usize = 0x98;

        /// `LootBuffData` (kind 11), `FUN_141e83040`: `u8` at `+0x90`, its only
        /// field.
        pub const LOOT_B90: usize = 0x90;

        /// `RegisterItemSellPriceRateBuffData` (kind 99) and
        /// `RegisterFactionOperationRewardRateBuffData` (kind 101), both
        /// `FUN_141e862c0`: `i64` rate at `+0x90`.
        pub const RATE: usize = 0x90;

        /// `RegisterCrimePriceRateBuffData` (kind 100) has **no resolved
        /// reader**: its vtable slot 10 tail-jumps into a region Ghidra has not
        /// analysed, so no field of it is known. Its line prints
        /// `+0x90..+0xC0` as hex instead of naming anything, which is the
        /// honest rendering and is also what lets the layout be read offline
        /// later.
        pub const CRIME_RAW: usize = 0x90;
        /// How many bytes of that tail are printed.
        pub const CRIME_RAW_LEN: usize = RAW - CRIME_RAW;
    }

    /// The string object a `ptr string` field points at.
    ///
    /// Kept here rather than in `census.rs` for the same reason as every other
    /// offset: it is a layout claim, and a layout claim belongs somewhere a
    /// native test can reach it.
    pub mod string {
        /// `char* chars` at `+0x00`.
        pub const CHARS: usize = 0x00;
        /// `i32 len` at `+0x08`.
        pub const LEN: usize = 0x08;
        /// `u32 hash` at `+0x0C`.
        pub const HASH: usize = 0x0C;
        /// `i32 refcount` at `+0x10`.
        pub const REFCOUNT: usize = 0x10;
        /// `u8 hashed` at `+0x14`.
        pub const HASHED: usize = 0x14;

        /// An empty string points at a **static sentinel object** shared by
        /// every empty string in the game, so a `len` of 0 is `""` and not a
        /// failed read.
        pub const EMPTY_LEN: i32 = 0;
    }

    /// One parsed `statusinfo` record, all of it CONFIRMED from the
    /// deserializer `FUN_141482d40`.
    pub mod status {
        /// Bytes of the record read as one block.
        pub const RAW: usize = 0xB8;

        /// `u32 _key` at `+0x00`.
        pub const KEY: usize = 0x00;
        /// `ptr _stringKey` at `+0x08`: the stat's name, and the field the
        /// whole census turns on - the game resolves `AddMoneyDropRate` and 18
        /// other names to a **record** by name hash at startup
        /// (`FUN_14250b680`), so the names are record names and this is where
        /// they live.
        pub const STRING_KEY: usize = 0x08;
        /// `u8 _isBlocked` at `+0x10`.
        pub const IS_BLOCKED: usize = 0x10;
        /// `u8 _regenerateType` at `+0x11`.
        pub const REGENERATE_TYPE: usize = 0x11;
        /// `u32 _statusIndexXXXXX` at `+0x14`.
        pub const STATUS_INDEX: usize = 0x14;
        /// `u8 _isHardCoded` at `+0x18`.
        pub const IS_HARD_CODED: usize = 0x18;
        /// `u8 _initValueType` at `+0x19`.
        pub const INIT_VALUE_TYPE: usize = 0x19;
        /// `u16 _minResistanceStatusInfo` at `+0x1A`.
        pub const MIN_RESISTANCE: usize = 0x1A;
        /// `u16 _maxResistanceStatusInfo` at `+0x1C`.
        pub const MAX_RESISTANCE: usize = 0x1C;
        /// `u8 _isResistanceStat` at `+0x1E`.
        pub const IS_RESISTANCE_STAT: usize = 0x1E;
        /// `u8 _isElementalStat` at `+0x1F`.
        pub const IS_ELEMENTAL_STAT: usize = 0x1F;
        /// An 8-byte `_blockRegenOnMinStatTick` at `+0x20`.
        pub const BLOCK_REGEN: usize = 0x20;
        /// `u8 _decreaseOnItemBroken` at `+0x28`.
        pub const DECREASE_ON_BROKEN: usize = 0x28;
        /// `u16 _buffInfo` at `+0x2A`: a `buffinfo` row index, `0xFFFF` none.
        /// The one field that joins the two halves of this census.
        pub const BUFF_INFO: usize = 0x2A;
        /// `u32 _actualStatusKeyToRefer` at `+0x2C`.
        pub const REFER: usize = 0x2C;
        /// `u8 _statType` at `+0x30`.
        ///
        /// **Meaning unknown.** The 19 well-known names are record names, not
        /// `_statType` values, so nothing says this byte is that enum - which
        /// is exactly why the census prints a histogram of it over every loaded
        /// row. If it turns out to take 19 values it is a candidate; if it
        /// takes 3 it is something else entirely.
        pub const STAT_TYPE: usize = 0x30;
        /// `u8 _staticStatType` at `+0x31`.
        pub const STATIC_STAT_TYPE: usize = 0x31;
        /// `u8 _elementalStatType` at `+0x32`.
        pub const ELEMENTAL_STAT_TYPE: usize = 0x32;
        /// `u16 _activeKnowledgeInfo` at `+0x34`.
        pub const ACTIVE_KNOWLEDGE: usize = 0x34;
        /// `u32 _sendGimmickEventKeyForStatChanged` at `+0x38`.
        pub const SEND_GIMMICK_EVENT: usize = 0x38;
        /// `ptr _reserveSlotInfoList` at `+0x40`, 4-byte entries.
        pub const SLOTS: usize = 0x40;
        /// `u32` count of those at `+0x48`.
        pub const SLOT_COUNT: usize = 0x48;
        /// `u32` capacity at `+0x4C`.
        pub const SLOT_CAPACITY: usize = 0x4C;
        /// `u8 _useLimitHitMinStat` at `+0x50`.
        pub const USE_LIMIT_MIN: usize = 0x50;
        /// `u8 _useLimitHitMaxStat` at `+0x51`.
        pub const USE_LIMIT_MAX: usize = 0x51;
        /// `u32 _statusKeyHashCode32` at `+0x54`: the hash the name lookup at
        /// startup matches against, so it is worth printing beside the name.
        pub const HASH: usize = 0x54;
        /// `u32 _minHashCode32` at `+0x58`.
        pub const MIN_HASH: usize = 0x58;
        /// `u32 _maxHashCode32` at `+0x5C`.
        pub const MAX_HASH: usize = 0x5C;
        /// `u8 _isFullRecoverWhenRevived` at `+0x60`.
        pub const FULL_RECOVER: usize = 0x60;
        /// `u8 _usePercent` at `+0x61`: whether the stat reads as a percentage,
        /// which is what a drop-*rate* stat would be expected to say.
        pub const USE_PERCENT: usize = 0x61;
        /// `u8 _isRepeatUpdateFromServer` at `+0x62`.
        pub const REPEAT_UPDATE: usize = 0x62;
        /// A 16-byte `_statLevelData` list at `+0x68`.
        pub const STAT_LEVEL_DATA: usize = 0x68;
        /// A 16-byte `_frameEventAttributeListByLevel` list at `+0x78`.
        pub const FRAME_EVENTS: usize = 0x78;
        /// `u8 _isResetOnRevive` at `+0x88`.
        pub const RESET_ON_REVIVE: usize = 0x88;
        /// 4 bytes of `_notEnoughResourceMessage` at `+0x8C`.
        pub const NOT_ENOUGH_MESSAGE: usize = 0x8C;
        /// `u16 _uiTemplateName` at `+0x90`.
        pub const UI_TEMPLATE: usize = 0x90;
        /// `u16 _uiComponentName` at `+0x92`.
        pub const UI_COMPONENT: usize = 0x92;
        /// A 16-byte `_minPassiveSkillList` at `+0x98`.
        pub const MIN_PASSIVE_SKILLS: usize = 0x98;
        /// A 16-byte `_maxPassiveSkillList` at `+0xA8`.
        pub const MAX_PASSIVE_SKILLS: usize = 0xA8;
    }
}

/// The `0xFFFF` every row-index field in both tables uses for "none". The same
/// sentinel [`crate::node::parsed::REWARD_NONE`] is, and printed the same way
/// by [`crate::node::row_text`].
pub const ROW_NONE: u16 = 0xFFFF;

/// The 19 stat names the game itself resolves to `statusinfo` records at
/// startup, **in the order its own name table has them**.
///
/// The order is evidence, not presentation: `FUN_14250b680` walks this list and
/// resolves each name to a record by hash, so a position in it is the only
/// thing a byte-sized stat field could plausibly be an index into. That is what
/// [`MONEY_STAT_POS`] rests on, and it is why this array must not be sorted.
///
/// `AddMoneyDropRate` is the reason the census exists.
pub const WELL_KNOWN_STATS: [&str; 19] = [
    "DDD",
    "DPV",
    "DHIT",
    "DDV",
    "DPVRate",
    "CriticalDamage",
    "CriticalRate",
    "AttackedDamageRate",
    "AttackedDamageReduction",
    "AttackSpeedRate",
    "MoveSpeedRate",
    "ClimbSpeedRate",
    "SwimSpeedRate",
    "EquipDropRate",
    "AddMoneyDropRate",
    "MoveRate",
    "AccRate",
    "RotationRate",
    "JumpRate",
];

/// The stat the drop-rate investigation is about.
pub const MONEY_STAT: &str = "AddMoneyDropRate";
/// The other half of the same question: equipment drops.
pub const EQUIP_DROP_STAT: &str = "EquipDropRate";

/// [`MONEY_STAT`]'s position in [`WELL_KNOWN_STATS`].
///
/// **PLAUSIBLE only** as a value of a stat *field*: it is what a byte-sized
/// index into the game's own name table would read for this stat, and
/// [`parsed::data::VARY_STAT_RATE_B90`] is such a byte. It is used for one
/// thing - putting a `MONEY ` prefix on a `VaryStatRate` line so a human can
/// find it - and never for deciding anything.
pub const MONEY_STAT_POS: u8 = 14;
/// [`EQUIP_DROP_STAT`]'s position in [`WELL_KNOWN_STATS`], with the same
/// caveat.
pub const EQUIP_DROP_STAT_POS: u8 = 13;

/// Most `buffinfo` records the census walks in one pass. A vanilla table is
/// far smaller; the cap is what holds when a `u32` count is misread.
pub const MAX_BUFF_RECORDS: usize = 65_536;
/// Most `_buffDataList` entries read out of one record.
///
/// 64 on the first launch, and six of the 292 vanilla records were capped by
/// it (`ChangeBuffLevelBuffData` alone counts 732 entries across the table),
/// so the census could not say what those six held. Raised to hold any
/// vanilla record whole; the global [`MAX_TOTAL_ENTRIES`] is still the bound
/// that actually holds, and a record over this cap is now named in the log.
pub const MAX_ENTRIES_PER_RECORD: usize = 1_024;
/// Most capped records the summary names; the count is kept either way.
pub const MAX_CAPPED_NAMED: usize = 32;
/// Most entries walked in one pass, whatever the per-record counts say. This
/// is the bound that actually holds: per-record caps multiply out, and one
/// misread count would otherwise wedge this thread for the session with nothing
/// in the log to say why. Same argument as [`crate::node::MAX_TOTAL_OPS`].
pub const MAX_TOTAL_ENTRIES: usize = 262_144;
/// Most `statusinfo` records the census walks in one pass.
pub const MAX_STATUS_RECORDS: usize = 8_192;
/// Most distinct class names the histogram keeps keys for; the rest are counted
/// in one [`OTHER_CLASS`] bucket.
pub const MAX_CLASSES: usize = 512;
/// Most vtables the class-name cache holds **per pass**. A miss past the cap is
/// re-read rather than cached, so the cap costs reads and never correctness.
pub const MAX_CLASS_CACHE: usize = 1_024;
/// Longest string or mangled class name read out of the game.
pub const MAX_NAME: usize = 96;
/// Most distinct [`parsed::data::VARY_STAT_RATE_B90`] values the histogram
/// keeps.
pub const MAX_VSR90_KEYS: usize = 64;

/// Where class names past [`MAX_CLASSES`] are counted.
pub const OTHER_CLASS: &str = "(other)";

/// The share of walked entries that must resolve to an RTTI name ending in
/// `BuffData` before the layout is called right. It is deliberately near 1: the
/// entries are pointers to objects of one class family, so anything else means
/// the entry stride or the list pointer is wrong, not that the table is odd.
const VERDICT_NAMED: f64 = 0.99;

/// The mangled-name prefix MSVC gives a class type descriptor.
const MANGLED_PREFIX: &str = ".?AV";
/// And the suffix every one of this game's classes carries: they all live in
/// the `pa` namespace.
const MANGLED_SUFFIX: &str = "@pa@@";

/// The class name inside an MSVC mangled type-descriptor name, or `None` if it
/// is not one of this game's classes.
///
/// `.?AVVaryStatBuffData@pa@@` -> `VaryStatBuffData`. Both ends are **required**
/// rather than trimmed if present: the census reads this name out of an address
/// it computed from three chained pointer reads, and the two fixed affixes are
/// what make a wrong chain answer `None` instead of a plausible-looking string.
///
/// This is the live counterpart of [`desert_core::rtti`], which documents the
/// same MSVC layout and searches a **file** image for a class by name. The
/// direction here is the opposite - object to name, in the running process -
/// and it stays in this crate while it has one caller.
pub fn class_from_mangled(mangled: &str) -> Option<&str> {
    mangled.strip_prefix(MANGLED_PREFIX)?.strip_suffix(MANGLED_SUFFIX)
}

// ---------------------------------------------------------------------------
// The ten classes worth a line
// ---------------------------------------------------------------------------

/// A `BuffData` subclass the census decodes and logs.
///
/// Ten of the 122 kinds `FUN_141e7a1a0` can construct. They are the ones that
/// could plausibly move a drop rate, a sell price, a crime price or a dispatch
/// reward rate - the question `docs/` opened and this census is the first pass
/// at. Every other class is counted in the histogram and nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Interest {
    /// Kind 2: a gather/collect drop rate, keyed.
    VaryCollectDropRate,
    /// Kind 3: a flat change to one **static** stat, named by `statusinfo` row
    /// (`FUN_141e80da0`). The first launch showed `AddMoneyDropRate` is a
    /// static stat (`_staticStatType` 14), so this family, not [`Self::VaryStat`],
    /// is where a money-drop buff would be.
    VaryStaticStat,
    /// Kind 4: a static stat change by level (`FUN_141e80fe0`): row, value, and
    /// one byte.
    VaryStaticStatLevel,
    /// Kind 5: a flat change to one `statusinfo` stat, named by row.
    VaryStat,
    /// Kind 7: a *rate* change to a static stat (`FUN_141e82920`): row and two
    /// `i64`s.
    VaryStaticStatRate,
    /// Kind 8: a *rate* change to a stat, selected by a byte rather than a row.
    VaryStatRate,
    /// Kind 11: loot.
    Loot,
    /// Kind 99: item sell price rate.
    RegisterItemSellPriceRate,
    /// Kind 100: crime price rate. Its reader is unresolved, so its line is hex.
    RegisterCrimePriceRate,
    /// Kind 101: the dispatch-mission reward rate, which is this subsystem's
    /// own subject arriving from the other direction.
    RegisterFactionOperationRewardRate,
}

impl Interest {
    /// All ten, in declaration order, so every line that names them names
    /// them in the same order.
    pub const ALL: [Interest; 10] = [
        Interest::VaryCollectDropRate,
        Interest::VaryStaticStat,
        Interest::VaryStaticStatLevel,
        Interest::VaryStat,
        Interest::VaryStaticStatRate,
        Interest::VaryStatRate,
        Interest::Loot,
        Interest::RegisterItemSellPriceRate,
        Interest::RegisterCrimePriceRate,
        Interest::RegisterFactionOperationRewardRate,
    ];

    /// The class an RTTI name is one of, or `None`.
    ///
    /// The match is **exact** on `<Label>BuffData`. That matters more than it
    /// looks: `VaryStatMaxValueBuffData` is a different class with a different
    /// layout, and a prefix match would decode its fields at
    /// `VaryStatBuffData`'s offsets and print the result as if it were a stat
    /// change.
    pub fn from_class(name: &str) -> Option<Self> {
        let stem = name.strip_suffix("BuffData")?;
        Interest::ALL.into_iter().find(|i| i.label() == stem)
    }

    /// The kind byte `FUN_141e7a1a0` constructs this class for.
    ///
    /// Carried for the log line only. The class comes from RTTI, never from
    /// this: the kind byte is read off the stream and is not known to be stored
    /// in the object at all.
    pub fn kind(self) -> u8 {
        match self {
            Interest::VaryCollectDropRate => 2,
            Interest::VaryStaticStat => 3,
            Interest::VaryStaticStatLevel => 4,
            Interest::VaryStat => 5,
            Interest::VaryStaticStatRate => 7,
            Interest::VaryStatRate => 8,
            Interest::Loot => 11,
            Interest::RegisterItemSellPriceRate => 99,
            Interest::RegisterCrimePriceRate => 100,
            Interest::RegisterFactionOperationRewardRate => 101,
        }
    }

    /// The class name without its `BuffData` suffix, which is both what the log
    /// prints and what [`Self::from_class`] matches on.
    pub fn label(self) -> &'static str {
        match self {
            Interest::VaryCollectDropRate => "VaryCollectDropRate",
            Interest::VaryStaticStat => "VaryStaticStat",
            Interest::VaryStaticStatLevel => "VaryStaticStatLevel",
            Interest::VaryStat => "VaryStat",
            Interest::VaryStaticStatRate => "VaryStaticStatRate",
            Interest::VaryStatRate => "VaryStatRate",
            Interest::Loot => "Loot",
            Interest::RegisterItemSellPriceRate => "RegisterItemSellPriceRate",
            Interest::RegisterCrimePriceRate => "RegisterCrimePriceRate",
            Interest::RegisterFactionOperationRewardRate => "RegisterFactionOperationRewardRate",
        }
    }
}

// ---------------------------------------------------------------------------
// The decoders
// ---------------------------------------------------------------------------

fn le_u8(b: &[u8], off: usize) -> Option<u8> {
    b.get(off).copied()
}

fn le_u16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off.checked_add(2)?)?.try_into().ok()?))
}

fn le_u32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off.checked_add(4)?)?.try_into().ok()?))
}

fn le_i32(b: &[u8], off: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(off..off.checked_add(4)?)?.try_into().ok()?))
}

fn le_i64(b: &[u8], off: usize) -> Option<i64> {
    Some(i64::from_le_bytes(b.get(off..off.checked_add(8)?)?.try_into().ok()?))
}

fn le_u64(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off.checked_add(8)?)?.try_into().ok()?))
}

/// The head of one parsed `buffinfo` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuffHead {
    pub key: u32,
    /// Address of the record's `_stringKey` string object, not the text: the
    /// text needs a second guarded read and so belongs to `census.rs`.
    pub string_key: u64,
    pub is_blocked: u8,
    /// Address of the `_buffDataList` entry array.
    pub list: u64,
    pub count: u32,
    pub capacity: u32,
    pub min_level: u32,
    pub max_level: u32,
    pub sequencer: u64,
    pub level_calc_type: u8,
    pub ui_template: u16,
    pub ui_component: u16,
    pub elemental_status: u16,
    pub use_skill_desc: u8,
    pub use_counting: u8,
}

/// Decode one `buffinfo` record head out of its [`parsed::buff::RAW`] bytes,
/// or `None` if the block is too short to hold the last named field.
///
/// A short block answers `None` rather than a half-decoded head for the reason
/// every decoder in this workspace does: a partial record is
/// indistinguishable from a wrong offset, and telling those apart is the whole
/// job of a census.
pub fn decode_buff_head(b: &[u8]) -> Option<BuffHead> {
    use parsed::buff as o;
    Some(BuffHead {
        key: le_u32(b, o::KEY)?,
        string_key: le_u64(b, o::STRING_KEY)?,
        is_blocked: le_u8(b, o::IS_BLOCKED)?,
        list: le_u64(b, o::LIST)?,
        count: le_u32(b, o::COUNT)?,
        capacity: le_u32(b, o::CAPACITY)?,
        min_level: le_u32(b, o::MIN_LEVEL)?,
        max_level: le_u32(b, o::MAX_LEVEL)?,
        sequencer: le_u64(b, o::SEQUENCER)?,
        level_calc_type: le_u8(b, o::LEVEL_CALC_TYPE)?,
        ui_template: le_u16(b, o::UI_TEMPLATE)?,
        ui_component: le_u16(b, o::UI_COMPONENT)?,
        elemental_status: le_u16(b, o::ELEMENTAL_STATUS)?,
        use_skill_desc: le_u8(b, o::USE_SKILL_DESC)?,
        use_counting: le_u8(b, o::USE_COUNTING)?,
    })
}

/// `(entry_u32, BuffData object address)` out of one
/// [`parsed::entry::STRIDE`]-byte entry.
///
/// A zero object address is passed straight back: the factory writes null for
/// an absent BuffData, so "null" is a reading the caller counts, not an error
/// this decoder should swallow.
pub fn decode_entry(b: &[u8]) -> Option<(u32, u64)> {
    Some((le_u32(b, parsed::entry::U32)?, le_u64(b, parsed::entry::OBJ)?))
}

/// The base fields every BuffData object carries, as the interest lines print
/// them.
///
/// Only the fields the `base=[...]` group names are kept. The rest of the base
/// block is declared in [`parsed::data`] and reaches the log through the raw
/// hex dump, which is the right home for a field nobody can name yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseFields {
    pub b08: u8,
    pub b09: u8,
    pub b0a: u8,
    pub b0b: u8,
    pub b0c: u32,
    pub b10: u32,
    pub b14: u8,
    pub b15: u8,
    pub b18: i64,
    pub b20: i64,
    pub b28: i64,
    pub b38: u16,
    pub b48: u32,
    pub b4c: u32,
    pub b50: u32,
    pub b54: u32,
    pub b80: u32,
    pub b88: u32,
}

/// Decode the base fields out of a BuffData object's [`parsed::data::RAW`]
/// bytes.
pub fn decode_base(b: &[u8]) -> Option<BaseFields> {
    use parsed::data as o;
    Some(BaseFields {
        b08: le_u8(b, o::B08)?,
        b09: le_u8(b, o::B09)?,
        b0a: le_u8(b, o::B0A)?,
        b0b: le_u8(b, o::B0B)?,
        b0c: le_u32(b, o::B0C)?,
        b10: le_u32(b, o::B10)?,
        b14: le_u8(b, o::B14)?,
        b15: le_u8(b, o::B15)?,
        b18: le_i64(b, o::B18)?,
        b20: le_i64(b, o::B20)?,
        b28: le_i64(b, o::B28)?,
        b38: le_u16(b, o::B38)?,
        b48: le_u32(b, o::B48)?,
        b4c: le_u32(b, o::B4C)?,
        b50: le_u32(b, o::B50)?,
        b54: le_u32(b, o::B54)?,
        b80: le_u32(b, o::B80)?,
        b88: le_u32(b, o::B88)?,
    })
}

/// The subclass fields of one of the ten classes, typed per class.
///
/// One variant per [`Interest`], so a line can only ever print the fields of
/// the class its RTTI actually named: the decode and the render agree by
/// construction rather than by a matching pair of `match` arms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataFields {
    VaryCollectDropRate {
        key: i32,
        u94: u32,
        value: i64,
    },
    /// Kind 3. `row` is a `statusinfo` row index, [`ROW_NONE`] for none.
    VaryStaticStat {
        row: u16,
        v98: i64,
    },
    /// Kind 4.
    VaryStaticStatLevel {
        row: u16,
        v98: i64,
        a0: u8,
    },
    /// Kind 7.
    VaryStaticStatRate {
        row: u16,
        v98: i64,
        a0: i64,
    },
    VaryStat {
        /// `statusinfo` row index, [`ROW_NONE`] for none.
        row: u16,
        v98: i64,
        a0: i64,
        a8: i64,
        b0: u32,
        b8: u8,
    },
    VaryStatRate {
        b90: u8,
        b91: u8,
        v98: i64,
    },
    Loot {
        b90: u8,
    },
    RegisterItemSellPriceRate {
        rate: i64,
    },
    /// The class whose reader is unresolved: its tail is carried as bytes and
    /// printed as hex, because naming a field of it would be a guess.
    RegisterCrimePriceRate {
        raw: [u8; parsed::data::CRIME_RAW_LEN],
    },
    RegisterFactionOperationRewardRate {
        rate: i64,
    },
}

/// Decode the subclass tail of a BuffData object for the class its RTTI named.
///
/// `what` comes from the object's own vtable, never from a field of the block,
/// which is what keeps this from decoding one class's bytes at another's
/// offsets.
pub fn decode_fields(what: Interest, b: &[u8]) -> Option<DataFields> {
    use parsed::data as o;
    Some(match what {
        Interest::VaryCollectDropRate => DataFields::VaryCollectDropRate {
            key: le_i32(b, o::COLLECT_KEY)?,
            u94: le_u32(b, o::COLLECT_U94)?,
            value: le_i64(b, o::COLLECT_VALUE)?,
        },
        Interest::VaryStaticStat => DataFields::VaryStaticStat {
            row: le_u16(b, o::STATIC_STAT_ROW)?,
            v98: le_i64(b, o::STATIC_STAT_V98)?,
        },
        Interest::VaryStaticStatLevel => DataFields::VaryStaticStatLevel {
            row: le_u16(b, o::STATIC_STAT_ROW)?,
            v98: le_i64(b, o::STATIC_STAT_V98)?,
            a0: le_u8(b, o::STATIC_STAT_LEVEL_A0)?,
        },
        Interest::VaryStaticStatRate => DataFields::VaryStaticStatRate {
            row: le_u16(b, o::STATIC_STAT_ROW)?,
            v98: le_i64(b, o::STATIC_STAT_V98)?,
            a0: le_i64(b, o::STATIC_STAT_RATE_A0)?,
        },
        Interest::VaryStat => DataFields::VaryStat {
            row: le_u16(b, o::VARY_STAT_ROW)?,
            v98: le_i64(b, o::VARY_STAT_V98)?,
            a0: le_i64(b, o::VARY_STAT_A0)?,
            a8: le_i64(b, o::VARY_STAT_A8)?,
            b0: le_u32(b, o::VARY_STAT_B0)?,
            b8: le_u8(b, o::VARY_STAT_B8)?,
        },
        Interest::VaryStatRate => DataFields::VaryStatRate {
            b90: le_u8(b, o::VARY_STAT_RATE_B90)?,
            b91: le_u8(b, o::VARY_STAT_RATE_B91)?,
            v98: le_i64(b, o::VARY_STAT_RATE_V98)?,
        },
        Interest::Loot => DataFields::Loot { b90: le_u8(b, o::LOOT_B90)? },
        Interest::RegisterItemSellPriceRate => {
            DataFields::RegisterItemSellPriceRate { rate: le_i64(b, o::RATE)? }
        }
        Interest::RegisterCrimePriceRate => DataFields::RegisterCrimePriceRate {
            raw: b
                .get(o::CRIME_RAW..o::CRIME_RAW.checked_add(o::CRIME_RAW_LEN)?)?
                .try_into()
                .ok()?,
        },
        Interest::RegisterFactionOperationRewardRate => {
            DataFields::RegisterFactionOperationRewardRate { rate: le_i64(b, o::RATE)? }
        }
    })
}

/// Every named field of one parsed `statusinfo` record.
///
/// All of it, rather than the handful the line prints: the offsets are the
/// claim under test, and a decoder that reads every one of them is what makes
/// the native test able to fail when a constant moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusFields {
    pub key: u32,
    pub string_key: u64,
    pub is_blocked: u8,
    pub regenerate_type: u8,
    pub status_index: u32,
    pub is_hard_coded: u8,
    pub init_value_type: u8,
    pub min_resistance: u16,
    pub max_resistance: u16,
    pub is_resistance_stat: u8,
    pub is_elemental_stat: u8,
    pub decrease_on_broken: u8,
    /// `buffinfo` row index, [`ROW_NONE`] for none: the one field that joins
    /// the two halves of the census.
    pub buff_info: u16,
    pub refer: u32,
    pub stat_type: u8,
    pub static_stat_type: u8,
    pub elemental_stat_type: u8,
    pub active_knowledge: u16,
    pub send_gimmick_event: u32,
    pub slots: u64,
    pub slot_count: u32,
    pub slot_capacity: u32,
    pub use_limit_min: u8,
    pub use_limit_max: u8,
    pub hash: u32,
    pub min_hash: u32,
    pub max_hash: u32,
    pub full_recover: u8,
    pub use_percent: u8,
    pub repeat_update: u8,
    pub reset_on_revive: u8,
    pub ui_template: u16,
    pub ui_component: u16,
}

/// Decode one `statusinfo` record out of its [`parsed::status::RAW`] bytes.
pub fn decode_status(b: &[u8]) -> Option<StatusFields> {
    use parsed::status as o;
    Some(StatusFields {
        key: le_u32(b, o::KEY)?,
        string_key: le_u64(b, o::STRING_KEY)?,
        is_blocked: le_u8(b, o::IS_BLOCKED)?,
        regenerate_type: le_u8(b, o::REGENERATE_TYPE)?,
        status_index: le_u32(b, o::STATUS_INDEX)?,
        is_hard_coded: le_u8(b, o::IS_HARD_CODED)?,
        init_value_type: le_u8(b, o::INIT_VALUE_TYPE)?,
        min_resistance: le_u16(b, o::MIN_RESISTANCE)?,
        max_resistance: le_u16(b, o::MAX_RESISTANCE)?,
        is_resistance_stat: le_u8(b, o::IS_RESISTANCE_STAT)?,
        is_elemental_stat: le_u8(b, o::IS_ELEMENTAL_STAT)?,
        decrease_on_broken: le_u8(b, o::DECREASE_ON_BROKEN)?,
        buff_info: le_u16(b, o::BUFF_INFO)?,
        refer: le_u32(b, o::REFER)?,
        stat_type: le_u8(b, o::STAT_TYPE)?,
        static_stat_type: le_u8(b, o::STATIC_STAT_TYPE)?,
        elemental_stat_type: le_u8(b, o::ELEMENTAL_STAT_TYPE)?,
        active_knowledge: le_u16(b, o::ACTIVE_KNOWLEDGE)?,
        send_gimmick_event: le_u32(b, o::SEND_GIMMICK_EVENT)?,
        slots: le_u64(b, o::SLOTS)?,
        slot_count: le_u32(b, o::SLOT_COUNT)?,
        slot_capacity: le_u32(b, o::SLOT_CAPACITY)?,
        use_limit_min: le_u8(b, o::USE_LIMIT_MIN)?,
        use_limit_max: le_u8(b, o::USE_LIMIT_MAX)?,
        hash: le_u32(b, o::HASH)?,
        min_hash: le_u32(b, o::MIN_HASH)?,
        max_hash: le_u32(b, o::MAX_HASH)?,
        full_recover: le_u8(b, o::FULL_RECOVER)?,
        use_percent: le_u8(b, o::USE_PERCENT)?,
        repeat_update: le_u8(b, o::REPEAT_UPDATE)?,
        reset_on_revive: le_u8(b, o::RESET_ON_REVIVE)?,
        ui_template: le_u16(b, o::UI_TEMPLATE)?,
        ui_component: le_u16(b, o::UI_COMPONENT)?,
    })
}

// ---------------------------------------------------------------------------
// The log lines
// ---------------------------------------------------------------------------

/// What the two stat rows the investigation is about resolved to, as far as the
/// status half of the census got.
///
/// `None` means the status half has not found that row yet - the tables load
/// lazily and independently - which is why a `MONEY ` prefix can appear on a
/// later pass and not an earlier one over the same entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Refs<'a> {
    /// The `statusinfo` row [`MONEY_STAT`] resolved to.
    pub money_row: Option<u16>,
    /// The row [`EQUIP_DROP_STAT`] resolved to.
    pub equip_row: Option<u16>,
    /// The name of the row a [`DataFields::VaryStat`] entry points at, when the
    /// census knows one.
    pub stat_name: Option<&'a str>,
}

/// The prefix an interest line carries so the two entries that matter can be
/// found with one `grep`.
///
/// `MONEY ` and `EQUIP ` only, and only on the two stat classes. For
/// [`DataFields::VaryStat`] the test is an **index match** against the row the
/// name `AddMoneyDropRate` actually resolved to, which is why the status half
/// runs first in a pass. For [`DataFields::VaryStatRate`] there is no row index
/// to match, only a byte, so the test is against [`MONEY_STAT_POS`] and is
/// PLAUSIBLE rather than confirmed - a prefix is a reading aid and decides
/// nothing.
pub fn prefix(fields: &DataFields, refs: &Refs) -> &'static str {
    match fields {
        DataFields::VaryStat { row, .. }
        | DataFields::VaryStaticStat { row, .. }
        | DataFields::VaryStaticStatLevel { row, .. }
        | DataFields::VaryStaticStatRate { row, .. } => {
            if refs.money_row == Some(*row) {
                "MONEY "
            } else if refs.equip_row == Some(*row) {
                "EQUIP "
            } else {
                ""
            }
        }
        DataFields::VaryStatRate { b90, .. } => {
            if *b90 == MONEY_STAT_POS {
                "MONEY "
            } else if *b90 == EQUIP_DROP_STAT_POS {
                "EQUIP "
            } else {
                ""
            }
        }
        _ => "",
    }
}

/// Everything one interest line names.
///
/// A struct rather than nine arguments, which is both clearer at the call site
/// and what keeps the renderer inside clippy's argument count.
#[derive(Debug, Clone, Copy)]
pub struct BuffLine<'a> {
    /// The record's index in the manager's object array.
    pub record: usize,
    pub key: u32,
    /// The record's `_stringKey`, or `None` when it would not read - printed as
    /// `?`, never as an empty name.
    pub name: Option<&'a str>,
    /// The entry's index in the record's `_buffDataList`.
    pub entry: usize,
    /// [`parsed::entry::U32`], printed as `lvl=`. See that constant for why the
    /// two names differ.
    pub entry_u32: u32,
    pub what: Interest,
    pub base: &'a BaseFields,
    pub fields: &'a DataFields,
    pub refs: Refs<'a>,
}

/// The one greppable line per interesting `buffDataList` entry.
///
/// Built here rather than in `census.rs` because it is what a human reads the
/// results out of, so it is worth a native test - the same argument
/// [`crate::node::Operation::log_line`] makes.
pub fn buff_line(l: &BuffLine) -> String {
    let name = l.name.unwrap_or("?");
    format!(
        "{}buff idx={} key={} name={name} entry={} lvl={} class={} kind={} {} base={}",
        prefix(l.fields, &l.refs),
        l.record,
        l.key,
        l.entry,
        l.entry_u32,
        l.what.label(),
        l.what.kind(),
        fields_text(l.fields, &l.refs),
        base_text(l.base)
    )
}

/// The class-specific half of an interest line.
fn fields_text(fields: &DataFields, refs: &Refs) -> String {
    match fields {
        DataFields::VaryCollectDropRate { key, u94, value } => {
            format!("key90={key} u94={u94} value={value}")
        }
        DataFields::VaryStaticStat { row, v98 } => format!(
            "stat={}({}) v98={v98}",
            crate::node::row_text(*row),
            refs.stat_name.unwrap_or("-")
        ),
        DataFields::VaryStaticStatLevel { row, v98, a0 } => format!(
            "stat={}({}) v98={v98} a0={a0}",
            crate::node::row_text(*row),
            refs.stat_name.unwrap_or("-")
        ),
        DataFields::VaryStaticStatRate { row, v98, a0 } => format!(
            "stat={}({}) v98={v98} a0={a0}",
            crate::node::row_text(*row),
            refs.stat_name.unwrap_or("-")
        ),
        DataFields::VaryStat { row, v98, a0, a8, b0, b8 } => format!(
            "stat={}({}) v98={v98} a0={a0} a8={a8} b0={b0} b8={b8}",
            crate::node::row_text(*row),
            refs.stat_name.unwrap_or("-")
        ),
        DataFields::VaryStatRate { b90, b91, v98 } => {
            format!("b90={b90} b91={b91} v98={v98}")
        }
        DataFields::Loot { b90 } => format!("b90={b90}"),
        DataFields::RegisterItemSellPriceRate { rate }
        | DataFields::RegisterFactionOperationRewardRate { rate } => format!("rate={rate}"),
        // No named field exists for this class: its reader was never resolved,
        // so the bytes are the whole of what can honestly be said about it.
        DataFields::RegisterCrimePriceRate { raw } => {
            format!("raw90={}", crate::node::hex_line(raw))
        }
    }
}

/// The unnamed base fields, by offset. Compact on purpose: every interest line
/// carries them, and they are there to be correlated against a buff whose
/// effect is known rather than read one at a time.
fn base_text(b: &BaseFields) -> String {
    format!(
        "[8={} 9={} a={} b={} c={} 10={} 14={} 15={} 18={} 20={} 28={} 38={} 48={} 4c={} 50={} \
         54={} 80={} 88={}]",
        b.b08,
        b.b09,
        b.b0a,
        b.b0b,
        b.b0c,
        b.b10,
        b.b14,
        b.b15,
        b.b18,
        b.b20,
        b.b28,
        b.b38,
        b.b48,
        b.b4c,
        b.b50,
        b.b54,
        b.b80,
        b.b88
    )
}

/// The one line per `statusinfo` row whose name the game itself looks up.
///
/// `hash=` is printed beside the name because the game's startup lookup
/// (`FUN_14250b680`) matches on that hash, so the pair is what proves this row
/// is the row the name resolves to rather than a row that happens to be called
/// that.
pub fn stat_line(record: usize, name: Option<&str>, f: &StatusFields) -> String {
    format!(
        "stat idx={record} key={} name={} statType={} staticStatType={} elementalStatType={} \
         initValueType={} regenerateType={} usePercent={} isHardCoded={} hash=0x{:X} \
         buffInfo={} refer={} minHash=0x{:X} maxHash=0x{:X} slots={}",
        f.key,
        name.unwrap_or("?"),
        f.stat_type,
        f.static_stat_type,
        f.elemental_stat_type,
        f.init_value_type,
        f.regenerate_type,
        f.use_percent,
        f.is_hard_coded,
        f.hash,
        f.buff_info,
        f.refer,
        f.min_hash,
        f.max_hash,
        f.slot_count
    )
}

// ---------------------------------------------------------------------------
// What a pass stepped over, and what it learned
// ---------------------------------------------------------------------------

/// Every reason a record or an entry was stepped over.
///
/// Each one is a place where the offsets above stopped matching what the game
/// actually has, which is the first thing to look at when the verdict comes
/// back bad. Same shape and the same purpose as [`crate::node::Skips`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skips {
    /// The record slot is still null: the game has not parsed that record.
    /// **Normal**, in both tables, and counted rather than complained about.
    pub unloaded: usize,
    /// A record slot would not read, or held something no pointer check accepts.
    pub record_read: usize,
    /// The record's head block would not read, or was too short to decode.
    pub head_read: usize,
    /// An entry's [`parsed::entry::STRIDE`] bytes would not read.
    pub entry_read: usize,
    /// The entry's BuffData pointer is null. Also normal: the factory writes
    /// null when the stream says the data is absent.
    pub null_entry: usize,
    /// The object's class could not be read out of its RTTI. Counted as walked
    /// anyway, because the verdict is about what share of walked entries named
    /// a class.
    pub no_rtti: usize,
    /// A record declared more entries than [`MAX_ENTRIES_PER_RECORD`].
    pub entries_capped: usize,
    /// Entries never reached because the pass hit [`MAX_TOTAL_ENTRIES`].
    /// Non-zero means a misread count, not a big table.
    pub entry_budget: usize,
    /// A string field would not read, or held bytes outside printable ASCII.
    pub string_read: usize,
}

impl Skips {
    /// True when nothing at all was stepped over.
    pub fn is_empty(&self) -> bool {
        *self == Skips::default()
    }

    /// `unloaded=3 no_rtti=1`, naming only the counters that moved.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (name, n) in [
            ("unloaded", self.unloaded),
            ("record_read", self.record_read),
            ("head_read", self.head_read),
            ("entry_read", self.entry_read),
            ("null_entry", self.null_entry),
            ("no_rtti", self.no_rtti),
            ("entries_capped", self.entries_capped),
            ("entry_budget", self.entry_budget),
            ("string_read", self.string_read),
        ] {
            if n > 0 {
                parts.push(format!("{name}={n}"));
            }
        }
        parts.join(" ")
    }
}

/// Everything one census pass learned, over both tables.
///
/// One type for both halves because the two are one finding: the buff half is
/// only interpretable against the `statusinfo` rows the status half named, and
/// the `statType` histogram is the evidence that decides whether that byte is
/// the same 19-value enum the names are. Each half fills the counters it owns
/// and the lines print what they need.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// Record slots walked, loaded or not.
    pub records: usize,
    /// Of those, the ones that held an object and read.
    pub loaded: usize,
    /// Non-null `buffDataList` entries walked. The denominator of the verdict.
    pub entries: usize,
    /// Entries whose RTTI gave a class name at all.
    pub named: usize,
    /// Of those, the ones whose name ends in `BuffData`.
    pub buffdata: usize,
    /// Entries of each of the seven classes.
    counts: BTreeMap<Interest, usize>,
    /// `VaryStat` entries seen, against the ones a line was written for: only
    /// the money/equip ones are logged, and there may be thousands of the rest.
    pub vary_stat_seen: usize,
    pub vary_stat_logged: usize,
    /// The same for `VaryStatRate`.
    pub vary_stat_rate_seen: usize,
    pub vary_stat_rate_logged: usize,
    /// Entries referencing the money row, and the equip-drop row.
    pub money_refs: usize,
    pub equip_refs: usize,
    /// Every class name seen, by count. Capped at [`MAX_CLASSES`] keys with an
    /// [`OTHER_CLASS`] bucket: this is the census that says what a
    /// `buffDataList` actually holds, so the tail matters as a number even when
    /// it cannot matter as a name.
    classes: BTreeMap<String, usize>,
    /// [`parsed::status::STAT_TYPE`] over every loaded `statusinfo` row. The
    /// whole point of the status half's second line: if it takes 19 values it
    /// may be the stat enum, and if it takes 3 it is something else.
    stat_types: BTreeMap<u8, usize>,
    /// The well-known stat names found, with the row each resolved to.
    well_known: Vec<(String, u16)>,
    /// [`parsed::data::VARY_STAT_RATE_B90`] over every `VaryStatRate` entry,
    /// capped at [`MAX_VSR90_KEYS`] keys. What settles whether that byte is a
    /// position in the 19-name table.
    vsr90: BTreeMap<u8, usize>,
    /// `(record name, declared entry count)` of every record whose list was
    /// longer than [`MAX_ENTRIES_PER_RECORD`], the first [`MAX_CAPPED_NAMED`]
    /// of them.
    capped: Vec<(String, u32)>,
    pub skips: Skips,
}

impl Summary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one record slot, and say whether it held a readable object.
    pub fn record(&mut self, loaded: bool) {
        self.records += 1;
        if loaded {
            self.loaded += 1;
        }
    }

    /// Count one walked non-null entry.
    pub fn entry(&mut self) {
        self.entries += 1;
    }

    /// Fold in the class name one entry's RTTI gave.
    pub fn class(&mut self, name: &str) {
        self.named += 1;
        if name.ends_with("BuffData") {
            self.buffdata += 1;
        }
        if let Some(n) = self.classes.get_mut(name) {
            *n += 1;
        } else if self.classes.len() < MAX_CLASSES {
            self.classes.insert(name.to_string(), 1);
        } else {
            *self.classes.entry(OTHER_CLASS.to_string()).or_insert(0) += 1;
        }
    }

    /// Count one entry of one of the seven classes.
    pub fn interest(&mut self, what: Interest) {
        *self.counts.entry(what).or_insert(0) += 1;
    }

    /// One `VaryStat` entry, and whether a line was written for it.
    pub fn vary_stat(&mut self, logged: bool) {
        self.vary_stat_seen += 1;
        if logged {
            self.vary_stat_logged += 1;
        }
    }

    /// One `VaryStatRate` entry: its `+0x90`, and whether it was logged.
    pub fn vary_stat_rate(&mut self, b90: u8, logged: bool) {
        self.vary_stat_rate_seen += 1;
        if logged {
            self.vary_stat_rate_logged += 1;
        }
        if self.vsr90.len() < MAX_VSR90_KEYS || self.vsr90.contains_key(&b90) {
            *self.vsr90.entry(b90).or_insert(0) += 1;
        }
    }

    /// One entry that references the money row.
    pub fn money_ref(&mut self) {
        self.money_refs += 1;
    }

    /// Remember a record whose entry list was cut at [`MAX_ENTRIES_PER_RECORD`].
    /// Past [`MAX_CAPPED_NAMED`] only the skip counter moves.
    pub fn capped_record(&mut self, name: &str, declared: u32) {
        if self.capped.len() < MAX_CAPPED_NAMED {
            self.capped.push((name.to_string(), declared));
        }
    }

    /// `name=declared ...` for the capped records, or `None` when no record
    /// was capped - so the line is written only when it says something.
    pub fn capped_line(&self) -> Option<String> {
        if self.capped.is_empty() {
            return None;
        }
        let mut s = String::new();
        for (i, (name, declared)) in self.capped.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{name}={declared}");
        }
        if self.skips.entries_capped > self.capped.len() {
            let _ = write!(s, " (+{} more)", self.skips.entries_capped - self.capped.len());
        }
        Some(s)
    }

    /// One entry that references the equipment-drop row.
    pub fn equip_ref(&mut self) {
        self.equip_refs += 1;
    }

    /// One loaded `statusinfo` row's [`parsed::status::STAT_TYPE`].
    pub fn stat_type(&mut self, v: u8) {
        *self.stat_types.entry(v).or_insert(0) += 1;
    }

    /// One of the 19 names, and the row it resolved to.
    pub fn found_stat(&mut self, name: &str, row: u16) {
        if self.well_known.iter().any(|(n, _)| n == name) {
            return;
        }
        self.well_known.push((name.to_string(), row));
    }

    /// The well-known rows found so far, in [`WELL_KNOWN_STATS`] order so two
    /// passes read the same way.
    pub fn well_known_rows(&self) -> Vec<(String, u16)> {
        let mut v = self.well_known.clone();
        v.sort_by_key(|(name, _)| {
            WELL_KNOWN_STATS.iter().position(|n| n == name).unwrap_or(WELL_KNOWN_STATS.len())
        });
        v
    }

    /// The names of [`WELL_KNOWN_STATS`] no loaded row carried.
    pub fn missing_stats(&self) -> Vec<&'static str> {
        WELL_KNOWN_STATS
            .into_iter()
            .filter(|want| !self.well_known.iter().any(|(name, _)| name == want))
            .collect()
    }

    /// `Name=idx Name=idx …` for the status half's first line.
    pub fn well_known_line(&self) -> String {
        let rows = self.well_known_rows();
        if rows.is_empty() {
            return "none".to_string();
        }
        let mut s = String::new();
        for (i, (name, row)) in rows.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            // `write!` into a String cannot fail; discarded rather than
            // unwrapped so this stays panic-free.
            let _ = write!(s, "{name}={row}");
        }
        s
    }

    /// How many of the 19 were found.
    pub fn well_known_count(&self) -> usize {
        self.well_known.len()
    }

    /// `0=120 1=7 …` over [`parsed::status::STAT_TYPE`], ascending by value so
    /// the shape of the enum is what the line shows.
    pub fn stat_type_line(&self) -> String {
        if self.stat_types.is_empty() {
            return "none".to_string();
        }
        let mut s = String::new();
        for (i, (v, n)) in self.stat_types.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{v}={n}");
        }
        let _ = write!(s, " ({} distinct values over {} rows)", self.stat_types.len(), self.loaded);
        s
    }

    /// `13=4 14=2 …` over [`parsed::data::VARY_STAT_RATE_B90`].
    pub fn vsr90_line(&self) -> String {
        if self.vsr90.is_empty() {
            return "none".to_string();
        }
        let mut s = String::new();
        for (i, (v, n)) in self.vsr90.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{v}={n}");
        }
        s
    }

    /// The seven classes and their counts, always all seven, so one `grep`
    /// over a log finds what a pass saw whether or not it saw any.
    pub fn interest_line(&self) -> String {
        let mut s = String::new();
        for (i, what) in Interest::ALL.into_iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{}={}", what.label(), self.counts.get(&what).copied().unwrap_or(0));
        }
        s
    }

    /// The class histogram, most common first, ties broken by name so two runs
    /// agree.
    pub fn class_histogram(&self) -> Vec<(&str, usize)> {
        let mut v: Vec<(&str, usize)> =
            self.classes.iter().map(|(k, n)| (k.as_str(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v
    }

    /// `VaryStatBuffData=3412 …`, capped at `max` names with a `(+N more)`
    /// tail. One line, however many classes there turn out to be.
    pub fn classes_line(&self, max: usize) -> String {
        let all = self.class_histogram();
        if all.is_empty() {
            return "none".to_string();
        }
        let shown = all.len().min(max);
        let mut s = String::new();
        for (i, (name, n)) in all.iter().take(shown).enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{name}={n}");
        }
        if all.len() > shown {
            let _ = write!(s, " (+{} more)", all.len() - shown);
        }
        s
    }

    /// The counts one pass ends with.
    pub fn counts_line(&self) -> String {
        format!(
            "records {} ({} loaded), entries walked {}, RTTI names {} ({} end in BuffData), \
             distinct classes {}; {}; row-stat classes (kinds 3/4/5/7) logged {} of {} seen, \
             VaryStatRate logged {} of {} seen, money-referencing {}, equip-referencing {}",
            self.records,
            self.loaded,
            self.entries,
            self.named,
            self.buffdata,
            self.classes.len(),
            self.interest_line(),
            self.vary_stat_logged,
            self.vary_stat_seen,
            self.vary_stat_rate_logged,
            self.vary_stat_rate_seen,
            self.money_refs,
            self.equip_refs
        )
    }

    /// Whether the offsets in [`parsed`] look like the right ones, as a
    /// sentence meant to be read in the log.
    ///
    /// Three conditions, and the test that carries the weight is the third: the
    /// entries of a `_buffDataList` all point at objects of one class family,
    /// so if the list pointer or the entry stride were wrong, the vtable read
    /// through them would resolve to a scatter of unrelated classes or to
    /// nothing at all. A wrong offset essentially never reads back 99%
    /// `*BuffData`.
    ///
    /// Unlike [`crate::node::Summary::verdict`] there is nothing to caveat
    /// here: this census writes nothing, so what it walks is always the table
    /// as the game parsed it.
    pub fn verdict(&self) -> String {
        let frac = if self.entries == 0 {
            0.0
        } else {
            self.buffdata as f64 / self.entries as f64
        };
        let mut why: Vec<&str> = Vec::new();
        if self.loaded == 0 {
            why.push("no buffinfo record was loaded");
        }
        if self.entries == 0 {
            why.push("no buffDataList entry was walked");
        } else if frac < VERDICT_NAMED {
            why.push("fewer than 99% of the walked entries resolved to a class named *BuffData");
        }
        if why.is_empty() {
            format!(
                "verdict: the layout LOOKS RIGHT - {} of {} records loaded, {} entries walked, \
                 {} of them ({:.1}%) resolved to a *BuffData class through their own RTTI",
                self.loaded,
                self.records,
                self.entries,
                self.buffdata,
                frac * 100.0
            )
        } else {
            format!(
                "verdict: the layout LOOKS WRONG - {} ({} of {} records loaded, {} entries \
                 walked, {} of them named a *BuffData class)",
                why.join("; "),
                self.loaded,
                self.records,
                self.entries,
                self.buffdata
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write `v` into `b` at `off`, little-endian, the way the game's own
    /// parsed record has it. Panics on a short buffer, which is a test bug.
    fn put(b: &mut [u8], off: usize, v: &[u8]) {
        b[off..off + v.len()].copy_from_slice(v);
    }

    /// One `buffinfo` head laid out at the offsets `parsed::buff` claims.
    fn head_bytes() -> [u8; parsed::buff::RAW] {
        let mut b = [0u8; parsed::buff::RAW];
        put(&mut b, parsed::buff::KEY, &4242u32.to_le_bytes());
        put(&mut b, parsed::buff::STRING_KEY, &0x1_2345_6780u64.to_le_bytes());
        put(&mut b, parsed::buff::IS_BLOCKED, &[1]);
        put(&mut b, parsed::buff::LIST, &0x2_0000_0000u64.to_le_bytes());
        put(&mut b, parsed::buff::COUNT, &3u32.to_le_bytes());
        put(&mut b, parsed::buff::CAPACITY, &4u32.to_le_bytes());
        put(&mut b, parsed::buff::MIN_LEVEL, &1u32.to_le_bytes());
        put(&mut b, parsed::buff::MAX_LEVEL, &10u32.to_le_bytes());
        put(&mut b, parsed::buff::SEQUENCER, &0x3_0000_0000u64.to_le_bytes());
        put(&mut b, parsed::buff::LEVEL_CALC_TYPE, &[2]);
        put(&mut b, parsed::buff::UI_TEMPLATE, &111u16.to_le_bytes());
        put(&mut b, parsed::buff::UI_COMPONENT, &222u16.to_le_bytes());
        put(&mut b, parsed::buff::ELEMENTAL_STATUS, &ROW_NONE.to_le_bytes());
        put(&mut b, parsed::buff::USE_SKILL_DESC, &[1]);
        put(&mut b, parsed::buff::USE_COUNTING, &[1]);
        b
    }

    /// A BuffData object block with the base fields set, ready for a subclass
    /// tail to be written over.
    fn data_bytes() -> [u8; parsed::data::RAW] {
        use parsed::data as o;
        let mut b = [0u8; o::RAW];
        put(&mut b, o::B0C, &12u32.to_le_bytes());
        put(&mut b, o::B10, &16u32.to_le_bytes());
        put(&mut b, o::B14, &[20]);
        put(&mut b, o::B15, &[21]);
        put(&mut b, o::B18, &(-24i64).to_le_bytes());
        put(&mut b, o::B20, &32i64.to_le_bytes());
        put(&mut b, o::B28, &40i64.to_le_bytes());
        put(&mut b, o::B38, &ROW_NONE.to_le_bytes());
        put(&mut b, o::B48, &72u32.to_le_bytes());
        put(&mut b, o::B4C, &76u32.to_le_bytes());
        put(&mut b, o::B50, &80u32.to_le_bytes());
        put(&mut b, o::B54, &84u32.to_le_bytes());
        put(&mut b, o::B80, &128u32.to_le_bytes());
        put(&mut b, o::B88, &136u32.to_le_bytes());
        b
    }

    /// The head offsets, tested the only way that can fail when one of them
    /// moves: by decoding bytes laid out the way the game's records are, with
    /// the values stated in a different place from the offsets.
    #[test]
    fn a_buffinfo_head_decodes_to_the_fields_at_those_offsets() {
        let b = head_bytes();
        let h = decode_buff_head(&b).expect("a full head decodes");
        assert_eq!(h.key, 4242);
        assert_eq!(h.string_key, 0x1_2345_6780);
        assert_eq!(h.is_blocked, 1);
        assert_eq!(h.list, 0x2_0000_0000);
        assert_eq!((h.count, h.capacity), (3, 4));
        assert_eq!((h.min_level, h.max_level), (1, 10));
        assert_eq!(h.sequencer, 0x3_0000_0000);
        assert_eq!(h.level_calc_type, 2);
        assert_eq!((h.ui_template, h.ui_component), (111, 222));
        assert_eq!(h.elemental_status, ROW_NONE);
        assert_eq!((h.use_skill_desc, h.use_counting), (1, 1));

        // A block that stops short of the last named field decodes to nothing
        // rather than to a plausible-looking half-record.
        assert_eq!(decode_buff_head(&b[..parsed::buff::USE_COUNTING]), None);
        assert_eq!(decode_buff_head(&[]), None);
        // Every field named is inside the block the census reads.
        const { assert!(parsed::buff::USE_COUNTING < parsed::buff::RAW) };
    }

    #[test]
    fn an_entry_decodes_to_its_level_word_and_object_pointer() {
        let mut b = [0u8; parsed::entry::STRIDE];
        put(&mut b, parsed::entry::U32, &7u32.to_le_bytes());
        put(&mut b, parsed::entry::OBJ, &0x1_4000_1000u64.to_le_bytes());
        assert_eq!(decode_entry(&b), Some((7, 0x1_4000_1000)));

        // A null object is a reading, not a failure: the factory writes null
        // when the stream says the data is absent.
        let mut b = [0u8; parsed::entry::STRIDE];
        put(&mut b, parsed::entry::U32, &2u32.to_le_bytes());
        assert_eq!(decode_entry(&b), Some((2, 0)));

        assert_eq!(decode_entry(&b[..parsed::entry::OBJ + 1]), None);
        assert_eq!(decode_entry(&[]), None);
    }

    #[test]
    fn the_base_fields_decode_at_the_offsets_they_are_printed_by() {
        let b = data_bytes();
        let f = decode_base(&b).expect("a full object decodes");
        assert_eq!((f.b08, f.b09, f.b0a, f.b0b), (0, 0, 0, 0));
        assert_eq!(f.b0c, 12);
        assert_eq!(f.b10, 16);
        assert_eq!((f.b14, f.b15), (20, 21));
        assert_eq!((f.b18, f.b20, f.b28), (-24, 32, 40));
        assert_eq!(f.b38, ROW_NONE);
        assert_eq!((f.b48, f.b4c, f.b50, f.b54), (72, 76, 80, 84));
        assert_eq!((f.b80, f.b88), (128, 136));

        assert_eq!(decode_base(&b[..parsed::data::B88]), None);
        assert_eq!(decode_base(&[]), None);
        // The base block ends before the subclass fields start, which is the
        // one thing that would silently corrupt both halves if it stopped
        // being true.
        const { assert!(parsed::data::B88 + 4 <= parsed::data::SUB) };
    }

    #[test]
    fn each_classs_own_fields_decode_at_its_own_offsets() {
        use parsed::data as o;

        let mut b = data_bytes();
        put(&mut b, o::COLLECT_KEY, &(-5i32).to_le_bytes());
        put(&mut b, o::COLLECT_U94, &60101u32.to_le_bytes());
        put(&mut b, o::COLLECT_VALUE, &1234i64.to_le_bytes());
        assert_eq!(
            decode_fields(Interest::VaryCollectDropRate, &b),
            Some(DataFields::VaryCollectDropRate { key: -5, u94: 60101, value: 1234 })
        );

        // The three static-stat kinds share the row and the first value and
        // differ only in the tail.
        let mut b = data_bytes();
        put(&mut b, o::STATIC_STAT_ROW, &46u16.to_le_bytes());
        put(&mut b, o::STATIC_STAT_V98, &250_000i64.to_le_bytes());
        put(&mut b, o::STATIC_STAT_LEVEL_A0, &[7]);
        assert_eq!(
            decode_fields(Interest::VaryStaticStat, &b),
            Some(DataFields::VaryStaticStat { row: 46, v98: 250_000 })
        );
        assert_eq!(
            decode_fields(Interest::VaryStaticStatLevel, &b),
            Some(DataFields::VaryStaticStatLevel { row: 46, v98: 250_000, a0: 7 })
        );
        let mut b = data_bytes();
        put(&mut b, o::STATIC_STAT_ROW, &21u16.to_le_bytes());
        put(&mut b, o::STATIC_STAT_V98, &1i64.to_le_bytes());
        put(&mut b, o::STATIC_STAT_RATE_A0, &(-2i64).to_le_bytes());
        assert_eq!(
            decode_fields(Interest::VaryStaticStatRate, &b),
            Some(DataFields::VaryStaticStatRate { row: 21, v98: 1, a0: -2 })
        );

        let mut b = data_bytes();
        put(&mut b, o::VARY_STAT_ROW, &4711u16.to_le_bytes());
        put(&mut b, o::VARY_STAT_V98, &100i64.to_le_bytes());
        put(&mut b, o::VARY_STAT_A0, &200i64.to_le_bytes());
        put(&mut b, o::VARY_STAT_A8, &300i64.to_le_bytes());
        put(&mut b, o::VARY_STAT_B0, &4u32.to_le_bytes());
        put(&mut b, o::VARY_STAT_B8, &[9]);
        assert_eq!(
            decode_fields(Interest::VaryStat, &b),
            Some(DataFields::VaryStat { row: 4711, v98: 100, a0: 200, a8: 300, b0: 4, b8: 9 })
        );

        let mut b = data_bytes();
        put(&mut b, o::VARY_STAT_RATE_B90, &[14]);
        put(&mut b, o::VARY_STAT_RATE_B91, &[1]);
        put(&mut b, o::VARY_STAT_RATE_V98, &(-7i64).to_le_bytes());
        assert_eq!(
            decode_fields(Interest::VaryStatRate, &b),
            Some(DataFields::VaryStatRate { b90: 14, b91: 1, v98: -7 })
        );

        let mut b = data_bytes();
        put(&mut b, o::LOOT_B90, &[3]);
        assert_eq!(decode_fields(Interest::Loot, &b), Some(DataFields::Loot { b90: 3 }));

        let mut b = data_bytes();
        put(&mut b, o::RATE, &15000i64.to_le_bytes());
        assert_eq!(
            decode_fields(Interest::RegisterItemSellPriceRate, &b),
            Some(DataFields::RegisterItemSellPriceRate { rate: 15000 })
        );
        assert_eq!(
            decode_fields(Interest::RegisterFactionOperationRewardRate, &b),
            Some(DataFields::RegisterFactionOperationRewardRate { rate: 15000 })
        );

        // The class with no resolved reader carries bytes, not fields.
        let mut b = data_bytes();
        put(&mut b, o::CRIME_RAW, &[0xAB]);
        match decode_fields(Interest::RegisterCrimePriceRate, &b) {
            Some(DataFields::RegisterCrimePriceRate { raw }) => {
                assert_eq!(raw.len(), o::CRIME_RAW_LEN);
                assert_eq!(raw.first(), Some(&0xAB));
            }
            other => panic!("kind 100 decodes to bytes: {other:?}"),
        }

        // Every class refuses a block that stops before its own tail.
        for what in Interest::ALL {
            assert_eq!(decode_fields(what, &b[..o::SUB]), None, "{}", what.label());
            assert_eq!(decode_fields(what, &[]), None, "{}", what.label());
        }
    }

    #[test]
    fn a_statusinfo_record_decodes_to_the_fields_at_those_offsets() {
        use parsed::status as o;
        let mut b = [0u8; o::RAW];
        put(&mut b, o::KEY, &555u32.to_le_bytes());
        put(&mut b, o::STRING_KEY, &0x1_1111_0000u64.to_le_bytes());
        put(&mut b, o::IS_BLOCKED, &[0]);
        put(&mut b, o::REGENERATE_TYPE, &[2]);
        put(&mut b, o::STATUS_INDEX, &33u32.to_le_bytes());
        put(&mut b, o::IS_HARD_CODED, &[1]);
        put(&mut b, o::INIT_VALUE_TYPE, &[3]);
        put(&mut b, o::MIN_RESISTANCE, &11u16.to_le_bytes());
        put(&mut b, o::MAX_RESISTANCE, &12u16.to_le_bytes());
        put(&mut b, o::IS_RESISTANCE_STAT, &[1]);
        put(&mut b, o::IS_ELEMENTAL_STAT, &[0]);
        put(&mut b, o::DECREASE_ON_BROKEN, &[1]);
        put(&mut b, o::BUFF_INFO, &77u16.to_le_bytes());
        put(&mut b, o::REFER, &88u32.to_le_bytes());
        put(&mut b, o::STAT_TYPE, &[5]);
        put(&mut b, o::STATIC_STAT_TYPE, &[6]);
        put(&mut b, o::ELEMENTAL_STAT_TYPE, &[7]);
        put(&mut b, o::ACTIVE_KNOWLEDGE, &99u16.to_le_bytes());
        put(&mut b, o::SEND_GIMMICK_EVENT, &1234u32.to_le_bytes());
        put(&mut b, o::SLOTS, &0x2_2222_0000u64.to_le_bytes());
        put(&mut b, o::SLOT_COUNT, &4u32.to_le_bytes());
        put(&mut b, o::SLOT_CAPACITY, &8u32.to_le_bytes());
        put(&mut b, o::USE_LIMIT_MIN, &[1]);
        put(&mut b, o::USE_LIMIT_MAX, &[1]);
        put(&mut b, o::HASH, &0xDEAD_BEEFu32.to_le_bytes());
        put(&mut b, o::MIN_HASH, &0x1111_2222u32.to_le_bytes());
        put(&mut b, o::MAX_HASH, &0x3333_4444u32.to_le_bytes());
        put(&mut b, o::FULL_RECOVER, &[1]);
        put(&mut b, o::USE_PERCENT, &[1]);
        put(&mut b, o::REPEAT_UPDATE, &[0]);
        put(&mut b, o::RESET_ON_REVIVE, &[1]);
        put(&mut b, o::UI_TEMPLATE, &101u16.to_le_bytes());
        put(&mut b, o::UI_COMPONENT, &102u16.to_le_bytes());

        let f = decode_status(&b).expect("a full record decodes");
        assert_eq!(f.key, 555);
        assert_eq!(f.string_key, 0x1_1111_0000);
        assert_eq!((f.regenerate_type, f.init_value_type), (2, 3));
        assert_eq!(f.status_index, 33);
        assert_eq!((f.min_resistance, f.max_resistance), (11, 12));
        assert_eq!(f.buff_info, 77);
        assert_eq!(f.refer, 88);
        // The three type bytes are adjacent and must not be confused: the
        // histogram the census prints is over the first of them alone.
        assert_eq!((f.stat_type, f.static_stat_type, f.elemental_stat_type), (5, 6, 7));
        assert_eq!(f.active_knowledge, 99);
        assert_eq!(f.send_gimmick_event, 1234);
        assert_eq!((f.slot_count, f.slot_capacity), (4, 8));
        assert_eq!(f.hash, 0xDEAD_BEEF);
        assert_eq!((f.min_hash, f.max_hash), (0x1111_2222, 0x3333_4444));
        assert_eq!((f.use_percent, f.full_recover, f.repeat_update), (1, 1, 0));
        assert_eq!(f.reset_on_revive, 1);
        assert_eq!((f.ui_template, f.ui_component), (101, 102));

        assert_eq!(decode_status(&b[..o::UI_COMPONENT + 1]), None);
        assert_eq!(decode_status(&[]), None);
        const { assert!(o::MAX_PASSIVE_SKILLS + 16 <= o::RAW) };
    }

    /// The seven names, and the one that is deliberately not one of them:
    /// `VaryStatMaxValueBuffData` is a different class with a different layout,
    /// and a prefix match would decode it at `VaryStatBuffData`'s offsets.
    #[test]
    fn from_class_takes_the_ten_and_nothing_adjacent() {
        for what in Interest::ALL {
            let name = format!("{}BuffData", what.label());
            assert_eq!(Interest::from_class(&name), Some(what), "{name}");
        }
        assert_eq!(Interest::from_class("VaryStatMaxValueBuffData"), None);
        assert_eq!(Interest::from_class("VaryStat"), None, "the suffix is required");
        assert_eq!(Interest::from_class("BuffData"), None);
        assert_eq!(Interest::from_class(""), None);
        // The kind bytes, which the log prints beside the class.
        assert_eq!(
            Interest::ALL.map(Interest::kind),
            [2, 3, 4, 5, 7, 8, 11, 99, 100, 101],
            "the by-kind switch FUN_141e7a1a0's numbers"
        );
    }

    #[test]
    fn a_mangled_name_needs_both_affixes() {
        assert_eq!(class_from_mangled(".?AVVaryStatBuffData@pa@@"), Some("VaryStatBuffData"));
        assert_eq!(class_from_mangled(".?AVVaryStatBuffData@@"), None, "wrong namespace");
        assert_eq!(class_from_mangled("VaryStatBuffData@pa@@"), None, "no prefix");
        assert_eq!(class_from_mangled(""), None);
    }

    /// The prefix is what a human greps for, so which line gets one is worth a
    /// test of its own. The row match is against the row the *name* resolved
    /// to; the byte match is against a position in the name table.
    #[test]
    fn the_money_and_equip_prefixes_land_on_the_right_lines() {
        let refs = Refs { money_row: Some(400), equip_row: Some(401), stat_name: None };
        let vary = |row| DataFields::VaryStat { row, v98: 0, a0: 0, a8: 0, b0: 0, b8: 0 };
        assert_eq!(prefix(&vary(400), &refs), "MONEY ");
        assert_eq!(prefix(&vary(401), &refs), "EQUIP ");
        assert_eq!(prefix(&vary(402), &refs), "");
        // Nothing is prefixed while the status half has not found the rows:
        // a `VaryStat` naming row 400 is only money if `AddMoneyDropRate` is
        // row 400, and until that is known it is just a stat change.
        assert_eq!(prefix(&vary(400), &Refs::default()), "");

        // The three static-stat kinds are judged by the same row match.
        assert_eq!(prefix(&DataFields::VaryStaticStat { row: 400, v98: 0 }, &refs), "MONEY ");
        assert_eq!(
            prefix(&DataFields::VaryStaticStatLevel { row: 401, v98: 0, a0: 0 }, &refs),
            "EQUIP "
        );
        assert_eq!(
            prefix(&DataFields::VaryStaticStatRate { row: 402, v98: 0, a0: 0 }, &refs),
            ""
        );
        assert_eq!(
            prefix(&DataFields::VaryStaticStat { row: 400, v98: 0 }, &Refs::default()),
            ""
        );

        let rate = |b90| DataFields::VaryStatRate { b90, b91: 0, v98: 0 };
        assert_eq!(prefix(&rate(MONEY_STAT_POS), &Refs::default()), "MONEY ");
        assert_eq!(prefix(&rate(EQUIP_DROP_STAT_POS), &Refs::default()), "EQUIP ");
        assert_eq!(prefix(&rate(0), &refs), "");
        // No other class is ever prefixed, whatever its fields say.
        assert_eq!(prefix(&DataFields::Loot { b90: 14 }, &refs), "");
        assert_eq!(
            prefix(&DataFields::RegisterItemSellPriceRate { rate: 14 }, &refs),
            ""
        );
    }

    /// The two positions the whole investigation turns on. They are asserted
    /// against the names rather than restated as numbers, so reordering the
    /// array fails here instead of quietly moving what `MONEY ` means.
    #[test]
    fn the_name_table_holds_the_two_drop_rate_stats_where_the_code_says() {
        assert_eq!(WELL_KNOWN_STATS.len(), 19);
        assert_eq!(WELL_KNOWN_STATS[14], "AddMoneyDropRate");
        assert_eq!(WELL_KNOWN_STATS[13], "EquipDropRate");
        assert_eq!(WELL_KNOWN_STATS[usize::from(MONEY_STAT_POS)], MONEY_STAT);
        assert_eq!(WELL_KNOWN_STATS[usize::from(EQUIP_DROP_STAT_POS)], EQUIP_DROP_STAT);
        // No duplicates: a name appearing twice would make the row lookup
        // ambiguous and the histogram misleading.
        let mut sorted = WELL_KNOWN_STATS;
        sorted.sort_unstable();
        let mut deduped = sorted.to_vec();
        deduped.dedup();
        assert_eq!(deduped.len(), WELL_KNOWN_STATS.len());
    }

    /// A pass that read nothing must not be able to say the layout is right.
    /// This is the condition that would hide a wrong `manager+0x58` walk.
    #[test]
    fn an_empty_pass_looks_wrong() {
        let sum = Summary::new();
        let v = sum.verdict();
        assert!(v.contains("LOOKS WRONG"), "{v}");
        assert!(v.contains("no buffinfo record was loaded"), "{v}");
        assert!(v.contains("no buffDataList entry was walked"), "{v}");
        assert!(sum.skips.is_empty());
        assert_eq!(sum.skips.summary(), "");
        assert_eq!(sum.classes_line(40), "none");
        assert_eq!(sum.vsr90_line(), "none");
        assert_eq!(sum.stat_type_line(), "none");
        assert_eq!(sum.well_known_line(), "none");
        assert_eq!(sum.missing_stats().len(), 19);
    }

    /// And a pass that read a table of `*BuffData` objects must say so. One
    /// unnamed entry in a hundred is inside the 99% floor; two are not.
    #[test]
    fn a_full_pass_of_buffdata_objects_looks_right() {
        let mut sum = Summary::new();
        sum.record(true);
        for i in 0..100 {
            sum.entry();
            if i == 0 {
                sum.skips.no_rtti += 1;
            } else {
                sum.class("VaryStatBuffData");
            }
        }
        let v = sum.verdict();
        assert!(v.contains("LOOKS RIGHT"), "{v}");

        sum.entry();
        sum.skips.no_rtti += 1;
        let v = sum.verdict();
        assert!(v.contains("LOOKS WRONG"), "two misses in 101 is below the floor: {v}");
    }

    #[test]
    fn the_class_histogram_is_capped_with_a_tail_bucket() {
        let mut sum = Summary::new();
        for i in 0..MAX_CLASSES + 10 {
            sum.class(&format!("Class{i:04}BuffData"));
        }
        // The cap holds, the overflow is counted, and nothing was lost from the
        // totals.
        assert!(sum.class_histogram().len() <= MAX_CLASSES + 1);
        assert_eq!(sum.named, MAX_CLASSES + 10);
        assert!(sum.classes_line(3).contains("more)"));
        assert!(sum.class_histogram().iter().any(|(n, _)| *n == OTHER_CLASS));
    }

    #[test]
    fn the_vary_stat_rate_histogram_is_capped_too() {
        let mut sum = Summary::new();
        for b90 in 0..=u8::MAX {
            sum.vary_stat_rate(b90, b90 == MONEY_STAT_POS);
        }
        assert_eq!(sum.vary_stat_rate_seen, 256);
        assert_eq!(sum.vary_stat_rate_logged, 1);
        assert_eq!(sum.vsr90_line().split(' ').count(), MAX_VSR90_KEYS);
    }

    /// The status half's bookkeeping: what was found, what is missing, and the
    /// order the names are reported in.
    #[test]
    fn the_well_known_rows_report_in_name_table_order() {
        let mut sum = Summary::new();
        sum.found_stat(MONEY_STAT, 4711);
        sum.found_stat(EQUIP_DROP_STAT, 4710);
        // A second sighting of the same name does not duplicate it.
        sum.found_stat(MONEY_STAT, 9999);
        assert_eq!(sum.well_known_count(), 2);
        assert_eq!(sum.well_known_line(), "EquipDropRate=4710 AddMoneyDropRate=4711");
        assert_eq!(sum.missing_stats().len(), 17);
        assert!(!sum.missing_stats().contains(&MONEY_STAT));
    }

    /// The lines themselves. A renderer is what a human reads the census out
    /// of, so the fields it names are worth pinning.
    #[test]
    fn the_lines_name_their_fields() {
        let base = decode_base(&data_bytes()).expect("base decodes");
        let fields = DataFields::VaryStat { row: 400, v98: 5, a0: 6, a8: 7, b0: 8, b8: 9 };
        let line = buff_line(&BuffLine {
            record: 12,
            key: 4242,
            name: Some("buff_money_drop"),
            entry: 1,
            entry_u32: 3,
            what: Interest::VaryStat,
            base: &base,
            fields: &fields,
            refs: Refs {
                money_row: Some(400),
                equip_row: None,
                stat_name: Some(MONEY_STAT),
            },
        });
        assert!(line.starts_with("MONEY buff idx=12 key=4242 name=buff_money_drop entry=1 lvl=3 "), "{line}");
        assert!(line.contains("class=VaryStat kind=5 "), "{line}");
        assert!(line.contains("stat=400(AddMoneyDropRate) v98=5 a0=6 a8=7 b0=8 b8=9"), "{line}");
        assert!(
            line.contains("base=[8=0 9=0 a=0 b=0 c=12 10=16 14=20 15=21 18=-24 20=32 28=40 38=65535 48=72"),
            "{line}"
        );

        // An unreadable name is `?`, never an empty field, and a row of
        // `0xFFFF` prints as `-` in both halves of the pair.
        let fields = DataFields::VaryStat {
            row: ROW_NONE,
            v98: 0,
            a0: 0,
            a8: 0,
            b0: 0,
            b8: 0,
        };
        let line = buff_line(&BuffLine {
            record: 0,
            key: 1,
            name: None,
            entry: 0,
            entry_u32: 0,
            what: Interest::VaryStat,
            base: &base,
            fields: &fields,
            refs: Refs::default(),
        });
        assert!(line.contains("name=? "), "{line}");
        assert!(line.contains("stat=-(-) "), "{line}");

        let mut b = [0u8; parsed::status::RAW];
        put(&mut b, parsed::status::KEY, &555u32.to_le_bytes());
        put(&mut b, parsed::status::STAT_TYPE, &[5]);
        put(&mut b, parsed::status::HASH, &0xDEAD_BEEFu32.to_le_bytes());
        let f = decode_status(&b).expect("status decodes");
        let line = stat_line(7, Some(MONEY_STAT), &f);
        assert!(line.starts_with("stat idx=7 key=555 name=AddMoneyDropRate statType=5 "), "{line}");
        assert!(line.contains("hash=0xDEADBEEF"), "{line}");
    }

    /// The caps, stated once so a future edit cannot quietly make the global
    /// bound smaller than the per-record one it is supposed to backstop.
    #[test]
    fn the_caps_bound_each_other() {
        const { assert!(MAX_ENTRIES_PER_RECORD <= MAX_TOTAL_ENTRIES) };
        const { assert!(MAX_STATUS_RECORDS <= MAX_BUFF_RECORDS) };
        const { assert!(MAX_CLASSES <= MAX_CLASS_CACHE) };
        const { assert!(MAX_NAME >= 32) };
    }
}
