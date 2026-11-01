# Menu and hotkeys

Source: `desert-looter/src/lib.rs` (main loop), `desert-looter/src/config.rs` and `desert-overlay/src/config.rs` `schema()` (key defaults), `desert-looter/src/recorder.rs` (F7 caps), `desert-overlay/src/ui.rs` (tabs), `desert-tooling/README.md` (The menu, Keys).

## The menu

Press **Insert** to show or hide the menu, drawn with Dear ImGui over the game's DirectX 12 frame. It has a row of tabs:

- **Looter**, **Gatherer**, **Dispatch**: one tab per subsystem section that has settings of its own, in that order. A tab whose subsystem is disabled is drawn dimmed. The Overlay has no tab of its own: all its settings sit on Settings and Debug.
- **Settings**: every hotkey, and how the menu itself looks.
- **Debug**: every logging switch, dump and dry run.

A change takes effect within about a second, with no restart. The menu writes `DesertTooling.ini` and each subsystem re-reads its own section once a second, so every change is saved, and editing the file by hand while the game runs also works. The menu edits only the value on the lines whose key it owns, under the right `[Section]`, and writes through a temporary file and a rename.

The Looter tab opens with four presets, `Everything`, `Plants only`, `Wood only` and `Rock and ore only`. Each sets the Looter's four gather families (`GatherForaging`, `GatherLogging`, `GatherMining`, `GatherOre`) and `GatherItems`, and leaves everything else alone.

While the menu is open, keyboard and mouse input is held back from the game. Alt-tab still works.

## Hotkeys

| key | ini key | action |
|---|---|---|
| Insert | `[Overlay] KeyMenu` | show and hide the menu |
| F9 | `[Looter] KeyGather` | gather the nearest eligible node or item once, whatever the auto state |
| F10 | `[Looter] KeyToggle` | toggle automatic gathering on and off |
| F11 | `[Looter] KeyScan` | write a survey of everything nearby to the log (read-only); gather nodes and items first, up to `SurveyLines` |
| F7 | `[Looter] KeyRecord` | debug: log the events the game queues, up to 25 lines per event type and 300 in all, then stop and print a census; press again to stop early |

Every Looter keypress beeps once. All five keys can be rebound in the ini or on the Settings tab; the accepted names are listed in the manual's "Key names" section.

With `[Looter] Enabled=0` the four Looter keys do nothing: they are still polled, so a press made while disabled is consumed rather than fired later. `AutoGather=1` starts automatic gathering on at launch; F10 flips it.
