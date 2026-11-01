# Looter internals

Sources: `desert-looter/src/lib.rs`, `game.rs`, `hunting.rs`, `CLAUDE.md` (How Desert Looter works). Evidence: `docs/reference-internals.md`, which the source comments cite by section; `docs/reference-ledger.md` for every address and offset.

Read `docs/reference-internals.md` before changing `game.rs`, `events.rs` or `actors.rs`.

## Start-up

`desert_looter::start()` runs on its own thread and never returns.

1. Read `[Looter]` from `DesertTooling.ini` through `ini::lines_in_section`; a missing file or key means the default.
2. `MainModule::locate()` maps the running image; `game::resolve()` finds the **nine** byte signatures in `game::SIGNATURES` (`loot_grant` is the ninth) and the `ClientActorManager` RTTI vtable. The signatures are one walk of the image (`pattern::find_unique_multi`), not one each. The log line is `[sig] N found, N missing, N actor-manager vtable(s)`.
3. Three inline trampoline hooks go in **immediately**, whatever `Enabled` says, while the game is still loading: `area_sweep` (the game-thread callback), `enqueue` (the F7 event recorder) and `loot_grant` (the `Hunting` multiplier, `hunting.rs`). Each checks the expected prologue bytes first and refuses to patch on a mismatch: a refusal degrades the plugin, a wrong patch crashes the game.
4. The event API (`prepare`, `desc_by_id`, `alloc_event`, `enqueue`, the steal check) is resolved from the signature hits.
5. The main loop polls hotkeys every 30 ms, stats the ini once a second, waits a 20-second boot grace before the first heap walk, retries finding the world every 3 s, and resolves the `TrocTrProcessPickUpItemOnceTimer` descriptor once the world exists.

## The write path

The game's event allocator and enqueue read thread-local state (`gs:[0x58]`), so the build-and-send sequence must run on the game thread. The plugin thread never sends: it parks a `PickupRequest`, and the `area_sweep` hook, which runs on the game thread every frame, drains it. **Anything new that writes to the game must follow this park-and-drain shape.**

What is sent is the game's own `PickUpItem` event, byte for byte: `payload.rs` builds the 13-byte payload, `events.rs` fills the 0x80-byte event object. No input is faked. `gatherer.rs` is pure policy on the plugin thread: pacing, per-node cooldowns, bag-full detection, giving up after repeated failures.

## The `Hunting` hook

A carcass's loot comes from two arrays on the animal's `characterinfo` record, rolled lazily inside the grant, and one of them is the shared `dropsetinfo` table that dispatch also writes. So the multiplier is applied downstream of both, in the hook on `loot_grant`, and only when the loot-method mask includes method 0 (search or skin). It edits nothing static, so uninstalling is the revert. The reasoning and the measured log are in the `hunting.rs` module comment.

## Hooks and stolen bytes

`trampoline` (in `desert-core`, always compiled) builds the stub and patch bytes so they are unit tested natively; `hook` (Windows only) does the RWX `VirtualAlloc` and the prologue overwrite. The stolen byte count is the caller's job and must be whole, position-independent instructions read off a disassembly. `SWEEP_STOLEN` and `ENQUEUE_STOLEN` in `lib.rs` carry the reasoning for their values.
