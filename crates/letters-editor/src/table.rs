//! Tables: the line that becomes one, and the commands that change rows and columns.
//!
//! A command leaves every row with a cell in every column. Two writers who add rows and columns at once
//! can leave a table short of that, so each command first fills the table out to its widest row.

use std::ops::Range;

use letters_model::{Align, Block, BlockId, BlockKind, Body, Mark, Op, Pos};

use crate::parts::{cells, rows};
use crate::{Editor, Platform, text, tree};

/// A change to the table the caret is in.
#[derive(Clone, Debug, PartialEq)]
pub enum TableEdit {
    /// Adds a row of empty cells below the caret's row, or above it.
    AddRow { below: bool },
    /// Adds a column of empty cells after the caret's column, or before it.
    AddColumn { after: bool },
    /// Takes the caret's row away. The last row takes the table with it.
    RemoveRow,
    /// Takes the caret's column away. The last column takes the table with it.
    RemoveColumn,
    /// Moves the caret's row to stand at `to` among the rows, counted from 0.
    MoveRow { to: usize },
    /// Moves the caret's column to stand at `to` among the columns, counted from 0.
    MoveColumn { to: usize },
    /// Sets how the caret's column lines its text up.
    Align(Align),
    /// Sets whether the first row is the table's header.
    Header(bool),
}

/// Where the caret is in a table. Rows and columns are counted from 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub table: BlockId,
    pub row: usize,
    pub column: usize,
    pub rows: usize,
    pub columns: usize,
}

/// The cells of a line such as `| a | b |`: where the text of each sits. `None` when the line is not a
/// row of two cells or more.
pub(crate) fn row(line: &Block) -> Option<Vec<Range<usize>>> {
    let (text, runs) = (line.plain_text(), line.body.runs());
    // A bar in code or in maths is part of it.
    let meant = |run: &letters_model::Run| run.marks.contains(&Mark::Code) || run.marks.contains(&Mark::Math);
    let bar = |(at, _): (usize, &str)| (!text::run_at(runs, at).is_some_and(meant)).then_some(at);
    let bars: Vec<usize> = text.match_indices('|').filter_map(bar).collect();
    let edged = bars.first() == Some(&0) && bars.last().map(|last| last + 1) == Some(text.trim_end().len());
    let cell = |pair: &[usize]| {
        let inner = &text[pair[0] + 1..pair[1]];
        let start = pair[0] + 1 + inner.len() - inner.trim_start().len();
        start..start + inner.trim().len()
    };
    (edged && bars.len() >= 3).then(|| bars.windows(2).map(cell).collect())
}

/// How many columns a table has: as many as its kind says, or as its longest row has cells.
fn width(table: &Block) -> usize {
    let columns = if let BlockKind::Table { columns, .. } = &table.kind { columns.len() } else { 0 };
    rows(table).iter().map(|row| cells(row).len()).fold(columns, usize::max)
}

/// The place among a holder's blocks of its `n`th row or cell, or the place after its last.
fn slot(holder: &Block, kind: &BlockKind, n: usize) -> usize {
    let mut own = holder.body.blocks().iter().enumerate().filter(|(_, part)| part.kind == *kind).map(|(at, _)| at);
    let last = own.clone().next_back().map_or(0, |last| last + 1);
    own.nth(n).unwrap_or(last)
}

/// The edit that moves a holder's `from`th row or cell to stand `to`th.
fn shift(holder: &Block, kind: &BlockKind, from: usize, to: usize) -> Vec<Op> {
    let mut own: Vec<(usize, &Block)> =
        holder.body.blocks().iter().enumerate().filter(|(_, part)| part.kind == *kind).collect();
    if from >= own.len() || from == to {
        return Vec::new();
    }
    let (was, block) = own.remove(from);
    // The others stand one place up once it is out from before them.
    let stands = |(at, _): &(usize, &Block)| at - usize::from(*at > was);
    let index = own.get(to).map(stands).or(own.last().map(|last| stands(last) + 1)).unwrap_or(0);
    let insert = Op::InsertBlock { parent: Some(holder.id.clone()), index, block: block.clone() };
    vec![Op::RemoveBlock { block: block.id.clone() }, insert]
}

fn empty_cell(platform: &mut dyn Platform) -> Block {
    Block::new(platform.new_id().as_str(), BlockKind::TableCell)
}

impl Editor {
    /// The cell the caret is in, for the commands a table offers there.
    pub fn cell(&self) -> Option<Cell> {
        let (table, row, column) = self.grid(&self.selection()?.head.block)?;
        Some(Cell { table: table.id.clone(), row, column, rows: rows(table).len(), columns: width(table) })
    }

    /// Enter after a line such as `| a | b |` makes it the header of a table, with an empty row to write
    /// in below. See [`row`].
    pub(crate) fn table_rule(&mut self, line: &BlockId, platform: &mut dyn Platform) {
        let (Some(block), Some(place)) = (self.block(line), self.seat(line)) else { return };
        let Some(found) = row(block) else { return };
        let head = |range: &Range<usize>| {
            let mut cell = empty_cell(platform);
            cell.body = Body::Text(text::slice(block.body.runs(), range.clone()));
            cell
        };
        let heads = found.iter().map(head).collect();
        let mut header = Block::new(platform.new_id().as_str(), BlockKind::TableRow);
        header.body = Body::Blocks(heads);
        let below = Self::empty_row(found.len(), platform);
        let first = below.body.blocks().first().map(|cell| cell.id.clone());
        let kind = BlockKind::Table { columns: vec![Align::Start; found.len()], header: true };
        let mut table = Block::new(platform.new_id().as_str(), kind);
        table.body = Body::Blocks(vec![header, below]);
        let insert = Op::InsertBlock { parent: place.parent_id(), index: place.index, block: table };
        if self.apply(vec![Op::RemoveBlock { block: line.clone() }, insert])
            && let Some(first) = first
        {
            self.place(Pos::new(first, 0));
        }
    }

    /// Puts in a table of empty cells with a header row, and the caret in its first cell. It goes after the
    /// block the selection ends in, or in the place of an empty line.
    pub(crate) fn insert_table(&mut self, rows: usize, columns: usize, platform: &mut dyn Platform) {
        let (rows, columns) = (rows.max(1), columns.max(1));
        let kind = BlockKind::Table { columns: vec![Align::Start; columns], header: true };
        let mut table = Block::new(platform.new_id().as_str(), kind);
        table.body = Body::Blocks((0..rows).map(|_| Self::empty_row(columns, platform)).collect());
        let Some(first) = tree::first_text(&table).map(|cell| cell.id.clone()) else { return };

        let end = self.caret.as_ref().map(|caret| self.ordered(&caret.selection).1.block);
        let seat = end.as_ref().and_then(|end| Some((self.block(end)?, self.seat(&self.unit(end))?)));
        let ops = match seat {
            Some((block, place)) => {
                let empty = block.kind == BlockKind::Paragraph && text::len(block) == 0;
                let (parent, index) = (place.parent_id(), place.index + usize::from(!empty));
                let insert = Op::InsertBlock { parent, index, block: table };
                let gone = empty.then(|| Op::RemoveBlock { block: block.id.clone() });
                gone.into_iter().chain([insert]).collect()
            }
            // A document with no text yet starts with the table.
            None => vec![Op::InsertBlock { parent: None, index: self.doc().blocks.len(), block: table }],
        };
        if self.apply(ops) {
            self.place(Pos::new(first, 0));
        }
    }

    /// Gives every row of a table a cell in every column.
    fn square(&mut self, table: &BlockId, platform: &mut dyn Platform) {
        let Some(block) = self.block(table) else { return };
        let BlockKind::Table { columns, header } = &block.kind else { return };
        let wide = width(block);
        let mut ops = Vec::new();
        if columns.len() != wide {
            let mut columns = columns.clone();
            columns.resize(wide, Align::Start);
            ops.push(Op::SetKind { block: table.clone(), kind: BlockKind::Table { columns, header: *header } });
        }
        for row in rows(block) {
            let (have, index) = (cells(row).len(), slot(row, &BlockKind::TableCell, usize::MAX));
            let fill =
                |n| Op::InsertBlock { parent: Some(row.id.clone()), index: index + n, block: empty_cell(platform) };
            ops.extend((0..wide - have).map(fill));
        }
        self.apply(ops);
    }

    /// Takes a table away, and leaves the caret in the text after it, or before it.
    fn remove_table(&mut self, table: &BlockId) {
        let doc = self.doc();
        let Some(block) = self.block(table) else { return };
        let texts = tree::texts(doc);
        let at = |cell: Option<&Block>| cell.and_then(|cell| texts.iter().position(|block| block.id == cell.id));
        let after = at(tree::last_text(block)).and_then(|last| texts.get(last + 1));
        let before = at(tree::first_text(block)).and_then(|first| first.checked_sub(1)).map(|at| &texts[at]);
        let next = after
            .map(|block| Pos::new(block.id.clone(), 0))
            .or(before.map(|block| Pos::new(block.id.clone(), self.span(block))));
        self.apply(vec![Op::RemoveBlock { block: tree::removal(doc, table) }]);
        match next.and_then(|pos| self.clamp(&pos)) {
            Some(pos) => self.land(self.arrive(pos)),
            None => self.restore(None),
        }
    }

    pub(crate) fn table_edit(&mut self, edit: TableEdit, platform: &mut dyn Platform) {
        let Some(head) = self.selection().map(|selection| selection.head.block.clone()) else { return };
        let Some(id) = self.grid(&head).map(|(table, ..)| table.id.clone()) else { return };
        self.square(&id, platform);
        let Some((table, row, column)) = self.grid(&head) else { return };
        let BlockKind::Table { columns, header } = &table.kind else { return };
        let (all, mut columns, mut header) = (rows(table), columns.clone(), *header);
        let (was, parent) = ((columns.clone(), header), Some(id.clone()));
        let (mut ops, mut to) = (Vec::new(), None);
        match edit {
            TableEdit::AddRow { below } => {
                let block = Self::empty_row(columns.len(), platform);
                to = block.body.blocks().get(column).map(|cell| cell.id.clone());
                let index = slot(table, &BlockKind::TableRow, row) + usize::from(below);
                ops.push(Op::InsertBlock { parent, index, block });
            }
            TableEdit::AddColumn { after } => {
                let at = column + usize::from(after);
                columns.insert(at, Align::Start);
                for (n, each) in all.iter().enumerate() {
                    let block = empty_cell(platform);
                    to = to.or((n == row).then(|| block.id.clone()));
                    let index = slot(each, &BlockKind::TableCell, at);
                    ops.push(Op::InsertBlock { parent: Some(each.id.clone()), index, block });
                }
            }
            TableEdit::RemoveRow if all.len() == 1 => return self.remove_table(&id),
            TableEdit::RemoveColumn if columns.len() == 1 => return self.remove_table(&id),
            TableEdit::RemoveRow => {
                // The caret keeps its column, in the row that comes up into this one's place.
                let near = all.get(row + 1).or(row.checked_sub(1).and_then(|above| all.get(above)));
                to = near.and_then(|near| cells(near).get(column).map(|cell| cell.id.clone()));
                ops.push(Op::RemoveBlock { block: all[row].id.clone() });
            }
            TableEdit::RemoveColumn => {
                columns.remove(column);
                let beside = cells(all[row]);
                let near = beside.get(column + 1).or(column.checked_sub(1).and_then(|left| beside.get(left)));
                to = near.map(|cell| cell.id.clone());
                let gone =
                    |each: &&Block| cells(each).get(column).map(|cell| Op::RemoveBlock { block: cell.id.clone() });
                ops.extend(all.iter().filter_map(gone));
            }
            TableEdit::MoveRow { to: at } => ops = shift(table, &BlockKind::TableRow, row, at.min(all.len() - 1)),
            TableEdit::MoveColumn { to: at } => {
                let at = at.min(columns.len() - 1);
                let align = columns.remove(column);
                columns.insert(at, align);
                ops.extend(all.iter().flat_map(|each| shift(each, &BlockKind::TableCell, column, at)));
            }
            TableEdit::Align(align) => columns[column] = align,
            TableEdit::Header(on) => header = on,
        }
        if was != (columns.clone(), header) {
            ops.push(Op::SetKind { block: id, kind: BlockKind::Table { columns, header } });
        }
        if self.apply(ops)
            && let Some(cell) = to
        {
            let end = self.block(&cell).map_or(0, text::len);
            self.place(Pos::new(cell, end));
        }
    }
}
