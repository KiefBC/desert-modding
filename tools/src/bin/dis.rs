//! Disassemble a range of CrimsonDesert.exe by RVA, Intel syntax.
//!
//! ```text
//! dis <start-rva-hex> <end-rva-hex>
//! ```
//!
//! Addresses and branch targets are printed as RVAs; add 0x140000000 for the
//! preferred VA that Ghidra shows. One line per instruction, `+<rva>` then the
//! instruction, truncated to 100 columns - the same shape `tools/dis.sh`
//! produced through `objdump | sed | cut`.
//!
//! # Two things this port deliberately changes
//!
//! **There is no cached image file any more, and that is the point.** `dis.sh`
//! wrote a ~385 MB image-layout copy of the exe into `$TMPDIR` and left it
//! there. The cache was originally keyed on existence alone, so after a game
//! update every call kept disassembling the PREVIOUS build's bytes - clean
//! looking, correctly formatted, and wrong, with nothing printed to say so. A
//! 2026-09-13 investigation lost time to that before noticing the cached image
//! was three days older than the exe and a different size. The script grew an
//! `-nt` check to paper over it. Nothing is cached here: the bytes for the
//! requested range are cut out of the exe on every run, so there is no stale
//! copy to serve and the whole class of bug is gone, not merely detected. It
//! is also why `window()` below builds only the requested range rather than
//! calling `pe::image()` - a 385 MB allocation to print forty instructions is
//! the same waste in RAM that the script committed on disk.
//!
//! **The disassembler is `iced-x86`, not `objdump`.** That removes the last
//! `objdump` dependency from `tools/`, so `dis` no longer needs the nix dev
//! shell and no longer fails with a bare "command not found" outside it. The
//! cost is that operand SPELLING differs from the old output in places -
//! `0x50` against `50h`-style hex is normalised back below, but iced and GNU
//! binutils disagree about a handful of mnemonics and about how to render a
//! RIP-relative operand. Spelling is cosmetic; the addresses are not. If ever
//! an instruction BOUNDARY moves - a `+rva` in this output that `dis.sh` does
//! not also print - that is a real decoding disagreement and worth chasing.
//!
//! Nothing here picks a section by name. This exe's twelve section names are
//! scrambled and the 80 MB one holding the code is called `.idata`; the range
//! is cut out by ADDRESS, from the section table. See `tests/pe_real_exe.rs`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use iced_x86::{Decoder, DecoderOptions, Formatter, Instruction, IntelFormatter, MemorySizeOptions};

use desert_tools::paths;
use desert_tools::pe::Pe;

/// `cut -c1-100` at the end of `dis.sh`'s pipeline. Long lines in this output
/// are almost always a decoder having found a wall of prefixes in data, and
/// letting them wrap makes a screen of real code unreadable.
const COLUMNS: usize = 100;

/// The longest an x86-64 instruction can be. The range is decoded with this
/// much slack past its end so that an instruction straddling the end address
/// still decodes whole, which is what `objdump --stop-address` does; without
/// it the last line of every run would be a truncated `(bad)`.
const MAX_INSTR: usize = 15;

/// Enough of the front of the exe to hold the DOS stub, the PE headers and the
/// whole section table - everything `Pe::parse` reads. On this exe the section
/// table ends around `0x3d8` and the first section's bytes start at `0x400`,
/// so this is comfortable without reaching into any section.
///
/// Reading only this much, plus the requested window, is what keeps `dis` from
/// pulling 375 MB through the page cache to print forty lines. It is not just
/// tidiness: a `read` of the whole file failed outright with ENOMEM on this
/// machine while two other builds were running.
const HEADER_BYTES: u64 = 0x1000;

#[derive(Parser)]
#[command(about = "Disassemble a range of CrimsonDesert.exe by RVA (Intel syntax)")]
struct Cli {
    /// First RVA to disassemble, hex, no `0x` needed.
    start: String,
    /// One past the last RVA to disassemble, hex.
    end: String,
}

fn parse_rva(s: &str, what: &str) -> Result<u32> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u32::from_str_radix(t, 16).with_context(|| format!("{what} `{s}` is not a hexadecimal RVA"))
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
/// `pe` is parsed from the file's first few KiB, so `pe.data` is NOT the whole
/// file here - `file_len` is passed separately for the same clamp `pe::image()`
/// applies with `self.data.len()`.
fn window<R: Read + Seek>(
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

/// An `IntelFormatter` tuned to read like the `objdump -M intel` output this
/// replaces, so that a diff against the old tool shows real disagreements
/// rather than a different house style on every line.
///
/// Only spelling is touched here. Nothing in these options can change which
/// bytes are one instruction.
fn formatter() -> IntelFormatter {
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    let start = parse_rva(&cli.start, "start address")?;
    let end = parse_rva(&cli.end, "end address")?;
    if end <= start {
        bail!("empty range: end {end:#x} is not past start {start:#x}");
    }

    let exe: PathBuf = paths::game_exe();
    let mut file = File::open(&exe)
        .with_context(|| format!("cannot open the game exe {}", exe.display()))?;
    let file_len = file
        .metadata()
        .with_context(|| format!("cannot stat {}", exe.display()))?
        .len();
    let mut head = Vec::with_capacity(HEADER_BYTES as usize);
    (&mut file)
        .take(HEADER_BYTES)
        .read_to_end(&mut head)
        .with_context(|| format!("cannot read the headers of {}", exe.display()))?;
    let pe = Pe::parse(&head).with_context(|| format!("{} is not a PE32+ image", exe.display()))?;
    if start >= pe.size_of_image {
        bail!(
            "start {start:#x} is past the end of the image ({:#x})",
            pe.size_of_image
        );
    }

    let len = (end - start) as usize + MAX_INSTR;
    let bytes = window(&pe, file_len, &mut file, start, len)?;

    // `with_ip(start)` is the whole reason this tool exists: the decoder's
    // addresses, and every RIP-relative operand it resolves, come out as RVAs
    // rather than as offsets into a buffer.
    let mut decoder = Decoder::with_ip(64, &bytes, start as u64, DecoderOptions::NONE);
    let mut fmt = formatter();
    let mut instr = Instruction::default();
    let mut text = String::new();

    // Nothing here calls `near_branch_target()`. It would be the obvious way to
    // annotate call targets, and it is a trap: on an instruction whose operand
    // is not a near branch it does not fail, it silently returns 0. Printing
    // the formatter's own rendering cannot go wrong that way.
    while decoder.can_decode() {
        decoder.decode_out(&mut instr);
        if instr.ip() >= end as u64 {
            break;
        }
        text.clear();
        fmt.format(&instr, &mut text);
        let line = format!("+{:x} {text}", instr.ip());
        // Truncate by CHARACTER, the way `cut -c` does. The formatter only
        // ever emits ASCII, but slicing a String by byte index is a panic
        // waiting for the day that stops being true.
        println!("{}", line.chars().take(COLUMNS).collect::<String>());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built PE32+ with two sections: one whose virtual size exceeds its
    /// raw size (so the window has to zero-fill a tail), and one after it.
    fn fixture() -> Vec<u8> {
        let mut data = vec![0u8; HEADER_BYTES as usize];
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
        // Parsed from the FIRST 0x1000 BYTES ONLY, exactly as `main` does it -
        // so this also pins that the header read is big enough to hold the
        // section table, which is the thing that would break silently.
        let head = data[..HEADER_BYTES as usize].to_vec();
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
        // file length, because its `pe` only holds the first few KiB and
        // clamping on THAT would read nothing at all.
        let data = fixture();
        let pe = Pe::parse(&data[..HEADER_BYTES as usize]).unwrap();
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
        // print a second line at +0x1764421 that `dis.sh` never printed. That
        // is the shape of the only difference from the old tool that matters.
        let code = [0x40u8, 0x53, 0x48, 0x83, 0xEC, 0x50];
        let mut d = Decoder::with_ip(64, &code, 0x1764420, DecoderOptions::NONE);
        let mut f = formatter();
        let mut out = Vec::new();
        let mut instr = Instruction::default();
        while d.can_decode() {
            d.decode_out(&mut instr);
            let mut s = String::new();
            f.format(&instr, &mut s);
            out.push(format!("+{:x} {s}", instr.ip()));
        }
        assert_eq!(out, vec!["+1764420 push rbx".to_string(), "+1764422 sub rsp,0x50".to_string()]);
    }

    #[test]
    fn hex_is_binutils_spelled_and_rip_relative_operands_resolve_to_rvas() {
        // `48 8D 05 2A 85 09 04` = lea rax,[rip+0x409852a] at +0x176445f, i.e.
        // +0x57fc990. objdump prints the displacement and the resolved address
        // in a trailing comment; iced prints the resolved address, which is
        // the number a reader actually wants and is only correct because the
        // decoder's IP is an RVA.
        let code = [0x48u8, 0x8D, 0x05, 0x2A, 0x85, 0x09, 0x04];
        let mut d = Decoder::with_ip(64, &code, 0x176_445f, DecoderOptions::NONE);
        let mut f = formatter();
        let mut s = String::new();
        let mut instr = Instruction::default();
        d.decode_out(&mut instr);
        f.format(&instr, &mut s);
        assert!(s.contains("0x57fc990"), "got {s}");
        assert!(!s.contains('h'), "no `50h`-style hex: {s}");
    }

    #[test]
    fn a_line_is_cut_at_a_hundred_columns() {
        let long = format!("+{:x} {}", 0x1000, "a".repeat(200));
        let cut: String = long.chars().take(COLUMNS).collect();
        assert_eq!(cut.len(), COLUMNS);
    }

    #[test]
    fn rva_parsing_takes_a_bare_hex_string_and_an_0x_prefixed_one() {
        assert_eq!(parse_rva("1764420", "start").unwrap(), 0x1764420);
        assert_eq!(parse_rva("0x1764420", "start").unwrap(), 0x1764420);
        assert!(parse_rva("nonsense", "start").is_err());
    }
}
