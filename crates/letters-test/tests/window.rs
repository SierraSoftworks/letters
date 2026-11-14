//! The editor's window, as a shell shows it: the page, the top bar and the side panel.

use accesskit::{Action, ActionData, ActionRequest, TextPosition, TextSelection, TreeId};
use letters_model::{Block, BlockKind, Document, Pos, Selection};
use letters_render::{Color, Item, Point, Rect};
use letters_test::{Harness, Role, WINDOW};
use letters_ui::SaveState;

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

/// The page area.
fn page(window: &mut Harness) -> Rect {
    window.find(Role::Document, "Letters").expect("the page").bounds
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

fn head(window: &Harness) -> Pos {
    window.editor().selection().expect("a selection").head.clone()
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
    window.assert_open(Role::Label, "Quote").assert_open(Role::Label, "Saved").assert_open(Role::Label, "Heading 1");

    window.ui().set_save_state(SaveState::Offline);
    window.ui().select(Selection::caret(Pos::new("p1", 0)));
    window.assert_open(Role::Label, "Offline").assert_open(Role::Label, "Body");
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
fn the_outline_is_a_line_for_each_heading_stepped_in_by_its_level() {
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
