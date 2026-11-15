//! Icons: SVG path data on a 24-unit grid, as yewi stores them, turned into outlines the renderer can draw.

use std::sync::Arc;

use letters_render::PathEl;

#[rustfmt::skip]
mod set;
pub use set::*;

/// The side of the grid icons are drawn on.
pub const GRID: f32 = 24.0;

/// An icon: SVG path data on a 24x24 grid. It is drawn as a round-capped stroke, or filled when made with
/// [`Icon::filled`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Icon {
    path: &'static str,
    filled: bool,
}

impl Icon {
    /// An outline icon: the path is stroked.
    pub const fn new(path: &'static str) -> Self {
        Self { path, filled: false }
    }

    /// A solid glyph: the path is filled (even-odd) and not stroked.
    pub const fn filled(path: &'static str) -> Self {
        Self { path, filled: true }
    }

    pub const fn path(self) -> &'static str {
        self.path
    }

    pub const fn is_filled(self) -> bool {
        self.filled
    }

    /// The path as drawing steps, in grid units. Steps that cannot be read are left out.
    pub fn outline(self) -> Arc<[PathEl]> {
        letters_render::svg_path(self.path).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use letters_render::Point;

    fn end(el: &PathEl) -> Option<Point> {
        match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => Some(p),
            PathEl::Close => None,
        }
    }

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    #[test]
    fn lines_and_implicit_repeats() {
        // `check`: a move, an implicit absolute line, then a relative one.
        assert_eq!(
            *CHECK.outline(),
            [PathEl::MoveTo(p(20.0, 6.0)), PathEl::LineTo(p(9.0, 17.0)), PathEl::LineTo(p(4.0, 12.0))]
        );
    }

    #[test]
    fn horizontal_and_vertical_lines_keep_the_other_axis() {
        let outline = Icon::new("M5 12h14M12 5v14").outline();
        assert_eq!(outline[1], PathEl::LineTo(p(19.0, 12.0)));
        assert_eq!(outline[3], PathEl::LineTo(p(12.0, 19.0)));
    }

    #[test]
    fn arcs_become_curves_that_end_where_the_arc_ends() {
        // Packed flags: "a4 4 0 010 8" is large-arc 0, sweep 1, then 0 8.
        let outline = Icon::new("M15 12a4 4 0 010 8").outline();
        assert_eq!(outline.last().and_then(end), Some(p(15.0, 20.0)));
        assert!(outline[1..].iter().all(|el| matches!(el, PathEl::CurveTo(..))), "{outline:?}");
        // A half turn of radius 4 bulges 4 units to the right of its ends.
        let PathEl::CurveTo(_, _, mid) = outline[1] else { panic!("expected a curve") };
        assert!((mid.x - 19.0).abs() < 0.01 && (mid.y - 16.0).abs() < 0.01, "{mid:?}");
    }

    #[test]
    fn every_bundled_icon_parses_inside_the_grid() {
        for icon in [BOLD, CLOUD_CHECK, COPY, FILE_TEXT, HISTORY, LINK, PALETTE, SETTINGS, USERS, X] {
            let outline = icon.outline();
            assert!(matches!(outline.first(), Some(PathEl::MoveTo(_))), "{}", icon.path());
            let inside = |point: Point| (0.0..=GRID).contains(&point.x) && (0.0..=GRID).contains(&point.y);
            assert!(outline.iter().filter_map(end).all(inside), "{}: {outline:?}", icon.path());
        }
    }

    #[test]
    fn unreadable_data_stops_without_panicking() {
        assert_eq!(*Icon::new("M1 2L3").outline(), [PathEl::MoveTo(p(1.0, 2.0))]);
        assert!(Icon::new("").outline().is_empty());
    }
}
