# Changelog

All notable changes to Desert Looter. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

## [Unreleased]

### Added

- Per-family gather switches `GatherForaging`, `GatherLogging`, `GatherMining` and `GatherOre`,
  default `1`, the same kind of gate `GatherItems` and `GatherGear` are for ground items. When a
  family is switched off, the "nothing found" message now says how many nearby nodes were skipped
  for that reason.

### Changed

- The release zip now bundles Desert Overlay: `DesertOverlay.asi` and `DesertOverlay.ini` sit at
  the archive root beside the plugin's own four files, so extracting into `bin64` installs the
  in-game settings menu along with the plugin. Press `Insert` to open it. It is optional, changes
  nothing about the hotkeys or editing `DesertLooter.ini` by hand, and greys out the section of any
  plugin that is not loaded. Desert Overlay is versioned and released separately; a mod zip carries
  whichever version was current when the mod was released.
- `DesertLooter.ini` is now re-read while the game runs: the plugin thread checks the file's
  modified time once a second and reloads on change, logging `[ini] reloaded: ...` in the same
  format as the startup `[ini]` line. Every key is live, including the hotkey bindings. A file that
  is missing, empty or caught mid-write is skipped and retried on the next check. `AutoGather` only
  takes effect on a reload when its value actually changed, so reloading for another key does not
  undo an F10 toggle.
- `Enabled=0` no longer keeps the plugin out entirely. The two game hooks are always installed at
  load, since that can only be done safely while the game is still loading, and `Enabled` now gates
  everything else: scanning, gathering, hotkey actions, `[recv]` logging and the startup beep.
  Setting it back to `1` in the ini while the game runs brings the plugin back without a restart.

## [0.1.1] - 2026-09-07

Game build 25116796.

### Fixed

- At a full bag, a node whose item was already stacked in the bag was refused until the plugin had
  seen one pickup of that node type with a free slot ("yield of this node not learned yet"), which a
  full bag never allows. The stacking rule now reads the items a node can give from the node's own
  gimmick record (its resource-output list, `record+0x278` on this build) and requires a stack of
  every one of them; `DesertLooter.yields` only refines the amount. The first time a node type is
  considered at a full bag, the log lists what its record declares (`[yield] record N name
  declares ...`).
- A full bag logged only the first refusal reason per fill, so a ground item's "needs a free slot"
  hid the real reason for the gather nodes beside it. Each distinct reason is now logged once, with
  the node's name.

### Changed

- Clippy-clean on both build targets: nine lints fixed mechanically (`map_or` to `is_none_or` /
  `is_some_and`, range checks to `contains`, one redundant closure) and `# Safety` sections added to
  the two hook callbacks. No behaviour change.
- The never-panic rule is now enforced by the compiler: workspace lints deny `unwrap`, `expect`,
  unchecked indexing and slicing, `panic!` and undocumented `unsafe` blocks in shipped code, and the
  sites that relied on review (byte helpers, the heap-scan loops, the built-in signature parse) were
  rewritten to bail out instead. No behaviour change on any input the old code handled; inputs that
  would have panicked now take the function's existing failure path.

## [0.1.0] - 2026-09-07

Game build 25116796. The first tagged release (`desert-looter-v0.1.0`): the plugin does what it
exists to do and has been verified in game on that build. Development before this was untagged and
unreleased; the working numbers used along the way are abandoned rather than renumbered.

### Added

- Automatic gathering of the four gather families: Foraging, Logging (the firewood cut from felled
  logs), Mining and Ore. It works by sending the game its own `PickUpItem` event, forged byte for
  byte, so nothing is simulated and no input is faked. Ore veins and rocks are mined outright,
  without a swing.
- Plain ground items (`GatherItems`) and, off by default, dropped weapons and armour (`GatherGear`).
  Every ground item is put to the game's own steal check first, so nothing the game would count as
  stealing is taken.
- A bag gate: `BagTab`, `StackLimit` and a full-bag check that mimics the game's own inventory
  behaviour. At a full bag only a stack the player already carries is grown, and repeated refusals
  switch auto mode off.
- Yield learning: `DesertLooter.yields` caches which item a node gives, learned while playing, and
  the stacking rule at a full bag reads it. It relearns itself if deleted.
- Hotkeys, all rebindable: F9 gathers the nearest eligible node once, F10 toggles auto mode, F11
  writes a survey of everything nearby to the log, and F7 is the event recorder. It records every
  event the game queues until pressed again.
- `LogReceived`, default `0`. With `LogReceived=1` the plugin logs every item the game hands the
  player as `[recv] item <key> x<count>`, whether or not the plugin caused the pickup, capped at 500
  lines a session. It is how Desert Gatherer's yield numbers were measured.
- Pacing and reach settings: `GatherRange`, `GatherInterval`, `NodeCooldown`, `GatherUnarmed`,
  `ScanRange`, `Enabled`, `Debug`, and the four key names.

### Known limitations

- The signature scanning is content-based, but `desert-looter/src/tables.rs` still holds hard-coded
  RVA slots (`GIMMICK_INFO_SLOT` and friends) for build 25116796. They are what a game update breaks
  first.
- Creature catching is undecided: neither implemented nor ruled out.

## Pre-release history

Everything before 0.1.0 is development, not releases. The plugin was written on 2026-09-06 in a run
of untagged commits: the ASI skeleton and the eight byte signatures, the `ClientActorManager` RTTI
vtable lookup and record classification, the survey and single-gather hotkeys, then automatic
gathering, the unarmed-node path that mines veins outright, ground items with the steal check, and
the bag rules. The workspace split came last: the plumbing shared with Desert Gatherer (logger,
guarded reads, hooks, PE/RTTI, patterns, ini, the gather-record table) moved to `desert-core` with
no behaviour change. None of this was tagged or released, and no version number from that period is
meaningful; 0.1.0 is where the history starts.
