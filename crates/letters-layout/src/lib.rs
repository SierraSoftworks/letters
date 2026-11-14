//! Lays a document out and answers questions about where things are.
//!
//! [`DocLayout`] is the whole interface: update it after each edit, then paint a viewport, hit-test a point or
//! ask where a caret sits. Layout is incremental (only changed blocks are redone) and viewport-first.

use letters_model::{Change, Document, Pos, Selection, Style};
use letters_render::{DisplayList, Font, Point, Rect};

/// The fonts layout may use. The shell supplies them, since the core never reads files.
#[derive(Clone, Debug, Default)]
pub struct Fonts {
    pub faces: Vec<Font>,
}

/// A laid-out document. Coordinates are logical pixels from the top left of the page column.
#[derive(Debug, Default)]
pub struct DocLayout {}

impl DocLayout {
    pub fn new(_fonts: Fonts) -> Self {
        todo!("layout")
    }

    /// Brings the layout up to date with `doc`. `change` is `None` for a first layout or a change of width or
    /// style, and otherwise names what an edit touched.
    pub fn update(&mut self, _doc: &Document, _style: &Style, _width: f32, _change: Option<&Change>) {
        todo!("layout")
    }

    /// The height of the whole document.
    pub fn height(&self) -> f32 {
        todo!("layout")
    }

    /// Emits the part of the document that falls inside `viewport`.
    pub fn paint(&self, _viewport: Rect, _out: &mut DisplayList) {
        todo!("layout")
    }

    /// The text position nearest to a point.
    pub fn hit(&self, _point: Point) -> Option<Pos> {
        todo!("layout")
    }

    /// Where a caret at `pos` is drawn.
    pub fn caret(&self, _pos: &Pos) -> Option<Rect> {
        todo!("layout")
    }

    /// The rectangles that highlight a selection, one per line it touches.
    pub fn selection(&self, _selection: &Selection) -> Vec<Rect> {
        todo!("layout")
    }

    /// The position one visual line above or below `pos`, keeping to the horizontal position `x`.
    pub fn line_step(&self, _pos: &Pos, _x: f32, _down: bool) -> Option<Pos> {
        todo!("layout")
    }
}
