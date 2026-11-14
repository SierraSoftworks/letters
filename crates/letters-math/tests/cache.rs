//! The cache hands back what it has already typeset, and stays within its size.

mod common;

use std::sync::Arc;

use common::*;
use letters_math::{Cache, Style, typeset};
use letters_render::Color;

#[test]
fn the_same_source_and_style_are_typeset_once() {
    let mut cache = Cache::default();

    let first = cache.typeset(r"\frac{a}{b}", &inline(16.0)).expect("a good source");
    let again = cache.typeset(r"\frac{a}{b}", &inline(16.0)).expect("a good source");

    assert!(Arc::ptr_eq(&first, &again));
    assert_eq!(*first, set(r"\frac{a}{b}", &inline(16.0)));
    assert_eq!(cache.len(), 1);
}

#[test]
fn each_part_of_the_style_is_part_of_the_key() {
    let base = inline(16.0);
    let others =
        [Style { size: 17.0, ..base }, Style { color: Color::rgb(200, 0, 0), ..base }, Style { display: true, ..base }];
    let mut cache = Cache::default();
    let source = r"\sum_{i=0}^{n} i";
    let plain = cache.typeset(source, &base).expect("a good source");

    for style in others {
        let styled = cache.typeset(source, &style).expect("a good source");

        assert_ne!(*styled, *plain, "{style:?} gave the formula of another style");
        assert_eq!(*styled, set(source, &style));
    }
    assert_eq!(cache.len(), 4);
}

#[test]
fn a_failure_is_kept_like_a_formula() {
    let mut cache = Cache::default();

    let first = cache.typeset(r"\frac{a}", &inline(16.0));
    let again = cache.typeset(r"\frac{a}", &inline(16.0));

    assert_eq!(first, typeset(r"\frac{a}", &inline(16.0)).map(Arc::new));
    assert!(first.is_err());
    assert_eq!(first, again);
    assert_eq!(cache.len(), 1);
}

#[test]
fn a_full_cache_drops_what_was_used_longest_ago() {
    let mut cache = Cache::new(4);
    let style = inline(16.0);
    let kept = cache.typeset("a", &style).expect("a good source");
    let dropped = cache.typeset("b", &style).expect("a good source");

    // Typing a formula: a new source at every keystroke, with "a" still on screen beside it.
    for step in 0..40 {
        cache.typeset(&format!("x + {step}"), &style).expect("a good source");
        cache.typeset("a", &style).expect("a good source");
    }

    assert!(cache.len() <= 4);
    assert!(Arc::ptr_eq(&kept, &cache.typeset("a", &style).expect("a good source")));
    assert!(!Arc::ptr_eq(&dropped, &cache.typeset("b", &style).expect("a good source")));
}

#[test]
fn clearing_forgets_everything() {
    let mut cache = Cache::default();
    let before = cache.typeset("x", &inline(16.0)).expect("a good source");

    cache.clear();

    assert!(cache.is_empty());
    assert!(!Arc::ptr_eq(&before, &cache.typeset("x", &inline(16.0)).expect("a good source")));
}
