use crate::BlockId;

/// A caret position: a UTF-8 byte offset into a text block, always on a character boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pos {
    pub block: BlockId,
    pub offset: usize,
}

impl Pos {
    pub fn new(block: impl Into<BlockId>, offset: usize) -> Self {
        Self { block: block.into(), offset }
    }
}

/// A selection from `anchor` to `head`, where `head` is the end that moves. Equal ends are a caret.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Selection {
    pub anchor: Pos,
    pub head: Pos,
}

impl Selection {
    pub fn caret(pos: Pos) -> Self {
        Self { anchor: pos.clone(), head: pos }
    }

    pub fn is_caret(&self) -> bool {
        self.anchor == self.head
    }
}
