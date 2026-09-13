#!/usr/bin/env python3
"""Find references to an RVA in CrimsonDesert.exe without a disassembler.

Usage: tools/xrefs.py <rva-hex> [exe] [--selfcheck]

Reports three kinds of reference, all against the file laid out as an image, so
every offset printed is an RVA:

  * `E8` call / `E9` jmp rel32 whose target is the RVA;
  * RIP-relative `lea` / `mov` (REX and no-REX, modrm mod=00 rm=101);
  * **pointer cells** - eight bytes somewhere in the image holding the RVA's
    *VA*. These are how the indirect accessor encoding reaches a table name
    (`desert-core`'s `gimmick::ACCESSORS`, `docs/reference-internals.md`
    section 19), and a target with no code xref at all often has several. A
    scan that reports "no references" while a pointer array names the address
    is the failure mode this half exists to prevent.

The scan is `bytes.find` per opcode form rather than a Python loop over every
byte of the 363 MB image, which is the difference between seconds and minutes.
Coverage is identical to the byte-loop it replaced; `xrefs_agree_with_the_byte
_loop` in the commit that introduced it is how that was checked.
"""
import re
import struct
import sys

DEFAULT_EXE = "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe"
# Positional args only: a flag must never be mistaken for the exe path.
ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
EXE = ARGS[1] if len(ARGS) > 1 else DEFAULT_EXE

# modrm bytes with mod=00, rm=101 (RIP-relative), any reg field.
MODRM = (0x05, 0x0D, 0x15, 0x1D, 0x25, 0x2D, 0x35, 0x3D)
REX = (0x48, 0x4C, 0x49, 0x4D)
OPS = {0x8D: "lea", 0x8B: "mov r,[rip]", 0x89: "mov [rip],r"}
OPS32 = {0x8D: "lea32", 0x8B: "mov32 r,[rip]", 0x89: "mov32 [rip],r"}


def file_to_image(d):
    """Lay the PE out as it would be mapped, and return it with its image base."""
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    nsec = struct.unpack_from("<H", d, pe + 6)[0]
    opt = pe + 24
    optsz = struct.unpack_from("<H", d, pe + 20)[0]
    size = struct.unpack_from("<I", d, opt + 56)[0]
    base = struct.unpack_from("<Q", d, opt + 24)[0]
    img = bytearray(size)
    s = opt + optsz
    hdr = min(struct.unpack_from("<I", d, s + i * 40 + 20)[0] for i in range(nsec))
    img[:hdr] = d[:hdr]
    for i in range(nsec):
        vs, va, rs, ro = struct.unpack_from("<IIII", d, s + i * 40 + 8)
        n = min(rs, len(d) - ro)
        img[va:va + n] = d[ro:ro + n]
    return bytes(img), base


def find_all(img, pat, start=0):
    """Every offset of `pat` in `img`, overlapping matches included."""
    out = []
    i = img.find(pat, start)
    while i != -1:
        out.append(i)
        i = img.find(pat, i + 1)
    return out


def scan(img, base, target):
    n = len(img)
    hits = []

    # E8/E9 rel32. One C-level pass picks the candidate opcodes; the arithmetic
    # is the only part that runs in Python.
    for m in re.finditer(rb"[\xe8\xe9]", img):
        i = m.start()
        if i + 5 > n:
            continue
        rel = struct.unpack_from("<i", img, i + 1)[0]
        if i + 5 + rel == target:
            hits.append((i, "call" if img[i] == 0xE8 else "jmp"))

    # RIP-relative lea/mov, REX-prefixed and not. Both are found with ONE set of
    # 24 (op, modrm) patterns rather than 24 + 96: a REX byte only shifts the
    # instruction's start, so `REX op modrm disp32` found at the op still reads
    # its displacement at +2 and still ends at +6, exactly as the no-REX form
    # does. That is worth knowing before "fixing" this into four REX loops - it
    # was four loops first, and they cost 10.4 s of a 15.8 s scan for nothing.
    for op, name in OPS.items():
        for mr in MODRM:
            for i in find_all(img, bytes((op, mr))):
                if i + 6 > n or i + 6 + struct.unpack_from("<i", img, i + 2)[0] != target:
                    continue
                hits.append((i, OPS32[op]))
                if i and img[i - 1] in REX:
                    hits.append((i - 1, name))

    cells = find_all(img, struct.pack("<Q", base + target))
    return sorted(set(hits)), cells


def reference_scan(img, target):
    """The byte-at-a-time scan this tool used until 2026-09-13, kept as the
    oracle for `--selfcheck`. It is ~8x slower and is not used for anything
    else; its only job is to be obviously correct so the fast path can be
    diffed against it."""
    n = len(img)
    hits = []
    i = 0
    while i < n - 7:
        b = img[i]
        if b in (0xE8, 0xE9):
            if i + 5 + struct.unpack_from("<i", img, i + 1)[0] == target:
                hits.append((i, "call" if b == 0xE8 else "jmp"))
        elif b in REX and img[i + 1] in OPS and (img[i + 2] & 0xC7) == 0x05:
            if i + 7 + struct.unpack_from("<i", img, i + 3)[0] == target:
                hits.append((i, OPS[img[i + 1]]))
        elif b in OPS32 and (img[i + 1] & 0xC7) == 0x05:
            if i + 6 + struct.unpack_from("<i", img, i + 2)[0] == target:
                hits.append((i, OPS32[b]))
        i += 1
    return sorted(set(hits))


def main():
    if not ARGS:
        print(__doc__.strip())
        return 2
    check = "--selfcheck" in sys.argv[1:]
    target = int(ARGS[0], 16)
    img, base = file_to_image(open(EXE, "rb").read())
    hits, cells = scan(img, base, target)
    if check:
        want = reference_scan(img, target)
        if want != hits:
            print(f"SELFCHECK FAILED: fast {len(hits)} vs byte-loop {len(want)}")
            for o, k in sorted(set(want) ^ set(hits)):
                print(f"  only in {'byte-loop' if (o, k) in want else 'fast':10s}: {k} at +0x{o:X}")
            return 1
        print(f"selfcheck ok: fast scan agrees with the byte loop on {len(hits)} reference(s)")
    for off, kind in hits:
        print(f"{kind:14s} at +0x{off:X}")
    for off in cells:
        print(f"{'ptr cell':14s} at +0x{off:X}  (holds VA 0x{base + target:X})")
    print(f"{len(hits)} code reference(s) and {len(cells)} pointer cell(s) to +0x{target:X}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
