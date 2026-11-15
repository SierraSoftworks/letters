# Working on Letters

The approved design is the doc "Letters: Initial Design" (https://claude.ai/artifact/TBxDzCmACwNw9FrLfTCSc7).
Build what it specifies, and prefer simple and polished over complex.

## Rules for every change

- **Main stays green.** Commit straight to `main` in small commits. Before each push, rebase on `origin/main`
  and run `./scripts/check.sh`.
- **Stay in your crate.** Each crate has one owner at a time. To change another crate's public interface, make
  the smallest additive change that unblocks you and say so in the commit message.
- **The core is wasm-clean.** Every crate under `crates/` builds for `wasm32-unknown-unknown`. No threads,
  files, clocks, randomness or GPU in the core: the shell injects them (see `letters_editor::Platform`).

## Shared interfaces

- **Document.** `letters_model::Document` is the tree `document.json` stores. All edits go through
  `Model::apply` as `Op`s, and `Change` tells layout what to redo. Offsets are UTF-8 bytes.
- **Drawing.** Everything visible is emitted as a `letters_render::DisplayList`. Nothing draws directly.
- **Fonts.** `letters_fonts::bundled()` is the one font set for layout, tests, export and every shell.
- **Input.** Text arrives as `letters_editor::InputEvent`, keys as `KeyEvent`, and actions as `Command`.

## Style

- **Code.** Comments and docs are short and say why. No em dashes. `todo!()` marks an interface that is
  agreed but not built.
- **Tests.** Cover core use cases and failure modes through public interfaces. Use data-driven cases and
  shared fixtures, keep assertions in the test body, and assert on what the user needs, not on exact output.
- **Dependencies.** Shared versions live in the root `Cargo.toml`. Pure Rust only in the core.
