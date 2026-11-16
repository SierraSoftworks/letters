//! What a shell has the window say about something that happened once: a sentence over the foot of the page
//! with one button, which stays until the person answers it.

use letters_render::{Item, Point};
use letters_test::{Found, Harness, Role};
use letters_ui::{Notice, Request};

/// What the desktop says of unsaved changes that could not be brought back, from the shortest to the longest.
const SAID: [&str; 3] = [
    "Some unsaved changes could not be brought back.",
    "Unsaved changes could not be brought back, so this document is shown as it was last saved.",
    "Letters closed unexpectedly while bringing back unsaved changes, so this document is shown as it was last saved.",
];

fn letter() -> Harness {
    let mut window = Harness::doc("# Rates\n\nRates change in January.");
    window.resize(1000.0, 700.0);
    window
}

fn notice(sentence: &str, key: &str) -> Notice {
    Notice { sentence: sentence.into(), button: "Show file".into(), key: key.into() }
}

/// The notice that shows, if one does.
fn told(window: &mut Harness) -> Option<Found> {
    window.controls().into_iter().find(|found| found.role == Role::Status && SAID.contains(&found.name.as_str()))
}

/// What the buttons of the notice asked of the shell.
fn pressed(window: &mut Harness) -> Vec<String> {
    let requests = window.ui().take_requests().into_iter().filter_map(|request| match request {
        Request::NoticePressed(key) => Some(key),
        _ => None,
    });
    requests.collect()
}

#[test]
fn a_notice_stays_until_its_button_is_pressed_or_it_is_put_away() {
    for (answer, asked) in [("Show file", vec!["journal-1".to_owned()]), ("Dismiss", vec![])] {
        let mut window = letter();
        assert_eq!(told(&mut window), None, "nothing is said until the shell says it");
        window.ui().notify(notice(SAID[0], "journal-1"));
        assert_eq!(told(&mut window).map(|told| told.name).as_deref(), Some(SAID[0]));
        assert!(window.find(Role::Button, "Show file").is_some(), "the button is called as the shell called it");

        // It says something that happened once, so it does not go by itself.
        window.advance(0).advance(60_000);
        assert!(told(&mut window).is_some(), "{answer}: still there after a minute");
        assert_eq!(pressed(&mut window), Vec::<String>::new(), "{answer}: nothing is asked until it is pressed");

        window.click(Role::Button, answer);
        assert_eq!(pressed(&mut window), asked, "{answer}: the shell is told which notice was pressed");
        assert_eq!(told(&mut window), None, "{answer}: answered, it goes");
        assert!(window.find(Role::Button, "Show file").is_none());
    }
}

#[test]
fn notices_show_one_after_another_and_one_said_again_takes_its_own_place() {
    let mut window = letter();
    window.ui().notify(notice(SAID[0], "first"));
    window.ui().notify(notice(SAID[1], "second"));
    assert_eq!(told(&mut window).map(|told| told.name).as_deref(), Some(SAID[0]), "the first said shows first");

    // Said again, a notice is not said twice.
    window.ui().notify(notice(SAID[2], "first"));
    assert_eq!(told(&mut window).map(|told| told.name).as_deref(), Some(SAID[2]));
    window.click(Role::Button, "Dismiss");
    assert_eq!(told(&mut window).map(|told| told.name).as_deref(), Some(SAID[1]), "then the next");
    window.click(Role::Button, "Show file");
    assert_eq!(pressed(&mut window), ["second"]);
    assert_eq!(told(&mut window), None, "and then none");
}

#[test]
fn a_notice_says_its_whole_sentence_inside_any_window() {
    for width in [1000.0, 360.0] {
        for sentence in SAID {
            let mut window = letter();
            window.resize(width, 700.0);
            window.ui().notify(notice(sentence, "journal-1"));
            let toast = told(&mut window).expect("the notice shows").bounds;
            assert!(toast.x >= 0.0 && toast.x + toast.width <= width, "{width}: inside the window: {toast:?}");
            assert!(toast.y + toast.height <= 700.0, "{width}: {toast:?}");

            // Nothing of the sentence is cut off: every letter of it is written, over as many lines as it needs.
            let [show, dismiss] =
                ["Show file", "Dismiss"].map(|name| window.find(Role::Button, name).expect(name).bounds);
            let runs = window.paint().items.into_iter().filter_map(|item| match item {
                Item::Glyphs { origin, glyphs, .. } if toast.contains(origin) && !show.contains(origin) => {
                    Some(glyphs.len())
                }
                _ => None,
            });
            let written: usize = runs.sum();
            let letters = sentence.chars().filter(|letter| !letter.is_whitespace()).count();
            assert!(written >= letters, "{width}: {written} of {letters} letters of {sentence:?}");
            for (name, button) in [("Show file", show), ("Dismiss", dismiss)] {
                let inside = toast.contains(Point { x: button.x, y: button.y })
                    && toast.contains(Point { x: button.x + button.width, y: button.y + button.height });
                assert!(inside, "{width}: {name} stands on the notice: {button:?} in {toast:?}");
            }
        }
    }
}

#[test]
fn the_page_under_a_notice_stays_the_persons() {
    let mut window = letter();
    window.ui().notify(notice(SAID[0], "journal-1"));
    let toast = told(&mut window).expect("the notice shows").bounds;

    // What is typed goes to the document, and a press beside the notice to the page.
    let before = window.editor().model().revision();
    window.type_text("New ");
    assert_ne!(window.editor().model().revision(), before, "typing writes");
    window.click_at(Point { x: toast.x - 40.0, y: toast.y - 40.0 });
    assert!(told(&mut window).is_some(), "and the notice stays through both");

    // It lies over the page, and waits while Print view has the window.
    window.key("Mod+P");
    assert_eq!(told(&mut window), None);
    window.key("Escape");
    assert!(told(&mut window).is_some(), "back on the page, it is still to be answered");

    // The keyboard comes to its button straight from the page.
    window.key("Tab");
    assert_eq!(window.focused().map(|found| found.name).as_deref(), Some("Show file"));
    window.key("Enter");
    assert_eq!(pressed(&mut window), ["journal-1"]);

    // It is said before the toast that counts what an import left, which follows it.
    window.ui().notify(notice(SAID[1], "journal-2"));
    window.ui().say_imported();
    let imported = |window: &mut Harness| window.controls().into_iter().any(|found| found.name.starts_with("Imported"));
    assert!(told(&mut window).is_some() && !imported(&mut window), "one thing is said at a time");
    window.click(Role::Button, "Dismiss");
    assert!(told(&mut window).is_none() && imported(&mut window), "the import's toast shows once the notice went");
}
