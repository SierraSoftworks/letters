//! The one building block of the interface.
//!
//! An [`Element`] is a styled box that holds children, text, an icon, a text input or a canvas. Widgets are
//! functions that return elements, and an app describes its whole window as one tree of them each time its
//! state changes. Elements send the app's own messages (`M`) when something happens to them.

use std::cell::Cell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use accesskit::Role;
use letters_editor::{InputEvent, KeyEvent, Modifiers};
use letters_render::{DisplayList, Point, Rect};

use crate::chrome::Edge;
use crate::icon::Icon;
use crate::style::{Align, Dir, Edges, Justify, Look, Ring, Style};
use crate::text::TextStyle;
use crate::tokens::{Color as Ink, Scheme, Shadow};
use crate::widgets::{Popover, Side};

/// The pointer shape an element asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Default,
    Pointer,
    Text,
    /// Over the grip at an edge of a window that draws its own frame.
    Resize(Edge),
}

/// Draws an element's own content, such as the page.
pub trait Paint {
    /// `rect` is the element's content box in window coordinates, and `visible` the part of it that is not
    /// scrolled or clipped away. Painting only what falls in `visible` keeps a long document cheap.
    fn paint(&self, rect: Rect, visible: Rect, scheme: Scheme, out: &mut DisplayList);
}

/// What the pointer did on a canvas. Positions are relative to the canvas's top left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    /// `clicks` counts presses in quick succession: 2 for a double click.
    Down {
        at: Point,
        mods: Modifiers,
        clicks: u8,
    },
    /// The pointer moved with the button held.
    Drag {
        at: Point,
    },
    Up {
        at: Point,
    },
    /// A menu about what is at `at` is wanted: the secondary button went down there, or the Menu key was
    /// pressed with the caret there.
    Menu {
        at: Point,
    },
    /// The wheel turned, or two fingers moved. A positive `y` moves the content up. Returning a message
    /// takes the scroll, so that nothing behind the canvas moves as well.
    Scroll {
        delta: Point,
    },
}

pub(crate) type KeyHandler<M> = Rc<dyn Fn(&KeyEvent) -> Option<M>>;
pub(crate) type MenuHandler<M> = Rc<dyn Fn(Point) -> M>;

pub(crate) enum Content<M> {
    None,
    Text { text: String, style: TextStyle, wrap: bool },
    Icon { icon: Icon, size: f32 },
    Input(Input<M>),
    Canvas(Canvas<M>),
}

pub(crate) struct Input<M> {
    pub value: String,
    pub placeholder: String,
    pub style: TextStyle,
    pub on_input: Option<Rc<dyn Fn(String) -> M>>,
    pub on_submit: Option<M>,
    pub on_leave: Option<M>,
    pub on_cancel: Option<M>,
}

pub(crate) struct Canvas<M> {
    pub paint: Rc<dyn Paint>,
    pub on_text: Option<Rc<dyn Fn(InputEvent) -> M>>,
    pub on_pointer: Option<Rc<dyn Fn(Pointer) -> Option<M>>>,
    /// Where the canvas's own caret is, relative to its top left, for the input method's window.
    pub caret: Option<Rect>,
    /// Whether the app hears where the pointer rests on it.
    pub pointed: bool,
    /// Whether positions on it are told as parts of its width and height.
    pub spread: bool,
}

/// Whether and how an element takes keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Focus {
    #[default]
    No,
    /// Tab and arrow keys reach it. A click does not, so pressing a toolbar button leaves the caret in the
    /// document.
    Keys,
    /// A click focuses it too, as for a text field.
    Click,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Access {
    pub role: Option<Role>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub pressed: Option<bool>,
    pub checked: Option<bool>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub modal: bool,
    /// Decoration: left out of the accessibility tree, with everything in it.
    pub hidden: bool,
}

pub struct Element<M> {
    pub(crate) key: Option<u64>,
    pub(crate) style: Style,
    pub(crate) hover: Look,
    pub(crate) active: Look,
    pub(crate) focused: Look,
    pub(crate) ring: Ring,
    pub(crate) content: Content<M>,
    pub(crate) children: Vec<Element<M>>,
    pub(crate) access: Access,
    pub(crate) on_press: Option<M>,
    pub(crate) on_drag: Option<M>,
    pub(crate) on_double: Option<M>,
    pub(crate) on_menu: Option<MenuHandler<M>>,
    pub(crate) on_key: Option<KeyHandler<M>>,
    pub(crate) focus: Focus,
    pub(crate) disabled: bool,
    pub(crate) rest: bool,
    pub(crate) takes_focus: bool,
    pub(crate) closes: bool,
    pub(crate) arrows: Option<Dir>,
    pub(crate) one_stop: bool,
    pub(crate) cursor: Cursor,
    pub(crate) popover: Option<Box<Popover<M>>>,
    pub(crate) tooltip: Option<(String, Side)>,
    pub(crate) telling: bool,
    pub(crate) in_view: bool,
    pub(crate) fitted: Option<Fitted>,
}

/// Where the layout says which child of an [`Element::fit`] it showed, counted from the first.
pub type Fitted = Rc<Cell<Option<usize>>>;

impl<M> Element<M> {
    fn new(content: Content<M>) -> Self {
        Self {
            key: None,
            style: Style::default(),
            hover: Look::NONE,
            active: Look::NONE,
            focused: Look::NONE,
            ring: Ring::None,
            content,
            children: Vec::new(),
            access: Access::default(),
            on_press: None,
            on_drag: None,
            on_double: None,
            on_menu: None,
            on_key: None,
            focus: Focus::No,
            disabled: false,
            rest: false,
            takes_focus: false,
            closes: false,
            arrows: None,
            one_stop: false,
            cursor: Cursor::Default,
            popover: None,
            tooltip: None,
            telling: false,
            in_view: false,
            fitted: None,
        }
    }

    /// A box whose children sit side by side.
    pub fn row() -> Self {
        Self::new(Content::None)
    }

    /// A box whose children are stacked top to bottom.
    pub fn column() -> Self {
        let mut element = Self::new(Content::None);
        element.style.dir = Dir::Column;
        element
    }

    /// One line of text. It takes its colour from the nearest ancestor that sets one.
    pub fn text(text: impl Into<String>, style: TextStyle) -> Self {
        Self::new(Content::Text { text: text.into(), style, wrap: false })
    }

    /// Text that wraps to the width it is given.
    pub fn paragraph(text: impl Into<String>, style: TextStyle) -> Self {
        Self::new(Content::Text { text: text.into(), style, wrap: true })
    }

    /// An icon in a square of `size` pixels, in the inherited text colour.
    pub fn icon(icon: Icon, size: f32) -> Self {
        let mut element = Self::new(Content::Icon { icon, size });
        element.access.hidden = true;
        element
    }

    /// Empty room that takes whatever its siblings leave.
    pub fn spacer() -> Self {
        Self::row().grow(1.0)
    }

    /// An area the app draws itself. It takes focus on a click, and text and pointer input once the handlers
    /// are set.
    pub fn canvas(paint: Rc<dyn Paint>) -> Self {
        let mut element = Self::new(Content::Canvas(Canvas {
            paint,
            on_text: None,
            on_pointer: None,
            caret: None,
            pointed: false,
            spread: false,
        }));
        element.focus = Focus::Click;
        element.style.clip = true;
        element
    }

    pub(crate) fn input(input: Input<M>) -> Self {
        let mut element = Self::new(Content::Input(input));
        element.focus = Focus::Click;
        element.cursor = Cursor::Text;
        element
    }

    // Identity

    /// Names this element among its siblings, so its focus and state survive when they come and go. Elements
    /// with a role and label are told apart by those already.
    pub fn key(mut self, key: impl Hash) -> Self {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        self.key = Some(hasher.finish());
        self
    }

    // Children

    pub fn child(mut self, child: impl Into<Element<M>>) -> Self {
        self.children.push(child.into());
        self
    }

    pub fn children<C: Into<Element<M>>>(mut self, children: impl IntoIterator<Item = C>) -> Self {
        self.children.extend(children.into_iter().map(Into::into));
        self
    }

    // Arrangement

    pub fn gap(mut self, gap: f32) -> Self {
        self.style.gap = gap;
        self
    }

    pub fn padding(mut self, padding: Edges) -> Self {
        self.style.padding = padding;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.style.align = align;
        self
    }

    pub fn justify(mut self, justify: Justify) -> Self {
        self.style.justify = justify;
        self
    }

    pub fn align_self(mut self, align: Align) -> Self {
        self.style.align_self = Some(align);
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.style.width = Some(width);
        self
    }

    pub fn height(mut self, height: f32) -> Self {
        self.style.height = Some(height);
        self
    }

    pub fn min_width(mut self, width: f32) -> Self {
        self.style.min_width = width;
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.style.max_width = width;
        self
    }

    pub fn min_height(mut self, height: f32) -> Self {
        self.style.min_height = height;
        self
    }

    pub fn max_height(mut self, height: f32) -> Self {
        self.style.max_height = height;
        self
    }

    pub fn grow(mut self, share: f32) -> Self {
        self.style.grow = share;
        self
    }

    /// Lets the element take less than its size when its row has too little room. It gives up what in it
    /// can shrink in turn, or all of itself when it clips. A line of text that shrinks ends in an ellipsis.
    pub fn shrink(mut self) -> Self {
        self.style.shrink = true;
        self
    }

    // Looks

    pub fn bg(mut self, ink: Ink) -> Self {
        self.style.bg = Some(ink);
        self
    }

    pub fn fg(mut self, ink: Ink) -> Self {
        self.style.fg = Some(ink);
        self
    }

    pub fn border(mut self, widths: Edges, ink: Ink) -> Self {
        self.style.border = widths;
        self.style.border_color = Some(ink);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.style.radius = radius;
        self
    }

    pub fn shadow(mut self, shadow: &'static [Shadow]) -> Self {
        self.style.shadow = shadow;
        self
    }

    /// Fills the element with the text colour it inherits, so a mark inside a control follows the control's
    /// states.
    pub fn inked(mut self) -> Self {
        self.style.inked = true;
        self
    }

    /// Shows only the first of its children that fits the width it has, or the last when none does. The
    /// others are left out as if they were not there, so a bar can offer its controls in full and folded.
    pub fn fit(mut self) -> Self {
        self.style.fit = true;
        self
    }

    /// Fits as [`Element::fit`] does and tells `shown` which child it settled on. Whoever builds the children
    /// can then build only that one, until something that decides the fit changes.
    pub fn fit_into(mut self, shown: Fitted) -> Self {
        self.fitted = Some(shown);
        self.fit()
    }

    /// Lays the element over its siblings at the top left of its parent. It takes no room among them, so it
    /// goes last among the children to be drawn and hit above them.
    pub fn overlay(mut self) -> Self {
        self.style.overlay = true;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.style.opacity = opacity;
        self
    }

    pub fn clip(mut self) -> Self {
        self.style.clip = true;
        self
    }

    /// Scrolls the children vertically when they are taller than the element.
    pub fn scroll(mut self) -> Self {
        self.style.scroll = true;
        self.style.clip = true;
        self
    }

    /// The colours while the pointer is over the element.
    pub fn hover(mut self, look: Look) -> Self {
        self.hover = look;
        self
    }

    /// The colours while the element is pressed.
    pub fn active(mut self, look: Look) -> Self {
        self.active = look;
        self
    }

    /// The colours while the element shows keyboard focus.
    pub fn focused(mut self, look: Look) -> Self {
        self.focused = look;
        self
    }

    pub fn ring(mut self, ring: Ring) -> Self {
        self.ring = ring;
        self
    }

    pub fn cursor(mut self, cursor: Cursor) -> Self {
        self.cursor = cursor;
        self
    }

    // Behaviour

    /// Sends `message` when the element is clicked, or activated with Enter or Space while focused.
    pub fn on_press(mut self, message: M) -> Self {
        self.on_press = Some(message);
        self.focus = self.focus.max(Focus::Keys);
        self.cursor = Cursor::Pointer;
        self
    }

    /// Sends `message` when the element is clicked, and keeps it out of the keyboard's reach: for a part of a
    /// focusable element whose job the keys do another way, as the arrow of a tree's row.
    pub fn on_click(mut self, message: M) -> Self {
        self.on_press = Some(message);
        self.cursor = Cursor::Pointer;
        self
    }

    /// Sends `message` when a press on the element moves a little before it is let go, which is then no
    /// click: for what is dragged as a whole, as a window is by its title bar. It answers for the room
    /// between the controls inside it, and never for a control.
    pub fn on_drag(mut self, message: M) -> Self {
        self.on_drag = Some(message);
        self
    }

    /// Sends `message` at the second of two quick presses on the element, outside the controls inside it.
    pub fn on_double_click(mut self, message: M) -> Self {
        self.on_double = Some(message);
        self
    }

    /// Sends what `handler` makes of a point of the window when the secondary button goes down there on the
    /// element, outside the controls inside it.
    pub fn on_menu(mut self, handler: impl Fn(Point) -> M + 'static) -> Self {
        self.on_menu = Some(Rc::new(handler));
        self
    }

    /// Offers this element the keys nothing inside it used. Returning a message uses the key.
    pub fn on_key(mut self, handler: impl Fn(&KeyEvent) -> Option<M> + 'static) -> Self {
        self.on_key = Some(Rc::new(handler));
        self
    }

    /// Lets Tab and arrow keys reach the element.
    pub fn focusable(mut self) -> Self {
        self.focus = self.focus.max(Focus::Keys);
        self
    }

    /// A disabled element ignores input and leaves the tab order, with everything in it.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Makes this the element focus rests on whenever nothing else holds it, as the page does.
    pub fn default_focus(mut self) -> Self {
        self.rest = true;
        self.focus = self.focus.max(Focus::Keys);
        self
    }

    /// Gives this element the keyboard's focus when it first shows, as a field that opens in place of what
    /// it edits.
    pub fn takes_focus(mut self) -> Self {
        self.takes_focus = true;
        self.focus = self.focus.max(Focus::Keys);
        self
    }

    /// Activating the element closes the popover it is in, as choosing a menu item does.
    pub fn closes(mut self) -> Self {
        self.closes = true;
        self
    }

    /// Moves focus between the focusable elements inside with the arrow keys along `dir`, and Home and End.
    pub fn arrows(mut self, dir: Dir) -> Self {
        self.arrows = Some(dir);
        self
    }

    /// Makes everything inside one stop for the Tab key: the element in it that has focus, or else its
    /// first. With [`Element::arrows`], a long list then costs a keyboard user one press to pass.
    pub fn one_stop(mut self) -> Self {
        self.one_stop = true;
        self
    }

    /// Shows `text` beside the element while the pointer is over it or the keyboard's focus is on it.
    pub fn tooltip(mut self, text: impl Into<String>, side: Side) -> Self {
        self.tooltip = Some((text.into(), side));
        self
    }

    /// Shows the tooltip without being pointed at, while the element has news to tell. One under the pointer
    /// or the keyboard's focus still comes first.
    pub fn telling(mut self, telling: bool) -> Self {
        self.telling = telling;
        self
    }

    /// Brings the element into view of whatever scrolls it, when it first shows and each time it takes this
    /// over from another: for the row a list has picked while focus stays in a field. One element of a
    /// window at a time.
    pub fn in_view(mut self, in_view: bool) -> Self {
        self.in_view = in_view;
        self
    }

    /// Attaches a popover. Without [`Popover::open`], pressing the element opens and closes it.
    pub fn popover(mut self, popover: Popover<M>) -> Self {
        self.popover = Some(Box::new(popover));
        self.focus = self.focus.max(Focus::Keys);
        self.cursor = Cursor::Pointer;
        self
    }

    /// Text typed while the canvas has focus.
    pub fn on_text(mut self, handler: impl Fn(InputEvent) -> M + 'static) -> Self {
        if let Content::Canvas(canvas) = &mut self.content {
            canvas.on_text = Some(Rc::new(handler));
            self.cursor = Cursor::Text;
        }
        self
    }

    /// Presses and drags on the canvas.
    pub fn on_pointer(mut self, handler: impl Fn(Pointer) -> Option<M> + 'static) -> Self {
        if let Content::Canvas(canvas) = &mut self.content {
            canvas.on_pointer = Some(Rc::new(handler));
        }
        self
    }

    /// Has the app told where the pointer rests on the canvas, through [`App::point`](crate::App::point). A
    /// window has one such canvas at most, since the app is not told which.
    pub fn pointed(mut self) -> Self {
        if let Content::Canvas(canvas) = &mut self.content {
            canvas.pointed = true;
        }
        self
    }

    /// Has positions on the canvas told as parts of its width and height, from 0 to 1 across it: for a
    /// canvas that spreads what it draws over whatever room it is given, so that what handles a press need
    /// not know how much room that was.
    pub fn spread(mut self) -> Self {
        if let Content::Canvas(canvas) = &mut self.content {
            canvas.spread = true;
        }
        self
    }

    /// Keeps a canvas from taking focus, for one that is only a picture. In a popover beside another element
    /// it may still show that element's caret: the caret set here blinks while that element has focus, and
    /// a press here leaves focus with it.
    pub fn passive(mut self) -> Self {
        self.focus = Focus::No;
        self
    }

    /// Where the canvas draws its caret, relative to its top left.
    pub fn caret(mut self, caret: Option<Rect>) -> Self {
        if let Content::Canvas(canvas) = &mut self.content {
            canvas.caret = caret;
        }
        self
    }

    // Accessibility

    pub fn role(mut self, role: Role) -> Self {
        self.access.role = Some(role);
        self.access.hidden = false;
        self
    }

    /// The accessible name. Buttons and menu items take theirs from their text when this is not set.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.access.label = Some(label.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.access.description = Some(description.into());
        self
    }

    /// Marks a toggle button on or off.
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.access.pressed = Some(pressed);
        self
    }

    /// Marks a checkable menu item.
    pub fn checked(mut self, checked: bool) -> Self {
        self.access.checked = Some(checked);
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.access.selected = Some(selected);
        self
    }

    /// Says whether a row that has others under it shows them, as a folder in a tree.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.access.expanded = Some(expanded);
        self
    }

    /// Hides the element and everything in it from assistive technology.
    pub fn decorative(mut self) -> Self {
        self.access.hidden = true;
        self
    }

    pub(crate) fn modal(mut self) -> Self {
        self.access.modal = true;
        self
    }
}

impl Focus {
    fn max(self, other: Focus) -> Focus {
        match (self, other) {
            (Focus::Click, _) | (_, Focus::Click) => Focus::Click,
            (Focus::Keys, _) | (_, Focus::Keys) => Focus::Keys,
            _ => Focus::No,
        }
    }
}
