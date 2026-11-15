//! A formula as bytes, for an engine that runs apart from what draws its formulas and shares no memory
//! with it, as a module of a page does. Both sides hold the same faces, so a run of glyphs names its face
//! and does not carry it.

use letters_render::{Color, Glyph, Item, PathEl, Point, Rect, Size};

use crate::{Formula, Part, fonts};

/// The body face, which draws the letters the maths faces lack. Any other number is a maths face's place
/// among the files.
const BODY: u8 = u8::MAX;

const FILL: u8 = 0;
const GLYPHS: u8 = 1;
const PATH: u8 = 2;

const CLOSE: u8 = 0;
const MOVE: u8 = 1;
const LINE: u8 = 2;
const QUAD: u8 = 3;
const CURVE: u8 = 4;

impl Formula {
    /// The formula as bytes that [`Formula::from_bytes`] reads back, where the same faces are at hand.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let Size { width, height } = self.size;
        floats(&mut out, &[width, height, self.baseline]);
        count(&mut out, self.items.len());
        for item in &self.items {
            write(&mut out, item);
        }
        count(&mut out, self.parts.len());
        for Part { source, items } in &self.parts {
            [source.start, source.end, items.start, items.end].into_iter().for_each(|at| count(&mut out, at));
        }
        out
    }

    /// The formula [`Formula::to_bytes`] wrote. `None` for bytes it did not write, and for a face that is
    /// not here.
    pub fn from_bytes(bytes: &[u8]) -> Option<Formula> {
        let mut bytes = Reader(bytes);
        let [width, height, baseline] = bytes.floats()?;
        let items = (0..bytes.count(1)?).map(|_| read(&mut bytes)).collect::<Option<Vec<Item>>>()?;
        let part = |bytes: &mut Reader| {
            let [from, to, first, last] = [bytes.index()?, bytes.index()?, bytes.index()?, bytes.index()?];
            Some(Part { source: from..to, items: first..last })
        };
        let parts = (0..bytes.count(16)?).map(|_| part(&mut bytes)).collect::<Option<Vec<Part>>>()?;
        bytes.0.is_empty().then_some(Formula { size: Size { width, height }, baseline, items, parts })
    }
}

fn floats(out: &mut Vec<u8>, values: &[f32]) {
    values.iter().for_each(|value| out.extend(value.to_le_bytes()));
}

fn count(out: &mut Vec<u8>, count: usize) {
    out.extend(u32::try_from(count).unwrap_or(u32::MAX).to_le_bytes());
}

fn color(out: &mut Vec<u8>, Color { r, g, b, a }: Color) {
    out.extend([r, g, b, a]);
}

fn write(out: &mut Vec<u8>, item: &Item) {
    match item {
        Item::Fill { rect: Rect { x, y, width, height }, color: ink, radius } => {
            out.push(FILL);
            floats(out, &[*x, *y, *width, *height, *radius]);
            color(out, *ink);
        }
        Item::Glyphs { origin, font, size, color: ink, skew, embolden, glyphs, .. } => {
            out.push(GLYPHS);
            let face = fonts::place(font).and_then(|place| u8::try_from(place).ok());
            out.extend([face.unwrap_or(BODY), u8::from(*embolden)]);
            floats(out, &[origin.x, origin.y, *size, *skew]);
            color(out, *ink);
            count(out, glyphs.len());
            for Glyph { id, x, y } in glyphs.iter() {
                out.extend(id.to_le_bytes());
                floats(out, &[*x, *y]);
            }
        }
        Item::Path { origin, scale, path, color: ink, stroke } => {
            out.push(PATH);
            floats(out, &[origin.x, origin.y, *scale, stroke.unwrap_or(f32::NAN)]);
            color(out, *ink);
            count(out, path.len());
            for el in path.iter() {
                let mut put = |kind: u8, points: &[Point]| {
                    out.push(kind);
                    points.iter().for_each(|point| floats(out, &[point.x, point.y]));
                };
                match el {
                    PathEl::Close => put(CLOSE, &[]),
                    PathEl::MoveTo(to) => put(MOVE, &[*to]),
                    PathEl::LineTo(to) => put(LINE, &[*to]),
                    PathEl::QuadTo(by, to) => put(QUAD, &[*by, *to]),
                    PathEl::CurveTo(by, then, to) => put(CURVE, &[*by, *then, *to]),
                }
            }
        }
        // An engine draws nothing else. Whatever it is keeps its place, so that the parts still count right.
        _ => {
            out.push(FILL);
            out.extend([0; 24]);
        }
    }
}

fn read(bytes: &mut Reader) -> Option<Item> {
    match bytes.byte()? {
        FILL => {
            let [x, y, width, height, radius] = bytes.floats()?;
            Some(Item::Fill { rect: Rect { x, y, width, height }, color: bytes.color()?, radius })
        }
        GLYPHS => {
            let [face, embolden] = bytes.take()?;
            let font = if face == BODY { fonts::body_font()? } else { fonts::placed(usize::from(face))? };
            let [x, y, size, skew] = bytes.floats()?;
            let color = bytes.color()?;
            let glyph = |bytes: &mut Reader| {
                let id = u32::from_le_bytes(bytes.take()?);
                let [x, y] = bytes.floats()?;
                Some(Glyph { id, x, y })
            };
            let glyphs = (0..bytes.count(12)?).map(|_| glyph(bytes)).collect::<Option<Vec<Glyph>>>()?;
            Some(Item::Glyphs {
                origin: Point { x, y },
                font,
                size,
                color,
                skew,
                embolden: embolden != 0,
                glyphs,
                text: None,
            })
        }
        PATH => {
            let [x, y, scale, stroke] = bytes.floats()?;
            let color = bytes.color()?;
            let el = |bytes: &mut Reader| {
                let kind = bytes.byte()?;
                let mut point = || bytes.floats().map(|[x, y]| Point { x, y });
                Some(match kind {
                    CLOSE => PathEl::Close,
                    MOVE => PathEl::MoveTo(point()?),
                    LINE => PathEl::LineTo(point()?),
                    QUAD => PathEl::QuadTo(point()?, point()?),
                    CURVE => PathEl::CurveTo(point()?, point()?, point()?),
                    _ => return None,
                })
            };
            let path = (0..bytes.count(1)?).map(|_| el(bytes)).collect::<Option<Vec<PathEl>>>()?;
            let stroke = (!stroke.is_nan()).then_some(stroke);
            Some(Item::Path { origin: Point { x, y }, scale, path: path.into(), color, stroke })
        }
        _ => None,
    }
}

/// What is left to read.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (head, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*head)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take::<1>().map(|[byte]| byte)
    }

    fn floats<const N: usize>(&mut self) -> Option<[f32; N]> {
        let mut floats = [0.0; N];
        for float in &mut floats {
            *float = f32::from_le_bytes(self.take()?);
        }
        Some(floats)
    }

    fn color(&mut self) -> Option<Color> {
        self.take().map(|[r, g, b, a]| Color { r, g, b, a })
    }

    fn index(&mut self) -> Option<usize> {
        self.take().map(|bytes| u32::from_le_bytes(bytes) as usize)
    }

    /// How many things follow, each at least `least` bytes: never more than the bytes left could hold.
    fn count(&mut self, least: usize) -> Option<usize> {
        self.index().filter(|count| count.saturating_mul(least) <= self.0.len())
    }
}
