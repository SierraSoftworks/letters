//! References to figures: what the pointer does with one on the page.
//!
//! A reference reads as its figure's label, such as "Figure 2", and says which figure it means while the
//! pointer rests on it. A figure in view has a band of light run over it, as the shapes of yewi's skeleton
//! have. One out of view shows small beside the reference, over its caption. A press on the reference
//! brings the figure into view.

use std::ops::Range;

use letters_layout::{DocLayout, figure_label, figure_numbers};
use letters_model::{Block, BlockId, BlockKind, Document, Mark, Pos, Selection, Style};
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};

use crate::color::resolve;
use crate::layout::position;
use crate::motion::{self, EASE_IN_OUT, FRAME_MS};
use crate::style::Align;
use crate::text::{TextShaper, TextStyle};
use crate::theme::accent;
use crate::tokens::{self, Scheme};
use crate::widgets::{Placement, Side};

/// How long the light takes to cross a figure, in milliseconds: as long as it takes a shape of yewi's
/// skeleton.
const PASS_MS: u64 = (tokens::DURATION * 9.0) as u64;
/// How much of the light's colour shows where the band is brightest.
const LIGHT: f32 = 0.22;
/// The strips the band is drawn in, each a little brighter than the last up to the middle.
const STRIPS: usize = 32;
/// How far the mark around a figure stands off it, for someone who asked for less motion.
const MARK_GAP: f32 = 4.0;

/// The most room a figure's picture takes in a preview.
const THUMB: Size = Size { width: 280.0, height: 180.0 };
/// The least width a preview's caption wraps at, under a narrow picture.
const CAPTION_WIDTH: f32 = 200.0;
/// The most characters of a caption a preview shows: about three lines.
const CAPTION: usize = 120;
const CAPTION_TEXT: TextStyle = TextStyle::control().size(tokens::TEXT_SM).leading(1.4);
const PADDING: f32 = tokens::SPACE_SM;
/// The room between a reference and its preview.
const GAP: f32 = 6.0;

/// A reference on the page that names a numbered figure.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reference {
    /// The figure or gallery it names.
    pub target: BlockId,
    /// The block it is written in, and its bytes there.
    block: BlockId,
    range: Range<usize>,
}

impl Reference {
    /// The reference whose label is drawn at a point of the layout. One whose figure is gone or has no
    /// number reads "Figure ?" and leads nowhere, so it is none.
    pub fn at(doc: &Document, layout: &DocLayout, at: Point) -> Option<Self> {
        let pos = layout.hit(at)?;
        let block = doc.block(&pos.block)?;
        let mut start = 0;
        for run in block.body.runs() {
            let range = start..start + run.text.len();
            start = range.end;
            let target = run.marks.iter().find_map(|mark| match mark {
                Mark::Ref(target) => Some(target),
                _ => None,
            });
            // A point on a label is nearest to one of its two ends, so most runs need no measuring.
            let near = !range.is_empty() && (range.start..=range.end).contains(&pos.offset);
            let Some(target) = target.filter(|_| near) else { continue };
            let found = Self { target: target.clone(), block: block.id.clone(), range };
            if found.lines(layout).iter().any(|line| line.contains(at)) {
                return figure_numbers(doc).iter().any(|(numbered, _)| numbered == target).then_some(found);
            }
        }
        None
    }

    /// Where the label is drawn: a rectangle for each line it is on.
    pub fn lines(&self, layout: &DocLayout) -> Vec<Rect> {
        let end = |offset| Pos::new(self.block.clone(), offset);
        layout.selection(&Selection { anchor: end(self.range.start), head: end(self.range.end) })
    }
}

/// A band of light on its way over a figure.
struct Light {
    target: BlockId,
    /// When it set out, once a frame has said so, and the time of the last frame.
    from: Option<u64>,
    now: u64,
    /// How long it runs, in milliseconds. `None` for as long as the pointer keeps it going.
    until: Option<u64>,
}

/// What the pointer is doing with the references on the page.
#[derive(Default)]
pub(crate) struct References {
    /// Where the pointer rests on the page area.
    at: Option<Point>,
    /// The reference it rests on, and whether that one's figure is in view.
    pointed: Option<(Reference, bool)>,
    /// The reference the button went down on, and where.
    pressed: Option<(Reference, Point)>,
    light: Option<Light>,
    reduced_motion: bool,
}

impl References {
    pub fn set_reduced_motion(&mut self, reduced: bool) {
        self.reduced_motion = reduced;
    }

    /// Where the pointer rests on the page area, as the window was last told.
    pub fn rest(&self) -> Option<Point> {
        self.at
    }

    pub fn set_rest(&mut self, at: Option<Point>) {
        self.at = at;
    }

    /// Takes the reference the pointer rests on now, and whether its figure is in view. Returns whether
    /// either is news.
    pub fn point(&mut self, pointed: Option<(Reference, bool)>) -> bool {
        if pointed == self.pointed {
            return false;
        }
        match &pointed {
            Some((reference, true)) => self.hold(&reference.target),
            _ => self.release(),
        }
        self.pointed = pointed;
        true
    }

    /// The reference under the pointer.
    pub fn pointed(&self) -> Option<&Reference> {
        self.pointed.as_ref().map(|(reference, _)| reference)
    }

    /// The reference under the pointer whose figure is out of view, and so shows beside it.
    pub fn previewed(&self) -> Option<&Reference> {
        self.pointed.as_ref().filter(|(_, in_view)| !in_view).map(|(reference, _)| reference)
    }

    /// Notes the reference the button went down on, and where in the page area. `None` for a press
    /// anywhere else.
    pub fn press(&mut self, pressed: Option<(Reference, Point)>) {
        self.pressed = pressed;
    }

    /// The figure to bring into view, when the button comes up at `at` on the `reference` it went down on
    /// without having dragged anything.
    pub fn followed(&mut self, at: Point, slop: f32, reference: Option<&Reference>) -> Option<BlockId> {
        let (pressed, from) = self.pressed.take()?;
        let still = (at.x - from.x).abs() <= slop && (at.y - from.y).abs() <= slop;
        (still && reference == Some(&pressed)).then_some(pressed.target)
    }

    /// Keeps the light running over a figure for as long as the pointer rests on a reference to it.
    fn hold(&mut self, target: &BlockId) {
        match &mut self.light {
            Some(light) if light.target == *target => light.until = None,
            _ => self.light = Some(Light { target: target.clone(), from: None, now: 0, until: None }),
        }
    }

    /// Lets the light the pointer kept going finish the pass it is on. A mark that does not move goes at
    /// once.
    fn release(&mut self) {
        if self.reduced_motion && self.light.as_ref().is_some_and(|light| light.until.is_none()) {
            self.light = None;
        }
        if let Some(light) = self.light.as_mut().filter(|light| light.until.is_none()) {
            let gone = light.from.map_or(0, |from| light.now.saturating_sub(from));
            light.until = Some((gone / PASS_MS + 1) * PASS_MS);
        }
    }

    /// Runs the light over a figure once, as when the page has just been brought to it.
    pub fn flash(&mut self, target: &BlockId) {
        let held = self.light.as_ref().is_some_and(|light| light.target == *target && light.until.is_none());
        if !held {
            self.light = Some(Light { target: target.clone(), from: None, now: 0, until: Some(PASS_MS) });
        }
    }

    /// Moves the light on. Returns when to call again, or `None` once nothing moves.
    pub fn animate(&mut self, now_ms: u64) -> Option<u64> {
        let light = self.light.as_mut()?;
        let from = *light.from.get_or_insert(now_ms);
        light.now = now_ms;
        match light.until {
            Some(until) if now_ms.saturating_sub(from) >= until => {
                self.light = None;
                None
            }
            // Nothing moves for someone who asked for less motion: the figure is marked until then.
            Some(until) if self.reduced_motion => Some(from + until),
            None if self.reduced_motion => None,
            _ => Some(now_ms + FRAME_MS),
        }
    }

    /// The figure the light is on, and how far over it the light has come: from 0, about to reach its left
    /// edge, to 1, gone past its right. `None` for the way stands for a mark that does not move.
    pub fn light(&self) -> Option<(&BlockId, Option<f32>)> {
        let light = self.light.as_ref()?;
        if self.reduced_motion {
            return Some((&light.target, None));
        }
        let gone = light.now.saturating_sub(light.from?);
        let pass = (gone % PASS_MS) as f32 / PASS_MS as f32;
        Some((&light.target, Some(motion::curve(EASE_IN_OUT, pass))))
    }
}

/// Draws the light `progress` of its way over a figure: a soft band as wide as the figure, that comes in from
/// the left and leaves on the right, as over a shape of yewi's skeleton. It shows on the figure's `pictures`
/// alone, and not on the page between those of a gallery. `scale` is the device pixels in a logical one,
/// which the band's strips keep to so that none shows an edge.
pub(crate) fn light(figure: Rect, pictures: &[Rect], progress: f32, scheme: Scheme, scale: f32, out: &mut DisplayList) {
    let snap = |x: f32| (x * scale).round() / scale;
    let left = figure.x + (2.0 * progress - 1.0) * figure.width;
    let strip = figure.width / STRIPS as f32;
    for n in 0..STRIPS {
        let (start, end) = (snap(left + n as f32 * strip), snap(left + (n + 1) as f32 * strip));
        // Brightest in the middle of the band, and nothing at its edges.
        let share = 1.0 - ((n as f32 + 0.5) / STRIPS as f32 * 2.0 - 1.0).abs();
        let color = resolve(accent().base, scheme, LIGHT * share);
        for picture in pictures {
            let (from, to) = (start.max(picture.x), end.min(picture.x + picture.width));
            if from < to {
                let rect = Rect::new(from, picture.y, to - from, picture.height);
                out.push(Item::Fill { rect, color, radius: 0.0 });
            }
        }
    }
}

/// Marks a picture without anything moving, for someone who asked for less motion: a line around it.
pub(crate) fn mark(picture: Rect, scheme: Scheme, out: &mut DisplayList) {
    let rect = Rect::new(
        picture.x - MARK_GAP,
        picture.y - MARK_GAP,
        picture.width + 2.0 * MARK_GAP,
        picture.height + 2.0 * MARK_GAP,
    );
    out.push(Item::Stroke { rect, color: resolve(accent().base, scheme, 1.0), width: 2.0, radius: MARK_GAP });
}

/// What a preview says under a figure's picture: its label and caption, cut short when it runs on. `None`
/// for a block that is no numbered figure.
pub(crate) fn caption(doc: &Document, style: &Style, target: &BlockId) -> Option<String> {
    let number = figure_numbers(doc).into_iter().find(|(numbered, _)| numbered == target)?.1;
    let block = doc.block(target)?;
    let said = match block.kind {
        // A gallery has one caption for all its pictures: the last, should a merge have left more.
        BlockKind::Gallery { .. } => {
            let caption = block.body.blocks().iter().rfind(|part| part.kind == BlockKind::Caption);
            caption.map(Block::plain_text).unwrap_or_default()
        }
        _ => block.plain_text(),
    };
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    let label = figure_label(style, number);
    if said.is_empty() {
        return Some(label.trim_end_matches(':').to_owned());
    }
    let whole = format!("{label} {said}");
    if whole.chars().count() <= CAPTION {
        return Some(whole);
    }
    let cut: String = whole.chars().take(CAPTION - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// A figure that is out of view, shown small beside a reference to it.
pub(crate) struct Preview<'a> {
    pub layout: &'a DocLayout,
    /// Where the layout draws the figure's picture.
    pub picture: Rect,
    /// The page's colour, which shows between the pictures of a gallery.
    pub page: Color,
    pub caption: &'a str,
    /// The line of the reference the pointer is on, in the page area.
    pub anchor: Rect,
    /// The size of the page area, which the preview stays inside.
    pub room: Size,
}

impl Preview<'_> {
    /// Draws the preview as a tooltip is drawn: yewi's bubble, under the reference or over it when there
    /// is no room below. Returns where it is.
    pub fn paint(&self, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) -> Rect {
        let picture = self.picture;
        let by = (THUMB.width / picture.width).min(THUMB.height / picture.height).min(1.0);
        let thumb = Size { width: (picture.width * by).round(), height: (picture.height * by).round() };
        let caption = shaper.shape(self.caption, &CAPTION_TEXT, Some(thumb.width.max(CAPTION_WIDTH)));
        let width = thumb.width.max(caption.size.width);
        // The bubble's border is inside its size, as a tooltip's is.
        let frame = 2.0 * (PADDING + 1.0);
        let size = Size { width: width + frame, height: thumb.height + tokens::SPACE_SM + caption.size.height + frame };
        let placement = Placement { side: Side::Bottom, align: Align::Start, gap: GAP, at: None };
        let rect = position(placement, self.anchor, size, self.room);

        let color = |ink| resolve(ink, scheme, 1.0);
        let radius = tokens::RADIUS_CONTROL;
        for shadow in tokens::SHADOW_CARD {
            let offset = Point { x: shadow.x, y: shadow.y };
            out.push(Item::Shadow { rect, color: color(shadow.color), blur: shadow.blur, offset, radius });
        }
        out.push(Item::Fill { rect, color: color(tokens::SURFACE_RAISED), radius });
        out.push(Item::Stroke { rect, color: color(tokens::CARD_LINE), width: 1.0, radius });

        // The picture is what the layout paints where the figure is, made small.
        let inner = Point { x: rect.x + PADDING + 1.0, y: rect.y + PADDING + 1.0 };
        let at = Rect::new(inner.x + ((width - thumb.width) / 2.0).round(), inner.y, thumb.width, thumb.height);
        out.push(Item::PushClip { rect: at, radius: tokens::RADIUS_SM });
        out.push(Item::Fill { rect: at, color: self.page, radius: 0.0 });
        let first = out.items.len();
        self.layout.paint(picture, out);
        out.translate(first, Point { x: -picture.x, y: -picture.y });
        out.scale(first, by);
        out.translate(first, Point { x: at.x, y: at.y });
        out.push(Item::PopClip);
        out.push(Item::Stroke { rect: at, color: color(tokens::LINE), width: 1.0, radius: tokens::RADIUS_SM });
        caption.paint(Point { x: inner.x, y: at.y + at.height + tokens::SPACE_SM }, color(tokens::FG), out);
        rect
    }
}
