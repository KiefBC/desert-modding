# vendor/

Third-party source that the workspace builds from a local copy instead of from
crates.io. There is one entry, and it is here because the published crate
stops the game from launching.

## `hudhook/`

The published [hudhook](https://github.com/veeenu/hudhook) 0.9.2 crate (MIT,
by Andrea Venuta) with a handful of changes applied. Desert Overlay is its only
user: hudhook hooks the game's DX12 present/resize and runs the Dear ImGui
render loop the settings menu draws in. Nothing else in the tree depends on it.

### Why a copy

Upstream 0.9.2 keeps a strong `IDXGISwapChain3` reference across
`ResizeBuffers` and across the game creating a replacement swapchain, and only
notices the change *after* the DXGI call it hooked has returned. Crimson Desert
(build 25116796) resizes and then recreates its swapchain a few seconds into
every launch. With the released crate, `ResizeBuffers` fails with
`E_INVALIDARG`, the retried `CreateSwapChainForHwnd` fails with
`E_ACCESSDENIED` because DXGI will not give a window a second swapchain while
one is still alive (ours was the reference keeping it alive), and the game
exits before the main menu.

The vendored copy drops every reference before calling those trampolines and
rebuilds on the next `Present`. Beside that it pins the ImGui framebuffer scale
to 1.0 (upstream's DPI-derived value clipped the UI at any Windows scaling
other than 100%), adds an output colour-space conversion so the overlay is not blown
out and oversaturated on the game's HDR10 back buffer, and demotes the once-per-launch
pipeline reset log line from `warn` to `info`.

`hudhook/DESERT-CHANGES.md` is the authoritative list, hunk by hunk, with the
log evidence and the notes (deadlock, WndProc restore, ReShade in the chain)
that whoever re-applies them to a newer hudhook will need. Read it before
touching anything under `hudhook/src`. The in-game side of the same story is
in `docs/reference-overlay.md`.

### How it is wired in

The root `Cargo.toml` carries

```toml
[patch.crates-io]
hudhook = { path = "vendor/hudhook" }
```

and `desert-overlay/Cargo.toml` still asks for `hudhook = "0.9.2"` from the
registry as if nothing had happened. It is a `[patch]` rather than a path
dependency on purpose: the version stays 0.9.2 everywhere, `Cargo.lock` records
it as such, and going back to the published crate is one deleted section.

`hudhook/Cargo.toml` is the registry-generated manifest with the parts this
workspace does not build removed (examples, integration tests, their
dev-dependencies, `[profile.test]`) and an empty `[workspace]` table added.
That table is what keeps the package **out** of the Crimson Desert workspace:
it sits under the workspace root, so without it cargo would insist on it being
a member and the workspace lints (deny `unwrap`, `expect`, `panic`, ...) would
land on upstream code that predates them.

### What is and is not vendored

`LICENSE`, `build.rs`, `src/` and `vendor/minhook/` (the MinHook C sources
that `build.rs` compiles) are as published, apart from the changes listed in
`DESERT-CHANGES.md`. The examples, tests, `hudbook/`, `CHANGELOG.md`,
`README.md`, `Cargo.lock`, `rust-toolchain.toml` and `rustfmt.toml` are not
carried; they add nothing to a build and would only drift.

### When hudhook updates

Do not `cargo update` past this. Take the new crate from crates.io, re-apply
the hunks in `DESERT-CHANGES.md` by hand (they are written to be
re-applicable), trim the manifest the same way, and verify in game: the
swapchain recreation happens on every launch, so a regression shows up as the
game exiting before the main menu with `E_ACCESSDENIED` in `ReShade.log`. If
upstream ever ships the swapchain fix itself, the `[patch]` section and this
directory can go together.
