//! Finding entries by a few letters of where they are, as a person types them.
//!
//! A [`Query`] is words and filters, split at spaces. A word finds an entry when its letters stand in the
//! entry's path in that order, with anything between them, and how well they stand there ranks what is
//! found. A filter is `key:value`, and an entry either passes it or is left out:
//!
//! - `author:sam` (or `by:`): someone whose name holds "sam" edited it.
//! - `is:folder`, `is:document`, `is:shared`, `is:conflict`, `is:editing`.
//! - `before:2026-11-01` and `after:2026-11-01`: when it was last written, in UTC.
//!
//! A value with a space in it is quoted: `author:"Sam Okafor"`. What reads like a filter and is not one is
//! a word, so a name with a colon in it is still found. Server and shells rank by this one module, so a
//! search answers alike wherever it is asked.

use crate::{Entry, EntryKind, Tag};

/// What a letter that is found is worth, and what more it is worth where a reader's eye would look for
/// it: right after the letter found before it, at the start of a word, and in the entry's own name
/// rather than in the folders that lead to it.
const FOUND: i32 = 16;
const RUN: i32 = 12;
const START: i32 = 10;
const NAME: i32 = 4;
/// What each letter skipped between two that are found costs, up to `GAP_MOST` for one gap.
const GAP: i32 = 1;
const GAP_MOST: i32 = 6;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Filter {
    Author(String),
    Kind(EntryKind),
    Shared,
    Conflict,
    Editing,
    /// Last written before this time, in seconds since 1970.
    Before(u64),
    /// Last written at this time or after it.
    After(u64),
}

impl Filter {
    fn read(key: &str, value: &str) -> Option<Self> {
        let value = value.to_lowercase();
        if value.is_empty() {
            return None;
        }
        match key.to_lowercase().as_str() {
            "author" | "by" => Some(Self::Author(value)),
            "is" => match value.as_str() {
                "folder" => Some(Self::Kind(EntryKind::Folder)),
                "document" | "doc" => Some(Self::Kind(EntryKind::Document)),
                "shared" => Some(Self::Shared),
                "conflict" => Some(Self::Conflict),
                "editing" => Some(Self::Editing),
                _ => None,
            },
            "before" => day(&value).map(Self::Before),
            "after" => day(&value).map(Self::After),
            _ => None,
        }
    }

    fn passes(&self, entry: &Entry) -> bool {
        let tagged = |wanted: fn(&Tag) -> bool| entry.tags.iter().any(wanted);
        match self {
            Self::Author(name) => entry.contributors.iter().any(|by| by.to_lowercase().contains(name)),
            Self::Kind(kind) => entry.kind == *kind,
            Self::Shared => tagged(|tag| matches!(tag, Tag::Shared { with: 1.. } | Tag::Links { count: 1.. })),
            Self::Conflict => tagged(|tag| matches!(tag, Tag::ConflictCopy { .. })),
            Self::Editing => tagged(|tag| matches!(tag, Tag::Editing { people } if !people.is_empty())),
            Self::Before(time) => entry.modified < *time,
            Self::After(time) => entry.modified >= *time,
        }
    }
}

/// The start of the day `YYYY-MM-DD` names, in seconds since 1970.
fn day(text: &str) -> Option<u64> {
    let mut parts = text.splitn(3, '-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (parts.next()??, parts.next()??, parts.next()??);
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from the civil date, counting years from March so that a leap day ends one.
    let year = if month <= 2 { year - 1 } else { year };
    let (era, of_era) = (year.div_euclid(400), year.rem_euclid(400));
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    u64::try_from(days * 86_400).ok()
}

/// What a person typed to find entries by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Each word's letters, in lower case.
    words: Vec<Vec<char>>,
    filters: Vec<Filter>,
}

impl Query {
    pub fn parse(text: &str) -> Self {
        let mut query = Self::default();
        for token in tokens(text) {
            let plain = || token.replace('"', "");
            let filter = token.split_once(':').and_then(|(key, value)| Filter::read(key, &value.replace('"', "")));
            match filter {
                Some(filter) => query.filters.push(filter),
                None => query.words.push(plain().chars().map(lower).collect()),
            }
        }
        query.words.retain(|word| !word.is_empty());
        query
    }

    /// Whether it asks for nothing, and so finds everything.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.filters.is_empty()
    }

    /// How well the entry answers: more is better, and `None` is an entry that is not found.
    pub fn score(&self, entry: &Entry) -> Option<i32> {
        if !self.filters.iter().all(|filter| filter.passes(entry)) {
            return None;
        }
        let path: Vec<char> = shown(entry).chars().collect();
        let name = path.iter().rposition(|&letter| letter == '/').map_or(0, |slash| slash + 1);
        self.words.iter().try_fold(0, |sum, word| Some(sum + stands(word, &path, name)?))
    }

    /// The entries that answer, the best first. Of those that answer as well, the one with the least else
    /// in its path is the nearer, and the rest keep the order they came in.
    pub fn rank<'a>(&self, entries: &'a [Entry]) -> Vec<&'a Entry> {
        let mut found: Vec<(i32, &Entry)> =
            entries.iter().filter_map(|entry| Some((self.score(entry)?, entry))).collect();
        found.sort_by_key(|(score, entry)| (std::cmp::Reverse(*score), entry.path.len().max(entry.name.len())));
        found.into_iter().map(|(_, entry)| entry).collect()
    }
}

/// The path an entry is found by: its folders, and its name as it is shown. One that says nothing of
/// where it is has its name alone.
fn shown(entry: &Entry) -> String {
    match entry.path.rsplit_once('/') {
        Some((folders, _)) => format!("{folders}/{}", entry.name),
        None => entry.name.clone(),
    }
}

/// What was typed, split at the spaces that no quotes hold.
fn tokens(text: &str) -> Vec<String> {
    let (mut tokens, mut quoted) = (vec![String::new()], false);
    for letter in text.chars() {
        quoted ^= letter == '"';
        match letter.is_whitespace() && !quoted {
            true => tokens.push(String::new()),
            false => tokens.last_mut().expect("there is always a token").push(letter),
        }
    }
    tokens.retain(|token| !token.is_empty());
    tokens
}

/// One letter for one letter, so that a place in the lowered path is the same place in the path.
fn lower(letter: char) -> char {
    letter.to_lowercase().next().unwrap_or(letter)
}

/// How well the letters of `word` stand in `path`, in order: the best of every way they do. `name` is
/// where the entry's own name starts in the path.
fn stands(word: &[char], path: &[char], name: usize) -> Option<i32> {
    // A letter starts a word after what parts words, and where capitals set words apart.
    let starts = |at: usize| match at.checked_sub(1).map(|before| path[before]) {
        None => true,
        Some(before) => !before.is_alphanumeric() || (before.is_lowercase() && path[at].is_uppercase()),
    };
    let worth = |at: usize| FOUND + if starts(at) { START } else { 0 } + if at >= name { NAME } else { 0 };
    // The best score of the word so far with its last letter at each place of the path.
    let mut ends: Vec<Option<i32>> = Vec::new();
    for (nth, &wanted) in word.iter().enumerate() {
        let before = std::mem::replace(&mut ends, vec![None; path.len()]);
        // The best of the letters before a gap: each letter skipped costs, up to what a gap costs at most,
        // so that a long path is not a worse one.
        let (mut near, mut best): (Option<i32>, Option<i32>) = (None, None);
        for at in nth..path.len() {
            let run = at.checked_sub(1).and_then(|left| before.get(left).copied().flatten());
            (near, best) = (near.map(|near| near - GAP).max(run), best.max(run));
            if lower(path[at]) == wanted {
                let led = if nth == 0 {
                    Some(0)
                } else {
                    run.map(|run| run + RUN).max(near).max(best.map(|best| best - GAP_MOST))
                };
                ends[at] = led.map(|led| led + worth(at));
            }
        }
    }
    ends.into_iter().flatten().max()
}
