//! A formula whose engine the shell has yet to hand over, as a page does that fetches the engine when a
//! document first shows maths.
//!
//! The engine is built in unless letters-math has its `provided` feature, and then there is nothing to wait
//! for: `cargo test -p letters-layout --features letters-math/provided --test pending` runs this in earnest.

use std::fs;
use std::path::Path;

use letters_layout::{DocLayout, Palette};
use letters_math::{FONT_FILES, built_in, provide_engine, provide_fonts, take_pending};
use letters_model::{Document, Mark, Pos};
use letters_render::{Color, Item};

mod common;
use common::*;

const TEX: &str = r"\frac{a}{b}";

/// `TEX` in a line and as a block, under `mark` where it is in the line.
fn written(mark: Mark, typeset: bool) -> Document {
    let alone = if typeset { formula("f", TEX) } else { rich("f", &[(TEX, &[Mark::Code])]) };
    doc(vec![rich("a", &[("Mass, ", &[]), (TEX, &[mark]), (", and energy.", &[])]), alone, paragraph("z", "End")])
}

fn across(items: &[Item]) -> Vec<(u32, f32)> {
    placed(items).into_iter().map(|(glyph, x, _)| (glyph, x)).collect()
}

#[test]
fn a_formula_shows_as_its_source_until_the_engine_comes_and_is_set_once_it_does() {
    let style = letters_math::Style { size: 16.0, color: Color::rgb(0, 0, 0), display: false, links: false };
    if !letters_math::typeset(TEX, &style).is_err_and(|error| error.is_pending()) {
        return;
    }
    let end = |layout: &DocLayout| layout.caret(&Pos::new("z", 0)).unwrap().y;

    let mut layout = laid(&written(Mark::Math, true));
    let mut plain = laid(&doc(vec![paragraph("z", "End")]));
    let items = painted(&layout, everything());
    let code = painted(&laid(&written(Mark::Code, false)), everything());
    assert_eq!(across(&items), across(&code), "each reads as code does");
    assert!(inked(&items, Palette::LIGHT.warn.accent).is_empty(), "and warns of nothing: its source may be good");
    assert!(take_pending(), "the shell is told an engine is wanted");
    let before = end(&layout);

    let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../letters-math/fonts");
    provide_fonts(FONT_FILES.iter().map(|file| fs::read(fonts.join(file)).expect("a face")).collect());
    provide_engine(built_in());

    assert!(layout.maths_arrived(), "the blocks that waited are laid out again");
    assert!(!plain.maths_arrived(), "and no others");
    settled(&mut layout);
    assert_ne!(across(&painted(&layout, everything())), across(&code), "each formula is typeset now");
    assert!(end(&layout) > before, "and what is under them has made room");
    assert!(!layout.maths_arrived(), "once");
}
