//! The list of figures under a reference that is being typed.

use letters_model::{Block, BlockId, BlockKind, Document, Mark};
use letters_test::{Harness, Role};

fn picture(id: &str, numbered: bool, caption: &str) -> Block {
    let kind = BlockKind::Figure {
        asset: Some(format!("assets/{id}.png")),
        src: None,
        alt: String::new(),
        width: 300.0,
        height: 180.0,
        numbered,
    };
    Block::text(id, kind, caption)
}

/// An empty paragraph `p` with the caret in it, over the pictures in `figures`: each an id and whether it
/// is numbered. A picture's caption is "The " and its id.
fn over(figures: &[(&str, bool)]) -> Harness {
    let mut blocks = vec![Block::text("p", BlockKind::Paragraph, "")];
    blocks.extend(figures.iter().map(|(id, numbered)| picture(id, *numbered, &format!("The {id}"))));
    Harness::open(Document { blocks, ..Document::default() })
}

/// A survey with three pictures, of which the roof is Figure 1 and the gutters are Figure 2.
fn survey() -> Harness {
    over(&[("roof", true), ("slate", false), ("gutters", true)])
}

/// The figures the list shows, top to bottom: each one's label, its caption, and whether it is picked.
fn listed(window: &mut Harness) -> Vec<(String, String, bool)> {
    let option = |found: letters_test::Found| {
        let picked = found.selected.filter(|_| found.role == Role::ListBoxOption)?;
        Some((found.name, found.description.unwrap_or_default(), picked))
    };
    window.controls().into_iter().filter_map(option).collect()
}

/// The figures that paragraph `p` refers to, in the order it does.
fn referred(window: &Harness) -> Vec<BlockId> {
    let block = window.editor().doc().block(&"p".into()).expect("the paragraph");
    let target = |mark: &Mark| if let Mark::Ref(to) = mark { Some(to.clone()) } else { None };
    block.body.runs().iter().filter_map(|run| run.marks.iter().find_map(target)).collect()
}

fn keys<'a>(window: &'a mut Harness, keys: &str) -> &'a mut Harness {
    keys.split(' ').filter(|key| !key.is_empty()).fold(window, |window, key| window.key(key))
}

#[test]
fn the_word_for_a_figure_lists_the_numbered_figures_with_none_picked() {
    let mut window = survey();
    window.type_text("See fig");
    window.assert_closed(Role::ListBox, "Figures");
    window.type_text("ure");
    window.assert_open(Role::ListBox, "Figures");
    let expected = [("Figure 1", "The roof", false), ("Figure 2", "The gutters", false)];
    let shown = listed(&mut window);
    let shown: Vec<_> =
        shown.iter().map(|(label, caption, picked)| (label.as_str(), caption.as_str(), *picked)).collect();
    assert_eq!(shown, expected, "a picture without a number is not listed");
}

#[test]
fn a_figure_is_taken_from_the_list_by_key_or_by_press() {
    // What is typed, how a figure is taken, and the figure the paragraph then refers to.
    let cases = [
        ("See Figure", "Down Enter", "roof"),
        ("See Figure", "Down Down Enter", "gutters"),
        ("See Figure", "Up Enter", "gutters"),
        ("See Figure", "Up Up Tab", "roof"),
        ("See Figure", "Down Down Down Enter", "roof"),
        ("See figure 2", "Enter", "gutters"),
        ("See Figure 2", "Up Enter", "gutters"),
        ("See Figure ", "a press", "gutters"),
    ];
    for (typed, take, figure) in cases {
        let mut window = survey();
        window.type_text(typed);
        match take {
            "a press" => window.click(Role::ListBoxOption, "Figure 2"),
            chords => keys(&mut window, chords),
        };
        assert_eq!(referred(&window), [BlockId::new(figure)], "{typed} then {take}");
        window.assert_closed(Role::ListBox, "Figures");
        let blocks = window.editor().doc().blocks.len();
        assert_eq!(blocks, 4, "{typed} then {take}: no line is broken");
    }
}

#[test]
fn enter_starts_a_new_paragraph_while_no_figure_is_picked() {
    // Neither a sentence that ends in the word nor a number that names no figure asks for a reference.
    for typed in ["As in the figure", "See Figure 7"] {
        let mut window = survey();
        window.type_text(typed).key("Enter");
        assert_eq!(referred(&window), [], "{typed}");
        assert_eq!(window.editor().doc().blocks.len(), 5, "{typed}");
    }
}

#[test]
fn a_typed_number_narrows_the_list_and_picks_its_first_figure() {
    let figures: Vec<String> = (1..=11).map(|number| format!("f{number}")).collect();
    let figures: Vec<(&str, bool)> = figures.iter().map(|id| (id.as_str(), true)).collect();
    let mut window = over(&figures);
    window.type_text("Figure 1");
    let shown = listed(&mut window);
    let labels: Vec<&str> = shown.iter().map(|figure| figure.0.as_str()).collect();
    assert_eq!(labels, ["Figure 1", "Figure 10", "Figure 11"]);
    assert!(shown[0].2 && !shown[1].2, "the figure the number names so far is picked: {shown:?}");

    // A walk of the list ends when another digit is typed.
    window.key("Down").type_text("1");
    assert_eq!(listed(&mut window), [("Figure 11".to_owned(), "The f11".to_owned(), true)]);
    window.type_text("1");
    window.assert_closed(Role::ListBox, "Figures");
}

#[test]
fn the_list_stays_away_where_the_words_are_only_words() {
    // Without a numbered figure there is nothing to refer to.
    let mut window = over(&[("slate", false)]);
    window.type_text("See Figure");
    window.assert_closed(Role::ListBox, "Figures");

    // Escape puts the list away and leaves the words, which are then text like any other.
    let mut window = survey();
    window.type_text("See Figure").key("Escape");
    window.assert_closed(Role::ListBox, "Figures");
    window.type_text(" 1");
    window.assert_closed(Role::ListBox, "Figures");
    window.key("Down").key("Enter");
    assert_eq!(referred(&window), [], "and the arrows and Enter are the page's again");

    // Words that go on are no reference.
    let mut window = survey();
    window.type_text("See Figure 1 and");
    window.assert_closed(Role::ListBox, "Figures");
}
