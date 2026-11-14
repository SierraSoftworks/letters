//! The one way to change a document.
//!
//! Every edit is a list of [`Op`]s applied as a unit: all of them land, or none do, and together they are one
//! undo step. This first backing keeps whole-document snapshots. A CRDT replaces it behind the same interface.

use std::collections::BTreeSet;
use std::fmt;
use std::ops::Range;

use crate::{Block, BlockId, BlockKind, Body, Document, Mark, Pos, Run};

/// One change to a document. Offsets are UTF-8 bytes into a text block and must sit on character boundaries.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Inserts text, which takes the marks of the character before it.
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
    /// Splits a text block in two. The text after `at` moves to a new block of the same kind, placed after it.
    Split {
        at: Pos,
        new_id: BlockId,
    },
    /// Appends a text block's content to the text block before it, then removes it.
    Join {
        block: BlockId,
    },
    /// Changes a block's kind to another kind that holds the same shape of body.
    SetKind {
        block: BlockId,
        kind: BlockKind,
    },
    /// Inserts a block at `index` among the children of `parent`, or at the top level.
    InsertBlock {
        parent: Option<BlockId>,
        index: usize,
        block: Block,
    },
    RemoveBlock {
        block: BlockId,
    },
}

/// Why an [`Op`] was refused. The document is left as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpError {
    NoSuchBlock(BlockId),
    /// The block does not hold what the operation needs (text, or child blocks).
    WrongBody(BlockId),
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
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchBlock(id) => write!(f, "there is no block \"{id}\""),
            Self::WrongBody(id) => write!(f, "block \"{id}\" does not hold what this operation changes"),
            Self::BadOffset { block, offset } => write!(f, "offset {offset} is not a position in block \"{block}\""),
            Self::BadIndex { index } => write!(f, "index {index} is past the end of the list"),
            Self::DuplicateId(id) => write!(f, "the id \"{id}\" is already in use"),
            Self::NothingToJoin(id) => write!(f, "there is no text block before \"{id}\" to join"),
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
}

/// A document and its undo history.
#[derive(Clone, Debug, Default)]
pub struct Model {
    doc: Document,
    undo: Vec<Document>,
    redo: Vec<Document>,
}

impl Model {
    pub fn new(doc: Document) -> Self {
        Self { doc, undo: Vec::new(), redo: Vec::new() }
    }

    pub fn doc(&self) -> &Document {
        &self.doc
    }

    /// Applies `ops` in order as one undo step.
    pub fn apply(&mut self, ops: impl IntoIterator<Item = Op>) -> Result<Change, OpError> {
        let before = self.doc.clone();
        let mut change = Change::default();
        for op in ops {
            if let Err(error) = apply(&mut self.doc, op, &mut change) {
                self.doc = before;
                return Err(error);
            }
        }
        if self.doc != before {
            self.undo.push(before);
            self.redo.clear();
        }
        Ok(change)
    }

    /// Steps back one edit. `None` when there is nothing to undo.
    pub fn undo(&mut self) -> Option<Change> {
        let doc = self.undo.pop()?;
        Some(self.swap(doc, false))
    }

    /// Reapplies the edit last undone. `None` when there is nothing to redo.
    pub fn redo(&mut self) -> Option<Change> {
        let doc = self.redo.pop()?;
        Some(self.swap(doc, true))
    }

    fn swap(&mut self, doc: Document, redo: bool) -> Change {
        let old = std::mem::replace(&mut self.doc, doc);
        let change = diff(&old, &self.doc);
        if redo {
            self.undo.push(old)
        } else {
            self.redo.push(old)
        }
        change
    }
}

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

fn apply(doc: &mut Document, op: Op, change: &mut Change) -> Result<(), OpError> {
    match op {
        Op::InsertText { at, text } => {
            let runs = text_of(doc, &at.block)?;
            insert_text(runs, at.offset, &text)
                .ok_or(OpError::BadOffset { block: at.block.clone(), offset: at.offset })?;
            change.blocks.insert(at.block);
        }
        Op::DeleteText { block, range } => {
            let runs = text_of(doc, &block)?;
            let (from, to) = span(runs, &block, &range)?;
            runs.drain(from..to);
            normalise(runs);
            change.blocks.insert(block);
        }
        Op::SetMark { block, range, mark, on } => {
            let runs = text_of(doc, &block)?;
            let (from, to) = span(runs, &block, &range)?;
            for run in &mut runs[from..to] {
                if on {
                    run.marks.insert(mark.clone());
                } else {
                    run.marks.remove(&mark);
                }
            }
            normalise(runs);
            change.blocks.insert(block);
        }
        Op::SetLink { block, range, link } => {
            let runs = text_of(doc, &block)?;
            let (from, to) = span(runs, &block, &range)?;
            for run in &mut runs[from..to] {
                run.link = link.clone();
            }
            normalise(runs);
            change.blocks.insert(block);
        }
        Op::Split { at, new_id } => {
            if doc.block(&new_id).is_some() {
                return Err(OpError::DuplicateId(new_id));
            }
            let runs = text_of(doc, &at.block)?;
            let index =
                cut(runs, at.offset).ok_or(OpError::BadOffset { block: at.block.clone(), offset: at.offset })?;
            let tail = runs.split_off(index);
            let (siblings, i) = locate(&mut doc.blocks, &at.block).expect("the block was just found");
            let mut new = Block::new(new_id.clone(), siblings[i].kind.clone());
            new.body = Body::Text(tail);
            siblings.insert(i + 1, new);
            change.blocks.extend([at.block, new_id]);
            change.structure = true;
        }
        Op::Join { block } => {
            let (siblings, i) = locate(&mut doc.blocks, &block).ok_or_else(|| OpError::NoSuchBlock(block.clone()))?;
            if !matches!(siblings[i].body, Body::Text(_)) {
                return Err(OpError::WrongBody(block));
            }
            if i == 0 || !matches!(siblings[i - 1].body, Body::Text(_)) {
                return Err(OpError::NothingToJoin(block));
            }
            let Body::Text(tail) = siblings.remove(i).body else { unreachable!() };
            let Body::Text(runs) = &mut siblings[i - 1].body else { unreachable!() };
            runs.extend(tail);
            normalise(runs);
            change.blocks.extend([siblings[i - 1].id.clone(), block]);
            change.structure = true;
        }
        Op::SetKind { block, kind } => {
            let (siblings, i) = locate(&mut doc.blocks, &block).ok_or_else(|| OpError::NoSuchBlock(block.clone()))?;
            if matches!(kind, BlockKind::Unknown { .. }) || siblings[i].kind.shape() != kind.shape() {
                return Err(OpError::WrongBody(block));
            }
            siblings[i].kind = kind;
            change.blocks.insert(block);
        }
        Op::InsertBlock { parent, index, block } => {
            let mut new_ids = BTreeSet::new();
            let mut stack = vec![&block];
            while let Some(b) = stack.pop() {
                if doc.block(&b.id).is_some() || !new_ids.insert(b.id.clone()) {
                    return Err(OpError::DuplicateId(b.id.clone()));
                }
                stack.extend(b.body.blocks());
            }
            let siblings = match &parent {
                None => &mut doc.blocks,
                Some(parent) => {
                    let (siblings, i) =
                        locate(&mut doc.blocks, parent).ok_or_else(|| OpError::NoSuchBlock(parent.clone()))?;
                    match &mut siblings[i].body {
                        Body::Blocks(children) => children,
                        _ => return Err(OpError::WrongBody(parent.clone())),
                    }
                }
            };
            if index > siblings.len() {
                return Err(OpError::BadIndex { index });
            }
            siblings.insert(index, block);
            change.blocks.extend(new_ids);
            change.structure = true;
        }
        Op::RemoveBlock { block } => {
            let (siblings, i) = locate(&mut doc.blocks, &block).ok_or_else(|| OpError::NoSuchBlock(block.clone()))?;
            let removed = siblings.remove(i);
            let mut stack = vec![&removed];
            while let Some(b) = stack.pop() {
                change.blocks.insert(b.id.clone());
                stack.extend(b.body.blocks());
            }
            change.structure = true;
        }
    }
    Ok(())
}

/// The list holding a block, and the block's index in it.
fn locate<'a>(blocks: &'a mut Vec<Block>, id: &BlockId) -> Option<(&'a mut Vec<Block>, usize)> {
    if let Some(i) = blocks.iter().position(|b| &b.id == id) {
        return Some((blocks, i));
    }
    blocks.iter_mut().find_map(|b| match &mut b.body {
        Body::Blocks(children) => locate(children, id),
        _ => None,
    })
}

fn text_of<'a>(doc: &'a mut Document, id: &BlockId) -> Result<&'a mut Vec<Run>, OpError> {
    let (siblings, i) = locate(&mut doc.blocks, id).ok_or_else(|| OpError::NoSuchBlock(id.clone()))?;
    match &mut siblings[i].body {
        Body::Text(runs) => Ok(runs),
        _ => Err(OpError::WrongBody(id.clone())),
    }
}

/// Makes `offset` a run boundary and returns the index of the run that starts there.
fn cut(runs: &mut Vec<Run>, offset: usize) -> Option<usize> {
    let mut start = 0;
    for i in 0..runs.len() {
        if offset == start {
            return Some(i);
        }
        let len = runs[i].text.len();
        if offset < start + len {
            let at = offset - start;
            if !runs[i].text.is_char_boundary(at) {
                return None;
            }
            let mut tail = runs[i].clone();
            tail.text = runs[i].text.split_off(at);
            runs.insert(i + 1, tail);
            return Some(i + 1);
        }
        start += len;
    }
    (offset == start).then_some(runs.len())
}

/// Cuts at both ends of `range` and returns the indices of the runs it covers.
fn span(runs: &mut Vec<Run>, block: &BlockId, range: &Range<usize>) -> Result<(usize, usize), OpError> {
    let bad = |offset| OpError::BadOffset { block: block.clone(), offset };
    if range.start > range.end {
        return Err(bad(range.start));
    }
    let from = cut(runs, range.start).ok_or_else(|| bad(range.start))?;
    let to = cut(runs, range.end).ok_or_else(|| bad(range.end))?;
    Ok((from, to))
}

fn insert_text(runs: &mut Vec<Run>, offset: usize, text: &str) -> Option<()> {
    let mut start = 0;
    for run in runs.iter_mut() {
        let end = start + run.text.len();
        if offset <= end {
            let at = offset - start;
            if !run.text.is_char_boundary(at) {
                return None;
            }
            run.text.insert_str(at, text);
            return Some(());
        }
        start = end;
    }
    if offset != start {
        return None;
    }
    runs.push(Run::plain(text));
    Some(())
}

/// Drops empty runs and merges neighbours that share their formatting.
fn normalise(runs: &mut Vec<Run>) {
    runs.retain(|run| !run.text.is_empty());
    runs.dedup_by(|next, prev| {
        let same = prev.same_format(next);
        if same {
            prev.text.push_str(&next.text);
        }
        same
    });
}
