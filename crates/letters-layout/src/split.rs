//! Two versions of a document on one page.
//!
//! Where both versions hold a block that differs, the page is cut in three: the part above, the block's two
//! versions as segments of the page, the part below. A segment has the page's own text measure, so its lines
//! break as they do on the page. What the versions share shows once, as the second has it.
//!
//! A [`Split`] is worked out from two layouts as they stand and keeps nothing of them. Make it again after
//! either changes, which costs a lookup for each block that is split.

use letters_model::{BlockId, Pos};
use letters_render::{DisplayList, Item, Point, Rect};

use crate::DocLayout;

/// From how many widths of one version a window keeps two side by side.
const PINNED: f32 = 1.25;

/// One of the two versions that a split shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    /// The earlier one: the left of a pair and the top of a stack. Before, in history.
    First,
    /// The later one, which is also where the page comes from that both versions share. After, in history.
    Second,
}

/// How the two versions of a block share a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrangement {
    /// Side by side, both whole.
    Whole,
    /// Side by side, with the window on one end of the pair: the active version whole against its margin,
    /// the other as far as the window reaches.
    Pinned,
    /// The first above the second, each at the page's width.
    Stacked,
}

impl Arrangement {
    /// What a window `room` wide does with two versions that each need `version`: its text measure and a
    /// margin on both sides. There is room for both, or for one and a quarter, or for less.
    pub fn of(room: f32, version: f32) -> Self {
        if room >= 2.0 * version {
            Self::Whole
        } else if room >= PINNED * version {
            Self::Pinned
        } else {
            Self::Stacked
        }
    }
}

/// The window that a split is shown in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// The width there is for the page, beside any open panel.
    pub width: f32,
    /// The margin on either side of a version's text where two stand side by side.
    pub margin: f32,
    /// The room above each version of a block for the line that says whose it is.
    pub head: f32,
    /// The version that holds the caret, or the second where neither does.
    pub active: Side,
    /// How far a pinned pair has slid to its active version's end, from 0 at the other end to 1 when it is
    /// there. To animate a change of `active`, step this from 0 to 1.
    pub slide: f32,
}

/// One segment of a split page.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    /// The version it shows. A stretch that both versions share shows the second.
    pub side: Side,
    /// The block it shows one version of, or `None` for a stretch of the page that both versions share.
    /// A block inside a table, a quote or a callout shows with all of that, under the first of the blocks
    /// asked for that lies in it.
    pub block: Option<BlockId>,
    /// Where it is in the window, with its margins and the room at its head.
    pub rect: Rect,
    /// The room at the top of `rect` for the line that says whose version it is. A shared stretch has none.
    pub head: f32,
    /// Where it shows the top left of its version's column.
    pub origin: Point,
    /// The part of its version's column that it shows: where that starts and ends, from the column's top.
    pub band: (f32, f32),
}

impl Piece {
    /// The part of the window its text shows in, under the room at its head.
    fn text(&self) -> Rect {
        Rect::new(self.rect.x, self.rect.y + self.head, self.rect.width, self.rect.height - self.head)
    }

    /// The part of its version's column that a viewport, a part of the window, shows.
    fn cut(&self, viewport: Rect) -> Option<Rect> {
        let top = (viewport.y - self.origin.y).max(self.band.0);
        let bottom = (viewport.y + viewport.height - self.origin.y).min(self.band.1);
        (top < bottom).then(|| Rect::new(viewport.x - self.origin.x, top, viewport.width, bottom - top))
    }
}

/// Two versions of a document as one page, in a window. See the [module](self).
#[derive(Clone, Debug, PartialEq)]
pub struct Split {
    pub arrangement: Arrangement,
    /// The height of the whole page as it is split.
    pub height: f32,
    /// The segments from the top down. The two versions of a block follow one another, first then second.
    pub pieces: Vec<Piece>,
}

impl Split {
    /// Splits the page at each of `blocks` that both versions hold. The layouts are to be of one width, the
    /// page's text measure. A block in only one version is not split: it shows as the second has it, or not
    /// at all.
    ///
    /// Heights that a layout still estimates are taken as they are. To have them exact where it matters,
    /// settle each layout on what [`Split::shown`] gives for the viewport, and split again.
    pub fn new(first: &DocLayout, second: &DocLayout, blocks: &[BlockId], window: &Window) -> Self {
        let (measure, margin, head) = (second.width(), window.margin.max(0.0), window.head.max(0.0));
        let (room, version) = (window.width.max(1.0), measure + 2.0 * margin);
        let arrangement = Arrangement::of(room, version);

        // The stretch of each column that every split block takes, down the second's column. Two blocks
        // under one top-level block are one stretch.
        let mut pairs: Vec<_> =
            blocks.iter().filter_map(|block| Some((block, first.band(block)?, second.band(block)?))).collect();
        pairs.sort_by(|(_, _, a), (_, _, b)| a.0.total_cmp(&b.0));
        pairs.dedup_by(|(_, _, later), (_, _, earlier)| later.0 < earlier.1);

        // The page's text stands in the middle of the window, and a pair in the middle or at one end.
        let centre = ((room - measure) / 2.0).max(0.0);
        let left = match arrangement {
            Arrangement::Pinned => {
                let ends = (0.0, room - 2.0 * version);
                let (from, to) = if window.active == Side::First { (ends.1, ends.0) } else { ends };
                from + (to - from) * window.slide.clamp(0.0, 1.0)
            }
            _ => (room - 2.0 * version) / 2.0,
        };

        let mut pieces = Vec::with_capacity(3 * pairs.len() + 1);
        // How far down the window the pieces reach, and how far down the second's column they have shown.
        let (mut y, mut shown) = (0.0, 0.0);
        let shared = |pieces: &mut Vec<Piece>, y: &mut f32, band: (f32, f32)| {
            if band.1 > band.0 {
                let rect = Rect::new(0.0, *y, room, band.1 - band.0);
                let origin = Point { x: centre, y: *y - band.0 };
                pieces.push(Piece { side: Side::Second, block: None, rect, head: 0.0, origin, band });
                *y += rect.height;
            }
        };
        for (block, of_first, of_second) in pairs {
            shared(&mut pieces, &mut y, (shown, of_second.0));
            let sides = [(Side::First, of_first), (Side::Second, of_second)];
            let tall = |band: (f32, f32)| head + band.1 - band.0;
            let row = tall(of_first).max(tall(of_second));
            for (index, (side, band)) in sides.into_iter().enumerate() {
                let rect = match arrangement {
                    Arrangement::Stacked => Rect::new(0.0, y, room, tall(band)),
                    _ => Rect::new(left + index as f32 * version, y, version, row),
                };
                let x = if arrangement == Arrangement::Stacked { centre } else { rect.x + margin };
                let origin = Point { x, y: y + head - band.0 };
                pieces.push(Piece { side, block: Some(block.clone()), rect, head, origin, band });
                // A stack goes down with each version, and a pair with its second.
                if arrangement == Arrangement::Stacked || side == Side::Second {
                    y += rect.height;
                }
            }
            shown = of_second.1;
        }
        shared(&mut pieces, &mut y, (shown, second.height()));
        Self { arrangement, height: y, pieces }
    }

    /// Emits what `viewport`, a part of the window, shows of the page. Each piece is cut to its place.
    pub fn paint(&self, first: &DocLayout, second: &DocLayout, viewport: Rect, out: &mut DisplayList) {
        for piece in self.pieces.iter().filter(|piece| piece.text().intersects(&viewport)) {
            let Some(shown) = piece.cut(viewport) else { continue };
            out.push(Item::PushClip { rect: piece.text(), radius: 0.0 });
            let from = out.items.len();
            let layout = if piece.side == Side::First { first } else { second };
            layout.paint(shown, out);
            out.translate(from, piece.origin);
            out.push(Item::PopClip);
        }
    }

    /// The version shown at a point of the window, or the nearest to it, and the position there. A point on
    /// the room at a piece's head, or beside its text, is at the nearest text of that piece.
    pub fn hit(&self, first: &DocLayout, second: &DocLayout, point: Point) -> Option<(Side, Pos)> {
        let last = self.pieces.last()?;
        // The pieces across the window at the point's height: one, or the two versions of a pair.
        let below = self.pieces.partition_point(|piece| piece.rect.y + piece.rect.height <= point.y);
        let top = self.pieces.get(below).unwrap_or(last).rect.y;
        let row = self.pieces.iter().filter(|piece| piece.rect.y == top);
        let off = |piece: &&Piece| (piece.rect.x - point.x).max(point.x - piece.rect.x - piece.rect.width).max(0.0);
        let piece = row.min_by(|a, b| off(a).total_cmp(&off(b)))?;
        let layout = if piece.side == Side::First { first } else { second };
        let within = Point { x: point.x - piece.origin.x, y: point.y - piece.origin.y };
        Some((piece.side, layout.hit_in(within, piece.band)?))
    }

    /// Where the window shows the top left of a version's column, for what is `y` down that column. Add it to
    /// a caret or a selection's rectangle to draw that in the window. `None` for a part of the first version
    /// that the page does not show, because the second has the same.
    pub fn origin(&self, side: Side, y: f32) -> Option<Point> {
        let of = |piece: &&Piece| piece.side == side && piece.band.0 <= y && y < piece.band.1;
        self.pieces.iter().find(of).map(|piece| piece.origin)
    }

    /// The part of a version's column that `viewport` shows, for [`DocLayout::settle`]. `None` when it shows
    /// nothing of that version.
    pub fn shown(&self, side: Side, viewport: Rect) -> Option<Rect> {
        let cuts = self.pieces.iter().filter(|piece| piece.side == side).filter_map(|piece| piece.cut(viewport));
        cuts.reduce(|all, cut| {
            let (top, bottom) = (all.y.min(cut.y), (all.y + all.height).max(cut.y + cut.height));
            Rect::new(all.x.min(cut.x), top, all.width.max(cut.width), bottom - top)
        })
    }
}
