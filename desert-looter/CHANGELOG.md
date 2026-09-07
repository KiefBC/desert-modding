# Changelog

All notable changes to Desert Looter. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

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

## [0.1.0] - 2026-09-07

Game build 25116796. The first tagged release (`desert-looter-v0.1.0`): the plugin does what it
exists to do and has been verified in game on that build. Development before this was untagged and
unreleased; the working numbers used along the way are abandoned rather than renumbered.

### Added

- Automatic gathering of the four gather families — Foraging, Logging (the firewood cut from felled
  logs), Mining and Ore — by sending the game its own `PickUpItem` event, forged byte for byte, so
  nothing is simulated and no input is faked. Ore veins and rocks are mined outright, without a
  swing.
- Plain ground items (`GatherItems`) and, off by default, dropped weapons and armour (`GatherGear`).
  Every ground item is put to the game's own steal check first, so nothing the game would count as
  stealing is taken.
- A bag gate: `BagTab`, `StackLimit` and a full-bag check that mimics the game's own inventory
  behaviour — at a full bag only a stack the player already carries is grown, and repeated refusals
  switch auto mode off.
- Yield learning: `DesertLooter.yields` caches which item a node gives, learned while playing, and
  the stacking rule at a full bag reads it. It relearns itself if deleted.
- Hotkeys, all rebindable: F9 gathers the nearest eligible node once, F10 toggles auto mode, F11
  writes a survey of everything nearby to the log, and F7 is the event recorder — it records every
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
the bag rules. The workspace split came last — the plumbing shared with Desert Gatherer (logger,
guarded reads, hooks, PE/RTTI, patterns, ini, the gather-record table) moved to `desert-core` with
no behaviour change. None of this was tagged or released, and no version number from that period is
meaningful; 0.1.0 is where the history starts.
