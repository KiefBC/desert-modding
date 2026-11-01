# After a game update

Source: `desert-tooling/README.md` (How it survives game updates), the refusal log calls in `desert-harvest/src/looter/game.rs`, `desert-harvest/src/gatherer/{entry,hook,catch}.rs`, `desert-dispatch/src/node.rs`.

This page is the player side. The developer checklist is in [Build, test and just recipes](../dev/build-and-test.md).

## Why an update usually changes nothing

The plugin carries no hard-coded game address. Everything is resolved by content at every launch:

- auto-loot finds its functions by nine byte signatures and the actor manager by its RTTI vtable;
- the yield multipliers find the record loader through the accessor that names the `gimmickinfo` table, refuses to patch unless the twelve bytes it would overwrite are exactly the expected prologue, and finds the yield numbers by walking each record's output lists;
- the `Bugs` and `Fish` patch scans for a 21-byte instruction sequence and refuses unless the thirteen bytes it would overwrite are the two instructions it expects;
- the Overlay looks up no game address at all.

If a function or a layout really changed, that part logs why and stays idle. It never guesses: a refusal is a subsystem that does nothing, never a crash.

## What a refusal looks like

| line | meaning |
|---|---|
| `[harvest] [sig] <name> NOT FOUND` (or `AMBIGUOUS`) | an auto-loot signature stopped matching; what depends on it is off |
| `[harvest] [hook] ... NOT hooking` | an auto-loot hook's prologue was not the expected bytes |
| `[harvest] [hunting] loot_grant signature missing` or `... NOT hooking` | the `Hunting` multiplier is off; everything else works |
| `[harvest] [gimmick] record loader NOT found` | gathering yields stay vanilla |
| `[harvest] [hook] loader prologue is ... not ...; NOT hooking` | gathering yields stay vanilla |
| `[harvest] [catch] signature not found` (or `ambiguous`, or `... NOT hooking`) | `Bugs` and `Fish` are off, alone; every other multiplier is untouched |
| `[harvest] [gimmick] stream layout unknown on this build: ...` | the plugin could not tell where the game keeps the table data it loads from; gathering yields stay vanilla and `DumpTable=1` writes nothing. `stream layout not parsed ...; probing` on its own is not a problem if a `stream layout: ... (probed on the first call ...)` line follows |
| `[harvest] [gimmick] WARN no record header parsed in N records: the record format changed; yields stay vanilla` | the game changed how a gathering record starts; gathering yields stay vanilla for the session and nothing else is affected. The plugin needs an update |
| `[harvest] [gimmick] WARN no record stream read in N records: the stream layout changed; yields stay vanilla` | the plugin could not read the table data the game loads from; gathering yields stay vanilla for the session and nothing else is affected. The plugin needs an update |
| `[harvest] [live] WARN parsed layout not found (...); sliders have no effect until the next load, the hook still multiplies at load` | moving a yield slider in game does nothing on this build; the multiplier you set still applies from the next launch, and nothing else is affected. Report it with a `Debug=1` log |
| `[harvest] [gimmick] idx N: stream cursor ... != offsets table ...` | the table load did not match the expected layout; every record that fails this check is left vanilla, and the line is written only once. After an update this is the first yield line to check |
| `[dispatch] [banking] ...` | read it after every update: it says whether the game's mission-banking gates are open on this build |

## What to report

Set `[Harvest] DryRun=1` if you want to see what the yield multipliers would do without them writing anything. Then launch once, and send `DesertTooling.log` from that session together with the game's Steam build number. The lines above, and the first lines of the log, are what a fix needs.
