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
- `crates/letters-test`: the headless interaction-test harness.
- `apps/letters`: the desktop app.

## Working on it

```sh
./scripts/check.sh   # format, lint, test and the wasm build
```
