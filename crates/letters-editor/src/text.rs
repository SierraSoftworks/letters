//! Reading a block's text: character and word boundaries, and the formatting at an offset.

use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{Block, BlockKind, Mark, Run};
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn len(block: &Block) -> usize {
    block.body.runs().iter().map(|run| run.text.len()).sum()
}

pub(crate) fn is_code(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Code { .. })
}

/// The nearest character boundary at or before `offset`.
pub(crate) fn snap(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

/// The nearest character boundary at or after `offset`.
pub(crate) fn snap_up(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset += 1;
    }
    offset
}

/// The boundary one user-perceived character before `offset`.
pub(crate) fn prev(text: &str, offset: usize) -> usize {
    text[..offset].graphemes(true).next_back().map_or(0, |g| offset - g.len())
}

/// The boundary one user-perceived character after `offset`.
pub(crate) fn next(text: &str, offset: usize) -> usize {
    text[offset..].graphemes(true).next().map_or(text.len(), |g| offset + g.len())
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

/// The start of the word before `offset`.
pub(crate) fn word_left(text: &str, offset: usize) -> usize {
    let words = text.split_word_bound_indices().take_while(|(start, _)| *start < offset);
    words.filter(|(_, segment)| is_word(segment)).last().map_or(0, |(start, _)| start)
}

/// The end of the word after `offset`.
pub(crate) fn word_right(text: &str, offset: usize) -> usize {
    let mut ends = text.split_word_bound_indices().map(|(start, segment)| (start + segment.len(), segment));
    ends.find(|(end, segment)| *end > offset && is_word(segment)).map_or(text.len(), |(end, _)| end)
}

/// The word, or the stretch between words, that holds `offset`.
pub(crate) fn word_around(text: &str, offset: usize) -> Range<usize> {
    let mut segments = text.split_word_bound_indices().map(|(start, segment)| start..start + segment.len());
    let found = segments.find(|range| offset < range.end || range.end == text.len());
    found.unwrap_or(offset..offset)
}

/// The start of the hard line holding `offset`.
pub(crate) fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |at| at + 1)
}

/// The end of the hard line holding `offset`, before its line break.
pub(crate) fn line_end(text: &str, offset: usize) -> usize {
    text[offset..].find('\n').map_or(text.len(), |at| offset + at)
}

/// How many characters into its hard line `offset` is.
pub(crate) fn column(text: &str, offset: usize) -> usize {
    text[line_start(text, offset)..offset].graphemes(true).count()
}

/// The offset `column` characters into the hard line that starts at `line`, or the line's end if it is shorter.
pub(crate) fn at_column(text: &str, line: usize, column: usize) -> usize {
    let end = line_end(text, line);
    text[line..end].grapheme_indices(true).nth(column).map_or(end, |(at, _)| line + at)
}

/// The run holding the character that starts at `offset`.
pub(crate) fn run_at(runs: &[Run], offset: usize) -> Option<&Run> {
    let mut end = 0;
    runs.iter().find(|run| {
        end += run.text.len();
        offset < end
    })
}

/// The marks of the character before `offset`. None at the start of the text.
pub(crate) fn marks_before(runs: &[Run], offset: usize) -> BTreeSet<Mark> {
    let run = offset.checked_sub(1).and_then(|before| run_at(runs, before));
    run.map(|run| run.marks.clone()).unwrap_or_default()
}

/// The marks of the character after `offset`. None at the end of the text.
pub(crate) fn marks_after(runs: &[Run], offset: usize) -> BTreeSet<Mark> {
    run_at(runs, offset).map(|run| run.marks.clone()).unwrap_or_default()
}

/// Where the stretch of runs that pass `has` and end at `offset` begins. `offset` itself when there is none.
pub(crate) fn stretch_start(runs: &[Run], offset: usize, has: impl Fn(&Run) -> bool) -> usize {
    let (mut start, mut stretch) = (0, None);
    for run in runs.iter().filter(|run| !run.text.is_empty()) {
        if start >= offset {
            break;
        }
        stretch = if has(run) { stretch.or(Some(start)) } else { None };
        start += run.text.len();
    }
    stretch.unwrap_or(offset)
}

/// Where the stretch of runs that pass `has` and start at `offset` ends. `offset` itself when there is none.
pub(crate) fn stretch_end(runs: &[Run], offset: usize, has: impl Fn(&Run) -> bool) -> usize {
    let (mut end, mut stretch) = (0, offset);
    for run in runs.iter().filter(|run| !run.text.is_empty()) {
        end += run.text.len();
        if end > offset {
            if !has(run) {
                break;
            }
            stretch = end;
        }
    }
    stretch
}

/// The runs with text inside `range`.
pub(crate) fn runs_in(runs: &[Run], range: Range<usize>) -> impl Iterator<Item = &Run> {
    let mut start = 0;
    runs.iter().filter(move |run| {
        let span = start..start + run.text.len();
        start = span.end;
        !span.is_empty() && span.start < range.end && range.start < span.end
    })
}

/// A copy of the part of `runs` inside `range`.
pub(crate) fn slice(runs: &[Run], range: Range<usize>) -> Vec<Run> {
    let mut start = 0;
    let mut out = Vec::new();
    for run in runs {
        let span = start..start + run.text.len();
        start = span.end;
        let (from, to) = (range.start.max(span.start), range.end.min(span.end));
        if from < to {
            out.push(Run { text: run.text[from - span.start..to - span.start].to_owned(), ..run.clone() });
        }
    }
    out
}
