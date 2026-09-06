//! Byte construction for the inline trampoline (platform independent, unit
//! tested). See src/hook.rs for the layout and the install step.

pub const MIN_STOLEN: usize = 12;
pub const MAX_STOLEN: usize = 30;

/// Build the stub bytes for `target` with `stolen` original bytes.
pub fn stub_bytes(target: usize, stolen: &[u8], callback: usize) -> Vec<u8> {
    let mut s = Vec::with_capacity(0x48 + stolen.len());
    s.extend_from_slice(&[0x48, 0x83, 0xEC, 0x48]); // sub rsp,0x48
    s.extend_from_slice(&[0x48, 0x89, 0x4C, 0x24, 0x20]); // mov [rsp+0x20],rcx
    s.extend_from_slice(&[0x48, 0x89, 0x54, 0x24, 0x28]); // mov [rsp+0x28],rdx
    s.extend_from_slice(&[0x4C, 0x89, 0x44, 0x24, 0x30]); // mov [rsp+0x30],r8
    s.extend_from_slice(&[0x4C, 0x89, 0x4C, 0x24, 0x38]); // mov [rsp+0x38],r9
    s.extend_from_slice(&[0x48, 0xB8]); // mov rax,imm64
    s.extend_from_slice(&(callback as u64).to_le_bytes());
    s.extend_from_slice(&[0xFF, 0xD0]); // call rax
    s.extend_from_slice(&[0x48, 0x8B, 0x4C, 0x24, 0x20]); // mov rcx,[rsp+0x20]
    s.extend_from_slice(&[0x48, 0x8B, 0x54, 0x24, 0x28]); // mov rdx,[rsp+0x28]
    s.extend_from_slice(&[0x4C, 0x8B, 0x44, 0x24, 0x30]); // mov r8,[rsp+0x30]
    s.extend_from_slice(&[0x4C, 0x8B, 0x4C, 0x24, 0x38]); // mov r9,[rsp+0x38]
    s.extend_from_slice(&[0x48, 0x83, 0xC4, 0x48]); // add rsp,0x48
    debug_assert_eq!(s.len(), 0x3C);
    s.extend_from_slice(stolen);
    s.extend_from_slice(&[0x48, 0xB8]); // mov rax,imm64
    s.extend_from_slice(&((target + stolen.len()) as u64).to_le_bytes());
    s.extend_from_slice(&[0xFF, 0xE0]); // jmp rax
    s
}

/// The bytes written over the start of the original function.
pub fn patch_bytes(stub: usize, stolen: usize) -> Vec<u8> {
    let mut p = Vec::with_capacity(stolen);
    p.extend_from_slice(&[0x48, 0xB8]);
    p.extend_from_slice(&(stub as u64).to_le_bytes());
    p.extend_from_slice(&[0xFF, 0xE0]);
    p.resize(stolen, 0x90);
    p
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_matches_reference_layout() {
        let stolen = [0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7C, 0x24, 0x20];
        let s = stub_bytes(0x1000, &stolen, 0xDEAD_BEEF);
        assert_eq!(s.len(), 0x48 + 15);
        assert_eq!(&s[0..4], &[0x48, 0x83, 0xEC, 0x48]);
        assert_eq!(&s[0x18..0x1A], &[0x48, 0xB8]);
        assert_eq!(u64::from_le_bytes(s[0x1A..0x22].try_into().unwrap()), 0xDEAD_BEEF);
        assert_eq!(&s[0x22..0x24], &[0xFF, 0xD0]);
        assert_eq!(&s[0x38..0x3C], &[0x48, 0x83, 0xC4, 0x48]);
        assert_eq!(&s[0x3C..0x3C + 15], &stolen);
        assert_eq!(&s[0x4B..0x4D], &[0x48, 0xB8]);
        assert_eq!(u64::from_le_bytes(s[0x4D..0x55].try_into().unwrap()), 0x1000 + 15);
        assert_eq!(&s[0x55..0x57], &[0xFF, 0xE0]);
        let p = patch_bytes(0x2000, 15);
        assert_eq!(p.len(), 15);
        assert_eq!(&p[..2], &[0x48, 0xB8]);
        assert_eq!(u64::from_le_bytes(p[2..10].try_into().unwrap()), 0x2000);
        assert_eq!(&p[10..12], &[0xFF, 0xE0]);
        assert!(p[12..].iter().all(|&b| b == 0x90));
    }
}
