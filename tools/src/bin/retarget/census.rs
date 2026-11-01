//! `retarget census`: the accessor template, measured over the whole image.
//!
//! The gatherer and dispatch reach every static-info table through one
//! compiler template (`desert-core/src/gimmick.rs`, module docs): an
//! accessor that names the table, a slot load a little before it that puts the
//! table's manager in `rbx`, and a call to the record loader a little after.
//! A game update changes the codegen around all three, and the plugin's
//! patterns are only as good as the census they were written from. This is
//! that census, re-measured in a second: how many accessor heads in each
//! encoding, what the instruction after each head is, where the slot load
//! sits and which register its bounds check uses, what shape the loader call
//! takes and how far away it is, whether its target opens with the loader
//! prologue, and whether the name each site passes is a known table name.
//!
//! **The constants below mirror `desert-core/src/gimmick.rs`** (`ACCESSORS`'
//! heads, `A_DISP_AT`, `A_RIP_AT`, `PAT_SLOT`, `is_slot_cmp`, `SLOT_WINDOW`,
//! `LOADER_PROLOGUE`, `parse_loader_call`'s grammar and `parse_stream_fill`).
//! `tools/` does not
//! depend on `desert-core`, deliberately; the plugin-side real-exe test
//! (`desert-core/tests/gimmick_real.rs`) asserts the same numbers this
//! measures, so a drift between the two shows up as one of them failing.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use desert_tools::pattern::Pattern;

use crate::{hex_bytes, Exe};

/// The four accessor heads: `xor r9d,r9d` (two encodings), the table name
/// into `r8` (`lea` the string, or `mov` a cell holding its VA), then
/// `mov rdx,[rbx+0x10]`. 14 bytes each; what follows is the tail. The bool
/// is "indirect": the operand is a pointer cell, not the string.
pub const HEADS: [(&str, &str, bool); 4] = [
    ("45 33 C9 + lea r8", "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10", false),
    ("45 33 C9 + mov r8", "45 33 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10", true),
    ("45 31 C9 + lea r8", "45 31 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10", false),
    ("45 31 C9 + mov r8", "45 31 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10", true),
];
pub const A_HEAD_LEN: usize = 14;
const A_DISP_AT: usize = 6;
const A_RIP_AT: usize = 10;
/// How far back from a head the slot load is looked for.
pub const SLOT_WINDOW: u32 = 0x60;
/// `mov rbx,[rip+disp32]`: opcode bytes, disp32 at +3, RIP at +7.
const SLOT_OPCODE: [u8; 3] = [0x48, 0x8B, 0x1D];
const SLOT_LEN: usize = 7;
/// The record loader's first 12 bytes (`gimmick::LOADER_PROLOGUE`).
pub const LOADER_PROLOGUE: [u8; 12] = [0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56];

const R32: [&str; 8] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi"];

/// `cmp r32,[rbx+0x8]`: `3B`, ModRM mod 01 / rm rbx with any reg, disp8 8.
/// The same test as `gimmick::is_slot_cmp`.
pub fn is_slot_cmp(b: &[u8]) -> bool {
    matches!(b, [0x3B, m, 0x08, ..] if m & 0xC7 == 0x43)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tail {
    /// `lea rcx,[rsp+d8]`, the codegen up to build 25477059.
    RspDisp8(u8),
    /// `lea rcx,[r11+d8]`, the frame-pointer codegen of 26345091.
    R11Disp8(u8),
    /// Anything else, with its first bytes: the early warning.
    Other(String),
}

impl Tail {
    pub fn label(&self) -> String {
        match self {
            Tail::RspDisp8(d) => format!("lea rcx,[rsp+{d:#x}]"),
            Tail::R11Disp8(d) => format!("lea rcx,[r11+{d:#x}]"),
            Tail::Other(b) => format!("other: {b}"),
        }
    }

    pub fn class(&self) -> &'static str {
        match self {
            Tail::RspDisp8(_) => "lea rcx,[rsp+d8]",
            Tail::R11Disp8(_) => "lea rcx,[r11+d8]",
            Tail::Other(_) => "other",
        }
    }
}

/// The tail after a head, and the offset of the accessor's own `E8` from the
/// head start, when the tail is one of the two known shapes.
pub fn parse_tail(after_head: &[u8]) -> (Tail, Option<usize>) {
    match after_head {
        [0x48, 0x8D, 0x4C, 0x24, d, 0xE8, ..] => (Tail::RspDisp8(*d), Some(A_HEAD_LEN + 5)),
        [0x49, 0x8D, 0x4B, d, 0xE8, ..] => (Tail::R11Disp8(*d), Some(A_HEAD_LEN + 4)),
        other => (Tail::Other(hex_bytes(&other[..other.len().min(6)])), None),
    }
}

/// What sits between `lea r9,[rsp+0x30]` and `lea rdx,[rsp+..]`: the index
/// load into `r8d`, if the site has one (`gimmick::IndexLoad`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexLoad {
    /// Nothing: `r8d` already holds the index.
    None,
    /// `44 0F B7 C0+r`: `movzx r8d,r16` (the register name given).
    Movzx(&'static str),
    /// `44 8B C0+r`: `mov r8d,r32`, the tables indexed by a full `u32`.
    Mov(&'static str),
}

/// The shape of one loader call: the index load and the `lea rdx` width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Shape {
    pub index: IndexLoad,
    /// `lea rdx,[rsp+disp32]` (`48 8D 94 24`) rather than disp8 (`48 8D 54 24`).
    pub rdx_disp32: bool,
}

impl Shape {
    pub fn label(self) -> String {
        let idx = match self.index {
            IndexLoad::None => "no index load".to_string(),
            IndexLoad::Movzx(r) => format!("movzx r8d,{r}"),
            IndexLoad::Mov(r) => format!("mov r8d,{r}"),
        };
        format!("{idx}; lea rdx disp{}", if self.rdx_disp32 { 32 } else { 8 })
    }
}

const R16: [&str; 8] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di"];

/// A parsed loader call: its shape, the `mov rcx,rbx` encoding, and where the
/// `E8` is from the grammar start. The same grammar as
/// `gimmick::parse_loader_call`:
///
/// ```text
/// 4C 8D 4C 24 30                 lea r9,[rsp+0x30]
/// [44 0F B7 C0+r | 44 8B C0+r]   optional index load into r8d
/// 48 8D 54 24 d8 | 48 8D 94 24 d32
/// 48 8B CB | 48 89 D9            mov rcx,rbx
/// E8                             call
/// ```
pub fn parse_loader_call(b: &[u8]) -> Option<(Shape, &'static str, usize)> {
    let rest = b.strip_prefix(&[0x4C, 0x8D, 0x4C, 0x24, 0x30])?;
    let (index, rest) = match rest {
        [0x44, 0x0F, 0xB7, m, r @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Movzx(R16[(m & 7) as usize]), r),
        [0x44, 0x8B, m, r @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Mov(R32[(m & 7) as usize]), r),
        _ => (IndexLoad::None, rest),
    };
    let (rdx_disp32, rest) = match rest {
        [0x48, 0x8D, 0x54, 0x24, _, r @ ..] => (false, r),
        [0x48, 0x8D, 0x94, 0x24, _, _, _, _, r @ ..] => (true, r),
        _ => return None,
    };
    let (mov, rest) = match rest {
        [0x48, 0x8B, 0xCB, r @ ..] => ("8B CB", r),
        [0x48, 0x89, 0xD9, r @ ..] => ("89 D9", r),
        _ => return None,
    };
    if rest.len() < 5 || rest[0] != 0xE8 {
        return None;
    }
    Some((Shape { index, rdx_disp32 }, mov, b.len() - rest.len()))
}

/// Where the loader's stream object keeps the table body's buffer pointer,
/// its size and the cursor (`gimmick::StreamLayout`): offsets from the
/// object's start, which is the loader call's `lea r9,[rsp+d8]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamLayout {
    pub buf: u32,
    pub size: u32,
    pub cursor: u32,
}

impl StreamLayout {
    pub fn label(&self) -> String {
        format!("buffer +{:#x} size +{:#x} cursor +{:#x}", self.buf, self.size, self.cursor)
    }
}

/// `mov rax,[base]`: `48`/`49`, `8B`, ModRM mod 00 reg rax, no SIB or RIP.
/// The same decode as `gimmick.rs`'s `holder_load`.
fn holder_load(b: &[u8]) -> Option<((u8, u8), usize)> {
    match b {
        [rex @ (0x48 | 0x49), 0x8B, m, ..] if m & 0xF8 == 0x00 && !matches!(m & 7, 4 | 5) => Some(((rex & 1, m & 7), 3)),
        _ => None,
    }
}

/// `mov eax,[base+d8]` or `mov eax,[base+index+d8]` with an optional REX
/// without W. The same decode as `gimmick.rs`'s `dword_load`.
fn dword_load(b: &[u8]) -> Option<((u8, u8), u8, usize)> {
    let (rex, rest) = match b {
        [r @ 0x40..=0x47, rest @ ..] => (*r, rest),
        _ => (0, b),
    };
    let pre = b.len() - rest.len();
    match rest {
        [0x8B, 0x44, sib, d, ..] => Some(((rex & 1, sib & 7), *d, pre + 4)),
        [0x8B, m, d, ..] if m & 0xC0 == 0x40 && m & 0x38 == 0 && m & 7 != 4 => Some(((rex & 1, m & 7), *d, pre + 3)),
        _ => None,
    }
}

/// `mov [rsp+d8],rax` when `wide`, else `mov [rsp+d8],eax`: its disp8.
fn rsp_store(b: &[u8], wide: bool) -> Option<u8> {
    match (wide, b) {
        (true, [0x48, 0x89, 0x44, 0x24, d, ..]) | (false, [0x89, 0x44, 0x24, d, ..]) => Some(*d),
        _ => None,
    }
}

/// The stream layout an accessor's fill sequence encodes, from the bytes
/// between its file-load call and the loader call, relative to the loader
/// call's `lea r9` displacement. **Mirrors `gimmick::parse_stream_fill` in
/// `desert-core/src/gimmick.rs`** (whose doc comment lists the instruction
/// forms accepted and why); the plugin-side census in
/// `desert-core/tests/gimmick_real.rs` asserts the same answer.
pub fn parse_stream_fill(code: &[u8], r9_disp: u8) -> Option<StreamLayout> {
    let field = |d: u8| (d < 0x80 && d >= r9_disp).then(|| u32::from(d - r9_disp));
    let tail = |i: usize| code.get(i..).unwrap_or_default();
    let mut buf = None;
    for i in 0..code.len() {
        if let Some((reg, n)) = holder_load(tail(i)) {
            if let Some(d) = rsp_store(tail(i + n), true) {
                if buf.is_some() {
                    return None;
                }
                buf = Some((reg, d));
            }
        }
    }
    let (holder, buf_d) = buf?;
    // Pairs keyed by store position: a disp8 of 0x40..0x47 before a load
    // decodes as a REX too, and the later start wins.
    let mut pairs: Vec<(usize, (u8, u8), u8, u8)> = Vec::new();
    for i in 0..code.len() {
        let Some((reg, disp, n)) = dword_load(tail(i)) else { continue };
        let Some(d) = rsp_store(tail(i + n), false) else { continue };
        match pairs.last_mut() {
            Some(last) if last.0 == i + n => *last = (i + n, reg, disp, d),
            _ => pairs.push((i + n, reg, disp, d)),
        }
    }
    let (sizes, cursors): (Vec<_>, Vec<_>) = pairs.iter().partition(|&&(_, reg, disp, _)| reg == holder && disp == 0x0C);
    match (sizes.as_slice(), cursors.as_slice()) {
        ([(_, _, _, s)], [(_, _, _, c)]) => Some(StreamLayout { buf: field(buf_d)?, size: field(*s)?, cursor: field(*c)? }),
        _ => None,
    }
}

/// Lower case with a trailing `info` removed: the key under which a table
/// name an accessor passes (`gimmickinfo`, `Skill`, `FactionNode`,
/// `statchangeInfo`) and the class name in `static-info-names.txt`
/// (`GimmickInfo`, `SkillInfo`, `FactionNodeInfo`, `StatChangeInfo`) are the
/// same name.
pub fn name_key(s: &str) -> String {
    let l = s.to_ascii_lowercase();
    l.strip_suffix("info").unwrap_or(&l).to_string()
}

#[derive(Debug, Clone)]
pub struct Slot {
    pub at: u32,
    /// Head minus slot load.
    pub dist: u32,
    /// The `cmp r32,[rbx+8]` register.
    pub reg: &'static str,
    /// The global slot the load reads.
    pub target: u32,
    /// Slot loads with the bounds check in the window; more than one means
    /// the nearest was taken.
    pub candidates: usize,
}

#[derive(Debug, Clone)]
pub struct LoaderCall {
    /// Where the grammar starts (`lea r9,[rsp+0x30]`).
    pub at: u32,
    pub shape: Shape,
    /// `8B CB` or `89 D9`, the two encodings of `mov rcx,rbx`.
    pub mov: &'static str,
    /// The `E8`.
    pub call_at: u32,
    pub target: u32,
    pub prologue_ok: bool,
}

#[derive(Debug, Clone)]
pub struct Site {
    pub head: usize,
    pub at: u32,
    pub tail: Tail,
    /// The accessor's own `E8`, right after the tail.
    pub call_at: Option<u32>,
    pub slot: Option<Slot>,
    pub loader: Option<LoaderCall>,
    /// The stream layout the fill sequence before the loader call encodes,
    /// when there is a loader call and its fill parses.
    pub stream: Option<StreamLayout>,
    pub name: Option<String>,
    pub known: bool,
}

impl Site {
    /// Loader-call grammar start minus the accessor's call: the distance
    /// `B_WINDOW` has to cover when the search starts at the call.
    pub fn loader_dist(&self) -> Option<u32> {
        Some(self.loader.as_ref()?.at - self.call_at?)
    }
}

pub struct Census {
    pub sites: Vec<Site>,
    pub b_window: usize,
    /// Names in the list file.
    pub names_total: usize,
    /// Names in the list file that no accessor site passes.
    pub names_unseen: Vec<String>,
}

impl Census {
    pub fn head_counts(&self) -> [usize; 4] {
        let mut c = [0; 4];
        for s in &self.sites {
            c[s.head] += 1;
        }
        c
    }

    pub fn tails(&self) -> BTreeMap<&'static str, usize> {
        let mut m = BTreeMap::new();
        for s in &self.sites {
            *m.entry(s.tail.class()).or_insert(0) += 1;
        }
        m
    }

    pub fn rejected_tails(&self) -> Vec<&Site> {
        self.sites.iter().filter(|s| matches!(s.tail, Tail::Other(_))).collect()
    }

    pub fn slot_hist(&self) -> BTreeMap<u32, usize> {
        let mut m = BTreeMap::new();
        for s in self.sites.iter().filter_map(|s| s.slot.as_ref()) {
            *m.entry(s.dist).or_insert(0) += 1;
        }
        m
    }

    pub fn slot_regs(&self) -> BTreeMap<&'static str, usize> {
        let mut m = BTreeMap::new();
        for s in self.sites.iter().filter_map(|s| s.slot.as_ref()) {
            *m.entry(s.reg).or_insert(0) += 1;
        }
        m
    }

    pub fn slots_found(&self) -> usize {
        self.sites.iter().filter(|s| s.slot.is_some()).count()
    }

    pub fn slot_ties(&self) -> usize {
        self.sites.iter().filter(|s| s.slot.as_ref().is_some_and(|x| x.candidates > 1)).count()
    }

    pub fn loader_shapes(&self) -> BTreeMap<(String, &'static str), usize> {
        let mut m = BTreeMap::new();
        for l in self.sites.iter().filter_map(|s| s.loader.as_ref()) {
            *m.entry((l.shape.label(), l.mov)).or_insert(0) += 1;
        }
        m
    }

    pub fn loaders_found(&self) -> usize {
        self.sites.iter().filter(|s| s.loader.is_some()).count()
    }

    pub fn loaders_with_prologue(&self) -> usize {
        self.sites.iter().filter(|s| s.loader.as_ref().is_some_and(|l| l.prologue_ok)).count()
    }

    /// Min and max of the loader-call distance from the accessor's call.
    pub fn loader_dist_range(&self) -> Option<(u32, u32)> {
        let d: Vec<u32> = self.sites.iter().filter_map(|s| s.loader_dist()).collect();
        Some((*d.iter().min()?, *d.iter().max()?))
    }

    pub fn loader_dist_hist(&self) -> BTreeMap<u32, usize> {
        let mut m = BTreeMap::new();
        for d in self.sites.iter().filter_map(|s| s.loader_dist()) {
            *m.entry(d).or_insert(0) += 1;
        }
        m
    }

    /// The `B_WINDOW` the plugin should use: the largest distance rounded up
    /// to 0x10, plus 0x40 of slack, as `gimmick.rs` documents it.
    pub fn suggested_b_window(&self) -> Option<usize> {
        let (_, max) = self.loader_dist_range()?;
        Some((max as usize).div_ceil(0x10) * 0x10 + 0x40)
    }

    pub fn names_seen(&self) -> BTreeSet<&str> {
        self.sites.iter().filter_map(|s| s.name.as_deref()).collect()
    }

    pub fn unknown_names(&self) -> Vec<&Site> {
        self.sites.iter().filter(|s| !s.known).collect()
    }

    /// Sites per parsed stream layout.
    pub fn stream_hist(&self) -> BTreeMap<StreamLayout, usize> {
        let mut m = BTreeMap::new();
        for l in self.sites.iter().filter_map(|s| s.stream) {
            *m.entry(l).or_insert(0) += 1;
        }
        m
    }

    /// Sites with a loader call whose stream fill does not parse.
    pub fn streams_unparsed(&self) -> Vec<&Site> {
        self.sites.iter().filter(|s| s.loader.is_some() && s.stream.is_none()).collect()
    }

    /// `stream fill: buffer +0x20 size +0x28 cursor +0x2c at 154 of 154 sites`,
    /// with every other layout and the unparsed count after it.
    pub fn stream_summary(&self) -> String {
        let h = self.stream_hist();
        let mut v: Vec<(&StreamLayout, &usize)> = h.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        let n = self.sites.len();
        let mut parts: Vec<String> = v.iter().map(|(l, c)| format!("{} at {c} of {n} sites", l.label())).collect();
        if parts.is_empty() {
            parts.push(format!("none parsed at {n} sites"));
        }
        let bad = self.streams_unparsed().len();
        let none = self.sites.iter().filter(|s| s.loader.is_none()).count();
        let mut s = format!("stream fill: {}", parts.join("; "));
        if bad > 0 {
            s.push_str(&format!(" ({bad} site(s) with a loader call: no fill parsed)"));
        }
        if none > 0 {
            s.push_str(&format!(" ({none} site(s) without a loader call)"));
        }
        s
    }

    pub fn site_named(&self, name: &str) -> Option<&Site> {
        self.sites.iter().find(|s| s.name.as_deref() == Some(name))
    }
}

/// One name per line, `#` lines and blank lines ignored, as `sigscan` reads it.
pub fn read_names(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read the name list {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

fn i32_at(b: &[u8], at: usize) -> Option<i32> {
    let b = b.get(at..at + 4)?;
    Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    let b = b.get(at..at + 8)?;
    Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

/// The RVA `rip + disp32` names, for an instruction whose next byte is `rip`.
fn rip_target(at: u32, rip_at: usize, disp: i32) -> u32 {
    (at as i64 + rip_at as i64 + disp as i64) as u32
}

/// The table name an accessor at `at` passes in `r8`.
fn site_name(exe: &Exe, at: u32, indirect: bool) -> Option<String> {
    let disp = i32_at(&exe.image, at as usize + A_DISP_AT)?;
    let target = rip_target(at, A_RIP_AT, disp);
    if !indirect {
        return exe.cstr(target);
    }
    let va = u64_at(&exe.image, target as usize)?;
    let rva = va.checked_sub(exe.base)?;
    exe.cstr(u32::try_from(rva).ok()?)
}

/// The nearest `mov rbx,[rip+slot]; cmp r32,[rbx+8]` before `head`, within
/// the window, and how many there were.
fn find_slot(img: &[u8], head: u32) -> Option<Slot> {
    let lo = head.saturating_sub(SLOT_WINDOW) as usize;
    let mut found: Vec<(u32, &'static str, u32)> = Vec::new();
    for p in lo..head as usize {
        let Some(w) = img.get(p..p + SLOT_LEN + 3) else { break };
        if w[..3] != SLOT_OPCODE || !is_slot_cmp(&w[SLOT_LEN..]) {
            continue;
        }
        let disp = i32_at(img, p + 3)?;
        let reg = R32[((w[SLOT_LEN + 1] >> 3) & 7) as usize];
        found.push((p as u32, reg, rip_target(p as u32, SLOT_LEN, disp)));
    }
    let candidates = found.len();
    let (at, reg, target) = found.pop()?;
    Some(Slot { at, dist: head - at, reg, target, candidates })
}

/// The first loader call starting in `[from, from + window)`, stopping at
/// `stop` (the next accessor head) if that comes first.
fn find_loader(img: &[u8], from: u32, window: usize, stop: Option<u32>) -> Option<LoaderCall> {
    let end = (from as usize + window).min(img.len());
    let end = match stop {
        Some(s) => end.min(s as usize),
        None => end,
    };
    for p in from as usize..end {
        if img[p] != 0x4C {
            continue;
        }
        let w = &img[p..(p + 32).min(img.len())];
        let Some((shape, mov, call_off)) = parse_loader_call(w) else { continue };
        let call_at = p + call_off;
        let rel = i32_at(img, call_at + 1)?;
        let target = rip_target(call_at as u32, 5, rel);
        let prologue_ok = img.get(target as usize..target as usize + LOADER_PROLOGUE.len()) == Some(&LOADER_PROLOGUE[..]);
        return Some(LoaderCall { at: p as u32, shape, mov, call_at: call_at as u32, target, prologue_ok });
    }
    None
}

pub fn measure(exe: &Exe, b_window: usize, names_file: &Path) -> Result<Census> {
    let names = read_names(names_file)?;
    let known: BTreeSet<String> = names.iter().map(|n| name_key(n)).collect();
    let img = &exe.image;

    let mut heads: Vec<(usize, u32)> = Vec::new();
    for (i, (_, pat, _)) in HEADS.iter().enumerate() {
        let p = Pattern::parse(pat).context("a HEADS pattern does not parse")?;
        heads.extend(p.find_all_overlapping(img).into_iter().map(|at| (i, at as u32)));
    }
    heads.sort_by_key(|&(_, at)| at);

    let mut sites = Vec::with_capacity(heads.len());
    for (n, &(head, at)) in heads.iter().enumerate() {
        let after = &img[(at as usize + A_HEAD_LEN).min(img.len())..];
        let (tail, call_off) = parse_tail(after);
        let call_at = call_off.map(|o| at + o as u32);
        let next_head = heads.get(n + 1).map(|&(_, a)| a);
        let from = call_at.unwrap_or(at + A_HEAD_LEN as u32);
        let loader = find_loader(img, from, b_window, next_head);
        // The fill runs from the end of the accessor's call (`E8` + rel32)
        // to the loader call's `lea r9,[rsp+d8]`, whose disp8 is at +4.
        let stream = match (&loader, call_at) {
            (Some(l), Some(c)) => img
                .get(c as usize + 5..l.at as usize)
                .zip(img.get(l.at as usize + 4))
                .and_then(|(code, &r9)| parse_stream_fill(code, r9)),
            _ => None,
        };
        let name = site_name(exe, at, HEADS[head].2);
        let is_known = name.as_deref().is_some_and(|n| known.contains(&name_key(n)));
        sites.push(Site {
            head,
            at,
            tail,
            call_at,
            slot: find_slot(img, at),
            loader,
            stream,
            name,
            known: is_known,
        });
    }

    let seen: BTreeSet<String> = sites.iter().filter_map(|s| s.name.as_deref()).map(name_key).collect();
    let names_unseen = names.iter().filter(|n| !seen.contains(&name_key(n))).cloned().collect();
    Ok(Census { sites, b_window, names_total: names.len(), names_unseen })
}

fn hist<K: std::fmt::Display>(m: &BTreeMap<K, usize>) -> String {
    m.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")
}

fn hist_hex(m: &BTreeMap<u32, usize>) -> String {
    m.iter().map(|(k, v)| format!("{k:#x}: {v}")).collect::<Vec<_>>().join(", ")
}

pub fn print_sites(out: &mut impl Write, c: &Census, tsv: bool) -> Result<()> {
    if tsv {
        writeln!(out, "rva\thead\ttail\tcall_at\tslot_at\tslot_dist\tslot_reg\tslot_target\tslot_candidates\tloader_at\tloader_dist\tloader_shape\tloader_mov\tloader_call\tloader_target\tprologue\tstream_buf\tstream_size\tstream_cursor\tname\tknown")?;
    }
    for s in &c.sites {
        let opt = |v: Option<u32>| v.map(|x| format!("{x:#x}")).unwrap_or_default();
        let (sa, sd, sr, st, sc) = match &s.slot {
            Some(x) => (format!("{:#x}", x.at), format!("{:#x}", x.dist), x.reg.to_string(), format!("{:#x}", x.target), x.candidates.to_string()),
            None => Default::default(),
        };
        let (la, ld, ls, lm, lc, lt, lp) = match &s.loader {
            Some(l) => (
                format!("{:#x}", l.at),
                opt(s.loader_dist()),
                l.shape.label(),
                l.mov.to_string(),
                format!("{:#x}", l.call_at),
                format!("{:#x}", l.target),
                if l.prologue_ok { "ok" } else { "NO" }.to_string(),
            ),
            None => Default::default(),
        };
        let name = s.name.clone().unwrap_or_else(|| "?".into());
        let (fb, fs, fc) = match &s.stream {
            Some(l) => (format!("{:#x}", l.buf), format!("{:#x}", l.size), format!("{:#x}", l.cursor)),
            None => Default::default(),
        };
        let fill = if fb.is_empty() { "-".to_string() } else { format!("{fb}/{fs}/{fc}") };
        if tsv {
            writeln!(
                out,
                "{:#x}\t{}\t{}\t{}\t{sa}\t{sd}\t{sr}\t{st}\t{sc}\t{la}\t{ld}\t{ls}\t{lm}\t{lc}\t{lt}\t{lp}\t{fb}\t{fs}\t{fc}\t{name}\t{}",
                s.at,
                s.head,
                s.tail.label(),
                opt(s.call_at),
                s.known
            )?;
        } else {
            writeln!(
                out,
                "  +{:#x} h{} {:<20} slot {:<5} {:<4} loader {:<5} {:<30} {:<5} {:<3} fill {:<14} {name}{}",
                s.at,
                s.head,
                s.tail.label(),
                if sd.is_empty() { "-" } else { &sd },
                if sr.is_empty() { "-" } else { &sr },
                if ld.is_empty() { "-" } else { &ld },
                if ls.is_empty() { "-" } else { &ls },
                if lm.is_empty() { "-" } else { &lm },
                if lp.is_empty() { "-" } else { &lp },
                fill,
                if s.known { "" } else { "  (NOT in the name list)" }
            )?;
        }
    }
    Ok(())
}

pub fn print(out: &mut impl Write, c: &Census, sites: bool, tsv: bool) -> Result<()> {
    if tsv {
        if sites {
            return print_sites(out, c, true);
        }
        writeln!(out, "key\tvalue")?;
        writeln!(out, "sites\t{}", c.sites.len())?;
        for (i, n) in c.head_counts().iter().enumerate() {
            writeln!(out, "head{i}\t{n}")?;
        }
        for (k, v) in c.tails() {
            writeln!(out, "tail\t{k}\t{v}")?;
        }
        for (k, v) in c.slot_hist() {
            writeln!(out, "slot_dist\t{k:#x}\t{v}")?;
        }
        for (k, v) in c.slot_regs() {
            writeln!(out, "slot_reg\t{k}\t{v}")?;
        }
        writeln!(out, "slots\t{}", c.slots_found())?;
        writeln!(out, "slot_ties\t{}", c.slot_ties())?;
        for ((shape, mov), v) in c.loader_shapes() {
            writeln!(out, "loader_shape\t{shape}\t{mov}\t{v}")?;
        }
        writeln!(out, "loaders\t{}", c.loaders_found())?;
        writeln!(out, "loaders_with_prologue\t{}", c.loaders_with_prologue())?;
        for (k, v) in c.loader_dist_hist() {
            writeln!(out, "loader_dist\t{k:#x}\t{v}")?;
        }
        if let Some(w) = c.suggested_b_window() {
            writeln!(out, "b_window_suggested\t{w:#x}")?;
        }
        for (l, v) in c.stream_hist() {
            writeln!(out, "stream_fill\t{:#x}\t{:#x}\t{:#x}\t{v}", l.buf, l.size, l.cursor)?;
        }
        writeln!(out, "stream_unparsed\t{}", c.streams_unparsed().len())?;
        writeln!(out, "names_total\t{}", c.names_total)?;
        writeln!(out, "names_seen\t{}", c.names_seen().len())?;
        writeln!(out, "names_unknown\t{}", c.unknown_names().len())?;
        for n in &c.names_unseen {
            writeln!(out, "name_unseen\t{n}")?;
        }
        return Ok(());
    }

    let hc = c.head_counts();
    writeln!(out, "census: {} accessor site(s), loader-call window {:#x}", c.sites.len(), c.b_window)?;
    for (i, (label, pat, _)) in HEADS.iter().enumerate() {
        writeln!(out, "  head {i} {label:<18} {:>4}  {pat}", hc[i])?;
    }
    writeln!(out, "  tails: {}", hist(&c.tails()))?;
    for s in c.rejected_tails() {
        writeln!(out, "    REJECTED tail at +{:#x}: {}", s.at, s.tail.label())?;
    }
    writeln!(
        out,
        "  slot loads: {}/{} found within {:#x} before the head; distances {}; cmp register {}{}",
        c.slots_found(),
        c.sites.len(),
        SLOT_WINDOW,
        hist_hex(&c.slot_hist()),
        hist(&c.slot_regs()),
        if c.slot_ties() > 0 { format!("; {} site(s) had more than one candidate", c.slot_ties()) } else { String::new() }
    )?;
    let shapes: Vec<String> = c
        .loader_shapes()
        .iter()
        .map(|((s, m), n)| format!("{s} / mov rcx,rbx {m}: {n}"))
        .collect();
    writeln!(
        out,
        "  loader calls: {}/{} found, {} with the loader prologue at the target; shapes {}",
        c.loaders_found(),
        c.sites.len(),
        c.loaders_with_prologue(),
        if shapes.is_empty() { "none".to_string() } else { shapes.join("; ") }
    )?;
    if let Some((lo, hi)) = c.loader_dist_range() {
        writeln!(
            out,
            "    distance from the accessor's call: {lo:#x}..{hi:#x} ({}); suggested B_WINDOW {:#x}",
            hist_hex(&c.loader_dist_hist()),
            c.suggested_b_window().unwrap_or(0)
        )?;
    }
    writeln!(out, "  {}", c.stream_summary())?;
    for s in c.streams_unparsed() {
        writeln!(out, "    no fill parsed: +{:#x} {}", s.at, s.name.clone().unwrap_or_else(|| "?".into()))?;
    }
    writeln!(
        out,
        "  names: {} distinct passed by the sites, {} in the list, {} site(s) pass a name not in the list, {} list name(s) passed by no site",
        c.names_seen().len(),
        c.names_total,
        c.unknown_names().len(),
        c.names_unseen.len()
    )?;
    for s in c.unknown_names() {
        writeln!(out, "    not in the list: +{:#x} passes {}", s.at, s.name.clone().unwrap_or_else(|| "(unreadable)".into()))?;
    }
    if !c.names_unseen.is_empty() {
        writeln!(out, "    unseen: {}", c.names_unseen.join(" "))?;
    }
    if let Some(g) = c.site_named("gimmickinfo") {
        writeln!(out, "  gimmickinfo: accessor +{:#x}, tail {}", g.at, g.tail.label())?;
        if let Some(s) = &g.slot {
            writeln!(out, "    slot load +{:#x} (head-{:#x}), slot +{:#x}, cmp {}", s.at, s.dist, s.target, s.reg)?;
        }
        if let Some(l) = &g.loader {
            writeln!(
                out,
                "    loader call +{:#x} ({}, call at +{:#x}, +{:#x} past the accessor's call) -> loader +{:#x}, prologue {}",
                l.at,
                l.shape.label(),
                l.call_at,
                g.loader_dist().unwrap_or(0),
                l.target,
                if l.prologue_ok { "ok" } else { "MISSING" }
            )?;
        }
        match &g.stream {
            Some(l) => writeln!(out, "    stream fill: {}", l.label())?,
            None => writeln!(out, "    stream fill: NOT PARSED")?,
        }
    } else {
        writeln!(out, "  gimmickinfo: NO accessor passes that name")?;
    }
    if sites {
        print_sites(out, c, false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_tails_parse_and_anything_else_is_rejected() {
        assert_eq!(parse_tail(&[0x48, 0x8D, 0x4C, 0x24, 0x70, 0xE8, 0, 0, 0, 0]), (Tail::RspDisp8(0x70), Some(19)));
        assert_eq!(parse_tail(&[0x49, 0x8D, 0x4B, 0x10, 0xE8, 0, 0, 0, 0]), (Tail::R11Disp8(0x10), Some(18)));
        let (t, c) = parse_tail(&[0x48, 0x8D, 0x4C, 0x24, 0x70, 0x90]);
        assert!(matches!(t, Tail::Other(_)), "a tail without the call is not a tail");
        assert_eq!(c, None);
        assert!(matches!(parse_tail(&[]).0, Tail::Other(_)));
    }

    #[test]
    fn the_slot_cmp_takes_any_register_against_rbx_plus_eight() {
        for m in [0x43u8, 0x4B, 0x53, 0x5B, 0x63, 0x6B, 0x73, 0x7B] {
            assert!(is_slot_cmp(&[0x3B, m, 0x08]), "{m:#x}");
        }
        assert!(!is_slot_cmp(&[0x3B, 0x7B, 0x10]));
        assert!(!is_slot_cmp(&[0x3B, 0x7A, 0x08]), "rm must be rbx");
        assert!(!is_slot_cmp(&[0x3B, 0xBB, 0x08]), "mod must be 01");
        assert!(!is_slot_cmp(&[0x39, 0x7B, 0x08]));
    }

    #[test]
    fn the_loader_call_grammar_covers_every_seen_shape() {
        let sh = |index, rdx_disp32| Shape { index, rdx_disp32 };
        let old = [0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x44, 0x0F, 0xB7, 0xC7, 0x48, 0x8D, 0x54, 0x24, 0x70, 0x48, 0x8B, 0xCB, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&old), Some((sh(IndexLoad::Movzx("di"), false), "8B CB", 17)));
        let new32 = [0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x48, 0x8D, 0x94, 0x24, 0x80, 0, 0, 0, 0x48, 0x89, 0xD9, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&new32), Some((sh(IndexLoad::None, true), "89 D9", 16)));
        let new8 = [0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x48, 0x8D, 0x54, 0x24, 0x70, 0x48, 0x8B, 0xCB, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&new8), Some((sh(IndexLoad::None, false), "8B CB", 13)));
        let mov = [0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x44, 0x8B, 0xC6, 0x48, 0x8D, 0x94, 0x24, 0x80, 0, 0, 0, 0x48, 0x8B, 0xCB, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&mov), Some((sh(IndexLoad::Mov("esi"), true), "8B CB", 19)));
        assert_eq!(sh(IndexLoad::Mov("esi"), true).label(), "mov r8d,esi; lea rdx disp32");
        // Cut before the call: not a loader call.
        assert_eq!(parse_loader_call(&new8[..14]), None);
        assert_eq!(parse_loader_call(&[0x90; 20]), None);
        // An index load into another register is not this grammar.
        let other = [0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x44, 0x8B, 0xCE, 0x48, 0x8D, 0x54, 0x24, 0x70, 0x48, 0x8B, 0xCB, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&other), None);
    }

    #[test]
    fn table_and_class_names_share_a_key() {
        assert_eq!(name_key("gimmickinfo"), name_key("GimmickInfo"));
        assert_eq!(name_key("Skill"), name_key("SkillInfo"));
        assert_eq!(name_key("statchangeInfo"), "statchange");
        assert_ne!(name_key("Faction"), name_key("FactionNode"));
    }

    #[test]
    fn the_nearest_slot_load_wins_and_ties_are_counted() {
        let mut img = vec![0x90u8; 0x100];
        let plant = |img: &mut Vec<u8>, at: usize, reg: u8| {
            img[at..at + 3].copy_from_slice(&SLOT_OPCODE);
            img[at + 7..at + 10].copy_from_slice(&[0x3B, reg, 0x08]);
        };
        plant(&mut img, 0x40, 0x7B);
        plant(&mut img, 0x60, 0x53);
        let s = find_slot(&img, 0x80).unwrap();
        assert_eq!((s.at, s.dist, s.reg, s.candidates), (0x60, 0x20, "edx", 2));
        // Outside the window: nothing.
        assert!(find_slot(&img, 0xF0).is_none());
    }

    #[test]
    fn the_loader_search_stops_at_the_next_head() {
        let mut img = vec![0x90u8; 0x200];
        let call = [0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x48, 0x8D, 0x54, 0x24, 0x70, 0x48, 0x8B, 0xCB, 0xE8, 0, 0, 0, 0];
        img[0x100..0x100 + call.len()].copy_from_slice(&call);
        img[0x11D..0x11D + LOADER_PROLOGUE.len()].copy_from_slice(&LOADER_PROLOGUE);
        let l = find_loader(&img, 0x80, 0x400, None).unwrap();
        assert_eq!((l.at, l.call_at, l.target, l.prologue_ok), (0x100, 0x10D, 0x112, false));
        assert!(find_loader(&img, 0x80, 0x400, Some(0xF0)).is_none(), "a head before it ends the search");
        assert!(find_loader(&img, 0x80, 0x40, None).is_none(), "the window is honoured");
    }

    #[test]
    fn b_window_is_the_rounded_maximum_plus_slack() {
        let c = Census { sites: Vec::new(), b_window: 0, names_total: 0, names_unseen: Vec::new() };
        assert_eq!(c.suggested_b_window(), None);
        let mut s = Site { head: 0, at: 0, tail: Tail::R11Disp8(0x10), call_at: Some(0x100), slot: None, loader: None, stream: None, name: None, known: false };
        s.loader = Some(LoaderCall {
            at: 0x17E,
            shape: Shape { index: IndexLoad::None, rdx_disp32: false },
            mov: "8B CB",
            call_at: 0x18B,
            target: 0,
            prologue_ok: true,
        });
        let c = Census { sites: vec![s], b_window: 0, names_total: 0, names_unseen: Vec::new() };
        assert_eq!(c.suggested_b_window(), Some(0xC0));
    }

    /// The `gimmickinfo` fill on 26361232, the same bytes
    /// `desert-core/src/gimmick.rs`'s unit test pins.
    const FILL_26361232: [u8; 33] = [
        0x48, 0x8B, 0x01, 0x48, 0x89, 0x44, 0x24, 0x50, 0x8B, 0x41, 0x0C, 0x89, 0x44, 0x24, 0x58, 0x44, 0x89, 0x74,
        0x24, 0x5C, 0x8B, 0x42, 0x04, 0x89, 0x44, 0x24, 0x5C, 0x90, 0x90, 0x90, 0x90, 0x90, 0x90,
    ];

    #[test]
    fn the_stream_fill_parses_like_the_plugin_side() {
        let new = StreamLayout { buf: 0x20, size: 0x28, cursor: 0x2C };
        assert_eq!(parse_stream_fill(&FILL_26361232, 0x30), Some(new));
        assert_eq!(new.label(), "buffer +0x20 size +0x28 cursor +0x2c");
        // Relative to the object's start, which is the lea r9 displacement.
        assert_eq!(parse_stream_fill(&FILL_26361232, 0x40), Some(StreamLayout { buf: 0x10, size: 0x18, cursor: 0x1C }));
        assert_eq!(parse_stream_fill(&FILL_26361232, 0x51), None);
        // The cursor through r9 and the holder in rdx (`NpcFriendlyInfo`).
        let npc = [
            0x48, 0x8B, 0x02, 0x48, 0x89, 0x44, 0x24, 0x50, 0x8B, 0x42, 0x0C, 0x89, 0x44, 0x24, 0x58, 0x41, 0x8B, 0x41,
            0x08, 0x89, 0x44, 0x24, 0x5C,
        ];
        assert_eq!(parse_stream_fill(&npc, 0x30), Some(new));
        // A second size store is ambiguous; no fill at all is nothing.
        let mut two = FILL_26361232.to_vec();
        two.extend_from_slice(&[0x8B, 0x41, 0x0C, 0x89, 0x44, 0x24, 0x60]);
        assert_eq!(parse_stream_fill(&two, 0x30), None);
        assert_eq!(parse_stream_fill(&[0x90; 40], 0x30), None);
    }
}
