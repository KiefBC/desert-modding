//! A minimal PE32+ builder for the `retarget` tests.
//!
//! Three sections, `.text`, `.rdata` and `.pdata`, at fixed RVAs, and data
//! directory 3 pointing at the `RUNTIME_FUNCTION` entries in `.pdata`. The
//! raw offsets are deliberately NOT the RVAs (headers take 0x400 of the file,
//! sections are packed after them), so a test can tell an `f:` file offset
//! from an RVA. Nothing here is a real program; the bytes a test plants are
//! the only code there is, and everything else is `int3`.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

pub const BASE: u64 = 0x1_4000_0000;
pub const TEXT: u32 = 0x1000;
pub const TEXT_LEN: usize = 0x2000;
pub const RDATA: u32 = 0x3000;
pub const RDATA_LEN: usize = 0x1000;
pub const PDATA: u32 = 0x4000;
pub const PDATA_LEN: usize = 0x1000;
pub const SIZE_OF_IMAGE: u32 = 0x5000;
pub const TIMESTAMP: u32 = 0x6AE5_8024;
/// Where `.text` starts in the file; an RVA in `.text` is this much further
/// from its file offset than zero.
pub const HEADERS: u32 = 0x400;
/// A plain version-1 unwind info every planted function shares.
pub const UNWIND: u32 = RDATA + 0xF00;

pub struct SynthPe {
    pub text: Vec<u8>,
    pub rdata: Vec<u8>,
    /// `(begin, end)` RVAs, written as `RUNTIME_FUNCTION`s in this order.
    pub functions: Vec<(u32, u32)>,
}

impl Default for SynthPe {
    fn default() -> Self {
        SynthPe { text: vec![0xCC; TEXT_LEN], rdata: vec![0; RDATA_LEN], functions: Vec::new() }
    }
}

fn put16(d: &mut [u8], at: usize, v: u16) {
    d[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(d: &mut [u8], at: usize, v: u32) {
    d[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(d: &mut [u8], at: usize, v: u64) {
    d[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

impl SynthPe {
    /// The file offset of an RVA in one of the three sections.
    pub fn file_offset(rva: u32) -> u32 {
        rva - TEXT + HEADERS
    }

    /// Plant bytes at an RVA in `.text`.
    pub fn code(&mut self, rva: u32, bytes: &[u8]) -> &mut Self {
        let at = (rva - TEXT) as usize;
        self.text[at..at + bytes.len()].copy_from_slice(bytes);
        self
    }

    /// Plant bytes at an RVA in `.rdata`.
    pub fn data(&mut self, rva: u32, bytes: &[u8]) -> &mut Self {
        let at = (rva - RDATA) as usize;
        self.rdata[at..at + bytes.len()].copy_from_slice(bytes);
        self
    }

    /// Record `[begin, end)` as a function in the exception directory.
    pub fn function(&mut self, begin: u32, end: u32) -> &mut Self {
        self.functions.push((begin, end));
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let file_len = HEADERS as usize + TEXT_LEN + RDATA_LEN + PDATA_LEN;
        let mut d = vec![0u8; file_len];
        d[..2].copy_from_slice(b"MZ");
        let pe = 0x80usize;
        put32(&mut d, 0x3c, pe as u32);
        d[pe..pe + 4].copy_from_slice(b"PE\0\0");
        put16(&mut d, pe + 4, 0x8664);
        put16(&mut d, pe + 6, 3);
        put32(&mut d, pe + 8, TIMESTAMP);
        put16(&mut d, pe + 20, 0xF0);
        put16(&mut d, pe + 22, 0x22);
        let opt = pe + 24;
        put16(&mut d, opt, 0x20b);
        put32(&mut d, opt + 16, TEXT); // entry point
        put64(&mut d, opt + 24, BASE);
        put32(&mut d, opt + 32, 0x1000); // section alignment
        put32(&mut d, opt + 36, 0x200); // file alignment
        put16(&mut d, opt + 40, 6); // OS version
        put16(&mut d, opt + 48, 6); // subsystem version
        put32(&mut d, opt + 56, SIZE_OF_IMAGE);
        put32(&mut d, opt + 60, HEADERS);
        put16(&mut d, opt + 68, 3); // console
        put32(&mut d, opt + 108, 16);
        let dir3 = opt + 112 + 3 * 8;
        put32(&mut d, dir3, PDATA);
        put32(&mut d, dir3 + 4, (self.functions.len() * 12) as u32);

        let mut sec = opt + 0xF0;
        for (name, rva, len, flags) in [
            (b".text\0\0\0", TEXT, TEXT_LEN, 0x6000_0020u32),
            (b".rdata\0\0", RDATA, RDATA_LEN, 0x4000_0040),
            (b".pdata\0\0", PDATA, PDATA_LEN, 0x4000_0040),
        ] {
            d[sec..sec + 8].copy_from_slice(name);
            put32(&mut d, sec + 8, len as u32);
            put32(&mut d, sec + 12, rva);
            put32(&mut d, sec + 16, len as u32);
            put32(&mut d, sec + 20, Self::file_offset(rva));
            put32(&mut d, sec + 36, flags);
            sec += 40;
        }

        let t = Self::file_offset(TEXT) as usize;
        d[t..t + TEXT_LEN].copy_from_slice(&self.text);
        let r = Self::file_offset(RDATA) as usize;
        d[r..r + RDATA_LEN].copy_from_slice(&self.rdata);
        let u = Self::file_offset(UNWIND) as usize;
        d[u..u + 4].copy_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        let p = Self::file_offset(PDATA) as usize;
        for (i, &(b, e)) in self.functions.iter().enumerate() {
            put32(&mut d, p + i * 12, b);
            put32(&mut d, p + i * 12 + 4, e);
            put32(&mut d, p + i * 12 + 8, UNWIND);
        }
        d
    }

    /// Write the image to `dir/name` and return the path.
    pub fn write(&self, dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, self.build()).unwrap();
        path
    }
}

/// The rel32 an instruction ending at `end` needs to reach `target`.
pub fn rel32(end: u32, target: u32) -> [u8; 4] {
    (target as i64 - end as i64).to_le_bytes()[..4].try_into().unwrap()
}
