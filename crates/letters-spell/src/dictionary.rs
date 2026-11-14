//! One Hunspell dictionary.

use std::collections::HashSet;
use std::fmt;

/// A Hunspell dictionary: the rules of an `.aff` file and the words of its `.dic` file.
pub struct Dictionary {
    words: spellbook::Dictionary,
    /// Every letter the word list uses, in both cases: a bit for each ASCII letter, and the rest.
    ascii: u128,
    letters: HashSet<char>,
}

impl Dictionary {
    /// Reads the text of an `.aff` file and of its `.dic` file. Both must be UTF-8 already.
    pub fn parse(aff: &str, dic: &str) -> Result<Self, DictionaryError> {
        let words = spellbook::Dictionary::new(aff, dic).map_err(|error| DictionaryError(error.to_string()))?;
        let (mut ascii, mut letters) = (0, HashSet::new());
        let stems = dic.lines().skip(1).map(|entry| entry.split(['/', '\t']).next().unwrap_or(entry));
        for letter in stems.flat_map(str::chars).filter(|c| c.is_alphabetic()) {
            if letter.is_ascii() {
                ascii |= 1 << letter.to_ascii_lowercase() as u8 | 1 << letter.to_ascii_uppercase() as u8;
            } else {
                letters.extend(letter.to_lowercase().chain(letter.to_uppercase()));
            }
        }
        Ok(Self { words, ascii, letters })
    }

    /// Whether `word` is spelled correctly. Either apostrophe is accepted.
    pub fn check(&self, word: &str) -> bool {
        self.words.check(word) || (word.contains('’') && self.words.check(&word.replace('’', "'")))
    }

    /// Corrections for a misspelled word, best first.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        self.words.suggest(word, &mut out);
        out
    }

    /// Whether `word` is written in this dictionary's letters, so could be one of its words at all.
    pub(crate) fn writes(&self, word: &str) -> bool {
        let known = |c: char| if c.is_ascii() { self.ascii >> c as u8 & 1 == 1 } else { self.letters.contains(&c) };
        word.chars().all(|c| !c.is_alphabetic() || known(c))
    }
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dictionary").finish_non_exhaustive()
    }
}

/// Why a dictionary's files could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictionaryError(String);

impl fmt::Display for DictionaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DictionaryError {}
