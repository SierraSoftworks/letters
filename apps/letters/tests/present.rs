use letters::present::Painter;
use letters_render::image::Images;
use letters_render::{Color, DisplayList, Item, Rect};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;
const PAPER: Color = Color::rgb(255, 255, 255);

/// A frame with a mark at each of the given places.
fn list(marks: &[(f32, f32)]) -> DisplayList {
    let mark = |&(x, y)| Item::Fill { rect: Rect::new(x, y, 6.0, 4.0), color: Color::rgb(200, 30, 30), radius: 0.0 };
    DisplayList { items: marks.iter().map(mark).collect() }
}

fn paint(painter: &mut Painter, list: &DisplayList, age: u8, out: &mut [u32]) {
    painter.paint(list, &Images::default(), 1.0, PAPER, false, WIDTH, age, out);
}

/// What a painter with no history makes of a list.
fn fresh(list: &DisplayList) -> Vec<u32> {
    let mut out = blank();
    paint(&mut Painter::new(), list, 0, &mut out);
    out
}

fn blank() -> Vec<u32> {
    vec![0; (WIDTH * HEIGHT) as usize]
}

#[test]
fn a_buffer_of_any_age_ends_up_showing_the_whole_frame() {
    let frames = [list(&[(2.0, 2.0)]), list(&[(30.0, 20.0)]), list(&[(30.0, 20.0), (50.0, 40.0)]), list(&[])];
    assert!(fresh(&frames[0]) != fresh(&frames[1]), "the frames differ, so stale pixels would show");

    // One buffer shown again and again, as on X11.
    let (mut painter, mut buffer) = (Painter::new(), blank());
    for (n, frame) in frames.iter().enumerate() {
        paint(&mut painter, frame, n.min(1) as u8, &mut buffer);
        assert!(buffer == fresh(frame), "frame {n} in a buffer one frame old");
    }

    // Two buffers taking turns, as on Wayland.
    let (mut painter, mut buffers) = (Painter::new(), [blank(), blank()]);
    for (n, frame) in frames.iter().enumerate() {
        let age = if n < 2 { 0 } else { 2 };
        paint(&mut painter, frame, age, &mut buffers[n % 2]);
        assert!(buffers[n % 2] == fresh(frame), "frame {n} in a buffer two frames old");
    }

    // A buffer nothing is known about, as on macOS.
    let mut painter = Painter::new();
    for (n, frame) in frames.iter().enumerate() {
        let mut unknown = vec![0x00ab_cdef; (WIDTH * HEIGHT) as usize];
        paint(&mut painter, frame, 0, &mut unknown);
        assert!(unknown == fresh(frame), "frame {n} in a buffer of unknown age");
    }
}

#[test]
fn a_painter_follows_its_buffer_to_a_new_size() {
    let frame = list(&[(2.0, 2.0)]);
    let mut painter = Painter::new();
    paint(&mut painter, &frame, 0, &mut blank());

    // The window grew, and the system says the new buffer is one frame old.
    let wider = WIDTH + 16;
    let mut out = vec![0; (wider * HEIGHT) as usize];
    painter.paint(&frame, &Images::default(), 1.0, PAPER, false, wider, 1, &mut out);
    let mut expected = vec![0; (wider * HEIGHT) as usize];
    Painter::new().paint(&frame, &Images::default(), 1.0, PAPER, false, wider, 0, &mut expected);
    assert!(out == expected, "the frame at the new size is whole");
}

#[test]
fn a_window_with_alpha_is_clear_outside_its_round_corners() {
    let marks = [list(&[(0.0, 0.0)]), list(&[(30.0, 20.0)]), list(&[(0.0, 0.0), (58.0, 44.0)])];
    let (mut painter, mut out) = (Painter::new(), blank());
    let corners = [0, WIDTH - 1, (HEIGHT - 1) * WIDTH, HEIGHT * WIDTH - 1].map(|at| at as usize);
    // One buffer shown again and again, whether or not a frame changed its corners.
    for (n, frame) in marks.iter().enumerate() {
        painter.paint(frame, &Images::default(), 1.0, PAPER, true, WIDTH, n.min(1) as u8, &mut out);
        painter.round(10, &mut out);
        assert_eq!(corners.map(|at| out[at]), [0; 4], "frame {n} is clear at its corners");
        assert_eq!(out[(HEIGHT / 2 * WIDTH + WIDTH / 2) as usize], 0xffff_ffff, "and covered in the middle");
        // The corner's edge is soft, and is not rounded further by each frame.
        let row: Vec<u32> = (0..10).map(|x| out[(3 * WIDTH + x) as usize] >> 24).collect();
        assert!(row.iter().any(|alpha| (0x20..0xe0).contains(alpha)), "frame {n} has a soft edge in {row:x?}");
        assert!(row.is_sorted() && row[9] == 0xff, "frame {n} is whole inside its corner: {row:x?}");
    }

    // The same painter then serves a surface without alpha, which shows nothing of the old layout.
    painter.paint(&marks[1], &Images::default(), 1.0, PAPER, false, WIDTH, 1, &mut out);
    assert_eq!(out[0], 0x00ff_ffff);
}
