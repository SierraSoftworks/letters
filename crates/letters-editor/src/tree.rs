//! Finding blocks in the document tree. Carets live only in blocks that hold text.

use letters_model::{Block, BlockId, BlockKind, Body, Document};

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

impl<'a> From<letters_model::Place<'a>> for Place<'a> {
    fn from(place: letters_model::Place<'a>) -> Self {
        Self { parent: place.parent, index: place.index }
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

pub(crate) fn is_figure(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Figure { .. })
}

pub(crate) fn is_gallery(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Gallery { .. })
}

/// Whether a block of text is a part of something larger: a figure, whose text is its caption, a cell in
/// its row, or the caption of a gallery. A part is never split, joined to other text or turned into
/// another kind of block. A cell or a caption that stands alone is a line like any other.
pub(crate) fn is_part(block: &Block, parent: Option<&Block>) -> bool {
    match block.kind {
        BlockKind::Figure { .. } => true,
        BlockKind::TableCell => parent.is_some_and(|parent| parent.kind == BlockKind::TableRow),
        BlockKind::Caption => parent.is_some_and(is_gallery),
        _ => false,
    }
}

/// Whether text flows through a block as it does through a paragraph: it holds text and is no figure.
pub(crate) fn flows(block: &Block) -> bool {
    is_text(block) && !is_figure(block)
}

/// The first and the last block of text inside `block`, or the block itself when it holds text.
pub(crate) fn first_text(block: &Block) -> Option<&Block> {
    if is_text(block) { Some(block) } else { block.body.blocks().iter().find_map(first_text) }
}

pub(crate) fn last_text(block: &Block) -> Option<&Block> {
    if is_text(block) { Some(block) } else { block.body.blocks().iter().rev().find_map(last_text) }
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
