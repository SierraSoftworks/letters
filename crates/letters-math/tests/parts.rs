//! A formula traced to its source: which bytes each part of it is set from, so that the term at the caret
//! can be lit.

mod common;

use std::ops::Range;

use common::*;
use letters_math::{Formula, commands, environments, typeset, typeset_mapped};
use letters_render::{Color, Item};

const LIT: Color = Color::rgb(200, 30, 60);

fn traced(source: &str) -> Formula {
    typeset_mapped(source, &inline(20.0)).unwrap_or_else(|error| panic!("{source}: {error}"))
}

/// The terms of a traced formula in the order they are written, each as its text in the source.
fn terms<'a>(source: &'a str, formula: &Formula) -> Vec<&'a str> {
    let mut sources: Vec<Range<usize>> = formula.parts.iter().map(|part| part.source.clone()).collect();
    sources.sort_by_key(|source| (source.start, source.end));
    sources.dedup();
    sources.into_iter().map(|bytes| &source[bytes]).collect()
}

/// What draws a formula, without where: each glyph by its number, and anything else as `None`.
fn drawn<'a>(items: impl IntoIterator<Item = &'a Item>) -> Vec<Option<u32>> {
    let shapes = |item: &Item| match item {
        Item::Glyphs { glyphs, .. } => glyphs.iter().map(|glyph| Some(glyph.id)).collect(),
        _ => vec![None],
    };
    let mut drawn: Vec<Option<u32>> = items.into_iter().flat_map(shapes).collect();
    drawn.sort();
    drawn
}

fn color(item: &Item) -> Option<Color> {
    match item {
        Item::Fill { color, .. } | Item::Glyphs { color, .. } | Item::Path { color, .. } => Some(*color),
        _ => None,
    }
}

#[test]
fn a_part_is_set_from_the_bytes_that_write_it() {
    let cases: [(&str, &[&str]); 22] = [
        ("a + b", &["a", "+", "b"]),
        ("x^2_i", &["x", "2", "i"]),
        (r"\alpha \times \beta", &[r"\alpha", r"\times", r"\beta"]),
        (r"\frac{a}{b} + \sqrt{c}", &["a", "b", "+", "c"]),
        (r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}", &["a", "b", "c", "d"]),
        // An operator is its command.
        (r"\sum_{i=0}^{n} i", &[r"\sum", "i", "=", "0", "n", "i"]),
        (r"\lim_{x \to 0} \sin x", &[r"\lim", "x", r"\to", "0", r"\sin", "x"]),
        (r"\left( \int f \right)", &[r"\int", "f"]),
        // So is what a macro of the engine's draws.
        (r"a \ne b \implies c \in \R", &["a", r"\ne", "b", r"\implies", "c", r"\in", r"\R"]),
        (r"1, \dots, n", &["1", ",", r"\dots", ",", "n"]),
        (r"\left( \ne \right)", &[r"\ne"]),
        // Not all is in the tree in the order it is written.
        (r"\N \sqrt[\R]{\ne} \Z", &[r"\N", r"\R", r"\ne", r"\Z"]),
        (r"\underset{x}{\max} \int_{a}^{\R} \ne", &["x", r"\max", r"\int", "a", r"\R", r"\ne"]),
        // A group holds what is written in it, whatever is written around it.
        (r"\left\lang {\ne} \right\rang", &[r"\ne"]),
        (r"\ne\boxed{a+b}", &[r"\ne", "a", "+", "b"]),
        // What a macro of the source's own makes is where that macro is written.
        (r"\def\x{a} \x + \x", &["a", "+"]),
        // What is set as one whole is one part, with its command.
        (r"\mathbb{R}^n", &[r"\mathbb{R}", "n"]),
        (r"\text{if } x \mathrm{d}t", &[r"\text{if }", "x", r"\mathrm{d}", "t"]),
        (r"\cancel{x} y", &[r"\cancel{x}", "y"]),
        (r"\frac{a}{\hbox{b}}", &["a", r"\hbox{b}"]),
        // Positions count bytes, as the document does, and a comment holds no term.
        ("é \\text{наука} % \\sum\n∞ \\sum", &["é", r"\text{наука}", "∞", r"\sum"]),
        // The engine places what its macros make in those macros, at bytes that are also in this source.
        (
            r"x_1 + x_2 + x_3 + x_4 \ne y \iff \R",
            &["x", "1", "+", "x", "2", "+", "x", "3", "+", "x", "4", r"\ne", "y", r"\iff", r"\R"],
        ),
    ];
    for (source, expected) in cases {
        for style in [inline(20.0), display(20.0)] {
            let formula = typeset_mapped(source, &style).unwrap_or_else(|error| panic!("{source}: {error}"));

            assert_eq!(terms(source, &formula), expected, "{source}");
        }
    }
}

#[test]
fn a_term_is_lit_as_what_it_draws_alone() {
    // (source, a term in it)
    let cases = [
        ("x + y", "y"),
        ("x^2", "2"),
        (r"\frac{a}{b}", "b"),
        (r"\sum_{i=0}^{n} i", r"\sum"),
        (r"a \ne b", r"\ne"),
        (r"\mathbb{R}^n", r"\mathbb{R}"),
        (r"\cancel{x} y", r"\cancel{x}"),
        // A comment holds no term.
        ("x % \\sum\n\\sum", r"\sum"),
    ];
    for (source, term) in cases {
        let formula = traced(source);
        let at = source.rfind(term).expect("the term is in the source");

        let lit = formula.tinted(&(at..at + term.len()), LIT);

        let (tinted, kept): (Vec<_>, Vec<_>) =
            lit.items.iter().zip(&formula.items).partition(|(item, _)| color(item) == Some(LIT));
        assert_eq!(drawn(tinted.iter().map(|(item, _)| *item)), drawn(&traced(term).items), "{term} in {source}");
        assert!(kept.iter().all(|(item, was)| item == was), "{source}: more than {term} changed");
        assert_eq!((lit.size, lit.baseline, lit.items.len()), (formula.size, formula.baseline, formula.items.len()));
    }
}

#[test]
fn no_term_is_lit_for_another() {
    // Sources whose terms are not in the tree in the order they are written, or are not in it at all.
    let sources = [
        r"X_\R^\N + \int_{\ne}^{\R}",
        r"x^{\frac{a}{b}}_{\sqrt{k}} + \sum_{\overline{k}}",
        r"\textstyle\sum\ne\R\implies\dots",
        r"\sqrt[\ne]{\R} + \sqrt{\ne} + \hat{\R}",
        r"\overset{\ne}{=} \underset{\R}{\N} \frac{\ne}{\R}",
        r"a~\ne~b \pmod{n} \bmod \not= \notin",
        r"\left\langle \ne \middle| \R \right\rangle",
        r"\begin{cases} \ne & \R \\ \sum & \dots \end{cases}",
        r"\text{a \dots b} \mathrm{\sin} \operatorname{\R}",
    ];
    for source in sources {
        let formula = traced(source);
        assert!(!formula.parts.is_empty(), "{source}");
        for term in formula.parts.iter().map(|part| &part.source) {
            let lit = formula.tinted(term, LIT);

            let tinted = lit.items.iter().filter(|item| color(item) == Some(LIT));
            assert_eq!(
                drawn(tinted),
                drawn(&traced(&source[term.clone()]).items),
                "{} in {source}",
                &source[term.clone()]
            );
        }
    }
}

#[test]
fn a_stretch_of_the_source_lights_every_term_in_it() {
    let source = r"a + \sum_{k} b";
    let formula = traced(source);

    let lit =
        |bytes: Range<usize>| formula.tinted(&bytes, LIT).items.iter().filter(|item| color(item) == Some(LIT)).count();

    assert_eq!(lit(0..source.len()), formula.items.len());
    assert_eq!(lit(0..5), 2, "a and +");
    // A term only part in the stretch is not lit, and neither is anything in a formula that is not traced.
    assert_eq!(lit(4..6), 0);
    let plain = set(source, &inline(20.0));
    assert_eq!(plain.tinted(&(0..source.len()), LIT), plain);
}

#[test]
fn the_caret_finds_the_term_it_is_at() {
    let source = r"ab + \sum_k \frac{c}{d}";
    let formula = traced(source);
    // (the caret, the term there): in a term or just after it, or else just before one.
    let cases = [
        (0, Some("a")),
        (1, Some("a")),
        (2, Some("b")),
        (3, Some("+")),
        (4, Some("+")),
        (5, Some(r"\sum")),
        (7, Some(r"\sum")),
        (9, Some(r"\sum")),
        (10, Some("k")),
        (11, Some("k")),
        // In a command that draws nothing of its own that is traced.
        (14, None),
        (19, Some("c")),
        (source.len(), None),
        (source.len() + 1, None),
    ];
    for (caret, expected) in cases {
        let term = formula.term(caret);

        assert_eq!(term.clone().map(|bytes| &source[bytes]), expected, "at {caret}");
        assert!(term.is_none_or(|bytes| bytes.start <= caret && caret <= bytes.end), "at {caret}");
    }
}

#[test]
fn a_traced_formula_draws_as_one_that_is_not() {
    /// A glyph as it is drawn: its number, size, colour and place.
    type Glyph = (u32, f32, Color, [f32; 2]);

    /// Every glyph where it is drawn, whatever run it is in, and everything else as it is.
    fn flat(formula: &Formula) -> (Vec<Glyph>, Vec<&Item>) {
        let (mut glyphs, mut others) = (Vec::new(), Vec::new());
        for item in &formula.items {
            match item {
                Item::Glyphs { origin, size, color, glyphs: run, .. } => {
                    glyphs.extend(
                        run.iter().map(|glyph| (glyph.id, *size, *color, [origin.x + glyph.x, origin.y + glyph.y])),
                    );
                }
                other => others.push(other),
            }
        }
        (glyphs, others)
    }
    let rows = commands().iter().chain(environments()).map(|row| row.sample);
    let others = [
        r"\sum_{i=0}^{n} x_i \ne \int_0^\infty e^{-x^2}\,dx",
        r"\left\{ \vec{v}_i \right\} \implies \R^n",
        r"\scriptstyle \hbox{text} \cancel{a}^{\mathrm{b}}",
        "",
    ];
    let mut seen = 0;

    for source in rows.chain(others) {
        for style in [inline(20.0), display(20.0)] {
            let plain = set(source, &style);

            let traced = typeset_mapped(source, &style).unwrap_or_else(|error| panic!("{source}: {error}"));

            assert_eq!((traced.size, traced.baseline), (plain.size, plain.baseline), "{source}");
            let ((glyphs, shapes), (plain_glyphs, plain_shapes)) = (flat(&traced), flat(&plain));
            assert_eq!(shapes, plain_shapes, "{source}");
            assert_eq!(glyphs.len(), plain_glyphs.len(), "{source}");
            for (glyph, plain) in glyphs.iter().zip(&plain_glyphs) {
                let near = glyph.3.iter().zip(plain.3).all(|(a, b)| (a - b).abs() < 0.001);
                assert!(
                    (glyph.0, glyph.1, glyph.2) == (plain.0, plain.1, plain.2) && near,
                    "{source}: {glyph:?} for {plain:?}"
                );
            }

            // Its parts are stretches of the source, each drawn by items of its own.
            assert!(plain.parts.is_empty(), "{source}");
            let mut next = 0;
            for part in &traced.parts {
                assert!(source.get(part.source.clone()).is_some_and(|text| !text.is_empty()), "{source}: {part:?}");
                assert!(next <= part.items.start && part.items.start < part.items.end, "{source}: {part:?}");
                next = part.items.end;
            }
            assert!(next <= traced.items.len(), "{source}");
            seen += usize::from(!traced.parts.is_empty());
        }
    }
    // Tracing gives up on a formula it would draw differently. That is not what happens to most.
    assert!(seen > 1500, "only {seen} formulas are traced");
}

#[test]
fn a_source_that_cannot_be_typeset_fails_the_same_traced() {
    for source in [r"\foo + 1", r"\frac{a}{b", "$x$"] {
        let style = inline(16.0);

        assert_eq!(typeset_mapped(source, &style).err(), typeset(source, &style).err(), "{source}");
        assert!(typeset(source, &style).is_err(), "{source}");
    }
}
