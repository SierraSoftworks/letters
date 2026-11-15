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

# The tests `cargo test` leaves out (#[ignore]) because a debug build takes minutes over them, or because
# they need a session bus. CI runs them each night. They are named, since other ignored tests rewrite
# files of the repository. Not one of the steps a bare `./scripts/check.sh` runs.
slow() {
  dbus-run-session -- cargo test --release -p letters --test chrome --test letters_server -- --ignored
}

[ $# -gt 0 ] || set -- fmt lint test wasm
for step in "$@"; do
  case $step in
    fmt) fmt ;;
    lint) lint ;;
    test) tests ;;
    wasm) wasm ;;
    slow) slow ;;
    *)
      echo "check.sh: no step named $step (fmt, lint, test, wasm, slow)" >&2
      exit 2
      ;;
  esac
done
