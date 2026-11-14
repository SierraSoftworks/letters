//! Turns a display list into the pixels a software surface shows.

use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::Images;
use letters_render::{Color, DisplayList};

/// One per window: it keeps the renderer's glyph caches and the frame between paints.
#[derive(Default)]
pub struct Painter {
    renderer: CpuRenderer,
    frame: Frame,
    /// Decoded assets by entry name. Empty until images are loaded after the first paint.
    images: Images,
}

impl Painter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Paints `list` at `scale` over `background` into `out`: one `0x00RRGGBB` per pixel, `width` to a row.
    pub fn paint(&mut self, list: &DisplayList, scale: f32, background: Color, width: u32, out: &mut [u32]) {
        let height = (out.len() as u32).checked_div(width).unwrap_or(0);
        if (self.frame.width(), self.frame.height()) != (width, height) {
            self.frame = Frame::new(width, height);
        }
        self.renderer.render(list, &self.images, scale, background, &mut self.frame);
        self.frame.copy_to_xrgb(out);
    }
}
