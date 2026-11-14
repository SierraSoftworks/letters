//! The Markdown markers shown around the caret.
//!
//! A layout never learns of them. Markers are shaped apart from the text and drawn over it: one inside a line
//! pushes the rest of that line aside, so line breaks stay where the text alone puts them.

use std::collections::BTreeSet;

use letters_model::{BlockId, Markers, Pos};
use parley::{Affinity, Layout};

use crate::leaf::Set;

use crate::engine::Engine;
use crate::flow::{Role, Types};
use crate::ink::{Brush, Ink};
use crate::{leaf, theme};

/// Where a marker goes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    /// Beside its block: left of a heading's first line, or in the corner of a callout.
    Block,
    /// Inside the text of its block.
    Inline {
        offset: usize,
        /// Drawn before a caret at the same offset.
        before: bool,
        /// Where its offset is a wrap it ends the upper line. Otherwise it starts the lower one.
        upstream: bool,
    },
}

struct Sign {
    block: BlockId,
    text: String,
    place: Place,
    /// How much of it showed when the markers last changed and how much will, from 0 for none to 1 for all.
    from: f32,
    to: f32,
}

impl Sign {
    /// Whether `other` is this marker again. Its side of the caret can change as the caret steps over it.
    fn same(&self, other: &Sign) -> bool {
        let placed = match (self.place, other.place) {
            (Place::Block, Place::Block) => true,
            (Place::Inline { offset, .. }, Place::Inline { offset: other, .. }) => offset == other,
            _ => false,
        };
        placed && self.block == other.block && self.text == other.text
    }
}

/// The markers to draw, with those still fading away.
pub(crate) struct Reveal {
    /// The markers to show in drawing order, then those on their way out.
    signs: Vec<Sign>,
    /// How far every marker is from where it was to where it is going.
    progress: f32,
    /// Whether a block that shows markers was edited since they were set, which moves their offsets.
    edited: bool,
}

impl Default for Reveal {
    fn default() -> Self {
        Self { signs: Vec::new(), progress: 1.0, edited: false }
    }
}

impl Reveal {
    /// Sets the markers to show around a caret. Returns whether any appear or go.
    pub fn set(&mut self, at: Option<(&Pos, &Markers)>) -> bool {
        let mut next = Vec::new();
        if let Some((caret, markers)) = at {
            let sign = |block: &BlockId, text: &str, place| Sign {
                block: block.clone(),
                text: text.to_owned(),
                place,
                from: 0.0,
                to: 0.0,
            };
            next.extend(markers.block.iter().map(|(block, text)| sign(block, text, Place::Block)));
            next.extend(markers.inline.iter().enumerate().map(|(index, marker)| {
                let before = index < markers.before_caret;
                let upstream = !before && marker.offset > caret.offset;
                sign(&caret.block, marker.text, Place::Inline { offset: marker.offset, before, upstream })
            }));
        }
        let mut old: Vec<Option<Sign>> = std::mem::take(&mut self.signs).into_iter().map(Some).collect();
        // After an edit the same markers come back at new offsets, so they are known by their order.
        let shown = |sign: &Option<Sign>| sign.as_ref().is_some_and(|sign| sign.to > 0.0);
        let again = |(old, new): (&Option<Sign>, &Sign)| {
            let old = old.as_ref().filter(|old| old.to > 0.0);
            old.is_some_and(|old| (&old.block, &old.text) == (&new.block, &new.text))
        };
        let in_order = std::mem::take(&mut self.edited)
            && old.iter().filter(|sign| shown(sign)).count() == next.len()
            && old.iter().zip(&next).all(again);

        let mut changed = false;
        for (index, sign) in next.iter_mut().enumerate() {
            let found = match in_order {
                true => old.get_mut(index),
                false => old.iter_mut().find(|old| old.as_ref().is_some_and(|old| old.same(sign))),
            };
            match found.and_then(Option::take) {
                Some(old) => {
                    // One that was fading away comes back.
                    changed |= old.to == 0.0;
                    (sign.from, sign.to) = (old.from, old.to);
                }
                None => changed = true,
            }
        }
        let mut gone: Vec<Sign> = old.into_iter().flatten().collect();
        changed |= gone.iter().any(|sign| sign.to > 0.0);
        if changed {
            // Each starts from what it shows now, and all are shown at once unless the caller animates.
            let progress = std::mem::replace(&mut self.progress, 1.0);
            let level = |sign: &Sign| sign.from + (sign.to - sign.from) * progress;
            next.iter_mut().for_each(|sign| (sign.from, sign.to) = (level(sign), 1.0));
            gone.iter_mut().for_each(|sign| (sign.from, sign.to) = (level(sign), 0.0));
            gone.retain(|sign| sign.from > 0.0);
        }
        next.extend(gone);
        self.signs = next;
        changed
    }

    pub fn set_progress(&mut self, progress: f32) {
        self.progress = if progress.is_nan() { 1.0 } else { progress.clamp(0.0, 1.0) };
        if self.progress >= 1.0 {
            self.signs.retain(|sign| sign.to > 0.0);
        }
    }

    /// Notes an edit. Markers on their way out of an edited block go at once, since their offsets are stale.
    pub fn edit(&mut self, blocks: &BTreeSet<BlockId>) {
        if self.signs.iter().any(|sign| blocks.contains(&sign.block)) {
            self.edited = true;
            self.signs.retain(|sign| sign.to > 0.0 || !blocks.contains(&sign.block));
        }
    }

    fn level(&self, sign: &Sign) -> f32 {
        sign.from + (sign.to - sign.from) * self.progress
    }

    /// The markers that show now for one block, each with how much of it shows.
    fn of<'a>(&'a self, block: &'a BlockId) -> impl Iterator<Item = (&'a Sign, f32)> {
        let signs = self.signs.iter().filter(move |sign| &sign.block == block);
        signs.map(|sign| (sign, self.level(sign))).filter(|(_, level)| *level > 0.0)
    }

    /// The markers beside a block that has no text of its own, as text and how much of each shows.
    pub fn beside<'a>(&'a self, block: &'a BlockId) -> impl Iterator<Item = (&'a str, f32)> {
        self.of(block).filter(|(sign, _)| sign.place == Place::Block).map(|(sign, level)| (sign.text.as_str(), level))
    }

    /// Where the markers of a text block sit over its layout. `shape` gives a marker's shape and full width,
    /// and `room` is how much of the margin left of the text is free to hang a marker in.
    pub fn over(&self, block: &BlockId, set: &Set, room: f32, mut shape: impl FnMut(&str) -> (usize, f32)) -> Shifts {
        let mut shifts = Shifts::default();
        for (sign, level) in self.of(block) {
            let (shape, width) = shape(&sign.text);
            match sign.place {
                Place::Block => {
                    // What does not fit in the margin pushes the first line, as a marker inside it would.
                    let short = (width + theme::MARKER_GAP - room).max(0.0);
                    shifts.indent = shifts.indent.max(short * level);
                    shifts.hung.push((shape, level));
                }
                Place::Inline { offset, before, upstream } => {
                    let affinity = if upstream { Affinity::Upstream } else { Affinity::Downstream };
                    let (line, x) = leaf::spot(set, offset, affinity);
                    shifts.inline.push(Shown { line, x, width: width * level, level, shape, offset, before });
                }
            }
        }
        // Stable, so markers at one place keep their drawing order.
        shifts.inline.sort_by(|a, b| (a.line, a.x).partial_cmp(&(b.line, b.x)).unwrap_or(std::cmp::Ordering::Equal));
        shifts
    }
}

/// A marker inside a line of a leaf.
pub(crate) struct Shown {
    pub line: usize,
    /// Where it starts when nothing on its line is pushed, from the leaf's left edge.
    pub x: f32,
    /// The room it takes, which grows as it fades in.
    pub width: f32,
    /// How much of it shows, from 0 to 1.
    pub level: f32,
    pub shape: usize,
    offset: usize,
    before: bool,
}

/// The markers of one leaf: those inside its lines in drawing order, and those hung beside it.
#[derive(Default)]
pub(crate) struct Shifts {
    inline: Vec<Shown>,
    /// The shape of each marker beside the leaf and how much of it shows.
    pub hung: Vec<(usize, f32)>,
    /// How far the first line is pushed to make room for the markers hung beside it.
    pub indent: f32,
}

/// Positions closer than this are one position.
const NEAR: f32 = 0.05;

impl Shifts {
    /// Whether nothing is pushed.
    pub fn is_empty(&self) -> bool {
        self.inline.is_empty() && self.indent == 0.0
    }

    /// The markers on a line, left to right.
    pub fn on(&self, line: usize) -> impl Iterator<Item = &Shown> {
        self.inline.iter().filter(move |shown| shown.line == line)
    }

    /// Whether anything on a line is pushed.
    pub fn touches(&self, line: usize) -> bool {
        self.lead(line) > 0.0 || self.on(line).next().is_some()
    }

    /// How far a whole line is pushed.
    fn lead(&self, line: usize) -> f32 {
        if line == 0 { self.indent } else { 0.0 }
    }

    /// How far whatever starts at `x` on a line is pushed. A marker at `x` itself goes before it.
    pub fn push(&self, line: usize, x: f32) -> f32 {
        let before = self.on(line).filter(|shown| shown.x <= x + NEAR);
        self.lead(line) + before.map(|shown| shown.width).sum::<f32>()
    }

    /// How far a stretch of a line from `from` to `to` is pushed, and how much wider the markers inside it
    /// make it. A marker at its end comes after it.
    pub fn spread(&self, line: usize, from: f32, to: f32) -> (f32, f32) {
        let lead = self.push(line, from);
        let inside = self.on(line).filter(|shown| shown.x > from + NEAR && shown.x < to - NEAR);
        (lead, inside.map(|shown| shown.width).sum())
    }

    /// How far a caret at a byte offset on a line is pushed.
    pub fn caret(&self, line: usize, offset: usize) -> f32 {
        let ahead = |shown: &&Shown| shown.offset < offset || (shown.offset == offset && shown.before);
        self.lead(line) + self.on(line).filter(ahead).map(|shown| shown.width).sum::<f32>()
    }

    /// Where a point on a pushed line falls in the line as laid out. A point on a marker falls where the
    /// marker starts.
    pub fn pull(&self, line: usize, x: f32) -> f32 {
        let mut pushed = self.lead(line);
        for shown in self.on(line) {
            if x < shown.x + pushed + shown.width {
                return (x - pushed).min(shown.x);
            }
            pushed += shown.width;
        }
        x - pushed
    }
}

/// The shaped text of markers, kept so that nothing is shaped while they fade.
#[derive(Default)]
pub(crate) struct Shapes(Vec<Shape>);

struct Shape {
    role: Role,
    small: bool,
    text: String,
    layout: Layout<Brush>,
}

impl Shapes {
    /// The shape of a marker set in a role's type, and its width. `small` is for a tag that sits in a corner.
    pub fn get(&mut self, engine: &mut Engine, types: &Types, role: Role, small: bool, text: &str) -> (usize, f32) {
        let known = |shape: &Shape| (shape.role, shape.small, shape.text.as_str()) == (role, small, text);
        let at = self.0.iter().position(known).unwrap_or_else(|| {
            let mut spec = types.get(role).spec(role == Role::Code, Brush { ink: Ink::Marker, back: None });
            spec.italic = false;
            if small {
                spec.size = (spec.size * theme::TAG_SCALE).round();
            }
            let mut layout = Layout::new();
            engine.build(text, &[spec], &[(0, 0..text.len())], None, &mut layout);
            layout.break_all_lines(None);
            self.0.push(Shape { role, small, text: text.to_owned(), layout });
            self.0.len() - 1
        });
        (at, self.0[at].layout.full_width())
    }

    pub fn layout(&self, shape: usize) -> Option<&Layout<Brush>> {
        self.0.get(shape).map(|shape| &shape.layout)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}
