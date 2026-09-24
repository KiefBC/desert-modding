# The Desert Gatherer

> **Superseded.** This pack is superseded by the gathering-yield subsystem of
> **DesertTooling.asi** (see [`../desert-tooling`](../desert-tooling)), which does the same thing in
> memory as the game loads its data, so no game file is modified and no rebase is needed after a
> game update. The pack is kept here for people who use DMM without an ASI loader. **Do not mount
> it together with the `.asi`**: both edit the same minimum/maximum quantities, and the two
> multiply on top of each other.

A DMM module pack for choosing independent 2x, 5x, or 10x gathering yields.

## Select your rates in DMM

Enable no more than ONE rate under each category:

- Foraging: 2X, 5X, or 10X
- Logging: 2X, 5X, or 10X
- Mining: 2X, 5X, or 10X
- Ore Nodes: 2X, 5X, or 10X

Categories are independent. For example, you can enable `Mining - 2X` and `Logging - 5X` together. Leave a category unchecked to keep its vanilla yield.

Do not enable two rates for the same category. They edit the same values and the last-applied option will win, just like the rate tiers in Enhanced Dispatch.

### Mining versus Ore Nodes

Crimson Desert stores mining-style resources in two separate internal gathering families:

- **Mining** targets the `collect_mine` family, including `mine_*` rocks and veins, cave and attached variants, and breakable stalactites.
- **Ore Nodes** targets the `collect_ore` family, including `ore_*` deposits, sulfur stone, and rare collectible stalactites.

These activities may appear similar in-game, but changing one family does not affect the other. Select the same multiplier for both Mining and Ore Nodes if you want all mining-style gathering increased consistently.

## Installation

1. Import or drag `DesertGatherer-DMM-1.2.zip` into Definitive Mod Manager.
2. Check the desired module(s), using at most one rate per category.
3. Mount mods.

The zip carries a `dmm_pack.json` at its root, so DMM shows the pack's own title and description on its card rather than the zip's folder name.

Disable DMM's built-in gathering multiplier preset if it is mounted; it edits the same game table and will conflict with these modules.

## What Version

This pack changes the minimum and maximum item quantities produced by genuine foraging, logging, mining, and ore collection nodes. It does not alter enemy loot, chests, fishing, skinning, quests, Abyss objects, artifacts, gates, or fast travel.

Built against Crimson Desert Enhanced Steam build 25477059 (pack version 1.2), from the plugin's own dump of that build's clean `gimmickinfo` table (sha256 `627de5cab68cf23856602498ca5fd0ed30e71e5faa8171368cc460ab4e7f7fed`). On this build every record moved within the table, so all of 1.1's absolute offsets were stale and the pack had to be rebased; inside each record nothing moved, and every one of the 275 records still carries the same output blocks (same items, same minimums and maximums). The rebase changed only absolute offsets: the same 275 records, 587 output blocks and 1174 offsets per rate tier as 1.1, with no change in coverage.

Version 1.1 targeted build 25116796 and also applied unchanged on build 25246367. Version 1.0 targeted build 24994088. Neither applies cleanly on 25477059.


## Verified coverage

- Foraging: 82 records, 322 outputs, 644 scalar edits per rate option
- Logging: 141 records, 141 outputs, 282 scalar edits per rate option
- Mining: 36 records, 108 outputs, 216 scalar edits per rate option
- Ore Nodes: 16 records, 16 outputs, 32 scalar edits per rate option

## Updating for a new game build

Record positions move whenever the game updates, and DMM's automatic offset relocation only recovers some of them. To rebase the pack:

1. Launch the game once with `[Gatherer] DumpTable=1` and `DryRun=1` (menu Debug tab), which writes `bin64/DesertTooling.gimmickinfo.bin`: the plugin's own copy of the clean table, taken from the game's loader buffer. (`CD_DMM_TABLE` points the tools at another copy instead, such as the one DMM extracts to `<DMM folder>\backups\gimmickinfo_pabgb_clean.bin`.)
2. The new build id is read from `steamapps\appmanifest_3321460.acf` automatically; nothing to do unless the manifest is somewhere unusual (`CD_APPMANIFEST`).
3. **Structural check (catches the failure mode that would break the rebase, before spending time on it).** Run `just test-game` (the pack oracle lives in `desert-core/tests/gimmick_real.rs`). Its `multiply_reproduces_the_dmm_pack_edits` resolves every record in the new clean table, runs the actual `gimmick::multiply` signature logic (the code the ASI ships) against it, and asserts it reproduces every edit in the current `desert-gatherer-dmm/*2X.json` files byte-for-byte (exact record-relative offsets, exact old/new values, exact record/block/patch counts). This is a stronger check than reading a decompile: it is the real signature code running against the real new data, not a human judgment call.
   - **A clean table is a precondition, not an option: if it is absent the cross-check passes by skipping**, printing a banner saying the oracle did not run and the run proved nothing. Do not read that `ok` as a structural check — it never happened. The test finds the table by the same rule as the tools: `CD_DMM_TABLE`, else the plugin's dump from step 1, else DMM's backup copy.
   - Nothing in the cross-check is pinned to a game build. It locates every record by key and name and compares `record_rel_offset` (counted from each record's `u32` key) rather than absolute offsets, so it runs against any build's clean table with no constants to update and without the pack being rebased first. That is what makes it usable as a pre-check here.
   - Six other tests in the same run carry build-pinned addresses (`LOADER_RVA` in the record-loader test among them) and can fail right after an update until someone re-pins them; they print the real resolved values when they do. They say nothing about the pack. `multiply_reproduces_the_dmm_pack_edits` is the one that matters here.
   - If the oracle passes cleanly (counts still 275 records / 587 blocks / 1174 offsets, no missing/extra offsets, no out-of-range values), the byte-signature assumptions the rebase depends on (68-byte blocks, minimum at +42, maximum at +50) are confirmed good for this build. Proceed to rebase.
   - If it fails on the pack cross-check itself, the block format likely changed. Only then is it worth decompiling the loader in Ghidra to see what moved. Compare control flow and struct offsets by eye or ask Claude to judge equivalence, never a literal text-diff, since Ghidra renames local variables between decompile runs even when a function hasn't changed.
4. Run `just dmm-rebase` (`just dmm-rebase --dry-run` first to see the report without writing anything).

   The tool (`tools/src/bin/dmm-rebase.rs`) re-locates every record by key and name, finds each resource-output list by its byte signature, verifies the vanilla values still match, rewrites all module JSONs, and regenerates `VERIFICATION.txt`. It checks every module before writing any, so if a single patch cannot be resolved unambiguously it writes nothing at all. It looks for each list within `SEARCH_WINDOW` (1024 bytes) of its last known position inside the record; if it fails on one or two specific records after step 3 passed, a list has most likely moved further than that, not changed format.
5. Bump the pack version in `dmm_pack.json`, run `just sync-versions` (it carries the version into every module file), and update the "Built against build ..." paragraph under "What Version" above, which is also the release notes, once the rebase and its `VERIFICATION.txt` both come back clean.
