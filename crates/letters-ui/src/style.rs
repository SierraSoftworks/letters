//! How an element is sized, arranged and drawn. The vocabulary is the subset of CSS flexbox yewi's components
//! use, so a component's stylesheet ports line by line.

use crate::tokens::{Color as Ink, Shadow};

/// A length on each side of a box.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Edges {
    pub const ZERO: Self = Self::all(0.0);

    pub const fn all(value: f32) -> Self {
        Self { top: value, right: value, bottom: value, left: value }
    }

    /// `x` on the left and right, `y` on the top and bottom.
    pub const fn xy(x: f32, y: f32) -> Self {
        Self { top: y, right: x, bottom: y, left: x }
    }

    pub const fn top(value: f32) -> Self {
        Self { top: value, ..Self::ZERO }
    }

    pub const fn bottom(value: f32) -> Self {
        Self { bottom: value, ..Self::ZERO }
    }

    pub const fn left(value: f32) -> Self {
        Self { left: value, ..Self::ZERO }
    }

    pub(crate) fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub(crate) fn vertical(&self) -> f32 {
        self.top + self.bottom
    }

    pub(crate) fn uniform(&self) -> Option<f32> {
        (self.top == self.right && self.top == self.bottom && self.top == self.left).then_some(self.top)
    }
}

/// The axis children are laid along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dir {
    #[default]
    Row,
    Column,
}

/// Where children sit across the axis (`align-items`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    /// Children without a size of their own fill the cross axis.
    #[default]
    Stretch,
}

/// Where spare room along the axis goes (`justify-content`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    Between,
}

/// The colours a state may override. `None` keeps the element's own.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub bg: Option<Ink>,
    pub fg: Option<Ink>,
    pub border: Option<Ink>,
}

impl Look {
    pub const NONE: Self = Self { bg: None, fg: None, border: None };

    pub const fn bg(ink: Ink) -> Self {
        Self { bg: Some(ink), ..Self::NONE }
    }

    pub const fn with_fg(self, ink: Ink) -> Self {
        Self { fg: Some(ink), ..self }
    }

    pub const fn with_border(self, ink: Ink) -> Self {
        Self { border: Some(ink), ..self }
    }

    pub(crate) fn over(self, base: Look) -> Look {
        Look { bg: self.bg.or(base.bg), fg: self.fg.or(base.fg), border: self.border.or(base.border) }
    }
}

/// How an element shows keyboard focus.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Ring {
    /// Nothing is drawn. The element's `focused` look, if any, still applies.
    #[default]
    None,
    /// yewi's focus ring: two pixels of the focus colour, this far outside the edge (negative is inside).
    Outline(f32),
    /// A field's ring: it sits on the border, and shows for pointer focus too.
    Field,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub dir: Dir,
    pub gap: f32,
    pub padding: Edges,
    pub align: Align,
    pub justify: Justify,
    /// Fixed outer sizes, border included. `None` fits the content.
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub min_width: f32,
    pub max_width: f32,
    pub min_height: f32,
    pub max_height: f32,
    /// The share of spare room along the parent's axis this element takes.
    pub grow: f32,
    /// Whether the element gives up room when its siblings do not fit.
    pub shrink: bool,
    pub align_self: Option<Align>,

    pub bg: Option<Ink>,
    /// The colour of text and icons, inherited by descendants.
    pub fg: Option<Ink>,
    pub border: Edges,
    pub border_color: Option<Ink>,
    pub radius: f32,
    pub shadow: &'static [Shadow],
    /// Applies to the element and everything in it.
    pub opacity: f32,
    /// Hides what falls outside the element's edge.
    pub clip: bool,
    /// Scrolls vertically when the content is taller than the element.
    pub scroll: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            dir: Dir::Row,
            gap: 0.0,
            padding: Edges::ZERO,
            align: Align::Stretch,
            justify: Justify::Start,
            width: None,
            height: None,
            min_width: 0.0,
            max_width: f32::INFINITY,
            min_height: 0.0,
            max_height: f32::INFINITY,
            grow: 0.0,
            shrink: false,
            align_self: None,
            bg: None,
            fg: None,
            border: Edges::ZERO,
            border_color: None,
            radius: 0.0,
            shadow: &[],
            opacity: 1.0,
            clip: false,
            scroll: false,
        }
    }
}

impl Style {
    pub(crate) fn frame_x(&self) -> f32 {
        self.padding.horizontal() + self.border.horizontal()
    }

    pub(crate) fn frame_y(&self) -> f32 {
        self.padding.vertical() + self.border.vertical()
    }

    pub(crate) fn look(&self) -> Look {
        Look { bg: self.bg, fg: self.fg, border: self.border_color }
    }
}
