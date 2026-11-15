#!/usr/bin/env sh
# Tells a Linux desktop about Letters: its entry in the launcher, its icon, and that `.let` files are its.
# `letters` itself must be on the PATH, as `cargo install --path apps/letters` leaves it.
#
#   scripts/install-desktop.sh [prefix]    (the prefix is ~/.local unless given)
set -eu
cd "$(dirname "$0")/.."
share=${1:-$HOME/.local}/share
id=com.sierrasoftworks.Letters
install -Dm644 apps/letters/linux/$id.desktop "$share/applications/$id.desktop"
install -Dm644 apps/letters/linux/$id.xml "$share/mime/packages/$id.xml"
for size in 16 32 48 64 128 256 512; do
  icons=$share/icons/hicolor/${size}x$size
  install -Dm644 assets/icons/png/app-$size.png "$icons/apps/$id.png"
  install -Dm644 assets/icons/png/file-$size.png "$icons/mimetypes/application-vnd.letters.document+zip.png"
done
# Each of these is the desktop's own, and one that is missing has nothing to be told.
command -v update-mime-database >/dev/null && update-mime-database "$share/mime"
command -v update-desktop-database >/dev/null && update-desktop-database "$share/applications"
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$share/icons/hicolor" || true
