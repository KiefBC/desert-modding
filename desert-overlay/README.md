# Desert Overlay

**Version 0.1.0**, for Crimson Desert Enhanced, Steam build **25116796**.
[Changelog](CHANGELOG.md) · [versioning](../VERSIONING.md).

An in-game settings menu for Desert Looter and Desert Gatherer. Press **Insert**
and a window appears over the game with the two mods' settings in it as
checkboxes and sliders. Change one and it takes effect within about a second,
without leaving the game and without a restart.

## What it actually does

It edits ini files in `bin64`, in place, while the game runs, and it has no list
of which ones. Desert Looter and Desert Gatherer each write a small **schema**
file beside their own ini at every start (`DesertLooter.overlay.ini`,
`DesertGatherer.overlay.ini`) saying what their ini contains and how the menu
should draw it; the overlay scans `bin64` for these once a second and builds one
collapsible section per file it finds. The overlay does not talk to the two
plugins at all; they re-read their own ini once a second and pick the change up.

Three things follow from that, and they are the reason it works this way:

- **Every change is permanent.** It is in the file, so it is still there next
  time you launch. There is no separate "apply" or "save".
- **Editing the files by hand still works**, even while the game is running.
  The menu notices within a second and shows the new value.
- **Your files are not rewritten, only edited.** The overlay replaces the value
  on the lines whose key it owns and touches nothing else: your comments, your
  ordering, and the keys it does not show (`BagTab`, `LogReceived` and `Debug`)
  come through exactly as they were. Writes go through a temporary file and a
  rename, so a plugin reading the file at the wrong moment never sees half of it.

The overlay reads and writes files. It does not read or write the game's memory,
has no byte signatures and no hard-coded addresses, and cannot affect the game's
behaviour except through the ini files a schema names.

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

To uninstall, delete the two files. Nothing else is touched: the `*.overlay.ini` schema files the
menu draws its sections from belong to Desert Looter and Desert Gatherer, not to this plugin, and
removing this `.asi` does not remove them - they just stop being read by anything until this plugin
comes back.

## Using the menu

Each section comes from the schema its plugin wrote, in the order the schema gives (Desert Looter
before Desert Gatherer today, by each plugin's own `Order`). What is currently in them:

| section | what is in it |
|---|---|
| Desert Looter | the master switch, auto gather, the four gather families, ground items / dropped gear / unarmed nodes, scan and gather range, gather interval, node cooldown, stack limit, and the four hotkeys (`KeyToggle`, `KeyScan`, `KeyGather`, `KeyRecord`) as key pickers. Presets above the fields: `Everything`, `Plants only`, `Wood only`, `Rock and ore only` - one click sets the four gather families and ground items and leaves everything else alone. |
| Desert Gatherer | the master switch, dry run, and the four yield multipliers |

The sliders and the number fields stop at the ranges the plugins accept, so the
menu cannot produce a value its plugin would refuse. A number typed into a field
is written when you press Enter or click away, not while you are still typing.
A mod that ships no schema file simply has no section; see "Adding a mod to the menu" below.

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
| `FontSize` | `20` | height of the menu's text in pixels before `Scale` is applied, from `8` to `72`. The font is rasterised at that size, so a larger value is sharper rather than blockier |
| `Font` | `segoeui.ttf` | the font the menu is drawn in. A bare file name is looked up in `%WINDIR%\Fonts`, an absolute path is used as it stands, and an empty value goes back to Dear ImGui's built-in font. `georgia.ttf`, `constan.ttf` and `cambria.ttc:0` give a more fantasy, serif look; the `:N` suffix picks a face out of a `.ttc` collection. A missing or unreadable file is a warning in the log and the built-in font |
| `HdrBrightness` | `203` | paper white in nits on an HDR display: how bright the menu's white is drawn. `80` to `1000`; ignored on SDR |
| `ColorSpace` | `auto` | what the menu's pixels are encoded for: `auto` (follow the swapchain), `sdr`, `hdr10` or `scrgb`. Anything else is `auto` with a warning in the log |
| `Theme` | `banner` | the menu's colour theme: `classic`, `parchment`, `gilded`, `splash`, `banner` or `steel`. The picker at the top of the menu switches between them live; this key is what makes a choice stick across launches |

The game presents an HDR10 (PQ) signal, so the menu is converted into the
swapchain's colour space before it is drawn; without that its sRGB colours come
out blown out and oversaturated. `auto` reads the colour space off the swapchain
and is right unless the detection is wrong, which is what the three forced
values are for. `HdrBrightness` is the only one worth touching in normal use:
raise it if the menu looks dull next to the game, lower it if it glares.

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

## Adding a mod to the menu

There is no list of mods to edit anywhere in this plugin. A section appears because something
beside the game exe named `<Name>.overlay.ini` describes an ini file; Desert Looter and Desert
Gatherer write theirs at every start, and any other ASI plugin can do the same. The format is
`desert_core::ini`'s own dialect - `Key=Value`, `;` comments, `[Section]` headers - and the full
rules (every key, every `Kind`, what makes a file get skipped) live in `desert-core/src/schema.rs`.
A small example, a header and two fields and a preset:

```ini
; MyMod.overlay.ini - written by My Mod at every start.
[overlay]
Schema=1
Title=My Mod
Ini=MyMod.ini
Module=MyMod.asi

[Enabled]
Kind=bool
Label=Enabled
Default=1
Help=Master switch.

[Speed]
Kind=int
Label=Speed
Default=10
Min=1
Max=100

[preset:Fast]
Set=Enabled=1;Speed=50
```

Drop that beside `MyMod.ini` and a "My Mod" section appears in the menu within a second, greyed out
as "not installed" if `MyMod.asi` is not loaded. Nothing in Desert Overlay needs to change or be
rebuilt.

## Log

`bin64/DesertOverlay.log`, written beside the game exe. The first line names the
version, then the settings that were read, then whether the graphics hook went
in. If the menu never appears, that file says why - and if hudhook itself
refused, its reason is in there too. `Debug=1` adds a line per ini write and
turns on hudhook's own debug and trace output, which is what to send with a
report that the game will not start; it stops after 20000 hudhook lines so the
file cannot grow all session. Resizing the menu logs one `[menu] window size`
line once the drag has settled, in pixels and in unscaled units, which is how a
size that looks right in game becomes the default.

Schema discovery adds its own lines: `[schema] <file>: <title>, N fields, M presets` when a section
loads, `[schema] WARN <file>: <why>` when a schema file will not parse (once per modified time, not
once a second), `[schema] <file> is gone; <title> left the menu` when a schema file disappears, and
`[schema] no *.overlay.ini beside the game exe; the menu has nothing to show` when nothing is found
at all. `[menu] <module> is loaded` / `[menu] <module> is not loaded` mark a section's plugin
appearing or vanishing from the process.

## Credits and licence

MIT, like everything in this repository. The menu is drawn with
[hudhook](https://github.com/veeenu/hudhook) (MIT) and
[Dear ImGui](https://github.com/ocornut/imgui) through
[imgui-rs](https://github.com/imgui-rs/imgui-rs) (both MIT).
