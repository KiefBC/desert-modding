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
//! never panic on arbitrary input — the game calls it with whatever the loader
//! hands us.
//!
//! # Format (verified in Ghidra against Steam build 25116796, and unchanged
//! through 25246367)
//!
//! The table body is a flat sequence of records with no header at all. Each
//! record starts `u32 key`, `u32 name_len`, `name_len` ASCII bytes, `0x00`,
//! then its fields. DMM's extracted copy of the body is
//! `<DMM>/backups/gimmickinfo_pabgb_clean.bin` (22,325,302 bytes, sha256
//! `95e2dcf4b3447cdf6186b9fa816065f4e09f3ce49fe7047fd111f1bb6517411a`) and it is
//! byte-identical to what the game loads into memory.
//!
//! Inside a gather record, resource outputs are stored as one or more lists: a
//! `u32 count` followed by `count` blocks of exactly [`BLOCK`] bytes:
//!
//! ```text
//! +0    u8   flag       always 1
//! +5    u32  item       item id
//! +42   u64  min        \ vanilla 1..=8
//! +50   u64  max        /
//! +58   u16  0xFFFF     literal FF FF
//! +64   u32  item       the same item id again
//! ```
//!
//! That signature (count 1..=64, every block valid, `1 <= min <= max <=`
//! [`MAX_QTY`]) was cross-checked offline against the whole 22 MB table: inside
//! the 275 gather records the DMM pack edits it finds exactly the 587 blocks the
//! pack touches, and nothing else — no false positives.
//! `desert-gatherer-dmm/rebase.py` is the original locator this ports.
//!
//! # The loader
//!
//! The record loader is `FUN_140385cd0` (RVA `0x385cd0`). Its own prologue bytes
//! occur 263 times in the exe — every static-info table has a loader built from
//! the same template — so it cannot be signature-scanned directly.
//! [`resolve_record_loader_for`] instead goes through its caller, the accessor
//! `FUN_140382860`, which is itself one of 149 copies of another template; the
//! copy for `gimmickinfo` is the one whose `lea r8,[rip+disp]` points at the C
//! string `"gimmickinfo\0"` (that string occurs exactly once in the exe).
//!
//! Every address quoted in this module is build 25246367's, and is here to be
//! read beside a disassembly rather than relied on: all of them moved when the
//! game updated, and nothing in this file resolves anything by address.
//!
//! ```text
//! 1403828af: 45 33 C9              xor    r9d,r9d          <- the accessor site
//! 1403828b2: 4C 8D 05 4F 95 33 05  lea    r8,[0x1456bbe08] ; -> "gimmickinfo"
//! 1403828b9: 48 8B 53 10           mov    rdx,[rbx+0x10]
//! 1403828bd: 48 8D 4C 24 70        lea    rcx,[rsp+0x70]
//! 1403828c2: E8 ...                call   FUN_142509cb0    ; loads the file
//! ...
//! 140382924: 4C 8D 4C 24 30        lea    r9,[rsp+0x30]    <- pattern B starts
//! 140382929: 44 0F B7 C7           movzx  r8d,di
//! 14038292d: 48 8D 54 24 70        lea    rdx,[rsp+0x70]
//! 140382932: 48 8B CB              mov    rcx,rbx
//! 140382935: E8 96 33 00 00        call   FUN_140385cd0    ; the loader
//! ```
//!
//! # The four encodings of the accessor
//!
//! The accessor above is not one template but the **same** template in four
//! encodings, and until 2026-09-10 this module scanned for only the first of
//! them. Census over the whole exe (`ACCESSORS`), first taken on build
//! 25116796 and re-run unchanged on 25246367 — every count below, and the 149
//! they sum to, survived the game update:
//!
//! ```text
//!   xor r9d,r9d   the table name into r8              copies
//!   45 33 C9      4C 8D 05   lea r8,[rip+name]            98
//!   45 33 C9      4C 8B 05   mov r8,[rip+cell]            13
//!   45 31 C9      4C 8D 05   lea r8,[rip+name]            31
//!   45 31 C9      4C 8B 05   mov r8,[rip+cell]             7
//!                                                    --------
//!                                                        149
//! ```
//!
//! Two axes, independent of each other:
//!
//! * `45 33 C9` and `45 31 C9` are the very same instruction. `31 /r` is
//!   `xor r/m32,r32` and `33 /r` is `xor r32,r/m32`; with `r9d` on both sides
//!   they encode the identical operation, and the compiler emitted both.
//!   Nothing else about the accessor differs.
//! * `lea` names the string directly, `mov` loads a pointer **cell** whose `u64`
//!   is the string's **virtual address**. That indirection is the only reason
//!   [`resolve_manager_slot_based`] needs an `image_base` at all, and the only
//!   reason [`resolve_manager_slot`], which has none, cannot see all 149.
//!
//! 149 is **exactly** the number of static-info types
//! `docs/reference-internals.md` section 19.2 enumerates, which is what makes
//! this census provably complete: one accessor per type, no fifth encoding left
//! to find. The old `45 33 C9`-only scan reached 111 of the 149 and left 38
//! tables invisible — [`DROPSET_TABLE`], the dispatch-mission reward table,
//! among them (a `45 31 C9` + `lea` copy).
//!
//! `PAT_B`, the call to the loader, has a gap of the same shape and is handled
//! the same way: `mov rcx,rbx` is `48 8B CB` at 90 sites and `48 89 D9` at 28,
//! and `dropsetinfo` is one of the 28. The loader prologue, `PAT_SLOT` and the
//! manager layout are identical across every encoding.
//!
//! # The manager slot
//!
//! Every copy of the template loads its table's manager object into `rbx`
//! from a **global pointer slot**, by a RIP-relative `mov rbx,[rip+disp32]`
//! (`48 8B 1D ...`) a short way *before* the accessor site, immediately followed
//! by `cmp edi,[rbx+0x8]` (`3B 7B 08`) — the bounds check against the `u32`
//! record count at `manager+8`:
//!
//! ```text
//! 140382875: 48 8B 1D 8C BA 8A 06  mov    rbx,[0x146c2e308] ; the slot
//! 14038287c: 3B 7B 08              cmp    edi,[rbx+0x8]     ; count check
//! 14038287f: 0F 83 ...             jae    ...
//! ...
//! 1403828af: 45 33 C9              xor    r9d,r9d           <- accessor site
//! ```
//!
//! Surveyed over the 98 copies of the first encoding there is exactly one such
//! `mov` in the [`SLOT_WINDOW`] bytes before each accessor site; it sits 0x3A
//! before the hit in 95 of them and 0x3D in the 3 whose prologue also pushes
//! `r15`. [`resolve_manager_slot`] returns the **RVA** of that slot for a named
//! table, so Desert Looter no longer carries the two addresses as constants: on
//! build 25246367 it answers `0x6C2E2E8` for [`ITEM_TABLE`] and `0x6C2E308` for
//! [`GIMMICK_TABLE`].
//!
//! [`resolve_manager_slot_based`] is handed an image base and so reaches **any**
//! of the 149: `0x6C30308` for [`FACTION_NODE_TABLE`], `0x6C2E330` for `Skill`,
//! `0x6C328A8` for [`DROPSET_TABLE`], and the same `0x6C2E308` for
//! [`GIMMICK_TABLE`]. [`resolve_manager_slot`] is deliberately left without one
//! rather than being made a wrapper with an extra argument: Desert Looter calls
//! it on every startup and has no image base to hand at that point, and the two
//! tables it asks about are both `lea` copies.

use crate::pattern::Pattern;

/// Bytes per resource-output block.
pub const BLOCK: usize = 68;
/// Offset of the `u64` minimum inside a block.
pub const MIN_AT: usize = 42;
/// Offset of the `u64` maximum inside a block.
pub const MAX_AT: usize = 50;
/// Offset of the `u32` item id inside a block.
pub const ITEM_AT: usize = 5;
/// Offset of the block's second copy of the item id, the last four bytes of
/// the block. The signature demands the two copies agree, which is what makes
/// either of them usable as an identity check against a parsed block object
/// later (`docs/reference-internals.md` section 16: the parsed entry carries
/// this one at `entry+0x08` and the item id at `block+0x6c`).
pub const ITEM_TAIL_AT: usize = 64;

/// Largest plausible block count in one output list. Vanilla lists are far
/// smaller; the bound is what keeps the scanner from walking off a random `u32`.
pub const MAX_COUNT: u32 = 64;
/// Largest plausible yield. Vanilla min/max are 1..=8.
pub const MAX_QTY: u64 = 100_000;

/// Bytes [`crate::hook`] steals from the loader's prologue. The first 12 bytes
/// are `mov [rsp+0x20],rbx; mov [rsp+0x10],rdx; push rbp; push rsi` — four whole
/// position-independent instructions, no RIP-relative operand, no branch.
pub const LOADER_STOLEN: usize = 12;
/// The bytes those 12 must be, checked before patching (a wrong patch is a
/// crash to desktop; a refusal is just a degraded plugin).
pub const LOADER_PROLOGUE: [u8; LOADER_STOLEN] =
    [0x48, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56];

/// One encoding of the accessor template: `xor r9d,r9d`, the table name into
/// `r8`, `mov rdx,[rbx+0x10]`, `lea rcx,[rsp+0x70]`, `call`.
struct Accessor {
    /// Short name for the two bytes that vary, used in the error messages.
    label: &'static str,
    /// The template bytes, from the `xor` through the `call`'s `E8`.
    pat: &'static str,
    /// `true` when the RIP target is a pointer **cell** holding the name's
    /// virtual address (`mov r8,[rip+cell]`), `false` when the target is the
    /// name string itself (`lea r8,[rip+name]`).
    indirect: bool,
}

/// The accessor template in all four encodings the compiler emitted, with each
/// one's copy count beside it: counted on build 25116796 and re-counted
/// unchanged on 25246367. They sum to 149, one per static-info type — see the
/// module docs for why that total is what proves the census complete.
///
/// [`A_DISP_AT`] and [`A_RIP_AT`] hold for all four: the encodings differ only
/// in which `xor` opcode was used and in the opcode byte that chooses `lea`
/// over `mov`, and neither moves the disp32.
const ACCESSORS: [Accessor; 4] = [
    Accessor {
        label: "45 33 C9 + lea r8",
        pat: "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8",
        indirect: false,
    }, // 98 copies
    Accessor {
        label: "45 33 C9 + mov r8",
        pat: "45 33 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8",
        indirect: true,
    }, // 13 copies
    Accessor {
        label: "45 31 C9 + lea r8",
        pat: "45 31 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8",
        indirect: false,
    }, // 31 copies
    Accessor {
        label: "45 31 C9 + mov r8",
        pat: "45 31 C9 4C 8B 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8",
        indirect: true,
    }, // 7 copies
];
/// Offset of the disp32 within an accessor hit, and the length of the
/// name-loading instruction from the hit (RIP is the address of the *next*
/// instruction).
const A_DISP_AT: usize = 6;
const A_RIP_AT: usize = 10;
/// Cap on the accessor hits collected per encoding. The largest real count is
/// 98; a scan that hits this cap has stopped being a census and the error
/// messages say so by printing the counts.
const MAX_SITES: usize = 4096;

/// `lea r9,[rsp+0x30]; movzx r8d,di; lea rdx,[rsp+0x70]; mov rcx,rbx; call`
/// — the call to the loader, in the two encodings of `mov rcx,rbx`: `48 8B CB`
/// at 90 sites and `48 89 D9` at 28. Both are 18 bytes and both end in the
/// `E8`, so the rel32 follows either at the same distance from its own hit.
/// Many hits exe-wide, so both are only searched inside [`B_WINDOW`] bytes
/// after the chosen accessor site.
const PAT_B: [&str; 2] = [
    "4C 8D 4C 24 30 44 0F B7 C7 48 8D 54 24 70 48 8B CB E8",
    "4C 8D 4C 24 30 44 0F B7 C7 48 8D 54 24 70 48 89 D9 E8",
];
const B_WINDOW: usize = 0x100;

/// `mov rbx,[rip+disp32]` — the load of a table's manager object out of its
/// global pointer slot. Seven bytes: opcode at +0, disp32 at [`SLOT_DISP_AT`],
/// RIP (the next instruction) at [`SLOT_RIP_AT`].
const PAT_SLOT: &str = "48 8B 1D ?? ?? ?? ??";
const SLOT_DISP_AT: usize = 3;
const SLOT_RIP_AT: usize = 7;
/// `cmp edi,[rbx+0x8]` — the record-count bounds check that follows the real
/// slot load. Used only to break a tie between several `mov rbx,[rip+...]`.
const SLOT_CMP: [u8; 3] = [0x3B, 0x7B, 0x08];
/// How far back from a pattern-A hit the slot load is looked for. The real
/// distance is 0x3A (0x3D in three copies); the window is generous.
pub const SLOT_WINDOW: usize = 0x60;

/// The name the `gimmickinfo` accessor passes: the record loader Desert
/// Gatherer hooks is the one reached through this string.
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

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o.checked_add(4)?)?.try_into().ok().map(u32::from_le_bytes)
}

fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o.checked_add(8)?)?.try_into().ok().map(u64::from_le_bytes)
}

/// The fixed part at the front of every record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordHeader {
    pub key: u32,
    pub name: String,
    /// Offset of the first byte after the NUL, i.e. where the fields start.
    pub body: usize,
}

/// Parse a record header from the start of `rec`.
///
/// `None` unless the name length is 1..=255, every name byte is printable
/// ASCII, and the NUL terminator is present within `rec`. Those three checks are
/// what make this usable as a record *detector* on a raw table scan.
pub fn parse_header(rec: &[u8]) -> Option<RecordHeader> {
    let key = u32_at(rec, 0)?;
    let len = u32_at(rec, 4)? as usize;
    if !(1..=255).contains(&len) {
        return None;
    }
    let name = rec.get(8..8usize.checked_add(len)?)?;
    if !name.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        return None;
    }
    let nul = 8 + len;
    if *rec.get(nul)? != 0 {
        return None;
    }
    Some(RecordHeader {
        key,
        name: String::from_utf8_lossy(name).into_owned(),
        body: nul + 1,
    })
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
    if b[5..9] != b[64..68] {
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
/// Scans byte by byte — the lists are not at a fixed place in a record and the
/// record's other fields are not parsed — and skips past a whole list once one
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
    /// The block's item id, taken from [`ITEM_TAIL_AT`].
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
/// reports what is there instead of what to write. Desert Gatherer uses it to
/// remember a record's vanilla yields at load time, because once the game has
/// parsed a multiplied record the original numbers are gone — the parsed
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
/// The `old` field is not re-verified here — [`multiply`] read it from these
/// same bytes — so an edit list must not be applied twice.
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
/// RIP target is a pointer cell: the cell holds a *virtual address* — at runtime
/// the loader has relocated it against the real module base, and in a
/// file-backed image it still carries the preferred base — so the name's RVA is
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

/// Every accessor site, across every encoding in [`ACCESSORS`], whose name
/// resolves to the C string `table`, in ascending address order.
///
/// With `image_base` of `None` the two indirect encodings are skipped entirely:
/// their name is only reachable through a VA, and a caller without a base
/// cannot turn one back into an RVA. Most sites name some other table and are
/// simply not collected — and so is one whose cell holds something that is not
/// a VA inside this image at all. Nothing here is an error, because the
/// caller's "exactly one match" is the real check.
fn accessors_naming(
    img: &[u8],
    image_base: Option<usize>,
    table: &[u8],
) -> Result<Vec<usize>, String> {
    let mut named = Vec::new();
    for t in ACCESSORS.iter().filter(|t| !t.indirect || image_base.is_some()) {
        let pat = Pattern::parse(t.pat)
            .ok_or_else(|| format!("the accessor pattern \"{}\" is malformed", t.label))?;
        for a in pat.find_all(img, MAX_SITES) {
            if accessor_name_rva(img, a, t, image_base)
                .is_some_and(|rva| c_str_is(img, rva, table))
            {
                named.push(a);
            }
        }
    }
    named.sort_unstable();
    named.dedup();
    Ok(named)
}

/// How many copies of each encoding are in `img`, as
/// `"45 33 C9 + lea r8=98, ..."`, for the resolvers' error messages. A count
/// that has drifted from the census in the module docs is the first thing to
/// look at after a game update.
fn accessor_census(img: &[u8], image_base: Option<usize>) -> String {
    ACCESSORS
        .iter()
        .filter(|t| !t.indirect || image_base.is_some())
        .map(|t| {
            let n = Pattern::parse(t.pat).map_or(0, |p| p.find_all(img, MAX_SITES).len());
            format!("{}={n}", t.label)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Offset of the one accessor site that names `table`.
///
/// Every table-specific resolver in this module starts here: the accessor is
/// what ties a table's name to its code and its data. `image_base` of `None`
/// restricts the search to the two `lea` encodings — see [`accessors_naming`].
fn named_accessor(
    img: &[u8],
    image_base: Option<usize>,
    table: &[u8],
) -> Result<usize, String> {
    let named = accessors_naming(img, image_base, table)?;
    // Which half of the census was in scope, so an error says what was and was
    // not searched rather than only how much of it there was.
    let scope = if image_base.is_some() { "any encoding" } else { "the lea encodings" };
    match named.as_slice() {
        [one] => Ok(*one),
        [] => Err(format!(
            "no accessor of {scope} names \"{}\" — template copies scanned: {}",
            String::from_utf8_lossy(table),
            accessor_census(img, image_base)
        )),
        many => Err(format!(
            "{} accessors of {scope} name \"{}\", expected exactly one: {:x?}",
            many.len(),
            String::from_utf8_lossy(table),
            many
        )),
    }
}

/// Resolve the **RVA** of the global pointer slot holding the manager object
/// for the table named `table` (e.g. [`ITEM_TABLE`], [`GIMMICK_TABLE`]).
///
/// `img` is the image laid out by RVA — `MainModule::bytes()` in the game, or
/// [`crate::pe::file_to_image`] on the exe file. The result is an RVA, not a
/// virtual address: callers add the module base themselves. On build 25246367
/// this answers `0x6C2E2E8` for `iteminfo` and `0x6C2E308` for `gimmickinfo`.
///
/// See the module docs for the `mov rbx,[rip+disp32]; cmp edi,[rbx+0x8]` shape
/// this looks for in the [`SLOT_WINDOW`] bytes before the named accessor.
pub fn resolve_manager_slot(img: &[u8], table: &[u8]) -> Result<usize, String> {
    slot_before_accessor(img, named_accessor(img, None, table)?)
}

/// The **RVA** of the global pointer slot holding the manager object for the
/// table named `table`, resolved through **any** of the four accessor
/// encodings.
///
/// Same answer as [`resolve_manager_slot`] for every table a `lea` encoding
/// names, and the only way to reach the 20 tables named through a pointer cell
/// — `FactionNode` and `Skill` among them. `image_base` is needed solely
/// because of that indirection: the cell's `u64` is the name string's **virtual
/// address**, so the base the image is laid out for is what turns it back into
/// an RVA. Pass the running module's base in the game (`MainModule::base`) and
/// `pe::Headers::image_base` for a file-backed image.
///
/// On build 25246367 this answers `0x6C30308` for [`FACTION_NODE_TABLE`],
/// `0x6C2E330` for `Skill`, `0x6C328A8` for [`DROPSET_TABLE`] and `0x6C2E308`
/// for [`GIMMICK_TABLE`].
///
/// [`resolve_manager_slot`] is deliberately left alone rather than being made a
/// wrapper with an extra argument: Desert Looter calls it on every startup and
/// has no image base to hand at that point.
pub fn resolve_manager_slot_based(
    img: &[u8],
    image_base: usize,
    table: &[u8],
) -> Result<usize, String> {
    slot_before_accessor(img, named_accessor(img, Some(image_base), table)?)
}

/// The `mov rbx,[rip+disp32]; cmp edi,[rbx+0x8]` that precedes the accessor
/// site `a`, as the **RVA** of the slot it loads. Shared by both resolvers:
/// the two templates differ in how they name the table and in nothing else, so
/// the slot search is identical once the site is known.
fn slot_before_accessor(img: &[u8], a: usize) -> Result<usize, String> {
    let pat_slot = Pattern::parse(PAT_SLOT).ok_or("the slot pattern is malformed")?;

    // Only the run-up to the accessor's pattern-A site is searched: the `mov`
    // is a few dozen bytes above it and the opcode is common exe-wide.
    let win_start = a.saturating_sub(SLOT_WINDOW);
    let window = img
        .get(win_start..a)
        .ok_or_else(|| format!("accessor at +0x{a:X} is past the image"))?;
    let hits: Vec<usize> = pat_slot.find_all(window, 64).iter().map(|h| win_start + h).collect();

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
            let checked: Vec<usize> = many
                .iter()
                .copied()
                .filter(|h| {
                    h.checked_add(SLOT_RIP_AT)
                        .and_then(|e| img.get(e..e.checked_add(SLOT_CMP.len())?))
                        == Some(&SLOT_CMP[..])
                })
                .collect();
            match checked.as_slice() {
                [one] => *one,
                _ => {
                    return Err(format!(
                        "{} mov rbx,[rip+disp] within 0x{SLOT_WINDOW:X} bytes before the accessor \
                         at +0x{a:X}, {} of them followed by cmp edi,[rbx+8]: {:x?}",
                        many.len(),
                        checked.len(),
                        many
                    ))
                }
            }
        }
    };

    let disp = disp32_at(img, hit + SLOT_DISP_AT)
        .ok_or_else(|| format!("slot load at +0x{hit:X} has no disp32 inside the image"))?;
    rip_target(hit + SLOT_RIP_AT, disp)
        .filter(|&t| t < img.len())
        .ok_or_else(|| format!("slot load at +0x{hit:X} targets +0x{disp:X} outside the image"))
}

/// Resolve the `gimmickinfo` record loader in a mapped image.
///
/// `img` is the image laid out by RVA — `MainModule::bytes()` in the game, or
/// [`crate::pe::file_to_image`] on the exe file — and `image_base` is the base
/// it is (or would be) mapped at. Returns `image_base + rva` of the loader,
/// which on build 25246367 is `image_base + 0x385cd0`.
///
/// This is what Desert Gatherer hooks, and the table it wants is the only one
/// it ever wants, so the name stays out of its call. [`resolve_record_loader_for`]
/// is the same thing for any other table.
pub fn resolve_record_loader(img: &[u8], image_base: usize) -> Result<usize, String> {
    resolve_record_loader_for(img, image_base, GIMMICK_TABLE)
}

/// Resolve the record loader for the table named `table`, as `image_base + rva`.
///
/// Every static-info table is loaded by its own copy of one loader template,
/// reached through that table's accessor, so this is [`resolve_record_loader`]
/// with the name spelled out: `0x385cd0` for [`GIMMICK_TABLE`], `0x437E70` for
/// [`DROPSET_TABLE`], `0x3C1F30` for [`FACTION_NODE_TABLE`] on build 25246367.
/// Whichever it is, its first [`LOADER_STOLEN`] bytes are [`LOADER_PROLOGUE`] —
/// which a caller about to hook it must check for itself before patching.
///
/// See the module docs for why this goes through the accessor instead of
/// signature-scanning the loader directly: the prologue occurs 263 times.
pub fn resolve_record_loader_for(
    img: &[u8],
    image_base: usize,
    table: &[u8],
) -> Result<usize, String> {
    let a = named_accessor(img, Some(image_base), table)?;

    // The call to the loader is the pattern-B site inside this accessor, in
    // whichever of its two encodings this copy was assembled with. Both are
    // searched: a site that matched one cannot also match the other, so a
    // second hit is a real ambiguity and not a double count.
    let win_end = a.saturating_add(B_WINDOW).min(img.len());
    let window =
        img.get(a..win_end).ok_or_else(|| format!("accessor at +0x{a:X} is past the image"))?;
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for text in PAT_B {
        let pat = Pattern::parse(text).ok_or("a loader-call pattern is malformed")?;
        hits.extend(pat.find_all(window, 4).into_iter().map(|h| (a + h, pat.len())));
    }
    hits.sort_unstable();
    let (b, b_len) = match hits.as_slice() {
        [one] => *one,
        [] => {
            return Err(format!(
                "no loader call (pattern B, either encoding) within 0x{B_WINDOW:X} bytes of the \
                 accessor at +0x{a:X}"
            ))
        }
        many => {
            return Err(format!(
                "{} loader calls (pattern B) within 0x{B_WINDOW:X} bytes of the accessor at \
                 +0x{a:X}: {:x?}",
                many.len(),
                many.iter().map(|&(h, _)| h).collect::<Vec<_>>()
            ))
        }
    };

    // The E8 is the pattern's last byte, so the rel32 follows it and the target
    // is measured from the end of the rel32.
    let rel_at = b + b_len;
    let rel = disp32_at(img, rel_at)
        .ok_or_else(|| format!("loader call at +0x{b:X} has no rel32 inside the image"))?;
    let end_of_rel32 = rel_at + 4;
    let rva = rip_target(end_of_rel32, rel)
        .filter(|&t| t < img.len())
        .ok_or_else(|| format!("loader call at +0x{b:X} targets +0x{rel:X} outside the image"))?;

    image_base
        .checked_add(rva)
        .ok_or_else(|| format!("image base 0x{image_base:X} + rva 0x{rva:X} overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A block whose fields all check out.
    fn block(item: u32, min: u64, max: u64) -> Vec<u8> {
        let mut b = vec![0u8; BLOCK];
        b[0] = 1;
        b[5..9].copy_from_slice(&item.to_le_bytes());
        b[MIN_AT..MIN_AT + 8].copy_from_slice(&min.to_le_bytes());
        b[MAX_AT..MAX_AT + 8].copy_from_slice(&max.to_le_bytes());
        b[58] = 0xFF;
        b[59] = 0xFF;
        b[64..68].copy_from_slice(&item.to_le_bytes());
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
        b[64] = 0x0A; // trailing item id mismatch
        assert!(output_lists(&good(&[b], 1)).is_empty());

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

    /// `mov rbx,[rip+disp32]` at `at` targeting `slot_rva`, optionally followed
    /// by the `cmp edi,[rbx+0x8]` bounds check.
    fn put_slot_load(img: &mut [u8], at: usize, slot_rva: usize, with_cmp: bool) {
        let mut b = vec![0x48, 0x8B, 0x1D];
        let disp = (slot_rva as i64 - (at + SLOT_RIP_AT) as i64) as i32;
        b.extend_from_slice(&disp.to_le_bytes());
        if with_cmp {
            b.extend_from_slice(&SLOT_CMP);
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

    /// An image with two copies of the accessor template — one naming
    /// "gimmickinfo", one naming "iteminfo" — a pattern-B call in each, and the
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
        // The decoy cell names "iteminfo" — and so does a *direct* accessor
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
        img[cmp_at..cmp_at + SLOT_CMP.len()].fill(0x90);
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
    /// exactly the contract [`resolve_manager_slot`] has with Desert Looter.
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
    /// different encodings — the point of scanning both is to find the one
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
}
