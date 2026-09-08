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
//! This lives in desert-core because two plugins need the same answer for
//! different reasons: Desert Looter decides whether to *target* a creature,
//! and Desert Gatherer decides how many of it to *grant* (its catch-count
//! hook, section 17). They must never disagree about what a fish is.
//!
//! The catch-count **site** constants are here for the same reason
//! `gimmick::LOADER_PROLOGUE` is: they are what a game update breaks first,
//! and `desert-core/tests/gimmick_real.rs` checks them against the real exe.

// ---------------------------------------------------------------------------
// The catch-count patch site (build 25116796, section 17.4)
// ---------------------------------------------------------------------------

/// The 21 bytes of section 17.4, unique in the mapped image:
///
/// ```text
/// 0x142a74151  41 B8 01 00 00 00        mov  r8d,1              <- the count
/// 0x142a74157  48 8D 95 D0 01 00 00     lea  rdx,[rbp+0x1d0]
/// 0x142a7415E  48 8D 8D B0 00 00 00     lea  rcx,[rbp+0xb0]
/// 0x142a74165  E8 ...                   call FUN_14234f210      ; (out, &item_index, count)
/// ```
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
/// category byte. There is deliberately no `Unknown` variant: see
/// [`catch_class`].
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
pub const BUG_CLASSES: &[u8] = &[0x80];

/// Interaction category bytes seen on fish caught by hand.
///
/// Four manual catches at a lake (build 25116796, 2026-09-08) read 0x23 and
/// 0x83, item ids 29817, 29805 and 29804; seven more taken by the plugin
/// itself in the same session all read 0x23.
pub const FISH_CLASSES: &[u8] = &[0x23, 0x83];

/// Classify a creature by its interaction category byte.
///
/// `None` means "not a class we have ever watched being caught". Every caller
/// must treat that as vanilla behaviour: Desert Looter skips the actor,
/// Desert Gatherer returns a multiplier of 1. Guessing here would mean
/// grabbing, or scaling, whatever else the game happens to route through the
/// same code - birds in flight read 0x20 and pass every other test.
pub fn catch_class(category: u8) -> Option<CatchClass> {
    if BUG_CLASSES.contains(&category) {
        Some(CatchClass::Bug)
    } else if FISH_CLASSES.contains(&category) {
        Some(CatchClass::Fish)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recorded_classes_map_and_nothing_else_does() {
        assert_eq!(catch_class(0x80), Some(CatchClass::Bug));
        assert_eq!(catch_class(0x23), Some(CatchClass::Fish));
        assert_eq!(catch_class(0x83), Some(CatchClass::Fish));
        // 0x20 is the birds-in-flight class of section 15.5, and the whole
        // reason `None` is not a third `CatchClass` variant: it must never
        // reach a code path that acts on it.
        for other in [0x00, 0x20, 0x2C, 0x44, 0x57, 0x65, 0x8C, 0x90, 0xFF] {
            assert_eq!(catch_class(other), None, "0x{other:02X}");
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
