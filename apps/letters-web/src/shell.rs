//! The page around the engine: a canvas it paints into, a hidden input beside the canvas that takes the
//! keyboard, and a mirror of the window as HTML for assistive technology.
//!
//! Every event ends the same way, in [`Shell::sync`], which asks for a frame if the view wants one and
//! moves the hidden input under the caret, where the input method's candidate window should open.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::{Rc, Weak};

use accesskit::{Action, ActionData, ActionRequest, NodeId, TextPosition, TextSelection, TreeId};
use js_sys::{Function, Object, Reflect};
use letters_editor::{Clip, Modifiers, Platform as _};
use letters_model::{BlockKind, Document, HistoryError, Mark, Pos, ReplicaId, Selection};
use letters_render::cpu::{CpuRenderer, Frame, PixelRect};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Point, Rect, Size};
use letters_ui::{Appearance, Cursor, Edge, Keys, Request, Scheme, Settings, Window as View, tokens};
use wasm_bindgen::prelude::*;
use wasm_bindgen::{Clamped, JsCast};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, BlobPropertyBag, CanvasRenderingContext2d, ClipboardEvent, CompositionEvent, Element, Event, EventTarget,
    HtmlAnchorElement, HtmlCanvasElement, HtmlIFrameElement, HtmlInputElement, HtmlTextAreaElement, ImageData,
    KeyboardEvent, MediaQueryList, MouseEvent, PointerEvent, Url, WheelEvent, Window,
};

use crate::browser::{self, Browser};
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

/// The mirror: over the canvas, so that what it says about where things are is true, and out of sight and
/// out of the pointer's way. Its text is not for the eye, so the page lays out as little of it as it can.
const MIRROR: &str = "position:fixed;overflow:hidden;opacity:0;pointer-events:none;user-select:none;contain:strict;";
/// How long the window must stay as it is before the mirror is brought up to date, in milliseconds. Reading
/// the whole window costs more than a key press should wait for.
const MIRROR_MS: i32 = 150;

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
    cursor: Cursor,
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
        later: Later,
    ) -> Result<Rc<RefCell<Self>>, JsValue> {
        let window = web_sys::window().ok_or("there is no window")?;
        let document = window.document().ok_or("there is no document")?;
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

        let mac = window.navigator().platform().is_ok_and(|name| name.starts_with("Mac") || name.starts_with("iP"));
        let dark_mode = window.match_media("(prefers-color-scheme: dark)")?;
        let less_motion = window.match_media("(prefers-reduced-motion: reduce)")?;
        let view = view(opened, &mut platform, mac);

        let shell = Rc::new_cyclic(|shell| {
            RefCell::new(Self {
                me: shell.clone(),
                later: Some(later),
                on_frame: callback(shell, Self::paint),
                on_wake: callback(shell, Self::wake),
                on_mirror: callback(shell, Self::reflect),
                mirror: Page::new(&document, holder.clone()),
                holder: holder.clone(),
                mirror_timer: None,
                chooser: chooser.clone(),
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
                cursor: Cursor::Default,
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
                shell.changed();
                shell.sync();
            });
        })?;

        hear(&shell, &canvas, "pointerdown", Self::pointer_down)?;
        hear(&shell, &canvas, "pointermove", Self::pointer_move)?;
        hear(&shell, &canvas, "pointerup", Self::pointer_up)?;
        hear(&shell, &canvas, "pointercancel", Self::pointer_cancel)?;
        hear(&shell, &canvas, "pointerleave", |shell, _: Event| shell.view.pointer_leave())?;
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
        hear(&shell, &document, "selectionchange", Self::selection_change)?;
        hear(&shell, &input, "copy", |shell, event| shell.clipboard(event, Clipboard::Copy))?;
        hear(&shell, &input, "cut", |shell, event| shell.clipboard(event, Clipboard::Cut))?;
        hear(&shell, &input, "paste", |shell, event| shell.clipboard(event, Clipboard::Paste))?;
        hear(&shell, &input, "focus", |shell, _: Event| shell.view.focus(true))?;
        hear(&shell, &input, "blur", |shell, _: Event| shell.view.focus(false))?;
        hear(&shell, &window, "resize", |shell, _: Event| shell.fit())?;
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
            shell.sync();
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
            shell.sync();
        });
    }

    /// Offers `text` to the person as a file to keep, named after the document.
    fn download(&self, text: &str, extension: &str) -> Result<(), JsValue> {
        let page = self.window.document().ok_or("there is no document")?;
        let parts = js_sys::Array::of1(&text.into());
        let options = BlobPropertyBag::new();
        options.set_type("application/json");
        let url = Url::create_object_url_with_blob(&Blob::new_with_str_sequence_and_options(&parts, &options)?)?;
        let link: HtmlAnchorElement = page.create_element("a")?.dyn_into()?;
        link.set_href(&url);
        link.set_download(&format!("{}.{extension}", page.title()));
        link.click();
        Url::revoke_object_url(&url)
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
        for request in self.view.take_requests() {
            match request {
                // There are no folders to move between, so a name is the person's to give.
                Request::Named(name) | Request::Rename(name) => {
                    self.view.set_document_name(&name);
                    if let Some(page) = self.window.document() {
                        page.set_title(&name);
                    }
                }
                Request::SaveDefaultStyle(json) => {
                    if !self.platform.keep_default_style(&json) {
                        self.view.set_default_style(self.platform.default_style());
                    }
                }
                // The browser asks the person for the file, and the chooser's `change` hands it over.
                Request::ImportStyle => self.chooser.click(),
                Request::ExportStyle(json) => report(self.download(&json, "letstyle")),
                // The window has taken the settings already. The rest is the shell's: keeping them, the
                // name edits carry, and light or dark.
                Request::Settings(settings) => {
                    self.platform.keep_settings(&settings);
                    follow(&mut self.view, settings);
                    self.view.set_dark(self.dark());
                }
                // Opening, starting and moving need files, which wait for a server behind the page.
                _ => {}
            }
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
        let cursor = self.view.cursor();
        if cursor != self.cursor {
            self.cursor = cursor;
            let name = match cursor {
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
            report(self.canvas.style().set_property("cursor", name));
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
        let damage = self.renderer.render(&self.list, &Images::default(), self.scale, background, &mut self.frame);
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
        }
        self.stats.frame_ms = end - start;
        self.wake_at(wake);
    }

    // The mirror

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
            }
            Reply::Text(text) => {
                self.view.input(text, &mut self.platform);
                event.prevent_default();
            }
            Reply::Select(range) => {
                let Some(block) = self.view.editor().selection().map(|selection| selection.head.block.clone()) else {
                    return;
                };
                let (anchor, head) = (Pos::new(block.clone(), range.start), Pos::new(block, range.end));
                self.view.select(Selection { anchor, head });
            }
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

    /// The selection moved somewhere on the page. In the hidden input, only a keyboard moves it.
    fn selection_change(&mut self, event: Event) {
        let active = self.window.document().and_then(|page| page.active_element());
        if active.is_some_and(|active| active == **self.input) {
            self.text(Dom::Select { value: &self.input.value(), selection: self.selected() }, &event);
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
            if text.is_none() && html.is_none() {
                return;
            }
            self.platform.pasted = Some(Clip { text: text.unwrap_or_default(), html, ..Clip::default() });
            self.view.key(key, &mut self.platform);
            self.platform.pasted = None;
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

    // The pointer

    fn pointer_down(&mut self, event: PointerEvent) {
        if event.button() != 0 {
            return;
        }
        // A drag keeps reporting to the canvas after it leaves it.
        report(self.canvas.set_pointer_capture(event.pointer_id()));
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
            }
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

/// Where the files called `names` are, in the folder `folder` under `base`.
pub fn urls(base: &str, folder: &str, names: &[&str]) -> Vec<String> {
    names.iter().map(|name| format!("{base}{folder}/{name}")).collect()
}

/// A window on `document`, set up for this browser.
fn view(document: Document, platform: &mut Browser, mac: bool) -> View {
    let mut view = page::window(document, ReplicaId::new(&platform.new_id()));
    view.set_keys(if mac { Keys::Mac } else { Keys::Standard });
    view.set_user_words(platform.user_words());
    view.set_default_style(platform.default_style());
    follow(&mut view, platform.settings());
    view
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
    handler: fn(&mut Shell, E),
) -> Result<(), JsValue> {
    let shell = Rc::downgrade(shell);
    // The pointer's travels change nothing the mirror says.
    let quiet = matches!(kind, "pointermove" | "pointerleave" | "wheel");
    listen(target, kind, move |event| {
        let Some(shell) = shell.upgrade() else { return };
        let mut shell = shell.borrow_mut();
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
