//! `retarget pe`: the headers, in one screen.
//!
//! Image base, SizeOfImage, the COFF timestamp, the file's sha256, the
//! section table, the sixteen data directories and the exception directory's
//! range and entry count. The last two are what a hand-rolled Python snippet
//! in CLAUDE.md used to print; the section names are here because this exe
//! scrambles them every build and the first question after an update is
//! "what are they called now".

use std::io::Write;

use anyhow::Result;

use desert_tools::pe::DATA_DIRECTORY_NAMES;

use crate::Exe;

pub fn print(out: &mut impl Write, exe: &Exe, tsv: bool) -> Result<()> {
    let sha = exe.sha256.clone().unwrap_or_default();
    let build = desert_tools::paths::build_id().unwrap_or_else(|| "?".into());
    let (xb, xe) = exe
        .funcs
        .entries
        .iter()
        .fold((u32::MAX, 0u32), |(b, e), f| (b.min(f.begin), e.max(f.end)));
    if tsv {
        writeln!(out, "key\tvalue")?;
        writeln!(out, "path\t{}", exe.path.display())?;
        writeln!(out, "build\t{build}")?;
        writeln!(out, "image_base\t{:#x}", exe.base)?;
        writeln!(out, "size_of_image\t{:#x}", exe.size_of_image)?;
        writeln!(out, "timestamp\t{:#x}", exe.timestamp)?;
        writeln!(out, "file_len\t{}", exe.file_len)?;
        writeln!(out, "sha256\t{sha}")?;
        for s in &exe.sections {
            writeln!(out, "section\t{}\t{:#x}\t{:#x}\t{:#x}\t{:#x}", s.name, s.virtual_address, s.virtual_size, s.raw_offset, s.raw_size)?;
        }
        for (i, d) in exe.dirs.iter().enumerate() {
            writeln!(out, "dir\t{}\t{:#x}\t{:#x}", DATA_DIRECTORY_NAMES.get(i).unwrap_or(&"?"), d.rva, d.size)?;
        }
        writeln!(out, "pdata_entries\t{}", exe.funcs.len())?;
        writeln!(out, "pdata_chained\t{}", exe.funcs.chained)?;
        if !exe.funcs.is_empty() {
            writeln!(out, "pdata_code_range\t{xb:#x}\t{xe:#x}")?;
        }
        return Ok(());
    }
    writeln!(out, "{}", exe.path.display())?;
    writeln!(out, "  build (appmanifest) {build}")?;
    writeln!(out, "  image base          {:#x}", exe.base)?;
    writeln!(out, "  SizeOfImage         {:#x}", exe.size_of_image)?;
    writeln!(out, "  timestamp           {:#x} ({})", exe.timestamp, exe.timestamp)?;
    writeln!(out, "  file size           {} bytes", exe.file_len)?;
    writeln!(out, "  sha256              {sha}")?;
    writeln!(out, "  sections ({}):", exe.sections.len())?;
    writeln!(out, "    {:<9} {:>10} {:>10} {:>10} {:>10}", "name", "rva", "vsize", "file", "rawsize")?;
    for s in &exe.sections {
        writeln!(out, "    {:<9} {:>#10x} {:>#10x} {:>#10x} {:>#10x}", s.name, s.virtual_address, s.virtual_size, s.raw_offset, s.raw_size)?;
    }
    writeln!(out, "  data directories:")?;
    for (i, d) in exe.dirs.iter().enumerate() {
        if d.rva == 0 && d.size == 0 {
            continue;
        }
        let name = DATA_DIRECTORY_NAMES.get(i).unwrap_or(&"?");
        let sect = exe.section_name(d.rva);
        writeln!(out, "    {i:>2} {name:<13} +{:#x} size {:#x}  ({sect})", d.rva, d.size)?;
    }
    match exe.dirs.get(desert_tools::pe::EXCEPTION_DIRECTORY) {
        Some(d) if d.rva != 0 => writeln!(
            out,
            "  exception directory: VA {:#x} - {:#x}, {} RUNTIME_FUNCTION entries ({} chained), code {xb:#x}..{xe:#x}",
            exe.base + d.rva as u64,
            exe.base + d.rva as u64 + d.size as u64 - 1,
            exe.funcs.len(),
            exe.funcs.chained
        )?,
        _ => writeln!(out, "  exception directory: none (function starts fall back to the prologue heuristic)")?,
    }
    Ok(())
}
