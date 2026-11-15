//! A frame kept between renders is repainted only where its display list changed, and always ends up
//! showing what a fresh frame would. Pixels that only moved up or down are moved, not repainted: a fresh
//! render may round those differently, by one step at most.

mod common;

use common::*;
use letters_render::cpu::{CpuRenderer, Damage, Frame, PixelRect};
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, Point, Rect};

const WIDTH: u32 = 240;
const HEIGHT: u32 = 160;

/// A renderer and the frame it keeps.
struct Kept {
    renderer: CpuRenderer,
    frame: Frame,
    scale: f32,
}

impl Kept {
    fn new(scale: f32) -> Self {
        let size = |side: u32| (side as f32 * scale) as u32;
        Self { renderer: CpuRenderer::new(), frame: Frame::new(size(WIDTH), size(HEIGHT)), scale }
    }

    fn render(&mut self, list: &DisplayList, images: &Images) -> Damage {
        self.renderer.render(list, images, self.scale, WHITE, &mut self.frame)
    }

    /// The pixels a fresh renderer and frame give for the same list.
    fn fresh(&self, list: &DisplayList, images: &Images) -> Frame {
        render_with(list, images, self.frame.width(), self.frame.height(), self.scale)
    }

    fn share(&self, damage: &Damage) -> f64 {
        let repainted: u32 = damage.rects().iter().map(|rect| rect.width * rect.height).sum();
        f64::from(repainted) / f64::from(self.frame.width() * self.frame.height())
    }
}

fn whole(frame: &Frame) -> Vec<PixelRect> {
    vec![PixelRect { x: 0, y: 0, width: frame.width(), height: frame.height() }]
}

/// A page of text lines over a background, like an editor's window.
fn page(lines: &[&str]) -> Vec<Item> {
    let mut items = vec![fill(0.0, 0.0, WIDTH as f32, HEIGHT as f32, Color::rgb(250, 250, 252))];
    for (row, line) in lines.iter().enumerate() {
        items.push(text(&sans(400.0), 13.0, BLACK, Point { x: 12.0, y: 24.0 + 18.0 * row as f32 }, line));
    }
    items
}

const LINES: [&str; 7] = [
    "The first line of a page",
    "A second line, to be edited",
    "A third that stays as it is",
    "A fourth that stays as it is",
    "A fifth that stays as it is",
    "A sixth that stays as it is",
    "The last line of the page",
];

fn caret(x: f32, row: usize) -> Item {
    fill(x, 12.0 + 18.0 * row as f32, 1.5, 16.0, BLUE)
}

#[test]
fn a_change_repaints_only_its_own_part_of_the_frame() {
    let swatch = || image(8, 8, |x, y| Color::rgb(30 * x as u8, 30 * y as u8, 120));
    let picture = || Item::Image { rect: Rect::new(150.0, 100.0, 60.0, 40.0), asset: "swatch".into() };
    let shadow = |x, y| Item::Shadow {
        rect: Rect::new(x, y, 80.0, 30.0),
        color: Color::rgba(0, 0, 0, 120),
        blur: 10.0,
        offset: Point { x: 0.0, y: 3.0 },
        radius: 6.0,
    };
    let mut typed = LINES;
    typed[1] = "A second line, to be edited!";
    let with = |extra: Vec<Item>| [page(&LINES), extra].concat();
    let mut decoded = Images::default();
    decoded.insert("swatch", swatch());
    let mut replaced = Images::default();
    replaced.insert("swatch", image(8, 8, |_, _| RED));

    // What happened, then the list and images before it, then those after it.
    type Shown = (Vec<Item>, Images);
    let none = Images::default;
    let cases: Vec<(&str, Shown, Shown)> = vec![
        (
            "a keystroke",
            (with(vec![caret(170.0, 1)]), none()),
            ([page(&typed), vec![caret(176.0, 1)]].concat(), none()),
        ),
        ("a caret blinking off", (with(vec![caret(40.0, 3)]), none()), (page(&LINES), none())),
        (
            "a selection appearing under the text",
            (page(&LINES), none()),
            (
                [
                    vec![page(&LINES)[0].clone(), fill(12.0, 48.0, 90.0, 16.0, Color::rgb(180, 210, 255))],
                    page(&LINES)[1..].to_vec(),
                ]
                .concat(),
                none(),
            ),
        ),
        ("a line removed", (page(&LINES), none()), (page(&LINES[..6]), none())),
        ("a shadow moving", (with(vec![shadow(140.0, 60.0)]), none()), (with(vec![shadow(136.0, 70.0)]), none())),
        ("an image being decoded", (with(vec![picture()]), none()), (with(vec![picture()]), decoded.clone())),
        ("an image being replaced", (with(vec![picture()]), decoded.clone()), (with(vec![picture()]), replaced)),
        (
            "two overlapping items changing places",
            (with(vec![fill(100.0, 100.0, 40.0, 40.0, RED), fill(120.0, 110.0, 40.0, 40.0, BLUE)]), none()),
            (with(vec![fill(120.0, 110.0, 40.0, 40.0, BLUE), fill(100.0, 100.0, 40.0, 40.0, RED)]), none()),
        ),
    ];
    for scale in [1.0, 2.0] {
        for (name, (before, images_before), (after, images_after)) in &cases {
            let mut kept = Kept::new(scale);
            kept.render(&list(before.clone()), images_before);
            let after = list(after.clone());

            let damage = kept.render(&after, images_after);

            let fresh = kept.fresh(&after, images_after);
            assert!(kept.frame == fresh, "{name} at scale {scale} shows what a fresh render would");
            assert!(!damage.is_empty(), "{name} at scale {scale} repaints something");
            let share = kept.share(&damage);
            assert!(share < 0.25, "{name} at scale {scale} repainted {:.0}% of the frame", share * 100.0);
        }
    }
}

#[test]
fn an_unchanged_list_repaints_nothing() {
    let mut kept = Kept::new(2.0);
    let scene = list([page(&LINES), vec![caret(40.0, 2)]].concat());
    let first = kept.render(&scene, &Images::default());

    assert_eq!(first.rects(), whole(&kept.frame), "a new frame is painted whole");
    assert!(kept.render(&scene.clone(), &Images::default()).is_empty());
    assert!(kept.frame == kept.fresh(&scene, &Images::default()), "the frame is left as it was");
}

#[test]
fn what_cannot_be_patched_is_repainted_whole() {
    let scene = || list(page(&LINES));
    let clipped = |width| {
        list(
            [
                vec![Item::PushClip { rect: Rect::new(0.0, 0.0, width, 160.0), radius: 0.0 }],
                page(&LINES),
                vec![Item::PopClip],
            ]
            .concat(),
        )
    };
    let none = Images::default();

    let mut kept = Kept::new(1.0);
    kept.render(&clipped(100.0), &none);
    assert_eq!(kept.render(&clipped(200.0), &none).rects(), whole(&kept.frame), "a clip changed");
    assert!(kept.frame == kept.fresh(&clipped(200.0), &none), "the new clip shows");

    kept.render(&scene(), &none);
    let aside = list(page(&LINES).into_iter().map(|item| moved(item, 7.0, 0.0)).collect::<Vec<_>>());
    assert_eq!(kept.render(&aside, &none).rects(), whole(&kept.frame), "everything moved sideways");
    assert!(kept.frame == kept.fresh(&aside, &none), "the moved page shows");

    let mut other = Frame::new(WIDTH, HEIGHT);
    kept.render(&scene(), &none);
    let damage = kept.renderer.render(&scene(), &none, 1.0, WHITE, &mut other);
    assert_eq!(damage.rects(), whole(&other), "a frame the renderer did not draw last");
    assert!(other == kept.fresh(&scene(), &none), "the other frame shows the list");
    assert_eq!(kept.render(&scene(), &none).rects(), whole(&kept.frame), "its own frame, after drawing another");

    let damage = kept.renderer.render(&scene(), &none, 1.0, BLUE, &mut kept.frame);
    assert_eq!(damage.rects(), whole(&kept.frame), "another background");
    let damage = kept.renderer.render(&scene(), &none, 1.5, BLUE, &mut kept.frame);
    assert_eq!(damage.rects(), whole(&kept.frame), "another scale");
}

fn moved(item: Item, x: f32, y: f32) -> Item {
    let mut list = list([item]);
    list.translate(0, Point { x, y });
    list.items.remove(0)
}

/// A window around a scrolling page: a bar above it, the page's lines inside a clip, and a thumb and a
/// rounded badge over it. `scrolled` is how far up the lines have moved.
fn window(lines: &[&str], scrolled: f32, clip_radius: f32) -> Vec<Item> {
    let mut items = vec![
        fill(0.0, 0.0, WIDTH as f32, HEIGHT as f32, Color::rgb(244, 246, 249)),
        fill(0.0, 0.0, WIDTH as f32, 20.0, WHITE),
        text(&sans(600.0), 11.0, BLACK, Point { x: 8.0, y: 14.0 }, "A bar that stays"),
        Item::PushClip { rect: Rect::new(0.0, 20.0, WIDTH as f32, 140.0), radius: clip_radius },
        fill(30.0, 20.0, 180.0, 140.0, WHITE),
    ];
    for (row, line) in lines.iter().enumerate() {
        let y = 40.0 + 18.0 * row as f32 - scrolled;
        items.push(text(&sans(400.0), 13.0, BLACK, Point { x: 36.0, y }, line));
    }
    items.push(Item::PopClip);
    // The thumb follows the scroll by a fraction of a pixel, and the badge stays where it is.
    items.push(Item::Fill { rect: Rect::new(232.0, 24.0 + scrolled / 3.0, 5.0, 30.0), color: BLUE, radius: 2.5 });
    items.push(Item::Fill { rect: Rect::new(150.0, 120.0, 50.0, 18.0), color: RED, radius: 9.0 });
    items
}

#[test]
fn a_scroll_or_a_rewrap_keeps_the_frame_within_a_step_of_a_fresh_one() {
    const BOOK: [&str; 12] = [
        "Call me Ishmael. Some years",
        "ago, never mind how long",
        "precisely, having little or",
        "no money in my purse, and",
        "nothing particular to",
        "interest me on shore, I",
        "thought I would sail about",
        "a little and see the watery",
        "part of the world. It is a",
        "way I have of driving off",
        "the spleen and regulating",
        "the circulation.",
    ];
    let rewrapped: Vec<&str> = [&BOOK[..3], &["no money in my", "purse or pocket, and"], &BOOK[4..]].concat();
    // What happens, as the lines, scroll and clip radius of each frame in turn.
    type Shown<'a> = (&'a [&'a str], f32, f32);
    let cases: [(&str, &[Shown<'_>]); 5] = [
        ("scrolling down and back", &[(&BOOK, 0.0, 0.0), (&BOOK, 18.0, 0.0), (&BOOK, 58.0, 0.0), (&BOOK, 4.0, 0.0)]),
        ("scrolling by part of a pixel", &[(&BOOK, 0.0, 0.0), (&BOOK, 10.3, 0.0), (&BOOK, 20.6, 0.0)]),
        ("scrolling in a rounded clip", &[(&BOOK, 0.0, 12.0), (&BOOK, 36.0, 12.0), (&BOOK, 30.0, 12.0)]),
        ("a line wrapping in two", &[(&BOOK, 18.0, 0.0), (&rewrapped, 18.0, 0.0), (&BOOK, 18.0, 0.0)]),
        ("wrapping while scrolling", &[(&BOOK, 0.0, 0.0), (&rewrapped, 36.0, 0.0), (&BOOK, 16.0, 0.0)]),
    ];
    let none = Images::default();
    for scale in [1.0, 1.5, 2.0] {
        for (name, frames) in &cases {
            let mut kept = Kept::new(scale);
            let mut window_pixels = vec![0_u32; kept.frame.pixels().len()];
            for (step, (lines, scrolled, radius)) in frames.iter().enumerate() {
                let scene = list(window(lines, *scrolled, *radius));

                let damage = kept.render(&scene, &none);
                damage.rects().iter().for_each(|rect| kept.frame.copy_rect_to(&mut window_pixels, *rect));

                let apart = furthest_apart(&kept.frame, &kept.fresh(&scene, &none));
                assert!(apart <= 1, "{name} at scale {scale}: frame {step} is {apart} steps from a fresh one");
                let mut whole_frame = vec![0_u32; window_pixels.len()];
                kept.frame.copy_to_xrgb(&mut whole_frame);
                assert!(window_pixels == whole_frame, "{name} at scale {scale}: the damage of frame {step} covers it");
            }
        }
    }
}

#[test]
fn any_run_of_changes_leaves_the_frame_a_fresh_render_would() {
    let mut images = Images::default();
    images.insert("swatch", image(4, 4, |x, _| Color::rgb(60 * x as u8, 90, 200)));
    // Edits alone repaint exactly. Whole lists moving up and down move pixels, which may round a step off.
    for (name, scrolls, allowed) in [("edits", false, 0), ("edits and scrolls", true, 1)] {
        // A fixed sequence of pseudo-random numbers, so a failure can be replayed.
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut random = move |below: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % below as u64) as usize
        };
        let emoji = colour_faces();
        let make = |random: &mut dyn FnMut(usize) -> usize| {
            let (x, y) = (random(200) as f32, random(130) as f32);
            let color = Color::rgba(random(256) as u8, random(256) as u8, random(256) as u8, 120 + random(136) as u8);
            match random(7) {
                0 => fill(x, y, 10.0 + random(60) as f32, 6.0 + random(40) as f32, color),
                1 => Item::Stroke { rect: Rect::new(x, y, 50.0, 30.0), color, width: 2.0, radius: random(12) as f32 },
                2 => Item::Shadow {
                    rect: Rect::new(x, y, 40.0, 20.0),
                    color,
                    blur: random(14) as f32,
                    offset: Point { x: 2.0, y: 4.0 },
                    radius: 4.0,
                },
                3 => text(
                    &sans(300.0 + 100.0 * random(5) as f32),
                    10.0 + random(14) as f32,
                    color,
                    Point { x, y: y + 20.0 },
                    "Wavy fjord",
                ),
                4 => Item::Image { rect: Rect::new(x, y, 30.0, 24.0), asset: "swatch".into() },
                5 => {
                    let (_, font, id) = &emoji[random(emoji.len())];
                    glyph(font, *id, 12.0 + random(30) as f32, Point { x, y: y + 20.0 })
                }
                _ => Item::Fill { rect: Rect::new(x, y, 36.0, 36.0), color, radius: 18.0 },
            }
        };
        // Three layers: under a clipped page, on it, and over it. A scroll moves the page's items together.
        let mut layers: [Vec<Item>; 3] = std::array::from_fn(|_| (0..10).map(|_| make(&mut random)).collect());
        let clip = |radius| Item::PushClip { rect: Rect::new(20.0, 10.0, 200.0, 130.0), radius };
        let scene = |layers: &[Vec<Item>; 3], radius: f32| {
            let [under, page, over] = layers.clone();
            list([under, vec![clip(radius)], page, vec![Item::PopClip], over].concat())
        };

        let mut kept = Kept::new(1.5);
        kept.render(&scene(&layers, 0.0), &images);
        for step in 0..40 {
            for _ in 0..1 + random(3) {
                let layer = &mut layers[random(3)];
                match random(3) {
                    0 => layer.insert(random(layer.len() + 1), make(&mut random)),
                    1 if !layer.is_empty() => drop(layer.remove(random(layer.len()))),
                    _ if !layer.is_empty() => {
                        // Never straight up or down, which is the move a scroll makes.
                        let (at, x) = (random(layer.len()), [-3.0, -1.0, 2.0, 4.0][random(4)]);
                        layer[at] = moved(layer[at].clone(), x, random(9) as f32 - 4.0);
                    }
                    _ => {}
                }
            }
            if scrolls && random(2) == 0 {
                // Whole pixels at this scale, either way.
                let rows = [-20.0, -8.0, -2.0, 2.0, 4.0, 14.0][random(6)];
                layers[1] = layers[1].drain(..).map(|item| moved(item, 0.0, rows)).collect();
            }
            let scene = scene(&layers, if step < 20 { 0.0 } else { 8.0 });

            kept.render(&scene, &images);

            let apart = furthest_apart(&kept.frame, &kept.fresh(&scene, &images));
            assert!(apart <= allowed, "{name}: after step {step} the frame is {apart} steps from a fresh one");
        }
    }
}

#[test]
fn a_window_frame_holds_the_same_picture_in_the_windows_layout() {
    let swatch = |x| fill(x, 100.0, 40.0, 30.0, Color::rgb(0x12, 0x34, 0x56));
    let scenes = [150.0, 160.0].map(|x| list([page(&LINES), vec![swatch(x)]].concat()));
    let none = Images::default();
    let mut renderer = CpuRenderer::new();
    let mut frame = Frame::xrgb(WIDTH, HEIGHT);
    // A window's buffer, kept up to date from the damage alone.
    let mut window = vec![0_u32; (WIDTH * HEIGHT) as usize];

    for scene in &scenes {
        let damage = renderer.render(scene, &none, 1.0, WHITE, &mut frame);
        damage.rects().iter().for_each(|rect| frame.copy_rect_to(&mut window, *rect));

        let mut fresh = Frame::xrgb(WIDTH, HEIGHT);
        CpuRenderer::new().render(scene, &none, 1.0, WHITE, &mut fresh);
        assert!(frame == fresh, "a kept frame shows the new list");
        assert!(window == frame.pixels(), "the damage brings a window up to date");
        let mut converted = vec![0_u32; window.len()];
        render(scene, WIDTH, HEIGHT, 1.0).copy_to_xrgb(&mut converted);
        assert!(fresh.pixels() == converted, "the frame holds what converting an RGBA frame gives");
    }
    assert_eq!(frame.pixel(170, 120), Some(Color::rgb(0x12, 0x34, 0x56)));
    assert_eq!(frame.pixels()[(120 * WIDTH + 170) as usize], 0x0012_3456);
}

#[test]
fn a_window_frame_with_alpha_is_clear_where_nothing_is_drawn() {
    let swatch = |x| fill(x, 100.0, 40.0, 30.0, Color::rgb(0x12, 0x34, 0x56));
    let scenes = [150.0, 160.0].map(|x| list([swatch(x)]));
    let (none, clear) = (Images::default(), Color::rgba(0, 0, 0, 0));
    let mut renderer = CpuRenderer::new();
    let mut frame = Frame::argb(WIDTH, HEIGHT);

    for scene in &scenes {
        renderer.render(scene, &none, 1.0, clear, &mut frame);
        let mut fresh = Frame::argb(WIDTH, HEIGHT);
        CpuRenderer::new().render(scene, &none, 1.0, clear, &mut fresh);
        assert!(frame == fresh, "a kept frame shows the new list");
    }
    assert_eq!(frame.pixels()[(120 * WIDTH + 170) as usize], 0xff12_3456);
    assert_eq!(frame.pixel(170, 120), Some(Color::rgb(0x12, 0x34, 0x56)));
    assert_eq!(frame.pixels()[(120 * WIDTH + 155) as usize], 0, "where the swatch was is clear again");
}

#[test]
fn damage_from_two_renders_can_be_joined() {
    let mut kept = Kept::new(1.0);
    let none = Images::default();
    let scenes =
        [caret(40.0, 1), caret(46.0, 1), caret(46.0, 5)].map(|caret| list([page(&LINES), vec![caret]].concat()));
    kept.render(&scenes[0], &none);
    let first = kept.render(&scenes[1], &none);
    let second = kept.render(&scenes[2], &none);

    let both = first.union(&second);

    let covers = |damage: &Damage, x: u32, y: u32| {
        damage.rects().iter().any(|r| (r.x..r.x + r.width).contains(&x) && (r.y..r.y + r.height).contains(&y))
    };
    assert!(covers(&both, 41, 36) && covers(&both, 47, 36) && covers(&both, 47, 108), "every changed pixel");
    assert!(!covers(&both, 200, 150), "and not the rest of the frame");
    assert!(kept.share(&both) < 0.1);
}
