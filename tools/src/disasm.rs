//! The decoder side of the binary tools: iced-x86 over an image laid out the
//! way the loader maps it, addresses as RVAs.
//!
//! `dis` was the only disassembler user until `retarget`, and two of its
//! pieces moved here unchanged so both tools share them: [`formatter`], whose
//! options are what keep `dis`'s listing byte-identical to what
//! `tests/dis_cli.rs` pins, and [`window`], which cuts one range of the image
//! out of the exe without building all 385 MB of it.
//!
//! What is new is the instruction-level view `retarget` needs: [`decode_range`]
//! returns whole instructions with their bytes, whether they read RIP-relative
//! memory or change control flow, where a branch or a RIP operand points, and
//! the constant offsets the decoder reports. [`auto_wildcard`] turns a run of
//! them into a signature with every rel32, RIP disp32 and imm32 masked, which
//! is the first draft of every signature this repo has re-derived by hand.

use std::io::{Read, Seek, SeekFrom};

use anyhow::{Context, Result};
use iced_x86::{
    ConstantOffsets, Decoder, DecoderOptions, FlowControl, Formatter, Instruction, IntelFormatter,
    MemorySizeOptions,
};

use crate::pattern::Pattern;
use crate::pe::Pe;

/// The longest an x86-64 instruction can be.
pub const MAX_INSTR: usize = 15;

/// One decoded instruction.
#[derive(Debug, Clone)]
pub struct Insn {
    /// Its RVA.
    pub rva: u32,
    /// Its encoded bytes, exactly `instr.len()` of them.
    pub bytes: Vec<u8>,
    pub instr: Instruction,
    /// Where its displacement and immediates sit inside `bytes`, from the
    /// decoder (`Decoder::get_constant_offsets`). Branch displacements are
    /// reported as immediates.
    pub offsets: ConstantOffsets,
}

impl Insn {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// One past its last byte.
    pub fn end(&self) -> u32 {
        self.rva + self.bytes.len() as u32
    }

    /// Reads or writes `[rip+disp32]`. Such an instruction cannot be copied to
    /// a trampoline as it stands: its operand would point somewhere else.
    pub fn is_rip_rel(&self) -> bool {
        self.instr.is_ip_rel_memory_operand()
    }

    /// Anything but straight-line flow: a jump, call, return, interrupt or
    /// exception. Copying one into a stolen range breaks the hook.
    pub fn is_branch(&self) -> bool {
        self.instr.flow_control() != FlowControl::Next
    }

    /// The target of a direct near call or jump, conditional or not.
    ///
    /// `Instruction::near_branch_target` returns 0, not an error, for an
    /// instruction that is not a near branch; that is the trap `dis` warns
    /// about, and why this checks the flow-control class first.
    pub fn rel_target(&self) -> Option<u32> {
        match self.instr.flow_control() {
            FlowControl::Call | FlowControl::UnconditionalBranch | FlowControl::ConditionalBranch => {
                Some(self.instr.near_branch_target() as u32)
            }
            _ => None,
        }
    }

    /// The RVA a RIP-relative operand resolves to.
    pub fn rip_target(&self) -> Option<u32> {
        self.is_rip_rel().then(|| self.instr.ip_rel_memory_address() as u32)
    }

    /// `E8 rel32`.
    pub fn is_call_rel32(&self) -> bool {
        self.bytes.first() == Some(&0xE8) && self.len() == 5
    }

    /// `E9 rel32`.
    pub fn is_jmp_rel32(&self) -> bool {
        self.bytes.first() == Some(&0xE9) && self.len() == 5
    }

    /// The text [`formatter`] gives it.
    pub fn text(&self) -> String {
        let mut s = String::new();
        formatter().format(&self.instr, &mut s);
        s
    }
}

/// Decode `len` bytes of `image` from `rva`, returning whole instructions.
///
/// An instruction that starts inside the range is returned whole even when it
/// runs past its end, as `dis` does. Decoding stops at the end of the image;
/// an instruction cut off by the end of the image decodes as invalid and is
/// returned as such. `image` is the loader layout (`Pe::image`, or a
/// [`window`] starting at RVA 0); a window starting elsewhere goes through
/// [`decode_bytes`].
pub fn decode_range(image: &[u8], rva: u32, len: usize) -> Vec<Insn> {
    let start = (rva as usize).min(image.len());
    let end = start.saturating_add(len).saturating_add(MAX_INSTR).min(image.len());
    decode_bytes(&image[start..end], rva, len)
}

/// [`decode_range`] over bytes that start at `rva`.
pub fn decode_bytes(bytes: &[u8], rva: u32, len: usize) -> Vec<Insn> {
    let mut decoder = Decoder::with_ip(64, bytes, rva as u64, DecoderOptions::NONE);
    let mut out = Vec::new();
    let stop = rva as u64 + len as u64;
    let mut instr = Instruction::default();
    while decoder.can_decode() {
        let at = decoder.position();
        decoder.decode_out(&mut instr);
        if instr.ip() >= stop {
            break;
        }
        let offsets = decoder.get_constant_offsets(&instr);
        let n = instr.len();
        out.push(Insn {
            rva: instr.ip() as u32,
            bytes: bytes[at..at + n].to_vec(),
            instr,
            offsets,
        });
    }
    out
}

/// The instructions' bytes, cut to `max_bytes`, with every rel32, every RIP
/// disp32 and every imm32 replaced by `??`.
///
/// Those are the bytes that move when the code around them or the data it
/// names moves, so a pattern without them survives a relink that keeps the
/// instruction stream. Everything else stays literal: disp8s, imm8s and
/// non-RIP disp32s are the codegen itself, and the point of a signature is to
/// notice when that changes. An instruction that does not fit whole in
/// `max_bytes` is cut where the limit falls.
pub fn auto_wildcard(insns: &[Insn], max_bytes: usize) -> Pattern {
    let mut bytes = Vec::new();
    let mut fixed = Vec::new();
    for i in insns {
        let mut mask = vec![true; i.len()];
        let co = &i.offsets;
        if co.has_displacement() && co.displacement_size() == 4 && i.is_rip_rel() {
            let at = co.displacement_offset();
            mask[at..at + 4].fill(false);
        }
        if co.has_immediate() && co.immediate_size() == 4 {
            let at = co.immediate_offset();
            mask[at..at + 4].fill(false);
        }
        if co.has_immediate2() && co.immediate_size2() == 4 {
            let at = co.immediate_offset2();
            mask[at..at + 4].fill(false);
        }
        bytes.extend_from_slice(&i.bytes);
        fixed.extend_from_slice(&mask);
        if bytes.len() >= max_bytes {
            break;
        }
    }
    bytes.truncate(max_bytes);
    fixed.truncate(max_bytes);
    Pattern::from_parts(bytes, fixed)
}

/// The bytes at `[start, start + len)` of the image, the way the loader would
/// have them: every section's raw bytes at its virtual address, zero elsewhere.
///
/// This is `pe::image()` restricted to one window, and it must agree with it
/// byte for byte - a unit test pins that against a hand-built PE. Reading only
/// the window is what keeps `dis` from pulling 375 MB off disk to print a
/// screenful, and the zero fill matters: a range reaching past a section's raw
/// size into its `.bss`-style tail must read as zeros, not as the next
/// section's bytes shifted into place.
///
/// `pe` may be parsed from the file's first few KiB only, so `pe.data` is NOT
/// necessarily the whole file - `file_len` is passed separately for the same
/// clamp `pe::image()` applies with `self.data.len()`.
pub fn window<R: Read + Seek>(
    pe: &Pe,
    file_len: u64,
    src: &mut R,
    start: u32,
    len: usize,
) -> Result<Vec<u8>> {
    let mut out = vec![0u8; len];
    let end = start as u64 + len as u64;
    for s in &pe.sections {
        let va = s.virtual_address as u64;
        let raw = (s.raw_size as u64).min(file_len.saturating_sub(s.raw_offset as u64));
        let lo = va.max(start as u64);
        let hi = (va + raw).min(end);
        if lo >= hi {
            continue;
        }
        let dst = (lo - start as u64) as usize;
        let at = s.raw_offset as u64 + (lo - va);
        let n = (hi - lo) as usize;
        src.seek(SeekFrom::Start(at))
            .with_context(|| format!("cannot seek to {at:#x} for section {}", s.name))?;
        src.read_exact(&mut out[dst..dst + n])
            .with_context(|| format!("cannot read {n:#x} bytes at {at:#x}"))?;
    }
    Ok(out)
}

/// An `IntelFormatter` tuned to read like the `objdump -M intel` output `dis`
/// replaced, so that a diff against the old tool shows real disagreements
/// rather than a different house style on every line.
///
/// Only spelling is touched here. Nothing in these options can change which
/// bytes are one instruction. `tests/dis_cli.rs` pins the result line for
/// line; change an option here and `dis`'s output changes with it.
pub fn formatter() -> IntelFormatter {
    let mut f = IntelFormatter::new();
    let o = f.options_mut();
    // `0x50`, not `50h`: binutils' spelling, and the one every address in the
    // rest of this repo's docs is written in.
    o.set_hex_prefix("0x");
    o.set_hex_suffix("");
    o.set_uppercase_hex(false);
    // `je 0x176445d`, not `je 0x0176445d` and not `je short 0x176445d`. The
    // `short` is true and useless: whether a jump encodes as rel8 or rel32
    // never changes what it does, and dropping it keeps the mnemonic column
    // aligned with the rest of the listing.
    o.set_branch_leading_zeros(false);
    o.set_show_branch_size(false);
    // `cmp byte ptr [rsp+0x30],0x0`, not `,0`. iced writes 0..9 in decimal by
    // default, which reads as a different KIND of number from every other
    // operand on the screen.
    o.set_small_hex_numbers_in_decimal(false);
    // objdump prints the operand size on every memory operand even when it is
    // implied; iced omits it by default. Keeping it makes a `mov [rcx],eax`
    // versus `mov [rcx],rax` misreading impossible at a glance. Upper case for
    // the same reason binutils does it - `QWORD PTR` is scaffolding, and
    // shouting it keeps it from being mistaken for part of the operand.
    o.set_memory_size_options(MemorySizeOptions::Always);
    o.set_uppercase_keywords(true);
    o.set_space_after_operand_separator(false);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_BYTES: usize = 0x1000;

    /// A hand-built PE32+ with two sections: one whose virtual size exceeds its
    /// raw size (so the window has to zero-fill a tail), and one after it.
    fn fixture() -> Vec<u8> {
        let mut data = vec![0u8; HEADER_BYTES];
        data[..2].copy_from_slice(b"MZ");
        let pe_off = 0x80usize;
        data[0x3c..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
        data[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
        data[pe_off + 6..pe_off + 8].copy_from_slice(&2u16.to_le_bytes());
        data[pe_off + 20..pe_off + 22].copy_from_slice(&0xF0u16.to_le_bytes());
        let opt = pe_off + 24;
        data[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        data[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        data[opt + 56..opt + 60].copy_from_slice(&0x4000u32.to_le_bytes());
        let mut sec = opt + 0xF0;
        for (name, vs, va, rs, ro) in [
            (b".idata\0\0", 0x400u32, 0x1000u32, 0x200u32, 0x400u32),
            (b".arch\0\0\0", 0x200u32, 0x2000u32, 0x200u32, 0x600u32),
        ] {
            data[sec..sec + 8].copy_from_slice(name);
            data[sec + 8..sec + 12].copy_from_slice(&vs.to_le_bytes());
            data[sec + 12..sec + 16].copy_from_slice(&va.to_le_bytes());
            data[sec + 16..sec + 20].copy_from_slice(&rs.to_le_bytes());
            data[sec + 20..sec + 24].copy_from_slice(&ro.to_le_bytes());
            sec += 40;
        }
        for i in 0..0x200 {
            data[0x400 + i] = (i % 251) as u8;
            data[0x600 + i] = 0xAA;
        }
        data
    }

    #[test]
    fn a_window_is_the_same_bytes_the_full_image_would_have() {
        let data = fixture();
        let img = Pe::parse(&data).unwrap().image();
        // Parsed from the FIRST 0x1000 BYTES ONLY, exactly as `dis` does it -
        // so this also pins that the header read is big enough to hold the
        // section table, which is the thing that would break silently.
        let head = data[..HEADER_BYTES].to_vec();
        let pe = Pe::parse(&head).unwrap();
        let mut src = std::io::Cursor::new(&data);

        // Across a section's raw bytes, across its zero-filled tail, across
        // the gap to the next section and into it. The zero-fill tail is the
        // case a naive `read at rva_to_offset(start)` gets wrong, and the
        // headers-to-first-section gap is the case `pe::image()` leaves zero.
        for (start, len) in [
            (0x1000u32, 0x10usize),
            (0x11F8, 0x20),
            (0x13F0, 0x300),
            (0x1FF0, 0x30),
            (0, 0x1010),
        ] {
            assert_eq!(
                window(&pe, data.len() as u64, &mut src, start, len).unwrap(),
                img[start as usize..start as usize + len],
                "window at {start:#x} len {len:#x}"
            );
        }
    }

    #[test]
    fn a_window_past_the_end_of_the_image_is_zero_not_a_panic() {
        let data = fixture();
        let pe = Pe::parse(&data).unwrap();
        let mut src = std::io::Cursor::new(&data);
        let w = window(&pe, data.len() as u64, &mut src, 0x3FF0, 0x40).unwrap();
        assert_eq!(w.len(), 0x40);
        assert!(w.iter().all(|&b| b == 0));
    }

    #[test]
    fn a_section_whose_raw_bytes_run_past_the_end_of_the_file_is_clamped() {
        // A truncated download, or a section table that lies. `pe::image()`
        // clamps with `self.data.len()`; `window` has to clamp with the real
        // file length, because its `pe` may only hold the first few KiB and
        // clamping on THAT would read nothing at all.
        let data = fixture();
        let pe = Pe::parse(&data[..HEADER_BYTES]).unwrap();
        let short = &data[..0x500];
        let mut src = std::io::Cursor::new(short);
        let w = window(&pe, short.len() as u64, &mut src, 0x1000, 0x200).unwrap();
        assert_eq!(&w[..0x100], &data[0x400..0x500], "the bytes that are there");
        assert!(w[0x100..].iter().all(|&b| b == 0), "the rest is zero, not an error");
    }

    #[test]
    fn addresses_are_rvas_and_instruction_lengths_advance_them() {
        // `40 53` (rex push rbx), `48 83 EC 50` (sub rsp,0x50): the first is
        // two bytes, not one, so a decoder that dropped the REX prefix would
        // print a second line at +0x1764421 that `dis.sh` never printed.
        let code = [0x40u8, 0x53, 0x48, 0x83, 0xEC, 0x50];
        let insns = decode_bytes(&code, 0x1764420, code.len());
        let out: Vec<String> = insns.iter().map(|i| format!("+{:x} {}", i.rva, i.text())).collect();
        assert_eq!(out, vec!["+1764420 push rbx".to_string(), "+1764422 sub rsp,0x50".to_string()]);
        assert_eq!(insns[0].bytes, vec![0x40, 0x53]);
        assert_eq!(insns[1].end(), 0x1764426);
    }

    #[test]
    fn hex_is_binutils_spelled_and_rip_relative_operands_resolve_to_rvas() {
        // `48 8D 05 2A 85 09 04` = lea rax,[rip+0x409852a] at +0x176445f, i.e.
        // +0x57fc990. iced prints the resolved address, which is the number a
        // reader actually wants and is only correct because the decoder's IP
        // is an RVA.
        let code = [0x48u8, 0x8D, 0x05, 0x2A, 0x85, 0x09, 0x04];
        let insns = decode_bytes(&code, 0x176_445f, code.len());
        let s = insns[0].text();
        assert!(s.contains("0x57fc990"), "got {s}");
        assert!(!s.contains('h'), "no `50h`-style hex: {s}");
        assert!(insns[0].is_rip_rel());
        assert_eq!(insns[0].rip_target(), Some(0x57fc990));
        assert!(!insns[0].is_branch());
    }

    #[test]
    fn calls_jumps_and_returns_are_branches_with_targets_where_they_have_one() {
        // call +0x20 (rel32 from the end, 0x105), jne short, ret, int3.
        let code = [0xE8u8, 0x1B, 0x00, 0x00, 0x00, 0x75, 0x02, 0xC3, 0xCC];
        let i = decode_bytes(&code, 0x100, code.len());
        assert_eq!(i.len(), 4);
        assert!(i.iter().all(Insn::is_branch));
        assert!(i[0].is_call_rel32());
        assert_eq!(i[0].rel_target(), Some(0x120));
        assert_eq!(i[1].rel_target(), Some(0x109));
        assert_eq!(i[2].rel_target(), None, "a ret has no static target");
        assert_eq!(i[3].rel_target(), None);
    }

    #[test]
    fn auto_wildcard_masks_rel32_rip_disp32_and_imm32_only() {
        // mov [rsp+0x8],rbx         48 89 5C 24 08     disp8 stays
        // mov edx,0x80              BA 80 00 00 00     imm32 goes
        // lea rcx,[rip+0x1000]      48 8D 0D 00 10 00 00   disp32 goes
        // mov rax,[rbx+0x120]       48 8B 83 20 01 00 00   non-RIP disp32 stays
        // call rel32                E8 00 00 00 00     rel32 goes
        let code = [
            0x48u8, 0x89, 0x5C, 0x24, 0x08, 0xBA, 0x80, 0x00, 0x00, 0x00, 0x48, 0x8D, 0x0D, 0x00,
            0x10, 0x00, 0x00, 0x48, 0x8B, 0x83, 0x20, 0x01, 0x00, 0x00, 0xE8, 0x00, 0x00, 0x00,
            0x00,
        ];
        let insns = decode_bytes(&code, 0x1000, code.len());
        let p = auto_wildcard(&insns, 64);
        assert_eq!(
            p.to_string(),
            "48 89 5C 24 08 BA ?? ?? ?? ?? 48 8D 0D ?? ?? ?? ?? 48 8B 83 20 01 00 00 E8 ?? ?? ?? ??"
        );
        // Cut where the limit falls, even inside an instruction.
        assert_eq!(auto_wildcard(&insns, 7).to_string(), "48 89 5C 24 08 BA ??");
    }

    #[test]
    fn decode_range_returns_whole_instructions_past_the_end() {
        let mut img = vec![0xCCu8; 0x40];
        img[0x10..0x14].copy_from_slice(&[0x48, 0x83, 0xEC, 0x50]);
        let i = decode_range(&img, 0x10, 1);
        assert_eq!(i.len(), 1);
        assert_eq!(i[0].len(), 4);
        // At the very end of the image it stops rather than panicking.
        assert!(decode_range(&img, 0x40, 0x10).is_empty());
    }
}
