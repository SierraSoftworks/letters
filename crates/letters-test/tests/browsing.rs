//! Opening a document a server keeps: the card that lists a server's places and what they hold, and what
//! the window asks of the shell.

use letters_api::{Entry, EntryKind, Role as Grade, Space, SpaceKind, Tag};
use letters_test::{Harness, Role};
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
    let places = ["Family", "Sam Okafor", "Shared with me", "Recent"].map(str::to_owned);
    assert_eq!(listed(&mut window, "Places"), (places.to_vec(), vec!["Sam Okafor".to_owned()]));

    window.ui().set_listing(listing(HOME, within("sam", ""), own()));
    assert!(!has(&mut window, Role::Status, ASKING));
    assert_eq!(listed(&mut window, "Documents").0, ["House", "Letters sent", "Roof plan", "Tax 2026"]);
    assert_eq!(window.ui().take_requests(), [], "the answer asks for nothing");
    // A document says when it was last written, by the person's clock.
    let about = |window: &mut Harness, name: &str| {
        window.find(Role::ListBoxOption, name).and_then(|found| found.description).unwrap_or_default()
    };
    assert_eq!(about(&mut window, "Roof plan"), "Document, Today");
    assert_eq!(about(&mut window, "Tax 2026"), "Document, 6 Nov");
    assert_eq!(about(&mut window, "House"), "Folder");

    // Half past eleven last night by UTC was today an hour to the east.
    let mut window = signed_in(&[HOME]);
    window.ui().set_utc_offset(60);
    open(&mut window, vec![document("sam", "Late.let", "d7", 12 * 3_600 + 1_800)]);
    assert_eq!(about(&mut window, "Late"), "Document, Today");
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
    let copy = Entry { tags: vec![copy], ..document("sam", "Minutes (conflict).let", "d4", 0) };
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
    // Back from the documents, past what stands over them, to the places.
    for _ in 0..3 {
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

#[test]
fn a_document_is_started_in_the_folder_that_is_looked_at() {
    // The folder that is gone into, and the path the document is started at.
    for (folder, path) in [(None, ""), (Some("House"), "House")] {
        let mut window = signed_in(&[HOME]);
        open(&mut window, own());
        if let Some(folder) = folder {
            window.click(Role::ListBoxOption, folder);
            window.ui().take_requests();
        }
        let new = window.find(Role::Button, "New document").expect("offered");
        assert_eq!(new.description.as_deref(), Some(format!("In {}", folder.unwrap_or("Sam Okafor")).as_str()));
        window.click(Role::Button, "New document");
        let asked = Request::NewAt { server: HOME.into(), space: "sam".into(), path: path.into() };
        assert_eq!(window.ui().take_requests(), [asked], "{folder:?}");
        window.assert_closed(Role::Dialog, "Open");
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
        assert_eq!(has(&mut window, Role::Button, "New document"), offered, "{role:?} in {place}");
    }
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
    assert_eq!(listed(&mut window, "Places").0, ["Shared with me", "Recent"], "its spaces are not known yet");

    // An answer that comes late, from the server that was left, is let go.
    window.ui().set_listing(listing(HOME, within("sam", ""), own()));
    assert!(listed(&mut window, "Documents").0.is_empty());
    window.ui().set_listing(Browsed {
        server: HOME.into(),
        place: Place::Spaces,
        spaces: spaces(),
        entries: Vec::new(),
    });
    assert_eq!(listed(&mut window, "Places").0, ["Shared with me", "Recent"]);
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
        (vec!["Shared with me".into(), "Recent".into()], vec!["Shared with me".into()])
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
    assert_eq!(listed(&mut window, "Places").0, ["Shared with me", "Recent"]);
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
