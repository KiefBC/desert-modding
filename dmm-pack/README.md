# The Desert Gatherer

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

1. Import or drag `The Desert Gatherer.zip` into Definitive Mod Manager.
2. Check the desired module(s), using at most one rate per category.
3. Mount mods.

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
3. Run `python rebase.py <path to gimmickinfo_pabgb_clean.bin> <build id>`.

The script re-locates every record by key and name, finds each resource-output list by its byte signature, verifies the vanilla values still match, rewrites all module JSONs, and regenerates `VERIFICATION.txt`. It refuses to write anything if a single patch cannot be resolved unambiguously.
