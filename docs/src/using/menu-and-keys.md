# Menu and hotkeys

Source: `crates/desert-harvest/src/entry.rs` (main loop), `crates/desert-harvest/src/config.rs` (the pinned menu order, `the_merged_fields_land_on_their_tabs_in_this_order`), `crates/desert-harvest/src/looter/config.rs`, `crates/desert-harvest/src/gatherer/config.rs`, `crates/desert-dispatch/src/config.rs` and `crates/desert-overlay/src/config.rs` `schema()` (key defaults, field order), `crates/desert-harvest/src/looter/recorder.rs` (F7 caps), `crates/desert-overlay/src/ui.rs` (tabs), `crates/desert-tooling/README.md` (The menu, Keys).

## The menu

Press **Insert** to show or hide the menu, drawn with Dear ImGui over the game's DirectX 12 frame. It has a row of tabs:

- **Harvest**, **Dispatch**: one tab per subsystem section that has settings of its own, in that order. A tab whose subsystem is disabled is drawn dimmed. The Overlay has no tab of its own: all its settings sit on Settings and Debug.
- **Settings**: every hotkey, and how the menu itself looks.
- **Debug**: every logging switch, dump and dry run.

A change takes effect within about a second, with no restart. The menu writes `DesertTooling.ini` and each subsystem re-reads its own section once a second, so every change is saved, and editing the file by hand while the game runs also works. The menu edits only the value on the lines whose key it owns, under the right `[Section]`, and writes through a temporary file and a rename.

The Harvest tab opens with four presets, `Everything`, `Plants only`, `Wood only` and `Rock and ore only`. Each sets the four gather families (`GatherForaging`, `GatherLogging`, `GatherMining`, `GatherOre`) and `GatherItems`, and leaves everything else alone. Below them, top to bottom:

- `Enabled` and `AutoGather`.
- Four auto-loot rows: "Gather nodes:" (`GatherForaging`, `GatherLogging`, `GatherMining`, `GatherOre`, `GatherMoney`, `GatherUnarmed`), "Loose items:" (`GatherItems`, `GatherGear`), "Creatures:" (`GatherBugs`, `GatherFish`, `GatherCritters`) and "Bodies:" (`GatherCarcass`, `GatherBodies`, `GatherFriendlyBodies`).
- `Hunting`.
- "Item log:" `LootLogMinTier`, then `LootLogMoney` ("Always show money") and `LootLogQuest` ("Always show quest items"), each on its own row.
- The ranges and pacing: `ScanRange`, `GatherRange`, `GatherInterval`, `NodeCooldown`, `StackLimit`.
- "Yield multipliers:" `Foraging`, `Logging`, `Mining`, `Ore`, `Money`, `Bugs`, `Fish`, `Critters`.
- "Pouches and breakables:" `Pouches` and `Breakables`. These are `[Harvest]` keys since 0.16.0 (they were under `[Dispatch]` in 0.15.0 and an old ini is moved over at launch), but dispatch does the writing, so they need `[Dispatch] Enabled=1`.

The Dispatch tab holds `Enabled` and the four mission levers, `Speed`, `NoSkillRequirement`, `AnyOperatorCount` and `Rewards`.

The Debug tab holds, for Harvest, `Debug` and `LogReceived` on one row, `SurveyLines`, `BagTab`, then `DryRun` and `DumpTable` on one row. Harvest's `DryRun` also covers `Pouches` and `Breakables`; `[Dispatch] DryRun` covers only the mission levers. Dispatch's own switches (`LogRecords`, `DumpRewards`, `MaxLines`, `DryRun`, `Debug`, `DumpRaw`, `DumpBuffs`) follow.

While the menu is open, keyboard and mouse input is held back from the game. Alt-tab still works.

## Hotkeys

| key | ini key | action |
|---|---|---|
| Insert | `[Overlay] KeyMenu` | show and hide the menu |
| F9 | `[Harvest] KeyGather` | gather the nearest eligible node or item once, whatever the auto state |
| F10 | `[Harvest] KeyToggle` | toggle automatic gathering on and off |
| F11 | `[Harvest] KeyScan` | write a survey of everything nearby to the log (read-only); gather nodes and items first, up to `SurveyLines` |
| F7 | `[Harvest] KeyRecord` | debug: log the events the game queues, up to 25 lines per event type and 300 in all, then stop and print a census; press again to stop early |

Every gathering keypress beeps once. All five keys can be rebound in the ini or on the Settings tab; the accepted names are listed in the manual's "Key names" section.

With `[Harvest] Enabled=0` the four harvest keys do nothing: they are still polled, so a press made while disabled is consumed rather than fired later. `AutoGather=1` starts automatic gathering on at launch; F10 flips it.
