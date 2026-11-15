//! What a selected picture offers on the page: corners to resize it by, and being carried to another place.

use letters_model::BlockId;
use letters_render::{DisplayList, Item, Point, Rect};

use crate::chrome::Edge;
use crate::color::resolve;
use crate::tokens::{self, Color as Ink, Scheme};

/// A handle's side, how far its middle stands out from the corner it is at, and how far round it a press
/// still takes hold of it.
const HANDLE: (f32, f32, f32) = (9.0, 2.5, 5.0);

/// The least the shorter side of a picture is dragged to.
const LEAST: f32 = 24.0;

/// The most a picture measures while it is carried under the pointer, and where it hangs from the pointer.
const CARRIED: (f32, f32, f32) = (160.0, 120.0, 12.0);

/// A corner of a picture: which way it lies from the middle, across and down, each -1 or 1.
pub(crate) type Corner = (f32, f32);

const CORNERS: [Corner; 4] = [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)];

fn grown(rect: Rect, by: f32) -> Rect {
    Rect::new(rect.x - by, rect.y - by, rect.width + 2.0 * by, rect.height + 2.0 * by)
}

/// Where the handle at a corner of `image` is drawn.
fn handle(image: Rect, (across, down): Corner) -> Rect {
    let x = image.x + if across > 0.0 { image.width } else { 0.0 } + across * HANDLE.1;
    let y = image.y + if down > 0.0 { image.height } else { 0.0 } + down * HANDLE.1;
    Rect::new(x - HANDLE.0 / 2.0, y - HANDLE.0 / 2.0, HANDLE.0, HANDLE.0)
}

/// The corner of `image` whose handle a press at `at` takes hold of.
pub(crate) fn corner_at(image: Rect, at: Point) -> Option<Corner> {
    CORNERS.into_iter().find(|corner| grown(handle(image, *corner), HANDLE.2).contains(at))
}

/// The shape of the pointer over a corner's handle.
pub(crate) fn edge((across, down): Corner) -> Edge {
    match (across > 0.0, down > 0.0) {
        (false, false) => Edge::NorthWest,
        (true, false) => Edge::NorthEast,
        (false, true) => Edge::SouthWest,
        (true, true) => Edge::SouthEast,
    }
}

/// Draws the handles at the corners of a selected picture.
pub(crate) fn handles(image: Rect, accent: Ink, scheme: Scheme, out: &mut DisplayList) {
    for corner in CORNERS {
        let rect = handle(image, corner);
        out.push(Item::Fill { rect, color: resolve(accent, scheme, 1.0), radius: 2.0 });
        out.push(Item::Fill { rect: grown(rect, -2.0), color: resolve(tokens::SURFACE, scheme, 1.0), radius: 0.5 });
    }
}

/// A picture being resized by one of its corners.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Resizing {
    pub figure: BlockId,
    pub corner: Corner,
    /// Where the corner was taken hold of.
    from: Point,
    /// The picture's place then.
    image: Rect,
    /// The place the picture would have were it let go now.
    now: Rect,
}

impl Resizing {
    pub fn start(figure: BlockId, image: Rect, corner: Corner, from: Point) -> Self {
        Self { figure, corner, from, image, now: image }
    }

    /// Follows the pointer. The picture keeps its shape and the corner across from the one that is held,
    /// and grows no wider than `widest`.
    pub fn drag(&mut self, at: Point, widest: f32) {
        let Rect { x, y, width, height } = self.image;
        let (across, down) = self.corner;
        let wider = (width + (at.x - self.from.x) * across) / width.max(1.0);
        let taller = (height + (at.y - self.from.y) * down) / height.max(1.0);
        // A picture that is already beyond a bound is not pulled back to it by a press alone.
        let least = (LEAST / width.min(height).max(1.0)).min(1.0);
        let most = (widest / width.max(1.0)).max(1.0);
        // The way the pointer went furthest sets the size, so a corner pulled straight across resizes too.
        let by = if (wider - 1.0).abs() > (taller - 1.0).abs() { wider } else { taller };
        let by = by.clamp(least, most);
        let (wide, high) = ((width * by).round(), (height * by).round());
        let left = if across < 0.0 { x + width - wide } else { x };
        let top = if down < 0.0 { y + height - high } else { y };
        self.now = Rect::new(left, top, wide, high);
    }

    /// The size to give the picture, when the drag changed it.
    pub fn size(&self) -> Option<(f32, f32)> {
        (self.now != self.image).then_some((self.now.width, self.now.height))
    }

    /// Draws the outline of the size the picture would take, with its handles.
    pub fn paint(&self, accent: Ink, scheme: Scheme, out: &mut DisplayList) {
        out.push(Item::Stroke { rect: self.now, color: resolve(accent, scheme, 1.0), width: 2.0, radius: 2.0 });
        handles(self.now, accent, scheme, out);
    }
}

/// A picture that is pressed, and carried once the pointer has gone far enough to mean it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Moving {
    pub figure: BlockId,
    /// Where it was pressed, in the page area.
    from: Point,
    /// Where the pointer has carried it to.
    at: Option<Point>,
}

impl Moving {
    pub fn start(figure: BlockId, from: Point) -> Self {
        Self { figure, from, at: None }
    }

    /// Follows the pointer, which carries the picture once it is `slop` from where it pressed. Returns
    /// whether the picture is carried.
    pub fn drag(&mut self, at: Point, slop: f32) -> bool {
        let far = (at.x - self.from.x).hypot(at.y - self.from.y) > slop;
        if far || self.at.is_some() {
            self.at = Some(at);
        }
        self.at.is_some()
    }

    /// Where the picture is carried to, once it is.
    pub fn carried(&self) -> Option<Point> {
        self.at
    }
}

/// Where a picture of the size of `image` is drawn while it is carried at `at`, and how much smaller.
pub(crate) fn carried(image: Rect, at: Point) -> (Rect, f32) {
    let by = (CARRIED.0 / image.width.max(1.0)).min(CARRIED.1 / image.height.max(1.0)).min(1.0);
    let (wide, high) = ((image.width * by).round(), (image.height * by).round());
    (Rect::new(at.x + CARRIED.2, at.y + CARRIED.2, wide, high), by)
}

/// What the pointer has hold of on a picture.
#[derive(Default)]
pub(crate) struct Grip {
    /// The corner of the selected picture the pointer is over.
    pub over: Option<Corner>,
    pub resizing: Option<Resizing>,
    pub moving: Option<Moving>,
}

impl Grip {
    /// Lets go of everything, as Escape does. Returns whether a picture was held by a corner or carried.
    pub fn release(&mut self) -> bool {
        let carried = self.moving.take().is_some_and(|moving| moving.carried().is_some());
        self.resizing.take().is_some() || carried
    }

    /// The corner whose shape the pointer takes.
    pub fn corner(&self) -> Option<Corner> {
        self.resizing.as_ref().map(|resizing| resizing.corner).or(self.over)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corner_is_dragged_to_a_size_of_the_same_shape_between_the_bounds() {
        let image = Rect::new(100.0, 50.0, 200.0, 100.0);
        // The corner, how far the pointer went, and the place the picture would take.
        let cases = [
            ((1.0, 1.0), (50.0, 0.0), Rect::new(100.0, 50.0, 250.0, 125.0)),
            ((1.0, 1.0), (0.0, 50.0), Rect::new(100.0, 50.0, 300.0, 150.0)),
            ((-1.0, -1.0), (-100.0, 0.0), Rect::new(0.0, 0.0, 300.0, 150.0)),
            ((-1.0, 1.0), (100.0, 0.0), Rect::new(200.0, 50.0, 100.0, 50.0)),
            // No wider than the text, and no smaller than can still be taken hold of.
            ((1.0, -1.0), (900.0, 0.0), Rect::new(100.0, -100.0, 500.0, 250.0)),
            ((1.0, 1.0), (-900.0, -900.0), Rect::new(100.0, 50.0, 48.0, 24.0)),
        ];
        for (corner, (across, down), now) in cases {
            let from = Point { x: 10.0, y: 10.0 };
            let mut resizing = Resizing::start("g".into(), image, corner, from);
            assert_eq!(resizing.size(), None, "a press alone changes nothing");
            resizing.drag(Point { x: from.x + across, y: from.y + down }, 500.0);
            assert_eq!(resizing.now, now, "{corner:?} by {across}, {down}");
            assert_eq!(resizing.size(), Some((now.width, now.height)));
        }
    }

    #[test]
    fn a_press_takes_hold_of_the_corner_it_is_nearest_and_of_none_from_the_middle() {
        let image = Rect::new(100.0, 50.0, 200.0, 100.0);
        let cases = [
            (Point { x: 98.0, y: 48.0 }, Some((-1.0, -1.0))),
            (Point { x: 305.0, y: 154.0 }, Some((1.0, 1.0))),
            (Point { x: 296.0, y: 52.0 }, Some((1.0, -1.0))),
            (Point { x: 200.0, y: 100.0 }, None),
            (Point { x: 200.0, y: 48.0 }, None),
        ];
        for (at, corner) in cases {
            assert_eq!(corner_at(image, at), corner, "{at:?}");
        }
    }
}
