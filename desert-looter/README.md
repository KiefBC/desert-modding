# Desert Looter

A gathering auto-loot plugin for Crimson Desert (Steam build 25116796).
It collects plants, ore, stone and wood around you by sending the game the
same pickup event it sends when you press E, so nothing is simulated and no
input is faked. Written in Rust, shipped as one `.asi` file.

## What it does

- **Gathers nodes** within a few metres: flowers, berries, fruit, ore veins
  and rocks, and the firewood you cut from felled logs. Veins and rocks are
  mined outright, without a swing.
- **Picks up plain ground items** such as ore chunks. Before every item it
  asks the game whether taking it would count as stealing, and skips it if
  so. Merchant goods, quest items and props are never touched.
- **Respects the bag.** Nothing is sent when the bag is full unless the item
  would stack onto a stack you already carry. Repeated refusals switch auto
  mode off.
- **Leaves alone**: standing trees, log chunks, creatures, furniture and, by
  default, dropped weapons and armour.

## Keys

| key | action |
|---|---|
| F9 | gather the nearest eligible node or item once |
| F10 | toggle automatic gathering on and off |
| F11 | write a survey of everything nearby to the log (read-only) |
| F7 | debug: record every event the game queues until pressed again |

Every keypress beeps once. Keys can be changed in the ini.

## Install

Requirements: the game, and Ultimate ASI Loader present in `bin64` as
`winmm.dll` (it loads every `.asi` file beside it).

Copy two files into `<game>\bin64\`:

- `DesertLooter.asi`: the plugin. Rename the built `desert_looter.dll` to this.
- `DesertLooter.ini`: the settings. Optional; built-in defaults apply without it.

To remove the plugin, delete `DesertLooter.asi`. Nothing is installed anywhere
else and no game file is modified.

## Files in the game folder

Everything the plugin touches lives in `bin64` next to the exe:

| file | what it is | safe to delete? |
|---|---|---|
| `DesertLooter.asi` | the plugin | yes, that uninstalls it |
| `DesertLooter.ini` | your settings, read once at game start | yes, defaults are used |
| `DesertLooter.log` | append-only log of what the plugin did; grows every session | yes, any time |
| `DesertLooter.yields` | a small cache of "this node gives this item", learned while you play; used for the stacking rule at a full bag | yes, it relearns itself |

The log is the first thing to look at when something seems off. A healthy
start shows eight `[sig]` lines ending in `= +0x...`, two `[hook]` lines, the
`[event]` descriptor lines, and later `[gather]` lines for each pickup.

## Settings (`DesertLooter.ini`)

| key | default | meaning |
|---|---|---|
| `Enabled` | 1 | 0 = load but do nothing |
| `GatherRange` | 6 | how far a node may be, in metres (max 50) |
| `AutoGather` | 0 | 1 = auto mode on from the start (F10 flips it) |
| `GatherInterval` | 500 | milliseconds between automatic pickups |
| `NodeCooldown` | 8000 | milliseconds before a node is tried again |
| `GatherUnarmed` | 1 | also take nodes the game has not "armed" yet (this is what mines whole veins) |
| `GatherItems` | 1 | pick up plain ground items such as ore chunks |
| `GatherGear` | 0 | also pick up dropped weapons and armour |
| `BagTab` | 1 | which inventory tab is the bag for the full check |
| `StackLimit` | 999 | at a full bag, do not grow a stack past this |
| `ScanRange` | 40 | radius of the F11 survey |
| `Debug` | 0 | 1 = very verbose survey (first F11 dumps hundreds of lines) |
| `KeyToggle`, `KeyScan`, `KeyGather`, `KeyRecord` | F10, F11, F9, F7 | key names: F1..F24, A..Z, 0..9, NUM0..NUM9, HOME, END, INSERT, DELETE, PAGEUP, PAGEDOWN, MOUSE3..MOUSE5 and a few more |

## Things worth knowing

- With a yield-multiplier mod installed, a mined vein gives the full
  multiplied amount straight to the bag, and no chunks are spawned.
- A full bag makes the game refuse pickups silently. The log shows it as
  nodes "still there" after the cooldown, and auto mode stops after three.
- If the game updates, the eight signatures may stop matching. The log will
  say `NOT FOUND` and the plugin stays idle; nothing dangerous happens.

## Building (developers)

Built on NixOS under WSL and cross-compiled to Windows. Desert Looter is one
crate of a Cargo workspace (see the repo root `README.md`); run cargo from the
workspace root, not from this directory. The `flake.nix` dev shell provides the
Rust toolchain with the `x86_64-pc-windows-gnu` target, the mingw cross-linker,
and the analysis tools (python, binutils, Ghidra).

```bash
nix develop
cargo build --release -p desert-looter
```

Output: `target/x86_64-pc-windows-gnu/release/desert_looter.dll`. If flakes
are not enabled in your Nix config, add
`--extra-experimental-features 'nix-command flakes'` to the `nix` command.
Keep the checkout on the Linux filesystem; building under `/mnt/c` is slow.

Native tests for the platform-independent modules:

```bash
cargo test --target x86_64-unknown-linux-gnu
```

One ignored test scans the real game exe (needs the Steam install mounted):

```bash
cargo test --release --target x86_64-unknown-linux-gnu -p desert-looter --test game_exe -- --ignored --nocapture
```

### How it works, briefly

The plugin resolves eight byte signatures in the game exe at load, hooks two
functions with inline trampolines (a per-frame job, for a callback on a game
thread, and the event queue's push, for observation), finds the actor
manager through its RTTI vtable, and reads all game memory through
`ReadProcessMemory` so a vanished page can never crash the game. Pickups are
built exactly as the game's own event builder builds them and queued from
inside the hook. The signature scanning, the trampolines, the guarded reads
and the logger all live in the `desert-core` crate, shared with Desert
Gatherer; what is left in `desert-looter/src/` is the loot logic itself. The
reverse-engineering record is in `docs/` and the reference mod's decompile is
regenerated by `tools/ghidra/`.

### Analysis tooling

`tools/sigscan.py` checks the signatures against the exe after a game
update; each must hit exactly once. `tools/dis.sh` disassembles an address
range, `tools/xrefs.py` finds references, and `tools/ghidra/decompile-game.sh`
decompiles functions from the finished Ghidra project.
