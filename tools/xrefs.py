#!/usr/bin/env python3
"""Find code references to an RVA in CrimsonDesert.exe without a disassembler.

Usage: tools/xrefs.py <rva-hex> [exe]

Reports RIP-relative LEA/MOV (48/4C 8D or 8B, modrm mod=00 rm=101) whose
target is the RVA, and E8 call / E9 jmp rel32 whose target is the RVA.
Operates on the file laid out as an image, so offsets are RVAs.
"""
import struct, sys
sys.path.insert(0, __file__.rsplit("/", 1)[0])
EXE = sys.argv[2] if len(sys.argv) > 2 else "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe"
target = int(sys.argv[1], 16)

def file_to_image(d):
    pe = struct.unpack_from("<I", d, 0x3c)[0]
    nsec = struct.unpack_from("<H", d, pe + 6)[0]
    opt = pe + 24; optsz = struct.unpack_from("<H", d, pe + 20)[0]
    size = struct.unpack_from("<I", d, opt + 56)[0]
    img = bytearray(size)
    s = opt + optsz
    hdr = min(struct.unpack_from("<I", d, s + i * 40 + 20)[0] for i in range(nsec))
    img[:hdr] = d[:hdr]
    for i in range(nsec):
        vs, va, rs, ro = struct.unpack_from("<IIII", d, s + i * 40 + 8)
        n = min(rs, len(d) - ro); img[va:va + n] = d[ro:ro + n]
    return bytes(img), (struct.unpack_from("<I", d, opt+56)[0])

d = open(EXE, "rb").read()
img, _ = file_to_image(d)
text_end = len(img)
hits = []
i = 0
n = len(img)
# rel32 targets: scan every byte, check E8/E9 and RIP-relative modrm forms
while i < n - 7:
    b = img[i]
    if b in (0xE8, 0xE9):
        rel = struct.unpack_from("<i", img, i + 1)[0]
        if i + 5 + rel == target:
            hits.append((i, "call" if b == 0xE8 else "jmp"))
    elif b in (0x48, 0x4C, 0x49, 0x4D) and img[i + 1] in (0x8D, 0x8B, 0x89) and (img[i + 2] & 0xC7) == 0x05:
        rel = struct.unpack_from("<i", img, i + 3)[0]
        if i + 7 + rel == target:
            op = {0x8D: "lea", 0x8B: "mov r,[rip]", 0x89: "mov [rip],r"}[img[i + 1]]
            hits.append((i, op))
    elif b in (0x8B, 0x89, 0x8D) and (img[i + 1] & 0xC7) == 0x05:  # no REX
        rel = struct.unpack_from("<i", img, i + 2)[0]
        if i + 6 + rel == target:
            op = {0x8D: "lea32", 0x8B: "mov32 r,[rip]", 0x89: "mov32 [rip],r"}[b]
            hits.append((i, op))
    i += 1
for off, kind in hits:
    print(f"{kind:14s} at +0x{off:X}")
print(f"{len(hits)} reference(s) to +0x{target:X}")
