# Reading the log

Source: `desert-core/src/log.rs` (`line`, `emit`), the `LOG_TAG` constants of each crate, the log calls in `desert-looter/src/{game,lib,hunting}.rs`, `desert-gatherer/src/{entry,hook,catch}.rs`, `desert-overlay/src/lib.rs`, `desert-dispatch/src/node.rs`; file names from `desert-tooling/src/lib.rs`, `desert-overlay/src/rewrite.rs`, `desert-looter/src/lib.rs`, `desert-gatherer/src/dump.rs`.

`DesertTooling.log` sits in `bin64` beside the game exe. Every subsystem writes into it, and it is append-only: it grows every session and is safe to delete at any time. Developers can follow it with `just log`.

## Line format

```text
[+seconds] [tid] [tag] message
```

The seconds are since the plugin started, to three decimals, and the thread id is the Windows thread that asked to log. A line that waited 2 ms or more for the log's lock gets ` [log-lock wait N ms]` appended. An example from the shape test in `log.rs`:

```text
[    1.500] [tid    42] [looter] hello
```

## Tags

| tag | written by |
|---|---|
| `[tooling]` | the plugin's own startup: the version line, ini seeding, the stale-`.asi` guard |
| `[core]` | `desert-core`, the shared plumbing |
| `[looter]` | the Looter |
| `[gatherer]` | the Gatherer |
| `[overlay]` | the Overlay (menu) |
| `[dispatch]` | Dispatch |
| `[buffs]` | Dispatch's read-only buff/stat census (`DumpBuffs=1`) |

Inside a subsystem's lines a second bracket names the part that wrote it, for example `[looter] [sig]`, `[gatherer] [gimmick]` or `[dispatch] [banking]`. To see one subsystem, grep for its tag: `just log | grep '\[gatherer\]'`.

## A healthy start

The first line names the plugin's version and each subsystem's (`Desert Tooling <ver> loaded, pid <n> (looter <ver>, gatherer <ver>, overlay <ver>, dispatch <ver>)`). After it:

- **Looter**: nine `[sig] <name> = +0x...` lines, one per byte signature, then the summary `[sig] 9 found, 0 missing, <n> actor-manager vtable(s), <n> ms`; two `[hook]` lines, `area_sweep +0x... -> stub ...` and `enqueue +0x... -> stub ...`; one `[hunting] loot_grant +0x... -> stub ...` line; then `[event]` lines.
- **Gatherer**: `[gimmick] record loader at +0x...`, then `[hook] record loader +0x... -> stub ...`, then `[catch] hook at +0x... -> stub ...`, then one `[gimmick]` line per gather record as the game loads its table.
- **Overlay**: `[hudhook] DX12 hooks applied in <n> ms; press the menu key in game`.
- **Dispatch**: a `[banking]` line reporting the two gates the game's mission-banking code sits behind, and what that means for this launch.

Anything that fails to resolve says so on its own line (`NOT FOUND`, `NOT hooking`, `FAILED`) and that part stays idle. See [After a game update](game-update.md).

`[gatherer]` lines record what was **written** to the records, not what you received. To measure real yields, set `[Looter] LogReceived=1` and read the `[recv] item <key> x<count>` lines. Coin props write no `[recv]` line; measure `Money` from the bag count an F11 survey prints, before and after the pickup.

## Files beside the plugin

| file | what it is |
|---|---|
| `DesertTooling.asi` | the plugin; deleting it uninstalls it |
| `DesertTooling.ini` | the settings; recreated at every default on the next launch if missing |
| `DesertTooling.ini.tmp` | transient: the menu writes the new ini here, then renames it over the real one |
| `DesertTooling.log` | this log |
| `DesertTooling.yields` | a small learned cache of "this node gave this item"; it relearns itself if deleted |
| `DesertTooling.gimmickinfo.bin` | the vanilla `gimmickinfo` table body, about 22 MB; written only with `[Gatherer] DumpTable=1` and `DryRun=1`, once per session, for the offline tools |
