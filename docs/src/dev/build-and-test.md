# Build, test and just recipes

Sources: `justfile`, `flake.nix`, `.cargo/config.toml`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `crates/desert-tooling/src/lib.rs` tests, `CLAUDE.md` (Commands).

## The toolchain

`nix develop` gives the Rust toolchain, the mingw linker, `just` and `mdbook`. Every `just` recipe also works from a plain shell: outside the dev shell it re-runs its commands through `nix develop --command` and exports the flakes `NIX_CONFIG` itself.

`.cargo/config.toml` makes `x86_64-pc-windows-gnu` the default target and holds the `[env]` block that keeps libstdc++ out of the plugin (`CXXSTDLIB=""`, `-fno-threadsafe-statics`). The linker and the `+crt-static` rustflags live in `flake.nix`, not in `.cargo/config.toml`: the `CARGO_TARGET_..._RUSTFLAGS` variable replaces any `target.<triple>.rustflags`, so every flag for that target must be in the flake.

## Recipes

`just` with no argument lists them.

| Group | Recipe | What it does |
| --- | --- | --- |
| Build | `build [crate]` | Release build of the plugin, or one crate |
| | `tools` | Build the host tools in `crates/tools/` (own workspace) |
| | `dist [packages]` | Release zips and `SHA256SUMS` into `dist/` |
| | `install` | Copy `desert_tooling.dll` into `bin64` as `DesertTooling.asi`; refuses while the game runs, warns about pre-merge `.asi` files; `CD_BIN64` overrides the path |
| | `clean` | Remove build output and `dist/` |
| Test | `test` | `test-native` then `test-win` (Windows binaries through WSL interop) |
| | `test-game` | The `#[ignore]`d tests against the real exe and table (`game_exe`, `gimmick_real`, `banking_real`) |
| | `tools-check` | Tools clippy and unit tests |
| | `tools-check-game` | The tools' `#[ignore]`d tests (exe, table or a built plugin) |
| Lint | `clippy` | Both targets, `-D warnings` |
| | `audit` | RustSec check of `Cargo.lock` |
| | `check-imports` | Fail if the built plugin imports a DLL outside the allowlist |
| | `check-versions` | Fail if `README.md` / `VERSIONING.md` drifted from `Cargo.toml` |
| | `sync-versions` | Rewrite those version strings from `Cargo.toml` |
| | `ci` | `clippy test tools-check audit check-versions check-imports` |
| Game | `retarget <subcommand>` | Re-target after a game update: `report`, `triage`, `census`, `calls`, `callers`, `prologue`, `pe` |
| | `sigscan` | Re-check every byte signature and static-info type name against the exe |
| | `evidence [flags]` | Optional: export Ghidra evidence into `evidence/` (needs Ghidra) |
| | `log` | Follow `DesertTooling.log` |
| | `yields` | Show the looter's learned yields cache |
| | `extract-tables [--out DIR]` | Every static-info table out of the game's archives into `analysis/tables/` |
| | `dmm-pack [--dry-run]` | Generate the DMM pack's 12 modules and `VERIFICATION.txt` from the archive table and `collect.rs` |
| Misc | `logo` | Rasterise `assets/logo.svg` into the overlay's logo blob |
| | `docs` | Build this book into `target/book` |
| | `docs-serve` | Serve it on `127.0.0.1:3000` with live reload |

For one test, call cargo directly:

```bash
nix develop --command cargo test --target x86_64-unknown-linux-gnu -p desert-core ini::tests::key_names
```

## What the tests cover

- **Unit tests** on both targets. The cfg split (see [Workspace and rules](workspace.md)) is what lets most of them run natively.
- **Property tests**, `crates/desert-core/tests/props.rs`: proptest over the byte parsers (`gimmick`, `pe`, `rtti`, `pattern`, `ini`). Arbitrary input never panics and the output is self-consistent. They stand in for fuzzing on stable. Add one for any new parser of game bytes.
- **Shipped-ini tests**, `crates/desert-tooling/src/lib.rs`: every subsystem parses `DesertTooling.ini` back to its own defaults with no warning, every key of every section is present, and the sections are in menu order. They compare keys and defaults only, not comment or help text.
- **Game tests**, `#[ignore]`d: `crates/desert-harvest/tests/game_exe.rs` (2: the nine looter signatures, `loot_grant` among them, resolve uniquely with the actor-manager vtable, and the one-pass scan agrees with one pattern at a time) and `crates/desert-core/tests/gimmick_real.rs` (11: record loader, manager slots, catch site, the shared accessor scan, `gimmick::multiply` doubling every `collect.rs` row against the archive table, the DMM pack oracle (every `*2X.json` edit equals what `gimmick::multiply` writes), output-block item ids, the record header shape, indirect-template tables, the 154-accessor census, every named table's slot and loader). The three table tests read `analysis/tables/` (`just extract-tables`, or `CD_TABLES`) and pass by skipping, with a banner, when it is absent. `crates/desert-dispatch/tests/banking_real.rs` (1: both `[banking]` gate signatures resolve exactly once). `just test-game` runs all fourteen.
- **Tool game tests**, `just tools-check-game`: the tools' `#[ignore]`d tests, among them `crates/tools/src/bin/retarget/expect.rs`, which pins `retarget census` and `triage` to the supported build. `crates/tools/tests/retarget_cli.rs` runs every `retarget` subcommand against a synthetic PE under plain `just tools-check`.

## CI and release

`ci.yml` runs on pull requests, pushes to `main` and Mondays 06:00 UTC: `just tools-check`, `just check-versions`, `just clippy`, `just test-native`, `cargo test --no-run` (Windows tests compile, not run), `just check-imports`. `just audit` is a separate job so an advisory does not redden a PR. Nothing that needs the game runs in CI.

`release.yml` runs on a `desert-tooling-v*` or `desert-gatherer-dmm-v*` tag. It checks that the tagged commit is on `main` and the tag matches the source version, reruns `check-versions`, `clippy` and `test-native`, builds only the tagged package, verifies `SHA256SUMS`, publishes the GitHub release, then a second job uploads the same zip to Nexus Mods per `crates/tools/nexus-targets.json`. Launching the game with the build is a manual step before the tag; see [Versioning and releases](../repo/VERSIONING.md).

## After a game update

The full procedure, with what each tool prints, is [Re-targeting and analysis tools](retarget.md). In short:

1. `just retarget report`. `Verdict: clean` means no signature and no template constant moved; otherwise the page says which, and is the skeleton of the findings note.
2. For what moved: `just retarget triage` (what is left of each dead signature), then `calls`, `callers` and `prologue` on it, and `census` for the accessor template. Accept a new signature only at an exe-wide count of 1, and a stolen count only from `prologue --stolen K`.
3. `just test-game` and `just tools-check-game`. A failure on a pinned address means re-pinning the test, not a broken plugin, when the plugin resolves at runtime.
4. Launch once with normal settings and read the first log lines, all `[harvest]` in this order: `[gimmick] record loader at`, `[hook] record loader ... -> stub`, `[catch] hook at`, `[sig] N found, 0 missing`, the three looter hook lines (`area_sweep`, `enqueue`, `[hunting] loot_grant`); then the dispatch slot lines, its `verdict` and `[banking]`. Any `[gimmick] idx N: stream cursor ...` line means every gimmick record was left vanilla for that pass.
5. Re-pin the walk calibration from the extracted table (`just extract-tables`; `crates/tools/src/bin/gen-collect-names/table.rs` `CALIBRATION`, `crates/tools/src/bin/items/checks.rs` `EXPECT_*`, `crates/tools/src/paths.rs` `CALIBRATED_BUILD`). Never override it. Then run `gen-collect-names` and diff `collect.rs`, then `just dmm-pack` and diff `desert-gatherer-dmm/`.
6. Write `docs/findings/<date>-build-<id>-retarget.md` and the ledger rows.

The worked examples are `docs/findings/2026-11-01-build-26345091-retarget.md` (the first with `retarget`, and a same-day hotfix that cost one report) and `docs/findings/2026-09-24-build-25477059-retarget.md`.
