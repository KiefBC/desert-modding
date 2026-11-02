//! Pure byte logic for the `gimmickinfo` table: record headers, resource-output
//! lists, yield multiplication, and finding the game's record loader and the
//! global slots its data-table managers live in.
//!
//! The gatherer subsystem of `DesertTooling.asi` hooks the loader and rewrites
//! the raw table bytes in memory just before the game parses each record, which
//! is what makes it a drop-in replacement for the `desert-gatherer-dmm/` offset
//! patches: same edits, but computed from the bytes instead of from a
//! build-specific offset list.
//!
//! Everything here is a pure function over a byte slice, so it compiles and is
//! unit tested natively on Linux (see the cfg-gating note in README.md). It must
//! never panic on arbitrary input: the game calls it with whatever the loader
//! hands us.
//!
//! # Format (verified in Ghidra against Steam build 25116796, and unchanged
//! through 25246367)
//!
//! The table body is a flat sequence of records with no header at all. Each
//! record starts `u32 key`, `u32 name_len`, `name_len` ASCII bytes, `0x00`,
//! then its fields. **On 26361232 the header changed shape**: `u32 name_len`,
//! the name with no terminator, `u32 key`, `0x00`, `u32 path_len`, a prefab
//! path, a `u8` flag; see [`HeaderShape`]. Everything after the header (the
//! output lists and their blocks) is found by signature and did not care. DMM's extracted copy of the body is
//! `<DMM>/backups/gimmickinfo_pabgb_clean.bin` (22,325,302 bytes, sha256
//! `95e2dcf4b3447cdf6186b9fa816065f4e09f3ce49fe7047fd111f1bb6517411a`) and it is
//! byte-identical to what the game loads into memory.
//!
//! Inside a gather record, resource outputs are stored as one or more lists: a
//! `u32 count` followed by `count` blocks of exactly [`BLOCK`] bytes:
//!
//! ```text
//! +0    u8   flag       always 1
//! +1    u32  item       item id
//! +5    u32  0          padding, zero on all 896 blocks
//! +42   u64  min        \ vanilla 1..=8
//! +50   u64  max        /
//! +58   u16  0xFFFF     literal FF FF
//! +60   u32  item       the same item id again
//! +64   u32  0          padding, zero on all 896 blocks
//! ```
//!
//! **Corrected 2026-09-12.** This diagram, [`ITEM_AT`] and [`ITEM_TAIL_AT`] all
//! had the item id 4 bytes too high (at `+5` and `+64`, which are the two zero
//! pads), and so did the block builders in this file's own tests and in
//! `crates/desert-core/tests/props.rs`, which is why nothing caught it. Confirmed over
//! **all 896** output blocks of `gimmickinfo_pabgb_clean.bin`: `u32@+1 != 0`
//! 896/896, `u32@+1 == u32@+60` 896/896, `u32@+5 == 0` 896/896, `u32@+64 == 0`
//! 896/896. [`MIN_AT`], [`MAX_AT`], [`BLOCK`] and the `FF FF` are unaffected, so
//! [`multiply`], which writes only the two scalars, was never wrong; what was
//! wrong is [`OutputBlock::item`], always read as 0, and so every item id
//! `desert_harvest::gatherer::remember` stored and `hook::reapply` cross-checks
//! against. See `docs/findings/2026-09-12-water-wells.md` section 5.
//!
//! That signature (count 1..=64, every block valid, `1 <= min <= max <=`
//! [`MAX_QTY`]) was cross-checked offline against the whole 22 MB table: inside
//! the 275 gather records the DMM pack edits it finds exactly the 587 blocks the
//! pack touches, and nothing else: no false positives.
//! This ports the pack's original locator, `desert-gatherer-dmm/rebase.py`,
//! which is now the Rust `dmm-rebase` in `crates/tools/src/bin/dmm-rebase.rs`.
//!
//! # The loader
//!
//! The record loader is `FUN_140385cd0` (RVA `0x385cd0`) on build 25246367,
//! `FUN_14038ba40` (RVA `0x38ba40`) on 25477059 and RVA `0x3d89a0` on
//! 26345091. Its own prologue bytes occur hundreds of times in the exe (every
//! static-info table has a loader built from the same template), so it cannot
//! be signature-scanned directly. [`resolve_record_loader_for`] instead goes
//! through its caller, the accessor, which is itself one of 154 copies of
//! another template; the copy for `gimmickinfo` is the one whose
//! `lea r8,[rip+disp]` points at the C string `"gimmickinfo\0"` (that string
//! occurs exactly once in the exe).
//!
//! The addresses below are build 26345091's (the same on 26356046 and 26361232) and are here to be read beside a
//! disassembly rather than relied on: all of them move when the game updates,
//! and nothing in this file resolves anything by address.
//!
//! ```text
//! +3dd975: 48 8B 1D ...           mov    rbx,[rip+slot]    <- the slot load
//! +3dd97c: 3B 53 08               cmp    edx,[rbx+0x8]     ; count check
//! ...
//! +3dd9a8: 45 33 C9               xor    r9d,r9d           <- the accessor head
//! +3dd9ab: 4C 8D 05 ...           lea    r8,[rip+name]     ; -> "gimmickinfo"
//! +3dd9b2: 48 8B 53 10            mov    rdx,[rbx+0x10]    ; end of the head
//! +3dd9b6: 49 8D 4B 10            lea    rcx,[r11+0x10]    <- the tail
//! +3dd9ba: E8 ...                 call   ...               ; loads the file
//! ...
//! +3dda34: 4C 8D 4C 24 30         lea    r9,[rsp+0x30]     <- the loader call
//! +3dda39: 48 8D 94 24 88 00 00 00 lea   rdx,[rsp+0x88]
//! +3dda41: 48 8B CB               mov    rcx,rbx
//! +3dda44: E8 ...                 call   +3d89a0           ; the loader
//! ```
//!
//! # The four encodings of the accessor head
//!
//! The accessor is the **same** template in four encodings of its first two
//! instructions, and [`ACCESSORS`] holds all four as 14-byte heads ending in
//! `mov rdx,[rbx+0x10]`. What follows the head, the instruction that sets up
//! the file-load call's first argument, is read by [`parse_accessor_tail`]
//! instead of being part of the pattern, because that is the part the compiler
//! changed between builds:
//!
//! ```text
//!   xor r9d,r9d   the table name into r8          25477059  26345091
//!   45 33 C9      4C 8D 05   lea r8,[rip+name]          98       132
//!   45 33 C9      4C 8B 05   mov r8,[rip+cell]          13        20
//!   45 31 C9      4C 8D 05   lea r8,[rip+name]          31         2
//!   45 31 C9      4C 8B 05   mov r8,[rip+cell]           7         0
//!                                                 --------  --------
//!                                                      149       154
//!
//!   tail                                          25477059  26345091
//!   48 8D 4C 24 d8 E8   lea rcx,[rsp+d8]   RspDisp8    149         0
//!   49 8D 4B d8 E8      lea rcx,[r11+d8]   R11Disp8      0       154
//! ```
//!
//! On 26345091 the compiler keeps the frame pointer in `r11` and every tail is
//! `lea rcx,[r11+0x10]`. A head whose tail is neither shape is counted and
//! logged once ([`AccessorSites::rejected`]): that line is the early warning
//! for the next codegen change, and a census that drops is the first thing to
//! check after a game update.
//!
//! Two axes of the head, independent of each other:
//!
//! * `45 33 C9` and `45 31 C9` are the very same instruction. `31 /r` is
//!   `xor r/m32,r32` and `33 /r` is `xor r32,r/m32`; with `r9d` on both sides
//!   they encode the identical operation, and the compiler emitted both.
//! * `lea` names the string directly, `mov` loads a pointer **cell** whose `u64`
//!   is the string's **virtual address**. That indirection is the only reason
//!   [`resolve_manager_slot_based`] needs an `image_base` at all, and the only
//!   reason [`resolve_manager_slot`], which has none, cannot see all of them.
//!
//! The census is one accessor per static-info type, each passing a different
//! name: 149 types on 25477059, 154 on 26345091 (`crates/tools/static-info-names.txt`
//! lists the names). The old `45 33 C9`-only scan reached 111 of the 149 and
//! left [`DROPSET_TABLE`], the dispatch-mission reward table, invisible.
//!
//! # The loader call
//!
//! The call to the loader is read by [`parse_loader_call`] over a small
//! grammar rather than matched as one pattern, because its run-up is where
//! instruction scheduling moves things:
//!
//! ```text
//!   4C 8D 4C 24 30               lea   r9,[rsp+0x30]
//!   [44 0F B7 C0+r | 44 8B C0+r] movzx r8d,r16 | mov r8d,r32   (optional)
//!   48 8D 54 24 d8 | 48 8D 94 24 d32   lea rdx,[rsp+d]
//!   48 8B CB | 48 89 D9          mov   rcx,rbx (both encodings)
//!   E8 rel32                     call  the loader
//! ```
//!
//! It is searched forward from the accessor's own `call`, at most
//! [`B_WINDOW`] bytes and never past the next accessor head, and its target
//! must open with [`LOADER_PROLOGUE`]. On 26345091, 153 of the 154 accessors
//! have one, every one with a disp32 `lea rdx`: 127 `48 8B CB` with no index
//! load, 1 `48 89 D9`, 24 `mov r8d,edi` and 1 `mov r8d,esi`. 151 of those reach
//! a target with the prologue (two land on a `jmp` thunk), and the one without is
//! `characterinfo`, whose accessor is obfuscated code. The 26356046 hotfix is
//! the same census with `tribeinfo` (the one `48 89 D9` call) obfuscated too:
//! 152 calls, 150 with the prologue, nothing else moved. On 26361232 all 154
//! have one again: 127 `48 8B CB`, 2 `48 89 D9` (`characterinfo` and
//! `tribeinfo`, neither with an index load) and 25 `mov r8d`; 152 reach the
//! prologue, the two `jmp` thunks being the same two. A table whose call is
//! missing is an error for that table only; for `gimmickinfo` the gatherer
//! treats it as fatal.
//!
//! # The stream layout
//!
//! The loader's fourth argument is a stream object the accessor builds on its
//! own stack at `[rsp+0x30]` (the `lea r9` above) just before the call, and the
//! gatherer reads three of its fields: the table body's address, its size and
//! the cursor. Through 25477059 they were at `+0x10`/`+0x18`/`+0x1c`; on
//! 26361232 the object grew two zeroed qwords and they are at
//! `+0x20`/`+0x28`/`+0x2c`, which read the hook a zero buffer and a zero size
//! for a whole dry run before anyone noticed. So the offsets are read off the
//! accessor's fill sequence ([`parse_stream_fill`], [`AccessorSites::stream_layout`])
//! rather than assumed: on 26361232 all 152 sites with a resolved loader call
//! parse, every one to [`StreamLayout::NEW`].
//!
//! # The manager slot
//!
//! Every copy of the template loads its table's manager object into `rbx`
//! from a **global pointer slot**, by a RIP-relative `mov rbx,[rip+disp32]`
//! (`48 8B 1D ...`) a short way *before* the accessor head, immediately
//! followed by `cmp r32,[rbx+0x8]` (`3B`, ModRM `0x43 | reg << 3`, `08`), the
//! bounds check against the `u32` record count at `manager+8`. The register
//! was `edi` (`3B 7B 08`) through 25477059 and is `edx` (`3B 53 08`) on all
//! 154 copies on 26345091; [`is_slot_cmp`] accepts any 32-bit register.
//!
//! On 26345091 every copy has its slot load in the [`SLOT_WINDOW`] bytes
//! before the head, followed by the bounds check: 0x33 before it in 130 copies
//! and 0x3B in 24 (110 and 22 of the 132 `45 33 C9` + `lea` ones). If more
//! than one checked load is in the window, the nearest is taken and logged.
//! [`resolve_manager_slot`] returns the **RVA** of that slot for a named
//! table, so the looter carries no addresses as constants.
//!
//! [`resolve_manager_slot_based`] is handed an image base and so reaches **any**
//! of the four encodings, `FactionNode` and `Skill` included.
//! [`resolve_manager_slot`] is deliberately left without one rather than being
//! made a wrapper with an extra argument: the looter calls it on every startup
//! and has no image base to hand at that point, and the two tables it asks
//! about are both `lea` copies.

use crate::pattern::{self, Pattern};

/// Bytes per resource-output block.
pub const BLOCK: usize = 68;
/// Offset of the `u64` minimum inside a block.
pub const MIN_AT: usize = 42;
/// Offset of the `u64` maximum inside a block.
pub const MAX_AT: usize = 50;
/// Offset of the `u32` item id inside a block. Immediately after the flag byte,
/// and followed by four bytes of zero padding at [`PAD_AT`], which is where
/// this constant wrongly pointed until 2026-09-12 (see the module docs).
pub const ITEM_AT: usize = 1;
/// Offset of the block's second copy of the item id, four bytes after the
/// `FF FF`. The signature demands the two copies agree, which is what makes
/// either of them usable as an identity check against a parsed block object
/// later (`docs/reference-internals.md` section 16: the parsed block carries
/// the item id at `block+0x68`, and the parsed list entry's `entry+0x08` is
/// *not* a second copy of it: that field is the raw block's [`PAD_TAIL_AT`],
/// which reads zero on every gather block and nonzero on the 16 lists
/// `block_ok` excludes). What that field *is* has no name: section 16.1 called
/// it `DropInfoData._dropTagNameHash`, and that was wrong - the tag hash is at
/// raw block +22, parsed `block+0x0C` (`docs/findings/2026-09-13-buff-stat-census.md`
/// section 4.1, nonzero on 606 of 606 gather blocks).
pub const ITEM_TAIL_AT: usize = 60;
/// The two four-byte zero pads, one after each copy of the item id. Both read
/// `0` on all 896 blocks of the clean body and the signature only requires them
/// to *agree*, not to be zero; `block_ok`'s own comment records the 16 lists
/// that requirement is what excludes.
pub const PAD_AT: usize = ITEM_AT + 4;
/// The second of the two pads. See [`PAD_AT`].
pub const PAD_TAIL_AT: usize = ITEM_TAIL_AT + 4;

/// Largest plausible block count in one output list. Vanilla lists are far
/// smaller; the bound is what keeps the scanner from walking off a random `u32`.
pub const MAX_COUNT: u32 = 64;
/// Largest plausible yield. Vanilla min/max are 1..=8.
pub const MAX_QTY: u64 = 100_000;

/// Bytes [`crate::hook`] steals from the loader's prologue. The first 12 bytes
/// are `mov [rsp+0x20],rbx; mov [rsp+0x10],rdx; push rbp; push rsi`: four whole
/// position-independent instructions, no RIP-relative operand, no branch.
pub const LOADER_STOLEN: usize = 12;
/// The bytes those 12 must be, checked before patching (a wrong patch is a
/// crash to desktop; a refusal is just a degraded plugin).
pub const LOADER_PROLOGUE: [u8; LOADER_STOLEN] =
    [0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56];

/// One encoding of the accessor template's head: `xor r9d,r9d`, the table name
/// into `r8`, `mov rdx,[rbx+0x10]`. What follows is the tail, which
/// [`parse_accessor_tail`] reads.
struct Accessor {
    /// Short name for the two bytes that vary, used in the error messages.
    label: &'static str,
    /// The head bytes, [`A_HEAD_LEN`] of them, from the `xor` through the
    /// `mov rdx,[rbx+0x10]`.
    head: &'static str,
    /// `true` when the RIP target is a pointer **cell** holding the name's
    /// virtual address (`mov r8,[rip+cell]`), `false` when the target is the
    /// name string itself (`lea r8,[rip+name]`).
    indirect: bool,
}

/// The accessor head in all four encodings the compiler emitted, with each
/// one's copy count on build 26345091 beside it (the module docs have the
/// 25477059 counts too).
///
/// [`A_DISP_AT`] and [`A_RIP_AT`] hold for all four: the encodings differ only
/// in which `xor` opcode was used and in the opcode byte that chooses `lea`
/// over `mov`, and neither moves the disp32.
const ACCESSORS: [Accessor; 4] = [
    Accessor {
        label: "45 33 C9 + lea r8",
        head: "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10",
        indirect: false,
    }, // 132 copies
    Accessor {
        label: "45 33 C9 + mov r8",
        head: "45 33 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10",
        indirect: true,
    }, // 20 copies
    Accessor {
        label: "45 31 C9 + lea r8",
        head: "45 31 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10",
        indirect: false,
    }, // 2 copies
    Accessor {
        label: "45 31 C9 + mov r8",
        head: "45 31 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10",
        indirect: true,
    }, // 0 copies on 26345091, 7 on 25477059
];
/// The labels of [`ACCESSORS`], in order, for a census printed by a caller.
pub const ACCESSOR_LABELS: [&str; 4] =
    [ACCESSORS[0].label, ACCESSORS[1].label, ACCESSORS[2].label, ACCESSORS[3].label];
/// Length of every accessor head; the tail starts here.
pub const A_HEAD_LEN: usize = 14;
/// Offset of the disp32 within an accessor hit, and the length of the
/// name-loading instruction from the hit (RIP is the address of the *next*
/// instruction).
const A_DISP_AT: usize = 6;
const A_RIP_AT: usize = 10;
/// Cap on the accessor hits collected per encoding. The largest real count is
/// 132; a scan that hits this cap has stopped being a census and the error
/// messages say so by printing the counts.
const MAX_SITES: usize = 4096;

/// How an accessor's file-load call gets its first argument, the instruction
/// right after the head. The disp8 is carried but never constrained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessorTail {
    /// `48 8D 4C 24 d8`: `lea rcx,[rsp+d8]`, every copy through build
    /// 25477059 (`d8` was `0x70`).
    RspDisp8(u8),
    /// `49 8D 4B d8`: `lea rcx,[r11+d8]`, every copy on build 26345091, where
    /// the frame pointer lives in `r11` (`d8` is `0x10`).
    R11Disp8(u8),
}

/// Read the tail of an accessor: `b` starts right after the head. Returns the
/// shape and the offset of the file-load call's `E8` within `b`, or `None`
/// when `b` is neither shape (or too short to say).
pub fn parse_accessor_tail(b: &[u8]) -> Option<(AccessorTail, usize)> {
    match b {
        [0x48, 0x8D, 0x4C, 0x24, d, 0xE8, ..] => Some((AccessorTail::RspDisp8(*d), 5)),
        [0x49, 0x8D, 0x4B, d, 0xE8, ..] => Some((AccessorTail::R11Disp8(*d), 4)),
        _ => None,
    }
}

/// One accessor-template copy: where its head is, which tail it has and
/// where the `E8` of its file-load call sits. All three are image offsets
/// (RVAs for an image laid out by RVA).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessorSite {
    /// Offset of the head, the `xor r9d,r9d`.
    pub at: usize,
    /// The tail [`parse_accessor_tail`] read.
    pub tail: AccessorTail,
    /// Offset of the file-load call's `E8`, where the search for the loader
    /// call starts.
    pub call_at: usize,
}

/// How the index reaches `r8d` inside a loader call, when it is set there at
/// all. On 26345091 the `movzx` moved earlier, out of the call's run-up, in
/// every copy that has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexLoad {
    /// Nothing between `lea r9` and `lea rdx`.
    None,
    /// `44 0F B7 C0+r`: `movzx r8d,r16`, the 25477059 shape.
    Movzx,
    /// `44 8B C0+r`: `mov r8d,r32`, the tables indexed by a full `u32`.
    Mov,
}

/// One loader call as [`parse_loader_call`] read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoaderCall {
    /// What sits between the `lea r9` and the `lea rdx`.
    pub index: IndexLoad,
    /// `true` for `lea rdx,[rsp+disp32]` (`48 8D 94 24`), `false` for the
    /// disp8 form (`48 8D 54 24`).
    pub rdx_disp32: bool,
    /// `true` for `mov rcx,rbx` encoded `48 89 D9`, `false` for `48 8B CB`.
    pub rcx_89: bool,
    /// Offset of the call's `E8` from the start of the parsed bytes; the
    /// rel32 follows it.
    pub call_at: usize,
}

/// Read a call to the record loader at the start of `b`:
///
/// ```text
/// 4C 8D 4C 24 30                 lea r9,[rsp+0x30]
/// [44 0F B7 C0+r | 44 8B C0+r]   optional index load into r8d
/// 48 8D 54 24 d8 | 48 8D 94 24 d32
/// 48 8B CB | 48 89 D9            mov rcx,rbx
/// E8                             call
/// ```
///
/// `None` for anything else. Never reads past `b`.
pub fn parse_loader_call(b: &[u8]) -> Option<LoaderCall> {
    let rest = b.strip_prefix(&[0x4C, 0x8D, 0x4C, 0x24, 0x30][..])?;
    let (index, rest) = match rest {
        [0x44, 0x0F, 0xB7, m, tail @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Movzx, tail),
        [0x44, 0x8B, m, tail @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Mov, tail),
        _ => (IndexLoad::None, rest),
    };
    let (rdx_disp32, rest) = match rest {
        [0x48, 0x8D, 0x54, 0x24, _, tail @ ..] => (false, tail),
        [0x48, 0x8D, 0x94, 0x24, _, _, _, _, tail @ ..] => (true, tail),
        _ => return None,
    };
    let (rcx_89, rest) = match rest {
        [0x48, 0x8B, 0xCB, tail @ ..] => (false, tail),
        [0x48, 0x89, 0xD9, tail @ ..] => (true, tail),
        _ => return None,
    };
    if rest.first() != Some(&0xE8) {
        return None;
    }
    Some(LoaderCall { index, rdx_disp32, rcx_89, call_at: b.len() - rest.len() })
}

/// How far past the accessor's file-load `E8` a loader call may **start**.
/// Measured on 26345091 over the 153 accessors that have one (and on 26361232
/// over all 154, with the same range): 0x74 to 0x7E
/// (0x74 for the `mov r8d,r32` copies, 0x79 for the one `mov r8d,esi`, 0x7A
/// to 0x7E for the rest). Set to the maximum rounded up to 0x80, plus 0x40.
pub const B_WINDOW: usize = 0xC0;

/// Where the loader's fourth argument, the stream object, keeps the three
/// fields the gatherer reads: the raw table body's address, its size and the
/// cursor parked at the start of the record being loaded. Offsets within the
/// stream object, which lives on the accessor's stack at `[rsp+r9_disp]`.
///
/// `(0x10, 0x18, 0x1c)` through 25477059 (and, unmeasured, 26345091 and
/// 26356046); `(0x20, 0x28, 0x2c)` on 26361232, where the object grew by 0x10
/// in front of the buffer pointer (two new zeroed qwords at `+0x10`/`+0x18`).
/// That move left the gatherer silently inert for one build, which is why the
/// layout is now read off the accessor's own fill sequence
/// ([`parse_stream_fill`]) rather than assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLayout {
    /// `ptr`, the raw table body.
    pub buf: u32,
    /// `u32`, the table body's size.
    pub size: u32,
    /// `u32`, the record being loaded's file offset at hook time.
    pub cursor: u32,
}

impl StreamLayout {
    /// The layout through build 25477059.
    pub const OLD: Self = Self { buf: 0x10, size: 0x18, cursor: 0x1C };
    /// The layout on build 26361232.
    pub const NEW: Self = Self { buf: 0x20, size: 0x28, cursor: 0x2C };
    /// Every layout seen so far, newest first: what a runtime probe tries when
    /// the fill sequence does not parse.
    pub const KNOWN: [Self; 2] = [Self::NEW, Self::OLD];
}

/// `mov rax,[base]` with no displacement: an optional REX (`48`, or `49` for
/// `r8`..`r15`), `8B`, ModRM mod 00 reg `rax`. Returns `(base register as REX.B
/// and r/m, length)`. `rsp`/`rbp` r/m values (SIB, RIP) are not a holder.
fn holder_load(b: &[u8]) -> Option<((u8, u8), usize)> {
    match b {
        [rex @ (0x48 | 0x49), 0x8B, m, ..] if m & 0xF8 == 0x00 && !matches!(m & 7, 4 | 5) => {
            Some(((rex & 1, m & 7), 3))
        }
        _ => None,
    }
}

/// `mov eax,[base+d8]` or `mov eax,[base+index+d8]`: an optional REX without W
/// (`40`..`47`), `8B`, ModRM mod 01 reg `eax`, a SIB when r/m is 100, then the
/// disp8. Returns `(base as REX.B and r/m, disp8, length)`.
fn dword_load(b: &[u8]) -> Option<((u8, u8), u8, usize)> {
    let (rex, rest) = match b {
        [r @ 0x40..=0x47, rest @ ..] => (*r, rest),
        _ => (0, b),
    };
    let pre = b.len() - rest.len();
    match rest {
        [0x8B, 0x44, sib, d, ..] => Some(((rex & 1, sib & 7), *d, pre + 4)),
        [0x8B, m, d, ..] if m & 0xC0 == 0x40 && m & 0x38 == 0 && m & 7 != 4 => {
            Some(((rex & 1, m & 7), *d, pre + 3))
        }
        _ => None,
    }
}

/// `mov [rsp+d8],rax` (`48 89 44 24 d8`) when `wide`, else `mov [rsp+d8],eax`
/// (`89 44 24 d8`), at the start of `b`: its disp8.
fn rsp_store(b: &[u8], wide: bool) -> Option<u8> {
    match (wide, b) {
        (true, [0x48, 0x89, 0x44, 0x24, d, ..]) | (false, [0x89, 0x44, 0x24, d, ..]) => Some(*d),
        _ => None,
    }
}

/// Read the stream layout off the instructions that fill the stream object,
/// as the accessor emits them between its file-load call and the loader call.
/// The `gimmickinfo` copy on 26361232:
///
/// ```text
/// 48 8B 01        mov rax,[rcx]            ; holder -> buffer
/// 48 89 44 24 d8  mov [rsp+d8],rax         ; buf = d8 - r9_disp
/// 8B 41 0C        mov eax,[rcx+0xc]        ; same holder
/// 89 44 24 d8     mov [rsp+d8],eax         ; size
/// 8B 42 04        mov eax,[rdx+0x4]        ; entry[idx].file_offset
/// 89 44 24 d8     mov [rsp+d8],eax         ; cursor
/// ```
///
/// The stores are the same in every copy; the loads are register-allocated
/// per copy, and only the forms seen on 26361232 are accepted: the holder in
/// `rcx` or `rdx` (any base register without a SIB or RIP operand), and the
/// cursor through `[r+d8]`, `[r9+0x8]` (`41 8B 41 08`, 16-byte offset entries)
/// or a SIB `[rdx+r15+0x4]` (`42 8B 44 3A 04`). The cursor is the one
/// `mov eax,[m+d8]; mov [rsp+d8],eax` pair that is not the size's.
///
/// `code` is the bytes from the end of the accessor's file-load call to the
/// start of the loader call; `r9_disp` is the disp8 of the loader call's
/// `lea r9,[rsp+d8]`, where the stream object starts. Each field must be found
/// exactly once, and each store must land at or past `r9_disp` and below 0x80
/// (a positive disp8). `None` for anything else. Never panics.
pub fn parse_stream_fill(code: &[u8], r9_disp: u8) -> Option<StreamLayout> {
    let field = |d: u8| -> Option<u32> {
        if d >= 0x80 || d < r9_disp {
            return None;
        }
        Some(u32::from(d - r9_disp))
    };
    let tail = |i: usize| code.get(i..).unwrap_or_default();

    // The buffer: the one holder load followed by a qword store.
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

    // Every dword load-and-store pair, identified by where its store sits: a
    // disp8 of 0x40..0x47 just before a load also reads as a REX prefix, and
    // that is the same pair decoded twice, not a second one. The later start
    // (the decode without the phantom prefix) wins, since a real REX would
    // have to sit right after the previous instruction's last byte anyway.
    let mut pairs: Vec<(usize, (u8, u8), u8, u8)> = Vec::new();
    for i in 0..code.len() {
        let Some((reg, disp, n)) = dword_load(tail(i)) else { continue };
        let Some(d) = rsp_store(tail(i + n), false) else { continue };
        match pairs.last_mut() {
            Some(last) if last.0 == i + n => *last = (i + n, reg, disp, d),
            _ => pairs.push((i + n, reg, disp, d)),
        }
    }
    // The size off the same holder at +0xC; the cursor is the one other pair.
    let (sizes, cursors): (Vec<_>, Vec<_>) =
        pairs.iter().partition(|&&(_, reg, disp, _)| reg == holder && disp == 0x0C);
    match (sizes.as_slice(), cursors.as_slice()) {
        ([(_, _, _, s)], [(_, _, _, c)]) => {
            Some(StreamLayout { buf: field(buf_d)?, size: field(*s)?, cursor: field(*c)? })
        }
        _ => None,
    }
}

/// What one candidate [`StreamLayout`] reads out of a live stream object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamReading {
    pub layout: StreamLayout,
    /// The buffer pointer, `None` when the read failed.
    pub buf: Option<usize>,
    pub size: Option<u32>,
    pub cursor: Option<u32>,
}

/// Pick the one candidate layout a live stream validates, for when
/// [`parse_stream_fill`] did not parse: its buffer pointer is non-null, its
/// size is `1..=cap`, and its cursor equals `expected_cursor`
/// (`offsets[idx].file_offset`). Returns the winner and why it won, or why
/// none did. Two candidates validating is an ambiguity, not a choice.
pub fn choose_layout(
    candidates: &[StreamReading],
    expected_cursor: u32,
    cap: u32,
) -> Result<(StreamLayout, String), String> {
    let mut why_not = Vec::new();
    let mut winners = Vec::new();
    for r in candidates {
        let l = r.layout;
        let verdict = match (r.buf, r.size, r.cursor) {
            (Some(0) | None, _, _) => Err("buffer pointer null or unreadable".to_string()),
            (_, None, _) | (_, _, None) => Err("size or cursor unreadable".to_string()),
            (Some(_), Some(s), _) if s == 0 || s > cap => Err(format!("size 0x{s:X} not in 1..=0x{cap:X}")),
            (Some(_), Some(_), Some(c)) if c != expected_cursor => {
                Err(format!("cursor 0x{c:X} != offsets table 0x{expected_cursor:X}"))
            }
            (Some(b), Some(s), Some(c)) => {
                Ok(format!("buffer 0x{b:X} non-null, size 0x{s:X} in range, cursor 0x{c:X} matches the offsets table"))
            }
        };
        match verdict {
            Ok(why) => winners.push((l, why)),
            Err(e) => why_not.push(format!("buffer +0x{:X}: {e}", l.buf)),
        }
    }
    match winners.as_slice() {
        [(l, why)] => Ok((*l, why.clone())),
        [] => Err(why_not.join("; ")),
        many => Err(format!(
            "{} layouts validate ({}), refusing to guess",
            many.len(),
            many.iter().map(|(l, _)| format!("buffer +0x{:X}", l.buf)).collect::<Vec<_>>().join(", ")
        )),
    }
}

/// `mov rbx,[rip+disp32]`, the load of a table's manager object out of its
/// global pointer slot. Seven bytes: opcode at +0, disp32 at [`SLOT_DISP_AT`],
/// RIP (the next instruction) at [`SLOT_RIP_AT`].
const PAT_SLOT: &str = "48 8B 1D ?? ?? ?? ??";
const SLOT_DISP_AT: usize = 3;
const SLOT_RIP_AT: usize = 7;
/// `cmp r32,[rbx+0x8]`, the record-count bounds check that follows the real
/// slot load: `3B`, a ModRM with mod 01, rm `rbx` and any register (`0x43`,
/// `0x4B`, ... `0x7B`), disp8 `08`. `edi` (`3B 7B 08`) through 25477059, `edx`
/// (`3B 53 08`) on 26345091.
pub fn is_slot_cmp(b: &[u8; 3]) -> bool {
    matches!(b, [0x3B, m, 0x08] if m & 0xC7 == 0x43)
}
/// How far back from an accessor head the slot load is looked for. The real
/// distance on 26345091 is 0x33 or 0x3B; the window is generous.
pub const SLOT_WINDOW: usize = 0x60;

/// The name the `gimmickinfo` accessor passes: the record loader the
/// gatherer half of `desert-harvest` hooks is the one reached through this string.
pub const GIMMICK_TABLE: &[u8] = b"gimmickinfo";
/// The name the `iteminfo` accessor passes.
pub const ITEM_TABLE: &[u8] = b"iteminfo";
/// The name the `FactionNode` accessor passes. Reachable only through the
/// indirect template variant, so only [`resolve_manager_slot_based`] finds it.
/// Its records carry the dispatch-mission ("faction operation") sub-records the
/// diagnostic subsystem in `desert-dispatch` reads.
pub const FACTION_NODE_TABLE: &[u8] = b"FactionNode";
/// The name the `dropsetinfo` accessor passes: the dispatch-mission reward
/// table. Its accessor is a `45 31 C9` + `lea` copy of the template, so it was
/// invisible to this module until the scan covered all four encodings. On build
/// 25246367 its manager slot is `0x6C328A8` and its record loader `0x437E70`.
pub const DROPSET_TABLE: &[u8] = b"dropsetinfo";
/// The name the `buffinfo` accessor passes: the game's buff table, whose
/// records carry the `BuffData` objects that move a drop rate. A `45 33 C9` +
/// `lea` copy of the template; on build 25246367 its manager slot is `0x6C367C8`
/// and its record loader `0x5FE880`. Walked **read-only** by the dispatch
/// subsystem's buff census (`DumpBuffs`), which writes nothing and installs
/// nothing.
pub const BUFF_TABLE: &[u8] = b"buffinfo";
/// The name the `statusinfo` accessor passes: the stat table the buffs above
/// name by row index, and where `AddMoneyDropRate` is a record rather than an
/// enum value. Also a `45 33 C9` + `lea` copy; on build 25246367 its manager
/// slot is `0x6C2E328` and its record loader `0x512A30`. Same census, same
/// read-only pass.
pub const STATUS_TABLE: &[u8] = b"statusinfo";
/// The name the `characterinfo` accessor passes: the species table, indexed by
/// the `u16` row a creature's `ClientStatusActorComponent+0x30` holds. A
/// `45 31 C9` + `lea` copy of the template on 26361232 (census site
/// `+0x8A29CB8`), so the base-less scan reaches it. Its accessor body was
/// obfuscated code on 26345091 and 26356046; only the slot load in front of
/// the head is used here. Read-only, by `desert-harvest`'s species set.
pub const CHARACTER_TABLE: &[u8] = b"characterinfo";

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?)?.try_into().ok().map(u32::from_le_bytes)
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o.checked_add(8)?)?.try_into().ok().map(u64::from_le_bytes)
}

/// Which of the two record-header layouts a table uses.
///
/// ```text
/// KeyFirst   u32 key, u32 name_len, name, 0x00                     through 25477059
/// NameFirst  u32 name_len, name, u32 key, 0x00, u32 path_len, path, through 26361232
///            u8 flag (0 or 1)
/// ```
///
/// On 26361232 every record of the 14210 in the table body is `NameFirst`:
/// the name has no terminator, the key follows it, and a prefab path (all but
/// one start `/object`, `gimmick_item_flowers_01` starts `/character`) and a
/// flag byte (0 on 11511 records, 1 on 2699) close the header. The keys are
/// the same ids as before, so `collect`'s tables stay valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderShape {
    KeyFirst,
    NameFirst,
}

/// The fixed part at the front of every record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordHeader {
    pub key: u32,
    pub name: String,
    /// The prefab path, `NameFirst` only.
    pub path: Option<String>,
    pub shape: HeaderShape,
    /// Offset of the first byte past the whole header (the NUL on
    /// `KeyFirst`, the flag byte on `NameFirst`): where the fields start.
    pub body: usize,
}

/// `len` printable ASCII bytes at `at`, as a string.
fn printable(rec: &[u8], at: usize, len: usize) -> Option<String> {
    let s = rec.get(at..at.checked_add(len)?)?;
    s.iter().all(|&b| (0x20..=0x7E).contains(&b)).then(|| String::from_utf8_lossy(s).into_owned())
}

/// Parse the start of `rec` as a header of exactly `shape`, strictly.
///
/// `KeyFirst`: the name length is 1..=255, every name byte is printable and
/// the NUL is there. `NameFirst`: the same for the name, then the key, a zero
/// byte, a path of 1..=255 printable bytes starting `/`, and a flag of 0 or 1.
/// Every check is what makes a parse usable as a record *detector*. Never
/// reads past `rec`.
pub fn parse_header_as(rec: &[u8], shape: HeaderShape) -> Option<RecordHeader> {
    match shape {
        HeaderShape::KeyFirst => {
            let key = u32_at(rec, 0)?;
            let len = u32_at(rec, 4)? as usize;
            if !(1..=255).contains(&len) {
                return None;
            }
            let name = printable(rec, 8, len)?;
            let nul = 8 + len;
            if *rec.get(nul)? != 0 {
                return None;
            }
            Some(RecordHeader { key, name, path: None, shape, body: nul + 1 })
        }
        HeaderShape::NameFirst => {
            let len = u32_at(rec, 0)? as usize;
            if !(1..=255).contains(&len) {
                return None;
            }
            let name = printable(rec, 4, len)?;
            let k = 4 + len;
            let key = u32_at(rec, k)?;
            if *rec.get(k + 4)? != 0 {
                return None;
            }
            let plen = u32_at(rec, k + 5)? as usize;
            if !(1..=255).contains(&plen) {
                return None;
            }
            let path = printable(rec, k + 9, plen)?;
            if !path.starts_with('/') {
                return None;
            }
            let flag = k + 9 + plen;
            if *rec.get(flag)? > 1 {
                return None;
            }
            Some(RecordHeader { key, name, path: Some(path), shape, body: flag + 1 })
        }
    }
}

/// The one shape the start of `rec` validates as, `None` when neither does or
/// when both do (a `KeyFirst` key below 256 can read as a `NameFirst` name
/// length; the full validation is what settles it, and a tie is not guessed).
pub fn detect_header_shape(rec: &[u8]) -> Option<HeaderShape> {
    let kf = parse_header_as(rec, HeaderShape::KeyFirst).is_some();
    let nf = parse_header_as(rec, HeaderShape::NameFirst).is_some();
    match (kf, nf) {
        (true, false) => Some(HeaderShape::KeyFirst),
        (false, true) => Some(HeaderShape::NameFirst),
        _ => None,
    }
}

/// Parse a record header from the start of `rec`, in whichever shape it
/// validates as ([`detect_header_shape`]). A caller that already knows the
/// table's shape should use [`parse_header_as`]: it is stricter, and it is
/// what the gatherer latches after the first record.
pub fn parse_header(rec: &[u8]) -> Option<RecordHeader> {
    parse_header_as(rec, detect_header_shape(rec)?)
}

/// Is there a well-formed output block at `at`?
fn block_ok(rec: &[u8], at: usize) -> bool {
    let end = match at.checked_add(BLOCK) {
        Some(e) => e,
        None => return false,
    };
    let b = match rec.get(at..end) {
        Some(b) => b,
        None => return false,
    };
    // b is exactly BLOCK long, so it converts to a fixed-size array and the
    // compiler, not a runtime check, proves the constant offsets below.
    let b: &[u8; BLOCK] = match b.try_into() {
        Ok(b) => b,
        Err(_) => return false,
    };
    if b[0] != 1 || b[58] != 0xFF || b[59] != 0xFF {
        return false;
    }
    // The two copies of the item id must agree. Until 2026-09-12 this compared
    // `PAD_AT` against `PAD_TAIL_AT` instead, believing them to be the item id,
    // so what it really tested was that two runs of zeroes matched.
    if b[ITEM_AT..ITEM_AT + 4] != b[ITEM_TAIL_AT..ITEM_TAIL_AT + 4] {
        return false;
    }
    // And the two pads must agree, which the old check is kept as. It is not
    // redundant and not cosmetic: measured over the whole 22 MB clean body,
    // dropping it grows the detector's population from 573 lists / 896 blocks to
    // 589 / 1038. The 16 extra lists are byte-for-byte the same shape (flag 1,
    // `FF FF`, plausible item ids agreeing across `+1` and `+60`, 68-byte stride)
    // and differ only in that every one of their blocks carries a **nonzero**
    // `u32` at `PAD_TAIL_AT` while `PAD_AT` is zero. That field is *not* the
    // high half of a wide item id (that reading was checked and ruled out on
    // 2026-09-12). It is the list entry's own key, `_dropTagNameHash`, which
    // the parsed loop reads into `entry+0x08` (section 16.1), and records that
    // share a value share a drop group. Reading it as a pad is right for every
    // record the plugin edits and wrong in general. None of
    // them is one of the 275 gather records the DMM pack edits; they are chests,
    // dig sites and dungeon loot: `Temple_Chest_01` (key 16060036),
    // `dff_chest_24` (16060043), `gimmick_item_dropset_treasurebox_01`
    // (1005213), `clawmachine_capsule_01` (1007407), `Action_dig_01`
    // (120040092), `gimmick_Dig_land_0001` (1005245), the
    // `gimmick_abyssone_bridge_gate_*` set and `gimmick_marni_teleportation_*`.
    // (An earlier revision of this comment named `player` and
    // `UnnamedTrigger_0` here. Both are false headers: a backwards scan for a
    // record start finds them because nested string fields use the same
    // `u32 len, bytes, NUL` shape as a record name. `player` is preceded by the
    // `u32` 1 and `UnnamedTrigger_0` by 0x01000000, neither a record key. See
    // `docs/findings/2026-09-12-water-wells.md` section 7.)
    // Whether they are gatherable outputs this mod
    // ought to multiply is an open question and deliberately not answered here:
    // this clause is what keeps the population exactly what it has always been,
    // so the item-offset correction changes no yield anywhere.
    if b[PAD_AT..PAD_AT + 4] != b[PAD_TAIL_AT..PAD_TAIL_AT + 4] {
        return false;
    }
    let (min, max) = match (u64_at(b, MIN_AT), u64_at(b, MAX_AT)) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    min >= 1 && min <= max && max <= MAX_QTY
}

/// Is there a well-formed output list (a `u32 count` then `count` blocks) at
/// `at`? Returns the count on success.
fn list_at(rec: &[u8], at: usize) -> Option<u32> {
    let count = u32_at(rec, at)?;
    if count == 0 || count > MAX_COUNT {
        return None;
    }
    // The whole list has to fit, and every block in it has to check out.
    let span = (count as usize).checked_mul(BLOCK)?;
    let end = at.checked_add(4)?.checked_add(span)?;
    if end > rec.len() {
        return None;
    }
    for n in 0..count as usize {
        if !block_ok(rec, at + 4 + n * BLOCK) {
            return None;
        }
    }
    Some(count)
}

/// `(offset of the u32 count, count)` for every valid output list in `rec`, in
/// order.
///
/// Scans byte by byte (the lists are not at a fixed place in a record and the
/// record's other fields are not parsed) and skips past a whole list once one
/// matches, so lists never overlap.
pub fn output_lists(rec: &[u8]) -> Vec<(usize, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 4 <= rec.len() {
        match list_at(rec, i) {
            Some(count) => {
                out.push((i, count));
                i += 4 + count as usize * BLOCK;
            }
            None => i += 1,
        }
    }
    out
}

/// One resource-output block, read out rather than rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputBlock {
    /// Offset of the block's first byte inside the record slice this was read
    /// from, so `offset + MIN_AT` / `offset + MAX_AT` are the two scalars
    /// [`multiply`] would edit.
    pub offset: usize,
    /// The block's item id, taken from [`ITEM_TAIL_AT`], the copy the parsed
    /// entry keeps at `entry+0x08`, which is what `desert_harvest::gatherer`'s live
    /// re-apply cross-checks against. The signature has already proved it equal
    /// to the copy at [`ITEM_AT`].
    pub item: u32,
    /// The vanilla minimum, exactly as the table holds it.
    pub min: u64,
    /// The vanilla maximum.
    pub max: u64,
}

/// Every block of every output list [`output_lists`] finds, in list order and
/// then block order.
///
/// This is the read-only twin of [`multiply`]: same blocks, same order, but it
/// reports what is there instead of what to write. The gatherer uses it to
/// remember a record's vanilla yields at load time, because once the game has
/// parsed a multiplied record the original numbers are gone: the parsed
/// object holds the product, and dividing it back out is not the same thing
/// (`multiply` saturates, and a later multiplier must scale the vanilla value,
/// not the current one).
pub fn output_blocks(rec: &[u8]) -> Vec<OutputBlock> {
    let mut out = Vec::new();
    for (at, count) in output_lists(rec) {
        for n in 0..count as usize {
            let offset = at + 4 + n * BLOCK;
            // `output_lists` only reports a list whose every block fits and
            // passes the signature, so these three reads always succeed; the
            // `else` is what keeps that from being an assumption.
            let (Some(item), Some(min), Some(max)) = (
                u32_at(rec, offset + ITEM_TAIL_AT),
                u64_at(rec, offset + MIN_AT),
                u64_at(rec, offset + MAX_AT),
            ) else {
                continue;
            };
            out.push(OutputBlock { offset, item, min, max });
        }
    }
    out
}

/// One `u64` to rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    /// Offset of the `u64` inside the record slice this was computed from.
    pub offset: usize,
    pub old: u64,
    pub new: u64,
}

/// Both the min and the max of every block of every output list in `rec`,
/// multiplied by `mult` (saturating).
///
/// `mult` of 1 or 0 yields an empty `Vec`: 1 is a no-op and 0 would zero the
/// yields, which is never what a gathering multiplier wants.
pub fn multiply(rec: &[u8], mult: u32) -> Vec<Edit> {
    let mut edits = Vec::new();
    if mult <= 1 {
        return edits;
    }
    let m = mult as u64;
    for (at, count) in output_lists(rec) {
        for n in 0..count as usize {
            let b = at + 4 + n * BLOCK;
            for field in [MIN_AT, MAX_AT] {
                let off = b + field;
                if let Some(old) = u64_at(rec, off) {
                    edits.push(Edit { offset: off, old, new: old.saturating_mul(m) });
                }
            }
        }
    }
    edits
}

/// Write `edits` into `rec`, bounds-checked. Edits whose `u64` does not fit are
/// skipped rather than panicking. Returns the number applied; anything skipped
/// is the difference from `edits.len()`.
///
/// The `old` field is not re-verified here ([`multiply`] read it from these
/// same bytes), so an edit list must not be applied twice.
pub fn apply(rec: &mut [u8], edits: &[Edit]) -> usize {
    let mut applied = 0usize;
    for e in edits {
        let end = match e.offset.checked_add(8) {
            Some(end) => end,
            None => continue,
        };
        if let Some(dst) = rec.get_mut(e.offset..end) {
            dst.copy_from_slice(&e.new.to_le_bytes());
            applied += 1;
        }
    }
    applied
}

/// Read a NUL-terminated string at `rva` and compare it to `want`.
fn c_str_is(img: &[u8], rva: usize, want: &[u8]) -> bool {
    let end = match rva.checked_add(want.len()) {
        Some(e) => e,
        None => return false,
    };
    img.get(rva..end) == Some(want) && img.get(end) == Some(&0)
}

/// Signed RIP-relative helper: `base + disp32`, `None` if it leaves `usize`.
fn rip_target(base: usize, disp: i32) -> Option<usize> {
    if disp >= 0 {
        base.checked_add(disp as usize)
    } else {
        base.checked_sub(disp.unsigned_abs() as usize)
    }
}

fn disp32_at(img: &[u8], o: usize) -> Option<i32> {
    u32_at(img, o).map(|v| v as i32)
}

/// The **RVA** of the name string an accessor site names, however it names it.
///
/// For a `lea` encoding that is the RIP target itself. For a `mov` encoding the
/// RIP target is a pointer cell: the cell holds a *virtual address* (at runtime
/// the loader has relocated it against the real module base, and in a
/// file-backed image it still carries the preferred base), so the name's RVA is
/// that value minus the base the image is laid out for. `image_base` of `None`
/// means the caller has no base to hand, and every `mov` encoding is then
/// unreadable and reports `None`.
fn accessor_name_rva(
    img: &[u8],
    a: usize,
    t: &Accessor,
    image_base: Option<usize>,
) -> Option<usize> {
    let disp = disp32_at(img, a.checked_add(A_DISP_AT)?)?;
    let target = rip_target(a.checked_add(A_RIP_AT)?, disp)?;
    if !t.indirect {
        return Some(target);
    }
    let va = u64_at(img, target)?;
    let rva = usize::try_from(va).ok()?.checked_sub(image_base?)?;
    (rva < img.len()).then_some(rva)
}

/// Log `msg` once per process under `flag`. Native builds never log from
/// here: the unit tests drive these paths on purpose and must not leave a log
/// file beside the test binary.
#[cfg(windows)]
fn log_once(flag: &core::sync::atomic::AtomicBool, msg: impl FnOnce() -> String) {
    if !flag.swap(true, core::sync::atomic::Ordering::Relaxed) {
        crate::log!("{}", msg());
    }
}
#[cfg(not(windows))]
fn log_once(_flag: &core::sync::atomic::AtomicBool, _msg: impl FnOnce() -> String) {}

static LOGGED_REJECTED_TAIL: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static LOGGED_SLOT_TIE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Every accessor-template site in one image, one list per [`ACCESSORS`]
/// encoding.
///
/// Finding a template copy depends on the pattern and the image and **nothing
/// else**: the table name is consulted only afterwards, to pick which of the
/// hits is the one wanted. So the walk that finds the copies is the same walk
/// whatever table is being resolved, and a caller wanting more than one used to
/// pay for it more than once: `desert-dispatch` resolves several tables and
/// scanned the image **eight** times at startup, 0.88 to 1.81 s of its boot
/// measured over six launches on build 25246367.
///
/// Scan once, resolve as often as wanted. The sites are a few hundred entries,
/// so keeping them costs nothing beside finding them again.
///
/// The image base is part of the **scan**, not of each lookup, because it
/// decides how much there is to scan: without one the two indirect encodings
/// are unreadable (their name is only reachable through a VA), so they are not
/// searched at all and the walk carries two patterns rather than four. The
/// looter resolves `iteminfo` and `gimmickinfo` without a base and declines it
/// for exactly that reason; making the base a per-lookup argument quietly
/// doubled its startup scan, which is why it lives here.
pub struct AccessorSites {
    /// The image these came from, as `(pointer, length)`. Checked on every
    /// lookup: a site is an offset into one particular image and means nothing
    /// in another, and answering confidently with the wrong address is the one
    /// failure this module exists to prevent.
    origin: (usize, usize),
    /// The base this was scanned for. `None` means the two indirect encodings
    /// were never searched, so this cannot answer for a table only they name.
    image_base: Option<usize>,
    /// Sites per entry of [`ACCESSORS`], in that order, ascending. The
    /// indirect encodings' lists are empty when [`Self::image_base`] is `None`,
    /// and empty is also what a genuinely absent encoding looks like, which is
    /// why the census is filtered by the same flag rather than by emptiness.
    sites: [Vec<AccessorSite>; 4],
    /// Every head found, recognised tail or not, ascending: where one
    /// accessor's loader-call search must stop.
    heads: Vec<usize>,
    /// Heads whose tail [`parse_accessor_tail`] did not recognise, ascending.
    rejected: Vec<usize>,
}

/// One accessor site with everything the resolvers derive from it, for a
/// census ([`AccessorSites::census`]). Each derivation is independent: a site
/// whose slot load is missing still reports its loader call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteReport {
    /// Index into [`ACCESSOR_LABELS`].
    pub encoding: usize,
    pub site: AccessorSite,
    /// RVA of the name the accessor passes, when it could be read.
    pub name_rva: Option<usize>,
    /// `(offset of the mov rbx, RVA of the slot it loads)`.
    pub slot: Result<(usize, usize), String>,
    /// `(offset of the loader call, its shape, RVA of its target)`. The target
    /// is known to open with [`LOADER_PROLOGUE`].
    pub loader: Result<(usize, LoaderCall, usize), String>,
    /// The stream layout the fill sequence before that loader call encodes
    /// ([`parse_stream_fill`]); `None` without a loader call or a fill that
    /// parses.
    pub stream: Option<StreamLayout>,
}

impl AccessorSites {
    /// Find every template copy in `img` in **one** walk of it, carrying every
    /// encoding a caller with this `image_base` could read: four heads with a
    /// base, two without. A head whose tail is not a known shape is kept out of
    /// the sites, counted in [`Self::rejected`] and logged once.
    ///
    /// The only error is a malformed pattern constant, which is a bug in this
    /// file rather than anything about the image.
    pub fn scan(img: &[u8], image_base: Option<usize>) -> Result<Self, String> {
        let mut sites: [Vec<AccessorSite>; 4] = Default::default();
        let mut searched = Vec::new();
        let mut pats = Vec::new();
        for (i, t) in ACCESSORS.iter().enumerate() {
            // Not searched at all without a base: a hit of an indirect encoding
            // could never be matched to a name, so finding it is pure cost.
            if t.indirect && image_base.is_none() {
                continue;
            }
            pats.push(
                Pattern::parse(t.head)
                    .ok_or_else(|| format!("the accessor head \"{}\" is malformed", t.label))?,
            );
            searched.push(i);
        }
        let mut heads = Vec::new();
        let mut rejected = Vec::new();
        // One pass for every encoding: the walk is the cost here, not the
        // patterns it carries.
        for (i, hits) in searched.into_iter().zip(pattern::find_all_multi(&pats, img, MAX_SITES)) {
            let Some(out) = sites.get_mut(i) else { continue };
            for at in hits {
                heads.push(at);
                let tail_at = at.saturating_add(A_HEAD_LEN);
                match img.get(tail_at..).and_then(parse_accessor_tail) {
                    Some((tail, e8)) => {
                        out.push(AccessorSite { at, tail, call_at: tail_at + e8 })
                    }
                    None => rejected.push(at),
                }
            }
        }
        heads.sort_unstable();
        rejected.sort_unstable();
        if let Some(&first) = rejected.first() {
            log_once(&LOGGED_REJECTED_TAIL, || {
                let bytes = img.get(first + A_HEAD_LEN..).unwrap_or_default();
                format!(
                    "[gimmick] {} accessor heads have a tail that is not lea rcx,[rsp+d8] or \
                     lea rcx,[r11+d8]; first at +0x{first:X}, tail {:02X?}. The template's \
                     codegen changed: those tables are unreachable until gimmick.rs learns it",
                    rejected.len(),
                    bytes.get(..8).unwrap_or(bytes)
                )
            });
        }
        Ok(Self { origin: (img.as_ptr() as usize, img.len()), image_base, sites, heads, rejected })
    }

    /// Refuse to answer about an image these sites were not scanned from.
    ///
    /// Cheap, and the alternative is a plausible-looking RVA read out of the
    /// wrong bytes. Callers hold an `AccessorSites` across several lookups by
    /// design, so handing one the wrong slice is a mistake worth catching.
    fn check(&self, img: &[u8]) -> Result<(), String> {
        if self.origin == (img.as_ptr() as usize, img.len()) {
            return Ok(());
        }
        Err(format!(
            "these accessor sites were scanned from a different image ({} bytes at 0x{:X}, asked \
             about {} bytes at 0x{:X})",
            self.origin.1,
            self.origin.0,
            img.len(),
            img.as_ptr() as usize
        ))
    }

    /// How many sites each encoding has, in [`ACCESSOR_LABELS`] order. An
    /// encoding that was not searched (an indirect one, without a base) is 0.
    pub fn counts(&self) -> [usize; 4] {
        let mut out = [0; 4];
        for (o, s) in out.iter_mut().zip(self.sites.iter()) {
            *o = s.len();
        }
        out
    }

    /// Heads that matched but whose tail is neither known shape. Zero on
    /// 26345091; anything else is the next codegen change arriving.
    pub fn rejected(&self) -> &[usize] {
        &self.rejected
    }

    /// Every site with its slot, its name and its loader call, ascending by
    /// encoding then address. This is what a census test asserts over; the
    /// resolvers below answer one table at a time from the same derivations.
    pub fn census(&self, img: &[u8]) -> Result<Vec<SiteReport>, String> {
        self.check(img)?;
        let mut out = Vec::new();
        for (encoding, (t, list)) in ACCESSORS.iter().zip(self.sites.iter()).enumerate() {
            for site in list {
                let loader = self.loader_call(img, site);
                let stream = loader.as_ref().ok().and_then(|&(b, _, _)| stream_fill_at(img, site, b));
                out.push(SiteReport {
                    encoding,
                    site: *site,
                    name_rva: accessor_name_rva(img, site.at, t, self.image_base),
                    slot: slot_load_before(img, site.at),
                    loader,
                    stream,
                });
            }
        }
        Ok(out)
    }

    /// [`resolve_manager_slot_based`] against sites already found, and
    /// [`resolve_manager_slot`] when these were scanned without a base.
    pub fn manager_slot(&self, img: &[u8], table: &[u8]) -> Result<usize, String> {
        slot_load_before(img, named_accessor(self, img, table)?.at).map(|(_, slot)| slot)
    }

    /// The loader call inside `site`: searched forward from the site's own
    /// file-load `E8`, starting at most [`B_WINDOW`] bytes past it and never
    /// at or past the next accessor head, and accepted only if its target
    /// opens with [`LOADER_PROLOGUE`]. Returns `(call offset, shape, target)`.
    fn loader_call(
        &self,
        img: &[u8],
        site: &AccessorSite,
    ) -> Result<(usize, LoaderCall, usize), String> {
        let a = site.at;
        let start = site.call_at;
        let next = self.heads.get(self.heads.partition_point(|&h| h <= a)).copied();
        let limit = start.saturating_add(B_WINDOW).min(next.unwrap_or(usize::MAX)).min(img.len());
        let mut hits: Vec<(usize, LoaderCall)> = Vec::new();
        for o in start..limit {
            if img.get(o) != Some(&0x4C) {
                continue;
            }
            if let Some(c) = img.get(o..).and_then(parse_loader_call) {
                hits.push((o, c));
            }
        }
        let (b, call) = match hits.as_slice() {
            [one] => *one,
            [] => {
                return Err(format!(
                    "no loader call within 0x{B_WINDOW:X} bytes of the accessor at +0x{a:X} \
                     (searched +0x{start:X}..+0x{limit:X})"
                ))
            }
            many => {
                return Err(format!(
                    "{} loader calls within 0x{B_WINDOW:X} bytes of the accessor at +0x{a:X}: \
                     {:x?}",
                    many.len(),
                    many.iter().map(|&(h, _)| h).collect::<Vec<_>>()
                ))
            }
        };
        // The rel32 follows the E8 and counts from its own end.
        let rel_at = b + call.call_at + 1;
        let rel = disp32_at(img, rel_at)
            .ok_or_else(|| format!("loader call at +0x{b:X} has no rel32 inside the image"))?;
        let rva = rip_target(rel_at + 4, rel)
            .filter(|&t| t < img.len())
            .ok_or_else(|| format!("loader call at +0x{b:X} targets +0x{rel:X} outside the image"))?;
        if img.get(rva..rva + LOADER_STOLEN) != Some(&LOADER_PROLOGUE[..]) {
            return Err(format!(
                "loader call at +0x{b:X} targets +0x{rva:X}, which does not open with \
                 LOADER_PROLOGUE"
            ));
        }
        Ok((b, call, rva))
    }
}

/// Every accessor site, across every encoding in [`ACCESSORS`], whose name
/// resolves to the C string `table`, in ascending address order.
///
/// With `image_base` of `None` the two indirect encodings are skipped entirely:
/// their name is only reachable through a VA, and a caller without a base
/// cannot turn one back into an RVA. Most sites name some other table and are
/// simply not collected, and so is one whose cell holds something that is not
/// a VA inside this image at all. Nothing here is an error, because the
/// caller's "exactly one match" is the real check.
fn accessors_naming(
    sites: &AccessorSites,
    img: &[u8],
    table: &[u8],
) -> Result<Vec<AccessorSite>, String> {
    sites.check(img)?;
    let mut named = Vec::new();
    for (t, hits) in ACCESSORS.iter().zip(sites.sites.iter()) {
        for s in hits {
            if accessor_name_rva(img, s.at, t, sites.image_base)
                .is_some_and(|rva| c_str_is(img, rva, table))
            {
                named.push(*s);
            }
        }
    }
    named.sort_unstable_by_key(|s| s.at);
    named.dedup_by_key(|s| s.at);
    Ok(named)
}

/// How many copies of each encoding are in `img`, as
/// `"45 33 C9 + lea r8=132, ..."`, for the resolvers' error messages, plus the
/// rejected-tail count when there are any. A count that has drifted from the
/// census in the module docs is the first thing to look at after a game update.
fn accessor_census(sites: &AccessorSites) -> String {
    let mut s = ACCESSORS
        .iter()
        .zip(sites.sites.iter())
        .filter(|(t, _)| !t.indirect || sites.image_base.is_some())
        .map(|(t, hits)| format!("{}={}", t.label, hits.len()))
        .collect::<Vec<_>>()
        .join(", ");
    if !sites.rejected.is_empty() {
        s.push_str(&format!(", unknown tail={}", sites.rejected.len()));
    }
    s
}

/// The one accessor site that names `table`.
///
/// Every table-specific resolver in this module starts here: the accessor is
/// what ties a table's name to its code and its data. `image_base` of `None`
/// restricts the search to the two `lea` encodings; see [`accessors_naming`].
fn named_accessor(
    sites: &AccessorSites,
    img: &[u8],
    table: &[u8],
) -> Result<AccessorSite, String> {
    let named = accessors_naming(sites, img, table)?;
    // Which half of the census was in scope, so an error says what was and was
    // not searched rather than only how much of it there was.
    let scope =
        if sites.image_base.is_some() { "any encoding" } else { "the lea encodings" };
    match named.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!(
            "no accessor of {scope} names \"{}\"; template copies scanned: {}",
            String::from_utf8_lossy(table),
            accessor_census(sites)
        )),
        many => Err(format!(
            "{} accessors of {scope} name \"{}\", expected exactly one: {:x?}",
            many.len(),
            String::from_utf8_lossy(table),
            many.iter().map(|s| s.at).collect::<Vec<_>>()
        )),
    }
}

/// Resolve the **RVA** of the global pointer slot holding the manager object
/// for the table named `table` (e.g. [`ITEM_TABLE`], [`GIMMICK_TABLE`]).
///
/// `img` is the image laid out by RVA (`MainModule::bytes()` in the game, or
/// [`crate::pe::file_to_image`] on the exe file). The result is an RVA, not a
/// virtual address: callers add the module base themselves. On build 26345091
/// this answers `0x71B6C20` for `iteminfo` and `0x71B6C48` for `gimmickinfo`
/// (`0x71B7BD0` and `0x71B7BF8` on 26356046 and 26361232).
///
/// See the module docs for the `mov rbx,[rip+disp32]; cmp r32,[rbx+0x8]`
/// shape this looks for in the [`SLOT_WINDOW`] bytes before the named
/// accessor. Scans the image on every call. A caller resolving more than one
/// table should hold an [`AccessorSites`] and call
/// [`AccessorSites::manager_slot`] instead.
pub fn resolve_manager_slot(img: &[u8], table: &[u8]) -> Result<usize, String> {
    AccessorSites::scan(img, None)?.manager_slot(img, table)
}

/// The **RVA** of the global pointer slot holding the manager object for the
/// table named `table`, resolved through **any** of the four accessor
/// encodings.
///
/// Same answer as [`resolve_manager_slot`] for every table a `lea` encoding
/// names, and the only way to reach the tables named through a pointer cell,
/// `FactionNode` and `Skill` among them. `image_base` is needed solely
/// because of that indirection: the cell's `u64` is the name string's **virtual
/// address**, so the base the image is laid out for is what turns it back into
/// an RVA. Pass the running module's base in the game (`MainModule::base`) and
/// `pe::Headers::image_base` for a file-backed image.
///
/// [`resolve_manager_slot`] is deliberately left alone rather than being made a
/// wrapper with an extra argument: the looter calls it on every startup and
/// has no image base to hand at that point. Scans the image on every call,
/// which is what [`AccessorSites`] exists to stop for a caller with several
/// tables.
pub fn resolve_manager_slot_based(
    img: &[u8],
    image_base: usize,
    table: &[u8],
) -> Result<usize, String> {
    AccessorSites::scan(img, Some(image_base))?.manager_slot(img, table)
}

/// The `mov rbx,[rip+disp32]; cmp r32,[rbx+0x8]` that precedes the accessor
/// head `a`, as `(offset of the mov, RVA of the slot it loads)`. Shared by
/// both resolvers and the census: the encodings differ in how they name the
/// table and in nothing else, so the slot search is identical once the site is
/// known.
///
/// One `mov rbx,[rip+...]` in the window is taken as it is. Several are
/// narrowed to the ones the bounds check follows; if that still leaves more
/// than one, the nearest to the head wins and the tie is logged once.
fn slot_load_before(img: &[u8], a: usize) -> Result<(usize, usize), String> {
    let pat_slot = Pattern::parse(PAT_SLOT).ok_or("the slot pattern is malformed")?;

    // Only the run-up to the accessor head is searched: the `mov` is a few
    // dozen bytes above it and the opcode is common exe-wide.
    let win_start = a.saturating_sub(SLOT_WINDOW);
    let window = img
        .get(win_start..a)
        .ok_or_else(|| format!("accessor at +0x{a:X} is past the image"))?;
    let hits: Vec<usize> = pat_slot.find_all(window, 64).iter().map(|h| win_start + h).collect();

    let checked_at = |h: usize| {
        h.checked_add(SLOT_RIP_AT)
            .and_then(|e| img.get(e..e.checked_add(3)?))
            .and_then(|b| <&[u8; 3]>::try_from(b).ok())
            .is_some_and(is_slot_cmp)
    };
    let hit = match hits.as_slice() {
        [one] => *one,
        [] => {
            return Err(format!(
                "no mov rbx,[rip+disp] within 0x{SLOT_WINDOW:X} bytes before the accessor at \
                 +0x{a:X}"
            ))
        }
        many => {
            // More than one candidate: the real slot load is the one the count
            // bounds check follows.
            let checked: Vec<usize> = many.iter().copied().filter(|&h| checked_at(h)).collect();
            match checked.as_slice() {
                [] => {
                    return Err(format!(
                        "{} mov rbx,[rip+disp] within 0x{SLOT_WINDOW:X} bytes before the accessor \
                         at +0x{a:X}, 0 of them followed by cmp r32,[rbx+8]: {:x?}",
                        many.len(),
                        many
                    ))
                }
                [one] => *one,
                [.., nearest] => {
                    let nearest = *nearest;
                    log_once(&LOGGED_SLOT_TIE, || {
                        format!(
                            "[gimmick] {} checked slot loads before the accessor at +0x{a:X} \
                             ({:x?}); took the nearest, +0x{nearest:X}",
                            checked.len(),
                            checked
                        )
                    });
                    nearest
                }
            }
        }
    };

    let disp = disp32_at(img, hit + SLOT_DISP_AT)
        .ok_or_else(|| format!("slot load at +0x{hit:X} has no disp32 inside the image"))?;
    rip_target(hit + SLOT_RIP_AT, disp)
        .filter(|&t| t < img.len())
        .map(|slot| (hit, slot))
        .ok_or_else(|| format!("slot load at +0x{hit:X} targets +0x{disp:X} outside the image"))
}

/// Resolve the `gimmickinfo` record loader in a mapped image.
///
/// `img` is the image laid out by RVA (`MainModule::bytes()` in the game, or
/// [`crate::pe::file_to_image`] on the exe file) and `image_base` is the base
/// it is (or would be) mapped at. Returns `image_base + rva` of the loader,
/// which on build 26345091 is `image_base + 0x3d89a0`.
///
/// This is what the gatherer hooks, and the table it wants is the only one
/// it ever wants, so the name stays out of its call. [`resolve_record_loader_for`]
/// is the same thing for any other table.
pub fn resolve_record_loader(img: &[u8], image_base: usize) -> Result<usize, String> {
    resolve_record_loader_for(img, image_base, GIMMICK_TABLE)
}

/// Resolve the record loader for the table named `table`, as `image_base + rva`.
///
/// Every static-info table is loaded by its own copy of one loader template,
/// reached through that table's accessor, so this is [`resolve_record_loader`]
/// with the name spelled out. Whichever it is, its first [`LOADER_STOLEN`]
/// bytes are [`LOADER_PROLOGUE`]: the resolver checks that before answering,
/// and a caller about to hook it must still check for itself before patching
/// (the image can change between the two).
///
/// See the module docs for why this goes through the accessor instead of
/// signature-scanning the loader directly.
pub fn resolve_record_loader_for(
    img: &[u8],
    image_base: usize,
    table: &[u8],
) -> Result<usize, String> {
    AccessorSites::scan(img, Some(image_base))?.record_loader(img, image_base, table)
}

impl AccessorSites {
    /// [`resolve_record_loader_for`] against sites already found.
    /// `image_base` here is the base the returned **address** is built against,
    /// and is deliberately separate from the one [`Self::scan`] was given.
    ///
    /// The two are the same number in practice and not the same job. The scan's
    /// base decides which encodings can be matched to a name at all; this one
    /// is only the addend. Keeping them apart is what lets a caller that knows
    /// its table is named by a `lea` encoding scan two encodings instead of
    /// four and still be handed an address, which is
    /// `desert_harvest::gatherer`'s whole startup, and it installs its hook against a
    /// table the game reads during loading.
    pub fn record_loader(
        &self,
        img: &[u8],
        image_base: usize,
        table: &[u8],
    ) -> Result<usize, String> {
        let site = named_accessor(self, img, table)?;
        let (_, _, rva) = self.loader_call(img, &site)?;
        image_base
            .checked_add(rva)
            .ok_or_else(|| format!("image base 0x{image_base:X} + rva 0x{rva:X} overflows"))
    }

    /// The stream layout the accessor naming `table` fills in before its
    /// loader call, read by [`parse_stream_fill`]. The gatherer asks for
    /// `gimmickinfo`'s at install time, before the hook goes in, so the hook
    /// reads the stream at the offsets this build actually uses.
    pub fn stream_layout(&self, img: &[u8], table: &[u8]) -> Result<StreamLayout, String> {
        let site = named_accessor(self, img, table)?;
        let (b, _, _) = self.loader_call(img, &site)?;
        stream_fill_at(img, &site, b).ok_or_else(|| {
            format!(
                "the stream fill between the accessor's call at +0x{:X} and the loader call at                  +0x{b:X} does not parse",
                site.call_at
            )
        })
    }
}

/// [`parse_stream_fill`] over the bytes between `site`'s file-load call (its
/// `E8` and rel32) and the loader call at `b`, with the disp8 of the loader
/// call's own `lea r9,[rsp+d8]` (`4C 8D 4C 24 d8`, the byte at `b + 4`).
fn stream_fill_at(img: &[u8], site: &AccessorSite, b: usize) -> Option<StreamLayout> {
    let r9_disp = *img.get(b.checked_add(4)?)?;
    let code = img.get(site.call_at.checked_add(5)?..b)?;
    parse_stream_fill(code, r9_disp)
}

/// [`AccessorSites::stream_layout`] for `gimmickinfo`, scanning first.
pub fn resolve_stream_layout(img: &[u8], image_base: usize) -> Result<StreamLayout, String> {
    AccessorSites::scan(img, Some(image_base))?.stream_layout(img, GIMMICK_TABLE)
}

// ---------------------------------------------------------------------------
// The parsed record layout, probed
// ---------------------------------------------------------------------------

/// Where the yields live in a *parsed* `gimmickinfo` record object, as the
/// gatherer's live re-apply path reads it: the key, the output list (a
/// pointer and a `u32` count), the list entry stride and its key field, and
/// the item id and the `u64` min/max inside each block object. All offsets.
///
/// [`ParsedLayout::OLD`] was read off the deserializer on build 25116796
/// (`docs/reference-internals.md` section 16) and held through 25477059. On
/// 26361232 the key is no longer at `+0x08` on any of the 279 remembered
/// records, so the layout is probed at runtime ([`probe_parsed_layout`]) from
/// what the load path already remembered, the way the stream layout and the
/// header shape are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedLayout {
    /// `u32` record key.
    pub key: usize,
    /// Where the list header is: `None` when it is inline in the record
    /// object (through 25477059), `Some(h)` when the object holds a pointer
    /// to it at `+h` (26361232). [`Self::list`] and [`Self::count`] are
    /// offsets into whichever it is.
    pub head: Option<usize>,
    /// Pointer to the list's entries, in the list header.
    pub list: usize,
    /// `u32` entry count, in the list header.
    pub count: usize,
    /// Bytes per list entry; the block pointer is at `entry+0`.
    pub entry: usize,
    /// The entry's own `u32` key field, zero on every gather record, when the
    /// layout has one (`entry+8` with 16-byte entries).
    pub entry_item: Option<usize>,
    /// `u32` item id in the block.
    pub block_item: usize,
    /// `u64` minimum in the block.
    pub block_min: usize,
    /// `u64` maximum in the block.
    pub block_max: usize,
}

impl ParsedLayout {
    /// Build 25116796 through 25477059.
    pub const OLD: Self = Self {
        key: 0x08,
        head: None,
        list: 0x278,
        count: 0x280,
        entry: 16,
        entry_item: Some(8),
        block_item: 0x68,
        block_min: 0x20,
        block_max: 0x28,
    };

    /// Build 26361232, read off three `Debug=1` launches (2026-11-01) with
    /// [`find_item_paths`] on the well (key 1001081) and `mine_rock_09`
    /// (1001482): the key at `+0x10` (a copy at `+0x44`), and at `+0x128` a
    /// pointer to the output list's header, `{u64 entries, u32 size, u32 cap}`
    /// (mine rock: `{0x39EBAD09FC0, 3, 3}`). The entries are 16 bytes as
    /// before, the block pointer at `+0` and the zero key field at `+8`, and
    /// the block is unchanged: item at `+0x68`, min/max at `+0x20`/`+0x28`
    /// (the well's read 22008 and 10/10 at Foraging=2, the mine rock's
    /// 1000689 and 1/1). The header is no longer inline: the record object
    /// shrank, and the `+0x70`/`+0x78`/`+0x80` pointers beside it lead to the
    /// neighbouring headers of the record's other lists.
    pub const NEW: Self = Self { key: 0x10, head: Some(0x128), list: 0x0, count: 0x8, ..Self::OLD };

    /// The layouts tried before the general probe, in order. The probe itself
    /// only searches inline headers (`head: None`).
    pub const KNOWN: [Self; 2] = [Self::OLD, Self::NEW];

    /// `key +0x8, list +0x278, count +0x280, ...` for an inline header, `key
    /// +0x10, list +0x128 -> +0x0, count +0x128 -> +0x8, ...` for one behind a
    /// pointer, the way the log names a layout.
    pub fn describe(&self) -> String {
        let via = self.head.map(|h| format!("+0x{h:x} -> ")).unwrap_or_default();
        format!(
            "key +0x{:x}, list {via}+0x{:x}, count {via}+0x{:x}, entry {}, block item +0x{:x}, min +0x{:x}, max +0x{:x}",
            self.key, self.list, self.count, self.entry, self.block_item, self.block_min, self.block_max
        )
    }
}

/// One remembered record to probe against: its key, the address of its
/// parsed object, and its vanilla output blocks `(item, min, max)` in list
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSample {
    pub key: u32,
    pub obj: usize,
    pub blocks: Vec<(u32, u64, u64)>,
}

/// How much of a parsed record object the probe reads.
pub const PARSED_WINDOW: usize = 0x400;
/// How much of a block object the probe reads.
pub const BLOCK_WINDOW: usize = 0x100;
/// The largest multiplier a record can have been loaded at. Current values
/// are `vanilla * k` for the multiplier in force when the record loaded, which
/// is not remembered, so min/max offsets are matched by "the same whole
/// factor `k` in `1..=MAX_LOAD_MULT` for both halves of a block".
pub const MAX_LOAD_MULT: u64 = 1000;
/// Entry strides tried, in order: the old one first.
const ENTRY_STRIDES: [usize; 3] = [16, 8, 24];

/// Read up to `max` bytes at `addr`, shrinking by halves when the full window
/// will not read (an object near the end of a mapping, or a short list).
/// `None` below 0x10.
fn read_window<R: Fn(usize, usize) -> Option<Vec<u8>>>(read: &R, addr: usize, max: usize) -> Option<Vec<u8>> {
    let mut n = max;
    while n >= 0x10 {
        if let Some(b) = read(addr, n) {
            return Some(b);
        }
        n /= 2;
    }
    None
}

fn usize_at(b: &[u8], o: usize) -> Option<usize> {
    u64_at(b, o).and_then(|v| usize::try_from(v).ok())
}

/// The factor `k` with `cur == vanilla * k`, `1 <= k <= MAX_LOAD_MULT`.
fn load_factor(cur: u64, vanilla: u64) -> Option<u64> {
    if vanilla == 0 || !cur.is_multiple_of(vanilla) {
        return None;
    }
    let k = cur / vanilla;
    (1..=MAX_LOAD_MULT).contains(&k).then_some(k)
}

/// The factor `k` with `cur_min == min * k` and `cur_max == max * k`,
/// `1 <= k <= MAX_LOAD_MULT`: what a block whose yields are vanilla times one
/// whole multiplier reads. `None` for anything else, which is a field that is
/// not this block's min/max, or a value something other than this plugin
/// wrote.
pub fn vanilla_factor(cur_min: u64, cur_max: u64, min: u64, max: u64) -> Option<u64> {
    let k = load_factor(cur_min, min)?;
    (load_factor(cur_max, max)? == k).then_some(k)
}

/// Do the block bytes `b` hold `(item, min, max)` at `l`'s offsets?
fn block_fits(b: &[u8], l: &ParsedLayout, (item, min, max): (u32, u64, u64)) -> bool {
    let (Some(i), Some(lo), Some(hi)) = (u32_at(b, l.block_item), u64_at(b, l.block_min), u64_at(b, l.block_max)) else {
        return false;
    };
    i == item && matches!((load_factor(lo, min), load_factor(hi, max)), (Some(a), Some(c)) if a == c)
}

/// One block as the probe holds it: its bytes, its list entry's bytes, and
/// the remembered `(item, min, max)`.
type ProbedBlock = (Vec<u8>, Vec<u8>, (u32, u64, u64));

/// Every block of every sample under `l`'s list offsets, as `(block bytes,
/// entry bytes, remembered block)`; `None` as soon as one does not read.
/// The list header's bytes for the record object at `obj` (window `w`): `w`
/// itself for an inline header, else `span` bytes at the pointer the object
/// holds at `head`. That pointer is read straight from `obj + head` when the
/// window came back too short to hold it.
fn list_header<'w, R: Fn(usize, usize) -> Option<Vec<u8>>>(
    w: &'w [u8],
    obj: usize,
    head: Option<usize>,
    span: usize,
    read: &R,
) -> Option<std::borrow::Cow<'w, [u8]>> {
    match head {
        None => Some(std::borrow::Cow::Borrowed(w)),
        Some(h) => {
            let at = usize_at(w, h)
                .or_else(|| usize_at(&read(obj.checked_add(h)?, 8)?, 0))
                .filter(|&p| crate::manager::plausible(p))?;
            Some(std::borrow::Cow::Owned(read(at, span)?))
        }
    }
}

fn blocks_under<R: Fn(usize, usize) -> Option<Vec<u8>>>(
    samples: &[(ParsedSample, Vec<u8>)],
    head: Option<usize>,
    list: usize,
    entry: usize,
    read: &R,
) -> Option<Vec<ProbedBlock>> {
    let mut out = Vec::new();
    for (s, w) in samples {
        let header = list_header(w, s.obj, head, list + 8, read)?;
        let data = usize_at(&header, list)?;
        if !crate::manager::plausible(data) {
            return None;
        }
        let n = s.blocks.len();
        let entries = read(data, n.checked_mul(entry)?)?;
        for (i, &rb) in s.blocks.iter().enumerate() {
            let e = entries.get(i * entry..(i + 1) * entry)?.to_vec();
            let block = usize_at(&e, 0)?;
            if !crate::manager::plausible(block) {
                return None;
            }
            out.push((read_window(read, block, BLOCK_WINDOW)?, e, rb));
        }
    }
    Some(out)
}

/// Does `l` describe every sample: the key, the count, a readable list of
/// readable blocks, each with the remembered item and a whole multiple of the
/// remembered min and max?
fn layout_fits<R: Fn(usize, usize) -> Option<Vec<u8>>>(
    samples: &[(ParsedSample, Vec<u8>)],
    l: &ParsedLayout,
    read: &R,
) -> bool {
    let span = (l.list + 8).max(l.count + 4);
    let heads_ok = samples.iter().all(|(s, w)| {
        u32_at(w, l.key) == Some(s.key)
            && list_header(w, s.obj, l.head, span, read)
                .and_then(|h| u32_at(&h, l.count))
                .map(|c| c as usize)
                == Some(s.blocks.len())
    });
    if !heads_ok {
        return false;
    }
    let Some(blocks) = blocks_under(samples, l.head, l.list, l.entry, read) else { return false };
    blocks.iter().all(|(b, e, rb)| {
        block_fits(b, l, *rb) && l.entry_item.is_none_or(|o| u32_at(e, o) == Some(0))
    })
}

/// The offsets `step`-aligned in `0..len` for which `pred(sample index,
/// offset)` holds on every sample.
fn common_offsets(len: usize, step: usize, n: usize, pred: impl Fn(usize, usize) -> bool) -> Vec<usize> {
    (0..len).step_by(step).filter(|&o| (0..n).all(|i| pred(i, o))).collect()
}

/// Derive the parsed record layout from remembered records.
///
/// `read(addr, len)` is a guarded read of `len` bytes (in the game,
/// `safe::read_into`); everything else here is pure. The old layout is
/// tried first and wins if it fits every sample, so a build where nothing
/// moved costs one check. Otherwise, each step narrows a candidate set by
/// what holds on **every** sample:
///
/// 1. the key: aligned `u32`s in the first [`PARSED_WINDOW`] bytes equal to
///    the record's key; every offset that remains is carried forward;
/// 2. the list: an aligned pointer and an aligned `u32` equal to the block
///    count, under each entry stride in turn (16, then 8, then 24), whose
///    entries' first `u64` is a readable block pointer;
/// 3. the block: the one `u32` offset holding the item id, and the one pair of
///    `u64` offsets holding the same whole multiple of the remembered min and
///    max, consistent across every block of every sample.
///
/// The known layouts ([`ParsedLayout::KNOWN`]) are tried before any of this.
/// Every full layout that fits every record is collected and handed to
/// [`choose_parsed`], which collapses redundant copies (two key offsets, a
/// vector's size and capacity, two item offsets) and refuses two different
/// structures.
/// The error names the step that failed and how many candidates it had.
pub fn probe_parsed_layout<R: Fn(usize, usize) -> Option<Vec<u8>>>(
    samples: &[ParsedSample],
    read: R,
) -> Result<ParsedLayout, String> {
    let usable: Vec<(ParsedSample, Vec<u8>)> = samples
        .iter()
        .filter(|s| !s.blocks.is_empty())
        .filter_map(|s| Some((s.clone(), read_window(&read, s.obj, PARSED_WINDOW)?)))
        .collect();
    if usable.is_empty() {
        return Err(format!("none of {} remembered records' objects would read", samples.len()));
    }
    for known in ParsedLayout::KNOWN {
        if layout_fits(&usable, &known, &read) {
            return Ok(known);
        }
    }
    let len = usable.iter().map(|(_, w)| w.len()).min().unwrap_or(0);
    let at = |i: usize| usable.get(i);

    // Every offset that holds the key on every record is carried forward: on
    // 26361232 the key sits at +0x10 and again at +0x44, and either copy
    // identifies the record equally well.
    let keys = common_offsets(len, 4, usable.len(), |i, o| {
        at(i).is_some_and(|(s, w)| u32_at(w, o) == Some(s.key))
    });
    if keys.is_empty() {
        return Err(format!("key: 0 candidate offsets across {} records", usable.len()));
    }
    let counts = common_offsets(len, 4, usable.len(), |i, o| {
        !keys.contains(&o) && at(i).is_some_and(|(s, w)| u32_at(w, o).map(|c| c as usize) == Some(s.blocks.len()))
    });
    let lists = common_offsets(len, 8, usable.len(), |i, o| {
        at(i).is_some_and(|(_, w)| usize_at(w, o).is_some_and(crate::manager::plausible))
    });
    if counts.is_empty() || lists.is_empty() {
        // The near miss: the offset holding the block count on the most
        // records, which says whether the count moved or is not stored as a
        // plain `u32` at all.
        let best = (0..len)
            .step_by(4)
            .filter(|o| !keys.contains(o))
            .map(|o| {
                let n = usable
                    .iter()
                    .filter(|(s, w)| u32_at(w, o).map(|c| c as usize) == Some(s.blocks.len()))
                    .count();
                (n, o)
            })
            .max_by_key(|&(n, o)| (n, usize::MAX - o));
        return Err(format!(
            "list: {} count candidates and {} pointer candidates across {} records ({} key candidates{})",
            counts.len(),
            lists.len(),
            usable.len(),
            keys.len(),
            best.map(|(n, o)| format!("; best count offset +0x{o:x} on {n} of {}", usable.len())).unwrap_or_default()
        ));
    }

    let mut block_why: Option<String> = None;
    for entry in ENTRY_STRIDES {
        let mut found: Vec<ParsedLayout> = Vec::new();
        for &list in &lists {
            // The pointer's eight bytes and the count's four must not overlap.
            let pairs: Vec<usize> = counts.iter().copied().filter(|&c| c + 4 <= list || c >= list + 8).collect();
            if pairs.is_empty() {
                continue;
            }
            let Some(blocks) = blocks_under(&usable, None, list, entry, &read) else { continue };
            match block_offsets(&blocks) {
                Ok((items, minmax)) => {
                    let entry_item = (entry >= 16)
                        .then_some(8)
                        .filter(|&o| blocks.iter().all(|(_, e, _)| u32_at(e, o) == Some(0)));
                    for &key in &keys {
                        for &count in &pairs {
                            for &block_item in &items {
                                for &(block_min, block_max) in &minmax {
                                    let l = ParsedLayout {
                                        key,
                                        head: None,
                                        list,
                                        count,
                                        entry,
                                        entry_item,
                                        block_item,
                                        block_min,
                                        block_max,
                                    };
                                    if layout_fits(&usable, &l, &read) {
                                        found.push(l);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    block_why.get_or_insert(format!("entry {entry}, list +0x{list:x}: {e}"));
                }
            }
        }
        if found.is_empty() {
            continue;
        }
        return choose_parsed(&found).map_err(|e| format!("{e} with entry {entry} ({} key candidates)", keys.len()));
    }
    Err(format!(
        "list/block: no pointer/count pair among {} pointers and {} counts led to blocks holding the \
         remembered item and yields, under entry strides 16, 8 or 24 ({} key candidates at {:x?}{})",
        lists.len(),
        counts.len(),
        keys.len(),
        keys,
        block_why.map(|w| format!("; first block failure: {w}")).unwrap_or_default()
    ))
}

/// Where a `u32` equal to `item` sits in memory reachable from `start` by
/// following aligned plausible pointers, as offset paths: `[a, b, c]` means
/// `*(*(start + a) + b) + c` holds the item. Breadth first, at most
/// `max_depth` pointers deep, at most `max_nodes` windows read
/// ([`PARSED_WINDOW`] bytes at `start`, [`BLOCK_WINDOW`] beyond), at most
/// `max_hits` paths, each address visited once. `read` is a guarded read.
///
/// The `Debug=1` live path runs this from a remembered record's object to its
/// first block's item id when the parsed-layout probe fails: the path it
/// prints is the record -> list -> entry -> block chain, offsets included.
pub fn find_item_paths<R: Fn(usize, usize) -> Option<Vec<u8>>>(
    start: usize,
    item: u32,
    read: R,
    max_depth: usize,
    max_nodes: usize,
    max_hits: usize,
) -> Vec<Vec<usize>> {
    let mut hits = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = std::collections::VecDeque::from([(start, Vec::new())]);
    let mut nodes = 0usize;
    while let Some((addr, path)) = queue.pop_front() {
        if nodes >= max_nodes || hits.len() >= max_hits {
            break;
        }
        if !seen.insert(addr) {
            continue;
        }
        nodes += 1;
        let window = if path.is_empty() { PARSED_WINDOW } else { BLOCK_WINDOW };
        let Some(b) = read_window(&read, addr, window) else { continue };
        for o in (0..b.len()).step_by(4) {
            if u32_at(&b, o) == Some(item) && hits.len() < max_hits {
                let mut p: Vec<usize> = path.clone();
                p.push(o);
                hits.push(p);
            }
        }
        if path.len() < max_depth {
            for o in (0..b.len()).step_by(8) {
                if let Some(p) = usize_at(&b, o).filter(|&p| crate::manager::plausible(p)) {
                    let mut next = path.clone();
                    next.push(o);
                    queue.push_back((p, next));
                }
            }
        }
    }
    hits
}

/// One layout out of every one that fit every record.
///
/// Layouts that agree on the structure (the list, the entry stride and key
/// field, and the block's min/max) and differ only in which copy of a
/// redundant field they read are the same answer: the lowest key offset, the
/// count right after its pointer when there is one (a `{ptr, u32 size, u32
/// cap}` vector reads the same number twice while it is full) else the lowest,
/// and the lowest item offset are taken. Two different structures are an
/// ambiguity, and refused.
pub fn choose_parsed(found: &[ParsedLayout]) -> Result<ParsedLayout, String> {
    let shape = |l: &ParsedLayout| (l.head, l.list, l.entry, l.entry_item, l.block_min, l.block_max);
    let mut shapes: Vec<_> = found.iter().map(shape).collect();
    shapes.sort_unstable();
    shapes.dedup();
    let [only] = shapes.as_slice() else {
        return Err(format!(
            "list: {} different layouts fit every record (list, entry, min, max: {:x?})",
            shapes.len(),
            shapes.iter().map(|s| (s.1, s.2, s.4, s.5)).collect::<Vec<_>>()
        ));
    };
    let same: Vec<&ParsedLayout> = found.iter().filter(|l| shape(l) == *only).collect();
    let pick = |f: fn(&ParsedLayout) -> usize| same.iter().map(|l| f(l)).min();
    let (Some(key), Some(block_item)) = (pick(|l| l.key), pick(|l| l.block_item)) else {
        return Err("list: nothing fit".to_string());
    };
    let count = same
        .iter()
        .map(|l| l.count)
        .find(|&c| c == only.1 + 8)
        .or_else(|| pick(|l| l.count))
        .unwrap_or(only.1 + 8);
    Ok(ParsedLayout {
        key,
        head: only.0,
        list: only.1,
        count,
        entry: only.2,
        entry_item: only.3,
        block_item,
        block_min: only.4,
        block_max: only.5,
    })
}

// ---------------------------------------------------------------------------
// Reading a parsed record under a layout, and picking the layout and the name
// offset from the records the manager holds (the looter's side of the
// 26361232 move: it has no remembered samples, only keys and names)
// ---------------------------------------------------------------------------

/// One output block of a parsed record, as [`parsed_outputs`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedOutput {
    /// Item id, from the block (`block_item`).
    pub item: u32,
    /// `u64` minimum, already multiplied if the load-time hook patched it.
    pub min: u64,
    /// `u64` maximum, likewise.
    pub max: u64,
}

/// Every output block of the parsed record object at `obj` under `l`, in list
/// order: the list header inline at `obj` (`head: None`) or behind the
/// pointer at `obj + head`, then `count` entries of `entry` bytes from the
/// list pointer, each holding its block pointer at `+0`.
///
/// The item id is the block's own (`block_item`, the low dword of the `u64`
/// at `+0x68` on both known builds). The entry's key field (`entry_item`) is
/// zero on every gather record on both builds and is cross-checked only when
/// it is not: a nonzero value that disagrees with the block refuses the
/// record. Any other surprise (an empty or oversized list, a pointer that is
/// not [`plausible`](crate::manager::plausible), a zero item, `min == 0`,
/// `min > max`, `max > MAX_QTY`) returns `None` for the whole record rather
/// than a half-right list. `read(addr, len)` is a guarded read, all or
/// nothing.
pub fn parsed_outputs<R: Fn(usize, usize) -> Option<Vec<u8>>>(
    obj: usize,
    l: &ParsedLayout,
    read: &R,
) -> Option<Vec<ParsedOutput>> {
    let ptr_at = |addr: usize| -> Option<usize> {
        usize_at(&read(addr, 8)?, 0).filter(|&p| crate::manager::plausible(p))
    };
    if l.entry < 8 {
        return None;
    }
    let base = match l.head {
        None => obj,
        Some(h) => ptr_at(obj.checked_add(h)?)?,
    };
    let data = ptr_at(base.checked_add(l.list)?)?;
    let count = u32_at(&read(base.checked_add(l.count)?, 4)?, 0)?;
    if count == 0 || count > MAX_COUNT {
        return None;
    }
    let n = count as usize;
    let entries = read(data, n.checked_mul(l.entry)?)?;
    let span = (l.block_item + 4).max(l.block_min + 8).max(l.block_max + 8);
    let mut out = Vec::with_capacity(n);
    for e in entries.chunks_exact(l.entry) {
        let block = usize_at(e, 0).filter(|&p| crate::manager::plausible(p))?;
        let b = read(block, span)?;
        let item = u32_at(&b, l.block_item).filter(|&i| i != 0)?;
        if let Some(o) = l.entry_item {
            let echoed = u32_at(e, o)?;
            if echoed != 0 && echoed != item {
                return None;
            }
        }
        let (min, max) = (u64_at(&b, l.block_min)?, u64_at(&b, l.block_max)?);
        if min == 0 || min > max || max > MAX_QTY {
            return None;
        }
        out.push(ParsedOutput { item, min, max });
    }
    Some(out)
}

/// A tally over parsed record objects of which [`ParsedLayout::KNOWN`]
/// layout reads them: a record votes for a layout when the `u32` at its
/// `key` is a known gather key (the caller's `is_known_key`, in the plugin
/// `collect::family_by_key`) **and** its output list reads cleanly
/// ([`parsed_outputs`]). Under the wrong layout the key offset holds a
/// pointer's low half or a neighbouring field, which is not one of 279 gather
/// keys, so a wrong layout collects no votes at all in practice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LayoutVote {
    hits: [usize; ParsedLayout::KNOWN.len()],
}

impl LayoutVote {
    /// Count `obj` toward every known layout that reads it.
    pub fn observe<R: Fn(usize, usize) -> Option<Vec<u8>>>(
        &mut self,
        obj: usize,
        is_known_key: impl Fn(u32) -> bool,
        read: &R,
    ) {
        for (l, h) in ParsedLayout::KNOWN.iter().zip(self.hits.iter_mut()) {
            let key = obj.checked_add(l.key).and_then(|a| read(a, 4)).and_then(|b| u32_at(&b, 0));
            if key.is_some_and(&is_known_key) && parsed_outputs(obj, l, read).is_some() {
                *h += 1;
            }
        }
    }

    /// Votes per layout, in [`ParsedLayout::KNOWN`] order.
    pub fn hits(&self) -> &[usize] {
        &self.hits
    }

    /// The most votes any one layout has.
    pub fn best(&self) -> usize {
        self.hits.iter().copied().max().unwrap_or(0)
    }

    /// The layout with the most votes, when it has at least `min` and no
    /// other layout has as many. A tie is refused rather than guessed.
    pub fn verdict(&self, min: usize) -> Option<ParsedLayout> {
        let best = self.best();
        if best < min.max(1) || self.hits.iter().filter(|&&h| h == best).count() != 1 {
            return None;
        }
        let i = self.hits.iter().position(|&h| h == best)?;
        ParsedLayout::KNOWN.get(i).copied()
    }
}

/// Where a parsed record object keeps its name: a C string reached from the
/// pointer at `obj + offset` through `derefs` pointers (`2` is
/// `**(obj+offset)`, the shape through 25477059 at `+0x10`; `1` is
/// `*(obj+offset)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameAt {
    pub offset: usize,
    pub derefs: u8,
}

impl NameAt {
    /// `**(rec+0x10)`, build 25116796 through 25477059.
    pub const OLD: Self = Self { offset: 0x10, derefs: 2 };

    /// `**(rec+0x10)` / `*(rec+0x10)`, the way the log names it.
    pub fn describe(&self) -> String {
        format!("{}(rec+0x{:x})", "*".repeat(self.derefs as usize), self.offset)
    }
}

/// Name offsets the probe tries: `0..NAME_PROBE_END`, 8-aligned.
pub const NAME_PROBE_END: usize = 0x60;
const NAME_SLOTS: usize = NAME_PROBE_END / 8;
/// The longest record name read, terminator included.
const NAME_MAX: usize = 64;
/// Indirections tried per offset, in order of preference.
const NAME_DEREFS: [u8; 2] = [2, 1];

/// The name at `at` in the parsed record object `obj`: a NUL-terminated run of
/// at least three printable ASCII bytes inside the first [`NAME_MAX`]. `None`
/// for anything else.
pub fn read_name_at<R: Fn(usize, usize) -> Option<Vec<u8>>>(obj: usize, at: NameAt, read: &R) -> Option<String> {
    let mut p = obj.checked_add(at.offset)?;
    for _ in 0..at.derefs {
        p = usize_at(&read(p, 8)?, 0).filter(|&q| crate::manager::plausible(q))?;
    }
    let b = read_window(read, p, NAME_MAX)?;
    let end = b.iter().position(|&c| c == 0)?;
    let s = b.get(..end)?;
    if s.len() < 3 || !s.iter().all(|c| (0x20..0x7F).contains(c)) {
        return None;
    }
    String::from_utf8(s.to_vec()).ok()
}

/// A tally over parsed record objects of which [`NameAt`] holds their known
/// name: for each object whose name is known (in the plugin, the
/// `collect` row for the key the latched layout reads), every candidate in
/// `0..NAME_PROBE_END` under both indirections that reads that exact name,
/// case-insensitively, gets a vote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameVote {
    hits: [[usize; NAME_DEREFS.len()]; NAME_SLOTS],
}

impl NameVote {
    /// Count `obj`, whose name should be `expected`.
    pub fn observe<R: Fn(usize, usize) -> Option<Vec<u8>>>(&mut self, obj: usize, expected: &str, read: &R) {
        for (slot, row) in self.hits.iter_mut().enumerate() {
            for (&derefs, h) in NAME_DEREFS.iter().zip(row.iter_mut()) {
                let at = NameAt { offset: slot * 8, derefs };
                if read_name_at(obj, at, read).is_some_and(|s| s.eq_ignore_ascii_case(expected)) {
                    *h += 1;
                }
            }
        }
    }

    /// The most votes any one candidate has.
    pub fn best(&self) -> usize {
        self.hits.iter().flatten().copied().max().unwrap_or(0)
    }

    /// The first candidate, by offset and then by indirection (`**` before
    /// `*`), with at least `min` votes.
    pub fn verdict(&self, min: usize) -> Option<NameAt> {
        self.hits.iter().enumerate().find_map(|(slot, row)| {
            NAME_DEREFS
                .iter()
                .zip(row.iter())
                .find(|(_, h)| **h >= min.max(1))
                .map(|(&derefs, _)| NameAt { offset: slot * 8, derefs })
        })
    }
}

// ---------------------------------------------------------------------------
// The parsed *item* record layout (`iteminfo`): where the key and the name
// sit, picked from the records the manager holds
// ---------------------------------------------------------------------------
//
// Through 25477059 an item record was `u32 key @0` and its name
// `**(rec+0x08)`. On 26361232 the looter read heap-pointer halves as keys
// there and no name at all, which is the shape of the gimmick record's move
// (key `+0x08` -> `+0x10`, name `**(rec+0x10)` -> `**(rec+0x08)`). Nothing in
// the item table says which offset holds what, so the probe does not assume
// the analogy: the key is the offset where several item ids the gimmick
// records pay out are found, unique across the table, and the name is the
// place most records read a plausible identifier from.

/// Where a parsed item record keeps its key and its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemLayout {
    /// Offset of the `u32` item key.
    pub key: usize,
    /// Where the name is.
    pub name: NameAt,
}

impl ItemLayout {
    /// `u32 key @0`, name `**(rec+0x08)`: build 25116796 through 25477059.
    pub const OLD: Self = Self { key: 0, name: NameAt { offset: 0x08, derefs: 2 } };

    /// `key +0x8, name **(rec+0x0)`, the way the log names it.
    pub fn describe(&self) -> String {
        format!("key +0x{:x}, name {}", self.key, self.name.describe())
    }

    /// The key's four bytes do not overlap the name's pointer cell: a key
    /// offset that reads half of the name pointer is not a key.
    pub fn consistent(&self) -> bool {
        let (k, n) = (self.key, self.name.offset);
        k.saturating_add(4) <= n || n.saturating_add(8) <= k
    }
}

/// Key offsets the item probe tries: `0..ITEM_KEY_PROBE_END`, 4-aligned.
pub const ITEM_KEY_PROBE_END: usize = 0x20;
const ITEM_KEY_SLOTS: usize = ITEM_KEY_PROBE_END / 4;
/// Item key 1, which the bag names `Money_Copper`.
pub const MONEY_ITEM_KEY: u32 = 1;
/// The name item key [`MONEY_ITEM_KEY`] has, compared case-insensitively.
pub const MONEY_ITEM_NAME: &str = "Money_Copper";

/// How one key offset fared over the walked item records.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemKeyScore {
    /// Distinct known item ids found as a key at this offset.
    pub known: usize,
    /// No two records read the same value here.
    pub unique: bool,
}

/// The `u32` at every candidate key offset of every item record walked, kept
/// in walk order so the record behind a key can be found again.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemKeyVote {
    objs: Vec<usize>,
    keys: [Vec<u32>; ITEM_KEY_SLOTS],
}

impl ItemKeyVote {
    /// Read `obj`'s first [`ITEM_KEY_PROBE_END`] bytes. An unreadable record
    /// is skipped and not counted.
    pub fn observe<R: Fn(usize, usize) -> Option<Vec<u8>>>(&mut self, obj: usize, read: &R) {
        let Some(b) = read(obj, ITEM_KEY_PROBE_END) else { return };
        let vals: Option<Vec<u32>> = (0..ITEM_KEY_SLOTS).map(|i| u32_at(&b, i * 4)).collect();
        let Some(vals) = vals else { return };
        self.objs.push(obj);
        for (v, k) in self.keys.iter_mut().zip(vals) {
            v.push(k);
        }
    }

    /// Records read.
    pub fn records(&self) -> usize {
        self.objs.len()
    }

    /// Per candidate offset, in offset order.
    pub fn scores(&self, known: &std::collections::HashSet<u32>) -> [ItemKeyScore; ITEM_KEY_SLOTS] {
        let mut out = [ItemKeyScore::default(); ITEM_KEY_SLOTS];
        for (s, v) in out.iter_mut().zip(&self.keys) {
            let mut sorted = v.clone();
            sorted.sort_unstable();
            let before = sorted.len();
            sorted.dedup();
            s.unique = sorted.len() == before;
            s.known = sorted.iter().filter(|k| known.contains(k)).count();
        }
        out
    }

    /// The key offset: unique over at least `min_records` records and holding
    /// at least `min_known` of the `known` ids, the most of any such offset.
    /// A tie is refused rather than guessed.
    pub fn verdict(&self, known: &std::collections::HashSet<u32>, min_records: usize, min_known: usize) -> Option<usize> {
        if self.records() < min_records.max(1) {
            return None;
        }
        let scores = self.scores(known);
        let ok = |s: &ItemKeyScore| s.unique && s.known >= min_known.max(1);
        let best = scores.iter().filter(|s| ok(s)).map(|s| s.known).max()?;
        let mut at = scores.iter().enumerate().filter(|(_, s)| ok(s) && s.known == best);
        let (i, _) = at.next()?;
        at.next().is_none().then_some(i * 4)
    }

    /// The record whose `u32` at `key_off` is `key`, if one was walked.
    pub fn record_with_key(&self, key_off: usize, key: u32) -> Option<usize> {
        if !key_off.is_multiple_of(4) {
            return None;
        }
        let v = self.keys.get(key_off / 4)?;
        let i = v.iter().position(|&k| k == key)?;
        self.objs.get(i).copied()
    }
}

/// A plausible record identifier: at least three bytes, every one ASCII
/// alphanumeric or `_` (`Money_Copper`, `item_basic_onehand`).
pub fn is_identifier(s: &str) -> bool {
    s.len() >= 3 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// A tally over item records of which [`NameAt`] reads a plausible
/// identifier ([`is_identifier`]), plus which candidates read
/// [`MONEY_ITEM_NAME`] off the money record when that one is walked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemNameVote {
    sampled: usize,
    hits: [[usize; NAME_DEREFS.len()]; NAME_SLOTS],
    money: [[bool; NAME_DEREFS.len()]; NAME_SLOTS],
}

impl ItemNameVote {
    /// Count `obj` toward every candidate that reads an identifier from it.
    pub fn observe<R: Fn(usize, usize) -> Option<Vec<u8>>>(&mut self, obj: usize, read: &R) {
        self.sampled += 1;
        for (slot, row) in self.hits.iter_mut().enumerate() {
            for (&derefs, h) in NAME_DEREFS.iter().zip(row.iter_mut()) {
                let at = NameAt { offset: slot * 8, derefs };
                if read_name_at(obj, at, read).is_some_and(|s| is_identifier(&s)) {
                    *h += 1;
                }
            }
        }
    }

    /// Mark every candidate that reads [`MONEY_ITEM_NAME`] off `obj`, the
    /// record whose key is [`MONEY_ITEM_KEY`].
    pub fn observe_money<R: Fn(usize, usize) -> Option<Vec<u8>>>(&mut self, obj: usize, read: &R) {
        for (slot, row) in self.money.iter_mut().enumerate() {
            for (&derefs, m) in NAME_DEREFS.iter().zip(row.iter_mut()) {
                let at = NameAt { offset: slot * 8, derefs };
                if read_name_at(obj, at, read).is_some_and(|s| s.eq_ignore_ascii_case(MONEY_ITEM_NAME)) {
                    *m = true;
                }
            }
        }
    }

    /// Records sampled.
    pub fn sampled(&self) -> usize {
        self.sampled
    }

    /// The best candidate and its votes, for the log.
    pub fn best(&self) -> Option<(NameAt, usize)> {
        self.candidates().max_by_key(|&(at, h)| (h, std::cmp::Reverse(at.offset)))
    }

    fn candidates(&self) -> impl Iterator<Item = (NameAt, usize)> + '_ {
        self.hits.iter().enumerate().flat_map(|(slot, row)| {
            NAME_DEREFS.iter().zip(row.iter()).map(move |(&derefs, &h)| (NameAt { offset: slot * 8, derefs }, h))
        })
    }

    fn is_money(&self, at: NameAt) -> bool {
        self.money
            .get(at.offset / 8)
            .and_then(|row| NAME_DEREFS.iter().position(|&d| d == at.derefs).and_then(|i| row.get(i)))
            .copied()
            .unwrap_or(false)
    }

    /// Every candidate that read an identifier from more than half the
    /// sampled records and from at least `min`, most votes first, then by
    /// offset and indirection (`**` before `*`).
    pub fn ranked(&self, min: usize) -> Vec<(NameAt, usize)> {
        let mut ok: Vec<(NameAt, usize)> =
            self.candidates().filter(|&(_, h)| h >= min.max(1) && h * 2 > self.sampled).collect();
        // `candidates` yields offset order, `**` first; a stable sort on the
        // votes keeps that order among equals.
        ok.sort_by_key(|&(_, h)| std::cmp::Reverse(h));
        ok
    }

    /// The name's place: a candidate that read an identifier from more than
    /// half the sampled records and from at least `min`. Among several, the
    /// one that read [`MONEY_ITEM_NAME`] off the money record wins, else the
    /// one with the most votes; a tie that the money record does not settle
    /// is refused.
    pub fn verdict(&self, min: usize) -> Option<NameAt> {
        let ok: Vec<(NameAt, usize)> =
            self.candidates().filter(|&(_, h)| h >= min.max(1) && h * 2 > self.sampled).collect();
        let money: Vec<&(NameAt, usize)> = ok.iter().filter(|(at, _)| self.is_money(*at)).collect();
        if let [only] = money.as_slice() {
            return Some(only.0);
        }
        let pool: Vec<(NameAt, usize)> =
            if money.is_empty() { ok } else { money.into_iter().copied().collect() };
        let best = pool.iter().map(|&(_, h)| h).max()?;
        let mut top = pool.iter().filter(|&&(_, h)| h == best);
        let first = top.next()?;
        top.next().is_none().then_some(first.0)
    }
}

/// Records whose names are sampled for [`ItemNameVote`] by [`probe_item_layout`].
pub const ITEM_NAME_SAMPLE: usize = 256;
/// Item records a probe must read before it decides anything.
pub const ITEM_MIN_RECORDS: usize = 32;
/// Known item ids that must be found as keys at the chosen offset.
pub const ITEM_MIN_KNOWN: usize = 3;
/// Records that must read an identifier at the chosen name place.
pub const ITEM_MIN_NAMED: usize = 8;

/// Pick the item record layout from the loaded item record objects `objs`
/// and the item ids `known` to be real (in the plugin, the gimmick records'
/// output items): the key offset by [`ItemKeyVote::verdict`], the name place
/// by [`ItemNameVote::verdict`] over the first [`ITEM_NAME_SAMPLE`] records,
/// cross-checked against the money record's name when the key finds it, and
/// the two must not overlap ([`ItemLayout::consistent`]). `Err` carries the
/// evidence for the log line that says nothing was chosen.
pub fn probe_item_layout<R: Fn(usize, usize) -> Option<Vec<u8>>>(
    objs: &[usize],
    known: &std::collections::HashSet<u32>,
    read: &R,
) -> Result<ItemLayout, String> {
    let mut keys = ItemKeyVote::default();
    let mut names = ItemNameVote::default();
    for (i, &obj) in objs.iter().enumerate() {
        keys.observe(obj, read);
        if i < ITEM_NAME_SAMPLE {
            names.observe(obj, read);
        }
    }
    let key = keys.verdict(known, ITEM_MIN_RECORDS, ITEM_MIN_KNOWN);
    if let Some(money) = key.and_then(|k| keys.record_with_key(k, MONEY_ITEM_KEY)) {
        names.observe_money(money, read);
    }
    let name = names.verdict(ITEM_MIN_NAMED);
    let evidence = || {
        let scores: Vec<String> = keys
            .scores(known)
            .iter()
            .enumerate()
            .map(|(i, s)| format!("+0x{:x}:{}{}", i * 4, s.known, if s.unique { "" } else { "(dup)" }))
            .collect();
        let best = names.best().map(|(at, h)| format!("{} {h}", at.describe())).unwrap_or_else(|| "-".into());
        format!(
            "{} records read, {} known ids; known ids found per key offset [{}]; best name {best} of {} sampled",
            keys.records(),
            known.len(),
            scores.join(" "),
            names.sampled()
        )
    };
    match (key, name) {
        (Some(key), Some(name)) => {
            let l = ItemLayout { key, name };
            if l.consistent() {
                Ok(l)
            } else {
                Err(format!("key +0x{key:x} overlaps the name pointer {}: {}", name.describe(), evidence()))
            }
        }
        (None, _) => Err(format!("no key offset: {}", evidence())),
        (_, None) => Err(format!("no name place: {}", evidence())),
    }
}

/// Where a parsed record keeps its name, for a table with no known name to
/// check against (`characterinfo`): the candidate in `0..NAME_PROBE_END`
/// that reads an identifier ([`is_identifier`]) from the most of the first
/// [`ITEM_NAME_SAMPLE`] records of `objs`, from more than half of them and
/// from at least `min`. A tie goes to the lowest offset (`**` before `*`),
/// the order [`NameVote::verdict`] uses, and the tied places come back beside
/// it so the caller can say so. `Err` carries the evidence for the log.
pub fn probe_record_name<R: Fn(usize, usize) -> Option<Vec<u8>>>(
    objs: &[usize],
    min: usize,
    read: &R,
) -> Result<(NameAt, Vec<NameAt>), String> {
    let mut names = ItemNameVote::default();
    for &obj in objs.iter().take(ITEM_NAME_SAMPLE) {
        names.observe(obj, read);
    }
    let ranked = names.ranked(min);
    let Some(&(first, votes)) = ranked.first() else {
        let best = names.best().map(|(at, h)| format!("{} {h}", at.describe())).unwrap_or_else(|| "-".into());
        return Err(format!(
            "no place reads an identifier from more than half of {} sampled records (best {best}, need {})",
            names.sampled(),
            min.max(1)
        ));
    };
    let tied = ranked.iter().skip(1).filter(|&&(_, h)| h == votes).map(|&(at, _)| at).collect();
    Ok((first, tied))
}

/// Item offset candidates, and `(min, max)` offset pair candidates.
type BlockCandidates = (Vec<usize>, Vec<(usize, usize)>);

/// Every item offset, and every `(min, max)` offset pair, that every block
/// agrees on; an error naming the half with no candidate at all.
fn block_offsets(blocks: &[ProbedBlock]) -> Result<BlockCandidates, String> {
    let len = blocks.iter().map(|(b, _, _)| b.len()).min().unwrap_or(0);
    let n = blocks.len();
    let b = |i: usize| blocks.get(i);
    let items = common_offsets(len, 4, n, |i, o| b(i).is_some_and(|(x, _, (it, _, _))| u32_at(x, o) == Some(*it)));
    let mins = common_offsets(len, 8, n, |i, o| {
        b(i).is_some_and(|(x, _, (_, mn, _))| u64_at(x, o).and_then(|v| load_factor(v, *mn)).is_some())
    });
    let maxs = common_offsets(len, 8, n, |i, o| {
        b(i).is_some_and(|(x, _, (_, _, mx))| u64_at(x, o).and_then(|v| load_factor(v, *mx)).is_some())
    });
    if items.is_empty() {
        return Err("block item: 0 candidates".to_string());
    }
    let mut pairs = Vec::new();
    for &lo in &mins {
        for &hi in &maxs {
            if lo == hi {
                continue;
            }
            let same = (0..n).all(|i| {
                b(i).is_some_and(|(x, _, (_, mn, mx))| {
                    let a = u64_at(x, lo).and_then(|v| load_factor(v, *mn));
                    let c = u64_at(x, hi).and_then(|v| load_factor(v, *mx));
                    a.is_some() && a == c
                })
            });
            if same {
                pairs.push((lo, hi));
            }
        }
    }
    if pairs.is_empty() {
        return Err(format!("block min/max: 0 candidate pairs ({} item candidates)", items.len()));
    }
    Ok((items, pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block whose fields all check out.
    ///
    /// Until 2026-09-12 this wrote the item id at `+5` and `+64`, the two zero
    /// pads, exactly as the constants then said, so every test below agreed
    /// with the code about a layout neither of them shared with the game. That
    /// is why `reads_the_item_id_at_the_offset_the_table_uses` spells the
    /// offsets out as literals rather than reusing the constants.
    fn block(item: u32, min: u64, max: u64) -> Vec<u8> {
        let mut b = vec![0u8; BLOCK];
        b[0] = 1;
        b[ITEM_AT..ITEM_AT + 4].copy_from_slice(&item.to_le_bytes());
        b[MIN_AT..MIN_AT + 8].copy_from_slice(&min.to_le_bytes());
        b[MAX_AT..MAX_AT + 8].copy_from_slice(&max.to_le_bytes());
        b[58] = 0xFF;
        b[59] = 0xFF;
        b[ITEM_TAIL_AT..ITEM_TAIL_AT + 4].copy_from_slice(&item.to_le_bytes());
        b
    }

    fn header(key: u32, name: &str) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&key.to_le_bytes());
        r.extend_from_slice(&(name.len() as u32).to_le_bytes());
        r.extend_from_slice(name.as_bytes());
        r.push(0);
        r
    }

    /// Header, a 2-block list, some filler, a 1-block list.
    fn record() -> (Vec<u8>, usize, usize) {
        let mut r = header(17030001, "mine_bluestone");
        r.extend_from_slice(&[0xAB; 16]); // fields we do not parse
        let l1 = r.len();
        r.extend_from_slice(&2u32.to_le_bytes());
        r.extend_from_slice(&block(101, 1, 3));
        r.extend_from_slice(&block(102, 2, 8));
        r.extend_from_slice(&[0x00; 24]);
        let l2 = r.len();
        r.extend_from_slice(&1u32.to_le_bytes());
        r.extend_from_slice(&block(103, 5, 5));
        r.extend_from_slice(&[0x7F; 8]);
        (r, l1, l2)
    }

    #[test]
    fn parses_a_header() {
        let (r, _, _) = record();
        let h = parse_header(&r).unwrap();
        assert_eq!(h.key, 17030001);
        assert_eq!(h.name, "mine_bluestone");
        assert_eq!(h.body, 8 + 14 + 1);
        assert_eq!(r[h.body - 1], 0);
    }

    /// A `NameFirst` header as 26361232 writes it.
    fn nf_header(name: &str, key: u32, path: &str, flag: u8) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&(name.len() as u32).to_le_bytes());
        r.extend_from_slice(name.as_bytes());
        r.extend_from_slice(&key.to_le_bytes());
        r.push(0);
        r.extend_from_slice(&(path.len() as u32).to_le_bytes());
        r.extend_from_slice(path.as_bytes());
        r.push(flag);
        r
    }

    #[test]
    fn parses_the_26361232_headers() {
        // Record 0 of the 26361232 table body, byte for byte, plus the first
        // field after the header.
        let mut r0 = vec![0x1b, 0, 0, 0];
        r0.extend_from_slice(b"operator_gimmickcomplete_00");
        r0.extend_from_slice(&[0x2b, 0x4e, 0x0f, 0x00, 0x00, 0x3d, 0, 0, 0]);
        r0.extend_from_slice(b"/object/cd_gimmick/gimmick_operator_gimmickcomplete_00.prefab");
        r0.extend_from_slice(&[0x00, 0x02, 0x00, 0x00, 0x00]);
        let h = parse_header(&r0).unwrap();
        assert_eq!((h.key, h.name.as_str(), h.shape), (1003051, "operator_gimmickcomplete_00", HeaderShape::NameFirst));
        assert_eq!(h.path.as_deref(), Some("/object/cd_gimmick/gimmick_operator_gimmickcomplete_00.prefab"));
        assert_eq!(h.body, r0.len() - 4, "the body starts past the flag byte");
        assert_eq!(detect_header_shape(&r0), Some(HeaderShape::NameFirst));
        assert_eq!(parse_header_as(&r0, HeaderShape::KeyFirst), None);

        // The well (key unchanged since 25116796), flag 1, and the one record
        // whose path does not start `/object`.
        let well = nf_header(
            "gimmick_well_0001_parts01",
            1001081,
            "/object/cd_gimmick/00_common/well/gimmick_well_0001_parts01.prefab",
            1,
        );
        assert_eq!(parse_header(&well).map(|h| (h.key, h.body)), Some((1001081, well.len())));
        let flowers = nf_header(
            "gimmick_item_flowers_01",
            1012413,
            "/character/prefab/6_object/tools/cd_gimmick_item_flowers_01.prefab",
            0,
        );
        assert_eq!(parse_header(&flowers).map(|h| h.key), Some(1012413));
    }

    #[test]
    fn name_first_edge_cases() {
        let good = nf_header("abcd", 1001081, "/object/x.prefab", 0);
        assert!(parse_header_as(&good, HeaderShape::NameFirst).is_some());
        // Truncated anywhere: nothing.
        for n in 0..good.len() {
            assert_eq!(parse_header_as(&good[..n], HeaderShape::NameFirst), None, "cut at {n}");
        }
        // The byte after the key must be zero, the flag 0 or 1, the path
        // rooted and printable.
        let k = 4 + 4;
        let mut bad = good.clone();
        bad[k + 4] = 1;
        assert_eq!(parse_header_as(&bad, HeaderShape::NameFirst), None);
        let mut bad = good.clone();
        *bad.last_mut().unwrap() = 2;
        assert_eq!(parse_header_as(&bad, HeaderShape::NameFirst), None);
        assert_eq!(parse_header_as(&nf_header("abcd", 1, "object/x", 0), HeaderShape::NameFirst), None);
        assert_eq!(parse_header_as(&nf_header("abcd", 1, "/obj\x01", 0), HeaderShape::NameFirst), None);
        assert_eq!(parse_header_as(&nf_header("", 1, "/x", 0), HeaderShape::NameFirst), None);
        assert_eq!(parse_header_as(&nf_header("abcd", 1, "", 0), HeaderShape::NameFirst), None);
    }

    #[test]
    fn the_two_shapes_never_both_validate_on_real_headers() {
        let (kf, _, _) = record();
        assert_eq!(detect_header_shape(&kf), Some(HeaderShape::KeyFirst));
        assert_eq!(parse_header(&kf).unwrap().shape, HeaderShape::KeyFirst);
        // A KeyFirst key below 256 reads as a NameFirst name length, and the
        // full NameFirst validation is what rejects it: its "name" would be
        // the KeyFirst length field, which is not printable.
        let small = header(5, "abcde");
        assert_eq!(detect_header_shape(&small), Some(HeaderShape::KeyFirst));
        assert!(parse_header_as(&small, HeaderShape::NameFirst).is_none());
        // Neither shape: nothing.
        assert_eq!(detect_header_shape(&[0xFF; 32]), None);
        assert_eq!(parse_header(&[0xFF; 32]), None);
    }

    #[test]
    fn header_edge_cases() {
        assert!(parse_header(&[]).is_none());
        assert!(parse_header(&[0u8; 7]).is_none(), "too short for the two u32s");
        // zero-length name
        let mut z = header(1, "");
        z.push(0);
        assert!(parse_header(&z).is_none());
        // name longer than 255
        let long = header(1, &"a".repeat(256));
        assert!(parse_header(&long).is_none());
        assert!(parse_header(&header(1, &"a".repeat(255))).is_some());
        // non-printable byte in the name
        let mut np = header(1, "abcd");
        np[9] = 0x01;
        assert!(parse_header(&np).is_none());
        let mut np = header(1, "abcd");
        np[9] = 0x80;
        assert!(parse_header(&np).is_none());
        // NUL terminator missing (truncated right at the end of the name)
        let full = header(1, "abcd");
        assert!(parse_header(&full[..full.len() - 1]).is_none());
        // NUL not a NUL
        let mut bad = header(1, "abcd");
        let n = bad.len() - 1;
        bad[n] = b'x';
        assert!(parse_header(&bad).is_none());
    }

    #[test]
    fn finds_both_lists() {
        let (r, l1, l2) = record();
        assert_eq!(output_lists(&r), vec![(l1, 2), (l2, 1)]);
    }

    #[test]
    fn reads_every_block_of_every_list() {
        let (r, l1, l2) = record();
        assert_eq!(
            output_blocks(&r),
            vec![
                OutputBlock { offset: l1 + 4, item: 101, min: 1, max: 3 },
                OutputBlock { offset: l1 + 4 + BLOCK, item: 102, min: 2, max: 8 },
                OutputBlock { offset: l2 + 4, item: 103, min: 5, max: 5 },
            ]
        );
        assert!(output_blocks(&[]).is_empty());
        assert!(output_blocks(&[0xAB; 200]).is_empty());
    }

    /// Where the item id actually lives, pinned as literals.
    ///
    /// Every other test here builds its blocks with `block`, so before
    /// 2026-09-12 they all agreed with `ITEM_AT`/`ITEM_TAIL_AT` about a layout
    /// four bytes off from the game's and none of them could have caught it.
    /// This one writes the bytes by hand at the offsets measured over all 896
    /// output blocks of `gimmickinfo_pabgb_clean.bin` and never names the
    /// constants, so it fails if either moves again. `peony_01`'s first block is
    /// the real-world case, asserted against the fixture itself in
    /// `crates/desert-core/tests/gimmick_real.rs`.
    #[test]
    fn reads_the_item_id_at_the_offset_the_table_uses() {
        let mut raw = vec![0u8; 4 + BLOCK];
        raw[0] = 1; // the u32 count
        let b = 4;
        raw[b] = 1; // +0  flag
        raw[b + 1..b + 5].copy_from_slice(&757_006u32.to_le_bytes()); // +1  item
        // +5 .. +9 stays zero: the padding the constants used to point at.
        raw[b + 42..b + 50].copy_from_slice(&4u64.to_le_bytes()); // +42 min
        raw[b + 50..b + 58].copy_from_slice(&7u64.to_le_bytes()); // +50 max
        raw[b + 58] = 0xFF;
        raw[b + 59] = 0xFF;
        raw[b + 60..b + 64].copy_from_slice(&757_006u32.to_le_bytes()); // +60 item
        // +64 .. +68 stays zero: the other padding.

        assert_eq!(
            output_blocks(&raw),
            vec![OutputBlock { offset: b, item: 757_006, min: 4, max: 7 }],
            "peony_01's first block, byte for byte"
        );
        // And the two pads really are zero here, so the block that proves the
        // item offsets is also the block that proves what is at +5 and +64.
        assert_eq!(u32_at(&raw, b + 5), Some(0));
        assert_eq!(u32_at(&raw, b + 64), Some(0));

        // The same bytes with the item id written where the constants used to
        // say it was are not a block at all: the two "copies" are then zero and
        // the two pads disagree.
        let mut wrong = raw.clone();
        wrong[b + 1..b + 5].copy_from_slice(&0u32.to_le_bytes());
        wrong[b + 5..b + 9].copy_from_slice(&757_006u32.to_le_bytes());
        wrong[b + 60..b + 64].copy_from_slice(&0u32.to_le_bytes());
        wrong[b + 64..b + 68].copy_from_slice(&757_006u32.to_le_bytes());
        assert_eq!(
            output_blocks(&wrong),
            vec![OutputBlock { offset: b, item: 0, min: 4, max: 7 }],
            "the old layout still parses, with item 0 - which is the bug"
        );
    }

    /// The blocks and the edits are two views of the same thing, which is what
    /// lets the live path re-derive an edit from a remembered block.
    #[test]
    fn blocks_line_up_with_the_edits() {
        let (r, _, _) = record();
        let blocks = output_blocks(&r);
        let edits = multiply(&r, 7);
        assert_eq!(edits.len(), blocks.len() * 2);
        for (n, b) in blocks.iter().enumerate() {
            let (Some(lo), Some(hi)) = (edits.get(n * 2), edits.get(n * 2 + 1)) else {
                panic!("edit pair {n} missing")
            };
            assert_eq!((lo.offset, lo.old), (b.offset + MIN_AT, b.min));
            assert_eq!((hi.offset, hi.old), (b.offset + MAX_AT, b.max));
            assert_eq!((lo.new, hi.new), (b.min * 7, b.max * 7));
            // Both copies of the item id agree, so either identifies the block.
            assert_eq!(u32_at(&r, b.offset + ITEM_AT), Some(b.item));
        }
    }

    #[test]
    fn multiply_and_apply_round_trip() {
        let (mut r, l1, l2) = record();
        let edits = multiply(&r, 3);
        assert_eq!(edits.len(), 6, "3 blocks x (min, max)");
        assert_eq!(
            edits[0],
            Edit { offset: l1 + 4 + MIN_AT, old: 1, new: 3 }
        );
        assert_eq!(
            edits[5],
            Edit { offset: l2 + 4 + MAX_AT, old: 5, new: 15 }
        );
        assert_eq!(apply(&mut r, &edits), 6);
        // The lists still parse and now read back multiplied.
        assert_eq!(output_lists(&r), vec![(l1, 2), (l2, 1)]);
        let after: Vec<(u64, u64)> = output_lists(&r)
            .iter()
            .flat_map(|&(at, c)| {
                (0..c as usize).map(move |n| {
                    let b = at + 4 + n * BLOCK;
                    (b + MIN_AT, b + MAX_AT)
                })
            })
            .map(|(a, b)| (u64_at(&r, a).unwrap(), u64_at(&r, b).unwrap()))
            .collect();
        assert_eq!(after, vec![(3, 9), (6, 24), (15, 15)]);
    }

    #[test]
    fn multiply_is_a_no_op_for_0_and_1() {
        let (r, _, _) = record();
        assert!(multiply(&r, 0).is_empty());
        assert!(multiply(&r, 1).is_empty());
    }

    #[test]
    fn multiply_saturates() {
        let mut r = 1u32.to_le_bytes().to_vec();
        r.extend_from_slice(&block(7, MAX_QTY, MAX_QTY));
        let edits = multiply(&r, u32::MAX);
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].new, MAX_QTY.saturating_mul(u32::MAX as u64));
    }

    #[test]
    fn apply_skips_out_of_range_edits() {
        let mut r = vec![0u8; 16];
        let edits = [
            Edit { offset: 0, old: 0, new: 7 },
            Edit { offset: 12, old: 0, new: 9 }, // only 4 bytes left
            Edit { offset: usize::MAX, old: 0, new: 9 },
        ];
        assert_eq!(apply(&mut r, &edits), 1);
        assert_eq!(u64_at(&r, 0), Some(7));
        assert_eq!(r[12..], [0, 0, 0, 0]);
    }

    /// Each way a block can fail the signature must make the list vanish.
    #[test]
    fn signature_rejects() {
        let good = |blocks: &[Vec<u8>], count: u32| {
            let mut r = count.to_le_bytes().to_vec();
            for b in blocks {
                r.extend_from_slice(b);
            }
            r
        };
        assert_eq!(output_lists(&good(&[block(9, 1, 8)], 1)), vec![(0, 1)]);

        let mut b = block(9, 1, 8);
        b[0] = 0; // flag != 1
        assert!(output_lists(&good(&[b], 1)).is_empty());

        let mut b = block(9, 1, 8);
        b[ITEM_TAIL_AT] = 0x0A; // the two copies of the item id disagree
        assert!(output_lists(&good(&[b], 1)).is_empty());

        let mut b = block(9, 1, 8);
        b[ITEM_AT] = 0x0A; // ... either way round
        assert!(output_lists(&good(&[b], 1)).is_empty());

        // The two pads have to agree too. Only this clause excludes the 16
        // otherwise-identical lists in the clean body whose tail pad is nonzero,
        // so a block that fails it must not be accepted.
        let mut b = block(9, 1, 8);
        b[PAD_TAIL_AT] = 0x0A;
        assert!(output_lists(&good(&[b], 1)).is_empty(), "tail pad set, head pad zero");
        let mut b = block(9, 1, 8);
        b[PAD_AT] = 0x0A;
        assert!(output_lists(&good(&[b], 1)).is_empty(), "head pad set, tail pad zero");
        // Agreeing non-zero pads are not what this clause is about and are
        // accepted, which is the honest statement of what it checks.
        let mut b = block(9, 1, 8);
        b[PAD_AT] = 0x0A;
        b[PAD_TAIL_AT] = 0x0A;
        assert_eq!(output_lists(&good(&[b], 1)), vec![(0, 1)]);

        let mut b = block(9, 1, 8);
        b[58] = 0xFE; // missing FF FF
        assert!(output_lists(&good(&[b], 1)).is_empty());
        let mut b = block(9, 1, 8);
        b[59] = 0x00;
        assert!(output_lists(&good(&[b], 1)).is_empty());

        assert!(output_lists(&good(&[block(9, 9, 8)], 1)).is_empty(), "min > max");
        assert!(output_lists(&good(&[block(9, 0, 8)], 1)).is_empty(), "min == 0");
        assert!(
            output_lists(&good(&[block(9, 1, MAX_QTY + 1)], 1)).is_empty(),
            "max over the cap"
        );

        assert!(output_lists(&good(&[block(9, 1, 8)], 0)).is_empty(), "count == 0");
        // A constant item id with no 0x01 byte in it, so that no `u32` inside a
        // block can pass for a count: a 65-block list must find nothing at all,
        // not merely fail to match at offset 0.
        let many: Vec<Vec<u8>> = (0..65).map(|_| block(0xBBBB, 8, 8)).collect();
        assert!(output_lists(&good(&many, 65)).is_empty(), "count > 64");
        assert_eq!(output_lists(&good(&many[..64], 64)), vec![(0, 64)], "count == 64 is fine");

        // A truncated list is not a list.
        let full = good(&[block(9, 1, 8), block(9, 1, 8)], 2);
        assert!(output_lists(&full[..full.len() - 1]).is_empty());
    }

    /// A lagged Fibonacci-ish LCG: no dev-dependency, deterministic.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            self.0 >> 11
        }
        fn fill(&mut self, buf: &mut [u8]) {
            for b in buf.iter_mut() {
                *b = self.next() as u8;
            }
        }
    }

    #[test]
    fn no_panic_on_garbage() {
        let mut rng = Rng(0x5EED_1234);
        let (rec, _, _) = record();
        for round in 0..64u32 {
            let mut buf = vec![0u8; (rng.next() % 600) as usize];
            rng.fill(&mut buf);
            // Bias a few rounds toward near-valid bytes so the scanner actually
            // walks into blocks instead of bailing on the count every time.
            if round % 4 == 0 {
                for chunk in buf.chunks_mut(9) {
                    if let Some(c) = chunk.first_mut() {
                        *c = 1;
                    }
                }
            }
            let _ = parse_header(&buf);
            let _ = output_lists(&buf);
            let _ = output_blocks(&buf);
            let e = multiply(&buf, round.wrapping_add(2));
            let mut copy = buf.clone();
            let _ = apply(&mut copy, &e);
            let _ = apply(&mut copy, &[Edit { offset: usize::MAX - 3, old: 0, new: 1 }]);
        }
        // Every truncation of a real record, both directions.
        for n in 0..rec.len() {
            let _ = parse_header(&rec[..n]);
            let _ = output_lists(&rec[..n]);
            let _ = output_blocks(&rec[..n]);
            let _ = multiply(&rec[..n], 2);
            let _ = parse_header(&rec[n..]);
            let _ = output_lists(&rec[n..]);
            let _ = output_blocks(&rec[n..]);
            let _ = multiply(&rec[n..], 2);
        }
        // And every single-byte corruption of one, at the head.
        for n in 0..64.min(rec.len()) {
            let mut r = rec.clone();
            r[n] ^= 0xFF;
            let _ = parse_header(&r);
            let _ = multiply(&r, 2);
        }
    }

    const SYN_BASE: usize = 0x1_4000_0000;
    const SYN_LOADER: usize = 0x2000;
    /// Distance from the slot load to the pattern-A hit, as in 94 of the 98
    /// real copies.
    const SYN_SLOT_BACK: usize = 0x3A;
    const SYN_GIMMICK_SLOT: usize = 0x3800;
    const SYN_ITEM_SLOT: usize = 0x3820;
    /// The bounds check after a slot load, as build 26345091 encodes it
    /// (`cmp edx,[rbx+0x8]`).
    const SYN_CMP: [u8; 3] = [0x3B, 0x53, 0x08];

    /// `mov rbx,[rip+disp32]` at `at` targeting `slot_rva`, optionally followed
    /// by the `cmp edi,[rbx+0x8]` bounds check.
    fn put_slot_load(img: &mut [u8], at: usize, slot_rva: usize, with_cmp: bool) {
        let mut b = vec![0x48, 0x8B, 0x1D];
        let disp = (slot_rva as i64 - (at + SLOT_RIP_AT) as i64) as i32;
        b.extend_from_slice(&disp.to_le_bytes());
        if with_cmp {
            b.extend_from_slice(&SYN_CMP);
        }
        img[at..at + b.len()].copy_from_slice(&b);
    }

    /// The six bytes an accessor site opens with, for one of the four
    /// encodings: `xor r9d,r9d` in either ModRM direction, then the `lea` or
    /// the `mov` that brings the table name into `r8`.
    fn accessor_head(xor31: bool, indirect: bool) -> [u8; 6] {
        let xor = if xor31 { 0x31 } else { 0x33 };
        let op = if indirect { 0x8B } else { 0x8D };
        [0x45, xor, 0xC9, 0x4C, op, 0x05]
    }

    /// A whole accessor site at `a`: the encoding's head, a disp32 reaching
    /// `rip_rva` (the name for a `lea` encoding, the pointer cell for a `mov`
    /// one) and the rest of the template through the file-load `call`.
    fn put_accessor(img: &mut [u8], a: usize, head: [u8; 6], rip_rva: usize) {
        let mut pa = head.to_vec();
        let disp = (rip_rva as i64 - (a + A_RIP_AT) as i64) as i32;
        pa.extend_from_slice(&disp.to_le_bytes());
        pa.extend_from_slice(&[0x48, 0x8B, 0x53, 0x10, 0x48, 0x8D, 0x4C, 0x24, 0x70, 0xE8]);
        pa.extend_from_slice(&0x1122_3344u32.to_le_bytes()); // the file-load call
        img[a..a + pa.len()].copy_from_slice(&pa);
    }

    /// The call to the record loader at `at`, in encoding `enc` of
    /// `mov rcx,rbx` (0 = `48 8B CB`, 1 = `48 89 D9`), targeting `callee`.
    fn put_call_b(img: &mut [u8], at: usize, enc: usize, callee: usize) {
        let mut pb = vec![
            0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x44, 0x0F, 0xB7, 0xC7, 0x48, 0x8D, 0x54, 0x24, 0x70,
        ];
        let mov = if enc == 0 { [0x48, 0x8B, 0xCB] } else { [0x48, 0x89, 0xD9] };
        pb.extend_from_slice(&mov);
        pb.push(0xE8);
        let rel = (callee as i64 - (at + pb.len() + 4) as i64) as i32;
        pb.extend_from_slice(&rel.to_le_bytes());
        img[at..at + pb.len()].copy_from_slice(&pb);
    }

    /// An image with two copies of the accessor template (one naming
    /// "gimmickinfo", one naming "iteminfo"), a pattern-B call in each, and the
    /// manager-slot load 0x3A before each accessor site. Both are the
    /// `45 33 C9` + `lea` encoding, as the real `gimmickinfo` accessor is.
    fn synthetic_image() -> Vec<u8> {
        let mut img = vec![0u8; 0x4000];
        let put = |img: &mut Vec<u8>, at: usize, b: &[u8]| {
            img[at..at + b.len()].copy_from_slice(b);
        };
        put(&mut img, 0x3000, b"gimmickinfo\0");
        put(&mut img, 0x3100, b"iteminfo\0");

        // site: an accessor at `a` naming `name_rva`, the slot load for
        // `slot_rva` at a-0x3A, the loader call at a+0x40.
        let site = |img: &mut Vec<u8>, a: usize, name_rva: usize, slot_rva: usize, callee: usize| {
            put_slot_load(img, a - SYN_SLOT_BACK, slot_rva, true);
            put_accessor(img, a, accessor_head(false, false), name_rva);
            put_call_b(img, a + 0x40, 0, callee);
        };
        site(&mut img, 0x1000, 0x3000, SYN_GIMMICK_SLOT, SYN_LOADER);
        site(&mut img, 0x1800, 0x3100, SYN_ITEM_SLOT, 0x2800);
        // Something that looks like the loader prologue at the target.
        put(&mut img, SYN_LOADER, &LOADER_PROLOGUE);
        img
    }

    #[test]
    fn resolves_the_loader_through_the_named_accessor() {
        let img = synthetic_image();
        assert_eq!(resolve_record_loader(&img, SYN_BASE), Ok(SYN_BASE + SYN_LOADER));
        // ...and it is really the rel32 that decided it, not the window.
        assert_eq!(
            img.get(SYN_LOADER..SYN_LOADER + LOADER_STOLEN),
            Some(&LOADER_PROLOGUE[..])
        );
    }

    #[test]
    fn resolve_reports_what_failed() {
        // No accessor names the table at all.
        let mut img = synthetic_image();
        img[0x3000..0x3000 + 12].copy_from_slice(b"gimmickinfX\0");
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no accessor"), "{e}");

        // Two accessors name it.
        let mut img = synthetic_image();
        img[0x3100..0x3100 + 12].copy_from_slice(b"gimmickinfo\0");
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("2 accessors"), "{e}");

        // The accessor is there but the call is not.
        let mut img = synthetic_image();
        img[0x1040] = 0x00;
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no loader call"), "{e}");

        // The call targets outside the image.
        let mut img = synthetic_image();
        let rel_at = 0x1040 + 18;
        img[rel_at..rel_at + 4].copy_from_slice(&0x7000_0000i32.to_le_bytes());
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("outside the image"), "{e}");

        // Empty and tiny images are errors, not panics.
        assert!(resolve_record_loader(&[], SYN_BASE).is_err());
        assert!(resolve_record_loader(&[0x45, 0x33, 0xC9], SYN_BASE).is_err());
    }

    /// Where the slot load sits in `synthetic_image` for each accessor.
    const SYN_GIMMICK_LOAD: usize = 0x1000 - SYN_SLOT_BACK;
    const SYN_ITEM_LOAD: usize = 0x1800 - SYN_SLOT_BACK;

    #[test]
    fn resolves_the_manager_slot_for_each_table() {
        let img = synthetic_image();
        assert_eq!(resolve_manager_slot(&img, GIMMICK_TABLE), Ok(SYN_GIMMICK_SLOT));
        assert_eq!(resolve_manager_slot(&img, ITEM_TABLE), Ok(SYN_ITEM_SLOT));

        // One table's load going missing does not affect the other.
        let mut img = synthetic_image();
        img[SYN_ITEM_LOAD] = 0x00;
        assert_eq!(resolve_manager_slot(&img, GIMMICK_TABLE), Ok(SYN_GIMMICK_SLOT));
        assert!(resolve_manager_slot(&img, ITEM_TABLE).is_err());
    }

    #[test]
    fn manager_slot_picks_the_load_the_count_check_follows() {
        // A second `mov rbx,[rip+...]` in the window, pointing somewhere else
        // and *not* followed by `cmp edi,[rbx+8]`: the checked one still wins.
        let mut img = synthetic_image();
        put_slot_load(&mut img, SYN_GIMMICK_LOAD - 0x16, 0x3840, false);
        assert_eq!(resolve_manager_slot(&img, GIMMICK_TABLE), Ok(SYN_GIMMICK_SLOT));
    }

    // -----------------------------------------------------------------------
    // The indirect accessor template
    // -----------------------------------------------------------------------

    const SYN_FACTION_SLOT: usize = 0x3860;
    const SYN_DECOY_SLOT: usize = 0x3880;
    /// Where the two pointer cells live, and where the indirect sites are.
    const SYN_FACTION_CELL: usize = 0x3900;
    const SYN_DECOY_CELL: usize = 0x3908;
    const SYN_FACTION_A: usize = 0x2000;
    const SYN_DECOY_A: usize = 0x2400;

    /// `synthetic_image` plus two copies of the **indirect** template: one
    /// naming "FactionNode" through a pointer cell, and a decoy naming
    /// "iteminfo" through another. The decoy is the point of the fixture:
    /// on the real exe 12 of the 13 cells belong to other tables and must
    /// simply not match.
    fn synthetic_image_ptr() -> Vec<u8> {
        let mut img = synthetic_image();
        img.resize(0x4000, 0);
        let put = |img: &mut Vec<u8>, at: usize, b: &[u8]| {
            img[at..at + b.len()].copy_from_slice(b);
        };
        put(&mut img, 0x3200, b"FactionNode\0");
        // The cells hold VAs, not RVAs: base + rva, exactly as the loader
        // leaves them after relocation.
        put(&mut img, SYN_FACTION_CELL, &((SYN_BASE + 0x3200) as u64).to_le_bytes());
        put(&mut img, SYN_DECOY_CELL, &((SYN_BASE + 0x3100) as u64).to_le_bytes());

        let site = |img: &mut Vec<u8>, a: usize, cell_rva: usize, slot_rva: usize| {
            put_slot_load(img, a - SYN_SLOT_BACK, slot_rva, true);
            put_accessor(img, a, accessor_head(false, true), cell_rva);
        };
        site(&mut img, SYN_FACTION_A, SYN_FACTION_CELL, SYN_FACTION_SLOT);
        site(&mut img, SYN_DECOY_A, SYN_DECOY_CELL, SYN_DECOY_SLOT);
        img
    }

    #[test]
    fn resolves_a_slot_through_the_indirect_template() {
        let img = synthetic_image_ptr();
        assert_eq!(
            resolve_manager_slot_based(&img, SYN_BASE, FACTION_NODE_TABLE),
            Ok(SYN_FACTION_SLOT)
        );
        // The decoy cell names "iteminfo", and so does a *direct* accessor
        // in the same image, so the two must not both count.
        let e = resolve_manager_slot_based(&img, SYN_BASE, ITEM_TABLE).unwrap_err();
        assert!(e.contains("2 accessors of any encoding"), "{e}");
    }

    #[test]
    fn the_direct_path_is_unaffected_by_the_new_resolver() {
        // Both fixtures, both functions: the direct template answers exactly
        // what it always did, through either entry point.
        for img in [synthetic_image(), synthetic_image_ptr()] {
            assert_eq!(resolve_manager_slot(&img, GIMMICK_TABLE), Ok(SYN_GIMMICK_SLOT));
            assert_eq!(
                resolve_manager_slot_based(&img, SYN_BASE, GIMMICK_TABLE),
                Ok(SYN_GIMMICK_SLOT)
            );
            // The old function cannot see the indirect template at all, which
            // is why the new one exists.
            assert!(resolve_manager_slot(&img, FACTION_NODE_TABLE).is_err());
        }
    }

    #[test]
    fn indirect_candidates_that_are_not_the_table_are_skipped_not_errors() {
        // A cell whose value is below the image base is not a VA in this
        // image; a cell whose VA lands past the end of it is not either.
        // Neither may turn into an error for a table that resolves fine.
        let mut img = synthetic_image_ptr();
        img[SYN_DECOY_CELL..SYN_DECOY_CELL + 8].copy_from_slice(&7u64.to_le_bytes());
        assert_eq!(
            resolve_manager_slot_based(&img, SYN_BASE, FACTION_NODE_TABLE),
            Ok(SYN_FACTION_SLOT)
        );

        let mut img = synthetic_image_ptr();
        img[SYN_DECOY_CELL..SYN_DECOY_CELL + 8]
            .copy_from_slice(&((SYN_BASE + 0x9000) as u64).to_le_bytes());
        assert_eq!(
            resolve_manager_slot_based(&img, SYN_BASE, FACTION_NODE_TABLE),
            Ok(SYN_FACTION_SLOT)
        );

        // And a wrong base makes the *name* stop resolving, rather than
        // reading some other byte range as a string.
        let img = synthetic_image_ptr();
        let e = resolve_manager_slot_based(&img, SYN_BASE + 0x10, FACTION_NODE_TABLE).unwrap_err();
        assert!(e.contains("no accessor of any encoding"), "{e}");
    }

    #[test]
    fn based_resolver_reports_every_encodings_count() {
        let img = synthetic_image_ptr();
        let e = resolve_manager_slot_based(&img, SYN_BASE, b"NoSuchTable").unwrap_err();
        // Two `45 33 C9` + lea sites and two `45 33 C9` + mov ones are in this
        // fixture; the other two encodings are absent and say so.
        assert!(e.contains("45 33 C9 + lea r8=2"), "{e}");
        assert!(e.contains("45 33 C9 + mov r8=2"), "{e}");
        assert!(e.contains("45 31 C9 + lea r8=0"), "{e}");
        assert!(e.contains("45 31 C9 + mov r8=0"), "{e}");
        // The base-less resolver reports only what it can actually search.
        let e = resolve_manager_slot(&img, b"NoSuchTable").unwrap_err();
        assert!(e.contains("45 33 C9 + lea r8=2"), "{e}");
        assert!(!e.contains("mov r8"), "{e}");

        // Empty and tiny images are errors, not panics.
        assert!(resolve_manager_slot_based(&[], SYN_BASE, FACTION_NODE_TABLE).is_err());
        assert!(resolve_manager_slot_based(&[0x45, 0x33, 0xC9], 0, ITEM_TABLE).is_err());
        assert!(resolve_manager_slot_based(&[0x4C, 0x8B, 0x05], usize::MAX, ITEM_TABLE).is_err());
    }

    #[test]
    fn manager_slot_reports_what_failed() {
        // No accessor names the table at all.
        let mut img = synthetic_image();
        img[0x3000..0x3000 + 12].copy_from_slice(b"gimmickinfX\0");
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("no accessor"), "{e}");

        // Two accessors name it.
        let mut img = synthetic_image();
        img[0x3100..0x3100 + 12].copy_from_slice(b"gimmickinfo\0");
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("2 accessors"), "{e}");

        // The accessor is there but nothing loads rbx before it.
        let mut img = synthetic_image();
        img[SYN_GIMMICK_LOAD] = 0x00;
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("no mov rbx"), "{e}");
        assert!(e.contains(&format!("0x{SLOT_WINDOW:X}")), "{e}");

        // Two loads in the window and the count check follows neither.
        let mut img = synthetic_image();
        let cmp_at = SYN_GIMMICK_LOAD + SLOT_RIP_AT;
        img[cmp_at..cmp_at + SYN_CMP.len()].fill(0x90);
        put_slot_load(&mut img, SYN_GIMMICK_LOAD - 0x16, 0x3840, false);
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains('2'), "{e}");
        assert!(e.contains("0 of them"), "{e}");

        // The slot is outside the image.
        let mut img = synthetic_image();
        let disp_at = SYN_GIMMICK_LOAD + SLOT_DISP_AT;
        img[disp_at..disp_at + 4].copy_from_slice(&0x7000_0000i32.to_le_bytes());
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("outside the image"), "{e}");

        // Empty and tiny images are errors, not panics.
        assert!(resolve_manager_slot(&[], GIMMICK_TABLE).is_err());
        assert!(resolve_manager_slot(&[0x45, 0x33, 0xC9], GIMMICK_TABLE).is_err());
        assert!(resolve_manager_slot(&[0x48, 0x8B, 0x1D], ITEM_TABLE).is_err());
    }

    // -----------------------------------------------------------------------
    // All four accessor encodings, both loader-call encodings
    // -----------------------------------------------------------------------

    /// `(table, manager slot, record loader)` for the four sites in
    /// [`four_encoding_image`], one per accessor encoding in the order of
    /// `ACCESSORS`: 33+lea, 33+mov, 31+lea, 31+mov.
    const FOUR: [(&[u8], usize, usize); 4] = [
        (GIMMICK_TABLE, 0x3800, 0x2800),
        (ITEM_TABLE, 0x3820, 0x2840),
        (DROPSET_TABLE, 0x3840, 0x2880),
        (FACTION_NODE_TABLE, 0x3860, 0x28C0),
    ];
    /// Where each of those four accessor sites is.
    const FOUR_A: [usize; 4] = [0x1000, 0x1200, 0x1400, 0x1600];
    /// A fifth site, of the `45 31 C9` + `mov` encoding, naming a table nobody
    /// asks about: on the real exe 148 of the 149 sites are this to any given
    /// question, and every one of them must be skipped rather than error.
    const FOUR_DECOY_A: usize = 0x1800;
    const FOUR_DECOY_SLOT: usize = 0x3880;

    /// One site per accessor encoding, each with its own name, manager slot and
    /// loader, and the two `mov rcx,rbx` encodings of the loader call split
    /// across them. The `mov r8` sites name their table through a pointer cell
    /// holding the name's VA, exactly as the real ones do.
    fn four_encoding_image() -> Vec<u8> {
        let mut img = vec![0u8; 0x4000];
        let put = |img: &mut Vec<u8>, at: usize, b: &[u8]| {
            img[at..at + b.len()].copy_from_slice(b);
        };
        put(&mut img, 0x3000, b"gimmickinfo\0");
        put(&mut img, 0x3100, b"iteminfo\0");
        put(&mut img, 0x3200, b"FactionNode\0");
        put(&mut img, 0x3300, b"dropsetinfo\0");
        put(&mut img, 0x3400, b"decoytable\0");
        // The cells the three `mov r8` sites read, holding VAs.
        for (cell, name_rva) in [(0x3900, 0x3100), (0x3908, 0x3200), (0x3910, 0x3400)] {
            put(&mut img, cell, &((SYN_BASE + name_rva) as u64).to_le_bytes());
        }

        // (site, xor31, indirect, rip target, slot, call-b encoding, loader)
        let sites: [(usize, bool, bool, usize, usize, usize, usize); 5] = [
            (FOUR_A[0], false, false, 0x3000, FOUR[0].1, 0, FOUR[0].2),
            (FOUR_A[1], false, true, 0x3900, FOUR[1].1, 1, FOUR[1].2),
            (FOUR_A[2], true, false, 0x3300, FOUR[2].1, 1, FOUR[2].2),
            (FOUR_A[3], true, true, 0x3908, FOUR[3].1, 0, FOUR[3].2),
            (FOUR_DECOY_A, true, true, 0x3910, FOUR_DECOY_SLOT, 0, 0x2900),
        ];
        for (a, xor31, indirect, rip, slot, enc, loader) in sites {
            put_slot_load(&mut img, a - SYN_SLOT_BACK, slot, true);
            put_accessor(&mut img, a, accessor_head(xor31, indirect), rip);
            put_call_b(&mut img, a + 0x40, enc, loader);
            put(&mut img, loader, &LOADER_PROLOGUE);
        }
        img
    }

    /// The fixture really is what it claims to be: four different accessor
    /// encodings and both encodings of the loader call. Without this the tests
    /// below could pass on an image that only exercises one of each.
    #[test]
    fn the_four_encoding_fixture_uses_all_four() {
        let img = four_encoding_image();
        let heads: Vec<[u8; 6]> = FOUR_A
            .iter()
            .map(|&a| {
                let mut h = [0u8; 6];
                h.copy_from_slice(&img[a..a + 6]);
                h
            })
            .collect();
        assert_eq!(
            heads,
            vec![
                accessor_head(false, false),
                accessor_head(false, true),
                accessor_head(true, false),
                accessor_head(true, true),
            ]
        );
        // `mov rcx,rbx` sits 14 bytes into the loader call, and the fixture
        // uses `48 8B CB` for two sites and `48 89 D9` for the other two.
        let mov = |a: usize| img[a + 0x40 + 14..a + 0x40 + 17].to_vec();
        assert_eq!(mov(FOUR_A[0]), vec![0x48, 0x8B, 0xCB]);
        assert_eq!(mov(FOUR_A[1]), vec![0x48, 0x89, 0xD9]);
        assert_eq!(mov(FOUR_A[2]), vec![0x48, 0x89, 0xD9]);
        assert_eq!(mov(FOUR_A[3]), vec![0x48, 0x8B, 0xCB]);
    }

    /// The gap this all exists to close: a table whose accessor is any of the
    /// four encodings resolves, through either the slot or the loader path.
    #[test]
    fn every_accessor_encoding_resolves() {
        let img = four_encoding_image();
        for (table, slot, loader) in FOUR {
            let name = String::from_utf8_lossy(table);
            assert_eq!(
                resolve_manager_slot_based(&img, SYN_BASE, table),
                Ok(slot),
                "slot for {name}"
            );
            assert_eq!(
                resolve_record_loader_for(&img, SYN_BASE, table),
                Ok(SYN_BASE + loader),
                "loader for {name}"
            );
            // And what it resolved to really is a loader.
            assert_eq!(
                img.get(loader..loader + LOADER_STOLEN),
                Some(&LOADER_PROLOGUE[..]),
                "prologue for {name}"
            );
        }
        // The gatherer's table-less entry point still answers for gimmickinfo.
        assert_eq!(resolve_record_loader(&img, SYN_BASE), Ok(SYN_BASE + FOUR[0].2));
    }

    /// The decoy is a working site of its own, which is what makes it a decoy
    /// rather than dead bytes: it resolves when asked for by name and is
    /// silently passed over otherwise.
    #[test]
    fn the_decoy_site_is_skipped_not_an_error() {
        let img = four_encoding_image();
        assert_eq!(
            resolve_manager_slot_based(&img, SYN_BASE, b"decoytable"),
            Ok(FOUR_DECOY_SLOT)
        );
        for (table, slot, _) in FOUR {
            assert_eq!(resolve_manager_slot_based(&img, SYN_BASE, table), Ok(slot));
        }
    }

    /// Without an image base only the two `lea` encodings can be read, which is
    /// One scan answers for every table, and answers what the per-call
    /// resolvers answer.
    ///
    /// This is the whole claim `AccessorSites` makes: `desert-dispatch`
    /// resolves `FactionNode` and `dropsetinfo` from one scan instead of two,
    /// and must get the same two addresses it got when each call scanned for
    /// itself. The decoy site is in this image too, so "same answer" includes
    /// still skipping it.
    #[test]
    fn one_scan_answers_for_every_table() {
        let img = four_encoding_image();
        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).expect("the patterns are well formed");
        for (table, slot, loader) in FOUR {
            assert_eq!(
                sites.manager_slot(&img, table),
                resolve_manager_slot_based(&img, SYN_BASE, table),
                "manager slot disagrees for {}",
                String::from_utf8_lossy(table)
            );
            assert_eq!(sites.manager_slot(&img, table), Ok(slot));
            assert_eq!(
                sites.record_loader(&img, SYN_BASE, table),
                resolve_record_loader_for(&img, SYN_BASE, table),
                "record loader disagrees for {}",
                String::from_utf8_lossy(table)
            );
            assert_eq!(sites.record_loader(&img, SYN_BASE, table), Ok(SYN_BASE + loader));
        }
        // A base-less scan is the looter's shape: the two indirect encodings are
        // never searched, so the two tables only they name stay unreachable and
        // the other two answer exactly what the per-call resolver answers.
        let lea = AccessorSites::scan(&img, None).expect("the patterns are well formed");
        assert_eq!(lea.manager_slot(&img, GIMMICK_TABLE), resolve_manager_slot(&img, GIMMICK_TABLE));
        assert_eq!(lea.manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));
        assert_eq!(lea.manager_slot(&img, DROPSET_TABLE), Ok(FOUR[2].1));
        assert!(lea.manager_slot(&img, ITEM_TABLE).is_err());
        assert!(lea.manager_slot(&img, FACTION_NODE_TABLE).is_err());
        // A base-less scan still hands back an address for a `lea`-named table:
        // the base it lacks is the one that reads a name out of a pointer cell,
        // not the one an address is built from. This is the shape
        // `desert_harvest::gatherer` uses to halve its hook-install scan.
        assert_eq!(
            lea.record_loader(&img, SYN_BASE, GIMMICK_TABLE),
            Ok(SYN_BASE + FOUR[0].2)
        );
        assert!(lea.record_loader(&img, SYN_BASE, FACTION_NODE_TABLE).is_err());
    }

    /// A base-less scan does not search the indirect encodings at all.
    ///
    /// This is a cost guard, not a correctness one, and it is here because the
    /// cost was got wrong once: with the base as a per-lookup argument the scan
    /// walked all four encodings whatever the caller could use, which doubled
    /// the looter's startup scan from two passes per table to four. The
    /// census is the only outward sign of which encodings were searched.
    #[test]
    fn a_base_less_scan_skips_the_indirect_encodings() {
        let img = four_encoding_image();
        let lea = AccessorSites::scan(&img, None).expect("the patterns are well formed");
        let e = lea.manager_slot(&img, b"NoSuchTable").unwrap_err();
        for t in ACCESSORS.iter().filter(|t| !t.indirect) {
            assert!(e.contains(t.label), "census is missing {}: {e}", t.label);
        }
        for t in ACCESSORS.iter().filter(|t| t.indirect) {
            assert!(!e.contains(t.label), "census names the unsearched {}: {e}", t.label);
        }
        assert!(e.contains("the lea encodings"), "{e}");
    }

    /// Sites from one image are refused for another rather than answered.
    ///
    /// A lookup against the wrong image would read a plausible RVA out of
    /// unrelated bytes, which is the one thing this module must never do. The
    /// two images here are the same length, so the pointer is what separates
    /// them.
    #[test]
    fn sites_from_another_image_are_refused() {
        let img = four_encoding_image();
        let other = four_encoding_image();
        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).expect("the patterns are well formed");
        assert_eq!(sites.manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));
        let e = sites.manager_slot(&other, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("different image"), "{e}");
        let e = sites.record_loader(&other, SYN_BASE, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("different image"), "{e}");
    }

    /// The census in a failure message still counts every encoding, now out of
    /// the sites already found rather than by scanning the image again.
    #[test]
    fn the_census_still_names_every_encoding() {
        let img = four_encoding_image();
        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).expect("the patterns are well formed");
        let e = sites.manager_slot(&img, b"NoSuchTable").unwrap_err();
        for t in ACCESSORS {
            assert!(e.contains(t.label), "census is missing {}: {e}", t.label);
        }
        // Two sites of `45 31 C9 + mov r8` in this image: FOUR_A[3] and the decoy.
        assert!(e.contains("45 31 C9 + mov r8=2"), "{e}");
    }

    /// exactly the contract [`resolve_manager_slot`] has with the looter.
    #[test]
    fn the_base_less_resolver_sees_the_lea_encodings_only() {
        let img = four_encoding_image();
        // 33+lea and 31+lea: both reachable, and the 31 one is what the old
        // scan missed.
        assert_eq!(resolve_manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));
        assert_eq!(resolve_manager_slot(&img, DROPSET_TABLE), Ok(FOUR[2].1));
        // 33+mov and 31+mov: not without a base.
        for table in [ITEM_TABLE, FACTION_NODE_TABLE] {
            let e = resolve_manager_slot(&img, table).unwrap_err();
            assert!(e.contains("no accessor of the lea encodings"), "{e}");
        }
    }

    /// Two loader calls in one window is an ambiguity even when they are in
    /// different encodings: the point of scanning both is to find the one
    /// call, not to prefer an encoding.
    #[test]
    fn a_second_loader_call_encoding_in_the_window_is_ambiguous() {
        let mut img = four_encoding_image();
        put_call_b(&mut img, FOUR_A[0] + 0x70, 1, 0x2900);
        let e = resolve_record_loader_for(&img, SYN_BASE, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("2 loader calls"), "{e}");
        // The other sites are untouched by it.
        assert_eq!(
            resolve_record_loader_for(&img, SYN_BASE, DROPSET_TABLE),
            Ok(SYN_BASE + FOUR[2].2)
        );
    }

    /// A named table with no call in its window, and the degenerate images,
    /// are errors rather than panics on the generalised entry point too.
    #[test]
    fn loader_for_reports_what_failed() {
        let mut img = four_encoding_image();
        img[FOUR_A[2] + 0x40] = 0x00;
        let e = resolve_record_loader_for(&img, SYN_BASE, DROPSET_TABLE).unwrap_err();
        assert!(e.contains("no loader call"), "{e}");

        let img = four_encoding_image();
        let e = resolve_record_loader_for(&img, SYN_BASE, b"NoSuchTable").unwrap_err();
        assert!(e.contains("no accessor of any encoding"), "{e}");
        assert!(e.contains("45 31 C9 + mov r8=2"), "{e}");

        assert!(resolve_record_loader_for(&[], SYN_BASE, DROPSET_TABLE).is_err());
        assert!(resolve_record_loader_for(&[0x45, 0x31, 0xC9], 0, DROPSET_TABLE).is_err());
        assert!(resolve_record_loader_for(&[0x4C, 0x8B, 0x05], usize::MAX, ITEM_TABLE).is_err());
    }

    // -----------------------------------------------------------------------
    // The 26345091 shapes: r11 tail, disp32 loader call, any cmp register
    // -----------------------------------------------------------------------

    #[test]
    fn accessor_tails_parse_in_both_shapes_and_nothing_else() {
        assert_eq!(
            parse_accessor_tail(&[0x48, 0x8D, 0x4C, 0x24, 0x70, 0xE8, 1, 2, 3, 4]),
            Some((AccessorTail::RspDisp8(0x70), 5))
        );
        assert_eq!(
            parse_accessor_tail(&[0x49, 0x8D, 0x4B, 0x10, 0xE8]),
            Some((AccessorTail::R11Disp8(0x10), 4))
        );
        // Not a call after the lea, a different base register, too short.
        assert_eq!(parse_accessor_tail(&[0x49, 0x8D, 0x4B, 0x10, 0xE9]), None);
        assert_eq!(parse_accessor_tail(&[0x49, 0x8D, 0x4A, 0x10, 0xE8]), None);
        assert_eq!(parse_accessor_tail(&[0x48, 0x8D, 0x4C, 0x24, 0x70]), None);
        assert_eq!(parse_accessor_tail(&[]), None);
    }

    #[test]
    fn slot_cmp_accepts_every_register_against_rbx_plus_8() {
        let ok: Vec<u8> = (0..=255u8).filter(|&m| is_slot_cmp(&[0x3B, m, 0x08])).collect();
        assert_eq!(ok, vec![0x43, 0x4B, 0x53, 0x5B, 0x63, 0x6B, 0x73, 0x7B]);
        assert!(!is_slot_cmp(&[0x3B, 0x53, 0x10]));
        assert!(!is_slot_cmp(&[0x39, 0x53, 0x08]));
    }

    #[test]
    fn loader_calls_parse_in_every_grammar_shape() {
        let lea_r9 = [0x4C, 0x8D, 0x4C, 0x24, 0x30];
        let idx: [(&[u8], IndexLoad); 3] = [
            (&[], IndexLoad::None),
            (&[0x44, 0x0F, 0xB7, 0xC7], IndexLoad::Movzx),
            (&[0x44, 0x8B, 0xC6], IndexLoad::Mov),
        ];
        let rdx: [(&[u8], bool); 2] =
            [(&[0x48, 0x8D, 0x54, 0x24, 0x70], false), (&[0x48, 0x8D, 0x94, 0x24, 0x88, 0, 0, 0], true)];
        let rcx: [(&[u8], bool); 2] = [(&[0x48, 0x8B, 0xCB], false), (&[0x48, 0x89, 0xD9], true)];
        for (i, index) in idx {
            for (r, rdx_disp32) in rdx {
                for (c, rcx_89) in rcx {
                    let mut b = lea_r9.to_vec();
                    b.extend_from_slice(i);
                    b.extend_from_slice(r);
                    b.extend_from_slice(c);
                    let call_at = b.len();
                    b.extend_from_slice(&[0xE8, 1, 2, 3, 4]);
                    assert_eq!(
                        parse_loader_call(&b),
                        Some(LoaderCall { index, rdx_disp32, rcx_89, call_at }),
                        "{b:02X?}"
                    );
                    // Every truncation short of the E8 is rejected, not a panic.
                    for n in 0..=call_at {
                        assert_eq!(parse_loader_call(&b[..n]), None);
                    }
                }
            }
        }
        // An index load into some register other than r8d is not this call.
        let mut b = lea_r9.to_vec();
        b.extend_from_slice(&[0x44, 0x8B, 0xCF, 0x48, 0x8D, 0x54, 0x24, 0x70, 0x48, 0x8B, 0xCB, 0xE8]);
        assert_eq!(parse_loader_call(&b), None);
    }

    /// One accessor laid out the way build 26345091 assembles it: the slot
    /// load 0x33 before the head with `cmp edx`, an `r11` tail, and a loader
    /// call 0x7A past the file-load `E8` with no index load and a disp32
    /// `lea rdx`.
    fn put_new_site(img: &mut [u8], a: usize, name_rva: usize, slot: usize, loader: usize) {
        put_slot_load(img, a - 0x33, slot, true);
        let mut pa = accessor_head(false, false).to_vec();
        let disp = (name_rva as i64 - (a + A_RIP_AT) as i64) as i32;
        pa.extend_from_slice(&disp.to_le_bytes());
        pa.extend_from_slice(&[0x48, 0x8B, 0x53, 0x10, 0x49, 0x8D, 0x4B, 0x10, 0xE8, 0, 0, 0, 0]);
        img[a..a + pa.len()].copy_from_slice(&pa);
        let b = a + A_HEAD_LEN + 4 + 0x7A;
        let mut pb = vec![0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x48, 0x8D, 0x94, 0x24, 0x88, 0, 0, 0];
        pb.extend_from_slice(&[0x48, 0x8B, 0xCB, 0xE8]);
        let rel = (loader as i64 - (b + pb.len() + 4) as i64) as i32;
        pb.extend_from_slice(&rel.to_le_bytes());
        img[b..b + pb.len()].copy_from_slice(&pb);
        img[loader..loader + LOADER_STOLEN].copy_from_slice(&LOADER_PROLOGUE);
    }

    fn new_shape_image() -> Vec<u8> {
        let mut img = vec![0u8; 0x4000];
        img[0x3000..0x300C].copy_from_slice(b"gimmickinfo\0");
        img[0x3100..0x3109].copy_from_slice(b"iteminfo\0");
        put_new_site(&mut img, 0x1000, 0x3000, 0x3800, 0x2800);
        put_new_site(&mut img, 0x1400, 0x3100, 0x3820, 0x2840);
        img
    }

    #[test]
    fn the_26345091_shape_resolves_and_the_census_reports_it() {
        let img = new_shape_image();
        assert_eq!(resolve_record_loader(&img, SYN_BASE), Ok(SYN_BASE + 0x2800));
        assert_eq!(resolve_manager_slot(&img, GIMMICK_TABLE), Ok(0x3800));
        assert_eq!(resolve_manager_slot(&img, ITEM_TABLE), Ok(0x3820));

        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).expect("patterns parse");
        assert_eq!(sites.counts(), [2, 0, 0, 0]);
        assert!(sites.rejected().is_empty());
        let census = sites.census(&img).expect("same image");
        assert_eq!(census.len(), 2);
        let first = &census[0];
        assert_eq!(first.site, AccessorSite {
            at: 0x1000,
            tail: AccessorTail::R11Disp8(0x10),
            call_at: 0x1000 + A_HEAD_LEN + 4,
        });
        assert_eq!(first.name_rva, Some(0x3000));
        assert_eq!(first.slot, Ok((0x1000 - 0x33, 0x3800)));
        let (b, call, target) = first.loader.clone().expect("loader call");
        assert_eq!(b - first.site.call_at, 0x7A);
        assert_eq!(call, LoaderCall { index: IndexLoad::None, rdx_disp32: true, rcx_89: false, call_at: 16 });
        assert_eq!(target, 0x2800);
    }

    #[test]
    fn an_unknown_tail_is_rejected_and_counted_not_a_site() {
        let mut img = new_shape_image();
        // `lea rcx,[r10+0x10]`: one REX bit off the known shape.
        img[0x1400 + A_HEAD_LEN] = 0x49;
        img[0x1400 + A_HEAD_LEN + 2] = 0x4A;
        let sites = AccessorSites::scan(&img, None).expect("patterns parse");
        assert_eq!(sites.counts(), [1, 0, 0, 0]);
        assert_eq!(sites.rejected(), &[0x1400]);
        let e = sites.manager_slot(&img, ITEM_TABLE).unwrap_err();
        assert!(e.contains("unknown tail=1"), "{e}");
        assert_eq!(sites.manager_slot(&img, GIMMICK_TABLE), Ok(0x3800));
    }

    #[test]
    fn the_loader_search_stops_at_the_next_head_and_at_the_window() {
        // The second accessor's head sits between the first one's call and its
        // loader call: the search must stop there and not borrow the call.
        let mut img = new_shape_image();
        let call_at = 0x1000 + A_HEAD_LEN + 4;
        img[call_at + 0x40..call_at + 0x40 + A_HEAD_LEN]
            .copy_from_slice(&[0x45, 0x33, 0xC9, 0x4C, 0x8D, 0x05, 0, 0, 0, 0, 0x48, 0x8B, 0x53, 0x10]);
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no loader call"), "{e}");

        // A call that starts past B_WINDOW is not found.
        let mut img = vec![0u8; 0x4000];
        img[0x3000..0x300C].copy_from_slice(b"gimmickinfo\0");
        put_new_site(&mut img, 0x1000, 0x3000, 0x3800, 0x2800);
        let b = call_at + 0x7A;
        let moved = img[b..b + 21].to_vec();
        img[b..b + 21].fill(0);
        let far = call_at + B_WINDOW;
        img[far..far + 21].copy_from_slice(&moved);
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no loader call"), "{e}");
        // One byte inside the window is found (the rel32 is re-aimed).
        let near = far - 1;
        img[far..far + 21].fill(0);
        img[near..near + 17].copy_from_slice(&moved[..17]);
        let rel = (0x2800i64 - (near + 21) as i64) as i32;
        img[near + 17..near + 21].copy_from_slice(&rel.to_le_bytes());
        assert_eq!(resolve_record_loader(&img, SYN_BASE), Ok(SYN_BASE + 0x2800));
    }

    #[test]
    fn a_loader_call_whose_target_is_not_a_loader_is_an_error() {
        let mut img = new_shape_image();
        img[0x2800] ^= 0xFF;
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("does not open with LOADER_PROLOGUE"), "{e}");
        // The other table is unaffected.
        let sites = AccessorSites::scan(&img, None).expect("patterns parse");
        assert_eq!(sites.record_loader(&img, SYN_BASE, ITEM_TABLE), Ok(SYN_BASE + 0x2840));
    }

    #[test]
    fn two_checked_slot_loads_take_the_nearest() {
        let mut img = new_shape_image();
        // A second checked load further from the head than the real one.
        put_slot_load(&mut img, 0x1000 - 0x33 - 0x10, 0x3880, true);
        assert_eq!(resolve_manager_slot(&img, GIMMICK_TABLE), Ok(0x3800));
    }

    /// The fill sequence of the `gimmickinfo` accessor on build 26361232, from
    /// the end of its file-load call (`+0x3DD9BF`) to the loader call's
    /// `lea r9,[rsp+0x30]` (`+0x3DDA34`), byte for byte.
    const FILL_26361232: &str = "8b 84 24 88 00 00 00 85 c0 0f 85 cf 00 00 00 e8 ad ab 21 01 48 05 d0 07 00 00 \
        48 89 83 98 00 00 00 49 8b 0e 44 0f b7 c7 48 8b 43 28 4a 8d 14 c0 48 8d 2d 2a bc 6d 05 \
        48 89 6c 24 30 c6 44 24 38 00 45 33 f6 4c 89 74 24 40 4c 89 74 24 48 48 8d 35 94 bc 6d 05 \
        48 89 74 24 30 48 8b 01 48 89 44 24 50 8b 41 0c 89 44 24 58 44 89 74 24 5c 8b 42 04 \
        89 44 24 5c";

    fn hex_bytes(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    #[test]
    fn the_26361232_fill_parses_to_the_new_layout() {
        let code = hex_bytes(FILL_26361232);
        assert_eq!(parse_stream_fill(&code, 0x30), Some(StreamLayout::NEW));
        // The r9 displacement is what the offsets are relative to, not 0x30.
        assert_eq!(
            parse_stream_fill(&code, 0x20),
            Some(StreamLayout { buf: 0x30, size: 0x38, cursor: 0x3C })
        );
        // A store below the object's start is not a field of it.
        assert_eq!(parse_stream_fill(&code, 0x51), None);
    }

    #[test]
    fn a_fill_missing_a_store_or_with_one_twice_does_not_parse() {
        let code = hex_bytes(FILL_26361232);
        // Break the cursor load (mod 01 -> mod 00): no cursor.
        let cut = code.windows(3).position(|w| w == [0x8B, 0x42, 0x04]).unwrap();
        let mut missing = code.clone();
        missing[cut + 1] = 0x02;
        assert_eq!(parse_stream_fill(&missing, 0x30), None);
        // The size pair twice: ambiguous.
        let mut twice = code.clone();
        twice.extend_from_slice(&[0x8B, 0x41, 0x0C, 0x89, 0x44, 0x24, 0x60]);
        assert_eq!(parse_stream_fill(&twice, 0x30), None);
        // A negative disp8 is not a field of an object at a positive one.
        let mut neg = code.clone();
        let at = neg.windows(4).position(|w| w == [0x48, 0x89, 0x44, 0x24]).unwrap();
        neg[at + 4] = 0x90;
        assert_eq!(parse_stream_fill(&neg, 0x30), None);
        assert_eq!(parse_stream_fill(&[], 0x30), None);
    }

    #[test]
    fn the_register_allocated_variants_seen_on_26361232_parse() {
        // `NpcFriendlyInfo`: holder in rdx, 16-byte entries through r9.
        let npc = [
            0x48, 0x8B, 0x02, 0x48, 0x89, 0x44, 0x24, 0x50, 0x8B, 0x42, 0x0C, 0x89, 0x44, 0x24, 0x58,
            0x44, 0x89, 0x74, 0x24, 0x5C, 0x41, 0x8B, 0x41, 0x08, 0x89, 0x44, 0x24, 0x5C,
        ];
        assert_eq!(parse_stream_fill(&npc, 0x30), Some(StreamLayout::NEW));
        // `questinfo`: the cursor through a SIB, [rdx+r15+0x4].
        let quest = [
            0x48, 0x8B, 0x01, 0x48, 0x89, 0x44, 0x24, 0x50, 0x8B, 0x41, 0x0C, 0x89, 0x44, 0x24, 0x58,
            0x44, 0x89, 0x74, 0x24, 0x5C, 0x42, 0x8B, 0x44, 0x3A, 0x04, 0x89, 0x44, 0x24, 0x5C,
        ];
        assert_eq!(parse_stream_fill(&quest, 0x30), Some(StreamLayout::NEW));
        // The size must come off the buffer's holder: off another register it
        // is a second cursor candidate, which is ambiguous.
        let mut other = quest;
        other[9] = 0x42;
        assert_eq!(parse_stream_fill(&other, 0x30), None);
    }

    #[test]
    fn the_old_layout_round_trips_through_a_synthesised_fill() {
        let mut code = vec![0x90; 4];
        code.extend_from_slice(&[0x48, 0x8B, 0x01, 0x48, 0x89, 0x44, 0x24, 0x40]);
        code.extend_from_slice(&[0x8B, 0x41, 0x0C, 0x89, 0x44, 0x24, 0x48]);
        code.extend_from_slice(&[0x8B, 0x42, 0x04, 0x89, 0x44, 0x24, 0x4C]);
        assert_eq!(parse_stream_fill(&code, 0x30), Some(StreamLayout::OLD));
    }

    fn reading(l: StreamLayout, buf: usize, size: u32, cursor: u32) -> StreamReading {
        StreamReading { layout: l, buf: Some(buf), size: Some(size), cursor: Some(cursor) }
    }

    #[test]
    fn choose_layout_takes_the_one_that_validates() {
        // The 26361232 first call: the old offsets read two zeroed qwords.
        let c = [
            reading(StreamLayout::NEW, 0x1238_0000, 0x0150_0000, 0),
            reading(StreamLayout::OLD, 0, 0, 0),
        ];
        let (l, why) = choose_layout(&c, 0, 256 << 20).unwrap();
        assert_eq!(l, StreamLayout::NEW);
        assert!(why.contains("matches the offsets table"), "{why}");
        // And the other way round on an old build.
        let c = [
            reading(StreamLayout::NEW, 0x7FF0_0000, 0x10, 0x99),
            reading(StreamLayout::OLD, 0x1238_0000, 0x0150_0000, 0x40),
        ];
        assert_eq!(choose_layout(&c, 0x40, 256 << 20).map(|r| r.0), Ok(StreamLayout::OLD));
    }

    #[test]
    fn choose_layout_refuses_none_and_refuses_two() {
        let none = [
            reading(StreamLayout::NEW, 0, 0, 0),
            reading(StreamLayout::OLD, 0x1000, 0, 0),
            StreamReading { layout: StreamLayout::OLD, buf: None, size: None, cursor: None },
            reading(StreamLayout::NEW, 0x1000, 0x20, 0x8),
        ];
        let e = choose_layout(&none, 0, 0x100).unwrap_err();
        assert!(e.contains("null") && e.contains("size 0x0") && e.contains("cursor 0x8"), "{e}");
        // Over the cap.
        assert!(choose_layout(&[reading(StreamLayout::NEW, 0x1000, 0x101, 0)], 0, 0x100).is_err());
        // Both validate: an ambiguity.
        let two = [reading(StreamLayout::NEW, 0x1000, 0x20, 0), reading(StreamLayout::OLD, 0x2000, 0x20, 0)];
        assert!(choose_layout(&two, 0, 0x100).unwrap_err().contains("refusing to guess"));
        assert!(choose_layout(&[], 0, 0x100).is_err());
    }

    /// A synthetic address space: regions of bytes at addresses, read the way
    /// `safe::read_into` reads (all or nothing).
    struct Mem(Vec<(usize, Vec<u8>)>);

    impl Mem {
        fn read(&self, addr: usize, len: usize) -> Option<Vec<u8>> {
            self.0.iter().find_map(|(at, b)| {
                let off = addr.checked_sub(*at)?;
                b.get(off..off.checked_add(len)?).map(<[u8]>::to_vec)
            })
        }
    }

    /// Lay out `records` (key, blocks, load factor) under `l` in fresh memory.
    /// `(key, vanilla blocks, load factor)` for one synthetic record.
    type SynthRecord = (u32, Vec<(u32, u64, u64)>, u64);

    fn synth(l: &ParsedLayout, records: &[SynthRecord]) -> (Mem, Vec<ParsedSample>) {
        let mut mem = Vec::new();
        let mut samples = Vec::new();
        let mut next = 0x1000_0000usize;
        for (key, blocks, k) in records {
            let obj = next;
            let list = next + 0x1_0000;
            next += 0x10_0000;
            let mut o = vec![0u8; PARSED_WINDOW];
            // Filler that looks like nothing in particular: small odd values.
            for (i, c) in o.chunks_mut(4).enumerate() {
                c.copy_from_slice(&((i as u32) * 2 + 1001).to_le_bytes());
            }
            o[l.key..l.key + 4].copy_from_slice(&key.to_le_bytes());
            match l.head {
                None => {
                    o[l.list..l.list + 8].copy_from_slice(&(list as u64).to_le_bytes());
                    o[l.count..l.count + 4].copy_from_slice(&(blocks.len() as u32).to_le_bytes());
                }
                Some(h) => {
                    let hdr_at = next - 0x8_0000;
                    let mut hdr = vec![0u8; 0x40];
                    hdr[l.list..l.list + 8].copy_from_slice(&(list as u64).to_le_bytes());
                    hdr[l.count..l.count + 4].copy_from_slice(&(blocks.len() as u32).to_le_bytes());
                    o[h..h + 8].copy_from_slice(&(hdr_at as u64).to_le_bytes());
                    mem.push((hdr_at, hdr));
                }
            }
            let mut entries = vec![0u8; blocks.len() * l.entry];
            for (i, &(item, mn, mx)) in blocks.iter().enumerate() {
                let baddr = list + 0x1000 + i * 0x200;
                entries[i * l.entry..i * l.entry + 8].copy_from_slice(&(baddr as u64).to_le_bytes());
                let mut b = vec![0x5Au8; BLOCK_WINDOW];
                b[l.block_item..l.block_item + 4].copy_from_slice(&item.to_le_bytes());
                b[l.block_min..l.block_min + 8].copy_from_slice(&(mn * k).to_le_bytes());
                b[l.block_max..l.block_max + 8].copy_from_slice(&(mx * k).to_le_bytes());
                mem.push((baddr, b));
            }
            mem.push((obj, o));
            mem.push((list, entries));
            samples.push(ParsedSample { key: *key, obj, blocks: blocks.clone() });
        }
        (Mem(mem), samples)
    }

    fn sample_records() -> Vec<SynthRecord> {
        vec![
            (1001081, vec![(22008, 5, 5)], 2),
            (17020006, vec![(757006, 4, 7), (757007, 2, 3), (1, 1, 1)], 1),
            (1002971, vec![(710001, 1, 1)], 2),
            (17080001, vec![(5001, 1, 2), (5002, 3, 6)], 5),
        ]
    }

    #[test]
    fn the_old_layout_is_taken_when_it_fits() {
        let (mem, samples) = synth(&ParsedLayout::OLD, &sample_records());
        assert_eq!(probe_parsed_layout(&samples, |a, n| mem.read(a, n)), Ok(ParsedLayout::OLD));
    }

    #[test]
    fn a_shifted_layout_is_found_by_narrowing() {
        let moved = ParsedLayout {
            key: 0x18,
            head: None,
            list: 0x1f0,
            count: 0x1f8,
            entry: 16,
            entry_item: Some(8),
            block_item: 0x60,
            block_min: 0x10,
            block_max: 0x18,
        };
        let (mem, samples) = synth(&moved, &sample_records());
        assert_eq!(probe_parsed_layout(&samples, |a, n| mem.read(a, n)), Ok(moved));
        assert!(moved.describe().starts_with("key +0x18, list +0x1f0, count +0x1f8, entry 16"));
        // An 8-byte stride with no entry key field is found too.
        let narrow = ParsedLayout { entry: 8, entry_item: None, ..moved };
        let (mem, samples) = synth(&narrow, &sample_records());
        assert_eq!(probe_parsed_layout(&samples, |a, n| mem.read(a, n)), Ok(narrow));
    }

    #[test]
    fn the_probe_says_which_step_failed() {
        let (mem, mut samples) = synth(&ParsedLayout::OLD, &sample_records());
        // A remembered key that is in no object: the key step fails.
        if let Some(s) = samples.get_mut(0) {
            s.key = 0xDEAD_BEEF;
        }
        let e = probe_parsed_layout(&samples, |a, n| mem.read(a, n)).unwrap_err();
        assert!(e.starts_with("key: 0 candidate offsets"), "{e}");
        // Nothing readable at all.
        let e = probe_parsed_layout(&samples, |_, _| None).unwrap_err();
        assert!(e.starts_with("none of 4 remembered records"), "{e}");
        assert!(probe_parsed_layout(&[], |_, _| None).is_err());
        // Remembered yields that are no whole multiple of what is there.
        let (mem, mut samples) = synth(&ParsedLayout::OLD, &sample_records());
        for s in &mut samples {
            for b in &mut s.blocks {
                b.1 = 1_000_003;
            }
        }
        let e = probe_parsed_layout(&samples, |a, n| mem.read(a, n)).unwrap_err();
        assert!(e.starts_with("list/block:"), "{e}");
    }

    #[test]
    fn a_vanilla_factor_is_one_whole_multiplier_for_both_halves() {
        assert_eq!(vanilla_factor(10, 10, 5, 5), Some(2));
        assert_eq!(vanilla_factor(8, 14, 4, 7), Some(2));
        assert_eq!(vanilla_factor(4, 7, 4, 7), Some(1));
        assert_eq!(vanilla_factor(8, 7, 4, 7), None, "two different factors");
        assert_eq!(vanilla_factor(9, 14, 4, 7), None, "not a multiple");
        assert_eq!(vanilla_factor(0, 0, 4, 7), None);
    }

    #[test]
    fn load_factors_are_whole_and_bounded() {
        assert_eq!(load_factor(10, 5), Some(2));
        assert_eq!(load_factor(5, 5), Some(1));
        assert_eq!(load_factor(7, 5), None);
        assert_eq!(load_factor(0, 5), None);
        assert_eq!(load_factor(5, 0), None);
        assert_eq!(load_factor(5 * (MAX_LOAD_MULT + 1), 5), None);
    }

    /// Write `v` at `off` in the object of every sample (a second key copy,
    /// a vector capacity).
    fn poke_objects(mem: &mut Mem, samples: &[ParsedSample], off: usize, v: impl Fn(&ParsedSample) -> u32) {
        for smp in samples {
            if let Some((_, b)) = mem.0.iter_mut().find(|(at, _)| *at == smp.obj) {
                b[off..off + 4].copy_from_slice(&v(smp).to_le_bytes());
            }
        }
    }

    /// A `{ptr, size, cap}`-shaped layout with the key at `+0x10`, as the
    /// 26361232 dumps showed the key; only a test fixture.
    const SHAPED: ParsedLayout = ParsedLayout { key: 0x10, list: 0xD0, count: 0xD8, ..ParsedLayout::OLD };

    #[test]
    fn the_count_step_names_its_best_near_miss() {
        let (mut mem, samples) = synth(&SHAPED, &sample_records());
        // One record's count stops matching: no offset holds it on all four,
        // and the error says +0xd8 held it on three.
        poke_objects(&mut mem, &samples[..1], 0xD8, |_| 9);
        let e = probe_parsed_layout(&samples, |a, n| mem.read(a, n)).unwrap_err();
        assert!(e.contains("best count offset +0xd8 on 3 of 4"), "{e}");
    }

    #[test]
    fn the_item_finder_follows_the_chain() {
        let l = ParsedLayout { list: 0x70, count: 0x78, ..SHAPED };
        let (mem, samples) = synth(&l, &sample_records());
        let s = &samples[1];
        let item = s.blocks[0].0;
        let paths = find_item_paths(s.obj, item, |a, n| mem.read(a, n), 3, 200, 4);
        assert!(paths.contains(&vec![0x70, 0, l.block_item]), "{paths:x?}");
        // Bounded: no nodes, no hits; depth 0 sees only the object itself.
        assert!(find_item_paths(s.obj, item, |a, n| mem.read(a, n), 3, 0, 4).is_empty());
        assert!(find_item_paths(s.obj, item, |a, n| mem.read(a, n), 0, 200, 4).is_empty());
        assert!(find_item_paths(s.obj, item, |_, _| None, 3, 200, 4).is_empty());
    }

    /// The shape the 26361232 dumps showed (the key twice, a full vector whose
    /// size and capacity read the same), with block offsets that are not the
    /// old ones, so neither known layout fits and the general probe has to
    /// carry both key copies and both counts through to the end.
    #[test]
    fn redundant_copies_are_carried_and_collapsed() {
        let moved = ParsedLayout { block_item: 0x60, block_min: 0x10, block_max: 0x18, ..SHAPED };
        let (mut mem, samples) = synth(&moved, &sample_records());
        poke_objects(&mut mem, &samples, 0x44, |s| s.key);
        poke_objects(&mut mem, &samples, 0xDC, |s| s.blocks.len() as u32);
        assert_eq!(probe_parsed_layout(&samples, |a, n| mem.read(a, n)), Ok(moved));
    }

    #[test]
    fn choose_parsed_collapses_copies_and_refuses_two_structures() {
        let a = SHAPED;
        let copies = [ParsedLayout { key: 0x44, ..a }, ParsedLayout { count: 0xDC, ..a }, a, ParsedLayout { block_item: 0x6C, ..a }];
        assert_eq!(choose_parsed(&copies), Ok(a));
        // No adjacent count: the lowest one.
        let far = [ParsedLayout { count: 0x120, ..a }, ParsedLayout { count: 0x100, ..a }];
        assert_eq!(choose_parsed(&far).map(|l| l.count), Ok(0x100));
        let two = [a, ParsedLayout { list: 0x70, count: 0x78, ..a }];
        let e = choose_parsed(&two).unwrap_err();
        assert!(e.starts_with("list: 2 different layouts"), "{e}");
        assert!(choose_parsed(&[]).is_err());
    }

    #[test]
    fn the_26361232_layout_is_a_known_one_and_describes_itself() {
        let (mem, samples) = synth(&ParsedLayout::NEW, &sample_records());
        assert_eq!(probe_parsed_layout(&samples, |a, n| mem.read(a, n)), Ok(ParsedLayout::NEW));
        assert_eq!(
            ParsedLayout::NEW.describe(),
            "key +0x10, list +0x128 -> +0x0, count +0x128 -> +0x8, entry 16, block item +0x68, min +0x20, max +0x28"
        );
        assert_eq!(
            ParsedLayout::OLD.describe(),
            "key +0x8, list +0x278, count +0x280, entry 16, block item +0x68, min +0x20, max +0x28"
        );
    }

    fn hexb(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    /// `mine_rock_09` (key 1001482) as the 26361232 launch of 2026-11-01
    /// 21:05 UTC dumped it: the object's first 0x130 bytes, the list-header
    /// run its `+0x70` points into (its output-list header is the fourth,
    /// at `+0x30`, which the object's `+0x128` points at), the entries and
    /// the first block, byte for byte. The other two blocks were not dumped
    /// and are the first one with its item id swapped for theirs.
    #[test]
    fn the_26361232_layout_fits_the_real_mine_rock_bytes() {
        let obj = 0x39E_BAE0_3800usize;
        let mut o = hexb(
            "68 88 B6 45 01 00 00 00 40 10 DF BA 9E 03 00 00 0A 48 0F 00 00 00 00 00 60 D6 D3 BA 9E 03 00 00 \
             00 00 00 00 00 00 00 00 80 F0 D9 BA 9E 03 00 00 07 00 00 00 07 00 00 00 A0 F0 D9 BA 9E 03 00 00 \
             00 02 00 00 0A 48 0F 00 20 00 00 00 00 00 00 00 80 10 DF BA 9E 03 00 00 20 CD E9 46 01 00 00 00",
        );
        // The probe reads a 0x400-byte window; the real object is at least
        // that big (0x300 of it was dumped), so pad past what was shown.
        o.resize(PARSED_WINDOW, 0);
        o[0x70..0x88].copy_from_slice(&hexb(
            "30 EE D6 BA 9E 03 00 00 40 EE D6 BA 9E 03 00 00 50 EE D6 BA 9E 03 00 00",
        ));
        o[0xC8] = 0xFF;
        o[0xD0..0xE0].copy_from_slice(&hexb("00 F1 D9 BA 9E 03 00 00 03 00 00 00 03 00 00 00"));
        o[0x128..0x130].copy_from_slice(&hexb("60 EE D6 BA 9E 03 00 00"));
        let headers = hexb(
            "18 FB 7D BA 9E 03 00 00 01 00 00 00 01 00 00 00 20 FB 7D BA 9E 03 00 00 02 00 00 00 02 00 00 00 \
             28 FB 7D BA 9E 03 00 00 01 00 00 00 01 00 00 00 C0 9F D0 BA 9E 03 00 00 03 00 00 00 03 00 00 00 \
             A4 4E 00 00 7E C6 D8 9A 0A 48 0F 00 00 00 00 00 48 FB 7D BA 9E 03 00 00 01 00 00 00 01 00 00 00",
        );
        let entries = hexb(
            "40 D7 D3 BA 9E 03 00 00 00 00 00 00 FF FF FF FF B0 D7 D3 BA 9E 03 00 00 00 00 00 00 5F 72 6F 63 \
             20 D8 D3 BA 9E 03 00 00 00 00 00 00 00 00 00 00 80 E3 D3 BA 9E 03 00 00 00 00 00 00 00 00 00 00",
        );
        let block = |item: u32| {
            let mut b = hexb(
                "00 00 00 00 FF FF FF FF FF FF 00 00 BA 8C A4 1D 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 \
                 01 00 00 00 00 00 00 00 01 00 00 00 00 00 00 00 FF FF 00 00 00 00 00 00 27 05 00 00 00 00 00 00 \
                 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 \
                 00 00 00 00 00 00 00 00 F1 44 0F 00 00 00 00 00 00 00 00 00 FF FF FF FF 66 23 00 00 BA 8C A4 1D",
            );
            b[0x68..0x6C].copy_from_slice(&item.to_le_bytes());
            b
        };
        let mem = Mem(vec![
            (obj, o),
            (0x39E_BAD6_EE30, headers),
            (0x39E_BAD0_9FC0, entries),
            (0x39E_BAD3_D740, block(1_000_689)),
            (0x39E_BAD3_D7B0, block(1_000_555)),
            (0x39E_BAD3_D820, block(1_000_610)),
        ]);
        let samples = [ParsedSample {
            key: 1_001_482,
            obj,
            blocks: vec![(1_000_689, 1, 1), (1_000_555, 1, 1), (1_000_610, 1, 1)],
        }];
        assert_eq!(probe_parsed_layout(&samples, |a, n| mem.read(a, n)), Ok(ParsedLayout::NEW));
        // The old layout reads a heap pointer where the key was: not it.
        assert_ne!(u32_at(mem.read(obj, 0x10).as_deref().unwrap_or_default(), 0x08), Some(1_001_482));
        // The looter's reader takes the same bytes to the same three blocks,
        // and only the new layout reads them at all.
        let read = |a, n| mem.read(a, n);
        let got = parsed_outputs(obj, &ParsedLayout::NEW, &read).unwrap();
        assert_eq!(
            got.iter().map(|o| (o.item, o.min, o.max)).collect::<Vec<_>>(),
            vec![(1_000_689, 1, 1), (1_000_555, 1, 1), (1_000_610, 1, 1)]
        );
        assert!(parsed_outputs(obj, &ParsedLayout::OLD, &read).is_none());
        let mut vote = LayoutVote::default();
        vote.observe(obj, |k| k == 1_001_482, &read);
        assert_eq!(vote.hits(), &[0, 1]);
    }

    /// Give every synthetic record object a name at `at`: a string region,
    /// and for `**` a pointer cell between, written into fresh memory.
    fn name_records(mem: &mut Mem, samples: &[ParsedSample], names: &[&str], at: NameAt) {
        for (i, (s, name)) in samples.iter().zip(names).enumerate() {
            let str_at = 0x7000_0000 + i * 0x1000;
            let cell_at = str_at + 0x800;
            let mut text = name.as_bytes().to_vec();
            text.push(0);
            text.resize(NAME_MAX, 0x41);
            mem.0.push((str_at, text));
            let first = if at.derefs == 2 {
                mem.0.push((cell_at, (str_at as u64).to_le_bytes().to_vec()));
                cell_at
            } else {
                str_at
            };
            let obj = mem.0.iter_mut().find(|(a, _)| *a == s.obj).unwrap();
            obj.1[at.offset..at.offset + 8].copy_from_slice(&(first as u64).to_le_bytes());
        }
    }

    const SAMPLE_NAMES: [&str; 4] = ["gimmick_well_0001_parts01", "Begonia_01", "mine_rock_09", "Wheat_01"];

    fn known_sample_key(k: u32) -> bool {
        sample_records().iter().any(|(key, _, _)| *key == k)
    }

    #[test]
    fn parsed_outputs_reads_both_known_layouts() {
        for l in ParsedLayout::KNOWN {
            let (mem, samples) = synth(&l, &sample_records());
            for s in &samples {
                let got = parsed_outputs(s.obj, &l, &|a, n| mem.read(a, n)).unwrap();
                // `synth` writes min/max times the load factor; compare items
                // and that each is a whole multiple of vanilla.
                assert_eq!(got.len(), s.blocks.len());
                for (o, &(item, mn, mx)) in got.iter().zip(&s.blocks) {
                    assert_eq!(o.item, item);
                    assert!(vanilla_factor(o.min, o.max, mn, mx).is_some());
                }
            }
        }
    }

    #[test]
    fn parsed_outputs_refuses_a_bad_record_whole() {
        let l = ParsedLayout::NEW;
        let (mut mem, samples) = synth(&l, &sample_records());
        let read = |m: &Mem, a: usize| parsed_outputs(a, &l, &|x, n| m.read(x, n));
        // The wrong layout reads nothing.
        assert!(parsed_outputs(samples[1].obj, &ParsedLayout::OLD, &|a, n| mem.read(a, n)).is_none());
        // min > max in one block of three: the whole record goes.
        let obj = samples[1].obj;
        let list = obj + 0x1_0000;
        let b1 = list + 0x1000 + 0x200;
        let blk = mem.0.iter_mut().find(|(a, _)| *a == b1).unwrap();
        blk.1[l.block_min..l.block_min + 8].copy_from_slice(&9u64.to_le_bytes());
        assert!(read(&mem, obj).is_none());
        // A nonzero entry item that disagrees with its block is refused; one
        // that agrees is accepted (the zero every real record carries is
        // covered above).
        let obj = samples[0].obj;
        let list = obj + 0x1_0000;
        let entries = mem.0.iter_mut().find(|(a, _)| *a == list).unwrap();
        entries.1[8..12].copy_from_slice(&22008u32.to_le_bytes());
        assert_eq!(read(&mem, obj).map(|v| v.len()), Some(1));
        let entries = mem.0.iter_mut().find(|(a, _)| *a == list).unwrap();
        entries.1[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert!(read(&mem, obj).is_none());
        // Unreadable memory is `None`, never a panic, and so is a zero stride.
        assert!(parsed_outputs(obj, &l, &|_, _| None).is_none());
        assert!(parsed_outputs(obj, &ParsedLayout { entry: 0, ..l }, &|a, n| mem.read(a, n)).is_none());
    }

    #[test]
    fn layout_vote_picks_the_layout_the_records_were_laid_out_in() {
        for l in ParsedLayout::KNOWN {
            let (mem, samples) = synth(&l, &sample_records());
            let mut vote = LayoutVote::default();
            for s in &samples {
                vote.observe(s.obj, known_sample_key, &|a, n| mem.read(a, n));
            }
            assert_eq!(vote.best(), samples.len());
            assert_eq!(vote.verdict(3), Some(l), "{vote:?}");
            // Not enough agreeing records yet: no verdict, so nothing latches.
            assert_eq!(vote.verdict(5), None);
        }
        // Keys that are not gather keys vote for nothing.
        let (mem, samples) = synth(&ParsedLayout::NEW, &sample_records());
        let mut vote = LayoutVote::default();
        for s in &samples {
            vote.observe(s.obj, |_| false, &|a, n| mem.read(a, n));
        }
        assert_eq!(vote.hits(), &[0, 0]);
        assert_eq!(vote.verdict(1), None);
        // A tie is refused.
        let tie = LayoutVote { hits: [4, 4] };
        assert_eq!(tie.verdict(3), None);
    }

    #[test]
    fn the_name_probe_finds_the_old_offset_under_the_old_layout() {
        let (mut mem, samples) = synth(&ParsedLayout::OLD, &sample_records());
        name_records(&mut mem, &samples, &SAMPLE_NAMES, NameAt::OLD);
        let mut vote = NameVote::default();
        for (s, name) in samples.iter().zip(SAMPLE_NAMES) {
            // The table's case need not match the collect row's.
            vote.observe(s.obj, &name.to_ascii_uppercase(), &|a, n| mem.read(a, n));
        }
        assert_eq!(vote.verdict(3), Some(NameAt::OLD));
        assert_eq!(NameAt::OLD.describe(), "**(rec+0x10)");
        assert_eq!(
            read_name_at(samples[0].obj, NameAt::OLD, &|a, n| mem.read(a, n)).as_deref(),
            Some("gimmick_well_0001_parts01")
        );
    }

    #[test]
    fn the_name_probe_finds_a_moved_name_under_the_new_layout() {
        // The key is at +0x10 now, so the old name slot holds the key and
        // reads nothing; the name sits behind +0x08 in this synthetic object.
        let (mut mem, samples) = synth(&ParsedLayout::NEW, &sample_records());
        let at = NameAt { offset: 0x08, derefs: 2 };
        name_records(&mut mem, &samples, &SAMPLE_NAMES, at);
        let read = |a, n| mem.read(a, n);
        assert!(read_name_at(samples[0].obj, NameAt::OLD, &read).is_none());
        let mut vote = NameVote::default();
        for (s, name) in samples.iter().zip(SAMPLE_NAMES) {
            vote.observe(s.obj, name, &read);
        }
        assert_eq!(vote.verdict(3), Some(at));
        assert_eq!(vote.best(), 4);
        // Two records are not enough to latch on.
        let mut two = NameVote::default();
        for (s, name) in samples.iter().zip(SAMPLE_NAMES).take(2) {
            two.observe(s.obj, name, &read);
        }
        assert_eq!(two.verdict(3), None);
        // A single indirection is found too, and a wrong name finds nothing.
        let (mut mem, samples) = synth(&ParsedLayout::NEW, &sample_records());
        let at = NameAt { offset: 0x30, derefs: 1 };
        name_records(&mut mem, &samples, &SAMPLE_NAMES, at);
        let mut vote = NameVote::default();
        let mut wrong = NameVote::default();
        for (s, name) in samples.iter().zip(SAMPLE_NAMES) {
            vote.observe(s.obj, name, &|a, n| mem.read(a, n));
            wrong.observe(s.obj, "item_basic_onehand", &|a, n| mem.read(a, n));
        }
        assert_eq!(vote.verdict(3), Some(at));
        assert_eq!(wrong.verdict(1), None);
    }

    /// The two item record shapes the probe must tell apart: `Old` is `u32
    /// key @0, **(rec+0x08)` (through 25477059), `Swapped` is
    /// `**(rec+0x00), u32 key @8` (what 26361232 looks like by analogy with
    /// the gimmick move). Every record also has a unique heap pointer at
    /// `+0x10` and a constant at `+0x18`, the kind of neighbours a real
    /// record has; with `category` the constant slot instead points straight
    /// at a second identifier, so two name places qualify.
    #[derive(Clone, Copy, PartialEq)]
    enum ItemShape {
        Old,
        Swapped,
    }

    fn synth_items(shape: ItemShape, keys: &[u32], category: bool) -> (Mem, Vec<usize>) {
        let mut mem = Mem(Vec::new());
        let mut objs = Vec::new();
        let cat_at = 0x5000_0000usize;
        let mut cat = b"Category_Material\0".to_vec();
        cat.resize(NAME_MAX, 0);
        mem.0.push((cat_at, cat));
        for (i, &key) in keys.iter().enumerate() {
            let obj = 0x1000_0000 + i * 0x100;
            let str_at = 0x2000_0000 + i * 0x100;
            let cell = 0x3000_0000 + i * 0x100;
            let name = if key == MONEY_ITEM_KEY { "Money_Copper".to_string() } else { format!("item_basic_{i:04}") };
            let mut text = name.into_bytes();
            text.push(0);
            text.resize(NAME_MAX, 0);
            mem.0.push((str_at, text));
            mem.0.push((cell, (str_at as u64).to_le_bytes().to_vec()));
            let mut b = vec![0u8; 0x40];
            let (key_at, name_at) = match shape {
                ItemShape::Old => (0, 8),
                ItemShape::Swapped => (8, 0),
            };
            b[key_at..key_at + 4].copy_from_slice(&key.to_le_bytes());
            b[name_at..name_at + 8].copy_from_slice(&(cell as u64).to_le_bytes());
            b[0x10..0x18].copy_from_slice(&((0x4000_0000 + i * 0x10) as u64).to_le_bytes());
            let tail = if category { cat_at as u64 } else { 7 };
            b[0x18..0x20].copy_from_slice(&tail.to_le_bytes());
            mem.0.push((obj, b));
            objs.push(obj);
        }
        (mem, objs)
    }

    const KNOWN_ITEMS: [u32; 5] = [22008, 1_000_689, 757_006, 710_001, 720_004];

    fn item_keys(n: usize, money: bool) -> Vec<u32> {
        let mut keys: Vec<u32> = KNOWN_ITEMS.to_vec();
        if money {
            keys.push(MONEY_ITEM_KEY);
        }
        keys.extend((0..n as u32).map(|i| 3_000_000 + i));
        keys
    }

    fn known_items() -> std::collections::HashSet<u32> {
        // One id the table does not hold, as in game: not every output item
        // has to be loaded for the vote to work.
        KNOWN_ITEMS.iter().copied().chain([999_999_999]).collect()
    }

    #[test]
    fn the_item_probe_reads_the_old_shape() {
        let (mem, objs) = synth_items(ItemShape::Old, &item_keys(40, true), false);
        let got = probe_item_layout(&objs, &known_items(), &|a, n| mem.read(a, n));
        assert_eq!(got, Ok(ItemLayout::OLD));
        assert_eq!(ItemLayout::OLD.describe(), "key +0x0, name **(rec+0x8)");
    }

    #[test]
    fn the_item_probe_reads_the_swapped_shape() {
        let (mem, objs) = synth_items(ItemShape::Swapped, &item_keys(40, false), false);
        let read = |a, n| mem.read(a, n);
        let want = ItemLayout { key: 8, name: NameAt { offset: 0, derefs: 2 } };
        assert_eq!(probe_item_layout(&objs, &known_items(), &read), Ok(want));
        // The old reading gets a pointer's low half for a key and no name.
        assert!(read_name_at(objs[0], ItemLayout::OLD.name, &read).is_none());
        let mut keys = ItemKeyVote::default();
        for &o in &objs {
            keys.observe(o, &read);
        }
        let scores = keys.scores(&known_items());
        assert_eq!(scores[0], ItemKeyScore { known: 0, unique: true });
        assert_eq!(scores[2], ItemKeyScore { known: 5, unique: true });
        assert!(!scores[1].unique);
        assert_eq!(keys.record_with_key(8, 22008), Some(objs[0]));
        assert_eq!(keys.record_with_key(6, 22008), None);
    }

    #[test]
    fn the_item_probe_does_not_latch_on_too_little() {
        let known = known_items();
        // Too few records walked.
        let (mem, objs) = synth_items(ItemShape::Swapped, &item_keys(10, false), false);
        let err = probe_item_layout(&objs, &known, &|a, n| mem.read(a, n)).unwrap_err();
        assert!(err.starts_with("no key offset"), "{err}");
        // Enough records, but not one known id among their keys.
        let keys: Vec<u32> = (0..60).map(|i| 4_000_000 + i).collect();
        let (mem, objs) = synth_items(ItemShape::Old, &keys, false);
        let err = probe_item_layout(&objs, &known, &|a, n| mem.read(a, n)).unwrap_err();
        assert!(err.contains("+0x0:0 "), "{err}");
        // Nothing readable at all: an error, never a panic.
        assert!(probe_item_layout(&objs, &known, &|_, _| None).is_err());
        assert!(probe_item_layout(&[], &known, &|_, _| None).is_err());
    }

    #[test]
    fn a_duplicated_key_offset_is_not_a_key() {
        let mut keys = item_keys(40, false);
        keys.push(22008);
        let (mem, objs) = synth_items(ItemShape::Old, &keys, false);
        let err = probe_item_layout(&objs, &known_items(), &|a, n| mem.read(a, n)).unwrap_err();
        assert!(err.contains("+0x0:5(dup)"), "{err}");
    }

    #[test]
    fn the_money_record_settles_two_name_places() {
        // With a second identifier behind `*(rec+0x18)` in every record, two
        // places read a name from all of them: a tie, refused ...
        let (mem, objs) = synth_items(ItemShape::Swapped, &item_keys(40, false), true);
        let err = probe_item_layout(&objs, &known_items(), &|a, n| mem.read(a, n)).unwrap_err();
        assert!(err.starts_with("no name place"), "{err}");
        // ... until the money record is loaded and names one of them.
        let (mem, objs) = synth_items(ItemShape::Swapped, &item_keys(40, true), true);
        let got = probe_item_layout(&objs, &known_items(), &|a, n| mem.read(a, n));
        assert_eq!(got, Ok(ItemLayout { key: 8, name: NameAt { offset: 0, derefs: 2 } }));
    }

    #[test]
    fn a_record_name_is_probed_without_a_known_name() {
        let (mem, objs) = synth_items(ItemShape::Swapped, &item_keys(40, false), false);
        let read = |a, n| mem.read(a, n);
        let at = NameAt { offset: 0, derefs: 2 };
        assert_eq!(probe_record_name(&objs, 8, &read), Ok((at, Vec::new())));
        // Two places reading a name from every record: the lower one wins,
        // and the other is reported as tied.
        let (mem, objs) = synth_items(ItemShape::Swapped, &item_keys(40, false), true);
        let read = |a, n| mem.read(a, n);
        let got = probe_record_name(&objs, 8, &read);
        assert_eq!(got, Ok((at, vec![NameAt { offset: 0x18, derefs: 1 }])));
        // Too few records, or none readable: an error, never a panic.
        let few = objs.get(..5).unwrap_or(&[]);
        assert!(probe_record_name(few, 8, &read).is_err());
        assert!(probe_record_name(&objs, 8, &|_, _| None).is_err());
        assert!(probe_record_name(&[], 8, &read).is_err());
    }

    #[test]
    fn an_item_key_may_not_overlap_its_name_pointer() {
        let n = |offset| NameAt { offset, derefs: 2 };
        assert!(ItemLayout::OLD.consistent());
        assert!(ItemLayout { key: 8, name: n(0) }.consistent());
        assert!(!ItemLayout { key: 8, name: n(8) }.consistent());
        assert!(!ItemLayout { key: 0xC, name: n(8) }.consistent());
        assert!(ItemLayout { key: 4, name: n(8) }.consistent());
        assert!(ItemLayout { key: 0x10, name: n(8) }.consistent());
        assert!(is_identifier("Money_Copper") && is_identifier("abc"));
        assert!(!is_identifier("ab") && !is_identifier("UI/Icon/x") && !is_identifier("a b c"));
    }
}
