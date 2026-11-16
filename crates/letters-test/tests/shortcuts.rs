//! The sheet of the keys that do things.

use letters_test::{Found, Harness, Role};

const SHEET: &str = "Keyboard shortcuts";

fn sheet(window: &mut Harness) -> Option<Found> {
    window.find(Role::Dialog, SHEET)
}

/// Whether the sheet writes `keys` on one of its lines.
fn writes(window: &mut Harness, keys: &str) -> bool {
    window.controls().iter().any(|found| found.role == Role::Label && found.name == keys)
}

#[test]
fn the_sheet_opens_on_its_key_and_goes_away_again() {
    // What puts it away again.
    for away in ["Escape", "Mod+/"] {
        let mut window = Harness::doc("Roof first");
        window.select("Roof");
        assert!(sheet(&mut window).is_none(), "it waits to be asked for");
        window.key("Mod+/");
        assert!(sheet(&mut window).is_some(), "the key shows it");
        assert!(writes(&mut window, "Ctrl+B"), "with the key that makes text bold");
        window.key(away);
        assert!(sheet(&mut window).is_none(), "{away} puts it away");
        window.type_text("Slates");
        window.assert_markdown("Slates first");
    }
}

#[test]
fn the_sheet_is_reached_from_wherever_keys_are_looked_for() {
    // The palette lists it by name.
    let mut window = Harness::doc("Roof first");
    window.key("Mod+Shift+P").type_text("keyboard").key("Enter");
    assert!(sheet(&mut window).is_some(), "the palette shows it");
    assert!(window.find(Role::Dialog, "Commands").is_none(), "and steps back");

    // Print view, which takes the page's place, answers its key as the page does.
    let mut window = Harness::doc("Roof first");
    window.key("Mod+P").key("Mod+/");
    assert!(sheet(&mut window).is_some(), "it shows over the sheets");
}

#[test]
fn the_sheet_lists_the_keys_for_windows_only_in_an_application() {
    for application in [false, true] {
        let mut window = Harness::doc("Roof first");
        window.ui().set_application(application);
        window.key("Mod+/");
        assert_eq!(writes(&mut window, "Ctrl+W"), application);
        assert!(writes(&mut window, "Ctrl+O"), "a document is opened by its key everywhere");
    }
}

#[test]
fn the_sheet_writes_a_macs_keys_on_a_mac() {
    let mut window = Harness::doc("Roof first").mac();
    window.key("Mod+/");
    assert!(writes(&mut window, "\u{2318}B"));
    assert!(!writes(&mut window, "Ctrl+B"));
}

#[test]
fn the_sheet_takes_the_place_of_another_dialog() {
    let mut window = Harness::doc("Roof first");
    window.key("Mod+,");
    window.assert_open(Role::Dialog, "Settings");
    window.key("Mod+/");
    assert!(sheet(&mut window).is_some());
    window.assert_closed(Role::Dialog, "Settings");
    window.key("Mod+,");
    assert!(sheet(&mut window).is_none(), "and gives it to the next");
}

#[test]
fn the_sheet_stays_inside_a_small_window() {
    // The window's width and height.
    for (width, height) in [(1200.0, 800.0), (420.0, 360.0)] {
        let mut window = Harness::doc("Roof first");
        window.resize(width, height).key("Mod+/");
        let card = sheet(&mut window).expect("the sheet").bounds;
        assert!(card.x >= 0.0 && card.x + card.width <= width, "{card:?} across a window {width} wide");
        assert!(card.y >= 0.0 && card.y + card.height <= height, "{card:?} down a window {height} high");
    }
}
