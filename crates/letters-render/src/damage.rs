//! Works out which pixels a new display list changes, so a renderer can leave the rest of a frame alone.
//!
//! A renderer keeps a [`Survey`] of the list it last drew: a hash and an extent per item, not the list. The
//! items a new list adds, drops or alters are the ones outside the longest run of items the two share in
//! order, and only pixels inside their extents can differ.

use crate::image::Images;
use crate::paint::FontCache;
use crate::{Color, DisplayList, Item, PathEl, Point, Rect};

/// A rectangle of physical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl PixelRect {
    fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    fn union(&self, other: &Self) -> Self {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        let right = (self.x + self.width).max(other.x + other.width);
        let bottom = (self.y + self.height).max(other.y + other.height);
        Self { x, y, width: right - x, height: bottom - y }
    }
}

/// The parts of a frame a render repainted. Empty when the frame already showed the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    rects: Vec<PixelRect>,
}

impl Damage {
    pub(crate) fn new(rects: Vec<PixelRect>) -> Self {
        Self { rects }
    }

    /// Rectangles that between them cover every changed pixel. They do not overlap much, but may.
    pub fn rects(&self) -> &[PixelRect] {
        &self.rects
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    /// The damage of this render and another together, for a window buffer more than one frame old.
    pub fn union(&self, other: &Self) -> Self {
        let mut rects = self.rects.clone();
        rects.extend(&other.rects);
        merge(&mut rects);
        Self { rects }
    }
}

/// Where an item can leave ink, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Extent {
    /// It draws nothing.
    Nowhere,
    /// Left, top, right, bottom.
    Within([f32; 4]),
    /// A clip: it changes what later items draw, wherever they are.
    Anywhere,
}

impl Extent {
    fn of(rect: &Rect, pad: f32) -> Self {
        let sides = [rect.x - pad, rect.y - pad, rect.x + rect.width + pad, rect.y + rect.height + pad];
        if rect.width > 0.0 && rect.height > 0.0 && sides.iter().all(|side| side.is_finite()) {
            Self::Within(sides)
        } else {
            Self::Nowhere
        }
    }

    /// Whether the item can show inside a rectangle of physical pixels.
    pub fn touches(&self, rect: &PixelRect, scale: f32) -> bool {
        match self {
            Self::Nowhere => false,
            Self::Anywhere => true,
            Self::Within(sides) => pixels(sides, scale, u32::MAX, u32::MAX).is_some_and(|own| {
                own.x < rect.x + rect.width
                    && rect.x < own.x.saturating_add(own.width)
                    && own.y < rect.y + rect.height
                    && rect.y < own.y.saturating_add(own.height)
            }),
        }
    }
}

/// What a renderer remembers of an item it drew.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    /// Equal for two items exactly when they draw the same pixels.
    hash: u64,
    pub extent: Extent,
}

/// One entry per item of a display list.
pub(crate) type Survey = Vec<Entry>;

/// Mixes values into a 64-bit hash. Not for untrusted input: it only has to tell display items apart.
struct Mix(u64);

impl Mix {
    fn add(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(23) ^ value).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }

    fn floats(&mut self, values: &[f32]) {
        for pair in values.chunks(2) {
            self.add(u64::from(pair[0].to_bits()) << 32 | u64::from(pair.get(1).map_or(0, |value| value.to_bits())));
        }
    }

    fn rect(&mut self, rect: &Rect) {
        self.floats(&[rect.x, rect.y, rect.width, rect.height]);
    }

    fn color(&mut self, color: Color) {
        self.add(u64::from(u32::from_be_bytes([color.r, color.g, color.b, color.a])));
    }

    fn finish(self) -> u64 {
        self.0 ^ (self.0 >> 29)
    }
}

/// Surveys `list` into `out`, reusing its storage.
pub(crate) fn survey(list: &DisplayList, images: &Images, fonts: &mut FontCache, out: &mut Survey) {
    out.clear();
    out.extend(list.items.iter().map(|item| entry(item, images, fonts)));
}

fn entry(item: &Item, images: &Images, fonts: &mut FontCache) -> Entry {
    // Each kind of item starts from its own seed, so a fill never matches a stroke of the same shape.
    let mut mix;
    let extent = match item {
        Item::Fill { rect, color, radius } => {
            mix = Mix(1);
            mix.rect(rect);
            mix.color(*color);
            mix.floats(&[*radius]);
            Extent::of(rect, 0.0)
        }
        Item::Stroke { rect, color, width, radius } => {
            mix = Mix(2);
            mix.rect(rect);
            mix.color(*color);
            mix.floats(&[*width, *radius]);
            Extent::of(rect, 0.0)
        }
        Item::Shadow { rect, color, blur, offset, radius } => {
            mix = Mix(3);
            mix.rect(rect);
            mix.color(*color);
            mix.floats(&[*blur, offset.x, offset.y, *radius]);
            let moved = Rect::new(rect.x + offset.x, rect.y + offset.y, rect.width, rect.height);
            // Vello stops a blur at two and a half standard deviations, and the blur is two of them.
            Extent::of(&moved, 1.25 * blur.max(0.0))
        }
        Item::Glyphs { origin, font, size, color, skew, embolden, glyphs } => {
            mix = Mix(4);
            let face = fonts.face(font);
            mix.add(face.id);
            mix.add(u64::from(font.index));
            for coord in font.coords.iter() {
                mix.add(*coord as u64);
            }
            mix.floats(&[origin.x, origin.y, *size, *skew]);
            mix.color(*color);
            mix.add(u64::from(*embolden) << 32 | glyphs.len() as u64);
            let mut span = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            for glyph in glyphs {
                mix.add(u64::from(glyph.id) << 32 | u64::from(glyph.x.to_bits()));
                mix.add(u64::from(glyph.y.to_bits()));
                span = [span[0].min(glyph.x), span[1].min(glyph.y), span[2].max(glyph.x), span[3].max(glyph.y)];
            }
            // The face's box holds its default instance. Other weights, a slant and synthetic bold reach a
            // little further, as does a glyph nudged onto the pixel grid.
            let [left, bottom, right, top] = face.glyph_box;
            let lean = skew.to_radians().tan().abs() * top.max(-bottom);
            let (aside, above) = (0.25 + if lean.is_finite() { lean } else { 0.0 }, 0.15);
            let run = Rect::new(
                origin.x + span[0] + (left - aside) * size,
                origin.y + span[1] - (top + above) * size,
                span[2] - span[0] + (right - left + 2.0 * aside) * size,
                span[3] - span[1] + (top - bottom + 2.0 * above) * size,
            );
            Extent::of(&run, 1.0)
        }
        Item::Path { origin, scale, path, color, stroke } => {
            mix = Mix(5);
            mix.floats(&[origin.x, origin.y, *scale, stroke.unwrap_or(-1.0)]);
            mix.color(*color);
            mix.add(path.len() as u64);
            let mut span = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            let mut visit = |mix: &mut Mix, kind: u64, points: &[Point]| {
                mix.add(kind);
                for point in points {
                    mix.floats(&[point.x, point.y]);
                    span = [span[0].min(point.x), span[1].min(point.y), span[2].max(point.x), span[3].max(point.y)];
                }
            };
            for element in path.iter() {
                match element {
                    PathEl::MoveTo(to) => visit(&mut mix, 1, &[*to]),
                    PathEl::LineTo(to) => visit(&mut mix, 2, &[*to]),
                    PathEl::QuadTo(control, to) => visit(&mut mix, 3, &[*control, *to]),
                    PathEl::CurveTo(first, second, to) => visit(&mut mix, 4, &[*first, *second, *to]),
                    PathEl::Close => visit(&mut mix, 5, &[]),
                }
            }
            // A curve stays inside its control points, and a round stroke reaches half its width past them.
            let reach = stroke.map_or(0.0, |width| width.max(0.0) / 2.0);
            let shape = Rect::new(
                origin.x + (span[0] - reach) * scale,
                origin.y + (span[1] - reach) * scale,
                (span[2] - span[0] + 2.0 * reach) * scale,
                (span[3] - span[1] + 2.0 * reach) * scale,
            );
            Extent::of(&shape, 1.0)
        }
        Item::Image { rect, asset } => {
            mix = Mix(6);
            mix.rect(rect);
            // The picture is part of what the item draws: it changes when the asset is decoded or replaced.
            match images.get(asset) {
                Some(image) => {
                    mix.add(image.id);
                    Extent::of(rect, 0.0)
                }
                None => Extent::Nowhere,
            }
        }
        Item::PushClip { rect, radius } => {
            mix = Mix(7);
            mix.rect(rect);
            mix.floats(&[*radius]);
            Extent::Anywhere
        }
        Item::PopClip => {
            mix = Mix(8);
            Extent::Anywhere
        }
    };
    Entry { hash: mix.finish(), extent }
}

/// The most items two lists may differ by before the whole frame is repainted instead.
const MAX_EDITS: usize = 96;
/// The share of a frame above which repainting it whole is cheaper than repainting its parts.
const MAX_SHARE: f64 = 0.5;

/// The rectangles of a `width` by `height` frame to repaint to turn `old` into `new`, or `None` to repaint
/// it all.
pub(crate) fn regions(old: &Survey, new: &Survey, scale: f32, width: u32, height: u32) -> Option<Vec<PixelRect>> {
    // Most of two lists is the same at both ends, which leaves little to compare.
    let prefix = old.iter().zip(new).take_while(|(a, b)| a.hash == b.hash).count();
    let (old, new) = (&old[prefix..], &new[prefix..]);
    let suffix = old.iter().rev().zip(new.iter().rev()).take_while(|(a, b)| a.hash == b.hash).count();
    let (old, new) = (&old[..old.len() - suffix], &new[..new.len() - suffix]);

    let mut rects = Vec::new();
    for entry in unshared(old, new)? {
        match entry.extent {
            Extent::Nowhere => {}
            Extent::Anywhere => return None,
            Extent::Within(sides) => rects.extend(pixels(&sides, scale, width, height)),
        }
    }
    merge(&mut rects);
    let share = rects.iter().map(PixelRect::area).sum::<u64>() as f64 / (f64::from(width) * f64::from(height));
    (share <= MAX_SHARE).then_some(rects)
}

/// A logical extent as whole physical pixels inside a frame, with a pixel to spare for antialiasing.
fn pixels(sides: &[f32; 4], scale: f32, width: u32, height: u32) -> Option<PixelRect> {
    let edge = |side: f32, limit: u32| (side * scale).clamp(0.0, limit as f32);
    let (left, top) = (edge(sides[0], width).floor() as u32, edge(sides[1], height).floor() as u32);
    let (right, bottom) = (edge(sides[2], width).ceil() as u32, edge(sides[3], height).ceil() as u32);
    let (x, y) = (left.saturating_sub(1), top.saturating_sub(1));
    let rect = PixelRect {
        x,
        y,
        width: right.saturating_add(1).min(width) - x,
        height: bottom.saturating_add(1).min(height) - y,
    };
    (right > left && bottom > top).then_some(rect)
}

/// Joins rectangles that overlap or nearly do, so no pixel is repainted many times over.
fn merge(rects: &mut Vec<PixelRect>) {
    /// Pixels worth repainting needlessly to save a separate pass.
    const SLACK: u64 = 4096;
    let mut at = 0;
    while at < rects.len() {
        let joined = (at + 1..rects.len())
            .find(|&other| rects[at].union(&rects[other]).area() <= rects[at].area() + rects[other].area() + SLACK);
        match joined {
            Some(other) => {
                rects[at] = rects[at].union(&rects[other]);
                rects.swap_remove(other);
                // The larger rectangle may now reach ones it passed over.
                at = 0;
            }
            None => at += 1,
        }
    }
}

/// The entries of `old` and `new` outside their longest common subsequence, by Myers' algorithm, or `None`
/// if there are more than [`MAX_EDITS`] of them.
fn unshared<'a>(old: &'a [Entry], new: &'a [Entry]) -> Option<Vec<&'a Entry>> {
    /// Marks a diagonal no path has reached.
    const UNREACHED: isize = isize::MIN / 2;
    let (rows, columns) = (old.len() as isize, new.len() as isize);
    let limit = (old.len() + new.len()).min(MAX_EDITS) as isize;
    let at = |k: isize| (k + limit) as usize;
    // furthest[at(k)] is how far along `old` a path on diagonal k has reached.
    let mut furthest = vec![UNREACHED; 2 * limit as usize + 2];
    furthest[at(1)] = 0;
    let mut history = Vec::new();
    for edits in 0..=limit {
        history.push(furthest.clone());
        for k in (-edits..=edits).step_by(2) {
            let mut x = if steps_down(&furthest, at, k, edits) { furthest[at(k + 1)] } else { furthest[at(k - 1)] + 1 };
            if x < 0 || x > rows || x - k > columns {
                furthest[at(k)] = UNREACHED;
                continue;
            }
            while x < rows && x - k < columns && old[x as usize].hash == new[(x - k) as usize].hash {
                x += 1;
            }
            furthest[at(k)] = x;
            if x >= rows && x - k >= columns {
                return Some(trace(old, new, &history, at, edits, k));
            }
        }
    }
    None
}

/// Whether the best path onto diagonal `k` comes from the one above (an added entry) and not the one below
/// (a dropped entry).
fn steps_down(furthest: &[isize], at: impl Fn(isize) -> usize, k: isize, edits: isize) -> bool {
    k == -edits || (k != edits && furthest[at(k - 1)] < furthest[at(k + 1)])
}

/// Walks a finished search back to its start, collecting the entry each step dropped or added.
fn trace<'a>(
    old: &'a [Entry],
    new: &'a [Entry],
    history: &[Vec<isize>],
    at: impl Fn(isize) -> usize,
    edits: isize,
    mut k: isize,
) -> Vec<&'a Entry> {
    let mut out = Vec::new();
    for edits in (1..=edits).rev() {
        let furthest = &history[edits as usize];
        if steps_down(furthest, &at, k, edits) {
            k += 1;
            out.push(&new[(furthest[at(k)] - k) as usize]);
        } else {
            k -= 1;
            out.push(&old[furthest[at(k)] as usize]);
        }
    }
    out
}
