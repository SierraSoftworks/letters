#!/usr/bin/env sh
# Builds the browser shell into `dist/`, ready to serve, and reports the size of its wasm.
# `--no-report` leaves the report out, which compresses every file to measure it and takes minutes.
set -eu
cd "$(dirname "$0")"
target=${CARGO_TARGET_DIR:-../../target}
wasm=$target/wasm32-unknown-unknown/web/letters_web.wasm

# The command-line wasm-bindgen must be the version the crate was built against.
version=$(sed -n '/^name = "wasm-bindgen"$/{n;s/version = "\(.*\)"/\1/p;}' ../../Cargo.lock)
bindgen=$target/tools/bin/wasm-bindgen
if [ "$("$bindgen" --version 2>/dev/null)" != "wasm-bindgen $version" ]; then
  # Its project publishes it built for these systems, which saves a minute of compiling it.
  case "$(uname -sm)" in
    "Linux x86_64") built=x86_64-unknown-linux-musl ;;
    "Linux aarch64") built=aarch64-unknown-linux-musl ;;
    "Darwin x86_64") built=x86_64-apple-darwin ;;
    "Darwin arm64") built=aarch64-apple-darwin ;;
    *) built= ;;
  esac
  mkdir -p "$target/tools/bin"
  [ -z "$built" ] ||
    curl -fsSL --retry 2 "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$version/wasm-bindgen-$version-$built.tar.gz" |
    tar -xz -C "$target/tools/bin" --strip-components 1 "wasm-bindgen-$version-$built/wasm-bindgen" || true
fi
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
cp www/start.js dist/
# The word lists of Thai, Lao, Khmer and Burmese, which a document in one of them fetches.
cargo run --quiet -p letters-web --example words -- dist/words
# The icons of the tab, of a home screen, and the list that names them.
cp ../../assets/icons/favicon.ico ../../assets/icons/web/* dist/
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

[ "${1:-}" != --no-report ] || exit 0

# The same module with its function names left in, which the report reads to say whose the code is.
"$bindgen" "$wasm" --target web --no-typescript --out-name letters --out-dir "$target/letters-web-named" --no-demangle
node size.mjs dist "$target/letters-web-named/letters_bg.wasm"
