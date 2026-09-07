#!/usr/bin/env python3
"""Regenerate desert-core/src/collect.rs from the Desert Gatherer DMM pack.

Usage: python3 tools/gen-collect-names.py [pack-dir]
Default pack dir: desert-gatherer-dmm/, the copy of the pack kept in this repo.
Reads the "* - 2X.json" modules and emits (record key, record name, family).
Run from the workspace root.
"""
import glob, json, os, sys

base = sys.argv[1] if len(sys.argv) > 1 else "desert-gatherer-dmm"
fam = {"Foraging": "Foraging", "Logging": "Logging", "Mining": "Mining", "Ore Nodes": "Ore"}
rows = {}
for f in sorted(glob.glob(os.path.join(base, "* - 2X.json"))):
    cat = os.path.basename(f).split(" - ")[0]
    d = json.load(open(f, encoding="utf-8"))
    for g in d["patches"]:
        for c in g["changes"]:
            rows.setdefault(c["entry"], (c["record_key"], fam[cat]))
out = [
    "//! Gather-node records of Crimson Desert build 25116796, generated from the",
    "//! Desert Gatherer DMM pack (gimmickinfo records patched by that pack).",
    "//! `(record key, record name, family)`. Regenerate with tools/gen-collect-names.py.",
    "",
    "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
    "pub enum Family {",
    "    Foraging,",
    "    Logging,",
    "    Mining,",
    "    Ore,",
    "}",
    "",
    "pub const COLLECT_RECORDS: &[(u32, &str, Family)] = &[",
]
for name, (key, famname) in sorted(rows.items(), key=lambda kv: kv[0].lower()):
    out.append(f'    ({key}, "{name}", Family::{famname}),')
out += [
    "];",
    "",
    "/// Family of a gather record by its key, if it is one.",
    "pub fn family_by_key(key: u32) -> Option<Family> {",
    "    COLLECT_RECORDS.iter().find(|(k, _, _)| *k == key).map(|(_, _, f)| *f)",
    "}",
    "",
    "/// Family of a gather record by its name (case-insensitive), if it is one.",
    "pub fn family_by_name(name: &str) -> Option<Family> {",
    "    COLLECT_RECORDS.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name)).map(|(_, _, f)| *f)",
    "}",
    "",
    "#[cfg(test)]",
    "mod tests {",
    "    use super::*;",
    "    #[test]",
    "    fn known_records() {",
    "        assert_eq!(family_by_name(\"peony_01\"), Some(Family::Foraging));",
    "        assert_eq!(family_by_key(17020006), Some(Family::Foraging));",
    "        assert_eq!(family_by_name(\"ore_copper_01\"), Some(Family::Ore));",
    "        assert_eq!(family_by_name(\"gimmick_gate_metal_lattice_01_dungeon\"), None);",
    f"        assert_eq!(COLLECT_RECORDS.len(), {len(rows)});",
    "    }",
    "}",
]
open("desert-core/src/collect.rs", "w").write("\n".join(out) + "\n")
print("records:", len(rows))
