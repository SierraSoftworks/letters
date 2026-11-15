//! Safe mode: the bar over a document that is held back, and links, which the shell judges before they open.

use letters_render::Point;
use letters_test::{Found, Harness, Role};
use letters_ui::{Request, SafeMode};

/// Where the written quote leads.
const HREF: &str = "https://harland.example/quotes/north-slope?ref=letter";

/// A letter with a link to a page, a link to a place of its own, and a line with neither.
fn letter() -> Harness {
    let text =
        format!("Their [written quote]({HREF}) covers the slates, as [the plan](#plan) says.\n\nReply by Friday.");
    Harness::doc(&text)
}

/// The bar that says the document is held back.
fn bar(window: &mut Harness) -> Option<Found> {
    window.controls().into_iter().find(|found| found.role == Role::Status && found.name.starts_with("Safe mode"))
}

fn page(window: &mut Harness) -> Found {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page")
}

/// A point on the last letter of the first place `text` appears. The text is left selected.
fn on(window: &mut Harness, text: &str) -> Point {
    window.select(text).paint();
    let end = window.ui().selection_ends()[1].expect("the selection ends in view");
    Point { x: end.x - 3.0, y: end.y + end.height / 2.0 }
}

/// The links the window has asked its shell to open, since this was last called.
fn links(window: &mut Harness) -> Vec<Request> {
    let asked = window.ui().take_requests();
    asked.into_iter().filter(|request| matches!(request, Request::OpenLink { .. })).collect()
}

fn open(href: &str, confirmed: bool) -> Request {
    Request::OpenLink { href: href.to_owned(), confirmed }
}

#[test]
fn a_held_document_says_so_on_a_bar_that_asks_for_trust() {
    // The folder the shell names, and what the bar offers then.
    let cases = [
        (None, vec![("Trust this document", Request::TrustDocument)]),
        (
            Some("Downloads"),
            vec![("Trust this document", Request::TrustDocument), ("Trust this folder", Request::TrustFolder)],
        ),
    ];
    for (folder, offers) in cases {
        let mut window = letter();
        assert!(bar(&mut window).is_none(), "a document is not held until the shell says so");
        let whole = page(&mut window).bounds;

        window.ui().set_safe_mode(Some(SafeMode { folder: folder.map(str::to_owned) }));
        let (held, under) = (bar(&mut window).expect("a bar").bounds, page(&mut window).bounds);
        assert!(under.y >= held.y + held.height, "{folder:?}: the page starts under the bar");
        assert_eq!(under.y + under.height, whole.y + whole.height, "{folder:?}: and ends where it did");

        let offered = |window: &mut Harness, offer: &str| window.find(Role::Button, offer).is_some();
        let trusts = ["Trust this document", "Trust this folder"].map(|offer| offered(&mut window, offer));
        assert_eq!(trusts, [true, folder.is_some()], "{folder:?}");
        for (offer, request) in offers {
            window.click(Role::Button, offer);
            assert_eq!(window.ui().take_requests(), [request], "{folder:?}: {offer}");
        }
        assert!(bar(&mut window).is_some(), "{folder:?}: the shell says when the document is trusted");
        window.ui().set_safe_mode(None);
        assert!(bar(&mut window).is_none(), "{folder:?}");
        assert_eq!(page(&mut window).bounds, whole, "{folder:?}: the page has its room back");
    }
}

#[test]
fn the_end_of_a_long_document_shows_whole_under_the_bar() {
    let mut window = Harness::doc(&"A line of the letter.\n\n".repeat(80));
    window.ui().set_safe_mode(Some(SafeMode::default()));
    window.key("Mod+End").paint();
    let (caret, page) = (window.ui().caret_rect().expect("the page shows a caret"), page(&mut window).bounds);
    assert!(caret.y + caret.height <= page.y + page.height, "the last line is in view: {caret:?} in {page:?}");
}

#[test]
fn the_bar_is_put_away_to_a_button_that_brings_it_back() {
    let mut window = letter();
    window.ui().set_safe_mode(Some(SafeMode::default()));
    assert!(window.find(Role::Button, "Safe mode").is_none(), "the bar says it already");

    window.click(Role::Button, "Hide");
    assert!(bar(&mut window).is_none());
    window.click(Role::Button, "Safe mode");
    assert!(bar(&mut window).is_some());
    assert!(window.find(Role::Button, "Safe mode").is_none());

    // A document that is trusted has neither, and the next one that is held shows its bar.
    window.click(Role::Button, "Hide");
    window.ui().set_safe_mode(None);
    assert!(bar(&mut window).is_none() && window.find(Role::Button, "Safe mode").is_none());
    window.ui().set_safe_mode(Some(SafeMode::default()));
    assert!(bar(&mut window).is_some());
}

#[test]
fn a_link_pressed_with_ctrl_or_cmd_is_the_shells_to_open() {
    // The text that is pressed, whether Ctrl or Cmd is held, and the address the shell is asked to open.
    let cases = [
        ("written", true, Some(HREF)),
        ("written", false, None),
        ("covers", true, None),
        // A place in this document is not the shell's to open.
        ("the plan", true, None),
    ];
    for (text, held, asked) in cases {
        let mut window = letter();
        let at = on(&mut window, text);
        window.select("Reply");
        let before = window.editor().selection().cloned();
        match held {
            true => window.mod_click_at(at),
            false => window.click_at(at),
        };
        let expected: Vec<Request> = asked.map(|href| open(href, false)).into_iter().collect();
        assert_eq!(links(&mut window), expected, "{text}, held: {held}");
        if asked.is_some() {
            assert_eq!(window.editor().selection().cloned(), before, "{text}: the caret stays where it was");
        }
    }
}

#[test]
fn the_link_editor_opens_the_address_in_its_field() {
    let mut window = letter();
    window.select("written").key("Mod+K").click(Role::Button, "Open link");
    assert_eq!(links(&mut window), [open(HREF, false)]);
    window.assert_closed(Role::Dialog, "Link");

    // With nothing in the field there is nothing to open.
    window.select("Reply").key("Mod+K");
    assert!(window.find(Role::Button, "Open link").is_some_and(|button| button.disabled));
}

#[test]
fn a_link_the_shell_asks_about_shows_its_whole_address_and_opens_on_a_yes() {
    // How the card is left, and whether the link is asked for again, as one the person agreed to.
    let cases = [("Open link", true), ("Cancel", false), ("Escape", false)];
    for (left, agreed) in cases {
        let mut window = letter();
        window.ui().ask_link(HREF);
        window.assert_open(Role::Dialog, "Open this link?");
        let shown = window.controls().into_iter().any(|found| found.role == Role::Note && found.name == HREF);
        assert!(shown, "{left}: the whole address is there to read");
        match left {
            "Escape" => window.key("Escape"),
            button => window.click(Role::Button, button),
        };
        window.assert_closed(Role::Dialog, "Open this link?");
        let expected: Vec<Request> = agreed.then(|| open(HREF, true)).into_iter().collect();
        assert_eq!(links(&mut window), expected, "{left}");
    }
}

#[test]
fn a_link_the_shell_refuses_says_why_and_has_nothing_to_open() {
    let mut window = letter();
    let why = "It would run a script instead of visiting a page.";
    window.ui().refuse_link("javascript:alert(1)", why);
    window.assert_open(Role::Dialog, "This link was not opened");
    assert!(window.controls().iter().any(|found| found.name == why), "the reason is there to read");
    assert!(window.find(Role::Button, "Open link").is_none());

    window.click(Role::Button, "Close").assert_closed(Role::Dialog, "This link was not opened");
    assert_eq!(links(&mut window), []);
}
