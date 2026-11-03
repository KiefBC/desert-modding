# Looter internals

Sources: `crates/desert-harvest/src/entry.rs`, `crates/desert-harvest/src/looter/entry.rs`, `game.rs`, `hunting.rs`, `CLAUDE.md` (How the looter works). Evidence: `docs/reference-internals.md`, which the source comments cite by section; `docs/reference-ledger.md` for every address and offset.

Read `docs/reference-internals.md` before changing `game.rs`, `events.rs` or `actors.rs`.

## Start-up

The looter is one half of `desert-harvest`. `desert_harvest::start()` runs on the `desert-harvest` thread and never returns; the looter's steps come after the gatherer's two hooks.

1. Read `[Harvest]` from `DesertTooling.ini` through `ini::lines_in_section`, once for both halves; a missing file or key means the default.
2. `MainModule::locate()` maps the running image; `game::resolve()` finds the **nine** byte signatures in `game::SIGNATURES` (`loot_grant` is the ninth) and the `ClientActorManager` RTTI vtable. The signatures are one walk of the image (`pattern::find_unique_multi`), not one each. The log line is `[sig] N found, N missing, N actor-manager vtable(s)`.
3. Three inline trampoline hooks go in **immediately**, whatever `Enabled` says, while the game is still loading: `area_sweep` (the game-thread callback), `enqueue` (the F7 event recorder) and `loot_grant` (the `Hunting` multiplier, `hunting.rs`). Each checks the expected prologue bytes first and refuses to patch on a mismatch: a refusal degrades the plugin, a wrong patch crashes the game.
4. The event API (`prepare`, `desc_by_id`, `alloc_event`, `enqueue`, the steal check) is resolved from the signature hits.
5. The main loop polls hotkeys every 30 ms, stats the ini once a second, waits a 20-second boot grace before the first heap walk, retries finding the world every 3 s, and resolves the `TrocTrProcessPickUpItemOnceTimer` descriptor once the world exists.

## The write path

The game's event allocator and enqueue read thread-local state (`gs:[0x58]`), so the build-and-send sequence must run on the game thread. The plugin thread never sends: it parks a `PickupRequest`, and the `area_sweep` hook, which runs on the game thread every frame, drains it. **Anything new that writes to the game must follow this park-and-drain shape.**

What is sent is the game's own `PickUpItem` event, byte for byte: `payload.rs` builds the 13-byte payload, `events.rs` fills the 0x80-byte event object. No input is faked. `autogather.rs` is pure policy on the plugin thread: pacing, per-node cooldowns, bag-full detection, giving up after repeated failures.

## Containers

The container table is a survey-only diagnostic. `just gen-containers` (`crates/tools/src/bin/gen-containers.rs`) writes `desert_core::container_table` from the archives: each `gimmickinfo` record is walked from its key through its tags, and each `interactioninfo` record's `checkLootableField` is read off its head. A record with a take interaction (`checkLootableField` 1, named `Gimmick_PickUp*`, `ItemHot_Collect*` and the like) that is not a puzzle box or part, chest, furniture storage or personal storage is a `FurnitureDrop` with the tag `furniture_drop` (12) or an `ItemContainer` with `catch_itemcontainer` (59). The key sets match the catalogue database for 26361232.

These 71 records carry a take interaction in the archive, but in game they are broken apart to loot, never taken: there is no take prompt. Nothing targets them. `actors::classify` does not look them up, so they classify by their objects like any other gimmick (armed in focus: `Interactable`; out of focus: `Inert`), and the `GatherItems` path cannot reach them because none is named `item_basic*` (`actors` test `no_container_is_a_ground_item_target`). The only consumer is the F11 survey, which looks the key up with `container_class(key)` (binary search over the sorted table) and adds `container=item-container` or `container=furniture-drop` to the line. A lever that sent them the ground-item `PickUpItem` event was built for 0.14.0 and withdrawn before release for that reason.

## The `Hunting` hook

Bodies are a third class beside animals and catches. `gen-species` classifies every non-`Animal_` `characterinfo` record with the `Dead_Loot` interaction by the game's ally groups in `charactergroupinfo` (0.14.0; 0.13.0 used `isEnableFriendly`, which means "can be befriended" and was wrong): a member of `CharacterGroup_Ally_Player`, or `nonePlayerSubType == 3` (playable), is `Protected` (41 rows; never searched, caught or skinned; includes seven hostile machines, deliberately); `Ally_NPC`, `Ally_Guard` or `Ally_Mercenary` is `FriendlyBody` (4402; the game always lists Guard and Mercenary with NPC); `Ally_Enemy` or `Ally_Aniaml_Wild` is `Body` (1414); a record in no ally group falls back to `isEnableFriendly` (0 `Body`, 1 `FriendlyBody`). A dead `Protected` actor is `None` in `corpse_kind` and logs `[corpse] dead protected actor ...` once per session. `species.rs` serves the classes, and `species.rs` serves them like the other classes; `GatherBodies` and `GatherFriendlyBodies` pick which are aimed at, using the same forged dead-drop event as `GatherCarcass`. Before sending, the body's owner id is read as the game's loot gate does, `*(*(actor+0x68)+0x118)`: null or a u64 at `+0x20` equal to -1 is unowned, anything else (or a failed read) is refused, once on the plugin thread and again on the game thread. Animal carcasses do not use this gate. Class byte 0x5A is shared by dead bandits and animals, so the species table, not the byte, decides. The loot-grant hook gates on the method only, so `Hunting` scales body searches too. Ground drops from enemies are `item_basic_*` props under `GatherItems`/`GatherGear`.

A carcass's loot comes from two arrays on the animal's `characterinfo` record, rolled lazily inside the grant, and one of them is the shared `dropsetinfo` table that dispatch also writes. So the multiplier is applied downstream of both, in the hook on `loot_grant`, and only when the loot-method mask includes method 0 (search or skin). It edits nothing static, so uninstalling is the revert. The reasoning and the measured log are in the `hunting.rs` module comment.

## Hooks and stolen bytes

`trampoline` (in `desert-core`, always compiled) builds the stub and patch bytes so they are unit tested natively; `hook` (Windows only) does the RWX `VirtualAlloc` and the prologue overwrite. The stolen byte count is the caller's job and must be whole, position-independent instructions read off a disassembly. `SWEEP_STOLEN` and `ENQUEUE_STOLEN` in `looter/entry.rs` carry the reasoning for their values.
