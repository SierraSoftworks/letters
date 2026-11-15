//! The menu about what is under the pointer: what a press of the secondary button, or the Menu key, asks for.
//!
//! A strip of small buttons heads it: the clipboard, the marks of text, and the kind of the block or how a
//! table's column lines up. Under the strip the actions of whatever the menu is about are listed by name,
//! each group under its heading. Nearly all of them are commands of the palette, named and keyed as there,
//! and listed only where they have something to act on.

use accesskit::{HasPopup, Role};
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use letters_editor::{Command, Editor, Keymap, Motion, Picture, TableEdit};
use letters_layout::DocLayout;
use letters_model::{Align as ColumnAlign, Block, BlockId, BlockKind, ListKind, Mark};
use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Point, Rect};

use crate::commands::{self, Act, Entry};
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::notes::Noting;
use crate::safe::Safely;
use crate::spelling::Fix;
use crate::style::{Align, Dir, Edges};
use crate::tokens;
use crate::widgets::{Button, Menu, MenuItem, Popover, Side};
use crate::window::Message;

/// What the menu is called.
pub(crate) const LABEL: &str = "Context menu";

/// The room the strip and the window's edges take from the height the list may have.
const AROUND: f32 = 132.0;
/// The least of the list that shows in a window too low for all of it.
const LEAST: f32 = 96.0;
/// How many pixels a picture of a block may have before it is made at its own size, not at twice that.
const PIXELS: f32 = 16_000_000.0;

/// What the menu asks for that no command of the palette does.
#[derive(Clone, Debug, PartialEq)]
pub enum Contexting {
    /// Puts the menu away.
    Close,
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
}

/// What the window knows that the menu is made from.
pub(crate) struct About<'a> {
    pub editor: &'a Editor,
    /// What a command asks for where the caret is, as the palette is told: `None` while it has nothing
    /// to act on.
    pub asked: &'a dyn Fn(&Act) -> Option<Message>,
    pub mac: bool,
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
        let name = commands::entry_of(&act).map_or("", |entry| entry.name);
        let mut button = Button::icon_only(icon, name).subtle().small().pressed(pressed);
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
        let mut strip = Element::column().gap(tokens::SPACE_2XS).padding(Edges::all(tokens::SPACE_XS)).child(first);

        let cell = editor.cell();
        let fixed = matches!(kind, Some(BlockKind::Figure { .. } | BlockKind::Caption | BlockKind::Gallery { .. }));
        if let Some(cell) = &cell {
            // How the caret's column lines its text up.
            let lined = match editor.doc().block(&cell.table).map(|table| &table.kind) {
                Some(BlockKind::Table { columns, .. }) => {
                    columns.get(cell.column).copied().unwrap_or(ColumnAlign::Start)
                }
                _ => ColumnAlign::Start,
            };
            let line =
                |icon, align| about.tool(icon, run(Command::Table(TableEdit::Align(align))), lined == align, false);
            let second = bar("Column")
                .child(line(icon::TEXT_ALIGN_START, ColumnAlign::Start))
                .child(line(icon::TEXT_ALIGN_CENTER, ColumnAlign::Center))
                .child(line(icon::TEXT_ALIGN_END, ColumnAlign::End));
            strip = strip.child(second);
        } else if !picture && !fixed && kind.is_some() {
            // What kind of block the caret's is.
            let listed = |list: &ListKind| match kind {
                Some(BlockKind::ListItem { list: at, .. }) => {
                    std::mem::discriminant(at) == std::mem::discriminant(list)
                }
                _ => false,
            };
            let block = |icon, to: BlockKind| {
                let is = match &to {
                    BlockKind::ListItem { list, .. } => listed(list),
                    BlockKind::Code { .. } => matches!(kind, Some(BlockKind::Code { .. })),
                    // A line in a quote is body text that a quote goes round.
                    BlockKind::Quote => around.iter().any(|block| block.kind == BlockKind::Quote),
                    to => kind == Some(to),
                };
                about.tool(icon, run(Command::SetBlock(to)), is, false)
            };
            let list = |icon, list| block(icon, BlockKind::ListItem { list, depth: 0 });
            let second = bar("Block")
                .child(block(icon::PILCROW, BlockKind::Paragraph))
                .child(block(icon::HEADING_1, BlockKind::Heading { level: 1 }))
                .child(block(icon::HEADING_2, BlockKind::Heading { level: 2 }))
                .child(block(icon::HEADING_3, BlockKind::Heading { level: 3 }))
                .child(divider())
                .child(list(icon::LIST, ListKind::Bullet))
                .child(list(icon::LIST_ORDERED, ListKind::Ordered))
                .child(list(icon::LIST_CHECKS, ListKind::Task { checked: false }))
                .child(divider())
                .child(block(icon::TEXT_QUOTE, BlockKind::Quote))
                .child(block(icon::CODE_XML, BlockKind::Code { language: None }));
            strip = strip.child(second);
        }

        let send = |asked: Contexting| Message::Context(asked);
        let mut menu = Menu::new().label(LABEL).leads();
        if let Some(fix) = about.fix {
            let correct = |word: &String| MenuItem::new(word).on_select(Message::Correct(word.clone()));
            let mut items: Vec<_> = fix.suggestions.iter().map(correct).collect();
            if items.is_empty() {
                items.push(MenuItem::new("No suggestions").disabled(true));
            }
            items.push(MenuItem::new("Add to this document").on_select(Message::Learn { mine: false }));
            items.push(MenuItem::new("Add to my dictionary").on_select(Message::Learn { mine: true }));
            menu = section(menu, "Spelling", items);
        }
        if let Some(address) = editor.link().filter(|_| !picture) {
            let mut items =
                vec![MenuItem::new("Open link").on_select(Message::Safe(Safely::Follow(address.to_owned())))];
            if let Some(edit) = (about.asked)(&Act::Link) {
                let item = MenuItem::new("Edit link").on_select(edit);
                items.push(match commands::shortcut(&Act::Link, about.keymap(), about.mac) {
                    Some(key) => item.aside(key),
                    None => item,
                });
            }
            items.push(MenuItem::new("Copy link address").on_select(send(Contexting::CopyLink(address.to_owned()))));
            items.push(MenuItem::new("Remove link").on_select(Message::Run(Command::SetLink(None))));
            menu = section(menu, "Link", items);
        }
        if let Some(figure) = &self.reference {
            let go = MenuItem::new("Go to figure").on_select(send(Contexting::ShowFigure(figure.clone())));
            menu = section(menu, "Reference", vec![go]);
        }
        if let Some(note) = &self.note {
            let show = MenuItem::new("Show comment").on_select(Message::Note(Noting::Open(note.clone())));
            menu = section(menu, "Comment", vec![show]);
        }

        let block = around.first().map(|block| block.id.clone());
        match (kind, block) {
            _ if picture => {}
            (Some(BlockKind::Code { .. }), Some(block)) => {
                let mut items = vec![
                    MenuItem::new("Copy code").on_select(send(Contexting::CopyText(block.clone()))),
                    MenuItem::new("Copy as picture").on_select(send(Contexting::CopyPicture(block))),
                ];
                items.extend(about.items(&[run(Command::Indent), run(Command::Outdent)]));
                menu = section(menu, "Code", items);
            }
            (Some(BlockKind::Formula), Some(block)) => {
                let copy = MenuItem::new("Copy TeX source").on_select(send(Contexting::CopyText(block)));
                menu = section(menu, "Formula", vec![copy]);
            }
            (Some(BlockKind::ListItem { .. }), _) => {
                let acts = [run(Command::Indent), run(Command::Outdent), run(Command::ToggleChecked)];
                menu = section(menu, "List", about.items(&acts));
            }
            _ => {}
        }
        if cell.is_some() {
            // The strip lines the column up.
            let listed = |entry: &&Entry| !matches!(entry.act, Act::Run(Command::Table(TableEdit::Align(_))));
            menu = section(
                menu,
                "Table",
                commands::group("Table").filter(listed).filter_map(|entry| about.item(entry)).collect(),
            );
        }
        if around.iter().any(|block| matches!(block.kind, BlockKind::Callout { .. })) {
            menu = section(menu, "Callout", commands::group("Callout").filter_map(|entry| about.item(entry)).collect());
        }
        if picture || matches!(kind, Some(BlockKind::Figure { .. })) {
            let mut items: Vec<_> = commands::group("Image").filter_map(|entry| about.item(entry)).collect();
            if picture {
                let delete = MenuItem::new("Delete image").danger();
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
        for item in about.items(&[run(Command::PastePlain), run(Command::SelectAll)]) {
            menu = menu.item(item);
        }
        if let Some(palette) = (about.asked)(&Act::Palette) {
            let all = MenuItem::new("All commands").on_select(palette);
            menu = menu.separator().item(match commands::shortcut(&Act::Palette, about.keymap(), about.mac) {
                Some(key) => all.aside(key),
                None => all,
            });
        }

        let rule = Element::row().height(1.0).bg(tokens::LINE).decorative();
        let list = Element::from(menu).max_height((about.room - AROUND).max(LEAST)).scroll();
        let content = Element::column().child(strip).child(rule).child(list);
        let spot = Rect::new(self.at.x, self.at.y, 0.0, 0.0);
        let menu = Popover::new(content).list(HasPopup::Menu).max_width(f32::MAX).gap(2.0).at(spot);
        menu.open(true).on_close(send(Contexting::Close))
    }
}

/// A part of the page as a PNG, at twice its size where that is not too much: `rect` of the layout, over
/// `page`. `None` when there is nothing there to show.
pub(crate) fn picture(layout: &DocLayout, images: &Images, rect: Rect, page: Color) -> Option<Picture> {
    let mut list = DisplayList::default();
    layout.paint(rect, &mut list);
    list.translate(0, Point { x: -rect.x, y: -rect.y });
    let scale = if rect.width * rect.height * 4.0 > PIXELS { 1.0 } else { 2.0 };
    let (width, height) = ((rect.width * scale).ceil() as u32, (rect.height * scale).ceil() as u32);
    let mut frame = Frame::new(width, height);
    if frame.width() == 0 || frame.height() == 0 {
        return None;
    }
    // Over an opaque page every pixel is opaque, so the frame's bytes are plain RGBA.
    CpuRenderer::new().render(&list, images, scale, Color { a: 255, ..page }, &mut frame);
    let (width, height) = (frame.width(), frame.height());
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes).write_image(frame.data(), width, height, ExtendedColorType::Rgba8).ok()?;
    Some(Picture { bytes, mime: "image/png".into(), width, height })
}
