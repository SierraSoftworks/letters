//! The editing state of a one-line text input: its selection, scroll and any text an input method is still
//! composing. The text itself belongs to the app, so every edit here returns the new value to send it.

use std::ops::Range;

use crate::text::Shaped;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct InputState {
    /// Where the selection started, as a byte offset into the value.
    pub anchor: usize,
    /// Where the caret is.
    pub head: usize,
    /// How far the text is scrolled to keep the caret in view.
    pub scroll: f32,
    /// Text an input method is composing at the caret. It is shown but is not part of the value yet.
    pub composing: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Start,
    End,
}

impl InputState {
    /// The selection as `(start, end)`, kept inside the value and on character boundaries.
    pub fn ordered(&self, value: &str) -> (usize, usize) {
        let (anchor, head) = (floor(value, self.anchor), floor(value, self.head));
        (anchor.min(head), anchor.max(head))
    }

    pub fn selected<'a>(&self, value: &'a str) -> &'a str {
        let (start, end) = self.ordered(value);
        &value[start..end]
    }

    /// What the input displays: the value with the composition at the caret.
    pub fn shown(&self, value: &str) -> String {
        match &self.composing {
            Some(text) => {
                let at = floor(value, self.head);
                format!("{}{}{}", &value[..at], text, &value[at..])
            }
            None => value.to_owned(),
        }
    }

    /// The composition's place in the shown text.
    pub fn composing_range(&self, value: &str) -> Option<Range<usize>> {
        let text = self.composing.as_ref()?;
        let at = floor(value, self.head);
        Some(at..at + text.len())
    }

    /// The caret's offset in the shown text: after the composition while there is one.
    pub fn caret(&self, value: &str) -> usize {
        floor(value, self.head) + self.composing.as_ref().map_or(0, String::len)
    }

    pub fn select_all(&mut self, value: &str) {
        (self.anchor, self.head) = (0, value.len());
    }

    /// Puts the caret at `offset`, extending the selection instead when `extend` is set.
    pub fn place(&mut self, value: &str, offset: usize, extend: bool) {
        self.head = floor(value, offset);
        if !extend {
            self.anchor = self.head;
        }
    }

    pub fn select_word(&mut self, value: &str, offset: usize) {
        let offset = floor(value, offset);
        let class = value[offset..].chars().next().or_else(|| value[..offset].chars().next_back()).map(Class::of);
        let Some(class) = class else { return };
        let before =
            value[..offset].chars().rev().take_while(|&c| Class::of(c) == class).map(char::len_utf8).sum::<usize>();
        let after = value[offset..].chars().take_while(|&c| Class::of(c) == class).map(char::len_utf8).sum::<usize>();
        (self.anchor, self.head) = (offset - before, offset + after);
    }

    /// Moves the caret. A plain move with text selected goes to that end of the selection.
    pub fn step(&mut self, value: &str, shaped: &Shaped, motion: Motion, extend: bool) {
        let (start, end) = self.ordered(value);
        let head = floor(value, self.head);
        let target = match motion {
            Motion::Left if !extend && start != end => start,
            Motion::Right if !extend && start != end => end,
            _ => target(value, shaped, head, motion),
        };
        self.place(value, target, extend);
    }

    /// Replaces the selection with `text`, on one line.
    pub fn insert(&mut self, value: &str, text: &str) -> String {
        let (start, end) = self.ordered(value);
        self.replace(value, start..end, text)
    }

    /// Replaces a byte range of the value, leaving the caret after the new text.
    pub fn replace(&mut self, value: &str, range: Range<usize>, text: &str) -> String {
        let (start, end) = (floor(value, range.start), floor(value, range.end.max(range.start)));
        let text: String = text.chars().map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c }).collect();
        self.head = start + text.len();
        self.anchor = self.head;
        format!("{}{}{}", &value[..start], text, &value[end..])
    }

    /// Deletes the selection, or else one step from the caret. `None` when there is nothing to delete.
    pub fn delete(&mut self, value: &str, shaped: &Shaped, motion: Motion) -> Option<String> {
        let (mut start, mut end) = self.ordered(value);
        if start == end {
            let to = target(value, shaped, start, motion);
            (start, end) = (start.min(to), start.max(to));
        }
        (start != end).then(|| self.replace(value, start..end, ""))
    }

    /// Deletes bytes around the caret, as a soft keyboard asks.
    pub fn delete_around(&mut self, value: &str, before: usize, after: usize) -> Option<String> {
        let (start, end) = self.ordered(value);
        let range = floor(value, start.saturating_sub(before))..floor(value, end.saturating_add(after));
        (!range.is_empty()).then(|| self.replace(value, range, ""))
    }

    /// Scrolls just far enough to keep the caret inside a box `width` wide.
    pub fn reveal(&mut self, value: &str, shaped: &Shaped, width: f32) {
        let caret = shaped.x_at(self.caret(value));
        let overflow = (shaped.size.width + 1.0 - width).max(0.0);
        self.scroll = self.scroll.clamp(caret + 1.0 - width.max(1.0), caret).clamp(0.0, overflow);
    }
}

fn target(value: &str, shaped: &Shaped, from: usize, motion: Motion) -> usize {
    match motion {
        Motion::Left => shaped.previous(from),
        Motion::Right => shaped.next(from),
        Motion::Start => 0,
        Motion::End => value.len(),
        // To the start of the word behind the caret, or the end of the word ahead of it.
        Motion::WordLeft => from - run(value[..from].chars().rev()),
        Motion::WordRight => from + run(value[from..].chars()),
    }
}

/// The length in bytes of any spaces and then one run of like characters.
fn run(chars: impl Iterator<Item = char> + Clone) -> usize {
    let spaces = chars.clone().take_while(|c| c.is_whitespace()).map(char::len_utf8).sum::<usize>();
    let mut rest = chars.skip_while(|c| c.is_whitespace()).peekable();
    let Some(class) = rest.peek().copied().map(Class::of) else { return spaces };
    spaces + rest.take_while(|&c| Class::of(c) == class).map(char::len_utf8).sum::<usize>()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

impl Class {
    fn of(c: char) -> Self {
        if c.is_alphanumeric() || c == '_' {
            Class::Word
        } else if c.is_whitespace() {
            Class::Space
        } else {
            Class::Other
        }
    }
}

/// The nearest character boundary at or before `offset`.
fn floor(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{FixedShaper, TextShaper, TextStyle};

    fn shaped(value: &str) -> Shaped {
        FixedShaper.shape(value, &TextStyle::control(), None)
    }

    fn at(offset: usize) -> InputState {
        InputState { anchor: offset, head: offset, ..InputState::default() }
    }

    #[test]
    fn motions_land_where_a_writer_expects() {
        let value = "send the quote, Ana";
        // (from, motion, to)
        let cases = [
            (0, Motion::Right, 1),
            (1, Motion::Left, 0),
            (0, Motion::Left, 0),
            (19, Motion::Right, 19),
            (7, Motion::Start, 0),
            (7, Motion::End, 19),
            (0, Motion::WordRight, 4),
            (4, Motion::WordRight, 8),
            (9, Motion::WordRight, 14),
            (14, Motion::WordRight, 15),
            (19, Motion::WordLeft, 16),
            (16, Motion::WordLeft, 14),
            (9, Motion::WordLeft, 5),
        ];
        for (from, motion, to) in cases {
            let mut state = at(from);
            state.step(value, &shaped(value), motion, false);
            assert_eq!((state.anchor, state.head), (to, to), "{motion:?} from {from}");
        }
    }

    #[test]
    fn extending_keeps_the_anchor_and_a_plain_move_collapses_to_an_end() {
        let value = "quote";
        let mut state = at(1);
        state.step(value, &shaped(value), Motion::End, true);
        assert_eq!(state.selected(value), "uote");
        state.step(value, &shaped(value), Motion::Left, false);
        assert_eq!((state.anchor, state.head), (1, 1));
    }

    #[test]
    fn typing_replaces_the_selection_and_stays_on_one_line() {
        let mut state = InputState { anchor: 4, head: 0, ..InputState::default() };
        assert_eq!(state.insert("send it", "post\nnow"), "post now it");
        assert_eq!((state.anchor, state.head), (8, 8));
    }

    #[test]
    fn deleting_takes_the_selection_or_one_step() {
        let value = "né quote";
        // (anchor, head, motion, result)
        let cases = [
            (3, 3, Motion::Left, Some("n quote")),
            (3, 3, Motion::Right, Some("néquote")),
            (4, 1, Motion::Left, Some("nquote")),
            (9, 9, Motion::WordLeft, Some("né ")),
            (0, 0, Motion::WordRight, Some(" quote")),
            (0, 0, Motion::Left, None),
            (9, 9, Motion::Right, None),
        ];
        for (anchor, head, motion, expected) in cases {
            let mut state = InputState { anchor, head, ..InputState::default() };
            let result = state.delete(value, &shaped(value), motion);
            assert_eq!(result.as_deref(), expected, "{motion:?} at {anchor}..{head}");
        }
    }

    #[test]
    fn offsets_inside_a_character_or_past_the_end_are_made_safe() {
        let value = "né";
        let mut state = InputState { anchor: 2, head: 99, ..InputState::default() };
        assert_eq!(state.ordered(value), (1, 3));
        assert_eq!(state.replace(value, 2..2, "x"), "nxé");
        assert_eq!(state.delete_around("abc", 99, 0), Some("c".to_owned()), "the caret was left after 'nx'");
    }

    #[test]
    fn a_composition_shows_at_the_caret_without_changing_the_value() {
        let state = InputState { anchor: 2, head: 2, composing: Some("にほ".into()), ..InputState::default() };
        assert_eq!(state.shown("abcd"), "abにほcd");
        assert_eq!(state.composing_range("abcd"), Some(2..8));
        assert_eq!(state.caret("abcd"), 8);
    }

    #[test]
    fn a_double_click_selects_the_word_under_it() {
        let value = "send the quote";
        for (offset, word) in [(0, "send"), (6, "the"), (14, "quote"), (4, " ")] {
            let mut state = at(0);
            state.select_word(value, offset);
            assert_eq!(state.selected(value), word, "at {offset}");
        }
    }

    #[test]
    fn the_caret_is_kept_in_view() {
        let value = "a long address that does not fit";
        let shape = shaped(value);
        let mut state = at(value.len());
        state.reveal(value, &shape, 70.0);
        let caret = shape.x_at(value.len()) - state.scroll;
        assert!((0.0..=70.0).contains(&caret), "caret at {caret}");
        state.place(value, 0, false);
        state.reveal(value, &shape, 70.0);
        assert_eq!(state.scroll, 0.0);
    }
}
