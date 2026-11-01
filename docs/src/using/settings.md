# Settings quick reference

Source: `schema()` in `crates/desert-harvest/src/config.rs` (with the field lists in `crates/desert-harvest/src/looter/config.rs` and `crates/desert-harvest/src/gatherer/config.rs`), `crates/desert-overlay/src/config.rs` and `crates/desert-dispatch/src/config.rs`; the `Font` key from `crates/desert-overlay/src/config.rs` and `crates/desert-tooling/DesertTooling.ini`.

Every key lives in `DesertTooling.ini` beside the game exe, under its subsystem's `[Section]` header. `Enabled` and `Debug` exist under all three sections and mean different things in each, and `DryRun` exists under `[Harvest]` and `[Dispatch]`, so a key belongs to the header above it. Each table below lists the key, its default, the range the menu and the parser accept, and the menu tab it is drawn on. Booleans are written `1`/`0`. The long form of every setting is in the manual's [Settings section](../repo/desert-tooling/index.md#settings-deserttoolingini), and the shipped `DesertTooling.ini` carries a comment above every key.

## `[Harvest]`

Auto-loot and the yield multipliers in one section. It used to be two, `[Looter]` and `[Gatherer]`; an ini that still has them is migrated on the first launch (see [Install and upgrade](install.md)). `Enabled` and `Debug` are read by both halves; every other key belongs to one of them. The rows are in the order the menu draws them.

| key | default | range | tab | meaning |
|---|---|---|---|---|
| `Enabled` | 1 | 0/1 | Harvest | master switch; 0 = no gathering, no hotkeys, vanilla yields (anything already multiplied is put back). Every hook stays installed |
| `AutoGather` | 0 | 0/1 | Harvest | automatic gathering on from the start (F10 flips it). 0 with the multipliers set is "multipliers without auto-loot" |
| `GatherForaging` | 1 | 0/1 | Harvest | plants, fruit, berries, mushrooms, crops; never the water well |
| `GatherLogging` | 1 | 0/1 | Harvest | firewood cut from felled trees |
| `GatherMining` | 1 | 0/1 | Harvest | the `collect_mine` family: rocks, veins, stalactites |
| `GatherOre` | 1 | 0/1 | Harvest | the `collect_ore` family: ore deposits and sulfur stone |
| `GatherMoney` | 0 | 0/1 | Harvest | also pick up coin props lying in the world (untested in game); the amount is the `Money` multiplier below |
| `GatherItems` | 1 | 0/1 | Harvest | also pick up basic items on the ground (`item_basic_*`) |
| `GatherGear` | 0 | 0/1 | Harvest | also pick up dropped weapons and armour |
| `GatherUnarmed` | 1 | 0/1 | Harvest | also target nodes the game has not armed with interaction data |
| `GatherBugs` | 1 | 0/1 | Harvest | catch insects within `GatherRange` |
| `GatherFish` | 1 | 0/1 | Harvest | catch fish within `GatherRange` by hand |
| `GatherCarcass` | 0 | 0/1 | Harvest | loot animal carcasses without the skinning animation |
| `Hunting` | 1 | 1..100 | Harvest | multiplier on what a looted carcass gives, however it is looted |
| `ScanRange` | 40 | 1..200 | Harvest | survey radius in game metres |
| `GatherRange` | 6 | 1..50 | Harvest | the gather key and auto mode pick the nearest node within this many metres |
| `GatherInterval` | 500 | 100..60000 | Harvest | milliseconds between two automatic sends |
| `NodeCooldown` | 8000 | 100..60000 | Harvest | milliseconds to leave a node alone after sending for it |
| `StackLimit` | 999 | 10..1000000 | Harvest | with a full bag, still gather onto a stack unless it would pass this |
| `Foraging` | 1 | 1..100 | Harvest | yield of plants, fruit, berries, mushrooms, crops, and the water well |
| `Logging` | 1 | 1..100 | Harvest | yield of firewood |
| `Mining` | 1 | 1..100 | Harvest | yield of the `collect_mine` family |
| `Ore` | 1 | 1..100 | Harvest | yield of the `collect_ore` family |
| `Money` | 1 | 1..100 | Harvest | the placed coin and silver bar props only |
| `Bugs` | 1 | 1..100 | Harvest | insects caught by hand (a code patch on the catch count) |
| `Fish` | 1 | 1..100 | Harvest | fish caught by hand (the same patch) |
| `KeyToggle` | F10 | key name | Settings | toggle automatic gathering |
| `KeyScan` | F11 | key name | Settings | write the survey to the log |
| `KeyGather` | F9 | key name | Settings | gather the nearest node once |
| `KeyRecord` | F7 | key name | Settings | toggle the event recorder |
| `Debug` | 0 | 0/1 | Debug | very verbose survey dump on the next F11, the non-gather records (capped at 400 lines) and one line per creature caught |
| `LogReceived` | 0 | 0/1 | Debug | log every item the game hands the player as `[recv] item <key> x<count>` |
| `SurveyLines` | 200 | 16..2000 | Debug | actor lines one F11 survey prints |
| `BagTab` | 1 | -1..15 | Debug | which inventory tab counts as the bag; -1 = the tab with the largest capacity |
| `DryRun` | 0 | 0/1 | Debug | yield multipliers only: log what would change and write nothing. Auto-loot still runs |
| `DumpTable` | 0 | 0/1 | Debug | write the vanilla `gimmickinfo` table to `DesertTooling.gimmickinfo.bin`; needs `DryRun=1` |

## `[Overlay]`

The menu notes on this section: `Theme` changes live; everything else here is read once at startup, so a change is saved at once and takes effect at the next launch.

| key | default | range | tab | meaning |
|---|---|---|---|---|
| `Enabled` | 1 | 0/1 | Settings | 0 = no graphics hook at all; the first thing to try while chasing a crash |
| `ShowOnStart` | 0 | 0/1 | Settings | the menu is open at the first frame |
| `KeyMenu` | INSERT | key name | Settings | show and hide the menu |
| `Scale` | 0 | 0..4 | Settings | menu size; 0 follows Windows display scaling, below 0.5 counts as 0 |
| `FontSize` | 20 | 8..72 | Settings | text height in pixels before `Scale` |
| `Theme` | banner | classic, parchment, gilded, splash, banner, steel | Settings | colour theme |
| `ColorSpace` | auto | auto, sdr, hdr10, scrgb | Settings | what the menu's pixels are encoded for; leave on auto |
| `HdrBrightness` | 203 | 80..1000 | Settings | brightness of the menu's white on an HDR display |
| `Debug` | 0 | 0/1 | Debug | log every ini write and reload, and hudhook's own debug lines |

`Font` is deliberately not in the schema, so the menu does not show it and a seeded ini does not contain it. Its value is a file name looked up in the Windows Fonts folder, an absolute path, or empty for Dear ImGui's built-in font, optionally suffixed `:N` to pick a face from a `.ttc`. The parser still reads it and the menu's rewriter leaves it alone. The default is `segoeui.ttf`.

## `[Dispatch]`

| key | default | range | tab | meaning |
|---|---|---|---|---|
| `Enabled` | 1 | 0/1 | Dispatch | master switch; off before launch, nothing is read for the whole session |
| `Speed` | 1 | 1..100 | Dispatch | divide every mission's duration by this; never below 6 in-game minutes, about half a real minute |
| `NoSkillRequirement` | 0 | 0/1 | Dispatch | clear the skill a mission demands of a worker |
| `AnyOperatorCount` | 0 | 0/1 | Dispatch | let every mission start with a single worker |
| `Rewards` | 1 | 1..100 | Dispatch | multiply how much of each item a mission pays |
| `LogRecords` | 1 | 0/1 | Debug | one log line per dispatch mission |
| `DumpRewards` | 1 | 0/1 | Debug | dump the reward rows the missions name |
| `MaxLines` | 4000 | 0..20000 | Debug | ceiling on the lines one pass writes; summaries are never capped |
| `DryRun` | 0 | 0/1 | Debug | log every change and write nothing |
| `Debug` | 0 | 0/1 | Debug | also log empty nodes, every change, and every skip reason |
| `DumpRaw` | 0 | 0/1 | Debug | raw hex of every mission and reward row |
| `DumpBuffs` | 0 | 0/1 | Debug | read-only census of the buff and stat tables, tagged `[buffs]` |

## Multipliers at 1 are vanilla

Every multiplier ships at 1, which is vanilla: the seven yield multipliers and `Hunting` under `[Harvest]`, and `Speed` and `Rewards` under `[Dispatch]`. Deleting `DesertTooling.ini` makes the plugin write a fresh one at every default on the next launch, which turns all of them off.
