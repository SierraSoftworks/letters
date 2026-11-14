//! Sizes and positions every node: flexbox rows and columns, then popovers against their anchors.
//!
//! Two passes. `measure` works out each node's own size from the bottom up, given the width it may use.
//! `place` hands out final rectangles from the top down, which is where spare room, stretching and scrolling
//! are settled.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use letters_render::{Point, Rect, Size};

use crate::element::Content;
use crate::style::{Align, Dir, Justify, Style};
use crate::text::{Shaped, TextShaper, TextStyle};
use crate::tree::{Id, Tree};
use crate::widgets::{Placement, Side};

/// The room kept between a popover and the window's edge.
const MARGIN: f32 = 8.0;
/// A text field's width when nothing else decides it.
const INPUT_WIDTH: f32 = 180.0;

/// Shaped labels, kept from one layout to the next. A label that is not asked for in a layout is dropped.
#[derive(Default)]
pub(crate) struct TextCache {
    current: HashMap<u64, Rc<Shaped>>,
    previous: HashMap<u64, Rc<Shaped>>,
}

impl TextCache {
    pub fn shape(
        &mut self,
        shaper: &dyn TextShaper,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
    ) -> Rc<Shaped> {
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        (style.family, style.size.to_bits(), style.weight, style.italic, style.leading.to_bits()).hash(&mut hasher);
        max_width.map(f32::to_bits).hash(&mut hasher);
        let key = hasher.finish();
        if let Some(shaped) = self.current.get(&key) {
            return shaped.clone();
        }
        let shaped = self.previous.remove(&key).unwrap_or_else(|| Rc::new(shaper.shape(text, style, max_width)));
        self.current.insert(key, shaped.clone());
        shaped
    }

    fn turn(&mut self) {
        self.previous = std::mem::take(&mut self.current);
    }
}

pub(crate) struct Layout<'a> {
    pub shaper: &'a dyn TextShaper,
    pub cache: &'a mut TextCache,
    pub window: Size,
    pub scale: f32,
    /// How far each scrolling element is scrolled. Offsets are clamped here as content shrinks.
    pub scroll: &'a mut HashMap<Id, f32>,
    /// What each text input shows: its value with any text being composed.
    pub inputs: &'a dyn Fn(Id, &str) -> String,
}

impl Layout<'_> {
    pub fn run<M>(&mut self, tree: &mut Tree<M>) {
        self.cache.turn();
        if tree.nodes.is_empty() {
            return;
        }
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        self.measure(tree, 0, window.width);
        self.place(tree, 0, window);

        for layer in 1..tree.layers.len() {
            let (root, anchor) = (tree.layers[layer].root, tree.layers[layer].anchor);
            let size = self.measure(tree, root, (window.width - 2.0 * MARGIN).max(0.0));
            let rect = if tree.layers[layer].modal {
                Rect::new(
                    (window.width - size.width) / 2.0,
                    (window.height - size.height) / 2.0,
                    size.width,
                    size.height,
                )
            } else {
                let anchor = anchor.map_or(window, |anchor| tree.nodes[anchor].rect);
                position(tree.layers[layer].placement, anchor, size, self.window)
            };
            self.place(tree, root, rect);
        }
    }

    fn measure<M>(&mut self, tree: &mut Tree<M>, i: usize, available: f32) -> Size {
        let style = tree.nodes[i].el.style;
        let limit = style.width.unwrap_or(available).min(style.max_width);
        let inner = (limit - style.frame_x()).max(0.0);

        let id = tree.nodes[i].id;
        let mut hint = None;
        let (content, shaped) = match &tree.nodes[i].el.content {
            Content::Text { text, style: text_style, wrap } => {
                let width = wrap.then_some(inner).filter(|width| width.is_finite());
                let shaped = self.cache.shape(self.shaper, text, text_style, width);
                (shaped.size, Some(shaped))
            }
            Content::Icon { size, .. } => (Size { width: *size, height: *size }, None),
            Content::Input(input) => {
                let shown = (self.inputs)(id, &input.value);
                let shaped = self.cache.shape(self.shaper, &shown, &input.style, None);
                if shown.is_empty() && !input.placeholder.is_empty() {
                    hint = Some(self.cache.shape(self.shaper, &input.placeholder, &input.style, None));
                }
                (Size { width: INPUT_WIDTH - style.frame_x(), height: shaped.size.height }, Some(shaped))
            }
            Content::Canvas(_) => (Size::default(), None),
            Content::None => (self.measure_children(tree, i, &style, inner), None),
        };
        tree.nodes[i].shaped = shaped;
        tree.nodes[i].hint = hint;
        tree.nodes[i].content_height = content.height;

        let mut height = content.height + style.frame_y();
        if style.scroll {
            // A scrolling element never asks for its content's height: it takes what it is given.
            height = if style.max_height.is_finite() { height.min(style.max_height) } else { 0.0 };
        }
        let size = Size {
            width: style.width.unwrap_or(content.width + style.frame_x()).min(style.max_width).max(style.min_width),
            height: style.height.unwrap_or(height).min(style.max_height).max(style.min_height),
        };
        tree.nodes[i].size = size;
        size
    }

    fn measure_children<M>(&mut self, tree: &mut Tree<M>, i: usize, style: &Style, inner: f32) -> Size {
        let children: Vec<usize> = tree.children(i).collect();
        let gaps = style.gap * children.len().saturating_sub(1) as f32;
        match style.dir {
            Dir::Column => {
                let mut content = Size { width: 0.0, height: gaps };
                for child in children {
                    let size = self.measure(tree, child, inner);
                    content.width = content.width.max(size.width);
                    content.height += size.height;
                }
                content
            }
            Dir::Row => {
                // Fixed children first, so the ones that grow know what is left to share.
                let mut content = Size { width: gaps, height: 0.0 };
                let mut growth = 0.0;
                for &child in &children {
                    let grow = tree.nodes[child].el.style.grow;
                    if grow > 0.0 {
                        growth += grow;
                        continue;
                    }
                    let size = self.measure(tree, child, (inner - content.width).max(0.0));
                    content.width += size.width;
                    content.height = content.height.max(size.height);
                }
                let spare = (inner - content.width).max(0.0);
                for &child in &children {
                    let grow = tree.nodes[child].el.style.grow;
                    if grow > 0.0 {
                        let share = if spare.is_finite() { spare * grow / growth } else { spare };
                        let size = self.measure(tree, child, share);
                        content.width += size.width;
                        content.height = content.height.max(size.height);
                    }
                }
                content
            }
        }
    }

    fn place<M>(&mut self, tree: &mut Tree<M>, i: usize, rect: Rect) {
        let rect = snap(rect, self.scale);
        tree.nodes[i].rect = rect;
        let style = tree.nodes[i].el.style;
        let children: Vec<usize> = tree.children(i).collect();
        if children.is_empty() {
            return;
        }
        let inner = inner(rect, &style);

        let column = style.dir == Dir::Column;
        let (main, cross) = if column { (inner.height, inner.width) } else { (inner.width, inner.height) };
        let along = |size: Size| if column { size.height } else { size.width };
        let across = |size: Size| if column { size.width } else { size.height };

        let used: f32 = children.iter().map(|&child| along(tree.nodes[child].size)).sum();
        let gaps = style.gap * (children.len() - 1) as f32;
        let growth: f32 = children.iter().map(|&child| tree.nodes[child].el.style.grow).sum();
        let spare = main - used - gaps;

        let mut scrolled = 0.0;
        if style.scroll {
            let offset = self.scroll.entry(tree.nodes[i].id).or_insert(0.0);
            *offset = offset.clamp(0.0, (tree.nodes[i].content_height - inner.height).max(0.0));
            scrolled = *offset;
        }

        // Too little room: the children that may shrink give it up in proportion to their size.
        let shrinkable: f32 =
            children.iter().filter(|&&c| tree.nodes[c].el.style.shrink).map(|&c| along(tree.nodes[c].size)).sum();
        let squeeze =
            if spare < 0.0 && shrinkable > 0.0 && !style.scroll { (-spare / shrinkable).min(1.0) } else { 0.0 };

        let (mut cursor, mut gap) = (0.0, style.gap);
        if spare > 0.0 && growth == 0.0 {
            match style.justify {
                Justify::Start => {}
                Justify::Center => cursor = spare / 2.0,
                Justify::End => cursor = spare,
                Justify::Between if children.len() > 1 => gap += spare / (children.len() - 1) as f32,
                Justify::Between => {}
            }
        }

        for child in children {
            let child_style = tree.nodes[child].el.style;
            let size = tree.nodes[child].size;
            let mut length = along(size);
            if spare > 0.0 && child_style.grow > 0.0 {
                length += spare * child_style.grow / growth;
            } else if child_style.shrink {
                length -= length * squeeze;
            }

            let align = child_style.align_self.unwrap_or(style.align);
            let fixed = if column { child_style.width } else { child_style.height };
            let limit = if column { child_style.max_width } else { child_style.max_height };
            let breadth = if align == Align::Stretch && fixed.is_none() { cross.min(limit) } else { across(size) };
            let offset = match align {
                Align::Start | Align::Stretch => 0.0,
                Align::Center => (cross - breadth) / 2.0,
                Align::End => cross - breadth,
            };

            let child_rect = if column {
                Rect::new(inner.x + offset, inner.y + cursor - scrolled, breadth, length)
            } else {
                Rect::new(inner.x + cursor, inner.y + offset, length, breadth)
            };
            self.place(tree, child, child_rect);
            cursor += length + gap;
        }
    }
}

/// The box inside an element's border and padding.
pub(crate) fn inner(rect: Rect, style: &Style) -> Rect {
    let (left, top) = (style.padding.left + style.border.left, style.padding.top + style.border.top);
    Rect::new(
        rect.x + left,
        rect.y + top,
        (rect.width - style.frame_x()).max(0.0),
        (rect.height - style.frame_y()).max(0.0),
    )
}

/// Rounds a rectangle's edges to device pixels, so hairlines stay sharp.
fn snap(rect: Rect, scale: f32) -> Rect {
    let round = |value: f32| (value * scale).round() / scale;
    let (x, y) = (round(rect.x), round(rect.y));
    Rect::new(x, y, round(rect.x + rect.width) - x, round(rect.y + rect.height) - y)
}

/// Puts a popover beside its anchor, on the other side when it does not fit, and always inside the window.
fn position(placement: Placement, anchor: Rect, size: Size, window: Size) -> Rect {
    let anchor = match placement.at {
        Some(at) => Rect::new(anchor.x + at.x, anchor.y + at.y, at.width, at.height),
        None => anchor,
    };
    let fits = |side: Side| match side {
        Side::Bottom => anchor.y + anchor.height + placement.gap + size.height + MARGIN <= window.height,
        Side::Top => anchor.y - placement.gap - size.height >= MARGIN,
        Side::Right => anchor.x + anchor.width + placement.gap + size.width + MARGIN <= window.width,
        Side::Left => anchor.x - placement.gap - size.width >= MARGIN,
    };
    let opposite = match placement.side {
        Side::Bottom => Side::Top,
        Side::Top => Side::Bottom,
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    };
    let side = if fits(placement.side) || !fits(opposite) { placement.side } else { opposite };

    let slide = |start: f32, length: f32, own: f32| match placement.align {
        Align::Start | Align::Stretch => start,
        Align::Center => start + (length - own) / 2.0,
        Align::End => start + length - own,
    };
    let at = match side {
        Side::Bottom => {
            Point { x: slide(anchor.x, anchor.width, size.width), y: anchor.y + anchor.height + placement.gap }
        }
        Side::Top => Point { x: slide(anchor.x, anchor.width, size.width), y: anchor.y - placement.gap - size.height },
        Side::Right => {
            Point { x: anchor.x + anchor.width + placement.gap, y: slide(anchor.y, anchor.height, size.height) }
        }
        Side::Left => {
            Point { x: anchor.x - placement.gap - size.width, y: slide(anchor.y, anchor.height, size.height) }
        }
    };
    let keep = |value: f32, own: f32, room: f32| value.min(room - MARGIN - own).max(MARGIN);
    Rect::new(keep(at.x, size.width, window.width), keep(at.y, size.height, window.height), size.width, size.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Size = Size { width: 800.0, height: 600.0 };
    const SIZE: Size = Size { width: 200.0, height: 100.0 };

    fn place(side: Side, align: Align, anchor: Rect) -> Rect {
        position(Placement { side, align, gap: 6.0, at: None }, anchor, SIZE, WINDOW)
    }

    #[test]
    fn a_popover_sits_beside_its_anchor() {
        let anchor = Rect::new(100.0, 40.0, 80.0, 32.0);
        assert_eq!(place(Side::Bottom, Align::Start, anchor), Rect::new(100.0, 78.0, 200.0, 100.0));
        assert_eq!(
            place(Side::Bottom, Align::End, anchor),
            Rect::new(8.0, 78.0, 200.0, 100.0),
            "kept inside the window"
        );
        assert_eq!(place(Side::Right, Align::Start, anchor), Rect::new(186.0, 40.0, 200.0, 100.0));
    }

    #[test]
    fn a_popover_flips_when_its_side_has_no_room() {
        let low = Rect::new(100.0, 540.0, 80.0, 32.0);
        assert_eq!(place(Side::Bottom, Align::Start, low).y, 540.0 - 6.0 - 100.0);
        let right = Rect::new(700.0, 40.0, 80.0, 32.0);
        assert_eq!(place(Side::Right, Align::Start, right).x, 700.0 - 6.0 - 200.0);
    }

    #[test]
    fn a_popover_can_hang_from_a_spot_inside_its_anchor() {
        let page = Rect::new(50.0, 50.0, 600.0, 500.0);
        let caret = Rect::new(120.0, 200.0, 2.0, 20.0);
        let placement = Placement { side: Side::Bottom, align: Align::Start, gap: 6.0, at: Some(caret) };
        assert_eq!(position(placement, page, SIZE, WINDOW), Rect::new(170.0, 276.0, 200.0, 100.0));
    }
}
