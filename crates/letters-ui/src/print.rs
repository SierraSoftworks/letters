//! Print view: the document on sheets of its paper, as it prints.
//!
//! The window swaps its bar's content for this view's and shows the sheets where the page was. The layout
//! pages the document as it does for the PDF and paints each sheet. This module stands the sheets side by
//! side at the size the zoom asks for, and scrolls through them.
//!
//! A page is made to start with a block here and nowhere else. Where one was, the sheet before says so
//! under its last block, with a button that takes the break away. Between two blocks near the pointer a
//! sheet offers to start a page.

use std::rc::Rc;

use accesskit::{Node, NodeId, Role};
use letters_editor::{Key, KeyEvent};
use letters_layout::{DocLayout, Pages, Palette};
use letters_model::{BlockId, Document, Pos};
use letters_render::image::Images;
use letters_render::{DisplayList, Item, PathEl, Point, Rect, Size};

use crate::color::resolve;
use crate::copying::{CopyFormat, Copying};
use crate::describe;
use crate::element::{Cursor, Element, Paint, Pointer};
use crate::icon::{self, Icon};
use crate::paint::scrollbar;
use crate::pictures;
use crate::text::{Shaped, TextShaper, TextStyle};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::widgets::{Button, Menu, MenuItem, Popover, Segmented, Side, TopBar};
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
/// The label of a page break and of the offer of one: how tall it is, the room at its ends and between its
/// parts, its sign, the button that removes a break, and the room it keeps from its line.
const LABEL: f32 = 24.0;
const LABEL_PAD: f32 = 10.0;
const LABEL_GAP: f32 = 6.0;
const SIGN: f32 = 14.0;
const REMOVE: f32 = 20.0;
const BESIDE_LABEL: f32 = tokens::SPACE_SM;
/// How far under the last block of its sheet a break is marked, and what is left blank under it is hatched.
const UNDER: f32 = tokens::SPACE_SM;
/// The line of a break: how thick, and its dashes with the room between them.
const RULE: f32 = 2.0;
const DASH: (f32, f32) = (6.0, 4.0);
/// How far apart the lines of the hatch are.
const HATCH: f32 = 8.0;
/// How near the pointer has to be to the room between two blocks for a break to be offered there.
const REACH: f32 = 32.0;

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
    /// The pointer went down on the sheets, and came up.
    Down(Point),
    Up(Point),
}

/// A change to where pages start: the blocks that start one, or no longer do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Breaking {
    pub blocks: Vec<BlockId>,
    pub on: bool,
}

/// Something on the sheets that takes a press: where it is in the view, the sheet it is on, and what a
/// press asks.
#[derive(Clone, Debug, PartialEq)]
struct Hot {
    rect: Rect,
    sheet: usize,
    asks: Breaking,
}

/// The room between two blocks of a sheet, where a page can be made to start with the second.
#[derive(Clone, Debug, PartialEq)]
struct Gap {
    /// The sheet it is on, and where that stands in the view.
    sheet: usize,
    on: Rect,
    /// The middle of the room, and the stretch of the sheet's text across it.
    y: f32,
    span: (f32, f32),
    block: BlockId,
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

/// The document as it is paged, which the sheets are drawn from.
pub(crate) struct Paged<'a> {
    pub doc: &'a Document,
    pub layout: &'a DocLayout,
    pub pages: &'a Pages,
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
    /// Where the pointer rests on the sheets.
    pointer: Option<Point>,
    /// Where the last frame's sheets can be made to start a page, and what on them takes a press.
    gaps: Vec<Gap>,
    hot: Vec<Hot>,
    /// What a press began on.
    pressed: Option<Breaking>,
    /// Whether the sheets are only looked at: where pages start and how the page is set up are for
    /// someone who writes the document.
    fixed: bool,
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
            pointer: None,
            gaps: Vec::new(),
            hot: Vec::new(),
            pressed: None,
            fixed: false,
        }
    }

    /// Says whether the sheets are only looked at, as by someone who does not write the document.
    pub fn fix(&mut self, fixed: bool) {
        (self.fixed, self.pressed) = (fixed, None);
    }

    /// Notes where the pointer rests, and says whether the sheets show something else for it.
    pub fn point(&mut self, at: Option<Point>) -> bool {
        let shows = |print: &Self| (print.over(), print.pointer.and_then(|at| print.offered(at)));
        let before = shows(self);
        self.pointer = at;
        before != shows(self)
    }

    /// Whether the pointer rests on something that takes a press.
    pub fn pressable(&self) -> bool {
        self.over().is_some()
    }

    /// Notes what a press began on.
    pub fn press(&mut self, at: Point) {
        self.pressed = self.at(at).map(|hot| hot.asks.clone());
    }

    /// What a press asks, when it ends on what it began on.
    pub fn release(&mut self, at: Point) -> Option<Breaking> {
        let pressed = self.pressed.take()?;
        self.at(at).is_some_and(|hot| hot.asks == pressed).then_some(pressed)
    }

    /// What the control that stands for `block` asks, for a press that comes by another way than the
    /// pointer.
    pub fn asked(&self, block: &BlockId) -> Option<Breaking> {
        self.hot.iter().find(|hot| hot.asks.blocks.first() == Some(block)).map(|hot| hot.asks.clone())
    }

    fn at(&self, at: Point) -> Option<&Hot> {
        self.hot.iter().find(|hot| hot.rect.contains(at))
    }

    fn over(&self) -> Option<usize> {
        self.pointer.and_then(|at| self.hot.iter().position(|hot| hot.rect.contains(at)))
    }

    /// The gap that offers a page break to a pointer at `at`: the nearest within reach on the sheet under
    /// it. None while it is on the button that removes a break.
    fn offered(&self, at: Point) -> Option<usize> {
        if self.hot.iter().any(|hot| !hot.asks.on && hot.rect.contains(at)) {
            return None;
        }
        let near = self.gaps.iter().enumerate().filter(|(_, gap)| gap.on.contains(at));
        let near = near.map(|(index, gap)| (index, (gap.y - at.y).abs())).filter(|(_, far)| *far <= REACH);
        near.min_by(|a, b| a.1.total_cmp(&b.1)).map(|(index, _)| index)
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
        paged: Paged<'_>,
        room: Size,
        images: &Images,
        shaper: &dyn TextShaper,
        scheme: Scheme,
    ) -> (Element<Message>, Vec<String>) {
        let Paged { doc, layout, pages } = paged;
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
        let labels = Labels::new(shaper);
        let (mut list, mut awaited) = (DisplayList::default(), Vec::new());
        self.shown.clear();
        self.gaps.clear();
        self.hot.clear();
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

            // Where the sheet's blocks stand says where a page can start, and where one was made to.
            let stand = layout.page_blocks(pages, index);
            let down = |y: f32| at.y + y * spread.scale;
            let span = ((at.x + pages.area.x * spread.scale).round(), (pages.area.width * spread.scale).round());
            for stands in stand.iter().skip(1) {
                let Some(block) = doc.blocks.get(stands.index).filter(|_| stands.starts && !self.fixed) else {
                    continue;
                };
                let y = down((stands.above + stands.top) / 2.0).round();
                self.gaps.push(Gap { sheet: index, on: at, y, span, block: block.id.clone() });
            }
            let made = forced(doc, layout, pages, index + 1);
            if let (Some(last), false) = (stand.last(), made.is_empty()) {
                let end = down(pages.area.y + pages.area.height);
                let y = (down(last.bottom) + UNDER + LABEL / 2.0).min(at.y + at.height - LABEL / 2.0 - 2.0).round();
                // The marker says why the sheet ends here. A press on it takes the break away.
                let over = self.pointer.filter(|_| !self.fixed);
                let remove = labels.marker(Point { x: span.0, y }, span.1, over, &mut list);
                hatch(Rect::new(span.0, y + LABEL / 2.0 + UNDER, span.1, end - y - LABEL / 2.0 - UNDER), &mut list);
                if !self.fixed {
                    self.hot.push(Hot { rect: remove, sheet: index, asks: Breaking { blocks: made, on: false } });
                }
            }
        }
        // The offer lies over its sheet, where the pointer is.
        if let Some(gap) = self.pointer.and_then(|at| self.offered(at)).map(|index| self.gaps[index].clone()) {
            let rect = labels.offer(Point { x: gap.span.0, y: gap.y }, gap.span.1, self.pointer, &mut list);
            self.hot.push(Hot { rect, sheet: gap.sheet, asks: Breaking { blocks: vec![gap.block], on: true } });
        }

        let sheets = Sheets { list, content: spread.content.height, scrolled: scrolled.y };
        let mut canvas = Element::canvas(Rc::new(sheets))
            .grow(1.0)
            .role(Role::Document)
            .label("Print view")
            .default_focus()
            .on_pointer(|event| match event {
                Pointer::Scroll { delta } => Some(Message::Print(Printing::Scroll(delta))),
                Pointer::Down { at, .. } => Some(Message::Print(Printing::Down(at))),
                Pointer::Up { at } => Some(Message::Print(Printing::Up(at))),
                _ => None,
            })
            .pointed();
        if self.pressable() {
            canvas = canvas.cursor(Cursor::Pointer);
        }
        (canvas, awaited)
    }

    /// Fills the bar while Print view shows: the way back and what the sheets are of, then the page's
    /// setup, the zoom and the two ways out to paper. A narrow window has it folded, with the zoom in a menu. `told`
    /// is what there is to say of a copy that was just written, which the button that asked for it says.
    pub fn bar(
        &self,
        bar: TopBar<Message>,
        name: &str,
        told: Option<&str>,
        paper: &str,
        setup: impl Fn() -> Popover<Message>,
    ) -> TopBar<Message> {
        let back = Message::Print(Printing::Back);
        let leave = if self.fixed { "Back to the document" } else { "Back to editing" };
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
            .start(Button::new(leave).icon(icon::ARROW_LEFT).small().on_press(back.clone()))
            .start(name())
            .start(count.fg(tokens::FG_MUTED));
        // The page is set up by someone who writes on it.
        let bar = match self.fixed {
            true => bar,
            false => bar.middle(
                Element::from(Button::new(paper).icon(icon::FILE_TEXT).small().popover(setup()))
                    .description("Page setup"),
            ),
        };
        let bar = bar
            .middle(zoom)
            .middle(TopBar::rule())
            .middle(export(Button::new("Export PDF").icon(icon::FILE_DOWN)))
            .middle(print());
        let bar =
            bar.form(FORMS.1).start(Button::icon_only(icon::ARROW_LEFT, leave).small().on_press(back)).start(name());
        let bar = match self.fixed {
            true => bar,
            false => bar.middle(
                Element::from(Button::icon_only(icon::FILE_TEXT, "Page setup").small().popover(setup()))
                    .description(paper),
            ),
        };
        bar.middle(Element::from(Button::new(shown).small().menu(zooms)).description("Zoom"))
            .middle(TopBar::rule())
            .middle(export(Button::icon_only(icon::FILE_DOWN, "Export PDF")))
            .middle(print())
    }

    /// Tells assistive technology of the sheets in view, under the canvas that shows them.
    pub fn describe(&self, canvas: &mut Node, nodes: &mut Vec<(NodeId, Node)>) {
        for (index, at) in &self.shown {
            describe::sheet(*index, &caption(*index, self.count), *at, canvas, nodes);
        }
        for Hot { rect, sheet, asks } in &self.hot {
            let Some(block) = asks.blocks.first() else { continue };
            let (label, place) = match asks.on {
                true => ("Page break here", format!("On page {}", sheet + 1)),
                false => ("Remove page break", format!("After page {}", sheet + 1)),
            };
            describe::page_break(block, label, &place, *rect, canvas, nodes);
        }
    }
}

/// The blocks that make sheet `index` start where it does: its first block, and the first text it shows,
/// when they are marked to start a page. None for a sheet that starts where the one before was full.
fn forced(doc: &Document, layout: &DocLayout, pages: &Pages, index: usize) -> Vec<BlockId> {
    let Some(sheet) = pages.sheets.get(index).filter(|_| index > 0) else { return Vec::new() };
    let leads = layout.page_blocks(pages, index).into_iter().next().filter(|block| block.starts);
    let outer = leads.and_then(|block| doc.blocks.get(block.index));
    let inner = sheet.blocks.first().filter(|(_, lines)| lines.start == 0).and_then(|(id, _)| doc.block(id));
    let mut made: Vec<BlockId> = Vec::new();
    for block in [outer, inner].into_iter().flatten().filter(|block| block.page_break) {
        if !made.contains(&block.id) {
            made.push(block.id.clone());
        }
    }
    made
}

/// The words on the sheets about page breaks, shaped once for a frame. They are drawn in the colours of
/// the light theme, as paper is white whatever the window's theme.
struct Labels {
    name: Shaped,
    note: Shaped,
    here: Shaped,
}

impl Labels {
    fn new(shaper: &dyn TextShaper) -> Self {
        let strong = TextStyle::control().size(tokens::TEXT_SM).weight(500);
        Self {
            name: shaper.shape("Page break", &strong, None),
            note: shaper.shape("print only", &TextStyle::control().size(tokens::TEXT_XS), None),
            here: shaper.shape("Page break here", &strong, None),
        }
    }

    /// Marks a page break on a dashed line `width` long, whose left end and middle is `at`. Says less
    /// where the line is short. Returns where its remove button is.
    fn marker(&self, at: Point, width: f32, pointer: Option<Point>, out: &mut DisplayList) -> Rect {
        let ink = |ink: Ink| resolve(ink, Scheme::Light, 1.0);
        let bare = LABEL_PAD + SIGN + LABEL_GAP + REMOVE + (LABEL - REMOVE) / 2.0;
        let words = [&self.name, &self.note].map(|word| word.size.width + LABEL_GAP);
        // The name goes before the note does, and a line stays to be seen at both ends.
        let room = width - 2.0 * (BESIDE_LABEL + DASH.0);
        let said = (0..=2).rev().find(|said| bare + words[..*said].iter().sum::<f32>() <= room).unwrap_or(0);
        let label = centred(at, width, bare + words[..said].iter().sum::<f32>());
        rule(at, width, label, Some(DASH), ink(tokens::LINE_STRONG), out);
        out.push(Item::Fill { rect: label, color: ink(tokens::SURFACE), radius: LABEL / 2.0 });
        out.push(Item::Stroke { rect: label, color: ink(tokens::LINE_STRONG), width: 1.0, radius: LABEL / 2.0 });
        let mut x = label.x + LABEL_PAD;
        sign(icon::SEPARATOR_HORIZONTAL, Point { x, y: at.y }, ink(tokens::FG), out);
        x += SIGN + LABEL_GAP;
        for (word, color) in [(&self.name, tokens::FG), (&self.note, tokens::FG_MUTED)].into_iter().take(said) {
            word.paint(Point { x, y: at.y - (word.size.height / 2.0).round() }, ink(color), out);
            x += word.size.width + LABEL_GAP;
        }
        let remove = Rect::new(x, at.y - REMOVE / 2.0, REMOVE, REMOVE);
        let over = pointer.is_some_and(|pointer| remove.contains(pointer));
        if over {
            out.push(Item::Fill { rect: remove, color: ink(tokens::NEUTRAL_SOFT), radius: REMOVE / 2.0 });
        }
        let cross = Point { x: remove.x + (REMOVE - SIGN) / 2.0, y: at.y };
        sign(icon::X, cross, ink(if over { tokens::FG } else { tokens::FG_MUTED }), out);
        remove
    }

    /// Offers a page break on a line `width` long, whose left end and middle is `at`. Returns where its
    /// label is, which takes the press.
    fn offer(&self, at: Point, width: f32, pointer: Option<Point>, out: &mut DisplayList) -> Rect {
        let ink = |ink: Ink| resolve(ink, Scheme::Light, 1.0);
        let bare = 2.0 * LABEL_PAD + SIGN;
        let worded = bare + LABEL_GAP + self.here.size.width;
        let said = worded <= width - 2.0 * (BESIDE_LABEL + DASH.0);
        let label = centred(at, width, if said { worded } else { bare });
        rule(at, width, label, None, ink(tokens::ACCENT_TEXT), out);
        out.push(Item::Fill { rect: label, color: ink(tokens::SURFACE), radius: LABEL / 2.0 });
        if pointer.is_some_and(|pointer| label.contains(pointer)) {
            out.push(Item::Fill { rect: label, color: ink(tokens::ACCENT_SOFT), radius: LABEL / 2.0 });
        }
        out.push(Item::Stroke { rect: label, color: ink(tokens::ACCENT_TEXT), width: 1.0, radius: LABEL / 2.0 });
        let x = label.x + LABEL_PAD;
        sign(icon::PLUS, Point { x, y: at.y }, ink(tokens::ACCENT_TEXT), out);
        if said {
            let origin = Point { x: x + SIGN + LABEL_GAP, y: at.y - (self.here.size.height / 2.0).round() };
            self.here.paint(origin, ink(tokens::ACCENT_TEXT), out);
        }
        label
    }
}

/// Where a label `wide` stands in the middle of a line that starts at `at`.
fn centred(at: Point, width: f32, wide: f32) -> Rect {
    Rect::new((at.x + (width - wide) / 2.0).round(), at.y - LABEL / 2.0, wide.round(), LABEL)
}

/// Draws a line from `at` on both sides of a label, whole or in dashes.
fn rule(
    at: Point,
    width: f32,
    label: Rect,
    dash: Option<(f32, f32)>,
    color: letters_render::Color,
    out: &mut DisplayList,
) {
    let y = at.y - RULE / 2.0;
    let sides = [(at.x, label.x - BESIDE_LABEL), (label.x + label.width + BESIDE_LABEL, at.x + width)];
    for (from, to) in sides {
        let (on, off) = dash.unwrap_or((to - from, 0.0));
        let mut x = from;
        while x < to && on > 0.0 {
            out.push(Item::Fill { rect: Rect::new(x, y, on.min(to - x), RULE), color, radius: 0.0 });
            x += on + off;
        }
    }
}

/// Draws an icon [`SIGN`] large, with its left edge at `at` and its middle at that height.
fn sign(icon: Icon, at: Point, color: letters_render::Color, out: &mut DisplayList) {
    out.push(Item::Path {
        origin: Point { x: at.x, y: at.y - SIGN / 2.0 },
        scale: SIGN / icon::GRID,
        path: icon.outline(),
        color,
        stroke: (!icon.is_filled()).then_some(tokens::ICON_STROKE),
    });
}

/// Hatches the part of a sheet that a page break leaves blank.
fn hatch(rect: Rect, out: &mut DisplayList) {
    if rect.height < 2.0 * HATCH {
        return;
    }
    // Each line climbs to the right, from the left or bottom edge to the top or right one.
    let mut lines = Vec::new();
    let mut along = HATCH;
    while along < rect.width + rect.height {
        let from = Point { x: (along - rect.height).max(0.0), y: along.min(rect.height) };
        let to = Point { x: along.min(rect.width), y: (along - rect.width).max(0.0) };
        lines.extend([PathEl::MoveTo(from), PathEl::LineTo(to)]);
        along += HATCH;
    }
    out.push(Item::PushClip { rect, radius: 3.0 });
    out.push(Item::Path {
        origin: Point { x: rect.x, y: rect.y },
        scale: 1.0,
        path: lines.into(),
        color: resolve(tokens::NEUTRAL_SOFT, Scheme::Light, 1.0),
        stroke: Some(1.0),
    });
    out.push(Item::PopClip);
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
