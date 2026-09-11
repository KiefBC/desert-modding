# Desert Tooling

**Version 0.4.0**, for Crimson Desert Enhanced, Steam build **25246367**.
[Changelog](CHANGELOG.md) · [versioning](../VERSIONING.md).

One `.asi` plugin with four subsystems: **auto-loot**, **gathering yield
multipliers**, **dispatch missions** and an **in-game settings menu**. Written in
Rust, installed by dropping two files into `bin64`.

| subsystem | what it does | ini section | log tag |
|---|---|---|---|
| Looter | gathers plants, ore, stone, wood, insects and fish around you | `[Looter]` | `[looter]` |
| Gatherer | multiplies what a gathering node or a caught creature gives | `[Gatherer]` | `[gatherer]` |
| Dispatch | shorter dispatch missions, bigger mission rewards, no skill or headcount gate | `[Dispatch]` | `[dispatch]` |
| Overlay | the settings menu, `Insert` | `[Overlay]` | `[overlay]` |

## Upgrading from Desert Looter, Desert Gatherer and Desert Overlay

Those three plugins are this one. **Delete them first:**

1. Remove `DesertLooter.asi`, `DesertGatherer.asi` and `DesertOverlay.asi` from
   `bin64`, along with their `.ini`, `.log` and `.overlay.ini` files.
2. Extract this release into `bin64`.
3. Re-enter your settings. Nothing is migrated — the old ini files are not
   read, and `DesertTooling.ini` starts at every key's default.

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

The release archive holds five files at its root — `DesertTooling.asi`,
`DesertTooling.ini`, `README.md`, `CHANGELOG.md` and `LICENSE` — so it suits
either way of installing it.

### By hand

Requirements: the game, and Ultimate ASI Loader present in `bin64` as
`winmm.dll` (it loads every `.asi` file beside it).

Extract the archive straight into `<game>\bin64\`, or copy two files there:

- `DesertTooling.asi`: the plugin. Rename the built `desert_tooling.dll` to this.
- `DesertTooling.ini`: the settings. Optional — if it is missing, the plugin
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
[Conflicts with DMM](#conflicts-with-dmm-important) first — DMM's own gathering
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
name, version, `KiefBC`, MIT). It is **not** digitally signed — a code-signing
certificate costs money — so check the `SHA256SUMS` file from the release
instead. And the source is public: all of it is in this repository.

## The menu

Press **Insert** and a window appears over the game (DirectX 12, Dear ImGui)
with every setting in it as a checkbox, slider or key picker, in three
collapsible sections. Change one and it takes effect within about a second,
without leaving the game and without a restart.

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

Presets sit above the Looter section: `Everything`, `Plants only`, `Wood only`,
`Rock and ore only`. One click sets the four gather families and ground items
and leaves everything else alone.

The sliders and number fields stop at the ranges the parsers accept, so the menu
cannot produce a value its own subsystem would refuse. A number typed into a
field is written when you press Enter or click away, not while you are still
typing. If the file cannot be read or written, the reason appears as a red line
under that section and in `DesertTooling.log`; nothing is lost and the game is
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
| F11 | write a survey of everything nearby to the log (read-only) |
| F7 | debug: record every event the game queues until pressed again |

Every gathering keypress beeps once. All five keys can be changed in the ini or
from the menu.

## What the Looter does

- **Gathers nodes** within a few metres: flowers, berries, fruit, ore veins and
  rocks, and the firewood you cut from felled logs. Veins and rocks are mined
  outright, without a swing.
- **Picks up plain ground items** such as ore chunks. Merchant goods, quest
  items and props are never touched.
- **Catches insects and fish** within the same range, using the game's own
  "catch" event rather than a pickup. Fish within reach at the water's edge go
  the same way, with the very same event as an insect. The steal check applies
  to both, so a creature the game counts as someone else's is left alone.
  `GatherBugs=0` and `GatherFish=0` turn them off separately.
- **Leaves alone what it does not recognise.** Insects and fish are told apart
  from birds in flight and other wildlife by a class byte on the creature
  itself, and only the classes that have actually been caught by hand are ever
  targeted. Anything else is passed over and reported in the log once per class,
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
- **Leaves alone**: standing trees, log chunks, animals and NPCs, furniture and,
  by default, dropped weapons and armour.

Nothing is simulated and no input is faked: the plugin sends the game the same
pickup event it sends when you press E.

## What the Gatherer does

Every gathering node in the game carries a minimum and a maximum quantity for
each item it can produce. Desert Tooling multiplies both, for the 275 records
the game counts as genuine gather nodes, split into four independent families:

| family | what it covers | records |
|---|---|---|
| Foraging | plants, fruit, berries, mushrooms, crops | 82 |
| Logging | firewood cut from felled trees (`firewood_*`) | 141 |
| Mining | the `collect_mine` family: rocks, veins, cave variants, breakable stalactites | 36 |
| Ore Nodes | the `collect_ore` family: `ore_*` deposits, sulfur stone, collectible stalactites | 16 |

Each family has its own multiplier, 1 to 100. Mining and Ore Nodes look alike in
game but are separate families in the game's data; set both if you want all
mining-style gathering raised together. Nothing is patched on disk: the numbers
are multiplied in memory as the game loads them.

Two more multipliers, **Bugs** and **Fish**, cover the creatures you catch by
hand. Those are not gathering nodes and there is no record anywhere saying what
one is worth: the game grants exactly one, from a constant in its own code. So
that pair works differently from the four families — it is a small patch on that
constant, applied as the creature is caught — and it is the one part of this
plugin a game update can switch off on its own. If that happens the log says so
once at startup (`[catch] signature not found; catch multipliers off`) and
everything else here goes on working.

Because that patch is in the game's own grant code rather than in anything the
Looter sends, it multiplies the creatures **you** catch by hand exactly as it
multiplies the ones auto mode catches for you.

One creature pays out twice: a Firefly Colony gives you colonies *and*
fireflies, and because the game rolls a handful of fireflies for each colony you
take, the fireflies come out at roughly double your `Bugs` setting and a
different number every time — at `Bugs=10`, ten colonies and somewhere around
twenty fireflies. That is the game's own drop rule, not a bug in the patch.

It does **not** touch enemy loot, chests, rod-and-line fishing, skinning, quests,
Abyss objects, artifacts, gates or fast travel.

### What to expect in game

The multiplier scales the numbers in the record, not the amount you happen to
get on one pick. Three things surprise people:

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

One more thing that looks odd: pressing E can hand the total over as several
separate "x1" pickups a second or so apart. That is the game's own delivery
animation. The total across them is the multiplied amount.

## Conflicts with DMM (important)

DMM's **"The Desert Gatherer" module pack** and DMM's **built-in gathering
multiplier preset** change the same minimum/maximum quantities, on disk.

If either is mounted while this `.asi` is installed, the two stack: a mounted
DMM 5X plus `Mining=2` here gives 10x. **Unmount both in DMM before using this
plugin.** The `desert-gatherer-dmm/` directory in this repository is that pack;
this plugin is its replacement, not its companion.

## Settings (`DesertTooling.ini`)

One file, four sections. A key belongs to the `[Section]` header above it:
`Enabled` and `Debug` exist under all four and mean different things in each,
and `DryRun` exists under `[Gatherer]` and `[Dispatch]`.

### `[Looter]`

| key | default | meaning |
|---|---|---|
| `Enabled` | 1 | 0 = idle: no gathering, no hotkeys, nothing logged. Live: set it to 1 while the game runs and it comes back |
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
| `GatherBugs` | 1 | 0 = do not catch insects (they are a separate game event, not a gather family) |
| `GatherFish` | 1 | 0 = do not catch fish (same event as insects, at the water's edge) |
| `BagTab` | 1 | which inventory tab is the bag for the full check |
| `StackLimit` | 999 | at a full bag, do not grow a stack past this |
| `ScanRange` | 40 | radius of the F11 survey |
| `Debug` | 0 | 1 = very verbose survey (first F11 dumps hundreds of lines) |
| `LogReceived` | 0 | 1 = log every item the game hands you as `[recv] item <key> x<count>`, plugin-caused or not (capped at 500 a session); for measuring yields |
| `KeyToggle`, `KeyScan`, `KeyGather`, `KeyRecord` | F10, F11, F9, F7 | see the key names below |

### `[Gatherer]`

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

### `[Overlay]`

Read **once** at game start; a change to this section needs a restart. (It is
only the menu's own settings that are not live — everything it puts on screen
is.)

| key | default | meaning |
|---|---|---|
| `Enabled` | `1` | `0` = install no graphics hook at all, so the game renders exactly as it would without the `.asi` and there is no menu. The other two subsystems are unaffected |
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

### `[Dispatch]`

Dispatch missions ("faction operations") are the jobs you send workers away on
from the faction map. This section makes them finish sooner, pay more, and stop
turning you away.

| key | default | meaning |
|---|---|---|
| `Enabled` | `1` | Turned off **while the game runs**, every mission and reward it changed goes back to vanilla. Turned off **before you launch**, the pass never runs and nothing in the game is read at all — and it stays that way for the session: turning it back on, here or in the menu, does nothing until the next launch |
| `Speed` | `1` | divide every mission's duration by this, 1..100. Vanilla runs 2h to 96h; `4` turns a 16h mission into a 4h one. Nothing ever goes below 6 minutes. Reaches missions already under way |
| `NoSkillRequirement` | `0` | `1` = clear the skill a mission demands of an assigned worker, on the 147 missions that demand one. Checked when a mission starts |
| `AnyOperatorCount` | `0` | `1` = let every mission start with a single worker. 693 of the game's 936 missions already ask for one, so this changes the other 243 — the ones wanting 2, 3, 5, 8, 10 or another number. Checked when a mission starts |
| `Rewards` | `1` | multiply how much of each item a mission pays, 1..100. Both ends of every reward range are scaled, so 2-3 becomes 6-9 at x3. Reaches rewards already waiting to be paid |
| `LogRecords` | `1` | `1` = one line per mission in the log |
| `MaxLines` | `4000` | ceiling on the lines one pass writes, 0..20000. The summaries are never capped, and hitting it never stops a change being made |
| `DryRun` | `0` | `1` = log every change that would be made and write nothing |
| `Debug` | `0` | `1` = also log the nodes with no missions, every change one line at a time, and why anything was stepped over |
| `DumpRaw` | `0` | `1` = log each mission's raw bytes as hex, for chasing an offset after a game update |
| `DumpRewards` | `1` | `1` = also dump the reward rows the missions name. A diagnostic only: `Rewards` reads and edits those rows either way |

A change takes effect within about a second. Only the 219 reward rows dispatch
missions actually name are touched — never the game's wider drop table — so this
does not become a loot multiplier for chests and carcasses. No game code is
patched and no hook is installed: the plugin edits five fields of the tables the
game has already read (four settings, five fields — `Rewards` writes both ends of
a min/max pair), on its own thread, and remembers what each one said first so it
can put it back.

**Two of the settings reach missions that are already out, and two do not.**
`Speed` shortens a mission already under way — the game checks a mission's
progress against the duration in the table every time it ticks — and `Rewards`
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

**Nor, on this game build, does anything else.** The game has code to bank a
single percentage figure for a finished mission in your save, and part of that
figure is a bonus for sending more workers than the mission needed — which is the
one thing `AnyOperatorCount` could have inflated, since it tells the game every
mission needs only one worker. Both of the switches that code sits behind are
turned **off** on build 25246367, as they were on 25116796 before it: no mission
defers a payout, and the figure ignores the worker count entirely. The plugin
reads both at startup and says so in `DesertTooling.log`, on the `[banking]`
line:

```text
[banking] deferred-reward gate (0x6BC6AA8) = 0 - no mission defers its payout, so nothing is
stored in the save; surplus-worker gate (0x6BA07C8) = 0 - the stored percent ignores the worker
count, so entry+0xC4 never enters it; on this launch nothing is ever banked: no completed mission
defers a reward, so AnyOperatorCount cannot outlive the plugin.
```

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
read, rewritten or replaced — the create is the OS's own atomic "only if
absent" — so this can never touch your settings. The generated file is bare,
unlike the shipped template's comments explaining each key, but every key in the
tables above is in it, including the three the menu groups under
`Diagnostics:`.

## How a changed multiplier becomes live

Short version: the game still reads its whole gather table once, about nine
seconds after launch, and the load-time hook still only runs inside that one
read — but the plugin also rewrites the records the game already parsed, right
after the ini changes, so a change reaches the game on the next gather rather
than the next launch. Everything below is what that involves and what it looks
like in the log.

**What the game does at launch.** The gathering rules live in a data table
called `gimmickinfo`: 13,906 records, 275 of which are gather nodes. Roughly
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

**What the plugin remembers.** Every time the hook sees a gather record — even
at `Enabled=0` and at `1x`, because vanilla is the only fixed point a later
change can be computed from — it reads the vanilla minimum, maximum and item id
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

The change shows on the very next gather — no restart, no save reload needed. A
record the game has not loaded yet still gets the new numbers the ordinary way,
through the load-time hook, whenever it does load.

**What the log shows.** Right after each `[ini] reloaded: ...` line:

```text
[gatherer] [live] re-applied Foraging=10 Logging=1 Mining=1 Ore=1: 82 records rewritten, 193 unchanged, 0 skipped; 644 scalars written
```

or, with `DryRun=1`, the same line as `[dry] would re-apply ...`. At `Enabled=0`
the multiplier part instead reads `Foraging=1 Logging=1 Mining=1 Ore=1
(Enabled=0)`: `Enabled=0` means vanilla, not "leave whatever is already there,"
so the pass writes every record's minimum and maximum back to their disk values.
With `Debug=1`, each rewritten record also gets its own line:

```text
[gatherer] [live] mine_bluestone key=17030001 Mining x5 blocks=2 wrote 4: 1->5/3->15, 2->10/8->40
```

Nothing is logged at all if the ini changes before the table has finished its
first load; the load path applies the current multipliers as it goes, so there
is nothing left for the re-apply pass to do yet.

**What a WARN means.** If any record or block failed one of the checks above,
the summary line is followed by one naming the reasons, for example:

```text
[gatherer] [live] WARN 12 of 275 records and 3 blocks skipped (not loaded 10, key mismatch 2, null block 3); the parsed record layout may have moved in this game build
```

or, if the record manager itself could not be read at all:

```text
[gatherer] [live] WARN the record manager is unreadable; nothing was re-applied
```

Either is the first thing a game update would trip, ahead of the load-time
patching breaking, because the re-apply pass walks live game pointers that the
load path never has to touch.

**What is different about this path.** Everywhere else, the hooks only ever
write into bytes the game has not parsed yet — a stream buffer about to be
handed to the deserialiser. This is the one place the plugin writes into an
object the game has already built and is actively using, reached by walking the
record manager rather than being handed a pointer. That write is guarded the
same way as everything else — a key check, an item-id check, a list-count check,
and a `WriteProcessMemory` that never assumes an address is still valid — but it
is worth being clear about what it writes: always the *vanilla* number times the
current multiplier, taken from what was remembered at load time, never the value
already sitting in the block scaled again. A record re-applied five times at
five different multipliers ends up exactly where a single load at the last
multiplier would have left it.

`Bugs` and `Fish` need none of that machinery: the multiplier is read at the
moment the creature is caught, so a change there takes effect on the **next
catch** with nothing to re-apply. Each multiplied catch logs one line,
`[gatherer] [catch] fish class=23 -> x3`; a creature whose class the plugin has
never seen caught is left vanilla and reported once per class per session.
`Enabled=0` and `DryRun=1` both mean "grant one, as the game would".

## Files in the game folder

Everything the plugin touches lives in `bin64` next to the exe:

| file | what it is | safe to delete? |
|---|---|---|
| `DesertTooling.asi` | the plugin | yes, that uninstalls it |
| `DesertTooling.ini` | your settings, read at game start and re-read while it runs | yes, the plugin recreates a bare one at every key's default on the next launch — which means vanilla `1x` for all six multipliers, so deleting it turns them off |
| `DesertTooling.log` | append-only log of what the plugin did; grows every session | yes, any time |
| `DesertTooling.yields` | a small cache of "this node gave this item, this many", learned while you play; refines the stacking rule at a full bag | yes, it relearns itself |

The log is the first thing to look at when something seems off. Every line
carries the subsystem that wrote it, so one file still separates cleanly:

```text
[    0.001] [tid  4812] [tooling] Desert Tooling 0.3.0 loaded, pid 21344 (looter 0.2.0, gatherer 0.2.0, overlay 0.2.0)
[    0.014] [tid  6120] [gatherer] [hook] record loader +0x3856B0 -> stub 0x1F2C0000
[    0.312] [tid  5008] [looter] [sig] 8 found, 0 missing, 1 actor-manager vtable(s), 41 ms
[    1.884] [tid  7744] [overlay] [hudhook] DX12 hooks applied in 96 ms; press the menu key in game
```

A healthy start shows, from `[looter]`, eight `[sig]` lines ending in `= +0x...`,
two `[hook]` lines and the `[event]` descriptor lines; from `[gatherer]`, a
`[gimmick] record loader at +0x...` line, a `[hook]` line and a `[catch]` line,
then one line per gather record as the game loads it; and from `[overlay]`, the
`[hudhook] DX12 hooks applied` line. If a signature stops matching after a game
update, the log says `NOT FOUND` and that part stays idle; nothing dangerous
happens.

`[gatherer]` lines record what was **written** to the records, not what you
received. To measure actual yields, set `LogReceived=1` under `[Looter]` and
read the `[recv] item <key> x<count>` lines as you gather. That is every item the
game hands you, and it is how the numbers in this README were measured.

## How it survives game updates

Nothing here is a hard-coded address. The Looter resolves eight byte signatures
and finds the actor manager through its RTTI vtable. The Gatherer scans the
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

`Bugs` and `Fish` are the fragile part, and deliberately so. There is no record
to edit for a caught creature, so that multiplier is a patch on the game's code:
the plugin scans for the 21-byte instruction sequence that hands the catch
handler its count, and refuses to patch unless the thirteen bytes it is about to
overwrite are exactly the two instructions it expects. A game update that
rewrites that function disables **only** those two keys, with a `[catch]` line
saying which check failed — every other multiplier is untouched by it, and the
game is untouched by us.

## Building (developers)

Built on NixOS under WSL and cross-compiled to Windows. Desert Tooling is one
crate of a Cargo workspace (see the repo root `README.md`); run cargo from the
workspace root, not from this directory. The `flake.nix` dev shell provides the
Rust toolchain with the `x86_64-pc-windows-gnu` target, the mingw cross-linker,
and the analysis tools (python, binutils, file). Ghidra is not in the dev shell:
it runs on the Windows side and is driven over its MCP bridge.

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
refuses to install anything if a pre-merge `.asi` is still loaded, and then
starts one thread per subsystem — the gatherer first, because its hook has to go
in before the game reads the gather table and must not queue behind the looter's
20-second boot grace.

`desert-looter` resolves eight byte signatures, hooks two functions with inline
trampolines (a per-frame job, for a callback on a game thread, and the event
queue's push, for observation), finds the actor manager through its RTTI vtable,
and builds pickups exactly as the game's own event builder builds them, queued
from inside the hook.

`desert-gatherer` hooks the gimmickinfo record loader, which is entered on
whichever game thread loads a record, before the deserializer has read a byte of
it, and rewrites the yield scalars in the raw bytes; a second hook replaces the
hard-coded catch count. `desert-overlay` draws the menu with Dear ImGui inside
the game's DirectX 12 frame and writes the ini. All four read game memory
through `ReadProcessMemory` on their own process, so a vanished page can never
crash the game.

The signature scanning, the trampolines, the guarded reads, the ini and schema
handling and the logger all live in `desert-core`.

### Analysis tooling

`just sigscan` checks the signatures against the exe after a game update; each
must hit exactly once. `tools/dis.sh` disassembles an address range and
`tools/xrefs.py` finds references. Decompilation goes through the Windows Ghidra
over its MCP bridge.

## Credits and licence

MIT, like everything in this repository. The menu is drawn with
[hudhook](https://github.com/veeenu/hudhook) (MIT) and
[Dear ImGui](https://github.com/ocornut/imgui) through
[imgui-rs](https://github.com/imgui-rs/imgui-rs) (both MIT).
