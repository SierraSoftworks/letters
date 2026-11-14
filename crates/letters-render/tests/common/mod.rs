//! Fixtures shared by the rendering tests.
#![allow(dead_code, reason = "each test file uses its own part")]

use std::path::PathBuf;
use std::sync::Arc;

use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::{Image, Images};
use letters_render::snapshot::Shot;
use letters_render::{Color, DisplayList, Font, Glyph, Item, PathEl, Point, Rect};
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

/// The largest difference between two frames on any channel of any pixel.
pub fn furthest_apart(a: &Frame, b: &Frame) -> u8 {
    assert_eq!((a.width(), a.height(), a.format()), (b.width(), b.height(), b.format()), "the frames match");
    a.data().iter().zip(b.data()).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0)
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
    Item::Glyphs { origin, font: font.clone(), size, color, skew: 0.0, embolden: false, glyphs, text: None }
}

/// Items that have nothing to draw, each with what is wrong with it. One draws an image named `pixel`.
pub fn undrawable() -> Vec<(&'static str, Item)> {
    let rect = |x, y, width, height| Rect::new(x, y, width, height);
    let glyphs = |font: Font, size, glyphs| Item::Glyphs {
        origin: Point { x: 5.0, y: 20.0 },
        font,
        size,
        color: BLACK,
        skew: 0.0,
        embolden: false,
        glyphs,
        text: None,
    };
    let path = |path: Vec<PathEl>, scale| Item::Path {
        origin: Point::default(),
        scale,
        path: path.into(),
        color: BLACK,
        stroke: Some(1.0),
    };
    let garbage = Font::new(Arc::from(&b"not a font"[..]), 0);
    let glyph = |id| vec![Glyph { id, x: 0.0, y: 0.0 }];
    vec![
        ("an empty fill", fill(5.0, 5.0, 0.0, 10.0, BLACK)),
        ("a negative fill", fill(5.0, 5.0, -10.0, 10.0, BLACK)),
        ("a fill that is not a number", fill(f32::NAN, 5.0, 10.0, 10.0, BLACK)),
        ("an infinite fill", fill(0.0, 0.0, f32::INFINITY, 10.0, BLACK)),
        (
            "a radius that is not a number",
            Item::Fill { rect: rect(40.0, 40.0, 5.0, 5.0), color: WHITE, radius: f32::NAN },
        ),
        (
            "a stroke of no width",
            Item::Stroke { rect: rect(5.0, 5.0, 20.0, 20.0), color: BLACK, width: 0.0, radius: 0.0 },
        ),
        (
            "a stroke that is not a number",
            Item::Stroke { rect: rect(5.0, 5.0, 20.0, 20.0), color: BLACK, width: f32::NAN, radius: 0.0 },
        ),
        (
            "a shadow that is not a number",
            Item::Shadow {
                rect: rect(5.0, 5.0, 20.0, 20.0),
                color: BLACK,
                blur: f32::NAN,
                offset: Point::default(),
                radius: 0.0,
            },
        ),
        ("a font that is not a font", glyphs(garbage, 16.0, glyph(1))),
        ("a glyph the font does not have", glyphs(sans(400.0), 16.0, glyph(u32::MAX))),
        ("text of no size", glyphs(sans(400.0), 0.0, glyph(40))),
        ("an empty run", glyphs(sans(400.0), 16.0, Vec::new())),
        ("an empty path", path(Vec::new(), 1.0)),
        ("a path with no start", path(vec![PathEl::LineTo(Point { x: 10.0, y: 10.0 })], 1.0)),
        (
            "a path of no scale",
            path(vec![PathEl::MoveTo(Point::default()), PathEl::LineTo(Point { x: 10.0, y: 10.0 })], 0.0),
        ),
        ("an empty image", Item::Image { rect: rect(5.0, 5.0, 0.0, 0.0), asset: "pixel".into() }),
    ]
}

/// A `width` by `height` image whose pixels come from `color(x, y)`.
pub fn image(width: u32, height: u32, color: impl Fn(u32, u32) -> Color) -> Image {
    let pixels = (0..height).flat_map(|y| (0..width).map(move |x| (x, y)));
    let rgba = pixels.map(|(x, y)| color(x, y)).flat_map(|c| [c.r, c.g, c.b, c.a]).collect();
    Image::from_rgba8(width, height, rgba).expect("a valid image")
}

/// Where the snapshot of a name is kept.
pub fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots").join(format!("{name}.png"))
}

/// A list to snapshot, drawn over white.
pub fn shot<'a>(list: &'a DisplayList, images: &'a Images, width: u32, height: u32, scale: f32) -> Shot<'a> {
    Shot { list, images, size: (width, height), scale, background: WHITE }
}

/// A scene 240 by 160 logical pixels, and the display scale to draw it at.
pub type Scene = (&'static str, fn() -> Vec<Item>, f32);

/// Scenes that between them use every kind of item.
pub const SCENES: [Scene; 3] = [("shapes", shapes, 1.0), ("type", type_specimen, 1.0), ("card@2x", card, 2.0)];

/// The images the scenes draw.
pub fn scene_images() -> Images {
    let mut images = Images::default();
    images.insert("assets/swatch", image(16, 16, |x, y| Color::rgb(40 + x as u8 * 13, 90 + y as u8 * 9, 200)));
    images
}

const INK: Color = Color::rgb(28, 30, 38);
const MUTED: Color = Color::rgb(110, 116, 130);
const PAPER: Color = Color::rgb(244, 245, 248);
const ACCENT: Color = Color::rgb(64, 99, 235);

fn at(x: f32, y: f32) -> Point {
    Point { x, y }
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::new(x, y, width, height)
}

/// Fills, strokes and shadows at a range of radii and opacities.
fn shapes() -> Vec<Item> {
    let mut items = vec![fill(0.0, 0.0, 240.0, 160.0, PAPER)];
    for (column, radius) in [0.0, 4.0, 12.0, 40.0].into_iter().enumerate() {
        let x = 12.0 + column as f32 * 57.0;
        items.extend([
            Item::Fill { rect: rect(x, 12.0, 45.0, 36.0), color: ACCENT, radius },
            Item::Stroke { rect: rect(x, 60.0, 45.0, 36.0), color: INK, width: 1.0 + column as f32, radius },
            Item::Shadow {
                rect: rect(x, 110.0, 45.0, 36.0),
                color: Color::rgba(0, 0, 0, 90),
                blur: 4.0 * column as f32,
                offset: at(0.0, 3.0),
                radius,
            },
            Item::Fill { rect: rect(x, 110.0, 45.0, 36.0), color: Color::rgb(255, 255, 255), radius },
            Item::Fill { rect: rect(x + 15.0, 30.0, 45.0, 50.0), color: Color::rgba(220, 30, 30, 70), radius },
        ]);
    }
    items
}

/// The bundled faces across sizes, weights and synthetic styles.
fn type_specimen() -> Vec<Item> {
    let mut items = vec![fill(0.0, 0.0, 240.0, 160.0, Color::rgb(255, 255, 255))];
    items.push(text(&display(700.0), 22.0, INK, at(10.0, 28.0), "Letters, typeset"));
    for (row, weight) in [300.0, 400.0, 600.0, 800.0].into_iter().enumerate() {
        let size = 10.0 + 2.0 * row as f32;
        let line = format!("Weight {weight} at {size} px: fjord quiz");
        items.push(text(&sans(weight), size, INK, at(10.0, 50.0 + 20.0 * row as f32), &line));
    }
    let synthetic = |label, skew, embolden, y| {
        let Item::Glyphs { origin, font, size, color, glyphs, .. } = text(&mono(), 13.0, ACCENT, at(10.0, y), label)
        else {
            unreachable!()
        };
        Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, text: None }
    };
    items.push(synthetic("let slanted = code();", 14.0, false, 132.0));
    items.push(synthetic("let heavier = code();", 0.0, true, 150.0));
    items
}

/// A card as the interface draws one, on a display with two physical pixels per logical pixel.
fn card() -> Vec<Item> {
    let panel = rect(20.0, 16.0, 200.0, 128.0);
    let tick: Arc<[PathEl]> =
        Arc::new([PathEl::MoveTo(at(20.0, 6.0)), PathEl::LineTo(at(9.0, 17.0)), PathEl::LineTo(at(4.0, 12.0))]);
    let pin: Arc<[PathEl]> = Arc::new([
        PathEl::MoveTo(at(12.0, 2.0)),
        PathEl::CurveTo(at(17.0, 2.0), at(20.0, 6.0), at(20.0, 10.0)),
        PathEl::QuadTo(at(20.0, 16.0), at(12.0, 22.0)),
        PathEl::QuadTo(at(4.0, 16.0), at(4.0, 10.0)),
        PathEl::CurveTo(at(4.0, 6.0), at(7.0, 2.0), at(12.0, 2.0)),
        PathEl::Close,
        PathEl::MoveTo(at(15.0, 10.0)),
        PathEl::QuadTo(at(15.0, 13.0), at(12.0, 13.0)),
        PathEl::QuadTo(at(9.0, 13.0), at(9.0, 10.0)),
        PathEl::QuadTo(at(9.0, 7.0), at(12.0, 7.0)),
        PathEl::QuadTo(at(15.0, 7.0), at(15.0, 10.0)),
        PathEl::Close,
    ]);
    vec![
        fill(0.0, 0.0, 240.0, 160.0, PAPER),
        Item::Shadow {
            rect: panel,
            color: Color::rgba(20, 30, 60, 70),
            blur: 18.0,
            offset: at(0.0, 6.0),
            radius: 10.0,
        },
        Item::Fill { rect: panel, color: Color::rgb(255, 255, 255), radius: 10.0 },
        Item::Stroke { rect: panel, color: Color::rgb(222, 225, 232), width: 1.0, radius: 10.0 },
        Item::PushClip { rect: rect(32.0, 28.0, 56.0, 56.0), radius: 8.0 },
        Item::Image { rect: rect(32.0, 28.0, 56.0, 56.0), asset: "assets/swatch".into() },
        Item::PopClip,
        text(&display(700.0), 15.0, INK, at(100.0, 46.0), "Quarterly"),
        text(&sans(400.0), 11.0, MUTED, at(100.0, 62.0), "Edited 2 min ago"),
        Item::Path { origin: at(100.0, 68.0), scale: 0.6, path: pin, color: ACCENT, stroke: None },
        text(&sans(500.0), 10.0, ACCENT, at(118.0, 79.0), "Shared"),
        Item::Fill { rect: rect(32.0, 98.0, 176.0, 1.0), color: Color::rgb(232, 234, 240), radius: 0.0 },
        Item::Fill { rect: rect(32.0, 110.0, 74.0, 22.0), color: ACCENT, radius: 6.0 },
        Item::Path {
            origin: at(38.0, 114.0),
            scale: 0.6,
            path: tick,
            color: Color::rgb(255, 255, 255),
            stroke: Some(2.0),
        },
        text(&sans(600.0), 11.0, Color::rgb(255, 255, 255), at(56.0, 125.0), "Saved"),
        Item::Fill { rect: rect(116.0, 110.0, 92.0, 22.0), color: Color::rgb(238, 240, 245), radius: 6.0 },
        text(&mono(), 10.0, INK, at(124.0, 125.0), "report.let"),
    ]
}
