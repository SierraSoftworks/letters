//! A formula as SVG: what the renderer draws, with nothing in it that could load or run.

mod common;

use common::*;
use letters_math::{Formula, Style, commands, environments};
use letters_render::cpu::Frame;
use letters_render::{Color, Item, PathEl, Size};

/// An element of what `Formula::svg` writes: its name and its attributes.
type Element<'a> = (&'a str, Vec<(&'a str, &'a str)>);

type Point = (f32, f32);

/// Reads elements written one after another, each closed in place. `None` for anything else.
fn elements(svg: &str) -> Option<Vec<Element<'_>>> {
    let (mut found, mut rest) = (Vec::new(), svg);
    while !rest.is_empty() {
        let (element, after) = rest.strip_prefix('<')?.split_once("/>")?;
        let (name, mut attributes) = element.split_once(' ').unwrap_or((element, ""));
        let mut pairs = Vec::new();
        while !attributes.is_empty() {
            let (key, value) = attributes.split_once("=\"")?;
            let (value, after) = value.split_once('"')?;
            pairs.push((key.trim(), value));
            attributes = after;
        }
        found.push((name, pairs));
        rest = after;
    }
    Some(found)
}

fn attribute<'a>(element: &Element<'a>, name: &str) -> Option<&'a str> {
    element.1.iter().find(|(key, _)| *key == name).map(|(_, value)| *value)
}

fn length(element: &Element<'_>, name: &str) -> f32 {
    attribute(element, name).and_then(|value| value.parse().ok()).unwrap_or_default()
}

/// The colours an SVG paints in.
fn paints<'a>(elements: &[Element<'a>]) -> Vec<&'a str> {
    let paints = elements.iter().flat_map(|element| [attribute(element, "fill"), attribute(element, "stroke")]);
    paints.flatten().filter(|paint| *paint != "none").collect()
}

/// The outlines of a `path` or a `rect`, with each curve cut into short lines.
fn outlines(element: &Element<'_>) -> Vec<Vec<Point>> {
    if element.0 == "rect" {
        let (x, y) = (length(element, "x"), length(element, "y"));
        let (right, bottom) = (x + length(element, "width"), y + length(element, "height"));
        return vec![vec![(x, y), (right, y), (right, bottom), (x, bottom)]];
    }
    // De Casteljau: a point on a curve is found by cutting between its points until one is left.
    let on = |curve: &[Point], t: f32| {
        let mut points = curve.to_vec();
        while points.len() > 1 {
            let cut =
                |pair: &[Point]| (pair[0].0 + (pair[1].0 - pair[0].0) * t, pair[0].1 + (pair[1].1 - pair[0].1) * t);
            points = points.windows(2).map(cut).collect();
        }
        points[0]
    };
    let data = attribute(element, "d").unwrap_or_default();
    // Each command is a letter and then its numbers.
    let starts = data.char_indices().filter(|(_, letter)| letter.is_ascii_uppercase()).map(|(at, _)| at);
    let ends = starts.clone().skip(1).chain([data.len()]);
    let mut found: Vec<Vec<Point>> = Vec::new();
    for (start, end) in starts.zip(ends) {
        let (letter, numbers) = data[start..end].split_at(1);
        let numbers: Vec<f32> = numbers.split_whitespace().filter_map(|number| number.parse().ok()).collect();
        let mut points: Vec<Point> = numbers.as_chunks::<2>().0.iter().map(|&[x, y]| (x, y)).collect();
        match (letter, found.last_mut()) {
            ("M", _) => found.push(points),
            ("L" | "Q" | "C", Some(outline)) => {
                points.insert(0, outline[outline.len() - 1]);
                outline.extend((1..=8).map(|step| on(&points, step as f32 / 8.0)));
            }
            _ => {}
        }
    }
    found
}

/// The left, top, right and bottom of everything an SVG outlines.
fn bounds(elements: &[Element<'_>]) -> [f32; 4] {
    let points = elements.iter().flat_map(outlines).flatten();
    points.fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |[left, top, right, bottom], (x, y)| {
        [left.min(x), top.min(y), right.max(x), bottom.max(y)]
    })
}

/// The area an element inks, in square logical pixels: what its outlines enclose, or a stroke's length
/// times its width.
fn inked(element: &Element<'_>) -> f32 {
    let stroked = attribute(element, "stroke").is_some();
    let edges = |outline: Vec<Point>| {
        let ends = outline.iter().skip(1).chain(outline.first().filter(|_| !stroked)).copied().collect::<Vec<_>>();
        outline.into_iter().zip(ends)
    };
    let edges = outlines(element).into_iter().flat_map(edges);
    if stroked {
        return edges.map(|(a, b)| (b.0 - a.0).hypot(b.1 - a.1)).sum::<f32>() * length(element, "stroke-width");
    }
    edges.map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum::<f32>().abs() / 2.0
}

/// How much ink a frame holds, in square pixels of full ink.
fn ink(frame: &Frame) -> f32 {
    let pixels = (0..frame.height()).flat_map(|y| (0..frame.width()).map(move |x| (x, y)));
    let darkness = |pixel: Color| f32::from(WHITE.g - pixel.g) / f32::from(WHITE.g - INK.g);
    pixels.filter_map(|(x, y)| frame.pixel(x, y)).map(darkness).sum()
}

#[test]
fn a_formula_is_written_as_shapes_alone() {
    const SHAPES: [&str; 2] = ["path", "rect"];
    const ATTRIBUTES: [&str; 14] = [
        "d",
        "x",
        "y",
        "width",
        "height",
        "rx",
        "fill",
        "fill-opacity",
        "fill-rule",
        "stroke",
        "stroke-opacity",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
    ];
    // A document that may link is the most a formula can hold.
    let style = Style { links: true, ..inline(20.0) };
    let rows = commands().iter().chain(environments()).map(|row| row.sample);
    let others = [r"\href{https://example.com/page}{a}", r"\url{https://example.com/page}", r"\text{naïve привет}"];

    for source in rows.chain(others) {
        let svg = set(source, &style).svg(INK);

        let elements = elements(&svg).unwrap_or_else(|| panic!("{source} is not written as elements: {svg}"));
        for (name, attributes) in elements {
            assert!(SHAPES.contains(&name), "{source} writes a {name} element");
            for (key, value) in attributes {
                // No address, script or style can be spelt with these.
                let plain = value.chars().all(|c| c.is_ascii_alphanumeric() || " .-#".contains(c));
                assert!(ATTRIBUTES.contains(&key) && plain, "{source} writes {key}=\"{value}\" on a {name}");
            }
        }
    }
}

/// Shapes drawn in curves of both kinds, which no source sets alone: a circle and a lens.
fn curves() -> Formula {
    /// How far along its tangents the control points of a quarter circle lie.
    const ARC: f32 = 0.5523;
    let at = |x: f32, y: f32| letters_render::Point { x, y };
    let circle = [
        PathEl::MoveTo(at(1.0, 0.0)),
        PathEl::CurveTo(at(1.0, ARC), at(ARC, 1.0), at(0.0, 1.0)),
        PathEl::CurveTo(at(-ARC, 1.0), at(-1.0, ARC), at(-1.0, 0.0)),
        PathEl::CurveTo(at(-1.0, -ARC), at(-ARC, -1.0), at(0.0, -1.0)),
        PathEl::CurveTo(at(ARC, -1.0), at(1.0, -ARC), at(1.0, 0.0)),
        PathEl::Close,
    ];
    let lens = [
        PathEl::MoveTo(at(-1.0, 0.0)),
        PathEl::QuadTo(at(0.0, 1.5), at(1.0, 0.0)),
        PathEl::QuadTo(at(0.0, -1.5), at(-1.0, 0.0)),
        PathEl::Close,
    ];
    let shape = |x: f32, path: &[PathEl]| Item::Path {
        origin: at(x, 10.0),
        scale: 10.0,
        path: path.into(),
        color: INK,
        stroke: None,
    };
    Formula {
        size: Size { width: 50.0, height: 20.0 },
        baseline: 10.0,
        items: vec![shape(10.0, &circle), shape(38.0, &lens)],
    }
}

#[test]
fn the_drawing_is_the_renderers() {
    /// Physical pixels to a logical one, enough that the edges of the ink are sharp.
    const SCALE: f32 = 4.0;
    // Nothing in these overlaps, so the ink is the sum of the shapes.
    let sources = [
        r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
        r"\left\{ \sum_{i=0}^{n} \vec{v}_i \right\}",
        r"\int_0^\infty e^{-x^2}\,dx",
        r"\begin{array}{c|c} a & b \\ \hline c & d \end{array}",
        r"a_{\angl{n}} + \widetilde{abc}",
        r"\text{naïve привет}",
    ];
    let set =
        sources.iter().flat_map(|source| [inline(20.0), display(20.0)].map(|style| (*source, set(source, &style))));

    for (name, formula) in set.chain([("the curves", curves())]) {
        let frame = render(std::slice::from_ref(&formula), SCALE);
        let seen = ink_bounds(&frame).expect("the formula draws").map(|edge| edge as f32 / SCALE - MARGIN);

        let svg = formula.svg(INK);
        let elements = elements(&svg).expect("elements");

        for (drawn, seen) in bounds(&elements).into_iter().zip(seen) {
            assert!((drawn - seen).abs() < 0.75, "{name} is drawn to {drawn}, and the renderer inks to {seen}");
        }
        let (drawn, seen) = (elements.iter().map(inked).sum::<f32>(), ink(&frame) / (SCALE * SCALE));
        assert!((drawn / seen - 1.0).abs() < 0.03, "{name} holds {drawn} of ink, and the renderer lays {seen}");
    }
}

#[test]
fn ink_follows_the_page_and_other_colours_are_kept() {
    let hex = |color: Color| format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    let formula = set(r"x + \cancel{y} + \textcolor{#c2255c}{z} + \colorbox{yellow}{w}", &inline(20.0));

    let svg = formula.svg(INK);
    let inked = paints(&elements(&svg).expect("elements"));
    assert!(inked.contains(&"currentColor") && !inked.contains(&hex(INK).as_str()), "{inked:?}");
    assert!(inked.contains(&"#c2255c") && inked.contains(&"#ffff00"), "{inked:?}");

    // On a page whose text is another colour, the formula's own is kept.
    let svg = formula.svg(WHITE);
    let kept = paints(&elements(&svg).expect("elements"));
    assert!(kept.contains(&hex(INK).as_str()) && !kept.contains(&"currentColor"), "{kept:?}");
}

#[test]
fn a_colour_that_shows_what_is_under_it_stays_so() {
    let glass = Color::rgba(200, 0, 0, 128);
    let svg = set(r"x + \cancel{y}", &Style { color: glass, ..inline(20.0) }).svg(INK);

    let elements = elements(&svg).expect("elements");
    assert!(!elements.is_empty());
    for element in &elements {
        let (paint, opacity) = match attribute(element, "stroke") {
            Some(stroke) => (stroke, length(element, "stroke-opacity")),
            None => (attribute(element, "fill").unwrap_or_default(), length(element, "fill-opacity")),
        };
        assert_eq!(paint, "#c80000");
        assert!((opacity - 128.0 / 255.0).abs() < 0.01, "an opacity of {opacity}");
    }
}

#[test]
fn detail_is_kept_whatever_size_a_formula_is_set_at() {
    // Glyphs alone: a rule is held to a pixel, so it does not scale.
    let source = r"x^2 + y_i";
    let at = |size: f32| {
        let svg = set(source, &inline(size)).svg(INK);
        bounds(&elements(&svg).expect("elements")).map(|edge| edge / size)
    };

    let reference = at(1000.0);
    for size in [1.0, 12.0, 96.0] {
        for (edge, reference) in at(size).into_iter().zip(reference) {
            assert!((edge - reference).abs() < 0.002, "at {size} px an edge lies at {edge} em, not {reference}");
        }
    }
}
