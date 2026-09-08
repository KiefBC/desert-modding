#!/usr/bin/env python3
"""Turn a release tag into a release title and release notes.

    tools/release-notes.py desert-looter-v0.1.1              notes, as markdown, on stdout
    tools/release-notes.py desert-looter-v0.1.1 --title      just the title
    tools/release-notes.py TAG --sums dist/SHA256SUMS        notes plus a table of the attached files
    tools/release-notes.py desert-looter-v0.1.1 --changelog  just the changelog entry
    tools/release-notes.py desert-looter-v0.1.1 --package    just the package name

The tag names one package and its version (VERSIONING.md step 6):

    desert-looter-v<semver>        Desert Looter        version from desert-looter/Cargo.toml
    desert-gatherer-v<semver>      Desert Gatherer      version from desert-gatherer/Cargo.toml
    desert-overlay-v<semver>       Desert Overlay       version from desert-overlay/Cargo.toml
    desert-gatherer-dmm-v<x.y>     Desert Gatherer DMM  version from desert-gatherer-dmm/dmm_pack.json

The version in the tag must equal the one in that source of truth, or this
exits 1: a release built from a commit whose Cargo.toml disagrees with its tag
would print one number on the log's first line and carry another on the
release page. The release workflow runs this before it builds anything.

The notes are the package's own changelog entry for that version (the DMM
pack keeps its history in its README's "What Version" section), then, with
--sums, the SHA256 of every file attached to the release.

--changelog prints that entry and nothing else. It is what the release
workflow sends to the mod's Nexus Mods page, where the file list and the
checksum table would be noise: a Nexus page carries one package, and its
Files tab already shows what is attached.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

SEMVER = r"\d+\.\d+\.\d+"
PACK_VERSION = r"\d+\.\d+"


def cargo_version(crate: str) -> str:
    text = (ROOT / crate / "Cargo.toml").read_text(encoding="utf-8")
    package = re.split(r"^\[", text, flags=re.M)[1]
    match = re.search(rf'^version\s*=\s*"({SEMVER})"', package, re.M)
    if not match:
        sys.exit(f"release-notes: no package version in {crate}/Cargo.toml")
    return match.group(1)


def pack_version() -> str:
    manifest = ROOT / "desert-gatherer-dmm" / "dmm_pack.json"
    version = json.loads(manifest.read_text(encoding="utf-8")).get("version")
    if not isinstance(version, str) or not re.fullmatch(PACK_VERSION, version):
        sys.exit(f"release-notes: no version in {manifest}")
    return version


# tag prefix -> (display name, version pattern, source of truth, history file, heading for a version)
PACKAGES = {
    "desert-looter": (
        "Desert Looter",
        SEMVER,
        lambda: cargo_version("desert-looter"),
        "desert-looter/CHANGELOG.md",
        lambda v: rf"## \[{re.escape(v)}\]",
    ),
    "desert-gatherer": (
        "Desert Gatherer",
        SEMVER,
        lambda: cargo_version("desert-gatherer"),
        "desert-gatherer/CHANGELOG.md",
        lambda v: rf"## \[{re.escape(v)}\]",
    ),
    "desert-overlay": (
        "Desert Overlay",
        SEMVER,
        lambda: cargo_version("desert-overlay"),
        "desert-overlay/CHANGELOG.md",
        lambda v: rf"## \[{re.escape(v)}\]",
    ),
    "desert-gatherer-dmm": (
        "Desert Gatherer DMM pack",
        PACK_VERSION,
        pack_version,
        "desert-gatherer-dmm/README.md",
        lambda v: r"## What Version",
    ),
}


# Packages whose zip also carries DesertOverlay.asi and its ini (tools/dist.sh).
# The notes say which overlay version went in, because a mod zip carries
# whichever one was current at tag time and that is not the mod's own version.
BUNDLES_OVERLAY = ("desert-looter", "desert-gatherer")


def parse_tag(tag: str) -> tuple[str, str]:
    """`desert-gatherer-dmm-v1.1` -> ("desert-gatherer-dmm", "1.1"). Longest prefix wins."""
    for prefix in sorted(PACKAGES, key=len, reverse=True):
        pattern = PACKAGES[prefix][1]
        match = re.fullmatch(rf"{re.escape(prefix)}-v({pattern})", tag)
        if match:
            return prefix, match.group(1)
    expected = ", ".join(f"{p}-v<version>" for p in PACKAGES)
    sys.exit(f"release-notes: tag {tag!r} is not one of {expected}")


def section(path: Path, heading: str) -> str:
    """The body under the first `## ` heading matching `heading`, up to the next `## `."""
    lines = path.read_text(encoding="utf-8").splitlines()
    start = next((i for i, line in enumerate(lines) if re.match(heading, line)), None)
    if start is None:
        sys.exit(f"release-notes: no heading matching {heading!r} in {path.relative_to(ROOT)}")
    body = []
    for line in lines[start + 1 :]:
        if line.startswith("## "):
            break
        body.append(line)
    text = "\n".join(body).strip()
    if not text:
        sys.exit(f"release-notes: empty section under {heading!r} in {path.relative_to(ROOT)}")
    return text


def sums_table(sums: Path) -> str:
    rows = []
    for line in sums.read_text(encoding="utf-8").splitlines():
        digest, _, name = line.partition("  ")
        if not digest or not name:
            sys.exit(f"release-notes: unreadable line in {sums}: {line!r}")
        rows.append(f"| `{name}` | `{digest}` |")
    rows.append(f"| `{sums.name}` | the list above, for `sha256sum --check` |")
    return "\n".join(["| File | SHA256 |", "| --- | --- |", *rows])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("tag")
    parser.add_argument("--title", action="store_true", help="print only the release title")
    parser.add_argument(
        "--package", action="store_true", help="print only the package name, for tools/dist.sh"
    )
    parser.add_argument(
        "--changelog", action="store_true", help="print only the changelog entry, for Nexus Mods"
    )
    parser.add_argument("--sums", type=Path, help="dist/SHA256SUMS, to list the attached files")
    args = parser.parse_args()

    prefix, tagged = parse_tag(args.tag)
    name, _, source_version, history, heading = PACKAGES[prefix]

    actual = source_version()
    if tagged != actual:
        sys.exit(
            f"release-notes: tag {args.tag} says {tagged}, but the source says {actual}. "
            "Bump the version (VERSIONING.md step 1) or fix the tag; never release the mismatch."
        )

    if args.package:
        print(prefix)
        return 0

    title = f"{name} {tagged}"
    if args.title:
        print(title)
        return 0

    entry = section(ROOT / history, heading(tagged))
    if args.changelog:
        print(entry)
        return 0

    parts = [
        entry,
        "",
        "## Files",
        "",
        f"{title} only, built from the tagged commit. The other packages in this repository "
        "are versioned separately and each has its own tag and its own releases.",
    ]
    if args.sums:
        parts += ["", sums_table(args.sums)]
    if prefix in BUNDLES_OVERLAY:
        parts += ["", f"Bundles Desert Overlay {cargo_version('desert-overlay')}"]
    print("\n".join(parts))
    return 0


if __name__ == "__main__":
    sys.exit(main())
