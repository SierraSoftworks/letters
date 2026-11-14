//! What the engine understands, for completion while a formula is typed.

use crate::table;

/// Something a formula can use: a command such as `\frac`, or an environment such as `matrix`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// What is typed: `\frac` for a command, and `matrix` (as in `\begin{matrix}`) for an environment.
    pub command: &'static str,
    /// What it is in plain words, such as "fraction" or "not equal to".
    pub name: &'static str,
    /// Other words people look for it under, such as "divide" or "!=".
    pub aliases: &'static [&'static str],
    /// How many braced arguments it takes: `\frac` is inserted as `\frac{}{}`. For an environment, those
    /// that follow its `\begin{…}`.
    pub arguments: u8,
    /// A source that shows it at work, to typeset beside it in a list: `\frac{a}{b}` for `\frac`.
    pub sample: &'static str,
}

/// Every command worth offering, in the order to list them before anything narrows the list. The engine
/// typesets each one, so a list built from these offers only what will render.
pub fn commands() -> &'static [Command] {
    table::COMMANDS
}

/// Every environment, for completing `\begin{`.
pub fn environments() -> &'static [Command] {
    table::ENVIRONMENTS
}
