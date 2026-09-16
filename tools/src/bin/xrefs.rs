//! Find references to an RVA in CrimsonDesert.exe without a disassembler.
//!
//! ```text
//! xrefs <rva-hex> [exe] [--selfcheck]
//! ```
//!
//! Ported from `tools/xrefs.py`. Its stdout is reproduced byte for byte -
//! column widths, uppercase hex, the `(s)` on every plural - because the only
//! way to show a port did not change an answer is to diff it against the thing
//! it replaced.
//!
//! **It takes an RVA. `sigscan` prints FILE OFFSETS.** The two are not the same
//! number and are not directly composable: feeding a `sigscan` offset to this
//! tool will usually report zero references, which looks like a bug and is not.
//! Convert through the PE section table (`desert_tools::pe`) first.
//! `tools/README.md` says the same thing, and it is repeated here because this
//! is where the confusion costs an hour.
//!
//! Nothing here picks a section by name, and nothing may: this exe's section
//! names are scrambled and the 80 MB one holding the code is called `.idata`.
//! The image is built from the section table's ADDRESSES, by
//! `desert_tools::pe::image()`.

// `#[path]` because a binary's submodules resolve against `src/bin/`, not
// against a directory named for the binary: a bare `mod scan;` here looks for
// `src/bin/scan.rs` and would collide with every other tool's submodules.
#[path = "xrefs/scan.rs"]
mod scan;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};

use desert_tools::pe::Pe;

/// What `xrefs` with no arguments prints, and the exit code it prints it with.
///
/// This was `xrefs.py`'s module docstring, taken over as the tool's only help
/// text so that through the port the two stayed interchangeable. Two lines
/// differed from the Python's even then, both deliberately: the usage line
/// names the binary rather than the script, and the last paragraph describes
/// the scan this file actually performs instead of the `bytes.find` one it was
/// ported from. Everything a user acts on - what is reported and why - is
/// unchanged from the script this replaced.
const USAGE: &str = r#"Find references to an RVA in CrimsonDesert.exe without a disassembler.

Usage: xrefs <rva-hex> [exe] [--selfcheck]

Reports three kinds of reference, all against the file laid out as an image, so
every offset printed is an RVA:

  * `E8` call / `E9` jmp rel32 whose target is the RVA;
  * RIP-relative `lea` / `mov` (REX and no-REX, modrm mod=00 rm=101);
  * **pointer cells** - eight bytes somewhere in the image holding the RVA's
    *VA*. These are how the indirect accessor encoding reaches a table name
    (`desert-core`'s `gimmick::ACCESSORS`, `docs/reference-internals.md`
    section 19), and a target with no code xref at all often has several. A
    scan that reports "no references" while a pointer array names the address
    is the failure mode this half exists to prevent.

The scan is one `memchr` pass per opcode form rather than a loop over every
byte of the 363 MB image, which is the difference between seconds and minutes.
Coverage is identical to the byte loop it replaced, which is still in
`scan::reference_scan`; `--selfcheck` diffs the two."#;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("xrefs: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    // Hand-rolled, not clap. The Python takes POSITIONAL arguments only and
    // ignores anything beginning with `--` except `--selfcheck`, so that a
    // flag can never be mistaken for the exe path - `xrefs 1764420 --selfcheck`
    // must not try to open a file called `--selfcheck`. clap would reject
    // unknown flags and print a different usage block on a different stream,
    // which is a change in behaviour for no gain.
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = argv.iter().filter(|a| !a.starts_with("--")).collect();
    if positional.is_empty() {
        println!("{USAGE}");
        return Ok(ExitCode::from(2));
    }
    let selfcheck = argv.iter().any(|a| a == "--selfcheck");

    let raw = positional[0].as_str();
    let target = u64::from_str_radix(raw.trim_start_matches("0x").trim_start_matches("0X"), 16)
        .with_context(|| format!("`{raw}` is not a hexadecimal RVA"))?;
    let exe = positional
        .get(1)
        .map(|p| PathBuf::from(p.as_str()))
        .unwrap_or_else(desert_tools::paths::game_exe);

    // The file and the image are each ~375 MB, and holding both for the whole
    // run is 760 MB for no reason: nothing after this block looks at the file
    // again. Six of these in parallel is what `cargo test -- --ignored` does,
    // and on this machine that was the difference between passing and ENOMEM.
    let (img, base) = {
        let data = std::fs::read(&exe)
            .with_context(|| format!("cannot read the game exe {}", exe.display()))?;
        let pe =
            Pe::parse(&data).with_context(|| format!("{} is not a PE32+ image", exe.display()))?;
        (image_the_way_the_python_built_it(&pe, &data), pe.image_base)
    };

    let hits = scan::fast_scan(&img, target);
    let cells = scan::pointer_cells(&img, base + target);

    if selfcheck {
        let want = scan::reference_scan(&img, target);
        if want != hits {
            println!(
                "SELFCHECK FAILED: fast {} vs byte-loop {}",
                hits.len(),
                want.len()
            );
            // The symmetric difference, in the same order the Python prints:
            // sorted by offset, then by kind.
            let mut only: Vec<(usize, &str, bool)> = Vec::new();
            for h in &want {
                if !hits.contains(h) {
                    only.push((h.0, h.1, true));
                }
            }
            for h in &hits {
                if !want.contains(h) {
                    only.push((h.0, h.1, false));
                }
            }
            only.sort_unstable();
            for (off, kind, in_want) in only {
                let who = if in_want { "byte-loop" } else { "fast" };
                println!("  only in {who:10}: {kind} at +0x{off:X}");
            }
            return Ok(ExitCode::FAILURE);
        }
        println!(
            "selfcheck ok: fast scan agrees with the byte loop on {} reference(s)",
            hits.len()
        );
    }

    for (off, kind) in &hits {
        println!("{kind:14} at +0x{off:X}");
    }
    for off in &cells {
        println!(
            "{:14} at +0x{off:X}  (holds VA 0x{:X})",
            "ptr cell",
            base + target
        );
    }
    println!(
        "{} code reference(s) and {} pointer cell(s) to +0x{target:X}",
        hits.len(),
        cells.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// `pe::image()`, plus the PE headers copied back over the front of it.
///
/// `desert_tools::pe::image()` lays out the SECTIONS and leaves everything
/// else zero, which is what `dis` wants - the headers are not code. `xrefs`
/// wants them: the loader maps them at RVA 0 and the Python this was ported
/// from copied them (`img[:hdr] = d[:hdr]`, `hdr` being the lowest raw
/// offset of any section). A pointer cell or a stray `E8` in the headers is
/// unlikely, but "unlikely" is not the same as "excluded", and a scan that
/// quietly covers less ground than the one it replaced is exactly the kind of
/// regression `--selfcheck` cannot catch, because both halves of it would be
/// looking at the same truncated buffer.
fn image_the_way_the_python_built_it(pe: &Pe, data: &[u8]) -> Vec<u8> {
    let mut img = pe.image();
    let hdr = pe
        .sections
        .iter()
        .map(|s| s.raw_offset as usize)
        .min()
        .unwrap_or(0)
        .min(img.len())
        .min(data.len());
    img[..hdr].copy_from_slice(&data[..hdr]);
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usage_block_still_warns_about_pointer_cells() {
        // The pointer-cell half is the half a reader is most likely to think
        // is optional. If the usage text loses the warning, the next person to
        // see "0 references" stops there.
        assert!(USAGE.contains("pointer cells"));
        assert!(USAGE.contains("no code xref at all often has several"));
        assert!(USAGE.contains("--selfcheck"));
    }

    #[test]
    fn headers_are_copied_over_the_image() {
        // A minimal PE32+ with one section, built by hand: the point is that
        // byte 0 of the image is `M`, not zero.
        let mut data = vec![0u8; 0x600];
        data[..2].copy_from_slice(b"MZ");
        let pe_off = 0x80usize;
        data[0x3c..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
        data[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
        data[pe_off + 6..pe_off + 8].copy_from_slice(&1u16.to_le_bytes()); // sections
        data[pe_off + 20..pe_off + 22].copy_from_slice(&0xF0u16.to_le_bytes()); // opt size
        let opt = pe_off + 24;
        data[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        data[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
        data[opt + 56..opt + 60].copy_from_slice(&0x2000u32.to_le_bytes()); // size of image
        let sec = opt + 0xF0;
        data[sec..sec + 8].copy_from_slice(b".idata\0\0");
        data[sec + 8..sec + 12].copy_from_slice(&0x200u32.to_le_bytes()); // virtual size
        data[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes()); // virtual address
        data[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes()); // raw size
        data[sec + 20..sec + 24].copy_from_slice(&0x400u32.to_le_bytes()); // raw offset
        data[0x400] = 0xAB;

        let pe = Pe::parse(&data).unwrap();
        let plain = pe.image();
        assert_eq!(plain[0], 0, "pe::image() leaves the headers zero");

        let img = image_the_way_the_python_built_it(&pe, &data);
        assert_eq!(&img[..2], b"MZ", "xrefs copies them back");
        assert_eq!(img[0x1000], 0xAB, "and the section still lands at its RVA");
        assert_eq!(img.len(), 0x2000);
    }
}
