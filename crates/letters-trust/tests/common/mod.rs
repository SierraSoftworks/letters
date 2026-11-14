//! Shared fixtures: a settings file in memory, and a user who trusts a little of everything.
#![allow(dead_code)]

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use letters_trust::{Gate, Origin, Store, Trust, TrustStore};

pub const HOME: &str = "/home/ada";

/// A settings file as a user might keep it, with settings that are not ours.
pub const SETTINGS: &str = r#"# Ada's settings
theme = "dark"

[trust]
documents = ["doc-trusted"]
folders = ["~/Documents", "/srv/shared"] # work and family
image_sources = ["https://upload.wikimedia.org/", "https://img.example.org/charts/"]

[spelling]
language = "en-GB"
"#;

/// A settings file in memory. Clones share the one file, so a test can read what the store wrote.
#[derive(Clone, Default)]
pub struct Memory {
    pub text: Rc<RefCell<Option<String>>>,
    pub writes: Rc<RefCell<usize>>,
    pub broken: Rc<RefCell<bool>>,
}

impl Memory {
    pub fn holding(text: &str) -> Self {
        let memory = Self::default();
        memory.set(text);
        memory
    }

    pub fn set(&self, text: &str) {
        *self.text.borrow_mut() = Some(text.into());
    }

    pub fn text(&self) -> String {
        self.text.borrow().clone().unwrap_or_default()
    }
}

impl Store for Memory {
    fn read(&mut self) -> io::Result<Option<String>> {
        match *self.broken.borrow() {
            true => Err(io::Error::other("the disk is gone")),
            false => Ok(self.text.borrow().clone()),
        }
    }

    fn write(&mut self, text: &str) -> io::Result<()> {
        if *self.broken.borrow() {
            return Err(io::Error::other("the disk is gone"));
        }
        *self.writes.borrow_mut() += 1;
        self.set(text);
        Ok(())
    }
}

pub fn trust(settings: &str) -> Trust {
    Trust::parse(settings, Some(HOME)).expect("the settings are valid TOML")
}

/// A loaded store over `settings`, and a handle on its file.
pub fn store(settings: &str) -> (TrustStore<Memory>, Memory) {
    let memory = Memory::holding(settings);
    let mut store = TrustStore::new(memory.clone(), Some(HOME));
    store.load().expect("the settings load");
    (store, memory)
}

/// The gate of a document the user trusts by id.
pub fn trusted(trust: &Trust) -> Gate<'_> {
    trust.gate("doc-trusted", Origin::Outside)
}

/// The gate of a document nobody vouches for.
pub fn safe(trust: &Trust) -> Gate<'_> {
    trust.gate("doc-unknown", Origin::Outside)
}
