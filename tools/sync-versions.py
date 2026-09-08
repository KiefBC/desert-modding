#!/usr/bin/env python3
"""Sync the version numbers in the docs with the crates' Cargo.toml.

Cargo.toml is the single source of truth (VERSIONING.md says so); this keeps
the tables and the tag examples in the markdown from drifting away from it.

    tools/sync-versions.py            rewrite the docs in place
    tools/sync-versions.py --check    exit 1 if a doc is stale, changing nothing

Three things get rewritten. In every file listed in DOCS:

  1. a table row whose first cell names a crate -> the cell holding a bare
     version number is set to that crate's version
  2. any `<crate>-v<version>` string anywhere -> the crate's version

and in each shipping crate's own README.md, additionally:

  3. the `**Version <version>**` line near the top -> that crate's version

The DMM pack is not a crate and has its own source of truth, dmm_pack.json,
which tools/release-notes.py reads for the `desert-gatherer-dmm-v<x.y>` tag.
Its twelve module files each repeat that version twice, so:

  4. every `"version": "<x.y>"` in desert-gatherer-dmm/*.json except the
     manifest itself -> the manifest's version

Finally, one thing is checked but never written, because only a human can
write it: each shipping crate's CHANGELOG.md must carry a `## [<version>]`
heading for the version in its Cargo.toml. VERSIONING.md step 3 and CLAUDE.md
both say the entry lands in the same commit as the bump; without this check
nothing notices a missing entry until the release workflow builds the notes,
which is after the tag has been pushed.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ("desert-looter", "desert-gatherer", "desert-overlay", "desert-core")
DOCS = ("README.md", "VERSIONING.md")
# Crates that ship a plugin and carry a `**Version x.y.z**` line in their README.
SHIPPING = ("desert-looter", "desert-gatherer", "desert-overlay")

# The DMM pack: a directory of JSON, versioned x.y, not a crate.
PACK_DIR = ROOT / "desert-gatherer-dmm"
PACK_MANIFEST = PACK_DIR / "dmm_pack.json"

SEMVER = r"\d+\.\d+\.\d+"
# A version *value* inside a JSON string: digits and dots, so the substitution
# cannot touch some future `"version": "unreleased"` by accident.
JSON_VERSION = r"\d[\d.]*"


def crate_versions() -> dict[str, str]:
    """Read `version` from the [package] section of each crate's Cargo.toml."""
    versions = {}
    for crate in CRATES:
        manifest = ROOT / crate / "Cargo.toml"
        text = manifest.read_text(encoding="utf-8")
        # Only the [package] table; stop at the next section header.
        package = re.split(r"^\[", text, flags=re.M)[1]
        match = re.search(rf'^version\s*=\s*"({SEMVER})"', package, re.M)
        if not match:
            sys.exit(f"sync-versions: no package version in {manifest}")
        versions[crate] = match.group(1)
    return versions


def sync(text: str, versions: dict[str, str]) -> str:
    out = []
    for line in text.splitlines(keepends=True):
        named = [c for c in CRATES if c in line]
        # Longest name wins: "desert-looter" is not a substring of another
        # crate today, but a future "desert-looter-cli" would be ambiguous.
        if named and line.lstrip().startswith("|"):
            crate = max(named, key=len)
            version = versions[crate]
            cells = line.split("|")
            for i, cell in enumerate(cells):
                if re.fullmatch(rf"\s*{SEMVER}\s*", cell):
                    cells[i] = cell.replace(cell.strip(), version)
            line = "|".join(cells)
        for crate, version in versions.items():
            line = re.sub(rf"{re.escape(crate)}-v{SEMVER}", f"{crate}-v{version}", line)
        out.append(line)
    return "".join(out)


def sync_readme(text: str, crate: str, versions: dict[str, str]) -> str:
    """A shipping crate's README: the general rules plus its own Version line."""
    text = sync(text, versions)
    return re.sub(
        rf"^(\*\*Version ){SEMVER}(\*\*)",
        rf"\g<1>{versions[crate]}\g<2>",
        text,
        count=1,
        flags=re.M,
    )


def pack_version() -> str:
    """The DMM pack's version, from its own manifest."""
    version = json.loads(PACK_MANIFEST.read_text(encoding="utf-8")).get("version")
    if not isinstance(version, str) or not re.fullmatch(JSON_VERSION, version):
        sys.exit(f"sync-versions: no usable version in {PACK_MANIFEST}")
    return version


def sync_pack_module(text: str, version: str) -> str:
    """Every version value in a DMM module file follows the manifest.

    There are two per file, the module's own and the one repeated inside
    `modinfo`, and both are the pack's version.
    """
    return re.sub(rf'("version"\s*:\s*"){JSON_VERSION}(")', rf"\g<1>{version}\g<2>", text)


def missing_changelog_entries(versions: dict[str, str]) -> list[str]:
    """Shipping crates whose CHANGELOG has no heading for their own version."""
    missing = []
    for crate in SHIPPING:
        path = ROOT / crate / "CHANGELOG.md"
        heading = rf"^## \[{re.escape(versions[crate])}\]"
        if not re.search(heading, path.read_text(encoding="utf-8"), re.M):
            missing.append(f"{crate}/CHANGELOG.md has no `## [{versions[crate]}]` entry")
    return missing


def main() -> int:
    check = "--check" in sys.argv[1:]
    versions = crate_versions()
    stale = []

    targets = [(name, ROOT / name, None) for name in DOCS]
    targets += [(f"{c}/README.md", ROOT / c / "README.md", c) for c in SHIPPING]

    for name, path, crate in targets:
        before = path.read_text(encoding="utf-8")
        after = sync_readme(before, crate, versions) if crate else sync(before, versions)
        if before == after:
            continue
        stale.append(name)
        if not check:
            path.write_text(after, encoding="utf-8")

    pack = pack_version()
    for path in sorted(PACK_DIR.glob("*.json")):
        if path == PACK_MANIFEST:
            continue
        before = path.read_text(encoding="utf-8")
        after = sync_pack_module(before, pack)
        if before == after:
            continue
        stale.append(str(path.relative_to(ROOT)))
        if not check:
            path.write_text(after, encoding="utf-8")

    listed = ", ".join(f"{c} {v}" for c, v in versions.items())
    listed += f", DMM pack {pack}"

    # Reported whether or not we are in --check mode: a missing entry cannot be
    # written for you, so `just sync-versions` must not exit 0 pretending it is
    # all done.
    missing = missing_changelog_entries(versions)
    if missing:
        for line in missing:
            print(f"sync-versions: {line}", file=sys.stderr)
        print(
            "sync-versions: write the entry in the same commit as the bump "
            "(VERSIONING.md step 3)",
            file=sys.stderr,
        )

    if check and stale:
        print(f"sync-versions: out of date: {', '.join(stale)}", file=sys.stderr)
        print(f"sync-versions: the sources of truth say {listed}", file=sys.stderr)
        print("sync-versions: run `just sync-versions` and commit the result", file=sys.stderr)
        return 1
    if stale:
        print(f"sync-versions: updated {', '.join(stale)} ({listed})")
    elif not missing:
        print(f"sync-versions: already in sync ({listed})")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
