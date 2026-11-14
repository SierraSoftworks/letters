//! Spell checking with Hunspell dictionaries.
//!
//! - [`Speller`] holds the dictionaries and the user's words. English is bundled, and a language without a
//!   dictionary is left unchecked.
//! - [`Checker`] checks words and text in one language and suggests corrections.
//! - [`Proofing`] keeps the misspellings of a whole document, a block at a time.
//!
//! A word is accepted when the language's dictionary, the document's dictionary or the user's words know it.
//! Nothing here starts a thread or takes long: the shell decides when each step runs.

mod bundled;
mod dictionary;
mod proofing;
mod speller;
mod text;

pub use dictionary::{Dictionary, DictionaryError};
pub use proofing::{Checked, Proofing, language_at};
pub use speller::{Checker, Speller};
