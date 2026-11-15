//! What text is drawn in. A layout names its colours instead of holding them, so a new palette needs no new
//! layout.

use letters_render::Color;

use crate::flow::{Role, Types};
use crate::palette::Palette;

/// The colour of a stretch of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Ink {
    /// The colour the document's style gives a role.
    Style(Role),
    Marker,
    Link,
    OnAccent,
    /// What is wrong as it stands, such as the source of a formula that cannot be typeset.
    Warning,
}

impl Default for Ink {
    fn default() -> Self {
        Self::Style(Role::Body)
    }
}

/// A fill behind a stretch of text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Back {
    Code,
    Highlight,
}

/// What Parley carries for each styled range.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Brush {
    pub ink: Ink,
    pub back: Option<Back>,
}

/// The colours of one paint: the palette, and each role's style colour as this page shows it.
pub(crate) struct Inks {
    pub palette: Palette,
    roles: Vec<Color>,
}

/// The colour a palette shows a style's colour as. One the style leaves out is the palette's text colour.
fn shown(palette: &Palette, color: Option<Color>) -> Color {
    color.map_or(palette.text, |color| palette.recolor(color))
}

/// The colour a role's text is painted in.
pub(crate) fn of(types: &Types, palette: &Palette, role: Role) -> Color {
    shown(palette, types.get(role).color)
}

impl Inks {
    pub fn new(types: &Types, palette: Palette) -> Self {
        Self { palette, roles: types.colors().map(|color| shown(&palette, color)).collect() }
    }

    pub fn ink(&self, ink: Ink) -> Color {
        match ink {
            Ink::Style(role) => self.roles.get(role as usize).copied().unwrap_or(self.palette.text),
            Ink::Marker => self.palette.marker,
            Ink::Link => self.palette.link,
            Ink::OnAccent => self.palette.on_accent,
            Ink::Warning => self.palette.warn.accent,
        }
    }

    pub fn back(&self, back: Back) -> Color {
        match back {
            Back::Code => self.palette.inline_code,
            Back::Highlight => self.palette.highlight,
        }
    }
}
