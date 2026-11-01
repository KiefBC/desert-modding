# Build, test and just recipes

Sources: `justfile`, `flake.nix`, `.cargo/config.toml`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`, `desert-tooling/src/lib.rs` tests, `CLAUDE.md` (Commands).

## The toolchain

`nix develop` gives the Rust toolchain, the mingw linker, `just` and `mdbook`. Every `just` recipe also works from a plain shell: outside the dev shell it re-runs its commands through `nix develop --command` and exports the flakes `NIX_CONFIG` itself.

`.cargo/config.toml` makes `x86_64-pc-windows-gnu` the default target and holds the `[env]` block that keeps libstdc++ out of the plugin (`CXXSTDLIB=""`, `-fno-threadsafe-statics`). The linker and the `+crt-static` rustflags live in `flake.nix`, not in `.cargo/config.toml`: the `CARGO_TARGET_..._RUSTFLAGS` variable replaces any `target.<triple>.rustflags`, so every flag for that target must be in the flake.

## Recipes

`just` with no argument lists them.

| Group | Recipe | What it does |
| --- | --- | --- |
| Build | `build [crate]` | Release build of the plugin, or one crate |
| | `tools` | Build the host tools in `tools/` (own workspace) |
| | `dist [packages]` | Release zips and `SHA256SUMS` into `dist/` |
| | `install` | Copy `desert_tooling.dll` into `bin64` as `DesertTooling.asi`; refuses while the game runs, warns about pre-merge `.asi` files; `CD_BIN64` overrides the path |
| | `clean` | Remove build output and `dist/` |
| Test | `test` | `test-native` then `test-win` (Windows binaries through WSL interop) |
| | `test-game` | The `#[ignore]`d tests against the real exe and table |
| | `tools-check` | Tools clippy and unit tests |
| | `tools-check-game` | The tools' `#[ignore]`d tests (exe, table or a built plugin) |
| Lint | `clippy` | Both targets, `-D warnings` |
| | `audit` | RustSec check of `Cargo.lock` |
| | `check-imports` | Fail if the built plugin imports a DLL outside the allowlist |
| | `check-versions` | Fail if `README.md` / `VERSIONING.md` drifted from `Cargo.toml` |
| | `sync-versions` | Rewrite those version strings from `Cargo.toml` |
| | `ci` | `clippy test tools-check audit check-versions check-imports` |
| Game | `sigscan` | Re-check every byte signature and static-info type name against the exe |
| | `evidence [flags]` | Export Ghidra evidence into `evidence/` (needs Ghidra) |
| | `log` | Follow `DesertTooling.log` |
| | `yields` | Show the looter's learned yields cache |
| | `dmm-rebase [table] [build] [--dry-run]` | Rebase the DMM pack onto the current build |
| Misc | `logo` | Rasterise `assets/logo.svg` into the overlay's logo blob |
| | `docs` | Build this book into `target/book` |
| | `docs-serve` | Serve it on `127.0.0.1:3000` with live reload |

For one test, call cargo directly:

```bash
nix develop --command cargo test --target x86_64-unknown-linux-gnu -p desert-core ini::tests::key_names
```

## What the tests cover

- **Unit tests** on both targets. The cfg split (see [Workspace and rules](workspace.md)) is what lets most of them run natively.
- **Property tests**, `desert-core/tests/props.rs`: proptest over the byte parsers (`gimmick`, `pe`, `rtti`, `pattern`, `ini`). Arbitrary input never panics and the output is self-consistent. They stand in for fuzzing on stable. Add one for any new parser of game bytes.
- **Shipped-ini tests**, `desert-tooling/src/lib.rs`: every subsystem parses `DesertTooling.ini` back to its own defaults with no warning, every key of every section is present, and the sections are in menu order. They compare keys and defaults only, not comment or help text.
- **Game tests**, `#[ignore]`d: `desert-looter/tests/game_exe.rs` (2: the nine signatures resolve uniquely with the actor-manager vtable, and the one-pass scan agrees with one pattern at a time) and `desert-core/tests/gimmick_real.rs` (9: record loader, manager slots, catch site, the shared accessor scan, the DMM pack oracle, output-block item ids, indirect-template tables, the 149-accessor census, every named table's slot and loader). The pack oracle passes by skipping, with a banner, when no table is found. `just test-game` runs all eleven.

## CI and release

`ci.yml` runs on pull requests, pushes to `main` and Mondays 06:00 UTC: `just tools-check`, `just check-versions`, `just clippy`, `just test-native`, `cargo test --no-run` (Windows tests compile, not run), `just check-imports`. `just audit` is a separate job so an advisory does not redden a PR. Nothing that needs the game runs in CI.

`release.yml` runs on a `desert-tooling-v*` or `desert-gatherer-dmm-v*` tag. It checks that the tagged commit is on `main` and the tag matches the source version, reruns `check-versions`, `clippy` and `test-native`, builds only the tagged package, verifies `SHA256SUMS`, publishes the GitHub release, then a second job uploads the same zip to Nexus Mods per `tools/nexus-targets.json`. Launching the game with the build is a manual step before the tag; see [Versioning and releases](../repo/VERSIONING.md).

## After a game update

1. `just sigscan`. It always exits 0 (a known defect in [Tools](../repo/tools/index.md)), so read the output: each signature must hit once and every type name must occur once.
2. `just test-game` and `just tools-check-game`. A failure on a pinned address means re-pinning the test, not a broken plugin, if the plugin resolves at runtime.
3. If the table moved, re-pin the walk calibration (`tools/src/bin/gen-collect-names/table.rs` `CALIBRATION`, `tools/src/bin/items/checks.rs` `EXPECT_*`). Never override it.
4. Launch once with `[Gatherer] DumpTable=1` and `DryRun=1` to get a clean `DesertTooling.gimmickinfo.bin` for the tools.
5. Read the first log lines: `[looter] [sig] N found, 0 missing`, the three looter hook lines (`enqueue`, `area_sweep`, `[hunting] loot_grant`), `[gatherer] [gimmick] record loader at`, `[gatherer] [catch] hook at`, the dispatch slot lines and `[banking]`. Any `[gimmick] idx N: stream cursor ...` line means every gimmick record was left vanilla for that pass.

The worked example is `docs/findings/2026-09-24-build-25477059-retarget.md`.
