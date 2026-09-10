# Crimson Desert mods - task runner. `just` (or `just --list`) shows the recipes.
#
# Every recipe works from inside `nix develop` and from a plain shell: outside
# the dev shell each cargo/python/zip command is re-run through
# `nix develop --command`, so `just build` is enough either way.

set shell := ["bash", "-euo", "pipefail", "-c"]

export NIX_CONFIG := "experimental-features = nix-command flakes"

# Empty inside the dev shell, the nix wrapper outside it.
nix := if env("IN_NIX_SHELL", "") == "" { "nix develop --command" } else { "" }

# Where the game lives; override with CD_BIN64=/path/to/bin64.
bin64 := env("CD_BIN64", "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64")

# DMM's extracted clean gimmickinfo table; override with CD_DMM_TABLE.
dmm_table := env("CD_DMM_TABLE", "/mnt/f/DMM/backups/gimmickinfo_pabgb_clean.bin")

# Steam appmanifest holding the build id; override with CD_APPMANIFEST.
appmanifest := env("CD_APPMANIFEST", bin64 / "../../../appmanifest_3321460.acf")

built := "target/x86_64-pc-windows-gnu/release"
native := "x86_64-unknown-linux-gnu"

# Every DLL DesertTooling.asi is allowed to import. All of these ship WITH
# Windows, so an .asi that imports only these loads in bin64 with nothing
# installed beside it. The list exists because desert-overlay links imgui,
# which is C++: by default the mingw `cc` crate links libstdc++-6.dll, which
# bin64 does not have, and the plugin would fail to load with an unhelpful
# error. Everything is linked into the one plugin now, so every entry below is
# an import of DesertTooling.asi. See the [env] block in .cargo/config.toml for
# how libstdc++ is avoided; `just check-imports` is what proves it stayed
# avoided.
#
#   kernel32 msvcrt ntdll bcryptprimitives userenv ws2_32
#                                  Rust's own std
#   api-ms-win-core-synch-l1-2-0   std's futex/condvar shims
#   user32                         GetAsyncKeyState, the window procedure
#   advapi32                       std's fallback entropy source on old builds
#   d3d12 dxgi d3dcompiler_47      hudhook's DX12 renderer, from the overlay
#   oleaut32 combase rpcrt4 api-ms-win-core-winrt-error-l1-1-0
#                                  COM, reached through the `windows` crate
#                                  that hudhook uses
allowed_imports := "advapi32 api-ms-win-core-synch-l1-2-0 api-ms-win-core-winrt-error-l1-1-0 bcryptprimitives combase d3d12 d3dcompiler_47 dxgi kernel32 msvcrt ntdll oleaut32 rpcrt4 user32 userenv ws2_32"

# List the recipes.
default:
    @just --list --unsorted

# Release build of the plugin (Windows x64). `just build desert-looter` for one crate.
build crate="":
    {{nix}} cargo build --release {{ if crate == "" { "" } else { "-p " + crate } }}

# Unit tests on both targets: native Linux, then the Windows binaries via WSL interop.
test: test-native test-win

# Unit tests, native Linux target (the pure-logic modules).
test-native:
    {{nix}} cargo test --target {{native}}

# Unit tests built for Windows and run through WSL interop.
test-win:
    {{nix}} cargo test

# The #[ignore]d tests against the real CrimsonDesert.exe: signatures, vtable, record loader. Run after a game update.
test-game:
    {{nix}} cargo test --release --target {{native}} -p desert-looter --test game_exe -- --ignored --nocapture
    {{nix}} cargo test --release --target {{native}} -p desert-core -- --ignored --nocapture

# Clippy on both targets; any warning fails.
clippy:
    {{nix}} cargo clippy --release --all-targets -- -D warnings
    {{nix}} cargo clippy --release --all-targets --target {{native}} -- -D warnings

# Check Cargo.lock against the RustSec advisory database.
audit:
    {{nix}} cargo audit

# Everything a commit should pass: clippy, tests, audit, doc versions, imports.
ci: clippy test audit check-versions check-imports

# Fail if a built plugin imports a DLL that is not part of Windows. Part of `just ci`.
check-imports: build
    {{nix}} tools/check-imports.sh {{allowed_imports}}

# Rewrite the version tables in README.md / VERSIONING.md from the Cargo.toml versions.
sync-versions:
    {{nix}} python3 tools/sync-versions.py

# Fail if those docs have drifted from the Cargo.toml versions. Part of `just ci`.
check-versions:
    {{nix}} python3 tools/sync-versions.py --check

# Rasterise assets/logo.svg into desert-overlay/src/logo.rgba (the menu's header logo).
logo:
    {{nix}} python3 tools/logo-to-rgba.py

# Re-check every byte signature against the game exe (each must hit once).
sigscan:
    {{nix}} python3 tools/sigscan.py

# Build the release zips into dist/ (the plugin, the DMM pack, SHA256SUMS).
# `just dist desert-tooling` builds only that package, as a release tag does.
dist *packages:
    {{nix}} tools/dist.sh {{packages}}

# Copy the built plugin into the game's bin64 as DesertTooling.asi. Refuses while the game runs.
install: build
    @if command -v tasklist.exe >/dev/null && tasklist.exe /FI "IMAGENAME eq CrimsonDesert.exe" 2>/dev/null | grep -q CrimsonDesert.exe; then \
        echo "install: CrimsonDesert.exe is running; close the game first" >&2; exit 1; fi
    @test -d "{{bin64}}" || { echo "install: {{bin64}} not found (set CD_BIN64)" >&2; exit 1; }
    cp "{{built}}/desert_tooling.dll" "{{bin64}}/DesertTooling.asi"
    @echo "installed into {{bin64}}"
    @for stale in DesertLooter DesertGatherer DesertOverlay; do \
        test -e "{{bin64}}/$stale.asi" || continue; \
        echo "install: WARNING {{bin64}}/$stale.asi is still there; delete it (two copies of a hook is a crash)" >&2; \
     done

# One plugin means one log: the old `glog` and `olog` recipes are gone, and every
# line carries its subsystem's tag, so `just log | grep '\[gatherer\]'` is what
# they were for.

# Follow DesertTooling.log from the game folder.
log:
    tail -n 40 -F "{{bin64}}/DesertTooling.log"

# Show the learned yields cache (the looter's node -> item table, written beside the log).
yields:
    @shopt -s nullglob; \
     files=("{{bin64}}"/Desert*.yields); \
     test "${#files[@]}" -gt 0 || { echo "yields: no Desert*.yields in {{bin64}} yet" >&2; exit 1; }; \
     for f in "${files[@]}"; do echo "== $(basename "$f")"; cat "$f"; done

# Remove build output and dist/.
clean:
    {{nix}} cargo clean
    rm -rf dist

# --- DMM pack (desert-gatherer-dmm/) --------------------------------------
# Not part of build/test/ci: it rewrites the packaged patch offsets, which only
# needs doing after a game update. Run `just test-game` first to confirm the
# record loader and output-block signature still match this build.

# The table comes from DMM's backups (CD_DMM_TABLE) and the build id from the
# Steam appmanifest (CD_APPMANIFEST). Either can be passed positionally instead:
# `just dmm-rebase /path/to/clean.bin 25116796`.

# Rebase desert-gatherer-dmm/*.json onto the current game build. Takes no arguments.
dmm-rebase table=dmm_table build="":
    @test -f "{{table}}" || { echo "dmm-rebase: {{table}} not found (set CD_DMM_TABLE)" >&2; exit 1; }
    @build="{{build}}"; \
    if [ -z "$build" ]; then \
        test -f "{{appmanifest}}" || { echo "dmm-rebase: {{appmanifest}} not found (set CD_APPMANIFEST, or pass the build id)" >&2; exit 1; }; \
        build=$(sed -n 's/.*"buildid"[^"]*"\([0-9][0-9]*\)".*/\1/p' "{{appmanifest}}" | head -1); \
        test -n "$build" || { echo "dmm-rebase: no buildid in {{appmanifest}}" >&2; exit 1; }; \
    fi; \
    echo "dmm-rebase: {{table}} -> build $build"; \
    {{nix}} python3 desert-gatherer-dmm/rebase.py "{{table}}" "$build"
