//! Inline trampoline hooks, the same shape as the reference mod.
//!
//! One RWX page holds the stubs (0xA0 bytes each). A stub saves the four
//! argument registers, calls our callback with them, restores them, runs the
//! stolen prologue bytes and jumps back to the original function just past
//! the patch. The original function always runs; the callback runs first.
//!
//! ```text
//! stub:  sub rsp,0x48; mov [rsp+0x20..0x38],rcx/rdx/r8/r9
//!        mov rax,<callback>; call rax
//!        mov rcx/rdx/r8/r9,[rsp+0x20..0x38]; add rsp,0x48
//!        <stolen bytes>; mov rax,<orig+n>; jmp rax
//! orig:  mov rax,<stub>; jmp rax; nop*(n-12)
//! ```
//!
//! The stolen bytes must be whole instructions with no RIP-relative operand
//! or relative branch; the caller is responsible for choosing `n` from a
//! disassembly (for `area_sweep` on build 25116796 the first 15 bytes are
//! three `mov [rsp+x],reg` spills).
//!
//! [`install_raw`] is the same machinery with the stub bytes supplied by the
//! caller, for a hook that replaces an instruction instead of observing a
//! prologue: Desert Gatherer's catch-count hook patches a `mov r8d,1` in the
//! middle of a function and lets its callback decide the value
//! ([`count_hook_stub`], `docs/reference-internals.md` section 17).

use std::sync::Mutex;

use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::safe;

/// Callback shape: the hooked function's first four integer arguments.
pub type Callback = unsafe extern "system" fn(usize, usize, usize, usize);

/// Callback shape for a [`count_hook_stub`] stub: one pointer in, the `u32`
/// the game will use in `r8d` out.
pub type CountCallback = unsafe extern "system" fn(usize) -> u32;

pub use crate::trampoline::{count_hook_stub, patch_bytes, stub_bytes, MAX_STOLEN, MIN_STOLEN};

const STUB_SIZE: usize = 0xA0;
const PAGE: usize = 0x1000;

static STUB_PAGE: Mutex<(usize, usize)> = Mutex::new((0, 0)); // (base, used)

#[derive(Debug, Clone)]
pub struct Hook {
    pub target: usize,
    pub stub: usize,
    pub stolen: usize,
    pub original: Vec<u8>,
}

fn alloc_stub() -> Result<usize, String> {
    let mut g = STUB_PAGE.lock().unwrap_or_else(|e| e.into_inner());
    if g.0 == 0 || g.1 + STUB_SIZE > PAGE {
        // SAFETY: VirtualAlloc with a null base has no precondition to uphold —
        // the kernel picks an address, so no existing mapping of ours can be
        // disturbed — and it reports failure by returning null, which is checked
        // on the next line before the value is ever used as a pointer.
        let p = unsafe {
            VirtualAlloc(core::ptr::null(), PAGE, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE)
        } as usize;
        if p == 0 {
            return Err("VirtualAlloc for the stub page failed".into());
        }
        *g = (p, 0);
    }
    let stub = g.0 + g.1;
    g.1 += STUB_SIZE;
    Ok(stub)
}

/// Read and vet the `stolen` bytes about to be overwritten at `target`.
///
/// The two checks are the same for every hook shape: a count outside the
/// range the patch needs, and a target that already begins with our own
/// `mov rax,imm64` (someone else's hook, or ours installed twice).
fn original_bytes(target: usize, stolen: usize) -> Result<Vec<u8>, String> {
    if !(MIN_STOLEN..=MAX_STOLEN).contains(&stolen) {
        return Err(format!("stolen byte count {stolen} outside {MIN_STOLEN}..={MAX_STOLEN}"));
    }
    let mut original = vec![0u8; stolen];
    if !safe::read_into(target, &mut original) {
        return Err(format!("target 0x{target:X} not readable"));
    }
    if original.starts_with(&[0x48, 0xB8]) {
        return Err(format!("target 0x{target:X} already starts with mov rax,imm64: hooked by someone else?"));
    }
    Ok(original)
}

/// Install an inline hook at `target`, stealing `stolen` bytes.
///
/// # Safety
/// `target` must be the start of a function whose first `stolen` bytes are
/// whole, position-independent instructions. Best done before the game
/// reaches its main loop, so no thread is executing the prologue while it is
/// being rewritten (the reference mod patches at load time for the same reason).
pub unsafe fn install(target: usize, stolen: usize, callback: Callback) -> Result<Hook, String> {
    let original = original_bytes(target, stolen)?;
    let stub_code = stub_bytes(target, &original, callback as usize);
    // SAFETY: forwarded from this function's own `# Safety` contract - the
    // caller has established that the `stolen` bytes at `target` are whole,
    // position-independent instructions no thread is executing. `stub_bytes`
    // replays exactly those bytes and jumps back to `target + stolen`, so the
    // original function still runs in full.
    unsafe { install_raw(target, stolen, &stub_code) }
}

/// Install an inline hook whose stub the caller built.
///
/// The RWX allocation, the prologue patch and the instruction-cache flush are
/// the same as [`install`]; the only difference is that the stub bytes come
/// from the caller instead of from [`stub_bytes`]. That is what lets a hook
/// *replace* an instruction rather than observe a prologue - see
/// [`count_hook_stub`], whose stub feeds the callback's return value into
/// `r8d` instead of replaying the `mov r8d,<imm>` it was installed over.
///
/// # Safety
/// As [`install`], plus: `stub_code` must be executable machine code that
/// leaves the process in a state the game can continue from and ends by
/// transferring control back into `target`'s function. Nothing here checks
/// what those bytes do.
pub unsafe fn install_raw(target: usize, stolen: usize, stub_code: &[u8]) -> Result<Hook, String> {
    let original = original_bytes(target, stolen)?;
    if stub_code.len() > STUB_SIZE {
        return Err(format!("stub of {} bytes does not fit in {STUB_SIZE}", stub_code.len()));
    }
    let stub = alloc_stub()?;
    // SAFETY: `alloc_stub` handed us STUB_SIZE (0xA0) bytes of a page it
    // VirtualAlloc'd PAGE_EXECUTE_READWRITE and never hands out twice, and
    // `stub_code.len()` was just checked against STUB_SIZE, so the whole copy
    // lands inside our own writable stub. `stub_code` is the caller's slice
    // and the destination is a page we just allocated, so they cannot overlap.
    unsafe {
        core::ptr::copy_nonoverlapping(stub_code.as_ptr(), stub as *mut u8, stub_code.len());
    }

    let patch = patch_bytes(stub, stolen);
    let mut old = 0u32;
    // SAFETY: `safe::read_into` read `stolen` bytes from `target` above, so that
    // range is inside a committed mapping of this process; VirtualProtect only
    // changes the protection of the pages spanning it and writes the previous
    // flags through `&mut old`, a live local.
    if unsafe { VirtualProtect(target as *const _, stolen, PAGE_EXECUTE_READWRITE, &mut old) } == 0
    {
        return Err(format!("VirtualProtect(0x{target:X}) failed"));
    }
    // SAFETY: the VirtualProtect above succeeded, so `stolen` bytes at `target`
    // are writable; `patch_bytes` resized `patch` to exactly `stolen` bytes, and
    // it is a fresh local Vec that cannot alias the game's code. That
    // overwriting those bytes is sound at all is the caller's `# Safety`
    // contract: they are whole position-independent instructions and no thread
    // is executing them.
    unsafe {
        core::ptr::copy_nonoverlapping(patch.as_ptr(), target as *mut u8, stolen);
    }
    let mut ignored = 0u32;
    // SAFETY: the same range as the call that returned `old`, put back the way
    // we found it; `ignored` is a live local for the out parameter.
    unsafe {
        VirtualProtect(target as *const _, stolen, old, &mut ignored);
    }
    // SAFETY: the GetCurrentProcess pseudo-handle always names this process,
    // cannot fail and must not be closed. `target..target + stolen` is the range
    // just rewritten and still mapped, and flushing it is what makes the new
    // bytes visible on a core that has the old ones in its instruction cache.
    unsafe {
        FlushInstructionCache(GetCurrentProcess(), target as *const _, stolen);
    }
    Ok(Hook { target, stub, stolen, original })
}

pub use crate::trampoline::hex;
