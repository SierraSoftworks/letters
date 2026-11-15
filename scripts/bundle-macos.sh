#!/usr/bin/env sh
# Builds `target/Letters.app`: the release build with its icon, and `.let` files named as its documents.
set -eu
cd "$(dirname "$0")/.."
cargo build --release -p letters
app=${CARGO_TARGET_DIR:-target}/Letters.app/Contents
rm -rf "${app%/Contents}"
mkdir -p "$app/MacOS" "$app/Resources"
cp "${CARGO_TARGET_DIR:-target}/release/letters" "$app/MacOS/letters"
cp apps/letters/macos/Info.plist "$app/Info.plist"
cp assets/icons/letters.icns assets/icons/document.icns "$app/Resources/"
echo "Built ${app%/Contents}"
