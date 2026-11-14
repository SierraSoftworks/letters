//! The wavy lines under misspelled words.

use std::ops::Range;

use letters_layout::{DocLayout, Palette};
use letters_model::{Model, Op, Pos, ReplicaId, Style};
use letters_render::{Item, PathEl, Point};

mod common;
use common::*;

const NARROW: f32 = 320.0;

/// Where `word` sits in [`PROSE`].
fn word(word: &str) -> Range<usize> {
    let start = PROSE.find(word).expect("a word of the prose");
    start..start + word.len()
}

/// The wavy lines painted: where each starts, and how far it runs.
fn waves(items: &[Item]) -> Vec<(Point, f32)> {
    let wave = |item: &Item| match item {
        Item::Path { origin, path, color, stroke: Some(_), .. } if *color == Palette::LIGHT.spelling => {
            let ends = path.iter().filter_map(|el| if let PathEl::QuadTo(_, end) = el { Some(end.x) } else { None });
            Some((*origin, ends.fold(0.0, f32::max)))
        }
        _ => None,
    };
    items.iter().filter_map(wave).collect()
}

fn marked(ranges: Vec<Range<usize>>) -> DocLayout {
    let mut layout = laid_at(&prose(2), NARROW);
    layout.set_spelling(&"p0".into(), ranges);
    layout
}

#[test]
fn a_misspelled_word_gets_a_wavy_line_and_nothing_moves() {
    let plain = painted(&laid_at(&prose(2), NARROW), everything());
    let layout = marked(vec![word("household")]);
    let shown = painted(&layout, everything());
    let rest: Vec<Item> = shown.iter().filter(|item| !matches!(item, Item::Path { .. })).cloned().collect();
    assert!(rest == plain, "the text is painted as it was");

    let [(origin, run)] = waves(&shown)[..] else { panic!("one word, one line") };
    let caret = |offset| layout.caret(&Pos::new("p0", offset)).unwrap();
    let (from, to) = (caret(word("household").start), caret(word("household").end));
    assert!((origin.x - from.x).abs() < 1.0 && (origin.x + run - to.x).abs() < 1.0, "it runs under the word");
    assert!(origin.y > from.y + from.height / 2.0 && origin.y < from.y + from.height + 3.0, "below its letters");
}

#[test]
fn marks_show_wherever_their_words_are() {
    let (whole, empty) = (0..PROSE.len(), 5..5);
    let lines = (laid_at(&prose(1), NARROW).height() / 24.0).round() as usize;
    let cases: Vec<(&str, &str, Vec<Range<usize>>, usize)> = vec![
        ("two words", "p0", vec![word("Rates"), word("notice")], 2),
        ("out of order", "p0", vec![word("notice"), word("Rates")], 2),
        ("a range over every line", "p0", vec![whole], lines),
        ("an empty range", "p0", vec![empty], 0),
        ("none", "p0", vec![], 0),
        ("a block that is not there", "missing", vec![word("Rates")], 0),
    ];
    for (name, block, ranges, lines) in cases {
        let mut layout = marked(vec![word("first")]);
        layout.set_spelling(&"p0".into(), Vec::new());
        layout.set_spelling(&block.into(), ranges);
        assert_eq!(waves(&painted(&layout, everything())).len(), lines, "{name}");
    }
}

#[test]
fn marks_follow_the_text_through_edits() {
    let Range { start, end } = word("household");
    let cases = vec![
        (
            "typing before it",
            vec![Op::InsertText { at: Pos::new("p0", 0), text: "New ".into() }],
            Some(start + 4..end + 4),
        ),
        ("typing after it", vec![Op::InsertText { at: Pos::new("p0", end + 1), text: "x".into() }], Some(start..end)),
        ("typing at its end", vec![Op::InsertText { at: Pos::new("p0", end), text: "s".into() }], Some(start..end + 1)),
        ("deleting before it", vec![Op::DeleteText { block: "p0".into(), range: 0..6 }], Some(start - 6..end - 6)),
        ("deleting it", vec![Op::DeleteText { block: "p0".into(), range: start..end }], None),
        ("splitting the block before it", vec![Op::Split { at: Pos::new("p0", 10), new_id: "n".into() }], None),
    ];
    for (name, ops, expected) in cases {
        let mut model = Model::new(prose(2), ReplicaId::from(1));
        let mut layout = marked(vec![word("household")]);
        let change = model.apply(ops).unwrap();
        layout.update(model.doc(), &Style::default(), NARROW, Some(&change));

        let found = (0..=PROSE.len() + 8).find_map(|offset| layout.spelling_at(&Pos::new("p0", offset)));
        assert_eq!(found, expected, "{name}");
        assert_eq!(waves(&painted(&layout, everything())).len(), usize::from(expected.is_some()), "{name}");
    }
}

#[test]
fn the_word_at_a_position_is_found_from_either_end() {
    let layout = marked(vec![word("Rates"), word("household")]);
    let household = word("household");
    let cases = [
        ("before the word", "p0", household.start - 1, None),
        ("at its start", "p0", household.start, Some(household.clone())),
        ("inside it", "p0", household.start + 3, Some(household.clone())),
        ("at its end", "p0", household.end, Some(household.clone())),
        ("after it", "p0", household.end + 1, None),
        ("another word", "p0", 2, Some(word("Rates"))),
        ("another block", "p1", household.start + 3, None),
    ];
    for (name, block, offset, expected) in cases {
        assert_eq!(layout.spelling_at(&Pos::new(block, offset)), expected, "{name}");
    }
    let caret = layout.caret(&Pos::new("p0", household.start + 3)).unwrap();
    let hit = layout.hit(Point { x: caret.x, y: caret.y + caret.height / 2.0 }).unwrap();
    assert_eq!(layout.spelling_at(&hit), Some(household), "a click on the word finds it");
}

#[test]
fn a_word_being_composed_shows_no_mark() {
    let mut layout = marked(vec![word("Rates"), word("household")]);
    let household = word("household");
    layout.set_composition(Some((&"p0".into(), household.end..household.end + 2)));
    assert_eq!(waves(&painted(&layout, everything())).len(), 1, "the word the composition touches is left alone");
    layout.set_composition(None);
    assert_eq!(waves(&painted(&layout, everything())).len(), 2);
}
