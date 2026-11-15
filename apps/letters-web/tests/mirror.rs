//! The window's accessibility tree as the elements of the mirror.

use std::collections::HashSet;

use letters_editor::Editor;
use letters_model::{Block, BlockId, BlockKind, Document, ListKind, ReplicaId};
use letters_render::Size;
use letters_ui::Window;
use letters_web::mirror::{Act, Item, Mirror};

/// A window on a document with one block of each of `kinds`, each holding its own name as text.
fn window(kinds: &[(&str, BlockKind)]) -> Window {
    let blocks = kinds.iter().map(|(name, kind)| Block::text(BlockId::new(*name), kind.clone(), *name)).collect();
    let editor = Editor::new(Document { blocks, ..Document::default() }, ReplicaId::new("test"));
    let mut window = Window::new(editor);
    window.resize(Size { width: 1000.0, height: 700.0 }, 1.0);
    window
}

fn mirror(window: &mut Window) -> Mirror {
    Mirror::of(&window.access())
}

/// The block that reads as `text`. The bar may show the same words in a label, which is a `span`.
fn reading<'a>(mirror: &'a Mirror, text: &str) -> &'a Item {
    let block = mirror.items.values().find(|item| item.text == text && item.tag != "span");
    block.unwrap_or_else(|| panic!("no block reads {text:?}"))
}

fn attribute<'a>(item: &'a Item, name: &str) -> Option<&'a str> {
    item.attributes.iter().find(|(known, _)| *known == name).map(|(_, value)| value.as_str())
}

#[test]
fn each_block_is_the_element_a_reader_expects() {
    let cases = [
        ("a paragraph", BlockKind::Paragraph, "p", None),
        ("a title", BlockKind::Heading { level: 1 }, "h1", None),
        ("a section", BlockKind::Heading { level: 3 }, "h3", None),
        ("some code", BlockKind::Code { language: None }, "pre", None),
        ("an item", BlockKind::ListItem { list: ListKind::Bullet, depth: 0 }, "div", Some("listitem")),
    ];
    let kinds: Vec<_> = cases.iter().map(|(name, kind, ..)| (*name, kind.clone())).collect();
    let mirror = mirror(&mut window(&kinds));

    for (name, _, tag, role) in cases {
        let item = reading(&mirror, name);
        assert_eq!((item.tag, attribute(item, "role")), (tag, role), "{name}");
        assert!(matches!(item.act, Some(Act::Caret { .. })), "{name}: activating it puts the caret in it");
    }
}

#[test]
fn the_mirror_is_one_tree_in_the_documents_order() {
    let kinds = [("first", BlockKind::Paragraph), ("second", BlockKind::Paragraph), ("third", BlockKind::Paragraph)];
    let mirror = mirror(&mut window(&kinds));

    // Every element is reached once from the root, in the order a reader meets it.
    let (mut order, mut next) = (Vec::new(), vec![mirror.root]);
    while let Some(id) = next.pop() {
        let item = mirror.items.get(&id).expect("a child that is listed exists");
        order.push(id);
        next.extend(item.children.iter().rev());
    }
    assert_eq!(order.iter().collect::<HashSet<_>>().len(), order.len(), "no element has two parents");
    assert_eq!(order.len(), mirror.items.len(), "nothing is left outside the tree");

    let read: Vec<&str> =
        order.iter().map(|id| mirror.items[id].text.as_str()).filter(|text| !text.is_empty()).collect();
    let blocks: Vec<&str> = read.into_iter().filter(|text| kinds.iter().any(|(name, _)| name == text)).collect();
    assert_eq!(blocks, ["first", "second", "third"]);
}

#[test]
fn controls_are_named_can_be_pressed_and_stay_out_of_the_tab_order() {
    let mirror = mirror(&mut window(&[("text", BlockKind::Paragraph)]));
    let buttons: Vec<&Item> = mirror.items.values().filter(|item| item.tag == "button").collect();

    assert!(buttons.len() > 3, "the bar's buttons are in the mirror");
    for button in buttons {
        let name = attribute(button, "aria-label").filter(|name| !name.is_empty());
        assert!(name.is_some() || !button.children.is_empty(), "a button has a name: {button:?}");
        assert_eq!(button.act, Some(Act::Click), "{name:?}");
        assert_eq!(attribute(button, "tabindex"), Some("-1"), "{name:?}: Tab stays with the hidden input");
        assert!(attribute(button, "style").is_some_and(|style| style.contains("left:")), "{name:?} says where it is");
    }
}

#[test]
fn the_keyboard_is_reported_only_while_a_control_has_it() {
    let mut window = window(&[("text", BlockKind::Paragraph)]);
    window.focus(true);
    assert_eq!(mirror(&mut window).focus, None, "the hidden input speaks for the document");
}

#[test]
fn element_names_round_trip() {
    for node in [0, 1, 6_585_860_780_055_086_444, u64::MAX] {
        assert_eq!(Mirror::node(&Mirror::name(node)), Some(node));
    }
    for other in ["", "letters", "letters-node-", "letters-node-x", "node-4"] {
        assert_eq!(Mirror::node(other), None, "{other:?}");
    }
}
