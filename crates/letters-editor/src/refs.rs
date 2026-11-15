//! References to figures: words that name a figure by its number, and follow the figure wherever it goes.
//!
//! A reference is a stretch of text that carries [`Mark::Ref`]. Its text is only what a reader without
//! references sees, since layout draws the label from the figure. No place inside it means anything, so
//! the caret takes it in one step and a selection takes it whole.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use letters_model::{Block, BlockId, Body, Mark, Op, Pos, Run};

use crate::{Caret, Editor, parts, text, tree};

/// A numbered figure or gallery, as a list to pick a reference from shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Numbered {
    pub id: BlockId,
    pub number: u32,
    /// The words of its caption.
    pub caption: String,
}

/// A reference in the making: the word for a figure typed before the caret, and the digits after it so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Referring {
    pub block: BlockId,
    /// Where the typed words are in the block's text.
    pub range: Range<usize>,
    /// The number typed, once one is.
    pub number: Option<u32>,
}

/// The figure a run refers to.
fn target(run: &Run) -> Option<&BlockId> {
    run.marks.iter().find_map(|mark| if let Mark::Ref(id) = mark { Some(id) } else { None })
}

/// Whether text in `marks` is meant as written, so that nothing in it is a reference.
fn literal(marks: &BTreeSet<Mark>) -> bool {
    marks.contains(&Mark::Code) || marks.contains(&Mark::Math)
}

/// Marks without any reference, which text typed beside one does not take.
pub(crate) fn plain(mut marks: BTreeSet<Mark>) -> BTreeSet<Mark> {
    marks.retain(|mark| !matches!(mark, Mark::Ref(_)));
    marks
}

/// The reference that `offset` is inside of, strictly: where in the block's text it sits.
pub(crate) fn around(block: &Block, offset: usize) -> Option<Range<usize>> {
    let runs = block.body.runs();
    let before = offset.checked_sub(1).and_then(|at| text::run_at(runs, at)).and_then(target)?;
    let same = |run: &Run| target(run) == Some(before);
    text::run_at(runs, offset)
        .is_some_and(same)
        .then(|| text::stretch_start(runs, offset, same)..text::stretch_end(runs, offset, same))
}

/// The reference that ends at `offset`, or with `after` set starts there, and the figure it refers to.
fn beside(block: &Block, offset: usize, after: bool) -> Option<(Range<usize>, BlockId)> {
    let runs = block.body.runs();
    let at = if after { Some(offset) } else { offset.checked_sub(1) };
    let to = at.and_then(|at| text::run_at(runs, at)).and_then(target)?.clone();
    let same = |run: &Run| target(run) == Some(&to);
    let range = if after {
        offset..text::stretch_end(runs, offset, same)
    } else {
        text::stretch_start(runs, offset, same)..offset
    };
    Some((range, to))
}

/// Where the words of a reference being written start in `typed`, the text from the start of a block to
/// the caret, and the number typed so far. The words are `word`, then a space and digits, any of which
/// may still be to come.
fn written(typed: &str, word: &str) -> Option<(usize, Option<u32>)> {
    let digits = typed.len() - typed.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (before, number) = typed.split_at(typed.len() - digits);
    let before = match before.strip_suffix(' ') {
        Some(before) => before,
        None if number.is_empty() => before,
        None => return None,
    };
    let start = before.len().checked_sub(word.len())?;
    let same = before.get(start..).is_some_and(|found| found.to_lowercase() == word.to_lowercase());
    // "Configure 2" names no figure.
    let apart = before.get(..start).is_some_and(|text| !text.ends_with(char::is_alphanumeric));
    (same && apart).then(|| (start, number.parse().ok()))
}

/// Points the references in pasted blocks at what they mean here. One to a block that was pasted along
/// and given a new id follows it there, and one to nothing the document has becomes plain words.
pub(crate) fn relink(blocks: &mut [Block], renewed: &BTreeMap<BlockId, BlockId>, known: &BTreeSet<BlockId>) {
    for block in blocks {
        match &mut block.body {
            Body::Blocks(children) => relink(children, renewed, known),
            Body::Text(runs) => {
                for run in runs {
                    let Some(to) = target(run).cloned() else { continue };
                    run.marks.remove(&Mark::Ref(to.clone()));
                    let there = renewed.get(&to).or(known.get(&to));
                    run.marks.extend(there.map(|id| Mark::Ref(id.clone())));
                }
            }
            Body::None => {}
        }
    }
}

impl Editor {
    /// The numbered figures and galleries in the order of their numbers, for a list to pick a reference
    /// from.
    pub fn numbered_figures(&self) -> Vec<Numbered> {
        let numbered = |(id, number): (BlockId, u32)| {
            let block = self.block(&id)?;
            let caption = if tree::is_gallery(block) { parts::caption(block) } else { Some(block) };
            Some(Numbered { id, number, caption: caption.map(Block::plain_text).unwrap_or_default() })
        };
        letters_layout::figure_numbers(self.doc()).into_iter().filter_map(numbered).collect()
    }

    /// The reference being typed at the caret: the style's word for a figure, such as "Figure", and any
    /// number after it. While there is one, a list of [`Editor::numbered_figures`] belongs by the caret.
    /// `None` when the document has no numbered figure, and once Esc has dismissed it.
    pub fn referring(&self) -> Option<Referring> {
        let caret = self.caret.as_ref().filter(|caret| caret.selection.is_caret() && !caret.picture)?;
        self.referring_at(&caret.selection.head, &caret.marks)
    }

    /// See [`Editor::referring`]. `marks` are those of the caret.
    fn referring_at(&self, at: &Pos, marks: &BTreeSet<Mark>) -> Option<Referring> {
        let typed = self.words_at(at, marks)?;
        let dismissed = self.dismissed.as_ref() == Some(&(typed.block.clone(), typed.range.start));
        (!dismissed).then_some(typed)
    }

    /// The words for a figure that end at `at`, whether or not they were dismissed.
    fn words_at(&self, at: &Pos, marks: &BTreeSet<Mark>) -> Option<Referring> {
        let block = self.block(&at.block).filter(|block| !text::is_source(block))?;
        let text = block.plain_text();
        let word = letters_layout::figure_word(self.model.style());
        let (start, number) = written(text.get(..at.offset)?, word)?;
        // Code and maths are meant as written, a reference is not made twice, and a label at the very
        // start of a caption is the caption's own.
        let made = beside(block, at.offset, false).is_some();
        let own = start == 0 && self.numbered_by(&at.block).is_some();
        let none = letters_layout::figure_numbers(self.doc()).is_empty();
        let typed = Referring { block: at.block.clone(), range: start..at.offset, number };
        (!literal(marks) && !made && !own && !none).then_some(typed)
    }

    /// A dismissal lasts while the caret stays in the words it was for.
    pub(crate) fn forget(&mut self) {
        let at = |caret: &&Caret| caret.selection.is_caret() && !caret.picture;
        let caret = self.caret.as_ref().filter(|_| self.dismissed.is_some()).filter(at);
        let words = caret.and_then(|caret| self.words_at(&caret.selection.head, &caret.marks));
        if words.map(|typed| (typed.block, typed.range.start)) != self.dismissed {
            self.dismissed = None;
        }
    }

    /// Whether a reference can go where the selection is: in text that is not code or maths.
    pub(crate) fn referable(&self) -> bool {
        let Some(caret) = self.caret.as_ref().filter(|caret| !caret.picture) else { return false };
        let source = self.block(&caret.selection.head.block).is_none_or(text::is_source);
        !source && !literal(&caret.marks)
    }

    /// The numbered figure that the number of a reference in the making names.
    fn named(&self, typed: &Referring) -> Option<BlockId> {
        let figures = letters_layout::figure_numbers(self.doc());
        figures.into_iter().find(|(_, number)| Some(*number) == typed.number).map(|(id, _)| id)
    }

    /// Writes a reference to a numbered figure over `range` of a block and leaves the caret after it.
    /// Returns whether `to` is a numbered figure.
    fn refer(&mut self, to: &BlockId, block: &BlockId, range: Range<usize>, marks: &BTreeSet<Mark>) -> bool {
        let figures = letters_layout::figure_numbers(self.doc());
        let Some((_, number)) = figures.iter().find(|(id, _)| id == to) else { return false };
        // The words are what a reader without references sees. Layout draws its own from the figure.
        let words = format!("{} {number}", letters_layout::figure_word(self.model.style()));
        let mut marks = plain(marks.clone());
        marks.insert(Mark::Ref(to.clone()));
        // Words that were a link stay one.
        let runs = self.block(block).map(|block| block.body.runs()).unwrap_or_default();
        let link = text::run_at(runs, range.start).filter(|_| !range.is_empty()).and_then(|run| run.link.clone());
        self.replace(block, range.clone(), &words, &marks, &link);
        self.place(Pos::new(block.clone(), range.start + words.len()));
        true
    }

    /// Puts a reference to a numbered figure at the caret, in place of the words typed for it or of the
    /// selection.
    pub(crate) fn take_reference(&mut self, to: &BlockId) {
        let typed = self.referring();
        let numbered = letters_layout::figure_numbers(self.doc()).iter().any(|(id, _)| id == to);
        let Some(caret) = self.caret.clone().filter(|_| numbered && self.referable()) else { return };
        self.delete_selection();
        let Some(at) = self.selection().map(|selection| selection.head.clone()) else { return };
        let range = typed.filter(|typed| typed.block == at.block).map_or(at.offset..at.offset, |typed| typed.range);
        self.refer(to, &at.block, range, &caret.marks);
    }

    /// A space or a full stop typed after the word for a figure and a number that a figure has makes the
    /// words a reference to it. `at` is the caret, after the character just typed.
    pub(crate) fn reference_rule(&mut self, at: &Pos, marks: &BTreeSet<Mark>) -> bool {
        let text = self.block(&at.block).map(Block::plain_text).unwrap_or_default();
        let Some(end) = text.get(..at.offset).and_then(|typed| typed.strip_suffix([' ', '.'])).map(str::len) else {
            return false;
        };
        let Some(typed) = self.referring_at(&Pos::new(at.block.clone(), end), marks) else { return false };
        let Some(to) = self.named(&typed) else { return false };
        if !self.refer(&to, &at.block, typed.range, marks) {
            return false;
        }
        // The caret goes back to where it was typing, after the character that closed the reference.
        let Some(after) = self.selection().map(|selection| selection.head.offset + at.offset - end) else {
            return true;
        };
        self.place_with(Pos::new(at.block.clone(), after), marks.clone());
        true
    }

    /// Enter after the word for a figure and a number that a figure has takes the reference. Returns
    /// whether it did.
    pub(crate) fn enter_reference(&mut self) -> bool {
        let Some((typed, marks)) = self.referring().zip(self.caret.as_ref().map(|caret| caret.marks.clone())) else {
            return false;
        };
        self.named(&typed).is_some_and(|to| self.refer(&to, &typed.block, typed.range, &marks))
    }

    /// Esc leaves the words typed for a reference as words.
    pub(crate) fn dismiss_reference(&mut self) {
        self.dismissed = self.referring().map(|typed| (typed.block, typed.range.start));
    }

    /// Backspace after a reference, or Delete before one, makes it the words it shows. Returns whether
    /// there was a reference there.
    pub(crate) fn unrefer(&mut self, at: &Pos, after: bool) -> bool {
        let Some(block) = self.block(&at.block) else { return false };
        let Some((range, to)) = beside(block, at.offset, after) else { return false };
        let runs = block.body.runs();
        let (old, marks) = (block.plain_text()[range.clone()].to_owned(), plain(text::marks_after(runs, range.start)));
        let link = text::run_at(runs, range.start).and_then(|run| run.link.clone());
        // What it shows is the figure's number as it is now, or a question mark for a figure that is gone.
        let number = letters_layout::figure_numbers(self.doc()).into_iter().find(|(id, _)| *id == to);
        let number = number.map_or("?".to_owned(), |(_, number)| number.to_string());
        let words = format!("{} {number}", letters_layout::figure_word(self.model.style()));
        if words == old {
            self.apply(vec![Op::SetMark {
                block: at.block.clone(),
                range: range.clone(),
                mark: Mark::Ref(to),
                on: false,
            }]);
        } else {
            self.replace(&at.block, range.clone(), &words, &marks, &link);
        }
        let end = if after { range.start } else { range.start + words.len() };
        self.place(Pos::new(at.block.clone(), end));
        // The words are left as words, so they do not ask to be a reference again.
        self.dismissed = Some((at.block.clone(), range.start));
        true
    }
}
