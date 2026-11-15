//! What a typeset formula promises layout: a box, a baseline, ink where the box says, and a useful error.

mod common;

use common::*;
use letters_math::{Style, typeset, typeset_mapped};
use letters_render::{Color, DisplayList, Item, Point};

/// Sources that between them use glyphs, rules, stretched outlines and strokes.
const SOURCES: [&str; 8] = [
    "x",
    r"e^{i\pi} + 1 = 0",
    r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
    r"\sum_{i=0}^{n} i^2",
    r"\left( \begin{matrix} a & b \\ c & d \end{matrix} \right)",
    r"\overrightarrow{AB} + \underbrace{a + b}_{n}",
    r"\cancel{x} + \boxed{y}",
    r"\text{if } x \ge 0",
];

#[test]
fn a_formula_has_a_box_with_its_baseline_inside() {
    for source in SOURCES {
        for style in [inline(16.0), display(16.0)] {
            let formula = set(source, &style);

            assert!(formula.size.width > 0.0 && formula.size.height > 0.0, "{source} has no box");
            assert!((0.0..=formula.size.height).contains(&formula.baseline), "{source}: baseline outside the box");
        }
    }
}

#[test]
fn ink_stays_about_the_box() {
    // Ink may overhang, as an italic does, but not by more than a fraction of an em.
    let (size, slack) = (20.0, 5.0);
    for source in SOURCES {
        let formula = set(source, &display(size));
        let [left, top, right, bottom] =
            ink_bounds(&render(std::slice::from_ref(&formula), 1.0)).expect("the formula draws");

        let (far_right, far_bottom) = (MARGIN + formula.size.width + slack, MARGIN + formula.size.height + slack);
        assert!(left as f32 >= MARGIN - slack && top as f32 >= MARGIN - slack, "{source}: ink before the box");
        assert!(right as f32 <= far_right && bottom as f32 <= far_bottom, "{source}: ink past the box");
        // And the box is not much larger than what is drawn in it.
        assert!((right - left) as f32 >= formula.size.width * 0.6, "{source}: the box is wider than its ink");
    }
}

#[test]
fn letters_sit_on_the_baseline() {
    // (source, whether its ink goes below the baseline)
    for (source, descends) in [("x", false), ("y", true), (r"\frac{a}{b}", true), ("x^2", false)] {
        let formula = set(source, &inline(40.0));
        let [_, _, _, bottom] = ink_bounds(&render(std::slice::from_ref(&formula), 1.0)).expect("the formula draws");
        let below = bottom as f32 - (MARGIN + formula.baseline);

        assert_eq!(below > 2.0, descends, "{source}: ink ends {below} below the baseline");
        assert!(below > -2.0, "{source}: ink ends {below} below the baseline");
    }
}

#[test]
fn size_scales_the_whole_formula() {
    for source in SOURCES {
        let (small, large) = (set(source, &display(12.0)), set(source, &display(36.0)));

        for (small, large) in [
            (small.size.width, large.size.width),
            (small.size.height, large.size.height),
            (small.baseline, large.baseline),
        ] {
            assert!((large - small * 3.0).abs() <= large * 0.01, "{source}: {small} at 12 and {large} at 36");
        }
    }
}

#[test]
fn display_style_sets_fractions_and_limits_larger() {
    for source in [r"\frac{a}{b}", r"\sum_{i=0}^{n} i", r"\lim_{x \to 0} x"] {
        let (tight, open) = (set(source, &inline(16.0)), set(source, &display(16.0)));

        assert!(open.size.height > tight.size.height * 1.15, "{source}: display style is no taller");
    }
}

#[test]
fn the_style_colours_what_the_source_does_not() {
    let blue = Color::rgb(30, 60, 220);
    let style = Style { color: blue, ..inline(40.0) };
    // (source, the colour of its ink)
    for (source, ink) in [("x + 1", blue), (r"\frac{a}{b}", blue), (r"\textcolor{#dc1e1e}{x}", Color::rgb(220, 30, 30))]
    {
        let frame = render(&[set(source, &style)], 1.0);

        assert_eq!(darkest(&frame), ink, "{source}");
    }
}

#[test]
fn paint_places_the_box_at_the_origin() {
    let formula = set(r"\frac{a}{b} + \sqrt{x}", &display(20.0));
    let bounds = |origin: Point| {
        let mut list = DisplayList::default();
        formula.paint(origin, &mut list);
        let mut frame = letters_render::cpu::Frame::new(200, 200);
        letters_render::cpu::CpuRenderer::new().render(&list, &Default::default(), 1.0, WHITE, &mut frame);
        ink_bounds(&frame).expect("the formula draws")
    };

    let (near, far) = (bounds(Point { x: 10.0, y: 10.0 }), bounds(Point { x: 70.0, y: 95.0 }));

    assert_eq!(far, [near[0] + 60, near[1] + 85, near[2] + 60, near[3] + 85]);
}

#[test]
fn rules_stay_visible_at_small_sizes() {
    for size in [6.0, 9.0, 13.0] {
        let formula = set(r"\frac{a}{b} + \sqrt{x} + \overline{y}", &inline(size));
        let rule = |item: &Item| match item {
            Item::Fill { rect, .. } => Some(*rect),
            _ => None,
        };
        let rules: Vec<_> = formula.items.iter().filter_map(rule).collect();

        assert_eq!(rules.len(), 3, "a bar for each of the fraction, the root and the overline");
        assert!(rules.iter().all(|rule| rule.height >= 1.0), "a rule under a pixel thick at size {size}");
    }
}

#[test]
fn letters_the_maths_faces_lack_are_still_drawn() {
    let style = inline(20.0);
    let cyrillic = set(r"\text{шжмш}", &style);
    let [left, _, right, _] = ink_bounds(&render(std::slice::from_ref(&cyrillic), 1.0)).expect("Cyrillic draws");

    // The box is measured in the face that draws the letters, so it holds them and no more.
    let (ink, width) = ((right - left) as f32, cyrillic.size.width);
    assert!(right as f32 <= MARGIN + width + 1.5, "ink runs to {right}, past a box {width} wide");
    assert!(ink >= width * 0.8, "ink {ink} wide in a box {width} wide");
    // A script no bundled face has is laid out without its glyphs, not refused.
    assert!(typeset(r"x + \text{数学}", &style).is_ok());
}

#[test]
fn an_error_points_at_the_fault() {
    // (source, the text at fault, a word of the message)
    let cases = [
        (r"\foo + 1", r"\foo", r"\foo"),
        (r"1 + \frac{a}{b", "", "end"),
        ("x^", "^", "^"),
        ("a } b", "}", "}"),
        (r"\left( a", "", r"\right"),
        (r"\hspace{wide} x", "{wide}", "wide"),
        // Positions count bytes, as the document does.
        (r"é + \foo", r"\foo", r"\foo"),
        (r"\text{наука} \frac{a}", "", "end"),
        (r"\begin{matrix} a & b \\ c", "", "end"),
    ];
    for (source, fault, word) in cases {
        let Err(error) = typeset(source, &inline(16.0)) else { panic!("{source} was typeset") };
        let span = error.span.clone().unwrap_or_else(|| panic!("{source}: no position for {error}"));

        assert_eq!(source.get(span.clone()), Some(fault), "{source}: {error} at {span:?}");
        assert!(error.message.contains(word), "{source}: {error}");
        if fault.is_empty() {
            assert_eq!(span.start, source.len(), "{source}: something missing is reported at the end");
        }
    }
}

#[test]
fn an_error_without_a_position_still_says_what_is_wrong() {
    for source in [r"\begin{nonsense} a \end{nonsense}", r"\begin{matrix} a \end{pmatrix}", "$x$"] {
        let Err(error) = typeset(source, &inline(16.0)) else { panic!("{source} was typeset") };

        assert!(!error.message.is_empty(), "{source}");
        assert!(error.span.is_none_or(|span| source.get(span).is_some()), "{source}: a position outside the source");
    }
}

#[test]
fn an_error_in_a_macro_of_the_engines_is_not_placed_elsewhere() {
    // (source, the macro at fault). The engine places each in its macro, at bytes that are also in the source.
    let cases = [(r"\text{\R}", r"\R"), (r"a + b + c + d + e + f + g + h \text{\iff}", r"\iff")];
    for (source, fault) in cases {
        let Err(error) = typeset(source, &inline(16.0)) else { panic!("{source} was typeset") };

        let at = error.span.clone().map(|span| &source[span]);
        assert!(at.is_none_or(|at| at.contains(fault)), "{source}: {error} at {at:?}");
    }
}

#[test]
fn hostile_sources_fail_or_stay_drawable() {
    let nested = format!("{}x{}", "{".repeat(20_000), "}".repeat(20_000));
    let roots = format!("{}x{}", r"\sqrt{".repeat(2_000), "}".repeat(2_000));
    let sources = [
        r"\def\a{\a\a}\a",
        r"\def\a{\b}\def\b{\a}\a",
        r"\rule{1000000000em}{1000000000em}",
        r"\hspace{1e30em} x",
        r"\begin{array}{c} a \\ \hdashline b \end{array} \kern 100000000em",
        r"\raisebox{1e20em}{x} \scriptscriptstyle \Huge \Huge x^{x^{x^{x^{x}}}}",
        &nested,
        &roots,
    ];
    let set = sources.iter().flat_map(|source| [typeset, typeset_mapped].map(|set| set(source, &display(16.0))));
    // Either answer is fine. Neither may hang, crash or hand the renderer something it cannot draw.
    for formula in set.flatten() {
        assert!(formula.size.width.is_finite() && formula.size.height.is_finite() && formula.baseline.is_finite());
        let mut list = DisplayList::default();
        formula.paint(Point { x: 4.0, y: 4.0 }, &mut list);
        let mut frame = letters_render::cpu::Frame::new(64, 64);
        letters_render::cpu::CpuRenderer::new().render(&list, &Default::default(), 1.0, WHITE, &mut frame);
    }
}

#[test]
fn an_empty_source_is_an_empty_formula() {
    for source in ["", "  ", "{}"] {
        let formula = set(source, &inline(16.0));

        assert_eq!(formula.size.width, 0.0, "{source:?}");
        assert!(formula.items.is_empty(), "{source:?}");
    }
}
