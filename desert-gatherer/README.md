# Desert Gatherer

**Version 0.2.0**, for Crimson Desert Enhanced, Steam build **25116796**.
[Changelog](CHANGELOG.md) · [versioning](../VERSIONING.md).

**Maturity:** verified in game on build 25116796. **It has not yet come through a game
update.** The yield multipliers are found by content and should survive one with no more
than a patch. The `Bugs` and `Fish` multipliers are the exception: they are a patch to the
game's code at a scanned site, the only one in the project, so they are the first thing an
update is likely to disable. If that happens they fail alone, with a `[catch]` line in the
log saying so, and every other multiplier keeps working.

A gathering yield multiplier, written in Rust and shipped as one `.asi` file.
It replaces the DMM module pack of the same name: instead of patching the
game's data on disk, it multiplies the yield numbers in memory as the game
loads them, so no game file is ever modified.

## What it does

Every gathering node in the game carries a minimum and a maximum quantity for
each item it can produce. Desert Gatherer multiplies both, for the 275 records
the game counts as genuine gather nodes, split into four independent families:

| family | what it covers | records |
|---|---|---|
| Foraging | plants, fruit, berries, mushrooms, crops | 82 |
| Logging | firewood cut from felled trees (`firewood_*`) | 141 |
| Mining | the `collect_mine` family: rocks, veins, cave variants, breakable stalactites | 36 |
| Ore Nodes | the `collect_ore` family: `ore_*` deposits, sulfur stone, collectible stalactites | 16 |

Each family has its own multiplier, 1 to 100. Mining and Ore Nodes look alike
in game but are separate families in the game's data; set both if you want all
mining-style gathering raised together.

Two more multipliers, **Bugs** and **Fish**, cover the creatures you catch by
hand - insects on the ground and fish at the water's edge. Those are not
gathering nodes and there is no record anywhere saying what one is worth: the
game grants exactly one, from a constant in its own code. So that pair works
differently from the four families - it is a small patch on that constant,
applied as the creature is caught - and it is the one part of this plugin a
game update can switch off on its own. If that happens the log says so once at
startup (`[catch] signature not found; catch multipliers off`) and everything
else here goes on working.

Because that patch is in the game's own grant code rather than in anything
Desert Looter sends, it multiplies the creatures **you** catch by hand exactly
as it multiplies the ones Desert Looter's auto mode catches for you.

One creature pays out twice: a Firefly Colony gives you colonies *and*
fireflies, and because the game rolls a handful of fireflies for each colony
you take, the fireflies come out at roughly double your `Bugs` setting and a
different number every time - at `Bugs=10`, ten colonies and somewhere around
twenty fireflies. That is the game's own drop rule, not a bug in the patch.

It does **not** touch enemy loot, chests, rod-and-line fishing, skinning,
quests, Abyss objects, artifacts, gates or fast travel.

## What to expect in game

The multiplier scales the numbers in the record, not the amount you happen to
get on one pick. Three things surprise people:

- **A node picks one of its ranges, then rolls it.** Most records carry
  several resource-output blocks, and a single gather pays out one of them,
  chosen at random, rolled between its own minimum and maximum. So the same
  bush can give you a big handful one time and a single item the next, in
  vanilla and with the plugin alike.
- **Everything shifts up together.** A peony carries four blocks, 4-7, 2-3,
  1-1 and 1-1. At `Foraging=2` those become 8-14, 4-6, 2-2 and 2-2:

  | pick | vanilla | at 2x |
  |---|---|---|
  | the big block | 4-7 | 8-14 |
  | the middle block | 2-3 | 4-6 |
  | either small block | 1 | 2 |

  Measured in game: a peony gave 2 in vanilla, and 2, 2 and 12 across three
  picks at 2x. Nothing is wrong there. The 2s are the small blocks doubled
  from 1, the 12 is the big block. Watch a dozen picks, not one.
- **Single-item nodes are the clean case.** Firewood and ore deposits carry
  one block of 1-1, and rocks carry three blocks that are each 1-1, so a pick
  goes from exactly 1 to exactly 2 at 2x, every time. If you want to check the
  plugin is working at all, check one of those.

One more thing that looks odd: pressing E can hand the total over as several
separate "x1" pickups a second or so apart. That is the game's own delivery
animation. The total across them is the multiplied amount.

## Install

The release archive holds six files at its root - `DesertGatherer.asi`,
`DesertGatherer.ini`, `README.md`, `CHANGELOG.md`, and `DesertOverlay.asi` with its
`DesertOverlay.ini` - so it suits either way of installing it.

### By hand

Requirements: the game, and Ultimate ASI Loader present in `bin64` as
`winmm.dll` (it loads every `.asi` file beside it).

Extract the archive straight into `<game>\bin64\`, next to `winmm.dll`, or copy
two files there:

- `DesertGatherer.asi`: the plugin. Rename the built `desert_gatherer.dll` to this.
- `DesertGatherer.ini`: the settings. Optional - if it is missing, the plugin
  writes one itself with every key at its default (everything at 1x, i.e.
  vanilla) on the next launch, so you always end up with a file to edit.

To remove the plugin, delete `DesertGatherer.asi`. Nothing is installed
anywhere else.

### The in-game menu

The zip also carries **Desert Overlay**, a second plugin that draws a settings
menu inside the game (DirectX 12, Dear ImGui) and edits `DesertGatherer.ini`
live while you play. Press **Insert** to open and close it. It is entirely
optional: editing the ini in a text editor works exactly as before without it,
and deleting `DesertOverlay.asi` removes it.

You only need **one** copy of it in `bin64`. Both mod zips ship the same file,
so if you install both, one `DesertOverlay.asi` covers them; where the two zips
were built at different times, keep the newer one. It shows a section per
plugin and greys out the section of anything that is not loaded, so it is fine
to run with only this mod installed. Desert Overlay has its own release zip and
its own version number, and a mod zip carries whichever version was current
when the mod was released.

To draw that section, Desert Gatherer writes `DesertGatherer.overlay.ini` beside its own ini at
every start - a small description of what the ini contains, for Desert Overlay to read. Editing
that file does nothing; the plugin regenerates it on the next launch, and `DesertGatherer.ini` is
still the only file that changes what the plugin does. It is safe to ignore if you never install
Desert Overlay, and uninstalling this plugin means deleting it along with `DesertGatherer.asi`.

### Install with DMM

Definitive Mod Manager accepts an ASI mod as either a single `.asi` file or a
directory, so drop the release zip onto DMM's window, or extract it and put the
folder under `<DMM>\mods\_asi\`. DMM copies `DesertGatherer.asi` and the `.ini`
beside it into the game's `bin64` itself, and it installs Ultimate ASI Loader as
`winmm.dll` for you, so there is nothing to set up beforehand. Enable the mod in
DMM's ASI tab; the same tab turns it off again. Read the conflicts section
below first - DMM's own gathering multiplier and the "The Desert Gatherer"
module pack change the same numbers, and must stay unmounted.

### About DMM's security scan

DMM grades every plugin it installs, and it will grade this one **Suspicious**.
That is an honest reading of what the plugin does, not a finding against it: it
imports `WriteProcessMemory`, `ReadProcessMemory` and `VirtualProtect`, because
that is how it installs its hook into the running game and how it reads and
writes game memory safely. (Every read of game memory goes through
`ReadProcessMemory` on the plugin's own process, so a page that has been
unmapped returns nothing instead of faulting and crashing the game.) No mod that
changes a running game's behaviour can avoid those imports.

Everything else the scan looks at is clean: the DLL is not packed and is of
ordinary entropy, has no data appended past its last PE section, has no
writable-and-executable sections, and carries a version-info resource (product
name, version, `KiefBC`, MIT). It is **not** digitally signed - a code-signing
certificate costs money - so check the `SHA256SUMS` file from the release
instead. And the source is public: all of it is in this repository.

## Conflicts with DMM (important)

DMM's **"The Desert Gatherer" module pack** and DMM's **built-in gathering
multiplier preset** change the same minimum/maximum quantities, on disk.

If either is mounted while this `.asi` is installed, the two stack: a mounted
DMM 5X plus `Mining=2` here gives 10x. **Unmount both in DMM before using
this plugin.** The `desert-gatherer-dmm/` directory in this repo is that
pack; this plugin is its replacement, not its companion.

## Settings (`DesertGatherer.ini`)

| key | default | meaning |
|---|---|---|
| `Enabled` | 1 | 0 = the record-loader hook still reads every record to keep its own remembered table current, but writes nothing as records load, and the re-apply pass (see below) writes vanilla numbers back into whatever is already parsed. Flipping it back to 1 re-applies the multipliers the same way |
| `DryRun` | 0 | 1 = log every change that would be made and write nothing; for troubleshooting and after game updates |
| `Debug` | 0 | 1 = also log the records that are not gather nodes (capped at 400 lines) |
| `Foraging` | 1 | multiplier for plants and crops, 1..100 |
| `Logging` | 1 | multiplier for firewood, 1..100 |
| `Mining` | 1 | multiplier for `collect_mine`, 1..100 |
| `Ore` | 1 | multiplier for `collect_ore`, 1..100 |
| `Bugs` | 1 | multiplier for insects caught by hand, 1..100. A code patch on the catch count, not a table edit |
| `Fish` | 1 | multiplier for fish caught by hand, 1..100. Same patch as `Bugs` |

`1` means vanilla: that family's records are read and left untouched. A value
outside 1..100, or one that is not a number, is refused with a warning in the
log and the default is kept.

`DesertGatherer.ini` is re-read while the game runs, about once a second, and
the log confirms each change with an `[ini] reloaded:` line. **A changed
multiplier reaches the game on the next gather**, not the next game start: the
plugin walks the records it has already loaded and rewrites them in place
right after that line, and any record the game has not loaded yet still gets
the new numbers the normal way when it loads. The section
[How a changed multiplier becomes live](#how-a-changed-multiplier-becomes-live)
explains the mechanism, the log lines to expect, and what a WARN there means.

`Bugs` and `Fish` need none of that machinery: the multiplier is read at the
moment the creature is caught, so a change there takes effect on the **next
catch** with nothing to re-apply, whether you caught the creature yourself or
Desert Looter caught it for you. Each multiplied catch logs one line,
`[catch] fish class=23 -> x3`; a creature whose class the plugin has never
seen caught is left vanilla and reported once per class per session
(`[catch] class=2C not a known bug/fish class; vanilla`). `Enabled=0` and
`DryRun=1` both mean "grant one, as the game would".

If `DesertGatherer.ini` is missing, the plugin writes one itself on the next
launch, with every key at its default, instead of leaving nothing to edit. An
existing file is never read, rewritten or replaced, so this can never touch
your settings. The generated file is bare, unlike the shipped template's
comments explaining each key, but every key in the table above is in it -
including `Debug`, which the menu groups under `Diagnostics:`.

## How a changed multiplier becomes live

Short version: the game still reads its whole gather table once, about nine
seconds after launch, and the load-time hook still only runs inside that one
read - but the plugin now also rewrites the records the game already parsed,
right after the ini changes, so a change reaches the game on the next gather
rather than the next launch. Everything below is what that involves and what
it looks like in the log.

**What the game does at launch.** The gathering rules live in a data table
called `gimmickinfo`: 13,906 records, 275 of which are gather nodes. Roughly
nine seconds after the process starts, before the main menu is up, the game
runs a preload pass that reads every record in order, parses each one into an
object in memory, stores the object's pointer in a slot table, and then closes
the file. From then on, whenever any part of the game needs a record, it looks
the pointer up in that slot table; the loader is only called again for a slot
that is still empty, and after the preload pass none of them are. Loading a
save, dying, fast travelling, running out of the area or backing out to the
main menu do not empty the slots; the parsed records live as long as the
process does. That is why editing the raw bytes as they load is not enough on
its own: past that first minute, there is nothing left for the load-time hook
to intercept.

**What the plugin remembers.** Every time the hook sees a gather record - even
at `Enabled=0` and at `1x`, because vanilla is the only fixed point a later
change can be computed from - it reads the vanilla minimum, maximum and item
id of each of the record's output blocks straight out of the raw bytes, along
with the record's index, and keeps them in a small fixed table. Nothing here
touches a parsed object; it is all taken from the same bytes the load-time
hook was about to edit anyway.

**What happens when the ini changes.** `DesertGatherer.ini` is still polled
about once a second. Right after the `[ini] reloaded:` line, the plugin thread
walks every record it remembered, finds the already-parsed object for it
through the game's own record manager, checks that the object's key still
matches, that its output list still has the same number of entries, and that
each block's item id still matches what was on disk, then writes vanilla times
the current multiplier into that block's minimum and maximum. A block already
carrying the wanted numbers is left alone. If any of those checks fails for a
record or a block, that record or block is skipped rather than patched, and
the pass still writes everything it safely can.

The change shows on the very next gather - no restart, no save reload needed.
A record the game has not loaded yet still gets the new numbers the ordinary
way, through the load-time hook, whenever it does load.

**What the log shows.** Right after each `[ini] reloaded: ...` line:

```
[live] re-applied Foraging=10 Logging=1 Mining=1 Ore=1: 82 records rewritten, 193 unchanged, 0 skipped; 644 scalars written
```

or, with `DryRun=1`:

```
[dry] would re-apply Foraging=10 Logging=1 Mining=1 Ore=1: 82 records rewritten, 193 unchanged, 0 skipped; 644 scalars written
```

At `Enabled=0` the multiplier part instead reads `Foraging=1 Logging=1
Mining=1 Ore=1 (Enabled=0)`: `Enabled=0` now means vanilla, not "leave
whatever is already there," so the pass writes every record's minimum and
maximum back to their disk values. `DryRun=1` logs what it would write, at
whichever multipliers are set, and writes nothing. With `Debug=1`, each
rewritten record also gets its own line:

```
[live] mine_bluestone key=17030001 Mining x5 blocks=2 wrote 4: 1->5/3->15, 2->10/8->40
```

Nothing is logged at all if the ini changes before the table has finished its
first load; the load path applies the current multipliers as it goes, so there
is nothing left for the re-apply pass to do yet.

**What a WARN means.** If any record or block failed one of the checks above,
the summary line is followed by one naming the reasons, for example:

```
[live] WARN 12 of 275 records and 3 blocks skipped (not loaded 10, key mismatch 2, null block 3); the parsed record layout may have moved in this game build
```

or, if the record manager itself could not be read at all:

```
[live] WARN the record manager is unreadable; nothing was re-applied
```

Either is the first thing a game update would trip, ahead of the load-time
patching breaking, because the re-apply pass walks live game pointers that the
load path never has to touch.

**What is different about this path.** Everywhere else in this plugin, the
hook only ever writes into bytes the game has not parsed yet - a stream buffer
about to be handed to the deserialiser. This is the first place the plugin
writes into an object the game has already built and is actively using,
reached by walking the record manager rather than being handed a pointer.
That write is guarded the same way as everything else here - a key check, an
item-id check, a list-count check, and a `WriteProcessMemory` that never
assumes an address is still valid - but it is worth being clear about what it
writes: always the *vanilla* number times the current multiplier, taken from
what was remembered at load time, never the value already sitting in the
block scaled again. A record re-applied five times at five different
multipliers ends up exactly where a single load at the last multiplier would
have left it.

## Files in the game folder

| file | what it is | safe to delete? |
|---|---|---|
| `DesertGatherer.asi` | the plugin | yes, that uninstalls it |
| `DesertGatherer.ini` | your settings, read at game start and re-read while it runs | yes, the plugin recreates a bare one at every key's default on the next launch - vanilla `1x` for all four families, so deleting it turns your multipliers off |
| `DesertGatherer.log` | append-only log of what the plugin did; grows every session | yes, any time |
| `DesertGatherer.overlay.ini` | written at every start for Desert Overlay's menu; describes `DesertGatherer.ini`, is not itself read for settings | yes, it is regenerated on the next launch |

A healthy log starts with the version line, one `[ini]` line echoing every
setting, a `[module]` line, a `[gimmick] record loader at +0x...` line and a
`[hook] record loader ... -> stub ...` line. After that comes one line per
gather record as the game loads it, `[gimmick]` when the record was patched,
or `[dry]` when the optional `DryRun=1` is set and nothing was written:

```
[gimmick] ore_copper_01 key=17080005 Ore x2 blocks=1 applied 2/2: 1->2/1->2
[dry] firewood_0001 key=1002971 Logging x2 blocks=1 would write 2: 1->2/1->2
```

and a `[stat]` summary line whenever the counters move. If the loader cannot
be found, or its first bytes are not what is expected, the log says so and the
plugin does nothing at all.

`DesertGatherer.log` records what was **written** to the records, not what you
received. To measure actual yields, install Desert Looter, set `LogReceived=1`
in `DesertLooter.ini`, and read the `[recv] item <key> x<count>` lines in
`DesertLooter.log` as you gather. That is every item the game hands you, and
it is how the numbers above were measured.

## How it survives game updates

Nothing here is a hard-coded address. The loader is found by content: the
plugin scans the running image for the accessor that names the `gimmickinfo`
table and follows it to the record-loading function, then refuses to patch
unless the twelve bytes it is about to overwrite are exactly the prologue it
expects. Inside each record, the yield numbers are found by walking the
resource-output lists and matching the fixed 68-byte block layout that carries
them, not by remembering where they were last time (the same content-based
approach that `desert-gatherer-dmm/rebase.py` uses to move the DMM pack
between builds).

That covers records and functions moving, which is what usually happens on a
patch. If the game changes the loader's shape or the block layout itself, the
plugin will fail to resolve, log why, and leave vanilla yields. It never
guesses: a refusal is a plugin that does nothing, and a wrong patch would be a
crash.

`Bugs` and `Fish` are the fragile part, and deliberately so. There is no
record to edit for a caught creature, so that multiplier is a patch on the
game's code: the plugin scans for the 21-byte instruction sequence that hands
the catch handler its count, and refuses to patch unless the thirteen bytes it
is about to overwrite are exactly the two instructions it expects. A game
update that rewrites that function disables **only** those two keys, with a
`[catch]` line saying which check failed - the four family multipliers are
untouched by it, and the game is untouched by us.

## Building (developers)

Built on NixOS under WSL and cross-compiled to Windows. Desert Gatherer is one
crate of a Cargo workspace (see the repo root `README.md`); run cargo from the
workspace root, not from this directory.

```bash
nix develop
cargo build --release -p desert-gatherer
```

Output: `target/x86_64-pc-windows-gnu/release/desert_gatherer.dll`, installed
as `DesertGatherer.asi`.

Native tests for the platform-independent modules (the ini parser here, the
record parsing and block arithmetic in `desert-core`):

```bash
cargo test --target x86_64-unknown-linux-gnu -p desert-gatherer
```

### How it works, briefly

`desert_core::gimmick::resolve_record_loader` finds the game's gimmickinfo
record loader in the running image. The plugin verifies its prologue and
installs one inline trampoline hook on it (`desert-core`'s, the same one
Desert Looter uses), which is entered on whichever game thread loads a record,
before the deserializer has read a byte of it. From the manager and the stream
the callback works out exactly which bytes belong to the record being loaded,
copies them out with `ReadProcessMemory`, classifies the record by key against
`desert_core::collect`, asks `desert_core::gimmick::multiply` for the list of
scalar edits, and writes them back into the stream buffer with
`WriteProcessMemory`. The buffer is a heap allocation the game frees a couple
of seconds after the last load and reallocates on demand, so the patch has to
happen per record inside the hook and can never be done once.

Every gather record's vanilla minimum, maximum and item id per output block,
plus its index, are also kept in a small lock-free table (`remember`), written
by the hook and read by the plugin's own thread. When the ini changes,
`hook::reapply` walks that table through the game's own record manager, finds
each record's already-parsed object, and writes `vanilla * multiplier`
straight into it with `safe::write`, after checking the object's key and each
block's item id against what was remembered. That is what makes a changed
multiplier reach the game on the next gather instead of the next launch.
