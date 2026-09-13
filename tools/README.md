# tools/

Small scripts that support the workspace. Nothing here is built or shipped; the
plugins do not depend on any of it at runtime.

Two kinds of thing live here, and the difference matters:

* **Wired** scripts are called by a `just` recipe or a GitHub workflow. If one
  breaks, `just ci` or a release breaks with it.
* **Manual** scripts are reverse-engineering aids you run by hand while working
  out how the game behaves. Nothing calls them, and nothing will tell you if
  they rot.

Most need the dev shell (`nix develop`) because `objdump`, `zip` and `jq` come
from `flake.nix`, not from the system. The `just` recipes handle that for you.

## Wired into `just` or CI

### `check-imports.sh` (`just check-imports`, and CI)
Fails if any built `.dll`/`.asi` imports a DLL outside the allowlist in
`justfile`. This is the only thing proving `libstdc++-6.dll` stays out of the
shipped plugins. Desert Overlay compiles Dear ImGui's C++ through the `cc`
crate, and `cc` would normally link the target's C++ runtime; the
`CXXSTDLIB=""` and `-fno-threadsafe-statics` pair in `.cargo/config.toml` is
what stops it. If that ever regresses the symptom is an `.asi` the game
silently refuses to load, so this check is the early warning. Needs the mingw
`objdump` from the dev shell.

### `dist.sh` (`just dist`, and the release workflow)
Builds the release zips into `dist/`, one per package, plus `SHA256SUMS`.
Archives are reproducible: fixed file order, `zip -X`, and a pinned
`DIST_EPOCH` rather than `SOURCE_DATE_EPOCH` (which the dev shell sets to a
1980 value that underflows the DOS timestamp west of UTC). There are only two
packages now: `DesertTooling-{version}.zip` (the one `.asi`, its `.ini`,
README, CHANGELOG and LICENSE) and the DMM pack; nothing bundles anything else
any more.

**It starts by deleting `dist/`.** Do not run it expecting the previous build
to survive.

### `release-notes.py` (release workflow only)
Turns a tag such as `desert-tooling-v0.3.0` into the release title, the package
name, and the release body pulled from that package's `CHANGELOG.md`. It exits
non-zero if the tag's version does not match the version in `Cargo.toml`, or if
the changelog has no matching `## [x.y.z]` heading. That second check is why a
new package cannot be released while its changelog still says
`## [Unreleased]`. Reads only; the workflow does the writing. Deliberately
stdlib-only so CI can run it before the Nix shell exists.

### `sync-versions.py` (`just sync-versions`, `just check-versions`, and CI)
Keeps the version numbers in `README.md`, `VERSIONING.md` and each shipping
crate's `README.md` in step with the crate `Cargo.toml` files, and the twelve
`desert-gatherer-dmm/*.json` module files in step with `dmm_pack.json` (the DMM
pack is not a crate and has its own `x.y` source of truth; each module repeats
it twice, once at the top level and once inside `modinfo`). `--check` is
read-only and is what CI runs; without it the script rewrites those files in
place. Stdlib-only, for the same reason as `release-notes.py`.

It also **checks** one thing it cannot write: that each shipping crate's
`CHANGELOG.md` has a `## [<version>]` heading for the version in its
`Cargo.toml`. That one fails in both modes, since `just sync-versions` must not
exit 0 on a bump whose entry nobody has written yet. Without it, a missing entry
goes unnoticed until the release workflow builds the notes — which is after the
tag has been pushed, and a tag is the release.

### `nexus-target.py` + `nexus-targets.json` (release workflow only)
Maps a tag to the Nexus Mods file it updates, and emits `key=value` lines for
`$GITHUB_OUTPUT`. All packages share one mod page, so the JSON records which
file each tag owns. A target with an empty `file_id` is the documented opt-out:
the script prints `publish=false` and the upload steps skip. No network access;
it is pure lookup and validation.

### `sigscan.py` + `reference-signatures.txt` (`just sigscan`)
Scans the installed `CrimsonDesert.exe` for the eight byte signatures the
plugins resolve at load, plus a handful of class and event names. Each
signature must hit **exactly once**; more than one hit means the pattern is no
longer unique and the plugin could bind to the wrong function. Run it after a
game update. Not in CI, because CI has no copy of the game.

`reference-signatures.txt` is the data file: one IDA-style pattern per line,
`??` for a wildcard byte, with a comment naming each and noting quirks. One
signature deliberately matches `0xF` bytes into its function rather than at the
entry point, and the comment says so.

It also checks `static-info-names.txt`, the 216 names of the game's 149
static-info types (149 class names plus the 67 lowercase table names that
differ), extracted from `FUN_1424fef70` and recorded in
`docs/reference-internals.md` section 19. Each must still occur **exactly once**
as a NUL-delimited literal; output is a single summary line plus any name that
went missing or gained a second hit. A change there means the game's type
registry moved, which is a much earlier and louder signal than a byte signature
going stale - the tables are what the gatherer edits.

That check is NUL-delimited on both sides, unlike the substring match the
`NAMES` list above it uses, because several of the names contain each other.
`iteminfo` is the visible example: the substring check reports 2 hits for it
because it also occurs inside `trademarketiteminfo`, while the NUL-delimited
check correctly reports 1.

### `evidence.py` (`just evidence`)
Walks the game's call graph outward from anchor functions and writes one
Markdown file per function into `evidence/` - signature, callers, direct
callees, decompilation, disassembly - plus `index.tsv` and `graph.json`. The
point is to answer "who else touches this" with `grep` instead of a few dozen
Ghidra MCP round-trips, and to survive the end of a session, which context does
not.

Unlike `dis.sh` and `xrefs.py` it is **not** offline: it needs the Windows
Ghidra running with GhidraMCP listening, and talks to that server's HTTP API
directly rather than through the MCP bridge, so it works whether or not the MCP
client is connected. The host is probed on `127.0.0.1` then the WSL default
gateway, the same order `~/.local/bin/ghidra-mcp-bridge-win` uses.

Default anchors are every in-module `FUN_1xxxxxxxx` named in `docs/*.md` or
appearing as a `// ==== FUN_x ====` header in `analysis/*.c` - about 120
functions, i.e. everything we have ever written about. Addresses outside
`[0x140000000, 0x160000000)` are dropped, which is what keeps the dead
CDLoot.asi names at `0x180000000` from becoming anchors. `--anchor <hex>`
overrides the set and `--anchors-file <path>` reads a list of them (one hex
address per line, `#` comments allowed) for when the set will not fit on a
command line; `--dry-run` prints the resulting anchors without contacting
Ghidra.

Three things about the walk are deliberate:

* **Hubs are recorded but not expanded.** A function with more than
  `--max-callers-expand` callers (default 40), or whose caller list came back
  truncated, contributes no new frontier. Without that, depth 2 finds
  `operator new` and the walk becomes the whole 250k-function program.
* **Tail calls are followed; intra-function jumps are not.** An unconditional
  `JMP` whose target is outside the function's own body is a tail call or a
  thunk stub and is walked through. This exe needs it: cold-code layout puts
  5-byte stubs at `0x1417xxxxx` that jump to real bodies at `0x14cxxxxxx`, and
  a CALL-only walk dead-ends at every one. In the first depth-2 tree, 16% of
  functions had such a jump and 1254 distinct targets were missing entirely -
  including 122 of the 149 `initStatic()` bodies in section 19's inventory.
* **Indirect calls are not followed.** Only `CALL 0x...` targets are callees;
  `CALL qword ptr [...]` names a slot, not a function.
* **`--max-functions` is a hard cap** (default 1500) and the run says so when
  it truncates a level. Depth 1 from the default anchors lands around 1500;
  depth 2 does not, and is what `--max-functions` is for.
* **Oversized "functions" are skipped.** Ghidra's auto-analysis glues runs of
  unanalysed bytes into single entries: one here claims a body of
  `14798013b - 15491057b`, 222 MB, 34.8 million disassembly lines and 14994
  callees. Anything whose body spans more than `--max-body-bytes` (default
  0x100000) gets a one-paragraph stub instead, and contributes no call edges -
  its callee list is noise, not a call graph. This matters more than it sounds:
  41 such entries once accounted for **9.9 GB of a 9.4 GB tree**, and their
  bogus callee lists inflated the walk far more than any real code did.
  `--max-section-bytes` (default 2 MiB) truncates an over-long decompilation or
  disassembly as a backstop, saying so in-band.

The tree is a snapshot of **one game build** (the build id is recorded in
`graph.json` and `evidence/README.md`, read from the Steam appmanifest). It is
gitignored like `analysis/`, and it is regenerated, never edited. After a game
update, re-export to a second directory and diff: the functions whose
decompilation moved are the candidate breakage list, which is a better starting
point than re-deriving every signature by hand.

Resumable - a function whose file already exists is not re-fetched, and its
graph edges are read back from the metadata comment on the file's first line.
`--force` re-fetches. Roughly one second per function at `--jobs 6`.

`index.tsv` and `graph.json` are rebuilt from **every file in the tree**, not
just the functions the current run walked, so a targeted top-up
(`--anchor`/`--anchors-file`) does not overwrite them with its handful of rows.
That also makes `--anchor <anything> --depth 0` a cheap way to regenerate both
without contacting Ghidra at all.

### `logo-to-rgba.py` (`just logo`)
Rasterises `assets/logo.svg` into `desert-overlay/src/logo.rgba`, the raw RGBA
blob the overlay embeds with `include_bytes!`. Output is deterministic, and the
committed blob matches the committed SVG today. Nothing verifies that
automatically, so if you edit the SVG, run this and commit both.

It understands only the subset of SVG the logo actually uses: axis-aligned
paths using `M H V Z`, integer coordinates, and a `fill` attribute written
before `d`. An unsupported path command is not rejected, it is silently
misparsed, so check the result by eye after editing the artwork.

## Manual, reverse-engineering aids

Nothing calls these. They exist for working out what the game does, and they
work offline on the shipped `.exe` with no Ghidra project, which is useful when
Ghidra is busy or closed. Decompilation itself goes through the Windows Ghidra
over its MCP bridge; these cover the cases where that is not to hand.

### `dis.sh <start-rva-hex> <end-rva-hex>`
Disassembles a range of the game exe as Intel syntax, with addresses printed as
RVAs. It first builds an image-layout copy of the exe in `$TMPDIR`, so that
file offsets and RVAs line up; that copy is about 385 MB and is not cleaned up.
Needs `objdump`, and unlike `check-imports.sh` it does not check for it first,
so outside the dev shell it fails with a bare "command not found".

### `xrefs.py <rva-hex> [exe] [--selfcheck]`
Finds references to an address without a disassembler, by scanning for
`E8`/`E9` rel32 calls and jumps and for RIP-relative `lea`/`mov` (REX-prefixed
and not). About **7 seconds** on the game exe, down from 65 s: it runs
`bytes.find` per opcode form instead of a Python loop over all 363 MB.
`--selfcheck` re-runs the old byte loop, which is kept in the file as
`reference_scan`, and diffs the two - use it after a change to the scanner, and
after a game update if a result looks wrong.

**It also reports pointer cells**, and that half is not a nicety. A target with
**no code xref at all** is normal in this exe: the indirect accessor encoding
reaches a table name through a pointer cell holding its VA (`gimmick::ACCESSORS`,
`docs/reference-internals.md` section 19), and whole families of class names
live only in pointer arrays. `SetAdditionalCollectDropRate` is the worked
example - zero code references, one pointer cell at `+0x56AF6E8`, which is entry
195 of a 208-name array and the only thing that identifies it at all. A scan
that printed "0 references" and stopped there is how that lead stayed
unexplored.

**It takes an RVA. `sigscan.py` prints file offsets.** The two are not directly
composable: feeding a `sigscan.py` offset to `xrefs.py` will usually report
zero references, which looks like a bug and is not. Convert through the PE
section table first.

### `fieldnames.py`
Recovers the **field names of every static-info record class** from the
shipped exe - 4675 `(class, field)` pairs across 536 classes on build
25246367 - and writes them to `analysis/fieldnames.json`.

They come from the game's own error strings. Every record deserializer
reports a per-field read failure with a UTF-8 **Korean** message of the
form `<ClassName>의 _<fieldName>를 읽어들이는데 실패했다.`, so both names
are sitting in the string pool. `strings` skips them (not ASCII) and
Ghidra has not typed them, which is why nothing in `docs/` mentioned
them before 2026-09-12. One regex over the image gets the lot; the method
and what it is worth are recorded in `docs/reference-internals.md`
section 19.9, and section 16.1 is the layout it named.

```
python3 tools/fieldnames.py                        # rebuild + self-checks
python3 tools/fieldnames.py GimmickInfo            # one class's fields
python3 tools/fieldnames.py --field dropTagNameHash    # who has this field?
python3 tools/fieldnames.py --grep drop            # fuzzy over class+field
python3 tools/fieldnames.py --list                 # one line per class
python3 tools/fieldnames.py --rescan               # force the walk
```

Offline, stdlib-only, about a second; `--exe` overrides the install path.
The JSON is gitignored like everything in `analysis/`, is read back for
the lookups, and is regenerated, never edited.

**It answers "what are the fields called", not "where are they."** A name
is evidence of a field the deserializer reads and nothing else - no type,
no width, no offset, and read order is not string-pool order. Getting an
**offset** is a second, mechanical step that is deliberately not
implemented: find the RIP-relative `48 8D 05 disp32` that loads the
message, then read the offset out of the surrounding
`lea rdx,[rec+OFF]; ... lea rax,[msg]` shape. Section 19.9 has the
procedure. The output carries each message's **RVA** as well as its file
offset precisely so it can be handed to `xrefs.py` without the
offset/RVA conversion that section warns about.

Two more things worth knowing before using a name:

* The classes are the **logical** CamelCase names (`GimmickInfo`,
  `DropSetInfo`), not the lowercase table names the accessor census keys
  on (`gimmickinfo`, `dropsetinfo`). `reference-internals.md` section
  19.2 carries both, and is the join.
* 536 classes against 149 tables, because nested record types get their
  own messages without being tables - `DropInfoData` inside `GimmickInfo`
  is the one that paid for the tool.

Every rebuild re-checks its two counts and four relations - the two
`GimmickInfo` yield lists by name, `DropInfoData`'s min/max/item fields,
`DropSetInfo`'s own, and that `dropTagNameHash` is on exactly those two
classes and nowhere else - and prints `FAIL` for any that moved. A `FAIL`
means the exe's field messages changed; explain it before trusting
anything downstream.

### `items.py`
Builds the **item cross-reference**: what every item id in the game's
resource-output blocks actually is, which records yield it, how much of
it they give, and which `Family` (if any) covers it today. This is the
answer to "the gatherer multiplies yields by record and there is no way
to look up what an item id *is*".

It walks the clean `gimmickinfo` table body DMM writes out
(`/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin`, `--table` overrides),
which carries record-relative offsets only and so needs no rebasing for
a game update, and reads `desert-core/src/collect.rs` for the current
family of each record. Offline, stdlib-only, about five seconds
(`--loose` walks the body twice, so about eight).

With no arguments it writes both outputs and prints its self-checks:

```
python3 tools/items.py                     # rebuild both outputs
python3 tools/items.py 22008               # what is item 22008?
python3 tools/items.py salt                # find an item by inferred name
python3 tools/items.py --record peony_01   # what a record yields (name or key)
python3 tools/items.py --family Foraging   # every item of one Family
python3 tools/items.py --unclassified      # items no Family covers yet
python3 tools/items.py --list              # one line per item
python3 tools/items.py --loose             # any of the above, wider detector
```

* `analysis/items.json` - one entry per item id: inferred name,
  confidence, every source record with its key, vanilla min/max and
  block offsets, and the current family. Machine-readable, and what the
  lookups read, so a query is instant. `--rescan` forces the walk.
* `docs/reference-items.md` - the human cross-reference: summary,
  items grouped by current `Family`, then the unclassified ones ordered
  by how widely they are placed, then a flat index by id.

`--loose` writes `analysis/items-loose.json` and
`docs/reference-items-loose.md` instead, and reads them back for its
lookups. **The two pairs never touch**: a loose run cannot overwrite the
default artifacts and a default run cannot overwrite the loose ones, and
a cache whose detector does not match what was asked for is ignored
rather than answered from. All four are gitignored, like everything else
in `analysis/` and `docs/`. None is edited by hand; regenerate instead.

#### `--loose`: the population the mod cannot see

There are two detectors and `items.py` makes the choice a named one
(`Detector`, `SHIPPED` and `LOOSE` at the top of the file), not a
boolean threaded through the walk. Both check the block's shape and
require the two copies of the item id at `+1` and `+60` to agree;
`shipped` additionally requires `+5` to equal `+64`, exactly as
`desert_core::gimmick::block_ok` does, and is the default.

| detector | lists | blocks | distinct items |
| --- | --- | --- | --- |
| `shipped` (default) | 573 | 896 | 215 |
| `--loose` | 589 | 1038 | 311 |

The 573 are a strict subset of the 589. The 16 extra lists are real
content - `Temple_Chest_01`, `dff_chest_24`,
`gimmick_item_dropset_treasurebox_01`, `clawmachine_capsule_01`,
`Action_dig_01`, `gimmick_Dig_land_0001`, the
`gimmick_abyssone_bridge_gate_*` set, `gimmick_marni_teleportation_*` -
and **none of them is one of the 275 gather records the DMM pack
edits**, so nothing the gatherer does today reaches any of them.

**Treat the loose output as weaker evidence than the default, because it
is.** Its doc says so at the top and both reasons are concrete: nine of
its item ids are nine digits (`391518521`..`391518546`) where every id
the shipped detector sees is eight or fewer, and all nine sit in the one
list `gimmick_item_dropset_treasurebox_01`, which is therefore being
mis-parsed; and only 5 of the 101 ids in the extra blocks also occur in
the shipped population, so almost nothing here is cross-checked. The
implausible ids are flagged, never filtered - the flag is the finding.
Every item, every source record and every terminal line carries whether
it is `shipped`-visible or `LOOSE-ONLY`, and a loose-only record also
carries its `+64` value.

**Two things it does are load-bearing and easy to get wrong.** Both are
recorded in `docs/findings-water-wells-2026-09-12.md` sections 5, 7 and 8,
and both cost an earlier investigation a wrong answer:

* The item id is at **`block+1`**, echoed at `+60`. Both `+5` and `+64`
  are zero on all 896 blocks the shipped detector admits, so `block_ok`'s
  `b[5..9] == b[64..68]` clause is vacuous *within that population* - but
  it is not vacuous, and calling `+64` a pad is wrong. It is zero on
  those 896 and **nonzero on all 142 blocks only `--loose` sees**, which
  is precisely what excludes them. `FUN_141a37180`, the block parser,
  consumes exactly `+0..+63`; the list loop `FUN_1414a7cc0` reads the
  four bytes at `+64` after each block and stores them at `entry+0x08`,
  so `+64` is the **list entry's own key field**. `+5` is the only real
  pad: zero on all 1038 blocks of both populations.
* A block is attributed to its record by the **key echo**: a real record
  header repeats its own `u32` key just before a later digits-only id
  sub-field. Nested string fields use the same `u32 len, bytes, NUL`
  shape as record names, so a plain backwards scan finds
  `NatureBuffTrigger` with the bogus key `16777216` where
  `firewood_0001` should be.

Every run re-checks the five numbers and three offset anchors that were
confirmed three independent ways - 573 output lists, 896 blocks, 215
distinct item ids, 13412 records, and fourteen known item names - and
prints `FAIL` for any that moved. A `FAIL` means the table or the walk
changed, and nothing downstream should be trusted until it is explained.
A `--loose` run checks its own three counts (589/1038/311) on top, plus
the relations that make it a superset rather than a different answer:
the 573 shipped lists all present, 16 extra lists and 142 extra blocks,
a nonzero `+64` on every one of those blocks and a zero `+5` on all
1038, 96 items that only it can see, and the 9 implausible ids confined
to one list. Those are this walk's own numbers and have had none of the
three-ways treatment, which is the point of stating them separately.

**Item names are inferred, never read.** The game's own item names live
in the `.paz` archives, which are not extracted; what this does instead
is read the name of the record that yields the item
(`gimmick_item_trade_salt_02` -> `1000648` is salt). A token appearing in
the records of many *different* items names the container rather than the
goods, so those lose; the `confidence` field says how much agreement
there was, and `guess` means one generic container named it and nothing
else did. Two ids have a curated name and a stated reason instead: item
`1` is **money** (coins and silver bars share it) and `22008` is water.

`--loose` adds one honesty rule that applies to loose-only items and
nowhere else: an id whose inferred name is shared with another id is
forced to `guess` whatever its agreement count, because a name one
record hands to several ids names the source and not the goods. Without
it the seven ids of `Action_dig_01` and `gimmick_Dig_land_0001` would
read as `single`-confidence items called "action_dig" and "dig_land" -
nothing contradicts those names because nothing else mentions those ids
at all. The pre-rule level is kept as `confidence_before_loose_rule`.
The wider corpus can also move a name outright; any id whose name
differs from the one the default doc gives it carries
`name_in_default_doc` saying so.

### `gen-collect-names.py`
Regenerates `desert-core/src/collect.rs` from the Desert Gatherer DMM pack
plus `tools/extra-families.json`.

**It rewrites the whole file**, and it now owns everything in it - the enum,
the rows, both lookups and every test, including the hand-written
`family_by_name` note and the `non_gather_records_stay_out` test that earlier
versions dropped - so running it straight over the file is safe. Anything added
to `collect.rs` by hand still dies on the next run; add it to the generator.

`extra-families.json` is the second input: records the pack has no module for,
**keyed by record** (the water well, added to `Foraging`, and the three placed
money props, which are the `Money` family on their own), with the item ids each
pays stored as derived data. Item 1 is money and the rule on it is two-way:
every family but `Money` refuses a record that pays it, and `Money` refuses a
record that pays anything else - so a yield slider can never become an economy
lever by accident, and the economy lever can never pick up a material. When DMM's clean table body is
present the generator re-derives those from the bytes and refuses to write on
any mismatch; when it is absent it prints a banner saying the rows were not
verified. Records measured to be unreachable by a table edit are kept in the
same file as `records_not_enabled`, verified the same way and emitted only as a
test that keeps them out. The script's docstring has the format and the reasons.
