//! `retarget prologue <rva>`: whole instructions from an address, and which
//! byte counts are safe to steal for a hook there.
//!
//! A hook copies the first K bytes of a function into a trampoline and
//! overwrites them with a 12-byte `mov rax,imm64; jmp rax`. K is safe when
//! it is at least [`MIN_STOLEN`](crate::MIN_STOLEN), ends on an instruction
//! boundary, and every instruction before it is position-independent: no
//! RIP-relative operand (it would point somewhere else from the trampoline)
//! and no branch (a relative one would land somewhere else, and any one would
//! leave the trampoline early). Every `*_STOLEN` constant in the workspace is
//! one of the counts this prints.

use std::io::Write;

use anyhow::Result;

use desert_tools::disasm::{self, Insn};

use crate::{hex_bytes, Exe, MIN_STOLEN};

/// The safe stolen counts in `insns`: each boundary at or past
/// [`MIN_STOLEN`] reached before the first RIP-relative or branch
/// instruction.
pub fn safe_counts(insns: &[Insn]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut at = 0usize;
    for i in insns {
        if i.is_rip_rel() || i.is_branch() || i.instr.is_invalid() {
            break;
        }
        at += i.len();
        if at >= MIN_STOLEN {
            out.push(at);
        }
    }
    out
}

/// `0x48, 0x89, 0x5C, ...`, the shape the `*_PROLOGUE` constants are written in.
fn rust_bytes(b: &[u8]) -> String {
    b.iter().map(|x| format!("0x{x:02X}")).collect::<Vec<_>>().join(", ")
}

/// Print the listing; returns `false` when `--stolen K` was asked for and K is
/// not safe.
pub fn run(
    out: &mut impl Write,
    exe: &Exe,
    rva: u32,
    bytes: usize,
    stolen: Option<usize>,
    tokens: bool,
    tsv: bool,
) -> Result<bool> {
    let insns = disasm::decode_range(&exe.image, rva, bytes.max(stolen.unwrap_or(0)));
    let safe = safe_counts(&insns);
    if tsv {
        writeln!(out, "rva\toffset\tend\tbytes\tflags\ttext")?;
        for i in &insns {
            let flags = [(i.is_rip_rel(), "RIP"), (i.is_branch(), "BR")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, f)| *f)
                .collect::<Vec<_>>()
                .join(",");
            writeln!(
                out,
                "{:#x}\t{:#x}\t{:#x}\t{}\t{flags}\t{}",
                i.rva,
                i.rva - rva,
                i.end() - rva,
                hex_bytes(&i.bytes),
                i.text()
            )?;
        }
    } else {
        writeln!(out, "prologue at +{rva:#x} ({}):", exe.pdata_status(rva))?;
        for i in &insns {
            let flags = format!(
                "{}{}",
                if i.is_rip_rel() { " RIP" } else { "" },
                if i.is_branch() { " BR" } else { "" }
            );
            writeln!(
                out,
                "  +{:<3x} ..{:<3x} {:<32} {:<8} {}",
                i.rva - rva,
                i.end() - rva,
                hex_bytes(&i.bytes),
                flags.trim(),
                i.text()
            )?;
        }
        let list: Vec<String> = safe.iter().map(|k| format!("{k:#x} ({k})")).collect();
        if list.is_empty() {
            writeln!(out, "safe stolen counts (>= {MIN_STOLEN}): none")?;
        } else {
            writeln!(out, "safe stolen counts (>= {MIN_STOLEN}): {}", list.join(", "))?;
        }
    }
    let Some(k) = stolen else { return Ok(true) };
    if !safe.contains(&k) {
        writeln!(
            out,
            "stolen {k:#x} ({k}) is NOT safe: not a whole-instruction boundary at or past {MIN_STOLEN} \
             before the first RIP-relative or branch instruction"
        )?;
        return Ok(false);
    }
    let b = exe.bytes(rva, k);
    writeln!(out, "stolen {k:#x} ({k}) is safe")?;
    if tokens {
        writeln!(out, "[")?;
        let mut at = 0usize;
        for i in &insns {
            if at >= k {
                break;
            }
            writeln!(out, "    {}, // {}", rust_bytes(&i.bytes), i.text())?;
            at += i.len();
        }
        writeln!(out, "]")?;
    } else {
        writeln!(out, "[{}]", rust_bytes(b))?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_stop_at_the_first_rip_or_branch_and_start_at_twelve() {
        // mov [rsp+0x20],rbx (5); mov [rsp+0x10],rdx (5); push rbp; push rsi;
        // push rdi; lea rax,[rip+..] (7); ...
        let code = [
            0x48u8, 0x89, 0x5C, 0x24, 0x20, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x48,
            0x8D, 0x05, 0, 0, 0, 0, 0x90,
        ];
        let insns = disasm::decode_bytes(&code, 0x1000, code.len());
        assert_eq!(safe_counts(&insns), vec![12, 13]);
        // A call before twelve leaves nothing.
        let code = [0x55u8, 0xE8, 0, 0, 0, 0, 0x48, 0x89, 0x5C, 0x24, 0x20, 0x56, 0x57, 0x90];
        assert!(safe_counts(&disasm::decode_bytes(&code, 0x1000, code.len())).is_empty());
    }
}
