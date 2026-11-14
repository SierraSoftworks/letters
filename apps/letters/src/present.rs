//! Turns a display list into what a window shows.
//!
//! - [`Painter`] rasterises on the CPU into plain pixels, and needs no window.
//! - [`Presenter`] puts frames in a window: through a software surface at first, and on the GPU once
//!   [`Presenter::use_gpu`] has handed it one.

use std::num::NonZeroU32;
use std::sync::Arc;

use letters_render::cpu::{CpuRenderer, Damage, Frame};
use letters_render::image::Images;
use letters_render::{Color, DisplayList};
use winit::window::Window;

use crate::gpu::{Gpu, GpuSurface};

/// Keeps the CPU renderer and the frame it draws into between paints, so that a paint redraws only what
/// changed since the one before.
#[derive(Default)]
pub struct Painter {
    renderer: CpuRenderer,
    /// The latest frame, in the window's own pixel layout.
    frame: Frame,
    /// What the paint before this one changed, which a buffer two frames old has also missed.
    before: Damage,
}

impl Painter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `out` show `list` at `scale` over `background`: one `0x00RRGGBB` per pixel, `width` to a row.
    ///
    /// `age` says what `out` holds now: the frame of the paint before this one (1), of the one before that
    /// (2), or nothing that can be relied on (anything else).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        list: &DisplayList,
        images: &Images,
        scale: f32,
        background: Color,
        width: u32,
        age: u8,
        out: &mut [u32],
    ) {
        let height = (out.len() as u32).checked_div(width).unwrap_or(0);
        let resized = (self.frame.width(), self.frame.height()) != (width, height);
        if resized {
            self.frame = Frame::xrgb(width, height);
        }
        let damage = self.renderer.render(list, images, scale, background, &mut self.frame);
        let stale = match age {
            _ if resized => None,
            1 => Some(damage.clone()),
            2 => Some(damage.union(&self.before)),
            _ => None,
        };
        match stale {
            Some(stale) => stale.rects().iter().for_each(|rect| self.frame.copy_rect_to(out, *rect)),
            None => out.copy_from_slice(self.frame.pixels()),
        }
        self.before = damage;
    }
}

type SoftwareSurface = softbuffer::Surface<Arc<Window>, Arc<Window>>;

fn software(window: &Arc<Window>) -> Result<SoftwareSurface, softbuffer::SoftBufferError> {
    softbuffer::Surface::new(&softbuffer::Context::new(window.clone())?, window.clone())
}

enum Surface {
    Software(Box<SoftwareSurface>),
    Gpu(Box<GpuSurface>),
}

/// One window's way of getting frames on screen.
pub struct Presenter {
    window: Arc<Window>,
    surface: Surface,
    painter: Painter,
    /// Decoded assets by entry name. Empty until images are loaded after the first paint.
    images: Images,
}

impl Presenter {
    pub fn new(window: Arc<Window>) -> Result<Self, softbuffer::SoftBufferError> {
        let surface = Surface::Software(Box::new(software(&window)?));
        Ok(Self { window, surface, painter: Painter::new(), images: Images::default() })
    }

    pub fn on_gpu(&self) -> bool {
        matches!(self.surface, Surface::Gpu(_))
    }

    /// Moves the window to the GPU if it will take a surface there. Otherwise nothing changes.
    pub fn use_gpu(&mut self, gpu: &mut Gpu) {
        if let Some(surface) = gpu.surface(self.window.clone()) {
            self.surface = Surface::Gpu(Box::new(surface));
        }
    }

    /// Paints one frame and presents it. A GPU that fails is given up for the CPU, in the same call.
    pub fn present(&mut self, list: &DisplayList, background: Color) -> Result<(), softbuffer::SoftBufferError> {
        let size = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        if let Surface::Gpu(gpu) = &mut self.surface {
            self.window.pre_present_notify();
            match gpu.present(list, &self.images, scale, background, (size.width, size.height)) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    eprintln!("letters: {error}. Painting on the CPU from here on.");
                    // The GPU surface must be gone before the window can take a software one.
                    self.surface = Surface::Software(Box::new(software(&self.window)?));
                }
            }
        }
        let Surface::Software(surface) = &mut self.surface else { unreachable!("the GPU surface was just replaced") };
        surface.resize(width, height)?;
        let mut buffer = surface.buffer_mut()?;
        let age = buffer.age();
        self.painter.paint(list, &self.images, scale, background, size.width, age, &mut buffer);
        self.window.pre_present_notify();
        // The whole buffer is presented even when little of it changed: a redraw may be the system's way
        // of asking for all of the window again.
        buffer.present()
    }
}
