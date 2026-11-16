//! The history: the document's versions listed by day beside the one that is picked, which is read where
//! the page was, shows how it differs from the version before it or from the document, and can be restored
//! or given a name, and a copy of it saved as a document of its own. A strip under the page has a cell for
//! each version. A passage of a version is selected with the pointer and copied out of it. Asked for with
//! text selected on the page, the history narrows to the versions that changed what is selected.

use letters_editor::Editor;
use letters_model::{Block, Change, ReplicaId, TagKind};
use letters_render::{Color, Item, Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::{CopyFormat, Landing, Request, Window, tokens};

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

/// The letter with the history of what is selected on its page showing: a word of one of its paragraphs.
fn passage(word: &str) -> Harness {
    let mut window = letter();
    window.select(word).key("Mod+Shift+P").type_text("History of selection").key("Enter");
    window
}

/// Whether the history shows narrowed to what was selected.
fn narrowed(window: &mut Harness) -> bool {
    window.find(Role::Complementary, "History of selection").is_some()
}

/// What lies over the page area from edge to edge and lets what is under it show through.
fn veils(window: &mut Harness) -> Vec<Rect> {
    let area = area(window);
    let fills = window.paint().items.into_iter().filter_map(|item| match item {
        Item::Fill { rect, color, .. } if color.a > 64 && color.a < 255 && rect.width >= area.width => Some(rect),
        _ => None,
    });
    fills.collect()
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

/// A count that was raised once an hour for so many hours until that Sunday began, with its history
/// showing twenty minutes later.
fn counted(sittings: u64) -> Harness {
    let mut window = Harness::doc("# Count\n\nThe count is 0.");
    window.ui().set_author("Sam Okafor");
    for n in 1..=sittings {
        write(&mut window, SUNDAY - (sittings - n) * HOUR, &format!("is {}", n - 1), &format!("is {n}"));
    }
    window.platform.wall = Some(SUNDAY + 20 * MINUTE);
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

/// What is filled with one of the toolkit's colours.
fn filled(window: &mut Harness, ink: tokens::Color) -> Vec<Rect> {
    let tint = Color::rgba(ink.light.r, ink.light.g, ink.light.b, ink.light.a);
    let fills = window.paint().items.into_iter().filter_map(|item| match item {
        Item::Fill { rect, color, .. } if color == tint => Some(rect),
        _ => None,
    });
    fills.collect()
}

/// What is tinted as only the earlier version's, which is red, and as only the later one's, which is green.
fn tinted(window: &mut Harness) -> [Vec<Rect>; 2] {
    [tokens::DANGER_SOFT, tokens::OK_SOFT].map(|ink| filled(window, ink))
}

/// How many stretches of words are marked as only the earlier version's, and as only the later one's.
fn marked(window: &mut Harness) -> [usize; 2] {
    let area = area(window);
    tinted(window).map(|tinted| tinted.iter().filter(|rect| rect.width < area.width).count())
}

/// The bands across the page area that hold what only the earlier version has, and what only the later.
fn bands(window: &mut Harness) -> [Vec<Rect>; 2] {
    let area = area(window);
    tinted(window).map(|tinted| tinted.into_iter().filter(|rect| rect.width >= area.width).collect())
}

/// The shadows that fall where the version shows: what casts each, and the part of the window it is kept
/// to, if it is kept to one.
fn shadows(window: &mut Harness) -> Vec<(Rect, Option<Rect>)> {
    let area = area(window);
    let (mut clips, mut found) = (Vec::new(), Vec::new());
    for item in window.paint().items {
        match item {
            Item::PushClip { rect, .. } => clips.push(rect),
            Item::PopClip => _ = clips.pop(),
            Item::Shadow { rect, .. } if rect.y < area.y + area.height && rect.y + rect.height > area.y => {
                found.push((rect, clips.last().copied()));
            }
            _ => {}
        }
    }
    found
}

/// The part of the window the version shows in.
fn area(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page").bounds
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

/// What is drawn where the version shows: its letters, and what marks them.
fn drawn(window: &mut Harness) -> Vec<Item> {
    let area = area(window);
    let items = window.paint().items.into_iter().filter(|item| match item {
        Item::Glyphs { origin, .. } => area.contains(*origin),
        Item::Fill { rect, .. } => area.contains(Point { x: rect.x + rect.width / 2.0, y: rect.y + rect.height / 2.0 }),
        _ => false,
    });
    items.collect()
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

/// Chooses a command from the More menu of the bar.
fn more(window: &mut Harness, command: &str) {
    window.click(Role::Button, "More").click(Role::MenuItem, command);
}

/// What the name field of the card that saves a copy holds, while the card shows.
fn copy_name(window: &mut Harness) -> Option<String> {
    window.find(Role::TextInput, "Name of the copy").and_then(|field| field.value)
}

/// How far down the window each line of text stands that shows in `within`, from the top one.
fn lines(window: &mut Harness, within: Rect) -> Vec<f32> {
    let mut down: Vec<f32> = Vec::new();
    for item in window.paint().items {
        let Item::Glyphs { origin, .. } = item else { continue };
        // A point on the line's letters, which stand on where the line is drawn from.
        let on = origin.y - 4.0;
        if within.contains(origin) && !down.contains(&on) {
            down.push(on);
        }
    }
    down.sort_by(f32::total_cmp);
    down
}

/// The left end and the right end of the part of the window that is `within`, at a height.
fn ends(within: Rect, y: f32) -> [Point; 2] {
    [1.0, within.width - 1.0].map(|across| Point { x: within.x + across, y })
}

/// What was last copied, as another application reads it.
fn copied(window: &Harness) -> Option<&str> {
    window.platform.clipboard.as_ref().map(|clip| clip.text.as_str())
}

/// Whether the tool shows that copies a passage, and whether it says that it has.
fn tool(window: &mut Harness) -> Option<bool> {
    let copied = window.find(Role::Button, "Copied").map(|_| true);
    copied.or_else(|| window.find(Role::Button, "Copy").map(|_| false))
}

/// A document with its history showing.
type Open = fn() -> Harness;
/// A way of picking the version that is so many back from the newest.
type Pick = fn(&mut Harness, usize);

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
fn a_version_that_is_stepped_to_shows_as_it_does_laid_out_whole() {
    // A step lays out only what differs from the version that showed. Another width lays all of it out.
    let cases: [(&str, Open); 2] = [("the letter", history), ("the plan", plan)];
    for (name, open) in cases {
        for compared in ["Previous", "Current", "Off"] {
            let mut window = open();
            window.click(Role::ListBoxOption, "Current version").click(Role::RadioButton, compared);
            let count = versions(&mut window).len();
            // Back to the oldest and on to the newest again, so each is come to from both sides.
            let steps = std::iter::repeat_n("Left", count - 1).chain(std::iter::repeat_n("Right", count - 1));
            for (step, key) in steps.enumerate() {
                window.key(key);
                let stepped = drawn(&mut window);
                window.resize(560.0, 800.0);
                window.paint();
                window.resize(1280.0, 800.0);
                let whole = drawn(&mut window);
                assert!(!whole.is_empty(), "{name}, {compared}, step {step}: the version shows");
                assert!(
                    stepped == whole,
                    "{name}, {compared}, step {step}: the version shows as it does laid out whole"
                );
            }
        }
    }
}

#[test]
fn code_is_coloured_as_the_version_that_shows_reads() {
    // The second block was taken out after a line was added to the first.
    let mut window = Harness::doc("```rust\nlet first = 1;\n```\n\n```rust\nlet second = 2;\n```\n");
    window.platform.wall = Some(SUNDAY + HOUR);
    window.select("first").key("End").key("Enter").type_text("fn main() {}");
    window.platform.wall = Some(SUNDAY + 3 * HOUR);
    window.select("let second = 2;");
    while window.editor().doc().blocks.len() > 1 {
        window.key("Backspace");
    }
    window.platform.wall = Some(SUNDAY + 4 * HOUR);
    window.click(Role::Button, "History").click(Role::RadioButton, "Off");
    let keyword = tokens::SYNTAX_KEYWORD.light;
    let keywords = |window: &mut Harness| {
        let inked = |item: &Item| match item {
            Item::Glyphs { color, .. } => (color.r, color.g, color.b) == (keyword.r, keyword.g, keyword.b),
            _ => false,
        };
        drawn(window).iter().filter(|item| inked(item)).count()
    };
    assert_eq!(keywords(&mut window), 2, "both keywords of the document as it is");
    for (key, expected) in [("Left", 3), ("Left", 2), ("Right", 3), ("Right", 2)] {
        window.key(key);
        assert_eq!(keywords(&mut window), expected, "{key}: the keywords that version has");
    }
}

#[test]
fn a_version_shows_what_it_changed_from_the_version_before_it() {
    let mut window = plan();
    let document = window.markdown();
    // What is said of each block that differs, and some of what the block reads.
    let expected = [
        ("Changed by Alice Moreau", "This plan keeps them in order."),
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
    // The paragraph that was rewritten shows as it read before as well.
    let before = before(&mut window).expect("the paragraph as it read before");
    assert_eq!(before.name, "Before");
    assert_eq!(before.value.as_deref(), Some(PLAN[1]));
    let [gone, came] = marked(&mut window);
    assert!(gone >= 2 && came >= 2, "words of both versions are marked: {gone} and {came}");
    // The amber and blue of a conflict's two sides mark nothing: of these versions one came first.
    let conflict = [tokens::WARN_SOFT, tokens::INFO_SOFT].map(|ink| filled(&mut window, ink).len());
    assert_eq!(conflict, [0, 0], "nothing is in a conflict's colours");
    assert_eq!(window.markdown(), document, "the document itself is as it was");
}

#[test]
fn what_a_version_took_out_and_what_it_put_in_lie_in_bands_across_the_page() {
    let mut window = plan();
    window.resize(1280.0, 1400.0);
    let area = area(&mut window);
    let [gone, came] = bands(&mut window);
    // The bands of each colour down the page, and the last paragraph each holds: the rewritten paragraph
    // as its version has it, then the stretch that only that version has.
    for (name, bands, holds) in [("taken out", gone, [PLAN[1], PLAN[4]]), ("added", came, [HERS[0], HERS[5]])] {
        assert_eq!(bands.len(), holds.len(), "{name}: a band for the paragraph and one for the stretch: {bands:?}");
        for (band, holds) in bands.into_iter().zip(holds) {
            assert!(band.x <= area.x && band.x + band.width >= area.x + area.width, "{name}: from edge to edge");
            // What the band holds is its last line's paragraph.
            let line = lines(&mut window, band).pop().expect("text in the band");
            let at = ends(area, line)[0];
            window.advance(2_000).click_at(at).click_at(at).click_at(at).key("Mod+C");
            assert_eq!(copied(&window), Some(holds), "{name}");
        }
    }
    let [gone, came] = bands(&mut window);
    for (gone, came) in gone.into_iter().zip(came) {
        assert!(gone.y + gone.height <= came.y, "what was taken out first: {gone:?} and {came:?}");
    }
}

#[test]
fn a_rewritten_block_reads_as_it_did_right_over_how_it_reads_now_whatever_the_room() {
    for width in [1720.0, 1440.0, 900.0] {
        let mut window = plan();
        window.resize(width, 800.0);
        let (area, before) = (area(&mut window), before(&mut window).expect("the earlier version").bounds);
        let [gone, came] = bands(&mut window);
        let (gone, came) = (gone[0], came[0]);
        assert!(gone.y > area.y, "{width}: under the heading, which both versions have");
        assert_eq!(gone.y + gone.height, came.y, "{width}: the two stand together, the earlier one on top");
        assert!(gone.width >= area.width && came.width >= area.width, "{width}: neither stands beside the other");
        // The band of what was taken out is where the paragraph is told as it read before.
        assert!((before.y - gone.y).abs() < 1.0 && (before.height - gone.height).abs() < 1.0, "{width}: {before:?}");
    }
}

#[test]
fn what_was_added_lies_over_the_page_and_what_was_taken_out_under_it() {
    let mut window = plan();
    let [gone, came] = bands(&mut window);
    let (gone, came) = (gone[0], came[0]);
    let shadows = shadows(&mut window);
    // What casts a shadow stands over what the shadow falls on.
    let over = |caster: &Rect, band: Rect| caster.y + caster.height <= band.y;
    let under = |caster: &Rect, band: Rect| caster.y >= band.y + band.height;
    let is = |caster: &Rect, band: Rect| caster.y == band.y && caster.height == band.height;
    // The band of what was added casts one on the page round it.
    assert!(shadows.iter().any(|(caster, kept)| is(caster, came) && *kept != Some(came)), "added: {shadows:?}");
    // The page casts one into the band of what was taken out, from each side of it. That band casts none.
    let into: Vec<&Rect> = shadows.iter().filter(|(_, kept)| *kept == Some(gone)).map(|(caster, _)| caster).collect();
    assert!(into.iter().any(|caster| over(caster, gone)), "taken out, from the page over it: {shadows:?}");
    assert!(into.iter().any(|caster| under(caster, gone)), "taken out, from the page under it: {shadows:?}");
    let cast = shadows.iter().filter(|(caster, _)| !over(caster, gone) && !under(caster, gone)).count();
    assert_eq!(cast, 0, "taken out: {shadows:?}");
}

#[test]
fn a_version_compared_with_the_document_shows_what_has_changed_since() {
    let mut window = plan();
    window.click(Role::ListBoxOption, "11:30").click(Role::RadioButton, "Current");
    let found = differing(&mut window);
    let says = |said: &str, holds: &str| found.iter().any(|(told, reads)| told == said && reads.contains(holds));
    assert!(says("Changed later", "This plan keeps them in order."), "{found:?}");
    assert!(says("Removed later", PLAN[4]), "{found:?}");
    assert!(says("Added later", "Budget"), "{found:?}");
    // The version that is picked is the earlier of the two.
    let before = before(&mut window).expect("the paragraph as the picked version has it");
    assert_eq!(before.value.as_deref(), Some(PLAN[1]));

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
        assert_eq!(bands(&mut window), [vec![], vec![]], "{name}: nothing lies in a band");
        // A heading is written here with the signs that Markdown makes one from.
        let expected: Vec<&str> = reads.iter().map(|text| text.trim_start_matches(['#', ' '])).collect();
        if !expected.is_empty() {
            assert_eq!(shown(&mut window), expected, "{name}: only what the version itself has shows");
        }
    }
}

#[test]
fn a_long_history_lists_the_versions_near_what_shows_and_brings_the_one_picked_into_view() {
    let mut window = counted(60);
    let panel = window.find(Role::Complementary, "History").expect("the panel").bounds;
    let in_view = |row: &Found| row.bounds.y >= panel.y && row.bounds.y + row.bounds.height <= panel.y + panel.height;
    let row = |window: &mut Harness, name: &str| versions(window).into_iter().find(|version| version.name == name);
    assert_eq!(strip(&mut window).description.as_deref(), Some("61 of 61"));
    // The panel builds the rows near what shows of it, which is its top.
    let listed = versions(&mut window);
    assert!(listed.len() < 61 && listed[0].name == "Current version", "{} rows are built", listed.len());

    // The oldest version is picked on the strip: its row is built, and is in view.
    let oldest = Point { x: strip(&mut window).bounds.x + 1.0, y: strip(&mut window).center().y };
    window.click_at(oldest);
    let first = row(&mut window, "First version").expect("the row of the oldest version");
    assert!(first.selected == Some(true) && in_view(&first), "it is picked and in view: {:?}", first.bounds);
    assert_eq!(shown(&mut window), ["The count is 0."]);

    // The panel scrolls back to the newest under the wheel, and a press there picks.
    window.scroll_at(Point { x: panel.x + panel.width / 2.0, y: panel.y + panel.height / 2.0 }, -1e6);
    let newest = row(&mut window, "Current version").expect("the row of the newest version");
    assert!(in_view(&newest), "the newest version is in view again: {:?}", newest.bounds);
    window.click(Role::ListBoxOption, "23:00");
    assert_eq!(shown(&mut window), ["The count is 59."]);
}

#[test]
fn the_place_the_panel_keeps_for_a_row_is_as_tall_as_the_row() {
    // A version nobody put their name to, with a name of its own: its row says when it was written under
    // the name, and shows no face.
    let mut unsigned = Harness::doc("The roof comes first.");
    write(&mut unsigned, SUNDAY + HOUR, "roof", "north roof");
    let named = unsigned.ui().change(|editor| {
        let at = editor.model().revision();
        editor.name_version(&at, Some("Draft")).map(|()| Change::default())
    });
    assert!(named.is_ok());
    write(&mut unsigned, SUNDAY + 3 * HOUR, "first", "first of all");
    unsigned.platform.wall = Some(SUNDAY + 4 * HOUR);
    unsigned.click(Role::Button, "History");
    for (name, mut window) in [("the letter", history()), ("an unsigned document", unsigned)] {
        let controls = window.controls();
        for day in controls.iter().filter(|found| found.role == Role::ListBox) {
            let within = |row: &&Found| row.role == Role::ListBoxOption && day.bounds.contains(row.center());
            let rows: f32 = controls.iter().filter(within).map(|row| row.bounds.height).sum();
            assert_eq!(rows, day.bounds.height, "{name}, {}: its rows are as tall as their places", day.name);
        }
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

#[test]
fn a_copy_of_the_version_that_shows_is_asked_of_the_shell() {
    // A way of asking for the copy, and the format the card opens on.
    let ways: [(&str, Act, CopyFormat); 3] = [
        ("the key", |window| _ = window.key("Mod+Shift+S"), CopyFormat::Letters),
        ("the menu", |window| more(window, "Save a copy of this version Ctrl+Shift+S"), CopyFormat::Letters),
        ("Export", |window| more(window, "Export this version"), CopyFormat::Pdf),
    ];
    for (way, ask, opens_on) in ways {
        let mut window = history();
        window.click(Role::ListBoxOption, "11:30");
        ask(&mut window);
        window.assert_open(Role::Dialog, "Save a copy of this version");
        // A copy of a version starts with its text alone, so no cleaner copy is offered.
        assert!(window.find(Role::Button, "Clean copy").is_none(), "by {way}");
        // The copy's name says which version it is of: the one written on Tuesday 10 November.
        assert_eq!(copy_name(&mut window).as_deref(), Some("Cottage renovation, 10 Nov"), "by {way}");
        window.key("Enter").assert_closed(Role::Dialog, "Save a copy of this version");
        assert!(showing(&mut window), "by {way}: the history still shows");

        let asked = window.ui().take_requests();
        let [Request::SaveVersion { revision, name, format, .. }] = &asked[..] else { panic!("by {way}: {asked:?}") };
        assert_eq!((name.as_str(), *format), ("Cottage renovation, 10 Nov", opens_on), "by {way}");
        let (read, _) = window.ui().document_at(revision).expect("a version of the history");
        let paragraphs: Vec<String> = read.blocks.iter().skip(1).map(Block::plain_text).collect();
        assert_eq!(paragraphs, VERSIONS[2].1, "by {way}: the revision asked for is the version that shows");
    }
}

#[test]
fn a_copy_is_named_after_its_version_and_the_newest_is_copied_as_the_document_is() {
    let mut window = history();
    window.click(Role::ListBoxOption, "16:42").click(Role::Button, "Name");
    window.type_text("Sent to the roofer").key("Enter").key("Mod+Shift+S");
    assert_eq!(copy_name(&mut window).as_deref(), Some("Cottage renovation, Sent to the roofer"));
    // Escape puts the card away first, and only then leaves the history.
    window.key("Escape");
    assert_eq!((copy_name(&mut window), showing(&mut window)), (None, true));

    // The newest version is the document as it reads now, which is copied whole, its history too.
    window.click(Role::ListBoxOption, "Current version").key("Mod+Shift+S");
    assert_eq!(copy_name(&mut window).as_deref(), Some("Cottage renovation copy"));
    window.ui().take_requests();
    window.key("Enter");
    let asked = window.ui().take_requests();
    assert!(matches!(asked[..], [Request::SaveCopy { format: CopyFormat::Letters, .. }]), "{asked:?}");
}

#[test]
fn what_the_shell_answers_about_a_copy_of_a_version_is_said_in_the_history() {
    let mut window = history();
    window.click(Role::ListBoxOption, "11:30").key("Mod+Shift+S").key("Enter");
    window.ui().refuse_copy("There is already a document of that name in the folder.", false);
    window.assert_open(Role::Dialog, "Save a copy of this version");
    window.assert_open(Role::Alert, "There is already a document of that name in the folder.");
    window.ui().take_requests();
    window.type_text(" for Sam").key("Enter");
    let asked = window.ui().take_requests();
    let again = |asked: &Request| matches!(asked, Request::SaveVersion { name, .. } if name.ends_with("for Sam"));
    assert!(asked.len() == 1 && again(&asked[0]), "{asked:?}");

    window.ui().set_copied("House");
    assert!(window.find(Role::Tooltip, "Copy saved in House").is_some(), "the bar says where the copy went");
    assert!(showing(&mut window) && copy_name(&mut window).is_none());
}

#[test]
fn a_card_for_a_copy_of_a_version_goes_when_the_history_does() {
    let mut window = history();
    window.click(Role::ListBoxOption, "11:30").key("Mod+Shift+S");
    assert!(copy_name(&mut window).is_some());
    // A card of the shell's over the window takes the history away, and the version with it.
    window.ui().set_landing(Some(Landing::default()));
    window.ui().set_landing(None);
    assert_eq!((showing(&mut window), copy_name(&mut window)), (false, None));
}

#[test]
fn a_passage_of_a_version_is_selected_with_the_pointer_and_copied_out_of_it() {
    // How the passage is selected, given where the lines of the version's two paragraphs start and end,
    // and what it reads.
    type Select = fn(&mut Harness, [Point; 2], [Point; 2]);
    let ways: [(&str, Select, &str); 5] = [
        ("a drag along a line", |window, _, [from, to]| _ = window.press_at(from).release_at(to), VERSIONS[2].1[1]),
        ("a drag back along it", |window, _, [from, to]| _ = window.press_at(to).release_at(from), VERSIONS[2].1[1]),
        (
            "a drag down the page",
            |window, [from, _], [_, to]| _ = window.press_at(from).release_at(to),
            "The roof comes first. Costs are rising.\n\nThe kitchen floor follows.",
        ),
        (
            "a press, and another with Shift",
            |window, [from, _], [_, to]| _ = window.click_at(from).shift_click_at(to),
            "The roof comes first. Costs are rising.\n\nThe kitchen floor follows.",
        ),
        ("a second press", |window, _, [from, _]| _ = window.click_at(from).click_at(from), "The"),
    ];
    let copies: [(&str, Act); 2] =
        [("the tool", |window| _ = window.click(Role::Button, "Copy")), ("the key", |window| _ = window.key("Mod+C"))];
    for (way, select, reads) in ways {
        for (by, copy) in copies {
            let mut window = history();
            window.click(Role::ListBoxOption, "11:30");
            let area = area(&mut window);
            let down = lines(&mut window, area);
            assert_eq!(down.len(), 3, "the heading and two paragraphs show");
            // With nothing selected there is nothing to copy.
            window.click_at(ends(area, down[0])[0]).key("Mod+C");
            assert_eq!((tool(&mut window), copied(&window)), (None, None), "{way}: a press alone");

            // The marks that lie along the second paragraph's line: over the word it gained, and over
            // what is selected of it.
            let line = down[2];
            let marks = |window: &mut Harness| {
                let along = |item: &&Item| matches!(item, Item::Fill { rect, .. } if rect.height < 40.0 && rect.y <= line && line <= rect.y + rect.height);
                drawn(window).iter().filter(along).count()
            };
            let unmarked = marks(&mut window);
            select(&mut window, ends(area, down[1]), ends(area, down[2]));
            assert_eq!(tool(&mut window), Some(false), "{way}: a tool offers to copy what is selected");
            assert!(marks(&mut window) > unmarked, "{way}: what is selected is marked");
            copy(&mut window);
            assert_eq!(copied(&window), Some(reads), "{way}, {by}");
            assert_eq!(tool(&mut window), Some(true), "{way}, {by}: the tool says so");
            assert_eq!(written(&window), VERSIONS[4].1, "{way}: the document is as it was");

            // Another passage, selected a while later, has not been copied.
            window.advance(2_000);
            window.press_at(ends(area, down[0])[0]).release_at(ends(area, down[0])[1]);
            assert_eq!(tool(&mut window), Some(false), "{way}, {by}");
        }
    }
}

#[test]
fn a_third_press_selects_the_paragraph_and_a_key_the_whole_version() {
    let mut window = history();
    window.click(Role::ListBoxOption, "11:30");
    let area = area(&mut window);
    let at = ends(area, lines(&mut window, area)[2])[0];
    window.click_at(at).click_at(at).click_at(at).key("Mod+C");
    assert_eq!(copied(&window), Some(VERSIONS[2].1[1]));

    window.key("Mod+A");
    assert_eq!(tool(&mut window), Some(false), "another passage has not been copied");
    window.key("Mod+C");
    let whole = format!("# Cottage renovation\n\n{}\n\n{}", VERSIONS[2].1[0], VERSIONS[2].1[1]);
    assert_eq!(copied(&window), Some(whole.as_str()));
}

#[test]
fn each_version_of_a_rewritten_block_is_copied_as_that_version_reads() {
    // Whether the drag ends in the block it began in, or far down the page.
    for beyond in [false, true] {
        let mut window = plan();
        window.resize(1280.0, 1400.0);
        let area = area(&mut window);
        let [gone, came] = bands(&mut window);
        let (before, after) = (gone[0], came[0]);
        // The line in the band of the earlier version says what it is, over its text.
        let down = lines(&mut window, before);
        let (first, last) = (down[1], if beyond { area.y + area.height - 2.0 } else { down[down.len() - 1] });
        window.press_at(ends(before, first)[0]).release_at(ends(before, last)[1]);
        let tool = window.find(Role::Button, "Copy").expect("the tool that copies").bounds;
        assert!(tool.y > first, "the line that says which version it is stays to be read: {tool:?}");
        window.key("Mod+C");
        assert_eq!(copied(&window), Some(PLAN[1]), "the earlier version, and no more than its block shows of it");

        // The later version stands under it, where the page goes on.
        let down = lines(&mut window, after);
        let (first, last) = (down[1], down[down.len() - 1]);
        window.advance(2_000);
        window.press_at(ends(after, first)[0]).release_at(ends(after, last)[1]).key("Mod+C");
        assert_eq!(copied(&window), Some(HERS[0]), "the later version");
        // A while later, so that the press is not the second of two.
        window.advance(2_000);
        window.press_at(ends(after, first)[0]).release_at(ends(area, area.y + area.height - 2.0)[1]).key("Mod+C");
        let all = copied(&window).expect("a passage down to the end of the page");
        assert!(all.starts_with(HERS[0]) && all.ends_with(HERS[5]), "the page, from the later version on: {all}");
    }

    // Of two blocks rewritten at one sitting, a drag from the earlier version of the first stays in it.
    let mut window = Harness::doc("# Plan\n\nThe roof comes first.\n\nThe floor follows.");
    write(&mut window, SUNDAY + HOUR, "roof comes", "slates come");
    write(&mut window, SUNDAY + HOUR, "floor follows", "kitchen is last");
    window.platform.wall = Some(SUNDAY + 2 * HOUR);
    window.click(Role::Button, "History");
    let (area, before) = (area(&mut window), before(&mut window).expect("the earlier version").bounds);
    let first = lines(&mut window, before)[1];
    window.press_at(ends(before, first)[0]).release_at(ends(area, area.y + area.height - 2.0)[1]).key("Mod+C");
    assert_eq!(copied(&window), Some("The roof comes first."));
}

#[test]
fn a_passage_goes_with_its_version_and_escape_drops_it_before_it_leaves_the_history() {
    let mut window = history();
    window.click(Role::ListBoxOption, "11:30");
    let area = area(&mut window);
    let down = lines(&mut window, area);
    let select = |window: &mut Harness| _ = window.press_at(ends(area, down[2])[0]).release_at(ends(area, down[2])[1]);
    // The tool waits for the passage: it shows once the button is let go.
    window.press_at(ends(area, down[1])[0]).hover_at(ends(area, down[1])[1]);
    assert_eq!(tool(&mut window), None, "the passage is still being selected");
    window.release_at(ends(area, down[1])[1]);
    assert_eq!(tool(&mut window), Some(false));
    window.key("Right");
    assert_eq!(tool(&mut window), None, "another version shows");
    window.key("Mod+C");
    assert_eq!(copied(&window), None, "and nothing of it is selected");

    // A while later, so that the press is not the second of two.
    window.advance(2_000);
    select(&mut window);
    window.key("Escape");
    assert_eq!((tool(&mut window), showing(&mut window)), (None, true), "the passage goes first");
    window.key("Escape");
    assert!(!showing(&mut window), "then the history");
}

#[test]
fn a_passage_copied_from_a_version_is_pasted_as_it_was_written() {
    let mut window = Harness::doc("# Roof\n\nThe **north** roof comes first.\n\nThe floor follows.");
    write(&mut window, SUNDAY + HOUR, "The north roof comes first.", "Nothing is decided.");
    window.platform.wall = Some(SUNDAY + 2 * HOUR);
    window.click(Role::Button, "History").key("Left");
    let area = area(&mut window);
    let at = ends(area, lines(&mut window, area)[1])[0];
    window.click_at(at).click_at(at).click_at(at).key("Mod+C").key("Escape").key("Escape");

    window.select("follows.").key("Right").key("Enter").key("Mod+V");
    let pasted = "The floor follows.\n\nThe **north** roof comes first.";
    assert!(window.markdown().contains(pasted), "the passage came with its marks: {}", window.markdown());
}

#[test]
fn a_drag_past_the_edge_of_the_page_area_scrolls_the_version_after_it() {
    let mut window = survey();
    let area = area(&mut window);
    let top = lines(&mut window, area)[1];
    let under = Point { x: area.x + area.width / 2.0, y: area.y + area.height + 200.0 };
    window.press_at(ends(area, top)[0]);
    for _ in 0..4 {
        window.hover_at(under);
    }
    window.release_at(under);
    assert!(lines(&mut window, area)[0] < top - 100.0, "the survey moved up under the drag");
}

#[test]
fn the_history_of_a_selection_lists_the_versions_that_changed_its_paragraph_with_what_each_did() {
    // What is selected, then the versions that changed its paragraph, newest first: where each stands among
    // all the versions, and what it is said to have done.
    let cases: [(&str, [(usize, &str); 3]); 2] = [
        ("floor", [(4, "Added 2 words"), (2, "Added 1 word"), (0, "Wrote the passage")]),
        ("roof", [(3, "Added 1 word"), (1, "Rewrote the passage"), (0, "Wrote the passage")]),
    ];
    for (selected, expected) in cases {
        let mut window = passage(selected);
        assert!(narrowed(&mut window), "{selected}");
        let listed = versions(&mut window);
        assert_eq!(listed.len(), expected.len(), "{selected}: {listed:?}");
        for (version, (at, did)) in listed.iter().zip(expected) {
            assert!(version.name.ends_with(VERSIONS[at].0), "{selected}: {}", version.name);
            assert!(version.description.as_deref().is_some_and(|said| said.ends_with(did)), "{selected}: {version:?}");
        }
        assert_eq!(listed[0].selected, Some(true), "{selected}: the newest of them shows");
        assert!(window.find(Role::Label, "3 of 5 versions").is_some(), "{selected}");
        assert_eq!(standing_out(&mut window), 3, "{selected}: the strip shows which they are");
    }
}

#[test]
fn the_history_of_a_selection_steps_through_the_versions_that_changed_it() {
    // The keys pressed from the newest of them, and the version that shows after them.
    let cases: [(&[&str], usize); 4] =
        [(&["Left"], 2), (&["Left", "Left"], 0), (&["Left"; 5], 0), (&["Left", "Left", "Right"], 2)];
    for (keys, at) in cases {
        let mut window = passage("floor");
        for key in keys {
            window.key(key);
        }
        assert!(picked(&mut window).is_some_and(|name| name.ends_with(VERSIONS[at].0)), "{keys:?}");
        assert_eq!(shown(&mut window), VERSIONS[at].1, "{keys:?}: the page reads as that version left it");
    }
    // A press on the strip picks the one that left the passage as it read then: the document as it reads
    // now did not change this one.
    let mut window = passage("roof");
    window.key("Left");
    let now = cell(&mut window, 4.5);
    window.click_at(now);
    assert!(picked(&mut window).is_some_and(|name| name.ends_with(VERSIONS[3].0)));
}

#[test]
fn all_but_the_passage_lies_under_a_veil() {
    let mut window = passage("floor");
    let blocks = window.controls().into_iter().filter(|found| matches!(found.role, Role::Paragraph | Role::Heading));
    let of: Vec<bool> = blocks.filter(|found| found.name.is_empty()).map(|found| found.description.is_some()).collect();
    assert_eq!(of, [false, false, true], "the paragraph that was selected is told as the selection");

    // The page's lines: its heading, its first paragraph, then the line that says what the passage is,
    // and the passage.
    let area = area(&mut window);
    let (lines, veils) = (lines(&mut window, area), veils(&mut window));
    let veiled: Vec<bool> =
        lines.iter().map(|line| veils.iter().any(|veil| veil.y <= *line && *line < veil.y + veil.height)).collect();
    assert_eq!(veiled, [true, true, false, false], "{lines:?} under {veils:?}");
    // Every version shows whole again, the first too, which shows as it read either way.
    window.key("Left").key("Left").click(Role::Button, "All versions");
    assert_eq!(self::veils(&mut window), [], "nothing is veiled once every version shows");
}

#[test]
fn a_version_that_rewrote_the_passage_shows_it_as_it_read_over_how_it_read_after() {
    let mut window = passage("roof");
    let quoted = |window: &mut Harness, side: &str| {
        let is = |found: &Found| found.role == Role::Paragraph && found.name.starts_with(side);
        window.controls().into_iter().find(is).unwrap_or_else(|| panic!("the passage as it read {side}"))
    };
    let (before, after) = (quoted(&mut window, "Before: "), quoted(&mut window, "After: "));
    assert!(before.name.ends_with(VERSIONS[0].1[0]) && after.name.ends_with(VERSIONS[1].1[0]));
    assert!(before.bounds.y + before.bounds.height <= after.bounds.y, "the earlier one is on top");
    // Each is in a band of the colour the page marks what was taken out and what was put in with.
    let [red, green] = tinted(&mut window);
    let within = |bands: &[Rect], quote: &Found| {
        bands.iter().any(|band| band.width >= quote.bounds.width && band.contains(quote.center()))
    };
    assert!(within(&red, &before) && !within(&green, &before), "what it read before is in red");
    assert!(within(&green, &after) && !within(&red, &after), "what it read after is in green");

    // The page has the passage as that version left it, and only that: the panel has the rest.
    window.key("Left");
    assert_eq!(shown(&mut window), VERSIONS[1].1);
    assert_eq!(bands(&mut window), [vec![], vec![]], "no band of the version before stands in the page");
}

#[test]
fn a_passage_far_down_the_page_is_brought_into_view() {
    let mut window = survey();
    window.click(Role::Button, "Back to editing");
    window.select("The last line").key("Mod+Shift+P").type_text("History of selection").key("Enter");
    assert!(narrowed(&mut window));
    // The veil over what comes before the passage ends where the passage begins: in the page area, with
    // some of the page over it and under it.
    let area = area(&mut window);
    let begins = |window: &mut Harness| {
        let veils = veils(window);
        let over = veils.iter().find(|veil| veil.y <= area.y + 1.0).expect("a veil from the top of the page area");
        over.y + over.height - area.y
    };
    let passage = begins(&mut window);
    assert!(passage > ROOM && passage < area.height - ROOM, "{passage} down a page area of {}", area.height);

    // It is brought there once: the page then goes where it is scrolled to.
    let middle = Point { x: area.x + area.width / 2.0, y: area.y + area.height / 2.0 };
    window.scroll_at(middle, -ROOM);
    let moved = begins(&mut window);
    assert!(moved > passage + ROOM / 2.0, "the passage went from {passage} to {moved} down the page area");
}

#[test]
fn restoring_a_passage_leaves_the_rest_of_the_document_as_it_reads() {
    let mut window = passage("floor");
    let before = window.markdown();
    // The passage as it reads now is not restored: its button takes no press.
    window.click(Role::Button, "Restore this passage");
    assert!(narrowed(&mut window));
    assert_eq!(window.markdown(), before);

    window.key("Left").click(Role::Button, "Restore this passage");
    assert!(!narrowed(&mut window) && !showing(&mut window), "the page is back, to be written on");
    // The other paragraph keeps what was written on it since.
    assert_eq!(written(&window), [VERSIONS[4].1[0], VERSIONS[2].1[1]]);
    window.key("Mod+z");
    assert_eq!(window.markdown(), before, "one step back is the document as it was");
}

#[test]
fn every_version_shows_again_with_the_same_one_picked() {
    let mut window = passage("floor");
    window.key("Left").click(Role::Button, "All versions");
    assert!(showing(&mut window) && !narrowed(&mut window));
    assert_eq!(versions(&mut window).len(), VERSIONS.len());
    assert_eq!(picked(&mut window).as_deref(), Some(VERSIONS[2].0));
    assert_eq!(marked(&mut window), [0, 1], "and shows what it changed, as every version does");
}

#[test]
fn the_passage_is_every_paragraph_the_selection_reaches_into() {
    // How far the selection is taken on from the end of the first paragraph, and how many versions
    // changed what it then touches: the very start of the next paragraph is not a part of that one.
    let cases: [(&[&str], usize); 3] = [(&[], 3), (&["Shift+Right"], 3), (&["Shift+Right", "Shift+Right"], 5)];
    for (keys, changed) in cases {
        let mut window = letter();
        window.select("rising.");
        for key in keys {
            window.key(key);
        }
        window.key("Mod+Shift+P").type_text("History of selection").key("Enter");
        assert_eq!(versions(&mut window).len(), changed, "{keys:?}");
    }
    // With nothing selected there is no passage to ask about.
    let mut window = letter();
    window.select("rising.").key("Right").key("Mod+Shift+P").type_text("History of selection").key("Enter");
    assert!(!narrowed(&mut window) && !showing(&mut window));
}

#[test]
fn edits_that_come_in_under_the_history_of_a_selection_are_listed_when_they_change_it() {
    // What another writer writes over, with what, and whether that is in the passage.
    for (old, new, of) in [("May", "June", true), ("north roof", "slate roof", false)] {
        let mut window = passage("floor");
        window.key("Left");
        merge(&mut window, "Alice Moreau", SUNDAY + 9 * HOUR + 40 * MINUTE, old, new);
        assert!(narrowed(&mut window), "{old}");
        assert_eq!(versions(&mut window).len(), 3 + usize::from(of), "{old}");
        assert!(picked(&mut window).is_some_and(|name| name.ends_with(VERSIONS[2].0)), "{old}");
        assert_eq!(shown(&mut window), VERSIONS[2].1, "{old}: the version that was being read still shows");
    }
}

// Since the last visit

/// When each of the others wrote on the letter that Sam comes back to.
const TUESDAY: u64 = SUNDAY - 5 * DAY + 16 * HOUR;

/// What they wrote over, one after the other, and with what: a sentence of each paragraph, then words.
const SINCE: [(&str, &str); 4] = [
    ("The floor follows.", "A slate floor is laid in the kitchen next spring."),
    ("The roof comes first.", "Scaffolding goes up on the north side in April."),
    ("Costs", "Prices"),
    ("rising", "rising fast"),
];

/// The letter Sam last saw on Monday, after Dana had written on it, as it is opened on Sunday morning: since
/// then each of `others` has written on it in turn, on a device of their own.
fn revisited(others: &[&str]) -> Harness {
    let mut sam = Harness::doc(&format!("# Cottage renovation\n\n{}\n\n{}", VERSIONS[0].1[0], VERSIONS[0].1[1]));
    sam.ui().set_author("Sam Okafor");
    write(&mut sam, SUNDAY - 6 * DAY + 11 * HOUR, "unknown", "rising");
    // A history knows one name for each device, so each writer has one.
    let wrote = |sam: &mut Harness, device: u64, writer: &str, at: u64, (old, new): (&str, &str)| {
        let copy = sam.editor().model().fork(ReplicaId::from(device));
        let mut theirs = Harness::with(Window::new(Editor::with_model(copy)));
        theirs.ui().set_author(writer);
        write(&mut theirs, at, old, new);
        let history = theirs.editor().model().save();
        sam.ui().change(|editor| editor.merge(&history)).expect("a history of the same document");
    };
    // What another wrote before Sam last looked is no news.
    wrote(&mut sam, 9, "Dana Reyes", SUNDAY - 6 * DAY + 12 * HOUR, ("renovation", "repairs"));
    let seen = sam.editor().model().revision();
    for (at, writer) in others.iter().enumerate() {
        let device = others.iter().position(|other| other == writer).unwrap_or(at) as u64;
        wrote(&mut sam, 2 + device, writer, TUESDAY + at as u64 * HOUR, SINCE[at]);
    }
    sam.platform.wall = Some(SUNDAY + 9 * HOUR + 20 * MINUTE);
    sam.ui().set_last_seen(Some(seen));
    sam
}

/// What the toast about the last visit says, while it shows.
fn news(window: &mut Harness) -> Option<String> {
    let toast = window.controls().into_iter().find(|found| found.role == Role::Status);
    toast.map(|toast| toast.name).filter(|said| said.starts_with("Edited by"))
}

/// What the picked version is compared with, as the bar says.
fn compared(window: &mut Harness) -> Option<String> {
    let segments = window.controls().into_iter().filter(|found| found.role == Role::RadioButton);
    segments.filter(|segment| segment.toggled == Some(true)).map(|segment| segment.name).next()
}

#[test]
fn opening_a_document_that_others_wrote_on_since_the_last_visit_says_who() {
    // Who wrote since, and what the toast says.
    let cases: [(&[&str], Option<&str>); 5] = [
        (&[], None),
        (&["Sam Okafor"], None),
        (&["Alice Moreau"], Some("Edited by Alice Moreau")),
        (&["Alice Moreau", "Sam Okafor", "Ben Adler"], Some("Edited by 2 people")),
        (&["Alice Moreau", "Ben Adler", "Chidi Eze", "Alice Moreau"], Some("Edited by 3 people")),
    ];
    for (others, said) in cases {
        let mut window = revisited(others);
        assert_eq!(news(&mut window).as_deref(), said, "{others:?}");
    }
}

#[test]
fn the_toast_shows_the_document_against_what_was_last_seen() {
    let mut window = revisited(&["Alice Moreau", "Ben Adler"]);
    window.click(Role::Button, "See changes");
    assert!(showing(&mut window) && news(&mut window).is_none());
    assert_eq!(picked(&mut window).as_deref(), Some("Current version"));
    assert_eq!(compared(&mut window).as_deref(), Some("Last visit"));
    // Both paragraphs were rewritten since, each by another writer: neither is told as one writer's.
    let found = differing(&mut window);
    let says = |holds: &str| {
        found.iter().any(|(told, reads)| told == "Changed since your last visit" && reads.contains(holds))
    };
    assert!(says("slate floor") && says("Scaffolding"), "{found:?}");
    // A paragraph shows as Sam last saw it too, with what Sam had written by then.
    let before = before(&mut window).and_then(|before| before.value).expect("a paragraph as it was last seen");
    assert_eq!(before, "The roof comes first. Costs are rising.");

    // What comes in while it shows is part of the document, which stays the version that is compared.
    merge(&mut window, "Chidi Eze", SUNDAY + 9 * HOUR + 40 * MINUTE, "Costs", "Prices");
    assert_eq!(picked(&mut window).as_deref(), Some("Current version"));
    assert_eq!(compared(&mut window).as_deref(), Some("Last visit"));
    assert!(differing(&mut window).iter().any(|(_, reads)| reads.contains("Prices")));

    // Back on the page the toast has had its answer.
    window.key("Escape");
    assert!(!showing(&mut window) && news(&mut window).is_none());
}

#[test]
fn the_toast_goes_when_it_is_put_away_or_the_person_writes() {
    let ways: [(&str, Act); 3] = [
        ("put away", |window| _ = window.click(Role::Button, "Dismiss")),
        ("written", |window| _ = window.select("Costs").type_text("Prices")),
        ("another writer's edit", |window| merge(window, "Ben Adler", SUNDAY + 10 * HOUR, "comes", "goes")),
    ];
    for (way, act) in ways {
        let mut window = revisited(&["Alice Moreau"]);
        act(&mut window);
        assert_eq!(news(&mut window).is_some(), way == "another writer's edit", "{way}");
        assert!(!showing(&mut window), "{way}");
    }
}

#[test]
fn the_toast_waits_while_another_view_has_the_page() {
    for view in ["the history", "Print view"] {
        let mut window = revisited(&["Alice Moreau"]);
        match view {
            "Print view" => window.key("Mod+P"),
            _ => window.click(Role::Button, "History"),
        };
        assert_eq!(news(&mut window), None, "{view}");
        window.key("Escape");
        assert!(news(&mut window).is_some(), "{view}: it has had no answer yet");
    }
}

#[test]
fn what_was_written_apart_before_the_visit_ended_is_news_once_it_comes_in() {
    let mut sam = revisited(&[]);
    // Alice takes a copy and writes on it offline. Sam writes on meanwhile, and last sees that.
    let copy = sam.editor().model().fork(ReplicaId::from(2));
    let mut alice = Harness::with(Window::new(Editor::with_model(copy)));
    alice.ui().set_author("Alice Moreau");
    write(&mut alice, TUESDAY, SINCE[0].0, SINCE[0].1);
    write(&mut sam, TUESDAY + HOUR, "roof", "north roof");
    write(&mut sam, TUESDAY + 2 * HOUR, "first", "first of all");
    let seen = sam.editor().model().revision();
    let hers = alice.editor().model().save();
    sam.ui().change(|editor| editor.merge(&hers)).expect("a history of the same document");
    sam.platform.wall = Some(SUNDAY + 9 * HOUR + 20 * MINUTE);
    sam.ui().set_last_seen(Some(seen));
    assert_eq!(news(&mut sam).as_deref(), Some("Edited by Alice Moreau"));

    // Only what Sam had not seen is told as changed, though Sam wrote after Alice did.
    sam.click(Role::Button, "See changes");
    let found = differing(&mut sam);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].0 == "Changed since your last visit" && found[0].1.contains("slate floor"), "{found:?}");
}

#[test]
fn what_the_person_writes_in_a_visit_is_compared_with_the_last_visit_too() {
    let mut window = revisited(&[]);
    write(&mut window, SUNDAY + 10 * HOUR, SINCE[0].0, SINCE[0].1);
    assert_eq!(news(&mut window), None, "it is no news to who wrote it");
    window.key("Mod+Shift+P").type_text("Changes since last visit").key("Enter");
    assert_eq!(compared(&mut window).as_deref(), Some("Last visit"));
    let found = differing(&mut window);
    let news =
        |(told, reads): &(String, String)| told == "Changed since your last visit" && reads.contains("slate floor");
    assert!(found.iter().any(news), "{found:?}");
}

#[test]
fn the_history_compares_with_the_last_visit_only_the_document_as_it_reads_now() {
    let mut window = revisited(&["Alice Moreau"]);
    // Offered from the palette too, and beside the other comparisons whenever the history shows.
    window.key("Mod+Shift+P").type_text("Changes since last visit").key("Enter");
    assert_eq!(compared(&mut window).as_deref(), Some("Last visit"));
    window.key("Escape").click(Role::Button, "History");
    assert_eq!(compared(&mut window).as_deref(), Some("Previous"));

    // An earlier version shows what it changed itself, and the last visit brings the newest back.
    window.key("Left").click(Role::RadioButton, "Last visit");
    assert_eq!(picked(&mut window).as_deref(), Some("Current version"));
    assert!(differing(&mut window).iter().any(|(told, _)| told == "Changed since your last visit"));
    window.key("Left");
    assert_eq!(compared(&mut window).as_deref(), Some("Previous"));
    assert!(differing(&mut window).iter().all(|(told, _)| told != "Changed since your last visit"));
}

#[test]
fn a_last_visit_the_shell_did_not_say_or_the_history_lacks_is_offered_nowhere() {
    // What the shell says it last saw.
    let stranger = Harness::doc("Another document").editor().model().revision();
    for (told, seen) in [("nothing", None), ("another document's", Some(stranger)), ("the document as it reads", None)]
    {
        let mut window = revisited(&["Alice Moreau"]);
        let now = window.editor().model().revision();
        window.ui().set_last_seen(if told == "the document as it reads" { Some(now) } else { seen });
        assert_eq!(news(&mut window), None, "{told}");
        window.key("Mod+Shift+P").type_text("Changes since last visit").key("Enter");
        assert!(!showing(&mut window), "{told}: the palette");
        window.key("Escape").click(Role::Button, "History");
        assert!(window.find(Role::RadioButton, "Last visit").is_none(), "{told}: the history");
    }
}

#[test]
fn only_one_who_writes_is_told_of_the_last_visit() {
    for access in [letters_ui::Access::Comments, letters_ui::Access::Reads] {
        let mut window = revisited(&["Alice Moreau"]);
        window.ui().set_access(access);
        assert_eq!(news(&mut window), None, "{access:?}");
    }
}

#[test]
fn someone_the_shell_keeps_the_history_from_is_offered_none_of_it() {
    // Whether the palette, asked for a command by its name, lists it.
    let listed = |window: &mut Harness, command: &str| {
        window.key("Mod+Shift+P").type_text(command);
        let listed = window.controls().iter().any(|found| found.role == Role::ListBoxOption && found.name == command);
        window.key("Escape");
        listed
    };
    let mut window = revisited(&["Alice Moreau"]);
    window.select("Cottage");
    for command in ["History", "History of selection", "Changes since last visit"] {
        assert!(listed(&mut window, command), "{command}: offered until the shell says");
    }

    window.ui().set_history_shown(false);
    assert_eq!(news(&mut window), None, "no word of the last visit");
    assert!(window.find(Role::Button, "History").is_none(), "the bar");
    for command in ["History", "History of selection", "Changes since last visit"] {
        assert!(!listed(&mut window, command), "{command}");
    }
    // The document is still theirs to write.
    let before = window.markdown();
    window.type_text("x");
    assert_ne!(window.markdown(), before);

    window.ui().set_history_shown(true);
    window.click(Role::Button, "History");
    assert!(showing(&mut window), "it is back when the shell says so");
}

#[test]
fn a_history_that_is_out_goes_when_the_shell_keeps_it_back() {
    for (whole, what) in [(true, "the history"), (false, "a passage's")] {
        let mut window = if whole { history() } else { passage("rising") };
        assert!(showing(&mut window) || narrowed(&mut window), "{what}");
        window.ui().set_history_shown(false);
        assert!(!showing(&mut window) && !narrowed(&mut window), "{what}");
        assert!(window.focused().is_some_and(|found| found.role == Role::Document), "{what}: the page is back");
    }
}

#[test]
fn a_version_is_named_in_a_window_too_narrow_for_the_whole_bar() {
    let mut window = letter();
    window.resize(700.0, 800.0);
    // The bar is folded, and the field that comes out where its button stood has the keyboard all the same.
    window.click(Role::Button, "History").click(Role::ListBoxOption, "16:42").click(Role::Button, "Name");
    window.type_text("Sent to the roofer").key("Enter");
    assert_eq!(picked(&mut window).as_deref(), Some("Sent to the roofer"));
    // A name it has is there to be typed over.
    window.click(Role::Button, "Name").type_text("Final").key("Enter");
    assert_eq!(picked(&mut window).as_deref(), Some("Final"));
}

#[test]
fn a_name_too_long_for_the_panel_is_cut_short_in_it() {
    const LONG: &str = "Alexandria Featherstonehaugh-Cholmondeley of the Harland Roofing and Guttering Company";
    let mut window = Harness::doc("# Cottage renovation\n\nThe roof comes first.");
    window.ui().set_author(LONG);
    write(&mut window, SUNDAY - 5 * DAY + 11 * HOUR, "roof", "north roof");
    write(&mut window, SUNDAY + 9 * HOUR, "first", "first of all");
    window.platform.wall = Some(SUNDAY + 9 * HOUR + 20 * MINUTE);
    // How far right the letters written in a panel reach, and where the panel ends.
    let reach = |window: &mut Harness, panel: &str| {
        let panel = window.find(Role::Complementary, panel).expect("the panel").bounds;
        let ends = window.paint().items.into_iter().filter_map(|item| match item {
            Item::Glyphs { origin, glyphs, .. } if panel.contains(origin) => {
                glyphs.last().map(|last| origin.x + last.x)
            }
            _ => None,
        });
        (ends.fold(0.0, f32::max), panel.x + panel.width)
    };

    // A writer's name, and a version's.
    window.click(Role::Button, "History");
    let earlier = versions(&mut window)[1].name.clone();
    window.click(Role::ListBoxOption, &earlier).click(Role::Button, "Name");
    window.type_text(LONG).key("Enter");
    assert_eq!(picked(&mut window).as_deref(), Some(LONG));
    let (ends, edge) = reach(&mut window, "History");
    assert!(ends < edge, "the letters end at {ends}, the panel at {edge}");

    window.key("Escape").select("north").key("Mod+Shift+P").type_text("History of selection").key("Enter");
    let (ends, edge) = reach(&mut window, "History of selection");
    assert!(ends < edge, "a passage's: the letters end at {ends}, the panel at {edge}");
}
