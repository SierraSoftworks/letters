//! A table of contents on the page: a press on one of its lines goes to the heading the line lists, and a
//! chip in its corner, which shows while the pointer is over the table, sets how deep the table lists.
//!
//! No caret stands in a table of contents, so the window asks this module what a press on one is on.

use letters_layout::ContentsBox;
use letters_model::BlockId;
use letters_render::{DisplayList, Item, Point, Rect};

use crate::color::resolve;
use crate::icon;
use crate::pictures;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Menu, MenuItem};
use crate::window::Message;

/// What the chip of a table of contents is called.
pub(crate) const LABEL: &str = "Table of contents options";

/// What each depth a table can list to is called, from Heading 1 alone.
const DEPTHS: [&str; 4] = ["Heading 1 only", "Headings 1 and 2", "Headings 1 to 3", "Headings 1 to 4"];

/// The size of the sign in the chip, which is drawn as the cloud of a linked picture is.
const SIGN: f32 = 16.0;
/// How far a line's fill reaches beyond the line on each side.
const REACH: f32 = 4.0;

/// What is asked of a table of contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    /// Shows the menu of a table, as a press on its chip does.
    Open(BlockId),
    /// Has the table whose menu is out list the headings down to this level.
    Levels(u8),
    /// Escape or a press elsewhere.
    Close,
}

/// What of a table of contents is at a point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// The line that lists this heading.
    Line(BlockId),
    Chip,
}

/// The tables of contents of the page, as the pointer and the menu have them.
#[derive(Default)]
pub(crate) struct Contents {
    /// The table whose menu is out.
    pub open: Option<BlockId>,
    /// The table under the pointer, and what of it.
    pub over: Option<(BlockId, Part)>,
}

/// Where the sign of a table's chip stands: in its corner, level with its first line, in the room a sheet
/// keeps for the number of a page.
pub(crate) fn badge(table: &ContentsBox) -> Rect {
    let first = table.lines.first().map_or(table.rect, |(_, line)| *line);
    let x = table.rect.x + table.rect.width - SIGN - REACH;
    Rect::new(x, first.y + (first.height - SIGN) / 2.0, SIGN, SIGN)
}

/// What of a table is at a point of the layout. The chip is over the end of the first line.
pub(crate) fn part_at(table: &ContentsBox, at: Point) -> Option<Part> {
    if pictures::pressed(badge(table), at) {
        return Some(Part::Chip);
    }
    let line = table.lines.iter().find(|(_, line)| line.contains(at));
    line.map(|(heading, _)| Part::Line(heading.clone()))
}

/// The menu on a table's chip, for a table that lists down to `levels`.
pub(crate) fn menu(levels: u8) -> Menu<Message> {
    let mut menu = Menu::new().label("Table of contents").heading("Lists");
    for (level, name) in (1..).zip(DEPTHS) {
        let item = MenuItem::new(name).checked(level == levels);
        menu = menu.item(item.on_select(Message::Contents(Listing::Levels(level))));
    }
    menu
}

impl Contents {
    /// Takes what the pointer is over now. Returns whether what shows changes with it.
    pub fn point(&mut self, found: Option<(BlockId, Part)>) -> bool {
        let changed = self.over != found;
        self.over = found;
        changed
    }

    /// Whether the pointer is on something of a table that a press does something to.
    pub fn pointed(&self) -> bool {
        self.over.is_some()
    }

    /// Shows what a press on a table would do: the line under the pointer is filled, and the table under
    /// the pointer, or with its menu out, has its chip. `accent` is the person's accent as it is written
    /// on a surface.
    pub fn paint(&self, table: &ContentsBox, accent: Ink, scheme: Scheme, out: &mut DisplayList) {
        let over = self.over.as_ref().filter(|(block, _)| *block == table.block).map(|(_, part)| part);
        let open = self.open.as_ref() == Some(&table.block);
        if let Some(Part::Line(heading)) = over
            && let Some((_, line)) = table.lines.iter().find(|(listed, _)| listed == heading)
        {
            let rect = Rect::new(line.x - REACH, line.y, line.width + 2.0 * REACH, line.height);
            let color = resolve(tokens::NEUTRAL_SOFT, scheme, 1.0);
            out.push(Item::Fill { rect, color, radius: tokens::RADIUS_CONTROL });
        }
        if over.is_some() || open {
            let lit = open || over == Some(&Part::Chip);
            pictures::sign(badge(table), icon::LIST_TREE, lit, (tokens::FG_MUTED, accent), scheme, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> ContentsBox {
        let line = |heading: &str, y: f32| (BlockId::new(heading), Rect::new(40.0, y, 400.0, 24.0));
        ContentsBox {
            block: BlockId::new("toc"),
            rect: Rect::new(40.0, 100.0, 400.0, 48.0),
            lines: vec![line("a", 100.0), line("b", 124.0)],
        }
    }

    #[test]
    fn a_point_on_a_table_is_on_a_line_or_on_the_chip_at_the_end_of_the_first() {
        let table = table();
        // A point, and what of the table is there.
        let cases = [
            ((60.0, 110.0), Some(Part::Line(BlockId::new("a")))),
            ((60.0, 130.0), Some(Part::Line(BlockId::new("b")))),
            ((428.0, 112.0), Some(Part::Chip)),
            // The end of any other line is that line's.
            ((428.0, 140.0), Some(Part::Line(BlockId::new("b")))),
            ((60.0, 160.0), None),
            ((20.0, 110.0), None),
        ];
        for ((x, y), part) in cases {
            assert_eq!(part_at(&table, Point { x, y }), part, "at {x}, {y}");
        }
    }

    #[test]
    fn a_table_with_nothing_to_list_still_has_its_chip() {
        let empty = ContentsBox { lines: Vec::new(), ..table() };
        let at = badge(&empty);
        let middle = Point { x: at.x + at.width / 2.0, y: at.y + at.height / 2.0 };
        assert_eq!(part_at(&empty, middle), Some(Part::Chip));
        assert!(empty.rect.contains(middle), "the chip stands in the table");
    }
}
