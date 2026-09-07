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

built := "target/x86_64-pc-windows-gnu/release"
native := "x86_64-unknown-linux-gnu"

# List the recipes.
default:
    @just --list --unsorted

# Release build of both plugins (Windows x64). `just build desert-looter` for one.
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

# Everything a commit should pass: clippy, tests, audit, doc versions.
ci: clippy test audit check-versions

# Rewrite the version tables in README.md / VERSIONING.md from the Cargo.toml versions.
sync-versions:
    {{nix}} python3 tools/sync-versions.py

# Fail if those docs have drifted from the Cargo.toml versions. Part of `just ci`.
check-versions:
    {{nix}} python3 tools/sync-versions.py --check

# Re-check every byte signature against the game exe (each must hit once).
sigscan:
    {{nix}} python3 tools/sigscan.py

# Build the release zips into dist/ (both plugins, SHA256SUMS).
dist:
    {{nix}} tools/dist.sh

# Copy the built plugins into the game's bin64 as .asi. Refuses while the game runs.
install: build
    @if command -v tasklist.exe >/dev/null && tasklist.exe /FI "IMAGENAME eq CrimsonDesert.exe" 2>/dev/null | grep -q CrimsonDesert.exe; then \
        echo "install: CrimsonDesert.exe is running; close the game first" >&2; exit 1; fi
    @test -d "{{bin64}}" || { echo "install: {{bin64}} not found (set CD_BIN64)" >&2; exit 1; }
    cp "{{built}}/desert_looter.dll"   "{{bin64}}/DesertLooter.asi"
    cp "{{built}}/desert_gatherer.dll" "{{bin64}}/DesertGatherer.asi"
    @echo "installed into {{bin64}}"

# Follow the looter's log from the game folder.
log:
    tail -n 40 -F "{{bin64}}/DesertLooter.log"

# Follow the gatherer's log from the game folder.
glog:
    tail -n 40 -F "{{bin64}}/DesertGatherer.log"

# Show the learned yields cache.
yields:
    cat "{{bin64}}/DesertLooter.yields"

# Remove build output and dist/.
clean:
    {{nix}} cargo clean
    rm -rf dist

# --- DMM pack (dmm-pack/) -------------------------------------------------
# Not part of build/test/ci: needs a build-specific clean table + build id
# that only exist after a game update, so there is no safe default to run
# automatically. Run `just test-game` first to confirm the record loader and
# output-block signature still match this build before rebasing.

# Rebase dmm-pack/*.json onto a new game build. table = path to DMM's
# extracted gimmickinfo_pabgb_clean.bin, build = the new Steam build id.
dmm-rebase table build:
    {{nix}} python3 dmm-pack/rebase.py {{table}} {{build}}
