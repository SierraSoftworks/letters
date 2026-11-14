//! Taking in what changes the document from outside while this editor is open.
//!
//! A [`Pos`] counts from the start of its block, so it drifts when another writer edits before it. A caret
//! that undo will put back is therefore kept twice: as it stood, and as positions that follow their text,
//! which are used once a merge has come between.
//!
//! Asking the model for such positions makes it build its history, which opening a document does not wait
//! for. So they are only taken when an edit or a merge is at hand, which needs the history anyway.

use letters_model::{Change, Document, HistoryError, Hold, Model, OpError, Pos, Prepared, Selection, StablePos};

use crate::{Caret, Editor, tree};

/// The anchor and the head of a selection as positions that follow their text.
type Held = (Option<StablePos>, Option<StablePos>);

/// The caret at one moment, for undo to put back.
#[derive(Clone, Debug, Default)]
pub(crate) struct Kept {
    pub caret: Option<Caret>,
    pub held: Option<Held>,
    /// How many merges there had been, which says whether `caret` still counts the same text.
    merges: u64,
}

impl Editor {
    /// Takes in what another writer wrote, as [`Model::merge`] does. The selection and a composition in
    /// progress stay with their text.
    pub fn merge(&mut self, history: &[u8]) -> Result<Change, HistoryError> {
        self.take_in(|model| model.merge(history))
    }

    /// Makes the document read as `doc`, which was changed outside the editor as a file edited by another
    /// tool is. See [`Model::rewrite`]. The selection stays with the text that is still there.
    pub fn rewrite(&mut self, doc: Document) -> Result<Change, OpError> {
        self.take_in(|model| model.rewrite(doc))
    }

    /// Takes over the history that [`Model::prepare`] made for this document, as [`Model::attach`] does.
    pub fn attach(&mut self, prepared: Prepared) -> Change {
        let change = self.model.attach(prepared);
        // Positions held in the history this one replaces mean nothing in it, and may name other text.
        let steps = self.undo.iter_mut().chain(&mut self.redo);
        let kept = steps.flat_map(|step| [&mut step.before, &mut step.after]).chain([&mut self.origin]);
        kept.for_each(|kept| kept.held = None);
        if change != Change::default() {
            self.merges += 1;
            self.restore(self.caret.clone());
        }
        change
    }

    fn take_in<E>(&mut self, edit: impl FnOnce(&mut Model) -> Result<Change, E>) -> Result<Change, E> {
        let kept = self.keep();
        let typing = self.undo.last().is_some_and(|last| last.after.caret == kept.caret);
        let composed = self.composition().map(|(block, range)| {
            let at = |offset: usize| Pos::new(block.clone(), offset);
            self.hold(&Selection { anchor: at(range.start), head: at(range.end) })
        });

        let change = edit(&mut self.model)?;
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
            // The composed text was broken apart, so it stays as it is.
            self.end_composition();
        }
        Ok(change)
    }

    /// Notes the caret as the one to come back to, unless edits are waiting to become an undo step.
    pub(crate) fn mark(&mut self) {
        if self.steps == 0 {
            self.origin = Kept { caret: self.caret.clone(), held: None, merges: self.merges };
        }
    }

    /// The caret as it stands, with the positions that follow its text.
    pub(crate) fn keep(&self) -> Kept {
        self.held(Kept { caret: self.caret.clone(), held: None, merges: self.merges })
    }

    /// Adds the positions that follow its text to a kept caret, whose text must still be as it was.
    pub(crate) fn held(&self, kept: Kept) -> Kept {
        let held = kept.caret.as_ref().map(|caret| self.hold(&caret.selection));
        Kept { held, ..kept }
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
        if selection.is_caret() {
            let held = self.model.stable(&selection.head, Hold::Prev);
            return (held.clone(), held);
        }
        let (start, end) = self.ordered(selection);
        let (from, to) = (self.model.stable(&start, Hold::Next), self.model.stable(&end, Hold::Prev));
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
