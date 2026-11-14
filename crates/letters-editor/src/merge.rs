//! Taking in what other writers wrote while this editor is open.
//!
//! A [`Pos`] counts from the start of its block, so it drifts when another writer edits before it. Whatever
//! the editor keeps across calls is therefore kept twice: as it stood, and as positions that follow their
//! text, which are used once a merge has come between.

use letters_model::{Change, HistoryError, Hold, Pos, Selection, StablePos};

use crate::{Caret, Editor, tree};

/// The anchor and the head of a selection as positions that follow their text.
type Held = (Option<StablePos>, Option<StablePos>);

/// The caret at one moment, for undo to put back.
#[derive(Clone, Debug)]
pub(crate) struct Kept {
    pub caret: Option<Caret>,
    held: Option<Held>,
    /// How many merges there had been, which says whether `caret` still counts the same text.
    merges: u64,
}

impl Editor {
    /// Takes in what another writer wrote, as [`Model::merge`](letters_model::Model::merge) does. The
    /// selection and a composition in progress stay with their text.
    pub fn merge(&mut self, history: &[u8]) -> Result<Change, HistoryError> {
        let kept = self.keep();
        let typing = self.undo.last().is_some_and(|last| last.after.caret == kept.caret);
        let composed = self.composition().map(|(block, range)| {
            let at = |offset: usize| Pos::new(block.clone(), offset);
            self.hold(&Selection { anchor: at(range.start), head: at(range.end) })
        });

        let change = self.model.merge(history)?;
        if change == Change::default() {
            return Ok(change);
        }
        self.merges += 1;
        self.recall(&kept);
        // What is typed next still continues the step it would have continued.
        let after = self.keep();
        if let Some(last) = self.undo.last_mut().filter(|_| typing) {
            last.after = after;
        }
        let head = self.selection().map(|selection| &selection.head.block);
        let range = self
            .found(composed)
            .filter(|found| Some(&found.anchor.block) == head && found.anchor.block == found.head.block)
            .map(|found| found.anchor.offset..found.head.offset.max(found.anchor.offset));
        if let (Some(composing), Some(range)) = (self.composing.as_mut(), range) {
            composing.range = range;
        } else {
            // Another writer broke the composed text apart, so it stays as it is.
            self.end_composition();
        }
        Ok(change)
    }

    pub(crate) fn keep(&self) -> Kept {
        let held = self.caret.as_ref().map(|caret| self.hold(&caret.selection));
        Kept { caret: self.caret.clone(), held, merges: self.merges }
    }

    /// Puts a kept caret back. When other writers have edited since, it goes to where its text is now.
    pub(crate) fn recall(&mut self, kept: &Kept) {
        let moved = (kept.merges != self.merges).then(|| self.found(kept.held.clone())).flatten();
        let caret = kept.caret.clone();
        self.restore(caret.map(|caret| Caret { selection: moved.unwrap_or(caret.selection), marks: caret.marks }));
    }

    /// A range holds on to the text inside it and a caret to the character before it, so what another
    /// writer types at an edge stays outside.
    fn hold(&self, selection: &Selection) -> Held {
        let (start, end) = self.ordered(selection);
        let first = if selection.is_caret() { Hold::Prev } else { Hold::Next };
        let (from, to) = (self.model.stable(&start, first), self.model.stable(&end, Hold::Prev));
        if start == selection.anchor { (from, to) } else { (to, from) }
    }

    fn found(&self, held: Option<Held>) -> Option<Selection> {
        let (anchor, head) = held?;
        Some(Selection { anchor: self.model.resolve(&anchor?)?, head: self.model.resolve(&head?)? })
    }

    /// Sets a caret that may no longer fit the document, on the text there is now.
    fn restore(&mut self, caret: Option<Caret>) {
        let fit = |caret: &Caret| {
            let (anchor, head) = (self.clamp(&caret.selection.anchor)?, self.clamp(&caret.selection.head)?);
            Some(Caret { selection: Selection { anchor, head }, marks: caret.marks.clone() })
        };
        self.caret = caret.as_ref().and_then(fit);
        // Its block is gone, so the caret goes to the first text there is.
        let first = tree::texts(self.doc()).first().map(|block| Pos::new(block.id.clone(), 0));
        if let (None, Some(first)) = (&self.caret, first) {
            self.place(first);
        }
    }
}
