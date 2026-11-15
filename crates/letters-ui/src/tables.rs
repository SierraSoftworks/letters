//! Tables on the page: the handles that add, remove and move their rows and columns.
//!
//! A table the pointer is over, or the caret is in, has a handle over one column and one beside one row:
//! those under the pointer, or else those of the caret. A press on a handle opens the menu of its column
//! or row, and a drag carries it to another place among them. A bar along the right of the table adds a
//! column at its end, and one under it a row. On the borders either side of the column and the row that
//! have a handle is a plus, which adds one there at a press. The page is drawn on a canvas, so all of it is drawn there
//! and the window asks this module what a press is on.

use letters_editor::{Command, TableEdit};
use letters_layout::TableBox;
use letters_model::{Align as ColumnAlign, BlockId};
use letters_render::{DisplayList, Item, Point, Rect};

use crate::color::resolve;
use crate::commands::Act;
use crate::icon::{self, GRID};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Menu, MenuItem};
use crate::window::Message;

/// How thick a handle is, and how long.
const THICK: f32 = 10.0;
const LONG: f32 = 28.0;
/// How far the handles and the bars keep from the table.
const GAP: f32 = 2.0;
/// How thick a bar that adds is, and the plus on it.
const BAR: f32 = 10.0;
const PLUS: f32 = 8.0;
/// The side of the plus on a border between two columns or rows.
const INSERT: f32 = 12.0;
/// How near a press has to come to count, as near as a finger comes.
const REACH: f32 = 3.0;
/// The dots on a handle: how big each is, and how far apart they start.
const DOT: f32 = 2.0;
const STEP: f32 = 5.0;
/// How wide the line is that shows where a carried row or column would land.
const LANDING: f32 = 2.0;

/// The columns of a table, or its rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Column,
    Row,
}

/// Something of a table that takes a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// The handle of a column or a row, counted from 0.
    Handle(Axis, usize),
    /// The bar that adds a column at the end, or a row.
    Add(Axis),
    /// The plus on the border before a column or row, which adds one there. Past the last, it adds one
    /// at the end.
    Insert(Axis, usize),
}

/// What a press that has ended asks for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Asked {
    /// The menu of a column or row.
    Menu(Axis, usize),
    /// A column at the end, or a row.
    Add(Axis),
    /// A column or row before the one counted `before`, or at the end when there is none.
    Insert { axis: Axis, before: usize },
    /// The column or row `from` to stand `to`th among them.
    Move { axis: Axis, from: usize, to: usize },
}

/// The table whose handles show, and the column and the row that have one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Shown {
    pub table: TableBox,
    pub column: Option<usize>,
    pub row: Option<usize>,
}

fn grown(rect: Rect, by: f32) -> Rect {
    Rect::new(rect.x - by, rect.y - by, rect.width + 2.0 * by, rect.height + 2.0 * by)
}

/// The table with its handles and bars, and the room between them: what the pointer is over the table in.
fn reach(table: &TableBox) -> Rect {
    let (before, after) = (GAP + THICK + REACH, GAP + BAR + REACH);
    let rect = table.rect;
    Rect::new(rect.x - before, rect.y - before, rect.width + before + after, rect.height + before + after)
}

/// The spans of a table's columns or rows along their axis: where each starts, and how long it is.
fn spans(table: &TableBox, axis: Axis) -> Vec<(f32, f32)> {
    match axis {
        Axis::Column => table.columns.clone(),
        Axis::Row => table.rows.iter().map(|row| (row.y, row.height)).collect(),
    }
}

/// How far along an axis a point is.
fn along(axis: Axis, at: Point) -> f32 {
    match axis {
        Axis::Column => at.x,
        Axis::Row => at.y,
    }
}

/// The column or row a point is in, when it is level with one.
fn index_at(table: &TableBox, axis: Axis, at: Point) -> Option<usize> {
    let at = along(axis, at);
    spans(table, axis).iter().position(|(start, long)| (*start..start + long).contains(&at))
}

/// Where the cell `cell` is in a table: its row and its column.
fn place(table: &TableBox, cell: &BlockId) -> Option<(usize, usize)> {
    let column = |row: &letters_layout::TableRow| row.cells.iter().position(|id| id == cell);
    table.rows.iter().enumerate().find_map(|(at, row)| Some((at, column(row)?)))
}

/// Where the handle of a column or row is.
fn handle(table: &TableBox, axis: Axis, index: usize) -> Option<Rect> {
    let (start, long) = *spans(table, axis).get(index)?;
    // A narrow column still has room for the plus on each of its borders.
    let (length, rect) = (LONG.min(long - INSERT - 2.0 * REACH).max(THICK), table.rect);
    let from = (start + (long - length) / 2.0).round();
    Some(match axis {
        Axis::Column => Rect::new(from, rect.y - GAP - THICK, length, THICK),
        Axis::Row => Rect::new(rect.x - GAP - THICK, from, THICK, length),
    })
}

/// Where the bar is that adds a column at the end, or a row.
fn bar(table: &TableBox, axis: Axis) -> Rect {
    let rect = table.rect;
    match axis {
        Axis::Column => Rect::new(rect.x + rect.width + GAP, rect.y, BAR, rect.height),
        Axis::Row => Rect::new(rect.x, rect.y + rect.height + GAP, rect.width, BAR),
    }
}

/// Where the plus is on the border before the column or row counted `gap`: on the line of the handles.
fn plus(table: &TableBox, axis: Axis, gap: usize) -> Rect {
    let spans = spans(table, axis);
    let edge = spans.get(gap).map(|(start, _)| *start).or(spans.last().map(|(start, long)| start + long));
    let (at, rect) = ((edge.unwrap_or_default() - INSERT / 2.0).round(), table.rect);
    let beside = GAP + THICK / 2.0 + INSERT / 2.0;
    match axis {
        Axis::Column => Rect::new(at, rect.y - beside, INSERT, INSERT),
        Axis::Row => Rect::new(rect.x - beside, at, INSERT, INSERT),
    }
}

/// The column or row a point is in, or the nearest one for a point on the line of their handles, where
/// the plus on an outer border reaches past the table.
fn index_near(table: &TableBox, axis: Axis, at: Point) -> Option<usize> {
    let rect = table.rect;
    let on_line = match axis {
        Axis::Column => at.y < rect.y,
        Axis::Row => at.x < rect.x,
    };
    let last = spans(table, axis).len().checked_sub(1)?;
    let nearest = || gap_at(table, axis, at).saturating_sub(1).min(last);
    index_at(table, axis, at).or_else(|| on_line.then(nearest))
}

/// The place among the columns or rows that a point is nearest to the start of: as many as there are for
/// a point past the middle of the last.
fn gap_at(table: &TableBox, axis: Axis, at: Point) -> usize {
    let at = along(axis, at);
    spans(table, axis).iter().filter(|(start, long)| at > start + long / 2.0).count()
}

/// The line between two columns or rows, before the one counted `gap`.
fn landing(table: &TableBox, axis: Axis, gap: usize) -> Rect {
    let spans = spans(table, axis);
    let edge = spans.get(gap).map(|(start, _)| *start).or(spans.last().map(|(start, long)| start + long));
    let (at, rect) = (edge.unwrap_or_default() - LANDING / 2.0, table.rect);
    match axis {
        Axis::Column => Rect::new(at, rect.y, LANDING, rect.height),
        Axis::Row => Rect::new(rect.x, at, rect.width, LANDING),
    }
}

impl Shown {
    /// The table to show handles on: the one the pointer is over at `pointer`, or else the one with the
    /// cell `caret` in it. The column and the row under the pointer have the handles, and where it is level
    /// with none, those of the caret.
    pub fn find(tables: Vec<TableBox>, pointer: Option<Point>, caret: Option<&BlockId>) -> Option<Self> {
        let over = |table: &TableBox| pointer.filter(|at| reach(table).contains(*at));
        let caret = |table: &TableBox| caret.and_then(|cell| place(table, cell));
        let at = tables.iter().position(|table| over(table).is_some());
        let at = at.or_else(|| tables.iter().position(|table| caret(table).is_some()))?;
        let table = tables.into_iter().nth(at)?;
        let (over, caret) = (over(&table), caret(&table));
        let column = over.and_then(|at| index_near(&table, Axis::Column, at)).or(caret.map(|(_, column)| column));
        let row = over.and_then(|at| index_near(&table, Axis::Row, at)).or(caret.map(|(row, _)| row));
        Some(Self { table, column, row })
    }

    /// Each thing that takes a press, and where it is.
    pub fn parts(&self) -> Vec<(Part, Rect)> {
        let handles = [(Axis::Column, self.column), (Axis::Row, self.row)];
        let handle = |(axis, index): (Axis, Option<usize>)| {
            let index = index?;
            Some((Part::Handle(axis, index), handle(&self.table, axis, index)?))
        };
        let bars = [Axis::Column, Axis::Row].map(|axis| (Part::Add(axis), bar(&self.table, axis)));
        // The borders either side of what has a handle each have a plus.
        let plus = |(axis, index): (Axis, Option<usize>)| {
            let sides = index.map(|index| [index, index + 1]);
            sides.into_iter().flatten().map(move |gap| (Part::Insert(axis, gap), plus(&self.table, axis, gap)))
        };
        let pluses = handles.into_iter().flat_map(plus);
        pluses.chain(handles.into_iter().filter_map(handle)).chain(bars).collect()
    }

    /// What a press at `at` is on.
    pub fn part_at(&self, at: Point) -> Option<Part> {
        self.parts().into_iter().find(|(_, rect)| grown(*rect, REACH).contains(at)).map(|(part, _)| part)
    }

    /// Where a part is.
    pub fn rect(&self, part: Part) -> Option<Rect> {
        self.parts().into_iter().find(|(found, _)| *found == part).map(|(_, rect)| rect)
    }

    /// The cell to put the caret in for something asked of a column or row: in the row the caret is in,
    /// or the column, when it is in this table at `caret`. For a column or row at the end, `index` is `None`.
    pub fn cell(&self, axis: Axis, index: Option<usize>, caret: Option<&BlockId>) -> Option<BlockId> {
        let caret = caret.and_then(|cell| place(&self.table, cell));
        let rows = &self.table.rows;
        match axis {
            Axis::Column => {
                // A row cut short by two writers at once may not reach the column.
                let reaches = |row: &&letters_layout::TableRow| index.is_none_or(|column| column < row.cells.len());
                let row =
                    caret.and_then(|(row, _)| rows.get(row)).filter(reaches).or_else(|| rows.iter().find(reaches))?;
                index.map_or_else(|| row.cells.last(), |column| row.cells.get(column)).cloned()
            }
            Axis::Row => {
                let row = index.map_or_else(|| rows.last(), |row| rows.get(row))?;
                let column = caret.map_or(0, |(_, column)| column);
                row.cells.get(column).or(row.cells.last()).cloned()
            }
        }
    }
}

/// A press that began on a part of a table.
#[derive(Clone, Debug, PartialEq)]
struct Pressed {
    shown: Shown,
    part: Part,
    from: Point,
    /// The place among the columns or rows a carried handle is over, once it has moved.
    over: Option<usize>,
}

/// What the pointer is doing with the handles of the tables on the page.
#[derive(Default)]
pub(crate) struct Tables {
    /// The part the pointer rests on.
    pointed: Option<Part>,
    pressed: Option<Pressed>,
    /// Whether the menu of the caret's column is out, or of its row.
    pub open: Option<Axis>,
}

impl Tables {
    /// Takes the part the pointer rests on now. Returns whether that is news.
    pub fn point(&mut self, part: Option<Part>) -> bool {
        std::mem::replace(&mut self.pointed, part) != part
    }

    /// Whether the pointer rests on something that takes a press.
    pub fn pointed(&self) -> bool {
        self.pointed.is_some()
    }

    /// Notes a press at `from` on a part of the table that shows.
    pub fn press(&mut self, shown: Shown, part: Part, from: Point) {
        self.pressed = Some(Pressed { shown, part, from, over: None });
    }

    /// Whether a press that began on a table's handles is still held, so that what the pointer does is theirs.
    pub fn held(&self) -> bool {
        self.pressed.is_some()
    }

    /// The table a held press began on, which shows as it was then.
    pub fn held_on(&self) -> Option<&Shown> {
        self.pressed.as_ref().map(|pressed| &pressed.shown)
    }

    /// Follows the pointer with the button held. A handle that has moved `slop` is carried.
    pub fn drag(&mut self, at: Point, slop: f32) {
        let Some(pressed) = &mut self.pressed else { return };
        let Part::Handle(axis, _) = pressed.part else { return };
        let moved = (at.x - pressed.from.x).hypot(at.y - pressed.from.y) > slop;
        if moved || pressed.over.is_some() {
            pressed.over = Some(gap_at(&pressed.shown.table, axis, at));
        }
    }

    /// Ends the press with the pointer at `at`. Returns the table it began on and what it asks for: a
    /// carried handle its move, and a press that stayed on its part what the part does.
    pub fn release(&mut self, at: Point) -> Option<(Shown, Asked)> {
        let pressed = self.pressed.take()?;
        let asked = match (pressed.part, pressed.over) {
            (Part::Handle(axis, from), Some(gap)) => {
                // Out from before the others, it lands one place sooner among them.
                let to = if gap > from { gap - 1 } else { gap };
                (to != from).then_some(Asked::Move { axis, from, to })?
            }
            (part, _) if pressed.shown.part_at(at) != Some(part) => return None,
            (Part::Handle(axis, index), _) => Asked::Menu(axis, index),
            (Part::Add(axis), _) => Asked::Add(axis),
            (Part::Insert(axis, before), _) => Asked::Insert { axis, before },
        };
        Some((pressed.shown, asked))
    }

    /// Draws the handles and bars of the table that shows. `accent` marks the handle whose menu is out and
    /// the one that is carried.
    pub fn paint(&self, shown: &Shown, accent: Ink, scheme: Scheme, out: &mut DisplayList) {
        let down = self.pressed.as_ref().map(|pressed| pressed.part);
        let carried = self.pressed.as_ref().and_then(|pressed| Some((pressed.part, pressed.over?)));
        if let Some((Part::Handle(axis, from), gap)) = carried {
            // The column or row that is carried is washed with the accent, and a line shows where it lands.
            let (start, long) = spans(&shown.table, axis).get(from).copied().unwrap_or_default();
            let rect = shown.table.rect;
            let whole = match axis {
                Axis::Column => Rect::new(start, rect.y, long, rect.height),
                Axis::Row => Rect::new(rect.x, start, rect.width, long),
            };
            out.push(Item::Fill { rect: whole, color: resolve(accent, scheme, 0.12), radius: 0.0 });
            let line = landing(&shown.table, axis, gap);
            out.push(Item::Fill { rect: line, color: resolve(accent, scheme, 1.0), radius: LANDING / 2.0 });
        }
        for (part, rect) in shown.parts() {
            let over = self.pointed == Some(part) || down == Some(part);
            match part {
                Part::Handle(axis, _) => {
                    let lit = carried.is_some_and(|(carried, _)| carried == part)
                        || (self.open == Some(axis) && self.pressed.is_none());
                    let (fill, edge, dots) = match (lit, over) {
                        (true, _) => (accent, accent, tokens::SURFACE),
                        (false, true) => (tokens::SURFACE, tokens::FG_MUTED, tokens::FG),
                        (false, false) => (tokens::SURFACE, tokens::LINE_STRONG, tokens::FG_MUTED),
                    };
                    let radius = THICK / 2.0;
                    out.push(Item::Fill { rect, color: resolve(fill, scheme, 1.0), radius });
                    out.push(Item::Stroke { rect, color: resolve(edge, scheme, 1.0), width: 1.0, radius });
                    let middle = Point { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 };
                    for step in [-STEP, 0.0, STEP] {
                        let at = match axis {
                            Axis::Column => Point { x: middle.x + step, y: middle.y },
                            Axis::Row => Point { x: middle.x, y: middle.y + step },
                        };
                        let dot = Rect::new(at.x - DOT / 2.0, at.y - DOT / 2.0, DOT, DOT);
                        out.push(Item::Fill { rect: dot, color: resolve(dots, scheme, 1.0), radius: DOT / 2.0 });
                    }
                }
                Part::Insert(axis, gap) => {
                    // Under the pointer it is lit, with a line where the new column or row would go.
                    let (fill, edge, ink) = match over {
                        true => (accent, accent, tokens::SURFACE),
                        false => (tokens::SURFACE, tokens::LINE_STRONG, tokens::FG_MUTED),
                    };
                    if over {
                        let line = landing(&shown.table, axis, gap);
                        out.push(Item::Fill { rect: line, color: resolve(accent, scheme, 1.0), radius: LANDING / 2.0 });
                    }
                    let radius = INSERT / 2.0;
                    out.push(Item::Fill { rect, color: resolve(fill, scheme, 1.0), radius });
                    out.push(Item::Stroke { rect, color: resolve(edge, scheme, 1.0), width: 1.0, radius });
                    out.push(Item::Path {
                        origin: Point { x: rect.x + (rect.width - PLUS) / 2.0, y: rect.y + (rect.height - PLUS) / 2.0 },
                        scale: PLUS / GRID,
                        path: icon::PLUS.outline(),
                        color: resolve(ink, scheme, 1.0),
                        stroke: Some(tokens::ICON_STROKE),
                    });
                }
                Part::Add(_) => {
                    let (wash, ink) = if over { (1.0, tokens::FG) } else { (0.5, tokens::FG_MUTED) };
                    let radius = tokens::RADIUS_SM;
                    out.push(Item::Fill { rect, color: resolve(tokens::NEUTRAL_SOFT, scheme, wash), radius });
                    out.push(Item::Path {
                        origin: Point { x: rect.x + (rect.width - PLUS) / 2.0, y: rect.y + (rect.height - PLUS) / 2.0 },
                        scale: PLUS / GRID,
                        path: icon::PLUS.outline(),
                        color: resolve(ink, scheme, 1.0),
                        stroke: Some(tokens::ICON_STROKE),
                    });
                }
            }
        }
    }
}

/// What a part is called, for a reader that cannot see it. Columns and rows are counted from 1.
pub(crate) fn label(part: Part) -> String {
    match part {
        Part::Handle(Axis::Column, index) => format!("Column {}", index + 1),
        Part::Handle(Axis::Row, index) => format!("Row {}", index + 1),
        Part::Add(Axis::Column) => "Add column".to_owned(),
        Part::Add(Axis::Row) => "Add row".to_owned(),
        Part::Insert(Axis::Column, before) => format!("Add column before column {}", before + 1),
        Part::Insert(Axis::Row, before) => format!("Add row before row {}", before + 1),
    }
}

/// The menu on the handle of the caret's column, or of its row. `asks` says what a command asks for where
/// the caret is, and `None` of one with nothing to act on. A column is set as `aligned`, and `header` is
/// whether the first row is the table's header, for that row's own menu.
pub(crate) fn menu(
    axis: Axis,
    asks: impl Fn(&Act) -> Option<Message>,
    aligned: Option<ColumnAlign>,
    header: Option<bool>,
) -> Menu<Message> {
    let item = |label: &str, act: Act| {
        let item = MenuItem::new(label);
        match asks(&act) {
            Some(asked) => item.on_select(asked),
            None => item.disabled(true),
        }
    };
    let edit = |edit: TableEdit| Act::Run(Command::Table(edit));
    match axis {
        Axis::Column => {
            let align = |label: &str, icon, align: ColumnAlign| {
                item(label, edit(TableEdit::Align(align))).icon(icon).checked(aligned == Some(align))
            };
            Menu::new()
                .label("Column")
                .item(item("Add column to the left", edit(TableEdit::AddColumn { after: false })))
                .item(item("Add column to the right", edit(TableEdit::AddColumn { after: true })))
                .separator()
                .item(item("Move column left", Act::MoveColumn { right: false }))
                .item(item("Move column right", Act::MoveColumn { right: true }))
                .separator()
                .item(align("Align column left", icon::TEXT_ALIGN_START, ColumnAlign::Start))
                .item(align("Align column centre", icon::TEXT_ALIGN_CENTER, ColumnAlign::Center))
                .item(align("Align column right", icon::TEXT_ALIGN_END, ColumnAlign::End))
                .separator()
                .item(item("Remove column", edit(TableEdit::RemoveColumn)).danger())
        }
        Axis::Row => {
            let mut menu = Menu::new()
                .label("Row")
                .item(item("Add row above", edit(TableEdit::AddRow { below: false })))
                .item(item("Add row below", edit(TableEdit::AddRow { below: true })))
                .separator()
                .item(item("Move row up", Act::MoveRow { down: false }))
                .item(item("Move row down", Act::MoveRow { down: true }));
            if let Some(on) = header {
                menu = menu.separator().item(item("Header row", edit(TableEdit::Header(!on))).checked(on));
            }
            menu.separator().item(item("Remove row", edit(TableEdit::RemoveRow)).danger())
        }
    }
}

#[cfg(test)]
mod tests {
    use letters_layout::TableRow;

    use super::*;

    /// A table of three columns and two rows at (100, 200), its cells named `<row><column>`.
    fn table() -> TableBox {
        let row = |at: usize, y: f32| {
            let cells = (0..3).map(|column| BlockId::new(format!("{at}{column}").as_str())).collect();
            TableRow { y, height: 30.0, cells }
        };
        TableBox {
            rect: Rect::new(100.0, 200.0, 300.0, 60.0),
            columns: vec![(100.0, 120.0), (220.0, 80.0), (300.0, 100.0)],
            rows: vec![row(0, 200.0), row(1, 230.0)],
        }
    }

    fn middle(rect: Rect) -> Point {
        Point { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 }
    }

    #[test]
    fn the_handles_are_those_of_the_pointer_or_else_of_the_caret() {
        let caret = BlockId::new("12");
        // (the pointer, the caret's cell, the column and row with a handle)
        let cases = [
            (Some(Point { x: 250.0, y: 210.0 }), None, Some((Some(1), Some(0)))),
            (Some(Point { x: 250.0, y: 210.0 }), Some(&caret), Some((Some(1), Some(0)))),
            // Over the handles of the columns the pointer is level with no row.
            (Some(Point { x: 110.0, y: 192.0 }), Some(&caret), Some((Some(0), Some(1)))),
            (Some(Point { x: 110.0, y: 192.0 }), None, Some((Some(0), None))),
            (Some(Point { x: 900.0, y: 210.0 }), Some(&caret), Some((Some(2), Some(1)))),
            (None, Some(&caret), Some((Some(2), Some(1)))),
            (Some(Point { x: 900.0, y: 210.0 }), None, None),
            (None, None, None),
            // On the plus of an outer border, just past the table, the column or row at that end keeps it.
            (Some(Point { x: 403.0, y: 192.0 }), None, Some((Some(2), None))),
            (Some(Point { x: 92.0, y: 201.0 }), None, Some((None, Some(0)))),
        ];
        for (pointer, caret, expected) in cases {
            let shown = Shown::find(vec![table()], pointer, caret);
            assert_eq!(shown.map(|shown| (shown.column, shown.row)), expected, "{pointer:?} with {caret:?}");
        }
    }

    #[test]
    fn handles_and_bars_stand_outside_the_table_beside_what_they_act_on() {
        let shown = Shown { table: table(), column: Some(1), row: Some(1) };
        let (rect, parts) = (shown.table.rect, shown.parts());
        assert_eq!(parts.len(), 8, "a handle and two pluses each for the column and the row, and two bars");
        for (part, at) in parts {
            let apart = at.x + at.width <= rect.x
                || at.y + at.height <= rect.y
                || at.x >= rect.x + rect.width
                || at.y >= rect.y + rect.height;
            assert!(apart, "{part:?} keeps clear of the table: {at:?}");
            assert_eq!(shown.part_at(middle(at)), Some(part), "a press on it is its own");
            match part {
                Part::Handle(Axis::Column, _) => assert!((220.0..300.0).contains(&middle(at).x) && at.y < rect.y),
                Part::Handle(Axis::Row, _) => assert!((230.0..260.0).contains(&middle(at).y) && at.x < rect.x),
                Part::Add(Axis::Column) => assert!(at.x >= rect.x + rect.width),
                Part::Add(Axis::Row) => assert!(at.y >= rect.y + rect.height),
                // On the border it adds at: those of the second column are at 220 and 300.
                Part::Insert(Axis::Column, gap) => {
                    assert_eq!((middle(at).x, at.y < rect.y), ([220.0, 300.0][gap - 1], true))
                }
                Part::Insert(Axis::Row, gap) => {
                    assert_eq!((middle(at).y, at.x < rect.x), ([230.0, 260.0][gap - 1], true))
                }
            }
        }
        assert_eq!(shown.part_at(middle(rect)), None, "the table itself is text to write in");
    }

    #[test]
    fn a_press_asks_for_what_its_part_does_and_a_carried_handle_for_its_move() {
        let shown = || Shown { table: table(), column: Some(0), row: Some(0) };
        let column = Part::Handle(Axis::Column, 0);
        let at = |part| middle(shown().rect(part).unwrap());
        let over = |x: f32| Point { x, y: 150.0 };
        let moved = |to| Some(Asked::Move { axis: Axis::Column, from: 0, to });
        // (pressed on, dragged through, let go at, asked)
        let cases = [
            (column, None, at(column), Some(Asked::Menu(Axis::Column, 0))),
            (Part::Add(Axis::Row), None, at(Part::Add(Axis::Row)), Some(Asked::Add(Axis::Row))),
            (Part::Add(Axis::Row), None, middle(table().rect), None),
            (
                Part::Insert(Axis::Row, 1),
                None,
                at(Part::Insert(Axis::Row, 1)),
                Some(Asked::Insert { axis: Axis::Row, before: 1 }),
            ),
            (column, Some(over(270.0)), over(270.0), moved(1)),
            (column, Some(over(390.0)), over(390.0), moved(2)),
            (column, Some(over(1000.0)), over(1000.0), moved(2)),
            // Carried away and back, it stays where it was and opens nothing.
            (column, Some(over(270.0)), over(150.0), None),
            // A bar is not carried.
            (Part::Add(Axis::Row), Some(over(270.0)), at(Part::Add(Axis::Row)), Some(Asked::Add(Axis::Row))),
        ];
        for (part, through, end, asked) in cases {
            let mut tables = Tables::default();
            tables.press(shown(), part, at(part));
            assert!(tables.held());
            for at in through.into_iter().chain([end]) {
                tables.drag(at, 4.0);
            }
            assert_eq!(tables.release(end).map(|(_, asked)| asked), asked, "{part:?} through {through:?} to {end:?}");
            assert!(!tables.held(), "the press is over");
        }
    }

    #[test]
    fn the_caret_goes_to_a_cell_of_the_column_or_row_asked_of() {
        let shown = Shown { table: table(), column: None, row: None };
        let caret = BlockId::new("11");
        // (the axis, the column or row, the caret's cell, the cell it goes to)
        let cases = [
            (Axis::Column, Some(2), Some(&caret), "12"),
            (Axis::Column, Some(2), None, "02"),
            (Axis::Column, None, Some(&caret), "12"),
            (Axis::Row, Some(0), Some(&caret), "01"),
            (Axis::Row, Some(0), None, "00"),
            (Axis::Row, None, None, "10"),
        ];
        for (axis, index, caret, cell) in cases {
            assert_eq!(shown.cell(axis, index, caret), Some(BlockId::new(cell)), "{axis:?} {index:?} from {caret:?}");
        }
    }
}
