//! The editor's window, as a shell shows it: the page, the top bar and the side panel.

use accesskit::{Action, ActionData, ActionRequest, NodeId, TextPosition, TextSelection, Toggled, TreeId, TreeUpdate};
use letters_editor::InputEvent;
use letters_model::{Align, Block, BlockKind, Document, ListKind, Pos, Selection, Style, TextStyle};
use letters_render::{Color, Item, PathEl, Point, Rect};
use letters_test::{Harness, Role, WINDOW};
use letters_ui::{Chrome, Cursor, Edge, Folder, Frame, Landing, Location, Recent, Request, SaveState, WindowButton};

fn text(id: &str, kind: BlockKind, text: &str) -> Block {
    Block::text(id, kind, text)
}

/// A short letter with two headings.
fn letter() -> Harness {
    let blocks = vec![
        text("h1", BlockKind::Heading { level: 1 }, "Roof repairs"),
        text("p1", BlockKind::Paragraph, "Harland Roofing sent a quote."),
        text("h2", BlockKind::Heading { level: 2 }, "Next steps"),
        text("p2", BlockKind::Paragraph, "Reply by Friday."),
    ];
    Harness::open(Document { blocks, ..Document::default() })
}

/// A document far taller than the window.
fn long() -> Harness {
    let blocks =
        (0..200).map(|n| text(&format!("p{n}"), BlockKind::Paragraph, "One more line of the letter.")).collect();
    Harness::open(Document { blocks, ..Document::default() })
}

/// The headings of [`report`], with their levels.
const REPORT: [(u8, &str); 4] = [(1, "Survey"), (2, "Roof"), (3, "Flashing"), (2, "Gutters")];

/// A long report: each heading of [`REPORT`] over forty paragraphs. Heading `n` is block `s{n}`.
fn report() -> Harness {
    let mut blocks = Vec::new();
    for (n, (level, title)) in REPORT.into_iter().enumerate() {
        blocks.push(text(&format!("s{n}"), BlockKind::Heading { level }, title));
        let paragraph = |p| text(&format!("s{n}p{p}"), BlockKind::Paragraph, "One more line of the report.");
        blocks.extend((0..40).map(paragraph));
    }
    Harness::open(Document { blocks, ..Document::default() })
}

/// A new document: one empty paragraph with the caret in it, and nothing written to disk.
fn blank() -> Harness {
    let mut window =
        Harness::open(Document { blocks: vec![text("p", BlockKind::Paragraph, "")], ..Document::default() });
    window.ui().set_save_state(SaveState::Unsaved);
    window
}

/// What the shell offers on the landing card: seven documents, the newest first, named "Doc 1" to "Doc 7".
fn landing() -> Landing {
    let opened = ["2026-11-14T10:28:00+01:00", "2026-11-13T09:00:00+01:00", "2026-11-03T09:00:00+01:00"];
    let recent = |n: usize| Recent {
        path: format!("/family/doc-{n}"),
        name: format!("Doc {n}"),
        folder: "Family / House".into(),
        opened: opened[(n - 1).min(2)].into(),
    };
    Landing { now: "2026-11-14T10:30:00+01:00".into(), recents: (1..=7).map(recent).collect() }
}

/// The page area.
fn page(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("the page").bounds
}

/// The lines the outline draws, top to bottom, each with its colour.
fn outline_lines(window: &mut Harness) -> Vec<(Rect, Color)> {
    let outline = window.find(Role::Navigation, "Outline").expect("the outline").bounds;
    let line = |item: &Item| match item {
        Item::Fill { rect, color, .. } if rect.height == 2.0 && outline.contains(Point { x: rect.x, y: rect.y }) => {
            Some((*rect, *color))
        }
        _ => None,
    };
    window.paint().items.iter().filter_map(line).collect()
}

/// Where a position of the document shows in the window.
fn spot(window: &mut Harness, block: &str, offset: usize) -> Rect {
    window.ui().select(Selection::caret(Pos::new(block, offset)));
    window.paint();
    window.ui().caret_rect().expect("the page shows a caret")
}

fn selected(window: &Harness) -> String {
    let selection = window.editor().selection().expect("a selection");
    let (start, end) =
        (selection.anchor.offset.min(selection.head.offset), selection.anchor.offset.max(selection.head.offset));
    block_text(window, selection.head.block.as_str())[start..end].to_owned()
}

fn block_text(window: &Harness, id: &str) -> String {
    window.editor().doc().block(&id.into()).map(Block::plain_text).unwrap_or_default()
}

/// Where the caret shows in the next frame.
fn caret(window: &mut Harness) -> Rect {
    window.paint();
    window.ui().caret_rect().expect("the page shows a caret")
}

/// A sentence with a bold word, whose markers show once the caret is inside it, as at `INSIDE`.
fn marked(reduced_motion: bool) -> Harness {
    let mut window = Harness::doc("Send the **quote** today.");
    window.ui().set_reduced_motion(reduced_motion);
    window.ui().select(Selection::caret(Pos::new("b1", 0)));
    window.paint();
    window
}

const INSIDE: usize = 11;

/// Lets the window do the work it keeps for idle time, all of it, as a shell does: it asks what moves, paints
/// the frame, and comes back for as long as it is told to at once.
fn rest(window: &mut Harness) {
    let mut steps = 0;
    loop {
        let again = window.ui().animate(0);
        window.paint();
        if again != Some(0) {
            break;
        }
        steps += 1;
        assert!(steps < 10_000, "idle work comes to an end");
    }
}

/// The wavy lines under misspelled words, left to right down the page: where each starts and ends.
fn waves(window: &mut Harness) -> Vec<(Point, Point)> {
    let page = page(window);
    let wave = |item: &Item| match item {
        Item::Path { origin, path, stroke: Some(_), .. } if page.contains(*origin) => {
            let ends = path.iter().filter_map(|el| if let PathEl::QuadTo(_, end) = el { Some(end.x) } else { None });
            Some((*origin, Point { x: origin.x + ends.fold(0.0, f32::max), y: origin.y }))
        }
        _ => None,
    };
    window.paint().items.iter().filter_map(wave).collect()
}

/// A sentence with one misspelled word, checked.
fn misspelled() -> Harness {
    let mut window = Harness::doc("The quik brown fox.");
    rest(&mut window);
    window
}

/// Asks for the menu on the first wavy line, as a press of the secondary button on its word does.
fn ask_about_first_wave(window: &mut Harness) {
    let (start, end) = waves(window)[0];
    window.secondary_click_at(Point { x: (start.x + end.x) / 2.0, y: start.y - 6.0 });
}

fn head(window: &Harness) -> Pos {
    window.editor().selection().expect("a selection").head.clone()
}

/// The style the window draws an element type with.
fn style_of(window: &mut Harness, element: &str) -> TextStyle {
    window.ui().style().types[element].clone()
}

/// Narrows the window until the bar has folded far enough to show a button.
fn narrow_until(window: &mut Harness, button: &str) {
    let shows = |width: u16| {
        window.resize(f32::from(width), 600.0);
        window.find(Role::Button, button).is_some()
    };
    assert!((600..=1400).rev().step_by(20).any(shows), "no width of window shows {button}");
}

/// The bar's buttons and fields, left to right.
fn bar_controls(window: &mut Harness) -> Vec<(String, Rect)> {
    let bar = window.find(Role::Toolbar, "Document").expect("the bar").bounds;
    let in_bar = |found: &letters_test::Found| {
        matches!(found.role, Role::Button | Role::TextInput) && bar.contains(found.center())
    };
    let mut controls: Vec<(String, Rect)> =
        window.controls().into_iter().filter(in_bar).map(|found| (found.name, found.bounds)).collect();
    controls.sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
    controls
}

#[test]
fn the_window_opens_on_the_page_with_every_control_reachable() {
    let mut window = letter();
    assert_eq!(window.focused().map(|found| found.role), Some(Role::Document));
    window.assert_open(Role::Toolbar, "Document");
    let buttons =
        ["Bold", "Italic", "Underline", "Strikethrough", "Code", "Share", "Styles", "Notes", "History", "More"];
    for name in buttons {
        let button = window.find(Role::Button, name).unwrap_or_else(|| panic!("no {name} button"));
        assert!(
            button.bounds.x >= 0.0 && button.bounds.x + button.bounds.width <= WINDOW.width,
            "{name} is in the window"
        );
    }
}

#[test]
fn typing_goes_into_the_document_at_the_caret() {
    let mut window = letter();
    let before = window.ui().edits();
    window.type_text("Re: ");
    assert_eq!(block_text(&window, "h1"), "Re: Roof repairs");
    assert!(window.ui().edits() > before, "the shell can tell the document changed");
}

#[test]
fn a_click_on_the_page_moves_the_caret_there() {
    let mut window = letter();
    window.paint();
    let caret = window.ui().caret_rect().expect("the page shows a caret");
    window.click_at(Point { x: caret.x + 60.0, y: caret.y + caret.height / 2.0 });
    let head = head(&window);
    assert_eq!(head.block.as_str(), "h1");
    assert!(head.offset > 0, "the caret moved along the line");
    window.type_text("!");
    assert_eq!(block_text(&window, "h1").len(), "Roof repairs!".len());
}

#[test]
fn the_bar_names_the_document_its_save_state_and_the_type_at_the_caret() {
    let mut window = letter();
    window.ui().set_document_name("Quote");
    window.assert_open(Role::Button, "Quote").assert_open(Role::Label, "Saved").assert_open(Role::Button, "Heading 1");

    window.ui().set_save_state(SaveState::Offline);
    window.ui().select(Selection::caret(Pos::new("p1", 0)));
    window.assert_open(Role::Label, "Offline").assert_open(Role::Button, "Body");
    window.assert_closed(Role::Label, "Saved");
}

#[test]
fn share_opens_a_dialog_that_closes_again() {
    let mut window = letter();
    window.click(Role::Button, "Share");
    window.assert_open(Role::Dialog, "Share");
    window.click(Role::Button, "Done");
    window.assert_closed(Role::Dialog, "Share");
    assert_eq!(window.focused().map(|found| found.role), Some(Role::Document), "focus is back on the page");
}

#[test]
fn one_side_panel_shows_at_a_time() {
    let mut window = letter();
    // (button pressed, panel showing afterwards)
    let steps = [
        ("Styles", Some("Styles")),
        ("Notes", Some("Notes")),
        ("Notes", None),
        ("History", Some("History")),
        ("Close", None),
    ];
    for (button, showing) in steps {
        window.click(Role::Button, button);
        for panel in ["Styles", "Notes", "History"] {
            let open = window.find(Role::Complementary, panel).is_some();
            assert_eq!(open, showing == Some(panel), "{panel} after pressing {button}");
            assert_eq!(window.find(Role::Button, panel).unwrap().toggled, Some(open), "the {panel} button says so");
        }
    }
}

#[test]
fn the_outline_is_a_line_for_each_heading_stepped_in_and_shortened_by_its_level() {
    let mut window = report();
    for (_, title) in REPORT {
        window.assert_open(Role::Link, title);
    }
    let lines: Vec<Rect> = outline_lines(&mut window).into_iter().map(|line| line.0).collect();
    assert_eq!(lines.len(), REPORT.len(), "one line for each heading");
    assert!(lines.windows(2).all(|pair| pair[0].y < pair[1].y), "in the document's order, down the page");
    // Survey is level 1, Roof and Gutters level 2, Flashing level 3.
    assert!(lines[0].x < lines[1].x && lines[1].x < lines[2].x, "deeper headings step in: {lines:?}");
    assert_eq!(lines[3].x, lines[1].x, "headings of one level line up");
    let right = lines[0].x + lines[0].width;
    assert!(lines.iter().all(|line| line.x + line.width == right), "every line ends at one edge: {lines:?}");

    let (outline, page) = (window.find(Role::Navigation, "Outline").unwrap().bounds, page(&mut window));
    assert_eq!((outline.x, outline.y), (page.x, page.y), "at the top left of the page");
    assert!(outline.width < 40.0 && outline.height < 80.0, "and very small: {outline:?}");
}

#[test]
fn an_outline_needs_two_headings() {
    // (document, whether it has an outline)
    let cases = [("Text.", false), ("# Title\n\nText.", false), ("# Title\n\nText.\n\n## Part\n\nMore.", true)];
    for (document, shown) in cases {
        let mut window = Harness::doc(document);
        assert_eq!(window.find(Role::Navigation, "Outline").is_some(), shown, "{document:?}");
    }
}

#[test]
fn a_line_of_the_outline_names_its_heading_under_the_pointer() {
    let mut window = report();
    window.assert_closed(Role::Tooltip, "Flashing");
    window.hover(Role::Link, "Flashing").assert_open(Role::Tooltip, "Flashing");
    let (tip, line) =
        (window.find(Role::Tooltip, "Flashing").unwrap().bounds, window.find(Role::Link, "Flashing").unwrap());
    assert!(tip.x >= line.bounds.x + line.bounds.width, "the name shows beside its line: {tip:?}");
    assert!(tip.y < line.center().y && line.center().y < tip.y + tip.height, "and level with it: {tip:?}");

    window.hover(Role::Link, "Gutters").assert_open(Role::Tooltip, "Gutters").assert_closed(Role::Tooltip, "Flashing");
    window.ui().pointer_leave();
    window.assert_closed(Role::Tooltip, "Gutters");
}

#[test]
fn pressing_a_line_of_the_outline_brings_its_heading_to_the_top() {
    // (heading pressed, its block)
    for (title, block) in [("Roof", "s1"), ("Gutters", "s3"), ("Survey", "s0")] {
        let mut window = report();
        window.click(Role::Link, title).paint();
        assert_eq!(head(&window), Pos::new(block, 0), "the caret goes to {title}");
        let (caret, page) = (window.ui().caret_rect().unwrap(), page(&mut window));
        assert!((0.0..80.0).contains(&(caret.y - page.y)), "{title} is at the top of the page: {caret:?}");
    }
}

#[test]
fn the_outline_marks_the_section_in_view() {
    let mut window = report();
    let marked = |window: &mut Harness| {
        let lines = outline_lines(window);
        let odd = |n: usize| lines.iter().filter(|line| line.1 == lines[n].1).count() == 1;
        (0..lines.len()).filter(|&n| odd(n)).collect::<Vec<_>>()
    };
    assert_eq!(marked(&mut window), [0], "the first section, at the top of the document");

    window.click(Role::Link, "Flashing");
    window.ui().pointer_leave();
    assert_eq!(marked(&mut window), [2], "the section brought to the top");

    let page = page(&mut window);
    window.scroll_at(Point { x: page.x + page.width / 2.0, y: page.y + page.height / 2.0 }, -1.0e6);
    assert_eq!(marked(&mut window), [0], "and it follows the wheel back");
}

#[test]
fn the_wheel_scrolls_the_page_from_over_the_outline() {
    let mut window = report();
    window.paint();
    let (start, over) = (window.ui().caret_rect().unwrap(), window.find(Role::Link, "Roof").unwrap().center());
    window.scroll_at(over, 300.0).paint();
    assert_eq!(window.ui().caret_rect().unwrap().y, start.y - 300.0);
}

#[test]
fn the_outline_is_one_stop_for_tab_and_the_arrows_move_along_it() {
    let mut window = report();
    let focused = |window: &mut Harness| window.focused().map(|found| (found.role, found.name));
    window.key("Tab");
    assert_eq!(focused(&mut window), Some((Role::Link, "Survey".into())));
    window.key("Down").key("Down");
    assert_eq!(focused(&mut window), Some((Role::Link, "Flashing".into())));
    window.assert_open(Role::Tooltip, "Flashing");

    window.key("Enter");
    assert_eq!(head(&window), Pos::new("s2", 0));
    window.key("Tab");
    assert_ne!(window.focused().map(|found| found.role), Some(Role::Link), "one press of Tab leaves the outline");
}

#[test]
fn the_page_gives_room_to_the_panel() {
    let mut window = letter();
    let wide = page(&mut window);
    window.click(Role::Button, "Styles");
    let (narrow, panel) = (page(&mut window), window.find(Role::Complementary, "Styles").unwrap().bounds);
    assert!(narrow.width < wide.width);
    assert_eq!(narrow.x + narrow.width, panel.x);
}

#[test]
fn a_frame_shows_the_text_and_a_caret() {
    let mut window = letter();
    let frame = window.paint();
    let glyphs = frame.items.iter().filter(|item| matches!(item, Item::Glyphs { .. })).count();
    assert!(glyphs >= 6, "labels and four blocks of text are drawn: {glyphs} runs");
    let (caret, page) = (window.ui().caret_rect().unwrap(), window.find(Role::Document, "Letters").unwrap().bounds);
    assert!(page.contains(Point { x: caret.x, y: caret.y }), "the caret is on the page: {caret:?}");
}

#[test]
fn the_page_scrolls_and_typing_brings_the_caret_back() {
    let mut window = long();
    window.paint();
    let start = window.ui().caret_rect().unwrap();
    window.scroll_at(Point { x: 500.0, y: 400.0 }, 2000.0).paint();
    let away = window.ui().caret_rect().unwrap();
    assert_eq!(away.y, start.y - 2000.0, "the page moved under the wheel");

    window.type_text("x").paint();
    let back = window.ui().caret_rect().unwrap();
    assert!(back.y > 44.0 && back.y + back.height < WINDOW.height, "typing shows the caret again: {back:?}");
}

// The design's own example.
#[test]
fn a_link_is_made_in_place_and_sharing_opens_a_dialog() {
    let mut ed = Harness::doc("Harland Roofing sent a quote.");
    ed.select("quote").key("Mod+K").type_text("https://example.com").key("Enter");
    ed.assert_markdown("Harland Roofing sent a [quote](https://example.com).");
    ed.click(Role::Button, "Share");
    ed.assert_open(Role::Dialog, "Share");
}

#[test]
fn keys_on_the_page_edit_the_document() {
    // (document, selected text, keys, result)
    let cases = [
        ("Send the quote.", "quote", "Mod+B", "Send the **quote**."),
        ("Send the quote.", "quote", "Mod+I", "Send the *quote*."),
        ("Send the quote.", "the ", "Backspace", "Send quote."),
        ("Send the quote.", "quote", "Backspace Mod+Z", "Send the quote."),
        ("Send the quote.", "quote", "Backspace Mod+Z Mod+Shift+Z", "Send the ."),
        ("SendQuote.", "Quote", "Left Enter", "Send\n\nQuote."),
    ];
    for (document, text, keys, result) in cases {
        let mut ed = Harness::doc(document);
        ed.select(text);
        keys.split(' ').for_each(|key| _ = ed.key(key));
        assert_eq!(ed.markdown().trim_end(), result.trim_end(), "{document:?}, {text:?} selected, then {keys}");
    }
}

#[test]
fn the_link_editor_edits_removes_and_gives_up() {
    // (keys in the editor, result)
    let cases = [
        ("'https://b.example/' Enter", "A [quote](https://b.example/)."),
        ("End 'x' Enter", "A [quote](https://a.example/x)."),
        ("Backspace Enter", "A quote."),
        ("'x' Escape", "A [quote](https://a.example/)."),
    ];
    for (keys, result) in cases {
        let mut ed = Harness::doc("A [quote](https://a.example/).");
        ed.select("quote").click(Role::Button, "Link");
        assert_eq!(
            ed.focused().map(|found| (found.name, found.value)),
            Some(("Link".into(), Some("https://a.example/".into())))
        );
        for key in keys.split(' ') {
            match key.strip_prefix('\'') {
                Some(text) => ed.type_text(text.trim_end_matches('\'')),
                None => ed.key(key),
            };
        }
        ed.assert_closed(Role::Dialog, "Link").assert_markdown(result);
        assert_eq!(ed.focused().map(|found| found.role), Some(Role::Document), "{keys}: focus is back on the page");
    }
}

#[test]
fn the_bar_shows_and_sets_the_marks_at_the_caret() {
    let mut ed = Harness::doc("Send the **quote** today.");
    ed.select("quote");
    assert_eq!(ed.find(Role::Button, "Bold").unwrap().toggled, Some(true));
    assert_eq!(ed.find(Role::Button, "Italic").unwrap().toggled, Some(false));
    ed.click(Role::Button, "Italic").click(Role::Button, "Bold");
    ed.assert_markdown("Send the *quote* today.");
    assert_eq!(ed.focused().map(|found| found.role), Some(Role::Document), "the bar leaves focus on the page");
}

#[test]
fn the_bar_restyles_every_block_of_the_type_at_the_caret() {
    let mut window = letter();
    let (text, edits) = (window.markdown(), window.ui().edits());
    let body = style_of(&mut window, "body");
    let left = spot(&mut window, "h1", 0).x;

    window.click(Role::Button, "Font").click(Role::MenuItemCheckBox, "Fira Code");
    window.click(Role::Button, "Font").click(Role::MenuItemCheckBox, "Light");
    window.click(Role::Button, "Align centre");

    let heading = style_of(&mut window, "heading1");
    assert_eq!((heading.family.as_str(), heading.weight, heading.align), ("Fira Code", 300, Align::Center));
    assert_eq!(style_of(&mut window, "body"), body, "other types keep their style");
    assert_eq!(window.markdown(), text, "the text is as it was");
    assert!(window.ui().edits() > edits, "the shell can tell there is something to save");
    assert!(spot(&mut window, "h1", 0).x > left, "the page shows the heading centred");
    let pressed = |window: &mut Harness, name| window.find(Role::Button, name).unwrap().toggled;
    assert_eq!((pressed(&mut window, "Align centre"), pressed(&mut window, "Align left")), (Some(true), Some(false)));
    assert_eq!(window.find(Role::Button, "Font").unwrap().description.as_deref(), Some("Fira Code"));

    // Each change to the style is a step that undo takes back.
    window.click(Role::Button, "Undo");
    assert_eq!(style_of(&mut window, "heading1").align, body.align);
    assert_eq!(spot(&mut window, "h1", 0).x, left, "and the page shows it");
    assert_eq!(style_of(&mut window, "heading1").weight, 300, "one step at a time");
}

#[test]
fn a_style_changed_by_another_writer_shows_on_the_page() {
    let mut window = letter();
    let left = spot(&mut window, "h1", 0).x;
    let mut style = window.ui().style().clone();
    style.types.get_mut("heading1").expect("a style for headings").align = Align::Center;
    window.ui().change(|editor| editor.rewrite_style(style)).expect("the style is taken in");
    assert!(spot(&mut window, "h1", 0).x > left, "the heading is centred");
    assert_eq!(window.find(Role::Button, "Align centre").unwrap().toggled, Some(true), "and the bar says so");
}

#[test]
fn what_is_typed_into_a_style_field_is_taken_or_dropped() {
    // What is typed, how the typing ends, and whether 18 points is then in force.
    let cases = [
        ("18", "Enter", true),
        (" 18 ", "leave", true),
        ("18", "Escape", false),
        ("big", "Enter", false),
        ("0", "leave", false),
    ];
    for (typed, end, taken) in cases {
        let mut window = letter();
        let elsewhere = spot(&mut window, "p1", 5);
        let before = style_of(&mut window, "body").size;
        window.click(Role::TextInput, "Size").key("Mod+A").type_text(typed);
        match end {
            "leave" => window.click_at(Point { x: elsewhere.x, y: elsewhere.y + elsewhere.height / 2.0 }),
            key => window.key(key),
        };

        let after = style_of(&mut window, "body").size;
        assert_eq!(after, if taken { 18.0 } else { before }, "{typed:?} then {end}");
        let field = window.find(Role::TextInput, "Size").unwrap();
        assert_eq!(field.value, Some(after.to_string()), "the field shows the size in force after {typed:?}");
        if end != "Enter" {
            assert_eq!(window.focused().map(|found| found.role), Some(Role::Document), "focus is on the page");
        }
    }
}

#[test]
fn a_colour_is_picked_from_the_swatches_or_typed() {
    let mut window = letter();
    window.click(Role::Button, "Colour").assert_open(Role::Dialog, "Colour");
    window.click(Role::Button, "Red").assert_closed(Role::Dialog, "Colour");
    let red = style_of(&mut window, "heading1").color;
    assert_eq!(window.find(Role::Button, "Colour").unwrap().description.as_deref(), Some("Red"));
    let page = page(&mut window);
    let inks: Vec<String> = window
        .paint()
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Glyphs { origin, color, .. } if page.contains(*origin) => {
                Some(format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b))
            }
            _ => None,
        })
        .collect();
    assert!(inks.contains(&red), "the page draws the heading in {red}: {inks:?}");
    assert!(inks.contains(&style_of(&mut window, "body").color), "and the body as it was");

    window.click(Role::Button, "Colour").click(Role::TextInput, "Hex").key("Mod+A").type_text("0A7").key("Enter");
    assert_eq!(style_of(&mut window, "heading1").color, "#00aa77");
}

#[test]
fn spacing_opens_up_the_lines_of_a_type_and_the_room_around_it() {
    let mut window = letter();
    let gap = |window: &mut Harness| {
        let next = spot(window, "h2", 0).y;
        next - spot(window, "p1", 0).y
    };
    let before = gap(&mut window);
    window.click(Role::Button, "Spacing").assert_open(Role::Dialog, "Spacing");
    for (field, typed) in [("Line height", "2"), ("Space after", "24")] {
        window.click(Role::TextInput, field).key("Mod+A").type_text(typed).key("Enter");
    }
    let body = style_of(&mut window, "body");
    assert_eq!((body.line_height, body.space_after), (2.0, 24.0));
    window.key("Escape").assert_closed(Role::Dialog, "Spacing");
    assert!(gap(&mut window) > before, "the paragraph takes more of the page");
}

#[test]
fn a_narrowing_window_folds_alignment_then_the_marks_then_the_whole_style() {
    let name = "Roof repairs at the cottage: the quotes, the plan and what was agreed with Harland";
    let mut window = letter();
    window.ui().set_document_name(name);
    let mut folds = Vec::new();
    for width in (680..=1400).rev().step_by(20) {
        window.resize(width as f32, 600.0);
        let controls = bar_controls(&mut window);
        let has = |name: &str| controls.iter().any(|(found, _)| found == name);
        folds.push(match (has("Align left"), has("Bold"), has("Font")) {
            (true, true, true) => 0,
            (false, true, true) if has("Alignment") => 1,
            (false, false, true) if has("Alignment") && has("Marks") => 2,
            (false, false, false) if has("Heading 1 style") && has("Marks") => 3,
            _ => panic!("at {width} the bar holds {controls:?}"),
        });
        for pair in controls.windows(2) {
            assert!(pair[0].1.x + pair[0].1.width <= pair[1].1.x, "at {width} {pair:?} overlap");
        }
        let (first, last) = (&controls[0].1, &controls[controls.len() - 1].1);
        assert!(first.x >= 0.0 && last.x + last.width <= width as f32, "at {width} the bar is in the window");
    }
    window.assert_open(Role::Button, name);
    assert!(folds.is_sorted(), "the bar only folds further as the window narrows: {folds:?}");
    assert_eq!((folds[0], folds[folds.len() - 1]), (0, 3), "from everything shown to everything folded");
    window.resize(1400.0, 600.0).assert_open(Role::Button, "Align left").assert_open(Role::Button, "Bold");
}

#[test]
fn folded_controls_do_what_the_buttons_did() {
    let mut ed = Harness::doc("Send the **quote** today.");
    ed.select("quote");
    narrow_until(&mut ed, "Marks");
    assert_eq!(ed.find(Role::Button, "Marks").unwrap().toggled, Some(true), "a mark is in force");
    ed.click(Role::Button, "Marks").click(Role::MenuItemCheckBox, "Bold");
    ed.assert_markdown("Send the quote today.");
    ed.click(Role::Button, "Alignment").click(Role::MenuItemCheckBox, "Align centre");
    assert_eq!(style_of(&mut ed, "body").align, Align::Center);

    narrow_until(&mut ed, "Body style");
    ed.click(Role::Button, "Body style").assert_open(Role::Dialog, "Body style");
    ed.click(Role::Button, "Justify");
    assert_eq!(style_of(&mut ed, "body").align, Align::Justify);
    ed.click(Role::Button, "Font").click(Role::MenuItemCheckBox, "Montserrat");
    assert_eq!(style_of(&mut ed, "body").family, "Montserrat");
}

#[test]
fn folding_puts_an_open_menu_away_and_leaves_focus_on_the_page() {
    let mut window = letter();
    window.click(Role::Button, "Font").assert_open(Role::Menu, "Font");
    window.resize(720.0, 600.0).assert_closed(Role::Menu, "Font");
    assert_eq!(window.focused().map(|found| found.role), Some(Role::Document));
}

#[test]
fn code_is_coloured_for_its_language_and_stays_so_as_it_is_typed() {
    let mut ed = Harness::doc("```rust\nlet first = 1;\n```\n\n```\nlet second = 2;\n```\n");
    let keyword = letters_ui::tokens::SYNTAX_KEYWORD.light;
    let keywords = |ed: &mut Harness| {
        let inked = |item: &&Item| match item {
            Item::Glyphs { color, .. } => (color.r, color.g, color.b) == (keyword.r, keyword.g, keyword.b),
            _ => false,
        };
        ed.paint().items.iter().filter(inked).count()
    };
    assert_eq!(keywords(&mut ed), 1, "the block that names a language has its keyword coloured, the other none");

    ed.select("first");
    ed.key("End").key("Enter").type_text("fn main() {}");
    assert_eq!(keywords(&mut ed), 2, "a keyword typed into the block is coloured too");
}

#[test]
fn undo_and_redo_are_offered_when_there_is_something_to_take_back() {
    let mut ed = Harness::doc("Quote.");
    let offered = |ed: &mut Harness| ["Undo", "Redo"].map(|name| !ed.find(Role::Button, name).unwrap().disabled);
    assert_eq!(offered(&mut ed), [false, false]);
    ed.type_text("New");
    assert_eq!(offered(&mut ed), [true, false]);
    ed.click(Role::Button, "Undo").assert_markdown("Quote.");
    assert_eq!(offered(&mut ed), [false, true]);
    ed.click(Role::Button, "Redo").assert_markdown("NewQuote.");
}

#[test]
fn arrow_keys_move_between_the_lines_the_layout_made() {
    let mut window = letter();
    window.ui().select(Selection::caret(Pos::new("p1", 9)));
    // (key, block the caret lands in)
    for (key, block) in
        [("Down", "h2"), ("Down", "p2"), ("Up", "h2"), ("Up", "p1"), ("Mod+End", "p2"), ("Mod+Home", "h1")]
    {
        window.key(key);
        assert_eq!(head(&window).block.as_str(), block, "after {key}");
    }
    window.key("End");
    assert_eq!(head(&window).offset, "Roof repairs".len());
}

#[test]
fn the_ends_of_a_line_and_of_the_document_have_each_systems_keys() {
    let ends = |block: &str| (Pos::new(block, 0), Pos::new(block, block_text(&letter(), block).len()));
    let (line, first, last) = (ends("p1"), ends("h1").0, ends("p2").1);
    // (macOS keys, key, where the caret lands from the middle of the second block)
    let cases = [
        (false, "Home", &line.0),
        (false, "End", &line.1),
        (false, "Mod+Home", &first),
        (false, "Mod+End", &last),
        (true, "Mod+Left", &line.0),
        (true, "Mod+Right", &line.1),
        (true, "Mod+Up", &first),
        (true, "Mod+Down", &last),
        (true, "Home", &first),
        (true, "End", &last),
    ];
    for (mac, key, lands) in cases {
        let mut window = if mac { letter().mac() } else { letter() };
        window.ui().select(Selection::caret(Pos::new("p1", 9)));
        window.key(key);
        assert_eq!(&head(&window), lands, "{key} with {} keys", if mac { "macOS" } else { "standard" });
    }
}

#[test]
fn a_double_click_selects_a_word_and_a_drag_selects_a_stretch() {
    let mut window = letter();
    let (start, end) = (spot(&mut window, "p1", 2), spot(&mut window, "p1", 16));
    let middle = |rect: Rect| Point { x: rect.x, y: rect.y + rect.height / 2.0 };
    window.click_at(middle(start)).click_at(middle(start));
    assert_eq!(selected(&window).trim(), "Harland");

    window.advance(1000).press_at(middle(start)).release_at(middle(end));
    assert_eq!(selected(&window), "rland Roofing ");
}

#[test]
fn a_dark_window_shows_a_dark_page_with_light_text() {
    let mut window = letter();
    window.ui().set_dark(true);
    let page = window.find(Role::Document, "Letters").unwrap().bounds;
    let frame = window.paint();
    let light = |color: &letters_render::Color| u32::from(color.r) + u32::from(color.g) + u32::from(color.b) > 384;
    let paper = frame.items.iter().find_map(|item| match item {
        Item::Fill { rect, color, .. } if *rect == page => Some(*color),
        _ => None,
    });
    assert!(paper.is_some_and(|color| !light(&color)), "the page is dark: {paper:?}");
    let inks: Vec<_> = frame
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Glyphs { origin, color, .. } if page.contains(*origin) => Some(*color),
            _ => None,
        })
        .collect();
    assert!(!inks.is_empty() && inks.iter().all(light), "its text is light: {inks:?}");
}

#[test]
fn markers_open_beside_the_caret_over_a_moment() {
    let inside = Selection::caret(Pos::new("b1", INSIDE));
    let mut settled = marked(true);
    settled.ui().select(inside.clone());
    let end = caret(&mut settled).x;

    let mut window = marked(false);
    window.ui().select(inside);
    let start = caret(&mut window).x;
    assert!(start < end, "the text has yet to make room for the markers: {start} of {end}");
    let now = window.platform.now_ms;
    let next = window.ui().animate(now).expect("a frame is asked for while they open");
    assert!(next - now <= 17, "and soon enough to look smooth: {} ms", next - now);

    let mut seen = vec![start];
    for _ in 0..40 {
        window.advance(8);
        seen.push(caret(&mut window).x);
    }
    assert!(seen.windows(2).all(|pair| pair[0] <= pair[1]), "the caret only slides towards its place: {seen:?}");
    assert!(seen.iter().any(|x| *x > start && *x < end), "by way of the places between: {seen:?}");
    assert_eq!(seen[40], end, "and comes to rest where it shows without motion");
}

#[test]
fn markers_show_at_once_under_reduced_motion_and_while_text_is_composed() {
    // Where the caret shows, some time after it lands inside the bold word.
    let shown = |reduced_motion: bool, composing: bool, wait: u64| {
        let mut window = marked(reduced_motion);
        window.ui().select(Selection::caret(Pos::new("b1", INSIDE)));
        if composing {
            window.input(InputEvent::SetComposition { text: "x".into(), selection: None });
        }
        // The first frame after a change begins what moves.
        window.advance(0).advance(wait);
        caret(&mut window).x
    };
    for (name, reduced_motion, composing) in [("reduced motion", true, false), ("composing", false, true)] {
        let rest = shown(false, composing, 400);
        for wait in [0, 8, 40] {
            assert_eq!(shown(reduced_motion, composing, wait), rest, "{name}: nothing is left to slide, {wait} ms on");
        }
    }
}

#[test]
fn a_narrow_window_keeps_a_headings_markers_in_sight_and_the_outline_off_the_text() {
    for (width, outlined) in [(1280.0, true), (700.0, true), (620.0, false), (400.0, false)] {
        let mut window = letter();
        window.resize(width, 800.0);
        let (page, edge) = (page(&mut window), spot(&mut window, "p2", 0).x);
        window.ui().select(Selection::caret(Pos::new("h2", 2)));
        window.advance(0).advance(400);
        // What is drawn left of the text edge is the heading's markers.
        let hung = |item: &Item| match item {
            Item::Glyphs { origin, .. } if origin.y > page.y && origin.x < edge => Some(origin.x),
            _ => None,
        };
        let hung: Vec<f32> = window.paint().items.iter().filter_map(hung).collect();
        assert!(!hung.is_empty(), "at {width} px the heading's markers hang left of the text");
        assert!(hung.iter().all(|x| *x >= page.x), "at {width} px the window's edge cuts none of them: {hung:?}");
        assert_eq!(window.find(Role::Navigation, "Outline").is_some(), outlined, "the outline at {width} px");
    }
}

#[test]
fn a_scroll_by_part_of_a_pixel_moves_the_page_by_whole_device_pixels() {
    // The device pixels in a logical one, and what a trackpad sends.
    let cases: [(f32, &[f32]); 4] =
        [(1.0, &[10.3, 10.3, -4.6]), (2.0, &[10.3, 0.3, 7.25]), (1.5, &[3.1, 3.1, 3.1]), (1.0, &[0.2; 10])];
    for (scale, deltas) in cases {
        let mut window = long();
        window.ui().resize(WINDOW, scale);
        window.click_at(Point { x: 500.0, y: 300.0 }).paint();
        let start = window.ui().caret_rect().unwrap().y;
        let mut asked = 0.0;
        for delta in deltas {
            window.scroll_at(Point { x: 500.0, y: 400.0 }, *delta).paint();
            asked += delta;
            let moved = (start - window.ui().caret_rect().unwrap().y) * scale;
            assert!((moved - moved.round()).abs() < 0.01, "at scale {scale} the page sits {moved} device pixels down");
            assert!((moved - asked * scale).abs() <= 0.51, "at scale {scale} it follows the {asked} px asked: {moved}");
        }
    }
}

#[test]
fn the_rest_of_a_long_document_is_laid_out_in_idle_time_without_moving_the_page() {
    let mut window = long();
    window.scroll_at(Point { x: 500.0, y: 400.0 }, 3000.0).click_at(Point { x: 500.0, y: 300.0 });
    window.paint();
    let before = window.ui().caret_rect().unwrap();

    let mut steps = 0;
    while window.ui().animate(0) == Some(0) {
        steps += 1;
        assert!(steps < 10_000, "idle work comes to an end");
        window.paint();
    }
    window.paint();
    let after = window.ui().caret_rect().unwrap();
    assert!(steps > 0, "there was text out of sight to lay out");
    assert!((after.y - before.y).abs() <= 1.0, "the line the writer was on stayed put: {before:?} to {after:?}");
}

#[test]
fn a_screen_reader_reads_the_document_and_edits_it() {
    let mut window = letter();
    window.ui().select(Selection::caret(Pos::new("p1", 8)));
    let tree = window.ui().access();
    let node = |id| &tree.nodes.iter().find(|(node, _)| *node == id).expect("a node the tree lists").1;
    let (page_id, page) = tree.nodes.iter().find(|(_, node)| node.role() == Role::Document).expect("the page");

    let blocks = page.children();
    let roles: Vec<Role> = blocks.iter().map(|id| node(*id).role()).collect();
    assert_eq!(roles, [Role::Heading, Role::Paragraph, Role::Heading, Role::Paragraph]);
    let run = node(blocks[1]).children()[0];
    assert_eq!(node(run).value(), Some("Harland Roofing sent a quote.\n"));
    let caret = page.text_selection().expect("the page reports the caret").focus;
    assert_eq!((caret.node, caret.character_index), (run, 8));

    let ask =
        |action, data| ActionRequest { action, target_tree: TreeId::ROOT, target_node: *page_id, data: Some(data) };
    let at = |character_index| TextPosition { node: run, character_index };
    let word = TextSelection { anchor: at(0), focus: at(7) };
    window.ask(ask(Action::SetTextSelection, ActionData::SetTextSelection(word)));
    assert_eq!(selected(&window), "Harland");
    window.ask(ask(Action::ReplaceSelectedText, ActionData::Value("Hartley".into())));
    assert_eq!(block_text(&window, "p1"), "Hartley Roofing sent a quote.");
}

/// The document as a screen reader walks it under the page: each node's role and what it says, set in by
/// how deep it is.
fn read(update: &TreeUpdate) -> Vec<String> {
    fn walk(update: &TreeUpdate, id: NodeId, depth: usize, out: &mut Vec<String>) {
        let node = &update.nodes.iter().find(|(node, _)| *node == id).expect("a node the tree lists").1;
        let ticked = node.toggled().map(|toggled| if toggled == Toggled::True { "ticked" } else { "unticked" });
        let says = [node.label(), node.value().map(str::trim_end), ticked].into_iter().flatten();
        let line = [format!("{:?}", node.role())].into_iter().chain(says.map(str::to_owned)).collect::<Vec<_>>();
        out.push(format!("{}{}", "  ".repeat(depth), line.join(" ")));
        node.children().iter().for_each(|child| walk(update, *child, depth + 1, out));
    }
    let page = update.nodes.iter().find(|(_, node)| node.role() == Role::Document).expect("the page");
    let mut out = Vec::new();
    page.1.children().iter().for_each(|child| walk(update, *child, 0, &mut out));
    out
}

/// Two bullets with one more inside the first, a numbered step, two tasks and a figure.
fn shopping() -> Harness {
    let item = |id: &str, list, depth, words: &str| text(id, BlockKind::ListItem { list, depth }, words);
    let figure = BlockKind::Figure {
        asset: None,
        src: Some("https://example.com/roof.png".into()),
        alt: "A slate roof".into(),
        width: 300.0,
        height: 200.0,
        numbered: false,
    };
    let blocks = vec![
        item("a", ListKind::Bullet, 0, "Slates"),
        item("b", ListKind::Bullet, 1, "Welsh"),
        item("c", ListKind::Bullet, 0, "Nails"),
        item("d", ListKind::Ordered, 0, "Strip"),
        item("e", ListKind::Task { checked: false }, 0, "Order"),
        item("f", ListKind::Task { checked: true }, 0, "Measure"),
        text("g", figure, "The roof today"),
    ];
    Harness::open(Document { blocks, ..Document::default() })
}

#[test]
fn a_screen_reader_finds_lists_ticks_and_pictures_as_what_they_are() {
    let mut window = shopping();
    window.ui().select(Selection::caret(Pos::new("a", 2)));
    let tree = window.ui().access();
    let expected = [
        "List",
        "  ListItem",
        "    TextRun Slates",
        "    List",
        "      ListItem",
        "        TextRun Welsh",
        "  ListItem",
        "    TextRun Nails",
        "List",
        "  ListItem",
        "    TextRun Strip",
        "List",
        "  ListItem",
        "    CheckBox unticked",
        "    TextRun Order",
        "  ListItem",
        "    CheckBox ticked",
        "    TextRun Measure",
        "Figure",
        "  Image A slate roof",
        "  TextRun The roof today",
    ];
    assert_eq!(read(&tree), expected);

    // A click on a tick ticks its task, and leaves the caret where the writer has it.
    let tick = tree.nodes.iter().find(|(_, node)| node.role() == Role::CheckBox).expect("a tick").0;
    window.ask(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node: tick, data: None });
    let ticked = |window: &Harness, id: &str| window.editor().doc().block(&id.into()).map(|block| block.kind.clone());
    assert!(matches!(ticked(&window, "e"), Some(BlockKind::ListItem { list: ListKind::Task { checked: true }, .. })));
    assert_eq!(head(&window), Pos::new("a", 2));
}

#[test]
fn a_key_typed_in_a_long_document_tells_a_screen_reader_of_its_block_alone() {
    // What the tree says of the letter's lines.
    let lines = |update: &TreeUpdate| {
        let said = update.nodes.iter().filter_map(|(_, node)| node.value().map(str::to_owned));
        said.filter(|said| said.contains("line of the letter")).collect::<Vec<_>>()
    };
    let mut window = long();
    window.ui().select(Selection::caret(Pos::new("p3", 0)));
    assert_eq!(lines(&window.ui().access()).len(), 200, "one that starts to listen is told all of it");
    window.type_text("x");
    assert_eq!(lines(&window.ui().access_changes()), ["xOne more line of the letter.\n"]);
    window.key("Right");
    assert_eq!(lines(&window.ui().access_changes()), [""; 0], "the caret moves with nothing told again");
    assert_eq!(lines(&window.ui().access()).len(), 200, "and one that starts later is told all of it too");
}

#[test]
fn misspelled_words_are_underlined_once_the_window_has_a_moment() {
    let mut window = Harness::doc("The quik brown fox.\n\nA plain line with nothing wrong.");
    rest(&mut window);
    let [(start, end)] = waves(&mut window)[..] else { panic!("one word is wrong, and one is marked") };
    let (from, to) = (spot(&mut window, "b1", 4), spot(&mut window, "b1", 8));
    assert!((start.x - from.x).abs() < 1.5 && (end.x - to.x).abs() < 1.5, "the line runs under the word");
    assert!(start.y > from.y && start.y < from.y + from.height + 4.0, "at the foot of its letters");
}

#[test]
fn a_long_document_is_checked_where_the_reader_is_first() {
    let lines = (0..400).map(|n| text(&format!("p{n}"), BlockKind::Paragraph, "One more lnie of the letter."));
    let mut window = Harness::open(Document { blocks: lines.collect(), ..Document::default() });
    let middle = page(&mut window);
    let middle = Point { x: middle.x + middle.width / 2.0, y: middle.y + middle.height / 2.0 };
    window.scroll_at(middle, 1e6);

    let mut steps = 0;
    while waves(&mut window).is_empty() {
        assert_eq!(window.ui().animate(0), Some(0), "there is still work to do");
        steps += 1;
        assert!(steps < 10_000, "the words in view are marked");
    }
    window.scroll_at(middle, -1e6);
    assert_eq!(waves(&mut window), [], "the top of the document waits its turn");
    rest(&mut window);
    assert!(!waves(&mut window).is_empty(), "which comes");
}

#[test]
fn a_word_is_not_called_wrong_while_it_is_being_typed() {
    // What the writer does, and how many words are then marked.
    let steps = [
        ("helo", 0, "a word still being typed is left alone"),
        (" ", 1, "it is marked once the caret has left it"),
        ("Backspace", 0, "going back into it takes the mark off"),
        ("Backspace", 0, ""),
        ("lo", 0, "and mending it leaves nothing to mark"),
        (" wrld", 0, ""),
        ("Home", 1, "a caret that leaves by a key marks the word as well"),
    ];
    let mut window = Harness::doc("Say");
    window.ui().select(Selection::caret(Pos::new("b1", 3)));
    window.type_text(" ");
    for (step, marked, why) in steps {
        match step {
            "Backspace" | "Home" => window.key(step),
            text => window.type_text(text),
        };
        assert_eq!(waves(&mut window).len(), marked, "after {step:?} in {:?}: {why}", window.markdown());
    }
}

#[test]
fn the_menu_on_a_misspelled_word_puts_a_correction_in_its_place() {
    let mut window = misspelled();
    ask_about_first_wave(&mut window);
    window.assert_open(Role::Menu, "Spelling").click(Role::MenuItem, "quick");
    window.assert_markdown("The quick brown fox.").assert_closed(Role::Menu, "Spelling");
    assert!(waves(&mut window).is_empty(), "nothing is left to mark");
    assert!(window.focused().is_some_and(|found| found.role == Role::Document), "the keyboard is back on the page");

    window.key("Mod+Z").assert_markdown("The quik brown fox.");
    assert_eq!(waves(&mut window).len(), 1, "one undo takes the correction back, mark and all");
}

#[test]
fn the_menu_opens_for_a_marked_word_alone_by_pointer_or_the_menu_key() {
    // Where in "The quik brown fox." the menu is asked for, and whether there is one to open.
    for (offset, opens) in [(6, true), (8, true), (1, false), (12, false)] {
        for by in ["pointer", "menu key"] {
            let mut window = misspelled();
            let at = spot(&mut window, "b1", offset);
            match by {
                "pointer" => window.secondary_click_at(Point { x: at.x, y: at.y + at.height / 2.0 }),
                _ => window.menu_key(),
            };
            assert_eq!(window.find(Role::Menu, "Spelling").is_some(), opens, "at {offset} by {by}");
            window.key("Escape").assert_closed(Role::Menu, "Spelling");
            assert!(window.focused().is_some_and(|found| found.role == Role::Document), "at {offset} by {by}");
        }
    }

    let mut window = Harness::doc("It ends in quik");
    rest(&mut window);
    let end = spot(&mut window, "b1", 15);
    window.secondary_click_at(Point { x: end.x + 200.0, y: end.y + end.height / 2.0 });
    window.assert_closed(Role::Menu, "Spelling");
}

#[test]
fn a_word_is_accepted_from_the_menu_or_from_the_persons_own_list() {
    // How the word comes to be accepted, and whether the shell is handed it to keep.
    for (route, kept) in [("Add to my dictionary", true), ("Add to this document", false), ("the shell's list", false)]
    {
        let mut window = Harness::doc("Ask Zorvex for a quote.\n\nZorvex replied.");
        rest(&mut window);
        assert_eq!(waves(&mut window).len(), 2, "{route}: the name is marked wherever it stands");
        match route {
            "the shell's list" => window.ui().set_user_words(["Zorvex"]),
            item => {
                ask_about_first_wave(&mut window);
                window.click(Role::MenuItem, item);
                assert_eq!(waves(&mut window).len(), 1, "{route}: its own block is cleared at once");
            }
        }
        rest(&mut window);
        assert!(waves(&mut window).is_empty(), "{route}: and the rest of the document after it");
        assert_eq!(window.platform.learned, if kept { vec!["Zorvex"] } else { vec![] }, "{route}");
        window.assert_markdown("Ask Zorvex for a quote.\n\nZorvex replied.");

        // A word added to the document is part of it: saved with it, and taken back by undo.
        let in_document = route == "Add to this document";
        assert_eq!(window.editor().doc().dictionary.contains("Zorvex"), in_document, "{route}");
        if in_document {
            window.key("Mod+z");
            rest(&mut window);
            assert_eq!(waves(&mut window).len(), 2, "undo has the name marked again");
        }
    }
}

#[test]
fn a_change_from_outside_shows_on_the_page_without_moving_it() {
    let mut window = long();
    window.scroll_at(Point { x: 500.0, y: 400.0 }, 3000.0).click_at(Point { x: 500.0, y: 300.0 });
    let before = caret(&mut window);

    // Another writer makes the first paragraph several lines long, far above what is in view.
    let mut doc = window.editor().doc().clone();
    doc.blocks[0] = text("p0", BlockKind::Paragraph, &"One more line of the letter. ".repeat(40));
    window.ui().change(|editor| editor.rewrite(doc)).expect("the document is taken in");
    assert!(block_text(&window, "p0").len() > 1000, "the editor holds the change");
    let after = caret(&mut window);
    assert!((after.y - before.y).abs() <= 1.0, "the line the writer is on stays put: {before:?} to {after:?}");
}

#[test]
fn the_landing_card_lists_recent_documents_and_asks_the_shell_for_one() {
    let mut window = blank();
    assert!(!window.ui().landing(), "no card until the shell offers one");
    window.ui().set_landing(Some(landing()));
    window.assert_open(Role::Dialog, "Open a document").assert_open(Role::ListBox, "Recent documents");
    let listed: Vec<String> = window
        .controls()
        .into_iter()
        .filter(|found| found.role == Role::ListBoxOption)
        .map(|found| found.name)
        .collect();
    assert_eq!(listed, ["Doc 1", "Doc 2", "Doc 3", "Doc 4", "Doc 5", "Doc 6"], "the six opened last");

    let first = window.focused().expect("the card takes the keyboard");
    assert_eq!((first.name.as_str(), first.description.as_deref()), ("Doc 1", Some("Family / House, 2 minutes ago")));
    // How a document is asked for, and the request the shell then finds.
    let asks = [
        ("Enter on the newest", Request::Open("/family/doc-1".into())),
        ("Down and Enter", Request::Open("/family/doc-2".into())),
        ("a click", Request::Open("/family/doc-5".into())),
        ("the Open button", Request::OpenDialog),
    ];
    for (ask, request) in asks {
        match ask {
            "Enter on the newest" => window.key("Enter"),
            "Down and Enter" => window.key("Down").key("Enter"),
            "a click" => window.click(Role::ListBoxOption, "Doc 5"),
            _ => window.click(Role::Button, "Open"),
        };
        assert_eq!(window.ui().take_requests(), [request], "{ask}");
        assert!(window.ui().landing(), "{ask}: the card stays until the shell has the document");
    }
}

#[test]
fn writing_starts_on_the_page_behind_the_landing_card() {
    for (start, recents) in [("a click on the page", 7), ("typing", 7), ("New document", 0), ("Escape", 0)] {
        let mut window = blank();
        window.ui().set_landing(Some(Landing { recents: landing().recents[..recents].to_vec(), ..landing() }));
        assert_eq!(window.find(Role::ListBox, "Recent documents").is_some(), recents > 0, "{start}");
        match start {
            "a click on the page" => window.click_at(Point { x: 80.0, y: 700.0 }),
            "typing" => window.type_text("D"),
            "New document" => window.click(Role::Button, "New document"),
            _ => window.key("Escape"),
        };
        assert!(!window.ui().landing(), "{start}: the card is gone");
        window.assert_closed(Role::Dialog, "Open a document");
        assert_eq!(
            window.focused().map(|found| found.role),
            Some(Role::Document),
            "{start}: the page has the keyboard"
        );
        window.type_text("ear Ada");
        window.assert_markdown(if start == "typing" { "Dear Ada" } else { "ear Ada" });
        assert_eq!(window.ui().take_requests(), [], "{start}: this window's document is the new one");
    }
}

#[test]
fn a_document_without_a_name_takes_its_first_heading_once_the_caret_leaves_it() {
    let mut window = blank();
    window.ui().select(Selection::caret(Pos::new("p", 0)));
    window.assert_open(Role::Button, "Untitled");
    window.type_text("# Cottage renov");
    window.assert_open(Role::Button, "Cottage renov").assert_closed(Role::Button, "Untitled");
    assert_eq!(window.ui().take_requests(), [], "nothing is asked while the heading is being typed");

    window.type_text("ation").key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::Named("Cottage renovation".into())]);

    // Once it has its name, the heading can change without the name following.
    window.ui().set_document_name("Cottage renovation");
    window.key("Up").type_text("The ").key("Down");
    window.assert_open(Role::Button, "Cottage renovation");
    assert_eq!(window.ui().take_requests(), [], "the name is taken once");
}

#[test]
fn the_bar_says_how_saving_stands_and_where_the_first_write_went() {
    let mut window = letter();
    // What the shell reports, and what the bar then says: nothing, for a document not yet written.
    let states = [
        (SaveState::Unsaved, None),
        (SaveState::Saving, Some("Saving")),
        (SaveState::Failed, Some("Not saved")),
        (SaveState::Saved, Some("Saved")),
    ];
    let bar = bar_controls(&mut window);
    for (state, said) in states {
        window.ui().set_save_state(state);
        let shown: Vec<&str> = ["Saving", "Not saved", "Saved"]
            .into_iter()
            .filter(|label| window.find(Role::Label, label).is_some())
            .collect();
        assert_eq!(shown, Vec::from_iter(said), "{state:?}");
        assert_eq!(bar_controls(&mut window), bar, "nothing in the bar moves when the document is {state:?}");
    }

    // Where the first write went is told under the icon for a moment, and the bar stays as it is.
    window.ui().set_saved_in("House");
    window.assert_open(Role::Label, "Saved in House").assert_open(Role::Tooltip, "Saved in House");
    assert_eq!(bar_controls(&mut window), bar);
    window.advance(0).advance(2000).assert_open(Role::Tooltip, "Saved in House");
    window.advance(3000).assert_closed(Role::Tooltip, "Saved in House").assert_open(Role::Label, "Saved");
}

#[test]
fn the_more_menu_asks_the_shell_for_a_new_document_or_another_one() {
    for (item, request) in [("New document", Request::New), ("Open", Request::OpenDialog)] {
        let mut window = letter();
        window.click(Role::Button, "More").click(Role::MenuItem, item);
        assert_eq!(window.ui().take_requests(), [request], "{item}");
    }
}

/// Where the cottage's plan is kept: in House, in the space called Family. Holidays holds a folder too.
fn family() -> Location {
    let folder = |path: &str| Folder {
        path: path.into(),
        name: path.rsplit('/').next().unwrap_or_default().into(),
        depth: path.matches('/').count() as u8 - 1,
    };
    let folders = [
        "/Family",
        "/Family/House",
        "/Family/House/Quotes",
        "/Family/House/Photos",
        "/Family/Holidays",
        "/Family/Holidays/2026",
        "/Family/School",
    ];
    Location {
        name: Some("Cottage renovation".into()),
        folder: "/Family/House".into(),
        folders: folders.map(folder).into(),
    }
}

/// The letter, kept where [`family`] says, with its name open to be typed over.
fn renaming() -> Harness {
    let mut window = letter();
    window.ui().set_location(family());
    window.click(Role::Button, "Cottage renovation");
    window
}

/// The folders the flyout shows, from the top.
fn folders(window: &mut Harness) -> Vec<String> {
    window.controls().into_iter().filter(|found| found.role == Role::TreeItem).map(|found| found.name).collect()
}

/// The role, name and value of what the keyboard's focus is on.
fn focus(window: &mut Harness) -> (Role, String, String) {
    let focused = window.focused().expect("something has focus");
    (focused.role, focused.name, focused.value.unwrap_or_default())
}

fn click_the_page(window: &mut Harness) {
    let page = page(window);
    window.click_at(Point { x: page.x + page.width / 2.0, y: page.y + page.height - 40.0 });
}

#[test]
fn the_name_in_the_bar_is_typed_over_and_the_shell_is_asked_for_it() {
    let mut window = renaming();
    assert_eq!(focus(&mut window), (Role::TextInput, "Document name".into(), "Cottage renovation".into()));
    window.type_text("Cottage plan").key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::Rename("Cottage plan".into())], "typing replaces the old name");
    window.assert_closed(Role::TextInput, "Document name");

    // The bar shows the name the shell answers with, and writing goes on in the document.
    window.ui().set_location(Location { name: Some("Cottage plan".into()), ..family() });
    window.assert_open(Role::Button, "Cottage plan");
    window.type_text("!");
    assert!(window.markdown().contains('!'), "{}", window.markdown());
}

#[test]
fn a_typed_name_is_taken_on_enter_or_a_press_elsewhere_and_dropped_on_escape() {
    type Leave = fn(&mut Harness);
    let cases: [(&str, &str, Leave, bool); 5] = [
        ("Enter", "Roof quotes", |window| _ = window.key("Enter"), true),
        ("a press on the page", "Roof quotes", click_the_page, true),
        ("Escape", "Roof quotes", |window| _ = window.key("Escape"), false),
        ("Escape among the folders", "Roof quotes", |window| _ = window.key("Tab").key("Escape"), false),
        ("Enter on the name as it was", "Cottage renovation", |window| _ = window.key("Enter"), false),
    ];
    for (leave, name, act, taken) in cases {
        let mut window = renaming();
        window.type_text(name);
        act(&mut window);
        let asked = if taken { vec![Request::Rename(name.into())] } else { Vec::new() };
        assert_eq!(window.ui().take_requests(), asked, "{leave}");
        window.assert_closed(Role::TextInput, "Document name").assert_closed(Role::Tree, "Move to");
    }
}

#[test]
fn a_name_the_shell_refuses_comes_back_to_be_changed() {
    let mut window = renaming();
    window.type_text("Roof quotes").key("Enter");
    window.ui().take_requests();
    let why = "There is already a document called \u{201c}Roof quotes\u{201d} in that folder.";
    window.ui().set_notice(Some(why));
    window.assert_open(Role::Alert, why);
    assert_eq!(focus(&mut window), (Role::TextInput, "Document name".into(), "Roof quotes".into()));

    // Another name takes the notice away, and is asked for in turn.
    window.type_text(", second round");
    window.assert_closed(Role::Alert, why);
    window.key("Enter");
    let asked = window.ui().take_requests();
    assert!(matches!(&asked[..], [Request::Rename(name)] if name.ends_with(", second round")), "{asked:?}");
}

#[test]
fn the_flyout_lists_the_folders_and_moves_the_document_to_the_one_picked() {
    let mut window = renaming();
    assert_eq!(folders(&mut window), ["Family", "House", "Quotes", "Photos", "Holidays", "School"]);
    let here = window.find(Role::TreeItem, "House").expect("the folder it is in");
    assert_eq!(here.description.as_deref(), Some("It is here"));
    let nowhere = |window: &mut Harness| window.find(Role::Button, "Move").is_some_and(|button| button.disabled);
    assert!(nowhere(&mut window), "there is nowhere to move to until a folder is picked");

    // The folder it is in already is nowhere to move to.
    window.click(Role::TreeItem, "Quotes").assert_open(Role::Button, "Move to Quotes");
    window.click(Role::TreeItem, "House");
    assert!(nowhere(&mut window));

    window.click(Role::TreeItem, "Quotes").click(Role::Button, "Move to Quotes");
    assert_eq!(window.ui().take_requests(), [Request::Move("/Family/House/Quotes".into())]);
    window.assert_closed(Role::Tree, "Move to").assert_closed(Role::TextInput, "Document name");

    let mut window = renaming();
    window.click(Role::Button, "Other location\u{2026}");
    assert_eq!(window.ui().take_requests(), [Request::MoveElsewhere]);
    window.assert_closed(Role::Tree, "Move to");
}

#[test]
fn the_folders_are_opened_and_picked_from_the_keyboard() {
    let mut window = renaming();
    window.key("Tab");
    assert_eq!(focus(&mut window).1, "Family", "Tab goes from the name to the folders");
    for _ in 0..4 {
        window.key("Down");
    }
    assert_eq!(focus(&mut window).1, "Holidays");
    window.key("Right");
    assert!(folders(&mut window).contains(&"2026".to_owned()), "Right opens a folder");
    window.key("Left");
    assert!(!folders(&mut window).contains(&"2026".to_owned()), "and Left closes it");

    // Enter picks a folder. The name and the flyout are one round for Tab, and Enter in the name moves too.
    window.key("Right").key("Down").key("Enter");
    window.assert_open(Role::Button, "Move to 2026");
    window.key("Shift+Tab");
    assert_eq!(focus(&mut window).1, "Document name");
    window.key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::Move("/Family/Holidays/2026".into())]);
}

#[test]
fn a_new_folder_is_asked_for_by_name_and_picked_once_the_shell_lists_it() {
    // The folder picked, if any, and the folder the new one then goes into.
    for (picked, parent) in [(Some("Photos"), "/Family/House/Photos"), (None, "/Family/House")] {
        let mut window = renaming();
        if let Some(picked) = picked {
            window.click(Role::TreeItem, picked);
        }
        window.click(Role::Button, "New folder");
        assert_eq!(focus(&mut window).1, "Folder name");
        window.type_text("Before").key("Enter");
        let asked = Request::NewFolder { parent: parent.into(), name: "Before".into() };
        assert_eq!(window.ui().take_requests(), [asked], "{picked:?}");

        let mut location = family();
        let at = location.folders.iter().position(|folder| folder.path == parent).expect("the parent");
        let depth = location.folders[at].depth + 1;
        location.folders.insert(at + 1, Folder { path: format!("{parent}/Before"), name: "Before".into(), depth });
        window.ui().set_location(location);
        window.assert_open(Role::TreeItem, "Before").assert_open(Role::Button, "Move to Before");
    }
}

#[test]
fn a_document_with_no_folders_to_show_is_renamed_from_the_field_alone() {
    let mut window = blank();
    window.click(Role::Button, "Untitled");
    window.assert_closed(Role::Tree, "Move to");
    // A name that is not the document's own is not offered to keep.
    assert_eq!(focus(&mut window), (Role::TextInput, "Document name".into(), String::new()));
    window.type_text("Shopping");
    click_the_page(&mut window);
    assert_eq!(window.ui().take_requests(), [Request::Rename("Shopping".into())]);
    window.assert_closed(Role::TextInput, "Document name");
}

#[test]
fn a_press_under_the_last_block_gives_a_line_to_write_on() {
    // What the document ends in, and the last block once "After" is typed where the press put the caret.
    let cases = [
        ("a code block", "Intro.\n\n```\nlet a = 1;\n```\n", "After"),
        ("a quote", "Intro.\n\n> Quoted.\n", "After"),
        ("body text", "Intro.\n\nLast words.", "Last words.After"),
    ];
    for (ends_in, markdown, last) in cases {
        let mut window = Harness::doc(markdown);
        click_the_page(&mut window);
        window.type_text("After");
        let block = window.editor().doc().blocks.last().expect("the document has blocks");
        assert!(matches!(block.kind, BlockKind::Paragraph), "after {ends_in} comes body text: {:?}", block.kind);
        assert_eq!(block.plain_text(), last, "after {ends_in}");
    }
}

/// The Styles panel's rows, top to bottom: each type's name, its font in words, and whether it is the
/// one the bar shows.
fn style_rows(window: &mut Harness) -> Vec<(String, String, bool)> {
    let panel = window.find(Role::Complementary, "Styles").expect("the Styles panel").bounds;
    let row = |found: letters_test::Found| {
        let open = found.toggled.filter(|_| found.role == Role::Button && panel.contains(found.center()))?;
        Some((found.name, found.description.unwrap_or_default(), open))
    };
    window.controls().into_iter().filter_map(row).collect()
}

/// The types whose rows are marked as the one the bar shows.
fn open_rows(window: &mut Harness) -> Vec<String> {
    style_rows(window).into_iter().filter(|row| row.2).map(|row| row.0).collect()
}

/// The bar's button for the type whose style it shows.
fn type_label(window: &mut Harness, name: &str) -> letters_test::Found {
    let bar = window.find(Role::Toolbar, "Document").expect("the bar").bounds;
    let named =
        |found: &letters_test::Found| found.role == Role::Button && found.name == name && bar.contains(found.center());
    window.controls().into_iter().find(named).unwrap_or_else(|| panic!("the bar does not name {name}"))
}

/// What the window asked of its shell about stylesheets, the latest last.
fn asked(window: &mut Harness) -> Vec<Request> {
    window.ui().take_requests()
}

/// A stylesheet that is not the letter's own style: body text in Fira Code.
fn other_sheet(window: &mut Harness) -> Style {
    let mut sheet = window.ui().style().clone();
    sheet.types.get_mut("body").expect("a style for body text").family = "Fira Code".into();
    sheet
}

#[test]
fn the_styles_panel_lists_each_named_style_with_its_font_and_marks_the_one_at_the_caret() {
    let mut window = letter();
    window.click(Role::Button, "Styles");
    let rows = style_rows(&mut window);
    let names: Vec<&str> = rows.iter().map(|row| row.0.as_str()).collect();
    let expected = ["Title", "Heading 1", "Heading 2", "Heading 3", "Heading 4", "Body", "Quote", "Code", "Caption"];
    assert_eq!(names, expected);
    let font = |rows: &[(String, String, bool)], name: &str| {
        rows.iter().find(|row| row.0 == name).map(|row| row.1.clone()).unwrap_or_default()
    };
    assert!(font(&rows, "Quote").contains("Italic"), "{rows:?}");
    assert!(font(&rows, "Code").contains("Fira Code"), "{rows:?}");
    assert_eq!(open_rows(&mut window), ["Heading 1"], "the caret is in a heading");

    // A change made in the bar shows in the row, and the mark follows the caret.
    window.click(Role::Button, "Font").click(Role::MenuItemCheckBox, "Fira Code");
    assert!(font(&style_rows(&mut window), "Heading 1").contains("Fira Code"));
    window.ui().select(Selection::caret(Pos::new("p1", 0)));
    assert_eq!(open_rows(&mut window), ["Body"]);
}

#[test]
fn a_row_of_the_panel_has_the_bar_restyle_a_type_the_caret_is_not_in() {
    let mut window = letter();
    window.click(Role::Button, "Styles").click(Role::Button, "Caption");
    type_label(&mut window, "Caption");
    assert_eq!(open_rows(&mut window), ["Caption"]);

    window.click(Role::Button, "Align centre");
    assert_eq!(style_of(&mut window, "caption").align, Align::Center);
    assert_eq!(style_of(&mut window, "heading1").align, Align::Start, "the type at the caret keeps its style");
    type_label(&mut window, "Caption");

    // The bar goes back to the caret once it moves.
    window.ui().select(Selection::caret(Pos::new("p1", 0)));
    type_label(&mut window, "Body");
    assert_eq!(open_rows(&mut window), ["Body"]);
}

#[test]
fn a_style_that_is_not_the_persons_default_can_be_reset_to_it_or_saved_as_it() {
    let mut window = letter();
    let default = window.ui().style().clone();
    window.ui().set_default_style(Some(default.clone()));
    window.click(Role::Button, "Styles");
    window.assert_closed(Role::Label, "Differs from your default");
    assert!(!type_label(&mut window, "Heading 1").description.unwrap_or_default().contains("default"));

    window.click(Role::Button, "Align centre");
    window.assert_open(Role::Label, "Differs from your default");
    let label = type_label(&mut window, "Heading 1");
    assert!(label.description.unwrap_or_default().contains("Differs from your default"), "the bar says so too");

    window.click(Role::Button, "Reset to my default");
    assert_eq!(window.ui().style(), &default);
    window.assert_closed(Role::Label, "Differs from your default");
    window.click(Role::Button, "Undo");
    assert_ne!(window.ui().style(), &default, "undo takes the reset back");

    window.click(Role::Button, "Save as my default");
    let saved = asked(&mut window).into_iter().find_map(|request| match request {
        Request::SaveDefaultStyle(json) => Some(json),
        _ => None,
    });
    let saved = Style::from_json(&saved.expect("the shell is asked to keep it")).expect("a style.json");
    assert_eq!(&saved, window.ui().style());
    window.assert_closed(Role::Label, "Differs from your default");
}

#[test]
fn the_bars_type_opens_the_styles_panel() {
    let mut window = letter();
    let label = type_label(&mut window, "Heading 1");
    window.click_at(label.center());
    window.assert_open(Role::Complementary, "Styles");
    window.click_at(label.center());
    window.assert_open(Role::Complementary, "Styles");
}

#[test]
fn a_stylesheet_is_shown_as_it_will_look_before_it_is_applied_in_one_step() {
    let mut window = letter();
    let (before, sheet) = (window.ui().style().clone(), other_sheet(&mut window));
    window.click(Role::Button, "Styles").click(Role::Button, "Import");
    assert!(asked(&mut window).contains(&Request::ImportStyle), "the shell is asked for a file");

    window.ui().offer_stylesheet("travel.letstyle", &sheet.to_json());
    window.assert_open(Role::Label, "travel.letstyle");
    let shows = |window: &mut Harness, font: &str| {
        window.controls().iter().any(|found| found.role == Role::Label && found.name.starts_with(font))
    };
    assert!(shows(&mut window, "Fira Code, 11"), "body text is listed as the stylesheet has it");
    assert_eq!(window.ui().style(), &before, "nothing has changed yet");

    window.click(Role::Button, "Apply");
    assert_eq!(window.ui().style(), &sheet);
    window.assert_closed(Role::Label, "travel.letstyle");
    window.click(Role::Button, "Undo");
    assert_eq!(window.ui().style(), &before, "the whole stylesheet is one step");
}

#[test]
fn a_stylesheet_on_offer_leaves_the_document_alone_unless_it_is_applied() {
    // The button pressed, and whether the shell is then asked to keep the stylesheet as the default.
    for (button, kept) in [("Cancel", false), ("Save as my default", true)] {
        let mut window = letter();
        let (before, sheet) = (window.ui().style().clone(), other_sheet(&mut window));
        window.ui().offer_stylesheet("travel.letstyle", &sheet.to_json());
        window.assert_open(Role::Complementary, "Styles");
        window.click(Role::Button, button);

        assert_eq!(window.ui().style(), &before, "{button}");
        window.assert_closed(Role::Label, "travel.letstyle");
        let saved = asked(&mut window).into_iter().find_map(|request| match request {
            Request::SaveDefaultStyle(json) => Style::from_json(&json).ok(),
            _ => None,
        });
        assert_eq!(saved, kept.then_some(sheet), "{button}");
    }
}

#[test]
fn a_file_that_is_no_stylesheet_is_refused_and_the_panel_says_why() {
    let mut window = letter();
    let before = window.ui().style().clone();
    window.ui().offer_stylesheet("notes.txt", "Dear Harland,");
    let alert = window.controls().into_iter().find(|found| found.role == Role::Alert).expect("the panel says why");
    assert!(alert.name.contains("notes.txt"), "{}", alert.name);
    assert!(window.find(Role::Button, "Apply").is_none(), "there is nothing to apply");
    assert_eq!(window.ui().style(), &before);

    // The next file is judged on its own.
    let sheet = other_sheet(&mut window);
    window.ui().offer_stylesheet("travel.letstyle", &sheet.to_json());
    assert!(window.controls().iter().all(|found| found.role != Role::Alert));
}

#[test]
fn export_hands_the_shell_the_documents_style_as_a_stylesheet() {
    let mut window = letter();
    window.click(Role::Button, "Align centre").click(Role::Button, "Styles").click(Role::Button, "Export");
    let sheet = asked(&mut window).into_iter().find_map(|request| match request {
        Request::ExportStyle(json) => Some(json),
        _ => None,
    });
    let sheet = Style::from_json(&sheet.expect("the shell is asked to write it")).expect("a stylesheet");
    assert_eq!(&sheet, window.ui().style());
}

#[test]
fn the_paper_and_its_margins_are_set_in_the_panel() {
    let mut window = letter();
    window.click(Role::Button, "Styles");
    let a4 = window.ui().style().page.clone();
    assert_eq!(window.find(Role::Button, "Paper").unwrap().description.as_deref(), Some("A4, portrait"));

    window.click(Role::Button, "Paper").click(Role::MenuItemCheckBox, "Letter");
    let letter = window.ui().style().page.clone();
    assert!(letter.width > a4.width && letter.height < a4.height, "US Letter is wider and shorter than A4");
    window.click(Role::Button, "Paper").click(Role::MenuItemCheckBox, "Landscape");
    let turned = window.ui().style().page.clone();
    assert_eq!((turned.width, turned.height), (letter.height, letter.width));
    assert_eq!(window.find(Role::Button, "Paper").unwrap().description.as_deref(), Some("Letter, landscape"));

    // Margins are typed in millimetres, and the paper has to have room for them.
    for (typed, taken) in [("10", true), ("500", false), ("wide", false)] {
        let before = window.ui().style().page.clone();
        window.click(Role::TextInput, "Margins").key("Mod+A").type_text(typed).key("Enter");
        let page = window.ui().style().page.clone();
        assert_eq!(page != before, taken, "{typed:?}");
        assert_eq!((page.width, page.height), (before.width, before.height), "the paper is as it was");
        if taken {
            let sides = [page.margin_top, page.margin_right, page.margin_bottom, page.margin_left];
            assert!(sides.iter().all(|side| (side - 28.35).abs() < 0.1), "10 mm on every side: {sides:?}");
        }
        window.key("Escape");
    }

    window.click(Role::Button, "Undo");
    assert_eq!(window.ui().style().page.margin_top, turned.margin_top, "page setup is undone like any edit");
}

/// The letter in a window that Letters frames itself, as on Linux: it floats, with all three buttons at the
/// end of the bar.
fn framed() -> Harness {
    let mut window = letter();
    let buttons = vec![WindowButton::Minimise, WindowButton::Maximise, WindowButton::Close];
    window.ui().set_chrome(Chrome { buttons, frame: Frame::Floating, ..Chrome::default() });
    window
}

/// A point of the bar with no control under it.
fn free_part_of_the_bar(window: &mut Harness) -> Point {
    let bar = window.find(Role::Toolbar, "Document").expect("the bar").bounds;
    let share = window.find(Role::Button, "Share").expect("the Share button").bounds;
    let at = Point { x: share.x - 24.0, y: bar.y + bar.height / 2.0 };
    let controls = bar_controls(window);
    assert!(controls.iter().all(|control| !control.1.contains(at)), "{at:?} is on a control: {controls:?}");
    at
}

/// A point a little way from another, further than a click may wander.
fn dragged(from: Point) -> Point {
    Point { x: from.x + 12.0, y: from.y + 3.0 }
}

#[test]
fn the_bar_shows_the_windows_buttons_where_the_desktop_keeps_them() {
    use WindowButton::{Close, Maximise, Minimise};
    // The desktop's buttons, whether it keeps them at the start, whether the window is maximised, and the
    // names the bar then shows in order.
    let cases: [(&[WindowButton], bool, bool, &[&str]); 5] = [
        (&[Minimise, Maximise, Close], false, false, &["Minimise", "Maximise", "Close window"]),
        (&[Close], false, false, &["Close window"]),
        (&[Close, Minimise, Maximise], true, false, &["Close window", "Minimise", "Maximise"]),
        (&[Minimise, Maximise, Close], false, true, &["Minimise", "Restore", "Close window"]),
        (&[], false, false, &[]),
    ];
    for (buttons, at_start, maximized, expected) in cases {
        let mut window = letter();
        let frame = if maximized { Frame::Filled } else { Frame::Floating };
        let chrome =
            Chrome { buttons: buttons.to_vec(), buttons_at_start: at_start, maximized, frame, ..Chrome::default() };
        window.ui().set_chrome(chrome);
        let names: Vec<String> = bar_controls(&mut window).into_iter().map(|control| control.0).collect();
        let shown = if at_start { &names[..expected.len()] } else { &names[names.len() - expected.len()..] };
        assert_eq!(shown, expected, "at the start: {at_start}, of {names:?}");
        let window_buttons = ["Minimise", "Maximise", "Restore", "Close window"];
        let count = names.iter().filter(|name| window_buttons.contains(&name.as_str())).count();
        assert_eq!(count, expected.len(), "{names:?}");
    }
}

#[test]
fn the_windows_buttons_ask_the_shell() {
    let cases =
        [("Minimise", Request::Minimise), ("Maximise", Request::ToggleMaximise), ("Close window", Request::Close)];
    for (button, expected) in cases {
        let mut window = framed();
        window.click(Role::Button, button);
        assert_eq!(window.ui().take_requests(), [expected], "{button}");
    }
}

#[test]
fn a_drag_on_a_title_bar_moves_the_window_from_its_free_parts_and_its_name_alone() {
    let mut window = framed();
    window.ui().set_document_name("Quote");
    let free = free_part_of_the_bar(&mut window);
    let name = window.find(Role::Button, "Quote").expect("the name").center();
    let bold = window.find(Role::Button, "Bold").expect("the Bold button").center();
    // Where the press starts, whether it moves before it is let go, and what the shell is asked.
    let cases = [
        (free, true, vec![Request::DragWindow]),
        (free, false, vec![]),
        (name, true, vec![Request::DragWindow]),
        (name, false, vec![]),
        (bold, true, vec![]),
    ];
    for (from, moves, expected) in cases {
        window.advance(1000).press_at(from).release_at(if moves { dragged(from) } else { from });
        assert_eq!(window.ui().take_requests(), expected, "from {from:?}, moving: {moves}");
        // Only a click on the name opens it to be typed over.
        let renaming = window.find(Role::TextInput, "Document name").is_some();
        assert_eq!(renaming, from == name && !moves, "from {from:?}, moving: {moves}");
        window.key("Escape");
    }
}

#[test]
fn a_bar_under_the_systems_title_bar_moves_nothing() {
    let mut window = letter();
    let free = free_part_of_the_bar(&mut window);
    window.press_at(free).release_at(dragged(free)).click_at(free).click_at(free).secondary_click_at(free);
    assert_eq!(window.ui().take_requests(), []);
}

#[test]
fn a_double_click_on_a_title_bar_maximises_and_a_secondary_press_asks_for_the_window_menu() {
    let mut window = framed();
    let free = free_part_of_the_bar(&mut window);
    window.click_at(free).click_at(free);
    assert_eq!(window.ui().take_requests(), [Request::ToggleMaximise]);
    window.advance(1000).secondary_click_at(free);
    assert_eq!(window.ui().take_requests(), [Request::WindowMenu(free)]);

    // A control keeps its presses to itself.
    window.advance(1000).double_click(Role::Button, "Bold");
    let bold = window.find(Role::Button, "Bold").unwrap().center();
    window.secondary_click_at(bold);
    assert_eq!(window.ui().take_requests(), []);
}

#[test]
fn a_floating_window_is_resized_from_the_grip_inside_its_edge() {
    let mut window = framed();
    let before = head(&window);
    let cases = [
        (Point { x: 2.0, y: 300.0 }, Edge::West),
        (Point { x: WINDOW.width - 2.0, y: 300.0 }, Edge::East),
        (Point { x: 400.0, y: WINDOW.height - 2.0 }, Edge::South),
        (Point { x: WINDOW.width - 3.0, y: WINDOW.height - 3.0 }, Edge::SouthEast),
        (Point { x: 3.0, y: 3.0 }, Edge::NorthWest),
    ];
    for (at, edge) in cases {
        window.press_at(at);
        assert_eq!(window.ui().take_requests(), [Request::ResizeWindow(edge)], "{at:?}");
        window.release_at(at);
        assert_eq!(window.ui().cursor(), Cursor::Resize(edge), "the pointer shows it at {at:?}");
    }
    assert_eq!(head(&window), before, "a press on the grip is not a press on the page under it");

    // A window that fills its place has no edge to take hold of.
    let buttons = vec![WindowButton::Close];
    window.ui().set_chrome(Chrome { buttons, frame: Frame::Filled, maximized: true, ..Chrome::default() });
    window.click_at(Point { x: 2.0, y: 300.0 });
    assert_eq!(window.ui().take_requests(), []);
    assert_ne!(window.ui().cursor(), Cursor::Resize(Edge::West));
}

/// The faintest ink of the text and icons drawn in the bar.
fn faintest_in_the_bar(window: &mut Harness) -> u8 {
    let bar = window.find(Role::Toolbar, "Document").expect("the bar").bounds;
    let ink = |item: &Item| match item {
        Item::Glyphs { origin, color, .. } if bar.contains(*origin) => Some(color.a),
        _ => None,
    };
    window.paint().items.iter().filter_map(ink).min().expect("the bar holds text")
}

#[test]
fn a_title_bar_dims_while_its_window_is_behind_another() {
    // A bar under the system's own title bar is no title bar, and stays as it is.
    for (frame, dims) in [(Frame::Floating, true), (Frame::Unified, true), (Frame::System, false)] {
        let mut window = letter();
        window.ui().set_chrome(Chrome { frame, ..Chrome::default() });
        let front = faintest_in_the_bar(&mut window);
        window.ui().focus(false);
        let behind = faintest_in_the_bar(&mut window);
        assert_eq!(behind < front, dims, "{frame:?}: {front} in front, {behind} behind");
        window.ui().focus(true);
        assert_eq!(faintest_in_the_bar(&mut window), front, "{frame:?}: in front again");
    }
}

#[test]
fn a_bar_under_the_systems_buttons_takes_their_height_and_leaves_them_room() {
    let mut window = letter();
    let chrome = Chrome { frame: Frame::Unified, bar_height: Some(52.0), inset_start: 78.0, ..Chrome::default() };
    window.ui().set_chrome(chrome);
    let bar = window.find(Role::Toolbar, "Document").expect("the bar").bounds;
    assert_eq!(bar.height, 52.0);
    assert_eq!(page(&mut window).y, 52.0, "the page starts under it");
    let first = bar_controls(&mut window).into_iter().next().expect("the bar holds controls");
    assert!(first.1.x >= 78.0, "{} starts at {}", first.0, first.1.x);
}

#[test]
fn a_click_on_the_page_shows_the_markers_where_it_lands() {
    let inside = spot(&mut marked(true), "b1", INSIDE);
    let mut window = marked(true);
    // The word has yet to make room for its markers, so it is still a little to the left.
    window.click_at(Point { x: inside.x - 12.0, y: inside.y + inside.height / 2.0 });
    let landed = head(&window);
    assert!((9..=14).contains(&landed.offset), "in the bold word: {landed:?}");
    let shown = spot(&mut marked(true), "b1", landed.offset);
    assert_eq!(caret(&mut window).x, shown.x, "the markers show as they do when the caret is put there");
}

/// The source of the inline formula [`formulas`] writes, and of its formula block.
const INLINE: &str = "c = a + b";
const DISPLAY: &str = "x = a\n+ b";

/// A paragraph `p1` that ends in inline maths with the caret just after it, over a formula block `f1` of
/// two lines. Nothing moves, so a sheet is out as soon as the caret is in a formula.
fn formulas() -> Harness {
    let blocks = vec![
        text("p1", BlockKind::Paragraph, "A rafter has length "),
        text("f1", BlockKind::Formula, DISPLAY),
        text("p2", BlockKind::Paragraph, "Reply by Friday."),
    ];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_reduced_motion(true);
    window.ui().select(Selection::caret(Pos::new("p1", 20)));
    window.type_text(&format!("${INLINE}$"));
    window
}

/// The sheet under a formula, if one is out: its description is the source it shows.
fn sheet(window: &mut Harness) -> Option<letters_test::Found> {
    window.find(Role::Code, "Formula source")
}

/// The source the sheet shows.
fn source(window: &mut Harness) -> Option<String> {
    sheet(window).and_then(|sheet| sheet.description)
}

#[test]
fn the_source_of_a_formula_is_on_a_sheet_under_it_while_the_caret_is_in_it() {
    let mut window = formulas();
    assert_eq!(source(&mut window), None, "the caret is past the formula it closed");
    let line = caret(&mut window);

    window.key("Left");
    assert_eq!(source(&mut window).as_deref(), Some(INLINE));
    let (sheet, caret) = (sheet(&mut window).expect("a sheet").bounds, caret(&mut window));
    assert!(sheet.y >= line.y + line.height - 4.0, "the sheet hangs under the line: {sheet:?} under {line:?}");
    let middle = Point { x: caret.x, y: caret.y + caret.height / 2.0 };
    assert!(sheet.contains(middle), "the caret is on the sheet: {caret:?} in {sheet:?}");

    // What is typed goes into the formula, and the sheet shows it.
    window.type_text("x");
    assert_eq!(block_text(&window, "p1"), format!("A rafter has length {INLINE}x"));
    assert_eq!(source(&mut window), Some(format!("{INLINE}x")));

    // The sheet is for the formula the caret is in, whichever kind.
    window.ui().select(Selection::caret(Pos::new("f1", 2)));
    assert_eq!(source(&mut window).as_deref(), Some(DISPLAY));
}

#[test]
fn the_sheet_goes_when_the_caret_leaves_its_formula() {
    let far = spot(&mut formulas(), "p2", 4);
    // How the caret leaves from the end of the inline formula. None of it adds to the document.
    for way in ["Escape", "Right", "a press elsewhere on the page"] {
        let mut window = formulas();
        window.key("Left");
        let before = window.markdown();
        match way {
            "Escape" | "Right" => window.key(way),
            _ => window.click_at(Point { x: far.x, y: far.y + far.height / 2.0 }),
        };
        assert_eq!(source(&mut window), None, "{way}");
        assert_eq!(window.editor().formula(), None, "{way}");
        assert_eq!(window.markdown(), before, "{way}: the document is as it was");
    }
}

#[test]
fn up_and_down_move_through_the_lines_of_the_source_before_they_leave_the_formula() {
    let mut window = formulas();
    window.ui().select(Selection::caret(Pos::new("f1", 2)));
    window.key("Down");
    assert_eq!(head(&window), Pos::new("f1", 8), "the same column of the second line");
    window.key("End");
    assert_eq!(head(&window), Pos::new("f1", 9));
    window.key("Home");
    assert_eq!(head(&window), Pos::new("f1", 6), "the start of the line of source, not of the formula");
    window.key("Down");
    assert_eq!(head(&window).block.as_str(), "p2", "past the last line the caret is on the page again");
    window.key("Up");
    assert_eq!(head(&window).block.as_str(), "f1");
    assert_eq!(source(&mut window).as_deref(), Some(DISPLAY));
}

#[test]
fn a_press_on_the_sheet_puts_the_caret_there_in_the_source() {
    let mut window = formulas();
    window.key("Left");
    let sheet = sheet(&mut window).expect("a sheet").bounds;
    let at = |part: f32| Point { x: sheet.x + sheet.width * part, y: sheet.y + sheet.height / 2.0 };

    window.click_at(at(0.5));
    let middle = head(&window).offset;
    assert!(middle > 20 && middle < 29, "somewhere inside the source: {middle}");
    assert_eq!(source(&mut window).as_deref(), Some(INLINE), "the sheet stays out");

    // The start of the source is still in the formula: what is typed there is part of it.
    window.click_at(at(0.0));
    assert_eq!(head(&window), Pos::new("p1", 20));
    window.type_text("2");
    assert_eq!(source(&mut window), Some(format!("2{INLINE}")));

    // A drag selects source, which typing replaces. It comes too late to be a double click.
    window.advance(1000).press_at(at(0.0)).release_at(at(1.0));
    assert_eq!(selected(&window), format!("2{INLINE}"));
    window.type_text("y");
    assert_eq!(source(&mut window).as_deref(), Some("y"));
    assert_eq!(block_text(&window, "p1"), "A rafter has length y");
}

/// Whether the next frame draws the caret where it can be seen: whole, inside whatever clips it.
fn caret_shows(window: &mut Harness) -> bool {
    let caret = caret(window);
    let holds = |clip: &Rect| clip.x <= caret.x && caret.x + caret.width <= clip.x + clip.width;
    let mut clips = Vec::new();
    window.paint().items.iter().any(|item| match item {
        Item::PushClip { rect, .. } => {
            clips.push(*rect);
            false
        }
        Item::PopClip => {
            clips.pop();
            false
        }
        Item::Fill { rect, .. } => {
            (rect.x, rect.y) == (caret.x, caret.y) && rect.width < 3.0 && clips.iter().all(holds)
        }
        _ => false,
    })
}

#[test]
fn the_caret_shows_and_blinks_at_the_end_of_a_source_as_wide_as_its_sheet() {
    // Written far wider than it typesets, so the sheet is no wider than the source.
    const LONG: &str = "c = \\sqrt{a^2 + b^2}";
    let blocks = vec![text("p1", BlockKind::Paragraph, "A rafter has length "), text("f1", BlockKind::Formula, LONG)];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_reduced_motion(true);
    window.ui().select(Selection::caret(Pos::new("p1", 20)));
    window.type_text(&format!("${LONG}$"));
    // How the caret comes to the end of a source: in a paragraph, in a block, and after a space typed there.
    for way in ["in a paragraph", "in a block", "after a space"] {
        match way {
            "in a paragraph" => _ = window.key("Left"),
            "in a block" => window.ui().select(Selection::caret(Pos::new("f1", LONG.len()))),
            _ => _ = window.type_text(" "),
        }
        assert_eq!(source(&mut window).map(|source| source.trim_end().to_owned()).as_deref(), Some(LONG), "{way}");
        let seen: Vec<bool> = (0..20).map(|_| caret_shows(window.advance(100))).collect();
        assert!(seen.contains(&true), "{way}: the caret shows past the last character");
        assert!(seen.contains(&false), "{way}: and it blinks");
    }
}

#[test]
fn what_is_typed_shows_the_caret_where_a_long_source_scrolls_under_it() {
    let long = "a + ".repeat(60);
    let mut window =
        Harness::open(Document { blocks: vec![text("f1", BlockKind::Formula, &long)], ..Document::default() });
    window.ui().set_reduced_motion(true);
    window.ui().select(Selection::caret(Pos::new("f1", long.len())));
    let at = caret(&mut window);
    for typed in ["b", "Left Right"] {
        // The caret has blinked out when the next key comes.
        let out = (0..20).any(|_| !caret_shows(window.advance(100)));
        assert!(out, "{typed}: the caret blinks");
        match typed {
            "b" => _ = window.type_text(typed),
            _ => typed.split(' ').for_each(|key| _ = window.key(key)),
        }
        assert_eq!(caret(&mut window), at, "{typed}: the caret keeps its place at the end of the sheet");
        assert!(caret_shows(&mut window), "{typed}: and shows at once");
    }
}

#[test]
fn a_sheet_slides_out_from_under_its_formula() {
    let mut window = formulas();
    window.ui().set_reduced_motion(false);
    window.key("Left");
    let mut heights = Vec::new();
    for _ in 0..30 {
        heights.push(sheet(&mut window).expect("a sheet").bounds.height);
        window.advance(8);
    }
    assert!(heights[0] < heights[29], "it starts short of its height: {heights:?}");
    assert!(heights.windows(2).all(|pair| pair[0] <= pair[1]), "and only grows: {heights:?}");
    assert_eq!(heights[28], heights[29], "to rest within a quarter of a second: {heights:?}");
}

/// The options a list beside the caret shows, top to bottom: each one's name, what is said beside it, and
/// whether it is the one picked. The sheet's symbols are such a list, and so are the insert menu's offers.
fn options(window: &mut Harness) -> Vec<(String, String, bool)> {
    let option = |found: letters_test::Found| {
        let picked = found.selected.filter(|_| found.role == Role::ListBoxOption)?;
        Some((found.name, found.description.unwrap_or_default(), picked))
    };
    window.controls().into_iter().filter_map(option).collect()
}

/// [`formulas`] with the caret at the end of the inline formula, and `typed` typed there.
fn typing(typed: &str) -> Harness {
    let mut window = formulas();
    window.key("Left").type_text(typed);
    window
}

#[test]
fn a_backslash_lists_the_symbols_that_match_what_follows_it() {
    let mut window = typing(r" \ap");
    window.assert_open(Role::ListBox, "Symbols");
    let listed = options(&mut window);
    assert_eq!(listed[0], (r"\approx".to_owned(), "approximately equal to".to_owned(), true), "{listed:?}");
    assert!(listed.len() > 1 && listed[1..].iter().all(|symbol| !symbol.2), "one is picked: {listed:?}");

    // The list follows what is typed, and a plain name finds a symbol as well as its command does.
    window.key("Backspace").key("Backspace").type_text("fraction");
    assert_eq!(options(&mut window)[0].0, r"\frac");
    window.type_text("zz");
    window.assert_closed(Role::ListBox, "Symbols");
}

#[test]
fn tab_writes_the_picked_symbol_and_a_press_writes_the_one_pressed() {
    let mut window = typing(r" \ap");
    let second = options(&mut window)[1].0.clone();
    window.key("Down");
    assert!(options(&mut window)[1].2, "Down picks the next one");
    window.key("Tab");
    assert_eq!(source(&mut window), Some(format!("{INLINE} {second}")));
    window.assert_closed(Role::ListBox, "Symbols");

    // One that takes arguments gets its braces, and the caret waits in the first pair.
    window.type_text(r" \fra").click(Role::ListBoxOption, r"\frac");
    assert_eq!(source(&mut window), Some(format!(r"{INLINE} {second} \frac{{}}{{}}")));
    window.type_text("a");
    assert_eq!(source(&mut window), Some(format!(r"{INLINE} {second} \frac{{a}}{{}}")));

    window.key("Mod+Z").key("Mod+Z");
    assert_eq!(source(&mut window), Some(format!(r"{INLINE} {second} \fra")), "taking a symbol is one step");
}

#[test]
fn the_list_of_symbols_is_put_away_without_leaving_the_formula() {
    // What is done while the list shows, and the source afterwards.
    let ways: [(&str, &str, &str); 3] = [
        ("Escape", "Escape", r"c = a + b \ap"),
        ("an arrow", "Left", r"c = a + b \ap"),
        ("a space", "Space", r"c = a + b \ap "),
    ];
    for (way, key, after) in ways {
        let mut window = typing(r" \ap");
        match key {
            "Space" => window.type_text(" "),
            key => window.key(key),
        };
        window.assert_closed(Role::ListBox, "Symbols");
        assert_eq!(source(&mut window).as_deref(), Some(after), "{way}: the sheet is still out");
    }
    // Escape once more steps out of the formula.
    let mut window = typing(r" \ap");
    window.key("Escape").key("Escape");
    assert_eq!(source(&mut window), None);
}

/// How strongly each run of glyphs on the page is drawn, back to front: the alpha of its ink.
fn inks(window: &mut Harness) -> Vec<u8> {
    let page = page(window);
    let ink = |item: &Item| match item {
        Item::Glyphs { origin, color, .. } if page.contains(*origin) => Some(color.a),
        _ => None,
    };
    window.paint().items.iter().filter_map(ink).collect()
}

#[test]
fn the_dollar_that_closes_a_formula_blends_its_source_into_its_glyphs() {
    // A formula closed in a line of text, and one closed alone in its paragraph.
    for (start, typed) in [("A rafter has length ", "$c = a + b$"), ("", "$$x = a + b$$")] {
        let closed = |reduced_motion: bool| {
            let blocks = vec![text("p1", BlockKind::Paragraph, start)];
            let mut window = Harness::open(Document { blocks, ..Document::default() });
            window.ui().set_reduced_motion(reduced_motion);
            window.ui().select(Selection::caret(Pos::new("p1", start.len())));
            window.type_text(typed);
            window
        };
        let rest = inks(&mut closed(true));
        assert_eq!(inks(closed(true).advance(400)), rest, "{typed}: nothing blends under reduced motion");

        let mut window = closed(false);
        let mut frames = Vec::new();
        for _ in 0..30 {
            frames.push(inks(&mut window));
            window.advance(8);
        }
        assert!(frames[0].len() > rest.len(), "{typed}: the source is still on the page: {:?}", frames[0]);
        assert!(frames[0].contains(&0), "{typed}: and the formula has yet to show: {:?}", frames[0]);
        let between = |inks: &Vec<u8>| inks.iter().filter(|ink| (1..255).contains(*ink)).count() >= 2;
        assert!(frames.iter().any(between), "{typed}: each shows through the other on the way: {frames:?}");
        assert_eq!(frames[29], rest, "{typed}: within a quarter of a second the formula is all that shows");
    }
}

/// The names of the options a list beside the caret shows.
fn offered(window: &mut Harness) -> Vec<String> {
    options(window).into_iter().map(|option| option.0).collect()
}

#[test]
fn a_slash_on_an_empty_line_offers_what_to_put_there() {
    let mut window = blank();
    window.type_text("/");
    window.assert_open(Role::ListBox, "Insert");
    let all = options(&mut window);
    assert_eq!(all[0], ("Heading 1".to_owned(), "#".to_owned(), true), "each offer shows its shorthand: {all:?}");
    assert!(all.len() > 8 && all[1..].iter().all(|offer| !offer.2), "one is picked: {all:?}");

    // What is typed after the slash narrows the menu, and what matches nothing puts it away for the moment.
    window.type_text("li");
    assert_eq!(offered(&mut window), ["Bulleted list", "Numbered list", "Task list"]);
    window.type_text("zz");
    window.assert_closed(Role::ListBox, "Insert");
    window.key("Backspace").key("Backspace");
    assert_eq!(offered(&mut window).len(), 3);
    assert_eq!(block_text(&window, "p"), "/li", "the line is written as any other until an offer is taken");
}

#[test]
fn the_insert_menu_puts_what_is_taken_on_its_line() {
    // What is typed, how an offer is taken, and the document once "Roof" is written there.
    let cases = [
        ("/", "Enter", "# Roof"),
        ("/", "Down Down Tab", "### Roof"),
        ("/", "Up Enter", "---\n\nRoof"),
        ("/task", "Enter", "- [ ] Roof"),
        ("/quote", "a press", "> Roof"),
        ("/code", "Enter", "```\nRoof\n```"),
    ];
    for (typed, take, expected) in cases {
        let mut window = blank();
        window.type_text(typed);
        match take {
            "a press" => window.click(Role::ListBoxOption, "Quote"),
            keys => keys.split(' ').fold(&mut window, |window, key| window.key(key)),
        };
        window.assert_closed(Role::ListBox, "Insert");
        window.type_text("Roof");
        assert_eq!(window.markdown().trim_end(), expected, "{typed} then {take}");
    }
}

#[test]
fn the_insert_menu_stays_away_where_a_slash_is_only_a_slash() {
    // A slash inside a line opens nothing.
    let mut window = blank();
    window.type_text("and/or");
    window.assert_closed(Role::ListBox, "Insert");

    // Escape puts the menu away and leaves the slash, which is then text like any other.
    let mut window = blank();
    window.type_text("/").key("Escape");
    window.assert_closed(Role::ListBox, "Insert");
    window.type_text("li");
    window.assert_closed(Role::ListBox, "Insert");
    assert_eq!(block_text(&window, "p"), "/li");

    // So does a caret that leaves the end of the line.
    let mut window = blank();
    window.type_text("/").key("Left").key("Right");
    window.assert_closed(Role::ListBox, "Insert");
}

/// A paragraph `p1`, a figure `g` with a caption, and a paragraph `p2`.
fn pictured() -> Harness {
    let figure = BlockKind::Figure {
        asset: None,
        src: Some("https://example.com/roof.png".into()),
        alt: "A slate roof".into(),
        width: 300.0,
        height: 150.0,
        numbered: false,
    };
    let blocks = vec![
        text("p1", BlockKind::Paragraph, "The roof today:"),
        text("g", figure, "Seen from the garden"),
        text("p2", BlockKind::Paragraph, "Reply by Friday."),
    ];
    Harness::open(Document { blocks, ..Document::default() })
}

/// The outlines the page draws round a point.
fn rings(window: &mut Harness, at: Point) -> usize {
    let round = |item: &Item| matches!(item, Item::Stroke { rect, .. } if rect.contains(at));
    window.paint().items.iter().filter(|item| round(item)).count()
}

#[test]
fn a_press_on_a_picture_selects_its_figure_and_one_on_its_caption_writes_there() {
    let mut window = pictured();
    let caption = spot(&mut window, "g", 4);
    // The picture is above its caption, across the middle of the page.
    let page = page(&mut window);
    let picture = Point { x: page.x + page.width / 2.0, y: caption.y - 40.0 };
    let before = rings(&mut window, picture);

    window.click_at(picture);
    assert_eq!(window.editor().picture().map(|figure| figure.as_str()), Some("g"));
    assert_eq!(rings(&mut window, picture), before + 1, "the picture has a ring round it");
    assert_eq!(window.ui().caret_rect(), None, "and no caret blinks in the text");
    window.type_text("x");
    assert_eq!(block_text(&window, "g"), "Seen from the garden", "what is typed on a picture goes nowhere");

    window.click_at(Point { x: caption.x, y: caption.y + caption.height / 2.0 });
    assert_eq!(window.editor().picture(), None);
    assert_eq!(head(&window), Pos::new("g", 4));
    assert_eq!(rings(&mut window, picture), before, "the ring goes with the selection");
}
