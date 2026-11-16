<p align="center"><img src="assets/icons/png/app-256.png" width="128" height="128" alt="The Letters icon" /></p>

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

## Installing

Each [release](https://github.com/SierraSoftworks/letters/releases/latest) carries the desktop app for every
system. Letters is not signed with a paid developer certificate, so macOS and Windows ask before they open it
the first time.

- **macOS.** Open `Letters.dmg` and drag Letters to Applications. Then either:
  - open Letters, press Done when macOS says it could not verify it, and press Open Anyway under
    System Settings > Privacy & Security; or
  - run `xattr -dr com.apple.quarantine /Applications/Letters.app` in Terminal, which also helps when macOS
    calls the app damaged.
- **Windows.** Run `letters-x86_64-pc-windows-msvc.exe`. When SmartScreen stops it, press More info, then
  Run anyway.
- **Linux.** `chmod +x letters-x86_64-unknown-linux-gnu`, put it on your `PATH` as `letters`, and run
  `./scripts/install-desktop.sh` from this repository for the desktop entry and icons.

Letters looks for a newer release when it starts and offers to restart into it. The Mac app is pointed at the
release's page instead, and is allowed the same way after each download. Turn the look off with
`[updates] check = false` in `~/.letters/settings.toml`, or with `LETTERS_UPDATES=0` in the environment.

## Usage reports

Release builds of the desktop app tell `analytics.sierrasoftworks.com` which features are used and how long
sessions last, as the site `letters.app` with its pages under `/desktop`: page and event names out of a closed list in `apps/letters/src/telemetry.rs`, the version, the
system's kind, its language and time zone, and a random number that lasts one run. Nothing of a document or
of a person is sent, and no errors. Turn it off in Settings, with `[telemetry] enabled = false` in
`~/.letters/settings.toml`, or with `LETTERS_TELEMETRY=0` or `DO_NOT_TRACK=1` in the environment.

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
