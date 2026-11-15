//! The editor's window: one top bar, the page, and one side panel at a time.
//!
//! [`Window`] is what a shell shows and the test harness drives. The page is a canvas holding what the
//! document's layout painted for the part in view. Everything around it is made of the toolkit's widgets.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use accesskit::{ActionRequest, Node as AccessNode, NodeId, Role};
use letters_editor::{
    Clip, Command, Editor, InputEvent, Key, KeyEvent, Keymap, Lines, Modifiers, Motion, Picture, Platform, Seek, Spot,
    TableEdit,
};
use letters_layout::{CAPTION_INSET, Callout, DocLayout, Fonts, Palette, Side as Version, Split, Syntax, Uncovered};
use letters_model::{
    Align as TextAlign, Block, BlockId, BlockKind, Body, Change, Document, Mark, Marker, Note, Notes, Pos, Revision,
    Selection, Style,
};
use letters_render::image::{Image, Images};
use letters_render::{Color, DisplayList, Font, Item, Point, Rect, Size};
use letters_spell::Dictionary;

use crate::callouts::{self, Calling, Callouts};
use crate::captions::{self, Altering, Captions};
use crate::chrome::Chrome;
use crate::code_copy::{self, CodeCopy};
use crate::color::{fade, resolve, same};
use crate::commands::{self, Act, Commanding, Commands, Entry};
use crate::context::{self, About, Context, Contexting};
use crate::copying::{self, Copy, CopyFormat, Copying};
use crate::describe;
use crate::element::{Cursor, Element, Fitted, Paint, Pointer};
use crate::figures::{self, Grip, Moving, Resizing};
use crate::finding::{self, Find, Finding};
use crate::highlight;
use crate::history::{self, History, Historying, Naming};
use crate::icon::{self, Icon};
use crate::importing::{Import, ImportOffer, Importing};
use crate::insert::{Insert, Inserting, Put};
use crate::maths::{self, Maths, Sheet, SheetLines, Source};
use crate::motion::{self, FRAME_MS};
use crate::notes::{self, Margin, Noting};
use crate::opening::{self, Landing, Location, Recent, Request};
use crate::paint::scrollbar;
use crate::pictures::{self, Linked, Linking, PictureState};
use crate::print::{Breaking, Paged, Print, Printing};
use crate::refer::{Picking, Refer};
use crate::references::{self, Preview, Reference, References};
use crate::renaming::{Outcome, Rename, Renaming};
use crate::restyle::{self, Field, StyleEdit};
use crate::review::{self, Review, Reviewing, Shown};
use crate::safe::{self, Safe, SafeMode, Safely};
use crate::settings::{Dialog, Setting, Settings};
use crate::spelling::Spelling;
use crate::style::{Align, Dir, Edges, Justify, Look, Ring};
use crate::styles::{self, Styles, Styling};
use crate::tables::{self, Tables};
use crate::text::{Family, FontShaper, TextShaper, TextStyle};
use crate::theme::{Accent, accent};
use crate::threads::{self, Said};
use crate::tokens::{self, Color as Ink, Scheme};
use crate::ui::{App, DRAG_SLOP, Idle, Keys, Ui};
use crate::widgets::{
    Button, ControlSize, Menu, MenuItem, Popover, Side, SidePanel, TextField, Title, TopBar, banner, side_panel,
    top_bar,
};

/// Space kept beside the text column.
pub(crate) const GUTTER: f32 = 24.0;
/// How wide the link editor is: its field and the button that opens the address in it.
const LINK_WIDTH: f32 = 336.0;
/// Space kept above and below the text.
pub(crate) const TOP: f32 = 48.0;
/// Points to logical pixels.
pub(crate) const PT: f32 = 96.0 / 72.0;
pub(crate) const CARET_WIDTH: f32 = 1.5;
/// How far outside a selected picture its ring is drawn.
const RING: f32 = 3.0;
/// The bar that says where pictures held over the page would land: how thick it is, and how far it stands
/// from what they would land beside.
const BAR: (f32, f32) = (3.0, 3.0);
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

/// What the top bar says about saving. A document is saved as it is written, so there is no save command:
/// the title is the file's name. Only a shell that keeps the document when asked reports
/// [`SaveState::Edited`], which is when the window asks.
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
    /// The document was written on since it was last kept, by a shell that keeps it only when asked, as a
    /// browser does without a server. The bar's mark is then a button that asks with [`Request::Save`],
    /// and so do the palette and Ctrl or Cmd with S.
    Edited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Styles,
    Notes,
}

impl Panel {
    const ALL: [(Panel, Icon); 2] = [(Panel::Styles, icon::PALETTE), (Panel::Notes, icon::MESSAGE_SQUARE)];

    fn title(self) -> &'static str {
        match self {
            Panel::Styles => "Styles",
            Panel::Notes => "Notes",
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
    /// What the menu about what is under the pointer asks for.
    Context(Contexting),
    /// Goes to the next misspelled word and offers its corrections, or to the one before.
    Misspelling {
        back: bool,
    },
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
    /// Chooses what an imported file brings, on the import card.
    Import(Import),
    /// What is done on the card that saves a copy of the document, or writes it in another format.
    Copy(Copying),
    /// What the Styles panel asks for: the person's default, page setup and stylesheets.
    Styles(Styling),
    /// Asks the shell for something only it can do, such as moving the window.
    Ask(Request),
    /// What is done on the sheet under a formula.
    Maths(Sheet),
    Insert(Inserting),
    /// What is asked of the list of figures under a reference being typed.
    Refer(Picking),
    /// What the settings dialog asks for.
    Settings(Setting),
    /// What the menu on a linked picture's cloud asks for.
    Linked(Linking),
    /// Puts the menu of the caret's column or row away.
    TableMenu,
    /// What is done on the card of a callout.
    Callout(Calling),
    /// What the field of a picture's alt text asks for.
    Alt(Altering),
    /// What the tools over a selection and the field of a comment ask for.
    Note(Noting),
    /// What the safe mode bar, a link and the card about a link ask for.
    Safe(Safely),
    /// What is asked of the command palette.
    Commands(Commanding),
    /// What is asked of the bar that finds and replaces.
    Find(Finding),
    /// What is asked of Print view.
    Print(Printing),
    /// What is asked of the history.
    History(Historying),
    /// What is asked about the conflicts that wait, and about the one that is open.
    Review(Reviewing),
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

    /// Says how far the person's clock is ahead of UTC, in minutes: the history tells its days and times
    /// by it, and by UTC until a shell says. One figure serves every date, so a time from the other side
    /// of a change to summer time reads an hour off.
    pub fn set_utc_offset(&mut self, minutes: i32) {
        self.app_mut().zone = minutes;
    }

    /// Names who writes in this window. Edits made from here on carry the name.
    pub fn set_author(&mut self, name: &str) {
        self.app_mut().margin.author = name.to_owned();
        self.app_mut().editor.set_author(name);
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

    /// Says that the copy a [`Request::SaveCopy`] asked for is written, and names its folder as the bar is
    /// to say it for a few seconds.
    pub fn set_copied(&mut self, folder: &str) {
        let app = self.app_mut();
        let Some(asked) = app.copy_asked.take() else { return };
        let said = if asked.format() == CopyFormat::Letters { "Copy saved in" } else { "Exported to" };
        app.note = Some((format!("{said} {folder}"), None));
    }

    /// Says why a copy could not be written, as the person is to read it. The card comes back as it was
    /// and says so until something on it changes. `replaceable` is for a file of the copy's name that is in
    /// the way: the card then offers to replace it, and asks again with `replace` set.
    pub fn refuse_copy(&mut self, notice: &str, replaceable: bool) {
        let app = self.app_mut();
        if let Some(mut copy) = app.copy_asked.take() {
            copy.refused(notice, replaceable);
            app.copying = Some(copy);
        }
    }

    /// Shows the landing card over a document nobody has written in, or takes it away. The card goes by
    /// itself when the person starts to write.
    pub fn set_landing(&mut self, landing: Option<Landing>) {
        let app = self.app_mut();
        app.landing = landing;
        app.uncover();
    }

    /// Whether the landing card is showing.
    pub fn landing(&self) -> bool {
        self.app().landing.is_some()
    }

    /// Shows the import card for a file that has to be imported, in place of the landing card, or takes it
    /// away. The card asks with [`Request::Import`] and [`Request::CancelImport`].
    pub fn offer_import(&mut self, offer: Option<ImportOffer>) {
        let app = self.app_mut();
        app.importing = offer.map(Importing::new);
        app.uncover();
    }

    /// Whether the import card is showing.
    pub fn importing(&self) -> bool {
        self.app().importing.is_some()
    }

    /// Has the bar say for a few seconds which folder the document was written to, after its first write.
    pub fn set_saved_in(&mut self, folder: &str) {
        self.app_mut().note = Some((format!("Saved in {folder}"), None));
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

    /// Says what the person has set for themselves, at the start and whenever another window changed it. The
    /// window takes the accent from it. Light or dark stays with [`Window::set_dark`], and the name edits
    /// carry with [`Window::set_author`], since only the shell knows what the system and the account say.
    pub fn set_settings(&mut self, settings: Settings) {
        let app = self.app_mut();
        app.settings = settings;
        app.tint();
    }

    pub fn settings(&self) -> &Settings {
        &self.app().settings
    }

    /// Names the languages the shell has dictionaries for, as BCP 47 tags, for the settings to offer beside
    /// the kinds of English that are built in.
    pub fn set_languages(&mut self, languages: impl IntoIterator<Item = impl Into<String>>) {
        self.app_mut().dialog.languages = languages.into_iter().map(Into::into).collect();
    }

    /// Says what the person's account calls them: the name their edits carry while they have set none,
    /// which the settings show in the empty name field.
    pub fn set_account_name(&mut self, name: Option<&str>) {
        self.app_mut().dialog.account = name.map(str::to_owned);
    }

    /// Says what the desktop expects of the window's frame and bar: see [`Chrome`]. Call it at the start
    /// and whenever the window's state changes. A bar that is the title bar asks for the window to be
    /// moved, resized, minimised, maximised and closed through [`Request`].
    pub fn set_chrome(&mut self, chrome: Chrome) {
        self.app_mut().chrome = chrome;
    }

    /// What the window asks of its shell, since this was last called. Call it after every event.
    pub fn take_requests(&mut self) -> Vec<Request> {
        // Reaching for the app's state asks for a frame, which a call after every event must not do.
        if self.app().requests.is_empty() {
            return Vec::new();
        }
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

    /// Adds faces that arrived after the window opened, for a shell whose fonts come one after another: pass
    /// `letters_fonts::bundled()` again once it holds more. Text that was set without a face is set again
    /// with it, and the next frame shows it. Faces the window already has change nothing.
    pub fn add_fonts(&mut self, faces: Vec<Font>) {
        let fonts = Fonts { faces };
        let app = self.app_mut();
        let labels = app.shaper.add_fonts(&fonts);
        app.said.again();
        app.review.add_fonts(&fonts);
        if let Some(history) = &mut app.history {
            history.add_fonts(&fonts);
        }
        if app.held(|app| app.layout.add_fonts(&fonts)) {
            // Blocks set again keep their heights as estimates until idle time settles them.
            app.estimated = true;
        }
        if labels {
            self.reshape();
        }
    }

    /// Adds faces of the system that only back the bundled fonts up, for the characters those lack: emoji,
    /// and the scripts no bundled face writes. Text is set in one only where no bundled face has its
    /// characters, so nothing else moves, and no style gets one by naming its family. Which faces to look
    /// for, [`Self::take_uncovered`] says. They never go through [`Self::add_fonts`].
    ///
    /// The faces are kept for whatever is laid out later. Returns whether text that was shown without a
    /// glyph has one now, which the next frame shows.
    pub fn add_fallback_fonts(&mut self, faces: Vec<Font>) -> bool {
        let fonts = Fonts { faces };
        let app = self.app_mut();
        app.backup.faces.extend(fonts.faces.iter().cloned());
        let labels = app.shaper.add_fallback_fonts(&fonts);
        let mut shown = app.said.add_fallback_fonts(&fonts);
        shown |= app.review.add_fallback_fonts(&fonts);
        if let Some(history) = &mut app.history {
            shown |= history.add_fallback_fonts(&fonts);
        }
        if app.held(|app| app.layout.add_fallback_fonts(&fonts)) {
            // Blocks set again keep their heights as estimates until idle time settles them.
            (app.estimated, shown) = (true, true);
        }
        if labels {
            self.reshape();
        }
        shown || labels
    }

    /// The characters the window showed without a glyph since this was last asked, since no face it has
    /// writes them: on the page, in an earlier version, in a note or in a label. Each is told once for as
    /// long as the window lives, as it was first met: with whether it is written as an emoji and the
    /// language of its text, which say what face to look for. Hand what is found to
    /// [`Self::add_fallback_fonts`].
    ///
    /// Characters are met as text is laid out, so ask after a frame, and once every face of
    /// [`Self::add_fonts`] has come. It costs nothing when there are none, and asks for no frame.
    pub fn take_uncovered(&mut self) -> Vec<Uncovered> {
        let app = self.app_unseen();
        let mut found = app.shaper.take_uncovered();
        found.extend(app.layout.take_uncovered());
        found.extend(app.said.take_uncovered());
        found.extend(app.review.take_uncovered());
        found.extend(app.history.iter_mut().flat_map(History::take_uncovered));
        found.retain(|found| app.uncovered.insert(found.ch));
        found
    }

    /// Lets formulas link, as `\\href` asks. The shell turns it on for a document the person trusts, and it
    /// is off until then.
    pub fn set_maths_links(&mut self, links: bool) {
        let app = self.app_mut();
        app.held(|app| app.layout.set_maths_links(links));
        // Formulas are set again, at heights that are estimates until idle time settles them.
        app.estimated = true;
    }

    /// Says that the document is held in safe mode, and how it can be trusted, or `None` once it is. A bar
    /// under the top bar tells the person and asks with [`Request::TrustDocument`] and
    /// [`Request::TrustFolder`]. What safe mode holds back stays the shell's to hold back. When a document
    /// becomes trusted the window asks for every linked picture not yet showing with
    /// [`Request::LinkedPicture`] again.
    pub fn set_safe_mode(&mut self, safe: Option<SafeMode>) {
        if self.app_mut().safe.set(safe) {
            self.app_mut().ask_again(true);
        }
    }

    /// Shows the whole address of a link and asks whether to open it, in answer to a
    /// [`Request::OpenLink`] that was not confirmed. A yes asks again with `confirmed` set.
    pub fn ask_link(&mut self, href: &str) {
        self.app_mut().safe.ask(href);
    }

    /// Says that a link is not opened and why, in a few words, in answer to a [`Request::OpenLink`].
    pub fn refuse_link(&mut self, href: &str, why: &str) {
        self.app_mut().safe.refuse(href, why);
    }

    /// The document's notes: the comments on its passages, and what imports and merges left on them. A
    /// shell writes them into the document's file with each save. A comment that is written counts among
    /// [`Window::edits`].
    pub fn notes(&self) -> &Notes {
        &self.app().margin.notes
    }

    /// Says that no comment can be written on this document and why, in a sentence that stands by
    /// itself, or `None` once one can. A shell says it of a document whose notes it could not read, since
    /// a comment added to those could not be saved. The Comment tool then shows the sentence in place of
    /// its field.
    pub fn refuse_comments(&mut self, why: Option<&str>) {
        self.app_mut().margin.refused = why.map(str::to_owned);
    }

    /// Hands over the notes of the document that was opened, or what a merge made of them.
    pub fn set_notes(&mut self, notes: Notes) {
        let app = self.app_mut();
        app.margin.take(notes, app.editor.model());
    }

    /// The decoded pictures the window's display list draws, by asset name: what a shell hands its
    /// renderer with each frame.
    pub fn images(&self) -> &Images {
        &self.app().images
    }

    /// Hands over a picture decoded: one the window asked for with [`Request::Asset`], or one the document
    /// links to, under the address the link is written as. `None` says it cannot be had: its place stays
    /// empty and it is not asked for again, and a linked picture goes back to its frame and cloud.
    pub fn set_image(&mut self, asset: &str, image: Option<Image>) {
        let app = self.app_mut();
        app.wanted.insert(asset.to_owned());
        // A linked picture is drawn once it is here, at its own shape. For any other name this says nothing.
        app.layout.set_loaded(asset, image.as_ref().map(|image| (image.width(), image.height())));
        match image {
            Some(image) => {
                app.images.insert(asset, image);
                // A picture that shows has nothing more said of it.
                app.linked.say(asset, None);
            }
            None => _ = app.images.remove(asset),
        }
    }

    /// Says how a linked picture that does not show is doing, by the address its link is written as: it is
    /// being fetched, the fetch failed for a short reason, or safe mode holds it back. Its cloud tells the
    /// person, and offers another try at one that failed with a [`Request::LoadPicture`]. Say it again as
    /// it changes. `None` says no more of it, as handing the picture to [`Window::set_image`] does.
    pub fn set_picture_state(&mut self, src: &str, state: Option<PictureState>) {
        if self.app().linked.said(src) != state.as_ref() {
            self.app_mut().linked.say(src, state);
        }
    }

    /// Says whether a linked picture comes from a source the person trusts already, by the address its
    /// link is written as: the menu on its cloud then does not offer "Always load from" its host again.
    /// The window takes it as said of every picture of a host once the person has chosen that from a menu.
    pub fn set_picture_trusted(&mut self, src: &str, trusted: bool) {
        if self.app().linked.vouched(src) != trusted {
            self.app_mut().linked.vouch(src, trusted);
        }
    }

    /// Keeps a linked picture in the document, in answer to a [`Request::EmbedPicture`]: the figure holds
    /// the picture from now on and no longer links to it. Nothing happens when the figure is gone or holds
    /// a picture already, or when the bytes are no picture Letters can show.
    pub fn embed_picture(&mut self, figure: &BlockId, picture: Picture, platform: &mut dyn Platform) {
        let figure = figure.clone();
        self.app_mut()
            .edit(platform, |editor, platform| editor.run(Command::EmbedFigure { figure, picture }, platform));
    }

    /// Shows where pictures held over a point of the window would land if they were let go there, as
    /// files dragged over the window are: a bar beside a picture, or between two blocks. `None` puts the
    /// bar away. Returns whether the point is on the page.
    pub fn drag_pictures(&mut self, at: Option<Point>) -> bool {
        let spot = at.and_then(|at| self.on_canvas(at)).and_then(|at| self.app().spot(at));
        let bar = spot.as_ref().and_then(|(_, bar)| *bar);
        if self.app().drop != bar {
            self.app_mut().drop = bar;
        }
        spot.is_some()
    }

    /// Puts pictures into the document: one as a figure, several as a gallery. Let go at a point of the
    /// page they land where [`Window::drag_pictures`] showed. With no point, or one that is not on the
    /// page, they land where the selection is, as pictures chosen in a dialog do (see
    /// [`Request::Pictures`]). What is no picture Letters can show is left out.
    pub fn insert_pictures(&mut self, pictures: Vec<Picture>, at: Option<Point>, platform: &mut dyn Platform) {
        // Nothing is written in Print view, nor on an earlier version.
        if self.app().away() {
            return;
        }
        let at = at.and_then(|at| self.on_canvas(at)).and_then(|at| self.app().spot(at)).map(|(spot, _)| spot);
        let app = self.app_mut();
        app.drop = None;
        app.edit(platform, |editor, platform| editor.run(Command::InsertFigures { pictures, at }, platform));
    }

    /// Where the two ends of the selection are in the window as last laid out, the anchor first and then
    /// the end that moves: each as tall as its line, for a shell that draws handles there on a touch
    /// screen. `None` for an end the page does not show, and for both without a selection.
    pub fn selection_ends(&self) -> [Option<Rect>; 2] {
        let app = self.app();
        let editing = self.canvas().filter(|_| !app.away());
        let Some((page, selection)) = editing.zip(app.editor.selection()) else { return [None, None] };
        let end = |pos: &Pos| {
            let caret = app.layout.caret(pos)?;
            let at = app.onto(caret);
            let (x, y) = (page.x + at.x, page.y + at.y);
            let shown = y + caret.height > page.y && y < page.y + page.height;
            shown.then_some(Rect { x, y, ..caret })
        };
        [end(&selection.anchor), end(&selection.head)]
    }

    /// Selects the word at a point of the window, as a double click there does: for a long press on a
    /// touch screen. Returns whether the page had text to select there. A point on anything that lies over
    /// the page selects nothing.
    pub fn select_word_at(&mut self, at: Point, platform: &mut dyn Platform) -> bool {
        // The version a conflict holds back is read, not selected.
        let on_page = self.on_canvas(at).filter(|at| !self.app().review.covers(*at));
        let Some(head) = on_page.and_then(|at| self.app().hit(at)) else { return false };
        let app = self.app_mut();
        app.editor.select(Selection::caret(head));
        app.edit(platform, |editor, platform| editor.run(Command::SelectWord, platform));
        true
    }

    /// The document and its style as they read at a revision of the history, for the shell that writes
    /// the copy a [`Request::SaveVersion`] asks for. `None` for a revision the history does not hold.
    pub fn document_at(&mut self, revision: &Revision) -> Option<(Document, Style)> {
        self.app_mut().editor.document_at(revision).ok()
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

    fn wall_time(&self) -> Option<u64> {
        self.platform.wall_time()
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
    /// The person's accent, worked out from their settings.
    accent: Accent,
    settings: Settings,
    dialog: Dialog,
    /// The text column's left edge inside the page area, and its width.
    column: (f32, f32),
    /// The width and the height of the page area.
    across: f32,
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
    /// The file the import card offers, while it shows.
    importing: Option<Importing>,
    requests: Vec<Request>,
    /// The pictures the shell has handed over, by asset name.
    images: Images,
    /// The assets the shell was asked for, or told of by itself.
    wanted: HashSet<String>,
    /// The bar that says where pictures held over the page would land, in the layout's coordinates.
    drop: Option<Rect>,
    linked: Linked,
    safe: Safe,
    grip: Grip,
    captions: Captions,
    margin: Margin,
    /// The conflict whose two versions show in the page.
    review: Review,
    /// What the note that is open says, set to be drawn.
    said: Said,
    /// What the bar says of a write that was just done, as where the document was first saved, and since
    /// when, once a frame has said so.
    note: Option<(String, Option<u64>)>,
    /// Whether the caret is in the heading that will name the document.
    naming: bool,
    /// The name field in the bar, while the document is being renamed or moved.
    renaming: Option<Renaming>,
    /// What the field held when the shell was last asked for it, to bring back if the shell refuses.
    asked: Option<Renaming>,
    /// The card that saves a copy, while it shows.
    copying: Option<Copy>,
    /// The card as it was when the shell was asked for the copy, to bring back if the shell refuses.
    copy_asked: Option<Copy>,
    /// Whether keys follow macOS conventions, which is how the menus then write them.
    mac: bool,
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
    insert: Insert,
    commands: Commands,
    find: Find,
    /// Print view, while it shows in place of the page.
    printing: Option<Print>,
    /// The history, while one of the document's versions shows in place of the page.
    history: Option<History>,
    /// How far the person's clock is ahead of UTC, in minutes.
    zone: i32,
    refer: Refer,
    references: References,
    code_copy: CodeCopy,
    tables: Tables,
    /// The callouts' icons and titles under the pointer, and the card of one.
    callouts: Callouts,
    /// The menu about what is under the pointer, while it is open.
    context: Option<Context>,
    /// How many of those menus were opened.
    menus: u32,
    /// The preview of a figure that shows over the page: its caption, and where it is in the page area.
    previewed: Option<(String, Rect)>,
    /// How the document read when assistive technology was last told of it.
    told: RefCell<describe::Told>,
    /// The faces of the system that back the bundled fonts up, for what is laid out after they came.
    backup: Fonts,
    /// The characters without a glyph that the shell was told of. None is told twice.
    uncovered: HashSet<char>,
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
            accent: Accent::YEWI,
            settings: Settings::default(),
            dialog: Dialog::default(),
            column: (0.0, 0.0),
            across: 0.0,
            height: 0.0,
            scroll: 0.0,
            scale: 1.0,
            estimated: true,
            location: Location::default(),
            landing: None,
            importing: None,
            requests: Vec::new(),
            images: Images::default(),
            wanted: HashSet::new(),
            drop: None,
            linked: Linked::default(),
            safe: Safe::default(),
            grip: Grip::default(),
            captions: Captions::default(),
            margin: Margin::default(),
            review: Review::default(),
            said: Said::default(),
            note: None,
            naming: false,
            renaming: None,
            asked: None,
            copying: None,
            copy_asked: None,
            mac: cfg!(target_os = "macos"),
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
            insert: Insert::default(),
            commands: Commands::default(),
            find: Find::default(),
            printing: None,
            history: None,
            zone: 0,
            refer: Refer::default(),
            references: References::default(),
            code_copy: CodeCopy::default(),
            tables: Tables::default(),
            callouts: Callouts::default(),
            context: None,
            menus: 0,
            previewed: None,
            told: RefCell::default(),
            backup: Fonts::default(),
            uncovered: HashSet::new(),
        }
    }

    /// Takes the page's colours for the scheme and the accent the person set. Of the page, only what is
    /// selected shows in the accent: the rest is the document's to colour, and looks the same to everyone.
    fn tint(&mut self) {
        self.accent = self.settings.accent.map_or(Accent::YEWI, Accent::of);
        self.palette = palette(self.scheme);
        if self.accent != Accent::YEWI {
            self.palette.selection = resolve(fade(self.accent.base, 0.3), self.scheme, 1.0);
        }
        self.layout.set_palette(self.palette);
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
        if let Some(copy) = &mut self.copying {
            copy.placed(&self.location);
        }
    }

    /// What the card that saves a copy asks for.
    fn copy(&mut self, message: Copying) {
        let title = self.name().0;
        let Some(mut copy) = self.copying.take() else {
            // The landing card stands over a document there is nothing of yet, and so does the import card.
            if let Copying::Open(format) = message
                && self.landing.is_none()
                && self.importing.is_none()
            {
                // While the history shows an earlier version, the copy is of that.
                let version = self.history.as_ref().and_then(History::copied);
                self.copying = Some(Copy::open(format, &self.location, &title, version));
            }
            return;
        };
        match copy.update(message, &self.location, &title, &mut self.requests) {
            copying::Outcome::Open => self.copying = Some(copy),
            copying::Outcome::Asked => self.copy_asked = Some(copy),
            copying::Outcome::Closed => {}
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

    /// Tells the shell of the pictures the document links to, each address once: all of them, or those of
    /// the blocks that changed.
    fn tell_links(&mut self, changed: Option<&BTreeSet<BlockId>>) {
        let doc = self.editor.doc();
        let blocks: Vec<&Block> = match changed {
            Some(changed) => changed.iter().filter_map(|id| doc.block(id)).collect(),
            None => doc.walk().collect(),
        };
        for src in blocks.into_iter().filter_map(pictures::link) {
            if self.wanted.insert(src.to_owned()) {
                self.requests.push(Request::LinkedPicture(src.to_owned()));
            }
        }
    }

    /// The address a figure links to, while it does not hold its picture.
    fn link_of(&self, figure: &BlockId) -> Option<&str> {
        self.editor.doc().block(figure).and_then(pictures::link)
    }

    /// The linked picture at a point of the page area, and whether the point is on its cloud.
    fn linked_at(&self, at: Point) -> Option<(BlockId, bool)> {
        let figure = self.hit(at).filter(|_| !self.hung())?.block;
        self.link_of(&figure)?;
        let at = self.in_layout(at);
        let place = self.layout.figure(&figure).filter(|place| place.image.contains(at))?;
        Some((figure, place.badge.is_some_and(|badge| pictures::pressed(badge, at))))
    }

    /// The page as the conflict that is open splits it, counted from the top of what scrolls. `None`
    /// while the page shows whole.
    fn cut(&self) -> Option<Split> {
        self.review.split(&self.layout, self.across)
    }

    /// Where the page area shows the top left of what a split counts from. A split stands the page's
    /// column in the very middle, and the page area on a whole pixel beside that.
    fn shift(&self) -> Point {
        let middle = ((self.across - self.column.1) / 2.0).max(0.0);
        Point { x: self.column.0 - middle, y: TOP - self.scrolled() }
    }

    /// Where the page area shows the top left of the layout's column, for what is `y` down the column.
    /// It is one place for the whole page, until a conflict that is open splits it.
    fn origin(&self, y: f32) -> Point {
        match self.cut() {
            None => Point { x: self.column.0, y: TOP - self.scrolled() },
            Some(cut) => {
                let (part, shift) = (review::origin(&cut, y), self.shift());
                Point { x: part.x + shift.x, y: part.y + shift.y }
            }
        }
    }

    /// Something of the layout where the page area shows it.
    fn onto(&self, rect: Rect) -> Rect {
        let origin = self.origin(rect.y);
        Rect { x: rect.x + origin.x, y: rect.y + origin.y, ..rect }
    }

    /// A point of the page area as the layout counts it.
    fn in_layout(&self, at: Point) -> Point {
        match self.cut() {
            None => Point { x: at.x - self.column.0, y: at.y + self.scrolled() - TOP },
            Some(cut) => {
                let shift = self.shift();
                review::within(&cut, Point { x: at.x - shift.x, y: at.y - shift.y })
            }
        }
    }

    /// How far down what scrolls something `y` down the layout's column shows.
    fn down(&self, y: f32) -> f32 {
        self.cut().map_or(y, |cut| y + review::origin(&cut, y).y)
    }

    /// The height of what scrolls, without the room over and under it.
    fn tall(&self) -> f32 {
        self.cut().map_or_else(|| self.layout.height(), |cut| cut.height)
    }

    /// Brings what is kept of the open conflict up to date with the notes, the document and the page.
    fn sync(&mut self) {
        // Before the page was first shown there is no width to lay a version out at.
        let open = self.margin.reviewing().filter(|_| self.column.1 > 0.0);
        let made = (self.column.1, self.editor.model().style(), self.palette);
        self.review.follow(open, self.editor.doc(), self.editor.selection(), made);
    }

    /// Does what is asked about the conflicts: steps through them, puts the open one away, has its two
    /// versions trade places, or settles it with the one on the page. The page keeps its place while the
    /// block's other version comes or goes.
    fn reviewing(&mut self, asked: Reviewing, platform: &mut dyn Platform) {
        match asked {
            Reviewing::Step { back } => {
                let Some(next) = self.margin.step(back) else { return };
                return self.noting(Noting::Open(next), platform);
            }
            Reviewing::Close => self.held(|app| {
                app.margin.shut();
                app.sync();
            }),
            Reviewing::Keep => self.held(|app| {
                // A choice is kept with the document, as a note that is resolved is.
                app.edits += u64::from(app.margin.settle());
                app.sync();
            }),
            Reviewing::Use => {
                let held = self.review.held_back().filter(|_| self.margin.refused.is_none());
                let Some((block, version)) = held else { return };
                // A version that took the block out leaves nothing to write on: using it settles.
                let settles = version.is_none();
                let command = Command::UseVersion { block, version };
                if !self.editor.applies(&command) {
                    return;
                }
                self.held(|app| {
                    if settles {
                        app.edits += u64::from(app.margin.settle());
                    }
                    // The review takes what the page read as the version held back, once the page reads
                    // as the one that was.
                    let change = app.run(platform, |editor, platform| editor.run(command, platform));
                    app.follow(&change);
                    app.sync();
                });
            }
        }
        self.margin.remark(self.editor.doc());
        self.sync();
    }

    /// Brings the two versions of the conflict that is open into view: a quarter of the way down the page
    /// area, unless they show whole where they are.
    fn show_review(&mut self) {
        // Twice, since laying out what comes into view can move the block.
        for _ in 0..2 {
            let Some((top, bottom)) = self.cut().as_ref().and_then(review::pair) else { return };
            let from = self.scroll - TOP;
            if top >= from && bottom <= from + self.height {
                break;
            }
            self.scroll = (top + TOP - self.height / 4.0).clamp(0.0, self.max_scroll());
            self.layout.settle(self.viewport(), 0);
        }
    }

    /// The gallery a picture is one of.
    fn gallery_of(&self, figure: &BlockId) -> Option<&Block> {
        let holds = |block: &&Block| {
            let gallery = matches!(block.kind, BlockKind::Gallery { .. });
            gallery && block.body.blocks().iter().any(|part| part.id == *figure)
        };
        self.editor.doc().walk().find(holds)
    }

    /// The selected picture and its place, when it stands alone. One of a gallery takes its size from its
    /// row, so it has no corners to be resized by.
    fn lone_picture(&self) -> Option<(BlockId, Rect)> {
        let figure = self.editor.picture().filter(|figure| self.gallery_of(figure).is_none())?;
        Some((figure.clone(), self.layout.figure(figure)?.image))
    }

    /// The figure the line under a picture is about: the one whose picture is selected, or one with the
    /// selection in its caption.
    fn captioned(&self) -> Option<&BlockId> {
        let selection = self.editor.selection()?;
        let within = selection.anchor.block == selection.head.block;
        let figure = self.editor.doc().block(&selection.head.block).filter(|block| within && is_figure(block))?;
        Some(&figure.id)
    }

    /// The line under a figure's picture, and the block that holds what is written there: the figure
    /// itself, or the caption of its gallery once it has one.
    fn caption_of(&self, figure: &BlockId) -> Option<(Rect, Option<&Block>)> {
        let line = self.layout.figure(figure)?.caption;
        let holder = match self.gallery_of(figure) {
            Some(gallery) => gallery.body.blocks().iter().find(|part| part.kind == BlockKind::Caption),
            None => self.editor.doc().block(figure),
        };
        Some((line, holder))
    }

    /// The size captions are written at and how they are aligned, as the document's style has them.
    fn caption_text(&self) -> (f32, TextAlign) {
        let style = self.editor.model().style().types.get("caption");
        style.map_or((tokens::TEXT_SM, TextAlign::Start), |style| (style.size * PT, style.align))
    }

    /// The line under the selected picture while nothing is written or numbered there, and where on it
    /// the hint to write a caption stands.
    fn hint(&self) -> Option<(Rect, Rect)> {
        let figure = self.editor.picture()?;
        let (line, holder) = self.caption_of(figure)?;
        let numbers = self.gallery_of(figure).or_else(|| self.editor.doc().block(figure));
        let numbered = numbers.is_some_and(|block| {
            matches!(block.kind, BlockKind::Figure { numbered: true, .. } | BlockKind::Gallery { numbered: true })
        });
        if numbered || holder.is_some_and(|block| !block.plain_text().is_empty()) {
            return None;
        }
        // The caret of an empty caption is where its first letter goes.
        let first = holder.and_then(|block| self.layout.caret(&Pos::new(block.id.clone(), 0)));
        let (size, align) = self.caption_text();
        Some((line, captions::hint_at(line, first, CAPTION_INSET, size, align, self.shaper.as_ref())))
    }

    /// The figure whose alt text the switch under it turns to, the line under its picture and where the
    /// switch stands at the end of that line.
    fn alt_switch(&self) -> Option<(&BlockId, Rect, Rect)> {
        let figure = self.captioned()?;
        let (line, holder) = self.caption_of(figure)?;
        let first = holder.and_then(|block| self.layout.caret(&Pos::new(block.id.clone(), 0)));
        let first = first.map_or((line.y, line.height), |caret| (caret.y, caret.height));
        Some((figure, line, captions::switch_at(line, first, self.shaper.as_ref())))
    }

    /// Turns the line under a picture to its alt text, and puts away what else hangs from the page.
    fn alter(&mut self, figure: BlockId) {
        let Some(BlockKind::Figure { alt, .. }) = self.editor.doc().block(&figure).map(|block| &block.kind) else {
            return;
        };
        self.captions.alt = Some((figure, alt.clone()));
        (self.link, self.spelling.fix, self.linked.open) = (None, None, None);
    }

    /// Does what the tools over a selection, the field of a comment, a marker in the margin, a note that
    /// is open or the panel of them all asked.
    fn noting(&mut self, asked: Noting, platform: &mut dyn Platform) {
        // Notes say when they were written against the time one of them was last asked about.
        self.margin.now = platform.wall_time().map(notes::stamp).unwrap_or_default();
        let opens = matches!(asked, Noting::Open(_) | Noting::Press(_));
        match asked {
            // What is typed changes nothing but itself.
            Noting::Draft(text) => return self.margin.typed(text),
            Noting::Reply(text) => return self.margin.reply(text),
            Noting::Away => self.margin.put_away(self.editor.selection()),
            Noting::Write => {
                self.margin.write(self.editor.model(), self.editor.selection());
                (self.link, self.spelling.fix, self.linked.open, self.captions.alt) = (None, None, None, None);
                self.context = None;
                self.context = None;
            }
            Noting::Drop => self.margin.drop_draft(),
            Noting::Send => {
                let created = notes::stamp(platform.wall_time().unwrap_or(0));
                if self.margin.send(self.editor.model(), platform.new_id(), created) {
                    // A comment is kept with the document, so it is something to save.
                    self.edits += 1;
                }
            }
            Noting::Open(id) => {
                self.margin.read(&id);
                self.show_note();
            }
            Noting::Press(block) => self.margin.press(&block),
            Noting::Close => self.margin.close(),
            Noting::Show(kind) => self.margin.filter = kind,
            // A reply, and a note that is resolved, are kept with the document as a comment is.
            Noting::Answer => {
                let created = notes::stamp(platform.wall_time().unwrap_or(0));
                self.edits += u64::from(self.margin.answer(created));
            }
            Noting::Resolve => self.edits += u64::from(self.margin.resolve()),
            Noting::ResolveImports => self.edits += u64::from(self.margin.resolve_imports()),
        }
        if opens && self.margin.reading().is_some() {
            (self.link, self.spelling.fix, self.linked.open, self.captions.alt) = (None, None, None, None);
            self.context = None;
        }
        // Once a comment is kept or dropped, the tools wait for another selection.
        if self.margin.writing().is_none() {
            self.margin.put_away(self.editor.selection());
        }
        self.margin.remark(self.editor.doc());
        // A conflict is not read under its passage: the page shows both of its versions.
        let reviewed = self.review.note().map(str::to_owned);
        self.held(Self::sync);
        if opens && self.review.note().is_some_and(|open| Some(open) != reviewed.as_deref()) {
            self.show_review();
        }
    }

    /// The first and the last line of the passage a note is about, in the layout's coordinates: of its
    /// stretch of text, or the first line of the block it is on as a whole. `None` once its block is gone.
    fn note_lines(&self, note: &Note) -> Option<(Rect, Rect)> {
        if let Some(passage) = notes::stretch(&note.anchor) {
            let lines = self.layout.selection(&passage);
            return Some((*lines.first()?, *lines.last()?));
        }
        let block = self.editor.doc().block(&note.anchor.block)?;
        if let Some(figure) = self.layout.figure(&block.id) {
            return Some((figure.image, figure.image));
        }
        let mut texts = Vec::new();
        written(block, &mut texts);
        let start = Pos::new(texts.first()?.id.clone(), 0);
        let from = self.layout.caret(&start)?;
        let to = self.layout.line_edge(&start, true).and_then(|end| self.layout.caret(&end));
        let line = Rect { width: to.map_or(0.0, |to| (to.x - from.x).max(0.0)), ..from };
        Some((line, line))
    }

    /// Brings the passage of the note that is open into view, as opening it from the panel asks. One that
    /// shows whole stays where it is.
    fn show_note(&mut self) {
        // Twice, since laying out what comes into view can move the passage.
        for _ in 0..2 {
            let lines = self.margin.reading().and_then(|(note, _)| self.note_lines(note));
            let Some((first, last)) = lines else { return };
            let (top, bottom, view) = (first.y, last.y + last.height, self.viewport());
            if top >= view.y && bottom <= view.y + view.height {
                break;
            }
            // A quarter of the way down, the note has the rest of the page to hang in.
            self.scroll = (self.down(top) + TOP - self.height / 4.0).clamp(0.0, self.max_scroll());
            self.layout.settle(self.viewport(), 0);
        }
    }

    /// The first line of a block that has a marker, in the layout's coordinates, while the block's start
    /// is in view.
    fn marked_line(&self, marker: &threads::Marker, view: Rect) -> Option<Rect> {
        let line = match (self.layout.figure(&marker.block), &marker.first) {
            (Some(figure), _) => Rect { height: 0.0, ..figure.image },
            (None, Some((text, long))) => {
                // Only text that shows is asked where it is: asking lays it out, and text laid out above
                // the view moves the page.
                let (start, end) = (Pos::new(text.clone(), 0), Pos::new(text.clone(), *long));
                let all = Selection { anchor: start.clone(), head: end };
                if *long > 0 && self.layout.selection_in(&all, view).is_empty() {
                    return None;
                }
                self.layout.caret(&start)?
            }
            (None, None) => return None,
        };
        (line.y + line.height >= view.y && line.y <= view.y + view.height).then_some(line)
    }

    /// The markers in view, each with where it stands in the layout's coordinates.
    fn markers(&self) -> Vec<(&threads::Marker, Rect)> {
        let view = self.viewport();
        let (column, room) = (self.column.1, self.column.0);
        let placed = |marker| {
            let line = self.marked_line(marker, view)?;
            Some((marker, threads::place(marker, line, column, room, self.shaper.as_ref())))
        };
        // The block of the conflict that is open has the two versions where its marker would stand.
        let open = self.review.note();
        let stands = |marker: &&threads::Marker| !open.is_some_and(|open| marker.notes.iter().any(|note| note == open));
        self.margin.marks().iter().filter(stands).filter_map(placed).collect()
    }

    /// The block whose marker is at a point of the layout.
    fn marker_at(&self, spot: Point) -> Option<BlockId> {
        let on = self.markers().into_iter().find(|(_, at)| threads::pressed(*at, spot));
        on.map(|(marker, _)| marker.block.clone())
    }

    /// The tools over a selected stretch of text, once it is selected: over its first line, or under its
    /// last when there is no room above.
    fn selection_tools(&self, viewport: Rect) -> Option<Popover<Message>> {
        let selection = self.editor.selection().filter(|selection| !selection.is_caret())?;
        if self.dragging || !self.margin.offers() || self.commands.open() {
            return None;
        }
        // A place that was found is selected to be looked at, not to have something made of it.
        if self.find.current(Some(selection)).is_some() {
            return None;
        }
        notes::passage(self.editor.doc(), selection)?;
        let lines = self.layout.selection_in(selection, viewport);
        let (first, last) = (lines.first()?, lines.last()?);
        let above = self.onto(*first).y >= notes::TOOLS_ROOM;
        let from = if above { first } else { last };
        Some(Margin::tools(self.editor.link().is_some(), above).at(self.onto(*from)))
    }

    /// Does what the field of a picture's alt text asked.
    fn altering(&mut self, asked: Altering, platform: &mut dyn Platform) {
        match asked {
            Altering::Open => {
                if let Some(figure) = self.captioned().cloned() {
                    self.reveal();
                    self.alter(figure);
                }
            }
            Altering::Draft(text) => {
                if let Some((_, typed)) = &mut self.captions.alt {
                    *typed = text;
                }
            }
            Altering::Drop => self.captions.alt = None,
            Altering::Apply => {
                let Some((figure, typed)) = self.captions.alt.take() else { return };
                let typed = typed.trim().to_owned();
                let before = match self.editor.doc().block(&figure).map(|block| &block.kind) {
                    Some(BlockKind::Figure { alt, .. }) => alt,
                    _ => return,
                };
                // Only for the picture the line is still about, and only what says something else.
                if self.captioned() == Some(&figure) && *before != typed {
                    self.edit(platform, |editor, platform| editor.run(Command::SetAlt(typed), platform));
                }
            }
        }
    }

    /// Where the picture that is carried would land if it were let go at a point of the page area, and the
    /// bar that says so. Nowhere when that is where it is.
    fn carried_to(&self, at: Point) -> Option<(Spot, Option<Rect>)> {
        let moved = &self.grip.moving.as_ref().filter(|moving| moving.carried().is_some())?.figure;
        let alone = self.gallery_of(moved).is_none();
        self.spot(at).filter(|(spot, _)| match spot {
            Spot::Beside { figure, .. } => figure != moved,
            // A picture of a gallery leaves it for a line of its own.
            Spot::Line { block, .. } => !alone || block != moved,
        })
    }

    /// Draws a picture small where the pointer carries it, in the page area.
    fn carry(&self, image: Rect, at: Point, out: &mut DisplayList) {
        let (rect, by) = figures::carried(image, at);
        let color = |ink| resolve(ink, self.scheme, 1.0);
        let radius = tokens::RADIUS_SM;
        for shadow in tokens::SHADOW_OVERLAY {
            let offset = Point { x: shadow.x, y: shadow.y };
            out.push(Item::Shadow { rect, color: color(shadow.color), blur: shadow.blur, offset, radius });
        }
        // The picture is what the layout paints where the figure is, made small.
        let mut picture = DisplayList::default();
        self.layout.paint(image, &mut picture);
        pictures::awaited(&self.images, self.scheme, &mut picture);
        out.push(Item::PushClip { rect, radius });
        out.push(Item::Fill { rect, color: self.palette.page, radius: 0.0 });
        let first = out.items.len();
        out.items.extend(picture.items);
        out.translate(first, Point { x: -image.x, y: -image.y });
        out.scale(first, by);
        out.translate(first, Point { x: rect.x, y: rect.y });
        out.push(Item::PopClip);
        out.push(Item::Stroke { rect, color: color(tokens::SURFACE), width: 2.0, radius });
    }

    /// Opens the menu on a linked picture's cloud, and puts away what else hangs from the page.
    fn link_menu(&mut self, figure: BlockId) {
        (self.link, self.spelling.fix, self.captions.alt) = (None, None, None);
        self.linked.open = Some(figure);
    }

    /// Does what the menu on a linked picture's cloud asked, which is the shell's to do.
    fn linking(&mut self, asked: Linking) {
        if asked == Linking::EmbedAll {
            return self.embed_all();
        }
        // The picture whose menu is out, or the selected one when a command asks.
        let Some(figure) = self.linked.open.take().or_else(|| self.editor.picture().cloned()) else { return };
        let Some(src) = self.link_of(&figure).map(str::to_owned) else { return };
        match asked {
            Linking::Load => self.requests.push(Request::LoadPicture(src)),
            Linking::Trust => {
                self.linked.trust(&src);
                self.requests.push(Request::TrustPictures(src));
                self.ask_again(false);
            }
            Linking::Embed => self.requests.push(Request::EmbedPicture { figure, src }),
            Linking::Close | Linking::EmbedAll => {}
        }
    }

    /// Asks the shell to keep every linked picture in the document. Those that safe mode holds back stay
    /// links, since nothing is fetched for a document that is not trusted.
    fn embed_all(&mut self) {
        let free = |src: &&str| self.linked.said(src) != Some(&PictureState::Held);
        let link = |block: &Block| Some((block.id.clone(), pictures::link(block).filter(free)?.to_owned()));
        let links: Vec<(BlockId, String)> = self.editor.doc().walk().filter_map(link).collect();
        for (figure, src) in links {
            self.requests.push(Request::EmbedPicture { figure, src });
        }
    }

    /// Whether the figure the caret is on or in carries a number, or the gallery it is one of does.
    fn numbered(&self) -> Option<bool> {
        let figure = self.captioned()?;
        match self.gallery_of(figure).or_else(|| self.editor.doc().block(figure))?.kind {
            BlockKind::Figure { numbered, .. } | BlockKind::Gallery { numbered } => Some(numbered),
            _ => None,
        }
    }

    /// Whether the first row of the caret's table is its header.
    fn header(&self) -> Option<bool> {
        match self.editor.doc().block(&self.editor.cell()?.table)?.kind {
            BlockKind::Table { header, .. } => Some(header),
            _ => None,
        }
    }

    /// What a command asks for where the caret is now, or `None` while it has nothing to act on.
    fn asked(&self, act: &Act) -> Option<Message> {
        let editor = &self.editor;
        let picture = editor.picture();
        let in_text = editor.selection().is_some() && picture.is_none();
        // The selected picture's address while it is a link that safe mode does not hold back.
        let held = |src: &str| self.linked.said(src) == Some(&PictureState::Held);
        let link = picture.and_then(|figure| self.link_of(figure)).filter(|src| !held(src));
        // One place along a row or column of `count`, if there is one that way.
        let beside = |at: usize, count: usize, on: bool| {
            if on { Some(at + 1).filter(|to| *to < count) } else { at.checked_sub(1) }
        };
        let message = match act {
            Act::Run(command) => {
                // The editor says where a command applies. One that turns something on or off is listed
                // for the way it would turn it, and a picture is no text to make a heading of.
                let fits = match command {
                    Command::Table(TableEdit::Header(on)) => self.header() == Some(!on),
                    Command::SetNumbered(on) => self.numbered() == Some(!on),
                    Command::SetBlock(_) => in_text,
                    Command::Callout(edit) => callouts::changes(editor, edit),
                    _ => true,
                };
                (fits && editor.applies(command)).then(|| Message::Run(command.clone()))?
            }
            Act::Misspelling { back } => Message::Misspelling { back: *back },
            Act::MoveRow { down } => {
                let cell = editor.cell()?;
                Message::Run(Command::Table(TableEdit::MoveRow { to: beside(cell.row, cell.rows, *down)? }))
            }
            Act::MoveColumn { right } => {
                let cell = editor.cell()?;
                Message::Run(Command::Table(TableEdit::MoveColumn { to: beside(cell.column, cell.columns, *right)? }))
            }
            Act::Link => in_text.then_some(Message::EditLink)?,
            Act::Comment => {
                let selection = editor.selection().filter(|selection| !selection.is_caret())?;
                notes::passage(editor.doc(), selection).map(|_| Message::Note(Noting::Write))?
            }
            Act::Pictures => Message::Ask(Request::Pictures),
            Act::AltText => self.captioned().map(|_| Message::Alt(Altering::Open))?,
            Act::Callout => editor.callout().map(|_| Message::Callout(Calling::Open))?,
            Act::Load => {
                let waits = |src: &&str| self.linked.said(src).is_none_or(|state| *state != PictureState::Loading);
                let src = link.filter(waits).filter(|src| self.images.get(src).is_none());
                src.map(|_| Message::Linked(Linking::Load))?
            }
            Act::AlwaysLoad => link.filter(|src| !self.linked.vouched(src)).map(|_| Message::Linked(Linking::Trust))?,
            Act::Embed => link.map(|_| Message::Linked(Linking::Embed))?,
            Act::EmbedAll => {
                let any = editor.doc().walk().filter_map(pictures::link).any(|src| !held(src));
                any.then_some(Message::Linked(Linking::EmbedAll))?
            }
            Act::Panel(panel) => Message::Panel(*panel),
            Act::NewDocument => Message::NewDocument,
            Act::Open => Message::OpenDialog,
            Act::Rename => Message::Rename(Rename::Open),
            // Only where the shell keeps the document when asked, and has something to keep.
            Act::Save => (self.save == SaveState::Edited).then_some(Message::Ask(Request::Save))?,
            Act::SaveCopy => Message::Copy(Copying::Open(CopyFormat::Letters)),
            Act::Export => Message::Copy(Copying::Open(CopyFormat::Pdf)),
            Act::Share => Message::Share(true),
            Act::Settings => Message::Settings(Setting::Show),
            Act::PageSetup => Message::Styles(Styling::PageSetup),
            Act::TrustDocument => self.safe.on().then_some(Message::Safe(Safely::TrustDocument))?,
            Act::TrustFolder => self.safe.folder().then_some(Message::Safe(Safely::TrustFolder))?,
            // A card over the window is answered first: there is no document to act on behind it.
            Act::Print => (!self.carded()).then_some(Message::Print(Printing::Show))?,
            Act::History => (!self.carded()).then_some(Message::History(Historying::Show))?,
            Act::Review(asked) => {
                let (open, can) = (self.margin.reviewing().is_some(), self.margin.refused.is_none());
                let fits = match asked {
                    // With one conflict that is open already there is nowhere to step to.
                    Reviewing::Step { .. } => self.margin.conflicts().len() > usize::from(open),
                    Reviewing::Keep | Reviewing::Use => open && can,
                    Reviewing::Close => open,
                };
                fits.then(|| Message::Review(asked.clone()))?
            }
            Act::Palette => (!self.carded()).then_some(Message::Commands(Commanding::Toggle))?,
            // Text is found on the page, so only while the page shows.
            Act::Find | Act::Replace => {
                let open = Finding::Open { replace: *act == Act::Replace };
                (!self.carded() && !self.away()).then_some(Message::Find(open))?
            }
            Act::FindStep { back } => {
                let steps = self.find.steps() && !self.carded() && !self.away();
                steps.then_some(Message::Find(Finding::Step { back: *back }))?
            }
        };
        Some(message)
    }

    /// Whether a card lies over the window that has to be answered before there is a document to act on.
    fn carded(&self) -> bool {
        self.landing.is_some() || self.importing.is_some()
    }

    /// Gives the window back to the page when a card has come to lie over it.
    fn uncover(&mut self) {
        if self.carded() {
            self.printing = None;
            self.leave();
        }
    }

    /// Whether a view has the page's place: Print view, or the history.
    fn away(&self) -> bool {
        self.printing.is_some() || self.history.is_some()
    }

    /// Does what is asked of the history.
    fn historying(&mut self, asked: Historying, platform: &mut dyn Platform) {
        match asked {
            Historying::Show if self.away() || self.carded() => {}
            Historying::Show => {
                // What hangs from the page waits there for the way back. What lies over the window goes.
                self.commands.close();
                self.sharing = false;
                if self.grip.release() {
                    self.drop = None;
                }
                self.dragging = false;
                self.unpoint();
                let (steps, current) = (self.editor.timeline(), self.editor.model().revision());
                let mut history = History::new(&steps, current, platform.wall_time(), self.zone);
                history.add_fallback_fonts(&self.backup);
                history.still = self.reduced_motion;
                self.history = Some(history);
                // The nodes that stand for the blocks go with the page, and are told whole when it is back.
                self.told.get_mut().forget();
                self.turn();
            }
            Historying::Back => {
                self.take_name();
                self.leave();
            }
            Historying::Pick(at) => {
                self.take_name();
                self.history.iter_mut().for_each(|history| history.pick(at));
                self.turn();
            }
            Historying::Step { back } => {
                self.history.iter_mut().for_each(|history| history.step(back));
                self.turn();
            }
            Historying::Scrub(across) => {
                self.take_name();
                let Some(history) = &mut self.history else { return };
                // A drag sends many points for each version it passes.
                let at = history.across(across);
                if at != history.at() {
                    history.pick(at);
                    self.turn();
                }
            }
            Historying::Name(Naming::Apply) => self.take_name(),
            Historying::Name(asked) => self.history.iter_mut().for_each(|history| history.name(asked.clone())),
            Historying::Scroll(delta) => self.history.iter_mut().for_each(|history| history.scroll(delta)),
            Historying::Down { at, clicks: 1, extend } => {
                self.history.iter_mut().for_each(|history| history.press(at, extend));
            }
            Historying::Down { clicks, .. } => {
                self.history.iter_mut().for_each(|history| history.widen(clicks, platform));
            }
            Historying::Drag(at) => self.history.iter_mut().for_each(|history| history.drag(at)),
            Historying::Up(at) => self.history.iter_mut().for_each(|history| history.release(at)),
            Historying::All => self.history.iter_mut().for_each(|history| history.select_all(platform)),
            Historying::Copy => self.history.iter_mut().for_each(|history| history.copy(platform)),
            Historying::Unselect => self.history.iter_mut().for_each(History::unselect),
            Historying::Compare(with) => {
                self.take_name();
                self.history.iter_mut().for_each(|history| history.compare(with));
                self.turn();
            }
            Historying::Restore => {
                self.take_name();
                let Some(revision) = self.history.as_ref().map(|history| history.picked().at.clone()) else { return };
                // The document reads as the version again, and is there to be written on.
                self.leave();
                self.edit(platform, |editor, platform| editor.run(Command::Restore(revision), platform));
            }
        }
    }

    /// Gives the picked version the name that was typed for it, when one was. A name is a mark in the
    /// history: something to save, though no edit and nothing that undo takes back.
    fn take_name(&mut self) {
        let Some((revision, name)) = self.history.as_mut().and_then(History::named) else { return };
        if self.editor.name_version(&revision, name.as_deref()).is_ok() {
            self.edits += 1;
            let (steps, current) = (self.editor.timeline(), self.editor.model().revision());
            self.history.iter_mut().for_each(|history| history.follow(&steps, current.clone()));
            self.turn();
        }
    }

    /// Reads what the history is to show and has not read: the version it has picked, and the one that
    /// is compared with.
    fn turn(&mut self) {
        let Some(history) = &mut self.history else { return };
        for revision in history.wanted() {
            if let Ok(read) = self.editor.document_at(&revision) {
                history.take(revision, read);
            }
        }
        history.show();
    }

    /// Gives the page its place back from the history.
    fn leave(&mut self) {
        if self.history.take().is_some() {
            // The nodes that stood for a version's blocks are not the page's.
            self.told.get_mut().forget();
            // A card that asks for a copy of a version has that version nowhere to be seen.
            self.copying.take_if(|copy| copy.earlier());
        }
    }

    /// The version the history has picked, for the area the page has while it is edited.
    fn version(&mut self, room: Size) -> Element<Message> {
        let Some(history) = self.history.as_mut() else { return Element::row() };
        let shaper = self.shaper.as_ref();
        let (version, awaited) = history.view(room, &self.images, shaper, self.palette, self.scheme, self.scale);
        for asset in awaited {
            if self.wanted.insert(asset.clone()) {
                self.requests.push(Request::Asset(asset));
            }
        }
        // The tool that copies a passage waits behind the card that copies the version.
        match (&self.copying, history.tools()) {
            (Some(copy), _) => version.popover(copy.card(&self.location, room)),
            (None, Some(tools)) => version.popover(tools),
            (None, None) => version,
        }
    }

    /// What a key does while the history shows: it steps through the versions, moves through the one
    /// that shows, or goes back to editing. Nothing is written there.
    fn history_key(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        // Saving where it is asked for writes nothing on the document, and is asked from here as well. So
        // is a copy, which is of the version that shows.
        let kept = commands::bound(event).filter(|act| matches!(act, Act::Save | Act::SaveCopy));
        if let Some(asked) = kept.and_then(|act| self.asked(act)) {
            self.update(asked, platform);
            return true;
        }
        let plain = !event.mods.primary && !event.mods.alt;
        // A passage of the version is selected and copied with the keys that do so on the page.
        let asked = match self.editor.keymap().command(event) {
            Some(Command::Copy) => Some(Historying::Copy),
            Some(Command::SelectAll) => Some(Historying::All),
            _ => None,
        };
        if let Some(asked) = asked {
            self.historying(asked, platform);
            return true;
        }
        let asked = match event.key {
            Key::Escape => Historying::Back,
            Key::Left if plain => Historying::Step { back: true },
            Key::Right if plain => Historying::Step { back: false },
            _ => return self.history.as_mut().is_some_and(|history| history.key(event)),
        };
        self.historying(asked, platform);
        true
    }

    /// Does what is asked of Print view.
    fn print(&mut self, asked: Printing, platform: &mut dyn Platform) {
        match asked {
            Printing::Show if self.away() || self.carded() => {}
            Printing::Show => {
                // What hangs from the page waits there for the way back. What lies over the window goes.
                self.commands.close();
                self.sharing = false;
                if self.grip.release() {
                    self.drop = None;
                }
                self.dragging = false;
                self.unpoint();
                let reading = self.layout.hit(Point { x: 0.0, y: self.viewport().y.max(0.0) });
                self.printing = Some(Print::new(reading));
                // The nodes that stand for the blocks go with the page, and are told whole when it is back.
                self.told.get_mut().forget();
            }
            Printing::Back => self.printing = None,
            Printing::Zoom(zoom) => self.printing.iter_mut().for_each(|print| print.zoom(zoom)),
            Printing::Scroll(delta) => self.printing.iter_mut().for_each(|print| print.scroll(delta)),
            Printing::Print => self.requests.push(Request::Print),
            Printing::Down(at) => self.printing.iter_mut().for_each(|print| print.press(at)),
            Printing::Up(at) => {
                if let Some(asked) = self.printing.as_mut().and_then(|print| print.release(at)) {
                    self.break_pages(asked, platform);
                }
            }
        }
    }

    /// Makes pages start with blocks, or no longer start there, as Print view asked. Each is an edit that
    /// undo takes back. The page is not brought to its caret for it: nothing of a break shows there.
    fn break_pages(&mut self, asked: Breaking, platform: &mut dyn Platform) {
        for block in asked.blocks {
            let command = Command::SetPageBreak { block, on: asked.on };
            let change = self.run(platform, |editor, platform| editor.run(command, platform));
            self.follow(&change);
        }
    }

    /// The sheets of Print view, for the area the page has while it is edited.
    fn sheets(&mut self, room: Size) -> Element<Message> {
        // A sheet's text is as wide as its paper leaves between the margins, whatever the window has.
        let page = &self.editor.model().style().page;
        let text = ((page.width - page.margin_left - page.margin_right) * PT).max(1.0);
        if text != self.column.1 {
            self.layout.update(self.editor.doc(), self.editor.model().style(), text, None);
            self.column.1 = text;
            self.highlight(None);
            self.tell_links(None);
        }
        // Paging lays out whatever was still estimated.
        let pages = self.layout.paginate(&self.editor.model().style().page);
        self.estimated = false;
        let Some(print) = self.printing.as_mut() else { return Element::row() };
        let paged = Paged { doc: self.editor.doc(), layout: &self.layout, pages: &pages };
        let (mut sheets, awaited) = print.view(paged, room, &self.images, self.shaper.as_ref(), self.scheme);
        for asset in awaited {
            if self.wanted.insert(asset.clone()) {
                self.requests.push(Request::Asset(asset));
            }
        }
        if let Some(copy) = &self.copying {
            sheets = sheets.popover(copy.card(&self.location, room));
        }
        sheets
    }

    /// What a key does while Print view shows: it moves through the sheets, prints, takes a change back
    /// or goes back to editing. Nothing is written there.
    fn print_key(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        if event.key == Key::Escape {
            self.print(Printing::Back, platform);
            return true;
        }
        if self.printing.as_mut().is_some_and(|print| print.key(event)) {
            return true;
        }
        // A change to the page is an edit, which undo takes back from here too.
        let command =
            self.editor.keymap().command(event).filter(|command| matches!(command, Command::Undo | Command::Redo));
        if let Some(command) = command.cloned() {
            self.edit(platform, |editor, platform| editor.run(command, platform));
            return true;
        }
        let asked = match commands::bound(event) {
            Some(Act::Print) => Some(Message::Print(Printing::Print)),
            Some(act @ (Act::Settings | Act::Save | Act::SaveCopy)) => self.asked(act),
            _ => None,
        };
        let bound = asked.is_some();
        if let Some(message) = asked {
            self.update(message, platform);
        }
        bound
    }

    /// The commands the palette lists: those that what is typed finds and that have something to act on,
    /// each with what it asks.
    fn offers(&self) -> Vec<(&'static Entry, Message)> {
        self.commands.matching().filter_map(|entry| Some((entry, self.asked(&entry.act)?))).collect()
    }

    /// Does what the command palette asked. A command that is taken is done as if its key were pressed.
    fn commanding(&mut self, asked: Commanding, platform: &mut dyn Platform) {
        let taken = match asked {
            Commanding::Toggle => return self.commands.toggle(),
            Commanding::Close => return self.commands.close(),
            Commanding::Type(typed) => return self.commands.typed(typed),
            Commanding::Step { down } => return self.commands.step(down, self.offers().len()),
            Commanding::Take(index) => Some(index),
            Commanding::Enter => None,
        };
        let mut offers = self.offers();
        let taken = taken.unwrap_or_else(|| self.commands.picked(offers.len()));
        self.commands.close();
        if taken < offers.len() {
            self.update(offers.swap_remove(taken).1, platform);
        }
    }

    /// Does what the bar that finds and replaces asked.
    fn finding(&mut self, asked: Finding, platform: &mut dyn Platform) {
        match self.find.update(asked, &self.editor) {
            Some(finding::Does::Go(to)) => self.go(to),
            Some(finding::Does::Run(command)) => {
                let one = matches!(&command, Command::Replace { only: Some(_), .. });
                let held = self.find.current(self.editor.selection()).is_some();
                self.edit(platform, |editor, platform| editor.run(command, platform));
                // After one place is replaced, the next is the one to look at. After all of them, the one
                // that was selected to be looked at is let go of.
                let start = self.editor.selection().map(|selection| Selection::caret(selection.anchor.clone()));
                let next = if one { self.find.seek(&self.editor, Seek::Here) } else { start.filter(|_| held) };
                if let Some(next) = next {
                    self.go(next);
                }
            }
            None => {}
        }
    }

    /// Selects a place the text was found at and brings it into view, clear of the bar that found it.
    fn go(&mut self, to: Selection) {
        self.editor.select(to);
        self.mark();
        self.unpoint();
        // Twice, since laying out what comes into view can move the place.
        for _ in 0..2 {
            let head = self.editor.selection().map(|selection| &selection.head);
            let Some(line) = head.and_then(|head| self.layout.caret(head)) else { break };
            let top = self.down(line.y) + TOP;
            let margin = GUTTER.min(self.height / 4.0);
            let cover = self.find.cover().min(self.height / 2.0);
            let seen = self.scroll + cover + margin..=self.scroll + self.height - margin;
            if !seen.contains(&top) || !seen.contains(&(top + line.height)) {
                // A place out of view is brought a third of the way down the page.
                self.scroll = (top - self.height / 3.0).clamp(0.0, self.max_scroll());
            }
            self.layout.settle(self.viewport(), 0);
        }
    }

    /// Tells the shell again of each linked picture that does not show, since what it held back may load
    /// now. With `freed` the document has left safe mode, which no longer holds any of them.
    fn ask_again(&mut self, freed: bool) {
        let links = self.editor.doc().walk().filter_map(pictures::link);
        let held: BTreeSet<String> = links.filter(|src| self.images.get(src).is_none()).map(str::to_owned).collect();
        for src in held {
            if freed && self.linked.said(&src) == Some(&PictureState::Held) {
                self.linked.say(&src, None);
            }
            self.requests.push(Request::LinkedPicture(src));
        }
    }

    /// Does what the safe mode bar, a link or the card about a link asked, most of which is the shell's
    /// to do.
    fn safely(&mut self, asked: Safely) {
        match asked {
            Safely::TrustDocument => self.requests.push(Request::TrustDocument),
            Safely::TrustFolder => self.requests.push(Request::TrustFolder),
            Safely::Hide => self.safe.hide(true),
            Safely::Show => self.safe.hide(false),
            Safely::Follow(href) => {
                self.link = None;
                self.open_link(&href, false);
            }
            Safely::Confirm => {
                if let Some(href) = self.safe.confirm() {
                    self.open_link(&href, true);
                }
            }
            Safely::Leave => self.safe.leave(),
        }
    }

    /// Asks the shell to open a link. A place in this document is not the shell's to open.
    fn open_link(&mut self, href: &str, confirmed: bool) {
        let href = href.trim();
        if !href.is_empty() && !href.starts_with('#') {
            self.requests.push(Request::OpenLink { href: href.to_owned(), confirmed });
        }
    }

    /// The address of the link at a point of the page area, while nothing hangs over the page.
    fn link_at(&self, at: Point) -> Option<String> {
        if self.hung() {
            return None;
        }
        safe::link_at(self.editor.doc(), &self.layout, self.in_layout(at))
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

    /// The most a row of a formula's source may take across on its sheet, which is no wider than the text.
    fn wide(&self) -> f32 {
        Maths::wide(self.column.1, 2.0 * self.column.0 + self.column.1)
    }

    /// Runs something on the editor, which may ask the layout about lines while it works.
    fn edit(&mut self, platform: &mut dyn Platform, act: impl FnOnce(&mut Editor, &mut dyn Platform) -> Change) {
        let change = self.run(platform, act);
        self.changed(&change);
    }

    /// Lets the editor do something with what the window lends it, and returns what that changed.
    fn run(
        &mut self,
        platform: &mut dyn Platform,
        act: impl FnOnce(&mut Editor, &mut dyn Platform) -> Change,
    ) -> Change {
        // On a sheet the caret moves by the lines of the source, which the page does not show.
        let wide = self.wide();
        let source = Source::of(&self.editor, self.shaper.as_ref(), wide);
        let formula = self.editor.selection().and_then(|selection| self.layout.formula_rect(&selection.head));
        let left = source.as_ref().zip(formula).map(|(source, rect)| Maths::left(source, rect.x, rect.width, wide));
        let lines = SheetLines {
            page: DocLines::new(&self.layout, self.height),
            source: source.filter(|_| left.is_some()),
            left: left.unwrap_or_default(),
        };
        let mut lent = Lent { platform, lines };
        act(&mut self.editor, &mut lent)
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
            // What the pointer rested on may be gone, or somewhere else.
            self.unpoint();
            self.highlight((!change.structure).then_some(&change.blocks));
            self.tell_links((!change.structure).then_some(&change.blocks));
            // A picture that was held may be gone, or another size.
            if self.grip.release() {
                self.drop = None;
            }
            let caret = self.editor.selection().filter(|selection| selection.is_caret()).map(|caret| &caret.head);
            self.spelling.edited(change, self.editor.doc(), caret, &mut self.layout);
            self.margin.edited(self.editor.model());
            self.find.refresh(&self.editor);
        }
        if change.document {
            self.spelling.reworded(None, self.editor.doc(), &mut self.layout);
        }
        if edited || change.style || change.document {
            self.edits += 1;
            // What came in under the history, as another writer's edits do, is its newest version.
            if self.history.is_some() {
                let (steps, current) = (self.editor.timeline(), self.editor.model().revision());
                self.history.iter_mut().for_each(|history| history.follow(&steps, current.clone()));
                self.turn();
            }
        }
        self.mark();
        self.sync();
    }

    /// Does something that may change the height of what is above the view, and keeps the view's top line
    /// where it is.
    fn held<R>(&mut self, act: impl FnOnce(&mut Self) -> R) -> R {
        let pin = self.layout.hit(Point { x: 0.0, y: self.viewport().y.max(0.0) });
        let top = |app: &Self| pin.as_ref().and_then(|pos| app.layout.caret(pos)).map(|caret| app.down(caret.y));
        let before = top(self);
        let done = act(self);
        if let (Some(before), Some(after)) = (before, top(self)) {
            self.scroll_by(after - before);
        }
        done
    }

    /// The part of the document in view, in the layout's coordinates.
    fn viewport(&self) -> Rect {
        let top = self.scrolled() - TOP;
        let Some(cut) = self.cut() else { return Rect::new(0.0, top, self.column.1, self.height) };
        // Less of the page's own column shows while the version held back takes room of its own.
        let (y, height) = review::shown(&cut, Rect::new(0.0, top, self.across, self.height));
        Rect::new(0.0, y, self.column.1, height)
    }

    fn max_scroll(&self) -> f32 {
        (self.tall() + 2.0 * TOP - self.height).max(0.0)
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
        let top = self.down(caret.y) + TOP;
        let bottom = top + caret.height;
        let margin = GUTTER.min(self.height / 4.0);
        if top < self.scroll + margin {
            self.scroll = top - margin;
        } else if bottom > self.scroll + self.height - margin {
            self.scroll = bottom + margin - self.height;
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// The text position under a point of the page area, while the page shows there.
    fn hit(&self, at: Point) -> Option<Pos> {
        if self.away() {
            return None;
        }
        self.layout.hit(self.in_layout(at))
    }

    /// Whether a card, a menu or a field that the person asked for is out over the page.
    fn covered(&self) -> bool {
        let card = self.landing.is_some() || self.importing.is_some() || self.copying.is_some();
        let asked = self.context.is_some() || self.link.is_some() || self.linked.open.is_some();
        let asked = asked || self.captions.alt.is_some() || self.tables.open.is_some();
        card || asked || self.commands.open() || self.callouts.open.is_some()
    }

    /// Whether something hangs over the page and takes what the pointer does there. A note the caret opened
    /// does not: the page under it is still being written on.
    fn hung(&self) -> bool {
        let read = self.margin.reading().is_some() && !self.margin.follows();
        let asked = self.covered() || self.margin.writing().is_some() || read;
        asked || self.insert.offers(&self.editor).is_some() || Refer::offers(&self.editor).is_some()
    }

    /// The open comment whose passage the selection ends in.
    fn noted(&self) -> Option<String> {
        let head = &self.editor.selection()?.head;
        self.margin.about(head).map(str::to_owned)
    }

    /// Has the notes follow the caret, after the person did something: a comment opens under its passage
    /// when the caret is put there, and goes when the caret leaves. `was` is what [`Self::noted`] gave
    /// before.
    fn attend(&mut self, was: Option<&str>, platform: &mut dyn Platform) {
        // A press opens nothing while the button is down: it may be the start of a selection.
        let free = !(self.dragging || self.away() || self.covered());
        let caret = self.editor.selection().filter(|selection| selection.is_caret()).map(|caret| &caret.head);
        if self.margin.attend(was, caret, free) {
            self.margin.now = platform.wall_time().map(notes::stamp).unwrap_or_default();
        }
    }

    /// The callout whose icon and title are at a point of the page area, while nothing hangs over the page.
    fn callout_at(&self, at: Point) -> Option<BlockId> {
        if self.hung() || self.away() {
            return None;
        }
        let at = self.in_layout(at);
        let heads = self.layout.callouts(Rect::new(at.x, at.y - 64.0, 0.0, 128.0));
        heads.into_iter().find(|(_, head)| callouts::target(*head).contains(at)).map(|(block, _)| block)
    }

    /// Opens the card of a callout, as a press on its icon or title does. The caret goes inside it first,
    /// since the card is about the callout the caret is in.
    fn call(&mut self, callout: &BlockId, platform: &mut dyn Platform) {
        let Some(inside) = self.editor.doc().block(callout).and_then(callouts::inside) else { return };
        self.editor.select(Selection::caret(inside));
        self.mark();
        self.calling(Calling::Open, platform);
    }

    /// Does what the card of a callout asked.
    fn calling(&mut self, asked: Calling, platform: &mut dyn Platform) {
        if asked == Calling::Open {
            (self.link, self.spelling.fix, self.linked.open, self.captions.alt) = (None, None, None, None);
        }
        if let Some(edit) = self.callouts.update(asked, &self.editor) {
            self.edit(platform, |editor, platform| editor.run(Command::Callout(edit), platform));
        }
    }

    /// The code block whose box is at a point of the page area, and whether the point is on its copy
    /// button, while nothing hangs over the page.
    fn code_at(&self, at: Point) -> Option<(BlockId, bool)> {
        if self.hung() {
            return None;
        }
        let at = self.in_layout(at);
        let boxes = self.layout.code_boxes(Rect::new(at.x, at.y, 0.0, 1.0));
        let (block, code) = boxes.into_iter().find(|(_, code)| code.contains(at))?;
        Some((block, code_copy::button(code).contains(at)))
    }

    /// The code block whose copy button is at a point of the page area.
    fn copy_button(&self, at: Point) -> Option<BlockId> {
        self.code_at(at).filter(|(_, on)| *on).map(|(block, _)| block)
    }

    /// Puts a code block's text on the clipboard as it is written, and has its button say so.
    fn copy_code(&mut self, block: BlockId, platform: &mut dyn Platform) {
        let Some(code) = self.editor.doc().block(&block) else { return };
        platform.clipboard_write(Clip { text: code.plain_text(), ..Clip::default() });
        self.code_copy.copied(block, platform.now_ms());
    }

    /// The cell the caret is in, when it is in one.
    fn caret_cell(&self) -> Option<&BlockId> {
        self.editor.cell()?;
        self.editor.selection().map(|selection| &selection.head.block)
    }

    /// The table whose handles show: the one a press on them holds, the one the pointer rests over or the
    /// one the caret is in. None shows while a conflict is open, or something else hangs over the page.
    fn table(&self) -> Option<tables::Shown> {
        if let Some(held) = self.tables.held_on() {
            return Some(held.clone());
        }
        let menu = self.tables.open.is_some();
        let gripped = self.grip.resizing.is_some() || self.grip.moving.is_some();
        if (self.hung() && !menu) || self.cut().is_some() || gripped {
            return None;
        }
        let pointer = self.references.rest().filter(|_| !menu).map(|at| self.in_layout(at));
        tables::Shown::find(self.layout.tables(self.viewport()), pointer, self.caret_cell())
    }

    /// The handle or bar of a table at a point of the page area.
    fn table_part(&self, at: Point) -> Option<(tables::Shown, tables::Part)> {
        let shown = self.table().filter(|_| self.tables.open.is_none())?;
        let part = shown.part_at(self.in_layout(at))?;
        Some((shown, part))
    }

    /// Does what a press on a table's handles asked for, at the column or row it was about.
    fn tabled(&mut self, shown: &tables::Shown, asked: tables::Asked, platform: &mut dyn Platform) {
        let (axis, index) = match asked {
            tables::Asked::Menu(axis, index) | tables::Asked::Move { axis, from: index, .. } => (axis, Some(index)),
            tables::Asked::Insert { axis, before } => {
                let count = match axis {
                    tables::Axis::Column => shown.table.columns.len(),
                    tables::Axis::Row => shown.table.rows.len(),
                };
                (axis, Some(before).filter(|before| *before < count))
            }
        };
        // The commands of a table act where the caret is, so it goes there first.
        let Some(cell) = shown.cell(axis, index, self.caret_cell()) else { return };
        if self.caret_cell() != Some(&cell) || !self.editor.selection().is_some_and(Selection::is_caret) {
            self.editor.select(Selection::caret(Pos::new(cell, 0)));
            self.mark();
        }
        let edit = match (asked, axis) {
            (tables::Asked::Menu(..), _) => {
                (self.link, self.spelling.fix, self.captions.alt) = (None, None, None);
                self.margin.shut();
                self.tables.open = Some(axis);
                return self.reveal();
            }
            // Past the last there is none to go before.
            (tables::Asked::Insert { .. }, tables::Axis::Column) => TableEdit::AddColumn { after: index.is_none() },
            (tables::Asked::Insert { .. }, tables::Axis::Row) => TableEdit::AddRow { below: index.is_none() },
            (tables::Asked::Move { to, .. }, tables::Axis::Column) => TableEdit::MoveColumn { to },
            (tables::Asked::Move { to, .. }, tables::Axis::Row) => TableEdit::MoveRow { to },
        };
        self.edit(platform, |editor, platform| editor.run(Command::Table(edit), platform));
    }

    /// The menu of the caret's column or row, hung from its handle.
    fn table_menu(&self) -> Option<Popover<Message>> {
        let (axis, cell, shown) = (self.tables.open?, self.editor.cell()?, self.table()?);
        let index = match axis {
            tables::Axis::Column => cell.column,
            tables::Axis::Row => cell.row,
        };
        let spot = self.onto(shown.rect(tables::Part::Handle(axis, index))?);
        let aligned = match &self.editor.doc().block(&cell.table)?.kind {
            BlockKind::Table { columns, .. } => columns.get(cell.column).copied(),
            _ => None,
        };
        // Only the first row can be the header.
        let header = self.header().filter(|_| cell.row == 0);
        let menu = tables::menu(axis, |act| self.asked(act), aligned, header);
        Some(menu.popover().at(spot).open(true).on_close(Message::TableMenu))
    }

    /// Whether the first line a code block shows runs as far as the copy button at `button`, in the layout.
    fn reaches(&self, block: &BlockId, button: Rect) -> bool {
        let end = self.layout.line_edge(&Pos::new(block.clone(), 0), true);
        let end = end.and_then(|end| self.layout.caret(&end));
        end.is_some_and(|end| end.x + code_copy::CLEAR > button.x)
    }

    /// The reference to a figure at a point of the page area, while nothing hangs over the page.
    fn reference(&self, at: Point) -> Option<Reference> {
        if self.hung() {
            return None;
        }
        let at = self.in_layout(at);
        Reference::at(self.editor.doc(), &self.layout, at)
    }

    /// Whether enough of a figure's picture is in view to tell it by: half of it, or of the page area.
    fn in_view(&self, figure: &BlockId) -> bool {
        let Some(picture) = self.layout.figure(figure).map(|figure| figure.image) else { return false };
        let view = self.viewport();
        let shown = (picture.y + picture.height).min(view.y + view.height) - picture.y.max(view.y);
        shown >= picture.height.min(view.height) / 2.0
    }

    /// Where a figure's picture is in the layout, and each picture of a gallery by itself.
    fn pictures(&self, figure: &BlockId) -> Option<(Rect, Vec<Rect>)> {
        let whole = self.layout.figure(figure)?.image;
        let parts = self.editor.doc().block(figure).map_or(&[][..], |block| block.body.blocks());
        let parts = parts.iter().filter(|part| matches!(part.kind, BlockKind::Figure { .. }));
        let each: Vec<Rect> = parts.filter_map(|part| Some(self.layout.figure(&part.id)?.image)).collect();
        Some((whole, if each.is_empty() { vec![whole] } else { each }))
    }

    /// Works out again which reference or linked picture the pointer rests on, after it or the page under
    /// it has moved. Returns whether what shows changed with it.
    fn repoint(&mut self) -> bool {
        let code = self.references.rest().and_then(|at| self.code_at(at));
        let copy = self.code_copy.point(code);
        // The button covers what is under it, a reference too.
        let found = self.references.rest().filter(|_| !self.code_copy.pointed()).and_then(|at| self.reference(at));
        let found = found.map(|reference| {
            let in_view = self.in_view(&reference.target);
            (reference, in_view)
        });
        let headed = self.references.rest().and_then(|at| self.callout_at(at));
        let called = std::mem::replace(&mut self.callouts.over, headed.clone()) != headed;
        let linked = self.references.rest().and_then(|at| self.linked_at(at));
        // Each is taken, whichever changed.
        let clouded = self.linked.point(linked);
        let corner = self.references.rest().filter(|_| !self.hung()).and_then(|at| {
            let (_, image) = self.lone_picture()?;
            figures::corner_at(image, self.in_layout(at))
        });
        let gripped = std::mem::replace(&mut self.grip.over, corner) != corner;
        let over = self.references.rest().filter(|_| !self.hung()).is_some_and(|at| {
            self.alt_switch().is_some_and(|(_, _, switch)| captions::pressed(switch, self.in_layout(at)))
        });
        let switched = std::mem::replace(&mut self.captions.over, over) != over;
        let rest = self.references.rest().filter(|_| !self.hung());
        let marker = rest.and_then(|at| self.marker_at(self.in_layout(at)));
        let marked = self.margin.over != marker;
        self.margin.over = marker;
        let part = rest.and_then(|at| self.table_part(at)).map(|(_, part)| part);
        let tabled = self.tables.point(part);
        self.references.point(found) || copy || clouded || gripped || switched || marked || tabled || called
    }

    /// Forgets where the pointer rests until it next moves, as when the person goes on writing.
    fn unpoint(&mut self) {
        self.references.set_rest(None);
        self.repoint();
    }

    /// Brings a figure into view, as a press on a reference to it asks: to the middle of the page area
    /// with its caption, or under the top when it is too tall for that. One that shows whole stays where it
    /// is. The caret stays at the reference, so the next key goes on from there.
    fn show_figure(&mut self, figure: &BlockId) {
        // Twice, since laying out what comes into view can move the figure.
        for _ in 0..2 {
            let Some(at) = self.layout.figure(figure) else { return };
            let (top, bottom) = (at.image.y, at.caption.y + at.caption.height);
            let view = self.viewport();
            if top >= view.y && bottom <= view.y + view.height {
                break;
            }
            let above = ((self.height - (bottom - top)) / 2.0).max(GUTTER.min(self.height / 4.0));
            self.scroll = (self.down(top) + TOP - above).clamp(0.0, self.max_scroll());
            self.layout.settle(self.viewport(), 0);
        }
        self.references.flash(figure);
    }

    fn pointer(&mut self, event: Pointer, platform: &mut dyn Platform) {
        match event {
            // What stands round the two versions of a conflict, and the one held back, are not the page's.
            Pointer::Down { at, .. } if self.review.press(at) => {
                self.dragging = false;
                self.references.press(None);
            }
            Pointer::Drag { .. } if self.review.pressed() => {}
            Pointer::Up { at } if self.review.pressed() => {
                if let Some(asked) = self.review.release(at) {
                    self.reviewing(asked, platform);
                }
            }
            Pointer::Menu { at } if self.review.covers(at) => {}
            Pointer::Down { at, mods, clicks } => {
                self.picked = None;
                self.maths.typed(false);
                // A press on a marker in the margin is about the notes of its block, and leaves the caret
                // where it is.
                if let Some(block) = self.marker_at(self.in_layout(at)) {
                    self.dragging = false;
                    self.references.press(None);
                    return self.noting(Noting::Press(block), platform);
                }
                self.margin.forget();
                // A press on a callout's icon or title opens its card, with the caret inside the callout.
                if let Some(block) = self.callout_at(at) {
                    self.dragging = false;
                    self.references.press(None);
                    return self.call(&block, platform);
                }
                // A press on a copy button is the button's, and leaves the caret where it is.
                let button = self.copy_button(at);
                self.code_copy.press(button);
                if self.code_copy.held() {
                    self.dragging = false;
                    return self.references.press(None);
                }
                // A press on a handle or a bar of a table is theirs, and so is where the pointer goes with it.
                if let Some((shown, part)) = self.table_part(at) {
                    self.tables.press(shown, part, self.in_layout(at));
                    self.dragging = false;
                    return self.references.press(None);
                }
                // A press on a corner of the selected picture takes hold of it.
                let spot = self.in_layout(at);
                if let Some((figure, image)) = self.lone_picture()
                    && let Some(corner) = figures::corner_at(image, spot)
                {
                    self.grip.resizing = Some(Resizing::start(figure, image, corner, spot));
                    self.dragging = false;
                    return self.references.press(None);
                }
                // A press on the switch at the end of the line under a picture turns that line to the alt
                // text, and one on a line that offers a caption starts it.
                if let Some((figure, _, switch)) = self.alt_switch()
                    && captions::pressed(switch, spot)
                {
                    let figure = figure.clone();
                    self.dragging = false;
                    self.references.press(None);
                    return self.alter(figure);
                }
                if self.hint().is_some_and(|(line, _)| line.contains(spot)) {
                    self.dragging = false;
                    self.references.press(None);
                    return self.edit(platform, |editor, platform| editor.run(Command::EditCaption, platform));
                }
                // A press on a link with Ctrl or Cmd held asks for it to be opened, and leaves the caret be.
                if let Some(href) = self.link_at(at).filter(|_| mods.primary && !mods.shift && clicks == 1) {
                    self.dragging = false;
                    self.references.press(None);
                    return self.open_link(&href, false);
                }
                // A press that stays on a reference follows it. Any other press is the caret's alone.
                let pressed = self.reference(at).filter(|_| clicks == 1 && !mods.shift);
                self.references.press(pressed.map(|reference| (reference, at)));
                // Under the last block there is always a line to write on, also when the document ends in
                // something that holds the caret in, as a code block does.
                if at.y + self.scrolled() - TOP > self.tall() && !mods.shift {
                    self.dragging = false;
                    return self.edit(platform, |editor, platform| editor.run(Command::LineBelow, platform));
                }
                let Some(head) = self.hit(at) else { return };
                // A press on a picture selects its figure. The caption under it is text like any other.
                let on_page = self.in_layout(at);
                let figure = self.layout.figure(&head.block).filter(|figure| figure.image.contains(on_page));
                if let Some(figure) = figure.filter(|_| !mods.shift) {
                    self.editor.select_picture(&head.block);
                    self.dragging = false;
                    // The cloud on a picture that is still a link opens its menu. From anywhere else on it
                    // the picture can be carried off.
                    let cloud = figure.badge.is_some_and(|badge| pictures::pressed(badge, on_page));
                    if cloud && self.link_of(&head.block).is_some() {
                        self.link_menu(head.block);
                    } else {
                        self.grip.moving = Some(Moving::start(head.block, at));
                    }
                    return self.reveal();
                }
                let anchor = match self.editor.selection() {
                    Some(selection) if mods.shift => selection.anchor.clone(),
                    _ => head.clone(),
                };
                // The later presses of a double or triple click are about where the first put the caret,
                // since the text may have moved under the pointer when the caret came to it.
                if clicks == 1 || mods.shift || self.editor.selection().is_none() {
                    self.editor.select(Selection { anchor, head });
                }
                self.dragging = clicks == 1;
                match clicks {
                    1 => self.reveal(),
                    2 => self.edit(platform, |editor, platform| editor.run(Command::SelectWord, platform)),
                    _ => self.edit(platform, |editor, platform| editor.run(Command::SelectBlock, platform)),
                }
            }
            Pointer::Drag { at } if self.tables.held() => self.tables.drag(self.in_layout(at), DRAG_SLOP),
            Pointer::Up { at } if self.tables.held() => {
                if let Some((shown, asked)) = self.tables.release(self.in_layout(at)) {
                    self.tabled(&shown, asked, platform);
                }
            }
            Pointer::Drag { at } if self.grip.resizing.is_some() => {
                let (spot, widest) = (self.in_layout(at), self.column.1);
                if let Some(resizing) = &mut self.grip.resizing {
                    resizing.drag(spot, widest);
                }
            }
            Pointer::Drag { at } if self.grip.moving.is_some() => {
                if self.grip.moving.as_mut().is_some_and(|moving| moving.drag(at, DRAG_SLOP)) {
                    self.drop = self.carried_to(at).and_then(|(_, bar)| bar);
                }
            }
            Pointer::Up { .. } if self.grip.resizing.is_some() => {
                let resized =
                    self.grip.resizing.take().filter(|resizing| self.editor.picture() == Some(&resizing.figure));
                if let Some((width, height)) = resized.and_then(|resizing| resizing.size()) {
                    let size = Command::ResizeFigure { width: width / PT, height: height / PT };
                    self.edit(platform, |editor, platform| editor.run(size, platform));
                }
            }
            Pointer::Up { at } if self.grip.moving.is_some() => {
                let spot = self.carried_to(at).map(|(spot, _)| spot);
                (self.grip.moving, self.drop) = (None, None);
                if let Some(spot) = spot {
                    self.edit(platform, |editor, platform| editor.run(Command::MoveFigure(spot), platform));
                }
            }
            Pointer::Drag { at } if self.dragging => {
                let (Some(head), Some(selection)) = (self.hit(at), self.editor.selection()) else { return };
                // A drag that selects is no longer pointing at anything.
                if head != selection.anchor {
                    self.references.point(None);
                }
                self.editor.select(Selection { anchor: selection.anchor.clone(), head });
                // Dragging past an edge scrolls the page after the selection.
                self.reveal();
            }
            Pointer::Drag { .. } => {}
            Pointer::Up { at } if self.code_copy.held() => {
                let on = self.copy_button(at);
                if let Some(block) = self.code_copy.release(on.as_ref()) {
                    self.copy_code(block, platform);
                }
            }
            Pointer::Up { at } => {
                self.dragging = false;
                // The markers at the caret show once the button is up: text that made room for them
                // under a pointer still held would be selected by its next move.
                self.mark();
                let reference = self.reference(at);
                if let Some(figure) = self.references.followed(at, DRAG_SLOP, reference.as_ref()) {
                    self.show_figure(&figure);
                }
            }
            Pointer::Menu { at } => self.menu(at),
            Pointer::Scroll { delta } => {
                self.scroll_by(delta.y);
                // Another part of the page is under the pointer now.
                self.repoint();
            }
        }
    }

    /// Opens the menu about what is at a point of the page area. A press on a picture selects it, and one
    /// outside what is selected puts the caret there, as the primary button would.
    fn menu(&mut self, at: Point) {
        if self.hung() {
            return;
        }
        let Some(pos) = self.hit(at) else { return };
        let spot = self.in_layout(at);
        (self.link, self.spelling.fix, self.linked.open) = (None, None, None);
        let near = |rect: &Rect| {
            (rect.x - 2.0..=rect.x + rect.width + 2.0).contains(&spot.x)
                && (rect.y..=rect.y + rect.height).contains(&spot.y)
        };
        if self.layout.figure(&pos.block).is_some_and(|figure| figure.image.contains(spot)) {
            self.editor.select_picture(&pos.block);
        } else {
            // A selection that was pressed on is left alone, so that the menu serves it.
            let ranged = self.editor.selection().filter(|selection| !selection.is_caret());
            if !ranged.is_some_and(|selection| self.layout.selection(selection).iter().any(near)) {
                self.editor.select(Selection::caret(pos.clone()));
            }
            // A misspelled word is offered its corrections where the word itself was meant, not the room
            // its line leaves beside it.
            let word = self.layout.spelling_at(&pos).map(|range| Selection {
                anchor: Pos::new(pos.block.clone(), range.start),
                head: Pos::new(pos.block.clone(), range.end),
            });
            if word.is_some_and(|word| self.layout.selection(&word).iter().any(near)) {
                self.spelling.ask(self.editor.doc(), &pos, &self.layout);
            }
        }
        self.mark();
        let reference = Reference::at(self.editor.doc(), &self.layout, spot).map(|reference| reference.target);
        let on = |note: &Note| {
            notes::stretch(&note.anchor).is_some_and(|passage| self.layout.selection(&passage).iter().any(near))
        };
        let note = self.margin.open().find(|(note, kind)| *kind == threads::Kind::Comment && on(note));
        let note = note.map(|(note, _)| note.id.clone());
        self.menus = self.menus.wrapping_add(1);
        self.context = Some(Context { at, reference, note, serial: self.menus });
    }

    /// Goes to the next misspelled word, or the one before, and opens the menu about it there.
    fn misspelling(&mut self, back: bool) {
        if self.away() || self.carded() {
            return;
        }
        self.context = None;
        if self.hung() {
            return;
        }
        let from = self.editor.selection().map(|selection| selection.head.clone());
        let Some((block, word)) = self.spelling.next(self.editor.doc(), from.as_ref(), back, &mut self.layout) else {
            return;
        };
        self.editor.select(Selection::caret(Pos::new(block, word.start)));
        self.mark();
        // Twice, since laying out what comes into view can move the word.
        for _ in 0..2 {
            self.reveal();
            self.layout.settle(self.viewport(), 0);
        }
        let Some(caret) = self.editor.selection().and_then(|selection| self.layout.caret(&selection.head)) else {
            return;
        };
        let caret = self.onto(caret);
        self.menu(Point { x: caret.x + 1.0, y: caret.y + caret.height / 2.0 });
    }

    /// Does what the menu about what is under the pointer asked.
    fn contexting(&mut self, asked: Contexting, platform: &mut dyn Platform) {
        match asked {
            Contexting::Close(serial) => {
                self.context.take_if(|context| context.serial == serial);
            }
            Contexting::CopyText(block) => match self.editor.doc().block(&block) {
                Some(code) if matches!(code.kind, BlockKind::Code { .. }) => self.copy_code(block, platform),
                Some(written) => platform.clipboard_write(Clip { text: written.plain_text(), ..Clip::default() }),
                None => {}
            },
            Contexting::CopyPicture(block) => {
                let Some((top, bottom)) = self.editor.doc().block(&block).and_then(|code| self.extent(code)) else {
                    return;
                };
                let boxes = self.layout.code_boxes(Rect::new(0.0, top, self.column.1, bottom - top));
                let Some((_, code)) = boxes.into_iter().find(|(id, _)| *id == block) else { return };
                // The picture alone, so that what it is pasted into takes it for a picture.
                // Where each line of the code begins, for its number.
                let text = self.editor.doc().block(&block).map(|code| code.plain_text()).unwrap_or_default();
                let starts = std::iter::once(0).chain(text.match_indices('\n').map(|(at, _)| at + 1));
                let lines: Vec<Rect> =
                    starts.filter_map(|at| self.layout.caret(&Pos::new(block.clone(), at))).collect();
                let (ink, page) = (self.palette.text, self.palette.page);
                let shaper = self.shaper.as_ref();
                if let Some(picture) = context::picture(&self.layout, &self.images, shaper, code, &lines, ink, page) {
                    platform.clipboard_write(Clip { pictures: vec![picture], ..Clip::default() });
                    self.code_copy.copied(block, platform.now_ms());
                }
            }
            Contexting::CopyLink(address) => platform.clipboard_write(Clip { text: address, ..Clip::default() }),
            Contexting::ShowFigure(figure) => self.show_figure(&figure),
        }
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

    /// Lays the document out for the room it has and paints the part in view.
    fn page(&mut self, room: Size) -> Element<Message> {
        let page = &self.editor.model().style().page;
        let text = (page.width - page.margin_left - page.margin_right) * PT;
        let width = text.min(room.width - 2.0 * GUTTER).max(1.0);
        let left = ((room.width - width) / 2.0).max(0.0).round();
        if width != self.column.1 {
            self.layout.update(self.editor.doc(), self.editor.model().style(), width, None);
            self.highlight(None);
            self.tell_links(None);
        }
        // A menu whose picture is gone, or is a link no longer, goes with it.
        if self.linked.open.as_ref().is_some_and(|figure| self.link_of(figure).is_none()) {
            self.linked.open = None;
        }
        self.callouts.follow(&self.editor);
        // The alt text of a picture the selection has left is no longer being written.
        if self.captions.alt.as_ref().is_some_and(|(figure, _)| self.captioned() != Some(figure)) {
            self.captions.alt = None;
        }
        // A note that was open goes when something else comes to hang from the page.
        let other = self.context.is_some() || self.link.is_some() || self.linked.open.is_some();
        if (other || self.captions.alt.is_some()) && self.margin.reviewing().is_none() {
            self.margin.shut();
        }
        (self.column, self.across, self.height) = ((left, width), room.width, room.height);
        self.sync();
        // The line under the picture the selection is at is made ready for its caption, once the button is
        // up: a caption that made room for its switch under a pointer still held would be selected by its
        // next move.
        if !self.dragging {
            let within = self.editor.selection().filter(|selection| selection.anchor.block == selection.head.block);
            let named = within.map(|selection| selection.head.block.clone());
            if named == self.captions.named {
                self.layout.set_caption_open(named.as_ref());
            } else {
                // A line that opens or closes above the view leaves what is read where it is.
                self.held(|page| page.layout.set_caption_open(named.as_ref()));
                self.captions.named = named;
            }
        }
        // A heading's markers hang in the margin, beside the outline when it shows.
        let taken = if self.outlined() { chapters_width() } else { tokens::SPACE_SM };
        self.layout.set_margin(left - taken);
        // What is about to show is laid out for real. The rest waits for idle time.
        self.estimated = self.layout.settle(self.viewport(), 0);
        // The version held back is laid out where it shows, like the page.
        if let Some(cut) = self.cut() {
            self.review.settle(&cut, Rect::new(0.0, self.scrolled() - TOP, room.width, room.height));
        }
        self.scroll = self.scroll.min(self.max_scroll());

        let viewport = self.viewport();
        self.insert.follow(&self.editor);
        let mut list = DisplayList::default();
        let selection = self.editor.selection();
        // A formula with the caret in it has its source out on a sheet, while the place under it is in view
        // and nothing else hangs from the page.
        let alone = self.landing.is_none()
            && self.importing.is_none()
            && self.copying.is_none()
            && self.context.is_none()
            && self.link.is_none();
        let formula = selection.filter(|_| alone).and_then(|selection| self.layout.formula_rect(&selection.head));
        let formula = formula.filter(|rect| {
            let shown = self.onto(*rect);
            (0.0..=room.height).contains(&(shown.y + shown.height))
        });
        let sheet = formula.and_then(|rect| Some((Source::of(&self.editor, self.shaper.as_ref(), self.wide())?, rect)));
        self.maths.follow(sheet.as_ref().map(|(source, _)| source), self.reduced_motion);
        self.margin.follow(selection);
        // The words in which the page's version of a block differs from the one held back.
        for words in self.review.words() {
            review::mark(
                &self.layout.selection_in(words, viewport),
                Version::Second,
                &review::TONES,
                self.scheme,
                &mut list,
            );
        }
        for (passage, open) in self.margin.commented() {
            notes::mark(&self.layout.selection_in(&passage, viewport), open, self.scheme, &mut list);
        }
        // Every place the find bar found, but for the one that is selected.
        let band = |block: &BlockId| self.layout.band(block);
        for found in self.find.shown(selection, band, (viewport.y, viewport.y + viewport.height)) {
            finding::mark(&self.layout.selection_in(found, viewport), self.scheme, &mut list);
        }
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
        // A picture the shell has not handed over is asked for once, and its place is kept until it comes.
        for asset in pictures::awaited(&self.images, self.scheme, &mut list) {
            if self.wanted.insert(asset.clone()) {
                self.requests.push(Request::Asset(asset));
            }
        }
        // A formula that was just closed still has its source over it, on the way out.
        match self.maths.closing().map(|at| self.layout.formula_rect(at)) {
            Some(Some(rect)) => self.maths.blend(rect, self.palette.text, &mut list),
            Some(None) => self.maths.settle(),
            None => {}
        }

        if let Some(bar) = self.drop {
            list.items.push(Item::Fill { rect: bar, color: resolve(self.accent.base, self.scheme, 1.0), radius: 2.0 });
        }
        // A callout's icon and title say that a press on them does something.
        for (block, head) in self.layout.callouts(viewport) {
            self.callouts.paint(&block, head, self.scheme, &mut list);
        }
        // Each code block has its copy button in its corner, over its text.
        for (block, code) in self.layout.code_boxes(viewport) {
            let button = code_copy::button(code);
            let behind = self.reaches(&block, button).then_some([self.palette.page, self.palette.code]);
            self.code_copy.paint(&block, button, behind, self.scheme, &mut list);
        }

        // The table the pointer is over or the caret is in has its handles round it.
        if let Some(shown) = self.table() {
            self.tables.paint(&shown, self.accent.base, self.scheme, &mut list);
        }

        // Beside each block with open notes is their marker, in the margin.
        let read = self.margin.reading().map(|(note, _)| note.id.as_str());
        for (marker, at) in self.markers() {
            let open = read.is_some_and(|read| marker.notes.iter().any(|note| note == read));
            let over = self.margin.over.as_ref() == Some(&marker.block);
            threads::mark(marker, at, (open, over), self.shaper.as_ref(), self.scheme, &mut list);
        }

        // A figure that is selected has a ring round its picture, and a selected picture is where the caret is.
        let ring = resolve(accent().base, self.scheme, 1.0);
        let pictures = self.editor.figures().into_iter().filter_map(|figure| self.layout.figure(&figure));
        list.items.extend(pictures.map(|figure| {
            let image = figure.image;
            let rect = Rect::new(image.x - RING, image.y - RING, image.width + 2.0 * RING, image.height + 2.0 * RING);
            Item::Stroke { rect, color: ring, width: 2.0, radius: 2.0 }
        }));
        // A selected picture that stands alone is resized by its corners, which are drawn as its ring is.
        let accent = self.accent.base;
        match (&self.grip.resizing, self.lone_picture()) {
            (Some(resizing), _) => resizing.paint(accent, self.scheme, &mut list),
            (None, Some((_, image))) => figures::handles(image, accent, self.scheme, &mut list),
            (None, None) => {}
        }
        // The line under it offers a caption while it has none, and has the switch to its alt text.
        if let Some((_, hint)) = self.hint() {
            captions::hint(hint, self.caption_text().0, self.shaper.as_ref(), self.scheme, &mut list);
        }
        if let Some((_, _, switch)) = self.alt_switch().filter(|_| self.captions.alt.is_none()) {
            captions::switch(switch, self.captions.over, self.shaper.as_ref(), self.scheme, &mut list);
        }
        // A picture that is carried is faint where it was.
        let carried = self.grip.moving.as_ref().and_then(|moving| Some((moving.carried()?, &moving.figure)));
        let carried = carried.and_then(|(at, figure)| Some((at, self.layout.figure(figure)?.image)));
        if let Some((_, image)) = carried {
            list.items.push(fill(image, Color { a: 178, ..self.palette.page }));
        }
        // A picture that is still a link has its cloud on a chip while it is selected or under the pointer,
        // and while the shell has something to say of it.
        for (figure, badge) in self.layout.linked(viewport) {
            let Some(src) = self.link_of(&figure) else { continue };
            let (shows, state) = (self.linked.shows(&figure, self.editor.picture()), self.linked.said(src));
            if shows.is_some() || state.is_some() {
                pictures::cloud(badge, shows == Some(true), state, self.accent.text, self.scheme, &mut list);
            }
        }
        // The figure a reference under the pointer means, or the one the page was just brought to.
        let lit = self.references.light().and_then(|(figure, way)| Some((self.pictures(figure)?, way)));
        match lit {
            Some(((figure, pictures), Some(way))) => {
                references::light(figure, &pictures, way, self.scheme, self.scale, &mut list);
            }
            Some(((figure, _), None)) => references::mark(figure, self.scheme, &mut list),
            None => {}
        }
        // What stands round the two versions of the conflict that is open, and the one held back.
        let (mut under, mut over) = (DisplayList::default(), DisplayList::default());
        let (cut, shift) = (self.cut(), self.shift());
        let cuts = cut.as_ref().map_or_else(Vec::new, |cut| {
            let shown = Shown {
                shift,
                room,
                now: &self.margin.now,
                choosing: self.margin.refused.is_none(),
                page: self.palette.page,
                scheme: self.scheme,
                shaper: self.shaper.as_ref(),
            };
            self.review.paint(cut, &shown, &mut under, &mut over);
            review::cuts(cut, shift, room)
        });
        self.previewed = self.preview(room, &mut over);
        if let Some((at, image)) = carried {
            self.carry(image, at, &mut over);
        }

        let written = sheet.is_none() && self.editor.picture().is_none();
        let caret = selection.filter(|_| written).and_then(|selection| self.layout.caret(&selection.head));
        let caret = caret.map(|caret| Rect { width: CARET_WIDTH, ..self.onto(caret) });
        let paint = PagePaint {
            list,
            under,
            over,
            origin: Point { x: left, y: TOP - self.scrolled() },
            cuts,
            page: self.palette.page,
            content: self.tall() + 2.0 * TOP,
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
            .on_pointer(|event| Some(Message::Pointer(event)))
            .pointed();
        // A note the caret opened waits behind what the caret itself brings out: a menu of what to insert, a
        // list of figures, the sheet of a formula.
        let brought = || self.insert.offers(&self.editor).is_some() || Refer::offers(&self.editor).is_some();
        let waits = self.margin.follows() && (sheet.is_some() || brought());
        if let Some(importing) = &self.importing {
            element = element.popover(importing.card(room));
        } else if let Some(landing) = &self.landing {
            element = element.popover(Self::card(landing, room));
        } else if let Some(copy) = &self.copying {
            element = element.popover(copy.card(&self.location, room));
        } else if let Some((figure, src)) =
            self.linked.open.as_ref().and_then(|figure| Some((figure, self.link_of(figure)?)))
        {
            // The menu hangs from the cloud, or from the corner of a picture too small to have one.
            let from = self.layout.figure(figure).map_or(Rect::default(), |place| {
                let corner = Rect::new(place.image.x + place.image.width, place.image.y, 0.0, 0.0);
                place.badge.map_or(corner, pictures::chip)
            });
            let spot = self.onto(from);
            let (loaded, said) = (self.images.get(src).is_some(), self.linked.said(src));
            let menu = pictures::menu(src, loaded, said, self.linked.vouched(src), self.safe.on());
            let menu = menu.popover().at(spot).align(Align::End);
            element = element.popover(menu.open(true).on_close(Message::Linked(Linking::Close)));
        } else if let Some(menu) = self.table_menu() {
            element = element.popover(menu);
        } else if let Some(((_, text), (_, line, switch))) = self.captions.alt.as_ref().zip(self.alt_switch()) {
            element = element.popover(captions::sheet(text, self.onto(line), self.onto(switch)));
        } else if let Some((kind, head)) = self.callouts.open.as_ref().and_then(|open| {
            let kind = &self.editor.doc().block(open)?.kind;
            let heads = self.layout.callouts(viewport);
            Some((kind, heads.into_iter().find(|(block, _)| block == open).map(|(_, head)| head)))
        }) {
            // The card hangs under the icon and title, or from the caret when the callout shows neither.
            let spot = head.map(|head| self.onto(callouts::target(head))).or(caret).unwrap_or_default();
            element = element.popover(self.callouts.card(kind, spot).expect("the block is a callout"));
        } else if let Some(context) = &self.context {
            let asked = |act: &Act| self.asked(act);
            let fix = self.spelling.fix.as_ref();
            let about = About { editor: &self.editor, asked: &asked, mac: self.mac, fix, room: room.height };
            element = element.popover(context.menu(&about));
        } else if let Some(address) = &self.link {
            let field = TextField::new("Link", address)
                .placeholder("Paste or type a link")
                .on_input(Message::LinkDraft)
                .on_submit(Message::LinkApply);
            // The address in the field can be tried from here, written down or not.
            let open = Button::icon_only(icon::EXTERNAL_LINK, "Open link")
                .subtle()
                .disabled(address.trim().is_empty())
                .on_press(Message::Safe(Safely::Follow(address.clone())));
            let row = Element::row().align(Align::Center).gap(tokens::SPACE_XS).width(LINK_WIDTH);
            let row = row.child(Element::from(field).grow(1.0)).child(open);
            let editor = Popover::new(row)
                .label("Link")
                .max_width(LINK_WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
                .open(true)
                .on_close(Message::LinkClose);
            // The editor hangs from the caret, in place.
            element = element.popover(editor.at(caret.unwrap_or_default())).cursor(Cursor::Text);
        } else if let Some((note, reply)) = self.margin.reading().filter(|_| !waits) {
            // The note hangs under the last line of its passage. One whose text is gone hangs from the
            // top of the margin its marker stood in.
            let under = self.note_lines(note).map(|(_, last)| self.onto(last));
            let said = self.said.of(note, threads::SAID, self.palette);
            let (now, refused) = (self.margin.now.as_str(), self.margin.refused.as_deref());
            let thread = threads::thread(note, reply, said, now, refused, self.margin.follows());
            element = element.popover(match under {
                Some(under) => thread.at(under),
                None => thread.at(Rect::new(room.width - GUTTER, GUTTER, 0.0, 0.0)).align(Align::End),
            });
        } else if let Some((field, passage)) = self.margin.field().zip(self.margin.writing()) {
            // The field hangs under the last line of the passage the comment is about.
            let lines = notes::stretch(passage).map(|passage| self.layout.selection(&passage)).unwrap_or_default();
            let under = lines.last().copied().unwrap_or_default();
            element = element.popover(field.at(self.onto(under)));
        } else if let Some((line, _, offers)) = self.insert.offers(&self.editor) {
            // The menu hangs from the slash that opened it.
            let slash = self.layout.caret(&Pos::new(line, 0)).unwrap_or_default();
            let spot = self.onto(slash);
            element = element.popover(self.insert.menu(&offers).at(spot));
        } else if let Some((typed, figures)) = Refer::offers(&self.editor) {
            // The list hangs from the start of the words typed for the reference.
            let start = self.layout.caret(&Pos::new(typed.block.clone(), typed.range.start)).unwrap_or_default();
            let spot = self.onto(start);
            element = element.popover(self.refer.list(&self.editor, &typed, &figures).at(spot));
        } else if let Some(((source, rect), selection)) = sheet.zip(selection) {
            let spot = self.onto(rect);
            element =
                element.popover(self.maths.sheet(source, selection, rect.width, self.wide(), self.scheme).at(spot));
        } else if let Some(tools) = self.selection_tools(viewport) {
            element = element.popover(tools);
        }
        // A press on a reference follows it, one on a copy button copies and one on a cloud opens its menu,
        // which the pointer's shape says.
        let pressable = self.references.pointed().is_some() || self.code_copy.pointed() || self.linked.on_cloud();
        let pressable = pressable || self.captions.over || self.margin.over.is_some() || self.review.pressable();
        let pressable = pressable || self.tables.pointed();
        let pressable = pressable || self.callouts.over.is_some();
        match self.grip.corner() {
            Some(corner) => element.cursor(Cursor::Resize(figures::edge(corner))),
            None if pressable => element.cursor(Cursor::Pointer),
            None => element,
        }
    }

    /// Draws the figure of the reference under the pointer small beside it, when it is out of view. `room`
    /// is the size of the page area. Returns what the preview says and where it is.
    fn preview(&self, room: Size, out: &mut DisplayList) -> Option<(String, Rect)> {
        let reference = self.references.previewed()?;
        let rest = self.references.rest()?;
        let picture = self.layout.figure(&reference.target)?.image;
        let caption = references::caption(self.editor.doc(), self.editor.model().style(), &reference.target)?;
        // The preview hangs from the line of the label that the pointer is on.
        let lines = reference.lines(&self.layout);
        let on_page = |line: &Rect| self.onto(*line);
        let anchor = lines.iter().map(on_page).find(|line| line.contains(rest)).or(lines.first().map(on_page))?;
        let page = self.palette.page;
        let preview = Preview { layout: &self.layout, picture, page, caption: &caption, anchor, room };
        let rect = preview.paint(self.shaper.as_ref(), self.scheme, out);
        Some((caption, rect))
    }

    /// How far down the page a block reaches, from its top to its bottom, in the layout's coordinates.
    fn extent(&self, block: &Block) -> Option<(f32, f32)> {
        if let Some(figure) = self.layout.figure(&block.id) {
            let captioned = !block.plain_text().is_empty() || block.body.blocks().iter().any(|part| !is_figure(part));
            let under = if captioned { figure.caption } else { figure.image };
            return Some((figure.image.y, under.y + under.height));
        }
        let mut texts = Vec::new();
        written(block, &mut texts);
        let (first, last) = (texts.first()?, texts.last()?);
        let top = self.layout.caret(&Pos::new(first.id.clone(), 0))?;
        let bottom = self.layout.caret(&Pos::new(last.id.clone(), last.plain_text().len()))?;
        Some((top.y, bottom.y + bottom.height))
    }

    /// Where pictures let go at a point of the page area would land, and the bar that says so, in the
    /// layout's coordinates: beside a picture from its left or right quarter, and otherwise on a line of
    /// their own, above or below the block there.
    fn spot(&self, at: Point) -> Option<(Spot, Option<Rect>)> {
        // Sheets take no pictures, nor does an earlier version: the page does, where it is edited.
        if self.away() {
            return None;
        }
        let at = self.in_layout(at);
        let block = self.layout.hit(at)?.block;
        let (thick, gap) = BAR;
        let picture = self.layout.figure(&block).map(|figure| figure.image).filter(|image| image.contains(at));
        let across = picture.map(|image| (image, (at.x - image.x) / image.width.max(1.0)));
        if let Some((image, across)) = across.filter(|(_, across)| !(0.25..=0.75).contains(across)) {
            let after = across > 0.5;
            let x = if after { image.x + image.width + gap } else { image.x - gap - thick };
            let bar = Rect::new(x, image.y, thick, image.height);
            return Some((Spot::Beside { figure: block, after }, Some(bar)));
        }
        // A block inside a table or a gallery stands for the whole of it.
        let extent = whole(self.editor.doc(), &block).and_then(|whole| self.extent(whole));
        let below = extent.is_some_and(|(top, bottom)| at.y > (top + bottom) / 2.0);
        let bar = extent.map(|(top, bottom)| {
            let y = if below { bottom + gap } else { top - gap - thick };
            Rect::new(0.0, y, self.column.1, thick)
        });
        Some((Spot::Line { block, below }, bar))
    }

    /// The heading the top of the view is under.
    fn section(&self) -> Option<&BlockId> {
        // A heading brought to the top sits just under the page's top margin.
        let top = self.layout.hit(self.in_layout(Point { x: self.column.0, y: TOP + CHAPTER_PITCH.0 }))?;
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
                .fg(if here { accent().base } else { fade(tokens::FG_MUTED, 0.5) })
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
        let chrome = &self.chrome;
        let mut bar = TopBar::new().inset(chrome.inset_start);
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
        if let Some(buttons) = chrome.buttons().filter(|_| chrome.buttons_at_start) {
            bar = bar.before(buttons).before(TopBar::rule());
        }
        // A view that takes the page's place keeps the window's controls and has the rest of the bar.
        bar = match (&self.printing, &self.history) {
            (Some(print), _) => {
                let page = &self.editor.model().style().page;
                let told = self.note.as_ref().map(|(note, _)| note.as_str());
                print.bar(bar, &self.name().0, told, &styles::paper_name(page), || self.styles.setup(page))
            }
            (None, Some(history)) => {
                let told = self.note.as_ref().map(|(note, _)| note.as_str());
                let keys = commands::shortcut(&Act::SaveCopy, self.editor.keymap(), self.mac).unwrap_or_default();
                history.bar(bar, &self.name().0, told, &keys)
            }
            (None, None) => self.editing(bar),
        };
        if let Some(buttons) = chrome.buttons().filter(|_| !chrome.buttons_at_start) {
            bar = bar.end(TopBar::rule()).end(buttons);
        }
        bar.into()
    }

    /// Fills the bar for editing: the document's name, the format controls, and what the window offers.
    fn editing(&self, mut bar: TopBar<Message>) -> TopBar<Message> {
        let share = Element::column()
            .gap(tokens::SPACE_LG)
            .min_width(320.0)
            .child(Element::text("Share", TextStyle::title()))
            .child(Element::from(Button::new("Done").on_press(Message::Share(false))).align_self(Align::End));
        let share = Popover::new(share).label("Share").modal().open(self.sharing).on_close(Message::Share(false));

        let keys = |act: Act| commands::shortcut(&act, self.editor.keymap(), self.mac).unwrap_or_default();
        let more = Menu::new()
            .label("More")
            .item(
                MenuItem::new("Commands")
                    .icon(icon::SEARCH)
                    .aside(keys(Act::Palette))
                    .on_select(Message::Commands(Commanding::Toggle)),
            )
            .separator()
            .item(MenuItem::new("New document").icon(icon::PLUS).on_select(Message::NewDocument))
            .item(MenuItem::new("Open").icon(icon::FOLDER_OPEN).on_select(Message::OpenDialog))
            .separator()
            .item(
                MenuItem::new("Save a copy")
                    .icon(icon::COPY)
                    .aside(keys(Act::SaveCopy))
                    .on_select(Message::Copy(Copying::Open(CopyFormat::Letters))),
            )
            .item(MenuItem::new("Export").icon(icon::DOWNLOAD).on_select(Message::Copy(Copying::Open(CopyFormat::Pdf))))
            .item(
                MenuItem::new("Print view")
                    .icon(icon::PRINTER)
                    .aside(keys(Act::Print))
                    .on_select(Message::Print(Printing::Show)),
            )
            .item(MenuItem::new("Page setup").icon(icon::FILE_TEXT).on_select(Message::Styles(Styling::PageSetup)))
            .separator()
            .item(MenuItem::new("Settings").icon(icon::SETTINGS).on_select(Message::Settings(Setting::Show)));

        // The forms of the bar, from the widest. Once the layout has said which fits, only that one is built.
        let forms = [
            (NAME_WIDTH.2, Fold::None),
            (NAME_WIDTH.1, Fold::None),
            (NAME_WIDTH.1, Fold::Alignment),
            (NAME_WIDTH.1, Fold::Marks),
            (NAME_WIDTH.1, Fold::Style),
        ];
        bar = bar.shown(self.folded.seen.clone());
        for (n, (name, fold)) in forms.into_iter().enumerate() {
            if self.folded.form.is_none_or(|form| form == n) {
                let shown = self.folded.form == Some(n);
                bar = self.lead(name, shown).into_iter().fold(bar.form(n), TopBar::start);
                bar = self.tools(fold).into_iter().fold(bar, TopBar::middle);
            }
        }
        if let Some(badge) = self.safe.badge() {
            bar = bar.end(badge);
        }
        bar = bar.end(Button::new("Share").icon(icon::USERS).small().on_press(Message::Share(true)).popover(share));
        bar = bar.end(TopBar::rule());
        let mut panels = cluster();
        for (panel, icon) in Panel::ALL {
            let button = tool(icon, panel.title()).pressed(self.panel == Some(panel));
            panels = panels.child(button.on_press(Message::Panel(panel)));
        }
        // The history is no panel: it takes the page's place, and lists the versions beside it.
        panels = panels.child(tool(icon::HISTORY, "History").on_press(Message::History(Historying::Show)));
        bar.end(panels.child(tool(icon::ELLIPSIS, "More").popover(more.popover().align(Align::End))))
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
            // What is not kept yet is kept by a press on the mark, which takes no more room for it.
            SaveState::Edited => {
                let save = Element::icon(icon::SAVE, SAVE_ICON)
                    .role(Role::Button)
                    .label("Save")
                    .description("Changes are not saved yet")
                    .tooltip("Save. Changes are not saved yet", Side::Bottom)
                    .on_press(Message::Ask(Request::Save));
                lead.push(save);
                return lead;
            }
        };
        // For a moment after its first write, the icon tells where the document went.
        let note = self.note.as_ref().filter(|_| self.save == SaveState::Saved);
        let said = note.map_or_else(|| said.to_owned(), |(note, _)| note.clone());
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
                .border(Edges::all(2.0), if chosen { accent().base } else { fade(tokens::FG, 0.0) })
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
            Panel::Notes => threads::listed(&self.margin, self.editor.doc()).unwrap_or_else(|| empty("No open notes.")),
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
        let source = Source::of(&self.editor, self.shaper.as_ref(), self.wide())?;
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

    /// Puts what an offer of the insert menu stands for on its line, in place of the slash and what was
    /// typed after it.
    fn put(&mut self, index: usize, platform: &mut dyn Platform) {
        let Some((line, typed, offers)) = self.insert.offers(&self.editor) else { return };
        let Some(put) = offers.get(index).map(|offer| offer.put.clone()) else { return };
        self.insert.close();
        self.editor.select(Selection { anchor: Pos::new(line.clone(), 0), head: Pos::new(line, typed) });
        self.edit(platform, |editor, platform| editor.run(Command::Delete(Motion::Left), platform));
        let command = match put {
            Put::Kind(kind) => Command::SetBlock(kind),
            // A header row over two more, three cells across: enough to start with.
            Put::Table => Command::InsertTable { rows: 3, columns: 3 },
            // The shell lets the person choose them, and they take the place of the line that is now empty.
            Put::Pictures => return self.requests.push(Request::Pictures),
        };
        self.edit(platform, |editor, platform| editor.run(command, platform));
    }

    /// Lets the insert menu have the keys that walk it and take from it, while it shows.
    fn insert_key(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let plain = event.mods == Modifiers::default();
        let Some((_, _, offers)) = self.insert.offers(&self.editor).filter(|_| plain) else { return false };
        match event.key {
            Key::Down | Key::Up => self.insert.pick(event.key == Key::Down, offers.len()),
            Key::Enter | Key::Tab => self.put(self.insert.picked(offers.len()), platform),
            _ => return false,
        }
        true
    }

    /// Lets the list of figures have the keys that walk it and take from it, while it shows. Until a figure
    /// is picked, Enter and Tab are the editor's.
    fn refer_key(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let plain = event.mods == Modifiers::default();
        let Some((typed, figures)) = Refer::offers(&self.editor).filter(|_| plain) else { return false };
        let picked = self.refer.picked(&typed, figures.len()).and_then(|picked| figures.get(picked));
        match (&event.key, picked) {
            (Key::Down | Key::Up, _) => self.refer.pick(&typed, event.key == Key::Down, figures.len()),
            (Key::Enter | Key::Tab, Some(figure)) => self.refer(figure.id.clone(), platform),
            _ => return false,
        }
        true
    }

    /// Makes a reference to a figure of the words typed for one.
    fn refer(&mut self, figure: BlockId, platform: &mut dyn Platform) {
        self.edit(platform, |editor, platform| editor.run(Command::TakeReference(figure), platform));
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
            Styling::PageSetup => {
                self.show(Some(Panel::Styles));
                self.styles.setup = true;
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

impl EditorApp {
    /// Does what a message asks.
    fn does(&mut self, message: Message, platform: &mut dyn Platform) {
        // A note that a press outside it put away is remembered for that press alone.
        if !matches!(message, Message::Note(Noting::Close) | Message::Pointer(Pointer::Down { .. })) {
            self.margin.forget();
        }
        match message {
            Message::Text(event) => {
                let dollar =
                    matches!(&event, InputEvent::Insert(text) | InputEvent::Commit(text) if text.ends_with('$'));
                // Where the caret is and where its block starts, should this close a formula.
                let before = self.editor.selection().filter(|_| dollar && !self.reduced_motion);
                let before = before.map(|selection| selection.head.clone());
                let before = before.map(|head| (self.layout.caret(&Pos::new(head.block.clone(), 0)), head));
                self.unpoint();
                self.edit(platform, |editor, platform| editor.input(event, platform));
                self.maths.typed(true);
                self.insert.typed(&self.editor);
                if let Some((start, head)) = before {
                    self.close(&head, start);
                }
            }
            // The page behind the import card is not written on.
            Message::Pointer(_) if self.importing.is_some() => {}
            Message::Pointer(event) => self.pointer(event, platform),
            Message::Run(command) => {
                // A shell that has the clipboard only later is asked to paste once it has.
                let plain = match command {
                    Command::Paste => Some(false),
                    Command::PastePlain => Some(true),
                    _ => None,
                };
                if let Some(plain) = plain.filter(|_| platform.clipboard_read().is_none()) {
                    return self.requests.push(Request::Paste { plain });
                }
                self.edit(platform, |editor, platform| editor.run(command, platform))
            }
            Message::Panel(panel) => {
                self.margin.now = platform.wall_time().map(notes::stamp).unwrap_or_default();
                self.show((self.panel != Some(panel)).then_some(panel));
            }
            Message::ClosePanel => self.show(None),
            Message::Share(open) => self.sharing = open,
            Message::Correct(text) => {
                self.context = None;
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
            Message::Context(asked) => self.contexting(asked, platform),
            Message::Misspelling { back } => self.misspelling(back),
            Message::Open(path) => self.requests.push(Request::Open(path)),
            Message::OpenDialog => self.requests.push(Request::OpenDialog),
            Message::NewDocument => self.requests.push(Request::New),
            Message::Start => self.start(),
            Message::Rename(message) => self.rename(message),
            Message::Import(message) => {
                let stays = self.importing.as_mut().is_some_and(|card| card.update(message, &mut self.requests));
                self.importing = self.importing.take().filter(|_| stays);
            }
            Message::Copy(message) => self.copy(message),
            Message::Styles(message) => self.styling(message, platform),
            Message::Ask(request) => self.requests.push(request),
            Message::Maths(message) => self.sheet(message, platform),
            Message::Insert(Inserting::Take(index)) => self.put(index, platform),
            Message::Insert(Inserting::Close) => self.insert.close(),
            Message::Refer(Picking::Take(figure)) => self.refer(figure, platform),
            Message::Linked(asked) => self.linking(asked),
            Message::TableMenu => self.tables.open = None,
            Message::Callout(asked) => self.calling(asked, platform),
            Message::Safe(asked) => self.safely(asked),
            Message::Alt(asked) => self.altering(asked, platform),
            Message::Note(asked) => self.noting(asked, platform),
            Message::Commands(asked) => self.commanding(asked, platform),
            Message::Find(asked) => self.finding(asked, platform),
            Message::Print(asked) => self.print(asked, platform),
            Message::History(asked) => self.historying(asked, platform),
            Message::Review(asked) => self.reviewing(asked, platform),
            Message::Refer(Picking::Close) => {
                self.edit(platform, |editor, platform| editor.run(Command::DismissReference, platform));
            }
            Message::Settings(message) => {
                if self.dialog.update(message, &mut self.settings) {
                    self.tint();
                    self.requests.push(Request::Settings(self.settings.clone()));
                }
            }
            Message::Jump(block) => {
                let start = Pos { block, offset: 0 };
                self.editor.select(Selection::caret(start.clone()));
                self.mark();
                // Twice, since laying out what comes into view can move the block.
                for _ in 0..2 {
                    let Some(line) = self.layout.caret(&start) else { break };
                    self.scroll = self.down(line.y).clamp(0.0, self.max_scroll());
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

    /// Does what a key the page was offered asks. Returns whether the key was used.
    fn keyed(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        if self.printing.is_some() {
            return self.print_key(event, platform);
        }
        if self.history.is_some() {
            return self.history_key(event, platform);
        }
        // Print view opens at what is being read. A key the editor is offered brings the caret into view,
        // so this one is not offered: it is none of the editor's.
        if commands::bound(event) == Some(&Act::Print) {
            let asked = self.asked(&Act::Print);
            return asked.map(|message| self.update(message, platform)).is_some();
        }
        // Escape lets go of a picture held by a corner or carried, which stays as it was.
        if event.key == Key::Escape && self.grip.release() {
            self.drop = None;
            return true;
        }
        if self.insert_key(event, platform) || self.refer_key(event, platform) || self.list_key(event, platform) {
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
            // The page may have moved under a pointer that rests where it was.
            self.unpoint();
        }
        if used {
            return true;
        }
        // Escape puts the conflict that is open away, once nothing nearer to the caret took it.
        if event.key == Key::Escape && self.review.note().is_some() {
            self.reviewing(Reviewing::Close, platform);
            return true;
        }
        // The editor leaves some keys to the window: the palette, the link that is edited in place, the
        // settings, saving where it is asked for, and the card that saves a copy.
        let asked = commands::bound(event).and_then(|act| self.asked(act));
        let bound = asked.is_some();
        if let Some(message) = asked {
            self.update(message, platform);
        }
        bound
    }
}

impl App for EditorApp {
    type Msg = Message;

    fn title(&self) -> String {
        self.location.name.clone().unwrap_or_else(|| "Letters".into())
    }

    fn view(&mut self, window: Size) -> Element<Message> {
        // A menu about the page does not wait for the page to come back.
        if self.away() || self.carded() {
            self.context = None;
        }
        // A floating window that Letters frames has a border, which everything else stands inside.
        let border = self.chrome.border();
        let window = Size { width: window.width - 2.0 * border, height: window.height - 2.0 * border };
        self.refold(window.width);
        // Print view has the window to itself: the panels and the outline are for editing. The history
        // has its versions listed where a panel would be.
        let (printing, away) = (self.printing.is_some(), self.away());
        let panel = match &self.history {
            Some(history) => Some(history.panel()),
            None => self.panel.filter(|_| !away).map(|panel| self.side(panel)),
        };
        let room = window.width - if panel.is_some() { side_panel::WIDTH } else { 0.0 };
        // While conflicts wait, a bar over the page counts them and steps through them.
        let waiting = self.margin.conflicts();
        let open = self.margin.reviewing().and_then(|open| waiting.iter().position(|id| *id == open.id));
        let conflicts = (!waiting.is_empty() && !away).then(|| review::bar(waiting.len(), open));
        let told = if conflicts.is_some() { banner::HEIGHT } else { 0.0 };
        // The safe mode bar lies under the top bar and takes its height from the page.
        let bar = self.chrome.bar_height.unwrap_or(top_bar::HEIGHT) + self.safe.height() + told;
        // The history has a strip of its versions under the page and the panel.
        let strip = self.history.as_ref().map(History::strip);
        let foot = if strip.is_some() { history::STRIP } else { 0.0 };
        let room = Size { width: room.max(0.0), height: (window.height - bar - foot).max(0.0) };
        let page = match (printing, away) {
            (true, _) => self.sheets(room),
            (false, true) => self.version(room),
            (false, false) => self.page(room),
        };
        let chapters = if away { None } else { self.chapters() };
        // The find bar hangs under the bars, at the end of the page area.
        let find = (self.find.open() && !away && !self.carded()).then(|| {
            let end = room.width - if chapters.is_some() { chapters_width() } else { 0.0 };
            self.find.view(self.find.current(self.editor.selection()), bar, end, room.width)
        });
        let page = Element::row().grow(1.0).child(page).children(chapters);
        let body = Element::row().grow(1.0).child(page).children(panel);
        // The settings and the card about a link come last, so that they lie over whatever else is out.
        let over = self.dialog.view(&self.settings).into_iter().chain(self.safe.card());
        let commands = self.commands.open().then(|| {
            let (offers, keymap) = (self.offers(), self.editor.keymap());
            self.commands.view(&offers, keymap, self.mac, bar, window)
        });
        let over = over.chain(commands);
        // It comes first, so that whatever else opens lies over it and is the first that Escape puts away.
        let all = Element::column().bg(tokens::BG).fg(tokens::FG).children(find);
        let all = all.child(self.bar()).children(self.safe.bar());
        let all = all.children(conflicts).child(body).children(strip).children(over);
        if border > 0.0 { all.border(Edges::all(border), tokens::LINE_STRONG) } else { all }
    }

    fn update(&mut self, message: Message, platform: &mut dyn Platform) {
        // A press that leaves the caret in a passage asks for its note, wherever the caret was before.
        let pressed = self.dragging && matches!(message, Message::Pointer(Pointer::Up { .. }));
        let was = self.noted().filter(|_| !pressed);
        self.does(message, platform);
        self.attend(was.as_deref(), platform);
    }

    fn key(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let was = self.noted();
        let used = self.keyed(event, platform);
        self.attend(was.as_deref(), platform);
        used
    }

    fn describe(&self, page: &mut AccessNode, nodes: &mut Vec<(NodeId, AccessNode)>, whole: bool) {
        if let Some(print) = &self.printing {
            return print.describe(page, nodes);
        }
        let told = &mut self.told.borrow_mut();
        if let Some(history) = &self.history {
            return history.describe(told, whole, page, nodes);
        }
        describe::page(self.editor.doc(), self.editor.selection(), told, whole, page, nodes);
        if let Some((caption, rect)) = &self.previewed {
            describe::preview(caption, *rect, page, nodes);
        }
        for (marker, at) in self.markers() {
            describe::marker(&marker.block, &marker.label(), self.onto(at), page, nodes);
        }
        if let Some(shown) = self.table() {
            for (part, at) in shown.parts() {
                let (axis, index, does) = match part {
                    tables::Part::Handle(axis, index) => (axis, index, 0),
                    tables::Part::Insert(axis, before) => (axis, before, 2),
                };
                // The plus past the last column or row is told by a cell of the last, as adding at the end.
                let caret = self.caret_cell();
                let within = shown.cell(axis, Some(index), caret).map(|cell| (cell, does));
                let Some((cell, does)) = within.or_else(|| Some((shown.cell(axis, None, caret)?, 1))) else {
                    continue;
                };
                let (told, label) = ((axis == tables::Axis::Row, does), tables::label(part, does == 1));
                describe::table_part(&cell, told, &label, self.onto(at), page, nodes);
            }
        }
        self.review.describe(page, nodes);
    }

    fn access_action(&mut self, request: &ActionRequest, platform: &mut dyn Platform) {
        // The sheets of Print view are read, and all that is changed on them is where pages start.
        if let Some(print) = &self.printing {
            let pressed = match describe::request(self.editor.doc(), request) {
                Some(describe::Request::Break(block)) => print.asked(&block),
                _ => None,
            };
            if let Some(asked) = pressed {
                self.break_pages(asked, platform);
            }
            return;
        }
        // An earlier version is read, and nothing in it takes a press.
        if self.history.is_some() {
            return;
        }
        match describe::request(self.editor.doc(), request) {
            Some(describe::Request::Select(selection)) => {
                let was = self.noted();
                self.editor.select(selection);
                self.mark();
                self.reveal();
                self.attend(was.as_deref(), platform);
            }
            Some(describe::Request::Replace(text)) => {
                self.edit(platform, |editor, platform| editor.input(InputEvent::Insert(text), platform));
            }
            Some(describe::Request::Copy(block)) => self.copy_code(block, platform),
            Some(describe::Request::Note(block)) => self.noting(Noting::Press(block), platform),
            Some(describe::Request::Table { cell, row, does }) => {
                // The caret goes to the cell, and its table then shows the handles of its column and row.
                self.editor.select(Selection::caret(Pos::new(cell, 0)));
                self.mark();
                let axis = if row { tables::Axis::Row } else { tables::Axis::Column };
                let Some((at, shown)) = self.editor.cell().zip(self.table()) else { return };
                let index = if row { at.row } else { at.column };
                let asked = match does {
                    0 => tables::Asked::Menu(axis, index),
                    1 => tables::Asked::Insert { axis, before: index + 1 },
                    _ => tables::Asked::Insert { axis, before: index },
                };
                self.tabled(&shown, asked, platform);
            }
            Some(describe::Request::Callout(block)) => self.call(&block, platform),
            Some(describe::Request::Alt(figure)) => {
                self.editor.select_picture(&figure);
                self.mark();
                self.reveal();
                self.alter(figure);
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
            Some(describe::Request::Choose { block, keep }) => {
                if let Some(asked) = self.review.asked(&block, keep) {
                    self.reviewing(asked, platform);
                }
            }
            Some(describe::Request::Break(_)) | None => {}
        }
    }

    fn text(&mut self, event: InputEvent, platform: &mut dyn Platform) {
        // With the menu about a misspelled word open, a number takes the correction of that place. A
        // browser keeps that number with Ctrl for its tabs.
        if let (Some(_), Some(fix), InputEvent::Insert(typed)) = (&self.context, &self.spelling.fix, &event) {
            let place = typed.parse::<usize>().ok().and_then(|place| place.checked_sub(1));
            if let Some(word) = place.and_then(|place| fix.suggestions.get(place)).cloned() {
                return self.update(Message::Correct(word), platform);
            }
        }
        // The first character typed over the landing card starts this document with it.
        let typed = matches!(event, InputEvent::Insert(_) | InputEvent::Commit(_) | InputEvent::SetComposition { .. });
        if self.landing.is_some() && self.importing.is_none() && typed {
            self.start();
            self.edit(platform, |editor, platform| editor.input(event, platform));
        }
    }

    fn keys(&mut self, keys: Keys) {
        self.mac = keys == Keys::Mac;
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
        self.tint();
    }

    fn accent(&self) -> Accent {
        self.accent
    }

    fn reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
        self.review.still = reduced;
        self.history.iter_mut().for_each(|history| history.still = reduced);
        self.references.set_reduced_motion(reduced);
    }

    fn point(&mut self, at: Option<Point>) -> bool {
        if let Some(print) = self.printing.as_mut() {
            return print.point(at);
        }
        if let Some(history) = self.history.as_mut() {
            return history.point(at);
        }
        // The page has nothing of its own under the version held back, nor under the buttons.
        let moved = self.review.point(at);
        self.references.set_rest(at.filter(|at| !self.review.covers(*at)));
        self.repoint() || moved
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
        let until = self.note.as_mut().map(|(_, from)| *from.get_or_insert(now_ms) + SAVED_NOTE_MS);
        let note = until.filter(|until| now_ms < *until);
        if note.is_none() {
            self.note = None;
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
        let light = self.references.animate(now_ms);
        let copied = self.code_copy.animate(now_ms);
        let slid = self.review.animate(now_ms);
        let brought = self.history.as_mut().and_then(|history| history.animate(now_ms));
        [note, slide, fade, light, copied, slid, brought].into_iter().flatten().min()
    }

    fn idle(&mut self) -> Idle {
        // The version that shows is laid out before the page behind it.
        if self.history.as_mut().is_some_and(History::settle) {
            return Idle::Unseen;
        }
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
    /// What lies under the page's text, placed in the page area: what stands round the two versions of a
    /// conflict, and the version held back.
    under: DisplayList,
    /// What hangs over the page, placed in the page area: the preview of a figure.
    over: DisplayList,
    /// Where the layout's origin is inside the page area, while the page shows whole.
    origin: Point,
    /// The parts a conflict cuts the page into, when one is open: what each is cut to in the page area,
    /// and where the layout's origin is for it.
    cuts: Vec<(Rect, Point)>,
    page: Color,
    /// The height of everything that scrolls, and how far it is scrolled.
    content: f32,
    scroll: f32,
}

impl Paint for PagePaint {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        out.push(fill(rect, self.page));
        let first = out.items.len();
        out.items.extend(self.under.items.iter().cloned());
        out.translate(first, Point { x: rect.x, y: rect.y });
        let whole = [(rect, self.origin)];
        // Each part of a page that is cut shows what the layout painted, moved to where the part stands.
        for (part, origin) in if self.cuts.is_empty() { &whole[..] } else { &self.cuts[..] } {
            let cut = !self.cuts.is_empty();
            if cut {
                out.push(Item::PushClip {
                    rect: Rect { x: rect.x + part.x, y: rect.y + part.y, ..*part },
                    radius: 0.0,
                });
            }
            let first = out.items.len();
            out.items.extend(self.list.items.iter().cloned());
            out.translate(first, Point { x: rect.x + origin.x, y: rect.y + origin.y });
            if cut {
                out.push(Item::PopClip);
            }
        }
        out.items.extend(scrollbar(rect, self.content, self.scroll, scheme, 1.0));
        let first = out.items.len();
        out.items.extend(self.over.items.iter().cloned());
        out.translate(first, Point { x: rect.x, y: rect.y });
    }
}

fn fill(rect: Rect, color: Color) -> Item {
    Item::Fill { rect, color, radius: 0.0 }
}

fn is_figure(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Figure { .. })
}

/// The blocks of text in `block`, itself among them, in the order of the document.
fn written<'a>(block: &'a Block, out: &mut Vec<&'a Block>) {
    if matches!(block.body, Body::Text(_)) {
        out.push(block);
    }
    block.body.blocks().iter().for_each(|part| written(part, out));
}

/// The block of the document's own that `id` is in, or is.
fn whole<'a>(doc: &'a Document, id: &BlockId) -> Option<&'a Block> {
    fn holds(block: &Block, id: &BlockId) -> bool {
        &block.id == id || block.body.blocks().iter().any(|part| holds(part, id))
    }
    doc.blocks.iter().find(|block| holds(block, id))
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
