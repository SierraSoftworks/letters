#!/usr/bin/env sh
# The gate every commit to main must pass: `./scripts/check.sh` runs every step in turn.
# CI runs the same steps as jobs side by side, each by its name: `./scripts/check.sh lint`.
set -eu

fmt() {
  cargo fmt --all --check
}

lint() {
  cargo clippy --workspace --all-targets -- -D warnings
}

tests() {
  cargo test --workspace
}

# The core crates must stay wasm-clean, and the browser shell must build on them.
wasm() {
  cargo build --target wasm32-unknown-unknown \
    -p letters-model -p letters-format -p letters-fonts -p letters-render -p letters-layout \
    -p letters-editor -p letters-ui -p letters-convert -p letters-spell -p letters-math -p letters-trust \
    -p letters-api -p letters-sync -p letters-web
}

[ $# -gt 0 ] || set -- fmt lint test wasm
for step in "$@"; do
  case $step in
    fmt) fmt ;;
    lint) lint ;;
    test) tests ;;
    wasm) wasm ;;
    *)
      echo "check.sh: no step named $step (fmt, lint, test, wasm)" >&2
      exit 2
      ;;
  esac
done
