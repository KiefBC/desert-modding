//! Locate MSVC x64 vtables by RTTI type name, e.g. `.?AVClientActorManager@pa@@`.
//!
//! Layouts (64-bit MSVC):
//!   TypeDescriptor        { void* vftable; void* spare; char name[]; }   name at +0x10
//!   CompleteObjectLocator { u32 signature(=1); u32 offset; u32 cdOffset;
//!                           u32 pTypeDescriptor(RVA); u32 pClassDescriptor(RVA); u32 pSelf(RVA) }
//!   vtable[-1] is a pointer (absolute VA) to the COL.

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?)?.try_into().ok().map(u32::from_le_bytes)
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o.checked_add(8)?)?.try_into().ok().map(u64::from_le_bytes)
}

/// RVA of every TypeDescriptor whose mangled name equals `name` exactly.
pub fn find_type_descriptors(img: &[u8], name: &str) -> Vec<usize> {
    let needle = name.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + needle.len() < img.len() {
        let Some(rest) = img.get(i..) else { break };
        match rest.iter().position(|&b| b == b'.') {
            None => break,
            Some(p) => {
                let s = i + p;
                let after = s + needle.len();
                if s >= 0x10 && img.get(s..after) == Some(needle) && img.get(after) == Some(&0) {
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
        if u32_at(img, o) == Some(1)
            && u32_at(img, o + 12).is_some_and(|v| v as usize == td_rva)
            && u32_at(img, o + 20).is_some_and(|v| v as usize == o)
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
    // `wrapping_add` only to keep a debug build from panicking on a nonsense
    // base: a real image base plus an in-image RVA never comes near u64::MAX,
    // and the release build (the one that ships) wraps here either way.
    let want = image_base.wrapping_add(col_rva as u64);
    let mut out = Vec::new();
    let mut o = 0;
    while o + 8 <= img.len() {
        if u64_at(img, o) == Some(want) {
            out.push(image_base.wrapping_add(o as u64).wrapping_add(8));
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
