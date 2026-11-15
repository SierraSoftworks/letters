use letters::shadow::{CORNER, Piece, Shadow};

/// The alpha of the pixel of `piece` nearest to a point given from the window's corner, in its units.
fn alpha(piece: &Piece, scale: u32, (x, y): (i32, i32)) -> u32 {
    let (column, row) = ((x - piece.x) as u32 * scale, (y - piece.y) as u32 * scale);
    piece.pixels[(row * piece.width + column) as usize] >> 24
}

#[test]
fn a_shadow_lies_round_a_window_and_under_its_corners() {
    let (width, height) = (400_i32, 300_i32);
    for (scale, dark) in [(1, false), (2, false), (1, true)] {
        let shadow = Shadow::new(scale, dark);
        let [above, below, left, right] = shadow.pieces(width as u32, height as u32).expect("a window this large");
        for piece in [&above, &below, &left, &right] {
            assert_eq!(piece.pixels.len() as u32, piece.width * piece.height, "a piece holds all its pixels");
        }

        // Together the pieces surround the window, and reach under no more of it than its corners need.
        let reach = -above.y;
        assert!(reach > 0 && above.x == -reach && left.x == -reach && right.x == width);
        assert_eq!(above.width, (width + 2 * reach) as u32 * scale);
        assert_eq!((above.y + (above.height / scale) as i32, below.y), (CORNER as i32, height - CORNER as i32));
        assert_eq!((left.y, left.y + (left.height / scale) as i32), (CORNER as i32, height - CORNER as i32));
        assert_eq!(below.y + (below.height / scale) as i32, height + reach);

        // It is cast downwards, fades with distance, and is there under a corner the window leaves clear.
        let (by_bottom, by_top) = (alpha(&below, scale, (200, height + 2)), alpha(&above, scale, (200, -3)));
        assert!(by_bottom > by_top && by_top > 0, "{by_bottom} below and {by_top} above at scale {scale}");
        assert!(alpha(&below, scale, (200, height + reach - 1)) < by_bottom / 4, "it fades out below");
        assert!(alpha(&above, scale, (1, 1)) > 0, "under the top left corner");
        assert_eq!(alpha(&above, scale, (-reach, -reach)), 0, "nothing at its far corner");

        // An edge looks the same all along, and the two sides match to a step of rounding.
        assert_eq!(alpha(&below, scale, (120, height + 5)), alpha(&below, scale, (300, height + 5)));
        let sides = (alpha(&left, scale, (-4, 150)), alpha(&right, scale, (width + 3, 150)));
        assert!(sides.0 > 0 && sides.0.abs_diff(sides.1) <= 1, "{sides:?} at scale {scale}");
    }
}

#[test]
fn a_window_too_small_for_a_plain_edge_casts_no_shadow() {
    let shadow = Shadow::new(1, false);
    for (width, height) in [(40, 300), (400, 40), (0, 0)] {
        assert!(shadow.pieces(width, height).is_none(), "{width} by {height}");
    }
}
