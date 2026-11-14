//! Keeps a document's misspellings up to date, one block at a time.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::ops::Range;

use letters_model::{Block, BlockId, BlockKind, Change, Document, Mark, Run};

use crate::Speller;

/// The misspellings of a document, found a block at a time so that no call takes long.
///
/// Tell it about every edit with [`Proofing::invalidate`], and call [`Proofing::step`] while the shell is
/// idle until it returns `None`. It starts with the whole document to check, and checks it all again by
/// itself when the document's language or dictionary, the user's words or the dictionaries change.
#[derive(Debug, Default)]
pub struct Proofing {
    marks: HashMap<BlockId, Vec<Range<usize>>>,
    /// The blocks to check, soonest first. Only those still in `waiting` count.
    queue: VecDeque<BlockId>,
    waiting: HashSet<BlockId>,
    /// What the marks were checked against.
    seen: Option<Seen>,
}

#[derive(Debug, PartialEq)]
struct Seen {
    language: String,
    words: BTreeSet<String>,
    revision: u64,
}

/// The block one [`Proofing::step`] checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub block: BlockId,
    /// Whether its misspellings differ from before, so it needs repainting.
    pub changed: bool,
}

impl Proofing {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets everything, for when another document takes this one's place.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Notes what an edit touched. Those blocks lose their misspellings, whose offsets no longer fit the
    /// text, and are checked first. Call [`Proofing::step`] once for each before painting and they never
    /// show without them.
    pub fn invalidate(&mut self, change: &Change) {
        for block in &change.blocks {
            self.marks.remove(block);
            self.waiting.insert(block.clone());
            self.queue.push_front(block.clone());
        }
    }

    /// Checks the next block that needs it. `None` when every block is up to date.
    pub fn step(&mut self, speller: &Speller, doc: &Document) -> Option<Checked> {
        let fresh = |seen: &Seen| {
            seen.revision == speller.revision && seen.language == doc.language && seen.words == doc.dictionary
        };
        if !self.seen.as_ref().is_some_and(fresh) {
            let (language, words) = (doc.language.clone(), doc.dictionary.clone());
            self.seen = Some(Seen { language, words, revision: speller.revision });
            // The text is the same, so the old misspellings stay until each block is checked again.
            for block in doc.walk().filter(|block| !block.body.runs().is_empty()) {
                if self.waiting.insert(block.id.clone()) {
                    self.queue.push_back(block.id.clone());
                }
            }
        }
        while let Some(id) = self.queue.pop_front() {
            if !self.waiting.remove(&id) {
                continue;
            }
            // A block that was removed, or holds no text, has nothing to show.
            let Some((block, language)) = find(&doc.blocks, &id, &doc.language) else { continue };
            let marks = misspellings(speller, doc, block, language);
            let changed = self.marks(&id) != marks;
            if marks.is_empty() {
                self.marks.remove(&id);
            } else {
                self.marks.insert(id.clone(), marks);
            }
            return Some(Checked { block: id, changed });
        }
        None
    }

    /// The misspelled words of a block as byte ranges of its text, in order. Empty for a block that has
    /// none, or that was edited and not yet checked again.
    pub fn marks(&self, block: &BlockId) -> &[Range<usize>] {
        self.marks.get(block).map_or(&[], Vec::as_slice)
    }
}

/// The BCP 47 tag of the text at a byte offset in a block: that of its run, else of the nearest block around
/// it that names one, else the document's. `None` when the document has no such block.
pub fn language_at<'a>(doc: &'a Document, block: &BlockId, offset: usize) -> Option<&'a str> {
    let (block, language) = find(&doc.blocks, block, &doc.language)?;
    let mut end = 0;
    let run = block.body.runs().iter().find(|run| {
        end += run.text.len();
        offset < end
    });
    Some(run.and_then(|run| lang(&run.extra)).unwrap_or(language))
}

/// A block that holds text, with the language its text is in unless a run says otherwise.
fn find<'a>(blocks: &'a [Block], id: &BlockId, language: &'a str) -> Option<(&'a Block, &'a str)> {
    blocks.iter().find_map(|block| {
        let language = lang(&block.extra).unwrap_or(language);
        if &block.id == id {
            return (!block.body.runs().is_empty()).then_some((block, language));
        }
        find(block.body.blocks(), id, language)
    })
}

/// The `lang` a block or run carries for a passage in another language.
// The model has no field for it yet, so it is read from the fields kept as written.
fn lang(extra: &letters_model::Extra) -> Option<&str> {
    extra.get("lang")?.as_str()
}

fn misspellings(speller: &Speller, doc: &Document, block: &Block, language: &str) -> Vec<Range<usize>> {
    // Code is not prose, and what a type from a newer build holds is unknown.
    if matches!(block.kind, BlockKind::Code { .. } | BlockKind::Unknown { .. }) {
        return Vec::new();
    }
    // `None` for code, which also parts the words on either side of it.
    fn spoken<'a>(run: &'a Run, language: &'a str) -> Option<&'a str> {
        (!run.marks.contains(&Mark::Code)).then_some(lang(&run.extra).unwrap_or(language))
    }
    let mut out = Vec::new();
    let mut start = 0;
    // Runs in one language are checked as one text, since a word can cross from one run into the next.
    for stretch in block.body.runs().chunk_by(|a, b| spoken(a, language) == spoken(b, language)) {
        let text: String = stretch.iter().map(|run| run.text.as_str()).collect();
        if let Some(checker) =
            spoken(&stretch[0], language).and_then(|language| speller.checker(language, &doc.dictionary))
        {
            out.extend(checker.misspellings(&text).into_iter().map(|range| start + range.start..start + range.end));
        }
        start += text.len();
    }
    out
}
