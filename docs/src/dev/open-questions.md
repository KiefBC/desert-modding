# Open design questions

This page replaces the root TODO file the repo carried until 2026-11-01, kept to what is still open or decided and not yet built. The full history, with every argument, refutation and confidence label, is archived locally in `docs/archive/` under that date (gitignored).

The findings notes cited below live in `docs/findings/`, which is gitignored and local-only, so they are named as paths rather than linked. Offsets quoted from before build 26361232 use that era's record layout (see [Known drift](known-drift.md)).

## The item-instance lever

**OPEN, untested.** Objects that hand the player a pre-built item instance never read their gimmickinfo `DropInfoData` block, so a table edit multiplies nothing in them. The proof is from the 2026-09-12 session: `gimmick_item_trade_pepper_01`/`_03` patched `1->3` still granted `[recv] item 1000608 x1`, and the water pot paid a flat 4 to 6 across a 100x range. `FUN_1429db730` calls the yield producer only when the actor's `comp+0xC0` instance pointer is null. The granted count sits at `instance+0x10`, a heap field that lives only while the prop does, and the looter's `actors::classify` already finds those actors (`Kind::Item`) and their record (`comp+0x48`).

The lever would be a plugin-thread `safe::write`, the dispatch shape: no hook, no patch. It is deferred on two blockers in the design note: the live read of `instance+0x08` (`1 @8`) does not reconcile with it being an `iteminfo` row index, so the link from `comp+0xC0` to the built object is PLAUSIBLE only; and nothing yet tells a game-spawned drop from a stack the player dropped. The write must also be idempotent per instance, skip the player's own equipment (`actors::refers_to_player`), and may be clamped by `ItemInfo._maxStackCount`.

**Settles it:** read a dropped prop's `instance+0x10`, write `vanilla * n`, pick it up with `[Harvest] LogReceived=1` and read `[recv] item <key> x<count>`. That one result decides trade goods and most of the unclassified records below. Design: `docs/findings/2026-09-12-design-instance-lever.md` (local).

## Survey truncation at 64 lines

**Closed 2026-09-13, in 0.4.1.** The looter's survey printed the 64 nearest actors and cut the rest silently, which made the 2026-09-12 pot session unreadable (the third pass printed 5 of 50 `Item` actors). It is now `[Looter] SurveyLines` (default 200, range 16 to 2000) with kind-first ordering and a `listing capped at N lines ...` line when anything is cut; `ScanRange` and `SurveyLines` descend from it. Kept here because code comments in `crates/desert-harvest/src/looter/` cite it.

## Trade goods

**OPEN.** Two groups the mod cannot reach. The 29 block-carrying `gimmick_item_trade_*` records (one `1..1` block each) have the same shape as the 15 edibles proven inert in game, so they need the instance lever, not table rows; if they enter `crates/tools/extra-families.json` before it exists, they go in as `records_not_enabled`. The 397 `gimmick_collection_prop_*` / `Collection_Prop_*` records (the Ceramic the player picked up) carry no output block of any shape, and how they grant an item is UNCONFIRMED.

When built, it is its own key (`TradeGoods`, record-keyed, default `1`, ini text saying these are stall goods under the steal check and an economy lever), not a gathering family and not folded into `Money`.

**Settles it:** one launch, no code change. Pick up a Ceramic prop with `LogReceived=1`, watch for a `[catch] class=XX not a known bug/fish class` line at the same moment, and note the survey's `Kind` for it. `Kind::Item` folds the group into the instance lever; a catch line means a class byte in `crates/desert-core/src/creature.rs` covers it. Also check whether it yields `1000619`, the stall pot's item.

## Hunting scales the copper from cleaning fish

**OPEN, unmeasured.** The fish carcass dropsets (`DropSet_*_Animal_Fish_*`) carry Copper x20..150, so `Hunting` (which scales whatever the game rolled for a corpse, and for a body searched since 0.13.0) multiplies that copper too, on top of whatever `Money` does to coins. Whether the two stack (a fish cleaned at `Hunting=10` paying the money lever's rate again), or the grant is a different path from the placed coin props `Money` edits, has not been checked. Found in the item catalogue (`analysis/ITEMS.md`).

**Settles it:** clean a fish with `LogReceived=1` at `Hunting=1`, then at `Hunting=10`, and compare the bag's copper; repeat with `Money` raised to see whether it moves.

## The second yield route

**OPEN.** `FUN_141775790` runs two yield loops back to back: the output blocks the gatherer scales (`rec+0x278`/`+0x280` on 25477059) and a drop-set list (`rec+0x268`/`+0x270`, amounts in `dropsetinfo entry+0x20`/`+0x28`) it never touches. Among the 573 records with an output list, none also carries a drop-set list (CONFIRMED, 573/573). The ~13,300 records with no output list are unmeasured, and a gimmick that pays through `rec+0x268` alone would be hooked, remembered and scaled by nothing, silently. `docs/reference-internals.md` section 19.6.1 carries the same split: hook coverage is proven complete, multiplier coverage is not.

**Settles it:** a census of `rec+0x268` counts over every record in a table dump. Source: `docs/findings/2026-09-12-water-wells.md` §6 (local).

## The 297 unclassified records

**OPEN.** The table holds 573 output lists and 896 blocks on 25477059; the gather families cover 276 records and 588 of those blocks, leaving 297 records carrying 308 blocks classified by nothing: containers, breakables, market props, dried food, donation boxes. The money records are the proof that a residual bucket is wrong, since a blanket `gimmick_item_*` family would have multiplied income. Each candidate group needs a record-keyed list in `crates/tools/extra-families.json` with evidence and a generator check, and first an answer to whether a table edit reaches the record at all (16 of the withdrawn `Ingredients` family's 17 rows were inert).

Before anything large lands, raise `MAX_RECORDS` (512, `crates/desert-harvest/src/gatherer/remember.rs`; 294 used today): overrunning it leaves the excess vanilla with no log line.

**Settles it:** per group, one logged pickup at a multiplier. Census: `docs/findings/2026-09-12-water-wells.md` §7 (local).

## Digging

**OPEN.** Two dig records carry output lists the block detector cannot see: `Action_dig_01` (`120040092`, items `1000629`, `200049`) and `gimmick_Dig_land_0001` (`1005245`, five items, each `1..1`). `block_ok` requires the zero pads at `+5` and `+64` to match, and these blocks carry a nonzero `u32` at `+64`. Dropping that clause grows the population from 573/896 to 589/1038; the clause stays because it held the population constant across the 2026-09-12 off-by-4 fix. The `+64` value is a list-entry key shared between records as reusable loot groups, which may make this the same problem as the second yield route; `treasurebox_01`'s nine ids around `391518521` show at least one of the 16 lists is mis-parsed.

**Settles it:** dig once with `LogReceived=1`. Ids that do not match mean the detector is right to skip them; ids that match mean teaching `block_ok` a second shape without moving the 573/896 population. Also check whether the `+64` keys are `dropsetinfo` keys. Source: `docs/findings/2026-09-12-water-wells.md` §7 (local).

## Item names and the category triple from `.paz`

**OPEN, deliberately deferred.** The `categoryinfo` main/middle/sub triple and the localized display strings live in table bodies inside the `.paz` archives (about 31 GB under `0000/`). The triple would let a family be populated from the game's own data instead of curated by hand. Internal item names no longer need it: the looter's `[survey] bag:` line prints them live (`Water key=22008`, `Money_Copper key=1`). Deferred because no shipped family waits on it; record-name inference covers current needs. Know before starting: `pathc_clean.bin` is a DDS thumbnail cache, not a table body.

**Settles it:** an extraction tool that reads one known table body out of the archives and round-trips it. Source: `docs/findings/2026-09-12-water-wells.md` §4 (local).

## Drop-rate levers

**Collect node CLOSED, buff system OPEN.** `GimmickEventHandlerData_SetAdditionalCollectDropRate` is a gimmick-chart node writing per-instance state, unreachable by any write path here; do not spend more static time on `gimmick+0x1A0`. What it uncovered is the 122-value buff-effect kind enum. `[Dispatch] DumpBuffs=1` (log tag `[buffs]`) is built and ran twice on 2026-09-13: kind 2's key is `DropInfoData._dropTagNameHash` (seven `BuffLevel_CollectDrop_*` records, +10% collect-drop chance per level), and the money axis is `BuffLevel_Socket_AddMoneyDropRate` (`buffinfo` key 1000117, ten `VaryStaticStat` levels on `statusinfo` row 46). Both are dispatch-shaped parsed fields, effective only while the player carries the buff.

**Settles it:** one launch with the money socket equipped and one coin prop picked up with `LogReceived=1`, to see whether the buff scales placed money or only dropped money. Sources: `docs/findings/2026-09-13-drop-rate-levers.md` and `docs/findings/2026-09-13-buff-stat-census.md` (local).

## Money pouches

**OPEN.** The `Money` slider edits the three placed money gimmickinfo records (two coin piles and one silver bar). Coin pouches are inventory items that pay through the item-use path: the September logs show the payout arriving as item `1` with the amount as the count (`[recv] item 1 x120` and `x20` from two pouches opened in the inventory, 2026-09-13), most likely a dropset row. A pouch lever would be its own key that finds the pouch's dropset row and multiplies its min and max, as the dispatch `Rewards` lever already does. What is unknown is the link from the pouch item to its row.

**Settles it:** find the reference from the pouch's `iteminfo` record to a `dropsetinfo` key, then confirm with one opened pouch at a multiplier. Source: `docs/findings/2026-09-12-water-wells.md` §13.3 (local).

## Two `Logging` records

**Half done (2026-11-02).** `gimmick_wood_firewood_0001` is a `Logging` row now: the archive-derived rule in `gen-collect-names` (`classify.rs`) picks it, because it is a `Gimmick_PickUp` record paying only wood-material items. `log_bamboo_1001` is still not a row: the rule does not pick it. The tag census found exactly two records with a `collect*` tag and an output block that no family covers: `log_bamboo_1001` (`1000669`, item `1002124`, `1..1`) and `gimmick_wood_firewood_0001` (`1010558`, item `710001`, `1..1`). The re-target unblocked it on 2026-09-24. The change: a `Logging` block in `crates/tools/extra-families.json`, one `HAND_TEST` assert in `gen-collect-names`' `render.rs`, regenerate (`COLLECT_RECORDS.len()` 279 to 281, `Logging` 141 to 143), then the help text, README row, ledger and loot-taxonomy note. Never hand-edit `collect.rs`. Each row stays CONFIRMED-STATIC until a pickup is logged, because a record holding an item instance ignores its block.

**Settles it:** pick up a firewood pile (survey `name=gimmick_wood_firewood_0001`) at `Logging=3` and expect `[recv] item 710001 x3`; `x1` moves it to `records_not_enabled`. Bamboo waits until the player can cut a stalk. Source: `docs/findings/2026-09-12-loot-taxonomy.md` §2.2, §4A (local).

## Item display-name table

**DECIDED, nothing built.** `docs/reference-items.md` names items by inference from the yielding records, the survey prints the internal `iteminfo` name, and neither is what the HUD shows (the inferred `firewood` and `firewood_fine` are **Timber** and **Fine Timber** in game, CONFIRMED-GAME 2026-09-24). Design: `crates/tools/item-display-names.json` keyed by item id, `{"display", "internal", "seen"}`, merged by the `items` tool into a Display name column and an `analysis/items.json` field. It fails on an id the walk does not know and refuses a row with no `seen`. Item `1` gets a note instead of a name, since copper, silver and gold are display units over one count.

**Settles it:** nothing to measure; it is a build task, and rows are added one pickup toast at a time.

## Settled

- **Currency multiplier:** resolved 2026-09-13, shipped in 0.5.0 as the `Money` family; `Money=3` on one `coin_0001` logged `[recv] item 1 x30`. Evidence: `docs/findings/2026-09-12-water-wells.md` §13.7 (local).
- **The water well is a `Foraging` record:** landed in 0.4.1; the 16 inert `Ingredients` rows became `records_not_enabled`. Evidence: `docs/findings/2026-09-12-water-wells.md` §3, §11 and §12 (local).
- **Multi-step gimmick test:** resolved 2026-09-13, no: a forged pickup at a well emptied it permanently, so the looter refuses the well by record key (`never_forge_pickup`). Evidence: `docs/findings/2026-09-12-water-wells.md` §13.1 (local).
