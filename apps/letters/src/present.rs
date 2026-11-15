//! Turns a display list into what a window shows.
//!
//! - [`Painter`] rasterises on the CPU into plain pixels, and needs no window.
//! - [`Presenter`] puts frames in a window: through a software surface at first, and on the GPU once
//!   [`Presenter::use_gpu`] has handed it one. On Wayland the frames have alpha, so a window can leave
//!   its corners clear, and the presenter lays the window's shadow round it.

use std::num::NonZeroU32;
use std::sync::Arc;

use letters_frame::Strip;
use letters_render::cpu::{CpuRenderer, Damage, Frame, PixelFormat};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, Rect};
use winit::window::Window;

use crate::gpu::{Gpu, GpuSurface};
use crate::shadow::{CORNER, Shadow};

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

    /// Makes `out` show `list` at `scale` over `background`, `width` pixels to a row: one `0x00RRGGBB`
    /// each, or `0xAARRGGBB` with the colours multiplied by alpha when `alpha` is set.
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
        alpha: bool,
        width: u32,
        age: u8,
        out: &mut [u32],
    ) {
        let height = (out.len() as u32).checked_div(width).unwrap_or(0);
        let format = if alpha { PixelFormat::Argb } else { PixelFormat::Xrgb };
        let resized = (self.frame.width(), self.frame.height(), self.frame.format()) != (width, height, format);
        if resized {
            self.frame = if alpha { Frame::argb(width, height) } else { Frame::xrgb(width, height) };
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

    /// Rounds the corners of what was just painted with alpha into `out`, by `radius` pixels: outside
    /// them `out` is left clear.
    ///
    /// The frame itself keeps its corners. Clipping the display list instead would round them too, but
    /// then every frame that repaints much of the window pays for a layer as large as the window.
    pub fn round(&self, radius: u32, out: &mut [u32]) {
        let (width, height) = (self.frame.width() as usize, self.frame.height() as usize);
        let (pixels, radius) = (self.frame.pixels(), radius as usize);
        if self.frame.format() != PixelFormat::Argb || out.len() != pixels.len() || 2 * radius > width.min(height) {
            return;
        }
        for (y, x) in (0..radius).flat_map(|y| (0..radius).map(move |x| (y, x))) {
            // How much of this pixel of the top left corner is inside the circle the corner follows.
            let from_centre = (radius as f32 - x as f32 - 0.5).hypot(radius as f32 - y as f32 - 0.5);
            let inside = ((radius as f32 - from_centre + 0.5).clamp(0.0, 1.0) * 256.0) as u32;
            if inside == 256 {
                continue;
            }
            for (x, y) in [(x, y), (width - 1 - x, y), (x, height - 1 - y), (width - 1 - x, height - 1 - y)] {
                let at = y * width + x;
                out[at] =
                    u32::from_be_bytes(pixels[at].to_be_bytes().map(|part| ((u32::from(part) * inside) >> 8) as u8));
            }
        }
    }
}

type SoftwareSurface = softbuffer::Surface<Arc<Window>, Arc<Window>>;

fn software(window: &Arc<Window>) -> Result<SoftwareSurface, softbuffer::SoftBufferError> {
    softbuffer::Surface::new(&softbuffer::Context::new(window.clone())?, window.clone())
}

/// A Wayland window's own surface, painted without a software surface between.
type Layer = letters_frame::wayland::Surface<Arc<Window>>;

enum Surface {
    Software(Box<SoftwareSurface>),
    /// Frames with alpha, through the presenter's [`Layer`].
    Layered,
    Gpu(Box<GpuSurface>),
}

/// What a shadow was last cut for: a window's size in its own units, the scale and the scheme.
type Shaded = (u32, u32, u32, bool);

/// One window's way of getting frames on screen.
pub struct Presenter {
    window: Arc<Window>,
    surface: Surface,
    painter: Painter,
    /// Decoded assets by entry name. Empty until images are loaded after the first paint.
    images: Images,
    /// The window's surface where it is a Wayland one, whatever paints it.
    layer: Option<Layer>,
    /// The shadow the window's is cut from, and what the one round it now was cut for.
    shadow: Option<Shadow>,
    shaded: Option<Shaded>,
}

impl Presenter {
    pub fn new(window: Arc<Window>) -> Result<Self, softbuffer::SoftBufferError> {
        let layer = Layer::new(window.clone());
        let surface = match layer {
            Some(_) => Surface::Layered,
            None => Surface::Software(Box::new(software(&window)?)),
        };
        let (painter, images) = (Painter::new(), Images::default());
        Ok(Self { window, surface, painter, images, layer, shadow: None, shaded: None })
    }

    /// Casts the window's shadow, or takes it away, where the desktop leaves the shadow to the window.
    /// It changes with the next frame.
    pub fn shade(&mut self, cast: bool, dark: bool) {
        let Some(layer) = &mut self.layer else { return };
        let scale = self.window.scale_factor();
        let size = self.window.inner_size().to_logical::<f64>(scale);
        let (width, height, scale) = (size.width.round() as u32, size.height.round() as u32, scale.ceil() as u32);
        let wanted = cast.then_some((width, height, scale, dark));
        if self.shaded == wanted {
            return;
        }
        let pieces = wanted.and_then(|_| {
            let kept = self.shadow.take().filter(|shadow| (shadow.scale(), shadow.dark()) == (scale, dark));
            self.shadow.insert(kept.unwrap_or_else(|| Shadow::new(scale, dark))).pieces(width, height)
        });
        let strips = pieces.iter().flatten().map(|piece| Strip {
            x: piece.x,
            y: piece.y,
            width: piece.width,
            height: piece.height,
            scale,
            pixels: &piece.pixels,
        });
        layer.set_shadow(&strips.collect::<Vec<_>>());
        self.shaded = wanted;
    }

    pub fn on_gpu(&self) -> bool {
        matches!(self.surface, Surface::Gpu(_))
    }

    /// Moves the window to the GPU if it will take a surface there. Otherwise nothing changes.
    pub fn use_gpu(&mut self, gpu: &mut Gpu) {
        // A window that could leave its corners clear until now goes on doing so if the GPU lets it.
        if let Some(surface) = gpu.surface(self.window.clone(), self.layer.is_some()) {
            self.surface = Surface::Gpu(Box::new(surface));
            if let Some(layer) = &mut self.layer {
                layer.release_frames();
            }
        }
    }

    /// Paints one frame and presents it. A GPU that fails is given up for the CPU, in the same call.
    ///
    /// `round` asks for round corners, and is the colour of the line that follows them. They are round
    /// where the surface can leave them clear, and square anywhere else. The list gets that line.
    pub fn present(
        &mut self,
        list: &mut DisplayList,
        background: Color,
        round: Option<Color>,
    ) -> Result<(), softbuffer::SoftBufferError> {
        let size = self.window.inner_size();
        let scale = self.window.scale_factor() as f32;
        let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            return Ok(());
        };
        let rect = Rect::new(0.0, 0.0, size.width as f32 / scale, size.height as f32 / scale);
        let line = |color| Item::Stroke { rect, color, width: 1.0, radius: CORNER as f32 };
        if let Surface::Gpu(gpu) = &mut self.surface {
            let mut background = background;
            if let Some(color) = round.filter(|_| gpu.clear()) {
                // The GPU has no frame to round afterwards, so the corners are cut out of the list.
                list.items.insert(0, Item::PushClip { rect, radius: CORNER as f32 });
                list.items.extend([Item::PopClip, line(color)]);
                background = Color::rgba(0, 0, 0, 0);
            }
            self.window.pre_present_notify();
            match gpu.present(list, &self.images, scale, background, (size.width, size.height)) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    eprintln!("letters: {error}. Painting on the CPU from here on.");
                    // The GPU surface must be gone before the window can take a software one.
                    self.surface = match self.layer {
                        Some(_) => Surface::Layered,
                        None => Surface::Software(Box::new(software(&self.window)?)),
                    };
                    // What the GPU was given is cut off again, down to the view's own items.
                    if matches!(list.items.first(), Some(Item::PushClip { .. })) && round.is_some() {
                        list.items.remove(0);
                        list.items.truncate(list.items.len().saturating_sub(2));
                    }
                }
            }
        }
        if let (Surface::Layered, Some(layer)) = (&self.surface, &mut self.layer) {
            if let Some(mut frame) = layer.frame(size.width, size.height) {
                let age = frame.age();
                // The view's own border is square: the line joins up what rounding leaves of it.
                list.items.extend(round.map(line));
                self.painter.paint(list, &self.images, scale, background, true, size.width, age, frame.pixels());
                if round.is_some() {
                    self.painter.round((CORNER as f32 * scale).round() as u32, frame.pixels());
                }
                self.window.pre_present_notify();
                frame.present();
                return Ok(());
            }
            // No memory could be shared with the compositor: an ordinary software surface has ways of its own.
            self.surface = Surface::Software(Box::new(software(&self.window)?));
        }
        let Surface::Software(surface) = &mut self.surface else { unreachable!("any other surface was just replaced") };
        surface.resize(width, height)?;
        let mut buffer = surface.buffer_mut()?;
        let age = buffer.age();
        self.painter.paint(list, &self.images, scale, background, false, size.width, age, &mut buffer);
        self.window.pre_present_notify();
        // The whole buffer is presented even when little of it changed: a redraw may be the system's way
        // of asking for all of the window again.
        buffer.present()
    }
}
