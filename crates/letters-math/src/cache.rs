//! Typeset formulas, kept so that painting, scrolling and editing around one do not set it again.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use letters_render::Color;

use crate::{Error, Formula, Style, typeset};

/// Typesets formulas and keeps the results, by source and style. A source that fails keeps its error.
///
/// It holds a fixed number of results and drops the one unused for longest, so a formula being typed (a new
/// source at every keystroke) does not grow it. Keep one for the life of a document's layout.
#[derive(Debug)]
pub struct Cache {
    entries: HashMap<u64, Entry>,
    capacity: usize,
    /// Counts lookups, to tell which entry was used longest ago.
    clock: u64,
}

#[derive(Debug)]
struct Entry {
    source: Box<str>,
    style: Style,
    result: Result<Arc<Formula>, Error>,
    used: u64,
}

impl Default for Cache {
    /// Room for the formulas of several screens.
    fn default() -> Self {
        Self::new(512)
    }
}

impl Cache {
    /// A cache of at most `capacity` results, and at least one.
    pub fn new(capacity: usize) -> Self {
        Self { entries: HashMap::new(), capacity: capacity.max(1), clock: 0 }
    }

    /// The formula for `source` in `style`, typeset now if it is not held.
    pub fn typeset(&mut self, source: &str, style: &Style) -> Result<Arc<Formula>, Error> {
        self.clock += 1;
        let key = key(source, style);
        if let Some(entry) = self.entries.get_mut(&key)
            && *entry.source == *source
            && same(&entry.style, style)
        {
            entry.used = self.clock;
            return entry.result.clone();
        }

        let result = typeset(source, style).map(Arc::new);
        // An engine that is not here yet will be: what it sets then is what to keep.
        if result.as_ref().is_err_and(Error::is_pending) {
            return result;
        }
        if self.entries.len() >= self.capacity
            && !self.entries.contains_key(&key)
            && let Some(oldest) = self.entries.iter().min_by_key(|(_, entry)| entry.used).map(|(key, _)| *key)
        {
            self.entries.remove(&oldest);
        }
        let entry = Entry { source: source.into(), style: *style, result: result.clone(), used: self.clock };
        self.entries.insert(key, entry);
        result
    }

    /// How many results are held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// Entries are found by a hash of source and style, so a lookup copies nothing. Two that collide share a
/// slot, and the later one takes it.
fn key(source: &str, style: &Style) -> u64 {
    let mut hasher = DefaultHasher::new();
    (source, parts(style)).hash(&mut hasher);
    hasher.finish()
}

/// Whether two styles set a formula alike.
fn same(a: &Style, b: &Style) -> bool {
    parts(a) == parts(b)
}

/// Everything in a style, with the size as its bits so that it can be hashed. Each field is named, so that
/// a new one cannot be left out of the key.
fn parts(style: &Style) -> (u32, Color, bool, bool) {
    let Style { size, color, display, links } = *style;
    (size.to_bits(), color, display, links)
}
