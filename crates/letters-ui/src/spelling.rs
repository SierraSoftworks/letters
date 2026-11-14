//! Spelling in the editor's window: which words the page marks as misspelled, and the menu about one.
//!
//! The document is checked a few blocks at a time while nothing else happens. What an edit touched is checked
//! before the next frame.

use std::collections::BTreeSet;
use std::ops::Range;

use letters_editor::Platform;
use letters_layout::DocLayout;
use letters_model::{BlockId, Change, Document, Pos};
use letters_spell::{Dictionary, Proofing, Speller};

/// The most blocks of an edit that are checked before the next frame. The rest wait for idle time.
const EAGER: usize = 16;
/// The blocks checked in one step of idle time: few enough to fit between two frames.
const BATCH: usize = 16;
/// The most corrections the menu offers.
const SUGGESTIONS: usize = 5;

/// Adds words to a document's own dictionary.
///
/// The document model does not take a word as an operation yet. Until it does, the window keeps the words
/// added to a document in a list of its own: they are accepted at once, but are not saved with it.
pub trait Words {
    fn learn(&mut self, word: &str);
}

impl Words for BTreeSet<String> {
    fn learn(&mut self, word: &str) {
        self.insert(word.to_owned());
    }
}

/// A misspelled word with the menu about it open.
pub(crate) struct Fix {
    pub block: BlockId,
    pub range: Range<usize>,
    pub word: String,
    /// Corrections, best first.
    pub suggestions: Vec<String>,
}

#[derive(Default)]
pub(crate) struct Spelling {
    speller: Speller,
    proofing: Proofing,
    /// The person's own words, as the shell gave them and as they were added here.
    mine: BTreeSet<String>,
    /// The words added to this document. See [`Words`].
    own: BTreeSet<String>,
    /// Whether every block has been checked since anything changed.
    done: bool,
    /// A misspelled word that is not marked, because the caret has not left it since it was typed.
    withheld: Option<(BlockId, Range<usize>)>,
    pub fix: Option<Fix>,
}

impl Spelling {
    /// Replaces the person's own words.
    pub fn set_user_words(&mut self, words: impl IntoIterator<Item = impl AsRef<str>>) {
        self.mine = words.into_iter().map(|word| word.as_ref().trim().to_owned()).collect();
        self.reword();
    }

    pub fn add_dictionary(&mut self, language: &str, dictionary: Dictionary) {
        self.speller.add_dictionary(language, dictionary);
        self.done = false;
    }

    fn reword(&mut self) {
        self.speller.set_user_words(self.mine.iter().chain(&self.own));
        self.done = false;
    }

    /// Checks what an edit touched, so that the next frame shows it marked. `caret` is where a caret stands
    /// after it: a misspelled word it touches is still being typed, and is not marked until the caret leaves.
    pub fn edited(&mut self, change: &Change, doc: &Document, caret: Option<&Pos>, layout: &mut DocLayout) {
        self.fix = None;
        self.done = false;
        self.proofing.invalidate(change);
        self.check(change.blocks.len().min(EAGER), doc, caret, true, layout);
    }

    /// Checks a little more of the document. Returns whether any of it was left to check.
    pub fn idle(&mut self, doc: &Document, caret: Option<&Pos>, layout: &mut DocLayout) -> bool {
        if self.done {
            return false;
        }
        self.check(BATCH, doc, caret, false, layout);
        true
    }

    fn check(&mut self, blocks: usize, doc: &Document, caret: Option<&Pos>, typing: bool, layout: &mut DocLayout) {
        for _ in 0..blocks {
            let Some(checked) = self.proofing.step(&self.speller, doc) else {
                self.done = true;
                return;
            };
            let mut marks = self.proofing.marks(&checked.block).to_vec();
            // A word that was held back stays so through a check that no typing asked for.
            let held = self.withheld.take_if(|(block, _)| *block == checked.block).is_some();
            let caret = caret.filter(|caret| (typing || held) && caret.block == checked.block);
            let touched = caret.and_then(|caret| marks.iter().position(|mark| touches(mark, caret.offset)));
            if let Some(touched) = touched {
                self.withheld = Some((checked.block.clone(), marks.remove(touched)));
            }
            // Set even when the block has none, since the page carries an edited block's old marks along.
            layout.set_spelling(&checked.block, marks);
        }
    }

    /// Marks the word that was being typed, once the caret has left it. `caret` is where a caret now stands.
    pub fn moved(&mut self, caret: Option<&Pos>, layout: &mut DocLayout) {
        let Some((block, word)) = &self.withheld else { return };
        if caret.is_some_and(|caret| caret.block == *block && touches(word, caret.offset)) {
            return;
        }
        layout.set_spelling(block, self.proofing.marks(block).to_vec());
        self.withheld = None;
    }

    /// Opens the menu about the word at `pos`, if the page marks it as misspelled.
    pub fn ask(&mut self, doc: &Document, pos: &Pos, layout: &DocLayout) {
        let Some(range) = layout.spelling_at(pos) else { return };
        let text = doc.block(&pos.block).map(|block| block.plain_text());
        let Some(word) = text.and_then(|text| text.get(range.clone()).map(str::to_owned)) else { return };
        let language = letters_spell::language_at(doc, &pos.block, range.start);
        let checker = language.and_then(|language| self.speller.checker(language, &doc.dictionary));
        let mut suggestions = checker.map(|checker| checker.suggest(&word)).unwrap_or_default();
        suggestions.truncate(SUGGESTIONS);
        self.fix = Some(Fix { block: pos.block.clone(), range, word, suggestions });
    }

    /// Accepts the word the menu is about from now on, in the person's own dictionary or in this document's,
    /// and checks its block again at once. The rest of the document follows in idle time.
    pub fn learn(&mut self, mine: bool, doc: &Document, layout: &mut DocLayout, platform: &mut dyn Platform) {
        let Some(fix) = self.fix.take() else { return };
        if mine {
            platform.learn_word(&fix.word);
            self.mine.insert(fix.word);
        } else {
            self.own.learn(&fix.word);
        }
        self.reword();
        self.proofing.invalidate(&Change { blocks: BTreeSet::from([fix.block]), structure: false });
        self.check(1, doc, None, false, layout);
    }
}

/// Whether a caret at `offset` is in a word or at either end of it.
fn touches(word: &Range<usize>, offset: usize) -> bool {
    word.start <= offset && offset <= word.end
}
