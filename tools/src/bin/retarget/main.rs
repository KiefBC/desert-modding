//! Re-target the plugin to a new game build without a disassembler project.
//!
//! ```text
//! retarget triage   [--sigs file]                 which signatures died, and what is left of them
//! retarget calls    <rva> <len> [--n 16] [--callers]   every E8/E9 target in a range
//! retarget callers  <rva>                         references grouped by enclosing function
//! retarget prologue <rva> [bytes] [--stolen K] [--tokens]   safe stolen-byte counts
//! retarget census   [--b-window 0x400] [--sites]  the accessor template, slot loads, loader calls
//! retarget pe                                     headers, sections, data directories
//! retarget report                                 triage + census + pe as a markdown checklist
//! ```
//!
//! Every subcommand takes `--exe` (default: `paths::game_exe()`, which honours
//! `EXE`) and `--tsv` for machine-readable rows. Addresses are printed as RVAs
//! and accepted as RVA hex (`3dd9a8`, `0x3dd9a8`) or as a FILE OFFSET with an
//! `f:` prefix (`f:29a8bc3`), which is what `sigscan` prints; the conversion
//! goes through the section table, and the output always says the RVA.
//!
//! This exists because a game update moves codegen, and re-deriving a
//! signature by hand with `xrefs` and `dis` is the same six steps every time:
//! find what is left of the old pattern, find the function it was in, read its
//! callees and callers, cut a new pattern and check it is unique, and pick a
//! stolen-byte count that is whole instructions with nothing RIP-relative.
//! Each subcommand is one of those steps, seconds per run, no Ghidra.
//!
//! Function boundaries come from the PE exception directory (data directory 3,
//! `desert_tools::funcs`), never from a section name, with a prologue-shape
//! heuristic as the fallback; every function start printed says which.

mod calls;
mod callers;
mod census;
#[cfg_attr(not(test), allow(dead_code))]
mod expect;
mod peinfo;
mod prologue;
mod report;
mod sigfile;
mod triage;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use desert_tools::funcs::{self, FunctionTable, StartSource};
use desert_tools::paths;
use desert_tools::pe::{self, DataDirectory, Pe, Section};

/// Every hook in the workspace writes a 12-byte `mov rax,imm64; jmp rax`, so
/// no stolen range may be shorter. `prologue` lists only boundaries at or past
/// this.
pub const MIN_STOLEN: usize = 12;

#[derive(Parser)]
#[command(about = "Re-target the plugin to a new game build: triage, census, call graph, prologues")]
struct Cli {
    #[command(flatten)]
    common: Common,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
pub struct Common {
    /// The exe to read (default: the Steam install, or $EXE / $CD_BIN64).
    #[arg(long, global = true)]
    exe: Option<PathBuf>,
    /// Tab-separated rows instead of the human report.
    #[arg(long, global = true)]
    tsv: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Per signature: hit count, and for a dead one what is left of it.
    Triage {
        /// The signature list (default: tools/reference-signatures.txt).
        #[arg(long)]
        sigs: Option<PathBuf>,
    },
    /// Every E8/E9 target in a range, with a pattern for each.
    Calls {
        rva: String,
        len: String,
        /// Bytes of each target to print and cut a pattern from.
        #[arg(long, default_value_t = 16)]
        n: usize,
        /// Also count each target's distinct calling functions (one more pass).
        #[arg(long)]
        callers: bool,
    },
    /// References to an address, grouped by the function they are in.
    Callers { rva: String },
    /// Whole instructions from an address, and the safe stolen-byte counts.
    Prologue {
        rva: String,
        /// How many bytes to decode (hex, default 0x30).
        bytes: Option<String>,
        /// Validate this stolen count (decimal, or 0x-prefixed hex) and print
        /// its bytes; exit 2 if unsafe.
        #[arg(long)]
        stolen: Option<String>,
        /// With --stolen: one commented line per instruction, for a const.
        #[arg(long)]
        tokens: bool,
    },
    /// The accessor-template census the gatherer and dispatch depend on.
    Census {
        /// How far past the accessor's call the loader call is looked for.
        #[arg(long, default_value = "0x400")]
        b_window: String,
        /// One row per accessor site.
        #[arg(long)]
        sites: bool,
        /// The static-info name list (default: tools/static-info-names.txt).
        #[arg(long)]
        names: Option<PathBuf>,
    },
    /// Image base, SizeOfImage, timestamp, sha256, sections, data directories.
    Pe,
    /// triage + census + pe as a markdown checklist.
    Report {
        #[arg(long)]
        sigs: Option<PathBuf>,
        #[arg(long)]
        names: Option<PathBuf>,
    },
}

/// The exe, read once and laid out as the loader would map it.
///
/// The file is dropped as soon as the image is built: holding both is 760 MB,
/// and the tests run several of these at once.
pub struct Exe {
    pub path: PathBuf,
    pub image: Vec<u8>,
    pub base: u64,
    pub size_of_image: u32,
    pub timestamp: u32,
    pub file_len: usize,
    pub sha256: Option<String>,
    pub sections: Vec<Section>,
    pub dirs: Vec<DataDirectory>,
    pub funcs: FunctionTable,
    /// IAT slot RVA -> `dll!name`.
    pub imports: HashMap<u32, String>,
}

impl Exe {
    pub fn load(path: PathBuf, want_sha: bool) -> Result<Exe> {
        let data = std::fs::read(&path)
            .with_context(|| format!("cannot read the game exe {}", path.display()))?;
        let pe = Pe::parse(&data).with_context(|| format!("{} is not a PE32+ image", path.display()))?;
        let mut image = pe.image();
        // The headers too, as `xrefs` lays them out: the loader maps them at 0.
        let hdr = pe.sections.iter().map(|s| s.raw_offset as usize).min().unwrap_or(0);
        let hdr = hdr.min(image.len()).min(data.len());
        image[..hdr].copy_from_slice(&data[..hdr]);
        let funcs = match pe.exception_directory() {
            Some(d) => FunctionTable::from_image(&image, d.rva, d.size)?,
            None => FunctionTable::default(),
        };
        let imports = pe::import_slots(&data).unwrap_or_default();
        let sha256 = want_sha.then(|| desert_tools::sha256::hex_digest(&data));
        Ok(Exe {
            path,
            base: pe.image_base,
            size_of_image: pe.size_of_image,
            timestamp: pe.timestamp,
            file_len: data.len(),
            sha256,
            sections: pe.sections.clone(),
            dirs: pe.data_directories.clone(),
            funcs,
            imports,
            image,
        })
    }

    /// The same conversion `Pe::offset_to_rva` makes, over the kept table.
    pub fn offset_to_rva(&self, off: usize) -> Option<u32> {
        let off = u32::try_from(off).ok()?;
        self.sections.iter().find_map(|s| {
            (s.raw_size != 0 && off >= s.raw_offset && off < s.raw_offset.checked_add(s.raw_size)?)
                .then(|| s.virtual_address + (off - s.raw_offset))
        })
    }

    /// The same conversion `Pe::rva_to_offset` makes.
    pub fn rva_to_offset(&self, rva: u32) -> Option<usize> {
        self.sections.iter().find_map(|s| {
            let end = s.virtual_address.checked_add(s.virtual_size.max(s.raw_size))?;
            if rva < s.virtual_address || rva >= end {
                return None;
            }
            let d = rva - s.virtual_address;
            (d < s.raw_size).then(|| (s.raw_offset + d) as usize)
        })
    }

    pub fn section_name(&self, rva: u32) -> &str {
        self.sections
            .iter()
            .find(|s| rva >= s.virtual_address && rva < s.virtual_address + s.virtual_size.max(s.raw_size))
            .map(|s| s.name.as_str())
            .unwrap_or("-")
    }

    /// An address from the command line: RVA hex, or `f:` and a file offset.
    pub fn addr(&self, text: &str) -> Result<u32> {
        if let Some(off) = text.strip_prefix("f:") {
            let off = parse_hex(off).with_context(|| format!("`{text}` is not a hex file offset"))?;
            return self
                .offset_to_rva(off as usize)
                .with_context(|| format!("file offset {off:#x} is in no section's raw bytes"));
        }
        let rva = parse_hex(text).with_context(|| format!("`{text}` is not a hexadecimal RVA"))?;
        let rva = u32::try_from(rva).with_context(|| format!("{rva:#x} is not a 32-bit RVA"))?;
        if rva as usize >= self.image.len() {
            bail!("RVA {rva:#x} is past the end of the image ({:#x})", self.image.len());
        }
        Ok(rva)
    }

    /// The start of the function holding `rva`, and where that answer came from.
    pub fn function_start(&self, rva: u32) -> (Option<u32>, StartSource) {
        funcs::function_start(&self.funcs, &self.image, rva)
    }

    /// `pdata start`, `pdata +0x1c in 0x...`, `no pdata`: what the exception
    /// directory says about an address.
    pub fn pdata_status(&self, rva: u32) -> String {
        match self.funcs.containing(rva) {
            Some((e, root)) if e.begin == rva && root.begin == rva => "pdata start".to_string(),
            Some((e, root)) if e.begin == rva => format!("pdata fragment of {:#x}", root.begin),
            Some((_, root)) => format!("inside {:#x}+{:#x}", root.begin, rva - root.begin),
            None => "no pdata".to_string(),
        }
    }

    pub fn bytes(&self, rva: u32, n: usize) -> &[u8] {
        let s = (rva as usize).min(self.image.len());
        &self.image[s..(s + n).min(self.image.len())]
    }

    /// A NUL-terminated ASCII string at `rva`, at most 256 bytes.
    pub fn cstr(&self, rva: u32) -> Option<String> {
        let b = self.bytes(rva, 256);
        let n = b.iter().position(|&c| c == 0)?;
        let s = &b[..n];
        (!s.is_empty() && s.iter().all(|c| c.is_ascii_graphic() || *c == b' '))
            .then(|| String::from_utf8_lossy(s).into_owned())
    }
}

/// Hex with or without `0x`.
pub fn parse_hex(s: &str) -> Result<u64> {
    let t = s.trim_start_matches("0x").trim_start_matches("0X");
    u64::from_str_radix(t, 16).with_context(|| format!("`{s}` is not hexadecimal"))
}

/// A byte count the way the `*_STOLEN` constants are written: decimal, or hex
/// with a `0x` prefix. (`--stolen 12` means twelve, not 0x12.)
pub fn parse_count(s: &str) -> Result<usize> {
    let n = match s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        Some(h) => usize::from_str_radix(h, 16),
        None => s.parse(),
    };
    n.with_context(|| format!("`{s}` is not a byte count"))
}

/// `48 8B 53 10`, upper case, space separated.
pub fn hex_bytes(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

/// `tools/` in the checkout, the way `sigscan` finds its data files.
pub fn tools_dir() -> PathBuf {
    paths::repo_root()
        .map(|r| r.join("tools"))
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("retarget: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    let path = cli.common.exe.clone().unwrap_or_else(paths::game_exe);
    let tsv = cli.common.tsv;
    let want_sha = matches!(cli.cmd, Cmd::Pe | Cmd::Report { .. });
    let exe = Exe::load(path, want_sha)?;
    let out = &mut std::io::stdout().lock();
    match cli.cmd {
        Cmd::Triage { sigs } => {
            let sigs = sigs.unwrap_or_else(|| tools_dir().join("reference-signatures.txt"));
            let t = triage::run(&exe, &sigs)?;
            triage::print(out, &exe, &t, tsv)?;
            Ok(if t.iter().all(|r| r.sig.optional || r.hits.len() == 1) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        Cmd::Calls { rva, len, n, callers } => {
            let rva = exe.addr(&rva)?;
            let len = parse_hex(&len)? as usize;
            calls::run(out, &exe, rva, len, n, callers, tsv)?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Callers { rva } => {
            let rva = exe.addr(&rva)?;
            callers::run(out, &exe, rva, tsv)?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Prologue { rva, bytes, stolen, tokens } => {
            let rva = exe.addr(&rva)?;
            let bytes = match bytes {
                Some(b) => parse_hex(&b)? as usize,
                None => 0x30,
            };
            let stolen = stolen.map(|s| parse_count(&s)).transpose()?;
            let ok = prologue::run(out, &exe, rva, bytes, stolen, tokens, tsv)?;
            Ok(if ok { ExitCode::SUCCESS } else { ExitCode::from(2) })
        }
        Cmd::Census { b_window, sites, names } => {
            let names = names.unwrap_or_else(|| tools_dir().join("static-info-names.txt"));
            let c = census::measure(&exe, parse_hex(&b_window)? as usize, &names)?;
            census::print(out, &c, sites, tsv)?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Pe => {
            peinfo::print(out, &exe, tsv)?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Report { sigs, names } => {
            let sigs = sigs.unwrap_or_else(|| tools_dir().join("reference-signatures.txt"));
            let names = names.unwrap_or_else(|| tools_dir().join("static-info-names.txt"));
            let clean = report::run(out, &exe, &sigs, &names)?;
            Ok(if clean { ExitCode::SUCCESS } else { ExitCode::from(1) })
        }
    }
}
