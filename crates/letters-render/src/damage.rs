//! Works out which pixels a new display list changes, so a renderer can leave the rest of a frame alone.
//!
//! A renderer keeps a [`Survey`] of the list it last drew: a few numbers per item, not the list. Pairing its
//! items with the new list's, in order, sorts them three ways. Items that stayed put need nothing. Items that
//! all moved up or down by the same whole number of pixels, as a scroll or a rewrap moves them, have pixels
//! that can move with them. Only the rest, and whatever a move uncovers, is painted afresh.

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

/// The parts of a frame a render changed. Empty when the frame already showed the list.
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

/// A rectangle that is the same on every row away from its top and bottom: a fill's, or a clip's.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Flat {
    /// Left, top, right, bottom.
    sides: [f32; 4],
    /// How far its rounded corners reach down from the top and up from the bottom.
    inset: f32,
}

impl Flat {
    /// The rectangle, or an empty one for a rectangle that draws or shows nothing.
    fn of(rect: &Rect, radius: f32) -> Self {
        let sides = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
        if rect.width > 0.0 && rect.height > 0.0 && sides.iter().all(|side| side.is_finite()) {
            let radius = if radius.is_finite() { radius.max(0.0) } else { 0.0 };
            Self { sides, inset: radius.min(rect.height / 2.0) }
        } else {
            Self { sides: [0.0; 4], inset: 0.0 }
        }
    }
}

/// What a renderer remembers of an item it drew.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    /// Equal for two items exactly when they draw the same pixels.
    hash: u64,
    /// Equal for two items exactly when they draw the same thing, wherever each draws it.
    shape: u64,
    /// Where the item draws it.
    at: [f32; 2],
    pub extent: Extent,
    /// Set for a fill and for the start of a clip.
    flat: Option<Flat>,
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

    fn bytes(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        words.iter().for_each(|word| self.add(u64::from_le_bytes(*word)));
        let mut last = [0; 8];
        last[..rest.len()].copy_from_slice(rest);
        self.add(u64::from_le_bytes(last) ^ (bytes.len() as u64) << 56);
    }

    fn color(&mut self, color: Color) {
        self.add(u64::from(u32::from_be_bytes([color.r, color.g, color.b, color.a])));
    }

    fn finish(self) -> u64 {
        self.0 ^ (self.0 >> 29)
    }
}

/// How an item's hash names the fonts and images it draws.
#[derive(Clone, Copy)]
enum By {
    /// By an id that lasts for the life of the process. Cheap, for telling two frames apart.
    Identity,
    /// By their bytes, so the hash means the same in every process.
    #[cfg_attr(not(feature = "snapshot"), expect(dead_code, reason = "only snapshots are keyed by content"))]
    Content,
}

/// Surveys `list` into `out`, reusing its storage.
pub(crate) fn survey(list: &DisplayList, images: &Images, fonts: &mut FontCache, out: &mut Survey) {
    out.clear();
    out.extend(list.items.iter().map(|item| entry(item, images, fonts, By::Identity)));
}

/// A hash of everything `list` draws, the same wherever and whenever it is taken.
#[cfg(feature = "snapshot")]
pub(crate) fn fingerprint(list: &DisplayList, images: &Images) -> u64 {
    let mut fonts = FontCache::default();
    let mut mix = Mix(list.items.len() as u64);
    list.items.iter().for_each(|item| mix.add(entry(item, images, &mut fonts, By::Content).hash));
    mix.finish()
}

/// A hash of some bytes, for [`fingerprint`]'s callers to fold more into a key.
#[cfg(feature = "snapshot")]
pub(crate) fn hash(parts: &[&[u8]]) -> u64 {
    let mut mix = Mix(parts.len() as u64);
    parts.iter().for_each(|part| mix.bytes(part));
    mix.finish()
}

fn entry(item: &Item, images: &Images, fonts: &mut FontCache, by: By) -> Entry {
    // Each kind of item starts from its own seed, so a fill never matches a stroke of the same shape.
    let mut mix;
    let mut flat = None;
    let (at, extent) = match item {
        Item::Fill { rect, color, radius } => {
            mix = Mix(1);
            mix.floats(&[rect.width, rect.height, *radius]);
            mix.color(*color);
            let extent = Extent::of(rect, 0.0);
            flat = (extent != Extent::Nowhere).then(|| Flat::of(rect, *radius));
            (Point { x: rect.x, y: rect.y }, extent)
        }
        Item::Stroke { rect, color, width, radius } => {
            mix = Mix(2);
            mix.floats(&[rect.width, rect.height, *width, *radius]);
            mix.color(*color);
            (Point { x: rect.x, y: rect.y }, Extent::of(rect, 0.0))
        }
        Item::Shadow { rect, color, blur, offset, radius } => {
            mix = Mix(3);
            mix.floats(&[rect.width, rect.height, *blur, offset.x, offset.y, *radius]);
            mix.color(*color);
            let moved = Rect::new(rect.x + offset.x, rect.y + offset.y, rect.width, rect.height);
            // Vello stops a blur at two and a half standard deviations, and the blur is two of them.
            (Point { x: rect.x, y: rect.y }, Extent::of(&moved, 1.25 * blur.max(0.0)))
        }
        Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, .. } => {
            mix = Mix(4);
            let face = fonts.face(font);
            match by {
                By::Identity => mix.add(face.id),
                By::Content => mix.bytes(&font.data),
            }
            mix.add(u64::from(font.index));
            for coord in font.coords.iter() {
                mix.add(*coord as u64);
            }
            mix.floats(&[*size, *skew]);
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
            (*origin, Extent::of(&run, 1.0))
        }
        Item::Path { origin, scale, path, color, stroke } => {
            mix = Mix(5);
            mix.floats(&[*scale, stroke.unwrap_or(-1.0)]);
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
            (*origin, Extent::of(&shape, 1.0))
        }
        Item::Image { rect, asset } => {
            mix = Mix(6);
            mix.floats(&[rect.width, rect.height]);
            // The picture is part of what the item draws: it changes when the asset is decoded or replaced.
            let extent = match images.get(asset) {
                Some(image) => {
                    match by {
                        By::Identity => mix.add(image.id),
                        By::Content => {
                            mix.add(u64::from(image.pixels.width()) << 32 | u64::from(image.pixels.height()));
                            mix.bytes(image.pixels.data_as_u8_slice());
                        }
                    }
                    Extent::of(rect, 0.0)
                }
                None => Extent::Nowhere,
            };
            (Point { x: rect.x, y: rect.y }, extent)
        }
        Item::PushClip { rect, radius } => {
            mix = Mix(7);
            mix.floats(&[rect.width, rect.height, *radius]);
            flat = Some(Flat::of(rect, *radius));
            (Point { x: rect.x, y: rect.y }, Extent::Anywhere)
        }
        Item::PopClip => {
            mix = Mix(8);
            (Point::default(), Extent::Anywhere)
        }
        // A part of the document shows nothing, so no change to one is a change to the frame.
        Item::Open { .. } | Item::Close => {
            mix = Mix(9);
            (Point::default(), Extent::Nowhere)
        }
    };
    let shape = mix.finish();
    let mut mix = Mix(shape);
    mix.floats(&[at.x, at.y]);
    Entry { hash: mix.finish(), shape, at: [at.x, at.y], extent, flat }
}

/// The most items two lists may differ by before the whole frame is repainted instead.
const MAX_EDITS: usize = 96;
/// The share of a frame above which repainting it whole is cheaper than repainting its parts.
const MAX_SHARE: f64 = 0.5;

/// How to make a kept frame show a new list.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Rectangles to fill, from the top down, with the pixels now `by` rows above them (below, when `by`
    /// is negative).
    pub moved: Vec<PixelRect>,
    pub by: i32,
    /// Rectangles to paint afresh once the pixels have moved.
    pub repaint: Vec<PixelRect>,
}

/// What became of an item of the new list.
#[derive(Clone, Copy, PartialEq)]
enum Fate {
    /// It draws what it drew.
    Kept,
    /// It draws the same thing this many rows lower, having been the old list's item at an index.
    Moved(i32, usize),
    /// It is new, or draws something else.
    Changed,
}

/// The plan that turns a `width` by `height` frame showing `old` into one showing `new`, or `None` to
/// repaint it all.
pub(crate) fn plan(old: &Survey, new: &Survey, scale: f32, width: u32, height: u32) -> Option<Plan> {
    // Most of two lists is the same at both ends, which leaves little to compare.
    let prefix = old.iter().zip(new).take_while(|(a, b)| a.hash == b.hash).count();
    let suffix = old[prefix..].iter().rev().zip(new[prefix..].iter().rev()).take_while(|(a, b)| a.hash == b.hash);
    let suffix = suffix.count();
    let (old, middle) = (&old[prefix..old.len() - suffix], &new[prefix..new.len() - suffix]);

    let mut fates = vec![Fate::Changed; middle.len()];
    let mut gone = vec![true; old.len()];
    for (was, is) in paired(old, middle)? {
        if old[was].hash == middle[is].hash {
            (fates[is], gone[was]) = (Fate::Kept, false);
        } else if let Some(by) = rows_between(&old[was], &middle[is], scale) {
            fates[is] = Fate::Moved(by, was);
        }
    }
    // One move can carry pixels with it: the one that carries the most. An item that moved any other way
    // was dropped from one place and added in another.
    let on_frame = |entry: &Entry| match entry.extent {
        Extent::Within(sides) => pixels(&sides, scale, width, height).map_or(0, |rect| rect.area()),
        _ => 0,
    };
    let mut carried: Vec<(i32, u64)> = Vec::new();
    for (fate, entry) in fates.iter().zip(middle) {
        if let Fate::Moved(by, _) = *fate {
            match carried.iter_mut().find(|(other, _)| *other == by) {
                Some((_, area)) => *area += on_frame(entry),
                None => carried.push((by, on_frame(entry))),
            }
        }
    }
    let by = carried.iter().filter(|(_, area)| *area > 0).max_by_key(|(_, area)| *area).map_or(0, |(by, _)| *by);
    for fate in &mut fates {
        match *fate {
            Fate::Moved(rows, was) if rows == by => gone[was] = false,
            Fate::Moved(..) => *fate = Fate::Changed,
            _ => {}
        }
    }

    let dropped = || old.iter().zip(&gone).filter(|(_, gone)| **gone).map(|(entry, _)| entry);
    let added = middle.iter().zip(&fates).filter(|(_, fate)| **fate == Fate::Changed).map(|(entry, _)| entry);
    let mut plan = Plan { by, ..Plan::default() };
    for entry in dropped().chain(added) {
        match entry.extent {
            Extent::Nowhere => {}
            Extent::Anywhere => return None,
            Extent::Within(sides) => plan.repaint.extend(pixels(&sides, scale, width, height)),
        }
    }
    if by != 0 {
        let fate =
            |index: usize| index.checked_sub(prefix).and_then(|index| fates.get(index)).copied().unwrap_or(Fate::Kept);
        let moves = Moves { new, old, fate: &fate, dropped: &mut dropped(), by: by.into(), scale };
        moves.plan(width, height, &mut plan);
    }
    merge(&mut plan.repaint);
    let share = plan.repaint.iter().map(PixelRect::area).sum::<u64>() as f64 / (f64::from(width) * f64::from(height));
    (share <= MAX_SHARE).then_some(plan)
}

/// How many whole rows of pixels lie between two entries of one shape, if they differ by that alone.
fn rows_between(was: &Entry, is: &Entry, scale: f32) -> Option<i32> {
    /// Rounding in an item's position may leave it this far from a whole row.
    const SLACK: f32 = 1.0 / 1024.0;
    let rows = (is.at[1] - was.at[1]) * scale;
    let whole = rows.round();
    let moved = was.at[0] == is.at[0] && (rows - whole).abs() <= SLACK && whole != 0.0 && whole.abs() < 65536.0;
    (moved && matches!((was.extent, is.extent), (Extent::Within(_), Extent::Within(_)))).then_some(whole as i32)
}

/// A run of rows or columns of physical pixels: from the first, up to the second.
type Span = (i64, i64);

fn meet(a: Span, b: Span) -> Span {
    (a.0.max(b.0), a.1.min(b.1))
}

fn empty(span: Span) -> bool {
    span.0 >= span.1
}

/// The spans sorted, with those that touch joined.
fn joined(mut spans: Vec<Span>) -> Vec<Span> {
    spans.retain(|span| !empty(*span));
    spans.sort_unstable();
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        match out.last_mut() {
            Some(last) if span.0 <= last.1 => last.1 = last.1.max(span.1),
            _ => out.push(span),
        }
    }
    out
}

/// The columns and rows an extent's ink can reach, with a pixel to spare as [`pixels`] has.
fn reach(sides: &[f32; 4], scale: f32) -> (Span, Span) {
    let edge = |side: f32| f64::from(side) * f64::from(scale);
    let span = |low: f32, high: f32| (edge(low).floor() as i64 - 1, edge(high).ceil() as i64 + 1);
    (span(sides[0], sides[2]), span(sides[1], sides[3]))
}

impl Flat {
    /// The rows the rectangle touches at all, and the rows it covers that are all alike.
    fn rows(&self, scale: f32) -> (Span, Span) {
        let edge = |side: f32| f64::from(side) * f64::from(scale);
        let outer = (edge(self.sides[1]).floor() as i64, edge(self.sides[3]).ceil() as i64);
        let alike = (edge(self.sides[1] + self.inset).ceil() as i64, edge(self.sides[3] - self.inset).floor() as i64);
        let top = alike.0.clamp(outer.0, outer.1);
        (outer, (top, alike.1.clamp(top, outer.1)))
    }

    fn columns(&self, scale: f32) -> Span {
        let edge = |side: f32| f64::from(side) * f64::from(scale);
        (edge(self.sides[0]).floor() as i64, edge(self.sides[2]).ceil() as i64)
    }
}

/// Two lists that differ by some of their items moving `by` rows down, and what became of each item.
struct Moves<'a> {
    new: &'a Survey,
    /// The part of the old list that [`Fate::Moved`] indexes.
    old: &'a [Entry],
    fate: &'a dyn Fn(usize) -> Fate,
    /// The old list's items that are gone from where they were.
    dropped: &'a mut dyn Iterator<Item = &'a Entry>,
    by: i64,
    scale: f32,
}

impl Moves<'_> {
    /// Adds to `plan` the pixels that can move with the items, and what must be painted instead.
    ///
    /// A pixel can move when everything under it and everything under where it came from is either an item
    /// that made the move, or a rectangle that looks the same on both rows.
    fn plan(self, width: u32, height: u32, plan: &mut Plan) {
        let (by, scale) = (self.by, self.scale);
        let frame = ((0, i64::from(width)), (0, i64::from(height)));

        // Where the moved items show, before and after: inside the frame and every clip around them.
        let mut clips: Vec<(Span, Span)> = Vec::new();
        let mut columns: Option<Span> = None;
        let mut touched = Vec::new();
        for (index, entry) in self.new.iter().enumerate() {
            let within = clips.last().copied().unwrap_or(frame);
            match ((self.fate)(index), entry.extent, entry.flat) {
                (_, Extent::Anywhere, Some(clip)) => {
                    clips.push((meet(within.0, clip.columns(scale)), meet(within.1, clip.rows(scale).0)));
                }
                (_, Extent::Anywhere, None) => drop(clips.pop()),
                (Fate::Moved(_, was), Extent::Within(is), _) => {
                    let Extent::Within(was) = self.old[was].extent else { continue };
                    for sides in [was, is] {
                        let (across, down) = reach(&sides, scale);
                        let (across, down) = (meet(across, within.0), meet(down, within.1));
                        if !empty(across) && !empty(down) {
                            touched.push(down);
                            columns = Some(columns.map_or(across, |all| (all.0.min(across.0), all.1.max(across.1))));
                        }
                    }
                }
                _ => {}
            }
        }
        let Some(columns) = columns else { return };

        // A pixel is spoiled when it or the pixel it came from is under something that did not make the move,
        // unless that is a rectangle both lie alike in. Spoiled pixels are painted again after the move.
        let both = |rows: Span| (rows.0.min(rows.0 + by), rows.1.max(rows.1 + by));
        let apart = |(outer, alike): (Span, Span)| [both((outer.0, alike.0)), both((alike.1, outer.1))];
        let mut spoiled: Vec<(Span, Span)> = Vec::new();
        // Each open clip, with the rows on which something inside it would otherwise be free to move.
        let mut open: Vec<(Flat, Option<Span>)> = Vec::new();
        let close = |(clip, inside): (Flat, Option<Span>), spoiled: &mut Vec<(Span, Span)>| {
            if let Some(inside) = inside {
                spoiled.extend(apart(clip.rows(scale)).map(|rows| (clip.columns(scale), meet(rows, inside))));
            }
        };
        let widen = |open: &mut Vec<(Flat, Option<Span>)>, rows: Span| {
            for (_, inside) in open {
                *inside = Some(inside.map_or(rows, |all| (all.0.min(rows.0), all.1.max(rows.1))));
            }
        };
        for (index, entry) in self.new.iter().enumerate() {
            match ((self.fate)(index), entry.extent, entry.flat) {
                (_, Extent::Anywhere, Some(clip)) => open.push((clip, None)),
                (_, Extent::Anywhere, None) => {
                    if let Some(clip) = open.pop() {
                        close(clip, &mut spoiled);
                    }
                }
                (Fate::Moved(..), Extent::Within(_), _) => widen(&mut open, (i64::MIN / 2, i64::MAX / 2)),
                (Fate::Kept, Extent::Within(sides), flat) => {
                    let (across, down) = reach(&sides, scale);
                    match flat {
                        _ if empty(meet(across, columns)) => {}
                        Some(flat) => {
                            let (outer, alike) = flat.rows(scale);
                            spoiled.extend(apart((outer, alike)).map(|rows| (across, rows)));
                            let still_alike = meet(alike, (alike.0 + by, alike.1 + by));
                            if !empty(still_alike) {
                                widen(&mut open, still_alike);
                            }
                        }
                        None => spoiled.extend([(across, down), (across, (down.0 + by, down.1 + by))]),
                    }
                }
                // What is new or altered is painted where it is in any case.
                _ => {}
            }
        }
        open.into_iter().for_each(|clip| close(clip, &mut spoiled));
        for entry in self.dropped {
            if let Extent::Within(sides) = entry.extent {
                let (across, down) = reach(&sides, scale);
                spoiled.push((across, (down.0 + by, down.1 + by)));
            }
        }

        let rect = |(across, down): (Span, Span)| PixelRect {
            x: across.0 as u32,
            y: down.0 as u32,
            width: (across.1 - across.0) as u32,
            height: (down.1 - down.0) as u32,
        };
        // Rows whose pixels would come from off the frame are painted instead.
        let sourced = (frame.1.0 + by, frame.1.1 + by);
        for rows in joined(touched) {
            let movable = meet(rows, sourced);
            if empty(movable) {
                plan.repaint.push(rect((columns, rows)));
                continue;
            }
            plan.moved.push(rect((columns, movable)));
            plan.repaint.extend(
                [(rows.0, movable.0), (movable.1, rows.1)]
                    .into_iter()
                    .filter(|rows| !empty(*rows))
                    .map(|rows| rect((columns, rows))),
            );
            for (across, down) in &spoiled {
                let spoiled = (meet(*across, columns), meet(*down, movable));
                if !empty(spoiled.0) && !empty(spoiled.1) {
                    plan.repaint.push(rect(spoiled));
                }
            }
        }
    }
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
pub(crate) fn merge(rects: &mut Vec<PixelRect>) {
    /// Pixels worth repainting needlessly to save a separate rectangle.
    const SLACK: u64 = 4096;
    let joins = |a: &PixelRect, b: &PixelRect| a.union(b).area() <= a.area() + b.area() + SLACK;
    let mut at = 0;
    while at < rects.len() {
        match (at + 1..rects.len()).find(|&other| joins(&rects[at], &rects[other])) {
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

/// The longest run of entries of one shape that `old` and `new` share in order, as pairs of their indices,
/// by Myers' algorithm. `None` if more than [`MAX_EDITS`] entries are outside it.
fn paired(old: &[Entry], new: &[Entry]) -> Option<Vec<(usize, usize)>> {
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
            while x < rows && x - k < columns && old[x as usize].shape == new[(x - k) as usize].shape {
                x += 1;
            }
            furthest[at(k)] = x;
            if x >= rows && x - k >= columns {
                return Some(trace(&history, at, edits, (rows, columns)));
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

/// Walks a finished search back from its `end`, collecting the pairs each run of shared entries holds.
fn trace(
    history: &[Vec<isize>],
    at: impl Fn(isize) -> usize,
    edits: isize,
    end: (isize, isize),
) -> Vec<(usize, usize)> {
    let mut pairs = Vec::new();
    let (mut x, mut y) = end;
    for edits in (0..=edits).rev() {
        let furthest = &history[edits as usize];
        let k = x - y;
        // Where the step onto this diagonal landed, before the run of shared entries after it.
        let landed = if steps_down(furthest, &at, k, edits) { furthest[at(k + 1)] } else { furthest[at(k - 1)] + 1 };
        while x > landed {
            (x, y) = (x - 1, y - 1);
            pairs.push((x as usize, y as usize));
        }
        if steps_down(furthest, &at, k, edits) {
            y -= 1;
        } else {
            x -= 1;
        }
    }
    pairs.reverse();
    pairs
}
