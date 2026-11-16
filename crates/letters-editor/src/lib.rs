//! The editor: turns input into edits of a [`Model`].
//!
//! - [`InputEvent`] is the one stream of text input every platform funnels into.
//! - [`KeyEvent`] carries key presses, which the [`Keymap`] resolves to [`Command`]s.
//! - [`Editor`] owns the model and the selection, and is driven with nothing else.
//!
//! The editor never reads a clock or a clipboard itself. The shell injects a [`Platform`].

mod callout;
mod clipboard;
mod edit;
mod figures;
mod find;
mod input;
mod keymap;
mod lang;
mod links;
mod markers;
mod merge;
mod motion;
mod parts;
mod refs;
mod rules;
mod shared;
mod table;
mod text;
mod tree;
mod versions;
mod whole;

use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{
    Block, BlockId, BlockKind, Change, Document, ListKind, Mark, Model, Op, OpError, Page, Pos, ReplicaId, Revision,
    Run, Selection, Stood, Style, TextStyle,
};

pub use callout::CalloutEdit;
pub use figures::Spot;
pub use find::{BadQuery, Query, Seek};
pub use keymap::Keymap;
pub use lang::Lang;
/// The markers [`Editor::markers`] returns. They live in the model so that layout can draw them.
pub use letters_model::{Marker, Markers};
use merge::Kept;
pub use motion::{LinePos, Lines};
pub use refs::{Numbered, Referring};
pub use table::{Cell, TableEdit};
pub use whole::PreviewEdit;

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
    /// A function key, by its number from 1.
    F(u8),
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
    /// Breaks the block at the caret, as Enter does. Inside inline maths it leaves the formula instead.
    Split,
    /// Steps out of the formula the caret is in, as Esc does: to just after inline maths, or to the line
    /// below a formula block, which is made when there is none.
    LeaveFormula,
    /// A line break that stays inside the block.
    LineBreak,
    /// Nests list items one level deeper, or steps code in: at a caret, or every line a selection takes in.
    /// Other text has no level and is left as it is.
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
    /// Puts in a table of empty cells, its first row the header, and the caret in its first cell. It
    /// follows the block the selection ends in, or takes the place of an empty line.
    InsertTable {
        rows: usize,
        columns: usize,
    },
    /// Changes the table the caret is in. [`Editor::cell`] says where in it the caret is.
    Table(TableEdit),
    /// Changes the callout the caret is in, which [`Editor::callout`] tells of.
    Callout(CalloutEdit),
    /// Puts in a table of contents: a list of the document's headings down to Heading 3, made where it is
    /// shown by [`letters_model::Document::contents`]. It is a block that holds no text, placed as a
    /// divider is: below the caret's block, or in place of an empty line, with the caret in the text
    /// below it.
    InsertContents,
    /// Puts in a link preview: a card that leads to an address, placed as a divider is. It shows what
    /// the document keeps of the page, and when that is nothing the address alone, until
    /// [`PreviewEdit::Fetched`] says what the page holds. An address with nothing in it is not taken.
    ///
    /// The address of a web page makes one by itself where it is put on a line of its own: pasted on an
    /// empty line, or typed there and followed by Enter. Backspace after a card turns it back.
    InsertPreview(String),
    /// Sets how deep the headings go that a table of contents lists, from 1 to 4. The table is named,
    /// since no caret stands in it.
    SetContentsLevels {
        table: BlockId,
        levels: u8,
    },
    /// Changes a link preview, which is named since no caret stands in it. One undo step.
    Preview {
        block: BlockId,
        edit: PreviewEdit,
    },
    /// Shows the link that the caret's line holds alone as a link preview in the line's place, with what
    /// the document keeps of its page. The caret goes to the text below it, on a line made when there is
    /// none. The words the link was written with are not kept.
    LinkAsCard,
    /// Shows a link preview as the link it leads to: a line of text in its place that holds the address
    /// as a link, as Backspace after the card does. What the card showed of its page is kept for the
    /// link by its address, see [`Command::Fetched`]. The card is named, since no caret stands in it.
    CardAsLink(BlockId),
    /// Says what a fetch found of the page at an address, for all the document shows of it. Every link
    /// preview that leads there takes it as [`PreviewEdit::Fetched`] says. For a link in the text that
    /// leads there, the document keeps it by the address the link is written with, in
    /// [`Document::previews`], for the link to show without the network. It takes the place of what
    /// was shown: what was not found is shown no longer. Nothing is taken for an address that neither
    /// leads to by now, as when the link went while the fetch took its time. The picture's bytes go to
    /// [`Platform::store_asset`]. One undo step.
    Fetched {
        url: String,
        title: Option<String>,
        description: Option<String>,
        picture: Option<Picture>,
    },
    /// Puts pictures in: one as a figure, several as a gallery, and the last is left selected. With no
    /// spot they go below the block the selection ends in, in place of an empty line, or beside the
    /// selected picture of a gallery. The bytes go to [`Platform::store_asset`], and what is no picture
    /// Letters can show is left out.
    InsertFigures {
        pictures: Vec<Picture>,
        at: Option<Spot>,
    },
    /// Moves the caret's figure, as dragging it does: the one whose picture is selected, or in whose
    /// caption the caret is. A gallery it leaves with one picture becomes a figure.
    MoveFigure(Spot),
    /// Moves the selected picture one place along its gallery.
    ShiftFigure {
        right: bool,
    },
    /// Sets the size the caret's figure is shown at, in points.
    ResizeFigure {
        width: f32,
        height: f32,
    },
    /// Says what the caret's figure shows, for a reader who cannot see it.
    SetAlt(String),
    /// Turns the generated label before a caption, such as "Figure 2:", on or off: for the caret's
    /// figure, or for the gallery it is in.
    SetNumbered(bool),
    /// Puts the caret in the caption of the selected picture, as Enter does there. A gallery has one
    /// caption for all its pictures, which is made when there is none.
    EditCaption,
    /// Steps out of a caption, as Esc does: back to the picture it stands under.
    LeaveCaption,
    /// Keeps a linked picture in the document: the figure shows `picture` from the file's assets in place
    /// of the address it was loaded from, as wide as it was and as high as the picture's own shape makes
    /// that. The figure is named, since the bytes may arrive when the caret is elsewhere. Nothing happens
    /// when the figure is gone or already kept, or the bytes are no picture Letters shows.
    EmbedFigure {
        figure: BlockId,
        picture: Picture,
    },
    /// Puts a reference to a numbered figure or gallery at the caret, in place of the words typed for
    /// it, which [`Editor::referring`] tells of, or of the selection. It reads as the figure's label and
    /// follows the figure's number.
    TakeReference(BlockId),
    /// Leaves the words typed for a reference as words, as Esc does while a list of figures is open:
    /// [`Editor::referring`] is `None` until other words are typed for one.
    DismissReference,
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
    /// Puts `with` in place of what a query finds, which [`Editor::find`] tells of: at the one place
    /// `only` names, or at every place. One undo step.
    Replace {
        query: Query,
        with: String,
        only: Option<Selection>,
    },
    /// Sets the style of one element type, such as `heading1`, which every block of that type takes.
    /// A style the file format would refuse changes nothing.
    SetStyle {
        element: String,
        style: TextStyle,
    },
    /// Sets the paper size and margins.
    SetPage(Page),
    /// Makes a page of print start with a block, or no longer start there. Nothing of it shows while
    /// writing: Print view places these breaks and takes them away.
    SetPageBreak {
        block: BlockId,
        on: bool,
    },
    /// Makes a block read as another version of it, or with `None` takes it away: the choice of the
    /// version that a conflict held back. One undo step, wherever the caret is.
    UseVersion {
        block: BlockId,
        version: Option<Box<Block>>,
    },
    /// Makes each of some blocks read as another version of it, as [`Command::UseVersion`] does one: a
    /// passage that is restored from the history. One undo step for all of them.
    UseVersions(Vec<(BlockId, Option<Box<Block>>)>),
    /// Puts a block back that the document no longer holds, where it stood: the version of a conflict
    /// whose other side took the block away. Behind the block it stood behind if that is still beside
    /// where it stood, else first there. One undo step.
    PutBack {
        block: Box<Block>,
        stood: Stood,
    },
    /// Makes the document and its style read as they did at a revision of the history. Nothing of the
    /// history is taken back, and it is one undo step. See [`letters_model::Model::restore`].
    Restore(Revision),
    /// Replaces the whole style, as applying a stylesheet does.
    ReplaceStyle(Style),
    /// Sets the language the document is written in.
    SetLanguage(String),
    /// Says what language a passage is written in, as a BCP 47 tag, where it is another than the
    /// document's. `None` leaves it to what is around it again. [`Editor::lang`] tells the one in force.
    ///
    /// At a caret it is said of the block the caret is in. Of a selection it is said of the selected
    /// text, and of each block that is selected whole. A block it is said of takes it as its own, and
    /// none of its text keeps another. Code and formulas have no language and are passed over. One undo
    /// step.
    SetLang(Option<String>),
    /// Adds a word to the document's own dictionary. [`Platform::learn_word`] keeps one for the person.
    AddWord(String),
    RemoveWord(String),
}

impl Command {
    /// Whether the command only moves the caret, selects or copies, which is all that is left to do
    /// where nothing may be written.
    fn reads(&self) -> bool {
        let selects = matches!(self, Command::SelectAll | Command::SelectWord | Command::SelectBlock);
        selects || matches!(self, Command::Move { .. } | Command::LineBelow | Command::Copy)
    }
}

/// What the shell lends the editor.
pub trait Platform {
    /// Milliseconds since an arbitrary start. Only differences are meaningful.
    fn now_ms(&self) -> u64;
    /// The time of day, in seconds since 1970, which edits are dated with. `None` leaves them undated.
    fn wall_time(&self) -> Option<u64> {
        None
    }
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
    /// Whether the caret is on a figure's picture, and not in the caption its position names.
    picture: bool,
    /// Whether the head, where a line wraps there, is at the end of that line: see [`LinePos`]. It holds
    /// for as long as the selection is the same, and layout makes nothing of it where no line wraps.
    before_wrap: bool,
}

impl Caret {
    /// The caret with another selection.
    fn at(self, selection: Selection) -> Self {
        Self { before_wrap: self.before_wrap && self.selection == selection, selection, ..self }
    }
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
    /// Where the words typed for a reference start that Esc has said are only words.
    dismissed: Option<(BlockId, usize)>,
    /// The blocks that the command at hand put back under another id, see [`shared`].
    renames: shared::Renames,
    /// The addresses the writer has put in that want a fetch, until the shell asks for them.
    unfetched: Vec<String>,
    /// Whether the shell can fetch what a page says of itself.
    fetching: bool,
    /// Whether typing leaves blocks as they are, see [`Editor::set_inline_only`].
    inline_only: bool,
    /// Whether nothing may be written here, see [`Editor::set_read_only`].
    read_only: bool,
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
            dismissed: None,
            renames: Vec::new(),
            unfetched: Vec::new(),
            fetching: true,
            inline_only: false,
            read_only: false,
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

    /// Names who writes here. Edits made from here on carry the name, for every copy of the document.
    pub fn set_author(&mut self, name: &str) {
        self.model.set_author(name);
    }

    /// `None` when the document has nowhere to put a caret.
    pub fn selection(&self) -> Option<&Selection> {
        self.caret.as_ref().map(|caret| &caret.selection)
    }

    /// Moves the selection, as a click or a drag does. Positions are brought onto the nearest character, and
    /// one that names a block with no text, such as an empty table, onto the text nearest to it. A
    /// selection that names no block is ignored. In a figure the positions are in its caption:
    /// [`Editor::select_picture`] selects the picture, and a picture in a gallery is only ever that.
    pub fn select(&mut self, selection: Selection) {
        self.select_at(selection, false);
    }

    /// As [`Editor::select`], with what layout says of the head: whether it is at the end of a line that
    /// wraps with no space to stand before, as a press past the end of such a line is. See [`LinePos`].
    pub fn select_at(&mut self, selection: Selection, before_wrap: bool) {
        self.end_composition();
        (self.rule, self.goal) = (false, None);
        self.seal();
        if let (Some(anchor), Some(head)) = (self.nearest(&selection.anchor), self.nearest(&selection.head)) {
            self.set_selection(Selection { anchor, head });
            self.side(&LinePos { pos: selection.head, before_wrap });
        }
        self.forget();
    }

    /// Whether the caret is at the end of a line that wraps with no space to stand before. Its position is
    /// then also the start of the next line, which is where it is drawn unless layout is told. See
    /// [`LinePos`].
    pub fn before_wrap(&self) -> bool {
        self.caret.as_ref().is_some_and(|caret| caret.before_wrap)
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

    /// The formula the caret or the selection is in: its block and where in it the TeX source sits. A formula
    /// block is all source. A caret at either end of inline maths is in it only when text typed there
    /// would be, which [`Editor::marks`] also says.
    pub fn formula(&self) -> Option<(&BlockId, Range<usize>)> {
        let caret = self.caret.as_ref()?;
        let (start, end) = self.ordered(&caret.selection);
        let block = self.doc().block(&start.block).filter(|_| start.block == end.block)?;
        if block.kind == BlockKind::Formula {
            return Some((&block.id, 0..text::len(block)));
        }
        let runs = block.body.runs();
        let maths = |run: &Run| run.marks.contains(&Mark::Math);
        let inside = match caret.selection.is_caret() {
            true => caret.marks.contains(&Mark::Math),
            false => text::runs_in(runs, start.offset..end.offset).all(maths),
        };
        let before = text::marks_before(runs, start.offset).contains(&Mark::Math);
        let after = text::marks_after(runs, end.offset).contains(&Mark::Math);
        let from = if before { text::stretch_start(runs, start.offset, maths) } else { start.offset };
        let to = if after { text::stretch_end(runs, end.offset, maths) } else { end.offset };
        inside.then_some((&block.id, from..to))
    }

    /// The text an input method is composing: its block and where in it the text sits.
    pub fn composition(&self) -> Option<(&BlockId, Range<usize>)> {
        let composing = self.composing.as_ref()?;
        Some((&self.selection()?.head.block, composing.range.clone()))
    }

    /// The addresses of web pages that the writer has put in since the last call, as a link in the text
    /// or as a link preview, and that the document keeps nothing of yet: by typing or pasting an address,
    /// by [`Command::SetLink`], or with a card. The shell fetches each page, and [`Command::Fetched`]
    /// takes what it finds. What came with another writer's edits, an undo, a version or a file is
    /// never named, so that reading a document fetches nothing, and neither are the links inside
    /// pasted text.
    pub fn take_unfetched(&mut self) -> Vec<String> {
        std::mem::take(&mut self.unfetched)
    }

    /// Says whether the shell can fetch what a page says of itself, which it can until told otherwise.
    /// Where it cannot, the address of a web page put on a line of its own stays a link in a line of
    /// text, since a card would have nothing to show, and [`Editor::take_unfetched`] names nothing. A
    /// card that is asked for is made all the same.
    pub fn set_preview_fetching(&mut self, offered: bool) {
        self.fetching = offered;
        if !offered {
            self.unfetched.clear();
        }
    }

    /// Says that this editor writes a stretch of marked text, such as a comment, and not a page. It
    /// writes a page until told otherwise. In a stretch of text, typing never makes or changes a block:
    /// of the shorthand only what marks text counts (bold, italic, strike, code, a link in brackets, an
    /// address typed out), a dollar sign starts no maths, no address becomes a card or is named by
    /// [`Editor::take_unfetched`], and Enter breaks the line inside its block. Commands do what they
    /// say all the same, so the caller holds back those that have no place there.
    pub fn set_inline_only(&mut self, on: bool) {
        self.inline_only = on;
        if on {
            self.unfetched.clear();
        }
    }

    /// Says that whoever sits here may read the document and not write it, as a session that was let
    /// in to read or to comment. It may write until told otherwise, and this can change while the
    /// document is open.
    ///
    /// While it is on, nothing that comes in through [`Editor::input`], [`Editor::key`] or
    /// [`Editor::run`] changes the document, its style or its history: each returns as a call that had
    /// nothing to do. The caret still moves and selects, and the selection is still copied, so of the
    /// commands only [`Command::Move`], [`Command::LineBelow`], the three that select and
    /// [`Command::Copy`] apply, and none of them makes a line for the caret. There is nothing to undo
    /// or redo, no reference is in the making, [`Editor::take_unfetched`] names nothing, and no
    /// version is marked or named. No marker is shown by [`Editor::markers`], and none stops the caret. What other writers wrote still comes in through
    /// [`Editor::merge`], [`Editor::rewrite`] and [`Editor::attach`], since the page follows them.
    /// What was there to undo is there again once writing is allowed.
    pub fn set_read_only(&mut self, on: bool) {
        if on {
            // What is being composed stays as it stands, and nothing waits to be taken back or fetched.
            self.end_composition();
            self.seal();
            self.rule = false;
            self.unfetched.clear();
        }
        self.read_only = on;
    }

    pub fn can_undo(&self) -> bool {
        !self.read_only && (!self.undo.is_empty() || self.composing.is_some())
    }

    pub fn can_redo(&self) -> bool {
        !self.read_only && !self.redo.is_empty()
    }

    /// Feeds text input. Returns what changed, for layout.
    pub fn input(&mut self, event: InputEvent, platform: &mut dyn Platform) -> Change {
        if self.read_only {
            return Change::default();
        }
        self.date(platform);
        self.handle(event, platform);
        self.forget();
        std::mem::take(&mut self.change)
    }

    /// Feeds a key press. Returns `None` when the key means nothing here, so the shell may use it. Of the
    /// commands a key is bound to, the first that has something to act on runs.
    pub fn key(&mut self, event: KeyEvent, platform: &mut dyn Platform) -> Option<Change> {
        let commands: Vec<Command> = self.keymap.commands(&event).cloned().collect();
        if commands.is_empty() {
            return None;
        }
        self.date(platform);
        let ran = commands.into_iter().any(|command| self.exec(command, platform));
        ran.then(|| std::mem::take(&mut self.change))
    }

    pub fn run(&mut self, command: Command, platform: &mut dyn Platform) -> Change {
        self.date(platform);
        self.exec(command, platform);
        std::mem::take(&mut self.change)
    }

    /// Tells the model the time before it is edited, as it has no clock of its own.
    fn date(&mut self, platform: &dyn Platform) {
        if let Some(seconds) = platform.wall_time() {
            self.model.set_time(seconds);
        }
    }

    /// Whether a command has anything to act on, so that a key it is bound to can fall through when not, and
    /// a menu can show what is of no use here.
    pub fn applies(&self, command: &Command) -> bool {
        if self.read_only && !command.reads() {
            return false;
        }
        if let Command::EmbedFigure { figure, picture } = command {
            // The figure is named, so the caret has no say.
            return self.embeddable(figure, picture);
        }
        if let Command::SetPageBreak { block, on } = command {
            return self.doc().block(block).is_some_and(|block| block.page_break != *on);
        }
        // A block that holds no text is named, and one goes in whether or not there is a caret.
        match command {
            Command::InsertContents => return true,
            Command::InsertPreview(url) => return whole::address(url).is_some(),
            Command::SetContentsLevels { table, levels } => return self.takes_levels(table, *levels),
            Command::Preview { block, edit } => return self.takes_preview(block, edit),
            Command::CardAsLink(card) => {
                return self.block(card).is_some_and(|card| matches!(card.kind, BlockKind::LinkPreview { .. }));
            }
            Command::Fetched { url, title, description, picture } => {
                return self.takes_fetched(url, title, description, picture.as_ref());
            }
            _ => {}
        }
        if let Command::UseVersion { block, version } = command {
            return self.takes_version(block, version.as_deref());
        }
        if let Command::UseVersions(blocks) = command {
            return blocks.iter().any(|(block, version)| self.takes_version(block, version.as_deref()));
        }
        if let Command::PutBack { block, stood } = command {
            return self.takes_back(block, stood).is_some();
        }
        if let Command::Replace { query, only, .. } = command {
            // What is found is replaced wherever the caret is.
            return self.replaceable(query, only.as_ref());
        }
        if let Command::Restore(revision) = command {
            // The document has no caret to ask, and a restore of what is there has nothing to do.
            return *revision != self.model.revision();
        }
        let Some(caret) = &self.caret else {
            return match command {
                Command::Undo => self.can_undo(),
                Command::Redo => self.can_redo(),
                Command::Paste | Command::PastePlain | Command::LineBelow => true,
                Command::InsertTable { .. } | Command::InsertFigures { .. } => true,
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
                // Lists nest and code steps in. No other text has a level, as Markdown gives it none.
                let steps = |kind: Option<&BlockKind>| {
                    matches!(kind, Some(BlockKind::ListItem { .. } | BlockKind::Code { .. }))
                };
                let doc = self.doc();
                let among = || self.touched(&caret.selection).iter().any(|id| steps(doc.block(id).map(|b| &b.kind)));
                steps(kind) || self.grid(&caret.selection.head.block).is_some() || (ranged && among())
            }
            Command::ToggleChecked => {
                matches!(kind, Some(BlockKind::ListItem { list: ListKind::Task { .. }, .. }))
            }
            Command::ToggleMark(_) => {
                ranged || !(caret.picture || matches!(kind, Some(BlockKind::Code { .. } | BlockKind::Formula)))
            }
            Command::SetLink(_) => ranged || self.link().is_some(),
            Command::SetLang(_) => self.takes_lang(),
            Command::LinkAsCard => self.takes_card(),
            Command::Copy | Command::Cut => ranged || caret.picture,
            Command::LeaveFormula => self.formula().is_some(),
            Command::Table(_) => self.cell().is_some(),
            Command::Callout(_) => self.callout().is_some(),
            Command::MoveFigure(_) | Command::ResizeFigure { .. } | Command::SetAlt(_) => self.subject().is_some(),
            Command::ShiftFigure { .. } => caret.picture && self.member(&caret.selection.head.block),
            Command::SetNumbered(_) => self.numbered_by(&caret.selection.head.block).is_some(),
            Command::EditCaption => caret.picture,
            Command::LeaveCaption => self.captioned().is_some(),
            Command::TakeReference(_) => self.referable(),
            Command::DismissReference => self.referring().is_some(),
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
        self.renames.clear();
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
            Command::Split => kind = self.split(true, platform),
            Command::LeaveFormula => self.leave_formula(platform),
            Command::LineBreak => self.insert("\n"),
            Command::Indent => self.indent(true, platform),
            Command::Outdent => self.indent(false, platform),
            Command::ToggleMark(mark) => self.toggle_mark(mark),
            Command::SetLink(link) => self.set_link(link),
            Command::SetBlock(block) => self.set_block(block, platform),
            Command::ToggleChecked => self.toggle_checked(),
            Command::InsertTable { rows, columns } => self.insert_table(rows, columns, platform),
            Command::Table(edit) => self.table_edit(edit, platform),
            Command::Callout(edit) => self.callout_edit(edit),
            Command::InsertContents => self.insert_contents(platform),
            Command::InsertPreview(url) => self.insert_preview(&url, platform),
            Command::SetContentsLevels { table, levels } => self.set_levels(&table, levels),
            Command::Preview { block, edit } => self.preview_edit(&block, &edit, platform),
            Command::LinkAsCard => self.link_as_card(platform),
            Command::CardAsLink(card) => self.card_as_link(&card, platform),
            Command::Fetched { url, title, description, picture } => {
                self.fetched(&url, &title, &description, picture.as_ref(), platform);
            }
            Command::InsertFigures { pictures, at } => self.insert_figures(&pictures, at.as_ref(), platform),
            Command::MoveFigure(spot) => self.move_figure(&spot, platform),
            Command::ShiftFigure { right } => self.shift_figure(right, platform),
            Command::ResizeFigure { width, height } => self.resize_figure(width, height),
            Command::SetAlt(alt) => self.set_alt(alt),
            Command::SetNumbered(on) => {
                let holder = self.selection().and_then(|selection| self.numbered_by(&selection.head.block));
                if let Some(holder) = holder.map(|holder| holder.id.clone()) {
                    self.set_numbered(&holder, on);
                }
            }
            Command::EditCaption => self.edit_caption(platform),
            Command::LeaveCaption => self.leave_caption(),
            Command::EmbedFigure { figure, picture } => self.embed_figure(&figure, &picture, platform),
            Command::TakeReference(to) => self.take_reference(&to),
            Command::DismissReference => self.dismiss_reference(),
            Command::Undo => _ = self.undo_step(),
            Command::Redo => self.redo_step(),
            Command::SelectAll => self.select_all(),
            Command::SelectWord => self.select_word(),
            Command::SelectBlock => self.select_block(),
            Command::Copy => self.copy(platform),
            Command::Cut => self.cut(platform),
            Command::Paste => self.paste(false, platform),
            Command::PastePlain => self.paste(true, platform),
            Command::Replace { query, with, only } => self.replace_found(&query, &with, only.as_ref()),
            Command::SetStyle { element, style } => _ = self.apply(vec![Op::SetStyle { element, style }]),
            Command::SetPage(page) => _ = self.apply(vec![Op::SetPage { page }]),
            Command::SetPageBreak { block, on } => {
                // The op sets all a block may carry, so the language goes back as it is.
                let lang = self.doc().block(&block).and_then(|block| block.lang.clone());
                _ = self.apply(vec![Op::SetBlockProps { block, page_break: on, lang }]);
            }
            Command::UseVersion { block, version } => self.use_version(block, version),
            Command::UseVersions(blocks) => {
                for (block, version) in blocks {
                    // One that came before may have changed what this one can take.
                    if self.takes_version(&block, version.as_deref()) {
                        self.use_version(block, version);
                    }
                }
            }
            Command::PutBack { block, stood } => self.put_back(*block, &stood),
            Command::Restore(revision) => self.restore_version(&revision),
            Command::ReplaceStyle(style) => _ = self.apply(vec![Op::ReplaceStyle { style }]),
            Command::SetLanguage(language) => _ = self.apply(vec![Op::SetLanguage { language }]),
            Command::SetLang(lang) => self.set_lang(lang),
            Command::AddWord(word) => _ = self.apply(vec![Op::AddWord { word }]),
            Command::RemoveWord(word) => _ = self.apply(vec![Op::RemoveWord { word }]),
        }
        self.commit(kind, now, false);
        self.forget();
        true
    }

    /// Applies `ops` as one model edit. Returns whether the document changed.
    fn apply(&mut self, ops: Vec<Op>) -> bool {
        // Where nothing may be written this is the floor under every command, whatever let it through.
        if ops.is_empty() || self.read_only {
            return false;
        }
        let (ops, renames) = shared::unshared(self.doc(), ops);
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
            | Op::SetLang { block, .. }
            | Op::SetKind { block, .. }
            | Op::SetBlockProps { block, .. } => Some(block.clone()),
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
        // The selection stays with a block that came back under another id.
        for pos in self.caret.iter_mut().flat_map(|caret| [&mut caret.selection.anchor, &mut caret.selection.head]) {
            pos.block = shared::renamed(&renames, &pos.block);
        }
        self.renames.extend(renames);
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
        // An edit may have taken away the block its caret was left in.
        if self.caret.as_ref().is_some_and(|caret| self.fit(caret).as_ref() != Some(caret)) {
            self.restore(self.caret.clone());
        }
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
        self.model.block(id)
    }

    /// The id that a block goes by now which the command at hand took out and put back. Any id held from
    /// before such an edit is asked for here, since a block that shares it may have taken it.
    fn renamed(&self, id: &BlockId) -> BlockId {
        shared::renamed(&self.renames, id)
    }

    /// The position on a character boundary of a text block that is nearest to `pos`.
    fn clamp(&self, pos: &Pos) -> Option<Pos> {
        let block = self.block(&pos.block).filter(|block| tree::is_text(block))?;
        let offset = text::snap(&block.plain_text(), pos.offset.min(self.span(block)));
        // A reference is come to at one of its ends, whichever is nearer.
        let offset = match refs::around(block, offset) {
            Some(range) if offset - range.start < range.end - offset => range.start,
            Some(range) => range.end,
            None => offset,
        };
        Some(Pos::new(pos.block.clone(), offset))
    }

    /// A selection with neither end inside a reference: a caret stands at the nearer end of one, and a
    /// range takes it whole.
    fn whole(&self, selection: Selection) -> Selection {
        let inside = |pos: &Pos| self.block(&pos.block).and_then(|block| refs::around(block, pos.offset));
        if inside(&selection.anchor).is_none() && inside(&selection.head).is_none() {
            return selection;
        }
        if selection.is_caret() {
            return self.clamp(&selection.head).map_or(selection, Selection::caret);
        }
        let (start, end) = self.ordered(&selection);
        let from = inside(&start).map_or(start.offset, |range| range.start);
        let to = inside(&end).map_or(end.offset, |range| range.end);
        let (from, to) = (Pos::new(start.block.clone(), from), Pos::new(end.block, to));
        if selection.anchor == start {
            Selection { anchor: from, head: to }
        } else {
            Selection { anchor: to, head: from }
        }
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
        let Some(block) = self.block(&pos.block).filter(|block| !text::is_source(block)) else {
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
        let selection = self.whole(selection);
        let marks = if selection.is_caret() {
            self.natural(&selection.head)
        } else {
            let (start, _) = self.ordered(&selection);
            let block = self.block(&start.block).filter(|block| !text::is_source(block));
            block.map(|block| text::marks_after(block.body.runs(), start.offset)).unwrap_or_default()
        };
        let picture = selection.is_caret() && self.member(&selection.head.block);
        let before_wrap = self.stays(&selection);
        // Text typed beside a reference is no part of it.
        self.caret = Some(Caret { selection, marks: refs::plain(marks), picture, before_wrap });
    }

    /// Whether a caret set to `selection` is on the line it was on: one that has not moved is.
    fn stays(&self, selection: &Selection) -> bool {
        self.caret.as_ref().is_some_and(|caret| caret.before_wrap && caret.selection == *selection)
    }

    fn place(&mut self, pos: Pos) {
        self.set_selection(Selection::caret(pos));
    }

    /// Puts the caret at `pos` with marks chosen by the caller.
    fn place_with(&mut self, pos: Pos, marks: BTreeSet<Mark>) {
        let pos = self.clamp(&pos).unwrap_or(pos);
        let picture = self.member(&pos.block);
        let selection = Selection::caret(pos);
        let before_wrap = self.stays(&selection);
        self.caret = Some(Caret { selection, marks: refs::plain(marks), picture, before_wrap });
    }
}
