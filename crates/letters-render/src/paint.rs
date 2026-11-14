//! Turns display list items into draw calls.
//!
//! Vello's CPU and GPU contexts have the same drawing methods but no common trait, so [`Canvas`] is that
//! trait and [`forward_canvas`] implements most of it for either.

use std::collections::HashMap;
use std::sync::Arc;

use skrifa::raw::TableProvider;

use vello_cpu::kurbo::{Affine, BezPath, Cap, Diagonal2, Join, Rect as KRect, RoundedRect, Shape, Stroke};
use vello_cpu::peniko::{Blob, Fill, FontData, ImageSampler};
use vello_cpu::{Glyph as RunGlyph, ImageSource, PaintType, color};

use crate::{Color, DisplayList, Font, Item, PathEl, Point, Rect};

/// How far a curve may stray from its true shape, in physical pixels.
const TOLERANCE: f64 = 0.05;
/// The outline growth per side of synthetic bold, as a share of the font size.
const EMBOLDEN: f64 = 1.0 / 48.0;

/// One run of glyphs, in the terms the glyph builders take.
pub(crate) struct Run<'a> {
    pub font: &'a FontData,
    pub coords: &'a [i16],
    pub size: f32,
    pub skew: Option<Affine>,
    pub embolden: Option<Diagonal2>,
}

pub(crate) trait Canvas {
    fn set_transform(&mut self, transform: Affine);
    fn set_paint(&mut self, paint: PaintType);
    fn set_paint_transform(&mut self, transform: Affine);
    fn set_fill_rule(&mut self, rule: Fill);
    fn fill_rect(&mut self, rect: &KRect);
    fn fill_path(&mut self, path: &BezPath);
    fn stroke_path(&mut self, path: &BezPath, stroke: Stroke);
    fn fill_blurred_rounded_rect(&mut self, rect: &KRect, radius: f32, std_dev: f32);
    fn push_clip_rect(&mut self, rect: &KRect);
    fn push_clip_path(&mut self, path: &BezPath);
    fn pop_clip(&mut self);
    fn fill_glyphs(&mut self, run: &Run<'_>, glyphs: impl Iterator<Item = RunGlyph> + Clone);
    /// The pixels of an asset and their size, or `None` while it is not decoded.
    fn image(&mut self, asset: &str) -> Option<(ImageSource, f64, f64)>;
}

/// Implements the drawing methods of [`Canvas`] for a struct holding a Vello context and its resources.
macro_rules! forward_canvas {
    ($ctx:ident, $resources:ident) => {
        fn set_transform(&mut self, transform: vello_cpu::kurbo::Affine) {
            self.$ctx.set_transform(transform);
        }
        fn set_paint(&mut self, paint: vello_cpu::PaintType) {
            self.$ctx.set_paint(paint);
        }
        fn set_paint_transform(&mut self, transform: vello_cpu::kurbo::Affine) {
            self.$ctx.set_paint_transform(transform);
        }
        fn set_fill_rule(&mut self, rule: vello_cpu::peniko::Fill) {
            self.$ctx.set_fill_rule(rule);
        }
        fn fill_rect(&mut self, rect: &vello_cpu::kurbo::Rect) {
            self.$ctx.fill_rect(rect);
        }
        fn fill_path(&mut self, path: &vello_cpu::kurbo::BezPath) {
            self.$ctx.fill_path(path);
        }
        fn stroke_path(&mut self, path: &vello_cpu::kurbo::BezPath, stroke: vello_cpu::kurbo::Stroke) {
            self.$ctx.set_stroke(stroke);
            self.$ctx.stroke_path(path);
        }
        fn fill_blurred_rounded_rect(&mut self, rect: &vello_cpu::kurbo::Rect, radius: f32, std_dev: f32) {
            self.$ctx.fill_blurred_rounded_rect(rect, radius, std_dev);
        }
        fn push_clip_rect(&mut self, rect: &vello_cpu::kurbo::Rect) {
            self.$ctx.push_clip_rect(rect);
        }
        fn push_clip_path(&mut self, path: &vello_cpu::kurbo::BezPath) {
            self.$ctx.push_clip_path(path);
        }
        fn pop_clip(&mut self) {
            self.$ctx.pop_clip();
        }
        fn fill_glyphs(&mut self, run: &crate::paint::Run<'_>, glyphs: impl Iterator<Item = vello_cpu::Glyph> + Clone) {
            let mut builder = self
                .$ctx
                .glyph_run(self.$resources, run.font)
                .font_size(run.size)
                .normalized_coords(bytemuck::cast_slice(run.coords));
            if let Some(skew) = run.skew {
                builder = builder.glyph_transform(skew);
            }
            if let Some(amount) = run.embolden {
                builder = builder.font_embolden(glifo::FontEmbolden::new(amount));
            }
            // A glyph the font cannot draw is skipped, and the rest of the run still shows.
            let _ = builder.fill_glyphs(glyphs);
        }
    };
}
pub(crate) use forward_canvas;

/// The Vello font handles for the fonts of a display list.
///
/// Vello keys its glyph caches on a handle's identity, so each font keeps one handle for as long as
/// anything else still holds its data.
#[derive(Default)]
pub(crate) struct FontCache {
    faces: HashMap<(usize, u32), Face>,
}

pub(crate) struct Face {
    /// Keeps the address the face is keyed by from being reused.
    data: Arc<[u8]>,
    /// `None` if the data is not a font: Vello panics on one.
    handle: Option<FontData>,
    /// Unique to this face for the life of the process, unlike its address.
    pub id: u64,
    /// The box that holds every glyph, in ems with y up: left, bottom, right, top.
    pub glyph_box: [f32; 4],
}

impl FontCache {
    pub fn face(&mut self, font: &Font) -> &Face {
        let key = (Arc::as_ptr(&font.data).cast::<u8>() as usize, font.index);
        self.faces.entry(key).or_insert_with(|| {
            let blob = Blob::new(Arc::new(font.data.clone()));
            let id = blob.id();
            let head = skrifa::FontRef::from_index(&font.data, font.index).map(|face| face.head());
            let glyph_box = match &head {
                Ok(Ok(head)) => {
                    let em = f32::from(head.units_per_em().max(1));
                    [head.x_min(), head.y_min(), head.x_max(), head.y_max()].map(|unit| f32::from(unit) / em)
                }
                // A face with no header still draws, so assume a generous box.
                _ => [-1.0, -1.0, 2.0, 2.0],
            };
            let handle = head.is_ok().then(|| FontData::new(blob, font.index));
            Face { data: font.data.clone(), handle, id, glyph_box }
        })
    }

    /// Forgets fonts nothing else uses any more. A handle holds a second reference.
    pub fn trim(&mut self) {
        self.faces.retain(|_, face| Arc::strong_count(&face.data) > 1 + usize::from(face.handle.is_some()));
    }
}

/// Which part of a display list to draw, and how large.
pub(crate) struct View<'a> {
    /// Physical pixels per logical pixel.
    pub scale: f64,
    /// Whether the item at an index can show at all. Clips are kept whatever this says.
    pub shows: &'a dyn Fn(usize) -> bool,
}

/// The display scale to draw at: `scale`, or 1 when it is not usable.
pub(crate) fn usable_scale(scale: f32) -> f32 {
    if scale.is_finite() && scale > 0.0 { scale } else { 1.0 }
}

/// Draws the part of `list` that `view` selects on `canvas`.
pub(crate) fn paint(canvas: &mut impl Canvas, fonts: &mut FontCache, list: &DisplayList, view: &View<'_>) {
    let tolerance = TOLERANCE / view.scale;
    let base = Affine::scale(view.scale);
    canvas.set_transform(base);
    canvas.set_paint_transform(Affine::IDENTITY);
    canvas.set_fill_rule(Fill::NonZero);

    let mut clips = 0_usize;
    for (index, item) in list.items.iter().enumerate() {
        if !matches!(item, Item::PushClip { .. } | Item::PopClip) && !(view.shows)(index) {
            continue;
        }
        match item {
            Item::Fill { rect, color, radius } => {
                let Some(shape) = rounded(rect, *radius) else { continue };
                canvas.set_paint(solid(*color));
                fill(canvas, &shape, tolerance);
            }
            Item::Stroke { rect, color, width, radius } => {
                let (Some(outer), true) = (rounded(rect, *radius), *width > 0.0) else { continue };
                canvas.set_paint(solid(*color));
                let inset =
                    Rect::new(rect.x + width, rect.y + width, rect.width - 2.0 * width, rect.height - 2.0 * width);
                match rounded(&inset, radius - width) {
                    // The band between the edge and the edge moved in by the width.
                    Some(inner) => {
                        let mut ring = outer.to_path(tolerance);
                        ring.extend(inner.path_elements(tolerance));
                        canvas.set_fill_rule(Fill::EvenOdd);
                        canvas.fill_path(&ring);
                        canvas.set_fill_rule(Fill::NonZero);
                    }
                    None => fill(canvas, &outer, tolerance),
                }
            }
            Item::Shadow { rect, color, blur, offset, radius } => {
                let moved = Rect::new(rect.x + offset.x, rect.y + offset.y, rect.width, rect.height);
                let (Some(shape), true) = (rounded(&moved, *radius), blur.is_finite()) else { continue };
                canvas.set_paint(solid(*color));
                let radius = shape.radii().top_left as f32;
                canvas.fill_blurred_rounded_rect(&shape.rect(), radius, blur.max(0.0) / 2.0);
            }
            Item::Glyphs { origin, font, size, color, skew, embolden, glyphs } => {
                if !(origin.x.is_finite() && origin.y.is_finite() && size.is_finite() && *size > 0.0) {
                    continue;
                }
                let Some(handle) = &fonts.face(font).handle else { continue };
                canvas.set_paint(solid(*color));
                let run = Run {
                    font: handle,
                    coords: &font.coords,
                    size: *size,
                    skew: (skew.is_finite() && *skew != 0.0)
                        .then(|| Affine::skew(f64::from(skew.to_radians().tan()), 0.0)),
                    embolden: embolden
                        .then(|| Diagonal2::new(f64::from(*size) * EMBOLDEN, f64::from(*size) * EMBOLDEN)),
                };
                let placed = glyphs
                    .iter()
                    .filter(|glyph| glyph.x.is_finite() && glyph.y.is_finite())
                    .map(|glyph| RunGlyph { id: glyph.id, x: origin.x + glyph.x, y: origin.y + glyph.y });
                canvas.fill_glyphs(&run, placed);
            }
            Item::Path { origin, scale, path, color, stroke } => {
                if !(origin.x.is_finite() && origin.y.is_finite() && scale.is_finite() && *scale > 0.0) {
                    continue;
                }
                let Some(shape) = bez_path(path) else { continue };
                canvas.set_paint(solid(*color));
                canvas.set_transform(
                    base * Affine::translate(point(*origin).to_vec2()) * Affine::scale(f64::from(*scale)),
                );
                match stroke {
                    None => {
                        canvas.set_fill_rule(Fill::EvenOdd);
                        canvas.fill_path(&shape);
                        canvas.set_fill_rule(Fill::NonZero);
                    }
                    Some(width) if width.is_finite() && *width > 0.0 => {
                        let style = Stroke::new(f64::from(*width)).with_caps(Cap::Round).with_join(Join::Round);
                        canvas.stroke_path(&shape, style);
                    }
                    Some(_) => {}
                }
                canvas.set_transform(base);
            }
            Item::Image { rect, asset } => {
                let Some(area) = rounded(rect, 0.0).map(|shape| shape.rect()) else { continue };
                let Some((source, width, height)) = canvas.image(asset) else { continue };
                canvas
                    .set_paint(PaintType::Image(vello_cpu::Image { image: source, sampler: ImageSampler::default() }));
                canvas.set_paint_transform(
                    Affine::translate((area.x0, area.y0))
                        * Affine::scale_non_uniform(area.width() / width, area.height() / height),
                );
                canvas.fill_rect(&area);
                canvas.set_paint_transform(Affine::IDENTITY);
            }
            Item::PushClip { rect, radius } => {
                clips += 1;
                match rounded(rect, *radius) {
                    Some(shape) if shape.radii().top_left > 0.0 => canvas.push_clip_path(&shape.to_path(tolerance)),
                    Some(shape) => canvas.push_clip_rect(&shape.rect()),
                    // An empty clip still clips, and keeps its pop matched.
                    None => canvas.push_clip_rect(&KRect::ZERO),
                }
            }
            // A pop with nothing pushed is ignored.
            Item::PopClip if clips > 0 => {
                clips -= 1;
                canvas.pop_clip();
            }
            Item::PopClip => {}
        }
    }
    for _ in 0..clips {
        canvas.pop_clip();
    }
}

fn fill(canvas: &mut impl Canvas, shape: &RoundedRect, tolerance: f64) {
    if shape.radii().top_left > 0.0 {
        canvas.fill_path(&shape.to_path(tolerance));
    } else {
        canvas.fill_rect(&shape.rect());
    }
}

/// A rectangle with its corner radius clamped to fit, or `None` if it has no area or is not finite.
fn rounded(rect: &Rect, radius: f32) -> Option<RoundedRect> {
    let finite = [rect.x, rect.y, rect.width, rect.height].iter().all(|value| value.is_finite());
    if !finite || rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }
    let (x, y) = (f64::from(rect.x), f64::from(rect.y));
    let radius = if radius.is_finite() { f64::from(radius.max(0.0)) } else { 0.0 };
    Some(RoundedRect::new(x, y, x + f64::from(rect.width), y + f64::from(rect.height), radius))
}

/// A path in Vello's terms, or `None` if it is empty or any of its points is not finite.
fn bez_path(path: &[PathEl]) -> Option<BezPath> {
    let mut out = BezPath::with_capacity(path.len());
    // A segment before the first move would have nowhere to start from.
    let mut started = false;
    for element in path {
        match *element {
            PathEl::MoveTo(to) => out.move_to(point(to)),
            _ if !started => return None,
            PathEl::LineTo(to) => out.line_to(point(to)),
            PathEl::QuadTo(control, to) => out.quad_to(point(control), point(to)),
            PathEl::CurveTo(first, second, to) => out.curve_to(point(first), point(second), point(to)),
            PathEl::Close => out.close_path(),
        }
        started = true;
    }
    (started && out.is_finite()).then_some(out)
}

fn point(point: Point) -> vello_cpu::kurbo::Point {
    vello_cpu::kurbo::Point::new(f64::from(point.x), f64::from(point.y))
}

fn solid(color: Color) -> PaintType {
    PaintType::Solid(color::AlphaColor::from_rgba8(color.r, color.g, color.b, color.a))
}
