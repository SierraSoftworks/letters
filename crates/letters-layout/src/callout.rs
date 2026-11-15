//! A callout's head: the icon and the title that stand above its text, and the colour it is drawn in.

use std::sync::{Arc, LazyLock};

use letters_model::Tone;
use letters_render::{Color, PathEl, svg_path};

use crate::palette::{Callout, Palette};

/// The side of a callout's icon, and the gap between it and what follows.
pub(crate) const ICON: f32 = 18.0;
pub(crate) const ICON_GAP: f32 = 8.0;
/// The gap between a callout's title and its text.
pub(crate) const HEAD_GAP: f32 = 4.0;
/// The icons are drawn on a grid of this side, as strokes this wide.
const GRID: f32 = 24.0;
const STROKE: f32 = 2.0;

/// The outline of each of [`Tone::ICONS`] as SVG path data on a 24 unit grid, as yewi stores them.
const PATHS: [&str; 12] = [
    "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0M12 16v-4M12 8h.01",
    "M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.7 1.3 1.5 1.5 2.5M9 18h6M10 22h4",
    "M21.73 18l-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3M12 9v4M12 17h.01",
    "M12 16h.01M12 8v4M15.312 2a2 2 0 0 1 1.414.586l4.688 4.688A2 2 0 0 1 22 8.688v6.624a2 2 0 0 1-.586 1.414l-4.688 4.688a2 2 0 0 1-1.414.586H8.688a2 2 0 0 1-1.414-.586l-4.688-4.688A2 2 0 0 1 2 15.312V8.688a2 2 0 0 1 .586-1.414l4.688-4.688A2 2 0 0 1 8.688 2z",
    "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0M16 9l-5.5 5.5L8 12",
    "M2 12a10 10 0 1 0 20 0a10 10 0 1 0 -20 0M9 9.5a3 2.5 0 016 0c0 2-2 2-3 3.5M12 17h.01",
    "M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z",
    "M12 3q1 4 4 6.5t3 5.5a1 1 0 0 1-14 0 5 5 0 0 1 1-3 1 1 0 0 0 5 0c0-2-1.5-3-1.5-5q0-2 2.5-4",
    "M17 3a2 2 0 0 1 2 2v15a1 1 0 0 1-1.496.868l-4.512-2.578a2 2 0 0 0-1.984 0l-4.512 2.578A1 1 0 0 1 5 20V5a2 2 0 0 1 2-2z",
    "M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497zM15 5l4 4",
    "M16 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2zM5 3a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2 1 1 0 0 1 1 1v1a2 2 0 0 1-2 2 1 1 0 0 0-1 1v2a1 1 0 0 0 1 1 6 6 0 0 0 6-6V5a2 2 0 0 0-2-2z",
    "M12 20v-9M14 7a4 4 0 0 1 4 4v3a6 6 0 0 1-12 0v-3a4 4 0 0 1 4-4zM14.12 3.88 16 2M21 21a4 4 0 0 0-3.81-4M21 5a4 4 0 0 1-3.55 3.97M22 13h-4M3 21a4 4 0 0 1 3.81-4M3 5a4 4 0 0 0 3.55 3.97M6 13H2M8 2l1.88 1.88M9 7.13V6a3 3 0 1 1 6 0v1.13",
];

static OUTLINES: LazyLock<Vec<Arc<[PathEl]>>> =
    LazyLock::new(|| PATHS.iter().map(|path| svg_path(path).into()).collect());

/// The SVG path data of a callout's icon, by its name among [`Tone::ICONS`], on a 24 unit grid and meant
/// to be stroked. For whoever offers the icons to choose from.
pub fn icon_path(name: &str) -> Option<&'static str> {
    Tone::ICONS.iter().position(|known| *known == name).map(|at| PATHS[at])
}

/// The outline of an icon and the scale and stroke that draw it [`ICON`] wide.
pub(crate) fn outline(name: &str) -> Option<(Arc<[PathEl]>, f32, f32)> {
    let at = Tone::ICONS.iter().position(|known| *known == name)?;
    Some((OUTLINES[at].clone(), ICON / GRID, STROKE))
}

/// What stands at the top of a callout.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Head {
    pub icon: Option<&'static str>,
    /// Empty for none. Then the icon stands beside the callout's first line.
    pub title: String,
    /// The callout's own colour, as written for a light page.
    pub color: Option<Color>,
    /// The height of the title's line, which is 0 without a title.
    pub line: f32,
}

impl Head {
    /// How much room the head takes above the callout's text, and to the left of it.
    pub fn room(&self) -> (f32, f32) {
        match (self.title.is_empty(), self.icon) {
            (false, _) => (self.line + HEAD_GAP, 0.0),
            (true, Some(_)) => (0.0, ICON + ICON_GAP),
            (true, None) => (0.0, 0.0),
        }
    }

    /// The colours it is drawn in: its own, or its tone's.
    pub fn colors(&self, palette: &Palette, tone: Tone) -> Callout {
        let of = palette.callout(tone);
        match self.color {
            Some(color) => {
                let accent = palette.recolor(color);
                Callout { accent, fill: Color::rgba(accent.r, accent.g, accent.b, of.fill.a) }
            }
            None => of,
        }
    }
}
