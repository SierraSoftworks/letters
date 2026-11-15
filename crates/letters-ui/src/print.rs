//! Print view: the document on sheets of its paper, as it prints.
//!
//! The window swaps its bar's content for this view's and shows the sheets where the page was. The layout
//! pages the document as it does for the PDF and paints each sheet. This module stands the sheets side by
//! side at the size the zoom asks for, and scrolls through them.

use std::rc::Rc;

use accesskit::{Node, NodeId, Role};
use letters_editor::{Key, KeyEvent};
use letters_layout::{DocLayout, Pages, Palette};
use letters_model::Pos;
use letters_render::image::Images;
use letters_render::{DisplayList, Item, Point, Rect, Size};

use crate::color::resolve;
use crate::copying::{CopyFormat, Copying};
use crate::describe;
use crate::element::{Element, Paint, Pointer};
use crate::icon;
use crate::paint::scrollbar;
use crate::pictures;
use crate::text::{TextShaper, TextStyle};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Button, Menu, MenuItem, Segmented, Side, TopBar};
use crate::window::Message;

/// The room kept round the sheets, between two sheets of a row, and between two rows.
const EDGE: f32 = tokens::SPACE_XL;
const BESIDE: f32 = tokens::SPACE_2XL;
const BETWEEN: f32 = tokens::SPACE_XL;
/// The line under a sheet that says which page it is: how far under, and how tall.
const CAPTION: (f32, f32) = (tokens::SPACE_SM, 18.0);
/// The smallest a sheet shows, as a share of its paper.
const LEAST: f32 = 0.1;
/// How far an arrow key scrolls.
const STEP: f32 = 48.0;
/// The keys of the bar's two forms, which are none of the editing bar's.
const FORMS: (usize, usize) = (10, 11);

/// What is asked of Print view.
#[derive(Clone, Debug, PartialEq)]
pub enum Printing {
    /// Shows the document on its sheets.
    Show,
    /// Goes back to editing.
    Back,
    Zoom(Zoom),
    /// The wheel turned over the sheets.
    Scroll(Point),
    /// Asks the shell to print.
    Print,
}

/// How large the sheets show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Zoom {
    /// Two whole sheets side by side.
    #[default]
    Two,
    /// One whole sheet.
    One,
    /// A sheet as large as its paper.
    Full,
}

impl Zoom {
    const ALL: [(Zoom, &'static str); 3] = [(Zoom::Two, "Two pages"), (Zoom::One, "One page"), (Zoom::Full, "100%")];

    /// How many sheets stand in a row, of `count` in all.
    fn across(self, count: usize) -> usize {
        if self == Zoom::Two { count.clamp(1, 2) } else { 1 }
    }
}

/// Where the sheets stand in what scrolls.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spread {
    /// How large a sheet shows, as a share of its paper.
    scale: f32,
    sheet: Size,
    /// How many sheets a row holds.
    across: usize,
    /// The left edge of a row, and how far it is from one row's top to the next.
    left: f32,
    pitch: f32,
    /// The size of everything that scrolls.
    content: Size,
}

impl Spread {
    fn of(paper: Size, count: usize, zoom: Zoom, room: Size) -> Self {
        let across = zoom.across(count);
        let gaps = (across - 1) as f32 * BESIDE;
        let wide = (room.width - 2.0 * EDGE - gaps) / (across as f32 * paper.width);
        let tall = (room.height - 2.0 * EDGE - CAPTION.0 - CAPTION.1) / paper.height;
        // Whole sheets fit the room, and none shows larger than its paper.
        let scale = if zoom == Zoom::Full { 1.0 } else { wide.min(tall).clamp(LEAST, 1.0) };
        let sheet = Size { width: (paper.width * scale).round(), height: (paper.height * scale).round() };
        let row = across as f32 * sheet.width + gaps;
        let pitch = sheet.height + CAPTION.0 + CAPTION.1 + BETWEEN;
        let rows = count.div_ceil(across).max(1) as f32;
        let content = Size { width: row + 2.0 * EDGE, height: rows * pitch - BETWEEN + 2.0 * EDGE };
        let left = ((room.width.max(content.width) - row) / 2.0).round();
        Self { scale, sheet, across, left, pitch, content }
    }

    /// Where sheet `index` stands before anything is scrolled.
    fn rect(&self, index: usize) -> Rect {
        let (row, column) = (index / self.across, index % self.across);
        let x = self.left + column as f32 * (self.sheet.width + BESIDE);
        Rect::new(x, EDGE + row as f32 * self.pitch, self.sheet.width, self.sheet.height)
    }

    /// The most that scrolls: rows down, and pixels across.
    fn reach(&self, room: Size) -> (f32, f32) {
        ((self.content.height - room.height).max(0.0) / self.pitch, (self.content.width - room.width).max(0.0))
    }
}

/// What Print view keeps while it shows.
pub(crate) struct Print {
    zoom: Zoom,
    /// How far down the sheets are scrolled, in rows of them: the same sheets stay in view when the window
    /// or the zoom changes how tall a row is.
    down: f32,
    /// How far across, in pixels, where a sheet is wider than the window.
    across: f32,
    /// The place in the document that was being read, whose sheet the first frame brings.
    from: Option<Pos>,
    /// How the sheets stood in the last frame, and the room they had.
    spread: Option<Spread>,
    room: Size,
    count: usize,
    /// The sheets the last frame showed, each with where it stood in the view.
    shown: Vec<(usize, Rect)>,
}

impl Print {
    /// Print view, to open at the sheet that holds `from`.
    pub fn new(from: Option<Pos>) -> Self {
        Self {
            zoom: Zoom::default(),
            down: 0.0,
            across: 0.0,
            from,
            spread: None,
            room: Size::default(),
            count: 1,
            shown: Vec::new(),
        }
    }

    /// Changes how large the sheets show. The sheet that led the top row is in the top row still.
    pub fn zoom(&mut self, zoom: Zoom) {
        let first = self.down.floor() as usize * self.zoom.across(self.count);
        self.down = (first / zoom.across(self.count)) as f32;
        (self.zoom, self.across) = (zoom, 0.0);
    }

    pub fn scroll(&mut self, delta: Point) {
        let Some(spread) = self.spread else { return };
        self.down += delta.y / spread.pitch;
        self.across += delta.x;
        self.keep(&spread);
    }

    /// Scrolls for a key that moves through the sheets, and says whether `event` was one.
    pub fn key(&mut self, event: &KeyEvent) -> bool {
        let Some(spread) = self.spread else { return false };
        if event.mods.primary || event.mods.alt {
            return false;
        }
        // Rows that fit the window are stepped whole, so each press shows the next sheets entire.
        let whole = spread.pitch <= self.room.height;
        let screen = 0.9 * self.room.height / spread.pitch;
        match event.key {
            Key::PageDown if whole => self.down = (self.down + 0.01).floor() + 1.0,
            Key::PageUp if whole => self.down = (self.down - 0.01).ceil() - 1.0,
            Key::PageDown => self.down += screen,
            Key::PageUp => self.down -= screen,
            Key::Down => self.down += STEP / spread.pitch,
            Key::Up => self.down -= STEP / spread.pitch,
            Key::Right => self.across += STEP,
            Key::Left => self.across -= STEP,
            Key::Home => self.down = 0.0,
            Key::End => self.down = f32::MAX,
            _ => return false,
        }
        self.keep(&spread);
        true
    }

    /// Keeps the scroll within what there is to show.
    fn keep(&mut self, spread: &Spread) {
        let (down, across) = spread.reach(self.room);
        (self.down, self.across) = (self.down.clamp(0.0, down), self.across.clamp(0.0, across));
    }

    /// The sheets, for the area where the page was. Also returns the pictures on them that the shell has
    /// not handed over yet, which keep their place.
    pub fn view(
        &mut self,
        layout: &DocLayout,
        pages: &Pages,
        room: Size,
        images: &Images,
        shaper: &dyn TextShaper,
        scheme: Scheme,
    ) -> (Element<Message>, Vec<String>) {
        let count = pages.sheets.len().max(1);
        let spread = Spread::of(Size { width: pages.width, height: pages.height }, count, self.zoom, room);
        (self.count, self.room) = (count, room);
        if let Some(sheet) = self.from.take().and_then(|from| layout.page_of(pages, &from)) {
            self.down = (sheet / spread.across) as f32;
        }
        self.keep(&spread);
        self.spread = Some(spread);
        let scrolled = Point { x: self.across.round(), y: (self.down * spread.pitch).round() };

        let color = |ink: Ink| resolve(ink, scheme, 1.0);
        let style = TextStyle::control().size(tokens::TEXT_SM);
        let (mut list, mut awaited) = (DisplayList::default(), Vec::new());
        self.shown.clear();
        for index in 0..count {
            let at = spread.rect(index);
            let at = Rect { x: at.x - scrolled.x, y: at.y - scrolled.y, ..at };
            if at.y + at.height + CAPTION.0 + CAPTION.1 <= 0.0 {
                continue;
            }
            if at.y >= room.height {
                break;
            }
            self.shown.push((index, at));
            // Paper is white whatever the window's theme.
            let radius = 2.0;
            for shadow in tokens::SHADOW_CARD {
                let offset = Point { x: shadow.x, y: shadow.y };
                list.push(Item::Shadow { rect: at, color: color(shadow.color), blur: shadow.blur, offset, radius });
            }
            list.push(Item::Fill { rect: at, color: Palette::LIGHT.page, radius });
            let mut sheet = DisplayList::default();
            layout.paint_page(pages, index, &mut sheet);
            awaited.extend(pictures::awaited(images, Scheme::Light, &mut sheet));
            sheet.scale(0, spread.scale);
            sheet.translate(0, Point { x: at.x, y: at.y });
            list.push(Item::PushClip { rect: at, radius });
            list.items.append(&mut sheet.items);
            list.push(Item::PopClip);
            list.push(Item::Stroke { rect: at, color: color(tokens::CARD_LINE), width: 1.0, radius });

            let said = shaper.shape(&caption(index, count), &style, None);
            let x = at.x + ((at.width - said.size.width) / 2.0).round();
            let y = at.y + at.height + CAPTION.0 + ((CAPTION.1 - said.size.height) / 2.0).round();
            said.paint(Point { x, y }, color(tokens::FG_MUTED), &mut list);
        }

        let sheets = Sheets { list, content: spread.content.height, scrolled: scrolled.y };
        let canvas = Element::canvas(Rc::new(sheets))
            .grow(1.0)
            .role(Role::Document)
            .label("Print view")
            .default_focus()
            .on_pointer(|event| match event {
                Pointer::Scroll { delta } => Some(Message::Print(Printing::Scroll(delta))),
                _ => None,
            });
        (canvas, awaited)
    }

    /// Fills the bar while Print view shows: the way back and what the sheets are of, then the paper, the
    /// zoom and the two ways out to paper. A narrow window has it folded, with the zoom in a menu. `told`
    /// is what there is to say of a copy that was just written, which the button that asked for it says.
    pub fn bar(
        &self,
        bar: TopBar<Message>,
        name: &str,
        told: Option<&str>,
        paper: &str,
        papers: impl Fn() -> Menu<Message>,
    ) -> TopBar<Message> {
        let back = Message::Print(Printing::Back);
        let name = || Element::text(name, TextStyle::control().weight(600)).min_width(tokens::SPACE_3XL).shrink();
        let pages = if self.count == 1 { "1 page".to_owned() } else { format!("{} pages", self.count) };
        let count = Element::text(format!("Print view, {pages}"), TextStyle::control().size(tokens::TEXT_SM));
        // Where a copy was just written is said under the button that asks for one.
        let export = |button: Button<Message>| {
            let button = Element::from(button.small().on_press(Message::Copy(Copying::Open(CopyFormat::Pdf))));
            match told {
                Some(told) => button.tooltip(told, Side::Bottom).telling(true),
                None => button,
            }
        };
        let print =
            || Button::new("Print").icon(icon::PRINTER).primary().small().on_press(Message::Print(Printing::Print));

        let mut zoom = Segmented::new("Zoom").small();
        let mut zooms = Menu::new().label("Zoom");
        for (to, label) in Zoom::ALL {
            let pick = Message::Print(Printing::Zoom(to));
            zoom = zoom.segment(label, self.zoom == to, pick.clone());
            zooms = zooms.item(MenuItem::new(label).checked(self.zoom == to).on_select(pick));
        }
        let shown = Zoom::ALL.iter().find(|(zoom, _)| *zoom == self.zoom).map_or("Zoom", |(_, label)| label);

        let bar = bar
            .apart()
            .label("Print view")
            .form(FORMS.0)
            .start(Button::new("Back to editing").icon(icon::ARROW_LEFT).small().on_press(back.clone()))
            .start(name())
            .start(count.fg(tokens::FG_MUTED))
            .middle(Element::from(Button::new(paper).icon(icon::FILE_TEXT).small().menu(papers())).description("Paper"))
            .middle(zoom)
            .middle(TopBar::rule())
            .middle(export(Button::new("Export PDF").icon(icon::FILE_DOWN)))
            .middle(print());
        bar.form(FORMS.1)
            .start(Button::icon_only(icon::ARROW_LEFT, "Back to editing").small().on_press(back))
            .start(name())
            .middle(
                Element::from(Button::icon_only(icon::FILE_TEXT, "Paper").small().menu(papers())).description(paper),
            )
            .middle(Element::from(Button::new(shown).small().menu(zooms)).description("Zoom"))
            .middle(TopBar::rule())
            .middle(export(Button::icon_only(icon::FILE_DOWN, "Export PDF")))
            .middle(print())
    }

    /// Tells assistive technology of the sheets in view, under the canvas that shows them.
    pub fn describe(&self, canvas: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
        for (index, at) in &self.shown {
            describe::sheet(*index, &caption(*index, self.count), *at, canvas, nodes);
        }
    }
}

/// What stands under a sheet: `Page 2 of 3`.
fn caption(index: usize, count: usize) -> String {
    format!("Page {} of {count}", index + 1)
}

/// The sheets in view as the last frame drew them, placed in the area that shows them.
struct Sheets {
    list: DisplayList,
    /// The height of everything that scrolls, and how far it is scrolled.
    content: f32,
    scrolled: f32,
}

impl Paint for Sheets {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        out.push(Item::Fill { rect, color: resolve(tokens::BG, scheme, 1.0), radius: 0.0 });
        // A sheet that is half scrolled away ends at the edge of its area, not over the bar.
        out.push(Item::PushClip { rect, radius: 0.0 });
        let first = out.items.len();
        out.items.extend(self.list.items.iter().cloned());
        out.translate(first, Point { x: rect.x, y: rect.y });
        out.push(Item::PopClip);
        out.items.extend(scrollbar(rect, self.content, self.scrolled, scheme, 1.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: Size = Size { width: 793.7, height: 1122.5 };

    #[test]
    fn whole_sheets_fit_the_room_and_none_is_larger_than_its_paper() {
        // The zoom, the room, how many sheets stand in a row, and whether a sheet shows as large as its paper.
        let cases = [
            (Zoom::Two, Size { width: 1280.0, height: 756.0 }, 2, false),
            (Zoom::One, Size { width: 1280.0, height: 756.0 }, 1, false),
            (Zoom::Two, Size { width: 390.0, height: 700.0 }, 2, false),
            (Zoom::One, Size { width: 390.0, height: 700.0 }, 1, false),
            (Zoom::Two, Size { width: 3000.0, height: 2000.0 }, 2, true),
            (Zoom::Full, Size { width: 390.0, height: 700.0 }, 1, true),
            (Zoom::Full, Size { width: 1280.0, height: 756.0 }, 1, true),
        ];
        for (zoom, room, across, full) in cases {
            let spread = Spread::of(A4, 5, zoom, room);
            assert_eq!(spread.across, across, "{zoom:?} in {room:?}");
            assert_eq!(spread.scale == 1.0, full, "{zoom:?} in {room:?}");
            let (first, last) = (spread.rect(0), spread.rect(across - 1));
            if zoom != Zoom::Full {
                assert!(
                    first.x >= EDGE - 1.0 && last.x + last.width <= room.width - EDGE + 1.0,
                    "{zoom:?} in {room:?}"
                );
                assert!(first.y + first.height + CAPTION.0 + CAPTION.1 <= room.height, "{zoom:?} in {room:?}");
            }
            // The rows are centred, and the last sheet ends above the room kept under it.
            assert!((first.x - (room.width.max(spread.content.width) - last.x - last.width)).abs() <= 1.0);
            let end = spread.rect(4);
            assert!((spread.content.height - (end.y + end.height + CAPTION.0 + CAPTION.1 + EDGE)).abs() <= 1.0);
        }
    }

    #[test]
    fn a_document_of_one_page_has_its_sheet_in_the_middle() {
        let room = Size { width: 1280.0, height: 756.0 };
        let spread = Spread::of(A4, 1, Zoom::Two, room);
        let sheet = spread.rect(0);
        assert_eq!(spread.across, 1);
        assert!((sheet.x + sheet.width / 2.0 - room.width / 2.0).abs() <= 1.0);
    }

    #[test]
    fn a_change_of_zoom_keeps_the_sheet_that_led_the_view() {
        // The zoom before and the row at the top, the zoom after, and the row at the top then.
        let cases = [
            (Zoom::Two, 3.0, Zoom::One, 6.0),
            (Zoom::One, 6.0, Zoom::Two, 3.0),
            (Zoom::One, 7.4, Zoom::Two, 3.0),
            (Zoom::One, 5.0, Zoom::Full, 5.0),
        ];
        for (from, down, to, expected) in cases {
            let mut print = Print { zoom: from, down, count: 20, ..Print::new(None) };
            print.zoom(to);
            assert_eq!((print.zoom, print.down), (to, expected), "{from:?} at {down} to {to:?}");
        }
    }
}
