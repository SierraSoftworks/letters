//! The document as its text sequence holds it: a flat run of blocks, each a marker and the text up to the next.
//!
//! The tree in [`Document`](crate::Document) is derived from that order and each marker's depth. Every order
//! and every set of depths gives one valid tree, which is what lets concurrent edits merge without a repair
//! step: a replica never has to write anything to make a merged sequence readable.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasher, DefaultHasher, Hash, Hasher};
use std::mem;

use loro::{LoroValue, TextDelta};
use serde_json::Value;

use crate::crdt::{attrs, clean, format};
use crate::undo::Script;
use crate::{Block, BlockId, BlockKind, Body, BodyShape, Mark, Run};

/// The attribute a marker carries its block in, as the block's JSON without its content. The history keeps
/// it beside the sequence: see [`crate::crdt`].
pub(crate) const BLOCK: &str = "block";
/// How many containers a marker asks to sit inside. Absent at the top level.
pub(crate) const DEPTH: &str = "depth";
/// The character a marker is in the sequence: the record separator, which text never holds.
pub(crate) const MARKER: &str = "\u{1e}";

/// Containers nest no deeper than this. `document.json` cannot hold more, as its reader stops at 128 levels
/// of JSON and each block costs two.
pub(crate) const MAX_DEPTH: usize = 60;

pub(crate) type Attrs<S> = HashMap<String, LoroValue, S>;

/// What the sequence records about one block. The model keeps these beside the tree, in document order.
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    /// The id the marker carries. The block's own id differs when an earlier block already uses it.
    pub stored: BlockId,
    /// The nesting the marker asks for.
    pub depth: usize,
    /// The nesting the block has, once fitted under the containers before it.
    pub level: usize,
    /// False for text that follows a block which holds none. It shows as a paragraph with a derived id.
    pub marker: bool,
    pub shape: BodyShape,
    /// Characters of text, without the marker.
    pub chars: usize,
    /// [`hash`] of the block's own id, to find a block by id without reading every id.
    pub hash: u64,
    /// The entry of the container the block sits in. Unused at the top level.
    pub parent: usize,
    /// The block's place among its siblings.
    pub nth: usize,
}

impl Entry {
    fn new(stored: BlockId, depth: usize, marker: bool, shape: BodyShape) -> Self {
        Self { stored, depth, level: 0, marker, shape, chars: 0, hash: 0, parent: 0, nth: 0 }
    }

    /// Characters the block takes in the sequence.
    pub fn len(&self) -> usize {
        self.chars + usize::from(self.marker)
    }
}

/// A number that differs for ids that differ, as near as makes no difference.
pub(crate) fn hash(id: &BlockId) -> u64 {
    let mut hasher = DefaultHasher::new();
    id.as_str().hash(&mut hasher);
    hasher.finish()
}

/// The block that entry `i` of `index` describes, reached by the turns its entry and its parents' give.
pub(crate) fn nth<'a>(blocks: &'a [Block], index: &[Entry], i: usize) -> Option<&'a Block> {
    let (path, len) = path(index, i)?;
    let (last, above) = path[..len].split_last()?;
    let mut blocks = blocks;
    for turn in above {
        blocks = blocks.get(*turn)?.body.blocks();
    }
    blocks.get(*last)
}

pub(crate) fn nth_mut<'a>(blocks: &'a mut [Block], index: &[Entry], i: usize) -> Option<&'a mut Block> {
    let (path, len) = path(index, i)?;
    let (last, above) = path[..len].split_last()?;
    let mut blocks = blocks;
    for turn in above {
        let Body::Blocks(children) = &mut blocks.get_mut(*turn)?.body else { return None };
        blocks = children;
    }
    blocks.get_mut(*last)
}

/// Which sibling to take at each level, from the top down, to reach the block of entry `i`.
fn path(index: &[Entry], i: usize) -> Option<([usize; MAX_DEPTH + 1], usize)> {
    let mut path = [0; MAX_DEPTH + 1];
    let len = index.get(i)?.level + 1;
    let mut at = i;
    for turn in path.get_mut(..len)?.iter_mut().rev() {
        let entry = index.get(at)?;
        (*turn, at) = (entry.nth, entry.parent);
    }
    Some((path, len))
}

/// One block out of the tree: its children are the blocks that follow it.
pub(crate) struct Flat {
    pub block: Block,
    pub entry: Entry,
    /// Whether the block may differ from the one the tree held.
    pub dirty: bool,
}

impl Flat {
    fn marker<S: BuildHasher>(head: &str, attrs: &Attrs<S>) -> Self {
        let block = read_head(head);
        let depth = attrs.get(DEPTH).map_or(0, read_depth);
        let entry = Entry::new(block.id.clone(), depth, true, block.kind.shape());
        Self { block, entry, dirty: true }
    }

    /// A paragraph for text that has no marker. [`build`] names it after the block before it.
    fn orphan() -> Self {
        let block = Block::new("", BlockKind::Paragraph);
        let entry = Entry::new(block.id.clone(), 0, false, BodyShape::Text);
        Self { block, entry, dirty: true }
    }
}

/// The JSON a marker carries for `block`.
pub(crate) fn head(block: &Block) -> String {
    block.clone_bare().to_value().to_string()
}

/// The insert that puts text into the sequence with exactly these attributes.
pub(crate) fn insert(text: &str, attrs: Vec<(String, LoroValue)>) -> TextDelta {
    let attributes = (!attrs.is_empty()).then(|| attrs.into_iter().collect());
    TextDelta::Insert { insert: text.to_owned(), attributes }
}

/// The attributes that make a character the marker of `block`.
fn marker_attrs(block: &Block, depth: usize) -> Vec<(String, LoroValue)> {
    let mut attrs = vec![(BLOCK.to_owned(), head(block).into())];
    if depth > 0 {
        attrs.push((DEPTH.to_owned(), (depth as i64).into()));
    }
    attrs
}

/// The insert that puts a block's marker into the sequence.
pub(crate) fn marker(block: &Block, depth: usize) -> TextDelta {
    insert(MARKER, marker_attrs(block, depth))
}

/// Reads a marker's JSON. One this build cannot read still has to show, so it becomes a paragraph.
fn read_head(head: &str) -> Block {
    let value: Option<Value> = serde_json::from_str(head).ok();
    let block = value.clone().and_then(|value| Block::from_value(value, "").ok());
    let block = block.unwrap_or_else(|| {
        let id = value.as_ref().and_then(|value| value["id"].as_str()).unwrap_or_default();
        Block::new(id, BlockKind::Paragraph)
    });
    // The marker's text and children follow it in the sequence, never inside its JSON.
    Block { body: Block::new("", block.kind.clone()).body, ..block }
}

fn read_depth(value: &LoroValue) -> usize {
    match value {
        LoroValue::I64(depth) => usize::try_from(*depth).unwrap_or(0).min(MAX_DEPTH),
        _ => 0,
    }
}

/// Takes the tree apart into document order. `index` must be the entries [`build`] gave for it.
pub(crate) fn flatten(blocks: Vec<Block>, index: Vec<Entry>) -> Vec<Flat> {
    fn walk(blocks: Vec<Block>, index: &mut std::vec::IntoIter<Entry>, out: &mut Vec<Flat>) {
        for mut block in blocks {
            let children = match &mut block.body {
                Body::Blocks(children) => mem::take(children),
                _ => Vec::new(),
            };
            let Some(entry) = index.next() else { return };
            out.push(Flat { block, entry, dirty: false });
            walk(children, index, out);
        }
    }
    let mut out = Vec::with_capacity(index.len());
    walk(blocks, &mut index.into_iter(), &mut out);
    out
}

pub(crate) struct Built {
    pub blocks: Vec<Block>,
    pub index: Vec<Entry>,
    /// The ids of the blocks that may have changed, and the ids that blocks no longer go by.
    pub dirty: Vec<BlockId>,
}

/// Puts flat blocks back into a tree. This is where every sequence is given its one reading:
///
/// - A block sits as deep as its marker asks, but no deeper than the containers open before it allow.
/// - An id that an earlier block uses gets a suffix, so ids stay unique whatever was merged.
/// - Text without a marker becomes a paragraph named after the block before it.
pub(crate) fn build(flat: Vec<Flat>) -> Built {
    /// Puts a finished block into the container it belongs to, and says where among its siblings.
    fn place(open: &mut [(Block, usize)], roots: &mut Vec<Block>, block: Block) -> usize {
        let siblings = match open.last_mut() {
            Some((Block { body: Body::Blocks(children), .. }, _)) => children,
            _ => roots,
        };
        siblings.push(block);
        siblings.len() - 1
    }

    let mut built = Built { blocks: Vec::new(), index: Vec::with_capacity(flat.len()), dirty: Vec::new() };
    let mut seen = HashSet::with_capacity(flat.len());
    // The stored id of the block before, and the depth that text straight after it takes.
    let mut before: Option<(BlockId, usize)> = None;
    // The containers still taking children, each with its entry.
    let mut open: Vec<(Block, usize)> = Vec::new();

    for Flat { mut block, mut entry, dirty } in flat {
        if !entry.marker {
            (entry.stored, entry.depth) = match &before {
                Some((id, depth)) => (BlockId::new(format!("{id}~")), *depth),
                None => (BlockId::new("~"), 0),
            };
        }
        before = Some((entry.stored.clone(), entry.depth + usize::from(entry.shape == BodyShape::Blocks)));

        let mut id = entry.stored.clone();
        let mut copy = 1;
        while !seen.insert(id.clone()) {
            copy += 1;
            id = BlockId::new(format!("{}~{copy}", entry.stored));
        }
        let renamed = block.id != id;
        if renamed && !block.id.as_str().is_empty() {
            built.dirty.push(block.id.clone());
        }
        if dirty || renamed {
            entry.hash = hash(&id);
            built.dirty.push(id.clone());
        }
        block.id = id;

        entry.level = entry.depth.min(open.len()).min(MAX_DEPTH);
        while open.len() > entry.level {
            let (closed, at) = open.pop().expect("the loop runs while a container is open");
            built.index[at].nth = place(&mut open, &mut built.blocks, closed);
        }
        entry.parent = open.last().map_or(0, |(_, at)| *at);
        let at = built.index.len();
        let container = entry.shape == BodyShape::Blocks;
        built.index.push(entry);
        if container {
            open.push((block, at));
        } else {
            built.index[at].nth = place(&mut open, &mut built.blocks, block);
        }
    }
    while let Some((closed, at)) = open.pop() {
        built.index[at].nth = place(&mut open, &mut built.blocks, closed);
    }
    built
}

/// The blocks a container holds, for a block whose kind is one.
pub(crate) fn children(block: &Block) -> &[Block] {
    if block.kind.shape() == BodyShape::Blocks { block.body.blocks() } else { &[] }
}

/// Writes a block as the sequence holds it: its marker, then its text.
fn spell_one(block: &Block, depth: usize, delta: &mut Vec<TextDelta>) {
    delta.push(marker(block, depth));
    if block.kind.shape() == BodyShape::Text {
        let runs = block.body.runs().iter().filter(|run| !run.text.is_empty());
        delta.extend(runs.map(|run| insert(&clean(&run.text), attrs(run))));
    }
}

/// Writes a block and everything in it as the sequence holds them.
pub(crate) fn spell(block: &Block, depth: usize, delta: &mut Vec<TextDelta>) {
    spell_one(block, depth, delta);
    children(block).iter().for_each(|child| spell(child, depth + 1, delta));
}

/// Whether a block, taken without its children, reads back from the sequence exactly as it stands.
fn canonical(block: &Block) -> bool {
    let kind = match &block.kind {
        BlockKind::Paragraph | BlockKind::ListItem { .. } | BlockKind::Code { .. } | BlockKind::Formula => true,
        BlockKind::Quote | BlockKind::Callout { .. } | BlockKind::Divider => true,
        BlockKind::Heading { level } => (1..=4).contains(level),
        // It may be a type this build knows under its real name.
        BlockKind::Unknown { .. } => false,
    };
    let body = match (&block.body, block.kind.shape()) {
        (Body::Text(runs), BodyShape::Text) => {
            let known = |run: &Run| !run.marks.iter().any(|mark| matches!(mark, Mark::Other(_)));
            let plain = |run: &Run| run.extra.is_empty() && run.lang.as_deref() != Some("") && known(run);
            let apart = runs.windows(2).all(|pair| !pair[0].same_format(&pair[1]));
            apart && runs.iter().all(|run| !run.text.is_empty() && !run.text.contains(MARKER) && plain(run))
        }
        (Body::Blocks(_), BodyShape::Blocks) | (Body::None, BodyShape::None) => true,
        _ => false,
    };
    let lang = block.lang.as_deref() != Some("");
    kind && body && lang && block.extra.is_empty() && !block.id.as_str().is_empty()
}

/// Reads a tree as the history would give it back had it been stored there: ids made unique, runs merged,
/// and whatever the sequence cannot hold put right. Every block has a marker that carries the id and the
/// depth it shows, so a history can be made from the result one block at a time.
pub(crate) fn normal(blocks: Vec<Block>) -> Built {
    fn walk(blocks: Vec<Block>, depth: usize, out: &mut Vec<Flat>) {
        for mut block in blocks {
            let inside = match &mut block.body {
                Body::Blocks(inside) if block.kind.shape() == BodyShape::Blocks => mem::take(inside),
                _ => Vec::new(),
            };
            if canonical(&block) {
                let mut entry = Entry::new(block.id.clone(), depth.min(MAX_DEPTH), true, block.kind.shape());
                entry.chars = block.body.runs().iter().map(|run| run.text.chars().count()).sum();
                out.push(Flat { block, entry, dirty: true });
            } else {
                // Anything else is spelled out and read, which is what a history does with it.
                let mut delta = Vec::new();
                spell_one(&block, depth, &mut delta);
                out.extend(
                    transform(Vec::new(), &delta, &mut Script::default()).map_or(Vec::new(), |read| read.blocks),
                );
            }
            walk(inside, depth + 1, out);
        }
    }
    fn settle<'a>(blocks: &[Block], entries: &mut impl Iterator<Item = &'a mut Entry>) {
        for block in blocks {
            let Some(entry) = entries.next() else { return };
            if entry.stored != block.id {
                entry.stored = block.id.clone();
            }
            (entry.depth, entry.marker) = (entry.level, true);
            settle(block.body.blocks(), entries);
        }
    }

    let mut flat = Vec::new();
    walk(blocks, 0, &mut flat);
    let mut built = build(flat);
    settle(&built.blocks, &mut built.index.iter_mut());
    built
}

/// A stretch of the sequence: one marker, or text in one format.
enum Piece {
    Marker(Flat),
    Text(Run),
}

impl Piece {
    fn chars(&self) -> usize {
        match self {
            Self::Marker(_) => 1,
            Self::Text(run) => run.text.chars().count(),
        }
    }

    /// The attributes the piece has in the sequence.
    fn attrs(&self) -> Vec<(String, LoroValue)> {
        match self {
            // The sequence names a block by the id it was given, which is not always the one it shows.
            Self::Marker(Flat { block, entry, .. }) => {
                marker_attrs(&Block { id: entry.stored.clone(), ..block.clone() }, entry.depth)
            }
            Self::Text(run) => attrs(run),
        }
    }

    /// The insert that would put this piece back.
    fn put_back(&self) -> TextDelta {
        let text = match self {
            Self::Marker(_) => MARKER,
            Self::Text(run) => &run.text,
        };
        insert(text, self.attrs())
    }

    /// The change that would give this piece back the values `new` is about to replace.
    fn keep<S: BuildHasher>(&self, new: &Attrs<S>) -> TextDelta {
        let mut old = self.attrs();
        let mut was = |key: &String| match old.iter().position(|(name, _)| name == key) {
            Some(at) => old.swap_remove(at).1,
            None => LoroValue::Null,
        };
        let attributes = new.keys().map(|key| (key.clone(), was(key))).collect();
        TextDelta::Retain { retain: self.chars(), attributes: Some(attributes) }
    }
}

/// Reads flat blocks as the characters they are in the sequence.
struct Reader {
    blocks: std::vec::IntoIter<Flat>,
    /// The pieces of the block being read.
    open: VecDeque<Piece>,
    /// The ids of the blocks read that had no marker. Their text may end up under another name.
    gone: Vec<BlockId>,
}

impl Reader {
    /// Breaks the next block into its marker and its runs.
    fn open_next(&mut self) -> Option<()> {
        let mut flat = self.blocks.next()?;
        let runs = match &mut flat.block.body {
            Body::Text(runs) => mem::take(runs),
            _ => Vec::new(),
        };
        if flat.entry.marker {
            flat.entry.chars = 0;
            self.open.push_back(Piece::Marker(flat));
        } else {
            self.gone.push(flat.block.id);
        }
        self.open.extend(runs.into_iter().map(Piece::Text));
        Some(())
    }

    /// Hands at most `n` characters to `f` as one piece, and says how many it was.
    fn piece(&mut self, n: usize, f: &mut dyn FnMut(Piece) -> Option<()>) -> Option<usize> {
        while self.open.is_empty() {
            self.open_next()?;
        }
        match self.open.pop_front()? {
            marker @ Piece::Marker(_) => f(marker).map(|()| 1),
            Piece::Text(mut run) => {
                let len = match run.text.char_indices().nth(n) {
                    Some((at, _)) => {
                        let tail = Run { text: run.text.split_off(at), ..run.clone() };
                        self.open.push_front(Piece::Text(tail));
                        n
                    }
                    None => run.text.chars().count(),
                };
                f(Piece::Text(run)).map(|()| len)
            }
        }
    }

    fn take(&mut self, mut n: usize, mut f: impl FnMut(Piece) -> Option<()>) -> Option<()> {
        while n > 0 {
            n -= self.piece(n, &mut f)?;
        }
        Some(())
    }

    /// Passes `n` characters through unchanged. A block that fits whole is moved, not rebuilt.
    fn copy(&mut self, mut n: usize, out: &mut Writer) -> Option<()> {
        while n > 0 {
            match self.blocks.as_slice().first() {
                Some(next) if self.open.is_empty() && next.entry.marker && next.entry.len() <= n => {
                    n -= next.entry.len();
                    out.blocks.extend(self.blocks.next());
                }
                _ => n -= self.piece(n, &mut |piece| out.push(piece))?,
            }
        }
        Some(())
    }

    /// Passes everything left through unchanged, and gives the ids noted in [`Reader::gone`].
    fn rest(mut self, out: &mut Writer) -> Vec<BlockId> {
        for piece in self.open.drain(..) {
            out.push(piece);
        }
        while let Some(next) = self.blocks.as_slice().first() {
            if next.entry.marker {
                out.blocks.extend(self.blocks.next());
            } else {
                self.open_next();
                for piece in self.open.drain(..) {
                    out.push(piece);
                }
            }
        }
        self.gone
    }
}

/// Collects pieces back into flat blocks.
#[derive(Default)]
struct Writer {
    blocks: Vec<Flat>,
}

impl Writer {
    fn push(&mut self, piece: Piece) -> Option<()> {
        match piece {
            Piece::Marker(mut flat) => {
                flat.dirty = true;
                self.blocks.push(flat);
            }
            Piece::Text(run) => self.text(run),
        }
        Some(())
    }

    /// Adds text to the block being written, or starts a paragraph when that block holds no text.
    fn text(&mut self, run: Run) {
        if run.text.is_empty() {
            return;
        }
        if !matches!(self.blocks.last(), Some(flat) if flat.entry.shape == BodyShape::Text) {
            self.blocks.push(Flat::orphan());
        }
        let Some(Flat { block: Block { body: Body::Text(runs), .. }, entry, dirty }) = self.blocks.last_mut() else {
            return;
        };
        *dirty = true;
        entry.chars += run.text.chars().count();
        match runs.last_mut() {
            Some(last) if last.same_format(&run) => last.text.push_str(&run.text),
            _ => runs.push(run),
        }
    }
}

/// Changes a piece's attributes. `None` when that would turn text into a marker or back, which no edit does.
fn restyle<S: BuildHasher>(piece: Piece, attrs: &Attrs<S>) -> Option<Piece> {
    match piece {
        Piece::Marker(mut flat) => {
            match attrs.get(BLOCK) {
                None => {}
                Some(LoroValue::String(head)) => {
                    flat.block = read_head(head);
                    flat.entry.stored = flat.block.id.clone();
                    flat.entry.shape = flat.block.kind.shape();
                }
                Some(_) => return None,
            }
            if let Some(depth) = attrs.get(DEPTH) {
                flat.entry.depth = read_depth(depth);
            }
            Some(Piece::Marker(flat))
        }
        Piece::Text(mut run) => {
            if matches!(attrs.get(BLOCK), Some(LoroValue::String(_))) {
                return None;
            }
            format(&mut run, attrs);
            Some(Piece::Text(run))
        }
    }
}

pub(crate) struct Moved {
    pub blocks: Vec<Flat>,
    /// The ids of blocks that may be gone: their marker was deleted or renamed, or they had none.
    pub gone: Vec<BlockId>,
}

/// Applies a change to the sequence, counted in characters, to the flat blocks. The change that takes it
/// back is written to `undo`.
///
/// `None` when the change does not fit them. The caller then reads the whole sequence again.
pub(crate) fn transform(flat: Vec<Flat>, delta: &[TextDelta], undo: &mut Script) -> Option<Moved> {
    let mut reader = Reader { blocks: flat.into_iter(), open: VecDeque::new(), gone: Vec::new() };
    let mut out = Writer::default();
    let mut gone = Vec::new();
    let mut note = |piece: &Piece| {
        if let Piece::Marker(flat) = piece {
            gone.push(flat.block.id.clone());
        }
    };
    for item in delta {
        match item {
            TextDelta::Retain { retain, attributes } => match attributes.as_ref().filter(|attrs| !attrs.is_empty()) {
                None => {
                    reader.copy(*retain, &mut out)?;
                    undo.retain(*retain);
                }
                Some(attrs) => reader.take(*retain, |piece| {
                    undo.push(piece.keep(attrs));
                    if attrs.contains_key(BLOCK) {
                        note(&piece);
                    }
                    out.push(restyle(piece, attrs)?)
                })?,
            },
            TextDelta::Insert { insert, attributes } => {
                undo.delete(insert.chars().count());
                let head = attributes.as_ref().and_then(|attrs| Some((attrs.get(BLOCK)?, attrs)));
                match (head, attributes) {
                    (Some((LoroValue::String(head), attrs)), _) => {
                        for _ in insert.chars() {
                            out.blocks.push(Flat::marker(head, attrs));
                        }
                    }
                    (_, attrs) => {
                        let mut run = Run::plain(insert.as_str());
                        if let Some(attrs) = attrs {
                            format(&mut run, attrs);
                        }
                        out.text(run);
                    }
                }
            }
            TextDelta::Delete { delete } => reader.take(*delete, |piece| {
                undo.push(piece.put_back());
                note(&piece);
                Some(())
            })?,
        }
    }
    gone.extend(reader.rest(&mut out));
    Some(Moved { blocks: out.blocks, gone })
}

/// The blocks of a tree in document order, each with its entry.
pub(crate) fn order<'a>(blocks: &'a [Block], index: &'a [Entry]) -> Vec<(&'a Block, &'a Entry)> {
    fn walk<'a>(blocks: &'a [Block], out: &mut Vec<&'a Block>) {
        for block in blocks {
            out.push(block);
            walk(block.body.blocks(), out);
        }
    }
    let mut out = Vec::with_capacity(index.len());
    walk(blocks, &mut out);
    out.into_iter().zip(index).collect()
}

/// The change that turns the sequence of one tree into that of another, block by block.
///
/// A block is kept when both trees hold its id and the kept blocks come in the same order in both. Only
/// what differs in a kept block is rewritten. `new` must be a tree as [`normal`] gives.
pub(crate) fn rewrite(old: &[(&Block, &Entry)], new: &[(&Block, &Entry)]) -> Vec<TextDelta> {
    let places: HashMap<&BlockId, usize> = old.iter().enumerate().map(|(at, (block, _))| (&block.id, at)).collect();
    let found: Vec<Option<usize>> = new.iter().map(|(block, _)| places.get(&block.id).copied()).collect();
    let mut out = Script::default();
    let (mut gone, mut come) = (0, 0);
    for (keep, kept) in rising(&found).into_iter().chain([(new.len(), old.len())]) {
        // Between two kept blocks, what the old tree held goes and what the new one holds comes.
        out.delete(old[gone..kept].iter().map(|(_, entry)| entry.len()).sum());
        for (block, entry) in &new[come..keep] {
            let mut spelled = Vec::new();
            spell_one(block, entry.level, &mut spelled);
            spelled.into_iter().for_each(|item| out.push(item));
        }
        if let (Some(was), Some(is)) = (old.get(kept), new.get(keep)) {
            reblock(*was, *is, &mut out);
        }
        (gone, come) = (kept + 1, keep + 1);
    }
    out.finish()
}

/// The longest run of blocks that stand in the same order in both trees, as pairs of places in the new
/// tree and the old. `found` gives each new block's place in the old tree.
fn rising(found: &[Option<usize>]) -> Vec<(usize, usize)> {
    // For each length of run, the new block that ends the run of that length soonest in the old tree.
    let mut ends: Vec<usize> = Vec::new();
    let mut before: Vec<Option<usize>> = vec![None; found.len()];
    for (new, old) in found.iter().enumerate() {
        let Some(old) = old else { continue };
        let longer = ends.partition_point(|end| found[*end].is_some_and(|end| end < *old));
        before[new] = longer.checked_sub(1).map(|shorter| ends[shorter]);
        match ends.get_mut(longer) {
            Some(end) => *end = new,
            None => ends.push(new),
        }
    }
    let mut run = Vec::with_capacity(ends.len());
    let mut next = ends.last().copied();
    while let Some(new) = next {
        run.extend(found[new].map(|old| (new, old)));
        next = before[new];
    }
    run.reverse();
    run
}

/// Changes one block of the sequence into another that keeps its place.
fn reblock((was, at): (&Block, &Entry), (is, to): (&Block, &Entry), out: &mut Script) {
    let fields = (&was.kind, &was.lang, was.page_break, &was.extra) == (&is.kind, &is.lang, is.page_break, &is.extra);
    let head = at.stored == is.id && at.depth == to.level && fields;
    if !at.marker {
        out.push(marker(is, to.level));
    } else if head {
        out.retain(1);
    } else {
        let mut attrs = marker_attrs(is, to.level);
        if to.level == 0 {
            attrs.push((DEPTH.to_owned(), LoroValue::Null));
        }
        out.push(TextDelta::Retain { retain: 1, attributes: Some(attrs.into_iter().collect()) });
    }

    let (old, new) = (was.body.runs(), if is.kind.shape() == BodyShape::Text { is.body.runs() } else { &[] });
    if old == new {
        return out.retain(at.chars);
    }
    // Each character with the run that formats it. What the two texts share at either end stays.
    let chars = |runs: &[Run]| -> Vec<(char, usize)> {
        runs.iter().enumerate().flat_map(|(run, Run { text, .. })| text.chars().map(move |c| (c, run))).collect()
    };
    let (a, b) = (chars(old), chars(new));
    let same = |(a, b): &(&(char, usize), &(char, usize))| a.0 == b.0 && old[a.1].same_format(&new[b.1]);
    let start = a.iter().zip(&b).take_while(same).count();
    let end = a[start..].iter().rev().zip(b[start..].iter().rev()).take_while(same).count();
    out.retain(start);
    out.delete(a.len() - start - end);
    for part in b[start..b.len() - end].chunk_by(|a, b| a.1 == b.1) {
        let text: String = part.iter().map(|(c, _)| c).collect();
        out.push(insert(&clean(&text), attrs(&new[part[0].1])));
    }
    out.retain(end);
}
