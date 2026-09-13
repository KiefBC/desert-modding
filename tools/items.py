#!/usr/bin/env python3
"""Item cross-reference for the gimmickinfo resource-output blocks.

Usage:
    python3 tools/items.py                     # rebuild both outputs
    python3 tools/items.py 22008               # what is item 22008?
    python3 tools/items.py salt                # find an item by inferred name
    python3 tools/items.py --record peony_01   # what does a record yield?
    python3 tools/items.py --family Foraging   # every item of one family
    python3 tools/items.py --unclassified      # items no Family covers yet
    python3 tools/items.py --list              # one line per item
    python3 tools/items.py --loose             # ditto, wider detector, own files

Outputs, all four gitignored and machine-generated:
    analysis/items.json             one entry per item id, machine-readable
    docs/reference-items.md         the human cross-reference
    analysis/items-loose.json       the same, `--loose` (SEPARATE FILES: a
    docs/reference-items-loose.md   loose run can never clobber the default)

Reads the clean `gimmickinfo` table body DMM writes out
(`/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin`), which carries
record-relative offsets only and so needs no rebasing for a game update, and
`desert-core/src/collect.rs` for the current `Family` of each record.

Method, and its limits, because the doc states them and this is where they are
implemented (`docs/findings-water-wells-2026-09-12.md` sections 3, 5, 7 and 8):

  * Blocks are found by the detector `desert_core::gimmick::output_lists` uses:
    a `u32 count` followed by `count` well-formed 68-byte blocks, walked greedily
    and non-overlapping. The item id is read at **block+1**, not +5.
  * A block is attributed to its record by the echo discriminator: a real record
    header echoes its own `u32` key immediately before a later digits-only id
    sub-field. A naive backwards scan finds nested string fields instead
    (`key = 16777216` is the usual false positive) and mis-attributes.
  * **Item names are inferred from the names of the records that yield them.**
    Nothing here reads the game's own item names: those live in the `.paz`
    archives and are not extracted. An inferred name is a hypothesis, and the
    confidence field says how much of one.
  * There are **two detectors** and the choice is named, never implied (see
    `Detector` below). `shipped` is the plugin's own, both equality clauses
    included, and is the default; `--loose` drops the pad clause and finds
    589 lists / 1038 blocks / 311 items instead of 573 / 896 / 215. The extra
    content is real - chests, dig sites, dungeon loot - but it is **invisible
    to the mod**, and it is not uniformly trustworthy: read the caveats at the
    top of `docs/reference-items-loose.md` before using an id from it.

Run from the workspace root, inside `nix develop` (stdlib-only, but the outputs
live next to everything else the dev shell builds).
"""
import argparse
import bisect
import collections
import json
import signal
import re
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TABLE = Path("/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin")
COLLECT_RS = ROOT / "desert-core" / "src" / "collect.rs"
JSON_OUT = ROOT / "analysis" / "items.json"
DOC_OUT = ROOT / "docs" / "reference-items.md"
# The loose run writes its own pair. They are deliberately different paths: a
# `--loose` run must never be able to overwrite the population the plugin
# actually sees, because that is the one anything downstream acts on.
JSON_OUT_LOOSE = ROOT / "analysis" / "items-loose.json"
DOC_OUT_LOOSE = ROOT / "docs" / "reference-items-loose.md"

# Block layout, from desert_core::gimmick. ITEM_AT is the corrected +1; the
# constant said 5 until 2026-09-12, which is four bytes of zero padding
# (findings §5). Keep these in step with desert-core/src/gimmick.rs.
BLOCK = 68
ITEM_AT = 1
ITEM_TAIL_AT = 60
PAD_AT = ITEM_AT + 4
PAD_TAIL_AT = ITEM_TAIL_AT + 4
MIN_AT = 42
MAX_AT = 50
MAX_COUNT = 64
MAX_QTY = 100_000

# `PAD_TAIL_AT` is the second half of `block_ok`'s pad clause, and calling it a
# pad is only true of the population that clause admits. What the deserialiser
# says it is: `FUN_141a37180`, the block parser, consumes exactly `+0..+63` and
# never reads these four bytes. The list loop `FUN_1414a7cc0` reads them after
# each block and stores them at `entry+0x08` of its 0x10-stride entries, so
# `+64` is **the list entry's own key field** - zero on every gather record,
# populated on the records only `--loose` can see. It is not a pad, not the high
# half of a wide item id, and not the variant tag at `+9` (both populations
# carry tags 0 and 4). `PAD_AT` (`+5`) really is zero on all 1038 blocks of both
# populations, so that half of the clause is the vacuous one.
ENTRY_KEY_AT = PAD_TAIL_AT

# Every id in the shipped population is eight digits or fewer, and the largest
# is 10021720 (`goldbar`); 205 of the 215 are six or seven. The bound is the
# next order of magnitude up from that, so it accuses nothing the plugin
# already trusts. Anything past it is **reported, never dropped**: an
# out-of-range id means the list it sits in is being mis-parsed, and saying so
# is worth more than hiding it.
MAX_PLAUSIBLE_ITEM_ID = 99_999_999


class Detector:
    """Which of the two block detectors a walk uses. A named choice, not a flag.

    Both check the block's shape - flag byte 1 at `+0`, `FF FF` at `+58`, a
    `u64` min/max pair at `+42`/`+50` inside `1..=MAX_QTY` - and both require
    the two copies of the item id at `ITEM_AT` and `ITEM_TAIL_AT` to agree.
    They differ in one clause:

    * `shipped` also requires `PAD_AT` to equal `PAD_TAIL_AT`, exactly as
      `desert_core::gimmick::block_ok` does. **This is the population the
      plugin sees**, and the only one whose yields the gatherer multiplies.
    * `loose` drops that clause and keeps everything else.

    Dropping it grows the walk from 573 lists / 896 blocks / 215 items to
    589 / 1038 / 311. The 573 are a strict subset of the 589; the 16 extra
    lists differ only in carrying a nonzero `u32` at `PAD_TAIL_AT`, which the
    deserialiser says is the list entry's key field (see `ENTRY_KEY_AT`) rather
    than a pad. **None of the 16 is one of the 275 gather records the DMM pack
    edits** - they are chests, dig sites and dungeon loot: `Temple_Chest_01`,
    `dff_chest_24`, `gimmick_item_dropset_treasurebox_01`,
    `clawmachine_capsule_01`, `Action_dig_01`, `gimmick_Dig_land_0001`, the
    `gimmick_abyssone_bridge_gate_*` set and `gimmick_marni_teleportation_*`.

    (An earlier revision of this docstring said the extra lists sit in records
    like `player`. They do not. `player` is a **false header**: it occurs 1267
    times, each preceded by the `u32` 1, which is a nested string field and not
    a record key. The claim came from a comment in
    `desert-core/src/gimmick.rs` that has since been corrected; the key-echo
    discriminator in `Table.records` is what rules such headers out. See
    `docs/findings-water-wells-2026-09-12.md` section 7.)
    """

    __slots__ = ("name", "pad_clause", "summary")

    def __init__(self, name: str, pad_clause: bool, summary: str):
        self.name = name
        self.pad_clause = pad_clause
        self.summary = summary

    def __repr__(self) -> str:
        return f"Detector({self.name!r})"


SHIPPED = Detector(
    "shipped", True,
    "both equality clauses, exactly desert_core::gimmick::block_ok - the "
    "population the plugin sees and the gatherer multiplies")
LOOSE = Detector(
    "loose", False,
    "the item-id clause only; the pad clause dropped. Finds everything the "
    "shipped detector finds plus 16 lists the mod cannot see")
DETECTORS = {d.name: d for d in (SHIPPED, LOOSE)}

# Record-attribution tuning, from findings §7/§8. Changing any of these
# invalidates the three calibration anchors below.
MIN_RECORD_KEY = 100_000
ECHO_WINDOW = 6000
ECHO_LOOKAHEAD = 400
BAD_NAME_CHARS = set('/.\\ <>="\'\t')

# Expected results. The scan prints a FAIL line for any of these that moves; all
# five were confirmed three independent ways in findings §9.
EXPECT_LISTS = 573
EXPECT_BLOCKS = 896
EXPECT_ITEMS = 215
EXPECT_RECORDS = 13412
# The same three for the loose walk, plus what must stay true *between* the two
# populations: every shipped list is also a loose list, and the extras are
# exactly the blocks with a nonzero entry key.
EXPECT_LOOSE_LISTS = 589
EXPECT_LOOSE_BLOCKS = 1038
EXPECT_LOOSE_ITEMS = 311
EXPECT_LOOSE_ONLY_LISTS = EXPECT_LOOSE_LISTS - EXPECT_LISTS      # 16
EXPECT_LOOSE_ONLY_BLOCKS = EXPECT_LOOSE_BLOCKS - EXPECT_BLOCKS   # 142
# Nine ids in `391518521..391518546`, all nine inside the one list. They are the
# reason the loose population is reported as less trustworthy than the default.
EXPECT_IMPLAUSIBLE_IDS = 9
IMPLAUSIBLE_HOME = "gimmick_item_dropset_treasurebox_01"
# 101 ids occur in the 142 loose-only blocks. Five of them also occur in the
# shipped population, so 96 items are ones the mod is blind to outright. Five
# out of 101 is not corroboration and is not meant to read as any: chests yield
# different things from gather nodes, so a low overlap is exactly what a
# correct walk would produce too. It is recorded because a *move* in it is
# worth noticing.
EXPECT_LOOSE_ONLY_ITEMS = 96
EXPECT_SHARED_IDS = [1, 53, 75001, 1001597, 1001957]
ANCHORS = [
    (12843209, "firewood_0001", 1999),   # matches DMM's own patch offset
    (4373870, "gimmick_well_0001_parts01", 965),
    (1049316, "Background_Breakable_66", 1600),
]
# Item id -> (inferred name, amounts) the doc confirmed by two agreeing records
# each. Checked, not trusted: a disagreement means attribution is off.
ANCHOR_NAMES = {
    1000648: "salt", 1000602: "sugar", 1000646: "flour", 1000608: "pepper",
    1000667: "cheese", 1000668: "fishmeat", 1000666: "ginseng",
    1000603: "honey", 1000647: "wine", 740001: "leather", 757006: "peony",
    710001: "firewood", 720004: "copper", 756802: "weed_sophora",
}

# Identities established by evidence the name inference cannot see. Each needs a
# reason, and the reason is what makes it not a guess.
CURATED = {
    22008: ("water", "the yielding record carries GIMMICK_WATER_PICKUP and "
                     "lives under /well/; the other source is a water pot "
                     "(findings-water-wells-2026-09-12.md section 1)"),
    1: ("money", "gimmick_item_common_coin_0001 gives 10..15 and "
                 "gimmick_item_common_silverbar_0001 gives 2500..2500 of the "
                 "same id: this is currency, not an item. The bag names it "
                 "Money_Copper; silver and gold are denominations of the one "
                 "count, not items (there is no silver bar in the world, "
                 "whatever silverbar_0001 is named). A currency multiplier is "
                 "planned separately and this id must stay out of any gather "
                 "family"),
    53: ("goldbar", "the bag names it GoldBar (every 2026-09-13 survey, "
                    "`GoldBar key=53 x1`). The name inference said itembox, "
                    "because the only records paying it are itembox_07, "
                    "itembox_11 and itembox_Field_Space - chests that contain "
                    "a gold bar, so the inference named the container. Not "
                    "to be confused with 10021720, the fake/prop gold bar"),
}

# Tokens that name the container, the prop kind or the variant rather than the
# goods. Stripped before a record name is read as an item name.
NOISE = {
    "gimmick", "item", "items", "cd", "in", "ex", "dpf", "dpfo", "common",
    "basic", "trade", "dropset", "tableset", "equip", "backpack", "shops",
    "shop", "collect", "rare", "catched", "attach", "background", "bg",
    "breakable", "docking", "target", "core", "mine", "ore", "cave", "part",
    "parts", "index", "off", "on", "obj", "prop", "deco",
    "tool", "craft", "battle", "log",
    # bare container nouns and size adjectives: `docking_core_big` is not an
    # item called "big", and `..._egg_bucket_01` is an egg
    "box", "bucket", "basket", "sack", "crate", "barrel", "bag", "pack",
    "big", "small", "large", "mid", "middle",
}
# A token that is itself the container (`mushroombasket`, `itembox`,
# `cerealstall`) names the furniture, not the goods. It is not dropped - for a
# few items it is all there is - but it never wins while a goods word is on
# offer.
CONTAINER_TOKEN_RE = re.compile(
    r"basket|box|sack|stall|bucket|crate|barrel|shelf|stand")
NOISE_RE = re.compile(r"^(?:\d+|index\d*|parts?\d*|\d{2,}[a-z]?)$")
# A record whose name says "container": it legitimately holds varied goods, so
# an item named only by one of these is a guess however clean the token is.
CONTAINER_RE = re.compile(
    r"dropset|tableset|basket|itembox|_box|box_|sack|stall|bucket|shops_|"
    r"breakable|backpack|noticeboard|docking|warehouse", re.I)


def die(msg: str) -> "None":
    print(f"items.py: {msg}", file=sys.stderr)
    raise SystemExit(1)


# ---------------------------------------------------------------- table walking

class Table:
    """The clean table body, with the two walks findings §8 records.

    A table carries the `Detector` it was opened with, and every walk takes an
    optional override so one loaded body can be walked both ways - which is
    what a `--loose` build does, because it has to know which of its lists the
    shipped detector would also have found.
    """

    def __init__(self, path: Path, detector: Detector = SHIPPED):
        try:
            self.b = path.read_bytes()
        except OSError as e:
            die(f"cannot read the clean table body {path}: {e}\n"
                "  DMM writes it out when it first patches gimmickinfo; "
                "pass --table if it lives elsewhere.")
        self.path = path
        self.detector = detector

    def u32(self, o: int) -> int:
        return struct.unpack_from("<I", self.b, o)[0]

    def u64(self, o: int) -> int:
        return struct.unpack_from("<Q", self.b, o)[0]

    def block_ok(self, at: int, detector: "Detector | None" = None) -> bool:
        """Is there a well-formed output block at `at`, per `detector`?

        Under `SHIPPED` this is a faithful port of
        `desert_core::gimmick::block_ok`, both equality clauses included: the
        two copies of the item id must agree, and so must `PAD_AT` and
        `PAD_TAIL_AT`. Under `LOOSE` the second clause is dropped and nothing
        else changes. Which clause buys what, and why `PAD_TAIL_AT` is not
        really a pad, is on `Detector` and `ENTRY_KEY_AT`.
        """
        d = detector or self.detector
        b = self.b
        if at + BLOCK > len(b):
            return False
        if b[at] != 1 or b[at + 58] != 0xFF or b[at + 59] != 0xFF:
            return False
        if (b[at + ITEM_AT:at + ITEM_AT + 4]
                != b[at + ITEM_TAIL_AT:at + ITEM_TAIL_AT + 4]):
            return False
        if d.pad_clause and (b[at + PAD_AT:at + PAD_AT + 4]
                             != b[at + PAD_TAIL_AT:at + PAD_TAIL_AT + 4]):
            return False
        mn, mx = self.u64(at + MIN_AT), self.u64(at + MAX_AT)
        return 1 <= mn <= mx <= MAX_QTY

    def output_lists(self, detector: "Detector | None" = None
                     ) -> "list[tuple[int, int]]":
        """Greedy, non-overlapping walk for `u32 count` + count blocks."""
        d = detector or self.detector
        out, i, n = [], 0, len(self.b)
        while i + 4 <= n:
            c = self.u32(i)
            if (1 <= c <= MAX_COUNT and i + 4 + c * BLOCK <= n
                    and all(self.block_ok(i + 4 + k * BLOCK, d)
                            for k in range(c))):
                out.append((i, c))
                i += 4 + c * BLOCK
            else:
                i += 1
        return out

    def strings(self) -> "dict[int, str]":
        """Every length-prefixed, NUL-terminated printable string in the body."""
        b, n, found = self.b, len(self.b), {}
        for m in re.finditer(rb"[\x20-\x7E]{3,120}", b):
            g = m.group()
            for rel in range(len(g) - 2):
                o = m.start() + rel
                if o < 8:
                    continue
                ln = self.u32(o - 4)
                if ln < 3 or o + ln > n or b[o + ln] != 0:
                    continue
                cand = b[o:o + ln]
                if all(0x20 <= ch <= 0x7E for ch in cand):
                    found[o] = cand.decode()
        return found

    def records(self) -> "list[tuple[int, int, str]]":
        """`(header offset, key, name)` for every record, echo-discriminated.

        The echo is what separates a record header from a nested string field:
        nested fields use the same `u32 len, bytes, NUL` shape, so a plain
        backwards scan finds `NatureBuffTrigger` with the bogus key 16777216
        where `firewood_0001` should be. A real record repeats its own key just
        before a later digits-only id sub-field.
        """
        strs = self.strings()
        soff = sorted(strs)
        out = []
        for o in soff:
            name = strs[o]
            if name.isdigit() or any(c in BAD_NAME_CHARS for c in name):
                continue
            key = self.u32(o - 8)
            if key < MIN_RECORD_KEY:
                continue
            j = bisect.bisect_right(soff, o)
            for o2 in soff[j:j + ECHO_LOOKAHEAD]:
                if o2 - o > ECHO_WINDOW:
                    break
                if strs[o2].isdigit() and self.u32(o2 - 8) == key:
                    out.append((o - 8, key, name))
                    break
        out.sort()
        return out


# ------------------------------------------------------------------- collect.rs

COLLECT_ROW = re.compile(r"^\s*\((\d+),\s*\"([^\"]+)\",\s*Family::(\w+)\)")


def read_collect(path: Path) -> "dict[int, tuple[str, str]]":
    """`record key -> (record name, Family)` from the generated key table."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as e:
        die(f"cannot read {path}: {e}")
    rows = {}
    for line in text.splitlines():
        m = COLLECT_ROW.match(line)
        if m:
            rows[int(m.group(1))] = (m.group(2), m.group(3))
    if not rows:
        die(f"no COLLECT_RECORDS rows parsed out of {path}")
    return rows


# -------------------------------------------------------------- name inference

def phrase_of(name: str) -> "list[str]":
    """A record name reduced to the tokens that might name the goods."""
    toks = [t for t in re.split(r"[_\-]+", name.lower()) if t]
    keep = [t for t in toks if t not in NOISE and not NOISE_RE.match(t)]
    return keep or toks


def infer_names(sources: "dict[int, list[dict]]") -> "dict[int, dict]":
    """Infer one name per item from the names of the records that yield it.

    A token that shows up in the records of many *different* items names the
    container, not the goods (`food` across a dozen `dropset_food_*` props), so
    the winning record is the one whose most generic token is least generic -
    plain inverse document frequency over items. Ties go to the shorter phrase,
    then to the phrase more records share, which is what makes `stalactite`
    (two records) beat `stalactites` (one).
    """
    df = collections.Counter()
    for item, srcs in sources.items():
        seen = set()
        for s in srcs:
            seen.update(phrase_of(s["name"]))
        for t in seen:
            df[t] += 1
    generic = len(sources) + 1          # worse than any real token can score
    for t in list(df):
        if CONTAINER_TOKEN_RE.search(t):
            df[t] = generic

    out = {}
    for item, srcs in sources.items():
        names = sorted({s["name"] for s in srcs})
        phrases = {n: phrase_of(n) for n in names}
        share = collections.Counter("_".join(p) for p in phrases.values())

        def rank(n):
            p = phrases[n]
            # Least generic token first; then the shortest phrase; then the
            # least decorated record name, so the game's own bare `peony_01`
            # beats `gimmick_equip_gimmick_Ssari_Mushroom_backpack`; then the
            # phrase more records share, which is what makes `stalactite` (two
            # records) beat `stalactites` (one).
            dropped = len(re.split(r"[_\-]+", n)) - len(p)
            return (max(df[t] for t in p), len(p), dropped,
                    -share["_".join(p)], len("_".join(p)), n)

        best = min(names, key=rank)
        subject = set(phrases[best])
        agree = [n for n in names if subject & set(phrases[n])]
        conflict = [n for n in names if n not in agree]
        inferred = "_".join(phrases[best])

        # Honesty rules, in order. A container that names the goods after
        # itself is a guess however many copies of it there are, and a subject
        # the minority of records agree on is not "strong" just because two of
        # them said it.
        if CONTAINER_RE.search(best):
            conf = "guess"
        elif len(agree) >= 2 and len(conflict) <= len(agree):
            conf = "strong"
        elif len(agree) >= 2:
            conf = "split"
        elif conflict:
            conf = "weak"
        else:
            conf = "single"
        note = None
        if item in CURATED:
            inferred, note = CURATED[item]
            conf = "curated"
        out[item] = {
            "name": inferred,
            "confidence": conf,
            "named_by": best,
            "agreeing_records": sorted(agree),
            "conflicting_records": sorted(conflict),
            "note": note,
        }
    return out


CONFIDENCE_DOC = {
    "curated": "identity established by evidence outside the name inference; "
               "see the note",
    "strong": "two or more independent records agree on the subject word, and "
              "they are the majority of this item's records",
    "single": "one record names it and nothing contradicts",
    "split": "two or more records agree, but more records of this item name "
             "something else - read the source records before trusting it",
    "weak": "one record names it and other records of this item disagree",
    "guess": "named only by a generic container, which legitimately holds "
             "varied goods",
}


# ------------------------------------------------------------------ the dataset

def build(table_path: Path, detector: Detector = SHIPPED,
          quiet: bool = False) -> dict:
    loose = detector is LOOSE
    t = Table(table_path, detector)
    lists = t.output_lists()
    # A loose build walks the body twice, because "which of these lists can the
    # plugin actually see?" is the whole question it exists to answer. The
    # shipped offsets are the answer, and they are also what the strict-subset
    # self-check compares against.
    shipped_offsets = ({lo for lo, _ in t.output_lists(SHIPPED)} if loose
                       else {lo for lo, _ in lists})
    recs = t.records()
    starts = [r[0] for r in recs]
    blocks = sum(c for _, c in lists)

    def owner(off: int):
        i = bisect.bisect_right(starts, off) - 1
        return recs[i] if i >= 0 else None

    collect = read_collect(COLLECT_RS)
    sources = collections.defaultdict(list)
    loose_only_lists = []
    unattributed = 0
    for lo, c in lists:
        h = owner(lo)
        if h is None:
            unattributed += 1
            continue
        hdr, key, name = h
        fam = collect.get(key, (None, None))[1]
        vis = "shipped" if lo in shipped_offsets else "loose-only"
        rows = []
        for k in range(c):
            bo = lo + 4 + k * BLOCK
            src = {
                "record_key": key,
                "record_name": name,
                "name": name,
                "min": t.u64(bo + MIN_AT),
                "max": t.u64(bo + MAX_AT),
                "family": fam,
                "record_offset": hdr,
                "block_offset": bo,
                "block_rel": bo - hdr,
                "list_items": c,
            }
            if loose:
                # Only a loose build carries these, so the default outputs stay
                # byte-for-byte what they have always been.
                src["visibility"] = vis
                src["entry_key"] = t.u32(bo + ENTRY_KEY_AT)
                src["pad"] = t.u32(bo + PAD_AT)
                rows.append((t.u32(bo + ITEM_AT), src["entry_key"], src["pad"],
                             src["min"], src["max"]))
            sources[t.u32(bo + ITEM_AT)].append(src)
        if loose and vis == "loose-only":
            loose_only_lists.append({
                "list_offset": lo,
                "record_key": key,
                "record_name": name,
                "family": fam,
                "blocks": c,
                "block_rel": lo - hdr,
                "entries": [
                    {"item": i, "min": mn, "max": mx, "entry_key": ek,
                     "pad": pad,
                     "implausible_id": i > MAX_PLAUSIBLE_ITEM_ID}
                    for i, ek, pad, mn, mx in rows
                ],
            })

    named = infer_names(sources)
    # The name inference weighs a token by how many *different* items mention
    # it, so a wider corpus can hand an id a different name: with the chests in
    # scope, item 53 stops being "itembox". Re-running the inference over the
    # shipped sources alone is what lets an entry say which name the default
    # doc gives it, instead of quietly disagreeing with that file.
    shipped_named = {}
    if loose:
        only_shipped = {i: [s for s in srcs if s["visibility"] == "shipped"]
                        for i, srcs in sources.items()}
        shipped_named = infer_names({i: v for i, v in only_shipped.items() if v})
    items = {}
    for item, srcs in sources.items():
        fams = sorted({s["family"] for s in srcs if s["family"]})
        recs_by_key = {}
        for s in srcs:
            e = recs_by_key.setdefault(s["record_key"], {
                "key": s["record_key"], "name": s["record_name"],
                "family": s["family"], "min": s["min"], "max": s["max"],
                "blocks": 0, "record_offset": s["record_offset"],
                "block_rel": [],
            })
            e["blocks"] += 1
            e["min"] = min(e["min"], s["min"])
            e["max"] = max(e["max"], s["max"])
            e["block_rel"].append(s["block_rel"])
            if loose:
                e.setdefault("visibility", s["visibility"])
                e.setdefault("entry_keys", [])
                if s["entry_key"] not in e["entry_keys"]:
                    e["entry_keys"].append(s["entry_key"])
        info = dict(named[item])
        info.update({
            "id": item,
            "blocks": len(srcs),
            "records": len(recs_by_key),
            "min": min(s["min"] for s in srcs),
            "max": max(s["max"] for s in srcs),
            "fixed_amount": all(s["min"] == s["max"] for s in srcs),
            "families": fams,
            "classified": bool(fams),
            "sources": sorted(recs_by_key.values(), key=lambda r: r["name"].lower()),
        })
        if loose:
            shipped_blocks = sum(1 for s in srcs if s["visibility"] == "shipped")
            # An item is "shipped" as soon as **one** block of it is in the
            # population the plugin sees: the gatherer would reach it there.
            # "loose-only" means no block of it is, i.e. the mod is blind to
            # this id entirely.
            info["visibility"] = "shipped" if shipped_blocks else "loose-only"
            info["shipped_blocks"] = shipped_blocks
            info["loose_only_blocks"] = len(srcs) - shipped_blocks
            info["implausible_id"] = item > MAX_PLAUSIBLE_ITEM_ID
            other = shipped_named.get(item, {}).get("name")
            if other and other != info["name"]:
                info["name_in_default_doc"] = other
        items[item] = info

    # An inferred name landing on two ids is the interesting case, not a bug:
    # the game gives the same substance a different id per source (foraged
    # ginseng vs the trade prop). Make it visible on both entries.
    by_name = collections.defaultdict(list)
    for item, info in items.items():
        by_name[info["name"]].append(item)
    for name, ids in by_name.items():
        if len(ids) > 1:
            for i in ids:
                items[i]["name_shared_with"] = sorted(x for x in ids if x != i)

    # One extra honesty rule, and it applies to loose-only items only - both
    # because the default outputs must not move and because this is the shape
    # the loose-only records actually have. `Action_dig_01` yields two ids and
    # `gimmick_Dig_land_0001` five; the inference calls all seven "action_dig"
    # or "dig_land", each `single` because nothing contradicts it. Nothing
    # contradicts it because nothing else mentions them at all. A name one
    # record hands to several different ids names **the source**, not the
    # goods, so it is a guess whatever the agreement count says.
    if loose:
        for info in items.values():
            if (info["visibility"] == "loose-only"
                    and info.get("name_shared_with")
                    and info["confidence"] not in ("curated", "guess")):
                info["confidence_before_loose_rule"] = info["confidence"]
                info["confidence"] = "guess"
                info["confidence_downgraded"] = (
                    f"the records naming this id name "
                    f"{len(info['name_shared_with'])} other id"
                    f"{'s' if len(info['name_shared_with']) > 1 else ''} the "
                    "same way, so the name is the source and not the goods")

    checks = run_checks(t, lists, blocks, recs, items, owner,
                        detector, shipped_offsets, loose_only_lists)
    covered_blocks = sum(1 for lo, c in lists for _ in range(c)
                         if (h := owner(lo)) and h[1] in collect)
    covered_records = len({h[1] for lo, _ in lists
                           if (h := owner(lo)) and h[1] in collect})
    data = {
        "source": str(table_path),
        "source_bytes": len(t.b),
        "generator": "tools/items.py",
        "method": (
            "Blocks found by the desert_core::gimmick::output_lists detector; "
            "item id read at block+1 (echoed at +60). Blocks attributed to "
            "records by the key-echo discriminator. Item NAMES are INFERRED "
            "from the names of the records that yield them - the game's own "
            "item names live in the .paz archives and are not read here."
        ),
        "confidence_levels": CONFIDENCE_DOC,
        "totals": {
            "output_lists": len(lists),
            "blocks": blocks,
            "records_in_table": len(recs),
            "records_with_output": len({h[1] for lo, _ in lists
                                        if (h := owner(lo))}),
            "distinct_items": len(items),
            "unattributed_lists": unattributed,
            "blocks_covered_by_collect_rs": covered_blocks,
            "records_covered_by_collect_rs": covered_records,
            "items_classified": sum(1 for i in items.values() if i["classified"]),
            "items_unclassified": sum(1 for i in items.values()
                                      if not i["classified"]),
        },
        "checks": checks,
        "items": [items[k] for k in sorted(items)],
    }
    if loose:
        # Everything below is loose-only, and so is every extra field on an
        # item and on a source. The default build writes none of it, which is
        # what keeps analysis/items.json byte-identical to what it always was.
        lo_items = [i for i in items.values() if i["visibility"] == "loose-only"]
        bad = sorted(i["id"] for i in items.values() if i["implausible_id"])
        data["detector"] = {
            "name": detector.name,
            "summary": detector.summary,
            "pad_clause": detector.pad_clause,
        }
        data["warning"] = (
            "LOOSE POPULATION. This file is the wider walk: the pad clause of "
            "desert_core::gimmick::block_ok is dropped, so it contains content "
            "the plugin CANNOT SEE. Any item or source marked "
            "visibility=loose-only is invisible to the gatherer and no "
            "multiplier reaches it. It is also less trustworthy than "
            "analysis/items.json: see implausible_item_ids below, and "
            "docs/reference-items-loose.md for the caveats in full."
        )
        data["totals"].update({
            "shipped_lists": len(shipped_offsets),
            "loose_only_lists": len(loose_only_lists),
            "loose_only_blocks": sum(l["blocks"] for l in loose_only_lists),
            "items_shipped_visible": len(items) - len(lo_items),
            "items_loose_only": len(lo_items),
            "implausible_item_ids": len(bad),
        })
        data["implausible_item_ids"] = {
            "ids": bad,
            "note": (
                "Every id in the shipped population is eight digits or fewer "
                f"(largest 10021720). These are past {MAX_PLAUSIBLE_ITEM_ID:,} "
                "and are reported, not filtered: they mean the list they sit "
                "in is being mis-parsed. Every one of them is inside "
                f"{IMPLAUSIBLE_HOME}, so the damage is known to be local - but "
                "it is proof the loose walk is not uniformly right, and no id "
                "that only this walk can see should be used without checking "
                "the record it came from."
            ),
            "records": sorted({s["name"] for i in items.values()
                               if i["implausible_id"] for s in i["sources"]}),
        }
        data["loose_confidence_rule"] = (
            "A loose-only item whose inferred name is shared with another id "
            "is forced to `guess`, whatever its agreement count: a name one "
            "record hands to several ids names the source and not the goods. "
            "The pre-rule level is kept as confidence_before_loose_rule. This "
            "rule is not applied to the shipped population, which is why "
            "analysis/items.json is unchanged by it."
        )
        data["loose_only_lists"] = sorted(loose_only_lists,
                                          key=lambda l: l["record_name"].lower())
    if not quiet:
        for line in checks:
            print(line)
    return data


def run_checks(t, lists, blocks, recs, items, owner, detector=SHIPPED,
               shipped_offsets=None, loose_only_lists=()) -> "list[str]":
    """Self-checks against the numbers findings §9 confirmed. Never fatal:
    a FAIL is the report, and it means the table or the walk moved.

    The loose walk gets the same treatment - its own three counts, plus the two
    relations that make it a *superset* of the shipped one rather than a
    different answer to the same question."""
    loose = detector is LOOSE
    out = []

    def chk(label, got, want):
        ok = got == want
        out.append(f"{'ok  ' if ok else 'FAIL'} {label}: {got}"
                   + ("" if ok else f" (expected {want})"))
        return ok

    def note(ok, label, detail):
        out.append(f"{'ok  ' if ok else 'FAIL'} {label}: {detail}")
        return ok

    if loose:
        # Only the loose run announces its detector: the default run's check
        # lines are copied verbatim into docs/reference-items.md, and that file
        # is meant to stay byte-identical to what it has always been.
        out.append(f"--   detector: {detector.name} ({detector.summary})")
    chk("output lists", len(lists),
        EXPECT_LOOSE_LISTS if loose else EXPECT_LISTS)
    chk("blocks", blocks, EXPECT_LOOSE_BLOCKS if loose else EXPECT_BLOCKS)
    chk("distinct item ids", len(items),
        EXPECT_LOOSE_ITEMS if loose else EXPECT_ITEMS)
    chk("records", len(recs), EXPECT_RECORDS)
    if loose:
        offs = {lo for lo, _ in lists}
        shipped_offsets = shipped_offsets or set()
        missing = sorted(shipped_offsets - offs)
        note(not missing and len(shipped_offsets) == EXPECT_LISTS,
             "shipped lists are a strict subset",
             f"{len(shipped_offsets)}/{len(offs)} shipped lists all present"
             + ("" if not missing else
                f"; {len(missing)} MISSING, first at {missing[0]}"))
        chk("loose-only lists", len(loose_only_lists), EXPECT_LOOSE_ONLY_LISTS)
        chk("loose-only blocks", sum(l["blocks"] for l in loose_only_lists),
            EXPECT_LOOSE_ONLY_BLOCKS)
        # The discriminator itself: every extra block carries a nonzero entry
        # key at +64 and a zero pad at +5. If that ever stops holding, the two
        # populations differ for some other reason and nothing below is safe.
        ents = [e for l in loose_only_lists for e in l["entries"]]
        nz = sum(1 for e in ents if e["entry_key"] != 0)
        note(nz == len(ents) and len(ents) == EXPECT_LOOSE_ONLY_BLOCKS,
             "every loose-only block has a nonzero +64",
             f"{nz}/{len(ents)}, {len({e['entry_key'] for e in ents})} distinct")
        pads = sum(1 for lo, c in lists for k in range(c)
                   if t.u32(lo + 4 + k * BLOCK + PAD_AT) == 0)
        note(pads == blocks, "+5 is zero on every block of both populations",
             f"{pads}/{blocks}")
        chk("items only this walk can see",
            sum(1 for e in items.values() if e["visibility"] == "loose-only"),
            EXPECT_LOOSE_ONLY_ITEMS)
        lo_ids = {e["item"] for l in loose_only_lists for e in l["entries"]}
        shared = sorted(i for i in lo_ids
                        if items[i]["visibility"] == "shipped")
        note(shared == EXPECT_SHARED_IDS,
             "ids in loose-only blocks that the mod also sees",
             f"{len(shared)} of {len(lo_ids)}: {shared}"
             + ("" if shared == EXPECT_SHARED_IDS
                else f" (expected {EXPECT_SHARED_IDS})"))
        bad = sorted(i for i, e in items.items()
                     if e.get("implausible_id"))
        homes = sorted({s["name"] for i in bad for s in items[i]["sources"]})
        note(len(bad) == EXPECT_IMPLAUSIBLE_IDS and homes == [IMPLAUSIBLE_HOME],
             "implausible item ids are confined to one list",
             f"{len(bad)} ids in {homes or ['none']}"
             + (f" (expected {EXPECT_IMPLAUSIBLE_IDS} in "
                f"{IMPLAUSIBLE_HOME})"
                if len(bad) != EXPECT_IMPLAUSIBLE_IDS
                or homes != [IMPLAUSIBLE_HOME] else ""))
    for off, name, rel in ANCHORS:
        h = owner(off)
        got = f"{h[2]} rel +{off - h[0]}" if h else "unattributed"
        want = f"{name} rel +{rel}"
        out.append(f"{'ok  ' if got == want else 'FAIL'} anchor list@{off}: "
                   f"{got}" + ("" if got == want else f" (expected {want})"))
    bad = []
    for item, want in ANCHOR_NAMES.items():
        got = items.get(item, {}).get("name")
        if got != want:
            bad.append(f"{item} -> {got!r} (expected {want!r})")
    out.append(f"{'ok  ' if not bad else 'FAIL'} anchor names: "
               f"{len(ANCHOR_NAMES) - len(bad)}/{len(ANCHOR_NAMES)} agree"
               + ("" if not bad else "; " + ", ".join(bad)))
    return out


# ----------------------------------------------------------------- the doc

def render_doc(d: dict) -> str:
    # A loose build is the one that carries a `detector` block. Every addition
    # below is behind this flag, because docs/reference-items.md is meant to
    # stay byte-identical to what it has always been.
    loose = bool(d.get("detector"))
    tot = d["totals"]
    items = d["items"]
    by_fam = collections.defaultdict(list)
    for it in items:
        by_fam["/".join(it["families"]) if it["families"] else ""].append(it)
    shared = sorted((it for it in items if it.get("name_shared_with")),
                    key=lambda i: (i["name"], i["id"]))

    L = []
    a = L.append
    if loose:
        a("# Item cross-reference - **LOOSE** detector")
        a("")
        a("**Generated by `tools/items.py --loose`. Do not edit.** Regenerate")
        a("with `python3 tools/items.py --loose`; the machine-readable form is")
        a("`analysis/items-loose.json`. Both are gitignored, like the rest of")
        a("`docs/` and `analysis/`.")
        a("")
        a("> ## Read this first")
        a(">")
        a("> **This file is not the mod's view of the game.** It is the wider")
        a("> walk: the pad clause of `desert_core::gimmick::block_ok` is")
        a("> dropped, so it contains content the plugin **cannot see** and no")
        a("> multiplier reaches. The mod's own view is")
        a("> `docs/reference-items.md`, and that is the file to use for")
        a("> anything the gatherer acts on.")
        a(">")
        a("> **It is also less trustworthy than that file, in two specific")
        a("> ways, and neither is hypothetical:**")
        a(">")
        bad = d.get("implausible_item_ids", {})
        ids = bad.get("ids", [])
        a(f"> 1. **{len(ids)} of the item ids here are implausible** -")
        a(f">    `{ids[0] if ids else '?'}`..`{ids[-1] if ids else '?'}`, nine")
        a(">    digits where every id in the shipped population is eight or")
        a(">    fewer (largest `10021720`). All of them sit in the single list")
        a(f">    `{IMPLAUSIBLE_HOME}`, so **at least that")
        a(">    one list is being mis-parsed.** They are left in and flagged,")
        a(">    never filtered out: the flag is the finding.")
        lo_only = [i for i in items if i["visibility"] == "loose-only"]
        a(f"> 2. **Corroboration is thin.** {EXPECT_LOOSE_ONLY_BLOCKS} blocks")
        a(">    are new, carrying 101 distinct ids, and only")
        a(f">    **{len(EXPECT_SHARED_IDS)}** of those 101 (`"
          + "`, `".join(str(i) for i in EXPECT_SHARED_IDS) + "`) also occur")
        a(">    in the population the plugin sees. A low overlap is what a")
        a(">    *correct* walk would produce too - chests yield different")
        a(">    things from gather nodes - so it is not evidence against these")
        a(">    blocks. It is simply not evidence for them either. Nothing")
        a(">    here is cross-checked the way the default population is.")
        a(">")
        a(f"> {len(lo_only)} of the {len(items)} items below are marked")
        a("> **`loose-only`**. That marking is the point of this file: an item")
        a("> the mod cannot see must never be mistaken for one it can.")
        a("")
    else:
        a("# Item cross-reference for the `gimmickinfo` output blocks")
        a("")
        a("**Generated by `tools/items.py`. Do not edit.** Regenerate with")
        a("`python3 tools/items.py`; the machine-readable form is")
        a("`analysis/items.json`. Both are gitignored, like the rest of `docs/`")
        a("and `analysis/`.")
        a("")
    a(f"Source: `{d['source']}` ({d['source_bytes']:,} bytes), the clean")
    a("`gimmickinfo` table body DMM writes out. Record-relative offsets only,")
    a("so this needs no rebasing for a game update.")
    a("")
    a("## What this is, and what it is not")
    a("")
    a("Every gather yield in the game is a **resource-output block** inside a")
    a("`gimmickinfo` record, and every block names an **item id**. This is the")
    a("index from item id back to what it is and where it comes from - the")
    a("thing that was missing when the gatherer multiplied yields by record")
    a("with no way to ask what a record was handing out.")
    a("")
    a("**The item names here are inferred, not read.** The game's own item")
    a("names live in the `.paz` archives and nothing extracts them. What this")
    a("does instead is read the name of the *record that yields the item*:")
    a("`gimmick_item_trade_salt_02` yields `1000648`, so `1000648` is salt")
    a("(`docs/findings-water-wells-2026-09-12.md` section 3). It works because")
    a("most items are placed by several independent props whose names agree,")
    a("and the `confidence` column says how much agreement there was. Treat a")
    a("`guess` as a hypothesis and nothing more.")
    a("")
    a("Method and its known failure modes:")
    a("")
    if loose:
        a("* Blocks come from the plugin's detector")
        a("  (`desert_core::gimmick::output_lists`) **with its pad clause")
        a("  removed**: a `u32 count` then `count` 68-byte blocks, walked")
        a("  greedily and non-overlapping, item id read at **`block+1`** and")
        a("  required to match its copy at `+60`. The shipped detector also")
        a("  requires `+5` to equal `+64`. Dropping that one clause takes the")
        a(f"  walk from 573 lists / 896 blocks / 215 items to "
          f"**{tot['output_lists']} /")
        a(f"  {tot['blocks']} / {tot['distinct_items']}**, and the 573 stay a "
          "strict subset.")
        a("* **`+64` is not a pad.** `+5` is zero on all 1038 blocks of both")
        a("  populations, so that half of the clause is vacuous; `+64` is zero")
        a("  on all 896 shipped blocks and **nonzero on all 142 extras** (95")
        a("  distinct values). The deserialiser settles what it is:")
        a("  `FUN_141a37180`, the block parser, consumes exactly `+0..+63` and")
        a("  never reads these four bytes, while `FUN_1414a7cc0`, the list")
        a("  loop, reads them after each block and stores them at `entry+0x08`")
        a("  of its `0x10`-stride entries. So `+64` is **the list entry's own")
        a("  key field**, which gather records leave zero and these records")
        a("  populate. It is not the high half of a wide item id, and it is")
        a("  not the variant tag at `+9` - both populations carry tags 0 and 4.")
        a("* So the clause the plugin keeps is not separating well-formed")
        a("  blocks from noise. It is separating *records whose list entries")
        a("  are keyed* from records whose are not, and the second set happens")
        a("  to be the gather nodes. Whether the first set ought to be")
        a("  multiplied is a **policy** question this file does not answer.")
    else:
        a("* Blocks come from the same detector the plugin uses")
        a("  (`desert_core::gimmick::output_lists`): a `u32 count` then `count`")
        a("  68-byte blocks, walked greedily and non-overlapping. The item id is")
        a("  read at **`block+1`**, echoed at `+60` - not `+5`/`+64`, which are")
        a("  zero padding on all 896 blocks (findings section 5). Both of the")
        a("  detector's equality clauses are kept, the pad one included: without it")
        a("  the population grows to 589 lists / 1038 blocks, which is where an")
        a("  earlier overcount came from.")
    a("* A block is attributed to its record by the **key echo**: a real record")
    a("  header repeats its own `u32` key just before a later digits-only id")
    a("  sub-field. Nested string fields use the same `u32 len, bytes, NUL`")
    a("  shape as record names, so a plain backwards scan mis-attributes -")
    a("  `key = 16777216` is the usual false positive (findings section 7).")
    a("* The name inference weights a token by how many *different* items its")
    a("  records mention it for, so a container word (`food` across a dozen")
    a("  `dropset_food_*` props) loses to the goods word. That is what stops")
    a("  every dropset item being called \"food\". Ties go to the least")
    a("  decorated record name, so the game's own bare `peony_01` wins over")
    a("  `gimmick_equip_gimmick_Peony_backpack`.")
    a("* Amounts are **vanilla**, straight out of the clean body. They are the")
    a("  `u64` min at `block+42` and max at `block+50`, the pair")
    a("  `gimmick::multiply` scales.")
    a("")
    a("## Summary")
    a("")
    a("| | |")
    a("| --- | --- |")
    a(f"| output lists | {tot['output_lists']} |")
    a(f"| output blocks | {tot['blocks']} |")
    a(f"| records in the table | {tot['records_in_table']:,} |")
    a(f"| records with an output list | {tot['records_with_output']} |")
    a(f"| **distinct item ids** | **{tot['distinct_items']}** |")
    a(f"| blocks covered by `collect.rs` today | "
      f"{tot['blocks_covered_by_collect_rs']} |")
    a(f"| records covered by `collect.rs` today | "
      f"{tot['records_covered_by_collect_rs']} |")
    a(f"| items with a `Family` | {tot['items_classified']} |")
    a(f"| **items with no `Family`** | **{tot['items_unclassified']}** |")
    a(f"| lists that could not be attributed | {tot['unattributed_lists']} |")
    if loose:
        a(f"| of those lists, ones the plugin also sees | "
          f"{tot['shipped_lists']} |")
        a(f"| **lists only this walk sees** | **{tot['loose_only_lists']}** |")
        a(f"| **blocks only this walk sees** | **{tot['loose_only_blocks']}** |")
        a(f"| items the plugin can see | {tot['items_shipped_visible']} |")
        a(f"| **items only this walk sees** | **{tot['items_loose_only']}** |")
        a(f"| **implausible item ids** | **{tot['implausible_item_ids']}** |")
    a("")
    if loose:
        a("Self-checks from this run. The first four numbers and the three")
        a("anchors are findings section 9's, confirmed three independent ways;")
        a("the loose-specific ones below them are this walk's own and have had")
        a("no such treatment - they are recorded so that a *move* in them is")
        a("noticed, which is a weaker claim than the default file's checks make.")
        a("A `FAIL` means the table or the walk moved:")
        a("")
    else:
        a("Self-checks from this run (findings section 9 confirmed each of these")
        a("three independent ways; a `FAIL` means the table or the walk moved):")
        a("")
    a("```")
    for line in d["checks"]:
        a(line)
    a("```")
    a("")
    a("Confidence levels:")
    a("")
    a("| level | meaning |")
    a("| --- | --- |")
    for k in ("curated", "strong", "single", "split", "weak", "guess"):
        a(f"| `{k}` | {CONFIDENCE_DOC[k]} |")
    a("")
    if loose:
        n = sum(1 for it in items if it.get("confidence_downgraded"))
        a(f"**One extra rule applies here and nowhere else**, to {n} items.")
        a(d["loose_confidence_rule"])
        a("It is what stops the seven ids of `Action_dig_01` and")
        a("`gimmick_Dig_land_0001` reading as `single`-confidence items called")
        a("\"action_dig\" and \"dig_land\" - which is what they would otherwise")
        a("be, because nothing contradicts a name nothing else ever mentions.")
        a("")
        drift = [it for it in items if it.get("name_in_default_doc")]
        a("**Names here can disagree with `docs/reference-items.md`.** The")
        a("inference weighs a token by how many *different* items mention it,")
        a("so a wider corpus can hand an id a different name. Any entry that")
        a("moved carries `name_in_default_doc` in the JSON, and this run has")
        if drift:
            a(f"{len(drift)}: "
              + ", ".join(f"`{it['id']}` is **{it['name']}** here and "
                          f"*{it['name_in_default_doc']}* there"
                          for it in drift) + ".")
        else:
            a("none.")
        a("")
    counts = collections.Counter(i["confidence"] for i in items)
    a("This run: " + ", ".join(f"{counts[k]} `{k}`" for k in
                               ("curated", "strong", "single", "split",
                                "weak", "guess")
                               if counts[k]) + ".")
    a("")

    a("## Read this before using an id")
    a("")
    for item in (1, 22008):
        it = next((x for x in items if x["id"] == item), None)
        if it:
            a(f"**Item `{item}` - {it['name']}.** {it['note']}")
            a("")
    a("**The same substance can have several ids, and the id is what the")
    a("multiplier keys on.** The game gives an item a different id depending on")
    a("how you obtain it, which is exactly why an item-keyed family can never")
    a("reach into another one by accident. The load-bearing case for this")
    a("project: **foraged ginseng is `756513` and `1000142`, both already")
    a("`Foraging`, while the trade prop is `1000666` and is in no family.** A")
    a("`Foraging` slider moves the first two and not the third; they are not")
    a("the same item as far as the table is concerned.")
    a("")
    a("Below is every inferred name that landed on more than one id. **Two very")
    a("different things look the same here and the table cannot tell them")
    a("apart** - read it as a list of things to check, not a list of findings:")
    a("")
    a("* the game really does use several ids for one substance obtained")
    a("  differently (`ginseng`) or at different grades (`rock` at `1..5`,")
    a("  `1..10` and `1..5`; firewood's three grades are `710001`, `1000081`")
    a("  and `1000489`, which keep their `_fine`/`_premium` names here and so")
    a("  do not show up below), and")
    a("* the record names are simply too coarse to separate genuinely distinct")
    a("  items - the eleven `Item_Collect_Hang_Meat_*` props are eleven")
    a("  different cuts of meat with nothing in their names to tell you which.")
    a("")
    a("A group of two or three is usually the first kind; a group of seven or")
    a("more is almost always the second.")
    a("")
    if shared:
        a("| inferred name | n | ids | already in a family |")
        a("| --- | --- | --- | --- |")
        seen = set()
        rows = []
        for it in shared:
            grp = tuple(sorted([it["id"]] + it["name_shared_with"]))
            if grp in seen:
                continue
            seen.add(grp)
            fams = [f"`{i}` {'/'.join(e['families'])}" for i in grp
                    if (e := next(x for x in items if x["id"] == i))["families"]]
            rows.append((len(grp), it["name"], grp, fams))
        for n, name, grp, fams in sorted(rows):
            a(f"| {name} | {n} | " + ", ".join(f"`{i}`" for i in grp)
              + " | " + (", ".join(fams) if fams else "none") + " |")
    else:
        a("_none in this run._")
    a("")
    a("**A single-item list is the norm, so multiplying a record usually")
    a("multiplies exactly one item.** Where a record's list mixes items the")
    a("`sources` entries in "
      + ("`analysis/items-loose.json`" if loose else "`analysis/items.json`")
      + " carry `list_items` so you can")
    a("see it.")
    a("")

    def mark(it) -> str:
        """The visibility cell. Loud on purpose: this column is the one thing
        a reader of this file must not skim past."""
        if it["visibility"] == "shipped":
            return "shipped"
        return "**LOOSE-ONLY**" + (" ⚠id" if it["implausible_id"] else "")

    def item_table(rows):
        if loose:
            a("| item | inferred name | amount | conf | visible to the mod | "
              "records | source records |")
            a("| --- | --- | --- | --- | --- | --- | --- |")
        else:
            a("| item | inferred name | amount | conf | records | source records |")
            a("| --- | --- | --- | --- | --- | --- |")
        for it in rows:
            amt = (f"{it['min']}" if it["min"] == it["max"]
                   else f"{it['min']}..{it['max']}")
            names = ", ".join(f"`{s['name']}`" for s in it["sources"][:6])
            if len(it["sources"]) > 6:
                names += f" _+{len(it['sources']) - 6} more_"
            vis = f"{mark(it)} | " if loose else ""
            a(f"| `{it['id']}` | {it['name']} | {amt} | `{it['confidence']}` "
              f"| {vis}{it['records']} | {names} |")
        a("")

    if loose:
        lo_lists = d["loose_only_lists"]
        by_id = {it["id"]: it for it in items}
        a(f"## The {len(lo_lists)} lists only this walk sees")
        a("")
        a("These are the whole difference between the two detectors. **None of")
        a("them is one of the 275 gather records the DMM pack edits**, and no")
        a("gatherer slider reaches any of them today: they are chests, dig")
        a("sites, dungeon loot, a claw machine and the Marni teleporters.")
        a("`+64` is the list entry's key field described above, and it is")
        a("nonzero on every block here - that is exactly what excludes them.")
        a("")
        a("| record | key | blocks | `+64` values | family |")
        a("| --- | --- | --- | --- | --- |")
        for l in lo_lists:
            ks = [e["entry_key"] for e in l["entries"]]
            uniq = sorted(set(ks))
            shown = ", ".join(f"`{k}`" for k in uniq[:3])
            if len(uniq) > 3:
                shown += f" _+{len(uniq) - 3} more_"
            dup = "" if len(uniq) == len(ks) else f" ({len(uniq)} distinct)"
            a(f"| `{l['record_name']}` | {l['record_key']} | {l['blocks']} | "
              f"{shown}{dup} | {l['family'] or '-'} |")
        a("")
        a("Every block of every one of them, in table order. A `⚠` marks an")
        a("item id past " + f"{MAX_PLAUSIBLE_ITEM_ID:,}" + " - see caveat 1 at")
        a("the top of this file.")
        a("")
        for l in lo_lists:
            a(f"### `{l['record_name']}` - key {l['record_key']}, "
              f"{l['blocks']} blocks")
            a("")
            a("| item | amount | inferred name | conf | `+64` | also seen "
              "by the mod |")
            a("| --- | --- | --- | --- | --- | --- |")
            for e in l["entries"]:
                it = by_id[e["item"]]
                amt = (f"{e['min']}" if e["min"] == e["max"]
                       else f"{e['min']}..{e['max']}")
                flag = " ⚠" if e["implausible_id"] else ""
                seen = ("**yes**" if it["visibility"] == "shipped" else "no")
                a(f"| `{e['item']}`{flag} | {amt} | {it['name']} | "
                  f"`{it['confidence']}` | `{e['entry_key']}` | {seen} |")
            a("")
        # Everything in this paragraph is measured off the tables above, not
        # asserted, because it is the one place the loose walk offers evidence
        # for itself and an overstated version of it would be worth nothing.
        allk = [e["entry_key"] for l in lo_lists for e in l["entries"]]
        in_many = {k for k in set(allk)
                   if len({l["record_name"] for l in lo_lists
                           for e in l["entries"] if e["entry_key"] == k}) > 1}
        seqs = {l["record_name"]: [(e["item"], e["entry_key"])
                                   for e in l["entries"]] for l in lo_lists}
        repeats = []
        for n1, s1 in sorted(seqs.items()):
            for n2, s2 in sorted(seqs.items()):
                if n1 == n2 or len(s1) >= len(s2):
                    continue
                if any(s2[i:i + len(s1)] == s1
                       for i in range(len(s2) - len(s1) + 1)):
                    repeats.append((n1, n2))
        dupes = sorted({(l["record_name"], e["item"])
                        for l in lo_lists for e in l["entries"]
                        if [x["item"] for x in l["entries"]].count(e["item"]) > 1})
        a("Some of what is in those tables is worth not glossing over, and")
        a("most of it cuts in the loose walk's favour.")
        a("")
        a(f"* The `+64` values **repeat**: {len(set(allk))} distinct values")
        a(f"  over {len(allk)} blocks, and {len(in_many)} of them occur in more")
        a("  than one record. Whatever the key means, it is not one per entry,")
        a("  and it is shared *between* records - which is what a reference to")
        a("  a common drop table would look like.")
        same = collections.defaultdict(list)
        for n, s in sorted(seqs.items()):
            same[tuple(s)].append(n)
        twins = [v for v in same.values() if len(v) > 1]
        if twins:
            a("* " + "; ".join(
                f"`{'`, `'.join(g)}` carry **identical** lists"
                for g in twins) + ".")
        if repeats:
            a("* Whole runs of entries repeat verbatim across records, item ids")
            a("  and keys together: "
              + "; ".join(f"every entry of `{n1}` appears in order inside "
                          f"`{n2}`" for n1, n2 in repeats[:3])
              + (f" (and {len(repeats) - 3} more such pairs)"
                 if len(repeats) > 3 else "") + ".")
            a("  Structure that regular is **not** what a misparse of arbitrary")
            a("  bytes produces, so these lists are very likely real. It says")
            a("  nothing about whether the amounts or the ids were read right.")
        if dupes:
            a("* A record can name the same item twice: "
              + ", ".join(f"`{n}` lists `{i}` twice" for n, i in dupes)
              + ", same amount and same key both times. A genuinely repeated")
            a("  entry and a misparse look identical here and nothing in this")
            a("  file settles which it is.")
        a("")

        lo_items = sorted((it for it in items
                           if it["visibility"] == "loose-only"),
                          key=lambda i: (not i["implausible_id"],
                                         -i["blocks"], i["name"], i["id"]))
        a(f"## Loose-only items - {len(lo_items)}")
        a("")
        a("**The mod cannot see a single one of these.** No block of any of")
        a("them is in the population `desert_core::gimmick::block_ok` admits,")
        a("so no multiplier, no `Family` and no slider reaches them, and")
        a("nothing in `docs/reference-items.md` mentions them at all. The")
        a("implausible ids are listed first, because they are the reason to")
        a("distrust the list they came from.")
        a("")
        item_table(lo_items)
        a("The remaining "
          f"{tot['distinct_items'] - len(lo_items)} items below are visible to")
        a("the mod: they also occur in at least one block the shipped detector")
        a("admits. Five of them - `"
          + "`, `".join(str(i) for i in EXPECT_SHARED_IDS)
          + "` - occur in **both** populations, which is the only")
        a("cross-check the loose-only blocks get.")
        a("")

    order = sorted((k for k in by_fam if k), key=str.lower)
    a("## Items by current `Family`")
    a("")
    a("`Family` comes from `desert-core/src/collect.rs`, which is generated")
    a("from the DMM pack. An item lands in a family because *a record that")
    a("yields it* is in that family, so an item can appear under two.")
    a("")
    for fam in order:
        rows = sorted(by_fam[fam], key=lambda i: (i["name"], i["id"]))
        a(f"### `Family::{fam}` - {len(rows)} items")
        a("")
        item_table(rows)

    unc = sorted(by_fam.get("", []), key=lambda i: (i["confidence"] != "curated",
                                                    -i["blocks"], i["name"]))
    a(f"## Unclassified - {len(unc)} items")
    a("")
    a("No record that yields these is in any `Family`, so no gatherer slider")
    a("touches them. **This is where future families come from.** Ordered by")
    a("how many blocks yield the item, i.e. how widely it is placed, so the")
    a("worthwhile candidates are at the top. Remember item `1` is money.")
    if loose:
        n = sum(1 for i in unc if i["visibility"] == "loose-only")
        a("")
        a(f"**{n} of these {len(unc)} are `loose-only`**, and putting one of")
        a("them in a `Family` would do nothing: the plugin's detector never")
        a("sees the block, so there is no yield for a slider to multiply. They")
        a("are candidates for a decision about the *detector*, not for a new")
        a("family. Read the `visible to the mod` column before acting on a row.")
    a("")
    item_table(unc)

    a("## Every item, by id")
    a("")
    a("The flat index, for when you have an id and want the row. `python3")
    a("tools/items.py <id>` prints the same thing with every source record.")
    a("")
    if loose:
        a("| item | inferred name | amount | conf | visible to the mod | "
          "family | blocks | records |")
        a("| --- | --- | --- | --- | --- | --- | --- | --- |")
    else:
        a("| item | inferred name | amount | conf | family | blocks | records |")
        a("| --- | --- | --- | --- | --- | --- | --- |")
    for it in items:
        amt = (f"{it['min']}" if it["min"] == it["max"]
               else f"{it['min']}..{it['max']}")
        vis = f"{mark(it)} | " if loose else ""
        a(f"| `{it['id']}` | {it['name']} | {amt} | `{it['confidence']}` | "
          f"{vis}{'/'.join(it['families']) or '-'} | {it['blocks']} | "
          f"{it['records']} |")
    a("")
    a("## Looking one up")
    a("")
    if loose:
        a("Every lookup takes `--loose` and then reads")
        a("`analysis/items-loose.json` instead of `analysis/items.json`, so it")
        a("answers out of this population. A `loose-only` item simply does not")
        a("exist without the flag.")
        a("")
        a("```")
        a("python3 tools/items.py --loose 1000629            # one item, "
          "every source record")
        a("python3 tools/items.py --loose --record Action_dig_01")
        a("python3 tools/items.py --loose --unclassified     # the list above")
        a("python3 tools/items.py --loose --list             # one line per item")
        a("python3 tools/items.py --loose --family Foraging")
        a("```")
        a("")
        a("Those read `analysis/items-loose.json` and are instant; pass")
        a("`--rescan` to walk the 22 MB body again. A `--loose` build walks it")
        a("**twice** - once each way - because knowing which lists the plugin")
        a("would also have found is the whole question this file answers.")
        a("")
        a("Drop the flag for the mod's own view:")
        a("")
        a("```")
        a("python3 tools/items.py                    # rebuild "
          "docs/reference-items.md")
        a("```")
    else:
        a("```")
        a("python3 tools/items.py 22008              # one item, every source record")
        a("python3 tools/items.py salt               # by inferred name")
        a("python3 tools/items.py --record peony_01  # what a record yields")
        a("python3 tools/items.py --unclassified     # the list above")
        a("python3 tools/items.py --family Foraging")
        a("```")
        a("")
        a("Those read `analysis/items.json` and are instant; pass `--rescan` to")
        a("walk the 22 MB body again (a few seconds).")
    return "\n".join(L) + "\n"


# ----------------------------------------------------------------------- lookup

def load(args) -> dict:
    """The cache for the population `args` asked for, or a fresh walk.

    Each detector has its own file, so `--loose` can never read - or write -
    the default one. A cache whose detector does not match is treated as no
    cache at all rather than as an answer.
    """
    want_loose = args.detector is LOOSE
    if not args.rescan and args.json_out.exists():
        try:
            d = json.loads(args.json_out.read_text(encoding="utf-8"))
            if bool(d.get("detector")) == want_loose:
                return d
        except (OSError, ValueError):
            pass
    return build(args.table, args.detector, quiet=True)


def fmt_amount(it) -> str:
    return (f"{it['min']}" if it["min"] == it["max"]
            else f"{it['min']}..{it['max']}")


def vis_tag(it) -> str:
    """The padded visibility column for a terminal line, **empty string and
    no padding at all** for the default population - every item there is
    visible by construction, and the default CLI output must not move."""
    v = it.get("visibility")
    if v == "loose-only":
        return "LOOSE-ONLY" + ("! " if it.get("implausible_id") else "  ")
    if v == "shipped":
        return "shipped     "
    return ""


def show(it) -> None:
    print(f"item {it['id']}  {it['name']}   [{it['confidence']}] "
          f"{fmt_amount(it)}" + ("  (fixed)" if it["fixed_amount"] else ""))
    if it.get("visibility") == "loose-only":
        print("  VISIBILITY  loose-only: NO block of this item is in the "
              "population the mod sees.")
        print("              No multiplier reaches it. "
              "It is absent from analysis/items.json.")
    elif it.get("visibility") == "shipped":
        print(f"  visibility  shipped ({it['shipped_blocks']} of "
              f"{it['blocks']} blocks; {it['loose_only_blocks']} only the "
              "loose walk sees)")
    if it.get("implausible_id"):
        print(f"  ⚠ ID       past {MAX_PLAUSIBLE_ITEM_ID:,}: the list this "
              "came from is being mis-parsed. Do not use this id.")
    print(f"  family      {'/'.join(it['families']) or 'unclassified'}")
    print(f"  named by    {it['named_by']}  "
          f"({len(it['agreeing_records'])} agreeing, "
          f"{len(it['conflicting_records'])} conflicting)")
    if it.get("confidence_downgraded"):
        print(f"  downgraded  from {it['confidence_before_loose_rule']}: "
              f"{it['confidence_downgraded']}")
    if it.get("name_in_default_doc"):
        print(f"  NB          docs/reference-items.md calls this id "
              f"{it['name_in_default_doc']!r}: the inference sees a wider "
              "corpus here")
    if it.get("note"):
        print(f"  note        {it['note']}")
    if it.get("name_shared_with"):
        print("  same name   " + ", ".join(str(i) for i in it["name_shared_with"]))
    print(f"  {it['blocks']} blocks in {it['records']} records:")
    for s in it["sources"]:
        rel = ",".join(f"+{r}" for r in sorted(s["block_rel"]))
        extra = ""
        if "visibility" in s:
            keys = ",".join(str(k) for k in s["entry_keys"])
            extra = (f"  [{s['visibility']}" +
                     (f", +64={keys}]" if s["visibility"] == "loose-only"
                      else "]"))
        print(f"    {s['key']:>9}  {s['name']:<58} "
              f"{s['min']}..{s['max']:<6} {s['family'] or '-':<9} rel {rel}"
              f"{extra}")


def main() -> None:
    p = argparse.ArgumentParser(
        description="Item cross-reference for the gimmickinfo output blocks.",
        epilog="With no arguments, rebuilds analysis/items.json and "
               "docs/reference-items.md. With --loose and no query, rebuilds "
               "analysis/items-loose.json and docs/reference-items-loose.md "
               "instead; the two pairs are never written by the same run.")
    p.add_argument("query", nargs="?",
                   help="an item id, or part of an inferred or record name")
    p.add_argument("--record", metavar="NAME",
                   help="what the records matching NAME yield")
    p.add_argument("--family", metavar="FAMILY",
                   help="every item of one collect.rs Family")
    p.add_argument("--unclassified", action="store_true",
                   help="items no Family covers")
    p.add_argument("--list", action="store_true", help="one line per item")
    p.add_argument("--loose", action="store_true",
                   help="use the loose detector (item-id clause only, no pad "
                        "clause): 589 lists / 1038 blocks / 311 items instead "
                        "of 573 / 896 / 215. Reads and writes its OWN files, "
                        "analysis/items-loose.json and "
                        "docs/reference-items-loose.md, and marks everything "
                        "the mod cannot see. Less trustworthy than the "
                        "default - read the caveats it prints.")
    p.add_argument("--rescan", action="store_true",
                   help="re-walk the table instead of reading the cache "
                        "for the detector in use")
    p.add_argument("--table", type=Path, default=TABLE,
                   help=f"clean gimmickinfo body (default {TABLE})")
    p.add_argument("--json", type=Path, default=None, dest="json_out")
    p.add_argument("--doc", type=Path, default=None, dest="doc_out")
    args = p.parse_args()

    # The detector decides the files. Nothing else in the tool picks an output
    # path, so a loose run cannot reach the default artifacts by any route
    # except an explicit `--json`/`--doc` from the caller.
    args.detector = LOOSE if args.loose else SHIPPED
    if args.json_out is None:
        args.json_out = JSON_OUT_LOOSE if args.loose else JSON_OUT
    if args.doc_out is None:
        args.doc_out = DOC_OUT_LOOSE if args.loose else DOC_OUT

    querying = bool(args.query or args.record or args.family
                    or args.unclassified or args.list)
    if not querying:
        d = build(args.table, args.detector)
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.doc_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(json.dumps(d, indent=1) + "\n",
                                 encoding="utf-8")
        args.doc_out.write_text(render_doc(d), encoding="utf-8")
        t = d["totals"]
        print(f"wrote {args.json_out.relative_to(ROOT)} and "
              f"{args.doc_out.relative_to(ROOT)}: {t['distinct_items']} items, "
              f"{t['blocks']} blocks, {t['items_unclassified']} unclassified")
        if args.loose:
            print(f"LOOSE population: {t['items_loose_only']} of those items "
                  f"are INVISIBLE to the mod ({t['loose_only_blocks']} blocks "
                  f"in {t['loose_only_lists']} lists the plugin's detector "
                  "rejects),")
            print(f"and {t['implausible_item_ids']} item ids are implausible "
                  f"(all inside {IMPLAUSIBLE_HOME}, which is therefore being "
                  "mis-parsed).")
            print("The mod's own view is analysis/items.json, untouched by "
                  "this run.")
        return

    d = load(args)
    items = d["items"]
    if args.loose:
        t = d["totals"]
        print(f"[LOOSE population: {t['distinct_items']} items, of which "
              f"{t['items_loose_only']} are INVISIBLE to the mod. Lines "
              "marked LOOSE-ONLY are ones", file=sys.stderr)
        print(" no multiplier reaches; a trailing `!` means the id itself is "
              "implausible. Drop --loose for the mod's own view.]",
              file=sys.stderr)

    if args.list:
        for it in items:
            print(f"{it['id']:>9}  {it['name']:<26} {fmt_amount(it):<10} "
                  f"{it['confidence']:<8} {vis_tag(it)}"
                  f"{'/'.join(it['families']) or '-'}")
        return
    if args.unclassified:
        rows = sorted((i for i in items if not i["classified"]),
                      key=lambda i: -i["blocks"])
        for it in rows:
            print(f"{it['id']:>9}  {it['name']:<26} {fmt_amount(it):<10} "
                  f"{it['confidence']:<8} {vis_tag(it)}{it['blocks']:>3} "
                  f"blocks  {it['sources'][0]['name']}")
        print(f"{len(rows)} unclassified items"
              + (f", {sum(1 for i in rows if i['visibility'] == 'loose-only')}"
                 " of them loose-only" if args.loose else ""))
        return
    if args.family:
        f = args.family.lower()
        rows = [i for i in items if any(x.lower() == f for x in i["families"])]
        if not rows:
            fams = sorted({x for i in items for x in i["families"]})
            die(f"no Family {args.family!r}; known: {', '.join(fams)}")
        for it in sorted(rows, key=lambda i: i["name"]):
            print(f"{it['id']:>9}  {it['name']:<26} {fmt_amount(it):<10} "
                  f"{it['confidence']:<8} {vis_tag(it)}"
                  f"{it['records']} records")
        print(f"{len(rows)} items in {args.family}")
        return
    if args.record:
        q = args.record.lower()
        hits = [(it, s) for it in items for s in it["sources"]
                if q in s["name"].lower() or q == str(s["key"])]
        if not hits:
            die(f"no record matching {args.record!r} yields anything"
                + ("" if args.loose else
                   ". The mod's detector may not see it: try --loose."))
        for it, s in sorted(hits, key=lambda h: h[1]["name"].lower()):
            tag = ""
            if s.get("visibility") == "loose-only":
                tag = ("   LOOSE-ONLY, +64="
                       + ",".join(str(k) for k in s["entry_keys"]))
            print(f"{s['name']} (key {s['key']}, {s['family'] or 'unclassified'})"
                  f" -> item {it['id']} {it['name']} [{it['confidence']}] "
                  f"{s['min']}..{s['max']}{tag}")
        return

    q = args.query
    if q.isdigit():
        exact = [i for i in items if i["id"] == int(q)]
        if exact:
            show(exact[0])
            return
    ql = q.lower()
    hits = [i for i in items
            if ql in i["name"].lower()
            or any(ql in s["name"].lower() for s in i["sources"])]
    if not hits:
        die(f"nothing matching {q!r}. `--list` prints every item."
            + ("" if args.loose else " The mod's detector may not see it: "
                                     "try --loose."))
    if len(hits) == 1:
        show(hits[0])
        return
    exact = [i for i in hits if i["name"].lower() == ql]
    if len(exact) == 1:
        show(exact[0])
        print(f"\n({len(hits) - 1} other partial matches; "
              f"`{Path(sys.argv[0]).name} --list | grep {q}` for all)")
        return
    print(f"{len(hits)} matches for {q!r}:")
    for it in sorted(hits, key=lambda i: (i["name"], i["id"])):
        print(f"{it['id']:>9}  {it['name']:<26} {fmt_amount(it):<10} "
              f"{it['confidence']:<8} {vis_tag(it)}"
              f"{'/'.join(it['families']) or '-':<9} "
              f"{it['sources'][0]['name']}")


if __name__ == "__main__":
    # `--list | grep ...` is documented usage, so a closed pipe is normal.
    if hasattr(signal, "SIGPIPE"):
        signal.signal(signal.SIGPIPE, signal.SIG_DFL)
    main()
