//! The history: the document's versions listed by day beside the one that is picked, which is read where
//! the page was and can be restored.

use letters_editor::Editor;
use letters_model::{Block, ReplicaId};
use letters_render::{Item, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::Window;

/// Midnight before Sunday 15 November 2026 in UTC, in seconds since 1970.
const SUNDAY: u64 = 1_794_700_800;
const MINUTE: u64 = 60;
const HOUR: u64 = 3_600;
const DAY: u64 = 86_400;
/// The room the page area keeps over its first line and under its last, less a little.
const ROOM: f32 = 40.0;

/// What each version of the letter is listed as and what its two paragraphs read, oldest first.
const VERSIONS: [(&str, [&str; 2]); 5] = [
    ("First version", ["The roof comes first. Costs are unknown.", "The floor follows."]),
    ("18:20", ["The roof comes first. Costs are rising.", "The floor follows."]),
    ("11:30", ["The roof comes first. Costs are rising.", "The kitchen floor follows."]),
    ("16:42", ["The north roof comes first. Costs are rising.", "The kitchen floor follows."]),
    ("Current version", ["The north roof comes first. Costs are rising.", "The kitchen floor follows in May."]),
];

/// Writes `new` over the first `old` in the window's document, at a time of day.
fn write(window: &mut Harness, at: u64, old: &str, new: &str) {
    window.platform.wall = Some(at);
    window.select(old).type_text(new);
}

/// Has another writer change the document on a device of their own, and brings the change in.
fn merge(window: &mut Harness, writer: &str, at: u64, old: &str, new: &str) {
    let copy = window.editor().model().fork(ReplicaId::from(2));
    let mut theirs = Harness::with(Window::new(Editor::with_model(copy)));
    theirs.ui().set_author(writer);
    write(&mut theirs, at, old, new);
    let history = theirs.editor().model().save();
    window.ui().change(|editor| editor.merge(&history)).expect("a history of the same document");
}

/// The letter Sam has open at twenty past nine on that Sunday. It was begun without a date, Sam wrote on
/// it on Friday 6 November and on Tuesday morning, Alice on Tuesday afternoon, and Sam again this morning.
fn letter() -> Harness {
    let mut sam = Harness::doc(&format!("# Cottage renovation\n\n{}\n\n{}", VERSIONS[0].1[0], VERSIONS[0].1[1]));
    sam.ui().set_author("Sam Okafor");
    write(&mut sam, SUNDAY - 9 * DAY + 18 * HOUR + 20 * MINUTE, "unknown", "rising");
    write(&mut sam, SUNDAY - 5 * DAY + 11 * HOUR + 30 * MINUTE, "floor", "kitchen floor");
    merge(&mut sam, "Alice Moreau", SUNDAY - 5 * DAY + 16 * HOUR + 42 * MINUTE, "roof", "north roof");
    write(&mut sam, SUNDAY + 9 * HOUR + 12 * MINUTE, "follows", "follows in May");
    sam.platform.wall = Some(SUNDAY + 9 * HOUR + 20 * MINUTE);
    sam
}

/// The letter with its history showing.
fn history() -> Harness {
    let mut window = letter();
    window.click(Role::Button, "History");
    window
}

/// A survey long enough to scroll, written on twice.
fn survey() -> Harness {
    // Each line is longer than the page is wide.
    let found = "says what was found on that part of the roof, and what the roofer made of it when he came down";
    let lines: Vec<String> = (1..=60).map(|line| format!("Line {line} of the survey {found}.")).collect();
    let mut window = Harness::doc(&format!("# Survey\n\n{}", lines.join("\n\n")));
    write(&mut window, SUNDAY + HOUR, "Line 1 ", "The first line ");
    write(&mut window, SUNDAY + 3 * HOUR, "Line 60 ", "The last line ");
    window.platform.wall = Some(SUNDAY + 4 * HOUR);
    window.click(Role::Button, "History");
    window
}

/// The versions the panel lists, newest first.
fn versions(window: &mut Harness) -> Vec<Found> {
    window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption).collect()
}

/// What the version that is picked is listed as.
fn picked(window: &mut Harness) -> Option<String> {
    versions(window).into_iter().find(|version| version.selected == Some(true)).map(|version| version.name)
}

/// The paragraphs of the version that shows where the page was.
fn shown(window: &mut Harness) -> Vec<String> {
    let runs = window.controls().into_iter().filter(|found| found.role == Role::TextRun);
    // The first run is the letter's heading.
    runs.filter_map(|run| run.value).map(|text| text.trim_end().to_owned()).skip(1).collect()
}

/// The paragraphs of the document itself.
fn written(window: &Harness) -> Vec<String> {
    window.editor().doc().blocks.iter().skip(1).map(Block::plain_text).collect()
}

/// Whether the history shows, and not the page.
fn showing(window: &mut Harness) -> bool {
    window.find(Role::Complementary, "History").is_some()
}

/// The room the text in the page area keeps from the area's edges: over its first line and under its
/// last, and from the left and the right to where its lines start and its last letters stand.
struct Kept {
    over: f32,
    under: f32,
    left: f32,
    right: f32,
}

fn kept(window: &mut Harness) -> Kept {
    let area: Rect = window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page").bounds;
    let (mut down, mut along) = (Vec::new(), Vec::new());
    for item in window.paint().items {
        let Item::Glyphs { origin, glyphs, .. } = item else { continue };
        if area.contains(origin) {
            down.push(origin.y - area.y);
            along.extend(glyphs.iter().map(|glyph| origin.x + glyph.x - area.x));
        }
    }
    let (least, most) = (
        |of: &[f32]| of.iter().copied().fold(f32::MAX, f32::min),
        |of: &[f32]| of.iter().copied().fold(f32::MIN, f32::max),
    );
    Kept { over: least(&down), under: area.height - most(&down), left: least(&along), right: area.width - most(&along) }
}

/// A way of doing something to a window.
type Act = fn(&mut Harness);
/// A way of picking the version that is so many back from the newest.
type Pick = fn(&mut Harness, usize);

const PICKS: [(&str, Pick); 2] = [
    ("a press", |window, back| {
        window.click(Role::ListBoxOption, VERSIONS[VERSIONS.len() - 1 - back].0);
    }),
    ("the arrow keys", |window, back| {
        for _ in 0..back {
            window.key("Left");
        }
    }),
];

#[test]
fn the_history_lists_the_versions_under_their_days_with_the_newest_picked() {
    let mut window = history();
    let days: Vec<String> =
        window.controls().into_iter().filter(|found| found.role == Role::ListBox).map(|day| day.name).collect();
    assert_eq!(days, ["Today", "Tuesday", "6 Nov", "Undated"]);

    let listed = versions(&mut window);
    let names: Vec<&str> = listed.iter().map(|version| version.name.as_str()).collect();
    let expected: Vec<&str> = VERSIONS.iter().rev().map(|(name, _)| *name).collect();
    assert_eq!(names, expected);
    // Each says who wrote it. The time of one that is not listed by its time comes after the writer.
    let told = |at: usize| listed[at].description.clone().unwrap_or_default();
    assert!(told(0).starts_with("Sam Okafor, 09:12"), "{}", told(0));
    assert!(told(1).starts_with("Alice Moreau") && !told(1).contains("16:42"), "{}", told(1));
    assert!(told(2).starts_with("Sam Okafor"), "{}", told(2));

    assert_eq!(picked(&mut window).as_deref(), Some("Current version"));
    assert_eq!(shown(&mut window), VERSIONS[4].1, "the document shows as it reads now");
    let restore = window.find(Role::Button, "Restore this version").expect("a button that restores");
    assert!(restore.disabled, "there is nothing to restore of the document as it is");
}

#[test]
fn a_version_that_is_picked_shows_as_it_read_then() {
    // How far back from the newest the version is, and what the bar says of it.
    let cases = [
        (1, "Tuesday 16:42, by Alice Moreau"),
        (2, "Tuesday 11:30, by Sam Okafor"),
        (3, "6 Nov 18:20, by Sam Okafor"),
        (4, "Undated"),
    ];
    for (way, pick) in PICKS {
        for (back, said) in cases {
            let mut window = history();
            let before = window.markdown();
            pick(&mut window, back);
            let (name, reads) = VERSIONS[VERSIONS.len() - 1 - back];
            assert_eq!(shown(&mut window), reads, "{way}: {name}");
            assert_eq!(picked(&mut window).as_deref(), Some(name), "{way}");
            assert!(window.find(Role::Label, said).is_some(), "{way}: the bar says {said}");
            let restore = window.find(Role::Button, "Restore this version").expect("a button that restores");
            assert!(!restore.disabled, "{way}: {name} can be restored");
            assert_eq!(window.markdown(), before, "{way}: the document itself is as it was");
        }
    }
}

#[test]
fn the_arrow_keys_step_through_the_versions_and_stop_at_the_ends() {
    // The keys pressed from the newest version, and the version that is picked after them.
    let cases: [(&[&str], &str); 4] = [
        (&["Right"], "Current version"),
        (&["Left", "Left", "Right"], "16:42"),
        (&["Left"; 4], "First version"),
        (&["Left"; 7], "First version"),
    ];
    for (keys, expected) in cases {
        let mut window = history();
        for key in keys {
            window.key(key);
        }
        assert_eq!(picked(&mut window).as_deref(), Some(expected), "{keys:?}");
    }
}

#[test]
fn days_and_times_are_told_on_the_persons_clock() {
    // How far the clock is ahead of UTC in minutes, and the day and time of what Alice wrote on Tuesday at
    // 16:42 in UTC.
    let cases =
        [(0, "Tuesday", "16:42"), (120, "Tuesday", "18:42"), (-300, "Tuesday", "11:42"), (840, "Wednesday", "06:42")];
    for (offset, day, time) in cases {
        let mut window = letter();
        window.ui().set_utc_offset(offset);
        window.click(Role::Button, "History").click(Role::ListBoxOption, time);
        assert_eq!(shown(&mut window), VERSIONS[3].1, "{offset}: {time} is what Alice wrote");
        let said = format!("{day} {time}, by Alice Moreau");
        assert!(window.find(Role::Label, &said).is_some(), "{offset}: the bar says {said}");
    }
}

#[test]
fn restoring_a_version_makes_the_document_read_as_it_did_in_one_step_that_undo_takes_back() {
    let mut window = history();
    let before = window.markdown();
    window.click(Role::ListBoxOption, "11:30").click(Role::Button, "Restore this version");
    assert!(!showing(&mut window), "the page is back, to be written on");
    assert_eq!(written(&window), VERSIONS[2].1);

    window.key("Mod+z");
    assert_eq!(window.markdown(), before, "one step back is the document as it was");
    // The newest version is not restored: its button takes no press.
    window.click(Role::Button, "History").click(Role::Button, "Restore this version");
    assert!(showing(&mut window));
    assert_eq!(window.markdown(), before);
}

#[test]
fn nothing_is_written_while_a_version_shows_and_the_way_back_returns_to_the_page() {
    let ways: [(&str, Act); 2] = [
        ("the button", |window| {
            window.click(Role::Button, "Back to editing");
        }),
        ("Escape", |window| {
            window.key("Escape");
        }),
    ];
    for (way, back) in ways {
        let mut window = letter();
        window.click(Role::Button, "Notes").click(Role::Button, "History");
        assert!(window.find(Role::Complementary, "Notes").is_none(), "the versions are listed where the panel was");
        let before = window.markdown();
        window.key("Left").type_text("x").key("Backspace").key("Enter").key("Mod+z");
        assert_eq!(window.markdown(), before, "{way}: a version is read, not written on");
        assert_eq!(window.ui().caret_rect(), None, "{way}: and has no caret");
        let page = window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page");
        assert!(!window.long_press_at(page.center()), "{way}: nor a word to select by touch");

        back(&mut window);
        assert!(!showing(&mut window), "{way}");
        assert!(window.find(Role::Complementary, "Notes").is_some(), "{way}: the panel that was open is back");
        assert_eq!(window.markdown(), before, "{way}: the document is as it was");
        window.type_text("!");
        assert!(window.markdown().contains("May!"), "{way}: the page is written on again, where the caret was");
    }
}

#[test]
fn edits_that_come_in_under_the_history_are_listed_and_the_picked_version_stays() {
    let mut window = history();
    window.click(Role::ListBoxOption, "11:30");
    let listed = versions(&mut window).len();
    merge(&mut window, "Alice Moreau", SUNDAY + 9 * HOUR + 40 * MINUTE, "May", "June");
    assert_eq!(versions(&mut window).len(), listed + 1, "what came in is a version");
    assert_eq!(picked(&mut window).as_deref(), Some("11:30"));
    assert_eq!(shown(&mut window), VERSIONS[2].1, "the version that was being read still shows");

    window.click(Role::ListBoxOption, "Current version");
    assert!(shown(&mut window)[1].ends_with("in June."), "the newest version has what came in");
    assert!(window.find(Role::Label, "Today 09:40, by Alice Moreau").is_some());
}

#[test]
fn a_long_version_scrolls_and_the_next_version_shows_the_same_part() {
    // What scrolls the version down from its start.
    let ways: [(&str, Act); 3] = [
        ("End", |window| {
            window.key("End");
        }),
        ("Page Down", |window| {
            window.key("PageDown");
        }),
        ("the wheel", |window| {
            let page = window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page");
            window.scroll_at(page.center(), 300.0);
        }),
    ];
    for (way, scroll) in ways {
        let mut window = survey();
        assert!(kept(&mut window).over >= ROOM, "{way}: a version opens at its start");
        scroll(&mut window);
        assert!(kept(&mut window).over < ROOM, "{way}: the start has gone up out of view");
        window.key("Left");
        assert_eq!(picked(&mut window).as_deref(), Some("01:00"), "{way}");
        assert!(kept(&mut window).over < ROOM, "{way}: the version before shows the same part");
        window.key("Home");
        assert!(kept(&mut window).over >= ROOM, "{way}: Home is the start again");
    }

    let mut window = survey();
    window.key("End").key("PageDown");
    assert!(kept(&mut window).under >= ROOM, "the end of a version keeps its room under the last line");
}

#[test]
fn a_version_is_set_again_for_a_window_of_another_width() {
    let mut window = survey();
    for width in [1280.0, 760.0, 1100.0] {
        window.resize(width, 800.0);
        let Kept { left, right, .. } = kept(&mut window);
        assert!(left >= 20.0 && right >= 0.0, "{width}: the text is inside the page area, {left} and {right} from it");
        // Its longest line ends about as far from the right as its lines start from the left.
        assert!(right < left + 80.0, "{width}: the text takes the width there is, {left} and {right} from the edges");
    }
}
