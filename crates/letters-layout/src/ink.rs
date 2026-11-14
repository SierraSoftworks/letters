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

impl Inks {
    pub fn new(types: &Types, palette: Palette) -> Self {
        let shown = |color: Option<Color>| color.map_or(palette.text, |color| palette.recolor(color));
        Self { palette, roles: types.colors().map(shown).collect() }
    }

    pub fn ink(&self, ink: Ink) -> Color {
        match ink {
            Ink::Style(role) => self.roles.get(role as usize).copied().unwrap_or(self.palette.text),
            Ink::Marker => self.palette.marker,
            Ink::Link => self.palette.link,
            Ink::OnAccent => self.palette.on_accent,
        }
    }

    pub fn back(&self, back: Back) -> Color {
        match back {
            Back::Code => self.palette.inline_code,
            Back::Highlight => self.palette.highlight,
        }
    }
}
