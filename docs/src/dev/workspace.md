# Workspace and rules

Sources: root `Cargo.toml`, `clippy.toml`, each crate's `src/lib.rs`, `CLAUDE.md` (Architecture).

## Crates

| Crate | Kind | Ships | Entry point |
| --- | --- | --- | --- |
| `desert-tooling` | cdylib | `DesertTooling.asi`, the only shipped binary | `DllMain`, the only one in the workspace |
| `desert-core` | rlib | nothing, linked in | none: plumbing (hooks, guarded reads, logger, PE/RTTI, byte patterns, ini, schema, the gather-record table) |
| `desert-looter` | rlib | nothing, linked in | `start()`, never returns |
| `desert-gatherer` | rlib | nothing, linked in | `start()` |
| `desert-overlay` | rlib | nothing, linked in | `start(hmodule, sections)`, returns once its hooks are applied |
| `desert-dispatch` | rlib | nothing, linked in | `start()` |
| `desert-gatherer-dmm/` | data, not a crate | the DMM offset-patch pack | none |

`tools/` is a separate cargo workspace of host binaries, excluded from this one (`exclude = ["tools"]`), because everything here cross-compiles to Windows and the tools do not.

`desert-tooling` holds no policy. Its thread inits the log, seeds the ini if it is missing, refuses to go on if a pre-merge `.asi` is loaded, and starts one named thread per subsystem (`desert-gatherer`, `desert-looter`, `desert-dispatch`, `desert-overlay`). Each subsystem keeps its own ini `[Section]`, log tag and `schema()`. Anything that is plumbing rather than mod policy belongs in `desert-core`, so the logger and the memory reads exist once.

## The five rules

Each one corresponds to a bug that killed the game.

1. **Never panic.** `panic = "abort"` in `[profile.release]`, so a panic is a crash to desktop.
2. **Never dereference game memory.** Every foreign read goes through `desert_core::safe`, which uses `ReadProcessMemory` on our own process so an unmapped page returns `None`.
3. **Host-exe gate.** `DllMain` compares `current_exe()` with `CrimsonDesert.exe`, calls `log::disable()` and returns if it does not match (the loader also injects into `crashpad_handler.exe`).
4. **No file I/O in `DllMain`.** It calls `DisableThreadLibraryCalls`, then `CreateThread`, and returns.
5. **The logger opens, appends and closes per write** with `FILE_SHARE_READ|WRITE|DELETE`, and drops a write it cannot make.

Rule 1 is enforced by the compiler. `[workspace.lints]` denies `clippy::unwrap_used`, `expect_used`, `indexing_slicing`, `panic`, `unreachable`, `todo`, `unimplemented`, `undocumented_unsafe_blocks` and rustc's `unsafe_op_in_unsafe_fn`. Tests are exempt through `clippy.toml`; shipped `src/` never carries an `#[allow]` for these. Every `unsafe` block carries a `// SAFETY:` comment.

## Threads and `Enabled=0`

The looter and gatherer install their hooks even under `Enabled=0` and gate behaviour live on the flag: an inline hook is only safe to install while the game is still loading and nothing is executing the prologue, so there is no later moment to do it. That is also why each subsystem gets its own thread: the gatherer's hook goes in with no boot grace and must not wait behind the looter's 20-second grace. The dispatch subsystem installs nothing, so its `Enabled=0` at launch means nothing is read at all. The overlay under `Enabled=0` installs no graphics hook.

## cfg gating

Every crate splits its modules into always-compiled and `#[cfg(windows)]`, so `cargo test --target x86_64-unknown-linux-gnu` runs the pure logic natively. Put a module on the Windows side only if it needs Win32 or a live game pointer.

| Crate | Always | Windows only |
| --- | --- | --- |
| `desert-core` | `collect creature gimmick ini log manager pattern pe rtti schema telemetry trampoline` | `hook hotkey module safe` |
| `desert-looter` | `config payload recorder` | `actors events game gatherer hunting tables` |
| `desert-gatherer` | `config dump remember` | `catch hook`, and the private `entry` |
| `desert-overlay` | `config dynmodel logo rewrite store theme themes` | `cursor trace ui` |
| `desert-dispatch` | `buff config gate node remember` | `apply census scan` (private) |

`desert-tooling` is Windows-only almost end to end. hudhook and imgui are declared under `[target.'cfg(windows)'.dependencies]` in `desert-overlay/Cargo.toml` and do not build natively.

## Logging and re-exports

`log!` expands to `log::write_tagged(crate::LOG_TAG, ...)`. Because `crate::` inside `macro_rules!` resolves at the call site, every crate declares its own `pub const LOG_TAG` (`tooling`, `core`, `looter`, `gatherer`, `overlay`, `dispatch`).

Each subsystem's `lib.rs` re-exports the `desert-core` modules it uses under their own names (`desert-looter`: `pub use desert_core::{collect, ini, log, pattern, pe, rtti, schema, telemetry, trampoline}` plus `hook, hotkey, module, safe` on Windows), so `crate::log!` and `crate::safe::read` resolve in every module. This is deliberate: do not rewrite it into direct `desert_core::` paths. `desert-gatherer` does not re-export `hook`: its own `hook` module (the record-loader callback) re-exports `desert_core::hook::{hex, install, Hook}`, so `crate::hook::` names both, and `catch` re-exports `install_raw` the same way. `desert-dispatch` omits `hook` and `trampoline` on purpose: it installs no hook, and a name that does not resolve guards against adding one by habit.
