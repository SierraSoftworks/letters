//! Finding blocks in the document tree. Carets live only in blocks that hold text.

use letters_model::{Block, BlockId, Body, Document};

pub(crate) fn is_text(block: &Block) -> bool {
    matches!(block.body, Body::Text(_))
}

/// Where a block sits: its parent, if it has one, and its index among its siblings.
pub(crate) struct Place<'a> {
    pub parent: Option<&'a Block>,
    pub index: usize,
}

impl<'a> Place<'a> {
    pub fn parent_id(&self) -> Option<BlockId> {
        self.parent.map(|parent| parent.id.clone())
    }

    pub fn siblings(&self, doc: &'a Document) -> &'a [Block] {
        self.parent.map_or(&doc.blocks, |parent| parent.body.blocks())
    }
}

pub(crate) fn place<'a>(doc: &'a Document, id: &BlockId) -> Option<Place<'a>> {
    fn find<'a>(blocks: &'a [Block], parent: Option<&'a Block>, id: &BlockId) -> Option<Place<'a>> {
        blocks.iter().enumerate().find_map(|(index, block)| {
            if &block.id == id { Some(Place { parent, index }) } else { find(block.body.blocks(), Some(block), id) }
        })
    }
    find(&doc.blocks, None, id)
}

/// The blocks that hold text, in document order.
pub(crate) fn texts(doc: &Document) -> Vec<&Block> {
    doc.walk().filter(|block| is_text(block)).collect()
}

/// The blocks with nothing nested in them, in document order: text, dividers and empty containers.
pub(crate) fn leaves(doc: &Document) -> Vec<&Block> {
    doc.walk().filter(|block| block.body.blocks().is_empty()).collect()
}

/// The outermost block to remove so that taking `id` away leaves no empty container behind.
pub(crate) fn removal(doc: &Document, id: &BlockId) -> BlockId {
    let mut victim = id.clone();
    while let Some(parent) = place(doc, &victim).and_then(|place| place.parent) {
        if parent.body.blocks().len() > 1 {
            break;
        }
        victim = parent.id.clone();
    }
    victim
}

/// Whether `ancestor` holds `id` at any depth.
pub(crate) fn contains(ancestor: &Block, id: &BlockId) -> bool {
    ancestor.body.blocks().iter().any(|child| &child.id == id || contains(child, id))
}

/// How many blocks `block` is, counting itself and everything nested in it.
pub(crate) fn size(block: &Block) -> usize {
    1 + block.body.blocks().iter().map(size).sum::<usize>()
}
