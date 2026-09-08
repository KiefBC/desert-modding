//! Pure byte logic for the `gimmickinfo` table: record headers, resource-output
//! lists, yield multiplication, and finding the game's record loader.
//!
//! `DesertGatherer.asi` hooks the loader and rewrites the raw table bytes in
//! memory just before the game parses each record, which is what makes it a
//! drop-in replacement for the `desert-gatherer-dmm/` offset patches: same
//! edits, but computed from the bytes instead of from a build-specific offset
//! list.
//!
//! Everything here is a pure function over a byte slice, so it compiles and is
//! unit tested natively on Linux (see the cfg-gating note in README.md). It must
//! never panic on arbitrary input — the game calls it with whatever the loader
//! hands us.
//!
//! # Format (verified in Ghidra against Steam build 25116796)
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
//! The record loader is `FUN_1403856b0` (RVA `0x3856b0`). Its own prologue bytes
//! occur twice in the exe — there is a sibling template for another table — so
//! it cannot be signature-scanned directly. [`resolve_record_loader`] instead
//! goes through its caller, the accessor `FUN_140382240`, which is itself one of
//! 98 copies of a template; the copy for `gimmickinfo` is the one whose
//! `lea r8,[rip+disp]` points at the C string `"gimmickinfo\0"` (that string
//! occurs exactly once in the exe).
//!
//! ```text
//! 14038228f: 45 33 C9              xor    r9d,r9d          <- pattern A starts
//! 140382292: 4C 8D 05 9F 64 33 05  lea    r8,[0x1456b8738] ; -> "gimmickinfo"
//! 140382299: 48 8B 53 10           mov    rdx,[rbx+0x10]
//! 14038229d: 48 8D 4C 24 70        lea    rcx,[rsp+0x70]
//! 1403822a2: E8 ...                call   FUN_142508710    ; loads the file
//! ...
//! 140382304: 4C 8D 4C 24 30        lea    r9,[rsp+0x30]    <- pattern B starts
//! 140382309: 44 0F B7 C7           movzx  r8d,di
//! 14038230d: 48 8D 54 24 70        lea    rdx,[rsp+0x70]
//! 140382312: 48 8B CB              mov    rcx,rbx
//! 140382315: E8 96 33 00 00        call   FUN_1403856b0    ; the loader
//! ```

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

/// `xor r9d,r9d; lea r8,[rip+X]; mov rdx,[rbx+0x10]; lea rcx,[rsp+0x70]; call`
/// — the accessor template. 98 hits in the exe, one per table.
const PAT_A: &str = "45 33 C9 4C 8D 05 ?? ?? ?? ?? 48 8B 53 10 48 8D 4C 24 70 E8";
/// Offset of the `lea`'s disp32 within a pattern-A hit, and the length of the
/// `lea` instruction from the hit (RIP is the address of the *next* instruction).
const A_DISP_AT: usize = 6;
const A_RIP_AT: usize = 10;

/// `lea r9,[rsp+0x30]; movzx r8d,di; lea rdx,[rsp+0x70]; mov rcx,rbx; call`
/// — the call to the loader. Many hits exe-wide, so it is only searched inside
/// [`B_WINDOW`] bytes after the chosen pattern-A hit.
const PAT_B: &str = "4C 8D 4C 24 30 44 0F B7 C7 48 8D 54 24 70 48 8B CB E8";
const B_WINDOW: usize = 0x100;

/// The name the accessor for our table passes: the record loader we want is the
/// one reached through this string.
const TABLE_NAME: &[u8] = b"gimmickinfo";

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

/// Resolve the `gimmickinfo` record loader in a mapped image.
///
/// `img` is the image laid out by RVA — `MainModule::bytes()` in the game, or
/// [`crate::pe::file_to_image`] on the exe file — and `image_base` is the base
/// it is (or would be) mapped at. Returns `image_base + rva` of the loader,
/// which on build 25116796 is `image_base + 0x3856b0`.
///
/// See the module docs for why this goes through the accessor instead of
/// signature-scanning the loader directly.
pub fn resolve_record_loader(img: &[u8], image_base: usize) -> Result<usize, String> {
    let pat_a = Pattern::parse(PAT_A).ok_or("pattern A is malformed")?;
    let pat_b = Pattern::parse(PAT_B).ok_or("pattern B is malformed")?;

    // Of the ~98 copies of the accessor template, keep the ones whose
    // `lea r8,[rip+disp]` points at "gimmickinfo".
    let mut named: Vec<usize> = Vec::new();
    for a in pat_a.find_all(img, 4096) {
        let disp = match disp32_at(img, a + A_DISP_AT) {
            Some(d) => d,
            None => continue,
        };
        let Some(target) = rip_target(a + A_RIP_AT, disp) else { continue };
        if c_str_is(img, target, TABLE_NAME) {
            named.push(a);
        }
    }
    let a = match named.as_slice() {
        [one] => *one,
        [] => {
            return Err(format!(
                "no accessor (pattern A) whose lea names \"{}\" — {} template copies scanned",
                String::from_utf8_lossy(TABLE_NAME),
                pat_a.find_all(img, 4096).len()
            ))
        }
        many => {
            return Err(format!(
                "{} accessors name \"{}\", expected exactly one: {:x?}",
                many.len(),
                String::from_utf8_lossy(TABLE_NAME),
                many
            ))
        }
    };

    // The call to the loader is the pattern-B site inside this accessor.
    let win_end = a.saturating_add(B_WINDOW).min(img.len());
    let window = img.get(a..win_end).ok_or_else(|| format!("accessor at +0x{a:X} is past the image"))?;
    let hits = pat_b.find_all(window, 4);
    let b = match hits.as_slice() {
        [one] => a + *one,
        [] => {
            return Err(format!(
                "no loader call (pattern B) within 0x{B_WINDOW:X} bytes of the accessor at +0x{a:X}"
            ))
        }
        many => {
            return Err(format!(
                "{} loader calls (pattern B) within 0x{B_WINDOW:X} bytes of the accessor at +0x{a:X}",
                many.len()
            ))
        }
    };

    // The E8 is the pattern's last byte, so the rel32 follows it and the target
    // is measured from the end of the rel32.
    let rel_at = b + pat_b.len();
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

    /// An image with two copies of the accessor template — one naming
    /// "gimmickinfo", one naming "iteminfo" — and a pattern-B call in each.
    fn synthetic_image() -> Vec<u8> {
        let mut img = vec![0u8; 0x4000];
        let put = |img: &mut Vec<u8>, at: usize, b: &[u8]| {
            img[at..at + b.len()].copy_from_slice(b);
        };
        put(&mut img, 0x3000, b"gimmickinfo\0");
        put(&mut img, 0x3100, b"iteminfo\0");

        // site: pattern A at `a` pointing at `name_rva`, pattern B at a+0x40
        // calling `callee`.
        let site = |img: &mut Vec<u8>, a: usize, name_rva: usize, callee: usize| {
            let mut pa = vec![0x45, 0x33, 0xC9, 0x4C, 0x8D, 0x05];
            let disp = (name_rva as i64 - (a + A_RIP_AT) as i64) as i32;
            pa.extend_from_slice(&disp.to_le_bytes());
            pa.extend_from_slice(&[0x48, 0x8B, 0x53, 0x10, 0x48, 0x8D, 0x4C, 0x24, 0x70, 0xE8]);
            pa.extend_from_slice(&0x1122_3344u32.to_le_bytes()); // the file-load call
            put(img, a, &pa);

            let b = a + 0x40;
            let mut pb = vec![
                0x4C, 0x8D, 0x4C, 0x24, 0x30, 0x44, 0x0F, 0xB7, 0xC7, 0x48, 0x8D, 0x54, 0x24,
                0x70, 0x48, 0x8B, 0xCB, 0xE8,
            ];
            let rel = (callee as i64 - (b + pb.len() + 4) as i64) as i32;
            pb.extend_from_slice(&rel.to_le_bytes());
            put(img, b, &pb);
        };
        site(&mut img, 0x1000, 0x3000, SYN_LOADER);
        site(&mut img, 0x1800, 0x3100, 0x2800);
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
}
