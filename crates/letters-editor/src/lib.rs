//! The editor: turns input into edits of a [`Model`].
//!
//! - [`InputEvent`] is the one stream of text input every platform funnels into.
//! - [`KeyEvent`] carries key presses, which the [`Keymap`] resolves to [`Command`]s.
//! - [`Editor`] owns the model and the selection, and is driven with nothing else.
//!
//! The editor never reads a clock or a clipboard itself. The shell injects a [`Platform`].

mod clipboard;
mod edit;
mod input;
mod keymap;
mod markers;
mod merge;
mod motion;
mod rules;
mod text;
mod tree;

use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{
    Block, BlockId, BlockKind, Change, Document, ListKind, Mark, Model, Op, OpError, Page, Pos, ReplicaId, Selection,
    Style, TextStyle,
};

pub use keymap::Keymap;
/// The markers [`Editor::markers`] returns. They live in the model so that layout can draw them.
pub use letters_model::{Marker, Markers};
use merge::Kept;
pub use motion::Lines;

/// Text input, as every platform delivers it. Offsets are UTF-8 bytes relative to the caret's block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    /// Replaces the selection with text.
    Insert(String),
    /// Replaces a range of the caret's block, as autocorrect does.
    ReplaceRange { range: Range<usize>, text: String },
    /// Shows text an IME is still composing, replacing any earlier composition. `selection` is the caret or
    /// the selected part inside `text`, in bytes of `text`. Empty text gives the composition up.
    SetComposition { text: String, selection: Option<Range<usize>> },
    /// Ends a composition with its final text.
    Commit(String),
    /// Deletes bytes around the caret, as a soft keyboard's backspace does.
    DeleteSurrounding { before: usize, after: usize },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Ctrl on Linux and Windows, Cmd on macOS.
    pub primary: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A printable key by the character it produces unshifted, in lower case.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Modifiers,
}

/// Where a caret can be sent. Left and Right stop at each character and each revealed Markdown marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    /// The edges of the visual line, which needs [`Lines`]. Without one, a line ends at a line break.
    LineStart,
    LineEnd,
    Up,
    Down,
    PageUp,
    PageDown,
    /// The start of the block, or of the block before when already there.
    BlockStart,
    BlockEnd,
    DocStart,
    DocEnd,
}

/// Everything the editor can be asked to do. Keys, menus and the command palette all go through here.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Moves the caret, or only the head of the selection when `extend` is set.
    ///
    /// An arrow with no text left to go to makes an empty line outside a code block, a quote or a callout,
    /// or past a block that holds no text, and that is an edit.
    Move {
        to: Motion,
        extend: bool,
    },
    /// Deletes the selection, or from the caret to where the motion leads.
    Delete(Motion),
    /// Puts the caret below everything, as a click under the last block asks. It goes to the end of the
    /// body text, heading or list item the document ends in. After anything else an empty line is made for it.
    LineBelow,
    /// Breaks the block at the caret, as Enter does.
    Split,
    /// A line break that stays inside the block.
    LineBreak,
    /// Nests list items one level deeper, or steps a line of code in.
    Indent,
    Outdent,
    /// Flips a mark on the selection, or on the text typed next at a caret.
    ToggleMark(Mark),
    /// Links the selection, or changes the link under the caret. `None` removes the link.
    SetLink(Option<String>),
    /// Turns the selected blocks into `kind`, or back into paragraphs when they already are.
    /// A quote or callout wraps them, and a divider is inserted after them.
    SetBlock(BlockKind),
    /// Ticks or unticks the task under the caret.
    ToggleChecked,
    Undo,
    Redo,
    SelectAll,
    /// Selects the word under the caret, as a double click does.
    SelectWord,
    /// Selects the block under the caret, as a triple click does.
    SelectBlock,
    Copy,
    Cut,
    Paste,
    /// Pastes the clipboard's text as it is, with no formatting.
    PastePlain,
    /// Sets the style of one element type, such as `heading1`, which every block of that type takes.
    /// A style the file format would refuse changes nothing.
    SetStyle {
        element: String,
        style: TextStyle,
    },
    /// Sets the paper size and margins.
    SetPage(Page),
    /// Replaces the whole style, as applying a stylesheet does.
    ReplaceStyle(Style),
    /// Sets the language the document is written in.
    SetLanguage(String),
    /// Adds a word to the document's own dictionary. [`Platform::learn_word`] keeps one for the person.
    AddWord(String),
    RemoveWord(String),
}

/// What the shell lends the editor.
pub trait Platform {
    /// Milliseconds since an arbitrary start. Only differences are meaningful.
    fn now_ms(&self) -> u64;
    fn clipboard_read(&mut self) -> Option<Clip>;
    fn clipboard_write(&mut self, clip: Clip);
    /// A new identifier, unique across devices.
    fn new_id(&mut self) -> String;
    /// The laid-out lines, for movement between them. Without them a line runs to the next line break.
    fn lines(&self) -> Option<&dyn Lines> {
        None
    }
    /// Keeps the bytes of an asset that the document now names. The name comes from the bytes, so being
    /// handed one twice changes nothing.
    fn store_asset(&mut self, _name: &str, _bytes: &[u8]) {}
    /// Keeps a word in the person's own dictionary, which every document is checked against.
    fn learn_word(&mut self, _word: &str) {}
}

/// A block id made of 80 random bits: 16 characters, short enough to keep `document.json` readable. The
/// shell brings the bits, as the core has no randomness of its own, and answers [`Platform::new_id`] with it.
pub fn block_id(random: [u8; 10]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let bits = random.iter().fold(0u128, |bits, byte| bits << 8 | u128::from(*byte));
    (0..16).map(|i| ALPHABET[(bits >> (5 * i) & 31) as usize] as char).collect()
}

/// Clipboard content: plain text (Markdown when Letters wrote it) and, where offered, HTML and images.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Clip {
    pub text: String,
    pub html: Option<String>,
    pub pictures: Vec<Picture>,
}

/// An image on its way into the document, as the shell hands it over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub bytes: Vec<u8>,
    /// Its media type, such as `image/png`.
    pub mime: String,
    /// Its size in pixels.
    pub width: u32,
    pub height: u32,
}

/// A selection and the marks that text typed at it will take.
#[derive(Clone, Debug, PartialEq)]
struct Caret {
    selection: Selection,
    marks: BTreeSet<Mark>,
}

/// One undo step: a run of model edits that are undone together, and the carets either side of them.
#[derive(Clone, Debug)]
struct Step {
    count: usize,
    before: Kept,
    after: Kept,
    kind: Kind,
    at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Typing and deleting each grow one step until the writer pauses or moves.
    Typing,
    Deleting,
    /// A Markdown input rule, kept apart so that undoing it leaves the typed characters.
    Rule,
    Other,
}

/// A composition in progress in the caret's block.
#[derive(Clone, Debug)]
struct Composing {
    range: Range<usize>,
    at: u64,
}

/// A pause this long, in milliseconds, starts a new undo step.
const PAUSE_MS: u64 = 1000;

/// A document being edited.
#[derive(Debug)]
pub struct Editor {
    model: Model,
    caret: Option<Caret>,
    keymap: Keymap,
    undo: Vec<Step>,
    redo: Vec<Step>,
    /// Model edits made since the last undo step was closed.
    steps: usize,
    /// What the call in progress has changed so far.
    change: Change,
    /// Whether the newest undo step is an input rule, which Backspace then takes back.
    rule: bool,
    composing: Option<Composing>,
    /// The horizontal position that consecutive vertical moves keep to.
    goal: Option<f32>,
    /// How many times other writers' edits have been taken in.
    merges: u64,
    /// The caret before the edits that are not yet an undo step.
    origin: Kept,
}

impl Editor {
    /// Starts editing a document that has no history. `replica` names this session: see [`ReplicaId`].
    pub fn new(doc: Document, replica: ReplicaId) -> Self {
        Self::with_model(Model::new(doc, replica))
    }

    /// Starts editing a document that comes with its history.
    pub fn with_model(model: Model) -> Self {
        let first = tree::texts(model.doc()).first().map(|block| Pos::new(block.id.clone(), 0));
        let mut editor = Self {
            model,
            caret: None,
            keymap: Keymap::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            steps: 0,
            change: Change::default(),
            rule: false,
            composing: None,
            goal: None,
            merges: 0,
            origin: Kept::default(),
        };
        if let Some(pos) = first {
            editor.place(pos);
        }
        editor
    }

    pub fn doc(&self) -> &Document {
        self.model.doc()
    }

    /// The model, for saving its history and comparing revisions. What changes it from outside comes in
    /// through [`Editor::merge`], [`Editor::rewrite`] and [`Editor::attach`].
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// `None` when the document has nowhere to put a caret.
    pub fn selection(&self) -> Option<&Selection> {
        self.caret.as_ref().map(|caret| &caret.selection)
    }

    /// Moves the selection, as a click or a drag does. Positions are brought onto the nearest character, and a
    /// selection that names no text block is ignored.
    pub fn select(&mut self, selection: Selection) {
        self.end_composition();
        (self.rule, self.goal) = (false, None);
        self.seal();
        if let (Some(anchor), Some(head)) = (self.clamp(&selection.anchor), self.clamp(&selection.head)) {
            self.set_selection(Selection { anchor, head });
        }
    }

    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
    }

    /// The marks in force: those typed text will take at a caret, or those every selected character has.
    pub fn marks(&self) -> BTreeSet<Mark> {
        let Some(caret) = &self.caret else { return BTreeSet::new() };
        if caret.selection.is_caret() {
            return caret.marks.clone();
        }
        let doc = self.doc();
        let pieces = self.pieces(&caret.selection);
        let mut runs = pieces.iter().flat_map(|(id, range)| {
            let runs = doc.block(id).map_or(&[][..], |block| block.body.runs());
            text::runs_in(runs, range.clone())
        });
        let first = runs.next().map(|run| run.marks.clone()).unwrap_or_default();
        runs.fold(first, |marks, run| marks.intersection(&run.marks).cloned().collect())
    }

    /// The address of the link under the caret, or at the start of the selection.
    pub fn link(&self) -> Option<&str> {
        let caret = self.caret.as_ref()?;
        let (start, _) = self.ordered(&caret.selection);
        let runs = self.doc().block(&start.block)?.body.runs();
        let after = text::run_at(runs, start.offset).and_then(|run| run.link.as_deref());
        let before =
            start.offset.checked_sub(1).and_then(|at| text::run_at(runs, at)).and_then(|run| run.link.as_deref());
        if caret.selection.is_caret() { before.or(after) } else { after }
    }

    /// The text an input method is composing: its block and where in it the text sits.
    pub fn composition(&self) -> Option<(&BlockId, Range<usize>)> {
        let composing = self.composing.as_ref()?;
        Some((&self.selection()?.head.block, composing.range.clone()))
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.composing.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Feeds text input. Returns what changed, for layout.
    pub fn input(&mut self, event: InputEvent, platform: &mut dyn Platform) -> Change {
        self.handle(event, platform);
        std::mem::take(&mut self.change)
    }

    /// Feeds a key press. Returns `None` when the key means nothing here, so the shell may use it.
    pub fn key(&mut self, event: KeyEvent, platform: &mut dyn Platform) -> Option<Change> {
        let command = self.keymap.command(&event)?.clone();
        self.exec(command, platform).then(|| std::mem::take(&mut self.change))
    }

    pub fn run(&mut self, command: Command, platform: &mut dyn Platform) -> Change {
        self.exec(command, platform);
        std::mem::take(&mut self.change)
    }

    /// Whether a command has anything to act on, so that a key it is bound to can fall through when not.
    fn applies(&self, command: &Command) -> bool {
        let Some(caret) = &self.caret else {
            return match command {
                Command::Undo => self.can_undo(),
                Command::Redo => self.can_redo(),
                Command::Paste | Command::PastePlain | Command::LineBelow => true,
                // The style and the document's own fields are there whether or not there is text.
                Command::SetStyle { .. } | Command::SetPage(_) | Command::ReplaceStyle(_) => true,
                Command::SetLanguage(_) | Command::AddWord(_) | Command::RemoveWord(_) => true,
                _ => false,
            };
        };
        let ranged = !caret.selection.is_caret();
        let kind = self.doc().block(&caret.selection.head.block).map(|block| &block.kind);
        match command {
            Command::Indent | Command::Outdent => {
                matches!(kind, Some(BlockKind::ListItem { .. } | BlockKind::Code { .. }))
            }
            Command::ToggleChecked => {
                matches!(kind, Some(BlockKind::ListItem { list: ListKind::Task { .. }, .. }))
            }
            Command::ToggleMark(_) => ranged || !matches!(kind, Some(BlockKind::Code { .. })),
            Command::SetLink(_) => ranged || self.link().is_some(),
            Command::Copy | Command::Cut => ranged,
            Command::Undo => self.can_undo(),
            Command::Redo => self.can_redo(),
            _ => true,
        }
    }

    fn exec(&mut self, command: Command, platform: &mut dyn Platform) -> bool {
        if !self.applies(&command) {
            return false;
        }
        let now = platform.now_ms();
        self.end_composition();
        let rule = std::mem::take(&mut self.rule);
        let goal = self.goal.take();
        self.mark();
        let mut kind = Kind::Other;
        match command {
            Command::Move { to, extend } => self.move_to(to, extend, goal, platform),
            // Straight after an input rule, Backspace gives back the characters that were typed.
            Command::Delete(Motion::Left) if rule && self.undo_step() => {}
            Command::Delete(motion) => {
                self.delete(motion, platform);
                kind = Kind::Deleting;
            }
            Command::LineBelow => self.line_below(platform),
            Command::Split => kind = self.split(platform),
            Command::LineBreak => self.insert("\n"),
            Command::Indent => self.indent(true),
            Command::Outdent => self.indent(false),
            Command::ToggleMark(mark) => self.toggle_mark(mark),
            Command::SetLink(link) => self.set_link(link),
            Command::SetBlock(block) => self.set_block(block, platform),
            Command::ToggleChecked => self.toggle_checked(),
            Command::Undo => _ = self.undo_step(),
            Command::Redo => self.redo_step(),
            Command::SelectAll => self.select_all(),
            Command::SelectWord => self.select_word(),
            Command::SelectBlock => self.select_block(),
            Command::Copy => self.copy(platform),
            Command::Cut => self.cut(platform),
            Command::Paste => self.paste(false, platform),
            Command::PastePlain => self.paste(true, platform),
            Command::SetStyle { element, style } => _ = self.apply(vec![Op::SetStyle { element, style }]),
            Command::SetPage(page) => _ = self.apply(vec![Op::SetPage { page }]),
            Command::ReplaceStyle(style) => _ = self.apply(vec![Op::ReplaceStyle { style }]),
            Command::SetLanguage(language) => _ = self.apply(vec![Op::SetLanguage { language }]),
            Command::AddWord(word) => _ = self.apply(vec![Op::AddWord { word }]),
            Command::RemoveWord(word) => _ = self.apply(vec![Op::RemoveWord { word }]),
        }
        self.commit(kind, now, false);
        true
    }

    /// Applies `ops` as one model edit. Returns whether the document changed.
    fn apply(&mut self, ops: Vec<Op>) -> bool {
        if ops.is_empty() {
            return false;
        }
        // The model records an undo step only for an edit that changed something, so the count kept here
        // must do the same. An edit that adds or removes blocks always does, and one to the style or the
        // document's own fields says so in its change. Any other is checked against the blocks it names.
        let structural = |op: &Op| {
            matches!(op, Op::Split { .. } | Op::Join { .. } | Op::InsertBlock { .. } | Op::RemoveBlock { .. })
        };
        let named = |op: &Op| match op {
            Op::InsertText { at, .. } => Some(at.block.clone()),
            Op::DeleteText { block, .. }
            | Op::SetMark { block, .. }
            | Op::SetLink { block, .. }
            | Op::SetKind { block, .. } => Some(block.clone()),
            _ => None,
        };
        let before: Option<Vec<Block>> = (!ops.iter().any(structural)).then(|| {
            let ids: BTreeSet<BlockId> = ops.iter().filter_map(named).collect();
            ids.iter().filter_map(|id| self.doc().block(id).cloned()).collect()
        });
        // The caret that undo comes back to is held while its text is still as it was. The model builds its
        // history for an edit in any case, so asking for positions here costs nothing more.
        if self.steps == 0 && self.origin.held.is_none() {
            self.origin = self.held(self.origin.clone());
        }
        let change = match self.model.apply(ops) {
            Ok(change) => change,
            // The style is the caller's, and the model is the judge of what one may hold.
            Err(OpError::BadStyle(_)) => return false,
            Err(error) => {
                debug_assert!(false, "the editor planned an edit the model refused: {error}");
                return false;
            }
        };
        let differs = |blocks: Vec<Block>| blocks.iter().any(|block| self.doc().block(&block.id) != Some(block));
        let changed = change.style || change.document || before.is_none_or(differs);
        if changed {
            self.steps += 1;
            self.note(change);
        }
        changed
    }

    fn note(&mut self, change: Change) {
        self.change.blocks.extend(change.blocks);
        self.change.structure |= change.structure;
        self.change.style |= change.style;
        self.change.document |= change.document;
    }

    /// Closes the edits made since the last call as one undo step, or adds them to the step before when
    /// they continue it. `fresh` keeps them apart.
    fn commit(&mut self, kind: Kind, now: u64, fresh: bool) {
        let before = self.origin.clone();
        let count = std::mem::take(&mut self.steps);
        if count == 0 {
            // Moving the caret ends the step, even when it comes back to where it was.
            if self.caret != before.caret {
                self.seal();
            }
            return;
        }
        self.redo.clear();
        let after = self.keep();
        let grows = !fresh && matches!(kind, Kind::Typing | Kind::Deleting);
        if let Some(last) = self.undo.last_mut()
            && grows
            && last.kind == kind
            && last.after.caret == before.caret
            && now.saturating_sub(last.at) <= PAUSE_MS
        {
            (last.count, last.after, last.at) = (last.count + count, after, now);
            return;
        }
        self.undo.push(Step { count, before, after, kind, at: now });
    }

    /// Stops the newest undo step from growing, so that what is typed next starts another.
    fn seal(&mut self) {
        if let Some(last) = self.undo.last_mut() {
            last.kind = Kind::Other;
        }
    }

    /// Takes back the edits made since the last undo step was closed, and puts the caret back.
    fn rollback(&mut self) {
        for _ in 0..std::mem::take(&mut self.steps) {
            if let Some(change) = self.model.undo() {
                self.note(change);
            }
        }
        let origin = self.origin.clone();
        self.recall(&origin);
    }

    fn undo_step(&mut self) -> bool {
        let Some(step) = self.undo.pop() else { return false };
        for _ in 0..step.count {
            if let Some(change) = self.model.undo() {
                self.note(change);
            }
        }
        self.recall(&step.before);
        self.redo.push(step);
        true
    }

    fn redo_step(&mut self) {
        let Some(step) = self.redo.pop() else { return };
        for _ in 0..step.count {
            if let Some(change) = self.model.redo() {
                self.note(change);
            }
        }
        self.recall(&step.after);
        self.undo.push(step);
    }

    /// Ends a composition where it stands, keeping its text.
    fn end_composition(&mut self) {
        if let Some(composing) = self.composing.take() {
            self.commit(Kind::Other, composing.at, true);
        }
    }

    fn block(&self, id: &BlockId) -> Option<&Block> {
        self.doc().block(id)
    }

    /// The position on a character boundary of a text block that is nearest to `pos`.
    fn clamp(&self, pos: &Pos) -> Option<Pos> {
        let block = self.block(&pos.block).filter(|block| tree::is_text(block))?;
        Some(Pos::new(pos.block.clone(), text::snap(&block.plain_text(), pos.offset)))
    }

    /// The ends of a selection in document order.
    fn ordered(&self, selection: &Selection) -> (Pos, Pos) {
        let (anchor, head) = (selection.anchor.clone(), selection.head.clone());
        let backwards = if anchor.block == head.block {
            head.offset < anchor.offset
        } else {
            let texts = tree::texts(self.doc());
            let index = |pos: &Pos| texts.iter().position(|block| block.id == pos.block);
            index(&head) < index(&anchor)
        };
        if backwards { (head, anchor) } else { (anchor, head) }
    }

    /// The selected part of each text block a selection touches, in document order.
    fn pieces(&self, selection: &Selection) -> Vec<(BlockId, Range<usize>)> {
        let (start, end) = self.ordered(selection);
        let texts = tree::texts(self.doc());
        let index = |pos: &Pos| texts.iter().position(|block| block.id == pos.block);
        let (Some(from), Some(to)) = (index(&start), index(&end)) else { return Vec::new() };
        let piece = |i: usize| {
            let from_offset = if i == from { start.offset } else { 0 };
            let to_offset = if i == to { end.offset } else { text::len(texts[i]) };
            (texts[i].id.clone(), from_offset..to_offset)
        };
        (from..=to).map(piece).filter(|(_, range)| !range.is_empty()).collect()
    }

    /// The marks text takes when nothing says otherwise: those of the character before. At the start of a
    /// block the caret sits before any opening marker, so only marks that show no marker carry over from the
    /// first character. Code blocks carry none.
    fn natural(&self, pos: &Pos) -> BTreeSet<Mark> {
        let Some(block) = self.block(&pos.block).filter(|block| !text::is_code(block)) else {
            return BTreeSet::new();
        };
        let runs = block.body.runs();
        if pos.offset > 0 {
            return text::marks_before(runs, pos.offset);
        }
        text::marks_after(runs, 0).into_iter().filter(|mark| !markers::PAIRED.contains(mark)).collect()
    }

    /// Selects with the marks that follow from the text: a range takes those of its first character.
    fn set_selection(&mut self, selection: Selection) {
        let marks = if selection.is_caret() {
            self.natural(&selection.head)
        } else {
            let (start, _) = self.ordered(&selection);
            let block = self.block(&start.block).filter(|block| !text::is_code(block));
            block.map(|block| text::marks_after(block.body.runs(), start.offset)).unwrap_or_default()
        };
        self.caret = Some(Caret { selection, marks });
    }

    fn place(&mut self, pos: Pos) {
        self.set_selection(Selection::caret(pos));
    }

    /// Puts the caret at `pos` with marks chosen by the caller.
    fn place_with(&mut self, pos: Pos, marks: BTreeSet<Mark>) {
        self.caret = Some(Caret { selection: Selection::caret(pos), marks });
    }
}
