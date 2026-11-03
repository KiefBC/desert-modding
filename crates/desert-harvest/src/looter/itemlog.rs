//! The item-log filter: a gate hook on the handler that builds each "Item xN"
//! line of the HUD's side item log.
//!
//! ## The function (build 26361232, found in game 2026-11-03)
//!
//! `0x116A5B0(this, const Grant* g, u16 source, u16 r9, ..., bool skip)`,
//! reached only through a `jmp` thunk in the UI notification manager's
//! handler table (`0x11729F2`). It returns at once when its stack `bool` is
//! set or `source` is 7 or 0x18; otherwise it looks the item up with
//! `0x3DCFC0(u16 index)` where the index is the `u16` at `g+8` - the
//! `iteminfo` record **index**, not the `u32` item key - builds one 0xF8-byte
//! `ItemLogData` and queues it for the side list. Its early-out leaves `eax`
//! unset: it is `void`.
//!
//! A probe build hooked all seven `ItemLogData` builders in that manager.
//! Only this one fired on pickups, once per `[recv]` line in the same
//! millisecond, with `g+8` = 1201 for a Dragonfly (`iteminfo` index 1201,
//! key 1001245) and 1293 for a Peony (index 1293, key 757006). `source` was
//! 0xD for the bug and 0x1C for the flowers. The first guess, `0x116C7A0`,
//! never fired: it is another notification.
//!
//! ## Why skipping it is safe
//!
//! The stub ([`desert_core::trampoline::gate_hook_stub`]) returns straight to
//! the thunk's caller when [`on_item_log`] says no. That is the very path the
//! function takes itself for an `0xFFFF` key - prologue, then epilogue, with
//! nothing allocated, queued or released - so a hidden line hands the game a
//! state it already produces. The item is granted elsewhere, before this
//! handler is told about it: the bag, the `[recv]` lines and every quest
//! condition (`HandleGameEvent`) are upstream and untouched. Only the line
//! is not built.
//!
//! ## What the callback may and may not do
//!
//! It runs on a game thread inside the game's UI code. It reads through
//! `desert_core::safe` only, loads a few atomics, and returns. It never calls
//! a game function, never allocates beyond a debug line, never blocks, and
//! **shows the line on every failure** ([`crate::looter::logfilter`]).

use std::sync::atomic::{AtomicU32, Ordering};

use desert_core::hook::{self, GateCallback};
use desert_core::manager::{self, Slot};
use desert_core::trampoline;

use crate::looter::logfilter::{self, Decision, ItemFacts, COUNTERS, LIVE};
use crate::looter::{game, tables};
use crate::module::MainModule;

/// Bytes the hook steals from the handler's prologue (build 26361232):
///
/// ```text
/// 48 89 5C 24 10        mov  [rsp+0x10],rbx
/// 48 89 6C 24 18        mov  [rsp+0x18],rbp
/// 48 89 74 24 20        mov  [rsp+0x20],rsi
/// ```
///
/// Three whole instructions, none RIP-relative and none a branch
/// (`retarget prologue 116a5b0`: 15 is a safe count). The gate stub replays
/// them at the entry `rsp`, so the spills land where the original puts them.
pub const ITEM_LOG_STOLEN: usize = 15;

/// The bytes those [`ITEM_LOG_STOLEN`] must be, checked before patching.
pub const ITEM_LOG_PROLOGUE: [u8; ITEM_LOG_STOLEN] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, // mov [rsp+0x10],rbx
    0x48, 0x89, 0x6C, 0x24, 0x18, // mov [rsp+0x18],rbp
    0x48, 0x89, 0x74, 0x24, 0x20, // mov [rsp+0x20],rsi
];

/// Where the `u16` item index sits in the grant the handler is given.
const GRANT_INDEX: usize = 0x08;

/// The index no record has; read as "no item" and left to the handler.
const NO_ITEM: u16 = 0xFFFF;

/// Ceiling on `[itemlog]` debug lines per session. One per pickup line under
/// `Debug=1`; the cap is there for a looting spree, not for an honest session.
pub const LOG_CAP: u32 = 500;
static LINES: AtomicU32 = AtomicU32::new(0);

fn may_log() -> bool {
    let n = LINES.fetch_add(1, Ordering::Relaxed);
    if n == LOG_CAP {
        crate::log!("[itemlog] {LOG_CAP} debug lines logged; no more this session");
    }
    n < LOG_CAP
}

/// The parsed `ItemInfo` record at `idx`, through the `iteminfo` manager the
/// looter's accessor scan resolved. `None` on any failed or implausible read.
fn item_record(idx: u16) -> Option<usize> {
    let slot = tables::item_slot_addr()?;
    let (count, records) = manager::view(slot)?;
    if u32::from(idx) >= count {
        return None;
    }
    match manager::slot(records, usize::from(idx)) {
        Slot::Loaded(rec) => Some(rec),
        Slot::Empty | Slot::Unreadable => None,
    }
}

/// `itemType` and `itemTier` of a record.
fn facts(rec: usize) -> Option<ItemFacts> {
    Some(ItemFacts {
        item_type: crate::safe::read::<u8>(rec.checked_add(logfilter::ITEM_TYPE_OFF)?)?,
        tier: crate::safe::read::<u8>(rec.checked_add(logfilter::ITEM_TIER_OFF)?)?,
    })
}

/// `0x116A5B0(this, grant, source, r9, ...)` - one side item-log line about
/// to be built. Returns `0` to skip the handler (the line is not shown),
/// anything else to run it.
///
/// # Safety
///
/// Only the gate stub installed on the item-log handler may call this, with
/// the game's own `rcx`/`rdx`/`r8`/`r9` for that call. `grant` is treated as
/// an untrusted address and read through `desert_core::safe` only; no game
/// function is called and nothing is retained past the call.
pub unsafe extern "system" fn on_item_log(_this: usize, grant: usize, source: usize, r9: usize) -> u8 {
    let min_tier = LIVE.min_tier();
    let debug = LIVE.debug();
    if min_tier == 0 && !debug {
        // The vanilla log: no reads at all. Still counted, which is what lets
        // the stat line say the hook is being reached.
        COUNTERS.record(Decision::Vanilla);
        return 1;
    }
    let idx = grant.checked_add(GRANT_INDEX).and_then(crate::safe::read::<u16>);
    if idx == Some(NO_ITEM) {
        return 1;
    }
    let rec = idx.and_then(item_record);
    let item = rec.and_then(facts);
    let d = logfilter::decide(min_tier, LIVE.money(), item);
    COUNTERS.record(d);
    if debug && may_log() {
        let opt = |v: Option<String>| v.unwrap_or_else(|| "?".to_string());
        crate::log!(
            "[itemlog] debug: idx={} source=0x{:X} r9={} type={} tier={} key={} -> {} ({d:?})",
            opt(idx.map(|i| i.to_string())),
            source & 0xFFFF,
            r9 & 0xFFFF,
            opt(item.map(|f| f.item_type.to_string())),
            opt(item.map(|f| f.tier.to_string())),
            opt(rec.and_then(tables::item_record_key_latched).map(|k| k.to_string())),
            d.word()
        );
    }
    u8::from(d.shows())
}

/// Log the `[itemlog] stat:` line when it moved since `last`. Called by the
/// harvest thread on the 60 s summary cadence.
pub fn log_counters(last: &mut Option<String>) {
    let now = COUNTERS.stat_line();
    if now.is_some() && now != *last {
        if let Some(line) = &now {
            crate::log!("[itemlog] stat: {line}");
        }
        *last = now;
    }
}

/// Patch the item-log handler's prologue. Done during load with the looter's
/// other hooks, before any pickup can reach it.
///
/// Returns whether the hook went in. A `false` is a vanilla item log whose
/// log says why: a refusal is always the right failure, a wrong patch is a
/// crash to desktop.
pub fn install(module: &MainModule, anchors: &game::Anchors) -> bool {
    let Some(target) = anchors.get(game::ITEM_LOG_SIG) else {
        crate::log!("[itemlog] item_log signature missing; the item-log filter is off");
        return false;
    };
    let mut have = [0u8; ITEM_LOG_STOLEN];
    if !crate::safe::read_into(target, &mut have) || have != ITEM_LOG_PROLOGUE {
        crate::log!(
            "[itemlog] handler prologue at +0x{:X} is {} not the expected bytes; NOT hooking",
            module.rva(target),
            hook::hex(&have)
        );
        return false;
    }
    // Through the alias first, so the compiler checks the callback has the
    // four-arguments-in, u8-out shape the stub calls.
    let callback: GateCallback = on_item_log;
    let stub_code = trampoline::gate_hook_stub(target, &have, callback as usize);
    // SAFETY: `target` is the unique `item_log` signature hit inside the main
    // module, and its `ITEM_LOG_STOLEN` bytes were just read back and matched
    // against `ITEM_LOG_PROLOGUE`: six whole instructions, no RIP-relative
    // operand, no branch, and as the function's entry nothing branches into
    // their middle (its early-outs jump forward to the epilogue). The stub saves and restores rcx/rdx/r8/r9 around
    // `on_item_log`, restores `rsp` to its entry value, and then either
    // replays those bytes and jumps to `target + ITEM_LOG_STOLEN` - the
    // function runs in full - or `ret`s to the thunk's caller, which is the
    // path the function takes itself for a 0xFFFF key: it is `void` and
    // builds nothing on that path. This runs during load, before any pickup
    // can reach the handler, so no thread is executing the prologue.
    match unsafe { hook::install_raw(target, ITEM_LOG_STOLEN, &stub_code) } {
        Ok(h) => {
            crate::log!(
                "[itemlog] handler +0x{:X} -> stub 0x{:X}; original bytes: {}",
                module.rva(h.target),
                h.stub,
                hook::hex(&h.original)
            );
            true
        }
        Err(e) => {
            crate::log!("[itemlog] install FAILED: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes the hook checks before patching are the head of the
    /// signature that found the function, with no wildcard among them: the
    /// stub replays them verbatim.
    #[test]
    fn the_stolen_bytes_are_a_literal_prefix_of_the_signature() {
        let sig: Vec<&str> = game::ITEM_LOG_PATTERN.split_whitespace().collect();
        let head: Vec<String> = sig.iter().take(ITEM_LOG_STOLEN).map(|s| s.to_string()).collect();
        let want: Vec<String> = ITEM_LOG_PROLOGUE.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(head, want);
        assert!(!head.iter().any(|t| t == "??"), "{head:?}");
        const {
            assert!(ITEM_LOG_STOLEN >= desert_core::trampoline::MIN_STOLEN);
            assert!(ITEM_LOG_STOLEN <= desert_core::trampoline::MAX_STOLEN);
        }
    }
}
