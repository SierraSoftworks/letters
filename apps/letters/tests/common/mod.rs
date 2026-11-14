//! A document open for writing, as a window holds it.

// Each test file uses its own part of this.
#![allow(dead_code)]

use letters::files::{self, Home, Opened, Working};
use letters::platform;
use letters_format::{Saved, WorkingCopy};
use letters_model::{Document, Model, ReplicaId, Style};

/// The two halves of a document being written: the editor's model, and the copy saves are made from.
pub struct Writer {
    pub model: Model,
    pub copy: WorkingCopy,
}

fn replica() -> ReplicaId {
    ReplicaId::new(&platform::block_id())
}

impl Writer {
    /// The writer of a new document.
    pub fn new(document: &Document, style: Style) -> Self {
        Self { model: Model::new(document.clone(), replica()), copy: files::new_copy(document.clone(), style) }
    }

    /// The writer of a document that was opened, once the rest of its file is read as it is after the first
    /// paint. Returns where its saves go too.
    pub fn of(opened: Opened) -> (Self, Home) {
        let replica = replica();
        let mut model = Model::new(opened.document, replica);
        let Working { copy, history } = opened.rest.read(replica).unwrap();
        if let Some(history) = history {
            model.attach(history);
        }
        (Self { model, copy }, opened.home)
    }

    /// Makes the document read as `document`, as typing would.
    pub fn write(&mut self, document: &Document) {
        self.model.rewrite(document.clone()).unwrap();
    }

    /// Writes `document` and saves it to `home`, as an autosave does. What the file held from another
    /// writer comes back into the model.
    pub fn save(&mut self, home: &Home, document: &Document) -> (Home, Saved) {
        self.write(document);
        self.copy.edit(&self.model.save()).unwrap();
        let saved = home.save(&mut self.copy).unwrap();
        self.model.merge(&self.copy.model().save()).unwrap();
        saved
    }
}
