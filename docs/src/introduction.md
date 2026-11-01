# Introduction

Source: root `README.md`, the header comment of the root `Cargo.toml`.

Desert Tooling is one ASI plugin for Crimson Desert Enhanced, written in Rust and cross-compiled to Windows x64. Ultimate ASI Loader, installed as `winmm.dll` in the game's `bin64`, loads it. It is supported on Steam build 26361232.

The plugin is one `DesertTooling.asi`, one `DesertTooling.ini` and one `DesertTooling.log`. Three subsystems live inside it, each with its own section of the ini and its own tag on every log line it writes:

| subsystem | what it does | ini section | log tag |
|---|---|---|---|
| Harvest | auto-loot for gathering nodes, and the gathering yield multipliers | `[Harvest]` | `[harvest]` |
| Overlay | the in-game settings menu, `Insert` | `[Overlay]` | `[overlay]` |
| Dispatch | faster dispatch missions, bigger mission rewards, a dump of both into the log, and a read-only buff/stat census (`[buffs]`) | `[Dispatch]` | `[dispatch]` |

The workspace also holds `desert-core`, the shared plumbing every subsystem links (logging, safe memory reads, hooks, PE and pattern scanning), and `desert-gatherer-dmm/`, the older offset-patch version of the gathering multiplier for Definitive Mod Manager users without an ASI loader. Never mount that pack alongside the plugin.

## How this book is organised

- **Using the mod**: short pages for players (install, menu and keys, settings, the log, game updates), then the full player manual, the changelog and the DMM pack's README, included verbatim from the repository.
- **Developing**: how the workspace is built, tested and laid out, and how each subsystem works, plus the tracked developer documents (tools, versioning, design notes, the hudhook fork) included verbatim.
- **Reverse-engineering record (local only)**: the private RE record in `docs/`, mounted as it is. Those files are gitignored and exist only in the maintainer's checkout. On a fresh clone the book still builds, and those pages show an unresolved include instead of content.

Pages marked "Included verbatim" are the repository files themselves; edit the original, not the page.

## Where versions live

This book's own pages name no version numbers. The current versions of every crate and of the DMM pack are in the table at the top of [Versioning and releases](repo/VERSIONING.md), and the first line the plugin writes to `DesertTooling.log` names the plugin's version and each subsystem's.
