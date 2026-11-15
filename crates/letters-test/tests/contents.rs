//! Tables of contents on the page: what a press on one does, and how one is put in.

use accesskit::{Action, ActionRequest, TreeId};
use letters_model::{Block, BlockKind, Document};
use letters_test::{Found, Harness, Role};
use letters_ui::SaveState;

const OPTIONS: &str = "Table of contents options";
const BAR: &str = "Table of contents";

/// How tall the window of [`plan`] is.
const HEIGHT: f32 = 600.0;

/// A plan far taller than its window: a table of contents under the title, and a heading of each level
/// with a long paragraph under it.
fn plan() -> Harness {
    let filler = |id: &str| Block::text(id, BlockKind::Paragraph, "Estimates so far. ".repeat(80));
    let blocks = vec![
        Block::text("title", BlockKind::Heading { level: 1 }, "Cottage renovation"),
        Block::new("toc", BlockKind::TableOfContents { levels: 3 }),
        Block::text("budget", BlockKind::Heading { level: 2 }, "Budget"),
        filler("p1"),
        Block::text("roof", BlockKind::Heading { level: 3 }, "Roof"),
        filler("p2"),
        Block::text("slates", BlockKind::Heading { level: 4 }, "Slates"),
        filler("p3"),
        Block::text("schedule", BlockKind::Heading { level: 2 }, "Schedule"),
        filler("p4"),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.resize(900.0, HEIGHT);
    window.paint();
    window
}

/// The lines of the table that show, from the top.
fn lines(window: &mut Harness) -> Vec<Found> {
    let links = window.controls().into_iter().filter(|found| found.role == Role::Link);
    // The chapters control in the margin links to the headings too, from marks far shorter than a line.
    links.filter(|found| found.bounds.width > 200.0).collect()
}

/// The headings the table lists.
fn listed(window: &mut Harness) -> Vec<String> {
    lines(window).into_iter().map(|line| line.name).collect()
}

/// The block the caret is in, and how far into it.
fn caret(window: &Harness) -> Option<(&str, usize)> {
    window.editor().selection().map(|selection| (selection.head.block.as_str(), selection.head.offset))
}

/// The kinds of the document's blocks, in order.
fn kinds(window: &Harness) -> Vec<BlockKind> {
    window.editor().doc().blocks.iter().map(|block| block.kind.clone()).collect()
}

#[test]
fn a_press_on_a_line_goes_to_the_heading_it_lists() {
    assert_eq!(listed(&mut plan()), ["Budget", "Roof", "Schedule"], "the headings below it, down to Heading 3");
    // The line pressed, and the block the caret is then at the start of.
    for (heading, block) in [("Schedule", "schedule"), ("Roof", "roof"), ("Budget", "budget")] {
        let mut window = plan();
        let line = lines(&mut window).into_iter().find(|line| line.name == heading).expect("a line for the heading");
        window.click_at(line.center());
        assert_eq!(caret(&window), Some((block, 0)), "{heading}");
        window.paint();
        let shown = window.ui().caret_rect().expect("the heading is in view");
        assert!((0.0..HEIGHT / 2.0).contains(&shown.y), "{heading} is brought up the page: {shown:?}");
    }
}

#[test]
fn the_chip_of_a_table_sets_how_deep_it_lists() {
    // The level chosen, and the headings the table then lists.
    let cases: [(&str, &[&str]); 4] = [
        ("1", &[]),
        ("2", &["Budget", "Schedule"]),
        ("3", &["Budget", "Roof", "Schedule"]),
        ("4", &["Budget", "Roof", "Slates", "Schedule"]),
    ];
    for (level, lists) in cases {
        let mut window = plan();
        window.click(Role::Button, OPTIONS).assert_open(Role::Dialog, BAR);
        let chosen = |window: &mut Harness, level| window.find(Role::RadioButton, level)?.toggled;
        assert_eq!(chosen(&mut window, "3"), Some(true), "the bar says how deep it lists now");
        assert_eq!(chosen(&mut window, "4"), Some(false));
        assert_eq!(caret(&window), Some(("title", 0)), "the caret stays where it was");

        window.click(Role::RadioButton, level);
        assert_eq!(chosen(&mut window, level), Some(true), "{level}: the bar stays, for another to be tried");
        window.key("Escape").assert_closed(Role::Dialog, BAR);
        assert_eq!(listed(&mut window), lists, "{level}");
    }

    // A table with nothing to list keeps its chip, and what it listed comes back in one step.
    let mut window = plan();
    window.click(Role::Button, OPTIONS).click(Role::RadioButton, "1").key("Escape");
    window.click(Role::Button, OPTIONS).assert_open(Role::Dialog, BAR);
    window.key("Escape").key("Mod+Z");
    assert_eq!(listed(&mut window).len(), 3, "undo brings the lines back");
}

#[test]
fn a_table_of_contents_is_put_in_from_the_insert_menu_and_from_the_commands() {
    // How it is asked for on an empty line.
    type Ask = fn(&mut Harness);
    let ways: [(&str, Ask); 2] = [
        ("the insert menu", |window| _ = window.type_text("/contents").key("Enter")),
        ("the commands", |window| _ = window.key("Mod+Shift+P").type_text("Table of contents").key("Enter")),
    ];
    for (way, ask) in ways {
        let blocks = vec![Block::text("p", BlockKind::Paragraph, "")];
        let mut window = Harness::open(Document { blocks, ..Document::default() });
        window.ui().set_save_state(SaveState::Unsaved);
        ask(&mut window);
        window.type_text("Roof");
        let expected = [BlockKind::TableOfContents { levels: 3 }, BlockKind::Paragraph];
        assert_eq!(kinds(&window), expected, "{way}: the table takes the line's place, with a line to write on below");
        assert_eq!(window.markdown().matches("Roof").count(), 1, "{way}: and nothing of what asked for it stays");
    }
}

#[test]
fn assistive_technology_presses_the_lines_and_the_chip_of_a_table() {
    // What is pressed, by role and name.
    for (role, name) in [(Role::Link, "Schedule"), (Role::Button, OPTIONS)] {
        let mut window = plan();
        let tree = window.ui().access();
        let named = |node: &accesskit::Node| node.role() == role && node.label() == Some(name);
        // The chapters control links to the heading too, from a mark far shorter than a line.
        let wide = |node: &accesskit::Node| node.bounds().map_or(0.0, |bounds| bounds.width());
        let told = tree.nodes.iter().filter(|(_, node)| named(node));
        let (target_node, node) = told.max_by(|a, b| wide(&a.1).total_cmp(&wide(&b.1))).expect("it is told");
        assert!(node.supports_action(Action::Click), "{name} says that it can be pressed");
        let target_node = *target_node;
        window.ask(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None });
        match role {
            Role::Link => assert_eq!(caret(&window), Some(("schedule", 0))),
            _ => _ = window.assert_open(Role::Dialog, BAR),
        }
    }
}
