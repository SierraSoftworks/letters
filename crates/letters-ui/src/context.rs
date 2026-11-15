//! The menu about what is under the pointer: what a press of the secondary button, or the Menu key, asks for.
//!
//! A strip of small buttons heads it: the clipboard and the marks of text, and in a table what is done with
//! the caret's row and its column. Under the strip the actions of whatever the menu is about are listed by
//! name, each with its icon and each group under its heading. A choice among several, as of what kind a
//! block is, is a menu of its own beside its row. Nearly all of them are commands of the palette, named and
//! keyed as there, and listed only where they have something to act on.

use accesskit::{HasPopup, Role};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use letters_editor::{CalloutEdit, Command, Editor, Key, Keymap, Motion, Picture, TableEdit};
use letters_layout::DocLayout;
use letters_model::{Align as ColumnAlign, Block, BlockId, BlockKind, ListKind, Mark};
use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, Point, Rect};

use crate::commands::{self, Act, Entry};
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::notes::Noting;
use crate::safe::Safely;
use crate::spelling::Fix;
use crate::style::{Align, Dir, Edges};
use crate::text::{Family, TextShaper, TextStyle};
use crate::tokens;
use crate::widgets::{Button, Menu, MenuItem, Popover, Side};
use crate::window::Message;

/// What the menu is called.
pub(crate) const LABEL: &str = "Context menu";

/// The room the strip and the window's edges take from the height the list may have.
const AROUND: f32 = 164.0;
/// The least of the list that shows in a window too low for all of it.
const LEAST: f32 = 96.0;
/// How many pixels a picture of a block may have before it is made at its own size, not at twice that.
const PIXELS: f32 = 16_000_000.0;

/// What the menu asks for that no command of the palette does.
#[derive(Clone, Debug, PartialEq)]
pub enum Contexting {
    /// Puts the menu away: the one that was opened as the so-manieth, if it is still the one that is open.
    Close(u32),
    /// Copies a block's text as it is written: the code of a code block, or the source of a formula.
    CopyText(BlockId),
    /// Copies a code block as a picture of how the page shows it.
    CopyPicture(BlockId),
    /// Copies the address of the link under the caret.
    CopyLink(String),
    /// Brings the figure that a reference names into view.
    ShowFigure(BlockId),
}

/// The menu that is open: where it hangs, and what was under the pointer besides the caret's own block.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Context {
    /// Where it was asked for, in the page area.
    pub at: Point,
    /// The figure that a reference there names.
    pub reference: Option<BlockId>,
    /// The open comment on a passage there.
    pub note: Option<String>,
    /// Which of the menus opened in this window it is, so that one opened from another is a new one.
    pub serial: u32,
}

/// What the window knows that the menu is made from.
pub(crate) struct About<'a> {
    pub editor: &'a Editor,
    /// What a command asks for where the caret is, as the palette is told: `None` while it has nothing
    /// to act on.
    pub asked: &'a dyn Fn(&Act) -> Option<Message>,
    pub mac: bool,
    /// Whether the document is the person's to change. Someone who only reads it has what copies and
    /// what leads somewhere, and a comment where they may write one.
    pub writes: bool,
    /// The misspelled word there, with its corrections.
    pub fix: Option<&'a Fix>,
    /// How high the page area is.
    pub room: f32,
}

impl About<'_> {
    fn keymap(&self) -> &Keymap {
        self.editor.keymap()
    }

    /// The caret's block and those it is inside of, the innermost first.
    fn around(&self) -> Vec<&Block> {
        let mut chain = Vec::new();
        if let Some(selection) = self.editor.selection() {
            find(&self.editor.doc().blocks, &selection.head.block, &mut chain);
        }
        chain
    }

    /// A button of the strip for a command: disabled while the command has nothing to act on.
    fn tool(&self, icon: Icon, act: Act, pressed: bool, closes: bool) -> Element<Message> {
        self.marked(icon, None, act, pressed, closes)
    }

    /// A button of the strip whose icon has a `mark` of another colour drawn over it.
    fn marked(
        &self,
        icon: Icon,
        mark: Option<(Icon, tokens::Color)>,
        act: Act,
        pressed: bool,
        closes: bool,
    ) -> Element<Message> {
        let name = commands::entry_of(&act).map_or("", |entry| entry.name);
        let mut button = Button::icon_only(icon, name).subtle().small().pressed(pressed);
        if let Some((mark, color)) = mark {
            button = button.mark(mark, color);
        }
        button = match (self.asked)(&act) {
            Some(message) => button.on_press(message),
            None => button.disabled(true),
        };
        let told = match commands::shortcut(&act, self.keymap(), self.mac) {
            Some(key) => format!("{name} ({key})"),
            None => name.to_owned(),
        };
        let button = Element::from(button).tooltip(told, Side::Top);
        if closes { button.closes() } else { button }
    }

    /// A row of the list for a command, where it has something to act on.
    fn item(&self, entry: &'static Entry) -> Option<MenuItem<Message>> {
        let message = (self.asked)(&entry.act)?;
        let item = MenuItem::new(entry.name).on_select(message);
        let item = match icon_of(&entry.act) {
            Some(icon) => item.icon(icon),
            None => item,
        };
        Some(match commands::shortcut(&entry.act, self.keymap(), self.mac) {
            Some(key) => item.aside(key),
            None => item,
        })
    }

    /// The rows for the commands that do `acts`, in that order.
    fn items(&self, acts: &[Act]) -> Vec<MenuItem<Message>> {
        acts.iter().filter_map(|act| self.item(commands::entry_of(act)?)).collect()
    }
}

/// The icon a command has beside its name.
fn icon_of(act: &Act) -> Option<Icon> {
    Some(match act {
        Act::Run(command) => match command {
            Command::Indent => icon::INDENT,
            Command::Outdent => icon::OUTDENT,
            Command::ToggleChecked => icon::LIST_CHECKS,
            Command::PastePlain => icon::CLIPBOARD_PASTE,
            Command::SelectAll => icon::SCAN_TEXT,
            Command::EditCaption => icon::TYPE,
            Command::SetNumbered(_) => icon::TAG,
            Command::ShiftFigure { right: false } => icon::ARROW_LEFT,
            Command::ShiftFigure { right: true } => icon::ARROW_RIGHT,
            Command::InsertTable { .. } => icon::TABLE,
            Command::Table(TableEdit::Header(_)) => icon::TABLE,
            Command::Callout(CalloutEdit::Reset) => icon::ROTATE_CCW,
            Command::Callout(CalloutEdit::Remove) => icon::PILCROW,
            Command::SetBlock(kind) => match kind {
                BlockKind::Paragraph => icon::PILCROW,
                BlockKind::Heading { level: 1 } => icon::HEADING_1,
                BlockKind::Heading { level: 2 } => icon::HEADING_2,
                BlockKind::Heading { level: 3 } => icon::HEADING_3,
                BlockKind::Heading { .. } => icon::TYPE,
                BlockKind::ListItem { list: ListKind::Bullet, .. } => icon::LIST,
                BlockKind::ListItem { list: ListKind::Ordered, .. } => icon::LIST_ORDERED,
                BlockKind::ListItem { list: ListKind::Task { .. }, .. } => icon::LIST_CHECKS,
                BlockKind::Quote => icon::TEXT_QUOTE,
                BlockKind::Code { .. } => icon::CODE_XML,
                BlockKind::Callout { .. } => icon::INFO,
                BlockKind::Formula => icon::SIGMA,
                BlockKind::Divider => icon::MINUS,
                _ => return None,
            },
            _ => return None,
        },
        Act::Link | Act::Callout => icon::PENCIL,
        Act::CodeLanguage => icon::CODE_XML,
        Act::AltText => icon::EYE,
        Act::Pictures => icon::IMAGE,
        Act::Load => icon::CLOUD_DOWNLOAD,
        Act::AlwaysLoad => icon::SHIELD,
        Act::Embed | Act::EmbedAll => icon::DOWNLOAD,
        Act::Misspelling { back: false } => icon::CHEVRON_DOWN,
        Act::Misspelling { back: true } => icon::CHEVRON_UP,
        Act::Find => icon::TEXT_SEARCH,
        Act::Replace => icon::REPLACE,
        Act::Palette => icon::SEARCH,
        _ => return None,
    })
}

/// Finds a block by its id and lists it and the blocks around it, the innermost first.
fn find<'a>(blocks: &'a [Block], id: &BlockId, chain: &mut Vec<&'a Block>) -> bool {
    for block in blocks {
        if block.id == *id || find(block.body.blocks(), id, chain) {
            chain.push(block);
            return true;
        }
    }
    false
}

/// A thin upright line between two groups of the strip.
fn divider() -> Element<Message> {
    let line = Element::row().width(1.0).height(tokens::CONTROL_HEIGHT_SM - 10.0).bg(tokens::LINE);
    Element::row().padding(Edges::xy(tokens::SPACE_2XS, 0.0)).child(line).decorative()
}

/// A row of the strip.
fn bar(label: &str) -> Element<Message> {
    Element::row().align(Align::Center).gap(1.0).role(Role::Toolbar).label(label).arrows(Dir::Row)
}

/// A group of the list under its heading, where it has any rows.
fn section(menu: Menu<Message>, heading: &str, items: Vec<MenuItem<Message>>) -> Menu<Message> {
    if items.is_empty() {
        return menu;
    }
    items.into_iter().fold(menu.heading(heading), Menu::item)
}

impl Context {
    /// The menu, to hang from where it was asked for.
    pub fn menu(&self, about: &About) -> Popover<Message> {
        let editor = about.editor;
        let around = about.around();
        let kind = around.first().map(|block| &block.kind);
        let picture = editor.picture().is_some();
        let run = |command: Command| Act::Run(command);

        // The clipboard, the marks of text, and what is made of a selection.
        let marks = editor.marks();
        let mark =
            |icon, mark: Mark| about.tool(icon, run(Command::ToggleMark(mark.clone())), marks.contains(&mark), false);
        let first = bar("Clipboard and format")
            .child(about.tool(icon::SCISSORS, run(Command::Cut), false, true))
            .child(about.tool(icon::COPY, run(Command::Copy), false, true))
            .child(about.tool(icon::CLIPBOARD_PASTE, run(Command::Paste), false, true))
            .child(divider())
            .child(mark(icon::BOLD, Mark::Bold))
            .child(mark(icon::ITALIC, Mark::Italic))
            .child(mark(icon::UNDERLINE, Mark::Underline))
            .child(mark(icon::STRIKETHROUGH, Mark::Strike))
            .child(mark(icon::HIGHLIGHTER, Mark::Highlight))
            .child(mark(icon::CODE, Mark::Code))
            .child(divider())
            .child(about.tool(icon::LINK, Act::Link, editor.link().is_some(), true))
            .child(about.tool(icon::MESSAGE_SQUARE_PLUS, Act::Comment, false, true));
        let first = match about.writes {
            true => first,
            false => {
                let comment = (about.asked)(&Act::Comment).is_some();
                let comment = comment.then(|| about.tool(icon::MESSAGE_SQUARE_PLUS, Act::Comment, false, true));
                bar("Clipboard").child(about.tool(icon::COPY, run(Command::Copy), false, true)).children(comment)
            }
        };
        let mut strip = Element::column().gap(tokens::SPACE_2XS).padding(Edges::all(tokens::SPACE_XS)).child(first);

        let cell = editor.cell();
        let fixed = matches!(kind, Some(BlockKind::Figure { .. } | BlockKind::Caption | BlockKind::Gallery { .. }));
        if let Some(cell) = cell.as_ref().filter(|_| about.writes) {
            // What is done with the caret's row, and then with its column: each icon shows the row or the
            // column and where the new one goes, which way it moves, or that it is taken away.
            let table = |edit: TableEdit| run(Command::Table(edit));
            let (add, cut) = (tokens::OK, tokens::DANGER);
            let adds = |icon, mark, edit| about.marked(icon, Some((mark, add)), table(edit), false, false);
            let cuts = |icon, mark, edit| about.marked(icon, Some((mark, cut)), table(edit), false, false);
            let moves = |icon, act| about.tool(icon, act, false, false);
            let mut rows = bar("Row")
                .child(adds(icon::ROW_LOW, icon::MARK_PLUS_TOP, TableEdit::AddRow { below: false }))
                .child(moves(icon::ROW_UP, Act::MoveRow { down: false }))
                .child(cuts(icon::ROW_SHORT, icon::MARK_CROSS_RIGHT, TableEdit::RemoveRow))
                .child(moves(icon::ROW_DOWN, Act::MoveRow { down: true }))
                .child(adds(icon::ROW_HIGH, icon::MARK_PLUS_BOTTOM, TableEdit::AddRow { below: true }));
            // The header row is turned the way it is not.
            let header = [true, false].into_iter().map(|on| (on, table(TableEdit::Header(on))));
            if let Some((on, act)) = header.into_iter().find(|(_, act)| (about.asked)(act).is_some()) {
                rows = rows.child(divider()).child(about.tool(icon::TABLE, act, !on, false));
            }
            // How the caret's column lines its text up.
            let lined = match editor.doc().block(&cell.table).map(|table| &table.kind) {
                Some(BlockKind::Table { columns, .. }) => {
                    columns.get(cell.column).copied().unwrap_or(ColumnAlign::Start)
                }
                _ => ColumnAlign::Start,
            };
            let line = |icon, align| about.tool(icon, table(TableEdit::Align(align)), lined == align, false);
            let columns = bar("Column")
                .child(adds(icon::COLUMN_RIGHT, icon::MARK_PLUS_LEFT, TableEdit::AddColumn { after: false }))
                .child(moves(icon::COLUMN_TO_LEFT, Act::MoveColumn { right: false }))
                .child(cuts(icon::COLUMN_SHORT, icon::MARK_CROSS_BOTTOM, TableEdit::RemoveColumn))
                .child(moves(icon::COLUMN_TO_RIGHT, Act::MoveColumn { right: true }))
                .child(adds(icon::COLUMN_LEFT, icon::MARK_PLUS_RIGHT, TableEdit::AddColumn { after: true }))
                .child(divider())
                .child(line(icon::TEXT_ALIGN_START, ColumnAlign::Start))
                .child(line(icon::TEXT_ALIGN_CENTER, ColumnAlign::Center))
                .child(line(icon::TEXT_ALIGN_END, ColumnAlign::End));
            strip = strip.child(rows).child(columns);
        }
        // What kind of block the caret's is, and the kinds it can be turned into.
        let is = |to: &BlockKind| match to {
            BlockKind::ListItem { list, .. } => {
                matches!(kind, Some(BlockKind::ListItem { list: at, .. })
                    if std::mem::discriminant(at) == std::mem::discriminant(list))
            }
            BlockKind::Code { .. } => matches!(kind, Some(BlockKind::Code { .. })),
            // A line in a quote is body text that a quote goes round.
            BlockKind::Quote => around.iter().any(|block| block.kind == BlockKind::Quote),
            to => kind == Some(to),
        };
        let mut kinds = Menu::new().label("Turn into");
        let mut turns = false;
        if cell.is_none() && !picture && !fixed && kind.is_some() {
            for entry in commands::group("Text") {
                let Act::Run(Command::SetBlock(to)) = &entry.act else { continue };
                if let Some(item) = about.item(entry) {
                    kinds = kinds.item(item.checked(is(to)));
                    turns = true;
                }
            }
        }

        let send = |asked: Contexting| Message::Context(asked);
        let mut menu = Menu::new().label(LABEL).leads();
        if let Some(fix) = about.fix {
            // Each correction is taken with the key of its place in the list.
            let correct = |(at, word): (usize, &String)| {
                let key = if about.mac { format!("\u{2318}{}", at + 1) } else { format!("Ctrl+{}", at + 1) };
                MenuItem::new(word).aside(key).on_select(Message::Correct(word.clone()))
            };
            let mut items: Vec<_> = fix.suggestions.iter().enumerate().map(correct).collect();
            if items.is_empty() {
                items.push(MenuItem::new("No suggestions").disabled(true));
            }
            let learn = |name: &str, mine| MenuItem::new(name).icon(icon::PLUS).on_select(Message::Learn { mine });
            items.push(learn("Add to this document", false));
            items.push(learn("Add to my dictionary", true));
            items.extend(about.items(&[Act::Misspelling { back: false }, Act::Misspelling { back: true }]));
            menu = section(menu, "Spelling", items);
        }
        if let Some(address) = editor.link().filter(|_| !picture) {
            let mut items = vec![
                MenuItem::new("Open link")
                    .icon(icon::EXTERNAL_LINK)
                    .on_select(Message::Safe(Safely::Follow(address.to_owned()))),
            ];
            if let Some(edit) = (about.asked)(&Act::Link) {
                let item = MenuItem::new("Edit link").icon(icon::PENCIL).on_select(edit);
                items.push(match commands::shortcut(&Act::Link, about.keymap(), about.mac) {
                    Some(key) => item.aside(key),
                    None => item,
                });
            }
            let copy = MenuItem::new("Copy link address").icon(icon::COPY);
            items.push(copy.on_select(send(Contexting::CopyLink(address.to_owned()))));
            if about.writes {
                let remove = MenuItem::new("Remove link").icon(icon::X);
                items.push(remove.on_select(Message::Run(Command::SetLink(None))));
            }
            menu = section(menu, "Link", items);
        }
        if let Some(figure) = &self.reference {
            let go =
                MenuItem::new("Go to figure").icon(icon::IMAGE).on_select(send(Contexting::ShowFigure(figure.clone())));
            menu = section(menu, "Reference", vec![go]);
        }
        if let Some(note) = &self.note {
            let show = MenuItem::new("Show comment")
                .icon(icon::MESSAGE_SQUARE)
                .on_select(Message::Note(Noting::Open(note.clone())));
            menu = section(menu, "Comment", vec![show]);
        }

        let block = around.first().map(|block| block.id.clone());
        match (kind, block) {
            _ if picture => {}
            (Some(BlockKind::Code { .. }), Some(block)) => {
                let mut items = vec![
                    MenuItem::new("Copy code").icon(icon::COPY).on_select(send(Contexting::CopyText(block.clone()))),
                    MenuItem::new("Copy as picture").icon(icon::IMAGE).on_select(send(Contexting::CopyPicture(block))),
                ];
                items.extend(about.items(&[Act::CodeLanguage, run(Command::Indent), run(Command::Outdent)]));
                menu = section(menu, "Code", items);
            }
            (Some(BlockKind::Formula), Some(block)) => {
                let copy =
                    MenuItem::new("Copy TeX source").icon(icon::COPY).on_select(send(Contexting::CopyText(block)));
                menu = section(menu, "Formula", vec![copy]);
            }
            (Some(BlockKind::ListItem { .. }), _) => {
                let acts = [run(Command::Indent), run(Command::Outdent), run(Command::ToggleChecked)];
                menu = section(menu, "List", about.items(&acts));
            }
            _ => {}
        }
        if about.writes && around.iter().any(|block| matches!(block.kind, BlockKind::Callout { .. })) {
            // The types are a choice of one, in a menu of their own. The one the callout has does nothing.
            let mut types = Menu::new().label("Callout type");
            let mut items = Vec::new();
            for entry in commands::group("Callout") {
                if matches!(entry.act, Act::Run(Command::Callout(CalloutEdit::Tone(_)))) {
                    let has = about.item(entry).is_none();
                    types = types.item(about.item(entry).unwrap_or_else(|| MenuItem::new(entry.name)).checked(has));
                } else {
                    items.extend(about.item(entry));
                }
            }
            items.insert(items.len().min(1), MenuItem::new("Type").icon(icon::INFO).submenu(types));
            menu = section(menu, "Callout", items);
        }
        if picture || matches!(kind, Some(BlockKind::Figure { .. })) {
            let mut items: Vec<_> = commands::group("Image").filter_map(|entry| about.item(entry)).collect();
            if picture && about.writes {
                let delete = MenuItem::new("Delete image").icon(icon::TRASH_2).danger();
                items.push(delete.on_select(Message::Run(Command::Delete(Motion::Right))));
            }
            menu = section(menu, "Image", items);
        }
        // An empty line is where something is put in.
        let empty =
            around.first().is_some_and(|block| block.kind == BlockKind::Paragraph && block.plain_text().is_empty());
        if empty && around.len() == 1 && !picture {
            menu = section(menu, "Insert", commands::group("Insert").filter_map(|entry| about.item(entry)).collect());
        }
        menu = menu.separator();
        if turns {
            menu = menu.item(MenuItem::new("Turn into").icon(icon::PILCROW).submenu(kinds));
        }
        let more = [run(Command::PastePlain), run(Command::SelectAll), Act::Find, Act::Replace, Act::SelectionHistory];
        for item in about.items(&more) {
            menu = menu.item(item);
        }
        if let Some(palette) = (about.asked)(&Act::Palette) {
            let all = MenuItem::new("All commands").icon(icon::SEARCH).on_select(palette);
            menu = menu.separator().item(match commands::shortcut(&Act::Palette, about.keymap(), about.mac) {
                Some(key) => all.aside(key),
                None => all,
            });
        }

        // With the menu open, the keys the list writes beside the corrections take them, and the keys that
        // step through the misspelled words go on doing so. A number alone is text, which the window takes.
        let picks: Vec<String> = about.fix.map(|fix| fix.suggestions.clone()).unwrap_or_default();
        let step = [false, true].map(|back| (about.asked)(&Act::Misspelling { back }));
        let keys = move |event: &letters_editor::KeyEvent| match event.key {
            Key::F(7) if !event.mods.primary && !event.mods.alt => step[usize::from(event.mods.shift)].clone(),
            Key::Char(digit) if event.mods.primary && !event.mods.alt && !event.mods.shift => {
                let place = digit.to_digit(10)?.checked_sub(1)?;
                picks.get(place as usize).cloned().map(Message::Correct)
            }
            _ => None,
        };

        let rule = Element::row().height(1.0).bg(tokens::LINE).decorative();
        let list = Element::from(menu).max_height((about.room - AROUND).max(LEAST)).scroll();
        let content = Element::column().key(("context menu", self.serial)).child(strip).child(rule).child(list);
        let spot = Rect::new(self.at.x, self.at.y, 0.0, 0.0);
        let menu = Popover::new(content.on_key(keys)).list(HasPopup::Menu).max_width(f32::MAX).gap(2.0).at(spot);
        menu.open(true).on_close(send(Contexting::Close(self.serial)))
    }
}

/// What stands behind the card in a picture of code.
const BACKDROP: Color = Color { r: 171, g: 184, b: 195, a: 255 };
/// The three lights at the head of the card, as a window on a Mac has them.
const LIGHTS: [(u8, u8, u8); 3] = [(255, 95, 86), (255, 189, 46), (39, 201, 63)];
/// The backdrop that shows round the card.
const MARGIN: f32 = 36.0;
/// The head of the card, which holds the lights.
const HEAD: f32 = 34.0;
const CORNER: f32 = 10.0;
/// The least width of the code's part of the card.
const NARROWEST: f32 = 260.0;
/// From the card's left edge to the line numbers, and from them to the code's own box.
const INSET: f32 = 16.0;

/// A code block as a picture made to be shown: its box as a card with a window's lights and a shadow, on
/// a backdrop, with its lines numbered. `code` is the block's box in the layout and `lines` the caret at
/// the start of each line of it. The picture is a PNG at twice its size where that is not too much, or
/// `None` when there is nothing to show.
pub(crate) fn picture(
    layout: &DocLayout,
    images: &Images,
    shaper: &dyn TextShaper,
    code: Rect,
    lines: &[Rect],
    ink: Color,
    page: Color,
) -> Option<Picture> {
    let mut painted = DisplayList::default();
    layout.paint(code, &mut painted);
    // The block's own box becomes the card, which is wider than it by the numbers.
    let near = |a: f32, b: f32| (a - b).abs() < 1.5;
    let boxed = painted.items.iter().position(|item| {
        matches!(item, Item::Fill { rect, .. } if near(rect.x, code.x) && near(rect.y, code.y) && near(rect.width, code.width))
    });
    // The box may be a wash over the page, and the card stands on no page.
    let over = |wash: Color| {
        let part = |under: u8, over: u8| {
            ((u16::from(under) * (255 - u16::from(wash.a)) + u16::from(over) * u16::from(wash.a)) / 255) as u8
        };
        Color { r: part(page.r, wash.r), g: part(page.g, wash.g), b: part(page.b, wash.b), a: 255 }
    };
    let surface = match boxed.map(|at| painted.items.remove(at)) {
        Some(Item::Fill { color, .. }) => over(color),
        _ => Color { a: 255, ..page },
    };
    // The card is as wide as the longest line needs, and no narrower than a window is.
    let ends = painted.items.iter().filter_map(|item| match item {
        Item::Glyphs { origin, size, glyphs, .. } => Some(origin.x + glyphs.last()?.x + size * 0.62),
        _ => None,
    });
    let written = ends.fold(code.x, f32::max) - code.x;
    let code = Rect { width: (written + INSET).clamp(NARROWEST.min(code.width), code.width), ..code };
    let size = painted.items.iter().find_map(|item| match item {
        Item::Glyphs { size, .. } => Some(*size),
        _ => None,
    });
    let style =
        TextStyle { family: Family::Mono, size: size.unwrap_or(13.0), weight: 400, italic: false, leading: 1.0 };
    let numbers: Vec<_> = (1..=lines.len()).map(|number| shaper.shape(&number.to_string(), &style, None)).collect();
    let widest = numbers.iter().map(|number| number.size.width).fold(0.0, f32::max);
    let gutter = if lines.is_empty() { 0.0 } else { INSET + widest };

    let card = Rect::new(MARGIN, MARGIN, gutter + code.width, HEAD + code.height + 6.0);
    let (wide, high) = (card.width + 2.0 * MARGIN, card.height + 2.0 * MARGIN);
    let mut list = DisplayList::default();
    let shade = Color { r: 0, g: 0, b: 0, a: 96 };
    list.push(Item::Shadow { rect: card, color: shade, blur: 40.0, offset: Point { x: 0.0, y: 16.0 }, radius: CORNER });
    list.push(Item::Fill { rect: card, color: surface, radius: CORNER });
    for (at, (r, g, b)) in LIGHTS.into_iter().enumerate() {
        let rect = Rect::new(card.x + INSET + at as f32 * 20.0, card.y + 13.0, 12.0, 12.0);
        list.push(Item::Fill { rect, color: Color { r, g, b, a: 255 }, radius: 6.0 });
    }
    let by = Point { x: card.x + gutter - code.x, y: card.y + HEAD - code.y };
    list.push(Item::PushClip { rect: Rect::new(card.x + gutter, card.y + HEAD, code.width, code.height), radius: 0.0 });
    let from = list.items.len();
    list.items.append(&mut painted.items);
    list.translate(from, by);
    list.push(Item::PopClip);
    // The numbers are quiet beside the code, and end where they all end.
    let quiet = Color { a: 96, ..ink };
    for (number, line) in numbers.iter().zip(lines) {
        let top = line.y + by.y + (line.height - number.size.height) / 2.0;
        let left = card.x + INSET + widest - number.size.width;
        for run in &number.runs {
            list.push(Item::Glyphs {
                origin: Point { x: left + run.origin.x, y: top + run.origin.y },
                font: run.font.clone(),
                size: run.size,
                color: quiet,
                skew: run.skew,
                embolden: run.embolden,
                glyphs: run.glyphs.clone(),
                text: None,
            });
        }
    }

    let scale = if wide * high * 4.0 > PIXELS { 1.0 } else { 2.0 };
    let (width, height) = ((wide * scale).ceil() as u32, (high * scale).ceil() as u32);
    let mut frame = Frame::new(width, height);
    if frame.width() == 0 || frame.height() == 0 {
        return None;
    }
    // Over an opaque backdrop every pixel is opaque, so the frame's bytes are plain RGBA.
    CpuRenderer::new().render(&list, images, scale, BACKDROP, &mut frame);
    let (width, height) = (frame.width(), frame.height());
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes).write_image(frame.data(), width, height, ExtendedColorType::Rgba8).ok()?;
    Some(Picture { bytes, mime: "image/png".into(), width, height })
}
