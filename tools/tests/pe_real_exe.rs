//! `desert_tools::pe` against the shipped CrimsonDesert.exe.
//!
//! Every binary tool in here converts between file offsets and RVAs, and the
//! two are not the same number: `sigscan` prints offsets, `xrefs` takes an RVA,
//! and `tools/README.md` warns that feeding one to the other looks like a bug
//! and is not. That conversion lives in one place now, so it is worth checking
//! against the real image rather than a fixture - a synthetic PE would agree
//! with any implementation, including a wrong one.
//!
//! Ignored by default: the game is not on a CI runner. `cargo test -- --ignored`
//! runs them, and `just` has no recipe for them for the same reason
//! `just test-game` is separate.

use std::path::PathBuf;

use desert_tools::paths;
use desert_tools::pe::Pe;

fn exe() -> Option<PathBuf> {
    let p = paths::game_exe();
    p.is_file().then_some(p)
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn headers_are_the_expected_shape() {
    let Some(path) = exe() else {
        panic!("CrimsonDesert.exe not found at {}", paths::game_exe().display());
    };
    let data = std::fs::read(&path).unwrap();
    let pe = Pe::parse(&data).unwrap();

    // The plugins assume this base everywhere; an RVA plus 0x140000000 is the
    // preferred VA that Ghidra and `dis` both print.
    assert_eq!(pe.image_base, 0x1_4000_0000);
    assert_eq!(pe.sections.len(), 12);

    // DO NOT look for `.text` here. This exe's section names are scrambled:
    // the twelve are `.idata .arch .debug$P .tls$ .shared .xcode .00cfg .sbss
    // .text1 .trace .xtls .link`, and the 80 MB first one holding the code is
    // the one called `.idata`. An earlier version of this test asserted a
    // `.text` section existed and failed against the real game.
    //
    // The lesson generalises past this assert: nothing in tools/ may select a
    // section by NAME. Match on the address range, which is what the section
    // table actually means.
    assert!(pe.sections.iter().all(|s| s.name != ".text"));
    assert_eq!(pe.sections.first().map(|s| s.name.as_str()), Some(".idata"));
    assert!(pe.size_of_image as usize > data.len() / 2);
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn offset_and_rva_round_trip_through_every_section() {
    let data = std::fs::read(exe().unwrap()).unwrap();
    let pe = Pe::parse(&data).unwrap();

    for s in &pe.sections {
        if s.raw_size == 0 {
            continue;
        }
        // First, middle and last byte that is actually in the file. The last
        // one is the interesting case: an off-by-one in the bounds check shows
        // up there and nowhere else.
        for delta in [0, s.raw_size / 2, s.raw_size - 1] {
            let rva = s.virtual_address + delta;
            let off = pe
                .rva_to_offset(rva)
                .unwrap_or_else(|| panic!("{} rva {rva:#x} has no file offset", s.name));
            assert_eq!(off, (s.raw_offset + delta) as usize, "section {}", s.name);
            assert_eq!(pe.offset_to_rva(off), Some(rva), "section {}", s.name);
        }
    }
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn tail_of_a_section_beyond_its_raw_bytes_is_not_in_the_file() {
    let data = std::fs::read(exe().unwrap()).unwrap();
    let pe = Pe::parse(&data).unwrap();

    // A section whose virtual size exceeds its raw size is zero fill past the
    // end of its bytes. Answering that with an offset would silently hand back
    // the NEXT section's bytes, which reads as real data and is not.
    let zero_filled = pe
        .sections
        .iter()
        .find(|s| s.virtual_size > s.raw_size && s.raw_size > 0);
    if let Some(s) = zero_filled {
        assert_eq!(pe.rva_to_offset(s.virtual_address + s.raw_size), None, "section {}", s.name);
    }
}

#[test]
#[ignore = "needs the installed CrimsonDesert.exe"]
fn image_layout_matches_the_loader() {
    let data = std::fs::read(exe().unwrap()).unwrap();
    let pe = Pe::parse(&data).unwrap();
    let img = pe.image();

    assert_eq!(img.len(), pe.size_of_image as usize);
    // Every section's bytes land at its virtual address. This is what makes a
    // disassembler's addresses line up with RVAs, which is the whole reason
    // `dis` builds an image copy at all.
    for s in &pe.sections {
        if s.raw_size == 0 {
            continue;
        }
        let n = (s.raw_size as usize).min(1 << 16);
        let src = s.raw_offset as usize;
        let dst = s.virtual_address as usize;
        assert_eq!(&img[dst..dst + n], &data[src..src + n], "section {}", s.name);
    }
    // The gap between the headers and the first section is zero, not stale.
    let first = pe.sections.iter().map(|s| s.virtual_address).min().unwrap() as usize;
    assert!(img[0x1000..first].iter().all(|&b| b == 0) || first <= 0x1000);
}
