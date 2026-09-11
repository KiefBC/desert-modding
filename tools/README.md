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

### `xrefs.py <rva-hex>`
Finds code references to an address without a disassembler, by scanning for
`E8`/`E9` rel32 calls and jumps and for RIP-relative `lea`/`mov`. Takes about a
minute and a half on the game exe.

**It takes an RVA. `sigscan.py` prints file offsets.** The two are not directly
composable: feeding a `sigscan.py` offset to `xrefs.py` will usually report
zero references, which looks like a bug and is not. Convert through the PE
section table first.

### `gen-collect-names.py`
Regenerates `desert-core/src/collect.rs` from the Desert Gatherer DMM pack.

**It rewrites the whole file and emits only the generated table.** The
committed `collect.rs` also carries hand-written material that the generator
does not reproduce: a note recording a design that was tried and rejected, and
the `non_gather_records_stay_out` test. Running this straight over the file
destroys both, silently, and the record table itself will look unchanged in the
diff. Generate to a scratch copy and merge the table across by hand.
