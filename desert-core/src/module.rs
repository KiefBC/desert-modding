//! The game's main module as a byte slice, for scanning.

use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::pe::{self, Headers};

pub struct MainModule {
    pub base: usize,
    pub size: usize,
    pub headers: Headers,
}

impl MainModule {
    pub fn locate() -> Option<MainModule> {
        // SAFETY: GetModuleHandleW(null) asks for the handle of the process's
        // own exe. It borrows no memory of ours, returns null on failure (checked
        // on the next line), and the handle it returns is not reference-counted,
        // so there is nothing to release.
        let base = unsafe { GetModuleHandleW(core::ptr::null()) } as usize;
        if base == 0 {
            return None;
        }
        // Headers are always mapped; 0x1000 covers every real-world header.
        // SAFETY: `base` is non-null and is the load address of the main exe, so
        // the loader has mapped its headers there. Headers are mapped rounded up
        // to the section alignment, which is at least one 0x1000 page on x64, so
        // all 0x1000 bytes are readable, and the main module stays loaded for the
        // life of the process. Nothing writes them while `hdr` is alive.
        let hdr = unsafe { core::slice::from_raw_parts(base as *const u8, 0x1000) };
        let headers = pe::parse(hdr)?;
        Some(MainModule { base, size: headers.size_of_image as usize, headers })
    }

    /// The mapped image. An EXE image is fully committed and readable
    /// (code, data, and zero-fill), so a plain slice is safe to read.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `self.base` and `self.size` were taken from the module's own
        // PE headers in `locate`, so they are the exact range the loader
        // reserved and committed for the main exe — code, data and zero-fill
        // alike are readable and stay mapped for the life of the process. The
        // returned slice borrows `self`, and this crate never builds a
        // `MainModule` for a module that can be unloaded.
        unsafe { core::slice::from_raw_parts(self.base as *const u8, self.size) }
    }

    pub fn contains(&self, va: usize) -> bool {
        va >= self.base && va < self.base + self.size
    }

    pub fn rva(&self, va: usize) -> usize {
        va.wrapping_sub(self.base)
    }
}
