//! The editor's window: one top bar, the page, and one side panel at a time.
//!
//! [`Window`] is what a shell shows and the test harness drives. The page is a canvas holding what the
//! document's layout painted for the part in view. Everything around it is made of the toolkit's widgets.

use std::rc::Rc;

use accesskit::{ActionRequest, Node as AccessNode, NodeId, Role};
use letters_editor::{Clip, Command, Editor, InputEvent, Key, KeyEvent, Lines, Modifiers, Platform};
use letters_layout::{Callout, DocLayout, Fonts, Palette, Syntax};
use letters_model::{Block, BlockId, BlockKind, Change, Mark, Pos, Selection, Style};
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};

use crate::color::{fade, resolve};
use crate::describe::{self, Request};
use crate::element::{Cursor, Element, Paint, Pointer};
use crate::icon::{self, Icon};
use crate::paint::scrollbar;
use crate::style::{Align, Edges, Justify, Look, Ring};
use crate::text::{FontShaper, TextStyle};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::ui::{App, Ui};
use crate::widgets::{Button, Menu, MenuItem, Popover, SidePanel, TextField, TopBar, side_panel, top_bar};

/// Space kept beside the text column.
const GUTTER: f32 = 24.0;
/// Space kept above and below the text.
const TOP: f32 = 48.0;
/// Points to logical pixels.
const PT: f32 = 96.0 / 72.0;
const CARET_WIDTH: f32 = 1.5;
/// How much text one idle step lays out, in bytes. Small enough to fit between two frames.
const SETTLE_BYTES: usize = 4096;

/// The editor's window.
pub type Window = Ui<EditorApp>;

/// What the top bar says about saving. There is no save command: the title is the file's name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SaveState {
    #[default]
    Saved,
    Saving,
    /// Changes are kept on this device until the server is back.
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Outline,
    Styles,
    Notes,
    History,
}

impl Panel {
    const ALL: [(Panel, Icon); 4] = [
        (Panel::Outline, icon::LIST_TREE),
        (Panel::Styles, icon::PALETTE),
        (Panel::Notes, icon::MESSAGE_SQUARE),
        (Panel::History, icon::HISTORY),
    ];

    fn title(self) -> &'static str {
        match self {
            Panel::Outline => "Outline",
            Panel::Styles => "Styles",
            Panel::Notes => "Notes",
            Panel::History => "History",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    Text(InputEvent),
    Pointer(Pointer),
    Run(Command),
    /// Shows a panel, or hides it when it is the one showing.
    Panel(Panel),
    ClosePanel,
    Share(bool),
    /// Puts the caret at the start of a block.
    Jump(BlockId),
    /// Opens the link editor at the caret.
    EditLink,
    LinkDraft(String),
    LinkApply,
    LinkClose,
}

impl Ui<EditorApp> {
    pub fn new(editor: Editor, style: Style) -> Self {
        Ui::host(EditorApp::new(editor, style), Rc::new(FontShaper::default()))
    }

    pub fn editor(&self) -> &Editor {
        &self.app().editor
    }

    /// Moves the selection and brings it into view, as a click on the page does.
    pub fn select(&mut self, selection: Selection) {
        let app = self.app_mut();
        app.editor.select(selection);
        app.reveal();
    }

    /// Goes up each time the document changes.
    pub fn edits(&self) -> u64 {
        self.app().edits
    }

    pub fn set_document_name(&mut self, name: &str) {
        self.app_mut().name = name.into();
    }

    pub fn set_save_state(&mut self, state: SaveState) {
        self.app_mut().save = state;
    }
}

/// A document's layout, answering the editor's questions about lines.
pub struct DocLines<'a> {
    layout: &'a DocLayout,
    page: f32,
}

impl<'a> DocLines<'a> {
    /// `page` is the height of the part of the document in view, which is how far Page Up and Page Down go.
    pub fn new(layout: &'a DocLayout, page: f32) -> Self {
        Self { layout, page }
    }
}

impl Lines for DocLines<'_> {
    fn caret_x(&self, pos: &Pos) -> Option<f32> {
        self.layout.caret(pos).map(|caret| caret.x)
    }

    fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        self.layout.line_step(pos, x, down)
    }

    fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos> {
        let caret = self.layout.caret(pos)?;
        // Far enough to either side to be past any line.
        let x = if end { 1e6 } else { -1e6 };
        self.layout.hit(Point { x, y: caret.y + caret.height / 2.0 })
    }

    fn page_lines(&self) -> usize {
        // One line of the last page stays in view, to keep the reader's place.
        let line = self.layout.caret(&self.layout.hit(Point::default()).unwrap_or(Pos::new("", 0)));
        line.map_or(20, |line| ((self.page / line.height.max(1.0)) as usize).saturating_sub(1).max(1))
    }
}

/// Lends the editor the shell's platform together with the laid-out lines.
struct Lent<'a> {
    platform: &'a mut dyn Platform,
    lines: DocLines<'a>,
}

impl Platform for Lent<'_> {
    fn now_ms(&self) -> u64 {
        self.platform.now_ms()
    }

    fn clipboard_read(&mut self) -> Option<Clip> {
        self.platform.clipboard_read()
    }

    fn clipboard_write(&mut self, clip: Clip) {
        self.platform.clipboard_write(clip);
    }

    fn new_id(&mut self) -> String {
        self.platform.new_id()
    }

    fn lines(&self) -> Option<&dyn Lines> {
        Some(&self.lines)
    }
}

pub struct EditorApp {
    editor: Editor,
    style: Style,
    layout: DocLayout,
    palette: Palette,
    /// The text column's left edge inside the page area, and its width.
    column: (f32, f32),
    /// The height of the page area.
    height: f32,
    scroll: f32,
    /// Whether the layout still holds estimated heights, to be replaced in idle time.
    estimated: bool,
    name: String,
    save: SaveState,
    edits: u64,
    panel: Option<Panel>,
    sharing: bool,
    /// The address in the link editor while it is open.
    link: Option<String>,
    dragging: bool,
}

impl EditorApp {
    fn new(editor: Editor, style: Style) -> Self {
        Self {
            editor,
            style,
            layout: DocLayout::new(Fonts { faces: letters_fonts::bundled() }),
            palette: Palette::LIGHT,
            column: (0.0, 0.0),
            height: 0.0,
            scroll: 0.0,
            estimated: true,
            name: String::new(),
            save: SaveState::default(),
            edits: 0,
            panel: None,
            sharing: false,
            link: None,
            dragging: false,
        }
    }

    /// Runs something on the editor, which may ask the layout about lines while it works.
    fn edit(&mut self, platform: &mut dyn Platform, act: impl FnOnce(&mut Editor, &mut dyn Platform) -> Change) {
        let mut lent = Lent { platform, lines: DocLines::new(&self.layout, self.height) };
        let change = act(&mut self.editor, &mut lent);
        self.changed(&change);
    }

    /// Brings the layout and the viewport up to date after the editor did something.
    fn changed(&mut self, change: &Change) {
        if !change.blocks.is_empty() || change.structure {
            self.edits += 1;
            self.layout.update(self.editor.doc(), &self.style, self.column.1, Some(change));
            self.estimated = true;
        }
        self.reveal();
    }

    /// The part of the document in view, in the layout's coordinates.
    fn viewport(&self) -> Rect {
        Rect::new(0.0, self.scroll - TOP, self.column.1, self.height)
    }

    fn max_scroll(&self) -> f32 {
        (self.layout.height() + 2.0 * TOP - self.height).max(0.0)
    }

    fn scroll_by(&mut self, delta: f32) {
        self.scroll = (self.scroll + delta).clamp(0.0, self.max_scroll());
    }

    /// Scrolls just far enough to show the head of the selection.
    fn reveal(&mut self) {
        let Some(caret) = self.editor.selection().and_then(|selection| self.layout.caret(&selection.head)) else {
            return;
        };
        let (top, bottom) = (caret.y + TOP, caret.y + caret.height + TOP);
        let margin = GUTTER.min(self.height / 4.0);
        if top < self.scroll + margin {
            self.scroll = top - margin;
        } else if bottom > self.scroll + self.height - margin {
            self.scroll = bottom + margin - self.height;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// The text position under a point of the page area.
    fn hit(&self, at: Point) -> Option<Pos> {
        self.layout.hit(Point { x: at.x - self.column.0, y: at.y + self.scroll - TOP })
    }

    fn pointer(&mut self, event: Pointer, platform: &mut dyn Platform) {
        match event {
            Pointer::Down { at, mods, clicks } => {
                let Some(head) = self.hit(at) else { return };
                let anchor = match self.editor.selection() {
                    Some(selection) if mods.shift => selection.anchor.clone(),
                    _ => head.clone(),
                };
                self.editor.select(Selection { anchor, head });
                self.dragging = clicks == 1;
                match clicks {
                    1 => self.reveal(),
                    2 => self.edit(platform, |editor, platform| editor.run(Command::SelectWord, platform)),
                    _ => self.edit(platform, |editor, platform| editor.run(Command::SelectBlock, platform)),
                }
            }
            Pointer::Drag { at } if self.dragging => {
                let (Some(head), Some(selection)) = (self.hit(at), self.editor.selection()) else { return };
                self.editor.select(Selection { anchor: selection.anchor.clone(), head });
                // Dragging past an edge scrolls the page after the selection.
                self.reveal();
            }
            Pointer::Drag { .. } => {}
            Pointer::Up { .. } => self.dragging = false,
            Pointer::Scroll { delta } => self.scroll_by(delta.y),
        }
    }

    /// The block the caret is in.
    fn block(&self) -> Option<&Block> {
        self.editor.doc().block(&self.editor.selection()?.head.block)
    }

    /// Lays the document out for the room it has and paints the part in view.
    fn page(&mut self, room: Size) -> Element<Message> {
        let page = &self.style.page;
        let text = (page.width - page.margin_left - page.margin_right) * PT;
        let width = text.min(room.width - 2.0 * GUTTER).max(1.0);
        let left = ((room.width - width) / 2.0).max(0.0).round();
        if width != self.column.1 {
            self.layout.update(self.editor.doc(), &self.style, width, None);
        }
        (self.column, self.height) = ((left, width), room.height);
        // What is about to show is laid out for real. The rest waits for idle time.
        self.estimated = self.layout.settle(self.viewport(), 0);
        self.scroll = self.scroll.min(self.max_scroll());

        let viewport = self.viewport();
        let mut list = DisplayList::default();
        let selection = self.editor.selection();
        if let Some(selection) = selection.filter(|selection| !selection.is_caret()) {
            let color = self.palette.selection;
            list.items.extend(self.layout.selection_in(selection, viewport).into_iter().map(|rect| fill(rect, color)));
        }
        self.layout.paint(viewport, &mut list);
        if let Some((block, range)) = self.editor.composition() {
            // Text still being composed is underlined, as native fields do.
            let composing =
                Selection { anchor: Pos::new(block.clone(), range.start), head: Pos::new(block.clone(), range.end) };
            let under =
                |rect: Rect| fill(Rect::new(rect.x, rect.y + rect.height - 1.0, rect.width, 1.0), self.palette.text);
            list.items.extend(self.layout.selection_in(&composing, viewport).into_iter().map(under));
        }

        let origin = Point { x: left, y: TOP - self.scroll };
        let caret = selection.and_then(|selection| self.layout.caret(&selection.head));
        let caret =
            caret.map(|caret| Rect { x: origin.x + caret.x, y: origin.y + caret.y, width: CARET_WIDTH, ..caret });
        let paint = PagePaint {
            list,
            origin,
            page: self.palette.page,
            content: self.layout.height() + 2.0 * TOP,
            scroll: self.scroll,
        };
        let mut element = Element::canvas(Rc::new(paint))
            .grow(1.0)
            .fg(tokens::FG)
            .role(Role::Document)
            .label(self.title())
            .default_focus()
            .caret(caret)
            .on_text(Message::Text)
            .on_pointer(|event| Some(Message::Pointer(event)));
        if let Some(address) = &self.link {
            let field = TextField::new("Link", address)
                .placeholder("Paste or type a link")
                .on_input(Message::LinkDraft)
                .on_submit(Message::LinkApply);
            let editor =
                Popover::new(Element::from(field).width(300.0)).label("Link").open(true).on_close(Message::LinkClose);
            // The editor hangs from the caret, in place.
            element = element.popover(editor.at(caret.unwrap_or_default())).cursor(Cursor::Text);
        }
        element
    }

    fn bar(&self) -> Element<Message> {
        let name = if self.name.is_empty() { "Untitled" } else { &self.name };
        let (state_icon, state) = match self.save {
            SaveState::Saved => (icon::CLOUD_CHECK, "Saved"),
            SaveState::Saving => (icon::REFRESH_CW, "Saving"),
            SaveState::Offline => (icon::CLOUD_OFF, "Offline"),
        };
        let small = TextStyle::control().size(tokens::TEXT_SM);
        let mut state = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .fg(tokens::FG_MUTED)
            .child(Element::icon(state_icon, 14.0))
            .child(Element::text(state, small));
        if self.save == SaveState::Offline {
            state = state.description("Changes are kept on this device");
        }

        let tool = |icon, label: &str| Button::icon_only(icon, label).subtle().small();
        let run = |icon, label: &str, command: Command| tool(icon, label).on_press(Message::Run(command));
        let marks = [
            (icon::BOLD, "Bold", Mark::Bold),
            (icon::ITALIC, "Italic", Mark::Italic),
            (icon::UNDERLINE, "Underline", Mark::Underline),
            (icon::STRIKETHROUGH, "Strikethrough", Mark::Strike),
            (icon::CODE, "Code", Mark::Code),
        ];
        let in_force = self.editor.marks();

        let share = Element::column()
            .gap(tokens::SPACE_LG)
            .min_width(320.0)
            .child(Element::text("Share", TextStyle::title()))
            .child(Element::from(Button::new("Done").on_press(Message::Share(false))).align_self(Align::End));
        let share = Popover::new(share).label("Share").modal().open(self.sharing).on_close(Message::Share(false));

        let more = Menu::new()
            .label("More")
            .item(MenuItem::new("Export").icon(icon::DOWNLOAD).disabled(true))
            .item(MenuItem::new("Print view").icon(icon::PRINTER).disabled(true))
            .item(MenuItem::new("Page setup").icon(icon::SETTINGS).disabled(true));

        let mut bar = TopBar::new()
            .start(Element::icon(icon::FILE_TEXT, 18.0).fg(tokens::FG_MUTED))
            .start(Element::text(name, TextStyle::control().weight(600)))
            .start(state)
            .middle(TopBar::rule())
            .middle(run(icon::UNDO_2, "Undo", Command::Undo).disabled(!self.editor.can_undo()))
            .middle(run(icon::REDO_2, "Redo", Command::Redo).disabled(!self.editor.can_redo()))
            .middle(TopBar::rule());
        if let Some(block) = self.block() {
            // The type follows the caret: it is a label here, and changed on the page.
            bar = bar.middle(Element::text(type_name(&block.kind), small).fg(tokens::FG_MUTED)).middle(TopBar::rule());
        }
        for (icon, label, mark) in marks {
            let pressed = in_force.contains(&mark);
            bar = bar.middle(run(icon, label, Command::ToggleMark(mark)).pressed(pressed));
        }
        let link = tool(icon::LINK, "Link").pressed(self.editor.link().is_some()).on_press(Message::EditLink);
        bar = bar.middle(link);
        bar = bar.end(Button::new("Share").icon(icon::USERS).small().on_press(Message::Share(true)).popover(share));
        bar = bar.end(TopBar::rule());
        for (panel, icon) in Panel::ALL {
            let button = tool(icon, panel.title()).pressed(self.panel == Some(panel));
            bar = bar.end(button.on_press(Message::Panel(panel)));
        }
        bar.end(tool(icon::ELLIPSIS, "More").popover(more.popover().align(Align::End))).into()
    }

    fn side(&self, panel: Panel) -> Element<Message> {
        let rows: Vec<Element<Message>> = match panel {
            Panel::Outline => self.editor.doc().walk().filter_map(outline_row).collect(),
            Panel::Styles => self.style.types.iter().map(|(name, style)| style_row(name, style)).collect(),
            Panel::Notes | Panel::History => Vec::new(),
        };
        let empty = match panel {
            Panel::Outline => "Headings appear here as the document grows.",
            Panel::Styles => "This document has no styles.",
            Panel::Notes => "No open notes.",
            Panel::History => "No earlier versions yet.",
        };
        let body = Element::column().padding(Edges::top(tokens::SPACE_MD)).gap(tokens::SPACE_2XS);
        let body = if rows.is_empty() {
            body.child(Element::paragraph(empty, TextStyle::body()).fg(tokens::FG_MUTED))
        } else {
            body.children(rows)
        };
        SidePanel::new(panel.title()).on_close(Message::ClosePanel).child(body).into()
    }
}

impl App for EditorApp {
    type Msg = Message;

    fn title(&self) -> String {
        if self.name.is_empty() { "Letters".into() } else { self.name.clone() }
    }

    fn view(&mut self, window: Size) -> Element<Message> {
        let panel = self.panel.map(|panel| self.side(panel));
        let room = window.width - if panel.is_some() { side_panel::WIDTH } else { 0.0 };
        let page = self.page(Size { width: room.max(0.0), height: (window.height - top_bar::HEIGHT).max(0.0) });
        let body = Element::row().grow(1.0).child(page).children(panel);
        Element::column().bg(tokens::BG).fg(tokens::FG).child(self.bar()).child(body)
    }

    fn update(&mut self, message: Message, platform: &mut dyn Platform) {
        match message {
            Message::Text(event) => self.edit(platform, |editor, platform| editor.input(event, platform)),
            Message::Pointer(event) => self.pointer(event, platform),
            Message::Run(command) => self.edit(platform, |editor, platform| editor.run(command, platform)),
            Message::Panel(panel) => self.panel = (self.panel != Some(panel)).then_some(panel),
            Message::ClosePanel => self.panel = None,
            Message::Share(open) => self.sharing = open,
            Message::Jump(block) => {
                self.editor.select(Selection::caret(Pos { block, offset: 0 }));
                self.reveal();
            }
            Message::EditLink => self.link = Some(self.editor.link().unwrap_or_default().to_owned()),
            Message::LinkDraft(address) => self.link = self.link.is_some().then_some(address),
            Message::LinkClose => self.link = None,
            Message::LinkApply => {
                let Some(address) = self.link.take() else { return };
                let address = Some(address.trim().to_owned()).filter(|address| !address.is_empty());
                self.edit(platform, |editor, platform| editor.run(Command::SetLink(address), platform));
            }
        }
    }

    fn key(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let mut used = false;
        self.edit(platform, |editor, platform| {
            let change = editor.key(event.clone(), platform);
            used = change.is_some();
            change.unwrap_or_default()
        });
        // The editor leaves this one to the window, which edits the link in place.
        let link = event.key == Key::Char('k') && event.mods == Modifiers { primary: true, ..Modifiers::default() };
        if !used && link {
            self.link = Some(self.editor.link().unwrap_or_default().to_owned());
        }
        used || link
    }

    fn describe(&self, page: &mut AccessNode, nodes: &mut Vec<(NodeId, AccessNode)>) {
        describe::page(self.editor.doc(), self.editor.selection(), page, nodes);
    }

    fn access_action(&mut self, request: &ActionRequest, platform: &mut dyn Platform) {
        match describe::request(self.editor.doc(), request) {
            Some(Request::Select(selection)) => {
                self.editor.select(selection);
                self.reveal();
            }
            Some(Request::Replace(text)) => {
                self.edit(platform, |editor, platform| editor.input(InputEvent::Insert(text), platform));
            }
            None => {}
        }
    }

    fn scheme(&mut self, scheme: Scheme) {
        self.palette = palette(scheme);
        self.layout.set_palette(self.palette);
    }

    fn idle(&mut self) -> bool {
        if !self.estimated {
            return false;
        }
        // Blocks above the part in view change height as they are laid out, so its top line is held still.
        let viewport = self.viewport();
        let pin = self.layout.hit(Point { x: 0.0, y: viewport.y.max(0.0) });
        let top = |layout: &DocLayout| pin.as_ref().and_then(|pos| layout.caret(pos)).map(|caret| caret.y);
        let before = top(&self.layout);
        self.estimated = self.layout.settle(viewport, SETTLE_BYTES);
        if let (Some(before), Some(after)) = (before, top(&self.layout)) {
            self.scroll_by(after - before);
        }
        true
    }
}

/// What the layout painted for the part of the page in view, placed where the page area is.
struct PagePaint {
    list: DisplayList,
    /// Where the layout's origin is inside the page area.
    origin: Point,
    page: Color,
    /// The height of everything that scrolls, and how far it is scrolled.
    content: f32,
    scroll: f32,
}

impl Paint for PagePaint {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        out.push(fill(rect, self.page));
        let first = out.items.len();
        out.items.extend(self.list.items.iter().cloned());
        out.translate(first, Point { x: rect.x + self.origin.x, y: rect.y + self.origin.y });
        out.items.extend(scrollbar(rect, self.content, self.scroll, scheme, 1.0));
    }
}

fn fill(rect: Rect, color: Color) -> Item {
    Item::Fill { rect, color, radius: 0.0 }
}

/// The colours the page is painted in. The light page is the one documents are styled for and exports use.
fn palette(scheme: Scheme) -> Palette {
    if scheme == Scheme::Light {
        return Palette::LIGHT;
    }
    let ink = |ink: Ink| resolve(ink, scheme, 1.0);
    let callout = |accent: Ink, soft: Ink| Callout { accent: ink(accent), fill: ink(soft) };
    Palette {
        scheme: letters_layout::Scheme::Dark,
        page: ink(tokens::SURFACE),
        text: ink(tokens::FG),
        selection: ink(fade(tokens::ACCENT, 0.3)),
        caret: ink(tokens::FG),
        marker: ink(tokens::FG_SUBTLE),
        link: ink(tokens::ACCENT_TEXT),
        accent: ink(tokens::ACCENT_SOLID),
        on_accent: ink(tokens::ON_SOLID),
        line: ink(tokens::LINE),
        line_strong: ink(tokens::LINE_STRONG),
        code: ink(fade(tokens::FG, 0.04)),
        inline_code: ink(fade(tokens::FG, 0.06)),
        highlight: Palette::LIGHT.highlight,
        tip: callout(tokens::OK, tokens::OK_SOFT),
        info: callout(tokens::INFO, tokens::INFO_SOFT),
        warn: callout(tokens::WARN, tokens::WARN_SOFT),
        danger: callout(tokens::DANGER, tokens::DANGER_SOFT),
        syntax: Syntax {
            comment: ink(tokens::SYNTAX_COMMENT),
            keyword: ink(tokens::SYNTAX_KEYWORD),
            string: ink(tokens::SYNTAX_STRING),
            number: ink(tokens::SYNTAX_NUMBER),
            type_name: ink(tokens::SYNTAX_TYPE),
            function: ink(tokens::SYNTAX_FUNCTION),
            property: ink(tokens::SYNTAX_PROPERTY),
        },
    }
}

/// The element type the top bar names for a block.
fn type_name(kind: &BlockKind) -> String {
    match kind {
        BlockKind::Paragraph => "Body".into(),
        BlockKind::Heading { level } => format!("Heading {level}"),
        BlockKind::ListItem { .. } => "List".into(),
        BlockKind::Code { .. } => "Code".into(),
        BlockKind::Quote => "Quote".into(),
        BlockKind::Callout { .. } => "Callout".into(),
        BlockKind::Divider => "Divider".into(),
        BlockKind::Unknown { .. } => "Block".into(),
    }
}

/// A heading in the outline. Pressing it puts the caret there.
fn outline_row(block: &Block) -> Option<Element<Message>> {
    let BlockKind::Heading { level } = block.kind else { return None };
    let text = block.plain_text();
    let indent = tokens::SPACE_SM + tokens::SPACE_MD * f32::from(level.saturating_sub(1));
    let row = Element::row()
        .key(block.id.as_str())
        .align(Align::Center)
        .min_height(tokens::CONTROL_HEIGHT_SM)
        .padding(Edges { left: indent, ..Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS) })
        .radius(tokens::RADIUS_SM)
        .hover(Look::bg(tokens::NEUTRAL_SOFT))
        .ring(Ring::Outline(0.0))
        .role(Role::Link)
        .child(Element::text(if text.is_empty() { "Untitled".into() } else { text }, TextStyle::control()))
        .on_press(Message::Jump(block.id.clone()));
    Some(row)
}

/// One of the document's element types, with the face it is set in.
fn style_row(name: &str, style: &letters_model::TextStyle) -> Element<Message> {
    let face = format!("{} {} pt", style.family, style.size);
    Element::row()
        .align(Align::Center)
        .justify(Justify::Between)
        .min_height(tokens::CONTROL_HEIGHT_SM)
        .child(Element::text(name, TextStyle::control()))
        .child(Element::text(face, TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED))
}
