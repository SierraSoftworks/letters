//! Whole scenes compared with stored pictures, to catch changes no single probe would.

mod common;

use std::sync::Arc;

use common::*;
use letters_render::image::Images;
use letters_render::{Color, Item, PathEl, Point, Rect};

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
        Item::Glyphs { origin, font, size, color, skew, embolden, glyphs }
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

#[test]
fn scenes_match_their_snapshots() {
    let mut images = Images::default();
    images.insert("assets/swatch", image(16, 16, |x, y| Color::rgb(40 + x as u8 * 13, 90 + y as u8 * 9, 200)));
    type Scene = fn() -> Vec<Item>;
    let scenes: [(&str, Scene, f32); 3] =
        [("shapes", shapes, 1.0), ("type", type_specimen, 1.0), ("card@2x", card, 2.0)];

    for (name, scene, scale) in scenes {
        let (width, height) = ((240.0 * scale) as u32, (160.0 * scale) as u32);
        let frame = render_with(&list(scene()), &images, width, height, scale);

        assert_snapshot(name, &frame);
    }
}
