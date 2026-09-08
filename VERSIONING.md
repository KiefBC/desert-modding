# Versioning

The plugins follow [Semantic Versioning](https://semver.org/), and each one carries its own
version: a single commit can bump one and leave the others exactly where they were.

| Crate | Version | Ships | Tagged |
| --- | --- | --- | --- |
| `desert-looter` | 0.1.1 | `DesertLooter.asi` | `desert-looter-v0.1.1` |
| `desert-gatherer` | 0.1.1 | `DesertGatherer.asi` | `desert-gatherer-v0.1.1` |
| `desert-overlay` | 0.1.0 | `DesertOverlay.asi` | `desert-overlay-v0.1.0` |
| `desert-core` | 0.3.0 | nothing | never |

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

desert-overlay earned its 0.1 on 2026-09-07 on build 25116796: the menu drew, took input, wrote
`DesertLooter.ini`, and Desert Looter reloaded the change within a second. It has no hard-coded
addresses to resolve at all, so its 0.2 and 0.3 come down to nothing but surviving a game update.

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
   nix develop --command tools/dist.sh                  # every package
   nix develop --command tools/dist.sh desert-looter    # just the one you are releasing
   ```
   You get `dist/DesertLooter-<version>.zip`, `dist/DesertGatherer-<version>.zip`,
   `dist/DesertOverlay-<version>.zip`, `dist/DesertGatherer-DMM-<version>.zip` and
   `dist/SHA256SUMS`. Each plugin zip holds the `.asi`, its `.ini`, the mod's README and CHANGELOG
   at the archive root, which is what lets one archive serve both a manual drop into `bin64` and
   Definitive Mod Manager. The Looter and Gatherer zips add `DesertOverlay.asi` and
   `DesertOverlay.ini` after those four, so either mod installs the in-game menu on its own; the
   overlay greys out the section of any plugin that is not loaded, so bundling it is safe either
   way. The release builds only the package its tag names, so pass that name here to see exactly
   what will ship.

   Bundling does not change the separate-versioning rule. A mod zip carries whichever overlay
   version was current when the mod was tagged, and an overlay-only change reaches those users
   through the overlay's own release or through the mod's next release, whichever comes first: an
   overlay bump never re-cuts a mod's zip by itself, and it is not a reason to bump the mod.
5. Actually play it. Copy the `.asi` and `.ini` into `bin64`, launch, read the log. There's no
   automated in-game test, and a release nobody has run in the game isn't a release.
6. Commit, get it onto `main` (the release branch), then tag the commit on `main` as
   `<crate>-v<version>` and push the tag:
   ```bash
   git tag desert-looter-v0.1.1
   git push origin desert-looter-v0.1.1
   ```
   The DMM pack is tagged `desert-gatherer-dmm-v<version>` with the version from `dmm_pack.json`.
7. Pushing the tag is the release. The `release` workflow (`.github/workflows/release.yml`) checks
   that the tagged commit is on `main` and that the tag's version equals the one in the source,
   re-runs the doc, clippy and test checks, builds **that package** with `tools/dist.sh` on a clean
   runner, and publishes a GitHub release named for the tag with its zip and `SHA256SUMS` attached.
   Only the tagged package: the four are versioned separately, so rebuilding all of them for every
   tag would eventually attach an untagged package's old version number to new bytes, and two
   release pages would disagree about what one version contains. The notes are the CHANGELOG entry
   for that version
   (`tools/release-notes.py`, which you can run locally to preview them). Any gate failing means
   nothing is published; fix and re-tag.
8. The same push then publishes to Nexus Mods, with no further action: the `nexus` job downloads
   the assets from the release it just made, checks them against `SHA256SUMS`, and adds that zip to
   its mod page as a new version of the existing file, with the CHANGELOG entry as the Nexus
   changelog. All four packages are separate files on one page (`crimsondesert/mods/3369`), so a
   tag only ever touches its own file. The mod page's version follows the file and the previous version is
   archived. Which page each tag goes to is `tools/nexus-targets.json`; a package with no entry
   there is skipped, and its GitHub release still happens. The API can only add a version to a file
   that already exists, so the mod page and its first file are created by hand on the site, once.
   The job needs the `NEXUS_API_KEY` repository secret (a personal API key from
   <https://www.nexusmods.com/settings/api-keys>).

## A note on desert-core

`desert-core` follows semver for its own Rust API (a removed or changed public function is a MAJOR,
a new module or function is a MINOR) purely so the plugins can reason about it. It ships
nothing, is never tagged, never released on its own, and never shows up in a log line or an ini
file. Its version is bookkeeping between the plugins, nothing more.
