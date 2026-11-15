//! The document's history, as a view of it asks through the editor.
//!
//! Reading a history takes the model whole, which the editor does not hand out, so what a history view
//! asks is passed on here. Restoring a version is an edit like any other: [`Command::Restore`].

use letters_model::{Change, Document, HistoryError, Point, Revision, Style, TagKind};

use crate::Editor;
use crate::merge::moves;

impl Editor {
    /// Every step the history was written in, oldest first. See [`letters_model::Model::timeline`].
    pub fn timeline(&mut self) -> Vec<Point> {
        self.model.timeline()
    }

    /// The document and its style as they were at `revision`, read without leaving the one that shows.
    /// See [`letters_model::Model::document_at`].
    pub fn document_at(&mut self, revision: &Revision) -> Result<(Document, Style), HistoryError> {
        self.model.document_at(revision)
    }

    /// Marks `revision` as where a session, a pause, an import or a merge ended, for a shell that knows
    /// of one, with the name of the file an import came from. Like a name, the mark is something to save
    /// and no edit.
    ///
    /// Nothing is marked where a mark stands already: on the revision itself, so that no name a person
    /// gave is lost, and on the step that ends it, since a mark with nothing written after the last one
    /// would list an empty version. A revision nothing was written in needs none either.
    pub fn mark_version(&mut self, revision: &Revision, kind: TagKind, name: Option<&str>) -> Result<(), HistoryError> {
        if self.read_only {
            // A mark is written into the history, which one who only reads leaves as it is.
            return Ok(());
        }
        let steps = self.model.timeline();
        // How many steps the revision holds, where that can be told: all of them when it is the newest,
        // and else those before the step that was written on it.
        let held = match *revision == self.model.revision() {
            true => Some(steps.len()),
            false => steps.iter().position(|step| &step.before == revision),
        };
        let ended = held.map(|held| held.checked_sub(1).is_none_or(|last| !steps[last].tags.is_empty()));
        if ended == Some(true) || self.model.tags().iter().any(|tag| &tag.at == revision) {
            return Ok(());
        }
        self.model.tag(revision, kind, name)
    }

    /// Marks `revision` as a version a person named, or with no name takes its mark off. A mark is part
    /// of the history and so something to save, but no edit and no undo step. See
    /// [`letters_model::Model::tag`].
    pub fn name_version(&mut self, revision: &Revision, name: Option<&str>) -> Result<(), HistoryError> {
        if self.read_only {
            return Ok(());
        }
        match name {
            Some(name) => self.model.tag(revision, TagKind::Named, Some(name)),
            None => {
                self.model.untag(revision);
                Ok(())
            }
        }
    }

    /// Makes the document and its style read as they did at `revision`. A revision the history lacks, and
    /// one that reads as the document does, change nothing.
    pub(crate) fn restore_version(&mut self, revision: &Revision) {
        // The caret that undo comes back to is held while its text is still as it was.
        if self.steps == 0 && self.origin.held.is_none() {
            self.origin = self.held(self.origin.clone());
        }
        let kept = self.keep();
        let Ok(change) = self.model.restore(revision) else { return };
        // The model records an undo step only for a restore that changed something.
        if change == Change::default() {
            return;
        }
        self.steps += 1;
        // Text came and went under the caret as it does when another writer's edits are taken in.
        if moves(&change) {
            self.merges += 1;
            self.recall(&kept);
        }
        self.note(change);
    }
}
