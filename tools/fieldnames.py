#!/usr/bin/env python3
"""Field-name inventory for the game's static-info record classes.

Usage:
    python3 tools/fieldnames.py                       # rebuild the inventory
    python3 tools/fieldnames.py GimmickInfo           # one class's fields
    python3 tools/fieldnames.py --field dropTagNameHash   # who has this field?
    python3 tools/fieldnames.py --grep drop           # fuzzy over class+field
    python3 tools/fieldnames.py --list                # one line per class
    python3 tools/fieldnames.py --rescan              # force the walk

Output, gitignored and machine-generated:
    analysis/fieldnames.json    one entry per (class, field), with the file
                                offset and the RVA of the error message that
                                names it

Where the names come from, because it is not obvious and nothing else in
`docs/` used to mention it. Every static-info record deserializer reports a
per-field read failure with a UTF-8 **Korean** message of the form

    <ClassName>의 _<fieldName>를 읽어들이는데 실패했다.
    ("failed to read <Class>'s <field>")

so the class name and the field name are sitting in the string pool of the
shipped exe, in plain sight, for every field of every record type. `strings`
misses them because they are not ASCII and Ghidra has not typed them. One
regex over the image recovers the lot:

    [A-Za-z0-9_:\\-]{2,80}\\xec\\x9d\\x98 _[A-Za-z0-9_]{1,80}\\xeb\\xa5\\xbc

The two escapes are the only Korean this needs: `\\xec\\x9d\\x98` is 의 (the
possessive particle) and `\\xeb\\xa5\\xbc` is 를 (the object marker). Nothing
here decodes Korean beyond matching those two literals.

**Scope: this tool answers "what are the fields called", not "where are the
fields".** Recovering a field's *offset* needs a second, mechanical step that
is deliberately not implemented here: find the RIP-relative `48 8D 05 disp32`
that loads the message (its RVA is in the output, and `tools/xrefs.py` takes
an RVA), then read the offset out of the surrounding
`lea rdx,[rec+OFF]; mov rcx,rdi; call <reader>; test al,al; jne ok;
lea rax,[msg]` shape, where OFF sits 0x12-0x20 bytes above the message load.
That procedure is written down in
`docs/findings-water-wells-2026-09-12.md` section 7 and the method itself in
`docs/reference-internals.md` section 19.9; it is how that investigation's
`ItemInfo` offsets were obtained.

What the names are and are not:

  * The class names are the **logical** CamelCase names (`GimmickInfo`,
    `DropSetInfo`), not the lowercase table names the accessor census keys on
    (`gimmickinfo`, `dropsetinfo`). Section 19.2's inventory carries both.
  * 536 classes appear here against 149 static-info **tables**, because nested
    record types (`DropInfoData` inside `GimmickInfo`) get their own messages
    and are not tables of their own.
  * A name is evidence of a field the deserializer reads, nothing more. It
    says nothing about the field's type, width, order or offset, and read
    order is not string-pool order.

Offline, stdlib-only, about a second. Run from the workspace root; the dev
shell is only needed because the output lives with everything else it builds.
"""
import argparse
import json
import mmap
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = Path("/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/CrimsonDesert.exe")
JSON_OUT = ROOT / "analysis" / "fieldnames.json"

# 의 and 를. The message is "<Class>의 _<field>를 읽어들이는데 실패했다."; the
# trailing verb is not matched, because a couple of variants word it
# differently and the two particles are what actually delimit the two names.
OF = "의".encode()
OBJ = "를".encode()
MSG_RE = re.compile(rb"[A-Za-z0-9_:\-]{2,80}" + OF + rb" _[A-Za-z0-9_]{1,80}" + OBJ)

# Anchors re-checked on every rebuild. The two counts are this walk's own
# numbers on build 25246367; the four relations under them are the ones that
# made the inventory worth keeping, and each is independently checkable.
EXPECT_PAIRS = 4675
EXPECT_CLASSES = 536
ANCHORS = [
    # (description, class, fields that must be present)
    ("the two GimmickInfo yield lists (sections 16, 19.6.1)",
     "GimmickInfo", ["dropInfoDataList", "dropSetInfoList"]),
    ("DropInfoData is the output block the gatherer edits (section 16)",
     "DropInfoData", ["minValue", "maxValue", "keyRaw", "dropTagNameHash"]),
    ("DropSetInfo is the reward table of section 20.22",
     "DropSetInfo", ["key", "dropRollType", "totalDropRate", "dropTagNameHash"]),
]
# Exactly two classes carry a drop tag hash, which is what makes section 16.1's
# "are these dropsetinfo keys?" question a sharp one rather than a vague one.
TAG_HASH_CLASSES = ["DropInfoData", "DropSetInfo"]


def rva_map(path):
    """Return a function mapping a file offset to an RVA, from the PE sections.

    `tools/xrefs.py` takes RVAs and `tools/sigscan.py` prints file offsets; the
    two are not composable without this. Carrying both in the output is what
    lets a message be handed straight to `xrefs.py`.
    """
    d = path.read_bytes()[:0x2000]
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    nsec = struct.unpack_from("<H", d, pe + 6)[0]
    optsz = struct.unpack_from("<H", d, pe + 20)[0]
    s = pe + 24 + optsz
    secs = []
    for i in range(nsec):
        vs, va, rs, ro = struct.unpack_from("<IIII", d, s + i * 40 + 8)
        secs.append((ro, ro + rs, va))

    def to_rva(off):
        for lo, hi, va in secs:
            if lo <= off < hi:
                return va + (off - lo)
        return None
    return to_rva


def scan(exe):
    """Every (class, field) the deserializers name, in file order."""
    to_rva = rva_map(exe)
    pairs = []
    with open(exe, "rb") as f:
        m = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
        for hit in MSG_RE.finditer(m):
            cls, field = hit.group().split(OF + b" _")
            pairs.append({
                "class": cls.decode(),
                "field": field[:-len(OBJ)].decode(),
                "message_off": hit.start(),
                "message_rva": to_rva(hit.start()),
            })
        m.close()
    return pairs


def by_class(pairs):
    out = {}
    for p in pairs:
        out.setdefault(p["class"], []).append(p)
    return out


def checks(pairs):
    """Print the self-checks. Any FAIL means the exe moved; explain it first."""
    groups = by_class(pairs)
    ok = True

    def line(good, text):
        nonlocal ok
        ok = ok and good
        print(f"  {'ok  ' if good else 'FAIL'}  {text}")

    line(len(pairs) == EXPECT_PAIRS, f"{len(pairs)} (class, field) pairs, expected {EXPECT_PAIRS}")
    line(len(groups) == EXPECT_CLASSES, f"{len(groups)} distinct classes, expected {EXPECT_CLASSES}")
    dupes = [c for c, ps in groups.items() if len({p["field"] for p in ps}) != len(ps)]
    line(not dupes, f"no class names a field twice{'' if not dupes else ': ' + ', '.join(dupes[:5])}")
    for why, cls, want in ANCHORS:
        have = {p["field"] for p in groups.get(cls, [])}
        missing = [w for w in want if w not in have]
        line(not missing, f"{cls}: {', '.join(want)} - {why}"
                          + ("" if not missing else f"  MISSING {', '.join(missing)}"))
    holders = sorted(c for c, ps in groups.items()
                     if any(p["field"] == "dropTagNameHash" for p in ps))
    line(holders == TAG_HASH_CLASSES,
         f"dropTagNameHash on exactly {', '.join(TAG_HASH_CLASSES)} (got: {', '.join(holders) or 'none'})")
    unmapped = [p for p in pairs if p["message_rva"] is None]
    line(not unmapped, f"every message offset maps to an RVA ({len(unmapped)} did not)")
    return ok


def load(exe, rescan):
    if not rescan and JSON_OUT.exists():
        try:
            return json.loads(JSON_OUT.read_text())["pairs"]
        except (ValueError, KeyError):
            pass  # a truncated or older file: rebuild rather than half-answer
    return rebuild(exe, quiet=True)


def rebuild(exe, quiet=False):
    if not exe.exists():
        sys.exit(f"{exe} not found (pass --exe)")
    pairs = scan(exe)
    JSON_OUT.parent.mkdir(parents=True, exist_ok=True)
    JSON_OUT.write_text(json.dumps({
        "source": str(exe),
        "count": len(pairs),
        "classes": len(by_class(pairs)),
        "method": "docs/reference-internals.md section 19.9",
        "pairs": pairs,
    }, indent=1) + "\n")
    if not quiet:
        print(f"{exe}\n  -> {JSON_OUT.relative_to(ROOT)}  "
              f"{len(pairs)} pairs, {len(by_class(pairs))} classes\n")
        print("--- self-checks ---")
        if not checks(pairs):
            print("\na FAIL means the exe's field messages moved. Do not trust anything")
            print("downstream of this inventory until it is explained.")
    return pairs


def show_class(pairs, name):
    groups = by_class(pairs)
    exact = [c for c in groups if c.lower() == name.lower()]
    hits = exact or sorted(c for c in groups if name.lower() in c.lower())
    if not hits:
        print(f"no class matching {name!r}")
        return
    for c in hits[:20]:
        print(f"\n{c}  ({len(groups[c])} fields)")
        for p in sorted(groups[c], key=lambda p: p["field"]):
            rva = p["message_rva"]
            print(f"  _{p['field']:<48s} msg @ file 0x{p['message_off']:X}"
                  + (f"  rva 0x{rva:X}" if rva is not None else ""))
    if len(hits) > 20:
        print(f"\n... and {len(hits) - 20} more classes matched")


def show_field(pairs, name):
    hits = [p for p in pairs if p["field"].lower() == name.lower()]
    if not hits:
        print(f"no field named {name!r} (try --grep)")
        return
    print(f"_{name} appears on {len({p['class'] for p in hits})} class(es):")
    for p in sorted(hits, key=lambda p: p["class"]):
        print(f"  {p['class']:<48s} msg @ file 0x{p['message_off']:X}")


def show_grep(pairs, needle):
    n = needle.lower()
    hits = [p for p in pairs if n in p["class"].lower() or n in p["field"].lower()]
    for p in sorted(hits, key=lambda p: (p["class"], p["field"]))[:400]:
        print(f"{p['class']}._{p['field']}")
    print(f"{len(hits)} match(es)" + (", first 400 shown" if len(hits) > 400 else ""))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("name", nargs="?", help="class name to look up (substring ok)")
    ap.add_argument("--field", help="which classes carry this field name")
    ap.add_argument("--grep", help="substring over both class and field names")
    ap.add_argument("--list", action="store_true", help="one line per class")
    ap.add_argument("--rescan", action="store_true", help="re-walk the exe")
    ap.add_argument("--exe", type=Path, default=EXE)
    a = ap.parse_args()

    if a.name or a.field or a.grep or a.list:
        pairs = load(a.exe, a.rescan)
        if a.list:
            for c, ps in sorted(by_class(pairs).items(), key=lambda kv: (-len(kv[1]), kv[0])):
                print(f"{len(ps):4d}  {c}")
        if a.field:
            show_field(pairs, a.field)
        if a.grep:
            show_grep(pairs, a.grep)
        if a.name:
            show_class(pairs, a.name)
        return
    rebuild(a.exe)


if __name__ == "__main__":
    main()
