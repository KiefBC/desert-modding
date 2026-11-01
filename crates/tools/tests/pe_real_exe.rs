//! `desert_tools::pe` against the shipped CrimsonDesert.exe.
//!
//! Every binary tool in here converts between file offsets and RVAs, and the
//! two are not the same number: `sigscan` prints offsets, `xrefs` takes an RVA,
//! and `crates/tools/README.md` warns that feeding one to the other looks like a bug
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

    // DO NOT select a section by its name. This exe's section names are
    // scrambled, and scrambled differently on every build. Build 26361232:
    //
    //   .sxdata .xtls .udata .debug$P .reloc .data .link .xtext .rdata .sdata
    //   .rodata .shared
    //
    // and the 89 MB first one holding the code is the one called `.sxdata`.
    // `.xtext` is the 200 MB one that is not where the functions start, and
    // the exception directory (data directory 3) is the whole of `.shared`.
    //
    // 26356046, the hotfix it replaced, was `.xtext .shared .sdata .didata
    // .trace .xcode .data .text .ecode .rsrc .tls .rdata`: the code in the
    // first section, then called `.xtext`, a `.text` that was the 200 MB one,
    // and the exception directory the whole of `.rdata`.
    //
    // 26345091, the build this retarget was derived on and which Steam
    // replaced the same day, was `.trace .xtls .reloc .arch .text1 .shared
    // .data2 .didata .debug .data1 .xtext .edata`: the same twelve sections at
    // nearly the same addresses, every name different, the code in `.trace`
    // and the exception directory in `.edata`. A hotfix rename is enough to
    // break a name-keyed lookup. Build 25477059 was `.rsrc .xdata .rdata
    // .00cfg .tls$ .tls .xtext .xtls .text .shared .impdata .trace`, code in
    // `.rsrc` and the exception directory all of `.trace`. Build 25246367 was
    // `.idata .arch .debug$P .tls$ .shared .xcode .00cfg .sbss .text1 .trace
    // .xtls .link`, code in `.idata`, the exception directory in `.link`, and
    // no `.text` at all: an earlier version of this test asserted a `.text`
    // existed and failed against that build; asserting one did NOT exist
    // failed against the next.
    //
    // The lesson generalises past this assert: nothing in crates/tools/ may select a
    // section by NAME. Match on the address range, which is what the section
    // table actually means, and find the exception directory through the data
    // directory, which is what `Pe::exception_directory` does.
    let names: Vec<&str> = pe.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            ".sxdata", ".xtls", ".udata", ".debug$P", ".reloc", ".data", ".link", ".xtext", ".rdata",
            ".sdata", ".rodata", ".shared",
        ],
        "the section names moved again - a game update, not a parser bug"
    );
    // `get_pos` (RVA 0x19375C0 on 26361232, 0x1937610 on 26356046 as on
    // 26345091, see tests/dis_cli.rs) is code, in the section that happens to
    // be called `.sxdata` this build (`.xtext` on 26356046).
    let get_pos = 0x193_75C0;
    let holding = pe.section_of(get_pos).expect("get_pos is in no section");
    assert_eq!(holding.name, ".sxdata");
    assert_eq!(holding.virtual_address, 0x1000);

    // The exception directory, found by index and never by name.
    let x = pe.exception_directory().expect("the exe has an exception directory");
    assert_eq!((x.rva, x.size), (0x13D9_A000, 0x2D_8844));
    assert_eq!(pe.section_of(x.rva).map(|s| s.name.as_str()), Some(".shared"));
    assert_eq!(pe.size_of_image(), 0x1407_3000);
    assert_eq!(pe.timestamp(), 0x6AE6_E3E8);
    let funcs = desert_tools::funcs::FunctionTable::from_image(&pe.image(), x.rva, x.size).unwrap();
    assert_eq!((funcs.len(), funcs.chained), (248_667, 51_083));
    let (_, root) = funcs.containing(get_pos + 0x20).expect("get_pos has a RUNTIME_FUNCTION");
    assert_eq!(root.begin, get_pos);
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
