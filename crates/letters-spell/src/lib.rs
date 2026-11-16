//! Spell checking with Hunspell dictionaries.
//!
//! - [`Speller`] holds the dictionaries and the user's words. English is bundled, and a language without a
//!   dictionary is left unchecked.
//! - [`Checker`] checks words and text in one language and suggests corrections.
//! - [`Proofing`] keeps the misspellings of a whole document, a block at a time.
//! - [`best_dictionary`] says which of the dictionaries a system or a server holds serves a language.
//!
//! A word is accepted when the language's dictionary, the document's dictionary or the user's words know it.
//! Nothing here starts a thread or takes long: the shell decides when each step runs.

mod bundled;
mod dictionary;
mod named;
mod proofing;
mod speller;
mod text;

pub use bundled::{FILES as ENGLISH_FILES, provide as provide_english};
pub use dictionary::{Dictionary, DictionaryError};
pub use named::{best_dictionary, dictionary_language};
pub use proofing::{Checked, Proofing, language_at};
pub use speller::{Checker, Speller};
