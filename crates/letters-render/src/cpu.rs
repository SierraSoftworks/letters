//! The CPU backend: draws a display list into memory.
//!
//! It paints the first frame and every frame on a host with no GPU, and it is what pixel tests run on. A
//! frame that is kept between renders is repainted only where its display list changed.

use std::sync::atomic::{AtomicU64, Ordering};

use vello_cpu::color::AlphaColor;
use vello_cpu::{ImageSource, PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};

use crate::damage::{self, Survey};
pub use crate::damage::{Damage, PixelRect};
use crate::image::Images;
use crate::paint::{self, Canvas, FontCache, View, forward_canvas};
use crate::{Color, DisplayList};

/// How a [`Frame`] stores a pixel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PixelFormat {
    /// Four bytes: red, green, blue and alpha, with the colours multiplied by alpha.
    #[default]
    Rgba8,
    /// One `0x00RRGGBB` word, the layout a window's software surface takes. It has no alpha, so draw over
    /// an opaque background.
    Xrgb,
}

/// The pixels of one frame, row by row from the top left.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    width: u16,
    height: u16,
    format: PixelFormat,
    /// One word per pixel, so that either format can be read without copying.
    pixels: Vec<u32>,
    /// Names the render that last filled the frame, or 0 if none has.
    drawn: u64,
}

impl PartialEq for Frame {
    fn eq(&self, other: &Self) -> bool {
        (self.width, self.height, self.format) == (other.width, other.height, other.format)
            && self.pixels == other.pixels
    }
}

impl Frame {
    /// A transparent RGBA8 frame of physical pixels. Each side is capped at 65535.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_format(width, height, PixelFormat::Rgba8)
    }

    /// A black frame in a window's own pixel layout, so presenting it needs no conversion.
    pub fn xrgb(width: u32, height: u32) -> Self {
        Self::with_format(width, height, PixelFormat::Xrgb)
    }

    fn with_format(width: u32, height: u32, format: PixelFormat) -> Self {
        let (width, height) = (width.min(u16::MAX.into()) as u16, height.min(u16::MAX.into()) as u16);
        Self { width, height, format, pixels: vec![0; usize::from(width) * usize::from(height)], drawn: 0 }
    }

    pub fn width(&self) -> u32 {
        self.width.into()
    }

    pub fn height(&self) -> u32 {
        self.height.into()
    }

    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// The frame as bytes, four per pixel, laid out as its format says.
    pub fn data(&self) -> &[u8] {
        bytemuck::cast_slice(&self.pixels)
    }

    /// The frame as one word per pixel. For an [`PixelFormat::Xrgb`] frame these are a window's pixels.
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    /// The colour of a pixel as stored (premultiplied in an RGBA8 frame), or `None` outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let pixel = self.pixels[y as usize * usize::from(self.width) + x as usize];
        Some(match self.format {
            PixelFormat::Rgba8 => {
                let [r, g, b, a] = pixel.to_ne_bytes();
                Color { r, g, b, a }
            }
            PixelFormat::Xrgb => {
                let [_, r, g, b] = pixel.to_be_bytes();
                Color { r, g, b, a: 255 }
            }
        })
    }

    /// Copies the frame as `0x00RRGGBB` pixels. Alpha is dropped, so draw over an opaque background.
    pub fn copy_to_xrgb(&self, out: &mut [u32]) {
        let whole = PixelRect { x: 0, y: 0, width: self.width(), height: self.height() };
        self.copy_rect_to(out, whole);
    }

    /// Copies one rectangle of the frame as `0x00RRGGBB` pixels into `out`, a buffer of the frame's size
    /// that already holds the rest. With [`Damage::rects`] this brings a window's buffer up to date.
    pub fn copy_rect_to(&self, out: &mut [u32], rect: PixelRect) {
        let stride = usize::from(self.width);
        let (left, right) = (rect.x as usize, ((rect.x + rect.width) as usize).min(stride));
        for y in rect.y as usize..((rect.y + rect.height) as usize).min(self.height.into()) {
            let row = y * stride;
            let (Some(from), Some(to)) =
                (self.pixels.get(row + left..row + right), out.get_mut(row + left..row + right))
            else {
                return;
            };
            match self.format {
                PixelFormat::Xrgb => to.copy_from_slice(from),
                PixelFormat::Rgba8 => to.iter_mut().zip(from).for_each(|(to, from)| *to = xrgb(*from)),
            }
        }
    }
}

/// An RGBA8 pixel as `0x00RRGGBB`.
fn xrgb(rgba: u32) -> u32 {
    u32::from_be_bytes(rgba.to_ne_bytes()) >> 8
}

/// Draws display lists on the CPU. Keep one for the life of a window: it holds the glyph and font caches,
/// and what it last drew.
#[derive(Default)]
pub struct CpuRenderer {
    context: Option<RenderContext>,
    resources: Resources,
    fonts: FontCache,
    /// What the frame of the last render shows.
    drawn: Option<Drawn>,
    /// Storage kept between renders: the next survey, and the rows a repaint draws into.
    survey: Survey,
    band: Vec<u8>,
}

/// What a frame shows, to repaint only what a new display list changes.
struct Drawn {
    /// Matches [`Frame::drawn`] for the one frame this describes.
    id: u64,
    survey: Survey,
    scale: f32,
    background: Color,
}

impl CpuRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `frame` show `list` over `background`, with logical pixels scaled by `scale`, and returns what
    /// it repainted. An image whose asset is not in `images` is left out.
    ///
    /// When `frame` is the one this renderer drew last, only what changed since is repainted. Any other
    /// frame is painted whole.
    pub fn render(
        &mut self,
        list: &DisplayList,
        images: &Images,
        scale: f32,
        background: Color,
        frame: &mut Frame,
    ) -> Damage {
        let whole = PixelRect { x: 0, y: 0, width: frame.width(), height: frame.height() };
        if whole.width == 0 || whole.height == 0 {
            return Damage::default();
        }
        let scale = paint::usable_scale(scale);
        let mut survey = std::mem::take(&mut self.survey);
        damage::survey(list, images, &mut self.fonts, &mut survey);

        let kept = self
            .drawn
            .as_ref()
            .filter(|drawn| drawn.id == frame.drawn && drawn.scale == scale && drawn.background == background);
        let regions = kept.and_then(|drawn| damage::regions(&drawn.survey, &survey, scale, whole.width, whole.height));
        let regions = match regions {
            Some(regions) => {
                for region in &regions {
                    self.repaint(list, images, scale, background, &survey, *region, frame);
                }
                regions
            }
            None => {
                self.paint(list, images, scale, background, frame);
                vec![whole]
            }
        };
        self.fonts.trim();

        static NEXT: AtomicU64 = AtomicU64::new(1);
        frame.drawn = NEXT.fetch_add(1, Ordering::Relaxed);
        let previous = self.drawn.replace(Drawn { id: frame.drawn, survey, scale, background });
        self.survey = previous.map(|drawn| drawn.survey).unwrap_or_default();
        Damage::new(regions)
    }

    fn context(&mut self, width: u16, height: u16) -> &mut RenderContext {
        let context = self.context.get_or_insert_with(|| RenderContext::new(width, height));
        context.reset_and_resize(width, height);
        context
    }

    /// Paints the whole frame.
    fn paint(&mut self, list: &DisplayList, images: &Images, scale: f32, background: Color, frame: &mut Frame) {
        let (width, height) = (frame.width, frame.height);
        self.context(width, height);
        let (Some(context), Some(target)) =
            (&mut self.context, PixmapMut::new(width, height, bytemuck::cast_slice_mut(&mut frame.pixels)))
        else {
            return;
        };
        let view = View { scale: scale.into(), shows: &|_| true };
        paint::paint(&mut CpuCanvas { context, resources: &mut self.resources, images }, &mut self.fonts, list, &view);
        context.flush();
        context.render_with(target, &mut self.resources, clear(background));
        if frame.format == PixelFormat::Xrgb {
            frame.pixels.iter_mut().for_each(|pixel| *pixel = xrgb(*pixel));
        }
    }

    /// Paints one rectangle of the frame afresh, from the items that can show in it.
    #[expect(clippy::too_many_arguments, reason = "one render's inputs, passed straight through")]
    fn repaint(
        &mut self,
        list: &DisplayList,
        images: &Images,
        scale: f32,
        background: Color,
        survey: &Survey,
        region: PixelRect,
        frame: &mut Frame,
    ) {
        // The scene keeps the whole frame's coordinates and the rows are drawn at full width, because
        // hinting and image sampling both depend on where a row starts.
        self.context(frame.width, frame.height);
        let (width, height) = (usize::from(frame.width), region.height as usize);
        self.band.resize(width * height * 4, 0);
        let target = PixmapMut::new(frame.width, region.height as u16, &mut self.band);
        let (Some(context), Some(target)) = (&mut self.context, target) else {
            return;
        };
        let view = View { scale: scale.into(), shows: &|index| survey[index].extent.touches(&region, scale) };
        paint::paint(&mut CpuCanvas { context, resources: &mut self.resources, images }, &mut self.fonts, list, &view);
        context.flush();
        let offset = (0, -(region.y as i32));
        context.render_with(target, &mut self.resources, RasterizerSettings { offset, ..clear(background) });

        let columns = region.x as usize..(region.x + region.width) as usize;
        let rows = self.band.as_chunks::<4>().0.chunks_exact(width);
        for (y, row) in (region.y as usize..).zip(rows) {
            let start = y * width;
            let kept = &mut frame.pixels[start + columns.start..start + columns.end];
            for (to, from) in kept.iter_mut().zip(&row[columns.clone()]) {
                let rgba = u32::from_ne_bytes(*from);
                *to = if frame.format == PixelFormat::Xrgb { xrgb(rgba) } else { rgba };
            }
        }
    }
}

fn clear(background: Color) -> RasterizerSettings {
    let background = AlphaColor::from_rgba8(background.r, background.g, background.b, background.a);
    RasterizerSettings { target_init: TargetInit::Clear(background), ..Default::default() }
}

struct CpuCanvas<'a> {
    context: &'a mut RenderContext,
    resources: &'a mut Resources,
    images: &'a Images,
}

impl Canvas for CpuCanvas<'_> {
    forward_canvas!(context, resources);

    fn image(&mut self, asset: &str) -> Option<(ImageSource, f64, f64)> {
        let image = self.images.get(asset)?;
        Some((ImageSource::Pixmap(image.pixels.clone()), image.width().into(), image.height().into()))
    }
}
