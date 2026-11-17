//! The winit event loop: windows, their input, painting and saving.
//!
//! Each window holds one document. Everything a window shows comes from its view as a display list, which
//! is rasterised into a software surface. Saving happens on a thread per save, a moment after typing stops,
//! and nobody is asked where: a new document gets a file of its own once it has content. What is typed
//! goes into a journal sooner than that, so that it outlives a crash.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::convert::Infallible;
use std::error::Error;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use letters_editor::{Editor, InputEvent, KeyEvent, Picture, Platform as _};
use letters_format::WorkingCopy;
use letters_frame::Lights;
use letters_layout::Named;
use letters_model::{BlockId, Change, Model, Notes, ReplicaId, Revision, Selection, Style, TagKind};
use letters_render::image::Image;
use letters_render::{Color, DisplayList, Font, Point, Size};
use letters_spell::Dictionary;
use letters_sync::{Local, Progress, Refusal};
use letters_trust::{FetchError, Gate, HttpsUrl, ImageDecision, Limits, MergeDecision, Origin, Trust, TrustError};
// The editor's window is the view here, where `Window` is the system's.
use letters_ui::{
    Access, Allows, Appearance, Browsed, Chrome, Connection, CopyFormat, Cursor, Edge, FontOffer, Frame, ImportFolder,
    ImportKind, ImportOffer, Landing, LinkPreviewFound, Location, MergeAsk, MergeFile, Notice, PictureState, Request,
    SafeMode, SaveState, Scheme, Settings, SignIn, Stored, Window as View, tokens,
};
use rfd::{FileDialog, MessageDialog, MessageLevel};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::error::EventLoopError;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{self, ModifiersState, NamedKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use winit::window::{CursorIcon, ResizeDirection, Theme, Window, WindowAttributes, WindowId};

use crate::chosen::{self, Chooser, Chosen};
use crate::chrome::{self, Buttons, Host};
use crate::cli::{Format, Mode, Options};
use crate::conflicts;
use crate::connected::{self, Line, Placed, Placing, Reached, Said, Sharing, Shelf, Told};
use crate::convert;
use crate::copies;
use crate::dictionaries;
use crate::fallback::{BUNDLED, Finder};
use crate::files::{self, EXTENSION, Exchange, Home, Merged, Opened, Rest, STYLESHEET, Working};
use crate::gpu::Gpu;
use crate::import::{self, Choices, Plan, Unplanned};
use crate::input::{self, Composer, Press};
use crate::journal::{self, Aside, Journal, Journals};
use crate::keychain::Keychain;
use crate::library::{Library, Place, Recent, Refused, UNTITLED, has_content};
use crate::links::{self, Step};
use crate::measure::{FIRST_FRAME, Launch, Typing};
use crate::menus::{self, Bar};
use crate::pictures::{self, Decoder, Source};
use crate::platform::{self, Desktop};
use crate::present::{Painter, Presenter};
use crate::printing;
use crate::remote::{self, Roots};
use crate::seen::{Seen, Telling};
use crate::server::Client;
use crate::servers;
use crate::session::Session;
use crate::settings;
use crate::telemetry::{self, Page, Telemetry};
use crate::trust::{self, Trusted};
use crate::update;
use crate::versions::Rests;
use crate::words::{self, Words};
use crate::workers::Workers;
use crate::writer;

const MAC: bool = cfg!(target_os = "macos");
/// How long typing must pause before a save starts.
const SAVE_AFTER: Duration = Duration::from_millis(800);
/// How long the app waits, as it ends, for servers to hear what was last seen of their documents.
const PARTING: Duration = Duration::from_secs(2);
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
/// The largest picture a link preview takes, in bytes. It is kept in the document, where a card shows it
/// small.
const PREVIEW_PICTURE: u64 = 4 * 1024 * 1024;
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1100.0, 800.0);
/// What a file dialog calls the files it lists for the Styles panel.
const STYLESHEETS: &str = "Letters stylesheet";

/// A session and a document of its server that is ready to open from its copy on this device.
type Entered = Box<(Arc<Session>, Reached)>;

/// What reaches the event loop from outside winit.
pub enum Event {
    Access(accesskit_winit::Event),
    /// A window's save has finished, and its result is ready to collect.
    Saved(WindowId),
    /// A window's document was written as a PDF to print, or could not be.
    Pdf(WindowId, Result<Vec<u8>, String>),
    /// The print dialog a window asked for was shown and is done with, or could not be shown.
    Printed(WindowId, Result<(), String>),
    /// The copy a window asked for was written at this path, or was not.
    Copied(WindowId, PathBuf, Result<Option<String>, copies::Refused>),
    /// Faces of the system were found for characters a window showed without a glyph.
    Faces(WindowId, Vec<Font>),
    /// The families a writer can choose a font from have been listed.
    Families(Vec<FontOffer>),
    /// The faces of the fonts a window's document names were looked for.
    Chosen(WindowId, Box<chosen::Found>),
    /// What a window's first paint did not need has been read.
    Read(WindowId),
    /// A newer release of Letters was found, and fetched where it can take this one's place.
    Update(Box<update::Found>),
    /// The file at this path, which a window asked to be shown where it is kept, could not be shown.
    Unshown(WindowId, PathBuf),
    /// The folder of a window's document, whose file is at this path, was looked through for conflict
    /// copies of it, and for the file it is itself a conflict copy of.
    Beside(WindowId, PathBuf, Box<conflicts::Beside>),
    /// The search for a GPU has ended, with one or without.
    Gpu(Option<Box<Gpu>>),
    /// The desktop has said whether it wants less motion.
    Motion(bool),
    /// The desktop has said which buttons its windows carry.
    Buttons(Buttons),
    /// What the person's account calls them has been found.
    Account(Option<String>),
    /// The languages the system has dictionaries for have been listed.
    Languages(Vec<String>),
    /// A picture a window's page asked for was decoded, to be held no wider than the last, or could not
    /// be had.
    Picture(WindowId, String, Option<Image>, u32),
    /// The file at a path, which a window was asked to bring in, has been read: what it would bring is
    /// known, or why it cannot be brought in.
    /// With where it is to be made, as the card says it, when that is a folder of a server.
    Planned(WindowId, PathBuf, Result<Box<Plan>, Unplanned>, Option<String>),
    /// A file of this name was brought in for a window and kept as a new document at this path, or could
    /// not be.
    Imported(WindowId, String, Result<PathBuf, (Box<Plan>, std::io::Error)>),
    /// How far the person's clock is ahead of UTC, in minutes.
    Offset(i32),
    /// A picture a window's document links to, at this address, was fetched for this: its file, or why
    /// there is none.
    Linked(WindowId, String, Link, Result<Vec<u8>, String>),
    /// The page at this address was read for a window's link previews: what it says of itself, or why
    /// there is nothing.
    Previewed(WindowId, String, Result<Box<LinkPreviewFound>, String>),
    /// A system dictionary a window's document asked for has been read, under the name it has on disk.
    Dictionary(WindowId, String, Box<Dictionary>),
    /// The server of a window's document said which languages it has dictionaries for.
    Served(WindowId, Vec<String>),
    /// These languages a window's document names were not looked for after all, since the server could
    /// not be asked. They are asked about again.
    Unasked(WindowId, Vec<String>),
    /// The way to a server's document was opened for the window that asked: the session at its server
    /// and the document's copy on this device, or why there is neither.
    /// There is nothing to say of a sign-in the person gave up.
    Reached(Option<WindowId>, Result<Box<(Arc<Session>, Reached)>, Option<String>>),
    /// The sessions kept from when the app last ran have been read.
    Kept(Vec<Arc<Session>>),
    /// The sign-in a window waits for, which ends when this is set, has sent the browser to the page at
    /// this address of the server of this name.
    SigningIn(WindowId, Arc<AtomicBool>, String, String),
    /// The sign-in a window asked for, which ended when this was set, has ended: in a session, or in a
    /// sentence on why not, or in neither if the person gave it up.
    SignedIn(WindowId, Arc<AtomicBool>, Result<Arc<Session>, Option<String>>),
    /// A server said what a place of it holds, for a window's Open card, or why it could not be read.
    Browsed(WindowId, Result<Box<Browsed>, String>),
    /// The server at this address said what is in its trash, for a window's Open card, or why not.
    Trash(WindowId, String, Result<Vec<letters_api::Trashed>, String>),
    /// The file of this name that a window offered was made a document of at a server and its copy is
    /// here to open, or it was not, with the file's plan for another try and a sentence on why.
    MadeThere(WindowId, String, Result<Entered, (Box<Plan>, String)>),
    /// The socket of this number, to the server of a window's document, has something to say.
    Heard(WindowId, Said),
    /// A picture of this name, which the copy of a window's document lacked, was asked of the document's
    /// server: its file, if the server had it.
    Brought(WindowId, String, Option<Vec<u8>>),
    /// The server of a window's document was asked about the document's name and place: how the
    /// document stands there now. The flag says that the window's bar asked.
    Placed(WindowId, bool, Box<Placed>),
    /// The server of a window's document answered what the window's Share dialog asked, in that order.
    Shared(WindowId, Vec<connected::Shared>),
    /// An item of the menu bar was taken: its place in [`menus::items`].
    Menu(usize),
}

impl From<accesskit_winit::Event> for Event {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Access(event)
    }
}

/// Runs the app until its last window closes.
pub fn run(options: Options, mut launch: Launch) -> Result<(), Box<dyn Error>> {
    // The file is read while the event loop connects to the display.
    // A file to bring in is no document to read: a window offers it once there is one.
    // The address of a document's page is no file: its server is asked once a window is up.
    let from_server = options.file.as_deref().and_then(Path::to_str).and_then(connected::address);
    let file = options.file.filter(|_| from_server.is_none());
    let document = file.clone().filter(|path| !import::offered(path));
    let reading = document.map(|path| thread::spawn(move || files::open(&path, new_style)));
    let report_launch = options.mode == Mode::LaunchReport;
    let event_loop = event_loop(report_launch)?;
    let home = std::env::home_dir().unwrap_or_default();
    launch.mark("event loop built");
    // The first frame is drawn light or dark and in the person's accent, so this one small file is read
    // before it. A run that only measures is the same for everyone.
    let measuring = options.mode != Mode::Edit;
    let settings = if measuring { Settings::default() } else { settings::load(&home) };
    let mut app = App {
        starting: true,
        report_launch,
        telemetry: Telemetry::off(),
        reports: None,
        notice: false,
        page: None,
        update: None,
        updating: None,
        typing: match options.mode {
            Mode::TypingReport { keys } => Some(TypingRun { keys, stats: Typing::default(), typed: None, next: None }),
            _ => None,
        },
        launch: Some(launch),
        reading,
        path: file,
        from_server,
        sessions: HashMap::new(),
        servers_told: false,
        open_asked: None,
        signing: HashMap::new(),
        proxy: event_loop.create_proxy(),
        platform: Desktop::new().with_words(Words::of(&home)),
        library: Library::of_person(&home),
        seen: Seen::of(&home),
        named: false,
        words: Words::of(&home),
        trust: trust::store(&home),
        journals: Journals::of(&home),
        decoder: Decoder::default(),
        fetching: Workers::new(4),
        dictionaries: dictionaries::system_folders(&home).into(),
        reduced_motion: false,
        offset: None,
        buttons: Buttons::guess(),
        settings,
        account: None,
        languages: Vec::new(),
        gpu: match std::env::var("LETTERS_RENDERER").as_deref() {
            Ok("cpu") => GpuState::Off,
            Ok("gpu") => GpuState::Wanted { software: true },
            _ => GpuState::Wanted { software: false },
        },
        windows: HashMap::new(),
        copies: Vec::new(),
        parting: Vec::new(),
        fonts: Finder::system({
            let proxy = event_loop.create_proxy();
            move |id, faces| drop(proxy.send_event(Event::Faces(id, faces)))
        }),
        chosen: Chooser::idle(),
        families: Vec::new(),
        mods: ModifiersState::empty(),
        bar: None,
        bar_stale: false,
        failure: None,
        home,
    };
    event_loop.run_app(&mut app)?;
    app.failure.map_or(Ok(()), |failure| Err(failure.into()))
}

/// Builds the event loop. A `background` app does not come to the front, so a run that only measures takes
/// no keystrokes from whatever the user is typing into.
pub(crate) fn event_loop<T: 'static>(background: bool) -> Result<EventLoop<T>, EventLoopError> {
    // Run from outside its bundle, the app would be called by its file's name in the menu bar.
    letters_frame::name_process(NAME);
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
/// to the app: under the system's lights on macOS, in a frame Letters draws on Linux, and in the border
/// the system draws on Windows.
pub(crate) fn window_attributes(event_loop: &ActiveEventLoop) -> WindowAttributes {
    // A Wayland window draws its own corners, which are round: what its frames leave clear must show
    // what is behind it. Elsewhere a window is all its own.
    let wayland =
        event_loop.display_handle().is_ok_and(|display| matches!(display.as_raw(), RawDisplayHandle::Wayland(_)));
    let attributes = Window::default_attributes()
        .with_title(UNTITLED)
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(LogicalSize::new(360.0, 240.0))
        .with_transparent(wayland);
    #[cfg(target_os = "macos")]
    let attributes = {
        use winit::platform::macos::WindowAttributesExtMacOS;
        attributes.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true)
    };
    // A Wayland desktop finds the icon through the desktop entry of this name. X11 is handed it.
    #[cfg(all(unix, not(target_os = "macos")))]
    let attributes = if wayland {
        winit::platform::wayland::WindowAttributesExtWayland::with_name(attributes, APP_ID, "letters")
    } else {
        winit::platform::x11::WindowAttributesExtX11::with_name(attributes, APP_ID, "letters")
            .with_window_icon(window_icon(include_bytes!("../../../assets/icons/png/app-128.png")))
    };
    // Windows keeps the border, the shadow and the snapping of a window whose title bar it leaves out.
    // The taskbar and Alt+Tab show the larger picture, and nothing shows the smaller one.
    #[cfg(windows)]
    let attributes = {
        use winit::platform::windows::WindowAttributesExtWindows;
        attributes
            .with_undecorated_shadow(true)
            .with_window_icon(window_icon(include_bytes!("../../../assets/icons/png/app-32.png")))
            .with_taskbar_icon(window_icon(include_bytes!("../../../assets/icons/png/app-256.png")))
    };
    attributes.with_decorations(!matches!(Host::HERE, Host::Linux | Host::Windows))
}

/// What the app is called where a person reads its name.
pub const NAME: &str = "Letters";

/// The name a desktop knows Letters by: its desktop entry on Linux and its bundle on macOS carry the same.
pub const APP_ID: &str = "com.sierrasoftworks.Letters";

/// The app's icon from one of its PNGs, for a desktop that takes it from the window.
#[cfg(not(target_os = "macos"))]
fn window_icon(png: &[u8]) -> Option<winit::window::Icon> {
    let icon = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.into_rgba8();
    let (width, height) = icon.dimensions();
    winit::window::Icon::from_rgba(icon.into_raw(), width, height).ok()
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
    painter.paint(&list, view.images(), scale as f32, background(false), false, width, 0, &mut pixels);
    launch.mark(FIRST_FRAME);
    let Mode::TypingReport { keys } = options.mode else {
        println!("{FIRST_FRAME}\n{}", launch.report());
        return Ok(true);
    };

    // Typing is timed with the pictures the page shows in place, as a window has them soon after its
    // first frame.
    let within = pictures::widest(view.editor().model().style(), scale);
    for request in view.take_requests() {
        let (Request::Asset(asset), Some(path)) = (request, &options.file) else { continue };
        let image = files::entry(path, &asset).and_then(|bytes| Image::decode_within(&bytes, within).ok());
        view.set_image(&asset, image);
    }
    list.items.clear();
    view.paint(&mut list);
    painter.paint(&list, view.images(), scale as f32, background(false), false, width, 0, &mut pixels);
    let mut platform = Desktop::new();
    let mut typing = Typing::default();
    for _ in 0..keys {
        let typed = Instant::now();
        view.input(InputEvent::Insert("a".into()), &mut platform);
        list.items.clear();
        view.paint(&mut list);
        // The pixels still hold the frame before, as a window's buffer does.
        painter.paint(&list, view.images(), scale as f32, background(false), false, width, 1, &mut pixels);
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

/// A sign-in at the browser that a window waits for.
struct Signing {
    /// Set to give the sign-in up.
    given_up: Arc<AtomicBool>,
    /// Whether the window shows it. One that opening a document began is not shown until there is a page
    /// to wait at.
    shown: bool,
}

/// Reads the sessions the keychain keeps for the servers that are listed in `home`, on another thread.
/// They arrive as an event.
fn read_kept(proxy: &EventLoopProxy<Event>, home: &Path) {
    let (proxy, home) = (proxy.clone(), home.to_owned());
    thread::spawn(move || {
        let kept = servers::kept(&home, || Keychain::system().ok(), &Roots::System);
        let _ = proxy.send_event(Event::Kept(kept));
    });
}

struct App {
    /// Whether the first window is still to open.
    starting: bool,
    report_launch: bool,
    /// Off until the first frame is drawn, and for good in a run that measures.
    telemetry: Telemetry,
    /// Whether the person lets this run report. `None` in a run that never would, which shows no switch.
    reports: Option<bool>,
    /// Whether the landing card says that reports are sent: for the one run that first sends any.
    notice: bool,
    /// The page last reported.
    page: Option<Page>,
    /// A newer release that was found, which the windows have a notice of.
    update: Option<update::Found>,
    /// What the new version is started with once everything is saved, when the person asked for it.
    updating: Option<Option<PathBuf>>,
    /// The typing report in progress, in that mode.
    typing: Option<TypingRun>,
    /// The stopwatch, until the first frame is on screen.
    launch: Option<Launch>,
    reading: Option<JoinHandle<Result<Opened, files::Error>>>,
    path: Option<PathBuf>,
    /// The server and the document the app was asked to open by the address of its page, until it has
    /// been asked for.
    from_server: Option<(String, String)>,
    /// The sessions at the servers the person is signed in to, by each server's address.
    sessions: HashMap<String, Arc<Session>>,
    /// Whether the kept sessions have been read, after the first frame. The windows hear where the
    /// person is signed in from then on.
    servers_told: bool,
    /// The window that asked to open a document before the keychain had said where the person is signed
    /// in, and whether its landing card showed. It is answered once that is known, with the card of a
    /// server's documents if there is a server, so a press at launch does not fall to the system's dialog.
    open_asked: Option<(WindowId, bool)>,
    /// The sign-ins under way, by the window that waits for each.
    signing: HashMap<WindowId, Signing>,
    /// The person's home folder, where the copies of servers' documents are kept.
    home: PathBuf,
    proxy: EventLoopProxy<Event>,
    platform: Desktop,
    library: Library,
    /// What the person last saw of each document.
    seen: Seen,
    /// Whether the account was asked for its name, so that who writes here is as known as it will be.
    named: bool,
    words: Words,
    trust: Trusted,
    journals: Journals,
    /// Decodes the pictures the windows' pages ask for.
    decoder: Decoder,
    /// Fetches the pictures the windows' documents link to.
    fetching: Workers,
    /// Where the system keeps dictionaries for languages other than English.
    dictionaries: Arc<[PathBuf]>,
    /// Whether the desktop asked for less motion. Asked once, after the first frame.
    reduced_motion: bool,
    /// How far the person's clock is ahead of UTC, in minutes, once that has been looked up.
    offset: Option<i32>,
    /// The buttons the desktop's windows carry: a guess until it has been asked, after the first frame.
    buttons: Buttons,
    /// What the person set for themselves.
    settings: Settings,
    /// What the person's account calls them, once it has been found, after the first frame. Edits are
    /// signed with it while the settings give no name.
    account: Option<String>,
    /// The languages the system has dictionaries for, once they have been listed, after the first frame.
    languages: Vec<String>,
    gpu: GpuState,
    windows: HashMap<WindowId, DocWindow>,
    /// The copies being written, which the app waits for before it ends.
    copies: Vec<JoinHandle<()>>,
    /// The tellings of what was seen that closed windows left on their way to their servers.
    parting: Vec<JoinHandle<bool>>,
    /// Looks among the system's fonts for what the bundled ones cannot draw, once a window shows any.
    fonts: Finder<WindowId>,
    /// Finds the faces of the fonts a writer chose, from the first frame on.
    chosen: Chooser<WindowId>,
    /// The families a writer can choose from, once they are listed.
    families: Vec<FontOffer>,
    mods: ModifiersState,
    /// The menus macOS shows at the top of the screen. `None` elsewhere, and until a window is up.
    bar: Option<Bar>,
    /// Whether something happened since the menus were last told what the window in front offers.
    bar_stale: bool,
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

/// A folder of this device as the window lists it. One whose path is no text is left out.
fn shown(folder: crate::library::Folder) -> Option<letters_ui::Folder> {
    Some(letters_ui::Folder { path: folder.path.to_str()?.to_owned(), name: folder.name, depth: folder.depth })
}

/// What a window holds of a document that lives on a server.
struct Remote {
    line: Line,
    /// What the server said of the document.
    opened: letters_api::Opened,
    /// The server's name for the bar, and its address as the Servers dialog lists it.
    host: String,
    origin: String,
    /// When the line's clock started. The line is told the time since.
    born: Instant,
    /// Whether the line was started, which waits for the file's history.
    started: bool,
    /// Whether an earlier line of this window may have left work the server lacks.
    owes: bool,
    /// The selection the line was last told of.
    selection: Option<Selection>,
    /// The pictures the server was asked for, each once.
    asked: BTreeSet<String>,
    /// Pictures the copy lacks, until the server is asked for them.
    lacking: Vec<String>,
    /// What is to be asked of the server about the document's name and place, in this order.
    asks: Vec<Placing>,
    /// Whether the server is being asked. One thing is asked at a time, so that the server does them
    /// in the order the person did.
    placing: bool,
    /// What the Share dialog asks of the server, in this order, and whether the server is being asked.
    shares: Vec<Sharing>,
    sharing: bool,
    /// Whether the line was live when it last said how it stood, and when the server was last asked how
    /// the document stands.
    live: bool,
    looked: Option<Instant>,
    /// The folders of the document's space as the server last listed them, which it can move among.
    shelves: Vec<Shelf>,
    /// The name and the place the list of recent documents has for it, once it is on that list.
    listed: Option<(String, String)>,
}

impl Remote {
    /// `now` on the line's clock, in milliseconds.
    fn ms(&self, now: Instant) -> u64 {
        u64::try_from(now.saturating_duration_since(self.born).as_millis()).unwrap_or(u64::MAX)
    }

    /// When the line next wants a tick.
    fn due(&self) -> Option<Instant> {
        Some(self.born + Duration::from_millis(self.line.wake_at()?))
    }

    /// What the person may do with the document: as the line says while it is live, and as the server
    /// last said otherwise, which is what holds while there is no line.
    fn access(&self) -> Access {
        let (edits, comments) = match self.line.link() {
            letters_sync::Link::Live(role) if self.started => (role.edits(), role.comments()),
            _ => (self.opened.role >= letters_api::Role::Editor, self.opened.role.comments()),
        };
        match (edits, comments) {
            (true, _) => Access::Edits,
            (false, true) => Access::Comments,
            (false, false) => Access::Reads,
        }
    }

    /// What the person may take away of the document, as the server last said.
    fn allows(&self) -> Allows {
        let letters_api::Allows { download, copy, print } = self.opened.allows;
        Allows { copy, download, print }
    }

    /// Whether the server vouches for the document: one in the person's own spaces, not one only shared
    /// with them.
    fn vouched(&self) -> bool {
        self.opened.trust == letters_api::Trust::Trusted
    }

    /// How the window stands to the server, for its bar.
    fn connection(&self) -> Connection {
        match self.line.link() {
            // The line waits for the file's history.
            _ if !self.started => Connection::Connecting,
            letters_sync::Link::Offline => Connection::Offline,
            letters_sync::Link::Connecting => Connection::Connecting,
            letters_sync::Link::Live(_) => Connection::Live,
            letters_sync::Link::Refused(refusal) => Connection::Refused(refusal.to_string()),
        }
    }

    /// Where the server keeps the document, for the card that opens its documents.
    fn stored(&self) -> Stored {
        let letters_api::Opened { id, space, path, .. } = &self.opened;
        Stored { server: self.origin.clone(), space: space.clone(), path: path.clone(), id: id.clone() }
    }

    /// What the bar is told a folder of the document's space by: no path on this device.
    fn key(&self, folder: &str) -> String {
        let parts = [self.host.as_str(), self.opened.space.as_deref().unwrap_or_default(), folder];
        parts.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join("/")
    }

    /// The folder the server keeps the document in, as the bar is told it.
    fn folder(&self) -> String {
        self.key(self.opened.path.rsplit_once('/').map_or("", |(folder, _)| folder))
    }

    /// Where the server keeps the document, as a list of recent documents says it: the server, the space
    /// by its name if the server told it, and the folders down to the document's.
    fn shown(&self) -> String {
        let space = self.shelves.first().map(|shelf| shelf.name.as_str());
        let folder = self.opened.path.rsplit_once('/').map_or("", |(folder, _)| folder);
        let parts = [Some(self.host.as_str()), space].into_iter().flatten().chain(folder.split('/'));
        parts.filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" / ")
    }

    /// The document as the list of recent ones is to hold it, if the list does not hold it so yet.
    fn lately(&mut self) -> Option<servers::Lately> {
        let now = (self.opened.name.clone(), self.shown());
        if self.listed.as_ref() == Some(&now) {
            return None;
        }
        self.listed = Some(now.clone());
        let address =
            format!("{}{}", self.line.session().server().origin(), letters_api::paths::editor_page(&self.opened.id));
        Some(servers::Lately { address, name: now.0, folder: now.1, opened: platform::now_rfc3339() })
    }

    /// The space and the folder of it that the bar knows by `key`, if it is one the server listed.
    fn shelf(&self, key: &str) -> Option<(String, String)> {
        let shelf = self.shelves.iter().find(|shelf| self.key(&shelf.path) == key)?;
        Some((self.opened.space.clone()?, shelf.path.clone()))
    }

    /// Has the server asked how the document stands, if it has not been for a while: nothing tells an
    /// open window that someone else renamed or moved the document.
    fn look(&mut self, now: Instant) {
        let lately = self.looked.is_some_and(|looked| now.saturating_duration_since(looked) < LOOK_AFTER);
        if self.live && !lately && !self.asks.contains(&Placing::Look) {
            self.asks.push(Placing::Look);
        }
    }

    /// Whether only a new sign-in opens the document again.
    fn signed_out(&self) -> bool {
        self.line.session().ended() || self.line.link() == letters_sync::Link::Refused(Refusal::SignedOut)
    }
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
    /// Where the window writes its words for a test to read. See [`SAYS`].
    says: Option<PathBuf>,
    /// Whether the accessibility tree is behind the view, and whether it must be told again in full.
    access_stale: bool,
    access_whole: bool,
    /// The frame the view was last told to draw.
    chrome: Chrome,
    /// Whether the person set the window light or dark, or left it to the system.
    appearance: Appearance,
    /// The height of the unified title bar the system gave the window, which the bar fills until the
    /// lights are seen.
    bar: Option<f32>,
    /// Where the system last stood the window's lights, which the bar is then centred on.
    lights: Option<Lights>,
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
    /// The faces the window's text has asked for that are none of the bundled families'.
    fonts_named: Vec<Named>,
    /// What the fonts were last looked for: a hash of the faces the style can ask for, the font files it
    /// lists, the faces the text has asked for, and whether the document may keep fonts. They are looked
    /// for again when it changes.
    fonts_seen: Option<u64>,
    /// The families the person was told are not to be had, each once.
    fonts_said: BTreeSet<String>,
    /// The font files asked of the document's server, each once.
    fonts_asked: BTreeSet<String>,
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
    /// The languages a dictionary has been looked for, each once whether or not there was one.
    asked: BTreeSet<String>,
    /// The languages the document's server has dictionaries for, once it has said.
    served: Option<Vec<String>>,
    /// The view's edit count that is safely on disk.
    saved_edits: u64,
    /// The edit count a save was last planned for.
    noticed_edits: u64,
    save_at: Option<Instant>,
    /// The save that is running, with the edit count and the editor's revision it started from.
    saving: Option<(u64, Revision, JoinHandle<Saving>)>,
    save_failed: bool,

    /// Where what the person saw of each document is kept, and what was last kept there of this one.
    seen: Seen,
    marked: Option<Revision>,
    /// What the account of a server's document is told the person saw.
    telling: Telling,
    /// What the person last saw of the document, until the window is told.
    last_seen: Option<Revision>,
    /// Whether the window was told by the copy on this device, the document's server being out of reach.
    seen_offline: bool,
    /// Whether who writes here is as known as it will be. Until then the person's own writing from
    /// elsewhere could pass for another's.
    named: bool,

    /// What the first paint did not need from the file, until reading it starts.
    rest: Option<Rest>,
    reading: Option<JoinHandle<Result<Working, &'static str>>>,
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
    /// The notes as the journal, or the file before it, last held them.
    journalled_notes: Notes,
    journal_at: Option<Instant>,
    /// Assets the document names that no save has been handed yet.
    assets: BTreeMap<String, Vec<u8>>,
    /// The file this window offers to bring in, as it was read for the card that asks about it.
    plan: Option<Box<Plan>>,
    /// Files let go over the window since it was last looked at. They come one at a time.
    dropped: Vec<PathBuf>,
    /// Whether the document is on its way to the print dialog, or the dialog is up.
    printing: bool,
    /// The name of the file this window offers to bring in, for the card and for the history.
    importing: Option<String>,
    /// Where the file this window offers is to be made a document, when that is at a server: the
    /// server's address, the space, and the folder in it. `None` for the default folder on this device.
    import_to: Option<(String, String, String)>,
    /// The name of the file this document was just brought in from, until its history says so.
    imported: Option<String>,
    /// Journals that were set aside, as the app ended when it last read them, until the window says so.
    aside: Vec<Aside>,
    /// Where the journals are that the window has said were set aside, for the button that shows one.
    kept: Vec<PathBuf>,
    /// Where the writing rested, for the versions the history lists.
    rests: Rests,
    /// Whether versions were marked that no save has been handed. A mark is no edit, but it is saved.
    marks: bool,
    /// The conflict copies the view was last told of. `None` until it was told of any, which it is
    /// once a saved document is read, with or without copies: only then does it offer a merge.
    copies: Option<Vec<PathBuf>>,
    /// Files to merge into the document with its next save, and whether each is a conflict copy, which
    /// goes to the trash once the document's file holds what it held.
    merging: Vec<(PathBuf, bool)>,
    /// The files the save that is running merges.
    carried: Vec<(PathBuf, bool)>,
    /// The file this document was copied from by hand, which it still says it is. With its first edit
    /// it becomes a document of its own. `None` once it has, and for a document with no such file.
    twin: Option<PathBuf>,
    /// Whether that file was looked for, which is done once.
    twinned: bool,
    /// Whether the person stopped trusting the document in this window, which then no longer vouches for
    /// what was made in it.
    forgotten: bool,
    /// The id the document had and the one it took as it became its own, until the app has carried what
    /// the person trusts of the first over to the second.
    forked: Option<(String, String)>,
    /// The server the document lives on and the line to it. `None` for a document that is its file.
    remote: Option<Remote>,
    /// The file this document is a conflict copy of, as the view was last told.
    original: Option<PathBuf>,
    /// The conflict copy this window left for its original, and that original. The copy goes to the
    /// trash once a save has merged the document into the original's file.
    left: Option<(PathBuf, PathBuf)>,
    /// Pictures the page asked for whose files were not to be had while a read or a save was under way.
    awaited: Vec<String>,
    /// Linked pictures that were asked for before the file said which document this is, which decides
    /// what it may fetch.
    links: Vec<(String, Link)>,
    /// The files of the linked pictures the page shows, by their addresses, so that a sharper screen, a
    /// print or a PDF copy does not ask their servers again.
    linked: copies::Linked,
    /// The pictures the view was handed, and how wide each could be held at most.
    held: BTreeMap<String, u32>,
}

/// A save on its way back from its thread: the copy, and how the save ended.
struct Saving {
    copy: WorkingCopy,
    result: Result<Exchange, String>,
}

/// What a document may fetch: one that was made in its window, or else the document `id` in the file at
/// `path`. Apart from its window, so that a thread can ask with what it was handed.
fn gate_of<'a>(trust: &'a Trust, made_here: bool, id: Option<&str>, path: Option<&Path>) -> Gate<'a> {
    match id {
        _ if made_here => trust.gate(id.unwrap_or_default(), Origin::Created),
        Some(id) => trust::gate(trust, id, path),
        // Nothing vouches for a document that has not said which one it is.
        None => trust.gate("", Origin::Outside),
    }
}

/// What a linked picture is fetched for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// To show, if the person's trust lets it load as its document opens.
    Open,
    /// To show this once, because the person asked.
    Once,
    /// To keep in the document, in this figure.
    Embed(BlockId),
}

/// Whether a change shows in the document, as text or a block that came, went or moved.
fn moved(change: &Change) -> bool {
    *change != Change::default()
}

/// A file as the window names it when it offers or asks to merge it.
fn merge_file(path: &Path, library: &Library) -> MergeFile {
    let folder = library.shown(path.parent().unwrap_or(Path::new("")));
    MergeFile { path: path.to_string_lossy().into_owned(), name: named(path), folder }
}

/// The name of a file, as a sentence about it says it.
fn named(path: &Path) -> String {
    path.file_name().unwrap_or_default().to_string_lossy().into_owned()
}

/// Whether two folders are one, however each is spelled. One that cannot be resolved is taken as written.
fn lies_at(folder: &Path, other: &Path) -> bool {
    let real = |path: &Path| fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    real(folder) == real(other)
}

/// What the person is told of a picture the document's server would not take.
const UNKEPT: &str = "The server did not take a picture, so the others who open this document will not see it.";

/// What the bar says when asked to rename or move a server's document.
/// How long a server's answer about a document's name and place holds for a window that is turned to
/// again.
const LOOK_AFTER: Duration = Duration::from_secs(30);
/// What the Comment tool says of a document whose notes this build could not read.
const NO_COMMENTS: &str = "Comments cannot be added here: this document's notes are in a form this version of \
                           Letters cannot read.";

/// Why a file that is no picture after all is not shown, as it reads on from "Not loaded:".
const NO_PICTURE: &str = "it is not a picture Letters can show";

/// Something a key asks of the app, not of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    New,
    /// Opens the document at a path, or one the person picks in the system's dialog.
    Open(Option<PathBuf>),
    /// Opens a document as the window's own Open does: from its card of a server's documents while the
    /// person is signed in to one, else from the system's dialog.
    Choose,
    Save,
    Close,
    Quit,
}

/// What a key asks of the app when the window had no use for it: the window binds these keys, and asks
/// for them itself wherever it can.
fn shortcut(event: &KeyEvent) -> Option<Action> {
    Some(match letters_ui::command_of_key(event)? {
        "New document" => Action::New,
        "Open" => Action::Choose,
        "Save" => Action::Save,
        "Close window" => Action::Close,
        "Quit" => Action::Quit,
        _ => return None,
    })
}

impl DocWindow {
    fn name(&self) -> String {
        self.place.name()
    }

    fn unsaved(&self) -> bool {
        self.keeps && (self.view.edits() != self.saved_edits || self.marks)
    }

    /// Marks a version in the document's history, where the editor finds one worth marking. The mark
    /// goes to the file with the next save.
    fn mark(&mut self, at: &Revision, kind: TagKind, name: Option<&str>) {
        let was = self.view.editor().model().revision();
        let marked = self.view.change(|editor| editor.mark_version(at, kind, name).map(|()| Change::default()));
        match marked {
            Ok(()) => self.marks |= self.view.editor().model().revision() != was,
            Err(error) => eprintln!("letters: could not mark a version of {}: {error}", self.name()),
        }
    }

    fn dark(&self) -> bool {
        match self.appearance {
            Appearance::System => self.window.theme() == Some(Theme::Dark),
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// Follows what the person set, at the start and whenever it changes in any window. `writer` is the
    /// name their edits carry: the one they set, or else their account's once that is known.
    fn follow(&mut self, settings: &Settings, writer: Option<&str>) {
        if *self.view.settings() != *settings {
            self.view.set_settings(settings.clone());
        }
        // A server's document is signed with the name its server has for the person.
        let theirs = self.remote.as_ref().map(|remote| remote.opened.author.as_str()).filter(|name| !name.is_empty());
        if let Some(writer) = theirs.or(writer) {
            self.view.set_author(writer);
            self.named = true;
            self.say_seen(false);
        }
        self.appearance = settings.appearance;
        // What the system draws of the window goes light or dark with it: a title bar, or its buttons.
        self.window.set_theme(match settings.appearance {
            Appearance::System => None,
            Appearance::Light => Some(Theme::Light),
            Appearance::Dark => Some(Theme::Dark),
        });
        self.fit();
        self.window.request_redraw();
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
        // A comment counts among the edits, and the notes move with the text they are about.
        if self.view.notes() != &self.journalled_notes {
            journal.notes(self.view.notes());
            self.journalled_notes = self.view.notes().clone();
        }
    }

    /// How wide a picture is held for this window, in pixels.
    fn widest(&self) -> u32 {
        pictures::widest(self.view.editor().model().style(), self.window.scale_factor())
    }

    /// Keeps the assets the editor stored for this window's document: in the journal at once, and in the
    /// file with the next save. The page asks for each when it comes to show it.
    fn keep(&mut self, assets: Vec<(String, Vec<u8>)>) {
        for (name, bytes) in assets {
            if let Some(journal) = &mut self.journal {
                journal.asset(&name, &bytes);
            }
            // The document's server is handed it before the edit that names it.
            if let Some(remote) = &mut self.remote {
                remote.line.keep(&name, bytes.clone());
            }
            self.assets.insert(name, bytes);
        }
    }

    /// Puts pictures into the document where the selection is, and keeps their files.
    fn insert(&mut self, pictures: Vec<Picture>, platform: &mut Desktop) {
        self.view.insert_pictures(pictures, None, platform);
        self.keep(platform.take_assets());
    }

    /// Whether the window has no file and nothing to put in one. Such a window gives way to a document
    /// that is opened from it.
    fn pristine(&self) -> bool {
        self.place.home().is_none() && !has_content(self.view.editor().doc())
    }

    /// Whether the document's file or the copy kept for it is away on another thread.
    fn busy(&self) -> bool {
        self.rest.is_some() || self.reading.is_some() || self.saving.is_some()
    }

    /// Has a picture the page asked for decoded, from wherever its file is now: with this window if no
    /// save has had it or it was fetched, else in the document's file.
    fn want(&mut self, name: String, decoder: &mut Decoder, proxy: &EventLoopProxy<Event>) {
        let unsaved = self.assets.get(&name).or_else(|| self.copy.as_ref()?.entries.get(&name));
        let unsaved = unsaved.or_else(|| self.linked.get(&name));
        let source = match unsaved {
            Some(bytes) => Some(Source::Bytes(bytes.clone())),
            None => self.file().map(|file| Source::Document(file.to_owned())),
        };
        let (id, proxy, within) = (self.window.id(), proxy.clone(), self.widest());
        match source {
            Some(source) => {
                let done = move |name, image| drop(proxy.send_event(Event::Picture(id, name, image, within)));
                decoder.decode(name, source, within, done);
            }
            // It may be in what is being read, or on its way into the file.
            None if self.busy() => self.awaited.push(name),
            None => self.view.set_image(&name, None),
        }
    }

    /// Takes a picture that was decoded for the page, or the word that it could not be had.
    fn picture(&mut self, name: String, image: Option<Image>, within: u32) {
        match image {
            None if self.linked.remove(&name).is_some() => {
                self.view.set_picture_state(&name, Some(PictureState::Failed(NO_PICTURE.to_owned())));
            }
            // A file that does not hold it yet may once the read or the save under way is done.
            None if self.busy() => self.awaited.push(name),
            // The copy of a server's document lacks what others put in since it was brought here.
            None if self.remote.as_mut().is_some_and(|remote| remote.asked.insert(name.clone())) => {
                self.remote.iter_mut().for_each(|remote| remote.lacking.push(name.clone()));
            }
            None => self.view.set_image(&name, None),
            Some(image) => {
                // A picture that is held whole has nothing sharper to give.
                let whole = image.width() < within && self.linked.contains_key(&name);
                self.view.set_image(&name, Some(image));
                match whole {
                    true => drop(self.held.remove(&name)),
                    false => drop(self.held.insert(name, within)),
                }
            }
        }
        self.window.request_redraw();
    }

    /// Asks again for the pictures that waited on a read or a save, and for those held narrower than the
    /// page now draws them, as after a move to a sharper screen.
    fn ask_again(&mut self, decoder: &mut Decoder, proxy: &EventLoopProxy<Event>) {
        let within = self.widest();
        let narrow = self.held.iter().filter(|(_, held)| **held < within).map(|(name, _)| name.clone());
        let mut again: Vec<String> = narrow.collect();
        again.iter().for_each(|name| _ = self.held.insert(name.clone(), within));
        if !self.busy() {
            again.append(&mut self.awaited);
            // The document's fonts waited too, and are looked for after the next frame.
            if self.fonts_seen.is_none() {
                self.window.request_redraw();
            }
        }
        again.into_iter().for_each(|name| self.want(name, decoder, proxy));
    }

    /// Tells the journal where the document's file is, which may have changed.
    fn rehome(&mut self) {
        let Some(mut journal) = self.journal.take() else { return };
        if let Some(file) = self.file() {
            journal.moved(file);
        }
        self.journal = Some(journal);
    }

    /// Tells the window what the person last saw of the document, when who writes here is known and the
    /// document is as new as it gets: once, but for a server's document that cannot be reached. That one
    /// is told by its copy on this device, and again if the server is reached and has more to show.
    /// `merged` says whether the server just sent what others wrote. What comes in after that is seen as
    /// it comes.
    fn say_seen(&mut self, merged: bool) {
        use letters_sync::Link::{Connecting, Live, Offline, Refused};
        if !self.named || self.last_seen.is_none() {
            return;
        }
        match self.remote.as_ref().map(|remote| (remote.started, remote.line.link())) {
            None | Some((_, Live(_))) => {
                let seen = self.last_seen.take();
                // A server that had nothing more leaves what was said by the copy as it was answered.
                if !self.seen_offline || merged {
                    self.view.set_last_seen(seen);
                }
            }
            Some((true, Offline | Refused(_))) if !self.seen_offline => {
                self.seen_offline = true;
                self.view.set_last_seen(self.last_seen.clone());
            }
            Some((_, Connecting | Offline | Refused(_))) => {}
        }
    }

    /// Keeps what the person has seen of the document, which is all the window shows of it, for the note
    /// of what others wrote when it is next opened. Kept at each save, so that a crash loses little.
    fn saw(&mut self, closing: bool) {
        let Some(id) = self.id.as_deref().filter(|_| self.keeps && self.file().is_some()) else { return };
        let revision = self.view.editor().model().revision();
        if self.marked.as_ref() != Some(&revision) {
            self.seen.saw(id, &revision);
            self.marked = Some(revision.clone());
        }
        // The account of a server's document is told too, for the person's other devices.
        if let Some(remote) = &self.remote {
            let (session, id) = (remote.line.session().clone(), remote.opened.id.clone());
            self.telling.tell(&revision, closing, move |seen| {
                let told = session.saw(&id, &seen);
                if let Err(refused) = &told {
                    eprintln!("letters: the server was not told what was seen of a document: {refused}");
                }
                told.is_ok()
            });
        }
    }

    /// Lets the journal go as the window closes with everything saved. A document that was never worth
    /// a file is not worth bringing back either. Returns the telling of what was seen that is still on
    /// its way to a server, which the app gives a moment before it ends.
    fn leave(&mut self) -> Option<JoinHandle<bool>> {
        self.saw(true);
        if let Some(remote) = &mut self.remote {
            remote.line.stop();
        }
        if let Some(mut journal) = self.journal.take()
            && self.file().is_none()
        {
            journal.forget();
        }
        self.telling.leave()
    }

    /// The top bar says whether the document is saved. The title adds what the bar has no word for: a
    /// save that failed.
    fn retitle(&mut self) {
        let remote = self.remote.as_ref();
        // A server's document that is all in its copy here is saved once the server keeps it.
        let progress = remote.filter(|remote| remote.started).map(|remote| remote.line.progress());
        self.view.set_connection(remote.map(Remote::connection));
        self.view.set_stored(remote.map(Remote::stored));
        self.view.set_access(remote.map_or(Access::Edits, Remote::access));
        self.view.set_allows(remote.map_or_else(Allows::default, Remote::allows));
        self.view.set_save_state(match (self.save_failed, self.place.home().is_some(), self.unsaved()) {
            (true, _, _) => SaveState::Failed,
            // A document with no file yet has nothing to say about saving.
            (false, false, _) => SaveState::Unsaved,
            (false, true, true) => SaveState::Saving,
            (false, true, false) => match progress {
                Some(Progress::Saving) => SaveState::Saving,
                Some(Progress::Edited) => SaveState::Offline,
                Some(Progress::Saved) | None => SaveState::Saved,
            },
        });
        let state = if self.save_failed { " · Not saved" } else { "" };
        self.window.set_title(&format!("{}{state}", self.name()));
    }

    /// Tells the view what the document is called and where it is kept, when either has changed. A
    /// document with no name of its own is told so, and the view shows its first heading meanwhile.
    fn relocate(&mut self, library: &Library) {
        // A server's document is called what the server says and is where the server says. One the server
        // has only a stand-in name for takes its first heading's, as a new document does.
        let named = self.remote.as_ref().map_or(!self.place.naming(), |remote| remote.opened.named);
        let name = named.then(|| self.place.name());
        let at = self.remote.as_ref().map(|remote| PathBuf::from(remote.folder()));
        let folder = at.unwrap_or_else(|| self.place.folder(library));
        if self.located.as_ref().is_some_and(|(told, at)| (told, at) == (&name, &folder)) {
            return;
        }
        // It moves among the folders of its space, which are all the places there are for it.
        let folders = match &self.remote {
            Some(remote) => {
                let shown = |shelf: &Shelf| letters_ui::Folder {
                    path: remote.key(&shelf.path),
                    name: shelf.name.clone(),
                    depth: shelf.depth,
                };
                remote.shelves.iter().map(shown).collect()
            }
            None => library.folders(&folder).into_iter().filter_map(shown).collect(),
        };
        self.view.set_location(Location {
            name: name.clone(),
            folder: folder.to_string_lossy().into_owned(),
            folders,
            closed: self.remote.is_some(),
        });
        self.located = Some((name, folder));
    }

    /// Tells the view where copies of a server's document go: among the person's own folders, since the
    /// folders the bar lists are the server's. A copy of any other document goes beside it.
    fn recopy(&mut self, library: &Library) {
        let own = self.remote.is_some().then(|| {
            let folder = library.default_folder();
            Location {
                name: None,
                folders: library.folders(&folder).into_iter().filter_map(shown).collect(),
                folder: folder.to_string_lossy().into_owned(),
                closed: false,
            }
        });
        self.view.set_copy_location(own);
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

    /// Tells the view how far the document is trusted: whether its formulas may link, and whether it is
    /// held in safe mode. One made in this window is trusted. One that was opened is unknown until its id
    /// is read.
    fn retrust(&mut self, trust: &Trust, library: &Library) {
        let links = self.trusted(trust);
        if std::mem::replace(&mut self.maths_links, links) != links {
            self.view.set_maths_links(links);
        }
        // Nothing is said of a document that has not said which one it is: the bar would come and go.
        if self.made_here || self.id.is_some() {
            let folder = self.folder().filter(|folder| !lies_at(folder, &library.default_folder()));
            let named = folder.and_then(|folder| Some(folder.file_name()?.to_string_lossy().into_owned()));
            self.view.set_safe_mode((!links).then_some(SafeMode { folder: named }));
        }
        // What the Settings dialog lists, and whether it offers to stop trusting this document.
        self.view.set_trusted_folders(trust::folders(trust));
        self.view.set_trusted_sources(trust::sources(trust));
        self.view.set_document_trusted(self.id.as_deref().is_some_and(|id| trust.trusts_document(id)));
    }

    /// Whether the document is trusted, and not in safe mode. One that has not said which document it is
    /// is not.
    fn trusted(&self, trust: &Trust) -> bool {
        let mode = match (&self.id, self.place.path()) {
            _ if self.vouched() => letters_trust::Mode::Trusted,
            (Some(id), path) => trust::mode(trust, id, path),
            (None, _) => letters_trust::Mode::Safe,
        };
        mode == letters_trust::Mode::Trusted
    }

    /// The folder the document's file is in, as the system spells it, which is how trusted folders are
    /// matched.
    fn folder(&self) -> Option<PathBuf> {
        Some(fs::canonicalize(self.place.path()?).ok()?.parent()?.to_owned())
    }

    /// What the document may fetch. One made in this window is trusted, and one that was opened is held
    /// to what the person trusts of its id and its folder.
    fn gate<'a>(&self, trust: &'a Trust) -> Gate<'a> {
        gate_of(trust, self.vouched(), self.id.as_deref(), self.place.path())
    }

    /// Whether the document is trusted for where it came from, with nothing asked of what the person
    /// trusts: it was made in this window, or its server vouches for it.
    fn vouched(&self) -> bool {
        (self.made_here && !self.forgotten) || self.remote.as_ref().is_some_and(Remote::vouched)
    }

    /// Looks for the system's dictionaries for the languages the document names and has not asked about
    /// yet, on another thread. One that is found arrives as an event. A language the system has none for
    /// is asked of the document's server, if it has one, which is first asked what it has: the Language
    /// dialog offers those too. One that nobody has is no one's concern.
    fn ask_dictionaries(&mut self, proxy: &EventLoopProxy<Event>, folders: &Arc<[PathBuf]>) {
        let mut wanted = dictionaries::languages(self.view.editor().doc());
        wanted.retain(|language| self.asked.insert(language.clone()));
        let session = self.remote.as_ref().map(|remote| remote.line.session().clone());
        let held = self.served.clone();
        if wanted.is_empty() && (session.is_none() || held.is_some()) {
            return;
        }
        let (id, proxy, folders) = (self.window.id(), proxy.clone(), folders.clone());
        thread::spawn(move || {
            let mut lacking = BTreeSet::new();
            for language in wanted {
                match dictionaries::load(&folders, &language) {
                    Some((name, dictionary)) => {
                        let _ = proxy.send_event(Event::Dictionary(id, name, Box::new(dictionary)));
                    }
                    None => _ = lacking.insert(language),
                }
            }
            let Some(session) = session else { return };
            let held = match (held, session.dictionaries()) {
                (Some(held), _) => held,
                (None, Ok(held)) => {
                    let _ = proxy.send_event(Event::Served(id, held.clone()));
                    held
                }
                // A server that is out of reach is asked again when the document is next saved.
                (None, Err(_)) => return drop(proxy.send_event(Event::Unasked(id, lacking.into_iter().collect()))),
            };
            for (tag, under) in letters_spell::served_dictionaries(lacking.iter().map(String::as_str), &held) {
                if let Some(dictionary) = dictionaries::fetch(&session, &tag) {
                    let _ = proxy.send_event(Event::Dictionary(id, under, Box::new(dictionary)));
                }
            }
        });
    }

    /// What to ask about the document's fonts, when it names any beyond the bundled ones and something
    /// changed since they were last looked for. A document that may be written and is trusted keeps the
    /// faces of the library's families with it.
    fn fonts(&mut self, trust: &Trust) -> Option<chosen::Ask> {
        self.fonts_named.extend(self.view.take_named());
        // A file that is being read has not said which document it is, nor given up its font files, and
        // one that is being saved may be about to hold them. The fonts wait: see `ask_again`.
        if self.busy() {
            self.fonts_seen = None;
            return None;
        }
        let writes = self.remote.as_ref().is_none_or(|remote| remote.access() == Access::Edits);
        let keeps = writes && self.trusted(trust);
        let style = self.view.editor().model().style();
        let (listed, faces) = (style.fonts(), letters_layout::faces(style));
        let mut seen = DefaultHasher::new();
        (&faces, &listed, &self.fonts_named, keeps).hash(&mut seen);
        if self.fonts_seen.replace(seen.finish()) == Some(seen.finish()) {
            return None;
        }
        let bundled = |face: &Named| BUNDLED.iter().any(|own| own.eq_ignore_ascii_case(&face.family));
        if self.fonts_named.is_empty() && listed.is_empty() && faces.iter().all(bundled) {
            return None;
        }
        let held = |name: &str| self.assets.get(name).or_else(|| self.copy.as_ref()?.entries.get(name)).cloned();
        let unsaved = listed.iter().filter_map(|font| Some((font.asset.clone(), held(&font.asset)?))).collect();
        let file = self.file().map(Path::to_owned);
        Some(chosen::Ask { named: self.fonts_named.clone(), listed, unsaved, file, keep: keeps.then_some(faces) })
    }

    /// Takes what was found of the document's fonts: sets its text in the faces, keeps the font files that
    /// are new with it, and says once which families are not to be had.
    fn chose(&mut self, found: chosen::Found, platform: &mut Desktop) {
        let chosen::Found { faces, stored, listed, lacking, unfetched, absent: _ } = found;
        // The files first: the document's server is handed them before the edit that names them.
        self.keep(stored);
        if !faces.is_empty() {
            self.view.add_named_fonts(faces);
        }
        if let Some(listed) = listed {
            self.view.keep_fonts(&listed, platform);
        }
        let says = [(lacking, NO_FONT, "OK", FONT_KEY), (unfetched, NO_FETCH, TRY_AGAIN, FETCH_KEY)];
        for (families, sentence, button, key) in says {
            for family in families.into_iter().filter(|family| self.fonts_said.insert(family.to_lowercase())) {
                let sentence = sentence.replace("{}", &family);
                self.view.notify(Notice { sentence, button: button.into(), key: format!("{key}{family}") });
            }
        }
    }

    /// Names the languages there are dictionaries for, for the Language dialog and the settings to offer:
    /// the `system`'s, and those of the document's server.
    fn tell_languages(&mut self, system: &[String]) {
        let served = self.served.iter().flatten();
        let languages: BTreeSet<&String> = system.iter().chain(served).collect();
        self.view.set_languages(languages);
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
        // The lights are read while they stand in the window, and remembered while they do not.
        if let Some(lights) = letters_frame::lights(&self.window) {
            self.lights = Some(lights);
        }
        let chrome = Host::HERE.chrome(buttons, maximized, full_screen, self.bar, self.lights);
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
        self.presenter.shade(self.chrome.frame == Frame::Floating, self.dark());
    }

    /// Paints one frame and puts it on screen.
    fn redraw(&mut self, now_ms: u64) -> Result<(), softbuffer::SoftBufferError> {
        // Idle work waits behind this frame, and the view asks to be called again for it.
        self.wake_ms = self.view.animate(now_ms);
        self.list.items.clear();
        self.view.paint(&mut self.list);
        // A window that floats in a frame of Letters' own has round corners wherever it can.
        let dark = self.dark();
        let line = tokens::LINE_STRONG.resolve(if dark { Scheme::Dark } else { Scheme::Light });
        let line = (self.chrome.frame == Frame::Floating).then(|| Color::rgba(line.r, line.g, line.b, line.a));
        self.presenter.present(&mut self.list, self.view.images(), background(dark), line)
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
        if let Some(file) = &self.says {
            // Whole each time, and to assistive technology too: a test is in no hurry.
            let tree = self.view.access();
            let said = tree.nodes.iter().flat_map(|(_, node)| [node.label(), node.value()]).flatten();
            say(file, &said.collect::<Vec<_>>().join("\n"));
            (self.access_stale, self.access_whole) = (false, false);
            self.access.update_if_active(|| tree);
        } else if std::mem::take(&mut self.access_stale) {
            // Assistive technology that has just started is told everything, and after that what changed.
            let (view, whole) = (&mut self.view, &mut self.access_whole);
            self.access.update_if_active(|| if std::mem::take(whole) { view.access() } else { view.access_changes() });
        }
        if self.view.edits() != self.noticed_edits {
            self.noticed_edits = self.view.edits();
            // Where the last session ended, or writing rested for long, is a version of its own.
            if let Some((at, kind)) = self.rests.edited(now) {
                self.mark(&at, kind, None);
            }
            self.rests.rest(self.view.editor().model().revision(), now);
            self.save_at = Some(now + SAVE_AFTER);
            self.journal_at.get_or_insert(now + JOURNAL_AFTER);
            self.retitle();
        }
        // The line to the document's server hears of whatever changed.
        let told = self.tell_line(now);
        self.told(told, now);
    }

    /// Ties the window to the server its document lives on, whose copy it has open. The line starts once
    /// the file's history is read.
    fn connect(
        &mut self,
        session: Arc<Session>,
        reached: Reached,
        proxy: &EventLoopProxy<Event>,
        library: &Library,
        now: Instant,
    ) {
        let (id, proxy) = (self.window.id(), proxy.clone());
        let tell: connected::Tell = Arc::new(move |said| drop(proxy.send_event(Event::Heard(id, said))));
        let origin = session.server().origin().to_owned();
        let host = origin.trim_start_matches("https://").to_owned();
        let mut line = Line::new(session, &reached.opened, &reached.path, Roots::System, tell);
        // What no save has put in the copy's file yet, the server may lack too.
        self.assets.iter().for_each(|(name, bytes)| line.keep(name, bytes.clone()));
        // The copy is kept under the document's id, and is no file the person copied by hand or would
        // look for among their own, whatever else has its id.
        self.place.call(&reached.opened.name);
        (self.twinned, self.listed) = (true, self.place.path().map(Path::to_owned));
        if !reached.opened.author.is_empty() {
            self.view.set_author(&reached.opened.author);
            self.named = true;
        }
        // A window that is tied again, as after a new sign-in, lets go of the line it had.
        let owes = self.remote.take().is_some_and(|mut before| {
            before.line.stop();
            before.started
        });
        self.remote = Some(Remote {
            line,
            opened: reached.opened,
            host,
            origin,
            born: now,
            started: false,
            owes,
            selection: None,
            asked: BTreeSet::new(),
            lacking: Vec::new(),
            asks: Vec::new(),
            placing: false,
            shares: Vec::new(),
            sharing: false,
            live: false,
            looked: None,
            shelves: Vec::new(),
            listed: None,
        });
        self.recopy(library);
        self.recomment();
        self.located = None;
        self.relocate(library);
        self.go_online(now);
        self.retitle();
    }

    /// Starts the line to the document's server, once the editor has the file's history.
    fn go_online(&mut self, now: Instant) {
        if self.id.is_none() {
            return;
        }
        let Some(remote) = self.remote.as_mut().filter(|remote| !remote.started) else { return };
        remote.started = true;
        let owes = remote.owes || self.journal.as_ref().is_some_and(Journal::owes);
        let told = remote.line.start(owes, remote.ms(now));
        self.told(told, now);
    }

    /// Asks the server again, after it ended the line for a reason the person may have mended since.
    fn try_again(&mut self, now: Instant) {
        if let Some(remote) = &mut self.remote {
            (remote.started, remote.owes) = (false, true);
        }
        self.go_online(now);
        self.retitle();
    }

    /// Says who else has the document open, each where their caret is in the text as it reads now.
    fn show_peers(&mut self) {
        let Some(remote) = &self.remote else { return };
        let model = self.view.editor().model();
        let shown = |peer: &letters_sync::Peer| letters_ui::Peer {
            session: peer.session.0.into(),
            name: peer.name.clone(),
            colour: peer.colour,
            selection: peer.selection.as_ref().and_then(|passage| passage.selection(model)),
        };
        let peers = remote.line.peers().iter().map(shown).collect();
        self.view.set_peers(peers);
    }

    /// Tells the line to the server how the document stands now, and gives what the line leaves to do.
    fn tell_line(&mut self, now: Instant) -> Vec<Told> {
        let Some(remote) = self.remote.as_mut().filter(|remote| remote.started) else { return Vec::new() };
        let (editor, ms) = (self.view.editor(), remote.ms(now));
        // A style set here is in the history, and travels with the edits.
        let mut told = remote.line.changed(Local { model: Some(editor.model()), notes: self.view.notes() }, ms);
        // The others see no caret while the page is not what shows here.
        if self.view.selection() != remote.selection.as_ref() {
            remote.selection = self.view.selection().cloned();
            told.extend(remote.line.selected(editor.model(), remote.selection.as_ref(), ms));
        }
        told
    }

    /// The line to the document's server was told something on another thread.
    fn heard(&mut self, said: Said, now: Instant) {
        let Some(remote) = self.remote.as_mut() else { return };
        let (editor, ms) = (self.view.editor(), remote.ms(now));
        let local = Local { model: Some(editor.model()), notes: self.view.notes() };
        let told = remote.line.heard(said, local, ms);
        self.told(told, now);
    }

    /// The time the line to the document's server asked to be woken at has come.
    fn tick(&mut self, now: Instant) {
        let Some(remote) = self.remote.as_mut() else { return };
        let told = remote.line.tick(remote.ms(now));
        self.told(told, now);
    }

    /// Does what the line to the document's server left to do, and tells it of what that changed.
    fn told(&mut self, told: Vec<Told>, now: Instant) {
        let mut told = VecDeque::from(told);
        let (mut stands, mut merged) = (false, false);
        while let Some(next) = told.pop_front() {
            match next {
                Told::Merge(edits) => {
                    merged = true;
                    let before = self.view.edits();
                    if let Err(error) = self.view.change(|editor| editor.merge(&edits)) {
                        eprintln!("letters: could not take in what the server sent of {}: {error}", self.name());
                    }
                    // What others wrote is no writing here, so no version is marked for it. The copy on
                    // this device takes it with its next save.
                    if self.noticed_edits == before {
                        self.noticed_edits = self.view.edits();
                    }
                    self.save_at.get_or_insert(now + SAVE_AFTER);
                    stands = true;
                    told.extend(self.tell_line(now));
                }
                Told::Notes(notes) => {
                    let mut joined = self.view.notes().clone();
                    if joined.merge(&notes) {
                        // A note says where its text was on the device that wrote it.
                        joined.follow(self.view.editor().model());
                        self.view.set_notes(joined);
                        // A note is no edit of the text, but the copy is to hold it.
                        self.marks = true;
                        self.save_at.get_or_insert(now + SAVE_AFTER);
                    }
                    stands = true;
                    told.extend(self.tell_line(now));
                }
                // The work stays in the copy here. The person is told why it went no further.
                Told::Refused(refusal) => self.view.set_notice(Some(&refusal.to_string())),
                Told::Unsent(sentence) => self.view.set_notice(Some(&sentence)),
                Told::Unkept(_) => self.view.set_notice(Some(UNKEPT)),
                // The file goes to the server whole, once a save has put everything in it.
                Told::Whole => {
                    if self.unsaved() || self.saving.is_some() {
                        self.save_at = Some(now);
                    } else if let Some(remote) = &mut self.remote {
                        remote.line.saved();
                    }
                }
                Told::Link => {
                    stands = true;
                    // A document may have been renamed or moved while there was no line to its server.
                    if let Some(remote) = &mut self.remote {
                        let live = matches!(remote.line.link(), letters_sync::Link::Live(_));
                        (remote.live, remote.looked) = (live, remote.looked.filter(|_| live));
                        remote.look(now);
                    }
                }
                Told::Progress => stands = true,
                // Said below, with where each of them is now.
                Told::Peers => {}
            }
        }
        // With the server caught up with, what others wrote since the last visit is all here.
        self.say_seen(merged);
        // Whatever was written here or there since may have moved the text under the others' carets.
        self.show_peers();
        if stands {
            self.recomment();
            self.retitle();
        }
    }

    /// Takes how the document stands at its server, which was asked about its name and place. `answers`
    /// says that the bar asked and waits to be told. Gives the document for the list of recent ones, if
    /// the server said how it stands and the list has it otherwise.
    fn placed(&mut self, answers: bool, placed: Placed, library: &Library) -> Option<servers::Lately> {
        let remote = self.remote.as_mut()?;
        remote.placing = false;
        let said = placed.opened.is_some();
        if let Some(now) = placed.opened {
            let opened = &mut remote.opened;
            (opened.name, opened.named, opened.space, opened.path) = (now.name, now.named, now.space, now.path);
            // What the person may do holds while there is no line, and says whether they share it.
            (opened.role, opened.allows) = (now.role, now.allows);
            self.place.call(&remote.opened.name);
        }
        if let Some(shelves) = placed.shelves.filter(|shelves| *shelves != remote.shelves) {
            remote.shelves = shelves;
            // The bar lists them, whether or not the document is where it was.
            self.located = None;
        }
        match placed.refused {
            // The name field comes back with what was typed, so the bar is told only what changed.
            Some(refused) => {
                self.relocate(library);
                self.view.set_notice(Some(&refused));
            }
            None => {
                if answers {
                    self.located = None;
                }
                self.relocate(library);
            }
        }
        self.retitle();
        self.remote.as_mut().filter(|_| said && self.keeps).and_then(Remote::lately)
    }

    /// Starts reading what the first paint did not need, on another thread.
    ///
    /// A document that would open in safe mode is read as a stranger's: see [`Rest::read`].
    fn start_read(&mut self, proxy: &EventLoopProxy<Event>, journals: &Journals, trust: &Trust) {
        let Some(rest) = self.rest.take() else { return };
        let (id, proxy, replica) = (self.window.id(), proxy.clone(), self.view.editor().model().replica());
        let journals = self.keeps.then(|| journals.folder().to_owned());
        let (vouched, trust, path) = (self.vouched(), trust.clone(), self.place.path().map(Path::to_owned));
        let wary = move |id: &str| !vouched && trust::mode(&trust, id, path.as_deref()) != letters_trust::Mode::Trusted;
        self.reading = Some(thread::spawn(move || {
            let working = rest.read(replica, journals.as_deref(), wary).map_err(|error| error.unopened());
            let _ = proxy.send_event(Event::Read(id));
            working
        }));
    }

    /// Collects the working copy and gives the editor the file's history, reading here if that had not
    /// started and waiting if it had not finished. Fails once, when the file could not be read for saving.
    fn finish_read(&mut self, journals: &mut Journals) -> Result<(), String> {
        let working = match (self.reading.take(), self.rest.take()) {
            // A read that stopped unexpectedly has said so itself, and there is no plain reason to give.
            (Some(reading), _) => reading.join().unwrap_or(Err("")),
            (None, Some(rest)) => {
                let replica = self.view.editor().model().replica();
                // With nothing at hand that says who is trusted, only what is vouched for here is.
                let (journals, vouched) = (self.keeps.then(|| journals.folder()), self.vouched());
                rest.read(replica, journals, |_| !vouched).map_err(|error| error.unopened())
            }
            (None, None) => return Ok(()),
        };
        let mut own = None;
        let read = match working {
            Ok(Working { copy, history, news, claim, journalled }) => {
                // The journal of this window's own document, set aside as the file was read or before.
                own = claim.as_ref().and_then(|claim| claim.aside().cloned());
                if let Some(history) = history {
                    let _ = self.view.change(|editor| Ok::<_, Infallible>(editor.attach(history)));
                }
                if let Some(news) = news
                    && let Err(error) = self.view.change(|editor| editor.merge(&news))
                {
                    eprintln!("letters: could not bring back unsaved work in {}: {error}", self.name());
                }
                self.id = Some(copy.id.clone());
                // Read once, before a save of this visit keeps what is seen now in its place.
                if self.keeps {
                    // The account of a server's document knows what its person saw on any device. This
                    // device's own word serves a file, and a server that said nothing.
                    let said = self.remote.as_ref().and_then(|remote| remote.opened.last_seen.as_deref());
                    let said: Option<Revision> = said.and_then(|said| said.parse().ok());
                    self.last_seen = said.clone().or_else(|| self.seen.last(&copy.id));
                    self.telling = Telling::after(said);
                    self.say_seen(false);
                }
                // What the file held is where the last session ended, should this one write anything.
                let found = copy.model().revision();
                let imported = self.imported.take();
                match &imported {
                    // A document that was just brought in says so, and from what.
                    Some(source) => {
                        self.mark(&found, TagKind::Import, Some(source));
                        self.save_at = Some(Instant::now() + SAVE_AFTER);
                    }
                    None if self.made_here && self.file().is_none() => {}
                    None => self.rests.opened(found),
                }
                // The window keeps the notes from here on. A comment written before they arrived stays.
                let mut notes = copy.notes.clone();
                let early = notes.merge(self.view.notes());
                self.view.set_notes(notes);
                // With its notes in the window, an import can say how many it left.
                if imported.is_some() {
                    self.view.say_imported();
                }
                if !early {
                    self.journalled_notes = self.view.notes().clone();
                }
                if let Some(claim) = claim {
                    let journal = journals.open(claim, &copy.id, &copy.created, self.file());
                    // What an earlier session left unsaved is saved now.
                    if journal.owes() {
                        self.save_at = Some(Instant::now());
                    }
                    (self.journal, self.journalled) = (Some(journal), journalled);
                    // What was put in or typed before the history arrived is kept all the same.
                    if let Some(journal) = &mut self.journal {
                        self.assets.iter().for_each(|(name, bytes)| journal.asset(name, bytes));
                    }
                    self.record();
                }
                self.copy = Some(copy);
                self.recomment();
                Ok(())
            }
            Err(why) => {
                self.save_failed = self.keeps;
                self.retitle();
                Err(format!("{} is open, but changes to it cannot be saved{why}.", self.name()))
            }
        };
        // Said once the document is whole on screen, and whether or not it can be saved.
        let told: Vec<_> = std::mem::take(&mut self.aside).into_iter().map(|aside| (aside, None)).collect();
        for (aside, here) in told.into_iter().chain(own.map(|aside| (aside, Some(true)))) {
            let shows = || aside.home.as_deref().is_some_and(|home| self.place.path() == Some(home));
            let sentence = aside.said(here.unwrap_or_else(shows));
            // The notice is known by where its journal is kept, which its button shows.
            let key = aside.kept.to_string_lossy().into_owned();
            self.view.notify(Notice { sentence, button: SHOW_FILE.into(), key });
            self.kept.push(aside.kept);
        }
        read
    }

    /// What the copy needs to catch up with the editor: the edits it has not seen, and the revision they
    /// lead to. The copy takes the style too, when it was changed here.
    fn edits_for(&mut self, copy: &mut WorkingCopy) -> (Vec<u8>, Revision) {
        if self.view.style() != &self.sent_style {
            self.sent_style = self.view.style().clone();
            // The window only offers styles the file format takes.
            let _ = copy.set_style(self.sent_style.clone());
        }
        // The assets go into the file with the edits that name them, and so do the versions marked.
        copy.entries.append(&mut self.assets);
        self.marks = false;
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
        if !self.unsaved() && !self.owes() && self.merging.is_empty() && self.left.is_none() {
            // Only the name was due.
            return self.relist(library);
        }
        let Some(mut copy) = self.copy.take() else { return };
        if let Some(journal) = &mut self.journal {
            journal.saving();
        }
        let (edits, revision) = self.edits_for(&mut copy);
        let (id, proxy, since) = (self.window.id(), proxy.clone(), revision.clone());
        let notes = self.view.notes().clone();
        let others = self.carry();
        let save = thread::spawn(move || {
            let result = home.exchange(&mut copy, &edits, notes, &since, &others).map_err(|error| error.to_string());
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
                self.recomment();
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
                match self.copy.take() {
                    Some(copy) => {
                        self.catch_up(&copy, &revision);
                        self.copy = Some(copy);
                    }
                    None => drop(self.requeue()),
                }
                self.save_failed = true;
                self.save_at = Some(now + RETRY_AFTER);
            }
        }
        self.retitle();
    }

    /// The files the save that is starting merges, which are then on their way in.
    fn carry(&mut self) -> Vec<PathBuf> {
        self.carried = std::mem::take(&mut self.merging);
        self.carried.iter().map(|(path, _)| path.clone()).collect()
    }

    /// After a save that failed with files to merge: the copy may hold them all the same, where no later
    /// save would find them new. The editor, at `revision`, takes what the copy holds, and the files
    /// wait for the next save to say how they fared.
    fn catch_up(&mut self, copy: &WorkingCopy, revision: &Revision) {
        if !self.requeue() {
            return;
        }
        let news = copy.model().updates_since(revision);
        let mut met = false;
        if let Err(error) = self.view.change(|editor| editor.merge(&news).inspect(|change| met = moved(change))) {
            eprintln!("letters: could not take in what was merged into {}: {error}", self.name());
        }
        let mut notes = self.view.notes().clone();
        if notes.merge(&copy.notes) {
            self.view.set_notes(notes);
        }
        if met {
            self.rests.merged(Instant::now());
            let at = self.view.editor().model().revision();
            self.mark(&at, TagKind::Merge, None);
        }
    }

    /// Puts the files a save was to merge back first in line, and says whether there were any.
    fn requeue(&mut self) -> bool {
        let carried = std::mem::take(&mut self.carried);
        let any = !carried.is_empty();
        self.merging.splice(0..0, carried);
        any
    }

    /// Looks beside the document's file for conflict copies of it, on another thread. What is found
    /// arrives as an event. Nothing is looked for until the file exists and has said which document it is.
    ///
    /// The first look is also for the file the document was copied from by hand, beside it or among the
    /// recent documents.
    fn look_for_copies(&mut self, proxy: &EventLoopProxy<Event>, library: &Library) {
        // No sync tool leaves copies beside the copy of a server's document.
        let home = self.place.home().filter(|home| self.keeps && home.written() && self.remote.is_none());
        let (Some(id), Some(home)) = (self.id.clone(), home.map(|home| home.path().to_owned())) else { return };
        let (window, proxy) = (self.window.id(), proxy.clone());
        let recent = || library.recents().into_iter().map(|recent| recent.path).collect::<Vec<_>>();
        let known = (!std::mem::replace(&mut self.twinned, true)).then(recent);
        thread::spawn(move || {
            let found = conflicts::look(&home, &id, known.as_deref());
            let _ = proxy.send_event(Event::Beside(window, home, Box::new(found)));
        });
    }

    /// Tells the view what was found beside the file at `home`: the conflict copies that hold what the
    /// document lacks and are not on their way in already, and the file the document is itself a
    /// conflict copy of.
    fn found_beside(&mut self, home: &Path, found: conflicts::Beside, library: &Library) {
        // The document may have been moved since its folder was read.
        if self.place.path() != Some(home) {
            return;
        }
        let model = self.view.editor().model();
        let waiting = |path: &Path| self.merging.iter().chain(&self.carried).any(|(other, _)| other == path);
        let adds = |copy: &conflicts::Copy| copy.adds_to(model) && !waiting(&copy.path);
        let copies: Vec<PathBuf> = found.copies.into_iter().filter(adds).map(|copy| copy.path).collect();
        let original = found.original.filter(|file| file.related_to(model)).map(|file| file.path);
        // Told once whatever was found, and then only of a change: a bar the person hid stays hidden.
        if self.copies.as_ref() != Some(&copies) {
            self.tell_copies(copies, library);
        }
        if self.original != original {
            self.view.set_conflict_original(original.as_deref().map(|path| merge_file(path, library)));
            self.original = original;
        }
        if found.twin.is_some() {
            self.twin = found.twin;
        }
    }

    /// Makes the document one of its own, if it is a copy made by hand that has just been written in:
    /// it takes a new id, which its next save writes, and keeps its history. Its original is the one
    /// document of the old id again.
    fn become_own(&mut self, journals: &mut Journals) {
        if self.twin.is_none() || self.view.edits() == self.saved_edits {
            return;
        }
        // The copy is away while a save runs. A journal that holds an earlier session's work goes by
        // the old id until a save has found that work in the file.
        if self.saving.is_some() || self.journal.as_ref().is_some_and(Journal::owes) {
            return;
        }
        let (Some(copy), Some(was)) = (&mut self.copy, self.id.clone()) else { return };
        let id = platform::document_id();
        copy.fork(id.clone());
        // Journals go by id, and the old one is the original's from here on. Nothing written in this
        // window is in it.
        self.journal = None;
        if self.keeps
            && let Some(claim) = journal::claim(journals.folder(), &id)
        {
            self.journal = Some(journals.open(claim, &id, &copy.created, self.place.path()));
        }
        (self.id, self.forked, self.twin) = (Some(id.clone()), Some((was, id)), None);
    }

    /// Leaves the conflict copy the window shows for `original`, the file it is a copy of: the next save
    /// goes there, and merges with what that file holds. Fails with what to tell the person when the
    /// copy's own file could not first be brought up to date.
    fn leave_for(&mut self, original: PathBuf, library: &Library, journals: &mut Journals) -> Result<(), String> {
        // Everything is in the copy's file first, so that nothing is only in memory if the original
        // does not take the merge.
        self.save_now(true, library, journals)?;
        let Some(copy) = self.place.path().map(Path::to_owned) else { return Ok(()) };
        self.place.saved(Home::at(&original));
        self.rehome();
        self.left = Some((copy, original));
        self.original = None;
        self.view.set_conflict_original(None);
        self.retitle();
        self.relocate(library);
        Ok(())
    }

    /// Ends the move from a conflict copy to its original, now that a save has gone through: the copy
    /// goes to the trash if the original took the document. Returns the copy's name if it did.
    fn arrived(&mut self, copy: &Path, original: &Path) -> Option<String> {
        if self.place.path() != Some(original) {
            // The save could not replace the original, and wrote the document beside it once more.
            self.view.refuse_merge(&format!(
                "This document could not be merged into {}, and is kept as {}.",
                named(original),
                self.place.path().map(named).unwrap_or_default()
            ));
            return None;
        }
        if let Err(error) = conflicts::discard(copy) {
            eprintln!("letters: could not move {} to the trash: {error}", copy.display());
            self.view.refuse_merge(&format!("{} was merged, but could not be moved to the trash.", named(copy)));
        }
        Some(named(copy))
    }

    fn tell_copies(&mut self, copies: Vec<PathBuf>, library: &Library) {
        self.view.set_conflict_copies(copies.iter().map(|path| merge_file(path, library)).collect());
        self.copies = Some(copies);
    }

    /// Whether the person's trust lets a file they picked be merged into the document without a question.
    /// Only a trusted folder vouches for such a file, and links are followed first, since folders are
    /// matched by name.
    fn may_merge(&self, trust: &Trust, file: &Path) -> bool {
        let real = fs::canonicalize(file).ok();
        let decision = real.as_deref().and_then(Path::to_str).map(|file| self.gate(trust).merge(file));
        decision == Some(MergeDecision::Merge)
    }

    /// Deals with the files a save was to merge: a conflict copy that is in the document goes to the
    /// trash, and the person reads of each file that is not. Returns the names of those that are.
    fn merged(&mut self, merged: Vec<Merged>, library: &Library) -> Vec<String> {
        let carried = std::mem::take(&mut self.carried);
        let (mut names, mut refused) = (Vec::new(), Vec::new());
        for Merged { path, result } in merged {
            let copy = carried.iter().any(|(carried, copy)| *copy && *carried == path);
            match result {
                Ok(()) => {
                    names.push(named(&path));
                    if copy && let Err(error) = conflicts::discard(&path) {
                        eprintln!("letters: could not move {} to the trash: {error}", path.display());
                        refused.push(format!("{} was merged, but could not be moved to the trash.", named(&path)));
                    }
                }
                Err(files::Error::Format(letters_format::Error::Unmerged)) => refused.push(format!(
                    "{} could not be merged: it is not a copy of this document, or a newer Letters wrote it.",
                    named(&path)
                )),
                Err(error) => refused.push(format!("{} could not be read{}.", named(&path), error.unopened())),
            }
        }
        if !refused.is_empty() {
            self.view.refuse_merge(&refused.join("\n"));
        }
        // A copy that went is no longer offered, whenever its folder is next looked at.
        if let Some(copies) = &self.copies {
            let left: Vec<PathBuf> = copies.iter().filter(|copy| copy.exists()).cloned().collect();
            if left.len() != copies.len() {
                self.tell_copies(left, library);
            }
        }
        names
    }

    /// Says whether comments can be written, now that the copy is back from a read or a save: not on
    /// notes this build could not read, where a save that added one would be refused.
    fn recomment(&mut self) {
        if let Some(copy) = &self.copy {
            self.view.refuse_comments(copy.notes_unread().then_some(NO_COMMENTS));
        }
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
        let Exchange { home, news, notes, held, merged } = exchange;
        // A merge the person asked for is a version however lately another was marked.
        let left = self.left.take();
        let asked = left.is_some() || merged.iter().any(|merged| merged.result.is_ok());
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
        if let Some(remote) = &mut self.remote {
            remote.line.saved();
        }
        let typed_since = self.view.edits() != edits;
        let mut met = false;
        if let Some(news) = news
            && let Err(error) = self.view.change(|editor| editor.merge(&news).inspect(|change| met = moved(change)))
        {
            eprintln!("letters: could not take in what {} held: {error}", self.name());
        }
        // What others wrote in the margin joins what was written here, since the save started too.
        if let Some(theirs) = notes {
            let mut merged = self.view.notes().clone();
            if merged.merge(&theirs) {
                self.view.set_notes(merged);
            }
        }
        // What came from the file is in the file, so it is no edit to save.
        self.saved_edits = if typed_since { edits } else { self.view.edits() };
        if !typed_since {
            self.noticed_edits = self.saved_edits;
        }
        // Another writer's words changed the document: where the two met is a version. A mark of theirs
        // alone changes nothing to read, and is not answered with one, or two windows would never rest.
        if met && (self.rests.merged(Instant::now()) || asked) {
            let at = self.view.editor().model().revision();
            self.mark(&at, TagKind::Merge, None);
        }
        let mut names = self.merged(merged, library);
        names.extend(left.and_then(|(copy, original)| self.arrived(&copy, &original)));
        if !names.is_empty() {
            // Said where "Saved" stands, so that is said first.
            self.retitle();
            self.view.set_merged(names);
        }
        self.saw(false);
    }

    /// Saves to `home` on this thread, with the copy the window keeps for it.
    fn exchange(&mut self, home: &Home, copy: &mut WorkingCopy, library: &Library) -> Result<(), files::Error> {
        if let Some(journal) = &mut self.journal {
            journal.saving();
        }
        let (edits, revision) = self.edits_for(copy);
        let others = self.carry();
        match home.exchange(copy, &edits, self.view.notes().clone(), &revision, &others) {
            Ok(exchange) => self.saved(exchange, self.view.edits(), revision, library),
            Err(error) => {
                self.unheld();
                self.catch_up(copy, &revision);
                return Err(error);
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
        self.become_own(journals);
        // A copy that could not be read was reported when that failed, and holds nothing back now.
        let Some(mut copy) = self.copy.take() else { return Ok(()) };
        self.record();
        let merges = |window: &Self| !window.merging.is_empty() || window.left.is_some();
        let due = |window: &Self| window.unsaved() || window.owes() || merges(window);
        let home = self.settle(caret, library, now).filter(|_| due(self));
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
        self.recomment();
        match saved {
            Some(Ok(())) => self.save_at = None,
            Some(Err(error)) => {
                self.save_failed = true;
                self.retitle();
                eprintln!("letters: could not save {}: {error}", self.name());
                return Err(error.unsaved(&self.name()));
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

/// The variable that names a file for a window to write its words to, a line for each thing it names
/// or holds, as assistive technology hears them. For a test that drives a real window and has to read
/// what it shows. The file holds the document's text, so nothing is written unless it is set.
pub const SAYS: &str = "LETTERS_SAYS";

/// Writes what a window says beside `file` and moves it there, so that a reader finds all of it or the
/// words before.
fn say(file: &Path, said: &str) {
    let beside = file.with_extension("new");
    if let Err(error) = fs::write(&beside, said).and_then(|()| fs::rename(&beside, file)) {
        eprintln!("letters: could not write what the window says: {error}");
    }
}

/// What the button of a notice about a journal that was set aside is called.
const SHOW_FILE: &str = "Show file";

/// What is said of a family nothing here has, which is named where the braces are.
const NO_FONT: &str = "This document uses the font {}, which is not on this device. Its text is shown in another font.";
/// What is said of a family of the library that could not be fetched.
const NO_FETCH: &str =
    "The font {} could not be fetched from Google Fonts. Its text is shown in another font until it can be.";
/// What the button that fetches it again is called.
const TRY_AGAIN: &str = "Try again";
/// What the keys of those two notices start with, before the family.
const FONT_KEY: &str = "font:";
const FETCH_KEY: &str = "font-fetch:";

fn alert(parent: Option<&Arc<Window>>, message: &str) {
    eprintln!("letters: {message}");
    let dialog = MessageDialog::new().set_level(MessageLevel::Error).set_title("Letters").set_description(message);
    match parent {
        Some(window) => dialog.set_parent(&**window).show(),
        None => dialog.show(),
    };
}

/// Why no window was made, to end a sentence a person reads. The desktop's own words are for the log, and
/// there is no plain reason to give in their place.
fn no_window(error: &dyn Error) -> &'static str {
    eprintln!("letters: a window could not be made: {error}");
    ""
}

impl App {
    /// Opens a window on a document. With no `opened`, the document is blank and has no file yet, and
    /// `landing` puts the card with the recent documents over it.
    fn open_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        opened: Option<Opened>,
        landing: bool,
    ) -> Result<WindowId, Box<dyn Error>> {
        // A document with no file was made in this app, though perhaps in a session that ended early, or
        // for a path that was named before anything was written there.
        let made_here = opened.as_ref().is_none_or(Opened::made_here);
        let (document, style, place, rest) = match opened {
            Some(Opened { document, style, home, rest }) => {
                (document, style, home.map_or_else(Place::new, Place::at), rest)
            }
            None => {
                let (mut document, style) = (files::blank(), new_style());
                // A new document is written in the person's language.
                if let Some(language) = &self.settings.language {
                    document.language.clone_from(language);
                }
                let rest = Rest::new(&document, &style);
                (document, style, Place::new(), rest)
            }
        };
        // AccessKit must attach before the window is first shown.
        let window = Arc::new(event_loop.create_window(window_attributes(event_loop).with_visible(false))?);
        // Only macOS has this to give: a taller title bar with the lights centred in it, where the bar's
        // own contents are centred too. Without it the lights sit higher than the name beside them.
        let bar = letters_frame::unify_title_bar(&window);
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
            says: std::env::var_os(SAYS).map(PathBuf::from),
            access_stale: false,
            access_whole: true,
            chrome: Chrome::default(),
            appearance: Appearance::System,
            bar,
            lights: None,
            wake_ms: None,
            place,
            keeps: self.typing.is_none() && !self.report_launch,
            listed: None,
            located: None,
            words_seen: None,
            fonts_named: Vec::new(),
            fonts_seen: None,
            fonts_said: BTreeSet::new(),
            fonts_asked: BTreeSet::new(),
            menu_press: false,
            made_here,
            vouch: None,
            id: None,
            maths_links: false,
            asked: BTreeSet::new(),
            served: None,
            saved_edits: 0,
            noticed_edits: 0,
            save_at: None,
            saving: None,
            save_failed: false,
            seen: self.seen.clone(),
            marked: None,
            telling: Telling::default(),
            last_seen: None,
            seen_offline: false,
            named: self.named,
            rest: Some(rest),
            reading: None,
            copy: None,
            sent: None,
            journal: None,
            journalled: None,
            journalled_edits: 0,
            journalled_notes: Notes::default(),
            journal_at: None,
            assets: BTreeMap::new(),
            plan: None,
            dropped: Vec::new(),
            printing: false,
            importing: None,
            import_to: None,
            imported: None,
            aside: Vec::new(),
            kept: Vec::new(),
            rests: Rests::default(),
            marks: false,
            copies: None,
            merging: Vec::new(),
            carried: Vec::new(),
            twin: None,
            twinned: false,
            forked: None,
            forgotten: false,
            remote: None,
            original: None,
            left: None,
            awaited: Vec::new(),
            links: Vec::new(),
            linked: BTreeMap::new(),
            held: BTreeMap::new(),
        };
        window.view.set_application(true);
        window.view.set_reduced_motion(self.reduced_motion);
        if let Some(minutes) = self.offset {
            window.view.set_utc_offset(minutes);
        }
        window.view.set_account_name(self.account.as_deref());
        window.view.set_languages(&self.languages);
        if !window.place.naming() {
            window.view.set_document_name(&window.place.name());
        }
        if landing && made_here && window.keeps {
            window.view.set_landing(Some(self.landing()));
        }
        window.retrust(self.trust.trust(), &self.library);
        window.retitle();
        window.reframe(&self.buttons);
        window.follow(&self.settings, self.writer());
        if let Some(on) = self.reports {
            window.view.set_usage_reports(on);
            window.view.set_usage_notice(self.notice);
        }
        if self.servers_told {
            window.view.set_servers(self.servers());
        }
        self.mark("laid out");
        window.window.set_visible(true);
        // macOS places a window's lights for good as it shows it, which is before the first frame.
        window.reframe(&self.buttons);
        self.mark("window shown");
        window.window.request_redraw();
        let id = window.window.id();
        window.view.set_families(self.families.clone());
        self.windows.insert(id, window);
        Ok(id)
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
        // A server's document is listed by the address of its page, which `Request::Open` asks its server for.
        let served = |lately: servers::Lately| {
            let (opened, at) = (platform::local(&lately.opened), lately.opened);
            (at, letters_ui::Recent { name: lately.name, folder: lately.folder, opened, path: lately.address })
        };
        let here = self.library.recents().into_iter().filter_map(|kept| Some((kept.opened.clone(), recent(kept)?)));
        let mut recents: Vec<_> = here.chain(servers::lately(&self.home).into_iter().map(served)).collect();
        // Both lists are newest first, and a sort that is stable keeps each in its order.
        recents.sort_by(|a, b| b.0.cmp(&a.0));
        Landing {
            now: platform::local(&platform::now_rfc3339()),
            recents: recents.into_iter().map(|(_, recent)| recent).take(LISTED).collect(),
        }
    }

    /// Does what a window asked of the app since it was last looked at.
    fn serve(&mut self, event_loop: &ActiveEventLoop, id: WindowId, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let mut refused = false;
        let requests = window.view.take_requests();
        let served = !requests.is_empty();
        let page = Page::of(window.view.mode());
        if self.reports.is_some() && self.page != Some(page) {
            self.page = Some(page);
            self.telemetry.page(page);
        }
        for request in requests {
            telemetry::Event::asked(&request).into_iter().for_each(|event| self.telemetry.record(event));
            match request {
                // The address of a document's page is no file: its server is asked.
                Request::Open(path) => match connected::address(&path) {
                    Some((origin, document)) => self.reach(Some(id), &origin, document),
                    None => self.act(event_loop, id, Action::Open(Some(path.into()))),
                },
                Request::OpenDialog if !self.servers_told => {
                    let landing = self.windows.get(&id).is_some_and(|window| window.view.landing());
                    self.open_asked = Some((id, landing));
                }
                Request::OpenDialog => self.act(event_loop, id, Action::Open(None)),
                Request::New => self.act(event_loop, id, Action::New),
                // The name is taken on the way to a save, which reads it from the document. A server's
                // document takes it at its server, if it has none of its own there.
                Request::Named(name) => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.save_at = Some(now);
                        if let Some(remote) = window.remote.as_mut().filter(|remote| !remote.opened.named) {
                            remote.asks.push(Placing::Name(name));
                        }
                    }
                }
                // What was asked along with something refused is left for the person to ask again.
                Request::Rename(_) | Request::Move(_) | Request::MoveElsewhere | Request::NewFolder { .. }
                    if refused => {}
                Request::Rename(_) | Request::Move(_) | Request::MoveElsewhere | Request::NewFolder { .. }
                    if self.windows.get(&id).is_some_and(|window| window.remote.is_some()) =>
                {
                    let Some(window) = self.windows.get_mut(&id) else { continue };
                    let Some(remote) = &mut window.remote else { continue };
                    // Its server renames and moves it, and the bar is told once the server has answered.
                    let ask = match request {
                        Request::Rename(name) => Some(Placing::Rename(name)),
                        Request::Move(key) => remote.shelf(&key).map(|(space, folder)| Placing::Move { space, folder }),
                        Request::NewFolder { parent, name } => match remote.shelf(&parent) {
                            Some((space, parent)) => Some(Placing::NewFolder { space, parent, name }),
                            // The card that saves a copy asked, for a folder of this device. The person's
                            // own folder is listed before anything was written to it.
                            None => {
                                let _ = fs::create_dir_all(&parent);
                                match self.library.new_folder(Path::new(&parent), &name) {
                                    Ok(_) => window.recopy(&self.library),
                                    Err(refused) => window.view.refuse_copy_folder(&refused.to_string()),
                                }
                                continue;
                            }
                        },
                        _ => None,
                    };
                    match ask {
                        Some(ask) => remote.asks.push(ask),
                        // A folder the server no longer lists: the bar is told what there is.
                        None => {
                            window.located = None;
                            window.relocate(&self.library);
                        }
                    }
                }
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
                Request::SaveCopy { name, format, folder, replace } => {
                    self.copy_or_say(id, None, &name, format, folder.as_deref(), replace);
                }
                Request::SaveVersion { revision, name, format, folder, replace } => {
                    self.copy_or_say(id, Some(revision), &name, format, folder.as_deref(), replace);
                }
                Request::Close => self.act(event_loop, id, Action::Close),
                Request::Quit => self.act(event_loop, id, Action::Quit),
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
                Request::Settings(settings) => self.keep_settings(id, settings),
                Request::UsageReports(on) => self.keep_reports(on),
                Request::ImportStyle => {
                    let Some(window) = self.windows.get_mut(&id) else { continue };
                    let dialog = FileDialog::new().add_filter(STYLESHEETS, &[STYLESHEET]);
                    let Some(path) = dialog.set_parent(&*window.window).pick_file() else { continue };
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    // The window says so itself when the text is no stylesheet.
                    match fs::read(&path) {
                        Ok(text) => window.view.offer_stylesheet(&name, &String::from_utf8_lossy(&text)),
                        Err(error) => window.alert(&format!("{name} could not be read{}.", files::unread(&error))),
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
                        eprintln!("letters: could not write {}: {error}", path.display());
                        window.alert(&files::unwritten("The stylesheet", &error));
                    }
                }
                Request::Asset(name) => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        window.want(name, &mut self.decoder, &self.proxy);
                    }
                }
                Request::Pictures => {
                    let Some(window) = self.windows.get_mut(&id) else { continue };
                    let dialog = FileDialog::new().add_filter("Pictures", &pictures::ENDINGS);
                    let chosen = dialog.set_parent(&*window.window).pick_files().unwrap_or_default();
                    // What is no picture after all is left out, and the rest go in.
                    let read = chosen.iter().filter_map(|file| pictures::read(fs::read(file).ok()?));
                    window.insert(read.collect(), &mut self.platform);
                }
                Request::LinkedPicture(src) => self.link(id, src, Link::Open),
                Request::LoadPicture(src) => self.link(id, src, Link::Once),
                Request::FetchPreview { url } => self.preview(id, url),
                Request::EmbedPicture { figure, src } => self.link(id, src, Link::Embed(figure)),
                // The window asks for the pictures of that host again by itself.
                Request::TrustPictures(src) => self.trust_host(id, &src),
                Request::TrustDocument => self.vouch_for(id, false),
                Request::TrustFolder => self.vouch_for(id, true),
                Request::TrustFolderDialog => {
                    let Some(window) = self.windows.get(&id) else { continue };
                    let dialog = FileDialog::new().set_directory(window.place.folder(&self.library));
                    let chosen = dialog.set_parent(&*window.window).pick_folder();
                    // Folders are matched as the system spells them.
                    let folder = chosen.and_then(|folder| fs::canonicalize(folder).ok());
                    if let Some(folder) = folder.as_deref().and_then(Path::to_str) {
                        let kept = self.trust.trust_folder(folder);
                        self.retrust(id, kept);
                    }
                }
                Request::ForgetFolder(folder) => {
                    let kept = self.trust.forget_folder(&folder);
                    self.retrust(id, kept);
                }
                Request::TrustSource(typed) => match trust::source(&typed) {
                    Ok(source) => {
                        let kept = self.trust.trust_source(&source);
                        self.retrust(id, kept);
                    }
                    Err(why) => {
                        if let Some(window) = self.windows.get_mut(&id) {
                            window.view.refuse_source(&why);
                        }
                    }
                },
                Request::ForgetSource(source) => {
                    // One that cannot be read is none the file holds: the list is said again all the same.
                    let kept = letters_trust::Source::parse(&source)
                        .map_or(Ok(()), |source| self.trust.forget_source(&source));
                    self.retrust(id, kept);
                }
                Request::ForgetDocument => {
                    let Some(window) = self.windows.get_mut(&id) else { continue };
                    let Some(document) = window.id.clone() else { continue };
                    window.forgotten = true;
                    let kept = self.trust.forget_document(&document);
                    self.retrust(id, kept);
                }
                Request::ForgetWord(word) => {
                    if let Err(error) = self.words.forget(&word) {
                        eprintln!("letters: could not take {word:?} out of the dictionary: {error}");
                        if let Some(window) = self.windows.get(&id) {
                            window.alert(&format!(
                                "The word could not be taken out of your dictionary{}.",
                                files::because(&error)
                            ));
                        }
                    }
                    // Every window reads the list again, the one that asked too: it then shows what is kept.
                    self.windows.values_mut().for_each(|window| window.reword(&self.words));
                }
                Request::OpenLink { href, confirmed } => self.open_link(id, &href, confirmed),
                Request::Import { folders, comments } => self.bring_in(id, &folders, comments),
                Request::CancelImport => {
                    if let Some(window) = self.windows.get_mut(&id) {
                        (window.plan, window.importing, window.import_to) = (None, None, None);
                    }
                }
                Request::Print => self.print(id),
                Request::MergeConflicts { confirmed } => self.merge_conflicts(id, confirmed, now),
                Request::MergeFrom { path, confirmed } => self.merge_from(id, path.map(PathBuf::from), confirmed, now),
                Request::MergeIntoOriginal { confirmed } => self.merge_into_original(id, confirmed, now),
                Request::Connect => self.reconnect(id, now),
                // The Share dialog asks of a document's server. With no server it asks nothing.
                Request::Shares
                | Request::SharePeople(_)
                | Request::MakeLink(_)
                | Request::RevokeLink(_)
                | Request::AccessLog
                | Request::AccessLogBefore(_) => {
                    let window = self.windows.get_mut(&id);
                    let Some(remote) = window.and_then(|window| window.remote.as_mut()) else { continue };
                    remote.shares.push(match request {
                        Request::SharePeople(people) => Sharing::People(people),
                        Request::MakeLink(new) => Sharing::Link(new),
                        Request::RevokeLink(link) => Sharing::Revoke(link),
                        Request::AccessLog => Sharing::Log,
                        Request::AccessLogBefore(time) => Sharing::Earlier(time),
                        _ => Sharing::Look,
                    });
                }
                Request::SignIn { server } => self.sign_in(id, &server, now),
                Request::CancelSignIn => {
                    if let Some(signing) = self.signing.remove(&id) {
                        signing.given_up.store(true, Ordering::Relaxed);
                    }
                }
                Request::SignOut { server } => self.sign_out(&server, now),
                Request::Browse { server, place } => self.browse(id, server, place, None, now),
                Request::MergeConflictsOf { server, id: document, place } => {
                    self.browse(id, server, place, Some(document), now);
                }
                Request::NoticePressed(key) if key == update::KEY => self.take_update(event_loop, id),
                Request::NoticePressed(key) if key.starts_with(FETCH_KEY) => {
                    // The fonts are looked for again, and what still cannot be fetched is said again.
                    let Some(window) = self.windows.get_mut(&id) else { continue };
                    window.fonts_said.remove(&key[FETCH_KEY.len()..].to_lowercase());
                    window.fonts_seen = None;
                    window.window.request_redraw();
                }
                Request::NoticePressed(key) if key.starts_with(FONT_KEY) => {}
                Request::NoticePressed(key) => self.show_kept(id, &key),
                Request::BrowseTrash { server } => self.browse_trash(id, server, None, now),
                Request::PutBack { server, space, file } => self.browse_trash(id, server, Some((space, file)), now),
                Request::ImportAt { server, space, path } => self.import_at(event_loop, id, server, space, path),
                Request::OpenFrom { server, id: document } => self.reach(Some(id), &server, document),
                Request::NewAt { server, space, path } => self.start_at(id, &server, space, path, false),
                Request::ImportFile { server, space, path } => self.start_at(id, &server, space, path, true),
                // A request this shell does not know yet.
                _ => {}
            }
        }
        // An answer may have changed what the window shows.
        if served && let Some(window) = self.windows.get_mut(&id) {
            window.access_stale = true;
            window.sync(now);
        }
        // What was asked of the document's server goes there now.
        self.place(id);
        self.share(id);
    }

    /// The name the person's edits carry: the one they set, or else their account's.
    fn writer(&self) -> Option<&str> {
        self.settings.name.as_deref().or(self.account.as_deref())
    }

    /// Keeps what the person set in one window, and has every window follow it. Settings that could not
    /// be written still hold until the app closes, and the window that asked says so.
    fn keep_settings(&mut self, id: WindowId, settings: Settings) {
        let kept = settings::keep(&std::env::home_dir().unwrap_or_default(), &settings);
        self.settings = settings;
        let writer = self.settings.name.as_deref().or(self.account.as_deref());
        self.windows.values_mut().for_each(|window| window.follow(&self.settings, writer));
        if let (Err(error), Some(window)) = (kept, self.windows.get(&id)) {
            window.alert(&format!(
                "Your settings could not be kept in ~/.letters{}. They will last only until Letters closes.",
                files::because(&error)
            ));
        }
    }

    /// Follows the person's word on usage reports in every window, and keeps it for the next launch. A
    /// word that could not be kept still holds until the app closes.
    fn keep_reports(&mut self, on: bool) {
        if let Err(error) = telemetry::keep(&self.home, "enabled", on) {
            eprintln!("letters: the choice on usage reports could not be kept, since {error}");
        }
        self.reports = Some(on);
        self.telemetry.allow(on);
        self.windows.values_mut().for_each(|window| window.view.set_usage_reports(on));
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
            window.alert(&format!("Your default style could not be kept in ~/.letters{}.", files::because(&error)));
        }
    }

    /// Writes the copy a window asked for, of its document or of an older `version` of it, and tells the
    /// window when it cannot be written.
    fn copy_or_say(
        &mut self,
        id: WindowId,
        version: Option<Revision>,
        name: &str,
        format: CopyFormat,
        folder: Option<&str>,
        replace: bool,
    ) {
        if let Err(refused) = self.copy(id, version, name, format, folder.map(Path::new), replace)
            && let Some(window) = self.windows.get_mut(&id)
        {
            window.view.refuse_copy(&refused.to_string(), refused.replaceable());
        }
    }

    /// Starts writing a copy of a window's document called `name`, in `folder` or where the person then
    /// chooses. The copy is made from the document's file on another thread, and arrives as an event.
    fn copy(
        &mut self,
        id: WindowId,
        version: Option<Revision>,
        name: &str,
        format: CopyFormat,
        folder: Option<&Path>,
        replace: bool,
    ) -> Result<(), copies::Refused> {
        let Some(window) = self.windows.get_mut(&id) else { return Ok(()) };
        // An older version is read here, where the history is, and written with the file's pictures.
        let version = match version {
            Some(revision) => Some(window.view.document_at(&revision).ok_or(copies::Refused::NoVersion)?),
            None => None,
        };
        // The file is brought up to date first. A title still being typed names it only if nothing
        // else would give the document a file.
        for caret in [true, false] {
            if window.file().is_none() || window.unsaved() {
                window.save_now(caret, &self.library, &mut self.journals).map_err(copies::Refused::Unsaved)?;
            }
        }
        let source = window.file().ok_or(copies::Refused::Nothing)?.to_owned();
        let (output, replace) = match folder {
            Some(folder) => {
                // The person's own folder is offered for a server's document before anything is in it.
                if window.remote.is_some() {
                    let _ = fs::create_dir_all(folder);
                }
                (copies::path(folder, name, format)?, replace)
            }
            None => {
                // A server's document is copied to the person's own folders, not beside its copy here.
                let beside = match window.remote {
                    Some(_) => self.library.default_folder(),
                    None => window.place.folder(&self.library),
                };
                let suggested = copies::path(&beside, name, format)?;
                let dialog = FileDialog::new()
                    .add_filter(copies::kind(format), &[copies::extension(format)])
                    .set_directory(suggested.parent().unwrap_or(Path::new("")))
                    .set_file_name(suggested.file_name().unwrap_or_default().to_string_lossy());
                let Some(chosen) = dialog.set_parent(&*window.window).save_file() else { return Ok(()) };
                // The system has asked about a file that is in the way.
                (copies::ending(chosen, format), true)
            }
        };
        // Only a PDF shows a linked picture, and only one the window has loaded.
        let linked = if format == CopyFormat::Pdf { window.linked.clone() } else { copies::Linked::new() };
        let proxy = self.proxy.clone();
        self.copies.retain(|copy| !copy.is_finished());
        self.copies.push(thread::spawn(move || {
            let written = match version {
                Some(version) => copies::write_version(&source, version, &output, format, replace, &linked),
                None => copies::write(&source, &output, format, replace, &linked),
            };
            let _ = proxy.send_event(Event::Copied(id, output, written));
        }));
        Ok(())
    }

    /// Starts printing a window's document: its file is written as a PDF on another thread, as an export
    /// is, and the PDF goes to the system's print dialog when it arrives.
    fn print(&mut self, id: WindowId) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        // One dialog at a time: a second Print while the first is on its way is the same wish.
        if window.printing {
            return;
        }
        // The file is brought up to date first, as for a copy.
        let saved = [true, false].into_iter().try_for_each(|caret| match window.file().is_none() || window.unsaved() {
            true => window.save_now(caret, &self.library, &mut self.journals),
            false => Ok(()),
        });
        let source = match (saved, window.file()) {
            (Err(error), _) => return window.alert(&error),
            (Ok(()), None) => return window.alert("There is nothing to print yet."),
            (Ok(()), Some(file)) => file.to_owned(),
        };
        window.printing = true;
        // The paper shows the linked pictures the window has loaded, and no other.
        let (proxy, title, linked) = (self.proxy.clone(), window.name(), window.linked.clone());
        self.copies.retain(|copy| !copy.is_finished());
        self.copies.push(thread::spawn(move || {
            // Said as the person is to read it. What went wrong in the file is for the log.
            let file =
                fs::read(&source).map_err(|error| format!("Its file could not be read{}.", files::unread(&error)));
            let mut linked = |address: &str| linked.get(address).cloned();
            let pdf = file.and_then(|file| {
                convert::exported_with(file, &title, Format::Pdf, &mut linked).map_err(|error| {
                    eprintln!("letters: could not make a PDF of {}: {error}", source.display());
                    "Its file could not be read.".to_owned()
                })
            });
            let pdf = pdf.map(|pdf| pdf.bytes);
            let _ = proxy.send_event(Event::Pdf(id, pdf));
        }));
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
        window.retrust(self.trust.trust(), &self.library);
        done
    }

    /// Does what the notice of a newer release offers: closes Letters to come back as the new version with
    /// this window's document, or shows the release's page where Letters cannot replace itself.
    fn take_update(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        match &self.update {
            Some(found) if found.way == update::Way::Itself => {
                let reopen = self.windows.get(&id).and_then(|window| window.place.path());
                self.updating = Some(reopen.map(Path::to_path_buf));
                // Everything is saved on the way out, and the new version is started after that.
                event_loop.exit();
            }
            _ => {
                if let Err(error) = links::open(update::PAGE) {
                    eprintln!("letters: {} could not be opened: {error}", update::PAGE);
                }
            }
        }
    }

    /// Shows the journal a notice of the window told of, where it is kept. The file manager is asked on
    /// another thread, and the window says where the file is if nothing shows it.
    fn show_kept(&self, id: WindowId, key: &str) {
        let kept =
            self.windows.get(&id).and_then(|window| window.kept.iter().find(|kept| kept.to_string_lossy() == key));
        let Some(kept) = kept.cloned() else { return };
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            if let Err(error) = links::reveal(&kept) {
                eprintln!("letters: {} could not be shown: {error}", kept.display());
                let _ = proxy.send_event(Event::Unshown(id, kept));
            }
        });
    }

    /// The documents that sessions which ended early left unsaved. A run that only measures looks for
    /// none, and leaves every journal as it found it.
    ///
    /// With them, the journals that were set aside because the app ended as it last read them. Such a
    /// document comes back as its file has it.
    fn left_unsaved(&self) -> (Vec<Opened>, Vec<Aside>) {
        if self.typing.is_some() || self.report_launch {
            return Default::default();
        }
        let claims = self.journals.orphans();
        let aside = claims.iter().filter_map(|claim| claim.aside().cloned()).collect();
        (claims.into_iter().filter_map(files::reopen).collect(), aside)
    }

    fn mark(&mut self, stage: &'static str) {
        if let Some(launch) = &mut self.launch {
            launch.mark(stage);
        }
    }

    /// Opens a file in a new window, or reports why it cannot be opened. Returns whether it opened.
    /// Opens the document `id` of the server at `origin` for the window that asked. On another thread,
    /// the person signs in at the browser if nobody is signed in there, and the document's file is
    /// brought here if no copy of it is. What came of it arrives as an event.
    fn reach(&mut self, asker: Option<WindowId>, origin: &str, id: String) {
        let server = match Client::new(origin, &Roots::System) {
            Ok(server) => server,
            Err(refused) => {
                return drop(self.proxy.send_event(Event::Reached(asker, Err(Some(refused.to_string())))));
            }
        };
        // The window that asked says so if the person is sent to sign in, and can give it up.
        let given_up = Arc::new(AtomicBool::new(false));
        if let Some(asker) = asker {
            self.wait(asker, &given_up, false);
        }
        let (held, proxy, home) = (self.sessions.get(server.origin()).cloned(), self.proxy.clone(), self.home.clone());
        thread::spawn(move || {
            let waiting = |name: &str, page: &str| {
                if let Some(asker) = asker {
                    let waits = Event::SigningIn(asker, given_up.clone(), name.to_owned(), page.to_owned());
                    let _ = proxy.send_event(waits);
                }
            };
            let keychain = Keychain::system().ok();
            let browse = |page: &str| links::open(page);
            let entered = connected::enter(&server, &id, held, keychain, &home, &browse, &waiting, &given_up);
            // Nothing is said of a sign-in the person gave up.
            let entered = entered.map(Box::new).map_err(|error| error.sign_in().map(|_| error.to_string()));
            let _ = proxy.send_event(Event::Reached(asker, entered));
        });
    }

    /// Notes that a window waits for a sign-in from here on, which ends when `given_up` is set. One it
    /// waited for before is given up.
    fn wait(&mut self, id: WindowId, given_up: &Arc<AtomicBool>, shown: bool) {
        let waiting = Signing { given_up: given_up.clone(), shown };
        if let Some(before) = self.signing.insert(id, waiting) {
            before.given_up.store(true, Ordering::Relaxed);
        }
    }

    /// Ends a window's wait for the sign-in that `given_up` ends, if that is the one it waits for, and
    /// says whether the window showed it.
    fn waited(&mut self, id: WindowId, given_up: &Arc<AtomicBool>) -> bool {
        let waits = self.signing.get(&id).is_some_and(|signing| Arc::ptr_eq(&signing.given_up, given_up));
        waits && self.signing.remove(&id).is_some_and(|signing| signing.shown)
    }

    /// Signs the person in to the server at `address`, as they typed it, for the window whose Servers
    /// dialog asked. The browser is waited for on another thread, and what came of it arrives as an event.
    fn sign_in(&mut self, id: WindowId, address: &str, now: Instant) {
        let server = match Client::new(address, &Roots::System) {
            Ok(server) => server,
            Err(refused) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                window.view.set_sign_in(Some(SignIn::Failed(format!("Signing in did not work: {refused}."))));
                return window.sync(now);
            }
        };
        let given_up = Arc::new(AtomicBool::new(false));
        self.wait(id, &given_up, true);
        let (proxy, home) = (self.proxy.clone(), self.home.clone());
        thread::spawn(move || {
            let waiting = |name: &str, page: &str| {
                let waits = Event::SigningIn(id, given_up.clone(), name.to_owned(), page.to_owned());
                let _ = proxy.send_event(waits);
            };
            let keychain = Keychain::system().ok();
            let browse = |page: &str| links::open(page);
            let signed = connected::sign_in(&server, keychain, &home, &browse, &waiting, &given_up);
            let signed = signed.map_err(|error| error.sign_in());
            let _ = proxy.send_event(Event::SignedIn(id, given_up.clone(), signed));
        });
    }

    /// Takes in what came of the sign-in a window's Servers dialog asked for.
    fn signed_in(
        &mut self,
        id: WindowId,
        given_up: &Arc<AtomicBool>,
        signed: Result<Arc<Session>, Option<String>>,
        now: Instant,
    ) {
        let shown = self.waited(id, given_up);
        let failed = match signed {
            Ok(session) => {
                self.sessions.insert(session.server().origin().to_owned(), session);
                self.tell_servers(now);
                None
            }
            Err(failed) => failed,
        };
        // A sign-in the window no longer waits for has nothing to say there.
        if shown && let Some(window) = self.windows.get_mut(&id) {
            window.view.set_sign_in(failed.map(SignIn::Failed));
            window.access_stale = true;
            window.sync(now);
        }
    }

    /// Signs the person out of the server at `origin`. The server is told on another thread, and the
    /// sign-in is forgotten whether or not it hears.
    fn sign_out(&mut self, origin: &str, now: Instant) {
        if let Some(session) = self.sessions.remove(origin) {
            let home = self.home.clone();
            thread::spawn(move || {
                let _ = session.sign_out();
                let _ = servers::forget(&home, session.server().origin());
            });
        }
        self.tell_servers(now);
    }

    /// Asks the server at `server` what a place of it holds, for a window's Open card, on another thread.
    /// The answer arrives as an event. With `merge`, the conflict copies of the document of that id are
    /// merged into it first, and the place is listed without them.
    fn browse(&mut self, id: WindowId, server: String, place: letters_ui::Place, merge: Option<String>, now: Instant) {
        let Some(session) = self.sessions.get(&server).filter(|session| !session.ended()).cloned() else {
            // The card lists a server nobody is signed in to any more: it goes with the server's row.
            self.browsed(id, Err("Sign in to this server again to see its documents.".to_owned()), now);
            return self.tell_servers(now);
        };
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            if let Some(Err(refused)) = merge.map(|document| session.merge_conflicts(&document)) {
                let refused = Err(connected::sentence(&refused));
                return drop(proxy.send_event(Event::Browsed(id, refused)));
            }
            let held = match &place {
                letters_ui::Place::Spaces => session.spaces().map(|spaces| (spaces, Vec::new())),
                letters_ui::Place::Folder { space, path } => {
                    session.entries(space, path).map(|listing| (Vec::new(), listing.entries))
                }
                letters_ui::Place::Whole { space } => {
                    session.everything(space).map(|listing| (Vec::new(), listing.entries))
                }
                letters_ui::Place::Shared => session.shared().map(|entries| (Vec::new(), entries)),
                letters_ui::Place::Recent => session.recent().map(|entries| (Vec::new(), entries)),
            };
            let browsed = held
                .map(|(spaces, entries)| Box::new(Browsed { server, place, spaces, entries }))
                .map_err(|refused| connected::sentence(&refused));
            let _ = proxy.send_event(Event::Browsed(id, browsed));
        });
    }

    /// Asks the server at `server` what is in its trash, for a window's Open card, on another thread.
    /// The answer arrives as an event. With `back`, what a space's trash holds under that name is put
    /// back first.
    fn browse_trash(&mut self, id: WindowId, server: String, back: Option<(String, String)>, now: Instant) {
        let Some(session) = self.sessions.get(&server).filter(|session| !session.ended()).cloned() else {
            self.browsed(id, Err("Sign in to this server again to see its documents.".to_owned()), now);
            return self.tell_servers(now);
        };
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let put = back.map_or(Ok(()), |(space, file)| session.put_back(&space, &file).map(drop));
            let thrown = put.and_then(|()| session.trash()).map_err(|refused| connected::sentence(&refused));
            let _ = proxy.send_event(Event::Trash(id, server, thrown));
        });
    }

    /// Has the person choose a Markdown or Word file to make a document of in `folder` of a space of the
    /// server at `server`. The file is offered on its card as any file that is brought in, and read on
    /// this device, so that its pictures come along: see [`Self::bring_in`].
    fn import_at(&mut self, event_loop: &ActiveEventLoop, id: WindowId, server: String, space: String, folder: String) {
        let Some(window) = self.windows.get(&id) else { return };
        let dialog = FileDialog::new().add_filter("Markdown and Word", &import::ENDINGS);
        let chosen = dialog.set_directory(self.library.default_folder()).set_parent(&*window.window).pick_file();
        if let Some(path) = chosen {
            self.import(event_loop, Some(id), path, Some((server, space, folder)));
        }
    }

    /// Starts a document in the folder `at` of a space of the server at `server` and brings it here, on
    /// another thread, for the window that asked. It opens as a document of the server does: see
    /// [`Self::reached`]. With `import`, `at` is a Markdown or Word file the space keeps, and the document
    /// is made of it, beside it.
    fn start_at(&mut self, id: WindowId, server: &str, space: String, at: String, import: bool) {
        let Some(session) = self.sessions.get(server).filter(|session| !session.ended()).cloned() else {
            let nobody = Err(Some("Nobody is signed in to that server any more.".to_owned()));
            return drop(self.proxy.send_event(Event::Reached(Some(id), nobody)));
        };
        let (proxy, home) = (self.proxy.clone(), self.home.clone());
        thread::spawn(move || {
            let started = match import {
                true => session.import_file(&space, &at),
                false => session.new_document(&space, &at, None),
            };
            let started = started.map_err(|refused| connected::sentence(&refused));
            let reached = started.and_then(|entry| {
                let document = entry.id.ok_or("The server did not say which document it started.")?;
                connected::reach(&session, &document, &home).map_err(|error| error.to_string())
            });
            let entered = reached.map(|reached| Box::new((session, reached))).map_err(Some);
            let _ = proxy.send_event(Event::Reached(Some(id), entered));
        });
    }

    /// Tells a window's Open card what a place holds, or why it could not be read.
    fn browsed(&mut self, id: WindowId, browsed: Result<Box<Browsed>, String>, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        match browsed {
            Ok(browsed) => window.view.set_listing(*browsed),
            Err(why) => window.view.refuse_browse(&why),
        }
        window.access_stale = true;
        window.sync(now);
    }

    /// Where the person is signed in, as the Servers dialog lists it.
    fn servers(&self) -> Vec<letters_ui::Server> {
        let signed_in = self.sessions.values().filter(|session| !session.ended());
        let mut servers: Vec<_> = signed_in
            .map(|session| letters_ui::Server {
                address: session.server().origin().to_owned(),
                account: session.person().name,
            })
            .collect();
        servers.sort_by(|a, b| a.address.cmp(&b.address));
        servers
    }

    /// Says to every window where the person is signed in.
    fn tell_servers(&mut self, now: Instant) {
        if !self.servers_told {
            return;
        }
        let servers = self.servers();
        for window in self.windows.values_mut() {
            window.view.set_servers(servers.clone());
            window.access_stale = true;
            window.sync(now);
        }
    }

    /// Asks the server of a window's document what the window has to ask about the document's name and
    /// place, on another thread. How the document then stands arrives as an event.
    fn place(&mut self, id: WindowId) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let Some(copy) = window.place.path().map(Path::to_owned) else { return };
        let waits = |remote: &&mut Remote| !remote.placing && !remote.asks.is_empty();
        let Some(remote) = window.remote.as_mut().filter(waits) else { return };
        let asks = std::mem::take(&mut remote.asks);
        (remote.placing, remote.looked) = (true, Some(Instant::now()));
        // What its first heading asks for is nothing the bar waits to hear of.
        let answers = asks.iter().any(|ask| !matches!(ask, Placing::Look | Placing::Name(_)));
        let (session, document, proxy) = (remote.line.session().clone(), remote.opened.id.clone(), self.proxy.clone());
        thread::spawn(move || {
            let placed = Box::new(connected::place(&session, &document, &copy, &asks));
            let _ = proxy.send_event(Event::Placed(id, answers, placed));
        });
    }

    /// Asks the server of a window's document what the window's Share dialog asked, on another thread.
    /// The answers arrive as an event.
    fn share(&mut self, id: WindowId) {
        let Some(remote) = self.windows.get_mut(&id).and_then(|window| window.remote.as_mut()) else { return };
        if remote.sharing || remote.shares.is_empty() {
            return;
        }
        let asks = std::mem::take(&mut remote.shares);
        remote.sharing = true;
        let (session, document, proxy) = (remote.line.session().clone(), remote.opened.id.clone(), self.proxy.clone());
        thread::spawn(move || {
            let answers = asks.iter().map(|ask| connected::share(&session, &document, ask)).collect();
            let _ = proxy.send_event(Event::Shared(id, answers));
        });
    }

    /// Tells a window's Share dialog what the document's server answered it.
    fn shared(&mut self, id: WindowId, answers: Vec<connected::Shared>, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let Some(remote) = &mut window.remote else { return };
        remote.sharing = false;
        let owner = remote.opened.role.shares();
        for answer in answers {
            match answer {
                connected::Shared::Shares(shares, links) => {
                    window.view.set_shares(letters_ui::Shared { shares, owner, links });
                }
                connected::Shared::Made(link) => window.view.link_made(link),
                // The dialog dropped the link when it asked.
                connected::Shared::Revoked => {}
                connected::Shared::Log(log, days) => {
                    window.view.set_log_days(days);
                    window.view.set_access_log(log);
                }
                connected::Shared::Earlier(rows) => window.view.add_access_log(rows),
                connected::Shared::Refused(why) => window.view.refuse_share(&why),
            }
        }
        window.access_stale = true;
        window.sync(now);
    }

    /// Tries a document's server again, as the person asked of a window whose line the server ended. A
    /// sign-in that is over starts with a new one.
    fn reconnect(&mut self, id: WindowId, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let Some(remote) = &window.remote else { return };
        if !remote.signed_out() {
            return window.try_again(now);
        }
        let (origin, document) = (remote.line.session().server().origin().to_owned(), remote.opened.id.clone());
        self.reach(Some(id), &origin, document);
    }

    /// Opens the copy of a server's document in a window of its own and ties the window to the server,
    /// or says why there is nothing to open. The window that asked gives way if it holds nothing. A window
    /// that has the document open already is tied to the server again instead.
    fn reached(
        &mut self,
        event_loop: &ActiveEventLoop,
        asker: Option<WindowId>,
        entered: Result<Box<(Arc<Session>, Reached)>, Option<String>>,
        now: Instant,
    ) {
        // The sign-in the window that asked may have waited for is over, however it ended.
        if let Some(id) = asker
            && self.signing.remove(&id).is_some_and(|signing| signing.shown)
            && let Some(window) = self.windows.get_mut(&id)
        {
            window.view.set_sign_in(None);
            window.access_stale = true;
            window.sync(now);
        }
        let shown = entered.and_then(|entered| {
            let (session, reached) = *entered;
            let has = |window: &DocWindow| {
                window.remote.as_ref().is_some_and(|remote| {
                    remote.opened.id == reached.opened.id
                        && remote.line.session().server().origin() == session.server().origin()
                })
            };
            if let Some(window) = self.windows.values_mut().find(|window| has(window)) {
                self.sessions.insert(session.server().origin().to_owned(), session.clone());
                window.connect(session, reached, &self.proxy, &self.library, now);
                window.window.focus_window();
                window.sync(now);
                return Ok(());
            }
            let opened = files::open(&reached.path, new_style)
                .map_err(|error| Some(format!("Its copy on this device could not be opened{}.", error.unopened())))?;
            let id = self
                .open_window(event_loop, Some(opened), false)
                .map_err(|error| Some(format!("A window could not be opened for it{}.", no_window(&*error))))?;
            self.sessions.insert(session.server().origin().to_owned(), session.clone());
            if let Some(window) = self.windows.get_mut(&id) {
                window.connect(session, reached, &self.proxy, &self.library, now);
                window.retrust(self.trust.trust(), &self.library);
                window.sync(now);
            }
            Ok(())
        });
        match shown {
            Ok(()) => {
                if let Some(id) = asker.filter(|id| self.windows.get(id).is_some_and(DocWindow::pristine))
                    && let Some(mut window) = self.windows.remove(&id)
                {
                    self.parting.extend(window.leave());
                }
            }
            Err(Some(error)) => {
                let parent = asker.and_then(|id| self.windows.get(&id)).map(|window| window.window.clone());
                alert(parent.as_ref(), &format!("The document could not be opened from its server.\n\n{error}"));
                // A sign-in that ended in no document is kept all the same, and listed once it is read.
                read_kept(&self.proxy, &self.home);
            }
            Err(None) => {}
        }
        self.tell_servers(now);
    }

    fn open_path(&mut self, event_loop: &ActiveEventLoop, path: &Path, parent: Option<WindowId>) -> bool {
        let parent = parent.and_then(|id| self.windows.get(&id)).map(|window| window.window.clone());
        let opened = files::open(path, new_style).map_err(|error| error.unopened());
        let shown = opened
            .and_then(|opened| self.open_window(event_loop, Some(opened), false).map_err(|error| no_window(&*error)))
            .map(drop);
        if let Err(why) = &shown {
            alert(parent.as_ref(), &format!("{} could not be opened{why}.", path.display()));
        }
        shown.is_ok()
    }

    /// Starts bringing in a Markdown or Word file. It is read off this thread, and offered on a card once
    /// what it holds is known: in the window that asked if that has nothing of its own, else in a new one.
    /// `to` is where its document is to be made if that is at a server: the server's address, the space
    /// and the folder in it. With none it is kept in the default folder on this device.
    fn import(
        &mut self,
        event_loop: &ActiveEventLoop,
        asked: Option<WindowId>,
        path: PathBuf,
        to: Option<(String, String, String)>,
    ) {
        let empty =
            asked.filter(|id| self.windows.get(id).is_some_and(|window| window.pristine() && window.plan.is_none()));
        let id = match empty.map(Ok).unwrap_or_else(|| self.open_window(event_loop, None, true)) {
            Ok(id) => id,
            Err(error) => return alert(None, &format!("A new window could not be opened{}.", no_window(&*error))),
        };
        let session = to.as_ref().and_then(|(server, ..)| self.sessions.get(server)).cloned();
        if let Some(window) = self.windows.get_mut(&id) {
            window.import_to = to.clone();
        }
        let proxy = self.proxy.clone();
        thread::spawn(move || {
            let plan = Plan::of(&path, &new_style()).map(Box::new);
            // The card names the space as its server does, which only the server can say.
            let shown = to.map(|(server, space, folder)| {
                let spaces = session.and_then(|session| session.spaces().ok()).unwrap_or_default();
                let named = spaces.iter().find(|held| held.id == space).map_or(&space, |held| &held.name);
                let parts = [server.trim_start_matches("https://"), named].into_iter().chain(folder.split('/'));
                parts.filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" / ")
            });
            let _ = proxy.send_event(Event::Planned(id, path, plan, shown));
        });
    }

    /// Fetches a picture a window's document links to, off this thread, if the document may: one that
    /// may not keeps its frame, and its cloud says why. Linked pictures are fetched a few at a time.
    fn link(&mut self, id: WindowId, src: String, link: Link) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        if !window.made_here && window.id.is_none() {
            return window.links.push((src, link));
        }
        let (gate, limits) = (window.gate(self.trust.trust()), Limits::default());
        let guard = match link {
            Link::Open => gate.fetch_on_open(&src, limits),
            Link::Once | Link::Embed(_) => gate.fetch_on_request(&src, limits),
        };
        // A host the person trusts already is not offered for trusting again.
        window.view.set_picture_trusted(&src, gate.image(&src) == ImageDecision::Load);
        let state = match &guard {
            Ok(_) => Some(PictureState::Loading),
            // The window asks again by itself once the document is trusted.
            Err(FetchError::Held) => Some(PictureState::Held),
            // It waits to be asked for, as any picture from a host the person has not said they trust.
            Err(FetchError::UntrustedSource) => None,
            Err(error) => Some(PictureState::Failed(error.to_string())),
        };
        window.view.set_picture_state(&src, state);
        let Ok(guard) = guard else { return };
        let proxy = self.proxy.clone();
        self.fetching.run(move || {
            let fetched = remote::fetch(guard, &Roots::System);
            let fetched = fetched.map(|fetched| fetched.bytes).map_err(|failed| failed.to_string());
            let _ = proxy.send_event(Event::Linked(id, src, link, fetched));
        });
    }

    /// Reads the page at an address for a window's link previews, off this thread, if the document may:
    /// what the page says of itself, and the picture it shows itself by. Never as a document opens, since
    /// the window asks only when the person does. Each asking is answered once.
    fn preview(&mut self, id: WindowId, url: String) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let trust = self.trust.trust();
        let guard = match window.gate(trust).fetch_page(&url, Limits::page()) {
            Ok(guard) => guard,
            Err(error) => return window.view.fail_preview(&url, &error.to_string()),
        };
        // The picture's address is known only once the page is read, so the thread asks about it itself,
        // by what the person trusted when they asked for the preview.
        let (trust, made_here) = (trust.clone(), window.made_here);
        let (document, path) = (window.id.clone(), window.place.path().map(Path::to_owned));
        let proxy = self.proxy.clone();
        self.fetching.run(move || {
            let gate = gate_of(&trust, made_here, document.as_deref(), path.as_deref());
            let limits = Limits { max_bytes: PREVIEW_PICTURE, ..Limits::default() };
            let found = remote::preview(guard, &|address| gate.fetch_on_request(address, limits), &Roots::System);
            let found = found.map(|preview| {
                let (title, description) = (preview.title, preview.description);
                let picture = preview.picture.and_then(pictures::read);
                Box::new(LinkPreviewFound { url: url.clone(), title, description, picture })
            });
            let _ = proxy.send_event(Event::Previewed(id, url, found.map_err(|failed| failed.to_string())));
        });
    }

    /// Takes a linked picture's file from its fetch, to show or to keep in the document, or tells the
    /// window why there is none.
    fn linked(&mut self, id: WindowId, src: String, link: Link, fetched: Result<Vec<u8>, String>) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let failed = |reason: &str| Some(PictureState::Failed(reason.to_owned()));
        match (fetched, link) {
            (Err(reason), _) => window.view.set_picture_state(&src, failed(&reason)),
            (Ok(bytes), Link::Embed(figure)) => match pictures::read(bytes) {
                Some(picture) => {
                    window.view.set_picture_state(&src, None);
                    window.view.embed_picture(&figure, picture, &mut self.platform);
                    window.keep(self.platform.take_assets());
                }
                None => window.view.set_picture_state(&src, failed(NO_PICTURE)),
            },
            // It is decoded as any picture of the document is, and again for a sharper screen.
            (Ok(bytes), Link::Open | Link::Once) => {
                window.linked.insert(src.clone(), bytes);
                window.want(src, &mut self.decoder, &self.proxy);
            }
        }
    }

    /// Keeps a window's document, or the folder it is in, as one the person trusts. Every window is
    /// looked at again, since a folder vouches for all that is in it.
    fn vouch_for(&mut self, id: WindowId, folder: bool) {
        let Some(window) = self.windows.get(&id) else { return };
        let kept = match (folder, window.folder(), &window.id) {
            (true, Some(folder), id) => folder.to_str().map(|folder| {
                self.trust.trust_folder(folder)?;
                // A folder does not vouch for a document that was imported. Asked from that document's
                // own bar, the person means it too.
                match id.as_deref().filter(|id| self.trust.trust().imported(id)) {
                    Some(id) => self.trust.trust_document(id),
                    None => Ok(()),
                }
            }),
            (false, _, Some(id)) => Some(self.trust.trust_document(id)),
            _ => None,
        };
        match kept {
            Some(Err(error)) => {
                window.alert(&format!("Your choice to trust it could not be kept{}.", trust::unkept(&error)));
            }
            Some(Ok(())) => {
                // Trusted again, a document made in this window is vouched for as it was.
                if let Some(window) = self.windows.get_mut(&id) {
                    window.forgotten = false;
                }
                let (trust, library) = (self.trust.trust(), &self.library);
                self.windows.values_mut().for_each(|window| window.retrust(trust, library));
            }
            None => {}
        }
    }

    /// Says to every window what the person trusts now, after a change the window `id` asked for, or tells
    /// that window that the change could not be kept.
    fn retrust(&mut self, id: WindowId, kept: Result<(), TrustError>) {
        if let (Err(error), Some(window)) = (&kept, self.windows.get(&id)) {
            window.alert(&format!("Your choice could not be kept{}.", trust::unkept(error)));
        }
        // Said again either way: a window shows a row until it is told the list without it.
        let (trust, library) = (self.trust.trust(), &self.library);
        self.windows.values_mut().for_each(|window| window.retrust(trust, library));
    }

    /// Opens a link that was clicked in a window, if the person's trust lets it: a link from a document
    /// in safe mode shows where it goes first, and one that would run something is never opened.
    fn open_link(&mut self, id: WindowId, href: &str, confirmed: bool) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        match links::step(&window.gate(self.trust.trust()), href, confirmed) {
            Step::Ask => window.view.ask_link(href),
            Step::Refuse(why) => window.view.refuse_link(href, why),
            Step::Open => {
                if links::open(href).is_err() {
                    window.view.refuse_link(href, "Nothing on this device could be found to open it.");
                }
            }
        }
    }

    /// Merges the conflict copies a window was told of with its next save. Every asking is answered: with
    /// a question, or with the copies still offered.
    ///
    /// The press on "Merge conflicts" is enough in a trusted document, as the design has it: the copies
    /// lie beside the document under a sync tool's name for it and say they are that document, and what
    /// they bring is held to the document's own trust. Safe mode holds merges back, so there the person
    /// is asked first.
    fn merge_conflicts(&mut self, id: WindowId, confirmed: bool, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let copies = window.copies.clone().unwrap_or_default();
        if !confirmed && !window.trusted(self.trust.trust()) {
            let files = copies.iter().map(|copy| merge_file(copy, &self.library)).collect();
            return window.view.ask_merge(MergeAsk { files, conflicts: true });
        }
        window.merging.extend(copies.into_iter().map(|copy| (copy, true)));
        window.save_at = Some(now);
        // What is on its way in is not offered again.
        window.tell_copies(Vec::new(), &self.library);
    }

    /// Merges a window's document, which is a conflict copy, into the file it is a copy of, and carries
    /// on in that file. The press is enough in a trusted document, and safe mode asks first, as for the
    /// copies of a document. Every asking is answered: with a question, a refusal, or no original left
    /// to offer.
    fn merge_into_original(&mut self, id: WindowId, confirmed: bool, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let Some(original) = window.original.clone() else { return window.view.set_conflict_original(None) };
        if !confirmed && !window.trusted(self.trust.trust()) {
            return window.view.ask_merge_into(merge_file(&original, &self.library));
        }
        if let Err(error) = window.leave_for(original, &self.library, &mut self.journals) {
            return window.view.refuse_merge(&error);
        }
        window.save_at = Some(now);
    }

    /// Merges a file into a window's document with its next save: the one at `path`, or one the person
    /// picks. A file no trusted folder vouches for is asked about first, unless that was `confirmed`.
    fn merge_from(&mut self, id: WindowId, path: Option<PathBuf>, confirmed: bool, now: Instant) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let path = path.or_else(|| {
            let dialog = FileDialog::new().add_filter("Letters documents", &[EXTENSION]);
            dialog.set_directory(window.place.folder(&self.library)).set_parent(&*window.window).pick_file()
        });
        let Some(path) = path else { return };
        if !window.place.home().is_some_and(Home::written) {
            return window.view.refuse_merge("This document has no file yet, so nothing can be merged into it.");
        }
        if !confirmed && !window.may_merge(self.trust.trust(), &path) {
            let files = vec![merge_file(&path, &self.library)];
            return window.view.ask_merge(MergeAsk { files, conflicts: false });
        }
        window.merging.push((path, false));
        window.save_at = Some(now);
    }

    /// Keeps the host of a linked picture as one whose pictures load from now on.
    fn trust_host(&mut self, id: WindowId, src: &str) {
        // Only a picture with an address pictures load from offers this, so an address that is none has no
        // plain reason to give.
        let kept = HttpsUrl::parse(src).map_err(|_| "").and_then(|url| {
            self.trust.trust_source(&letters_trust::Source::host_of(&url)).map_err(|error| trust::unkept(&error))
        });
        match (kept, self.windows.get(&id)) {
            (Err(why), Some(window)) => {
                window.alert(&format!("The host could not be kept as one you trust{why}."));
            }
            (Err(_), None) => {}
            // The Settings dialog of every window lists it from now on.
            (Ok(()), _) => {
                let (trust, library) = (self.trust.trust(), &self.library);
                self.windows.values_mut().for_each(|window| window.retrust(trust, library));
            }
        }
    }

    /// Shows the card that asks what to bring in with a file, now that the file has been read.
    fn offer(&mut self, id: WindowId, path: &Path, plan: Result<Box<Plan>, Unplanned>, to: Option<String>) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let plan = match plan {
            Ok(plan) => plan,
            Err(unplanned) => return window.alert(&format!("{name} could not be brought in{}.", unplanned.why())),
        };
        let home = std::env::home_dir();
        let folders = plan.folders.iter().map(|folder| ImportFolder {
            path: folder.path.to_string_lossy().into_owned(),
            shown: import::shown(&folder.path, plan.folder(), home.as_deref()),
            pictures: folder.pictures as u32,
            chosen: folder.beside,
        });
        window.importing = Some(name.clone());
        window.view.offer_import(Some(ImportOffer {
            name,
            kind: match plan.kind {
                import::Kind::Markdown => ImportKind::Markdown,
                import::Kind::Word => ImportKind::Word,
            },
            folders: folders.collect(),
            missing: plan.missing as u32,
            comments: plan.comments as u32,
            destination: to.unwrap_or_else(|| self.library.shown(&self.library.default_folder())),
        }));
        window.plan = Some(plan);
        window.sync(Instant::now());
    }

    /// Brings in the file a window offered, with what the person chose on its card, off this thread. The
    /// document is kept as a new one in the default folder, and the file it came from is left as it is.
    /// A file that was offered for a folder of a server is made a document of there instead.
    fn bring_in(&mut self, id: WindowId, folders: &[String], comments: bool) {
        let Some(window) = self.windows.get_mut(&id) else { return };
        let Some(plan) = window.plan.take() else { return window.view.offer_import(None) };
        // The card names the folders as it was given them.
        let chosen =
            plan.folders.iter().filter(|folder| folders.iter().any(|path| *path == folder.path.to_string_lossy()));
        let choices = Choices { pictures: chosen.map(|folder| folder.path.clone()).collect(), comments };
        let source = window.importing.clone().unwrap_or_default();
        if let Some((server, space, folder)) = window.import_to.clone() {
            let Some(session) = self.sessions.get(&server).filter(|session| !session.ended()).cloned() else {
                window.plan = Some(plan);
                window.view.refuse_import("Sign in to the server again to bring the file in there.");
                return window.sync(Instant::now());
            };
            // Nothing is recorded of where the file came from: the server vouches for its documents.
            let (proxy, home) = (self.proxy.clone(), self.home.clone());
            thread::spawn(move || {
                let imported = (*plan).clone().import(&choices);
                let brought = connected::bring(&session, &space, &folder, &plan.name, &imported.package, &home);
                let brought = match brought {
                    Ok(reached) => Ok(Box::new((session, reached))),
                    Err(connected::Unreached::Refused(refused)) => Err((plan, connected::sentence(&refused))),
                    Err(other) => Err((plan, format!("The document could not be made at the server: {other}."))),
                };
                let _ = proxy.send_event(Event::MadeThere(id, source, brought));
            });
            return;
        }
        // Recorded before the document exists: unless the file lies in a trusted folder, the document opens
        // in safe mode from the first, wherever it is saved, until the person trusts it from its own bar.
        if let Err(error) = trust::import(&mut self.trust, &plan.id, plan.source()) {
            // The card stays and says why, with the file's plan for another try.
            window.plan = Some(plan);
            eprintln!("letters: the import could not be recorded: {error}");
            window.view.refuse_import("The file was not imported because Letters could not record where it came from.");
            return window.sync(Instant::now());
        }
        let (proxy, folder) = (self.proxy.clone(), self.library.default_folder());
        thread::spawn(move || {
            // The plan comes back with a document that could not be kept, for the card to try again.
            let kept = (*plan).clone().import(&choices).keep(&folder, &plan.name);
            let _ = proxy.send_event(Event::Imported(id, source, kept.map_err(|error| (plan, error))));
        });
    }

    /// Opens the document of a server that a file was brought in as, in place of the window that offered
    /// it. If the document could not be made there, the card says why and can ask again.
    fn made_there(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        source: String,
        brought: Result<Entered, (Box<Plan>, String)>,
        now: Instant,
    ) {
        let entered = match brought {
            Ok(entered) => entered,
            Err((plan, why)) => {
                match self.windows.get_mut(&id) {
                    Some(window) if window.view.importing() => {
                        window.plan = Some(plan);
                        window.view.refuse_import(&why);
                        window.sync(now);
                    }
                    // The card was put away meanwhile, or its window closed.
                    window => alert(
                        window.map(|window| &window.window),
                        &format!("{source} could not be brought in.\n\n{why}"),
                    ),
                }
                return;
            }
        };
        if let Some(window) = self.windows.get_mut(&id) {
            (window.plan, window.importing, window.import_to) = (None, None, None);
            window.view.offer_import(None);
            window.sync(now);
        }
        let (origin, document) = (entered.0.server().origin().to_owned(), entered.1.opened.id.clone());
        self.reached(event_loop, Some(id), Ok(entered), now);
        // Its history says where it came from, once the window that shows it has read the history.
        let shows = |window: &&mut DocWindow| {
            window
                .remote
                .as_ref()
                .is_some_and(|remote| remote.opened.id == document && remote.line.session().server().origin() == origin)
        };
        if let Some(window) = self.windows.values_mut().find(shows) {
            window.imported = Some(source);
        }
    }

    /// Opens the document a file was brought in as, in place of the window that offered it. If the document
    /// could not be kept, the card says why and can ask again.
    fn imported(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        source: String,
        kept: Result<PathBuf, (Box<Plan>, std::io::Error)>,
    ) {
        let path = match kept {
            Ok(path) => path,
            Err((plan, error)) => {
                eprintln!("letters: could not save what was imported from {source}: {error}");
                let why = files::unwritten("The document", &error);
                match self.windows.get_mut(&id) {
                    Some(window) if window.view.importing() => {
                        window.plan = Some(plan);
                        window.view.refuse_import(&why);
                        window.sync(Instant::now());
                    }
                    // The card was put away meanwhile, or its window closed.
                    window => alert(window.map(|window| &window.window), &why),
                }
                return;
            }
        };
        if let Some(window) = self.windows.get_mut(&id) {
            (window.plan, window.importing, window.import_to) = (None, None, None);
            window.view.offer_import(None);
            window.sync(Instant::now());
        }
        match self.windows.contains_key(&id) {
            true => self.act(event_loop, id, Action::Open(Some(path.clone()))),
            // The window was closed meanwhile, and the document is there all the same.
            false => _ = self.open_path(event_loop, &path, None),
        }
        // Its history says where it came from, once the window that shows it has read the history.
        if let Some(window) = self.windows.values_mut().find(|window| window.file() == Some(path.as_path())) {
            window.imported = Some(source);
        }
    }

    /// Takes the files that were let go over a window: pictures go into its document, and a single file
    /// of another kind is opened.
    fn take_dropped(&mut self, event_loop: &ActiveEventLoop, id: WindowId, files: Vec<PathBuf>) {
        let pictures = pictures::from_files(&files);
        match (self.windows.get_mut(&id), files.as_slice()) {
            (Some(window), _) if !pictures.is_empty() => {
                // Where they were let go is not known: the desktop does not say.
                window.insert(pictures, &mut self.platform);
                window.sync(Instant::now());
            }
            (Some(_), [file]) => self.act(event_loop, id, Action::Open(Some(file.clone()))),
            _ => {}
        }
    }

    fn act(&mut self, event_loop: &ActiveEventLoop, id: WindowId, action: Action) {
        match &action {
            Action::New => self.telemetry.record(telemetry::Event::DocumentOpened(telemetry::Source::New)),
            Action::Open(Some(_)) => self.telemetry.record(telemetry::Event::DocumentOpened(telemetry::Source::Listed)),
            Action::Open(None) => self.telemetry.record(telemetry::Event::DocumentOpened(telemetry::Source::Chosen)),
            _ => {}
        }
        match action {
            Action::New => {
                if let Err(error) = self.open_window(event_loop, None, false) {
                    alert(None, &format!("A new window could not be opened{}.", no_window(&*error)));
                }
            }
            Action::Choose => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                window.view.run_command("Open", &mut self.platform);
                window.access_stale = true;
                let now = Instant::now();
                window.sync(now);
                self.serve(event_loop, id, now);
            }
            Action::Open(path) => {
                let Some(window) = self.windows.get(&id) else { return };
                let pick = || {
                    let mut known = vec![EXTENSION];
                    known.extend(import::ENDINGS);
                    let dialog = FileDialog::new()
                        .add_filter("Documents", &known)
                        .add_filter("Letters document", &[EXTENSION])
                        .add_filter("Markdown and Word", &import::ENDINGS);
                    dialog.set_directory(self.library.default_folder()).set_parent(&*window.window).pick_file()
                };
                let Some(path) = path.or_else(pick) else { return };
                if import::offered(&path) {
                    return self.import(event_loop, Some(id), path, None);
                }
                // A window with no file and nothing to put in one gives way to the document that was asked for.
                let pristine = window.pristine();
                match self.open_path(event_loop, &path, Some(id)) {
                    true if pristine => {
                        if let Some(mut window) = self.windows.remove(&id) {
                            self.parting.extend(window.leave());
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

    /// Does the item of the menu bar at a place in [`menus::items`], for the window in front.
    fn menu(&mut self, event_loop: &ActiveEventLoop, place: usize) {
        let Some(item) = menus::items().get(place).copied() else { return };
        let front = self.windows.iter_mut().find(|(_, window)| window.window.has_focus());
        let Some((id, window)) = front.map(|(id, window)| (*id, window)) else { return };
        let mut action = None;
        match (menus::key(item, &window.view), item) {
            // As pressing the key does: the window is offered it, and the app what the window leaves.
            (Some(key), _) => {
                let used = window.view.key(key.clone(), &mut self.platform);
                action = (!used).then_some(&key).and_then(shortcut);
            }
            (None, menus::Item::Command(_, name)) => drop(window.view.run_command(name, &mut self.platform)),
            (None, _) => {}
        }
        window.access_stale = true;
        // A paste may have put pictures in.
        window.keep(self.platform.take_assets());
        let now = Instant::now();
        window.sync(now);
        self.serve(event_loop, id, now);
        if let Some(action) = action {
            self.act(event_loop, id, action);
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
        // A copy made by hand is trusted as its original was when the two parted: a document the person
        // trusts stays trusted, and one they brought in from elsewhere is still no folder's to vouch for.
        let forked: Vec<(String, String)> =
            self.windows.values_mut().filter_map(|window| window.forked.take()).collect();
        for (was, id) in &forked {
            let (trusted, imported) = (self.trust.trust().trusts_document(was), self.trust.trust().imported(was));
            let kept = imported.then(|| self.trust.import_document(id)).transpose();
            let kept = kept.and_then(|_| trusted.then(|| self.trust.trust_document(id)).transpose());
            if let Err(error) = kept {
                eprintln!("letters: could not carry over what is trusted of a copied document: {error}");
            }
        }
        if !forked.is_empty() {
            for window in self.windows.values_mut() {
                window.retrust(self.trust.trust(), &self.library);
            }
        }
    }

    fn close(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        // Nothing is asked at closing: the document is saved, and stays open only if that fails.
        let (library, journals) = (&self.library, &mut self.journals);
        let saved = self.windows.get_mut(&id).is_some_and(|window| window.save_or_say(false, library, journals));
        self.vouch();
        if saved && let Some(mut window) = self.windows.remove(&id) {
            // Nobody waits for the sign-in a closed window asked for.
            if let Some(signing) = self.signing.remove(&id) {
                signing.given_up.store(true, Ordering::Relaxed);
            }
            self.parting.retain(|telling| !telling.is_finished());
            self.parting.extend(window.leave());
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
        // The Dock has the bundle's icon for an app started from it. The bare program hands over its own.
        #[cfg(target_os = "macos")]
        if !std::env::current_exe().is_ok_and(|program| update::bundled(&program)) {
            letters_frame::set_dock_icon(include_bytes!("../../../assets/icons/png/app-512.png"));
        }
        let opened = match self.reading.take().map(|reading| reading.join().expect("reading does not panic")) {
            Some(Ok(opened)) => Ok(Some(opened)),
            // The person reads the plain reason, and the command line the error as it was.
            Some(Err(error)) => {
                let path = self.path.as_deref().unwrap_or(Path::new("")).display();
                let failure = format!("{path} could not be opened.\n\n{error}");
                Err((format!("{path} could not be opened{}.", error.unopened()), failure))
            }
            None => Ok(None),
        };
        // The document was read long before the desktop finished starting the app.
        self.mark("app launched");
        let shown = opened.and_then(|opened| {
            // What sessions that ended early left unsaved comes back first, and the document that was
            // asked for may be among it.
            let (left, aside) = self.left_unsaved();
            let mut documents: Vec<_> = left.into_iter().map(Some).collect();
            let asked = opened.filter(|opened| !documents.iter().flatten().any(|kept| same_file(kept, opened)));
            if asked.is_some() || documents.is_empty() {
                documents.push(asked);
            }
            let shown = documents.into_iter().try_for_each(|opened| {
                let landing = opened.is_none();
                self.open_window(event_loop, opened, landing).map(drop).map_err(|error| {
                    ("No window could be opened.".to_owned(), format!("No window could be opened.\n\n{error}"))
                })
            });
            // The window of its document says that a journal was set aside, and any window one whose
            // document has no file to show.
            for aside in aside {
                let shows =
                    |window: &DocWindow| aside.home.as_deref().is_some_and(|home| window.place.path() == Some(home));
                let its = self.windows.iter().find(|(_, window)| shows(window)).or(self.windows.iter().next());
                if let Some(window) = its.map(|(id, _)| *id).and_then(|id| self.windows.get_mut(&id)) {
                    window.aside.push(aside);
                }
            }
            shown
        });
        // A Markdown or Word file that was asked for is offered in a window that has nothing of its own.
        if let Some(path) = self.path.clone().filter(|path| shown.is_ok() && import::offered(path)) {
            let empty = self.windows.iter().find(|(_, window)| window.pristine()).map(|(id, _)| *id);
            self.import(event_loop, empty, path, None);
        }
        // A document that was asked for by the address of its page is asked of its server.
        if let Some((origin, id)) = self.from_server.take().filter(|_| shown.is_ok()) {
            let empty = self.windows.iter().find(|(_, window)| window.pristine()).map(|(id, _)| *id);
            self.reach(empty, &origin, id);
        }
        if let Err((said, failure)) = shown {
            if !self.report_launch {
                alert(None, &said);
            }
            self.fail(event_loop, failure);
        }
        // The menus write the keys of the window's commands, so they wait for a window to ask.
        if let Some(window) = self.windows.values().next() {
            let proxy = self.proxy.clone();
            self.bar = Bar::install(&window.view, move |place| drop(proxy.send_event(Event::Menu(place))));
            self.bar_stale = true;
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // What a window offers changes with what is done in it, not with where the pointer is or a frame.
        let passing = matches!(event, WindowEvent::CursorMoved { .. } | WindowEvent::RedrawRequested);
        self.bar_stale |= !passing;
        let Self { windows, platform, mods, library, fonts, chosen, trust, .. } = self;
        let Some(window) = windows.get_mut(&id) else { return };
        window.access.process_event(&window.window, &event);
        let mut action = None;
        match event {
            WindowEvent::RedrawRequested => {
                if let Err(error) = window.redraw(platform.now_ms()) {
                    return self.fail(event_loop, format!("the window could not be painted: {error}"));
                }
                // What the frame showed as an empty box, the system may have a face for. It is looked
                // for off this thread, and comes back as an event.
                let uncovered = window.view.take_uncovered();
                if !uncovered.is_empty() {
                    fonts.ask(id, uncovered);
                }
                // The fonts a writer chose are looked for the same way, once there is a thread for it.
                if chosen.started()
                    && let Some(ask) = window.fonts(trust.trust())
                {
                    chosen.ask(id, ask);
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
                    // Nor where the person is signed in, which the keychain is asked for.
                    read_kept(&self.proxy, &self.home);
                    // Nor what Letters reports of its use, which a run that measures never does.
                    if self.typing.is_none() && telemetry::here() {
                        let said = letters_trust::Store::read(&mut trust::Settings::of(&self.home))
                            .ok()
                            .flatten()
                            .unwrap_or_default();
                        let allowed = telemetry::allowed(&said);
                        self.telemetry = Telemetry::start(telemetry::SERVER, allowed);
                        self.reports = Some(allowed);
                        // The card says so in the first run that reports, and not again.
                        self.notice = allowed && !telemetry::noticed(&said);
                        if self.notice {
                            let _ = telemetry::keep(&self.home, "noticed", true);
                        }
                        window.view.set_usage_reports(allowed);
                        window.view.set_usage_notice(self.notice);
                    }
                    // Nor whether there is a newer Letters, which GitHub is asked.
                    if self.typing.is_none() && update::here() {
                        let said = letters_trust::Store::read(&mut trust::Settings::of(&self.home));
                        if update::allowed(&said.ok().flatten().unwrap_or_default()) {
                            let proxy = self.proxy.clone();
                            update::look(&self.home, move |found| {
                                let _ = proxy.send_event(Event::Update(Box::new(found)));
                            });
                        }
                    }
                    // Nor the fonts a writer can choose from, which are the system's and a library's.
                    let (home, offered, answered) = (self.home.clone(), self.proxy.clone(), self.proxy.clone());
                    self.chosen.start(
                        move || Chosen::system(Some(&home)),
                        move |offers| drop(offered.send_event(Event::Families(offers))),
                        move |id, found| drop(answered.send_event(Event::Chosen(id, Box::new(found)))),
                    );
                    // The desktop is asked over D-Bus, which is not worth holding a frame for.
                    let (proxy, folders) = (self.proxy.clone(), self.dictionaries.clone());
                    thread::spawn(move || {
                        // First, since it is wanted before the first edit.
                        let _ = proxy.send_event(Event::Account(writer::account()));
                        if let Some(reduced) = chrome::reduced_motion() {
                            let _ = proxy.send_event(Event::Motion(reduced));
                        }
                        let _ = proxy.send_event(Event::Offset(platform::utc_offset()));
                        if let Some(buttons) = Buttons::ask().filter(|_| Host::HERE == Host::Linux) {
                            let _ = proxy.send_event(Event::Buttons(buttons));
                        }
                        // Last, since only the settings offer them.
                        let languages = dictionaries::installed(&folders).into_iter().collect();
                        let _ = proxy.send_event(Event::Languages(languages));
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
                    window.start_read(&self.proxy, &self.journals, self.trust.trust());
                    window.ask_dictionaries(&self.proxy, &self.dictionaries);
                    window.relocate(library);
                }
                window.relist(library);
                // The page asks for the pictures it came to show while it was painted.
                return self.serve(event_loop, id, Instant::now());
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::ThemeChanged(_) => {
                // A window that was maximised or let go again changed its size on the way.
                window.reframe(&self.buttons);
                window.fit();
                // A sharper screen draws pictures from more pixels than were held for the last.
                window.ask_again(&mut self.decoder, &self.proxy)
            }
            WindowEvent::Focused(focused) => {
                window.view.focus(focused);
                // The lights may have been moved since they were last looked for.
                window.reframe(&self.buttons);
                // Another window, or another copy of the app, may have taught the dictionary a word.
                if focused && window.rest.is_none() {
                    window.reword(&self.words);
                    // A sync tool may have left a copy of the document while the window was left alone.
                    window.look_for_copies(&self.proxy, &self.library);
                    // And someone may have renamed or moved it at its server.
                    if let Some(remote) = &mut window.remote {
                        remote.look(Instant::now());
                    }
                }
                // The clock may have moved to summer time, or the person to another zone, while the
                // window was left alone.
                if focused && self.offset.is_some() {
                    let minutes = platform::utc_offset();
                    self.offset = Some(minutes);
                    window.view.set_utc_offset(minutes);
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
            // Several files come one after another, and are taken together once all of them are here.
            WindowEvent::DroppedFile(file) => window.dropped.push(file),
            _ => {}
        }
        // A paste or a drop may have put pictures in.
        window.keep(platform.take_assets());
        let now = Instant::now();
        window.sync(now);
        self.serve(event_loop, id, now);
        if let Some(action) = action {
            self.act(event_loop, id, action);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        let now = Instant::now();
        self.bar_stale = true;
        match event {
            Event::Menu(place) => self.menu(event_loop, place),
            Event::Beside(id, home, found) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.found_beside(&home, *found, &self.library);
                    window.sync(now);
                }
            }
            Event::Saved(id) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.finish_save(&self.library, now);
                    // A sync tool may have left a copy since the folder was last looked at.
                    window.look_for_copies(&self.proxy, &self.library);
                    window.ask_again(&mut self.decoder, &self.proxy);
                    // Another writer's edits may have come in with it, in a language of their own.
                    window.ask_dictionaries(&self.proxy, &self.dictionaries);
                    // Another writer's edits may have come in with it.
                    window.access_stale = true;
                    window.sync(now);
                }
            }
            Event::Copied(id, output, written) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                match written {
                    Ok(made) => {
                        window.view.set_copied(&self.library.shown(output.parent().unwrap_or(Path::new(""))));
                        // A copy of a document the person trusts is theirs as much as the document is.
                        if let Some(made) = made.filter(|_| window.maths_links)
                            && let Err(error) = self.trust.trust_document(&made)
                        {
                            eprintln!("letters: could not record a copy as trusted: {error}");
                        }
                    }
                    Err(refused) => window.view.refuse_copy(&refused.to_string(), refused.replaceable()),
                }
                window.access_stale = true;
                window.sync(now);
            }
            Event::Read(id) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                if let Err(error) = window.finish_read(&mut self.journals) {
                    match window.keeps {
                        true => window.alert(&error),
                        false => eprintln!("letters: {error}"),
                    }
                }
                window.ask_again(&mut self.decoder, &self.proxy);
                // The editor has the file's history, which the line to a server's document waited for.
                window.go_online(now);
                // The file has said which document it is, so what the person trusts can be asked about it.
                window.retrust(self.trust.trust(), &self.library);
                window.look_for_copies(&self.proxy, &self.library);
                let links = std::mem::take(&mut window.links);
                window.access_stale = true;
                window.sync(now);
                // And the pictures it links to can be fetched, if it may.
                links.into_iter().for_each(|(src, link)| self.link(id, src, link));
            }
            Event::Gpu(None) => self.gpu = GpuState::Off,
            Event::Reached(asker, entered) => self.reached(event_loop, asker, entered, now),
            Event::Kept(kept) => {
                // A session the app holds is the newer one.
                for session in kept {
                    self.sessions.entry(session.server().origin().to_owned()).or_insert(session);
                }
                self.servers_told = true;
                self.tell_servers(now);
                // An Open that waited for this is done now, unless the person went on to something else.
                if let Some((id, landing)) = self.open_asked.take()
                    && let Some(window) = self.windows.get_mut(&id).filter(|window| window.view.landing() == landing)
                {
                    window.view.run_command("Open", &mut self.platform);
                    window.access_stale = true;
                    window.sync(now);
                    self.serve(event_loop, id, now);
                }
            }
            Event::SigningIn(id, given_up, name, page) => {
                let waits = |signing: &&mut Signing| Arc::ptr_eq(&signing.given_up, &given_up);
                if let Some(signing) = self.signing.get_mut(&id).filter(waits)
                    && let Some(window) = self.windows.get_mut(&id)
                {
                    signing.shown = true;
                    window.view.set_sign_in(Some(SignIn::Waiting { name, page }));
                    window.access_stale = true;
                    window.sync(now);
                }
            }
            Event::SignedIn(id, given_up, signed) => self.signed_in(id, &given_up, signed, now),
            Event::Browsed(id, browsed) => {
                self.browsed(id, browsed, now);
                // A session that ended while it was asked is no longer listed, and the card goes with it.
                if self.sessions.values().any(|session| session.ended()) {
                    self.sessions.retain(|_, session| !session.ended());
                    self.tell_servers(now);
                }
                // The card asks for the next place once it has the spaces.
                self.serve(event_loop, id, now);
            }
            Event::Update(found) => {
                for window in self.windows.values_mut() {
                    window.view.notify(found.notice());
                    window.sync(now);
                }
                self.update = Some(*found);
            }
            Event::Unshown(id, kept) => {
                if let Some(window) = self.windows.get(&id) {
                    window.alert(&format!("The file could not be shown. It is kept at {}.", journal::shown(&kept)));
                }
            }
            Event::Trash(id, server, thrown) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    match thrown {
                        Ok(thrown) => window.view.set_trash(&server, thrown),
                        Err(why) => window.view.refuse_browse(&why),
                    }
                    window.access_stale = true;
                    window.sync(now);
                }
                if self.sessions.values().any(|session| session.ended()) {
                    self.sessions.retain(|_, session| !session.ended());
                    self.tell_servers(now);
                }
            }
            Event::MadeThere(id, source, brought) => self.made_there(event_loop, id, source, brought, now),
            Event::Heard(id, said) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.heard(said, now);
                    window.sync(now);
                }
                self.place(id);
            }
            Event::Placed(id, answers, placed) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    let lately = window.placed(answers, *placed, &self.library);
                    window.sync(now);
                    // A list that cannot be written costs a row on a card.
                    let _ = lately.map(|lately| servers::opened(&self.home, lately));
                }
                // What was asked meanwhile goes to the server now.
                self.place(id);
            }
            Event::Shared(id, answers) => {
                self.shared(id, answers, now);
                // A dialog that was refused may ask how things stand, and more may have been asked.
                self.serve(event_loop, id, now);
            }
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
            Event::Picture(id, asset, image, within) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                window.picture(asset, image, within);
                // What the copy of a server's document lacks is asked of the server.
                let Some(remote) = &mut window.remote else { return };
                for name in std::mem::take(&mut remote.lacking) {
                    let (session, document) = (remote.line.session().clone(), remote.opened.id.clone());
                    let proxy = self.proxy.clone();
                    self.fetching.run(move || {
                        let brought = connected::picture(&session, &document, &name);
                        let _ = proxy.send_event(Event::Brought(id, name, brought));
                    });
                }
            }
            Event::Brought(id, name, brought) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                if window.fonts_asked.contains(&name) {
                    // A font file: the copy is to hold it, and the fonts are looked for again with it.
                    if let Some(bytes) = brought {
                        window.assets.insert(name, bytes);
                        window.marks = true;
                        window.save_at.get_or_insert(now + SAVE_AFTER);
                    }
                    window.fonts_seen = None;
                    window.window.request_redraw();
                    return;
                }
                match brought {
                    Some(bytes) => {
                        // No edit, but the copy is to hold it: it goes in with the next save.
                        window.assets.insert(name.clone(), bytes);
                        window.marks = true;
                        window.save_at.get_or_insert(now + SAVE_AFTER);
                        window.want(name, &mut self.decoder, &self.proxy);
                    }
                    None => window.view.set_image(&name, None),
                }
                window.sync(now);
            }
            Event::Previewed(id, url, found) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    match found {
                        Ok(found) => {
                            window.view.fill_preview(*found, &mut self.platform);
                            window.keep(self.platform.take_assets());
                        }
                        Err(why) => window.view.fail_preview(&url, &why),
                    }
                    window.sync(now);
                }
            }
            Event::Linked(id, src, link, fetched) => {
                self.linked(id, src, link, fetched);
                if let Some(window) = self.windows.get_mut(&id) {
                    window.sync(now);
                }
            }
            Event::Faces(id, faces) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                if window.view.add_fallback_fonts(faces) {
                    window.sync(now);
                }
            }
            Event::Families(offers) => {
                self.families = offers;
                for window in self.windows.values_mut() {
                    window.view.set_families(self.families.clone());
                    // The frame before this could not ask for its document's fonts yet.
                    window.window.request_redraw();
                }
            }
            Event::Chosen(id, found) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                let mut found = *found;
                // What the copy of a server's document lacks is asked of the server, as its pictures are.
                // Until it answers, nobody is told that a family is missing.
                let absent = std::mem::take(&mut found.absent);
                let asked: Vec<String> = match &window.remote {
                    Some(_) => absent.into_iter().filter(|name| window.fonts_asked.insert(name.clone())).collect(),
                    None => Vec::new(),
                };
                if !asked.is_empty() {
                    found.lacking.clear();
                }
                for name in asked {
                    let Some(remote) = &window.remote else { break };
                    let (session, document) = (remote.line.session().clone(), remote.opened.id.clone());
                    let proxy = self.proxy.clone();
                    self.fetching.run(move || {
                        let brought = connected::picture(&session, &document, &name);
                        let _ = proxy.send_event(Event::Brought(id, name, brought));
                    });
                }
                window.chose(found, &mut self.platform);
                window.sync(now);
            }
            Event::Planned(id, path, plan, to) => self.offer(id, &path, plan, to),
            Event::Pdf(id, pdf) => {
                let Some(window) = self.windows.get(&id) else { return };
                let proxy = self.proxy.clone();
                let done = move |shown| drop(proxy.send_event(Event::Printed(id, shown)));
                match pdf {
                    Ok(pdf) => printing::print(pdf, window.name(), &window.window, done),
                    Err(error) => done(Err(error)),
                }
            }
            Event::Printed(id, shown) => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                window.printing = false;
                if let Err(error) = shown {
                    window.alert(&format!("{} could not be printed.\n\n{error}", window.name()));
                }
            }
            Event::Imported(id, source, kept) => self.imported(event_loop, id, source, kept),
            Event::Offset(minutes) => {
                self.offset = Some(minutes);
                for window in self.windows.values_mut() {
                    window.view.set_utc_offset(minutes);
                }
            }
            Event::Account(name) => {
                (self.account, self.named) = (name, true);
                let writer = self.settings.name.as_deref().or(self.account.as_deref());
                for window in self.windows.values_mut() {
                    window.view.set_account_name(self.account.as_deref());
                    if let Some(writer) = writer {
                        window.view.set_author(writer);
                    }
                    // Named or not, who writes here is as known as it will be.
                    window.named = true;
                    window.say_seen(false);
                    window.window.request_redraw();
                }
            }
            Event::Languages(languages) => {
                self.languages = languages;
                self.windows.values_mut().for_each(|window| window.tell_languages(&self.languages));
            }
            Event::Served(id, held) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.served = Some(held);
                    window.tell_languages(&self.languages);
                }
            }
            Event::Unasked(id, languages) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    languages.iter().for_each(|language| _ = window.asked.remove(language));
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
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        (window.access_stale, window.access_whole) = (true, true);
                    }
                    accesskit_winit::WindowEvent::ActionRequested(request) => {
                        window.view.access_action(request, &mut self.platform);
                        window.keep(self.platform.take_assets());
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
                Ok(()) => self.parting.extend(window.leave()),
                Err(error) => eprintln!("letters: {error}"),
            }
            // What could not be saved stays in its journal, which closes here.
            window.journal = None;
        }
        self.vouch();
        self.journals.finish();
        // Half a copy is worse than a moment's wait.
        for copy in self.copies.drain(..) {
            let _ = copy.join();
        }
        // A server hears what was last seen if it answers soon. If not, this device remembers.
        let until = Instant::now() + PARTING;
        while self.parting.iter().any(|telling| !telling.is_finished()) && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        // Last, and briefly: the person's work is safe by now, and a report is not worth their wait.
        std::mem::replace(&mut self.telemetry, Telemetry::off()).stop(Duration::from_millis(300));
        // The new version takes this one's place as it ends, and opens what the person was in.
        if let (Some(found), Some(reopen)) = (&self.update, self.updating.take())
            && let Err(error) = found.install(reopen.as_deref())
        {
            eprintln!("letters: the new version could not be started: {error}");
        }
    }

    /// Runs after each batch of events: starts due saves and sleeps until the next thing is due.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let now_ms = self.platform.now_ms();
        self.vouch();
        let dropped = |(id, window): (&WindowId, &mut DocWindow)| {
            Some((*id, std::mem::take(&mut window.dropped))).filter(|(_, files)| !files.is_empty())
        };
        for (id, files) in self.windows.iter_mut().filter_map(dropped).collect::<Vec<_>>() {
            self.take_dropped(event_loop, id, files);
        }
        let mut next: Option<Instant> = None;
        for window in self.windows.values_mut() {
            if window.wake_ms.is_some_and(|wake| wake <= now_ms) {
                window.wake_ms = window.view.animate(now_ms);
                window.sync(now);
            }
            if window.remote.as_ref().and_then(Remote::due).is_some_and(|due| due <= now) {
                window.tick(now);
                window.sync(now);
            }
            // Before anything of the edit is journalled or saved under the id of its original.
            window.become_own(&mut self.journals);
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
            let line = window.remote.as_ref().and_then(Remote::due);
            for due in [wake, save, window.journal_at, type_next, line].into_iter().flatten() {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        // A document that just became its own is trusted as its original was, before anything asks.
        if self.windows.values().any(|window| window.forked.is_some()) {
            self.vouch();
        }
        if let Some(bar) = self.bar.as_mut().filter(|_| std::mem::take(&mut self.bar_stale)) {
            let front = self.windows.values().find(|window| window.window.has_focus());
            bar.offer(|name| front.is_some_and(|window| window.view.offers(name)));
        }
        event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
