//! Locate MSVC x64 vtables by RTTI type name, e.g. `.?AVClientActorManager@pa@@`.
//!
//! Layouts (64-bit MSVC):
//!   TypeDescriptor        { void* vftable; void* spare; char name[]; }   name at +0x10
//!   CompleteObjectLocator { u32 signature(=1); u32 offset; u32 cdOffset;
//!                           u32 pTypeDescriptor(RVA); u32 pClassDescriptor(RVA); u32 pSelf(RVA) }
//!   vtable[-1] is a pointer (absolute VA) to the COL.

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// RVA of every TypeDescriptor whose mangled name equals `name` exactly.
pub fn find_type_descriptors(img: &[u8], name: &str) -> Vec<usize> {
    let needle = name.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() < img.len() {
        match img[i..].iter().position(|&b| b == b'.') {
            None => break,
            Some(p) => {
                let s = i + p;
                if s >= 0x10
                    && img[s..].starts_with(needle)
                    && img.get(s + needle.len()) == Some(&0)
                {
                    out.push(s - 0x10);
                }
                i = s + 1;
            }
        }
    }
    out
}

/// RVA of every CompleteObjectLocator that references `td_rva`, validated by
/// its self-RVA field.
pub fn find_object_locators(img: &[u8], td_rva: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut o = 0;
    while o + 24 <= img.len() {
        if u32_at(img, o) == 1
            && u32_at(img, o + 12) as usize == td_rva
            && u32_at(img, o + 20) as usize == o
        {
            out.push(o);
        }
        o += 4;
    }
    out
}

/// Absolute VAs of every vtable whose meta pointer targets the COL at `col_rva`.
/// `image_base` must be the base the pointers in `img` are relative to: the
/// live base for a mapped image, the preferred base for a raw file.
pub fn find_vtables(img: &[u8], image_base: u64, col_rva: usize) -> Vec<u64> {
    let want = image_base + col_rva as u64;
    let mut out = Vec::new();
    let mut o = 0;
    while o + 8 <= img.len() {
        if u64_at(img, o) == want {
            out.push(image_base + o as u64 + 8);
        }
        o += 8;
    }
    out
}

/// Convenience: all vtables for a mangled class name. Usually exactly one.
pub fn vtables_for_class(img: &[u8], image_base: u64, name: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for td in find_type_descriptors(img, name) {
        for col in find_object_locators(img, td) {
            out.extend(find_vtables(img, image_base, col));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pe;

    #[test]
    fn finds_std_exception_vtable_in_cdloot() {
        let f = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/source-mod/CDLoot.asi")).unwrap();
        let h = pe::parse(&f).unwrap();
        let img = pe::file_to_image(&f).unwrap();
        let tds = find_type_descriptors(&img, ".?AVexception@std@@");
        assert_eq!(tds.len(), 1, "type descriptor");
        let cols = find_object_locators(&img, tds[0]);
        assert!(!cols.is_empty(), "object locator");
        let vt = vtables_for_class(&img, h.image_base, ".?AVexception@std@@");
        assert_eq!(vt.len(), 1, "vtable: {vt:x?}");
        // The vtable lives in .rdata.
        let rdata = &h.sections[1];
        let rva = (vt[0] - h.image_base) as u32;
        assert!(rva >= rdata.virtual_address && rva < rdata.virtual_address + rdata.virtual_size);
    }
}
