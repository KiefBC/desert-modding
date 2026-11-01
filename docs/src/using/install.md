# Install and upgrade

Source: root `README.md` (Install), `crates/desert-tooling/README.md` (Upgrading, Install, Conflicts with DMM), `justfile` (`install`).

## Requirements

The game, and Ultimate ASI Loader present in `bin64` as `winmm.dll`. It loads every `.asi` file beside it.

## Install by hand

The release zip holds five files flat at its root: `DesertTooling.asi`, `DesertTooling.ini`, `README.md`, `CHANGELOG.md` and `LICENSE`. Extract it straight into `<game>\bin64\`, or copy just two files there:

- `DesertTooling.asi`, the plugin. A self-built `desert_tooling.dll` is renamed to this.
- `DesertTooling.ini`, the settings. Optional: if it is missing, the plugin writes one on the next launch with every key at its default.

To remove the plugin, delete `DesertTooling.asi`. Nothing is installed anywhere else and no game file is modified.

Developers can run `just install`, which copies the built plugin into `bin64` as `DesertTooling.asi`, refuses while `CrimsonDesert.exe` is running, and warns if a pre-merge `.asi` is still there. `CD_BIN64` overrides the game path.

## Install with DMM

Definitive Mod Manager accepts an ASI mod as a single `.asi` or a directory: drop the release zip onto DMM's window, or extract it under `<DMM>\mods\_asi\`. DMM copies the `.asi` and `.ini` into `bin64` and installs Ultimate ASI Loader as `winmm.dll` itself. Enable the mod in DMM's ASI tab. DMM grades the plugin **Suspicious** because it imports `WriteProcessMemory`, `ReadProcessMemory` and `VirtualProtect`; the manual's "About DMM's security scan" section explains why.

## Upgrading an ini with `[Looter]` and `[Gatherer]`

Older releases had a `[Looter]` and a `[Gatherer]` section in the ini. They are now one, `[Harvest]`, and there is nothing to do: on the first launch the plugin rewrites the two into one `[Harvest]` section where `[Looter]` was, keeping every value, your comments and any key you added, and leaves `[Overlay]` and `[Dispatch]` exactly as they were. The log says so once:

```text
[tooling] [ini] [Looter] and [Gatherer] were merged into [Harvest] (Enabled is on only if both were, Debug if either was)
```

The section now has one `Enabled` and one `Debug`. `Enabled` comes out on only if both old sections had it on, so nothing starts that you had switched off, and when the two disagreed a comment above the new line says so. If what you wanted was the yield multipliers without auto-loot, that is `AutoGather=0` now. A file that already has `[Harvest]` is left alone; any old section still beside it is not read, and the log warns about it.

## Upgrading from the three separate plugins

Desert Looter, Desert Gatherer and Desert Overlay are this plugin.

1. Delete `DesertLooter.asi`, `DesertGatherer.asi` and `DesertOverlay.asi` from `bin64`, with their `.ini`, `.log` and `.overlay.ini` files.
2. Install as above.
3. Re-enter your settings. Nothing is migrated from those plugins: their ini files are not read.

**The stale-`.asi` guard.** Two copies of the same hook over one game function crash the game. So before installing anything, the plugin asks the Windows loader for those three module names, and if any is loaded it installs nothing, starts no subsystem, and names the file to delete:

```text
[tooling] REFUSING TO START: DesertLooter.asi still loaded in this process alongside DesertTooling.asi
```

## Never mount the DMM pack alongside the plugin

DMM's "The Desert Gatherer" module pack (the `desert-gatherer-dmm/` directory in this repository) and DMM's built-in gathering multiplier preset change the same minimum and maximum quantities on disk that the plugin multiplies. Mounted together they stack: a DMM 5X plus `Mining=2` gives 10x. Unmount both in DMM before using the plugin. The pack is the plugin's predecessor, not its companion.
