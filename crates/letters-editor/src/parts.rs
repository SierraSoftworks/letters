//! Blocks made of parts: figures, galleries and tables.
//!
//! A figure is a block of text whose text is its caption, so a caret in one is in one of two places: on the
//! picture, or in the caption. A picture in a gallery has only the first, since the gallery holds the caption.
//! A figure, a cell and a gallery's caption are parts: nothing splits them or joins them to other text, and
//! what moves, wraps and is left behind is the unit they belong to.

use letters_model::{Block, BlockId, BlockKind, Op, Pos, Selection, Stood};

use crate::{Caret, Editor, Platform, text, tree};

/// The rows of a table, and the cells of a row. A type this build does not know may sit among them.
pub(crate) fn rows(table: &Block) -> Vec<&Block> {
    table.body.blocks().iter().filter(|row| row.kind == BlockKind::TableRow).collect()
}

pub(crate) fn cells(row: &Block) -> Vec<&Block> {
    row.body.blocks().iter().filter(|cell| cell.kind == BlockKind::TableCell).collect()
}

/// The nearest row below the `row`th of a table, or above it, that has cells. A merge can leave a row with
/// none, and such a row holds no caret.
pub(crate) fn row_beside(table: &Block, row: usize, down: bool) -> Option<&Block> {
    let rows = rows(table);
    let held = |row: &&Block| !cells(row).is_empty();
    match down {
        true => rows.into_iter().skip(row + 1).find(held),
        false => rows.into_iter().take(row).rev().find(held),
    }
}

/// The caption of a gallery: the last one, should a merge have left it several.
pub(crate) fn caption(gallery: &Block) -> Option<&Block> {
    gallery.body.blocks().iter().rev().find(|part| part.kind == BlockKind::Caption)
}

/// What a gallery with fewer than two pictures comes down to: its one picture as a figure that takes the
/// gallery's caption and number, or its caption as a line of text. `None` when nothing is left of it.
pub(crate) fn lone(gallery: &Block) -> Option<Block> {
    let BlockKind::Gallery { numbered } = gallery.kind else { return None };
    let caption = caption(gallery);
    match gallery.body.blocks().iter().find(|part| tree::is_figure(part)) {
        Some(figure) => {
            let mut figure = figure.clone();
            if let BlockKind::Figure { numbered: own, .. } = &mut figure.kind {
                *own = numbered;
            }
            if let Some(caption) = caption {
                figure.body = caption.body.clone();
            }
            Some(figure)
        }
        None => caption.filter(|caption| text::len(caption) > 0).map(|caption| {
            let mut line = caption.clone();
            line.kind = BlockKind::Paragraph;
            line
        }),
    }
}

fn pictures(gallery: &Block) -> usize {
    gallery.body.blocks().iter().filter(|part| tree::is_figure(part)).count()
}

/// Brings every gallery in `blocks` that has fewer than two pictures down to what [`lone`] makes of it.
pub(crate) fn settled(blocks: Vec<Block>) -> Vec<Block> {
    let settle = |mut block: Block| {
        if tree::is_gallery(&block) && pictures(&block) < 2 {
            return lone(&block);
        }
        if let letters_model::Body::Blocks(children) = &mut block.body {
            *children = settled(std::mem::take(children));
        }
        Some(block)
    };
    blocks.into_iter().filter_map(settle).collect()
}

impl Editor {
    /// The figure whose picture is selected: the caret is on the picture and not in its caption. Text typed
    /// then goes nowhere, Enter starts the caption, and Backspace takes the figure away.
    pub fn picture(&self) -> Option<&BlockId> {
        self.caret.as_ref().filter(|caret| caret.picture).map(|caret| &caret.selection.head.block)
    }

    /// Selects a figure's picture, as a click on it does. Anything that is not a figure is ignored.
    pub fn select_picture(&mut self, figure: &BlockId) {
        if self.block(figure).is_some_and(tree::is_figure) {
            self.end_composition();
            (self.rule, self.goal) = (false, None);
            self.seal();
            self.place_picture(figure);
        }
    }

    /// The figures to show as selected, in document order: the one whose picture the caret is on, or those
    /// a range takes whole, which deleting the range would take away with their captions.
    pub fn figures(&self) -> Vec<BlockId> {
        let Some(caret) = &self.caret else { return Vec::new() };
        if caret.picture {
            return vec![caret.selection.head.block.clone()];
        }
        let (start, end) = self.ordered(&caret.selection);
        self.taken(&start, &end)
    }

    /// The figures a range takes whole: each whose caption it covers from a block before or to a block
    /// after. A picture in a gallery has no caption of its own, so a range that reaches it takes it.
    pub(crate) fn taken(&self, start: &Pos, end: &Pos) -> Vec<BlockId> {
        if start.block == end.block {
            return Vec::new();
        }
        let texts = tree::texts(self.doc());
        let index = |pos: &Pos| texts.iter().position(|block| block.id == pos.block);
        let (Some(from), Some(to)) = (index(start), index(end)) else { return Vec::new() };
        let whole = |i: &usize| {
            let block = texts[*i];
            let covered = (*i > from || start.offset == 0) && (*i < to || end.offset >= self.span(block));
            tree::is_figure(block) && covered
        };
        (from..=to).filter(whole).map(|i| texts[i].id.clone()).collect()
    }

    /// Where a block sits, by the model's map of ids.
    pub(crate) fn seat(&self, id: &BlockId) -> Option<tree::Place<'_>> {
        self.model.place(id).map(Into::into)
    }

    /// Whether a block can be made to read as `version`: it is there and reads otherwise, and no block
    /// outside it goes by an id that the version holds.
    pub(crate) fn takes_version(&self, id: &BlockId, version: Option<&Block>) -> bool {
        fn ids<'a>(block: &'a Block, out: &mut Vec<&'a BlockId>) {
            out.push(&block.id);
            block.body.blocks().iter().for_each(|part| ids(part, out));
        }
        // The version comes to stand under the block's id, whatever it goes by itself.
        let reads = |block: &'_ Block| (block.kind.clone(), block.body.clone(), block.lang.clone(), block.page_break);
        let same = |held: &Block| version.is_some_and(|version| reads(version) == reads(held));
        let Some(held) = self.block(id).filter(|held| !same(held)) else { return false };
        let (mut mine, mut theirs) = (Vec::new(), Vec::new());
        ids(held, &mut mine);
        version.into_iter().for_each(|version| ids(version, &mut theirs));
        theirs.iter().skip(1).all(|id| mine.contains(id) || self.block(id).is_none())
    }

    /// Puts another version of a block in its place, under the block's own id, or takes the block away.
    pub(crate) fn use_version(&mut self, id: BlockId, version: Option<Box<Block>>) {
        let Some(place) = self.seat(&id) else { return };
        let (parent, index) = (place.parent_id(), place.index);
        let mut ops = vec![Op::RemoveBlock { block: id.clone() }];
        ops.extend(version.map(|version| Op::InsertBlock { parent, index, block: Block { id, ..*version } }));
        if self.apply(ops) {
            // The caret may have stood in text that reads otherwise now, or is gone.
            self.restore(self.caret.clone());
        }
    }

    /// Where a block that the document no longer holds can be put back: the block that held it, if one
    /// did, and its number among that one's blocks. `None` while the document holds a block by its id or
    /// by an id inside it, or no longer holds the block it stood in as one that takes its kind.
    pub(crate) fn takes_back(&self, block: &Block, stood: &Stood) -> Option<(Option<BlockId>, usize)> {
        fn free(editor: &Editor, block: &Block) -> bool {
            editor.block(&block.id).is_none() && block.body.blocks().iter().all(|part| free(editor, part))
        }
        let beside = match &stood.parent {
            Some(parent) => self.block(parent).filter(|parent| parent.kind.admits(&block.kind))?.body.blocks(),
            None => &self.doc().blocks,
        };
        // The block it stood behind may be gone since, or elsewhere: then it goes first, as one that stood
        // first does.
        let after = stood.after.as_ref().and_then(|after| beside.iter().position(|block| block.id == *after));
        free(self, block).then(|| (stood.parent.clone(), after.map_or(0, |after| after + 1)))
    }

    /// Puts a block back where it stood. The caret stays where it is.
    pub(crate) fn put_back(&mut self, block: Block, stood: &Stood) {
        if let Some((parent, index)) = self.takes_back(&block, stood) {
            _ = self.apply(vec![Op::InsertBlock { parent, index, block }]);
        }
    }

    /// See [`tree::is_part`].
    pub(crate) fn part(&self, id: &BlockId) -> bool {
        let parent = self.seat(id).and_then(|place| place.parent);
        self.block(id).is_some_and(|block| tree::is_part(block, parent))
    }

    /// Whether a block is a picture in a gallery.
    pub(crate) fn member(&self, id: &BlockId) -> bool {
        self.block(id).is_some_and(tree::is_figure) && self.gallery(id).is_some()
    }

    /// The gallery that holds a picture or a caption.
    pub(crate) fn gallery(&self, id: &BlockId) -> Option<&Block> {
        self.seat(id).and_then(|place| place.parent).filter(|parent| tree::is_gallery(parent))
    }

    /// How much text of a block a caret can be in. A picture in a gallery shows none.
    pub(crate) fn span(&self, block: &Block) -> usize {
        if self.member(&block.id) { 0 } else { text::len(block) }
    }

    /// The place in text nearest to a position that may name a block with none, as a click on an empty
    /// gallery or table does: the first text in the block, or else the text after it, or before it.
    pub(crate) fn nearest(&self, pos: &Pos) -> Option<Pos> {
        if let Some(pos) = self.clamp(pos) {
            return Some(pos);
        }
        let all: Vec<&Block> = self.doc().walk().collect();
        let at = all.iter().position(|block| block.id == pos.block)?;
        let text = |block: &&&Block| tree::is_text(block);
        let after = all[at + 1..].iter().find(text).map(|block| Pos::new(block.id.clone(), 0));
        let before = || all[..at].iter().rev().find(text).map(|block| Pos::new(block.id.clone(), self.span(block)));
        after.or_else(before).and_then(|pos| self.clamp(&pos))
    }

    /// The block that a part moves with: the gallery of a picture or a caption, and the table of a cell.
    /// Any other block is its own unit.
    pub(crate) fn unit(&self, id: &BlockId) -> BlockId {
        let Some(parent) = self.seat(id).and_then(|place| place.parent) else { return id.clone() };
        match (self.block(id).map(|block| &block.kind), &parent.kind) {
            (Some(BlockKind::Figure { .. } | BlockKind::Caption), BlockKind::Gallery { .. }) => parent.id.clone(),
            (Some(BlockKind::TableCell), BlockKind::TableRow) => self.unit(&parent.id),
            (Some(BlockKind::TableRow), BlockKind::Table { .. }) => parent.id.clone(),
            _ => id.clone(),
        }
    }

    /// The table a cell is in, and the cell's row and column there.
    pub(crate) fn grid(&self, cell: &BlockId) -> Option<(&Block, usize, usize)> {
        let row = self.seat(cell)?.parent.filter(|row| row.kind == BlockKind::TableRow)?;
        let table = self.seat(&row.id)?.parent.filter(|table| matches!(table.kind, BlockKind::Table { .. }))?;
        let column = cells(row).iter().position(|other| other.id == *cell)?;
        let at = rows(table).iter().position(|other| other.id == row.id)?;
        Some((table, at, column))
    }

    pub(crate) fn place_picture(&mut self, figure: &BlockId) {
        let selection = Selection::caret(Pos::new(figure.clone(), 0));
        self.caret = Some(Caret { selection, marks: Default::default(), picture: true, before_wrap: false });
    }

    /// Where a caret that comes to `pos` from another block stands. The start of a figure is its picture,
    /// and so is any place in a picture of a gallery.
    pub(crate) fn arrive(&self, pos: Pos) -> (Pos, bool) {
        let figure = self.block(&pos.block).is_some_and(tree::is_figure);
        if figure && (pos.offset == 0 || self.member(&pos.block)) {
            return (Pos::new(pos.block, 0), true);
        }
        (pos, false)
    }

    pub(crate) fn land(&mut self, (pos, picture): (Pos, bool)) {
        if picture { self.place_picture(&pos.block) } else { self.place(pos) }
    }

    /// The caret as it can stand in the document now: on text that is there, and on a picture only where
    /// there is one.
    pub(crate) fn fit(&self, caret: &Caret) -> Option<Caret> {
        let (anchor, head) = (self.clamp(&caret.selection.anchor)?, self.clamp(&caret.selection.head)?);
        let selection = Selection { anchor, head };
        let figure = self.block(&selection.head.block).is_some_and(tree::is_figure);
        let picture = selection.is_caret() && (self.member(&selection.head.block) || caret.picture && figure);
        let selection = if picture { Selection::caret(Pos::new(selection.head.block, 0)) } else { selection };
        let before_wrap = caret.before_wrap && !picture && selection == caret.selection;
        Some(Caret { selection, marks: caret.marks.clone(), picture, before_wrap })
    }

    /// Brings every gallery that an edit left with fewer than two pictures down to what [`lone`] makes of
    /// it, in the same undo step. Returns each caption that became a figure's own, with that figure.
    pub(crate) fn settle(&mut self) -> Vec<(BlockId, BlockId)> {
        let doc = self.doc();
        let (mut ops, mut moved) = (Vec::new(), Vec::new());
        // Later galleries go first, so that the places of earlier ones still hold.
        let galleries: Vec<&Block> =
            doc.walk().filter(|block| tree::is_gallery(block) && pictures(block) < 2).collect();
        for gallery in galleries.into_iter().rev() {
            let Some(place) = self.seat(&gallery.id) else { continue };
            match lone(gallery) {
                Some(block) => {
                    let caption = caption(gallery).filter(|_| tree::is_figure(&block));
                    moved.extend(caption.map(|caption| (caption.id.clone(), block.id.clone())));
                    ops.push(Op::RemoveBlock { block: gallery.id.clone() });
                    ops.push(Op::InsertBlock { parent: place.parent_id(), index: place.index, block });
                }
                None => ops.push(Op::RemoveBlock { block: tree::removal(doc, &gallery.id) }),
            }
        }
        if ops.is_empty() {
            return moved;
        }
        self.apply(ops);
        let moved: Vec<_> = moved.into_iter().map(|(caption, figure)| (caption, self.renamed(&figure))).collect();
        // A caret in a caption that moved stays in its text.
        let follow = |pos: &Pos| match moved.iter().find(|(caption, _)| *caption == pos.block) {
            Some((_, figure)) => Pos::new(figure.clone(), pos.offset),
            None => pos.clone(),
        };
        if let Some(caret) = &mut self.caret {
            caret.selection =
                Selection { anchor: follow(&caret.selection.anchor), head: follow(&caret.selection.head) };
        }
        moved
    }

    /// Takes a figure away, and leaves the caret on the picture beside it or in the text around it.
    pub(crate) fn remove_picture(&mut self, figure: &BlockId) {
        let doc = self.doc();
        let texts = tree::texts(doc);
        let Some(at) = texts.iter().position(|block| block.id == *figure) else { return };
        let gallery = self.gallery(figure).map(|gallery| &gallery.id);
        let beside = |other: &&Block| tree::is_figure(other) && self.gallery(&other.id).map(|g| &g.id) == gallery;
        let (before, after) = (at.checked_sub(1).map(|at| texts[at]), texts.get(at + 1).copied());
        // Another picture of the gallery comes first. Then the text after, and then the text before.
        let sibling = [after, before].into_iter().flatten().find(|other| gallery.is_some() && beside(other));
        let next = sibling.map(|other| Pos::new(other.id.clone(), 0)).or_else(|| {
            let after = after.map(|block| Pos::new(block.id.clone(), 0));
            after.or_else(|| before.map(|block| Pos::new(block.id.clone(), self.span(block))))
        });
        self.apply(vec![Op::RemoveBlock { block: tree::removal(doc, figure) }]);
        self.settle();
        match next.and_then(|pos| self.clamp(&pos)) {
            Some(pos) => self.land(self.arrive(pos)),
            None => self.restore(None),
        }
    }

    /// Puts the caret in the caption that goes with the selected picture: its own, or its gallery's, which
    /// is made when there is none.
    pub(crate) fn edit_caption(&mut self, platform: &mut dyn Platform) {
        let Some(figure) = self.picture().cloned() else { return };
        let Some(gallery) = self.gallery(&figure) else {
            let end = self.block(&figure).map_or(0, text::len);
            return self.place(Pos::new(figure, end));
        };
        if let Some(caption) = caption(gallery) {
            return self.place(Pos::new(caption.id.clone(), text::len(caption)));
        }
        let new = BlockId::new(platform.new_id());
        let (parent, index) = (Some(gallery.id.clone()), gallery.body.blocks().len());
        let block = Block::new(new.clone(), BlockKind::Caption);
        if self.apply(vec![Op::InsertBlock { parent, index, block }]) {
            self.place(Pos::new(new, 0));
        }
    }

    /// Makes an empty line after the unit the caret is in, and puts the caret on it.
    pub(crate) fn line_after(&mut self, platform: &mut dyn Platform) {
        let Some(head) = self.selection().map(|selection| selection.head.block.clone()) else { return };
        let Some(place) = self.seat(&self.unit(&head)) else { return };
        let new = BlockId::new(platform.new_id());
        let line = Block::new(new.clone(), BlockKind::Paragraph);
        if self.apply(vec![Op::InsertBlock { parent: place.parent_id(), index: place.index + 1, block: line }]) {
            self.place(Pos::new(new, 0));
        }
    }

    /// Enter in a part never breaks it in two. In a cell it goes down the column, and from the last row
    /// out of the table. In a caption it leaves for a new line. Returns whether `at` is in a part.
    pub(crate) fn split_part(&mut self, at: &Pos, platform: &mut dyn Platform) -> bool {
        if !self.part(&at.block) {
            return false;
        }
        let below = self.grid(&at.block).and_then(|(table, row, column)| {
            let cells = cells(row_beside(table, row, true)?);
            let cell = cells.get(column).or(cells.last())?;
            Some(Pos::new(cell.id.clone(), text::len(cell)))
        });
        match below {
            Some(pos) => self.place(pos),
            None => self.line_after(platform),
        }
        true
    }

    /// A row of `width` empty cells.
    pub(crate) fn empty_row(width: usize, platform: &mut dyn Platform) -> Block {
        let mut row = Block::new(platform.new_id().as_str(), BlockKind::TableRow);
        let cell = |_| Block::new(platform.new_id().as_str(), BlockKind::TableCell);
        row.body = letters_model::Body::Blocks((0..width).map(cell).collect());
        row
    }

    /// Tab in a table goes to the next cell, or with `forward` unset to the one before. Past the last cell
    /// it adds a row.
    pub(crate) fn cell_step(&mut self, cell: &BlockId, forward: bool, platform: &mut dyn Platform) {
        let Some((table, row, _)) = self.grid(cell) else { return };
        let rows = rows(table);
        let all: Vec<&Block> = rows.iter().flat_map(|row| cells(row)).collect();
        let Some(at) = all.iter().position(|other| other.id == *cell) else { return };
        let to = if forward { all.get(at + 1) } else { at.checked_sub(1).and_then(|at| all.get(at)) };
        if let Some(to) = to {
            return self.place(Pos::new(to.id.clone(), text::len(to)));
        }
        if !forward {
            return;
        }
        let BlockKind::Table { columns, .. } = &table.kind else { return };
        let width = columns.len().max(cells(rows[row]).len());
        let (parent, index) = (Some(table.id.clone()), table.body.blocks().len());
        let block = Self::empty_row(width, platform);
        let first = block.body.blocks().first().map(|cell| cell.id.clone());
        if self.apply(vec![Op::InsertBlock { parent, index, block }])
            && let Some(first) = first
        {
            self.place(Pos::new(first, 0));
        }
    }
}
