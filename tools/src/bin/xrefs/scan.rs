//! The two scanners: the fast one the tool uses, and the obvious one it is
//! checked against.
//!
//! They are in the same file on purpose. `--selfcheck` only means anything
//! while the reference implementation is visibly, boringly correct and sitting
//! next to the clever one - the moment it moves somewhere it can rot, it stops
//! being an oracle and starts being a second thing to trust.

use memchr::{memchr2_iter, memmem};
use rayon::prelude::*;

/// modrm bytes with mod=00, rm=101 (RIP-relative), any reg field.
pub const MODRM: [u8; 8] = [0x05, 0x0D, 0x15, 0x1D, 0x25, 0x2D, 0x35, 0x3D];
/// The REX prefixes that appear in front of these forms in this exe.
pub const REX: [u8; 4] = [0x48, 0x4C, 0x49, 0x4D];

/// `(opcode, name with a REX prefix, name without one)`.
///
/// The two names are reported separately because they are different
/// instructions: `48 8D 05 ..` is a 64-bit `lea` and `8D 05 ..` is the 32-bit
/// one, and which of them loads a table pointer is worth being able to see at a
/// glance.
pub const OPS: [(u8, &str, &str); 3] = [
    (0x8D, "lea", "lea32"),
    (0x8B, "mov r,[rip]", "mov32 r,[rip]"),
    (0x89, "mov [rip],r", "mov32 [rip],r"),
];

fn disp32(img: &[u8], at: usize) -> Option<i32> {
    let b = img.get(at..at + 4)?;
    Some(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Every offset of `pat` in `hay`, OVERLAPPING matches included.
///
/// `memmem::find_iter` skips past a match before resuming, which is wrong for
/// the pointer-cell scan: a run of repeated pointer bytes can hold the same
/// eight-byte value at two offsets one byte apart, and the Python this was
/// ported from stepped by one (`img.find(pat, i + 1)`).
pub fn find_all(hay: &[u8], pat: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = memmem::find(&hay[from..], pat) {
        let at = from + rel;
        out.push(at);
        from = at + 1;
    }
    out
}

/// Find every `E8`/`E9` rel32 and RIP-relative `lea`/`mov` that reaches
/// `target`, as `(rva, what it is)` sorted and deduplicated.
///
/// The image is 385 MB, so nothing here walks it a byte at a time in the host
/// language. Two shapes of pass do the work:
///
///   * one `memchr2` over the whole image picks out the `E8`/`E9` candidates,
///     and only the rel32 arithmetic runs per candidate;
///   * ONE set of 24 `(opcode, modrm)` patterns covers both the REX-prefixed
///     and the un-prefixed forms, rather than 24 + 96. A REX byte only shifts
///     where the instruction STARTS: `REX op modrm disp32` found at the opcode
///     still reads its displacement at +2 and still ends at +6, exactly as the
///     no-REX form does. That is worth knowing before "fixing" this into four
///     REX loops - the Python had four loops first, and they cost 10.4 s of a
///     15.8 s scan for nothing.
pub fn fast_scan(img: &[u8], target: u64) -> Vec<(usize, &'static str)> {
    let target = target as i64;
    let mut hits: Vec<(usize, &'static str)> = Vec::new();

    for i in memchr2_iter(0xE8, 0xE9, img) {
        let Some(rel) = disp32(img, i + 1) else { continue };
        if i as i64 + 5 + rel as i64 == target {
            hits.push((i, if img[i] == 0xE8 { "call" } else { "jmp" }));
        }
    }

    let pairs: Vec<(u8, u8, &'static str, &'static str)> = OPS
        .iter()
        .flat_map(|&(op, rex_name, name32)| {
            MODRM.iter().map(move |&mr| (op, mr, rex_name, name32))
        })
        .collect();
    let found: Vec<Vec<(usize, &'static str)>> = pairs
        .par_iter()
        .map(|&(op, mr, rex_name, name32)| {
            let mut out = Vec::new();
            for i in find_all(img, &[op, mr]) {
                let Some(rel) = disp32(img, i + 2) else { continue };
                if i as i64 + 6 + rel as i64 != target {
                    continue;
                }
                out.push((i, name32));
                // The un-prefixed form is always reported; the 64-bit one is
                // reported as well when a REX byte sits in front of it. Both,
                // not either - a REX byte is also a perfectly ordinary opcode
                // and there is no way to tell from here which reading the
                // decoder took, so the scan reports what it can see and lets
                // the reader disassemble the address.
                if i > 0 && REX.contains(&img[i - 1]) {
                    out.push((i - 1, rex_name));
                }
            }
            out
        })
        .collect();
    hits.extend(found.into_iter().flatten());

    hits.sort_unstable();
    hits.dedup();
    hits
}

/// Eight bytes anywhere in the image holding the target's VA.
///
/// This half is not a nicety. A target with NO code xref at all is normal in
/// this exe: the indirect accessor encoding reaches a table name through a
/// pointer cell holding its VA (`desert-core`'s `gimmick::ACCESSORS`,
/// `docs/reference-internals.md` section 19), and whole families of class
/// names live only in pointer arrays. `SetAdditionalCollectDropRate` is the
/// worked example in `tools/README.md` - zero code references, one pointer
/// cell at `+0x56AF6E8`, entry 195 of a 208-name array, and the only thing
/// that identifies it at all. A scan that printed "0 references" and stopped
/// there is how that lead stayed unexplored for a day.
pub fn pointer_cells(img: &[u8], va: u64) -> Vec<usize> {
    find_all(img, &va.to_le_bytes())
}

/// The byte-at-a-time scan this tool used until 2026-09-13, kept as the oracle
/// for `--selfcheck`.
///
/// It is several times slower and is not used for anything else; its only job
/// is to be obviously correct so the fast path can be diffed against it. Do not
/// optimise it. If it ever needs to be fast, it has stopped being useful.
pub fn reference_scan(img: &[u8], target: u64) -> Vec<(usize, &'static str)> {
    let target = target as i64;
    let mut hits: Vec<(usize, &'static str)> = Vec::new();
    let n = img.len();
    // `n - 7`, matching the Python's `while i < n - 7`, and NOT `n - 5`: the
    // last seven bytes of the image go unscanned by both. They are tail
    // zero-padding past the final section, so nothing has ever been missed
    // there, but the fast scan does look at them - see the tests.
    let mut i = 0usize;
    while i + 7 < n {
        let b = img[i];
        if b == 0xE8 || b == 0xE9 {
            if i as i64 + 5 + disp32(img, i + 1).unwrap_or(0) as i64 == target {
                hits.push((i, if b == 0xE8 { "call" } else { "jmp" }));
            }
        } else if REX.contains(&b) && (img[i + 2] & 0xC7) == 0x05 {
            if let Some(&(_, rex_name, _)) = OPS.iter().find(|&&(op, _, _)| op == img[i + 1]) {
                if i as i64 + 7 + disp32(img, i + 3).unwrap_or(0) as i64 == target {
                    hits.push((i, rex_name));
                }
            }
        } else if (img[i + 1] & 0xC7) == 0x05 {
            if let Some(&(_, _, name32)) = OPS.iter().find(|&&(op, _, _)| op == b) {
                if i as i64 + 6 + disp32(img, i + 2).unwrap_or(0) as i64 == target {
                    hits.push((i, name32));
                }
            }
        }
        i += 1;
    }
    hits.sort_unstable();
    hits.dedup();
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A buffer with `bytes` planted at `at` and zero everywhere else, long
    /// enough that both scanners look at all of it.
    fn planted(at: usize, bytes: &[u8]) -> Vec<u8> {
        let mut v = vec![0u8; at + bytes.len() + 64];
        v[at..at + bytes.len()].copy_from_slice(bytes);
        v
    }

    /// The rel32 an instruction ENDING at `end` needs to reach `target`.
    fn rel(end: i64, target: i64) -> [u8; 4] {
        ((target - end) as i32).to_le_bytes()
    }

    #[test]
    fn e8_call_rel32_is_measured_from_the_end_of_the_instruction() {
        // The off-by-one that hides: rel32 is relative to the NEXT
        // instruction, i.e. the call's offset plus five, not plus one.
        let target = 0x200i64;
        let mut ins = vec![0xE8u8];
        ins.extend_from_slice(&rel(0x100 + 5, target));
        let img = planted(0x100, &ins);
        assert_eq!(fast_scan(&img, target as u64), vec![(0x100, "call")]);
        assert_eq!(reference_scan(&img, target as u64), vec![(0x100, "call")]);

        // One byte off in either direction is not a hit.
        assert!(fast_scan(&img, target as u64 + 1).is_empty());
        assert!(fast_scan(&img, target as u64 - 1).is_empty());
    }

    #[test]
    fn e9_is_a_jmp_and_a_negative_rel32_reaches_backwards() {
        let target = 0x40i64;
        let mut ins = vec![0xE9u8];
        ins.extend_from_slice(&rel(0x100 + 5, target));
        let img = planted(0x100, &ins);
        assert_eq!(fast_scan(&img, target as u64), vec![(0x100, "jmp")]);
        assert_eq!(reference_scan(&img, target as u64), fast_scan(&img, target as u64));
    }

    #[test]
    fn rip_relative_lea_is_measured_from_the_end_too_and_rex_shifts_only_the_start() {
        // `48 8D 05 disp32` at 0x100: the instruction is seven bytes, so the
        // displacement is relative to 0x107. The opcode is at 0x101, and the
        // fast scan finds it THERE - which is why its arithmetic is +6 from
        // the opcode and the byte loop's is +7 from the REX byte. Both must
        // land on the same target, and both forms must be reported.
        let target = 0x900i64;
        let mut ins = vec![0x48u8, 0x8D, 0x05];
        ins.extend_from_slice(&rel(0x100 + 7, target));
        let img = planted(0x100, &ins);

        let want = vec![(0x100, "lea"), (0x101, "lea32")];
        assert_eq!(fast_scan(&img, target as u64), want);
        assert_eq!(reference_scan(&img, target as u64), want);
    }

    #[test]
    fn an_unprefixed_form_is_reported_once() {
        let target = 0x900i64;
        let mut ins = vec![0x8Du8, 0x0D];
        ins.extend_from_slice(&rel(0x100 + 6, target));
        // The byte in front is deliberately NOT a REX prefix.
        let mut img = planted(0x100, &ins);
        img[0xFF] = 0x90;
        assert_eq!(fast_scan(&img, target as u64), vec![(0x100, "lea32")]);
        assert_eq!(reference_scan(&img, target as u64), vec![(0x100, "lea32")]);
    }

    #[test]
    fn every_opcode_and_every_rip_relative_modrm_is_covered() {
        // 3 opcodes x 8 modrm reg fields. A scanner that only knew `05` would
        // pass every other test in this file and miss five sixths of the real
        // hits.
        let target = 0x4000i64;
        for &(op, rex_name, name32) in &OPS {
            for &mr in &MODRM {
                let mut ins = vec![0x4Cu8, op, mr];
                ins.extend_from_slice(&rel(0x100 + 7, target));
                let img = planted(0x100, &ins);
                let want = vec![(0x100, rex_name), (0x101, name32)];
                assert_eq!(fast_scan(&img, target as u64), want, "op {op:#x} modrm {mr:#x}");
                assert_eq!(reference_scan(&img, target as u64), want, "op {op:#x} modrm {mr:#x}");
            }
        }
    }

    #[test]
    fn a_modrm_that_is_not_rip_relative_is_not_a_hit() {
        // mod=01 rm=101 is `[rbp+disp8]`, not `[rip+disp32]`. Masking with
        // 0xC7 is what separates them; dropping the mask matches both.
        let target = 0x900i64;
        let mut ins = vec![0x48u8, 0x8D, 0x45];
        ins.extend_from_slice(&rel(0x100 + 7, target));
        let img = planted(0x100, &ins);
        assert!(fast_scan(&img, target as u64).is_empty());
        assert!(reference_scan(&img, target as u64).is_empty());
    }

    #[test]
    fn overlapping_pointer_cells_are_all_found() {
        // Eight bytes of 0x11 repeated: the same value sits at three offsets,
        // one byte apart. `memmem::find_iter` reports one of them.
        let img = vec![0x11u8; 10];
        assert_eq!(pointer_cells(&img, 0x1111_1111_1111_1111), vec![0, 1, 2]);
    }

    #[test]
    fn a_pointer_cell_holds_the_va_not_the_rva() {
        let va = 0x1_4569_C7C0u64;
        let mut img = vec![0u8; 64];
        img[16..24].copy_from_slice(&va.to_le_bytes());
        assert_eq!(pointer_cells(&img, va), vec![16]);
        // The RVA alone must not match, or every report would be doubled.
        assert!(pointer_cells(&img, 0x569_C7C0).is_empty());
    }

    #[test]
    fn the_two_scanners_agree_on_a_buffer_full_of_near_misses() {
        // Pseudo-random bytes drawn from the alphabet that actually triggers
        // the scanners, so the buffer is dense with candidates that mostly do
        // not resolve to the target. This is the shape of test that catches a
        // branch ordering difference between the two implementations.
        let alphabet = [0xE8u8, 0xE9, 0x48, 0x4C, 0x49, 0x4D, 0x8D, 0x8B, 0x89, 0x05, 0x0D, 0x00, 0x01, 0xFF];
        let mut img = Vec::with_capacity(8192);
        let mut x: u32 = 0x1234_5678;
        for _ in 0..8192 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            img.push(alphabet[(x >> 16) as usize % alphabet.len()]);
        }
        for target in [0u64, 0x100, 0x1000, 0x1234, 0x2000] {
            assert_eq!(
                fast_scan(&img, target),
                reference_scan(&img, target),
                "target {target:#x}"
            );
        }
    }

    #[test]
    fn the_byte_loop_does_not_look_at_the_last_seven_bytes() {
        // A real, deliberate difference between the two, inherited from the
        // Python's `while i < n - 7`. In the game image the tail is zero
        // padding past the final section, so `--selfcheck` has never tripped
        // on it - but a fixture can, and a future reader deserves to find this
        // written down rather than as a mysterious selfcheck failure.
        let target = 0x10i64;
        let mut img = vec![0u8; 0x100];
        let at = img.len() - 5; // a call whose last byte is the last byte
        img[at] = 0xE8;
        img[at + 1..at + 5].copy_from_slice(&rel(at as i64 + 5, target));
        assert_eq!(fast_scan(&img, target as u64), vec![(at, "call")]);
        assert!(reference_scan(&img, target as u64).is_empty());
    }
}
