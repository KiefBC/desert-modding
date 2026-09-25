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
//! different reasons: Desert Looter decides whether to *target* a creature,
//! and Desert Gatherer decides how many of it to *grant* (its catch-count
//! hook, section 17). They must never disagree about what a fish is. What is
//! deliberately *not* here is how each plugin reaches the
//! `ClientStatusActorComponent` the category byte sits on: Desert Looter
//! finds it by RTTI name because it is surveying arbitrary actors, and Desert
//! Gatherer uses the fixed `sub+0x20` slot because that is what the function
//! it is standing inside does. Both are right where they are.
//!
//! The catch-count **site** constants are here for the same reason
//! `gimmick::LOADER_PROLOGUE` is: they are what a game update breaks first,
//! and `desert-core/tests/gimmick_real.rs` checks them against the real exe.

// ---------------------------------------------------------------------------
// The catch-count patch site (build 25246367, section 17.4)
// ---------------------------------------------------------------------------

/// The 21 bytes of section 17.4, unique in the mapped image:
///
/// ```text
/// 0x142a75891  41 B8 01 00 00 00        mov  r8d,1              <- the count
/// 0x142a75897  48 8D 95 D0 01 00 00     lea  rdx,[rbp+0x1d0]
/// 0x142a7589E  48 8D 8D B0 00 00 00     lea  rcx,[rbp+0xb0]
/// 0x142a758A5  E8 ...                   call FUN_1423507b0      ; (out, &item_index, count)
/// ```
///
/// The bytes are the contract and the addresses are only illustration: the
/// site moved 0x1740 between builds 25116796 and 25246367 without one of these
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

/// A creature the player can catch by hand, as told by its interaction
/// category byte and, for a fish, its type byte. There is deliberately no
/// `Unknown` variant: see [`catch_class`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatchClass {
    Bug,
    Fish,
}

impl CatchClass {
    /// The word the log lines use.
    pub fn name(self) -> &'static str {
        match self {
            CatchClass::Bug => "bug",
            CatchClass::Fish => "fish",
        }
    }
}

/// Interaction category bytes seen on insects caught by hand.
///
/// Six catches in the first verified field session (build 25116796,
/// 2026-09-08) all read 0x80, across four item ids (1001254, 1000680,
/// 1001238, 1001245).
///
/// The Rice Fish reads 0x80 too (`type=06 class=80 chr=3935`, item 1001241,
/// build 25477059), so it is caught - and multiplied - as a bug, under
/// `GatherBugs` and `Bugs`. Neither byte tells it from the insects; its
/// species row sits among theirs.
pub const BUG_CLASSES: &[u8] = &[0x80];

/// Interaction category bytes seen on fish caught by hand.
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
/// [gatherer] [catch] class=65 not a known bug/fish class; vanilla
/// [recv] item 29809 x1
/// ```
///
/// A class on this list is a fish **only on [`FISH_TYPES`]**: 0x23 is also
/// carried by a deer. See [`catch_class`].
pub const FISH_CLASSES: &[u8] = &[0x23, 0x65, 0x83];

/// The [`type_byte`] values on which a [`FISH_CLASSES`] byte means a fish.
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
/// 6 too (Kiwi, class 0x75; class 0x5C), which is why 0x44 - a type-6 fish on
/// 25477059, a type-6 bird carcass on 25246367 - is still on neither list.
/// Insects are not held to this: the Firefly Colony is caught by hand on
/// type 3.
pub const FISH_TYPES: &[u8] = &[6];

/// Interaction category bytes seen on **NPCs** - people, not animals.
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
/// skinned in game read 0xD9, 0x75, 0x44 and 0xD7, none of which any recorded
/// catch has shown, so a whitelist would refuse the very species the feature
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
/// Every caller must treat that as vanilla behaviour: Desert Looter skips the
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
/// with Desert Looter's F7 event recorder running. All three insects read the
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
        // 0x20 is the birds-in-flight class of section 15.5, and the whole
        // reason `None` is not a third `CatchClass` variant: it must never
        // reach a code path that acts on it. 0x44 gave a fish by hand on
        // 25477059 but a bird's drops as a carcass on 25246367, and no survey
        // has read its type yet, so it stays off.
        for other in [0x00, 0x20, 0x2C, 0x44, 0x57, 0x8C, 0x90, 0xFF] {
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

    #[test]
    fn names_are_the_log_vocabulary() {
        assert_eq!(CatchClass::Bug.name(), "bug");
        assert_eq!(CatchClass::Fish.name(), "fish");
    }
}
