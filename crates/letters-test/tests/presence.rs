//! The others who have the document open: their faces on the bar, their carets and selections on the page,
//! and what the bar says of the server that tells of them.

use letters_model::{Block, BlockKind, Document, Pos, Selection};
use letters_render::{Color, Item, Point, Rect};
use letters_test::{Found, Harness, Role};
use letters_ui::{Connection, Peer, Request, SaveState};

/// A title and three paragraphs `a`, `b` and `c`, then `more` paragraphs `p0`, `p1` and so on.
fn letter(more: usize) -> Harness {
    let mut blocks = vec![
        Block::text("title", BlockKind::Heading { level: 1 }, "Cottage renovation"),
        Block::text("a", BlockKind::Paragraph, "Harland Roofing sent a written quote for the north slope."),
        Block::text("b", BlockKind::Paragraph, "The roofer needs the scaffold up by 3 March."),
        Block::text("c", BlockKind::Paragraph, "Alice is confirming dates with the scaffolders this week."),
    ];
    let filler =
        |n: usize| Block::text(format!("p{n}").as_str(), BlockKind::Paragraph, format!("Paragraph {n} of the plan."));
    blocks.extend((0..more).map(filler));
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_author("Sam Okafor");
    window
}

/// Someone with their caret at a place in a block, or with the stretch between two places selected.
fn peer(session: u64, name: &str, colour: u8, at: Option<(&str, usize, usize)>) -> Peer {
    let selection = at.map(|(block, from, to)| Selection { anchor: Pos::new(block, from), head: Pos::new(block, to) });
    Peer { session, name: name.to_owned(), colour, selection }
}

const NAMES: [&str; 6] = ["Alice Martin", "Ben Price", "Chidi Obi", "Dana Lund", "Eli Rao", ""];
/// The six as the window names them: one who gave no name is still someone.
const SHOWN: [&str; 6] = ["Alice Martin", "Ben Price", "Chidi Obi", "Dana Lund", "Eli Rao", "Someone"];

/// The first `count` of six people, none of them with a caret in the text.
fn people(count: usize) -> Vec<Peer> {
    let person = |(n, name): (usize, &&str)| peer(n as u64 + 1, name, n as u8, None);
    NAMES[..count].iter().enumerate().map(person).collect()
}

fn page(window: &mut Harness) -> Rect {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page").bounds
}

/// The faces on the bar, by the names they answer to.
fn faces(window: &mut Harness) -> Vec<String> {
    let bar = page(window).y;
    let face =
        |found: &Found| found.role == Role::Button && found.description.as_deref() == Some("Has this document open");
    window.controls().into_iter().filter(|found| face(found) && found.bounds.y < bar).map(|found| found.name).collect()
}

/// Where a place in a block's text is on the page. The person's own caret is left at the start of the title.
fn place(window: &mut Harness, block: &str, offset: usize) -> Rect {
    window.ui().select(Selection::caret(Pos::new(block, offset)));
    window.paint();
    let at = window.ui().caret_rect().expect("the page shows a caret");
    window.ui().select(Selection::caret(Pos::new("title", 0)));
    at
}

/// The colours of the carets a frame draws at a place: each a fill as tall as the line and a caret wide.
fn carets_at(window: &mut Harness, at: Rect) -> Vec<Color> {
    let caret = |item: &Item| match item {
        Item::Fill { rect, color, .. } => {
            let stands = (rect.x - at.x).abs() < 1.5 && (rect.y - at.y).abs() < 1.0;
            (stands && rect.width <= 3.0 && (rect.height - at.height).abs() < 1.0).then_some(*color)
        }
        _ => None,
    };
    window.paint().items.iter().filter_map(caret).collect()
}

/// The name a frame has over the caret at a place, by the ground it is written on: one that lies over the
/// caret, ends on its top and is wider than the line is tall, with something written on it.
fn flag(window: &mut Harness, at: Rect) -> Option<Rect> {
    let list = window.paint();
    let written = |flag: &Rect| {
        list.items.iter().any(|item| matches!(item, Item::Glyphs { origin, .. } if flag.contains(*origin)))
    };
    let over = |item: &Item| match item {
        Item::Fill { rect, .. } => {
            let over = (rect.y + rect.height - at.y).abs() < 1.5 && rect.x <= at.x + 1.0 && rect.x + rect.width > at.x;
            (over && rect.width > at.height).then_some(*rect)
        }
        _ => None,
    };
    list.items.iter().filter_map(over).find(written)
}

fn named(window: &mut Harness, at: Rect) -> bool {
    flag(window, at).is_some()
}

/// The colour a frame lays under the text between two places on one line, when it lays one there.
fn washed(window: &mut Harness, (start, end): (Rect, Rect)) -> Option<Color> {
    let under = |item: &Item| match item {
        Item::Fill { rect, color, .. } => {
            let spans = (rect.x - start.x).abs() < 1.5 && (rect.x + rect.width - end.x).abs() < 1.5;
            (spans && (rect.y - start.y).abs() < 4.0 && rect.height > start.height / 2.0).then_some(*color)
        }
        _ => None,
    };
    window.paint().items.iter().find_map(under)
}

/// What the bar says of saving: the name of its mark, between the document's name and the tools.
fn mark(window: &mut Harness) -> String {
    let says = ["Saved", "Saving", "Offline", "Connecting", "Not saved"];
    let found =
        window.controls().into_iter().find(|found| found.role == Role::Label && says.contains(&found.name.as_str()));
    found.map(|found| found.name).unwrap_or_default()
}

#[test]
fn everyone_else_who_has_the_document_open_has_a_face_on_the_bar() {
    // How wide the window is, how many people are here, the faces the bar shows, and what it says of
    // those it only counts.
    let cases: [(f32, usize, &[&str], Option<&str>); 7] = [
        (1100.0, 0, &[], None),
        (1100.0, 1, &["Alice Martin"], None),
        (1100.0, 4, &["Alice Martin", "Ben Price", "Chidi Obi", "Dana Lund"], None),
        (1100.0, 6, &["Alice Martin", "Ben Price", "Chidi Obi"], Some("3 more")),
        // A narrow window keeps its tools: its bar has one place for whoever is here.
        (600.0, 0, &[], None),
        (600.0, 1, &["Alice Martin"], None),
        (600.0, 6, &[], Some("6 people here")),
    ];
    for (width, count, shown, counted) in cases {
        let mut window = letter(0);
        window.resize(width, 700.0);
        window.ui().set_peers(people(count));
        assert_eq!(faces(&mut window), shown, "{count} people in {width}");
        let counts = |found: &Found| found.name.ends_with(" more") || found.name.ends_with(" people here");
        let rest = window.controls().into_iter().find(|found| found.role == Role::Label && counts(found));
        assert_eq!(rest.as_ref().map(|rest| rest.name.as_str()), counted, "{count} people in {width}");
        if let Some(rest) = rest {
            // Those it counts are named to whoever asks.
            assert_eq!(rest.description, Some(SHOWN[shown.len()..count].join(", ")), "{count} people in {width}");
        }
        assert!(window.find(Role::Button, "Share").is_some(), "{count} people: the bar keeps its Share button");
    }
}

#[test]
fn the_faces_follow_who_comes_and_goes() {
    let mut window = letter(0);
    window.ui().set_peers(people(2));
    window.ui().set_peers(vec![peer(2, "Ben Price", 1, None), peer(9, "", 4, None)]);
    assert_eq!(faces(&mut window), ["Ben Price", "Someone"]);
    window.ui().set_peers(Vec::new());
    assert_eq!(faces(&mut window), [""; 0]);
}

#[test]
fn the_page_shows_where_the_others_are_each_in_their_own_colour() {
    let mut window = letter(0);
    let (alice, ben) = (place(&mut window, "c", 46), place(&mut window, "b", 10));
    let quote = (place(&mut window, "a", 23), place(&mut window, "a", 36));
    assert_eq!(carets_at(&mut window, alice), [], "nobody is here yet");
    window.ui().set_peers(vec![
        peer(1, "Alice Martin", 0, Some(("c", 46, 46))),
        peer(2, "Ben Price", 1, Some(("b", 10, 10))),
        peer(3, "Chidi Obi", 2, Some(("a", 23, 36))),
        peer(4, "Dana Lund", 250, Some(("a", 0, 0))),
    ]);
    let (hers, his) = (carets_at(&mut window, alice), carets_at(&mut window, ben));
    assert_eq!((hers.len(), his.len()), (1, 1), "one caret for each of them");
    assert_ne!(hers, his, "two people are told apart by their colours");
    // What someone selected is washed in their colour, with their caret at the end that moves.
    let wash = washed(&mut window, quote).expect("the stretch Chidi selected is marked");
    let chidi = carets_at(&mut window, quote.1);
    assert_eq!(chidi.len(), 1);
    assert_eq!((wash.r, wash.g, wash.b), (chidi[0].r, chidi[0].g, chidi[0].b));
    assert!(wash.a < chidi[0].a, "the text reads through the wash");
    assert_eq!(carets_at(&mut window, quote.0), [], "the end that stays has no caret");
    // A colour past the last the window has is one of them again.
    let dana = place(&mut window, "a", 0);
    assert_eq!(carets_at(&mut window, dana).len(), 1);
    // The others' selections are theirs: the person's own caret and document are as they were.
    assert_eq!(window.editor().selection(), Some(&Selection::caret(Pos::new("title", 0))));
    window.ui().set_peers(Vec::new());
    assert_eq!((carets_at(&mut window, alice), washed(&mut window, quote)), (Vec::new(), None));
}

#[test]
fn a_name_stands_over_a_caret_that_came_or_moved_and_goes_once_it_rests() {
    let mut window = letter(0);
    let (first, then, ben) = (place(&mut window, "c", 46), place(&mut window, "c", 20), place(&mut window, "b", 10));
    let here = |alice: usize| {
        vec![peer(1, "Alice Martin", 0, Some(("c", alice, alice))), peer(2, "Ben Price", 1, Some(("b", 10, 10)))]
    };
    window.ui().set_peers(here(46));
    window.advance(0);
    assert!(named(&mut window, first) && named(&mut window, ben), "a caret that came says whose it is");
    window.advance(1000);
    assert!(named(&mut window, first), "for longer than a glance takes");
    window.advance(4000);
    assert!(!named(&mut window, first) && !named(&mut window, ben), "a caret that rests gives the line above back");
    assert_eq!(carets_at(&mut window, first).len(), 1, "the caret itself stays");
    // The list told again as it was moves nobody.
    window.ui().set_peers(here(46));
    window.advance(0);
    assert!(!named(&mut window, first));
    // Alice moves: her name is over her caret again, and Ben's stays away.
    window.ui().set_peers(here(20));
    window.advance(0);
    assert!(named(&mut window, then) && !named(&mut window, ben));
    assert_eq!(carets_at(&mut window, first), [], "she is no longer where she was");
}

#[test]
fn the_pointer_at_a_caret_brings_its_name_out() {
    let mut window = letter(0);
    let (alice, ben) = (place(&mut window, "c", 46), place(&mut window, "b", 10));
    window
        .ui()
        .set_peers(vec![peer(1, "Alice Martin", 0, Some(("c", 46, 46))), peer(2, "Ben Price", 1, Some(("b", 10, 10)))]);
    window.advance(0).advance(5000);
    assert!(!named(&mut window, alice));
    window.hover_at(Point { x: alice.x + 1.0, y: alice.y + alice.height / 2.0 });
    assert!(named(&mut window, alice) && !named(&mut window, ben), "only the caret under the pointer");
    window.hover_at(Point { x: alice.x + 80.0, y: alice.y + alice.height / 2.0 });
    assert!(!named(&mut window, alice));
}

/// The carets of others that a frame draws on the page: each a fill two wide and as tall as a line.
fn carets(window: &mut Harness) -> Vec<Rect> {
    let area = page(window);
    let caret = |item: &Item| match item {
        Item::Fill { rect, .. } if rect.width == 2.0 && rect.height > 10.0 => {
            area.contains(Point { x: rect.x, y: rect.y }).then_some(*rect)
        }
        _ => None,
    };
    window.paint().items.iter().filter_map(caret).collect()
}

#[test]
fn a_press_on_a_face_brings_the_page_to_that_person() {
    let mut window = letter(60);
    window.ui().set_peers(vec![peer(1, "Alice Martin", 0, Some(("p55", 9, 9))), peer(2, "Ben Price", 1, None)]);
    window.advance(0).advance(5000);
    assert_eq!(carets(&mut window), [], "she is far below what shows");
    window.click(Role::Button, "Alice Martin").advance(0);
    let [hers] = carets(&mut window)[..] else { panic!("the page went to her") };
    assert!(named(&mut window, hers), "and says that it is her");
    assert_eq!(window.editor().selection(), Some(&Selection::caret(Pos::new("title", 0))), "the person's caret stays");
    // Someone with no caret in the text is nowhere to go to.
    window.click(Role::Button, "Ben Price");
    assert_eq!(carets(&mut window), [hers]);
}

#[test]
fn a_name_keeps_inside_the_page_area() {
    let mut window = letter(0);
    window.resize(420.0, 700.0);
    // The place in the first paragraph that is furthest across: the end of a full line.
    let places = (0..57).map(|offset| (offset, place(&mut window, "a", offset)));
    let (offset, end) = places.max_by(|(_, a), (_, b)| a.x.total_cmp(&b.x)).expect("a place");
    window.ui().set_peers(vec![peer(1, "Alice Martin of Harland Roofing", 0, Some(("a", offset, offset)))]);
    window.advance(0);
    let (area, name) = (page(&mut window), flag(&mut window, end).expect("her name is over her caret"));
    assert!(name.x >= area.x && name.x + name.width <= area.x + area.width, "{name:?} in {area:?}");
}

/// What stands on the bar, each with its place.
fn bar(window: &mut Harness) -> Vec<(String, Rect)> {
    let under = page(window).y;
    let controls = window.controls().into_iter().filter(|found| found.bounds.y + found.bounds.height <= under);
    controls.map(|found| (found.name, found.bounds)).collect()
}

#[test]
fn the_bar_makes_room_for_the_faces_whenever_they_come() {
    for width in (700..=1300).step_by(50) {
        // The people were here when the window opened, or came once it had shown.
        let [there, came] = [false, true].map(|late| {
            let mut window = letter(0);
            window.resize(width as f32, 700.0);
            window.ui().set_document_name("Cottage renovation");
            if late {
                window.paint();
            }
            window.ui().set_peers(people(6));
            window.paint();
            bar(&mut window)
        });
        assert_eq!(came, there, "in {width}");
        // No tool lies under a face.
        let face = |(name, _): &&(String, Rect)| name == "Alice Martin" || name == "6 people here";
        let (first, link) = (there.iter().find(face), there.iter().find(|(name, _)| name == "Link"));
        let (first, link) = (first.expect("a face").1, link.expect("a tool").1);
        assert!(link.x + link.width <= first.x, "in {width}: {link:?} under {first:?}");
    }
}

#[test]
fn someone_whose_place_the_document_no_longer_has_shows_on_the_bar_alone() {
    // A block that is gone, and a place past the end of one that is there.
    for at in [("gone", 3, 3), ("b", 400, 400), ("b", 2, 400)] {
        let mut window = letter(0);
        window.ui().set_peers(vec![peer(1, "Alice Martin", 0, Some(at))]);
        window.advance(0);
        window.paint();
        assert_eq!(faces(&mut window), ["Alice Martin"], "{at:?}");
        window.click(Role::Button, "Alice Martin").type_text("So. ");
        assert!(window.markdown().starts_with("# So. Cottage renovation"), "{at:?}: the page is the person's to write");
    }
}

#[test]
fn a_list_that_says_nothing_new_asks_for_no_frame() {
    let mut window = letter(0);
    let here = vec![peer(1, "Alice Martin", 0, Some(("c", 46, 46)))];
    window.ui().set_peers(here.clone());
    window.ui().set_connection(Some(Connection::Live));
    window.advance(0).advance(5000);
    window.paint();
    window.ui().set_peers(here);
    window.ui().set_connection(Some(Connection::Live));
    assert!(!window.ui().needs_paint());
}

#[test]
fn the_window_tells_of_the_persons_selection_while_the_page_shows() {
    let mut window = letter(0);
    window.select("written quote");
    let selection = window.editor().selection().cloned();
    assert!(selection.is_some());
    assert_eq!(window.ui().selection().cloned(), selection);
    window.click(Role::Button, "History");
    assert_eq!(window.ui().selection(), None, "an earlier version is not where the person writes");
    assert_eq!(faces(&mut window), [""; 0]);
}

#[test]
fn the_bar_says_how_the_document_stands_to_its_server() {
    let refused = || Connection::Refused("You do not have access to this document.".to_owned());
    // What the shell says of the server and of saving, and what the bar's mark says.
    let cases = [
        (None, SaveState::Saved, "Saved"),
        (None, SaveState::Saving, "Saving"),
        (Some(Connection::Live), SaveState::Saved, "Saved"),
        (Some(Connection::Live), SaveState::Saving, "Saving"),
        (Some(Connection::Live), SaveState::Offline, "Offline"),
        (Some(Connection::Offline), SaveState::Saved, "Offline"),
        (Some(Connection::Offline), SaveState::Saving, "Offline"),
        (Some(Connection::Connecting), SaveState::Saved, "Connecting"),
        (Some(refused()), SaveState::Saved, "Offline"),
        // A write to this device that failed is told whatever the server does.
        (Some(Connection::Offline), SaveState::Failed, "Not saved"),
    ];
    for (connection, save, says) in cases {
        let mut window = letter(0);
        window.ui().set_save_state(save);
        window.ui().set_connection(connection.clone());
        assert_eq!(mark(&mut window), says, "{connection:?}, {save:?}");
    }
}

#[test]
fn a_session_the_server_ended_says_why_and_offers_another_try() {
    let why = "You do not have access to this document.";
    let mut window = letter(0);
    let before = page(&mut window);
    assert!(window.find(Role::Alert, why).is_none());
    window.ui().set_connection(Some(Connection::Refused(why.to_owned())));
    let bar = window.find(Role::Alert, why).expect("a bar says why").bounds;
    let after = page(&mut window);
    assert!(after.y >= bar.y + bar.height && after.height < before.height, "the bar takes its room from the page");
    window.click(Role::Button, "Try again");
    assert_eq!(window.ui().take_requests(), [Request::Connect]);
    // The end of a long document still comes into what the window shows.
    let mut long = letter(60);
    long.resize(900.0, 500.0);
    long.ui().set_connection(Some(Connection::Refused(why.to_owned())));
    long.click_at(Point { x: 450.0, y: 300.0 }).key("Mod+End");
    long.paint();
    let caret = long.ui().caret_rect().expect("a caret at the end");
    assert!(caret.y + caret.height <= 500.0, "{caret:?}");
    // The page under it is still the person's to write.
    window.type_text("So. ");
    assert!(window.markdown().starts_with("# So. Cottage renovation"));
    // The bar stays until the shell says how the next try went.
    assert!(window.find(Role::Alert, why).is_some());
    for next in [Some(Connection::Connecting), Some(Connection::Live), None] {
        window.ui().set_connection(Some(Connection::Refused(why.to_owned())));
        window.ui().set_connection(next.clone());
        assert!(window.find(Role::Alert, why).is_none(), "{next:?}");
        assert!(window.find(Role::Button, "Try again").is_none(), "{next:?}");
    }
}
