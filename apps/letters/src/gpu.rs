//! The GPU the windows share, and a window's surface on it.
//!
//! Finding an adapter and building the pipelines takes far longer than the launch budget allows, so
//! [`Gpu::start`] runs on its own thread once the first frame is on screen. Windows paint on the CPU until
//! it is ready, and go back to the CPU if the GPU ever fails them.

use std::sync::Arc;

use letters_render::gpu::{GpuRenderer, wgpu};
use letters_render::image::Images;
use letters_render::{Color, DisplayList};
use winit::window::Window;

/// Formats whose colours blend as they do on the CPU, most common first.
const FORMATS: [wgpu::TextureFormat; 2] = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm];

/// How a surface takes frames that are clear in places, as the renderer makes them.
const CLEAR: wgpu::CompositeAlphaMode = wgpu::CompositeAlphaMode::PreMultiplied;

pub struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// A renderer built ahead for the first window, in the format a surface most likely takes.
    ready: Option<GpuRenderer>,
}

impl Gpu {
    /// Slow: call from a thread of its own. `None` when there is no GPU worth using.
    ///
    /// A software adapter is refused unless `software` is set, since the CPU renderer beats it.
    pub fn start(software: bool) -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu && !software {
            return None;
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let ready = Some(GpuRenderer::new(&device, &queue, FORMATS[0]));
        Some(Self { instance, adapter, device, queue, ready })
    }

    /// A surface for presenting to `window`, or `None` if the window cannot take one. With `clear`, what
    /// a frame leaves clear shows what is behind the window, where the surface can do that.
    pub fn surface(&mut self, window: Arc<Window>, clear: bool) -> Option<GpuSurface> {
        let size = window.inner_size();
        let surface = self.instance.create_surface(window).ok()?;
        let capabilities = surface.get_capabilities(&self.adapter);
        let format = FORMATS.into_iter().find(|format| capabilities.formats.contains(format))?;
        let mut config = surface.get_default_config(&self.adapter, size.width.max(1), size.height.max(1))?;
        config.format = format;
        if clear && capabilities.alpha_modes.contains(&CLEAR) {
            config.alpha_mode = CLEAR;
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&self.device, &config);
        let renderer = match self.ready.take() {
            Some(renderer) if format == FORMATS[0] => renderer,
            _ => GpuRenderer::new(&self.device, &self.queue, format),
        };
        Some(GpuSurface { surface, config, renderer, device: self.device.clone(), queue: self.queue.clone() })
    }
}

pub struct GpuSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: GpuRenderer,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl GpuSurface {
    /// Whether what a frame leaves clear shows what is behind the window.
    pub fn clear(&self) -> bool {
        self.config.alpha_mode == CLEAR
    }

    /// Paints one frame of `width` by `height` physical pixels and presents it.
    ///
    /// An error means the surface is of no more use, and the caller should paint on the CPU.
    pub fn present(
        &mut self,
        list: &DisplayList,
        images: &Images,
        scale: f32,
        background: Color,
        (width, height): (u32, u32),
    ) -> Result<(), String> {
        if (self.config.width, self.config.height) != (width, height) {
            (self.config.width, self.config.height) = (width, height);
            self.surface.configure(&self.device, &self.config);
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            // Nothing is visible, or the display is busy: this frame is skipped, and the next one may land.
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => return Ok(()),
            // The window changed under the surface. Configuring again is the remedy, once.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                        frame
                    }
                    other => return Err(format!("the GPU surface was lost: {other:?}")),
                }
            }
            wgpu::CurrentSurfaceTexture::Validation => return Err("the GPU refused the surface".into()),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.render(list, images, scale, background, &view).map_err(|error| error.to_string())?;
        self.queue.present(frame);
        Ok(())
    }
}
