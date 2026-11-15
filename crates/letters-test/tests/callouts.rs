//! Changing a callout after it is made: the card under its icon and title, and the commands.

use letters_model::BlockKind;
use letters_render::{Item, Point, Rect};
use letters_test::{Harness, Role};

/// A note with one callout between two lines, and the caret in the first line.
fn note() -> Harness {
    let mut window = Harness::doc("Before\n\n> [!NOTE]\n> Reply by Friday.\n\nAfter");
    window.select("Before");
    window.paint();
    window
}

/// The callout as the document has it.
fn callout(window: &Harness) -> BlockKind {
    let found = window.editor().doc().walk().find(|block| matches!(block.kind, BlockKind::Callout { .. }));
    found.expect("the note has a callout").kind.clone()
}

/// What the callout shows: its title, its icon and its colour, the tone's own where none was chosen.
fn shown(window: &Harness) -> (String, Option<&'static str>, Option<String>) {
    let kind = callout(window);
    let look = kind.callout_look().expect("a callout has a look");
    (look.title.to_owned(), look.icon, look.color.map(str::to_owned))
}

/// A point on the callout's icon, which is the one icon the page draws.
fn icon(window: &mut Harness) -> Point {
    let frame = window.paint();
    let page: Rect = window.controls().into_iter().find(|found| found.role == Role::Document).expect("the page").bounds;
    let mut icons = frame.items.into_iter().filter_map(|item| match item {
        Item::Path { origin, stroke: Some(_), .. } if page.contains(origin) => Some(origin),
        _ => None,
    });
    let origin = icons.next().expect("the callout shows its icon");
    Point { x: origin.x + 6.0, y: origin.y + 6.0 }
}

#[test]
fn a_press_on_a_callouts_icon_opens_a_card_that_changes_it() {
    let mut window = note();
    assert_eq!(shown(&window), ("Info".to_owned(), Some("info"), None), "a tone gives its own title and icon");
    let at = icon(&mut window);
    window.click_at(at).assert_open(Role::Dialog, "Callout");

    // Each choice shows on the page at once, and the card stays for the next.
    window.click(Role::RadioButton, "Warning");
    assert_eq!(shown(&window), ("Warning".to_owned(), Some("triangle-alert"), None), "the title follows the tone");
    window.click(Role::Button, "Flame").click(Role::Button, "Violet").assert_open(Role::Dialog, "Callout");
    let (title, icon, colour) = shown(&window);
    assert_eq!((title.as_str(), icon), ("Warning", Some("flame")));
    assert!(colour.is_some(), "a colour was chosen");

    // What was chosen stays when the type changes, and goes with one press.
    window.click(Role::RadioButton, "Danger");
    assert_eq!(shown(&window), ("Danger".to_owned(), Some("flame"), colour));
    window.click(Role::Button, "None");
    assert_eq!(shown(&window).1, None, "a callout can show no icon");
    window.click(Role::Button, "Use the defaults for Danger");
    assert_eq!(shown(&window), ("Danger".to_owned(), Some("octagon-alert"), None));

    window.key("Escape").assert_closed(Role::Dialog, "Callout");
    assert!(window.markdown().contains("Reply by Friday."), "the text is as it was: {}", window.markdown());
}

#[test]
fn a_callout_that_reads_from_the_right_is_pressed_and_carded_at_its_right_end() {
    // How far the card's edge may stand from the icon it hangs under.
    const NEAR: f32 = 48.0;
    // What the callout says, and whether its icon and title stand at the right.
    let cases = [
        ("Reply by Friday.", false),
        ("\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}", true),
    ];
    for (says, from_right) in cases {
        let mut window = Harness::doc(&format!("Before\n\n> [!NOTE]\n> {says}\n\nAfter"));
        window.select("Before");
        let at = icon(&mut window);
        let page = window.controls().into_iter().find(|found| found.role == Role::Document).expect("the page").bounds;
        assert_eq!(at.x > page.x + page.width / 2.0, from_right, "{says}: where the icon stands");

        window.click_at(at).assert_open(Role::Dialog, "Callout");
        let card = window.find(Role::Dialog, "Callout").expect("the card").bounds;
        let (left, right) = (at.x - card.x, card.x + card.width - at.x);
        assert!(left >= 0.0 && right >= 0.0, "{says}: the card hangs under the icon, {card:?} at {at:?}");
        let near = if from_right { right } else { left };
        assert!(near < NEAR, "{says}: from the end the icon is at, {card:?} at {at:?}");
    }
}

#[test]
fn a_title_is_typed_on_the_card() {
    let mut window = note();
    let at = icon(&mut window);
    window.click_at(at).click(Role::TextInput, "Title").key("Mod+A").type_text("Heads up");
    assert_eq!(shown(&window).0, "Heads up");
    // An emptied title leaves the callout with none, and the tone's own title typed is the tone's again.
    window.key("Mod+A").key("Backspace");
    assert_eq!(shown(&window).0, "");
    window.type_text("Info");
    assert_eq!(callout(&window), BlockKind::callout(letters_model::Tone::Info));
}

#[test]
fn a_colour_that_is_typed_is_taken_when_it_is_one() {
    // Each case: what is typed, and the colour the callout has after Enter.
    let cases = [("#7A3FB5", Some("#7a3fb5")), ("7a3fb5", Some("#7a3fb5")), ("violet", None)];
    for (typed, colour) in cases {
        let mut window = note();
        let at = icon(&mut window);
        window.click_at(at).click(Role::TextInput, "Colour").key("Mod+A").type_text(typed).key("Enter");
        assert_eq!(shown(&window).2.as_deref(), colour, "typing {typed:?}");
    }
}

#[test]
fn the_palette_changes_the_callout_the_caret_is_in() {
    let mut window = note();
    // Outside a callout there is nothing to change.
    window.key("Mod+Shift+P").type_text("use the type's defaults");
    assert!(window.find(Role::ListBoxOption, "Use the type's defaults").is_none(), "no callout, no command");
    window.key("Escape");

    window.select("Friday");
    window.key("Mod+Shift+P").type_text("callout danger").key("Enter");
    assert_eq!(shown(&window).0, "Danger");
    window.key("Mod+Shift+P").type_text("edit callout").key("Enter").assert_open(Role::Dialog, "Callout");
    window.key("Escape");
    window.key("Mod+Shift+P").type_text("callout body text").key("Enter");
    assert!(window.editor().doc().walk().all(|block| !matches!(block.kind, BlockKind::Callout { .. })));
    assert!(window.markdown().contains("Reply by Friday."), "the text stays: {}", window.markdown());
}
