//! Inline trampoline hooks, the same shape as the reference mod
//! (docs/cdloot-internals.md section 4).
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

use std::sync::Mutex;

use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    VirtualAlloc, VirtualProtect, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::safe;

/// Callback shape: the hooked function's first four integer arguments.
pub type Callback = unsafe extern "system" fn(usize, usize, usize, usize);

pub use crate::trampoline::{patch_bytes, stub_bytes, MAX_STOLEN, MIN_STOLEN};

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

/// Install an inline hook at `target`, stealing `stolen` bytes.
///
/// # Safety
/// `target` must be the start of a function whose first `stolen` bytes are
/// whole, position-independent instructions. Best done before the game
/// reaches its main loop, so no thread is executing the prologue while it is
/// being rewritten (the reference mod patches at load time for the same reason).
pub unsafe fn install(target: usize, stolen: usize, callback: Callback) -> Result<Hook, String> {
    if !(MIN_STOLEN..=MAX_STOLEN).contains(&stolen) {
        return Err(format!("stolen byte count {stolen} outside {MIN_STOLEN}..={MAX_STOLEN}"));
    }
    let mut original = vec![0u8; stolen];
    if !safe::read_into(target, &mut original) {
        return Err(format!("target 0x{target:X} not readable"));
    }
    if original[..2] == [0x48, 0xB8] {
        return Err(format!("target 0x{target:X} already starts with mov rax,imm64: hooked by someone else?"));
    }
    let stub = alloc_stub()?;
    let stub_code = stub_bytes(target, &original, callback as usize);
    // The stub page is ours (RWX), so a direct copy is fine.
    core::ptr::copy_nonoverlapping(stub_code.as_ptr(), stub as *mut u8, stub_code.len());

    let patch = patch_bytes(stub, stolen);
    let mut old = 0u32;
    if VirtualProtect(target as *const _, stolen, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
        return Err(format!("VirtualProtect(0x{target:X}) failed"));
    }
    core::ptr::copy_nonoverlapping(patch.as_ptr(), target as *mut u8, stolen);
    let mut ignored = 0u32;
    VirtualProtect(target as *const _, stolen, old, &mut ignored);
    FlushInstructionCache(GetCurrentProcess(), target as *const _, stolen);
    Ok(Hook { target, stub, stolen, original })
}

pub use crate::trampoline::hex;
