//! Minimal PE32+ header reader. Works on the raw file or on a mapped image.

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o.checked_add(2)?)?.try_into().ok().map(u16::from_le_bytes)
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?)?.try_into().ok().map(u32::from_le_bytes)
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o.checked_add(8)?)?.try_into().ok().map(u64::from_le_bytes)
}

#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub virtual_address: u32,
    pub virtual_size: u32,
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

impl Section {
    pub const MEM_EXECUTE: u32 = 0x2000_0000;
    pub const MEM_WRITE: u32 = 0x8000_0000;
    /// Writable, non-executable data: where a game keeps its globals.
    pub fn is_data(&self) -> bool {
        self.characteristics & (Self::MEM_WRITE | Self::MEM_EXECUTE) == Self::MEM_WRITE
    }
}

#[derive(Debug, Clone)]
pub struct Headers {
    pub image_base: u64,
    pub size_of_image: u32,
    pub sections: Vec<Section>,
}

/// Parse the headers from the first bytes of a PE32+ file or image.
pub fn parse(b: &[u8]) -> Option<Headers> {
    if u16_at(b, 0)? != 0x5A4D {
        return None;
    }
    let pe = u32_at(b, 0x3C)? as usize;
    if u32_at(b, pe)? != 0x0000_4550 {
        return None;
    }
    let nsec = u16_at(b, pe + 6)? as usize;
    let opt_size = u16_at(b, pe + 20)? as usize;
    let opt = pe + 24;
    if u16_at(b, opt)? != 0x20B {
        return None; // not PE32+
    }
    let image_base = u64_at(b, opt + 24)?;
    let size_of_image = u32_at(b, opt + 56)?;
    let mut sections = Vec::with_capacity(nsec);
    let mut s = opt + opt_size;
    for _ in 0..nsec {
        let name = b.get(s..s + 8)?;
        let name = String::from_utf8_lossy(name).trim_end_matches('\0').to_string();
        sections.push(Section {
            name,
            virtual_size: u32_at(b, s + 8)?,
            virtual_address: u32_at(b, s + 12)?,
            raw_size: u32_at(b, s + 16)?,
            raw_offset: u32_at(b, s + 20)?,
            characteristics: u32_at(b, s + 36)?,
        });
        s += 40;
    }
    Some(Headers { image_base, size_of_image, sections })
}

/// Lay a raw PE file out the way the loader would (sections at their RVAs),
/// so the same scanners work on a file and on the live image.
pub fn file_to_image(file: &[u8]) -> Option<Vec<u8>> {
    let h = parse(file)?;
    let mut img = vec![0u8; h.size_of_image as usize];
    let hdr_len = h
        .sections
        .iter()
        .map(|s| s.raw_offset as usize)
        .filter(|&o| o > 0)
        .min()
        .unwrap_or(0)
        .min(file.len())
        .min(img.len());
    if let (Some(dst), Some(src)) = (img.get_mut(..hdr_len), file.get(..hdr_len)) {
        dst.copy_from_slice(src);
    }
    for s in &h.sections {
        let take = s.raw_size.min(s.virtual_size.max(s.raw_size)) as usize;
        let start = (s.raw_offset as usize).min(file.len());
        let end = start.saturating_add(take).min(file.len());
        let dst = s.virtual_address as usize;
        let n = (end - start).min(img.len().saturating_sub(dst));
        if let (Some(d), Some(sr)) = (img.get_mut(dst..dst + n), file.get(start..start + n)) {
            d.copy_from_slice(sr);
        }
    }
    Some(img)
}
