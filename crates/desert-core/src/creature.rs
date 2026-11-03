//! Creatures the player catches by hand, and the one byte that says which
//! kind a creature is.
//!
//! Insects and fish are not gimmicks: they are characters taken with
//! `TrocTrPushCharacterToInventoryOnceTimer`
//! (`docs/reference-internals.md` section 15). What separates a catchable
//! insect from a catchable fish - and both from the birds and the unknown
//! species that pass every other test - is the **interaction category byte**
//! at `ClientStatusActorComponent+0x5A` (section 15.6).
//!
//! The two lists below are exactly the byte values seen on creatures that
//! were **actually caught by hand** on build 25116796, recorded with Desert
//! Looter's F7 event recorder and F11 survey (sections 15.4 and 15.5). They
//! are not a range the game defines and they are not derived from the exe, so
//! the only way to grow one is another recorded catch. A byte that is on
//! neither list classifies as nothing at all, and every caller here treats
//! that as "leave it alone" rather than guessing.
//!
//! The gate in front of that byte is the **actor type byte**
//! ([`CATCHABLE_TYPES`], `*(actor+0x88) -> byte @1`, sections 10.1 and 14),
//! which is here for the same reason: both plugins ask it first, and asking
//! it differently is how they would come to disagree. It is only ever a
//! gate: the type byte alone never decides anything, because the same types
//! carry creatures nobody can catch.
//!
//! This lives in desert-core because two plugins need the same answer for
//! different reasons: the looter decides whether to *target* a creature,
//! and Desert Gatherer decides how many of it to *grant* (its catch-count
//! hook, section 17). They must never disagree about what a fish is. What is
//! deliberately *not* here is how each plugin reaches the
//! `ClientStatusActorComponent` the category byte sits on: the looter
//! finds it by RTTI name because it is surveying arbitrary actors, and Desert
//! Gatherer uses the fixed `sub+0x20` slot because that is what the function
//! it is standing inside does. Both are right where they are.
//!
//! The catch-count **site** constants are here for the same reason
//! `gimmick::LOADER_PROLOGUE` is: they are what a game update breaks first,
//! and `crates/desert-core/tests/gimmick_real.rs` checks them against the real exe.

pub use crate::species_table::SpeciesClass;

// ---------------------------------------------------------------------------
// The catch-count patch site (build 26345091; section 17.4 has 25246367's)
// ---------------------------------------------------------------------------

/// The 21 bytes of section 17.4, unique in the mapped image:
///
/// ```text
/// +2dd6448  41 B8 01 00 00 00        mov  r8d,1              <- the count
/// +2dd644e  48 8D 95 D0 01 00 00     lea  rdx,[rbp+0x1d0]
/// +2dd6455  48 8D 8D B0 00 00 00     lea  rcx,[rbp+0xb0]
/// +2dd645c  E8 ...                   call +25fa160           ; (out, &item_index, count)
/// ```
///
/// The bytes are the contract and the RVAs (26345091's, file offset
/// `0x2DD5848`; `+0x2DD6778` on 26356046, `+0x2DD6728` on 26361232) are only illustration: the site moved 0x1740 between builds
/// 25116796 and 25246367, and again on every build since, without one of these
/// bytes changing, which is exactly why it is found by scanning for them.
///
/// The trailing `lea` and `E8` are not stolen; they are in the pattern
/// because they are what makes it unique and what proves the immediate really
/// is that call's third argument - the count of the item stack it builds.
pub const CATCH_SITE: &str = "41 B8 01 00 00 00 48 8D 95 D0 01 00 00 48 8D 8D B0 00 00 00 E8";

/// Bytes stolen at the site: `mov r8d,1` (6) + `lea rdx,[rbp+0x1d0]` (7).
/// Twelve is the minimum the `mov rax,imm64; jmp rax` patch needs; thirteen
/// is the next whole-instruction boundary past it, and the thirteenth patch
/// byte is a `nop`.
pub const CATCH_STOLEN: usize = 13;

/// How many of [`CATCH_BYTES`] the stub replaces rather than replays: the
/// `mov r8d,<imm32>` itself, whose value the callback supplies instead.
pub const CATCH_REPLACED: usize = 6;

/// The bytes those [`CATCH_STOLEN`] must be, checked before patching. A wrong
/// patch here is a crash to desktop; a refusal is a plugin that multiplies
/// gathers and leaves catches vanilla.
pub const CATCH_BYTES: [u8; CATCH_STOLEN] = [
    0x41, 0xB8, 0x01, 0x00, 0x00, 0x00, // mov r8d,1           - replaced
    0x48, 0x8D, 0x95, 0xD0, 0x01, 0x00, 0x00, // lea rdx,[rbp+0x1d0] - replayed
];

/// A creature the player can catch by hand, as told by its species record
/// ([`catch_class_by_species`]) or, failing that, its interaction category
/// byte and, for a fish, its type byte. There is deliberately no `Unknown`
/// variant: see [`catch_class`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchClass {
    Bug,
    Fish,
    /// A bag catch outside the insect and fish tribe groups: lizards, small
    /// birds, rats, frogs. Only a species record ever says this; no class
    /// byte maps to it.
    Critter,
}

impl CatchClass {
    /// The word the log lines use.
    pub fn name(self) -> &'static str {
        match self {
            CatchClass::Bug => "bug",
            CatchClass::Fish => "fish",
            CatchClass::Critter => "critter",
        }
    }
}

/// Interaction category bytes seen on insects caught by hand.
///
/// **The fallback since the species table**: [`catch_class_by_species`]
/// decides by the species record whenever the species is known, and these
/// class-byte lists (this one, [`FISH_CLASSES`], [`FISH_TYPES`] and
/// [`NPC_CLASSES`]) answer only for a species the table does not know, or
/// before the plugin has read it. The history below is why they say what they
/// say.
///
/// Six catches in the first verified field session (build 25116796,
/// 2026-09-08) all read 0x80, across four item ids (1001254, 1000680,
/// 1001238, 1001245).
///
/// The Rice Fish reads 0x80 too (`type=06 class=80 chr=3935`, item 1001241,
/// build 25477059; `chr=3966` on 26361232), and neither byte tells it from
/// the insects: its species row sits among theirs (bugs on 26361232 read chr
/// 3887, 3888, 3895, 3902 and 3916). [`catch_class`] alone therefore answers
/// [`CatchClass::Bug`] for it, which filed it under `GatherBugs` and `Bugs`.
/// What does tell it apart is its **species record**: `Animal_RiceFish` is
/// in the fish tribe group, so [`catch_class_by_species`] files it as a fish
/// from the species table (`crate::species_table`), and
/// [`catch_class_with_species`] keeps the old one-pair override for a caller
/// that only has a fish flag. Rows are not used as the key because they shift
/// between game builds (3935 -> 3966); names do not.
///
/// 0x80 is not only insects: row 6684 on 26361232, a class-0x80 species, is
/// `Animal_Raccoon_Wild_30042`, which is carried and skinned, never bagged.
/// The class byte alone would walk the auto-catch up to it; its species
/// record says `NotHandCatchable`.
pub const BUG_CLASSES: &[u8] = &[0x80];

/// Interaction category bytes seen on fish caught by hand.
///
/// The fallback for an unknown species since the species table; see
/// [`BUG_CLASSES`].
///
/// Fish are taken with the **same** event as insects
/// (`TrocTrPushCharacterToInventoryOnceTimer`, 8-byte payload). Four manual
/// catches at a lake (build 25116796, 2026-09-08), each surveyed with Desert
/// Looter's F11 immediately before and recorded with F7, read
/// `type=06 status=00/00` and:
///
/// ```text
/// eid=B0100550  cat=83   -> [recv] item 29817 x1
/// eid=B01005D8  cat=23   -> [recv] item 29805 x1
/// eid=B01005AB  cat=23   -> [recv] item 29804 x1
/// eid=B010068C  cat=83   -> [recv] item 29817 x1
/// ```
///
/// Seven more taken by the plugin itself in the same session all read 0x23.
///
/// **0x65** was added on build 25477059 (2026-09-24) by a Tench caught by
/// hand, the fish the plugin had just passed over as an unknown class:
///
/// ```text
/// [record] TrocTrPushCharacterToInventoryOnceTimer ... 00 08 FF 21 0A 10 B0 03
/// [harvest] [catch] class=65 not a known bug/fish class; vanilla
/// [recv] item 29809 x1
/// ```
///
/// **0x44** was added on build 26361232 (2026-11-01 19:33 UTC): the second
/// fish caught by hand on type 6 with that class, after one on 25477059
/// (2026-09-24). The live session, in the order the user caught them (a bug,
/// a skinning, then the fish), logged
///
/// ```text
/// [catch] type=06 class=44 not a known bug/fish class on this type; vanilla
/// ```
///
/// for the fish. It had been kept off because a type-6 bird **carcass** on
/// 25246367 carried 0x44 too, but a carcass never comes through here: the
/// looter's carcass gate takes every dead catchable-type actor first
/// (`desert_harvest::looter::actors`, dead status before any class), and the
/// catch hook only ever sees a creature being granted by hand. What the
/// addition can do is let the F9 auto-catch approach a **live** type-6
/// class-0x44 bird, if one exists; the catch event then fails the way it
/// does on any creature the game will not hand over, and nothing is granted
/// or multiplied.
///
/// A class on this list is a fish **only on [`FISH_TYPES`]**: 0x23 is also
/// carried by a deer. See [`catch_class`].
pub const FISH_CLASSES: &[u8] = &[0x23, 0x44, 0x65, 0x83];

/// The [`type_byte`] values on which a [`FISH_CLASSES`] byte means a fish.
///
/// The fallback for an unknown species since the species table; see
/// [`BUG_CLASSES`].
///
/// The class byte is not a species, and 0x23 proves it. On build 25477059
/// (2026-09-24, `docs/reference-internals.md` section 15.8) one session read
/// both halves, and an earlier one that day skinned the carcass:
///
/// ```text
/// [catch] debug: type=06 class=23 chr=4000        -> [recv] item 29805 x1
/// Catchable eid=B01012B9 (-867.2 609.8 -936.8) type=03 cat=23 chr=3458 status=00/00
/// [skin] carcass cat=23 eid=B0100AA4 gave: Animal_Bone x3; Meat_II x5; Deer_Leather x2
/// ```
///
/// a fish on type 6 and a land animal on type 3, two different species rows,
/// and a class-0x23 carcass that paid deer loot. Build 25246367 had already
/// shown the land half (`B01003D9`, `cat=23 type=03`,
/// `docs/findings/2026-09-15-skinning.md` section 7), and every fish ever
/// read, on every build, has been type 6. So a fish class on any other type is
/// not taken for a fish.
///
/// This separates 0x23 and nothing more general: land animals come on type
/// 6 too (Kiwi, class 0x75; class 0x5C). 0x44 - a type-6 fish caught by hand
/// on 25477059 and again on 26361232, a type-6 bird carcass on 25246367 - is
/// on [`FISH_CLASSES`] since the second fish; see there for why the carcass
/// does not argue against it.
/// Insects are not held to this: the Firefly Colony is caught by hand on
/// type 3.
pub const FISH_TYPES: &[u8] = &[6];

/// Interaction category bytes seen on **NPCs** - people, not animals.
///
/// Kept beside the catch lists, which are the fallback for an unknown
/// species since the species table ([`BUG_CLASSES`]); the carcass gate still
/// reads this one directly.
///
/// Recorded by the surveys behind [`CATCHABLE_TYPES`]: 0x21, 0x33, 0x66 and
/// 0x71, every one of them on the type byte 3 that also carries the Firefly
/// Colony. None is a catch class.
///
/// This list exists for the looter's carcass gate, which needs the one thing
/// the catch path never has to ask: whether a **dead** type-3 actor is an
/// animal or a person. The type byte cannot answer it - see
/// [`CATCHABLE_TYPES`], "the type byte alone never decides anything" - and a
/// whitelist of animal classes cannot either, because the carcasses actually
/// skinned in game read 0xD9, 0x75, 0x44 and 0xD7, three of which no recorded
/// catch has shown (0x44 only ever on fish, alive, on type 6), so a whitelist
/// of catch classes would refuse the very species the feature
/// works on. A blacklist of the classes known to be people is what is left. It
/// is not airtight: a dead NPC whose class is not one of these four is still
/// taken for an animal, and the honest fix for that is another survey.
pub const NPC_CLASSES: &[u8] = &[0x21, 0x33, 0x66, 0x71];

/// Whether an interaction category byte is one [`NPC_CLASSES`] names.
pub fn is_npc_class(cat: u8) -> bool {
    NPC_CLASSES.contains(&cat)
}

/// Classify a creature by its type byte and interaction category byte.
///
/// `None` means "not a class we have ever watched being caught on this type".
/// Every caller must treat that as vanilla behaviour: the looter skips the
/// actor, Desert Gatherer returns a multiplier of 1. Guessing here would mean
/// grabbing, or scaling, whatever else the game happens to route through the
/// same code - birds in flight read 0x20 and pass every other test, and a
/// live deer reads the fish class 0x23 ([`FISH_TYPES`]).
///
/// The type byte is asked here and not only by the callers' own
/// [`CATCHABLE_TYPES`] gate because that gate admits type 3, and type 3 is
/// where the deer is. It does not replace that gate: a bug class is answered
/// on any type, as before.
pub fn catch_class(ty: u8, category: u8) -> Option<CatchClass> {
    if BUG_CLASSES.contains(&category) {
        Some(CatchClass::Bug)
    } else if FISH_CLASSES.contains(&category) && FISH_TYPES.contains(&ty) {
        Some(CatchClass::Fish)
    } else {
        None
    }
}

/// Whether a species override may apply to this type and class: the class
/// alone says [`CatchClass::Bug`] (one of [`BUG_CLASSES`]) and the type is one
/// of [`FISH_TYPES`]. This is the **only** pair [`catch_class_with_species`]
/// ever changes, and callers use it to skip reading the species row at all
/// for every other creature.
pub fn species_can_override(ty: u8, category: u8) -> bool {
    BUG_CLASSES.contains(&category) && FISH_TYPES.contains(&ty)
}

/// [`catch_class`], corrected by a fish flag: `fish_species` says the
/// creature's species is a fish.
///
/// Only one answer can change: a bug class on a fish type becomes
/// [`CatchClass::Fish`] when the species is a fish (the Rice Fish, see
/// [`BUG_CLASSES`]). The flag never turns a class [`catch_class`] does not
/// know into a catch - `None` stays `None` whatever the species - and it never
/// touches a fish class, a bug on a non-fish type (the Firefly Colony on type
/// 3), or anything with `fish_species == false`, which is exactly
/// [`catch_class`]. [`catch_class_by_species`] is what the plugin calls; this
/// is its fallback's shape, kept for a caller that has only the flag.
pub fn catch_class_with_species(ty: u8, category: u8, fish_species: bool) -> Option<CatchClass> {
    if fish_species && species_can_override(ty, category) {
        return Some(CatchClass::Fish);
    }
    catch_class(ty, category)
}

/// Classify a creature by its species record first, its bytes second.
///
/// `species` is the creature's [`SpeciesClass`] from the generated species
/// table (`crate::species_table::species_class` on its `characterinfo` record
/// name). A known species decides on its own, whatever the class byte says:
///
/// - [`SpeciesClass::NotHandCatchable`] is `None` - ducks and raccoons are
///   carried, killed and skinned, never caught, and the raccoon reads the
///   insects' class 0x80 (row 6684 on 26361232);
/// - [`SpeciesClass::Bug`], [`SpeciesClass::Fish`] and
///   [`SpeciesClass::Critter`] are that class - the record lists the
///   `Insect_Catch` interaction and names the item it turns into, which is
///   what a bag catch is. That files `Animal_Insect_Lobster` and the Rice Fish
///   as fish, which no class byte could;
/// - [`SpeciesClass::Body`] and [`SpeciesClass::FriendlyBody`] are `None` -
///   a person or monster whose corpse is searched (`Dead_Loot`) is never a
///   catch;
/// - [`SpeciesClass::Protected`] is `None` - the player's own characters and
///   their doubles are never caught, searched or skinned.
///
/// `None` - a species the table does not know, or a table the plugin has not
/// read yet - is exactly the old answer: [`catch_class`], the class-byte
/// lists, with no fish flag (the one creature the flag was for, the Rice
/// Fish, is in the table).
///
/// The callers' [`CATCHABLE_TYPES`] gate stays in front of this either way.
pub fn catch_class_by_species(ty: u8, category: u8, species: Option<SpeciesClass>) -> Option<CatchClass> {
    match species {
        Some(SpeciesClass::Bug) => Some(CatchClass::Bug),
        Some(SpeciesClass::Fish) => Some(CatchClass::Fish),
        Some(SpeciesClass::Critter) => Some(CatchClass::Critter),
        Some(
            SpeciesClass::NotHandCatchable
            | SpeciesClass::Body
            | SpeciesClass::FriendlyBody
            | SpeciesClass::Protected,
        ) => None,
        None => catch_class_with_species(ty, category, false),
    }
}

/// A known species that is never a hand catch, whatever its class byte:
/// [`SpeciesClass::NotHandCatchable`] (ducks, raccoons), both body classes
/// (people and monsters, searched when dead, never bagged alive) and
/// [`SpeciesClass::Protected`] (the player's own characters).
pub fn is_never_a_catch(species: Option<SpeciesClass>) -> bool {
    matches!(
        species,
        Some(
            SpeciesClass::NotHandCatchable
                | SpeciesClass::Body
                | SpeciesClass::FriendlyBody
                | SpeciesClass::Protected
        )
    )
}

/// The species row is a `u16` (`ClientStatusActorComponent+0x30`), so a
/// bitset of this many bits covers every row the game can name.
pub const SPECIES_ROWS: usize = 1 << 16;
const SPECIES_WORDS: usize = SPECIES_ROWS / 64;

/// One bit per `characterinfo` row, lock-free: the plugin thread sets bits,
/// a game-thread hook reads them. Bits are only ever set, never cleared, for
/// the life of the process. 8 KiB, const-constructible, so it lives in a
/// `static`.
pub struct SpeciesBits {
    words: [core::sync::atomic::AtomicU64; SPECIES_WORDS],
}

impl SpeciesBits {
    pub const fn new() -> Self {
        Self { words: [const { core::sync::atomic::AtomicU64::new(0) }; SPECIES_WORDS] }
    }

    /// Set `row`'s bit; true when it was not set before.
    pub fn set(&self, row: u16) -> bool {
        let Some(w) = self.words.get(usize::from(row >> 6)) else { return false };
        let bit = 1u64 << (row & 63);
        w.fetch_or(bit, core::sync::atomic::Ordering::Relaxed) & bit == 0
    }

    /// Is `row`'s bit set? One atomic load; safe on any thread.
    pub fn contains(&self, row: u16) -> bool {
        self.words
            .get(usize::from(row >> 6))
            .is_some_and(|w| w.load(core::sync::atomic::Ordering::Relaxed) & (1u64 << (row & 63)) != 0)
    }

    /// How many bits are set.
    pub fn count(&self) -> usize {
        self.words.iter().map(|w| w.load(core::sync::atomic::Ordering::Relaxed).count_ones() as usize).sum()
    }
}

impl Default for SpeciesBits {
    fn default() -> Self {
        Self::new()
    }
}

/// One [`SpeciesClass`] per `characterinfo` row, lock-free: the plugin
/// thread stores each row's class once it has read the row's name, a
/// game-thread hook loads it. One byte per row, 0 for "not known" (not read
/// yet, or a name the species table does not hold). 64 KiB,
/// const-constructible, so it lives in a `static`.
pub struct SpeciesClasses {
    rows: [core::sync::atomic::AtomicU8; SPECIES_ROWS],
}

impl SpeciesClasses {
    pub const fn new() -> Self {
        Self { rows: [const { core::sync::atomic::AtomicU8::new(0) }; SPECIES_ROWS] }
    }

    fn encode(class: SpeciesClass) -> u8 {
        match class {
            SpeciesClass::Bug => 1,
            SpeciesClass::Fish => 2,
            SpeciesClass::Critter => 3,
            SpeciesClass::NotHandCatchable => 4,
            SpeciesClass::Body => 5,
            SpeciesClass::FriendlyBody => 6,
            SpeciesClass::Protected => 7,
        }
    }

    fn decode(code: u8) -> Option<SpeciesClass> {
        match code {
            1 => Some(SpeciesClass::Bug),
            2 => Some(SpeciesClass::Fish),
            3 => Some(SpeciesClass::Critter),
            4 => Some(SpeciesClass::NotHandCatchable),
            5 => Some(SpeciesClass::Body),
            6 => Some(SpeciesClass::FriendlyBody),
            7 => Some(SpeciesClass::Protected),
            _ => None,
        }
    }

    /// Store `row`'s class; true when that changed what the row held.
    pub fn set(&self, row: u16, class: SpeciesClass) -> bool {
        let Some(slot) = self.rows.get(usize::from(row)) else { return false };
        let code = Self::encode(class);
        slot.swap(code, core::sync::atomic::Ordering::Relaxed) != code
    }

    /// `row`'s class, or `None` when it is not known. One atomic load; safe
    /// on any thread.
    pub fn get(&self, row: u16) -> Option<SpeciesClass> {
        self.rows
            .get(usize::from(row))
            .and_then(|slot| Self::decode(slot.load(core::sync::atomic::Ordering::Relaxed)))
    }

    /// How many rows hold each class: `[bug, fish, critter, not catchable,
    /// body, friendly body, protected]`.
    pub fn counts(&self) -> [usize; 7] {
        let mut out = [0usize; 7];
        for slot in &self.rows {
            let code = slot.load(core::sync::atomic::Ordering::Relaxed);
            if let Some(n) = usize::from(code).checked_sub(1).and_then(|i| out.get_mut(i)) {
                *n += 1;
            }
        }
        out
    }
}

impl Default for SpeciesClasses {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a category byte is on either class list, whatever the type. For
/// diagnostics that ask "is this a class we know?" about an actor whose type
/// is already known to be the odd one out.
pub fn is_catch_class(category: u8) -> bool {
    BUG_CLASSES.contains(&category) || FISH_CLASSES.contains(&category)
}

// ---------------------------------------------------------------------------
// The actor type byte (sections 10.1, 14 and 15)
// ---------------------------------------------------------------------------

/// The actor's type byte: `*(actor+0x88) -> byte @1`.
///
/// The reference mod reads it as its per-actor "flag byte +0x2E"
/// (`FUN_18000e560`, `docs/reference-internals.md` section 10.1) and treats 6
/// as "catchable". The game's own steal check (`FUN_14251BA50`, section 14)
/// switches on the same byte: 4/5/6 are characters, 7 is a gimmick.
pub const TYPE_OBJECT_OFF: usize = 0x88;
/// Byte index into the object at [`TYPE_OBJECT_OFF`].
pub const TYPE_BYTE_OFF: usize = 1;

/// The [`type_byte`] values every creature caught by hand has had.
///
/// **Type 6** was recorded live on build 25116796 (2026-09-08) by surveying
/// the world and then catching the very actors the survey had just listed,
/// with the looter's F7 event recorder running. All three insects read the
/// same way:
///
/// ```text
/// Character eid=B01002C3 ClientNormalInGameActor type=06 status=00/00
///   comps=[Status EquipSlot CharacterControl Vehicle Detect Ai Effect
///          FrameEvent Catch RemoteCatch Attack]
/// ```
///
/// and each catch queued one `TrocTrPushCharacterToInventoryOnceTimer`
/// naming that eid; the fish of section 15.5 read `type=06` too.
///
/// **Type 3** was added the same day by the **Firefly Colony**, which is
/// caught by hand with that very same event (recorded payload
/// `00 08 FF EA 08 10 B0 03`) while surveying as:
///
/// ```text
/// Character eid=B01008EA ClientNormalInGameActor type=03 cat=80 status=00/00
///   comps=[Status EquipSlot CharacterControl Vehicle Detect Ai Effect
///          FrameEvent Catch RemoteCatch Attack]
/// ```
///
/// That is the insect class byte 0x80 on a type byte every earlier survey had
/// only ever seen on NPCs (classes 0x21, 0x33, 0x66 and 0x71, none of them a
/// known catch class).
///
/// So **the type byte alone never decides anything**. It is a necessary
/// condition and no more: type 3 holds NPCs *and* the Firefly Colony, type 6
/// holds the insects and fish *and* the birds in flight of section 15.5
/// (class 0x20). The class byte is what decides, through [`catch_class`], and
/// a class no recorded catch has shown is never targeted and never multiplied
/// whatever the type says.
///
/// Type 5 is deliberately **not** on the list: type-05 characters have been
/// surveyed at distance carrying 0x80, 0x8C and 0x90, and not one of them has
/// ever been caught. Growing this list, like growing the class lists, takes a
/// recorded catch.
pub const CATCHABLE_TYPES: &[u8] = &[3, 6];

/// `*(actor+0x88) -> byte @1`, the actor type byte, or `None` if either read
/// falls on an unmapped page.
#[cfg(windows)]
pub fn type_byte(actor: usize) -> Option<u8> {
    let p = crate::safe::read_ptr(actor + TYPE_OBJECT_OFF)?;
    crate::safe::read(p + TYPE_BYTE_OFF)
}

/// Is this actor shaped like a creature the player can catch by hand?
///
/// A gate, never an answer: see [`CATCHABLE_TYPES`]. An unreadable actor
/// answers `false` - both callers want "leave it alone" when they cannot
/// tell.
#[cfg(windows)]
pub fn is_catchable_type(actor: usize) -> bool {
    type_byte(actor).is_some_and(|t| CATCHABLE_TYPES.contains(&t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recorded_classes_map_and_nothing_else_does() {
        assert_eq!(catch_class(6, 0x80), Some(CatchClass::Bug));
        assert_eq!(catch_class(6, 0x23), Some(CatchClass::Fish));
        assert_eq!(catch_class(6, 0x65), Some(CatchClass::Fish));
        assert_eq!(catch_class(6, 0x83), Some(CatchClass::Fish));
        // Two hand-caught type-6 fish, on 25477059 and 26361232.
        assert_eq!(catch_class(6, 0x44), Some(CatchClass::Fish));
        // 0x20 is the birds-in-flight class of section 15.5, and the whole
        // reason `None` is not a third `CatchClass` variant: it must never
        // reach a code path that acts on it.
        for other in [0x00, 0x20, 0x2C, 0x57, 0x8C, 0x90, 0xFF] {
            for ty in [3u8, 6] {
                assert_eq!(catch_class(ty, other), None, "type {ty} 0x{other:02X}");
            }
            assert!(!is_catch_class(other), "0x{other:02X}");
        }
    }

    /// 0x23 is a fish in the water and a deer on land. The fish read
    /// `type=06` (section 15.5, build 25116796, and five plugin catches on
    /// 25477059 giving 29805/29804); the land animal `B01003D9` read
    /// `cat=23 type=03` (skinning findings section 7, 25246367), and on
    /// 25477059 a class-0x23 carcass gave Animal_Bone, Meat_II and
    /// Deer_Leather. A live one of those must never be sent a catch event.
    #[test]
    fn a_fish_class_is_a_fish_only_on_a_fish_type() {
        for &c in FISH_CLASSES {
            assert_eq!(catch_class(6, c), Some(CatchClass::Fish), "0x{c:02X}");
            assert!(is_catch_class(c), "0x{c:02X}");
            for &ty in CATCHABLE_TYPES.iter().filter(|t| !FISH_TYPES.contains(t)) {
                assert_eq!(catch_class(ty, c), None, "type {ty} 0x{c:02X}");
            }
        }
        // The deer, by name.
        assert_eq!(catch_class(3, 0x23), None);
        // Every fish type is a catchable type, or the gate in front would
        // keep the fish away before this rule is ever asked.
        for t in FISH_TYPES {
            assert!(CATCHABLE_TYPES.contains(t), "fish type {t} is not catchable");
        }
    }

    /// The type rule is for fish only. The Firefly Colony is the insect class
    /// 0x80 on type 3 and is caught by hand with the ordinary catch event.
    #[test]
    fn a_bug_class_is_a_bug_on_either_catchable_type() {
        for &ty in CATCHABLE_TYPES {
            assert_eq!(catch_class(ty, 0x80), Some(CatchClass::Bug), "type {ty}");
        }
    }

    #[test]
    fn the_recorded_types_are_gates_and_nothing_else_is() {
        // 6 is the insects and fish; 3 is the NPC type the Firefly Colony
        // turned out to share. 5 carries 0x80/0x8C/0x90 at distance and has
        // never been caught, and 7 is the gimmick type of section 14.
        for t in [3u8, 6] {
            assert!(CATCHABLE_TYPES.contains(&t), "type {t} must be catchable");
        }
        for t in [0u8, 1, 2, 4, 5, 7, 0xFF] {
            assert!(!CATCHABLE_TYPES.contains(&t), "type {t} must not be catchable");
        }
    }

    /// The NPC classes are what keeps the looter's carcass gate off people, so
    /// none of them may be a catch class - if one were, a species the mod
    /// catches would also be one it refused to skin, and the two rules would be
    /// answering the same byte differently.
    #[test]
    fn the_npc_classes_are_not_catch_classes() {
        for &c in NPC_CLASSES {
            assert!(is_npc_class(c), "0x{c:02X}");
            assert!(!BUG_CLASSES.contains(&c), "0x{c:02X} is both an NPC and a bug class");
            assert!(!FISH_CLASSES.contains(&c), "0x{c:02X} is both an NPC and a fish class");
        }
        // The classes the carcasses skinned in game on 2026-09-15 carried. If
        // one of these ever lands on the NPC list, skinning silently stops
        // working for that species, so the overlap is worth a test.
        for c in [0xD9u8, 0x75, 0x44, 0xD7] {
            assert!(!is_npc_class(c), "0x{c:02X} is a skinned carcass class, not an NPC class");
        }
    }

    #[test]
    fn no_byte_is_on_both_lists() {
        for b in BUG_CLASSES {
            assert!(!FISH_CLASSES.contains(b), "0x{b:02X} is both a bug and a fish");
        }
    }

    #[test]
    fn the_stolen_bytes_are_a_literal_prefix_of_the_signature() {
        // The byte check before the patch reads CATCH_STOLEN bytes and
        // compares them with CATCH_BYTES; if those were not the head of the
        // signature the scan matched, it would be checking something else.
        let head: Vec<&str> = CATCH_SITE.split_whitespace().take(CATCH_STOLEN).collect();
        let want: Vec<String> = CATCH_BYTES.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(head, want);
        assert_eq!(CATCH_SITE.split_whitespace().count(), 21);
        // mov r8d,imm32, immediate 1 - the vanilla count.
        assert_eq!(&CATCH_BYTES[..CATCH_REPLACED], &[0x41, 0xB8, 0x01, 0x00, 0x00, 0x00]);
        // lea rdx,[rbp+0x1d0] - replayed by the stub, byte for byte.
        assert_eq!(&CATCH_BYTES[CATCH_REPLACED..], &[0x48, 0x8D, 0x95, 0xD0, 0x01, 0x00, 0x00]);
        // A patch of fewer than MIN_STOLEN bytes does not fit `mov
        // rax,imm64; jmp rax`, and `hook::install_raw` refuses either way.
        const {
            assert!(CATCH_STOLEN >= crate::trampoline::MIN_STOLEN);
            assert!(CATCH_STOLEN <= crate::trampoline::MAX_STOLEN);
        }
    }

    /// The species decides whenever it is known, and the class byte is
    /// only the fallback. Names and classes are the real 26361232 ones.
    #[test]
    fn a_known_species_decides_on_its_own() {
        use crate::species_table::species_class;
        let by = |ty, cat, name: &str| catch_class_by_species(ty, cat, species_class(name));
        // Row 6684: the raccoon reads the insects' class 0x80, on either
        // catchable type. It is carried and skinned, never caught.
        for ty in [6u8, 3] {
            assert_eq!(catch_class(ty, 0x80), Some(CatchClass::Bug), "the bytes alone would catch it");
            assert_eq!(by(ty, 0x80, "Animal_Raccoon_Wild_30042"), None, "type {ty}");
        }
        assert_eq!(by(6, 0x80, "Animal_Duck_Wild_30020"), None);
        assert_eq!(by(6, 0x23, "Animal_Duck_Wild_30020"), None);
        // A critter whatever its class byte: no class byte maps to Critter.
        for cat in [0x00u8, 0x20, 0x80] {
            assert_eq!(by(6, cat, "Animal_Iguana_Wild_30069"), Some(CatchClass::Critter), "0x{cat:02X}");
            assert_eq!(by(6, cat, "Animal_Sparrow_Wild_30071"), Some(CatchClass::Critter), "0x{cat:02X}");
        }
        // Fish-group species the old name marker could not see, and the Rice
        // Fish on the insects' class.
        assert_eq!(by(6, 0x80, "Animal_Insect_Lobster"), Some(CatchClass::Fish));
        assert_eq!(by(6, 0x80, "Animal_Insect_Sea_Slater"), Some(CatchClass::Fish));
        assert_eq!(by(6, 0x80, "Animal_RiceFish"), Some(CatchClass::Fish));
        assert_eq!(by(6, 0x80, "Animal_Insect_GrassHopper"), Some(CatchClass::Bug));
        assert_eq!(by(3, 0x23, "Animal_Insect_GrassHopper"), Some(CatchClass::Bug));
        // A body is never a catch, whatever its bytes say: a dead bandit on
        // the insects' class, or on the fish class of a fish type.
        for (ty, cat) in [(6u8, 0x80u8), (3, 0x80), (6, 0x23), (3, 0x5A)] {
            assert_eq!(by(ty, cat, "NHM_Bleed_Bandits_OneHandAxe_54140"), None, "type {ty} 0x{cat:02X}");
            assert_eq!(by(ty, cat, "NHM_Citizen_3053"), None, "type {ty} 0x{cat:02X}");
            assert_eq!(by(ty, cat, "NHM_Wandering_Mercenary_OneHandMusket_03"), None, "type {ty} 0x{cat:02X}");
        }
    }

    /// An unknown species (`None`) is exactly the class-byte answer, for
    /// every type and class.
    #[test]
    fn an_unknown_species_falls_back_to_the_class_bytes() {
        assert_eq!(crate::species_table::species_class("Animal_Not_In_The_Table_1"), None);
        for ty in 0u8..=255 {
            for cat in 0u8..=255 {
                assert_eq!(catch_class_by_species(ty, cat, None), catch_class(ty, cat), "type {ty} 0x{cat:02X}");
            }
        }
        assert_eq!(catch_class_by_species(6, 0x80, None), Some(CatchClass::Bug));
        assert_eq!(catch_class_by_species(6, 0x23, None), Some(CatchClass::Fish));
        assert_eq!(catch_class_by_species(3, 0x23, None), None);
        assert_eq!(catch_class_by_species(6, 0x20, None), None);
    }

    #[test]
    fn never_a_catch_is_the_four_non_catch_classes() {
        assert!(!is_never_a_catch(None));
        assert!(!is_never_a_catch(Some(SpeciesClass::Bug)));
        assert!(!is_never_a_catch(Some(SpeciesClass::Fish)));
        assert!(!is_never_a_catch(Some(SpeciesClass::Critter)));
        assert!(is_never_a_catch(Some(SpeciesClass::NotHandCatchable)));
        assert!(is_never_a_catch(Some(SpeciesClass::Body)));
        assert!(is_never_a_catch(Some(SpeciesClass::FriendlyBody)));
        assert!(is_never_a_catch(Some(SpeciesClass::Protected)));
    }

    /// The player's own characters are never a catch, whatever their bytes.
    #[test]
    fn a_protected_species_is_never_a_catch() {
        use crate::species_table::species_class;
        for name in ["Kliff", "Damian", "Oongka", "Nairah"] {
            assert_eq!(species_class(name), Some(SpeciesClass::Protected), "{name}");
            for (ty, cat) in [(6u8, 0x80u8), (3, 0x80), (6, 0x23), (3, 0x5A)] {
                assert_eq!(catch_class_by_species(ty, cat, species_class(name)), None, "{name} {ty} 0x{cat:02X}");
            }
        }
    }

    #[test]
    fn species_classes_store_and_count_every_row() {
        let s = SpeciesClasses::new();
        assert_eq!(s.counts(), [0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(s.get(0), None);
        assert!(s.set(0, SpeciesClass::Bug));
        assert!(!s.set(0, SpeciesClass::Bug), "the same class again is no change");
        assert!(s.set(3966, SpeciesClass::Fish));
        assert!(s.set(6684, SpeciesClass::NotHandCatchable));
        assert!(s.set(u16::MAX, SpeciesClass::Critter));
        assert!(s.set(7, SpeciesClass::Body));
        assert_eq!(s.get(7), Some(SpeciesClass::Body));
        assert!(s.set(8, SpeciesClass::FriendlyBody));
        assert_eq!(s.get(8), Some(SpeciesClass::FriendlyBody));
        assert!(s.set(9, SpeciesClass::Protected));
        assert_eq!(s.get(9), Some(SpeciesClass::Protected));
        assert_eq!(s.get(0), Some(SpeciesClass::Bug));
        assert_eq!(s.get(3966), Some(SpeciesClass::Fish));
        assert_eq!(s.get(6684), Some(SpeciesClass::NotHandCatchable));
        assert_eq!(s.get(u16::MAX), Some(SpeciesClass::Critter));
        assert_eq!(s.get(1), None);
        assert_eq!(s.counts(), [1, 1, 1, 1, 1, 1, 1]);
        assert!(s.set(0, SpeciesClass::Fish), "a changed class is a change");
        assert_eq!(s.counts(), [0, 2, 1, 1, 1, 1, 1]);
    }

    /// The Rice Fish on the flag-only path: class 0x80 on type 6, flagged a
    /// fish, is a fish.
    #[test]
    fn a_fish_species_turns_a_type_6_bug_into_a_fish() {
        assert_eq!(catch_class_with_species(6, 0x80, true), Some(CatchClass::Fish));
        assert_eq!(catch_class_with_species(6, 0x80, false), Some(CatchClass::Bug));
        assert!(species_can_override(6, 0x80));
    }

    /// Only class 0x80 on a fish type is ever overridden: the Firefly Colony
    /// (0x80 on type 3) stays a bug whatever its species flag says, and a
    /// fish class stays a fish.
    #[test]
    fn only_a_bug_class_on_a_fish_type_is_overridden() {
        for ty in 0u8..=255 {
            for cat in 0u8..=255 {
                let plain = catch_class(ty, cat);
                assert_eq!(catch_class_with_species(ty, cat, false), plain, "type {ty} 0x{cat:02X}");
                let with = catch_class_with_species(ty, cat, true);
                if species_can_override(ty, cat) {
                    assert_eq!(plain, Some(CatchClass::Bug));
                    assert_eq!(with, Some(CatchClass::Fish));
                } else {
                    assert_eq!(with, plain, "type {ty} 0x{cat:02X}");
                }
            }
        }
        assert_eq!(catch_class_with_species(3, 0x80, true), Some(CatchClass::Bug));
        assert!(!species_can_override(3, 0x80));
        assert_eq!(catch_class_with_species(6, 0x23, true), Some(CatchClass::Fish));
    }

    /// The species flag can never make a catch out of a class nobody has
    /// caught: the birds in flight (0x20), the deer (0x23 on type 3), an
    /// NPC class.
    #[test]
    fn a_species_flag_never_creates_a_catch_from_an_unknown_class() {
        for (ty, cat) in [(6u8, 0x20u8), (3, 0x23), (6, 0x2C), (3, 0x33), (5, 0x8C), (6, 0x00)] {
            assert_eq!(catch_class(ty, cat), None);
            assert_eq!(catch_class_with_species(ty, cat, true), None, "type {ty} 0x{cat:02X}");
        }
    }

    #[test]
    fn species_bits_set_and_read_every_row() {
        let b = SpeciesBits::new();
        assert_eq!(b.count(), 0);
        for row in [0u16, 63, 64, 3966, 4031, u16::MAX] {
            assert!(!b.contains(row));
            assert!(b.set(row), "{row}");
            assert!(!b.set(row), "second set of {row}");
            assert!(b.contains(row));
        }
        assert_eq!(b.count(), 6);
        assert!(!b.contains(3965) && !b.contains(3967) && !b.contains(1));
    }

    #[test]
    fn names_are_the_log_vocabulary() {
        assert_eq!(CatchClass::Bug.name(), "bug");
        assert_eq!(CatchClass::Fish.name(), "fish");
        assert_eq!(CatchClass::Critter.name(), "critter");
    }
}
