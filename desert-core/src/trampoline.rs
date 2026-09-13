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

/// Build a stub for a hook that **replaces** an instruction rather than
/// observing one: the callback's `u32` return value lands in `r8d`.
///
/// Unlike [`stub_bytes`] this is not a prologue hook. It is installed at a
/// hand-picked site in the middle of a function, over a `mov r8d,<imm32>`
/// that sets up an argument for the call a few bytes later, and the whole
/// point is that the immediate never runs again:
///
/// ```text
/// stub:  sub rsp,0x20         ; shadow space; rsp stays 16-aligned for the call
///        mov rcx,r15          ; the site's live pointer -> first argument
///        mov rax,<callback>; call rax     ; extern "system" fn(usize) -> u32
///        add rsp,0x20
///        mov r8d,eax          ; the value the game will use, from us
///        <replay>             ; the stolen instructions after the one replaced
///        mov rax,<resume>; jmp rax
/// ```
///
/// The caller owns three facts a disassembly has to establish: that `r15`
/// holds what the callback wants, that every volatile register (rax, rcx,
/// rdx, r8-r11, xmm0-5) is dead at the site, and that `rsp` is 16-byte
/// aligned there. Nothing is saved or restored, so a live volatile register
/// would be lost. `replay` is the tail of the stolen bytes - the whole,
/// position-independent instructions that followed the replaced one - and
/// `resume` is the address just past the last of them.
///
/// For Desert Gatherer's catch-count hook the site is `0x142a75891` on build
/// 25246367 (`docs/reference-internals.md` section 17): 13 stolen bytes,
/// `mov r8d,1` replaced and `lea rdx,[rbp+0x1d0]` replayed. The address is
/// there to be read beside a disassembly - the hook finds the site by its
/// bytes (`desert_core::creature::CATCH_SITE`), which is why it survived that
/// address moving 0x1740 in the update from 25116796.
pub fn count_hook_stub(callback: usize, resume: usize, replay: &[u8]) -> Vec<u8> {
    let mut s = Vec::with_capacity(0x26 + replay.len());
    s.extend_from_slice(&[0x48, 0x83, 0xEC, 0x20]); // sub rsp,0x20
    s.extend_from_slice(&[0x4C, 0x89, 0xF9]); // mov rcx,r15
    s.extend_from_slice(&[0x48, 0xB8]); // mov rax,imm64
    s.extend_from_slice(&(callback as u64).to_le_bytes());
    s.extend_from_slice(&[0xFF, 0xD0]); // call rax
    s.extend_from_slice(&[0x48, 0x83, 0xC4, 0x20]); // add rsp,0x20
    s.extend_from_slice(&[0x41, 0x89, 0xC0]); // mov r8d,eax
    debug_assert_eq!(s.len(), 0x1A);
    s.extend_from_slice(replay);
    s.extend_from_slice(&[0x48, 0xB8]); // mov rax,imm64
    s.extend_from_slice(&(resume as u64).to_le_bytes());
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

    /// Every byte of the count-hook stub, against a disassembly.
    ///
    /// The whole sequence was assembled into a file and read back with
    /// `objdump -D -b binary -m i386:x86-64 -M intel`, which printed exactly
    /// the mnemonics named in the comments below - in particular `41 89 C0` is
    /// `mov r8d,eax` and `41 8B C0` is `mov eax,r8d`, the wrong direction.
    #[test]
    fn count_hook_stub_matches_the_disassembly() {
        // The real site's second stolen instruction: lea rdx,[rbp+0x1d0].
        let replay = [0x48, 0x8D, 0x95, 0xD0, 0x01, 0x00, 0x00];
        let s = count_hook_stub(0xDEAD_BEEF, 0x1_4000_1234, &replay);
        assert_eq!(s.len(), 0x26 + replay.len());

        assert_eq!(&s[0x00..0x04], &[0x48, 0x83, 0xEC, 0x20], "sub rsp,0x20");
        assert_eq!(&s[0x04..0x07], &[0x4C, 0x89, 0xF9], "mov rcx,r15");
        assert_eq!(&s[0x07..0x09], &[0x48, 0xB8], "mov rax,imm64");
        assert_eq!(u64::from_le_bytes(s[0x09..0x11].try_into().unwrap()), 0xDEAD_BEEF);
        assert_eq!(&s[0x11..0x13], &[0xFF, 0xD0], "call rax");
        assert_eq!(&s[0x13..0x17], &[0x48, 0x83, 0xC4, 0x20], "add rsp,0x20");
        assert_eq!(&s[0x17..0x1A], &[0x41, 0x89, 0xC0], "mov r8d,eax");
        assert_eq!(&s[0x1A..0x21], &replay, "the replayed stolen instruction");
        assert_eq!(&s[0x21..0x23], &[0x48, 0xB8], "mov rax,imm64");
        assert_eq!(u64::from_le_bytes(s[0x23..0x2B].try_into().unwrap()), 0x1_4000_1234);
        assert_eq!(&s[0x2B..0x2D], &[0xFF, 0xE0], "jmp rax");

        // Nothing is pushed before the call, so the 0x20 of shadow space is
        // all that moves rsp: 16-aligned in, 16-aligned at the call, which is
        // what puts the callee's rsp at 8 mod 16 the way the ABI wants.
        assert_eq!(0x20 % 16, 0);
    }

    #[test]
    fn count_hook_stub_takes_an_empty_replay() {
        // Degenerate but legal: the replaced instruction was the only stolen
        // one, so the stub jumps straight back.
        let s = count_hook_stub(1, 2, &[]);
        assert_eq!(s.len(), 0x26);
        assert_eq!(&s[0x17..0x1A], &[0x41, 0x89, 0xC0]);
        assert_eq!(&s[0x1A..0x1C], &[0x48, 0xB8]);
        assert_eq!(u64::from_le_bytes(s[0x1C..0x24].try_into().unwrap()), 2);
        assert_eq!(&s[0x24..0x26], &[0xFF, 0xE0]);
    }
}
