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
1980 value that underflows the DOS timestamp west of UTC). The looter and
gatherer zips also bundle the overlay's `.asi` and `.ini`.

**It starts by deleting `dist/`.** Do not run it expecting the previous build
to survive.

### `release-notes.py` (release workflow only)
Turns a tag such as `desert-looter-v0.1.1` into the release title, the package
name, and the release body pulled from that package's `CHANGELOG.md`. It exits
non-zero if the tag's version does not match the version in `Cargo.toml`, or if
the changelog has no matching `## [x.y.z]` heading. That second check is why a
new package cannot be released while its changelog still says
`## [Unreleased]`. Reads only; the workflow does the writing. Deliberately
stdlib-only so CI can run it before the Nix shell exists.

### `sync-versions.py` (`just sync-versions`, `just check-versions`, and CI)
Keeps the version numbers in `README.md`, `VERSIONING.md` and each shipping
crate's `README.md` in step with the crate `Cargo.toml` files. `--check` is
read-only and is what CI runs; without it the script rewrites the docs in
place. Stdlib-only, for the same reason as `release-notes.py`.

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
