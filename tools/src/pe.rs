//! Just enough PE to answer the three questions the binary tools ask:
//! where a section's bytes are, what RVA a file offset is, and what the image
//! looks like once the loader has laid it out.
//!
//! `sigscan` reports FILE OFFSETS and `xrefs` takes an RVA. They are not the
//! same number and converting between them is this module's whole job; see
//! `tools/README.md`, which warns about exactly that confusion.

use anyhow::{bail, Context, Result};

/// One entry of the PE section table.
#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub raw_size: u32,
    pub raw_offset: u32,
}

/// A parsed PE32+ image header, holding a borrow of the whole file.
#[derive(Debug, Clone)]
pub struct Pe<'a> {
    pub data: &'a [u8],
    pub image_base: u64,
    pub size_of_image: u32,
    pub sections: Vec<Section>,
}

fn u16_at(d: &[u8], off: usize) -> Result<u16> {
    let b = d
        .get(off..off + 2)
        .with_context(|| format!("truncated at {off:#x}"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(d: &[u8], off: usize) -> Result<u32> {
    let b = d
        .get(off..off + 4)
        .with_context(|| format!("truncated at {off:#x}"))?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(d: &[u8], off: usize) -> Result<u64> {
    let b = d
        .get(off..off + 8)
        .with_context(|| format!("truncated at {off:#x}"))?;
    Ok(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

impl<'a> Pe<'a> {
    /// Parse the headers of a PE32+ file held whole in memory.
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.get(..2) != Some(b"MZ") {
            bail!("not a PE file: no MZ signature");
        }
        let pe = u32_at(data, 0x3c)? as usize;
        if data.get(pe..pe + 4) != Some(b"PE\0\0") {
            bail!("not a PE file: no PE signature at {pe:#x}");
        }
        let nsections = u16_at(data, pe + 6)? as usize;
        let opt_size = u16_at(data, pe + 20)? as usize;
        let opt = pe + 24;
        let magic = u16_at(data, opt)?;
        if magic != 0x20b {
            bail!("not PE32+ (optional header magic {magic:#x})");
        }
        let size_of_image = u32_at(data, opt + 56)?;
        let image_base = u64_at(data, opt + 24)?;

        let table = opt + opt_size;
        let mut sections = Vec::with_capacity(nsections);
        for i in 0..nsections {
            let e = table + i * 40;
            let raw = data
                .get(e..e + 40)
                .with_context(|| format!("section table entry {i} runs past the file"))?;
            let name = String::from_utf8_lossy(&raw[..8])
                .trim_end_matches('\0')
                .to_string();
            sections.push(Section {
                name,
                virtual_size: u32_at(data, e + 8)?,
                virtual_address: u32_at(data, e + 12)?,
                raw_size: u32_at(data, e + 16)?,
                raw_offset: u32_at(data, e + 20)?,
            });
        }
        Ok(Pe {
            data,
            image_base,
            size_of_image,
            sections,
        })
    }

    /// The file offset holding the byte at `rva`, if any section covers it.
    ///
    /// The section is matched on its VIRTUAL size, but a hit past `raw_size` is
    /// not in the file at all - that is `.bss`-style zero fill - and comes back
    /// `None` rather than as an offset into the next section's bytes.
    pub fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        for s in &self.sections {
            let start = s.virtual_address;
            let end = start.checked_add(s.virtual_size.max(s.raw_size))?;
            if rva >= start && rva < end {
                let delta = rva - start;
                if delta >= s.raw_size {
                    return None;
                }
                return Some((s.raw_offset + delta) as usize);
            }
        }
        None
    }

    /// The RVA of a file offset, if it is inside a section's raw bytes.
    pub fn offset_to_rva(&self, off: usize) -> Option<u32> {
        let off = u32::try_from(off).ok()?;
        for s in &self.sections {
            if s.raw_size == 0 {
                continue;
            }
            if off >= s.raw_offset && off < s.raw_offset.checked_add(s.raw_size)? {
                return Some(s.virtual_address + (off - s.raw_offset));
            }
        }
        None
    }

    /// The file laid out the way the loader maps it: every section's raw bytes
    /// copied to its virtual address, everything else zero.
    ///
    /// This is what makes a disassembler's addresses line up with RVAs, and it
    /// is why `dis` can hand a flat buffer to a decoder and print RVAs.
    pub fn image(&self) -> Vec<u8> {
        let mut img = vec![0u8; self.size_of_image as usize];
        for s in &self.sections {
            let take = (s.raw_size as usize).min(self.data.len().saturating_sub(s.raw_offset as usize));
            let src = s.raw_offset as usize;
            let dst = s.virtual_address as usize;
            let take = take.min(img.len().saturating_sub(dst));
            if take == 0 {
                continue;
            }
            img[dst..dst + take].copy_from_slice(&self.data[src..src + take]);
        }
        img
    }
}

/// Every DLL named in a PE's import directory, lowercased and without `.dll`.
///
/// Capitalisation in an import table is not stable - one binary can carry both
/// `KERNEL32.dll` and `kernel32.dll` - so the names are normalised here rather
/// than at every call site. Sorted and deduplicated.
pub fn import_dlls(data: &[u8]) -> Result<Vec<String>> {
    use object::read::pe::PeFile64;
    use object::LittleEndian as LE;

    let file = PeFile64::parse(data).context("cannot parse as a 64-bit PE")?;
    let mut names: Vec<String> = Vec::new();
    if let Some(table) = file.import_table().context("cannot read the import table")? {
        let mut descriptors = table.descriptors().context("cannot walk the import descriptors")?;
        while let Some(desc) = descriptors.next().context("truncated import descriptor")? {
            let raw = table
                .name(desc.name.get(LE))
                .context("import descriptor names a string outside the image")?;
            let name = String::from_utf8_lossy(raw).to_ascii_lowercase();
            let name = name.strip_suffix(".dll").unwrap_or(&name).to_string();
            names.push(name);
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}
