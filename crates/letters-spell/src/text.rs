//! Finds the words of running text.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

/// Calls `each` with every word of `text`, as the byte ranges of its hyphen-joined parts: one range for
/// "known", two for "well-known". Web and mail addresses are passed over, and so is a word that directly
/// follows a backslash: a TeX command such as `\sqrt` in a formula still being typed, or a step of a path.
pub(crate) fn compounds(text: &str, mut each: impl FnMut(&[Range<usize>])) {
    let mut parts: Vec<Range<usize>> = Vec::new();
    let mut rest = 0;
    while let Some(start) = text[rest..].find(|c: char| !c.is_whitespace()).map(|at| rest + at) {
        rest = text[start..].find(char::is_whitespace).map_or(text.len(), |at| start + at);
        let chunk = &text[start..rest];
        if is_address(chunk) {
            continue;
        }
        for (at, word) in chunk.unicode_word_indices() {
            let range = start + at..start + at + word.len();
            if text[..range.start].ends_with('\\') {
                continue;
            }
            if parts.last().is_some_and(|last| !is_hyphen(&text[last.end..range.start])) {
                each(&parts);
                parts.clear();
            }
            parts.push(range);
        }
        if !parts.is_empty() {
            each(&parts);
            parts.clear();
        }
    }
}

/// Whether `word` is ordinary prose: letters and apostrophes, and more than a single letter. Anything else,
/// such as "B12", "report.pdf" or "snake_case", is not for a dictionary to judge.
pub(crate) fn is_prose(word: &str) -> bool {
    word.chars().all(|c| c.is_alphabetic() || is_apostrophe(c))
        && word.chars().filter(|c| c.is_alphabetic()).nth(1).is_some()
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’')
}

fn is_hyphen(between: &str) -> bool {
    matches!(between, "-" | "\u{2010}" | "\u{2011}")
}

fn is_address(chunk: &str) -> bool {
    chunk.contains("://")
        || chunk.contains('@')
        || chunk.get(..4).is_some_and(|start| start.eq_ignore_ascii_case("www."))
}
