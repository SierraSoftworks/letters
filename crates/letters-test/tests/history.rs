//! The history: the document's versions listed by day beside the one that is picked, which is read where
//! the page was, shows how it differs from the version before it or from the document, and can be restored
//! or given a name. A strip under the page has a cell for each version.

use letters_editor::Editor;
use letters_model::{Block, Change, ReplicaId, TagKind};
use letters_render::{Color, Item, Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::{Window, tokens};

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

/// Has another writer change the document on a device of their own, and brings the changes in.
fn merged(window: &mut Harness, writer: &str, change: impl FnOnce(&mut Harness)) {
    let copy = window.editor().model().fork(ReplicaId::from(2));
    let mut theirs = Harness::with(Window::new(Editor::with_model(copy)));
    theirs.ui().set_author(writer);
    change(&mut theirs);
    let history = theirs.editor().model().save();
    window.ui().change(|editor| editor.merge(&history)).expect("a history of the same document");
}

fn merge(window: &mut Harness, writer: &str, at: u64, old: &str, new: &str) {
    merged(window, writer, |theirs| write(theirs, at, old, new));
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

/// A survey long enough to scroll, written on twice. The second time a line was added at its end.
fn survey() -> Harness {
    // Each line is longer than the page is wide.
    let found = "says what was found on that part of the roof, and what the roofer made of it when he came down";
    let lines: Vec<String> = (1..=60).map(|line| format!("Line {line} of the survey {found}.")).collect();
    let mut window = Harness::doc(&format!("# Survey\n\n{}", lines.join("\n\n")));
    write(&mut window, SUNDAY + HOUR, "Line 1 ", "The first line ");
    write(&mut window, SUNDAY + 3 * HOUR, "Line 60 ", "The last line ");
    window.key("Mod+End").key("Enter").type_text("The roofer signed the survey.");
    window.platform.wall = Some(SUNDAY + 4 * HOUR);
    window.click(Role::Button, "History");
    window
}

/// What the plan read when it was begun, block by block.
const PLAN: [&str; 6] = [
    "# Cottage renovation",
    "The cottage needs three jobs done. Costs are still unknown.",
    "## Roof first",
    "The quote covers slates and battens.",
    "The scaffold is hired by the week.",
    "The old boards come up. The joists are sound, so the new floor goes straight on.",
];
/// What its blocks under the heading read once Alice had written on it.
const HERS: [&str; 6] = [
    "The cottage needs three jobs done. This plan keeps them in order.",
    "Roof first",
    "The quote covers slates and battens and the lead flashing.",
    "The old pine boards come up. The new floor goes straight on.",
    "Budget",
    "Roof and gutters come to 8,400.",
];

/// A plan with its history showing, and what Alice wrote on Tuesday afternoon picked. Sam wrote on it
/// that morning. Then Alice rewrote the end of its first paragraph, added to the quote, took the scaffold
/// out and some words about the floor, and added a budget. Sam wrote on it again this morning.
fn plan() -> Harness {
    let mut sam = Harness::doc(&PLAN.join("\n\n"));
    sam.ui().set_author("Sam Okafor");
    write(&mut sam, SUNDAY - 5 * DAY + 11 * HOUR + 30 * MINUTE, "old boards", "old pine boards");
    merged(&mut sam, "Alice Moreau", |alice| {
        write(
            alice,
            SUNDAY - 5 * DAY + 16 * HOUR + 42 * MINUTE,
            "Costs are still unknown.",
            "This plan keeps them in order.",
        );
        alice.select("battens").type_text("battens and the lead flashing");
        alice.select(PLAN[4]).key("Backspace").key("Backspace");
        alice.select("The joists are sound, so the").type_text("The");
        alice.select("straight on.").key("Right").key("Enter").type_text("## Budget");
        alice.key("Enter").type_text("Roof and gutters come to 8,400.");
    });
    write(&mut sam, SUNDAY + 9 * HOUR + 12 * MINUTE, "pine boards", "wide pine boards");
    sam.platform.wall = Some(SUNDAY + 9 * HOUR + 20 * MINUTE);
    sam.click(Role::Button, "History").click(Role::ListBoxOption, "16:42");
    sam
}

/// What the version that shows says of each block that differs, with what the block reads, down the page.
fn differing(window: &mut Harness) -> Vec<(String, String)> {
    let controls = window.controls();
    let blocks = controls.iter().enumerate().filter(|(_, found)| matches!(found.role, Role::Paragraph | Role::Heading));
    let said = blocks.filter_map(|(at, block)| {
        let reads = controls[at..].iter().find(|found| found.role == Role::TextRun)?.value.clone()?;
        Some((block.description.clone()?, reads.trim_end().to_owned()))
    });
    said.collect()
}

/// A rewritten block as it read before, which shows beside the block or over it.
fn before(window: &mut Harness) -> Option<Found> {
    window.controls().into_iter().find(|found| found.role == Role::Note && found.name.starts_with("Before"))
}

/// How many stretches of words are marked as only the earlier version's, and as only the later one's.
fn marked(window: &mut Harness) -> [usize; 2] {
    let fills: Vec<Color> = window
        .paint()
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fill { color, .. } => Some(color),
            _ => None,
        })
        .collect();
    [tokens::WARN_SOFT, tokens::INFO_SOFT].map(|ink| {
        let tint = Color::rgba(ink.light.r, ink.light.g, ink.light.b, ink.light.a);
        fills.iter().filter(|color| **color == tint).count()
    })
}

/// The part of the window the version shows in.
fn area(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page").bounds
}

/// Whether `inner` shows whole between the left and right edges of `outer`.
fn across(inner: Rect, outer: Rect) -> bool {
    inner.x >= outer.x && inner.x + inner.width <= outer.x + outer.width
}

/// The versions the panel lists, newest first.
fn versions(window: &mut Harness) -> Vec<Found> {
    window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption).collect()
}

/// What the version that is picked is listed as.
fn picked(window: &mut Harness) -> Option<String> {
    versions(window).into_iter().find(|version| version.selected == Some(true)).map(|version| version.name)
}

/// The strip of versions under the page.
fn strip(window: &mut Harness) -> Found {
    window.find(Role::Image, "Versions, oldest first").expect("the strip of versions")
}

/// The point of the strip that is so many versions across it, counted from the oldest: 2.5 is the middle
/// of the third.
fn cell(window: &mut Harness, versions: f32) -> Point {
    let strip = strip(window);
    let wide = strip.bounds.width / VERSIONS.len() as f32;
    Point { x: strip.bounds.x + versions * wide, y: strip.center().y }
}

/// How many versions the strip draws as standing out from the rest.
fn standing_out(window: &mut Harness) -> usize {
    let (strip, ink) = (strip(window).bounds, tokens::INFO.light);
    let tone = Color::rgba(ink.r, ink.g, ink.b, ink.a);
    let on_strip = |rect: Rect| strip.contains(Point { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 });
    let fills = window.paint().items.into_iter().filter_map(|item| match item {
        Item::Fill { rect, color, .. } => Some((rect, color)),
        _ => None,
    });
    fills.filter(|(rect, color)| *color == tone && on_strip(*rect)).count()
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
/// How far into the page area a press is, given how much of the earlier version shows at its left.
type Reach = fn(f32) -> f32;

const PICKS: [(&str, Pick); 3] = [
    ("a press on the strip", |window, back| {
        let at = cell(window, (VERSIONS.len() - 1 - back) as f32 + 0.5);
        window.click_at(at);
    }),
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

#[test]
fn a_version_shows_what_it_changed_from_the_version_before_it() {
    let mut window = plan();
    let document = window.markdown();
    // What is said of each block that differs, and some of what the block reads.
    let expected = [
        ("After, by Alice Moreau, Tuesday 16:42", "This plan keeps them in order."),
        ("Added: and the lead flashing", "battens and the lead flashing."),
        ("Removed by Alice Moreau", PLAN[4]),
        ("Removed: joists are sound, so the", "The joists are sound, so the new floor"),
        ("Added by Alice Moreau", "Budget"),
        ("Added by Alice Moreau", "Roof and gutters"),
    ];
    let found = differing(&mut window);
    assert_eq!(found.len(), expected.len(), "{found:?}");
    for ((said, reads), (says, holds)) in found.iter().zip(expected) {
        assert!(said == says && reads.contains(holds), "{said:?} of {reads:?}, for {says:?} of {holds:?}");
    }
    // The paragraph that was rewritten shows as it read before as well, under the name of who wrote that.
    let before = before(&mut window).expect("the paragraph as it read before");
    assert_eq!(before.name, "Before, by Sam Okafor, Tuesday 11:30");
    assert_eq!(before.value.as_deref(), Some(PLAN[1]));
    let [gone, came] = marked(&mut window);
    assert!(gone >= 2 && came >= 2, "words of both versions are marked: {gone} and {came}");
    assert_eq!(window.markdown(), document, "the document itself is as it was");
}

#[test]
fn a_version_compared_with_the_document_shows_what_has_changed_since() {
    let mut window = plan();
    window.click(Role::ListBoxOption, "11:30").click(Role::RadioButton, "Current");
    let found = differing(&mut window);
    let says = |said: &str, holds: &str| found.iter().any(|(told, reads)| told == said && reads.contains(holds));
    assert!(says("After, by Sam Okafor, Today 09:12", "This plan keeps them in order."), "{found:?}");
    assert!(says("Removed later", PLAN[4]), "{found:?}");
    assert!(says("Added later", "Budget"), "{found:?}");
    // The version that is picked is the earlier of the two.
    let before = before(&mut window).expect("the paragraph as the picked version has it");
    assert_eq!(before.name, "Before, by Sam Okafor, Tuesday 11:30");

    // What comes in while it shows is part of the document it is compared with.
    merge(&mut window, "Alice Moreau", SUNDAY + 9 * HOUR + 40 * MINUTE, "8,400", "9,100");
    assert!(differing(&mut window).iter().any(|(said, reads)| said == "Added later" && reads.contains("9,100")));
    assert_eq!(picked(&mut window).as_deref(), Some("11:30"));
}

#[test]
fn a_version_with_nothing_to_compare_it_with_shows_as_it_read() {
    // What is picked and pressed, and what the version then reads under its heading.
    let cases: [(&str, Act, &[&str]); 3] = [
        ("compared with nothing", |window| _ = window.click(Role::RadioButton, "Off"), &HERS),
        ("the first version", |window| _ = window.click(Role::ListBoxOption, "First version"), &PLAN[1..]),
        (
            "the document against itself",
            |window| {
                window.click(Role::ListBoxOption, "Current version").click(Role::RadioButton, "Current");
            },
            &[],
        ),
    ];
    for (name, pick, reads) in cases {
        let mut window = plan();
        pick(&mut window);
        assert_eq!(differing(&mut window), [], "{name}: nothing is said to differ");
        assert!(before(&mut window).is_none(), "{name}: no block shows twice");
        assert_eq!(marked(&mut window), [0, 0], "{name}: no words are marked");
        // A heading is written here with the signs that Markdown makes one from.
        let expected: Vec<&str> = reads.iter().map(|text| text.trim_start_matches(['#', ' '])).collect();
        if !expected.is_empty() {
            assert_eq!(shown(&mut window), expected, "{name}: only what the version itself has shows");
        }
    }
}

#[test]
fn both_versions_of_a_rewritten_block_stand_side_by_side_while_the_window_has_room_for_them() {
    // How wide the window is, whether the earlier version shows whole, and whether it takes the page's
    // whole width, as the upper one of a stack does.
    let cases = [(1720.0, true, false), (1440.0, false, false), (900.0, true, true)];
    for (width, whole, stacked) in cases {
        let mut window = plan();
        window.resize(width, 800.0);
        let (area, before) = (area(&mut window), before(&mut window).expect("the earlier version").bounds);
        assert_eq!(across(before, area), whole, "{width}: {before:?} in {area:?}");
        assert_eq!(before.width > 0.9 * area.width, stacked, "{width}: {before:?} in {area:?}");
        assert!(before.y > area.y, "{width}: under the heading, which both versions have");
    }
}

#[test]
fn a_version_the_window_cuts_off_comes_across_when_it_is_pressed() {
    // Where the press is: how far into the page area from its left edge, given how much of the earlier
    // version shows there.
    let ways: [(&str, Reach); 2] = [
        ("what shows of the version", |showing| showing / 2.0),
        ("the round button at the window's edge", |_| tokens::SPACE_LG + 16.0),
    ];
    for (way, across_to) in ways {
        let mut window = plan();
        window.resize(1440.0, 800.0);
        let whole = |window: &mut Harness| across(before(window).expect("the earlier version").bounds, area(window));
        assert!(!whole(&mut window), "{way}: the later version is the one that shows whole at first");

        let (area, before) = (area(&mut window), before(&mut window).expect("the earlier version").bounds);
        let x = area.x + across_to(before.x + before.width - area.x);
        window.click_at(Point { x, y: before.y + before.height / 2.0 });
        window.advance(16).advance(400);
        assert!(whole(&mut window), "{way}: the pair slid across");
        // Another version starts with its later side whole again.
        window.key("Right").key("Left");
        assert!(!whole(&mut window), "{way}");
    }
}

#[test]
fn a_version_is_listed_with_how_much_it_put_in_and_took_out() {
    let mut window = plan();
    let listed = versions(&mut window);
    let told = |name: &str| {
        let version = listed.iter().find(|version| version.name == name).expect("a version of that name");
        version.description.clone().unwrap_or_default()
    };
    // Sam wrote fifteen characters over ten, and Alice both wrote and took out.
    assert!(told("11:30").ends_with("15 characters added, 10 removed"), "{}", told("11:30"));
    assert!(told("16:42").contains(" added, ") && told("16:42").ends_with(" removed"), "{}", told("16:42"));
    assert!(!told("First version").contains("removed"), "{}", told("First version"));
}

#[test]
fn the_strip_under_the_page_says_which_version_is_picked_and_follows_a_drag() {
    let mut window = history();
    // It lies under the page and the panel, and takes its room from them.
    let (page, cells) = (area(&mut window), strip(&mut window));
    assert!(cells.bounds.y >= page.y + page.height, "{:?} under {page:?}", cells.bounds);
    assert_eq!(cells.description.as_deref(), Some("5 of 5"));
    // It runs from the day of the oldest version that has one to now.
    let ends = ["6 Nov", "Now"].map(|end| window.find(Role::Label, end).map(|label| label.center().x));
    let middle = cells.center();
    assert!(ends[0].is_some_and(|x| x < middle.x) && ends[1].is_some_and(|x| x > middle.x), "{ends:?}");
    window.key("Left");
    assert_eq!(strip(&mut window).description.as_deref(), Some("4 of 5"));

    // A drag goes through the versions, and past an end of the strip stays at the version there.
    let start = cell(&mut window, 4.5);
    window.press_at(start);
    let stops =
        [(2.5, "11:30"), (0.5, "First version"), (-3.0, "First version"), (3.5, "16:42"), (9.0, "Current version")];
    for (to, version) in stops {
        let at = cell(&mut window, to);
        window.hover_at(at);
        assert_eq!(picked(&mut window).as_deref(), Some(version), "dragged to {to}");
    }
    let end = cell(&mut window, 1.5);
    window.release_at(end);
    assert_eq!(picked(&mut window).as_deref(), Some("18:20"));
    assert_eq!(shown(&mut window), VERSIONS[1].1);
    // The pointer let go: where it moves next picks nothing.
    window.hover_at(start);
    assert_eq!(picked(&mut window).as_deref(), Some("18:20"));
}

#[test]
fn a_version_is_named_where_the_bar_asks_for_it_and_the_name_is_kept_with_the_history() {
    let mut window = history();
    window.click(Role::ListBoxOption, "16:42");
    let (edits, text) = (window.ui().edits(), written(&window));
    assert_eq!(standing_out(&mut window), 0);
    window.click(Role::Button, "Name");
    let field = window.focused().expect("the keyboard is in the field");
    assert_eq!((field.role, field.name.as_str()), (Role::TextInput, "Version name"));
    window.type_text("Sent to the roofer").key("Enter");

    // The version is listed by its name and stays picked. It still says who wrote it and when.
    assert_eq!(picked(&mut window).as_deref(), Some("Sent to the roofer"));
    assert_eq!(shown(&mut window), VERSIONS[3].1);
    let named = versions(&mut window).into_iter().find(|version| version.name == "Sent to the roofer");
    let told = named.and_then(|version| version.description).unwrap_or_default();
    assert!(told.starts_with("Alice Moreau, 16:42"), "{told}");
    assert_eq!(window.find(Role::TextInput, "Version name"), None, "the field is put away");
    assert_eq!(standing_out(&mut window), 1, "the strip shows which version has a name");

    // A name is something to save, and no edit: the document reads as it did, and undo leaves the name.
    assert!(window.ui().edits() > edits);
    assert_eq!(written(&window), text);
    window.key("Escape").key("Mod+Z").click(Role::Button, "History");
    let names: Vec<String> = versions(&mut window).into_iter().map(|version| version.name).collect();
    assert!(names.contains(&"Sent to the roofer".to_owned()), "{names:?}");
}

#[test]
fn a_name_that_was_typed_is_taken_however_the_field_is_left_unless_it_is_dropped() {
    // A way of leaving the field, whether the name is taken, and whether the history still shows.
    let ways: [(&str, Act, bool, bool); 8] = [
        ("Enter", |window| _ = window.key("Enter"), true, true),
        ("Escape", |window| _ = window.key("Escape"), false, true),
        ("a press on another version", |window| _ = window.click(Role::ListBoxOption, "11:30"), true, true),
        (
            "a press on the strip",
            |window| {
                let oldest = cell(window, 0.5);
                window.click_at(oldest);
            },
            true,
            true,
        ),
        ("another comparison", |window| _ = window.click(Role::RadioButton, "Off"), true, true),
        (
            "a press on the page",
            |window| {
                let page = area(window);
                window.click_at(Point { x: page.x + page.width / 2.0, y: page.y + page.height / 2.0 });
            },
            true,
            true,
        ),
        ("the way back", |window| _ = window.click(Role::Button, "Back to editing"), true, false),
        ("a restore", |window| _ = window.click(Role::Button, "Restore this version"), true, false),
    ];
    for (way, leave, taken, stays) in ways {
        let mut window = history();
        window.click(Role::ListBoxOption, "16:42").click(Role::Button, "Name").type_text("Sent to the roofer");
        leave(&mut window);
        assert_eq!(showing(&mut window), stays, "{way}");
        if !stays {
            window.click(Role::Button, "History");
        }
        assert_eq!(window.find(Role::TextInput, "Version name"), None, "{way}: the field is put away");
        let named: Vec<Found> =
            versions(&mut window).into_iter().filter(|version| version.name == "Sent to the roofer").collect();
        assert_eq!(named.len(), usize::from(taken), "{way}");
        // The name goes to the version that was picked when it was typed, wherever the pick went then.
        let told = named.first().and_then(|version| version.description.clone()).unwrap_or_default();
        assert!(!taken || told.starts_with("Alice Moreau"), "{way}: {told}");
    }
}

#[test]
fn a_cleared_name_is_taken_off_and_a_version_the_shell_marked_keeps_its_mark_until_it_is_named() {
    let mut window = letter();
    // The shell marks what the letter began with as an import.
    let marked = window.ui().change(|editor| {
        let began = editor.timeline().into_iter().rfind(|step| step.time == 0).expect("a first step").after;
        editor.mark_version(&began, TagKind::Import, Some("Renovation.docx")).map(|()| Change::default())
    });
    assert!(marked.is_ok());
    window.click(Role::Button, "History").click(Role::ListBoxOption, "Imported from Renovation.docx");
    assert_eq!(standing_out(&mut window), 1, "the strip shows what came from elsewhere");

    // What is done at the field, and what the version is then listed as.
    let steps: [(&str, Act, &str); 4] = [
        ("nothing typed", |window| _ = window.key("Enter"), "Imported from Renovation.docx"),
        ("a name typed", |window| _ = window.type_text("Architect's draft").key("Enter"), "Architect's draft"),
        ("the name left as it is", |window| _ = window.key("Enter"), "Architect's draft"),
        ("the name cleared", |window| _ = window.key("Mod+A").key("Backspace").key("Enter"), "First version"),
    ];
    for (what, act, listed) in steps {
        let was = picked(&mut window).filter(|name| name == "Architect's draft").unwrap_or_default();
        window.click(Role::Button, "Name");
        // The field opens with the name the version has.
        assert_eq!(window.focused().and_then(|field| field.value).unwrap_or_default(), was, "{what}");
        act(&mut window);
        assert_eq!(picked(&mut window).as_deref(), Some(listed), "{what}");
        assert_eq!(shown(&mut window), VERSIONS[0].1, "{what}");
    }
}
