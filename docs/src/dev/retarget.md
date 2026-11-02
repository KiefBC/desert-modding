# Re-targeting and analysis tools

Sources: `CLAUDE.md` (Re-targeting after a game update, Analysis tooling), `crates/tools/README.md`, `crates/tools/src/bin/retarget/`, `justfile`, `docs/findings/2026-11-01-build-26345091-retarget.md`. Evidence: `docs/reference-internals.md` §19 and §19.5.

Every tool here is a Rust binary in `crates/tools/`, built by `just tools`, offline, and seconds per run on the 330 MB exe. [Tools](../repo/tools/index.md) is the full manual. Ghidra is no longer part of the workflow: a fresh auto-analysis of this exe takes far longer than a whole re-target with these tools, and the last two re-targets used none.

## After a game update

The plugin finds everything by content, so an update breaks it only where the compiler's output changed under a byte pattern. The procedure is a checklist:

1. `just retarget report`. One markdown page: the headers, every signature's hit count, the accessor census, and a table of the constants that have to move. `Verdict: clean` means nothing does; otherwise it says what.
2. For each dead signature, `just retarget triage`. It prints the longest prefix and suffix of the old pattern that still hit, and the function start a suffix implies. Then, on what is left:
   - `retarget calls <rva> <len> [--callers]`: every `E8`/`E9` target in a range, with its `.pdata` status, its first bytes, an auto-wildcarded pattern and that pattern's exe-wide count. `--callers` adds each target's distinct calling functions.
   - `retarget callers <rva>`: every reference to an address grouped by the function it is in, with a pattern per function.
   - `retarget prologue <rva> [--stolen K] [--tokens]`: whole instructions, the RIP and branch flags, and the safe stolen-byte counts (at or past 12, before the first RIP-relative or branch instruction). `--stolen K` checks one and prints the bytes for a `*_PROLOGUE` constant, exit 2 if unsafe.
   A new signature is accepted only at an exe-wide count of 1, and only after the function's identity is confirmed.
3. `just retarget census`. The accessor template the gatherer and dispatch reach every table through: the four heads, the tail after each, the slot load and its bounds-check register, the loader-call shape and distance, the loader prologue, and whether each table name is in `crates/tools/static-info-names.txt`. `--sites` prints one row per site. Its numbers are what `crates/desert-core/src/gimmick.rs` and `crates/desert-core/tests/gimmick_real.rs` are written against, and `crates/tools/src/bin/retarget/expect.rs` pins them.
4. `just test-game` and `just tools-check-game`. A failing pin on an address is a re-pin, not a broken plugin, when the plugin resolves at runtime.
5. One launch with `[Harvest] DumpTable=1` and `DryRun=1`, then read the log (see below).
6. Re-pin the walk calibrations from the new table dump (`gen-collect-names` `CALIBRATION`, `items` `EXPECT_*`, `paths::CALIBRATED_BUILD`), never override them; then `just dmm-rebase` for the DMM pack.
7. Write the findings note, `docs/findings/<date>-build-<id>-retarget.md`, from the report.

The first lines of `DesertTooling.log` to read, all `[harvest]`, in this order: `[gimmick] record loader at`, `[hook] record loader ... -> stub`, `[catch] hook at`, `[sig] N found, 0 missing`, `[hook] area_sweep`, `[hook] enqueue`, `[hunting] loot_grant`; then the `[dispatch]` slot lines, its `verdict` and `[banking]`; and the `[dump] gimmickinfo table body ... bytes` line. Any `[gimmick] idx N: stream cursor ...` line means every gimmick record was left vanilla for that pass.

The 2026-11-01 update is the worked example: the retarget was derived on build 26345091, and Steam shipped 26356046 and then 26361232 the same day. Each hotfix cost one `just retarget report` and a re-pin of the tests (on 26361232 every signature moved by the same 0x50 and nothing needed re-deriving), which is what "a checklist" is meant to mean.

## Function boundaries without Ghidra

`retarget` takes function starts from the PE exception directory: data directory 3, an array of `RUNTIME_FUNCTION` records, with chained cold fragments followed to their parent (`crates/tools/src/funcs.rs`). A leaf function with no unwind data falls back to a prologue-shape heuristic, and every start printed says which source it came from.

**Never find anything by section name.** This exe scrambles them on every build, hotfixes included: the exception directory has been `.link` (25246367), all of `.trace` (25477059), `.edata` (26345091), `.rdata` (26356046) and `.shared` (26361232). `just retarget pe` prints the sections, the sixteen data directories and the exception directory's range and entry count (248667 on 26361232, 248666 on 26356046), and replaces the Python snippet that used to live in `CLAUDE.md`.

## The other offline tools

- **Table source**, in order: `CD_DMM_TABLE`; else `bin64/DesertTooling.gimmickinfo.bin` from a `DumpTable=1` + `DryRun=1` launch; else DMM's backup copy, which DMM deletes at will.
- **Pinned calibration**: `gen-collect-names` (`CALIBRATION`) and `items` (`EXPECT_*`) refuse a table that does not match their build's counts and anchors. After an update, re-pin; never override.
- `just sigscan`: every signature must hit once and every static-info type name occur once (221 names on 26361232). It always exits 0; `retarget triage` is the one with an exit code.
- `xrefs <rva>`: rel32 calls and jumps, RIP-relative `lea`/`mov`, and pointer cells holding the VA. `--selfcheck` diffs against the reference loop. One REX instruction reads as two references.
- `dis <start> <end>`: disassemble an RVA range, addresses as RVAs (add `0x140000000` for the VA).
- `fieldnames`: field names of every static-info record class from the game's Korean per-field read-failure strings, into `analysis/fieldnames.json` (§19.9).
- `sigscan` prints **file offsets**, `xrefs` and `dis` take RVAs. `retarget` takes either (`f:` marks a file offset) and prints RVAs.

## Appendix: Ghidra and `just evidence`

Optional, and only for `just evidence`, which exports one Markdown file per function into `evidence/` over GhidraMCP's HTTP API. `evidence/` as it stands is the frozen build 25116796 reference and is still useful to grep for what an old function did; nothing in the re-target needs it regenerated.

Ghidra runs on Windows with the upstream GhidraMCP extension, which binds the wildcard address so WSL reaches it on the host IP; GhidraMCP reads its port (8081 here) once at plugin construction and logs `BindException` to `application.log` if it is taken. "Connection refused" is the HTTP side; "Connection closed" is the `ghidra-mcp-bridge` process dying at launch, which must be in `environment.systemPackages`. Two traps in the project itself: `searchFunctions` cannot find a string constant (grep `evidence/` instead), and every function's phantom `[DATA]` xref is its own `RUNTIME_FUNCTION` record (§19.5).
