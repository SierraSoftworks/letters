//! The display list: what to draw, with no opinion on how.
//!
//! Layout and widgets emit a [`DisplayList`]. Backends (GPU, CPU, PDF, HTML) consume one. Coordinates are
//! logical pixels with the origin at the top left, and a backend applies the display scale.

use std::sync::Arc;

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

/// One face of a font file. Cheap to clone, and compared by identity of its bytes.
#[derive(Clone, Debug)]
pub struct Font {
    pub data: Arc<[u8]>,
    /// The face's index in a collection, or 0.
    pub index: u32,
}

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) && self.index == other.index
    }
}

/// A positioned glyph. `x` and `y` are relative to the origin of its run, with `y` on the baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    pub id: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
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
    /// A soft shadow cast by a rectangle.
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
        glyphs: Vec<Glyph>,
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
}
