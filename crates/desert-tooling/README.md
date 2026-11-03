# Desert Tooling

**Version 0.16.0**, for Crimson Desert Enhanced, Steam build **26361232**.
[Changelog](CHANGELOG.md) · [versioning](../VERSIONING.md).

One `.asi` plugin with three subsystems: **harvest** (auto-loot and the
gathering yield multipliers), **dispatch missions** and an **in-game settings
menu**. Written in Rust, installed by dropping two files into `bin64`.

| subsystem | what it does | ini section | log tag |
|---|---|---|---|
| Harvest | gathers plants, ore, stone, wood, insects and fish around you, and multiplies what a gathering node, a water well, a caught creature or a coin lying in the world gives | `[Harvest]` | `[harvest]` |
| Dispatch | shorter dispatch missions, bigger mission rewards, no skill or headcount gate | `[Dispatch]` | `[dispatch]` |
| Overlay | the settings menu, `Insert` | `[Overlay]` | `[overlay]` |

## Upgrading from 0.9 and earlier

`[Looter]` and `[Gatherer]` are now one section, `[Harvest]`, and your
`DesertTooling.ini` is **migrated on first launch**: the plugin rewrites the two
old sections into one `[Harvest]` section where `[Looter]` was, keeping every
value, comment and hand-added key, and leaves `[Overlay]` and `[Dispatch]`
exactly as they were. The log says so once:

```text
[tooling] [ini] [Looter] and [Gatherer] were merged into [Harvest] (Enabled is on only if both were, Debug if either was)
```

There is now one `Enabled` and one `Debug` for the whole section. `Enabled` comes
out on only if it was on in **both** old sections, so nothing starts doing
something you had switched off; if the two disagreed, a comment above the new
`Enabled` line says so and what to set. If what you wanted was the multipliers
without the auto-loot, that is `AutoGather=0` now (the hotkeys stay live).
`DryRun` and `DumpTable` stay, and still act on the yield multipliers only.

A file that already has a `[Harvest]` section is not touched. If it still has an
old `[Looter]` or `[Gatherer]` section beside it, those are not read, and the log
warns about it.

## Upgrading from Desert Looter, Desert Gatherer and Desert Overlay

Those three plugins are this one. **Delete them first:**

1. Remove `DesertLooter.asi`, `DesertGatherer.asi` and `DesertOverlay.asi` from
   `bin64`, along with their `.ini`, `.log` and `.overlay.ini` files.
2. Extract this release into `bin64`.
3. Re-enter your settings. Nothing is migrated from those plugins: their ini
   files are not read, and `DesertTooling.ini` starts at every key's default.

Leaving an old `.asi` in place is not just untidy: two copies of the same plugin
patch the same game function, the second trampoline overwrites the first one's
stolen bytes, and the game crashes to desktop. So Desert Tooling asks the
Windows loader for those three names before it installs anything, and if any of
them answers it installs **nothing**, starts no subsystem, and names the file to
delete in the log:

```text
[tooling] REFUSING TO START: DesertLooter.asi still loaded in this process alongside DesertTooling.asi
```

## Install

The release archive holds five files at its root (`DesertTooling.asi`,
`DesertTooling.ini`, `README.md`, `CHANGELOG.md` and `LICENSE`), so it suits
either way of installing it.

### By hand

Requirements: the game, and Ultimate ASI Loader present in `bin64` as
`winmm.dll` (it loads every `.asi` file beside it).

Extract the archive straight into `<game>\bin64\`, or copy two files there:

- `DesertTooling.asi`: the plugin. Rename the built `desert_tooling.dll` to this.
- `DesertTooling.ini`: the settings. Optional: if it is missing, the plugin
  writes one itself with every key at its default on the next launch, so you
  always end up with a file to edit.

To remove the plugin, delete `DesertTooling.asi`. Nothing is installed anywhere
else and no game file is modified.

### Install with DMM

Definitive Mod Manager accepts an ASI mod as either a single `.asi` file or a
directory, so drop the release zip onto DMM's window, or extract it and put the
folder under `<DMM>\mods\_asi\`. DMM copies `DesertTooling.asi` and the `.ini`
beside it into the game's `bin64` itself, and it installs Ultimate ASI Loader as
`winmm.dll` for you, so there is nothing to set up beforehand. Enable the mod in
DMM's ASI tab; the same tab turns it off again. Read
[Conflicts with DMM](#conflicts-with-dmm-important) first: DMM's own gathering
multiplier and the "The Desert Gatherer" module pack change the same numbers,
and must stay unmounted.

### About DMM's security scan

DMM grades every plugin it installs, and it will grade this one **Suspicious**.
That is an honest reading of what the plugin does, not a finding against it: it
imports `WriteProcessMemory`, `ReadProcessMemory` and `VirtualProtect`, because
that is how it installs its hooks into the running game and how it reads and
writes game memory safely. (Every read of game memory goes through
`ReadProcessMemory` on the plugin's own process, so a page that has been
unmapped returns nothing instead of faulting and crashing the game.) No mod that
changes a running game's behaviour can avoid those imports.

Everything else the scan looks at is clean: the DLL is not packed and is of
ordinary entropy, has no data appended past its last PE section, has no
writable-and-executable sections, and carries a version-info resource (product
name, version, `KiefBC`, MIT). It is **not** digitally signed (a code-signing
certificate costs money), so check the `SHA256SUMS` file from the release
instead. And the source is public: all of it is in this repository.

## The menu

Press **Insert** and a window appears over the game (DirectX 12, Dear ImGui)
with every setting in it as a checkbox, slider or key picker, behind a row of
tabs: **Harvest** and **Dispatch**, then **Settings** (every key you can bind,
and how the menu itself looks, the overlay's own included) and **Debug** (every
logging switch and dump). Change one and
it takes effect within about a second, without leaving the game and without a
restart.

The ini is the whole contract. The menu writes `DesertTooling.ini` and each
subsystem re-reads its own section of it once a second, so:

- **Every change is permanent.** It is in the file, so it is still there next
  time you launch. There is no separate "apply" or "save".
- **Editing the file by hand still works**, even while the game is running. The
  menu notices within a second and shows the new value.
- **Your file is not rewritten, only edited.** The menu replaces the value on
  the lines whose key it owns, under the right `[Section]`, and touches nothing
  else: your comments, your ordering and any key it does not draw come through
  exactly as they were. Writes go through a temporary file and a rename, so a
  subsystem reading the file at the wrong moment never sees half of it.

Presets sit at the top of the Harvest tab: `Everything`, `Plants only`, `Wood only`,
`Rock and ore only`. One click sets the four gather families and ground items
and leaves everything else alone. Below them come `Enabled` and `AutoGather`,
what auto-loot takes, `Hunting`, the item-log filter, the ranges and pacing, and then the yield
multipliers, `Foraging` to `Fish`.

The sliders and number fields stop at the ranges the parsers accept, so the menu
cannot produce a value its own subsystem would refuse. A number typed into a
field is written when you press Enter or click away, not while you are still
typing. If the file cannot be read or written, the reason appears in red in the
menu's footer, in full on hover, and in `DesertTooling.log`; nothing is lost and the game is
unaffected.

**While the menu is open**, keyboard and mouse input is held back from the game,
so typing a number does not also swing your sword and dragging a slider does not
turn the camera. Window messages are not blocked: alt-tab still works. The game
hides the hardware cursor, so the overlay draws its own and releases the cursor
clip while the menu is up; the game takes the cursor back the next time it wants
it.

## Keys

| key | action |
|---|---|
| Insert | show and hide the settings menu |
| F9 | gather the nearest eligible node or item once |
| F10 | toggle automatic gathering on and off |
| F11 | write a survey of everything nearby to the log (read-only); gather nodes and items are listed first, up to `SurveyLines`, and a final line says how many actors were left unprinted |
| F7 | debug: log the events the game queues, up to 25 lines per event type and 300 in all, then it stops by itself and prints a census of everything it saw; press again to stop early |

Every gathering keypress beeps once. All five keys can be changed in the ini or
from the menu: Insert is `KeyMenu` under `[Overlay]`, the other four are
`KeyGather`, `KeyToggle`, `KeyScan` and `KeyRecord` under `[Harvest]`.

## What the Looter does

- **Gathers nodes** within a few metres: flowers, berries, fruit, ore veins and
  rocks, and the firewood you cut from felled logs. Veins and rocks are mined
  outright, without a swing.
- **Picks up plain ground items** such as ore chunks. Merchant goods, quest
  items and props are never touched.
- **Skins animal carcasses** within the same range when `GatherCarcass=1`, off
  by default. The game's own "search the carcass" event is queued directly, so
  the loot arrives without the skinning animation. A carcass you have already
  emptied is left alone. `Hunting` multiplies what a carcass gives, 1..100, and
  ships at 1 - it works whether the plugin skins the animal or you do.
- **Searches dead bodies** within the same range when `GatherBodies=1`, off by default, with
  the same game event as a carcass. `GatherBodies` covers hostile people and monsters (bandit
  gangs, criminals, monsters, bosses: the bodies with a grey map marker). `GatherFriendlyBodies=1`,
  which needs `GatherBodies=1`, adds people who were not enemies (citizens, traders, nobles,
  named unique NPCs, guards, mercenaries: the white-marker bodies). Which a body is comes from the
  game's own ally groups (enemy, wild, or NPC, guard and mercenary), not from whether a character
  can be befriended. Your companions and the playable characters are never searched, whatever
  these two settings say. A body with an owner or quest trigger set on it is never searched (the same test the
  game makes before its own loot prompt), and a body already emptied is left alone. `Hunting`
  multiplies what a body gives too. Weapons and armour an enemy drops on the ground are ordinary
  ground items, picked up under `GatherItems` (and `GatherGear` for armour and jewellery), not by
  `GatherBodies`.
- **Catches insects, fish and critters** within the same range, using the game's own
  "catch" event rather than a pickup. Fish within reach at the water's edge go
  the same way, with the very same event as an insect. The steal check applies
  to all of them, so a creature the game counts as someone else's is left alone.
  `GatherBugs=0`, `GatherFish=0` and `GatherCritters=0` turn them off separately.
  A **critter** is a small animal the game puts in the bag when you catch it by
  hand: the five small lizards (Chameleon, Frilled-Neck, Goanna, Heloderma,
  Iguana), ten small birds, four rats, frogs, toads, salamanders, axolotls, the
  Mole, Muskrat, Opossum and the Baby Hedgehog. The giant riding iguanas are not
  critters. Which of the three a creature is comes from the game's own species
  records, so the Rice Fish, Lobster and Sea Slater are fish whatever their
  record names say. **Ducks and raccoons are never caught**: you carry, kill and
  skin them, they never go into the bag, so neither auto-catch nor any
  multiplier touches them.
- **Leaves alone what it does not recognise.** Insects and fish are told apart
  from birds in flight and other wildlife by a class byte on the creature
  itself, and only the classes that have actually been caught by hand are ever
  targeted. For fish the creature's type has to match too, because one fish
  class is shared with a land animal. Anything else is passed over and reported in the log once per class,
  so an unrecognised species shows up as a line to add rather than as something
  grabbed by mistake.
- **Asks the game about stealing first.** Before every pickup, node or ground
  item alike, it asks the game whether taking that thing would count as
  stealing, and skips it if so. The fruit trees and food plants inside a
  settlement, which the game offers as "Steal" rather than "Gather", are left
  alone; the same plants out in the wild are gathered as usual.
- **Respects the bag.** Nothing is sent when the bag is full unless every item
  the node can give would stack onto a stack you already carry, read from the
  node's own record. Repeated refusals switch auto mode off.
- **Leaves alone**: standing trees, log chunks, animals and NPCs, furniture,
  breakable pots, market-stall goods and, by default, dropped weapons and
  armour.

Nothing is simulated and no input is faked: the plugin sends the game the same
pickup event it sends when you press E.

## What the Gatherer does

Every gathering node in the game carries a minimum and a maximum quantity for
each item it can produce. Desert Tooling multiplies both, in 294 records across
five independent families:

| family | what it covers | records |
|---|---|---|
| Foraging | plants, fruit, berries, mushrooms, crops, tree branches, wild rock honey, and the water you draw from a **water well** | 97 |
| Logging | firewood cut from felled trees (`firewood_*`) and firewood piles | 142 |
| Mining | the `collect_mine` family: rocks, veins, cave variants, breakable stalactites | 36 |
| Ore Nodes | the `collect_ore` family: `ore_*` deposits, sulfur stone, collectible stalactites | 16 |
| Money | the coins placed in the world: **money lying around, and nothing else** | 3 |

Each family has its own multiplier, 1 to 100. Mining and Ore Nodes look alike in
game but are separate families in the game's data; set both if you want all
mining-style gathering raised together. Nothing is patched on disk: the numbers
are multiplied in memory as the game loads them.

They are read off the game's own data (how each record is interacted with, and
what it pays out), plus a few records that data alone does not mark, among them the
three placed money records, which are the `Money` family and get their own
section below, and the water well, which pays out through the same kind of
record as a bush and so sits under `Foraging`. Water from a well is a thing you gather out of the world
like everything else there, and a player reading "plants, fruit, berries" would
not guess it, so it is said here: **`Foraging` also multiplies the water you
draw from a well.** It does *not* touch the breakable water pots or the goods
sitting on market stalls: those hand you a ready-made item and never read the
table, so they are not in any family.

**`Money` multiplies the money you find lying in the world, and nothing else.**
Three records in the gather table are placed money props - the small coin prop
(`gimmick_item_common_coin_0001`, 10..15), the large one (100..150) and a
silver-bar record (a fixed 2500) - and all three pay the game's one money item,
which the bag calls `Money_Copper`. Copper, silver and gold are display units
over that one count (2500 copper shows as 25 silver), so the amount *is* the
denomination, and a multiplied pickup may show in a different unit than the
vanilla one did. It does **not** touch what enemies drop, what quests pay, what a
coin pouch opens to, what a chest holds, what anything sells for, or what a
dispatch mission returns. This is the one setting here that is a *balance* lever
rather than a convenience one, which is why it ships at `1` on purpose. Measured
in vanilla on 2026-09-13: three pickups of the small coin prop paid 15, 14 and
15, all inside its 10..15 block, and one pickup at `Money=3` paid 30 with the
bag up by exactly 30, so the prop rolls its record and the same edit that
multiplies a bush multiplies it, live, after a slider change mid-session.
At `Money=100` the same day, two coin pickups between two F11 surveys moved the
bag from 11919 to 14384 copper, two grants inside the multiplied 1000..1500.

**The pickup prompt counts copper; the HUD counts silver.** A prompt reading
1,400 on a multiplied coin is 1,400 copper, which lands as 14 silver on the
HUD. The base coin is worth a few copper, so even at 100x one coin is a
handful of silver, never a thousand.

Three more multipliers, **Bugs**, **Fish** and **Critters**, cover the creatures you catch by
hand. Those are not gathering nodes and there is no record anywhere saying what
one is worth: the game grants exactly one, from a constant in its own code. So
that pair works differently from the four families (it is a small patch on that
constant, applied as the creature is caught), and it is the one part of this
plugin a game update can switch off on its own. If that happens the log says so
once at startup (`[catch] signature not found; catch multipliers off`) and
everything else here goes on working.

Because that patch is in the game's own grant code rather than in anything the
auto-loot sends, it multiplies the creatures **you** catch by hand exactly as it
multiplies the ones auto mode catches for you.

One creature pays out twice: a Firefly Colony gives you colonies *and*
fireflies, and because the game rolls a handful of fireflies for each colony you
take, the fireflies come out at roughly double your `Bugs` setting and a
different number every time: at `Bugs=10`, ten colonies and somewhere around
twenty fireflies. That is the game's own drop rule, not a bug in the patch.

**The multipliers** do not touch enemy loot, chests, rod-and-line fishing, quests,
Abyss objects, artifacts, gates or fast travel - and `Money` does not reach the
money any of those pay, only the coins placed in the world. They do not reach
what a skinned carcass gives either: a carcass's drops come from a different
table than the gather nodes these multipliers edit. **That one has its own key
now** - `[Harvest] Hunting` - and it works a different way: rather than editing a
table, it hooks the game's own loot grant and scales the amounts the game rolled
for that one corpse a moment before they reach your bag. So it works however the
carcass was looted - by the gather key or by hand - and nothing else in the
game's loot moves with it. Its gate is the game's own "loot method": method 0 is
the one the game calls searching or skinning, which is the honest limit of the
key, since the plugin can tell which method is in play but not what is lying on
the ground.

### What to expect in game

The multiplier scales the numbers in the record, not the amount you happen to
get on one pick. Four things surprise people:

- **A node picks one of its ranges, then rolls it.** Most records carry several
  resource-output blocks, and a single gather pays out one of them, chosen at
  random, rolled between its own minimum and maximum. So the same bush can give
  you a big handful one time and a single item the next, in vanilla and with the
  plugin alike.
- **Everything shifts up together.** A peony carries four blocks, 4-7, 2-3, 1-1
  and 1-1. At `Foraging=2` those become 8-14, 4-6, 2-2 and 2-2:

  | pick | vanilla | at 2x |
  |---|---|---|
  | the big block | 4-7 | 8-14 |
  | the middle block | 2-3 | 4-6 |
  | either small block | 1 | 2 |

  Measured in game: a peony gave 2 in vanilla, and 2, 2 and 12 across three
  picks at 2x. Nothing is wrong there. The 2s are the small blocks doubled from
  1, the 12 is the big block. Watch a dozen picks, not one.
- **Single-item nodes are the clean case.** Firewood and ore deposits carry one
  block of 1-1, and rocks carry three blocks that are each 1-1, so a pick goes
  from exactly 1 to exactly 2 at 2x, every time. If you want to check the plugin
  is working at all, check one of those.
- **A well never rolls.** Its record's minimum and maximum are already the same
  number, so unlike a bush there is no range to land anywhere in: a well that
  gives 5 gives exactly 15 at `Foraging=3`, every time - measured, with the
  bag count logged before and after. And the multiplier is read when you
  **take** the water, not when the bucket fills: raise a full bucket at 3,
  turn the slider to 6, take it, and you get 30.
- **A coin prop rolls a small range.** The common coin prop pays 10 to 15 in
  vanilla (measured: 15, 14, 15); at `Money=3` one pickup pays 30 to 45, and the
  game may show it in silver once it crosses the display threshold. The prompt
  on the coin is in copper; divide by 100 for what the HUD will add.

One more thing that looks odd: pressing E can hand the total over as several
separate "x1" pickups a second or so apart. That is the game's own delivery
animation. The total across them is the multiplied amount.

## Conflicts with DMM (important)

DMM's **"The Desert Gatherer" module pack** and DMM's **built-in gathering
multiplier preset** change the same minimum/maximum quantities, on disk.

If either is mounted while this `.asi` is installed, the two stack: a mounted
DMM 5X plus `Mining=2` here gives 10x. **Unmount both in DMM before using this
plugin.** The `desert-gatherer-dmm/` directory in this repository is that pack,
for players who use DMM without an ASI loader; it is generated from the same
records this plugin multiplies (all but the three Money records), so every one
of them stacks.

## Settings (`DesertTooling.ini`)

One file, three sections. A key belongs to the `[Section]` header above it:
`Enabled` and `Debug` exist under all three and mean different things in each,
and `DryRun` exists under `[Harvest]` and `[Dispatch]`.

### `[Harvest]`

Auto-loot and the yield multipliers, one section since 0.10.0 (it was `[Looter]`
and `[Gatherer]` before; see [Upgrading](#upgrading-from-09-and-earlier)).
`Enabled` and `Debug` are read by both halves; every other key belongs to one.

| key | default | meaning |
|---|---|---|
| `Enabled` | 1 | 0 = idle: no gathering, no hotkeys, and vanilla yields. Every hook stays installed: the record-loader hook still reads each record to keep its remembered table current but writes nothing, and a pass right after the change writes vanilla numbers back into whatever is already loaded. Live: set it to 1 while the game runs and both halves come back, the multipliers re-applied the same way |
| `Debug` | 0 | 1 = very verbose survey (first F11 dumps hundreds of lines), plus the records that are not gather nodes (capped at 400 lines) and one `[catch] debug: type=.. class=.. chr=..` line per creature caught |
| `GatherRange` | 6 | how far a node may be, in metres (max 50) |
| `AutoGather` | 0 | 1 = auto mode on from the start (F10 flips it) |
| `GatherInterval` | 500 | milliseconds between automatic pickups |
| `NodeCooldown` | 8000 | milliseconds before a node is tried again |
| `GatherUnarmed` | 1 | also take nodes the game has not "armed" yet (this is what mines whole veins) |
| `GatherItems` | 1 | pick up plain ground items such as ore chunks |
| `GatherGear` | 0 | also pick up dropped weapons and armour |
| `GatherForaging` | 1 | 0 = pass over plants, fruit, berries, mushrooms, crops |
| `GatherLogging` | 1 | 0 = pass over firewood cut from felled trees |
| `GatherMining` | 1 | 0 = pass over the `collect_mine` family: rocks, veins, stalactites |
| `GatherOre` | 1 | 0 = pass over the `collect_ore` family: ore deposits and sulfur stone (separate from Mining; set both to gather all of them) |
| `GatherMoney` | 0 | 1 = also pick up the coin props lying in the world. Auto-loot only; the amount is the `Money` multiplier below. Untested in game, so off by default |
| `GatherBugs` | 1 | 0 = do not catch insects (they are a separate game event, not a gather family) |
| `GatherFish` | 1 | 0 = do not catch fish (same event as insects, at the water's edge) |
| `GatherCritters` | 1 | 0 = do not catch critters: small lizards, small birds, rats, frogs, toads, salamanders (same event as insects). Ducks and raccoons are never caught, whatever this is |
| `GatherCarcass` | 0 | 1 = also loot animal carcasses within `GatherRange`, without the skinning animation. A carcass already skinned gives nothing, and the classes known to be people are refused. 0 by default: opt in |
| `GatherBodies` | 0 | 1 = also search dead hostile people and monsters (bandits, criminals, monsters, bosses: grey map marker) within `GatherRange`, with the carcass event. A body with an owner or quest trigger is skipped. Items dropped on the ground are not part of this: they stay under `GatherItems`/`GatherGear`. 0 by default: opt in |
| `GatherFriendlyBodies` | 0 | 1 = also search the bodies of people who were not enemies (citizens, traders, nobles, named NPCs, guards, mercenaries: white map marker). Companions and playable characters are never searched. Needs `GatherBodies=1`. 0 by default |
| `Hunting` | 1 | 1..100, how much a looted carcass or searched body gives. Does **not** need `GatherCarcass` - it hooks the game's own grant, so it reaches a carcass whether the plugin skins it or you do. It scales the amounts the game rolled for that one corpse, so no table is edited and no other kind of loot changes. A code patch, so a game update breaks it first and the log says so. 1 by default: it changes how much you get, not how many trips you make |
| `LootLogMinTier` | 0 | 0..6, which pickups get an "Item xN" line in the item log at the side of the screen. 0 = every line, as the game does; 1..5 = only items of at least that tier; 6 = none. Only the line is hidden: the item still goes into the bag and quests still count it. Takes effect at once. A code patch on the game's own line builder, so a game update can break it; the log says so and the item log stays vanilla |
| `LootLogMoney` | 1 | with `LootLogMinTier` above 0, coins and camp funds still get their line |
| `BagTab` | 1 | which inventory tab is the bag for the full check |
| `StackLimit` | 999 | at a full bag, do not grow a stack past this |
| `ScanRange` | 40 | radius of the F11 survey |
| `SurveyLines` | 200 | how many actor lines one F11 prints; gather nodes and items come first, and the survey says how many it cut |
| `LogReceived` | 0 | 1 = log every item the game hands you as `[recv] item <key> x<count>`, plugin-caused or not (capped at 500 a session); for measuring yields. Coin props are not logged: measure money with the bag line of two F11 surveys |
| `KeyToggle`, `KeyScan`, `KeyGather`, `KeyRecord` | F10, F11, F9, F7 | see the key names below |
| `DryRun` | 0 | 1 = log every yield change that would be made and write nothing; for troubleshooting and after game updates. The yield multipliers only: auto-loot still runs |
| `DumpTable` | 0 | 1 = on the next launch, write the game's vanilla gimmickinfo table body to `DesertTooling.gimmickinfo.bin` beside the log (about 22 MB, once per session, logged as `[dump]`). Needs `DryRun=1` so nothing in the copy is multiplied; without it the log says it was ignored. A diagnostic copy; nothing in the game or the tools needs it |
| `Foraging` | 1 | multiplier for plants and crops, and for the water drawn from a water well, 1..100 |
| `Logging` | 1 | multiplier for firewood, 1..100 |
| `Mining` | 1 | multiplier for `collect_mine`, 1..100 |
| `Ore` | 1 | multiplier for `collect_ore`, 1..100 |
| `Money` | 1 | multiplier for the coins lying in the world, 1..100. Nothing else: not enemy drops, quest rewards, pouches, chests or prices |
| `Bugs` | 1 | multiplier for insects caught by hand, 1..100. A code patch on the catch count, not a table edit |
| `Fish` | 1 | multiplier for fish caught by hand, 1..100. Same patch as `Bugs` |
| `Critters` | 1 | multiplier for critters caught by hand (small lizards, small birds, rats, frogs, toads, salamanders), 1..100. Same patch as `Bugs`. Ducks and raccoons are never multiplied |
| `Pouches` | 1 | multiply what an item pays when you open it, 1..100: the silver pouches, Box of Fortune, seed packs, recipe and gear boxes (88 drop-table rows no other source pays from). Not `Money` above, which is the coin piles in the world; the two do not stack. The first pouch of a kind opened in a session may pay vanilla while the game loads its row. Written by the Dispatch subsystem, so it needs `[Dispatch] Enabled=1`; `[Harvest] DryRun` covers it. A `[Dispatch]` key in 0.15, moved here automatically |
| `Breakables` | 1 | multiply the loot of barrels, crates, pots and sacks you smash, 1..100 (15 drop-table rows no other source pays from, so a chest sharing a barrel's table stays vanilla). The first of a kind smashed in a session may pay vanilla. Needs `[Dispatch] Enabled=1`, like `Pouches` |

For the multipliers, `1` means vanilla: that family's records are read and left
untouched. A value outside 1..100, or one that is not a number, is refused with
a warning in the log and the default is kept.

**The plugin never takes a water well's bucket**, whatever `GatherForaging`
says, and F9 will not either. The well's record is a `Foraging` record, so the
`Foraging` multiplier reaches it - but a pickup the plugin sends at the bucket
is delivered from outside the well's own crank sequence, and that sequence is
what puts the next bucket there. Tried on 2026-09-13: the water arrived, the
bucket vanished, and the well stayed empty for good. So at a well you do all of
it yourself - lower, crank, take - and the multiplier still applies to what
you take. When the nearest thing is a well the F9 line says it was refused
rather than pretending nothing was there.

On the menu, `Enabled`, `AutoGather`, what auto-loot takes, `Hunting`, the item-log filter, the ranges
and pacing and every multiplier are on the Harvest tab; the four keys are on
Settings; `Debug`, `LogReceived`, `SurveyLines`, `BagTab`, `DryRun` and
`DumpTable` are on Debug.

### `[Overlay]`

Read **once** at game start; a change to this section needs a restart. (It is
only the menu's own settings that are not live; everything it puts on screen
is.)

| key | default | meaning |
|---|---|---|
| `Enabled` | `1` | `0` = install no graphics hook at all, so the game renders exactly as it would without the `.asi` and there is no menu. The other subsystems are unaffected |
| `Debug` | `0` | `1` = log every ini write and reload, plus everything hudhook says (its per-frame debug and trace lines included, capped at 20000 lines per session) |
| `KeyMenu` | `Insert` | the key that shows and hides the menu |
| `ShowOnStart` | `0` | `1` = the menu is already open at the first frame |
| `Scale` | `0` | menu size: `0` follows the Windows display scaling, otherwise a fixed factor from `0.5` to `4` |
| `FontSize` | `20` | height of the menu's text in pixels before `Scale` is applied, from `8` to `72`. The font is rasterised at that size, so a larger value is sharper rather than blockier |
| `Font` | `segoeui.ttf` | the font the menu is drawn in. A bare file name is looked up in `%WINDIR%\Fonts`, an absolute path is used as it stands, and an empty value goes back to Dear ImGui's built-in font. `georgia.ttf`, `constan.ttf` and `cambria.ttc:0` give a more fantasy, serif look; the `:N` suffix picks a face out of a `.ttc` collection. A missing or unreadable file is a warning in the log and the built-in font |
| `HdrBrightness` | `203` | paper white in nits on an HDR display: how bright the menu's white is drawn. `80` to `1000`; ignored on SDR |
| `ColorSpace` | `auto` | what the menu's pixels are encoded for: `auto` (follow the swapchain), `sdr`, `hdr10` or `scrgb`. Anything else is `auto` with a warning in the log |
| `Theme` | `banner` | the menu's colour theme: `classic`, `parchment`, `gilded`, `splash`, `banner` or `steel`. The `Theme` setting on the menu's Settings tab switches between them live; this key is what makes a choice stick across launches |

The game presents an HDR10 (PQ) signal, so the menu is converted into the
swapchain's colour space before it is drawn; without that its sRGB colours come
out blown out and oversaturated. `auto` reads the colour space off the swapchain
and is right unless the detection is wrong, which is what the three forced
values are for. `HdrBrightness` is the only one worth touching in normal use:
raise it if the menu looks dull next to the game, lower it if it glares.

### `[Dispatch]`

Dispatch missions ("faction operations") are the jobs you send workers away on
from the faction map. This section makes them finish sooner, pay more, and stop
turning you away.

| key | default | meaning |
|---|---|---|
| `Enabled` | `1` | Turned off **while the game runs**, every mission and reward it changed goes back to vanilla. Turned off **before you launch**, the pass never runs and nothing in the game is read at all, and it stays that way for the session: turning it back on, here or in the menu, does nothing until the next launch |
| `Speed` | `1` | divide every mission's duration by this, 1..100. Vanilla runs 2h to 96h; `4` turns a 16h mission into a 4h one. Nothing ever goes below 6 minutes. All in-game time (the clock runs about 12x real time, so the floor is about half a real minute). Reaches missions already under way |
| `NoSkillRequirement` | `0` | `1` = clear the skill a mission demands of an assigned worker, on the 147 missions that demand one. Checked when a mission starts |
| `AnyOperatorCount` | `0` | `1` = let every mission start with a single worker. 693 of the game's 936 missions already ask for one, so this changes the other 243, the ones wanting 2, 3, 5, 8, 10 or another number. Checked when a mission starts |
| `Rewards` | `1` | multiply how much of each item a mission pays, 1..100. Both ends of every reward range are scaled, so 2-3 becomes 6-9 at x3. Reaches rewards already waiting to be paid |
| `LogRecords` | `1` | `1` = one line per mission in the log |
| `MaxLines` | `4000` | ceiling on the lines one pass writes, 0..20000. The summaries are never capped, and hitting it never stops a change being made |
| `DryRun` | `0` | `1` = log every change that would be made and write nothing |
| `Debug` | `0` | `1` = also log the nodes with no missions, every change one line at a time, and why anything was stepped over |
| `DumpRaw` | `0` | `1` = log each mission's raw bytes as hex, for chasing an offset after a game update |
| `DumpRewards` | `1` | `1` = also dump the reward rows the missions name. A diagnostic only: `Rewards` reads and edits those rows either way |
| `DumpBuffs` | `0` | `1` = census the game's buff and stat tables into the log, read-only: the buff effects that touch drop rates, sell prices, crime prices or dispatch reward rates, and the stat rows behind money and equipment drops. A diagnostic for one investigation, tagged `[buffs]`; it edits nothing |

A change takes effect within about a second. Only the 219 reward rows dispatch
missions actually name are touched, never the game's wider drop table, so this
does not become a loot multiplier for chests and carcasses. `[Harvest]`'s
`Pouches` and `Breakables` are the two exceptions, written by this subsystem's
pass (so they need `Enabled=1` here) but set, and dry-run, from `[Harvest]`,
by design and by a fixed list: the rows
`just gen-dropsets` picked out of the game's tables because nothing but a pouch
or a breakable pays from them, each checked against that list (key, entry count
and vanilla amounts) before a byte of it is written. No game code is
patched and no hook is installed: the plugin edits five fields of the tables the
game has already read (four settings, five fields: `Rewards` writes both ends of
a min/max pair), on its own thread, and remembers what each one said first so it
can put it back.

**It checks before it writes.** Every launch, the plugin first reads every
mission and reward row as the game left them and judges whether the layout it
knows still fits: real durations and crew sizes that vary from mission to mission,
and reward rows whose odds add up to the total the game keeps beside them. Until
that check passes on this launch, nothing in this section writes anything, exactly
as if `DryRun=1` were set, and the log says why, once:

```text
[dispatch] writes disabled (missions): <reason> ...
```

(or `(reward rows)`; the reward side also waits for the missions to pass). Once
it passes it stays passed for the session. This is what keeps a game update that
moved these tables from turning `Speed` or `Rewards` into writes in the wrong
places: after an update that moved them, the section does nothing until a new
version of the plugin knows the new layout. Build 26345091 moved them, and 0.10.0
knows its layout.

**Two of the settings reach missions that are already out, and two do not.**
`Speed` shortens a mission already under way (the game checks a mission's
progress against the duration in the table every time it ticks), and `Rewards`
reaches rewards already waiting to be paid, because the game reads the amounts at
the moment the items land. `NoSkillRequirement` and `AnyOperatorCount` are
checked when a mission *starts*, so a mission already out keeps running under the
rules it started with. A **repeating** mission is the exception to that: it
re-checks on every restart, which is why a repeating mission that needed a skill
or a full crew stops with an error message at its next restart once the plugin is
gone.

**Nothing here is saved into your game.** The mission and reward tables are read
from the game's own files every launch, so setting these back to `1` and `0` puts
every mission back, and deleting the plugin leaves nothing to undo. `Rewards`
banks nothing either: a reward still waiting on you is worked out from the table
at the moment it lands, so set it back to `1`, or remove the plugin, and what is
waiting pays vanilla.

**Nor, on any build measured so far, does anything else.** The game has code to bank a
single percentage figure for a finished mission in your save, and part of that
figure is a bonus for sending more workers than the mission needed, which is the
one thing `AnyOperatorCount` could have inflated, since it tells the game every
mission needs only one worker. Both of the switches that code sits behind are
turned **off** on build 26361232, read in game on 2026-11-01, as they were on
25246367 and 25116796 before it: no mission defers a payout, and the figure
ignores the worker count entirely. The plugin
finds both switches in the game by their code, whatever the build, reads them at
startup and says so in `DesertTooling.log`, on the `[banking]` line (the
addresses are build 26361232's, the same as on 26356046):

```text
[banking] deferred-reward gate (0x7150E18) = 0 - no mission defers its payout, so nothing is
stored in the save; surplus-worker gate (0x7128CA8) = 0 - the stored percent ignores the worker
count, so the minimum operator count never enters it; on this launch nothing is ever banked: no
completed mission defers a reward, so AnyOperatorCount cannot outlive the plugin. Both are
read-only and nothing here acts on either.
```

If a game update moves the code they are found by, the line says `not found by
signature on this build` in place of the address, and the warning below stands as
it is. On 26361232 both gates resolved at the addresses above and both read `0`.

**Check that line after a game update**, because these are the kind of switch an
update can flip, and it is the only thing that would tell you.

So nothing here reaches your save today. This section exists because the mechanism
exists: if a future build turned those switches on, finishing a mission with
`AnyOperatorCount=1` would bank a bigger figure than you earned, on the 142 of the
game's 936 missions that can repeat, and it would pay out later even with the
plugin gone. It would be bounded to the missions you finished with the setting on,
it would clear itself as those rewards landed, and it would not be a corrupted
save. `Speed` and `NoSkillRequirement` are nowhere in that figure and leave
nothing behind at all.

### Key names

`F1`..`F24`, `A`..`Z`, `0`..`9`, `NUM0`..`NUM9`, `NUMPLUS` `NUMMINUS` `NUMMULT`
`NUMDIV` `NUMDOT`, `HOME` `END` `INSERT` `DELETE` `PAGEUP` `PAGEDOWN` `TAB`
`SPACE` `BACKSPACE` `SCROLLLOCK` `PAUSE`, `MOUSE3` `MOUSE4` `MOUSE5`. Avoid keys
the game uses.

### If the ini is missing

The plugin writes one itself on the next launch, with every key of every section
at its default, instead of leaving nothing to edit. An existing file is never
read, rewritten or replaced (the create is the OS's own atomic "only if
absent"), so this can never touch your settings. The generated file is bare,
unlike the shipped template's comments explaining each key, but every key in the
tables above is in it, including the ones the menu draws on its Debug tab.

## How a changed multiplier becomes live

Short version: the game still reads its whole gather table once, about nine
seconds after launch, and the load-time hook still only runs inside that one
read, but the plugin also rewrites the records the game already parsed, right
after the ini changes, so a change reaches the game on the next gather rather
than the next launch. Everything below is what that involves and what it looks
like in the log.

**What the game does at launch.** The gathering rules live in a data table
called `gimmickinfo`: 14,210 records on build 26361232 (13,941 on 25477059), 294 of which this plugin multiplies: the gather nodes as the game's own data
marks them, plus the water well, the wild rock honey, three stalactites and the
three placed money records. Roughly
nine seconds after the process starts, before the main menu is up, the game runs
a preload pass that reads every record in order, parses each one into an object
in memory, stores the object's pointer in a slot table, and then closes the
file. From then on, whenever any part of the game needs a record, it looks the
pointer up in that slot table; the loader is only called again for a slot that
is still empty, and after the preload pass none of them are. Loading a save,
dying, fast travelling, running out of the area or backing out to the main menu
do not empty the slots; the parsed records live as long as the process does.
That is why editing the raw bytes as they load is not enough on its own: past
that first minute, there is nothing left for the load-time hook to intercept.

**What the plugin remembers.** Every time the hook sees a gather record (even
at `Enabled=0` and at `1x`, because vanilla is the only fixed point a later
change can be computed from), it reads the vanilla minimum, maximum and item id
of each of the record's output blocks straight out of the raw bytes, along with
the record's index, and keeps them in a small fixed table. Nothing here touches
a parsed object; it is all taken from the same bytes the load-time hook was
about to edit anyway.

**What happens when the ini changes.** `DesertTooling.ini` is polled about once
a second. Right after the `[ini] reloaded:` line, the plugin walks every record
it remembered, finds the already-parsed object for it through the game's own
record manager, checks that the object's key still matches, that its output list
still has the same number of entries, and that each block's item id still
matches what was on disk, then writes vanilla times the current multiplier into
that block's minimum and maximum. A block already carrying the wanted numbers is
left alone. If any of those checks fails for a record or a block, that record or
block is skipped rather than patched, and the pass still writes everything it
safely can.

The change shows on the very next gather: no restart, no save reload needed. A
record the game has not loaded yet still gets the new numbers the ordinary way,
through the load-time hook, whenever it does load.

**What the log shows.** Right after each `[ini] reloaded: ...` line:

```text
[harvest] [live] re-applied Foraging=10 Logging=1 Mining=1 Ore=1 Money=1: 83 records rewritten, 196 unchanged, 0 skipped; 646 scalars written
```

or, with `DryRun=1`, the same line as `[dry] would re-apply ...`. At `Enabled=0`
the multiplier part instead reads `Foraging=1 Logging=1 Mining=1 Ore=1 Money=1
(Enabled=0)`: `Enabled=0` means vanilla, not "leave whatever is already there,"
so the pass writes every record's minimum and maximum back to their disk values.
With `Debug=1`, each rewritten record also gets its own line:

```text
[harvest] [live] mine_bluestone key=17030001 Mining x5 blocks=2 wrote 4: 1->5/3->15, 2->10/8->40
```

Nothing is logged at all if the ini changes before the table has finished its
first load; the load path applies the current multipliers as it goes, so there
is nothing left for the re-apply pass to do yet.

**What a WARN means.** If any record or block failed one of the checks above,
the summary line is followed by one naming the reasons, for example:

```text
[harvest] [live] WARN 12 of 279 records and 3 blocks skipped (not loaded 10, key mismatch 2, null block 3); the parsed record layout may have moved in this game build
```

or, if the record manager itself could not be read at all:

```text
[harvest] [live] WARN the record manager is unreadable; nothing was re-applied
```

Either is the first thing a game update would trip, ahead of the load-time
patching breaking, because the re-apply pass walks live game pointers that the
load path never has to touch.

**What is different about this path.** Everywhere else, the hooks only ever
write into bytes the game has not parsed yet, a stream buffer about to be
handed to the deserialiser. This is the one place the plugin writes into an
object the game has already built and is actively using, reached by walking the
record manager rather than being handed a pointer. That write is guarded the
same way as everything else (a key check, an item-id check, a list-count check,
and a `WriteProcessMemory` that never assumes an address is still valid), but it
is worth being clear about what it writes: always the *vanilla* number times the
current multiplier, taken from what was remembered at load time, never the value
already sitting in the block scaled again. A record re-applied five times at
five different multipliers ends up exactly where a single load at the last
multiplier would have left it.

`Bugs`, `Fish` and `Critters` need none of that machinery: the multiplier is read at the
moment the creature is caught, so a change there takes effect on the **next
catch** with nothing to re-apply. Each multiplied catch logs one line,
`[harvest] [catch] fish class=23 -> x3`; a creature the game's species records do
not list and whose class the plugin has never seen caught is left vanilla and reported once per class per session.
`Enabled=0` and `DryRun=1` both mean "grant one, as the game would".

## Files in the game folder

Everything the plugin touches lives in `bin64` next to the exe:

| file | what it is | safe to delete? |
|---|---|---|
| `DesertTooling.asi` | the plugin | yes, that uninstalls it |
| `DesertTooling.ini` | your settings, read at game start and re-read while it runs | yes, the plugin recreates a bare one at every key's default on the next launch, which means vanilla `1x` for every multiplier (the seven yield multipliers and `Hunting` under `[Harvest]`, `Speed` and `Rewards` under `[Dispatch]`), so deleting it turns them off |
| `DesertTooling.log` | append-only log of what the plugin did; grows every session | yes, any time |
| `DesertTooling.yields` | a small cache of "this node gave this item, this many", learned while you play; refines the stacking rule at a full bag | yes, it relearns itself |
| `DesertTooling.ini.tmp` | transient: the menu writes the new ini here and renames it over `DesertTooling.ini`, so it is normally gone a moment later | yes |
| `DesertTooling.gimmickinfo.bin` | the game's vanilla `gimmickinfo` table body, about 22 MB, a diagnostic copy; written once per session only with `[Harvest] DumpTable=1` and `DryRun=1` | yes |

The log is the first thing to look at when something seems off. Every line
carries the subsystem that wrote it, so one file still separates cleanly:

```text
[    0.001] [tid  4812] [tooling] Desert Tooling 0.10.0 loaded, pid 21344 (harvest 0.6.0, overlay 0.3.0, dispatch 0.3.0)
[    0.014] [tid  6120] [harvest] [hook] record loader +0x38BA40 -> stub 0x1F2C0000
[    0.312] [tid  6120] [harvest] [sig] 9 found, 0 missing, 1 actor-manager vtable(s), 41 ms
[    1.884] [tid  7744] [overlay] [hudhook] DX12 hooks applied in 96 ms; press the menu key in game
```

A healthy start shows, from `[harvest]`, a `[gimmick] record loader at +0x...`
line, a `[gimmick] stream layout: buffer +0x.. size +0x.. cursor +0x..` line (where
the plugin found the table data the game loads from), a `[hook]` line and a `[catch]` line, then nine `[sig]` lines ending in
`= +0x...`, the `[hook]` lines for `area_sweep` and `enqueue`, the
`[hunting] loot_grant` line and the `[event]` descriptor lines, then one line
per gather record as the game loads it; and from `[overlay]`, the
`[hudhook] DX12 hooks applied` line. If a signature stops matching after a game
update, the log says `NOT FOUND` and that part stays idle; nothing dangerous
happens. The same goes for the yields: `[gimmick] stream layout unknown on this
build` means gathering stays vanilla for that session, and nothing is dumped.

The `[gimmick]`, `[live]` and `[dry]` lines record what was **written** to the
records, not what you received. To measure actual yields, set `LogReceived=1`
under `[Harvest]` and
read the `[recv] item <key> x<count>` lines as you gather. That is every item the
game hands you, and it is how the numbers in this README were measured. The one
exception is money: picking up a coin prop writes no `[recv]` line at all, so
measure `Money` with the bag line an F11 survey prints (`Money_Copper key=1
x<count>`), once before the pickup and once after.

## How it survives game updates

Nothing here is a hard-coded address. The auto-loot half resolves ten byte
signatures and finds the actor manager through its RTTI vtable. The yield half
scans the
running image for the accessor that names the `gimmickinfo` table and follows it
to the record-loading function, then refuses to patch unless the twelve bytes it
is about to overwrite are exactly the prologue it expects; inside each record,
the yield numbers are found by walking the resource-output lists and matching
the fixed 68-byte block layout that carries them. The Overlay looks up no game
address at all.

That covers records and functions moving, which is what usually happens on a
patch. If the game changes a function's shape or a block's layout, the plugin
will fail to resolve, log why, and leave that part of itself idle. It never
guesses: a refusal is a subsystem that does nothing, and a wrong patch would be
a crash.

`Bugs`, `Fish` and `Critters` are the fragile part, and deliberately so. There is no record
to edit for a caught creature, so that multiplier is a patch on the game's code:
the plugin scans for the 21-byte instruction sequence that hands the catch
handler its count, and refuses to patch unless the thirteen bytes it is about to
overwrite are exactly the two instructions it expects. A game update that
rewrites that function disables **only** those three keys, with a `[catch]` line
saying which check failed: every other multiplier is untouched by it, and the
game is untouched by us.

## Building (developers)

Built on NixOS under WSL and cross-compiled to Windows. Desert Tooling is one
crate of a Cargo workspace (see the repo root `README.md`); run cargo from the
workspace root, not from this directory. The `flake.nix` dev shell provides the
Rust toolchain with the `x86_64-pc-windows-gnu` target, the mingw cross-linker,
and the analysis tools (python, binutils, file). Re-targeting to a new game
build needs nothing outside the repo: the tools in `crates/tools/` do it offline.

```bash
nix develop
cargo build --release
```

Output: `target/x86_64-pc-windows-gnu/release/desert_tooling.dll`, the only
cdylib in the workspace, installed as `DesertTooling.asi`. If flakes are not
enabled in your Nix config, add
`--extra-experimental-features 'nix-command flakes'` to the `nix` command. Keep
the checkout on the Linux filesystem; building under `/mnt/c` is slow.

Native tests for the platform-independent modules:

```bash
cargo test --target x86_64-unknown-linux-gnu
```

Ignored tests that scan the real game exe (needs the Steam install mounted):

```bash
just test-game
```

### How it works, briefly

`desert-tooling` is the only `cdylib` and the only `DllMain`. It gates on the
host exe (the ASI loader also injects into `crashpad_handler.exe`), hands off to
a thread immediately because the loader lock is held, opens `DesertTooling.log`,
seeds `DesertTooling.ini` from all three subsystems' schemas if it is missing,
migrates a pre-0.10 `[Looter]`/`[Gatherer]` pair into `[Harvest]`, refuses to
install anything if a pre-merge `.asi` is still loaded, and then starts one
thread per subsystem, harvest first. That thread installs the yield hooks before
anything else, because the record-loader hook has to go in before the game
reads the gather table and must not queue behind the auto-loot's signature scan
or its 20-second boot grace.

`desert-harvest` has two halves. The looter resolves ten byte signatures,
hooks four functions with inline trampolines (a per-frame job, for a callback on
a game thread; the event queue's push, for observation; the loot grant, for
`Hunting`; and the side item log's line builder, which the item-log filter can
skip for a line it hides), finds the actor
manager through its RTTI vtable, and builds pickups exactly as the game's own
event builder builds them, queued from inside the hook.

The gatherer hooks the gimmickinfo record loader, which is entered on
whichever game thread loads a record, before the deserializer has read a byte of
it, and rewrites the yield scalars in the raw bytes; a second hook replaces the
hard-coded catch count. `desert-overlay` draws the menu with Dear ImGui inside
the game's DirectX 12 frame and writes the ini. All of them read game memory
through `ReadProcessMemory` on their own process, so a vanished page can never
crash the game.

The signature scanning, the trampolines, the guarded reads, the ini and schema
handling and the logger all live in `desert-core`.

### Analysis tooling

After a game update, `just retarget report` says which signatures and
constants moved, and `just retarget triage`, `census`, `calls`, `callers` and
`prologue` re-derive them; `just sigscan` checks that each signature hits
exactly once. `dis` disassembles an address range and `xrefs` finds
references. All are Rust binaries in `crates/tools/`, built by `just tools`, and none
needs Ghidra.

## Credits and licence

MIT, like everything in this repository. The menu is drawn with
[hudhook](https://github.com/veeenu/hudhook) (MIT) and
[Dear ImGui](https://github.com/ocornut/imgui) through
[imgui-rs](https://github.com/imgui-rs/imgui-rs) (both MIT).
