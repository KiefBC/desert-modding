//! The auto-loot half's helpers, called from the one harvest thread in
//! `crate::entry`. This was the inline `mod entry` of the old `desert-looter`
//! crate's `lib.rs`.

use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::UI::WindowsAndMessaging::MB_OK;

use crate::looter::{actors, events, game};
use crate::module::MainModule;
use crate::{hook, log};

/// The `area_sweep` signature hits 15 bytes into the function; the
/// function starts with three 5-byte `mov [rsp+x],reg` spills, which are
/// exactly the bytes the hook steals (build 25116796, verified by
/// disassembly; the reference mod uses the same 0xF).
const SWEEP_HIT_OFFSET: usize = 0xF;
const SWEEP_STOLEN: usize = 0xF;
/// `mov [rsp+0x10],rbx; mov [rsp+0x18],rsi; mov [rsp+0x20],rdi`
const SWEEP_PROLOGUE: [u8; 15] = [
    0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7C, 0x24, 0x20,
];

/// `enqueue` starts `mov [rsp+0x8],rbx; mov [rsp+0x20],rsi; push rdi;
/// sub rsp,0x20` on build 26345091: 5 + 5 + 1 + 4 bytes, so 15 is the first
/// whole-instruction boundary at or past the 12 the patch takes (11 falls
/// short), and none of the four is RIP-relative or a branch. It was 14 through
/// 25477059 (`mov [rsp+8],rbx; push rdi; sub rsp,0x20; mov rbx,[rdx+0x38]`).
/// All 15 are literal bytes of [`game::ENQUEUE_PATTERN`], which the test below
/// holds them to.
const ENQUEUE_STOLEN: usize = 15;
const ENQUEUE_PROLOGUE: [u8; ENQUEUE_STOLEN] = [
    0x48, 0x89, 0x5C, 0x24, 0x08, // mov [rsp+0x08],rbx
    0x48, 0x89, 0x74, 0x24, 0x20, // mov [rsp+0x20],rsi
    0x57, // push rdi
    0x48, 0x83, 0xEC, 0x20, // sub rsp,0x20
];

/// Observer hook on the game's enqueue, for the record key.
pub(crate) fn install_enqueue_hook(module: &MainModule, anchors: &game::Anchors) -> bool {
    let Some(target) = anchors.get("enqueue") else {
        crate::log!("[hook] enqueue signature missing; no recorder");
        return false;
    };
    let mut have = [0u8; ENQUEUE_STOLEN];
    if !crate::safe::read_into(target, &mut have) || have != ENQUEUE_PROLOGUE {
        crate::log!("[hook] enqueue prologue is {} not the expected bytes; NOT hooking", hook::hex(&have));
        return false;
    }
    // SAFETY: `target` is the unique `enqueue` signature hit inside the
    // main module and the `ENQUEUE_STOLEN` (15) bytes there were just read
    // back and matched against `ENQUEUE_PROLOGUE`, so the bytes the stub
    // copies really are whole, position-independent instructions.
    // This runs during load, before any thread executes that prologue, and
    // `events::on_enqueue` has the four-integer-argument shape the stub
    // calls it with.
    match unsafe { hook::install(target, ENQUEUE_STOLEN, events::on_enqueue) } {
        Ok(h) => {
            crate::log!("[hook] enqueue +0x{:X} -> stub 0x{:X}; original bytes: {}", module.rva(h.target), h.stub, hook::hex(&h.original));
            true
        }
        Err(e) => {
            crate::log!("[hook] enqueue install FAILED: {e}");
            false
        }
    }
}

/// Patch the sweep function's prologue. Done right after signature
/// resolution, while the game is still loading and no thread runs it.
pub(crate) fn install_sweep_hook(module: &MainModule, anchors: &game::Anchors) -> bool {
    let Some(hit) = anchors.get("area_sweep+0xF") else {
        crate::log!("[hook] area_sweep signature missing; no game-thread callback");
        return false;
    };
    let target = hit - SWEEP_HIT_OFFSET;
    let mut have = [0u8; 15];
    if !crate::safe::read_into(target, &mut have) || have != SWEEP_PROLOGUE {
        crate::log!(
            "[hook] area_sweep prologue at +0x{:X} is {} not the expected spills; NOT hooking",
            module.rva(target), hook::hex(&have)
        );
        return false;
    }
    // SAFETY: `target` is the `area_sweep` signature hit backed up by
    // `SWEEP_HIT_OFFSET` and the 15 bytes there were just read back and
    // matched against `SWEEP_PROLOGUE`, so the `SWEEP_STOLEN` (0xF) bytes
    // the stub copies are the three whole, position-independent register
    // spills. This runs during load, before any thread executes that
    // prologue, and `events::on_sweep` has the four-integer-argument shape
    // the stub calls it with.
    match unsafe { hook::install(target, SWEEP_STOLEN, events::on_sweep) } {
        Ok(h) => {
            crate::log!(
                "[hook] area_sweep +0x{:X} -> stub 0x{:X}; original bytes: {}",
                module.rva(h.target), h.stub, hook::hex(&h.original)
            );
            true
        }
        Err(e) => {
            crate::log!("[hook] area_sweep install FAILED: {e}");
            false
        }
    }
}

pub(crate) fn resolve_event_api(module: &MainModule, anchors: &game::Anchors) -> bool {
    match events::resolve(module, anchors) {
        Ok(api) => {
            crate::log!(
                "[event] prepare=+0x{:X} desc_by_id=+0x{:X} alloc_event=+0x{:X} enqueue=+0x{:X} queue_slot=+0x{:X} desc_mask=+0x{:X}",
                module.rva(api.prepare), module.rva(api.desc_by_id), module.rva(api.alloc_event),
                module.rva(api.enqueue), module.rva(api.queue_slot), module.rva(api.desc_mask)
            );
            if api.steal_check != 0 && api.steal_ctx != 0 {
                crate::log!("[event] steal_check=+0x{:X} ctx=+0x{:X}", module.rva(api.steal_check), module.rva(api.steal_ctx));
            } else {
                crate::log!("[event] steal check NOT resolved: every pickup (gather nodes and ground items) will be treated as owned and skipped");
            }
            events::set_api(api);
            true
        }
        Err(e) => {
            crate::log!("[event] api resolve FAILED: {e}");
            false
        }
    }
}

/// Sample the player's position and hand it to `desert_core::telemetry`
/// for the overlay to draw.
///
/// Cheap and unconditional: three guarded pointer reads and six floats,
/// once per 30 ms tick. It needs no hook and no game thread because it
/// only *reads* - the `gs:[0x58]` constraint in `events.rs` is about
/// building and sending events, not about looking at memory, and every
/// read here goes through `safe` like all the others.
///
/// The manager is re-read from its global slot rather than taken from the
/// cached `World`, the same way `game::survey` does it, so the readout
/// survives the manager moving across a load screen. Anything unreadable
/// publishes `None`, which is what puts the menu back to "unavailable"
/// instead of freezing the last coordinates on screen.
pub(crate) fn publish_position(world: Option<&game::World>) {
    let pos = world.and_then(|w| {
        let manager = actors::find_manager_current(w).unwrap_or(w.manager);
        actors::player_actor(manager).and_then(actors::actor_position)
    });
    crate::telemetry::publish(pos.map(|p| crate::telemetry::Position {
        x: p.x,
        y: p.y,
        z: p.z,
    }));
}

/// Walk the descriptor table once the game has built it (after the boot grace).
pub(crate) fn resolve_descriptor(module: &MainModule, api: &events::EventApi) -> bool {
    let t = std::time::Instant::now();
    match events::find_descriptor(module, api, events::PICKUP_DESCRIPTOR) {
        Ok(d) => {
            let note = if d.id == events::PICKUP_ID_EXPECTED && d.payload_size as usize == events::PICKUP_PAYLOAD_SIZE {
                "as expected"
            } else {
                "DIFFERS from the reference mod's 2057/13"
            };
            crate::log!(
                "[event] {} id={} payload={} dispatch={} desc=0x{:X} ({note}, {:.0} ms)",
                d.name, d.id, d.payload_size, d.dispatch, d.ptr, t.elapsed().as_secs_f64() * 1000.0
            );
            events::set_descriptor(d);
            match events::find_descriptor(module, api, events::HANDLE_GAME_EVENT) {
                Ok(h) => {
                    crate::log!("[event] {} id={} payload={} (yield learning armed)", h.name, h.id, h.payload_size);
                    events::set_handle_descriptor(h.ptr);
                }
                Err(e) => crate::log!("[event] {}: {e}; yields will not be learned", events::HANDLE_GAME_EVENT),
            }
            // The catch event is optional: without it insects are simply
            // not caught, and gathering carries on exactly as before.
            match events::find_descriptor(module, api, events::CATCH_DESCRIPTOR) {
                Ok(c) => {
                    let note = if c.id == events::CATCH_ID_EXPECTED
                        && c.payload_size as usize == events::CATCH_PAYLOAD_SIZE
                    {
                        "as expected"
                    } else {
                        "DIFFERS from the recorded 2048/8"
                    };
                    crate::log!(
                        "[event] {} id={} payload={} dispatch={} ({note})",
                        c.name, c.id, c.payload_size, c.dispatch
                    );
                    events::set_catch_descriptor(c);
                }
                Err(e) => crate::log!("[event] {}: {e}; bugs will not be caught", events::CATCH_DESCRIPTOR),
            }
            // The carcass-skinning event is optional on exactly the same
            // terms: without it `GatherCarcass` is simply off and nothing
            // else notices. Never a startup failure.
            match events::find_descriptor(module, api, events::SKIN_DESCRIPTOR) {
                Ok(k) => {
                    let note = if k.id == events::SKIN_ID_EXPECTED
                        && k.payload_size as usize == events::SKIN_PAYLOAD_SIZE
                    {
                        "as expected"
                    } else {
                        "DIFFERS from the recorded 2024/7"
                    };
                    crate::log!(
                        "[event] {} id={} payload={} dispatch={} ({note})",
                        k.name, k.id, k.payload_size, k.dispatch
                    );
                    events::set_skin_descriptor(k);
                }
                Err(e) => crate::log!("[event] {}: {e}; carcasses will not be skinned", events::SKIN_DESCRIPTOR),
            }
            true
        }
        Err(e) => {
            crate::log!("[event] descriptor: {e} ({:.0} ms)", t.elapsed().as_secs_f64() * 1000.0);
            false
        }
    }
}

/// `DesertTooling.yields` beside the log: one `record=item` pair per line.
///
/// Renamed with the merge into one .asi. A `DesertLooter.yields` left over
/// from the separate plugin is simply not read: the cache is rebuilt by
/// playing, which is the same trade the rest of the merge makes.
fn yields_path() -> std::path::PathBuf {
    log::exe_dir().join("DesertTooling.yields")
}

/// Files without this header come from a build that also learned from
/// ground items (wrong: their record is generic) and are ignored.
///
/// It still says `desert-looter`, and must: it is the first line of the
/// player's `DesertTooling.yields` cache, and a file whose first line does not
/// match is thrown away. Renaming it with the crate would discard every learned
/// pair on the first launch after the merge for no gain.
const YIELDS_HEADER: &str = "# desert-looter yields v2";

pub(crate) fn load_yields() {
    let Ok(text) = std::fs::read_to_string(yields_path()) else { return };
    if text.lines().next().map(str::trim) != Some(YIELDS_HEADER) {
        crate::log!("[yield] ignoring an old-format DesertTooling.yields; it will be rewritten");
        return;
    }
    let pairs: Vec<(u16, u32, u32)> = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let (r, rest) = l.trim().split_once('=')?;
            let (i, c) = rest.split_once(',').unwrap_or((rest, "1"));
            Some((r.trim().parse().ok()?, i.trim().parse().ok()?, c.trim().parse().unwrap_or(1)))
        })
        .collect();
    crate::log!("[yield] {} learned record->item pairs loaded", pairs.len());
    events::load_yields(pairs);
}

pub(crate) fn save_yields() {
    let mut body = format!("{YIELDS_HEADER}\n");
    body.extend(events::yields_snapshot().iter().map(|(r, i, c)| format!("{r}={i},{c}\n")));
    // Best effort; the file is a cache and is rebuilt by playing.
    let _ = std::fs::write(yields_path(), body);
}

pub(crate) fn tid_now() -> u32 {
    // SAFETY: `GetCurrentThreadId` takes no arguments and only reads the
    // calling thread's own TEB; it is sound to call from any thread.
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

/// The plugin's only UI: an audible acknowledgement of a hotkey.
pub(crate) fn beep() {
    // SAFETY: `MessageBeep` takes a plain sound-type flag, borrows nothing
    // of ours and has no thread affinity; it is sound to call from the
    // plugin thread at any time.
    unsafe { MessageBeep(MB_OK) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes the enqueue hook checks before patching are the head of the
    /// signature that found the function, with no wildcard among them: the
    /// stub replays them verbatim. `hunting.rs` holds the loot grant to the
    /// same property.
    #[test]
    fn the_enqueue_stolen_bytes_are_a_literal_prefix_of_the_signature() {
        let sig: Vec<&str> = game::ENQUEUE_PATTERN.split_whitespace().collect();
        let head: Vec<String> = sig.iter().take(ENQUEUE_STOLEN).map(|s| s.to_string()).collect();
        let want: Vec<String> = ENQUEUE_PROLOGUE.iter().map(|b| format!("{b:02X}")).collect();
        assert_eq!(head, want);
        assert!(!head.iter().any(|t| t == "??"), "{head:?}");
        const {
            assert!(ENQUEUE_STOLEN >= desert_core::trampoline::MIN_STOLEN);
            assert!(ENQUEUE_STOLEN <= desert_core::trampoline::MAX_STOLEN);
        }
    }

    /// The same for the sweep hook, whose signature hit is `SWEEP_HIT_OFFSET`
    /// past the stolen bytes rather than at them, so only the count is held.
    #[test]
    fn the_sweep_stolen_count_fits_the_trampoline() {
        const {
            assert!(SWEEP_STOLEN >= desert_core::trampoline::MIN_STOLEN);
            assert!(SWEEP_STOLEN <= desert_core::trampoline::MAX_STOLEN);
        }
        assert_eq!(SWEEP_PROLOGUE.len(), SWEEP_STOLEN);
    }
}
