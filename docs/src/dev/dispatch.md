# Dispatch internals

Sources: `desert-dispatch/src/lib.rs`, `scan.rs`, `apply.rs`, `node.rs`, `buff.rs`, `config.rs`, `desert-core/src/gimmick.rs`, `CLAUDE.md` (How Desert Dispatch works). Evidence: `docs/findings/2026-09-10-dispatch.md`; `docs/reference-internals.md` §20 (§20.3.1 duration, §20.5 and §20.18 banking, §20.14 accessor census, §20.17 and §20.22 `dropsetinfo`).

## What it is

The dispatch subsystem began as a read-only census of the game's dispatch missions (**FactionOperation** internally) and now also edits them. It still installs **no hook and patches no game code**, so its place in `desert-tooling`'s spawn list cannot matter, and `Enabled=0` at launch means nothing is read for the session.

A dispatch mission is not its own table: it is a 0x120-byte sub-record inside a `FactionNodeInfo` record, pointer at `record+0xC0`, count at `+0xC8` on 26345091 and later (`+0xA0` and `+0xA8` through 25477059). `node.rs`'s `mod parsed` is the whole offset table.

## Reaching the tables

The static-info accessor template exists in **four encodings** (`gimmick::ACCESSORS`): `xor r9d,r9d` as `45 33 C9` or `45 31 C9`, and the table name loaded directly (`lea r8,[rip+string]`) or through a pointer cell holding its VA (`mov r8,[rip+cell]`). 98 + 13 + 31 + 7 = **149 copies, one per static-info type**. `gimmick::AccessorSites::scan(img, image_base)` does the passes once; `sites.manager_slot(img, table)` and `sites.record_loader(img, table)` look up against them. The image base belongs to the scan: without one the two indirect encodings are not searched. This subsystem scans with a base (it needs `FactionNode`, `dropsetinfo`, `buffinfo`, `statusinfo`); the looter and gatherer scan without.

Across the plugin, startup makes 8 full passes over the image:

| Site | Passes |
| --- | --- |
| `desert-harvest/src/looter/game.rs` `pattern::find_unique_multi`, the nine signatures | 1 (was one per signature) |
| `desert-harvest/src/looter/game.rs` `rtti::vtables_for_class` | 3 |
| `desert-harvest/src/looter/tables.rs` accessor scan, no base | 1 |
| `desert-harvest/src/gatherer/entry.rs` accessor scan, no base | 1 |
| `desert-harvest/src/gatherer/catch.rs` `CATCH_SITE` | 1 |
| `desert-dispatch/src/scan.rs` accessor scan, with base | 1 |

The three RTTI walks are a dependency chain (descriptor, then locator, then vtable), so they cannot be batched like the signatures.

## The pass

The game parses its static-info tables once during loading and keeps the objects for the session, so a plugin-thread walk of `manager+0x58` sees everything. `scan.rs` waits (bounded) for a non-zero record count, then `watch()` never returns: every poll re-reads the count and the record array, walks every record and operation entry, and logs one line per mission **not logged before**. Each pass ends with counts, the condition-key histogram, the skill census and a `verdict()` line on whether the offsets look right. `verdict()`'s constant detection (`all_equal`) is what caught the wrong duration offset (§20.3.1); do not remove it. `node.rs` holds everything judgeable without the game, so the verdict is unit tested natively. `MaxLines` budgets every line per pass; every failed read is a counted skip.

## The levers

`apply.rs` edits records the game already parsed: `Speed` divides the duration (the **middle** of the three-entry step list, `parsed::STEP_DURATION`, a `u32` in tenths of an hour, never `entry+0xC0`), `NoSkillRequirement` clears `OP_SKILL_REQ`, `AnyOperatorCount` sets `OP_MIN_OPERATORS` to 1, `Rewards` multiplies both halves of the `dropsetinfo` min/max pair. Four settings, **five** fields. Every write goes through `Ctx::put`, whose body is `self.dry || safe::write(at, v)`, and there are exactly four call sites. Under `DryRun=1` the pass reports and writes nothing. Nothing outlives the process: the game re-reads these tables from disk every launch.

`OP_SKILL_REQ` and `OP_MIN_OPERATORS` share a trap: `0xFFFF` clears a requirement, `0` makes the mission unstartable (§20.16).

## Banking and the buff census

At startup `scan.rs` reads two gates that would let a finished mission bank a figure (deferred reward, surplus-worker bonus) and prints them on the `[banking]` line. They were off on every build measured so far (25116796, 25246367, 26361232); check that line after an update. `DumpBuffs=1` runs a separate read-only census of `buffinfo` and `statusinfo` (`buff.rs`, `census.rs`), logged under its own `[buffs]` tag.
