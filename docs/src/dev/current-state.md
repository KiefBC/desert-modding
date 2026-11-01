# Current state

Sources: `git tag`, `git ls-remote --tags origin`, `gh release list`, `git log` on `dev` and `main`, `Cargo.toml` files, `desert-gatherer-dmm/dmm_pack.json`, `desert-tooling/CHANGELOG.md`, `TODO.md`, the GitHub branch-protection API. Collected on **2026-11-01**; everything below is a snapshot of that day.

## Game build

Steam build **25477059**, installed 2026-09-24. The plugin resolved everything it needs on that build without a code change; the re-target pass (test pins, tool calibration, DMM pack rebase) is recorded in `docs/findings/2026-09-24-build-25477059-retarget.md`.

## Branches

| Branch | Head | What it holds |
| --- | --- | --- |
| `dev` | `3db8ae1`, 2026-09-25 | `desert-tooling` 0.9.0, untagged, plus an `[Unreleased]` CHANGELOG entry. 8 commits ahead of `main`. Same as `origin/dev`. |
| `main` | `83391c7`, 2026-09-13 | The merge of PR #5, which released 0.6.0. Same as `origin/main`. |

`main` is the release branch: `release.yml` refuses a tag whose commit is not on `main`. Work lands on `dev` and reaches `main` through a pull request. `main` has **no branch protection** (the API answers "Branch not protected"), so the `ci` check reports but does not block a merge. `ci.yml` runs on pull requests, on pushes to `main` and weekly; not on pushes to `dev`.

## Versions in source

| Package | Version | Source |
| --- | --- | --- |
| `desert-tooling` (the plugin) | 0.9.0 | `desert-tooling/Cargo.toml` |
| `desert-core` | 0.7.0 | `desert-core/Cargo.toml` |
| `desert-looter` | 0.5.0 | `desert-looter/Cargo.toml` |
| `desert-gatherer` | 0.4.0 | `desert-gatherer/Cargo.toml` |
| `desert-overlay` | 0.3.0 | `desert-overlay/Cargo.toml` |
| `desert-dispatch` | 0.3.0 | `desert-dispatch/Cargo.toml` |
| DMM pack | 1.2 | `desert-gatherer-dmm/dmm_pack.json` |

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

So 0.7.0, 0.8.0 and 0.9.0 exist only on `dev`, with CHANGELOG entries and no tag, and DMM pack 1.2 is in source and unreleased. The `Tagged` column in `VERSIONING.md` and the release links in the root `README.md` already name `desert-tooling-v0.9.0`: `just sync-versions` writes them from `Cargo.toml`, so they name the next release by design (see [Known drift](known-drift.md)).

## Open design entries in `TODO.md`

Headings whose **Status** line is not resolved or shipped, in file order:

| Entry | Status |
| --- | --- |
| The item-instance lever | OPEN, untested |
| Trade goods: 29 records that are not one line each, and 397 that are a different problem | OPEN, re-costed 2026-09-12 |
| The second yield route the gatherer multiplies nothing in | OPEN |
| The unclassified records: 297 carrying 308 blocks | OPEN |
| Digging: a gathering activity the mod cannot even see | OPEN |
| Extracting item names and the category triple from the `.paz` archives | OPEN, deliberately deferred |
| Drop *rate* levers | original lead CLOSED; a larger one it uncovered is OPEN |
| Two tagged, block-carrying records the tag census says belong in `Logging` | DECIDED to add, NOT DONE |
| An item display-name table | DECIDED, design agreed, nothing built |

The full text is [Design notes (TODO.md)](../repo/TODO.md).
