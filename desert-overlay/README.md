# Desert Overlay

**Version 0.1.0**, for Crimson Desert Enhanced, Steam build **25116796**.
[Changelog](CHANGELOG.md) · [versioning](../VERSIONING.md).

An in-game settings menu for Desert Looter and Desert Gatherer. Press **Insert**
and a window appears over the game with the two mods' settings in it as
checkboxes and sliders. Change one and it takes effect within about a second,
without leaving the game and without a restart.

## What it actually does

It edits `DesertLooter.ini` and `DesertGatherer.ini` in `bin64`, in place, while
the game runs. That is the whole mechanism. The overlay does not talk to the two
plugins at all; they re-read their own ini once a second and pick the change up.

Three things follow from that, and they are the reason it works this way:

- **Every change is permanent.** It is in the file, so it is still there next
  time you launch. There is no separate "apply" or "save".
- **Editing the files by hand still works**, even while the game is running.
  The menu notices within a second and shows the new value.
- **Your files are not rewritten, only edited.** The overlay replaces the value
  on the lines whose key it owns and touches nothing else: your comments, your
  ordering, and the keys it does not show (`BagTab`, `LogReceived`, `Debug`, the
  `KeyToggle`/`KeyScan`/`KeyGather`/`KeyRecord` bindings) come through exactly as
  they were. Writes go through a temporary file and a rename, so a plugin reading
  the file at the wrong moment never sees half of it.

The overlay reads and writes files. It does not read or write the game's memory,
has no byte signatures and no hard-coded addresses, and cannot affect the game's
behaviour except through those two ini files.

A section whose plugin is not loaded in the game is greyed out and marked "not installed", so the
menu can be shipped with either mod on its own.

## Requirements

- **The same ASI loader the other two mods use** ([Ultimate ASI
  Loader](https://github.com/ThirteenAG/Ultimate-ASI-Loader) as `winmm.dll` in
  `bin64`). This is a plain `.asi`, loaded the same way; if Desert Looter or
  Desert Gatherer works, this will load too.
- **DirectX 12.** The menu is drawn through the game's DX12 swapchain. Crimson
  Desert is a DX12 game, so this is only a caveat for anything that forces
  another API.
- Either or both of Desert Looter and Desert Gatherer, obviously. A file whose
  plugin is not installed is still edited; nothing reads it, and nothing breaks.

## Install

1. Extract `DesertOverlay.asi` and `DesertOverlay.ini` into the game's `bin64`
   folder, next to `winmm.dll` and the other `.asi` files.
2. Launch the game and press **Insert**.

To uninstall, delete the two files. Nothing else is touched.

## Using the menu

| section | what is in it |
|---|---|
| Presets | `Everything`, `Plants only`, `Wood only`, `Rock and ore only` - one click sets Desert Looter's four gather families and turns ground items on. Nothing else is changed, so tuned ranges and timings survive a preset. |
| Desert Looter | the master switch, auto gather, the four gather families, ground items / dropped gear / unarmed nodes, scan and gather range, gather interval, node cooldown and stack limit |
| Desert Gatherer | the master switch, dry run, and the four yield multipliers |

The sliders and the number fields stop at the ranges the plugins accept, so the
menu cannot produce a value its plugin would refuse. A number typed into a field
is written when you press Enter or click away, not while you are still typing.

If a file cannot be read or written, the reason appears as a red line under that
section and in `DesertOverlay.log`. Nothing is lost and the game is unaffected.

## Settings

`DesertOverlay.ini`, read once at game start (a change to it needs a restart -
it is only the *other* two files that are live):

| key | default | meaning |
|---|---|---|
| `Enabled` | `1` | `0` = load, log one line and install no graphics hook at all |
| `Debug` | `0` | `1` = log every ini write and reload, plus everything hudhook says (its per-frame debug and trace lines included, capped at 20000 lines per session) |
| `KeyMenu` | `Insert` | the key that shows and hides the menu |
| `ShowOnStart` | `0` | `1` = the menu is already open at the first frame |
| `Scale` | `0` | menu size: `0` follows the Windows display scaling, otherwise a fixed factor from `0.5` to `4` |

Key names: `F1`..`F24`, `A`..`Z`, `0`..`9`, `NUM0`..`NUM9`, `NUMPLUS`
`NUMMINUS` `NUMMULT` `NUMDIV` `NUMDOT`, `HOME` `END` `INSERT` `DELETE` `PAGEUP`
`PAGEDOWN` `TAB` `SPACE` `BACKSPACE` `SCROLLLOCK` `PAUSE`, `MOUSE3` `MOUSE4`
`MOUSE5`. Avoid keys the game uses and the ones Desert Looter already has
(`F7`, `F9`, `F10`, `F11` by default).

## While the menu is open

Keyboard and mouse input is held back from the game, so typing a number does not
also swing your sword and dragging a slider does not turn the camera. Window
messages are not blocked: alt-tab still works. The game hides the hardware
cursor, so the overlay draws its own and releases the cursor clip when the menu
opens; the game takes the cursor back the next time it wants it.

## Log

`bin64/DesertOverlay.log`, written beside the game exe. The first line names the
version, then the settings that were read, then whether the graphics hook went
in. If the menu never appears, that file says why - and if hudhook itself
refused, its reason is in there too. `Debug=1` adds a line per ini write and
turns on hudhook's own debug and trace output, which is what to send with a
report that the game will not start; it stops after 20000 hudhook lines so the
file cannot grow all session.

## Credits and licence

MIT, like everything in this repository. The menu is drawn with
[hudhook](https://github.com/veeenu/hudhook) (MIT) and
[Dear ImGui](https://github.com/ocornut/imgui) through
[imgui-rs](https://github.com/imgui-rs/imgui-rs) (both MIT).
