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

The release archive holds four files at its root - `DesertGatherer.asi`,
`DesertGatherer.ini`, `README.md` and `CHANGELOG.md` - so it suits either way of
installing it.

### By hand

Requirements: the game, and Ultimate ASI Loader present in `bin64` as
`winmm.dll` (it loads every `.asi` file beside it).

Extract the archive straight into `<game>\bin64\`, next to `winmm.dll`, or copy
two files there:

- `DesertGatherer.asi`: the plugin. Rename the built `desert_gatherer.dll` to this.
- `DesertGatherer.ini`: the settings. Optional; built-in defaults (everything
  at 1x, i.e. vanilla) apply without it.

To remove the plugin, delete `DesertGatherer.asi`. Nothing is installed
anywhere else.

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
this plugin.** The `dmm-pack/` directory in this repo is that pack; this
plugin is its replacement, not its companion.

## Settings (`DesertGatherer.ini`)

| key | default | meaning |
|---|---|---|
| `Enabled` | 1 | 0 = load but hook nothing |
| `DryRun` | 0 | 1 = log every change that would be made and write nothing; for troubleshooting and after game updates |
| `Debug` | 0 | 1 = also log the records that are not gather nodes (capped at 400 lines) |
| `Foraging` | 1 | multiplier for plants and crops, 1..100 |
| `Logging` | 1 | multiplier for firewood, 1..100 |
| `Mining` | 1 | multiplier for `collect_mine`, 1..100 |
| `Ore` | 1 | multiplier for `collect_ore`, 1..100 |

`1` means vanilla: that family's records are read and left untouched. A value
outside 1..100, or one that is not a number, is refused with a warning in the
log and the default is kept.

## Files in the game folder

| file | what it is | safe to delete? |
|---|---|---|
| `DesertGatherer.asi` | the plugin | yes, that uninstalls it |
| `DesertGatherer.ini` | your settings, read once at game start | yes, vanilla defaults are used |
| `DesertGatherer.log` | append-only log of what the plugin did; grows every session | yes, any time |

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
approach that `dmm-pack/rebase.py` uses to move the DMM pack between builds).

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
