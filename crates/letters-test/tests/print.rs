//! Print view: the document on the sheets it prints on.

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
    for name in ["Back to editing", "Paper", "Two pages", "Export PDF", "Print"] {
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
