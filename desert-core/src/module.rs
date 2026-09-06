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
        let base = unsafe { GetModuleHandleW(core::ptr::null()) } as usize;
        if base == 0 {
            return None;
        }
        // Headers are always mapped; 0x1000 covers every real-world header.
        let hdr = unsafe { core::slice::from_raw_parts(base as *const u8, 0x1000) };
        let headers = pe::parse(hdr)?;
        Some(MainModule { base, size: headers.size_of_image as usize, headers })
    }

    /// The mapped image. An EXE image is fully committed and readable
    /// (code, data, and zero-fill), so a plain slice is safe to read.
    pub fn bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.base as *const u8, self.size) }
    }

    pub fn contains(&self, va: usize) -> bool {
        va >= self.base && va < self.base + self.size
    }

    pub fn rva(&self, va: usize) -> usize {
        va.wrapping_sub(self.base)
    }
}
