//! The `Hunting` multiplier: how much a looted carcass gives.
//!
//! ## Why this is a hook and not a table edit
//!
//! A carcass's loot does not come from one table. It comes from two arrays on
//! the animal's own `characterinfo` record, **both** rolled in one call by
//! `FUN_141e93920`, and a species can leave either empty:
//!
//! * `rec+0x2B8` - per-species item rows, private to that record. Multiplying
//!   these in the table would be clean, and would reach perhaps half the
//!   species.
//! * `rec+0x280` - keys into **`dropsetinfo`**, the game's general-purpose
//!   reward table. `FUN_1422179c0`, which rolls it, is reached from 35 call
//!   sites across the exe: a row is a shared reward definition with nothing
//!   marking one as a carcass's, so raising a row's amounts would raise it for
//!   every chest and mission that names the same row. Worse,
//!   `desert_dispatch::apply` already writes those exact two fields for its own
//!   `Rewards` lever and remembers what it first saw as vanilla, so a second
//!   subsystem writing them first would leave dispatch unable to revert.
//!
//! So this scales **downstream of both**, where the two routes have already
//! converged onto one corpse and been separated by loot method. Nothing static
//! is edited, so there is nothing to remember and nothing to put back:
//! uninstalling the plugin is the revert.
//!
//! ## Why the grant and not before the event
//!
//! The first build of this did the multiply from the plugin's own sweep
//! callback, just before the skinning event went out. It was wrong, and wrong
//! in the way this whole feature was investigated to avoid - it worked on the
//! species that happened to be tested and silently did nothing for the rest.
//!
//! The roll is **lazy**. `FUN_142ab8140` rolls the two arrays only when it
//! finds the latch at `comp+0x6C` still zero, and then grants in the same
//! synchronous call, so a write from another thread beforehand has no rows to
//! find. In the 2026-09-15 log every `cat=D9` and `cat=85` corpse read
//! `dd=0/0/0(0)` while dead and unlooted, then skinned normally. Only some
//! species are pre-rolled when they die - `cat=82` reads `1/0/4(4)` - which is
//! exactly what made the pre-send write look like it worked.
//!
//! By the time `FUN_142ab6f20` has the component, every roll path has run.
//!
//! ## What that means for what it covers
//!
//! The hook is on the **grant**, not on the plugin's own send, so `Hunting`
//! reaches a carcass however it was looted: with `GatherCarcass=1` and the
//! gather key, or by skinning one by hand the game's own way. It does not need
//! `GatherCarcass` at all.
//!
//! The gate is the loot method, and it is the tightest one available: the grant
//! is handed a mask of `1 << method` (`FUN_142ab5270`:
//! `mask = 1 << (*param_5 & 0x1f)`), and this only acts when that mask includes
//! **method 0**, the search/skin method. Each row is then tested against its
//! own mask as well, so a row some other method owns is never touched. What
//! this does *not* do is tell a dead animal from any other thing the game
//! grants under method 0 - "search the corpse" is the same method - so if the
//! game ever searches something else this way, that is multiplied too. The
//! README and the ini say so.
//!
//! ## What the callback may and may not do
//!
//! It runs on a game thread, inside the game's own grant, with the game's
//! registers live around it. It reads and writes through `desert_core::safe`
//! only, loads two atomics, and returns. It never calls a game function, never
//! allocates beyond a log line, never blocks, and **leaves vanilla behind on
//! every failure** - an unreadable pointer, a count outside the band a drop
//! amount lives in, a row whose mask is not skinning's. The rules are
//! `desert-gatherer/src/catch.rs`'s, for the same reason.

use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use desert_core::hook;

use crate::actors;
use crate::config::HUNTING_RANGE;
use crate::game;
use crate::module::MainModule;

/// Bytes the hook steals from `FUN_142ab6f20`'s prologue:
///
/// ```text
/// 48 89 5C 24 08        mov  [rsp+0x08],rbx
/// 4C 89 4C 24 20        mov  [rsp+0x20],r9
/// 55                    push rbp
/// 56                    push rsi
/// 57                    push rdi
/// ```
///
/// Five whole instructions, none RIP-relative and none a branch, which is what
/// the trampoline needs. Thirteen is the first whole-instruction boundary at or
/// past the twelve a `mov rax,imm64; jmp rax` patch takes.
pub const GRANT_STOLEN: usize = 13;

/// The bytes those [`GRANT_STOLEN`] must be, checked before patching. A wrong
/// patch here is a crash to desktop; a refusal is a plugin whose carcasses pay
/// vanilla.
pub const GRANT_PROLOGUE: [u8; GRANT_STOLEN] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, // mov [rsp+0x08],rbx
    0x4C, 0x89, 0x4C, 0x24, 0x20, // mov [rsp+0x20],r9
    0x55, // push rbp
    0x56, // push rsi
    0x57, // push rdi
];

/// The live multiplier, read by the callback on every method-0 grant.
///
/// An atomic rather than a copy parked with a request: this callback is
/// reached from the game's own code on paths the plugin never initiates - a
/// carcass skinned by hand is one - so there is no request to park anything on.
/// `Relaxed` throughout for `catch.rs`'s reason: nothing here synchronises with
/// any other memory access, only the eventual visibility of a new value, and a
/// grant that reads one tick's older number is a carcass at the previous
/// setting.
static MULT: AtomicU32 = AtomicU32::new(1);

/// Publish the `Hunting` value. Called from the plugin thread at startup and on
/// every ini reload; the callback only ever reads.
pub fn set_multiplier(n: u32) {
    // A value outside the range `config::parse` enforces cannot come from the
    // ini, but this is the last gate in front of a write into game memory and
    // it costs nothing to be the one that holds.
    MULT.store(n.clamp(HUNTING_RANGE.0, HUNTING_RANGE.1), Ordering::Relaxed);
}

/// What [`set_multiplier`] last published.
pub fn multiplier() -> u32 {
    MULT.load(Ordering::Relaxed)
}

/// How many components the "already multiplied" ring remembers.
///
/// It exists for one case, and the case is close-range in time: an inventory
/// add that fails on a full bag puts the refused rows **back** on the component
/// (`FUN_142ab5270` -> `FUN_142090560`), at their already-multiplied counts, and
/// the carcass stays a target. The next attempt comes one `NodeCooldown` later,
/// 8 s by default, so anything that has survived 64 *multiplied carcasses* is
/// long since granted and empty. A ring rather than a growing list because this
/// runs on a game thread inside the game's own function: it may not allocate and
/// it may not lock.
const SEEN: usize = 64;

/// Components already multiplied, newest overwriting oldest.
static SEEN_COMPS: [AtomicUsize; SEEN] = [const { AtomicUsize::new(0) }; SEEN];
/// Where the next component goes.
static SEEN_NEXT: AtomicUsize = AtomicUsize::new(0);

/// Whether this component has already had its counts raised.
fn already_multiplied(comp: usize) -> bool {
    comp != 0 && SEEN_COMPS.iter().any(|s| s.load(Ordering::Relaxed) == comp)
}

/// Remember a component whose counts were actually raised.
///
/// **Only a component something was written to may be remembered**, and that is
/// load bearing rather than tidy. A component address is a heap address: the
/// game frees it when the corpse goes, and the allocator hands the same address
/// out again. Every entry in this ring is therefore a landmine for whatever
/// lands on that address next, which would read as "already multiplied" and pay
/// vanilla. So the ring is kept as short as it can be: entries are spent only
/// on carcasses that really were multiplied, never on the many method-0 grants
/// that reach this hook with nothing on either list for us.
///
/// That matters most for **caught creatures**. `FUN_142a75360`, the insect and
/// fish catch handler, grants through this very function with mask 1, and a
/// caught creature is destroyed at once - so its component address is exactly
/// the kind the allocator reuses next. Claiming a slot per catch would have put
/// twenty dead addresses in the ring for one firefly colony.
///
/// The residual is still real and is not closed here: two carcasses that both
/// have loot and land on the same address within 64 multiplied carcasses of
/// each other, and the second pays vanilla. [`on_grant`] logs that rather than
/// letting it pass in silence, which is the property that matters - a
/// multiplier that quietly does nothing is the failure this whole feature was
/// investigated to avoid.
///
/// Two grants for the same component arriving here at once would both be
/// remembered and the carcass multiplied twice. The game grants loot on the
/// thread that processes the event, and two grants for one corpse in flight
/// together is not a thing that happens; a lock that would close it is not
/// allowed on this thread anyway.
fn remember(comp: usize) {
    if comp == 0 {
        return;
    }
    let i = SEEN_NEXT.fetch_add(1, Ordering::Relaxed) % SEEN;
    // `get` rather than an index: `i` is already reduced modulo the length, but
    // a panic on a game thread inside the game's own function is not something
    // to leave to an invariant nobody can check at runtime.
    if let Some(slot) = SEEN_COMPS.get(i) {
        slot.store(comp, Ordering::Relaxed);
    }
}

/// Ceiling on `[hunting]` lines per session. Every skinned carcass writes one,
/// so a long session is a few hundred; the cap is there for a pathological
/// caller, not for an honest one.
pub const LOG_CAP: u32 = 500;
static LINES: AtomicU32 = AtomicU32::new(0);

/// True while there is still room in the log for another line, and says so once
/// as it runs out.
fn may_log() -> bool {
    let n = LINES.fetch_add(1, Ordering::Relaxed);
    if n == LOG_CAP {
        crate::log!("[hunting] {LOG_CAP} lines logged; no more this session");
    }
    n < LOG_CAP
}

/// `FUN_142ab6f20(component, &mask, flag, &out)` - the loot grant.
///
/// # Safety
///
/// Only the trampoline installed on `FUN_142ab6f20` may call this, with the
/// game's own `rcx`/`rdx` for that call. `comp` and `mask_ptr` are treated as
/// untrusted addresses and read through `desert_core::safe` only; no game
/// function is called and nothing is retained past the call.
pub unsafe extern "system" fn on_grant(comp: usize, mask_ptr: usize, _r8: usize, _r9: usize) {
    let mult = multiplier();
    if mult <= 1 {
        return;
    }
    // The mask is `1 << method`. Method 0 is search/skin; anything without that
    // bit is some other kind of loot and is not this key's business.
    let Some(mask) = crate::safe::read::<u32>(mask_ptr) else { return };
    if mask & actors::SKIN_METHOD_MASK == 0 {
        return;
    }
    if already_multiplied(comp) {
        // Either the put-back this guard exists for - a full bag refused the
        // stacks and the game handed them back at their raised counts - or an
        // address the allocator has reused. The two are indistinguishable from
        // here, so the safe answer is the same for both and the line is what
        // tells them apart afterwards: a carcass that pays vanilla says so.
        if may_log() {
            crate::log!(
                "[hunting] comp=0x{comp:X} left vanilla: already multiplied once (a retry after a \
                 full bag, or this address has been reused)"
            );
        }
        return;
    }
    match actors::multiply_carcass_drops(comp, mult) {
        Ok(n) if n.changed() == 0 && n.refused == 0 => {
            // Common and uninteresting: most method-0 grants reaching here have
            // nothing on either list for us - every insect and fish caught by
            // hand comes through here too. Silent, and no slot spent: see
            // `remember`.
        }
        Ok(n) => {
            if n.changed() > 0 {
                remember(comp);
            }
            if may_log() {
                crate::log!(
                    "[hunting] comp=0x{comp:X} x{mult}: {} item rows, {} drop-set rows \
                     multiplied ({} not skinning's, {} refused)",
                    n.item_rows, n.set_rows, n.not_ours, n.refused
                );
            }
        }
        Err(e) => {
            if may_log() {
                crate::log!("[hunting] comp=0x{comp:X} left vanilla: {e}");
            }
        }
    }
}

/// Patch the loot grant's prologue. Done during load, alongside the looter's
/// other two hooks and for the same reason: a prologue can only be rewritten
/// while no thread is executing it.
///
/// Returns whether the hook went in. A `false` is a plugin whose carcasses pay
/// vanilla and whose log says why, which is the right failure: the alternative
/// to a refusal here is a wrong patch, and that is a crash to desktop.
pub fn install(module: &MainModule, anchors: &game::Anchors) -> bool {
    let Some(target) = anchors.get(game::LOOT_GRANT_SIG) else {
        crate::log!("[hunting] loot_grant signature missing; the Hunting multiplier is off");
        return false;
    };
    let mut have = [0u8; GRANT_STOLEN];
    if !crate::safe::read_into(target, &mut have) || have != GRANT_PROLOGUE {
        crate::log!(
            "[hunting] loot_grant prologue at +0x{:X} is {} not the expected bytes; NOT hooking",
            module.rva(target),
            hook::hex(&have)
        );
        return false;
    }
    // SAFETY: `target` is the unique `loot_grant` signature hit inside the main
    // module, and the `GRANT_STOLEN` bytes there were just read back and
    // matched against `GRANT_PROLOGUE`, so they really are the five whole,
    // position-independent instructions that constant documents - no
    // RIP-relative operand, no branch, and no xref lands inside them. This runs
    // during load, before any thread executes that prologue, and `on_grant` has
    // the four-integer-argument shape the stub calls it with.
    match unsafe { hook::install(target, GRANT_STOLEN, on_grant) } {
        Ok(h) => {
            crate::log!(
                "[hunting] loot_grant +0x{:X} -> stub 0x{:X}; original bytes: {}",
                module.rva(h.target),
                h.stub,
                hook::hex(&h.original)
            );
            true
        }
        Err(e) => {
            crate::log!("[hunting] loot_grant install FAILED: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes the hook checks before patching have to be the head of the
    /// signature that found the function, or it is vetting one thing and
    /// patching another - the same property `desert_core::creature`'s catch
    /// site is held to.
    #[test]
    fn the_stolen_bytes_are_a_literal_prefix_of_the_signature() {
        let sig: Vec<&str> = game::LOOT_GRANT_PATTERN.split_whitespace().collect();
        let head: Vec<String> = sig.iter().take(GRANT_STOLEN).map(|s| s.to_string()).collect();
        let want: Vec<String> = GRANT_PROLOGUE.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(head, want);
        // None of the stolen bytes may be a wildcard: they are replayed
        // verbatim by the stub.
        assert!(!head.iter().any(|t| t == "??"), "{head:?}");
        // The patch does not fit in fewer, and the trampoline refuses more.
        const {
            assert!(GRANT_STOLEN >= desert_core::trampoline::MIN_STOLEN);
            assert!(GRANT_STOLEN <= desert_core::trampoline::MAX_STOLEN);
        }
    }

    /// The multiplier published is the multiplier read, and a value the ini
    /// parser would never produce is still clamped into range rather than
    /// reaching game memory.
    #[test]
    fn the_published_multiplier_is_clamped_to_the_key_range() {
        set_multiplier(5);
        assert_eq!(multiplier(), 5);
        set_multiplier(0);
        assert_eq!(multiplier(), HUNTING_RANGE.0, "0 would zero a carcass out");
        set_multiplier(u32::MAX);
        assert_eq!(multiplier(), HUNTING_RANGE.1);
        set_multiplier(1);
        assert_eq!(multiplier(), 1);
    }

    /// The guard that keeps a carcass from being multiplied twice when the
    /// game puts refused stacks back on a full bag.
    #[test]
    fn a_remembered_component_is_refused_until_it_falls_out_of_the_ring() {
        // A distinctive base, so this test cannot collide with another's
        // entries in the shared ring.
        let base = 0x7E57_0000_0000;
        assert!(!already_multiplied(base + 0x10), "first sight");
        remember(base + 0x10);
        assert!(already_multiplied(base + 0x10), "the put-back retry must be refused");
        assert!(!already_multiplied(base + 0x20), "a different carcass is unaffected");
        // A null component never claims a slot, and never answers true - or the
        // first unreadable component would pin the ring on address zero.
        remember(0);
        assert!(!already_multiplied(0));
        // Filling the ring evicts the oldest, which is the deliberate bound:
        // the retry it exists to catch comes one NodeCooldown later, not 64
        // carcasses later.
        for i in 0..SEEN {
            remember(base + 0x1000 + i * 0x40);
        }
        assert!(!already_multiplied(base + 0x10), "evicted after a full ring");
    }

    /// The ring's entries are the cost of the whole guard: each one is an
    /// address that will read as "already multiplied" if the allocator hands it
    /// out again. A grant that multiplied nothing must not spend one - which is
    /// every insect and fish caught by hand, since the catch handler grants
    /// through the same function with the same mask.
    #[test]
    fn a_grant_that_multiplied_nothing_spends_no_slot() {
        let base = 0x7E58_0000_0000;
        let before = SEEN_NEXT.load(Ordering::Relaxed);
        // What `on_grant` does with `Scaled::default()`: nothing changed, so
        // nothing is remembered.
        let n = actors::Scaled::default();
        assert_eq!(n.changed(), 0);
        if n.changed() > 0 {
            remember(base);
        }
        assert_eq!(SEEN_NEXT.load(Ordering::Relaxed), before, "no slot may be spent");
        assert!(!already_multiplied(base));
    }
}
