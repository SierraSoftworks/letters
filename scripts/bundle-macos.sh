#!/usr/bin/env sh
# Builds `target/Letters.app`: the release build with its icon, and `.let` files named as its documents.
# `./scripts/bundle-macos.sh PROGRAM` bundles a program that is built already, as the release workflow's
# universal one is.
set -eu
cd "$(dirname "$0")/.."
target=${CARGO_TARGET_DIR:-target}
program=${1:-}
if [ -z "$program" ]; then
  cargo build --release -p letters
  program=$target/release/letters
fi
# The workspace's version, which is the first of the root manifest.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
app=$target/Letters.app/Contents
rm -rf "${app%/Contents}"
mkdir -p "$app/MacOS" "$app/Resources"
cp "$program" "$app/MacOS/letters"
chmod 755 "$app/MacOS/letters"
sed -E "/<key>CFBundle(ShortVersionString|Version)<\/key>/s#<string>[^<]*</string>#<string>$version</string>#" \
  apps/letters/macos/Info.plist >"$app/Info.plist"
cp assets/icons/letters.icns assets/icons/document.icns "$app/Resources/"
echo "Built ${app%/Contents} ($version)"
