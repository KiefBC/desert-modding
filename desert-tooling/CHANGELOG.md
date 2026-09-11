# Changelog

All notable changes to Desert Tooling. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

Desert Tooling is the merge of three plugins that shipped separately up to 0.2.0: Desert Looter,
Desert Gatherer and Desert Overlay. Their histories are kept below the 0.3.0 entry, one section
each, because the code did not change when they became subsystems and the reasons behind it are
still the reasons. Only `## [x.y.z]` headings name a release of *this* package.

## [0.4.0] - 2026-09-11

Game build 25116796.

### Added

- **A fourth subsystem, `[Dispatch]`.** It watches the dispatch missions ("faction
  operations") the game parses, and the reward rows those missions name in the `dropsetinfo`
  table, logs what is in them tagged `[dispatch]`, and edits five of their fields - four
  settings over five fields, because the reward multiplier writes both ends of a min/max
  pair - to whatever the ini asks for. It installs **no hook** and patches **no game code**:
  everything runs on its own thread over records the game has already parsed.

  It is on by default and costs a pointer read per record every two seconds. Set
  `[Dispatch] Enabled=0` before launch and nothing in the game is read at all - and that holds
  for the whole session: turning it back on mid-game, from the ini or from the menu, does nothing
  until the next launch, because there is no pass left to wake up. (`Enabled=0` *while the game
  runs* is the revert, and that does work both ways.) `LogRecords`, `MaxLines`, `Debug`,
  `DumpRaw` and `DumpRewards` are the diagnostic half; `MaxLines` caps every line one pass
  writes, raw hex dumps included, and a pass logs only the missions and reward rows it has not
  logged before.

- **Four settings that change dispatch missions**, all shipping at vanilla so a fresh install
  changes nothing until you ask:

  - `Speed` (1..100) divides every mission's duration. Vanilla runs 2h to 96h; `Speed=4` turns
    a 16h mission into a 4h one. No mission ever goes below 6 minutes. It reaches missions
    already under way: the game compares a mission's progress against the duration in the table
    on every tick, so lowering the duration can finish a mission that is already out.
  - `Rewards` (1..100) multiplies how much of each item a mission pays. Both ends of every
    reward range are scaled together, so 2-3 becomes 6-9 at x3, and only the 219 reward rows
    dispatch missions actually name are touched - never the game's wider drop table. It is not
    subject to the game's 10x reward clamp, which sits further downstream. It applies to rewards
    already waiting to be paid, because the game reads the amounts out of the table at the moment
    the items land.
  - `NoSkillRequirement` clears the skill a mission demands of an assigned worker, on the 147
    missions that demand one. The bonus a mission pays for a *skilled* worker is a different
    field two bytes away and is deliberately left alone.
  - `AnyOperatorCount` lets every mission start with a single worker. 693 of the game's 936
    missions already ask for one, so this changes the other 243.
  - `DryRun` logs every change that would be made and writes nothing, exactly like the
    `[Gatherer]` key of the same name.

  A change takes effect within about a second. `NoSkillRequirement` and `AnyOperatorCount` are
  checked when a mission *starts*, so a mission already out keeps running under the rules it
  started with - except a **repeating** mission, which re-checks on every restart and so stops
  with an error message at its next restart once the plugin is gone.

  Turning a setting back - or `Enabled=0` while the game runs - puts every mission and reward
  row back to the value it had before the plugin touched it: each one is remembered the first
  time the pass sees it and never re-derived, so applying twice is applying once and reverting
  is exact.

- One line per mission carries its node, key, group, duration in tenths of an hour and in hours,
  operator counts, combat power, step count, the skill it **requires**, the skill it pays a
  **bonus** for, the reward rows it names and its condition keys. The pass ends with the counts,
  a condition-key histogram, a skill census and a verdict saying whether the offsets still look
  like the right ones at all.

### Fixed

- The static-info accessor scan in `desert-core` saw only one of the four encodings the compiler
  emitted for the same template, reaching 111 of the game's 149 tables. All four are scanned now,
  which is what makes `FactionNode` and `dropsetinfo` reachable by content rather than by
  address.

### Note on saves

- **The `[Dispatch]` settings are not saved into your game.** The mission and reward tables are
  read from the game's own files every launch, so setting everything back to 1 and 0 restores
  vanilla, and removing the plugin leaves nothing to undo. `Rewards` banks nothing either: a
  reward still waiting on you is worked out from the table at the moment it lands, so set it back
  to 1, or remove the plugin, and what is waiting pays vanilla.

- **Nor, on this game build, does anything else.** The game has code to bank a single percentage
  figure for a finished mission in the save, and part of that figure is a bonus for sending more
  workers than the mission needed - which is the one thing `AnyOperatorCount` could have
  inflated, since it tells the game every mission needs only one worker. **Both of the switches
  that code sits behind are off on build 25116796**, measured in game: no mission defers a
  payout, and the figure ignores the worker count entirely. So nothing any `[Dispatch]` setting
  does reaches your save on this build.

  The plugin reads both switches at startup and prints them on the `[banking]` line of
  `DesertTooling.log`. **That line is the check after a game update**, because these are the kind
  of switch an update can flip, and it is the only thing that would tell you.

  The warning is kept rather than dropped because the code path is real. If a future build turned
  those switches on, finishing a mission with `AnyOperatorCount=1` would bank a bigger figure
  than you earned, on the 142 of the game's 936 missions that can repeat, and it would pay out
  later even with the plugin gone - bounded to the missions finished with the setting on,
  clearing itself as those rewards landed, and not a corrupted save. `Speed` and
  `NoSkillRequirement` are nowhere in that figure and leave nothing behind either way.
  `DesertTooling.ini`, both READMEs and the in-game menu all say so, and
  `docs/reference-internals.md` section 20.18 carries the measurement and its evidence level.

## [0.3.0] - 2026-09-08

Game build 25116796.

**Three plugins become one. You must delete the old files.** `DesertLooter.asi`,
`DesertGatherer.asi` and `DesertOverlay.asi` are replaced by a single `DesertTooling.asi`, their
three ini files by a single sectioned `DesertTooling.ini`, and their three logs by a single
`DesertTooling.log` whose every line carries the subsystem that wrote it. Nothing is migrated:
the old ini files are not read, and the new one is written with every key at its default the
first time the plugin runs.

### How to upgrade

1. Delete `DesertLooter.asi`, `DesertGatherer.asi` and `DesertOverlay.asi` from `bin64`, along
   with their `.ini`, `.log` and `.overlay.ini` files.
2. Extract this release into `bin64`: `DesertTooling.asi` and `DesertTooling.ini`.
3. Re-enter your settings, either in the ini or from the in-game menu (`Insert`). Your old
   multipliers and hotkeys are not carried over.

**Leaving an old `.asi` in place is worse than untidy.** Two copies of the same plugin patch the
same game function, and the second trampoline overwrites the first one's stolen bytes: that is a
crash to desktop, not a degraded mod. So the plugin now asks the Windows loader for
`DesertLooter.asi`, `DesertGatherer.asi` and `DesertOverlay.asi` before it installs anything, and
if any of them answers it installs *nothing*, starts no subsystem, and writes the file to delete
into the log:

```text
[tooling] REFUSING TO START: DesertLooter.asi still loaded in this process alongside DesertTooling.asi
```

The session then runs on the old plugin alone - playable, just not this one.

### Changed

- **One `.asi`.** `desert-tooling` is the workspace's only `cdylib` and the only `DllMain`.
  `desert-looter`, `desert-gatherer` and `desert-overlay` are `rlib`s linked into it, entered
  through `start()`, and are no longer versioned or released on their own. The host-exe gate
  (`crashpad_handler.exe` gets nothing), `DisableThreadLibraryCalls`, the hand-off to a thread and
  the never-panic rules are all in that one place now instead of three.
- **One ini, in sections.** `DesertTooling.ini` carries `[Looter]`, `[Gatherer]` and `[Overlay]`,
  and every key keeps the name, default, range and meaning it had. Each subsystem reads only its
  own section through the new `desert_core::ini::lines_in_section`, so the three `Enabled` keys -
  and the two `Debug`s, and `DryRun` - no longer collide. Each still polls the file once a second
  and picks its own section back up, exactly as before, and the in-game menu still edits it in
  place, replacing only the values of the keys it draws and only under the right header.
- **One log, tagged.** `DesertTooling.log` replaces the three. Every line now reads
  `[  12.345] [tid  1234] [gatherer] ...`, so `just log | grep '\[gatherer\]'` is what the three
  files were for. The tag comes from the calling crate's own `LOG_TAG`, which is what let hundreds
  of existing log call sites move without being touched.
- **No more `*.overlay.ini`.** The schema *file format* is gone, along with the overlay's
  once-a-second scan of `bin64` for it. Each subsystem still declares its settings as a
  `desert_core::schema::Section` in its own `config.rs` - that is still the one place a menu
  setting is added - but `desert-tooling` now collects all three at startup and hands them to the
  menu directly. Delete any `DesertLooter.overlay.ini` or `DesertGatherer.overlay.ini` you have;
  nothing writes or reads them any more.
- The menu no longer greys a section out as "not installed". Every subsystem ships in the one
  `.asi`, so the question answers itself; the mechanism is still there for a section that comes
  from somewhere else.
- The overlay's own settings (`Enabled`, `Debug`, `KeyMenu`, `ShowOnStart`, `Scale`, `FontSize`,
  `HdrBrightness`, `ColorSpace`, `Theme`) now have a menu section of their own, under `[Overlay]`,
  like the two subsystems it draws. They are still read once at startup, so a change to them still
  needs a restart, and the section says so. `Font` is the one key that stays hand-edited only: its
  value is a file name, an absolute path or an empty string, optionally suffixed `:N` for a `.ttc`
  face, which no menu widget can express. It is read as before and the menu leaves it alone.
- **A subsystem only reacts to its own section.** One file means the menu's write of a `[Looter]`
  key moves the modified time the gatherer's thread is watching, and vice versa. Each subsystem now
  compares the section it just parsed against the values it is already running on and does nothing
  when they match, so dragging a slider in one section no longer fills the log with another's
  `[ini] reloaded:` lines - or, worse, fires the gatherer's re-apply pass over every remembered
  record once a second for a setting that has nothing to do with it.
- The release zip is one package again: `DesertTooling.asi`, `DesertTooling.ini`, this changelog,
  the README and the licence, flat at the archive root. The DMM data pack
  (`DesertGatherer-DMM`) is unchanged and still has its own tag and its own zip.

### Added

- The stale-`.asi` guard described above, and a matching warning from `just install` when one of
  the three old files is still sitting in `bin64`.
- One thread per subsystem, started from `desert-tooling`'s own thread after the log is open and
  the ini has been seeded. The gatherer is started first and is never made to wait behind
  anything: it hooks the gimmickinfo record loader, which the game reads while the level is still
  loading, so the timing of the three separate DLLs is reproduced exactly rather than being
  serialised into one thread with the looter's 20-second boot grace in front of it.
- `DesertTooling.ini` is seeded from all three schemas at once when the file is absent
  (`schema::create_ini_if_missing_all`), before any subsystem thread starts - which is also what
  keeps the looter's once-a-second ini poll from firing a spurious reload on its first tick.
  The create is `create_new`, the OS's own atomic "only if absent", so an existing ini is never
  read, rewritten or replaced.
- The banner line names all four versions: `Desert Tooling 0.3.0 loaded, pid 1234 (looter 0.2.0,
  gatherer 0.2.0, overlay 0.2.0)`.
- A test in `desert-tooling` parses the shipped `DesertTooling.ini` - the very bytes that go into
  the release zip - with all three subsystems' own parsers and requires each one to read back its
  `Config::default()` with no warnings. Each plugin used to carry that test against its own
  template, where it repeatedly caught a default changed in code and not in the ini; with one
  shipped file it is stronger, because a key that landed under the wrong `[Section]` fails it
  twice over.

### Fixed

- The overlay's `ui.rs` and `trace.rs` wrote straight to the logger instead of through the tagged
  macro, so their lines - including everything hudhook says - came out untagged while every other
  line carried its subsystem. Invisible when the overlay had a log to itself; an inconsistency in
  a shared one. They are `[overlay]` now like the rest.

## Desert Looter, before the merge

The auto-loot subsystem, released on its own up to 0.2.0. Its `.asi`, its `DesertLooter.ini`
and its `DesertLooter.log` are what 0.3.0 above folded into `DesertTooling.asi`; everything
these entries describe about what it *does* is still true, under `[Looter]`.

### [0.2.0] - 2026-09-08

Game build 25116796. Verified in game on 2026-09-08: all eight signatures resolved
(`[sig] 8 found, 0 missing, 1 actor-manager vtable(s)`), the two info-manager slots resolved by
content, `DesertLooter.overlay.ini` was current, insects and fish were caught by hand and by auto
mode, and the session logged no warnings.

#### Added

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

#### Changed

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

#### Fixed

- Gather nodes are now put to the game's own steal check exactly like ground items, which were the
  only thing asked about before. The fruit trees and food plants inside a settlement, which the game
  counts as stealing and offers as "Steal" rather than "Gather", are skipped instead of harvested;
  the same plants in the wild are unaffected. A node the game says is owned is logged as `[event]
  NOT sent for eid=...: the game says taking this would be stealing; skipped` and is left alone for
  the rest of the session.

### [0.1.1] - 2026-09-07

Game build 25116796.

#### Fixed

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

#### Changed

- Clippy-clean on both build targets: nine lints fixed mechanically (`map_or` to `is_none_or` /
  `is_some_and`, range checks to `contains`, one redundant closure) and `# Safety` sections added to
  the two hook callbacks. No behaviour change.
- The never-panic rule is now enforced by the compiler: workspace lints deny `unwrap`, `expect`,
  unchecked indexing and slicing, `panic!` and undocumented `unsafe` blocks in shipped code, and the
  sites that relied on review (byte helpers, the heap-scan loops, the built-in signature parse) were
  rewritten to bail out instead. No behaviour change on any input the old code handled; inputs that
  would have panicked now take the function's existing failure path.

### [0.1.0] - 2026-09-07

Game build 25116796. The first tagged release (`desert-looter-v0.1.0`): the plugin does what it
exists to do and has been verified in game on that build. Development before this was untagged and
unreleased; the working numbers used along the way are abandoned rather than renumbered.

#### Added

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

#### Known limitations

- The signature scanning is content-based, but `desert-looter/src/tables.rs` still holds hard-coded
  RVA slots (`GIMMICK_INFO_SLOT` and friends) for build 25116796. They are what a game update breaks
  first.
- Creature catching is undecided: neither implemented nor ruled out.

### Pre-release history

Everything before 0.1.0 is development, not releases. The plugin was written on 2026-09-06 in a run
of untagged commits: the ASI skeleton and the eight byte signatures, the `ClientActorManager` RTTI
vtable lookup and record classification, the survey and single-gather hotkeys, then automatic
gathering, the unarmed-node path that mines veins outright, ground items with the steal check, and
the bag rules. The workspace split came last: the plumbing shared with Desert Gatherer (logger,
guarded reads, hooks, PE/RTTI, patterns, ini, the gather-record table) moved to `desert-core` with
no behaviour change. None of this was tagged or released, and no version number from that period is
meaningful; 0.1.0 is where the history starts.

## Desert Gatherer, before the merge

The gathering yield multiplier, released on its own up to 0.2.0. Its settings are the
`[Gatherer]` section now; nothing about the multipliers, the hooks or the re-apply pass
changed in the merge.

### [0.2.0] - 2026-09-08

Game build 25116796. The catch multipliers were verified in game on 2026-09-08 with `Bugs=10` and
`Fish=10`: the record-loader and catch hooks both installed, and the session logged 14
`[catch] bug class=80 -> x10` and 12 `[catch] fish class=23 -> x10` lines with no warnings.

The live re-apply was verified in game on 2026-09-08: changing `Foraging`
from the overlay while playing logged `[live] re-applied ... 82 records rewritten, 193 unchanged,
0 skipped; 644 scalars written` within 150 ms of the ini change, the next gather paid out at the
new rate, and setting it back to 1 restored vanilla the same way.

#### Added

- **Bugs and Fish: two more multipliers, for the creatures caught by hand.**
  Insects and fish are not gather nodes - they are characters taken with
  `TrocTrPushCharacterToInventoryOnceTimer`, and the amount granted is a hard-coded
  constant in the game's code, not a field of any record. So this pair is not a table
  edit like the four families: it is a 13-byte inline patch on the instruction that
  loads that constant (`docs/reference-internals.md` section 17), installed at startup
  after the plugin has scanned for a 21-byte signature that hits exactly once and
  checked, byte for byte, that what it is about to overwrite is the `mov r8d,1` and the
  `lea` it expects. A stub calls back into the plugin with the creature's actor, the
  callback reads the type byte and the interaction class byte through
  `desert_core::safe`, and the value it returns becomes the count. New keys `Bugs` and
  `Fish`, both `1..100`, both defaulting to 1, both in the in-game menu under the same
  **Yield multipliers:** heading as the four families.

  Unlike the families, a change here needs no re-apply pass of any kind: the multiplier
  is read at the instant of the catch, so it takes effect on the **next catch**.

  Log lines to expect: `[catch] hook at +0x2A74151 -> stub 0x...; original bytes: 41 B8
  01 00 00 00 48 8D 95 D0 01 00 00` once at startup, then one `[catch] bug class=80 ->
  x3` or `[catch] fish class=23 -> x3` per multiplied catch (capped at 500 lines a
  session). `DryRun=1` logs `[dry] fish class=23 would be x3, granting 1` and grants
  one; `Enabled=0` grants one silently.

  Only insects and fish the plugin has a **recorded hand catch** for are multiplied -
  class bytes `0x80` (insects) and `0x23`/`0x83` (fish), the same lists Desert Looter
  targets from. Anything else that reaches the same code is left completely alone and
  reported once per class per session (`[catch] class=2C not a known bug/fish class;
  vanilla`). That matters because the function this patches has five callers and only
  one of them is the catch event; the class gate is what keeps the multiplier off the
  other four.

  **Type-3 creatures with a known class are multiplied too (2026-09-08).** The Firefly
  Colony is caught by hand with the same event as every other insect but its actor reads
  `type=03 cat=80`, where every creature caught before it read `type=06`; the hook saw a
  type it did not recognise, returned the vanilla 1 and said nothing at all, so `Bugs=10`
  quietly did nothing for it. The type gate is now the set {3, 6} rather than the single
  value 6, and the class byte still decides everything, so the colony's first item now
  follows the `Bugs` multiplier exactly. **Its second item, the variable one, is verified
  to scale too** (measured in game, `docs/reference-internals.md` section 17.10.7): the
  game rolls it once per unit of the first item, so the multiplier turns one roll of its
  own 1-3 drop into N independent rolls. That lands it at roughly **twice** the slider on
  average, with real variance rather than a fixed number - `Bugs=10` gave 19, 20, 22 and
  27 across four colonies where vanilla gave 1 or 2. Nothing there needs fixing; it is
  the game's own drop rule applied to a multiplied count.

  To stop that costing a field session again, a **known** bug or fish class byte arriving
  on a type outside the set is now reported once per `(type, class)` pair per session:

  ```text
  [catch] type=05 class=80 is a known class on a non-catchable type; vanilla (logged once)
  ```

  Nothing is multiplied on the strength of that line - the creature is still granted
  vanilla - and everything else stays as silent as it was, so the four non-event callers
  of the patched function still say nothing.

  **The game-update caveat, stated plainly: this is the one part of Desert Gatherer that
  is a patch on code rather than on data, so it is the first thing a game update breaks.**
  When it does, the plugin refuses to patch and says so once (`[catch] signature not
  found; catch multipliers off`, or a line printing the bytes it found instead of the
  ones it wanted), and everything else - the four family multipliers, the live re-apply -
  carries on unaffected.

  **Verified in game on 2026-09-08**, build 25116796, with `Bugs=10` and `Fish=10`. Three
  insects caught by hand before auto mode was switched on each arrived as a single
  `[recv] item 1001323 x10`; auto mode then took three more insects (`1001245` once,
  `1001323` twice) and seven fish - six of class `0x23` (`29805`) and one of class `0x83`
  (`29816`), the first class-`0x83` fish the plugin has taken itself - every one of them
  at `x10`. Each count arrived as **one** `xN` receipt: the game's inventory add neither
  clamps a count above 1 nor splits it into separate `x1` lines, so the multiplied amount
  lands in a single pickup. Every creature vanished normally, 0.1-1.2 s after the catch.
  The signature, its uniqueness and the thirteen stolen bytes are checked against the real
  `CrimsonDesert.exe` by `just test-game` and `just sigscan`, and every byte of the stub
  was verified against a disassembler.

  **The multiplier applies to catches you make by hand too**, not only to the ones Desert
  Looter takes for you - the patch sits in the game's own grant routine, downstream of
  whatever asked for the catch. That is intended behaviour, not a side effect to work
  around: `Bugs=10` means ten insects per insect, however you caught it.

- **A changed multiplier now reaches the game on the next gather, not the next launch.** For every
  gather record the load-time hook already remembers the vanilla minimum, maximum and item id of
  each output block, plus the record's index, in a fixed table - even at `Enabled=0` and `1x`,
  since vanilla is the only fixed point a later change can be computed from. When
  `DesertGatherer.ini` changes, right after the `[ini] reloaded:` line the plugin walks those
  remembered records through the game's own record manager, finds each already-parsed object and
  its output list, checks the record's key and each block's item id against what was remembered,
  and writes vanilla times the current multiplier - never a value already sitting in the block
  scaled again. Blocks already at the wanted value are left alone, and a mismatch (not loaded yet,
  key or item id changed, list count changed, unreadable) skips that record or block rather than
  guessing. `Enabled=0` now means vanilla yields even for records the game already loaded;
  `DryRun=1` logs what would be written and writes nothing. Logged as `[live] re-applied
  Foraging=10 Logging=1 Mining=1 Ore=1: 82 records rewritten, 193 unchanged, 0 skipped; 246
  scalars written` (`[dry]` under `DryRun`, and `Foraging=1 Logging=1 Mining=1 Ore=1 (Enabled=0)`
  at `Enabled=0`), with a `[live] WARN ...` line naming the reasons whenever something was
  skipped, and, under `Debug=1`, one line per rewritten record.

#### Fixed

- The README, the shipped ini's header and the menu notice originally said a changed multiplier
  shows on the next gather because the game reloads its table a few seconds after use. That was
  never true: the game reads all 13,906 `gimmickinfo` records in one preload pass about nine
  seconds after launch and keeps the parsed objects for the whole session, and the load-time hook
  only runs inside that read. Earlier in this cycle the docs were corrected to say "next game
  start" instead, which was accurate for what the plugin did at the time. The re-apply mechanism
  under Added above makes "next gather" true again, this time for real, so the docs now say that
  throughout.

#### Changed

- **The shipped `DesertGatherer.ini` now sets all four multipliers to `1` (vanilla) instead of
  `2`.** The plugin's built-in default has always been `1`, and the README's settings table has
  always documented `1`, so the template was the odd one out: installing the mod quietly doubled
  every yield before you had chosen anything. Raise the families you want, in the ini or from
  Desert Overlay's menu. **If you extract a new release zip over an existing install it replaces
  your `DesertGatherer.ini`**, so back it up first or re-enter your multipliers afterwards - this
  is true of any release, but it is the one that will change your yields.

#### Added

- `Debug` is now in Desert Overlay's menu, at the bottom of the Desert Gatherer section under a
  `Diagnostics:` heading, and so is also present in a plugin-generated ini. `DryRun` stays where it
  was, near the top. Nothing about what the key does has changed - it was simply invisible to
  anyone who had not read the shipped ini.
- The plugin now writes `DesertGatherer.ini` itself when the file is not there at all, with every
  key at its default and a short header saying where it came from. Dropping `DesertGatherer.asi`
  into `bin64` on its own is now enough: the first launch leaves a file to edit, either by hand or
  from Desert Overlay's menu, instead of nothing. The commented template that explains every key
  still ships in the release zip and is still the better starting point; the generated file is
  bare. An existing ini is never read, rewritten or replaced, so this cannot touch your settings.
  Logged as `[ini] DesertGatherer.ini was missing, so it was created with every key at its
  default`, or `[ini] WARN could not create DesertGatherer.ini: ...; the defaults are in effect`
  if the folder is not writable - in which case the plugin runs on its built-in defaults exactly
  as before.
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

#### Changed

- The release zip now bundles Desert Overlay: `DesertOverlay.asi` and `DesertOverlay.ini` sit at
  the archive root beside the plugin's own four files, so extracting into `bin64` installs the
  in-game settings menu along with the plugin. Press `Insert` to open it. It is optional, changes
  nothing about editing `DesertGatherer.ini` by hand, and greys out the section of any plugin that
  is not loaded. Desert Overlay is versioned and released separately; a mod zip carries whichever
  version was current when the mod was released.
- `Enabled=0` no longer keeps the plugin out entirely. The record-loader hook is always installed at
  load, and `Enabled=0` now makes it a pass-through that reads and writes nothing. Setting
  `Enabled=1` later in the ini turns the plugin on without a restart.

### [0.1.1] - 2026-09-07

Game build 25116796.

#### Changed

- `DryRun` is now shipped as `0`. The dry-run-first install step is gone: the plugin multiplies
  yields on its first launch. `DryRun=1` stays available for troubleshooting and for checking what
  the plugin would do on a new game build. It was never the safety net it looked like: the record
  parser already refuses any output block whose markers do not match or whose vanilla minimum and
  maximum are implausible, so a layout shift after a game update leaves the record vanilla and says
  so in the log rather than writing garbage.
- The never-panic rule is now enforced by the compiler: workspace lints deny `unwrap`, `expect`,
  unchecked indexing and slicing, `panic!` and undocumented `unsafe` blocks in shipped code
  (`desert-core` included, whose byte parsers gained proptest properties). No behaviour change.

### [0.1.0] - 2026-09-07

Game build 25116796. The first tagged release (`desert-gatherer-v0.1.0`), verified in game on that
build. That is the whole of the evidence behind the number: it does what it exists to do, on one
game build, and has not yet seen a game update.

#### Added

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

#### Changed

- Replaces the DMM module pack "The Desert Gatherer" 1.1 (kept in `desert-gatherer-dmm/` for people
  who use DMM without an ASI loader). The two edit the same scalars and must not be mounted
  together: they multiply on top of each other. The same goes for DMM's built-in gathering
  multiplier preset.

#### Known limitations

- Verified on build 25116796 only. The resolution is content-based by design, but no game update has
  yet tested that claim.

## Desert Overlay, before the merge

The in-game menu, released on its own up to 0.2.0. It is the `[Overlay]` section now, and it
is handed its sections at startup instead of discovering `*.overlay.ini` files - see 0.3.0
above for what that removed.

### [0.2.0] - 2026-09-08

Game build 25116796. **The first published release.** 0.1.0 was prepared and verified but never
tagged, so this is the version that reaches players first. Its entry in
[the changelog](CHANGELOG.md) is where the menu itself, the fonts, the themes, the HDR handling
and the vendored hudhook are described; the notes below cover only what changed after it.

Verified in game on 2026-09-08: the menu drew and took input, the cursor hooks installed
(`[cursor] ClipCursor and SetCursorPos are hooked`), both `DesertGatherer.overlay.ini` and
`DesertLooter.overlay.ini` were discovered and drawn as sections, and the session logged no
warnings.

#### Changed

- The menu no longer hard-codes Desert Looter's and Desert Gatherer's ini keys, defaults, ranges
  and widgets. Each plugin now declares a `desert_core::schema::Section` in its own `config.rs` and
  writes it as `<Name>.overlay.ini` beside its ini at every start; the overlay scans `bin64` for
  `*.overlay.ini` once a second, parses each through the new `desert_core::schema` module, and
  builds one collapsible section per file (`sections.rs` for discovery, `dynmodel.rs` for the
  values, replacing `model.rs` and `presets.rs`). Widgets, labels, ranges, headings, same-line rows,
  tooltips, presets and the notice line all now come from the schema, so a new mod appears in the
  menu by shipping a schema file - this crate needs no change. The header, the logo, the fonts, the
  themes and HDR handling are untouched.
- Desert Looter's section gained the four `Key*` bindings (`KeyToggle`, `KeyScan`, `KeyGather`,
  `KeyRecord`) as key pickers over the same key names the ini accepts; they were not editable from
  the menu before.
- New log lines: `[schema] <file>: <title>, N fields, M presets` when a section loads, `[schema]
  WARN <file>: <why>` when a schema file will not parse (once per modified time), `[schema] <file>
  is gone; <title> left the menu` when one disappears, and `[schema] no *.overlay.ini beside the
  game exe; the menu has nothing to show` when nothing is found. An empty menu now shows one dim
  line saying so instead of an empty window.

#### Fixed

- The mouse pointer could not move while the menu was open: pressing the menu key drew the menu
  but left the pointer pinned in place, and the only way to free it was to press the Windows key
  to leave the game, click, and come back (which worked because a window that has lost and
  regained focus with its input blocked by the overlay does not resume pinning until the menu
  closes). The game claims the hardware cursor every frame for camera control, through user32
  `ClipCursor` (clip to a point or rect) and `SetCursorPos` (re-centre it), and hudhook feeds
  imgui's pointer from both raw-input deltas (`WM_INPUT`) and absolute `WM_MOUSEMOVE`
  coordinates, so every re-pin snapped the imgui pointer straight back to the same spot. The
  overlay already released the clip once when the menu opened, but the game re-applied it on the
  next frame. `desert-overlay/src/cursor.rs` now installs two MinHook inline hooks on user32's
  `ClipCursor` and `SetCursorPos` right after `Hudhook::apply` succeeds (the builder initialises
  MinHook and `apply` enables its hooks, so this is the first moment ours can go in): while the menu is open, `ClipCursor` is forwarded with a null rectangle
  (no clip) and `SetCursorPos` returns success without moving anything; while it is closed, both
  pass straight through. This is the same fix ReShade's own overlay uses (`HookClipCursor` /
  `HookSetCursorPos` in its `input.cpp`), which is why ReShade's menu never had the problem. New
  log lines: `[cursor] ClipCursor and SetCursorPos are hooked; the game cannot pin the pointer
  while the menu is open` on success, and `[cursor] WARN could not hook <what>: <why>; the
  pointer may not move while the menu is open (Win key out and back frees it)` on failure, which
  is not fatal - the menu still works, just with the old symptom.
- `DesertOverlay.ini` was never created for a new install: Looter and Gatherer each seed a
  missing ini from their schema (`schema::create_ini_if_missing`), but the overlay only logged
  `DesertOverlay.ini not found, using defaults` and ran on defaults, so a user had no file to
  edit unless they copied one out of the zip by hand. `main_thread` now writes the shipped, fully
  commented `DesertOverlay.ini` (embedded with `include_str!`) beside the exe if it is absent,
  using an atomic create-only open (`create_new`), so an existing file is never read, rewritten
  or replaced. It can't go through the schema path the other two use: the overlay's own `Font`
  key is free text, which `desert_core::schema::Kind` cannot express, and none of its keys are
  live in the first place. New log line: `[ini] DesertOverlay.ini was missing, so it was created
  with every key at its default`; on failure, `[ini] WARN could not create <path>: <err>; the
  defaults are in effect`. Nothing changes for anyone who already has the file.

### [0.1.0] - 2026-09-08

Game build 25116796. Verified in game on 2026-09-07: the menu draws, moves and resizes, a preset
click reaches `DesertLooter.ini` and Desert Looter reloads it within a second.

#### Added

- The menu window is titled "Desert Tooling" and opens with a header row: the goblin logo beside
  the name in a larger size of the menu font. The picture is embedded in the plugin as raw RGBA
  pixels, generated from `assets/logo.svg` by `tools/logo-to-rgba.py` (`just logo`); it is uploaded
  to the renderer once per graphics pipeline, and a failed upload is a warning in the log and a
  header with only its title.
- The menu is drawn in a real TrueType font instead of Dear ImGui's 13 px bitmap face blown up by
  the scale factor, which is what made it hard to read on a 4K display. Two ini keys steer it:
  `FontSize` (pixels before `Scale`, default `20`, `8` to `72`) and `Font` (default `segoeui.ttf`,
  a bare name looked up in `%WINDIR%\Fonts`, an absolute path taken as it stands, empty for the
  built-in font, and an optional `:N` suffix to pick a face out of a `.ttc` collection).
  `georgia.ttf`, `constan.ttf` and `cambria.ttc:0` are the serif options. The file is read once on
  the plugin's own thread; a missing or unreadable one is a warning in the log and the built-in
  font, never a failure to draw.
- Colour themes. A `Theme` picker at the top of the menu switches the look live and logs the name
  to put in the ini; the `Theme` ini key (default `banner`, gold lettering on the game's black banner; `classic` is the stock Dear ImGui dark look) makes
  it stick. Five themes drawn from the game's key art (off-white poster ground, antique gold
  lettering, the blood-red splash, the black banner, steel armour): `parchment`, `gilded`,
  `splash`, `banner` and `steel`. Themes are plain data, one file each under
  `src/themes/`, checked by unit tests for range and unique names.
- A section whose plugin is not loaded in the game is greyed out and titled "not installed", with
  a note under the header. The check asks the Windows loader for `DesertLooter.asi` and
  `DesertGatherer.asi` once a second, so a file that is present but failed to load counts as
  absent. The presets follow the Desert Looter section.
- `Scale` ini key: menu size, `0` follows the Windows display scaling, otherwise a fixed factor
  from `0.5` to `4`. Fonts, spacing and the window geometry all follow it.
- First version of the plugin: an in-game settings menu for Desert Looter and Desert Gatherer,
  drawn with Dear ImGui inside the game's DirectX 12 frame (hudhook 0.9.2) and toggled with
  `Insert` (`KeyMenu`).
- Desert Looter section: `Enabled`, `AutoGather`, the four gather families, `GatherItems`,
  `GatherGear` and `GatherUnarmed` as checkboxes; `ScanRange` and `GatherRange` as sliders;
  `GatherInterval`, `NodeCooldown` and `StackLimit` as number fields. Every widget's range is the
  range the plugin accepts, so the menu cannot write a value that would be refused.
- Desert Gatherer section: `Enabled` and `DryRun` as checkboxes, and the four yield multipliers as
  1..100 sliders.
- Presets: `Everything`, `Plants only`, `Wood only`, `Rock and ore only`. Each sets the four gather
  families and `GatherItems` and leaves every other setting alone.
- Live in both directions. The ini files are the only channel to the plugins: a change in the menu
  is on disk within 250 ms and the plugins apply it within about a second, and a file edited by hand
  while the game runs shows up in the menu within a second.
- Writes preserve the file. Only the values of the keys the menu owns are replaced; comments, blank
  lines, section headers, key order, key spelling and unknown keys survive. A write is staged in
  `<name>.ini.tmp` and renamed over the original, so a plugin reading the file mid-write cannot see
  a partial one. A missing file is created with a short header rather than failing.
- Failures are visible, not fatal: a read or write that fails puts one red line under its section
  and one line in `DesertOverlay.log`, and nothing retries in a loop.
- hudhook's own `tracing` output is forwarded into `DesertOverlay.log`, so a graphics hook that
  fails to install says why: WARN and ERROR always, and every level including DEBUG and TRACE when
  `Debug=1`, capped at 20000 forwarded lines per session so a long game cannot fill the disk.
- hudhook is built from `vendor/hudhook`, a patched copy of 0.9.2. The released crate holds a
  reference to the game's swapchain across `ResizeBuffers` and across the game creating a
  replacement swapchain, which made Crimson Desert fail to launch: the resize failed with
  E_INVALIDARG and the retried `CreateSwapChainForHwnd` with E_ACCESSDENIED, because DXGI will not
  give a window a second swapchain while one is still alive. The patched copy releases everything
  before those calls and rebuilds on the next frame. See `vendor/hudhook/DESERT-CHANGES.md`.

#### Fixed

- The menu is no longer blown out and oversaturated on an HDR display. The game presents an HDR10
  (PQ) swapchain, and hudhook wrote imgui's sRGB colours into it unconverted; the vendored hudhook
  now tracks the swapchain's colour space (`IDXGISwapChain3::SetColorSpace1`, DXGI's default for
  the back buffer format when the game never calls it) and converts in its pixel shader. Two new
  ini keys steer it: `HdrBrightness` (paper white in nits, default `203`, `80` to `1000`) and
  `ColorSpace` (`auto`, `sdr`, `hdr10`, `scrgb`, default `auto`). SDR is untouched: the shader's
  passthrough mode is byte for byte what it did before.
- The window drew enlarged and clipped to part of itself whenever Windows display scaling was
  not 100%: the vendored hudhook set imgui's framebuffer scale to the DPI factor while its DX12
  renderer only scaled the viewport, not the scissor rectangles. The framebuffer scale is now 1
  and DPI is applied as font and style scaling instead.
- Two mouse cursors while the menu was open in the game's own screens: imgui now draws its own
  cursor only while Windows is not showing a hardware one.
