//! Print view: the document on the sheets it prints on.

use accesskit::{Action, ActionRequest, TreeId};
use letters_editor::Picture;
use letters_model::{Block, BlockKind, Document};
use letters_render::Point;
use letters_test::{Found, Harness, Role};
use letters_ui::{Landing, Request};

/// A window on a document of `count` pages: a line for each, which after the first starts a page.
fn paged(count: usize) -> Harness {
    let line = |n: usize| {
        let text = format!("Part {} of the plan", n + 1);
        Block { page_break: n > 0, ..Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, text) }
    };
    let mut window = Harness::open(Document { blocks: (0..count).map(line).collect(), ..Document::default() });
    window.resize(1280.0, 800.0);
    window
}

/// The sheets in view, in order.
fn sheets(window: &mut Harness) -> Vec<Found> {
    let shown = |found: &Found| found.role == Role::Image && found.name.starts_with("Page ");
    window.controls().into_iter().filter(shown).collect()
}

fn numbers(window: &mut Harness) -> Vec<String> {
    sheets(window).into_iter().map(|sheet| sheet.name).collect()
}

/// The area that shows the sheets.
fn view(window: &mut Harness) -> Found {
    window.find(Role::Document, "Print view").expect("Print view to show")
}

fn printing(window: &mut Harness) -> bool {
    window.find(Role::Document, "Print view").is_some()
}

type Way = fn(&mut Harness);

/// A window in Print view on a page of short paragraphs, none of which starts a page.
fn listed(count: usize) -> Harness {
    let line = |n: usize| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, format!("Item {}", n + 1));
    let mut window = Harness::open(Document { blocks: (0..count).map(line).collect(), ..Document::default() });
    window.resize(1280.0, 800.0);
    window.key("Mod+P");
    window
}

/// The blocks that start a page, by their ids.
fn breaks(window: &Harness) -> Vec<String> {
    let starts = window.editor().doc().walk().filter(|block| block.page_break);
    starts.map(|block| block.id.as_str().to_owned()).collect()
}

/// The buttons that take a page break away, each with where it says it is.
fn markers(window: &mut Harness) -> Vec<String> {
    let marks = |found: &Found| found.role == Role::Button && found.name == "Remove page break";
    window.controls().into_iter().filter(marks).filter_map(|found| found.description).collect()
}

/// Rests the pointer on a sheet, `down` of the way from its top to its bottom.
fn point_at(window: &mut Harness, sheet: usize, down: f32) {
    let on = sheets(window)[sheet].bounds;
    window.hover_at(Point { x: on.x + on.width / 2.0, y: on.y + on.height * down });
}

/// Presses a button of the sheets as assistive technology does.
fn ask_to_press(window: &mut Harness, name: &str) {
    let tree = window.ui().access();
    let named = |node: &accesskit::Node| node.role() == Role::Button && node.label() == Some(name);
    let target_node = tree.nodes.iter().find(|(_, node)| named(node)).unwrap_or_else(|| panic!("a button {name}")).0;
    window.ask(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None });
}

#[test]
fn print_view_takes_the_place_of_the_page_and_gives_it_back() {
    let ways_in: [(&str, Way); 3] = [
        ("its key", |window| {
            window.key("Mod+P");
        }),
        ("the More menu", |window| {
            window.click(Role::Button, "More").click(Role::MenuItem, "Print view Ctrl+P");
        }),
        ("the palette", |window| {
            window.key("Mod+Shift+P").type_text("print view").key("Enter");
        }),
    ];
    let ways_back: [(&str, Way); 2] = [
        ("Escape", |window| {
            window.key("Escape");
        }),
        ("the bar", |window| {
            window.click(Role::Button, "Back to editing");
        }),
    ];
    for (name, show) in ways_in {
        for (back_name, back) in ways_back {
            let mut window = paged(3);
            window.select("Part 2");
            show(&mut window);
            assert!(printing(&mut window), "by {name}");
            assert_eq!(numbers(&mut window), ["Page 1 of 3", "Page 2 of 3"], "by {name}");
            assert!(window.find(Role::Label, "Print view, 3 pages").is_some(), "by {name}");
            assert_eq!(window.find(Role::Button, "Bold"), None, "the bar is the view's, by {name}");

            back(&mut window);
            assert!(!printing(&mut window), "by {name}, back by {back_name}");
            assert!(window.find(Role::Button, "Bold").is_some(), "by {name}, back by {back_name}");
            let read = window.controls().into_iter().filter(|found| found.role == Role::Paragraph).count();
            assert_eq!(read, 3, "the page is read again, by {name}, back by {back_name}");
            // What was selected is selected still, and is written over.
            window.type_text("Second part");
            window.assert_markdown("Part 1 of the plan\n\nSecond part of the plan\n\nPart 3 of the plan");
        }
    }
}

#[test]
fn every_page_has_a_sheet_that_says_its_number() {
    // How many pages the document has, what the bar says, and the sheets that lead the view.
    let cases = [
        (1, "Print view, 1 page", vec!["Page 1 of 1"]),
        (2, "Print view, 2 pages", vec!["Page 1 of 2", "Page 2 of 2"]),
        (5, "Print view, 5 pages", vec!["Page 1 of 5", "Page 2 of 5"]),
    ];
    for (count, said, first) in cases {
        let mut window = paged(count);
        window.key("Mod+P");
        assert!(window.find(Role::Label, said).is_some(), "{count} pages");
        assert_eq!(numbers(&mut window), first, "{count} pages");
        // Every sheet comes into view on the way to the last, in the order of its number.
        let mut seen = Vec::new();
        for _ in 0..count {
            let shown = numbers(&mut window);
            seen.extend(shown.into_iter().filter(|sheet| !seen.contains(sheet)).collect::<Vec<_>>());
            window.key("PageDown");
        }
        let all: Vec<String> = (1..=count).map(|page| format!("Page {page} of {count}")).collect();
        assert_eq!(seen, all, "{count} pages");
    }
}

#[test]
fn text_that_fills_a_page_runs_on_to_the_next() {
    let line = "Harland Roofing quoted for the full north slope, slates and battens and lead.\n\n";
    let mut window = Harness::doc(&line.repeat(120));
    window.resize(1280.0, 800.0);
    window.key("Mod+P").key("End");
    let last = numbers(&mut window).pop().expect("a last sheet");
    let (page, count) = last.trim_start_matches("Page ").split_once(" of ").expect("a number of a count");
    assert_eq!(page, count, "the last sheet is the last page");
    assert!(count.parse::<usize>().expect("a count") > 1, "{last}");
}

#[test]
fn the_zoom_says_how_many_whole_sheets_show() {
    // The zoom, the sheets in view, and whether a sheet shows as large as its paper.
    let cases = [
        ("Two pages", vec!["Page 1 of 3", "Page 2 of 3"], false),
        ("One page", vec!["Page 1 of 3"], false),
        ("100%", vec!["Page 1 of 3"], true),
    ];
    for (zoom, shown, full) in cases {
        let mut window = paged(3);
        window.key("Mod+P").click(Role::RadioButton, zoom);
        assert_eq!(window.find(Role::RadioButton, zoom).and_then(|zoom| zoom.toggled), Some(true), "{zoom}");
        assert_eq!(numbers(&mut window), shown, "{zoom}");
        let (area, sheets) = (view(&mut window).bounds, sheets(&mut window));
        // A4 is 210 mm across, which is 794 pixels.
        assert_eq!((sheets[0].bounds.width - 794.0).abs() <= 1.0, full, "{zoom}");
        if !full {
            let whole = |sheet: &Found| {
                let bounds = sheet.bounds;
                bounds.y >= area.y && bounds.y + bounds.height <= area.y + area.height
            };
            assert!(sheets.iter().all(whole), "{zoom}: {sheets:?} in {area:?}");
        }
        // Sheets of a row stand side by side, inside the view.
        assert!(sheets.iter().all(|sheet| sheet.bounds.y == sheets[0].bounds.y), "{zoom}");
        assert!(sheets.windows(2).all(|pair| pair[0].bounds.x + pair[0].bounds.width < pair[1].bounds.x), "{zoom}");
        let last = sheets.last().expect("a sheet").bounds;
        assert!(sheets[0].bounds.x >= area.x && last.x + last.width <= area.x + area.width, "{zoom}");
    }
}

#[test]
fn a_change_of_zoom_or_of_the_window_keeps_the_sheet_in_view() {
    let mut window = paged(6);
    window.key("Mod+P").click(Role::RadioButton, "One page");
    window.key("PageDown").key("PageDown").key("PageDown");
    assert_eq!(numbers(&mut window), ["Page 4 of 6"]);
    window.click(Role::RadioButton, "Two pages");
    assert_eq!(numbers(&mut window), ["Page 3 of 6", "Page 4 of 6"], "the row that holds it");
    window.resize(1000.0, 500.0);
    assert_eq!(numbers(&mut window), ["Page 3 of 6", "Page 4 of 6"], "in a smaller window");
    window.click(Role::RadioButton, "100%");
    assert_eq!(numbers(&mut window), ["Page 3 of 6"], "the sheet that led the row");
}

#[test]
fn keys_and_the_wheel_move_through_the_sheets() {
    let mut window = paged(6);
    window.key("Mod+P");
    let at = view(&mut window).center();
    // What is done, and the sheets in view after it.
    let steps: [(&str, Way, &[&str]); 6] = [
        (
            "Page Down",
            |window| {
                window.key("PageDown");
            },
            &["Page 3 of 6", "Page 4 of 6"],
        ),
        (
            "End",
            |window| {
                window.key("End");
            },
            &["Page 5 of 6", "Page 6 of 6"],
        ),
        (
            "Page Down at the end",
            |window| {
                window.key("PageDown");
            },
            &["Page 5 of 6", "Page 6 of 6"],
        ),
        (
            "Page Up",
            |window| {
                window.key("PageUp");
            },
            &["Page 3 of 6", "Page 4 of 6"],
        ),
        (
            "Home",
            |window| {
                window.key("Home");
            },
            &["Page 1 of 6", "Page 2 of 6"],
        ),
        (
            "Page Up at the start",
            |window| {
                window.key("PageUp");
            },
            &["Page 1 of 6", "Page 2 of 6"],
        ),
    ];
    for (name, step, shown) in steps {
        step(&mut window);
        assert_eq!(numbers(&mut window), shown, "after {name}");
    }
    let top = sheets(&mut window)[0].bounds.y;
    window.scroll_at(at, 120.0);
    assert_eq!(sheets(&mut window)[0].bounds.y, top - 120.0, "the wheel moves the sheets by what it turned");
    window.key("Down");
    assert!(sheets(&mut window)[0].bounds.y < top - 120.0, "an arrow moves them a little");
    window.scroll_at(at, -10_000.0);
    assert_eq!(sheets(&mut window)[0].bounds.y, top, "and no further than the first");
}

#[test]
fn nothing_is_written_in_print_view() {
    let before = "Part 1 of the plan\n\nPart 2 of the plan";
    for key in ["x", "Backspace", "Delete", "Enter", "Mod+B", "Mod+V", "Mod+X", "Mod+K", "Mod+Shift+P"] {
        let mut window = paged(2);
        window.select("Part 2").key("Mod+P").key(key);
        assert!(printing(&mut window), "{key}");
        assert_eq!(window.find(Role::Dialog, "Commands"), None, "{key}");
        assert_eq!(window.find(Role::Dialog, "Link"), None, "{key}");
        window.assert_markdown(before);
    }
}

#[test]
fn print_view_opens_at_the_sheet_that_was_being_read() {
    let mut window = Harness::doc(&"Harland Roofing quoted for the full north slope.\n\n".repeat(200));
    window.resize(1280.0, 800.0);
    let area = window.find(Role::Document, "Letters").expect("the page").center();
    window.scroll_at(area, 100_000.0).key("Mod+P");
    let shown = numbers(&mut window);
    let (page, count) = shown[0].trim_start_matches("Page ").split_once(" of ").expect("a number of a count");
    let (page, count): (usize, usize) = (page.parse().expect("a page"), count.parse().expect("a count"));
    assert!(count > 2 && page + 1 >= count, "the end of the document was in view: {shown:?}");
}

#[test]
fn the_paper_is_changed_from_the_bar_and_undo_takes_it_back() {
    let mut window = paged(2);
    window.key("Mod+P");
    let upright = sheets(&mut window)[0].bounds;
    assert!(upright.height > upright.width);

    window.click(Role::Button, "A4, portrait").click(Role::MenuItemCheckBox, "Landscape");
    let page = window.ui().style().page.clone();
    assert!(page.width > page.height, "the document's page is turned");
    assert!(window.find(Role::Button, "A4, landscape").is_some());
    let turned = sheets(&mut window)[0].bounds;
    assert!(turned.width > turned.height, "{turned:?}");

    window.key("Mod+Z");
    assert!(printing(&mut window));
    let page = window.ui().style().page.clone();
    assert!(page.height > page.width, "undo turns it back");
    assert!(window.find(Role::Button, "A4, portrait").is_some());
}

#[test]
fn the_bar_prints_and_exports() {
    let mut window = paged(2);
    window.key("Mod+P");
    window.ui().take_requests();
    // The shell is asked to print by the button, and by the key that opened the view.
    let ways: [(&str, Way); 2] = [
        ("the button", |window| {
            window.click(Role::Button, "Print");
        }),
        ("the key", |window| {
            window.key("Mod+P");
        }),
    ];
    for (name, print) in ways {
        print(&mut window);
        assert_eq!(window.ui().take_requests(), [Request::Print], "by {name}");
        assert!(printing(&mut window), "by {name}");
    }
    window.click(Role::Button, "Export PDF").assert_open(Role::Dialog, "Save a copy");
    // Escape puts the card away first, and only then leaves the view.
    window.key("Escape").assert_closed(Role::Dialog, "Save a copy");
    assert!(printing(&mut window));
}

#[test]
fn a_copy_written_from_print_view_is_said_there() {
    let mut window = paged(2);
    window.key("Mod+P").click(Role::Button, "Export PDF").key("Enter");
    let asked = window.ui().take_requests();
    assert!(matches!(asked[..], [Request::SaveCopy { .. }]), "{asked:?}");
    window.ui().set_copied("Documents");
    assert!(window.find(Role::Tooltip, "Exported to Documents").is_some());
    assert!(printing(&mut window));
}

#[test]
fn the_shell_writes_nothing_on_the_sheets() {
    let mut window = paged(2);
    window.select("Part 2").key("Mod+P");
    let at = view(&mut window).center();
    assert!(!window.hold_pictures(Some(at)), "pictures held over a sheet have nowhere to land");
    assert!(!window.long_press_at(at), "a long press selects no word");
    assert_eq!(window.ui().selection_ends(), [None, None], "and the selection has no ends to draw");
    // A picture as a shell hands one over: bytes that begin as a PNG's do.
    let png = || {
        let bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        Picture { bytes, mime: "image/png".into(), width: 400, height: 200 }
    };
    let edits = window.ui().edits();
    window.drop_pictures(Some(at), vec![png()]).drop_pictures(None, vec![png()]).click_at(at);
    assert_eq!(window.ui().edits(), edits, "pictures let go there are not put in");
    assert!(printing(&mut window));
    window.assert_markdown("Part 1 of the plan\n\nPart 2 of the plan");
}

#[test]
fn a_narrow_window_has_the_bar_folded_with_everything_in_reach() {
    let mut window = paged(3);
    window.resize(420.0, 700.0).key("Mod+P");
    let bar = window.find(Role::Toolbar, "Print view").expect("the bar").bounds;
    for name in ["Back to editing", "Page setup", "Two pages", "Export PDF", "Print"] {
        let button = window.find(Role::Button, name).unwrap_or_else(|| panic!("{name} in the bar")).bounds;
        assert!(button.x >= bar.x && button.x + button.width <= bar.x + bar.width, "{name}: {button:?}");
    }
    window.click(Role::Button, "Two pages").click(Role::MenuItemCheckBox, "One page");
    assert!(window.find(Role::Button, "One page").is_some(), "the button says the zoom in force");
    let sheets = sheets(&mut window);
    assert!(sheets[1].bounds.y > sheets[0].bounds.y + sheets[0].bounds.height, "one sheet to a row: {sheets:?}");
    window.click(Role::Button, "Back to editing");
    assert!(!printing(&mut window));
}

#[test]
fn panels_step_aside_for_print_view_and_come_back() {
    let mut window = paged(2);
    window.click(Role::Button, "Styles");
    let beside = window.find(Role::Document, "Letters").expect("the page").bounds.width;
    assert!(beside < 1200.0, "the panel takes its room from the page");
    window.key("Mod+P");
    assert!(view(&mut window).bounds.width > beside, "the sheets have the window's width");
    window.key("Escape");
    assert_eq!(window.find(Role::Document, "Letters").expect("the page").bounds.width, beside);
}

#[test]
fn print_view_waits_for_a_document() {
    let mut window = paged(1);
    window.ui().set_landing(Some(Landing::default()));
    window.key("Mod+P");
    assert!(!printing(&mut window), "the landing card is answered first");

    // A card that comes while the sheets show has the window back for itself.
    let mut window = paged(1);
    window.key("Mod+P");
    window.ui().set_landing(Some(Landing::default()));
    assert!(!printing(&mut window));
    assert!(window.find(Role::Document, "Letters").is_some());
}

#[test]
fn a_picture_on_a_sheet_is_asked_for() {
    let figure = BlockKind::Figure {
        asset: Some("roof.png".into()),
        src: None,
        alt: String::new(),
        width: 300.0,
        height: 150.0,
        numbered: false,
    };
    // The picture is far below what the page shows when the document opens, and on the last sheet.
    let line = |n: usize| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, "A part of the plan");
    let mut blocks: Vec<Block> = (0..120).map(line).collect();
    blocks.push(Block::text("g", figure, ""));
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.resize(1280.0, 800.0).paint();
    window.ui().take_requests();
    window.key("Mod+P").paint();
    assert_eq!(window.ui().take_requests(), [], "nothing is asked for a sheet that is out of view");
    window.key("End").paint();
    assert_eq!(window.ui().take_requests(), [Request::Asset("roof.png".into())]);
    // The pointer over a sheet does nothing to the document.
    let at = view(&mut window).center();
    window.click_at(Point { x: at.x - 200.0, y: at.y });
    assert!(printing(&mut window));
}

#[test]
fn a_page_is_made_to_start_between_two_blocks_under_the_pointer() {
    // How the offer is taken.
    let ways: [(&str, Way); 2] = [
        ("a click", |window| {
            window.click(Role::Button, "Page break here");
        }),
        ("assistive technology", |window| ask_to_press(window, "Page break here")),
    ];
    for (name, take) in ways {
        let mut window = listed(12);
        assert_eq!(window.find(Role::Button, "Page break here"), None, "nothing is offered far from the pointer");
        point_at(&mut window, 0, 0.3);
        let offer = window.find(Role::Button, "Page break here").expect("an offer under the pointer");
        assert_eq!(offer.description.as_deref(), Some("On page 1"));
        let sheet = sheets(&mut window)[0].bounds;
        assert!((offer.center().y - (sheet.y + sheet.height * 0.3)).abs() < 40.0, "{offer:?} is by the pointer");

        take(&mut window);
        let made = breaks(&window);
        assert_eq!(made.len(), 1, "by {name}");
        assert_ne!(made, ["p0"], "the page starts with a block that had one before it, by {name}");
        assert_eq!(numbers(&mut window), ["Page 1 of 2", "Page 2 of 2"], "by {name}");
        assert_eq!(markers(&mut window), ["After page 1"], "the break is marked where it is, by {name}");
        assert!(printing(&mut window), "by {name}");

        // It is an edit, which undo takes back from here.
        window.key("Mod+Z");
        assert_eq!((breaks(&window), numbers(&mut window)), (vec![], vec!["Page 1 of 1".to_owned()]), "by {name}");
        assert_eq!(markers(&mut window), Vec::<String>::new(), "by {name}");
    }
}

#[test]
fn a_page_break_is_marked_with_a_button_that_takes_it_away() {
    let ways: [(&str, Way); 2] = [
        ("a click", |window| {
            window.click(Role::Button, "Remove page break");
        }),
        ("assistive technology", |window| ask_to_press(window, "Remove page break")),
    ];
    for (name, remove) in ways {
        // The document comes with its breaks, each marked on the sheet before it.
        let mut window = paged(3);
        window.key("Mod+P");
        assert_eq!(markers(&mut window), ["After page 1", "After page 2"], "by {name}");

        remove(&mut window);
        assert_eq!(breaks(&window), ["p2"], "the first break is gone, by {name}");
        assert_eq!(numbers(&mut window), ["Page 1 of 2", "Page 2 of 2"], "by {name}");
        assert_eq!(markers(&mut window), ["After page 1"], "by {name}");
        window.key("Mod+Z");
        assert_eq!(breaks(&window), ["p1", "p2"], "undone, by {name}");
    }
}

#[test]
fn a_page_break_is_offered_only_where_one_can_be() {
    // What the document is, where on its first sheet the pointer rests, and why nothing is offered.
    let cases = [
        (1, 0.15, "one block has no block to part from"),
        (12, 0.9, "the blank end of a sheet is far from any block"),
        (12, 0.02, "the top of a sheet is far from the first room between two blocks"),
    ];
    for (count, down, why) in cases {
        let mut window = listed(count);
        point_at(&mut window, 0, down);
        assert_eq!(window.find(Role::Button, "Page break here"), None, "{why}");
    }
    // The first block starts its page already: at the top of the text the offer is to part it from the second.
    let mut window = listed(12);
    point_at(&mut window, 0, 0.085);
    window.click(Role::Button, "Page break here");
    assert_eq!(breaks(&window), ["p1"]);

    // Off the sheets nothing is offered either, and an offer goes when the pointer does.
    let mut window = listed(12);
    point_at(&mut window, 0, 0.3);
    assert!(window.find(Role::Button, "Page break here").is_some());
    let sheet = sheets(&mut window)[0].bounds;
    window.hover_at(Point { x: sheet.x - 20.0, y: sheet.y + sheet.height * 0.3 });
    assert_eq!(window.find(Role::Button, "Page break here"), None, "beside the sheet");
}

#[test]
fn a_press_that_ends_off_its_button_changes_no_page() {
    let mut window = listed(12);
    point_at(&mut window, 0, 0.3);
    let offer = window.find(Role::Button, "Page break here").expect("an offer").center();
    let sheet = sheets(&mut window)[0].bounds;
    window.press_at(offer).release_at(Point { x: sheet.x + 4.0, y: sheet.y + 4.0 });
    assert_eq!(breaks(&window), Vec::<String>::new());

    let mut window = paged(2);
    window.key("Mod+P");
    let remove = window.find(Role::Button, "Remove page break").expect("a marker").center();
    let sheet = sheets(&mut window)[0].bounds;
    window.press_at(remove).release_at(Point { x: sheet.x + 4.0, y: sheet.y + 4.0 });
    assert_eq!(breaks(&window), ["p1"]);
}

#[test]
fn a_page_break_shows_only_in_print_view() {
    let mut window = listed(12);
    point_at(&mut window, 0, 0.3);
    window.click(Role::Button, "Page break here");
    let made = breaks(&window);
    assert_eq!(made.len(), 1);

    window.key("Escape");
    let all: Vec<String> = (1..=12).map(|n| format!("Item {n}")).collect();
    window.assert_markdown(&all.join("\n\n"));
    assert_eq!(markers(&mut window), Vec::<String>::new(), "the page that is edited marks none");
    assert_eq!(window.find(Role::Button, "Page break here"), None);
    // The break is the document's, and is there when the sheets come back.
    window.key("Mod+P");
    assert_eq!((breaks(&window), markers(&mut window)), (made, vec!["After page 1".to_owned()]));
}

#[test]
fn the_margins_are_set_from_the_bar_and_the_sheets_follow() {
    // Lines that nearly fill a sheet.
    let mut window = listed(26);
    assert_eq!(numbers(&mut window), ["Page 1 of 1"]);
    let setup = window.find(Role::Button, "A4, portrait").expect("the paper's button");
    assert_eq!(setup.description.as_deref(), Some("Page setup"));

    // Margins are typed in millimetres, and the paper has to have room for them.
    for (typed, taken) in [("40", true), ("500", false), ("wide", false)] {
        let page = window.ui().style().page.clone();
        window.click(Role::Button, "A4, portrait");
        window.click(Role::TextInput, "Margins").key("Mod+A").type_text(typed).key("Enter");
        let after = window.ui().style().page.clone();
        assert_eq!(after != page, taken, "{typed:?}");
        assert_eq!((after.width, after.height), (page.width, page.height), "the paper is as it was");
        // Escape puts the card away, and the view stays.
        window.key("Escape");
        assert_eq!(window.find(Role::TextInput, "Margins"), None, "{typed:?}");
        assert!(printing(&mut window), "{typed:?}");
    }
    let sides = window.ui().style().page.clone();
    let sides = [sides.margin_top, sides.margin_right, sides.margin_bottom, sides.margin_left];
    assert!(sides.iter().all(|side| (side - 113.4).abs() < 0.1), "40 mm on every side: {sides:?}");
    assert_eq!(numbers(&mut window), ["Page 1 of 2", "Page 2 of 2"], "less fits a sheet between wider margins");

    window.key("Mod+Z");
    assert_eq!(numbers(&mut window), ["Page 1 of 1"], "undo takes the margins back");
    assert!(printing(&mut window));
}

#[test]
fn page_setup_is_asked_for_while_editing_and_found_in_the_styles_panel() {
    let ways: [(&str, Way); 2] = [
        ("the More menu", |window| {
            window.click(Role::Button, "More").click(Role::MenuItem, "Page setup");
        }),
        ("the palette", |window| {
            window.key("Mod+Shift+P").type_text("margins").key("Enter");
        }),
    ];
    for (name, ask) in ways {
        // A window too short to show the whole panel at once.
        let mut window = paged(2);
        window.resize(1280.0, 420.0);
        assert_eq!(window.find(Role::TextInput, "Margins"), None, "by {name}");
        ask(&mut window);
        let paper = window.find(Role::Button, "Paper").unwrap_or_else(|| panic!("the paper in the panel, by {name}"));
        let margins = window.find(Role::TextInput, "Margins").unwrap_or_else(|| panic!("the margins, by {name}"));
        for found in [paper, margins] {
            let (top, bottom) = (found.bounds.y, found.bounds.y + found.bounds.height);
            assert!(top >= 0.0 && bottom <= 420.0, "{found:?} is in view, by {name}");
        }
        assert!(!printing(&mut window), "by {name}");
    }
}
