# Desert Looter

A gathering-focused auto-loot ASI for Crimson Desert, written in Rust and
cross-compiled to a Windows x64 DLL from NixOS (WSL).

This is the **first write** stage. On load it resolves the game-side anchors
(byte signatures and the actor-manager RTTI vtable), hooks the per-frame sweep
function for a game-thread callback, logs to `DesertLooter.log` in `bin64`,
reads `DesertLooter.ini` from the same folder, and reacts to three hotkeys:
F10 toggles automatic gathering, F11 surveys (read-only), F9 gathers the
nearest node once. Gathering forges one PickUpItem event per node on the game
thread; auto mode paces sends by `GatherInterval` and leaves each node alone
for `NodeCooldown` before retrying it.

## What an ASI is

An ASI file is just a Windows DLL with a different extension. Ultimate ASI
Loader (already installed in `bin64` as `winmm.dll`) loads every `*.asi` beside
it into the game. So the plan is: build a `cdylib`, rename the `.dll` to `.asi`,
drop it in `bin64`.

## Toolchain (NixOS WSL)

The `flake.nix` dev shell provides:

- a Rust stable toolchain with the `x86_64-pc-windows-gnu` target, and
- the mingw-w64 cross-linker (`x86_64-w64-mingw32-gcc`), wired to Cargo via an
  environment variable in the shell.

No Visual Studio, no rustup-on-Windows. Enter the shell with:

```bash
nix develop
```

If flakes are not enabled, add `--extra-experimental-features 'nix-command flakes'`.

## Build

Inside the dev shell, from this directory:

```bash
cargo build --release
```

Output:

```
target/x86_64-pc-windows-gnu/release/desert_looter.dll
```

### A note on the WSL filesystem

Building under `/mnt/c` or `/mnt/f` (the Windows drives) works but is slow and
can trip file-watching. Prefer keeping the crate on the Linux filesystem (e.g.
`~/cdgatherloot`) and copying the finished `.asi` over to the game. The source
also lives in the repo at
`/mnt/c/MODDING/Crimson Desert/Gathering AutoLoot/Desert Looter` if you want it
version-controlled there.

## Install / test

Copy the DLL into the game, renamed, and the sample ini beside it:

```bash
cp target/x86_64-pc-windows-gnu/release/desert_looter.dll \
   "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/DesertLooter.asi"
```

```bash
cp DesertLooter.ini "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64/"
```

Launch the game. `bin64/DesertLooter.log` should show the ini summary, the
module base, one `[sig]` line per signature with `= +0x...` (never `NOT FOUND`
or `AMBIGUOUS`), and one `[rtti]` vtable line. F10 toggles (logs ON/OFF), F11
logs a survey request. Each keypress also beeps.

To remove it, delete `bin64/DesertLooter.asi`.

## Link notes

Rust's `x86_64-pc-windows-gnu` target links `-l:libpthread.a` (mingw winpthreads).
The Nix mingw cross compiler does not put that library on its search path, so
the dev shell passes `-L native=<winpthreads>/lib` through
`CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS`, together with `+crt-static`.
That env var replaces any `rustflags` in `.cargo/config.toml`, which is why
the config file only sets the default target.

The finished DLL imports only kernel32, ntdll, user32, msvcrt and one
`api-ms-win-core-synch` forwarder: no mingw runtime DLLs.

## Analysis tooling

The dev shell also provides `python3`, `strings`, `objdump`, `file` and Ghidra
(`ghidra` for the GUI through WSLg, `ghidra-analyzeHeadless` for batch work).
`tools/sigscan.py` scans `CrimsonDesert.exe` for the byte signatures in
`tools/cdloot-signatures.txt` (lifted from the reference mod) and for the
engine class and event names it relies on. Re-run it after every game update;
each signature should hit exactly once.

## Roadmap

1. **Smoke test (this build)** - load, log, hotkey. Prove the pipeline.
2. **Read-only observer** - locate the game's actor manager and gather nodes by
   byte-signature scan (not fixed addresses), log what is nearby. No writes.
3. **Gathering collector** - trigger gather-node interactions and pick up their
   drops via the game's own event queue. Strictly limited to plants, ore, wood
   and their outputs. Never owned goods, corpses, quest items, or furniture.

## Tests

Pure modules (pattern scanner, PE reader, RTTI finder, ini parser) are tested
natively on Linux against the reference mod's own binary:

```bash
cargo test --target x86_64-unknown-linux-gnu
```

One ignored test scans the real game exe (needs the Steam install mounted):

```bash
cargo test --release --target x86_64-unknown-linux-gnu --test game_exe -- --ignored --nocapture
```
