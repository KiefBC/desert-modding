#!/usr/bin/env python3
"""Turn a release tag into the Nexus Mods file it publishes to.

    tools/nexus-target.py desert-looter-v0.1.2

Prints `key=value` lines for `$GITHUB_OUTPUT`; the `nexus` job in
.github/workflows/release.yml appends them and reads them back as step outputs:

    publish         "true" or "false" - false means this package has no Nexus
                    target configured, and the job skips the rest of its steps
    version         0.1.2, from the tag
    zip             DesertGatherer-0.1.2.zip, the release's one package zip
    display_name    what the file is called on the Nexus page
    category        main / optional / miscellaneous, where the file sits
    update_mod_version  whether this file's version becomes the page's version
    changelog       whether to append the CHANGELOG entry to the page's changelog
    game_domain     crimsondesert
    game_scoped_id  the number in the mod page URL
    file_id         the Nexus file (update group) to add a version to

The mapping lives in tools/nexus-targets.json, which says where the two IDs
come from. Everything here is a lookup and a format check - no network calls,
so a misconfigured target fails in a second instead of half way through an
upload.

The tag's version is NOT re-checked against Cargo.toml here: release.yml
already ran tools/release-notes.py, which exits 1 on a mismatch, before
anything was built. This runs after that.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGETS = ROOT / "tools" / "nexus-targets.json"

# What the Nexus API accepts for a mod file version (openapi.yaml,
# CreateModFileVersionRequest). Checked here so a bad name is a red step
# before the upload rather than a 422 after the bytes are already in S3.
VERSION_RE = re.compile(r"^[a-zA-Z0-9.-]{1,50}$")
NAME_RE = re.compile(r"^[a-zA-Z0-9 _'().-]{1,50}$")
CATEGORIES = ("main", "optional", "miscellaneous")


def parse_tag(tag: str, prefixes: list[str]) -> tuple[str, str]:
    """`desert-gatherer-dmm-v1.1` -> ("desert-gatherer-dmm", "1.1"). Longest prefix wins."""
    for prefix in sorted(prefixes, key=len, reverse=True):
        if tag.startswith(f"{prefix}-v"):
            return prefix, tag[len(prefix) + 2 :]
    sys.exit(f"nexus-target: tag {tag!r} names no package in {TARGETS.name}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("tag")
    args = parser.parse_args()

    config = json.loads(TARGETS.read_text(encoding="utf-8"))
    targets = config["targets"]
    domain = config["game_domain"]

    prefix, version = parse_tag(args.tag, list(targets))
    target = targets[prefix]

    # One mod page holds all four files, so exactly one of them may own the
    # page's version field. Catch a second claimant here rather than watching
    # two releases overwrite each other on the site.
    owners = [name for name, t in targets.items() if t.get("update_mod_version")]
    if len(owners) > 1:
        sys.exit(f"nexus-target: {len(owners)} targets set update_mod_version ({', '.join(owners)}); at most one may.")

    # An unconfigured package is not an error: the tag still cut a GitHub
    # release, and this is how a package opts out of Nexus entirely.
    if not target["file_id"] or not target["game_scoped_id"]:
        print("publish=false")
        # stderr, not stdout: stdout is redirected into $GITHUB_OUTPUT, and a
        # workflow command has to reach the log to be seen.
        print(f"::notice::{prefix} has no Nexus target in {TARGETS.name}, so nothing was published to Nexus.",
              file=sys.stderr)
        return 0

    zip_name = target["zip"].format(version=version)
    display_name = target["display_name"].format(version=version)

    category = target["category"]
    if category not in CATEGORIES:
        sys.exit(f"nexus-target: category {category!r} is not one of {', '.join(CATEGORIES)}")
    if not VERSION_RE.fullmatch(version):
        sys.exit(f"nexus-target: version {version!r} is not accepted by Nexus ({VERSION_RE.pattern})")
    if not NAME_RE.fullmatch(display_name):
        sys.exit(f"nexus-target: display name {display_name!r} is not accepted by Nexus ({NAME_RE.pattern})")

    for key, value in [
        ("publish", "true"),
        ("version", version),
        ("zip", zip_name),
        ("display_name", display_name),
        ("category", category),
        ("update_mod_version", str(bool(target["update_mod_version"])).lower()),
        ("changelog", str(bool(target["changelog"])).lower()),
        ("game_domain", domain),
        ("game_scoped_id", target["game_scoped_id"]),
        ("file_id", target["file_id"]),
    ]:
        print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
