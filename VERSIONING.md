# Versioning

The shipped plugin follows [Semantic Versioning](https://semver.org/). One `.asi` ships, so one
version number is the one a player ever sees; the library crates behind it carry their own numbers
for the workspace's benefit and are never tagged or released.

| Crate | Version | Ships | Tagged |
| --- | --- | --- | --- |
| `desert-tooling` | 0.3.0 | `DesertTooling.asi` | `desert-tooling-v0.3.0` |
| `desert-looter` | 0.2.0 | nothing, linked in | never |
| `desert-gatherer` | 0.2.0 | nothing, linked in | never |
| `desert-overlay` | 0.2.0 | nothing, linked in | never |
| `desert-core` | 0.5.0 | nothing, linked in | never |

The DMM offset-patch pack in `desert-gatherer-dmm/` is versioned separately off its own
`dmm_pack.json` and tagged `desert-gatherer-dmm-v<x.y>`. It is data, not a crate, and nothing below
about crates applies to it.

## What the version actually describes

The version is about what a *player* sees, not what the source looks like. So the interface is:

- **The ini file**: `DesertTooling.ini` — its `[Looter]`, `[Gatherer]` and `[Overlay]` sections,
  every key in them, its default, the values it takes. This is the big one; most bumps live here.
- **The log prefixes people grep for**: the subsystem tag every line carries (`[looter]`,
  `[gatherer]`, `[overlay]`, `[tooling]`) and the prefix after it (`[gimmick]`, `[recv]`, `[stat]`,
  `[dry]`, `[gather]`, `[sig]`, `[hook]`, `[ini]`, `[survey]`). The tag, the prefix and the fields on
  the line count; the prose after them doesn't.
- **The shipped file names**: `DesertTooling.asi`, `DesertTooling.ini`, `DesertTooling.log`, and the
  learned-yields cache written beside them.
- **The hotkeys** and their defaults (F9, F10, F11, F7, and `Insert` for the menu).

Rust APIs, signature bytes, module layout, which crate a subsystem lives in, the RE notes: none of
that is the interface. Rewriting `game.rs` from scratch with identical behaviour is a PATCH, however
much work it was — and so is moving code between `desert-looter` and `desert-core`, because a player
cannot tell.

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
- anything that makes the player install something they do not already have

**PATCH** means nothing new to learn:

- bug fixes
- re-targeting a new game build with no interface change (new signatures, new offsets, same keys,
  same behaviour)
- wording, comments, refactors, tests

One caveat while it is below 1.0: **a MINOR can still break things.** This is young and still
finding its shape, so defaults and keys can move without a MAJOR. Skim the CHANGELOG before
upgrading.

## The milestone ladder

Below 1.0 a MINOR level isn't handed out for a pile of work. It gets *earned*, and each level has
exactly one thing to show for it. A release sits at the highest level it's genuinely met, and stays
put until the next one is real.

| Minor | Earned when |
| --- | --- |
| 0.1 | it works in game for its core purpose, on one build |
| 0.2 | no hard-coded addresses left: everything it needs, it finds by content |
| 0.3 | the feature set is decided and complete |
| 0.4+ | stabilising: no known bugs, docs done, only fixes landing |
| 1.0 | the interface is settled and worth defending, known-issues list empty |

Every rung is something this project can go and do. That is deliberate, and it is a change: the
ladder used to gate 0.2 on *surviving a game update*, and 1.0 on surviving two.

### Why the game-update rung went away

It measured the game's behaviour, not the mod's. Crimson Desert updates often, and the rule cut
badly in both directions. A quiet build that changed nothing near us would have handed out a MINOR
for no work at all. A build that moved things would have meant real work — and the rule's own words
were "if a build forces a rewrite of how something gets found, it wasn't earned; fix it and stay at
0.1", so the harder the update hit, the longer the version stayed pinned. Doing more work lowered
the number. That is backwards.

It also put this document at odds with itself. "What bumps what" above says a new ini key defaulted
to today's behaviour is a MINOR. The looter added six of them and could not bump, because a patch
note somewhere else had not landed. Two rules, one number, opposite answers.

None of that means update-survival is uninteresting — it is the single most useful thing a player
can know about an ASI mod. It just isn't a version digit. It goes in the mod's README and on its
Nexus page as a plain statement of fact: which build it is verified on, and whether it has yet come
through an update. That says more than a `0.2` ever did, and it stays true whatever the version is.

### Where it sits

**0.3**, as of 2026-09-08.

There used to be three ladders here, one per shipped `.asi`, and all three cleared **0.2** on
2026-09-08. Desert Gatherer always did — it reaches the record loader through the accessor that
names `gimmickinfo`, and the catch site by signature. Desert Overlay had no game addresses to
resolve at all. Desert Looter was the last holdout, carrying `ITEM_INFO_SLOT` and `GIMMICK_INFO_SLOT`
as bare RVAs in [`desert-looter/src/tables.rs`](desert-looter/src/tables.rs); both are resolved by
content through `desert_core::gimmick`, and there is no bare image address left anywhere in the
workspace. That rung is cleared for the whole plugin, because it is cleared for every subsystem in
it.

**0.3** wants the feature set decided, and the merge is what decided it. The two open questions were
creature catching — settled by shipping it, under `GatherBugs` and `GatherFish` — and how many mods
this actually is. It is one: auto-loot, yield multipliers and the menu that edits both, in a single
`.asi` with a single ini, which is the shape it keeps.

### Why the first `desert-tooling` release is 0.3.0 and not 0.1.0

Two readings, one answer.

By the ladder: a version describes the mod as a player meets it, and the mod a player meets did not
get younger by being merged. All three of its parts had cleared 0.2 and shipped as 0.2.0; starting
the merged plugin at 0.1.0 would claim a regression that did not happen, and 0.3 is the rung the
merge itself clears.

By "What bumps what": merging is a MINOR against those 0.2.0 releases. It needs something new from
the player — delete three files, install one — and nothing an upgrader had is taken away: every ini
key, hotkey, log prefix and behaviour survives, moved into a `[Section]` and given a tag. 0.2.0 plus
a MINOR is 0.3.0.

The `desert-looter`, `desert-gatherer` and `desert-overlay` crates keep their 0.2.0 and stop moving
as player-facing numbers. They are internal libraries now, like `desert-core`: they ship nothing,
are never tagged, and their versions are bookkeeping between crates.

## The version isn't the game build

The version says nothing about which build it runs on. That's stated separately: near the top of the
mod's README, and in the CHANGELOG entry for the release. The version itself is printed on the log's
first line.

When the game updates and the code has to move to keep up, that's a PATCH if players notice nothing,
and a MINOR if the new build needs something new from them: a key, a setting, a changed default.

## Cutting a release

1. Bump `version` in `desert-tooling/Cargo.toml`. That's the single source of truth: the log's first
   line prints `CARGO_PKG_VERSION`, so nothing else needs editing to stay honest. (Bumping a library
   crate is bookkeeping and releases nothing; only `desert-tooling` is tagged.)
2. Run `just sync-versions` to pull that number through the version tables in this file, the top
   README and the mod's own README — and, for the DMM pack, from `dmm_pack.json` through its
   twelve module files. CI checks it, so a stale table fails the build rather than shipping
   quietly. You never edit a version by hand anywhere except the one source of truth.
3. Write the CHANGELOG entry in the same commit: version, date, game build, and Added / Changed /
   Fixed. `just sync-versions` fails until this exists — it is the one part of a bump that cannot
   be generated, and the release notes are built from it.
4. Build the packages:
   ```bash
   nix develop --command tools/dist.sh                   # both packages
   nix develop --command tools/dist.sh desert-tooling    # just the one you are releasing
   ```
   You get `dist/DesertTooling-<version>.zip`, `dist/DesertGatherer-DMM-<version>.zip` and
   `dist/SHA256SUMS`. The plugin zip holds `DesertTooling.asi`, `DesertTooling.ini`, the README, the
   CHANGELOG and `LICENSE` at the archive root, which is what lets one archive serve both a manual
   drop into `bin64` and Definitive Mod Manager. The release builds only the package its tag names,
   so pass that name here to see exactly what will ship.
5. Actually play it. Copy the `.asi` and `.ini` into `bin64` — after deleting any `DesertLooter.asi`,
   `DesertGatherer.asi` or `DesertOverlay.asi` left from before the merge — launch, read
   `DesertTooling.log`. There's no automated in-game test, and a release nobody has run in the game
   isn't a release.
6. Commit, get it onto `main` (the release branch), then tag the commit on `main` as
   `<package>-v<version>` and push the tag:
   ```bash
   git tag desert-tooling-v0.3.0
   git push origin desert-tooling-v0.3.0
   ```
   The DMM pack is tagged `desert-gatherer-dmm-v<version>` with the version from `dmm_pack.json`.
7. Pushing the tag is the release. The `release` workflow (`.github/workflows/release.yml`) checks
   that the tagged commit is on `main` and that the tag's version equals the one in the source,
   re-runs the doc, clippy and test checks, builds **that package** with `tools/dist.sh` on a clean
   runner, and publishes a GitHub release named for the tag with its zip and `SHA256SUMS` attached.
   Only the tagged package: the two are versioned separately, so rebuilding both for every tag would
   eventually attach an untagged package's old version number to new bytes, and two release pages
   would disagree about what one version contains. The notes are the CHANGELOG entry for that version
   (`tools/release-notes.py`, which you can run locally to preview them). Any gate failing means
   nothing is published; fix and re-tag.
8. The same push then publishes to Nexus Mods, with no further action: the `nexus` job downloads
   the assets from the release it just made, checks them against `SHA256SUMS`, and adds that zip to
   its mod page as a new version of the existing file, with the CHANGELOG entry as the Nexus
   changelog. Both packages are separate files on one page (`crimsondesert/mods/3369`), so a
   tag only ever touches its own file. The mod page's version follows the file and the previous
   version is archived. Which file each tag goes to is `tools/nexus-targets.json`; a package with no
   `file_id` there is skipped, and its GitHub release still happens. The API can only add a version
   to a file that already exists, so the mod page and its first file are created by hand on the site,
   once — which is why `desert-tooling`'s `file_id` is empty until someone uploads its first zip
   through the Files tab. The job needs the `NEXUS_API_KEY` repository secret (a personal API key
   from <https://www.nexusmods.com/settings/api-keys>).

## A note on the library crates

`desert-core`, `desert-looter`, `desert-gatherer` and `desert-overlay` follow semver for their own
Rust APIs (a removed or changed public function is a MAJOR, a new module or function is a MINOR)
purely so the crates can reason about each other. They ship nothing, are never tagged, are never
released on their own, and never show up in a log line or an ini file — the tag on a log line names
the subsystem, not the crate version. Their versions are bookkeeping inside the workspace, nothing
more.
