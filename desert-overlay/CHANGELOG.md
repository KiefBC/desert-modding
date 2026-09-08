# Changelog

All notable changes to Desert Overlay. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

## [Unreleased]

Game build 25116796. Verified in game on 2026-09-07: the menu draws, moves and resizes, a preset
click reaches `DesertLooter.ini` and Desert Looter reloads it within a second.

### Fixed

- The menu is no longer blown out and oversaturated on an HDR display. The game presents an HDR10
  (PQ) swapchain, and hudhook wrote imgui's sRGB colours into it unconverted; the vendored hudhook
  now tracks the swapchain's colour space (`IDXGISwapChain3::SetColorSpace1`, DXGI's default for
  the back buffer format when the game never calls it) and converts in its pixel shader. Two new
  ini keys steer it: `HdrBrightness` (paper white in nits, default `203`, `80` to `1000`) and
  `ColorSpace` (`auto`, `sdr`, `hdr10`, `scrgb`, default `auto`). SDR is untouched: the shader's
  passthrough mode is byte for byte what it did before.

### Added

- A section whose plugin is not loaded in the game is greyed out and titled "not installed", with
  a note under the header. The check asks the Windows loader for `DesertLooter.asi` and
  `DesertGatherer.asi` once a second, so a file that is present but failed to load counts as
  absent. The presets follow the Desert Looter section.
- `Scale` ini key: menu size, `0` follows the Windows display scaling, otherwise a fixed factor
  from `0.5` to `4`. Fonts, spacing and the window geometry all follow it.
- First version of the plugin: an in-game settings menu for Desert Looter and Desert Gatherer,
  drawn with Dear ImGui inside the game's DirectX 12 frame (hudhook 0.9.2) and toggled with
  `Insert` (`KeyMenu`).
- Desert Looter section: `Enabled`, `AutoGather`, the four gather families, `GatherItems`,
  `GatherGear` and `GatherUnarmed` as checkboxes; `ScanRange` and `GatherRange` as sliders;
  `GatherInterval`, `NodeCooldown` and `StackLimit` as number fields. Every widget's range is the
  range the plugin accepts, so the menu cannot write a value that would be refused.
- Desert Gatherer section: `Enabled` and `DryRun` as checkboxes, and the four yield multipliers as
  1..100 sliders.
- Presets: `Everything`, `Plants only`, `Wood only`, `Rock and ore only`. Each sets the four gather
  families and `GatherItems` and leaves every other setting alone.
- Live in both directions. The ini files are the only channel to the plugins: a change in the menu
  is on disk within 250 ms and the plugins apply it within about a second, and a file edited by hand
  while the game runs shows up in the menu within a second.
- Writes preserve the file. Only the values of the keys the menu owns are replaced; comments, blank
  lines, section headers, key order, key spelling and unknown keys survive. A write is staged in
  `<name>.ini.tmp` and renamed over the original, so a plugin reading the file mid-write cannot see
  a partial one. A missing file is created with a short header rather than failing.
- Failures are visible, not fatal: a read or write that fails puts one red line under its section
  and one line in `DesertOverlay.log`, and nothing retries in a loop.
- hudhook's own `tracing` output is forwarded into `DesertOverlay.log`, so a graphics hook that
  fails to install says why: WARN and ERROR always, and every level including DEBUG and TRACE when
  `Debug=1`, capped at 20000 forwarded lines per session so a long game cannot fill the disk.
- hudhook is built from `vendor/hudhook`, a patched copy of 0.9.2. The released crate holds a
  reference to the game's swapchain across `ResizeBuffers` and across the game creating a
  replacement swapchain, which made Crimson Desert fail to launch: the resize failed with
  E_INVALIDARG and the retried `CreateSwapChainForHwnd` with E_ACCESSDENIED, because DXGI will not
  give a window a second swapchain while one is still alive. The patched copy releases everything
  before those calls and rebuilds on the next frame. See `vendor/hudhook/DESERT-CHANGES.md`.

### Fixed

- The window drew enlarged and clipped to part of itself whenever Windows display scaling was
  not 100%: the vendored hudhook set imgui's framebuffer scale to the DPI factor while its DX12
  renderer only scaled the viewport, not the scissor rectangles. The framebuffer scale is now 1
  and DPI is applied as font and style scaling instead.
- Two mouse cursors while the menu was open in the game's own screens: imgui now draws its own
  cursor only while Windows is not showing a hardware one.
