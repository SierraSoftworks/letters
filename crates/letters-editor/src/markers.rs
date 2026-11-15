//! The Markdown markers shown around the caret, and the caret stops they add.
//!
//! Markers are a view of the formatting, not text in the document. The caret counts as an empty span that
//! carries the marks typed text will take, so a marker sits wherever those marks differ from a neighbour's.

use std::collections::BTreeSet;

use letters_model::{Block, BlockId, BlockKind, Mark, Marker, Markers, Tone};

use crate::{Editor, text};

/// The marks that show a pair of markers, outermost first.
pub(crate) const PAIRED: [Mark; 5] = [Mark::Bold, Mark::Italic, Mark::Strike, Mark::Code, Mark::Math];

fn delimiter(mark: &Mark) -> &'static str {
    match mark {
        Mark::Bold => "**",
        Mark::Italic => "*",
        Mark::Strike => "~~",
        Mark::Code => "`",
        Mark::Math => "$",
        _ => "",
    }
}

/// A marker at the caret's own offset: its mark, and whether it opens or closes the span.
pub(crate) type Stop = (Mark, bool);

/// The paired marks of the characters either side of an offset.
pub(crate) struct Sides {
    pub left: BTreeSet<Mark>,
    pub right: BTreeSet<Mark>,
}

impl Sides {
    /// Code and formula blocks show their text as written, so nothing in one is a marker.
    pub fn at(block: &Block, offset: usize) -> Self {
        if text::is_source(block) {
            return Self { left: BTreeSet::new(), right: BTreeSet::new() };
        }
        let runs = block.body.runs();
        Self { left: text::marks_before(runs, offset), right: text::marks_after(runs, offset) }
    }

    /// The markers between the character before the caret and the caret, in drawing order.
    pub fn before(&self, typing: &BTreeSet<Mark>) -> Vec<Stop> {
        between(&self.left, typing)
    }

    /// The markers between the caret and the character after it, in drawing order.
    pub fn after(&self, typing: &BTreeSet<Mark>) -> Vec<Stop> {
        between(typing, &self.right)
    }
}

/// Spans close innermost first, then open outermost first.
fn between(left: &BTreeSet<Mark>, right: &BTreeSet<Mark>) -> Vec<Stop> {
    let closes = PAIRED.iter().rev().filter(|mark| left.contains(mark) && !right.contains(mark));
    let opens = PAIRED.iter().filter(|mark| !left.contains(mark) && right.contains(mark));
    closes.map(|mark| (mark.clone(), false)).chain(opens.map(|mark| (mark.clone(), true))).collect()
}

fn tag(tone: Tone) -> &'static str {
    match tone {
        Tone::Tip => "[!TIP]",
        Tone::Info => "[!INFO]",
        Tone::Warn => "[!WARN]",
        Tone::Danger => "[!DANGER]",
    }
}

impl Editor {
    /// The markers to draw. Inline pairs show only at a caret, so a selection never moves the text under it.
    ///
    /// Stepping over a marker changes what this returns and leaves the selection where it was, so read it
    /// again after every call that may move the caret.
    pub fn markers(&self) -> Markers {
        let Some(caret) = &self.caret else { return Markers::default() };
        let head = &caret.selection.head;
        let Some(block) = self.block(&head.block) else { return Markers::default() };
        let mut markers = Markers { block: self.block_marker(block), ..Markers::default() };
        if !caret.selection.is_caret() || caret.picture || text::is_source(block) {
            return markers;
        }

        let runs = block.body.runs();
        let sides = Sides::at(block, head.offset);
        let marker = |offset: usize, mark: &Mark| Marker { offset, text: delimiter(mark) };
        // Each span that reaches the caret from the left opens further back, and each on the right closes
        // further on.
        let mut opens: Vec<Marker> = PAIRED
            .iter()
            .filter(|mark| sides.left.contains(mark))
            .map(|mark| marker(text::stretch_start(runs, head.offset, |run| run.marks.contains(mark)), mark))
            .collect();
        opens.sort_by_key(|marker| marker.offset);
        let mut closes: Vec<Marker> = PAIRED
            .iter()
            .rev()
            .filter(|mark| sides.right.contains(mark))
            .map(|mark| marker(text::stretch_end(runs, head.offset, |run| run.marks.contains(mark)), mark))
            .collect();
        closes.sort_by_key(|marker| marker.offset);

        let here = |stops: Vec<Stop>| stops.into_iter().map(|(mark, _)| marker(head.offset, &mark)).collect::<Vec<_>>();
        markers.inline = opens;
        markers.inline.extend(here(sides.before(&caret.marks)));
        markers.before_caret = markers.inline.len();
        markers.inline.extend(here(sides.after(&caret.marks)));
        markers.inline.extend(closes);
        markers
    }

    /// A heading shows its `#`s. A block inside a callout shows the callout's tag.
    fn block_marker(&self, block: &Block) -> Option<(BlockId, String)> {
        if let BlockKind::Heading { level } = block.kind {
            return Some((block.id.clone(), "#".repeat(level.into())));
        }
        let mut id = block.id.clone();
        while let Some(parent) = self.seat(&id).and_then(|place| place.parent) {
            if let BlockKind::Callout { tone } = parent.kind {
                return Some((parent.id.clone(), tag(tone).to_owned()));
            }
            id = parent.id.clone();
        }
        None
    }
}
