<img src="assets/logo.png" alt="Desert Tooling goblin" width="96" align="left">

# Crimson Desert mods

Cargo workspace for my [Crimson Desert
Enhanced](https://store.steampowered.com/) mods (Steam build 25246367). They
load as one `.asi` plugin through [Ultimate ASI
Loader](https://github.com/ThirteenAG/Ultimate-ASI-Loader) (`winmm.dll` in the
game's `bin64`).

<br clear="left">

<p align="center">
  <img src="assets/readme_image.webp" alt="The Desert Tooling in-game menu: a Looter section with presets, gather families and ranges, and a Gatherer section with the four yield multipliers" width="560">
</p>

<p align="center"><em>The in-game menu (<code>Insert</code>): every setting, saved to <code>DesertTooling.ini</code> as you change it.</em></p>

| Mod | Version | What it does |
| --- | --- | --- |
| [desert-tooling](desert-tooling) | 0.5.0 | Auto-loot, gathering yields and the in-game menu, in one plugin |

One `DesertTooling.asi`, one `DesertTooling.ini`, one `DesertTooling.log`. It
carries four subsystems, each with its own section in that ini and its own tag
on every log line:

- **Looter** (`[Looter]`, `[looter]`) — auto-loot for gathering nodes.
- **Gatherer** (`[Gatherer]`, `[gatherer]`) — gathering yield multiplier.
- **Overlay** (`[Overlay]`, `[overlay]`) — the in-game settings menu, `Insert`.
- **Dispatch** (`[Dispatch]`, `[dispatch]`) — faster dispatch missions, bigger
  mission rewards, and a dump of both into the log.

They live in [`desert-looter/`](desert-looter),
[`desert-gatherer/`](desert-gatherer), [`desert-overlay/`](desert-overlay) and
[`desert-dispatch/`](desert-dispatch), which are internal libraries now: they build no `.asi`, are never tagged and
are never released on their own. Also here: [`desert-core/`](desert-core), the
shared plumbing they all link (logging, safe memory reads, hooks, PE/pattern
scanning), and [`desert-gatherer-dmm/`](desert-gatherer-dmm), the older
offset-patch version of the gathering multiplier for people on Definitive Mod
Manager without an ASI loader. Never mount it alongside the plugin.

## Install

Grab the zip from the latest release, or build it yourself, then copy
`DesertTooling.asi` and `DesertTooling.ini` into the game's `bin64`.

- **Desert Tooling**: [desert-tooling-v0.5.0](https://github.com/KiefBC/desert-modding/releases/tag/desert-tooling-v0.5.0)
- **Desert Gatherer (DMM pack)**: [desert-gatherer-dmm-v1.1](https://github.com/KiefBC/desert-modding/releases/tag/desert-gatherer-dmm-v1.1)

Older versions are on the [releases page](https://github.com/KiefBC/desert-modding/releases).

**Upgrading from the separate plugins:** delete `DesertLooter.asi`,
`DesertGatherer.asi` and `DesertOverlay.asi` from `bin64`, along with their
`.ini`, `.log` and `.overlay.ini` files. Nothing is migrated — the old ini files
are not read, and `DesertTooling.ini` is written with every key at its default
the first time the plugin runs. Leaving an old `.asi` in place is worse than
untidy: two copies of the same hook over one function crashes the game, so the
plugin refuses to install anything if it finds one still loaded, and says which
file to delete.

Editing the gathering multipliers from the in-game menu takes effect on the
**next gather**, not the next game start: the game still reads its whole gather
table once, about nine seconds after launch, but the plugin rewrites the records
it already loaded right after the ini change is picked up. How, and what the log
shows:
<https://github.com/KiefBC/desert-modding/blob/main/desert-tooling/README.md#how-a-changed-multiplier-becomes-live>.

The `[Dispatch]` settings edit tables the game re-reads from its own files every
launch, so they are not saved into your game: turn them back and the missions go
back, and removing the plugin leaves nothing to undo. The reward multiplier banks
nothing either — a reward still waiting on you is worked out from the table at
the moment it lands, so set `Rewards` back to 1 and it pays vanilla. **Nor, on
this game build, does anything else.** The game has code to bank a single
percentage figure for a finished mission in the save, part of it a bonus for
sending more workers than the mission needed — the one thing `AnyOperatorCount`
could have inflated, since it tells the game every mission needs only one
worker. Both switches that code sits behind are **off** on build 25246367, as
they were on 25116796 before it: no mission defers a payout, and the figure
ignores the worker count entirely. The plugin reads both at startup and says so
on the `[banking]` line of `DesertTooling.log` — check it after a game update,
because an update can flip them. If a future build did, it would be bounded to
the 142 of 936 missions that can repeat, it would clear itself as those rewards
landed, and it would not be a corrupted save; `Speed` and `NoSkillRequirement`
leave nothing behind at all either way. `DesertTooling.ini` says all of this
above the key.

The zip holds `DesertTooling.asi`, `DesertTooling.ini`, the README, the
CHANGELOG and the licence, flat at the archive root, so it can be extracted
straight into `bin64` or handed to Definitive Mod Manager as-is.

## Build

Developed on NixOS; the flake provides the Rust toolchain and the Windows
cross-compiler.

```bash
nix develop
cargo build --release
```

Outputs `desert_tooling.dll` under `target/x86_64-pc-windows-gnu/release/` —
the only cdylib in the workspace. The game loads it as `DesertTooling.asi`;
`just install` copies it into `bin64` under that name, and `just dist` packs it
into the release zip.

The `justfile` wraps the common tasks; run `just` to list them.

## Rules the plugin follows

1. **Never panic**: `panic = "abort"` is set, so a panic is a crash to
   desktop. Clippy denies `unwrap`, `expect`, unchecked indexing, `panic!` and
   friends in shipped code.
2. **Never dereference game memory**: all foreign reads go through
   `desert_core::safe`, and no pointer is cached across frames.
3. **Only run inside `CrimsonDesert.exe`**: the loader also pulls plugins into
   `crashpad_handler.exe`; bail out of `DllMain` there.
4. **No file I/O in `DllMain`**: the loader lock is held; work happens on
   threads the plugin starts.

## More

[`VERSIONING.md`](VERSIONING.md) covers the versioning scheme and release
procedure. Changelog: [desert-tooling](desert-tooling/CHANGELOG.md).

## License

MIT ([`LICENSE`](LICENSE)). Fork it, take pieces of it, ship your own version.
Keep the copyright notice with the source and you have met the only condition.
