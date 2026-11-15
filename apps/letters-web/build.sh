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
# The exports are a module of their own, which the editor fetches when the person first prints or saves a copy.
cargo build -p letters-web --example export --profile web --target wasm32-unknown-unknown
"$bindgen" "${wasm%/*}/examples/export.wasm" --target web --no-typescript --out-name export --out-dir dist \
  --remove-name-section --remove-producers-section
cargo run --quiet -p letters-web --example page -- www/index.html > dist/index.html
# What the editor fetches beside the wasm, each with its licence.
for files in fonts:letters-fonts/fonts maths:letters-math/fonts dictionaries:letters-spell/dictionaries; do
  mkdir "dist/${files%%:*}"
  cp ../../crates/"${files#*:}"/* "dist/${files%%:*}/"
done

# The faces that back up the fonts are too large to keep in the repository. Each is fetched once, as it was
# published, and checked. A build that cannot reach one goes on without it, and the page without its glyphs.
faces=$target/letters-web-faces
sum() { { sha256sum "$1" 2>/dev/null || shasum -a 256 "$1"; } | cut -d' ' -f1; }
mkdir -p "$faces" dist/fallback
grep -v '^#' fallback.txt | while read -r wanted name url; do
  if [ ! -f "$faces/$name" ] || [ "$(sum "$faces/$name")" != "$wanted" ]; then
    rm -f "$faces/$name"
    if curl -fsSL --retry 2 -o "$faces/$name.part" "$url" && [ "$(sum "$faces/$name.part")" = "$wanted" ]; then
      mv "$faces/$name.part" "$faces/$name"
    else
      rm -f "$faces/$name.part"
      echo "warning: $name could not be fetched as it was published, and is left out" >&2
      continue
    fi
  fi
  cp "$faces/$name" dist/fallback/
done

# The same module with its function names left in, which the report reads to say whose the code is.
"$bindgen" "$wasm" --target web --no-typescript --out-name letters --out-dir "$target/letters-web-named" --no-demangle
node size.mjs dist "$target/letters-web-named/letters_bg.wasm"
