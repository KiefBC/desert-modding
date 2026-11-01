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
//! The formatter options and the window builder live in `desert_tools::disasm`
//! since 2026-11-01, shared with `retarget`, and moved there unchanged with
//! their tests; this file is the decode loop and the command line.
//!
//! Nothing here picks a section by name. This exe's twelve section names are
//! scrambled and the 80 MB one holding the code is called `.idata`; the range
//! is cut out by ADDRESS, from the section table. See `tests/pe_real_exe.rs`.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use iced_x86::{Decoder, DecoderOptions, Formatter, Instruction};

use desert_tools::disasm::{formatter, window, MAX_INSTR};
use desert_tools::paths;
use desert_tools::pe::Pe;

/// `cut -c1-100` at the end of `dis.sh`'s pipeline. Long lines in this output
/// are almost always a decoder having found a wall of prefixes in data, and
/// letting them wrap makes a screen of real code unreadable.
const COLUMNS: usize = 100;

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
