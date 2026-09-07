# Crimson Desert mods

One Cargo workspace holding my mods for **Crimson Desert Enhanced**, Steam
build 25116796. They load through [Ultimate ASI
Loader](https://github.com/ThirteenAG/Ultimate-ASI-Loader) (`winmm.dll` in the
game's `bin64`), which side-loads every `.asi` next to it.

| Crate / dir | Kind | Version | Ships |
| --- | --- | --- | --- |
| [`desert-core/`](desert-core) | rlib | 0.2.0 | nothing — it is linked into the plugins |
| [`desert-looter/`](desert-looter) | cdylib | 0.1.0 | `DesertLooter.asi` — auto-loot for gathering nodes. Working, in-game verified. |
| [`desert-gatherer/`](desert-gatherer) | cdylib | 0.1.0 | `DesertGatherer.asi` — gathering yield multiplier: it hooks the record loader and multiplies the yield scalars in memory. Working, in-game verified. |
| [`dmm-pack/`](dmm-pack) | data | 1.1 | "The Desert Gatherer", the DMM offset-patch mod that `desert-gatherer` supersedes. Kept for people who use DMM without an ASI loader; never mount it alongside the plugin. |

## Versioning

Both plugins follow Semantic Versioning, versioned and tagged separately
(`desert-looter-v0.1.0`, `desert-gatherer-v0.1.0`). Both are at 0.1.0: each
works in game for its core purpose on build 25116796, which is exactly what
0.1 is worth. `VERSIONING.md` carries a milestone ladder saying what each
MINOR level below 1.0 has to be earned with — 0.2 is a survived game update —
so a bump is evidence, not a mood. The version number is not the supported
game build; that is stated in each mod's README. See
[`VERSIONING.md`](VERSIONING.md) for that ladder, what counts as the public
interface, what bumps MAJOR / MINOR / PATCH, and the release procedure.
Per-mod changelogs: [`desert-looter/CHANGELOG.md`](desert-looter/CHANGELOG.md),
[`desert-gatherer/CHANGELOG.md`](desert-gatherer/CHANGELOG.md).

## desert-core

The plumbing that must not exist twice. Two of the worst bugs in this project
— both of which stopped the game from launching — were in the logger and in
the memory reads, so those live in exactly one place:

- `log` — crash-safe logger. Never panics (`panic = "abort"`, a panic kills
  the game), opens-appends-closes per write with full share flags, and is
  never called from `DllMain` or from a helper process. Each plugin names its
  own file via `log::init("DesertLooter.log")`.
- `safe` — every read of foreign memory goes through `ReadProcessMemory` on
  our own process, so an unmapped page fails instead of faulting. Nothing is
  cached; game memory is never dereferenced directly.
- `hook` / `trampoline` — inline trampoline hooks. `trampoline` builds the
  bytes and is unit tested natively; `hook` does the RWX allocation and the
  prologue patch.
- `pe`, `pattern`, `rtti`, `module` — PE32+ headers, IDA-style byte patterns,
  MSVC RTTI vtable lookup, and the running image as a slice.
- `ini` — the `Key=Value` tokeniser and the virtual-key name table. Each
  plugin keeps its own key list and defaults.
- `collect` — the 275 gather records of build 25116796 (key, name, family).
  Shared *data*: Looter classifies nodes with it, Gatherer multiplies yields
  for it.

`hook`, `hotkey`, `module` and `safe` are `#[cfg(windows)]`; everything else
compiles and tests natively on Linux.

## Building

```bash
nix develop
cargo build --release
```

Add `--extra-experimental-features 'nix-command flakes'` (or export
`NIX_CONFIG="experimental-features = nix-command flakes"`) if flakes are not
enabled in your Nix config. Outputs, both to be copied into the game's
`bin64`:

| Built | Install as |
| --- | --- |
| `target/x86_64-pc-windows-gnu/release/desert_looter.dll` | `DesertLooter.asi` |
| `target/x86_64-pc-windows-gnu/release/desert_gatherer.dll` | `DesertGatherer.asi` |

`.cargo/config.toml` makes `x86_64-pc-windows-gnu` the default target; the
linker and the per-target rustflags come from the dev shell in `flake.nix`.
Keep the checkout on the Linux filesystem — building under `/mnt/c` is slow.

A `justfile` wraps all of this: `just` lists the recipes (`build`, `test`,
`clippy`, `audit`, `ci`, `dist`, `install`, `test-game`, `sigscan`, `log`).
Each works inside `nix develop` and from a plain shell. `just install` copies
the built `.asi` files into `bin64` (path from `CD_BIN64`, default the Steam
install on `F:`) and refuses while the game is running.

Releasing: `just dist` (`nix develop --command tools/dist.sh`) builds both plugins and writes
`dist/DesertLooter-<version>.zip`, `dist/DesertGatherer-<version>.zip` and
`dist/SHA256SUMS` — each zip holding the `.asi`, its `.ini`, `README.md` and
`CHANGELOG.md` at the archive root, so it can be extracted straight into `bin64`
or handed to Definitive Mod Manager.

Tests (`just test` runs both targets):

```bash
cargo test --target x86_64-unknown-linux-gnu
```

The platform-independent half of every crate links natively, which is the
whole reason for the `#[cfg(windows)]` split and for the `rlib` on the plugin
crates. One ignored test scans the real game exe; it needs the Steam install
mounted:

```bash
cargo test --release --target x86_64-unknown-linux-gnu -p desert-looter --test game_exe -- --ignored --nocapture
```

## Rules every plugin here follows

1. **Never panic.** `panic = "abort"` is set, and an abort inside the game's
   process is a crash to desktop. Clippy enforces it: the workspace lints deny
   `unwrap`, `expect`, unchecked indexing and slicing, `panic!` and friends in
   shipped code, and every `unsafe` block carries a `// SAFETY:` comment.
   `desert-core/tests/props.rs` feeds the byte parsers arbitrary input.
2. **Never dereference game memory.** All foreign reads go through
   `desert_core::safe`, and no pointer is cached across frames. The first heap
   scan waits out a 20-second boot grace.
3. **Only run inside `CrimsonDesert.exe`.** The ASI loader is also pulled into
   `crashpad_handler.exe`; a plugin that finds itself there disables its log
   and returns from `DllMain` immediately.
4. **No file I/O in `DllMain`.** The loader lock is held; work happens on a
   thread the plugin starts.

`analysis/`, `docs/` and `source-mod/` hold reverse-engineering material and
are deliberately kept out of commits.
