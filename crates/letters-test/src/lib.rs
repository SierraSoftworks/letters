//! The headless interaction-test harness: the real editor and widgets with an injected clock, clipboard and
//! font set. Tests script keys, pointer and IME events, then assert on Markdown, selection and the
//! accessibility tree.
//!
//! A [`Harness`] drives a window through the calls a shell makes, and finds controls by role and name in the
//! AccessKit tree. A control a test cannot find is one a screen reader cannot find either.

use std::rc::Rc;

use accesskit::{ActionRequest, Toggled};
use accesskit_consumer::{NodeRef, Tree, TreeChangeHandler};
use letters_editor::{Clip, Editor, InputEvent, Key, KeyEvent, Modifiers, Picture, Platform};
use letters_model::{Document, Pos, ReplicaId, Selection};
use letters_render::{DisplayList, Point, Rect, Size};
use letters_ui::{App, EditorApp, FixedShaper, Keys, Ui, Window};

pub use letters_ui::Role;

/// The window every harness starts with, in logical pixels.
pub const WINDOW: Size = Size { width: 1280.0, height: 800.0 };

/// The clock, clipboard and ids a test controls.
#[derive(Clone, Debug, Default)]
pub struct TestPlatform {
    pub now_ms: u64,
    /// The time of day, in seconds since 1970. `None` is a shell without a clock.
    pub wall: Option<u64>,
    pub clipboard: Option<Clip>,
    /// The words the person added to their own dictionary.
    pub learned: Vec<String>,
    ids: u64,
}

impl Platform for TestPlatform {
    fn now_ms(&self) -> u64 {
        self.now_ms
    }

    fn wall_time(&self) -> Option<u64> {
        self.wall
    }

    fn clipboard_read(&mut self) -> Option<Clip> {
        self.clipboard.clone()
    }

    fn clipboard_write(&mut self, clip: Clip) {
        self.clipboard = Some(clip);
    }

    fn new_id(&mut self) -> String {
        self.ids += 1;
        format!("t{}", self.ids)
    }

    fn learn_word(&mut self, word: &str) {
        self.learned.push(word.to_owned());
    }
}

/// A control as assistive technology sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub role: Role,
    pub name: String,
    pub value: Option<String>,
    pub description: Option<String>,
    /// Where it is in the window.
    pub bounds: Rect,
    pub focused: bool,
    pub disabled: bool,
    /// Whether a toggle button or checkable item is on.
    pub toggled: Option<bool>,
    /// Whether the popover it opens is showing.
    pub expanded: Option<bool>,
    /// Whether it is the one picked among its siblings, as an option of a list is.
    pub selected: Option<bool>,
}

impl Found {
    fn of(node: &NodeRef<'_>) -> Self {
        let bounds = node.bounding_box().unwrap_or_default();
        // A label is named by the text it shows.
        let name = if node.label_comes_from_value() { node.raw_value().map(str::to_owned) } else { node.label() };
        Self {
            role: node.role(),
            name: name.unwrap_or_default(),
            value: node.value(),
            description: node.description(),
            bounds: Rect::new(bounds.x0 as f32, bounds.y0 as f32, bounds.width() as f32, bounds.height() as f32),
            focused: node.is_focused(),
            disabled: node.is_disabled(),
            toggled: node.toggled().map(|toggled| toggled == Toggled::True),
            expanded: node.data().is_expanded(),
            selected: node.is_selected(),
        }
    }

    pub fn center(&self) -> Point {
        Point { x: self.bounds.x + self.bounds.width / 2.0, y: self.bounds.y + self.bounds.height / 2.0 }
    }
}

/// A window with nothing beneath it but a test's clock and clipboard.
pub struct Harness<A: App = EditorApp> {
    ui: Ui<A>,
    pub platform: TestPlatform,
    /// The accessibility tree as a screen reader holds it: given whole once, and kept up by changes for as
    /// long as the harness alone has asked the window for them.
    heard: Option<Tree>,
}

/// Takes the changes to the tree without a word, as the harness only reads how it stands.
struct Unheard;

impl TreeChangeHandler for Unheard {
    fn node_added(&mut self, _node: &NodeRef) {}
    fn node_updated(&mut self, _old_node: &NodeRef, _new_node: &NodeRef) {}
    fn focus_moved(&mut self, _old_node: Option<&NodeRef>, _new_node: Option<&NodeRef>) {}
    fn node_removed(&mut self, _node: &NodeRef) {}
}

impl<A: App> Harness<A> {
    /// Hosts `app` with fixed text metrics, so widget tests need no fonts.
    pub fn new(app: A) -> Self {
        Self::with(Ui::host(app, Rc::new(FixedShaper)))
    }

    /// Drives `ui` with the key conventions of Linux and Windows, whatever system the tests run on.
    pub fn with(mut ui: Ui<A>) -> Self {
        ui.resize(WINDOW, 1.0);
        ui.set_keys(Keys::Standard);
        Self { ui, platform: TestPlatform::default(), heard: None }
    }

    /// The same window under macOS key conventions.
    pub fn mac(mut self) -> Self {
        self.ui.set_keys(Keys::Mac);
        self
    }

    pub fn app(&self) -> &A {
        self.ui.app()
    }

    /// The window itself, for calls the harness has no shorthand for.
    pub fn ui(&mut self) -> &mut Ui<A> {
        // The caller may ask for the tree too, and the changes after that would not add up to it here.
        self.heard = None;
        &mut self.ui
    }

    pub fn resize(&mut self, width: f32, height: f32) -> &mut Self {
        self.ui.resize(Size { width, height }, 1.0);
        self
    }

    /// Moves the clock on, as time passing between two events.
    pub fn advance(&mut self, ms: u64) -> &mut Self {
        self.platform.now_ms += ms;
        self.ui.animate(self.platform.now_ms);
        self
    }

    // Finding controls

    /// Everything in the accessibility tree, parents before children.
    pub fn controls(&mut self) -> Vec<Found> {
        fn walk(node: NodeRef<'_>, out: &mut Vec<Found>) {
            out.push(Found::of(&node));
            node.children().for_each(|child| walk(child, out));
        }
        // A screen reader is given the tree whole once and the changes from then on, and so is this one.
        let tree = match self.heard.take() {
            Some(mut tree) => {
                tree.update_and_process_changes(self.ui.access_changes(), &mut Unheard);
                tree
            }
            None => Tree::new(self.ui.access(), true),
        };
        let mut out = Vec::new();
        walk(tree.state().root(), &mut out);
        self.heard = Some(tree);
        out
    }

    /// The first control with a role and name.
    pub fn find(&mut self, role: Role, name: &str) -> Option<Found> {
        self.controls().into_iter().find(|found| found.role == role && found.name == name)
    }

    /// The control that has keyboard focus.
    pub fn focused(&mut self) -> Option<Found> {
        self.controls().into_iter().find(|found| found.focused)
    }

    /// Does what assistive technology asks of a node of [`Ui::access`]'s tree.
    pub fn ask(&mut self, request: ActionRequest) -> &mut Self {
        self.ui.access_action(request, &mut self.platform);
        self
    }

    #[track_caller]
    fn expect(&mut self, role: Role, name: &str) -> Found {
        let Some(found) = self.find(role, name) else {
            let named = |found: &Found| format!("  {:?} {:?}", found.role, found.name);
            let all: Vec<String> = self.controls().iter().filter(|found| !found.name.is_empty()).map(named).collect();
            panic!("no {role:?} named {name:?}. The window has:\n{}", all.join("\n"));
        };
        found
    }

    #[track_caller]
    pub fn assert_open(&mut self, role: Role, name: &str) -> &mut Self {
        self.expect(role, name);
        self
    }

    #[track_caller]
    pub fn assert_closed(&mut self, role: Role, name: &str) -> &mut Self {
        assert!(self.find(role, name).is_none(), "{role:?} {name:?} is showing");
        self
    }

    // The pointer

    /// Clicks the middle of a control.
    #[track_caller]
    pub fn click(&mut self, role: Role, name: &str) -> &mut Self {
        let at = self.expect(role, name).center();
        self.click_at(at)
    }

    #[track_caller]
    pub fn double_click(&mut self, role: Role, name: &str) -> &mut Self {
        let at = self.expect(role, name).center();
        self.click_at(at).click_at(at)
    }

    pub fn click_at(&mut self, at: Point) -> &mut Self {
        self.press_at(at).release_at(at)
    }

    #[track_caller]
    pub fn hover(&mut self, role: Role, name: &str) -> &mut Self {
        let at = self.expect(role, name).center();
        self.ui.pointer_move(at, &mut self.platform);
        self
    }

    /// Moves the pointer to a point with no button held.
    pub fn hover_at(&mut self, at: Point) -> &mut Self {
        self.ui.pointer_move(at, &mut self.platform);
        self
    }

    /// Clicks at a point with Ctrl or Cmd held.
    pub fn mod_click_at(&mut self, at: Point) -> &mut Self {
        self.ui.pointer_move(at, &mut self.platform);
        self.ui.pointer_down(at, Modifiers { primary: true, ..Modifiers::default() }, &mut self.platform);
        self.release_at(at)
    }

    pub fn press_at(&mut self, at: Point) -> &mut Self {
        self.ui.pointer_move(at, &mut self.platform);
        self.ui.pointer_down(at, Modifiers::default(), &mut self.platform);
        self
    }

    /// Moves the pointer to `at` with the button held, then lets go.
    pub fn release_at(&mut self, at: Point) -> &mut Self {
        self.ui.pointer_move(at, &mut self.platform);
        self.ui.pointer_up(at, &mut self.platform);
        self
    }

    /// Presses the secondary button at a point, as to ask for a menu there.
    pub fn secondary_click_at(&mut self, at: Point) -> &mut Self {
        self.ui.pointer_move(at, &mut self.platform);
        self.ui.secondary_down(at, &mut self.platform);
        self
    }

    /// Presses the Menu key.
    pub fn menu_key(&mut self) -> &mut Self {
        self.ui.menu_key(&mut self.platform);
        self
    }

    /// Turns the wheel over a point. A positive `down` moves the content up.
    pub fn scroll_at(&mut self, at: Point, down: f32) -> &mut Self {
        self.ui.scroll(at, Point { x: 0.0, y: down }, &mut self.platform);
        self
    }

    // The keyboard

    /// Presses a key, written as a shell would describe it: `"Enter"`, `"Shift+Tab"`, `"Mod+K"`. `Mod` is
    /// Ctrl or Cmd. A character on its own is typed as text, as it is in a shell.
    #[track_caller]
    pub fn key(&mut self, chord: &str) -> &mut Self {
        let mut mods = Modifiers::default();
        let mut key = None;
        for part in chord.split('+') {
            match part {
                "Mod" => mods.primary = true,
                "Shift" => mods.shift = true,
                "Alt" => mods.alt = true,
                name => key = Some(named(name).unwrap_or_else(|| panic!("{chord:?} is not a key"))),
            }
        }
        match key.unwrap_or_else(|| panic!("{chord:?} names no key")) {
            Key::Char(c) if !mods.primary && !mods.alt => {
                let text = if mods.shift { c.to_uppercase().to_string() } else { c.to_string() };
                self.type_text(&text)
            }
            key => {
                self.ui.key(KeyEvent { key, mods }, &mut self.platform);
                self
            }
        }
    }

    /// Types text, one character at a time.
    pub fn type_text(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.ui.input(InputEvent::Insert(c.to_string()), &mut self.platform);
        }
        self
    }

    /// Feeds one event of an input method or a soft keyboard.
    pub fn input(&mut self, event: InputEvent) -> &mut Self {
        self.ui.input(event, &mut self.platform);
        self
    }

    // What shows

    /// Paints a frame.
    pub fn paint(&mut self) -> DisplayList {
        let mut list = DisplayList::default();
        self.ui.paint(&mut list);
        list
    }
}

impl Harness<EditorApp> {
    /// The editor's window on a document, with the bundled fonts and the default style.
    pub fn open(document: Document) -> Self {
        Self::with(Window::new(Editor::new(document, ReplicaId::from(1))))
    }

    /// The editor's window on a document written as Markdown.
    pub fn doc(markdown: &str) -> Self {
        let mut ids = 0;
        let mut new_id = || {
            ids += 1;
            format!("b{ids}")
        };
        Self::open(letters_convert::from_markdown(markdown, &mut new_id).0)
    }

    /// Selects the first place `text` appears in the document.
    #[track_caller]
    pub fn select(&mut self, text: &str) -> &mut Self {
        let found = self.ui.editor().doc().walk().find_map(|block| {
            let start = block.plain_text().find(text)?;
            Some((block.id.clone(), start))
        });
        let (block, start) = found.unwrap_or_else(|| panic!("the document does not hold {text:?}"));
        let anchor = Pos { block: block.clone(), offset: start };
        self.ui.select(Selection { anchor, head: Pos { block, offset: start + text.len() } });
        self
    }

    /// Holds pictures over a point, as files dragged over the window are, or takes them away again with
    /// `None`. Returns whether they could be let go there.
    pub fn hold_pictures(&mut self, at: Option<Point>) -> bool {
        self.ui.drag_pictures(at)
    }

    /// Lets pictures go at a point, or hands over pictures chosen in a dialog with `None`.
    pub fn drop_pictures(&mut self, at: Option<Point>, pictures: Vec<Picture>) -> &mut Self {
        self.ui.insert_pictures(pictures, at, &mut self.platform);
        self
    }

    /// Hands over the bytes of a linked picture to keep in the document, as a shell that was asked to
    /// embed it does.
    pub fn embed_picture(&mut self, figure: &str, picture: Picture) -> &mut Self {
        self.ui.embed_picture(&figure.into(), picture, &mut self.platform);
        self
    }

    /// Holds a finger on a point, as a touch screen's shell passes it on. Returns whether a word was
    /// selected.
    pub fn long_press_at(&mut self, at: Point) -> bool {
        self.ui.select_word_at(at, &mut self.platform)
    }

    pub fn editor(&self) -> &Editor {
        self.ui.editor()
    }

    /// The document as Markdown.
    pub fn markdown(&self) -> String {
        letters_convert::to_markdown(self.ui.editor().doc())
    }

    #[track_caller]
    pub fn assert_markdown(&mut self, expected: &str) -> &mut Self {
        assert_eq!(self.markdown().trim_end(), expected.trim_end());
        self
    }
}

fn named(name: &str) -> Option<Key> {
    Some(match name {
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Escape" => Key::Escape,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Space" => Key::Char(' '),
        _ => {
            let mut chars = name.chars();
            let c = chars.next().filter(|_| chars.next().is_none())?;
            Key::Char(c.to_lowercase().next().unwrap_or(c))
        }
    })
}
