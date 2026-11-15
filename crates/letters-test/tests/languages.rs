//! The language of a code block: the list under a fence while its name is typed, and the name in the corner
//! of the block's box, where it is changed.

use accesskit::{Action, ActionRequest, TreeId};
use letters_model::{Block, BlockKind, Document, Pos, Selection};
use letters_render::{Item, Point, Rect};
use letters_test::{Found, Harness, Role};

const CODE: &str = "fn main() {\n    run();\n}";

/// An empty document with a fence and `name` typed on its one line.
fn fenced(name: &str) -> Harness {
    let mut window = Harness::doc("");
    window.type_text("```").type_text(name);
    window
}

/// A note with [`CODE`] in a language between two paragraphs. The caret waits in the first.
fn note(language: Option<&str>) -> Harness {
    noted(CODE, language)
}

/// A note with one code block between two paragraphs. The caret waits in the first.
fn noted(code: &str, language: Option<&str>) -> Harness {
    let blocks = vec![
        Block::text("before", BlockKind::Paragraph, "Run this once."),
        Block::text("code", BlockKind::Code { language: language.map(str::to_owned) }, code),
        Block::text("after", BlockKind::Paragraph, "Then restart."),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.paint();
    window.ui().select(Selection::caret(Pos::new("before", 3)));
    window
}

/// The languages a list shows, in its order.
fn listed(window: &mut Harness) -> Vec<Found> {
    window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption).collect()
}

fn names(window: &mut Harness) -> Vec<String> {
    listed(window).into_iter().map(|found| found.name).collect()
}

/// The language that a key would take.
fn picked(window: &mut Harness) -> Option<String> {
    listed(window).into_iter().find(|found| found.selected == Some(true)).map(|found| found.name)
}

/// What the first block of the document is, and what it holds.
fn first(window: &Harness) -> (BlockKind, String) {
    let block = &window.editor().doc().blocks[0];
    (block.kind.clone(), block.plain_text())
}

fn code(language: Option<&str>) -> BlockKind {
    BlockKind::Code { language: language.map(str::to_owned) }
}

/// The language the code block of [`note`] is in.
fn language(window: &Harness) -> Option<String> {
    match &window.editor().doc().block(&"code".into()).expect("the block").kind {
        BlockKind::Code { language } => language.clone(),
        other => panic!("the block is code no longer: {other:?}"),
    }
}

/// The page area.
fn page(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("the page").bounds
}

/// The corner of the code block of [`note`] where its language is named: beside its copy button.
fn corner(window: &mut Harness) -> Rect {
    let (frame, page) = (window.paint(), page(window));
    let button = frame.items.into_iter().find_map(|item| match item {
        Item::Path { origin, stroke: Some(_), .. } if page.contains(origin) => Some(origin),
        _ => None,
    });
    let button = button.expect("a copy button");
    Rect::new(button.x - 120.0, button.y - 8.0, 112.0, 30.0)
}

/// Whether a frame names a language in a corner: small text, as code is not set.
fn says(window: &mut Harness, corner: Rect) -> bool {
    let named =
        |item: &Item| matches!(item, Item::Glyphs { origin, size, .. } if corner.contains(*origin) && *size < 13.0);
    window.paint().items.iter().any(named)
}

/// A point on the language named in a corner.
fn on(corner: Rect) -> Point {
    Point { x: corner.x + corner.width - 14.0, y: corner.y + corner.height / 2.0 }
}

/// A point in the text of the code block of [`note`].
fn in_code(window: &mut Harness) -> Point {
    let held = window.editor().selection().cloned().expect("a caret");
    window.ui().select(Selection::caret(Pos::new("code", 1)));
    window.paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    window.ui().select(held);
    window.paint();
    Point { x: caret.x, y: caret.y + caret.height / 2.0 }
}

/// Does one step of a case: `type:` types, `press:` presses a language of the list that shows, and anything
/// else is a key.
fn step(window: &mut Harness, step: &str) {
    match step.split_once(':') {
        Some(("type", text)) => window.type_text(text),
        Some((_, name)) => window.click(Role::ListBoxOption, name),
        None => window.key(step),
    };
}

/// Opens the list that changes the language of the code block of [`note`], by a press on its language.
fn open(window: &mut Harness) {
    let corner = corner(window);
    window.click_at(on(corner));
    window.assert_open(Role::Dialog, "Code language");
}

#[test]
fn a_fence_lists_the_languages_and_what_is_typed_after_it_narrows_them() {
    let mut window = fenced("");
    let all = names(&mut window);
    assert!(all.len() > 10 && all.contains(&"Rust".to_owned()) && all.contains(&"Plain text".to_owned()), "{all:?}");
    assert_eq!(window.focused().map(|found| found.role), Some(Role::Document), "the page keeps the keys");

    // What is typed after the fence, and the language a key would take: found by its name, by another name
    // it goes by, or by the ending of its files.
    let cases = [
        ("ru", "Rust"),
        ("PY", "Python"),
        ("js", "JavaScript"),
        ("yml", "YAML"),
        ("golang", "Go"),
        ("text", "Plain text"),
    ];
    for (typed, found) in cases {
        let mut window = fenced(typed);
        assert_eq!(picked(&mut window).as_deref(), Some(found), "{typed}");
        assert!(names(&mut window).len() < all.len(), "{typed}: the rest are gone");
    }
    assert_eq!(names(&mut fenced("klingon")), [""; 0], "a name nothing goes by lists nothing");
}

#[test]
fn only_a_fence_at_the_start_of_body_text_lists_languages() {
    // What is typed on an empty line.
    for typed in ["Run ```", "``` ", "``"] {
        let mut window = Harness::doc("");
        window.type_text(typed);
        assert_eq!(names(&mut window), [""; 0], "{typed:?}");
    }
    let mut window = Harness::doc("");
    window.type_text("# ").type_text("```");
    assert_eq!(first(&window), (BlockKind::Heading { level: 1 }, "```".to_owned()));
    assert_eq!(names(&mut window), [""; 0], "a heading is no place for a fence");
}

#[test]
fn tab_or_a_press_takes_a_language_and_enter_keeps_the_name_as_typed() {
    // What is typed after the fence, the keys pressed or the language pressed, and the language the code
    // is in then.
    let cases: [(&str, &[&str], Option<&str>); 10] = [
        ("js", &["Tab"], Some("javascript")),
        ("js", &["Enter"], Some("js")),
        ("", &["Down", "Tab"], Some("diff")),
        ("", &["Up", "Up", "Tab"], Some("yaml")),
        ("", &["Enter"], None),
        ("pl", &["Tab"], None),
        ("klingon", &["Enter"], Some("klingon")),
        ("j", &["press:JSON"], Some("json")),
        ("", &["press:HTML"], Some("html")),
        ("te", &["press:Plain text"], None),
    ];
    for (typed, taken, language) in cases {
        let mut window = fenced(typed);
        taken.iter().for_each(|done| step(&mut window, done));
        assert_eq!(first(&window), (code(language), String::new()), "{typed:?}, {taken:?}");
        assert_eq!(names(&mut window), [""; 0], "{typed:?}, {taken:?}: the list is gone");
        window.type_text("run();");
        assert_eq!(first(&window).1, "run();", "{typed:?}, {taken:?}: the caret is in the code");
    }
}

#[test]
fn the_list_goes_when_asked_to_or_left_and_the_line_stays_as_typed() {
    // What puts the list away, and what makes the line and the caret as they were before that.
    for (away, back) in [("Escape", None), ("Left", Some("Right")), ("type: ", Some("Backspace"))] {
        let mut window = fenced("ru");
        assert!(!names(&mut window).is_empty());
        step(&mut window, away);
        assert_eq!(names(&mut window), [""; 0], "{away}");
        assert_eq!(first(&window).0, BlockKind::Paragraph, "{away}: the line is body text still");
        assert!(first(&window).1.starts_with("```ru"), "{away}: {:?}", first(&window).1);
        // A list that went is not in the way again a key later.
        back.into_iter().for_each(|back| step(&mut window, back));
        assert_eq!(names(&mut window), [""; 0], "{away}, {back:?}");
    }
    // A list that was put away does not come back for more of the name.
    let mut window = fenced("ru");
    window.key("Escape").type_text("st");
    assert_eq!(names(&mut window), [""; 0]);
    window.key("Enter");
    assert_eq!(first(&window), (code(Some("rust")), String::new()), "and the fence is the editor's as before");
}

#[test]
fn a_code_block_names_its_language_in_its_corner() {
    // What the fence wrote, what the block is called by, and whether the corner says so by itself.
    let cases = [
        (Some("rust"), "Language: Rust", true),
        (Some("yml"), "Language: YAML", true),
        (Some("mermaid"), "Language: mermaid", true),
        // Code in no language says so only under the pointer.
        (None, "Language: Plain text", false),
    ];
    for (written, called, shows) in cases {
        let mut window = note(written);
        assert!(window.find(Role::Button, called).is_some(), "{written:?}");
        let corner = corner(&mut window);
        assert_eq!(says(&mut window, corner), shows, "{written:?}");
        let inside = in_code(&mut window);
        window.hover_at(inside);
        assert!(says(&mut window, corner), "{written:?}: under the pointer");
    }
}

#[test]
fn a_first_line_that_reaches_the_corner_has_the_name_only_under_the_pointer() {
    let mut window = noted(&format!("{}\nshort", "long ".repeat(60)), Some("rust"));
    let inside = in_code(&mut window);
    window.hover_at(inside);
    let corner = corner(&mut window);
    assert!(says(&mut window, corner), "under the pointer the name covers the end of the line");

    let away = Point { x: inside.x, y: page(&mut window).y + 4.0 };
    window.hover_at(away);
    assert!(!says(&mut window, corner), "the name is not drawn over the code");
}

#[test]
fn a_press_on_the_language_lists_the_languages_and_one_is_taken() {
    // The language the block is in, what is done in the list, and the language the block is in then.
    let cases: [(Option<&str>, &[&str], Option<&str>); 10] = [
        (Some("rust"), &["type:py", "Enter"], Some("python")),
        (Some("rust"), &["press:Python"], Some("python")),
        (Some("rust"), &["Down", "Enter"], Some("bash")),
        (Some("rust"), &["type:plain", "Enter"], None),
        (None, &["type:ts", "Enter"], Some("typescript")),
        // The language it is in already leaves the block as it is, and so does putting the list away.
        (Some("rust"), &["Enter"], Some("rust")),
        (Some("rust"), &["type:py", "Escape"], Some("rust")),
        (Some("mermaid"), &["Enter"], Some("mermaid")),
        // From a language that is not listed the keys go to the first that is, or to the last.
        (Some("mermaid"), &["Down", "Enter"], Some("css")),
        (Some("mermaid"), &["Up", "Enter"], None),
    ];
    for (written, done, language_then) in cases {
        let mut window = note(written);
        open(&mut window);
        let field = window.focused().expect("something focused");
        assert_eq!((field.role, field.name.as_str()), (Role::TextInput, "Language"), "{written:?}");
        done.iter().for_each(|done| step(&mut window, done));
        assert_eq!(language(&window).as_deref(), language_then, "{written:?}, {done:?}");
        let code = window.editor().doc().block(&"code".into()).map(Block::plain_text);
        assert_eq!(code.as_deref(), Some(CODE), "{written:?}, {done:?}: the code is as it was");
        window.assert_closed(Role::Dialog, "Code language");
        assert_eq!(window.focused().map(|found| found.role), Some(Role::Document), "{written:?}, {done:?}");
    }
}

#[test]
fn the_list_opens_on_the_language_the_block_is_in() {
    // The language the block is in, and the language picked when the list opens.
    let cases = [(Some("rs"), Some("Rust")), (None, Some("Plain text")), (Some("mermaid"), None)];
    for (written, shown) in cases {
        let mut window = note(written);
        open(&mut window);
        assert_eq!(picked(&mut window).as_deref(), shown, "{written:?}");
        assert_eq!(names(&mut window).len(), 15, "{written:?}: every language is listed");
    }
}

#[test]
fn a_change_of_language_is_undone_in_one_step() {
    let mut window = note(Some("rust"));
    open(&mut window);
    window.type_text("py").key("Enter");
    assert_eq!(language(&window).as_deref(), Some("python"));
    assert!(window.find(Role::Button, "Language: Python").is_some(), "the corner says so");
    window.key("Mod+Z");
    assert_eq!(language(&window).as_deref(), Some("rust"));
}

#[test]
fn assistive_technology_finds_the_language_and_presses_it() {
    let mut window = note(Some("rust"));
    let tree = window.ui().access();
    let named = |node: &accesskit::Node| node.role() == Role::Button && node.label() == Some("Language: Rust");
    let (target_node, node) = tree.nodes.iter().find(|(_, node)| named(node)).expect("the language");
    assert!(node.supports_action(Action::Click), "it says that it can be pressed");
    let target_node = *target_node;
    window.ask(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None });
    window.assert_open(Role::Dialog, "Code language");
    assert_eq!(window.editor().selection().map(|selection| selection.head.block.as_str()), Some("code"));
}

#[test]
fn the_palette_changes_the_language_of_the_code_the_caret_is_in() {
    let offered = |window: &mut Harness| {
        window.key("Mod+Shift+P").type_text("Change language");
        let listed = window.find(Role::ListBoxOption, "Change language").is_some();
        window.key("Escape");
        listed
    };
    let mut window = note(Some("rust"));
    assert!(!offered(&mut window), "body text is in no language");
    window.ui().select(Selection::caret(Pos::new("code", 4)));
    assert!(offered(&mut window));
    window.key("Mod+Shift+P").type_text("Change language").key("Enter");
    window.assert_open(Role::Dialog, "Code language");
    window.type_text("go").key("Enter");
    assert_eq!(language(&window).as_deref(), Some("go"));
}
