#!/usr/bin/env bash
# Lays out a Linux build as the distro packages install it, under <destdir>:
# the binary, its desktop entry (which declares the deep link) and its icons.
# Used by the release workflow for the distro tarball and by the AUR
# `exilence-ce` package, which builds from source.
#
#   packaging/linux/stage.sh <binary> <destdir>
set -euo pipefail

binary=$1
dest=$2
root=$(cd "$(dirname "$0")/../.." && pwd)

install -Dm755 "$binary" "$dest/usr/bin/exilence-ce-ng"
install -Dm644 "$root/packaging/linux/exilence-ce-ng.desktop" \
  "$dest/usr/share/applications/exilence-ce-ng.desktop"
for size in 32x32:32x32 128x128:128x128 256x256:128x128@2x; do
  install -Dm644 "$root/src-tauri/icons/${size#*:}.png" \
    "$dest/usr/share/icons/hicolor/${size%%:*}/apps/exilence-ce-ng.png"
done
