//! Spelling in the editor's window: which words the page marks as misspelled, and the menu about one.
//!
//! The document is checked a few blocks at a time while nothing else happens. What an edit touched is checked
//! before the next frame.

use std::collections::BTreeSet;
use std::ops::Range;

use letters_layout::DocLayout;
use letters_model::{BlockId, Change, Document, Pos};
use letters_spell::{Dictionary, Proofing, Speller};

/// The most blocks of an edit that are checked before the next frame. The rest wait for idle time.
const EAGER: usize = 16;
/// The blocks checked in one step of idle time: few enough to fit between two frames.
const BATCH: usize = 16;
/// The most corrections the menu offers.
const SUGGESTIONS: usize = 5;

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
    /// The person's own words, as the shell gave them and as they were added here. The document's own are
    /// in the document.
    mine: BTreeSet<String>,
    /// Whether the shell has said them, and so keeps them.
    said: bool,
    /// Whether every block has been checked since anything changed.
    done: bool,
    /// A misspelled word that is not marked, because the caret has not left it since it was typed.
    withheld: Option<(BlockId, Range<usize>)>,
    /// Whether no word is marked, as for someone who cannot mend one.
    quiet: bool,
    pub fix: Option<Fix>,
}

impl Spelling {
    /// Replaces the person's own words.
    pub fn set_user_words(&mut self, words: impl IntoIterator<Item = impl AsRef<str>>) {
        let words = words.into_iter().map(|word| word.as_ref().trim().to_owned());
        // A dictionary file may hold empty lines, which are no words.
        self.mine = words.filter(|word| !word.is_empty()).collect();
        self.said = true;
        self.reword();
    }

    /// The person's own words, once the shell has said them: until then nothing keeps a list to show.
    pub fn mine(&self) -> Option<&BTreeSet<String>> {
        self.said.then_some(&self.mine)
    }

    /// Stops marking misspelled words, or marks them again. Words are marked for someone who can mend
    /// them: to anyone else the marks are noise on what they read.
    pub fn hush(&mut self, quiet: bool, doc: &Document, layout: &mut DocLayout) {
        if self.quiet == quiet {
            return;
        }
        (self.quiet, self.fix, self.withheld) = (quiet, None, None);
        for block in doc.walk() {
            let marks = if quiet { Vec::new() } else { self.proofing.marks(&block.id).to_vec() };
            layout.set_spelling(&block.id, marks);
        }
    }

    pub fn add_dictionary(&mut self, language: &str, dictionary: Dictionary) {
        self.speller.add_dictionary(language, dictionary);
        self.done = false;
    }

    fn reword(&mut self) {
        self.speller.set_user_words(&self.mine);
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

    /// Whether every block has been checked since anything changed.
    pub fn done(&self) -> bool {
        self.done
    }

    /// Checks a little more of the document, the blocks `in_view` before the rest. Returns whether the marks
    /// of any of those changed.
    pub fn idle(&mut self, doc: &Document, caret: Option<&Pos>, in_view: &[BlockId], layout: &mut DocLayout) -> bool {
        self.proofing.check_first(in_view);
        let changed = self.check(BATCH, doc, caret, false, layout);
        changed.iter().any(|block| in_view.contains(block))
    }

    /// Checks the next `blocks` blocks that wait for it. Returns those whose marks changed.
    fn check(
        &mut self,
        blocks: usize,
        doc: &Document,
        caret: Option<&Pos>,
        typing: bool,
        layout: &mut DocLayout,
    ) -> Vec<BlockId> {
        let mut changed = Vec::new();
        for _ in 0..blocks {
            let Some(checked) = self.proofing.step(&self.speller, doc) else {
                self.done = true;
                break;
            };
            if checked.changed {
                changed.push(checked.block.clone());
            }
            let mut marks = self.proofing.marks(&checked.block).to_vec();
            // A word that was held back stays so through a check that no typing asked for.
            let held = self.withheld.take_if(|(block, _)| *block == checked.block).is_some();
            let caret = caret.filter(|caret| (typing || held) && caret.block == checked.block);
            let touched = caret.and_then(|caret| marks.iter().position(|mark| touches(mark, caret.offset)));
            if let Some(touched) = touched {
                self.withheld = Some((checked.block.clone(), marks.remove(touched)));
            }
            // Set even when the block has none, since the page carries an edited block's old marks along.
            layout.set_spelling(&checked.block, if self.quiet { Vec::new() } else { marks });
        }
        changed
    }

    /// Marks the word that was being typed, once the caret has left it. `caret` is where a caret now stands.
    pub fn moved(&mut self, caret: Option<&Pos>, layout: &mut DocLayout) {
        let Some((block, word)) = self.withheld.as_ref().filter(|_| !self.quiet) else { return };
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

    /// The misspelled word after `from`, or the one before it, going on round the document's ends. What
    /// still waits to be checked is checked first, so that the answer is about the whole document.
    pub fn next(
        &mut self,
        doc: &Document,
        from: Option<&Pos>,
        back: bool,
        layout: &mut DocLayout,
    ) -> Option<(BlockId, Range<usize>)> {
        while !self.done {
            self.check(BATCH, doc, None, false, layout);
        }
        // Every block in the order it is read, and the words marked in each.
        fn read<'a>(blocks: &'a [letters_model::Block], order: &mut Vec<&'a BlockId>) {
            for block in blocks {
                order.push(&block.id);
                read(block.body.blocks(), order);
            }
        }
        let mut order = Vec::new();
        read(&doc.blocks, &mut order);
        let words = order
            .iter()
            .enumerate()
            .flat_map(|(at, block)| self.proofing.marks(block).iter().map(move |word| (at, *block, word.clone())));
        let words: Vec<_> = words.collect();
        let here = from.and_then(|from| Some((order.iter().position(|block| **block == from.block)?, from.offset)));
        let found = match (here, back) {
            (None, false) => words.first(),
            (None, true) => words.last(),
            (Some(here), false) => words.iter().find(|(at, _, word)| (*at, word.start) > here).or(words.first()),
            (Some(here), true) => words.iter().rev().find(|(at, _, word)| (*at, word.end) < here).or(words.last()),
        };
        found.map(|(_, block, word)| ((*block).clone(), word.clone()))
    }

    /// Accepts a word from now on as one of the person's own.
    pub fn learn(&mut self, word: &str) {
        self.mine.insert(word.to_owned());
        self.reword();
    }

    /// Takes a word back out of the person's own.
    pub fn forget(&mut self, word: &str) {
        self.mine.remove(word);
        self.reword();
    }

    /// Checks again after the words that are right changed, the person's or the document's: `block` at
    /// once, and the rest of the document in idle time.
    pub fn reworded(&mut self, block: Option<BlockId>, doc: &Document, layout: &mut DocLayout) {
        self.fix = None;
        self.done = false;
        if let Some(block) = block {
            self.proofing.invalidate(&Change { blocks: BTreeSet::from([block]), ..Change::default() });
            self.check(1, doc, None, false, layout);
        }
    }
}

/// Whether a caret at `offset` is in a word or at either end of it.
fn touches(word: &Range<usize>, offset: usize) -> bool {
    word.start <= offset && offset <= word.end
}
