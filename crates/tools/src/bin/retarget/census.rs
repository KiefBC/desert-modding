//! `retarget census`: the static-info access chain, measured over the whole image.
//!
//! The gatherer and dispatch reach every static-info table through one
//! compiler template (`crates/desert-core/src/gimmick.rs`, module docs). Since
//! build 26636972 it is a chain of three functions:
//!
//! ```text
//! getter     sub rsp,0x28; movzx r8d,word [rcx]        (prologue varies; often inlined)
//!            mov rcx,[rip+slot]                         the table's manager
//!            cmp r8d,[rcx+0x28]                         record count
//!            mov rax,[rcx+0x20]; lea rdx,[r8+r8*2]      entries, 24 bytes each
//!            cmp qword [rax+rdx*8+8],0                  parsed object at entry+8
//!            ... E8 accessor                            only when not loaded yet
//! accessor   push/spill prologue, 0x30 or 0x31 bytes, then the HEAD:
//!            xor r9d,r9d; lea|mov r8,<name>; mov rdx,[rcx+8]
//!            lea rcx,[rbp+0x40]; E8 file-load           the TAIL
//!            ... the stream object filled rbp-relative (buffer, size, cursor)
//!            lea r9,[rbp-0x30]; movzx r8d,r14w; mov rdx,rdi; mov rcx,rsi; E8 loader
//! loader     the record loader (`FUN_1403856b0` on 25116796), hooked for gimmickinfo
//! ```
//!
//! A game update changes the codegen around all three, and the plugin's
//! patterns are only as good as the census they were written from. This is
//! that census, re-measured in seconds: every getter (its slot load, the
//! manager offsets its body uses, its prologue, how far its call is from the
//! slot load), every accessor (head encoding, offset into its function,
//! tail), every loader call (shape, distance from the head, the prologue at
//! its target), every stream fill, one slot per table name, and the names
//! against `static-info-names.txt`.
//!
//! History, for reading old findings notes: through 26480954 there was no
//! getter. The slot load `mov rbx,[rip+slot]; cmp r32,[rbx+8]` sat 0x33/0x3B
//! bytes before an accessor HEAD inside a larger function, the head ended
//! `mov rdx,[rbx+0x10]`, the tail was `lea rcx,[rsp+d8]` (through 25477059)
//! or `lea rcx,[r11+0x10]` (26345091 on), the loader call was
//! `lea r9,[rsp+0x30]; ...; mov rcx,rbx; E8`, the fill was rsp-relative, the
//! loader prologue 12 bytes, and the manager kept its count at +0x08, an
//! offsets table (8-byte entries) at +0x28 and an object-pointer array at
//! +0x58. 149 accessors through 25477059, 154 on 26345091..26480954.
//!
//! **The constants below mirror `crates/desert-core/src/gimmick.rs`** (the
//! accessor heads, the slot-load pattern, `LOADER_PROLOGUE`, the loader-call
//! grammar and `parse_stream_fill`) and `crates/desert-core/src/manager.rs`.
//! `crates/tools/` does not depend on `desert-core`, deliberately; the
//! plugin-side real-exe test (`crates/desert-core/tests/gimmick_real.rs`)
//! asserts the same numbers this measures, so a drift between the two shows up
//! as one of them failing.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};

use desert_tools::pattern::Pattern;

use crate::{hex_bytes, Exe};

/// The four accessor heads: `xor r9d,r9d` (two encodings), the table name
/// into `r8` (`lea` the string, or `mov` a cell holding its VA), then
/// `mov rdx,[rcx+0x8]` (the manager's file context). 14 bytes each; what
/// follows is the tail. The bool is "indirect": the operand is a pointer
/// cell, not the string. Only the `45 33 C9` pair occurs on 26636972; the
/// `45 31 C9` pair is kept because the compiler has emitted it before.
pub const HEADS: [(&str, &str, bool); 4] = [
    ("45 33 C9 + lea r8", "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 51 08", false),
    ("45 33 C9 + mov r8", "45 33 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 51 08", true),
    ("45 31 C9 + lea r8", "45 31 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 51 08", false),
    ("45 31 C9 + mov r8", "45 31 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 51 08", true),
];
pub const A_HEAD_LEN: usize = 14;
const A_DISP_AT: usize = 6;
const A_RIP_AT: usize = 10;
/// How far past a getter's call target the accessor head may sit (it is
/// 0x30 or 0x31 on 26636972).
pub const HEAD_REACH: u32 = 0x80;
/// How far past the slot load's bounds check the getter's call is looked for.
pub const CALL_WINDOW: usize = 0x60;
/// A getter whose function starts at most this far before its slot load is
/// a standalone getter; anything further is the getter inlined into a caller.
pub const STANDALONE_MAX: u32 = 0x30;
/// The record loader's first 15 bytes (`gimmick::LOADER_PROLOGUE`): three
/// whole 5-byte spills, nothing RIP-relative, no branch.
pub const LOADER_PROLOGUE: [u8; 15] =
    [0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4C, 0x24, 0x08];

const R64: [&str; 16] = ["rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15"];
const R32: [&str; 16] = ["eax", "ecx", "edx", "ebx", "esp", "ebp", "esi", "edi", "r8d", "r9d", "r10d", "r11d", "r12d", "r13d", "r14d", "r15d"];
const R16: [&str; 16] = ["ax", "cx", "dx", "bx", "sp", "bp", "si", "di", "r8w", "r9w", "r10w", "r11w", "r12w", "r13w", "r14w", "r15w"];
const R8: [&str; 16] = ["al", "cl", "dl", "bl", "spl", "bpl", "sil", "dil", "r8b", "r9b", "r10b", "r11b", "r12b", "r13b", "r14b", "r15b"];

fn reg(names: &'static [&'static str; 16], n: u8) -> &'static str {
    names[(n & 15) as usize]
}

/// A getter's slot load and bounds check:
/// `48 8B modrm(00 r 101) disp32` (`mov r64,[rip+slot]`) directly followed
/// by `[REX] 3B modrm(01 c r) d8` (`cmp r32,[r64+d8]`, base the same
/// register). Returns the slot register, the slot's disp32, the compared
/// register, the count displacement and the length of the pair.
pub fn parse_slot_load(b: &[u8]) -> Option<(u8, i32, u8, u8, usize)> {
    let [0x48, 0x8B, m, d0, d1, d2, d3, rest @ ..] = b else { return None };
    if m & 0xC7 != 0x05 {
        return None;
    }
    let base = (m >> 3) & 7;
    if base == 4 {
        return None;
    }
    let disp = i32::from_le_bytes([*d0, *d1, *d2, *d3]);
    let (rex, rest) = match rest {
        // REX without W and without B: a 32-bit compare against a legacy base.
        [r @ 0x40..=0x46, rest @ ..] if r & 0x01 == 0 => (*r, rest),
        _ => (0, rest),
    };
    match rest {
        [0x3B, c, d, ..] if c & 0xC7 == 0x40 | base => {
            let creg = ((rex >> 2) & 1) << 3 | (c >> 3) & 7;
            Some((base, disp, creg, *d, 7 + usize::from(rex != 0) + 3))
        }
        _ => None,
    }
}

/// The manager offsets a getter's body uses, from the bounds check to its
/// call: the record count's displacement (from the bounds check), the entries
/// pointer's (`mov r64,[slot_reg+d8]`, or `add r64,[slot_reg+d8]`), the entry
/// stride (`lea r,[x+x*k]` times the scale of the indexed access, or a
/// `shl r64,k`) and the parsed object's offset in an entry
/// (`cmp qword [..+d8],0`). Most tables have 24-byte entries with the object
/// at +8 on 26636972; four have 32-byte entries with the object at +0x10.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ManagerLayout {
    pub count: u8,
    pub entries: Option<u8>,
    pub stride: Option<u32>,
    pub object: Option<u8>,
}

impl ManagerLayout {
    pub fn label(&self) -> String {
        let o = |v: Option<u8>| v.map(|x| format!("+{x:#x}")).unwrap_or_else(|| "?".into());
        format!(
            "count +{:#x}, entries {}, stride {}, object {}",
            self.count,
            o(self.entries),
            self.stride.map(|s| s.to_string()).unwrap_or_else(|| "?".into()),
            o(self.object)
        )
    }
}

/// The entries load, stride and object offset in a getter body (the bytes
/// from the end of the bounds check to the call). Byte-scanned rather than
/// decoded, which the short window makes safe: the forms looked for are
/// `mov|add r64,[slot+d8]` (`48|4C 8B|03`, mod 01, rm the slot register),
/// `lea r,[x+x*s]` (index == base: the `*3`), `lea r,[b+i*s]` (index != base:
/// the scale), `shl r64,k` (`48|49 C1 /4`: a `1 << k` stride, scale 1) and
/// `cmp qword [b(+i*s)+d8],0` (`REX.W 83 /7` mod 01, imm8 0).
pub fn parse_manager(body: &[u8], slot_reg: u8, count: u8) -> ManagerLayout {
    let (mut entries, mut mult, mut scale, mut object) = (None, None, None, None);
    for i in 0..body.len() {
        let w = &body[i..];
        // mov r64,[slot+d8] or add r64,[slot+d8]
        if let [0x48 | 0x4C, 0x8B | 0x03, m, d, ..] = w {
            if entries.is_none() && m & 0xC7 == 0x40 | slot_reg {
                entries = Some(*d);
                continue;
            }
        }
        // shl r64,imm8: a power-of-two stride with no scaled access after it.
        if let [0x48 | 0x49, 0xC1, m, k, ..] = w {
            if m & 0xF8 == 0xE0 && *k < 16 {
                mult.get_or_insert(1u32 << k);
                scale.get_or_insert(1);
            }
        }
        if let [rex @ 0x48..=0x4F, 0x8D, m, sib, ..] = w {
            if m & 0xC7 == 0x04 && sib & 7 != 5 {
                let idx = ((rex >> 1) & 1) << 3 | (sib >> 3) & 7;
                let base = (rex & 1) << 3 | sib & 7;
                let s = 1u32 << (sib >> 6);
                if idx == base {
                    mult.get_or_insert(1 + s);
                } else if idx != 4 {
                    scale.get_or_insert(s);
                }
            }
        }
        if let [0x48..=0x4F, 0x83, m, rest @ ..] = w {
            if m & 0xF8 == 0x78 {
                match (m & 7, rest) {
                    (4, [sib, d, 0, ..]) => {
                        scale.get_or_insert(1u32 << (sib >> 6));
                        object.get_or_insert(*d);
                    }
                    (r, [d, 0, ..]) if r != 4 => {
                        object.get_or_insert(*d);
                    }
                    _ => {}
                }
            }
        }
    }
    ManagerLayout { count, entries, stride: mult.zip(scale).map(|(m, s)| m * s), object }
}

/// What a getter's slot load sits behind: its own short function, or a caller
/// the compiler inlined it into.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum GetterPrologue {
    /// The bytes from the function start to the slot load.
    Standalone(String),
    /// More than `STANDALONE_MAX` into a function, or no function start.
    Inlined,
}

impl GetterPrologue {
    pub fn label(&self) -> String {
        match self {
            GetterPrologue::Standalone(b) => format!("standalone {b}"),
            GetterPrologue::Inlined => "inlined".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Getter {
    /// The slot load.
    pub at: u32,
    pub fn_start: Option<u32>,
    pub prologue: GetterPrologue,
    pub slot_reg: &'static str,
    /// The global slot the load reads.
    pub slot: u32,
    pub count_reg: &'static str,
    pub manager: ManagerLayout,
    /// The `E8` that reaches the accessor.
    pub call_at: u32,
    /// The accessor function the call lands on.
    pub target: u32,
    /// Index into `Census::sites`.
    pub site: usize,
}

impl Getter {
    pub fn call_dist(&self) -> u32 {
        self.call_at - self.at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tail {
    /// `lea rcx,[rbp+d8]`, the frame-pointer codegen of 26636972.
    RbpDisp8(i8),
    /// Anything else, with its first bytes: the early warning.
    Other(String),
}

impl Tail {
    pub fn label(&self) -> String {
        match self {
            Tail::RbpDisp8(d) => format!("lea rcx,[rbp{}]", signed(*d)),
            Tail::Other(b) => format!("other: {b}"),
        }
    }

    pub fn class(&self) -> &'static str {
        match self {
            Tail::RbpDisp8(_) => "lea rcx,[rbp+d8]",
            Tail::Other(_) => "other",
        }
    }
}

fn signed(d: i8) -> String {
    if d < 0 {
        format!("-{:#x}", -i16::from(d))
    } else {
        format!("+{d:#x}")
    }
}

/// The tail after a head, and the offset of the accessor's own `E8` from the
/// head start, when the tail is the known shape.
pub fn parse_tail(after_head: &[u8]) -> (Tail, Option<usize>) {
    match after_head {
        [0x48, 0x8D, 0x4D, d, 0xE8, ..] => (Tail::RbpDisp8(*d as i8), Some(A_HEAD_LEN + 4)),
        other => (Tail::Other(hex_bytes(&other[..other.len().min(6)])), None),
    }
}

/// The optional index load into `r8d` before the loader call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexLoad {
    /// Nothing: `r8d` already holds the index.
    None,
    /// `44|45 0F B7 C0+r`: `movzx r8d,r16`.
    Movzx16(&'static str),
    /// `44|45 0F B6 C0+r`: `movzx r8d,r8`.
    Movzx8(&'static str),
    /// `44|45 8B C0+r`: `mov r8d,r32`, the tables indexed by a full `u32`.
    Mov(&'static str),
}

/// The shape of one loader call: the index load and the two argument moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Shape {
    pub index: IndexLoad,
    /// The source of `mov rdx,r64` (the status pointer).
    pub rdx: &'static str,
    /// The source of `mov rcx,r64` (the manager).
    pub rcx: &'static str,
}

impl Shape {
    pub fn label(self) -> String {
        let idx = match self.index {
            IndexLoad::None => "no index load".to_string(),
            IndexLoad::Movzx16(r) | IndexLoad::Movzx8(r) => format!("movzx r8d,{r}"),
            IndexLoad::Mov(r) => format!("mov r8d,{r}"),
        };
        format!("{idx}; mov rdx,{}; mov rcx,{}", self.rdx, self.rcx)
    }
}

/// `mov <dst>,r64` in either encoding (`8B` dst<-src or `89` src->dst), for
/// a legacy destination register `dst`: the source register.
fn mov_to(b: &[u8], dst: u8) -> Option<(u8, &[u8])> {
    match b {
        [rex @ (0x48 | 0x49), 0x8B, m, r @ ..] if m & 0xF8 == 0xC0 | dst << 3 => Some(((rex & 1) << 3 | m & 7, r)),
        [rex @ (0x48 | 0x4C), 0x89, m, r @ ..] if m & 0xC7 == 0xC0 | dst => Some((((rex >> 2) & 1) << 3 | (m >> 3) & 7, r)),
        _ => None,
    }
}

/// A parsed loader call: its shape, the `lea r9` displacement (the stream
/// object's start) and where the `E8` is from the grammar start. The same
/// grammar as `gimmick::parse_loader_call`:
///
/// ```text
/// 4C 8D 4D d8                                lea r9,[rbp+d8]
/// [44|45 0F B7 C0+r | 0F B6 C0+r | 8B C0+r]  optional index load into r8d
/// 48 8B D0+r | 48 89 C2+8r                   mov rdx,r64
/// 48 8B C8+r | 48 89 C1+8r                   mov rcx,r64
/// E8                                         call
/// ```
pub fn parse_loader_call(b: &[u8]) -> Option<(Shape, i8, usize)> {
    let [0x4C, 0x8D, 0x4D, d, rest @ ..] = b else { return None };
    let (index, rest) = match rest {
        [rex @ (0x44 | 0x45), 0x0F, 0xB7, m, r @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Movzx16(reg(&R16, (rex & 1) << 3 | m & 7)), r),
        [rex @ (0x44 | 0x45), 0x0F, 0xB6, m, r @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Movzx8(reg(&R8, (rex & 1) << 3 | m & 7)), r),
        [rex @ (0x44 | 0x45), 0x8B, m, r @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Mov(reg(&R32, (rex & 1) << 3 | m & 7)), r),
        _ => (IndexLoad::None, rest),
    };
    let (rdx, rest) = mov_to(rest, 2)?;
    let (rcx, rest) = mov_to(rest, 1)?;
    if rest.len() < 5 || rest[0] != 0xE8 {
        return None;
    }
    Some((Shape { index, rdx: reg(&R64, rdx), rcx: reg(&R64, rcx) }, *d as i8, b.len() - rest.len()))
}

/// Where the loader's stream object keeps the table body's buffer pointer,
/// its size and the cursor (`gimmick::StreamLayout`): offsets from the
/// object's start, which is the loader call's `lea r9,[rbp+d8]`.
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

/// `mov [rbp+d8],rax` when `wide`, else `mov [rbp+d8],eax`: its disp8.
fn rbp_store(b: &[u8], wide: bool) -> Option<i8> {
    match (wide, b) {
        (true, [0x48, 0x89, 0x45, d, ..]) | (false, [0x89, 0x45, d, ..]) => Some(*d as i8),
        _ => None,
    }
}

/// The stream layout an accessor's fill sequence encodes, from the bytes
/// between its file-load call and the loader call, relative to the loader
/// call's `lea r9,[rbp+d8]` displacement: the buffer is the only qword stored
/// from `[holder]`, the size the dword from `[holder+0xC]`, the cursor the one
/// other dword load-and-store (the offsets-table entry's `+4`). **Mirrors
/// `gimmick::parse_stream_fill` in `crates/desert-core/src/gimmick.rs`**; the
/// plugin-side census in `crates/desert-core/tests/gimmick_real.rs` asserts
/// the same answer. Through 26480954 the stores were `[rsp+d8]`.
pub fn parse_stream_fill(code: &[u8], r9_disp: i8) -> Option<StreamLayout> {
    let field = |d: i8| (d >= r9_disp).then(|| (i16::from(d) - i16::from(r9_disp)) as u32);
    let tail = |i: usize| code.get(i..).unwrap_or_default();
    let mut buf = None;
    for i in 0..code.len() {
        if let Some((reg, n)) = holder_load(tail(i)) {
            if let Some(d) = rbp_store(tail(i + n), true) {
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
    let mut pairs: Vec<(usize, (u8, u8), u8, i8)> = Vec::new();
    for i in 0..code.len() {
        let Some((reg, disp, n)) = dword_load(tail(i)) else { continue };
        let Some(d) = rbp_store(tail(i + n), false) else { continue };
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

/// The first bytes of a loader target, as a census key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoaderPrologue {
    /// `E9 rel32`: an incremental-link style jump, not the loader body.
    Jmp,
    /// The first `LOADER_PROLOGUE.len()` bytes.
    Bytes(String),
}

impl LoaderPrologue {
    pub fn label(&self) -> String {
        match self {
            LoaderPrologue::Jmp => "E9 jmp".into(),
            LoaderPrologue::Bytes(b) => b.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoaderCall {
    /// Where the grammar starts (`lea r9,[rbp+d8]`).
    pub at: u32,
    pub shape: Shape,
    /// The `lea r9` displacement: the stream object's start.
    pub r9_disp: i8,
    /// The `E8`.
    pub call_at: u32,
    pub target: u32,
    pub prologue: LoaderPrologue,
    /// The target opens with `LOADER_PROLOGUE`.
    pub prologue_ok: bool,
}

#[derive(Debug, Clone)]
pub struct Site {
    pub head: usize,
    pub at: u32,
    /// The accessor function: the getters' call target, else the exception
    /// directory's start.
    pub fn_start: Option<u32>,
    /// `true` when `fn_start` came from a getter's call.
    pub fn_from_getter: bool,
    pub tail: Tail,
    /// The accessor's own `E8` (the file-load call), right after the tail.
    pub call_at: Option<u32>,
    /// Indices into `Census::getters` of the getters calling this accessor.
    pub getters: Vec<usize>,
    /// The distinct slots those getters load.
    pub slots: BTreeSet<u32>,
    pub loader: Option<LoaderCall>,
    /// The stream layout the fill sequence before the loader call encodes,
    /// when there is a loader call and its fill parses.
    pub stream: Option<StreamLayout>,
    pub name: Option<String>,
    pub known: bool,
}

impl Site {
    /// Head offset into the accessor function.
    pub fn head_offset(&self) -> Option<u32> {
        Some(self.at - self.fn_start.filter(|&s| s <= self.at)?)
    }

    /// Loader-call grammar start minus the head.
    pub fn loader_head_dist(&self) -> Option<u32> {
        Some(self.loader.as_ref()?.at - self.at)
    }

    /// Loader-call grammar start minus the accessor's call: the distance
    /// `B_WINDOW` has to cover when the search starts at the call.
    pub fn loader_dist(&self) -> Option<u32> {
        Some(self.loader.as_ref()?.at - self.call_at?)
    }

    /// The one slot this accessor's getters agree on.
    pub fn slot(&self) -> Option<u32> {
        match self.slots.len() {
            1 => self.slots.first().copied(),
            _ => None,
        }
    }
}

pub struct Census {
    pub getters: Vec<Getter>,
    /// Slot-load + bounds-check pairs whose call reaches no accessor head.
    pub lookalikes: usize,
    pub sites: Vec<Site>,
    pub b_window: usize,
    /// Names in the list file.
    pub names_total: usize,
    /// Names in the list file that no accessor site passes.
    pub names_unseen: Vec<String>,
}

fn count_by<K: Ord, I: IntoIterator<Item = K>>(it: I) -> BTreeMap<K, usize> {
    let mut m = BTreeMap::new();
    for k in it {
        *m.entry(k).or_insert(0) += 1;
    }
    m
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
        count_by(self.sites.iter().map(|s| s.tail.class()))
    }

    pub fn rejected_tails(&self) -> Vec<&Site> {
        self.sites.iter().filter(|s| matches!(s.tail, Tail::Other(_))).collect()
    }

    /// Head offset into the accessor function, as `(offset, sites)`.
    pub fn head_offsets(&self) -> BTreeMap<u32, usize> {
        count_by(self.sites.iter().filter_map(Site::head_offset))
    }

    /// Accessors whose function start came from a getter's call.
    pub fn fns_from_getters(&self) -> usize {
        self.sites.iter().filter(|s| s.fn_from_getter).count()
    }

    /// Getters per prologue variant.
    pub fn getter_prologues(&self) -> BTreeMap<String, usize> {
        count_by(self.getters.iter().map(|g| g.prologue.label()))
    }

    /// Slot load to the accessor call, as `(distance, getters)`.
    pub fn getter_call_dists(&self) -> BTreeMap<u32, usize> {
        count_by(self.getters.iter().map(Getter::call_dist))
    }

    /// `(slot register, bounds-check register)` pairs.
    pub fn getter_regs(&self) -> BTreeMap<(&'static str, &'static str), usize> {
        count_by(self.getters.iter().map(|g| (g.slot_reg, g.count_reg)))
    }

    /// The manager offsets the getter bodies use.
    pub fn manager_layouts(&self) -> BTreeMap<ManagerLayout, usize> {
        count_by(self.getters.iter().map(|g| g.manager))
    }

    /// The `(count, entries)` displacements the getters use: one pair on a
    /// sane build, whatever the entry stride.
    pub fn manager_heads(&self) -> BTreeMap<(u8, Option<u8>), usize> {
        count_by(self.getters.iter().map(|g| (g.manager.count, g.manager.entries)))
    }

    /// Tables whose getters use a layout other than the most common one.
    pub fn manager_outliers(&self) -> BTreeMap<ManagerLayout, BTreeSet<&str>> {
        let common = self.manager_layouts().into_iter().max_by_key(|&(_, n)| n).map(|(m, _)| m);
        let mut m: BTreeMap<ManagerLayout, BTreeSet<&str>> = BTreeMap::new();
        for g in self.getters.iter().filter(|g| Some(g.manager) != common) {
            let name = self.sites.get(g.site).and_then(|s| s.name.as_deref()).unwrap_or("?");
            m.entry(g.manager).or_default().insert(name);
        }
        m
    }

    /// Getters per accessor, as `(getters, accessors)`.
    pub fn getters_per_site(&self) -> BTreeMap<usize, usize> {
        count_by(self.sites.iter().map(|s| s.getters.len()))
    }

    pub fn sites_without_getter(&self) -> Vec<&Site> {
        self.sites.iter().filter(|s| s.getters.is_empty()).collect()
    }

    /// Accessors whose getters load more than one slot.
    pub fn slot_disagreements(&self) -> Vec<&Site> {
        self.sites.iter().filter(|s| s.slots.len() > 1).collect()
    }

    /// Table name to the slots its accessors' getters load.
    pub fn slots_by_name(&self) -> BTreeMap<&str, BTreeSet<u32>> {
        let mut m: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
        for s in &self.sites {
            if let Some(n) = s.name.as_deref() {
                m.entry(n).or_default().extend(s.slots.iter().copied());
            }
        }
        m
    }

    /// Names passed by more than one accessor, or whose accessors' getters
    /// load more than one slot.
    pub fn names_without_one_slot(&self) -> Vec<(&str, usize)> {
        self.slots_by_name().into_iter().filter(|(_, s)| s.len() > 1).map(|(n, s)| (n, s.len())).collect()
    }

    pub fn distinct_slots(&self) -> usize {
        self.getters.iter().map(|g| g.slot).collect::<BTreeSet<_>>().len()
    }

    pub fn loader_shapes(&self) -> BTreeMap<String, usize> {
        count_by(self.sites.iter().filter_map(|s| s.loader.as_ref()).map(|l| l.shape.label()))
    }

    pub fn loaders_found(&self) -> usize {
        self.sites.iter().filter(|s| s.loader.is_some()).count()
    }

    pub fn loaders_with_prologue(&self) -> usize {
        self.sites.iter().filter(|s| s.loader.as_ref().is_some_and(|l| l.prologue_ok)).count()
    }

    /// Distinct loader-target prologues, as `(prologue, sites)`.
    pub fn loader_prologues(&self) -> BTreeMap<LoaderPrologue, usize> {
        count_by(self.sites.iter().filter_map(|s| s.loader.as_ref()).map(|l| l.prologue.clone()))
    }

    /// Loader-call distance from the head, as `(distance, sites)`.
    pub fn loader_head_dist_hist(&self) -> BTreeMap<u32, usize> {
        count_by(self.sites.iter().filter_map(Site::loader_head_dist))
    }

    /// Min and max of the loader-call distance from the accessor's call.
    pub fn loader_dist_range(&self) -> Option<(u32, u32)> {
        let d: Vec<u32> = self.sites.iter().filter_map(|s| s.loader_dist()).collect();
        Some((*d.iter().min()?, *d.iter().max()?))
    }

    pub fn loader_dist_hist(&self) -> BTreeMap<u32, usize> {
        count_by(self.sites.iter().filter_map(Site::loader_dist))
    }

    /// The `B_WINDOW` the plugin should use: the largest distance from the
    /// accessor's call rounded up to 0x10, plus 0x40 of slack, as `gimmick.rs`
    /// documents it.
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
        count_by(self.sites.iter().filter_map(|s| s.stream))
    }

    /// Sites with a loader call whose stream fill does not parse.
    pub fn streams_unparsed(&self) -> Vec<&Site> {
        self.sites.iter().filter(|s| s.loader.is_some() && s.stream.is_none()).collect()
    }

    /// `stream fill: buffer +0x20 size +0x28 cursor +0x2c at 151 of 151 sites`,
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

    /// The getters of the accessor passing `name`.
    pub fn getters_of(&self, name: &str) -> Vec<&Getter> {
        self.site_named(name).map(|s| s.getters.iter().filter_map(|&i| self.getters.get(i)).collect()).unwrap_or_default()
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

/// The index of the head in `heads` (sorted by address) that a call target
/// `t` reaches: the first head at or after `t`, if it is within `HEAD_REACH`.
fn head_reached(heads: &[(usize, u32)], t: u32) -> Option<usize> {
    let i = heads.partition_point(|&(_, a)| a < t);
    let &(_, a) = heads.get(i)?;
    (a - t < HEAD_REACH).then_some(i)
}

/// The first `E8` in `[from, from + CALL_WINDOW)` whose target reaches an
/// accessor head: the call's address, its target and the head's index.
fn find_getter_call(img: &[u8], from: usize, heads: &[(usize, u32)]) -> Option<(u32, u32, usize)> {
    for p in from..(from + CALL_WINDOW).min(img.len().saturating_sub(5)) {
        if img[p] != 0xE8 {
            continue;
        }
        let Some(rel) = i32_at(img, p + 1) else { continue };
        let t = rip_target(p as u32, 5, rel);
        if let Some(h) = head_reached(heads, t) {
            return Some((p as u32, t, h));
        }
    }
    None
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
        let Some((shape, r9_disp, call_off)) = parse_loader_call(w) else { continue };
        let call_at = p + call_off;
        let rel = i32_at(img, call_at + 1)?;
        let target = rip_target(call_at as u32, 5, rel);
        let head = img.get(target as usize..target as usize + LOADER_PROLOGUE.len());
        let prologue_ok = head == Some(&LOADER_PROLOGUE[..]);
        let prologue = match head {
            Some([0xE9, ..]) => LoaderPrologue::Jmp,
            Some(b) => LoaderPrologue::Bytes(hex_bytes(b)),
            None => LoaderPrologue::Bytes("(outside the image)".into()),
        };
        return Some(LoaderCall { at: p as u32, shape, r9_disp, call_at: call_at as u32, target, prologue, prologue_ok });
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

    // Getters: every slot load + bounds check whose call reaches a head.
    let mut getters = Vec::new();
    let mut lookalikes = 0;
    for p in 0..img.len().saturating_sub(12) {
        if img[p] != 0x48 || img[p + 1] != 0x8B || img[p + 2] & 0xC7 != 0x05 {
            continue;
        }
        let Some((slot_reg, disp, creg, count, len)) = parse_slot_load(&img[p..(p + 12).min(img.len())]) else { continue };
        let Some((call_at, target, site)) = find_getter_call(img, p + len, &heads) else {
            lookalikes += 1;
            continue;
        };
        let manager = parse_manager(&img[p + len..call_at as usize], slot_reg, count);
        let at = p as u32;
        let fn_start = exe.function_start(at).0;
        let prologue = match fn_start {
            Some(s) if s <= at && at - s <= STANDALONE_MAX => GetterPrologue::Standalone(hex_bytes(&img[s as usize..p])),
            _ => GetterPrologue::Inlined,
        };
        getters.push(Getter {
            at,
            fn_start,
            prologue,
            slot_reg: reg(&R64, slot_reg),
            slot: rip_target(at, 7, disp),
            count_reg: reg(&R32, creg),
            manager,
            call_at,
            target,
            site,
        });
    }

    let mut sites = Vec::with_capacity(heads.len());
    for (n, &(head, at)) in heads.iter().enumerate() {
        let after = &img[(at as usize + A_HEAD_LEN).min(img.len())..];
        let (tail, call_off) = parse_tail(after);
        let call_at = call_off.map(|o| at + o as u32);
        let next_head = heads.get(n + 1).map(|&(_, a)| a);
        let from = call_at.map(|c| c + 5).unwrap_or(at + A_HEAD_LEN as u32);
        let loader = find_loader(img, from, b_window, next_head);
        // The fill runs from the end of the accessor's call (`E8` + rel32)
        // to the loader call's `lea r9,[rbp+d8]`.
        let stream = match (&loader, call_at) {
            (Some(l), Some(c)) => img.get(c as usize + 5..l.at as usize).and_then(|code| parse_stream_fill(code, l.r9_disp)),
            _ => None,
        };
        let mine: Vec<usize> = getters.iter().enumerate().filter(|(_, g)| g.site == n).map(|(i, _)| i).collect();
        let slots: BTreeSet<u32> = mine.iter().map(|&i| getters[i].slot).collect();
        let from_getter = mine.iter().map(|&i| getters[i].target).min();
        let (fn_start, fn_from_getter) = match from_getter {
            Some(t) => (Some(t), true),
            None => (exe.function_start(at).0, false),
        };
        let name = site_name(exe, at, HEADS[head].2);
        let is_known = name.as_deref().is_some_and(|n| known.contains(&name_key(n)));
        sites.push(Site {
            head,
            at,
            fn_start,
            fn_from_getter,
            tail,
            call_at,
            getters: mine,
            slots,
            loader,
            stream,
            name,
            known: is_known,
        });
    }

    let seen: BTreeSet<String> = sites.iter().filter_map(|s| s.name.as_deref()).map(name_key).collect();
    let names_unseen = names.iter().filter(|n| !seen.contains(&name_key(n))).cloned().collect();
    Ok(Census { getters, lookalikes, sites, b_window, names_total: names.len(), names_unseen })
}

fn hist<K: std::fmt::Display>(m: &BTreeMap<K, usize>) -> String {
    m.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join(", ")
}

fn hist_hex(m: &BTreeMap<u32, usize>) -> String {
    m.iter().map(|(k, v)| format!("{k:#x}: {v}")).collect::<Vec<_>>().join(", ")
}

pub fn print_sites(out: &mut impl Write, c: &Census, tsv: bool) -> Result<()> {
    if tsv {
        writeln!(out, "rva\thead\tfn\thead_off\tfn_from\ttail\tcall_at\tgetters\tslots\tloader_at\tloader_head_dist\tloader_shape\tloader_call\tloader_target\tloader_prologue\tprologue_ok\tstream_buf\tstream_size\tstream_cursor\tname\tknown")?;
    }
    for s in &c.sites {
        let opt = |v: Option<u32>| v.map(|x| format!("{x:#x}")).unwrap_or_default();
        let slots = s.slots.iter().map(|x| format!("{x:#x}")).collect::<Vec<_>>().join(",");
        let (la, ld, ls, lc, lt, lp, lok) = match &s.loader {
            Some(l) => (
                format!("{:#x}", l.at),
                opt(s.loader_head_dist()),
                l.shape.label(),
                format!("{:#x}", l.call_at),
                format!("{:#x}", l.target),
                l.prologue.label(),
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
        let from = if s.fn_from_getter { "getter" } else { "pdata" };
        if tsv {
            writeln!(
                out,
                "{:#x}\t{}\t{}\t{}\t{from}\t{}\t{}\t{}\t{slots}\t{la}\t{ld}\t{ls}\t{lc}\t{lt}\t{lp}\t{lok}\t{fb}\t{fs}\t{fc}\t{name}\t{}",
                s.at,
                s.head,
                opt(s.fn_start),
                opt(s.head_offset()),
                s.tail.label(),
                opt(s.call_at),
                s.getters.len(),
                s.known
            )?;
        } else {
            writeln!(
                out,
                "  +{:#x} h{} fn {:<10} +{:<5} {:<18} getters {} slot {:<10} loader +{:<5} {:<50} {:<3} fill {:<14} {name}{}",
                s.at,
                s.head,
                opt(s.fn_start),
                opt(s.head_offset()),
                s.tail.label(),
                s.getters.len(),
                if slots.is_empty() { "-" } else { &slots },
                if ld.is_empty() { "-" } else { &ld },
                if ls.is_empty() { "-" } else { &ls },
                if lok.is_empty() { "-" } else { &lok },
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
        writeln!(out, "getters\t{}", c.getters.len())?;
        writeln!(out, "getter_lookalikes\t{}", c.lookalikes)?;
        for (k, v) in c.getter_prologues() {
            writeln!(out, "getter_prologue\t{k}\t{v}")?;
        }
        for (k, v) in c.getter_call_dists() {
            writeln!(out, "getter_call_dist\t{k:#x}\t{v}")?;
        }
        for ((s, r), v) in c.getter_regs() {
            writeln!(out, "getter_regs\t{s}\t{r}\t{v}")?;
        }
        for (k, v) in c.manager_layouts() {
            writeln!(out, "manager\t{}\t{v}", k.label())?;
        }
        writeln!(out, "sites\t{}", c.sites.len())?;
        for (i, n) in c.head_counts().iter().enumerate() {
            writeln!(out, "head{i}\t{n}")?;
        }
        for (k, v) in c.head_offsets() {
            writeln!(out, "head_offset\t{k:#x}\t{v}")?;
        }
        for (k, v) in c.tails() {
            writeln!(out, "tail\t{k}\t{v}")?;
        }
        for (k, v) in c.getters_per_site() {
            writeln!(out, "getters_per_site\t{k}\t{v}")?;
        }
        writeln!(out, "slot_disagreements\t{}", c.slot_disagreements().len())?;
        writeln!(out, "names_without_one_slot\t{}", c.names_without_one_slot().len())?;
        writeln!(out, "distinct_slots\t{}", c.distinct_slots())?;
        for (k, v) in c.loader_shapes() {
            writeln!(out, "loader_shape\t{k}\t{v}")?;
        }
        writeln!(out, "loaders\t{}", c.loaders_found())?;
        writeln!(out, "loaders_with_prologue\t{}", c.loaders_with_prologue())?;
        for (k, v) in c.loader_prologues() {
            writeln!(out, "loader_prologue\t{}\t{v}", k.label())?;
        }
        for (k, v) in c.loader_head_dist_hist() {
            writeln!(out, "loader_head_dist\t{k:#x}\t{v}")?;
        }
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

    writeln!(
        out,
        "census: {} getter(s), {} accessor site(s), {} loader call(s); loader-call window {:#x}",
        c.getters.len(),
        c.sites.len(),
        c.loaders_found(),
        c.b_window
    )?;
    let regs: Vec<String> = c.getter_regs().iter().map(|((s, r), n)| format!("mov {s} / cmp {r}: {n}")).collect();
    writeln!(
        out,
        "  getters: slot load `mov r64,[rip+slot]; cmp r32,[r64+d8]` with a call into an accessor within {CALL_WINDOW:#x}; {} other slot-load pair(s) reach none; registers {}",
        c.lookalikes,
        if regs.is_empty() { "none".to_string() } else { regs.join(", ") }
    )?;
    let mgr: Vec<String> = c.manager_layouts().iter().map(|(m, n)| format!("{}: {n}", m.label())).collect();
    writeln!(out, "    manager: {}", if mgr.is_empty() { "none".to_string() } else { mgr.join("; ") })?;
    for (m, names) in c.manager_outliers() {
        writeln!(out, "      {}: {}", m.label(), names.iter().copied().collect::<Vec<_>>().join(" "))?;
    }
    writeln!(out, "    slot load -> accessor call: {}", hist_hex(&c.getter_call_dists()))?;
    for (p, n) in c.getter_prologues() {
        writeln!(out, "    prologue {p}: {n}")?;
    }
    let hc = c.head_counts();
    writeln!(
        out,
        "  accessors: {} head(s), function start from a getter's call at {}, head offset into the function {}",
        c.sites.len(),
        c.fns_from_getters(),
        hist_hex(&c.head_offsets())
    )?;
    for (i, (label, pat, _)) in HEADS.iter().enumerate() {
        writeln!(out, "    head {i} {label:<18} {:>4}  {pat}", hc[i])?;
    }
    writeln!(out, "    tails: {}", hist(&c.tails()))?;
    for s in c.rejected_tails() {
        writeln!(out, "      REJECTED tail at +{:#x}: {}", s.at, s.tail.label())?;
    }
    writeln!(
        out,
        "  slots: getters per accessor {}; {} accessor(s) without a getter; {} with getters disagreeing on the slot; {} distinct slot(s); {} name(s) without exactly one slot",
        hist(&c.getters_per_site()),
        c.sites_without_getter().len(),
        c.slot_disagreements().len(),
        c.distinct_slots(),
        c.names_without_one_slot().len()
    )?;
    for s in c.sites_without_getter() {
        writeln!(out, "    no getter: +{:#x} {}", s.at, s.name.clone().unwrap_or_else(|| "?".into()))?;
    }
    for (n, k) in c.names_without_one_slot() {
        writeln!(out, "    {n}: {k} slots")?;
    }
    let shapes: Vec<String> = c.loader_shapes().iter().map(|(s, n)| format!("{s}: {n}")).collect();
    writeln!(
        out,
        "  loader calls: {}/{} found, {} open with LOADER_PROLOGUE ({} bytes); shapes {}",
        c.loaders_found(),
        c.sites.len(),
        c.loaders_with_prologue(),
        LOADER_PROLOGUE.len(),
        if shapes.is_empty() { "none".to_string() } else { shapes.join("; ") }
    )?;
    writeln!(out, "    distance from the head: {}", hist_hex(&c.loader_head_dist_hist()))?;
    if let Some((lo, hi)) = c.loader_dist_range() {
        writeln!(
            out,
            "    distance from the accessor's call: {lo:#x}..{hi:#x} ({}); suggested B_WINDOW {:#x}",
            hist_hex(&c.loader_dist_hist()),
            c.suggested_b_window().unwrap_or(0)
        )?;
    }
    for (p, n) in c.loader_prologues() {
        let mark = if p == LoaderPrologue::Bytes(hex_bytes(&LOADER_PROLOGUE)) { "  (= LOADER_PROLOGUE)" } else { "" };
        writeln!(out, "    loader prologue {}: {n}{mark}", p.label())?;
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
        let fs = g.fn_start.map(|s| format!("+{s:#x}")).unwrap_or_else(|| "?".into());
        writeln!(
            out,
            "  gimmickinfo: accessor function {fs}, head +{:#x} ({}), tail {}",
            g.at,
            g.head_offset().map(|o| format!("+{o:#x} in")).unwrap_or_else(|| "offset unknown".into()),
            g.tail.label()
        )?;
        for x in c.getters_of("gimmickinfo") {
            writeln!(
                out,
                "    getter {} slot load +{:#x}, slot +{:#x}, call +{:#x}; {}",
                x.fn_start.map(|s| format!("+{s:#x}")).unwrap_or_else(|| "?".into()),
                x.at,
                x.slot,
                x.call_at,
                x.manager.label()
            )?;
        }
        if let Some(l) = &g.loader {
            writeln!(
                out,
                "    loader call +{:#x} ({}, call at +{:#x}, head+{:#x}) -> loader +{:#x}, prologue {}",
                l.at,
                l.shape.label(),
                l.call_at,
                g.loader_head_dist().unwrap_or(0),
                l.target,
                if l.prologue_ok { "= LOADER_PROLOGUE".to_string() } else { format!("DIFFERS: {}", l.prologue.label()) }
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

    /// The `gimmickinfo` getter on 26636972 (`+0x3dac70`, 0x76 bytes).
    const GETTER_26636972: [u8; 0x76] = [
        0x48, 0x83, 0xEC, 0x28, 0x44, 0x0F, 0xB7, 0x01, 0x48, 0x8B, 0x0D, 0x19, 0xB7, 0xE6, 0x06, 0x44, 0x3B, 0x41,
        0x28, 0x72, 0x0C, 0x48, 0x8B, 0x05, 0xFC, 0xE8, 0xDD, 0x06, 0x48, 0x83, 0xC4, 0x28, 0xC3, 0x48, 0x8B, 0x41,
        0x20, 0x4B, 0x8D, 0x14, 0x40, 0x48, 0x83, 0x7C, 0xD0, 0x08, 0x00, 0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x8D,
        0x1C, 0xD0, 0x75, 0x20, 0x48, 0x8D, 0x54, 0x24, 0x30, 0xE8, 0x0C, 0x60, 0x00, 0x00, 0x83, 0x38, 0x00, 0x74,
        0x11, 0x48, 0x8B, 0x05, 0xC8, 0xE8, 0xDD, 0x06, 0x48, 0x8B, 0x5C, 0x24, 0x20, 0x48, 0x83, 0xC4, 0x28, 0xC3,
        0x48, 0x8B, 0x4B, 0x08, 0x48, 0x8B, 0x05, 0xB3, 0xE8, 0xDD, 0x06, 0x48, 0x85, 0xC9, 0x48, 0x8B, 0x5C, 0x24,
        0x20, 0x48, 0x0F, 0x45, 0xC1, 0x48, 0x83, 0xC4, 0x28, 0xC3,
    ];

    #[test]
    fn the_getter_slot_load_and_manager_offsets_parse() {
        let (slot_reg, disp, creg, count, len) = parse_slot_load(&GETTER_26636972[8..]).unwrap();
        assert_eq!((reg(&R64, slot_reg), disp, reg(&R32, creg), count, len), ("rcx", 0x06E6B719, "r8d", 0x28, 11));
        // From the end of the bounds check to the E8 at +0x3f.
        let m = parse_manager(&GETTER_26636972[8 + len..0x3F], slot_reg, count);
        assert_eq!(m, ManagerLayout { count: 0x28, entries: Some(0x20), stride: Some(24), object: Some(0x08) });
        assert_eq!(m.label(), "count +0x28, entries +0x20, stride 24, object +0x8");
        // The inlined form: lea first, entries into rsi, object through lea rdi.
        // lea rdx,[rax+rax*2]; mov rax,[rcx+0x20]; lea rsi,[rax+rdx*8]; cmp qword [rsi+8],0
        let inl = [0x48, 0x8D, 0x14, 0x40, 0x48, 0x8B, 0x41, 0x20, 0x48, 0x8D, 0x34, 0xD0, 0x48, 0x83, 0x7E, 0x08, 0x00];
        assert_eq!(parse_manager(&inl, 1, 0x28), m);
        // The 32-byte entries: mov rbx,r8; shl rbx,5; add rbx,[rcx+0x20]; cmp qword [rbx+0x10],0
        let wide = [0x49, 0x8B, 0xD8, 0x48, 0xC1, 0xE3, 0x05, 0x48, 0x03, 0x59, 0x20, 0x48, 0x83, 0x7B, 0x10, 0x00];
        assert_eq!(parse_manager(&wide, 1, 0x28), ManagerLayout { count: 0x28, entries: Some(0x20), stride: Some(32), object: Some(0x10) });
        // The old shape (mov rbx,[rip]; cmp edi,[rbx+8]) still parses as a pair.
        let old = [0x48, 0x8B, 0x1D, 1, 0, 0, 0, 0x3B, 0x7B, 0x08];
        assert_eq!(parse_slot_load(&old).map(|t| (t.0, t.2, t.3, t.4)), Some((3, 7, 0x08, 10)));
        // A compare against another base is not a bounds check.
        assert_eq!(parse_slot_load(&[0x48, 0x8B, 0x0D, 1, 0, 0, 0, 0x44, 0x3B, 0x42, 0x28]), None);
        assert_eq!(parse_slot_load(&[0x48, 0x8B, 0x0D, 1, 0, 0, 0, 0x90, 0x90, 0x90]), None);
    }

    #[test]
    fn the_tail_parses_and_anything_else_is_rejected() {
        assert_eq!(parse_tail(&[0x48, 0x8D, 0x4D, 0x40, 0xE8, 0, 0, 0, 0]), (Tail::RbpDisp8(0x40), Some(18)));
        assert_eq!(Tail::RbpDisp8(0x40).label(), "lea rcx,[rbp+0x40]");
        assert_eq!(Tail::RbpDisp8(-0x30).label(), "lea rcx,[rbp-0x30]");
        // The 26345091..26480954 tail is not this build's.
        let (t, c) = parse_tail(&[0x49, 0x8D, 0x4B, 0x10, 0xE8, 0, 0, 0, 0]);
        assert!(matches!(t, Tail::Other(_)));
        assert_eq!(c, None);
        assert!(matches!(parse_tail(&[0x48, 0x8D, 0x4D, 0x40, 0x90]).0, Tail::Other(_)), "a tail without the call is not a tail");
        assert!(matches!(parse_tail(&[]).0, Tail::Other(_)));
    }

    #[test]
    fn the_loader_call_grammar_covers_every_seen_shape() {
        let sh = |index, rdx, rcx| Shape { index, rdx, rcx };
        let movzx = [0x4C, 0x8D, 0x4D, 0xD0, 0x45, 0x0F, 0xB7, 0xC6, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&movzx), Some((sh(IndexLoad::Movzx16("r14w"), "rdi", "rsi"), -0x30, 14)));
        assert_eq!(sh(IndexLoad::Movzx16("r14w"), "rdi", "rsi").label(), "movzx r8d,r14w; mov rdx,rdi; mov rcx,rsi");
        let mov = [0x4C, 0x8D, 0x4D, 0xD0, 0x45, 0x8B, 0xC6, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&mov), Some((sh(IndexLoad::Mov("r14d"), "rdi", "rsi"), -0x30, 13)));
        let byte = [0x4C, 0x8D, 0x4D, 0xD0, 0x45, 0x0F, 0xB6, 0xC6, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&byte).map(|t| t.0.index), Some(IndexLoad::Movzx8("r14b")));
        // No index load, and the 89 encodings of the two moves.
        let bare = [0x4C, 0x8D, 0x4D, 0xD0, 0x48, 0x89, 0xFA, 0x48, 0x89, 0xF1, 0xE8, 0, 0, 0, 0];
        assert_eq!(parse_loader_call(&bare), Some((sh(IndexLoad::None, "rdi", "rsi"), -0x30, 10)));
        // Cut before the call, and the old rsp-relative lea r9: not a loader call.
        assert_eq!(parse_loader_call(&movzx[..15]), None);
        assert_eq!(parse_loader_call(&[0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8, 0, 0, 0, 0]), None);
        assert_eq!(parse_loader_call(&[0x90; 20]), None);
    }

    #[test]
    fn table_and_class_names_share_a_key() {
        assert_eq!(name_key("gimmickinfo"), name_key("GimmickInfo"));
        assert_eq!(name_key("Skill"), name_key("SkillInfo"));
        assert_eq!(name_key("statchangeInfo"), "statchange");
        assert_ne!(name_key("Faction"), name_key("FactionNode"));
    }

    #[test]
    fn a_getter_call_must_reach_a_head() {
        let heads = [(0usize, 0x130u32), (1, 0x400)];
        assert_eq!(head_reached(&heads, 0x100), Some(0));
        assert_eq!(head_reached(&heads, 0x130), Some(0));
        assert_eq!(head_reached(&heads, 0x0B0), None, "0x80 or more before the head is not this accessor");
        assert_eq!(head_reached(&heads, 0x500), None);
        let mut img = vec![0x90u8; 0x100];
        // A stray E8 to nowhere, then the real call to 0x100 -> head 0x130.
        img[0x10] = 0xE8;
        img[0x11..0x15].copy_from_slice(&0x1000i32.to_le_bytes());
        img[0x20] = 0xE8;
        img[0x21..0x25].copy_from_slice(&(0x100i32 - 0x25).to_le_bytes());
        assert_eq!(find_getter_call(&img, 0x08, &heads), Some((0x20, 0x100, 0)));
        assert_eq!(find_getter_call(&img, 0x21, &heads), None);
    }

    #[test]
    fn the_loader_search_stops_at_the_next_head() {
        let mut img = vec![0x90u8; 0x200];
        let call = [0x4C, 0x8D, 0x4D, 0xD0, 0x45, 0x0F, 0xB7, 0xC6, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8, 0, 0, 0, 0];
        img[0x100..0x100 + call.len()].copy_from_slice(&call);
        img[0x113..0x113 + LOADER_PROLOGUE.len()].copy_from_slice(&LOADER_PROLOGUE);
        let l = find_loader(&img, 0x80, 0x400, None).unwrap();
        assert_eq!((l.at, l.call_at, l.target, l.prologue_ok, l.r9_disp), (0x100, 0x10E, 0x113, true, -0x30));
        assert!(find_loader(&img, 0x80, 0x400, Some(0xF0)).is_none(), "a head before it ends the search");
        assert!(find_loader(&img, 0x80, 0x40, None).is_none(), "the window is honoured");
        img[0x113] = 0xE9;
        assert_eq!(find_loader(&img, 0x80, 0x400, None).unwrap().prologue, LoaderPrologue::Jmp);
    }

    #[test]
    fn b_window_is_the_rounded_maximum_plus_slack() {
        let c = Census { getters: Vec::new(), lookalikes: 0, sites: Vec::new(), b_window: 0, names_total: 0, names_unseen: Vec::new() };
        assert_eq!(c.suggested_b_window(), None);
        let s = Site {
            head: 0,
            at: 0xF0,
            fn_start: Some(0xC0),
            fn_from_getter: true,
            tail: Tail::RbpDisp8(0x40),
            call_at: Some(0x102),
            getters: Vec::new(),
            slots: BTreeSet::new(),
            loader: Some(LoaderCall {
                at: 0x17D,
                shape: Shape { index: IndexLoad::None, rdx: "rdi", rcx: "rsi" },
                r9_disp: -0x30,
                call_at: 0x18B,
                target: 0,
                prologue: LoaderPrologue::Jmp,
                prologue_ok: false,
            }),
            stream: None,
            name: None,
            known: false,
        };
        assert_eq!((s.head_offset(), s.loader_head_dist(), s.loader_dist()), (Some(0x30), Some(0x8D), Some(0x7B)));
        let c = Census { getters: Vec::new(), lookalikes: 0, sites: vec![s], b_window: 0, names_total: 0, names_unseen: Vec::new() };
        assert_eq!(c.suggested_b_window(), Some(0xC0));
    }

    /// The `gimmickinfo` fill on 26636972, `+0x3e0d35..+0x3e0d7e` (the record
    /// path from the entry address to the loader call's `lea r9,[rbp-0x30]`).
    const FILL_26636972: [u8; 73] = [
        0x4B, 0x8D, 0x0C, 0x76, 0x48, 0x8B, 0x46, 0x20, 0x48, 0x8D, 0x14, 0xC8, 0x4C, 0x8D, 0x25, 0x80, 0xB1, 0x75,
        0x05, 0x4C, 0x89, 0x65, 0xD0, 0xC6, 0x45, 0xD8, 0x00, 0x4C, 0x89, 0x6D, 0xE0, 0x4C, 0x89, 0x6D, 0xE8, 0x4C,
        0x8D, 0x3D, 0xB9, 0xB0, 0x75, 0x05, 0x4C, 0x89, 0x7D, 0xD0, 0x48, 0x8B, 0x5D, 0x58, 0x48, 0x8B, 0x03, 0x48,
        0x89, 0x45, 0xF0, 0x8B, 0x43, 0x0C, 0x89, 0x45, 0xF8, 0x44, 0x89, 0x6D, 0xFC, 0x8B, 0x42, 0x04, 0x89, 0x45,
        0xFC,
    ];

    #[test]
    fn the_stream_fill_parses_like_the_plugin_side() {
        let new = StreamLayout { buf: 0x20, size: 0x28, cursor: 0x2C };
        assert_eq!(parse_stream_fill(&FILL_26636972, -0x30), Some(new));
        assert_eq!(new.label(), "buffer +0x20 size +0x28 cursor +0x2c");
        // Relative to the object's start, which is the lea r9 displacement.
        assert_eq!(parse_stream_fill(&FILL_26636972, -0x20), Some(StreamLayout { buf: 0x10, size: 0x18, cursor: 0x1C }));
        assert_eq!(parse_stream_fill(&FILL_26636972, -0x0F), None, "a field before the object is no field");
        // A second size store is ambiguous; no fill at all is nothing.
        let mut two = FILL_26636972.to_vec();
        two.extend_from_slice(&[0x8B, 0x43, 0x0C, 0x89, 0x45, 0xE0]);
        assert_eq!(parse_stream_fill(&two, -0x30), None);
        assert_eq!(parse_stream_fill(&[0x90; 40], -0x30), None);
    }
}
