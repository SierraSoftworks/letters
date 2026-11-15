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
        let model = Model::new(document.clone(), replica()).with_style(style.clone());
        Self { model, copy: files::new_copy(document.clone(), style) }
    }

    /// The writer of a document that was opened, once the rest of its file is read as it is after the first
    /// paint. Returns where its saves go too.
    pub fn of(opened: Opened) -> (Self, Home) {
        let replica = replica();
        // The editor's history starts from the style on screen, as a window's does.
        let mut model = Model::new(opened.document, replica).with_style(opened.style);
        let Working { copy, history, .. } = opened.rest.read(replica, None).unwrap();
        if let Some(history) = history {
            model.attach(history);
        }
        (Self { model, copy }, opened.home.expect("an opened file is its home"))
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

/// A scratch folder that is removed when the test ends.
pub struct Scratch(pub std::path::PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("letters-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    /// The names of what a folder in it holds, which is nothing when the folder is not there.
    pub fn names(&self, folder: &str) -> Vec<String> {
        let entries = std::fs::read_dir(self.0.join(folder)).into_iter().flatten();
        let mut names: Vec<String> = entries.map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
