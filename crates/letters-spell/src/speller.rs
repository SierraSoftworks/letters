//! Chooses a dictionary for a language and layers the word lists on top of it.

use std::collections::BTreeSet;
use std::ops::Range;

use crate::bundled::English;
use crate::{Dictionary, text};

/// The dictionaries of every language that can be checked, and the user's own words.
///
/// English is built in and parsed the first time it is used, which takes a few milliseconds. Other languages
/// are checked once the shell adds a dictionary for them.
#[derive(Debug, Default)]
pub struct Speller {
    added: Vec<(Tag, Dictionary)>,
    user: BTreeSet<String>,
    /// Counts the changes that can alter a verdict, so that [`crate::Proofing`] knows to check again.
    pub(crate) revision: u64,
}

impl Speller {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the dictionary for a BCP 47 tag such as `de` or `pt-BR`. Hunspell's file names, such as `pt_BR`,
    /// are read too. It replaces a dictionary added for the same tag, and is preferred to a bundled one.
    pub fn add_dictionary(&mut self, language: &str, dictionary: Dictionary) {
        // A tag that names no language is never asked for, so its dictionary is dropped.
        let Some(tag) = Tag::parse(language) else { return };
        self.added.retain(|(added, _)| *added != tag);
        self.added.push((tag, dictionary));
        self.revision += 1;
    }

    /// Replaces the user's own words, which are accepted in every language and every document. Blank
    /// entries are dropped, so the lines of the user's `dictionary.txt` can be passed as they are.
    pub fn set_user_words(&mut self, words: impl IntoIterator<Item = impl AsRef<str>>) {
        self.user = words.into_iter().map(|w| w.as_ref().trim().to_owned()).filter(|w| !w.is_empty()).collect();
        self.revision += 1;
    }

    pub fn user_words(&self) -> &BTreeSet<String> {
        &self.user
    }

    /// Whether text in `language` is checked. Text in any other language is left alone, never flagged.
    pub fn supports(&self, language: &str) -> bool {
        self.dictionary(language).is_some()
    }

    /// A checker for text in `language` that also accepts `words`, the document's own dictionary, and the
    /// user's words. `None` when no dictionary serves the language.
    pub fn checker<'a>(&'a self, language: &str, words: &'a BTreeSet<String>) -> Option<Checker<'a>> {
        Some(Checker { dictionary: self.dictionary(language)?, lists: [words, &self.user] })
    }

    /// The dictionary for a tag: the one added for its language and region, else the bundled English for
    /// its region, else one added for the language alone, else the first added for the language.
    fn dictionary(&self, language: &str) -> Option<&Dictionary> {
        let tag = Tag::parse(language)?;
        let same = || self.added.iter().rev().filter(|(added, _)| added.language == tag.language);
        if let Some((_, exact)) = same().find(|(added, _)| added.region == tag.region) {
            return Some(exact);
        }
        if tag.language == "en" {
            return English::of(tag.region.as_deref()).dictionary();
        }
        same()
            .find(|(added, _)| added.region.is_none())
            .or_else(|| same().next_back())
            .map(|(_, dictionary)| dictionary)
    }
}

/// The parts of a BCP 47 tag that choose a dictionary.
#[derive(Debug, PartialEq, Eq)]
struct Tag {
    /// In lower case.
    language: String,
    /// In upper case.
    region: Option<String>,
}

impl Tag {
    fn parse(tag: &str) -> Option<Self> {
        let alphabetic = |part: &str| part.bytes().all(|b| b.is_ascii_alphabetic());
        let mut parts = tag.trim().split(['-', '_']);
        let language = parts.next().filter(|part| matches!(part.len(), 2 | 3) && alphabetic(part))?;
        // The region follows any script, and comes before the single letter that opens an extension.
        let is_region = |part: &&str| match part.len() {
            2 => alphabetic(part),
            3 => part.bytes().all(|b| b.is_ascii_digit()),
            _ => false,
        };
        let region = parts.take_while(|part| part.len() > 1).find(is_region);
        Some(Self { language: language.to_ascii_lowercase(), region: region.map(str::to_ascii_uppercase) })
    }
}

/// Checks text in one language against its dictionary, the document's words and the user's words.
#[derive(Clone, Copy, Debug)]
pub struct Checker<'a> {
    dictionary: &'a Dictionary,
    lists: [&'a BTreeSet<String>; 2],
}

impl Checker<'_> {
    /// Whether `word` is spelled correctly.
    pub fn check(&self, word: &str) -> bool {
        self.dictionary.check(word) || self.listed(word)
    }

    /// Corrections for a misspelled word, best first. Close words from the word lists lead.
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let typed: Vec<char> = word.to_lowercase().chars().collect();
        let limit = if typed.len() > 4 { 2 } else { 1 };
        let mut near: Vec<(usize, &str)> = (self.lists.iter().copied().flatten())
            .map(|entry| (distance(&typed, &entry.to_lowercase().chars().collect::<Vec<_>>()), entry.as_str()))
            .filter(|&(distance, entry)| distance <= limit && entry != word)
            .collect();
        near.sort();
        let mut out: Vec<String> = near.into_iter().map(|(_, entry)| entry.to_owned()).collect();
        for suggestion in self.dictionary.suggest(word) {
            if !out.contains(&suggestion) {
                out.push(suggestion);
            }
        }
        out
    }

    /// The byte ranges of the misspelled words in `text`, in order.
    ///
    /// Only prose is judged: words with digits or inner punctuation, single letters, web and mail addresses,
    /// and words in letters the language does not use are passed over. A hyphenated word is correct when
    /// its parts are, or when it is listed whole.
    pub fn misspellings(&self, text: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        text::compounds(text, |parts| {
            let wrong = |part: &&Range<usize>| {
                let word = &text[part.start..part.end];
                text::is_prose(word) && self.dictionary.writes(word) && !self.check(word)
            };
            let wrong: Vec<Range<usize>> = parts.iter().filter(wrong).cloned().collect();
            let whole = || text[parts[0].start..parts[parts.len() - 1].end].replace(['\u{2010}', '\u{2011}'], "-");
            if !wrong.is_empty() && (parts.len() == 1 || !self.check(&whole())) {
                out.extend(wrong);
            }
        });
        out
    }

    /// Whether a word list accepts `word`. As with a Hunspell personal dictionary, a listed word also covers
    /// its possessive, and its capitalised and upper-case forms.
    fn listed(&self, word: &str) -> bool {
        let has = |spelling: &str| {
            let curly = spelling.replace('\'', "’");
            self.lists.iter().any(|list| list.contains(spelling) || list.contains(&curly))
        };
        let word = word.replace('’', "'");
        let bare = word.strip_suffix("'s").unwrap_or(&word);
        if has(&word) || has(bare) {
            return true;
        }
        let lower = bare.to_lowercase();
        let mut letters = lower.chars();
        let capitalised: String = letters.next().into_iter().flat_map(char::to_uppercase).chain(letters).collect();
        let upper_case = !bare.chars().any(char::is_lowercase);
        (bare.starts_with(char::is_uppercase) && has(&lower)) || (upper_case && has(&capitalised))
    }
}

/// How many single-letter edits (insert, delete, replace, or swap neighbours) turn `a` into `b`.
fn distance(a: &[char], b: &[char]) -> usize {
    let (mut before, mut last) = (vec![0; b.len() + 1], (0..=b.len()).collect::<Vec<_>>());
    for i in 1..=a.len() {
        let mut row = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            row[j] = (last[j - 1] + usize::from(a[i - 1] != b[j - 1])).min(last[j] + 1).min(row[j - 1] + 1);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                row[j] = row[j].min(before[j - 2] + 1);
            }
        }
        before = std::mem::replace(&mut last, row);
    }
    last[b.len()]
}
