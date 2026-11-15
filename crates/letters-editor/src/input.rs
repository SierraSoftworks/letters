//! Text input: typing, autocorrect, soft-keyboard deletes and IME composition.

use std::ops::Range;

use letters_model::{Block, BlockId, BlockKind, Op, Pos, Selection};

use crate::{Composing, Editor, InputEvent, Kind, Platform, text};

impl Editor {
    pub(crate) fn handle(&mut self, event: InputEvent, platform: &mut dyn Platform) {
        let now = platform.now_ms();
        self.renames.clear();
        let rule = std::mem::take(&mut self.rule);
        self.goal = None;
        match event {
            InputEvent::SetComposition { text, selection } => self.compose(&text, selection, platform, now),
            InputEvent::Commit(text) => self.commit_composition(&text, platform, now),
            InputEvent::Insert(text) => {
                self.end_composition();
                self.typed(&text, platform, now);
            }
            InputEvent::ReplaceRange { range, text } => {
                self.end_composition();
                self.replace_range(range, &text, now);
            }
            InputEvent::DeleteSurrounding { before, after } => {
                self.end_composition();
                // A soft keyboard's backspace takes back an input rule just as the key does.
                if !(rule && before > 0 && after == 0 && self.undo_step()) {
                    self.delete_surrounding(before, after, now);
                }
            }
        }
    }

    /// Makes sure there is a caret to type at. A document with no text gets a paragraph.
    pub(crate) fn ensure_caret(&mut self, platform: &mut dyn Platform) -> bool {
        if self.caret.is_none() {
            let id = BlockId::new(platform.new_id());
            let block = Block::new(id.clone(), BlockKind::Paragraph);
            let index = self.doc().blocks.len();
            if self.apply(vec![Op::InsertBlock { parent: None, index, block }]) {
                self.place(Pos::new(id, 0));
            }
        }
        self.caret.is_some()
    }

    fn typed(&mut self, text: &str, platform: &mut dyn Platform, now: u64) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.mark();
        if text.is_empty() || !self.ensure_caret(platform) {
            return;
        }
        if self.picture().is_some() {
            // A picture takes no text. A line break typed on one is Enter, which starts its caption.
            if text == "\n" {
                self.edit_caption(platform);
                self.commit(Kind::Other, now, true);
            }
            return;
        }
        if self.type_marker(&text) {
            return self.commit(Kind::Other, now, true);
        }
        // A word undoes together with the space before it, so a space after text starts a new step.
        let fresh = text.starts_with(char::is_whitespace) && !self.after_space();
        self.insert_lines(&text, platform);
        self.commit(Kind::Typing, now, fresh);
        self.run_rules(platform, now);
    }

    /// Whether the character before the selection is white space. The start of a block counts as one.
    fn after_space(&self) -> bool {
        let Some(caret) = &self.caret else { return true };
        let (start, _) = self.ordered(&caret.selection);
        let text = self.block(&start.block).map(Block::plain_text).unwrap_or_default();
        text[..text::snap(&text, start.offset)].chars().next_back().is_none_or(char::is_whitespace)
    }

    /// Inserts text that may hold line breaks. Outside code each one starts a new block, as Enter does.
    pub(crate) fn insert_lines(&mut self, text: &str, platform: &mut dyn Platform) {
        let Some(head) = self.selection().map(|selection| selection.head.block.clone()) else { return };
        if self.block(&head).is_some_and(text::is_source) || !text.contains('\n') {
            return self.insert(text);
        }
        if self.part(&head) && text != "\n" {
            // A caption and a cell hold one stretch of text, so lines pasted into one run on.
            return self.insert(&text.replace('\n', " "));
        }
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                self.split(platform);
            }
            self.insert(line);
        }
    }

    fn run_rules(&mut self, platform: &mut dyn Platform, now: u64) {
        self.mark();
        if self.apply_rules(platform) {
            self.commit(Kind::Rule, now, true);
            self.rule = true;
        }
    }

    /// Shows the text being composed in the document, in place of what the composition showed before.
    fn compose(&mut self, text: &str, selection: Option<Range<usize>>, platform: &mut dyn Platform, now: u64) {
        if self.composing.is_none() {
            self.mark();
            if text.is_empty() || !self.ensure_caret(platform) || self.picture().is_some() {
                return;
            }
            self.redo.clear();
            if self.delete_selection() && (!self.ensure_caret(platform) || self.picture().is_some()) {
                // What was selected went with all the text there was to compose in.
                return self.commit(Kind::Other, now, true);
            }
            let Some(at) = self.selection().map(|selection| selection.head.offset) else { return };
            self.composing = Some(Composing { range: at..at, at: now });
        }
        let (Some(mut composing), Some(caret)) = (self.composing.take(), self.caret.clone()) else { return };
        if text.is_empty() {
            // The input method gave the composition up, so the document goes back to how it was.
            return self.rollback();
        }
        let block = caret.selection.head.block.clone();
        let at = |offset: usize| Pos::new(block.clone(), offset);
        let start = composing.range.start;
        let composed = Selection { anchor: at(start), head: at(composing.range.end) };
        self.caret = Some(caret.clone().at(composed));
        self.insert(text);

        let inside = selection.unwrap_or(text.len()..text.len());
        let (anchor, head) = (start + text::snap(text, inside.start), start + text::snap(text, inside.end));
        self.caret = Some(caret.at(Selection { anchor: at(anchor), head: at(head) }));
        (composing.range, composing.at) = (start..start + text.len(), now);
        self.composing = Some(composing);
    }

    /// Ends the composition with `text`. With none in progress, the text is simply typed.
    fn commit_composition(&mut self, text: &str, platform: &mut dyn Platform, now: u64) {
        let (Some(composing), Some(caret)) = (self.composing.take(), self.caret.clone()) else {
            return self.typed(text, platform, now);
        };
        if text.is_empty() {
            return self.rollback();
        }
        let block = caret.selection.head.block.clone();
        let at = |offset: usize| Pos::new(block.clone(), offset);
        let composed = Selection { anchor: at(composing.range.start), head: at(composing.range.end) };
        self.caret = Some(caret.at(composed));
        self.insert(text);
        self.commit(Kind::Other, now, true);
        self.run_rules(platform, now);
    }

    /// Replaces a range of the caret's block, keeping the formatting of the text it replaces.
    fn replace_range(&mut self, range: Range<usize>, replacement: &str, now: u64) {
        self.mark();
        let Some(caret) = self.caret.clone().filter(|caret| !caret.picture) else { return };
        let head = caret.selection.head.clone();
        let Some(block) = self.block(&head.block) else { return };
        let text = block.plain_text();
        let range = text::snap(&text, range.start)..text::snap(&text, range.end.max(range.start));
        if range.is_empty() {
            // Nothing is replaced, so this is typing at that place.
            self.place(Pos::new(head.block.clone(), range.start));
            self.insert(replacement);
        } else {
            let Some(first) = text::run_at(block.body.runs(), range.start) else { return };
            let (marks, link) = (first.marks.clone(), first.link.clone());
            self.replace(&head.block, range.clone(), replacement, &marks, &link);
        }
        // The caret keeps its place in the text around the replacement.
        let offset = match head.offset {
            offset if offset >= range.end => offset - range.len() + replacement.len(),
            offset if offset > range.start => range.start + replacement.len(),
            offset => offset,
        };
        self.place_with(Pos::new(head.block, offset), caret.marks);
        self.commit(Kind::Other, now, true);
    }

    fn delete_surrounding(&mut self, before: usize, after: usize, now: u64) {
        self.mark();
        let Some(caret) = self.caret.clone() else { return };
        if caret.picture {
            // A soft keyboard's backspace takes a selected picture away as the key does.
            if before > 0 {
                self.remove_picture(&caret.selection.head.block);
                self.commit(Kind::Other, now, true);
            }
            return;
        }
        let (start, end) = self.ordered(&caret.selection);
        let Some(block) = self.block(&start.block).filter(|_| start.block == end.block) else { return };
        let text = block.plain_text();
        let from = text::snap(&text, start.offset.saturating_sub(before));
        let to = text::snap_up(&text, end.offset.saturating_add(after));
        let delete = |range: Range<usize>| Op::DeleteText { block: start.block.clone(), range };
        // The later range goes first, so the earlier one's offsets still hold.
        let ops = [end.offset..to, from..start.offset].into_iter().filter(|range| !range.is_empty()).map(delete);
        self.apply(ops.collect());
        let moved = |pos: &Pos| Pos::new(pos.block.clone(), pos.offset - (start.offset - from));
        let selection = Selection { anchor: moved(&caret.selection.anchor), head: moved(&caret.selection.head) };
        self.caret = Some(caret.at(selection));
        self.commit(Kind::Deleting, now, false);
    }
}
