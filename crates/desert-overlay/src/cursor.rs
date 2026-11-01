//! Two inline hooks on user32 that stop the game pinning the mouse pointer
//! while the menu is open.
//!
//! Crimson Desert drives its camera from the mouse the way every third-person
//! game does: each frame it calls `ClipCursor` to confine the pointer (in
//! camera control, to a single point) and `SetCursorPos` to re-centre it. Both
//! come straight out of USER32.dll - they are in the exe's import table - so
//! there is no game-side flag to switch off and no address to find.
//!
//! That is invisible until an overlay wants the pointer. hudhook feeds imgui's
//! mouse position from two sources at once (`imgui_wnd_proc_impl`, in its
//! `renderer/input.rs`): raw-input deltas from `WM_INPUT`, which it *adds* to
//! `io.mouse_pos`, and the absolute coordinates in `WM_MOUSEMOVE`, which it
//! *assigns*. When the cursor is pinned every `WM_MOUSEMOVE` carries the same
//! point, so each raw-input delta is undone before it can be drawn and the
//! pointer sits still on screen. Releasing the clip once when the menu opens
//! is not enough: the game re-applies it on the very next frame.
//!
//! It also explains the workaround that looked like a fix - tapping the
//! Windows key and clicking back into the game. The round trip costs the game
//! its focus, and on the way back its mouse input is being swallowed by
//! [`hudhook::MessageFilter::InputAll`], so it never resumes pinning until the
//! menu closes and the filter lifts.
//!
//! ReShade solves exactly this in its `input.cpp`, by hooking `ClipCursor` and
//! `SetCursorPos` and neutering them while its overlay is blocking mouse
//! input, which is why its menu behaves in this game and ours did not. This is
//! the same trick: while the menu is open `ClipCursor` is forced to a null
//! rectangle (the documented "no confinement") and `SetCursorPos` reports
//! success without moving anything. With the menu closed both calls go
//! straight through the MinHook trampoline and the game's camera is exactly as
//! it was.
//!
//! The hooks are installed once and never removed. An .asi is never unloaded,
//! so there is no point at which the detours could be retired safely, and a
//! closed menu already costs one relaxed atomic load per call.
//!
//! ReShade may well have hooked the same two exports first. MinHook does not
//! mind: it chains on top of whatever jmp is already at the entry point, so
//! ours ends up the outermost of the two and its trampoline calls into
//! ReShade's. Both then get their say, in that order, which is the behaviour
//! either one alone would produce.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use hudhook::mh::{MhHook, MH_ApplyQueued};
use windows_sys::Win32::Foundation::{BOOL, FALSE, RECT, TRUE};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

/// `ClipCursor`'s real signature. A null rectangle releases the confinement.
type ClipCursorFn = unsafe extern "system" fn(*const RECT) -> BOOL;

/// `SetCursorPos`'s real signature, in screen coordinates.
type SetCursorPosFn = unsafe extern "system" fn(i32, i32) -> BOOL;

/// `user32.dll`, NUL-terminated UTF-16 for `GetModuleHandleW`. Written out
/// because `windows-sys` ships no string literal macro.
const USER32_W: &[u16] = &[
    b'u' as u16,
    b's' as u16,
    b'e' as u16,
    b'r' as u16,
    b'3' as u16,
    b'2' as u16,
    b'.' as u16,
    b'd' as u16,
    b'l' as u16,
    b'l' as u16,
    0,
];

/// Set by [`ui`](crate::ui) on every change of the menu's visibility. Read by
/// both detours on game threads, so it is an atomic; nothing is ordered
/// against it, and a frame's lag either way is invisible.
static MENU_OPEN: AtomicBool = AtomicBool::new(false);

/// Tell the detours whether the menu is on screen. Cheap enough to call every
/// time `visible` is assigned rather than only when it changes.
pub fn set_menu_open(open: bool) {
    MENU_OPEN.store(open, Ordering::Relaxed);
}

/// The two live hooks and the trampolines that reach the real exports.
struct Hooks {
    /// The `ClipCursor` hook. Kept for the life of the process: it is what
    /// `queue_enable` is called on, and MinHook owns the patch behind it.
    clip: MhHook,
    /// As above, for `SetCursorPos`.
    set_pos: MhHook,
    /// Trampoline to the real `user32!ClipCursor`.
    clip_orig: ClipCursorFn,
    /// Trampoline to the real `user32!SetCursorPos`.
    set_pos_orig: SetCursorPosFn,
}

// SAFETY: the two `MhHook`s hold raw pointers into user32's code and into
// MinHook's own trampoline pages, and the two function pointers are the same
// addresses typed. All four are immutable for the life of the process (the
// hooks are never disabled or removed), point at code rather than at data this
// crate owns, and are only ever read, so sharing them across the game's
// threads cannot race with anything.
unsafe impl Send for Hooks {}
// SAFETY: as above.
unsafe impl Sync for Hooks {}

/// Written once by [`install`], before either hook is enabled, so a detour can
/// never observe it empty in practice. Both detours still fail safe if it is.
static HOOKS: OnceLock<Hooks> = OnceLock::new();

/// Detour for `user32!ClipCursor`. With the menu open the requested rectangle
/// is thrown away and the real call is made with null, which releases whatever
/// confinement was in force; with the menu closed the caller's rectangle goes
/// through untouched.
unsafe extern "system" fn clip_cursor(rect: *const RECT) -> BOOL {
    let Some(hooks) = HOOKS.get() else {
        // Nothing to call through to. Report failure rather than pretending a
        // clip was set that is not there.
        return FALSE;
    };
    let rect = if MENU_OPEN.load(Ordering::Relaxed) { core::ptr::null() } else { rect };
    // SAFETY: `clip_orig` is MinHook's trampoline for `user32!ClipCursor`,
    // typed with that export's signature, and it stays valid for the life of
    // the process. `rect` is either our own null or the pointer the caller
    // passed in, which is that caller's to validate; we neither read nor
    // retain it.
    unsafe { (hooks.clip_orig)(rect) }
}

/// Detour for `user32!SetCursorPos`. With the menu open the move is swallowed
/// and reported as done, so the game's per-frame re-centring cannot drag the
/// pointer out from under the user.
unsafe extern "system" fn set_cursor_pos(x: i32, y: i32) -> BOOL {
    if MENU_OPEN.load(Ordering::Relaxed) {
        return TRUE;
    }
    let Some(hooks) = HOOKS.get() else {
        // Same answer as the open menu: claim success and move nothing. A
        // caller told the move failed may well try something worse.
        return TRUE;
    };
    // SAFETY: `set_pos_orig` is MinHook's trampoline for
    // `user32!SetCursorPos`, typed with that export's signature and valid for
    // the life of the process. Both arguments are plain integers.
    unsafe { (hooks.set_pos_orig)(x, y) }
}

/// Look one export up in an already-loaded module.
///
/// `name` must be NUL-terminated ASCII.
fn export(module: *mut c_void, name: &[u8]) -> Option<*mut c_void> {
    // SAFETY: `module` is a live HMODULE the caller obtained from
    // `GetModuleHandleW`, and `name` is a NUL-terminated byte string that
    // outlives the call. `GetProcAddress` only reads both.
    let proc = unsafe { GetProcAddress(module.cast(), name.as_ptr()) }?;
    Some(proc as *mut c_void)
}

/// Create and enable both hooks. Split out from [`install`] so the log lines
/// live in one place.
fn hook_user32() -> Result<(), String> {
    // SAFETY: `USER32_W` is a NUL-terminated UTF-16 literal that outlives the
    // call, and `GetModuleHandleW` only reads it. It takes no reference on the
    // module, which is what we want: user32 is loaded for the life of the
    // process and this handle is only used to resolve two exports.
    let user32 = unsafe { GetModuleHandleW(USER32_W.as_ptr()) };
    if user32.is_null() {
        return Err("user32.dll: not loaded".to_string());
    }
    let user32: *mut c_void = user32.cast();

    let clip_addr =
        export(user32, b"ClipCursor\0").ok_or_else(|| "user32!ClipCursor: not found".to_string())?;
    let set_pos_addr = export(user32, b"SetCursorPos\0")
        .ok_or_else(|| "user32!SetCursorPos: not found".to_string())?;

    // SAFETY: both addresses are exported entry points in user32's code, and
    // both detours have the matching Win32 signature and live in this module
    // for the life of the process. `MhHook::new` only creates the hook - the
    // patch is not live until the queued enable below - so the trampolines are
    // read and `HOOKS` is filled in before any game thread can reach a detour.
    // MinHook itself is already initialised: hudhook's `apply()` ran first.
    let (clip, set_pos) = unsafe {
        let clip = MhHook::new(clip_addr, clip_cursor as *mut c_void)
            .map_err(|s| format!("user32!ClipCursor: {s:?}"))?;
        let set_pos = MhHook::new(set_pos_addr, set_cursor_pos as *mut c_void)
            .map_err(|s| format!("user32!SetCursorPos: {s:?}"))?;
        (clip, set_pos)
    };

    // SAFETY: each trampoline is the address MinHook produced for that hook,
    // and each is transmuted to exactly the signature of the export it stands
    // in for - the same signature the detour above was declared with.
    let hooks = unsafe {
        Hooks {
            clip_orig: core::mem::transmute::<*mut c_void, ClipCursorFn>(clip.trampoline()),
            set_pos_orig: core::mem::transmute::<*mut c_void, SetCursorPosFn>(
                set_pos.trampoline(),
            ),
            clip,
            set_pos,
        }
    };
    let Ok(()) = HOOKS.set(hooks) else {
        return Err("user32!ClipCursor: already hooked".to_string());
    };
    let Some(hooks) = HOOKS.get() else {
        return Err("user32!ClipCursor: hooks vanished".to_string());
    };

    // SAFETY: both hooks were created by `MhHook::new` above and neither has
    // been enabled or removed, and `HOOKS` is filled in, so a detour that runs
    // the instant `MH_ApplyQueued` patches the entry point has its trampoline.
    unsafe {
        hooks.clip.queue_enable().map_err(|s| format!("user32!ClipCursor: {s:?}"))?;
        hooks.set_pos.queue_enable().map_err(|s| format!("user32!SetCursorPos: {s:?}"))?;
        MH_ApplyQueued().ok().map_err(|s| format!("user32 (applying the queued hooks): {s:?}"))?;
    }
    Ok(())
}

/// Install the two hooks, once, from the plugin's own thread.
///
/// Must be called **after** `Hudhook::builder()...apply()` has returned `Ok`:
/// the builder is what runs `MH_Initialize`, and `apply()` is what flushes
/// MinHook's enable queue, so calling this earlier would either fail outright
/// or leave these two hooks created but not enabled.
///
/// Failure is not fatal and is logged, not returned: the menu draws and works
/// exactly as it did before, with the pointer pinned while it is open.
pub fn install() {
    match hook_user32() {
        Ok(()) => crate::log!(
            "[cursor] ClipCursor and SetCursorPos are hooked; the game cannot pin the \
             pointer while the menu is open"
        ),
        Err(e) => crate::log!(
            "[cursor] WARN could not hook {e}; the pointer may not move while the menu is \
             open (Win key out and back frees it)"
        ),
    }
}
