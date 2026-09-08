# Changelog

All notable changes to Desert Gatherer. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

## [Unreleased]

Game build 25116796. In-game verification of the changes below is pending.

### Added

- `DesertGatherer.ini` is now re-read while the game runs, the same mechanism as Desert Looter: the
  plugin thread checks the file's modified time once a second and reloads on change, logging
  `[ini] reloaded: ...`. A changed multiplier applies to records the game loads from then on; the
  game reloads its own gather table a few seconds after use, so the change is seen on the next
  gather.
- `config::schema()` declares every ini key, its default and its widget for Desert Overlay's menu.
  The plugin writes it as `DesertGatherer.overlay.ini` beside `DesertGatherer.ini` at every start,
  logging `[schema] wrote ...` / `[schema] ... is current` / `[schema] WARN could not write ...`;
  editing that file has no effect, and the overlay redraws its section from it within a second of a
  change. This is what lets Desert Overlay draw the Desert Gatherer section without any
  Desert-Gatherer-specific code of its own.

### Changed

- The release zip now bundles Desert Overlay: `DesertOverlay.asi` and `DesertOverlay.ini` sit at
  the archive root beside the plugin's own four files, so extracting into `bin64` installs the
  in-game settings menu along with the plugin. Press `Insert` to open it. It is optional, changes
  nothing about editing `DesertGatherer.ini` by hand, and greys out the section of any plugin that
  is not loaded. Desert Overlay is versioned and released separately; a mod zip carries whichever
  version was current when the mod was released.
- `Enabled=0` no longer keeps the plugin out entirely. The record-loader hook is always installed at
  load, and `Enabled=0` now makes it a pass-through that reads and writes nothing. Setting
  `Enabled=1` later in the ini turns the plugin on without a restart.

## [0.1.1] - 2026-09-07

Game build 25116796.

### Changed

- `DryRun` is now shipped as `0`. The dry-run-first install step is gone: the plugin multiplies
  yields on its first launch. `DryRun=1` stays available for troubleshooting and for checking what
  the plugin would do on a new game build. It was never the safety net it looked like: the record
  parser already refuses any output block whose markers do not match or whose vanilla minimum and
  maximum are implausible, so a layout shift after a game update leaves the record vanilla and says
  so in the log rather than writing garbage.
- The never-panic rule is now enforced by the compiler: workspace lints deny `unwrap`, `expect`,
  unchecked indexing and slicing, `panic!` and undocumented `unsafe` blocks in shipped code
  (`desert-core` included, whose byte parsers gained proptest properties). No behaviour change.

## [0.1.0] - 2026-09-07

Game build 25116796. The first tagged release (`desert-gatherer-v0.1.0`), verified in game on that
build. That is the whole of the evidence behind the number: it does what it exists to do, on one
game build, and has not yet seen a game update.

### Added

- Gathering yield multiplier as a single `.asi`. It hooks the game's `gimmickinfo` record loader and
  multiplies the minimum and maximum quantities in the raw record bytes as each record is loaded, so
  no game file is ever modified.
- Four independent multipliers, `1..100`, all defaulting to `1` (vanilla): `Foraging` (82 records),
  `Logging` (141), `Mining` (36, the `collect_mine` family) and `Ore` (16, the `collect_ore`
  family). Both the minimum and the maximum of every resource-output block are scaled, so the whole
  distribution moves.
- `DryRun`: shipped as `1` in `DesertGatherer.ini` so the first run is provably harmless. The hook
  logs a `[dry]` line per record with each block's old and new min/max and writes nothing. The
  built-in default is `0`.
- `Debug`: also log the records that are not gather nodes, capped at 400 lines.
- `Enabled`: `0` loads the plugin, writes one log line and hooks nothing.
- `[gimmick]`, `[dry]` and `[stat]` log lines: one per record as the game loads it, plus a running
  summary of the counters.
- The loader is found by content, through the accessor that names `gimmickinfo`, and the hook
  refuses to patch unless the twelve prologue bytes are exactly what it expects. A refusal is a
  plugin that does nothing and vanilla yields, never a wrong patch.

### Changed

- Replaces the DMM module pack "The Desert Gatherer" 1.1 (kept in `desert-gatherer-dmm/` for people
  who use DMM without an ASI loader). The two edit the same scalars and must not be mounted
  together: they multiply on top of each other. The same goes for DMM's built-in gathering
  multiplier preset.

### Known limitations

- Verified on build 25116796 only. The resolution is content-based by design, but no game update has
  yet tested that claim.
