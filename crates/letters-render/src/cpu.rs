//! The CPU backend: draws a display list into memory.
//!
//! It paints the first frame and every frame on a host with no GPU, and it is what pixel tests run on.

use vello_cpu::color::AlphaColor;
use vello_cpu::{ImageSource, PixmapMut, RasterizerSettings, RenderContext, Resources, TargetInit};

use crate::image::Images;
use crate::paint::{self, Canvas, FontCache, forward_canvas};
use crate::{Color, DisplayList};

/// The pixels of one frame: premultiplied RGBA8, row by row from the top left.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    width: u16,
    height: u16,
    data: Vec<u8>,
}

impl Frame {
    /// A transparent frame of physical pixels. Each side is capped at 65535.
    pub fn new(width: u32, height: u32) -> Self {
        let (width, height) = (width.min(u16::MAX.into()) as u16, height.min(u16::MAX.into()) as u16);
        Self { width, height, data: vec![0; usize::from(width) * usize::from(height) * 4] }
    }

    pub fn width(&self) -> u32 {
        self.width.into()
    }

    pub fn height(&self) -> u32 {
        self.height.into()
    }

    /// Four bytes per pixel: red, green, blue and alpha, with the colours multiplied by alpha.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// The premultiplied colour of a pixel, or `None` outside the frame.
    pub fn pixel(&self, x: u32, y: u32) -> Option<Color> {
        if x >= self.width() || y >= self.height() {
            return None;
        }
        let at = (y as usize * usize::from(self.width) + x as usize) * 4;
        let [r, g, b, a] = self.data[at..at + 4] else { return None };
        Some(Color { r, g, b, a })
    }

    /// Copies the frame as `0x00RRGGBB` pixels, the layout a window's software surface takes. Alpha is
    /// dropped, so draw over an opaque background.
    pub fn copy_to_xrgb(&self, out: &mut [u32]) {
        for (out, pixel) in out.iter_mut().zip(self.data.as_chunks::<4>().0) {
            *out = u32::from_be_bytes([0, pixel[0], pixel[1], pixel[2]]);
        }
    }
}

/// Draws display lists on the CPU. Keep one for the life of a window: it holds the glyph and font caches.
#[derive(Default)]
pub struct CpuRenderer {
    context: Option<RenderContext>,
    resources: Resources,
    fonts: FontCache,
}

impl CpuRenderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fills `frame` with `background`, then draws `list` over it with logical pixels scaled by `scale`.
    /// An image whose asset is not in `images` is left out.
    pub fn render(&mut self, list: &DisplayList, images: &Images, scale: f32, background: Color, frame: &mut Frame) {
        let (width, height) = (frame.width, frame.height);
        let Some(target) = PixmapMut::new(width, height, &mut frame.data).filter(|_| width > 0 && height > 0) else {
            return;
        };
        let context = match &mut self.context {
            Some(context) if context.width() == width && context.height() == height => {
                context.reset();
                context
            }
            Some(context) => {
                context.reset_and_resize(width, height);
                context
            }
            empty => empty.insert(RenderContext::new(width, height)),
        };

        let mut canvas = CpuCanvas { context, resources: &mut self.resources, images };
        paint::paint(&mut canvas, &mut self.fonts, list, scale);
        self.fonts.trim();

        let background = AlphaColor::from_rgba8(background.r, background.g, background.b, background.a);
        let settings = RasterizerSettings { target_init: TargetInit::Clear(background), ..Default::default() };
        context.flush();
        context.render_with(target, &mut self.resources, settings);
    }
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
