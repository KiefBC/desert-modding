# Changelog

All notable changes to Desert Looter. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

## [0.2.0] - 2026-09-08

Game build 25116796. Verified in game on 2026-09-08: all eight signatures resolved
(`[sig] 8 found, 0 missing, 1 actor-manager vtable(s)`), the two info-manager slots resolved by
content, `DesertLooter.overlay.ini` was current, insects and fish were caught by hand and by auto
mode, and the session logged no warnings.

### Added

- **Catches insects and fish.** Auto mode and the gather key now also target
  the small creatures the game lets you catch by hand, inside the same
  `GatherRange`. A catch is not a pickup: it is the game's own
  `TrocTrPushCharacterToInventoryOnceTimer` event with an 8-byte payload,
  forged byte for byte from catches recorded live with `KeyRecord` (F7) on
  build 25116796. Fish use the identical event and payload; nothing about the
  send differs between the two. The steal check runs for a catch exactly as it
  does for a node or a ground item, so a creature the game counts as someone
  else's is refused and remembered. New keys `GatherBugs` and `GatherFish`,
  both default `1`, each switching its own kind off. Both are in Desert
  Overlay's menu as *Catch insects* and *Catch fish*, and no preset button
  touches either.

  **Only creatures of a class seen caught by hand are targeted.** Which kind
  a creature is comes from the interaction-category byte on its own status
  component (`ClientStatusActorComponent+0x5A`), which turns out to be a
  species class rather than per-actor noise: insects read `0x80` (six
  catches, four item ids), fish read `0x23` and `0x83` (four catches, item
  ids 29817, 29805 and 29804). Anything else is skipped, and says so once per
  class per session:

  ```text
  [gather] catchable creature cat=2C at 8 m is not a known bug/fish class; skipped (catch one by hand with F7 recording to add it)
  ```

  This replaces the first cut, which took any actor with type byte 6 and
  status kind 0. That rule would have grabbed birds in flight - lake surveys
  found them 9-25 m overhead reading `cat=20`, passing the type-and-status
  test perfectly - along with three unidentified ground and water species
  (`cat=2C`, `44`, `65`). The type byte still gates, because the class byte
  alone does not: type-05 characters read `cat=80`, `8C` and `90`.

  What to expect in the log:

  ```text
  [event] TrocTrPushCharacterToInventoryOnceTimer id=2048 payload=8 dispatch=1 (as expected)
  [gather] auto: bug cat=80 (Bug) eid=B01002C3 at 3.4 m -> request #7 parked
  [event] enqueued PushCharacterToInventory (Catch) for eid=B01002C3: event 0x...
  ```

  If the descriptor is not found the line reads
  `[event] TrocTrPushCharacterToInventoryOnceTimer: ...; bugs will not be caught`
  and everything else carries on unchanged.

  **Insects verified in game on build 25116796 (2026-09-08):** the first field
  session caught six insects in a row with no false positives - no non-insect
  was ever targeted - and all six read interaction category `cat=80`.
  **Fish verified in game the same day:** with `GatherFish=1` and every other
  switch off, auto mode caught **seven fish in a row** at 3.6-5.8 m, each one
  running the full path - `[gather] auto: fish cat=23 (Fish) eid=...`, then
  `[event] enqueued PushCharacterToInventory (Catch)`, then `[recv] item 29805
  x1` for the first and `[recv] item 29804 x1` for the other six, then
  `[gather] fish cat=23 eid=... gone after 0.1 s`. No false targets, no
  refusals and no `ownership unknown`. All seven read class `0x23`; `0x83` is
  on the fish list from hand catches, but the plugin has not yet been watched
  taking one. Two unknown classes were in range at 5 m, `cat=57` and `cat=2C`,
  and both were skipped with the once-per-class line exactly as designed.

  **The class lists grow from field logs, not from guesses.** A creature is
  recognised structurally, with no name lookup: type byte 6, status kind byte
  0, and a class byte on one of the two lists above. Only classes that have
  been caught by hand and logged are on those lists, so the failure mode is
  passing something over, never grabbing it. The survey (F11) prints `type=`,
  `cat=` and `status=` for every character-shaped actor, and the skip line
  names the class directly, which is all the evidence a new entry needs.
  The reference mod's own test for this (`ClientStatusActorComponent+0x273 == 6`)
  reads 0 on this build for a confirmed insect and is not used.

  **Type-3 creatures with a known class are included too (2026-09-08).** The
  Firefly Colony is caught by hand with the same event as every other insect
  but surveys as `type=03 cat=80`, where every creature caught before it read
  `type=06`; it was classified as a plain `Character` and never targeted. The
  type gate is now the set {3, 6} rather than the single value 6, so a colony
  is `Catchable` and is taken under `GatherBugs` like any other insect. The
  class byte still decides everything - type 3 is mostly NPCs (`cat=21`, `33`,
  `66`, `71`), and none of those is a known catch class, so nothing about
  which creatures are taken changes apart from the colony itself. Note that a
  colony grants a second, variable-count item by a separate path; both items
  do follow Desert Gatherer's `Bugs` multiplier, the second one with variance
  - see Desert Gatherer's changelog for what to expect from it.

  The pre-check in front of the game's own steal check moved with it. It used
  to run only for type bytes 4, 5 and 6, since those are the branch that reads
  a target's owner record; the status-component and transform reads that
  follow that branch happen for **every** non-gimmick type, so a type-3
  creature was reaching them unchecked. The owner-record check is still asked
  only of types 4-6, and the other two are now asked of everything except
  gimmicks, whose path is untouched.
- `Debug`, `LogReceived` and `BagTab` are now in Desert Overlay's menu, grouped at the bottom of
  the Desert Looter section under a `Diagnostics:` heading, and so are also present in a
  plugin-generated ini. `Debug` and `LogReceived` are checkboxes; `BagTab` is a number input over
  -1..15, where `-1` is the "auto" the parser has always accepted (the tab with the largest
  capacity). Nothing about what the keys do has changed - they were simply invisible to anyone who
  had not read the shipped ini.
- The plugin now writes `DesertLooter.ini` itself when the file is not there at all, with every
  key at its default and a short header saying where it came from. Dropping `DesertLooter.asi`
  into `bin64` on its own is now enough: the first launch leaves a file to edit, either by hand or
  from Desert Overlay's menu, instead of nothing. The commented template that explains every key
  still ships in the release zip and is still the better starting point; the generated file is
  bare. An existing ini is never read, rewritten or replaced, so this cannot touch your settings.
  Logged as `[ini] DesertLooter.ini was missing, so it was created with every key at its default`,
  or `[ini] WARN could not create DesertLooter.ini: ...; the defaults are in effect` if the folder
  is not writable - in which case the plugin runs on its built-in defaults exactly as before.
- Per-family gather switches `GatherForaging`, `GatherLogging`, `GatherMining` and `GatherOre`,
  default `1`, the same kind of gate `GatherItems` and `GatherGear` are for ground items. When a
  family is switched off, the "nothing found" message now says how many nearby nodes were skipped
  for that reason.
- `config::schema()` declares every ini key, its default and its widget for Desert Overlay's menu,
  including the four `Key*` bindings as key pickers and the four gather presets, which moved here
  from the overlay. The plugin writes it as `DesertLooter.overlay.ini` beside `DesertLooter.ini` at
  every start, logging `[schema] wrote ...` / `[schema] ... is current` / `[schema] WARN could not
  write ...`; editing that file has no effect, and the overlay redraws its section from it within a
  second of a change. This is what lets Desert Overlay draw the Desert Looter section without any
  Desert-Looter-specific code of its own.

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

### Fixed

- Gather nodes are now put to the game's own steal check exactly like ground items, which were the
  only thing asked about before. The fruit trees and food plants inside a settlement, which the game
  counts as stealing and offers as "Steal" rather than "Gather", are skipped instead of harvested;
  the same plants in the wild are unaffected. A node the game says is owned is logged as `[event]
  NOT sent for eid=...: the game says taking this would be stealing; skipped` and is left alone for
  the rest of the session.

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
