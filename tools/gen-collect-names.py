#!/usr/bin/env python3
"""Regenerate desert-core/src/collect.rs.

Usage: python3 tools/gen-collect-names.py [pack-dir]
Default pack dir: desert-gatherer-dmm/, the copy of the pack kept in this repo.
Run from the workspace root.

Two sources, one output. The generator owns the whole file - enum, rows, both
lookups and every test - so `collect.rs` can be regenerated straight over
itself. That was not true before: the hand-written "Tried and rejected" note on
`family_by_name` and the `non_gather_records_stay_out` test used to be dropped
on every run, and the file's header said as much. They are emitted here now
(HAND_NOTE and HAND_TEST below); anything else hand-added to `collect.rs` still
dies on the next run, so add it here instead.

1. The DMM pack (`desert-gatherer-dmm/* - 2X.json`). Its four modules give the
   four families the pack itself multiplies: Foraging, Logging, Mining, Ore.
   Each `changes` entry already carries `entry` (record name) and `record_key`.

2. tools/extra-families.json - records the DMM pack has no module for, added to
   a family by hand. Today that is the water well `gimmick_well_0001_parts01`,
   which joins `Foraging` (water drawn from a well is gathered out of the world
   like everything else there), and the three placed money props, which are
   the `Money` family on their own. An entry may name an existing DMM family,
   in which case its records extend that family and its note is appended to
   the variant's doc comment, or a new one, in which case the generator emits
   the variant. Either way a hand-added row in `collect.rs` dies on the next
   regenerate; this input is how the generator owns it.

FORMAT of tools/extra-families.json: a JSON object mapping family name ->
{note, records, records_not_enabled?, candidates_not_enabled?}, where

  note                     free text: what the entry adds and how sure we are.
                           Emitted as a doc comment on the enum variant.
  records                  {"<record key>": {name, items, why}} - the SOURCE
                           OF TRUTH: the gimmickinfo records this entry adds,
                           by key. `name` is the record's name, `items` the
                           item ids its resource-output blocks carry (derived
                           data, verified below), `why` the evidence that a
                           table edit reaches the player through this record.
  records_not_enabled      optional, same shape: records measured or argued to
                           be unreachable by a table edit and deliberately NOT
                           rows. Verified against the body the same way, and
                           emitted only as a test asserting they stay out of
                           the table, so the trace is machine-checked rather
                           than a paragraph somebody has to find.
  candidates_not_enabled   optional {"<item id>": "why not"} - items considered
                           and deferred, recorded so the decision is visible.
                           Never emitted.

Entries are record-keyed, not item-keyed, and that is a finding rather than a
convenience. The same item (water, 22008) comes from two records, and only one
of them - the well - ever reads its record's output block; the other, the
breakable pot, hands the player a pre-built item instance and the table is
never consulted (docs/findings-water-wells-2026-09-12.md section 11). So which
RECORD a row names is what decides whether the multiplier reaches anything, and
the item is only what it pays. An earlier item-keyed form of this file expanded
22008 to both records and shipped a pot row that did nothing.

`items` is derived data, and this script verifies it rather than trusting it.
When DMM's clean table body (CLEAN_TABLE) is present, every record in `records`
and `records_not_enabled` is looked up by key in the record walk: its name must
match, the item ids across the output blocks it owns must equal the stored
`items`, and a record in `records` must own at least one block (a row nothing
can multiply is a mistake). Any mismatch is a hard error, because a game update
that changed the table is exactly what that should catch. When the body is
absent the stored rows are used and a banner says they were NOT verified.
Either way `collect.rs` regenerates. `$CD_CLEAN_TABLE` overrides the path, and
pointing it somewhere that does not exist is how that second branch gets tested
deliberately.

Item `1` is money (coin_0001 10..15, silverbar_0001 2500..2500). A record
paying it inside Foraging or any other gathering family would turn a yield
slider into an economy lever by accident, so every family but one refuses it
outright, in the stored `items` and again in what the body says the record
pays - see `MONEY_ITEM`. The one exception is the `Money` family
(MONEY_FAMILY), which exists for exactly those records and has the opposite
rule: every record in it, enabled or not, must pay item 1 and NOTHING ELSE,
checked the same two ways. So money can never ride into a gathering family and
a gathering item can never ride into Money; the two refusals are each other's
mirror. The family is the currency multiplier's whole surface: the placed coin
props, which read their block (docs/findings-water-wells-2026-09-12.md section
13.7, measured 2026-09-13 on build 25246367).

The walk re-implements `desert_core::gimmick::output_lists` + `block_ok`
(greedy, non-overlapping, item id at block+1) and attributes each list to its
record with the echo discriminator from
`docs/findings-water-wells-2026-09-12.md` sections 7 and 8: nested string
fields use the same `u32 len, bytes, NUL` shape as record names, so a backwards
scan for a header finds false positives freely, and the test that works is that
a real record echoes its own `u32` key immediately before a later digits-only
id sub-field. CALIBRATION below is what proves the walk still resolves; all
three anchors must match or the script refuses to write.
"""
import bisect
import glob
import json
import os
import re
import struct
import sys

# DMM's untouched copy of the gimmickinfo table body. Record-relative offsets
# only, so any build's clean body will do and this never needs rebasing. DMM
# writes it out when it first patches the table; a machine that has never run
# DMM will not have it, which is why the verification is optional. Same path
# desert-core/tests/gimmick_real.rs uses; $CD_CLEAN_TABLE overrides it, which is
# also how the "absent" path gets exercised on a machine that has the file.
CLEAN_TABLE = os.environ.get(
    "CD_CLEAN_TABLE", "/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin"
)

# desert_core::gimmick's block layout. ITEM_AT is 1, not 5: bytes +5..+8 are
# zero padding (findings-water-wells section 5).
BLOCK, ITEM_AT, MIN_AT, MAX_AT, MAX_QTY = 68, 1, 42, 50, 100_000

# Item 1 is money. Paid by no row outside MONEY_FAMILY, and by every row inside
# it; see the module docstring.
MONEY_ITEM = 1
MONEY_FAMILY = "Money"


def check_money_rule(where, name, family, items, source):
    """The two-way money rule, applied to one record's item list.

    `source` names where the list came from ("stored" for the json, "the clean
    body" for the walk) so the message says which of the two disagreed.
    """
    if family == MONEY_FAMILY:
        if items != [MONEY_ITEM]:
            die(
                f"{where}: {name!r} is a {MONEY_FAMILY} record but {source} says it "
                f"pays {items}; a {MONEY_FAMILY} record must pay item {MONEY_ITEM} "
                "(money) and nothing else"
            )
    elif MONEY_ITEM in items:
        die(
            f"{where}: {name!r} pays item {MONEY_ITEM} ({source}), which is MONEY "
            f"(coin_0001 10..15, silverbar_0001 2500..2500). Money never rides into "
            f"a gathering family; the {MONEY_FAMILY} family is the only place for it."
        )

# Table-wide totals and three (list offset -> record, record-relative offset)
# anchors the record walk must reproduce. `firewood_0001` is the load-bearing
# one: DMM patches file offset 12843255 = 12841210 + 1999 + 4 + MIN_AT, so a
# walk that puts this list anywhere else is not calibrated. Without the echo
# test the first two resolve to `NatureBuffTrigger` and `UnnamedTrigger_0`,
# both with the bogus key 16777216. The well and the pot are anchors because
# they are the two records the walk was first proved on; that the pot is no
# longer a row changes nothing about whether the walk resolves it.
CALIBRATION = {
    "lists": 573,
    "blocks": 896,
    "records": 13412,
    "anchors": [
        (12843209, 1002971, "firewood_0001", 1999),
        (4373870, 1001081, "gimmick_well_0001_parts01", 965),
        (1049316, 21030076, "Background_Breakable_66", 1600),
    ],
}

# The four DMM modules, in the order their families are emitted.
DMM_FAMILIES = [
    ("Foraging", "Foraging"),
    ("Logging", "Logging"),
    ("Mining", "Mining"),
    ("Ore Nodes", "Ore"),
]

# Hand-written parts the generator used to drop. They record findings the table
# cannot, so they live here rather than in the generated file.
HAND_NOTE = [
    "/// Exact record names only. Tried and rejected (2026-09-06): treating the",
    "/// felled-tree chunks (`log_*`) as Logging. The pickup event is ignored for",
    "/// them; the chunk actor even survives having its `firewood_*` materials cut",
    "/// out. Only the materials are gather nodes.",
]
HAND_TEST = [
    "    #[test]",
    "    fn non_gather_records_stay_out() {",
    '        assert_eq!(family_by_name("log_1002_index07"), None);',
    '        assert_eq!(family_by_name("gimmick_tree_cd_crop_apple_02_collect"), None);',
    '        assert_eq!(family_by_name("item_basic_onehand"), None);',
    '        assert_eq!(family_by_name("firewood_1002_index07"), Some(Family::Logging));',
    "    }",
]


def die(msg):
    sys.exit(f"gen-collect-names: {msg}")


def wrap_doc(text, width=76):
    """`text` as `/// ` lines, hard-wrapped, with no trailing whitespace."""
    out, line = [], "///"
    for word in text.split():
        if line != "///" and len(line) + 1 + len(word) > width:
            out.append(line)
            line = "///"
        line += " " + word
    out.append(line)
    return out


def by_name(records):
    """A record map's entries as (key, name) pairs, sorted like the table."""
    return sorted(((k, r["name"]) for k, r in records.items()), key=lambda kn: kn[1].lower())


# ---------------------------------------------------------------------------
# Source 1: the DMM pack
# ---------------------------------------------------------------------------


def read_dmm(base):
    fam = dict(DMM_FAMILIES)
    rows = {}
    files = sorted(glob.glob(os.path.join(base, "* - 2X.json")))
    if not files:
        die(f"no '* - 2X.json' modules under {base!r}")
    for path in files:
        cat = os.path.basename(path).split(" - ")[0]
        if cat not in fam:
            die(f"{path}: unknown DMM module {cat!r}")
        with open(path, encoding="utf-8") as fh:
            d = json.load(fh)
        for group in d["patches"]:
            for c in group["changes"]:
                rows.setdefault(c["entry"], (c["record_key"], fam[cat]))
    return rows


# ---------------------------------------------------------------------------
# Source 2: extra-families.json, and the clean-body walk that checks it
# ---------------------------------------------------------------------------


def read_record_map(path, family, field, raw):
    """{key: {name, items, why}} from one of the two record maps, validated."""
    if not isinstance(raw, dict):
        die(f"{path}: {family}.{field} must be an object keyed by record key")
    out = {}
    for key_text, rec in raw.items():
        where = f"{path}: {family}.{field}[{key_text!r}]"
        try:
            key = int(key_text)
        except ValueError:
            die(f"{where}: the key is not an integer")
        if key <= 0:
            die(f"{where}: non-positive record key")
        if not isinstance(rec, dict):
            die(f"{where}: must be {{name, items, why}}")
        name = rec.get("name")
        if not isinstance(name, str) or not name:
            die(f"{where}: `name` must be the record's name")
        items = rec.get("items")
        if (
            not isinstance(items, list)
            or not items
            or not all(isinstance(i, int) and i > 0 for i in items)
        ):
            die(f"{where}: `items` must be a non-empty list of item ids")
        check_money_rule(where, name, family, sorted(set(items)), "stored")
        why = rec.get("why", "")
        if not isinstance(why, str):
            die(f"{where}: `why` must be text")
        out[key] = {"name": name, "items": sorted(set(items)), "why": why}
    return out


def read_extras(path):
    if not os.path.exists(path):
        die(f"{path} is missing; it is the source of every non-DMM row")
    with open(path, encoding="utf-8") as fh:
        spec = json.load(fh)
    for family, body in spec.items():
        if not re.fullmatch(r"[A-Z][A-Za-z0-9]*", family):
            die(f"{path}: {family!r} is not a usable Rust enum variant name")
        body["records"] = read_record_map(path, family, "records", body.get("records", {}))
        body["records_not_enabled"] = read_record_map(
            path, family, "records_not_enabled", body.get("records_not_enabled", {})
        )
        if not body["records"]:
            die(f"{path}: {family} has no records; it is the source of truth")
        both = sorted(set(body["records"]) & set(body["records_not_enabled"]))
        if both:
            die(f"{path}: {family}: {both} are in both records and records_not_enabled")
        seen = {}
        for field in ("records", "records_not_enabled"):
            for key, rec in body[field].items():
                if rec["name"] in seen:
                    die(f"{path}: {family}: record name {rec['name']!r} appears twice")
                seen[rec["name"]] = key
        for raw in body.get("candidates_not_enabled", {}):
            if int(raw) == MONEY_ITEM:
                die(f"{path}: {family} lists item {MONEY_ITEM} as a candidate; never.")
    return spec


def output_lists(buf):
    """Every resource-output list in the table body, as (offset, block count).

    A direct port of `desert_core::gimmick::output_lists` + `block_ok`: a `u32`
    count of 1..=64 followed by that many valid blocks, walked greedily and
    non-overlapping. `b[5..9] == b[64..68]` is kept for fidelity even though
    both ranges are zero on every real block (findings section 5); the flag,
    the `FF FF` and `1 <= min <= max` are what make it selective.
    """
    n = len(buf)
    u32 = lambda o: struct.unpack_from("<I", buf, o)[0]  # noqa: E731
    u64 = lambda o: struct.unpack_from("<Q", buf, o)[0]  # noqa: E731

    def block_ok(at):
        if at + BLOCK > n:
            return False
        if buf[at] != 1 or buf[at + 58] != 0xFF or buf[at + 59] != 0xFF:
            return False
        if buf[at + 5 : at + 9] != buf[at + 64 : at + 68]:
            return False
        lo, hi = u64(at + MIN_AT), u64(at + MAX_AT)
        return 1 <= lo <= hi <= MAX_QTY

    lists, i = [], 0
    while i + 4 <= n:
        count = u32(i)
        if (
            1 <= count <= 64
            and i + 4 + count * BLOCK <= n
            and all(block_ok(i + 4 + k * BLOCK) for k in range(count))
        ):
            lists.append((i, count))
            i += 4 + count * BLOCK
        else:
            i += 1
    return lists


def record_headers(buf):
    """Every real record header, as a sorted list of (offset, key, name).

    Nested string fields share the record name's `u32 len, bytes, NUL` shape,
    so shape alone is not enough. The discriminator that works: a real record
    echoes its own `u32` key immediately before a later digits-only id
    sub-field. See findings section 7.
    """
    n = len(buf)
    u32 = lambda o: struct.unpack_from("<I", buf, o)[0]  # noqa: E731
    strs = {}
    for m in re.finditer(rb"[\x20-\x7E]{3,120}", buf):
        run = m.group()
        for rel in range(len(run) - 2):
            o = m.start() + rel
            if o < 8:
                continue
            length = u32(o - 4)
            if not (3 <= length and o + length <= n and buf[o + length] == 0):
                continue
            cand = buf[o : o + length]
            if all(0x20 <= ch <= 0x7E for ch in cand):
                strs[o] = cand.decode()
    soff = sorted(strs)
    bad = set('/.\\ <>="\'\t')
    hdrs = []
    for o in soff:
        name = strs[o]
        if name.isdigit() or any(c in bad for c in name):
            continue
        key = u32(o - 8)
        if key < 100_000:  # key == 1 and friends are nested fields
            continue
        j = bisect.bisect_right(soff, o)
        for o2 in soff[j : j + 400]:
            if o2 - o > 6000:
                break
            if strs[o2].isdigit() and u32(o2 - 8) == key:
                hdrs.append((o - 8, key, name))
                break
    hdrs.sort()
    return hdrs


def walk(buf):
    """(lists, headers, owner) for the table body - calibrated, or dies.

    Refuses to answer unless CALIBRATION reproduces exactly, because an
    uncalibrated record walk mis-spans boundaries silently and that is the
    failure mode this whole method exists to avoid.
    """
    lists = output_lists(buf)
    hdrs = record_headers(buf)
    blocks = sum(c for _, c in lists)
    got = {"lists": len(lists), "blocks": blocks, "records": len(hdrs)}
    for what in ("lists", "blocks", "records"):
        if got[what] != CALIBRATION[what]:
            die(
                f"calibration failed: {got[what]} {what}, expected "
                f"{CALIBRATION[what]}. The table changed or the walk broke; "
                "do not trust the walk. See "
                "docs/findings-water-wells-2026-09-12.md section 8."
            )
    starts = [h[0] for h in hdrs]
    owner = lambda off: hdrs[bisect.bisect_right(starts, off) - 1]  # noqa: E731
    for off, key, name, rel in CALIBRATION["anchors"]:
        h = owner(off)
        if (h[1], h[2], off - h[0]) != (key, name, rel):
            die(
                f"calibration anchor {off} resolved to {h[2]!r} (key {h[1]}, "
                f"rel +{off - h[0]}), expected {name!r} (key {key}, rel +{rel})"
            )
    return lists, hdrs, owner


def what_records_pay(buf, lists, owner):
    """{header offset: (sorted item ids, list count)} over every output list."""
    u32 = lambda o: struct.unpack_from("<I", buf, o)[0]  # noqa: E731
    paid = {}
    for off, count in lists:
        h = owner(off)
        items, n = paid.get(h[0], (set(), 0))
        items = items | {u32(off + 4 + k * BLOCK + ITEM_AT) for k in range(count)}
        paid[h[0]] = (items, n + 1)
    return {off: (sorted(items), n) for off, (items, n) in paid.items()}


def verify_or_warn(spec):
    """Re-derive every stored record's `items` from the clean body when it is here.

    Returns the banner lines to print. A drift is fatal: the stored items are
    derived data, and a game update changing the table is exactly what this
    should catch.
    """
    if not os.path.exists(CLEAN_TABLE):
        return [
            "",
            "!" * 72,
            f"! {CLEAN_TABLE} is ABSENT.",
            "! The records in tools/extra-families.json were NOT verified against",
            "! the table body; their stored rows were used as-is. That is fine for",
            "! a routine regenerate and NOT fine after a game update - re-run this",
            "! on a machine that has the clean body before trusting the result.",
            "!" * 72,
            "",
        ]
    with open(CLEAN_TABLE, "rb") as fh:
        buf = fh.read()
    lists, hdrs, owner = walk(buf)
    paid = what_records_pay(buf, lists, owner)
    by_key = {}
    for h in hdrs:
        by_key.setdefault(h[1], []).append(h)
    out = []
    for family, body in spec.items():
        for field, must_pay in (("records", True), ("records_not_enabled", False)):
            for key, rec in body[field].items():
                name = rec["name"]
                where = f"{family}.{field}: record {key} {name!r}"
                hits = [h for h in by_key.get(key, []) if h[2] == name]
                if len(hits) != 1:
                    others = sorted({h[2] for h in by_key.get(key, [])})
                    die(
                        f"{where}: {len(hits)} record headers in the clean body, "
                        f"expected exactly 1 (headers with that key: {others or 'none'}). "
                        "The table changed, or the row is wrong."
                    )
                items, nlists = paid.get(hits[0][0], ([], 0))
                if must_pay and nlists == 0:
                    die(f"{where}: owns no resource-output block; nothing to multiply")
                check_money_rule(where, name, family, items, "the clean body")
                if items != rec["items"]:
                    die(
                        f"{where}: the stored items do not match the clean body.\n"
                        f"  stored: {rec['items']}\n"
                        f"  in the table: {items} over {nlists} output list(s)\n"
                        "Fix the json or work out what changed in the game's table first."
                    )
            out.append(f"verified {family}.{field}: {len(body[field])} records against {CLEAN_TABLE}")
    return out


# ---------------------------------------------------------------------------
# Emit
# ---------------------------------------------------------------------------


def render(rows, order, notes):
    out = [
        "//! Gather-node records of Crimson Desert build 25116796, generated from the",
        "//! Desert Gatherer DMM pack (gimmickinfo records patched by that pack) plus",
        "//! tools/extra-families.json (the records DMM has no module for).",
        "//! `(record key, record name, family)`. Regenerate with tools/gen-collect-names.py.",
        "//!",
        "//! That generator rewrites this whole file and it is now safe to run straight",
        "//! over it: it owns the enum, the rows, both lookups AND the tests below,",
        "//! including the `family_by_name` note and `non_gather_records_stay_out`,",
        "//! which earlier versions dropped. Anything hand-added HERE still dies on the",
        "//! next run - add it to the generator instead.",
        "",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub enum Family {",
    ]
    for i, family in enumerate(order):
        if family in notes:
            if i:
                out.append("")
            out += ["    " + line for line in wrap_doc(notes[family])]
            out.append(f"    {family},")
            if i + 1 < len(order):
                out.append("")
        else:
            out.append(f"    {family},")
    out += [
        "}",
        "",
        "pub const COLLECT_RECORDS: &[(u32, &str, Family)] = &[",
    ]
    for name, (key, family) in sorted(rows.items(), key=lambda kv: kv[0].lower()):
        out.append(f'    ({key}, "{name}", Family::{family}),')
    out += [
        "];",
        "",
        "/// Family of a gather record by its key, if it is one.",
        "pub fn family_by_key(key: u32) -> Option<Family> {",
        "    COLLECT_RECORDS.iter().find(|(k, _, _)| *k == key).map(|(_, _, f)| *f)",
        "}",
        "",
        "/// Family of a gather record by its name (case-insensitive), if it is one.",
    ]
    out += HAND_NOTE
    out += [
        "pub fn family_by_name(name: &str) -> Option<Family> {",
        "    COLLECT_RECORDS.iter().find(|(_, n, _)| n.eq_ignore_ascii_case(name)).map(|(_, _, f)| *f)",
        "}",
        "",
        "#[cfg(test)]",
        "mod tests {",
        "    use super::*;",
    ]
    out += HAND_TEST
    out += [
        "",
        "    #[test]",
        "    fn known_records() {",
        '        assert_eq!(family_by_name("peony_01"), Some(Family::Foraging));',
        "        assert_eq!(family_by_key(17020006), Some(Family::Foraging));",
        '        assert_eq!(family_by_name("ore_copper_01"), Some(Family::Ore));',
        '        assert_eq!(family_by_name("gimmick_gate_metal_lattice_01_dungeon"), None);',
        f"        assert_eq!(COLLECT_RECORDS.len(), {len(rows)});",
        "    }",
    ]
    return out


def render_extra_tests(spec, rows):
    """Two tests per extra-families entry.

    The first says its rows are in the table under the right family and that
    the family's total is what the DMM pack plus this entry add up to. The
    second is the more valuable one: every `records_not_enabled` record must
    resolve to NO family, by key and by name. Those are the records a table
    edit was measured not to reach, and a row for one would be work the log
    reports and the player never sees - which is exactly what shipped once.
    """
    out = []
    for family, body in spec.items():
        rec = by_name(body["records"])
        total = sum(1 for (_, f) in rows.values() if f == family)
        items = ", ".join(str(i) for i in sorted({i for r in body["records"].values() for i in r["items"]}))
        out += ["", "    /// Rows from tools/extra-families.json, not from the DMM pack."]
        out += ["    " + line for line in wrap_doc(f"Items paid: {items}.")]
        out += [
            "    #[test]",
            f"    fn {family.lower()}_extra_records_are_present() {{",
            # A named slice rather than an array literal in the `for` head:
            # clippy's single_element_loop fires on the latter when an entry
            # has exactly one record, and this must be clean at any count.
            "        let extra: &[(u32, &str)] = &[",
        ]
        for key, name in rec:
            out.append(f'            ({key}, "{name}"),')
        out += [
            "        ];",
            "        for &(key, name) in extra {",
            f'            assert_eq!(family_by_key(key), Some(Family::{family}), "{{name}}");',
            f'            assert_eq!(family_by_name(name), Some(Family::{family}), "{{name}}");',
            "        }",
            "        assert_eq!(",
            f"            COLLECT_RECORDS.iter().filter(|(_, _, f)| *f == Family::{family}).count(),",
            f"            {total}",
            "        );",
            "    }",
        ]
        skipped = by_name(body["records_not_enabled"])
        if not skipped:
            continue
        out += [
            "",
            "    /// `records_not_enabled` in tools/extra-families.json: records a table",
            "    /// edit was measured or argued not to reach - they hand the player a",
            "    /// pre-built item instance and never read their output block (see",
            "    /// docs/findings-water-wells-2026-09-12.md section 11), or they are",
            "    /// not pickups at all; each record's `why` in that file says which.",
            "    /// They must stay OUT of the table, not sit in it inert: a row here",
            "    /// would be an edit the log reports and the player never sees.",
            "    #[test]",
            f"    fn {family.lower()}_records_not_enabled_stay_out() {{",
            "        let out: &[(u32, &str)] = &[",
        ]
        for key, name in skipped:
            out.append(f'            ({key}, "{name}"),')
        out += [
            "        ];",
            "        for &(key, name) in out {",
            '            assert_eq!(family_by_key(key), None, "{name}");',
            '            assert_eq!(family_by_name(name), None, "{name}");',
            "        }",
            "    }",
        ]
    out.append("}")
    return out


def main():
    base = sys.argv[1] if len(sys.argv) > 1 else "desert-gatherer-dmm"
    extras_path = os.path.join("tools", "extra-families.json")

    rows = read_dmm(base)
    dmm_count = len(rows)
    spec = read_extras(extras_path)
    banner = verify_or_warn(spec)

    notes = {f: b["note"] for f, b in spec.items() if b.get("note")}
    dmm_names = [rust for _, rust in DMM_FAMILIES]
    for family, body in spec.items():
        for key, rec in body["records_not_enabled"].items():
            name = rec["name"]
            if name in rows or any(k == key for k, _ in rows.values()):
                die(
                    f"{family}: {name!r} (key {key}) is listed as not enabled but is "
                    "a DMM pack record. One of the two is wrong; decide which."
                )
        for key, rec in body["records"].items():
            name = rec["name"]
            if name in rows:
                die(
                    f"{family}: record {name!r} (key {key}) is already a "
                    f"{rows[name][1]} record from the DMM pack. A record has "
                    "one family; decide which."
                )
            clash = [n for n, (k, _) in rows.items() if k == key]
            if clash:
                die(f"{family}: key {key} is already used by {clash[0]!r}")
            rows[name] = (key, family)

    order = dmm_names + sorted(f for f in spec if f not in dmm_names)
    text = render(rows, order, notes) + render_extra_tests(spec, rows)
    with open("desert-core/src/collect.rs", "w", encoding="utf-8") as fh:
        fh.write("\n".join(text) + "\n")

    print(f"records: {len(rows)} (DMM {dmm_count} + extras {len(rows) - dmm_count})")
    for family, body in spec.items():
        print(
            f"{family}: +{len(body['records'])} records, "
            f"{len(body['records_not_enabled'])} recorded as not enabled, "
            f"{len(body.get('candidates_not_enabled', {}))} candidate items deferred"
        )
    for line in banner:
        print(line)


if __name__ == "__main__":
    main()
