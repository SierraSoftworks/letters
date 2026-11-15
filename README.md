# Letters

A local-first document editor: one Rust editing engine, one self-contained `.let` file per document, and a
server that adds identity, sharing and sync without owning any content.

## Layout

- `crates/letters-model`: the document, its operations and selections.
- `crates/letters-format`: the `.let` container.
- `crates/letters-render`: the display list and its backends.
- `crates/letters-fonts`: the bundled open-licence fonts.
- `crates/letters-layout`: paragraph and page layout.
- `crates/letters-editor`: commands, Markdown input rules, keymap, clipboard and composition.
- `crates/letters-ui`: the native widgets and the editor window's panels.
- `crates/letters-convert`: import and export.
- `crates/letters-spell`: spell checking with Hunspell dictionaries, English bundled.
- `crates/letters-trust`: safe mode, trusted sources and the guards on remote image fetches.
- `crates/letters-api`: what a server and the shells say to each other over HTTP.
- `crates/letters-test`: the headless interaction-test harness.
- `apps/letters`: the desktop app.
- `apps/letters-web`: the browser shell, which paints the same engine on a canvas.
- `apps/letters-server`: the server, which shares folders of documents with the people an identity provider names.
  [Letters on a tailnet](apps/letters-server/docs/tailscale.md) sets one up with Tailscale as that provider.
- `fuzz`: the fuzz targets, which a nightly workflow runs and files issues from.

## Working on it

```sh
./scripts/check.sh   # format, lint, test and the wasm build
./scripts/budgets.sh # the launch and keystroke budgets, on a release build
./scripts/fuzz       # names the fuzz targets, and runs one under AFL++ when given its name
```

The browser shell is built and tested from `apps/letters-web`:

```sh
./build.sh           # the page into dist/, and a report of the wasm's size
npm start            # serve it at http://localhost:8080/
npm install && npx playwright install firefox chromium && npm test
```
