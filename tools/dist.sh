#!/usr/bin/env bash
#
# Build the release packages for both plugins.
#
#   nix develop --command tools/dist.sh
#
# Writes dist/DesertLooter-<version>.zip, dist/DesertGatherer-<version>.zip and
# dist/SHA256SUMS. dist/ is gitignored; the zips are what gets attached to a
# GitHub release.
#
# The layout inside each zip is FLAT - the four files sit at the zip root, with
# no wrapper folder. That is what makes one archive serve both kinds of user:
#
#   * by hand: extract straight into <game>\bin64 and the .asi lands next to
#     winmm.dll with nothing to move afterwards.
#   * Definitive Mod Manager: DMM takes "an .asi / ReShade add-on file or a
#     directory", so it sees the .asi immediately whether the zip is dropped on
#     its window or the extracted folder is put under <DMM>/mods/_asi/.
#
# Reproducible: fixed file order, `zip -X` (no extra timestamp/uid/gid fields),
# a fixed mtime on the staged copies, and each zip deleted before it is
# rebuilt. Two runs of the same commit give byte-identical archives.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

dist="$root/dist"
built="$root/target/x86_64-pc-windows-gnu/release"

# Timestamp stamped on every file in the zips, so two runs of the same commit
# agree. Deliberately NOT SOURCE_DATE_EPOCH: nix's dev shell exports that as
# 315532800 (1980-01-01 UTC), which is the oldest stamp a zip can hold at all
# and underflows in any timezone west of UTC. Override DIST_EPOCH if you want
# a specific stamp.
: "${DIST_EPOCH:=1577836800}"  # 2020-01-01 UTC

for tool in cargo zip unzip sha256sum; do
  command -v "$tool" >/dev/null || { echo "dist.sh: $tool not on PATH - run inside 'nix develop'" >&2; exit 1; }
done

echo "==> cargo build --release"
cargo build --release

rm -rf "$dist"
mkdir -p "$dist"

# crate-dir  ShippedName  built-dll
package() {
  local crate="$1" name="$2" dll="$3"

  # The crate's Cargo.toml version is the single source of truth (VERSIONING.md).
  local version
  version="$(grep -m1 '^version' "$root/$crate/Cargo.toml" | cut -d'"' -f2)"
  [ -n "$version" ] || { echo "dist.sh: no version in $crate/Cargo.toml" >&2; exit 1; }

  local stage="$dist/.stage/$name"
  rm -rf "$stage"
  mkdir -p "$stage"

  # Fixed order, and the same four names in both zips.
  install -m 644 "$built/$dll"                 "$stage/$name.asi"
  install -m 644 "$root/$crate/$name.ini"      "$stage/$name.ini"
  install -m 644 "$root/$crate/README.md"      "$stage/README.md"
  install -m 644 "$root/$crate/CHANGELOG.md"   "$stage/CHANGELOG.md"
  touch -d "@$DIST_EPOCH" "$stage"/*

  local zipfile="$dist/$name-$version.zip"
  rm -f "$zipfile"
  ( cd "$stage" && zip -q -X -9 "$zipfile" "$name.asi" "$name.ini" README.md CHANGELOG.md )
  echo "==> $(basename "$zipfile")"
}

package desert-looter   DesertLooter   desert_looter.dll
package desert-gatherer DesertGatherer desert_gatherer.dll

rm -rf "$dist/.stage"

( cd "$dist" && sha256sum ./*.zip | sed 's| \./| |' > SHA256SUMS )

echo
for z in "$dist"/*.zip; do
  unzip -l "$z"
done
echo "dist/SHA256SUMS:"
sed 's/^/  /' "$dist/SHA256SUMS"
