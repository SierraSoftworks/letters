//! What the CPU backend puts on screen for each kind of item, checked by probing pixels.

mod common;

use std::sync::Arc;

use common::*;
use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Font, Item, PathEl, Point, Rect, Role};

fn at(frame: &Frame, x: u32, y: u32) -> Color {
    frame.pixel(x, y).expect("the probe is inside the frame")
}

#[test]
fn a_fill_covers_its_rectangle_and_nothing_else() {
    // (scale, a pixel inside, a pixel outside), for a 20 by 10 fill at (10, 10).
    for (scale, inside, outside) in [(1.0, (29, 19), (30, 20)), (2.0, (59, 39), (60, 40)), (0.5, (14, 9), (15, 10))] {
        let frame = render(&list([fill(10.0, 10.0, 20.0, 10.0, RED)]), 80, 60, scale);

        assert_eq!(at(&frame, inside.0, inside.1), RED, "inside at scale {scale}");
        assert_eq!(at(&frame, outside.0, outside.1), WHITE, "outside at scale {scale}");
    }
}

#[test]
fn later_items_paint_over_earlier_ones() {
    let frame = render(&list([fill(0.0, 0.0, 20.0, 20.0, RED), fill(10.0, 0.0, 20.0, 20.0, BLUE)]), 30, 20, 1.0);

    assert_eq!(at(&frame, 5, 10), RED);
    assert_eq!(at(&frame, 15, 10), BLUE);
}

#[test]
fn a_translucent_fill_blends_with_what_is_under_it() {
    let frame = render(&list([fill(0.0, 0.0, 10.0, 10.0, Color::rgba(0, 0, 0, 128))]), 10, 10, 1.0);

    assert!(close(at(&frame, 5, 5), Color::rgb(127, 127, 127), 2));
}

#[test]
fn a_radius_rounds_the_corners() {
    let rounded = |radius| Item::Fill { rect: Rect::new(0.0, 0.0, 40.0, 40.0), color: RED, radius };
    // An oversized radius is clamped to a circle.
    for radius in [12.0, 500.0] {
        let frame = render(&list([rounded(radius)]), 40, 40, 1.0);

        assert_eq!(at(&frame, 20, 20), RED, "the centre, radius {radius}");
        assert_eq!(at(&frame, 0, 0), WHITE, "the corner, radius {radius}");
        assert_eq!(at(&frame, 20, 1), RED, "the middle of an edge, radius {radius}");
    }
}

#[test]
fn a_stroke_is_drawn_inside_the_edge() {
    for radius in [0.0, 6.0] {
        let stroke = Item::Stroke { rect: Rect::new(10.0, 10.0, 40.0, 30.0), color: BLUE, width: 3.0, radius };
        let frame = render(&list([stroke]), 60, 50, 1.0);

        assert_eq!(at(&frame, 30, 10), BLUE, "the outer edge, radius {radius}");
        assert_eq!(at(&frame, 30, 12), BLUE, "the inner edge, radius {radius}");
        assert_eq!(at(&frame, 30, 9), WHITE, "outside, radius {radius}");
        assert_eq!(at(&frame, 30, 13), WHITE, "the middle, radius {radius}");
    }
}

#[test]
fn a_stroke_wider_than_its_rectangle_fills_it() {
    let stroke = Item::Stroke { rect: Rect::new(10.0, 10.0, 20.0, 4.0), color: BLUE, width: 5.0, radius: 0.0 };
    let frame = render(&list([stroke]), 40, 24, 1.0);

    assert_eq!(at(&frame, 20, 12), BLUE);
    assert_eq!(at(&frame, 20, 15), WHITE);
}

#[test]
fn a_shadow_fades_out_and_follows_its_offset() {
    let shadow = |offset| Item::Shadow {
        rect: Rect::new(40.0, 40.0, 40.0, 40.0),
        color: BLACK,
        blur: 12.0,
        offset,
        radius: 4.0,
    };
    let centred = render(&list([shadow(Point::default())]), 120, 120, 1.0);
    let darkness = |frame: &Frame, x, y| ink(frame, x, y, 1, 1);

    assert!(darkness(&centred, 60, 60) > darkness(&centred, 60, 36), "darkest under the rectangle");
    assert!(darkness(&centred, 60, 36) > darkness(&centred, 60, 30), "fading with distance");
    assert!(darkness(&centred, 60, 36) > 0, "soft past the edge");
    assert_eq!(at(&centred, 60, 5), WHITE, "gone well past the blur");

    let lowered = render(&list([shadow(Point { x: 0.0, y: 8.0 })]), 120, 120, 1.0);
    assert!(darkness(&lowered, 60, 86) > darkness(&centred, 60, 86), "more shadow below");
    assert!(darkness(&lowered, 60, 34) < darkness(&centred, 60, 34), "less shadow above");
}

#[test]
fn a_shadow_with_no_blur_is_crisp() {
    let shadow = Item::Shadow {
        rect: Rect::new(10.0, 10.0, 20.0, 20.0),
        color: BLACK,
        blur: 0.0,
        offset: Point::default(),
        radius: 0.0,
    };
    let frame = render(&list([shadow]), 40, 40, 1.0);

    assert_eq!(at(&frame, 20, 20), BLACK);
    assert_eq!(at(&frame, 32, 20), WHITE);
}

#[test]
fn a_clip_limits_drawing_until_it_is_popped() {
    let frame = render(
        &list([
            Item::PushClip { rect: Rect::new(10.0, 10.0, 20.0, 20.0), radius: 0.0 },
            fill(0.0, 0.0, 60.0, 40.0, RED),
            Item::PopClip,
            fill(40.0, 0.0, 10.0, 10.0, BLUE),
        ]),
        60,
        40,
        1.0,
    );

    assert_eq!(at(&frame, 20, 20), RED, "inside the clip");
    assert_eq!(at(&frame, 5, 5), WHITE, "outside the clip");
    assert_eq!(at(&frame, 45, 5), BLUE, "after the pop");
}

#[test]
fn clips_nest_and_round() {
    let frame = render(
        &list([
            Item::PushClip { rect: Rect::new(0.0, 0.0, 40.0, 40.0), radius: 14.0 },
            Item::PushClip { rect: Rect::new(0.0, 0.0, 20.0, 40.0), radius: 0.0 },
            fill(0.0, 0.0, 40.0, 40.0, RED),
            Item::PopClip,
            Item::PopClip,
        ]),
        40,
        40,
        1.0,
    );

    assert_eq!(at(&frame, 10, 20), RED, "inside both");
    assert_eq!(at(&frame, 30, 20), WHITE, "outside the inner clip");
    assert_eq!(at(&frame, 1, 1), WHITE, "outside the rounded corner");
}

#[test]
fn unbalanced_clips_do_not_stop_the_frame() {
    let push = || Item::PushClip { rect: Rect::new(0.0, 0.0, 20.0, 20.0), radius: 0.0 };
    let cases: [(&str, Vec<Item>); 3] = [
        ("a pop with no push", vec![Item::PopClip, fill(0.0, 0.0, 20.0, 20.0, RED)]),
        ("a push with no pop", vec![push(), fill(0.0, 0.0, 20.0, 20.0, RED)]),
        ("more pops than pushes", vec![push(), Item::PopClip, Item::PopClip, fill(0.0, 0.0, 20.0, 20.0, RED)]),
    ];
    for (name, items) in cases {
        let frame = render(&list(items), 20, 20, 1.0);

        assert_eq!(at(&frame, 10, 10), RED, "{name}");
    }
}

#[test]
fn text_is_inked_in_its_colour_on_its_line() {
    // The baseline is at y = 40, so a 24 px line of capitals sits between about y = 20 and y = 40.
    let line = text(&sans(400.0), 24.0, RED, Point { x: 10.0, y: 40.0 }, "HELLO");
    let frame = render(&list([line]), 120, 60, 1.0);

    assert!(ink(&frame, 10, 20, 90, 21) > 0, "ink on the line");
    assert_eq!(ink(&frame, 0, 0, 120, 16), 0, "nothing above the line");
    assert_eq!(ink(&frame, 0, 44, 120, 16), 0, "nothing below the baseline");
    let coloured = |c: Color| c.r >= c.g && c.g == c.b;
    assert!((0..120).all(|x| (0..60).all(|y| coloured(at(&frame, x, y)))), "only red ink on white");
}

#[test]
fn text_scales_with_the_display() {
    let line = || list([text(&sans(400.0), 20.0, BLACK, Point { x: 5.0, y: 30.0 }, "Scale")]);
    let single = ink(&render(&line(), 80, 40, 1.0), 0, 0, 80, 40);
    let double = ink(&render(&line(), 160, 80, 2.0), 0, 0, 160, 80);

    let ratio = double as f64 / single as f64;
    assert!((3.5..4.5).contains(&ratio), "four times the ink at twice the scale, got {ratio}");
}

#[test]
fn weight_slant_and_synthetic_bold_change_the_glyphs() {
    let draw = |font: &Font, skew: f32, embolden: bool| {
        let Item::Glyphs { origin, font, size, color, glyphs, .. } =
            text(font, 28.0, BLACK, Point { x: 10.0, y: 40.0 }, "Illinois")
        else {
            unreachable!()
        };
        render(&list([Item::Glyphs { origin, font, size, color, skew, embolden, glyphs, text: None }]), 160, 60, 1.0)
    };
    let all = |frame: &Frame| ink(frame, 0, 0, 160, 60);
    let regular = draw(&sans(400.0), 0.0, false);

    assert!(all(&draw(&sans(700.0), 0.0, false)) > all(&regular) * 11 / 10, "a bold instance is heavier");
    assert!(all(&draw(&sans(400.0), 0.0, true)) > all(&regular) * 11 / 10, "synthetic bold is heavier");
    // The baseline is at y 40: a lean to the right carries the tops of the letters right and their feet not.
    let (upright, slanted) = (draw(&mono(), 0.0, false), draw(&mono(), 14.0, false));
    let middle = |frame: &Frame, rows: std::ops::Range<u32>| {
        let ink = |x| ink(frame, x, rows.start, 1, rows.end - rows.start) as f64;
        (0..160).map(|x| f64::from(x) * ink(x)).sum::<f64>() / (0..160).map(ink).sum::<f64>()
    };
    assert!(middle(&slanted, 15..25) > middle(&upright, 15..25) + 2.0, "a skew leans the tops to the right");
    assert!((middle(&slanted, 36..40) - middle(&upright, 36..40)).abs() < 1.0, "and leaves the feet in place");
}

#[test]
fn an_image_is_scaled_to_fill_its_rectangle() {
    let quadrants = image(8, 8, |x, y| [[RED, BLUE], [BLACK, Color::rgb(0, 200, 0)]][y as usize / 4][x as usize / 4]);
    let mut images = Images::default();
    images.insert("assets/quadrants", quadrants);
    let picture = Item::Image { rect: Rect::new(10.0, 10.0, 40.0, 20.0), asset: "assets/quadrants".into() };
    let frame = render_with(&list([picture]), &images, 60, 40, 1.0);

    assert_eq!(at(&frame, 20, 15), RED);
    assert_eq!(at(&frame, 40, 15), BLUE);
    assert_eq!(at(&frame, 20, 25), BLACK);
    assert_eq!(at(&frame, 40, 25), Color::rgb(0, 200, 0));
    assert_eq!(at(&frame, 5, 5), WHITE, "nothing outside the rectangle");
    assert_eq!(at(&frame, 51, 20), WHITE, "nothing outside the rectangle");
}

#[test]
fn a_translucent_image_blends_with_what_is_under_it() {
    let mut images = Images::default();
    images.insert("glass", image(4, 4, |_, _| Color::rgba(255, 0, 0, 128)));
    let picture = Item::Image { rect: Rect::new(0.0, 0.0, 20.0, 20.0), asset: "glass".into() };
    let frame = render_with(&list([picture]), &images, 20, 20, 1.0);

    assert!(close(at(&frame, 10, 10), Color::rgb(255, 127, 127), 2));
}

#[test]
fn an_image_that_is_not_decoded_yet_draws_nothing() {
    let picture = Item::Image { rect: Rect::new(0.0, 0.0, 20.0, 20.0), asset: "assets/missing".into() };
    let frame = render(&list([fill(0.0, 0.0, 20.0, 20.0, BLUE), picture]), 20, 20, 1.0);

    assert_eq!(at(&frame, 10, 10), BLUE);
}

#[test]
fn a_path_is_filled_or_stroked_at_its_origin_and_scale() {
    let p = |x, y| Point { x, y };
    // A square with a square hole, in a 10 unit box.
    let frame_shape: Arc<[PathEl]> = Arc::new([
        PathEl::MoveTo(p(0.0, 0.0)),
        PathEl::LineTo(p(10.0, 0.0)),
        PathEl::LineTo(p(10.0, 10.0)),
        PathEl::LineTo(p(0.0, 10.0)),
        PathEl::Close,
        PathEl::MoveTo(p(3.0, 3.0)),
        PathEl::LineTo(p(7.0, 3.0)),
        PathEl::LineTo(p(7.0, 7.0)),
        PathEl::LineTo(p(3.0, 7.0)),
        PathEl::Close,
    ]);
    let filled = Item::Path { origin: p(10.0, 10.0), scale: 4.0, path: frame_shape, color: RED, stroke: None };
    let frame = render(&list([filled]), 60, 60, 1.0);
    assert_eq!(at(&frame, 14, 14), RED, "the band");
    assert_eq!(at(&frame, 30, 30), WHITE, "the hole");
    assert_eq!(at(&frame, 52, 30), WHITE, "outside");

    let line: Arc<[PathEl]> = Arc::new([PathEl::MoveTo(p(2.0, 5.0)), PathEl::LineTo(p(8.0, 5.0))]);
    let stroked = Item::Path { origin: p(10.0, 10.0), scale: 4.0, path: line, color: BLUE, stroke: Some(2.0) };
    let frame = render(&list([stroked]), 60, 60, 1.0);
    assert_eq!(at(&frame, 30, 30), BLUE, "on the line");
    assert_eq!(at(&frame, 30, 33), BLUE, "half the scaled width from the line");
    assert_eq!(at(&frame, 30, 36), WHITE, "past the stroke");
    assert_eq!(at(&frame, 16, 30), BLUE, "the round cap");
    assert_eq!(at(&frame, 12, 30), WHITE, "past the cap");
}

#[test]
fn curves_are_drawn() {
    let p = |x, y| Point { x, y };
    // A circle of radius 10 from four cubic arcs.
    let k = 5.5228;
    let circle: Arc<[PathEl]> = Arc::new([
        PathEl::MoveTo(p(10.0, 0.0)),
        PathEl::CurveTo(p(10.0, k), p(k, 10.0), p(0.0, 10.0)),
        PathEl::CurveTo(p(-k, 10.0), p(-10.0, k), p(-10.0, 0.0)),
        PathEl::QuadTo(p(-10.0, -10.0), p(0.0, -10.0)),
        PathEl::QuadTo(p(10.0, -10.0), p(10.0, 0.0)),
        PathEl::Close,
    ]);
    let disc = Item::Path { origin: p(20.0, 20.0), scale: 1.0, path: circle, color: RED, stroke: None };
    let frame = render(&list([disc]), 40, 40, 1.0);

    assert_eq!(at(&frame, 20, 20), RED, "the centre");
    assert_eq!(at(&frame, 26, 26), RED, "inside the cubic arc");
    assert_eq!(at(&frame, 29, 29), WHITE, "outside the cubic arc");
    assert_eq!(at(&frame, 14, 14), RED, "inside the quadratic arc");
}

#[test]
fn items_that_cannot_be_drawn_are_skipped() {
    let mut images = Images::default();
    images.insert("pixel", image(1, 1, |_, _| BLACK));

    for (name, item) in undrawable() {
        // The item under test, then one that must still be drawn.
        let frame = render_with(&list([item, fill(50.0, 50.0, 10.0, 10.0, RED)]), &images, 64, 64, 1.0);

        assert_eq!(ink(&frame, 0, 0, 50, 50), 0, "{name} draws nothing");
        assert_eq!(at(&frame, 55, 55), RED, "{name} does not stop the frame");
    }
}

#[test]
fn an_empty_clip_hides_everything_inside_it() {
    let items = [
        Item::PushClip { rect: Rect::new(0.0, 0.0, 0.0, f32::NAN), radius: 0.0 },
        fill(0.0, 0.0, 20.0, 20.0, RED),
        Item::PopClip,
        fill(20.0, 0.0, 20.0, 20.0, BLUE),
    ];
    let frame = render(&list(items), 40, 20, 1.0);

    assert_eq!(at(&frame, 10, 10), WHITE);
    assert_eq!(at(&frame, 30, 10), BLUE);
}

#[test]
fn a_renderer_is_reused_across_frames_and_sizes() {
    let scene =
        list([fill(0.0, 0.0, 30.0, 30.0, BLUE), text(&sans(600.0), 18.0, WHITE, Point { x: 2.0, y: 20.0 }, "Aa")]);
    let mut renderer = CpuRenderer::new();
    let mut draw = |list: &DisplayList, width, height| {
        let mut frame = Frame::new(width, height);
        renderer.render(list, &Images::default(), 1.0, WHITE, &mut frame);
        frame
    };

    let first = draw(&scene, 40, 40);
    assert_eq!(draw(&list([]), 40, 40), Frame::clone(&render(&list([]), 40, 40, 1.0)), "nothing is left over");
    assert_eq!(draw(&scene, 40, 40), first, "the same list gives the same frame");
    assert_eq!(at(&draw(&scene, 90, 20), 80, 10), WHITE, "a frame of another size is drawn whole");
    assert_eq!(draw(&scene, 0, 0).data(), &[] as &[u8], "an empty frame is left alone");
}

#[test]
fn a_list_can_be_moved_after_it_is_emitted() {
    let mut scene = list([fill(0.0, 0.0, 10.0, 10.0, RED)]);
    let first = scene.items.len();
    scene.items.extend([
        Item::PushClip { rect: Rect::new(0.0, 0.0, 10.0, 10.0), radius: 0.0 },
        fill(0.0, 0.0, 30.0, 30.0, BLUE),
        Item::PopClip,
        text(&sans(700.0), 14.0, BLACK, Point { x: 14.0, y: 9.0 }, "I"),
    ]);
    scene.translate(first, Point { x: 30.0, y: 20.0 });
    let frame = render(&scene, 60, 40, 1.0);

    assert_eq!(at(&frame, 5, 5), RED, "items before the index stay put");
    assert_eq!(at(&frame, 35, 25), BLUE, "the clipped fill moved with its clip");
    assert_eq!(at(&frame, 45, 35), WHITE, "the clip still limits it");
    assert!(ink(&frame, 42, 18, 10, 12) > 0, "the text moved");
    assert_eq!(ink(&frame, 12, 0, 10, 10), 0, "nothing is left where the text was");
}

#[test]
fn a_list_can_be_made_smaller_after_it_is_emitted() {
    let mut scene = list([fill(0.0, 0.0, 10.0, 10.0, RED)]);
    let first = scene.items.len();
    scene.items.extend([
        Item::PushClip { rect: Rect::new(20.0, 20.0, 40.0, 40.0), radius: 0.0 },
        fill(20.0, 20.0, 80.0, 80.0, BLUE),
        Item::PopClip,
        Item::Stroke { rect: Rect::new(80.0, 0.0, 40.0, 40.0), color: BLACK, width: 8.0, radius: 0.0 },
        text(&sans(700.0), 40.0, BLACK, Point { x: 80.0, y: 100.0 }, "I"),
    ]);
    scene.scale(first, 0.5);
    let frame = render(&scene, 70, 60, 1.0);

    assert_eq!(at(&frame, 8, 8), RED, "items before the index keep their size");
    assert_eq!(at(&frame, 12, 12), BLUE, "the fill is half as far out");
    assert_eq!(at(&frame, 28, 28), BLUE, "and half as large, inside its clip");
    assert_eq!(at(&frame, 32, 32), WHITE, "which shrank with it");
    assert_eq!((at(&frame, 42, 10), at(&frame, 46, 10)), (BLACK, WHITE), "a stroke is half as wide");
    assert!(ink(&frame, 38, 30, 14, 22) > 0, "the text is where half the way leads");
    assert_eq!(ink(&frame, 38, 52, 30, 8), 0, "and half as tall");
}

#[test]
fn a_frame_converts_to_window_pixels() {
    let frame = render(&list([fill(1.0, 0.0, 1.0, 1.0, Color::rgb(0x12, 0x34, 0x56))]), 2, 1, 1.0);
    let mut window = [0xdead_beef_u32; 2];
    frame.copy_to_xrgb(&mut window);

    assert_eq!(window, [0x00ff_ffff, 0x0012_3456]);
}

#[test]
fn the_parts_of_a_document_change_nothing_on_screen() {
    let words = text(&sans(400.0), 16.0, BLACK, Point { x: 10.0, y: 30.0 }, "Parts");
    let plain = list([fill(0.0, 0.0, 30.0, 30.0, RED), words.clone()]);
    let mut marked = part(Role::Paragraph, 1, [fill(0.0, 0.0, 30.0, 30.0, RED), words]);
    marked.extend([Item::Close, Item::Open { role: Role::Heading(1), id: 2 }]);

    let (plain, marked) = (render(&plain, 80, 40, 1.0), render(&list(marked), 80, 40, 1.0));

    assert_eq!(furthest_apart(&plain, &marked), 0);
}

#[test]
fn a_colour_glyph_is_drawn_in_its_own_colours() {
    for (format, font, id) in colour_faces() {
        let plain = glyph(&font, id, 32.0, Point { x: 10.0, y: 50.0 });
        let Item::Glyphs { origin, font, size, glyphs, text, .. } = plain.clone() else { unreachable!() };
        let styled = Item::Glyphs { origin, font, size, color: BLACK, skew: 14.0, embolden: true, glyphs, text };

        let (plain, styled) = (render(&list([plain]), 70, 70, 1.0), render(&list([styled]), 70, 70, 1.0));

        let pixels = || (0..70).flat_map(|y| (0..70).map(move |x| (x, y)));
        let coloured = |frame: &Frame| {
            let spread = |color: Color| color.r.max(color.g).max(color.b) - color.r.min(color.g).min(color.b);
            pixels().filter(|(x, y)| frame.pixel(*x, *y).is_some_and(|color| spread(color) > 60)).count()
        };
        assert!(coloured(&plain) > 40, "{format} shows its colours, not the text's black");
        assert!(coloured(&styled) > 40, "{format} keeps its colours in bold and slanted text");
    }
}

#[test]
fn a_font_draws_from_bytes_that_something_else_keeps() {
    let owned = sans(400.0);
    let lent = lent(&owned);
    let line = |font: &Font| list([text(font, 16.0, BLACK, Point { x: 10.0, y: 30.0 }, "Mapped")]);

    let (from_owned, from_lent) = (render(&line(&owned), 100, 40, 1.0), render(&line(&lent), 100, 40, 1.0));

    assert!(ink(&from_lent, 0, 0, 100, 40) > 0, "the text is drawn");
    assert_eq!(furthest_apart(&from_owned, &from_lent), 0, "as it is from bytes the font owns");
    assert!(lent == lent.clone() && lent != owned, "fonts are told apart by where their bytes are");
}
