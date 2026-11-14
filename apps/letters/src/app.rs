//! The winit event loop: windows, their input, painting and saving.
//!
//! Each window holds one document. Everything a window shows comes from its [`View`] as a display list, which
//! is rasterised into a software surface. Saving happens on a thread per save, a moment after typing stops.

use std::collections::HashMap;
use std::error::Error;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use letters_editor::{Editor, InputEvent, Key, KeyEvent, Platform as _};
use letters_model::Style;
use letters_render::{DisplayList, Point, Size};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy, OwnedDisplayHandle};
use winit::keyboard::ModifiersState;
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
use winit::window::{CursorIcon, Window, WindowId};

use crate::cli::{Mode, Options};
use crate::files::{self, EXTENSION, Home, Opened};
use crate::input::{self, Composer, Press};
use crate::launch::{FIRST_FRAME, Launch};
use crate::platform::Desktop;
use crate::raster::Painter;
use crate::view::{self, Cursor, View};

const MAC: bool = cfg!(target_os = "macos");
const UNTITLED: &str = "Untitled";
/// How long typing must pause before a save starts.
const SAVE_AFTER: Duration = Duration::from_millis(800);
/// How long to wait before trying a failed save again.
const RETRY_AFTER: Duration = Duration::from_secs(5);
/// The distance one notch of a mouse wheel scrolls, in logical pixels.
const WHEEL_LINE: f32 = 40.0;
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(1100.0, 800.0);

/// What reaches the event loop from outside winit.
#[derive(Debug)]
pub enum Event {
    Access(accesskit_winit::Event),
    /// A window's save has finished, and its result is ready to collect.
    Saved(WindowId),
}

impl From<accesskit_winit::Event> for Event {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Access(event)
    }
}

/// Runs the app until its last window closes.
pub fn run(options: Options, mut launch: Launch) -> Result<(), Box<dyn Error>> {
    // The file is read while the event loop connects to the display.
    let reading = options.file.clone().map(|path| thread::spawn(move || files::open(&path)));
    let event_loop = EventLoop::<Event>::with_user_event().build()?;
    launch.mark("event loop ready");
    let mut app = App {
        starting: true,
        report_launch: options.mode == Mode::LaunchReport,
        launch: Some(launch),
        reading,
        path: options.file,
        proxy: event_loop.create_proxy(),
        platform: Desktop::new(),
        context: softbuffer::Context::new(event_loop.owned_display_handle())?,
        windows: HashMap::new(),
        mods: ModifiersState::empty(),
        failure: None,
    };
    event_loop.run_app(&mut app)?;
    app.failure.map_or(Ok(()), |failure| Err(failure.into()))
}

/// Opens the document and paints its first frame into memory, as [`run`] would into a window.
pub fn run_headless(options: &Options, mut launch: Launch) -> Result<Launch, Box<dyn Error>> {
    let (document, style) = match &options.file {
        Some(path) => {
            let opened = files::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
            (opened.document, opened.style)
        }
        None => (files::blank(), Style::default()),
    };
    launch.mark("document read");
    // A window of the default size on a display like the reference machine's.
    let scale = 2.0;
    let (width, height) = ((WINDOW_SIZE.width * scale) as u32, (WINDOW_SIZE.height * scale) as u32);
    let mut view = View::new(Editor::new(document), style);
    view.resize(Size { width: WINDOW_SIZE.width as f32, height: WINDOW_SIZE.height as f32 }, scale as f32);
    launch.mark("laid out");
    let mut list = DisplayList::default();
    view.paint(&mut list);
    launch.mark("display list");
    let mut pixels = vec![0; (width * height) as usize];
    Painter::new().paint(&list, scale as f32, view::PAGE, width, &mut pixels);
    launch.mark(FIRST_FRAME);
    Ok(launch)
}

struct App {
    /// Whether the first window is still to open.
    starting: bool,
    report_launch: bool,
    /// The stopwatch, until the first frame is on screen.
    launch: Option<Launch>,
    reading: Option<JoinHandle<Result<Opened, files::Error>>>,
    path: Option<PathBuf>,
    proxy: EventLoopProxy<Event>,
    platform: Desktop,
    context: softbuffer::Context<OwnedDisplayHandle>,
    windows: HashMap<WindowId, DocWindow>,
    mods: ModifiersState,
    /// Why the app is exiting, when it is not by choice.
    failure: Option<String>,
}

struct DocWindow {
    window: Rc<Window>,
    surface: softbuffer::Surface<OwnedDisplayHandle, Rc<Window>>,
    access: accesskit_winit::Adapter,
    view: View,
    painter: Painter,
    list: DisplayList,
    composer: Composer,
    pointer: Point,
    cursor: Cursor,
    /// Whether the accessibility tree is behind the view.
    access_stale: bool,
    /// When the view next wants a frame without being asked, on the editor's clock.
    wake_ms: Option<u64>,

    /// Where the document is saved, or `None` until the user has chosen.
    home: Option<Home>,
    /// The view's edit count that is safely on disk.
    saved_edits: u64,
    /// The edit count a save was last planned for.
    noticed_edits: u64,
    save_at: Option<Instant>,
    saving: Option<(u64, JoinHandle<Result<Home, String>>)>,
    save_failed: bool,
}

/// Something a key asks of the app, not of the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    New,
    Open,
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
        Key::Char('o') => Action::Open,
        Key::Char('s') => Action::Save,
        Key::Char('w') => Action::Close,
        Key::Char('q') => Action::Quit,
        _ => return None,
    })
}

impl DocWindow {
    fn name(&self) -> String {
        self.home.as_ref().map_or_else(|| UNTITLED.into(), Home::title)
    }

    fn unsaved(&self) -> bool {
        self.view.edits() != self.saved_edits
    }

    /// The window title carries the save state until the top bar exists to show it.
    fn retitle(&mut self) {
        let name = self.name();
        self.view.set_document_name(&name);
        let state = match (self.save_failed, self.unsaved()) {
            (true, _) => " · Not saved",
            (false, true) => " · Edited",
            (false, false) => "",
        };
        self.window.set_title(&format!("{name}{state}"));
    }

    /// Fits the view to the window, which may have changed size or moved to another display.
    fn fit(&mut self) {
        let scale = self.window.scale_factor();
        let size = self.window.inner_size().to_logical::<f32>(scale);
        self.view.resize(Size { width: size.width, height: size.height }, scale as f32);
    }

    /// Paints one frame and puts it on screen.
    fn redraw(&mut self, now_ms: u64) -> Result<(), softbuffer::SoftBufferError> {
        let size = self.window.inner_size();
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        self.wake_ms = self.view.animate(now_ms);
        self.list.items.clear();
        self.view.paint(&mut self.list);
        self.surface.resize(width, height)?;
        let mut buffer = self.surface.buffer_mut()?;
        self.painter.paint(&self.list, self.window.scale_factor() as f32, view::PAGE, size.width, &mut buffer);
        self.window.pre_present_notify();
        buffer.present()
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

    /// Starts saving on another thread, if there is somewhere to save and no save is running.
    fn start_save(&mut self, proxy: &EventLoopProxy<Event>) {
        let Some(home) = self.home.clone().filter(|_| self.saving.is_none()) else { return };
        self.save_at = None;
        let document = self.view.editor().doc().clone();
        let (id, proxy) = (self.window.id(), proxy.clone());
        let save = thread::spawn(move || {
            let result = home.save(&document).map(|(home, _)| home).map_err(|error| error.to_string());
            let _ = proxy.send_event(Event::Saved(id));
            result
        });
        self.saving = Some((self.view.edits(), save));
    }

    /// Collects a running save, waiting for it if it has not finished.
    fn finish_save(&mut self, now: Instant) {
        let Some((edits, save)) = self.saving.take() else { return };
        match save.join().unwrap_or_else(|_| Err("the save stopped unexpectedly".into())) {
            Ok(home) => {
                self.home = Some(home);
                self.saved_edits = edits;
                self.save_failed = false;
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

    /// Saves now, on this thread, if anything is unsaved. Returns whether everything is on disk.
    fn save_now(&mut self) -> bool {
        self.finish_save(Instant::now());
        !self.unsaved() || self.write()
    }

    /// Writes the document to its home on this thread. Returns whether that worked.
    fn write(&mut self) -> bool {
        let Some(home) = &self.home else { return false };
        let saved = home.save(self.view.editor().doc());
        self.save_failed = saved.is_err();
        match saved {
            Ok((home, _)) => {
                self.home = Some(home);
                self.saved_edits = self.view.edits();
                self.noticed_edits = self.saved_edits;
                self.save_at = None;
            }
            Err(error) => self.alert(&format!("{} could not be saved.\n\n{error}", self.name())),
        }
        self.retitle();
        !self.save_failed
    }

    /// Asks where an untitled document should live, then saves it there, blank or not.
    fn choose_home(&mut self) -> bool {
        let dialog = FileDialog::new().add_filter("Letters document", &[EXTENSION]).set_parent(&*self.window);
        let Some(path) = dialog.set_file_name(format!("{UNTITLED}.{EXTENSION}")).save_file() else { return false };
        self.home = Some(Home::new(path.with_extension(EXTENSION)));
        self.write()
    }

    fn alert(&self, message: &str) {
        alert(Some(&self.window), message);
    }

    /// Makes sure nothing would be lost by closing. Returns `false` if the user chose to stay.
    fn ready_to_close(&mut self) -> bool {
        if self.home.is_some() {
            return self.save_now();
        }
        if !self.unsaved() {
            return true;
        }
        let answer = MessageDialog::new()
            .set_level(MessageLevel::Warning)
            .set_title("Keep this document?")
            .set_description("It has not been saved anywhere yet.")
            .set_buttons(MessageButtons::YesNoCancelCustom("Save".into(), "Discard".into(), "Cancel".into()))
            .set_parent(&*self.window)
            .show();
        match answer {
            MessageDialogResult::Yes => self.choose_home(),
            MessageDialogResult::Custom(choice) if choice == "Save" => self.choose_home(),
            MessageDialogResult::No => true,
            MessageDialogResult::Custom(choice) => choice == "Discard",
            _ => false,
        }
    }
}

fn alert(parent: Option<&Rc<Window>>, message: &str) {
    eprintln!("letters: {message}");
    let dialog = MessageDialog::new().set_level(MessageLevel::Error).set_title("Letters").set_description(message);
    match parent {
        Some(window) => dialog.set_parent(&**window).show(),
        None => dialog.show(),
    };
}

impl App {
    /// Opens a window on a document. With no `opened`, the document is blank and has no home yet.
    fn open_window(&mut self, event_loop: &ActiveEventLoop, opened: Option<Opened>) -> Result<(), Box<dyn Error>> {
        let (document, style, home) = match opened {
            Some(Opened { document, style, home }) => (document, style, Some(home)),
            None => (files::blank(), Style::default(), None),
        };
        let attributes = Window::default_attributes()
            .with_title(UNTITLED)
            .with_inner_size(WINDOW_SIZE)
            .with_min_inner_size(LogicalSize::new(360.0, 240.0))
            // AccessKit must attach before the window is first shown.
            .with_visible(false);
        let window = Rc::new(event_loop.create_window(attributes)?);
        let access = accesskit_winit::Adapter::with_event_loop_proxy(event_loop, &window, self.proxy.clone());
        let surface = softbuffer::Surface::new(&self.context, window.clone())?;
        window.set_ime_allowed(true);
        self.mark("window created");

        let mut window = DocWindow {
            view: View::new(Editor::new(document), style),
            window,
            surface,
            access,
            painter: Painter::new(),
            list: DisplayList::default(),
            composer: Composer::default(),
            pointer: Point::default(),
            cursor: Cursor::Default,
            access_stale: false,
            wake_ms: None,
            home,
            saved_edits: 0,
            noticed_edits: 0,
            save_at: None,
            saving: None,
            save_failed: false,
        };
        window.retitle();
        window.fit();
        self.mark("laid out");
        window.window.set_visible(true);
        window.window.request_redraw();
        self.windows.insert(window.window.id(), window);
        Ok(())
    }

    fn mark(&mut self, stage: &'static str) {
        if let Some(launch) = &mut self.launch {
            launch.mark(stage);
        }
    }

    /// Opens a file in a new window, or reports why it cannot be opened. Returns whether it opened.
    fn open_path(&mut self, event_loop: &ActiveEventLoop, path: &Path, parent: Option<WindowId>) -> bool {
        let parent = parent.and_then(|id| self.windows.get(&id)).map(|window| window.window.clone());
        let opened = files::open(path).map_err(|error| error.to_string());
        let shown = opened.and_then(|opened| self.open_window(event_loop, Some(opened)).map_err(|e| e.to_string()));
        if let Err(error) = &shown {
            alert(parent.as_ref(), &format!("{} could not be opened.\n\n{error}", path.display()));
        }
        shown.is_ok()
    }

    fn act(&mut self, event_loop: &ActiveEventLoop, id: WindowId, action: Action) {
        match action {
            Action::New => {
                if let Err(error) = self.open_window(event_loop, None) {
                    alert(None, &format!("A new window could not be opened.\n\n{error}"));
                }
            }
            Action::Open => {
                let Some(window) = self.windows.get(&id) else { return };
                let dialog = FileDialog::new().add_filter("Letters document", &[EXTENSION]).set_parent(&*window.window);
                let Some(path) = dialog.pick_file() else { return };
                // A blank, untouched window gives way to the document that was asked for.
                let pristine = window.home.is_none() && !window.unsaved();
                if self.open_path(event_loop, &path, Some(id)) && pristine {
                    self.windows.remove(&id);
                }
            }
            Action::Save => {
                let Some(window) = self.windows.get_mut(&id) else { return };
                if window.home.is_some() {
                    window.save_now();
                } else {
                    window.choose_home();
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

    fn close(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        if self.windows.get_mut(&id).is_some_and(DocWindow::ready_to_close) {
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
        self.mark("document read");
        let shown = opened.and_then(|opened| {
            self.open_window(event_loop, opened).map_err(|error| format!("No window could be opened.\n\n{error}"))
        });
        if let Err(failure) = shown {
            if !self.report_launch {
                alert(None, &failure);
            }
            self.fail(event_loop, failure);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Self { windows, platform, mods, .. } = self;
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
                        event_loop.exit();
                    }
                }
                return;
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => window.fit(),
            WindowEvent::Focused(focused) => {
                window.view.focus(focused);
                window.access_stale = true;
            }
            WindowEvent::ModifiersChanged(modifiers) => *mods = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !window.composer.active() =>
            {
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
                    ElementState::Pressed => {
                        window.view.pointer_down(window.pointer, input::modifiers(*mods, MAC), platform)
                    }
                    ElementState::Released => window.view.pointer_up(window.pointer, platform),
                }
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
                window.view.scroll(window.pointer, delta);
            }
            WindowEvent::CloseRequested => action = Some(Action::Close),
            _ => {}
        }
        window.sync(Instant::now());
        if let Some(action) = action {
            self.act(event_loop, id, action);
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: Event) {
        let now = Instant::now();
        match event {
            Event::Saved(id) => {
                if let Some(window) = self.windows.get_mut(&id) {
                    window.finish_save(now);
                }
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
            }
        }
    }

    /// Runs after each batch of events: starts due saves and sleeps until the next thing is due.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let now_ms = self.platform.now_ms();
        let mut next: Option<Instant> = None;
        for window in self.windows.values_mut() {
            if window.wake_ms.is_some_and(|wake| wake <= now_ms) {
                window.wake_ms = window.view.animate(now_ms);
                window.sync(now);
            }
            if window.save_at.is_some_and(|at| at <= now) {
                window.start_save(&self.proxy);
            }
            let wake = window.wake_ms.map(|ms| self.platform.instant(ms));
            // A save that is due but waiting on another needs no timer: that save's end wakes the loop.
            let save = window.save_at.filter(|_| window.saving.is_none());
            for due in [wake, save].into_iter().flatten() {
                next = Some(next.map_or(due, |next| next.min(due)));
            }
        }
        event_loop.set_control_flow(next.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}
