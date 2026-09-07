# Versioning

Both plugins follow [Semantic Versioning](https://semver.org/), and each one carries its own
version: a single commit can bump one and leave the other exactly where it was.

| Crate | Version | Ships | Tagged |
| --- | --- | --- | --- |
| `desert-looter` | 0.1.1 | `DesertLooter.asi` | `desert-looter-v0.1.1` |
| `desert-gatherer` | 0.1.1 | `DesertGatherer.asi` | `desert-gatherer-v0.1.1` |
| `desert-core` | 0.2.0 | nothing | never |

## What the version actually describes

The version is about what a *player* sees, not what the source looks like. So the interface is:

- **The ini file**: every key, its default, the values it takes. This is the big one; most bumps
  live here.
- **The log prefixes people grep for**: `[gimmick]`, `[recv]`, `[stat]`, `[dry]`, `[gather]`,
  `[sig]`, `[hook]`, `[ini]`, `[survey]`. The prefix and the fields on the line count; the prose
  after it doesn't.
- **The shipped file names**: `DesertLooter.asi`, `.ini`, `.log`, `.yields`, and the same set for
  the gatherer.
- **The hotkeys** and their defaults (F9, F10, F11, F7).

Rust APIs, signature bytes, module layout, the RE notes: none of that is the interface. Rewriting
`game.rs` from scratch with identical behaviour is a PATCH, however much work it was.

## What bumps what

**MAJOR** means someone's existing install behaves differently after upgrading, without them
touching a thing:

- an ini key removed or renamed
- a default changed in a way that changes behaviour for someone already running it
- a multiplier meaning something new (say `Foraging=2` stops doubling every output block)
- a feature, a hotkey or a shipped file going away

**MINOR** means something new, nothing broken:

- a new ini key, defaulted so today's behaviour is preserved
- a new feature, hotkey, or log line
- a new game build that brings a new capability along with it

**PATCH** means nothing new to learn:

- bug fixes
- re-targeting a new game build with no interface change (new signatures, new offsets, same keys,
  same behaviour)
- wording, comments, refactors, tests

One caveat while both are below 1.0: **a MINOR can still break things.** These are young and still
finding their shape, so defaults and keys can move without a MAJOR. Skim the CHANGELOG before
upgrading.

## The milestone ladder

Below 1.0 a MINOR level isn't handed out for a pile of work. It gets *earned*, and each level has
exactly one thing to show for it. A release sits at the highest level it's genuinely met, and stays
put until the next one is real.

| Minor | Earned when |
| --- | --- |
| 0.1 | it works in game for its core purpose, on one build |
| 0.2 | it survived a game update on content-based resolution, a PATCH at most |
| 0.3 | no hard-coded addresses left |
| 0.4 | the feature set is decided and complete |
| 0.5+ | stabilising: no known bugs, docs done, only fixes landing |
| 1.0 | two game updates survived with no code changes, known-issues list empty |

Both sit at **0.1** today, for the same reason: each does the thing it exists to do, and each has
been run in the game on build 25116796. That's the whole of the evidence. Neither has seen a game
update, so neither has anything to show for 0.2 yet.

What they need for **0.2** is simply the next build. When it lands, re-target and re-verify. If
the signatures and content lookups carry over on a PATCH (new offsets, no new keys, no behaviour
change), the one that survived earns it. If a build forces a rewrite of how something gets found,
it wasn't earned; fix it and stay at 0.1.

After that, **0.3** wants the last hard-coded address gone. Gatherer already resolves everything by
content, so 0.3 is Looter's alone to earn: it still carries `GIMMICK_INFO_SLOT` and the other RVA
slots in [`desert-looter/src/tables.rs`](desert-looter/src/tables.rs). **0.4** wants the feature set
settled, which for Looter means creature catching decided: shipped, or ruled out on the record.

They climb independently. That they're level today is coincidence, not a rule.

## The version isn't the game build

A plugin's version says nothing about which build it runs on. That's stated separately: near the top
of each mod's README, and in the CHANGELOG entry for the release. The version itself is printed on
the log's first line.

When the game updates and the code has to move to keep up, that's a PATCH if players notice nothing,
and a MINOR if the new build needs something new from them: a key, a setting, a changed default.

## Cutting a release

1. Bump `version` in the crate's `Cargo.toml`. That's the single source of truth: the log's first
   line prints `CARGO_PKG_VERSION`, so nothing else needs editing to stay honest.
2. Run `just sync-versions` to pull that number through the version tables in this file and the
   README. CI checks it, so a stale table fails the build rather than shipping quietly.
3. Write the CHANGELOG entry in the same commit: version, date, game build, and Added / Changed /
   Fixed.
4. Build the packages:
   ```bash
   nix develop --command tools/dist.sh
   ```
   You get `dist/DesertLooter-<version>.zip`, `dist/DesertGatherer-<version>.zip` and
   `dist/SHA256SUMS`. Each zip holds the `.asi`, its `.ini`, the mod's README and CHANGELOG at the
   archive root, which is what lets one archive serve both a manual drop into `bin64` and
   Definitive Mod Manager.
5. Actually play it. Copy the `.asi` and `.ini` into `bin64`, launch, read the log. There's no
   automated in-game test, and a release nobody has run in the game isn't a release.
6. Commit, then tag `<crate>-v<version>`:
   ```bash
   git tag desert-looter-v0.1.1
   ```
7. Upload the zips and `SHA256SUMS` to the release.

## A note on desert-core

`desert-core` follows semver for its own Rust API (a removed or changed public function is a MAJOR,
a new module or function is a MINOR) purely so the two plugins can reason about it. It ships
nothing, is never tagged, never released on its own, and never shows up in a log line or an ini
file. Its version is bookkeeping between the two plugins, nothing more.
