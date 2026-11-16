//! Turns the laid-out tree into a display list.

use std::collections::HashMap;

use std::rc::Rc;

use letters_render::{Color, DisplayList, Item, Point, Rect, Size};

use crate::color::{fade, resolve};
use crate::element::Content;
use crate::input::InputState;
use crate::layout::inner;
use crate::style::{Edges, Ring, Sides};
use crate::text::{Shaped, TextStyle};
use crate::theme::accent;
use crate::tokens::{self, Color as Ink, Scheme};
use crate::tree::{Id, Tree};

/// The width of yewi's focus ring.
const RING: f32 = 2.0;
const CARET: f32 = 1.0;
const SCROLLBAR: f32 = 6.0;

/// A tooltip that is showing: yewi's bubble, beside the element it names.
pub(crate) struct Tip {
    pub text: String,
    pub shaped: Rc<Shaped>,
    pub rect: Rect,
}

impl Tip {
    pub const TEXT: TextStyle = TextStyle::control().size(tokens::TEXT_SM).leading(1.4);
    /// The widest a bubble's text runs before it wraps.
    pub const WIDTH: f32 = 320.0;
    /// The room between a bubble and its element.
    pub const GAP: f32 = 6.0;
    const PADDING: Edges = Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS);

    /// The bubble's size around its text, border included.
    pub fn size(shaped: &Shaped) -> Size {
        Size {
            width: shaped.size.width + Self::PADDING.horizontal() + 2.0,
            height: shaped.size.height + Self::PADDING.vertical() + 2.0,
        }
    }

    fn paint(&self, scheme: Scheme, out: &mut DisplayList) {
        let (rect, radius) = (self.rect, tokens::RADIUS_CONTROL);
        let color = |ink: Ink| resolve(ink, scheme, 1.0);
        for shadow in tokens::SHADOW_CARD {
            let offset = Point { x: shadow.x, y: shadow.y };
            out.push(Item::Shadow { rect, color: color(shadow.color), blur: shadow.blur, offset, radius });
        }
        out.push(Item::Fill { rect, color: color(tokens::SURFACE_RAISED), radius });
        out.push(Item::Stroke { rect, color: color(tokens::CARD_LINE), width: 1.0, radius });
        let at = Point { x: rect.x + Self::PADDING.left + 1.0, y: rect.y + Self::PADDING.top + 1.0 };
        self.shaped.paint(at, color(tokens::FG), out);
    }
}

/// What painting needs to know beyond the tree: the scheme and where the pointer and focus are.
pub(crate) struct Scene<'a> {
    pub scheme: Scheme,
    /// The node under the pointer and its ancestors.
    pub hover: &'a [Id],
    pub pressed: Option<Id>,
    pub focus: Option<Id>,
    /// Whether focus came from the keyboard, which is when a ring is drawn.
    pub focus_visible: bool,
    /// The element whose caret shows, while the blink is in its visible half.
    pub caret: Option<Id>,
    pub inputs: &'a HashMap<Id, InputState>,
    pub scroll: &'a HashMap<Id, f32>,
    pub tip: Option<&'a Tip>,
}

#[derive(Clone, Copy)]
struct Inherited {
    fg: Ink,
    opacity: f32,
    /// The part of the window that is not clipped away.
    visible: Rect,
}

pub(crate) fn paint<M>(tree: &Tree<M>, scene: &Scene<'_>, window: Rect, out: &mut DisplayList) {
    let inherited = Inherited { fg: tokens::FG, opacity: 1.0, visible: window };
    let mut rings = Vec::new();
    for layer in &tree.layers {
        if layer.modal {
            out.push(Item::Fill { rect: window, color: resolve(tokens::SCRIM, scene.scheme, 1.0), radius: 0.0 });
        }
        node(tree, scene, layer.root, inherited, &mut rings, out);
        out.items.append(&mut rings);
    }
    if let Some(tip) = scene.tip {
        tip.paint(scene.scheme, out);
    }
}

fn node<M>(
    tree: &Tree<M>,
    scene: &Scene<'_>,
    i: usize,
    inherited: Inherited,
    rings: &mut Vec<Item>,
    out: &mut DisplayList,
) {
    let n = &tree.nodes[i];
    let (el, style, rect) = (&n.el, &n.el.style, n.rect);
    let opacity = inherited.opacity * style.opacity;
    let color = |ink: Ink| resolve(ink, scene.scheme, opacity);

    let focused = scene.focus == Some(n.id);
    let shows_focus = focused && (scene.focus_visible || el.ring == Ring::Field);
    let mut look = style.look();
    if !tree.disabled(i) {
        if scene.hover.contains(&n.id) {
            look = el.hover.over(look);
        }
        if shows_focus {
            look = el.focused.over(look);
        }
        if scene.pressed == Some(n.id) && scene.hover.contains(&n.id) {
            look = el.active.over(look);
        }
    }
    let fg = look.fg.unwrap_or(inherited.fg);

    for shadow in style.shadow {
        out.push(Item::Shadow {
            rect,
            color: color(shadow.color),
            blur: shadow.blur,
            offset: Point { x: shadow.x, y: shadow.y },
            radius: style.radius,
        });
    }
    if let Some(bg) = look.bg.or(style.inked.then_some(fg)) {
        let color = color(bg);
        rounded(&mut out.items, rect, style.radius, style.square, |rect, radius| Item::Fill { rect, color, radius });
    }
    if let Some(border) = look.border {
        match style.border.uniform() {
            Some(width) if width > 0.0 => {
                outline(&mut out.items, rect, color(border), width, style.radius, style.square)
            }
            Some(_) => {}
            // One-sided rules, as under a bar or beside a panel.
            None => {
                let b = style.border;
                let sides = [
                    Rect::new(rect.x, rect.y, rect.width, b.top),
                    Rect::new(rect.x, rect.y + rect.height - b.bottom, rect.width, b.bottom),
                    Rect::new(rect.x, rect.y, b.left, rect.height),
                    Rect::new(rect.x + rect.width - b.right, rect.y, b.right, rect.height),
                ];
                for side in sides.into_iter().filter(|side| side.width > 0.0 && side.height > 0.0) {
                    out.push(Item::Fill { rect: side, color: color(border), radius: 0.0 });
                }
            }
        }
    }

    let ring_mark = rings.len();
    let mut visible = inherited.visible;
    // A clip with square corners is two: the element's box, and the round one run out past its square sides.
    let cut = style.clip && style.radius > 0.0 && style.square.any();
    if style.clip {
        out.push(Item::PushClip { rect, radius: if cut { 0.0 } else { style.radius } });
        visible = intersect(visible, rect);
    }
    if cut {
        out.push(Item::PushClip { rect: run_out(rect, style.radius, style.square), radius: style.radius });
    }

    let content = inner(rect, style);
    match &el.content {
        Content::None => {}
        Content::Text { .. } => {
            if let Some(shaped) = &n.shaped {
                shaped.paint(Point { x: rect.x, y: rect.y + (rect.height - shaped.size.height) / 2.0 }, color(fg), out);
            }
        }
        Content::Icon { icon, size } => out.push(Item::Path {
            origin: Point { x: rect.x + (rect.width - size) / 2.0, y: rect.y + (rect.height - size) / 2.0 },
            scale: size / crate::icon::GRID,
            path: icon.outline(),
            color: color(fg),
            stroke: (!icon.is_filled()).then_some(tokens::ICON_STROKE),
        }),
        Content::Input(input) => {
            if let Some(shaped) = &n.shaped {
                let state = scene.inputs.get(&n.id).cloned().unwrap_or_default();
                let top = content.y + (content.height - shaped.size.height) / 2.0;
                let left = content.x - state.scroll;
                let span = |range: std::ops::Range<usize>, y: f32, height: f32| {
                    let x = left + shaped.x_at(range.start);
                    Rect::new(x, y, left + shaped.x_at(range.end) - x, height)
                };
                out.push(Item::PushClip { rect: content, radius: 0.0 });

                let composing = state.composing_range(&input.value);
                let (start, end) = state.ordered(&input.value);
                if focused && composing.is_none() && start != end {
                    let rect = span(start..end, top, shaped.size.height);
                    out.push(Item::Fill { rect, color: color(fade(accent().base, 0.3)), radius: 0.0 });
                }
                match &n.hint {
                    Some(hint) => hint.paint(Point { x: content.x, y: top }, color(tokens::FG_SUBTLE), out),
                    None => shaped.paint(Point { x: left, y: top }, color(fg), out),
                }
                if let Some(range) = composing {
                    // Text still being composed is underlined, as native fields do.
                    let rect = span(range, top + shaped.baseline + 2.0, 1.0);
                    out.push(Item::Fill { rect, color: color(fg), radius: 0.0 });
                }
                if scene.caret == Some(n.id) {
                    let x = left + shaped.x_at(state.caret(&input.value));
                    let rect = Rect::new(x, top, CARET, shaped.size.height);
                    out.push(Item::Fill { rect, color: color(fg), radius: 0.0 });
                }
                out.push(Item::PopClip);
            }
        }
        Content::Canvas(canvas) => {
            canvas.paint.paint(content, intersect(visible, content), scene.scheme, out);
            if let Some(caret) = canvas.caret.filter(|_| scene.caret == Some(n.id)) {
                let rect =
                    Rect { x: content.x + caret.x, y: content.y + caret.y, width: caret.width.max(CARET), ..caret };
                out.push(Item::Fill { rect, color: color(fg), radius: 0.0 });
            }
        }
    }

    let below = Inherited { fg, opacity, visible };
    for child in tree.children(i) {
        // Scrolled-away children are skipped whole.
        if style.scroll && !tree.nodes[child].rect.intersects(&visible) {
            continue;
        }
        node(tree, scene, child, below, rings, out);
    }

    if style.scroll {
        let view = Rect { y: content.y, height: content.height, ..rect };
        let offset = scene.scroll.get(&n.id).copied().unwrap_or(0.0);
        out.items.extend(scrollbar(view, n.content_height, offset, scene.scheme, opacity));
    }

    if shows_focus {
        let offset = match el.ring {
            Ring::None => None,
            Ring::Outline(offset) => Some(offset + RING),
            // A field's ring covers its border: one pixel outside the edge, two pixels wide.
            Ring::Field => Some(1.0),
        };
        if let Some(grow) = offset {
            let ring = Rect::new(rect.x - grow, rect.y - grow, rect.width + 2.0 * grow, rect.height + 2.0 * grow);
            let radius = if style.radius > 0.0 { (style.radius + grow).max(0.0) } else { 0.0 };
            let ink = el.focused.border.unwrap_or(accent().focus());
            outline(rings, ring, color(ink), RING, radius, style.square);
        }
    }
    if style.clip {
        // Rings inside a clip are drawn before it ends, over the content they belong to.
        out.items.extend(rings.drain(ring_mark..));
        out.push(Item::PopClip);
    }
    if cut {
        out.push(Item::PopClip);
    }
}

/// The thumb of a scrollbar down the right edge of `view`, when the content is taller than the view.
pub(crate) fn scrollbar(view: Rect, content: f32, offset: f32, scheme: Scheme, opacity: f32) -> Option<Item> {
    if content <= view.height {
        return None;
    }
    let length = (view.height * view.height / content).max(24.0).min(view.height);
    let travel = (view.height - length) * offset / (content - view.height);
    let rect = Rect::new(view.x + view.width - SCROLLBAR - 2.0, view.y + travel, SCROLLBAR, length);
    Some(Item::Fill { rect, color: resolve(fade(tokens::NEUTRAL, 0.45), scheme, opacity), radius: SCROLLBAR / 2.0 })
}

/// Draws a shape of `rect` whose corners are round by `radius`, but for those on the `square` sides. The shape
/// runs out past those sides as far as its corners are deep and is cut back to `rect`, so that only the
/// corners that stay round are seen.
fn rounded(out: &mut Vec<Item>, rect: Rect, radius: f32, square: Sides, shape: impl Fn(Rect, f32) -> Item) {
    if radius <= 0.0 || !square.any() {
        return out.push(shape(rect, radius));
    }
    out.push(Item::PushClip { rect, radius: 0.0 });
    out.push(shape(run_out(rect, radius, square), radius));
    out.push(Item::PopClip);
}

/// `rect` run out by this much past these sides.
fn run_out(rect: Rect, by: f32, sides: Sides) -> Rect {
    let [top, right, bottom, left] = [sides.top, sides.right, sides.bottom, sides.left].map(|out| f32::from(out) * by);
    Rect::new(rect.x - left, rect.y - top, rect.width + left + right, rect.height + top + bottom)
}

/// Draws the line inside the edge of `rect`, its corners round by `radius` but for those on the `square`
/// sides.
fn outline(out: &mut Vec<Item>, rect: Rect, color: Color, width: f32, radius: f32, square: Sides) {
    rounded(out, rect, radius, square, |rect, radius| Item::Stroke { rect, color, width, radius });
    if radius <= 0.0 || !square.any() {
        return;
    }
    // The line of a square side ran out of sight with the corners: it is drawn by itself, between the lines it
    // meets, so that no part of the line is drawn twice.
    let (right, bottom) = (rect.x + rect.width - width, rect.y + rect.height - width);
    let (from, to) = (f32::from(!square.left) * width, f32::from(!square.right) * width);
    let across = |y: f32| Rect::new(rect.x + from, y, rect.width - from - to, width);
    let down = |x: f32| Rect::new(x, rect.y + width, width, rect.height - 2.0 * width);
    let sides = [
        square.top.then(|| across(rect.y)),
        square.bottom.then(|| across(bottom)),
        square.left.then(|| down(rect.x)),
        square.right.then(|| down(right)),
    ];
    out.extend(sides.into_iter().flatten().map(|rect| Item::Fill { rect, color, radius: 0.0 }));
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let (x, y) = (a.x.max(b.x), a.y.max(b.y));
    let (right, bottom) = ((a.x + a.width).min(b.x + b.width), (a.y + a.height).min(b.y + b.height));
    Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
}
