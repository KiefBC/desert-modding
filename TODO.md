# TODO

Open design questions, with enough context to pick them up cold. Not a task list:
each entry is a decision that has been discussed. An entry that has since been decided
keeps its argument and gains a **Resolution** at the bottom, so the reasoning survives
without reading as a live question - check an entry's **Status** line before acting on it.

## One ASI per mod instead of a separate overlay DLL

**Raised** 2026-09-08. **Status: DECIDED and shipped, 2026-09-08.** Everything from here to
"Recommendation" is the argument as it stood *before* the decision and is kept as the record
of it; it describes a three-plugin world that no longer exists. The outcome is at the bottom.

### Where things stood when this was raised (historical)

Three plugins: `DesertLooter.asi`, `DesertGatherer.asi` and `DesertOverlay.asi`. The
overlay is an in-game menu (hudhook DX12 + imgui) that edits the two mods' ini files
live; the mods hot-reload them. The overlay greys out the section of any plugin that
is not loaded (checked through the Windows loader once a second), so it is safe to
ship with either mod alone. Since 2026-09-08 every mod zip bundles `DesertOverlay.asi`
and `DesertOverlay.ini` beside the mod's own files, and the overlay also has its own
zip as the canonical release (`tools/dist.sh`, VERSIONING.md).

### The question

Could each mod's DLL contain the overlay, so a player gets one `.asi` per mod rather
than two, with the same "grey out what is not installed" logic deciding which
sections show?

### How it would work

The overlay is already a library crate, so each mod DLL could link it and start it
from its own main thread. With both mods installed two copies would load, each with
its own MinHook and imgui, both wanting to hook Present. So the copies need an
election before hooking: each registers its overlay version in a small shared-memory
section, waits about two seconds for the other to appear, and only the highest
version installs the hooks. The game takes over ten seconds to create its swapchain,
so the wait costs nothing. Estimate: about a day, plus in-game testing of the
one-mod, both-mods and version-skew cases.

### What it costs

- The DirectX and imgui code, plus the d3d12, dxgi and d3dcompiler imports, go into
  the looter and gatherer DLLs. Those two are deliberately tiny and dumb (8 imports
  each, see `just check-imports`). The 2026-09-07 launch failure was a hudhook bug;
  merged, it would have taken both mods down instead of one file a player can delete.
- Debugging gets harder. "Does it crash without the overlay?" becomes an ini edit
  instead of removing a file, and every bug report mixes three components in one
  binary.
- Every overlay fix forces a release of both mods, and the two copies in the wild
  drift apart. The election hides that, but nobody can tell which copy is drawing.
- The gatherer's load-time inline patch is meant to stay as small and boring as
  possible; linking a hooking library into that DLL goes the other way.

### What it gains

One file per mod instead of two. The zips already give that at install level
(extract into bin64, done; DMM takes the zip as one mod), so the gain is mostly how
the file list looks on the Nexus page.

### Recommendation as of 2026-09-08

Keep the overlay as its own DLL bundled in every zip. If the two-file look is the
concern, fix it in the page description ("includes the in-game menu").

### Third option, not planned at the time

One combined `.asi` with everything inside and features switched per ini. No election
problem, one file, but it merges two separately versioned products into one (separate
Nexus files, separate tags, separate changelogs). That is a product decision, not an
overlay tweak; plan it properly if it is wanted.

### Resolution (2026-09-08)

The third option is what shipped, planned properly as its own piece of work: one
`DesertTooling.asi` (`desert-tooling` 0.3.0), one `DesertTooling.ini` with a `[Looter]`,
`[Gatherer]` and `[Overlay]` section, one tagged `DesertTooling.log`. The three plugin
crates became untagged internal rlibs exposing `start()`; `desert-tooling` owns the only
`DllMain`. The product question the entry flagged was answered by folding the three
products into one: one Nexus file, one tag, one changelog (`VERSIONING.md` and
`tools/nexus-targets.json` carry the new shape).

What the objections above cost, and what was done about them:

- **DirectX and imgui in every mod DLL.** Moot: there is one DLL, and `just check-imports`
  holds it to the allowlist that used to be the overlay's.
- **The election problem.** Never arose: one process, one copy of hudhook, no shared-memory
  handshake. Instead there is a stale-`.asi` guard - `desert-tooling` asks the loader for
  `DesertLooter.asi`, `DesertGatherer.asi` and `DesertOverlay.asi` and refuses to install
  anything if one of them answers, because two trampolines over one prologue is a crash.
- **"Does it crash without the overlay?" becomes an ini edit.** It does, and that is what
  `Enabled=0` under `[Overlay]` is for: no graphics hook is installed at all and the other
  two subsystems are untouched.
- **Every overlay fix forces a release of both mods.** True and accepted; there is one
  release now, so there is nothing to drift apart.

Do not reopen this by re-reading the argument above without the resolution.
