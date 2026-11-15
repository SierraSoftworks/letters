//! Presence: who else has the document open, and where their caret is. Passed on, never kept.

use letters_model::{Model, Selection, StablePos};

use crate::wire::SessionId;

/// A selection by positions that stay with their text, so that it means the same on a replica that has
/// seen other edits than the one it was taken on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passage {
    pub anchor: StablePos,
    /// The end that moves. Equal to `anchor` for a caret.
    pub head: StablePos,
}

impl Passage {
    /// The selection as it can be told to others. `None` when it is not in the document `model` shows.
    pub fn of(model: &Model, selection: &Selection) -> Option<Self> {
        let _ = (model, selection);
        todo!()
    }

    /// Where the selection is in `model` now. `None` when its text is gone.
    pub fn selection(&self, model: &Model) -> Option<Selection> {
        let _ = model;
        todo!()
    }
}

/// Someone else with the document open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// The same for as long as their session lasts, and never given to two sessions of one room.
    pub session: SessionId,
    pub name: String,
    /// Which of the window's presence colours is theirs. The room hands them out, so everyone sees a
    /// person in the same colour. A window with fewer colours takes it modulo their number.
    pub colour: u8,
    /// `None` while they have no caret in the text.
    pub selection: Option<Passage>,
}
