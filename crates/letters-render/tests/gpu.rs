//! The GPU backend must draw what the CPU backend draws. These need a graphics adapter (a software one
//! will do). On a machine with none they pass without checking anything, unless `LETTERS_GPU_TESTS` is set.
#![cfg(feature = "gpu")]

mod common;

use common::*;
use letters_render::gpu::{GpuRenderer, wgpu};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, Rect};

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: GpuRenderer,
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl Gpu {
    fn new() -> Option<Self> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()));
        let Ok(adapter) = adapter else {
            assert!(std::env::var_os("LETTERS_GPU_TESTS").is_none(), "no graphics adapter: {adapter:?}");
            return None;
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        let renderer = GpuRenderer::new(&device, &queue, FORMAT);
        Some(Self { device, queue, renderer })
    }

    /// Draws `list` over white and reads the pixels back as RGBA8.
    fn render(&mut self, list: &DisplayList, images: &Images, width: u32, height: u32, scale: f32) -> Vec<u8> {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.renderer.render(list, images, scale, WHITE, &view).expect("the frame renders");

        let stride = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(stride * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let layout = wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: None };
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo { buffer: &buffer, layout },
            size,
        );
        self.queue.submit([encoder.finish()]);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |mapped| mapped.expect("the frame maps"));
        self.device.poll(wgpu::PollType::wait_indefinitely()).expect("the device answers");
        let mapped = buffer.slice(..).get_mapped_range().expect("the frame is mapped");
        mapped.chunks(stride as usize).flat_map(|row| &row[..width as usize * 4]).copied().collect()
    }
}

/// The share of pixels that differ by more than a little on any channel.
fn difference(a: &[u8], b: &[u8]) -> f64 {
    let (a, b) = (a.as_chunks::<4>().0, b.as_chunks::<4>().0);
    let differing = a.iter().zip(b).filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 24)).count();
    differing as f64 / a.len().max(1) as f64
}

#[test]
fn the_gpu_draws_what_the_cpu_draws() {
    let Some(mut gpu) = Gpu::new() else { return };
    let images = scene_images();
    for (name, scene, scale) in SCENES {
        let (width, height) = ((240.0 * scale) as u32, (160.0 * scale) as u32);
        let list = list(scene());

        let on_gpu = gpu.render(&list, &images, width, height, scale);
        let on_cpu = render_with(&list, &images, width, height, scale);

        let share = difference(&on_gpu, on_cpu.data());
        assert!(share < 0.005, "{name}: {:.2}% of pixels differ", share * 100.0);
    }
}

#[test]
fn an_image_follows_its_asset() {
    let Some(mut gpu) = Gpu::new() else { return };
    let picture = list([Item::Image { rect: Rect::new(0.0, 0.0, 16.0, 16.0), asset: "tile".into() }]);
    let mut images = Images::default();
    let mut centre = |images: &Images| {
        let pixels = gpu.render(&picture, images, 16, 16, 1.0);
        let at = (8 * 16 + 8) * 4;
        Color::rgba(pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3])
    };

    assert_eq!(centre(&images), WHITE, "not decoded yet");
    images.insert("tile", image(4, 4, |_, _| RED));
    assert_eq!(centre(&images), RED, "decoded");
    images.insert("tile", image(4, 4, |_, _| BLUE));
    assert_eq!(centre(&images), BLUE, "replaced");
    images.remove("tile");
    assert_eq!(centre(&images), WHITE, "removed");
}
