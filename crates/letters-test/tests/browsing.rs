//! Opening a document a server keeps: the card that lists a server's places and what they hold, and what
//! the window asks of the shell.

use letters_api::{Entry, EntryKind, Role as Grade, Space, SpaceKind, Tag, Trashed};
use letters_render::{Item, Point};
use letters_test::{Found, Harness, Role};
use letters_ui::{Browsed, Landing, Place, Request, Server};

const HOME: &str = "https://letters.example.org";
const WORK: &str = "https://docs.example.net";
/// Noon on 15 November 2026, in seconds since 1970.
const NOON: u64 = 1_794_744_000;
const DAY: u64 = 86_400;
const ASKING: &str = "Loading documents from letters.example.org.";
const REFUSAL: &str = "letters.example.org is out of reach.";

fn server(address: &str) -> Server {
    Server { address: address.into(), account: "Sam Okafor".into() }
}

/// A window whose clock says noon, signed in to these servers.
fn signed_in(servers: &[&str]) -> Harness {
    let mut window = Harness::doc("# Roof plan\n\nThe roofer needs the scaffold up by 3 March.");
    window.platform.wall = Some(NOON);
    window.ui().set_servers(servers.iter().map(|address| server(address)).collect());
    window
}

fn space(id: &str, name: &str, kind: SpaceKind) -> Space {
    Space { id: id.into(), name: name.into(), kind, role: Grade::Editor }
}

/// The person's own space and one a group shares.
fn spaces() -> Vec<Space> {
    vec![space("family", "Family", SpaceKind::Shared), space("sam", "Sam Okafor", SpaceKind::Personal)]
}

fn entry(kind: EntryKind, space: &str, path: &str, id: Option<&str>, ago: u64) -> Entry {
    Entry {
        kind,
        name: path.rsplit('/').next().unwrap_or(path).trim_end_matches(".let").into(),
        space: Some(space.into()),
        path: path.into(),
        id: id.map(str::to_owned),
        created: None,
        modified: NOON - ago,
        role: Grade::Editor,
        contributors: Vec::new(),
        tags: Vec::new(),
    }
}

fn folder(space: &str, path: &str) -> Entry {
    entry(EntryKind::Folder, space, path, None, 0)
}

fn document(space: &str, path: &str, id: &str, ago: u64) -> Entry {
    entry(EntryKind::Document, space, path, Some(id), ago)
}

fn within(space: &str, path: &str) -> Place {
    Place::Folder { space: space.into(), path: path.into() }
}

fn browse(server: &str, place: Place) -> Request {
    Request::Browse { server: server.into(), place }
}

fn listing(server: &str, place: Place, entries: Vec<Entry>) -> Browsed {
    Browsed { server: server.into(), place, spaces: Vec::new(), entries }
}

/// Opens the card and answers what it first asks, as a shell does: the spaces, then the person's own.
fn open(window: &mut Harness, held: Vec<Entry>) {
    window.click(Role::Button, "More").click(Role::MenuItem, "Open").assert_open(Role::Dialog, "Open");
    window.ui().set_listing(Browsed {
        server: HOME.into(),
        place: Place::Spaces,
        spaces: spaces(),
        entries: Vec::new(),
    });
    window.ui().set_listing(listing(HOME, within("sam", ""), held));
    window.ui().take_requests();
}

/// What the person's own space holds: two folders, then two documents.
fn own() -> Vec<Entry> {
    vec![
        folder("sam", "House"),
        folder("sam", "Letters sent"),
        document("sam", "Roof plan.let", "d1", 2 * 3_600),
        document("sam", "Tax 2026.let", "d2", 9 * DAY),
    ]
}

/// The names one of the card's lists holds, in order, with those that are picked.
fn listed(window: &mut Harness, list: &str) -> (Vec<String>, Vec<String>) {
    let bounds = window.find(Role::ListBox, list).map(|found| found.bounds);
    let inside = |found: &letters_test::Found| {
        let at = found.center();
        found.role == Role::ListBoxOption && bounds.is_some_and(|bounds| bounds.contains(at))
    };
    let rows: Vec<letters_test::Found> = window.controls().into_iter().filter(inside).collect();
    let picked = rows.iter().filter(|row| row.selected == Some(true)).map(|row| row.name.clone()).collect();
    (rows.into_iter().map(|row| row.name).collect(), picked)
}

fn has(window: &mut Harness, role: Role, name: &str) -> bool {
    window.find(role, name).is_some()
}

#[test]
fn open_lists_what_a_server_keeps_for_someone_signed_in() {
    // Signed in nowhere, Open is the system's own dialog.
    let mut window = signed_in(&[]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    assert_eq!(window.ui().take_requests(), [Request::OpenDialog]);
    window.assert_closed(Role::Dialog, "Open");

    let mut window = signed_in(&[HOME]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open").assert_open(Role::Dialog, "Open");
    assert_eq!(window.ui().take_requests(), [browse(HOME, Place::Spaces)]);
    assert!(has(&mut window, Role::Status, ASKING), "the wait is said");
    assert!(!has(&mut window, Role::RadioButton, "letters.example.org"), "one server is named, not chosen");

    // The card opens on the person's own space, wherever the server lists it.
    let spaces = Browsed { server: HOME.into(), place: Place::Spaces, spaces: spaces(), entries: Vec::new() };
    window.ui().set_listing(spaces);
    assert_eq!(window.ui().take_requests(), [browse(HOME, within("sam", ""))]);
    let places = ["Family", "Sam Okafor", "Shared with me", "Recent", "Trash"].map(str::to_owned);
    assert_eq!(listed(&mut window, "Places"), (places.to_vec(), vec!["Sam Okafor".to_owned()]));

    window.ui().set_listing(listing(HOME, within("sam", ""), own()));
    assert!(!has(&mut window, Role::Status, ASKING));
    assert_eq!(listed(&mut window, "Documents").0, ["House", "Letters sent", "Roof plan", "Tax 2026"]);
    assert_eq!(window.ui().take_requests(), [], "the answer asks for nothing");
    // A document says when it was last written, by the person's clock.
    let about = |window: &mut Harness, name: &str| {
        window.find(Role::ListBoxOption, name).and_then(|found| found.description).unwrap_or_default()
    };
    assert_eq!(about(&mut window, "Roof plan"), "Document, Modified Today");
    assert_eq!(about(&mut window, "Tax 2026"), "Document, Modified 6 Nov");
    assert_eq!(about(&mut window, "House"), "Folder");

    // Half past eleven last night by UTC was today an hour to the east.
    let mut window = signed_in(&[HOME]);
    window.ui().set_utc_offset(60);
    open(&mut window, vec![document("sam", "Late.let", "d7", 12 * 3_600 + 1_800)]);
    assert_eq!(about(&mut window, "Late"), "Document, Modified Today");
}

/// A document these people wrote, made at this moment as RFC 3339 writes it.
fn written(path: &str, id: &str, by: &[&str], made: &str) -> Entry {
    let contributors = by.iter().map(|name| (*name).to_owned()).collect();
    Entry { contributors, created: Some(made.into()), ..document("sam", path, id, 0) }
}

/// Whose name the pointer is told over the face that stands `at` places along under "Contributors", in the
/// row of the document with this name.
fn face(window: &mut Harness, name: &str, at: usize) -> Option<String> {
    let head = window.find(Role::Label, "Contributors").expect("the heading").bounds;
    let row = window.find(Role::ListBoxOption, name).expect("listed").bounds;
    window.hover_at(Point { x: head.x + 26.0 * at as f32 + 12.0, y: row.y + row.height / 2.0 });
    window.controls().into_iter().find(|found| found.role == Role::Tooltip).map(|found| found.name)
}

/// Whether the row of the document with this name has anything written under a heading of the list.
fn says(window: &mut Harness, name: &str, heading: (Role, &str)) -> bool {
    let head = window.find(heading.0, heading.1).expect("the heading").bounds;
    let row = window.find(Role::ListBoxOption, name).expect("listed").bounds;
    let under =
        |at: Point| at.x >= head.x - 1.0 && at.x < head.x + head.width && at.y > row.y && at.y < row.y + row.height;
    window.paint().items.iter().any(|item| matches!(item, Item::Glyphs { origin, .. } if under(*origin)))
}

#[test]
fn a_row_says_who_wrote_a_document_and_when_it_was_made() {
    let mut window = signed_in(&[HOME]);
    window.resize(1100.0, 820.0);
    let crowd = ["Sam Okafor", "Alice Moreau", "Sota Ono", "Chidi Obi", "Priya Nair"];
    let held = vec![
        folder("sam", "House"),
        written("Roof plan.let", "d1", &crowd[..2], "2026-11-06T09:00:00Z"),
        written("Survey.let", "d2", &crowd[..3], "2025-10-28T09:00:00Z"),
        written("Minutes.let", "d3", &crowd, "2026-11-14T09:00:00Z"),
        document("sam", "Tax 2026.let", "d4", 0),
    ];
    open(&mut window, held);
    let about = |window: &mut Harness, name: &str| {
        window.find(Role::ListBoxOption, name).and_then(|found| found.description).unwrap_or_default()
    };
    // A screen reader is told the row across, as the columns stand.
    assert_eq!(about(&mut window, "Roof plan"), "Document, By Sam Okafor, Alice Moreau, Created 6 Nov, Modified Today");
    assert!(about(&mut window, "Survey").contains("Created 28 Oct 2025"), "a year back says its year");
    assert_eq!(about(&mut window, "Tax 2026"), "Document, Modified Today", "what is not known is not said");
    // The eye is told the same under each heading: the row, and whether it has a day there.
    let (made, modified) = ((Role::Label, "Created"), (Role::Button, "Modified"));
    for (name, days) in [("Roof plan", [true, true]), ("Tax 2026", [false, true]), ("House", [false, false])] {
        let written = [made, modified].map(|heading| says(&mut window, name, heading));
        assert_eq!(written, days, "{name}");
    }

    // Each writer has a face under the heading, which says whose it is under the pointer.
    let faces = [
        ("Roof plan", vec![Some("Sam Okafor"), Some("Alice Moreau"), None]),
        ("Survey", vec![Some("Sam Okafor"), Some("Alice Moreau"), Some("Sota Ono")]),
        // More than the column holds: the last place counts the rest and names them.
        ("Minutes", vec![Some("Sam Okafor"), Some("Alice Moreau"), Some("Sota Ono, Chidi Obi, Priya Nair")]),
        ("Tax 2026", vec![None]),
        ("House", vec![None]),
    ];
    for (name, told) in faces {
        for (at, whose) in told.into_iter().enumerate() {
            assert_eq!(face(&mut window, name, at).as_deref(), whose, "{name}, face {at}");
        }
    }
}

#[test]
fn the_day_a_thing_was_made_or_thrown_away_is_told_by_the_persons_clock() {
    // Half past eleven at night by UTC is the next day an hour to the east.
    for (zone, day) in [(0, "Yesterday"), (60, "Today")] {
        let mut window = signed_in(&[HOME]);
        window.ui().set_utc_offset(zone);
        let about = |window: &mut Harness| {
            let about =
                window.find(Role::ListBoxOption, "Late").and_then(|found| found.description).unwrap_or_default();
            about.split(", ").map(str::to_owned).collect::<Vec<String>>()
        };
        open(&mut window, vec![written("Late.let", "d7", &[], "2026-11-14T23:30:00Z")]);
        assert!(about(&mut window).contains(&format!("Created {day}")), "{zone}");
        window.click(Role::ListBoxOption, "Trash");
        window.ui().set_trash(HOME, vec![thrown(EntryKind::Document, "sam", "", "Late", 12 * 3_600 + 1_800)]);
        assert!(about(&mut window).contains(&format!("Thrown away {day}")), "{zone}");
    }
}

/// Two folders and three documents as a server lists them: folders first, each by name whatever its case.
/// The folder that comes first by name was written longest ago.
fn mixed() -> Vec<Entry> {
    vec![
        Entry { modified: NOON - 30 * DAY, ..folder("sam", "House") },
        folder("sam", "Letters sent"),
        document("sam", "attic notes.let", "d3", DAY),
        document("sam", "Roof plan.let", "d1", 2 * 3_600),
        document("sam", "Tax 2026.let", "d2", 9 * DAY),
    ]
}

/// The headings that are pressed, how the place is then listed, and what "Name" and "Modified" say of it.
type Sorted = (&'static [&'static str], [&'static str; 5], [Option<&'static str>; 2]);

#[test]
fn what_a_place_holds_is_listed_by_name_or_by_when_it_was_last_written() {
    const BY_NAME: [&str; 5] = ["House", "Letters sent", "attic notes", "Roof plan", "Tax 2026"];
    const NEWEST: [&str; 5] = ["House", "Letters sent", "Roof plan", "attic notes", "Tax 2026"];
    // Folders stay first, and keep to their names when the documents go by their days.
    let cases: [Sorted; 7] = [
        (&[], BY_NAME, [Some("Sorted from A to Z"), None]),
        (
            &["Name"],
            ["Letters sent", "House", "Tax 2026", "Roof plan", "attic notes"],
            [Some("Sorted from Z to A"), None],
        ),
        (&["Name", "Name"], BY_NAME, [Some("Sorted from A to Z"), None]),
        (&["Modified"], NEWEST, [None, Some("Sorted newest first")]),
        (
            &["Modified", "Modified"],
            ["House", "Letters sent", "Tax 2026", "attic notes", "Roof plan"],
            [None, Some("Sorted oldest first")],
        ),
        (&["Modified", "Modified", "Modified"], NEWEST, [None, Some("Sorted newest first")]),
        // Another heading starts the way it runs first, whichever way the last one ran.
        (&["Modified", "Name"], BY_NAME, [Some("Sorted from A to Z"), None]),
    ];
    for (pressed, order, said) in cases {
        let mut window = signed_in(&[HOME]);
        open(&mut window, mixed());
        for heading in pressed {
            window.click(Role::Button, heading);
        }
        assert_eq!(listed(&mut window, "Documents").0, order, "{pressed:?}");
        let says = ["Name", "Modified"].map(|heading| window.find(Role::Button, heading).expect("pressed").description);
        assert_eq!(says, said.map(|said| said.map(str::to_owned)), "{pressed:?}");
        assert_eq!(window.ui().take_requests(), [], "{pressed:?}: the server is not asked again");
    }
}

#[test]
fn the_order_that_was_chosen_holds_from_place_to_place_but_for_what_was_open_lately() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, mixed());
    window.click(Role::Button, "Modified");
    let lately = || vec![document("sam", "Tax 2026.let", "d2", 9 * DAY), document("sam", "Roof plan.let", "d1", 0)];
    // What was open lately stays as the server tells it, the last opened first, and is not to be sorted.
    window.click(Role::ListBoxOption, "Recent");
    window.ui().set_listing(listing(HOME, Place::Recent, lately()));
    assert_eq!(listed(&mut window, "Documents").0, ["Tax 2026", "Roof plan"]);
    assert!(!has(&mut window, Role::Button, "Name") && !has(&mut window, Role::Button, "Modified"));
    assert!(has(&mut window, Role::Label, "Modified"), "the columns still say what they hold");
    // Every other place is listed as the last one was.
    window.click(Role::ListBoxOption, "Shared with me");
    window.ui().set_listing(listing(HOME, Place::Shared, lately()));
    assert_eq!(listed(&mut window, "Documents").0, ["Roof plan", "Tax 2026"]);
}

#[test]
fn a_heading_keeps_the_keyboard_while_the_order_is_turned_round() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, mixed().split_off(2));
    let at = |window: &mut Harness| window.focused().map(|found| found.name).unwrap_or_default();
    assert_eq!(at(&mut window), "attic notes", "what a place holds takes the keyboard as it comes");
    // Back from the documents to the heading over them. The row that comes first then does not take the
    // keyboard from it.
    window.key("Shift+Tab");
    assert_eq!(at(&mut window), "Modified");
    window.key("Enter");
    assert_eq!(listed(&mut window, "Documents").0, ["Roof plan", "attic notes", "Tax 2026"]);
    window.key("Enter");
    assert_eq!(listed(&mut window, "Documents").0, ["Tax 2026", "attic notes", "Roof plan"]);
    assert_eq!(at(&mut window), "Modified");
}

/// What the row of this name writes: how many letters each thing it says has. The longest is its name, where
/// little else is said.
fn letters(window: &mut Harness, name: &str) -> Vec<usize> {
    let row = window.find(Role::ListBoxOption, name).expect("listed").bounds;
    let runs = window.paint().items.into_iter().filter_map(|item| match item {
        Item::Glyphs { origin, glyphs, .. } if row.contains(origin) => Some(glyphs.len()),
        _ => None,
    });
    runs.collect()
}

/// Whether a button says its name, or shows its icon alone.
fn named(button: &Found) -> bool {
    button.bounds.width > 2.0 * button.bounds.height
}

/// The windows a card is looked at in, the narrowest the desktop lets a window be: how wide each is, whether
/// a row has room to say who wrote a document and when it was made, and whether it has room for a day.
const WINDOWS: [(f32, bool, bool); 4] =
    [(1100.0, true, true), (700.0, false, true), (420.0, false, false), (360.0, false, false)];

#[test]
fn a_narrow_window_holds_the_card_and_leaves_out_what_it_has_no_room_for() {
    for (width, roomy, dated) in WINDOWS {
        let mut window = signed_in(&[HOME]);
        window.resize(width, 820.0);
        open(&mut window, vec![written("Roof plan.let", "d1", &["Sam Okafor"], "2026-11-06T09:00:00Z")]);
        let card = window.find(Role::Dialog, "Open").expect("the card").bounds;
        assert!(card.x >= 0.0 && card.x + card.width <= width, "{width}: {card:?}");
        for heading in ["Contributors", "Created"] {
            assert_eq!(has(&mut window, Role::Label, heading), roomy, "{width}: {heading}");
        }
        assert_eq!(has(&mut window, Role::Button, "Modified"), dated, "{width}");
        // What is offered is inside the card at any width, and the name is written whole. The buttons over
        // the list say their names where that leaves the way to a folder its room.
        let offered = ["Import", "New document"].into_iter().chain(dated.then_some("Modified"));
        for button in offered {
            let found = window.find(Role::Button, button).expect("offered");
            let ends = found.bounds.x + found.bounds.width;
            assert!(ends < card.x + card.width, "{width}: {button} at {:?} in {card:?}", found.bounds);
            assert!(button == "Modified" || named(&found) == roomy, "{width}: {button}");
        }
        assert!(letters(&mut window, "Roof plan").into_iter().max() >= Some(8), "{width}");
        window.click(Role::ListBoxOption, "Roof plan");
        assert_eq!(window.ui().take_requests(), [Request::OpenFrom { server: HOME.into(), id: "d1".into() }]);

        // A folder some way down keeps what is done in it inside the card: the steps to it are cut short, and
        // a tight card keeps the one that leads up.
        let mut window = signed_in(&[HOME]);
        window.resize(width, 820.0);
        let (house, quotes) = ("House and garden works", "House and garden works/Quotes for the roof");
        open(&mut window, vec![folder("sam", house)]);
        window.click(Role::ListBoxOption, house);
        window.ui().set_listing(listing(HOME, within("sam", house), vec![folder("sam", quotes)]));
        window.click(Role::ListBoxOption, "Quotes for the roof");
        let deep = format!("{quotes}/Roofers and gutters");
        window.ui().set_listing(listing(HOME, within("sam", quotes), vec![folder("sam", &deep)]));
        window.click(Role::ListBoxOption, "Roofers and gutters");
        window.ui().set_listing(listing(HOME, within("sam", &deep), Vec::new()));
        let card = window.find(Role::Dialog, "Open").expect("the card").bounds;
        for button in ["Import", "New document"] {
            let found = window.find(Role::Button, button).expect("offered").bounds;
            assert!(found.x + found.width < card.x + card.width, "{width}: {button} at {found:?} in {card:?}");
        }
        assert_eq!(has(&mut window, Role::Button, house), dated, "{width}: the steps further up");
        let up = window.find(Role::Button, "Quotes for the roof").expect("the step up");
        window.hover_at(up.center());
        assert!(named(&up) || has(&mut window, Role::Tooltip, "Quotes for the roof"), "{width}");
        window.click_at(up.center());
        assert_eq!(window.ui().take_requests().last(), Some(&browse(HOME, within("sam", quotes))), "{width}");
    }
}

#[test]
fn what_is_done_from_a_row_stays_in_reach_beside_its_name_in_a_narrow_window() {
    for (width, roomy, dated) in WINDOWS {
        // A button says its name where a row has room for a day. Where it has not, it shows its icon and is
        // named under the pointer.
        let reached = |window: &mut Harness, button: Found, name: &str| {
            let card = window.find(Role::Dialog, "Open").expect("the card").bounds;
            let within = button.bounds.x + button.bounds.width < card.x + card.width;
            window.hover_at(button.center());
            within && named(&button) == dated && (dated || has(window, Role::Tooltip, name))
        };
        let mut window = signed_in(&[HOME]);
        window.resize(width, 820.0);
        let tags = vec![Tag::ConflictCopy { of: "Minutes.let".into() }];
        let copy = Entry { tags, ..document("sam", "Minutes (conflicted copy).let", "d3", 0) };
        open(&mut window, vec![document("sam", "Minutes.let", "d3", 0), copy]);
        let merge = button(&mut window, "Merge conflicts", "Into Minutes").expect("offered");
        assert!(reached(&mut window, merge.clone(), "Merge conflicts"), "{width}: {:?}", merge.bounds);
        let written = letters(&mut window, "Minutes (conflicted copy)");
        assert!(written.into_iter().max() >= Some(6), "{width}: the copy is told by its name");
        window.click_at(merge.center());
        let asked = Request::MergeConflictsOf { server: HOME.into(), id: "d3".into(), place: within("sam", "") };
        assert_eq!(window.ui().take_requests(), [asked], "{width}");

        let mut window = signed_in(&[HOME]);
        window.resize(width, 820.0);
        open_trash(&mut window, bin());
        assert_eq!(has(&mut window, Role::Label, "Thrown away"), dated, "{width}");
        let back = button(&mut window, "Put back", "Roof plan").expect("offered");
        assert!(reached(&mut window, back.clone(), "Put back"), "{width}: {:?}", back.bounds);
        // The name is written whole, and where the thing was and when it went only where there is the room.
        let written = letters(&mut window, "Roof plan");
        assert!(written.iter().max() >= Some(&8), "{width}: {written:?}");
        assert_eq!(written.len() > 1, dated, "{width}: {written:?}");
        window.click_at(back.center());
        let asked = Request::PutBack { server: HOME.into(), space: "sam".into(), file: "Roof plan.let~1".into() };
        assert_eq!(window.ui().take_requests(), [asked], "{width}");
        // While it waits the button says so in its row, in words where the row is at its widest.
        let waits = button(&mut window, "Putting back", "Roof plan").expect("the row says that it waits").bounds;
        let row = window.find(Role::ListBoxOption, "Roof plan").expect("listed").bounds;
        assert!(waits.x > row.x && waits.x + waits.width < row.x + row.width, "{width}: {waits:?} in {row:?}");
        assert_eq!(waits.width > 2.0 * waits.height, roomy, "{width}: {waits:?}");
    }
}

#[test]
fn a_folder_is_gone_into_and_left_by_the_way_that_led_to_it() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, own());
    window.click(Role::ListBoxOption, "House");
    assert_eq!(window.ui().take_requests(), [browse(HOME, within("sam", "House"))]);
    assert!(has(&mut window, Role::Status, ASKING), "what was listed is not this folder's");
    // An answer about the place that was left comes late, and is let go.
    window.ui().set_listing(listing(HOME, within("sam", ""), own()));
    assert!(has(&mut window, Role::Status, ASKING));
    window.ui().set_listing(listing(HOME, within("sam", "House"), vec![folder("sam", "House/Quotes")]));
    window.click(Role::ListBoxOption, "Quotes");
    assert_eq!(window.ui().take_requests(), [browse(HOME, within("sam", "House/Quotes"))]);
    window.ui().set_listing(listing(HOME, within("sam", "House/Quotes"), Vec::new()));
    assert!(has(&mut window, Role::Status, "This folder is empty."));
    assert_eq!(listed(&mut window, "Places").1, ["Sam Okafor"], "a folder is in its space");
    // The spaces said again, as when one is new, leave the card where it is.
    window.ui().set_listing(Browsed {
        server: HOME.into(),
        place: Place::Spaces,
        spaces: spaces(),
        entries: Vec::new(),
    });
    assert_eq!(window.ui().take_requests(), []);

    // Each step of the way leads back, but the last, which is where the card is.
    assert!(!has(&mut window, Role::Button, "Quotes"));
    window.click(Role::Button, "House");
    assert_eq!(window.ui().take_requests(), [browse(HOME, within("sam", "House"))]);
    window.click(Role::Button, "Sam Okafor");
    assert_eq!(window.ui().take_requests(), [browse(HOME, within("sam", ""))]);
}

#[test]
fn the_places_of_a_server_are_its_spaces_what_is_shared_and_what_was_open_lately() {
    // The place that is pressed, what the shell is asked for, and what the place says while it holds nothing.
    let cases = [
        ("Family", within("family", ""), "This folder is empty."),
        ("Shared with me", Place::Shared, "No documents have been shared with you yet."),
        ("Recent", Place::Recent, "You have not opened any documents from this server yet."),
        ("Sam Okafor", within("sam", ""), "This folder is empty."),
    ];
    for (name, place, empty) in cases {
        let mut window = signed_in(&[HOME]);
        open(&mut window, own());
        window.click(Role::ListBoxOption, name);
        assert_eq!(window.ui().take_requests(), [browse(HOME, place.clone())], "{name}");
        assert_eq!(listed(&mut window, "Places").1, [name], "{name}");
        window.ui().set_listing(listing(HOME, place.clone(), Vec::new()));
        assert!(has(&mut window, Role::Status, empty), "{name}");
        let folder = matches!(place, Place::Folder { .. });
        assert_eq!(has(&mut window, Role::Group, "Folder"), folder, "{name}: only a folder has a way that led to it");
        // A document that reaches the person only through a share has no space, and opens all the same.
        let shared = Entry { space: None, path: String::new(), ..document("x", "Wedding plan.let", "d9", DAY) };
        window.ui().set_listing(listing(HOME, place, vec![shared]));
        window.click(Role::ListBoxOption, "Wedding plan");
        assert_eq!(window.ui().take_requests(), [Request::OpenFrom { server: HOME.into(), id: "d9".into() }]);
    }
}

#[test]
fn what_is_too_long_for_a_row_is_cut_short_inside_the_card() {
    const LONG: &str = "Roof plan and every quote the roofers sent for the north slope and the gutters in March";
    let mut window = signed_in(&[HOME]);
    let people = vec!["Alice Moreau".into(), "Priya Nair".into(), "Chidi Obi".into()];
    let tags = vec![Tag::Editing { people }, Tag::Shared { with: 2 }, Tag::Links { count: 2 }];
    let busy = Entry { tags, ..document("sam", "Minutes of the residents' meeting.let", "d2", 0) };
    open(&mut window, vec![document("sam", &format!("{LONG}.let"), "d1", 0), busy]);

    let list = window.find(Role::ListBox, "Documents").expect("the list").bounds;
    for name in [LONG, "Minutes of the residents' meeting"] {
        let row = window.find(Role::ListBoxOption, name).expect("listed").bounds;
        assert!(row.x + row.width <= list.x + list.width + 0.5, "{name}: {row:?} in {list:?}");
    }
    let card = window.find(Role::Dialog, "Open").expect("the card").bounds;
    assert!(list.x + list.width < card.x + card.width, "the list is inside the card: {list:?} in {card:?}");

    // So is the way to a folder with such a name, which ends in the button that starts a document there.
    window.ui().set_listing(listing(HOME, within("sam", ""), vec![folder("sam", LONG)]));
    window.click(Role::ListBoxOption, LONG);
    window.ui().set_listing(listing(HOME, within("sam", LONG), Vec::new()));
    let new = window.find(Role::Button, "New document").expect("a document can be started here").bounds;
    assert!(new.x + new.width < card.x + card.width, "{new:?} in {card:?}");
}

#[test]
fn servers_with_long_addresses_are_chosen_between_inside_the_card() {
    let home = "https://letters.a-very-long-subdomain-for-the-family.documents.example.org";
    let work = "https://documents.the-northern-slate-quarry-cooperative.example.net";
    let mut window = signed_in(&[home, work]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    let card = window.find(Role::Dialog, "Open").expect("the card").bounds;
    // Each is cut short where it is chosen, and the way to this device's files keeps its place beside them.
    let device = window.find(Role::Button, "This device").expect("the way to a file on this device").bounds;
    assert!(device.x + device.width < card.x + card.width, "{device:?} in {card:?}");
}

#[test]
fn a_document_that_is_pressed_is_opened_and_the_card_goes() {
    let mut window = signed_in(&[HOME]);
    let editing = Tag::Editing { people: vec!["Alice Moreau".into()] };
    let busy = Entry { tags: vec![editing], ..document("sam", "Minutes.let", "d3", 0) };
    // A file that could not be read is listed, and is not to be opened.
    let damaged = Entry { id: None, tags: vec![Tag::Damaged], ..document("sam", "Old notes.let", "", 0) };
    let copy = Tag::ConflictCopy { of: "Minutes.let".into() };
    let copy = Entry { role: Grade::Reader, tags: vec![copy], ..document("sam", "Minutes (conflict).let", "d3", 0) };
    // Who else reaches a document, and what an import left to settle in it.
    let reached = vec![Tag::ImportNotes { count: 3 }, Tag::Shared { with: 2 }, Tag::Links { count: 1 }];
    let reached = Entry { tags: reached, ..document("sam", "Roof survey.let", "d5", 0) };
    let unshared = Entry { tags: vec![Tag::Links { count: 0 }], ..document("sam", "Kitchen.let", "d6", 0) };
    open(&mut window, vec![busy, copy, damaged, reached, unshared]);
    let about = |window: &mut Harness, name: &str| window.find(Role::ListBoxOption, name).expect("listed");
    // What a row says of its file, beside its name.
    let notes = [
        ("Minutes", "Alice Moreau editing"),
        ("Minutes (conflict)", "Conflict copy"),
        ("Old notes", "Could not be read"),
        ("Roof survey", "3 import notes"),
        ("Roof survey", "Shared with 2 others"),
        ("Roof survey", "1 link"),
    ];
    for (name, note) in notes {
        let said = about(&mut window, name).description.unwrap_or_default();
        assert!(said.split(", ").any(|part| part == note), "{name}: {said}");
    }
    let said = about(&mut window, "Kitchen").description.unwrap_or_default();
    assert!(!said.contains("link"), "no links are nothing to say: {said}");
    assert!(about(&mut window, "Old notes").disabled);
    window.click(Role::ListBoxOption, "Old notes");
    assert_eq!(window.ui().take_requests(), []);

    window.click(Role::ListBoxOption, "Minutes");
    assert_eq!(window.ui().take_requests(), [Request::OpenFrom { server: HOME.into(), id: "d3".into() }]);
    window.assert_closed(Role::Dialog, "Open");
}

#[test]
fn a_conflict_copy_is_merged_into_its_original_from_its_row() {
    let original = || document("sam", "House/Minutes.let", "d3", 0);
    // A copy goes by the id of the document it is a copy of.
    let copy = |role| {
        let tags = vec![Tag::ConflictCopy { of: "House/Minutes.let".into() }];
        Entry { role, tags, ..document("sam", "House/Minutes (conflicted copy).let", "d3", 0) }
    };
    let about = |window: &mut Harness| window.find(Role::ListBoxOption, "Minutes (conflicted copy)").expect("listed");
    let mut window = signed_in(&[HOME]);
    open(&mut window, vec![original(), copy(Grade::Editor)]);
    let merge = window.find(Role::Button, "Merge conflicts").expect("offered to whoever edits there");
    assert_eq!(merge.description.as_deref(), Some("Into Minutes"));
    let said = about(&mut window).description.unwrap_or_default();
    assert!(!said.contains("Conflict copy"), "the button says what the note would: {said}");
    // The row itself opens nothing: the id is the original's.
    let row = about(&mut window).bounds;
    window.click_at(Point { x: row.x + 12.0, y: row.y + row.height / 2.0 });
    assert_eq!(window.ui().take_requests(), []);
    window.assert_open(Role::Dialog, "Open");

    window.click(Role::Button, "Merge conflicts");
    let asked = Request::MergeConflictsOf { server: HOME.into(), id: "d3".into(), place: within("sam", "") };
    assert_eq!(window.ui().take_requests(), std::slice::from_ref(&asked));
    // The list stays while the server merges, and the row alone says that it waits.
    assert_eq!(listed(&mut window, "Documents").0, ["Minutes", "Minutes (conflicted copy)"]);
    assert!(!has(&mut window, Role::Status, ASKING));
    let waits = button(&mut window, "Merging", "Into Minutes").expect("the row says what is being done");
    assert!(waits.disabled, "which is not a thing to press");
    assert!(!has(&mut window, Role::Button, "Merge conflicts"), "and the merge is not asked for twice");
    // Why it could not be done is said over the list, and the merge can be asked for again.
    window.ui().refuse_browse(REFUSAL);
    window.assert_open(Role::Alert, REFUSAL);
    assert_eq!(listed(&mut window, "Documents").0.len(), 2, "the list is still there");
    window.click(Role::Button, "Merge conflicts");
    assert_eq!(window.ui().take_requests(), [asked]);
    window.assert_closed(Role::Alert, REFUSAL);
    // The answer is the place without the copy.
    window.ui().set_listing(listing(HOME, within("sam", ""), vec![original()]));
    assert_eq!(listed(&mut window, "Documents").0, ["Minutes"]);
    assert!(!has(&mut window, Role::Button, "Merging"));

    // Whoever may not edit there is told what the file is, and can neither merge it nor open it.
    for role in [Grade::Commenter, Grade::Reader] {
        let mut window = signed_in(&[HOME]);
        open(&mut window, vec![original(), copy(role)]);
        assert!(!has(&mut window, Role::Button, "Merge conflicts"), "{role:?}");
        let row = about(&mut window);
        assert!(row.disabled, "{role:?}");
        assert!(row.description.unwrap_or_default().contains("Conflict copy"), "{role:?}");
        window.click(Role::ListBoxOption, "Minutes (conflicted copy)");
        assert_eq!(window.ui().take_requests(), [], "{role:?}");
    }
}

/// Two documents with a conflict copy each, and one without, less those of these names.
fn copied(without: &[&str]) -> Vec<Entry> {
    let copy = |name: &str, id: &str| {
        let tags = vec![Tag::ConflictCopy { of: format!("{name}.let") }];
        Entry { tags, ..document("sam", &format!("{name} (conflicted copy).let"), id, 0) }
    };
    let held = vec![
        document("sam", "Minutes.let", "d3", 0),
        copy("Minutes", "d3"),
        document("sam", "Rota.let", "d5", 0),
        copy("Rota", "d5"),
        document("sam", "Tax 2026.let", "d2", 0),
    ];
    held.into_iter().filter(|entry| !without.contains(&entry.name.as_str())).collect()
}

/// What the rows that wait for a merge say they are merged into.
fn merging(window: &mut Harness) -> Vec<String> {
    let waits = window.controls().into_iter().filter(|found| found.role == Role::Button && found.name == "Merging");
    waits.filter_map(|found| found.description).collect()
}

#[test]
fn the_rows_beside_one_that_waits_for_its_answer_stay_usable() {
    let merge = |window: &mut Harness, into: &str| {
        let merge = button(window, "Merge conflicts", into).expect("offered");
        window.click_at(merge.center());
    };
    let asked = |id: &str| Request::MergeConflictsOf { server: HOME.into(), id: id.into(), place: within("sam", "") };
    let mut window = signed_in(&[HOME]);
    open(&mut window, copied(&[]));
    // Another copy is merged while the first waits.
    merge(&mut window, "Into Minutes");
    assert_eq!(merging(&mut window), ["Into Minutes"]);
    merge(&mut window, "Into Rota");
    assert_eq!(window.ui().take_requests(), [asked("d3"), asked("d5")]);
    assert_eq!(merging(&mut window), ["Into Minutes", "Into Rota"]);
    // Why one could not be done is said of the first that was asked, and the other waits on.
    window.ui().refuse_browse(REFUSAL);
    assert_eq!(merging(&mut window), ["Into Rota"]);
    merge(&mut window, "Into Minutes");

    // A place that is left takes its waits with it.
    window.click(Role::ListBoxOption, "Family");
    window.click(Role::ListBoxOption, "Sam Okafor");
    window.ui().set_listing(listing(HOME, within("sam", ""), copied(&[])));
    assert_eq!(merging(&mut window), Vec::<String>::new());
    merge(&mut window, "Into Minutes");
    merge(&mut window, "Into Rota");

    // Each answer is what the place holds by then, in whatever order they come: the row it leaves out is
    // done, and the other waits on.
    let left = || listing(HOME, within("sam", ""), copied(&["Rota (conflicted copy)"]));
    window.ui().set_listing(left());
    assert_eq!(merging(&mut window), ["Into Minutes"]);
    assert!(!has(&mut window, Role::ListBoxOption, "Rota (conflicted copy)"));
    // An answer that still lists the copy ends its wait all the same, so that no row waits for ever.
    window.ui().set_listing(left());
    assert_eq!(merging(&mut window), Vec::<String>::new());
    assert!(button(&mut window, "Merge conflicts", "Into Minutes").is_some(), "and it can be asked for again");

    // A document beside a row that waits is opened as ever.
    merge(&mut window, "Into Minutes");
    window.ui().take_requests();
    window.click(Role::ListBoxOption, "Tax 2026");
    assert_eq!(window.ui().take_requests(), [Request::OpenFrom { server: HOME.into(), id: "d2".into() }]);
    window.assert_closed(Role::Dialog, "Open");
}

#[test]
fn a_row_that_waits_has_the_keyboard_and_the_list_is_walked_on_when_it_goes() {
    let at = |window: &mut Harness| window.focused().map(|found| found.name).unwrap_or_default();
    // Presses this key until the keyboard is on what has this name.
    let reach = |window: &mut Harness, key: &str, name: &str| {
        for _ in 0..8 {
            if at(window) == name {
                break;
            }
            window.key(key);
        }
        assert_eq!(at(window), name, "reached from the keyboard with {key}");
    };
    // The first copy is merged from the keyboard, in the order a place is first listed in or in one that
    // was chosen, after which the top of the list no longer asks for the keyboard as it comes.
    let merged = |chosen: bool| {
        let mut window = signed_in(&[HOME]);
        open(&mut window, copied(&[]));
        if chosen {
            window.click(Role::Button, "Name").click(Role::Button, "Name");
        }
        reach(&mut window, "Down", "Merge conflicts");
        window.key("Enter");
        assert_eq!(merging(&mut window), ["Into Minutes"]);
        window
    };
    let done = || listing(HOME, within("sam", ""), copied(&["Minutes (conflicted copy)"]));
    for chosen in [false, true] {
        // The button is none while it waits, so its row has the keyboard, and the rows after it are walked to.
        let mut window = merged(chosen);
        assert_eq!(at(&mut window), "Minutes (conflicted copy)", "chosen: {chosen}");
        window.key("Down");
        assert_eq!(at(&mut window), "Rota", "chosen: {chosen}");
        // The answer leaves the keyboard where the person has it by then.
        window.ui().set_listing(done());
        assert_eq!(at(&mut window), "Rota", "chosen: {chosen}");

        // An answer that takes away the row the keyboard is on leaves it on the list, at its top.
        let mut window = merged(chosen);
        window.ui().set_listing(done());
        assert_eq!(at(&mut window), "Minutes", "chosen: {chosen}");
        window.key("Down");
        assert_eq!(at(&mut window), "Rota", "chosen: {chosen}: and the list is walked on");
    }

    // So it is in the trash.
    let mut window = signed_in(&[HOME]);
    open_trash(&mut window, bin());
    reach(&mut window, "Tab", "Roof plan");
    reach(&mut window, "Down", "Old letters");
    window.key("Down");
    assert_eq!(at(&mut window), "Put back");
    window.key("Enter");
    assert_eq!(at(&mut window), "Old letters", "the row that waits has the keyboard");
    window.ui().set_trash(HOME, bin().into_iter().filter(|gone| gone.name != "Old letters").collect());
    assert_eq!(at(&mut window), "Roof plan", "and the top of what is left has it when the row has gone");
}

#[test]
fn the_list_is_walked_and_a_document_opened_from_the_keyboard() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, own());
    let at = |window: &mut Harness| window.focused().map(|found| found.name).unwrap_or_default();
    // What a place holds takes the keyboard as it comes: a folder is gone into and walked on.
    window.key("Down");
    assert_eq!(at(&mut window), "House");
    window.key("Enter");
    assert_eq!(window.ui().take_requests(), [browse(HOME, within("sam", "House"))]);
    let held = vec![document("sam", "House/Roof plan.let", "d1", 0), document("sam", "House/Survey.let", "d3", DAY)];
    window.ui().set_listing(listing(HOME, within("sam", "House"), held));
    assert_eq!(at(&mut window), "Roof plan");
    window.key("Down").key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::OpenFrom { server: HOME.into(), id: "d3".into() }]);
}

#[test]
fn the_places_are_one_stop_of_the_tab_key_at_the_place_that_is_looked_at() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, own());
    let at = |window: &mut Harness| window.focused().map(|found| found.name).unwrap_or_default();
    // Back from the documents, past the headings and what else stands over them, to the places.
    for _ in 0..6 {
        if at(&mut window) != "Sam Okafor" {
            window.key("Shift+Tab");
        }
    }
    assert_eq!(at(&mut window), "Sam Okafor");
    window.key("Shift+Tab");
    assert_eq!(at(&mut window), "This device", "the places cost one press to pass");
    window.key("Tab").key("Up").key("Enter");
    assert_eq!(window.ui().take_requests(), [browse(HOME, within("family", ""))]);

    // While the server is asked, what is typed does not reach the page under the card.
    window.type_text("x").key("Enter").key("Backspace");
    window.assert_markdown("# Roof plan\n\nThe roofer needs the scaffold up by 3 March.");
}

/// What is asked of a folder, by its server, its space and its path there.
type Asked = fn((String, String, String)) -> Request;

#[test]
fn a_document_is_started_or_imported_in_the_folder_that_is_looked_at() {
    let at = |path: &str| (HOME.to_owned(), "sam".to_owned(), path.to_owned());
    // The button, what it says of the folder, and what the shell is asked for there.
    let new = |(server, space, path)| Request::NewAt { server, space, path };
    let import = |(server, space, path)| Request::ImportAt { server, space, path };
    let cases: [(&str, &str, Asked); 2] = [("New document", "In", new), ("Import", "Into", import)];
    for (button, word, asked) in cases {
        // The folder that is gone into, and its path in the space.
        for (folder, path) in [(None, ""), (Some("House"), "House")] {
            let mut window = signed_in(&[HOME]);
            open(&mut window, own());
            if let Some(folder) = folder {
                window.click(Role::ListBoxOption, folder);
                window.ui().take_requests();
            }
            let offered = window.find(Role::Button, button).expect("offered");
            let said = format!("{word} {}", folder.unwrap_or("Sam Okafor"));
            assert_eq!(offered.description.as_deref(), Some(said.as_str()), "{button}");
            window.click(Role::Button, button);
            assert_eq!(window.ui().take_requests(), [asked(at(path))], "{button} in {folder:?}");
            window.assert_closed(Role::Dialog, "Open");
        }
    }
}

#[test]
fn a_document_is_started_only_where_the_person_may_write() {
    // What the person may do in the space, the place that is pressed, and whether a document is offered.
    let cases = [
        (Grade::Owner, "Family", true),
        (Grade::Editor, "Family", true),
        (Grade::Commenter, "Family", false),
        (Grade::Reader, "Family", false),
        (Grade::Owner, "Shared with me", false),
        (Grade::Owner, "Recent", false),
    ];
    for (role, place, offered) in cases {
        let mut window = signed_in(&[HOME]);
        window.click(Role::Button, "More").click(Role::MenuItem, "Open");
        let spaces = vec![Space { role, ..space("family", "Family", SpaceKind::Shared) }];
        window.ui().set_listing(Browsed { server: HOME.into(), place: Place::Spaces, spaces, entries: Vec::new() });
        window.click(Role::ListBoxOption, place);
        for button in ["New document", "Import"] {
            assert_eq!(has(&mut window, Role::Button, button), offered, "{button}: {role:?} in {place}");
        }
    }
}

/// Something in the trash of a space, thrown away this long ago out of this folder.
fn thrown(kind: EntryKind, space: &str, folder: &str, name: &str, ago: u64) -> Trashed {
    let (name, space, folder) = (name.to_owned(), space.to_owned(), folder.to_owned());
    Trashed { kind, file: format!("{name}.let~1"), name, space, folder, thrown: NOON - ago, role: Grade::Editor }
}

/// What is in the trash, the last thrown away first: a document and a folder of the person's own space, and
/// a document of a space they may not edit in.
fn bin() -> Vec<Trashed> {
    vec![
        thrown(EntryKind::Document, "sam", "House/Quotes", "Roof plan", 3_600),
        thrown(EntryKind::Folder, "sam", "", "Old letters", DAY),
        Trashed { role: Grade::Commenter, ..thrown(EntryKind::Document, "family", "", "Rota", 9 * DAY) },
    ]
}

/// The button of this name that says this of itself: the one in the row of the thing it is about.
fn button(window: &mut Harness, name: &str, about: &str) -> Option<Found> {
    let fits = |found: &Found| found.role == Role::Button && found.name == name;
    window.controls().into_iter().find(|found| fits(found) && found.description.as_deref() == Some(about))
}

/// Opens the card and looks at the trash, which holds this.
fn open_trash(window: &mut Harness, held: Vec<Trashed>) {
    open(window, own());
    window.click(Role::ListBoxOption, "Trash");
    assert_eq!(window.ui().take_requests(), [Request::BrowseTrash { server: HOME.into() }]);
    window.ui().set_trash(HOME, held);
}

#[test]
fn what_was_thrown_away_is_listed_with_where_it_was_and_when_it_went() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, own());
    window.click(Role::ListBoxOption, "Trash");
    assert_eq!(window.ui().take_requests(), [Request::BrowseTrash { server: HOME.into() }]);
    assert!(has(&mut window, Role::Status, ASKING), "the wait is said");
    assert_eq!(listed(&mut window, "Places").1, ["Trash"]);
    // Nothing is started in the trash, and no way leads into it.
    assert!(!has(&mut window, Role::Button, "New document") && !has(&mut window, Role::Group, "Folder"));

    // An answer from another server is not this trash's.
    window.ui().set_trash(WORK, bin());
    assert!(has(&mut window, Role::Status, ASKING));
    window.ui().set_trash(HOME, bin());
    assert_eq!(listed(&mut window, "Trash").0, ["Roof plan", "Old letters", "Rota"], "as the server tells it");
    let about = |window: &mut Harness, name: &str| {
        window.find(Role::ListBoxOption, name).and_then(|found| found.description).unwrap_or_default()
    };
    assert_eq!(about(&mut window, "Roof plan"), "Document, Was in Sam Okafor / House / Quotes, Thrown away Today");
    assert_eq!(about(&mut window, "Old letters"), "Folder, Was in Sam Okafor, Thrown away Yesterday");
    assert_eq!(about(&mut window, "Rota"), "Document, Was in Family, Thrown away 6 Nov");
    // The eye is told the day under its heading.
    assert!(says(&mut window, "Roof plan", (Role::Label, "Thrown away")));
    // A row opens nothing.
    window.click(Role::ListBoxOption, "Old letters");
    assert_eq!(window.ui().take_requests(), []);
    window.assert_open(Role::Dialog, "Open");

    window.ui().set_trash(HOME, Vec::new());
    assert!(has(&mut window, Role::Status, "The trash is empty."));
}

#[test]
fn whoever_edits_in_a_space_puts_back_what_was_thrown_away_there() {
    let mut window = signed_in(&[HOME]);
    open_trash(&mut window, bin());
    assert!(button(&mut window, "Put back", "Rota").is_none(), "whoever only comments there puts nothing back");
    let back = button(&mut window, "Put back", "Roof plan").expect("offered");
    let card = window.find(Role::Dialog, "Open").expect("the card").bounds;
    assert!(back.bounds.x + back.bounds.width < card.x + card.width, "{:?} in {card:?}", back.bounds);
    window.click_at(back.center());
    let asked =
        |name: &str| Request::PutBack { server: HOME.into(), space: "sam".into(), file: format!("{name}.let~1") };
    assert_eq!(window.ui().take_requests(), [asked("Roof plan")]);
    // The trash stays while the server puts it back, and the row alone says that it waits.
    assert_eq!(listed(&mut window, "Trash").0, ["Roof plan", "Old letters", "Rota"]);
    assert!(!has(&mut window, Role::Status, ASKING));
    let waits = button(&mut window, "Putting back", "Roof plan").expect("the row says what is being done");
    assert!(waits.disabled, "which is not a thing to press");
    assert!(waits.bounds.x + waits.bounds.width < card.x + card.width, "{:?} in {card:?}", waits.bounds);
    assert!(button(&mut window, "Put back", "Roof plan").is_none(), "and it is not asked for twice");
    // Why it could not be done is said over the trash, and it can be asked for again.
    window.ui().refuse_browse(REFUSAL);
    window.assert_open(Role::Alert, REFUSAL);
    assert_eq!(listed(&mut window, "Trash").0.len(), 3, "the trash is still there");
    let again = button(&mut window, "Put back", "Roof plan").expect("offered again");
    window.click_at(again.center());
    window.assert_closed(Role::Alert, REFUSAL);
    // What is beside it is put back meanwhile.
    let other = button(&mut window, "Put back", "Old letters").expect("still offered");
    window.click_at(other.center());
    assert_eq!(window.ui().take_requests(), [asked("Roof plan"), asked("Old letters")]);
    // Each answer is the trash as it is by then, in whatever order they come: what it leaves out is back,
    // and the other waits on.
    let left = || bin().into_iter().filter(|gone| gone.name != "Old letters").collect::<Vec<_>>();
    window.ui().set_trash(HOME, left());
    assert_eq!(listed(&mut window, "Trash").0, ["Roof plan", "Rota"]);
    assert!(button(&mut window, "Putting back", "Roof plan").is_some());
    // An answer that still lists it ends its wait all the same, so that no row waits for ever.
    window.ui().set_trash(HOME, left());
    assert!(button(&mut window, "Put back", "Roof plan").is_some(), "and it can be asked for again");
}

#[test]
fn the_trash_and_the_places_do_not_take_each_others_answers() {
    // A place's late answer leaves the trash, and why it could not be read, as they are.
    let mut window = signed_in(&[HOME]);
    open(&mut window, own());
    window.click(Role::ListBoxOption, "Trash");
    window.ui().refuse_browse(REFUSAL);
    window.ui().set_listing(listing(HOME, within("sam", ""), own()));
    window.assert_open(Role::Alert, REFUSAL);
    assert!(listed(&mut window, "Documents").0.is_empty());
    // A refusal that comes after the trash was told is about a place that was left.
    window.ui().set_trash(HOME, bin());
    window.ui().refuse_browse(REFUSAL);
    window.assert_closed(Role::Alert, REFUSAL);

    // The trash's late answer leaves a place, and why it could not be read, as they are.
    window.click(Role::ListBoxOption, "Sam Okafor");
    assert_eq!(window.ui().take_requests().last(), Some(&browse(HOME, within("sam", ""))));
    assert_eq!(listed(&mut window, "Places").1, ["Sam Okafor"]);
    window.ui().refuse_browse(REFUSAL);
    window.ui().set_trash(HOME, bin());
    window.assert_open(Role::Alert, REFUSAL);
    assert!(listed(&mut window, "Trash").0.is_empty());

    // The trash is asked for afresh each time it is turned to: neither what it held before nor what a place
    // said of itself stands there meanwhile.
    window.click(Role::ListBoxOption, "Trash");
    assert_eq!(window.ui().take_requests().last(), Some(&Request::BrowseTrash { server: HOME.into() }));
    window.assert_closed(Role::Alert, REFUSAL);
    assert!(has(&mut window, Role::Status, ASKING));
    assert!(listed(&mut window, "Trash").0.is_empty());
    // And a place that is turned back to is one a document is started in again.
    window.click(Role::ListBoxOption, "Sam Okafor");
    window.ui().set_listing(listing(HOME, within("sam", ""), own()));
    assert!(has(&mut window, Role::Button, "New document"));
}

#[test]
fn the_trash_is_looked_at_before_the_spaces_are_told_and_left_with_its_server() {
    let mut window = signed_in(&[HOME, WORK]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    window.click(Role::ListBoxOption, "Trash");
    window.ui().take_requests();
    // The spaces come late: the card stays where the person went.
    window.ui().set_listing(Browsed {
        server: HOME.into(),
        place: Place::Spaces,
        spaces: spaces(),
        entries: Vec::new(),
    });
    assert_eq!(window.ui().take_requests(), []);
    assert_eq!(listed(&mut window, "Places").1, ["Trash"]);
    window.ui().set_trash(HOME, bin());
    assert_eq!(listed(&mut window, "Trash").0.len(), 3);

    // Another server is opened on its own space, as ever.
    window.click(Role::RadioButton, "docs.example.net");
    assert_eq!(window.ui().take_requests(), [browse(WORK, Place::Spaces)]);
    let group = vec![space("office", "Office", SpaceKind::Shared)];
    window.ui().set_listing(Browsed { server: WORK.into(), place: Place::Spaces, spaces: group, entries: Vec::new() });
    assert_eq!(window.ui().take_requests(), [browse(WORK, within("office", ""))]);
    assert_eq!(listed(&mut window, "Places").1, ["Office"]);
}

#[test]
fn a_file_of_this_device_is_still_one_press_away() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, own());
    window.click(Role::Button, "This device");
    assert_eq!(window.ui().take_requests(), [Request::OpenDialog]);
    window.assert_closed(Role::Dialog, "Open");
}

#[test]
fn each_server_the_person_is_signed_in_to_is_looked_at_in_turn() {
    let mut window = signed_in(&[HOME, WORK]);
    open(&mut window, own());
    let chosen =
        |window: &mut Harness, name: &str| window.find(Role::RadioButton, name).and_then(|found| found.toggled);
    assert_eq!(chosen(&mut window, "letters.example.org"), Some(true));
    window.click(Role::RadioButton, "letters.example.org");
    assert_eq!(window.ui().take_requests(), [], "the server that is looked at is not asked again");
    window.click(Role::RadioButton, "docs.example.net");
    assert_eq!(window.ui().take_requests(), [browse(WORK, Place::Spaces)]);
    assert_eq!(chosen(&mut window, "docs.example.net"), Some(true));
    assert!(listed(&mut window, "Documents").0.is_empty(), "nothing of the other server stays listed");
    assert_eq!(listed(&mut window, "Places").0, ["Shared with me", "Recent", "Trash"], "its spaces are not known yet");

    // An answer that comes late, from the server that was left, is let go.
    window.ui().set_listing(listing(HOME, within("sam", ""), own()));
    assert!(listed(&mut window, "Documents").0.is_empty());
    window.ui().set_listing(Browsed {
        server: HOME.into(),
        place: Place::Spaces,
        spaces: spaces(),
        entries: Vec::new(),
    });
    assert_eq!(listed(&mut window, "Places").0, ["Shared with me", "Recent", "Trash"]);
    assert_eq!(window.ui().take_requests(), []);

    // A server where the person has no space of their own opens on its first space.
    let group = vec![space("office", "Office", SpaceKind::Shared)];
    window.ui().set_listing(Browsed { server: WORK.into(), place: Place::Spaces, spaces: group, entries: Vec::new() });
    assert_eq!(window.ui().take_requests(), [browse(WORK, within("office", ""))]);
    window.ui().set_listing(listing(WORK, within("office", ""), vec![document("office", "Rota.let", "w1", 0)]));
    window.click(Role::ListBoxOption, "Rota");
    assert_eq!(window.ui().take_requests(), [Request::OpenFrom { server: WORK.into(), id: "w1".into() }]);

    // The card comes back to the server it was left on.
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    assert_eq!(window.ui().take_requests(), [browse(WORK, Place::Spaces)]);
    // Signed out of it since, the card turns to one the person is still signed in to.
    window.key("Escape");
    window.ui().set_servers(vec![server(HOME)]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    assert_eq!(window.ui().take_requests(), [browse(HOME, Place::Spaces)]);
    assert!(!has(&mut window, Role::RadioButton, "docs.example.net"));

    // The card of a server whose session ends while it is looked at goes. Another server's coming and
    // going leaves it.
    window.ui().set_servers(vec![server(HOME), server(WORK)]);
    window.assert_open(Role::Dialog, "Open");
    window.ui().set_servers(vec![server(WORK)]);
    window.assert_closed(Role::Dialog, "Open");
    window.ui().set_listing(listing(HOME, Place::Spaces, Vec::new()));
    assert_eq!(window.ui().take_requests(), [], "its late answer asks nothing more");
}

#[test]
fn a_server_where_the_person_has_no_space_opens_on_what_is_shared_with_them() {
    let mut window = signed_in(&[WORK]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    window.ui().take_requests();
    window.ui().set_listing(listing(WORK, Place::Spaces, Vec::new()));
    assert_eq!(window.ui().take_requests(), [browse(WORK, Place::Shared)]);
    assert_eq!(
        listed(&mut window, "Places"),
        (vec!["Shared with me".into(), "Recent".into(), "Trash".into()], vec!["Shared with me".into()])
    );
}

#[test]
fn a_place_the_server_does_not_tell_says_why() {
    let mut window = signed_in(&[HOME]);
    open(&mut window, own());
    window.click(Role::ListBoxOption, "Family");
    window.ui().take_requests();
    window.ui().refuse_browse(REFUSAL);
    window.assert_open(Role::Alert, REFUSAL).assert_open(Role::Dialog, "Open");
    assert!(!has(&mut window, Role::Status, ASKING));
    assert_eq!(window.ui().take_requests(), [], "a server out of reach is not asked for ever");

    // Another place is asked for, and the sentence goes with it.
    window.click(Role::ListBoxOption, "Recent");
    window.assert_closed(Role::Alert, REFUSAL);
    assert_eq!(window.ui().take_requests(), [browse(HOME, Place::Recent)]);

    // A refusal that comes after the place was told is about one that was left.
    window.ui().set_listing(listing(HOME, Place::Recent, own()));
    window.ui().refuse_browse(REFUSAL);
    window.assert_closed(Role::Alert, REFUSAL);
    assert_eq!(listed(&mut window, "Documents").0.len(), 4);

    // A server that will not even tell its spaces still has the other places and this device.
    let mut window = signed_in(&[HOME]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    window.ui().take_requests();
    window.ui().refuse_browse(REFUSAL);
    window.assert_open(Role::Alert, REFUSAL);
    assert_eq!(listed(&mut window, "Places").0, ["Shared with me", "Recent", "Trash"]);
    assert!(has(&mut window, Role::Button, "This device"));
}

#[test]
fn the_landing_card_makes_way_for_the_servers_documents() {
    let mut window = signed_in(&[HOME]);
    window.ui().set_landing(Some(Landing::default()));
    window.click(Role::Button, "Open").assert_open(Role::Dialog, "Open");
    assert_eq!(window.ui().take_requests(), [browse(HOME, Place::Spaces)]);
    assert!(!has(&mut window, Role::Button, "New document"), "one card at a time");
    // What is typed meanwhile starts no document behind the card.
    window.type_text("x");
    // Put away, it leaves the window as it was: with nothing opened, the landing card offers again.
    window.key("Escape").assert_closed(Role::Dialog, "Open");
    assert!(window.ui().landing());
    assert!(has(&mut window, Role::Button, "New document"));
}

/// A Markdown or Word file a folder keeps, which is no document yet, and the document it is listed under.
fn kept(path: &str, beside: Option<&str>, role: Grade) -> Entry {
    let name = path.rsplit('/').next().unwrap_or(path).to_owned();
    let tags = vec![Tag::Unimported { beside: beside.map(str::to_owned) }];
    Entry { name, role, tags, ..entry(EntryKind::Document, "sam", path, None, DAY) }
}

#[test]
fn a_file_that_is_no_document_yet_is_listed_under_its_document_or_offered_to_import() {
    let held = |role| {
        vec![
            document("sam", "Minutes.let", "d3", 0),
            kept("Minutes.md", Some("Minutes.let"), role),
            kept("Agenda.docx", None, role),
            kept("Minutes.docx", Some("Minutes.let"), role),
            document("sam", "Zoning.let", "d4", 3 * DAY),
        ]
    };
    let mut window = signed_in(&[HOME]);
    open(&mut window, held(Grade::Editor));
    // A file follows the document of its name, however the list is ordered.
    let by_name = ["Agenda.docx", "Minutes", "Minutes.docx", "Minutes.md", "Zoning"];
    assert_eq!(listed(&mut window, "Documents").0, by_name);
    window.click(Role::Button, "Modified");
    assert_eq!(listed(&mut window, "Documents").0, ["Minutes", "Minutes.docx", "Minutes.md", "Agenda.docx", "Zoning"]);

    // Only the file with no document is offered, and pressing a row of either does nothing.
    let said = window.find(Role::ListBoxOption, "Agenda.docx").expect("listed").description.unwrap_or_default();
    assert!(said.starts_with("Word file"), "{said}");
    for name in ["Agenda.docx", "Minutes.md"] {
        let row = window.find(Role::ListBoxOption, name).expect("listed").bounds;
        window.click_at(Point { x: row.x + 60.0, y: row.y + row.height / 2.0 });
        assert_eq!(window.ui().take_requests(), [], "{name}");
    }
    let import = button(&mut window, "Import", "As a new document").expect("offered to whoever edits there");
    window.click_at(import.center());
    let asked = Request::ImportFile { server: HOME.into(), space: "sam".into(), path: "Agenda.docx".into() };
    assert_eq!(window.ui().take_requests(), [asked]);
    window.assert_closed(Role::Dialog, "Open");

    // A reader sees the files, and is told of the one that is not imported.
    let mut window = signed_in(&[HOME]);
    window.click(Role::Button, "More").click(Role::MenuItem, "Open");
    let reads = Space { role: Grade::Reader, ..space("sam", "Sam Okafor", SpaceKind::Personal) };
    window.ui().set_listing(Browsed {
        server: HOME.into(),
        place: Place::Spaces,
        spaces: vec![reads],
        entries: vec![],
    });
    window.ui().set_listing(listing(HOME, within("sam", ""), held(Grade::Reader)));
    assert_eq!(listed(&mut window, "Documents").0, by_name);
    assert!(button(&mut window, "Import", "As a new document").is_none());
    let said = window.find(Role::ListBoxOption, "Agenda.docx").expect("listed").description.unwrap_or_default();
    assert!(said.contains("Not imported"), "{said}");
}
