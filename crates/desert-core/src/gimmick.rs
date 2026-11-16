//! Pure byte logic for the `gimmickinfo` table: record headers, resource-output
//! lists, yield multiplication, and finding the game's record loader and the
//! global slots its data-table managers live in.
//!
//! The gatherer subsystem of `DesertTooling.asi` hooks the loader and rewrites
//! the raw table bytes in memory just before the game parses each record, which
//! is what made it a drop-in replacement for the on-disk offset-patch packs
//! it superseded: the same edits, but computed from the bytes instead of from a
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
//! output lists and their blocks) is found by signature and did not care. The
//! body in the game's archive (`gamedata/binarystaticinfo__/bin/gimmickinfo.staticinfobody`,
//! which `crates/tools` reads) is byte-identical to what the game loads into
//! memory.
//!
//! Inside a gather record, resource outputs are stored as one or more lists: a
//! `u32 count` followed by `count` blocks of exactly [`BLOCK`] bytes
//! (`DropInfoData` on disk):
//!
//! ```text
//! +0    u8   flag       always 1
//! +1    u32  item       item id
//! +5    u32  0          padding, zero on all 896 blocks
//! +42   u64  min        \ vanilla 1..=8
//! +50   u64  max        /
//! +58   u16  0xFFFF     literal FF FF (enchantLevel)
//! +60   u8   0          new on 26636972, unnamed; 0 on all 904 blocks
//! +61   u32  item       the same item id again          (+60 through 26480954)
//! +65   u32  0          padding, zero on all 904 blocks (+64 through 26480954)
//! ```
//!
//! **26636972 inserted one byte** into every on-disk `DropInfoData`, right
//! after the `FF FF`, so the block is 69 bytes and the second item copy and
//! its pad moved by one; everything at or before `+58` did not move. With the
//! old 68/60 the detector finds 0 lists in the 26636972 body; with 69/61 it
//! finds 581 lists / 904 blocks, the old population. The parsed block object
//! is unchanged (min `+0x20`, max `+0x28`, the new byte at `+0x61`). The
//! offsets in the 2026-09-12 note below are the 68-byte ones of that day.
//!
//! **Corrected 2026-09-12.** This diagram, [`ITEM_AT`] and [`ITEM_TAIL_AT`] all
//! had the item id 4 bytes too high (at `+5` and `+64`, which are the two zero
//! pads), and so did the block builders in this file's own tests and in
//! `crates/desert-core/tests/props.rs`, which is why nothing caught it. Confirmed over
//! **all 896** output blocks of build 25246367's clean table: `u32@+1 != 0`
//! 896/896, `u32@+1 == u32@+60` 896/896, `u32@+5 == 0` 896/896, `u32@+64 == 0`
//! 896/896. [`MIN_AT`], [`MAX_AT`], [`BLOCK`] and the `FF FF` are unaffected, so
//! [`multiply`], which writes only the two scalars, was never wrong; what was
//! wrong is [`OutputBlock::item`], always read as 0, and so every item id
//! `desert_harvest::gatherer::remember` stored and `hook::reapply` cross-checks
//! against. See `docs/findings/2026-09-12-water-wells.md` section 5.
//!
//! That signature (count 1..=64, every block valid, `1 <= min <= max <=`
//! [`MAX_QTY`]) was cross-checked offline against the whole table: inside the
//! 275 gather records the old offset pack edited it found exactly the 587
//! blocks the pack touched, and nothing else: no false positives. On
//! 26361232 `multiply` doubles every block of every `collect` row and nothing
//! else (`tests/gimmick_real.rs`, against the archive's table).
//!
//! # The loader
//!
//! The record loader is RVA `0x3DC7F0` on build 26636972 (`0x3D9280` on
//! 26480954, `0x3d89a0` on 26345091, `FUN_14038ba40` on 25477059). Its prologue
//! is shared by every static-info table's copy of the loader template, so it
//! cannot be signature-scanned directly. [`resolve_record_loader_for`] instead
//! goes through its caller, the table's **accessor function**, which is one of
//! 151 copies of another template; the copy for `gimmickinfo` is the one whose
//! `lea r8,[rip+disp]` points at the C string `"gimmickinfo\0"` (that string
//! occurs exactly once in the exe).
//!
//! # Getters and accessor functions (build 26636972)
//!
//! Each table's manager object lives in a global pointer **slot**. Since
//! 26636972 the code around it is two functions: one to four small
//! **getters** (record index in, parsed object or a default out), which load
//! the slot and bounds-check the index, and the **accessor function** they
//! call when the record is not parsed yet, which names the table, loads its
//! file and calls the record loader. The addresses are `gimmickinfo`'s, to be
//! read beside a disassembly and never relied on:
//!
//! ```text
//! getter +3dac70
//!   48 8B 0D rel32        mov  rcx,[rip+slot]     <- the manager slot (+7246398)
//!   44 3B 41 28           cmp  r8d,[rcx+0x28]     ; record count, manager+0x28
//!   ...                                           ; entries at manager+0x20, 24 bytes each
//!   E8 rel32              call +3e0cc0            ; the accessor function
//!
//! accessor function +3e0cc0  (mgr = rcx, status* = rdx, idx = r8w)
//!   +00  48 89 5C 24 10 55 56 57 41 54 ... mov rbp,rsp; sub rsp,0x60
//!        movzx r14d,r8w; mov rdi,rdx; mov rsi,rcx; ...
//!   +31  45 33 C9         xor  r9d,r9d            <- the head
//!        4C 8D 05 rel32   lea  r8,[rip+name]      ; -> "gimmickinfo"
//!        48 8B 51 08      mov  rdx,[rcx+0x8]      ; end of the head
//!        48 8D 4D 40      lea  rcx,[rbp+0x40]     <- the tail
//!        E8 rel32         call file-load
//!   ...  the stream fill, rbp-relative (see "The stream layout")
//!        4C 8D 4D D0      lea  r9,[rbp-0x30]      <- the loader call
//!        45 0F B7 C6      movzx r8d,r14w
//!        48 8B D7         mov  rdx,rdi
//!        48 8B CE         mov  rcx,rsi
//!        E8 rel32         call +3dc7f0            ; the loader
//! ```
//!
//! [`AccessorSites::scan`] walks the image **once**, for the getter's slot
//! load and bounds check (`48 8B 0D ?? ?? ?? ?? 44 3B 41 28`,
//! [`CENSUS_GETTERS`] = 159 hits), follows each getter's first `call` within
//! [`CALL_WINDOW`] bytes that lands on a function with an accessor head in
//! its first [`HEAD_WINDOW`] bytes (0x30 or 0x31 in, measured), and keeps one
//! site per accessor function. The head comes in two encodings ([`ACCESSORS`]),
//! with [`CENSUS_SITES`] copies each: `lea r8,[rip+name]` names the table
//! directly (131), `mov r8,[rip+cell]` through a pointer **cell** holding the
//! name's **virtual address** (20), which is the only reason
//! [`resolve_manager_slot_based`] needs an `image_base` at all. Where two to
//! four getters reach one accessor they all load the same slot, and the
//! resolvers refuse a table whose getters disagree.
//!
//! **History.** Through 26480954 there were no getters: each accessor was a
//! run inside a larger function, with its slot load (`mov rbx,[rip+slot];
//! cmp r32,[rbx+0x8]`) within 0x60 bytes *before* an inline head
//! `xor r9d,r9d; lea|mov r8,name; mov rdx,[rbx+0x10]` in four encodings (`45 33
//! C9` and `45 31 C9`, each with `lea` or `mov`), a tail `lea rcx,[r11+0x10]`
//! (`lea rcx,[rsp+0x70]` through 25477059), a loader call `lea r9,[rsp+0x30];
//! [index load]; lea rdx,[rsp+d]; mov rcx,rbx; call`, an rsp-relative stream
//! fill and a 12-byte loader prologue ending `push rbp; push rsi`. That was 154
//! copies on 26345091 through 26480954 and 149 through 25477059. The manager
//! kept its count at `+0x08` then, not `+0x28` (see [`crate::manager`]).
//!
//! # The loader call
//!
//! The call to the loader is read by [`parse_loader_call`] over a small
//! grammar rather than matched as one pattern, because its run-up is where
//! instruction scheduling moves things:
//!
//! ```text
//!   4C 8D 4D d8                                lea   r9,[rbp+d8]   the stream object
//!   [44|45 0F B7 C0+r | 44|45 8B C0+r | 44|45 0F B6 C0+r]   index into r8d (optional)
//!   48|49 8B D0+r                              mov   rdx,r64       the status pointer
//!   48|49 8B C8+r                              mov   rcx,r64       the manager
//!   E8 rel32                                   call  the loader
//! ```
//!
//! It is searched forward from the accessor's own file-load `call`, at most
//! [`B_WINDOW`] bytes and never past the next accessor head, and its target
//! must open with [`LOADER_PREFIX`]. On 26636972 all 151 have one: 126
//! `movzx r8d,r14w`, 24 `mov r8d,r14d`, 1 `movzx r8d,r14b`, every one with
//! `mov rdx,rdi; mov rcx,rsi`. Two (`AutoSpawnFilterInfo` and `KeyMap`, as on
//! every build since 26361232) land on a `jmp` thunk rather than a loader, so
//! 149 resolve;
//! a table whose call is missing is an error for that table only, and for
//! `gimmickinfo` the gatherer treats it as fatal.
//!
//! # The stream layout
//!
//! The loader's fourth argument is a stream object the accessor builds on its
//! own frame at `[rbp+d8]` (the `lea r9` above) just before the call, and the
//! gatherer reads three of its fields: the table body's address, its size and
//! the cursor. Through 25477059 they were at `+0x10`/`+0x18`/`+0x1c`; since
//! 26361232 the object has two more zeroed qwords and they are at
//! `+0x20`/`+0x28`/`+0x2c`, which read the hook a zero buffer and a zero size
//! for a whole dry run before anyone noticed. So the offsets are read off the
//! accessor's fill sequence ([`parse_stream_fill`], [`AccessorSites::stream_layout`])
//! rather than assumed. On 26636972 the fill is rbp-relative (it was
//! rsp-relative through 26480954) and still encodes [`StreamLayout::NEW`].
//!
//! # The manager slot
//!
//! [`resolve_manager_slot`] returns the **RVA** of the slot a named table's
//! getters load, so the looter carries no addresses as constants. It scans
//! without an image base, so it reaches only the `lea` accessors;
//! [`resolve_manager_slot_based`] is handed one and reaches both encodings,
//! `FactionNode` and `Skill` included. [`resolve_manager_slot`] is deliberately
//! left without one rather than being made a wrapper with an extra argument:
//! the looter calls it on every startup and has no image base to hand at that
//! point, and the two tables it asks about are both `lea` copies.

use crate::pattern::Pattern;

/// Bytes per resource-output block.
/// 69 since 26636972 (68 through 26480954; see the module docs).
pub const BLOCK: usize = 69;
/// Offset of the `u64` minimum inside a block.
pub const MIN_AT: usize = 42;
/// Offset of the `u64` maximum inside a block.
pub const MAX_AT: usize = 50;
/// Offset of the `u32` item id inside a block. Immediately after the flag byte,
/// and followed by four bytes of zero padding at [`PAD_AT`], which is where
/// this constant wrongly pointed until 2026-09-12 (see the module docs).
pub const ITEM_AT: usize = 1;
/// Offset of the block's second copy of the item id, after the `FF FF` and the
/// one byte 26636972 inserted behind it (`+60` through 26480954). The signature demands the two copies agree, which is what makes
/// either of them usable as an identity check against a parsed block object
/// later (`docs/reference-internals.md` section 16: the parsed block carries
/// the item id at `block+0x68`, and the parsed list entry's `entry+0x08` is
/// *not* a second copy of it: that field is the raw block's [`PAD_TAIL_AT`],
/// which reads zero on every gather block and nonzero on the 16 lists
/// `block_ok` excludes). What that field *is* has no name: section 16.1 called
/// it `DropInfoData._dropTagNameHash`, and that was wrong - the tag hash is at
/// raw block +22, parsed `block+0x0C` (`docs/findings/2026-09-13-buff-stat-census.md`
/// section 4.1, nonzero on 606 of 606 gather blocks).
pub const ITEM_TAIL_AT: usize = 61;
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

/// Bytes [`crate::hook`] steals from the `gimmickinfo` loader's prologue. The
/// first 15 bytes are `mov [rsp+0x20],rbx; mov [rsp+0x10],rdx; mov
/// [rsp+0x8],rcx`: three whole position-independent instructions, no
/// RIP-relative operand, no branch. 12 through 26480954, where the third
/// instruction was `push rbp; push rsi`.
pub const LOADER_STOLEN: usize = 15;
/// The bytes those 15 must be, checked before patching (a wrong patch is a
/// crash to desktop; a refusal is just a degraded plugin). Six of the 151
/// loaders open this way on 26636972, `gimmickinfo`'s among them; the other
/// 143 continue `push rbp; push rsi` after [`LOADER_PREFIX`].
pub const LOADER_PROLOGUE: [u8; LOADER_STOLEN] = [
    0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4C, 0x24, 0x08,
];
/// What every loader of the template opens with, `mov [rsp+0x20],rbx; mov
/// [rsp+0x10],rdx`: the check that a loader call's target **is** a loader.
/// [`LOADER_PROLOGUE`] is the stricter check a hook makes before patching one.
pub const LOADER_PREFIX: [u8; 10] = [0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10];

/// One encoding of the accessor function's head: `xor r9d,r9d`, the table
/// name into `r8`, `mov rdx,[rcx+0x8]`. What follows is the tail, which
/// [`parse_accessor_tail`] reads.
struct Accessor {
    /// Short name for the two bytes that vary, used in the error messages.
    label: &'static str,
    /// The head bytes, [`A_HEAD_LEN`] of them, from the `xor` through the
    /// `mov rdx,[rcx+0x8]`.
    head: &'static str,
    /// `true` when the RIP target is a pointer **cell** holding the name's
    /// virtual address (`mov r8,[rip+cell]`), `false` when the target is the
    /// name string itself (`lea r8,[rip+name]`).
    indirect: bool,
}

/// The accessor head in both encodings the compiler emits on 26636972, with
/// each one's copy count beside it ([`CENSUS_SITES`]). The `45 31 C9` form of
/// the `xor` that earlier builds also used is gone.
///
/// [`A_DISP_AT`] and [`A_RIP_AT`] hold for both: they differ only in the
/// opcode byte that chooses `lea` over `mov`.
const ACCESSORS: [Accessor; 2] = [
    Accessor { label: "lea r8", head: "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 51 08", indirect: false }, // 131
    Accessor { label: "mov r8", head: "45 33 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 51 08", indirect: true },  // 20
];
/// The labels of [`ACCESSORS`], in order, for a census printed by a caller.
pub const ACCESSOR_LABELS: [&str; 2] = [ACCESSORS[0].label, ACCESSORS[1].label];
/// Length of every accessor head; the tail starts here.
pub const A_HEAD_LEN: usize = 14;
/// Offset of the disp32 within an accessor hit, and the length of the
/// name-loading instruction from the hit (RIP is the address of the *next*
/// instruction).
const A_DISP_AT: usize = 6;
const A_RIP_AT: usize = 10;
/// How far into an accessor function its head may start. 0x30 or 0x31 on
/// every copy on 26636972 (24 and 127 of them).
pub const HEAD_WINDOW: usize = 0x80;

/// The getter's slot load and bounds check, `mov rcx,[rip+slot]; cmp
/// r8d,[rcx+0x28]`: the one pattern the scan walks the image for. Exactly
/// [`CENSUS_GETTERS`] hits on 26636972, every one a getter.
pub const GETTER_PATTERN: &str = "48 8B 0D ?? ?? ?? ?? 44 3B 41 28";
/// Offset of the slot load's disp32 and of the next instruction (its RIP).
const GETTER_DISP_AT: usize = 3;
const GETTER_RIP_AT: usize = 7;
/// Length of [`GETTER_PATTERN`]; the `call` is searched for after it.
const GETTER_LEN: usize = 11;
/// How far past the start of the slot load the getter's `call` to its accessor
/// may sit. 0x26 to 0x37 bytes in on 26636972 (0x37 in 142 of the 159). The
/// first `E8` byte is not always the call (in `gimmickinfo`'s getter one sits
/// inside a disp32 0x11 bytes in), so every `E8` in the window is tried in
/// order and the first that lands on an accessor head is taken.
pub const CALL_WINDOW: usize = 0x60;

/// Getters on 26636972: one per [`GETTER_PATTERN`] hit, 2 to 4 sharing an
/// accessor for a few tables.
pub const CENSUS_GETTERS: usize = 159;
/// Accessor functions on 26636972, per [`ACCESSORS`] encoding: 131 `lea`, 20
/// `mov`, 151 tables in all, each passing a different name.
pub const CENSUS_SITES: [usize; 2] = [131, 20];

/// Cap on the getter hits collected. The real count is 159; a scan that hits
/// this cap has stopped being a census and the error messages say so by
/// printing the counts.
const MAX_SITES: usize = 4096;

/// How an accessor's file-load call gets its first argument, the instruction
/// right after the head. The disp8 is carried but never constrained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessorTail {
    /// `48 8D 4D d8`: `lea rcx,[rbp+d8]`, every copy on 26636972 (`d8` is
    /// `0x40`). It was `lea rcx,[r11+0x10]` on 26345091 through 26480954 and
    /// `lea rcx,[rsp+0x70]` before that.
    RbpDisp8(u8),
}

/// Read the tail of an accessor: `b` starts right after the head. Returns the
/// shape and the offset of the file-load call's `E8` within `b`, or `None`
/// when `b` is not that shape (or too short to say).
pub fn parse_accessor_tail(b: &[u8]) -> Option<(AccessorTail, usize)> {
    match b {
        [0x48, 0x8D, 0x4D, d, 0xE8, ..] => Some((AccessorTail::RbpDisp8(*d), 4)),
        _ => None,
    }
}

/// One accessor function: where it starts, where its head is, which tail it
/// has and where the `E8` of its file-load call sits. All image offsets (RVAs
/// for an image laid out by RVA).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessorSite {
    /// Offset of the head, the `xor r9d,r9d`.
    pub at: usize,
    /// Offset of the accessor function, the target of its getters' `call`.
    pub func: usize,
    /// The tail [`parse_accessor_tail`] read.
    pub tail: AccessorTail,
    /// Offset of the file-load call's `E8`, where the search for the loader
    /// call starts.
    pub call_at: usize,
}

/// One getter: where its slot load is, the slot it loads and the accessor
/// function it calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Getter {
    /// Offset of the `mov rcx,[rip+slot]`.
    pub at: usize,
    /// RVA of the manager slot.
    pub slot: usize,
    /// Offset of the accessor function its `call` reaches.
    pub accessor: usize,
    /// The manager entry layout the getter's own bounds-checked lookup
    /// encodes ([`parse_getter_entries`]), `None` when it does not parse.
    pub entries: Option<EntryLayout>,
}

/// How a manager lays out its entries, as a getter indexes them: bytes per
/// entry and where the parsed object pointer sits in one.
///
/// 155 of the 159 getters on 26636972 read [`EntryLayout::MANAGER`] (24 bytes,
/// object at `+8`: `lea r,[r8+r8*2]` ... `cmp qword [rax+r*8+8],0`); 4, for
/// `AIEventTableInfo`, `NpcFriendlyInfo` and `characterappearanceindexinfo`,
/// read 32 bytes with the object at `+0x10` (`shl r,5; add r,[rcx+0x20]; cmp
/// qword [r+0x10],0`). The count at `+0x28` and the entries at `+0x20` are the
/// same in all 159. [`crate::manager`] reads only the first layout, so the
/// resolvers refuse a table whose getters encode any other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryLayout {
    /// Bytes per entry.
    pub stride: usize,
    /// Offset of the parsed object pointer inside an entry.
    pub object: usize,
}

impl EntryLayout {
    /// The layout [`crate::manager`] reads.
    pub const MANAGER: Self =
        Self { stride: crate::manager::ENTRY_STRIDE, object: crate::manager::ENTRY_OBJECT };
}

/// `lea r64,[x+x*2]` at the start of `b`: REX.W, `8D`, ModRM mod 00 r/m 100,
/// a SIB with scale 2 whose index and base are the same register.
fn is_lea_times_3(b: &[u8]) -> bool {
    matches!(b, [rex @ 0x48..=0x4F, 0x8D, m, sib, ..]
        if m & 0xC7 == 0x04
            && sib & 0xC0 == 0x40
            && (sib >> 3) & 7 == sib & 7
            && (rex >> 1) & 1 == rex & 1)
}

/// `shl r64,5` at the start of `b`: `48|49 C1 E0+r 05`.
fn is_shl_5(b: &[u8]) -> bool {
    matches!(b, [0x48 | 0x49, 0xC1, m, 0x05, ..] if m & 0xF8 == 0xE0)
}

/// The manager entry layout a getter's lookup encodes, read off `code`, the
/// bytes from the end of its slot load and bounds check to its `call`.
///
/// The object offset is the disp8 of the first `cmp qword [m+d8],0` (`REX.W
/// 83 /7 ib 00`, mod 01). The stride is 24 when that compare indexes by 8 off
/// a `lea r,[x+x*2]` (a SIB with scale 8, or a base register a `lea` times 3
/// fed), and 32 when its base came out of a `shl r,5` and no `lea` times 3
/// precedes it. Anything else is `None`. Never panics.
pub fn parse_getter_entries(code: &[u8]) -> Option<EntryLayout> {
    let tail = |i: usize| code.get(i..).unwrap_or_default();
    let (at, sib, disp) = (0..code.len()).find_map(|i| match tail(i) {
        [0x48..=0x4F, 0x83, m, sib, d, 0x00, ..] if m & 0xC0 == 0x40 && m & 0x38 == 0x38 && m & 7 == 4 => {
            Some((i, Some(*sib), *d))
        }
        [0x48..=0x4F, 0x83, m, d, 0x00, ..] if m & 0xC0 == 0x40 && m & 0x38 == 0x38 && m & 7 != 4 => {
            Some((i, None, *d))
        }
        _ => None,
    })?;
    let before = code.get(..at)?;
    let lea3 = (0..before.len()).any(|i| is_lea_times_3(before.get(i..).unwrap_or_default()));
    let shl5 = (0..before.len()).any(|i| is_shl_5(before.get(i..).unwrap_or_default()));
    let stride = match (sib, lea3, shl5) {
        (Some(sib), true, _) if sib & 0xC0 == 0xC0 => 24,
        (None, true, false) => 24,
        (None, false, true) => 32,
        _ => return None,
    };
    Some(EntryLayout { stride, object: usize::from(disp) })
}

/// How the index reaches `r8d` inside a loader call, when it is set there at
/// all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexLoad {
    /// Nothing between `lea r9` and `mov rdx`.
    None,
    /// `44|45 0F B7 C0+r`: `movzx r8d,r16`, 126 copies on 26636972 (124 of
    /// the 149 whose call reaches a loader).
    Movzx,
    /// `44|45 0F B6 C0+r`: `movzx r8d,r8b`, 1 copy.
    MovzxByte,
    /// `44|45 8B C0+r`: `mov r8d,r32`, the tables indexed by a full `u32`, 24
    /// copies.
    Mov,
}

/// One loader call as [`parse_loader_call`] read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoaderCall {
    /// What sits between the `lea r9` and the `mov rdx`.
    pub index: IndexLoad,
    /// The disp8 of `lea r9,[rbp+d8]`: where the stream object starts in the
    /// accessor's frame (`-0x30` on every copy on 26636972).
    pub r9_disp: i8,
    /// Offset of the call's `E8` from the start of the parsed bytes; the
    /// rel32 follows it.
    pub call_at: usize,
}

/// Read a call to the record loader at the start of `b`:
///
/// ```text
/// 4C 8D 4D d8                                   lea r9,[rbp+d8]
/// [44|45 0F B7 C0+r | 44|45 0F B6 C0+r | 44|45 8B C0+r]   optional index load into r8d
/// 48|49 8B D0+r                                 mov rdx,r64
/// 48|49 8B C8+r                                 mov rcx,r64
/// E8                                            call
/// ```
///
/// `None` for anything else. Never reads past `b`.
pub fn parse_loader_call(b: &[u8]) -> Option<LoaderCall> {
    let (r9_disp, rest) = match b {
        [0x4C, 0x8D, 0x4D, d, rest @ ..] => (*d as i8, rest),
        _ => return None,
    };
    let (index, rest) = match rest {
        [0x44 | 0x45, 0x0F, 0xB7, m, tail @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Movzx, tail),
        [0x44 | 0x45, 0x0F, 0xB6, m, tail @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::MovzxByte, tail),
        [0x44 | 0x45, 0x8B, m, tail @ ..] if m & 0xF8 == 0xC0 => (IndexLoad::Mov, tail),
        _ => (IndexLoad::None, rest),
    };
    let rest = match rest {
        [0x48 | 0x49, 0x8B, m, tail @ ..] if m & 0xF8 == 0xD0 => tail,
        _ => return None,
    };
    let rest = match rest {
        [0x48 | 0x49, 0x8B, m, tail @ ..] if m & 0xF8 == 0xC8 => tail,
        _ => return None,
    };
    if rest.first() != Some(&0xE8) {
        return None;
    }
    Some(LoaderCall { index, r9_disp, call_at: b.len() - rest.len() })
}

/// How far past the accessor's file-load `E8` a loader call may **start**.
/// Measured on 26636972 over all 151 accessors: 0x7A or 0x7B (the `lea r9`
/// 140 or 141 bytes past the head). The 0x74..0x7E of 26345091 through
/// 26480954 set it at 0xC0, and it stays there.
pub const B_WINDOW: usize = 0xC0;

/// Where the loader's fourth argument, the stream object, keeps the three
/// fields the gatherer reads: the raw table body's address, its size and the
/// cursor parked at the start of the record being loaded. Offsets within the
/// stream object, which lives in the accessor's frame at `[rbp+r9_disp]`
/// (`[rsp+r9_disp]` through 26480954).
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
    /// The layout since build 26361232, unchanged on 26636972.
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

/// `mov [rbp+d8],rax` (`48 89 45 d8`) when `wide`, else `mov [rbp+d8],eax`
/// (`89 45 d8`), at the start of `b`: its signed disp8.
fn rbp_store(b: &[u8], wide: bool) -> Option<i8> {
    match (wide, b) {
        (true, [0x48, 0x89, 0x45, d, ..]) | (false, [0x89, 0x45, d, ..]) => Some(*d as i8),
        _ => None,
    }
}

/// Read the stream layout off the instructions that fill the stream object,
/// as the accessor emits them between its file-load call and the loader call.
/// The `gimmickinfo` copy on 26636972:
///
/// ```text
/// 48 8B 5D 58     mov rbx,[rbp+0x58]       ; the holder
/// 48 8B 03        mov rax,[rbx]            ; holder -> buffer
/// 48 89 45 F0     mov [rbp-0x10],rax       ; buf = d8 - r9_disp
/// 8B 43 0C        mov eax,[rbx+0xc]        ; same holder
/// 89 45 F8        mov [rbp-0x8],eax        ; size
/// 44 89 6D FC     mov [rbp-0x4],r13d       ; (cursor zeroed first)
/// 8B 42 04        mov eax,[rdx+0x4]        ; entry[idx].file_offset
/// 89 45 FC        mov [rbp-0x4],eax        ; cursor
/// ```
///
/// The stores are the same in every copy; the loads are register-allocated
/// per copy, so the holder may be any base register without a SIB or RIP
/// operand, and the cursor may come through `[r+d8]` or a SIB `[b+i+d8]`. The
/// size is the one `mov eax,[holder+0xC]; mov [rbp+d8],eax` pair, the cursor
/// the one other such pair. Through 26480954 the stores were rsp-relative
/// (`mov [rsp+d8],rax`, `48 89 44 24 d8`); that form is no longer read.
///
/// `code` is the bytes from the end of the accessor's file-load call to the
/// start of the loader call; `r9_disp` is the disp8 of the loader call's
/// `lea r9,[rbp+d8]`, where the stream object starts. Each field must be found
/// exactly once, and each store must land at or past `r9_disp`. `None` for
/// anything else. Never panics.
pub fn parse_stream_fill(code: &[u8], r9_disp: i8) -> Option<StreamLayout> {
    let field = |d: i8| -> Option<u32> { u32::try_from(i32::from(d) - i32::from(r9_disp)).ok() };
    let tail = |i: usize| code.get(i..).unwrap_or_default();

    // The buffer: the one holder load followed by a qword store.
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

    // Every dword load-and-store pair, identified by where its store sits: a
    // disp8 of 0x40..0x47 just before a load also reads as a REX prefix, and
    // that is the same pair decoded twice, not a second one. The later start
    // (the decode without the phantom prefix) wins, since a real REX would
    // have to sit right after the previous instruction's last byte anyway.
    let mut pairs: Vec<(usize, (u8, u8), u8, i8)> = Vec::new();
    for i in 0..code.len() {
        let Some((reg, disp, n)) = dword_load(tail(i)) else { continue };
        let Some(d) = rbp_store(tail(i + n), false) else { continue };
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
/// (the record's manager entry, `entry[idx].file_offset`). Returns the winner and why it won, or why
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
                Err(format!("cursor 0x{c:X} != entry[idx].file_offset 0x{expected_cursor:X}"))
            }
            (Some(b), Some(s), Some(c)) => {
                Ok(format!("buffer 0x{b:X} non-null, size 0x{s:X} in range, cursor 0x{c:X} matches entry[idx].file_offset"))
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

/// The name the `gimmickinfo` accessor passes: the record loader the
/// gatherer half of `desert-harvest` hooks is the one reached through this string.
pub const GIMMICK_TABLE: &[u8] = b"gimmickinfo";
/// The name the `iteminfo` accessor passes.
pub const ITEM_TABLE: &[u8] = b"iteminfo";
/// The name the `FactionNode` accessor passes. Reachable only through the
/// indirect (`mov r8`) encoding, so only [`resolve_manager_slot_based`] finds
/// it. Its records carry the dispatch-mission ("faction operation")
/// sub-records `desert-dispatch` reads. Slot `0x7248518` on 26636972.
pub const FACTION_NODE_TABLE: &[u8] = b"FactionNode";
/// The name the `dropsetinfo` accessor passes: the dispatch-mission reward
/// table. A `lea` copy on 26636972 (slot `0x724AD60`); it was a `45 31 C9` +
/// `lea` copy on 25477059, invisible to this module until the scan covered
/// every encoding.
pub const DROPSET_TABLE: &[u8] = b"dropsetinfo";
/// The name the `buffinfo` accessor passes: the game's buff table, whose
/// records carry the `BuffData` objects that move a drop rate. A `lea` copy;
/// slot `0x724E130` on 26636972. Walked **read-only** by the dispatch
/// subsystem's buff census (`DumpBuffs`), which writes nothing and installs
/// nothing.
pub const BUFF_TABLE: &[u8] = b"buffinfo";
/// The name the `statusinfo` accessor passes: the stat table the buffs above
/// name by row index, and where `AddMoneyDropRate` is a record rather than an
/// enum value. Also a `lea` copy; slot `0x72463C8` on 26636972. Same census,
/// same read-only pass.
pub const STATUS_TABLE: &[u8] = b"statusinfo";
/// The name the `characterinfo` accessor passes: the species table, indexed by
/// the `u16` row a creature's `ClientStatusActorComponent+0x30` holds. A `lea`
/// copy, so the base-less scan reaches it; on 26636972 two getters call its
/// accessor and both load slot `0x7246310`. Its accessor body was obfuscated
/// code on 26345091 and 26356046; only the slot is used here. Read-only, by
/// `desert-harvest`'s species set.
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
    // `FF FF`, plausible item ids agreeing across `+1` and `+60`, 68-byte stride;
    // `+61` and 69 bytes since 26636972, where 581 lists / 904 blocks pass)
    // and differ only in that every one of their blocks carries a **nonzero**
    // `u32` at `PAD_TAIL_AT` while `PAD_AT` is zero. That field is *not* the
    // high half of a wide item id (that reading was checked and ruled out on
    // 2026-09-12). It is the list entry's own key, `_dropTagNameHash`, which
    // the parsed loop reads into `entry+0x08` (section 16.1), and records that
    // share a value share a drop group. Reading it as a pad is right for every
    // record the plugin edits and wrong in general. None of
    // them is one of the 275 gather records the old offset pack edited; they are chests,
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
static LOGGED_ORPHANS: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Every getter and accessor function in one image.
///
/// Finding them depends on the image and **nothing else**: the table name is
/// consulted only afterwards, to pick which accessor is the one wanted. So the
/// walk that finds them is the same walk whatever table is being resolved,
/// and a caller wanting more than one used to pay for it more than once:
/// `desert-dispatch` resolves several tables and scanned the image **eight**
/// times at startup, 0.88 to 1.81 s of its boot measured over six launches on
/// build 25246367.
///
/// Scan once, resolve as often as wanted. The getters and sites are a few
/// hundred entries, so keeping them costs nothing beside finding them again.
///
/// The image base is part of the **scan**, not of each lookup: without one
/// the `mov r8` encoding is unreadable (its name is only reachable through a
/// VA), so its accessors are not kept and never appear in a census or an
/// error. The looter resolves `iteminfo` and `gimmickinfo` without a base and
/// declines it for exactly that reason. Through 26480954 the head was what the
/// image walk searched for, so a base-less scan also walked for half as many
/// patterns; since 26636972 the walk is for the getter in either case, and
/// the base decides only which accessor functions are kept.
pub struct AccessorSites {
    /// The image these came from, as `(pointer, length)`. Checked on every
    /// lookup: a site is an offset into one particular image and means nothing
    /// in another, and answering confidently with the wrong address is the one
    /// failure this module exists to prevent.
    origin: (usize, usize),
    /// The base this was scanned for. `None` means the indirect encoding was
    /// never kept, so this cannot answer for a table only it names.
    image_base: Option<usize>,
    /// Sites per entry of [`ACCESSORS`], in that order, ascending. The
    /// indirect encoding's list is empty when [`Self::image_base`] is `None`,
    /// and empty is also what a genuinely absent encoding looks like, which is
    /// why the census is filtered by the same flag rather than by emptiness.
    sites: [Vec<AccessorSite>; 2],
    /// Every getter whose call reached an accessor head, kept or not,
    /// ascending by slot-load offset.
    getters: Vec<Getter>,
    /// Getters whose slot would not resolve or whose calls reached no
    /// accessor head, ascending.
    orphans: Vec<usize>,
    /// Every head kept, recognised tail or not, ascending: where one
    /// accessor's loader-call search must stop.
    heads: Vec<usize>,
    /// Heads whose tail [`parse_accessor_tail`] did not recognise, ascending.
    rejected: Vec<usize>,
}

/// One accessor site with everything the resolvers derive from it, for a
/// census ([`AccessorSites::census`]). Each derivation is independent: a site
/// whose slot is in doubt still reports its loader call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteReport {
    /// Index into [`ACCESSOR_LABELS`].
    pub encoding: usize,
    pub site: AccessorSite,
    /// RVA of the name the accessor passes, when it could be read.
    pub name_rva: Option<usize>,
    /// `(offset of the first getter's slot load, RVA of the slot every getter
    /// of this accessor loads)`.
    pub slot: Result<(usize, usize), String>,
    /// How many getters call this accessor.
    pub getters: usize,
    /// The entry layout those getters agree on, `None` if one does not parse
    /// or they disagree.
    pub entries: Option<EntryLayout>,
    /// `(offset of the loader call, its shape, RVA of its target)`. The target
    /// is known to open with [`LOADER_PREFIX`].
    pub loader: Result<(usize, LoaderCall, usize), String>,
    /// The stream layout the fill sequence before that loader call encodes
    /// ([`parse_stream_fill`]); `None` without a loader call or a fill that
    /// parses.
    pub stream: Option<StreamLayout>,
}

/// The one accessor head within the first [`HEAD_WINDOW`] bytes of `func`,
/// as `(index into ACCESSORS, head offset)`. `None` when there is none, or
/// more than one (which is not an accessor function this module knows).
fn head_in(img: &[u8], func: usize, pats: &[Pattern; 2]) -> Option<(usize, usize)> {
    let end = func.checked_add(HEAD_WINDOW + A_HEAD_LEN)?.min(img.len());
    let window = img.get(func..end)?;
    let mut found = None;
    for (enc, p) in pats.iter().enumerate() {
        for h in p.find_all(window, 2) {
            if found.is_some() {
                return None;
            }
            found = Some((enc, func + h));
        }
    }
    found
}

/// The accessor function the getter at `at` calls: the first `E8` within
/// [`CALL_WINDOW`] of the slot load whose target has an accessor head, as
/// `(function, encoding, head)`.
fn follow_getter(
    img: &[u8],
    at: usize,
    pats: &[Pattern; 2],
) -> Option<(usize, usize, usize, Option<EntryLayout>)> {
    let end = at.checked_add(CALL_WINDOW)?.min(img.len());
    for o in at.checked_add(GETTER_LEN)?..end {
        if img.get(o) != Some(&0xE8) {
            continue;
        }
        let Some(target) = disp32_at(img, o + 1)
            .and_then(|rel| rip_target(o + 5, rel))
            .filter(|&t| t < img.len())
        else {
            continue;
        };
        if let Some((enc, head)) = head_in(img, target, pats) {
            let entries = img.get(at + GETTER_LEN..o).and_then(parse_getter_entries);
            return Some((target, enc, head, entries));
        }
    }
    None
}

impl AccessorSites {
    /// Find every getter in `img` in **one** walk of it, follow each to its
    /// accessor function, and keep one site per function: both encodings with
    /// a base, the `lea` one without. A head whose tail is not a known shape
    /// is kept out of the sites, counted in [`Self::rejected`] and logged
    /// once; a getter that reaches no head is counted in [`Self::orphans`] and
    /// logged once.
    ///
    /// The only error is a malformed pattern constant, which is a bug in this
    /// file rather than anything about the image.
    pub fn scan(img: &[u8], image_base: Option<usize>) -> Result<Self, String> {
        let getter = Pattern::parse(GETTER_PATTERN).ok_or("the getter pattern is malformed")?;
        let parse = |t: &Accessor| {
            Pattern::parse(t.head).ok_or_else(|| format!("the accessor head \"{}\" is malformed", t.label))
        };
        let pats = [parse(&ACCESSORS[0])?, parse(&ACCESSORS[1])?];

        let mut getters = Vec::new();
        let mut orphans = Vec::new();
        // function -> (encoding, head), one entry per accessor however many
        // getters reach it.
        let mut funcs: std::collections::BTreeMap<usize, (usize, usize)> = Default::default();
        for at in getter.find_all(img, MAX_SITES) {
            let slot = disp32_at(img, at + GETTER_DISP_AT)
                .and_then(|d| rip_target(at + GETTER_RIP_AT, d))
                .filter(|&s| s < img.len());
            match (slot, follow_getter(img, at, &pats)) {
                (Some(slot), Some((func, enc, head, entries))) => {
                    getters.push(Getter { at, slot, accessor: func, entries });
                    funcs.insert(func, (enc, head));
                }
                _ => orphans.push(at),
            }
        }

        let mut sites: [Vec<AccessorSite>; 2] = Default::default();
        let mut heads = Vec::new();
        let mut rejected = Vec::new();
        for (&func, &(enc, at)) in &funcs {
            let Some(t) = ACCESSORS.get(enc) else { continue };
            // Not kept without a base: a site of the indirect encoding could
            // never be matched to a name.
            if t.indirect && image_base.is_none() {
                continue;
            }
            let Some(out) = sites.get_mut(enc) else { continue };
            heads.push(at);
            let tail_at = at.saturating_add(A_HEAD_LEN);
            match img.get(tail_at..).and_then(parse_accessor_tail) {
                Some((tail, e8)) => out.push(AccessorSite { at, func, tail, call_at: tail_at + e8 }),
                None => rejected.push(at),
            }
        }
        for list in &mut sites {
            list.sort_unstable_by_key(|s| s.at);
        }
        heads.sort_unstable();
        rejected.sort_unstable();
        if let Some(&first) = rejected.first() {
            log_once(&LOGGED_REJECTED_TAIL, || {
                let bytes = img.get(first + A_HEAD_LEN..).unwrap_or_default();
                format!(
                    "[gimmick] {} accessor heads have a tail that is not lea rcx,[rbp+d8]; first \
                     at +0x{first:X}, tail {:02X?}. The template's codegen changed: those tables \
                     are unreachable until gimmick.rs learns it",
                    rejected.len(),
                    bytes.get(..8).unwrap_or(bytes)
                )
            });
        }
        if let Some(&first) = orphans.first() {
            log_once(&LOGGED_ORPHANS, || {
                format!(
                    "[gimmick] {} of {} getters reach no accessor head; first at +0x{first:X}. \
                     Their tables are unreachable until gimmick.rs learns the new shape",
                    orphans.len(),
                    orphans.len() + getters.len()
                )
            });
        }
        Ok(Self {
            origin: (img.as_ptr() as usize, img.len()),
            image_base,
            sites,
            getters,
            orphans,
            heads,
            rejected,
        })
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

    /// How many sites each encoding has, in [`ACCESSOR_LABELS`] order. The
    /// indirect encoding, without a base, is 0. [`CENSUS_SITES`] on 26636972.
    pub fn counts(&self) -> [usize; 2] {
        let mut out = [0; 2];
        for (o, s) in out.iter_mut().zip(self.sites.iter()) {
            *o = s.len();
        }
        out
    }

    /// Every getter whose call reached an accessor head, whichever encoding,
    /// ascending. [`CENSUS_GETTERS`] of them on 26636972.
    pub fn getters(&self) -> &[Getter] {
        &self.getters
    }

    /// Getters (their slot-load offsets) that reached no accessor head. Zero
    /// on 26636972; anything else is the next codegen change arriving.
    pub fn orphans(&self) -> &[usize] {
        &self.orphans
    }

    /// Heads whose tail is not the known shape. Zero on 26636972.
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
                    slot: self.slot_of(site.func),
                    getters: self.getters.iter().filter(|g| g.accessor == site.func).count(),
                    entries: self.entries_of(site.func),
                    loader,
                    stream,
                });
            }
        }
        Ok(out)
    }

    /// [`resolve_manager_slot_based`] against sites already found, and
    /// [`resolve_manager_slot`] when these were scanned without a base.
    ///
    /// Refuses a table whose getters do not all encode
    /// [`EntryLayout::MANAGER`]: a slot handed out here is read through
    /// [`crate::manager`], and a 32-byte-entry manager read at 24 would be
    /// every record but the first read from the wrong place.
    pub fn manager_slot(&self, img: &[u8], table: &[u8]) -> Result<usize, String> {
        let func = named_accessor(self, img, table)?.func;
        let (_, slot) = self.slot_of(func)?;
        match self.entries_of(func) {
            Some(EntryLayout::MANAGER) => Ok(slot),
            Some(l) => Err(format!(
                "the getters of \"{}\" index {}-byte entries with the object at +0x{:X}, which \
                 desert_core::manager does not read",
                String::from_utf8_lossy(table),
                l.stride,
                l.object
            )),
            None => Err(format!(
                "the getters of \"{}\" do not agree on an entry layout that parses",
                String::from_utf8_lossy(table)
            )),
        }
    }

    /// Every slot whose getters all encode [`EntryLayout::MANAGER`], ascending
    /// and without repeats: the managers [`crate::manager`] may walk. For a
    /// caller that reaches managers some other way than by table name (the
    /// looter's RTTI census) and must not read a 32-byte one at 24.
    pub fn readable_slots(&self) -> Vec<usize> {
        let mut slots: Vec<usize> = self.getters.iter().map(|g| g.slot).collect();
        slots.sort_unstable();
        slots.dedup();
        slots.retain(|&s| {
            self.getters.iter().filter(|g| g.slot == s).all(|g| g.entries == Some(EntryLayout::MANAGER))
        });
        slots
    }

    /// The entry layout every getter of the accessor function at `func`
    /// encodes, `None` when one does not parse or they disagree.
    fn entries_of(&self, func: usize) -> Option<EntryLayout> {
        let mut layouts = self.getters.iter().filter(|g| g.accessor == func).map(|g| g.entries);
        let first = layouts.next()??;
        layouts.all(|l| l == Some(first)).then_some(first)
    }

    /// The slot every getter of the accessor function at `func` loads, as
    /// `(first getter's slot-load offset, slot RVA)`. An error when no getter
    /// calls it or when its getters disagree.
    fn slot_of(&self, func: usize) -> Result<(usize, usize), String> {
        let mine: Vec<&Getter> = self.getters.iter().filter(|g| g.accessor == func).collect();
        match mine.as_slice() {
            [] => Err(format!("no getter calls the accessor function at +0x{func:X}")),
            [first, rest @ ..] => {
                if rest.iter().all(|g| g.slot == first.slot) {
                    Ok((first.at, first.slot))
                } else {
                    Err(format!(
                        "the {} getters of the accessor function at +0x{func:X} load different \
                         slots: {:x?}",
                        mine.len(),
                        mine.iter().map(|g| (g.at, g.slot)).collect::<Vec<_>>()
                    ))
                }
            }
        }
    }

    /// The loader call inside `site`: searched forward from the site's own
    /// file-load `E8`, starting at most [`B_WINDOW`] bytes past it and never
    /// at or past the next accessor head, and accepted only if its target
    /// opens with [`LOADER_PREFIX`]. Returns `(call offset, shape, target)`.
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
        if img.get(rva..rva + LOADER_PREFIX.len()) != Some(&LOADER_PREFIX[..]) {
            return Err(format!(
                "loader call at +0x{b:X} targets +0x{rva:X}, which does not open with \
                 LOADER_PREFIX"
            ));
        }
        Ok((b, call, rva))
    }
}

/// Every accessor site, across every encoding kept, whose name resolves to
/// the C string `table`, in ascending address order.
///
/// With `image_base` of `None` the indirect encoding was never kept: its name
/// is only reachable through a VA, and a caller without a base cannot turn one
/// back into an RVA. Most sites name some other table and are simply not
/// collected, and so is one whose cell holds something that is not a VA inside
/// this image at all. Nothing here is an error, because the caller's "exactly
/// one match" is the real check.
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

/// How many accessors of each encoding are in `img`, as `"lea r8=131, ..."`,
/// for the resolvers' error messages, plus the getter count and the
/// rejected-tail and orphan-getter counts when there are any. A count that has
/// drifted from [`CENSUS_SITES`] / [`CENSUS_GETTERS`] is the first thing to
/// look at after a game update.
fn accessor_census(sites: &AccessorSites) -> String {
    let mut s = ACCESSORS
        .iter()
        .zip(sites.sites.iter())
        .filter(|(t, _)| !t.indirect || sites.image_base.is_some())
        .map(|(t, hits)| format!("{}={}", t.label, hits.len()))
        .collect::<Vec<_>>()
        .join(", ");
    s.push_str(&format!(", getters={}", sites.getters.len()));
    if !sites.rejected.is_empty() {
        s.push_str(&format!(", unknown tail={}", sites.rejected.len()));
    }
    if !sites.orphans.is_empty() {
        s.push_str(&format!(", orphan getters={}", sites.orphans.len()));
    }
    s
}

/// The one accessor site that names `table`.
///
/// Every table-specific resolver in this module starts here: the accessor is
/// what ties a table's name to its code and its data. Without a base only the
/// `lea` encoding is in scope; see [`accessors_naming`].
fn named_accessor(
    sites: &AccessorSites,
    img: &[u8],
    table: &[u8],
) -> Result<AccessorSite, String> {
    let named = accessors_naming(sites, img, table)?;
    // Which half of the census was in scope, so an error says what was and was
    // not searched rather than only how much of it there was.
    let scope = if sites.image_base.is_some() { "any encoding" } else { "the lea encoding" };
    match named.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!(
            "no accessor of {scope} names \"{}\"; scanned: {}",
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
/// virtual address: callers add the module base themselves. On build 26636972
/// this answers `0x7246370` for `iteminfo` and `0x7246398` for `gimmickinfo`
/// (`0x71C1080` and `0x71C10A8` on 26480954).
///
/// The slot is the one every getter of the named accessor function loads
/// (see the module docs). Scans the image on every call. A caller resolving
/// more than one table should hold an [`AccessorSites`] and call
/// [`AccessorSites::manager_slot`] instead.
pub fn resolve_manager_slot(img: &[u8], table: &[u8]) -> Result<usize, String> {
    AccessorSites::scan(img, None)?.manager_slot(img, table)
}

/// The **RVA** of the global pointer slot holding the manager object for the
/// table named `table`, resolved through **either** accessor encoding.
///
/// Same answer as [`resolve_manager_slot`] for every table a `lea` encoding
/// names, and the only way to reach the tables named through a pointer cell,
/// `FactionNode` and `Skill` among them. `image_base` is needed solely
/// because of that indirection: the cell's `u64` is the name string's **virtual
/// address**, so the base the image is laid out for is what turns it back into
/// an RVA. Pass the running module's base in the game (`MainModule::base`) and
/// `pe::Headers::image_base` for a file-backed image.
///
/// Scans the image on every call, which is what [`AccessorSites`] exists to
/// stop for a caller with several tables.
pub fn resolve_manager_slot_based(
    img: &[u8],
    image_base: usize,
    table: &[u8],
) -> Result<usize, String> {
    AccessorSites::scan(img, Some(image_base))?.manager_slot(img, table)
}

/// Resolve the `gimmickinfo` record loader in a mapped image.
///
/// `img` is the image laid out by RVA (`MainModule::bytes()` in the game, or
/// [`crate::pe::file_to_image`] on the exe file) and `image_base` is the base
/// it is (or would be) mapped at. Returns `image_base + rva` of the loader,
/// which on build 26636972 is `image_base + 0x3DC7F0`.
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
/// with the name spelled out. Whichever it is, it opens with [`LOADER_PREFIX`]:
/// the resolver checks that before answering. A caller about to hook it must
/// still check its own stolen bytes ([`LOADER_PROLOGUE`] for `gimmickinfo`)
/// before patching (the image can change between the two).
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
    /// its table is named by a `lea` encoding scan without a base and still be
    /// handed an address, which is `desert_harvest::gatherer`'s whole startup.
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
                "the stream fill between the accessor's call at +0x{:X} and the loader call at \
                 +0x{b:X} does not parse",
                site.call_at
            )
        })
    }
}

/// [`parse_stream_fill`] over the bytes between `site`'s file-load call (its
/// `E8` and rel32) and the loader call at `b`, with the disp8 of the loader
/// call's own `lea r9,[rbp+d8]` (`4C 8D 4D d8`, the byte at `b + 3`).
fn stream_fill_at(img: &[u8], site: &AccessorSite, b: usize) -> Option<StreamLayout> {
    let r9_disp = *img.get(b.checked_add(3)?)? as i8;
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
    /// output blocks of build 25246367's clean table and never names the
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
        // +60 stays zero: the byte 26636972 inserted.
        raw[b + 61..b + 65].copy_from_slice(&757_006u32.to_le_bytes()); // +61 item
        // +65 .. +69 stays zero: the other padding.

        assert_eq!(
            output_blocks(&raw),
            vec![OutputBlock { offset: b, item: 757_006, min: 4, max: 7 }],
            "peony_01's first block, byte for byte"
        );
        // And the two pads really are zero here, so the block that proves the
        // item offsets is also the block that proves what is at +5 and +65.
        assert_eq!(u32_at(&raw, b + 5), Some(0));
        assert_eq!(u32_at(&raw, b + 65), Some(0));
        assert_eq!(raw.len(), 4 + 69);

        // The same bytes with the item id written where the constants used to
        // say it was are not a block at all: the two "copies" are then zero and
        // the two pads disagree.
        let mut wrong = raw.clone();
        wrong[b + 1..b + 5].copy_from_slice(&0u32.to_le_bytes());
        wrong[b + 5..b + 9].copy_from_slice(&757_006u32.to_le_bytes());
        wrong[b + 61..b + 65].copy_from_slice(&0u32.to_le_bytes());
        wrong[b + 65..b + 69].copy_from_slice(&757_006u32.to_le_bytes());
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

    fn hex_bytes(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|h| u8::from_str_radix(h, 16).unwrap()).collect()
    }

    fn put(img: &mut [u8], at: usize, b: &[u8]) {
        img[at..at + b.len()].copy_from_slice(b);
    }

    fn rel32(from_end: usize, to: usize) -> [u8; 4] {
        ((to as i64 - from_end as i64) as i32).to_le_bytes()
    }

    /// The `gimmickinfo` getter on build 26636972 (`+0x3DAC70`) up to and
    /// including its `call` to the accessor, with the slot disp32 and the call
    /// rel32 zeroed. Note the `E8` inside `mov rax,[rip+default]`'s disp32 at
    /// `+0x19`: the scan must step over it.
    const GETTER_26636972: &str = "48 83 ec 28 44 0f b7 01 48 8b 0d 00 00 00 00 44 3b 41 28 72 0c \
        48 8b 05 fc e8 dd 06 48 83 c4 28 c3 48 8b 41 20 4b 8d 14 40 48 83 7c d0 08 00 48 89 5c 24 \
        20 48 8d 1c d0 75 20 48 8d 54 24 30 e8 00 00 00 00";
    /// Where the slot load sits in a getter, and where its `call` does.
    const G_SLOT_AT: usize = 8;
    const G_CALL_AT: usize = 0x3F;

    /// A 32-byte-entry getter on 26636972 (`+0x547D8F`, the same prologue in
    /// front of its slot load at `+0x547D97`), up to and including its call,
    /// slot disp32 and call rel32 zeroed.
    const GETTER32_26636972: &str = "48 83 ec 28 44 0f b7 01 48 8b 0d 00 00 00 00 44 3b 41 28 72 0c \
        48 8b 05 15 46 c7 06 48 83 c4 28 c3 48 89 5c 24 20 49 8b d8 48 c1 e3 05 48 03 59 20 48 83 7b \
        10 00 75 20 48 8d 54 24 30 e8 00 00 00 00";
    const G32_CALL_AT: usize = 0x3D;

    /// A 32-byte-entry getter at `at` loading `slot` and calling `func`.
    fn put_getter32(img: &mut [u8], at: usize, slot: usize, func: usize) {
        let mut g = hex_bytes(GETTER32_26636972);
        let s = at + G_SLOT_AT;
        g[G_SLOT_AT + 3..G_SLOT_AT + 7].copy_from_slice(&rel32(s + 7, slot));
        g[G32_CALL_AT + 1..G32_CALL_AT + 5].copy_from_slice(&rel32(at + G32_CALL_AT + 5, func));
        put(img, at, &g);
    }

    /// A getter at `at` loading `slot` and calling `func`.
    fn put_getter(img: &mut [u8], at: usize, slot: usize, func: usize) {
        let mut g = hex_bytes(GETTER_26636972);
        let s = at + G_SLOT_AT;
        g[G_SLOT_AT + 3..G_SLOT_AT + 7].copy_from_slice(&rel32(s + 7, slot));
        g[G_CALL_AT + 1..G_CALL_AT + 5].copy_from_slice(&rel32(at + G_CALL_AT + 5, func));
        put(img, at, &g);
    }

    /// The `gimmickinfo` accessor function's opening on 26636972, the 0x31
    /// bytes before its head.
    const ACC_PROLOGUE_26636972: &str = "48 89 5c 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8b ec \
        48 83 ec 60 45 0f b7 f0 48 8b fa 48 8b f1 45 33 ed 4c 89 6d 58 48 8d 45 58 48 89 44 24 20";
    const HEAD_AT: usize = 0x31;

    /// The same function from the end of its file-load call to its loader
    /// call's `lea r9,[rbp-0x30]` (`+0x3E0D08` to `+0x3E0D7E`), byte for byte:
    /// the stream fill [`parse_stream_fill`] reads, at its tail.
    const FILL_26636972: &str = "8b 45 40 85 c0 74 26 89 07 48 8b 5d 58 48 85 db 0f 84 ea 00 00 00 \
        48 8b cb e8 7a 4d 1b 01 e8 15 4a 1b 01 be fe 01 00 00 e9 ae 00 00 00 4b 8d 0c 76 48 8b 46 20 \
        48 8d 14 c8 4c 8d 25 80 b1 75 05 4c 89 65 d0 c6 45 d8 00 4c 89 6d e0 4c 89 6d e8 4c 8d 3d b9 \
        b0 75 05 4c 89 7d d0 48 8b 5d 58 48 8b 03 48 89 45 f0 8b 43 0c 89 45 f8 44 89 6d fc 8b 42 04 \
        89 45 fc";
    /// Offsets inside a synthetic accessor function: the file-load `E8`, and
    /// the loader call, 0x7B past it as on 26636972.
    const CALL_OFF: usize = HEAD_AT + A_HEAD_LEN + 4;
    const B_OFF: usize = CALL_OFF + 5 + 0x76;

    /// A whole accessor function at `func` as 26636972 assembles it: the
    /// prologue, the head of the given encoding naming `rip` (the name for
    /// `lea`, the pointer cell for `mov`), the `lea rcx,[rbp+0x40]` tail and
    /// file-load call, the real fill and a `movzx` loader call to `loader`.
    fn put_accessor_fn(img: &mut [u8], func: usize, indirect: bool, rip: usize, loader: usize) {
        let mut f = hex_bytes(ACC_PROLOGUE_26636972);
        assert_eq!(f.len(), HEAD_AT);
        let h = func + HEAD_AT;
        f.extend_from_slice(&[0x45, 0x33, 0xC9, 0x4C, if indirect { 0x8B } else { 0x8D }, 0x05]);
        f.extend_from_slice(&rel32(h + A_RIP_AT, rip));
        f.extend_from_slice(&[0x48, 0x8B, 0x51, 0x08, 0x48, 0x8D, 0x4D, 0x40, 0xE8]);
        f.extend_from_slice(&0x0102_0304u32.to_le_bytes());
        f.extend_from_slice(&hex_bytes(FILL_26636972));
        assert_eq!(f.len(), B_OFF);
        f.extend_from_slice(&[0x4C, 0x8D, 0x4D, 0xD0, 0x45, 0x0F, 0xB7, 0xC6, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8]);
        f.extend_from_slice(&rel32(func + f.len() + 4, loader));
        put(img, func, &f);
    }

    /// A loader in the 143-copy form (`LOADER_PREFIX`, then `push rbp; push
    /// rsi`), which the resolver accepts and the gatherer's hook would not.
    fn put_plain_loader(img: &mut [u8], at: usize) {
        put(img, at, &LOADER_PREFIX);
        put(img, at + LOADER_PREFIX.len(), &[0x55, 0x56]);
    }

    /// `(table, slot, loader)` for the four tables of [`synthetic_image`], in
    /// the order of [`FUNCS`]: two `lea`, two `mov`.
    const FOUR: [(&[u8], usize, usize); 4] = [
        (GIMMICK_TABLE, 0x7800, 0x5000),
        (DROPSET_TABLE, 0x7808, 0x5040),
        (ITEM_TABLE, 0x7810, 0x5080),
        (FACTION_NODE_TABLE, 0x7818, 0x50C0),
    ];
    /// Where the four accessor functions are, and a fifth, a `mov` decoy
    /// naming a table nobody asks about.
    const FUNCS: [usize; 4] = [0x1000, 0x1200, 0x1400, 0x1600];
    const DECOY_FUNC: usize = 0x1800;
    const DECOY_SLOT: usize = 0x7820;
    /// Where each function's getter is; the decoy's is the last.
    const GETTERS: [usize; 5] = [0x400, 0x480, 0x500, 0x580, 0x600];

    /// Four accessor functions, one getter each: `gimmickinfo` and
    /// `dropsetinfo` named with `lea`, `iteminfo` and `FactionNode` through
    /// pointer cells holding the names' VAs, plus a `mov` decoy naming
    /// "decoytable". `gimmickinfo`'s loader opens with [`LOADER_PROLOGUE`], the
    /// others with only [`LOADER_PREFIX`].
    fn synthetic_image() -> Vec<u8> {
        let mut img = vec![0u8; 0x8000];
        put(&mut img, 0x6000, b"gimmickinfo\0");
        put(&mut img, 0x6100, b"dropsetinfo\0");
        put(&mut img, 0x6200, b"iteminfo\0");
        put(&mut img, 0x6300, b"FactionNode\0");
        put(&mut img, 0x6400, b"decoytable\0");
        for (cell, name) in [(0x6900, 0x6200), (0x6908, 0x6300), (0x6910, 0x6400)] {
            put(&mut img, cell, &((SYN_BASE + name) as u64).to_le_bytes());
        }
        // (function, indirect, rip target, slot, loader)
        let fns = [
            (FUNCS[0], false, 0x6000, FOUR[0].1, FOUR[0].2),
            (FUNCS[1], false, 0x6100, FOUR[1].1, FOUR[1].2),
            (FUNCS[2], true, 0x6900, FOUR[2].1, FOUR[2].2),
            (FUNCS[3], true, 0x6908, FOUR[3].1, FOUR[3].2),
            (DECOY_FUNC, true, 0x6910, DECOY_SLOT, 0x5100),
        ];
        for ((func, indirect, rip, slot, loader), g) in fns.into_iter().zip(GETTERS) {
            put_accessor_fn(&mut img, func, indirect, rip, loader);
            put_getter(&mut img, g, slot, func);
            put_plain_loader(&mut img, loader);
        }
        put(&mut img, FOUR[0].2, &LOADER_PROLOGUE);
        img
    }

    #[test]
    fn the_fixture_is_what_it_claims() {
        let img = synthetic_image();
        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).unwrap();
        assert_eq!(sites.counts(), [2, 3]);
        assert_eq!(sites.getters().len(), 5);
        assert!(sites.orphans().is_empty());
        assert!(sites.rejected().is_empty());
        // The decoy E8 inside the getter's disp32 is really there.
        assert_eq!(img[GETTERS[0] + 0x19], 0xE8);
        let census = sites.census(&img).unwrap();
        assert_eq!(census.len(), 5);
        let g = census.iter().find(|r| r.site.func == FUNCS[0]).unwrap();
        assert_eq!(
            g.site,
            AccessorSite {
                at: FUNCS[0] + HEAD_AT,
                func: FUNCS[0],
                tail: AccessorTail::RbpDisp8(0x40),
                call_at: FUNCS[0] + CALL_OFF,
            }
        );
        assert_eq!(g.encoding, 0);
        assert_eq!(g.name_rva, Some(0x6000));
        assert_eq!(g.slot, Ok((GETTERS[0] + G_SLOT_AT, FOUR[0].1)));
        assert_eq!(g.getters, 1);
        assert_eq!(g.entries, Some(EntryLayout::MANAGER));
        assert_eq!(sites.readable_slots(), vec![FOUR[0].1, FOUR[1].1, FOUR[2].1, FOUR[3].1, DECOY_SLOT]);
        let (b, call, target) = g.loader.clone().unwrap();
        assert_eq!(b, FUNCS[0] + B_OFF);
        assert_eq!(b - g.site.call_at, 0x7B);
        assert_eq!(call, LoaderCall { index: IndexLoad::Movzx, r9_disp: -0x30, call_at: 14 });
        assert_eq!(target, FOUR[0].2);
        assert_eq!(g.stream, Some(StreamLayout::NEW));
    }

    #[test]
    fn getter_entry_layouts_parse_in_both_strides() {
        let code = |t: &str, call: usize| hex_bytes(t)[G_SLOT_AT + GETTER_LEN..call].to_vec();
        assert_eq!(parse_getter_entries(&code(GETTER_26636972, G_CALL_AT)), Some(EntryLayout::MANAGER));
        assert_eq!(
            parse_getter_entries(&code(GETTER32_26636972, G32_CALL_AT)),
            Some(EntryLayout { stride: 32, object: 0x10 })
        );
        // The no-SIB stride-24 form (`+0x6B9C7B`): the entry address is built
        // by `lea rsi,[rax+rdx*8]` off a `lea rdx,[rax+rax*2]`.
        let nosib = hex_bytes(
            "72 09 48 8b 0d b1 0b b0 06 eb 40 41 8b c0 48 8d 14 40 48 8b 41 20 48 8d 34 d0 48 83 7e 08 00 75 18",
        );
        assert_eq!(parse_getter_entries(&nosib), Some(EntryLayout::MANAGER));
        // No compare, or no stride evidence, is no layout.
        assert_eq!(parse_getter_entries(&[]), None);
        assert_eq!(parse_getter_entries(&hex_bytes("48 83 7b 10 00")), None);
        assert_eq!(parse_getter_entries(&hex_bytes("48 8d 14 40 48 c1 e3 05 48 83 7b 10 00")), None);
    }

    /// A table whose getters index 32-byte entries is never handed out as a
    /// slot `crate::manager` would read at 24, and is not a readable slot.
    #[test]
    fn a_32_byte_entry_table_is_refused() {
        let mut img = synthetic_image();
        put_getter32(&mut img, GETTERS[1], FOUR[1].1, FUNCS[1]);
        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).unwrap();
        let e = sites.manager_slot(&img, DROPSET_TABLE).unwrap_err();
        assert!(e.contains("32-byte entries with the object at +0x10"), "{e}");
        assert!(!sites.readable_slots().contains(&FOUR[1].1));
        assert!(sites.readable_slots().contains(&FOUR[0].1));
        let r = sites.census(&img).unwrap().into_iter().find(|r| r.site.func == FUNCS[1]).unwrap();
        assert_eq!(r.entries, Some(EntryLayout { stride: 32, object: 0x10 }));
        // The loader is still reachable: it is the manager read that is refused.
        assert_eq!(sites.record_loader(&img, SYN_BASE, DROPSET_TABLE), Ok(SYN_BASE + FOUR[1].2));
        // And two getters of one accessor disagreeing on the layout is no layout.
        let mut img = synthetic_image();
        put_getter32(&mut img, 0x700, FOUR[0].1, FUNCS[0]);
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("do not agree on an entry layout"), "{e}");
    }

    #[test]
    fn resolves_the_loader_through_the_named_accessor() {
        let img = synthetic_image();
        assert_eq!(resolve_record_loader(&img, SYN_BASE), Ok(SYN_BASE + FOUR[0].2));
        assert_eq!(img.get(FOUR[0].2..FOUR[0].2 + LOADER_STOLEN), Some(&LOADER_PROLOGUE[..]));
        assert_eq!(resolve_stream_layout(&img, SYN_BASE), Ok(StreamLayout::NEW));
    }

    #[test]
    fn every_table_resolves_through_either_encoding() {
        let img = synthetic_image();
        for (table, slot, loader) in FOUR {
            let name = String::from_utf8_lossy(table);
            assert_eq!(resolve_manager_slot_based(&img, SYN_BASE, table), Ok(slot), "slot for {name}");
            assert_eq!(
                resolve_record_loader_for(&img, SYN_BASE, table),
                Ok(SYN_BASE + loader),
                "loader for {name}"
            );
        }
        // The decoy is a working accessor of its own: it resolves when asked
        // for by name and is silently passed over otherwise.
        assert_eq!(resolve_manager_slot_based(&img, SYN_BASE, b"decoytable"), Ok(DECOY_SLOT));
    }

    #[test]
    fn resolve_reports_what_failed() {
        // No accessor names the table at all.
        let mut img = synthetic_image();
        put(&mut img, 0x6000, b"gimmickinfX\0");
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no accessor"), "{e}");

        // Two accessors name it.
        let mut img = synthetic_image();
        put(&mut img, 0x6100, b"gimmickinfo\0");
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("2 accessors"), "{e}");

        // The accessor is there but the call is not.
        let mut img = synthetic_image();
        img[FUNCS[0] + B_OFF] = 0x00;
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no loader call"), "{e}");

        // The call targets outside the image.
        let mut img = synthetic_image();
        let rel_at = FUNCS[0] + B_OFF + 15;
        img[rel_at..rel_at + 4].copy_from_slice(&0x7000_0000i32.to_le_bytes());
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("outside the image"), "{e}");

        // Empty and tiny images are errors, not panics.
        assert!(resolve_record_loader(&[], SYN_BASE).is_err());
        assert!(resolve_record_loader(&[0x48, 0x8B, 0x0D], SYN_BASE).is_err());
        assert!(resolve_manager_slot(&[], GIMMICK_TABLE).is_err());
        assert!(resolve_manager_slot_based(&[0x45, 0x33, 0xC9], usize::MAX, ITEM_TABLE).is_err());
    }

    #[test]
    fn manager_slot_reports_what_failed() {
        // No getter: the accessor is there and named, nothing calls it, so it
        // is never found at all.
        let mut img = synthetic_image();
        img[GETTERS[0] + G_SLOT_AT] = 0x00;
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("no accessor"), "{e}");
        // ...and one table's getter going missing does not affect another.
        assert_eq!(resolve_manager_slot(&img, DROPSET_TABLE), Ok(FOUR[1].1));

        // The slot is outside the image: the getter is an orphan.
        let mut img = synthetic_image();
        let disp_at = GETTERS[0] + G_SLOT_AT + 3;
        img[disp_at..disp_at + 4].copy_from_slice(&0x7000_0000i32.to_le_bytes());
        let sites = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(sites.orphans(), &[GETTERS[0] + G_SLOT_AT]);
        let e = sites.manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("orphan getters=1"), "{e}");
    }

    /// Two to four getters share an accessor for a few real tables. They must
    /// load one slot; a disagreement is an error, not a pick.
    #[test]
    fn getters_sharing_an_accessor_must_agree() {
        let mut img = synthetic_image();
        put_getter(&mut img, 0x700, FOUR[0].1, FUNCS[0]);
        let sites = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(sites.manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));
        let census = sites.census(&img).unwrap();
        assert_eq!(census.iter().find(|r| r.site.func == FUNCS[0]).unwrap().getters, 2);

        let mut img = synthetic_image();
        put_getter(&mut img, 0x700, 0x7900, FUNCS[0]);
        let e = resolve_manager_slot(&img, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("load different slots"), "{e}");
        assert_eq!(resolve_manager_slot(&img, DROPSET_TABLE), Ok(FOUR[1].1));
    }

    #[test]
    fn indirect_candidates_that_are_not_the_table_are_skipped_not_errors() {
        // A decoy cell whose value is below the image base, or whose VA lands
        // past the end of the image, is not a name in this image. Neither may
        // turn into an error for a table that resolves fine.
        for bad in [7u64, (SYN_BASE + 0x9000) as u64] {
            let mut img = synthetic_image();
            put(&mut img, 0x6910, &bad.to_le_bytes());
            assert_eq!(resolve_manager_slot_based(&img, SYN_BASE, FACTION_NODE_TABLE), Ok(FOUR[3].1));
        }
        // And a wrong base makes the *name* stop resolving, rather than
        // reading some other byte range as a string.
        let img = synthetic_image();
        let e = resolve_manager_slot_based(&img, SYN_BASE + 0x10, FACTION_NODE_TABLE).unwrap_err();
        assert!(e.contains("no accessor of any encoding"), "{e}");
    }

    #[test]
    fn the_census_names_every_encoding_it_kept() {
        let img = synthetic_image();
        let e = resolve_manager_slot_based(&img, SYN_BASE, b"NoSuchTable").unwrap_err();
        assert!(e.contains("lea r8=2"), "{e}");
        assert!(e.contains("mov r8=3"), "{e}");
        assert!(e.contains("getters=5"), "{e}");
        let e = resolve_manager_slot(&img, b"NoSuchTable").unwrap_err();
        assert!(e.contains("lea r8=2"), "{e}");
        assert!(!e.contains("mov r8"), "{e}");
    }

    /// One scan answers for every table, and answers what the per-call
    /// resolvers answer.
    ///
    /// This is the whole claim `AccessorSites` makes: `desert-dispatch`
    /// resolves `FactionNode` and `dropsetinfo` from one scan instead of two,
    /// and must get the same two addresses it got when each call scanned for
    /// itself. The decoy is in this image too, so "same answer" includes still
    /// skipping it.
    #[test]
    fn one_scan_answers_for_every_table() {
        let img = synthetic_image();
        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).unwrap();
        for (table, slot, loader) in FOUR {
            assert_eq!(sites.manager_slot(&img, table), resolve_manager_slot_based(&img, SYN_BASE, table));
            assert_eq!(sites.manager_slot(&img, table), Ok(slot));
            assert_eq!(
                sites.record_loader(&img, SYN_BASE, table),
                resolve_record_loader_for(&img, SYN_BASE, table)
            );
            assert_eq!(sites.record_loader(&img, SYN_BASE, table), Ok(SYN_BASE + loader));
        }
        // A base-less scan is the looter's shape: the indirect encoding is
        // never kept, so the two tables only it names stay unreachable and the
        // other two answer exactly what the per-call resolver answers.
        let lea = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(lea.manager_slot(&img, GIMMICK_TABLE), resolve_manager_slot(&img, GIMMICK_TABLE));
        assert_eq!(lea.manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));
        assert_eq!(lea.manager_slot(&img, DROPSET_TABLE), Ok(FOUR[1].1));
        assert!(lea.manager_slot(&img, ITEM_TABLE).is_err());
        assert!(lea.manager_slot(&img, FACTION_NODE_TABLE).is_err());
        // A base-less scan still hands back an address for a `lea`-named
        // table: the base it lacks is the one that reads a name out of a
        // pointer cell, not the one an address is built from. This is the
        // shape `desert_harvest::gatherer` uses.
        assert_eq!(lea.record_loader(&img, SYN_BASE, GIMMICK_TABLE), Ok(SYN_BASE + FOUR[0].2));
        assert!(lea.record_loader(&img, SYN_BASE, FACTION_NODE_TABLE).is_err());
    }

    /// A base-less scan keeps nothing of the indirect encoding.
    ///
    /// This was a cost guard when the head was what the image walk searched
    /// for (a per-lookup base once doubled the looter's startup scan); since
    /// 26636972 the walk is for the getter either way, and what stays is the
    /// contract: a base-less census never names, counts or answers for the
    /// `mov r8` encoding.
    #[test]
    fn a_base_less_scan_skips_the_indirect_encodings() {
        let img = synthetic_image();
        let lea = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(lea.counts(), [2, 0]);
        let e = lea.manager_slot(&img, b"NoSuchTable").unwrap_err();
        for t in ACCESSORS.iter().filter(|t| !t.indirect) {
            assert!(e.contains(t.label), "census is missing {}: {e}", t.label);
        }
        for t in ACCESSORS.iter().filter(|t| t.indirect) {
            assert!(!e.contains(t.label), "census names the unkept {}: {e}", t.label);
        }
        assert!(e.contains("the lea encoding"), "{e}");
        assert!(lea.census(&img).unwrap().iter().all(|r| r.encoding == 0));
    }

    /// Sites from one image are refused for another rather than answered. The
    /// two images here are the same length, so the pointer is what separates
    /// them.
    #[test]
    fn sites_from_another_image_are_refused() {
        let img = synthetic_image();
        let other = synthetic_image();
        let sites = AccessorSites::scan(&img, Some(SYN_BASE)).unwrap();
        assert_eq!(sites.manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));
        let e = sites.manager_slot(&other, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("different image"), "{e}");
        let e = sites.record_loader(&other, SYN_BASE, GIMMICK_TABLE).unwrap_err();
        assert!(e.contains("different image"), "{e}");
    }

    /// Two loader calls in one window is an ambiguity: the point is to find
    /// the one call, not to prefer one.
    #[test]
    fn a_second_loader_call_in_the_window_is_ambiguous() {
        let mut img = synthetic_image();
        // A second call in the fill, before the real one.
        let at = FUNCS[0] + CALL_OFF + 0x20;
        let mut c = vec![0x4C, 0x8D, 0x4D, 0xD0, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8];
        c.extend_from_slice(&rel32(at + c.len() + 4, FOUR[1].2));
        put(&mut img, at, &c);
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("2 loader calls"), "{e}");
        assert_eq!(resolve_record_loader_for(&img, SYN_BASE, DROPSET_TABLE), Ok(SYN_BASE + FOUR[1].2));
    }

    #[test]
    fn the_loader_search_stops_at_the_window_and_the_next_head() {
        // A call that starts past B_WINDOW is not found; one byte inside it is.
        let mut img = synthetic_image();
        let call = img[FUNCS[0] + B_OFF..FUNCS[0] + B_OFF + 19].to_vec();
        img[FUNCS[0] + B_OFF..FUNCS[0] + B_OFF + 19].fill(0);
        let far = FUNCS[0] + CALL_OFF + B_WINDOW;
        let mut moved = call.clone();
        moved[15..19].copy_from_slice(&rel32(far + 19, FOUR[0].2));
        put(&mut img, far, &moved);
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no loader call"), "{e}");
        img[far..far + 19].fill(0);
        let near = far - 1;
        moved[15..19].copy_from_slice(&rel32(near + 19, FOUR[0].2));
        put(&mut img, near, &moved);
        assert_eq!(resolve_record_loader(&img, SYN_BASE), Ok(SYN_BASE + FOUR[0].2));

        // Another accessor's head between the file-load call and the loader
        // call: the search stops there and does not borrow the call. Its
        // function starts 0x60 into this one and a getter of its own reaches it.
        let mut img = synthetic_image();
        let inner = FUNCS[0] + 0x60;
        put(&mut img, inner + HEAD_AT, &[0x45, 0x33, 0xC9, 0x4C, 0x8D, 0x05, 0, 0, 0, 0, 0x48, 0x8B, 0x51, 0x08]);
        put_getter(&mut img, 0x700, 0x7900, inner);
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("no loader call"), "{e}");
    }

    #[test]
    fn a_loader_call_whose_target_is_not_a_loader_is_an_error() {
        let mut img = synthetic_image();
        img[FOUR[0].2] ^= 0xFF;
        let e = resolve_record_loader(&img, SYN_BASE).unwrap_err();
        assert!(e.contains("does not open with LOADER_PREFIX"), "{e}");
        // The other tables are unaffected, and a loader with only the common
        // prefix is still a loader to the resolver.
        let sites = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(sites.record_loader(&img, SYN_BASE, DROPSET_TABLE), Ok(SYN_BASE + FOUR[1].2));
        assert_ne!(img.get(FOUR[1].2..FOUR[1].2 + LOADER_STOLEN), Some(&LOADER_PROLOGUE[..]));
    }

    #[test]
    fn an_unknown_tail_is_rejected_and_counted_not_a_site() {
        let mut img = synthetic_image();
        // `lea rcx,[rbx+0x40]`: one ModRM bit off the known shape.
        img[FUNCS[1] + HEAD_AT + A_HEAD_LEN + 2] = 0x4B;
        let sites = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(sites.counts(), [1, 0]);
        assert_eq!(sites.rejected(), &[FUNCS[1] + HEAD_AT]);
        let e = sites.manager_slot(&img, DROPSET_TABLE).unwrap_err();
        assert!(e.contains("unknown tail=1"), "{e}");
        assert_eq!(sites.manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));
    }

    #[test]
    fn a_getter_that_reaches_no_head_is_an_orphan() {
        // The head moved past HEAD_WINDOW.
        let mut img = synthetic_image();
        let h = FUNCS[1] + HEAD_AT;
        let head = img[h..h + A_HEAD_LEN].to_vec();
        img[h..h + A_HEAD_LEN].fill(0x90);
        put(&mut img, FUNCS[1] + HEAD_WINDOW + 1, &head);
        let sites = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(sites.orphans(), &[GETTERS[1] + G_SLOT_AT]);
        assert!(sites.manager_slot(&img, DROPSET_TABLE).is_err());
        assert_eq!(sites.manager_slot(&img, GIMMICK_TABLE), Ok(FOUR[0].1));

        // Two heads in the window: not an accessor function this module knows.
        let mut img = synthetic_image();
        put(&mut img, FUNCS[1] + 0x10, &[0x45, 0x33, 0xC9, 0x4C, 0x8D, 0x05, 0, 0, 0, 0, 0x48, 0x8B, 0x51, 0x08]);
        let sites = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(sites.orphans(), &[GETTERS[1] + G_SLOT_AT]);

        // The call moved past CALL_WINDOW.
        let mut img = synthetic_image();
        let g = GETTERS[1];
        let call = img[g + G_CALL_AT..g + G_CALL_AT + 5].to_vec();
        img[g + G_CALL_AT..g + G_CALL_AT + 5].fill(0x90);
        let far = g + G_SLOT_AT + CALL_WINDOW;
        put(&mut img, far, &call);
        put(&mut img, far + 1, &rel32(far + 5, FUNCS[1]));
        let sites = AccessorSites::scan(&img, None).unwrap();
        assert_eq!(sites.orphans(), &[g + G_SLOT_AT]);
    }

    #[test]
    fn accessor_tails_parse_in_the_one_shape_and_nothing_else() {
        assert_eq!(parse_accessor_tail(&[0x48, 0x8D, 0x4D, 0x40, 0xE8, 1, 2, 3, 4]), Some((AccessorTail::RbpDisp8(0x40), 4)));
        // Not a call after the lea, a different base register, the old shapes,
        // too short.
        assert_eq!(parse_accessor_tail(&[0x48, 0x8D, 0x4D, 0x40, 0xE9]), None);
        assert_eq!(parse_accessor_tail(&[0x48, 0x8D, 0x4B, 0x40, 0xE8]), None);
        assert_eq!(parse_accessor_tail(&[0x49, 0x8D, 0x4B, 0x10, 0xE8]), None);
        assert_eq!(parse_accessor_tail(&[0x48, 0x8D, 0x4C, 0x24, 0x70, 0xE8]), None);
        assert_eq!(parse_accessor_tail(&[0x48, 0x8D, 0x4D, 0x40]), None);
        assert_eq!(parse_accessor_tail(&[]), None);
    }

    #[test]
    fn loader_calls_parse_in_every_grammar_shape() {
        let lea_r9 = [0x4C, 0x8D, 0x4D, 0xD0];
        let idx: [(&[u8], IndexLoad); 5] = [
            (&[], IndexLoad::None),
            (&[0x45, 0x0F, 0xB7, 0xC6], IndexLoad::Movzx),
            (&[0x44, 0x0F, 0xB7, 0xC7], IndexLoad::Movzx),
            (&[0x45, 0x0F, 0xB6, 0xC6], IndexLoad::MovzxByte),
            (&[0x45, 0x8B, 0xC6], IndexLoad::Mov),
        ];
        let rdx: [&[u8]; 2] = [&[0x48, 0x8B, 0xD7], &[0x49, 0x8B, 0xD0]];
        let rcx: [&[u8]; 2] = [&[0x48, 0x8B, 0xCE], &[0x48, 0x8B, 0xCB]];
        for (i, index) in idx {
            for r in rdx {
                for c in rcx {
                    let mut b = lea_r9.to_vec();
                    b.extend_from_slice(i);
                    b.extend_from_slice(r);
                    b.extend_from_slice(c);
                    let call_at = b.len();
                    b.extend_from_slice(&[0xE8, 1, 2, 3, 4]);
                    assert_eq!(
                        parse_loader_call(&b),
                        Some(LoaderCall { index, r9_disp: -0x30, call_at }),
                        "{b:02X?}"
                    );
                    // Every truncation short of the E8 is rejected, not a panic.
                    for n in 0..=call_at {
                        assert_eq!(parse_loader_call(&b[..n]), None);
                    }
                }
            }
        }
        // An index load into some register other than r8d, the two moves the
        // other way round, and the old rsp form are not this call.
        let no = [
            &[0x4C, 0x8D, 0x4D, 0xD0, 0x45, 0x8B, 0xCE, 0x48, 0x8B, 0xD7, 0x48, 0x8B, 0xCE, 0xE8][..],
            &[0x4C, 0x8D, 0x4D, 0xD0, 0x48, 0x8B, 0xCE, 0x48, 0x8B, 0xD7, 0xE8],
            &[0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x48, 0x8D, 0x94, 0x24, 0x88, 0, 0, 0, 0x48, 0x8B, 0xCB, 0xE8],
        ];
        for b in no {
            assert_eq!(parse_loader_call(b), None, "{b:02X?}");
        }
    }

    #[test]
    fn the_26636972_fill_parses_to_the_new_layout() {
        let code = hex_bytes(FILL_26636972);
        assert_eq!(parse_stream_fill(&code, -0x30), Some(StreamLayout::NEW));
        // The r9 displacement is what the offsets are relative to.
        assert_eq!(parse_stream_fill(&code, -0x40), Some(StreamLayout { buf: 0x30, size: 0x38, cursor: 0x3C }));
        // A store below the object's start is not a field of it.
        assert_eq!(parse_stream_fill(&code, -0x0F), None);
    }

    #[test]
    fn a_fill_missing_a_store_or_with_one_twice_does_not_parse() {
        let code = hex_bytes(FILL_26636972);
        // Break the cursor load (mod 01 -> mod 00): no cursor.
        let cut = code.windows(3).position(|w| w == [0x8B, 0x42, 0x04]).unwrap();
        let mut missing = code.clone();
        missing[cut + 1] = 0x02;
        assert_eq!(parse_stream_fill(&missing, -0x30), None);
        // The size pair twice: ambiguous.
        let mut twice = code.clone();
        twice.extend_from_slice(&[0x8B, 0x43, 0x0C, 0x89, 0x45, 0xF4]);
        assert_eq!(parse_stream_fill(&twice, -0x30), None);
        // The buffer stored twice: ambiguous.
        let mut buf2 = code.clone();
        buf2.extend_from_slice(&[0x48, 0x8B, 0x03, 0x48, 0x89, 0x45, 0xE0]);
        assert_eq!(parse_stream_fill(&buf2, -0x30), None);
        // The old rsp-relative stores are not read any more.
        let old = [
            0x48, 0x8B, 0x01, 0x48, 0x89, 0x44, 0x24, 0x50, 0x8B, 0x41, 0x0C, 0x89, 0x44, 0x24, 0x58, 0x8B, 0x42,
            0x04, 0x89, 0x44, 0x24, 0x5C,
        ];
        assert_eq!(parse_stream_fill(&old, 0x30), None);
        assert_eq!(parse_stream_fill(&[], -0x30), None);
    }

    #[test]
    fn register_allocated_variants_parse() {
        // Holder in rcx, the cursor through a SIB, [rdx+r15+0x4].
        let sib = [
            0x48, 0x8B, 0x01, 0x48, 0x89, 0x45, 0xF0, 0x8B, 0x41, 0x0C, 0x89, 0x45, 0xF8, 0x44, 0x89, 0x6D, 0xFC,
            0x42, 0x8B, 0x44, 0x3A, 0x04, 0x89, 0x45, 0xFC,
        ];
        assert_eq!(parse_stream_fill(&sib, -0x30), Some(StreamLayout::NEW));
        // The size must come off the buffer's holder: off another register it
        // is a second cursor candidate, which is ambiguous.
        let mut other = sib;
        other[8] = 0x42;
        assert_eq!(parse_stream_fill(&other, -0x30), None);
    }

    #[test]
    fn the_old_layout_round_trips_through_a_synthesised_fill() {
        let mut code = vec![0x90; 4];
        code.extend_from_slice(&[0x48, 0x8B, 0x03, 0x48, 0x89, 0x45, 0xE0]);
        code.extend_from_slice(&[0x8B, 0x43, 0x0C, 0x89, 0x45, 0xE8]);
        code.extend_from_slice(&[0x8B, 0x42, 0x04, 0x89, 0x45, 0xEC]);
        assert_eq!(parse_stream_fill(&code, -0x30), Some(StreamLayout::OLD));
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
        assert!(why.contains("matches entry[idx].file_offset"), "{why}");
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
