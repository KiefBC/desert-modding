//! Function boundaries without a disassembler's auto-analysis.
//!
//! A 64-bit PE carries its own function table: the exception directory (data
//! directory 3) is an array of `RUNTIME_FUNCTION { begin, end, unwind_info }`
//! records, one per function or function fragment that has unwind data, sorted
//! by `begin`. That is how Windows unwinds a stack, so it has to be right, and
//! it is the most reliable "where does this function start" there is. Ghidra's
//! own analysis leans on it; `retarget` reads it directly in milliseconds.
//!
//! Two things make it less than a complete answer, and both are handled here:
//!
//! * **Chained entries.** The compiler splits hot and cold code; a cold
//!   fragment gets its own entry whose unwind info has `UNW_FLAG_CHAININFO`
//!   and names the parent's `RUNTIME_FUNCTION` after its unwind codes.
//!   [`FunctionTable::containing`] follows the chain to the root, so a
//!   reference inside a cold block is attributed to the function it belongs
//!   to, not to a nameless fragment.
//! * **Leaf functions** that touch neither the stack nor a non-volatile
//!   register need no unwind data and have no entry. For those,
//!   [`guess_start_heuristic`] scans back for padding or a `ret`/`jmp`
//!   followed by a known prologue shape. Its answer is marked as a guess
//!   ([`StartSource::Heuristic`]) everywhere it is printed.
//!
//! Nothing here finds the directory by a section name. This exe scrambles
//! them every build; see `tests/pe_real_exe.rs`.

use anyhow::{bail, Result};

use crate::pe::Pe;

/// `UNW_FLAG_CHAININFO`, in the top five bits of the unwind info's first byte.
const UNW_FLAG_CHAININFO: u8 = 0x4;

/// How far a chain is followed before it is called a loop. Real chains are
/// one or two links long.
const MAX_CHAIN: usize = 32;

/// How far back [`guess_start_heuristic`] looks before giving up.
pub const HEURISTIC_REACH: u32 = 0x4000;

/// One `RUNTIME_FUNCTION` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RuntimeFunction {
    pub begin: u32,
    pub end: u32,
    pub unwind: u32,
}

/// Where a function start came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartSource {
    /// The exception directory, chain-resolved.
    Pdata,
    /// The prologue-shape scan. A guess.
    Heuristic,
}

impl StartSource {
    pub fn label(self) -> &'static str {
        match self {
            StartSource::Pdata => "pdata",
            StartSource::Heuristic => "heuristic",
        }
    }
}

/// The exception directory, sorted by `begin`, with each entry's chain parent
/// resolved once up front.
#[derive(Debug, Clone, Default)]
pub struct FunctionTable {
    pub entries: Vec<RuntimeFunction>,
    /// `root[i]` is the index of the entry `entries[i]` chains up to, itself
    /// for a primary entry.
    root: Vec<usize>,
    /// How many entries are chained fragments rather than primaries.
    pub chained: usize,
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

impl FunctionTable {
    /// Read the table out of a laid-out image (RVA-indexed bytes), given the
    /// exception directory's RVA and size.
    pub fn from_image(image: &[u8], rva: u32, size: u32) -> Result<FunctionTable> {
        let start = rva as usize;
        let n = size as usize / 12;
        if start.checked_add(n * 12).is_none_or(|end| end > image.len()) {
            bail!("the exception directory {rva:#x}+{size:#x} runs past the image");
        }
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let at = start + i * 12;
            let e = RuntimeFunction {
                begin: u32_at(image, at).unwrap_or(0),
                end: u32_at(image, at + 4).unwrap_or(0),
                unwind: u32_at(image, at + 8).unwrap_or(0),
            };
            if e.begin == 0 && e.end == 0 {
                continue; // zero padding at the end of the directory
            }
            entries.push(e);
        }
        entries.sort_unstable();
        entries.dedup();

        let mut root: Vec<usize> = Vec::with_capacity(entries.len());
        let mut chained = 0;
        for i in 0..entries.len() {
            let mut cur = i;
            let mut hops = 0;
            while let Some(parent) = chain_parent(image, &entries[cur]) {
                let Some(p) = index_of(&entries, parent.begin) else { break };
                if p == cur || hops >= MAX_CHAIN {
                    break;
                }
                cur = p;
                hops += 1;
            }
            if cur != i {
                chained += 1;
            }
            root.push(cur);
        }
        Ok(FunctionTable { entries, root, chained })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry whose `[begin, end)` holds `rva`, and the root entry it
    /// chains up to (the same one for a primary entry).
    pub fn containing(&self, rva: u32) -> Option<(RuntimeFunction, RuntimeFunction)> {
        let i = self.entries.partition_point(|e| e.begin <= rva).checked_sub(1)?;
        let e = self.entries[i];
        (rva < e.end).then(|| (e, self.entries[self.root[i]]))
    }

    /// Whether some entry begins exactly at `rva`.
    pub fn is_begin(&self, rva: u32) -> bool {
        index_of(&self.entries, rva).is_some()
    }
}

fn index_of(entries: &[RuntimeFunction], begin: u32) -> Option<usize> {
    entries.binary_search_by_key(&begin, |e| e.begin).ok()
}

/// The parent `RUNTIME_FUNCTION` a chained entry's unwind info names.
///
/// The unwind info is `{ u8 version|flags, u8 prolog size, u8 code count,
/// u8 frame }` then `count` two-byte codes, padded to an even count, and with
/// `UNW_FLAG_CHAININFO` set a `RUNTIME_FUNCTION` follows them.
fn chain_parent(image: &[u8], e: &RuntimeFunction) -> Option<RuntimeFunction> {
    let at = e.unwind as usize;
    let head = image.get(at..at + 4)?;
    if head[0] >> 3 & UNW_FLAG_CHAININFO == 0 {
        return None;
    }
    let codes = (head[2] as usize + 1) & !1;
    let p = at + 4 + codes * 2;
    Some(RuntimeFunction {
        begin: u32_at(image, p)?,
        end: u32_at(image, p + 4)?,
        unwind: u32_at(image, p + 8)?,
    })
}

/// [`FunctionTable::from_image`] for a parsed PE. It lays out the whole image
/// to read the unwind infos by RVA, so a caller that already has the image
/// should call `from_image` instead.
pub fn pdata_functions(pe: &Pe) -> Result<FunctionTable> {
    let Some(dir) = pe.exception_directory() else {
        return Ok(FunctionTable::default());
    };
    FunctionTable::from_image(&pe.image(), dir.rva, dir.size)
}

/// Whether the bytes at `b` start with one of the prologue shapes this exe's
/// functions open with.
///
/// `mov r11,rsp` (the new build's frame-pointer codegen), `mov [rsp+d8],rbx`,
/// `push rbx`..`push rdi` with a REX byte, `sub rsp,imm8`, `push r12`..`r15`,
/// `mov rax,rsp`. Deliberately short: each one is common at a function entry
/// and rare right after padding anywhere else.
pub fn is_prologue(b: &[u8]) -> bool {
    matches!(
        b,
        [0x4C, 0x8B, 0xDC, ..]
            | [0x48, 0x89, 0x5C, 0x24, ..]
            | [0x40, 0x53..=0x57, ..]
            | [0x48, 0x83, 0xEC, ..]
            | [0x41, 0x54..=0x57, ..]
            | [0x48, 0x8B, 0xC4, ..]
    )
}

/// Scan back from `rva` for the nearest offset that follows padding (`CC` or
/// `90`) or a `ret` (`C3`) and starts with [`is_prologue`].
///
/// It is a guess, and it is printed as one. It can stop short at a prologue-
/// shaped run of bytes inside a function, and it cannot see a function that
/// starts with anything else; the exception directory is always asked first.
pub fn guess_start_heuristic(image: &[u8], rva: u32) -> Option<u32> {
    let lo = rva.saturating_sub(HEURISTIC_REACH).max(1);
    let mut p = rva.min(image.len().saturating_sub(1) as u32);
    while p >= lo {
        let prev = image[p as usize - 1];
        if matches!(prev, 0xCC | 0x90 | 0xC3) && is_prologue(&image[p as usize..]) {
            return Some(p);
        }
        p -= 1;
    }
    None
}

/// The start of the function holding `rva`: the exception directory's chain
/// root if an entry covers it, else the heuristic's guess.
pub fn function_start(table: &FunctionTable, image: &[u8], rva: u32) -> (Option<u32>, StartSource) {
    if let Some((_, root)) = table.containing(rva) {
        return (Some(root.begin), StartSource::Pdata);
    }
    (guess_start_heuristic(image, rva), StartSource::Heuristic)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(img: &mut [u8], at: usize, v: u32) {
        img[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// An image with three entries at 0x1000: a primary, a second primary,
    /// and a cold fragment of the first chained to it.
    fn image() -> Vec<u8> {
        let mut img = vec![0xCCu8; 0x4000];
        let dir = 0x3000;
        img[dir..dir + 48].fill(0);
        // Unwind infos: a plain one at 0x3800, a chained one at 0x3810 whose
        // three codes pad to four before the parent record.
        img[0x3800..0x3804].copy_from_slice(&[0x01, 0x04, 0x00, 0x00]);
        img[0x3810..0x3814].copy_from_slice(&[0x01 | (UNW_FLAG_CHAININFO << 3), 0x00, 0x03, 0x00]);
        let parent = 0x3814 + 4 * 2;
        put(&mut img, parent, 0x1000);
        put(&mut img, parent + 4, 0x1080);
        put(&mut img, parent + 8, 0x3800);
        for (i, (b, e, u)) in [(0x1000u32, 0x1080u32, 0x3800u32), (0x1100, 0x1180, 0x3800), (0x2000, 0x2040, 0x3810)]
            .into_iter()
            .enumerate()
        {
            put(&mut img, dir + i * 12, b);
            put(&mut img, dir + i * 12 + 4, e);
            put(&mut img, dir + i * 12 + 8, u);
        }
        img
    }

    #[test]
    fn entries_are_found_and_chains_resolve_to_their_root() {
        let img = image();
        let t = FunctionTable::from_image(&img, 0x3000, 36 + 12).unwrap();
        assert_eq!(t.len(), 3, "the trailing zero record is padding");
        assert_eq!(t.chained, 1);
        let (frag, root) = t.containing(0x2010).unwrap();
        assert_eq!((frag.begin, root.begin), (0x2000, 0x1000));
        let (e, r) = t.containing(0x1100).unwrap();
        assert_eq!((e.begin, r.begin), (0x1100, 0x1100));
        assert!(t.containing(0x1080).is_none(), "end is exclusive");
        assert!(t.containing(0x0FFF).is_none());
        assert!(t.is_begin(0x2000) && !t.is_begin(0x2001));
        assert_eq!(function_start(&t, &img, 0x2030), (Some(0x1000), StartSource::Pdata));
    }

    #[test]
    fn a_directory_past_the_image_is_an_error() {
        assert!(FunctionTable::from_image(&[0u8; 16], 8, 24).is_err());
    }

    #[test]
    fn the_heuristic_finds_a_padded_prologue_and_says_it_guessed() {
        let mut img = vec![0xCCu8; 0x200];
        img[0x100..0x103].copy_from_slice(&[0x4C, 0x8B, 0xDC]);
        img[0x103..0x140].fill(0x90 + 1); // body, no padding inside
        let t = FunctionTable::default();
        assert_eq!(function_start(&t, &img, 0x130), (Some(0x100), StartSource::Heuristic));
        // Nothing prologue-shaped behind it: no guess.
        let flat = vec![0x11u8; 0x200];
        assert_eq!(guess_start_heuristic(&flat, 0x100), None);
    }

    #[test]
    fn every_documented_prologue_shape_is_recognised() {
        for b in [
            &[0x4C, 0x8B, 0xDC][..],
            &[0x48, 0x89, 0x5C, 0x24, 0x08],
            &[0x40, 0x53],
            &[0x40, 0x57],
            &[0x48, 0x83, 0xEC, 0x28],
            &[0x41, 0x54],
            &[0x41, 0x57],
            &[0x48, 0x8B, 0xC4],
        ] {
            assert!(is_prologue(b), "{b:02X?}");
        }
        assert!(!is_prologue(&[0x40, 0x58]));
        assert!(!is_prologue(&[0x48, 0x8B, 0xC5]));
    }
}
