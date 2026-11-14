#!/usr/bin/env sh
# The gate every commit to main must pass.
set -eu
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# The core crates must stay wasm-clean, and the browser shell must build on them.
cargo build --target wasm32-unknown-unknown \
  -p letters-model -p letters-format -p letters-fonts -p letters-render -p letters-layout \
  -p letters-editor -p letters-ui -p letters-convert -p letters-spell -p letters-math -p letters-trust \
  -p letters-web
