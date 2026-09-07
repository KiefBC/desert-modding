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
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES = ("desert-looter", "desert-gatherer", "desert-overlay", "desert-core")
DOCS = ("README.md", "VERSIONING.md")
# Crates that ship a plugin and carry a `**Version x.y.z**` line in their README.
SHIPPING = ("desert-looter", "desert-gatherer", "desert-overlay")

SEMVER = r"\d+\.\d+\.\d+"


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

    listed = ", ".join(f"{c} {v}" for c, v in versions.items())
    if check and stale:
        print(f"sync-versions: out of date: {', '.join(stale)}", file=sys.stderr)
        print(f"sync-versions: Cargo.toml says {listed}", file=sys.stderr)
        print("sync-versions: run `just sync-versions` and commit the result", file=sys.stderr)
        return 1
    if stale:
        print(f"sync-versions: updated {', '.join(stale)} ({listed})")
    else:
        print(f"sync-versions: already in sync ({listed})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
