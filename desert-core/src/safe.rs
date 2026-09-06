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
    if addr < 0x10000 || addr.checked_add(buf.len()).map_or(true, |e| e > USER_MAX) {
        return false;
    }
    let mut got: usize = 0;
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
        return addr >= 0x10000 && addr < USER_MAX;
    }
    if addr < 0x10000 || addr.checked_add(len).map_or(true, |e| e > USER_MAX) {
        return false;
    }
    // Probe page by page so a huge range does not need a huge buffer.
    let mut probe = [0u8; 8];
    let mut cur = addr;
    let end = addr + len;
    loop {
        if !read_into(cur, &mut probe[..(end - cur).min(8)]) {
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
    let buf = unsafe {
        core::slice::from_raw_parts_mut(v.as_mut_ptr() as *mut u8, core::mem::size_of::<T>())
    };
    if !read_into(addr, buf) {
        return None;
    }
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
        while n < max && read_into(addr + n, &mut b) {
            buf[n] = b[0];
            n += 1;
            if b[0] == 0 {
                break;
            }
        }
        n
    };
    let mut out = Vec::new();
    for &b in &buf[..n] {
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
    if addr < 0x10000 || addr.checked_add(buf.len()).map_or(true, |e| e > USER_MAX) {
        return false;
    }
    let mut done: usize = 0;
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
    let buf = unsafe {
        core::slice::from_raw_parts(&v as *const T as *const u8, core::mem::size_of::<T>())
    };
    write_into(addr, buf)
}
