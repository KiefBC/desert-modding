# Reading the log

Source: `crates/desert-core/src/log.rs` (`line`, `emit`), the `LOG_TAG` constants of each crate, the log calls in `crates/desert-harvest/src/entry.rs`, `crates/desert-harvest/src/looter/{game,entry,hunting}.rs`, `crates/desert-harvest/src/gatherer/{entry,hook,catch}.rs`, `crates/desert-overlay/src/lib.rs`, `crates/desert-dispatch/src/node.rs`; file names from `crates/desert-tooling/src/lib.rs`, `crates/desert-overlay/src/rewrite.rs`, `crates/desert-harvest/src/looter/entry.rs`, `crates/desert-harvest/src/gatherer/dump.rs`.

`DesertTooling.log` sits in `bin64` beside the game exe. Every subsystem writes into it, and it is append-only: it grows every session and is safe to delete at any time. Developers can follow it with `just log`.

## Line format

```text
[+seconds] [tid] [tag] message
```

The seconds are since the plugin started, to three decimals, and the thread id is the Windows thread that asked to log. A line that waited 2 ms or more for the log's lock gets ` [log-lock wait N ms]` appended. An example from the shape test in `log.rs`:

```text
[    1.500] [tid    42] [harvest] hello
```

## Tags

| tag | written by |
|---|---|
| `[tooling]` | the plugin's own startup: the version line, ini seeding, the `[Looter]`/`[Gatherer]` migration, the stale-`.asi` guard |
| `[core]` | `desert-core`, the shared plumbing |
| `[harvest]` | Harvest: auto-loot and the yield multipliers |
| `[overlay]` | the Overlay (menu) |
| `[dispatch]` | Dispatch |
| `[buffs]` | Dispatch's read-only buff/stat census (`DumpBuffs=1`) |

Inside a subsystem's lines a second bracket names the part that wrote it, for example `[harvest] [sig]`, `[harvest] [gimmick]` or `[dispatch] [banking]`. To see one subsystem, grep for its tag: `just log | grep '\[harvest\]'`.

## A healthy start

The first line names the plugin's version and each subsystem's (`Desert Tooling <ver> loaded, pid <n> (harvest <ver>, overlay <ver>, dispatch <ver>)`). After it:

- **Harvest**, the yield hooks first: `[gimmick] record loader at +0x...`, then `[gimmick] stream layout: buffer +0x20 size +0x28 cursor +0x2c (parsed from the accessor's fill sequence)`, then `[hook] record loader +0x... -> stub ...`, and once the table starts loading `[gimmick] record header: name first, key after it (26361232 shape)` (on builds up to 25477059, `key first (25477059 shape)`), then `[catch] hook at +0x... -> stub ...`. Then the auto-loot: nine `[sig] <name> = +0x...` lines, one per byte signature, then the summary `[sig] 9 found, 0 missing, <n> actor-manager vtable(s), <n> ms`; two `[hook]` lines, `area_sweep +0x... -> stub ...` and `enqueue +0x... -> stub ...`; one `[hunting] loot_grant +0x... -> stub ...` line; then `[event]` lines. When the game first loads a record, `[gimmick] first call: ...` prints the stream it was handed, ending with the layout in use, `(buffer +0x20 size +0x28 cursor +0x2c)`. One `[gimmick]` line per gather record follows as the game loads its table.
- **Overlay**: `[hudhook] DX12 hooks applied in <n> ms; press the menu key in game`.
- **Dispatch**: a `[banking]` line reporting the two gates the game's mission-banking code sits behind, each found by signature (an address, or `not found by signature on this build`), and what that means for this launch. Then the mission census, ending `[sum] verdict: the layout LOOKS RIGHT`, and with `DumpRewards=1` the reward census, ending `[rewards] verdict: the dropset layout LOOKS RIGHT` and `weight totals: N/M fully read rows match record+0x38`. Dispatch writes nothing until those verdicts pass on this launch: a `[dispatch] writes disabled (missions): <reason> ...` or `(reward rows)` line, once per reason, means the levers are reporting only, as under `DryRun=1`. Since 0.10.0 the mission lines print the always-1 field as `+0xAC` (was `+0xC0`) and the two reward slots as `+0xA0`/`+0xA2` (were `+0xA8`/`+0xAA`): the game build 26345091 moved them.

If the stream layout could not be read off the game's code, the `[gimmick] stream layout` line in the harvest start is instead `[gimmick] stream layout not parsed (<reason>); probing the known layouts on the first call`, and the first call then settles it: `[gimmick] stream layout: buffer +0x.. size +0x.. cursor +0x.. (probed on the first call: ...)` means the yields work as usual, and `[gimmick] stream layout unknown on this build: not patching anything and not dumping (<why each candidate failed>)` means gathering yields stay vanilla for the session and no table dump is written.

From its once-a-second loop the yield half prints `[stat] records seen N, stream unread N, headers unparsed N, gather records recognised N, patched N, scalars written N`. On a healthy build `stream unread` and `headers unparsed` are 0 and `recognised` is not; `patched` and `scalars written` are 0 when every multiplier is 1x or under `DryRun=1`. Two warnings are written at most once each, checked at every `[stat]` line and when `records seen` reaches 1000: `[gimmick] WARN no record header parsed in N records: the record format changed; yields stay vanilla` and `[gimmick] WARN no record stream read in N records: the stream layout changed; yields stay vanilla`. Either one means gathering yields are vanilla for this session; see [After a game update](game-update.md).

After a yield slider moves, the plugin rewrites the records the game has already loaded. Its first re-apply says which in-memory layout it found: `[live] parsed layout: key +0x10, list +0x128 -> +0x0, count +0x128 -> +0x8, entry 16, block item +0x68, min +0x20, max +0x28 (probed against 279 remembered records)` on build 26361232. Each move then logs `[live] re-applied Foraging=N Logging=N Mining=N Ore=N Money=N: N records rewritten, N unchanged, 0 skipped; N scalars written`. A record whose current values are not a whole multiple of vanilla is skipped with the reason `yield not a multiple of vanilla`. If no layout fits, `[live] WARN parsed layout not found (<step: candidates>); sliders have no effect until the next load, the hook still multiplies at load` is followed by the summary `[live] WARN the parsed record layout is unknown on this build (see the parsed layout line); nothing was re-applied`: the change you made reaches the game at the next launch instead. With `Debug=1` that search also prints the paths and hex windows a developer needs to read the new layout.

The catch hook (`Bugs`, `Fish`, `Critters`) logs `[catch] bug class=80 -> x3` or `[catch] fish class=23 -> x3` for a multiplied catch, and a creature it does not know as `[catch] type=06 class=.. not a known bug/fish class on this type; vanilla (logged once per class)`. With the 60-second summary, when anything moved, it prints `[catch] stat: grants seen N, type unreadable N, not a catchable type N, class unreadable N, unknown class N, not hand-catchable species N, multiplied N, left at 1x N`. With `Debug=1` every grant gets `[catch] debug: actor=0x.. type=06 class=.. species=.. chr=..` first. At startup `[species] table: N bug, N fish, N critter, N not catchable, N body, N friendly body, N unknown (...)` says how the characterinfo rows were classed.

Bodies (`GatherBodies`): the F11 survey lists a dead person as kind `Body` (hostile), `FriendlyBody` or `BodyEmpty` (already searched), with `dd=` and `owner=-1` (unowned), `owner=<hex>` or `owner=?` (unreadable). A search logs `[skin] body ... gave: ...` where an animal logs `[skin] carcass ...`. A body with an owner or quest trigger is refused with `[gather] body eid=.. has an owner/trigger (id=..); skipped`, once per body. Dropped weapons and armour are ordinary items and appear as `Item`, governed by `GatherItems` and `GatherGear`. Once the catch side's 500-line cap is reached, a single line says so.

Anything that fails to resolve says so on its own line (`NOT FOUND`, `NOT hooking`, `FAILED`) and that part stays idle. See [After a game update](game-update.md).

The `[gimmick]`, `[live]` and `[dry]` lines record what was **written** to the records, not what you received. To measure real yields, set `[Harvest] LogReceived=1` and read the `[recv] item <key> x<count>` lines. Coin props write no `[recv]` line; measure `Money` from the bag count an F11 survey prints, before and after the pickup.

## Files beside the plugin

| file | what it is |
|---|---|
| `DesertTooling.asi` | the plugin; deleting it uninstalls it |
| `DesertTooling.ini` | the settings; recreated at every default on the next launch if missing |
| `DesertTooling.ini.tmp` | transient: the menu writes the new ini here, then renames it over the real one |
| `DesertTooling.log` | this log |
| `DesertTooling.yields` | a small learned cache of "this node gave this item"; it relearns itself if deleted |
| `DesertTooling.gimmickinfo.bin` | the vanilla `gimmickinfo` table body, about 22 MB; written only with `[Harvest] DumpTable=1` and `DryRun=1`, once per session, for the offline tools |
