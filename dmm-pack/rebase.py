"""Rebase The Desert Gatherer patch offsets onto a new clean gimmickinfo table.

Usage:
    python rebase.py <clean_gimmickinfo_body> <game_build> [--dry-run]

DMM extracts the clean table to <DMM>/backups/gimmickinfo_pabgb_clean.bin and
the Steam build id is in steamapps/appmanifest_3321460.acf.

Each patch stores `record_key`, `entry` and `record_rel_offset` (bytes from the
record's u32 key). Records move between builds and the resource-output list can
shift inside a record, but each output block is a fixed 68-byte structure
(u8 flag=1, u32 item, ..., u64 min at +42, u64 max at +50, ff ff at +58, u32
item again at +64) preceded by a u32 count. This script finds every record by
key+name, locates each output list by that signature near its old position,
verifies the original bytes, rewrites the JSONs and regenerates VERIFICATION.txt.
"""
import hashlib
import json
import re
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MODULES = sorted(p for p in HERE.glob("*.json"))
LABEL_RE = re.compile(r" output (\d+)\.(\d+) (minimum|maximum) ")
BLOCK = 68                # bytes per resource-output block
MIN_AT, MAX_AT = 42, 50   # offsets of the u64 min/max inside a block
SEARCH = 1024             # how far from the old position to look for the list


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def locate_records(table: bytes, wanted: dict) -> dict:
    """Map record_key -> absolute offset of the u32 key (u32 key, u32 len, name, NUL)."""
    found = {}
    for key, name in wanted.items():
        kb = struct.pack("<I", key)
        nb = name.encode()
        hits = []
        for m in re.finditer(re.escape(kb), table):
            p = m.start()
            if struct.unpack_from("<I", table, p + 4)[0] != len(nb):
                continue
            if table[p + 8:p + 8 + len(nb)] == nb and table[p + 8 + len(nb)] == 0:
                hits.append(p)
        if len(hits) != 1:
            raise SystemExit(f"record {key} {name!r}: expected 1 hit, got {hits}")
        found[key] = hits[0]
    return found


def find_output_list(table, keypos, old_list, count, expect):
    """Return absolute offsets of every u32-count-prefixed output list near
    keypos+old_list whose blocks match `expect` {n: (min_bytes, max_bytes)}."""
    lo = max(keypos + 8, keypos + old_list - SEARCH)
    hi = keypos + old_list + SEARCH
    hits = []
    for q in range(lo, hi):
        if struct.unpack_from("<I", table, q)[0] != count:
            continue
        ok = True
        for n, (mn, mx) in expect.items():
            b = q + 4 + (n - 1) * BLOCK
            if not (table[b] == 1
                    and table[b + MIN_AT:b + MIN_AT + 8] == mn
                    and table[b + MAX_AT:b + MAX_AT + 8] == mx
                    and table[b + 58:b + 60] == b"\xff\xff"
                    and table[b + 5:b + 9] == table[b + 64:b + 68]):
                ok = False
                break
        if ok:
            hits.append(q)
    return hits


def rebase_module(path: Path, table: bytes, build: str, dry: bool):
    mod = json.loads(path.read_text(encoding="utf-8"))
    mult = mod["multiplier"]
    all_changes = []
    for group in mod["patches"]:
        changes = group["changes"]
        wanted = {c["record_key"]: c["entry"] for c in changes}
        recs = locate_records(table, wanted)
        lists = {}
        for c in changes:
            m = LABEL_RE.search(c["label"])
            if not m:
                raise SystemExit(f"{path.name}: unparsable label {c['label']!r}")
            g, n, kind = int(m.group(1)), int(m.group(2)), m.group(3)
            orig = bytes.fromhex(c["original"])
            pv = int.from_bytes(bytes.fromhex(c["patched"]), "little")
            if pv != int.from_bytes(orig, "little") * mult:
                raise SystemExit(f"{path.name}: {c['label']}: bad multiplier")
            at = MIN_AT if kind == "minimum" else MAX_AT
            old_list = c["record_rel_offset"] - at - 4 - (n - 1) * BLOCK
            L = lists.setdefault((c["record_key"], g),
                                 {"old_list": old_list, "expect": {}, "changes": []})
            if L["old_list"] != old_list:
                raise SystemExit(f"{path.name}: {c['label']}: inconsistent list offset")
            L["expect"].setdefault(n, [None, None])[0 if kind == "minimum" else 1] = orig
            L["changes"].append((c, n, at))
        moved = stable = 0
        for (key, g), L in lists.items():
            count = max(L["expect"])
            if sorted(L["expect"]) != list(range(1, count + 1)) or \
                    any(None in v for v in L["expect"].values()):
                raise SystemExit(f"{path.name}: record {key} group {g}: incomplete coverage")
            expect = {n: tuple(v) for n, v in L["expect"].items()}
            hits = find_output_list(table, recs[key], L["old_list"], count, expect)
            if len(hits) != 1:
                raise SystemExit(f"{path.name}: record {key} ({wanted[key]}) group {g}: "
                                 f"{len(hits)} candidate lists {hits}")
            q = hits[0]
            if q - recs[key] == L["old_list"]:
                stable += 1
            else:
                moved += 1
            for c, n, at in L["changes"]:
                new_off = q + 4 + (n - 1) * BLOCK + at
                assert table[new_off:new_off + 8] == bytes.fromhex(c["original"])
                c["offset"] = new_off
                c["record_rel_offset"] = new_off - recs[key]
                c["rel_offset"] = new_off - (recs[key] + 8 + len(c["entry"]))
        all_changes.extend(changes)
        print(f"  {path.name}: {len(changes)} patches verified; output lists: "
              f"{stable} unchanged, {moved} shifted inside record")
    mod["game_build"] = build
    if not dry:
        path.write_text(json.dumps(mod, indent=2, ensure_ascii=False) + "\n",
                        encoding="utf-8")
    return mod, all_changes


def simulate(table: bytes, changes) -> str:
    out = bytearray(table)
    for c in changes:
        p = bytes.fromhex(c["patched"])
        out[c["offset"]:c["offset"] + len(p)] = p
    return sha256(bytes(out))


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    dry = "--dry-run" in sys.argv
    if len(args) != 2:
        raise SystemExit(__doc__)
    table = Path(args[0]).read_bytes()
    build = args[1]
    print(f"table: {len(table)} bytes sha256={sha256(table)} build={build}")

    options, by_cat = [], {}
    for path in MODULES:
        mod, changes = rebase_module(path, table, build, dry)
        by_cat.setdefault(mod["category"], {})[mod["multiplier"]] = changes
        options.append({
            "option": mod["name"],
            "records": mod["coverage"]["records"],
            "resource_outputs": mod["coverage"]["resource_outputs"],
            "scalar_patches": len(changes),
            "simulated_patched_sha256": simulate(table, changes),
            "result": "PASS",
        })

    cat_offsets = {cat: {c["offset"] for c in next(iter(v.values()))}
                   for cat, v in by_cat.items()}
    cats = list(cat_offsets)
    disjoint = all(cat_offsets[a].isdisjoint(cat_offsets[b])
                   for i, a in enumerate(cats) for b in cats[i + 1:])
    mixed = by_cat["Mining"][2] + by_cat["Logging"][5]
    first = {cat: next(o for o in options if o["option"].startswith(cat)) for cat in cats}
    report = {
        "game_build": build,
        "source_gimmickinfo_sha256": sha256(table),
        "module_count": len(MODULES),
        "categories": {cat: {
            "records": first[cat]["records"],
            "resource_outputs": first[cat]["resource_outputs"],
            "scalar_patches": len(cat_offsets[cat]),
        } for cat in cats},
        "options": options,
        "category_offset_sets_disjoint": disjoint,
        "mixed_selection_test": {
            "selections": {"Mining": 2, "Logging": 5},
            "scalar_patches": len(mixed),
            "simulated_patched_sha256": simulate(table, mixed),
            "result": "PASS",
        },
        "result": "PASS" if disjoint else "FAIL",
    }
    text = "Automated package verification\n\n" + json.dumps(report, indent=2) + "\n"
    if dry:
        print(text)
    else:
        (HERE / "VERIFICATION.txt").write_text(text, encoding="utf-8")
    print("disjoint:", disjoint, "| result:", report["result"])


if __name__ == "__main__":
    main()
