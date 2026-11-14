//! Icons: SVG path data on a 24-unit grid, as yewi stores them, turned into outlines the renderer can draw.

use std::sync::Arc;

use letters_render::{PathEl, Point};

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
        parse(self.path).into()
    }
}

/// Reads SVG path data: every command of the `d` attribute, absolute and relative.
fn parse(data: &str) -> Vec<PathEl> {
    let mut out = Vec::new();
    let mut reader = Reader { bytes: data.as_bytes(), at: 0 };
    let (mut pen, mut start) = (Point::default(), Point::default());
    // The last control point, which the smooth commands mirror.
    let mut control: Option<Point> = None;
    let mut command = b'M';
    loop {
        match reader.command() {
            Some(next) => command = next,
            None if reader.done() => break,
            // Numbers with no letter repeat the last command, and a move repeats as lines.
            None => match command {
                b'M' => command = b'L',
                b'm' => command = b'l',
                b'Z' | b'z' => break,
                _ => {}
            },
        }
        let relative = command.is_ascii_lowercase();
        let origin = if relative { pen } else { Point::default() };
        let point = |reader: &mut Reader| {
            let (x, y) = (reader.number()?, reader.number()?);
            Some(Point { x: origin.x + x, y: origin.y + y })
        };
        let mut next_control = None;
        let step = match command.to_ascii_uppercase() {
            b'M' => point(&mut reader).map(|to| {
                start = to;
                (PathEl::MoveTo(to), to)
            }),
            b'L' => point(&mut reader).map(|to| (PathEl::LineTo(to), to)),
            b'H' => reader.number().map(|x| Point { x: origin.x + x, y: pen.y }).map(|to| (PathEl::LineTo(to), to)),
            b'V' => reader.number().map(|y| Point { x: pen.x, y: origin.y + y }).map(|to| (PathEl::LineTo(to), to)),
            b'C' => (|| {
                let (c1, c2, to) = (point(&mut reader)?, point(&mut reader)?, point(&mut reader)?);
                next_control = Some(c2);
                Some((PathEl::CurveTo(c1, c2, to), to))
            })(),
            b'S' => (|| {
                let (c2, to) = (point(&mut reader)?, point(&mut reader)?);
                let c1 = mirror(control, pen);
                next_control = Some(c2);
                Some((PathEl::CurveTo(c1, c2, to), to))
            })(),
            b'Q' => (|| {
                let (c, to) = (point(&mut reader)?, point(&mut reader)?);
                next_control = Some(c);
                Some((PathEl::QuadTo(c, to), to))
            })(),
            b'T' => point(&mut reader).map(|to| {
                let c = mirror(control, pen);
                next_control = Some(c);
                (PathEl::QuadTo(c, to), to)
            }),
            b'A' => (|| {
                let (rx, ry, turn) = (reader.number()?, reader.number()?, reader.number()?);
                let (large, sweep) = (reader.flag()?, reader.flag()?);
                let to = point(&mut reader)?;
                arc(pen, to, rx, ry, turn.to_radians(), large, sweep, &mut out);
                Some((PathEl::MoveTo(to), to))
            })(),
            b'Z' => {
                out.push(PathEl::Close);
                pen = start;
                control = None;
                continue;
            }
            _ => None,
        };
        let Some((el, to)) = step else { break };
        // An arc has already written its curves.
        if !command.eq_ignore_ascii_case(&b'A') {
            out.push(el);
        }
        pen = to;
        control = next_control;
    }
    out
}

fn mirror(control: Option<Point>, pen: Point) -> Point {
    control.map_or(pen, |c| Point { x: 2.0 * pen.x - c.x, y: 2.0 * pen.y - c.y })
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn skip(&mut self) {
        while self.bytes.get(self.at).is_some_and(|b| b.is_ascii_whitespace() || *b == b',') {
            self.at += 1;
        }
    }

    fn done(&mut self) -> bool {
        self.skip();
        self.at >= self.bytes.len()
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let byte = *self.bytes.get(self.at)?;
        (byte.is_ascii_alphabetic() && !matches!(byte, b'e' | b'E')).then(|| {
            self.at += 1;
            byte
        })
    }

    /// An arc's flag is one digit, and may run straight into the next number.
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let flag = match self.bytes.get(self.at)? {
            b'0' => false,
            b'1' => true,
            _ => return None,
        };
        self.at += 1;
        Some(flag)
    }

    fn number(&mut self) -> Option<f32> {
        self.skip();
        let start = self.at;
        let (mut seen_dot, mut seen_exp) = (false, false);
        while let Some(&byte) = self.bytes.get(self.at) {
            let first = self.at == start;
            let after_exp = !first && matches!(self.bytes[self.at - 1], b'e' | b'E');
            let part = match byte {
                b'0'..=b'9' => true,
                b'-' | b'+' => first || after_exp,
                b'.' => !seen_dot && !seen_exp,
                b'e' | b'E' => !first && !seen_exp,
                _ => false,
            };
            if !part {
                break;
            }
            seen_dot |= byte == b'.';
            seen_exp |= matches!(byte, b'e' | b'E');
            self.at += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.at]).ok()?.parse().ok()
    }
}

/// Writes an elliptical arc as cubic curves of at most a quarter turn each (SVG 1.1, appendix F.6).
#[allow(clippy::too_many_arguments)]
fn arc(from: Point, to: Point, rx: f32, ry: f32, turn: f32, large: bool, sweep: bool, out: &mut Vec<PathEl>) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || from == to {
        out.push(PathEl::LineTo(to));
        return;
    }
    let (sin, cos) = turn.sin_cos();
    let (dx, dy) = ((from.x - to.x) / 2.0, (from.y - to.y) / 2.0);
    let (x1, y1) = (cos * dx + sin * dy, -sin * dx + cos * dy);
    // Radii too small to span the gap are scaled up until they do.
    let reach = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if reach > 1.0 {
        rx *= reach.sqrt();
        ry *= reach.sqrt();
    }
    let span = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let scale = ((rx * rx * ry * ry - span) / span).max(0.0).sqrt() * if large == sweep { -1.0 } else { 1.0 };
    let (cx1, cy1) = (scale * rx * y1 / ry, -scale * ry * x1 / rx);
    let centre =
        Point { x: cos * cx1 - sin * cy1 + (from.x + to.x) / 2.0, y: sin * cx1 + cos * cy1 + (from.y + to.y) / 2.0 };

    let angle = |x: f32, y: f32| y.atan2(x);
    let start = angle((x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut sweep_angle = angle((-x1 - cx1) / rx, (-y1 - cy1) / ry) - start;
    let full = std::f32::consts::TAU;
    if sweep && sweep_angle < 0.0 {
        sweep_angle += full;
    } else if !sweep && sweep_angle > 0.0 {
        sweep_angle -= full;
    }

    let pieces = (sweep_angle.abs() / std::f32::consts::FRAC_PI_2).ceil().max(1.0);
    let step = sweep_angle / pieces;
    let handle = 4.0 / 3.0 * (step / 4.0).tan();
    let place =
        |x: f32, y: f32| Point { x: centre.x + rx * x * cos - ry * y * sin, y: centre.y + rx * x * sin + ry * y * cos };
    for piece in 0..pieces as usize {
        let (a, b) = (start + step * piece as f32, start + step * (piece + 1) as f32);
        let ((sa, ca), (sb, cb)) = (a.sin_cos(), b.sin_cos());
        let end = if piece + 1 == pieces as usize { to } else { place(cb, sb) };
        out.push(PathEl::CurveTo(
            place(ca - handle * sa, sa + handle * ca),
            place(cb + handle * sb, sb - handle * cb),
            end,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
