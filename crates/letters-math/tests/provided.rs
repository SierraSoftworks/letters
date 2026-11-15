//! A build whose shell hands the engine over: `cargo test -p letters-math --features provided --test provided`.
#![cfg(feature = "provided")]

use std::fs;
use std::path::Path;

use letters_math::{Cache, FONT_FILES, Style, built_in, mathml, provide_engine, provide_fonts, take_pending, typeset};
use letters_render::Color;

#[test]
fn a_formula_waits_for_the_engine_and_is_set_once_it_is_handed_over() {
    let style = Style { size: 16.0, color: Color::rgb(0, 0, 0), display: false, links: false };
    let mut cache = Cache::default();

    // Nothing says the source is wrong, and the shell is told that an engine is wanted, once.
    assert!(!take_pending(), "nothing was asked for yet");
    assert!(typeset(r"\frac{a}{b}", &style).is_err_and(|error| error.is_pending()));
    assert!(mathml("x", false).is_err_and(|error| error.is_pending()));
    assert!(cache.typeset(r"\frac{a}{b}", &style).is_err_and(|error| error.is_pending()));
    assert!(cache.is_empty(), "an answer that will change is not kept");
    assert!(take_pending());
    assert!(!take_pending());

    let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("fonts");
    provide_fonts(FONT_FILES.iter().map(|file| fs::read(fonts.join(file)).expect("a face")).collect());
    provide_engine(built_in());

    let formula = cache.typeset(r"\frac{a}{b}", &style).expect("a good source");
    assert!(formula.size.width > 0.0 && !formula.items.is_empty());
    assert!(mathml("x", false).is_ok_and(|written| written.contains("<mi>x</mi>")));
    // A source that is wrong is told so, and is not taken for one that waits.
    assert!(typeset(r"\frac{a}", &style).is_err_and(|error| !error.is_pending()));
    assert!(!take_pending());
}
