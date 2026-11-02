# Overlay internals

Sources: `crates/desert-overlay/src/lib.rs`, `ui.rs`, `rewrite.rs`, `store.rs`, `crates/desert-core/src/schema.rs`, `crates/desert-tooling/src/lib.rs`, root `Cargo.toml`, `.cargo/config.toml`, `CLAUDE.md` (How Desert Overlay works). Evidence: `docs/reference-overlay.md`.

Read `docs/reference-overlay.md` before touching `crates/desert-overlay/src/ui.rs` or `vendor/hudhook`: it records the swapchain, DPI, HDR, font and input traps that each cost a launch to find, and how to verify a change in game.

## Start-up

hudhook, vendored under `vendor/hudhook` as a `[patch.crates-io]` (the changes are in [hudhook fork changes](../repo/vendor/hudhook/DESERT-CHANGES.md)), hooks DX12 `Present`, `ResizeBuffers` and `ExecuteCommandLists` through a throwaway device of its own and drives a Dear ImGui render loop inside the game's present. `desert-tooling` calls `desert_overlay::start(hmodule, sections)` on its own thread with the module handle and every subsystem's `schema::Section`. `start` reads `[Overlay]`, returns early with no graphics hook under `Enabled=0`, installs the tracing bridge, applies the hooks, then installs the cursor hooks, and **returns**: from then on the menu runs on the game's render thread. A failed apply is logged and never ejects the DLL.

## The menu is the schemas

Each subsystem declares its settings once, as `schema()` in its own `config.rs`, naming the `[Section]` it owns in `DesertTooling.ini`. **That is the one place a menu setting is added**; nothing in `desert-overlay` changes. Sections are sorted by `(order, title)`.

The menu is a tab bar: one tab per section that has a field of its own (Looter, Gatherer, Dispatch; every `[Overlay]` key sits on a shared tab), then the shared Settings and Debug tabs. A tab's label is drawn dimmed while that subsystem's `Enabled` is off. The widget comes from the field's `Kind`: `Bool` checkbox, `Int` input or slider, `Float` slider, `Choice` combo, `Key` combo over `desert_core::ini::key_names()`. One `Store<DynModel>` sits behind each section.

## Writing the ini

The overlay never talks to another subsystem. It edits `DesertTooling.ini` and each subsystem picks the change up on its own once-a-second poll. Writes go through `rewrite::write_atomically` (`DesertTooling.ini.tmp` beside the target, then rename) and touch only the lines whose key the overlay owns inside the right `[Section]`, so comments, order and unshown keys survive. The hazard is same-named keys under different headers (`Enabled`, `Debug`, `DryRun`); `rewrite`'s tests cover it, and `ini::lines_in_section` is the reading side of the same problem. `store.rs` re-reads only when the file's mtime moved.

Nothing on the overlay side checks that a schema agrees with its subsystem's parser; each subsystem's own `config::parse`-against-`schema()` test does. The same sections seed the file: `desert-tooling` calls `schema::create_ini_if_missing_all` before any subsystem reads it, opening with `create_new`, so an existing ini is never rewritten.

## Keeping libstdc++ out

imgui is C++, and the mingw `cc` crate would link `libstdc++-6.dll`, which `bin64` does not have. The `[env]` block in `.cargo/config.toml` (`CXXSTDLIB=""`, `CXXFLAGS_x86_64_pc_windows_gnu=-fno-threadsafe-statics`) keeps the C++ runtime out; `just check-imports`, part of `just ci` and of CI, proves it stayed out.
