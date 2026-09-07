#!/usr/bin/env bash
#
# Fail if a built plugin imports a DLL that is not part of Windows.
#
#   nix develop --command tools/check-imports.sh <allowed> [<allowed> ...]
#
# `just check-imports` calls this with the `allowed_imports` list from the
# justfile, which is where the allowlist and the reason for every entry live.
#
# Why this exists: desert-overlay links imgui, which is C++. By default the
# `cc` crate links the target's C++ standard library, and for our mingw target
# that is libstdc++-6.dll - a DLL the game's bin64 does not have. The .asi then
# fails to load, and the ASI loader says nothing useful about why. The [env]
# block in .cargo/config.toml is what keeps that dependency out; this script is
# what proves it stayed out, on every `just ci`.
#
# Names are compared lowercased and without the `.dll` suffix, because the
# import table's capitalisation is not stable (KERNEL32.dll and kernel32.dll
# both appear in one binary).
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
built="$root/target/x86_64-pc-windows-gnu/release"
objdump=x86_64-w64-mingw32-objdump

[ "$#" -gt 0 ] || { echo "check-imports: no allowlist given" >&2; exit 2; }
allowed=" $* "

command -v "$objdump" >/dev/null ||
  { echo "check-imports: $objdump not on PATH - run inside 'nix develop'" >&2; exit 1; }

shopt -s nullglob
files=("$built"/*.dll "$built"/*.asi)
[ "${#files[@]}" -gt 0 ] ||
  { echo "check-imports: nothing built in $built - run 'just build' first" >&2; exit 1; }

rc=0
for f in "${files[@]}"; do
  bad=""
  imports="$("$objdump" -p "$f" | sed -n 's/.*DLL Name: //p' | tr 'A-Z' 'a-z' | sed 's/\.dll$//' | sort -u)"
  for dll in $imports; do
    case "$allowed" in
      *" $dll "*) ;;
      *) bad="$bad $dll" ;;
    esac
  done
  if [ -n "$bad" ]; then
    echo "check-imports: $(basename "$f") imports non-system DLL(s):$bad" >&2
    rc=1
  else
    echo "check-imports: $(basename "$f") ok ($(echo "$imports" | wc -l) imports)"
  fi
done
exit $rc
