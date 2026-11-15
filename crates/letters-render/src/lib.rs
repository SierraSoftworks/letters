//! The display list: what to draw, with no opinion on how.
//!
//! Layout and widgets emit a [`DisplayList`]. Backends (GPU, CPU, PDF, HTML) consume one. Coordinates are
//! logical pixels with the origin at the top left, and a backend applies the display scale.

use std::sync::Arc;

pub mod cpu;
mod damage;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod image;

mod paint;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "snapshot")]
pub mod snapshot;
mod svg;
pub use svg::svg_path;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x && point.x < self.x + self.width && point.y >= self.y && point.y < self.y + self.height
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.width
            && other.x < self.x + self.width
            && self.y < other.y + other.height
            && other.y < self.y + self.height
    }
}

/// Straight (not premultiplied) sRGB.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }
}

/// One instance of a face in a font file. Cheap to clone, and compared by identity of its bytes.
#[derive(Clone, Debug)]
pub struct Font {
    pub data: Arc<[u8]>,
    /// The face's index in a collection, or 0.
    pub index: u32,
    /// A variable font's normalised design coordinates (F2Dot14, in axis order), or empty for the default
    /// instance.
    pub coords: Arc<[i16]>,
}

impl Font {
    /// The default instance of a face.
    pub fn new(data: Arc<[u8]>, index: u32) -> Self {
        Self { data, index, coords: Arc::default() }
    }

    pub fn with_coords(mut self, coords: impl Into<Arc<[i16]>>) -> Self {
        self.coords = coords.into();
        self
    }
}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) && self.index == other.index && self.coords == other.coords
    }
}

/// A positioned glyph. `x` and `y` are relative to the origin of its run, with `y` on the baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

/// The text a run of glyphs was shaped from, for a backend that keeps text as text, such as PDF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunText {
    pub text: Box<str>,
    /// For each glyph of the run in order, the byte offset in `text` where its cluster starts. A cluster
    /// ends where the next one in the text starts.
    pub clusters: Box<[u32]>,
}

/// One segment of a vector path, in the path's own units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathEl {
    MoveTo(Point),
    LineTo(Point),
    /// A quadratic curve: its control point, then its end.
    QuadTo(Point, Point),
    /// A cubic curve: its two control points, then its end.
    CurveTo(Point, Point, Point),
    Close,
}

/// What a part of a document is, for a backend that writes its structure, such as a tagged PDF.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// A heading, 1 for the top level.
    Heading(u8),
    Paragraph,
    /// One item of a list. Items that follow one another make a list, and a deeper item lies under the
    /// item before it. `checked` is the state of a task's box, which is drawn in the item's [`Role::Label`].
    ListItem {
        depth: u8,
        numbered: bool,
        checked: Option<bool>,
    },
    /// The bullet, number or box that marks its list item.
    Label,
    Quote,
    /// A callout: a note, tip or warning set apart from the text.
    Note,
    Code,
    /// A picture. `alt` says what it shows, and is empty if nobody said.
    Figure {
        alt: Arc<str>,
    },
    Caption,
    /// Maths. `alt` is how it was written.
    Formula {
        alt: Arc<str>,
    },
    /// Text that takes the reader somewhere, inside the text around it. A backend with links of its own,
    /// such as [`pdf::Link`], is told which part a link belongs to by the part's id.
    Link,
    Table,
    Row,
    Cell {
        header: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Item {
    /// A filled rectangle, with rounded corners when `radius` is above zero.
    Fill {
        rect: Rect,
        color: Color,
        radius: f32,
    },
    /// A rectangle's outline, drawn inside its edge.
    Stroke {
        rect: Rect,
        color: Color,
        width: f32,
        radius: f32,
    },
    /// A soft shadow cast by a rectangle. `blur` is a CSS blur radius: twice the standard deviation.
    Shadow {
        rect: Rect,
        color: Color,
        blur: f32,
        offset: Point,
        radius: f32,
    },
    /// Shaped text in one font, size and colour.
    Glyphs {
        origin: Point,
        font: Font,
        size: f32,
        color: Color,
        /// A synthetic slant in degrees, for a face with no italic. 0 for none.
        skew: f32,
        /// Thickens the outlines, for a face with no bold.
        embolden: bool,
        glyphs: Vec<Glyph>,
        /// What the glyphs spell. `None` where it is not kept: nothing on screen needs it.
        text: Option<Arc<RunText>>,
    },
    /// A vector shape such as an icon. Path units are multiplied by `scale` and placed at `origin`.
    Path {
        origin: Point,
        scale: f32,
        path: Arc<[PathEl]>,
        color: Color,
        /// `None` fills the path by the even-odd rule. A width, in path units, strokes it with round caps
        /// and joins.
        stroke: Option<f32>,
    },
    /// An asset of the document, by entry name, scaled to fill `rect`.
    Image {
        rect: Rect,
        asset: String,
    },
    /// Clips everything up to the matching [`Item::PopClip`].
    PushClip {
        rect: Rect,
        radius: f32,
    },
    PopClip,
    /// Starts a part of the document: what is drawn up to the matching [`Item::Close`] is its content, and
    /// a part opened inside it lies under it. An `id` that opens again goes on with the same part, as a
    /// paragraph does on its next page. What is drawn inside no part is decoration. Nothing on screen
    /// changes.
    Open {
        role: Role,
        id: u64,
    },
    Close,
}

/// Items in paint order, back to front.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayList {
    pub items: Vec<Item>,
}

impl DisplayList {
    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    /// Moves the items from index `from` onward by `by`. A caller that paints something in its own
    /// coordinates (a page, a widget) uses this to place what was just emitted.
    pub fn translate(&mut self, from: usize, by: Point) {
        for item in self.items.iter_mut().skip(from) {
            match item {
                Item::Fill { rect, .. }
                | Item::Stroke { rect, .. }
                | Item::Shadow { rect, .. }
                | Item::Image { rect, .. }
                | Item::PushClip { rect, .. } => {
                    rect.x += by.x;
                    rect.y += by.y;
                }
                Item::Glyphs { origin, .. } | Item::Path { origin, .. } => {
                    origin.x += by.x;
                    origin.y += by.y;
                }
                Item::PopClip | Item::Open { .. } | Item::Close => {}
            }
        }
    }

    /// Makes the items from index `from` onward `by` times their size, about the origin. A caller that
    /// shows something small, as a preview of a picture, paints it whole and uses this to fit it.
    pub fn scale(&mut self, from: usize, by: f32) {
        let fit = |rect: &mut Rect| *rect = Rect::new(rect.x * by, rect.y * by, rect.width * by, rect.height * by);
        for item in self.items.iter_mut().skip(from) {
            match item {
                Item::Fill { rect, radius, .. } | Item::PushClip { rect, radius } => {
                    fit(rect);
                    *radius *= by;
                }
                Item::Stroke { rect, width, radius, .. } => {
                    fit(rect);
                    (*width, *radius) = (*width * by, *radius * by);
                }
                Item::Shadow { rect, blur, offset, radius, .. } => {
                    fit(rect);
                    (*blur, *radius) = (*blur * by, *radius * by);
                    *offset = Point { x: offset.x * by, y: offset.y * by };
                }
                Item::Image { rect, .. } => fit(rect),
                Item::Glyphs { origin, size, glyphs, .. } => {
                    *origin = Point { x: origin.x * by, y: origin.y * by };
                    *size *= by;
                    for glyph in glyphs {
                        (glyph.x, glyph.y) = (glyph.x * by, glyph.y * by);
                    }
                }
                // A stroke's width is in the path's own units, so it follows the scale.
                Item::Path { origin, scale, .. } => {
                    *origin = Point { x: origin.x * by, y: origin.y * by };
                    *scale *= by;
                }
                Item::PopClip | Item::Open { .. } | Item::Close => {}
            }
        }
    }
}
