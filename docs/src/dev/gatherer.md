# Gatherer internals

Sources: `desert-gatherer/src/entry.rs`, `hook.rs`, `catch.rs`, `dump.rs`, `desert-core/src/gimmick.rs`, `desert-core/src/creature.rs`, `CLAUDE.md` (How Desert Gatherer works). Evidence: `docs/reference-internals.md` §16, §17, §19.6.

## The record-loader hook

One hook, no polling. `desert_core::gimmick` finds the `gimmickinfo` record loader in the running image by content, through the accessor that names `gimmickinfo` (one accessor scan without an image base). `start()` checks the 12 bytes of `LOADER_PROLOGUE` and installs the trampoline at once: the table is read during loading, so there is no boot grace and nothing ahead of it on its thread. Log: `[gimmick] record loader at +0x...`.

The callback runs on whichever game thread loads a record, before the deserializer reads it. `idx` arrives in `r8w`, masked with `0xFFFF`. From the manager: record count `+0x08`, offsets table `+0x28` (`u32` file offset at entry+4, records contiguous in index order), loaded-object array `+0x58`. From the stream: buffer `+0x10`, size `+0x18`, cursor `+0x1C`. The record bytes are copied out, classified by key against `desert_core::collect`, and `gimmick::multiply` returns the `u64` min/max edits, written back with `safe::write`.

**One hook is enough**: the record deserializer has exactly one reference in the image, the call inside the loader, and every code reference to the `"gimmickinfo"` string is accounted for (`docs/reference-internals.md` §19.6). The caveat is the preload path, which starts its cursor at 0 instead of consulting the offsets table, so the hook's `cursor != offsets[idx]` guard depends on records staying contiguous. If that breaks, the failure is silent and total for that pass: one `[gimmick] idx N: stream cursor ...` line and every record vanilla, no crash. **After a game update that is the first line to check.**

The stream buffer is freed about 2 s after the last load and reallocated on demand, so patching happens per record inside the hook. The hook's only shared state is atomics: it reads its settings from `config::LIVE`, a struct of atomics the plugin thread publishes into, so a change reaches the next record with no lock. It never calls a game function and returns vanilla on any failure.

## Re-apply

Every gather record's vanilla min/max and item id per output block, and its index, are kept in `remember`, a fixed lock-free table of atomics. After an `[ini] reloaded:` line, `hook::reapply` on the plugin thread walks that table through the game's record manager and writes `vanilla * multiplier` into the already-parsed objects, using the offsets in `mod parsed` (`docs/reference-internals.md` §16) and checking each record's key and block's item id first. Under `Enabled=0` records are read but not written at load, and a re-apply writes vanilla back.

The ini is shared, so its mtime moves for other sections' keys too. The gatherer compares the section it just parsed with what it is running on and does nothing when it did not move; otherwise a `[Looter]` slider drag would re-apply every record once a second.

## Yield model

Measured in game on build 25116796 on a `Foraging` node: a gather pays out **one** of the record's output blocks, apparently at random, rolled between that block's min and max. Multiplying every block scales the whole distribution. Single-block records (firewood, ore) follow by the same arithmetic but `Logging`, `Mining` and `Ore` have not been observed in play. Measure with the looter's `LogReceived=1` (`[recv]` lines), not with `[gatherer]` lines, which show only what was written.

## The catch hook

`Bugs` and `Fish` are not gimmickinfo: a creature caught by hand is a character, and the amount granted is a hard-coded `mov r8d,1` (`docs/reference-internals.md` §17). `catch.rs` scans for the 21-byte `CATCH_SITE` (must hit once), checks the 13 bytes it will overwrite, and installs a stub from `desert_core::trampoline::count_hook_stub` through `hook::install_raw` that **replaces** the immediate with its callback's return value. The callback reads the actor's type and class bytes through `safe` and returns 1 for anything that is not a recorded bug or fish class. A game update that moves this site disables only this hook, with a `[catch]` line.

## Table dump

`DumpTable=1` together with `DryRun=1` copies the whole table body from the loader's stream buffer, once per session on its first call, to `DesertTooling.gimmickinfo.bin` (`dump.rs`). `DryRun=1` is required because with multipliers live the copy could hold already-multiplied records. The offline tools read this file (see [Ghidra and analysis](ghidra.md)).

DMM's Desert Gatherer pack and DMM's built-in gathering multiplier edit the same scalars on disk; mounting either with this plugin multiplies yields twice.
