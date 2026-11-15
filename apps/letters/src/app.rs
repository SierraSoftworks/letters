//! The winit event loop: windows, their input, painting and saving.
//!
//! Each window holds one document. Everything a window shows comes from its view as a display list, which
//! is rasterised into a software surface. Saving happens on a thread per save, a moment after typing stops,
//! and nobody is asked where: a new document gets a file of its own once it has content.

use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::error::Error;
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
use letters_ui::{Cursor, Landing, Location, Request, SaveState, Scheme, Window as View, tokens};
use rfd::{FileDialog, MessageDialog, MessageLevel};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::error::EventLoopError;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{self, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, Theme, Window, WindowAttributes, WindowId};

use crate::chrome;
use crate::cli::{Mode, Options};
use crate::dictionaries;
use crate::files::{self, EXTENSION, Home, Opened, Rest, Working};
use crate::gpu::Gpu;
use crate::input::{self, Composer, Press};
use crate::library::{Library, Place, Recent, Refused, UNTITLED, has_content};
use crate::measure::{FIRST_FRAME, Launch, Typing};
use crate::platform::{self, Desktop};
use crate::present::{Painter, Presenter};
use crate::trust::{self, Trusted};
use crate::words::{self, Words};

const MAC: bool = cfg!(target_os = "macos");
/// How long typing must pause before a save starts.
const SAVE_AFTER: Duration = Duration::from_millis(800);
/// How long to wait before trying a failed save again.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// The gap between the keystrokes of a typing report, long enough that no frame waits on the one before.
const TYPING_GAP: Duration = Duration::from_millis(20);
/// How many recent documents the landing card is handed. It shows the first few.
const LISTED: usize = 12;
/// The distance one notch of a mouse wheel scrolls, in logical pixels.
const WHEEL_LINE: f32 = 40.0;
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1100.0, 800.0);

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
        dictionaries: dictionaries::system_folders(&home).into(),
        reduced_motion: false,
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

/// What every window starts as, document or not.
pub(crate) fn window_attributes() -> WindowAttributes {
    Window::default_attributes()
        .with_title(UNTITLED)
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(LogicalSize::new(360.0, 240.0))
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
    /// Where the system keeps dictionaries for languages other than English.
    dictionaries: Arc<[PathBuf]>,
    /// Whether the desktop asked for less motion. Asked once, after the first frame.
    reduced_motion: bool,
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
}

/// A save on its way back from its thread: the copy, and the home with what the file held from others.
struct Saving {
    copy: WorkingCopy,
    result: Result<(Home, Option<Vec<u8>>), String>,
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
        home.ok().flatten()
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
        self.view.animate(now_ms);
        self.list.items.clear();
        self.view.paint(&mut self.list);
        // The view puts off its idle work while a frame is owed, and says when to call again only once
        // none is. So it is asked again as soon as this frame is out.
        self.wake_ms = Some(now_ms);
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
            });
        }
        if std::mem::take(&mut self.access_stale) {
            self.access.update_if_active(|| self.view.access());
        }
        if self.view.edits() != self.noticed_edits {
            self.noticed_edits = self.view.edits();
            self.save_at = Some(now + SAVE_AFTER);
            self.retitle();
        }
    }

    /// Starts reading what the first paint did not need, on another thread.
    fn start_read(&mut self, proxy: &EventLoopProxy<Event>) {
        let Some(rest) = self.rest.take() else { return };
        let (id, proxy, replica) = (self.window.id(), proxy.clone(), self.view.editor().model().replica());
        self.reading = Some(thread::spawn(move || {
            let working = rest.read(replica).map_err(|error| error.to_string());
            let _ = proxy.send_event(Event::Read(id));
            working
        }));
    }

    /// Collects the working copy and gives the editor the file's history, reading here if that had not
    /// started and waiting if it had not finished. Fails once, when the file could not be read for saving.
    fn finish_read(&mut self) -> Result<(), String> {
        let working = match (self.reading.take(), self.rest.take()) {
            (Some(reading), _) => reading.join().unwrap_or_else(|_| Err("reading it stopped unexpectedly".into())),
            (None, Some(rest)) => rest.read(self.view.editor().model().replica()).map_err(|error| error.to_string()),
            (None, None) => return Ok(()),
        };
        match working {
            Ok(Working { copy, history }) => {
                if let Some(history) = history {
                    let _ = self.view.change(|editor| Ok::<_, Infallible>(editor.attach(history)));
                }
                self.id = Some(copy.id.clone());
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
        let Some(home) = self.settle(true, library, now) else { return };
        if !self.unsaved() {
            // Only the name was due.
            return self.relist(library);
        }
        let Some(mut copy) = self.copy.take() else { return };
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
        let result = match save.join() {
            Ok(Saving { copy, result }) => {
                self.copy = Some(copy);
                result
            }
            Err(_) => Err("the save stopped unexpectedly".into()),
        };
        match result {
            Ok((home, news)) => {
                self.saved(home, news, edits, revision, library);
                if self.unsaved() && self.save_at.is_none() {
                    self.save_at = Some(now + SAVE_AFTER);
                }
            }
            Err(error) => {
                eprintln!("letters: could not save {}: {error}", self.name());
                self.save_failed = true;
                self.save_at = Some(now + RETRY_AFTER);
            }
        }
        self.retitle();
    }

    /// Notes a save that worked: the document as it was at `edits` and `revision` is in its file at `home`,
    /// and `news` is what the file held from other writers.
    fn saved(&mut self, home: Home, news: Option<Vec<u8>>, edits: u64, revision: Revision, library: &Library) {
        self.place.saved(home);
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

    /// Saves now, on this thread, if anything is unsaved. Fails with what to tell the writer.
    ///
    /// With the `caret` counted, a first Heading 1 it is still in does not name the file yet.
    fn save_now(&mut self, caret: bool, library: &Library) -> Result<(), String> {
        if !self.keeps {
            return Ok(());
        }
        let now = Instant::now();
        self.finish_read()?;
        self.finish_save(library, now);
        // A copy that could not be read was reported when that failed, and holds nothing back now.
        let Some(mut copy) = self.copy.take() else { return Ok(()) };
        let home = self.settle(caret, library, now).filter(|_| self.unsaved());
        let saved = home.map(|home| {
            let (edits, revision) = self.edits_for(&mut copy);
            home.exchange(&mut copy, &edits, &revision).map(|(home, news)| (home, news, revision))
        });
        self.copy = Some(copy);
        match saved {
            Some(Ok((home, news, revision))) => {
                self.saved(home, news, self.view.edits(), revision, library);
                self.save_at = None;
            }
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
    fn save_or_say(&mut self, caret: bool, library: &Library) -> bool {
        let saved = self.save_now(caret, library);
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
        let made_here = opened.is_none();
        let (document, style, place, rest) = match opened {
            Some(Opened { document, style, home, rest }) => (document, style, Place::at(home), rest),
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
        for request in window.view.take_requests() {
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
                // A request this shell does not know yet.
                _ => {}
            }
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
                    true if pristine => drop(self.windows.remove(&id)),
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
                    window.save_or_say(true, &self.library);
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
        let saved = self.windows.get_mut(&id).is_some_and(|window| window.save_or_say(false, &self.library));
        self.vouch();
        if saved {
            self.windows.remove(&id);
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
            self.open_window(event_loop, opened, true).map_err(|error| format!("No window could be opened.\n\n{error}"))
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
                        chrome::reduced_motion().map(|reduced| proxy.send_event(Event::Motion(reduced)))
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
                    window.start_read(&self.proxy);
                    window.ask_dictionaries(&self.proxy, &self.dictionaries);
                    window.relocate(library);
                }
                window.relist(library);
                return;
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::ThemeChanged(_) => {
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
                if let Err(error) = window.finish_read() {
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
            if let Err(error) = window.save_now(false, &self.library) {
                eprintln!("letters: {error}");
            }
        }
        self.vouch();
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
            for due in [wake, save, type_next].into_iter().flatten() {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
