#!/usr/bin/env sh
# Builds the browser shell into `dist/`, ready to serve, and reports the size of its wasm.
set -eu
cd "$(dirname "$0")"
target=${CARGO_TARGET_DIR:-../../target}
wasm=$target/wasm32-unknown-unknown/web/letters_web.wasm

# The command-line wasm-bindgen must be the version the crate was built against.
version=$(sed -n '/^name = "wasm-bindgen"$/{n;s/version = "\(.*\)"/\1/p;}' ../../Cargo.lock)
bindgen=$target/tools/bin/wasm-bindgen
if [ "$("$bindgen" --version 2>/dev/null)" != "wasm-bindgen $version" ]; then
  cargo install wasm-bindgen-cli --version "$version" --locked --root "$target/tools"
fi

# The `web` profile is the release build made small where no frame waits on it.
cargo build -p letters-web --lib --profile web --target wasm32-unknown-unknown
rm -rf dist
"$bindgen" "$wasm" --target web --no-typescript --out-name letters --out-dir dist \
  --remove-name-section --remove-producers-section
cargo run --quiet -p letters-web --example page -- www/index.html > dist/index.html
mkdir dist/fonts
cp ../../crates/letters-fonts/fonts/*.ttf dist/fonts/

# The same module with its function names left in, which the report reads to say whose the code is.
"$bindgen" "$wasm" --target web --no-typescript --out-name letters --out-dir "$target/letters-web-named" --no-demangle
node size.mjs dist "$target/letters-web-named/letters_bg.wasm"
