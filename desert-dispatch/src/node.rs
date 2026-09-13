//! Pure logic for the parsed `FactionNodeInfo` records and the dispatch-mission
//! ("faction operation") sub-records inside them: the offsets, the sanity
//! bounds, the one-line rendering of an operation, and the accumulator that
//! turns a whole pass into a summary, a condition histogram and a verdict.
//!
//! Nothing here touches the game. It is a byte-offset table plus arithmetic
//! over values somebody else read, so it compiles and is unit tested natively
//! on Linux, which is the whole reason the judgement about whether the layout
//! looks right lives here rather than in `scan.rs`.
//!
//! Every offset comes from `docs/findings-dispatch-2026-09-10.md`, which
//! recovered them from the field-by-field record copy in `FUN_140ee53d0` and
//! corroborated them in the validator and the UI. They are **parsed-object**
//! offsets - the objects the game's record loader produced - not offsets into
//! the raw table bytes, the same distinction `desert_gatherer::hook`'s own
//! `mod parsed` makes.
//!
//! **Two of those offsets were wrong, and live data is what corrected them.**
//! Decompilation put the mission length at `entry+0xC0` and called the middle
//! of the three step-list fields a "cost". A dump of a vanilla table read
//! `entry+0xC0` as exactly `1` for all 936 missions, and read `step[1]+0x08`
//! as a spread of durations whose 47 values above 240 match, multiset for
//! multiset, the 47 originals DMM's "Long Mission to 24HR" lists as the
//! missions longer than 24 h. So `+0xC0` is a flag of some kind
//! ([`parsed::OP_UNKNOWN_C0`]) and the duration is
//! [`parsed::STEP_DURATION`], a `u32` in **tenths of an hour**. See
//! [`VANILLA_DISTRIBUTION`] for what a correct dump looks like, and
//! `docs/reference-internals.md` section 20.
//!
//! The **reward** half - [`parsed::dropset`], [`DropsetRow`], [`DropsetEntry`]
//! and [`RewardSummary`] - was one step further back until 2026-09-10: not one
//! of its offsets had ever been read at runtime. A vanilla capture of 219 rows
//! and 725 entries has now read all of them, and corrected three:
//!
//! 1. `entry+0x20` is not *the* amount. It is the amount **minimum**, paired
//!    with a **maximum** at `entry+0x28` - `min <= max` on 725/725, and the two
//!    differ on 136 of them (`(2,3)`, `(3,5)`, ...). It is the same shape as
//!    the `gimmickinfo` output block the gatherer multiplies. A multiplier that
//!    scaled only the minimum would leave `min > max` on those 136: a corrupt
//!    reward table, not a cosmetic bug.
//! 2. `kind == 13` entries are **not item drops**. The 34 of them are exactly
//!    the 34 entries whose amount is `0`, and exactly the 34 whose `entry+0x38`
//!    does not fit in a `u16` - which is also why that field is read as a
//!    `u32` now. See [`parsed::dropset::ENTRY_KIND_NOT_AN_ITEM`].
//! 3. The kind set is `{0, 1, 4, 6, 13}`, not the `{0, 1, 6, 9}` decompilation
//!    reported. There is no `9` anywhere in the capture.
//!
//! Four offsets in that half are still **UNCONFIRMED** and say so where they
//! are declared: each reads one constant value across the whole capture, which
//! is exactly the reading a wrong offset gives. The two halves still keep
//! separate accumulators and separate verdicts, because they are still two
//! hypotheses at different stages of test.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::config::{
    derived_amount, derived_duration, derived_min_operators, derived_skill_req, Levers,
};
use crate::remember::{Mission, Reward};

/// Offsets inside the objects the game parsed the `FactionNode` table into.
///
/// The manager pair is the same shape every static-info manager has, and is
/// the same one `desert_gatherer::hook::reapply` walks: a `u32` record count
/// and an array of record-object pointers, null where the game has not loaded
/// that record yet.
pub mod parsed {
    /// `u32` record count at `manager+0x08`, and the array of record-object
    /// pointers at `manager+0x58` - one `usize` per record, indexed by record
    /// index, null where the game has not parsed that record yet.
    ///
    /// Re-exported from [`desert_core::manager`] rather than declared again:
    /// the gatherer reads the same two offsets out of the same manager shape,
    /// and two copies of a stride is a table walked wrong in one subsystem and
    /// right in the other. The name is unchanged, so every `parsed::MGR_COUNT`
    /// in this crate still resolves.
    pub use desert_core::manager::{MGR_COUNT, MGR_RECORDS};

    /// `u32` FactionNodeKey, at `record+0x08`.
    pub const REC_KEY: usize = 0x08;
    /// Pointer to the record's operation entries, at `record+0xA0`.
    pub const REC_OPS: usize = 0xA0;
    /// Count of those entries, at `record+0xA8`.
    pub const REC_OP_COUNT: usize = 0xA8;

    /// Bytes per operation entry.
    pub const OP_STRIDE: usize = 0x120;
    /// Pointer to the entry's step list, at `entry+0x08`. Three steps, always,
    /// one per operation state (`cpp-operation-state-goto`, `-working`,
    /// `-leave`).
    pub const OP_STEPS: usize = 0x08;
    /// Count of those steps, at `entry+0x10`. Every vanilla entry reports 3.
    pub const OP_STEP_COUNT: usize = 0x10;
    /// Bytes per step.
    pub const STEP_STRIDE: usize = 0x30;
    /// `u32` **duration in tenths of an hour**, at `step+0x08`.
    ///
    /// Only the **middle** step - `step[1]`, the "working" state - carries it;
    /// steps 0 and 2 read `0` there. So the mission length is
    /// `steps[1] + STEP_DURATION`, not a sum: summing three steps of which two
    /// are zero happens to give the same number, but the shape of the field is
    /// a per-state duration and calling it a summed "cost" is what hid the
    /// meaning for a session.
    ///
    /// Decompilation called this a cost (`docs/findings-dispatch-2026-09-10.md`,
    /// and section 20.4 of the reference, where the tick subtracts it from an
    /// accumulator). It is a duration: see the module header for the DMM
    /// cross-check that settles it.
    pub const STEP_DURATION: usize = 0x08;
    /// Which step of the three carries it: the middle one, index `1`, the
    /// "working" state. Steps `0` (goto) and `2` (leave) read `0`.
    pub const DURATION_STEP: usize = 1;

    /// `u32` FactionOperationKey, at `entry+0x60`.
    pub const OP_KEY: usize = 0x60;
    /// `u32` FactionOperationGroupKey, at `entry+0x90`.
    pub const OP_GROUP: usize = 0x90;
    /// `u32` at `entry+0xC0`, meaning unknown. **Not the duration.**
    ///
    /// Decompilation read the completion test as `step >= [entry+0xC0]` and
    /// concluded this was the mission length in steps ("days"). A dump of a
    /// vanilla table reads it as exactly `1` for **every one of 936 missions**,
    /// so whatever it is - a step count that is always one, a kind byte
    /// widened to a word, a flag - it is not a duration and nothing should be
    /// scaled through it. The real duration is [`STEP_DURATION`].
    ///
    /// It is still read on every pass, and still reported on the mission line
    /// as `c0=`, precisely because it is constant today: a field that starts
    /// varying after a game update is worth seeing.
    pub const OP_UNKNOWN_C0: usize = 0xC0;
    /// `u32` minimum operator count, at `entry+0xC4`. The hard gate - and
    /// `0` is *stricter* than `1`, not laxer.
    pub const OP_MIN_OPERATORS: usize = 0xC4;
    /// `u32` maximum operator count, at `entry+0xC8`. Display only.
    pub const OP_MAX_OPERATORS: usize = 0xC8;
    /// `u32` standard combat power, at `entry+0xCC`. The success-ratio divisor.
    pub const OP_COMBAT_POWER: usize = 0xCC;

    /// `u16` work-stat index at `entry+0xD8`: the *skill category* a mission
    /// pays a bonus for. [`SKILL_NONE`] means none; 231 of 936 vanilla missions
    /// set it.
    ///
    /// This is the **soft** half of the pair. `FUN_1416F0750` multiplies the
    /// best operator's level in this stat by the per-point value at
    /// `entry+0xE0` and adds it to the payout. It gates nothing: a mission with
    /// this set and no [`OP_SKILL_REQ`] starts for anybody, and vanilla has
    /// several (Mining, Ranching).
    ///
    /// **A write path must never clear this alongside [`OP_SKILL_REQ`].** The
    /// two fields are two bytes apart and trivial to confuse, and blanking this
    /// one silently deletes a reward bonus nobody asked to lose - with no error
    /// and nothing in the UI to show it happened.
    pub const OP_SKILL_STAT: usize = 0xD8;
    /// `i16` required `Skill`-table index, at `entry+0xDA`. [`SKILL_NONE`]
    /// (`-1`) means no requirement; 147 of 936 vanilla missions set one, over
    /// 13 distinct values.
    ///
    /// This is the **hard gate**, and it is independent of [`OP_SKILL_STAT`]:
    /// `FUN_1416F09A0` returns false unless some assigned operator holds this
    /// skill, and both operation-start validators turn that into
    /// `eErrNoWorkStatNotEnough` and refuse to start the mission.
    ///
    /// **`0` is stricter than "none", not laxer - [`OP_MIN_OPERATORS`]'s trap
    /// exactly.** The predicate tests `== -1`, and `0` is a real `Skill` index
    /// that no worker has, so writing `0` here makes the mission
    /// **permanently unstartable**. To clear a requirement, write
    /// [`SKILL_NONE`] and nothing else. DMM's "No Skill Requirement" writes
    /// `0`, which is right in the *raw* table - there `0` means "no key" and
    /// the deserialiser resolves it to `-1` - and catastrophic in the parsed
    /// record this subsystem reads. The raw and parsed encodings of "absent"
    /// are different values; that is the single likeliest way to get a future
    /// write path here wrong.
    pub const OP_SKILL_REQ: usize = 0xDA;
    /// The "no skill" sentinel both fields carry: `0xFFFF`, which is the `-1`
    /// the game's own `i16` comparison tests for.
    pub const SKILL_NONE: u16 = 0xFFFF;

    /// Pointer to the entry's condition list, at `entry+0xF8`.
    pub const OP_CONDS: usize = 0xF8;
    /// Count of those conditions, at `entry+0x100`.
    pub const OP_COND_COUNT: usize = 0x100;
    /// Bytes per condition entry:
    /// `{u16 conditioninfo key, u16 showFlag, u16 failReasonStringKey, u16}`.
    pub const COND_STRIDE: usize = 8;
    /// Offsets inside one condition entry.
    pub const COND_KEY: usize = 0x00;
    pub const COND_SHOW: usize = 0x02;
    pub const COND_FAIL: usize = 0x04;

    /// `u16` `dropsetinfo` row index, reward set 1, at `entry+0xA8`.
    /// [`REWARD_NONE`] means the mission pays no items from that set.
    ///
    /// The index is the **manager row directly**: the game's accessor is
    /// `if (key < mgr[+0x08]) rec = ((void**)mgr[+0x58])[key]`, with no hash
    /// step in between, which is why the reward pass needs neither a loader
    /// hook nor a key table - see `docs/reference-internals.md` section 20.17.
    pub const OP_REWARD_1: usize = 0xA8;
    /// `u16` `dropsetinfo` row index, reward set 2, at `entry+0xAA`.
    pub const OP_REWARD_2: usize = 0xAA;
    /// The "no row" sentinel both reward-set fields carry.
    pub const REWARD_NONE: u16 = 0xFFFF;

    /// Offsets inside the objects the game parsed the **`dropsetinfo`** table
    /// into - the reward rows [`OP_REWARD_1`] and [`OP_REWARD_2`] name.
    ///
    /// The manager pair is the same shape as every other static-info manager's
    /// and is not repeated here: the reward pass reads the dropset manager
    /// through [`super::MGR_COUNT`] and [`super::MGR_RECORDS`].
    ///
    /// **Every offset below was decompilation-only until 2026-09-10**, when a
    /// vanilla capture of 219 rows and 725 entries read all of them at once.
    /// Each one now says what that capture saw, and the four that read a single
    /// constant value across every sample are marked **UNCONFIRMED**: a field
    /// that never varies is the reading a wrong offset gives, and nothing may
    /// be built on one until something varies. See the module header for the
    /// three corrections the capture forced.
    pub mod dropset {
        /// `i32` claimed draw count, at `record+0x20`: how many entries one
        /// roll pays. **UNCONFIRMED** - `0` on all 219 rows of the live
        /// capture, and a table where every row draws nothing pays nothing, so
        /// a wrong offset is the likelier reading of the two.
        pub const ROW_DRAWS: usize = 0x20;
        /// Pointer to an **array of entry pointers** at `record+0x28`, stride
        /// [`ENTRY_PTR_STRIDE`]. Not an array of inline entries: an entry is
        /// reached as `((void**)*(void**)(rec + 0x28))[i]`, one dereference
        /// more than the operation list needs. CONFIRMED - 219 distinct arrays,
        /// every one of them plausible.
        pub const ROW_ENTRIES: usize = 0x28;
        /// `u32` count of those entries, at `record+0x30`. CONFIRMED - `1..60`
        /// over 18 distinct values in the live capture.
        pub const ROW_ENTRY_COUNT: usize = 0x30;
        /// `i64` cached **weight total** at `record+0x48`: it equals the sum
        /// of that row's entry [`ENTRY_WEIGHT`]s on **219/219** rows of the
        /// live capture, exactly.
        ///
        /// **This is a free integrity check for the write path**, and it is
        /// worth more than its size suggests. Summing the weights is a loop
        /// the pass is already doing, and a row whose entries do not add up to
        /// this is not the record the pass thinks it is walking - a stale
        /// pointer, a row that moved under the reader, or an offset a game
        /// update shifted. Caught before a single byte is written, at the cost
        /// of nothing.
        ///
        /// The corollary is a rule: anything that ever edits [`ENTRY_WEIGHT`]
        /// has to recompute this. The reward multiplier scales amounts and
        /// never touches a weight, so it leaves the invariant intact by
        /// construction - which is exactly why it is allowed to assert it.
        /// See `docs/reference-internals.md` section 20.22.
        pub const ROW_WEIGHT_SUM: usize = 0x48;
        /// `i64` claimed "no drop" chance in parts per million, at
        /// `record+0x50`. **UNCONFIRMED** - `0` on all 219 rows.
        pub const ROW_NO_DROP_PPM: usize = 0x50;
        /// Bytes per element of the pointer array at [`ROW_ENTRIES`].
        pub const ENTRY_PTR_STRIDE: usize = 8;

        /// The three `u16` condition keys, at `entry+0x04/+0x06/+0x08`.
        /// [`COND_NONE`] where the slot is unused.
        ///
        /// Only the **middle** one is confirmed: `+0x06` carries 20 distinct
        /// values and reads [`COND_NONE`] on 685 of 725 entries, which is what
        /// a real, mostly-unused key field looks like. `+0x04` and `+0x08` are
        /// **UNCONFIRMED** - both read `0xFFFF` on all 725, so all that is
        /// known about them is that they are not obviously anything else.
        pub const ENTRY_CONDS: [usize; 3] = [0x04, 0x06, 0x08];
        /// The "no condition" sentinel those three carry.
        pub const COND_NONE: u16 = 0xFFFF;
        /// `i64` roll weight, at `entry+0x10`. CONFIRMED - `1000..1000000`
        /// over 31 distinct values.
        pub const ENTRY_WEIGHT: usize = 0x10;
        /// `i64` amount **minimum** - the fewest of the item an entry pays -
        /// at `entry+0x20`. CONFIRMED, and it is half of a pair: see
        /// [`ENTRY_AMOUNT_MAX`].
        pub const ENTRY_AMOUNT_MIN: usize = 0x20;
        /// `i64` amount **maximum**, at `entry+0x28`. CONFIRMED:
        /// `min <= max` on 725/725 entries of the live capture, and the two
        /// differ on 136 of them.
        ///
        /// Decompilation reported a single `AMOUNT` at [`ENTRY_AMOUNT_MIN`] and
        /// that is what the first build of this subsystem read. The pair is the
        /// same shape as the `gimmickinfo` output block's `MIN_AT`/`MAX_AT`
        /// that `desert_gatherer` already multiplies, and a reward multiplier
        /// here has to be the same shape too: **scale both, remember both**.
        /// Scaling only the minimum yields `min > max` on 136 entries, which is
        /// a corrupt table rather than a wrong number.
        pub const ENTRY_AMOUNT_MAX: usize = 0x28;
        /// `u32` `iteminfo` row of the item paid, at `entry+0x38`. CONFIRMED -
        /// 421 distinct values, and it is a `u32` rather than the `u16`
        /// decompilation implied: the 34 [`ENTRY_KIND_NOT_AN_ITEM`] entries
        /// read values above `0xFFFF` there, which a `u16` read silently
        /// truncates. Every entry that *is* an item drop fits in a `u16`
        /// (max 6868).
        pub const ENTRY_ITEM: usize = 0x38;
        /// `u8` entry kind, at `entry+0x60`. CONFIRMED, with a live census of
        /// `{0: 633, 6: 35, 13: 34, 4: 16, 1: 7}` over 725 entries.
        ///
        /// Decompilation claimed `{0, 1, 6, 9}`; there is no `9` in the
        /// capture, and the `4` and `13` it did not predict are both real. What
        /// the values mean is still unknown, so they are reported and not
        /// judged - except for `13`, which is [`ENTRY_KIND_NOT_AN_ITEM`].
        pub const ENTRY_KIND: usize = 0x60;
        /// The one [`ENTRY_KIND`] value that is **not an item drop**, and the
        /// skip rule any future multiplier owes it.
        ///
        /// The 34 entries with this kind are exactly the 34 whose amount is
        /// `0`, and exactly the 34 whose [`ENTRY_ITEM`] does not fit in a
        /// `u16` (same set, verified over the whole capture). Whatever they
        /// are, they are not a quantity of an item.
        ///
        /// **A reward multiplier must skip them**: scaling `0` pays nothing
        /// either way, and the `+0x38` value they carry is not an item row, so
        /// anything that treats them as drops is acting on a field it has
        /// mis-parsed. Excluding them, the remaining 691 entries are clean -
        /// every amount at least 1, every item row inside `u16`.
        pub const ENTRY_KIND_NOT_AN_ITEM: u8 = 13;

        /// How many bytes of a dropset record head the raw dump prints. Wide
        /// enough to cover every named offset above with room after the last
        /// of them, because the point of the raw line is the bytes nobody has
        /// named yet.
        pub const ROW_RAW: usize = 0x60;
        /// How many bytes of one entry the raw dump prints, same reasoning -
        /// [`ENTRY_KIND`] is the last named field and it is at `+0x60`.
        pub const ENTRY_RAW: usize = 0x70;
    }
}

/// The two globals that decide whether a completed mission ever banks anything
/// into the save - read **once**, read-only, and the one place this subsystem
/// names an address instead of finding it by content.
///
/// Everything else here is resolved from bytes, because a signature survives a
/// rebuild and an address does not (the 0.2 rung of `VERSIONING.md`). These two
/// are data, not code: two bytes of the exe's zero-initialised data, named only
/// by the instructions that test them, with nothing around them to match on.
/// A build that moved them makes the read return a meaningless byte or nothing
/// at all, and because **nothing acts on either value**, the whole cost of that
/// is one wrong log line.
///
/// They were read to settle whether the `AnyOperatorCount` warning describes
/// anything that happens. **Measured on build 25116796, 2026-09-11: both read
/// `0`** - no mission defers its payout and the stored percent ignores the
/// worker count, so nothing any lever here does reaches the save. The warning
/// was therefore *downgraded, not deleted*: the code path exists, this is one
/// launch of one build, and `0x6BA07C8`'s own initialiser writes `1` (something
/// zeroes it before the game runs, see section 20.18), so neither value is safe
/// to assume. They stay read, and the `[banking]` line is how a game update
/// that flips one gets noticed.
pub mod globals {
    /// `DAT_146BC6AA8`, tested as `cmp byte ptr [0x146bc6aa8],0` at RVA
    /// `0x2781B08`, whose `je` skips the deferred-reward branch entirely.
    /// **Zero means no mission ever defers a payout**: nothing is pushed to
    /// `_factionStoredOperationRewardList`, nothing is banked in the save, and
    /// nothing this plugin writes can outlive it. Non-zero means the 142
    /// repeating missions can bank a percent (section 20.18).
    pub const DEFERRED_REWARD_GATE_RVA: usize = 0x6BC6AA8;
    /// `DAT_146BA07C8`, tested as `cmp byte ptr [0x146ba07c8],bl` at RVA
    /// `0x27819B3`, whose `jz` skips the surplus-worker term. Zero means the
    /// stored percent is the flat 1,000,000 base and `entry+0xC4` - the field
    /// `AnyOperatorCount` rewrites to `1` - never enters the arithmetic. The
    /// same byte is the accrual-rate flag the timing tick branches on, where
    /// non-zero means the flat rate and the worker count does not affect
    /// progress (section 20.4).
    pub const SURPLUS_WORKER_GATE_RVA: usize = 0x6BA07C8;

    /// The one startup line those two bytes are worth, in plain words. `None`
    /// is a read that did not come back - the page is zero-initialised data and
    /// may not be mapped at all this early.
    ///
    /// Pure, so the sentence a future reader will have to trust is unit tested
    /// natively rather than proof-read in a log.
    pub fn banking_gate_line(deferred: Option<u8>, surplus: Option<u8>) -> String {
        let say = |v: Option<u8>, zero: &str, set: &str| match v {
            None => "would not read".to_string(),
            Some(0) => format!("0 - {zero}"),
            Some(n) => format!("{n} - {set}"),
        };
        let verdict = match (deferred, surplus) {
            (Some(0), _) => {
                "on this launch nothing is ever banked: no completed mission defers a reward, so \
                 AnyOperatorCount cannot outlive the plugin"
            }
            (Some(_), Some(0)) => {
                "on this launch a reward can be deferred, but the stored percent does not carry \
                 the surplus-worker term, so AnyOperatorCount does not inflate it"
            }
            (Some(_), Some(_)) => {
                "on this launch both gates are open: a repeating mission completed under \
                 AnyOperatorCount=1 can bank an inflated percent that pays out later, even after \
                 the plugin is removed"
            }
            _ => "one of the two gates would not read, so the warning in the README stands as it is",
        };
        format!(
            "[banking] deferred-reward gate (0x{:X}) = {}; surplus-worker gate (0x{:X}) = {}; \
             {verdict}. Both are read-only and nothing here acts on either.",
            DEFERRED_REWARD_GATE_RVA,
            say(
                deferred,
                "no mission defers its payout, so nothing is stored in the save",
                "repeating missions can defer a payout into the save",
            ),
            SURPLUS_WORKER_GATE_RVA,
            say(
                surplus,
                "the stored percent ignores the worker count, so entry+0xC4 never enters it",
                "the stored percent carries the surplus-worker term that entry+0xC4 feeds",
            ),
        )
    }
}

/// Largest record count this pass will walk. The real table is 1119 records,
/// and the bound is what keeps a misread `u32` from turning into an hour of
/// guarded reads inside the game's process.
pub const MAX_RECORDS: usize = 4_096;
/// Largest operation count read out of one record. Vanilla spreads 936
/// missions over 1119 records, a handful each.
pub const MAX_OPS: usize = 256;
/// Largest number of operation entries **one whole pass** will read, across
/// every record.
///
/// [`MAX_RECORDS`] times [`MAX_OPS`] is a million entries, each costing a
/// dozen guarded reads and a pointer chase or two, so the product of the two
/// caps is not a bound worth having: a single misread `u32` would wedge this
/// subsystem's thread for the rest of the session. This is the bound that
/// actually holds, and a pass that reaches it stops walking and says so.
pub const MAX_TOTAL_OPS: usize = 8_192;
/// Largest condition count read out of one operation.
pub const MAX_CONDS: usize = 64;
/// Largest step count read out of one operation. Every vanilla entry carries
/// exactly three, one per operation state, so this is already generous.
pub const MAX_STEPS: usize = 8;
/// Largest number of distinct `dropsetinfo` rows the reward pass will read in
/// one go. A vanilla table names 219; the cap is what stops a misread reward
/// index from turning the pass into a walk of the game-wide drop table.
pub const MAX_DROP_ROWS: usize = 4_096;
/// Largest entry count **read** out of one `dropsetinfo` row, by the read-only
/// dump.
///
/// Deliberately four times [`DROP_ENTRIES_HI`], and the two are different kinds
/// of number rather than a disagreement. This one is a resource cap, the
/// reward-row twin of [`MAX_OPS`]: how many entries one row is worth reading
/// before the pass decides a count is being misread. `DROP_ENTRIES_HI` is an
/// *identity* band - how many entries a real reward row has - and the write path
/// refuses a row outside it rather than capping it. So the dump can print a row
/// of 100 entries that the apply pass will not touch, and that is the intended
/// behaviour on both sides: the dump's job is to show what is there, including
/// the rows a write would refuse, and `RewardCounts::skip_shape` is what says
/// out loud that a row was refused for its count (see `crate::apply::read_row`).
pub const MAX_DROP_ENTRIES: usize = 256;

/// A reward amount outside this band is not a quantity of items and says
/// `entry+0x20` / `entry+0x28` are the wrong offsets. The ceiling is
/// deliberately loose - some rows pay currency - and is still four orders of
/// magnitude below the smallest value a misread pointer half produces, which is
/// what the check is actually for.
///
/// The floor is 1, but **not** because "an entry that pays nothing would not be
/// an entry": 34 of the live capture's 725 entries pay exactly `0`, and they
/// are precisely the [`parsed::dropset::ENTRY_KIND_NOT_AN_ITEM`] entries, which
/// are not item drops at all. They fall out of band on purpose - being visible
/// is the whole point of them - and they are why a correct vanilla dump reads
/// about 95% here rather than 100%.
pub const AMOUNT_LO: i64 = 1;
pub const AMOUNT_HI: i64 = 1_000_000;
/// How many entries one `dropsetinfo` row plausibly holds. A row with none is
/// not a reward row, and a row with hundreds says `record+0x30` is not a
/// count. The live capture's widest row holds 60.
///
/// This is an identity band, not a resource cap, and it is **narrower** than
/// [`MAX_DROP_ENTRIES`] on purpose - that constant says why.
pub const DROP_ENTRIES_LO: usize = 1;
pub const DROP_ENTRIES_HI: usize = 64;

/// The declared entry count of a `dropsetinfo` row, as a count the write path
/// may walk, or `None` when it is outside [`DROP_ENTRIES_LO`]..=[`DROP_ENTRIES_HI`].
///
/// Pure, and separated out so that "the count moved" and "the row would not
/// read" are two different answers rather than one: `crate::apply::read_row`
/// used to return `None` for both, which charged a moved count to the
/// "unreadable" counter and left the `skip_shape` counter - the one a game
/// update would move - unable to fire at all.
pub fn drop_entry_count(declared: u32) -> Option<usize> {
    let n = declared as usize;
    (DROP_ENTRIES_LO..=DROP_ENTRIES_HI).contains(&n).then_some(n)
}

/// Add one `dropsetinfo` row to the set of rows a pass will act on, **capped at
/// [`MAX_DROP_ROWS`] here, where the row is inserted**. `true` when the row is
/// in the set afterwards.
///
/// The cap has to bite at insert and not at read. The set only ever grows, so
/// taking the first `MAX_DROP_ROWS` of it at read time takes the numerically
/// smallest rows *of that moment*: a smaller row index turning up later would
/// push an already-multiplied larger row out of the window, and nothing would
/// ever revert it. Capping here makes the acted-on set stable - a row that got
/// in stays in - which is the property a revert depends on. (219 rows on build
/// 25116796, so nothing is anywhere near this.)
pub fn insert_capped(set: &mut BTreeSet<u16>, row: u16) -> bool {
    if set.contains(&row) {
        return true;
    }
    if set.len() >= MAX_DROP_ROWS {
        return false;
    }
    set.insert(row);
    true
}

/// The reward census of a vanilla table on build 25116796, confirmed in game
/// over the 936 distinct operations, so a fresh dump can be judged at a
/// glance. `+0xAC` carries a third reward set on 18 operations naming a single
/// row (6244); this pass deliberately does not request it, because route A of
/// section 20.17 is scoped to the two sets below and a row set that grows by
/// accident is exactly how a dispatch mod becomes a global loot mod.
pub const REWARD_OPS_SET1: usize = 243;
/// Distinct rows those [`REWARD_OPS_SET1`] operations name.
pub const REWARD_ROWS_SET1: usize = 208;
/// Operations whose `+0xAA` names a row.
pub const REWARD_OPS_SET2: usize = 18;
/// Distinct rows those [`REWARD_OPS_SET2`] operations name.
pub const REWARD_ROWS_SET2: usize = 13;
/// Distinct rows in the **union** of the two sets: the row set a reward
/// multiplier would ever touch. Less than 208 + 13 because the sets overlap.
pub const REWARD_ROWS_UNION: usize = 219;

/// A duration outside this band is not a mission length and says the offset is
/// wrong. The unit is **tenths of an hour**, so the band is 1.0 h to 432.0 h:
/// vanilla runs `20..960` (2.0 h to 96.0 h) and this is that with a wide
/// margin on both sides for a mod that shortened or lengthened things - DMM
/// ships one of each.
pub const DURATION_LO: u32 = 10;
pub const DURATION_HI: u32 = 4_320;

/// What a vanilla dump of build 25116796 looks like, so a future reader can
/// tell at a glance whether a fresh one is right.
///
/// 936 missions. Every duration is a multiple of 10 (a whole number of hours),
/// the range is `20..=960` (2.0 h to 96.0 h) and the three commonest values
/// are `160` (16 h, 298 missions), `120` (12 h, 237) and `80` (8 h, 196).
///
/// Exactly **47** missions read above `240` (24 h), and that is the
/// cross-check that settles the offset rather than merely making it plausible:
/// DMM's mod "Long Mission to 24HR" says it shortens "every dispatch mission
/// and activity longer than 24h (47 missions)" and lists 47 byte patches with
/// their original `u16` values. The 47 read out of `step[1]+0x08` are the same
/// multiset, multiplicities included:
///
/// | tenths | hours | missions |
/// | --- | --- | --- |
/// | 540 | 54.0 h | 19 |
/// | 720 | 72.0 h | 9 |
/// | 360 | 36.0 h | 7 |
/// | 480 | 48.0 h | 7 |
/// | 300 | 30.0 h | 2 |
/// | 960 | 96.0 h | 2 |
/// | 900 | 90.0 h | 1 |
///
/// `entry+0xC0` read `1` on all 936 - see [`parsed::OP_UNKNOWN_C0`].
pub const VANILLA_DISTRIBUTION: &str =
    "936 missions, 20..960 tenths of an hour, all multiples of 10; commonest 160 (298), \
     120 (237), 80 (196); 47 above 240, matching DMM's 47-value list";
/// Likewise for the minimum operator count. `0` is legal (and means something
/// specific - see [`parsed::OP_MIN_OPERATORS`]) so the low end is 0, not 1.
pub const OPERATORS_LO: u32 = 0;
pub const OPERATORS_HI: u32 = 64;

/// Fraction of both samples that must land in range for the layout to be
/// called right, and the fraction below which it is called wrong. Between the
/// two the verdict refuses to commit, which is the honest answer for, say, one
/// field having moved and the other not.
const VERDICT_GOOD: f64 = 0.90;
const VERDICT_BAD: f64 = 0.50;

/// Is `p` an address that could be a live heap or image pointer?
///
/// [`desert_core::manager::plausible`] under this crate's own name, which is
/// what it always was: the gatherer's copy was byte-identical, and a
/// plausibility bound that differed between two subsystems walking the same
/// managers would be a bug in whichever one was looser. Re-exported rather than
/// re-spelled so the ~30 call sites in this crate read as they did.
pub use desert_core::manager::plausible;

/// The order the two halves of a reward amount pair must be written in, as
/// `[(offset, value), (offset, value)]` relative to the entry.
///
/// `WriteProcessMemory` writes one scalar at a time and a game thread can roll
/// a reward between the two calls, so *some* order is always observed. One of
/// the two orders is always safe and which one depends on where the pair is
/// going, not on which lever moved:
///
/// * **The ceiling is rising** (`want_max > cur_max`): write the maximum
///   first. Afterwards `cur_min <= cur_max < want_max` holds, so the window
///   between the two writes shows `cur_min..want_max` - wider than either end
///   state, never inverted - and the second write raises the floor into it.
/// * **Otherwise** (`want_max <= cur_max`): write the minimum first.
///   `want_min <= want_max <= cur_max` holds, so the window shows
///   `want_min..cur_max`, again wider rather than inverted, and the second
///   write lowers the ceiling.
///
/// Both cases assume only `cur_min <= cur_max` and `want_min <= want_max`,
/// which `apply::reward_vanilla`'s band check and `Levers::amount`'s single
/// non-negative factor respectively guarantee.
///
/// **The rule cannot be keyed on the lever's direction**, which is why it is
/// keyed on the field. A previous pass that was refused the maximum leaves
/// `(vanilla_min * R_old, vanilla_max)`, and a *smaller* `R_new` then lowers
/// the minimum while raising the maximum - a mixed move under a shrinking
/// lever. `cur_max` is the only thing that answers it.
pub fn amount_write_order(
    cur_max: i64,
    want_min: i64,
    want_max: i64,
) -> [(usize, i64); 2] {
    use parsed::dropset::{ENTRY_AMOUNT_MAX, ENTRY_AMOUNT_MIN};
    if want_max > cur_max {
        [(ENTRY_AMOUNT_MAX, want_max), (ENTRY_AMOUNT_MIN, want_min)]
    } else {
        [(ENTRY_AMOUNT_MIN, want_min), (ENTRY_AMOUNT_MAX, want_max)]
    }
}

// ---------------------------------------------------------------------------
// The write decisions
// ---------------------------------------------------------------------------
//
// What a pass should write, decided over plain values: the remembered vanilla,
// what the field reads now, and the levers. No address, no `safe` call and
// nothing `#[cfg(windows)]`, which is the whole point - `crate::apply` is
// windows-only, so until these moved here not one band check, `derived_*`
// predicate or `want == 0` guard was exercised by `cargo test --target
// x86_64-unknown-linux-gnu`. `apply` is now reads -> plan -> writes ->
// counters, and every one of those decisions is a native unit test.

/// What one mission entry should have written into it.
///
/// A `None` field means "write nothing", and the two reasons for that are not
/// the same thing: the field already says what it should, or what is in it is
/// not ours to write. [`Self::refused`] counts the second, because an entry
/// every field of which was refused must be reported as skipped rather than
/// unchanged ([`crate::apply::MissionCounts::skipped`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MissionPlan {
    /// `steps[1]+0x08`, in tenths of an hour.
    pub duration: Option<u32>,
    /// `entry+0xDA`. Never `0` - see [`crate::config::Levers::skill_req`].
    pub skill: Option<u16>,
    /// `entry+0xC4`. Never `0` - see [`crate::config::Levers::min_operators`].
    pub min_operators: Option<u32>,
    /// Fields refused because what is in them now is neither the remembered
    /// vanilla nor anything this plugin could have written from it. At most 3.
    pub refused: u8,
}

impl MissionPlan {
    /// How many fields this plan asks for. Zero with a non-zero
    /// [`Self::refused`] is the "wholly refused" reading.
    pub fn writes(&self) -> usize {
        usize::from(self.duration.is_some())
            + usize::from(self.skill.is_some())
            + usize::from(self.min_operators.is_some())
    }
}

/// Decide what to write into one operation entry.
///
/// `van` is what [`crate::remember`] holds for the mission - the values the game
/// parsed, from the first pass that ever saw it - and `cur` is what the three
/// fields read right now. Every value written is computed from `van` and never
/// from `cur`, which is what makes a second pass idempotent and a revert exact.
///
/// The two `want == 0` guards are the ones worth reading twice. Neither
/// [`Levers::skill_req`] nor [`Levers::min_operators`] can invent a `0` - the
/// first returns the vanilla or [`parsed::SKILL_NONE`], the second the vanilla
/// or `1` - but a `0` in either field is **stricter** than the value it replaces
/// (`docs/reference-internals.md` section 20.16: a `0` skill index makes a
/// mission permanently unstartable, a `0` minimum commits every worker the node
/// has), so it is refused a second time here. The `want != van` half of each
/// guard is what keeps a **revert** possible: with the lever off, `want` *is*
/// the vanilla, and a mission whose vanilla value were `0` would otherwise be
/// refused the one value that is correct. No vanilla mission on build 25116796
/// is in that position, but `0` is a legal `Skill` index and `OPERATORS_LO` is
/// `0`, and a guard that blocks a revert is worse than no guard.
pub fn plan_mission(van: Mission, cur: Mission, lev: &Levers) -> MissionPlan {
    let mut plan = MissionPlan::default();

    // 1. Duration: steps[1] + 0x08, u32, tenths of an hour.
    let want = lev.duration(van.duration_tenths);
    if want != cur.duration_tenths {
        if derived_duration(van.duration_tenths, cur.duration_tenths) {
            plan.duration = Some(want);
        } else {
            plan.refused += 1;
        }
    }

    // 2. Required skill: entry + 0xDA, u16, 0xFFFF to clear. `entry + 0xD8`,
    //    two bytes below it, is the reward-bonus stat and is never touched.
    let want = lev.skill_req(van.skill_req);
    if want != cur.skill_req {
        if (want == 0 && want != van.skill_req) || !derived_skill_req(van.skill_req, cur.skill_req) {
            plan.refused += 1;
        } else {
            plan.skill = Some(want);
        }
    }

    // 3. Minimum operators: entry + 0xC4, u32, 1 to open up.
    let want = lev.min_operators(van.min_operators);
    if want != cur.min_operators {
        if (want == 0 && want != van.min_operators)
            || !derived_min_operators(van.min_operators, cur.min_operators)
        {
            plan.refused += 1;
        } else {
            plan.min_operators = Some(want);
        }
    }

    plan
}

/// Could `now` be the **vanilla** reading of a mission record?
///
/// The band check a **first sight** gets and no later sight does: on a first
/// sight the values being read are the ones the game parsed, so they have to
/// look like it, and on every later pass the duration may already be a quarter
/// of what it was - which is the point. Out of band means the offset is not the
/// field, so nothing is remembered and nothing is written.
pub fn vanilla_mission_in_band(now: Mission) -> bool {
    (DURATION_LO..=DURATION_HI).contains(&now.duration_tenths)
        && (OPERATORS_LO..=OPERATORS_HI).contains(&now.min_operators)
}

/// The same for a reward entry: both halves inside the amount band, and in
/// order. `min <= max` holds on 725/725 vanilla entries, and a pair that is not
/// ordered is not a pair of amounts.
pub fn vanilla_reward_in_band(now: Reward) -> bool {
    let band = AMOUNT_LO..=AMOUNT_HI;
    band.contains(&now.amount_min)
        && band.contains(&now.amount_max)
        && now.amount_min <= now.amount_max
}

/// What one `dropsetinfo` reward entry should have written into it - one
/// answer for the pair, because the two halves are never written apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewardPlan {
    /// [`parsed::dropset::ENTRY_KIND_NOT_AN_ITEM`]: 34 of 725 vanilla entries,
    /// exactly the ones paying `0` and exactly the ones whose `+0x38` is not an
    /// item row. Not a failure, and not a thing to scale.
    NotAnItemDrop,
    /// The entry is not paying the item it was paying when its vanilla amounts
    /// were recorded, so this is not the entry that was remembered.
    ItemMismatch,
    /// Both halves already say what they should.
    Unchanged,
    /// What is in the pair now is not something this plugin could have written
    /// from the remembered vanilla.
    Refused,
    /// Write both halves. The *order* is [`amount_write_order`]'s, which needs
    /// the entry's current maximum as well and so stays at the write site.
    Write {
        want_min: i64,
        want_max: i64,
    },
}

/// Decide what to write into one reward entry.
///
/// Both halves or neither, always: `min <= max` on 725/725 vanilla entries and
/// the two differ on 136 of them, so writing one alone would leave those 136
/// inverted - a corruption of the game's own drop table rather than a bigger
/// reward.
///
/// The `kind` and item-row checks are here rather than only at the call site so
/// that this function is total over any entry: what [`crate::apply`] must still
/// do in its own order is refuse a non-item-drop **before** remembering it, as
/// a `kind == 13` entry's `0` amounts are out of band on purpose and would
/// otherwise be reported as a vanilla value that looks wrong.
pub fn plan_entry(van: Reward, cur: &DropsetEntry, lev: &Levers) -> RewardPlan {
    if !cur.is_item_drop() {
        return RewardPlan::NotAnItemDrop;
    }
    if van.item_row != cur.item_row {
        return RewardPlan::ItemMismatch;
    }
    let want_min = lev.amount(van.amount_min);
    let want_max = lev.amount(van.amount_max);
    if want_min == cur.amount_min && want_max == cur.amount_max {
        return RewardPlan::Unchanged;
    }
    if !derived_amount(van.amount_min, cur.amount_min)
        || !derived_amount(van.amount_max, cur.amount_max)
    {
        return RewardPlan::Refused;
    }
    RewardPlan::Write { want_min, want_max }
}

/// One entry of an operation's condition list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cond {
    /// The `conditioninfo` record key. **This is the open question** the whole
    /// subsystem exists to answer: which keys actually appear.
    pub key: u16,
    /// Whether the UI shows the condition on the mission's row.
    pub show: u16,
    /// String key of the "why you cannot start this" message.
    pub fail: u16,
}

/// One dispatch mission, as read out of one 0x120-byte operation entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// The `FactionNodeInfo` record this entry sits inside.
    pub node_key: u32,
    /// Its index within that record's entry array.
    pub index: usize,
    pub key: u32,
    pub group: u32,
    /// How long the mission takes, in **tenths of an hour**, out of the middle
    /// step of the step list ([`parsed::STEP_DURATION`]). `0` when the step
    /// list was short or unreadable.
    pub duration_tenths: u32,
    pub min_operators: u32,
    pub max_operators: u32,
    pub combat_power: u32,
    /// Number of entries in the step list. Three, on every vanilla entry.
    pub steps: usize,
    /// Whatever `entry+0xC0` holds ([`parsed::OP_UNKNOWN_C0`]). Read and
    /// reported, never interpreted: it is `1` on every vanilla mission.
    pub flag_c0: u32,
    /// The `dropsetinfo` row this mission's reward set 1 names
    /// ([`parsed::OP_REWARD_1`]), or [`parsed::REWARD_NONE`].
    pub reward1: u16,
    /// The same for reward set 2 ([`parsed::OP_REWARD_2`]).
    pub reward2: u16,
    /// The `Skill` index this mission **requires** ([`parsed::OP_SKILL_REQ`]),
    /// or [`parsed::SKILL_NONE`]. The hard gate.
    pub skill_req: u16,
    /// The work-stat index this mission pays a **bonus** for
    /// ([`parsed::OP_SKILL_STAT`]), or [`parsed::SKILL_NONE`]. Independent of
    /// [`Self::skill_req`]: vanilla missions carry either, both or neither.
    pub skill_stat: u16,
    pub conds: Vec<Cond>,
}

/// The scalar fields of one 0x120-byte operation entry, decoded from the
/// entry's own bytes rather than read one guarded field at a time.
///
/// This is what makes the offsets testable natively: `scan.rs` reads the entry
/// as one block through `safe::read_into` and hands the bytes to
/// [`decode_operation`], so the unit tests exercise the same decode the game
/// process does, over bytes laid out the way the live capture found them. A
/// test that restated the constants instead would pass however wrong they were.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpFields {
    pub key: u32,
    pub group: u32,
    pub flag_c0: u32,
    pub min_operators: u32,
    pub max_operators: u32,
    pub combat_power: u32,
    pub reward1: u16,
    pub reward2: u16,
    pub skill_req: u16,
    pub skill_stat: u16,
}

fn le_u8(b: &[u8], off: usize) -> Option<u8> {
    b.get(off).copied()
}

fn le_u16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off.checked_add(2)?)?.try_into().ok()?))
}

fn le_u32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off.checked_add(4)?)?.try_into().ok()?))
}

fn le_i64(b: &[u8], off: usize) -> Option<i64> {
    Some(i64::from_le_bytes(b.get(off..off.checked_add(8)?)?.try_into().ok()?))
}

/// Decode the scalar fields of one operation entry out of its bytes, or `None`
/// if the buffer is too short to hold the last of them.
///
/// The step list and the condition list are not here: both are pointers the
/// entry only names, so following them needs a live process and belongs in
/// `scan.rs`.
pub fn decode_operation(b: &[u8]) -> Option<OpFields> {
    Some(OpFields {
        key: le_u32(b, parsed::OP_KEY)?,
        group: le_u32(b, parsed::OP_GROUP)?,
        flag_c0: le_u32(b, parsed::OP_UNKNOWN_C0)?,
        min_operators: le_u32(b, parsed::OP_MIN_OPERATORS)?,
        max_operators: le_u32(b, parsed::OP_MAX_OPERATORS)?,
        combat_power: le_u32(b, parsed::OP_COMBAT_POWER)?,
        reward1: le_u16(b, parsed::OP_REWARD_1)?,
        reward2: le_u16(b, parsed::OP_REWARD_2)?,
        skill_req: le_u16(b, parsed::OP_SKILL_REQ)?,
        skill_stat: le_u16(b, parsed::OP_SKILL_STAT)?,
    })
}

/// Decode one `dropsetinfo` entry out of its bytes, or `None` if the buffer is
/// too short. `index` is its position in the row's entry-pointer array and is
/// not read from the bytes.
///
/// Both halves of the amount pair are decoded, and the item row as a `u32`:
/// see [`parsed::dropset::ENTRY_AMOUNT_MAX`] and
/// [`parsed::dropset::ENTRY_ITEM`] for what a live capture had to say about
/// each of those two decisions.
pub fn decode_entry(index: usize, b: &[u8]) -> Option<DropsetEntry> {
    let mut conds = [parsed::dropset::COND_NONE; 3];
    for (slot, off) in conds.iter_mut().zip(parsed::dropset::ENTRY_CONDS) {
        *slot = le_u16(b, off)?;
    }
    Some(DropsetEntry {
        index,
        item_row: le_u32(b, parsed::dropset::ENTRY_ITEM)?,
        amount_min: le_i64(b, parsed::dropset::ENTRY_AMOUNT_MIN)?,
        amount_max: le_i64(b, parsed::dropset::ENTRY_AMOUNT_MAX)?,
        weight: le_i64(b, parsed::dropset::ENTRY_WEIGHT)?,
        kind: le_u8(b, parsed::dropset::ENTRY_KIND)?,
        conds,
    })
}

/// A `u16` row index as the log prints it: the number, or `-` for the
/// [`parsed::REWARD_NONE`] / [`parsed::dropset::COND_NONE`] /
/// [`parsed::SKILL_NONE`] sentinel, which is the same `0xFFFF` in all three
/// places. `65535` on a line means "none" and reads like a value, so it is
/// never printed.
pub fn row_text(v: u16) -> String {
    if v == parsed::REWARD_NONE {
        "-".to_string()
    } else {
        v.to_string()
    }
}

/// `160` -> `"16.0"`. Tenths of an hour as a human reads them, with no float
/// and so no rounding to argue about.
pub fn hours(tenths: u32) -> String {
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Render bytes as lowercase hex, for a raw dump line. Pure and trivially
/// testable, which is why it lives here and not in `scan.rs`.
pub fn hex_line(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for byte in b {
        // `write!` into a String cannot fail; discarded rather than unwrapped
        // so this stays panic-free.
        let _ = write!(s, "{byte:02x}");
    }
    s
}

impl Operation {
    /// The one greppable log line for this mission. Deliberately compact and
    /// deliberately built here rather than in `scan.rs`: it is what a human
    /// reads the results out of, so it is worth a native test.
    ///
    /// The duration is printed both ways - `dur=160(16.0h)` - because the raw
    /// tenths are what a `grep` sorts and compares against DMM's numbers, and
    /// the hours are what a person reads. `c0=` is `entry+0xC0`, named for its
    /// offset rather than for a meaning, because its meaning is not known.
    ///
    /// `skill=<required>/<bonus stat>` is [`parsed::OP_SKILL_REQ`] then
    /// [`parsed::OP_SKILL_STAT`], in that order and never merged: the first
    /// stops a mission starting and the second only sweetens its payout, and a
    /// line that let the two be confused would be worse than one that omitted
    /// them. A `-` in either half is the [`parsed::SKILL_NONE`] sentinel.
    pub fn log_line(&self) -> String {
        let mut s = format!(
            "op node={} idx={} key={} group={} dur={}({}h) ops={}..{} cp={} steps={} c0={} \
             skill={}/{} drop={}/{} conds={}",
            self.node_key,
            self.index,
            self.key,
            self.group,
            self.duration_tenths,
            hours(self.duration_tenths),
            self.min_operators,
            self.max_operators,
            self.combat_power,
            self.steps,
            self.flag_c0,
            row_text(self.skill_req),
            row_text(self.skill_stat),
            row_text(self.reward1),
            row_text(self.reward2),
            self.conds.len()
        );
        if !self.conds.is_empty() {
            s.push_str(" [");
            for (i, c) in self.conds.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                // `write!` into a String cannot fail; the result is discarded
                // rather than unwrapped so this stays panic-free.
                let _ = write!(s, "{}/{}/{}", c.key, c.show, c.fail);
            }
            s.push(']');
        }
        s
    }
}

/// Every reason a record or an entry was stepped over. Each one is a place
/// where the layout above stopped matching what the game actually has, which
/// is the first thing to look at when the verdict comes back bad.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skips {
    /// The record slot is still null: the game never loaded that record.
    pub unloaded: usize,
    /// A guarded read of the record failed, or its pointer was implausible.
    pub record_read: usize,
    /// The record has no operation list pointer at all.
    pub no_ops: usize,
    /// The operation count was past [`MAX_OPS`] and the rest was dropped.
    pub ops_capped: usize,
    /// A guarded read of an operation entry failed.
    pub op_read: usize,
    /// A condition list pointer that could not be read or was implausible.
    pub cond_read: usize,
    /// Operation entries never reached because the pass hit [`MAX_TOTAL_OPS`]
    /// and stopped walking. Non-zero means a misread count, not a big table.
    pub ops_budget: usize,
}

impl Skips {
    /// True when nothing at all was stepped over.
    pub fn is_empty(&self) -> bool {
        *self == Skips::default()
    }

    /// `unloaded=3 op_read=1`, naming only the counters that moved. Empty
    /// string when nothing did.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (name, n) in [
            ("unloaded", self.unloaded),
            ("record_read", self.record_read),
            ("no_ops", self.no_ops),
            ("ops_capped", self.ops_capped),
            ("op_read", self.op_read),
            ("cond_read", self.cond_read),
            ("ops_budget", self.ops_budget),
        ] {
            if n > 0 {
                parts.push(format!("{name}={n}"));
            }
        }
        parts.join(" ")
    }
}

/// Never keep more than this many samples for the medians. The real table is
/// far smaller; the cap is only here so a misread count cannot turn into an
/// unbounded allocation inside the game's process.
const MAX_SAMPLES: usize = 65_536;

/// Everything one pass learned, accumulated one [`Operation`] at a time.
///
/// The condition histogram is the valuable part: `docs/findings-dispatch-2026-09-10.md`
/// says which `conditioninfo` records actually appear "is polymorphic and
/// opaque statically - that needs a runtime logging pass, and it is the one
/// thing static analysis cannot settle here". This is that pass's answer.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    /// Record slots walked, loaded or not.
    pub records: usize,
    /// Records that were loaded and readable.
    pub records_read: usize,
    /// Records carrying at least one operation entry. Private, and moved
    /// through [`Summary::record_with_ops`] like every other counter: it was
    /// the one field `scan.rs` poked directly.
    records_with_ops: usize,
    /// Operation entries read in full.
    pub ops: usize,
    /// Of those, how many carry an empty condition list.
    pub ops_without_conds: usize,
    /// Condition entries read, counting repeats.
    pub cond_entries: usize,
    pub skips: Skips,

    /// Durations in tenths of an hour, one per mission.
    durations: Vec<u32>,
    min_operators: Vec<u32>,
    /// What `entry+0xC0` read, one per mission. Not judged, only watched.
    flags_c0: Vec<u32>,
    cond_keys: BTreeMap<u16, usize>,

    /// Missions whose `+0xA8` names a `dropsetinfo` row.
    pub ops_with_reward1: usize,
    /// Missions whose `+0xAA` names one.
    pub ops_with_reward2: usize,
    /// The distinct rows each set named. Sets, not counts: the reward pass
    /// reads each row once however many missions point at it, and the union of
    /// these two is **the only** row set it is ever allowed to touch -
    /// `dropsetinfo` is the game-wide drop table.
    reward1_rows: BTreeSet<u16>,
    reward2_rows: BTreeSet<u16>,

    /// Missions carrying a skill **requirement** ([`parsed::OP_SKILL_REQ`]),
    /// and the census of which index each one wants. Vanilla: 147 of 936
    /// missions over 13 distinct values.
    pub ops_with_skill_req: usize,
    skill_req_keys: BTreeMap<u16, usize>,
    /// Missions carrying a skill **bonus stat** ([`parsed::OP_SKILL_STAT`]).
    /// Vanilla: 231 of 936. Counted, not histogrammed: it is the soft field,
    /// and a write path would only ever leave it alone.
    pub ops_with_skill_stat: usize,
}

/// The one value every sample carries, or `None` if they differ or there are
/// fewer than two of them. A field that never varies across a whole table is
/// the loudest cheap signal that an offset is wrong.
///
/// Generic because the reward side needs exactly the same check over `i64`
/// amounts and weights and `u16` item rows. It is the check that caught
/// `entry+0xC0`, and it is the only one that would catch a reward offset
/// pointing at a field the whole table happens to share.
fn all_equal<T: Copy + PartialEq>(v: &[T]) -> Option<T> {
    let first = *v.first()?;
    if v.len() < 2 {
        return None;
    }
    v.iter().all(|&x| x == first).then_some(first)
}

impl Summary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one record slot, and say whether it was loaded and readable.
    pub fn record(&mut self, read: bool) {
        self.records += 1;
        if read {
            self.records_read += 1;
        }
    }

    /// Count one record that turned out to carry at least one mission.
    pub fn record_with_ops(&mut self) {
        self.records_with_ops += 1;
    }

    /// How many records carried at least one mission.
    pub fn records_with_ops(&self) -> usize {
        self.records_with_ops
    }

    /// Fold one fully read operation in.
    pub fn operation(&mut self, op: &Operation) {
        self.ops += 1;
        if self.durations.len() < MAX_SAMPLES {
            self.durations.push(op.duration_tenths);
            self.min_operators.push(op.min_operators);
            self.flags_c0.push(op.flag_c0);
        }
        if op.reward1 != parsed::REWARD_NONE {
            self.ops_with_reward1 += 1;
            self.reward1_rows.insert(op.reward1);
        }
        if op.reward2 != parsed::REWARD_NONE {
            self.ops_with_reward2 += 1;
            self.reward2_rows.insert(op.reward2);
        }
        if op.skill_req != parsed::SKILL_NONE {
            self.ops_with_skill_req += 1;
            *self.skill_req_keys.entry(op.skill_req).or_insert(0) += 1;
        }
        if op.skill_stat != parsed::SKILL_NONE {
            self.ops_with_skill_stat += 1;
        }
        if op.conds.is_empty() {
            self.ops_without_conds += 1;
        }
        for c in &op.conds {
            self.cond_entries += 1;
            *self.cond_keys.entry(c.key).or_insert(0) += 1;
        }
    }

    /// Every distinct `dropsetinfo` row the missions named, ascending: the
    /// union of `+0xA8` and `+0xAA`, and the exact set the reward pass reads.
    ///
    /// Capped at [`MAX_DROP_ROWS`] so a misread index cannot turn the reward
    /// pass into a walk of the whole game-wide drop table.
    pub fn reward_rows(&self) -> Vec<u16> {
        let mut v: Vec<u16> = self.reward1_rows.union(&self.reward2_rows).copied().collect();
        v.truncate(MAX_DROP_ROWS);
        v
    }

    /// `(distinct rows in set 1, distinct rows in set 2, distinct rows in the
    /// union)`.
    pub fn reward_row_counts(&self) -> (usize, usize, usize) {
        (self.reward1_rows.len(), self.reward2_rows.len(), self.reward_rows().len())
    }

    /// The line a human checks the reward census against at a glance: what
    /// this pass found beside what a vanilla table of build 25116796 read.
    ///
    /// The vanilla numbers are not decoration. Two of them were wrong in an
    /// earlier pass (36 operations and 221 rows, from a snapshot that held two
    /// sessions and counted shared nodes twice), and printing the expected
    /// figure on the same line as the measured one is what makes that kind of
    /// mistake visible without a second tool.
    pub fn reward_request_line(&self) -> String {
        let (n1, n2, union) = self.reward_row_counts();
        let agrees = union == REWARD_ROWS_UNION
            && n1 == REWARD_ROWS_SET1
            && n2 == REWARD_ROWS_SET2
            && self.ops_with_reward1 == REWARD_OPS_SET1
            && self.ops_with_reward2 == REWARD_OPS_SET2;
        format!(
            "{union} distinct dropset rows requested ({n1} from +0xA8 on {} missions, {n2} from \
             +0xAA on {} missions, overlapping); vanilla build 25116796 reads \
             {REWARD_ROWS_UNION} ({REWARD_ROWS_SET1} from {REWARD_OPS_SET1}, {REWARD_ROWS_SET2} \
             from {REWARD_OPS_SET2}){}",
            self.ops_with_reward1,
            self.ops_with_reward2,
            if agrees { " - AGREES" } else { " - DIFFERS, see the missions above" }
        )
    }

    /// The distinct `conditioninfo` keys seen, most common first, ties broken
    /// by key so the output is stable between runs.
    pub fn condition_histogram(&self) -> Vec<(u16, usize)> {
        let mut v: Vec<(u16, usize)> = self.cond_keys.iter().map(|(k, n)| (*k, *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// `12=340 15=120 …`, capped at `max` entries with a `(+N more)` tail.
    /// One line, however many keys there turn out to be.
    pub fn histogram_line(&self, max: usize) -> String {
        let all = self.condition_histogram();
        if all.is_empty() {
            return "none".to_string();
        }
        let shown = all.len().min(max);
        let mut s = String::new();
        for (i, (key, n)) in all.iter().take(shown).enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{key}={n}");
        }
        if all.len() > shown {
            let _ = write!(s, " (+{} more)", all.len() - shown);
        }
        s
    }

    /// The distinct required-skill indices seen, most common first, ties
    /// broken by index so two runs agree - [`Self::condition_histogram`] for
    /// [`parsed::OP_SKILL_REQ`].
    ///
    /// This is the census a "no skill requirement" option would be built on:
    /// which indices are demanded, and how many missions each one locks. The
    /// missions with no requirement at all are not in it; they are
    /// `ops - ops_with_skill_req`.
    pub fn skill_histogram(&self) -> Vec<(u16, usize)> {
        let mut v: Vec<(u16, usize)> = self.skill_req_keys.iter().map(|(k, n)| (*k, *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// The skill census as one line: how many missions are gated, over how
    /// many distinct indices, and the histogram itself capped at `max` entries.
    ///
    /// Vanilla build 25116796 reads 147 of 936 gated over 13 indices, and 231
    /// of 936 carrying the bonus stat, which are different missions: the two
    /// fields are independent and the line prints both so they cannot be read
    /// as one number.
    pub fn skill_line(&self, max: usize) -> String {
        let hist = self.skill_histogram();
        let mut s = format!(
            "{} of {} missions require a skill (+0x{:X}) over {} distinct indices, {} carry a \
             bonus stat (+0x{:X}): ",
            self.ops_with_skill_req,
            self.ops,
            parsed::OP_SKILL_REQ,
            hist.len(),
            self.ops_with_skill_stat,
            parsed::OP_SKILL_STAT
        );
        if hist.is_empty() {
            s.push_str("none");
            return s;
        }
        let shown = hist.len().min(max);
        for (i, (key, n)) in hist.iter().take(shown).enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{key}={n}");
        }
        if hist.len() > shown {
            let _ = write!(s, " (+{} more)", hist.len() - shown);
        }
        s
    }

    /// `lo..hi` of the durations seen, in tenths of an hour, or `None` when
    /// none were.
    pub fn duration_range(&self) -> Option<(u32, u32)> {
        range_of(&self.durations)
    }

    /// `lo..hi` of the minimum operator counts seen.
    pub fn operator_range(&self) -> Option<(u32, u32)> {
        range_of(&self.min_operators)
    }

    /// The median duration, in tenths of an hour.
    pub fn duration_median(&self) -> Option<u32> {
        median(&self.durations)
    }

    pub fn operator_median(&self) -> Option<u32> {
        median(&self.min_operators)
    }

    /// How many durations, and how many minimum operator counts, landed in the
    /// range a real value would be in.
    pub fn in_range_counts(&self) -> (usize, usize) {
        (
            self.durations.iter().filter(|d| (DURATION_LO..=DURATION_HI).contains(d)).count(),
            self.min_operators.iter().filter(|m| (OPERATORS_LO..=OPERATORS_HI).contains(m)).count(),
        )
    }

    /// Whether the offsets in [`parsed`] look like the right ones, as a
    /// sentence meant to be read in the log.
    ///
    /// The test is deliberately crude and deliberately about two fields: if
    /// `step[1]+0x08` really is a mission length in tenths of an hour and
    /// `+0xC4` really is an operator count, almost every value of each lands
    /// in a small range, and if either offset is wrong what comes back is
    /// arbitrary 32-bit noise, which essentially never does. That is enough to
    /// answer the only question a first launch asks.
    ///
    /// It judges the **real** duration. It used to judge `entry+0xC0` against
    /// a `1..=100` day count, which that field passed perfectly while being
    /// the wrong field entirely - see [`parsed::OP_UNKNOWN_C0`].
    ///
    /// **It is a layout check over whatever is in the records right now, which
    /// is only a vanilla census while this plugin has written nothing.** The
    /// levers edit two of the three fields it measures, so on a pass that
    /// follows a write the ranges and the verdict describe this plugin's own
    /// output: `Speed=20` turns a vanilla 20 into a 1, below [`DURATION_LO`],
    /// and the line would then say LOOKS WRONG about a perfectly good offset.
    /// `scan.rs` dumps a record before applying to it for exactly this reason,
    /// and says so in the log on the one pass where it cannot
    /// ([`Summary::written_to_caveat`]).
    pub fn verdict(&self) -> String {
        if self.ops == 0 {
            return format!(
                "verdict: NO DATA - {} record slots walked, {} readable, none carried an \
                 operation entry, so nothing here says anything about the layout",
                self.records, self.records_read
            );
        }
        let sampled = self.durations.len();
        if sampled == 0 {
            return "verdict: NO DATA - no samples were kept".to_string();
        }
        let (dur_ok, ops_ok) = self.in_range_counts();
        let dur_frac = dur_ok as f64 / sampled as f64;
        let ops_frac = ops_ok as f64 / sampled as f64;
        let worst = dur_frac.min(ops_frac);
        let call = if worst >= VERDICT_GOOD {
            "LOOKS RIGHT"
        } else if worst >= VERDICT_BAD {
            "UNCONVINCING"
        } else {
            "LOOKS WRONG"
        };
        // A range check alone is not enough, and this cost a session to learn:
        // every one of 936 missions read `entry+0xC0` as exactly 1, which
        // passed the old "is it in 1..=100 days" test perfectly while telling
        // us nothing. This check is what caught that, so it stays, now aimed
        // at the real duration. A field that never varies is the likeliest
        // sign of a wrong offset, so it gets its own sentence - though it is
        // not proof on its own, because a mod that flattens the real values
        // produces the same reading.
        let mut out = format!(
            "verdict: the layout {call} - {dur_ok}/{sampled} durations in \
             {DURATION_LO}..={DURATION_HI} tenths of an hour ({:.0}%), {ops_ok}/{sampled} minimum \
             operator counts in {OPERATORS_LO}..={OPERATORS_HI} ({:.0}%)",
            dur_frac * 100.0,
            ops_frac * 100.0
        );
        // The cause named first is the likeliest one, and it is this plugin:
        // `AnyOperatorCount=1` writes exactly `1` into every minimum operator
        // count, and a `Speed` high enough to hit the floor gives every mission
        // the same duration. Both produce a perfectly flat field out of a
        // perfectly good offset, so a sentence that said "wrong offset or a mod
        // that edits the table on disk" was naming everything but the most
        // likely culprit - and the first bug report would have been about an
        // offset that had not moved.
        for (label, constant, ours) in [
            (
                "duration",
                self.constant_duration(),
                "a Speed high enough to floor every mission at the same length",
            ),
            (
                "minimum operator count",
                self.constant_min_ops(),
                "AnyOperatorCount=1, which writes exactly 1 into this field on every mission",
            ),
        ] {
            if let Some(v) = constant {
                out.push_str(&format!(
                    "; CONSTANT: every one of {sampled} missions reports {label} {v}. Check this \
                     plugin's own settings first ({ours}); then a mod that edits the table on \
                     disk; only then a wrong offset"
                ));
            }
        }
        out
    }

    /// The sentence a census owes the log when this plugin has already written
    /// to the records it walked.
    ///
    /// The dump is run before the apply pass on every record it can (see
    /// `scan.rs`), so the normal case is that this is never printed. It exists
    /// for the one case that cannot be avoided: records that appeared, were
    /// applied to, and are only being dumped on a later pass. Saying it is much
    /// cheaper than a bug report about a duration range that is exactly
    /// `vanilla / Speed`.
    pub fn written_to_caveat() -> &'static str {
        "NOTE: this plugin has already written to some of the records above, so the ranges, the \
         medians and the verdict describe the table as it is now and not a vanilla census. Set \
         every lever to its vanilla value (or Enabled=0) and restart the game for a clean layout \
         check."
    }

    /// The single duration every mission reported, in tenths of an hour, if
    /// they all agree and there was more than one of them.
    pub fn constant_duration(&self) -> Option<u32> {
        all_equal(&self.durations)
    }

    /// The single `entry+0xC0` value every mission reported, if they all
    /// agree. `Some(1)` on a vanilla table of this build, which is the whole
    /// reason that field is no longer called a duration.
    pub fn constant_flag_c0(&self) -> Option<u32> {
        all_equal(&self.flags_c0)
    }

    /// `lo..hi` of the `entry+0xC0` values seen.
    pub fn flag_c0_range(&self) -> Option<(u32, u32)> {
        range_of(&self.flags_c0)
    }

    /// The single minimum operator count every mission reported, if they all
    /// agree and there was more than one of them.
    pub fn constant_min_ops(&self) -> Option<u32> {
        all_equal(&self.min_operators)
    }

    /// The counts line that precedes the histogram and the verdict.
    ///
    /// Durations are tenths of an hour, printed with the hours beside them the
    /// same way [`Operation::log_line`] does.
    pub fn counts_line(&self) -> String {
        let (dlo, dhi) = self.duration_range().unwrap_or((0, 0));
        let (olo, ohi) = self.operator_range().unwrap_or((0, 0));
        let (clo, chi) = self.flag_c0_range().unwrap_or((0, 0));
        let dmed = self.duration_median().unwrap_or(0);
        format!(
            "records {} ({} loaded, {} with missions), missions {}, duration {dlo}..{dhi} tenths \
             of an hour ({}h..{}h, median {dmed} = {}h), min operators {olo}..{ohi} (median {}), \
             +0xC0 {clo}..{chi}, missions with no conditions {}, condition entries {}, distinct \
             condition keys {}",
            self.records,
            self.records_read,
            self.records_with_ops,
            self.ops,
            hours(dlo),
            hours(dhi),
            hours(dmed),
            self.operator_median().unwrap_or(0),
            self.ops_without_conds,
            self.cond_entries,
            self.cond_keys.len()
        )
    }
}

// ---------------------------------------------------------------------------
// The reward rows: `dropsetinfo` records, reached by the row index at
// `entry+0xA8` / `+0xAA`.
//
// Everything below reads a layout a single live capture has now seen once -
// see `parsed::dropset`, where each offset says what the 219 rows and 725
// entries of that capture read. That is still one capture, on one build, of
// one vanilla table, and four of those offsets have yet to show a value that
// varies at all, so these types keep their own accumulator and their own
// verdict rather than folding into `Summary`: the operation layout is settled
// and the reward layout is one capture old, and mixing the two would let a
// confident verdict about missions be read as one about rewards.
// ---------------------------------------------------------------------------

/// One entry of a `dropsetinfo` row: one item, and how many of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropsetEntry {
    /// Position in the row's entry-pointer array.
    pub index: usize,
    /// `iteminfo` row of the item paid ([`parsed::dropset::ENTRY_ITEM`]). A
    /// `u32`, because [`Self::kind`] `13` entries carry values a `u16` read
    /// would silently truncate.
    pub item_row: u32,
    /// The fewest of it a roll pays
    /// ([`parsed::dropset::ENTRY_AMOUNT_MIN`]).
    pub amount_min: i64,
    /// The most of it a roll pays ([`parsed::dropset::ENTRY_AMOUNT_MAX`]).
    ///
    /// **The pair is what a reward multiplier scales, both halves of it.**
    /// `min <= max` on every entry of the live capture and the two differ on
    /// 136 of 725, so scaling only the minimum inverts those 136. This is the
    /// same shape as the `gimmickinfo` output block `desert_gatherer`
    /// multiplies, deliberately: the two write paths should end up looking
    /// alike.
    pub amount_max: i64,
    /// Roll weight ([`parsed::dropset::ENTRY_WEIGHT`]).
    pub weight: i64,
    /// Entry kind ([`parsed::dropset::ENTRY_KIND`]); reported, not
    /// interpreted - except
    /// [`parsed::dropset::ENTRY_KIND_NOT_AN_ITEM`], which a multiplier has to
    /// skip.
    pub kind: u8,
    /// The three condition keys, `0xFFFF` where unused.
    pub conds: [u16; 3],
}

impl DropsetEntry {
    /// The greppable per-entry line. `row=` repeats on every line on purpose:
    /// it is what makes `grep 'dropentry row=6244'` pull one row's whole
    /// payout out of a log that has 219 of them interleaved with everything
    /// else.
    ///
    /// `amount=` is always printed as `min..max`, even where the two agree
    /// (589 of 725 entries in the live capture): one shape means a `grep` over
    /// the pair needs one pattern, and a single number here is what hid the
    /// second half of the field for a whole session.
    pub fn log_line(&self, row: u16) -> String {
        format!(
            "dropentry row={row} idx={} item={} amount={}..{} weight={} kind={} conds={}/{}/{}{}",
            self.index,
            self.item_row,
            self.amount_min,
            self.amount_max,
            self.weight,
            self.kind,
            row_text(self.conds[0]),
            row_text(self.conds[1]),
            row_text(self.conds[2]),
            if self.is_item_drop() { "" } else { " (not an item drop)" }
        )
    }

    /// Whether this entry pays an item at all.
    ///
    /// False for [`parsed::dropset::ENTRY_KIND_NOT_AN_ITEM`], whose 34
    /// occurrences in the live capture are exactly the entries with a zero
    /// amount and a `+0x38` too wide for a `u16`. **This is the predicate a
    /// reward multiplier owes those entries**, and it lives here rather than in
    /// the write path that does not exist yet so that the dump and that write
    /// path can never disagree about which entries are drops.
    pub fn is_item_drop(&self) -> bool {
        self.kind != parsed::dropset::ENTRY_KIND_NOT_AN_ITEM
    }
}

/// One `dropsetinfo` record, as read out of the parsed object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropsetRow {
    /// Its manager row index - which is also the number an operation entry
    /// carries at `+0xA8` / `+0xAA`.
    pub row: u16,
    /// [`parsed::dropset::ROW_DRAWS`]: how many entries one roll pays.
    pub draws: i32,
    /// What [`parsed::dropset::ROW_ENTRY_COUNT`] declared, which is not always
    /// how many entries actually read - a short read is the interesting case
    /// and must stay visible.
    pub declared_entries: u32,
    /// [`parsed::dropset::ROW_NO_DROP_PPM`], parts per million.
    pub no_drop_ppm: i64,
    pub entries: Vec<DropsetEntry>,
}

impl DropsetRow {
    /// The greppable per-row line, in [`Operation::log_line`]'s style.
    /// `entries=` is what was read and `decl=` what the record claimed; they
    /// differ only when something went wrong, and then the difference is the
    /// finding.
    pub fn log_line(&self) -> String {
        format!(
            "drop row={} entries={}(decl={}) draws={} nodrop={}ppm",
            self.row,
            self.entries.len(),
            self.declared_entries,
            self.draws,
            self.no_drop_ppm
        )
    }
}

/// What one reward pass learned, accumulated one [`DropsetRow`] at a time.
///
/// Rows load lazily, so "still unloaded" is a normal reading and not an error:
/// a row nobody has needed yet has a null slot, and the pass reports it and
/// looks again later rather than calling the layout wrong.
#[derive(Debug, Clone, Default)]
pub struct RewardSummary {
    /// Distinct rows the missions named.
    pub rows_requested: usize,
    /// Of those, rows that were loaded and read.
    pub rows_loaded: usize,
    /// Rows whose slot is still null: the game has not parsed them yet.
    /// **Normal**, and the reason the pass repeats.
    pub rows_null: usize,
    /// Rows whose index is past the manager's own record count. Not normal:
    /// it says the index at `+0xA8` is not a row index at all.
    pub rows_out_of_range: usize,
    /// Rows whose slot held something that would not read.
    pub rows_unreadable: usize,
    /// Entries read in full, across every row.
    pub entries: usize,
    /// Rows whose declared entry count was past [`MAX_DROP_ENTRIES`].
    pub entries_capped: usize,
    /// Entries dropped because a field of them, or their pointer, would not read.
    pub entry_read_fail: usize,
    /// Entries whose item row is neither 0 nor `0xFFFF`.
    item_rows_valid: usize,
    /// Entries that are not item drops at all
    /// ([`parsed::dropset::ENTRY_KIND_NOT_AN_ITEM`]). 34 of 725 in the live
    /// capture, and the entries a multiplier would skip.
    pub entries_not_items: usize,
    /// Entries whose amount pair is the right way round. 725 of 725 live; a
    /// pair that is not is either a wrong offset or a table something has
    /// already scaled by halves.
    ordered_amounts: usize,

    amount_mins: Vec<i64>,
    amount_maxes: Vec<i64>,
    weights: Vec<i64>,
    draws: Vec<i32>,
    no_drop_ppms: Vec<i64>,
    /// The first and third condition slots, kept only so [`all_equal`] can see
    /// them: both read `0xFFFF` on all 725 entries of the live capture, which
    /// is why they are UNCONFIRMED. The middle slot varies and is not watched
    /// this way.
    cond_slot0: Vec<u16>,
    cond_slot2: Vec<u16>,
    entry_counts: Vec<usize>,
    item_rows: BTreeMap<u32, usize>,
    kinds: BTreeMap<u8, usize>,
}

impl RewardSummary {
    pub fn new(rows_requested: usize) -> Self {
        RewardSummary { rows_requested, ..Default::default() }
    }

    /// Fold one fully read row, and every entry of it, in.
    pub fn row(&mut self, r: &DropsetRow) {
        self.rows_loaded += 1;
        if self.entry_counts.len() < MAX_SAMPLES {
            self.entry_counts.push(r.entries.len());
            self.draws.push(r.draws);
            self.no_drop_ppms.push(r.no_drop_ppm);
        }
        for e in &r.entries {
            self.entries += 1;
            if e.item_row != 0 && e.item_row != u32::from(parsed::REWARD_NONE) {
                self.item_rows_valid += 1;
            }
            if !e.is_item_drop() {
                self.entries_not_items += 1;
            }
            if e.amount_min <= e.amount_max {
                self.ordered_amounts += 1;
            }
            *self.item_rows.entry(e.item_row).or_insert(0) += 1;
            *self.kinds.entry(e.kind).or_insert(0) += 1;
            if self.amount_mins.len() < MAX_SAMPLES {
                self.amount_mins.push(e.amount_min);
                self.amount_maxes.push(e.amount_max);
                self.weights.push(e.weight);
                if let (Some(&c0), Some(&c2)) = (e.conds.first(), e.conds.get(2)) {
                    self.cond_slot0.push(c0);
                    self.cond_slot2.push(c2);
                }
            }
        }
    }

    /// `lo..hi` of the amount **minimums**.
    pub fn amount_range(&self) -> Option<(i64, i64)> {
        range_of(&self.amount_mins)
    }

    /// `lo..hi` of the amount **maximums**. Separate from
    /// [`Self::amount_range`] on purpose: the two bands moving apart is what a
    /// half-scaled table looks like.
    pub fn amount_max_range(&self) -> Option<(i64, i64)> {
        range_of(&self.amount_maxes)
    }

    pub fn amount_median(&self) -> Option<i64> {
        median(&self.amount_mins)
    }

    /// How many entries have `min <= max`, and how many were sampled. Live:
    /// 725 of 725.
    pub fn ordered_amounts(&self) -> (usize, usize) {
        (self.ordered_amounts, self.entries)
    }

    pub fn weight_range(&self) -> Option<(i64, i64)> {
        range_of(&self.weights)
    }

    pub fn entries_per_row_range(&self) -> Option<(usize, usize)> {
        range_of(&self.entry_counts)
    }

    /// Distinct `iteminfo` rows paid across every reward row.
    pub fn distinct_item_rows(&self) -> usize {
        self.item_rows.len()
    }

    /// The single amount minimum every entry reported, if they all agree and
    /// there was more than one of them. See [`all_equal`].
    pub fn constant_amount_min(&self) -> Option<i64> {
        all_equal(&self.amount_mins)
    }

    /// The same for the amount maximum. Both are checked, because a wrong
    /// offset for one half of the pair is exactly as damaging as a wrong
    /// offset for the other and neither is more likely.
    pub fn constant_amount_max(&self) -> Option<i64> {
        all_equal(&self.amount_maxes)
    }

    pub fn constant_weight(&self) -> Option<i64> {
        all_equal(&self.weights)
    }

    /// The single draw count every **row** reported. `Some(0)` on the live
    /// capture, which is why [`parsed::dropset::ROW_DRAWS`] is UNCONFIRMED.
    pub fn constant_draws(&self) -> Option<i32> {
        all_equal(&self.draws)
    }

    /// The single no-drop chance every row reported. `Some(0)` live, and the
    /// reason [`parsed::dropset::ROW_NO_DROP_PPM`] is UNCONFIRMED.
    pub fn constant_no_drop_ppm(&self) -> Option<i64> {
        all_equal(&self.no_drop_ppms)
    }

    /// The single value the first condition slot reported. `Some(0xFFFF)`
    /// live.
    pub fn constant_cond_slot0(&self) -> Option<u16> {
        all_equal(&self.cond_slot0)
    }

    /// The same for the third slot. `Some(0xFFFF)` live.
    pub fn constant_cond_slot2(&self) -> Option<u16> {
        all_equal(&self.cond_slot2)
    }

    /// The single item row every entry reported. A whole table paying one item
    /// is not a table.
    pub fn constant_item_row(&self) -> Option<u32> {
        let rows: Vec<u32> = self.item_rows.keys().copied().collect();
        match (rows.first(), rows.len(), self.entries) {
            (Some(&only), 1, n) if n > 1 => Some(only),
            _ => None,
        }
    }

    /// `1=340 0=12 …`, kinds ascending so two runs agree.
    pub fn kind_line(&self) -> String {
        if self.kinds.is_empty() {
            return "none".to_string();
        }
        let mut s = String::new();
        for (i, (kind, n)) in self.kinds.iter().enumerate() {
            if i > 0 {
                s.push(' ');
            }
            let _ = write!(s, "{kind}={n}");
        }
        s
    }

    /// `(amount pairs in band, entries with a real item row, rows with a
    /// plausible entry count)`, the three fractions [`Self::verdict`] weighs.
    ///
    /// An amount counts only when **both** halves are in band *and* the pair is
    /// the right way round, so a wrong offset for either half, and an inverted
    /// pair, all show up in the same number.
    pub fn in_range_counts(&self) -> (usize, usize, usize) {
        (
            self.amount_mins
                .iter()
                .zip(&self.amount_maxes)
                .filter(|(lo, hi)| {
                    lo <= hi
                        && (AMOUNT_LO..=AMOUNT_HI).contains(*lo)
                        && (AMOUNT_LO..=AMOUNT_HI).contains(*hi)
                })
                .count(),
            self.item_rows_valid,
            self.entry_counts
                .iter()
                .filter(|n| (DROP_ENTRIES_LO..=DROP_ENTRIES_HI).contains(n))
                .count(),
        )
    }

    /// The counts line: what was asked for, what was there, and the spread of
    /// every field the layout claims.
    pub fn counts_line(&self) -> String {
        let (alo, ahi) = self.amount_range().unwrap_or((0, 0));
        let (xlo, xhi) = self.amount_max_range().unwrap_or((0, 0));
        let (wlo, whi) = self.weight_range().unwrap_or((0, 0));
        let (elo, ehi) = self.entries_per_row_range().unwrap_or((0, 0));
        let (dlo, dhi) = range_of(&self.draws).unwrap_or((0, 0));
        format!(
            "rows requested {}, loaded {}, still unloaded {}, out of range {}, unreadable {}; \
             entries {} (capped rows {}, unreadable {}), amount min {alo}..{ahi} (median {}), \
             amount max {xlo}..{xhi}, min<=max on {}/{}, weight {wlo}..{whi}, entries per row \
             {elo}..{ehi} (median {}), draws {dlo}..{dhi}, distinct item rows {}, kinds {} \
             (not item drops {})",
            self.rows_requested,
            self.rows_loaded,
            self.rows_null,
            self.rows_out_of_range,
            self.rows_unreadable,
            self.entries,
            self.entries_capped,
            self.entry_read_fail,
            self.amount_median().unwrap_or(0),
            self.ordered_amounts,
            self.entries,
            median(&self.entry_counts).unwrap_or(0),
            self.distinct_item_rows(),
            self.kind_line(),
            self.entries_not_items
        )
    }

    /// Whether the offsets in [`parsed::dropset`] look like the right ones.
    ///
    /// Three crude tests, for the same reason [`Summary::verdict`] uses two:
    /// if `entry+0x20`/`+0x28` really are an amount pair, `entry+0x38` really
    /// is an item row and `record+0x30` really is an entry count, almost every
    /// value of each lands in a small band, and a wrong offset yields 64-bit
    /// noise, which essentially never does. A vanilla dump reads about 95% on
    /// the amounts, not 100%: the [`parsed::dropset::ENTRY_KIND_NOT_AN_ITEM`]
    /// entries pay `0` and fall out of band, which is the point of them.
    ///
    /// And a band check alone is not enough - that is the lesson `entry+0xC0`
    /// cost a session to teach, so [`all_equal`] runs over **every** field the
    /// layout claims, not only the three the bands cover: both halves of the
    /// amount, the weight, the item row, the row's draw count and no-drop
    /// chance, and the first and third condition slots. The last four are
    /// exactly the fields the live capture read as one constant value, so a
    /// check that skipped them would be a check that missed everything it
    /// caught. A field that never varies over a whole table is the loudest
    /// cheap sign of a wrong offset there is.
    ///
    /// The same caveat [`Summary::verdict`] carries applies here: both halves of
    /// the amount pair are fields the `Rewards` lever writes, so this is a
    /// vanilla census only on a pass where nothing has been written to the rows
    /// it read. `scan.rs` dumps a row before applying to it, and prints
    /// [`Summary::written_to_caveat`] when it could not.
    pub fn verdict(&self) -> String {
        if self.rows_loaded == 0 {
            return format!(
                "verdict: NO REWARD DATA - {} rows requested, none loaded yet ({} still unloaded, \
                 {} out of range, {} unreadable); dropset rows load on demand, so this is normal \
                 early and the pass will look again",
                self.rows_requested, self.rows_null, self.rows_out_of_range, self.rows_unreadable
            );
        }
        let sampled = self.amount_mins.len();
        if sampled == 0 || self.entries == 0 {
            return format!(
                "verdict: NO REWARD DATA - {} rows loaded but not one entry read, so either \
                 record+0x{:X} is not an entry count or record+0x{:X} is not a pointer array",
                self.rows_loaded,
                parsed::dropset::ROW_ENTRY_COUNT,
                parsed::dropset::ROW_ENTRIES
            );
        }
        let (amount_ok, item_ok, count_ok) = self.in_range_counts();
        let rows = self.entry_counts.len().max(1);
        let amount_frac = amount_ok as f64 / sampled as f64;
        let item_frac = item_ok as f64 / self.entries as f64;
        let count_frac = count_ok as f64 / rows as f64;
        let worst = amount_frac.min(item_frac).min(count_frac);
        let call = if worst >= VERDICT_GOOD {
            "LOOKS RIGHT"
        } else if worst >= VERDICT_BAD {
            "UNCONVINCING"
        } else {
            "LOOKS WRONG"
        };
        let mut out = format!(
            "verdict: the dropset layout {call} - {amount_ok}/{sampled} amounts in \
             {AMOUNT_LO}..={AMOUNT_HI} ({:.0}%), {item_ok}/{} entries name a real item row \
             ({:.0}%), {count_ok}/{rows} rows hold {DROP_ENTRIES_LO}..={DROP_ENTRIES_HI} entries \
             ({:.0}%)",
            amount_frac * 100.0,
            self.entries,
            item_frac * 100.0,
            count_frac * 100.0
        );
        // Per-entry fields, then per-row ones: the sentence names how many
        // samples agreed, and rows and entries are different counts.
        let rows_sampled = self.entry_counts.len();
        for (label, value, over, unit) in [
            ("amount min", self.constant_amount_min().map(|v| v.to_string()), self.entries, "entries"),
            ("amount max", self.constant_amount_max().map(|v| v.to_string()), self.entries, "entries"),
            ("weight", self.constant_weight().map(|v| v.to_string()), self.entries, "entries"),
            ("item row", self.constant_item_row().map(|v| v.to_string()), self.entries, "entries"),
            (
                "condition slot 0",
                self.constant_cond_slot0().map(|v| format!("0x{v:04X}")),
                self.entries,
                "entries",
            ),
            (
                "condition slot 2",
                self.constant_cond_slot2().map(|v| format!("0x{v:04X}")),
                self.entries,
                "entries",
            ),
            ("draw count", self.constant_draws().map(|v| v.to_string()), rows_sampled, "rows"),
            (
                "no-drop chance",
                self.constant_no_drop_ppm().map(|v| format!("{v}ppm")),
                rows_sampled,
                "rows",
            ),
        ] {
            if let Some(v) = value {
                let _ = write!(
                    out,
                    "; CONSTANT: every one of {over} {unit} reports {label} {v}, so that field is \
                     either at the wrong offset or genuinely flat across every row read so far"
                );
            }
        }
        out
    }
}

fn range_of<T: Copy + Ord>(v: &[T]) -> Option<(T, T)> {
    let lo = v.iter().min()?;
    let hi = v.iter().max()?;
    Some((*lo, *hi))
}

/// The lower median, which needs no averaging and so cannot overflow or round.
fn median<T: Copy + Ord>(v: &[T]) -> Option<T> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_unstable();
    s.get(s.len() / 2).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `dur` is tenths of an hour, the unit the real field is in. Neither
    /// reward set is named; [`rewarded`] is the variant that names one.
    fn op(node: u32, idx: usize, dur: u32, min: u32, conds: &[(u16, u16, u16)]) -> Operation {
        Operation {
            node_key: node,
            index: idx,
            key: 900 + idx as u32,
            group: 7,
            duration_tenths: dur,
            min_operators: min,
            max_operators: min + 2,
            combat_power: 1234,
            steps: 3,
            flag_c0: 1,
            reward1: parsed::REWARD_NONE,
            reward2: parsed::REWARD_NONE,
            skill_req: parsed::SKILL_NONE,
            skill_stat: parsed::SKILL_NONE,
            conds: conds.iter().map(|&(key, show, fail)| Cond { key, show, fail }).collect(),
        }
    }

    /// The same mission with the two skill fields set. `req` is the hard gate
    /// at `+0xDA`, `stat` the reward-bonus stat at `+0xD8`.
    fn skilled(idx: usize, req: u16, stat: u16) -> Operation {
        Operation { skill_req: req, skill_stat: stat, ..op(1, idx, 160, 3, &[]) }
    }

    /// The same mission with its two reward-set fields set.
    fn rewarded(idx: usize, r1: u16, r2: u16) -> Operation {
        Operation { reward1: r1, reward2: r2, ..op(1, idx, 160, 3, &[]) }
    }

    /// One synthetic dropset entry that pays a fixed amount.
    fn entry(index: usize, item: u32, amount: i64) -> DropsetEntry {
        DropsetEntry {
            index,
            item_row: item,
            amount_min: amount,
            amount_max: amount,
            weight: 10_000 + amount,
            kind: 1,
            conds: [0xFFFF, 0xFFFF, 0xFFFF],
        }
    }

    /// One synthetic dropset row of `n` entries.
    fn drow(row: u16, n: usize) -> DropsetRow {
        let entries: Vec<DropsetEntry> =
            (0..n).map(|i| entry(i, u32::from(4000 + row) + i as u32, 1 + i as i64)).collect();
        DropsetRow {
            row,
            draws: 1,
            declared_entries: n as u32,
            no_drop_ppm: 0,
            entries,
        }
    }

    /// Write `v` into `b` at `off`, little-endian, the way the game's own
    /// parsed record has it. Panics on a short buffer, which is a test bug.
    fn put(b: &mut [u8], off: usize, v: &[u8]) {
        b[off..off + v.len()].copy_from_slice(v);
    }

    /// The bytes of one operation entry, laid out at the offsets `mod parsed`
    /// claims, carrying one real vanilla mission: a Fishing dispatch, whose
    /// `+0xD8`/`+0xDA` pair the live cross-check resolved to work stat 63 and
    /// required skill 1944.
    fn vanilla_op_bytes() -> [u8; parsed::OP_STRIDE] {
        let mut b = [0u8; parsed::OP_STRIDE];
        put(&mut b, parsed::OP_KEY, &1_000_902u32.to_le_bytes());
        put(&mut b, parsed::OP_GROUP, &7u32.to_le_bytes());
        put(&mut b, parsed::OP_REWARD_1, &6244u16.to_le_bytes());
        put(&mut b, parsed::OP_REWARD_2, &parsed::REWARD_NONE.to_le_bytes());
        put(&mut b, parsed::OP_UNKNOWN_C0, &1u32.to_le_bytes());
        put(&mut b, parsed::OP_MIN_OPERATORS, &3u32.to_le_bytes());
        put(&mut b, parsed::OP_MAX_OPERATORS, &5u32.to_le_bytes());
        put(&mut b, parsed::OP_COMBAT_POWER, &1234u32.to_le_bytes());
        put(&mut b, parsed::OP_SKILL_STAT, &63u16.to_le_bytes());
        put(&mut b, parsed::OP_SKILL_REQ, &1944u16.to_le_bytes());
        b
    }

    /// The offsets, tested the only way that can fail when one of them is
    /// wrong: by decoding bytes laid out the way the game's records are.
    ///
    /// This replaces a test that asserted `OP_KEY == 0x60` and friends, which
    /// could not fail - editing a constant and its assertion together left it
    /// green. Here the field values and the offsets are stated in two different
    /// places, so a moved constant moves one and not the other.
    #[test]
    fn an_operation_entrys_bytes_decode_to_the_fields_at_those_offsets() {
        let b = vanilla_op_bytes();
        let f = decode_operation(&b).expect("a full entry decodes");
        assert_eq!(f.key, 1_000_902);
        assert_eq!(f.group, 7);
        assert_eq!(f.flag_c0, 1);
        assert_eq!((f.min_operators, f.max_operators), (3, 5));
        assert_eq!(f.combat_power, 1234);
        assert_eq!((f.reward1, f.reward2), (6244, parsed::REWARD_NONE));
        // The two skill fields are two bytes apart and must not be swapped:
        // the requirement gates the mission, the stat only pays a bonus.
        assert_eq!(f.skill_req, 1944, "+0xDA is the required Skill index");
        assert_eq!(f.skill_stat, 63, "+0xD8 is the work-stat bonus basis");

        // A buffer that stops short of the last named field decodes to
        // nothing rather than to a plausible-looking half-entry.
        assert_eq!(decode_operation(&b[..parsed::OP_SKILL_REQ + 1]), None);
        assert_eq!(decode_operation(&[]), None);

        // Every field named is inside the entry it belongs to, and the
        // condition entries are inside their stride.
        const { assert!(parsed::OP_COND_COUNT + 4 <= parsed::OP_STRIDE) };
        const { assert!(parsed::OP_SKILL_REQ + 2 <= parsed::OP_STRIDE) };
        const { assert!(parsed::COND_KEY + 2 <= parsed::COND_STRIDE) };
        const { assert!(parsed::COND_FAIL + 2 <= parsed::COND_STRIDE) };
        // The duration is the middle of the three steps, not a sum of them,
        // and the step it comes from has to exist inside the cap.
        const { assert!(parsed::DURATION_STEP < MAX_STEPS) };
    }

    /// The skill pair reaches the log line, and reaches it in the right order.
    #[test]
    fn the_skill_pair_lands_on_the_mission_line_requirement_first() {
        let line = skilled(0, 1944, 63).log_line();
        assert!(line.contains(" skill=1944/63 "), "{line}");
        // Vanilla has missions with a bonus stat and no requirement (Mining,
        // Ranching): the two fields are independent and the line shows it.
        let bonus_only = skilled(1, parsed::SKILL_NONE, 61).log_line();
        assert!(bonus_only.contains(" skill=-/61 "), "{bonus_only}");
        let neither = op(1, 2, 160, 3, &[]).log_line();
        assert!(neither.contains(" skill=-/- "), "{neither}");
    }

    /// The census the write path would be built on: how many missions are
    /// gated, by which indices, and how many merely pay a bonus.
    #[test]
    fn the_skill_histogram_is_the_gate_census_most_common_first() {
        let mut s = Summary::new();
        for i in 0..58 {
            s.operation(&skilled(i, 1965, 73));
        }
        for i in 0..37 {
            s.operation(&skilled(100 + i, 1962, 72));
        }
        for i in 0..18 {
            s.operation(&skilled(200 + i, 1944, 63));
        }
        // A bonus stat with no requirement, and a mission with neither.
        s.operation(&skilled(300, parsed::SKILL_NONE, 61));
        s.operation(&op(1, 301, 160, 3, &[]));

        assert_eq!(s.ops_with_skill_req, 113);
        assert_eq!(s.ops_with_skill_stat, 114);
        assert_eq!(s.skill_histogram(), vec![(1965, 58), (1962, 37), (1944, 18)]);
        let line = s.skill_line(8);
        assert!(line.contains("113 of 115 missions require a skill (+0xDA)"), "{line}");
        assert!(line.contains("over 3 distinct indices"), "{line}");
        assert!(line.contains("114 carry a bonus stat (+0xD8)"), "{line}");
        assert!(line.ends_with("1965=58 1962=37 1944=18"), "{line}");
        // The cap is a cap, not a silent truncation.
        assert!(s.skill_line(2).ends_with("1965=58 1962=37 (+1 more)"), "{}", s.skill_line(2));
        // A table with no requirements at all says so rather than printing an
        // empty list.
        let mut none = Summary::new();
        none.operation(&op(1, 0, 160, 3, &[]));
        assert!(none.skill_line(8).ends_with("none"), "{}", none.skill_line(8));
    }

    // -----------------------------------------------------------------------
    // The write decisions. None of this was reachable from a native test run
    // until it moved here: `crate::apply` is `#[cfg(windows)]`, so every band,
    // every `derived_*` guard and both `want == 0` locks used to be exercised
    // only by installing the plugin and reading a log.
    // -----------------------------------------------------------------------

    fn mission(dur: u32, skill: u16, ops: u32) -> Mission {
        Mission { key: 17030001, duration_tenths: dur, skill_req: skill, min_operators: ops }
    }

    fn all_on() -> Levers {
        Levers { speed: 4, rewards: 3, clear_skill: true, any_operators: true }
    }

    fn reward(min: i64, max: i64) -> Reward {
        Reward { row: 6244, index: 3, amount_min: min, amount_max: max, item_row: 1701 }
    }

    fn entry_of(r: Reward, kind: u8) -> DropsetEntry {
        DropsetEntry {
            index: usize::from(r.index),
            item_row: r.item_row,
            amount_min: r.amount_min,
            amount_max: r.amount_max,
            weight: 1,
            kind,
            conds: [parsed::dropset::COND_NONE; 3],
        }
    }

    /// A first sight with every lever on: all three fields are asked for, and
    /// each is `lever(vanilla)`.
    #[test]
    fn a_first_sight_plans_all_three_fields() {
        let van = mission(160, 61, 5);
        let p = plan_mission(van, van, &all_on());
        assert_eq!(p.duration, Some(40));
        assert_eq!(p.skill, Some(parsed::SKILL_NONE));
        assert_eq!(p.min_operators, Some(1));
        assert_eq!((p.writes(), p.refused), (3, 0));
    }

    /// A field that already says what it should is not written again - which is
    /// what makes a second pass over a shortened mission cost nothing and, more
    /// to the point, not shorten it twice.
    #[test]
    fn a_field_that_already_says_it_is_not_planned() {
        let van = mission(160, 61, 5);
        let cur = mission(40, parsed::SKILL_NONE, 1);
        let p = plan_mission(van, cur, &all_on());
        assert_eq!(p, MissionPlan::default());
        assert_eq!((p.writes(), p.refused), (0, 0));
        // And with the levers off over an untouched record: still nothing.
        assert_eq!(plan_mission(van, van, &Levers::VANILLA), MissionPlan::default());
    }

    /// Every lever off is a revert, and a revert writes the remembered vanilla
    /// back into whatever the last apply left behind.
    #[test]
    fn the_levers_off_plan_the_vanilla_back() {
        let van = mission(160, 61, 5);
        let cur = mission(40, parsed::SKILL_NONE, 1);
        let p = plan_mission(van, cur, &Levers::VANILLA);
        assert_eq!((p.duration, p.skill, p.min_operators), (Some(160), Some(61), Some(5)));
        assert_eq!(p.refused, 0);
    }

    /// A value this plugin could not have produced from the remembered vanilla
    /// is somebody else's: the field is refused, one field at a time, and the
    /// others in the same entry are unaffected.
    #[test]
    fn a_foreign_field_is_refused_on_its_own() {
        let van = mission(160, 61, 5);
        // Longer than vanilla: nothing here lengthens a mission.
        let p = plan_mission(van, mission(999, 61, 5), &all_on());
        assert_eq!((p.duration, p.refused), (None, 1));
        assert_eq!((p.skill, p.min_operators), (Some(parsed::SKILL_NONE), Some(1)));
        // Neither the vanilla index nor "none".
        let p = plan_mission(van, mission(160, 62, 5), &all_on());
        assert_eq!((p.skill, p.refused), (None, 1));
        // Neither the vanilla count nor 1.
        let p = plan_mission(van, mission(160, 61, 9), &all_on());
        assert_eq!((p.min_operators, p.refused), (None, 1));
        // All three at once is the "wholly refused" reading `crate::apply`
        // reports as skipped rather than unchanged.
        let p = plan_mission(van, mission(999, 62, 9), &all_on());
        assert_eq!((p.writes(), p.refused), (0, 3));
    }

    /// The two `0` locks, from both sides. `docs/reference-internals.md`
    /// section 20.16: a `0` skill index makes a mission permanently
    /// unstartable and a `0` minimum commits every worker the node has, so
    /// neither is ever *invented* - and a vanilla `0` is still put back, which
    /// is the half of the rule a plain "never write 0" guard would break.
    #[test]
    fn no_plan_ever_asks_for_a_zero_it_did_not_find() {
        for van in [mission(160, 0, 0), mission(160, 61, 5), mission(160, 0, 3)] {
            for cur in [van, mission(40, parsed::SKILL_NONE, 1), mission(160, 0, 0)] {
                for clear in [false, true] {
                    for any in [false, true] {
                        let lev = Levers { speed: 4, rewards: 1, clear_skill: clear, any_operators: any };
                        let p = plan_mission(van, cur, &lev);
                        if let Some(v) = p.skill {
                            assert!(v != 0 || van.skill_req == 0, "invented a 0 skill index");
                        }
                        if let Some(v) = p.min_operators {
                            assert!(v != 0 || van.min_operators == 0, "invented a 0 headcount");
                        }
                    }
                }
            }
        }
        // The restore itself: a vanilla `0` in either field is planned back.
        let van = mission(160, 0, 0);
        let p = plan_mission(van, mission(160, parsed::SKILL_NONE, 1), &Levers::VANILLA);
        assert_eq!((p.skill, p.min_operators), (Some(0), Some(0)));
        assert_eq!(p.refused, 0, "a revert to a vanilla zero must not be refused");
    }

    /// The five answers for a reward entry, over the one lever that reaches it.
    #[test]
    fn a_reward_entry_gets_one_of_five_answers() {
        let x3 = Levers { speed: 1, rewards: 3, clear_skill: false, any_operators: false };
        let van = reward(2, 3);
        // Both halves, from the remembered vanilla.
        assert_eq!(
            plan_entry(van, &entry_of(van, 0), &x3),
            RewardPlan::Write { want_min: 6, want_max: 9 }
        );
        // Already scaled: nothing to do, and emphatically not 18..27.
        assert_eq!(plan_entry(van, &entry_of(reward(6, 9), 0), &x3), RewardPlan::Unchanged);
        // `kind == 13` is not an item drop, whatever the lever says.
        assert_eq!(plan_entry(van, &entry_of(van, 13), &x3), RewardPlan::NotAnItemDrop);
        assert_eq!(
            plan_entry(van, &entry_of(van, parsed::dropset::ENTRY_KIND_NOT_AN_ITEM), &Levers::VANILLA),
            RewardPlan::NotAnItemDrop
        );
        // The entry is paying something else now.
        let moved = DropsetEntry { item_row: 1702, ..entry_of(van, 0) };
        assert_eq!(plan_entry(van, &moved, &x3), RewardPlan::ItemMismatch);
        // Not a whole multiple of the vanilla: not ours, so neither half.
        assert_eq!(plan_entry(van, &entry_of(reward(5, 9), 0), &x3), RewardPlan::Refused);
    }

    /// Both halves or neither, and the write order is the one that cannot be
    /// observed inverted. The 136 live entries whose halves differ are why.
    #[test]
    fn a_reward_write_names_both_halves_in_a_safe_order() {
        let x3 = Levers { speed: 1, rewards: 3, clear_skill: false, any_operators: false };
        let van = reward(2, 3);
        let RewardPlan::Write { want_min, want_max } = plan_entry(van, &entry_of(van, 0), &x3) else {
            panic!("a first sight under x3 is a write");
        };
        assert!(want_min <= want_max);
        // Rising: the maximum goes first, so the window between the two stores
        // is wider than either end state rather than inverted.
        assert_eq!(
            amount_write_order(van.amount_max, want_min, want_max),
            [
                (parsed::dropset::ENTRY_AMOUNT_MAX, want_max),
                (parsed::dropset::ENTRY_AMOUNT_MIN, want_min)
            ]
        );
        // Falling, which is what a revert is: the minimum goes first.
        let RewardPlan::Write { want_min, want_max } =
            plan_entry(van, &entry_of(reward(6, 9), 0), &Levers::VANILLA)
        else {
            panic!("a revert of a scaled entry is a write");
        };
        assert_eq!((want_min, want_max), (2, 3));
        assert_eq!(
            amount_write_order(9, want_min, want_max),
            [
                (parsed::dropset::ENTRY_AMOUNT_MIN, want_min),
                (parsed::dropset::ENTRY_AMOUNT_MAX, want_max)
            ]
        );
    }

    /// The first-sight bands: what a vanilla record's values look like, and
    /// what says the offset is not the field. These used to be spelled out
    /// inline in the windows-only write path.
    #[test]
    fn the_first_sight_bands_are_what_a_vanilla_record_looks_like() {
        assert!(vanilla_mission_in_band(mission(DURATION_LO, 61, OPERATORS_LO)));
        assert!(vanilla_mission_in_band(mission(DURATION_HI, 61, OPERATORS_HI)));
        assert!(vanilla_mission_in_band(mission(160, 0, 5)), "the skill index is not banded");
        assert!(!vanilla_mission_in_band(mission(DURATION_LO - 1, 61, 5)));
        assert!(!vanilla_mission_in_band(mission(DURATION_HI + 1, 61, 5)));
        assert!(!vanilla_mission_in_band(mission(0, 61, 5)), "a zero duration is not a mission");
        assert!(!vanilla_mission_in_band(mission(160, 61, OPERATORS_HI + 1)));

        assert!(vanilla_reward_in_band(reward(AMOUNT_LO, AMOUNT_HI)));
        assert!(!vanilla_reward_in_band(reward(0, 3)), "the 34 kind-13 entries pay 0");
        assert!(!vanilla_reward_in_band(reward(2, AMOUNT_HI + 1)));
        assert!(!vanilla_reward_in_band(reward(3, 2)), "an inverted pair is not a pair");
        assert!(!vanilla_reward_in_band(reward(-1, 3)));
    }

    /// The predicate itself is `desert_core::manager`'s and is tested there,
    /// over both ends of the band. What is worth asserting here is that the
    /// name this crate uses ~30 times still means that: a re-export that
    /// drifted would be silent.
    #[test]
    fn the_plausible_this_crate_uses_is_cores() {
        assert!(!plausible(0));
        assert!(plausible(0x1_4000_0000));
        assert!(!plausible(usize::MAX));
        for p in [0usize, 0xFFFF, 0x10000, 0x1_4000_0000, 0x7FFF_FFFF_0000, usize::MAX] {
            assert_eq!(plausible(p), desert_core::manager::plausible(p), "0x{p:X}");
        }
    }

    #[test]
    fn tenths_of_an_hour_render_as_hours() {
        assert_eq!(hours(0), "0.0");
        assert_eq!(hours(5), "0.5");
        assert_eq!(hours(20), "2.0");
        assert_eq!(hours(160), "16.0");
        assert_eq!(hours(240), "24.0");
        assert_eq!(hours(540), "54.0");
        assert_eq!(hours(960), "96.0");
        // Nothing rounds, because nothing is a float: an odd value keeps its
        // tenth rather than being smoothed into a neighbouring hour.
        assert_eq!(hours(163), "16.3");
        assert_eq!(hours(DURATION_HI), "432.0");
    }

    #[test]
    fn the_log_line_is_greppable_and_carries_every_field() {
        let line = op(1000401, 2, 160, 3, &[(11, 1, 4200), (12, 0, 4201)]).log_line();
        assert_eq!(
            line,
            concat!(
                "op node=1000401 idx=2 key=902 group=7 dur=160(16.0h) ops=3..5 cp=1234 ",
                "steps=3 c0=1 skill=-/- drop=-/- conds=2 [11/1/4200, 12/0/4201]"
            )
        );
        // A mission with no conditions says so and prints no list at all.
        let bare = op(1, 0, 40, 2, &[]).log_line();
        assert!(bare.ends_with("conds=0"), "{bare}");
        assert!(!bare.contains('['), "{bare}");
    }

    /// The duration must be readable as a number *and* as a time, because the
    /// raw tenths are what gets compared against DMM's list and the hours are
    /// what a person reads. Both, or the line is not doing its job.
    #[test]
    fn the_log_line_prints_the_duration_raw_and_in_hours() {
        let line = op(1, 0, 540, 2, &[]).log_line();
        assert!(line.contains("dur=540(54.0h)"), "{line}");
        // `+0xC0` is still reported, under a name that claims nothing.
        assert!(line.contains(" c0=1 "), "{line}");
        assert!(!line.contains("cost="), "the step list is not a cost: {line}");
    }

    #[test]
    fn an_empty_summary_says_it_has_nothing() {
        let s = Summary::new();
        assert_eq!(s.ops, 0);
        assert!(s.verdict().contains("NO DATA"), "{}", s.verdict());
        assert_eq!(s.histogram_line(8), "none");
        assert_eq!(s.duration_range(), None);
        assert_eq!(s.duration_median(), None);
        assert!(s.skips.is_empty());
        assert_eq!(s.skips.summary(), "");
    }

    #[test]
    fn the_accumulator_counts_records_operations_and_conditions() {
        let mut s = Summary::new();
        s.record(true);
        s.record_with_ops();
        s.record(false);
        s.record(true);
        s.operation(&op(1, 0, 50, 3, &[(11, 1, 0), (12, 0, 0)]));
        s.operation(&op(1, 1, 70, 3, &[(11, 1, 0)]));
        s.operation(&op(2, 0, 90, 1, &[]));

        assert_eq!((s.records, s.records_read, s.records_with_ops()), (3, 2, 1));
        assert_eq!(s.ops, 3);
        assert_eq!(s.ops_without_conds, 1);
        assert_eq!(s.cond_entries, 3);
        assert_eq!(s.duration_range(), Some((50, 90)));
        assert_eq!(s.duration_median(), Some(70));
        assert_eq!(s.operator_range(), Some((1, 3)));
        assert_eq!(s.operator_median(), Some(3));
    }

    #[test]
    fn the_histogram_is_the_condition_census_most_common_first() {
        let mut s = Summary::new();
        for _ in 0..5 {
            s.operation(&op(1, 0, 30, 2, &[(20, 1, 0)]));
        }
        for _ in 0..9 {
            s.operation(&op(1, 1, 30, 2, &[(31, 1, 0)]));
        }
        s.operation(&op(1, 2, 30, 2, &[(7, 1, 0), (20, 1, 0)]));

        assert_eq!(s.condition_histogram(), vec![(31, 9), (20, 6), (7, 1)]);
        assert_eq!(s.histogram_line(8), "31=9 20=6 7=1");
        // The cap is a cap, not a silent truncation.
        assert_eq!(s.histogram_line(2), "31=9 20=6 (+1 more)");
    }

    #[test]
    fn the_histogram_ties_break_by_key_so_two_runs_agree() {
        let mut a = Summary::new();
        a.operation(&op(1, 0, 30, 2, &[(9, 0, 0)]));
        a.operation(&op(1, 1, 30, 2, &[(4, 0, 0)]));
        let mut b = Summary::new();
        b.operation(&op(1, 0, 30, 2, &[(4, 0, 0)]));
        b.operation(&op(1, 1, 30, 2, &[(9, 0, 0)]));
        assert_eq!(a.histogram_line(8), b.histogram_line(8));
        assert_eq!(a.histogram_line(8), "4=1 9=1");
    }

    #[test]
    fn the_verdict_calls_plausible_values_right() {
        let mut s = Summary::new();
        for i in 0..50 {
            s.operation(&op(1, i, 20 + (i as u32 % 12) * 10, (i as u32) % 5, &[]));
        }
        let v = s.verdict();
        assert!(v.contains("LOOKS RIGHT"), "{v}");
        assert!(v.contains("50/50 durations"), "{v}");
        assert!(v.contains("(100%)"), "{v}");
    }

    #[test]
    fn the_verdict_calls_32_bit_noise_wrong() {
        // What a wrong offset actually produces: pointer halves, flags, junk.
        let mut s = Summary::new();
        for i in 0..40u32 {
            s.operation(&op(1, i as usize, 0x1400_0000 + i, 0xFFFF_0000 + i, &[]));
        }
        let v = s.verdict();
        assert!(v.contains("LOOKS WRONG"), "{v}");
        assert!(v.contains("0/40 durations"), "{v}");
    }

    #[test]
    fn the_verdict_refuses_to_commit_when_only_one_field_moved() {
        // Durations fine, operator counts noise: neither "right" nor a clean
        // "wrong", and the sentence has to say so rather than pick a side.
        let mut s = Summary::new();
        for i in 0..40u32 {
            let bad = if i % 4 == 0 { 3 } else { 0x7FFF_0000 + i };
            s.operation(&op(1, i as usize, 20 + (i % 10) * 10, bad, &[]));
        }
        let v = s.verdict();
        assert!(v.contains("LOOKS WRONG"), "{v}");

        let mut s = Summary::new();
        for i in 0..40u32 {
            let bad = if i % 5 == 0 { 0x7FFF_0000 + i } else { 3 };
            s.operation(&op(1, i as usize, 20 + (i % 10) * 10, bad, &[]));
        }
        let v = s.verdict();
        assert!(v.contains("UNCONVINCING"), "{v}");
        assert!(v.contains("32/40 minimum operator counts"), "{v}");
    }

    #[test]
    fn zero_operators_counts_as_in_range() {
        // `+0xC4 = 0` is a real, legal value with its own meaning (and it is
        // *stricter* than 1), so it must not drag the verdict down.
        let mut s = Summary::new();
        for i in 0..20 {
            s.operation(&op(1, i, 30, 0, &[]));
        }
        assert_eq!(s.in_range_counts(), (20, 20));
        assert!(s.verdict().contains("LOOKS RIGHT"));
    }

    #[test]
    fn the_counts_line_carries_the_ranges_and_the_medians() {
        let mut s = Summary::new();
        s.record(true);
        s.record_with_ops();
        s.operation(&op(1, 0, 20, 1, &[(11, 1, 0)]));
        s.operation(&op(1, 1, 80, 4, &[]));
        let line = s.counts_line();
        assert!(line.contains("records 1 (1 loaded, 1 with missions)"), "{line}");
        assert!(line.contains("missions 2"), "{line}");
        assert!(line.contains("duration 20..80 tenths of an hour (2.0h..8.0h"), "{line}");
        assert!(line.contains("median 80 = 8.0h"), "{line}");
        assert!(line.contains("+0xC0 1..1"), "{line}");
        assert!(line.contains("min operators 1..4"), "{line}");
        assert!(line.contains("missions with no conditions 1"), "{line}");
        assert!(line.contains("distinct condition keys 1"), "{line}");
    }

    #[test]
    fn skips_name_only_the_counters_that_moved() {
        let mut k = Skips::default();
        assert!(k.is_empty());
        k.unloaded = 3;
        k.op_read = 1;
        assert!(!k.is_empty());
        assert_eq!(k.summary(), "unloaded=3 op_read=1");
    }

    /// The check that caught the mislabelled field, pinned.
    ///
    /// `entry+0xC0` read `1` on all 936 vanilla missions and sailed through
    /// the old `1..=100` range test; only "every value is identical" said
    /// anything was wrong. The wording matters as much as the check, and it
    /// used to blame a wrong offset or a mod that edits the table on disk while
    /// never naming **this plugin**, whose own levers flatten two of the three
    /// fields the verdict measures. The likeliest cause has to be named first
    /// or the first bug report is about an offset that never moved.
    #[test]
    fn a_constant_duration_blames_this_plugins_own_settings_first() {
        let mut s = Summary::new();
        for i in 0..936 {
            s.operation(&op(1, i, 160, 3, &[]));
        }
        assert_eq!(s.constant_duration(), Some(160));
        let v = s.verdict();
        assert!(v.contains("CONSTANT: every one of 936 missions reports duration 160"), "{v}");
        assert!(v.contains("Check this plugin's own settings first"), "{v}");
        assert!(v.contains("Speed high enough to floor every mission"), "{v}");
        // The other two causes are still named, in order, after ours.
        assert!(v.contains("then a mod that edits the table on disk"), "{v}");
        assert!(v.contains("only then a wrong offset"), "{v}");
        // In range and constant at once: the range test alone would have said
        // LOOKS RIGHT, which is exactly the trap.
        assert!(v.contains("LOOKS RIGHT"), "{v}");

        // One sample is not a constant, and neither is a field that varies.
        let mut one = Summary::new();
        one.operation(&op(1, 0, 160, 3, &[]));
        assert_eq!(one.constant_duration(), None);
        let mut varied = Summary::new();
        varied.operation(&op(1, 0, 160, 3, &[]));
        varied.operation(&op(1, 1, 120, 4, &[]));
        assert_eq!(varied.constant_duration(), None);
        assert!(!varied.verdict().contains("CONSTANT"), "{}", varied.verdict());
    }

    /// `+0xC0` is still read and still watched, but it is not judged and it is
    /// not called a duration anywhere.
    #[test]
    fn the_c0_flag_is_watched_not_judged() {
        let mut s = Summary::new();
        for i in 0..936 {
            s.operation(&op(1, i, 20 + (i as u32 % 40) * 10, 1 + (i as u32 % 4), &[]));
        }
        // Every synthetic op carries flag_c0 = 1, the vanilla reading.
        assert_eq!(s.constant_flag_c0(), Some(1));
        assert_eq!(s.flag_c0_range(), Some((1, 1)));
        // It never drags the verdict anywhere, constant though it is.
        let v = s.verdict();
        assert!(v.contains("LOOKS RIGHT"), "{v}");
        assert!(!v.contains("CONSTANT"), "{v}");
    }

    /// A whole vanilla-shaped table, built to
    /// [`VANILLA_DISTRIBUTION`]: 936 missions, every duration a multiple of
    /// ten tenths, `20..=960`, and exactly 47 of them above 240 in the
    /// multiset DMM's "Long Mission to 24HR" lists.
    #[test]
    fn a_vanilla_like_distribution_reads_as_a_correct_layout() {
        // The 47 missions longer than 24 h, as DMM counts them.
        let long: Vec<u32> =
            [(540, 19), (720, 9), (360, 7), (480, 7), (300, 2), (960, 2), (900, 1)]
                .iter()
                .flat_map(|&(v, n)| std::iter::repeat_n(v, n))
                .collect();
        assert_eq!(long.len(), 47, "DMM lists 47 missions longer than 24h");

        // The bulk: the three commonest values, then a spread of shorter ones
        // to make the table up to 936.
        let mut durations: Vec<u32> = Vec::new();
        durations.extend(std::iter::repeat_n(160u32, 298));
        durations.extend(std::iter::repeat_n(120u32, 237));
        durations.extend(std::iter::repeat_n(80u32, 196));
        for i in 0..158u32 {
            durations.push(20 + (i % 22) * 10);
        }
        durations.extend(long.iter().copied());
        assert_eq!(durations.len(), 936);

        let mut s = Summary::new();
        for (i, &d) in durations.iter().enumerate() {
            s.operation(&op(1_000_401, i, d, 1 + (i as u32 % 4), &[(498, 1, 4200)]));
        }

        assert_eq!(s.ops, 936);
        assert_eq!(s.duration_range(), Some((20, 960)));
        assert_eq!(s.duration_median(), Some(120));
        assert_eq!(s.constant_duration(), None);
        assert!(durations.iter().all(|d| d % 10 == 0), "every vanilla duration is a whole hour");
        assert_eq!(durations.iter().filter(|&&d| d > 240).count(), 47);

        // Every one of them is inside the band, and the verdict says so
        // without a CONSTANT caveat.
        assert_eq!(s.in_range_counts().0, 936);
        let v = s.verdict();
        assert!(v.contains("LOOKS RIGHT"), "{v}");
        assert!(v.contains("936/936 durations in 10..=4320 tenths of an hour (100%)"), "{v}");
        assert!(!v.contains("CONSTANT"), "{v}");

        let line = s.counts_line();
        assert!(line.contains("duration 20..960 tenths of an hour (2.0h..96.0h"), "{line}");
        assert!(line.contains("median 120 = 12.0h"), "{line}");
    }

    /// The band is the real duration's, in tenths of an hour - not the old
    /// `1..=100` day count that `entry+0xC0` passed while being wrong.
    #[test]
    fn the_duration_band_is_tenths_of_an_hour() {
        assert_eq!((DURATION_LO, DURATION_HI), (10, 4_320));
        assert_eq!(
            (hours(DURATION_LO), hours(DURATION_HI)),
            ("1.0".to_string(), "432.0".to_string())
        );
        // The whole vanilla range fits, with room on both sides.
        const { assert!(DURATION_LO < 20 && DURATION_HI > 960) };
        // A day count of 1, which is what the wrong offset produced, is now
        // below the floor rather than comfortably inside it.
        let mut s = Summary::new();
        for i in 0..936 {
            s.operation(&op(1, i, 1, 3, &[]));
        }
        assert_eq!(s.in_range_counts().0, 0);
        assert!(s.verdict().contains("LOOKS WRONG"), "{}", s.verdict());
    }

    // -----------------------------------------------------------------------
    // The reward rows. Every offset behind these is decompilation-only, so
    // what is tested here is the *judgement* over them: that a plausible dump
    // reads as right, that a wrong-offset dump reads as wrong, and that a
    // field which never varies is called out either way.
    // -----------------------------------------------------------------------

    /// The bytes of one `dropsetinfo` entry, laid out at the offsets
    /// `mod parsed::dropset` claims. `(min, max)` is the amount pair, `item`
    /// the `+0x38` row and `kind` the `+0x60` byte.
    fn dropset_entry_bytes(
        min: i64,
        max: i64,
        item: u32,
        kind: u8,
    ) -> [u8; parsed::dropset::ENTRY_RAW] {
        use parsed::dropset;
        let mut b = [0u8; dropset::ENTRY_RAW];
        put(&mut b, dropset::ENTRY_CONDS[0], &dropset::COND_NONE.to_le_bytes());
        put(&mut b, dropset::ENTRY_CONDS[1], &12u16.to_le_bytes());
        put(&mut b, dropset::ENTRY_CONDS[2], &dropset::COND_NONE.to_le_bytes());
        put(&mut b, dropset::ENTRY_WEIGHT, &100_000i64.to_le_bytes());
        put(&mut b, dropset::ENTRY_AMOUNT_MIN, &min.to_le_bytes());
        put(&mut b, dropset::ENTRY_AMOUNT_MAX, &max.to_le_bytes());
        put(&mut b, dropset::ENTRY_ITEM, &item.to_le_bytes());
        put(&mut b, dropset::ENTRY_KIND, &[kind]);
        b
    }

    /// The reward offsets, tested by decoding bytes rather than by restating
    /// the constants - see the mission-side twin for why a mirror test could
    /// not fail.
    ///
    /// The three values checked here are the three the live capture of
    /// 2026-09-10 corrected: the amount is a pair, `kind == 13` is not an item
    /// drop, and the item row is a `u32`.
    #[test]
    fn a_dropset_entrys_bytes_decode_to_the_pair_the_live_capture_found() {
        use parsed::dropset;

        // A real vanilla shape: a 2..3 flower drop.
        let e = decode_entry(4, &dropset_entry_bytes(2, 3, 4242, 0)).expect("a full entry decodes");
        assert_eq!(e.index, 4, "the index is the caller's, not the bytes'");
        assert_eq!((e.amount_min, e.amount_max), (2, 3));
        assert_eq!(e.item_row, 4242);
        assert_eq!(e.weight, 100_000);
        assert_eq!(e.kind, 0);
        assert_eq!(e.conds, [dropset::COND_NONE, 12, dropset::COND_NONE]);
        assert!(e.is_item_drop());
        assert!(e.log_line(6244).contains("amount=2..3"), "{}", e.log_line(6244));

        // The 34 entries of the live capture that are not item drops: amount
        // zero, and a `+0x38` a `u16` read would have truncated to 1.
        let odd = decode_entry(0, &dropset_entry_bytes(0, 0, 0x1_0001, dropset::ENTRY_KIND_NOT_AN_ITEM))
            .expect("decodes");
        assert_eq!(odd.item_row, 0x1_0001, "+0x38 is a u32; a u16 read truncates 34 entries");
        assert_eq!((odd.amount_min, odd.amount_max), (0, 0));
        assert!(!odd.is_item_drop(), "kind 13 is not an item drop");
        assert!(odd.log_line(1).ends_with("(not an item drop)"), "{}", odd.log_line(1));

        // A buffer too short for the kind byte decodes to nothing.
        let short = dropset_entry_bytes(1, 1, 7, 0);
        assert_eq!(decode_entry(0, &short[..dropset::ENTRY_KIND]), None);
        // The two halves of the pair are distinct offsets, and swapping them
        // is visible: a decoder reading one field twice would pass every test
        // above but not this one.
        let wide = decode_entry(0, &dropset_entry_bytes(4, 7, 9, 0)).expect("decodes");
        assert_ne!(wide.amount_min, wide.amount_max);

        // The reward-set fields, and the raw dumps reaching past the last
        // named field of each object.
        const { assert!(parsed::OP_REWARD_2 + 2 <= parsed::OP_STRIDE) };
        const { assert!(dropset::ROW_RAW > dropset::ROW_NO_DROP_PPM + 8) };
        const { assert!(dropset::ENTRY_RAW > dropset::ENTRY_KIND) };
        const { assert!(dropset::ENTRY_AMOUNT_MIN < dropset::ENTRY_AMOUNT_MAX) };
        // Everything the entry decoder reads is inside the block it reads.
        const { assert!(dropset::ENTRY_AMOUNT_MAX + 8 <= dropset::ENTRY_RAW) };
        const { assert!(dropset::ENTRY_ITEM + 4 <= dropset::ENTRY_RAW) };
    }

    #[test]
    fn the_vanilla_reward_census_is_the_one_the_dump_confirmed() {
        assert_eq!((REWARD_OPS_SET1, REWARD_ROWS_SET1), (243, 208));
        assert_eq!((REWARD_OPS_SET2, REWARD_ROWS_SET2), (18, 13));
        assert_eq!(REWARD_ROWS_UNION, 219);
        // The union is smaller than the sum because the sets overlap - a mod
        // that scaled 221 rows would be scaling two rows nothing names.
        const { assert!(REWARD_ROWS_UNION < REWARD_ROWS_SET1 + REWARD_ROWS_SET2) };
    }

    #[test]
    fn a_missions_reward_rows_land_on_its_log_line_with_a_dash_for_none() {
        assert_eq!(row_text(parsed::REWARD_NONE), "-");
        assert_eq!(row_text(0), "0");
        assert_eq!(row_text(6244), "6244");
        let line = rewarded(0, 6244, parsed::REWARD_NONE).log_line();
        assert!(line.contains(" drop=6244/- "), "{line}");
        let both = rewarded(1, 12, 6244).log_line();
        assert!(both.contains(" drop=12/6244 "), "{both}");
    }

    #[test]
    fn the_requested_row_set_is_the_union_of_the_two_reward_fields() {
        let mut s = Summary::new();
        s.operation(&rewarded(0, 10, parsed::REWARD_NONE));
        s.operation(&rewarded(1, 10, parsed::REWARD_NONE)); // same row twice
        s.operation(&rewarded(2, 11, 10)); // set 2 repeats a set 1 row
        s.operation(&rewarded(3, parsed::REWARD_NONE, 12));
        s.operation(&op(1, 4, 160, 3, &[])); // no rewards at all

        assert_eq!((s.ops_with_reward1, s.ops_with_reward2), (3, 2));
        assert_eq!(s.reward_rows(), vec![10, 11, 12]);
        assert_eq!(s.reward_row_counts(), (2, 2, 3));
        // The sentinel is never a row: a mission that pays nothing must not
        // put row 65535 into the set a multiplier would edit.
        assert!(!s.reward_rows().contains(&parsed::REWARD_NONE));
    }

    #[test]
    fn the_request_line_states_the_vanilla_numbers_beside_the_measured_ones() {
        let mut s = Summary::new();
        s.operation(&rewarded(0, 10, 11));
        let line = s.reward_request_line();
        assert!(line.starts_with("2 distinct dropset rows requested"), "{line}");
        assert!(line.contains("1 from +0xA8 on 1 missions"), "{line}");
        assert!(line.contains("1 from +0xAA on 1 missions"), "{line}");
        // The expected figures are on the same line as the measured ones, so a
        // wrong count is visible without a second tool.
        assert!(line.contains("vanilla build 25116796 reads 219 (208 from 243, 13 from 18)"), "{line}");
        assert!(line.ends_with("DIFFERS, see the missions above"), "{line}");

        // A vanilla-shaped census agrees, and says so.
        let mut v = Summary::new();
        for i in 0..REWARD_OPS_SET1 {
            let row = (i % REWARD_ROWS_SET1) as u16;
            let second = if i < REWARD_OPS_SET2 {
                (1000 + i % REWARD_ROWS_SET2) as u16
            } else {
                parsed::REWARD_NONE
            };
            v.operation(&rewarded(i, row, second));
        }
        assert_eq!(v.reward_row_counts(), (208, 13, 221));
        let line = v.reward_request_line();
        assert!(line.contains("221 distinct dropset rows requested"), "{line}");
        // 221 is not 219: the sets overlapping is part of the expectation, so
        // a synthetic census that keeps them disjoint must NOT read as agreeing.
        assert!(line.ends_with("DIFFERS, see the missions above"), "{line}");
    }

    #[test]
    fn the_dropset_log_lines_are_greppable_and_carry_every_field() {
        let row = drow(6244, 2);
        assert_eq!(row.log_line(), "drop row=6244 entries=2(decl=2) draws=1 nodrop=0ppm");
        assert_eq!(
            row.entries[0].log_line(6244),
            "dropentry row=6244 idx=0 item=10244 amount=1..1 weight=10001 kind=1 conds=-/-/-"
        );
        // A row whose declared count and read count disagree says so rather
        // than quietly printing the shorter number.
        let short = DropsetRow { declared_entries: 9, ..drow(7, 2) };
        assert!(short.log_line().contains("entries=2(decl=9)"), "{}", short.log_line());
        // Real condition keys print as numbers.
        let conditional = DropsetEntry { conds: [3, 0xFFFF, 9], ..entry(1, 42, 5) };
        assert!(conditional.log_line(7).ends_with("conds=3/-/9"), "{}", conditional.log_line(7));
    }

    #[test]
    fn an_empty_reward_summary_says_the_rows_are_not_loaded_yet() {
        let mut s = RewardSummary::new(219);
        s.rows_null = 219;
        let v = s.verdict();
        assert!(v.contains("NO REWARD DATA"), "{v}");
        assert!(v.contains("219 rows requested, none loaded yet"), "{v}");
        // A row that has not loaded is normal, and the sentence must say so or
        // the next reader will go looking for a bug that is not there.
        assert!(v.contains("load on demand"), "{v}");
        assert_eq!(s.amount_range(), None);
        assert_eq!(s.kind_line(), "none");
    }

    #[test]
    fn rows_that_loaded_but_carry_no_entries_blame_the_right_two_offsets() {
        let mut s = RewardSummary::new(4);
        s.row(&drow(1, 0));
        s.row(&drow(2, 0));
        let v = s.verdict();
        assert!(v.contains("NO REWARD DATA"), "{v}");
        assert!(v.contains("record+0x30"), "{v}");
        assert!(v.contains("record+0x28"), "{v}");
    }

    #[test]
    fn a_plausible_reward_dump_reads_as_a_correct_layout() {
        let mut s = RewardSummary::new(REWARD_ROWS_UNION);
        for i in 0..REWARD_ROWS_UNION {
            s.row(&drow(i as u16, 1 + i % 4));
        }
        assert_eq!(s.rows_loaded, 219);
        assert!(s.entries > 219);
        let (alo, ahi) = s.amount_range().unwrap();
        assert!(alo >= AMOUNT_LO && ahi <= AMOUNT_HI);
        let v = s.verdict();
        assert!(v.contains("the dropset layout LOOKS RIGHT"), "{v}");
        assert!(v.contains("(100%)"), "{v}");
        // Nothing the bands cover is flat, and the sentence must not say it is.
        assert!(!v.contains("reports amount"), "{v}");
        assert!(!v.contains("reports weight"), "{v}");
        assert!(!v.contains("reports item row"), "{v}");
        // The synthetic rows carry one draw count and no conditions, exactly
        // as the live capture's 219 rows did, so those constants are reported.
        assert!(v.contains("CONSTANT: every one of 219 rows reports draw count 1"), "{v}");
        assert!(v.contains("reports no-drop chance 0ppm"), "{v}");
        assert!(v.contains("reports condition slot 0 0xFFFF"), "{v}");

        let line = s.counts_line();
        assert!(line.contains("rows requested 219, loaded 219"), "{line}");
        assert!(line.contains("entries per row 1..4"), "{line}");
        assert!(line.contains("draws 1..1"), "{line}");
        assert!(line.contains("kinds 1="), "{line}");
    }

    #[test]
    fn the_reward_verdict_calls_64_bit_noise_wrong() {
        // What a wrong offset actually produces: pointer halves and junk, and
        // a "count" that is nothing of the kind.
        let mut s = RewardSummary::new(40);
        for i in 0..40u16 {
            let junk = DropsetEntry {
                amount_min: 0x0000_7FF6_1234_0000 + i64::from(i),
                amount_max: 0x0000_7FF6_1234_0000 + i64::from(i),
                item_row: 0,
                ..entry(0, 0, 1)
            };
            s.row(&DropsetRow {
                row: i,
                draws: 0,
                declared_entries: 1,
                no_drop_ppm: 0,
                entries: vec![junk],
            });
        }
        let v = s.verdict();
        assert!(v.contains("the dropset layout LOOKS WRONG"), "{v}");
        assert!(v.contains("0/40 amounts"), "{v}");
        assert!(v.contains("0/40 entries name a real item row"), "{v}");
    }

    #[test]
    fn the_reward_verdict_refuses_to_commit_when_only_one_field_moved() {
        // Amounts fine, item rows mostly missing: neither a clean right nor a
        // clean wrong, and the sentence has to say so.
        let mut s = RewardSummary::new(40);
        for i in 0..40u16 {
            let item: u16 = if i % 5 == 0 { 0 } else { 4000 + i };
            s.row(&DropsetRow {
                row: i,
                draws: 1,
                declared_entries: 1,
                no_drop_ppm: 0,
                entries: vec![DropsetEntry { item_row: u32::from(item), ..entry(0, u32::from(item), 3) }],
            });
        }
        let v = s.verdict();
        assert!(v.contains("UNCONVINCING"), "{v}");
        assert!(v.contains("32/40 entries name a real item row"), "{v}");
    }

    /// The check that caught `entry+0xC0`, aimed at the reward fields.
    ///
    /// An amount that reads the same on all 219 rows passes every band test
    /// while saying nothing, which is exactly the trap the operation side fell
    /// into for a session. The wording must not pick a cause: a flat field can
    /// also be a mod's doing.
    #[test]
    fn a_constant_reward_field_is_called_out_without_picking_a_cause() {
        let mut s = RewardSummary::new(REWARD_ROWS_UNION);
        for i in 0..REWARD_ROWS_UNION {
            s.row(&DropsetRow {
                row: i as u16,
                draws: 1,
                declared_entries: 1,
                no_drop_ppm: 0,
                // Amount and weight identical everywhere, item row varying.
                entries: vec![DropsetEntry { weight: 10_000, ..entry(0, 4000 + i as u32, 1) }],
            });
        }
        assert_eq!(s.constant_amount_min(), Some(1));
        assert_eq!(s.constant_amount_max(), Some(1));
        assert_eq!(s.constant_weight(), Some(10_000));
        assert_eq!(s.constant_item_row(), None);
        let v = s.verdict();
        // In band and constant at once: the band test alone says LOOKS RIGHT,
        // which is the trap.
        assert!(v.contains("LOOKS RIGHT"), "{v}");
        assert!(v.contains("CONSTANT: every one of 219 entries reports amount min 1"), "{v}");
        assert!(v.contains("CONSTANT: every one of 219 entries reports amount max 1"), "{v}");
        assert!(v.contains("reports weight 10000"), "{v}");
        assert!(v.contains("either at the wrong offset or genuinely flat"), "{v}");

        // One entry is not a constant, and neither is a field that varies.
        let mut one = RewardSummary::new(1);
        one.row(&drow(1, 1));
        assert_eq!(one.constant_amount_min(), None);
        assert_eq!(one.constant_item_row(), None);
        let mut varied = RewardSummary::new(2);
        varied.row(&drow(1, 3));
        assert_eq!(varied.constant_amount_min(), None);
        let v = varied.verdict();
        assert!(!v.contains("reports amount"), "{v}");
        assert!(!v.contains("reports weight"), "{v}");
    }

    #[test]
    fn one_item_row_across_a_whole_table_is_a_constant_too() {
        let mut s = RewardSummary::new(20);
        for i in 0..20u16 {
            s.row(&DropsetRow {
                row: i,
                draws: 1,
                declared_entries: 1,
                no_drop_ppm: 0,
                entries: vec![entry(0, 4242, 1 + i64::from(i))],
            });
        }
        assert_eq!(s.constant_item_row(), Some(4242));
        assert_eq!(s.distinct_item_rows(), 1);
        assert!(s.verdict().contains("reports item row 4242"), "{}", s.verdict());
    }

    /// The vanilla reward census of 2026-09-10, rebuilt from the capture's own
    /// numbers: 219 rows, 725 entries, the kind census `{0: 633, 6: 35,
    /// 13: 34, 4: 16, 1: 7}`, 136 entries whose amount pair differs, and 34
    /// that are not item drops at all.
    ///
    /// The row-level fields are flat because they were flat in the capture -
    /// that is what makes them UNCONFIRMED - and the entry conditions are
    /// `0xFFFF` in slots 0 and 2 for the same reason.
    fn live_shaped_census() -> RewardSummary {
        let mut entries: Vec<DropsetEntry> = Vec::new();
        let mut differing = 0usize;
        for (kind, n) in [(0u8, 633usize), (6, 35), (13, 34), (4, 16), (1, 7)] {
            for i in 0..n {
                let j = entries.len();
                if kind == parsed::dropset::ENTRY_KIND_NOT_AN_ITEM {
                    // Zero amount, and an item field too wide for a u16: the
                    // capture's 34 kind-13 entries were exactly this set.
                    entries.push(DropsetEntry {
                        index: i,
                        item_row: 0x1_0000 + i as u32,
                        amount_min: 0,
                        amount_max: 0,
                        weight: 1_000 * (1 + (j % 31)) as i64,
                        kind,
                        conds: [0xFFFF, (j % 20) as u16, 0xFFFF],
                    });
                    continue;
                }
                let min = 1 + (j % 9) as i64;
                // 136 of 725 pay a range rather than a fixed amount.
                let max = if differing < 136 {
                    differing += 1;
                    min + 1
                } else {
                    min
                };
                entries.push(DropsetEntry {
                    index: i,
                    item_row: 1_000 + (j % 421) as u32,
                    amount_min: min,
                    amount_max: max,
                    weight: 1_000 * (1 + (j % 31)) as i64,
                    kind,
                    conds: [0xFFFF, (j % 20) as u16, 0xFFFF],
                });
            }
        }
        assert_eq!(entries.len(), 725);
        assert_eq!(differing, 136);

        // 219 rows holding all 725 entries: 68 rows of four and 151 of three.
        let mut sum = RewardSummary::new(REWARD_ROWS_UNION);
        let mut it = entries.into_iter();
        for row in 0..REWARD_ROWS_UNION {
            let n = if row < 68 { 4 } else { 3 };
            let taken: Vec<DropsetEntry> = it.by_ref().take(n).collect();
            sum.row(&DropsetRow {
                row: row as u16,
                draws: 0,
                declared_entries: taken.len() as u32,
                no_drop_ppm: 0,
                entries: taken,
            });
        }
        assert_eq!(sum.entries, 725);
        sum
    }

    /// A dump shaped like the real vanilla capture reads as a correct layout -
    /// **and** the four fields that capture read as one flat value each are
    /// called out, because those four are precisely what the constant check
    /// exists to find and precisely what the first version of this code
    /// accumulated without ever checking.
    #[test]
    fn the_live_vanilla_census_reads_right_and_flags_the_four_flat_fields() {
        let s = live_shaped_census();
        assert_eq!(s.entries, 725);
        assert_eq!(s.entries_not_items, 34, "the kind-13 entries");
        assert_eq!(s.ordered_amounts(), (725, 725), "min <= max on every entry");
        assert_eq!(s.rows_loaded, REWARD_ROWS_UNION);

        let v = s.verdict();
        // 691 of 725: the 34 that are not item drops pay 0 and fall out of
        // band on purpose, which is why a correct dump reads 95%, not 100%.
        assert!(v.contains("691/725 amounts in 1..=1000000 (95%)"), "{v}");
        assert!(v.contains("the dropset layout LOOKS RIGHT"), "{v}");
        // The confirmed fields all vary, so none of them is flagged.
        assert!(!v.contains("reports amount"), "{v}");
        assert!(!v.contains("reports weight"), "{v}");
        assert!(!v.contains("reports item row"), "{v}");
        // The four UNCONFIRMED ones are.
        assert!(v.contains("CONSTANT: every one of 219 rows reports draw count 0"), "{v}");
        assert!(v.contains("CONSTANT: every one of 219 rows reports no-drop chance 0ppm"), "{v}");
        assert!(v.contains("CONSTANT: every one of 725 entries reports condition slot 0 0xFFFF"), "{v}");
        assert!(v.contains("CONSTANT: every one of 725 entries reports condition slot 2 0xFFFF"), "{v}");

        let line = s.counts_line();
        assert!(line.contains("min<=max on 725/725"), "{line}");
        assert!(line.contains("not item drops 34"), "{line}");
        // The kind census the capture recorded, ascending.
        assert!(line.contains("kinds 0=633 1=7 4=16 6=35 13=34"), "{line}");
        // And the two halves of the amount are reported as two ranges: a
        // table scaled by halves shows up as the bands moving apart.
        // Both bands start at 0 rather than 1, and only because of the 34
        // entries that are not item drops: theirs is the whole of the zero.
        assert_eq!(s.amount_range(), Some((0, 9)));
        assert_eq!(s.amount_max_range(), Some((0, 10)));
    }

    /// The same census with every entry paying one fixed amount: still in
    /// band, still LOOKS RIGHT on the bands alone, and caught anyway. This is
    /// the `entry+0xC0` lesson applied to both halves of the pair.
    #[test]
    fn a_census_with_one_amount_everywhere_trips_the_constant_check() {
        let mut s = RewardSummary::new(REWARD_ROWS_UNION);
        for row in 0..REWARD_ROWS_UNION {
            let entries: Vec<DropsetEntry> = (0..3)
                .map(|i| DropsetEntry {
                    index: i,
                    item_row: 1_000 + (row * 3 + i) as u32,
                    amount_min: 1,
                    amount_max: 1,
                    weight: 1_000 * (1 + (row % 31)) as i64,
                    kind: 0,
                    conds: [0xFFFF, (row % 20) as u16, 0xFFFF],
                })
                .collect();
            s.row(&DropsetRow {
                row: row as u16,
                draws: 1 + (row % 3) as i32,
                no_drop_ppm: row as i64,
                declared_entries: entries.len() as u32,
                entries,
            });
        }
        let v = s.verdict();
        // The band test alone says the layout is fine, which is the trap.
        assert!(v.contains("LOOKS RIGHT"), "{v}");
        assert!(v.contains("(100%)"), "{v}");
        assert!(v.contains("reports amount min 1"), "{v}");
        assert!(v.contains("reports amount max 1"), "{v}");
        // The row fields vary here, so they are not flagged: the check does
        // not simply always fire.
        assert!(!v.contains("reports draw count"), "{v}");
        assert!(!v.contains("reports no-drop chance"), "{v}");
    }

    #[test]
    fn the_reward_caps_bound_what_one_pass_reads() {
        assert_eq!((MAX_DROP_ROWS, MAX_DROP_ENTRIES), (4_096, 256));
        // The vanilla row set fits inside the cap many times over: the cap is
        // there for a misread index, not for a big table.
        const { assert!(REWARD_ROWS_UNION * 4 < MAX_DROP_ROWS) };
        const { assert!(DROP_ENTRIES_HI < MAX_DROP_ENTRIES) };
        // And the requested set is truncated to it rather than growing without
        // bound when every mission names a different row.
        let mut s = Summary::new();
        for i in 0..(MAX_DROP_ROWS + 500) {
            s.operation(&rewarded(i, (i % 60_000) as u16, parsed::REWARD_NONE));
        }
        assert_eq!(s.reward_rows().len(), MAX_DROP_ROWS);
    }

    #[test]
    fn the_sample_cap_bounds_the_allocation_but_not_the_counts() {
        let mut s = Summary::new();
        for i in 0..(MAX_SAMPLES + 100) {
            s.operation(&op(1, i, 50, 2, &[]));
        }
        assert_eq!(s.ops, MAX_SAMPLES + 100);
        assert_eq!(s.durations.len(), MAX_SAMPLES);
        // The verdict is about the samples it kept and says which count it used.
        assert!(s.verdict().contains(&format!("{MAX_SAMPLES}/{MAX_SAMPLES}")), "{}", s.verdict());
    }

    // -----------------------------------------------------------------------
    // The reward amount pair, and the window between its two writes
    // -----------------------------------------------------------------------

    /// Replay the two writes in the order [`amount_write_order`] asks for and
    /// return every `(min, max)` a game thread could observe: the state before,
    /// the state between the two writes, and the state after.
    fn observable(cur: (i64, i64), want: (i64, i64)) -> Vec<(i64, i64)> {
        use parsed::dropset::ENTRY_AMOUNT_MIN;
        let mut seen = vec![cur];
        let mut now = cur;
        for (off, v) in amount_write_order(cur.1, want.0, want.1) {
            if off == ENTRY_AMOUNT_MIN {
                now.0 = v;
            } else {
                now.1 = v;
            }
            seen.push(now);
        }
        seen
    }

    /// The property the order exists for: `min <= max` at every moment, in all
    /// three directions the pair can move.
    #[test]
    fn an_amount_pair_is_never_observably_inverted() {
        let cases = [
            // Growing: Rewards 1 -> 3 on the 2..3 shape, 136 of 725 entries.
            ((2, 3), (6, 9)),
            // Shrinking: the same entry coming back down, Rewards 3 -> 1.
            ((6, 9), (2, 3)),
            // A revert of a fully applied pair.
            ((200, 200), (100, 100)),
            // Mixed, and reachable: a pass refused the maximum, leaving
            // (vanilla_min * 4, vanilla_max), and Rewards then moves to 2. The
            // minimum falls while the maximum rises.
            ((8, 3), (4, 6)),
            // Mixed the other way: the minimum rises while the maximum falls.
            ((2, 30), (6, 9)),
            // Nothing to do at all.
            ((5, 7), (5, 7)),
        ];
        for (cur, want) in cases {
            let seen = observable(cur, want);
            // Whatever the pass started from, it ends ordered.
            let last = seen.last().copied().unwrap_or(cur);
            assert!(last.0 <= last.1, "{cur:?} -> {want:?} ended {last:?}");

            if cur.0 > cur.1 {
                // The pair was ALREADY inverted when the pass found it, which
                // only a previously refused store can produce. Two stores
                // cannot repair that without one inverted intermediate:
                // whichever end is written first, the other still holds its
                // old, crossed value. So the guarantee here is the end state,
                // not the window - and the window is no worse than what the
                // pass was handed. Asserting otherwise would be asserting
                // something unachievable.
                continue;
            }
            // Started ordered, so nothing this function does may invert it.
            for (a, b) in seen {
                assert!(a <= b, "{cur:?} -> {want:?} showed {a}..{b}");
            }
        }
    }

    /// Growing raises the ceiling first, shrinking lowers the floor first, and
    /// the mixed case is decided by the field rather than by the lever.
    #[test]
    fn the_write_order_is_keyed_on_the_ceiling_not_the_lever() {
        use parsed::dropset::{ENTRY_AMOUNT_MAX, ENTRY_AMOUNT_MIN};
        // Grow: 2..3 x3.
        assert_eq!(
            amount_write_order(3, 6, 9),
            [(ENTRY_AMOUNT_MAX, 9), (ENTRY_AMOUNT_MIN, 6)]
        );
        // Shrink: 6..9 back to 2..3.
        assert_eq!(
            amount_write_order(9, 2, 3),
            [(ENTRY_AMOUNT_MIN, 2), (ENTRY_AMOUNT_MAX, 3)]
        );
        // Mixed under a *shrinking* lever: cur is 8..3 because the maximum was
        // refused last pass, and Rewards=2 wants 4..6. The ceiling is rising,
        // so the maximum still goes first even though the lever went down.
        assert_eq!(
            amount_write_order(3, 4, 6),
            [(ENTRY_AMOUNT_MAX, 6), (ENTRY_AMOUNT_MIN, 4)]
        );
        // Equal ceilings: either order is safe, and the minimum goes first.
        assert_eq!(
            amount_write_order(9, 4, 9),
            [(ENTRY_AMOUNT_MIN, 4), (ENTRY_AMOUNT_MAX, 9)]
        );
    }

    /// Both offsets, exactly once each, whichever way round they go: the pass
    /// writes a pair and never the same half twice.
    #[test]
    fn the_order_is_always_the_two_halves_once_each() {
        use parsed::dropset::{ENTRY_AMOUNT_MAX, ENTRY_AMOUNT_MIN};
        for cur_max in [-1i64, 0, 1, 3, 9, i64::MAX] {
            for (wmin, wmax) in [(1i64, 1i64), (4, 6), (0, 0), (9, 9)] {
                let o = amount_write_order(cur_max, wmin, wmax);
                let mut offs = [o[0].0, o[1].0];
                offs.sort_unstable();
                assert_eq!(offs, [ENTRY_AMOUNT_MIN, ENTRY_AMOUNT_MAX]);
                for (off, v) in o {
                    assert_eq!(v, if off == ENTRY_AMOUNT_MIN { wmin } else { wmax });
                }
            }
        }
    }

    /// BLOCKING 8's other half: `AnyOperatorCount=1` writes exactly `1` into
    /// every minimum operator count, which is precisely the reading the
    /// CONSTANT check fires on. The sentence has to name the setting, because
    /// that is the state a player who turned it on is in, not a bug.
    #[test]
    fn a_constant_operator_count_names_any_operator_count() {
        let mut s = Summary::new();
        for i in 0..936 {
            // Varied durations, every minimum operator count forced to 1.
            s.operation(&op(1, i, 20 + (i as u32 % 40) * 10, 1, &[]));
        }
        assert_eq!(s.constant_min_ops(), Some(1));
        assert_eq!(s.constant_duration(), None);
        let v = s.verdict();
        assert!(
            v.contains("CONSTANT: every one of 936 missions reports minimum operator count 1"),
            "{v}"
        );
        assert!(v.contains("AnyOperatorCount=1, which writes exactly 1 into this field"), "{v}");
        // Only the field that is flat is named.
        assert!(!v.contains("reports duration"), "{v}");
    }

    /// The census is a layout check over what is in the records *now*, and the
    /// caveat has to say so in words a log reader can act on: a high `Speed`
    /// puts every duration below [`DURATION_LO`] and the verdict then says
    /// LOOKS WRONG about a perfectly good offset.
    #[test]
    fn a_written_table_reads_as_a_wrong_layout_which_is_what_the_caveat_is_for() {
        let mut s = Summary::new();
        // 936 vanilla 2.0 h missions under Speed=20: 20 / 20 = 1, floored at
        // MIN_DURATION_TENTHS, which is two below DURATION_LO.
        for i in 0..936 {
            s.operation(&op(1, i, 1, 3, &[]));
        }
        let v = s.verdict();
        assert!(v.contains("LOOKS WRONG"), "{v}");
        let c = Summary::written_to_caveat();
        assert!(c.contains("already written"), "{c}");
        assert!(c.contains("not a vanilla census"), "{c}");
    }

    /// BLOCKING 3: the cap has to bite where the row is inserted. A row that
    /// got into the acted-on set stays in it, so a smaller row index arriving
    /// later cannot push an already-multiplied larger row out of the window
    /// where a revert would never reach it again.
    #[test]
    fn the_row_cap_bites_at_insert_and_never_evicts() {
        let mut set: BTreeSet<u16> = BTreeSet::new();
        // Fill to the cap with the largest row indices there are.
        for row in 0..MAX_DROP_ROWS {
            let row = u16::try_from((u16::MAX as usize - row).min(u16::MAX as usize)).unwrap();
            assert!(insert_capped(&mut set, row), "row {row} should have got in");
        }
        assert_eq!(set.len(), MAX_DROP_ROWS);
        let before = set.clone();
        // Now the smallest row index imaginable turns up. It is refused, and
        // nothing already in the set moves.
        assert!(!insert_capped(&mut set, 0));
        assert_eq!(set, before, "a refused insert must not evict anything");
        // A row already in the set is still reported as in it, cap or no cap.
        let known = *set.iter().next().unwrap();
        assert!(insert_capped(&mut set, known));
        assert_eq!(set.len(), MAX_DROP_ROWS);
    }

    /// Under the cap it is an ordinary set insert, and it is idempotent.
    #[test]
    fn the_row_cap_is_invisible_at_vanilla_size() {
        let mut set: BTreeSet<u16> = BTreeSet::new();
        for row in [6244u16, 14665, 14715, 6244] {
            assert!(insert_capped(&mut set, row));
        }
        assert_eq!(set.len(), 3, "a repeat is not a second row");
        assert!(set.contains(&6244) && set.contains(&14665) && set.contains(&14715));
    }

    /// BLOCKING 6: "the count moved" and "the row would not read" are two
    /// different findings. The band is the identity check, and it is narrower
    /// than the dump's resource cap on purpose.
    #[test]
    fn the_declared_entry_count_has_its_own_answer() {
        assert_eq!(drop_entry_count(1), Some(1));
        assert_eq!(drop_entry_count(60), Some(60), "the widest row of the live capture");
        assert_eq!(drop_entry_count(DROP_ENTRIES_HI as u32), Some(DROP_ENTRIES_HI));
        // Out of band: a row with no entries is not a reward row, and a row
        // with hundreds says `record+0x30` is not a count.
        assert_eq!(drop_entry_count(0), None);
        assert_eq!(drop_entry_count(DROP_ENTRIES_HI as u32 + 1), None);
        assert_eq!(drop_entry_count(u32::MAX), None);
        // The two bounds differ deliberately: the dump reads up to
        // MAX_DROP_ENTRIES and prints rows the write path refuses.
        const { assert!(DROP_ENTRIES_HI < MAX_DROP_ENTRIES) };
        assert_eq!(drop_entry_count(MAX_DROP_ENTRIES as u32), None);
    }

    /// The one line the two banking gates are worth. Nothing acts on them, so
    /// the sentence *is* the feature, and each reading has to say plainly what
    /// it means for the one effect that can outlive the plugin.
    #[test]
    fn the_banking_gate_line_says_what_each_byte_means() {
        use globals::{banking_gate_line, DEFERRED_REWARD_GATE_RVA, SURPLUS_WORKER_GATE_RVA};
        // The reading that would let the README's warning be deleted.
        let l = banking_gate_line(Some(0), Some(1));
        assert!(l.contains("0x6BC6AA8"), "{l}");
        assert!(l.contains("nothing is ever banked"), "{l}");
        assert!(l.contains("no mission defers its payout"), "{l}");
        // Deferral on, surplus term off: something is stored, but not because
        // of +0xC4.
        let l = banking_gate_line(Some(1), Some(0));
        assert!(l.contains("does not carry the surplus-worker term"), "{l}");
        // Both open: the warning stands exactly as written.
        let l = banking_gate_line(Some(1), Some(1));
        assert!(l.contains("both gates are open"), "{l}");
        assert!(l.contains("after the plugin is removed"), "{l}");
        // An unmapped page says so rather than claiming a reading.
        let l = banking_gate_line(None, Some(1));
        assert!(l.contains("would not read"), "{l}");
        assert!(l.contains("the warning in the README stands"), "{l}");
        // Every line names both addresses and the read-only promise.
        for l in [
            banking_gate_line(Some(0), Some(0)),
            banking_gate_line(None, None),
            banking_gate_line(Some(2), None),
        ] {
            assert!(l.contains(&format!("0x{DEFERRED_REWARD_GATE_RVA:X}")), "{l}");
            assert!(l.contains(&format!("0x{SURPLUS_WORKER_GATE_RVA:X}")), "{l}");
            assert!(l.contains("nothing here acts on either"), "{l}");
        }
    }

}
