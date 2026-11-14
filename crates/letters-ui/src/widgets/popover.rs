//! Popover: a surface that floats beside the element it is attached to, above everything else.
//!
//! Attach one with [`Element::popover`]. Left to itself it opens and closes when its anchor is pressed, and
//! closes on Escape or a press outside. With [`Popover::open`] the app decides instead.

use accesskit::{HasPopup, Role};
use letters_render::Rect;

use crate::element::Element;
use crate::style::{Align, Edges};
use crate::tokens;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

/// Where a popover sits against its anchor. It moves to the opposite side when it would leave the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub side: Side,
    /// Which edge of the anchor the popover lines up with: `Start`, `Center` or `End`.
    pub align: Align,
    /// The space between the anchor and the popover.
    pub gap: f32,
    /// A spot inside the anchor to hang from instead of its whole box, relative to its top left.
    pub at: Option<Rect>,
}

impl Default for Placement {
    fn default() -> Self {
        Self { side: Side::Bottom, align: Align::Start, gap: 6.0, at: None }
    }
}

pub struct Popover<M> {
    pub(crate) content: Element<M>,
    pub(crate) placement: Placement,
    pub(crate) open: Option<bool>,
    pub(crate) on_close: Option<M>,
    pub(crate) modal: bool,
    pub(crate) popup: HasPopup,
}

impl<M> Popover<M> {
    /// A raised surface around `content`.
    pub fn new(content: impl Into<Element<M>>) -> Self {
        let surface = Element::column()
            .gap(tokens::SPACE_SM)
            .padding(Edges::all(tokens::SPACE_MD))
            .max_width(320.0)
            .bg(tokens::SURFACE_RAISED)
            .fg(tokens::FG)
            .border(Edges::all(1.0), tokens::CARD_LINE)
            .radius(tokens::RADIUS_CARD)
            .shadow(tokens::SHADOW_OVERLAY)
            .child(content);
        Self {
            content: surface,
            placement: Placement::default(),
            open: None,
            on_close: None,
            modal: false,
            popup: HasPopup::Dialog,
        }
    }

    /// Names the surface, which makes it a dialog to assistive technology.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.content = self.content.role(Role::Dialog).label(label);
        self
    }

    /// Lets the content run edge to edge.
    pub fn flush(mut self) -> Self {
        self.content = self.content.padding(Edges::ZERO).gap(0.0);
        self
    }

    /// A surface for a list of choices: flush, with the tighter corner of a control.
    pub(crate) fn list(mut self, popup: HasPopup) -> Self {
        self.popup = popup;
        self.content = self.content.radius(tokens::RADIUS_CONTROL).clip();
        self.flush()
    }

    pub fn side(mut self, side: Side) -> Self {
        self.placement.side = side;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.placement.align = align;
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.placement.gap = gap;
        self
    }

    /// Hangs the popover from a spot inside its anchor, such as the caret on the page.
    pub fn at(mut self, spot: Rect) -> Self {
        self.placement.at = Some(spot);
        self
    }

    pub fn max_width(mut self, width: f32) -> Self {
        self.content = self.content.max_width(width);
        self
    }

    /// Makes the app decide when the popover shows. Pair it with [`Popover::on_close`].
    pub fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }

    /// Sent when an app-controlled popover asks to close: on Escape, a press outside, or a choice inside.
    pub fn on_close(mut self, message: M) -> Self {
        self.on_close = Some(message);
        self
    }

    /// Centres the surface over a dimmed window and keeps the pointer and Tab inside it.
    pub fn modal(mut self) -> Self {
        self.modal = true;
        self.content = self.content.modal();
        self
    }
}
