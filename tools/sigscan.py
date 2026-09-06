#!/usr/bin/env python3
"""Scan CrimsonDesert.exe for byte signatures and class/event names.

Usage:
    python3 tools/sigscan.py [exe] [sigfile]

Defaults: the Steam install on /mnt/f and tools/cdloot-signatures.txt.
Signature lines are IDA-style: "48 8B ?? 20 01 00 00" (?? = wildcard).
Prints hit counts and file offsets; a good anchor hits exactly once.
"""
import mmap
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
EXE = sys.argv[1] if len(sys.argv) > 1 else \
    "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe"
SIGFILE = Path(sys.argv[2]) if len(sys.argv) > 2 else HERE / "cdloot-signatures.txt"

SIG_RE = re.compile(r"^([0-9A-Fa-f]{2}|\?\?)( ([0-9A-Fa-f]{2}|\?\?))+$")
NAMES = [
    b".?AVClientActorManager@pa@@",
    b"ClientActorManager", b"ClientGimmickActorComponent",
    b"ClientStatusActorComponent", b"ClientAiActorComponent", b"CameraManager",
    b"TrocTrProcessLootingDeadDropOnceTimer", b"TrocTrProcessPickUpItemOnceTimer",
    b"TrocTrPushCharacterToInventoryOnceTimer",
    b"TrocTrInteractionDoStepDoInteractionOnceTimer",
    b"TrocTrStealItemByFrameEventOnceTimer",
    b"gimmickinfo", b"iteminfo",
]


def sig_to_regex(sig: str) -> bytes:
    return b"".join(b"." if t == "??" else re.escape(bytes([int(t, 16)]))
                    for t in sig.split())


def main() -> None:
    sigs = [l.strip() for l in SIGFILE.read_text().splitlines()
            if l.strip() and not l.startswith("#") and SIG_RE.match(l.strip())]
    with open(EXE, "rb") as f:
        m = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
        print(f"{EXE}\n  size {len(m):,} bytes\n")
        print("--- signatures ---")
        for s in sigs:
            hits = [x.start() for x in re.finditer(sig_to_regex(s), m, re.S)]
            where = "  @ " + ", ".join(hex(h) for h in hits[:5]) if hits else ""
            print(f"{len(hits):3d}  {s}{where}")
        print("\n--- names ---")
        for n in NAMES:
            hits = [x.start() for x in re.finditer(re.escape(n), m)]
            first = hex(hits[0]) if hits else "-"
            print(f"{len(hits):4d}  {n.decode():48s} first @ {first}")


if __name__ == "__main__":
    main()
