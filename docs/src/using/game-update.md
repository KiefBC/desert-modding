# After a game update

Source: `desert-tooling/README.md` (How it survives game updates), the refusal log calls in `desert-looter/src/game.rs`, `desert-gatherer/src/{entry,hook,catch}.rs`, `desert-dispatch/src/node.rs`.

This page is the player side. The developer checklist is in [Build, test and just recipes](../dev/build-and-test.md).

## Why an update usually changes nothing

The plugin carries no hard-coded game address. Everything is resolved by content at every launch:

- the Looter finds its functions by nine byte signatures and the actor manager by its RTTI vtable;
- the Gatherer finds the record loader through the accessor that names the `gimmickinfo` table, refuses to patch unless the twelve bytes it would overwrite are exactly the expected prologue, and finds the yield numbers by walking each record's output lists;
- the `Bugs` and `Fish` patch scans for a 21-byte instruction sequence and refuses unless the thirteen bytes it would overwrite are the two instructions it expects;
- the Overlay looks up no game address at all.

If a function or a layout really changed, that part logs why and stays idle. It never guesses: a refusal is a subsystem that does nothing, never a crash.

## What a refusal looks like

| line | meaning |
|---|---|
| `[looter] [sig] <name> NOT FOUND` (or `AMBIGUOUS`) | a Looter signature stopped matching; what depends on it is off |
| `[looter] [hook] ... NOT hooking` | a Looter hook's prologue was not the expected bytes |
| `[looter] [hunting] loot_grant signature missing` or `... NOT hooking` | the `Hunting` multiplier is off; everything else works |
| `[gatherer] [gimmick] record loader NOT found` | gathering yields stay vanilla |
| `[gatherer] [hook] loader prologue is ... not ...; NOT hooking` | gathering yields stay vanilla |
| `[gatherer] [catch] signature not found` (or `ambiguous`, or `... NOT hooking`) | `Bugs` and `Fish` are off, alone; every other multiplier is untouched |
| `[gatherer] [gimmick] idx N: stream cursor ... != offsets table ...` | the table load did not match the expected layout; every record that fails this check is left vanilla, and the line is written only once. After an update this is the first gatherer line to check |
| `[dispatch] [banking] ...` | read it after every update: it says whether the game's mission-banking gates are open on this build |

## What to report

Set `[Gatherer] DryRun=1` if you want to see what the gatherer would do without it writing anything. Then launch once, and send `DesertTooling.log` from that session together with the game's Steam build number. The lines above, and the first lines of the log, are what a fix needs.
