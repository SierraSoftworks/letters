//! Blocks that share an id.
//!
//! The model has no move: a block that moves is taken out and put back under its id. Two writers who move
//! one block at once each put it back, so a merge leaves two blocks with one id. The model tells them
//! apart by a number (`id~2`), counted from where they stand, so when one is taken out the others close
//! ranks, and one of them may go by the id of the block that left.

use std::collections::BTreeSet;

use letters_model::{Block, BlockId, Body, Document, Op};

/// What a block went by, and what it goes by now.
pub(crate) type Renames = Vec<(BlockId, BlockId)>;

/// The block an op names, for one that names a block already there.
fn named(op: &mut Op) -> Option<&mut BlockId> {
    match op {
        Op::InsertText { at, .. } | Op::Split { at, .. } => Some(&mut at.block),
        Op::DeleteText { block, .. } | Op::SetMark { block, .. } | Op::SetLink { block, .. } => Some(block),
        Op::SetLang { block, .. } | Op::SetKind { block, .. } | Op::SetBlockProps { block, .. } => Some(block),
        Op::Join { block } | Op::RemoveBlock { block } => Some(block),
        Op::InsertBlock { parent, .. } => parent.as_mut(),
        _ => None,
    }
}

/// The id a block goes by after `renames`.
pub(crate) fn renamed(renames: &[(BlockId, BlockId)], id: &BlockId) -> BlockId {
    renames.iter().fold(id.clone(), |id, (was, now)| if *was == id { now.clone() } else { id })
}

/// Gives a block that is put back, and those inside it, a name no block goes by where another block
/// may come to go by theirs. `shown` holds every id the document shows, and takes the new names.
fn rename(block: &mut Block, shown: &mut BTreeSet<BlockId>, renames: &mut Renames) {
    let id = block.id.as_str();
    let numbered = BlockId::new(format!("{id}~"));
    // Only a block that shares its id has a number, or stands before one that has.
    let follows = |other: &BlockId| other.as_str().starts_with(numbered.as_str());
    let shared = id.contains('~') || shown.range(&numbered..).next().is_some_and(follows);
    if shared && shown.contains(&block.id) {
        // The blocks left behind only ever take names that are shown now, so any other is free.
        let stem = id.split('~').next().unwrap_or_default();
        let free = (2usize..).map(|n| BlockId::new(format!("{stem}~{n}"))).find(|name| !shown.contains(name));
        if let Some(free) = free {
            shown.insert(free.clone());
            renames.push((std::mem::replace(&mut block.id, free.clone()), free));
        }
    }
    if let Body::Blocks(children) = &mut block.body {
        children.iter_mut().for_each(|child| rename(child, shown, renames));
    }
}

/// Makes an edit that takes blocks out and puts them back one the model accepts, where a block it puts
/// back shares its id with another. Returns the ops, and what each such block went by and goes by now.
pub(crate) fn unshared(doc: &Document, mut ops: Vec<Op>) -> (Vec<Op>, Renames) {
    let mut renames = Renames::new();
    let moves = |op: &Op| matches!(op, Op::RemoveBlock { .. });
    let puts = |op: &Op| matches!(op, Op::InsertBlock { .. });
    if !(ops.iter().any(moves) && ops.iter().any(puts)) {
        return (ops, renames);
    }
    let mut shown: BTreeSet<BlockId> = doc.walk().map(|block| block.id.clone()).collect();
    for op in &mut ops {
        // An op that follows means the block that was put back, not the one that took its name.
        if let Some(id) = named(op) {
            *id = renamed(&renames, id);
        }
        if let Op::InsertBlock { block, .. } = op {
            rename(block, &mut shown, &mut renames);
        }
    }
    (ops, renames)
}
