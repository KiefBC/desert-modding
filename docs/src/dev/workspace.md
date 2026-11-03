# Workspace and rules

Sources: root `Cargo.toml`, the repo root listing, `clippy.toml`, each crate's `src/lib.rs`, `CLAUDE.md` (Architecture).

## Layout

```
desert-mods/
  Cargo.toml  Cargo.lock  justfile  flake.nix  clippy.toml  .cargo/
  crates/
    desert-core  desert-harvest  desert-dispatch  desert-overlay  desert-tooling
    tools               its own cargo workspace, excluded from the root one
  desert-gatherer-dmm/  the DMM pack (data, not a crate)
  docs/                 the mdBook (tracked) and the RE record (gitignored)
  vendor/hudhook/       the patched hudhook fork
  assets/  .github/
  analysis/  evidence/  dist/  target/    gitignored
```

Every Cargo crate lives under `crates/`, the usual Rust layout, so the root holds only what spans the workspace: the manifest, the `justfile`, `docs/`, the DMM pack and `vendor/`. Crate names in `-p` flags and release tags such as `desert-tooling-v0.10.0` are not paths and did not change when the crates moved on 2026-11-01.

## Crates

| Crate | Kind | Ships | Entry point |
| --- | --- | --- | --- |
| `desert-tooling` | cdylib | `DesertTooling.asi`, the only shipped binary | `DllMain`, the only one in the workspace |
| `desert-core` | rlib | nothing, linked in | none: plumbing (hooks, guarded reads, logger, PE/RTTI, byte patterns, ini, schema, the gather-record table) |
| `desert-harvest` | rlib | nothing, linked in | `start()`, never returns. Two halves as modules, `looter` (auto-loot) and `gatherer` (yield multipliers) |
| `desert-overlay` | rlib | nothing, linked in | `start(hmodule, sections)`, returns once its hooks are applied |
| `desert-dispatch` | rlib | nothing, linked in | `start()` |
| `desert-gatherer-dmm/` | data, not a crate | the DMM offset-patch pack | none |

`crates/tools/` is a separate cargo workspace of host binaries, excluded from this one (`exclude = ["crates/tools"]`), because everything here cross-compiles to Windows and the tools do not.

`desert-tooling` holds no policy. Its thread inits the log, seeds the ini if it is missing, migrates an old `[Looter]`/`[Gatherer]` pair into `[Harvest]` (`desert_harvest::migrate`), refuses to go on if a pre-merge `.asi` is loaded, and starts one named thread per subsystem (`desert-harvest`, `desert-dispatch`, `desert-overlay`). Each subsystem keeps its own ini `[Section]`, log tag and `schema()`. The looter and the gatherer share one of each: they act on the same in-game system, so they are one crate, one `[Harvest]` section, one `[harvest]` tag and one menu tab, with the two halves kept as separate modules. Anything that is plumbing rather than mod policy belongs in `desert-core`, so the logger and the memory reads exist once.

## The five rules

Each one corresponds to a bug that killed the game.

1. **Never panic.** `panic = "abort"` in `[profile.release]`, so a panic is a crash to desktop.
2. **Never dereference game memory.** Every foreign read goes through `desert_core::safe`, which uses `ReadProcessMemory` on our own process so an unmapped page returns `None`.
3. **Host-exe gate.** `DllMain` compares `current_exe()` with `CrimsonDesert.exe`, calls `log::disable()` and returns if it does not match (the loader also injects into `crashpad_handler.exe`).
4. **No file I/O in `DllMain`.** It calls `DisableThreadLibraryCalls`, then `CreateThread`, and returns.
5. **The logger opens, appends and closes per write** with `FILE_SHARE_READ|WRITE|DELETE`, and drops a write it cannot make.

Rule 1 is enforced by the compiler. `[workspace.lints]` denies `clippy::unwrap_used`, `expect_used`, `indexing_slicing`, `panic`, `unreachable`, `todo`, `unimplemented`, `undocumented_unsafe_blocks` and rustc's `unsafe_op_in_unsafe_fn`. Tests are exempt through `clippy.toml`; shipped `src/` never carries an `#[allow]` for these. Every `unsafe` block carries a `// SAFETY:` comment.

## Threads and `Enabled=0`

The harvest subsystem installs all its hooks even under `Enabled=0` and gates behaviour live on the flag: an inline hook is only safe to install while the game is still loading and nothing is executing the prologue, so there is no later moment to do it. Its one thread installs the gatherer's hooks first (record loader, catch count) and the looter's after (the signature scan, `area_sweep`, `enqueue`, `loot_grant`), because the record-loader hook goes in with no boot grace and must not wait behind the looter's scan or its 20-second grace. The gatherer's once-a-second work in the loop (pending table dump, re-apply, `[stat]`) sits above the `Enabled` gate and keeps running under `Enabled=0`. The dispatch subsystem installs nothing, so its `Enabled=0` at launch means nothing is read at all. The overlay under `Enabled=0` installs no graphics hook.

## cfg gating

Every crate splits its modules into always-compiled and `#[cfg(windows)]`, so `cargo test --target x86_64-unknown-linux-gnu` runs the pure logic natively. Put a module on the Windows side only if it needs Win32 or a live game pointer.

| Crate | Always | Windows only |
| --- | --- | --- |
| `desert-core` | `collect container_table creature drop_levers dropset_table gimmick ini log manager pattern pe quest_items rtti schema species_table telemetry trampoline` (`container_table`, `dropset_table`, `quest_items` and `species_table` are generated by `just gen-*`) | `hook hotkey module safe` |
| `desert-harvest` (root) | `config migrate` | `species`, and the private `entry` (the thread) |
| `desert-harvest::looter` | `config logfilter payload recorder` | `actors autogather events game hunting itemlog tables`, and the private `entry` |
| `desert-harvest::gatherer` | `config dump remember` | `catch hook`, and the private `entry` |
| `desert-overlay` | `config dynmodel logo rewrite store theme themes` | `cursor trace ui` |
| `desert-dispatch` | `buff config gate node remember` | `apply census scan` (private) |

`desert-tooling` is Windows-only almost end to end. hudhook and imgui are declared under `[target.'cfg(windows)'.dependencies]` in `crates/desert-overlay/Cargo.toml` and do not build natively.

## Logging and re-exports

`log!` expands to `log::write_tagged(crate::LOG_TAG, ...)`. Because `crate::` inside `macro_rules!` resolves at the call site, every crate declares its own `pub const LOG_TAG` (`tooling`, `core`, `harvest`, `overlay`, `dispatch`). Both halves of `desert-harvest` log as `[harvest]`.

Each subsystem's `lib.rs` re-exports the `desert-core` modules it uses under their own names (`desert-harvest`: `pub use desert_core::{collect, gimmick, ini, log, pattern, pe, rtti, schema, telemetry, trampoline}` plus `hook, hotkey, module, safe` on Windows), so `crate::log!` and `crate::safe::read` resolve in every module of both halves. This is deliberate: do not rewrite it into direct `desert_core::` paths. `crate::hook` is the core module, which the looter installs through; the gatherer's own `hook` module (the record-loader callback) is always `crate::gatherer::hook` and re-exports `desert_core::hook::{hex, install, Hook}`, and `catch` re-exports `install_raw` the same way. `desert-dispatch` omits `hook` and `trampoline` on purpose: it installs no hook, and a name that does not resolve guards against adding one by habit.
