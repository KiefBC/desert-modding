# The Desert Gatherer

> **Superseded.** This pack is superseded by **DesertGatherer.asi** (see
> [`../desert-gatherer`](../desert-gatherer)), which does the same thing in memory as the game loads
> its data, so no game file is modified and no rebase is needed after a game update. The pack is
> kept here for people who use DMM without an ASI loader. **Do not mount it together with the
> `.asi`**: both edit the same minimum/maximum quantities, and the two multiply on top of each
> other.

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

1. Import or drag `DesertGatherer-DMM-1.1.zip` into Definitive Mod Manager.
2. Check the desired module(s), using at most one rate per category.
3. Mount mods.

The zip carries a `dmm_pack.json` at its root, so DMM shows the pack's own title and description on its card rather than the zip's folder name.

Disable DMM's built-in gathering multiplier preset if it is mounted; it edits the same game table and will conflict with these modules.

## What Version

This pack changes the minimum and maximum item quantities produced by genuine foraging, logging, mining, and ore collection nodes. It does not alter enemy loot, chests, fishing, skinning, quests, Abyss objects, artifacts, gates, or fast travel.

Built and verified against Crimson Desert Enhanced Steam build 25116796 (pack version 1.1). Version 1.0 targeted build 24994088 and no longer applies cleanly.

## Verified coverage

- Foraging: 82 records, 322 outputs, 644 scalar edits per rate option
- Logging: 141 records, 141 outputs, 282 scalar edits per rate option
- Mining: 36 records, 108 outputs, 216 scalar edits per rate option
- Ore Nodes: 16 records, 16 outputs, 32 scalar edits per rate option

## Updating for a new game build

Record positions move whenever the game updates, and DMM's automatic offset relocation only recovers some of them. To rebase the pack:

1. Mount anything in DMM once so it extracts the clean table to `<DMM folder>\backups\gimmickinfo_pabgb_clean.bin`.
2. Read the new build id from `steamapps\appmanifest_3321460.acf`.
3. **Structural check (catches the failure mode that silently breaks `rebase.py`, before spending time on it).** Run
   `nix develop --command cargo test --release --target x86_64-unknown-linux-gnu -p desert-core -- --ignored --nocapture`
   (`desert-core/tests/gimmick_real.rs`). It resolves the record loader in the new `CrimsonDesert.exe` the same way the ASI hook does, then runs the actual `gimmick::multiply` signature logic (the code the ASI ships) against the freshly extracted clean table and asserts it reproduces every edit in the current `desert-gatherer-dmm/*2X.json` files byte-for-byte (exact offsets, exact old/new values, exact record/block/patch counts). This is a stronger check than reading a decompile: it is the real signature code running against the real new data, not a human judgment call.
   - The test has build-pinned constants (`LOADER_RVA`, the clean-table byte length) left over from build 25116796; on a new build it will fail those specific asserts first, but the `println!`s above them print the real resolved RVA and table length before failing. Update the constants to match and re-run.
   - If it then passes cleanly (counts still 275 records / 587 blocks / 1174 offsets, no missing/extra offsets, no out-of-range values), the byte-signature assumptions rebase.py depends on (`BLOCK=68`, `MIN_AT=42`, `MAX_AT=50`) are confirmed good for this build. Proceed to rebase.
   - If it fails on the pack cross-check itself (not just the pinned constants), the block format likely changed. Only then is it worth decompiling the loader in Ghidra (`analysis/gimmick-loader.c` / `gimmick-record.c` are the archived baseline for build 25116796) to see what moved. Compare control flow and struct offsets by eye or ask Claude to judge equivalence, never a literal text-diff, since Ghidra renames local variables between decompile runs even when a function hasn't changed.
4. Run `python rebase.py <path to gimmickinfo_pabgb_clean.bin> <build id>`.

The script re-locates every record by key and name, finds each resource-output list by its byte signature, verifies the vanilla values still match, rewrites all module JSONs, and regenerates `VERIFICATION.txt`. It refuses to write anything if a single patch cannot be resolved unambiguously. If it exits on one specific record rather than most/all of them after step 3 passed, that usually means the record moved further than `SEARCH` (1024 bytes) from its last known position; try widening `SEARCH` before suspecting the format itself.
5. Bump the pack version and the "Built and verified against build ..." line below (see the 1.0 → 1.1 precedent) once `rebase.py` and its `VERIFICATION.txt` both come back clean.
