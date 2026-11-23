#!/usr/bin/env bash
# Fails when the CHANGELOG.md section of a released version (a `v<version>`
# tag) differs from its text at that tag: a change after a release goes into a
# new entry. CI fetches the tags first.
set -euo pipefail

# The `## <version>` section of CHANGELOG.md on stdin.
section() { awk -v v="$1" '/^## /{p = ($2 == v)} p'; }

status=0
for tag in $(git tag --list 'v*'); do
  version=${tag#v}
  released=$(git show "$tag:CHANGELOG.md" 2>/dev/null | section "$version")
  [[ -n $released ]] || continue
  now=$(section "$version" <CHANGELOG.md)
  if [[ $released != "$now" ]]; then
    echo "CHANGELOG.md: the $version section changed since $tag; put the change in a new entry" >&2
    diff <(echo "$released") <(echo "$now") >&2 || true
    status=1
  fi
done
exit $status
