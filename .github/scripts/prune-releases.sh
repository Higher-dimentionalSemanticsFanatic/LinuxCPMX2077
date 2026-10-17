#!/bin/sh
# Delete every GitHub release except one, so the Releases page offers only
# the current version. Git tags stay. Keeps $1, or the highest version.
set -eu
tags=$(gh release list --limit 500 --json tagName -q '.[].tagName')
keep="${1:-$(printf '%s\n' "$tags" | sort -V | tail -n1)}"
printf '%s\n' "$tags" | grep -qxF "$keep" || { echo "No release $keep; nothing deleted" >&2; exit 1; }
for t in $tags; do
  [ "$t" = "$keep" ] && continue
  echo "Deleting release $t (tag kept)"
  gh release delete "$t" --yes
done
