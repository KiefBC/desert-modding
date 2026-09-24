# tools/

Small host binaries that support the workspace. Nothing here is built into a
plugin or shipped; the plugins do not depend on any of it at runtime.

Two kinds of thing live here, and the difference matters:

* **Wired** tools are called by a `just` recipe or a GitHub workflow. If one
  breaks, `just ci` or a release breaks with it.
* **Manual** tools are reverse-engineering aids you run by hand while working
  out how the game behaves. Nothing calls them, and nothing will tell you if
  they rot.

These were thirteen Python and bash scripts until the port. They are thirteen
Rust binaries now, in one package (`desert-tools`), and the `.py`/`.sh`
originals are gone. A fourteenth, `dmm-rebase`, replaced the last Python in
the repo, the DMM pack's own `rebase.py`. What that bought, in the order it matters:

* **The dev shell is needed for far less.** No tool shells out to `objdump`,
  `zip`, `unzip`, `sha256sum` or `python3` any more. The only external program
  any tool runs is `cargo`, from `dist`. `nix develop` is still where the Rust
  toolchain and the mingw linker come from, so `just tools` wants it - but a
  built `check-imports` or `dis` needs nothing outside itself.
* **Speed, in the places it changed an answer.** `sigscan` 25.34s -> 2.39s,
  `xrefs` 7.88s -> 1.83s, `xrefs --selfcheck` 48.99s -> 2.66s, `items` 5.6s ->
  0.13s, `gen-collect-names` 5.5s -> 0.22s, `fieldnames` 8.8s -> 1.6s, and
  `dis` from 3.14s on a *warm* cache to 0.007s with no cache at all. A tool you
  can run in a loop gets run; one that takes half a minute gets guessed at.
* **Two silent-wrong-answer bugs are closed**, not merely detected: `dis`'s
  stale image cache and `logo-to-rgba`'s silent misparse. Both are written up
  below, because both cost real time.

Byte-identical output was the acceptance criterion for every port, so stdout
still carries the old column widths, the `(s)` on every plural, and in one case
a sentence that has always been wrong (see **Known defects**).

## Building and checking them

```
just tools              build them (host target, release)
just tools-check        clippy + unit tests; part of `just ci`
just tools-check-game   the #[ignore]d tests needing the game exe, DMM's
                        table or a built plugin
```

`tools-check-game` is separate from `tools-check` for exactly the reason
`test-game` is separate from `test`: a CI runner has no game, no DMM table and
no Ghidra.

Every tool is **run with the repo root as the working directory**, and none of
them resolves a default path against it. `desert_tools::paths::repo_root()`
walks up for an ancestor holding both `justfile` and `flake.nix`, and every
default hangs off that. `dist` deletes a directory called `dist`; one resolved
against wherever the shell happened to be would eventually delete the wrong
one.

The shared library is `desert_tools`, and it is deliberately two modules:
`paths` (every default overridable by the same environment variable the
justfile uses, so a tool run by hand and the same tool run by `just` look in
the same place) and `pe` (offset <-> RVA conversion, the loader's image layout,
and reading the import table). Anything only one binary needs stays in that
binary.

## Its own cargo workspace, and the config trap

`tools/` is a **separate cargo workspace**, excluded from the root one with
`exclude = ["tools"]` in the root `Cargo.toml`. The root pins
`build.target = x86_64-pc-windows-gnu` because everything it builds is a
Windows plugin. Nothing here is: these are host binaries that read the game
exe, rewrite docs and build the release zips.

**Cargo discovers `.cargo/config.toml` by walking up from the current
directory, not from `--manifest-path`.** That is the trap, and it has two
different faces depending on where you are standing:

* From the repo root **outside** the dev shell,
  `cargo build --manifest-path tools/Cargo.toml` inherits the *root's* Windows
  target and dies with `error[E0463]: can't find crate for core` - "the
  `x86_64-pc-windows-gnu` target may not be installed".
* From the repo root **inside** the dev shell it does not fail at all. It
  succeeds, and quietly produces `tools/target/x86_64-pc-windows-gnu/release/
  sigscan.exe` and twelve friends: Windows executables you cannot run. That is
  the worse of the two, because nothing says anything is wrong.

`tools/.cargo/config.toml` pins the host target back, but it only helps
someone who has already `cd`'d into `tools/`. So the `justfile` sets
`CARGO_BUILD_TARGET` in the environment instead, which outranks both config
files, and that is what lets every recipe stay in the repo root - where a
relative path handed to a tool still means what it says.

## Wired into `just` or CI

### `check-imports` (`just check-imports`, and CI)
Fails if any built `.dll`/`.asi` imports a DLL outside the allowlist in
`justfile`. This is the only thing proving `libstdc++-6.dll` stays out of the
shipped plugin. Desert Overlay compiles Dear ImGui's C++ through the `cc`
crate, and `cc` would normally link the target's C++ runtime; the
`CXXSTDLIB=""` and `-fno-threadsafe-statics` pair in `.cargo/config.toml` is
what stops it. If that ever regresses the symptom is an `.asi` the game
silently refuses to load, so this check is the early warning.

It reads the PE import directory in process (`desert_tools::pe::import_dlls`)
rather than shelling out to the mingw `objdump`, so it needs nothing external -
not even the dev shell. That was diffed against
`x86_64-w64-mingw32-objdump -p` on the real built `desert_tooling.dll` before
the shell version was deleted: identical, 10 entries.

### `dist` (`just dist`, and the release workflow)
Builds the release zips into `dist/`, one per package, plus `SHA256SUMS`.
There are only two packages: `DesertTooling-{version}.zip` (the one `.asi`,
its `.ini`, README, CHANGELOG and LICENSE) and the DMM pack; nothing bundles
anything else any more.

**It starts by deleting `dist/`.** Do not run it expecting the previous build
to survive.

Archives are reproducible: fixed entry order, a fixed mode, no per-file extra
fields, and a fixed stamp from `DIST_EPOCH` rather than `SOURCE_DATE_EPOCH`
(which the dev shell sets to a 1980 value that underflows the DOS timestamp
west of UTC). Two things improved over the `zip -X` mechanics this replaces:

* **The stamp is converted to UTC by the tool.** The shell version wrote it
  with `touch -d @epoch` and let Info-ZIP convert through the local zone. Run
  under `TZ=America/Los_Angeles`, the shell version stamped every entry
  `12-31-2019 16:00` where the Rust tool still stamps `01-01-2020 00:00`. The
  archives are now reproducible *across machines in different timezones*, which
  the old ones were not, despite the header that said they were.
* **Release checksums differ from the Info-ZIP era**, and will not come back.
  The deflate streams differ at the same nominal level and no setting makes two
  compressors agree. Everything that is a decision rather than an encoding is
  identical - contents, entry order, stamps, modes - and each release publishes
  the checksums of the bytes it actually attached, so the change is not
  something anyone downstream can observe.

This is the one recipe that still needs the dev shell at run time, because
`dist` spawns `cargo build --release` for the Windows target and that needs the
mingw linker. `cargo` is the only external program it runs.

### `release-notes` (release workflow only)
Turns a tag such as `desert-tooling-v0.3.0` into the release title, the package
name, and the release body pulled from that package's `CHANGELOG.md`. It exits
non-zero if the tag's version does not match the version in `Cargo.toml`, or if
the changelog has no matching `## [x.y.z]` heading. That second check is why a
new package cannot be released while its changelog still says
`## [Unreleased]`. Reads only; the workflow does the writing.

`--sums dist/SHA256SUMS` appends the checksum table; `--changelog` prints the
entry alone, which is what goes to the Nexus page, where a file list and a
checksum table would be noise.

### `sync-versions` (`just sync-versions`, `just check-versions`, and CI)
Keeps the version numbers in `README.md`, `VERSIONING.md` and each shipping
crate's `README.md` in step with the crate `Cargo.toml` files, and the twelve
`desert-gatherer-dmm/*.json` module files in step with `dmm_pack.json` (the DMM
pack is not a crate and has its own `x.y` source of truth; each module repeats
it twice, once at the top level and once inside `modinfo`). `--check` is
read-only and is what CI runs; without it the tool rewrites those files in
place.

It also **checks** one thing it cannot write: that each shipping crate's
`CHANGELOG.md` has a `## [<version>]` heading for the version in its
`Cargo.toml`. That one fails in both modes, since `just sync-versions` must not
exit 0 on a bump whose entry nobody has written yet. Without it, a missing entry
goes unnoticed until the release workflow builds the notes - which is after the
tag has been pushed, and a tag is the release.

### `dmm-rebase` (`just dmm-rebase`)
Rebases the DMM pack's twelve module files onto a new game build, and
regenerates `desert-gatherer-dmm/VERIFICATION.txt`. Run it after a game update,
once `just test-game`'s pack oracle (`multiply_reproduces_the_dmm_pack_edits`)
has passed; `desert-gatherer-dmm/README.md` has the whole procedure. Not part
of `just ci`: it only has work to do when the game changes.

```
just dmm-rebase                        defaults for both inputs
just dmm-rebase <table> <build>        either or both given positionally
just dmm-rebase --dry-run              verify and print the report, write nothing
```

Two inputs. The clean `gimmickinfo` table body comes from
`paths::dmm_table()` - the plugin's own dump
`<bin64>/DesertTooling.gimmickinfo.bin`, written by one launch with
`[Gatherer] DumpTable=1` and `DryRun=1`, unless `CD_DMM_TABLE` names another
copy (DMM's backup is the last resort). The build id comes from the Steam
appmanifest (`CD_APPMANIFEST`); with neither a `--build` nor a manifest it
refuses rather than stamp the pack with a guess. The first line it prints is
the table's size, SHA-256 and the build, so a run on the wrong table shows
itself immediately.

Each change is found again by record key and name, then by the 68-byte output
block's signature within `SEARCH_WINDOW` (1024 bytes) of where its list used to
sit in the record, and the vanilla bytes are checked at the new offset before
`offset`, `record_rel_offset` and `rel_offset` are rewritten. Per module it
prints how many output lists stayed put inside their record and how many
shifted.

**All or nothing.** Every module is verified, and the whole report built,
before anything is written; then every file goes to a temporary sibling and is
renamed into place. The Python this replaces wrote module by module, so a
record it could not resolve in module seven left a pack half on one build and
half on the other. An overlap between two categories' offsets is a failure too
(the Python wrote `"result": "FAIL"` and exited 0).

**Output fidelity.** The module files are CRLF in git and are written back
CRLF, formatted exactly as Python's `json.dumps(obj, indent=2,
ensure_ascii=False)` - which serde_json's pretty printer matches once
`preserve_order` keeps every key where it was. So a rebase diff is the offset
lines and `game_build`, nothing else. That is not taken on trust: a unit test
round-trips every committed module and `VERIFICATION.txt` byte for byte, and the
first real run (build 25477059) produced output identical, all thirteen files,
to `rebase.py` run on the same table. `tests/dmm_rebase.rs` covers a rebase, a
dry run, a failure part way through that must leave every file untouched, and
(ignored, needs the table) that rebasing the committed pack onto its own table
changes nothing.

It uses `desert_tools::sha256`, the library's dependency-free SHA-256 that
`dist` uses for `SHA256SUMS`, for the table digest and the simulated
patched-table digests in the report.

### `nexus-target` + `nexus-targets.json` (release workflow only)
Maps a tag to the Nexus Mods file it updates, and emits `key=value` lines for
`$GITHUB_OUTPUT`. All packages share one mod page, so the JSON records which
file each tag owns. A target with an empty `file_id` is the documented opt-out:
the tool prints `publish=false` and the upload steps skip. No network access;
it is pure lookup and validation, so a misconfigured target fails in a second
instead of half way through an upload.

### Both workflows now install Nix first
`release-notes` and `sync-versions` used to be stdlib-only Python precisely so
CI could run them before the Nix shell existed. **That is superseded.** They
are Rust binaries and need a toolchain, so `ci.yml` and `release.yml` both
install Nix as their first step and build the tools before they gate anything.

`release.yml` has a second job (`nexus`) that runs after the release is cut,
and it does not install a toolchain of its own: the first job uploads
`nexus-target` and `release-notes` as a build artifact named `tools`, and the
`nexus` job downloads and `chmod +x`es them. Two Nix installs to run two
lookups would be the slower and more fragile arrangement.

### `sigscan` + `reference-signatures.txt` (`just sigscan`)
Scans the installed `CrimsonDesert.exe` for the nine byte signatures the
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

**It always exits 0.** A signature with no hits, a signature with hundreds, and
a type name that vanished all print and then exit 0 exactly as a clean run
does. `just sigscan` can therefore never fail a script or CI; it is a thing you
read, not a gate. See **Known defects**.

**The offsets it prints are FILE OFFSETS.** `xrefs` takes an RVA. See the note
under `xrefs`.

### `evidence` (`just evidence`)
Walks the game's call graph outward from anchor functions and writes one
Markdown file per function into `evidence/` - signature, callers, direct
callees, decompilation, disassembly - plus `index.tsv` and `graph.json`. The
point is to answer "who else touches this" with `grep` instead of a few dozen
Ghidra MCP round-trips, and to survive the end of a session, which context does
not.

Unlike `dis` and `xrefs` it is **not** offline: it needs the Windows Ghidra
running with GhidraMCP listening, and talks to that server's HTTP API directly
rather than through the MCP bridge, so it works whether or not the MCP client is
connected. The host is probed on `127.0.0.1` then the WSL default gateway, the
same order `~/.local/bin/ghidra-mcp-bridge-win` uses.

Default anchors are every in-module `FUN_1xxxxxxxx` named in `docs/*.md` or
appearing as a `// ==== FUN_x ====` header in `analysis/*.c` - **201** functions
on the current corpus, i.e. everything we have ever written about. Addresses
outside `[0x140000000, 0x160000000)` are dropped, which is what keeps the dead
CDLoot.asi names at `0x180000000` from becoming anchors. `--anchor <hex>`
overrides the set and `--anchors-file <path>` reads a list of them (one hex
address per line, `#` comments allowed) for when the set will not fit on a
command line; `--dry-run` prints the resulting anchors without contacting
Ghidra.

Five things about the walk are deliberate:

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
`--anchor <anything> --depth 0` is the cheap way to regenerate both - but it is
not an offline one: the host probe still runs, so it needs `--host` or a live
server, and it still fetches the anchor itself unless that function's file is
already on disk.

**The response-shape risk of the port is retired.** `evidence` was checked
against a synthetic GhidraMCP server *and* against the live one: a real depth-1
export was run with the old Python and the new Rust against Ghidra on the WSL
gateway, and the trees, `index.tsv` and `graph.json` came out identical - 7
functions, 28 requests, build id 25246367.

Two things it gets wrong are in **Known defects**, and one of them is the
worst thing in this directory.

### `logo-to-rgba` (`just logo`)
Rasterises `assets/logo.svg` into `desert-overlay/src/logo.rgba`, the raw RGBA
blob the overlay embeds with `include_bytes!`. Output is deterministic, and the
committed blob matches the committed SVG today. Nothing verifies that
automatically, so if you edit the SVG, run this and commit both. `--png <path>`
writes the same pixels somewhere you can look at them.

It understands only the subset of SVG the logo actually uses: axis-aligned
paths using `M H V Z`, integer coordinates, and a `fill` attribute written
before `d`. That narrowness is the design - a renderer that only does what the
artwork does cannot silently disagree with a browser about arcs, transforms or
stroke.

**An unsupported path command is now a hard error.** It used to be a silent
misparse: the tokeniser only matched `[MmHhVvZz]`, so a `C`, `A` or `L` was
swallowed as operands of the *previous* command, and the tool drew a plausible
wrong logo and reported success. The tokeniser now matches any ASCII letter, so
an unsupported command reaches the state machine and is rejected there. Still
check the result by eye after editing artwork - being rejected is not the same
as being right - but the silent-wrong-answer path is closed.

## Manual, reverse-engineering aids

Nothing calls these. They exist for working out what the game does, and they
work offline on the shipped `.exe` with no Ghidra project, which is useful when
Ghidra is busy or closed. Decompilation itself goes through the Windows Ghidra
over its MCP bridge; these cover the cases where that is not to hand.

**None of them may select a PE section by name, and there is a test asserting
it.** This exe's twelve section names are scrambled - `.idata .arch .debug$P
.tls$ .shared .xcode .00cfg .sbss .text1 .trace .xtls .link` - and the 80 MB
one holding the code is the one called `.idata`. No section is called `.text`;
the `.text1` that exists is 1.9 MB of something else. Match on the address
range, from the section table. See
`tools/tests/pe_real_exe.rs`, which exists because an earlier version of it
asserted a `.text` section and failed against the real game.

### `dis <start-rva-hex> <end-rva-hex>`
Disassembles a range of the game exe as Intel syntax, with addresses printed as
RVAs. Add `0x140000000` for the preferred VA that Ghidra shows.

**There is no image cache any more, and the staleness bug went with it.** The
shell version wrote a ~380 MB image-layout copy of the exe into `$TMPDIR` and
left it there; the cache was keyed on existence alone, so after a game update
every call kept disassembling the *previous* build's bytes - clean looking,
correctly formatted, and wrong, with nothing printed to say so. A 2026-09-13
investigation lost time to that before noticing the cached image was three days
older than the exe and a different size, and the script grew an `-nt` check to
paper over it. This version cuts the requested window out of the exe on every
run: there is no copy to go stale, the whole class of bug is gone rather than
detected, and a forty-instruction listing no longer costs a 380 MB allocation.
It is also why this went from 3.14s on a warm cache to 0.007s with none.

It disassembles with `iced-x86` rather than `objdump`, which removed the last
`objdump` dependency in this directory - so `dis` no longer needs the dev shell
and no longer fails outside it with a bare "command not found". The cost is
that operand **spelling** differs from the old output in places:

```
iced                       objdump
push rbx                   rex push rbx
[0x57fc990]                [rip+0x409852a]        # 0x57fc990
gs:[0x58]                  gs:0x58
```

Spelling is cosmetic; addresses are not. Instruction **boundaries** were
verified identical across four code ranges before the script was deleted. If a
`+rva` ever appears here that the old output did not also print, that is a real
decoding disagreement and worth chasing.

One thing to know when reading a listing: it decodes from wherever you point
it, so a start address that is not an instruction boundary produces a few lines
of garbage before the stream resynchronises. If a single instruction matters,
disassemble it from two or three different earlier starts and check they agree.

### `xrefs <rva-hex> [exe] [--selfcheck]`
Finds references to an address without a disassembler, by scanning for
`E8`/`E9` rel32 calls and jumps and for RIP-relative `lea`/`mov` (REX-prefixed
and not). About **two seconds** on the game exe: one `memchr` pass per opcode
form rather than a loop over all 380 MB, and one set of 24 `(opcode, modrm)`
patterns covering the REX and no-REX forms together rather than 24 + 96.
`--selfcheck` re-runs the obvious byte loop, which is kept beside the fast one
as `scan::reference_scan`, and diffs the two - use it after a change to the
scanner, and after a game update if a result looks wrong.

**It also reports pointer cells**, and that half is not a nicety. A target with
**no code xref at all** is normal in this exe: the indirect accessor encoding
reaches a table name through a pointer cell holding its VA (`gimmick::ACCESSORS`,
`docs/reference-internals.md` section 19), and whole families of class names
live only in pointer arrays. `SetAdditionalCollectDropRate` is the worked
example - zero code references, one pointer cell at `+0x56AF6E8`, which is entry
195 of a 208-name array and the only thing that identifies it at all. A scan
that printed "0 references" and stopped there is how that lead stayed
unexplored.

That is the class-name literal at `+0x56A5EB0`. The *short* name literal at
`+0x569C7C0` is the same story told the other way, and it is the example to
reach for when explaining the counting defect below: `xrefs 569C7C0` reports
"2 code reference(s)" and one pointer cell at `+0x5861BF8` (entry 193 of a
separate 206-name array), but the two code hits are one instruction - the
`4C 8D 05` at `+0x1529527`, reported once at the REX byte as `lea` and once at
the opcode as `lea32`.

**It takes an RVA. `sigscan` prints file offsets.** The two are not directly
composable: feeding a `sigscan` offset to `xrefs` will usually report zero
references, which looks like a bug and is not. Convert through the PE section
table (`desert_tools::pe`) first.

### `fieldnames`
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
section 19.9, and section 16.1 is the layout it named. The scan runs over raw
bytes with `regex::bytes`, not `regex`: 375 MB of mostly non-UTF-8 will not
decode as a `str` at all.

```
fieldnames                            # rebuild + self-checks
fieldnames GimmickInfo                # one class's fields
fieldnames --field dropTagNameHash    # who has this field?
fieldnames --grep drop                # fuzzy over class+field
fieldnames --list                     # one line per class
fieldnames --rescan                   # force the walk
```

Offline, no network, about 1.6 s for a full walk and instant for a lookup;
`--exe` overrides the install path. The JSON is gitignored like everything in
`analysis/`, is read back for the lookups, and is regenerated, never edited.

**It answers "what are the fields called", not "where are they."** A name
is evidence of a field the deserializer reads and nothing else - no type,
no width, no offset, and read order is not string-pool order. Getting an
**offset** is a second, mechanical step that is deliberately not
implemented: find the RIP-relative `48 8D 05 disp32` that loads the
message, then read the offset out of the surrounding
`lea rdx,[rec+OFF]; ... lea rax,[msg]` shape. Section 19.9 has the
procedure. The output carries each message's **RVA** as well as its file
offset precisely so it can be handed to `xrefs` without the offset/RVA
conversion that section warns about.

Two more things worth knowing before using a name:

* The classes are the **logical** CamelCase names (`GimmickInfo`,
  `DropSetInfo`), not the lowercase table names the accessor census keys
  on (`gimmickinfo`, `dropsetinfo`). `reference-internals.md` section
  19.2 carries both, and is the join.
* 536 classes against 149 tables, because nested record types get their
  own messages without being tables - `DropInfoData` inside `GimmickInfo`
  is the one that paid for the tool.

Every rebuild re-checks its two counts, four relations - the two
`GimmickInfo` yield lists by name, `DropInfoData`'s min/max/item fields,
`DropSetInfo`'s own, and that `dropTagNameHash` is on exactly those two
classes and nowhere else - and two structural invariants: that no class
names a field twice, and that every message file offset maps back to an
RVA. It prints `FAIL` for any that moved. A `FAIL`
means the exe's field messages changed; explain it before trusting
anything downstream. It exits 0 either way - see **Known defects**.

### `items`
Builds the **item cross-reference**: what every item id in the game's
resource-output blocks actually is, which records yield it, how much of
it they give, and which `Family` (if any) covers it today. This is the
answer to "the gatherer multiplies yields by record and there is no way
to look up what an item id *is*".

It walks the clean `gimmickinfo` table body (`--table` overrides). The
preferred source is the plugin's own dump,
`<bin64>/DesertTooling.gimmickinfo.bin`, written by one launch with
`[Gatherer] DumpTable=1` and `DryRun=1`; DMM's copy
(`/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin`) is the fallback when
there is no dump, and `CD_DMM_TABLE` beats both. The bytes are identical -
the dump is preferred because DMM deletes its backups at will. The body
carries record-relative offsets only and so needs no rebasing for a game
update. It also reads `desert-core/src/collect.rs` for the current family
of each record. Offline, about a sixth of a second - a full rebuild
of both outputs is now cheaper than the old tool's startup.

With no arguments it writes both outputs and prints its self-checks:

```
items                     # rebuild both outputs
items 22008               # what is item 22008?
items salt                # find an item by inferred name
items --record peony_01   # what a record yields (name or key)
items --family Foraging   # every item of one Family
items --unclassified      # items no Family covers yet
items --list              # one line per item
items --loose             # any of the above, wider detector
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

There are two detectors and `items` makes the choice a named one - the
`Detector` enum, `Shipped` and `Loose`, in `items/table.rs` - not a
boolean threaded through the walk. Both check the block's shape and
require the two copies of the item
id at `+1` and `+60` to agree; `shipped` additionally requires `+5` to
equal `+64`, exactly as `desert_core::gimmick::block_ok` does, and is the
default.

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
recorded in `docs/findings/2026-09-12-water-wells.md` sections 5, 7 and 8,
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

### `gen-collect-names`
Regenerates `desert-core/src/collect.rs` from the Desert Gatherer DMM pack
plus `tools/extra-families.json`.

**It rewrites the whole file**, and it owns everything in it - the enum, the
rows, both lookups and every test, including the hand-written `family_by_name`
note and the `non_gather_records_stay_out` test that earlier versions dropped -
so running it straight over the file is safe. Anything added to `collect.rs` by
hand still dies on the next run; add it to the generator. `--out <file>` writes
somewhere else, which is the way to diff a change before it lands.

`extra-families.json` is the second input: records the pack has no module for,
**keyed by record** (the water well, added to `Foraging`, and the three placed
money props, which are the `Money` family on their own), with the item ids each
pays stored as derived data. Item 1 is money and the rule on it is two-way:
every family but `Money` refuses a record that pays it, and `Money` refuses a
record that pays anything else - so a yield slider can never become an economy
lever by accident, and the economy lever can never pick up a material. When
the clean table body is present (the plugin's dump or DMM's copy, chosen as
`items` chooses) the generator re-derives those from the bytes
and refuses to write on any mismatch; when it is absent it prints a banner
saying the rows were not verified. Records measured to be unreachable by a
table edit are kept in the same file as `records_not_enabled`, verified the
same way and emitted only as a test that keeps them out. `spec` has the format
and the reasons.

## Known defects

These were found during the port and deliberately **not** fixed, so they are
live. They are here rather than in a commit message because each one can waste
an afternoon.

* **`evidence` caches a failed request forever, and says nothing.** When a
  fetch fails, the error text becomes the body of that function's file, with a
  valid metadata line on it. Resume then treats the file as done and never
  re-fetches. Only `--force` recovers, nothing reports which files are
  affected, and `--force` redoes the whole tree. This is the most consequential
  defect in the directory: a tree exported while Ghidra was briefly unhappy has
  holes that look exactly like functions.
* **`sigscan` always exits 0.** A signature with zero hits, a signature with
  hundreds, and a static-info type name that went missing all print and exit 0.
  `just sigscan` cannot fail a script or CI. `fieldnames` has the same shape: a
  self-check `FAIL` prints the "do not trust anything downstream" banner and
  still exits 0.
* **`evidence`'s `graph.json` `calls` can name an address that has no entry in
  `nodes`** - a frontier callee that was never exported, or an address that is
  not a function at all. The current tree has 412 such edges out of 807. Anything
  joining the two must tolerate a dangling callee.
* **`docs/reference-items.md` promises something that has never been true.** It
  says "`sources` entries in `analysis/items.json` carry `list_items`". They do
  not, and never did: `grep -c list_items` is 0 in both JSON files, because the
  fold to `recs_by_key` drops it. The sentence survived the port because
  byte-identical output was the acceptance criterion. It is a known wrong
  promise awaiting a decision, not an oversight.
* **`xrefs` double-counts the REX forms.** One `48 8D 05 ...` is reported twice,
  as `lea` at the REX byte and `lea32` at the opcode, so a single instruction
  reads as "2 code reference(s)". The scanner cannot tell from the bytes alone
  which reading a decoder took and reports both on purpose; it is the summary
  line that misleads. Disassemble the address before believing a count of 2 -
  the `+0x569C7C0` example above is what this looks like in the wild.
* **`xrefs --selfcheck` has a blind spot at the tail.** The reference scanner
  stops 7 bytes short of the end of the image while the fast one does not, so
  the two are not compared over the last seven bytes. Harmless on this exe -
  they are zero padding past the final section - and reproduced deliberately
  from the Python so the oracle stays an oracle.
* **`xrefs` holds a ~380 MB image.** Several instances in parallel can hit
  ENOMEM. It already drops the raw file before scanning, which is what made the
  `--ignored` tests pass at all; running six copies by hand is still asking for
  it.
