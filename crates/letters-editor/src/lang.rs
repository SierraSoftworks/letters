//! The language of a passage: what a block or a stretch of its text says it is written in.
//!
//! A block reads in the language it names, else in that of the nearest block around it that names one,
//! else in the document's. A stretch of text that names its own reads in that.

use std::ops::Range;

use letters_model::{Block, Op, Pos};

use crate::{Editor, text, tree};

/// The language that the text at the caret or under the selection says it is written in. See
/// [`Editor::lang`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lang {
    /// It names none, and neither does a block around it, so it reads in the document's.
    Document,
    /// The BCP 47 tag that all of it reads in: its own, its block's, or that of a block around it.
    Tag(String),
    /// A selection that reads in more than one.
    Mixed,
}

impl Editor {
    /// The language of the text at the caret, which is what text typed there takes, or of the text
    /// under the selection. Code and formulas have none to speak of and are passed over.
    pub fn lang(&self) -> Lang {
        let caret = self.caret.as_ref().filter(|caret| caret.selection.is_caret());
        let mut said: Vec<Option<&str>> = Vec::new();
        for (block, range) in self.spoken() {
            let around = self.lang_around(block);
            let runs = block.body.runs();
            let tags: Vec<Option<&str>> = match caret.map(|caret| caret.selection.head.offset) {
                // Typed text takes after the character before it, and at the start after the first.
                Some(at) => vec![text::run_at(runs, at.saturating_sub(1)).and_then(|run| run.lang.as_deref())],
                None => text::runs_in(runs, range).map(|run| run.lang.as_deref()).collect(),
            };
            for tag in tags.into_iter().map(|tag| tag.or(around)) {
                if !said.contains(&tag) {
                    said.push(tag);
                }
            }
        }
        match said[..] {
            [] | [None] => Lang::Document,
            [Some(tag)] => Lang::Tag(tag.to_owned()),
            _ => Lang::Mixed,
        }
    }

    /// The language a block reads in before any of its text says otherwise: its own, or that of the
    /// nearest block around it that names one. `None` leaves it to the document's.
    fn lang_around<'a>(&'a self, block: &'a Block) -> Option<&'a str> {
        let mut at = block;
        loop {
            if let Some(lang) = at.lang.as_deref() {
                return Some(lang);
            }
            at = self.seat(&at.id)?.parent?;
        }
    }

    /// The text within reach that has a language to speak of: each block of prose the selection
    /// touches, with the stretch of it that is selected. A caret reaches the whole of its block.
    fn spoken(&self) -> Vec<(&Block, Range<usize>)> {
        let Some(caret) = self.caret.as_ref().filter(|caret| !caret.picture) else { return Vec::new() };
        let (start, end) = self.ordered(&caret.selection);
        let texts = tree::texts(self.doc());
        let index = |pos: &Pos| texts.iter().position(|block| block.id == pos.block);
        let (Some(from), Some(to)) = (index(&start), index(&end)) else { return Vec::new() };
        let whole = caret.selection.is_caret();
        let piece = |i: usize| {
            let (block, len) = (texts[i], text::len(texts[i]));
            let selected = if i == from { start.offset } else { 0 }..if i == to { end.offset } else { len };
            (block, if whole { 0..len } else { selected })
        };
        // A picture in a gallery has no caption of its own, and so no text.
        let prose = |block: &Block| !text::is_source(block) && !self.member(&block.id);
        (from..=to).map(piece).filter(|(block, range)| prose(block) && (whole || !range.is_empty())).collect()
    }

    /// Whether there is text within reach whose language can be said.
    pub(crate) fn takes_lang(&self) -> bool {
        !self.spoken().is_empty()
    }

    /// Says what language the text within reach is written in, or with `None` leaves it to what is
    /// around it. A block that is reached whole takes it as its own, and none of its text keeps another.
    /// A block that is reached in part takes it for the selected text.
    pub(crate) fn set_lang(&mut self, lang: Option<String>) {
        let lang = lang.map(|lang| lang.trim().to_owned()).filter(|lang| !lang.is_empty());
        let mut ops = Vec::new();
        for (block, range) in self.spoken() {
            let (id, all) = (block.id.clone(), 0..text::len(block));
            if range != all {
                ops.push(Op::SetLang { block: id, range, lang: lang.clone() });
                continue;
            }
            if block.body.runs().iter().any(|run| run.lang.is_some()) {
                ops.push(Op::SetLang { block: id.clone(), range: all, lang: None });
            }
            // The op sets all a block may carry, so the page break goes back as it is.
            ops.push(Op::SetBlockProps { block: id, page_break: block.page_break, lang: lang.clone() });
        }
        self.apply(ops);
    }
}
