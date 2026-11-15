//! The one way to change a document.
//!
//! Every edit is a list of [`Op`]s applied as a unit: all of them land, or none do, and together they are one
//! undo step. Behind the document sits its history, a CRDT, so any two copies of one document merge.
//! [`Model::doc`] is a view of that history, brought up to date by each change and never rebuilt.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};
use std::mem;
use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use loro::cursor::{Cursor, Side};
use loro::{ContainerTrait, ExportMode, Frontiers, LoroDoc, LoroText, LoroValue, Subscription, TextDelta};

use crate::conflict::{self, Conflict};
use crate::crdt::{self, Assign, Event, attrs};
use crate::flat::{self, BLOCK, DEPTH, Entry, Flat, children, head, insert, marker, nth, nth_mut, spell};
use crate::history::{HistoryError, Hold, ReplicaId, Revision, StablePos};
use crate::undo::{Delta, Edit, Script, Step, Steps, is_skip};
use crate::versions::{self, Point, Tag, TagKind};
use crate::{Block, BlockId, BlockKind, Body, BodyShape, Document, Mark, Page, Pos, Properties, Run, Style, TextStyle};

/// One change to a document. Offsets are UTF-8 bytes into a text block and must sit on character boundaries.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Inserts text, which takes the formatting of the character before it, or at the start of a block
    /// that of the character after it. Text put at the end of a reference is no part of it.
    InsertText {
        at: Pos,
        text: String,
    },
    DeleteText {
        block: BlockId,
        range: Range<usize>,
    },
    SetMark {
        block: BlockId,
        range: Range<usize>,
        mark: Mark,
        on: bool,
    },
    SetLink {
        block: BlockId,
        range: Range<usize>,
        link: Option<String>,
    },
    /// Sets the language of a stretch of text, or with `None` leaves it to its block's.
    SetLang {
        block: BlockId,
        range: Range<usize>,
        lang: Option<String>,
    },
    /// Splits a text block in two. The text after `at` moves to a new block of the same kind, placed after it.
    Split {
        at: Pos,
        new_id: BlockId,
    },
    /// Appends a text block's content to the text block before it in the document, then removes it.
    Join {
        block: BlockId,
    },
    /// Changes a block's kind to another kind that holds the same shape of body, or changes the fields of
    /// its kind, as resizing a figure does.
    SetKind {
        block: BlockId,
        kind: BlockKind,
    },
    /// Sets what any block may carry: whether it starts a new page, and its language as a BCP 47 tag.
    SetBlockProps {
        block: BlockId,
        page_break: bool,
        lang: Option<String>,
    },
    /// Inserts a block at `index` among the children of `parent`, or at the top level. Removing a block
    /// and inserting it elsewhere under its own id, in one edit, moves it.
    InsertBlock {
        parent: Option<BlockId>,
        index: usize,
        block: Block,
    },
    RemoveBlock {
        block: BlockId,
    },
    /// Sets the style of one element type, such as `heading1`. Each field that differs is a change of its
    /// own, so two replicas that change different fields of one type both keep theirs.
    SetStyle {
        element: String,
        style: TextStyle,
    },
    /// Sets the paper size and margins.
    SetPage {
        page: Page,
    },
    /// Replaces the whole style, as applying a stylesheet does.
    ReplaceStyle {
        style: Style,
    },
    SetLanguage {
        language: String,
    },
    /// Adds a word to the document's dictionary. It stays when another replica removes the word at the
    /// same time.
    AddWord {
        word: String,
    },
    RemoveWord {
        word: String,
    },
    /// Sets the document's properties, or with `None` takes them away.
    SetProperties {
        properties: Option<Properties>,
    },
}

/// Why an [`Op`] was refused. The document is left as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    NoSuchBlock(BlockId),
    /// The block does not hold what the operation needs: text, child blocks, or children of that kind.
    WrongBody(BlockId),
    /// The kind the block was to have breaks the rules of its type, as a figure without a picture does.
    BadKind(BlockId),
    /// An offset is past the end of the text or inside a character.
    BadOffset {
        block: BlockId,
        offset: usize,
    },
    BadIndex {
        index: usize,
    },
    DuplicateId(BlockId),
    /// There is no text block before this one to join.
    NothingToJoin(BlockId),
    /// The model shows an older revision, which cannot be edited.
    ReadOnly,
    /// The style holds a value `style.json` would refuse, and this says which.
    BadStyle(String),
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchBlock(id) => write!(f, "there is no block \"{id}\""),
            Self::WrongBody(id) => write!(f, "block \"{id}\" does not hold what this operation changes"),
            Self::BadKind(id) => write!(f, "block \"{id}\" was given a kind that breaks the rules of its type"),
            Self::BadOffset { block, offset } => write!(f, "offset {offset} is not a position in block \"{block}\""),
            Self::BadIndex { index } => write!(f, "index {index} is past the end of the list"),
            Self::DuplicateId(id) => write!(f, "the id \"{id}\" is already in use"),
            Self::NothingToJoin(id) => write!(f, "there is no text block before \"{id}\" to join"),
            Self::ReadOnly => f.write_str("an older revision cannot be edited"),
            Self::BadStyle(why) => write!(f, "the style cannot be used: {why}"),
        }
    }
}

impl std::error::Error for OpError {}

/// What an edit touched, so that layout can redo only that.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// Blocks whose content or kind changed, in no particular order. Includes added and removed blocks.
    pub blocks: BTreeSet<BlockId>,
    /// Whether blocks were added, removed or moved.
    pub structure: bool,
    /// Whether the style changed, which the layout of every block depends on.
    pub style: bool,
    /// Whether the document's own fields changed: its language, its dictionary or its properties.
    pub document: bool,
}

impl Change {
    /// Takes in what another change touched, for two that are told as one.
    pub fn absorb(&mut self, other: Change) {
        self.blocks.extend(other.blocks);
        self.structure |= other.structure;
        self.style |= other.style;
        self.document |= other.document;
    }
}

/// The entry of a block, and where the block starts in the sequence, in characters.
type Seat = (usize, usize);

/// Where a block sits: the block that holds it, and its place among that block's children or, at the top
/// level, among the document's blocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place<'a> {
    pub parent: Option<&'a Block>,
    pub index: usize,
}

/// Each block's entry by the hash of its id, which is spread well enough to be the map's own hash.
type Lookup = HashMap<u64, u32, BuildHasherDefault<Ready>>;

#[derive(Default)]
struct Ready(u64);

impl Hasher for Ready {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0 = bytes.iter().fold(self.0, |hash, byte| hash.rotate_left(8) ^ u64::from(*byte));
    }

    fn write_u64(&mut self, hash: u64) {
        self.0 = hash;
    }
}

/// The history behind a document, and what it has reported since the model last caught up with it.
struct History {
    loro: LoroDoc,
    body: LoroText,
    events: Arc<Mutex<Vec<Event>>>,
    _watch: Subscription,
}

impl History {
    fn open(loro: LoroDoc, replica: ReplicaId) -> Self {
        // Loro draws an id of its own for every document. It is replaced before anything is written.
        loro.set_peer_id(replica.0).expect("a replica id is one Loro accepts");
        let body = loro.get_text(crdt::BODY);
        let (events, watch) = crdt::watch(&loro, &body);
        Self { loro, body, events, _watch: watch }
    }
}

/// Who writes with a replica, and when: what the shell tells a model that has no clock and knows no one.
#[derive(Clone, Debug, Default)]
struct Pen {
    author: Option<String>,
    /// Whether the history holds the author's name against this replica.
    signed: bool,
    /// Seconds since 1970. 0 until the shell says.
    time: u64,
}

/// One replica of a document: the document, its history and this session's undo steps.
///
/// A model is not `Clone`, because two writers must never share a [`ReplicaId`]. Clone [`Model::doc`] to
/// read, and [`Model::fork`] to write.
pub struct Model {
    doc: Document,
    /// One entry per block, in document order.
    index: Vec<Entry>,
    style: Style,
    /// Whether the style is this model's own: given to it, edited, or read from its history. The built-in
    /// style a model starts with is not, and gives way to the style of a history it is handed.
    styled: bool,
    replica: ReplicaId,
    /// Started from the document when something first needs it, unless one is attached before.
    history: OnceLock<History>,
    steps: Steps,
    /// Made when a block is first looked up by id, and again after blocks come, go, move or are renamed.
    lookup: OnceLock<Lookup>,
    pen: Pen,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut model = f.debug_struct("Model");
        model.field("replica", &self.replica).field("doc", &self.doc).field("style", &self.style).finish()
    }
}

/// A history made ready for a model, away from it. See [`Model::prepare`].
pub struct Prepared(Model);

impl fmt::Debug for Prepared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Prepared").field(&self.0.replica).finish()
    }
}

/// A shell hands these to the thread that owns the window, and may make them on another.
const _: fn() = || {
    fn sent<T: Send>() {}
    sent::<Model>();
    sent::<Prepared>();
};

impl Model {
    /// Shows a document at once, in the built-in style. Its history is not built until an edit, a save or
    /// a merge needs it, or until [`Model::attach`] hands one over, so opening a document never waits.
    ///
    /// A history started here depends on the document and its style alone, so two devices that open the
    /// same file without a history can still merge their copies. Runs are stored merged, and a repeated
    /// block id is renamed.
    pub fn new(mut doc: Document, replica: ReplicaId) -> Self {
        let built = flat::normal(mem::take(&mut doc.blocks));
        doc.blocks = built.blocks;
        crdt::normal_meta(&mut doc);
        let (index, style, lookup) = (built.index, Style::default(), OnceLock::new());
        let (history, steps, pen) = (OnceLock::new(), Steps::default(), Pen::default());
        Self { doc, index, style, styled: false, replica, history, steps, lookup, pen }
    }

    /// The model with the style its file holds, for one that was just made.
    pub fn with_style(mut self, style: Style) -> Self {
        match self.history.get() {
            // Nothing has been written yet, so the history will start from this style.
            None => self.style = crdt::normal_style(&style),
            // Only an older revision refuses a style, and a model that was just made shows none.
            Some(_) => drop(self.rewrite_style(style)),
        }
        self.styled = true;
        self
    }

    /// Opens a saved history, as [`Model::save`] or [`Model::updates_since`] wrote it.
    pub fn load(history: &[u8], replica: ReplicaId) -> Result<Self, HistoryError> {
        let loro = LoroDoc::new();
        crdt::configure(&loro);
        loro.import(history).map_err(unreadable)?;
        let history = History::open(loro, replica);
        let (doc, index) = crdt::read(&history.loro, &history.body);
        let style = crdt::read_style(&history.loro);
        let (history, steps, lookup) = (OnceLock::from(history), Steps::default(), OnceLock::new());
        Ok(Self { doc, index, style, styled: true, replica, history, steps, lookup, pen: Pen::default() })
    }

    /// A second replica at the newest revision, for another writer. It starts with no undo steps.
    pub fn fork(&self, replica: ReplicaId) -> Self {
        match self.history.get() {
            Some(_) => Self::load(&self.save(), replica).expect("a history that was just saved loads"),
            // Both then start the same history from the same document.
            None => {
                let (doc, index, style) = (self.doc.clone(), self.index.clone(), self.style.clone());
                let (history, steps, lookup) = (OnceLock::new(), Steps::default(), self.lookup.clone());
                Self { doc, index, style, styled: self.styled, replica, history, steps, lookup, pen: Pen::default() }
            }
        }
    }

    /// Does the slow part of opening a file ahead of the model that shows it: reads its `history`, or
    /// starts one when the file has none. A shell makes a second model of the same document and style,
    /// runs this on another thread once the first is on screen, and hands the result to [`Model::attach`].
    ///
    /// A document or style that is not the newest in `history` was changed outside Letters. The history
    /// takes that in as one more change, as [`Model::rewrite`] does, so the file is always the document.
    /// A model that was given no style has none to hold against the history's.
    pub fn prepare(self, history: Option<&[u8]>) -> Result<Prepared, HistoryError> {
        let model = match history {
            None => self,
            Some(history) => {
                let mut model = Self::load(history, self.replica)?;
                // A history that was just loaded shows its newest revision, which can be rewritten.
                let _ = model.rewrite(self.doc);
                if self.styled {
                    let _ = model.rewrite_style(self.style);
                }
                model
            }
        };
        model.history();
        Ok(Prepared(model))
    }

    /// Takes over a history that [`Model::prepare`] made for the document this model shows. The document
    /// and its style stay as they are: what was edited in the meantime goes into that history as one
    /// change. Only a model that was given no style takes the history's, and says so in the change.
    ///
    /// Until a file's history is attached, the model keeps one that starts at the document, which is all
    /// a file without a history needs. When the file has one, revisions and stable positions taken before
    /// it is attached mean nothing after, and neither does anything saved, so a shell saves and merges
    /// once it has attached.
    pub fn attach(&mut self, prepared: Prepared) -> Change {
        let Prepared(theirs) = prepared;
        let Some(history) = theirs.history.into_inner() else { return Change::default() };
        let held = |ours: &History| ours.loro.oplog_vv().partial_cmp(&history.loro.oplog_vv());
        if self.history.get().and_then(held).is_some_and(Ordering::is_ge) {
            // This model's history holds all of that one already, as two started from one document do.
            return Change::default();
        }
        let mut change = self.checkout_latest();
        // Whatever replica the history was prepared for, this model's is the one that writes.
        let _ = history.loro.set_peer_id(self.replica.0);
        self.history = OnceLock::from(history);
        self.pen.signed = false;
        let shown = mem::replace(&mut self.doc, theirs.doc);
        let layout = mem::replace(&mut self.index, theirs.index);
        self.lookup = theirs.lookup;
        let style = mem::replace(&mut self.style, theirs.style);
        let steps = mem::take(&mut self.steps);
        // What this model shows that the history does not is its own writer's work.
        if shown != self.doc {
            let _ = self.adopt(shown.clone(), false);
            change.absorb(diff(&shown, &self.doc));
        }
        if mem::replace(&mut self.styled, true) && style != self.style {
            let _ = self.adopt_style(style.clone(), false);
        }
        change.style |= style != self.style;
        // An undo step counts characters, so it holds for as long as the sequence is laid out as before.
        let alike =
            |a: &Entry, b: &Entry| (a.marker, a.chars, a.depth, &a.stored) == (b.marker, b.chars, b.depth, &b.stored);
        if layout.len() == self.index.len() && layout.iter().zip(&self.index).all(|(a, b)| alike(a, b)) {
            self.steps = steps;
        }
        change
    }

    /// Makes the newest document read as `doc`, which was changed without the model, as a file edited by
    /// another tool is. What differs goes into the history block by block, as one change. It is not an
    /// undo step, and the undo steps already there still hold.
    ///
    /// The change is named after what it starts from and what it leads to, not after this replica. Two
    /// replicas that take in the same outside change therefore write the same change, and merge as one.
    pub fn rewrite(&mut self, doc: Document) -> Result<Change, OpError> {
        self.adopt(doc, true)
    }

    /// Makes the newest style read as `style`, which was changed without the model. Like
    /// [`Model::rewrite`], it is one change in the history and no undo step.
    pub fn rewrite_style(&mut self, style: Style) -> Result<Change, OpError> {
        self.adopt_style(style, true)
    }

    /// Makes the newest document read as `doc`. `outside` says the change is not this replica's own.
    fn adopt(&mut self, mut doc: Document, outside: bool) -> Result<Change, OpError> {
        if !self.is_latest() {
            return Err(OpError::ReadOnly);
        }
        let to = flat::normal(mem::take(&mut doc.blocks));
        crdt::normal_meta(&mut doc);
        doc.blocks = to.blocks;
        let mut change = Change::default();
        if doc == self.doc {
            return Ok(change);
        }
        let delta = (doc.blocks != self.doc.blocks).then(|| {
            let old = flat::order(&self.doc.blocks, &self.index);
            flat::rewrite(&old, &flat::order(&doc.blocks, &to.index))
        });
        if outside {
            let fields = Document { blocks: Vec::new(), ..doc.clone() }.to_json();
            let from = self.revision().to_string();
            let _ = self.loro().set_peer_id(crdt::outsider(&from, delta.as_deref().unwrap_or_default(), &fields));
        }
        if let Some(delta) = delta {
            let (changed, undo) = self.shift(&delta, None);
            change.absorb(changed);
            debug_assert!(undo.is_some() && self.doc.blocks == doc.blocks, "a rewrite did not give its document");
            if undo.is_some() && self.write(std::slice::from_ref(&delta), &mut change) {
                self.steps.rebase(&delta);
            }
        }
        let assigns = crdt::meta_assigns(self.loro(), &self.doc, &doc);
        self.set(&assigns, &mut change);
        let _ = self.loro().set_peer_id(self.replica.0);
        Ok(change)
    }

    fn adopt_style(&mut self, style: Style, outside: bool) -> Result<Change, OpError> {
        if !self.is_latest() {
            return Err(OpError::ReadOnly);
        }
        let style = crdt::normal_style(&style);
        self.styled = true;
        let mut change = Change::default();
        if style != self.style {
            if outside {
                let _ = self.loro().set_peer_id(crdt::outsider(&self.revision().to_string(), &[], &style.to_json()));
            }
            self.set(&crdt::style_assigns(self.loro(), &style), &mut change);
            let _ = self.loro().set_peer_id(self.replica.0);
        }
        Ok(change)
    }

    fn history(&self) -> &History {
        let start = || History::open(crdt::create(&self.doc, &self.index, &self.style), self.replica);
        self.history.get_or_init(start)
    }

    fn loro(&self) -> &LoroDoc {
        &self.history().loro
    }

    fn body(&self) -> &LoroText {
        &self.history().body
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    pub fn style(&self) -> &Style {
        &self.style
    }

    /// Names who writes with this replica. The history keeps the name with the replica's next edit, and
    /// every copy of the document then reads this replica's edits as that writer's.
    pub fn set_author(&mut self, name: &str) {
        if self.pen.author.as_deref() != Some(name) {
            self.pen = Pen { author: Some(name.to_owned()), signed: false, time: self.pen.time };
        }
    }

    /// Tells the model the time, in seconds since 1970, as it has no clock to read. Edits made from here
    /// on are dated with it, to the minute. An edit is never dated before one it builds on.
    pub fn set_time(&mut self, seconds: u64) {
        self.pen.time = seconds;
    }

    /// The names of everyone who has written in the document, as far as the revision shown.
    pub fn authors(&self) -> BTreeSet<String> {
        self.history.get().map(|history| crdt::authors(&history.loro)).unwrap_or_default()
    }

    /// Finds a block by its id, as [`Document::block`] does, without searching the document for it.
    pub fn block(&self, id: &BlockId) -> Option<&Block> {
        nth(&self.doc.blocks, &self.index, self.find(id)?)
    }

    /// Where a block sits. `None` when the document has no such block.
    pub fn place(&self, id: &BlockId) -> Option<Place<'_>> {
        let entry = &self.index[self.find(id)?];
        let parent = (entry.level > 0).then(|| nth(&self.doc.blocks, &self.index, entry.parent)).flatten();
        Some(Place { parent, index: entry.nth })
    }

    /// The entry of the block with this id.
    fn find(&self, id: &BlockId) -> Option<usize> {
        let lookup = self.lookup.get_or_init(|| {
            let mut lookup = Lookup::with_capacity_and_hasher(self.index.len(), BuildHasherDefault::default());
            for (i, entry) in self.index.iter().enumerate() {
                lookup.entry(entry.hash).or_insert(i as u32);
            }
            lookup
        });
        let i = *lookup.get(&flat::hash(id))? as usize;
        let found = nth(&self.doc.blocks, &self.index, i).is_some_and(|block| block.id == *id);
        // Two ids that share a hash are told apart the slow way.
        if found { Some(i) } else { self.spot(id).ok().map(|spot| spot.i) }
    }

    pub fn replica(&self) -> ReplicaId {
        self.replica
    }

    /// Applies `ops` in order as one undo step. Ops that leave everything as it was are no step at all.
    ///
    /// Blocks that share an id go by a number each, as in `id~2`. The ops name them as they stood before
    /// the first of the ops, also once an earlier op has taken one of them away.
    pub fn apply(&mut self, ops: impl IntoIterator<Item = Op>) -> Result<Change, OpError> {
        if self.loro().is_detached() {
            return Err(OpError::ReadOnly);
        }
        let mut ops: Vec<Op> = ops.into_iter().collect();
        let mut copies = Copies::of(self, &mut ops);
        // The document's own fields and the style are changed on copies, made when an op first needs
        // them, and written once every op has been checked.
        let mut settings: Option<(Document, Style)> = None;
        // One op on a block changes the document whenever it changes the sequence. Several may cancel
        // out, unless one of them adds or removes a block.
        fn named(op: &Op) -> Option<&BlockId> {
            match op {
                Op::InsertText { at, .. } => Some(&at.block),
                Op::DeleteText { block, .. } | Op::SetMark { block, .. } | Op::SetLink { block, .. } => Some(block),
                Op::SetLang { block, .. } | Op::SetKind { block, .. } | Op::SetBlockProps { block, .. } => Some(block),
                _ => None,
            }
        }
        let texts = || ops.iter().filter(|op| !is_setting(op));
        let before: Option<Vec<Block>> = (texts().count() > 1)
            .then(|| texts().map(named).collect())
            .flatten()
            .map(|ids: BTreeSet<&BlockId>| ids.into_iter().filter_map(|id| self.doc.block(id).cloned()).collect());

        let mut change = Change::default();
        let mut edits = Vec::new();
        let mut step = Step::new();
        let mut refused = None;
        for (nth, op) in ops.iter_mut().enumerate() {
            if let Some((named, now)) = target(op).zip(copies.now(nth)) {
                *named = now;
            }
            let op = &*op;
            let compiled = if is_setting(op) {
                let (fields, style) = settings.get_or_insert_with(|| (self.fields(), self.style.clone()));
                set(op, fields, style).map(|()| (Vec::new(), None))
            } else {
                self.compile(op)
            };
            match compiled {
                Ok((delta, _)) if delta.is_empty() => {}
                Ok((delta, place)) => {
                    let (changed, undo) = self.shift(&delta, place);
                    debug_assert!(undo.is_some(), "an edit that was checked did not fit the document");
                    change.absorb(changed);
                    step.push(Edit::Text(undo.unwrap_or_default()));
                    edits.push(delta);
                    if matches!(op, Op::RemoveBlock { .. } | Op::Join { .. }) {
                        copies.took(nth);
                    }
                }
                Err(error) => {
                    refused = Some(error);
                    break;
                }
            }
        }
        step.reverse();
        let same = |blocks: &Vec<Block>| blocks.iter().all(|block| self.doc.block(&block.id) == Some(block));
        if refused.is_some() || before.as_ref().is_some_and(same) {
            // Nothing has reached the history yet, so the document's own undo puts it back.
            for undo in step.drain(..) {
                if let Edit::Text(undo) = undo {
                    self.shift(&undo, None);
                }
            }
            if let Some(error) = refused {
                return Err(error);
            }
            (change, edits) = (Change::default(), Vec::new());
        }
        let written = edits.is_empty() || self.write(&edits, &mut change);
        if let Some((fields, style)) = settings {
            let mut assigns = crdt::meta_assigns(self.loro(), &self.doc, &fields);
            if style != self.style {
                assigns.extend(crdt::style_assigns(self.loro(), &style));
                self.styled = true;
            }
            step.insert(0, Edit::Set(self.set(&assigns, &mut change)));
        }
        if written {
            self.steps.record(step, false);
        }
        Ok(change)
    }

    /// Steps back one of this replica's edits, leaving what others wrote since. `None` when there is none.
    /// An edit that others have left nothing of is still a step, and stepping back over it changes nothing.
    pub fn undo(&mut self) -> Option<Change> {
        self.step(true)
    }

    /// Reapplies the edit last undone. `None` when there is nothing to redo.
    pub fn redo(&mut self) -> Option<Change> {
        self.step(false)
    }

    fn step(&mut self, back: bool) -> Option<Change> {
        let step = self.steps.take(back).filter(|_| !self.loro().is_detached())?;
        let mut change = Change::default();
        let mut reverse = Step::new();
        let mut edits = Vec::new();
        for edit in &step {
            let Edit::Text(delta) = edit else { continue };
            let (changed, undo) = self.shift(delta, None);
            change.absorb(changed);
            let Some(undo) = undo else {
                // The document has been read again from the history, which this step never reached.
                debug_assert!(false, "an undo step no longer fits the document");
                self.steps.clear();
                return Some(change);
            };
            reverse.push(Edit::Text(undo));
            edits.push(delta.clone());
        }
        let written = edits.is_empty() || self.write(&edits, &mut change);
        for edit in &step {
            if let Edit::Set(assigns) = edit {
                reverse.push(Edit::Set(self.set(assigns, &mut change)));
            }
        }
        reverse.reverse();
        if written {
            self.steps.stepped(reverse, back);
        }
        Some(change)
    }

    /// Makes assignments in the history's maps, as one change there, and reads what they changed. Gives
    /// the assignments that take them back.
    fn set(&mut self, assigns: &[Assign], change: &mut Change) -> Vec<Assign> {
        if assigns.is_empty() {
            return Vec::new();
        }
        // Loro refuses none of these: every key is a string, and every value a string or a flag.
        let back = crdt::assign(self.loro(), assigns).unwrap_or_default();
        self.commit();
        self.queue().clear();
        change.absorb(self.reread());
        back
    }

    /// The document's own fields, without its blocks.
    fn fields(&self) -> Document {
        let doc = &self.doc;
        Document {
            blocks: Vec::new(),
            language: doc.language.clone(),
            dictionary: doc.dictionary.clone(),
            properties: doc.properties.clone(),
            extra: doc.extra.clone(),
        }
    }

    /// Reads the document's own fields and the style from the history again, and says which changed.
    fn reread(&mut self) -> Change {
        let Some(history) = self.history.get() else { return Change::default() };
        let mut read = Document::default();
        crdt::read_meta(&history.loro, &mut read);
        let style = crdt::read_style(&history.loro);
        let change =
            Change { style: style != self.style, document: fields(&read) != fields(&self.doc), ..Change::default() };
        let doc = &mut self.doc;
        (doc.language, doc.dictionary, doc.properties, doc.extra) =
            (read.language, read.dictionary, read.properties, read.extra);
        self.style = style;
        change
    }

    /// Closes a change in the history. This replica's own carries the time and, the first time, its
    /// writer's name. An outside change carries neither, as every device must write it alike.
    fn commit(&mut self) {
        let loro = &self.history().loro;
        if loro.peer_id() == self.replica.0 {
            if let Some(author) = self.pen.author.as_deref().filter(|_| !self.pen.signed) {
                crdt::sign(loro, author);
            }
            if self.pen.time > 0 {
                loro.set_next_commit_timestamp(i64::try_from(self.pen.time).unwrap_or(i64::MAX));
            }
            loro.commit();
            self.pen.signed = true;
        } else {
            loro.commit();
        }
    }

    /// Sends changes the document already shows to the history, as one change there.
    fn write(&mut self, edits: &[Delta], change: &mut Change) -> bool {
        let (mut written, mut edits) = (true, edits.iter().peekable());
        while let Some(delta) = edits.next().filter(|_| written) {
            // Text put in where text was just taken out replaces it, and is written as that.
            let both = edits.peek().and_then(|put| replacement(delta, put));
            if both.is_some() {
                edits.next();
            }
            written = crdt::write(self.loro(), self.body(), both.as_ref().unwrap_or(delta)).is_some();
        }
        self.commit();
        if written {
            // The history reports the edit back, and the document already shows it.
            self.queue().clear();
        } else {
            // The history is what every other replica sees, so it wins over what was meant.
            change.absorb(self.resync());
            self.steps.clear();
        }
        written
    }

    /// The whole history, for the file. Any two saves of one document merge.
    pub fn save(&self) -> Vec<u8> {
        // Every edit, without Loro's snapshot of where they lead. Loro 1.17 reads the snapshot of a long
        // text in quadratic time, and replays the edits of the same text in a tenth of that.
        self.loro().export(ExportMode::all_updates()).expect("a history can always be written out")
    }

    /// What this replica holds beyond `revision`, for a replica that is already there. Everything, when
    /// this history does not know the revision.
    pub fn updates_since(&self, revision: &Revision) -> Vec<u8> {
        let from = self.loro().frontiers_to_vv(&revision.0).unwrap_or_default();
        self.loro().export(ExportMode::updates(&from)).expect("a history can always be written out")
    }

    /// Takes in what another replica wrote. Merging is commutative and idempotent: replicas that have seen
    /// the same edits show the same document, and a merge that adds nothing changes nothing.
    pub fn merge(&mut self, history: &[u8]) -> Result<Change, HistoryError> {
        self.loro().import(history).map_err(unreadable)?;
        Ok(self.drain())
    }

    /// How this history stands to a saved one: `Greater` holds everything in it and more, `Less` lacks
    /// some of it, and `None` means each holds edits the other lacks.
    pub fn compare(&self, history: &[u8]) -> Result<Option<Ordering>, HistoryError> {
        let other = LoroDoc::decode_import_blob_meta(history, true).map_err(unreadable)?;
        Ok(self.loro().oplog_vv().partial_cmp(&other.partial_end_vv))
    }

    /// Whether a saved history and this one hold an edit in common, as two that began as one file do.
    /// Histories that hold none still merge, into a document with everything from both.
    pub fn related(&self, history: &[u8]) -> Result<bool, HistoryError> {
        let other = LoroDoc::decode_import_blob_meta(history, true).map_err(unreadable)?;
        let ours = self.loro().oplog_vv();
        let shared = |(peer, end): (&u64, &i32)| *end > 0 && ours.get(peer).is_some_and(|held| *held > 0);
        Ok(other.partial_end_vv.iter().any(shared))
    }

    /// The point in history the document shows.
    pub fn revision(&self) -> Revision {
        Revision(self.loro().state_frontiers())
    }

    /// Whether the document shows the newest revision, and so can be edited.
    pub fn is_latest(&self) -> bool {
        self.history.get().is_none_or(|history| !history.loro.is_detached())
    }

    /// Shows the document as it was at `revision`, read-only. Leaving the newest revision forgets the
    /// undo steps.
    pub fn checkout(&mut self, revision: &Revision) -> Result<Change, HistoryError> {
        if revision.0 == self.loro().oplog_frontiers() {
            return Ok(self.checkout_latest());
        }
        self.loro().checkout(&revision.0).map_err(|_| HistoryError::UnknownRevision)?;
        self.steps.clear();
        Ok(self.drain())
    }

    /// Returns to the newest revision, with everything merged in the meantime.
    pub fn checkout_latest(&mut self) -> Change {
        if self.is_latest() {
            return Change::default();
        }
        self.loro().checkout_to_latest();
        self.drain()
    }

    /// The blocks that two revisions both changed, in the same words, since they last agreed. After a
    /// merge, give the revision each copy stood at before it, in either order.
    ///
    /// Each comes with the block as either side left it, and who last changed it there and when. A block
    /// that one side took away while the other changed its text is among them. Changes to different words
    /// of a block are not, nor is formatting: those merge. So does text put in beside a word, which
    /// changes the word only when it reads on from it or into it, with no space or mark between. Nor is a
    /// block that both sides left reading alike, where there is nothing to choose.
    /// Fails with [`HistoryError::UnknownRevision`] for a revision the history lacks.
    ///
    /// The model reads its history at other revisions meanwhile, and so is taken whole. The document it
    /// shows stays as it is.
    pub fn conflicts(&mut self, ours: &Revision, theirs: &Revision) -> Result<Vec<Conflict>, HistoryError> {
        let history = self.history();
        let (shown, detached) = (history.loro.state_frontiers(), history.loro.is_detached());
        let found = conflict::find(&history.loro, &history.body, &history.events, [&ours.0, &theirs.0]);
        if detached {
            // The history held this revision a moment ago.
            let _ = history.loro.checkout(&shown);
        } else {
            history.loro.checkout_to_latest();
        }
        self.queue().clear();
        Ok(conflict::told(found?))
    }

    /// Finds the conflicts as [`Model::conflicts`] does, and settles each: its block comes to read as the
    /// side that was edited last left it, until a person picks a version. Call it after a merge, with the
    /// revision each copy stood at before. Returns the conflicts, for the notes to be raised.
    ///
    /// The text of the block is the later side's: what the other side cut from it is put back, and what the
    /// other side put into its own block is taken out. A block the later side took away goes, and one the
    /// earlier side took away comes back. Formatting and the kind of block merge as they do anywhere, and
    /// a block that one side joined to its neighbour stays joined.
    ///
    /// Every device that merges the same two revisions writes the same change, so two that settle them
    /// apart settle them once. A device that merged another revision, having written on, writes a change
    /// of its own: when the two meet, settling again leaves the later of them standing, and returns no
    /// conflict for it, as each device returned its own. Settling is no undo step, and the undo steps
    /// already there still hold.
    pub fn settle(&mut self, ours: &Revision, theirs: &Revision) -> Result<(Change, Vec<Conflict>), HistoryError> {
        let history = self.history();
        let loro = &history.loro;
        let (shown, detached) = (loro.state_frontiers(), loro.is_detached());
        let found = conflict::find(loro, &history.body, &history.events, [&ours.0, &theirs.0]);
        let mut written = false;
        if let Ok(clashes) = &found {
            // The change is written at an earlier revision, which Loro allows on request. It then draws a
            // random replica for each revision it shows, under which nothing is written.
            loro.set_detached_editing(true);
            written = conflict::settle(loro, &history.body, [&ours.0, &theirs.0], clashes);
            loro.set_detached_editing(false);
            let _ = loro.set_peer_id(self.replica.0);
        }
        if detached || written {
            // The history held this revision a moment ago.
            let _ = loro.checkout(&shown);
            self.queue().clear();
        }
        let mut change = Change::default();
        if !detached {
            self.loro().checkout_to_latest();
            // Coming back from the revision shown before tells what the settling did to the document.
            if written {
                change = self.drain();
            }
            self.queue().clear();
        }
        Ok((change, conflict::told(found?)))
    }

    /// Shows the history at `revision`, or at its newest revision for `None`, for `read` to look at, and
    /// then as it was. The document the model shows stays as it is.
    fn look<T>(&self, revision: Option<&Revision>, read: impl FnOnce(&History) -> T) -> Result<T, HistoryError> {
        let history = self.history();
        let loro = &history.loro;
        let shown = loro.is_detached().then(|| loro.state_frontiers());
        let there = match revision {
            Some(revision) => loro.checkout(&revision.0).map_err(|_| HistoryError::UnknownRevision),
            None => {
                loro.checkout_to_latest();
                Ok(())
            }
        };
        let read = there.map(|()| read(history));
        self.show(shown);
        read
    }

    /// Shows the history as it was before a look elsewhere: at the revision given, or else at its newest.
    fn show(&self, shown: Option<Frontiers>) {
        match shown {
            // The history held this revision a moment ago.
            Some(shown) => drop(self.loro().checkout(&shown)),
            None => self.loro().checkout_to_latest(),
        }
        self.queue().clear();
    }

    /// Every step the history was written in, oldest first: the revisions either side of it, who wrote it
    /// and when, how much it changed, and the versions marked at it. [`Model::changed`] tells which
    /// blocks a step changed, and [`Model::document_at`] reads the document either side of it.
    ///
    /// Every device lists the same history alike. Steps that two devices wrote apart are listed one
    /// after the other, in an order all devices agree on.
    pub fn timeline(&mut self) -> Vec<Point> {
        self.look(None, |history| versions::timeline(&history.loro)).unwrap_or_default()
    }

    /// The document and its style as they were at `revision`, read without leaving the revision shown,
    /// and with the undo steps kept. Fails with [`HistoryError::UnknownRevision`] for a revision the
    /// history lacks.
    pub fn document_at(&mut self, revision: &Revision) -> Result<(Document, Style), HistoryError> {
        if *revision == self.revision() {
            return Ok((self.doc.clone(), self.style.clone()));
        }
        self.read_at(revision).map(|(doc, _, style)| (doc, style))
    }

    fn read_at(&self, revision: &Revision) -> Result<(Document, Vec<Entry>, Style), HistoryError> {
        self.look(Some(revision), |history| {
            let (doc, index) = crdt::read(&history.loro, &history.body);
            (doc, index, crdt::read_style(&history.loro))
        })
    }

    /// What differs between two revisions, as a change from `from` to `to` would tell it: the blocks that
    /// read differently, came or went, and whether the style or the document's own fields differ.
    pub fn changed(&mut self, from: &Revision, to: &Revision) -> Result<Change, HistoryError> {
        let ((old, was), (new, is)) = (self.document_at(from)?, self.document_at(to)?);
        let mut change = diff(&old, &new);
        change.document = fields(&old) != fields(&new);
        change.style = was != is;
        Ok(change)
    }

    /// Marks `revision` as a version: a point to list in the history, and to keep however the history is
    /// thinned. It carries this writer's name and the time the shell last told. A revision holds one
    /// mark, and marking it again replaces the one it has.
    ///
    /// A mark is part of the history, so it merges and travels with it. It is no step of the timeline
    /// and no undo step, and it can be made while an older revision is shown.
    pub fn tag(&mut self, revision: &Revision, kind: TagKind, name: Option<&str>) -> Result<(), HistoryError> {
        self.mark(Tag { kind, name: name.map(str::to_owned), ..self.unnamed(revision) })
    }

    /// A mark of this writer's on `revision`, made now.
    fn unnamed(&self, revision: &Revision) -> Tag {
        let (at, author, time) = (revision.clone(), self.pen.author.clone(), self.pen.time);
        Tag { at, kind: TagKind::Named, name: None, author, time, restored: None }
    }

    fn mark(&mut self, tag: Tag) -> Result<(), HistoryError> {
        let loro = self.loro();
        loro.frontiers_to_vv(&tag.at.0).ok_or(HistoryError::UnknownRevision)?;
        // The marks are read and written where the history is newest.
        let shown = loro.is_detached().then(|| loro.state_frontiers());
        loro.checkout_to_latest();
        versions::mark(loro, &tag);
        self.commit();
        self.show(shown);
        Ok(())
    }

    /// Takes the mark off `revision`, which stays in the history like any other. Nothing happens when it
    /// has no mark.
    pub fn untag(&mut self, revision: &Revision) {
        let loro = self.loro();
        let shown = loro.is_detached().then(|| loro.state_frontiers());
        loro.checkout_to_latest();
        if versions::unmark(loro, revision) {
            self.commit();
        }
        self.show(shown);
    }

    /// The versions marked in the history, oldest first.
    pub fn tags(&mut self) -> Vec<Tag> {
        self.look(None, |history| versions::tags(&history.loro)).unwrap_or_default()
    }

    /// Makes the newest document and style read as they did at `revision`, and shows them. Nothing of
    /// the history is taken back: the restoring is one more edit of this writer's, and one undo step.
    /// The revision it leads to is marked as a [`TagKind::Restore`] that names `revision`.
    ///
    /// What others wrote since `revision` goes with the rest, and comes back with an undo.
    pub fn restore(&mut self, revision: &Revision) -> Result<Change, HistoryError> {
        let (doc, index, style) = self.read_at(revision)?;
        let mut change = self.checkout_latest();
        let from = self.revision();
        let mut step = Step::new();
        if doc.blocks != self.doc.blocks {
            let old = flat::order(&self.doc.blocks, &self.index);
            let delta = flat::rewrite(&old, &flat::order(&doc.blocks, &index));
            let (changed, undo) = self.shift(&delta, None);
            change.absorb(changed);
            debug_assert!(undo.is_some() && self.doc.blocks == doc.blocks, "a restore did not give its document");
            if let Some(undo) = undo
                && self.write(std::slice::from_ref(&delta), &mut change)
            {
                step.push(Edit::Text(undo));
            }
        }
        let mut assigns = crdt::meta_assigns(self.loro(), &self.doc, &doc);
        if style != self.style {
            assigns.extend(crdt::style_assigns(self.loro(), &style));
            self.styled = true;
        }
        step.insert(0, Edit::Set(self.set(&assigns, &mut change)));
        self.steps.record(step, false);
        let now = self.revision();
        if now != from {
            let tag = Tag { kind: TagKind::Restore, restored: Some(revision.clone()), ..self.unnamed(&now) };
            // The history holds the revision it has just come to.
            let _ = self.mark(tag);
        }
        Ok(change)
    }

    /// A position that follows its text, for anything kept across edits. `None` when `pos` is not a
    /// position in the document.
    pub fn stable(&self, pos: &Pos, hold: Hold) -> Option<StablePos> {
        let (spot, runs) = self.text(&pos.block).ok()?;
        let at = spot.text() + chars_before(runs, pos.offset)?;
        let id = |at: usize| self.body().get_cursor(at, Side::Middle)?.id;
        Some(match hold {
            Hold::Prev if at > 0 => StablePos { id: Some(id(at - 1)?), after: true },
            _ if at < self.start(self.index.len()) => StablePos { id: Some(id(at)?), after: false },
            _ => StablePos { id: None, after: false },
        })
    }

    /// Where a stable position is now. `None` when it no longer sits in text.
    pub fn resolve(&self, stable: &StablePos) -> Option<Pos> {
        let at = match stable.id {
            None => self.start(self.index.len()),
            Some(id) => {
                let found =
                    self.loro().get_cursor_pos(&Cursor::new(Some(id), self.body().id(), Side::Middle, 0)).ok()?;
                // A character that is gone leaves the position where it stood.
                found.current.pos + usize::from(stable.after && found.update.is_none())
            }
        };
        let mut start = 0;
        let (i, text) = self.index.iter().enumerate().find_map(|(i, entry)| {
            let text = start + usize::from(entry.marker);
            start += entry.len();
            (entry.shape == BodyShape::Text && (text..=text + entry.chars).contains(&at)).then_some((i, text))
        })?;
        let block = nth(&self.doc.blocks, &self.index, i)?;
        let mut chars = block.body.runs().iter().flat_map(|run| run.text.chars());
        let offset = chars.by_ref().take(at - text).map(char::len_utf8).sum();
        Some(Pos { block: block.id.clone(), offset })
    }

    fn queue(&self) -> MutexGuard<'_, Vec<Event>> {
        self.history().events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Brings the document up to date with what the history reported.
    fn drain(&mut self) -> Change {
        let (mut bodies, mut keys, mut fields) = (Vec::new(), Vec::new(), false);
        for event in mem::take(&mut *self.queue()) {
            match event {
                Event::Body(delta) => bodies.push(delta),
                Event::Markers(changed) => keys.extend(changed),
                Event::Meta | Event::Style => fields = true,
                // Nothing of it shows.
                Event::Settled(_) => {}
            }
        }
        let mut change = if fields { self.reread() } else { Change::default() };
        match bodies.as_slice() {
            [] => {
                self.follow(&crdt::restyles(self.loro(), self.body(), &keys, &HashSet::new()), &mut change);
            }
            [raw] => {
                let mut new = HashSet::new();
                let delta = crdt::lift(self.loro(), self.body(), raw, &mut new);
                let restyles = crdt::restyles(self.loro(), self.body(), &keys, &new);
                // A change that does not fit has the whole document read again, restyled markers and all.
                if self.follow(&delta, &mut change) {
                    self.follow(&restyles, &mut change);
                }
            }
            // Each was told against the sequence as it then stood, which only the last still is.
            _ => {
                self.steps.clear();
                change.absorb(self.resync());
            }
        }
        change
    }

    /// Makes the document and the undo steps follow a change the history reported. False when it did not
    /// fit, and the whole document was read again.
    fn follow(&mut self, delta: &[TextDelta], change: &mut Change) -> bool {
        if delta.is_empty() {
            return true;
        }
        let (changed, undo) = self.shift(delta, None);
        change.absorb(changed);
        match undo {
            // The undo steps were made for the sequence as it was before this.
            Some(_) => self.steps.rebase(delta),
            None => self.steps.clear(),
        }
        undo.is_some()
    }

    /// Reads the whole document from the history again.
    fn resync(&mut self) -> Change {
        let (doc, index) = crdt::read(self.loro(), self.body());
        let style = crdt::read_style(self.loro());
        let old = mem::replace(&mut self.doc, doc);
        (self.index, self.lookup) = (index, OnceLock::new());
        self.queue().clear();
        let mut change = diff(&old, &self.doc);
        change.document = fields(&old) != fields(&self.doc);
        change.style = mem::replace(&mut self.style, style) != self.style;
        change
    }

    /// Makes the document follow a change to the sequence. Says what that touched, and gives the change
    /// that takes it back: `None` when it did not fit and the document was read again from the history.
    ///
    /// `place` is the entry of the block the change is known to be made in, and where that block starts.
    fn shift(&mut self, delta: &[TextDelta], place: Option<Seat>) -> (Change, Option<Delta>) {
        if let Some(done) = self.shift_text(delta, place) {
            return done;
        }
        let before: Vec<(u64, usize)> = self.index.iter().map(|entry| (entry.hash, entry.level)).collect();
        let flat = flat::flatten(mem::take(&mut self.doc.blocks), mem::take(&mut self.index));
        let mut undo = Script::default();
        let Some(moved) = flat::transform(flat, delta, &mut undo) else {
            // The tree was taken apart for nothing, so every block is reported.
            let mut change = self.resync();
            change.structure = true;
            return (change, None);
        };
        let built = flat::build(moved.blocks);
        (self.doc.blocks, self.index, self.lookup) = (built.blocks, built.index, OnceLock::new());
        // Blocks came, went or moved exactly when the ids or their nesting read differently in order.
        let structure = !before.iter().copied().eq(self.index.iter().map(|entry| (entry.hash, entry.level)));
        let blocks = built.dirty.into_iter().chain(moved.gone).collect();
        (Change { blocks, structure, ..Change::default() }, Some(undo.finish()))
    }

    /// The common case of [`Model::shift`]: a change inside one block's text edits that block in place.
    fn shift_text(&mut self, delta: &[TextDelta], place: Option<Seat>) -> Option<(Change, Option<Delta>)> {
        let (first, last) = span(delta)?;
        let holds = |entry: &Entry, start: usize| {
            entry.marker && entry.shape == BodyShape::Text && start < first && last <= start + entry.len()
        };
        let (i, start) = place.filter(|(i, start)| holds(&self.index[*i], *start)).or_else(|| {
            let mut start = 0;
            let i = self.index.iter().position(|entry| {
                let hit = holds(entry, start);
                start += usize::from(!hit) * entry.len();
                hit
            })?;
            Some((i, start))
        })?;
        let block = nth_mut(&mut self.doc.blocks, &self.index, i)?;
        let Body::Text(runs) = &mut block.body else { return None };

        // The block is read alone, as if the sequence began at its marker.
        let alone = Block { body: Body::Text(mem::take(runs)), ..Block::new(block.id.clone(), BlockKind::Paragraph) };
        let alone = Flat { block: alone, entry: self.index[i].clone(), dirty: false };
        let mut local = vec![retain(first - start)];
        local.extend(delta.iter().skip_while(|item| is_skip(item)).cloned());
        while local.last().is_some_and(is_skip) {
            local.pop();
        }
        let mut undo = Script::default();
        match flat::transform(vec![alone], &local, &mut undo).map(|moved| moved.blocks).as_deref_mut() {
            Some([Flat { block: Block { body: Body::Text(edited), .. }, entry, .. }]) => {
                *runs = mem::take(edited);
                self.index[i].chars = entry.chars;
                let mut undo = undo.finish();
                // Read alone, the block's change starts at the block. In the sequence it starts before.
                if let Some(TextDelta::Retain { retain, .. }) = undo.first_mut() {
                    *retain += start;
                }
                Some((Change { blocks: BTreeSet::from([block.id.clone()]), ..Change::default() }, Some(undo)))
            }
            _ => {
                let mut change = self.resync();
                change.structure = true;
                Some((change, None))
            }
        }
    }

    /// Where entry `i` starts in the sequence, in characters.
    fn start(&self, i: usize) -> usize {
        self.index[..i].iter().map(Entry::len).sum()
    }

    /// The entry after block `i` and everything nested in it.
    fn after(&self, i: usize) -> usize {
        let level = self.index[i].level;
        (i + 1..self.index.len()).find(|&next| self.index[next].level <= level).unwrap_or(self.index.len())
    }

    /// Finds a block by id in one pass over the entries, which also counts up to where it starts.
    fn spot(&self, id: &BlockId) -> Result<Spot<'_>, OpError> {
        let hash = flat::hash(id);
        let mut start = 0;
        for (i, entry) in self.index.iter().enumerate() {
            if entry.hash == hash
                && let Some(block) = nth(&self.doc.blocks, &self.index, i)
                && block.id == *id
            {
                return Ok(Spot { i, start, block, entry });
            }
            start += entry.len();
        }
        Err(OpError::NoSuchBlock(id.clone()))
    }

    fn text(&self, id: &BlockId) -> Result<(Spot<'_>, &[Run]), OpError> {
        let spot = self.spot(id)?;
        match &spot.block.body {
            Body::Text(runs) => Ok((spot, runs)),
            _ => Err(OpError::WrongBody(id.clone())),
        }
    }

    /// Turns an operation into the change it makes to the sequence, or says why it cannot be made.
    ///
    /// An operation on one block's text also names that block's place, which saves looking for it again.
    fn compile(&self, op: &Op) -> Result<(Delta, Option<Seat>), OpError> {
        let mut delta = Vec::new();
        let mut place = None;
        match op {
            Op::InsertText { at, text } => {
                let (spot, runs) = self.text(&at.block)?;
                let chars = chars_before(runs, at.offset).ok_or_else(|| bad(&at.block, at.offset))?;
                place = Some((spot.i, spot.start));
                if !text.is_empty() {
                    let mut end = 0;
                    let run = runs.iter().position(|run| {
                        end += run.text.len();
                        at.offset <= end
                    });
                    let mut format = run.map(|run| attrs(&runs[run])).unwrap_or_default();
                    // Text is part of a reference only when it is typed between two characters of it.
                    let target = |run: &Run| run.marks.iter().find(|mark| matches!(mark, Mark::Ref(_))).cloned();
                    let after = run.filter(|_| at.offset == end).and_then(|run| runs.get(run + 1));
                    let goes_on = run.zip(after).is_some_and(|(run, after)| target(&runs[run]) == target(after));
                    if at.offset == 0 || (at.offset == end && !goes_on) {
                        format.retain(|(key, _)| key != crdt::REF);
                    }
                    spot.reach(chars, &mut delta);
                    delta.push(insert(&crdt::clean(text), format));
                }
            }
            Op::DeleteText { block, range } => {
                let (spot, runs) = self.text(block)?;
                let chars = char_range(runs, block, range)?;
                place = Some((spot.i, spot.start));
                if !chars.is_empty() {
                    spot.reach(chars.start, &mut delta);
                    delta.push(TextDelta::Delete { delete: chars.len() });
                }
            }
            Op::SetMark { block, range, mark, on } => {
                let (key, value) = crdt::mark_attr(mark);
                let value = if *on { value } else { LoroValue::Null };
                let set = |run: &Run| run.marks.contains(mark) == *on;
                place = Some(self.restyle(block, range, key, value, set, &mut delta)?);
            }
            Op::SetLink { block, range, link } => {
                let value = link.as_deref().map_or(LoroValue::Null, Into::into);
                let set = |run: &Run| run.link == *link;
                place = Some(self.restyle(block, range, crdt::LINK.into(), value, set, &mut delta)?);
            }
            Op::SetLang { block, range, lang } => {
                let lang = lang.as_deref().filter(|lang| !lang.is_empty());
                let set = |run: &Run| run.lang.as_deref() == lang;
                place = Some(self.restyle(block, range, crdt::LANG.into(), crdt::lang_value(lang), set, &mut delta)?);
            }
            Op::Split { at, new_id } => {
                if self.spot(new_id).is_ok() {
                    return Err(OpError::DuplicateId(new_id.clone()));
                }
                let (spot, runs) = self.text(&at.block)?;
                let chars = chars_before(runs, at.offset).ok_or_else(|| bad(&at.block, at.offset))?;
                spot.reach(chars, &mut delta);
                delta.push(marker(&Block::new(new_id.clone(), spot.block.kind.clone()), spot.entry.level));
            }
            Op::Join { block } => {
                let (spot, _) = self.text(block)?;
                let before = spot.i.checked_sub(1).map(|i| &self.index[i]);
                if !spot.entry.marker || before.is_none_or(|entry| entry.shape != BodyShape::Text) {
                    return Err(OpError::NothingToJoin(block.clone()));
                }
                // Without its marker, the block's text is the end of the text before it.
                seek(&mut delta, spot.start);
                delta.push(TextDelta::Delete { delete: 1 });
            }
            Op::SetKind { block, kind } => {
                let spot = self.spot(block)?;
                if matches!(kind, BlockKind::Unknown { .. }) || spot.block.kind.shape() != kind.shape() {
                    return Err(OpError::WrongBody(block.clone()));
                }
                if spot.block.kind == *kind {
                    return Ok((delta, None));
                }
                if !kind.sound() {
                    return Err(OpError::BadKind(block.clone()));
                }
                // The block must still fit where it sits, and what it holds must fit it.
                let parent = (spot.entry.level > 0).then(|| nth(&self.doc.blocks, &self.index, spot.entry.parent));
                let fits = parent.flatten().is_none_or(|parent| parent.kind.admits(kind));
                if !fits || !children(spot.block).iter().all(|child| kind.admits(&child.kind)) {
                    return Err(OpError::WrongBody(block.clone()));
                }
                self.rehead(&spot, Block { kind: kind.clone(), ..spot.block.clone_bare() }, &mut delta);
            }
            Op::SetBlockProps { block, page_break, lang } => {
                let spot = self.spot(block)?;
                let lang = lang.clone().filter(|lang| !lang.is_empty());
                if (spot.block.page_break, &spot.block.lang) == (*page_break, &lang) {
                    return Ok((delta, None));
                }
                self.rehead(&spot, Block { page_break: *page_break, lang, ..spot.block.clone_bare() }, &mut delta);
            }
            Op::InsertBlock { parent, index, block } => {
                let mut ids = BTreeSet::new();
                let mut stack = vec![block];
                while let Some(block) = stack.pop() {
                    if self.spot(&block.id).is_ok() || !ids.insert(&block.id) {
                        return Err(OpError::DuplicateId(block.id.clone()));
                    }
                    if !block.kind.sound() {
                        return Err(OpError::BadKind(block.id.clone()));
                    }
                    if !children(block).iter().all(|child| block.kind.admits(&child.kind)) {
                        return Err(OpError::WrongBody(block.id.clone()));
                    }
                    stack.extend(children(block));
                }
                let (siblings, depth, end) = match parent {
                    None => (self.doc.blocks.as_slice(), 0, self.index.len()),
                    Some(parent) => {
                        let spot = self.spot(parent)?;
                        let Body::Blocks(children) = &spot.block.body else {
                            return Err(OpError::WrongBody(parent.clone()));
                        };
                        if !spot.block.kind.admits(&block.kind) {
                            return Err(OpError::WrongBody(parent.clone()));
                        }
                        (children.as_slice(), spot.entry.level + 1, self.after(spot.i))
                    }
                };
                let at = match siblings.get(*index) {
                    Some(next) => self.spot(&next.id)?.i,
                    None if *index == siblings.len() => end,
                    None => return Err(OpError::BadIndex { index: *index }),
                };
                seek(&mut delta, self.start(at));
                spell(block, depth, &mut delta);
                self.seat(at, &mut delta);
            }
            Op::RemoveBlock { block } => {
                let spot = self.spot(block)?;
                let end = self.after(spot.i);
                seek(&mut delta, spot.start);
                delta.push(TextDelta::Delete { delete: self.index[spot.i..end].iter().map(Entry::len).sum() });
                self.seat(end, &mut delta);
            }
            // These change no block. `apply` makes them itself.
            Op::SetStyle { .. } | Op::SetPage { .. } | Op::ReplaceStyle { .. } => {}
            Op::SetLanguage { .. } | Op::AddWord { .. } | Op::RemoveWord { .. } | Op::SetProperties { .. } => {}
        }
        Ok((delta, place))
    }

    /// Changes what a block is, leaving what it holds. `changed` is the block without its body.
    fn rehead(&self, spot: &Spot, changed: Block, delta: &mut Vec<TextDelta>) {
        seek(delta, spot.start);
        if spot.entry.marker {
            delta.push(style(1, BLOCK.into(), head(&changed).into()));
        } else {
            delta.push(marker(&changed, spot.entry.level));
        }
    }

    /// Sets one attribute over a range of a block's text. Text that `set` says has it already is left
    /// alone, so formatting that changes nothing is no edit.
    fn restyle(
        &self,
        block: &BlockId,
        range: &Range<usize>,
        key: String,
        value: LoroValue,
        set: impl Fn(&Run) -> bool,
        delta: &mut Vec<TextDelta>,
    ) -> Result<Seat, OpError> {
        let (spot, runs) = self.text(block)?;
        let chars = char_range(runs, block, range)?;
        // Where the change has got to in the block's text. `None` until it has somewhere to start.
        let mut reached = None;
        let mut start = 0;
        for run in runs {
            let end = start + run.text.chars().count();
            let (from, to) = (chars.start.max(start), chars.end.min(end));
            if from < to && !set(run) {
                match reached {
                    None => spot.reach(from, delta),
                    Some(at) => seek(delta, from - at),
                }
                delta.push(style(to - from, key.clone(), value.clone()));
                reached = Some(to);
            }
            start = end;
        }
        Ok((spot.i, spot.start))
    }

    /// Gives the text at entry `at` a marker if it has none, when the block before it is about to change.
    /// Otherwise that text would join whatever comes to stand before it.
    fn seat(&self, at: usize, delta: &mut Vec<TextDelta>) {
        if let Some(entry) = self.index.get(at).filter(|entry| !entry.marker)
            && let Some(block) = nth(&self.doc.blocks, &self.index, at)
        {
            delta.push(marker(&block.clone_bare(), entry.level));
        }
    }
}

impl Block {
    /// The block without its text or children.
    pub(crate) fn clone_bare(&self) -> Block {
        Block {
            id: self.id.clone(),
            kind: self.kind.clone(),
            body: Body::None,
            lang: self.lang.clone(),
            page_break: self.page_break,
            extra: self.extra.clone(),
        }
    }
}

/// A block and its place in the sequence.
struct Spot<'a> {
    /// Its place in document order.
    i: usize,
    /// Where it starts in the sequence, in characters.
    start: usize,
    block: &'a Block,
    entry: &'a Entry,
}

impl Spot<'_> {
    /// Where the block's text starts.
    fn text(&self) -> usize {
        self.start + usize::from(self.entry.marker)
    }

    /// Moves to `chars` into the block's text. Text without a marker is given one on the way, so a block
    /// that an edit names cannot vanish with its last character.
    fn reach(&self, chars: usize, delta: &mut Vec<TextDelta>) {
        if self.entry.marker {
            seek(delta, self.text() + chars);
        } else {
            seek(delta, self.start);
            delta.push(marker(&self.block.clone_bare(), self.entry.level));
            seek(delta, chars);
        }
    }
}

/// The block an operation names, for one that names a block already there.
fn target(op: &mut Op) -> Option<&mut BlockId> {
    match op {
        Op::InsertText { at, .. } | Op::Split { at, .. } => Some(&mut at.block),
        Op::DeleteText { block, .. } | Op::SetMark { block, .. } | Op::SetLink { block, .. } => Some(block),
        Op::SetLang { block, .. } | Op::SetKind { block, .. } | Op::SetBlockProps { block, .. } => Some(block),
        Op::Join { block } | Op::RemoveBlock { block } => Some(block),
        Op::InsertBlock { parent, .. } => parent.as_mut(),
        _ => None,
    }
}

/// Which blocks the ops of one batch mean by the ids of blocks that share one. Such blocks go by a number
/// each, counted from where they stand, so taking one away renames those after it. A batch names them as
/// they stood before it.
#[derive(Default)]
struct Copies {
    /// For each op, the id its block is stored under, and which of the blocks with that id it is, from 1.
    named: Vec<Option<(BlockId, usize)>>,
    /// Those the batch has taken away so far.
    gone: Vec<(BlockId, usize)>,
}

impl Copies {
    fn of(model: &Model, ops: &mut [Op]) -> Self {
        // A batch that names no numbered block has nothing to keep track of.
        let mut numbered = |op: &mut Op| target(op).is_some_and(|id| id.as_str().contains('~'));
        if ops.len() < 2 || !ops.iter_mut().any(&mut numbered) {
            return Self::default();
        }
        let copy = |id: &mut BlockId| {
            let stored = &model.index[model.find(id)?].stored;
            let number = match id.as_str().strip_prefix(stored.as_str())? {
                "" => 1,
                rest => rest.strip_prefix('~')?.parse().ok()?,
            };
            Some((stored.clone(), number))
        };
        Self { named: ops.iter_mut().map(|op| target(op).and_then(copy)).collect(), gone: Vec::new() }
    }

    /// What the block of the `nth` op goes by now, if no longer by what the op calls it.
    fn now(&self, nth: usize) -> Option<BlockId> {
        let (stored, number) = self.named.get(nth)?.as_ref()?;
        let earlier = self.gone.iter().filter(|(id, gone)| id == stored && gone < number).count();
        let there = !self.gone.iter().any(|(id, gone)| id == stored && gone == number);
        (earlier > 0 && there).then(|| match number - earlier {
            1 => stored.clone(),
            now => BlockId::new(format!("{stored}~{now}")),
        })
    }

    /// Notes that the `nth` op took its block away.
    fn took(&mut self, nth: usize) {
        self.gone.extend(self.named.get(nth).cloned().flatten());
    }
}

/// Whether an operation changes the document's own fields or its style, and no block.
fn is_setting(op: &Op) -> bool {
    let style = matches!(op, Op::SetStyle { .. } | Op::SetPage { .. } | Op::ReplaceStyle { .. });
    let fields = matches!(op, Op::SetLanguage { .. } | Op::AddWord { .. } | Op::RemoveWord { .. });
    style || fields || matches!(op, Op::SetProperties { .. })
}

/// Makes such an operation, on copies of the fields and the style.
fn set(op: &Op, fields: &mut Document, style: &mut Style) -> Result<(), OpError> {
    match op {
        Op::SetLanguage { language } => fields.language.clone_from(language),
        Op::AddWord { word } => drop(fields.dictionary.insert(word.clone())),
        Op::RemoveWord { word } => drop(fields.dictionary.remove(word)),
        Op::SetProperties { properties } => fields.properties.clone_from(properties),
        Op::SetStyle { element, style: text } => drop(style.types.insert(element.clone(), text.clone())),
        Op::SetPage { page } => style.page.clone_from(page),
        Op::ReplaceStyle { style: whole } => style.clone_from(whole),
        _ => {}
    }
    if matches!(op, Op::SetStyle { .. } | Op::SetPage { .. } | Op::ReplaceStyle { .. }) {
        // The file format is the judge of what a style may hold.
        Style::from_json(&style.to_json()).map_err(|error| OpError::BadStyle(error.to_string()))?;
        *style = crdt::normal_style(style);
    }
    crdt::normal_meta(fields);
    Ok(())
}

/// The document's own fields that a [`Change`] speaks of.
fn fields(doc: &Document) -> (&String, &BTreeSet<String>, &Option<Properties>) {
    (&doc.language, &doc.dictionary, &doc.properties)
}

/// The one change that two make together, when the first takes text out and the second puts text in at
/// the same place.
fn replacement(cut: &[TextDelta], put: &[TextDelta]) -> Option<Delta> {
    let (TextDelta::Delete { .. }, lead) = cut.split_last()? else { return None };
    let text = put.strip_prefix(lead).filter(|text| !text.is_empty() && lead.iter().all(is_skip))?;
    let plain = |item: &TextDelta| match item {
        TextDelta::Insert { attributes, .. } => attributes.as_ref().is_none_or(|attrs| !attrs.contains_key(BLOCK)),
        _ => false,
    };
    text.iter().all(plain).then(|| cut.iter().chain(text).cloned().collect())
}

fn unreadable(error: impl fmt::Display) -> HistoryError {
    HistoryError::Unreadable(error.to_string())
}

fn bad(block: &BlockId, offset: usize) -> OpError {
    OpError::BadOffset { block: block.clone(), offset }
}

fn retain(retain: usize) -> TextDelta {
    TextDelta::Retain { retain, attributes: None }
}

fn seek(delta: &mut Vec<TextDelta>, chars: usize) {
    if chars > 0 {
        delta.push(retain(chars));
    }
}

fn style(retain: usize, key: String, value: LoroValue) -> TextDelta {
    TextDelta::Retain { retain, attributes: Some([(key, value)].into_iter().collect()) }
}

/// The stretch of the sequence a change touches, if it changes nothing but text.
fn span(delta: &[TextDelta]) -> Option<(usize, usize)> {
    let (mut at, mut first, mut last) = (0, None, 0);
    for item in delta {
        match item {
            _ if is_skip(item) => {}
            TextDelta::Retain { retain, attributes } => {
                if attributes.as_ref().is_some_and(|attrs| attrs.contains_key(BLOCK) || attrs.contains_key(DEPTH)) {
                    return None;
                }
                first.get_or_insert(at);
                last = at + retain;
            }
            TextDelta::Insert { attributes, .. } => {
                if attributes.as_ref().is_some_and(|attrs| attrs.contains_key(BLOCK)) {
                    return None;
                }
                first.get_or_insert(at);
                last = last.max(at);
            }
            TextDelta::Delete { delete } => {
                first.get_or_insert(at);
                last = at + delete;
            }
        }
        at += match item {
            TextDelta::Retain { retain, .. } => *retain,
            TextDelta::Delete { delete } => *delete,
            TextDelta::Insert { .. } => 0,
        };
    }
    Some((first?, last))
}

/// The characters before byte `offset` of a block's text. `None` when that is not a position in it.
fn chars_before(runs: &[Run], offset: usize) -> Option<usize> {
    let (mut bytes, mut chars) = (0, 0);
    for run in runs {
        if offset <= bytes + run.text.len() {
            return Some(chars + run.text.get(..offset - bytes)?.chars().count());
        }
        bytes += run.text.len();
        chars += run.text.chars().count();
    }
    (offset == bytes).then_some(chars)
}

fn char_range(runs: &[Run], block: &BlockId, range: &Range<usize>) -> Result<Range<usize>, OpError> {
    if range.start > range.end {
        return Err(bad(block, range.start));
    }
    let start = chars_before(runs, range.start).ok_or_else(|| bad(block, range.start))?;
    let end = chars_before(runs, range.end).ok_or_else(|| bad(block, range.end))?;
    Ok(start..end)
}

/// What differs between two documents, for the rare case where one was read whole.
fn diff(old: &Document, new: &Document) -> Change {
    let mut change = Change::default();
    let ids = |doc: &Document| doc.walk().map(|b| b.id.clone()).collect::<Vec<_>>();
    change.structure = ids(old) != ids(new);
    for block in new.walk() {
        if old.block(&block.id) != Some(block) {
            change.blocks.insert(block.id.clone());
        }
    }
    for block in old.walk() {
        if new.block(&block.id).is_none() {
            change.blocks.insert(block.id.clone());
        }
    }
    change
}
