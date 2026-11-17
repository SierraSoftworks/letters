//! The page around the engine: a canvas it paints into, a hidden input beside the canvas that takes the
//! keyboard, and a mirror of the window as HTML for assistive technology.
//!
//! Every event ends the same way, in [`Shell::sync`], which asks for a frame if the view wants one and
//! moves the hidden input under the caret, where the input method's candidate window should open.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::convert::Infallible;
use std::mem;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use accesskit::{Action, ActionData, ActionRequest, NodeId, TextPosition, TextSelection, TreeId};
use js_sys::{Array, Function, Map, Object, Promise, Reflect, Uint8Array};
use letters_api::{Code, Fetch, Opened, Preview, Seen, Shares, paths};
use letters_editor::{Clip, Editor, Key, KeyEvent, Modifiers, Picture, Platform as _};
use letters_model::{
    BlockId, BlockKind, Change, Document, HistoryError, Mark, Model, Notes, Pos, ReplicaId, Revision, Selection, Style,
    TagKind,
};
use letters_render::cpu::{CpuRenderer, Frame, PixelRect};
use letters_render::{Color, DisplayList, Font, Point, Rect, Size};
use letters_sync::{Client, Link, Local, Progress, Role};
use letters_trust::{HttpsUrl, LinkDecision, Origin, Trust};
use letters_ui::{
    Access, Appearance, Connection, CopyFormat, Cursor, Edge, FontOffer, FontSource, Keys, LinkPreviewFound, Named,
    Notice, Peer, PictureState, Request, SaveState, Scheme, Settings, Shared, TrustedSource, Uncovered, Window as View,
    WordList, tokens,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::{JsFuture, future_to_promise};
use web_sys::{
    BinaryType, Blob, BlobPropertyBag, CanvasRenderingContext2d, ClipboardEvent, CompositionEvent, DragEvent, Element,
    Event, EventTarget, File, FileList, HtmlAnchorElement, HtmlCanvasElement, HtmlElement, HtmlIFrameElement,
    HtmlInputElement, HtmlTextAreaElement, ImageData, KeyboardEvent, MediaQueryList, MouseEvent, PointerEvent, Url,
    WebSocket, WheelEvent, Window,
};

use crate::browser::{self, Browser};
use crate::chosen::{self, Want};
use crate::faces::{self, Face};
use crate::input::{Around, Clipboard, Dom, Finger, Gesture, Held, Reply, TextInput, Touch, modifiers};
use crate::mirror::{Act, Mirror, Page};
use crate::page;

/// The distance one line of a mouse wheel scrolls, in logical pixels.
const WHEEL_LINE: f32 = 40.0;
/// The hidden input: on the page, so it can take focus and anchor the input method, and invisible. The
/// font size keeps phones from zooming in on it.
const HIDDEN: &str = "position:fixed;width:1px;padding:0;border:0;margin:0;outline:0;resize:none;overflow:hidden;\
                      opacity:0;pointer-events:none;white-space:pre;font-size:16px;";

/// The hidden input when a keyboard is to leave what is typed as it is.
const HELP: [(&str, &str); 2] = [("autocapitalize", "off"), ("autocorrect", "off")];

/// The frame a PDF is printed from. It has no size, and still has to count as shown for the browser to
/// load a PDF into it.
const PRINTED: &str = "position:fixed;width:0;height:0;border:0;visibility:hidden;";

#[wasm_bindgen(inline_js = "export async function load(url) {
    const module = await import(url);
    await module.default();
    return module;
}")]
extern "C" {
    /// Fetches and starts a module that `wasm-bindgen` made, which the page did not load with the editor.
    #[wasm_bindgen(catch)]
    async fn load(url: &str) -> Result<JsValue, JsValue>;
}

/// A handle at one end of a selection a finger made: a dot under the line, in a square large enough for a
/// finger to take hold of. It scrolls nothing, so that a finger on it moves the selection's end.
const HANDLE: &str = "position:fixed;width:40px;height:40px;margin-left:-20px;touch-action:none;";
/// The dot's radius, in pixels.
const DOT: f32 = 7.0;

/// The kinds of picture a document keeps, which is what the person is offered to choose from.
const PICTURES: &str = "image/png,image/jpeg,image/gif,image/webp";
/// What a document's file is called, which is what the person is offered to open.
const DOCUMENTS: &str = ".let";
/// How long writing rests before where it stopped is marked as a version, in seconds. The desktop marks
/// after the same rest, and the history ends a version at it.
const REST: u64 = 600;

/// The mirror: over the canvas, so that what it says about where things are is true, and out of sight and
/// out of the pointer's way. Its text is not for the eye, so the page lays out as little of it as it can.
const MIRROR: &str = "position:fixed;overflow:hidden;opacity:0;pointer-events:none;user-select:none;contain:strict;";
/// How long the window must stay as it is before the mirror is brought up to date, in milliseconds. Reading
/// the whole window costs more than a key press should wait for.
const MIRROR_MS: i32 = 150;
/// How long a page waits for its first session before the bar says that it is connecting.
const QUIET_MS: u64 = 1000;

/// The modifiers held during a keyboard or pointer event, which report them under the same names.
macro_rules! held {
    ($event:expr) => {
        Held {
            ctrl: $event.ctrl_key(),
            meta: $event.meta_key(),
            shift: $event.shift_key(),
            alt: $event.alt_key(),
            alt_graph: $event.get_modifier_state("AltGraph"),
        }
    };
}

/// How painting has gone, for the harness page and the tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub frames: u32,
    /// When the first frame was on the canvas, in milliseconds since the page started loading.
    pub first_frame_ms: Option<f64>,
    /// How long the latest frame took, from the display list to the canvas.
    pub frame_ms: f64,
    /// How long the mirror last took to bring up to date.
    pub mirror_ms: f64,
    /// Whether the dictionaries have arrived, so that misspellings are marked.
    pub spelling: bool,
    /// Whether every font has arrived, so that no text stands in another's face.
    pub fonts: bool,
    /// Whether the page is in session with its server, and its document's history is the server's.
    pub live: bool,
}

/// What the first frame does without. The fonts are fetched at once, and the rest once the frame is on the
/// canvas.
pub struct Later {
    /// Where the files are found: a URL that is empty or ends in a slash.
    pub base: String,
    /// Where the fonts still to come are, in the order `letters_fonts::provide` takes them.
    pub fonts: Vec<String>,
    /// Whether the maths faces are still to come. A document that holds maths had them before it was shown.
    pub maths: bool,
}

pub struct Shell {
    /// The shell itself, for what finishes after the call that began it.
    me: Weak<RefCell<Self>>,
    later: Option<Later>,
    /// Where the files beside the wasm are found: a URL that is empty or ends in a slash.
    base: String,
    /// The exports, once the person has printed or saved a copy: a module of its own, fetched then.
    exporter: Option<Promise>,
    /// How often the exports could not be fetched.
    refetches: u32,
    /// The files asked for because a document needed them, and how far each has come.
    extras: Vec<(Extra, Arrival)>,
    /// Those among them that the exports were handed.
    handed: Vec<Extra>,
    /// The fonts the document's writer chose, as far as the page has come by them.
    fonts: Chosen,
    /// The characters the window showed without a glyph, until a face that is here has them or none can.
    uncovered: Vec<Uncovered>,
    /// What waits for such files that are on their way, to be told whether any came.
    awaited: Vec<(Vec<Extra>, Arrived)>,
    /// How many files the exports are writing.
    exporting: u32,
    /// Whether a PDF is being made for the person to print.
    printing: bool,
    /// The frame the last PDF was printed from, and the address the PDF has in this page.
    printed: Option<(HtmlIFrameElement, String)>,
    window: Window,
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
    input: HtmlTextAreaElement,
    dark_mode: Option<MediaQueryList>,
    less_motion: Option<MediaQueryList>,
    on_frame: Function,
    on_wake: Function,
    on_mirror: Function,
    mirror: Page,
    /// The element the mirror is kept in.
    holder: Element,
    /// The timer that brings the mirror up to date once the window has settled.
    mirror_timer: Option<i32>,
    /// The file input the person chooses a stylesheet with. It is never shown.
    chooser: HtmlInputElement,
    /// The file input the person chooses pictures with.
    picker: HtmlInputElement,
    /// The file input the person chooses a document to open with.
    opener: HtmlInputElement,
    /// What `View::edits` said when the document was opened or last kept as a file of its own. What was
    /// written since is nowhere but on the page.
    kept: u64,
    /// The document's id and when it was created, which its file or its server says. One that never had
    /// a file is given them when it is first saved.
    identity: Option<[String; 2]>,
    /// What the server behind the page says of the document on it. `None` for a page with no server, and
    /// once the person has opened a file of their own in the server's document's place.
    server: Option<Opened>,
    /// The pictures the document links to that the server fetched, by the address each link is written
    /// as, for a PDF to show what the window shows.
    linked: HashMap<String, Vec<u8>>,
    /// The linked pictures the server would not fetch just then, having fetched all it does in a while, in
    /// the order they are asked for again.
    slow: VecDeque<(String, Linking)>,
    on_slow: Function,
    /// The timer for the next of them. `None` with pictures waiting while one is being asked for.
    slow_timer: Option<i32>,
    /// The sources of linked pictures the person trusts: those their account keeps, which the server
    /// fetches from unasked, or without an account those they said on this page, for as long as it is open.
    sources: Vec<letters_trust::Source>,
    /// The languages the server has a spelling dictionary for, once it has said, and those among them a
    /// dictionary was asked for.
    languages: Vec<String>,
    dictionaries: Vec<String>,
    /// The page of the server the person asked for, which waits until the server has what they wrote.
    leaving: Option<String>,
    /// Whether the server makes links to its documents, once it has said.
    links: Option<bool>,
    /// What the person set for themselves as their server keeps it, once it has said. What this browser
    /// keeps of it is a copy, which the page starts with.
    account: Option<letters_api::Settings>,
    /// The document's line to its server: it says what to send over the socket, and what to merge.
    client: Option<Client>,
    /// The socket the client last asked for.
    socket: Option<Line>,
    /// The socket before it, whose listeners are let go with the next: one of them may be running.
    hung: Option<Line>,
    /// How many sockets were asked for.
    sockets: u32,
    /// Whether the document's history is the server's, which comes with the first session. Until then
    /// the page shows the document it was sent, and the client is told nothing of it.
    attached: bool,
    /// Whether the person's name is as they set it, or their server did not say what they set: the window
    /// tells who else wrote by it. And whether the window was told what they last saw of the document.
    named: bool,
    seen: bool,
    /// The revision the person's account was last told they read the document up to, and when.
    seen_told: (Option<String>, Option<u64>),
    /// How many pictures are on their way to the server. The edits that name them wait for them.
    uploading: u32,
    /// What the server lets this person do with the document. It says so with the page, and again with
    /// each session.
    role: Role,
    /// What the window was last told the person may do.
    access: Access,
    /// Whether the editor is kept from writing though the window offers to: in the first moments of a
    /// page, before the server's history is the document's.
    held: bool,
    /// The selection as the client was last told it.
    told: Option<Selection>,
    /// How the window was last told it stands to the server.
    connection: Option<Connection>,
    /// Until when a wait for the first session shows nothing: a page that loads says nothing of a
    /// wait the person does not notice.
    quiet_until: Option<u64>,
    /// The count of edits at which the others' carets were last placed. `None` once they moved.
    peers_at: Option<u64>,
    on_beat: Function,
    /// The timer for the client's next move of its own, such as asking for a socket again.
    beat_timer: Option<i32>,
    /// Whether the document's file is being written.
    saving: bool,
    /// What the window was last told of saving.
    save_state: SaveState,
    /// The revision a file was opened at, until the person writes on from it.
    session: Option<Revision>,
    /// What `View::edits` last said.
    written: u64,
    /// The revision the last edit led to, and when it was made by the wall clock, in seconds.
    rested: Option<(Revision, u64)>,
    /// How far the browser's clock is ahead of UTC in minutes, as the window was last told.
    zone: i32,
    /// The handles at the ends of the selection, the anchor's first, and how each was last styled.
    handles: [(HtmlElement, String); 2],
    /// Whether the page was last pressed by a finger, which is who the handles are for.
    fingers: bool,
    /// Whether the event being handled is an edit to the hidden input.
    editing: bool,
    /// The handle a finger holds, and where the finger is from the middle of the line its end is on.
    grab: Option<(usize, Point)>,

    view: View,
    platform: Browser,
    text: TextInput,
    touch: Touch,
    mac: bool,

    renderer: CpuRenderer,
    frame: Frame,
    list: DisplayList,
    /// The pixels of one repainted rectangle, on their way to the canvas.
    strip: Vec<u8>,
    scale: f32,

    pointer: Point,
    /// The pointer's shape on the canvas, by its name in CSS.
    cursor: &'static str,
    /// Where the hidden input was last put.
    caret: Option<Rect>,
    /// Whether a frame has been asked for and not yet painted.
    frame_due: bool,
    /// The timer for the view's next move of its own, such as the caret's blink.
    timer: Option<i32>,
    stats: Stats,
}

impl Shell {
    /// Shows the editor in `canvas`, whose size the page's CSS decides, and adds the hidden input after it.
    pub fn start(
        canvas: HtmlCanvasElement,
        mut platform: Browser,
        opened: Document,
        server: Option<Opened>,
        later: Later,
    ) -> Result<Rc<RefCell<Self>>, JsValue> {
        let window = web_sys::window().ok_or("there is no window")?;
        let document = window.document().ok_or("there is no document")?;
        // The window offers no printing that is withheld. The browser's own would print the canvas: the
        // page's styles leave it out of what a page so marked prints.
        if let Some(root) =
            document.document_element().filter(|_| server.as_ref().is_some_and(|server| !server.allows.print))
        {
            root.set_attribute("data-print", "withheld")?;
        }
        // The view paints every pixel, so the canvas needs no alpha.
        let options = Object::new();
        Reflect::set(&options, &"alpha".into(), &false.into())?;
        let context = canvas.get_context_with_context_options("2d", &options)?.ok_or("the canvas is in use")?;
        let context: CanvasRenderingContext2d = context.dyn_into()?;

        let input: HtmlTextAreaElement = document.create_element("textarea")?.dyn_into()?;
        for (name, value) in
            [("style", HIDDEN), ("aria-label", "Document"), ("autocomplete", "off"), ("spellcheck", "false")]
                .into_iter()
                .chain(HELP)
        {
            input.set_attribute(name, value)?;
        }
        canvas.insert_adjacent_element("afterend", &input)?;
        // The canvas itself says nothing. The mirror follows the input, where a reader comes to it next.
        canvas.set_attribute("aria-hidden", "true")?;
        let holder = document.create_element("div")?;
        input.insert_adjacent_element("afterend", &holder)?;
        let chooser: HtmlInputElement = document.create_element("input")?.dyn_into()?;
        for (name, value) in
            [("type", "file"), ("accept", ".letstyle,application/json"), ("hidden", ""), ("tabindex", "-1")]
        {
            chooser.set_attribute(name, value)?;
        }
        holder.insert_adjacent_element("afterend", &chooser)?;
        let picker: HtmlInputElement = document.create_element("input")?.dyn_into()?;
        for (name, value) in
            [("type", "file"), ("accept", PICTURES), ("multiple", ""), ("hidden", ""), ("tabindex", "-1")]
        {
            picker.set_attribute(name, value)?;
        }
        chooser.insert_adjacent_element("afterend", &picker)?;
        let opener: HtmlInputElement = document.create_element("input")?.dyn_into()?;
        for (name, value) in [("type", "file"), ("accept", DOCUMENTS), ("hidden", ""), ("tabindex", "-1")] {
            opener.set_attribute(name, value)?;
        }
        picker.insert_adjacent_element("afterend", &opener)?;
        let handle = || -> Result<(HtmlElement, String), JsValue> {
            let handle: HtmlElement = document.create_element("div")?.dyn_into()?;
            for (name, value) in [("data-letters", "handle"), ("aria-hidden", "true"), ("hidden", "")] {
                handle.set_attribute(name, value)?;
            }
            picker.insert_adjacent_element("afterend", &handle)?;
            Ok((handle, String::new()))
        };
        let handles = [handle()?, handle()?];

        let mac = window.navigator().platform().is_ok_and(|name| name.starts_with("Mac") || name.starts_with("iP"));
        let dark_mode = window.match_media("(prefers-color-scheme: dark)")?;
        let less_motion = window.match_media("(prefers-reduced-motion: reduce)")?;
        let view = match &server {
            Some(server) => {
                let replica = ReplicaId::new(&platform.new_id());
                let mut view = set_up(page::served_window(opened, server, replica), &mut platform, mac);
                // The session's name signs the edits, whatever name this browser kept from another time.
                view.set_author(&server.author);
                // The server reads the pages a browser may not.
                view.set_preview_fetching(true);
                view
            }
            None => view(opened, &mut platform, mac),
        };
        let identity = server.as_ref().map(|server| [server.id.clone(), server.created.clone()]);

        let shell = Rc::new_cyclic(|shell| {
            RefCell::new(Self {
                me: shell.clone(),
                base: later.base.clone(),
                exporter: None,
                refetches: 0,
                extras: Vec::new(),
                handed: Vec::new(),
                fonts: Chosen::default(),
                uncovered: Vec::new(),
                awaited: Vec::new(),
                exporting: 0,
                printing: false,
                printed: None,
                later: Some(later),
                on_frame: callback(shell, Self::paint),
                on_wake: callback(shell, Self::wake),
                on_mirror: callback(shell, Self::reflect),
                mirror: Page::new(&document, holder.clone()),
                holder: holder.clone(),
                mirror_timer: None,
                chooser: chooser.clone(),
                picker: picker.clone(),
                opener: opener.clone(),
                kept: view.edits(),
                identity,
                server,
                linked: HashMap::new(),
                slow: VecDeque::new(),
                on_slow: callback(shell, Self::link_again),
                slow_timer: None,
                sources: Vec::new(),
                languages: Vec::new(),
                dictionaries: Vec::new(),
                leaving: None,
                links: None,
                account: None,
                client: None,
                socket: None,
                hung: None,
                sockets: 0,
                attached: false,
                named: false,
                seen: false,
                seen_told: (None, None),
                uploading: 0,
                role: Role::Editor,
                access: Access::Edits,
                held: false,
                told: None,
                connection: None,
                quiet_until: None,
                peers_at: None,
                on_beat: callback(shell, Self::beat),
                beat_timer: None,
                saving: false,
                save_state: SaveState::default(),
                session: None,
                written: view.edits(),
                rested: None,
                zone: zone(),
                handles: handles.clone(),
                fingers: false,
                editing: false,
                grab: None,
                window: window.clone(),
                canvas: canvas.clone(),
                context,
                input: input.clone(),
                dark_mode: dark_mode.clone(),
                less_motion: less_motion.clone(),
                view,
                platform,
                text: TextInput::new(mac),
                touch: Touch::default(),
                mac,
                renderer: CpuRenderer::new(),
                frame: Frame::default(),
                list: DisplayList::default(),
                strip: Vec::new(),
                scale: 1.0,
                pointer: Point::default(),
                cursor: "default",
                caret: None,
                frame_due: false,
                timer: None,
                stats: Stats::default(),
            })
        });

        // A press on the canvas must leave the keyboard with the hidden input, or give it back. The input
        // is focused here, outside the shell, because the browser reports the focus before `focus` returns.
        // A finger asks for the keyboard with a tap, which the browser reports as a click. One that goes
        // down may be about to scroll, and must not bring the soft keyboard up.
        let focus = input.clone();
        listen(&canvas, "pointerdown", move |event| {
            if event.unchecked_ref::<PointerEvent>().pointer_type() != "touch" {
                report(focus.focus());
            }
        })?;
        let focus = input.clone();
        listen(&canvas, "click", move |_| report(focus.focus()))?;
        listen(&canvas, "mousedown", |event| event.prevent_default())?;
        // One finger scrolls the document, which is this shell's to do. Two still zoom the page.
        canvas.style().set_property("touch-action", "pinch-zoom")?;

        // Assistive technology presses what it finds in the mirror. The keyboard goes back to the hidden
        // input afterwards, as it does after a press on the canvas.
        let (pressed, focus) = (Rc::downgrade(&shell), input.clone());
        listen(&holder, "click", move |event| {
            let mut nodes = Vec::new();
            let mut at = event.target().and_then(|target| target.dyn_into::<Element>().ok());
            while let Some(element) = at {
                nodes.extend(Mirror::node(&element.id()));
                at = element.parent_element();
            }
            if let Some(shell) = pressed.upgrade() {
                let mut shell = shell.borrow_mut();
                shell.act(&nodes);
                shell.changed();
                shell.sync();
            }
            report(focus.focus());
        })?;

        // A stylesheet the person chose is read, which takes a moment, and then offered in the window.
        let offered = Rc::downgrade(&shell);
        listen(&chooser, "change", move |event| {
            let chooser: HtmlInputElement = event.target().expect("the chooser heard it").unchecked_into();
            let Some(file) = chooser.files().and_then(|files| files.get(0)) else { return };
            // The same file chosen again is a change again.
            chooser.set_value("");
            let (offered, name) = (offered.clone(), file.name());
            wasm_bindgen_futures::spawn_local(async move {
                let text = JsFuture::from(file.text()).await.ok().and_then(|text| text.as_string());
                let Some(shell) = offered.upgrade() else { return };
                let mut shell = shell.borrow_mut();
                shell.view.offer_stylesheet(&name, &text.unwrap_or_default());
                shell.arrived();
            });
        })?;

        hear(&shell, &picker, "change", Self::picked)?;
        hear(&shell, &opener, "change", Self::chosen)?;
        hear(&shell, &canvas, "dragover", Self::drag_over)?;
        hear(&shell, &canvas, "dragleave", |shell, _: Event| _ = shell.view.drag_pictures(None))?;
        hear(&shell, &canvas, "drop", Self::drop)?;
        hear(&shell, &canvas, "pointerdown", Self::pointer_down)?;
        hear(&shell, &canvas, "pointermove", Self::pointer_move)?;
        hear(&shell, &canvas, "pointerup", Self::pointer_up)?;
        hear(&shell, &canvas, "pointercancel", Self::pointer_cancel)?;
        hear(&shell, &canvas, "pointerleave", |shell, _: Event| shell.view.pointer_leave())?;
        for (which, (handle, _)) in handles.iter().enumerate() {
            // A finger that rests on a handle is holding it, not asking the browser for a menu.
            listen(handle, "contextmenu", |event| event.prevent_default())?;
            for kind in ["pointerdown", "pointermove", "pointerup", "pointercancel"] {
                hear(&shell, handle, kind, move |shell, event: PointerEvent| shell.handle(which, event))?;
            }
        }
        hear(&shell, &canvas, "wheel", Self::wheel)?;
        // The secondary button, Ctrl with a press on a Mac and a long press on a touch screen all ask for
        // a menu this way. The Menu key and Shift+F10 ask the element that has the keyboard.
        hear(&shell, &canvas, "contextmenu", Self::menu)?;
        hear(&shell, &input, "contextmenu", Self::menu_key)?;
        hear(&shell, &input, "keydown", Self::key_down)?;
        hear(&shell, &input, "beforeinput", Self::before_input)?;
        hear(&shell, &input, "input", Self::input)?;
        hear(&shell, &input, "compositionstart", |shell, event: Event| shell.text(Dom::CompositionStart, &event))?;
        hear(&shell, &input, "compositionend", Self::composition_end)?;
        // Every browser reports a selection that moved to the document, and only some to the input too.
        // Hearing it is enough: whatever the shell hears, it first looks where the keyboard left the caret.
        hear(&shell, &document, "selectionchange", |_, _: Event| {})?;
        hear(&shell, &input, "copy", |shell, event| shell.clipboard(event, Clipboard::Copy))?;
        hear(&shell, &input, "cut", |shell, event| shell.clipboard(event, Clipboard::Cut))?;
        hear(&shell, &input, "paste", |shell, event| shell.clipboard(event, Clipboard::Paste))?;
        hear(&shell, &input, "focus", |shell, _: Event| shell.view.focus(true))?;
        hear(&shell, &input, "blur", |shell, _: Event| shell.view.focus(false))?;
        hear(&shell, &window, "resize", |shell, _: Event| shell.fit())?;
        // Someone else may have renamed or moved the document while the person was away.
        hear(&shell, &window, "focus", |shell, _: Event| shell.look())?;
        // The browser asks before it leaves a page whose work the server lacks.
        hear(&shell, &window, "beforeunload", |shell, event: Event| {
            if shell.unsent() {
                event.prevent_default();
            }
        })?;
        hear(&shell, &window, "pagehide", |shell, _: Event| shell.keep_seen(false))?;
        // Another tab of the editor may have added to the person's words, kept another default style, or
        // changed their settings.
        hear(&shell, &window, "storage", |shell, _: Event| {
            shell.view.set_user_words(shell.platform.user_words());
            shell.view.set_default_style(shell.platform.default_style());
            let settings = shell.platform.settings();
            if *shell.view.settings() != settings {
                follow(&mut shell.view, settings);
                shell.fit();
            }
        })?;
        for setting in [&dark_mode, &less_motion].into_iter().flatten() {
            hear(&shell, setting, "change", |shell, _: Event| shell.fit())?;
        }

        input.focus()?;
        shell.borrow_mut().rest();
        shell.borrow_mut().show();
        shell.borrow_mut().fetch_fonts();
        shell.borrow_mut().go_online();
        shell.borrow_mut().ask_settings();
        Ok(shell)
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn document(&self) -> &Document {
        self.view.editor().doc()
    }

    /// Shows `document` in place of the one on screen.
    pub fn open(&mut self, document: Document) {
        self.view = view(document, &mut self.platform, self.mac);
        self.backed();
        self.go_offline();
        (self.kept, self.identity, self.server) = (self.view.edits(), None, None);
        self.linked.clear();
        (self.session, self.written, self.rested) = (None, self.kept, None);
        self.save_state = SaveState::default();
        self.text = TextInput::new(self.mac);
        self.touch = Touch::default();
        self.rest();
        self.show();
    }

    /// Everything this replica knows of the document, for another replica to merge.
    pub fn history(&self) -> Vec<u8> {
        self.view.editor().model().save()
    }

    /// Takes in what another replica wrote, as [`Shell::history`] gives it. The page stays where it is
    /// scrolled to, and the caret stays with its text.
    pub fn merge(&mut self, history: &[u8]) -> Result<(), HistoryError> {
        self.view.change(|editor| editor.merge(history))?;
        self.changed();
        self.sync();
        Ok(())
    }

    /// Fetches the fonts the first frame does not wait for. Text that asks for one is set in the first font
    /// until it arrives, and set again then.
    fn fetch_fonts(&mut self) {
        let urls = self.later.as_mut().map(|later| std::mem::take(&mut later.fonts)).unwrap_or_default();
        if urls.is_empty() {
            self.stats.fonts = true;
            return;
        }
        let (shell, window) = (self.me.clone(), self.window.clone());
        wasm_bindgen_futures::spawn_local(async move {
            match browser::files(&window, &urls).await {
                Ok(files) => letters_fonts::provide(files),
                Err(error) => return web_sys::console::error_1(&error),
            }
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            shell.view.add_fonts(letters_fonts::bundled());
            shell.stats.fonts = true;
            shell.arrived();
        });
    }

    /// Fetches what the first frame did without, and puts it to use: the document's spelling is checked
    /// once there are dictionaries to check it against.
    fn fetch(&self, later: Later) {
        let (shell, window) = (self.me.clone(), self.window.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let mut urls = urls(&later.base, "dictionaries", &letters_spell::ENGLISH_FILES);
            if later.maths {
                urls.extend(self::urls(&later.base, "maths", &letters_math::FONT_FILES));
            }
            let mut files = match browser::files(&window, &urls).await {
                Ok(files) => files,
                Err(error) => return web_sys::console::error_1(&error),
            };
            let faces = files.split_off(letters_spell::ENGLISH_FILES.len());
            if later.maths {
                letters_math::provide_fonts(faces);
            }
            let text = |file: Vec<u8>| String::from_utf8_lossy(&file).into_owned();
            letters_spell::provide_english(files.into_iter().map(text).collect());
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            let words = shell.platform.user_words();
            shell.view.set_user_words(words);
            shell.stats.spelling = true;
            // The check runs in the time a frame leaves over, so there must be a frame.
            if !shell.frame_due {
                shell.frame_due = shell.window.request_animation_frame(&shell.on_frame).is_ok();
            }
            shell.arrived();
        });
    }

    /// Opens a link the person followed, in another tab. Only a server keeps word of what they trust, so
    /// without its word every address is shown whole first, and what would run something here is never
    /// opened.
    fn open_link(&mut self, href: &str, confirmed: bool) {
        let trusted = self.server.as_ref().is_some_and(|server| server.trust == letters_api::Trust::Trusted);
        let origin = if trusted { Origin::Created } else { Origin::Outside };
        match Trust::default().gate("", origin).link(href) {
            LinkDecision::Refuse => self.view.refuse_link(href, "Letters does not open addresses of this kind."),
            LinkDecision::Ask if !confirmed => self.view.ask_link(href),
            // The page it opens learns nothing of this one, and cannot reach back into it.
            _ => report(
                self.window.open_with_url_and_target_and_features(href, "_blank", "noopener,noreferrer").map(drop),
            ),
        }
    }

    /// Has the server keep that the person trusts this document, which ends its safe mode for them here
    /// and wherever they open it next.
    fn trust(&mut self) {
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else { return };
        let (shell, window) = (self.me.clone(), self.window.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let answer = browser::ask(&window, "POST", &paths::trust(&id), None).await;
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // The page may show another document by now.
            let Some(server) = shell.server.as_mut().filter(|server| server.id == id) else { return };
            match answer {
                Ok(_) => {
                    server.trust = letters_api::Trust::Trusted;
                    shell.view.set_safe_mode(None);
                }
                Err(refusal) => shell.say(refusal.message),
            }
            shell.arrived();
        });
    }

    /// Offers `file` to the person as a file to keep, called `name`: text, or bytes.
    fn download(&self, file: &JsValue, kind: &str, name: &str) -> Result<(), JsValue> {
        let page = self.window.document().ok_or("there is no document")?;
        let options = BlobPropertyBag::new();
        options.set_type(kind);
        let file = Blob::new_with_u8_array_sequence_and_options(&Array::of1(file), &options)?;
        let url = Url::create_object_url_with_blob(&file)?;
        keep(&page, &url, name)?;
        Url::revoke_object_url(&url)
    }

    // Printing and copies

    /// The exports with their fonts, fetched when they are first asked for so that no other visit waits
    /// for them.
    fn exporter(&mut self) -> Promise {
        if let Some(exporter) = &self.exporter {
            return exporter.clone();
        }
        let (window, base) = (self.window.clone(), self.base.clone());
        // A browser remembers a module it could not fetch, so another try asks under another address.
        let again = Some(self.refetches).filter(|tries| *tries > 0).map(|tries| format!("?again={tries}"));
        let exporter = future_to_promise(async move {
            let module = format!("{base}export.js{}", again.unwrap_or_default());
            let module = Url::new_with_base(&module, &window.location().href()?)?;
            let exporter = load(&module.href()).await?;
            // The editor has fetched these already, so the browser has them at hand.
            let fonts = browser::files(&window, &urls(&base, "fonts", &letters_fonts::FILES)).await?;
            let maths = browser::files(&window, &urls(&base, "maths", &letters_math::FONT_FILES)).await?;
            let files =
                |files: Vec<Vec<u8>>| -> Array { files.iter().map(|file| Uint8Array::from(&file[..])).collect() };
            call(&exporter, "fonts", &Array::of2(&files(fonts), &files(maths)))?;
            Ok(exporter)
        });
        self.exporter.insert(exporter).clone()
    }

    /// Has the exports write `source` in the format whose files end in `format`, and hands `then` the
    /// file, or why there is none in words that end a sentence.
    fn export(
        &mut self,
        format: &'static str,
        source: Source,
        title: String,
        then: impl FnOnce(&mut Self, Result<Exported, String>) + 'static,
    ) {
        let document = match &source {
            Source::Version(version) => version.0.clone(),
            Source::Own | Source::Copy => self.view.editor().doc().clone(),
        };
        let style = match &source {
            Source::Version(version) => version.1.clone(),
            Source::Own | Source::Copy => self.view.style().clone(),
        };
        self.gather(&document, &style, move |shell, gathered| match gathered {
            Ok(()) => shell.export_with(format, source, title, 2, Box::new(then)),
            Err(why) => then(shell, Err(why)),
        });
    }

    /// Writes the file [`Self::export`] asks for. A PDF that shows characters as boxes for want of a face
    /// that backs up the fonts is made again once the face is fetched, as often as `again` allows: the
    /// face a character is first set in may lack it, and the next be asked for.
    fn export_with(&mut self, format: &'static str, source: Source, title: String, again: u8, then: Made) {
        let editor = self.view.editor();
        // A document's file takes the history with it, which nothing else has a use for.
        let kept = format == "let";
        let (document, style, notes, history) = match &source {
            // A version starts a document of its own, whose history begins with what it read. The notes
            // are on what is written now, and stay with it.
            Source::Version(version) => {
                let (document, style) = &**version;
                let replica = ReplicaId::new(&self.platform.new_id());
                let history = kept.then(|| Model::new(document.clone(), replica).with_style(style.clone()).save());
                (document.to_json(), style.to_json(), Notes::default().to_json(), history)
            }
            Source::Own | Source::Copy => {
                let history = kept.then(|| editor.model().save());
                (editor.doc().to_json(), self.view.style().to_json(), self.view.notes().to_json(), history)
            }
        };
        let history = history.map(|history| Uint8Array::from(&history[..]));
        let authors: Array = editor.model().authors().iter().map(|author| JsValue::from(author.as_str())).collect();
        let created = self.platform.wall_time().map(|seconds| seconds as f64);
        let assets = Map::new();
        // The linked pictures the window shows go along by their addresses, for a PDF to show them too.
        let held = self.platform.assets.iter().chain(&self.linked);
        // The font files the style lists go along too.
        for (name, file) in held.filter(|(name, _)| document.contains(name.as_str()) || style.contains(name.as_str())) {
            assets.set(&name.into(), &Uint8Array::from(&file[..]));
        }
        let identity = kept.then(|| match source {
            Source::Own => self.identity.get_or_insert_with(|| self.platform.identity()).clone(),
            // A copy never passes for the document it was made of.
            Source::Copy | Source::Version(..) => self.platform.identity(),
        });
        let identity = identity.map(|identity| identity.into_iter().map(JsValue::from).collect::<Array>());
        let asked: [JsValue; 10] = [
            format.into(),
            title.as_str().into(),
            document.into(),
            style.into(),
            notes.into(),
            authors.into(),
            created.into(),
            assets.into(),
            history.into(),
            identity.into(),
        ];
        let retry = again.checked_sub(1).map(|again| (source, title.clone(), again));
        self.ask("exported", asked.into_iter().collect(), move |shell, made| {
            let lacking: Vec<Extra> =
                made.iter().flat_map(|made| Array::from(&made.get(3))).filter_map(extra).collect();
            let made = made.map(|made| Exported {
                name: made.get(0).as_string(),
                kind: made.get(1).as_string().unwrap_or_default(),
                file: made.get(2),
            });
            let Some((source, title, again)) = retry.filter(|_| !lacking.is_empty()) else {
                return then(shell, made);
            };
            // The file as it is will do if what it lacks cannot be had.
            shell.exporting += 1;
            shell.back_up(lacking, move |shell, arrived| {
                shell.exporting -= 1;
                match arrived {
                    true => shell.export_with(format, source, title, again, then),
                    false => then(shell, made),
                }
            });
        });
    }

    /// Calls a function of the exports with what is `asked`, and hands `then` what it returns, or why
    /// there is nothing in words that end a sentence.
    fn ask(
        &mut self,
        function: &'static str,
        asked: Array,
        then: impl FnOnce(&mut Self, Result<Array, String>) + 'static,
    ) {
        self.exporting += 1;
        let (shell, exporter) = (self.me.clone(), self.exporter());
        wasm_bindgen_futures::spawn_local(async move {
            let asked_of = exporter.clone();
            let exporter = JsFuture::from(exporter).await;
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            let made = match &exporter {
                Ok(exporter) => {
                    shell.hand_backups(exporter);
                    call(exporter, function, &asked).map(JsCast::unchecked_into).map_err(|error| said(&error))
                }
                Err(error) => {
                    web_sys::console::error_1(error);
                    Err("the part of Letters that does it could not be fetched".to_owned())
                }
            };
            shell.exporting -= 1;
            // What could not be fetched is asked for again the next time.
            if exporter.is_err() && shell.exporter.as_ref().is_some_and(|kept| Object::is(kept, &asked_of)) {
                (shell.exporter, shell.refetches) = (None, shell.refetches + 1);
                shell.handed.clear();
            }
            then(&mut shell, made);
            shell.arrived();
        });
    }

    /// The files a document needed, as far as they have come.
    fn fetched(&self) -> impl Iterator<Item = (Extra, &Arc<[u8]>)> {
        self.extras.iter().filter_map(|(extra, arrival)| match arrival {
            Arrival::Here(file) => Some((*extra, file)),
            Arrival::Coming | Arrival::Lost => None,
        })
    }

    /// The faces that back up the fonts, as far as they have come.
    fn faces(&self) -> impl Iterator<Item = (Face, &Arc<[u8]>)> {
        self.fetched().filter_map(|(extra, file)| match extra {
            Extra::Face(face) => Some((face, file)),
            Extra::Words(_) => None,
        })
    }

    /// Hands a window that was just made the faces that back up the fonts and the word lists, as far as
    /// they were fetched. What the window before it lacked is no longer asked about.
    fn backed(&mut self) {
        self.uncovered.clear();
        // Another document's fonts are none of this one's. The library's list holds for the page.
        self.fonts = Chosen { library: mem::take(&mut self.fonts.library), ..Chosen::default() };
        self.offer_fonts();
        let fetched: Vec<Font> = self.faces().map(|(_, file)| Font::new(file.clone(), 0)).collect();
        if !fetched.is_empty() {
            self.view.add_fallback_fonts(fetched);
        }
        let fetched: Vec<_> = self.fetched().map(|(extra, file)| (extra, file.clone())).collect();
        for (extra, file) in fetched {
            if let Extra::Words(list) = extra {
                self.view.add_word_list(list, &file);
            }
        }
    }

    /// Asks what the window showed that it lacks something for, and fetches that: the word list of a
    /// script written without spaces, and the faces that have the characters no face has. A font that is
    /// still on its way may have them too, so no face is asked for before the last has come.
    fn cover(&mut self) {
        let unlisted: Vec<Extra> = self.view.take_unlisted().into_iter().map(Extra::Words).collect();
        self.fetch_extras(&unlisted);
        if self.stats.fonts {
            self.uncovered.extend(self.view.take_uncovered());
        }
        if self.uncovered.is_empty() {
            return;
        }
        let arrival = |face: Face| self.extras.iter().find(|(asked, _)| *asked == Extra::Face(face));
        let lacks = |face: Face, ch: char| match arrival(face) {
            Some((_, Arrival::Here(file))) => !faces::has(file, ch),
            Some((_, Arrival::Lost)) => true,
            Some((_, Arrival::Coming)) | None => false,
        };
        let asked: Vec<Face> = Face::ALL.into_iter().filter(|face| arrival(*face).is_some()).collect();
        let told = self.uncovered.iter().map(|one| (one.ch, one.emoji, one.lang.as_deref()));
        let chosen = faces::chosen(told, &asked, lacks);
        // A character waits only for a face that is still to come.
        let here: Vec<Face> = self.faces().map(|(face, _)| face).collect();
        let mut waits = chosen.iter().map(|face| face.is_some_and(|face| !here.contains(&face)));
        self.uncovered.retain(|_| waits.next() == Some(true));
        let lacking: Vec<Extra> = faces::lacking(&chosen, &asked).into_iter().map(Extra::Face).collect();
        self.fetch_extras(&lacking);
    }

    /// Whether the person may fetch a family of the library and keep it with the document: one who
    /// writes a document of a server that they trust.
    fn keeps_fonts(&self) -> bool {
        let trusted = self.server.as_ref().is_some_and(|server| server.trust == letters_api::Trust::Trusted);
        trusted && self.role == Role::Editor
    }

    /// Offers the library's families in the Font menu, once their list is here.
    fn offer_fonts(&mut self) {
        let Listing::Here(library) = &self.fonts.library else { return };
        let families = letters_fonts::library::read(library).filter(|family| !chosen::BUNDLED.contains(&family.name));
        let offers = families.map(|family| FontOffer { family: family.name.to_owned(), source: FontSource::Library });
        self.view.set_families(offers.collect());
    }

    /// Sets the document in the fonts its writer chose, as far as they can be had: reads the font files
    /// its style lists, fetches the faces of a family of the library for whoever may keep them, lists
    /// those in the style once all are here, and says once which families are not to be had.
    fn choose(&mut self) {
        self.fonts.named.extend(self.view.take_named());
        let writes = self.server.is_some() && self.role == Role::Editor;
        if writes && self.fonts.library == Listing::Unasked {
            self.list_library();
        }
        let style = self.view.style();
        let (listed, faces) = (style.fonts(), letters_layout::faces(style));
        let bundled = |face: &Named| chosen::BUNDLED.iter().any(|own| own.eq_ignore_ascii_case(&face.family));
        if listed.is_empty() && self.fonts.named.is_empty() && faces.iter().all(bundled) {
            return;
        }
        // What the page holds is here, and what no server can give is lost.
        let mut here = Vec::new();
        for font in &listed {
            if self.fonts.files.contains_key(&font.asset) {
                continue;
            }
            let arrival = match self.platform.assets.get(&font.asset) {
                Some(file) if letters_fonts::is_font(file) => {
                    here.push(Font::new(file.as_slice().into(), 0));
                    chosen::Arrival::Here
                }
                None if self.server.is_some() => continue,
                Some(_) | None => chosen::Arrival::Lost,
            };
            self.fonts.files.insert(font.asset.clone(), arrival);
        }
        if !here.is_empty() {
            self.view.add_named_fonts(here);
        }
        let keeps = self.keeps_fonts();
        let library = match &self.fonts.library {
            Listing::Here(library) if keeps => Some(library.clone()),
            _ => None,
        };
        // Until the library's list has come, no family is given up on.
        let waits = keeps && matches!(self.fonts.library, Listing::Unasked | Listing::Coming);
        let plan = chosen::plan(&chosen::Known {
            listed: &listed,
            faces: &faces,
            named: &self.fonts.named,
            library: library.as_deref(),
            file: &|name| self.fonts.files.get(name).copied(),
            face: &|want| self.fonts.faces.get(want).copied(),
        });
        plan.read.into_iter().for_each(|name| self.read_font(name));
        plan.fetch.into_iter().for_each(|want| self.fetch_face(want));
        let coming = self.fonts.faces.values().any(|arrival| *arrival == chosen::Arrival::Coming);
        if library.is_some() && !coming {
            // One edit for the whole family, once the last of its faces is here.
            if let Some(list) = chosen::kept(&listed, &faces, &self.fonts.arrived) {
                self.view.keep_fonts(&list, &mut self.platform);
            }
        }
        let says =
            [(plan.lacking, page::NO_FONT, "OK", FONT_KEY), (plan.unfetched, page::NO_FETCH, "Try again", FETCH_KEY)];
        for (families, sentence, button, key) in says {
            for family in families.into_iter().filter(|_| !waits) {
                if self.fonts.said.insert(family.to_lowercase()) {
                    let sentence = sentence.replace("{}", &family);
                    self.view.notify(Notice { sentence, button: button.into(), key: format!("{key}{family}") });
                }
            }
        }
    }

    /// Asks the server for the library's families, once for the page.
    fn list_library(&mut self) {
        self.fonts.library = Listing::Coming;
        let asked = browser::ask(&self.window, "GET", paths::FONTS, None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let listed = asked.await.ok().and_then(|listing| String::from_utf8(listing).ok());
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // A server without the list offers no library, and the fonts a document names are missing.
            shell.fonts.library = listed.map_or(Listing::Lost, |listing| Listing::Here(listing.into()));
            shell.offer_fonts();
            shell.arrived();
        });
    }

    /// Reads a font file the style lists from the document's server.
    fn read_font(&mut self, name: String) {
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else { return };
        self.fonts.files.insert(name.clone(), chosen::Arrival::Coming);
        let asked = browser::ask(&self.window, "GET", &asset(&id, &name), None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let file = asked.await.ok().filter(|file| letters_fonts::is_font(file));
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // The page may show another document by now.
            if shell.server.as_ref().is_none_or(|server| server.id != id) {
                return;
            }
            let arrival = match file {
                Some(file) => {
                    shell.view.add_named_fonts(vec![Font::new(file.as_slice().into(), 0)]);
                    // A copy of the document takes its fonts along.
                    shell.platform.assets.entry(name.clone()).or_insert(file);
                    chosen::Arrival::Here
                }
                None => chosen::Arrival::Lost,
            };
            shell.fonts.files.insert(name, arrival);
            shell.arrived();
        });
    }

    /// Fetches a face of the library through the document's server, and keeps its file with the
    /// document: the server is handed it as it is handed a picture, ahead of the edit that lists it.
    fn fetch_face(&mut self, want: Want) {
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else { return };
        self.fonts.faces.insert(want.clone(), chosen::Arrival::Coming);
        let path = paths::font_face(&id, &want.family, want.weight, want.italic);
        let asked = browser::ask(&self.window, "GET", &path, None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let file = asked.await.ok().filter(|file| letters_fonts::is_font(file));
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            if shell.server.as_ref().is_none_or(|server| server.id != id) {
                return;
            }
            let arrival = match file {
                Some(file) => {
                    let name = chosen::asset_name(&file);
                    shell.view.add_named_fonts(vec![Font::new(file.as_slice().into(), 0)]);
                    shell.platform.store_asset(&name, &file);
                    shell.fonts.files.insert(name.clone(), chosen::Arrival::Here);
                    shell.fonts.arrived.push((want.clone(), name));
                    chosen::Arrival::Here
                }
                None => chosen::Arrival::Lost,
            };
            shell.fonts.faces.insert(want, arrival);
            shell.arrived();
        });
    }

    /// Fetches again the faces of a family that could not be fetched, as its notice offers.
    fn fetch_again(&mut self, family: &str) {
        self.fonts.faces.retain(|want, arrival| want.family != family || *arrival != chosen::Arrival::Lost);
        self.fonts.said.remove(&family.to_lowercase());
    }

    /// Fetches the files among `extras` that were never asked for. One that arrives is the window's from
    /// then on.
    fn fetch_extras(&mut self, extras: &[Extra]) {
        for extra in extras.iter().copied() {
            if self.extras.iter().any(|(asked, _)| *asked == extra) {
                continue;
            }
            self.extras.push((extra, Arrival::Coming));
            let (shell, window) = (self.me.clone(), self.window.clone());
            let (folder, file) = extra.file();
            let url = urls(&self.base, folder, &[&file]);
            wasm_bindgen_futures::spawn_local(async move {
                let fetched = browser::files(&window, &url).await.map(|files| files.into_iter().next());
                let Some(shell) = shell.upgrade() else { return };
                let mut shell = shell.borrow_mut();
                let arrival = match fetched {
                    Ok(Some(file)) => {
                        let file: Arc<[u8]> = file.into();
                        match extra {
                            Extra::Face(_) => _ = shell.view.add_fallback_fonts(vec![Font::new(file.clone(), 0)]),
                            Extra::Words(list) => _ = shell.view.add_word_list(list, &file),
                        }
                        Arrival::Here(file)
                    }
                    Ok(None) => Arrival::Lost,
                    Err(error) => {
                        web_sys::console::error_1(&error);
                        Arrival::Lost
                    }
                };
                if let Some((_, kept)) = shell.extras.iter_mut().find(|(asked, _)| *asked == extra) {
                    *kept = arrival;
                }
                shell.settle();
                shell.arrived();
            });
        }
    }

    /// Fetches the files among `extras` that were never asked for, and tells `then` whether any of `extras`
    /// is here once none is on its way. One that cannot be fetched is done without.
    fn back_up(&mut self, extras: Vec<Extra>, then: impl FnOnce(&mut Self, bool) + 'static) {
        self.fetch_extras(&extras);
        self.awaited.push((extras, Box::new(then)));
        self.settle();
    }

    /// Tells whoever waited for files of which none is on its way any more.
    fn settle(&mut self) {
        let coming = |extra: &Extra| self.extras.contains(&(*extra, Arrival::Coming));
        let (waiting, ready): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.awaited).into_iter().partition(|(extras, _)| extras.iter().any(&coming));
        self.awaited = waiting;
        for (extras, then) in ready {
            let arrived = self.fetched().any(|(extra, _)| extras.contains(&extra));
            then(self, arrived);
        }
    }

    /// Hands the exports the files a document needed, as far as they were fetched since it was last handed
    /// any.
    fn hand_backups(&mut self, exporter: &JsValue) {
        let new: Vec<(Extra, Array)> = self
            .fetched()
            .filter(|(extra, _)| !self.handed.contains(extra))
            .map(|(extra, file)| (extra, Array::of2(&extra.file().1.into(), &Uint8Array::from(&file[..]))))
            .collect();
        for (extra, asked) in new {
            match call(exporter, "backup", &asked) {
                Ok(_) => self.handed.push(extra),
                Err(error) => web_sys::console::error_1(&error),
            }
        }
    }

    /// Writes a copy of the document, as a file the browser keeps for the person, and answers the card
    /// that asked for it.
    fn copy(&mut self, name: String, format: CopyFormat, version: Option<&Revision>) {
        let source = match version.map(|version| self.view.document_at(version)) {
            // A clean copy starts a document of its own from what is written now, as a copy of a version
            // does from what was written then: neither the history nor the notes go with it.
            // So is the Letters copy of one a share link alone leads in.
            None if format == CopyFormat::Clean || (format == CopyFormat::Letters && self.by_link()) => {
                self.as_it_stands()
            }
            None => Source::Copy,
            Some(Some(version)) => Source::Version(Box::new(version)),
            Some(None) => return self.view.refuse_copy("That version could not be read.", false),
        };
        // Only a copy of the document as it is holds everything written up to when it was asked for.
        let whole = (format == CopyFormat::Letters && version.is_none()).then(|| self.view.edits());
        let format = match format {
            CopyFormat::Letters | CopyFormat::Clean => "let",
            CopyFormat::Word => "docx",
            CopyFormat::Pdf => "pdf",
            CopyFormat::WebPage => "html",
            CopyFormat::Markdown => "md",
        };
        self.export(format, source, name, move |shell, made| {
            let failed = |why: String| format!("The copy could not be written: {why}.");
            let refused = match made {
                Ok(Exported { name: None, .. }) => Some("A copy needs a name.".to_owned()),
                Ok(Exported { name: Some(name), kind, file }) => {
                    shell.download(&file, &kind, &name).err().map(|error| failed(said(&error)))
                }
                Err(why) => Some(failed(why)),
            };
            match refused {
                Some(notice) => shell.view.refuse_copy(&notice, false),
                None => {
                    shell.kept = whole.unwrap_or(shell.kept);
                    shell.view.set_copied("your downloads");
                }
            }
        });
    }

    // Saving and opening a file

    /// Keeps the document as its own file, which the browser hands to the person: there is nowhere else
    /// to write it until a server is behind the page. The file has the document's id and history, so the
    /// desktop takes it for a newer state of the file it was opened from.
    fn save(&mut self) {
        let written = self.view.edits();
        let title = self.window.document().map(|page| page.title()).unwrap_or_default();
        self.saving = true;
        let source = if self.by_link() { self.as_it_stands() } else { Source::Own };
        self.export("let", source, title, move |shell, made| {
            shell.saving = false;
            let saved = made.and_then(|file| {
                // A document whose name no file can have is kept all the same.
                let name = file.name.unwrap_or_else(|| format!("Untitled{DOCUMENTS}"));
                shell.download(&file.file, &file.kind, &name).map_err(|error| said(&error))
            });
            match saved {
                Ok(()) => {
                    shell.kept = written;
                    shell.view.set_saved_in("your downloads");
                }
                Err(why) => shell.say(format!("The document could not be saved: {why}.")),
            }
        });
    }

    /// Whether a share link alone leads the person to the document. What they take of it is then the
    /// document as it stands, and the window shows them none of its past (`page::served_on`). A courtesy:
    /// the history is on the page.
    fn by_link(&self) -> bool {
        self.server.as_ref().is_some_and(|server| server.by_link)
    }

    /// The document as it reads now, for a copy that starts a document of its own: neither the history
    /// nor the notes go with it.
    fn as_it_stands(&self) -> Source {
        Source::Version(Box::new((self.view.editor().doc().clone(), self.view.style().clone())))
    }

    /// The document the person chose after "Open" takes the place of the one on the page.
    fn chosen(&mut self, _: Event) {
        let file = self.opener.files().and_then(|files| files.get(0));
        // The same file chosen again is a change again.
        self.opener.set_value("");
        if let Some(file) = file {
            self.open_file(file);
        }
    }

    /// Opens a document's file in place of the document on the page, which has nowhere to be kept: the
    /// person is asked first when they wrote in it. What reads the file is fetched the first time.
    fn open_file(&mut self, file: File) {
        let name = file.name();
        let name = name.strip_suffix(DOCUMENTS).unwrap_or(&name).to_owned();
        if self.view.edits() != self.kept {
            let lost = format!(
                "Open \u{201c}{name}\u{201d} in place of this document? What was written here is not kept unless \
                 a copy of it was saved."
            );
            if !self.window.confirm_with_message(&lost).unwrap_or(false) {
                return;
            }
        }
        self.exporting += 1;
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let read = JsFuture::from(file.array_buffer()).await;
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            shell.exporting -= 1;
            let failed = |name: &str, why: String| format!("\u{201c}{name}\u{201d} could not be opened: {why}.");
            match read {
                Ok(read) => shell.ask("opened", Array::of1(&Uint8Array::new(&read)), move |shell, file| {
                    if let Err(why) = file.and_then(|file| shell.show_file(&name, &file)) {
                        shell.say(failed(&name, why));
                    }
                }),
                Err(error) => shell.say(failed(&name, said(&error))),
            }
            shell.arrived();
        });
    }

    /// Shows a file the exports read, called `name`, in place of the document on the page.
    fn show_file(&mut self, name: &str, file: &Array) -> Result<(), String> {
        let failed = |error: letters_model::JsonError| error.to_string();
        let text = |at: u32| file.get(at).as_string();
        let document = Document::from_json(&text(0).unwrap_or_default()).map_err(failed)?;
        // The built-in style stands in for one this build cannot read, as it does on the desktop.
        let style = text(1).map(|style| Style::from_json(&style)).transpose().map_err(failed)?.unwrap_or_default();
        let notes = text(2).map(|notes| Notes::from_json(&notes)).transpose().map_err(failed)?;
        let history = Some(file.get(3)).filter(|history| !history.is_undefined());
        let pictures: Map = file.get(4).unchecked_into();
        let identity: Array = file.get(5).unchecked_into();
        let identity = [0, 1].map(|at| identity.get(at).as_string().unwrap_or_default());

        // The page shows the file as it is. Its history takes the document in, so that a file changed
        // outside Letters is still the document.
        let replica = ReplicaId::new(&self.platform.new_id());
        let shown = || Model::new(document.clone(), replica).with_style(style.clone());
        let mut view = set_up(View::new(Editor::with_model(shown())), &mut self.platform, self.mac);
        let mut session = None;
        if let Some(history) = history {
            match shown().prepare(Some(&Uint8Array::new(&history).to_vec())) {
                Ok(history) => {
                    _ = view.change(|editor| Ok::<_, Infallible>(editor.attach(history)));
                    session = Some(view.editor().model().revision());
                }
                // A history that cannot be read starts over from the document.
                Err(error) => web_sys::console::error_1(&format!("{name}: {error}").into()),
            }
        }
        match notes {
            Some(notes) => view.set_notes(notes),
            None => view.refuse_comments(Some(page::NO_COMMENTS)),
        }
        view.set_document_name(name);
        if let Some(page) = self.window.document() {
            page.set_title(name);
        }
        self.platform.assets.clear();
        pictures.for_each(&mut |picture, name| {
            let picture = Uint8Array::new(&picture).to_vec();
            self.platform.assets.extend(name.as_string().map(|name| (name, picture)));
        });
        self.go_offline();
        (self.kept, self.identity, self.server) = (view.edits(), Some(identity), None);
        self.linked.clear();
        (self.session, self.written, self.rested) = (session, self.kept, None);
        self.save_state = SaveState::default();
        self.view = view;
        self.backed();
        self.text = TextInput::new(self.mac);
        self.touch = Touch::default();
        self.rest();
        self.show();
        Ok(())
    }

    /// Makes the PDF of the document, which is what Print view shows, and hands it to the browser's print
    /// dialog.
    fn print(&mut self) {
        // One dialog at a time: a second Print while the first is on its way is the same wish.
        if std::mem::replace(&mut self.printing, true) {
            return;
        }
        let title = self.window.document().map(|page| page.title()).unwrap_or_default();
        self.export("pdf", Source::Copy, title, |shell, made| {
            shell.printing = false;
            let printed = made.and_then(|pdf| shell.hand_to_print(&pdf.file).map_err(|error| said(&error)));
            if let Err(why) = printed {
                shell.say(format!("The document could not be printed: {why}."));
            }
        });
    }

    /// Tells the person what went wrong where the window has no place to say it, as the browser tells
    /// them things. It does so once this event is over, since nothing else happens while they read.
    fn say(&self, message: String) {
        let window = self.window.clone();
        let say = Closure::once_into_js(move || report(window.alert_with_message(&message)));
        report(self.window.set_timeout_with_callback(say.unchecked_ref()).map(drop));
    }

    /// Hands a PDF to the browser's print dialog, from a frame nobody sees. A browser that shows no PDFs
    /// in a page keeps the file instead, which is the next best thing to do with it.
    fn hand_to_print(&mut self, pdf: &JsValue) -> Result<(), JsValue> {
        let page = self.window.document().ok_or("there is no document")?;
        let options = BlobPropertyBag::new();
        options.set_type("application/pdf");
        let file = Blob::new_with_u8_array_sequence_and_options(&Array::of1(pdf), &options)?;
        let url = Url::create_object_url_with_blob(&file)?;
        if let Some((frame, last)) = self.printed.take() {
            frame.remove();
            Url::revoke_object_url(&last)?;
        }
        let frame: HtmlIFrameElement = page.create_element("iframe")?.dyn_into()?;
        for (name, value) in
            [("data-letters", "print"), ("aria-hidden", "true"), ("tabindex", "-1"), ("style", PRINTED)]
        {
            frame.set_attribute(name, value)?;
        }
        let (shown, kept) = (frame.clone(), url.clone());
        let print = Closure::once_into_js(move || {
            let printed =
                shown.content_window().ok_or_else(|| "the frame has no window".into()).and_then(|pdf| pdf.print());
            // A frame the page may not print from still holds a file the person can keep.
            report(printed.or_else(|_: JsValue| keep(&page, &kept, &format!("{}.pdf", page.title()))));
        });
        frame.set_onload(Some(print.unchecked_ref()));
        frame.set_src(&url);
        self.picker.insert_adjacent_element("afterend", &frame)?;
        self.printed = Some((frame, url));
        Ok(())
    }

    /// Brings a new view to the canvas.
    fn show(&mut self) {
        let active = self.window.document().and_then(|page| page.active_element());
        self.view.focus(active.is_some_and(|active| active == **self.input));
        self.fit();
        self.changed();
        self.sync();
    }

    /// Fits the view to the canvas, which may have changed size, display or theme.
    fn fit(&mut self) {
        let scale = self.window.device_pixel_ratio() as f32;
        let size = Size { width: self.canvas.client_width() as f32, height: self.canvas.client_height() as f32 };
        let (width, height) = ((size.width * scale).round() as u32, (size.height * scale).round() as u32);
        if (width, height) != (self.frame.width(), self.frame.height()) {
            // A canvas is cleared when it is resized, and a new frame is painted whole.
            self.canvas.set_width(width);
            self.canvas.set_height(height);
            self.frame = Frame::new(width, height);
        }
        let place = self.canvas.get_bounding_client_rect();
        let place =
            format!("left:{}px;top:{}px;width:{}px;height:{}px", place.left(), place.top(), size.width, size.height);
        report(self.holder.set_attribute("style", &format!("{MIRROR}{place}")));
        self.scale = scale;
        self.view.set_dark(self.dark());
        self.view.set_reduced_motion(self.less_motion.as_ref().is_some_and(MediaQueryList::matches));
        self.view.resize(size, scale);
    }

    /// Whether the window is dark: as the person set it, else as their system is.
    fn dark(&self) -> bool {
        match self.view.settings().appearance {
            Appearance::System => self.dark_mode.as_ref().is_some_and(MediaQueryList::matches),
            Appearance::Light => false,
            Appearance::Dark => true,
        }
    }

    /// Brings everything outside the canvas up to date with the view, after any event.
    fn sync(&mut self) {
        // During an edit the hidden input's selection is what the edit is aimed at, and no move of the caret.
        if !std::mem::take(&mut self.editing) {
            self.keyboard_moved();
        }
        // The browser's clock changes with the season and the place.
        let zone = zone();
        if zone != self.zone {
            self.zone = zone;
            self.view.set_utc_offset(zone);
        }
        let edits = self.view.edits();
        if edits != self.written {
            self.written = edits;
            self.wrote();
        }
        for request in self.view.take_requests() {
            match request {
                Request::Named(name) | Request::Rename(name) => self.rename(name),
                Request::SaveDefaultStyle(json) => {
                    if !self.platform.keep_default_style(&json) {
                        self.view.set_default_style(self.platform.default_style());
                    }
                    self.keep_settings();
                }
                // The browser asks the person for the file, and the chooser's `change` hands it over.
                Request::ImportStyle => self.chooser.click(),
                Request::ExportStyle(json) => {
                    let name = self.window.document().map(|page| page.title()).unwrap_or_default();
                    report(self.download(&json.into(), "application/json", &format!("{name}.letstyle")));
                }
                Request::Pictures => self.picker.click(),
                // A server's documents are listed on its own pages. Without one, the browser asks the person
                // for a file, and the chooser's `change` hands it over.
                Request::OpenDialog => match self.folder() {
                    Some((space, folder)) => self
                        .leave(space.map_or(paths::HOME_PAGE.to_owned(), |space| paths::folder_page(&space, &folder))),
                    None => self.opener.click(),
                },
                Request::New => self.start_document(),
                Request::Save => self.save(),
                Request::Paste { plain } => self.paste_later(plain),
                Request::Print => self.print(),
                Request::Asset(name) => self.load(name),
                Request::OpenLink { href, confirmed } => self.open_link(&href, confirmed),
                Request::TrustDocument => self.trust(),
                Request::LinkedPicture(src) => self.link(src, Linking::Open),
                Request::LoadPicture(src) => self.link(src, Linking::Once),
                Request::EmbedPicture { figure, src } => self.link(src, Linking::Embed(figure)),
                // The window asks for that host's pictures again by itself.
                Request::TrustPictures(src) => {
                    if let Ok(url) = HttpsUrl::parse(&src) {
                        let host = letters_trust::Source::host_of(&url);
                        self.tell_source(host.to_string(), true, false);
                        self.sources.push(host);
                    }
                }
                Request::TrustSource(source) => self.tell_source(source, true, true),
                Request::ForgetSource(source) => self.tell_source(source, false, false),
                Request::FetchPreview { url } => self.preview(url),
                Request::Connect => self.tell(|client, _, now| client.start(now)),
                // The window has taken the settings already. The rest is the shell's: keeping them, the
                // name edits carry, and light or dark.
                Request::Settings(settings) => {
                    self.platform.keep_settings(&settings);
                    follow(&mut self.view, settings);
                    self.view.set_dark(self.dark());
                    self.keep_settings();
                }
                // The window has dropped the word already, and the person's other tabs hear of it from the
                // browser's storage.
                Request::ForgetWord(word) => {
                    self.platform.forget_word(&word);
                    self.tell_word(word, false);
                }
                // A page has no folders to write in: a copy is a file the browser keeps for the person.
                Request::SaveCopy { name, format, .. } => self.copy(name, format, None),
                // A version that shows in the history is copied as it read then.
                Request::SaveVersion { revision, name, format, .. } => self.copy(name, format, Some(&revision)),
                asked @ (Request::Shares
                | Request::SharePeople(_)
                | Request::MakeLink(_)
                | Request::RevokeLink(_)
                | Request::AccessLog) => self.share(asked),
                Request::NoticePressed(key) => {
                    if let Some(family) = key.strip_prefix(FETCH_KEY) {
                        self.fetch_again(family);
                    }
                }
                // Moves and folders wait for the server's listings.
                _ => {}
            }
        }
        // The window offers to save what is nowhere but on the page.
        let state = match self.client.as_ref().filter(|_| self.attached).map(Client::progress) {
            // What is written on a served page is kept when the server holds it.
            Some(Progress::Saved) => SaveState::Saved,
            Some(Progress::Saving) => SaveState::Saving,
            Some(Progress::Edited) => SaveState::Edited,
            None if self.saving => SaveState::Saving,
            None if self.view.edits() == self.kept => SaveState::Saved,
            None => SaveState::Edited,
        };
        self.allow();
        let connection = self.connection();
        self.stats.live = self.attached && connection == Some(Connection::Live);
        if state != self.save_state || connection != self.connection {
            if state == SaveState::Saved && self.save_state != state {
                self.keep_seen(true);
            }
            self.save_state = state;
            self.view.set_save_state(state);
            self.connection.clone_from(&connection);
            self.view.set_connection(connection);
            // The server says so in its own time, after no key of the person's.
            self.changed();
        }
        self.keep_pictures();
        for word in mem::take(&mut self.platform.learned) {
            self.tell_word(word, true);
        }
        self.keep_up();
        self.go();
        self.cover();
        if self.stats.fonts {
            self.choose();
        }
        if (self.view.needs_paint() || self.touch.gliding()) && !self.frame_due {
            self.frame_due = self.window.request_animation_frame(&self.on_frame).is_ok();
        }
        self.rest();
        let caret = self.view.caret_rect();
        if let Some(at) = caret.filter(|_| caret != self.caret) {
            let canvas = self.canvas.get_bounding_client_rect();
            let (left, top) = (canvas.left() + f64::from(at.x), canvas.top() + f64::from(at.y));
            self.input.style().set_css_text(&format!("{HIDDEN}left:{left}px;top:{top}px;height:{}px", at.height));
        }
        self.caret = caret;
        self.place_handles();
        let cursor = match self.view.cursor() {
            // Writing a file takes a moment, and the first one the time to fetch what writes it.
            _ if self.exporting > 0 => "progress",
            Cursor::Default => "default",
            Cursor::Pointer => "pointer",
            Cursor::Text => "text",
            Cursor::Resize(edge) => match edge {
                Edge::North => "n-resize",
                Edge::NorthEast => "ne-resize",
                Edge::East => "e-resize",
                Edge::SouthEast => "se-resize",
                Edge::South => "s-resize",
                Edge::SouthWest => "sw-resize",
                Edge::West => "w-resize",
                Edge::NorthWest => "nw-resize",
            },
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            report(self.canvas.style().set_property("cursor", cursor));
        }
    }

    /// Paints one frame and puts what changed on the canvas.
    fn paint(&mut self) {
        self.frame_due = false;
        let start = self.platform.now();
        // What stood in for the window until now. The person may have read on in it while the editor was
        // on its way, and the editor opens where they are.
        let page = self.window.document().filter(|_| self.stats.first_frame_ms.is_none());
        let first = page.and_then(|page| page.get_element_by_id(page::FIRST_PAINT));
        if let Some(read) = first.as_ref().map(read_to).filter(|read| *read > 0.0) {
            let middle =
                Point { x: self.canvas.client_width() as f32 / 2.0, y: self.canvas.client_height() as f32 / 2.0 };
            self.view.scroll(middle, Point { x: 0.0, y: read as f32 }, &mut self.platform);
        }
        if let Some((at, by)) = self.touch.glide(start) {
            self.view.scroll(at, by, &mut self.platform);
            // Nothing moved, so the page is at its end.
            if !self.view.needs_paint() {
                self.touch.stop();
            }
        }
        let wake = self.view.animate(start as u64);
        self.list.items.clear();
        self.view.paint(&mut self.list);
        let ink = tokens::BG.resolve(if self.dark() { Scheme::Dark } else { Scheme::Light });
        let background = Color::rgba(ink.r, ink.g, ink.b, ink.a);
        let damage = self.renderer.render(&self.list, self.view.images(), self.scale, background, &mut self.frame);
        for rect in damage.rects() {
            report(put(&self.context, &self.frame, *rect, &mut self.strip));
        }
        let end = self.platform.now();
        self.stats.frames += 1;
        if self.stats.first_frame_ms.is_none() {
            self.stats.first_frame_ms = Some(end);
        }
        // The canvas shows the window now, so what stood in for it goes.
        if let Some(first) = first {
            first.remove();
        }
        if let Some(later) = self.later.take() {
            self.fetch(later);
            // The server may have said which dictionaries it has while the frame was on its way.
            self.want_dictionaries();
        }
        self.stats.frame_ms = end - start;
        self.wake_at(wake);
    }

    // The mirror

    /// Takes in what came after a wait: a file that was fetched or read, or an answer of the exports.
    /// Nothing the person did says that the window reads differently now, so the mirror is told here.
    fn arrived(&mut self) {
        self.changed();
        self.sync();
    }

    /// Says that the window may read differently, so the mirror is due once it has settled.
    fn changed(&mut self) {
        if let Some(timer) = self.mirror_timer.take() {
            self.window.clear_timeout_with_handle(timer);
        }
        let set = self.window.set_timeout_with_callback_and_timeout_and_arguments_0(&self.on_mirror, MIRROR_MS);
        self.mirror_timer = set.ok();
    }

    /// Brings the mirror up to date with the window.
    fn reflect(&mut self) {
        self.mirror_timer = None;
        // What was written may name a language for the first time.
        self.want_dictionaries();
        let start = self.platform.now();
        let mirror = Mirror::of(&self.view.access());
        // The hidden input keeps the keyboard, and says which control inside the window has it.
        report(match mirror.focus {
            Some(focus) => self.input.set_attribute("aria-activedescendant", &Mirror::name(focus)),
            None => self.input.remove_attribute("aria-activedescendant"),
        });
        report(self.mirror.show(mirror));
        self.stats.mirror_ms = self.platform.now() - start;
    }

    /// Does what a press in the mirror asks for. `nodes` are the pressed element's node and those around
    /// it, innermost first: the first that can be pressed takes it.
    fn act(&mut self, nodes: &[u64]) {
        let items = &self.mirror.shown().items;
        let Some((node, act)) = nodes.iter().find_map(|node| Some((*node, items.get(node)?.act?))) else { return };
        let request = match act {
            Act::Click => ActionRequest {
                action: Action::Click,
                target_tree: TreeId::ROOT,
                target_node: NodeId(node),
                data: None,
            },
            Act::Caret { page, run } => {
                let start = TextPosition { node: NodeId(run), character_index: 0 };
                ActionRequest {
                    action: Action::SetTextSelection,
                    target_tree: TreeId::ROOT,
                    target_node: NodeId(page),
                    data: Some(ActionData::SetTextSelection(TextSelection { anchor: start, focus: start })),
                }
            }
        };
        self.view.access_action(request, &mut self.platform);
        self.hand_over();
    }

    /// The document was written in. What it held when its file was opened, and where writing rested for
    /// long, is a version of its own. It is marked only now, as the desktop does: a mark is part of the
    /// history, and a file that was only read is not to change.
    fn wrote(&mut self) {
        let now = self.platform.wall_time().unwrap_or_default();
        let opened = self.session.take().map(|opened| (opened, TagKind::Session));
        let paused = self.rested.take_if(|(_, since)| now.saturating_sub(*since) >= REST);
        if let Some((before, kind)) = opened.or(paused.map(|(before, _)| (before, TagKind::Pause))) {
            self.mark(&before, kind);
        }
        self.rested = Some((self.view.editor().model().revision(), now));
    }

    /// Marks `revision` as a version. A mark is part of the history, and no edit.
    fn mark(&mut self, revision: &Revision, kind: TagKind) {
        let marked = self.view.change(|editor| editor.mark_version(revision, kind, None).map(|()| Change::default()));
        report(marked.map_err(|error| error.to_string().into()));
    }

    /// Lets the view move what moves on its own, when the time it asked for has come.
    fn wake(&mut self) {
        self.timer = None;
        let wake = self.view.animate(self.platform.now_ms());
        self.wake_at(wake);
    }

    fn wake_at(&mut self, wake: Option<u64>) {
        if let Some(timer) = self.timer.take() {
            self.window.clear_timeout_with_handle(timer);
        }
        let Some(at) = wake else { return };
        let delay = at.saturating_sub(self.platform.now_ms()).min(i32::MAX as u64) as i32;
        self.timer = self.window.set_timeout_with_callback_and_timeout_and_arguments_0(&self.on_wake, delay).ok();
    }

    // The keyboard

    /// Runs an event on the hidden input through the input rules, and does what they say.
    fn text(&mut self, dom: Dom<'_>, event: &Event) {
        match self.text.event(dom) {
            Reply::Pass => {}
            Reply::Drop => event.prevent_default(),
            Reply::Key { event: key, claim } => {
                if self.view.key(key, &mut self.platform) || claim {
                    event.prevent_default();
                }
                // A key on a menu's row copies as a press on it does.
                self.hand_over();
            }
            Reply::Text(text) => {
                self.view.input(text, &mut self.platform);
                event.prevent_default();
            }
            // Asked for only by `keyboard_moved`, which does the selecting.
            Reply::Select(_) => {}
        }
    }

    /// Puts the hidden input back to what it holds between edits: the text around the caret, with the
    /// selection where the editor's is. A keyboard that corrects, capitalises or predicts reads it there.
    fn rest(&mut self) {
        let around = self.around();
        // Code and formulas are written as they are meant, with no help from the keyboard. Nor is there
        // any where the input holds none of the sentence: every word would be taken for its first.
        let help = if around.is_some() { [("autocapitalize", "sentences"), ("autocorrect", "on")] } else { HELP };
        for (name, value) in help {
            if self.input.get_attribute(name).as_deref() != Some(value) {
                report(self.input.set_attribute(name, value));
            }
        }
        let Some((value, selection)) = self.text.hold(&around.unwrap_or_default()) else { return };
        // Writing to the input restarts a soft keyboard's session, so it is left alone when it can be.
        if self.input.value() != value {
            self.input.set_value(value);
        }
        let at = |end: Result<Option<u32>, JsValue>| end.ok().flatten();
        if (at(self.input.selection_start()), at(self.input.selection_end()))
            != (Some(selection.start), Some(selection.end))
        {
            report(self.input.set_selection_range(selection.start, selection.end));
        }
    }

    /// The text around the editor's selection, when the keyboard is with the page and the selection is in
    /// one block of prose.
    fn around(&self) -> Option<Around> {
        let editor = self.view.editor();
        let selection = editor.selection().filter(|_| self.view.types_in_canvas() && editor.picture().is_none())?;
        let (anchor, head) = (&selection.anchor, &selection.head);
        let block = editor.doc().block(&head.block).filter(|_| anchor.block == head.block)?;
        let range = anchor.offset.min(head.offset)..anchor.offset.max(head.offset);
        let mut end = 0;
        let source = block.body.runs().iter().any(|run| {
            let start = end;
            end += run.text.len();
            let written = run.marks.contains(&Mark::Code) || run.marks.contains(&Mark::Math);
            written && start <= range.end && range.start <= end
        });
        let prose = !matches!(block.kind, BlockKind::Code { .. } | BlockKind::Formula) && !source;
        prose.then(|| Around::of(&block.plain_text(), range))
    }

    fn key_down(&mut self, event: KeyboardEvent) {
        let ime = event.is_composing() || event.key_code() == 229;
        // The window takes the key only while there is something to keep. The browser's own answer to it
        // would save the page, and a file that was lost is asked for again with the same key.
        let primary = if self.mac { event.meta_key() } else { event.ctrl_key() };
        if primary && !ime && !event.shift_key() && !event.alt_key() && event.key().eq_ignore_ascii_case("s") {
            event.prevent_default();
            return self.save();
        }
        self.text(Dom::KeyDown { key: &event.key(), code: &event.code(), held: held!(event), ime }, &event);
    }

    fn before_input(&mut self, event: web_sys::InputEvent) {
        let (kind, data) = (event.input_type(), event.data());
        let (cancelable, selection) = (event.cancelable(), self.selected());
        self.text(Dom::BeforeInput { kind: &kind, data: data.as_deref(), cancelable, selection }, &event);
    }

    fn input(&mut self, event: Event) {
        self.text(Dom::Input { value: &self.input.value(), selection: self.selected() }, &event);
    }

    /// Follows a caret the keyboard moved in the hidden input, as a swipe along a soft keyboard's space bar
    /// does. The browser says so only later, and by then the shell may have put the selection back.
    fn keyboard_moved(&mut self) {
        let moved = self.text.event(Dom::Select { value: &self.input.value(), selection: self.selected() });
        let block = self.view.editor().selection().map(|selection| selection.head.block.clone());
        if let (Reply::Select(range), Some(block)) = (moved, block) {
            let (anchor, head) = (Pos::new(block.clone(), range.start), Pos::new(block, range.end));
            self.view.select(Selection { anchor, head });
        }
    }

    fn composition_end(&mut self, event: CompositionEvent) {
        let (data, value) = (event.data().unwrap_or_default(), self.input.value());
        self.text(Dom::CompositionEnd { data: &data, value: &value }, &event);
    }

    /// The selection in the hidden input, in UTF-16 units.
    fn selected(&self) -> Range<u32> {
        let end = |end: Result<Option<u32>, JsValue>| end.ok().flatten().unwrap_or(0);
        end(self.input.selection_start())..end(self.input.selection_end())
    }

    /// Carries a clipboard event out through the editor's own copy, cut and paste, which read and write the
    /// clipboard through the platform while the event lasts.
    fn clipboard(&mut self, event: ClipboardEvent, action: Clipboard) {
        let Some(data) = event.clipboard_data().filter(|_| !self.text.composing()) else { return };
        event.prevent_default();
        let key = self.text.clipboard_key(action);
        if action == Clipboard::Paste {
            let read = |kind| data.get_data(kind).ok().filter(|content| !content.is_empty());
            let (text, html) = (read("text/plain"), read("text/html"));
            let files = files(data.files());
            // Pictures and no words, as a screenshot is, or picture files copied where they are kept. A
            // picture copied from a page comes with HTML that only points at it.
            if text.is_none() && !files.is_empty() {
                let count = files.len();
                return self.pictures(files, move |shell, pictures| {
                    let pictures = if pictures.len() == count { pictures } else { Vec::new() };
                    if !pictures.is_empty() || html.is_some() {
                        let html = html.filter(|_| pictures.is_empty());
                        shell.paste(key, Clip { html, pictures, ..Clip::default() });
                    }
                });
            }
            if text.is_some() || html.is_some() {
                self.paste(key, Clip { text: text.unwrap_or_default(), html, ..Clip::default() });
            }
        } else {
            self.platform.copied = None;
            self.view.key(key, &mut self.platform);
            let Some(clip) = self.platform.copied.take() else { return };
            report(data.set_data("text/plain", &clip.text));
            if let Some(html) = clip.html {
                report(data.set_data("text/html", &html));
            }
        }
    }

    /// Has the editor paste `clip`, which it reads from the clipboard while it handles the key.
    fn paste(&mut self, key: KeyEvent, clip: Clip) {
        self.platform.pasted = Some(clip);
        self.view.key(key, &mut self.platform);
        self.platform.pasted = None;
    }

    /// Reads the clipboard for a paste that no paste event carries, as one asked from a menu is, and has
    /// the editor paste what it holds once the browser hands it over.
    fn paste_later(&self, plain: bool) {
        let (shell, window) = (self.me.clone(), self.window.clone());
        let mods = Modifiers { primary: true, shift: plain, alt: false };
        wasm_bindgen_futures::spawn_local(async move {
            let Some(clip) = browser::paste(&window).await else { return };
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            shell.paste(KeyEvent { key: Key::Char('v'), mods }, clip);
            shell.arrived();
        });
    }

    // Pictures

    /// Reads `files`, which takes a moment, and hands `then` those that are pictures Letters can show.
    fn pictures(&self, files: Vec<File>, then: impl FnOnce(&mut Self, Vec<Picture>) + 'static) {
        let (shell, window) = (self.me.clone(), self.window.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let mut pictures = Vec::new();
            for file in &files {
                pictures.extend(browser::picture(&window, file).await);
            }
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            then(&mut shell, pictures);
            shell.arrived();
        });
    }

    /// Decodes a picture the page has come to show, which takes a moment, and hands it to the window.
    fn load(&mut self, name: String) {
        // What was put in here is here. The rest is the server's to give, where there is one.
        let held = self.platform.assets.get(&name).cloned();
        let asked = self
            .server
            .as_ref()
            .filter(|_| held.is_none())
            .map(|server| (server.id.clone(), browser::ask(&self.window, "GET", &asset(&server.id, &name), None)));
        if held.is_none() && asked.is_none() {
            return self.view.set_image(&name, None);
        }
        let (shell, window) = (self.me.clone(), self.window.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let (bytes, from) = match (held, asked) {
                (None, Some((id, asked))) => (asked.await.ok(), Some(id)),
                (held, _) => (held, None),
            };
            let image = match &bytes {
                Some(bytes) => browser::decode(&window, bytes).await,
                None => None,
            };
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // A copy of the document takes its pictures along. The page may show another document by now.
            if let (Some(bytes), Some(id)) = (bytes, from)
                && shell.server.as_ref().is_some_and(|server| server.id == id)
            {
                shell.platform.assets.entry(name.clone()).or_insert(bytes);
            }
            shell.view.set_image(&name, image);
            shell.arrived();
        });
    }

    /// Hands the server the pictures the editor put into its document since the last look, so that
    /// they are there before the edits that name them.
    fn keep_pictures(&mut self) {
        let stored = std::mem::take(&mut self.platform.stored);
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else { return };
        for name in stored {
            let Some(bytes) = self.platform.assets.get(&name) else { continue };
            let kind = browser::mime(bytes).unwrap_or("application/octet-stream");
            let sent = browser::ask(&self.window, "PUT", &asset(&id, &name), Some((kind, bytes)));
            self.uploading += 1;
            let shell = self.me.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let kept = sent.await;
                let Some(shell) = shell.upgrade() else { return };
                let mut shell = shell.borrow_mut();
                shell.uploading -= 1;
                if let Err(refusal) = kept {
                    // The picture stays on the page, and the person knows that it is nowhere else.
                    shell.say(refusal.message);
                }
                // The edit that names it may go now.
                shell.sync();
            });
        }
    }

    // The server's documents

    /// The space and the folder the served document is in. `None` on a page of no server, and no space
    /// for a document that reaches the person only through a share.
    fn folder(&self) -> Option<(Option<String>, String)> {
        let server = self.server.as_ref()?;
        let folder = server.path.rsplit_once('/').map_or("", |(folder, _)| folder);
        Some((server.space.clone(), folder.to_owned()))
    }

    /// Calls the document `name`. A server's document is renamed by its server, which may refuse.
    fn rename(&mut self, name: String) {
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else {
            // There are no folders to move between, so a name is the person's to give.
            return self.called(&name);
        };
        let body = serde_json::to_vec(&letters_api::Rename { name }).unwrap_or_default();
        let asked = browser::ask(&self.window, "POST", &paths::rename(&id), Some((letters_api::JSON, &body)));
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let answer = asked.await;
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // The page may show another document by now.
            if shell.server.as_ref().is_none_or(|server| server.id != id) {
                return;
            }
            match answer.map(|entry| serde_json::from_slice::<letters_api::Entry>(&entry)) {
                Ok(Ok(entry)) => {
                    shell.called(&entry.name);
                    if let Some(server) = &mut shell.server {
                        (server.name, server.path, server.named) = (entry.name, entry.path, true);
                    }
                }
                Ok(Err(_)) => shell.view.set_notice(Some("The server's answer could not be read.")),
                Err(refusal) => shell.view.set_notice(Some(&refusal.message)),
            }
            shell.arrived();
        });
    }

    /// Asks the document's server what the Share dialog asked, and tells the window the answer: the shares
    /// as they then are, the link that was made, who reached the document, or the server's sentence for
    /// what it would not do.
    fn share(&mut self, asked: Request) {
        const UNREAD: &str = "The server's answer could not be read.";
        let Some(server) = &self.server else { return };
        let (id, owner) = (server.id.clone(), server.role.shares());
        let (method, path, body) = match &asked {
            Request::Shares => ("GET", paths::shares(&id), None),
            // The people are put as the shares are read. The links are not a put's to change.
            Request::SharePeople(people) => {
                let shares = Shares { people: people.clone(), links: Vec::new() };
                ("PUT", paths::shares(&id), serde_json::to_vec(&shares).ok())
            }
            Request::MakeLink(link) => ("POST", paths::links(&id), serde_json::to_vec(link).ok()),
            Request::RevokeLink(link) => ("DELETE", paths::link(&id, link), None),
            Request::AccessLog => ("GET", paths::access(&id), None),
            _ => return,
        };
        // Whether the server makes links is asked once, with the first of these.
        let about = self.links.is_none().then(|| browser::ask(&self.window, "GET", paths::SERVER, None));
        let body = body.as_deref().map(|body| (letters_api::JSON, body));
        let sent = browser::ask(&self.window, method, &path, body);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let about = match about {
                Some(about) => {
                    about.await.ok().and_then(|about| serde_json::from_slice::<letters_api::Server>(&about).ok())
                }
                None => None,
            };
            let answer = sent.await;
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // The page may show another document by now.
            if shell.server.as_ref().is_none_or(|server| server.id != id) {
                return;
            }
            shell.links = shell.links.or(about.map(|server| server.links));
            // A server that does not say makes none.
            let links = shell.links.unwrap_or(false);
            match (asked, answer) {
                (_, Err(refusal)) => shell.view.refuse_share(&refusal.message),
                (Request::Shares | Request::SharePeople(_), Ok(answer)) => match serde_json::from_slice(&answer) {
                    Ok(shares) => shell.view.set_shares(Shared { shares, owner, links }),
                    Err(_) => shell.view.refuse_share(UNREAD),
                },
                (Request::MakeLink(_), Ok(answer)) => match serde_json::from_slice(&answer) {
                    Ok(made) => shell.view.link_made(made),
                    Err(_) => shell.view.refuse_share(UNREAD),
                },
                (Request::AccessLog, Ok(answer)) => match serde_json::from_slice(&answer) {
                    Ok(log) => shell.view.set_access_log(log),
                    Err(_) => shell.view.refuse_share(UNREAD),
                },
                // The dialog dropped a revoked link already.
                _ => {}
            }
            shell.arrived();
        });
    }

    /// Asks the server what the document is called and where it is now: nothing tells an open page that
    /// someone else renamed or moved it.
    fn look(&mut self) {
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else { return };
        let asked = browser::ask(&self.window, "GET", &paths::document(&id), None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(Ok(now)) = asked.await.map(|answer| serde_json::from_slice::<Opened>(&answer)) else { return };
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            let Some(server) = shell.server.as_mut().filter(|server| server.id == id) else { return };
            let renamed = server.name != now.name;
            (server.space, server.path, server.named) = (now.space, now.path, now.named);
            if renamed {
                server.name.clone_from(&now.name);
                shell.called(&now.name);
                shell.arrived();
            }
        });
    }

    /// Shows the document under `name`, in the window and on the browser's tab.
    fn called(&mut self, name: &str) {
        self.view.set_document_name(name);
        if let Some(page) = self.window.document() {
            page.set_title(name);
        }
    }

    /// Starts another document beside the served one and goes to it. A page of no server has nowhere to
    /// keep a second one.
    fn start_document(&mut self) {
        let Some((space, folder)) = self.folder() else { return };
        // A document shared from elsewhere has no folder of the person's: they choose one on the server's pages.
        let Some(space) = space else { return self.leave(paths::HOME_PAGE.to_owned()) };
        let body = serde_json::to_vec(&letters_api::NewDocument { folder, ..Default::default() }).unwrap_or_default();
        let asked = browser::ask(&self.window, "POST", &paths::documents(&space), Some((letters_api::JSON, &body)));
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let answer = asked.await;
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            let made = answer
                .map(|entry| serde_json::from_slice::<letters_api::Entry>(&entry).ok().and_then(|entry| entry.id));
            match made {
                Ok(Some(id)) => shell.leave(paths::editor_page(&id)),
                Ok(None) => shell.say("The server's answer could not be read.".to_owned()),
                Err(refusal) => shell.say(refusal.message),
            }
            shell.sync();
        });
    }

    /// Goes to another of the server's pages, once the server has what the person wrote on this one.
    fn leave(&mut self, to: String) {
        self.leaving = Some(to);
    }

    /// Whether the page holds work its server lacks.
    fn unsent(&self) -> bool {
        let saved = self.client.as_ref().is_none_or(|client| !self.attached || client.progress() == Progress::Saved);
        self.uploading > 0 || !saved
    }

    /// Leaves for the page the person asked for, if nothing written here would be lost with this one.
    fn go(&mut self) {
        if self.leaving.is_none() || self.uploading > 0 {
            return;
        }
        match self.client.as_ref().filter(|_| self.attached).map(Client::progress) {
            // It is on its way, and the page goes once it is there.
            Some(Progress::Saving) => {}
            Some(Progress::Edited) => {
                self.leaving = None;
                self.say("What you wrote is not with the server yet. Try again once the bar says Saved.".to_owned());
            }
            _ => {
                if let Some(to) = self.leaving.take() {
                    report(self.window.location().assign(&to));
                }
            }
        }
    }

    // The person's settings

    /// Asks the server what the person set for themselves, on whatever device. The page starts with
    /// what this browser kept, so that it does not change its look once it shows.
    fn ask_settings(&mut self) {
        if self.server.is_none() {
            return;
        }
        let asked = browser::ask(&self.window, "GET", paths::SETTINGS, None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let settings = asked.await.map(|answer| serde_json::from_slice(&answer));
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // A server that does not say leaves the person with what this browser kept.
            if let Ok(Ok(settings)) = settings {
                shell.take_settings(settings);
            }
            shell.named = true;
            shell.say_seen();
            shell.arrived();
        });
        // The dictionaries are anyone's to ask for. A server that has none leaves every language but
        // English unchecked, as a page with no server does.
        let asked = browser::ask(&self.window, "GET", paths::DICTIONARIES, None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(Ok(languages)) = asked.await.map(|answer| serde_json::from_slice::<Vec<String>>(&answer)) else {
                return;
            };
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            shell.view.set_languages(&languages);
            shell.languages = languages;
            shell.want_dictionaries();
            shell.arrived();
        });
        // A visitor has no sources of their own, and their settings then list none.
        let asked = browser::ask(&self.window, "GET", paths::OWN_SOURCES, None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Ok(Ok(sources)) = asked.await.map(|answer| serde_json::from_slice(&answer)) else { return };
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            shell.take_sources(sources);
            shell.arrived();
        });
    }

    /// Fetches the server's dictionary for each language the document names that it has one for, once.
    /// A dictionary is large, so none is fetched for a language nothing is written in, and none before
    /// the first frame, which must not wait for one. The text is checked when it has come, and left alone
    /// if it does not.
    fn want_dictionaries(&mut self) {
        if self.languages.is_empty() || self.stats.first_frame_ms.is_none() {
            return;
        }
        // By the rule every shell picks a server's dictionaries with.
        let named = page::languages(self.view.editor().doc());
        let mut wanted = letters_spell::served_dictionaries(named.iter().map(String::as_str), &self.languages);
        wanted.retain(|held, _| !self.dictionaries.contains(held));
        for (held, tag) in wanted {
            self.dictionaries.push(held.clone());
            let urls = [paths::dictionary_aff(&held), paths::dictionary_dic(&held)];
            let (shell, window) = (self.me.clone(), self.window.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let files = match browser::files(&window, &urls).await {
                    Ok(files) => files,
                    Err(error) => return web_sys::console::error_1(&error),
                };
                let [aff, dic] = [0, 1].map(|at| String::from_utf8_lossy(&files[at]));
                let dictionary = match letters_spell::Dictionary::parse(&aff, &dic) {
                    Ok(dictionary) => dictionary,
                    Err(error) => {
                        return web_sys::console::error_1(&format!("the dictionary for {tag}: {error}").into());
                    }
                };
                let Some(shell) = shell.upgrade() else { return };
                let mut shell = shell.borrow_mut();
                shell.view.add_dictionary(&tag, dictionary);
                // The check runs in the time a frame leaves over, so there must be a frame.
                if !shell.frame_due {
                    shell.frame_due = shell.window.request_animation_frame(&shell.on_frame).is_ok();
                }
                shell.arrived();
            });
        }
    }

    /// Takes the sources of linked pictures the person's account keeps, as the server writes them.
    fn take_sources(&mut self, sources: Vec<String>) {
        self.sources = sources.iter().filter_map(|source| letters_trust::Source::parse(source).ok()).collect();
        let listed = sources.into_iter().map(|source| TrustedSource { source, fixed: false }).collect();
        self.view.set_trusted_sources(listed);
    }

    /// Has the person's account keep a source of linked pictures, or let one go. The server answers all
    /// of them, and says in a sentence why something is no source: `typed` is whether the person wrote it
    /// in their settings, where the sentence shows under the field. Without an account nothing is kept.
    fn tell_source(&mut self, source: String, add: bool, typed: bool) {
        if self.account.is_none() {
            return;
        }
        let sent = browser::ask(&self.window, if add { "PUT" } else { "DELETE" }, &paths::own_source(&source), None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let answer = sent.await.map(|sources| serde_json::from_slice(&sources));
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            match answer {
                Ok(Ok(sources)) => shell.take_sources(sources),
                Ok(Err(_)) => {}
                Err(refusal) if typed => shell.view.refuse_source(&refusal.message),
                Err(refusal) => shell.say(refusal.message),
            }
            shell.arrived();
        });
    }

    /// Takes what the server keeps for the person, in place of what this browser kept. A default style
    /// this browser has and the server lacks is theirs too, and the server is told.
    fn take_settings(&mut self, theirs: letters_api::Settings) {
        let settings = Settings {
            appearance: match theirs.appearance {
                Some(letters_api::Appearance::Light) => Appearance::Light,
                Some(letters_api::Appearance::Dark) => Appearance::Dark,
                None => Appearance::System,
            },
            accent: theirs.accent,
            name: theirs.name.clone(),
            language: theirs.language.clone(),
        };
        self.platform.keep_settings(&settings);
        // The account's words are all there are, so that one taken out elsewhere is not brought back here.
        // It is still to be told those this browser took when it could tell no account.
        let mut alone = self.platform.unsent_words();
        alone.retain(|word| !theirs.dictionary.contains(word));
        self.platform.keep_unsent_words(&alone);
        let mut words = theirs.dictionary.clone();
        words.extend(alone.iter().cloned());
        self.platform.keep_user_words(&words);
        if let Some(style) = &theirs.default_style {
            self.platform.keep_default_style(&style.to_string());
        }
        self.view.set_user_words(words);
        self.view.set_default_style(self.platform.default_style());
        // Without a name of their own choosing, the person is who the server says they are.
        if let (None, Some(server)) = (&settings.name, &self.server) {
            self.view.set_author(&server.author);
        }
        follow(&mut self.view, settings);
        self.fit();
        self.account = Some(theirs);
        self.keep_settings();
        alone.into_iter().for_each(|word| self.tell_word(word, true));
    }

    /// Has the person's account keep a word they added to their own, or take out one they removed. The
    /// server takes a word at a time and answers all of them, which are then the words this browser has:
    /// two devices that change them at once both have their way. Without an account the browser keeps
    /// the word, for the account that next answers.
    fn tell_word(&mut self, word: String, add: bool) {
        if self.account.is_none() {
            return;
        }
        let sent = browser::ask(&self.window, if add { "PUT" } else { "DELETE" }, &paths::word(&word), None);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let answer = sent.await.map(|words| serde_json::from_slice::<Vec<String>>(&words));
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            let mut unsent = shell.platform.unsent_words();
            match answer {
                Ok(Ok(mut words)) => {
                    unsent.retain(|unsent| *unsent != word && !words.contains(unsent));
                    if let Some(account) = &mut shell.account {
                        account.dictionary.clone_from(&words);
                    }
                    // An answer may overtake that for a word sent after it.
                    words.extend(unsent.iter().cloned());
                    shell.platform.keep_user_words(&words);
                    shell.view.set_user_words(words);
                }
                Ok(Err(_)) => {}
                Err(refusal) => {
                    // A word the server will not have is not offered to it again. One it could not be
                    // asked for is, when the page next opens.
                    if refusal.code == Code::Invalid {
                        unsent.retain(|unsent| *unsent != word);
                    }
                    shell.say(refusal.message);
                }
            }
            shell.platform.keep_unsent_words(&unsent);
            shell.arrived();
        });
    }

    /// Has the server keep what the person set, where it differs from what the server has. The server
    /// keeps the whole as it is sent.
    fn keep_settings(&mut self) {
        let Some(kept) = &self.account else { return };
        let set = self.view.settings();
        let ours = letters_api::Settings {
            name: set.name.clone(),
            language: set.language.clone(),
            appearance: match set.appearance {
                Appearance::Light => Some(letters_api::Appearance::Light),
                Appearance::Dark => Some(letters_api::Appearance::Dark),
                Appearance::System => None,
            },
            accent: set.accent,
            // The words go a word at a time (`tell_word`): the server takes none from here.
            dictionary: kept.dictionary.clone(),
            default_style: self.platform.kept_default_style().or_else(|| kept.default_style.clone()),
        };
        if ours == *kept {
            return;
        }
        let Ok(body) = serde_json::to_vec(&ours) else { return };
        let sent = browser::ask(&self.window, "PUT", paths::SETTINGS, Some((letters_api::JSON, &body)));
        self.account = Some(ours);
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let Err(refusal) = sent.await else { return };
            let Some(shell) = shell.upgrade() else { return };
            // The settings hold in this browser, and the person knows that they are nowhere else.
            shell.borrow().say(refusal.message);
        });
    }

    // The line to the server

    /// Starts the served document's line to its server. The socket is asked for at once, beside the
    /// fonts, and the document's history comes with its first session.
    fn go_online(&mut self) {
        let Some(server) = &self.server else { return };
        let mut client = Client::new(server.id.clone());
        let now = self.platform.now_ms();
        client.start(now);
        self.client = Some(client);
        self.quiet_until = Some(now + QUIET_MS);
        self.role = match server.role {
            letters_api::Role::Reader => Role::Reader,
            letters_api::Role::Commenter => Role::Commenter,
            letters_api::Role::Editor | letters_api::Role::Owner => Role::Editor,
        };
        self.allow();
        self.pump();
    }

    /// Ends the line, as when the page comes to show a document that is not the server's.
    fn go_offline(&mut self) {
        if let Some(mut client) = self.client.take() {
            client.stop();
        }
        self.hang_up();
        // The window that shows the other document is a new one.
        (self.attached, self.told, self.quiet_until, self.peers_at) = (false, None, None, None);
        (self.access, self.held) = (Access::Edits, false);
        self.beat_at();
    }

    /// Tells the client what happened, and does what it then asks.
    fn tell(&mut self, tell: impl FnOnce(&mut Client, Local, u64)) {
        let now = self.platform.now_ms();
        if let Some(client) = &mut self.client {
            tell(client, local(&self.view, self.attached), now);
        }
        self.pump();
    }

    /// Does what the client asks, in the order it asks.
    fn pump(&mut self) {
        while let Some(asked) = self.client.as_mut().and_then(Client::poll) {
            match asked {
                letters_sync::Event::Open => self.connect(),
                letters_sync::Event::Send(frame) => {
                    if let Some(line) = &self.socket {
                        report(line.socket.send_with_u8_array(&frame));
                    }
                }
                letters_sync::Event::Close => self.hang_up(),
                letters_sync::Event::Merge(history) => self.take_edits(&history),
                letters_sync::Event::Notes(theirs) => self.take_notes(&theirs),
                // The page keeps no journal that could let go of what the server holds.
                letters_sync::Event::Held(_) => {}
                // The session goes on without what was refused, and the person knows what is not kept.
                letters_sync::Event::Refused(why) => self.say(why.to_string()),
                letters_sync::Event::Peers => self.peers_at = None,
                // The bar is told how the link and saving stand with every look, in `sync`.
                letters_sync::Event::Link => {
                    if let Some(Link::Live(role)) = self.client.as_ref().map(Client::link) {
                        (self.role, self.quiet_until) = (role, None);
                        // Back after a while without the server, the document may be called otherwise.
                        if self.sockets > 1 {
                            self.look();
                        }
                    }
                }
                letters_sync::Event::Progress => {}
            }
        }
        self.beat_at();
    }

    /// Opens the socket the client asked for.
    fn connect(&mut self) {
        self.hang_up();
        self.sockets += 1;
        let line = self.server.as_ref().and_then(|server| line(&self.window, &server.socket, &self.me, self.sockets));
        match line {
            Some(line) => self.socket = Some(line),
            None => {
                let now = self.platform.now_ms();
                self.client.iter_mut().for_each(|client| client.closed(now));
            }
        }
    }

    /// Closes the socket. The client is told nothing more of it.
    fn hang_up(&mut self) {
        if let Some(line) = self.socket.take() {
            report(line.socket.close());
            self.hung = Some(line);
        }
    }

    /// Takes in what was written elsewhere. The first of it is the document's whole history.
    fn take_edits(&mut self, history: &[u8]) {
        let taken = match self.attached {
            true => self.view.change(|editor| editor.merge(history)).map_err(|error| error.to_string()),
            false => self.take_history(history),
        };
        match taken {
            Ok(()) => self.changed(),
            Err(error) => web_sys::console::error_1(&format!("what the server sent cannot be merged: {error}").into()),
        }
        // The text may have moved under the others' carets.
        self.peers_at = None;
        let now = self.platform.now_ms();
        if let Some(client) = &mut self.client {
            client.changed(local(&self.view, self.attached), now);
        }
    }

    /// Makes the server's history the document's. A page is sent the document without it, so that it
    /// reads at once.
    fn take_history(&mut self, history: &[u8]) -> Result<(), String> {
        let replica = ReplicaId::new(&self.platform.new_id());
        let held = Model::load(history, replica).map_err(|error| error.to_string())?;
        if held.doc() == self.view.editor().doc() && held.style() == self.view.style() {
            // The page shows what the server holds, so the window stays as the person has it.
            let held = held.prepare(None).map_err(|error| error.to_string())?;
            _ = self.view.change(|editor| Ok::<_, Infallible>(editor.attach(held)));
        } else {
            // The document was changed after the page was written, and the server's is the document.
            self.show_held(held);
        }
        self.attached = true;
        self.say_seen();
        Ok(())
    }

    /// Tells the window what the person last saw of the document, as their account keeps it, once: when
    /// the server's history is the document's and the person has their name, by which the window knows
    /// what others wrote since.
    fn say_seen(&mut self) {
        let ready = self.attached && self.named && !self.seen;
        let Some(server) = self.server.as_ref().filter(|_| ready) else { return };
        self.seen = true;
        self.seen_told.0.clone_from(&server.last_seen);
        let seen = server.last_seen.as_ref().and_then(|seen| seen.parse().ok());
        self.view.set_last_seen(seen);
    }

    /// Has the person's account keep how the document reads now as what they last saw of it: when the
    /// page goes, and `saved` when what they wrote is the server's, so that a page that dies has lost
    /// little. A save soon after another is left to the next. Not before the window was told what the
    /// account kept the last time.
    fn keep_seen(&mut self, saved: bool) {
        let Some(server) = self.server.as_ref().filter(|_| self.attached && self.seen) else { return };
        let (revision, now) = (self.view.editor().model().revision().to_string(), self.platform.now_ms());
        let (told, at) = &self.seen_told;
        let soon = at.is_some_and(|at| now.saturating_sub(at) < SEEN_APART_MS);
        // A document nothing was ever written in reads at no revision, which is nothing to keep.
        if revision.is_empty() || told.as_ref() == Some(&revision) || (saved && soon) {
            return;
        }
        let seen = serde_json::to_vec(&Seen { revision: revision.clone() }).expect("a revision is JSON");
        browser::tell(&self.window, "PUT", &paths::seen(&server.id), (letters_api::JSON, &seen));
        self.seen_told = (Some(revision), Some(now));
    }

    /// Tells the window what the person may do with the document: what their role allows, and nothing
    /// but reading until the server's history is the document's, since what is written before would have
    /// no place in it.
    fn allow(&mut self) {
        let waits = self.client.is_some() && !self.attached;
        let access = match self.role {
            _ if self.client.is_none() => Access::Edits,
            // A wait the person notices is told by the bar. A shorter one changes nothing they see.
            _ if waits && self.quiet_until.is_none() => Access::Reads,
            Role::Editor => Access::Edits,
            Role::Commenter => Access::Comments,
            Role::Reader => Access::Reads,
        };
        if access != self.access {
            self.access = access;
            self.view.set_access(access);
        }
        // For that shorter wait the window offers its tools, and the editor writes nothing.
        let held = waits && access == Access::Edits;
        if held != self.held {
            self.held = held;
            if access == Access::Edits {
                _ = self.view.change(|editor| {
                    editor.set_read_only(held);
                    Ok::<_, Infallible>(Change::default())
                });
            }
        }
    }

    /// Shows the served document as the server's history holds it, in place of what the page was sent.
    fn show_held(&mut self, held: Model) {
        let Some(server) = &self.server else { return };
        let mut view = set_up(page::served_on(held, server), &mut self.platform, self.mac);
        view.set_author(&server.author);
        view.set_preview_fetching(true);
        // The notes and the caret stay where the person has them, as far as the document still has the place.
        view.set_notes(self.view.notes().clone());
        if let Some(selection) = self.view.editor().selection() {
            view.select(selection.clone());
        }
        self.view = view;
        self.backed();
        // The new window is told of the link and the others anew, and lets the person write until told.
        (self.connection, self.peers_at, self.access, self.held) = (None, None, Access::Edits, false);
        (self.kept, self.written, self.rested, self.session) = (self.view.edits(), self.view.edits(), None, None);
        self.text = TextInput::new(self.mac);
        self.touch = Touch::default();
        self.rest();
        self.show();
    }

    /// Joins in the notes that were added or changed elsewhere.
    fn take_notes(&mut self, theirs: &Notes) {
        let mut notes = self.view.notes().clone();
        if notes.merge(theirs) {
            notes.follow(self.view.editor().model());
            self.view.set_notes(notes);
            self.changed();
        }
        let now = self.platform.now_ms();
        if let Some(client) = &mut self.client {
            client.changed(local(&self.view, self.attached), now);
        }
    }

    /// Tells the client what the person did since the last look: what they wrote, and where their caret is.
    fn keep_up(&mut self) {
        let Some(client) = self.client.as_mut().filter(|_| self.attached) else { return };
        let now = self.platform.now_ms();
        // A picture is the server's before the edit that names it is.
        if self.uploading == 0 {
            client.changed(local(&self.view, true), now);
        }
        let selection = self.view.selection();
        if selection != self.told.as_ref() {
            client.selected(self.view.editor().model(), selection, now);
            self.told = selection.cloned();
        }
        self.pump();
        if self.peers_at != Some(self.view.edits()) {
            self.peers_at = Some(self.view.edits());
            self.show_peers();
        }
    }

    /// Shows the others' carets where the document has them now.
    fn show_peers(&mut self) {
        let Some(client) = &self.client else { return };
        let model = self.view.editor().model();
        let peer = |peer: &letters_sync::Peer| Peer {
            session: peer.session.0.into(),
            name: peer.name.clone(),
            colour: peer.colour,
            selection: peer.selection.as_ref().and_then(|passage| passage.selection(model)),
        };
        let peers = client.peers().iter().map(peer).collect();
        self.view.set_peers(peers);
        self.changed();
    }

    /// How the page stands to its server, as the window is to say it. `None` for a document that is not
    /// the server's, and for the first moments of a page: a wait that short shows nothing.
    fn connection(&self) -> Option<Connection> {
        match self.client.as_ref()?.link() {
            Link::Live(_) => Some(Connection::Live),
            Link::Refused(why) => Some(Connection::Refused(why.to_string())),
            _ if self.quiet_until.is_some() => None,
            Link::Connecting => Some(Connection::Connecting),
            Link::Offline => Some(Connection::Offline),
        }
    }

    /// The time the client asked to be woken at has come.
    fn beat(&mut self) {
        self.beat_timer = None;
        let now = self.platform.now_ms();
        self.quiet_until = self.quiet_until.filter(|&until| until > now);
        self.tell(|client, _, now| client.tick(now));
        self.sync();
    }

    /// Sets the timer for the client's next move of its own.
    fn beat_at(&mut self) {
        if let Some(timer) = self.beat_timer.take() {
            self.window.clear_timeout_with_handle(timer);
        }
        let wake = self.client.as_ref().and_then(Client::wake_at);
        let Some(at) = [wake, self.quiet_until].into_iter().flatten().min() else { return };
        let delay = at.saturating_sub(self.platform.now_ms()).min(i32::MAX as u64) as i32;
        self.beat_timer = self.window.set_timeout_with_callback_and_timeout_and_arguments_0(&self.on_beat, delay).ok();
    }

    /// Has the server fetch a picture the document links to, which a page may not read from another site
    /// itself: to show it, or to keep it in the document. The server fetches what its own rules allow, and
    /// what nobody asked for only from a source it trusts.
    fn link(&mut self, src: String, link: Linking) {
        // Without a server the picture keeps its frame and cloud.
        let Some(server) = &self.server else { return };
        if server.trust == letters_api::Trust::Safe {
            // The window asks again by itself once the document is trusted.
            return self.view.set_picture_state(&src, Some(PictureState::Held));
        }
        let trusted = HttpsUrl::parse(&src).is_ok_and(|url| self.sources.iter().any(|source| source.allows(&url)));
        self.view.set_picture_trusted(&src, trusted);
        let asked = trusted || !matches!(link, Linking::Open);
        // A picture nobody asked for waits as it is: the server may well not fetch it.
        if !matches!(link, Linking::Open) {
            self.view.set_picture_state(&src, Some(PictureState::Loading));
        }
        let (id, fetch) = (server.id.clone(), Fetch { url: src.clone(), asked });
        let fetch = serde_json::to_vec(&fetch).expect("an address to fetch is JSON");
        let sent =
            browser::ask_and_wait(&self.window, "POST", &paths::fetch_picture(&id), Some((letters_api::JSON, &fetch)));
        let (shell, window) = (self.me.clone(), self.window.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let fetched = sent.await;
            let (mut picture, mut image) = (None, None);
            match (&fetched, &link) {
                (Ok(bytes), Linking::Embed(_)) => picture = browser::picture_of(&window, bytes).await,
                (Ok(bytes), _) => image = browser::decode(&window, bytes).await,
                (Err(_), _) => {}
            }
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            let shell = &mut *shell;
            // The page may show another document by now, which those that waited are not of either.
            if shell.server.as_ref().is_none_or(|server| server.id != id) {
                return shell.slow.clear();
            }
            let failed = |why: &str| Some(PictureState::Failed(why.to_owned()));
            match (fetched, link) {
                // It waits to be asked for, as any picture from a source nobody said they trust.
                (Err((refusal, _)), Linking::Open) if refusal.code == Code::Untrusted => {}
                // The server has fetched all it does for now: the picture waits as it is, and its turn comes.
                (Err((refusal, wait)), link) if refusal.code == Code::Slow => {
                    shell.slow.push_back((src, link));
                    return shell.link_later(wait.unwrap_or(SLOW_SECONDS));
                }
                (Err((refusal, _)), _) => shell.view.set_picture_state(&src, failed(&reason(&refusal))),
                (Ok(_), Linking::Embed(figure)) => match picture {
                    Some(picture) => {
                        shell.view.set_picture_state(&src, None);
                        shell.view.embed_picture(&figure, picture, &mut shell.platform);
                    }
                    None => shell.view.set_picture_state(&src, failed(NO_PICTURE)),
                },
                (Ok(bytes), _) => match image {
                    Some(image) => {
                        shell.linked.insert(src.clone(), bytes);
                        shell.view.set_image(&src, Some(image));
                    }
                    None => shell.view.set_picture_state(&src, failed(NO_PICTURE)),
                },
            }
            // The server fetches again, so the next of those it would not is asked for.
            if shell.slow_timer.is_none() {
                shell.link_again();
            }
            shell.arrived();
        });
    }

    /// Asks for the first of the pictures that wait for the server in `seconds`, unless a time is set.
    fn link_later(&mut self, seconds: u32) {
        let wait = seconds.clamp(1, SLOW_LONGEST).saturating_mul(1000) as i32;
        let set = || self.window.set_timeout_with_callback_and_timeout_and_arguments_0(&self.on_slow, wait).ok();
        self.slow_timer = self.slow_timer.or_else(set);
    }

    /// Asks for the first of the pictures that wait for the server. One is asked at a time: all of them
    /// at once would be all the server fetches in that while, and each refused again.
    fn link_again(&mut self) {
        self.slow_timer = None;
        if let Some((src, link)) = self.slow.pop_front() {
            self.link(src, link);
        }
    }

    /// Has the server read the page at an address for the window's link previews: what the page says of
    /// itself, and the picture it shows itself by, which the document then keeps.
    fn preview(&mut self, url: String) {
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else {
            return self.view.fail_preview(&url, "there is no server to fetch it");
        };
        let sent = fetching(&self.window, &paths::fetch_preview(&id), &Fetch { url: url.clone(), asked: true });
        let (shell, window) = (self.me.clone(), self.window.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let found = match sent.await {
                Ok(found) => serde_json::from_slice::<Preview>(&found)
                    .map_err(|_| "the server's answer could not be read".to_owned()),
                Err(refusal) => Err(reason(&refusal)),
            };
            // A page whose picture cannot be had is still what it says of itself.
            let mut picture = None;
            if let Ok(Preview { picture: Some(address), .. }) = &found {
                let fetch = Fetch { url: address.clone(), asked: true };
                if let Ok(bytes) = fetching(&window, &paths::fetch_picture(&id), &fetch).await {
                    picture = browser::picture_of(&window, &bytes).await;
                }
            }
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            let shell = &mut *shell;
            match found {
                Ok(Preview { title, description, .. }) => {
                    let found = LinkPreviewFound { url, title, description, picture };
                    shell.view.fill_preview(found, &mut shell.platform);
                }
                Err(why) => shell.view.fail_preview(&url, &why),
            }
            shell.arrived();
        });
    }

    /// Fetches the pictures `document` shows that the page never came to show, and the font files `style`
    /// lists, so that a copy of it leaves none out. `then` is told why if one cannot be had.
    fn gather(
        &mut self,
        document: &Document,
        style: &Style,
        then: impl FnOnce(&mut Self, Result<(), String>) + 'static,
    ) {
        let lacking = |name: &&str| !self.platform.assets.contains_key(*name);
        let Some(id) = self.server.as_ref().map(|server| server.id.clone()) else {
            return then(self, Ok(()));
        };
        let names = document.assets().into_iter().chain(style.assets());
        let asked: Vec<_> = names
            .into_iter()
            .filter(lacking)
            .map(|name| (name.to_owned(), browser::ask(&self.window, "GET", &asset(&id, name), None)))
            .collect();
        if asked.is_empty() {
            return then(self, Ok(()));
        }
        self.exporting += 1;
        let shell = self.me.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let mut fetched = Vec::new();
            for (name, asked) in asked {
                fetched.push((name, asked.await));
            }
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            shell.exporting -= 1;
            let mut all = Ok(());
            for (name, bytes) in fetched {
                match bytes {
                    Ok(bytes) => _ = shell.platform.assets.entry(name).or_insert(bytes),
                    Err(_) => all = Err("one of its pictures could not be fetched from the server".to_owned()),
                }
            }
            then(&mut shell, all);
            shell.arrived();
        });
    }

    /// The pictures chosen after "Image" in the insert menu go in at the caret.
    fn picked(&mut self, _: Event) {
        let files = files(self.picker.files());
        // The same files chosen again are a change again.
        self.picker.set_value("");
        self.pictures(files, |shell, pictures| shell.view.insert_pictures(pictures, None, &mut shell.platform));
    }

    /// Files held over the page show where their pictures would land.
    fn drag_over(&mut self, event: DragEvent) {
        let Some(data) = event.data_transfer().filter(|data| data.types().includes(&"Files".into(), 0)) else { return };
        // The browser would leave the page for a file let go on it, so every place answers.
        event.prevent_default();
        // Until they are let go, files say only what kind they are. One that is no picture may be a
        // document, which is opened wherever it lands.
        let items = data.items();
        let pictures =
            (0..items.length()).filter_map(|at| items.get(at)).all(|item| item.type_().starts_with("image/"));
        let at = Point { x: event.offset_x() as f32, y: event.offset_y() as f32 };
        let lands = self.view.drag_pictures(pictures.then_some(at));
        data.set_drop_effect(if lands || !pictures { "copy" } else { "none" });
    }

    fn drop(&mut self, event: DragEvent) {
        event.prevent_default();
        let at = Point { x: event.offset_x() as f32, y: event.offset_y() as f32 };
        let mut files = files(event.data_transfer().and_then(|data| data.files()));
        // A document among them is what was meant, and it takes the page.
        if let Some(document) = files.iter().position(|file| file.name().ends_with(DOCUMENTS)) {
            _ = self.view.drag_pictures(None);
            return self.open_file(files.swap_remove(document));
        }
        self.pictures(files, move |shell, pictures| {
            shell.view.insert_pictures(pictures, Some(at), &mut shell.platform)
        });
    }

    // The pointer

    fn pointer_down(&mut self, event: PointerEvent) {
        if event.button() != 0 {
            return;
        }
        // A drag keeps reporting to the canvas after it leaves it.
        report(self.canvas.set_pointer_capture(event.pointer_id()));
        self.fingers = touch(&event).is_some();
        if let Some(touch) = touch(&event) {
            return self.finger(Finger::Down { id: touch, at: place(&event), ms: event.time_stamp() });
        }
        self.pointer_move(event.clone());
        self.view.pointer_down(self.pointer, modifiers(held!(event), self.mac), &mut self.platform);
    }

    fn pointer_move(&mut self, event: PointerEvent) {
        if let Some(touch) = touch(&event) {
            return self.finger(Finger::Move { id: touch, at: place(&event), ms: event.time_stamp() });
        }
        self.pointer = place(&event);
        self.view.pointer_move(self.pointer, &mut self.platform);
    }

    fn pointer_up(&mut self, event: PointerEvent) {
        if let Some(touch) = touch(&event) {
            self.finger(Finger::Up { id: touch, ms: event.time_stamp() });
        } else if event.button() == 0 {
            self.view.pointer_up(self.pointer, &mut self.platform);
            self.hand_over();
        }
    }

    /// Puts on the clipboard what the view copied with no clipboard event to carry it, as a press on a
    /// code block's copy button does.
    fn hand_over(&mut self) {
        if let Some(clip) = self.platform.copied.take() {
            report(browser::copy(&self.window, &clip));
        }
    }

    fn pointer_cancel(&mut self, event: PointerEvent) {
        match touch(&event) {
            Some(touch) => self.finger(Finger::Cancel { id: touch }),
            None => self.view.pointer_up(self.pointer, &mut self.platform),
        }
    }

    /// Does what a finger's movement comes to: a scroll, or the press and release of a tap.
    fn finger(&mut self, finger: Finger) {
        match self.touch.event(finger) {
            Gesture::None => {}
            Gesture::Scroll { at, by } => self.view.scroll(at, by, &mut self.platform),
            Gesture::Tap(at) => {
                self.pointer = at;
                self.view.pointer_move(at, &mut self.platform);
                self.view.pointer_down(at, Modifiers::default(), &mut self.platform);
                self.view.pointer_up(at, &mut self.platform);
                self.hand_over();
            }
        }
    }

    // Selection handles

    /// Puts the handles under the two ends of a selection a finger made, and away otherwise.
    fn place_handles(&mut self) {
        let ranged = self.view.editor().selection().is_some_and(|selection| !selection.is_caret());
        let mut ends = match self.fingers && ranged && self.view.types_in_canvas() {
            true => self.view.selection_ends(),
            false => [None, None],
        };
        // The end a held handle moves is the head for as long as it is held.
        if matches!(self.grab, Some((0, _))) {
            ends.swap(0, 1);
        }
        let canvas = ends.iter().flatten().next().map(|_| self.canvas.get_bounding_client_rect());
        let ink = match self.view.settings().accent {
            Some([r, g, b]) => (r, g, b),
            None => {
                let ink = tokens::ACCENT.resolve(if self.dark() { Scheme::Dark } else { Scheme::Light });
                (ink.r, ink.g, ink.b)
            }
        };
        for ((handle, styled), end) in self.handles.iter_mut().zip(ends) {
            let style = match canvas.as_ref().zip(end) {
                Some((canvas, end)) => {
                    let (left, top) = (canvas.left() + f64::from(end.x), canvas.top() + f64::from(end.y + end.height));
                    let (r, g, b) = ink;
                    let dot = format!(
                        "radial-gradient(circle {DOT}px at 50% {}px,rgb({r} {g} {b}) 95%,transparent)",
                        DOT + 1.0
                    );
                    format!("{HANDLE}left:{left}px;top:{top}px;background:{dot}")
                }
                None => String::new(),
            };
            if style != *styled {
                handle.set_hidden(style.is_empty());
                report(handle.set_attribute("style", &style));
                *styled = style;
            }
        }
    }

    /// A finger on a handle moves the end of the selection it is at, as a drag with Shift held does.
    fn handle(&mut self, which: usize, event: PointerEvent) {
        let canvas = self.canvas.get_bounding_client_rect();
        let finger = Point {
            x: (f64::from(event.client_x()) - canvas.left()) as f32,
            y: (f64::from(event.client_y()) - canvas.top()) as f32,
        };
        if event.type_() == "pointerdown" {
            let (Some(end), Some(selection)) = (self.view.selection_ends()[which], self.view.editor().selection())
            else {
                return;
            };
            event.prevent_default();
            report(self.handles[which].0.set_pointer_capture(event.pointer_id()));
            // A drag moves the head, so the anchor's handle makes its end the head first.
            if which == 0 {
                let turned = Selection { anchor: selection.head.clone(), head: selection.anchor.clone() };
                self.view.select(turned);
            }
            let line = Point { x: end.x, y: end.y + end.height / 2.0 };
            self.grab = Some((which, Point { x: finger.x - line.x, y: finger.y - line.y }));
            self.pointer = line;
            let shift = Modifiers { shift: true, ..Modifiers::default() };
            return self.view.pointer_down(line, shift, &mut self.platform);
        }
        let Some((_, from)) = self.grab.filter(|(held, _)| *held == which) else { return };
        self.pointer = Point { x: finger.x - from.x, y: finger.y - from.y };
        if event.type_() == "pointermove" {
            self.view.pointer_move(self.pointer, &mut self.platform);
        } else {
            self.view.pointer_up(self.pointer, &mut self.platform);
            self.grab = None;
        }
    }

    fn menu(&mut self, event: MouseEvent) {
        // The browser's own menu offers nothing for a canvas but saving it as a picture.
        event.prevent_default();
        self.pointer = Point { x: event.offset_x() as f32, y: event.offset_y() as f32 };
        self.view.secondary_down(self.pointer, &mut self.platform);
    }

    fn menu_key(&mut self, event: Event) {
        event.prevent_default();
        self.view.menu_key(&mut self.platform);
    }

    fn wheel(&mut self, event: WheelEvent) {
        // Ctrl with the wheel zooms the page, and so does a pinch, which arrives the same way.
        if event.ctrl_key() {
            return;
        }
        let unit = match event.delta_mode() {
            WheelEvent::DOM_DELTA_LINE => WHEEL_LINE,
            WheelEvent::DOM_DELTA_PAGE => self.canvas.client_height() as f32,
            _ => 1.0,
        };
        let delta = Point { x: event.delta_x() as f32 * unit, y: event.delta_y() as f32 * unit };
        self.view.scroll(self.pointer, delta, &mut self.platform);
    }
}

/// A file the exports wrote.
/// A file fetched when a document first needs it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Extra {
    /// A face that backs up the fonts.
    Face(Face),
    /// The word list of a script that is written without spaces.
    Words(WordList),
}

impl Extra {
    /// Its folder beside the wasm, and what it is called there.
    fn file(self) -> (&'static str, String) {
        match self {
            Self::Face(face) => ("fallback", face.file().to_owned()),
            Self::Words(list) => ("words", list.file()),
        }
    }
}

/// What the keys of the notices about a document's fonts start with, before the family: one nothing has,
/// and one of the library that could not be fetched.
const FONT_KEY: &str = "font:";
const FETCH_KEY: &str = "font-fetch:";

/// How far the list of the library's families has come.
#[derive(Clone, Default, PartialEq, Eq)]
enum Listing {
    #[default]
    Unasked,
    Coming,
    Here(Rc<str>),
    /// The server has none to give.
    Lost,
}

/// What the page knows of the fonts its document's writer chose.
#[derive(Default)]
struct Chosen {
    library: Listing,
    /// How far each font file the style lists has come, by its name.
    files: HashMap<String, chosen::Arrival>,
    /// How far each face asked of the library has come.
    faces: HashMap<Want, chosen::Arrival>,
    /// The faces of the library that came, and the names their files are kept under.
    arrived: Vec<(Want, String)>,
    /// The faces the document's text asked for.
    named: Vec<Named>,
    /// The families a notice was shown for, in lower case.
    said: BTreeSet<String>,
}

/// How far such a file has come.
#[derive(Clone, PartialEq, Eq)]
enum Arrival {
    Coming,
    Here(Arc<[u8]>),
    /// It could not be fetched, and is not asked for again.
    Lost,
}

/// What is told whether files it waited for are here.
type Arrived = Box<dyn FnOnce(&mut Shell, bool)>;

/// What is done with a file the exports made, or with why there is none.
type Made = Box<dyn FnOnce(&mut Shell, Result<Exported, String>)>;

/// The file a document may need that is called `name`.
fn extra(name: JsValue) -> Option<Extra> {
    let all = Face::ALL.into_iter().map(Extra::Face).chain(WordList::ALL.into_iter().map(Extra::Words));
    all.into_iter().find(|extra| name.as_string() == Some(extra.file().1))
}

/// What a file is written from.
enum Source {
    /// The document as it now is, as its own file.
    Own,
    /// The document as it now is, as a document of its own that starts with this one's text and history.
    Copy,
    /// The document and its style as they read at a version, as a document of its own.
    Version(Box<(Document, Style)>),
}

struct Exported {
    /// What to call it, unless nothing of what it was to be called can name a file.
    name: Option<String>,
    /// Its media type.
    kind: String,
    file: JsValue,
}

/// Offers what is at `url` to the person as a file to keep, called `name`.
fn keep(page: &web_sys::Document, url: &str, name: &str) -> Result<(), JsValue> {
    let link: HtmlAnchorElement = page.create_element("a")?.dyn_into()?;
    link.set_href(url);
    link.set_download(name);
    link.click();
    Ok(())
}

/// What an error says, for a sentence the person reads.
fn said(error: &JsValue) -> String {
    let message = error.dyn_ref::<js_sys::Error>().map(|error| String::from(error.message()));
    message.or_else(|| error.as_string()).unwrap_or_else(|| "it failed without saying why".to_owned())
}

/// Calls a function that a module exports.
fn call(module: &JsValue, name: &str, arguments: &Array) -> Result<JsValue, JsValue> {
    Reflect::get(module, &name.into())?.dyn_into::<Function>()?.apply(&JsValue::NULL, arguments)
}

/// The files a paste, a drop or a chooser carries.
fn files(list: Option<FileList>) -> Vec<File> {
    list.map(|list| (0..list.length()).filter_map(|at| list.get(at)).collect()).unwrap_or_default()
}

/// The pointer's id if it is a finger, whose movements are gestures.
fn touch(event: &PointerEvent) -> Option<i32> {
    (event.pointer_type() == "touch").then(|| event.pointer_id())
}

/// Where on the canvas a pointer event happened.
fn place(event: &PointerEvent) -> Point {
    Point { x: event.offset_x() as f32, y: event.offset_y() as f32 }
}

/// How far down the document the person has scrolled the HTML that stands in for it.
fn read_to(first: &Element) -> f64 {
    let frame =
        first.query_selector("iframe").ok().flatten().and_then(|frame| frame.dyn_into::<HtmlIFrameElement>().ok());
    frame.and_then(|frame| frame.content_window()).and_then(|inner| inner.scroll_y().ok()).unwrap_or(0.0)
}

/// The document the page carries for the editor to open. The harness may carry none, and is welcomed.
pub fn carried(window: &Window, platform: &mut Browser) -> Document {
    let page = window.document();
    let carried =
        page.and_then(|page| page.get_element_by_id(page::DOCUMENT)).and_then(|carried| carried.text_content());
    match carried.as_deref().map(Document::from_json) {
        Some(Ok(opened)) => opened,
        Some(Err(error)) => {
            web_sys::console::error_1(&format!("the page's document cannot be read: {error}").into());
            page::welcome(&mut || platform.new_id())
        }
        None => {
            let mut welcome = page::welcome(&mut || platform.new_id());
            // A new document is written in the person's language.
            if let Some(language) = platform.settings().language {
                welcome.language = language;
            }
            welcome
        }
    }
}

/// A socket to the document's server, and what listens to it.
struct Line {
    socket: WebSocket,
    /// Which of the sockets asked for it is.
    number: u32,
    _heard: [Closure<dyn FnMut(JsValue)>; 3],
}

/// Opens a socket to `path` on the page's own server, and has the shell told what comes of it. `None`
/// when the browser will not open one.
fn line(window: &Window, path: &str, shell: &Weak<RefCell<Shell>>, number: u32) -> Option<Line> {
    let address = Url::new_with_base(path, &window.location().href().ok()?).ok()?;
    address.set_protocol(if address.protocol() == "https:" { "wss:" } else { "ws:" });
    let socket = WebSocket::new(&address.href()).ok()?;
    socket.set_binary_type(BinaryType::Arraybuffer);
    let hear = |kind: &str, heard: fn(&mut Shell, JsValue)| {
        let shell = shell.clone();
        let heard = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
            let Some(shell) = shell.upgrade() else { return };
            let mut shell = shell.borrow_mut();
            // What an earlier socket still reports is no news of this one.
            if shell.socket.as_ref().is_some_and(|line| line.number == number) {
                heard(&mut shell, event);
                shell.sync();
            }
        });
        socket.add_event_listener_with_callback(kind, heard.as_ref().unchecked_ref()).ok()?;
        Some(heard)
    };
    let heard = [
        hear("open", |shell, _| shell.tell(|client, local, now| client.opened(local, now)))?,
        hear("message", |shell, event| {
            let frame = Reflect::get(&event, &"data".into()).map(|data| Uint8Array::new(&data).to_vec());
            shell.tell(|client, local, now| client.received(&frame.unwrap_or_default(), local, now));
        })?,
        // A socket that could not be opened closes too.
        hear("close", |shell, _| {
            shell.hung = shell.socket.take();
            shell.tell(|client, _, now| client.closed(now));
        })?,
    ];
    Some(Line { socket, number, _heard: heard })
}

/// The document as the client is lent it. Until the server's history is the document's, the page holds
/// no copy the client could read.
fn local(view: &View, attached: bool) -> Local<'_> {
    Local { model: attached.then(|| view.editor().model()), notes: view.notes() }
}

/// What a linked picture is fetched for.
enum Linking {
    /// The document opened, and nobody asked.
    Open,
    /// The person asked for it this once.
    Once,
    /// The person wants it kept in this figure.
    Embed(BlockId),
}

/// The least time between two tellings of what the person last saw, while they write.
const SEEN_APART_MS: u64 = 30_000;

/// How long a picture waits for a server that has fetched all it does for now and does not say how long:
/// its count starts again with each minute.
const SLOW_SECONDS: u32 = 60;
/// The longest such a wait is, whatever the server says.
const SLOW_LONGEST: u32 = 600;

/// What a figure says of a file that was fetched for it and is no picture, as the desktop does.
const NO_PICTURE: &str = "it is not a picture Letters can show";

/// Asks the server to fetch an address on the document's behalf.
fn fetching(
    window: &Window,
    path: &str,
    fetch: &Fetch,
) -> impl Future<Output = Result<Vec<u8>, letters_api::Error>> + 'static + use<> {
    let fetch = serde_json::to_vec(fetch).expect("an address to fetch is JSON");
    browser::ask(window, "POST", path, Some((letters_api::JSON, &fetch)))
}

/// Why the server did not fetch, in words that read on from "Not loaded:": its own sentence, as part of one.
fn reason(refusal: &letters_api::Error) -> String {
    let said = refusal.message.trim().trim_end_matches('.');
    let mut letters = said.chars();
    letters.next().map(|first| first.to_lowercase().chain(letters).collect()).unwrap_or_default()
}

/// Where the server keeps a picture of the document `id`, by the name the document has for it.
fn asset(id: &str, name: &str) -> String {
    paths::asset(id, name.strip_prefix("assets/").unwrap_or(name))
}

/// What the server behind the page says of the document it carries. `None` for a page with no server.
pub fn served(window: &Window) -> Option<Opened> {
    let said = window.document()?.get_element_by_id(page::OPENED)?.text_content()?;
    match serde_json::from_str(&said) {
        Ok(opened) => Some(opened),
        Err(error) => {
            web_sys::console::error_1(&format!("what the server says of the document cannot be read: {error}").into());
            None
        }
    }
}

/// Where the files called `names` are, in the folder `folder` under `base`.
pub fn urls(base: &str, folder: &str, names: &[&str]) -> Vec<String> {
    names.iter().map(|name| format!("{base}{folder}/{name}")).collect()
}

/// A window on `document`, set up for this browser.
fn view(document: Document, platform: &mut Browser, mac: bool) -> View {
    set_up(page::window(document, ReplicaId::new(&platform.new_id())), platform, mac)
}

/// Sets a window up for this browser and the person in it.
fn set_up(mut view: View, platform: &mut Browser, mac: bool) -> View {
    view.set_keys(if mac { Keys::Mac } else { Keys::Standard });
    view.set_utc_offset(zone());
    view.set_user_words(platform.user_words());
    view.set_default_style(platform.default_style());
    // A page may not read another site's pages, so a link preview's fetch waits for a server. None is
    // offered until then, and a preview shows what the document keeps of its page.
    view.set_preview_fetching(false);
    follow(&mut view, platform.settings());
    view
}

/// How far the browser's clock is ahead of UTC, in minutes.
fn zone() -> i32 {
    -js_sys::Date::new_0().get_timezone_offset() as i32
}

/// Has `view` take `settings`, and sign the person's edits with the name they set. There is no account to
/// name them otherwise.
fn follow(view: &mut View, settings: Settings) {
    if let Some(name) = &settings.name {
        view.set_author(name);
    }
    if *view.settings() != settings {
        view.set_settings(settings);
    }
}

/// Copies one rectangle of the frame to the same place on the canvas.
fn put(context: &CanvasRenderingContext2d, frame: &Frame, rect: PixelRect, strip: &mut Vec<u8>) -> Result<(), JsValue> {
    let stride = frame.width() as usize * 4;
    let (left, right) = (rect.x as usize * 4, ((rect.x + rect.width) as usize * 4).min(stride));
    let (top, bottom) = (rect.y as usize, ((rect.y + rect.height) as usize).min(frame.height() as usize));
    if left >= right || top >= bottom {
        return Ok(());
    }
    let rows = &frame.data()[top * stride..bottom * stride];
    // A frame painted over an opaque background has no alpha to undo. Whole rows go as they are.
    let pixels = if right - left == stride {
        rows
    } else {
        strip.clear();
        rows.chunks_exact(stride).for_each(|row| strip.extend_from_slice(&row[left..right]));
        strip
    };
    let width = ((right - left) / 4) as u32;
    let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(pixels), width, (bottom - top) as u32)?;
    context.put_image_data(&image, f64::from(rect.x), f64::from(rect.y))
}

/// Calls `handler` for each `kind` of event on `target`, for as long as the page lives.
fn listen(target: &EventTarget, kind: &str, handler: impl FnMut(Event) + 'static) -> Result<(), JsValue> {
    let handler = Closure::<dyn FnMut(Event)>::new(handler).into_js_value();
    target.add_event_listener_with_callback(kind, handler.unchecked_ref())
}

/// Hands each `kind` of event on `target` to the shell, then brings the page up to date.
fn hear<E: JsCast + 'static>(
    shell: &Rc<RefCell<Shell>>,
    target: &EventTarget,
    kind: &str,
    handler: impl Fn(&mut Shell, E) + 'static,
) -> Result<(), JsValue> {
    let shell = Rc::downgrade(shell);
    // The pointer's travels change nothing the mirror says.
    let quiet = matches!(kind, "pointermove" | "pointerleave" | "wheel" | "dragover" | "dragleave");
    let edit = matches!(kind, "beforeinput" | "input" | "compositionstart" | "compositionend");
    listen(target, kind, move |event| {
        let Some(shell) = shell.upgrade() else { return };
        let mut shell = shell.borrow_mut();
        // What the event asks for is done where the keyboard left the caret.
        match edit {
            true => shell.editing = true,
            false => shell.keyboard_moved(),
        }
        handler(&mut shell, event.unchecked_into());
        if !quiet {
            shell.changed();
        }
        shell.sync();
    })
}

/// A function the browser can call back, for frames and timers.
fn callback(shell: &Weak<RefCell<Shell>>, run: fn(&mut Shell)) -> Function {
    let shell = shell.clone();
    let run = move || {
        let Some(shell) = shell.upgrade() else { return };
        let mut shell = shell.borrow_mut();
        run(&mut shell);
        shell.sync();
    };
    Closure::<dyn FnMut()>::new(run).into_js_value().unchecked_into()
}

/// A browser call that fails has nowhere better to be reported than the console.
fn report(result: Result<(), JsValue>) {
    if let Err(error) = result {
        web_sys::console::error_1(&error);
    }
}
