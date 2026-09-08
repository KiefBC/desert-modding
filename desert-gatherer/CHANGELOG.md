# Changelog

All notable changes to Desert Gatherer. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

## [Unreleased]

Game build 25116796. The live re-apply was verified in game on 2026-09-08: changing `Foraging`
from the overlay while playing logged `[live] re-applied ... 82 records rewritten, 193 unchanged,
0 skipped; 644 scalars written` within 150 ms of the ini change, the next gather paid out at the
new rate, and setting it back to 1 restored vanilla the same way.

### Added

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

### Fixed

- The README, the shipped ini's header and the menu notice originally said a changed multiplier
  shows on the next gather because the game reloads its table a few seconds after use. That was
  never true: the game reads all 13,906 `gimmickinfo` records in one preload pass about nine
  seconds after launch and keeps the parsed objects for the whole session, and the load-time hook
  only runs inside that read. Earlier in this cycle the docs were corrected to say "next game
  start" instead, which was accurate for what the plugin did at the time. The re-apply mechanism
  under Added above makes "next gather" true again, this time for real, so the docs now say that
  throughout.

### Changed

- **The shipped `DesertGatherer.ini` now sets all four multipliers to `1` (vanilla) instead of
  `2`.** The plugin's built-in default has always been `1`, and the README's settings table has
  always documented `1`, so the template was the odd one out: installing the mod quietly doubled
  every yield before you had chosen anything. Raise the families you want, in the ini or from
  Desert Overlay's menu. **If you extract a new release zip over an existing install it replaces
  your `DesertGatherer.ini`**, so back it up first or re-enter your multipliers afterwards - this
  is true of any release, but it is the one that will change your yields.

### Added

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
