//! The editor's window: one top bar, the page, and one side panel at a time.
//!
//! [`Window`] is what a shell shows and the test harness drives. The page is a canvas holding what the
//! document's layout painted for the part in view. Everything around it is made of the toolkit's widgets.

use std::collections::{BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use accesskit::{ActionRequest, Node as AccessNode, NodeId, Role};
use letters_editor::{Clip, Command, Editor, InputEvent, Key, KeyEvent, Keymap, Lines, Modifiers, Platform};
use letters_layout::{Callout, DocLayout, Fonts, Palette, Syntax};
use letters_model::{Align as TextAlign, Block, BlockId, BlockKind, Change, Mark, Pos, Selection, Style};
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};

use crate::color::{fade, resolve, same};
use crate::describe::{self, Request};
use crate::element::{Cursor, Element, Fitted, Paint, Pointer};
use crate::highlight;
use crate::icon::{self, Icon};
use crate::paint::scrollbar;
use crate::restyle::{self, Field, StyleEdit, Styles};
use crate::style::{Align, Dir, Edges, Justify, Look, Ring};
use crate::text::{Family, FontShaper, TextStyle};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::ui::{App, Keys, Ui};
use crate::widgets::{
    Button, ControlSize, Menu, MenuItem, Popover, Side, SidePanel, TextField, TopBar, side_panel, top_bar,
};

/// Space kept beside the text column.
const GUTTER: f32 = 24.0;
/// Space kept above and below the text.
const TOP: f32 = 48.0;
/// Points to logical pixels.
const PT: f32 = 96.0 / 72.0;
const CARET_WIDTH: f32 = 1.5;
/// How much text one idle step lays out, in bytes. Small enough to fit between two frames.
const SETTLE_BYTES: usize = 4096;

/// The chapters control: where it sits in the page's corner, and the size of its shortest line.
const CHAPTERS_INSET: Edges = Edges::xy(6.0, 16.0);
const CHAPTER_LINE: Size = Size { width: 12.0, height: 2.0 };
/// How far a line steps in for each level below the document's shallowest heading, and the most steps it
/// takes: headings run from level 1 to 4. Every line ends at the same edge, so a deeper one is shorter.
const CHAPTER_INDENT: f32 = 3.0;
const CHAPTER_STEPS: u8 = 3;
/// The room a line takes down the control, and the least it closes up to in a document of many headings.
const CHAPTER_PITCH: (f32, f32) = (8.0, 4.0);

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
    Styles,
    Notes,
    History,
}

impl Panel {
    const ALL: [(Panel, Icon); 3] =
        [(Panel::Styles, icon::PALETTE), (Panel::Notes, icon::MESSAGE_SQUARE), (Panel::History, icon::HISTORY)];

    fn title(self) -> &'static str {
        match self {
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
    /// Puts the caret at the start of a block and brings the block to the top of the page.
    Jump(BlockId),
    /// Opens the link editor at the caret.
    EditLink,
    LinkDraft(String),
    LinkApply,
    LinkClose,
    /// Changes the style of the element type under the caret.
    Restyle(StyleEdit),
    /// What is typed into one of the style's fields, before it is taken.
    Draft(Field, String),
    /// Takes what was typed into a field, or drops it if it makes no sense there.
    Apply(Field),
    /// Drops what was typed into a field.
    Forget,
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

    /// The document's style, which a shell writes when it saves.
    pub fn style(&self) -> &Style {
        &self.app().style
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
    /// How far down the page was scrolled, which may be part of a pixel. See `scrolled`.
    scroll: f32,
    /// The device pixels in a logical one.
    scale: f32,
    /// Whether the layout still holds estimated heights, to be replaced in idle time.
    estimated: bool,
    name: String,
    save: SaveState,
    edits: u64,
    panel: Option<Panel>,
    sharing: bool,
    /// The address in the link editor while it is open.
    link: Option<String>,
    /// What is typed into one of the style's fields, and the element type it was typed for.
    draft: Option<(Field, &'static str, String)>,
    folded: Folded,
    /// What each code block was last coloured for: its text and language, hashed.
    lexed: HashMap<BlockId, u64>,
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
            scale: 1.0,
            estimated: true,
            name: String::new(),
            save: SaveState::default(),
            edits: 0,
            panel: None,
            sharing: false,
            link: None,
            draft: None,
            folded: Folded::default(),
            lexed: HashMap::new(),
            dragging: false,
        }
    }

    /// The element type whose style the caret's block is drawn with.
    fn element(&self) -> Option<&'static str> {
        restyle::element(self.editor.doc(), &self.editor.selection()?.head.block)
    }

    /// Keeps up which form of the bar fits a window of `width`: forgotten when something that decides it
    /// has changed, and learnt from the layout after that.
    fn refold(&mut self, width: f32) {
        let style = self.element().and_then(|element| Some((element, &self.style.types.get(element)?.family)));
        let mut hasher = DefaultHasher::new();
        (width.to_bits(), &self.name, std::mem::discriminant(&self.save), style).hash(&mut hasher);
        let key = hasher.finish();
        if key != self.folded.key {
            self.folded = Folded { key, form: None, seen: Fitted::default() };
        } else if self.folded.form.is_none() {
            self.folded.form = self.folded.seen.get();
        }
    }

    /// Restyles every block of the type under the caret. Line breaks everywhere may change, so the whole
    /// document is laid out again.
    fn restyle(&mut self, edit: &StyleEdit) {
        let Some(element) = self.element() else { return };
        let Some(mut style) = self.style.style().types.get(element).cloned() else { return };
        edit.apply(&mut style);
        self.style.restyle(element, style);
        self.edits += 1;
        self.layout.update(self.editor.doc(), &self.style, self.column.1, None);
        self.estimated = true;
        self.reveal();
    }

    /// Colours the code blocks among `changed`, or every one in the document. A block is lexed again only
    /// when its text or language is not what it was last lexed for, so a keystroke lexes one block at most.
    fn highlight(&mut self, changed: Option<&BTreeSet<BlockId>>) {
        let doc = self.editor.doc();
        let blocks: Vec<&Block> = match changed {
            Some(changed) => changed.iter().filter_map(|id| doc.block(id)).collect(),
            None => doc.walk().collect(),
        };
        let mut lexed = HashMap::new();
        for block in blocks {
            let BlockKind::Code { language } = &block.kind else { continue };
            let text = block.plain_text();
            let mut hasher = DefaultHasher::new();
            (&text, language).hash(&mut hasher);
            let key = hasher.finish();
            if self.lexed.get(&block.id) != Some(&key) {
                let language = language.as_deref().unwrap_or_default();
                self.layout.set_tokens(&block.id, highlight::tokens(&text, language));
            }
            lexed.insert(block.id.clone(), key);
        }
        // A walk of the whole document also forgets the blocks that are gone.
        match changed {
            Some(_) => self.lexed.extend(lexed),
            None => self.lexed = lexed,
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
            self.highlight((!change.structure).then_some(&change.blocks));
        }
        self.reveal();
    }

    /// The part of the document in view, in the layout's coordinates.
    fn viewport(&self) -> Rect {
        Rect::new(0.0, self.scrolled() - TOP, self.column.1, self.height)
    }

    fn max_scroll(&self) -> f32 {
        (self.layout.height() + 2.0 * TOP - self.height).max(0.0)
    }

    /// How far down the page shows: the nearest whole device pixel to where it was scrolled to. A frame that
    /// differs from the last by whole pixels can move them, and hinted text stays sharp. Parts of a pixel
    /// still add up in `scroll`.
    fn scrolled(&self) -> f32 {
        (self.scroll * self.scale).round() / self.scale
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
        self.layout.hit(Point { x: at.x - self.column.0, y: at.y + self.scrolled() - TOP })
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

    /// Lays the document out for the room it has and paints the part in view.
    fn page(&mut self, room: Size) -> Element<Message> {
        let page = &self.style.page;
        let text = (page.width - page.margin_left - page.margin_right) * PT;
        let width = text.min(room.width - 2.0 * GUTTER).max(1.0);
        let left = ((room.width - width) / 2.0).max(0.0).round();
        if width != self.column.1 {
            self.layout.update(self.editor.doc(), &self.style, width, None);
            self.highlight(None);
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

        let origin = Point { x: left, y: TOP - self.scrolled() };
        let caret = selection.and_then(|selection| self.layout.caret(&selection.head));
        let caret =
            caret.map(|caret| Rect { x: origin.x + caret.x, y: origin.y + caret.y, width: CARET_WIDTH, ..caret });
        let paint = PagePaint {
            list,
            origin,
            page: self.palette.page,
            content: self.layout.height() + 2.0 * TOP,
            scroll: self.scrolled(),
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

    /// The heading the top of the view is under.
    fn section(&self) -> Option<&BlockId> {
        // A heading brought to the top sits just under the page's top margin.
        let top = self.layout.hit(Point { x: 0.0, y: self.scrolled() + CHAPTER_PITCH.0 })?;
        let mut section = None;
        for block in self.editor.doc().walk() {
            if matches!(block.kind, BlockKind::Heading { .. }) {
                section = Some(&block.id);
            }
            if block.id == top.block {
                break;
            }
        }
        section
    }

    /// The document's outline, at the top left of the page: one short line per heading, stepped in by its
    /// level. A line names its heading in a tooltip, and pressing it brings the heading to the top.
    fn chapters(&self) -> Option<Element<Message>> {
        let level = |block: &Block| match block.kind {
            BlockKind::Heading { level } => Some(level),
            _ => None,
        };
        let headings: Vec<(&Block, u8)> =
            self.editor.doc().walk().filter_map(|block| Some((block, level(block)?))).collect();
        let shallowest = headings.iter().map(|heading| heading.1).min()?;
        // One heading is no outline.
        if headings.len() < 2 {
            return None;
        }
        // The lines close up when the page has no room for them all.
        let room = (self.height - CHAPTERS_INSET.vertical()).max(0.0);
        let pitch = (room / headings.len() as f32).floor().clamp(CHAPTER_PITCH.1, CHAPTER_PITCH.0);
        let width = CHAPTER_LINE.width + f32::from(CHAPTER_STEPS) * CHAPTER_INDENT;
        let section = self.section();

        let line = |(block, level): (&Block, u8)| {
            let title = Some(block.plain_text()).filter(|title| !title.trim().is_empty());
            let title = title.unwrap_or_else(|| "Untitled".into());
            let here = section == Some(&block.id);
            let mark = Element::row().grow(1.0).height(CHAPTER_LINE.height).radius(1.0).inked();
            Element::row()
                .key(block.id.as_str())
                .align(Align::Center)
                .width(width)
                .height(pitch)
                .padding(Edges::left(CHAPTER_INDENT * f32::from((level - shallowest).min(CHAPTER_STEPS))))
                .fg(if here { tokens::ACCENT } else { fade(tokens::FG_MUTED, 0.5) })
                .hover(Look::NONE.with_fg(tokens::FG))
                .focused(Look::NONE.with_fg(tokens::FG))
                .ring(Ring::Outline(0.0))
                .role(Role::Link)
                .label(title.clone())
                .tooltip(title, Side::Right)
                .on_press(Message::Jump(block.id.clone()))
                .child(mark)
        };
        let chapters = Element::column()
            .overlay()
            .padding(CHAPTERS_INSET)
            .max_height(self.height)
            .clip()
            .role(Role::Navigation)
            .label("Outline")
            .arrows(Dir::Column)
            .one_stop()
            .children(headings.into_iter().map(line));
        Some(chapters)
    }

    fn bar(&self) -> Element<Message> {
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

        // The forms of the bar, from the widest. Once the layout has said which fits, only that one is built.
        let forms = [
            (NAME_WIDTH.2, Fold::None, false),
            (NAME_WIDTH.1, Fold::None, false),
            (NAME_WIDTH.1, Fold::Alignment, false),
            (NAME_WIDTH.1, Fold::Marks, false),
            (NAME_WIDTH.1, Fold::Style, false),
            (NAME_WIDTH.1, Fold::Style, true),
        ];
        let mut bar = TopBar::new().shown(self.folded.seen.clone());
        for (n, (name, fold, brief)) in forms.into_iter().enumerate() {
            if self.folded.form.is_none_or(|form| form == n) {
                bar = self.lead(name, brief).into_iter().fold(bar.form(n), TopBar::start);
                bar = self.tools(fold).into_iter().fold(bar, TopBar::middle);
            }
        }
        bar = bar.end(Button::new("Share").icon(icon::USERS).small().on_press(Message::Share(true)).popover(share));
        bar = bar.end(TopBar::rule());
        let mut panels = cluster();
        for (panel, icon) in Panel::ALL {
            let button = tool(icon, panel.title()).pressed(self.panel == Some(panel));
            panels = panels.child(button.on_press(Message::Panel(panel)));
        }
        bar.end(panels.child(tool(icon::ELLIPSIS, "More").popover(more.popover().align(Align::End)))).into()
    }

    /// What the bar starts with: the document's name, in no more than `room`, and whether it is saved, which
    /// `brief` says with the icon alone.
    fn lead(&self, room: f32, brief: bool) -> Vec<Element<Message>> {
        let name = if self.name.is_empty() { "Untitled" } else { &self.name };
        let (state_icon, state) = match self.save {
            SaveState::Saved => (icon::CLOUD_CHECK, "Saved"),
            SaveState::Saving => (icon::REFRESH_CW, "Saving"),
            SaveState::Offline => (icon::CLOUD_OFF, "Offline"),
        };
        let mut saved = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .fg(tokens::FG_MUTED)
            .child(Element::icon(state_icon, 14.0));
        saved = match brief {
            true => saved.role(Role::Label).label(state).tooltip(state, Side::Bottom),
            false => saved.child(Element::text(state, TextStyle::control().size(tokens::TEXT_SM))),
        };
        if self.save == SaveState::Offline {
            saved = saved.description("Changes are kept on this device");
        }
        // The name gives way to the controls after it, down to a few letters.
        let name = Element::text(name, TextStyle::control().weight(600)).min_width(NAME_WIDTH.0).max_width(room);
        vec![Element::icon(icon::FILE_TEXT, 18.0).fg(tokens::FG_MUTED), name.shrink(), saved]
    }

    /// The bar's format controls: undo and redo, the style of the type under the caret, then the marks and
    /// the link. `fold` says how much of it goes into menus, for a window too narrow to show it all.
    fn tools(&self, fold: Fold) -> Vec<Element<Message>> {
        let run = |icon, label: &str, command: Command| tool(icon, label).on_press(Message::Run(command));
        let history = cluster()
            .child(run(icon::UNDO_2, "Undo", Command::Undo).disabled(!self.editor.can_undo()))
            .child(run(icon::REDO_2, "Redo", Command::Redo).disabled(!self.editor.can_redo()));
        let mut tools = vec![TopBar::rule(), history, TopBar::rule()];
        tools.extend(self.style_group(fold));

        let in_force = self.editor.marks();
        let toggle = |mark| Message::Run(Command::ToggleMark(mark));
        let mut marks = cluster();
        if fold >= Fold::Marks {
            let mut menu = Menu::new().label("Marks");
            for (icon, label, mark) in MARKS {
                let item = MenuItem::new(label).icon(icon).checked(in_force.contains(&mark));
                menu = menu.item(item.on_select(toggle(mark)));
            }
            let any = MARKS.iter().any(|(_, _, mark)| in_force.contains(mark));
            marks = marks.child(tool(icon::TYPE, "Marks").pressed(any).popover(menu.popover()));
        } else {
            for (icon, label, mark) in MARKS {
                marks = marks.child(tool(icon, label).pressed(in_force.contains(&mark)).on_press(toggle(mark)));
            }
        }
        let link = tool(icon::LINK, "Link").pressed(self.editor.link().is_some()).on_press(Message::EditLink);
        tools.push(marks.child(link));
        tools
    }

    /// The style of the element type under the caret: its name, then its font, size and alignment. Each
    /// control restyles every block of that type. Folded, the alignments are one button with a menu, and
    /// folded further the name is a button that opens the rest.
    fn style_group(&self, fold: Fold) -> Vec<Element<Message>> {
        let Some(element) = self.element() else { return Vec::new() };
        let Some(style) = self.style.style().types.get(element) else { return Vec::new() };

        // The type follows the caret: it is a label here, and changed on the page.
        let name = restyle::name(element);
        let label = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .height(tokens::CONTROL_HEIGHT_SM)
            .min_width(TYPE_WIDTH)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .radius(tokens::RADIUS_CONTROL)
            .bg(tokens::TONE_1)
            .fg(tokens::FG_MUTED)
            .child(Element::text(name.clone(), TextStyle::control().size(tokens::TEXT_SM).weight(500)));

        let mut fonts = Menu::new().label("Font");
        for (family, face) in FAMILIES {
            let pick = Message::Restyle(StyleEdit::Family(family.into()));
            fonts = fonts.item(MenuItem::new(family).face(face).checked(style.family == family).on_select(pick));
        }
        fonts = fonts.heading("Weight");
        for (weight, name) in WEIGHTS {
            let pick = Message::Restyle(StyleEdit::Weight(weight));
            fonts = fonts.item(MenuItem::new(name).checked(style.weight == weight).on_select(pick));
        }
        let face = FAMILIES.iter().find(|(family, _)| *family == style.family).map_or(Family::Sans, |found| found.1);
        let font = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .height(tokens::CONTROL_HEIGHT_SM)
            .min_width(128.0)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .border(Edges::all(1.0), tokens::LINE_STRONG)
            .radius(tokens::RADIUS_CONTROL)
            .bg(tokens::SURFACE)
            .hover(Look::bg(tokens::TONE_1))
            .ring(Ring::Outline(2.0))
            .role(Role::Button)
            .label("Font")
            .description(style.family.clone())
            .child(
                Element::text(style.family.clone(), TextStyle::control().size(tokens::TEXT_SM).family(face)).grow(1.0),
            )
            .child(Element::icon(icon::CHEVRON_DOWN, 14.0).fg(tokens::FG_MUTED))
            .popover(fonts.popover());

        let size = self.field(Field::Size, element, style).width(56.0);

        // The colour shows as the page draws it, which a dark page does not do as written.
        let shown = |hex: &str| {
            let color = self.palette.recolor(hex_color(hex).unwrap_or(self.palette.text));
            same(color.r, color.g, color.b)
        };
        let mut swatches = Element::row().gap(tokens::SPACE_XS);
        for (name, ink) in INKS {
            let hex = hex(ink);
            let chosen = style.color.eq_ignore_ascii_case(&hex);
            let dot = Element::row().width(18.0).height(18.0).radius(9.0).bg(shown(&hex));
            let swatch = Element::row()
                .padding(Edges::all(2.0))
                .border(Edges::all(2.0), if chosen { tokens::ACCENT } else { fade(tokens::FG, 0.0) })
                .radius(13.0)
                .hover(Look::bg(tokens::NEUTRAL_SOFT))
                .ring(Ring::Outline(2.0))
                .role(Role::Button)
                .label(name)
                .pressed(chosen)
                .tooltip(name, Side::Top)
                .closes()
                .on_press(Message::Restyle(StyleEdit::Color(hex)))
                .child(dot);
            swatches = swatches.child(swatch);
        }
        let any = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .child(Element::text("Hex", TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED).grow(1.0))
            .child(self.field(Field::Color, element, style).width(96.0));
        let named = INKS.iter().find(|(_, ink)| style.color.eq_ignore_ascii_case(&hex(*ink)));
        let colour = Element::row()
            .align(Align::Center)
            .justify(Justify::Center)
            .width(tokens::CONTROL_HEIGHT_SM)
            .height(tokens::CONTROL_HEIGHT_SM)
            .radius(tokens::RADIUS_CONTROL)
            .fg(shown(&style.color))
            .hover(Look::bg(tokens::NEUTRAL_SOFT))
            .ring(Ring::Outline(2.0))
            .role(Role::Button)
            .label("Colour")
            .description(named.map_or(style.color.as_str(), |(name, _)| name))
            .child(Element::icon(icon::BASELINE, 16.0))
            .popover(Popover::new(Element::column().gap(tokens::SPACE_MD).child(swatches).child(any)).label("Colour"));

        let spaces = [(Field::LineHeight, ""), (Field::SpaceBefore, "pt"), (Field::SpaceAfter, "pt")];
        let spaces = spaces.map(|(field, unit)| {
            let label = Element::text(field.label(), TextStyle::control().size(tokens::TEXT_SM)).grow(1.0);
            let unit = Element::text(unit, TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED).width(16.0);
            Element::row()
                .align(Align::Center)
                .gap(tokens::SPACE_SM)
                .child(label)
                .child(self.field(field, element, style).width(56.0))
                .child(unit)
        });
        let spacing = Element::column().gap(tokens::SPACE_SM).min_width(200.0).children(spaces);
        let spacing = tool(icon::LINE_HEIGHT, "Spacing").popover(Popover::new(spacing).label("Spacing"));

        let align = |align| Message::Restyle(StyleEdit::Align(align));
        let mut aligns = cluster();
        if fold == Fold::Alignment || fold == Fold::Marks {
            let mut menu = Menu::new().label("Alignment");
            for (icon, label, to) in ALIGNS {
                menu = menu.item(MenuItem::new(label).icon(icon).checked(style.align == to).on_select(align(to)));
            }
            let shown = ALIGNS.iter().find(|(_, _, to)| *to == style.align).map_or(icon::TEXT_ALIGN_START, |at| at.0);
            aligns = aligns.child(tool(shown, "Alignment").popover(menu.popover()));
        } else {
            for (icon, label, to) in ALIGNS {
                aligns = aligns.child(tool(icon, label).pressed(style.align == to).on_press(align(to)));
            }
        }

        let looks = cluster().child(colour).child(aligns).child(spacing);
        if fold < Fold::Style {
            return vec![label, font, size, looks, TopBar::rule()];
        }
        let title = format!("{name} style");
        let rest = Element::column()
            .gap(tokens::SPACE_SM)
            .child(font)
            .child(Element::row().align(Align::Center).gap(tokens::SPACE_SM).child(size).child(looks));
        let open = label
            .child(Element::icon(icon::CHEVRON_DOWN, 14.0))
            .hover(Look::bg(tokens::NEUTRAL_SOFT))
            .ring(Ring::Outline(2.0))
            .role(Role::Button)
            .label(title.clone())
            .popover(Popover::new(rest).label(title));
        vec![open, TopBar::rule()]
    }

    /// A field for one part of the type's style. It shows what is being typed into it, and the style's own
    /// value otherwise. What was typed is taken on Enter and when focus leaves.
    fn field(&self, field: Field, element: &'static str, style: &letters_model::TextStyle) -> Element<Message> {
        let typed = self.draft.as_ref().filter(|draft| draft.0 == field && draft.1 == element).map(|draft| &draft.2);
        let mut input = TextField::new(field.label(), typed.cloned().unwrap_or_else(|| field.show(style)))
            .size(ControlSize::Small)
            .invalid(typed.is_some_and(|text| field.read(text).is_none()))
            .on_input(move |text| Message::Draft(field, text))
            .on_submit(Message::Apply(field))
            .on_leave(Message::Apply(field))
            .on_cancel(Message::Forget);
        if field == Field::Color {
            input = input.mono();
        }
        input.into()
    }

    fn side(&self, panel: Panel) -> Element<Message> {
        let rows: Vec<Element<Message>> = match panel {
            Panel::Styles => self.style.types.iter().map(|(name, style)| style_row(name, style)).collect(),
            Panel::Notes | Panel::History => Vec::new(),
        };
        let empty = match panel {
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
        self.refold(window.width);
        let panel = self.panel.map(|panel| self.side(panel));
        let room = window.width - if panel.is_some() { side_panel::WIDTH } else { 0.0 };
        let page = self.page(Size { width: room.max(0.0), height: (window.height - top_bar::HEIGHT).max(0.0) });
        let page = Element::row().grow(1.0).child(page).children(self.chapters());
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
                let start = Pos { block, offset: 0 };
                self.editor.select(Selection::caret(start.clone()));
                // Twice, since laying out what comes into view can move the block.
                for _ in 0..2 {
                    let Some(line) = self.layout.caret(&start) else { break };
                    self.scroll = line.y.clamp(0.0, self.max_scroll());
                    self.layout.settle(self.viewport(), 0);
                }
            }
            Message::EditLink => self.link = Some(self.editor.link().unwrap_or_default().to_owned()),
            Message::LinkDraft(address) => self.link = self.link.is_some().then_some(address),
            Message::LinkClose => self.link = None,
            Message::Restyle(edit) => {
                // A choice made outright replaces whatever was half typed.
                self.draft = None;
                self.restyle(&edit);
            }
            Message::Forget => self.draft = None,
            Message::Draft(field, text) => self.draft = self.element().map(|element| (field, element, text)),
            Message::Apply(field) => {
                let typed =
                    self.draft.take_if(|draft| draft.0 == field).filter(|draft| Some(draft.1) == self.element());
                if let Some(edit) = typed.and_then(|draft| field.read(&draft.2)) {
                    self.restyle(&edit);
                }
            }
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

    fn keys(&mut self, keys: Keys) {
        self.editor.set_keymap(match keys {
            Keys::Standard => Keymap::standard(),
            Keys::Mac => Keymap::mac(),
        });
    }

    fn scale(&mut self, scale: f32) {
        self.scale = scale;
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
        spelling: ink(tokens::DANGER),
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

/// Which form of the bar fits the window.
#[derive(Default)]
struct Folded {
    /// Everything that decides the fit, hashed.
    key: u64,
    /// The form that fits, once the layout has said.
    form: Option<usize>,
    seen: Fitted,
}

/// The room the bar gives the document's name: the least it keeps, what it keeps before any control folds,
/// and the most a name can take.
const NAME_WIDTH: (f32, f32, f32) = (48.0, 160.0, 320.0);

/// Room for the longest type name, so the controls after it stay put as the caret goes from type to type.
const TYPE_WIDTH: f32 = 88.0;

/// How much of the bar's format controls is folded away. A narrowing window loses the alignments to a menu
/// first, then the marks, and at last the whole style of the type to one button.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Fold {
    None,
    Alignment,
    Marks,
    Style,
}

/// A small button of the bar that shows only an icon.
fn tool(icon: Icon, label: &str) -> Button<Message> {
    Button::icon_only(icon, label).subtle().small()
}

/// Buttons that belong together, set closer than the bar's other controls.
fn cluster() -> Element<Message> {
    Element::row().align(Align::Center).gap(tokens::SPACE_2XS)
}

/// The colours offered for text, as they are on a light page: yewi's ink and its tones that read on white.
/// Any other colour can be typed.
const INKS: [(&str, Ink); 8] = [
    ("Ink", same(0x1f, 0x23, 0x28)),
    ("Grey", tokens::FG_MUTED),
    ("Red", tokens::DANGER),
    ("Amber", tokens::WARN),
    ("Green", tokens::OK),
    ("Teal", tokens::SYNTAX_TYPE),
    ("Blue", tokens::INFO),
    ("Purple", tokens::SYNTAX_KEYWORD),
];

/// A colour as a style stores it.
fn hex(ink: Ink) -> String {
    format!("#{:02x}{:02x}{:02x}", ink.light.r, ink.light.g, ink.light.b)
}

/// Reads a style's `#rrggbb`.
fn hex_color(hex: &str) -> Option<Color> {
    let rgb = u32::from_str_radix(hex.strip_prefix('#').filter(|digits| digits.len() == 6)?, 16).ok()?;
    Some(Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
}

const MARKS: [(Icon, &str, Mark); 5] = [
    (icon::BOLD, "Bold", Mark::Bold),
    (icon::ITALIC, "Italic", Mark::Italic),
    (icon::UNDERLINE, "Underline", Mark::Underline),
    (icon::STRIKETHROUGH, "Strikethrough", Mark::Strike),
    (icon::CODE, "Code", Mark::Code),
];

const ALIGNS: [(Icon, &str, TextAlign); 4] = [
    (icon::TEXT_ALIGN_START, "Align left", TextAlign::Start),
    (icon::TEXT_ALIGN_CENTER, "Align centre", TextAlign::Center),
    (icon::TEXT_ALIGN_END, "Align right", TextAlign::End),
    (icon::TEXT_ALIGN_JUSTIFY, "Justify", TextAlign::Justify),
];

/// The bundled families, each with the face the bar shows its name in.
const FAMILIES: [(&str, Family); 3] = [
    (letters_fonts::SANS, Family::Sans),
    (letters_fonts::DISPLAY, Family::Display),
    (letters_fonts::MONO, Family::Mono),
];

/// The weights offered for a type. The bundled faces are variable, so every family has them all.
const WEIGHTS: [(u16, &str); 5] = [(300, "Light"), (400, "Regular"), (500, "Medium"), (600, "Semibold"), (700, "Bold")];

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
