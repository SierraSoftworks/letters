//! Fixtures shared by the rendering tests.
#![allow(dead_code, reason = "each test file uses its own part")]

use std::path::PathBuf;

use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::{Image, Images};
use letters_render::{Color, DisplayList, Font, Glyph, Item, Point, Rect};
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::types::F2Dot14;
use skrifa::{FontRef, MetadataProvider};

pub const WHITE: Color = Color::rgb(255, 255, 255);
pub const BLACK: Color = Color::rgb(0, 0, 0);
pub const RED: Color = Color::rgb(220, 30, 30);
pub const BLUE: Color = Color::rgb(30, 60, 220);

pub fn list(items: impl IntoIterator<Item = Item>) -> DisplayList {
    DisplayList { items: items.into_iter().collect() }
}

pub fn fill(x: f32, y: f32, width: f32, height: f32, color: Color) -> Item {
    Item::Fill { rect: Rect::new(x, y, width, height), color, radius: 0.0 }
}

/// Draws `list` over white on a fresh renderer.
pub fn render(list: &DisplayList, width: u32, height: u32, scale: f32) -> Frame {
    render_with(list, &Images::default(), width, height, scale)
}

pub fn render_with(list: &DisplayList, images: &Images, width: u32, height: u32, scale: f32) -> Frame {
    let mut frame = Frame::new(width, height);
    CpuRenderer::new().render(list, images, scale, WHITE, &mut frame);
    frame
}

/// Whether two colours are within `tolerance` on every channel.
pub fn close(a: Color, b: Color, tolerance: u8) -> bool {
    [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)].iter().all(|(a, b)| a.abs_diff(*b) <= tolerance)
}

/// How much a frame departs from white inside a rectangle of physical pixels: the summed darkness.
pub fn ink(frame: &Frame, x: u32, y: u32, width: u32, height: u32) -> u64 {
    let pixels = (y..y + height).flat_map(|y| (x..x + width).map(move |x| (x, y)));
    pixels.filter_map(|(x, y)| frame.pixel(x, y)).map(|c| 765 - u64::from(c.r) - u64::from(c.g) - u64::from(c.b)).sum()
}

/// The bundled sans face at a weight.
pub fn sans(weight: f32) -> Font {
    instance(&letters_fonts::bundled()[0], weight)
}

/// The bundled monospace face, which has no italic.
pub fn mono() -> Font {
    instance(&letters_fonts::bundled()[2], 400.0)
}

/// The bundled display face at a weight.
pub fn display(weight: f32) -> Font {
    instance(&letters_fonts::bundled()[3], weight)
}

fn instance(font: &Font, weight: f32) -> Font {
    let face = FontRef::from_index(&font.data, font.index).expect("a bundled font parses");
    let location = face.axes().location([("wght", weight)]);
    font.clone().with_coords(location.coords().iter().map(|coord| coord.to_bits()).collect::<Vec<_>>())
}

/// A line of text set left to right from `origin` on its baseline, with no shaping beyond advances.
pub fn text(font: &Font, size: f32, color: Color, origin: Point, text: &str) -> Item {
    let face = FontRef::from_index(&font.data, font.index).expect("a bundled font parses");
    let coords: Vec<F2Dot14> = font.coords.iter().map(|bits| F2Dot14::from_bits(*bits)).collect();
    let metrics = face.glyph_metrics(Size::new(size), LocationRef::new(&coords));
    let charmap = face.charmap();
    let mut x = 0.0;
    let glyphs = text
        .chars()
        .map(|character| {
            let id = charmap.map(character).unwrap_or_default();
            let glyph = Glyph { id: id.to_u32(), x, y: 0.0 };
            x += metrics.advance_width(id).unwrap_or_default();
            glyph
        })
        .collect();
    Item::Glyphs { origin, font: font.clone(), size, color, skew: 0.0, embolden: false, glyphs }
}

/// A `width` by `height` image whose pixels come from `color(x, y)`.
pub fn image(width: u32, height: u32, color: impl Fn(u32, u32) -> Color) -> Image {
    let pixels = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
    let rgba = pixels.map(|(x, y)| color(x, y)).flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
    Image::from_rgba8(width, height, rgba).expect("a valid image")
}

/// Compares a frame with `tests/snapshots/<name>.png`, allowing for small differences in antialiasing between
/// machines. Run with `LETTERS_BLESS=1` to accept the frame as the new snapshot.
pub fn assert_snapshot(name: &str, frame: &Frame) {
    /// A pixel differs when a channel is off by more than this.
    const CHANNEL: u8 = 24;
    /// The share of pixels that may differ.
    const SHARE: f64 = 0.002;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(format!("{name}.png"));
    let save = |path: &PathBuf| {
        image::save_buffer(path, frame.data(), frame.width(), frame.height(), image::ColorType::Rgba8)
            .expect("the snapshot can be written");
    };
    if std::env::var_os("LETTERS_BLESS").is_some() {
        return save(&path);
    }

    let actual = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.png"));
    let hint = format!("see {} or accept it with LETTERS_BLESS=1", actual.display());
    let Ok(expected) = image::open(&path).map(|image| image.into_rgba8()) else {
        save(&actual);
        panic!("snapshot {name} is missing: {hint}");
    };
    let same_size = (expected.width(), expected.height()) == (frame.width(), frame.height());
    let differing = expected
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(frame.data().as_chunks::<4>().0)
        .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > CHANNEL))
        .count();
    let allowed = (f64::from(frame.width() * frame.height()) * SHARE) as usize;
    if !same_size || differing > allowed {
        save(&actual);
        panic!("snapshot {name} changed ({differing} pixels differ, {allowed} allowed): {hint}");
    }
}
