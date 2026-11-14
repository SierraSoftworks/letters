//! The GPU backend: draws a display list into a texture with Vello.
//!
//! The shell owns the device, the queue and the window's surface, and hands each frame's texture in. This
//! module only turns a display list into draw calls, so it draws exactly what [`crate::cpu`] draws.

use std::collections::HashMap;
use std::sync::Arc;

use vello_cpu::color::AlphaColor;
use vello_cpu::{ImageSource, Pixmap};
pub use vello_gpu::RenderError as GpuError;
use vello_gpu::{
    ClearSettings, RenderOptions, RenderTarget, RenderTargetConfig, Renderer, Resources, Scene, TargetInit,
    TextureBindings, TextureId, TextureRegion,
};
/// The wgpu this backend is built against. A shell must create its device with this version.
pub use wgpu;

use crate::image::{Image, Images};
use crate::paint::{self, Canvas, FontCache, View, forward_canvas};
use crate::{Color, DisplayList};

/// Draws display lists on the GPU. Keep one for the life of a window: it holds the pipelines, the glyph
/// and font caches and the uploaded images.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    resources: Resources,
    scene: Scene,
    fonts: FontCache,
    textures: Textures,
}

impl GpuRenderer {
    /// A renderer for targets of `format`. Use a format that is not sRGB-encoded (`Bgra8Unorm`, not
    /// `Bgra8UnormSrgb`), so colours blend as they do on the CPU.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        // The size here only seeds buffers: each frame takes its size from its target.
        let (renderer, resources) = Renderer::new(device, queue, &RenderTargetConfig { format, width: 1, height: 1 });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            renderer,
            resources,
            scene: Scene::new(1, 1),
            fonts: FontCache::default(),
            textures: Textures::default(),
        }
    }

    /// Fills `target` with `background`, draws `list` over it with logical pixels scaled by `scale`, and
    /// submits the work to the queue. An image whose asset is not in `images`, or is larger than the device's
    /// textures may be, is left out.
    pub fn render(
        &mut self,
        list: &DisplayList,
        images: &Images,
        scale: f32,
        background: Color,
        target: &wgpu::TextureView,
    ) -> Result<(), GpuError> {
        let size = |side: u32| side.min(u16::MAX.into()) as u16;
        let (width, height) = (size(target.texture().width()), size(target.texture().height()));
        if self.scene.width() == width && self.scene.height() == height {
            self.scene.reset();
        } else {
            self.scene.reset_and_resize(width, height);
        }

        self.textures.keep(images);
        let mut canvas = GpuCanvas {
            scene: &mut self.scene,
            resources: &mut self.resources,
            images,
            textures: &mut self.textures,
            device: &self.device,
            queue: &self.queue,
        };
        let view = View { scale: paint::usable_scale(scale).into(), shows: &|_| true };
        paint::paint(&mut canvas, &mut self.fonts, list, &view);
        self.fonts.trim();

        let background = AlphaColor::from_rgba8(background.r, background.g, background.b, background.a);
        let mut options = RenderOptions::default();
        options.target_init = TargetInit::Clear(ClearSettings::Viewport { color: background });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let drawn = self.renderer.render(
            &self.scene,
            &mut self.resources,
            &mut encoder,
            RenderTarget { view: target, depth_view: None },
            &self.textures.bindings,
            options,
        );
        // Submitted even on failure: the renderer's own state depends on it.
        self.queue.submit([encoder.finish()]);
        drawn
    }
}

/// The images uploaded to the GPU, by asset entry name.
#[derive(Default)]
struct Textures {
    uploaded: HashMap<String, Uploaded>,
    bindings: TextureBindings,
    next: u64,
}

struct Uploaded {
    /// The pixels the texture was made from, to notice an asset being replaced.
    pixels: Arc<Pixmap>,
    id: TextureId,
}

impl Textures {
    /// Drops the textures of assets that are gone or replaced.
    fn keep(&mut self, images: &Images) {
        let bindings = &mut self.bindings;
        self.uploaded.retain(|asset, uploaded| {
            let current = images.get(asset).is_some_and(|image| Arc::ptr_eq(&image.pixels, &uploaded.pixels));
            if !current {
                bindings.remove(uploaded.id);
            }
            current
        });
    }

    /// The texture for an image, uploading it on first use.
    fn get(&mut self, asset: &str, image: &Image, device: &wgpu::Device, queue: &wgpu::Queue) -> Option<TextureId> {
        if let Some(uploaded) = self.uploaded.get(asset) {
            return Some(uploaded.id);
        }
        let limit = device.limits().max_texture_dimension_2d;
        if image.width() > limit || image.height() > limit {
            return None;
        }
        let size = wgpu::Extent3d { width: image.width(), height: image.height(), depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(asset),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let layout =
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(image.width() * 4), rows_per_image: None };
        queue.write_texture(texture.as_image_copy(), image.pixels.data_as_u8_slice(), layout, size);

        let id = TextureId(self.next);
        self.next += 1;
        self.bindings.insert(id, texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.uploaded.insert(asset.to_owned(), Uploaded { pixels: image.pixels.clone(), id });
        Some(id)
    }
}

struct GpuCanvas<'a> {
    scene: &'a mut Scene,
    resources: &'a mut Resources,
    images: &'a Images,
    textures: &'a mut Textures,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
}

impl Canvas for GpuCanvas<'_> {
    forward_canvas!(scene, resources);

    fn image(&mut self, asset: &str) -> Option<(ImageSource, f64, f64)> {
        let image = self.images.get(asset)?;
        let id = self.textures.get(asset, image, self.device, self.queue)?;
        let transparent = image.pixels.may_have_transparency();
        let source = ImageSource::external_texture(id, TextureRegion::Full, transparent);
        Some((source, image.width().into(), image.height().into()))
    }
}
