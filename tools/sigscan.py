#!/usr/bin/env python3
"""Scan CrimsonDesert.exe for byte signatures and class/event names.

Usage:
    python3 tools/sigscan.py [exe] [sigfile]

Defaults: the Steam install on /mnt/f and tools/reference-signatures.txt.
Signature lines are IDA-style: "48 8B ?? 20 01 00 00" (?? = wildcard).
Prints hit counts and file offsets; a good anchor hits exactly once.

Also checks every static-info type name in static-info-names.txt: each must
still occur exactly once as a NUL-delimited literal. A name that vanishes or
doubles after a game update means the type registry moved, which is an earlier
and louder warning than a byte signature going stale.
"""
import mmap
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
EXE = sys.argv[1] if len(sys.argv) > 1 else \
    "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe"
SIGFILE = Path(sys.argv[2]) if len(sys.argv) > 2 else HERE / "reference-signatures.txt"
NAMEFILE = HERE / "static-info-names.txt"

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


def check_static_info_names(m) -> None:
    """Every static-info type name must still occur exactly once, NUL-delimited.

    Substring matching is not good enough here: several names contain each other
    (`FactionInfo` inside `FactionInfoManager`, and so on), so the check is for a
    NUL on both sides. Adjacent strings in a table share a terminator, so the
    trailing NUL is matched and the leading one is tested by hand rather than
    made part of the pattern - `re` will not overlap two matches.
    """
    if not NAMEFILE.exists():
        print(f"  ({NAMEFILE.name} not found, skipped)")
        return
    names = [l.strip() for l in NAMEFILE.read_text().splitlines()
             if l.strip() and not l.startswith("#")]
    once, missing, dupes = 0, [], []
    for n in names:
        b = n.encode()
        hits = [x.start() for x in re.finditer(re.escape(b) + b"\x00", m)
                if x.start() > 0 and m[x.start() - 1] == 0]
        if len(hits) == 1:
            once += 1
        elif not hits:
            missing.append(n)
        else:
            dupes.append((n, len(hits)))
    print(f"{once:4d}/{len(names)} type names still occur exactly once")
    for n in missing:
        print(f"       MISSING  {n}")
    for n, c in dupes:
        print(f"       {c} HITS  {n}")
    if not missing and not dupes:
        print("       the static-info type registry is unchanged")


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
        print("\n--- static-info type names ---")
        check_static_info_names(m)


if __name__ == "__main__":
    main()
