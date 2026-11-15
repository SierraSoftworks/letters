//! The winit event loop: windows, their input, painting and saving.
//!
//! Each window holds one document. Everything a window shows comes from its view as a display list, which
//! is rasterised into a software surface. Saving happens on a thread per save, a moment after typing stops,
//! and nobody is asked where: a new document gets a file of its own once it has content. What is typed
//! goes into a journal sooner than that, so that it outlives a crash.

use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use letters_editor::{Editor, InputEvent, Key, KeyEvent, Platform as _};
use letters_format::WorkingCopy;
use letters_model::{Model, ReplicaId, Revision, Style};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Point, Size};
use letters_spell::Dictionary;
use letters_trust::Trust;
// The editor's window is the view here, where `Window` is the system's.
use letters_ui::{Chrome, Cursor, Edge, Landing, Location, Request, SaveState, Scheme, Window as View, tokens};
use rfd::{FileDialog, MessageDialog, MessageLevel};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::error::EventLoopError;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{self, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, ResizeDirection, Theme, Window, WindowAttributes, WindowId};

use crate::chrome::{self, Buttons, Host};
use crate::cli::{Mode, Options};
use crate::dictionaries;
use crate::files::{self, EXTENSION, Exchange, Home, Opened, Rest, STYLESHEET, Working};
use crate::gpu::Gpu;
use crate::input::{self, Composer, Press};
use crate::journal::{Journal, Journals};
use crate::library::{Library, Place, Recent, Refused, UNTITLED, has_content};
use crate::measure::{FIRST_FRAME, Launch, Typing};
use crate::platform::{self, Desktop};
use crate::present::{Painter, Presenter};
use crate::trust::{self, Trusted};
use crate::words::{self, Words};

const MAC: bool = cfg!(target_os = "macos");
/// How long typing must pause before a save starts.
const SAVE_AFTER: Duration = Duration::from_millis(800);
/// How long after an edit the journal has it, however long the typing goes on.
const JOURNAL_AFTER: Duration = Duration::from_millis(300);
/// How long to wait before trying a failed save again.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// The gap between the keystrokes of a typing report, long enough that no frame waits on the one before.
const TYPING_GAP: Duration = Duration::from_millis(20);
/// How many recent documents the landing card is handed. It shows the first few.
const LISTED: usize = 12;
/// The distance one notch of a mouse wheel scrolls, in logical pixels.
const WHEEL_LINE: f32 = 40.0;
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1100.0, 800.0);
/// What a file dialog calls the files it lists for the Styles panel.
const STYLESHEETS: &str = "Letters stylesheet";

/// What reaches the event loop from outside winit.
pub enum Event {
    Access(accesskit_winit::Event),
    /// A window's save has finished, and its result is ready to collect.
    Saved(WindowId),
    /// What a window's first paint did not need has been read.
    Read(WindowId),
    /// The search for a GPU has ended, with one or without.
    Gpu(Option<Box<Gpu>>),
    /// The desktop has said whether it wants less motion.
    Motion(bool),
    /// The desktop has said which buttons its windows carry.
    Buttons(Buttons),
    /// A system dictionary a window's document asked for has been read, under the name it has on disk.
    Dictionary(WindowId, String, Box<Dictionary>),
}

impl From<accesskit_winit::Event> for Event {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Access(event)
    }
}

/// Runs the app until its last window closes.
pub fn run(options: Options, mut launch: Launch) -> Result<(), Box<dyn Error>> {
    // The file is read while the event loop connects to the display.
    let reading = options.file.clone().map(|path| thread::spawn(move || files::open(&path, new_style)));
    let report_launch = options.mode == Mode::LaunchReport;
    let event_loop = event_loop(report_launch)?;
    let home = std::env::home_dir().unwrap_or_default();
    launch.mark("event loop built");
    let mut app = App {
        starting: true,
        report_launch,
        typing: match options.mode {
            Mode::TypingReport { keys } => Some(TypingRun { keys, stats: Typing::default(), typed: None, next: None }),
            _ => None,
        },
        launch: Some(launch),
        reading,
        path: options.file,
        proxy: event_loop.create_proxy(),
        platform: Desktop::new().with_words(Words::of(&home)),
        library: Library::of(&home),
        words: Words::of(&home),
        trust: trust::store(&home),
        journals: Journals::of(&home),
        dictionaries: dictionaries::system_folders(&home).into(),
        reduced_motion: false,
        buttons: Buttons::guess(),
        gpu: match std::env::var("LETTERS_RENDERER").as_deref() {
            Ok("cpu") => GpuState::Off,
            Ok("gpu") => GpuState::Wanted { software: true },
            _ => GpuState::Wanted { software: false },
        },
        windows: HashMap::new(),
        mods: ModifiersState::empty(),
        failure: None,
    };
    event_loop.run_app(&mut app)?;
    app.failure.map_or(Ok(()), |failure| Err(failure.into()))
}

/// Builds the event loop. A `background` app does not come to the front, so a run that only measures takes
/// no keystrokes from whatever the user is typing into.
pub(crate) fn event_loop<T: 'static>(background: bool) -> Result<EventLoop<T>, EventLoopError> {
    let mut builder = EventLoop::<T>::with_user_event();
    #[cfg(target_os = "macos")]
    if background {
        use winit::platform::macos::EventLoopBuilderExtMacOS;
        builder.with_activate_ignoring_other_apps(false);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = background;
    builder.build()
}

/// What every window starts as, document or not. Its bar is its title bar where the desktop leaves that
/// to the app: under the system's lights on macOS, and in a frame Letters draws on Linux.
pub(crate) fn window_attributes() -> WindowAttributes {
    let attributes = Window::default_attributes()
        .with_title(UNTITLED)
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(LogicalSize::new(360.0, 240.0));
    #[cfg(target_os = "macos")]
    let attributes = {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attributes.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
    };
    attributes.with_decorations(Host::HERE != Host::Linux)
}

/// Opens the document and paints into memory, as [`run`] would into a window, then prints the report the
/// mode asks for. Returns whether the typing budget was met: the launch budget is for `--bench-launch`.
pub fn run_headless(options: &Options, mut launch: Launch) -> Result<bool, Box<dyn Error>> {
    let (document, style) = match &options.file {
        Some(path) => {
            let opened = files::open(path, new_style).map_err(|error| format!("{}: {error}", path.display()))?;
            (opened.document, opened.style)
        }
        None => (files::blank(), new_style()),
    };
    launch.mark("document read");
    // A window of the default size on a display like the reference machine's.
    let scale = 2.0;
    let (width, height) = ((WINDOW_SIZE.width * scale) as u32, (WINDOW_SIZE.height * scale) as u32);
    let replica = ReplicaId::new(&Desktop::new().new_id());
    let mut view = View::new(Editor::with_model(Model::new(document, replica).with_style(style)));
    view.resize(Size { width: WINDOW_SIZE.width as f32, height: WINDOW_SIZE.height as f32 }, scale as f32);
    launch.mark("laid out");
    let mut list = DisplayList::default();
    view.paint(&mut list);
    launch.mark("display list");
    let mut pixels = vec![0; (width * height) as usize];
    let mut painter = Painter::new();
    painter.paint(&list, &Images::default(), scale as f32, background(false), width, 0, &mut pixels);
    launch.mark(FIRST_FRAME);
    let Mode::TypingReport { keys } = options.mode else {
        println!("{FIRST_FRAME}\n{}", launch.report());
        return Ok(true);
    };

    let mut platform = Desktop::new();
    let mut typing = Typing::default();
    for _ in 0..keys {
        let typed = Instant::now();
        view.input(InputEvent::Insert("a".into()), &mut platform);
        list.items.clear();
        view.paint(&mut list);
        // The pixels still hold the frame before, as a window's buffer does.
        painter.paint(&list, &Images::default(), scale as f32, background(false), width, 1, &mut pixels);
        typing.record(typed.elapsed());
    }
    print!("{}", typing.report("CPU"));
    Ok(typing.within_budget())
}

/// What a window is cleared to before the view paints over all of it.
fn background(dark: bool) -> Color {
    let ink = tokens::BG.resolve(if dark { Scheme::Dark } else { Scheme::Light });
    Color::rgba(ink.r, ink.g, ink.b, ink.a)
}

/// Whether two opened documents are one file.
fn same_file(a: &Opened, b: &Opened) -> bool {
    let file = |opened: &Opened| opened.home.as_ref().and_then(|home| fs::canonicalize(home.path()).ok());
    file(a).is_some_and(|a| Some(a) == file(b))
}

/// The style of a new document.
fn new_style() -> Style {
    files::new_style(std::env::home_dir().as_deref())
}

struct App {
    /// Whether the first window is still to open.
    starting: bool,
    report_launch: bool,
    /// The typing report in progress, in that mode.
    typing: Option<TypingRun>,
    /// The stopwatch, until the first frame is on screen.
    launch: Option<Launch>,
    reading: Option<JoinHandle<Result<Opened, files::Error>>>,
    path: Option<PathBuf>,
    proxy: EventLoopProxy<Event>,
    platform: Desktop,
    library: Library,
    words: Words,
    trust: Trusted,
    journals: Journals,
    /// Where the system keeps dictionaries for languages other than English.
    dictionaries: Arc<[PathBuf]>,
    /// Whether the desktop asked for less motion. Asked once, after the first frame.
    reduced_motion: bool,
    /// The buttons the desktop's windows carry: a guess until it has been asked, after the first frame.
    buttons: Buttons,
    gpu: GpuState,
    windows: HashMap<WindowId, DocWindow>,
    mods: ModifiersState,
    /// Why the app is exiting, when it is not by choice.
    failure: Option<String>,
}

/// A typing report: the app types into its own window and times each keystroke to its frame.
struct TypingRun {
    keys: u32,
    stats: Typing,
    /// When the keystroke now waiting for its frame was typed.
    typed: Option<Instant>,
    /// When to type the next one. `None` until the window has its first frame and its final renderer.
    next: Option<Instant>,
}

/// Where the GPU stands. Windows paint on the CPU in every state but the last.
enum GpuState {
    /// Not to be used: `LETTERS_RENDERER=cpu`, or none was found.
    Off,
    /// To be looked for once the first frame is on screen. `software` accepts an adapter with no hardware.
    Wanted {
        software: bool,
    },
    Starting,
    Ready(Box<Gpu>),
}

struct DocWindow {
    window: Arc<Window>,
    presenter: Presenter,
    access: accesskit_winit::Adapter,
    view: View,
    list: DisplayList,
    composer: Composer,
    pointer: Point,
    cursor: Cursor,
    /// Whether the window has been offered the GPU. It is offered once, after its first frame.
    gpu_offered: bool,
    /// Whether the accessibility tree is behind the view.
    access_stale: bool,
    /// The frame the view was last told to draw.
    chrome: Chrome,
    /// When the view next wants a frame without being asked, on the editor's clock.
    wake_ms: Option<u64>,

    /// Where the document is saved, once it has somewhere.
    place: Place,
    /// Whether the document is saved at all. A run that only measures leaves every file as it found it.
    keeps: bool,
    /// The path the recents list has for this document.
    listed: Option<PathBuf>,
    /// The name and folder the view was last told, so it is told again only when they change.
    located: Option<(Option<String>, PathBuf)>,
    /// The person's own dictionary as this window last read it.
    words_seen: words::Stamp,
    /// Whether the primary button went down as a secondary one, as it does with Control held on a Mac.
    menu_press: bool,
    /// Whether the document was made in this window, which is what makes it trusted from the start.
    made_here: bool,
    /// The id of a document made here that has just been written for the first time, for the app to
    /// record as trusted wherever its file goes.
    vouch: Option<String>,
    /// The document's id, once its file's manifest has been read or made.
    id: Option<String>,
    /// Whether the view was told that formulas may link, which a trusted document's may.
    maths_links: bool,
    /// The languages a system dictionary has been looked for, each once whether or not there was one.
    asked: BTreeSet<String>,
    /// The view's edit count that is safely on disk.
    saved_edits: u64,
    /// The edit count a save was last planned for.
    noticed_edits: u64,
    save_at: Option<Instant>,
    /// The save that is running, with the edit count and the editor's revision it started from.
    saving: Option<(u64, Revision, JoinHandle<Saving>)>,
    save_failed: bool,

    /// What the first paint did not need from the file, until reading it starts.
    rest: Option<Rest>,
    reading: Option<JoinHandle<Result<Working, String>>>,
    /// What saves are written from. `None` while it is being read, and while a save has it.
    copy: Option<WorkingCopy>,
    /// The editor's revision that the copy has seen all of.
    sent: Option<Revision>,
    /// The style the copy was last given, which tells a change made here from one a save merged in.
    sent_style: Style,

    /// The document's journal, once its history is read. `None` too when another window has it.
    journal: Option<Journal>,
    /// The editor's revision the journal has seen all of. `None` while it has seen nothing of a new
    /// document, whose whole history it is given first.
    journalled: Option<Revision>,
    /// The view's edit count the journal was last given.
    journalled_edits: u64,
    journal_at: Option<Instant>,
}

/// A save on its way back from its thread: the copy, and how the save ended.
struct Saving {
    copy: WorkingCopy,
    result: Result<Exchange, String>,
}

/// Something a key asks of the app, not of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    New,
    /// Opens the document at a path, or one the person picks.
    Open(Option<PathBuf>),
    Save,
    Close,
    Quit,
}

fn shortcut(event: &KeyEvent) -> Option<Action> {
    if !event.mods.primary || event.mods.alt || event.mods.shift {
        return None;
    }
    Some(match event.key {
        Key::Char('n') => Action::New,
        Key::Char('o') => Action::Open(None),
        Key::Char('s') => Action::Save,
        Key::Char('w') => Action::Close,
        Key::Char('q') => Action::Quit,
        _ => return None,
    })
}

impl DocWindow {
    fn name(&self) -> String {
        self.place.name()
    }

    fn unsaved(&self) -> bool {
        self.keeps && self.view.edits() != self.saved_edits
    }

    fn dark(&self) -> bool {
        self.window.theme() == Some(Theme::Dark)
    }

    /// Where the document's file is, once it has been written. A journal builds on the file it is told of.
    fn file(&self) -> Option<&Path> {
        self.place.home().filter(|home| home.written()).map(Home::path)
    }

    /// Whether the journal holds what no save has found in the file, which one more save settles.
    fn owes(&self) -> bool {
        self.journal.as_ref().is_some_and(Journal::owes)
    }

    /// Gives the journal what was edited since it was last given anything.
    fn record(&mut self) {
        self.journal_at = None;
        let Some(journal) = &mut self.journal else { return };
        if self.view.edits() == self.journalled_edits {
            return;
        }
        let model = self.view.editor().model();
        journal.edits(self.journalled.as_ref().map_or_else(|| model.save(), |seen| model.updates_since(seen)));
        (self.journalled, self.journalled_edits) = (Some(model.revision()), self.view.edits());
    }

    /// Tells the journal where the document's file is, which may have changed.
    fn rehome(&mut self) {
        let Some(mut journal) = self.journal.take() else { return };
        if let Some(file) = self.file() {
            journal.moved(file);
        }
        self.journal = Some(journal);
    }

    /// Lets the journal go as the window closes with everything saved. A document that was never worth
    /// a file is not worth bringing back either.
    fn leave(&mut self) {
        if let Some(mut journal) = self.journal.take()
            && self.file().is_none()
        {
            journal.forget();
        }
    }

    /// The top bar says whether the document is saved. The title adds what the bar has no word for: a
    /// save that failed.
    fn retitle(&mut self) {
        self.view.set_save_state(match (self.save_failed, self.place.home().is_some(), self.unsaved()) {
            (true, _, _) => SaveState::Failed,
            // A document with no file yet has nothing to say about saving.
            (false, false, _) => SaveState::Unsaved,
            (false, true, true) => SaveState::Saving,
            (false, true, false) => SaveState::Saved,
        });
        let state = if self.save_failed { " · Not saved" } else { "" };
        self.window.set_title(&format!("{}{state}", self.name()));
    }

    /// Tells the view what the document is called and where it is kept, when either has changed. A
    /// document with no name of its own is told so, and the view shows its first heading meanwhile.
    fn relocate(&mut self, library: &Library) {
        let name = (!self.place.naming()).then(|| self.place.name());
        let folder = self.place.folder(library);
        if self.located.as_ref().is_some_and(|(told, at)| (told, at) == (&name, &folder)) {
            return;
        }
        let shown = |folder: crate::library::Folder| {
            Some(letters_ui::Folder { path: folder.path.to_str()?.to_owned(), name: folder.name, depth: folder.depth })
        };
        self.view.set_location(Location {
            name: name.clone(),
            folder: folder.to_string_lossy().into_owned(),
            folders: library.folders(&folder).into_iter().filter_map(shown).collect(),
        });
        self.located = Some((name, folder));
    }

    /// Renames or moves the document, or makes a folder for it, as the bar asked, and answers the bar:
    /// where the document now is, or why it is not there. Returns whether it was done.
    fn replace(
        &mut self,
        library: &Library,
        now: Instant,
        change: impl FnOnce(&mut Place, &Library) -> Result<(), Refused>,
    ) -> bool {
        // The file is at rest while it moves, so a save that is running is waited for.
        self.finish_save(library, now);
        let done = change(&mut self.place, library);
        match &done {
            Ok(()) => {
                self.rehome();
                self.relist(library);
                // Told even when nothing changed, since the bar waits for its answer.
                self.located = None;
                self.relocate(library);
            }
            Err(refused) => self.view.set_notice(Some(&refused.to_string())),
        }
        self.retitle();
        done.is_ok()
    }

    /// Tells the view how far the document is trusted, which for now decides whether its formulas may
    /// link. One made in this window is trusted. One that was opened is unknown until its id is read.
    fn retrust(&mut self, trust: &Trust) {
        let mode = match (&self.id, self.place.path()) {
            _ if self.made_here => letters_trust::Mode::Trusted,
            (Some(id), path) => trust::mode(trust, id, path),
            (None, _) => letters_trust::Mode::Safe,
        };
        let links = mode == letters_trust::Mode::Trusted;
        if std::mem::replace(&mut self.maths_links, links) != links {
            self.view.set_maths_links(links);
        }
    }

    /// Looks for the system's dictionaries for the languages the document names and has not asked about
    /// yet, on another thread. One that is found arrives as an event. One that is not is no one's concern.
    fn ask_dictionaries(&mut self, proxy: &EventLoopProxy<Event>, folders: &Arc<[PathBuf]>) {
        let mut wanted = dictionaries::languages(self.view.editor().doc());
        wanted.retain(|language| self.asked.insert(language.clone()));
        if wanted.is_empty() {
            return;
        }
        let (id, proxy, folders) = (self.window.id(), proxy.clone(), folders.clone());
        thread::spawn(move || {
            for (name, dictionary) in wanted.iter().filter_map(|language| dictionaries::load(&folders, language)) {
                let _ = proxy.send_event(Event::Dictionary(id, name, Box::new(dictionary)));
            }
        });
    }

    /// Reads the person's own words again, if their dictionary changed since this window last looked.
    fn reword(&mut self, words: &Words) {
        if let Some(words) = words.changed(&mut self.words_seen) {
            self.view.set_user_words(words);
        }
    }

    /// Tells the recents list where the document is, when that is news to it.
    fn relist(&mut self, library: &Library) {
        let news = |path: &&Path| self.keeps && Some(*path) != self.listed.as_deref() && path.is_file();
        let Some(path) = self.place.path().filter(news).map(Path::to_owned) else { return };
        library.moved(self.listed.as_deref().unwrap_or(&path), &path, &platform::now_rfc3339());
        self.listed = Some(path);
    }

    /// Finds where a save goes, giving a new document its file and its name when they are due. `None` when
    /// there is nothing to write yet, or nowhere to write it, which is tried again.
    ///
    /// With the `caret` counted, a first Heading 1 it is still in does not name the document yet.
    fn settle(&mut self, caret: bool, library: &Library, now: Instant) -> Option<Home> {
        let editor = self.view.editor();
        let caret = editor.selection().filter(|_| caret).map(|selection| &selection.head.block);
        let home = self.place.settle(editor.doc(), caret, library).map(|home| home.cloned());
        match &home {
            // A document with nothing in it has nothing to lose. A title still being typed is waited for.
            Ok(None) if !has_content(editor.doc()) => {
                (self.saved_edits, self.save_failed) = (self.view.edits(), false);
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("letters: could not save {}: {error}", self.name());
                self.save_failed = true;
                self.save_at = Some(now + RETRY_AFTER);
            }
        }
        self.retitle();
        self.relocate(library);
        // Taking its name may have moved the file.
        self.rehome();
        home.ok().flatten()
    }

    /// Does what the bar asks of the window itself, as the title bar it stands in for would.
    fn frame(&self, request: &Request) {
        let window = &self.window;
        match request {
            // A desktop that will not move or resize a window this way leaves it where it is.
            Request::DragWindow => drop(window.drag_window()),
            Request::ResizeWindow(edge) => drop(window.drag_resize_window(match edge {
                Edge::North => ResizeDirection::North,
                Edge::NorthEast => ResizeDirection::NorthEast,
                Edge::East => ResizeDirection::East,
                Edge::SouthEast => ResizeDirection::SouthEast,
                Edge::South => ResizeDirection::South,
                Edge::SouthWest => ResizeDirection::SouthWest,
                Edge::West => ResizeDirection::West,
                Edge::NorthWest => ResizeDirection::NorthWest,
            })),
            Request::WindowMenu(at) => window.show_window_menu(LogicalPosition::new(at.x, at.y)),
            Request::Minimise => window.set_minimized(true),
            Request::ToggleMaximise => window.set_maximized(!window.is_maximized()),
            _ => {}
        }
    }

    /// Tells the view what frame the desktop expects around it, when that has changed: the window's
    /// buttons, and whether it floats or fills its place.
    fn reframe(&mut self, buttons: &Buttons) {
        let (maximized, full_screen) = (self.window.is_maximized(), self.window.fullscreen().is_some());
        let chrome = Host::HERE.chrome(buttons, maximized, full_screen);
        if self.chrome != chrome {
            self.chrome = chrome.clone();
            self.view.set_chrome(chrome);
        }
    }

    /// Fits the view to the window, which may have changed size, display or theme.
    fn fit(&mut self) {
        self.view.set_dark(self.dark());
        let scale = self.window.scale_factor();
        let size = self.window.inner_size().to_logical::<f32>(scale);
        self.view.resize(Size { width: size.width, height: size.height }, scale as f32);
    }

    /// Paints one frame and puts it on screen.
    fn redraw(&mut self, now_ms: u64) -> Result<(), softbuffer::SoftBufferError> {
        // Idle work waits behind this frame, and the view asks to be called again for it.
        self.wake_ms = self.view.animate(now_ms);
        self.list.items.clear();
        self.view.paint(&mut self.list);
        self.presenter.present(&self.list, background(self.dark()))
    }

    /// Brings everything outside the canvas up to date with the view, after any event.
    fn sync(&mut self, now: Instant) {
        if self.view.needs_paint() {
            self.window.request_redraw();
        }
        if let Some(caret) = self.view.caret_rect() {
            let position = LogicalPosition::new(caret.x, caret.y);
            self.window.set_ime_cursor_area(position, LogicalSize::new(caret.width.max(1.0), caret.height));
        }
        let cursor = self.view.cursor();
        if cursor != self.cursor {
            self.cursor = cursor;
            self.window.set_cursor(match cursor {
                Cursor::Default => CursorIcon::Default,
                Cursor::Pointer => CursorIcon::Pointer,
                Cursor::Text => CursorIcon::Text,
                Cursor::Resize(edge) => match edge {
                    Edge::North => CursorIcon::NResize,
                    Edge::NorthEast => CursorIcon::NeResize,
                    Edge::East => CursorIcon::EResize,
                    Edge::SouthEast => CursorIcon::SeResize,
                    Edge::South => CursorIcon::SResize,
                    Edge::SouthWest => CursorIcon::SwResize,
                    Edge::West => CursorIcon::WResize,
                    Edge::NorthWest => CursorIcon::NwResize,
                },
            });
        }
        if std::mem::take(&mut self.access_stale) {
            self.access.update_if_active(|| self.view.access());
        }
        if self.view.edits() != self.noticed_edits {
            self.noticed_edits = self.view.edits();
            self.save_at = Some(now + SAVE_AFTER);
            self.journal_at.get_or_insert(now + JOURNAL_AFTER);
            self.retitle();
        }
    }

    /// Starts reading what the first paint did not need, on another thread.
    fn start_read(&mut self, proxy: &EventLoopProxy<Event>, journals: &Journals) {
        let Some(rest) = self.rest.take() else { return };
        let (id, proxy, replica) = (self.window.id(), proxy.clone(), self.view.editor().model().replica());
        let journals = self.keeps.then(|| journals.folder().to_owned());
        self.reading = Some(thread::spawn(move || {
            let working = rest.read(replica, journals.as_deref()).map_err(|error| error.to_string());
            let _ = proxy.send_event(Event::Read(id));
            working
        }));
    }

    /// Collects the working copy and gives the editor the file's history, reading here if that had not
    /// started and waiting if it had not finished. Fails once, when the file could not be read for saving.
    fn finish_read(&mut self, journals: &mut Journals) -> Result<(), String> {
        let working = match (self.reading.take(), self.rest.take()) {
            (Some(reading), _) => reading.join().unwrap_or_else(|_| Err("reading it stopped unexpectedly".into())),
            (None, Some(rest)) => {
                let replica = self.view.editor().model().replica();
                rest.read(replica, self.keeps.then(|| journals.folder())).map_err(|error| error.to_string())
            }
            (None, None) => return Ok(()),
        };
        match working {
            Ok(Working { copy, history, news, claim, journalled }) => {
                if let Some(history) = history {
                    let _ = self.view.change(|editor| Ok::<_, Infallible>(editor.attach(history)));
                }
                if let Some(news) = news
                    && let Err(error) = self.view.change(|editor| editor.merge(&news))
                {
                    eprintln!("letters: could not bring back unsaved work in {}: {error}", self.name());
                }
                self.id = Some(copy.id.clone());
                if let Some(claim) = claim {
                    let journal = journals.open(claim, &copy.id, &copy.created, self.file());
                    // What an earlier session left unsaved is saved now.
                    if journal.owes() {
                        self.save_at = Some(Instant::now());
                    }
                    (self.journal, self.journalled) = (Some(journal), journalled);
                    // What was typed before the history arrived is typed all the same.
                    self.record();
                }
                self.copy = Some(copy);
                Ok(())
            }
            Err(error) => {
                self.save_failed = self.keeps;
                self.retitle();
                Err(format!("{} is open, but changes to it cannot be saved.\n\n{error}", self.name()))
            }
        }
    }

    /// What the copy needs to catch up with the editor: the edits it has not seen, and the revision they
    /// lead to. The copy takes the style too, when it was changed here.
    fn edits_for(&mut self, copy: &mut WorkingCopy) -> (Vec<u8>, Revision) {
        if self.view.style() != &self.sent_style {
            self.sent_style = self.view.style().clone();
            // The window only offers styles the file format takes.
            let _ = copy.set_style(self.sent_style.clone());
        }
        let model = self.view.editor().model();
        let edits = self.sent.as_ref().map_or_else(|| model.save(), |sent| model.updates_since(sent));
        (edits, model.revision())
    }

    /// Starts saving on another thread, unless a save is running or there is nothing to save.
    fn start_save(&mut self, proxy: &EventLoopProxy<Event>, library: &Library, now: Instant) {
        // A save waits for the one before it, and for the file's history.
        if self.saving.is_some() || self.copy.is_none() {
            return;
        }
        self.save_at = None;
        if !self.keeps {
            return;
        }
        // The journal has everything a save is handed, and a new document's before it is given a file.
        self.record();
        let Some(home) = self.settle(true, library, now) else { return };
        if !self.unsaved() && !self.owes() {
            // Only the name was due.
            return self.relist(library);
        }
        let Some(mut copy) = self.copy.take() else { return };
        if let Some(journal) = &mut self.journal {
            journal.saving();
        }
        let (edits, revision) = self.edits_for(&mut copy);
        let (id, proxy, since) = (self.window.id(), proxy.clone(), revision.clone());
        let save = thread::spawn(move || {
            let result = home.exchange(&mut copy, &edits, &since).map_err(|error| error.to_string());
            let _ = proxy.send_event(Event::Saved(id));
            Saving { copy, result }
        });
        self.saving = Some((self.view.edits(), revision, save));
    }

    /// Collects a running save, waiting for it if it has not finished.
    fn finish_save(&mut self, library: &Library, now: Instant) {
        let Some((edits, revision, save)) = self.saving.take() else { return };
        // A save with nothing to carry was there to read the file back for the journal.
        let carried = edits != self.saved_edits || self.sent.is_none();
        let result = match save.join() {
            Ok(Saving { copy, result }) => {
                self.copy = Some(copy);
                result
            }
            Err(_) => Err("the save stopped unexpectedly".into()),
        };
        match result {
            Ok(exchange) => {
                self.saved(exchange, edits, revision, library);
                // A file that was written is read back once things are quiet, which lets the journal go.
                // That is tried once: a file that keeps changing is read again with the next edit.
                let again = self.unsaved() || (carried && self.owes());
                if again && self.save_at.is_none() {
                    self.save_at = Some(now + SAVE_AFTER);
                }
            }
            Err(error) => {
                eprintln!("letters: could not save {}: {error}", self.name());
                self.unheld();
                self.save_failed = true;
                self.save_at = Some(now + RETRY_AFTER);
            }
        }
        self.retitle();
    }

    /// Tells the journal of a save that failed, which leaves it the only place the edits are.
    fn unheld(&mut self) {
        if let Some(journal) = &mut self.journal {
            journal.saved(false);
        }
    }

    /// Notes a save that worked: the document as it was at `edits` and `revision` is in its file, and
    /// `exchange` says where that is and what the file held from other writers.
    fn saved(&mut self, exchange: Exchange, edits: u64, revision: Revision, library: &Library) {
        let Exchange { home, news, held } = exchange;
        self.place.saved(home);
        self.rehome();
        if let Some(journal) = &mut self.journal {
            journal.saved(held);
        }
        // The first write is to a folder nobody chose, so the bar names it for a moment.
        let first = self.keeps && self.listed.is_none();
        self.relist(library);
        if first && let Some(path) = &self.listed {
            self.view.set_saved_in(&library.shown(path.parent().unwrap_or(Path::new(""))));
            self.vouch = self.id.clone().filter(|_| self.made_here);
        }
        self.relocate(library);
        self.sent = Some(revision);
        self.save_failed = false;
        let typed_since = self.view.edits() != edits;
        if let Some(news) = news
            && let Err(error) = self.view.change(|editor| editor.merge(&news))
        {
            eprintln!("letters: could not take in what {} held: {error}", self.name());
        }
        // What came from the file is in the file, so it is no edit to save.
        self.saved_edits = if typed_since { edits } else { self.view.edits() };
        if !typed_since {
            self.noticed_edits = self.saved_edits;
        }
    }

    /// Saves to `home` on this thread, with the copy the window keeps for it.
    fn exchange(&mut self, home: &Home, copy: &mut WorkingCopy, library: &Library) -> Result<(), String> {
        if let Some(journal) = &mut self.journal {
            journal.saving();
        }
        let (edits, revision) = self.edits_for(copy);
        match home.exchange(copy, &edits, &revision) {
            Ok(exchange) => self.saved(exchange, self.view.edits(), revision, library),
            Err(error) => {
                self.unheld();
                return Err(error.to_string());
            }
        }
        Ok(())
    }

    /// Saves now, on this thread, if anything is unsaved. Fails with what to tell the writer.
    ///
    /// With the `caret` counted, a first Heading 1 it is still in does not name the file yet.
    fn save_now(&mut self, caret: bool, library: &Library, journals: &mut Journals) -> Result<(), String> {
        if !self.keeps {
            return Ok(());
        }
        let now = Instant::now();
        self.finish_read(journals)?;
        self.finish_save(library, now);
        // A copy that could not be read was reported when that failed, and holds nothing back now.
        let Some(mut copy) = self.copy.take() else { return Ok(()) };
        self.record();
        let home = self.settle(caret, library, now).filter(|_| self.unsaved() || self.owes());
        let saved = home.map(|home| self.exchange(&home, &mut copy, library));
        // A file that was just written is read back, which lets the journal go. A journal that stays is
        // looked at when the app next starts.
        if matches!(saved, Some(Ok(())))
            && self.owes()
            && let Some(home) = self.place.home().cloned()
        {
            let _ = self.exchange(&home, &mut copy, library);
        }
        self.copy = Some(copy);
        match saved {
            Some(Ok(())) => self.save_at = None,
            Some(Err(error)) => {
                self.save_failed = true;
                self.retitle();
                return Err(format!("{} could not be saved.\n\n{error}", self.name()));
            }
            // Nothing to write, though a name may have been due. A place that could not be made is retried.
            None if self.save_failed => return Err(format!("{} could not be saved.", self.name())),
            None => self.relist(library),
        }
        self.retitle();
        Ok(())
    }

    fn alert(&self, message: &str) {
        alert(Some(&self.window), message);
    }

    /// Saves and says so if that fails. Returns whether everything is on disk.
    fn save_or_say(&mut self, caret: bool, library: &Library, journals: &mut Journals) -> bool {
        let saved = self.save_now(caret, library, journals);
        if let Err(error) = &saved {
            self.alert(error);
        }
        saved.is_ok()
    }
}

fn alert(parent: Option<&Arc<Window>>, message: &str) {
    eprintln!("letters: {message}");
    let dialog = MessageDialog::new().set_level(MessageLevel::Error).set_title("Letters").set_description(message);
    match parent {
        Some(window) => dialog.set_parent(&**window).show(),
        None => dialog.show(),
    };
}

impl App {
    /// Opens a window on a document. With no `opened`, the document is blank and has no file yet, and
    /// `landing` puts the card with the recent documents over it.
    fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        opened: Option<Opened>,
        landing: bool,
    ) -> Result<(), Box<dyn Error>> {
        // A document with no file was made in this app, though perhaps in a session that ended early.
        let made_here = opened.as_ref().is_none_or(|opened| opened.home.is_none());
        let (document, style, place, rest) = match opened {
            Some(Opened { document, style, home, rest }) => {
                (document, style, home.map_or_else(Place::new, Place::at), rest)
            }
            None => {
                let (document, style) = (files::blank(), new_style());
                let rest = Rest::new(&document, &style);
                (document, style, Place::new(), rest)
            }
        };
        // AccessKit must attach before the window is first shown.
        let window = Arc::new(event_loop.create_window(window_attributes().with_visible(false))?);
        let access = accesskit_winit::Adapter::with_event_loop_proxy(event_loop, &window, self.proxy.clone());
        let presenter = Presenter::new(window.clone())?;
        window.set_ime_allowed(true);
        self.mark("window created");

        let replica = ReplicaId::new(&self.platform.new_id());
        let mut window = DocWindow {
            sent_style: style.clone(),
            // The editor's history starts from the style on screen, as the working copy's does.
            view: View::new(Editor::with_model(Model::new(document, replica).with_style(style))),
            window,
            presenter,
            access,
            list: DisplayList::default(),
            composer: Composer::default(),
            pointer: Point::default(),
            cursor: Cursor::Default,
            gpu_offered: false,
            access_stale: false,
            chrome: Chrome::default(),
            wake_ms: None,
            place,
            keeps: self.typing.is_none() && !self.report_launch,
            listed: None,
            located: None,
            words_seen: None,
            menu_press: false,
            made_here,
            vouch: None,
            id: None,
            maths_links: false,
            asked: BTreeSet::new(),
            saved_edits: 0,
            noticed_edits: 0,
            save_at: None,
            saving: None,
            save_failed: false,
            rest: Some(rest),
            reading: None,
            copy: None,
            sent: None,
            journal: None,
            journalled: None,
            journalled_edits: 0,
            journal_at: None,
        };
        window.view.set_reduced_motion(self.reduced_motion);
        if !window.place.naming() {
            window.view.set_document_name(&window.place.name());
        }
        if landing && made_here && window.keeps {
            window.view.set_landing(Some(self.landing()));
        }
        window.retrust(self.trust.trust());
        window.retitle();
        window.reframe(&self.buttons);
        window.fit();
        self.mark("laid out");
        window.window.set_visible(true);
        self.mark("window shown");
        window.window.request_redraw();
        self.windows.insert(window.window.id(), window);
        Ok(())
    }

    /// What the landing card offers: the documents opened lately, with times on the person's own clock.
    fn landing(&self) -> Landing {
        let recent = |recent: Recent| {
            Some(letters_ui::Recent {
                name: recent.name(),
                folder: self.library.shown(recent.path.parent()?),
                opened: platform::local(&recent.opened),
                path: recent.path.to_str()?.to_owned(),
            })
        };
        Landing {
            now: platform::local(&platform::now_rfc3339()),
            recents: self.library.recents().into_iter().filter_map(recent).take(LISTED).collect(),
        }
    }

    /// Does what a window asked of the app since it was last looked at.
    fn serve(&mut self, event_loop: &ActiveEventLoop, id: WindowId, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let mut refused = false;
        let requests = window.view.take_requests();
        let served = !requests.is_empty();
        for request in requests {
            match request {
                Request::Open(path) => self.act(event_loop, id, Action::Open(Some(path.into()))),
                Request::OpenDialog => self.act(event_loop, id, Action::Open(None)),
                Request::New => self.act(event_loop, id, Action::New),
                // The name is taken on the way to a save, which reads it from the document.
                Request::Named(_) => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.save_at = Some(now);
                    }
                }
                // What was asked along with something refused is left for the person to ask again.
                Request::Rename(_) | Request::Move(_) | Request::MoveElsewhere | Request::NewFolder { .. }
                    if refused => {}
                Request::Rename(name) => {
                    refused = !self.replace(id, now, |place, library| place.rename(&name, library))
                }
                Request::Move(folder) => refused = !self.replace(id, now, |place, _| place.move_to(Path::new(&folder))),
                Request::MoveElsewhere => {
                    let Some(window) = self.windows.get(&id) else { continue };
                    let dialog = FileDialog::new().set_directory(window.place.folder(&self.library));
                    if let Some(folder) = dialog.set_parent(&*window.window).pick_folder() {
                        refused = !self.replace(id, now, |place, _| place.move_to(&folder));
                    }
                }
                Request::NewFolder { parent, name } => {
                    let make = |library: &Library| library.new_folder(Path::new(&parent), &name).map(drop);
                    refused = !self.replace(id, now, |_, library| make(library));
                }
                Request::Close => self.act(event_loop, id, Action::Close),
                Request::DragWindow
                | Request::ResizeWindow(_)
                | Request::WindowMenu(_)
                | Request::Minimise
                | Request::ToggleMaximise => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.frame(&request);
                        window.reframe(&self.buttons);
                    }
                }
                Request::SaveDefaultStyle(json) => self.keep_style(id, &json),
                Request::ImportStyle => {
                    let Some(window) = self.windows.get_mut(&id) else { continue };
                    let dialog = FileDialog::new().add_filter(STYLESHEETS, &[STYLESHEET]);
                    let Some(path) = dialog.set_parent(&*window.window).pick_file() else { continue };
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    // The window says so itself when the text is no stylesheet.
                    match fs::read(&path) {
                        Ok(text) => window.view.offer_stylesheet(&name, &String::from_utf8_lossy(&text)),
                        Err(error) => window.alert(&format!("{name} could not be read.\n\n{error}")),
                    }
                }
                Request::ExportStyle(json) => {
                    let Some(window) = self.windows.get(&id) else { continue };
                    let dialog = FileDialog::new()
                        .add_filter(STYLESHEETS, &[STYLESHEET])
                        .set_directory(window.place.folder(&self.library))
                        .set_file_name(format!("{}.{STYLESHEET}", window.name()));
                    if let Some(path) = dialog.set_parent(&*window.window).save_file()
                        && let Err(error) = files::write_stylesheet(&path, &json)
                    {
                        window.alert(&format!("The stylesheet could not be written.\n\n{error}"));
                    }
                }
                // A request this shell does not know yet.
                _ => {}
            }
        }
        // An answer may have changed what the window shows.
        if served && let Some(window) = self.windows.get_mut(&id) {
            window.access_stale = true;
            window.sync(now);
        }
    }

    /// Keeps a style as the person's default, for every window to offer from now on. The window that
    /// asked is told the old one again if the new one could not be kept.
    fn keep_style(&mut self, id: WindowId, json: &str) {
        let kept = files::keep_style(&std::env::home_dir().unwrap_or_default(), json);
        let default = new_style();
        for (other, window) in &mut self.windows {
            if kept.is_err() || *other != id {
                window.view.set_default_style(Some(default.clone()));
            }
        }
        if let (Err(error), Some(window)) = (kept, self.windows.get(&id)) {
            window.alert(&format!("Your default style could not be kept.\n\n{error}"));
        }
    }

    /// Has a window rename or move its document. Returns whether it was done.
    fn replace(
        &mut self,
        id: WindowId,
        now: Instant,
        change: impl FnOnce(&mut Place, &Library) -> Result<(), Refused>,
    ) -> bool {
        let Some(window) = self.windows.get_mut(&id) else { return false };
        let done = window.replace(&self.library, now, change);
        // A trusted folder vouches for what is in it, so a move can change how the document is treated.
        window.retrust(self.trust.trust());
        done
    }

    /// The documents that sessions which ended early left unsaved. A run that only measures looks for
    /// none, and leaves every journal as it found it.
    fn left_unsaved(&self) -> Vec<Opened> {
        if self.typing.is_some() || self.report_launch {
            return Vec::new();
        }
        self.journals.orphans().into_iter().filter_map(files::reopen).collect()
    }

    fn mark(&mut self, stage: &'static str) {
        if let Some(launch) = &mut self.launch {
            launch.mark(stage);
        }
    }

    /// Opens a file in a new window, or reports why it cannot be opened. Returns whether it opened.
    fn open_path(&mut self, event_loop: &ActiveEventLoop, path: &Path, parent: Option<WindowId>) -> bool {
        let parent = parent.and_then(|id| self.windows.get(&id)).map(|window| window.window.clone());
        let opened = files::open(path, new_style).map_err(|error| error.to_string());
        let shown =
            opened.and_then(|opened| self.open_window(event_loop, Some(opened), false).map_err(|e| e.to_string()));
        if let Err(error) = &shown {
            alert(parent.as_ref(), &format!("{} could not be opened.\n\n{error}", path.display()));
        }
        shown.is_ok()
    }

    fn act(&mut self, event_loop: &ActiveEventLoop, id: WindowId, action: Action) {
        match action {
            Action::New => {
                if let Err(error) = self.open_window(event_loop, None, false) {
                    alert(None, &format!("A new window could not be opened.\n\n{error}"));
                }
            }
            Action::Open(path) => {
                let Some(window) = self.windows.get(&id) else { return };
                let pick = || {
                    let dialog = FileDialog::new().add_filter("Letters document", &[EXTENSION]);
                    dialog.set_directory(self.library.default_folder()).set_parent(&*window.window).pick_file()
                };
                let Some(path) = path.or_else(pick) else { return };
                // A window with no file and nothing to put in one gives way to the document that was asked for.
                let pristine = window.place.home().is_none() && !has_content(window.view.editor().doc());
                match self.open_path(event_loop, &path, Some(id)) {
                    true if pristine => {
                        if let Some(mut window) = self.windows.remove(&id) {
                            window.leave();
                        }
                    }
                    true => {}
                    // A recent document that has gone is no longer offered.
                    false => {
                        let landing = self.landing();
                        if let Some(window) = self.windows.get_mut(&id).filter(|window| window.view.landing()) {
                            window.view.set_landing(Some(landing));
                        }
                    }
                }
            }
            Action::Save => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.save_or_say(true, &self.library, &mut self.journals);
                }
            }
            Action::Close => self.close(event_loop, id),
            Action::Quit => {
                for id in self.windows.keys().copied().collect::<Vec<_>>() {
                    self.close(event_loop, id);
                }
            }
        }
    }

    /// Records the documents made here that have just been written, so that they stay trusted wherever
    /// their files go.
    fn vouch(&mut self) {
        for id in self.windows.values_mut().filter_map(|window| window.vouch.take()) {
            if let Err(error) = self.trust.trust_document(&id) {
                eprintln!("letters: could not record a new document as trusted: {error}");
            }
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        // Nothing is asked at closing: the document is saved, and stays open only if that fails.
        let (library, journals) = (&self.library, &mut self.journals);
        let saved = self.windows.get_mut(&id).is_some_and(|window| window.save_or_say(false, library, journals));
        self.vouch();
        if saved && let Some(mut window) = self.windows.remove(&id) {
            window.leave();
        }
        if self.windows.is_empty() {
            event_loop.exit();
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, failure: String) {
        self.failure = Some(failure);
        event_loop.exit();
    }
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // A desktop may resume more than once. Only the first opens the window.
        if !std::mem::take(&mut self.starting) {
            return;
        }
        let opened = match self.reading.take().map(|reading| reading.join().expect("reading does not panic")) {
            Some(Ok(opened)) => Ok(Some(opened)),
            Some(Err(error)) => {
                let path = self.path.as_deref().unwrap_or(Path::new(""));
                Err(format!("{} could not be opened.\n\n{error}", path.display()))
            }
            None => Ok(None),
        };
        // The document was read long before the desktop finished starting the app.
        self.mark("app launched");
        let shown = opened.and_then(|opened| {
            // What sessions that ended early left unsaved comes back first, and the document that was
            // asked for may be among it.
            let mut documents: Vec<_> = self.left_unsaved().into_iter().map(Some).collect();
            let asked = opened.filter(|opened| !documents.iter().flatten().any(|kept| same_file(kept, opened)));
            if asked.is_some() || documents.is_empty() {
                documents.push(asked);
            }
            documents.into_iter().try_for_each(|opened| {
                let landing = opened.is_none();
                self.open_window(event_loop, opened, landing)
                    .map_err(|error| format!("No window could be opened.\n\n{error}"))
            })
        });
        if let Err(failure) = shown {
            if !self.report_launch {
                alert(None, &failure);
            }
            self.fail(event_loop, failure);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Self { windows, platform, mods, library, .. } = self;
        let Some(window) = windows.get_mut(&id) else { return };
        window.access.process_event(&window.window, &event);
        let mut action = None;
        match event {
            WindowEvent::RedrawRequested => {
                if let Err(error) = window.redraw(platform.now_ms()) {
                    return self.fail(event_loop, format!("the window could not be painted: {error}"));
                }
                if let Some(mut launch) = self.launch.take() {
                    launch.mark(FIRST_FRAME);
                    if self.report_launch {
                        println!("{FIRST_FRAME}\n{}", launch.report());
                        return event_loop.exit();
                    }
                    // What the person trusts is in a file, which the first frame did not need either.
                    if let Err(error) = self.trust.load() {
                        eprintln!("letters: nothing is trusted, since {error}");
                    }
                    // The desktop is asked over D-Bus, which is not worth holding a frame for.
                    let proxy = self.proxy.clone();
                    thread::spawn(move || {
                        if let Some(reduced) = chrome::reduced_motion() {
                            let _ = proxy.send_event(Event::Motion(reduced));
                        }
                        if let Some(buttons) = Buttons::ask().filter(|_| Host::HERE == Host::Linux) {
                            let _ = proxy.send_event(Event::Buttons(buttons));
                        }
                    });
                }
                if let Some(run) = &mut self.typing
                    && let Some(typed) = run.typed.take()
                {
                    run.stats.record(typed.elapsed());
                    if run.stats.len() as u32 == run.keys {
                        let renderer = if window.presenter.on_gpu() { "GPU" } else { "CPU" };
                        print!("{}", run.stats.report(renderer));
                        if !run.stats.within_budget() {
                            self.failure = Some("typing is over its budget".into());
                        }
                        return event_loop.exit();
                    }
                    run.next = Some(Instant::now() + TYPING_GAP);
                }
                // A frame is on screen, so the GPU can now be brought in without holding anything up.
                match &mut self.gpu {
                    GpuState::Wanted { software } => {
                        let (software, proxy) = (*software, self.proxy.clone());
                        thread::spawn(move || proxy.send_event(Event::Gpu(Gpu::start(software).map(Box::new))));
                        self.gpu = GpuState::Starting;
                    }
                    GpuState::Ready(gpu) if !std::mem::replace(&mut window.gpu_offered, true) => {
                        window.presenter.use_gpu(gpu);
                        window.window.request_redraw();
                    }
                    _ => {}
                }
                // Off the path to the first frame: the file's history and the person's own words are read,
                // and the recents list learns of an opened document.
                if window.rest.is_some() {
                    window.reword(&self.words);
                    window.view.set_default_style(Some(new_style()));
                    window.start_read(&self.proxy, &self.journals);
                    window.ask_dictionaries(&self.proxy, &self.dictionaries);
                    window.relocate(library);
                }
                window.relist(library);
                return;
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::ThemeChanged(_) => {
                // A window that was maximised or let go again changed its size on the way.
                window.reframe(&self.buttons);
                window.fit()
            }
            WindowEvent::Focused(focused) => {
                window.view.focus(focused);
                // Another window, or another copy of the app, may have taught the dictionary a word.
                if focused && window.rest.is_none() {
                    window.reword(&self.words);
                }
                window.access_stale = true;
            }
            WindowEvent::ModifiersChanged(modifiers) => *mods = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !window.composer.active() =>
            {
                // The Menu key and Shift+F10 ask for the menu at the caret.
                let menu = match event.logical_key {
                    keyboard::Key::Named(NamedKey::ContextMenu) => true,
                    keyboard::Key::Named(NamedKey::F10) => mods.shift_key(),
                    _ => false,
                };
                if menu {
                    window.view.menu_key(platform);
                    window.access_stale = true;
                    let now = Instant::now();
                    window.sync(now);
                    return self.serve(event_loop, id, now);
                }
                let unmodified = event.key_without_modifiers();
                let press = Press {
                    key: &event.logical_key,
                    unmodified: &unmodified,
                    text: event.text.as_deref(),
                    mods: *mods,
                };
                let translated = input::translate(press, MAC);
                let used = translated.key.clone().is_some_and(|key| window.view.key(key, platform));
                action = translated.key.as_ref().filter(|_| !used).and_then(shortcut);
                if let Some(text) = translated.text.filter(|_| !used && action.is_none()) {
                    window.view.input(InputEvent::Insert(text), platform);
                }
                window.access_stale = true;
            }
            WindowEvent::Ime(ime) => {
                if let Some(event) = window.composer.event(&ime) {
                    window.view.input(event, platform);
                    window.access_stale = true;
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let position = position.to_logical::<f32>(window.window.scale_factor());
                window.pointer = Point { x: position.x, y: position.y };
                window.view.pointer_move(window.pointer, platform);
                window.access_stale |= window.view.needs_paint();
            }
            WindowEvent::CursorLeft { .. } => window.view.pointer_leave(),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                match state {
                    // On a Mac a press with Control held asks for the menu, as the secondary button does.
                    ElementState::Pressed if MAC && mods.control_key() => {
                        window.menu_press = true;
                        window.view.secondary_down(window.pointer, platform);
                    }
                    ElementState::Pressed => {
                        window.view.pointer_down(window.pointer, input::modifiers(*mods, MAC), platform)
                    }
                    ElementState::Released if std::mem::take(&mut window.menu_press) => {}
                    ElementState::Released => window.view.pointer_up(window.pointer, platform),
                }
                window.access_stale = true;
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => {
                window.view.secondary_down(window.pointer, platform);
                window.access_stale = true;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // winit reports how far the content should move. The view wants how far to scroll.
                let delta = match delta {
                    MouseScrollDelta::LineDelta(x, y) => Point { x: -x * WHEEL_LINE, y: -y * WHEEL_LINE },
                    MouseScrollDelta::PixelDelta(delta) => {
                        let delta = delta.to_logical::<f32>(window.window.scale_factor());
                        Point { x: -delta.x, y: -delta.y }
                    }
                };
                window.view.scroll(window.pointer, delta, platform);
            }
            WindowEvent::CloseRequested => action = Some(Action::Close),
            _ => {}
        }
        let now = Instant::now();
        window.sync(now);
        self.serve(event_loop, id, now);
        if let Some(action) = action {
            self.act(event_loop, id, action);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        let now = Instant::now();
        match event {
            Event::Saved(id) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.finish_save(&self.library, now);
                    // Another writer's edits may have come in with it, in a language of their own.
                    window.ask_dictionaries(&self.proxy, &self.dictionaries);
                    // Another writer's edits may have come in with it.
                    window.access_stale = true;
                    window.sync(now);
                }
            }
            Event::Read(id) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                if let Err(error) = window.finish_read(&mut self.journals) {
                    match window.keeps {
                        true => window.alert(&error),
                        false => eprintln!("letters: {error}"),
                    }
                }
                // The file has said which document it is, so what the person trusts can be asked about it.
                window.retrust(self.trust.trust());
                window.access_stale = true;
                window.sync(now);
            }
            Event::Gpu(None) => self.gpu = GpuState::Off,
            Event::Dictionary(id, name, dictionary) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.view.add_dictionary(&name, *dictionary);
                    window.access_stale = true;
                    window.sync(now);
                }
            }
            Event::Motion(reduced) => {
                self.reduced_motion = reduced;
                for window in self.windows.values_mut() {
                    window.view.set_reduced_motion(reduced);
                }
            }
            Event::Buttons(buttons) => {
                self.buttons = buttons;
                for window in self.windows.values_mut() {
                    window.reframe(&self.buttons);
                    window.access_stale = true;
                    window.sync(now);
                }
            }
            Event::Gpu(Some(mut gpu)) => {
                for window in self.windows.values_mut() {
                    window.gpu_offered = true;
                    window.presenter.use_gpu(&mut gpu);
                    window.window.request_redraw();
                }
                self.gpu = GpuState::Ready(gpu);
            }
            Event::Access(accesskit_winit::Event { window_id, window_event }) => {
                let Some(window) = self.windows.get_mut(&window_id) else { return };
                match window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => window.access_stale = true,
                    accesskit_winit::WindowEvent::ActionRequested(request) => {
                        window.view.access_action(request, &mut self.platform);
                        window.access_stale = true;
                    }
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => {}
                }
                window.sync(now);
                self.serve(event_loop, window_id, now);
            }
        }
    }

    /// The last chance to save, which is the only one when the system ends the app, as Quit in the macOS
    /// menu does. Nothing may ask a question here.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        for window in self.windows.values_mut() {
            match window.save_now(false, &self.library, &mut self.journals) {
                Ok(()) => window.leave(),
                Err(error) => eprintln!("letters: {error}"),
            }
            // What could not be saved stays in its journal, which closes here.
            window.journal = None;
        }
        self.vouch();
        self.journals.finish();
    }

    /// Runs after each batch of events: starts due saves and sleeps until the next thing is due.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let now_ms = self.platform.now_ms();
        self.vouch();
        let mut next: Option<Instant> = None;
        for window in self.windows.values_mut() {
            if window.wake_ms.is_some_and(|wake| wake <= now_ms) {
                window.wake_ms = window.view.animate(now_ms);
                window.sync(now);
            }
            if window.journal_at.is_some_and(|at| at <= now) {
                window.record();
            }
            if window.save_at.is_some_and(|at| at <= now) {
                // What was written since the last save may be in a language not seen before.
                window.ask_dictionaries(&self.proxy, &self.dictionaries);
                window.start_save(&self.proxy, &self.library, now);
            }
            if let Some(run) = &mut self.typing {
                // Typing starts once the window is on the renderer it will stay on.
                let read = window.rest.is_none() && window.reading.is_none();
                let settled =
                    read && self.launch.is_none() && (window.gpu_offered || matches!(self.gpu, GpuState::Off));
                if settled && run.typed.is_none() && run.next.is_none_or(|next| next <= now) {
                    run.typed = Some(Instant::now());
                    run.next = None;
                    window.view.input(InputEvent::Insert("a".into()), &mut self.platform);
                    window.sync(now);
                }
            }
            let wake = window.wake_ms.map(|ms| self.platform.instant(ms));
            // A save that is due but waiting on another needs no timer: that save's end wakes the loop.
            let save = window.save_at.filter(|_| window.saving.is_none() && window.copy.is_some());
            let type_next = self.typing.as_ref().and_then(|run| run.next);
            for due in [wake, save, window.journal_at, type_next].into_iter().flatten() {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
