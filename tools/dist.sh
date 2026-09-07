#!/usr/bin/env bash
#
# Build the release packages: both plugins and the DMM module pack.
#
#   nix develop --command tools/dist.sh                  all three
#   nix develop --command tools/dist.sh desert-looter    just that one
#
# Writes dist/DesertLooter-<version>.zip, dist/DesertGatherer-<version>.zip,
# dist/DesertGatherer-DMM-<version>.zip and dist/SHA256SUMS. dist/ is
# gitignored; the zips are what gets attached to a GitHub release.
#
# The release workflow passes the single package its tag names, so a release
# carries only the zip it is actually about. That is not tidiness: the three
# packages are versioned separately, so rebuilding all of them for every tag
# would eventually publish an untagged package's OLD version number over NEW
# bytes (any change to desert-core between two releases does it), and two
# releases would then disagree about the contents of one version.
#
# dist/ is wiped first, so SHA256SUMS only ever lists what this run built.
#
# The layout inside each zip is FLAT - the plugin files sit at the zip root,
# with no wrapper folder. That is what makes one archive serve both kinds of
# user:
#
#   * by hand: extract straight into <game>\bin64 and the .asi lands next to
#     winmm.dll with nothing to move afterwards.
#   * Definitive Mod Manager: DMM takes "an .asi / ReShade add-on file or a
#     directory", so it sees the .asi immediately whether the zip is dropped on
#     its window or the extracted folder is put under <DMM>/mods/_asi/.
#
# The pack zip is flat for a different reason: DMM only picks up a pack's title
# and description when dmm_pack.json sits at the zip ROOT, so a wrapper folder
# would leave the pack card showing the folder name instead.
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

# Package names are the release-tag prefixes (VERSIONING.md step 6), so the
# workflow can pass through what tools/release-notes.py --package printed.
all_packages=(desert-looter desert-gatherer desert-gatherer-dmm)
selected=("$@")
[ "${#selected[@]}" -gt 0 ] || selected=("${all_packages[@]}")

needs_cargo=0
for name in "${selected[@]}"; do
  case " ${all_packages[*]} " in
    *" $name "*) ;;
    *) echo "dist.sh: unknown package '$name'; expected one of ${all_packages[*]}" >&2; exit 1 ;;
  esac
  [ "$name" = desert-gatherer-dmm ] || needs_cargo=1
done

if [ "$needs_cargo" = 1 ]; then
  echo "==> cargo build --release"
  cargo build --release
fi

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

# The DMM module pack. No build step: the JSONs in desert-gatherer-dmm/ are the
# shipped artefact. Everything goes in FLAT so dmm_pack.json is at the zip root.
package_dmm() {
  local src="$root/desert-gatherer-dmm" name="DesertGatherer-DMM"

  # dmm_pack.json's version is the single source of truth for the pack, the way
  # Cargo.toml's is for a plugin. The module JSONs carry the same version.
  local version
  version="$(grep -m1 '"version"' "$src/dmm_pack.json" | sed 's/.*: *"\(.*\)".*/\1/')"
  [ -n "$version" ] || { echo "dist.sh: no version in desert-gatherer-dmm/dmm_pack.json" >&2; exit 1; }

  local stage="$dist/.stage/$name"
  rm -rf "$stage"
  mkdir -p "$stage"

  # Fixed order: the manifest, then the twelve modules by name (C collation, so
  # the order does not drift with the locale), then the docs and the rebaser.
  local modules=()
  mapfile -t modules < <( cd "$src" && printf '%s\n' *' - '*X.json | LC_ALL=C sort )
  [ "${#modules[@]}" -eq 12 ] || { echo "dist.sh: expected 12 pack modules, got ${#modules[@]}" >&2; exit 1; }

  local files=(dmm_pack.json "${modules[@]}" README.md VERIFICATION.txt rebase.py)
  local f
  for f in "${files[@]}"; do
    install -m 644 "$src/$f" "$stage/$f"
  done
  touch -d "@$DIST_EPOCH" "$stage"/*

  local zipfile="$dist/$name-$version.zip"
  rm -f "$zipfile"
  ( cd "$stage" && zip -q -X -9 "$zipfile" "${files[@]}" )
  echo "==> $(basename "$zipfile")"
}

for name in "${selected[@]}"; do
  case "$name" in
    desert-looter)       package desert-looter   DesertLooter   desert_looter.dll ;;
    desert-gatherer)     package desert-gatherer DesertGatherer desert_gatherer.dll ;;
    desert-gatherer-dmm) package_dmm ;;
  esac
done

rm -rf "$dist/.stage"

( cd "$dist" && sha256sum ./*.zip | sed 's| \./| |' > SHA256SUMS )

echo
for z in "$dist"/*.zip; do
  unzip -l "$z"
done
echo "dist/SHA256SUMS:"
sed 's/^/  /' "$dist/SHA256SUMS"
