//! Hosts an app's elements in a window.
//!
//! [`Ui`] rebuilds the element tree when the app's state changes, lays it out, routes pointer, key and text
//! input to elements, keeps focus and open popovers, and paints. A shell and the test harness drive it
//! through the same calls, so a test exercises what a person does.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use accesskit::{Action, ActionData, ActionRequest, Node as AccessNode, NodeId, TreeUpdate};
use letters_editor::{Clip, InputEvent, Key, KeyEvent, Modifiers, Platform};
use letters_render::{DisplayList, Point, Rect, Size};

use crate::access;
use crate::element::{Content, Cursor, Element, Focus, Pointer};
use crate::input::{InputState, Motion};
use crate::layout::{Layout, TextCache, inner, position};
use crate::paint::{self, Scene, Tip};
use crate::style::{Align, Dir};
use crate::text::TextShaper;
use crate::tokens::Scheme;
use crate::tree::{Id, Tree};
use crate::widgets::Placement;

/// Half a caret blink.
const BLINK_MS: u64 = 500;
/// The longest gap between the presses of a double click.
const DOUBLE_CLICK_MS: u64 = 400;
/// How far the pointer may wander between them.
const DOUBLE_CLICK_SLOP: f32 = 4.0;
/// How far a press moves before it is a drag.
pub(crate) const DRAG_SLOP: f32 = 4.0;
/// Room kept between a canvas's caret and the edge of what scrolls it.
const CARET_MARGIN: f32 = 24.0;
/// Which system's key conventions a window follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    /// Linux and Windows: Control moves by word, and Home and End by line.
    Standard,
    /// macOS: Option moves by word, and Command by line.
    Mac,
}

impl Default for Keys {
    /// The conventions of the system this build is for.
    fn default() -> Self {
        if cfg!(target_os = "macos") { Self::Mac } else { Self::Standard }
    }
}

/// What a window shows and does. The app owns all of its state: elements only send it messages.
pub trait App {
    type Msg: Clone + 'static;

    /// The window's name, as assistive technology announces it.
    fn title(&self) -> String;

    /// Describes the whole window for the current state. The app may bring what it draws itself up to date
    /// here, such as a document's layout for the window's width.
    fn view(&mut self, window: Size) -> Element<Self::Msg>;

    fn update(&mut self, message: Self::Msg, platform: &mut dyn Platform);

    /// Offered every key while the page has focus, and otherwise the shortcuts no element used. `true` uses
    /// the key.
    fn key(&mut self, _event: &KeyEvent, _platform: &mut dyn Platform) -> bool {
        false
    }

    /// Fills in the accessibility node of a canvas and adds the nodes under it: what the app draws there
    /// itself, such as a document's blocks.
    ///
    /// With `whole` unset, only what differs from the last time it was described has to be added: the nodes
    /// left out stay as assistive technology knows them.
    fn describe(&self, _canvas: &mut AccessNode, _nodes: &mut Vec<(NodeId, AccessNode)>, _whole: bool) {}

    /// A request from assistive technology for a canvas or a node described under one.
    fn access_action(&mut self, _request: &ActionRequest, _platform: &mut dyn Platform) {}

    /// Told when the window turns light or dark, for what the app draws itself. Elements follow on their own.
    fn scheme(&mut self, _scheme: Scheme) {}

    /// Told which system's key conventions to follow, for the keys the app handles itself. Text fields
    /// follow on their own.
    fn keys(&mut self, _keys: Keys) {}

    /// Text typed while the keyboard is on something that takes none, such as a button.
    fn text(&mut self, _event: InputEvent, _platform: &mut dyn Platform) {}

    /// Told how many device pixels make one logical pixel, which is one until then.
    fn scale(&mut self, _scale: f32) {}

    /// Told whether the person asked their system for less motion. What the app animates then changes at
    /// once.
    fn reduced_motion(&mut self, _reduced: bool) {}

    /// Moves on what the app animates by itself. Returns when to call again, or `None` once nothing moves.
    /// A frame is painted after every call that returns a time, and after the one that ends the motion.
    fn animate(&mut self, _now_ms: u64) -> Option<u64> {
        None
    }

    /// Told whether the window is the one in front, which has the keyboard.
    fn active(&mut self, _active: bool) {}

    /// Told where the pointer rests on the canvas that asked to hear of it ([`Element::pointed`]), relative
    /// to its top left, and `None` when it rests there no longer. It is told on every move with no button
    /// held, so this has to be cheap. `true` says that what the app shows changed with it.
    fn point(&mut self, _at: Option<Point>) -> bool {
        false
    }

    /// What the window's own frame does at a point: the pointer's shape there, and what a press sends. For
    /// a window that draws its frame, whose edge comes before whatever lies under it.
    fn edge(&self, _at: Point, _window: Size) -> Option<(Cursor, Self::Msg)> {
        None
    }

    /// Does a little of any work that was put off, such as laying out text that is out of sight. It is
    /// called while nothing else is happening, until it says there was none.
    fn idle(&mut self) -> Idle {
        Idle::Done
    }
}

/// What a step of an app's idle work came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idle {
    /// There was nothing to do.
    Done,
    /// Something was done that changed nothing in sight.
    Unseen,
    /// Something was done that shows, so the window is described and painted again before the next step.
    Seen,
}

#[derive(Clone, Copy)]
struct Stale {
    view: bool,
    layout: bool,
    paint: bool,
}

#[derive(Clone, Copy)]
struct Press {
    id: Id,
    at: Point,
    time_ms: u64,
    clicks: u8,
}

enum Step {
    Previous,
    Next,
    First,
    Last,
}

pub struct Ui<A: App> {
    app: A,
    shaper: Rc<dyn TextShaper>,
    cache: TextCache,
    tree: Tree<A::Msg>,
    queue: Vec<A::Msg>,
    size: Size,
    scale: f32,
    scheme: Scheme,
    keys: Keys,
    /// Whether the window has the keyboard.
    active: bool,
    stale: Stale,
    pointer: Option<Point>,
    /// The element under the pointer and its ancestors.
    hover: Vec<Id>,
    pressed: Option<Id>,
    last_press: Option<Press>,
    focus: Option<Id>,
    focus_visible: bool,
    /// The anchors of the open popovers, as of the last build.
    layers: Vec<Id>,
    /// For each open popover, where focus was before it opened.
    returns: Vec<(Id, Option<Id>)>,
    /// The elements that ask for focus when they first show, as of the last build.
    asking: Vec<Id>,
    /// The popovers that open and close themselves, by anchor.
    open: HashSet<Id>,
    inputs: HashMap<Id, InputState>,
    scroll: HashMap<Id, f32>,
    /// The focused canvas's caret at the last layout, to scroll after it when it moves.
    canvas_caret: Option<Rect>,
    /// Whether the app may have work for idle time, and whether it did any since the window was described.
    idle: (bool, bool),
    blink_from: Option<u64>,
    /// Whether the app was animating when it was last asked.
    moving: bool,
    caret_on: bool,
    /// The tooltip that is showing.
    tip: Option<Tip>,
    /// Whether the app was last told that the pointer rests on its canvas.
    pointed: bool,
}

impl<A: App> Ui<A> {
    /// Hosts `app`, measuring and drawing its text with `shaper`.
    pub fn host(app: A, shaper: Rc<dyn TextShaper>) -> Self {
        Self {
            app,
            shaper,
            cache: TextCache::default(),
            tree: Tree::default(),
            queue: Vec::new(),
            size: Size::default(),
            scale: 1.0,
            scheme: Scheme::Light,
            keys: Keys::default(),
            active: true,
            stale: Stale { view: true, layout: true, paint: true },
            pointer: None,
            hover: Vec::new(),
            pressed: None,
            last_press: None,
            focus: None,
            focus_visible: false,
            layers: Vec::new(),
            returns: Vec::new(),
            asking: Vec::new(),
            open: HashSet::new(),
            inputs: HashMap::new(),
            scroll: HashMap::new(),
            canvas_caret: None,
            idle: (true, false),
            blink_from: None,
            moving: false,
            caret_on: true,
            tip: None,
            pointed: false,
        }
    }

    /// Shapes every label again before the next frame, for a shaper whose fonts changed.
    pub fn reshape(&mut self) {
        self.cache = TextCache::default();
        self.stale.view = true;
    }

    pub fn app(&self) -> &A {
        &self.app
    }

    /// The app, to change its state from outside. The window is described again afterwards.
    pub fn app_mut(&mut self) -> &mut A {
        self.stale.view = true;
        &mut self.app
    }

    // The window

    /// `size` is in logical pixels. `scale` is the device pixels in one of them, for crisp edges.
    pub fn resize(&mut self, size: Size, scale: f32) {
        if size != self.size || scale != self.scale {
            if scale != self.scale {
                self.app.scale(scale);
            }
            (self.size, self.scale) = (size, scale);
            self.stale.view = true;
        }
    }

    pub fn set_dark(&mut self, dark: bool) {
        let scheme = if dark { Scheme::Dark } else { Scheme::Light };
        if scheme != self.scheme {
            self.scheme = scheme;
            self.app.scheme(scheme);
            self.stale.view = true;
        }
    }

    /// Follows one system's key conventions, in place of those of the system this build is for. A browser
    /// shell picks at run time, and tests pin one so they give the same result on every host.
    pub fn set_keys(&mut self, keys: Keys) {
        self.keys = keys;
        self.app.keys(keys);
    }

    /// Tells the window that the person asked their system for less motion.
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.app.reduced_motion(reduced);
    }

    /// Tells the window whether it has the keyboard. Without it no caret blinks.
    pub fn focus(&mut self, focused: bool) {
        self.active = focused;
        self.pressed = None;
        self.app.active(focused);
        self.stale.view = true;
        self.restart_blink();
    }

    pub fn needs_paint(&self) -> bool {
        self.stale.view || self.stale.layout || self.stale.paint
    }

    pub fn paint(&mut self, out: &mut DisplayList) {
        self.refresh();
        self.update_tip();
        self.stale.paint = false;
        let scene = Scene {
            scheme: self.scheme,
            hover: &self.hover,
            pressed: self.pressed,
            focus: self.focus,
            focus_visible: self.focus_visible,
            caret: self.caret_holder().filter(|_| self.caret_on && self.active).map(|i| self.tree.nodes[i].id),
            inputs: &self.inputs,
            scroll: &self.scroll,
            tip: self.tip.as_ref(),
        };
        paint::paint(&self.tree, &scene, self.window(), out);
    }

    /// Advances what moves on its own: the caret's blink, and the app's idle work. Returns when to call
    /// again, or `None` when nothing is left to move. Idle work waits for a frame that is owed, so paint
    /// between calls: the answer is `now_ms` until that work is done.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        self.refresh();
        let moving = self.app.animate(now_ms);
        // The call that ends a motion changed what shows as well.
        if std::mem::replace(&mut self.moving, moving.is_some()) || moving.is_some() {
            self.stale.view = true;
            self.refresh();
        }
        let sooner = |next: Option<u64>| match (next, moving) {
            (Some(next), Some(moving)) => Some(next.min(moving)),
            (next, moving) => next.or(moving),
        };
        // A frame that is owed comes before work on what is out of sight, which the call after it does.
        if self.idle.0 && self.needs_paint() {
            return Some(now_ms);
        }
        if self.idle.0 {
            let step = self.app.idle();
            if step != Idle::Done {
                self.stale.view |= step == Idle::Seen;
                self.idle.1 |= step == Idle::Unseen;
                return Some(now_ms);
            }
            // The work may have changed what shows, such as how far a page scrolls.
            self.stale.view = std::mem::take(&mut self.idle.1);
            self.idle.0 = false;
            self.refresh();
        }
        let caret = self.caret_holder().is_some_and(|i| match &self.tree.nodes[i].el.content {
            Content::Input(_) => true,
            Content::Canvas(canvas) => canvas.caret.is_some(),
            _ => false,
        });
        if !self.active || !caret {
            return sooner(None);
        }
        // A caret that has just moved shows solid from the next frame.
        let from = *self.blink_from.get_or_insert(now_ms);
        let phase = now_ms.saturating_sub(from) / BLINK_MS;
        let on = phase.is_multiple_of(2);
        self.stale.paint |= on != self.caret_on;
        self.caret_on = on;
        sooner(Some(from + (phase + 1) * BLINK_MS))
    }

    /// Whether what is typed goes to the app's own canvas, such as the page, and not to a field of the
    /// window, as last laid out. A shell that shows a keyboard the text around the caret asks this first.
    pub fn types_in_canvas(&self) -> bool {
        self.caret_holder().is_some_and(|i| matches!(self.tree.nodes[i].el.content, Content::Canvas(_)))
    }

    /// Where the app's own canvas is in the window as last laid out: the one focus rests on, such as the
    /// page.
    pub(crate) fn canvas(&self) -> Option<Rect> {
        let own = self.tree.nodes.iter().find(|node| node.el.rest && matches!(node.el.content, Content::Canvas(_)));
        own.map(|node| inner(node.rect, &node.el.style))
    }

    /// A point of the window as the app's own canvas counts it, when a press there would be the canvas's:
    /// nothing lies over it at that point.
    pub(crate) fn on_canvas(&self, at: Point) -> Option<Point> {
        let i = self.hit(at).and_then(|hit| self.target(hit))?;
        let node = &self.tree.nodes[i];
        (node.el.rest && matches!(node.el.content, Content::Canvas(_))).then(|| self.within(i, at))
    }

    /// Where the caret is in the window as last laid out, for the input method's candidate window.
    pub fn caret_rect(&self) -> Option<Rect> {
        let node = &self.tree.nodes[self.caret_holder()?];
        let content = inner(node.rect, &node.el.style);
        match &node.el.content {
            Content::Input(input) => {
                let shaped = node.shaped.as_ref()?;
                let state = self.inputs.get(&node.id).cloned().unwrap_or_default();
                let x = content.x - state.scroll + shaped.x_at(state.caret(&input.value));
                let y = content.y + (content.height - shaped.size.height) / 2.0;
                Some(Rect::new(x, y, 1.0, shaped.size.height))
            }
            Content::Canvas(canvas) => {
                let caret = canvas.caret?;
                Some(Rect { x: content.x + caret.x, y: content.y + caret.y, ..caret })
            }
            _ => None,
        }
    }

    /// The pointer shape for what is under the pointer.
    pub fn cursor(&self) -> Cursor {
        let edge = self.pointer.filter(|_| self.pressed.is_none()).and_then(|at| self.app.edge(at, self.size));
        if let Some((cursor, _)) = edge {
            return cursor;
        }
        // A drag keeps the shape of what it started on.
        let held = self.pressed.iter().chain(&self.hover);
        let nodes = held.filter_map(|&id| self.tree.find(id)).filter(|&i| !self.tree.disabled(i));
        nodes.map(|i| self.tree.nodes[i].el.cursor).find(|&cursor| cursor != Cursor::Default).unwrap_or_default()
    }

    // Accessibility

    /// The whole accessibility tree.
    pub fn access(&mut self) -> TreeUpdate {
        self.described(true)
    }

    /// The accessibility tree as an update for whoever was last given it, whole or as changes: of what the
    /// app describes under a canvas, such as a document's blocks, only what differs since then. On a long
    /// document this is far less to make, and to read, than the whole tree.
    pub fn access_changes(&mut self) -> TreeUpdate {
        self.described(false)
    }

    fn described(&mut self, whole: bool) -> TreeUpdate {
        self.refresh();
        self.update_tip();
        let title = self.app.title();
        let window = access::Window {
            title: &title,
            rect: self.window(),
            scale: self.scale,
            focus: self.focus,
            inputs: &self.inputs,
            tip: self.tip.as_ref().map(|tip| (tip.text.as_str(), tip.rect)),
            describe: &|node, nodes| self.app.describe(node, nodes, whole),
        };
        access::build(&self.tree, &window)
    }

    /// Does what assistive technology asked of a node.
    pub fn access_action(&mut self, request: ActionRequest, platform: &mut dyn Platform) {
        self.refresh();
        let id = Id(request.target_node.0);
        // What a canvas holds is the app's, and so is anything asked of it beyond a click or focus.
        let found = self.tree.find(id);
        let canvas = found.is_some_and(|i| matches!(self.tree.nodes[i].el.content, Content::Canvas(_)));
        let (Some(i), true) = (found, !canvas || matches!(request.action, Action::Click | Action::Focus)) else {
            self.app.access_action(&request, platform);
            self.stale.view = true;
            return;
        };
        match (request.action, request.data) {
            (Action::Click, _) => {
                if self.tree.nodes[i].el.focus == Focus::Click {
                    self.set_focus(Some(id), true);
                }
                self.activate(i);
            }
            (Action::Focus, _) if self.tree.focusable(i) => self.set_focus(Some(id), true),
            (Action::SetValue, Some(ActionData::Value(text))) => {
                if let Content::Input(input) = &self.tree.nodes[i].el.content {
                    self.inputs.entry(id).or_default().select_all(&input.value);
                }
                self.input_text(i, InputEvent::Insert(text.into()));
            }
            (Action::ReplaceSelectedText, Some(ActionData::Value(text))) => {
                self.input_text(i, InputEvent::Insert(text.into()));
            }
            (Action::SetTextSelection, Some(ActionData::SetTextSelection(selection))) => {
                let Some(shaped) = self.tree.nodes[i].shaped.clone() else { return };
                let offset = |position| access::offset_of(id, &shaped, position);
                if let (Some(anchor), Some(head)) = (offset(&selection.anchor), offset(&selection.focus)) {
                    let state = self.inputs.entry(id).or_default();
                    (state.anchor, state.head) = (anchor, head);
                    self.restart_blink();
                }
            }
            _ => {}
        }
        self.flush(platform);
    }

    // The pointer

    pub fn pointer_move(&mut self, at: Point, platform: &mut dyn Platform) {
        self.refresh();
        self.pointer = Some(at);
        if let Some(i) = self.pressed.and_then(|id| self.tree.find(id)) {
            let node = &self.tree.nodes[i];
            match &node.el.content {
                Content::Input(input) => {
                    let offset = self.offset_at(i, at);
                    self.inputs.entry(node.id).or_default().place(&input.value, offset, true);
                    self.restart_blink();
                }
                Content::Canvas(canvas) => {
                    let event = Pointer::Drag { at: self.within(i, at) };
                    self.queue.extend(canvas.on_pointer.as_ref().and_then(|handler| handler(event)));
                }
                // A press that moves is a drag for what can be dragged, and a click no more.
                _ => {
                    let from = self.last_press.map_or(at, |press| press.at);
                    let moved = (at.x - from.x).abs() > DRAG_SLOP || (at.y - from.y).abs() > DRAG_SLOP;
                    if let Some(message) = node.el.on_drag.clone().filter(|_| moved) {
                        self.queue.push(message);
                        self.pressed = None;
                        self.stale.paint = true;
                    }
                }
            }
        }
        self.update_hover();
        self.flush(platform);
        self.update_pointed();
    }

    pub fn pointer_down(&mut self, at: Point, mods: Modifiers, platform: &mut dyn Platform) {
        self.refresh();
        self.pointer = Some(at);
        if let Some((_, message)) = self.app.edge(at, self.size) {
            self.queue.push(message);
            return self.flush(platform);
        }
        let blocked = self.dismiss(self.hit(at));
        self.flush(platform);
        self.refresh();
        if blocked {
            return;
        }

        let Some(i) = self.hit(at).and_then(|hit| self.target(hit)).filter(|&i| !self.tree.disabled(i)) else { return };
        let id = self.tree.nodes[i].id;
        let now = platform.now_ms();
        let clicks = match self.last_press {
            Some(last)
                if last.id == id
                    && now.saturating_sub(last.time_ms) <= DOUBLE_CLICK_MS
                    && (last.at.x - at.x).abs() <= DOUBLE_CLICK_SLOP
                    && (last.at.y - at.y).abs() <= DOUBLE_CLICK_SLOP =>
            {
                last.clicks.saturating_add(1)
            }
            _ => 1,
        };
        self.last_press = Some(Press { id, at, time_ms: now, clicks });
        self.pressed = Some(id);
        self.stale.paint = true;
        if self.tree.nodes[i].el.focus == Focus::Click {
            self.set_focus(Some(id), false);
        }

        let node = &self.tree.nodes[i];
        if clicks == 2 {
            self.queue.extend(node.el.on_double.clone());
        }
        match &node.el.content {
            Content::Input(input) => {
                let offset = self.offset_at(i, at);
                let state = self.inputs.entry(id).or_default();
                match clicks {
                    1 => state.place(&input.value, offset, mods.shift),
                    2 => state.select_word(&input.value, offset),
                    _ => state.select_all(&input.value),
                }
                self.restart_blink();
            }
            Content::Canvas(canvas) => {
                let event = Pointer::Down { at: self.within(i, at), mods, clicks };
                self.queue.extend(canvas.on_pointer.as_ref().and_then(|handler| handler(event)));
            }
            _ => {}
        }
        self.flush(platform);
    }

    /// The secondary button went down, which asks for a menu about what is under the pointer.
    pub fn secondary_down(&mut self, at: Point, platform: &mut dyn Platform) {
        self.refresh();
        self.pointer = Some(at);
        let blocked = self.dismiss(self.hit(at));
        self.flush(platform);
        self.refresh();
        if blocked {
            return;
        }
        let Some(i) = self.hit(at).and_then(|hit| self.target(hit)).filter(|&i| !self.tree.disabled(i)) else { return };
        if self.tree.nodes[i].el.focus == Focus::Click {
            self.set_focus(Some(self.tree.nodes[i].id), false);
        }
        if let Some(handler) = &self.tree.nodes[i].el.on_menu {
            self.queue.push(handler(at));
            return self.flush(platform);
        }
        self.ask_menu(i, self.within(i, at), platform);
    }

    /// The Menu key or Shift+F10, which asks for a menu about what is at the caret.
    pub fn menu_key(&mut self, platform: &mut dyn Platform) {
        self.refresh();
        let Some(i) = self.focused() else { return };
        let Content::Canvas(canvas) = &self.tree.nodes[i].el.content else { return };
        let Some(caret) = canvas.caret else { return };
        self.ask_menu(i, Point { x: caret.x, y: caret.y + caret.height / 2.0 }, platform);
    }

    fn ask_menu(&mut self, i: usize, at: Point, platform: &mut dyn Platform) {
        if let Content::Canvas(canvas) = &self.tree.nodes[i].el.content {
            self.queue.extend(canvas.on_pointer.as_ref().and_then(|handler| handler(Pointer::Menu { at })));
        }
        self.flush(platform);
    }

    pub fn pointer_up(&mut self, at: Point, platform: &mut dyn Platform) {
        self.refresh();
        self.pointer = Some(at);
        self.update_hover();
        if let Some(id) = self.pressed.take() {
            self.stale.paint = true;
            if let Some(i) = self.tree.find(id) {
                match &self.tree.nodes[i].el.content {
                    Content::Canvas(canvas) => {
                        let event = Pointer::Up { at: self.within(i, at) };
                        self.queue.extend(canvas.on_pointer.as_ref().and_then(|handler| handler(event)));
                    }
                    Content::Input(_) => {}
                    // A press counts when it is let go over the element it started on.
                    _ if self.hover.contains(&id) => self.activate(i),
                    _ => {}
                }
            }
        }
        self.flush(platform);
        self.update_pointed();
    }

    pub fn pointer_leave(&mut self) {
        self.pointer = None;
        self.update_hover();
        self.update_pointed();
    }

    /// `delta` is in logical pixels. A positive `y` moves the content up.
    pub fn scroll(&mut self, at: Point, delta: Point, platform: &mut dyn Platform) {
        self.refresh();
        let Some(hit) = self.hit(at) else { return };
        if !self.scroll_from(hit, delta) {
            // An overlay with nothing to scroll lets the wheel through to what it covers.
            let under = self.tree.layers.iter().rev().find_map(|layer| self.hit_in(layer.root, at, true));
            if let Some(under) = under.filter(|&under| under != hit) {
                self.scroll_from(under, delta);
            }
        }
        self.flush(platform);
    }

    /// Offers a scroll to an element and then to what it sits in. The innermost one that scrolls and has
    /// room to move takes it.
    fn scroll_from(&mut self, hit: usize, delta: Point) -> bool {
        for i in self.tree.ancestors(hit).collect::<Vec<_>>() {
            let handler = match &self.tree.nodes[i].el.content {
                Content::Canvas(canvas) => canvas.on_pointer.as_ref(),
                _ => None,
            };
            if let Some(message) = handler.and_then(|handler| handler(Pointer::Scroll { delta })) {
                self.queue.push(message);
                return true;
            }
            if self.scroll_by(i, delta.y) {
                return true;
            }
        }
        false
    }

    // The keyboard

    /// Offers a key to the focused element, then to the window and the app. `false` means nothing used it.
    pub fn key(&mut self, event: KeyEvent, platform: &mut dyn Platform) -> bool {
        self.refresh();
        let used = self.route(&event, platform);
        self.flush(platform);
        used
    }

    /// Text from the keyboard or an input method, for whatever has focus.
    pub fn input(&mut self, event: InputEvent, platform: &mut dyn Platform) {
        self.refresh();
        let Some(i) = self.focused() else { return };
        match &self.tree.nodes[i].el.content {
            Content::Input(_) => self.input_text(i, event),
            Content::Canvas(canvas) => {
                self.queue.extend(canvas.on_text.as_ref().map(|handler| handler(event)));
                // What is typed shows the caret, even where the text scrolls under it and it keeps its place.
                self.restart_blink();
            }
            // Space presses a focused button. It arrives as text, like every other character.
            _ if matches!(&event, InputEvent::Insert(text) | InputEvent::Commit(text) if text == " ") => {
                self.activate(i)
            }
            _ => {
                self.app.text(event, platform);
                self.stale.view = true;
            }
        }
        self.flush(platform);
    }

    fn route(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let focused = self.focused();
        let plain = !event.mods.primary && !event.mods.alt;
        if event.key == Key::Escape {
            // A field drops what was typed into it, and then Escape closes the topmost popover before it
            // means anything else.
            let cancel = |i: usize| match &self.tree.nodes[i].el.content {
                Content::Input(input) => input.on_cancel.clone(),
                _ => None,
            };
            // In a popover beside a field, Escape means what it means in the field.
            let dropped = focused.and_then(|i| {
                let layer = &self.tree.layers[self.tree.nodes[i].layer];
                cancel(i).or_else(|| layer.anchor.filter(|_| layer.beside).and_then(cancel))
            });
            let cancelled = dropped.is_some();
            self.queue.extend(dropped);
            if let Some(top) = self.tree.top_layer() {
                self.close_layer(top);
                return true;
            }
            if cancelled {
                self.set_focus(self.rest(), false);
                self.stale.view = true;
                return true;
            }
        }
        let on_page = focused.is_none_or(|i| matches!(self.tree.nodes[i].el.content, Content::Canvas(_)));
        if on_page && self.offer(event, platform) {
            self.restart_blink();
            return true;
        }
        if let Some(i) = focused {
            if self.input_key(i, event, platform) {
                return true;
            }
            let node = &self.tree.nodes[i];
            if event.key == Key::Enter && plain && (node.el.on_press.is_some() || node.trigger.is_some()) {
                self.activate(i);
                return true;
            }
            // What the element did not use is offered to each element it sits in.
            for at in self.tree.ancestors(i).collect::<Vec<_>>() {
                let el = &self.tree.nodes[at].el;
                if let Some(message) = el.on_key.as_ref().and_then(|handler| handler(event)) {
                    self.queue.push(message);
                    return true;
                }
                if let Some(step) = el.arrows.filter(|_| plain).and_then(|dir| step(dir, &event.key)) {
                    self.rove(at, i, step);
                    return true;
                }
            }
        }
        let used = match event.key {
            Key::Escape => self.escape(),
            Key::Tab if plain => self.tab(event.mods.shift),
            Key::PageUp | Key::PageDown => self.page(event.key == Key::PageDown),
            _ => false,
        };
        // Shortcuts reach the app wherever focus is.
        used || (!on_page && event.mods.primary && self.offer(event, platform))
    }

    fn offer(&mut self, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let used = self.app.key(event, platform);
        self.stale.view |= used;
        used
    }

    /// Hands focus back to the page.
    fn escape(&mut self) -> bool {
        let rest = self.rest();
        let away = rest.is_some() && rest != self.focus;
        if away {
            self.set_focus(rest, false);
        }
        away
    }

    /// Moves focus to the next or previous element in the layer that holds it. A modal layer keeps it in.
    fn tab(&mut self, back: bool) -> bool {
        let focused = self.focused();
        let modal = self.tree.modal_layer();
        let held = focused.map(|i| self.tree.nodes[i].layer).filter(|&layer| modal.is_none_or(|modal| layer >= modal));
        let layer = &self.tree.layers[held.or(modal).unwrap_or(0)];
        // A popover beside its anchor is one round with it.
        let beside = self.tree.layers.iter().find(|layer| {
            layer.beside && focused.is_some_and(|i| layer.anchor == Some(i) || (layer.root..layer.end).contains(&i))
        });
        let inside = beside.map(|layer| (layer.anchor, self.tab_stops(layer.root..layer.end, focused)));
        let stops: Vec<usize> = match inside {
            // One with nothing to stop at leaves Tab to the window.
            Some((anchor, inside)) if !inside.is_empty() => anchor.into_iter().chain(inside).collect(),
            _ => self.tab_stops(layer.root..layer.end, focused),
        };
        let at = focused.and_then(|i| stops.iter().position(|&stop| stop == i));
        let next = match (at, back) {
            (Some(at), false) => (at + 1) % stops.len(),
            (Some(at), true) => (at + stops.len() - 1) % stops.len(),
            (None, false) => 0,
            (None, true) => stops.len().saturating_sub(1),
        };
        if let Some(&stop) = stops.get(next) {
            self.set_focus(Some(self.tree.nodes[stop].id), true);
        }
        true
    }

    /// The elements Tab stops at. A group that is one stop gives the element in it that has focus, or else
    /// its first.
    fn tab_stops(&self, range: std::ops::Range<usize>, focused: Option<usize>) -> Vec<usize> {
        let group = |i: usize| self.tree.ancestors(i).skip(1).find(|&at| self.tree.nodes[at].el.one_stop);
        let within = focused.and_then(group);
        let mut entered = HashSet::new();
        let stop = |i: &usize| match group(*i) {
            None => true,
            Some(group) if Some(group) == within => Some(*i) == focused,
            Some(group) => entered.insert(group),
        };
        self.tree.focusables(range).into_iter().filter(stop).collect()
    }

    /// Moves focus among the focusable elements inside `container`, wrapping at the ends.
    fn rove(&mut self, container: usize, from: usize, step: Step) {
        let stops = self.tree.focusables(container + 1..self.tree.nodes[container].end);
        let mut at = stops.iter().position(|&stop| stop == from);
        if !self.focus_visible {
            self.focus_visible = true;
            self.stale.paint = true;
            if self.tree.nodes[container].layer > 0 {
                // In a popover, focus that does not show was put there as it opened. To the person nothing
                // is focused yet, so the key goes to the end it names or points away from.
                at = None;
            } else if matches!(step, Step::Next | Step::Previous) {
                // After the pointer, the first arrow key only shows where focus is.
                return;
            }
        }
        let next = match (step, at) {
            (Step::First, _) | (Step::Next, None) => 0,
            (Step::Last, _) | (Step::Previous, None) => stops.len().saturating_sub(1),
            (Step::Next, Some(at)) => (at + 1) % stops.len(),
            (Step::Previous, Some(at)) => (at + stops.len() - 1) % stops.len(),
        };
        if let Some(&stop) = stops.get(next) {
            self.set_focus(Some(self.tree.nodes[stop].id), true);
        }
    }

    /// Scrolls what holds focus by most of its height.
    fn page(&mut self, down: bool) -> bool {
        let scrolls = |i: &usize| self.tree.nodes[*i].el.style.scroll;
        let scroller = match self.focused() {
            Some(i) => self.tree.ancestors(i).find(scrolls),
            None => (0..self.tree.nodes.len()).find(scrolls),
        };
        let Some(i) = scroller else { return false };
        let node = &self.tree.nodes[i];
        let step = inner(node.rect, &node.el.style).height * 0.9;
        self.scroll_by(i, if down { step } else { -step });
        true
    }

    /// The keys of a one-line text input.
    fn input_key(&mut self, i: usize, event: &KeyEvent, platform: &mut dyn Platform) -> bool {
        let node = &self.tree.nodes[i];
        let (Content::Input(input), Some(shaped)) = (&node.el.content, &node.shaped) else { return false };
        let state = self.inputs.entry(node.id).or_default();
        let value = input.value.as_str();
        let mods = event.mods;
        let mac = self.keys == Keys::Mac;
        let by_line = mac && mods.primary;
        let by_word = if mac { mods.alt } else { mods.primary };

        let motion = match event.key {
            Key::Home => Some(Motion::Start),
            Key::End => Some(Motion::End),
            Key::Left | Key::Backspace if by_line => Some(Motion::Start),
            Key::Right | Key::Delete if by_line => Some(Motion::End),
            Key::Left | Key::Backspace if by_word => Some(Motion::WordLeft),
            Key::Right | Key::Delete if by_word => Some(Motion::WordRight),
            Key::Left | Key::Backspace => Some(Motion::Left),
            Key::Right | Key::Delete => Some(Motion::Right),
            _ => None,
        };
        let edited = match (&event.key, motion) {
            (Key::Backspace | Key::Delete, Some(motion)) => state.delete(value, shaped, motion),
            (_, Some(motion)) => {
                state.step(value, shaped, motion, mods.shift);
                None
            }
            (Key::Char('a'), _) if mods.primary => {
                state.select_all(value);
                None
            }
            (Key::Char('c' | 'x'), _) if mods.primary => {
                let selected = state.selected(value);
                if !selected.is_empty() {
                    platform.clipboard_write(Clip { text: selected.to_owned(), ..Clip::default() });
                }
                (event.key == Key::Char('x') && !selected.is_empty()).then(|| state.insert(value, ""))
            }
            (Key::Char('v'), _) if mods.primary => {
                platform.clipboard_read().map(|clip| state.insert(value, &clip.text))
            }
            // A field keeps no history of its own, and the document's must not move under it.
            (Key::Char('z' | 'y'), _) if mods.primary => None,
            (Key::Enter, _) if !mods.primary && !mods.alt => {
                let Some(message) = input.on_submit.clone() else { return false };
                self.queue.push(message);
                return true;
            }
            _ => return false,
        };
        self.queue.extend(edited.and_then(|value| input.on_input.as_ref().map(|handler| handler(value))));
        self.restart_blink();
        self.stale.layout = true;
        true
    }

    /// Text arriving at a text input.
    fn input_text(&mut self, i: usize, event: InputEvent) {
        let node = &self.tree.nodes[i];
        let Content::Input(input) = &node.el.content else { return };
        let Some(on_input) = &input.on_input else { return };
        let state = self.inputs.entry(node.id).or_default();
        let value = input.value.as_str();
        let edited = match event {
            InputEvent::Insert(text) | InputEvent::Commit(text) => {
                state.composing = None;
                (!text.is_empty() || !state.selected(value).is_empty()).then(|| state.insert(value, &text))
            }
            InputEvent::ReplaceRange { range, text } => Some(state.replace(value, range, &text)),
            InputEvent::SetComposition { text, .. } => {
                // Composing over a selection replaces it.
                let cleared = (!text.is_empty() && !state.selected(value).is_empty()).then(|| state.insert(value, ""));
                state.composing = (!text.is_empty()).then_some(text);
                cleared
            }
            InputEvent::DeleteSurrounding { before, after } => state.delete_around(value, before, after),
        };
        self.queue.extend(edited.map(|value| on_input(value)));
        self.restart_blink();
        self.stale.layout = true;
    }

    // Elements

    fn window(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.width, self.size.height)
    }

    fn focused(&self) -> Option<usize> {
        self.focus.and_then(|id| self.tree.find(id))
    }

    /// The element focus rests on when nothing else holds it.
    fn rest(&self) -> Option<Id> {
        self.tree.nodes.iter().find(|node| node.el.rest).map(|node| node.id)
    }

    /// The topmost element at a point. A modal popover hides everything beneath it.
    fn hit(&self, at: Point) -> Option<usize> {
        for layer in self.tree.layers.iter().rev() {
            let hit = self.hit_in(layer.root, at, false);
            if hit.is_some() || layer.modal {
                return hit;
            }
        }
        None
    }

    /// The topmost element at a point within `i`. `under` looks beneath overlays.
    fn hit_in(&self, i: usize, at: Point, under: bool) -> Option<usize> {
        let node = &self.tree.nodes[i];
        let inside = node.rect.contains(at);
        if (node.el.style.clip && !inside) || (under && node.el.style.overlay) {
            return None;
        }
        // Later children are painted over earlier ones.
        let child = self.tree.children(i).filter_map(|child| self.hit_in(child, at, under)).last();
        child.or(inside.then_some(i))
    }

    /// Works out which tooltip shows: the one of the element under the pointer, or else of the element the
    /// keyboard's focus is on, or else of one that has news to tell.
    fn update_tip(&mut self) {
        let has = |&i: &usize| self.tree.nodes[i].el.tooltip.is_some() && !self.tree.disabled(i);
        let hovered = self.hover.iter().filter_map(|id| self.tree.find(*id)).find(has);
        let focused = self.focused().filter(|_| self.focus_visible).filter(has);
        let telling =
            || (0..self.tree.nodes.len()).filter(|&i| self.tree.nodes[i].el.telling && !self.tree.gone(i)).find(has);
        let Some(node) = hovered.or(focused).or_else(telling).map(|i| &self.tree.nodes[i]) else {
            self.tip = None;
            return;
        };
        let Some((text, side)) = &node.el.tooltip else { return };
        let shaped = match self.tip.take() {
            Some(tip) if tip.text == *text => tip.shaped,
            _ => Rc::new(self.shaper.shape(text, &Tip::TEXT, Some(Tip::WIDTH))),
        };
        let placement = Placement { side: *side, align: Align::Center, gap: Tip::GAP, at: None };
        let rect = position(placement, node.rect, Tip::size(&shaped), self.size);
        self.tip = Some(Tip { text: text.clone(), shaped, rect });
    }

    /// The element a press at `hit` is meant for: the nearest one that does something with it.
    fn target(&self, hit: usize) -> Option<usize> {
        self.tree.ancestors(hit).find(|&i| {
            let node = &self.tree.nodes[i];
            // A canvas that is only a picture leaves the pointer to what it is drawn in.
            let takes_input = match &node.el.content {
                Content::Input(_) => true,
                Content::Canvas(canvas) => canvas.on_pointer.is_some(),
                _ => false,
            };
            let el = &node.el;
            let pointed = el.on_drag.is_some() || el.on_double.is_some() || el.on_menu.is_some();
            takes_input || pointed || el.on_press.is_some() || node.trigger.is_some() || el.focus != Focus::No
        })
    }

    /// A window position relative to an element's content box.
    fn within(&self, i: usize, at: Point) -> Point {
        let node = &self.tree.nodes[i];
        let content = inner(node.rect, &node.el.style);
        Point { x: at.x - content.x, y: at.y - content.y }
    }

    /// The offset in a text input's value nearest to a window position.
    fn offset_at(&self, i: usize, at: Point) -> usize {
        let node = &self.tree.nodes[i];
        let scroll = self.inputs.get(&node.id).map_or(0.0, |state| state.scroll);
        node.shaped.as_ref().map_or(0, |shaped| shaped.offset_at(self.within(i, at).x + scroll))
    }

    fn update_hover(&mut self) {
        let hit = self.pointer.and_then(|at| self.hit(at));
        let hover: Vec<Id> =
            hit.into_iter().flat_map(|i| self.tree.ancestors(i)).map(|i| self.tree.nodes[i].id).collect();
        if hover != self.hover {
            self.hover = hover;
            self.stale.paint = true;
        }
    }

    /// Tells the app where the pointer rests on the canvas that asked to hear of it, or that it has left.
    /// A held button keeps the app's last word: a drag is the canvas's own to follow.
    fn update_pointed(&mut self) {
        if self.pressed.is_some() {
            return;
        }
        // What the app does with a message may have moved what is under the pointer.
        self.refresh();
        let rests = |i: &usize| matches!(&self.tree.nodes[*i].el.content, Content::Canvas(canvas) if canvas.pointed);
        let over = self.pointer.and_then(|at| Some((self.hit(at).filter(rests)?, at)));
        let at = over.map(|(i, at)| self.within(i, at));
        if (at.is_some() || self.pointed) && self.app.point(at) {
            // The pointer's shape is read before the next frame, and may be part of what changed.
            self.stale.view = true;
            self.refresh();
        }
        self.pointed = at.is_some();
    }

    /// Presses an element: sends its message, toggles its popover, and closes the popover it is in if it
    /// asks for that.
    fn activate(&mut self, i: usize) {
        if self.tree.disabled(i) {
            return;
        }
        let node = &self.tree.nodes[i];
        self.queue.extend(node.el.on_press.clone());
        // A popover the app controls opens and closes through the message.
        if node.trigger.is_some_and(|trigger| !trigger.controlled) && !self.open.remove(&node.id) {
            self.open.insert(node.id);
        }
        self.stale.view = true;
        if node.el.closes && node.layer > 0 {
            self.close_layer(node.layer);
        }
    }

    /// Tells a field that focus is leaving it.
    fn leave(&mut self, i: usize) {
        if let Content::Input(input) = &self.tree.nodes[i].el.content {
            self.queue.extend(input.on_leave.clone());
            self.stale.view |= input.on_leave.is_some();
        }
    }

    fn close_layer(&mut self, layer: usize) {
        if let Some(focus) = self.focused().filter(|&i| self.tree.in_layer(i, layer)) {
            self.leave(focus);
        }
        let layer = &self.tree.layers[layer];
        if layer.controlled {
            self.queue.extend(layer.on_close.clone());
        } else if let Some(anchor) = layer.anchor {
            self.open.remove(&self.tree.nodes[anchor].id);
        }
        self.stale.view = true;
    }

    /// Closes a popover a press landed outside of, unless it is one that stays.
    fn press_outside(&mut self, layer: usize) {
        if !self.tree.layers[layer].stays {
            self.close_layer(layer);
        }
    }

    /// Closes the popovers a press landed outside of. Returns whether the press ends there, as one on a
    /// modal popover's backdrop does.
    fn dismiss(&mut self, hit: Option<usize>) -> bool {
        let Some(top) = self.tree.top_layer() else { return false };
        let Some(hit) = hit else {
            if self.tree.modal_layer().is_some() {
                self.close_layer(top);
                return true;
            }
            (1..=top).rev().for_each(|layer| self.press_outside(layer));
            return false;
        };
        for layer in (1..=top).rev() {
            let Layer { anchor, placement, .. } = &self.tree.layers[layer];
            // A press on the anchor is left to toggle its own popover.
            let on_anchor = placement.at.is_none() && self.tree.ancestors(hit).any(|i| Some(i) == *anchor);
            if on_anchor || self.tree.in_layer(hit, layer) {
                break;
            }
            self.press_outside(layer);
        }
        false
    }

    fn set_focus(&mut self, id: Option<Id>, keyboard: bool) {
        self.focus_visible = keyboard;
        self.stale.paint = true;
        if id == self.focus {
            return;
        }
        // Text an input method had not finished does not survive leaving the field.
        if let Some(state) = self.focus.and_then(|old| self.inputs.get_mut(&old)) {
            state.composing = None;
            self.stale.layout = true;
        }
        if let Some(old) = self.focused() {
            self.leave(old);
        }
        self.focus = id;
        self.restart_blink();
        let Some(i) = id.and_then(|id| self.tree.find(id)) else { return };
        let node = &self.tree.nodes[i];
        if let (Content::Input(input), true) = (&node.el.content, keyboard) {
            self.inputs.entry(node.id).or_default().select_all(&input.value);
        }
        self.reveal(i, node.rect, 0.0);
    }

    /// Scrolls `rect`, which lies inside element `i`, into view.
    fn reveal(&mut self, i: usize, rect: Rect, margin: f32) {
        for at in self.tree.ancestors(i).skip(1) {
            let node = &self.tree.nodes[at];
            if !node.el.style.scroll {
                continue;
            }
            let view = inner(node.rect, &node.el.style);
            let margin = margin.min(view.height / 4.0);
            let (top, bottom) = (rect.y - margin, rect.y + rect.height + margin);
            let (view_top, view_bottom) = (view.y, view.y + view.height);
            let by = if top < view_top {
                top - view_top
            } else if bottom > view_bottom {
                // Something taller than the view shows its top.
                (bottom - view_bottom).min(top - view_top)
            } else {
                0.0
            };
            if by != 0.0 {
                *self.scroll.entry(node.id).or_insert(0.0) += by;
                self.stale.layout = true;
            }
        }
    }

    /// Scrolls element `i` if it scrolls and has room to. Returns whether it moved.
    fn scroll_by(&mut self, i: usize, delta: f32) -> bool {
        let node = &self.tree.nodes[i];
        if !node.el.style.scroll {
            return false;
        }
        let room = (node.content_height - inner(node.rect, &node.el.style).height).max(0.0);
        let offset = self.scroll.entry(node.id).or_insert(0.0);
        let next = (*offset + delta).clamp(0.0, room);
        let moved = next != *offset;
        *offset = next;
        self.stale.layout |= moved;
        moved
    }

    fn restart_blink(&mut self) {
        self.blink_from = None;
        self.caret_on = true;
        self.stale.paint = true;
    }

    // Upkeep

    /// Hands the app the messages its elements sent.
    fn flush(&mut self, platform: &mut dyn Platform) {
        for message in std::mem::take(&mut self.queue) {
            self.app.update(message, platform);
            self.stale.view = true;
        }
    }

    /// Brings the tree and its layout up to date with the app.
    fn refresh(&mut self) {
        if self.stale.view {
            self.stale = Stale { view: false, layout: true, paint: true };
            self.idle.0 = true;
            let open = &self.open;
            self.tree = Tree::build(self.app.view(self.size), |id| open.contains(&id));
            self.settle();
        }
        if self.stale.layout {
            self.stale.layout = false;
            self.stale.paint = true;
            self.layout();
            // A caret that moved is scrolled into view, once the layout says where it is.
            if self.follow_caret() {
                self.stale.layout = false;
                self.layout();
            }
            if self.fold() {
                // A popover hung from something that is no longer shown: build the window without it.
                self.stale.view = true;
                return self.refresh();
            }
            self.update_hover();
        }
    }

    /// Takes focus and popovers away from what the layout left out, as a bar folding for a narrower window.
    /// Returns whether a popover was closed.
    fn fold(&mut self) -> bool {
        if self.focused().is_some_and(|i| self.tree.gone(i)) {
            self.focus = self.rest();
        }
        let tree = &self.tree;
        let before = self.open.len();
        self.open.retain(|id| !tree.find(*id).is_some_and(|i| tree.gone(i)));
        self.open.len() < before
    }

    fn layout(&mut self) {
        let inputs = &self.inputs;
        let shown = |id: Id, value: &str| inputs.get(&id).map_or_else(|| value.to_owned(), |state| state.shown(value));
        let mut layout = Layout {
            shaper: &*self.shaper,
            cache: &mut self.cache,
            window: self.size,
            scale: self.scale,
            scroll: &mut self.scroll,
            inputs: &shown,
        };
        layout.run(&mut self.tree);
    }

    /// The element whose caret shows: the one with focus, or a canvas that holds a caret for it in a popover
    /// beside it.
    fn caret_holder(&self) -> Option<usize> {
        let focused = self.focused()?;
        let holds =
            |i: &usize| matches!(&self.tree.nodes[*i].el.content, Content::Canvas(canvas) if canvas.caret.is_some());
        let beside = self.tree.layers.iter().filter(|layer| layer.beside && layer.anchor == Some(focused));
        beside.flat_map(|layer| layer.root..layer.end).find(holds).or(Some(focused))
    }

    /// Keeps the focused element's caret in view. Returns whether anything scrolled.
    fn follow_caret(&mut self) -> bool {
        let Some(i) = self.caret_holder() else { return false };
        let node = &self.tree.nodes[i];
        let content = inner(node.rect, &node.el.style);
        match &node.el.content {
            Content::Input(input) => {
                let (Some(shaped), Some(state)) = (&node.shaped, self.inputs.get_mut(&node.id)) else { return false };
                state.reveal(&input.value, shaped, content.width);
                false
            }
            Content::Canvas(canvas) => {
                let moved = canvas.caret != self.canvas_caret;
                self.canvas_caret = canvas.caret;
                let Some(caret) = canvas.caret.filter(|_| moved) else { return false };
                self.restart_blink();
                self.reveal(i, Rect { x: content.x + caret.x, y: content.y + caret.y, ..caret }, CARET_MARGIN);
                self.stale.layout
            }
            _ => false,
        }
    }

    /// Carries focus, popovers and per-element state over to a newly built tree.
    fn settle(&mut self) {
        let tree = &self.tree;
        let anchors: Vec<Id> = tree.layers.iter().filter_map(|layer| layer.anchor).map(|i| tree.nodes[i].id).collect();
        let mut focus = self.focus;

        // A popover that closed gives focus back to where it was.
        let mut restored = None;
        self.returns.retain(|(anchor, before)| {
            let open = anchors.contains(anchor);
            if !open {
                restored.get_or_insert(*before);
            }
            open
        });
        if let Some(before) = restored {
            focus = before;
        }
        // One that opened takes it, as does an element that asks for it when it first shows.
        let mut took = Vec::new();
        for (layer, anchor) in tree.layers.iter().skip(1).zip(&anchors) {
            if !self.layers.contains(anchor) {
                self.returns.push((*anchor, took.last().map_or(focus, |&i: &usize| Some(tree.nodes[i].id))));
                // One beside its anchor leaves focus where it is.
                let first = tree.focusables(layer.root..layer.end).first().copied();
                if let Some(first) = first.filter(|_| !layer.beside) {
                    took.push(first);
                    self.focus_visible |= layer.shows_focus;
                }
            }
        }
        self.layers = anchors;
        let asking: Vec<Id> = tree.nodes.iter().filter(|node| node.el.takes_focus).map(|node| node.id).collect();
        took.extend(asking.iter().find(|id| !self.asking.contains(id)).and_then(|id| tree.find(*id)));
        self.asking = asking;
        if let Some(&took) = took.last() {
            let node = &tree.nodes[took];
            // A field reached this way is ready to be typed over, as one reached by Tab is.
            if let Content::Input(input) = &node.el.content {
                self.inputs.entry(node.id).or_default().select_all(&input.value);
            }
            focus = Some(node.id);
        }

        if focus.and_then(|id| tree.find(id)).is_none_or(|i| !tree.focusable(i)) {
            focus = self.rest();
        }
        if focus != self.focus {
            self.focus = focus;
            self.restart_blink();
        }

        // State is kept only for elements that are still there.
        let tree = &self.tree;
        self.open.retain(|id| tree.find(*id).is_some_and(|i| tree.nodes[i].trigger.is_some()));
        self.inputs.retain(|id, _| tree.find(*id).is_some());
        self.scroll.retain(|id, _| tree.find(*id).is_some());
        self.pressed = self.pressed.filter(|id| tree.find(*id).is_some());
    }
}

use crate::tree::Layer;

fn step(dir: Dir, key: &Key) -> Option<Step> {
    match (dir, key) {
        (Dir::Row, Key::Left) | (Dir::Column, Key::Up) => Some(Step::Previous),
        (Dir::Row, Key::Right) | (Dir::Column, Key::Down) => Some(Step::Next),
        (_, Key::Home) => Some(Step::First),
        (_, Key::End) => Some(Step::Last),
        _ => None,
    }
}
