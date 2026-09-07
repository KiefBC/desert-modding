//! Guarded reads of arbitrary game memory.
//!
//! Every read goes through `ReadProcessMemory` on our own process. Unlike a
//! direct dereference guarded by `VirtualQuery`, this cannot fault: if any byte
//! of the range is unmapped at the moment of the copy, the call fails and we
//! return `None`. That closes the race that killed the game during loading,
//! when heap pages vanish between a check and the read. No caching: the answer
//! is only valid for the instant of the call.

use std::ffi::c_void;

use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const USER_MAX: usize = 0x7FFF_FFFF_FFFF;

/// Copy `buf.len()` bytes from `addr`. All-or-nothing.
pub fn read_into(addr: usize, buf: &mut [u8]) -> bool {
    if buf.is_empty() {
        return true;
    }
    if addr < 0x10000 || addr.checked_add(buf.len()).is_none_or(|e| e > USER_MAX) {
        return false;
    }
    let mut got: usize = 0;
    // SAFETY: the pseudo-handle GetCurrentProcess returns always names this
    // process, cannot fail and must not be closed. `buf` is a live `&mut [u8]`,
    // so its pointer and `buf.len()` describe exactly the bytes we own and are
    // allowed to overwrite; `addr` was range-checked above and is only ever
    // dereferenced by the kernel, which reports an unmapped page as a failed
    // call instead of faulting in our thread.
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            addr as *const c_void,
            buf.as_mut_ptr() as *mut c_void,
            buf.len(),
            &mut got,
        )
    };
    ok != 0 && got == buf.len()
}

/// True if `[addr, addr+len)` can be read right now.
pub fn readable(addr: usize, len: usize) -> bool {
    if len == 0 {
        return (0x10000..USER_MAX).contains(&addr);
    }
    if addr < 0x10000 || addr.checked_add(len).is_none_or(|e| e > USER_MAX) {
        return false;
    }
    // Probe page by page so a huge range does not need a huge buffer.
    let mut probe = [0u8; 8];
    let mut cur = addr;
    let end = addr + len;
    loop {
        let want = end.saturating_sub(cur).min(probe.len());
        let Some(chunk) = probe.get_mut(..want) else { return false };
        if !read_into(cur, chunk) {
            return false;
        }
        let next_page = (cur & !0xFFF) + 0x1000;
        if next_page >= end {
            // last partial page: check its tail too
            if end - 8 > cur && !read_into(end - 8, &mut probe) {
                return false;
            }
            return true;
        }
        cur = next_page;
    }
}

pub fn read<T: Copy>(addr: usize) -> Option<T> {
    let mut v = core::mem::MaybeUninit::<T>::uninit();
    // SAFETY: `v` is a live local, so `v.as_mut_ptr()` is non-null and aligned
    // for T and therefore for u8, and it owns exactly `size_of::<T>()` bytes.
    // The slice borrows `v` for less than its scope and is only ever written
    // through (`read_into` writes it whole or not at all), never read, so no
    // uninitialised byte is observed through it.
    let buf = unsafe {
        core::slice::from_raw_parts_mut(v.as_mut_ptr().cast::<u8>(), core::mem::size_of::<T>())
    };
    if !read_into(addr, buf) {
        return None;
    }
    // SAFETY: `read_into` returned true, which it only does after copying
    // exactly `size_of::<T>()` bytes over every byte of `v`, so `v` is fully
    // initialised. This crate instantiates T only with integers and
    // pointer-sized words read out of the game, for which every bit pattern is
    // a valid value.
    Some(unsafe { v.assume_init() })
}

pub fn read_ptr(addr: usize) -> Option<usize> {
    read::<usize>(addr).filter(|&p| p != 0)
}

/// Read a NUL-terminated ASCII string of at most `max` bytes.
pub fn read_cstr(addr: usize, max: usize) -> Option<String> {
    let mut buf = vec![0u8; max];
    // Whole-range copy first; if that crosses into an unmapped page, fall back
    // to a byte-by-byte read that stops at the first failure.
    let n = if read_into(addr, &mut buf) {
        max
    } else {
        let mut n = 0;
        let mut b = [0u8; 1];
        while n < max && addr.checked_add(n).is_some_and(|a| read_into(a, &mut b)) {
            let Some(slot) = buf.get_mut(n) else { break };
            *slot = b[0];
            n += 1;
            if b[0] == 0 {
                break;
            }
        }
        n
    };
    let mut out = Vec::new();
    for &b in buf.iter().take(n) {
        if b == 0 {
            break;
        }
        if !(0x20..0x7F).contains(&b) {
            return None;
        }
        out.push(b);
    }
    if out.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// Copy `buf` to `addr` in our own process through `WriteProcessMemory`, which
/// fails instead of faulting if the page is gone. Used only for objects the
/// game just handed us (a freshly allocated event); never for patching code.
pub fn write_into(addr: usize, buf: &[u8]) -> bool {
    use windows_sys::Win32::System::Diagnostics::Debug::WriteProcessMemory;
    if buf.is_empty() {
        return true;
    }
    if addr < 0x10000 || addr.checked_add(buf.len()).is_none_or(|e| e > USER_MAX) {
        return false;
    }
    let mut done: usize = 0;
    // SAFETY: as in `read_into` — the GetCurrentProcess pseudo-handle names this
    // process and needs no close, `buf` is a live `&[u8]` so its pointer and
    // length describe exactly the bytes the kernel may read, and `addr` was
    // range-checked above and is touched only by the kernel, which fails the
    // call rather than faulting if the destination page is gone.
    let ok = unsafe {
        WriteProcessMemory(
            GetCurrentProcess(),
            addr as *const c_void,
            buf.as_ptr() as *const c_void,
            buf.len(),
            &mut done,
        )
    };
    ok != 0 && done == buf.len()
}

pub fn write<T: Copy>(addr: usize, v: T) -> bool {
    // SAFETY: `v` is an initialised local of type T, so the pointer is non-null,
    // aligned and owns exactly `size_of::<T>()` bytes; the slice borrows it for
    // the `write_into` call only, well inside `v`'s scope. T: Copy has no drop
    // glue, and the plain-data types this is called with have no padding.
    let buf = unsafe {
        core::slice::from_raw_parts((&raw const v).cast::<u8>(), core::mem::size_of::<T>())
    };
    write_into(addr, buf)
}
