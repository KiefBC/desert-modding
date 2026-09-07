# Versioning

Both shipped plugins follow [Semantic Versioning](https://semver.org/). The version is **per mod**,
not per repo: one commit can bump one crate and leave the other alone.

| Crate | Version | Ships | Tagged |
| --- | --- | --- | --- |
| `desert-looter` | 0.1.1 | `DesertLooter.asi` | `desert-looter-v0.1.1` |
| `desert-gatherer` | 0.1.0 | `DesertGatherer.asi` | `desert-gatherer-v0.1.0` |
| `desert-core` | 0.2.0 | nothing | never |

## What counts as the public interface

A mod's version describes what a **user** sees, not what the source looks like. The interface is:

- **The ini file** — every key, its default, and the values it accepts (`DesertLooter.ini`,
  `DesertGatherer.ini`). This is the big one; most bumps are here.
- **The log line prefixes** people grep for: `[gimmick]`, `[recv]`, `[stat]`, `[dry]`, `[gather]`,
  `[sig]`, `[hook]`, `[ini]`, `[survey]`. The wording after the prefix is not part of the interface,
  the prefix and the fields on the line are.
- **The shipped file names** — `DesertLooter.asi`, `DesertLooter.ini`, `DesertLooter.log`,
  `DesertLooter.yields`, and the same set for the gatherer.
- **The documented hotkeys** and their defaults (F9, F10, F11, F7).

Internal Rust APIs, signature bytes, module layout and the reverse-engineering notes are **not** the
interface. Rewriting `game.rs` end to end with the same behaviour is a PATCH.

## What bumps what

**MAJOR** — an existing install behaves differently after the user upgrades, without them changing
anything:

- removing or renaming an ini key
- changing a default in a way that changes behaviour for an existing user
- changing what a multiplier *means* (e.g. if `Foraging=2` stopped doubling every output block)
- dropping a feature, a hotkey or a shipped file

**MINOR** — new capability, nothing existing breaks:

- a new ini key (with a default that preserves today's behaviour)
- a new feature, a new hotkey, a new log line
- supporting a new game build that brings a new capability with it

**PATCH** — nothing new to learn:

- bug fixes
- re-targeting a new game build with no interface change (new signatures, new offsets, same keys and
  the same behaviour)
- doc and log **wording**, comments, refactors, test changes

### Pre-1.0: both plugins

Both plugins are below 1.0, so **a MINOR bump may break things**. They are still finding their
shape; defaults and keys can move without a MAJOR. Read the CHANGELOG before upgrading.

## The milestone ladder

Below 1.0 a MINOR level is not awarded for a batch of work — it is *earned*, and each level has one
criterion. A release sits at the highest level whose criterion is met, and stays there until the
next one is genuinely satisfied.

| Minor | Earned when |
| --- | --- |
| 0.1 | works in game for its core purpose, on one game build |
| 0.2 | survived a game update with the content-based resolution proven — at most a PATCH was needed |
| 0.3 | no hard-coded addresses left |
| 0.4 | feature set decided and complete |
| 0.5 and up | stabilising: no known bugs, docs complete, only fixes landing |
| 1.0 | two game updates survived without code changes, and nothing on the known-issues list |

Both plugins are at **0.1** today (looter 0.1.1, gatherer 0.1.0), and for the same reason: each does what it exists to do, and
has been run in the game on build 25116796 — and that is the whole of the evidence. Neither has
seen a game update yet, so neither has anything to show for 0.2.

What each needs for **0.2** is the next game build. When it lands, re-target and re-verify: if the
signatures and the content-based lookups carry over with at most a PATCH — new offsets, no new keys,
no behaviour change — the mod that survived it earns 0.2. If a build forces a rewrite of how
something is found, the level is not earned; fix it and stay at 0.1.

Beyond that: **0.3** needs the last hard-coded address gone. Desert Gatherer already resolves
everything by content; Desert Looter still carries `GIMMICK_INFO_SLOT` and the other RVA slots in
`desert-looter/src/tables.rs`, so 0.3 is the looter's alone to earn. **0.4** needs the feature set
settled — for Desert Looter that means creature catching decided, either shipped or ruled out on
the record.

The two plugins climb the ladder independently. They start level today by coincidence, not by rule.

## The version is not the game build

A plugin version says nothing about which game build it works on. The supported build is stated
separately: near the top of each mod's README, and on the CHANGELOG entry for the release. The
plugin's own version is printed on the log's first line.

When the game updates and the code has to change to keep up, that is a **PATCH** if the user-facing
behaviour is unchanged, and a **MINOR** if the new build needs something new from the user (a new
key, a new setting, a changed default).

## Release procedure

1. Bump `version` in the crate's `Cargo.toml`. That is the **single source of truth** — the log's
   first line prints `CARGO_PKG_VERSION`, so nothing else has to be edited to keep it honest.
2. Add the CHANGELOG entry, in the same commit as the bump: the version, the date, the game build,
   and Added / Changed / Fixed.
3. Build the release packages:
   ```bash
   nix develop --command tools/dist.sh
   ```
   That builds both plugins and writes `dist/DesertLooter-<version>.zip`,
   `dist/DesertGatherer-<version>.zip` and `dist/SHA256SUMS`. Each zip holds the
   `.asi`, its `.ini`, the mod's `README.md` and its `CHANGELOG.md` at the
   archive root, which is what makes one archive serve both a manual install
   into `bin64` and Definitive Mod Manager.
4. Verify in game: copy the `.asi` and `.ini` into `bin64`, launch, read the log. There is no
   automated in-game test; a release that has not been run in the game is not a release.
5. Commit, then tag `<crate>-v<version>`:
   ```bash
   git tag desert-looter-v0.1.0
   git tag desert-gatherer-v0.1.0
   ```
6. Upload the two zips from `dist/` and the `SHA256SUMS` to the release.

## desert-core

`desert-core` follows semver for its own Rust API — a removed or changed public function is a MAJOR,
a new module or function is a MINOR — so that the two plugins can reason about it. It ships nothing,
is never tagged, and is never released on its own. Its version is bookkeeping between the two
plugins, and it does not appear in any log line or ini file.
