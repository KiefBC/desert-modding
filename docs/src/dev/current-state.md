# Current state

Sources: `git tag`, `git ls-remote --tags origin`, `gh release list`, `git log` on `dev` and `main`, `Cargo.toml` files, `desert-gatherer-dmm/dmm_pack.json`, `crates/desert-tooling/CHANGELOG.md`, [Open design questions](open-questions.md), the GitHub branch-protection API. Collected on **2026-11-01**; everything below is a snapshot of that day.

## Game build

Steam build **26480954**, installed 2026-11-06 and launched the same day with the 26423880 build of the plugin: every hook installed (record loader `+0x3D9280`, stream layout parsed, catch hook `+0x2DDCBD8`), the `[banking]` gates resolved (`0x715A1D8` / `0x7131ED8`), the steal check `+0x28061C0` and its context `+0x58F4960` resolved, the live parsed layout was the 26361232 one, and both dispatch verdicts read `LOOKS RIGHT`. Offline, `just retarget report` found all 13 signatures once again (each moved up by between 0x7B0 and 0xDB0 from 26423880) and the census unchanged; the re-pin is again pure, and none of the generated tables changed a row. 32 `gimmickinfo` records changed (none with an output list) and the five Goblin Merchant item keys 26423880 retired came back beside their replacements. It is recorded in `docs/findings/2026-11-06-build-26480954-retarget.md`; `just test-game` green (game_exe 2/2, gimmick_real 11/11, banking_real 1/1).

Before it, Steam build **26423880**, installed 2026-11-04 and never launched. `just retarget report` found all 13 signatures once (each moved up by between 0x600 and 0x5A60) and the accessor census unchanged at 154 = 132 + 20 + 2 + 0 with the stream fill at `+0x20`/`+0x28`/`+0x2C` on all 154 sites, so the plugin side is a pure re-pin of the build pins, the walk calibrations and the generated files; no constant in shipped code was re-derived. On the table side two record layouts grew (`ItemInfo` gained two strings after `filterType`, `SpawningPoolAutoSpawnInfo` a `u8`), 17 `House_Construction_*` conditions name a game variable no table defines, and five Goblin Merchant items were re-keyed; the event ids did not change. It is recorded in `docs/findings/2026-11-04-build-26423880-retarget.md`, with the catalogue diff in `docs/findings/2026-11-04-catalog-diff.md`. **26423880 has not been launched yet**: everything verified in game below was verified on 26361232. `just test-game` green (game_exe 2/2, gimmick_real 11/11, banking_real 1/1)

Before that, Steam build **26361232**, installed 2026-11-01 about 16:55 UTC, the second hotfix of the day: **26345091** (2026-10-31) was followed by **26356046** (2026-11-01 05:07 UTC) and then by 26361232. 26345091 changed the compiler's codegen under the accessor template and five looter signatures, so 0.10.0 carries a re-target: derived on 26345091 with the new `retarget` tool and re-pinned to 26356046 and then to 26361232 the same day, each time when `just retarget report` came back clean on the hotfix (on 26361232 every signature sits 0x50 lower and the only verdict item was the build pin). It is recorded in `docs/findings/2026-11-01-build-26345091-retarget.md`; the one before it, 25477059, in `docs/findings/2026-09-24-build-25477059-retarget.md`. 26361232 has been launched in game, in dry runs only (`DryRun=1` in both sections). The first resolved and hooked everything and both dispatch verdicts passed, but the record loader's stream layout had moved, so the table dump was refused and no gather record was patched; 0.10.0 now reads that layout off the game's code (findings note §7.7). The dump then landed (2026-11-01 18:46 UTC: 17,621,227 bytes, 14,210 records, both `[banking]` gates `0`, dispatch back to vanilla with 0 changes) and exposed a second break: the gimmickinfo record header changed shape and every output list moved inside its record. 0.10.0 reads both header shapes (findings note §7.8). From that dump the walk calibrations are re-pinned (`paths::CALIBRATED_BUILD` is 26361232), `collect.rs` regenerates byte-identical. DMM pack 1.3 was rebased onto 26361232 the same day; since 2026-11-02 the pack is generated from the archive table and `collect.rs` by `dmm-pack` (1.4), see below. `just test-game` passes on 26361232: `game_exe` 2/2, `gimmick_real` 10/10, `banking_real` 1/1. Live launches followed (19:28 to about 21:15 UTC, findings note §7.9 and §7.10). **Verified in game on 26361232:** the load-time patch (`[stat] ... recognised 279, patched 224, scalars written 928` with `Foraging` and `Logging` at 2x), the live re-apply (the parsed record layout moved and is now probed: `[live] parsed layout: key +0x10, list +0x128 -> +0x0, ...`, then `0 skipped` on every slider move, and the user saw yields change on the next pick), the catch hook (a bug, class `0x80`, and a fish, `0x23`, both multiplied; `0x44` is now a fish class), `Hunting` (one carcass, 4 drop-set rows multiplied) the dispatch read side (both verdicts `LOOKS RIGHT`, `[banking]` 0 / 0) and, on a ninth launch, the dispatch write path: 32 lever changes through the in-game menu, every mission pass `939 entries visited, 0 skipped`, the reward pass at `Rewards=36` writing `693 changed, 0 skipped; 1386 amounts written`, `NoSkillRequirement` and `AnyOperatorCount` writing their fields, the revert rewriting all 939 missions, no `[apply] WARN`, and the user confirmed durations and payouts in game. Remaining notes: F9's manual gather was not exercised, and one launch-4 bug grant (`cat=2E type=03`) that logged nothing is unexplained and not reproduced.

## Branches

| Branch | Head | What it holds |
| --- | --- | --- |
| `dev` | `3db8ae1`, 2026-09-25 | `desert-tooling` 0.9.0, untagged, plus an `[Unreleased]` CHANGELOG entry. 8 commits ahead of `main`. Same as `origin/dev`. The 0.10.0 merge of `desert-looter` and `desert-gatherer` into `desert-harvest` goes on top of it. |
| `main` | `83391c7`, 2026-09-13 | The merge of PR #5, which released 0.6.0. Same as `origin/main`. |

`main` is the release branch: `release.yml` refuses a tag whose commit is not on `main`. Work lands on `dev` and reaches `main` through a pull request. `main` has **no branch protection** (the API answers "Branch not protected"), so the `ci` check reports but does not block a merge. `ci.yml` runs on pull requests, on pushes to `main` and weekly; not on pushes to `dev`.

## Versions in source

| Package | Version | Source |
| --- | --- | --- |
| `desert-tooling` (the plugin) | 0.10.0 | `crates/desert-tooling/Cargo.toml` |
| `desert-core` | 0.7.0 | `crates/desert-core/Cargo.toml` |
| `desert-harvest` | 0.6.0 | `crates/desert-harvest/Cargo.toml` (the merge of `desert-looter` 0.5.0 and `desert-gatherer` 0.4.0) |
| `desert-overlay` | 0.3.0 | `crates/desert-overlay/Cargo.toml` |
| `desert-dispatch` | 0.3.0 | `crates/desert-dispatch/Cargo.toml` |
| DMM pack | 1.4 | `desert-gatherer-dmm/dmm_pack.json` |

`just check-versions` prints the same list from the sources and is the answer to trust over this table.

## Tags and releases

Six tags exist, identical locally and on `origin`:

| Tag | Date | GitHub release |
| --- | --- | --- |
| `desert-tooling-v0.6.0` | 2026-09-13 | "Desert Tooling 0.6.0", marked Latest |
| `desert-gatherer-dmm-v1.1` | 2026-09-07 | "Desert Gatherer DMM pack 1.1" |
| `desert-looter-v0.1.1` | 2026-09-07 | "Desert Looter 0.1.1" |
| `desert-gatherer-v0.1.1` | 2026-09-07 | "Desert Gatherer 0.1.1" |
| `desert-looter-v0.1.0` | 2026-09-07 | none |
| `desert-gatherer-v0.1.0` | 2026-09-07 | none |

So 0.7.0, 0.8.0 and 0.9.0 exist only on `dev`, with CHANGELOG entries and no tag (0.10.0 likewise once committed). DMM pack 1.4 is in source and unreleased. Since 2026-11-02 no tool reads anything from the DMM tool (its backups, its dumps, its path dictionary): every offline tool reads the game's own archives, and `dmm-pack` generates the pack from them, replacing `dmm-rebase`. The four `desert-looter-v*` and `desert-gatherer-v*` tags are the pre-merge plugins' and name crates that no longer exist; they stay as history. The `Tagged` column in `VERSIONING.md` and the release links in the root `README.md` already name `desert-tooling-v0.10.0`: `just sync-versions` writes them from `Cargo.toml`, so they name the next release by design (see [Known drift](known-drift.md)).

## Open design questions

Nine design questions are still open or decided and not yet built, from the item-instance lever to the item display-name table, plus the money pouch lever found on 2026-11-01. Each has its status and the one check that would settle it on [Open design questions](open-questions.md).
