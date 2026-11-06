# The Desert Gatherer

> **Superseded.** This pack is superseded by the gathering-yield subsystem of
> **DesertTooling.asi** (see [`../crates/desert-tooling`](../crates/desert-tooling)), which does the same thing in
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

1. Import or drag `DesertGatherer-DMM-1.5.zip` into Definitive Mod Manager.
2. Check the desired module(s), using at most one rate per category.
3. Mount mods.

The zip carries a `dmm_pack.json` at its root, so DMM shows the pack's own title and description on its card rather than the zip's folder name.

Disable DMM's built-in gathering multiplier preset if it is mounted; it edits the same game table and will conflict with these modules.

## What Version

This pack changes the minimum and maximum item quantities produced by genuine foraging, logging, mining, and ore collection nodes. It does not alter enemy loot, chests, fishing, skinning, quests, Abyss objects, artifacts, gates, or fast travel.

Built against Crimson Desert Enhanced Steam build 26480954 (pack version 1.5, 2026-11-06), from the game's own `gimmickinfo` table read straight out of its archives (17,622,309 bytes, sha256 `f5d572291c63c77d87ccd32aef268472e12aa8b62bdd227f2caa1aab85873810`, 14,210 records). Version 1.5 patches exactly what 1.4 patches, at the same offsets with the same values; the two Steam updates since 1.4 changed no gather node's record (below).

Version 1.4 was built against Steam build 26361232 (2026-11-02), from that build's table (17,621,227 bytes, sha256 `3aba0954725e6fc91ca490a5e1afde21357190d58aee112dc9944872ef26bf1b`, 14,210 records). The game build and the table were the same as 1.3's; what changed is where the record list comes from. Every module is now generated from the records the DesertTooling plugin multiplies, which `gen-collect-names` derives from the game's own records, so the pack and the plugin cover exactly the same nodes and the same output blocks:

- Foraging: 82 -> 97 records (322 -> 337 outputs). New: the water well (`gimmick_well_0001_parts01`), razor clams, rare rubber, the four wood and bamboo branch nodes, `docking_core_small` and the seven `seockcheong` nodes.
- Logging: 141 -> 142 records (141 -> 142 outputs). New: `gimmick_wood_firewood_0001`.
- Mining: 36 records, 108 outputs, unchanged.
- Ore Nodes: 16 records, 16 outputs, unchanged.

Every record 1.3 already patched is patched at the same offsets with the same values: 291 records, 603 output blocks and 1206 offsets per rate tier, against 1.3's 275, 587 and 1174.

Steam build 26423880 (2026-11-04) changed `gimmickinfo` only in place: the same 14,210 records, 254 of them with a same-size shift in string-pool offsets, and 3 armour drop boxes whose loot now names the re-keyed Goblin Merchant items; no gather node's yield changed in the catalogue. `just dmm-pack` on the 26423880 table regenerates every module and `VERIFICATION.txt` with identical offsets and values; only the build stamp, the pack version (1.5) and the hashes changed.

Steam build 26480954 (2026-11-06) changed 32 `gimmickinfo` records, puzzles, doors and devices with no output list; one of them grew 1,082 bytes, and every change sits past the last gather record, so no patch offset moved. `just dmm-pack` on the 26480954 table again regenerates every module and `VERIFICATION.txt` with identical offsets and values; only the build stamp and the hashes changed, and the version stays 1.5 because 1.5 was never released.

Version 1.3 targeted the same build 26361232. That build changed the table more than any before it: each record's header starts with its name and carries the key after it, and inside every record the output lists sit 120 to 320 bytes nearer the start of the record than in 1.2, so every list had to be found again rather than carried across. Version 1.2 targeted build 25477059; inside each record nothing had moved since 1.1, so that rebase changed only absolute offsets. Version 1.1 targeted build 25116796 and also applied unchanged on build 25246367. Version 1.0 targeted build 24994088. None of 1.0 to 1.2 applies on 26361232.

## Verified coverage

- Foraging: 97 records, 337 outputs, 674 scalar edits per rate option
- Logging: 142 records, 142 outputs, 284 scalar edits per rate option
- Mining: 36 records, 108 outputs, 216 scalar edits per rate option
- Ore Nodes: 16 records, 16 outputs, 32 scalar edits per rate option

## Regenerating for a new game build

Record positions move whenever the game updates, and DMM's automatic offset relocation only recovers some of them. The pack is not edited by hand: `dmm-pack` (`crates/tools/src/bin/dmm-pack.rs`) generates all twelve modules and `VERIFICATION.txt` from two inputs, the game's own `gimmickinfo` table and the `Foraging`, `Logging`, `Mining` and `Ore` rows of `crates/desert-core/src/collect.rs`. For every row it finds the record by key, checks the record's name, walks its output blocks exactly the way the plugin does, and writes each block's minimum (+42) and maximum (+50) times the rate. To regenerate:

1. `just extract-tables`. Writes the game's static-info tables out of its archives into `analysis/tables/`, where the pack oracle reads them. (`dmm-pack` itself reads the archives directly; `--tables analysis/tables` makes it read the extracted copies instead.) If the game update changed which records gather, run `gen-collect-names` first so `collect.rs` follows it: the pack takes its records from there.
2. `just test-game`. Its `multiply_reproduces_the_dmm_pack_edits` (in `crates/desert-core/tests/gimmick_real.rs`) runs the plugin's own `gimmick::multiply` against every record the committed pack names and asserts it edits exactly the pack's record-relative offsets with exactly the pack's values. Before regenerating, that says whether the block format the generator assumes (68-byte blocks, minimum at +42, maximum at +50) still holds on this build; after it, that the new pack and the plugin agree.
   - **The table is a precondition: if `analysis/tables/` is empty the oracle passes by skipping**, printing a banner saying it did not run and proved nothing. Do not read that `ok` as a check.
   - If it fails on a mismatch, the block format likely changed. Only then is it worth decompiling the loader to see what moved.
3. `just dmm-pack --dry-run`. Builds everything in memory, prints per-category coverage against the published pack and the full `VERIFICATION.txt`, and writes nothing. The build id comes from `steamapps\appmanifest_3321460.acf` (`CD_APPMANIFEST`, or `--build ID`); there is no fallback.
4. `just dmm-pack`. Writes the twelve modules and `VERIFICATION.txt`. It checks every row and every module before writing any, so a row whose record is missing, renamed or pays no output block writes nothing at all, and so does an overlap between two categories (which would make mixing rates unsafe).
5. Diff the result (`git diff --stat desert-gatherer-dmm`). Records already in the pack should only move offsets; records added or removed should be the ones `collect.rs` added or removed. Then bump the pack version in `dmm_pack.json`, rerun `just dmm-pack` (it stamps that version into every module), run `just sync-versions`, and update the paragraph under "What Version" above, which is also the release notes.
