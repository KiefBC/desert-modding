//! The catch-count hook: decide how many of a creature a hand catch grants.
//!
//! ## Why this is a code patch and not a table edit
//!
//! Everything else this plugin does is data: a gather node's yields live in
//! `gimmickinfo`, so multiplying them is a matter of rewriting scalars the
//! game has not read yet. Insects and fish are not gimmicks. They are
//! characters, taken with `TrocTrPushCharacterToInventoryOnceTimer`
//! (`docs/reference-internals.md` section 15), and the amount granted is a
//! **hard-coded immediate in the handler** — there is no record to multiply
//! (section 17.6). So this one lever is a patch on the game's own code.
//!
//! ## The site (build 25116796, section 17.3/17.4)
//!
//! Inside `FUN_142a73c20(inventory, &err, actor, creature, &out_list, &mode)`:
//!
//! ```text
//! 0x142a74151  41 B8 01 00 00 00        mov  r8d,1              <- the count
//! 0x142a74157  48 8D 95 D0 01 00 00     lea  rdx,[rbp+0x1d0]
//! 0x142a7415E  48 8D 8D B0 00 00 00     lea  rcx,[rbp+0xb0]
//! 0x142a74165  E8 ...                   call FUN_14234f210      ; (out, &item_index, count)
//! ```
//!
//! [`CATCH_SITE`] matches those 21 bytes and hits exactly once in the mapped
//! image, so nothing here is hard-coded to an address. The hook steals the
//! first **13** bytes — the whole `mov r8d,1` and the whole
//! `lea rdx,[rbp+0x1d0]`, both position-independent — which is the 12 the
//! `mov rax,<stub>; jmp rax` patch needs plus one `nop`. The stub does not
//! replay the `mov r8d,1`: it *replaces* it with our callback's return value
//! ([`desert_core::trampoline::count_hook_stub`]), replays the `lea` and
//! jumps back to `site + 13`.
//!
//! Three facts about the site make that stub legal, all read off the
//! disassembly: `r15` holds the creature actor there (the instructions just
//! past the call reload `[r15+0x68]` for the same component this callback
//! reads), every volatile register is dead because the very next thing is a
//! call whose only arguments — rcx, rdx, r8 — are all set after this point,
//! and `rsp` is 16-byte aligned because that call is about to happen. The one
//! branch that reaches this range (`je 0x2a74151` at `0x2a74031`) targets its
//! first byte, not the middle of it.
//!
//! ## What the callback may and may not do
//!
//! It runs on a game thread, inside the game's own function, with the game's
//! registers live around it. It reads two pointer chains through
//! `desert_core::safe`, loads a couple of atomics and returns. It never calls
//! a game function, never allocates beyond a log line, never blocks, and
//! **returns 1 on every single failure** — an unreadable pointer, a type byte
//! that is not a creature's, a class byte no recorded catch has ever shown.
//! Vanilla is always the right answer when anything is unclear.
//!
//! `FUN_142a73c20` has five callers and only one is the catch event handler
//! (section 17.6), so the class gate is not a nicety: it is the only thing
//! keeping this multiplier off whatever the other four grant.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use desert_core::creature::{self, CATCH_BYTES, CATCH_REPLACED, CATCH_SITE, CATCH_STOLEN};
use desert_core::pattern::{Found, Pattern};
use desert_core::safe;
use desert_core::trampoline;

use crate::config::{LIVE, MULT_MAX, MULT_MIN};
use crate::module::MainModule;

// Re-exported for the same reason `crate::hook` re-exports them.
pub use desert_core::hook::{hex, install_raw, CountCallback, Hook};

/// Where the class byte lives, relative to the creature actor.
///
/// `*(*(actor+0x68)+0x20)` is the `ClientStatusActorComponent`, and `+0x5A`
/// is the interaction category byte (`docs/reference-internals.md` sections
/// 15.6 and 17.3). The `+0x20` slot is the game's own constant here: the
/// instructions right after the patched call read the same component the same
/// way (`mov rdx,[r15+0x68]; mov rax,[rdx+0x20]`), so this is not the RTTI
/// component lookup Desert Looter does — it is what this function does.
mod off {
    pub const ACTOR_SUB: usize = 0x68;
    pub const SUB_STATUS: usize = 0x20;
    pub const STATUS_CLASS: usize = 0x5A;
    /// `*(actor+0x88) -> byte @1`: 6 on a creature that can be caught.
    pub const ACTOR_TYPE_OBJ: usize = 0x88;
    pub const TYPE_BYTE: usize = 1;
}

/// The type byte a catchable creature has (`desert_looter::actors`'s
/// `CATCHABLE_TYPE`; the game's own steal check switches on the same byte).
const CATCHABLE_TYPE: u8 = 6;

/// Ceiling on `[catch]` lines per session. Catches are rare — a good session
/// is a few dozen — so this is only there to stop a pathological caller of
/// `FUN_142a73c20` from filling the log.
pub const CATCH_LOG_CAP: u32 = 500;

/// `[catch]` lines emitted so far.
static CATCH_LINES: AtomicU32 = AtomicU32::new(0);

/// One bit per class byte value, set the first time an unknown class is seen,
/// so the "not a known bug/fish class" line is logged once per value per
/// session. A bitmap of atomics rather than anything with a lock: this is
/// touched from a game thread inside the game's own function.
static UNKNOWN_LOGGED: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// True the first time this class byte is passed.
fn first_unknown(class: u8) -> bool {
    let Some(word) = UNKNOWN_LOGGED.get(usize::from(class >> 6)) else { return false };
    let bit = 1u64 << (class & 63);
    word.fetch_or(bit, Ordering::Relaxed) & bit == 0
}

/// True while the `[catch]` line budget lasts.
fn may_log() -> bool {
    CATCH_LINES.fetch_add(1, Ordering::Relaxed) < CATCH_LOG_CAP
}

/// `*(actor+0x88) -> byte @1`, the actor type byte.
fn type_byte(actor: usize) -> Option<u8> {
    let obj = safe::read_ptr(actor + off::ACTOR_TYPE_OBJ)?;
    safe::read(obj + off::TYPE_BYTE)
}

/// `*(*(actor+0x68)+0x20) + 0x5A`, the interaction category byte.
fn class_byte(actor: usize) -> Option<u8> {
    let sub = safe::read_ptr(actor + off::ACTOR_SUB)?;
    let status = safe::read_ptr(sub + off::SUB_STATUS)?;
    safe::read(status + off::STATUS_CLASS)
}

/// The count the game should use for this creature, or 1 for vanilla.
///
/// Split out of the callback so the decision has one exit and the callback
/// has none of its own. Every `None` on the way is vanilla.
fn count_for(actor: usize) -> u32 {
    // Not a creature at all: one of the four non-event callers of
    // `FUN_142a73c20`, or an actor whose pointers are not readable this
    // instant. Silent — this is the common case for those callers.
    if type_byte(actor) != Some(CATCHABLE_TYPE) {
        return 1;
    }
    let Some(class) = class_byte(actor) else { return 1 };

    let Some(kind) = creature::catch_class(class) else {
        if first_unknown(class) && may_log() {
            crate::log!(
                "[catch] class={class:02X} not a known bug/fish class; vanilla \
                 (logged once per class)"
            );
        }
        return 1;
    };

    if !LIVE.enabled() {
        return 1;
    }
    // The ini parser already refuses anything outside the range, but this
    // number goes straight into a register the game builds an inventory stack
    // from, so it is clamped here too rather than trusted.
    let mult = LIVE.catch_multiplier(kind).clamp(MULT_MIN, MULT_MAX);
    if mult <= 1 {
        return 1;
    }
    if LIVE.dry_run() {
        if may_log() {
            crate::log!(
                "[dry] {} class={class:02X} would be x{mult}, granting 1",
                kind.name()
            );
        }
        return 1;
    }
    if may_log() {
        crate::log!("[catch] {} class={class:02X} -> x{mult}", kind.name());
    }
    mult
}

/// Installed over the `mov r8d,1` at the catch site. Runs on a game thread,
/// once per creature granted, and its return value is the count.
///
/// # Safety
/// Called from a [`trampoline::count_hook_stub`] stub with `r15` in `rcx`,
/// which at that site is the creature actor. It only reads through
/// `desert_core::safe`, writes nothing anywhere, calls no game function and
/// returns a value in `MULT_MIN..=MULT_MAX`.
pub unsafe extern "system" fn on_catch_count(actor: usize) -> u32 {
    count_for(actor)
}

/// Find the catch site, check the 13 bytes really are what we are about to
/// overwrite, and patch. Any doubt means refusing: the gather multipliers go
/// on working and only catches stay vanilla.
///
/// Installed at start, like the record-loader hook. A catch cannot happen
/// before the world exists, so no thread is executing these bytes yet.
pub fn install_catch_hook(module: &MainModule) -> bool {
    let Some(pat) = Pattern::parse(CATCH_SITE) else {
        crate::log!("[catch] the catch-site pattern is malformed; catch multipliers off");
        return false;
    };
    let target = match pat.find_unique(module.bytes()) {
        Found::Unique(off) => module.base + off,
        Found::None => {
            crate::log!("[catch] signature not found; catch multipliers off");
            return false;
        }
        Found::Ambiguous(n) => {
            crate::log!("[catch] signature ambiguous ({n} hits); catch multipliers off");
            return false;
        }
    };

    let mut have = [0u8; CATCH_STOLEN];
    if !safe::read_into(target, &mut have) {
        crate::log!(
            "[catch] site at +0x{:X} unreadable; NOT hooking",
            module.rva(target)
        );
        return false;
    }
    if have != CATCH_BYTES {
        crate::log!(
            "[catch] site at +0x{:X} is {} not {}; NOT hooking",
            module.rva(target),
            hex(&have),
            hex(&CATCH_BYTES)
        );
        return false;
    }

    // The tail of the stolen bytes: everything the stub replays, which is
    // every stolen instruction except the `mov r8d,imm32` it replaces.
    let Some(replay) = CATCH_BYTES.get(CATCH_REPLACED..) else {
        crate::log!("[catch] internal: REPLACED past the stolen bytes; NOT hooking");
        return false;
    };
    // Through the alias first: that is what makes the compiler check the
    // callback really has the one-pointer-in, u32-out shape the stub calls,
    // and it is what stops the cast being a bare function-item-to-integer.
    let callback: CountCallback = on_catch_count;
    let stub_code =
        trampoline::count_hook_stub(callback as usize, target + CATCH_STOLEN, replay);

    // SAFETY: `target` is the unique `CATCH_SITE` hit inside the running
    // image and its 13 bytes were just read back and compared byte for byte
    // with `CATCH_BYTES`, so they are the whole, position-independent
    // `mov r8d,1` and `lea rdx,[rbp+0x1d0]` of section 17.4 and nothing
    // branches into the middle of them. `stub_code` calls `on_catch_count`
    // with `r15` — the creature actor at that site — puts its `u32` result in
    // `r8d` in place of the replaced immediate, replays the `lea` and jumps
    // to `target + CATCH_STOLEN`, so the game's own code continues from the
    // next instruction with every non-volatile register untouched; the
    // volatile ones it clobbers are dead there, and `rsp` is 16-aligned so
    // the stub's own call is ABI-correct. This runs at plugin start, long
    // before a world exists to catch anything in, so no thread is executing
    // those bytes.
    match unsafe { install_raw(target, CATCH_STOLEN, &stub_code) } {
        Ok(h) => {
            crate::log!(
                "[catch] hook at +0x{:X} -> stub 0x{:X}; original bytes: {}",
                module.rva(h.target),
                h.stub,
                hex(&h.original)
            );
            true
        }
        Err(e) => {
            crate::log!("[catch] install FAILED: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pattern_the_scan_uses_parses() {
        let pat = Pattern::parse(CATCH_SITE).expect("the catch-site pattern parses");
        assert_eq!(pat.len(), 21, "the 21 bytes of section 17.4");
        // That the stolen bytes are a literal prefix of it, and that they are
        // the two instructions they claim to be, is asserted in desert-core
        // right next to the constants themselves.
    }

    #[test]
    fn the_stub_jumps_back_past_every_stolen_byte() {
        let site = 0x1_4000_0000usize;
        let replay = CATCH_BYTES.get(CATCH_REPLACED..).expect("replay tail");
        let s = trampoline::count_hook_stub(0xCAFE, site + CATCH_STOLEN, replay);
        // The stub ends with `mov rax,imm64` (10 bytes) then `jmp rax` (2).
        let tail = s.len() - 12;
        assert_eq!(s.get(tail..tail + 2), Some(&[0x48u8, 0xB8][..]));
        let imm: [u8; 8] = s.get(tail + 2..tail + 10).and_then(|b| b.try_into().ok()).expect("imm64");
        assert_eq!(u64::from_le_bytes(imm) as usize, site + CATCH_STOLEN);
        assert_eq!(s.get(tail + 10..), Some(&[0xFFu8, 0xE0][..]));
    }

    #[test]
    fn unknown_classes_are_reported_once_each() {
        // The bitmap is a session-global, so use values the rest of the file
        // never touches. 0x11 and 0x51 share a low six bits, which is the
        // interesting case: they must not mask each other out.
        assert!(first_unknown(0x11));
        assert!(!first_unknown(0x11));
        assert!(first_unknown(0x51));
        assert!(!first_unknown(0x51));
        assert!(first_unknown(0xFE));
        assert!(!first_unknown(0xFE));
    }

    #[test]
    fn every_class_byte_lands_in_the_bitmap() {
        // 256 values over four 64-bit words, with no gap and no overlap.
        for c in 0u8..=255 {
            assert!(UNKNOWN_LOGGED.get(usize::from(c >> 6)).is_some(), "0x{c:02X}");
        }
    }
}
