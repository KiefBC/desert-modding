# Changelog

All notable changes to Desert Overlay. The format is [Keep a
Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows [Semantic
Versioning](../VERSIONING.md).

## [Unreleased]

Game build 25116796. In-game verification of the changes below is pending.

### Changed

- The menu no longer hard-codes Desert Looter's and Desert Gatherer's ini keys, defaults, ranges
  and widgets. Each plugin now declares a `desert_core::schema::Section` in its own `config.rs` and
  writes it as `<Name>.overlay.ini` beside its ini at every start; the overlay scans `bin64` for
  `*.overlay.ini` once a second, parses each through the new `desert_core::schema` module, and
  builds one collapsible section per file (`sections.rs` for discovery, `dynmodel.rs` for the
  values, replacing `model.rs` and `presets.rs`). Widgets, labels, ranges, headings, same-line rows,
  tooltips, presets and the notice line all now come from the schema, so a new mod appears in the
  menu by shipping a schema file - this crate needs no change. The header, the logo, the fonts, the
  themes and HDR handling are untouched.
- Desert Looter's section gained the four `Key*` bindings (`KeyToggle`, `KeyScan`, `KeyGather`,
  `KeyRecord`) as key pickers over the same key names the ini accepts; they were not editable from
  the menu before.
- New log lines: `[schema] <file>: <title>, N fields, M presets` when a section loads, `[schema]
  WARN <file>: <why>` when a schema file will not parse (once per modified time), `[schema] <file>
  is gone; <title> left the menu` when one disappears, and `[schema] no *.overlay.ini beside the
  game exe; the menu has nothing to show` when nothing is found. An empty menu now shows one dim
  line saying so instead of an empty window.

## [0.1.0] - 2026-09-08

Game build 25116796. Verified in game on 2026-09-07: the menu draws, moves and resizes, a preset
click reaches `DesertLooter.ini` and Desert Looter reloads it within a second.

### Added

- The menu window is titled "Desert Tooling" and opens with a header row: the goblin logo beside
  the name in a larger size of the menu font. The picture is embedded in the plugin as raw RGBA
  pixels, generated from `assets/logo.svg` by `tools/logo-to-rgba.py` (`just logo`); it is uploaded
  to the renderer once per graphics pipeline, and a failed upload is a warning in the log and a
  header with only its title.
- The menu is drawn in a real TrueType font instead of Dear ImGui's 13 px bitmap face blown up by
  the scale factor, which is what made it hard to read on a 4K display. Two ini keys steer it:
  `FontSize` (pixels before `Scale`, default `20`, `8` to `72`) and `Font` (default `segoeui.ttf`,
  a bare name looked up in `%WINDIR%\Fonts`, an absolute path taken as it stands, empty for the
  built-in font, and an optional `:N` suffix to pick a face out of a `.ttc` collection).
  `georgia.ttf`, `constan.ttf` and `cambria.ttc:0` are the serif options. The file is read once on
  the plugin's own thread; a missing or unreadable one is a warning in the log and the built-in
  font, never a failure to draw.
- Colour themes. A `Theme` picker at the top of the menu switches the look live and logs the name
  to put in the ini; the `Theme` ini key (default `banner`, gold lettering on the game's black banner; `classic` is the stock Dear ImGui dark look) makes
  it stick. Five themes drawn from the game's key art (off-white poster ground, antique gold
  lettering, the blood-red splash, the black banner, steel armour): `parchment`, `gilded`,
  `splash`, `banner` and `steel`. Themes are plain data, one file each under
  `src/themes/`, checked by unit tests for range and unique names.
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

- The menu is no longer blown out and oversaturated on an HDR display. The game presents an HDR10
  (PQ) swapchain, and hudhook wrote imgui's sRGB colours into it unconverted; the vendored hudhook
  now tracks the swapchain's colour space (`IDXGISwapChain3::SetColorSpace1`, DXGI's default for
  the back buffer format when the game never calls it) and converts in its pixel shader. Two new
  ini keys steer it: `HdrBrightness` (paper white in nits, default `203`, `80` to `1000`) and
  `ColorSpace` (`auto`, `sdr`, `hdr10`, `scrgb`, default `auto`). SDR is untouched: the shader's
  passthrough mode is byte for byte what it did before.
- The window drew enlarged and clipped to part of itself whenever Windows display scaling was
  not 100%: the vendored hudhook set imgui's framebuffer scale to the DPI factor while its DX12
  renderer only scaled the viewport, not the scissor rectangles. The framebuffer scale is now 1
  and DPI is applied as font and style scaling instead.
- Two mouse cursors while the menu was open in the game's own screens: imgui now draws its own
  cursor only while Windows is not showing a hardware one.
