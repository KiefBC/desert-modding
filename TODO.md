# TODO

Open design questions, with enough context to pick them up cold. Not a task list:
each entry is a decision that has been discussed. Check an entry's **Status** line
before acting on it - an entry that has since been decided either keeps its argument
and gains a **Resolution** at the bottom, or leaves this file once its reasoning is
recorded somewhere durable (git history, `VERSIONING.md`, `desert-tooling/CHANGELOG.md`,
`docs/`). "One ASI per mod instead of a separate overlay DLL" was the second kind: it
shipped as `DesertTooling.asi` in `desert-tooling` 0.3.0 and the argument lives in the
0.3.0 changelog entry and in `VERSIONING.md`. Do not reopen it from memory.

Most of what follows came out of the `water-wells` investigation;
`docs/findings-water-wells-2026-09-12.md` is its record, with the evidence, the
confidence labels and the file:line references. Entries cite it by section rather than
restating it - and they keep its confidence labels, so **do not promote a PLAUSIBLE or
UNCONFIRMED claim to fact while acting on one of these**. That doc's section 9 is the
confidence table; section 7 is why it exists at all.

**The in-game session of 2026-09-12 then refuted parts of it**, and the three entries it
touched say so in place rather than quietly dropping the claim. A finding measured in game is
labelled **CONFIRMED IN GAME** and outranks anything static analysis concluded; the first entry
below is where all of those refutations converge, and it is the one to read first.

## The item-instance lever: one write that would reach every case a table edit cannot

**Raised** 2026-09-12. **Status: OPEN, untested.** Nothing has been written and nothing has
been measured. Do not read anything below as known to work.

**Read `docs/design-instance-lever-2026-09-12.md` before acting on this entry.** It is the
costed design and it carries two blockers this entry's hazard list does not:
**§2.5** - `reference-internals` §9.2's live read of `instance+0x08` gives `1 @8`, which does
not reconcile with `+0x08` being an `iteminfo` row index, so "`comp+0xC0` points at the object
`FUN_14234f210` builds" is **PLAUSIBLE, not CONFIRMED**, and *no write may be built until that
is settled*; and **§3.2** - nothing found so far distinguishes a game-spawned drop from a stack
the player dropped, which is why §8 falls back to an item allowlist. Note also that this entry
and the design doc disagree about the config surface: this entry assumes `comp+0x48` reaches
the source record's `collect::Family`, which holds for a **placed prop** that is still its own
actor but not for a **spawned carrier** (`item_basic_cup` after the pot breaks), where
`comp+0x48` names the generic carrier record. Which side the market stalls fall on is
unmeasured and is the F11 observation in `what-we-found` §7.1 #4.

### One cause behind every failure of 2026-09-12

The in-game session of 2026-09-12 refuted three separate "this is just a table edit" plans,
and every one of them failed for the **same** reason. An object that hands the player a
**pre-built item instance** never consults its `gimmickinfo` record's `DropInfoData` block at
all, so multiplying that block multiplies nothing:

| case | records | evidence |
| --- | --- | --- |
| the water pot `Background_Breakable_66` (`21030076`) | 1 | **CONFIRMED IN GAME** that the receipts were flat across a 100x range; **PLAUSIBLE-strong** that this pot produced them - `[recv]` names the item, never the prop, and item `22008` has exactly two block-carrying sources (this and the well). `tools/extra-families.json` carries the same split |
| the 15 `gimmick_item_trade_*` edibles (briefly rows of the withdrawn `Ingredients` family; now `records_not_enabled` in `tools/extra-families.json`) | 15 | **CONFIRMED IN GAME** - pepper patched `1->3`, granted `x1` |
| the other 29 block-carrying `gimmick_item_trade_*` | 29 | same shape, **untested** |
| the three placed money props | 3 | same shape, **untested**; the three donation-box money records are a *different* shape, see the currency entry |
| the 397 collection props | 397 | no block of any shape; **PLAUSIBLE** that this is why |

Three measurements pin it down; **one is CONFIRMED IN GAME outright and the other two are
weaker, which this paragraph used to flatten.** The pepper is the confirmed one. The pot is
PLAUSIBLE-strong on attribution (the receipts are certainly flat; that *this* prop paid them is an
inference from item `22008` having exactly two sources). The egg-bucket control is PLAUSIBLE only,
as the paragraph itself says four sentences below. The pot was smashed at four different
multipliers and paid, read straight off the one saved log
(`analysis/logs/DesertTooling-2026-09-12-water-pot-survey.log`): **x3 -> 5**, **x1 -> 1** (a
partial pickup) and **x1 -> 5** (the full sweep), **x10 -> 6**, **x100 -> 4** - flat across a
100x range, against a block the mod had rewritten from 5 to 500 by the last of them. (A
five-multiplier form of this sequence was in circulation until 2026-09-12 and was **wrong** in
every figure; the five numbers above are what the log holds and are the only ones to quote. Only
the numbers were wrong - the *flat* conclusion is exactly what the log shows.) And
`gimmick_item_trade_pepper_01`/`_03` were both patched `1->3` (both logged) while the player
received `[recv] item 1000608 x1`; pepper has no other block-carrying source record. A control
points the same way with the mod out of it
entirely: `gimmick_ex_dpf_egg_bucket_01` carries a **vanilla, unpatched** `2..2` block and the
log shows `[recv] item 755015 x1`. That control is **PLAUSIBLE only** - the `[recv]` line is
unattributed and nothing in the log names the record that produced it - so it corroborates the
two CONFIRMED measurements rather than standing on its own.

**The mechanism is CONFIRMED by decompile and corroborated by the in-game actor survey.**
`FUN_1429db730` calls the yield producer `FUN_141775790` **only when the instance pointer is
null**. The well `gimmick_well_0001_parts01` has no instance, so it rolls its block and the
`Foraging` slider reaches it (it was the `Ingredients` slider when this was measured; the family
was folded into `Foraging` before release, see the well entry below); the pot and the trade
props do have one, so their block is ignored. The survey printed those very drops live as generic `item_basic_cup` /
`item_basic_onehand` actors classifying as `Kind::Item`.

Note what this does **not** mean. Vanilla, the well and the pot pay the **same** (~5). The
well is not inherently richer - it is only reachable.

### The discriminator is not breakability, and it is not a name

**CONFIRMED against the bytes:** each of the 275 DMM-derived gather records carries a
`collect_<class>` tag and **the class agrees with the family the record is in 275/275, with
zero disagreements** - Foraging 82, Logging 141, Mining 36, Ore 16 - while **0 of those 17
records - the well and the 16 - carry any tag list at all**. (An earlier form of this
paragraph said "274 carry the bare `collect` tag, the 275th carries `collect_botany`";
`findings-water-wells` §9 withdrew it, and a predicate built on bare `collect` is a
different predicate from one built on `collect_<class>`. The well itself carries no tag,
which is why the tag cannot be the generator's rule.) Breakability is
**not** it either: `cave_break_stalac_02` and `gimmick_stalactite_0003`/`_0004` are breakables
sitting in `Family::Mining` with the working signature. Whatever a future "will a table edit
reach this record" predicate is built from, those two facts are the ones it has to survive.

### Where the count lives, and why the looter has already found it

The granted count is at **`instance+0x10`** - a runtime heap field on the `+0xC0` instance
hanging off the gimmick actor's `ClientGimmickActorComponent`, not a record field. It exists
only while the prop exists, which is what makes this a live write rather than a load-time one.

`desert-looter/src/actors.rs` **already locates that instance**, which is most of why this is
tractable at all. `Kind::Item` in `actors::classify` *is* the `comp+0xC0` case
(`actors::gimmick_info`, `desert-looter/src/actors.rs:365`), and `comp+0x48`
(`actors::GIMMICK_RECORD_INDEX_OFF`) gives the gimmickinfo row index - so the record's key,
its name and its `collect::Family` are all reachable **from the actor**. That is what would
let a multiplier be gated per family instead of applied to every item lying on the ground.

### Shape: a plugin-thread write, no hook and no code patch

The same shape as `desert-dispatch`'s parsed-record edits - walk, read, `safe::write`, patch
nothing in the image. That is the cheapest and lowest-risk write shape in this workspace, and
it is the reason this deserves costing out before any of the table-edit entries below are
built rather than after.

### Hazards, all of which belong in the design before the code

- **The player's own equipment rides on `+0xC0` instances too.** Multiplying those is a
  different mod and quite possibly a corrupt save. `desert-looter` already separates them -
  `Kind::Equipment` and `actors::refers_to_player` (`desert-looter/src/actors.rs:505`) - and
  this must reuse that check rather than re-derive it.
- **The write must be idempotent.** A dropped prop persists until it is taken, so a repeating
  pass over the same instance compounds the multiplier every tick. Writing once per instance
  pointer, or remembering the vanilla value per instance the way `desert-gatherer`'s
  `remember` does per record, is most of the actual difficulty of the feature.
- **`ItemInfo._maxStackCount` may clamp the result.** The field name is **CONFIRMED** in the
  exe (`python3 tools/fieldnames.py --grep maxStack`; `ItemInfo._applyMaxStackCap` sits
  beside it); that it clamps *this* path is **UNCONFIRMED**. A lever that silently tops out
  is worse than one that refuses.
- **Owned props.** Most of the trade props are market-stall goods, so this lever raises the
  size of a *stolen* pickup through the looter's take-or-steal check
  (`reference-internals.md` §14) - the same warning the trade-goods entry below carries, but
  reaching many more records.

### What it costs to find out

One measurement, and the survey has to be usable first (next entry). Read a dropped prop's
`instance+0x10` from the plugin thread, write `vanilla * n`, then pick it up with the
looter's `LogReceived=1` and read `[recv] item <key> x<count>`. That single result decides
the currency entry, the 16 inert records parked in the well entry, both halves of the
trade-goods entry and most of the unclassified-records entry - which is the argument for
spending a launch on it ahead of any of them.

## The looter's survey truncates at 64 lines, and that is what made this session hard

**Raised** 2026-09-12. **Status: RESOLVED 2026-09-13**, in 0.4.1's *Fixed*, taking two of the
three candidates below at once. `64` is gone: `[Looter] SurveyLines` (default 200, range 16..2000,
`desert-looter/src/config.rs` `SURVEY_LINES_RANGE`) is the budget, and `game::survey` orders the
listing **kind-first** (`survey_rank`: Gather, Unarmed, Item/Equipment, Interactable, Catchable,
Character/Player/Other, Inert) and by distance within a kind, so the budget can only ever cut the least
interesting actors. It also prints `[survey] listing capped at N lines ...: M more not printed -
<kind=count ...>` whenever it cut anything, which is the fix for the actual failure here - not the
truncation but the *silent* truncation. A far inert prop that prints nothing outside debug no
longer consumes a line. The census line was never truncated and is untouched. **Verified in game
the same day** (`analysis/logs/DesertTooling-2026-09-13-survey-lines.log`, `t=211.9`): 470 actors,
192 within 40 m, 192 listing lines in strict rank order - `Equipment 9, Item 22, Interactable 11,
Catchable 24, Character 4, Inert 122` - equal to the census, no cap line. And the capped case, same
log at `t=351.9`, `ScanRange=165` in Hernand: 699 in range, 200 printed - all 9 equipment, all 17
items, all 21 interactables, then 153 of 339 catchables - and `499 more not printed - Catchable=186
Character=39 Inert=274`. Nothing a gather or pickup question needs was cut. The third candidate
(let `Item`/`Gather` through *ahead of* the cap) is subsumed: with kind-first ordering they are the
first lines, so the cap reaches them last.

The argument as it stood, kept because it is the reason the default is 200 and not 64:

`desert-looter/src/lib.rs:560` passes a hard-coded `64` as `game::survey`'s `max_lines`, and
`game::survey` (`desert-looter/src/game.rs:156`) sorts candidates by distance from the player
before printing. Together those mean the printed lines are the **64 nearest actors**, and with
a heap of smashed pottery underfoot that is a window a few metres wide. The `Kind` histogram at
the foot of the pass is *not* truncated - it counts every actor inside `ScanRange` - but it
counts kinds, not records, so it cannot say which prop granted what.

**The saved log measures exactly how bad it was**
(`analysis/logs/DesertTooling-2026-09-12-water-pot-survey.log`, three passes at `ScanRange=40`):

| pass | printed lines | furthest line | `Item` printed | `Item` in the histogram |
| --- | --- | --- | --- | --- |
| 1 | 64 (the cap) | 8.6 m | 15 | **25** |
| 2 | 64 (the cap) | 3.1 m | 30 | **103** |
| 3 | 64 (the cap) | 5.9 m | 5 | **50** |

Every pass hit the cap, and the third printed **5** of the 50 `Item` actors it had counted.
Read the untruncated census instead of the listing and the population is plainly moving:
`Item` goes **25 -> 103 -> 50** across the three passes. Any claim of the "36 new actors" or
"5 cups for 5 water" kind was read off the 64-line window and counts the window, not the
world - **do not carry those numbers forward**; the histogram line is the only per-pass count
in that log that means anything.

That is the whole argument for the entry. The survey is the only instrument this project has
for the questions the instance-lever entry above asks, it is otherwise well designed for
them, and a hard-coded constant is what made it unusable.

The judgement is what to replace `64` with, and it is not simply "raise it": the log is one
file shared with the gatherer's hook, the dispatch scan and the looter, every line takes the
process-wide lock, and the budget exists for that reason. Candidates, cheapest first - an ini
key in `[Looter]`; keep the cap but let `Kind::Item` and `Kind::Gather` lines through ahead
of the rest; or sort kind-first rather than distance-first so the interesting actors are
never the ones cut. `desert-dispatch`'s `scan::Budget` is the existing precedent for a
per-pass line budget that is a setting rather than a constant.

## A currency multiplier: Silver, Gold and other coins

**Raised** 2026-09-12. **Status: DECIDED and built, 2026-09-13, shipped in 0.5.0.** Everything
from here to the Resolution at the bottom is the argument as it moved, kept as the record; the
2026-09-13 correction at the end of the recommendation is what the build follows.

### What is known

Item `1` is money, **CONFIRMED** (`findings-water-wells` §9). Exactly **six** records in the
clean table yield it (`analysis/items.json`, item `1`), and they split into two shapes that
matter for very different reasons:

| record | key | vanilla | shape |
| --- | --- | --- | --- |
| `gimmick_item_common_coin_0001` | `1000183` | `10..15` | placed prop |
| `gimmick_item_common_coin_0002` | `1006419` | `100..150` | placed prop |
| `gimmick_item_common_silverbar_0001` | `1000712` | `2500..2500` | placed prop |
| `gimmick_box_donation_reward_coin_01` | `1005717` | `150..250` | opened container |
| `gimmick_box_donation_reward_coin_02` | `1005718` | `270..450` | opened container |
| `gimmick_box_donation_reward_coin_03` | `1005719` | `500..1000` | opened container |

The denomination is the **quantity**, not a separate item, and this is **CONFIRMED from the
exe's own field names** (`python3 tools/fieldnames.py --grep money`): `ItemInfo._moneyTypeDefine`
→ `MoneyTypeDefine._unitDataListMap` → `UnitData{_minimum, _itemName, _moneyIconPath}`. Copper,
Silver and Gold are **display rows over one item**, selected by threshold. That answers the old
"which items are money" question in the sense that mattered: there is one money item and looking
for a second id for gold is looking for something the game does not model. The looter's own
`[survey] bag:` line corroborates it from live memory - the player's 11,993 coins sit in **one**
stack, printed with the game's own name for it, `Money_Copper key=1`
(`analysis/logs/DesertTooling-2026-09-12-water-pot-survey.log`).

### Why it is no longer the cheapest possible feature

This entry used to claim a `Currency` slider was "mechanically the cheapest possible feature" -
rows in `collect.rs` plus one multiplier, no hook and no change to `gimmick::multiply`. **Write
that lever today and it would be inert.** The three placed money props are exactly the shape
that the item-instance entry above proves does not read its `DropInfoData` block: an object
that hands over a pre-built `+0xC0` item instance. No money measurement was taken in game, so
this is **PLAUSIBLE** rather than CONFIRMED - and be precise about what backs the analogy, because
this entry previously overstated it: of the 16 records of that shape, **one item (the two pepper
records) is CONFIRMED IN GAME**, the pot is **PLAUSIBLE-strong**, and the remaining 13 are analogy
on the same mechanism. "CONFIRMED IN GAME for 16 records" is not a thing that was ever measured.
The argument is still the right way to bet, and the cost of being wrong the other way
is shipping a slider that silently does nothing to a player's economy.

**The real lever for the placed props is the item-instance one.** Any design here now begins
from that entry, not from `collect.rs`.

**The denomination model now has live evidence (2026-09-13).** The archived
`analysis/logs/DesertTooling-2026-09-12-2231-later-session.log` holds four grants of item id `1` at
four different counts - `x124`, `x23`, `x2`, `x1` - which is "one money item, denomination =
quantity" observed rather than inferred from the field inventory. It does **not** say which prop
paid them or whether a table edit would have moved them; it says the shape of the thing a
multiplier here would be multiplying is right. **A second source, 2026-09-13:** opening coin
pouches from the inventory gave `[recv] item 1 x120` and `x20`
(`analysis/logs/DesertTooling-2026-09-13-well-coin-bugs.log`, `t=301`, `t=310`) - so a
*consumable* also pays money as item `1` with the amount as the count. Moving a gold bar from a
chest into the bag logged nothing, as expected: a storage move is not a pickup. The coin-prop
versus silver-bar-prop test above is **still not taken**; pouches are a different source and do
not answer which placed props read their block.

The **three `gimmick_box_donation_reward_coin_*` records are a different shape** - opened
containers rather than placed props - and they are **UNCONFIRMED in both directions**: nothing
measured says a container reads its block, and nothing says it does not. The well is *not* a
precedent for them - it is a multi-step world gimmick, not a container, and its own evidence is
thinner than it looks (see the well entry below) - so do not reason "the well works, therefore
boxes work". Even so this is the one cheap thing here that could still come back positive, and
it is one pickup: open a donation box with the looter's `LogReceived=1` and compare
`[recv] item 1 x<count>` against that box's `150..250`, `270..450` or `500..1000`. If the
container shape reads its block, a block-edit `Currency` lever covering the donation boxes is
real and the placed coins wait for the instance lever.

### Why it is not just another family

These records live in the same `gimmick_item_*` space the withdrawn `Ingredients` family drew
from (the well entry below) and were **deliberately excluded from it**: a family defined as a
sweep of `gimmick_item_*` would silently become an economy mod. That exclusion was the reason
Ingredients was drawn up as a curated list rather than a prefix match (§3), and it is the reason
money gets its own lever or none at all. Whatever ships here must not be reachable by accident
from `Foraging` or any other gathering slider - and the same now applies to the instance lever,
which sees an actor's `collect::Family` and must not treat "no family" as "multiply anyway".

### The open questions

- **One slider or several?** Coins at `10..15`, coins at `100..150` and a `2500..2500` silver
  bar are three denominations of one item. A single `Currency` multiplier scales all three,
  which multiplies the *gap* between them as well as the amounts. Coins and bars as separate
  keys is the alternative, and costs an extra ini key and schema field. The donation boxes
  (`150..250` through `500..1000`) are a fourth axis again, and are quest and progression rewards
  rather than world pickups.
- **Is multiplying a `2500..2500` bar even wanted?** A fixed amount times three is a
  deterministic 7500, not a wider roll. That may be exactly right, or it may mean bars should
  be excluded and only the rolled denominations scale.
- **Does the `UnitData._minimum` threshold table react?** If Copper/Silver/Gold are chosen by
  crossing thresholds, a multiplied pickup may display as a different denomination than the
  vanilla one did. Harmless, but it will look like a bug to whoever sees it first.

### The thing that makes this different from every existing slider

A gathering multiplier changes how fast the player accumulates materials. A currency multiplier
changes prices, progression gates and every purchase decision in the game at once - it is a
**balance** lever, not a convenience one, and it is the first setting in this plugin where the
sensible default is arguably "do not touch this". That argues for a louder warning in the ini
text than the other families get, and for a default of `1` that is commented as deliberate
rather than incidental. None of that changed on 2026-09-12; only the implementation did.

### Recommendation as of 2026-09-12

Do not build this yet, and for a new reason. The old blocker was the item-to-family generator
bridge Ingredients was going to need - which was never built, and is not coming, since the
generator went record-keyed instead. The current blocker is that **the lever this feature needs
does not exist**.

**Corrected 2026-09-13, and this reverses the order this entry used to give.** The taxonomy's
bucket assignment (`findings-loot-taxonomy` §4, `what-we-found` §7.1 #1) says the three **placed
coin props are bucket E** - no `catch_*` preset, so the predicate says they *do* read their block -
while the three **donation boxes are bucket D**, predicted inert. This entry previously said "take
the donation-box pickup first"; that sends the launch at the one predicted *not* to pay. Take
**coin-versus-silver-bar** first instead: `gimmick_item_common_coin_0001` carries no tag list at
all and `gimmick_item_common_silverbar_0001` carries `catch_onehand`, so one pickup of each is a
direct test of the predicate, on the same trip.

**The coin half is taken (2026-09-13, `findings-water-wells` §13.7).** Three vanilla pickups of
`coin_0001` paid `x15`, `x14`, `x15` - all inside its `10..15` block, varying - against a bag
delta of exactly +44. Every instance-carrier ever dumped holds a count of 1, so a single prop
paying `x14` is the block being rolled: **coin_0001 reads its block, PLAUSIBLE-strong**, one
multiplied pickup short of CONFIRMED. Bucket E behaves as the predicate said. **The bucket-D
half is still untaken** - and it is not the silver bar: the player reports **no silver bar exists**
in the world (silver is a denomination of copper; only gold bars are items, and the gold bar is
item `53`, `findings-water-wells` §13.8), so `silverbar_0001` is a record with no known placement.
And the three `gimmick_box_donation_reward_coin_*` records are **not** a pickup either: the
donation box in the world is where the player *gives* items to the camp (three Honey trade goods,
2026-09-13); what comes back is `[recv] item 103 x1` - an id no `gimmickinfo` record pays, so a
token through the pickup path, not money and not a block roll - and the reward goes to the faction. So on the
money side there is **no known placed bucket-D prop to take**, and none is needed: bucket D's
"ignores its block" is already measured on the pepper stalls. What the money entry needed from
the silver bar was a *money* D-record, and there isn't one to find. Close that line of testing;
the coin props are the whole reachable currency surface.

**So the shape of this feature is now known for the three placed coin props:** the lever is the
block edit the gatherer already makes, reached live by `reapply`. What it needs is a family - call
it `Money` or `Currency`, its own ini key and slider - and the generator's money refusal lifted
**for that family only**. The refusal in `tools/gen-collect-names.py` exists so item `1` never
rides into `Foraging` by accident; it must stay for every other family. The three donation boxes
and the silver bar are bucket D and are not covered by this; they wait on the instance lever like
everything else in that bucket. Write the ini warning - "this multiplies money you find lying
around, and nothing else" - before the slider. The old closing line of this entry, "let this
feature follow the instance lever rather than lead it", is **withdrawn for the coin props**: they
do not need that lever.

### Resolution (2026-09-13)

Built as the coin-prop entry above describes, in 0.5.0: a fifth family `Money` in
`desert_core::collect` holding the three placed money records (`coin_0001` 10..15, `coin_0002`
100..150, `silverbar_0001` 2500..2500), from a `Money` block in `tools/extra-families.json`; a
`Money` key and slider under `[Gatherer]` with the same 1..100 range, reached by the existing
block edit and by `hook::reapply`; the generator's money refusal lifted for `Money` only and
made two-way (no other family may pay item 1, and no `Money` record may pay anything but item
1, both checked against the clean body). The three donation-box records are `records_not_enabled`
in that block and a generated test keeps them out. The open questions above settled as: one
slider, not several (three denominations of one item, and the gap between them scaling is the
price of one key); the silver bar stays a row, with its `why` saying nobody has seen one placed
and that the row is inert rather than wrong if it ignores its block; the display-threshold
effect is documented, not worked around. The looter side is `GatherMoney=0`: with a family the
coin props classify `Unarmed` rather than `Inert`, so `allows_family` needed an arm and the
switch is the honest one, off because a forged pickup at a coin has never been tried and
because a MINOR must preserve behaviour. The louder warning this entry asked for is in the ini,
the menu help and the README: money lying in the world, and nothing else. **Measured the same
day with 0.5.0 installed:** `Money=3`, one `coin_0001` by hand, `[recv] item 1 x30`, bag
`Money_Copper x11889` -> `x11919` between two F11 surveys with the taken coin gone from the
second and ten coin props listed `Unarmed family=Money` in the first. That is section 13.7's
PLAUSIBLE-strong turned CONFIRMED, and it was the live re-apply pass that wrote the 30.

## The water well is a `Foraging` record; the 16 inert records are the instance lever's payload

**Raised** 2026-09-12 as "the `Ingredients` family". **Status: DECIDED and LANDED, released in 0.4.1 on 2026-09-12.**
On branch `water-wells`. `docs/findings-water-wells-2026-09-12.md` §3 is the design as it was
first written and §11 is the mechanism that narrowed it; this entry records what was done about
it. Every other entry in this file was written before that decision landed; they were audited
against it on 2026-09-12 and now name `Ingredients` only where they describe what was measured
or what was withdrawn - never as something that still exists.

### What was decided

`Family::Ingredients` held 17 records and **exactly one worked**: the water well
`gimmick_well_0001_parts01` (key `1001081`), which at x3 paid **x15** against a vanilla 5,
CONFIRMED IN GAME. The other 16 - the water pot `Background_Breakable_66` (`21030076`) and the
15 `gimmick_item_trade_*` props - hand the player a pre-built item instance and never read
their block (the instance-lever entry at the top of this file; the pot's own sequence there is
**x3 -> 5**, **x1 -> 1** then **x1 -> 5**, **x10 -> 6**, **x100 -> 4**, flat against a block
rewritten 5 -> 500, out of
`analysis/logs/DesertTooling-2026-09-12-water-pot-survey.log`). A whole family for one record
was the wrong shape, and the maintainer's call was that **water from a well is a thing you
gather out of the world like everything else in `Foraging`, so it belongs in `Foraging`.**

**One caveat on the x15, and it belongs next to the claim rather than buried.** It is an
**on-screen count**, read off the game as it was played in a session whose log was later
overwritten. No saved log line names the well with x15; the pot survey log that survives covers
the pot, not the well. That single observation is the **sole** evidence that any non-DMM row
works at all - the whole of `Foraging`'s 83rd record rests on it. It is not in doubt and it is
not being downgraded here, but it is not re-readable either, so **re-take it the next time a
well is passed with `LogReceived=1` on**: one crank and one `[recv] item 22008 x<count>` line
puts it in a log for good.

**Half of that is now done (2026-09-13).** `analysis/logs/DesertTooling-2026-09-13-well-coin-bugs.log`
holds `[recv] item 22008 x5` from `gimmick_well_0001_parts01` at `Foraging=1` - the well pays its
block's `5..5`, logged. **And the other half, later the same day:** `Foraging=3`, two F11 surveys
either side of a hand-taken bucket, `Water x33` -> `x48` in the bag line - **+15, logged**
(`analysis/logs/DesertTooling-2026-09-13-well-x15-x30.log`, `t=272.621` / `t=319.063`;
`LogReceived` was 0 so there is no `[recv]` line, the bag count is the artefact). The caveat
paragraph above is now history. One more thing the run showed: the slider is read at the
**take**, not the fill - a bucket raised full at 3, then `Foraging=6` (`83 records rewritten, 0
skipped` at `t=323.176`), then taken, paid 30 on screen. Consistent with §11: the yield producer
rolls from the parsed record at grant time, and `reapply` had rewritten it.

So, before anything shipped:

- The `Ingredients` variant, ini key, schema field and `LiveConfig` slot are **gone**. No player
  ever had the key, so there is no migration and the changelog describes the end state.
- The well is a `Foraging` row (`collect.rs` 292 -> **276** records, Foraging 82 -> 83). The
  `Foraging` help text, the ini comment and the README say the family covers water from a well,
  because "plants, fruit, berries, mushrooms, crops" would never make a player guess it.
- The 16 inert records are **deleted from the table**, not left in it: a row for one is an edit
  the log reports and the player never sees. They are kept as `records_not_enabled` in
  `tools/extra-families.json`, each with its evidence, and the generator emits
  `foraging_records_not_enabled_stay_out`, which fails if any of them comes back as a row.
  That is the machine-readable trace; this file and `docs/` carry the argument.
- `tools/extra-families.json` is **record-keyed** now, not item-keyed. The item-keyed design
  ("water is water regardless of source") was the premise §11 refuted: item `22008` comes from
  two records and only one of them reads its block, so *which record* is what decides whether
  a table edit reaches anything. The generator verifies each record's name and the items it
  pays against DMM's clean body on every run, refuses a record that pays item 1 (money), and
  refuses a `records` entry that owns no output block.

### What is still true, and where it went

- The **instance lever** (top of this file) is what would reach the 16. When it lands, the
  family it gates on is `collect::Family`, and the 16 will need rows again - which is exactly
  why they are stored with their item ids in `records_not_enabled` rather than forgotten.
- Owned props still need a word in the ini text *when* that happens: many
  `gimmick_item_trade_*` records are market-stall goods, so raising them through the instance
  lever raises a *stolen* pickup through the looter's take-or-steal check
  (`reference-internals.md` §14). Inert today; that text is owed on the day they go live.
- The pass that measured all this also **confirmed the plumbing**: `hook::reapply` reported
  **0 skipped** on every slider change that survives in a saved log - seven `[live] re-applied`
  lines in `analysis/logs/DesertTooling-2026-09-12-water-pot-survey.log`, each of them
  `17 records rewritten, 275 unchanged, 0 skipped; 34 scalars written` - which is what validates
  the `BLOCK_ITEM = 0x68` correction. (The count of those lines was given as "nine" before this
  file was audited on 2026-09-12; seven is what the log holds, and the 17/275 split is the
  pre-removal table.) No native test can reach the `reapply` path, so that pass is the only
  evidence the correction has, and it is strong.

## Can the looter drive a multi-step gimmick? The well is the test case

**Raised** 2026-09-12. **Status: RESOLVED 2026-09-13 - NO, and it is worse than the exploit this
entry feared.** Tried in game, build 25246367, 0.4.1 draft
(`analysis/logs/DesertTooling-2026-09-13-well-coin-bugs.log`, `t=197.084`-`197.206`): F9 at a well
found `gimmick_well_0001_parts01 (Foraging, UNARMED)` at 2.3 m, forged the pickup, the player
received `[recv] item 22008 x5`, the actor was gone 0.1 s later - and **the well never produced
another bucket**. The crank sequence is what respawns it; a pickup delivered from outside that
sequence leaves the well permanently empty. So the answer to the question in the heading is no:
the looter cannot drive a multi-step gimmick, and firing at one step of it breaks the object.

**Resolution.** `desert-looter/src/config.rs` `WELL_BUCKET_RECORD_KEY` / `never_forge_pickup`,
checked in `nearest_gather` **before** `allows_family`, as this entry's last paragraph said the
fix should be shaped - a record-key refusal, not a family carve-out. The F9 line reports the
refusal; the `GatherForaging` help and the README say the switch never covers the well; a test
holds both halves (Foraging for the multiplier, refused for the looter). Changelog 0.4.1 *Fixed*.

**Two things the same run settled as a side effect.** (1) The well pays through its block:
`[recv] item 22008 x5` at `Foraging=1` is the **first well receipt ever logged**, and it matches
the block's `5..5` exactly - so the well row's evidence is no longer a single unlogged count,
though the multiplied case (`Foraging=3` -> 15) is still unlogged. (2) It arrived as **one `x5`
grant**, not five `x1`s, which is a second counterexample (after the 22:31 log's money and
colony grants) to "a block's total arrives as N separate `x1` pickups" - that model is
Foraging-bush behaviour, not a rule.

The text below is the entry as it stood before the test, kept because its reasoning about
*where* the fix goes was right.

**Status before the test: UNCONFIRMED / untested.** Nobody had tried it.

Folding the well into `Foraging` had a side effect on Desert Looter that was chosen
deliberately rather than left to fall out of the enum: `desert-looter/src/config.rs`'s
`allows_family(Family::Foraging)` is `GatherForaging`, which defaults to `1`, and
`actors::classify` returns `Kind::Gather`/`Kind::Unarmed` for any actor whose record has a
family. So with `AutoGather=1` the looter will now target the well's `parts01` record. The
comment on that arm carries the reasoning; this entry carries the test.

**The facts.** Drawing water is a stateful four-step sequence, not a pickup: interact to lower
the bucket, interact again to crank it back up, the water then sits **in the bucket** as its own
interactable, interact once more to take it. `gimmick_well_0001_parts02` is the crank
(`GIMMICK_CRANK_DIALTURN_WELL_START` / `activity_wellcrank`); `gimmick_well_0001_parts01` is the
filled bucket - the record with the `DropInfoData` block and `GIMMICK_WATER_PICKUP`, the one
`collect.rs` classifies. The looter forges exactly one `PickUpItem` event per node
(`events.rs`); it cannot lower a bucket or turn a crank. So the intended behaviour is that the
player still cranks and auto-gather saves the final interact, as it does at every other
Foraging node.

**The QoL claim to confirm.** `AutoGather=1`, `GatherForaging=1`, crank a well: the water should
arrive without the final interact, and the `[looter]` log should show one gather at
`gimmick_well_0001_parts01`.

**The failure mode to rule out.** If `parts01` classifies as a gather node **before** the bucket
is raised, the looter could fire at it and grant water with no crank at all - a duplication
exploit that would ship untested. The cheap check is the looter's own survey: stand at an
untouched well, press F11, and look for `gimmick_well_0001_parts01` in the output; then crank
and press F11 again. If it only appears after the crank, the QoL behaviour is safe. If it is
there before, the fix is a narrow exclusion of that one record key in `nearest_gather`
(`desert-looter/src/game.rs`), not a family-wide carve-out, and this entry is where to say so.
(The survey caps at 64 distance-sorted lines, so stand close.)

**The multiplier is independent either way.** `Foraging` edits the record's block at load time
and never consults the looter; whatever this test finds, x3 at a well pays 15 - subject to the
provenance caveat on that measurement in the well entry above.

## Trade goods: 29 records that are not one line each, and 397 that are a different problem

**Raised** 2026-09-12. **Status: OPEN, and re-costed on 2026-09-12.** Came straight out of the
in-game Ingredients test: a water well at `Ingredients=3` gave **x15** where vanilla is 5
(`findings-water-wells` §2, CONFIRMED IN GAME - that was the key's name at the time; the well
is a `Foraging` row now and there is no `Ingredients` key), and the next prop picked up was a
**"Trade Good" of Ceramic**, which the mod has no way to reach at all.

### What this entry used to say, and why it was wrong

It said the 29 block-carrying trade props were **"one line each in `tools/extra-families.json`"**
with "nothing mechanical" unknown, against 397 collection props that were the hard half. **That
is refuted.** Fifteen records of *exactly that shape* - the `gimmick_item_trade_*` edibles -
were rows of `Family::Ingredients` on this branch and are **inert, CONFIRMED IN GAME**:
`gimmick_item_trade_pepper_01`/`_03` were both patched `1->3`, both logged, and the player
received `[recv] item 1000608 x1`. Pepper has no other block-carrying source record, so nothing
else can account for that. That family was withdrawn before it ever shipped and those 15 are now
`records_not_enabled` in `tools/extra-families.json`, held out of the table by
`collect.rs`'s `foraging_records_not_enabled_stay_out` (the well entry above). Twenty-nine more
lines of the same kind would buy 29 records of nothing.

So the two groups are **not** a cheap half and an expensive half any more. Both of them need
the **item-instance lever** (first entry in this file), and neither is a table edit:

| group | records | output blocks | what it needs | status |
| --- | --- | --- | --- | --- |
| block-carrying trade props | **29** `gimmick_item_trade_*` | one each, all `1..1` | the instance lever; a `collect.rs` row is only the family gate | **PLAUSIBLE** inert - same shape as 15 proven inert, untested |
| collection props | **397** header-shaped `gimmick_collection_prop_*` / `Collection_Prop_*` | **none, of any shape** | unknown; probably the instance lever too | **UNCONFIRMED** how they grant an item at all |

The Ceramic the player picked up is in the **second** group. The research below is still good
and is kept in full - the enumeration, the naming, the dead hypotheses and the collection-prop
scan are all unaffected by the refutation, which changed only what the records are worth
*editing through*.

### Group one: the 29 block-carrying records

44 `gimmick_item_trade_*` records carry an output block. 15 of them - the edible ones - are the
records the withdrawn `Ingredients` family briefly held, **and all 15 are the ones now proven
inert**. These are the other 29, 23 distinct items, every one a single-block `1..1` list.
Mechanically they are identical to the 15, which is precisely the problem: the same table edit
that did nothing for pepper would do nothing for jade. They remain worth enumerating because the
instance lever needs a list of records to gate on, and this is that list.

**Item names below are inferred from the names of the records that yield them**
(`tools/items.py`, `findings-water-wells` §3), not read from the game's string table. `strong`
means two or more records agree, `single` means one record names it and nothing contradicts,
`guess` means a container named it.

**Inference is no longer the only route, as of 2026-09-12.** The looter's `[survey] bag:` line
prints the game's **own** item names out of live `iteminfo` - 77 distinct `name key=<id>` pairs
across the three passes in `analysis/logs/DesertTooling-2026-09-12-water-pot-survey.log` - which
names any id the player happens to be carrying with no `.paz` extraction at all (see that entry
below, which is now narrower than it was). Three ids in the table below are already settled that
way: `1000612` is `Trade_Sulfur_02` (inferred `brimstone` - right in substance, wrong in
wording), `1000615` is `Trade_Gunpowder_02`, `1000643` is `Trade_Fur_01`. Carrying one of each of
the other 26 past a survey keypress would name the whole group in a single pass, and that is the
cheapest way to turn this table from inference into data.

| record | key | item | inferred name | conf |
| --- | --- | --- | --- | --- |
| `gimmick_item_trade_Brimstone_01` | `1001169` | `1000612` | brimstone | `strong` |
| `gimmick_item_trade_Brimstone_03` | `1001180` | `1000612` | brimstone | `strong` |
| `gimmick_item_trade_jade_01` | `1001134` | `1000674` | jade | `strong` |
| `gimmick_item_trade_jade_02` | `1001172` | `1000674` | jade | `strong` |
| `gimmick_item_trade_pot_01` | `1001147` | `1000619` | pot | `strong` |
| `gimmick_item_trade_pot_02` | `1001148` | `1000619` | pot | `strong` |
| `gimmick_item_trade_sword_01` | `1001136` | `1000671` | sword | `strong` |
| `gimmick_item_trade_sword_02` | `1001174` | `1000671` | sword | `strong` |
| `gimmick_item_trade_wool_01` | `1001119` | `1000661` | wool | `strong` |
| `gimmick_item_trade_wool_02` | `1001154` | `1000661` | wool | `strong` |
| `gimmick_item_trade_armor_01` | `1001137` | `1000672` | armor | `single` |
| `gimmick_item_trade_armor_wild_chain_03` | `1003195` | `1000703` | armor_wild_chain | `single` |
| `gimmick_item_trade_armor_wild_fabric_03` | `1003193` | `1000702` | armor_wild_fabric | `single` |
| `gimmick_item_trade_calligraphypainting_01` | `1001152` | `1000598` | calligraphypainting | `single` |
| `gimmick_item_trade_carpet_02` | `1001151` | `1000597` | carpet | `single` |
| `gimmick_item_trade_cigarette_01` | `1001163` | `1000607` | cigarette | `single` |
| `gimmick_item_trade_fur_01` | `1001121` | `1000643` | fur | `single` |
| `gimmick_item_trade_glass_01` | `1001114` | `1000596` | glass | `single` |
| `gimmick_item_trade_golden_sword_01` | `1003197` | `1001170` | golden_sword | `single` |
| `gimmick_item_trade_gun_01` | `1001138` | `1000618` | gun | `single` |
| `gimmick_item_trade_gunpowder_01` | `1001135` | `1000615` | gunpowder | `single` |
| `gimmick_item_trade_ivory_02` | `1001171` | `1000670` | ivory | `single` |
| `gimmick_item_trade_silks_02` | `1001156` | `1000600` | silks | `single` |
| `gimmick_item_trade_wine_02` | `1001161` | `1000647` | wine | `single` |
| `gimmick_item_trade_Food_02` | `1009063` | `1001240` | food | `single` |
| `gimmick_item_trade_Stone_02` | `1009065` | `1001801` | stone | `single` |
| `gimmick_item_trade_Weapon_02` | `1009066` | `1001802` | weapon | `single` |
| `gimmick_item_trade_Wood_02` | `1009064` | `1001799` | wood | `single` |
| `gimmick_item_trade_relicbox_02` | `1003154` | `1000638` | relicbox | `guess` |

Three things in that table are decisions, not data:

- **It cannot be a prefix sweep**, for the same reason Ingredients was drawn up as a curated
  list rather than a prefix match (§3): `gun`, `sword`, `golden_sword`, the three `armor*`,
  `gunpowder`, `Weapon`, `Wood`, `Stone` and `Food` are not trade goods in any player-facing
  sense, they are props that happen to sit in the `gimmick_item_trade_*` namespace. A
  `gimmick_item_trade_*` match would also sweep back in the 15 edible records that were
  deliberately taken out of the table.
- **`wine` (`1000647`) sits in `candidates_not_enabled`** in `tools/extra-families.json`,
  where it was left as taste rather than fact when Ingredients was withdrawn - and the note
  there now gives a second reason to leave it alone: it is a market-stall prop of the same
  inert shape as the 15. So it is not a choice between two families any more, it is one more
  record waiting on the instance lever. What still has to be decided, and decided here rather
  than in two candidate lists, is which side it lands on when that lever exists.
- **Every one of the 29 is a market-stall prop, so they are owned.** The well entry above
  records this as something needing "a word in the ini text"; here it is the whole family.
  Raising these raises the size of a *stolen* pickup through the looter's take-or-steal
  check (`reference-internals.md` §14), and a player who moved a trade-goods slider did not
  ask to steal more per theft.

Capacity is not a constraint: `collect.rs` holds **276** rows against
`desert-gatherer/src/remember.rs`'s `MAX_RECORDS = 512`, and 29 more is 305 (the count was 292
before the 16 inert records came out on 2026-09-12). That was the
answer when these rows were going to drive `gimmick::multiply`; it stays true for rows that
exist only to give the instance lever a family to gate on, and the 397 in group two are the
population that would actually test the ceiling (see the unclassified-records entry below).

### Group two: collection props, and the two hypotheses that are now dead

**CONFIRMED against the bytes, and deliberately attribution-independent**: all **397**
header-shaped `collection_prop` names in DMM's clean table body, each spanned to the next
header-shaped name anywhere in the body, contain **zero** 64-byte block-shaped structures.
Not one `_dropInfoDataList` block, and not one free-standing block either. `tools/items.py`
agrees from the other direction - neither the shipped detector nor `--loose` attributes a
single output list to any of them. (The scan does not depend on the key-echo discriminator
on purpose: only 164 of those 397 names pass it, so an echo-scoped answer would have been
proving something about a subset. §7's false-header trap is why that mattered.)

That kills both of the cheap hypotheses:

- **`rec+0x288` is not it, twice over.** §16's optional single block that "the raw scanner
  has never touched" is **absent from these records on disk**, and separately
  `FUN_141775790` - the gather/pickup yield producer of §19.6.1 - **never reads `rec+0x288`
  at all**: its two loops are `rec+0x268` and `rec+0x278`/`+0x280` and there is no third.
  **CONFIRMED** by decompile. Ghidra project addresses are build **25116796**; the installed
  exe and the clean body are **25246367** and the shift is not uniform, so read that
  function name as structure, not as an installed-build address.
- **`rec+0x268` is not ruled out, but nothing supports it.** Finding a `_dropSetInfoList`
  inside a record that has *no* output list needs a field-accurate sequential parse of the
  record, which nothing in this project does and which the tail-offset-from-the-next-header
  shortcut cannot substitute for (72 distinct distances over 573 lists - the tail is
  variable-length). Settled on the way, and worth carrying to the second-yield-route entry
  below rather than restating there: the `u32` immediately before the output-list count is
  `0` on **573/573** lists, read directly at each list offset.

**The best remaining lead is `GimmickInfo._convertItemInfo`, and it is PLAUSIBLE.** The
field is in the exe (`python3 tools/fieldnames.py --grep convert`, so the *name* is
CONFIRMED), and it is **scalar - no list, no min, no max**. Its sibling
`CharacterInfo._convertItemInfo` is the same shape, and §17 already established how that one
behaves: the record names the item and **the amount is a hard-coded immediate in code**, the
`mov r8d,1` that `desert-gatherer/src/catch.rs` patches. If a collection prop grants the same
way, **there is no scalar anywhere for a multiplier to scale** and the only lever is a second
code patch.

**Static analysis on its own will probably not settle this, and saying so is the honest
answer.** Neither `_convertItemInfo`'s record offset nor its reader has been located, and
§19.9's name-to-offset procedure has never been run for a single row. This project has had
static analysis falsified more than once (§7; `reference-internals.md` §20.20), so the
mechanism stays UNCONFIRMED until a launch says otherwise.

### What one game launch settles, at zero cost and with no code change

Both lines are already in the log today:

1. **Pick up a Ceramic collection prop with the looter's `LogReceived=1`.**
   `[recv] item <key> x<count>` gives the item id and the amount in one line. A flat `1` on
   every collection prop is what the `_convertItemInfo` hypothesis predicts.
2. **Watch for `[catch] class=XX not a known bug/fish class; vanilla (logged once per
   class)` at the same moment.** `catch.rs` emits that for any class byte that reaches the
   catch-count site. If it appears, the grant runs through the site the plugin **already
   patches**, and covering collection props is a class byte in `desert-core/src/creature.rs`
   rather than a new hook. If nothing appears, it does not, and the mechanism is still open.
3. **Read what the looter's survey classifies the prop as, in the same pass.** If the Ceramic
   comes back `Kind::Item` it carries a `+0xC0` instance and belongs to the item-instance
   entry at the top of this file rather than to a mechanism of its own - which would fold most
   of group two into one open question instead of two. The survey's 64-line truncation (entry
   above) is what has to be dealt with before this line is readable.

Worth one extra look in the same launch: **whether the Ceramic yields item `1000619`.** The
nearest block-carrying relative is `gimmick_item_trade_pot_01`/`_02`, which yields `1000619`
(inferred `pot`, two records agreeing), and the Ceramic props' own activity field reads
`catch_pot`. If they are the same item then one id is reached by two records of two different
shapes, and **that is an argument for keying this family by record, not by item** - the same
shape as item `22008`, where the well reads its block and the pot does not. Note which way that
cuts: the item-keyed "water is water regardless of source" premise Ingredients was first
designed around (§3) is the one §11 **refuted**, and `tools/extra-families.json` is record-keyed
today because of it. A `TradeGoods` key that named item `1000619` would claim both the stall pot
and the ceramic one; naming records claims neither by accident. (Under a table edit it would
reach neither anyway - group one is PLAUSIBLE inert - so this decides what the *instance lever*
is allowed to touch, not what a multiplier scales.)

### Does `TradeGoods` belong with the gathering families? No - and not with the currency slider either

- **Not a gathering family.** Trade goods exist to be sold. A player who sets `Foraging=3`
  is asking to accumulate materials faster; `TradeGoods=3` triples sale income. That is the
  currency entry's axis at one remove - a **balance** lever, not a convenience one - and it
  earns the same treatment that entry argues for: a default of `1` commented as deliberate,
  and a louder warning than the gathering families carry.
- **Not folded into `Currency` either.** That entry is deliberately scoped to item `1`, and
  the gap between the two is real: coins are income immediately, trade goods are income only
  after the player carries them to a buyer. One key covering both would be one key a player
  would plausibly want set two different ways.
- **So: its own key** - one `[Gatherer]` key, one schema field, and a curated **record** list
  in `tools/extra-families.json` (the item-to-family generator bridge that was going to expand
  an item list into records was never built, and the well decision made the store record-keyed
  instead; see that entry). What has changed is what the key *drives*: not `gimmick::multiply`
  over a table edit, but the item-instance lever, with the family only deciding which actors it
  may touch. The ini text has to say two things, not one: these are stall goods and the steal
  check applies, and this is an economy lever.

### Recommendation as of 2026-09-12

**Take the game launch first**, and take it for group two - it costs one pickup, changes no
code, and it is still the only thing that separates "collection props are a family like any
other" from "collection props need a second code patch".

Group one is no longer cheap and must not be built as a table edit. Its 29 rows are worth
keeping as a **list** - the instance lever needs a set of records to gate on and this is it -
but writing them into `tools/extra-families.json` as live `records` and calling the feature
done would ship 29 records of nothing, which is exactly what happened to the 15 edible ones.
If they go into that file at all before the lever exists, they go in as `records_not_enabled`,
which is the slot the 15 now occupy and the one the generator's test polices. Order of work:
the instance lever, measured; then group one and the currency entry decided **together**, as
one pair of economy levers argued once rather than two levers argued separately and worded
inconsistently.

## The second yield route the gatherer multiplies nothing in

**Raised** 2026-09-12. **Status: OPEN.** `findings-water-wells` §6.

`FUN_141775790`, on the gather/pickup path, runs **two** yield loops back to back. The mod
scales the first and is blind to the second:

| route | list | amount at | gatherer touches it |
| --- | --- | --- | --- |
| output blocks (`reference-internals.md` §16) | `rec+0x278` ptr, `+0x280` count | `block+0x20` min / `+0x28` max | **yes** - the whole mod |
| drop sets | `rec+0x268` ptr, `+0x270` count | `dropsetinfo entry+0x20` / `+0x28` (§20.22) | **no** |

### Why this is worth writing down even though nothing is broken today

`reference-internals.md` §19.6 proves the gatherer's **hook** coverage is complete: no
`gimmickinfo` record is parsed without passing through the loader hook. It does **not**
prove its **multiplier** coverage is complete. A gimmick whose payout comes through
`rec+0x268` passes the hook, is remembered, and is then scaled by nothing - silently, with
no log line and no failure. That distinction belongs next to §19.6, because §19.6 is
exactly the claim someone will read as "the gatherer sees everything".

### Confidence, and what it costs to settle

**Both were settled on 2026-09-12 and the entry stays open for a different reason.**

The route is **CONFIRMED**: `FUN_141775790` was re-decompiled independently through the
Ghidra MCP and reads `param_1+0x268`/`+0x270` and `param_1+0x278`/`+0x280`, walking each
list with the consumers named above - and reads no other offset off the record at all.

The census is **CONFIRMED** too: the `u32` immediately before each of the 573 output-list
counts, read directly at the list offset with no record attribution involved, is zero
**573/573**. No record carrying an output list also carries a drop-set list.

What that does *not* settle is the part that matters, and it is why this stays open: the
two routes are disjoint *among records that have an output list*. It says nothing about the
~13,300 records with **no** output list, which is exactly where a `rec+0x268`-only gimmick
would live - and the 397 collection props in the trade-goods entry above are precisely that
shape. Multiplier coverage is now measured complete for the records the plugin can see, and
still unmeasured for the ones it cannot. `reference-internals.md` §19.6.1 carries the same
split.

## The unclassified records: 297 carrying 308 blocks

**Raised** 2026-09-12 as "the 298 unclassified records / 309 blocks". **Status: OPEN.**
`findings-water-wells` §7 has the census and why three different numbers were reported for it.

The clean table body holds **573 output lists / 896 blocks** (CONFIRMED; greedy and
exhaustive scans agree exactly). The DMM pack covers **275 records / 587 blocks**, and since
2026-09-12 `collect.rs` adds the water well, so the table covers **276 records / 588 blocks**
(`analysis/items.json` `totals`, regenerated after the change). So **297 records carrying 308
blocks are classified by nothing** - containers, breakables, market props, dried food, donation
boxes. (The 298/309 in the heading this entry was raised under, and in
`findings-water-wells` §7, is the pre-well figure; one record and one block moved, nothing
else did.)

### The question

Which of those deserve families, and which are deliberately left vanilla? The money records
in the currency entry are the proof that "sweep the rest into a residual bucket" is the wrong
answer: a blanket family over `gimmick_item_*` would have quietly multiplied the player's
income. Each candidate group needs the treatment Ingredients ended up with - a **record**-keyed
list in `tools/extra-families.json`, each row carrying its items, its evidence and a
generator check against DMM's clean body - plus, now, an answer to whether a table edit reaches
the record at all before a row is written for it. Ingredients was the worked example of what
happens when that last question is skipped: 16 of its 17 rows were inert.

### The capacity ceiling to check before growing anything

`desert-gatherer/src/remember.rs:43` caps the table at `MAX_RECORDS = 512` against **276** rows
today. Adding a handful of rows is free; acting on 297 records is not. It is a
**refusal**, not a truncation - over-running it leaves the excess records vanilla with no
crash, so the failure mode is silent, and the ceiling has to be raised *before* a large
family lands rather than after someone notices half a family doing nothing.

## Digging: a gathering activity the mod cannot even see

**Raised** 2026-09-12. **Status: OPEN.** `findings-water-wells` §7 (the 16 lists the pad
clause excludes). This came out of the off-by-4 fix, not out of looking for it.

The game has a dig interaction and the table has records for it. The mod does nothing with
them, and **not for the usual reason** - this is not another unclassified record like the
entry above. `desert-core`'s output-block detector **cannot find their blocks at all**, so
adding a `collect.rs` row for a dig site today would multiply nothing.

### What is there

Two records carry output lists, both CONFIRMED by direct scan with the key-echo attribution
(§7) rather than a backwards header search:

| record | key | list rel | items |
| --- | --- | --- | --- |
| `Action_dig_01` | `120040092` | `+614` | `1000629` `1..1`, `200049` `1..1` |
| `gimmick_Dig_land_0001` | `1005245` | `+613` | `1002728`, `1002729`, `1002730`, `1003282`, `1001752`, each `1..1` |

`gimmick_Dig_land_0001_parts01` (key `1005244`) exists but carries **no** output list - worth
noting because the well went the other way, where `_parts01` was the payout and the parent
record was inert. Do not assume the `_parts` convention holds.

### Why the detector is blind to them

`block_ok` applies two equality clauses: the item id at `+1` against its echo at `+60`, and
the two zero pads at `+5` and `+64` against each other. Both dig lists satisfy the item check
and **fail the pad check** - their blocks carry a nonzero `u32` at `+64` where `+5` is zero.
Dropping the pad clause grows the detector from 573 lists / 896 blocks to 589 / 1038, and
these two lists are among the 16 it adds.

That clause is deliberate and is **not** simply wrong: keeping it is what held the detected
population constant across the off-by-4 correction, so that fix provably changed no yield
(`desert-core/src/gimmick.rs`, `block_ok`, carries the reasoning inline). The open question
is whether the 16 excluded lists are a *different kind of output block* the mod should learn
to read, or malformed data it is right to skip.

The evidence cuts both ways and that is why this is not decided:

- **For reading them:** the records are real and coherent - dig sites, `Temple_Chest_01`
  (`16060036`), `dff_chest_24` (`16060043`), `clawmachine_capsule_01` (`1007407`),
  `gimmick_item_dropset_treasurebox_01` (`1005213`), the `gimmick_abyssone_bridge_gate_*` set,
  `gimmick_marni_teleportation_*`. Not one is a false-header artefact; not one is among the
  275 DMM gather records.
- **Against:** `treasurebox_01` lists **9** item ids in the `391518521`..`391518546` range,
  implausible beside everything else - every id in the shipped population is eight digits or
  fewer, the largest being `10021720` (`goldbar`, legitimate). All nine sit in that one list,
  so at least it is being mis-parsed and the layout is not simply "the same block with a
  dirty pad". Note the bound is 99,999,999, not "seven digits": a tighter one accuses
  `goldbar`, which the plugin already trusts.

### `+64` is a drop-group key, and groups are shared between records

**This is the finding that most changes the entry, and it is CONFIRMED against the bytes.**
`+64` is not a pad, not the high half of a wide item id, and not a variant-tag artefact - all
three were checked and ruled out. It is the **list entry's own key**, read by the list loop
`FUN_1414a7cc0` into `entry+0x08` (`docs/reference-internals.md` §16), and **items sharing a
value form a group**:

```
Temple_Chest_01 : 1000953 1000981 1000872 1000985 1000992      all entry_key 1992030708
dff_chest_24    : 800174 1002757 | 1003283 | 1001751 1001751 | 1002802 1001027 | <that run>
                  2437171491       1388160556  4159428909      280763949         1992030708
```

`Temple_Chest_01`'s whole content is one group, and that group appears as a **contiguous run**
at the tail of `dff_chest_24` - same ids, same order, same key. `dff_chest_24` is five groups
concatenated. The five `gimmick_abyssone_bridge_gate_*` records carry byte-identical lists, as
do the two `gimmick_marni_teleportation_*`, and 14 keys occur in more than one record.

**Structure that regular is not what a misparse of arbitrary bytes produces**, which is real
evidence for reading these blocks rather than skipping them - and it points somewhere specific:
a shared, reusable loot group is exactly what the `rec+0x268` → `dropsetinfo` route in the
second-yield-route entry above describes. **Check whether these `entry_key` values are
`dropsetinfo` keys or hashes of them before treating this as a separate mechanism.** If they
are, the two entries are one problem.

What this does *not* settle is whether the amounts and ids inside those lists were read
correctly. `dff_chest_24` lists `1001751` twice with the same amount and the same key, and a
genuine repeat and a misparse are indistinguishable from the bytes alone.

### What already exists to work with

`tools/items.py --loose` walks the 589/1038 population and writes
`analysis/items-loose.json` + `docs/reference-items-loose.md`, kept strictly separate from the
default pair so a loose run cannot touch what the mod actually sees. Every item carries a
`visibility` of `shipped` or `loose-only` (96 are loose-only), every block its `+64`, and the
9 implausible ids are surfaced rather than filtered.

The seven dig ids **cannot be named from this table**. Each is named by exactly one record, and
that record names the *action* - `Action_dig_01` yields `action_dig` for two ids,
`gimmick_Dig_land_0001` yields `dig_land` for five. All seven are graded `guess` for that
reason. Naming them needs the `.paz` item names (entry below).

### Before building anything

One in-game read. Dig with the looter's `LogReceived=1` and see whether the items granted match
the ids above at all. If they do not, these lists are not the dig payout and the entry collapses
to "the detector is right to skip them". If they do, the question becomes whether to teach
`block_ok` a second accepted shape - which must be done without moving the 573/896 population
the plugin sees today, since holding that constant is what proved the 2026-09-12 off-by-4
correction changed no yield.

## Extracting item names and the category triple from the `.paz` archives

**Raised** 2026-09-12. **Status: OPEN, deliberately deferred.**
`findings-water-wells` §4.

Item **display names** and the game's own `CategoryInfo` main/middle/sub triple live in the
`categoryinfo` / `stringinfo` / `localstringinfo` table bodies inside the `.paz` archives.
Getting the **triple** out is what would let a family be **populated from the game's own data**
instead of curated by hand - the right answer to the general "which items belong together"
question, of which the withdrawn Ingredients family was one instance.

**Narrowed on 2026-09-12: names no longer need any of this.** The looter's `[survey] bag:`
line prints the game's own name for every stack the player is carrying, read live out of
`iteminfo` - 77 distinct `name key=<id>` pairs in
`analysis/logs/DesertTooling-2026-09-12-water-pot-survey.log`, e.g. `Money_Copper key=1`,
`Water key=22008`, `Trade_Sulfur_02 key=1000612`. So "extraction is what would settle an item's
name" is **no longer true**; carrying the item and pressing the survey key settles it. What
still needs the archives is the **`CategoryInfo` triple** (and localized display strings), which
no running structure this project reads exposes - and the triple is the half that was actually
wanted here, since it is what would define a family without a human curating one.

It is deferred because it is not on the critical path: §3's record-name inference plus the live
names cover current needs, and the inference cross-validates rather than guessing (a yielding
record's name identifies its item - CONFIRMED by 79 multi-record agreements, `leather` across
nine separate records). This is a separate piece of work with its own tooling, and shipping a
family does not wait on it.

Two things to know before starting, both of which have already cost time:

- The archives are roughly **31 GB under `0000/`**. This is an extraction pipeline, not a
  grep.
- **`pathc_clean.bin` is a DDS thumbnail cache, not a table body.** Someone already lost
  time treating it as one. DMM's `backups/` holds exactly one usable table body,
  `gimmickinfo_pabgb_clean.bin`; `papgt_clean.bin` is 679 bytes of `(index, hash, flags)`.
  Nothing loose under the install names items.

Note also that all category and group **display names** are **UNCONFIRMED** for exactly
this reason - the group names quoted in §4 (`ItemGroup_Food`, `ItemGroup_Trade`, …) are
literals the code references, not names read out of a table. The live `iteminfo` names above do
not touch that: they name individual items, never a category or a group.

## Drop *rate* levers: the collect node is a dead end, the buff system is not

**Raised** 2026-09-12 as "`GimmickEventHandlerData_SetAdditionalCollectDropRate`, OPEN lead,
unexplored". **Status: the original lead is CLOSED; a larger one it uncovered is OPEN and
cheap.** Explored 2026-09-13 - `docs/findings-drop-rate-levers-2026-09-13.md` is the record,
with the addresses, the decompilation and the commands.

### The original lead, closed

The string is real but **the address in the old form of this entry was wrong**: `0x1469dec70`
holds the tail of an unrelated mangled RTTI name. On build 25246367 the class-name literal is
at VA `0x1456A5EB0`. **No code references it.** It is entry **195** of a 208-name
`GimmickEventHandlerData_*` array at `0x56AF0D0`, indexed by a `u16` - i.e. a **gimmick-chart
node type**, not a record field. Its execute function (`0x1423491B0`) copies 8 bytes from the
node into the *gimmick instance* at `gimmick+0x1A0`.

Three things follow, and together they close it:

- It is **per-instance and chart-driven**. There is no parsed record to edit, nothing resolved
  at load time, and no static table - so this is **not** the shape the gatherer's existing
  write path can reach. Reachability was ranked honestly at ~65% "event/script state the mod
  cannot touch", ~30% "needs a hook on `0x1423491B0`", ~5% "a writable record field".
- **`fieldnames.json` does not contain it, and that is the informative part.** No
  `GimmickEventHandlerData_*` class appears anywhere in the 4675-pair harvest, because chart
  nodes are not static-info records and never emit the Korean per-field deserializer messages.
  That is a reusable rule: **if a class is absent from `fieldnames.json`, it is probably not a
  table the record-loader hook can see.**
- The name does mean what it says. The drop path's own error enum ships Korean descriptions at
  `0x580D4A8`/`0x580D4E8` reading 드랍 **확률** - *probability*, not 수량/amount - so "DropRate"
  in this engine is a **chance** with an error code for failing the roll. CONFIRMED at the
  vocabulary level; **PLAUSIBLE** that this particular node scales the collect roll, since the
  consumer of `gimmick+0x1A0` was not found.

Do not spend more static-analysis time on `gimmick+0x1A0`.

### What it uncovered, and this is the part worth taking

A **124-value buff-effect kind enum**, registered at `0x1539300` into a registry at
`0x6932700`, that nothing in this project has ever looked at. `VaryCollectDropRate` is
**kind 2**, and its neighbours are the interesting ones: `VaryStat` (5), `VaryStatRate` (8),
`Loot` (11), `RegisterItemSellPriceRate` (99), `RegisterCrimePriceRate` (100),
`RegisterFactionOperationRewardRate` (101 - which is the *dispatch* subsystem's reward axis,
arriving from a completely different direction). The kind-2 processor (`FUN_141fe8de0`) keeps a
keyed `i64` accumulator on the character at `comp+0x3E8` (count `+0x3F0`, cap `+0x3F4`,
`0x10`-byte entries `{i32 key; i32 pad; i64 value}`) and adds `new - old`, the delta form that
makes buff removal work. The key is `BuffData+0x90`; **PLAUSIBLE** it is a `dropTagNameHash`,
with item key and `DropSetInfo.key` as the alternatives.

### And the money answer, which is not where the currency entry assumed

**`AddMoneyDropRate` exists** (RVA `0x58A08A0`) and is **index 14 of a 19-entry character-stat
name array** at `0x58A19C0` (`DDD, DPV, DHIT, DDV, DPVRate, CriticalDamage, CriticalRate,
AttackedDamageRate, AttackedDamageReduction, AttackSpeedRate, MoveSpeedRate, ClimbSpeedRate,
SwimSpeedRate, EquipDropRate, AddMoneyDropRate, MoveRate, AccRate, RotationRate, JumpRate`).
**PLAUSIBLE** that this is the name table for `StatusInfo._statType` / `._staticStatType` -
and `statusinfo` **is** a static-info table, which would put it inside the shape this project
already reads. **CONFIRMED: there is no `*MoneyDropRate*` gimmick-chart node** - the 208-entry
family has exactly one `Rate` name, the collect one. So the currency entry above may be looking
in the wrong place: its answer is more likely the character stat `AddMoneyDropRate` than
`ItemInfo._moneyTypeDefine`.

### The one cheap thing to do next

**A one-launch census that needs no hook**, reusing the `desert-dispatch` static-info walk
verbatim - that subsystem already resolves arbitrary tables through `AccessorSites` and walks
`manager+0x58` from its own thread, which is why this costs almost nothing:

1. every `buffinfo` `buffDataList` entry of kind **2, 99, 100, 101**, with its `+0x90` key; and
2. every `statusinfo` row whose stat type is `AddMoneyDropRate`.

That answers whether any of this is secretly a record-field lever the gatherer could write, and
it settles the currency entry at the same time. It is the same shape as the dispatch dump that
paid for itself twice, and it carries the same near-zero risk: reads only, no hook, no patch.
