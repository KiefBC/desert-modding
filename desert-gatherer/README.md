# Desert Gatherer

**Version 0.1.1**, for Crimson Desert Enhanced, Steam build **25116796**.
[Changelog](CHANGELOG.md) · [versioning](../VERSIONING.md).

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

It does **not** touch enemy loot, chests, fishing, skinning, quests, Abyss
objects, artifacts, gates or fast travel.

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
| `Enabled` | 1 | 0 = the hook is installed but a pass-through: it reads and writes nothing. Like the multipliers, a change only counts for records the game reads afterwards, which in practice means the next launch (see below) |
| `DryRun` | 0 | 1 = log every change that would be made and write nothing; for troubleshooting and after game updates |
| `Debug` | 0 | 1 = also log the records that are not gather nodes (capped at 400 lines) |
| `Foraging` | 1 | multiplier for plants and crops, 1..100 |
| `Logging` | 1 | multiplier for firewood, 1..100 |
| `Mining` | 1 | multiplier for `collect_mine`, 1..100 |
| `Ore` | 1 | multiplier for `collect_ore`, 1..100 |

`1` means vanilla: that family's records are read and left untouched. A value
outside 1..100, or one that is not a number, is refused with a warning in the
log and the default is kept.

`DesertGatherer.ini` is re-read while the game runs, about once a second, and
the log confirms each change with an `[ini] reloaded:` line. But a changed
multiplier only reaches records the game reads *after* the change, and the
game reads its whole gather table once, at launch. **A new multiplier takes
effect on the next game start**, not on the next gather; reloading a save or
going back to the main menu does not help. The section
[Why a changed multiplier needs a restart](#why-a-changed-multiplier-needs-a-restart)
explains what the game does and why the plugin cannot do better yet.

If `DesertGatherer.ini` is missing, the plugin writes one itself on the next
launch, with every key at its default, instead of leaving nothing to edit. An
existing file is never read, rewritten or replaced, so this can never touch
your settings. The generated file is bare, unlike the shipped template's
comments explaining each key, but every key in the table above is in it -
including `Debug`, which the menu groups under `Diagnostics:`.

## Why a changed multiplier needs a restart

Short version: the plugin edits each gather record at the one moment the game
reads it from its data file, and the game does that once per session, about
nine seconds after launch. Everything below is what was found in the game
(Steam build 25116796) while chasing a report that a change from 1x to 40x in
the in-game menu did nothing.

**What the game does at launch.** The gathering rules live in a data table
called `gimmickinfo`: 13,906 records, 275 of which are gather nodes. Roughly
nine seconds after the process starts, before the main menu is up, the game
runs a preload pass that reads every record in order, parses each one into an
object in memory, stores the object's pointer in a slot table, and then closes
the file. From then on, whenever any part of the game needs a record, it looks
the pointer up in that slot table. The loader is only called again for a slot
that is still empty, and after the preload pass none of them are. Loading a
save, dying, fast travelling, running out of the area or backing out to the
main menu do not empty the slots; the parsed records live as long as the
process does.

**What the plugin does.** Desert Gatherer hooks the loader. Each time the game
is about to parse a record, the hook looks at the raw bytes first, finds the
yield ranges of a gather record, multiplies their minimum and maximum, and
writes the multiplied bytes back, so the object the game builds carries the
new numbers. That is the whole mechanism, and it is why the plugin is so
small and safe: it never calls a game function and never touches a parsed
object. It is also why a later change does nothing. The hook only runs when
the loader runs, and the loader has already run for every record.

**What the log shows.** A session looks like this:

```
[    8.847] [gimmick] first call: mgr=... count=13906 idx=0 ...
[   60.161] [stat] records seen 13906, gather records patched 0, scalars written 0
[  234.232] [ini] reloaded: Enabled=1 DryRun=0 Debug=0 Foraging=5 Logging=1 Mining=1 Ore=1
[  235.233] [ini] reloaded: Enabled=1 DryRun=0 Debug=0 Foraging=10 Logging=1 Mining=1 Ore=1
```

All 13,906 records went through the hook in the first minute while every
family was still at 1x, so nothing was patched. The two `reloaded` lines show
the menu's edits arriving within a second, which is the plugin working as
designed. The `[stat]` line is printed whenever the counters change, and it
never appears again: no record was loaded after the first pass, including
across a save reload. Had the game re-read its table, a second `[stat]` line
with a non-zero patched count would follow within a minute.

**What this means for you.**

- Set the multipliers you want *before* launching: in `DesertGatherer.ini`
  with a text editor, or from Desert Overlay's menu during the previous
  session. The values in the file when the game starts are the values for
  that session.
- The same goes for `Enabled`. Flipping it from 0 to 1 while playing is
  accepted and logged, but there are no records left for it to apply to.
- A change you make mid-session is not lost. It is in the ini, and it takes
  effect on the next launch.
- Earlier versions of this README and of the ini's header said the game
  reloads its table a few seconds after use and that a change shows on the
  next gather. That was wrong; it was never measured, and the log above is
  the correction.

**Why it is not fixed yet.** Two ways to make a change live are known, and
neither is in this version. The plugin could empty the 275 gather slots when
the ini changes, so the game's lazy loader re-reads those records through the
hook on next use; that leaks the old objects and relies on nothing in the game
holding a stale pointer. Or it could rewrite the yields inside the already
parsed objects; the layout of those objects has since been worked out (the
resource list sits at a fixed offset in the record, each block holds its
minimum and maximum as plain 64-bit numbers), which makes this the clean
route. It means the plugin writing into game objects rather than into bytes
the game has not parsed yet, so it is being built and tested separately rather
than slipped into a patch release.

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
