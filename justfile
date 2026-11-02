# Crimson Desert mods - task runner. `just` (or `just --list`) shows the recipes.
#
# Every recipe works from inside `nix develop` and from a plain shell: outside
# the dev shell each cargo command is re-run through `nix develop --command`,
# so `just build` is enough either way.
#
# Nothing here shells out to python3, objdump, zip or sha256sum any more. Every
# support script in crates/tools/ is a Rust binary built by `just tools`, and the dev
# shell is now needed only for the Rust toolchain and the mingw linker.

set shell := ["bash", "-euo", "pipefail", "-c"]

export NIX_CONFIG := "experimental-features = nix-command flakes"

# Empty inside the dev shell, the nix wrapper outside it.
nix := if env("IN_NIX_SHELL", "") == "" { "nix develop --command" } else { "" }

# Where the game lives; override with CD_BIN64=/path/to/bin64.
bin64 := env("CD_BIN64", "/mnt/f/SteamLibrary/steamapps/common/Crimson Desert/bin64")

# The clean gimmickinfo table body and the Steam appmanifest are not named
# here: the tools that need them resolve them through crates/tools/src/paths.rs.
# One rule in one place, instead of a copy here that could drift from it.

built := "target/x86_64-pc-windows-gnu/release"
native := "x86_64-unknown-linux-gnu"

# Where `just tools` puts the support binaries. Everything in crates/tools/ is Rust
# now: there are no .py or .sh files left there, and no recipe shells out to
# python3, objdump, zip or sha256sum any more.
tools_bin := "crates/tools/target/" + native + "/release"

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

# Release build of the plugin (Windows x64). `just build desert-harvest` for one crate.
build crate="":
    {{nix}} cargo build --release {{ if crate == "" { "" } else { "-p " + crate } }}

# crates/tools/ is its OWN cargo workspace, excluded from this one, because everything
# here cross-compiles to Windows and none of the tools do.
#
# CARGO_BUILD_TARGET is not decoration. crates/tools/.cargo/config.toml pins the host
# target, but cargo discovers config by walking up from the CURRENT DIRECTORY,
# not from --manifest-path - so run from the repo root, the tools pick up the
# ROOT config instead and try to cross-compile themselves to Windows, failing
# with "can't find crate for core". Setting the target in the environment wins
# over both config files, which lets every recipe below stay in the repo root,
# where a relative path handed to a tool still means what it says.

# Build the support tools in crates/tools/ (host binaries, their own workspace).
tools:
    CARGO_BUILD_TARGET={{native}} {{nix}} cargo build --release --manifest-path crates/tools/Cargo.toml

# Clippy and unit tests for the tools (host target only; nothing here ships).
tools-check:
    CARGO_BUILD_TARGET={{native}} {{nix}} cargo clippy --manifest-path crates/tools/Cargo.toml --all-targets -- -D warnings
    CARGO_BUILD_TARGET={{native}} {{nix}} cargo test --manifest-path crates/tools/Cargo.toml

# Separate from `tools-check` for the same reason `test-game` is separate from
# `test`: a CI runner has no game, no table dump and no Ghidra.

# The tools' tests needing the game exe, the game's archives, the table dump or a
# built plugin. `-- --ignored` runs every #[ignore]d test in the workspace, so a
# new file such as tests/paz_real.rs (the .pamt/.paz reader against the install
# and analysis/tables/) is picked up with no change here.
tools-check-game: tools
    CARGO_BUILD_TARGET={{native}} {{nix}} cargo test --manifest-path crates/tools/Cargo.toml -- --ignored

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
    {{nix}} cargo test --release --target {{native}} -p desert-harvest --test game_exe -- --ignored --nocapture
    {{nix}} cargo test --release --target {{native}} -p desert-core -- --ignored --nocapture
    {{nix}} cargo test --release --target {{native}} -p desert-dispatch --test banking_real -- --ignored --nocapture

# Clippy on both targets; any warning fails.
clippy:
    {{nix}} cargo clippy --release --all-targets -- -D warnings
    {{nix}} cargo clippy --release --all-targets --target {{native}} -- -D warnings

# Check Cargo.lock against the RustSec advisory database.
audit:
    {{nix}} cargo audit

# Everything a commit should pass: clippy, tests, audit, doc versions, imports.
ci: clippy test tools-check audit check-versions check-imports

# Fail if a built plugin imports a DLL that is not part of Windows. Part of `just ci`.
check-imports: build tools
    {{tools_bin}}/check-imports {{allowed_imports}}

# Rewrite the version tables in README.md / VERSIONING.md from the Cargo.toml versions.
sync-versions: tools
    {{tools_bin}}/sync-versions

# Fail if those docs have drifted from the Cargo.toml versions. Part of `just ci`.
check-versions: tools
    {{tools_bin}}/sync-versions --check

# Build the documentation book in docs/; output in target/book. The
# reverse-engineering part is local-only: on a fresh clone each of its missing
# files logs a warning and the build still succeeds.
docs:
    {{nix}} mdbook build docs

# Serve the book with live reload on http://127.0.0.1:3000.
docs-serve:
    {{nix}} mdbook serve docs -n 127.0.0.1 -p 3000

# Rasterise assets/logo.svg into crates/desert-overlay/src/logo.rgba (the menu's header logo).
logo: tools
    {{tools_bin}}/logo-to-rgba

# Re-check every byte signature against the game exe (each must hit once).
sigscan: tools
    {{tools_bin}}/sigscan

# Re-target after a game update: `just retarget report` first, then `triage`,
# `census`, `calls`, `callers`, `prologue`, `pe`. `just retarget --help` lists them.
retarget *args: tools
    {{tools_bin}}/retarget {{args}}

# Extract every static-info table (.staticinfobody / .staticinfoheader) out of
# the game's archives, group 0008, into analysis/tables/. `--out DIR` and
# `--game-dir DIR` are passed through; CD_GAME_DIR overrides the install.
extract-tables *args: tools
    {{tools_bin}}/paz extract-tables {{args}}

# Re-export the Ghidra evidence tree into evidence/ (needs Ghidra running on
# Windows with GhidraMCP listening). Resumable: existing files are not
# re-fetched. `just evidence --depth 2` and any other flag is passed through.
evidence *flags: tools
    {{tools_bin}}/evidence {{flags}}

# Build the release zip into dist/ (the plugin and SHA256SUMS).
#
# The {{nix}} prefix is still needed here, unlike the other tool recipes: `dist`
# itself spawns `cargo build --release` for the Windows target, which needs the
# mingw linker from the dev shell. The tool no longer shells out to zip, unzip
# or sha256sum - cargo is the only external program it runs.

# `just dist desert-tooling` builds only that package, as a release tag does.
dist *packages: tools
    {{nix}} {{tools_bin}}/dist {{packages}}

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
# line carries its subsystem's tag, so `just log | grep '\[harvest\]'` is what
# they were for.

# Follow DesertTooling.log from the game folder.
log:
    tail -n 40 -F "{{bin64}}/DesertTooling.log"

# Show the learned yields cache (the auto-loot half's node -> item table, written beside the log).
yields:
    @shopt -s nullglob; \
     files=("{{bin64}}"/Desert*.yields); \
     test "${#files[@]}" -gt 0 || { echo "yields: no Desert*.yields in {{bin64}} yet" >&2; exit 1; }; \
     for f in "${files[@]}"; do echo "== $(basename "$f")"; cat "$f"; done

# Remove build output and dist/.
clean:
    {{nix}} cargo clean
    {{nix}} cargo clean --manifest-path crates/tools/Cargo.toml
    rm -rf dist

# --- DMM pack (desert-gatherer-dmm/) --------------------------------------
# Not part of build/test/ci: it regenerates the pack's twelve modules and
# VERIFICATION.txt, which only needs doing after a game update or a change to
# collect.rs. Run `just test-game` first: its pack oracle,
# `multiply_reproduces_the_dmm_pack_edits`, confirms the output-block walk
# still matches this build. The table comes out of the game's archives
# (`--tables DIR` reads extracted copies instead), the build id from the Steam
# appmanifest (CD_APPMANIFEST, or `--build ID`). Flags pass through, and
# `[positional-arguments]` keeps a path with spaces in it one argument.

# Regenerate desert-gatherer-dmm/ from the game's gimmickinfo and collect.rs. `--dry-run` writes nothing.
[positional-arguments]
dmm-pack *args: tools
    {{tools_bin}}/dmm-pack "$@"
