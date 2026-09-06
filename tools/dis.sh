#!/usr/bin/env bash
# Disassemble a range of CrimsonDesert.exe by RVA (Intel syntax). Addresses and
# branch targets are printed as RVAs (add 0x140000000 for the preferred VA).
#   tools/dis.sh <start-rva-hex> <end-rva-hex>
# Builds an image-layout copy of the exe on first use in $TMPDIR (or /tmp).
set -euo pipefail
EXE="${EXE:-/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe}"
IMG="${IMG:-${TMPDIR:-/tmp}/crimsondesert.img}"
if [ ! -s "$IMG" ]; then
  python3 - "$EXE" "$IMG" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
pe = struct.unpack_from("<I", d, 0x3c)[0]; nsec = struct.unpack_from("<H", d, pe + 6)[0]
opt = pe + 24; optsz = struct.unpack_from("<H", d, pe + 20)[0]
img = bytearray(struct.unpack_from("<I", d, opt + 56)[0]); s = opt + optsz
for i in range(nsec):
    vs, va, rs, ro = struct.unpack_from("<IIII", d, s + i * 40 + 8)
    n = min(rs, len(d) - ro); img[va:va + n] = d[ro:ro + n]
open(sys.argv[2], "wb").write(img)
PY
fi
objdump -D -b binary -m i386:x86-64 -M intel \
  --start-address=$((0x$1)) --stop-address=$((0x$2)) "$IMG" \
  | tail -n +8 | sed -E 's/^ *([0-9a-f]+):\t[0-9a-f ]+\t?/+\1  /; s/ +/ /g' | cut -c1-100
