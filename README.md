# Crimson Desert mods

Cargo workspace for my [Crimson Desert
Enhanced](https://store.steampowered.com/) mods (Steam build 25116796). They
load as `.asi` plugins through [Ultimate ASI
Loader](https://github.com/ThirteenAG/Ultimate-ASI-Loader) (`winmm.dll` in the
game's `bin64`).

| Mod | Version | What it does |
| --- | --- | --- |
| [desert-looter](desert-looter) | 0.1.1 | Auto-loot for gathering nodes |
| [desert-gatherer](desert-gatherer) | 0.1.0 | Gathering yield multiplier |

Also here: [`desert-core/`](desert-core), the shared library both plugins link
(logging, safe memory reads, hooks, PE/pattern scanning), and
[`dmm-pack/`](dmm-pack), the older offset-patch version of Gatherer for people
on Definitive Mod Manager without an ASI loader — never mount it alongside the
plugin.

## Install

Grab the zips from a release, or build them yourself, then copy the `.asi` and
its `.ini` into the game's `bin64`.

## Build

```bash
nix develop
cargo build --release
```

Outputs `desert_looter.dll` and `desert_gatherer.dll` under
`target/x86_64-pc-windows-gnu/release/`; install them renamed to
`DesertLooter.asi` and `DesertGatherer.asi`.

The `justfile` wraps the common tasks — run `just` to list them. `just install`
copies both plugins into `bin64` (path from `CD_BIN64`), `just dist` builds the
release zips, `just test` runs the tests, `just ci` runs everything.

Notes: keep the checkout on the Linux filesystem, building under `/mnt/c` is
slow. If flakes aren't enabled in your Nix config, add
`--extra-experimental-features 'nix-command flakes'`.

## Rules every plugin follows

1. **Never panic** — `panic = "abort"` is set, so a panic is a crash to
   desktop. Clippy denies `unwrap`, `expect`, unchecked indexing, `panic!` and
   friends in shipped code.
2. **Never dereference game memory** — all foreign reads go through
   `desert_core::safe`, and no pointer is cached across frames.
3. **Only run inside `CrimsonDesert.exe`** — the loader also pulls plugins into
   `crashpad_handler.exe`; bail out of `DllMain` there.
4. **No file I/O in `DllMain`** — the loader lock is held; work happens on a
   thread the plugin starts.

## More

[`VERSIONING.md`](VERSIONING.md) covers the versioning scheme and release
procedure. Changelogs: [desert-looter](desert-looter/CHANGELOG.md),
[desert-gatherer](desert-gatherer/CHANGELOG.md).
