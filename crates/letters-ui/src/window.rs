//! The editor's window: one top bar, the page, and one side panel at a time.
//!
//! [`Window`] is what a shell shows and the test harness drives. The page is a canvas holding what the
//! document's layout painted for the part in view. Everything around it is made of the toolkit's widgets.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use accesskit::{ActionRequest, Node as AccessNode, NodeId, Role};
use letters_editor::{Clip, Command, Editor, InputEvent, Key, KeyEvent, Keymap, Lines, Modifiers, Motion, Platform};
use letters_layout::{Callout, DocLayout, Fonts, Palette, Syntax};
use letters_model::{Align as TextAlign, Block, BlockId, BlockKind, Body, Change, Mark, Marker, Pos, Selection, Style};
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};
use letters_spell::Dictionary;

use crate::chrome::Chrome;
use crate::color::{fade, resolve, same};
use crate::describe;
use crate::element::{Cursor, Element, Fitted, Paint, Pointer};
use crate::highlight;
use crate::icon::{self, Icon};
use crate::maths::{self, Maths, Sheet, SheetLines, Source};
use crate::motion::{self, FRAME_MS};
use crate::opening::{self, Landing, Location, Recent, Request};
use crate::paint::scrollbar;
use crate::renaming::{Outcome, Rename, Renaming};
use crate::restyle::{self, Field, StyleEdit};
use crate::spelling::{Fix, Spelling};
use crate::style::{Align, Dir, Edges, Justify, Look, Ring};
use crate::styles::{self, Styles, Styling};
use crate::text::{Family, FontShaper, TextShaper, TextStyle};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::ui::{App, Idle, Keys, Ui};
use crate::widgets::{
    Button, ControlSize, Menu, MenuItem, Popover, Side, SidePanel, TextField, Title, TopBar, side_panel, top_bar,
};

/// Space kept beside the text column.
const GUTTER: f32 = 24.0;
/// Space kept above and below the text.
const TOP: f32 = 48.0;
/// Points to logical pixels.
pub(crate) const PT: f32 = 96.0 / 72.0;
pub(crate) const CARET_WIDTH: f32 = 1.5;
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

/// The width of the chapters control with the room either side of it.
fn chapters_width() -> f32 {
    CHAPTERS_INSET.horizontal() + CHAPTER_LINE.width + f32::from(CHAPTER_STEPS) * CHAPTER_INDENT
}

/// The landing card: how many recent documents it lists, how wide it is, and the height of one of its rows.
const RECENTS: usize = 6;
const CARD_WIDTH: f32 = 544.0;
const RECENT_HEIGHT: f32 = 38.0;
/// How long the bar says which folder a document was first written to, in milliseconds.
const SAVED_NOTE_MS: u64 = 4000;
/// The size of the icon that says how saving stands.
const SAVE_ICON: f32 = 14.0;

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
    /// Nothing has been written yet, as in a new document nobody has typed in. The bar says nothing.
    Unsaved,
    /// The last write did not succeed.
    Failed,
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
    /// Puts a correction in place of the misspelled word the menu is about.
    Correct(String),
    /// Accepts that word from now on: in the person's own dictionary, or in this document's.
    Learn {
        mine: bool,
    },
    /// Puts the menu about a misspelled word away.
    Unfix,
    /// Asks the shell for a recent document, by its path.
    Open(String),
    /// Asks the shell to let the person choose a document.
    OpenDialog,
    /// Asks the shell for another new document.
    NewDocument,
    /// Puts the landing card away: this window's document is the one to write.
    Start,
    /// Renames the document where its name stands, or moves it from the flyout under the name.
    Rename(Rename),
    /// What the Styles panel asks for: the person's default, page setup and stylesheets.
    Styles(Styling),
    /// Asks the shell for something only it can do, such as moving the window.
    Ask(Request),
    /// What is done on the sheet under a formula.
    Maths(Sheet),
}

impl Ui<EditorApp> {
    /// The window on a document. It is drawn with the style its model holds: see `Model::with_style`.
    pub fn new(editor: Editor) -> Self {
        let shaper = Rc::new(FontShaper::default());
        Ui::host(EditorApp::new(editor, shaper.clone()), shaper)
    }

    pub fn editor(&self) -> &Editor {
        &self.app().editor
    }

    /// Moves the selection and brings it into view, as a click on the page does.
    pub fn select(&mut self, selection: Selection) {
        let app = self.app_mut();
        app.editor.select(selection);
        app.mark();
        app.reveal();
    }

    /// The document's style, which a shell writes when it saves.
    pub fn style(&self) -> &Style {
        self.app().editor.model().style()
    }

    /// Goes up each time the document changes.
    pub fn edits(&self) -> u64 {
        self.app().edits
    }

    /// Names the document. Shorthand for a [`Window::set_location`] that changes nothing else.
    pub fn set_document_name(&mut self, name: &str) {
        let app = self.app_mut();
        let name = Some(name.to_owned()).filter(|name| !name.is_empty());
        app.place(Location { name, ..app.location.clone() });
    }

    /// Says what the document is called and where it is kept. It is also the answer to a
    /// [`Request::Rename`], [`Request::Move`] or [`Request::NewFolder`] that was done.
    pub fn set_location(&mut self, location: Location) {
        self.app_mut().place(location);
    }

    /// Says why a rename, a move or a new folder could not be done, as the person is to read it. The name
    /// field comes back with what was typed, marked as wrong, and says so until something in it changes.
    /// `None` takes the notice away.
    pub fn set_notice(&mut self, notice: Option<&str>) {
        let app = self.app_mut();
        match notice {
            Some(notice) => {
                let asked = app.renaming.take().or(app.asked.take());
                let mut renaming = asked.unwrap_or_else(|| Renaming::open(&app.location));
                renaming.notice = Some(notice.to_owned());
                app.renaming = Some(renaming);
            }
            None => _ = app.renaming.as_mut().map(|renaming| renaming.notice = None),
        }
        app.asked = None;
    }

    /// Shows the landing card over a document nobody has written in, or takes it away. The card goes by
    /// itself when the person starts to write.
    pub fn set_landing(&mut self, landing: Option<Landing>) {
        self.app_mut().landing = landing;
    }

    /// Whether the landing card is showing.
    pub fn landing(&self) -> bool {
        self.app().landing.is_some()
    }

    /// Has the bar say for a few seconds which folder the document was written to, after its first write.
    pub fn set_saved_in(&mut self, folder: &str) {
        self.app_mut().saved_in = Some((folder.to_owned(), None));
    }

    /// Says what the person's default style is, which their new documents start with. The Styles panel
    /// then offers to save or reset to it whenever the document's style differs. `None` is for a shell that
    /// keeps no default. It is also the answer to a [`Request::SaveDefaultStyle`].
    pub fn set_default_style(&mut self, style: Option<Style>) {
        self.app_mut().styles.default = style;
    }

    /// Shows a stylesheet the person chose, by its file's name and text, in the Styles panel as it will
    /// look. Nothing changes until they apply it there. Text that is no stylesheet is refused there too, with
    /// the reason. It is the answer to a [`Request::ImportStyle`].
    pub fn offer_stylesheet(&mut self, name: &str, json: &str) {
        let app = self.app_mut();
        app.show(Some(Panel::Styles));
        app.styles.offer(name, json);
    }

    /// Says what the desktop expects of the window's frame and bar: see [`Chrome`]. Call it at the start
    /// and whenever the window's state changes. A bar that is the title bar asks for the window to be
    /// moved, resized, minimised, maximised and closed through [`Request`].
    pub fn set_chrome(&mut self, chrome: Chrome) {
        self.app_mut().chrome = chrome;
    }

    /// What the window asks of its shell, since this was last called. Call it after every event.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.app_mut().requests)
    }

    pub fn set_save_state(&mut self, state: SaveState) {
        self.app_mut().save = state;
    }

    /// The person's own words, which are right in every document: the lines of their dictionary file. A
    /// word they add from the page is handed to `Platform::learn_word` to be kept there.
    pub fn set_user_words(&mut self, words: impl IntoIterator<Item = impl AsRef<str>>) {
        self.app_mut().spelling.set_user_words(words);
    }

    /// Adds the dictionary of a language for its text to be checked against. English is built in, and text
    /// in a language without a dictionary is left alone.
    pub fn add_dictionary(&mut self, language: &str, dictionary: Dictionary) {
        self.app_mut().spelling.add_dictionary(language, dictionary);
    }

    /// Lets formulas link, as `\\href` asks. The shell turns it on for a document the person trusts, and it
    /// is off until then.
    pub fn set_maths_links(&mut self, links: bool) {
        let app = self.app_mut();
        app.held(|app| app.layout.set_maths_links(links));
        // Formulas are set again, at heights that are estimates until idle time settles them.
        app.estimated = true;
    }

    /// Runs something on the editor from outside, such as taking in another writer's edits, and brings the
    /// page up to date with what it changed. The page stays where it is scrolled to.
    pub fn change<E>(&mut self, act: impl FnOnce(&mut Editor) -> Result<Change, E>) -> Result<(), E> {
        let app = self.app_mut();
        app.held(|app| act(&mut app.editor).map(|change| app.follow(&change)))
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
        self.layout.line_edge(pos, end)
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
    lines: SheetLines<DocLines<'a>>,
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

    fn store_asset(&mut self, name: &str, bytes: &[u8]) {
        self.platform.store_asset(name, bytes);
    }

    fn learn_word(&mut self, word: &str) {
        self.platform.learn_word(word);
    }
}

pub struct EditorApp {
    editor: Editor,
    layout: DocLayout,
    /// Shapes what the window draws on a canvas of its own, as the interface's text is shaped.
    shaper: Rc<FontShaper>,
    palette: Palette,
    scheme: Scheme,
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
    location: Location,
    /// What the landing card offers, while it shows.
    landing: Option<Landing>,
    requests: Vec<Request>,
    /// The folder the bar says the document was saved in, and since when, once a frame has said so.
    saved_in: Option<(String, Option<u64>)>,
    /// Whether the caret is in the heading that will name the document.
    naming: bool,
    /// The name field in the bar, while the document is being renamed or moved.
    renaming: Option<Renaming>,
    /// What the field held when the shell was last asked for it, to bring back if the shell refuses.
    asked: Option<Renaming>,
    save: SaveState,
    edits: u64,
    panel: Option<Panel>,
    sharing: bool,
    /// The address in the link editor while it is open.
    link: Option<String>,
    /// What is typed into one of the style's fields, and the element type it was typed for.
    draft: Option<(Field, &'static str, String)>,
    styles: Styles,
    chrome: Chrome,
    /// Whether the window is the one in front. Behind another, a bar that is the title bar dims.
    active: bool,
    /// The type a row of the Styles panel has the bar show, and where the selection was then: the bar goes
    /// back to the type at the caret once the caret moves.
    picked: Option<(&'static str, Option<Selection>)>,
    folded: Folded,
    /// What each code block was last coloured for: its text and language, hashed.
    lexed: HashMap<BlockId, u64>,
    /// The Markdown markers' last change while it is still fading in, with when it began once a frame has
    /// said so.
    fading: Option<Option<u64>>,
    reduced_motion: bool,
    spelling: Spelling,
    dragging: bool,
    maths: Maths,
    /// How the document read when assistive technology was last told of it.
    told: RefCell<describe::Told>,
}

impl EditorApp {
    fn new(editor: Editor, shaper: Rc<FontShaper>) -> Self {
        let mut layout = DocLayout::new(Fonts { faces: letters_fonts::bundled() });
        // A formula stays typeset under the caret: its source is written on a sheet under it.
        layout.set_formula_source(false);
        Self {
            editor,
            layout,
            shaper,
            palette: Palette::LIGHT,
            scheme: Scheme::Light,
            column: (0.0, 0.0),
            height: 0.0,
            scroll: 0.0,
            scale: 1.0,
            estimated: true,
            location: Location::default(),
            landing: None,
            requests: Vec::new(),
            saved_in: None,
            naming: false,
            renaming: None,
            asked: None,
            save: SaveState::default(),
            edits: 0,
            panel: None,
            sharing: false,
            link: None,
            draft: None,
            styles: Styles::default(),
            chrome: Chrome::default(),
            active: true,
            picked: None,
            folded: Folded::default(),
            lexed: HashMap::new(),
            fading: None,
            reduced_motion: false,
            spelling: Spelling::default(),
            dragging: false,
            maths: Maths::default(),
            told: RefCell::default(),
        }
    }

    /// The first Heading 1, which names a document that has no name of its own.
    fn heading(&self) -> Option<&Block> {
        self.editor.doc().walk().find(|block| matches!(block.kind, BlockKind::Heading { level: 1 }))
    }

    /// The name the bar shows, and whether it is the document's own. Without one it is what the first
    /// Heading 1 says so far.
    fn name(&self) -> (String, bool) {
        if let Some(name) = &self.location.name {
            return (name.clone(), true);
        }
        let heading = self.heading().map(|heading| heading.plain_text().trim().to_owned());
        (heading.filter(|heading| !heading.is_empty()).unwrap_or_else(|| "Untitled".into()), false)
    }

    /// Takes the shell's word for where the document is, which also answers what the name field asked.
    fn place(&mut self, location: Location) {
        self.location = location;
        self.asked = None;
        if let Some(renaming) = &mut self.renaming {
            renaming.placed(&self.location);
        }
    }

    /// What the name field and its flyout ask for.
    fn rename(&mut self, message: Rename) {
        let Some(mut renaming) = self.renaming.take() else {
            self.renaming = (message == Rename::Open).then(|| Renaming::open(&self.location));
            return;
        };
        match renaming.update(message, &self.location, &mut self.requests) {
            Outcome::Open => self.renaming = Some(renaming),
            Outcome::Asked => self.asked = Some(renaming),
            Outcome::Closed => {}
        }
    }

    /// Puts the landing card away and the caret on the page.
    fn start(&mut self) {
        self.landing = None;
        if self.editor.selection().is_none() {
            let first = self.editor.doc().walk().find(|block| matches!(block.body, Body::Text(_)));
            if let Some(first) = first.map(|block| Pos::new(block.id.clone(), 0)) {
                self.editor.select(Selection::caret(first));
                self.mark();
            }
        }
    }

    /// The element type whose style the bar shows: the one the caret's block is drawn with, unless a row of
    /// the Styles panel picked another since the caret last moved.
    fn element(&self) -> Option<&'static str> {
        let selection = self.editor.selection();
        match &self.picked {
            Some((element, at)) if at.as_ref() == selection => Some(element),
            _ => restyle::element(self.editor.doc(), &selection?.head.block),
        }
    }

    /// Keeps up which form of the bar fits a window of `width`: forgotten when something that decides it
    /// has changed, and learnt from the layout after that.
    fn refold(&mut self, width: f32) {
        // The name field keeps its place in the bar for as long as it is open.
        if self.renaming.is_some() && self.folded.form.is_some() {
            return;
        }
        let types = &self.editor.model().style().types;
        let style = self.element().and_then(|element| Some((element, &types.get(element)?.family)));
        let mut hasher = DefaultHasher::new();
        let differs = self.styles.differs(self.editor.model().style());
        let chrome = (self.chrome.buttons.len(), self.chrome.inset_start.to_bits());
        (width.to_bits(), self.name(), style, differs, chrome).hash(&mut hasher);
        let key = hasher.finish();
        if key != self.folded.key {
            self.folded = Folded { key, form: None, seen: Fitted::default() };
        } else if self.folded.form.is_none() {
            self.folded.form = self.folded.seen.get();
        }
    }

    /// Restyles every block of the type under the caret, as one step that undo takes back.
    fn restyle(&mut self, edit: &StyleEdit, platform: &mut dyn Platform) {
        let Some(element) = self.element() else { return };
        let Some(mut style) = self.editor.model().style().types.get(element).cloned() else { return };
        edit.apply(&mut style);
        let command = Command::SetStyle { element: element.to_owned(), style };
        self.edit(platform, |editor, platform| editor.run(command, platform));
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

    /// Shows the Markdown markers that belong where the caret now is, after anything that may have moved it.
    /// A change fades in. It shows at once when motion is reduced, and while an input method composes, whose
    /// text must not slide under its window.
    fn mark(&mut self) {
        let head = self.editor.selection().map(|selection| selection.head.clone());
        let mut markers = self.editor.markers();
        // A formula's source is on its sheet, so nothing on the page has to say where it begins and ends.
        let dollar = |marker: &Marker| marker.text == "$";
        markers.before_caret -= markers.inline[..markers.before_caret].iter().filter(|marker| dollar(marker)).count();
        markers.inline.retain(|marker| !dollar(marker));
        let changed = self.layout.reveal(head.as_ref().map(|head| (head, &markers)));
        self.layout.set_composition(self.editor.composition());
        let caret = self.editor.selection().filter(|selection| selection.is_caret()).map(|caret| &caret.head);
        self.spelling.moved(caret, &mut self.layout);
        // A document without a name takes its first Heading 1, once the caret has left it.
        let heading = self.heading().filter(|_| self.location.name.is_none());
        let naming = heading.zip(head.as_ref()).is_some_and(|(heading, head)| heading.id == head.block);
        let name = heading.map(|heading| heading.plain_text().trim().to_owned()).filter(|name| !name.is_empty());
        if let (true, false, Some(name)) = (self.naming, naming, name) {
            self.requests.push(Request::Named(name));
        }
        self.naming = naming;
        let composing = self.editor.composition().is_some();
        if changed {
            let at_once = self.reduced_motion || composing;
            self.fading = (!at_once).then_some(None);
            self.layout.set_reveal_progress(if at_once { 1.0 } else { 0.0 });
        } else if composing && self.fading.take().is_some() {
            self.layout.set_reveal_progress(1.0);
        }
    }

    /// Runs something on the editor, which may ask the layout about lines while it works.
    fn edit(&mut self, platform: &mut dyn Platform, act: impl FnOnce(&mut Editor, &mut dyn Platform) -> Change) {
        // On a sheet the caret moves by the lines of the source, which the page does not show.
        let source = Source::of(&self.editor, self.shaper.as_ref());
        let formula = self.editor.selection().and_then(|selection| self.layout.formula_rect(&selection.head));
        let room = 2.0 * self.column.0 + self.column.1;
        let left = source.as_ref().zip(formula).map(|(source, rect)| Maths::left(source, rect.x, rect.width, room));
        let lines = SheetLines {
            page: DocLines::new(&self.layout, self.height),
            source: source.filter(|_| left.is_some()),
            left: left.unwrap_or_default(),
        };
        let mut lent = Lent { platform, lines };
        let change = act(&mut self.editor, &mut lent);
        self.changed(&change);
    }

    /// Brings the layout and the viewport up to date after the editor did something.
    fn changed(&mut self, change: &Change) {
        self.follow(change);
        self.reveal();
    }

    /// Brings the layout up to date with a change to the document, and what is marked on the page with it.
    fn follow(&mut self, change: &Change) {
        let edited = !change.blocks.is_empty() || change.structure;
        if edited || change.style {
            // Every block is drawn from the style, so a change to it lays all of them out again.
            let touched = (!change.style).then_some(change);
            self.layout.update(self.editor.doc(), self.editor.model().style(), self.column.1, touched);
            self.estimated = true;
        }
        if edited {
            self.highlight((!change.structure).then_some(&change.blocks));
            let caret = self.editor.selection().filter(|selection| selection.is_caret()).map(|caret| &caret.head);
            self.spelling.edited(change, self.editor.doc(), caret, &mut self.layout);
        }
        if change.document {
            self.spelling.reworded(None, self.editor.doc(), &mut self.layout);
        }
        if edited || change.style || change.document {
            self.edits += 1;
        }
        self.mark();
    }

    /// Does something that may change the height of what is above the view, and keeps the view's top line
    /// where it is.
    fn held<R>(&mut self, act: impl FnOnce(&mut Self) -> R) -> R {
        let pin = self.layout.hit(Point { x: 0.0, y: self.viewport().y.max(0.0) });
        let top = |layout: &DocLayout| pin.as_ref().and_then(|pos| layout.caret(pos)).map(|caret| caret.y);
        let before = top(&self.layout);
        let done = act(self);
        if let (Some(before), Some(after)) = (before, top(&self.layout)) {
            self.scroll_by(after - before);
        }
        done
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
                self.picked = None;
                self.maths.typed(false);
                // Under the last block there is always a line to write on, also when the document ends in
                // something that holds the caret in, as a code block does.
                if at.y + self.scrolled() - TOP > self.layout.height() && !mods.shift {
                    self.dragging = false;
                    return self.edit(platform, |editor, platform| editor.run(Command::LineBelow, platform));
                }
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
            Pointer::Up { .. } => {
                self.dragging = false;
                // The markers at the caret show once the button is up: text that made room for them
                // under a pointer still held would be selected by its next move.
                self.mark();
            }
            Pointer::Menu { at } => self.menu(at),
            Pointer::Scroll { delta } => self.scroll_by(delta.y),
        }
    }

    /// Opens the menu about what is at a point of the page area: for now, a word marked as misspelled.
    fn menu(&mut self, at: Point) {
        let Some(pos) = self.hit(at) else { return };
        let Some(range) = self.layout.spelling_at(&pos) else { return };
        // The word itself was meant, not the room its line leaves beside it.
        let word = Selection {
            anchor: Pos::new(pos.block.clone(), range.start),
            head: Pos::new(pos.block.clone(), range.end),
        };
        let spot = Point { x: at.x - self.column.0, y: at.y + self.scrolled() - TOP };
        let near = |rect: &Rect| {
            (rect.x - 2.0..=rect.x + rect.width + 2.0).contains(&spot.x)
                && (rect.y..=rect.y + rect.height).contains(&spot.y)
        };
        if !self.layout.selection(&word).iter().any(near) {
            return;
        }
        // A selection is left alone, so that the menu can come to serve it as well.
        if self.editor.selection().is_none_or(Selection::is_caret) {
            self.editor.select(Selection::caret(pos.clone()));
            self.mark();
        }
        self.link = None;
        self.spelling.ask(self.editor.doc(), &pos, &self.layout);
    }

    /// The landing card, over the middle of a page nobody has written on: the documents opened lately, and
    /// the ways to open another or to start this one. A press on the page behind it starts this one.
    fn card(landing: &Landing, room: Size) -> Popover<Message> {
        let quiet = |text: &str| Element::text(text, TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED);
        let recent = |recent: &Recent| {
            let when = opening::ago(&landing.now, &recent.opened);
            Element::row()
                .key(recent.path.as_str())
                .align(Align::Center)
                .gap(tokens::SPACE_SM)
                .height(RECENT_HEIGHT)
                .padding(Edges::xy(tokens::SPACE_SM, 0.0))
                .radius(tokens::RADIUS_CONTROL)
                .hover(Look::bg(tokens::NEUTRAL_SOFT))
                .focused(Look::bg(tokens::NEUTRAL_SOFT))
                .role(Role::ListBoxOption)
                .label(recent.name.as_str())
                .description(format!("{}, {when}", recent.folder))
                .focusable()
                .on_press(Message::Open(recent.path.clone()))
                .child(Element::icon(icon::FILE_TEXT, 16.0).fg(tokens::FG_MUTED))
                .child(Element::text(recent.name.as_str(), TextStyle::control().weight(500)).shrink())
                .child(quiet(&recent.folder).shrink())
                .child(Element::row().grow(1.0))
                .child(quiet(&when))
        };
        let recents: Vec<Element<Message>> = landing.recents.iter().take(RECENTS).map(recent).collect();
        let listed = recents.len() as f32;

        let hint = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .grow(1.0)
            .shrink()
            .child(Element::icon(icon::MOUSE_POINTER_CLICK, 14.0).fg(tokens::FG_MUTED))
            .child(quiet("Or click the page to start writing").shrink());
        let foot = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .child(hint)
            .child(Button::new("Open").icon(icon::FOLDER_OPEN).on_press(Message::OpenDialog))
            .child(Button::new("New document").icon(icon::PLUS).primary().on_press(Message::Start));

        let mut card = Element::column().gap(tokens::SPACE_SM).width(CARD_WIDTH);
        if !recents.is_empty() {
            let title = Element::text("Recent", TextStyle::control().size(tokens::TEXT_SM).weight(600));
            let list = Element::column()
                .role(Role::ListBox)
                .label("Recent documents")
                .arrows(Dir::Column)
                .one_stop()
                .children(recents);
            let line = Element::row().height(1.0).bg(tokens::LINE).decorative();
            card = card
                .child(title.fg(tokens::FG_MUTED).padding(Edges::xy(tokens::SPACE_SM, 0.0)))
                .child(list)
                .child(line);
        }
        // The card sits a little above the middle of the page area.
        let height = listed * RECENT_HEIGHT + if listed > 0.0 { 120.0 } else { 64.0 };
        let spot = Rect::new(room.width / 2.0, ((room.height - height) * 0.4).max(0.0), 0.0, 0.0);
        Popover::new(card.child(foot))
            .label("Open a document")
            .max_width(CARD_WIDTH + 2.0 * tokens::SPACE_MD)
            .at(spot)
            .align(Align::Center)
            .gap(0.0)
            .shows_focus()
            .open(true)
            .on_close(Message::Start)
    }

    /// The menu about a misspelled word: corrections, and the two dictionaries that can take the word.
    fn fixes(fix: &Fix) -> Menu<Message> {
        let mut menu = Menu::new().label("Spelling");
        for suggestion in &fix.suggestions {
            menu = menu.item(MenuItem::new(suggestion).on_select(Message::Correct(suggestion.clone())));
        }
        if fix.suggestions.is_empty() {
            menu = menu.item(MenuItem::new("No suggestions").disabled(true));
        }
        menu.separator()
            .item(MenuItem::new("Add to this document").on_select(Message::Learn { mine: false }))
            .item(MenuItem::new("Add to my dictionary").on_select(Message::Learn { mine: true }))
    }

    /// Lays the document out for the room it has and paints the part in view.
    fn page(&mut self, room: Size) -> Element<Message> {
        let page = &self.editor.model().style().page;
        let text = (page.width - page.margin_left - page.margin_right) * PT;
        let width = text.min(room.width - 2.0 * GUTTER).max(1.0);
        let left = ((room.width - width) / 2.0).max(0.0).round();
        if width != self.column.1 {
            self.layout.update(self.editor.doc(), self.editor.model().style(), width, None);
            self.highlight(None);
        }
        (self.column, self.height) = ((left, width), room.height);
        // A heading's markers hang in the margin, beside the outline when it shows.
        let taken = if self.outlined() { chapters_width() } else { tokens::SPACE_SM };
        self.layout.set_margin(left - taken);
        // What is about to show is laid out for real. The rest waits for idle time.
        self.estimated = self.layout.settle(self.viewport(), 0);
        self.scroll = self.scroll.min(self.max_scroll());

        let viewport = self.viewport();
        let origin = Point { x: left, y: TOP - self.scrolled() };
        let mut list = DisplayList::default();
        let selection = self.editor.selection();
        // A formula with the caret in it has its source out on a sheet, while the place under it is in view
        // and nothing else hangs from the page.
        let alone = self.landing.is_none() && self.spelling.fix.is_none() && self.link.is_none();
        let formula = selection.filter(|_| alone).and_then(|selection| self.layout.formula_rect(&selection.head));
        let formula = formula.filter(|rect| (0.0..=room.height).contains(&(origin.y + rect.y + rect.height)));
        let sheet = formula.and_then(|rect| Some((Source::of(&self.editor, self.shaper.as_ref())?, rect)));
        self.maths.follow(sheet.as_ref().map(|(source, _)| source), self.reduced_motion);
        match &sheet {
            Some((_, rect)) => list.items.push(maths::tint(*rect, self.scheme)),
            // On a sheet, the sheet shows what is selected.
            None => {
                let color = self.palette.selection;
                let ranged = selection.filter(|selection| !selection.is_caret());
                let spans = ranged.map(|selection| self.layout.selection_in(selection, viewport)).unwrap_or_default();
                list.items.extend(spans.into_iter().map(|rect| fill(rect, color)));
            }
        }
        self.layout.paint(viewport, &mut list);
        // A formula that was just closed still has its source over it, on the way out.
        match self.maths.closing().map(|at| self.layout.formula_rect(at)) {
            Some(Some(rect)) => self.maths.blend(rect, self.palette.text, &mut list),
            Some(None) => self.maths.settle(),
            None => {}
        }

        let caret = selection.filter(|_| sheet.is_none()).and_then(|selection| self.layout.caret(&selection.head));
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
        if let Some(landing) = &self.landing {
            element = element.popover(Self::card(landing, room));
        } else if let Some(fix) = &self.spelling.fix {
            // The menu hangs from the start of its word.
            let start = self.layout.caret(&Pos::new(fix.block.clone(), fix.range.start)).unwrap_or_default();
            let spot = Rect { x: origin.x + start.x, y: origin.y + start.y, ..start };
            let menu = Self::fixes(fix).popover().at(spot).open(true).on_close(Message::Unfix);
            element = element.popover(menu);
        } else if let Some(address) = &self.link {
            let field = TextField::new("Link", address)
                .placeholder("Paste or type a link")
                .on_input(Message::LinkDraft)
                .on_submit(Message::LinkApply);
            let editor =
                Popover::new(Element::from(field).width(300.0)).label("Link").open(true).on_close(Message::LinkClose);
            // The editor hangs from the caret, in place.
            element = element.popover(editor.at(caret.unwrap_or_default())).cursor(Cursor::Text);
        } else if let Some(((source, rect), selection)) = sheet.zip(selection) {
            let spot = Rect { x: origin.x + rect.x, y: origin.y + rect.y, ..rect };
            element =
                element.popover(self.maths.sheet(source, selection, rect.width, room.width, self.scheme).at(spot));
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

    /// Whether the outline shows: the margin left of the text holds it, so no text runs under it, and the
    /// document has more than the one heading that is no outline.
    fn outlined(&self) -> bool {
        let heading = |block: &&Block| matches!(block.kind, BlockKind::Heading { .. });
        self.column.0 >= chapters_width() && self.editor.doc().walk().filter(heading).take(2).count() == 2
    }

    /// The document's outline, at the top left of the page: one short line per heading, stepped in by its
    /// level. A line names its heading in a tooltip, and pressing it brings the heading to the top.
    fn chapters(&self) -> Option<Element<Message>> {
        if !self.outlined() {
            return None;
        }
        let level = |block: &Block| match block.kind {
            BlockKind::Heading { level } => Some(level),
            _ => None,
        };
        let headings: Vec<(&Block, u8)> =
            self.editor.doc().walk().filter_map(|block| Some((block, level(block)?))).collect();
        let shallowest = headings.iter().map(|heading| heading.1).min()?;
        // The lines close up when the page has no room for them all.
        let room = (self.height - CHAPTERS_INSET.vertical()).max(0.0);
        let pitch = (room / headings.len() as f32).floor().clamp(CHAPTER_PITCH.1, CHAPTER_PITCH.0);
        let width = chapters_width() - CHAPTERS_INSET.horizontal();
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
            .item(MenuItem::new("New document").icon(icon::PLUS).on_select(Message::NewDocument))
            .item(MenuItem::new("Open").icon(icon::FOLDER_OPEN).on_select(Message::OpenDialog))
            .separator()
            .item(MenuItem::new("Export").icon(icon::DOWNLOAD).disabled(true))
            .item(MenuItem::new("Print view").icon(icon::PRINTER).disabled(true))
            .item(MenuItem::new("Page setup").icon(icon::SETTINGS).disabled(true));

        // The forms of the bar, from the widest. Once the layout has said which fits, only that one is built.
        let forms = [
            (NAME_WIDTH.2, Fold::None),
            (NAME_WIDTH.1, Fold::None),
            (NAME_WIDTH.1, Fold::Alignment),
            (NAME_WIDTH.1, Fold::Marks),
            (NAME_WIDTH.1, Fold::Style),
        ];
        let chrome = &self.chrome;
        let mut bar = TopBar::new().shown(self.folded.seen.clone()).inset(chrome.inset_start);
        if let Some(height) = chrome.bar_height {
            bar = bar.height(height);
        }
        // A bar that is the title bar moves its window, and dims while the window is behind another.
        if chrome.titled() {
            let title = Title {
                drag: Message::Ask(Request::DragWindow),
                double_click: Message::Ask(Request::ToggleMaximise),
                menu: Box::new(|at| Message::Ask(Request::WindowMenu(at))),
            };
            bar = bar.title(title).dim(!self.active);
        }
        // The window's buttons stand where the desktop keeps them, a rule apart from the bar's own.
        let buttons = chrome.buttons();
        if let Some(buttons) = buttons.filter(|_| chrome.buttons_at_start) {
            bar = bar.before(buttons).before(TopBar::rule());
        }
        for (n, (name, fold)) in forms.into_iter().enumerate() {
            if self.folded.form.is_none_or(|form| form == n) {
                let shown = self.folded.form == Some(n);
                bar = self.lead(name, shown).into_iter().fold(bar.form(n), TopBar::start);
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
        bar = bar.end(panels.child(tool(icon::ELLIPSIS, "More").popover(more.popover().align(Align::End))));
        if let Some(buttons) = chrome.buttons().filter(|_| !chrome.buttons_at_start) {
            bar = bar.end(TopBar::rule()).end(buttons);
        }
        bar.into()
    }

    /// What the bar starts with: the document's name, in no more than `room`, and whether it is saved. A
    /// press on the name makes it a field, in the form of the bar that is `shown`.
    fn lead(&self, room: f32, shown: bool) -> Vec<Element<Message>> {
        // A name that is not the document's own is grey, as a field's placeholder is.
        let (name, own) = self.name();
        let page = Element::icon(icon::FILE_TEXT, 18.0).fg(tokens::FG_MUTED);
        if let Some(renaming) = self.renaming.as_ref().filter(|_| shown) {
            return vec![page, renaming.field(&name, room, &self.location).min_width(NAME_WIDTH.0)];
        }
        let style = TextStyle::control().weight(if own { 600 } else { 400 });
        // The name gives way to the controls after it, down to a few letters.
        let name = Element::text(name.as_str(), style)
            .min_width(NAME_WIDTH.0)
            .max_width(room)
            .shrink()
            .role(Role::Button)
            .label(name)
            .description("Rename or move")
            .tooltip("Rename or move", Side::Bottom)
            .on_press(Message::Rename(Rename::Open))
            .cursor(Cursor::Text);
        // On a title bar a drag that starts on the name moves the window, and only a click renames.
        let name = if self.chrome.titled() { name.on_drag(Message::Ask(Request::DragWindow)) } else { name };
        let name = if own { name } else { name.fg(tokens::FG_MUTED) };
        let mut lead = vec![page, name];

        // How saving stands is an icon of one size whatever it says, so that nothing after it moves when
        // saving starts. A document not yet written keeps the room.
        let (state, said, more, ink) = match self.save {
            SaveState::Saved => (icon::CLOUD_CHECK, "Saved", None, tokens::FG_MUTED),
            SaveState::Saving => (icon::REFRESH_CW, "Saving", None, tokens::FG_MUTED),
            SaveState::Offline => {
                (icon::CLOUD_OFF, "Offline", Some("Changes are kept on this device"), tokens::FG_MUTED)
            }
            SaveState::Failed => {
                (icon::CLOUD_ALERT, "Not saved", Some("The last changes could not be written"), tokens::DANGER)
            }
            SaveState::Unsaved => {
                lead.push(Element::row().width(SAVE_ICON).decorative());
                return lead;
            }
        };
        // For a moment after its first write, the icon tells where the document went.
        let note = self.saved_in.as_ref().filter(|_| self.save == SaveState::Saved);
        let said = note.map_or_else(|| said.to_owned(), |(folder, _)| format!("Saved in {folder}"));
        let tip = more.map_or_else(|| said.clone(), |more| format!("{said}. {more}"));
        let mut saved = Element::icon(state, SAVE_ICON)
            .fg(ink)
            .role(Role::Label)
            .label(said)
            .tooltip(tip, Side::Bottom)
            .telling(note.is_some());
        if let Some(more) = more {
            saved = saved.description(more);
        }
        lead.push(saved);
        lead
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
        let Some(style) = self.editor.model().style().types.get(element) else { return Vec::new() };

        // The type follows the caret: it is named here and changed on the page. A dot says that the
        // document's style is no longer the person's default, which the Styles panel can mend.
        let name = restyle::name(element);
        let differs = self.styles.differs(self.editor.model().style());
        let label = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .height(tokens::CONTROL_HEIGHT_SM)
            .min_width(TYPE_WIDTH)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .radius(tokens::RADIUS_CONTROL)
            .bg(tokens::TONE_1)
            .fg(tokens::FG_MUTED)
            .child(Element::text(name.clone(), TextStyle::control().size(tokens::TEXT_SM).weight(500)))
            .children(differs.then(styles::dot));

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
            let opens = "Opens the Styles panel";
            let about = if differs { format!("Differs from your default. {opens}") } else { opens.to_owned() };
            let label = label
                .hover(Look::bg(tokens::NEUTRAL_SOFT))
                .ring(Ring::Outline(2.0))
                .role(Role::Button)
                .label(name)
                .description(about.clone())
                .tooltip(about, Side::Bottom)
                .on_press(Message::Styles(Styling::Show));
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
        let empty = |said: &str| {
            let said = Element::paragraph(said, TextStyle::body()).fg(tokens::FG_MUTED);
            Element::column().padding(Edges::top(tokens::SPACE_MD)).child(said)
        };
        let body = match panel {
            Panel::Styles => self.styles.view(self.editor.model().style(), self.element(), &self.palette),
            Panel::Notes => empty("No open notes."),
            Panel::History => empty("No earlier versions yet."),
        };
        SidePanel::new(panel.title()).on_close(Message::ClosePanel).child(body).into()
    }

    /// Shows a panel, or none. What the Styles panel was in the middle of goes with it.
    fn show(&mut self, panel: Option<Panel>) {
        if self.panel == Some(Panel::Styles) && panel != self.panel {
            self.picked = None;
            self.styles.leave();
        }
        self.panel = panel;
    }

    /// Does what was asked on the sheet under the formula the caret is in.
    fn sheet(&mut self, message: Sheet, platform: &mut dyn Platform) {
        let Some((block, source)) = self.editor.formula().map(|(block, source)| (block.clone(), source)) else {
            return;
        };
        let at = |offset: usize| Pos::new(block.clone(), offset.clamp(source.start, source.end));
        match message {
            Sheet::Press { offset, extend, clicks } => {
                let head = at(offset);
                let anchor = match self.editor.selection() {
                    Some(selection) if extend => selection.anchor.clone(),
                    _ => head.clone(),
                };
                self.editor.select(Selection { anchor, head });
                match clicks {
                    1 => self.stay(offset == source.start, platform),
                    2 => self.edit(platform, |editor, platform| editor.run(Command::SelectWord, platform)),
                    _ => self.editor.select(Selection { anchor: at(source.start), head: at(source.end) }),
                }
                self.maths.dragging = clicks == 1;
                self.maths.typed(false);
                self.mark();
            }
            Sheet::Drag(offset) => {
                // A pointer that has not left its character leaves the caret as the press put it.
                let held = self.editor.selection().filter(|selection| selection.head != at(offset));
                if let Some(anchor) = held.filter(|_| self.maths.dragging).map(|selection| selection.anchor.clone()) {
                    self.editor.select(Selection { anchor, head: at(offset) });
                    self.mark();
                }
            }
            Sheet::Release => self.maths.dragging = false,
            Sheet::Take(index) => self.take(index, platform),
            // Escape puts the list of symbols away first.
            Sheet::Leave if self.symbols().is_some() => self.maths.typed(false),
            Sheet::Leave => self.edit(platform, |editor, platform| editor.run(Command::LeaveFormula, platform)),
        }
    }

    /// The symbols the sheet offers for the command being typed at the caret, and the stretch of the
    /// formula's block that one of them would replace.
    fn symbols(&self) -> Option<(Pos, usize, Vec<&'static letters_math::Command>)> {
        let source = Source::of(&self.editor, self.shaper.as_ref())?;
        let (typed, symbols) = self.maths.symbols(&source, self.editor.selection()?)?;
        Some((Pos::new(source.block, typed.start), typed.end, symbols))
    }

    /// Writes a symbol of the list in place of the command being typed. One that takes arguments gets its
    /// braces, and the caret waits in the first pair.
    fn take(&mut self, index: usize, platform: &mut dyn Platform) {
        let Some((start, end, symbols)) = self.symbols() else { return };
        let Some(symbol) = symbols.get(index) else { return };
        self.editor.select(Selection { anchor: start.clone(), head: Pos::new(start.block.clone(), end) });
        let written = InputEvent::Insert(maths::written(symbol));
        self.edit(platform, |editor, platform| editor.input(written, platform));
        if symbol.arguments > 0 {
            let first = Pos::new(start.block, start.offset + symbol.command.len() + 1);
            self.editor.select(Selection::caret(first));
            self.mark();
        }
        self.maths.typed(false);
    }

    /// Lets the list of symbols have the keys that walk it and take from it, while it shows.
    fn list_key(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let plain = event.mods == Modifiers::default();
        let Some((_, _, symbols)) = self.symbols().filter(|_| plain) else { return false };
        match event.key {
            Key::Down | Key::Up => self.maths.pick(event.key == Key::Down, symbols.len()),
            Key::Tab => self.take(self.maths.picked(symbols.len()), platform),
            _ => return false,
        }
        true
    }

    /// Blends the source of a formula into its glyphs, when a `$` typed at `before` has just closed one.
    /// `start` is where the block of `before` began on the page then.
    fn close(&mut self, before: &Pos, start: Option<Rect>) {
        let Some(block) = self.editor.doc().block(&before.block) else { return };
        let Some(head) = self.editor.selection().map(|selection| &selection.head) else { return };
        let closed = if block.kind == BlockKind::Formula {
            // `$$x$$` alone in its paragraph made a formula of it, and the caret went on below. The source
            // began where the paragraph did, and the formula may stand anywhere on its line.
            let from = start.map(|start| Point { x: start.x, y: start.y + start.height / 2.0 });
            (head.block != block.id).then(|| (0, format!("$${}$$", block.plain_text()), from))
        } else if head.block == block.id && self.editor.formula().is_none() {
            // Inline maths that ends at the caret, which is outside it, was closed there.
            let (mut at, mut from) = (0, None);
            for run in block.body.runs() {
                if at >= head.offset {
                    break;
                }
                from = if run.marks.contains(&Mark::Math) { from.or(Some(at)) } else { None };
                at += run.text.len();
            }
            let text = block.plain_text();
            let source = from.filter(|_| at == head.offset).and_then(|from| Some((from, text.get(from..at)?)));
            source.map(|(from, source)| (from, format!("${source}$"), None))
        } else {
            None
        };
        let Some((offset, typed, from)) = closed else { return };
        // The source goes as it was written: in the face of its block, as near as the interface has it.
        let types = &self.editor.model().style().types;
        let written = restyle::element(self.editor.doc(), &block.id).and_then(|element| types.get(element));
        let face = written.map_or(TextStyle::body(), |style| TextStyle {
            family: FAMILIES.iter().find(|(family, _)| *family == style.family).map_or(Family::Sans, |found| found.1),
            size: style.size * PT,
            weight: style.weight,
            italic: style.italic,
            leading: style.line_height,
        });
        let typed = self.shaper.shape(&typed.replace('\n', " "), &face, None);
        self.maths.close(Pos::new(block.id.clone(), offset), typed, from);
    }

    /// Keeps a caret set at an end of inline maths inside it. Left there, text typed next would be written
    /// beside the formula, and the sheet that was pressed would go.
    fn stay(&mut self, at_start: bool, platform: &mut dyn Platform) {
        if self.editor.formula().is_none() {
            let to = if at_start { Motion::Right } else { Motion::Left };
            self.edit(platform, |editor, platform| editor.run(Command::Move { to, extend: false }, platform));
        }
    }

    /// What the Styles panel asks for. A change to the style is one step for undo, and files are asked of
    /// the shell.
    fn styling(&mut self, message: Styling, platform: &mut dyn Platform) {
        let style = self.editor.model().style();
        let command = match message {
            Styling::Show => {
                self.show(Some(Panel::Styles));
                None
            }
            Styling::Pick(element) => {
                self.picked = Some((element, self.editor.selection().cloned()));
                None
            }
            Styling::SaveDefault => {
                let kept = self.styles.offered.take().map_or_else(|| style.clone(), |(_, offered)| offered);
                self.requests.push(Request::SaveDefaultStyle(kept.to_json()));
                self.styles.default = Some(kept);
                None
            }
            Styling::Import => {
                self.styles.refused = None;
                self.requests.push(Request::ImportStyle);
                None
            }
            Styling::Export => {
                self.requests.push(Request::ExportStyle(style.to_json()));
                None
            }
            Styling::Decline => {
                self.styles.offered = None;
                None
            }
            Styling::Margins(typed) => {
                self.styles.margins = Some(typed);
                None
            }
            Styling::ForgetMargins => {
                self.styles.margins = None;
                None
            }
            Styling::Reset => self.styles.default.clone().map(Command::ReplaceStyle),
            Styling::Apply => self.styles.offered.take().map(|(_, offered)| Command::ReplaceStyle(offered)),
            Styling::Paper(paper) => styles::with_paper(&style.page, paper).map(Command::SetPage),
            Styling::Landscape(landscape) => Some(Command::SetPage(styles::turned(&style.page, landscape))),
            Styling::TakeMargins => {
                let typed = self.styles.margins.take();
                typed.and_then(|typed| styles::with_margins(&style.page, &typed)).map(Command::SetPage)
            }
        };
        if let Some(command) = command {
            self.edit(platform, |editor, platform| editor.run(command, platform));
        }
    }
}

impl App for EditorApp {
    type Msg = Message;

    fn title(&self) -> String {
        self.location.name.clone().unwrap_or_else(|| "Letters".into())
    }

    fn view(&mut self, window: Size) -> Element<Message> {
        // A floating window that Letters frames has a border, which everything else stands inside.
        let border = self.chrome.border();
        let window = Size { width: window.width - 2.0 * border, height: window.height - 2.0 * border };
        self.refold(window.width);
        let panel = self.panel.map(|panel| self.side(panel));
        let room = window.width - if panel.is_some() { side_panel::WIDTH } else { 0.0 };
        let bar = self.chrome.bar_height.unwrap_or(top_bar::HEIGHT);
        let page = self.page(Size { width: room.max(0.0), height: (window.height - bar).max(0.0) });
        let page = Element::row().grow(1.0).child(page).children(self.chapters());
        let body = Element::row().grow(1.0).child(page).children(panel);
        let all = Element::column().bg(tokens::BG).fg(tokens::FG).child(self.bar()).child(body);
        if border > 0.0 { all.border(Edges::all(border), tokens::LINE_STRONG) } else { all }
    }

    fn update(&mut self, message: Message, platform: &mut dyn Platform) {
        match message {
            Message::Text(event) => {
                let dollar =
                    matches!(&event, InputEvent::Insert(text) | InputEvent::Commit(text) if text.ends_with('$'));
                // Where the caret is and where its block starts, should this close a formula.
                let before = self.editor.selection().filter(|_| dollar && !self.reduced_motion);
                let before = before.map(|selection| selection.head.clone());
                let before = before.map(|head| (self.layout.caret(&Pos::new(head.block.clone(), 0)), head));
                self.edit(platform, |editor, platform| editor.input(event, platform));
                self.maths.typed(true);
                if let Some((start, head)) = before {
                    self.close(&head, start);
                }
            }
            Message::Pointer(event) => self.pointer(event, platform),
            Message::Run(command) => self.edit(platform, |editor, platform| editor.run(command, platform)),
            Message::Panel(panel) => self.show((self.panel != Some(panel)).then_some(panel)),
            Message::ClosePanel => self.show(None),
            Message::Share(open) => self.sharing = open,
            Message::Correct(text) => {
                let Some(fix) = self.spelling.fix.take() else { return };
                let (anchor, head) = (Pos::new(fix.block.clone(), fix.range.start), Pos::new(fix.block, fix.range.end));
                self.editor.select(Selection { anchor, head });
                self.edit(platform, |editor, platform| editor.input(InputEvent::Insert(text), platform));
            }
            Message::Learn { mine } => {
                let Some(fix) = self.spelling.fix.take() else { return };
                if mine {
                    platform.learn_word(&fix.word);
                    self.spelling.learn(&fix.word);
                } else {
                    // The document's own words are part of it, so undo takes one back and saving keeps it.
                    let word = Command::AddWord(fix.word);
                    self.edit(platform, |editor, platform| editor.run(word, platform));
                }
                self.spelling.reworded(Some(fix.block), self.editor.doc(), &mut self.layout);
            }
            Message::Unfix => self.spelling.fix = None,
            Message::Open(path) => self.requests.push(Request::Open(path)),
            Message::OpenDialog => self.requests.push(Request::OpenDialog),
            Message::NewDocument => self.requests.push(Request::New),
            Message::Start => self.start(),
            Message::Rename(message) => self.rename(message),
            Message::Styles(message) => self.styling(message, platform),
            Message::Ask(request) => self.requests.push(request),
            Message::Maths(message) => self.sheet(message, platform),
            Message::Jump(block) => {
                let start = Pos { block, offset: 0 };
                self.editor.select(Selection::caret(start.clone()));
                self.mark();
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
                self.restyle(&edit, platform);
            }
            Message::Forget => self.draft = None,
            Message::Draft(field, text) => self.draft = self.element().map(|element| (field, element, text)),
            Message::Apply(field) => {
                let typed =
                    self.draft.take_if(|draft| draft.0 == field).filter(|draft| Some(draft.1) == self.element());
                if let Some(edit) = typed.and_then(|draft| field.read(&draft.2)) {
                    self.restyle(&edit, platform);
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
        if self.list_key(event, platform) {
            return true;
        }
        let (mut used, mut edited) = (false, false);
        self.edit(platform, |editor, platform| {
            let change = editor.key(event.clone(), platform);
            used = change.is_some();
            edited = change.as_ref().is_some_and(|change| !change.blocks.is_empty());
            change.unwrap_or_default()
        });
        // A key that deletes is typing too. One that moves the caret is not, and puts the symbols away.
        if used {
            self.maths.typed(edited);
        }
        // The editor leaves this one to the window, which edits the link in place.
        let link = event.key == Key::Char('k') && event.mods == Modifiers { primary: true, ..Modifiers::default() };
        if !used && link {
            self.link = Some(self.editor.link().unwrap_or_default().to_owned());
        }
        used || link
    }

    fn describe(&self, page: &mut AccessNode, nodes: &mut Vec<(NodeId, AccessNode)>, whole: bool) {
        let told = &mut self.told.borrow_mut();
        describe::page(self.editor.doc(), self.editor.selection(), told, whole, page, nodes);
    }

    fn access_action(&mut self, request: &ActionRequest, platform: &mut dyn Platform) {
        match describe::request(self.editor.doc(), request) {
            Some(describe::Request::Select(selection)) => {
                self.editor.select(selection);
                self.mark();
                self.reveal();
            }
            Some(describe::Request::Replace(text)) => {
                self.edit(platform, |editor, platform| editor.input(InputEvent::Insert(text), platform));
            }
            Some(describe::Request::Tick(block)) => {
                // The editor ticks the task the caret is in, and the caret is not the reader's to lose.
                let held = self.editor.selection().cloned();
                self.editor.select(Selection::caret(Pos::new(block, 0)));
                self.edit(platform, |editor, platform| editor.run(Command::ToggleChecked, platform));
                if let Some(held) = held {
                    self.editor.select(held);
                    self.mark();
                }
            }
            None => {}
        }
    }

    fn text(&mut self, event: InputEvent, platform: &mut dyn Platform) {
        // The first character typed over the landing card starts this document with it.
        let typed = matches!(event, InputEvent::Insert(_) | InputEvent::Commit(_) | InputEvent::SetComposition { .. });
        if self.landing.is_some() && typed {
            self.start();
            self.edit(platform, |editor, platform| editor.input(event, platform));
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
        self.scheme = scheme;
        self.palette = palette(scheme);
        self.layout.set_palette(self.palette);
    }

    fn reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    fn active(&mut self, active: bool) {
        self.active = active;
    }

    fn edge(&self, at: Point, window: Size) -> Option<(Cursor, Message)> {
        let edge = self.chrome.edge(at, window)?;
        Some((Cursor::Resize(edge), Message::Ask(Request::ResizeWindow(edge))))
    }

    fn animate(&mut self, now_ms: u64) -> Option<u64> {
        // The note about where the document was saved goes after its few seconds.
        let until = self.saved_in.as_mut().map(|(_, from)| *from.get_or_insert(now_ms) + SAVED_NOTE_MS);
        let note = until.filter(|until| now_ms < *until);
        if note.is_none() {
            self.saved_in = None;
        }
        let slide = self.maths.animate(now_ms);
        let fade = self.fading.as_mut().map(|from| *from.get_or_insert(now_ms)).and_then(|from| {
            let progress = now_ms.saturating_sub(from) as f32 / tokens::DURATION_FAST;
            // The last step is the page exactly as it will stay.
            self.layout.set_reveal_progress(if progress < 1.0 { motion::ease(progress) } else { 1.0 });
            if progress >= 1.0 {
                self.fading = None;
            }
            (progress < 1.0).then_some(now_ms + FRAME_MS)
        });
        [note, slide, fade].into_iter().flatten().min()
    }

    fn idle(&mut self) -> Idle {
        if self.estimated {
            // Blocks above the part in view change height as they are laid out.
            self.held(|app| app.estimated = app.layout.settle(app.viewport(), SETTLE_BYTES));
            return Idle::Unseen;
        }
        if self.spelling.done() {
            return Idle::Done;
        }
        // What the reader is looking at is checked first.
        let (doc, view) = (self.editor.doc(), self.viewport());
        let at = |y: f32| self.layout.hit(Point { x: 0.0, y }).map(|pos| pos.block);
        let mut in_view = Vec::new();
        if let (Some(first), Some(last)) = (at(view.y.max(0.0)), at(view.y + view.height)) {
            in_view.extend(doc.walk().map(|block| block.id.clone()).skip_while(|id| *id != first));
            let shown = in_view.iter().position(|id| *id == last).map_or(in_view.len(), |at| at + 1);
            in_view.truncate(shown);
        }
        let caret = self.editor.selection().filter(|selection| selection.is_caret()).map(|caret| &caret.head);
        match self.spelling.idle(doc, caret, &in_view, &mut self.layout) {
            true => Idle::Seen,
            false => Idle::Unseen,
        }
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
pub fn palette(scheme: Scheme) -> Palette {
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
pub(crate) fn hex_color(hex: &str) -> Option<Color> {
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
pub(crate) const FAMILIES: [(&str, Family); 3] = [
    (letters_fonts::SANS, Family::Sans),
    (letters_fonts::DISPLAY, Family::Display),
    (letters_fonts::MONO, Family::Mono),
];

/// The weights offered for a type. The bundled faces are variable, so every family has them all.
pub(crate) const WEIGHTS: [(u16, &str); 5] =
    [(300, "Light"), (400, "Regular"), (500, "Medium"), (600, "Semibold"), (700, "Bold")];
