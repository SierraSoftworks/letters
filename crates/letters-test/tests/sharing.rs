//! The Share dialog: who a document on a server is shared with, the links made for it, and what the window
//! asks of the shell.

use letters_api::{
    Access as Reach, Action, Allows, CreatedLink, Grant, Grantee, Link, NewLink, Role as Grade, Shares, Via, Who,
};
use letters_render::cpu::{CpuRenderer, Frame};
use letters_render::{Color, Item, Rect};
use letters_test::{Found, Harness, Role, WINDOW};
use letters_ui::{Access, Connection, Request, Shared};

/// Noon on 15 November 2026, in seconds since 1970.
const NOON: u64 = 1_794_744_000;
const DAY: u64 = 86_400;
const REFUSAL: &str = "Only the owner says who this document is shared with.";
/// A name that no row has the room for.
const LONG: &str = "Alexandria Featherstonehaugh-Cholmondeley of the Harland Roofing and Guttering Company";

/// A plan that a server keeps, in a window whose clock says noon.
fn plan() -> Harness {
    let mut window = Harness::doc("# Roof plan\n\nThe roofer needs the scaffold up by 3 March.");
    window.platform.wall = Some(NOON);
    window.ui().set_connection(Some(Connection::Live));
    window
}

fn person(id: &str, name: &str, role: Grade) -> Grant {
    Grant { to: Grantee::Person { id: id.into(), name: name.into() }, role }
}

fn group(name: &str, role: Grade) -> Grant {
    Grant { to: Grantee::Group { name: name.into() }, role }
}

/// A link made on 4 November for a reader, with no end, no passphrase and nothing to take away.
fn link(id: &str, label: &str) -> Link {
    let (expires, allows) = (None, Allows::default());
    Link {
        id: id.into(),
        label: label.into(),
        role: Grade::Reader,
        expires,
        passphrase: false,
        allows,
        created: NOON - 11 * DAY,
        revoked: None,
    }
}

/// Who the plan is shared with: its owner, an editor and a group that comments.
fn people() -> Vec<Grant> {
    vec![
        person("p1", "Alice Martin", Grade::Owner),
        person("p2", "Ben Price", Grade::Editor),
        group("Family", Grade::Commenter),
    ]
}

/// The plan's shares as its owner or as someone else is told them, on a server that makes links.
fn shared(owner: bool, links: Vec<Link>) -> Shared {
    Shared { shares: Shares { people: people(), links }, owner, links: true }
}

/// Opens the dialog and answers what it asks, as a shell does.
fn open(window: &mut Harness, shared: Shared) {
    window.click(Role::Button, "Share");
    window.ui().take_requests();
    window.ui().set_shares(shared);
}

/// The rows of the people, as a screen reader says them.
fn rows(window: &mut Harness) -> Vec<String> {
    window.controls().into_iter().filter(|found| found.role == Role::ListItem).map(|found| found.name).collect()
}

/// A button by its name and what it is about, as each link has a Revoke.
fn button(window: &mut Harness, name: &str, about: &str) -> Option<Found> {
    let is =
        |found: &Found| found.role == Role::Button && found.name == name && found.description.as_deref() == Some(about);
    window.controls().into_iter().find(is)
}

/// The dialog's place in the window, and how far right the letters written in it reach: where the last
/// letter of any run that starts there stands.
fn written(window: &mut Harness) -> (Rect, f32) {
    let dialog = window.find(Role::Dialog, "Share").expect("the dialog").bounds;
    let ends = window.paint().items.into_iter().filter_map(|item| match item {
        Item::Glyphs { origin, glyphs, .. } if dialog.contains(origin) => glyphs.last().map(|last| origin.x + last.x),
        _ => None,
    });
    (dialog, ends.fold(0.0, f32::max))
}

/// Something a test does to the window.
type Does = fn(&mut Harness);

const HOUR: u64 = 3_600;
const ASKING: &str = "Asking the server who has reached this document.";

fn by(name: &str) -> Who {
    Who::Person { id: name.to_lowercase(), name: name.into() }
}

fn guest(label: &str) -> Who {
    Who::Link { id: label.to_lowercase(), label: label.into() }
}

/// Something done to the plan through a share, so long before noon.
fn reach(ago: u64, who: Who, action: Action, role: Grade) -> Reach {
    let (address, browser) = ("203.0.113.7".into(), "Firefox on Linux".into());
    Reach { time: NOON - ago, who, link: None, action, role, refused: false, lasted: None, address, browser }
}

/// The plan's log, newest first: two entries today, one yesterday, two on Thursday and one in October.
fn log() -> Vec<Reach> {
    vec![
        reach(HOUR, by("Ben Price"), Action::Edit, Grade::Editor),
        Reach { refused: true, ..reach(3 * HOUR, guest("Builder"), Action::Download, Grade::Reader) },
        reach(DAY + 2 * HOUR, by("Carla Diaz"), Action::Comment, Grade::Commenter),
        reach(3 * DAY, guest("Builder"), Action::Open, Grade::Reader),
        reach(3 * DAY + HOUR, guest("Builder"), Action::Print, Grade::Reader),
        reach(20 * DAY, by("Ben Price"), Action::Download, Grade::Editor),
    ]
}

/// Opens the dialog on its Activity tab and answers what it asks, as a shell does.
fn activity(window: &mut Harness, log: Vec<Reach>) {
    open(window, shared(true, Vec::new()));
    window.click(Role::Tab, "Activity");
    window.ui().take_requests();
    window.ui().set_access_log(log);
}

/// The names of what the dialog lists under a role, in order.
fn listed(window: &mut Harness, role: Role) -> Vec<String> {
    window.controls().into_iter().filter(|found| found.role == role).map(|found| found.name).collect()
}

/// Which of the dialog's tabs is shown.
fn shown(window: &mut Harness) -> Vec<String> {
    let tabs = window.controls().into_iter().filter(|found| found.role == Role::Tab && found.selected == Some(true));
    tabs.map(|found| found.name).collect()
}

fn has(window: &mut Harness, role: Role, name: &str) -> bool {
    window.find(role, name).is_some()
}

/// Presses a button of the dialog from the keyboard, as one far down a list that scrolls is reached. One
/// that is not there, or is not to be pressed, is passed by.
fn press(window: &mut Harness, name: &str) {
    for _ in 0..6 {
        if window.focused().is_some_and(|found| found.name == name) {
            window.key("Enter");
            return;
        }
        window.key("Tab");
    }
}

#[test]
fn the_dialog_asks_the_server_and_lists_who_the_document_is_shared_with() {
    let mut window = plan();
    window.click(Role::Button, "Share").assert_open(Role::Dialog, "Share");
    assert_eq!(window.ui().take_requests(), [Request::Shares]);
    assert!(has(&mut window, Role::Label, "Share \u{201c}Roof plan\u{201d}"), "the dialog names the document");
    assert!(rows(&mut window).is_empty(), "nothing is listed before the server has answered");

    window.ui().set_shares(shared(true, vec![link("l1", "Builder")]));
    assert_eq!(rows(&mut window), ["Alice Martin, Owner", "Ben Price, Editor", "Family, Commenter"]);
    assert!(has(&mut window, Role::Group, "Link for Builder"), "the link is listed");
    assert_eq!(window.ui().take_requests(), [], "the answer asks for nothing");

    // Whatever puts the dialog away, the next opening asks again and shows what was last said meanwhile.
    for close in ["Done", "Close"] {
        window.click(Role::Button, close).assert_closed(Role::Dialog, "Share");
        window.click(Role::Button, "Share").assert_open(Role::Dialog, "Share");
        assert_eq!(window.ui().take_requests(), [Request::Shares], "{close}");
        assert_eq!(rows(&mut window).len(), 3, "{close}");
    }
    window.key("Escape").assert_closed(Role::Dialog, "Share");
}

#[test]
fn the_owner_gives_someone_another_role_or_takes_them_away() {
    let with = |at: usize, role: Option<Grade>| {
        let mut people = people();
        match role {
            Some(role) => people[at].role = role,
            None => _ = people.remove(at),
        }
        people
    };
    // Whose role is pressed, what is chosen there, and the people the shell is then asked to keep.
    let cases = [
        ("Ben Price", Role::MenuItemCheckBox, "Commenter Reads and comments", Some(with(1, Some(Grade::Commenter)))),
        ("Family", Role::MenuItemCheckBox, "Editor Reads, comments and edits", Some(with(2, Some(Grade::Editor)))),
        ("Ben Price", Role::MenuItem, "Remove", Some(with(1, None))),
        // The role they have already changes nothing.
        ("Ben Price", Role::MenuItemCheckBox, "Editor Reads, comments and edits", None),
    ];
    for (name, role, choice, asked) in cases {
        let mut window = plan();
        open(&mut window, shared(true, Vec::new()));
        window.click(Role::Button, &format!("Role of {name}")).click(role, choice);
        let expected: Vec<Request> = asked.clone().into_iter().map(Request::SharePeople).collect();
        assert_eq!(window.ui().take_requests(), expected, "{name}: {choice}");
        // The dialog shows the change without waiting for the server.
        let said = |grant: &Grant| match &grant.to {
            Grantee::Person { name, .. } | Grantee::Group { name } => format!("{name}, {:?}", grant.role),
        };
        let shown: Vec<String> = asked.unwrap_or_else(people).iter().map(said).collect();
        assert_eq!(rows(&mut window), shown, "{name}: {choice}");
    }
}

#[test]
fn the_controls_of_the_roles_line_up_whatever_role_each_says() {
    let mut window = plan();
    open(&mut window, shared(true, vec![]));
    let wide = |window: &mut Harness, name: &str| {
        window.find(Role::Button, &format!("Role of {name}")).expect("a control for the role").bounds.width
    };
    let (editor, commenter) = (wide(&mut window, "Ben Price"), wide(&mut window, "Family"));
    assert!((editor - commenter).abs() < 0.5, "an editor's is {editor} wide and a commenter's {commenter}");
}

#[test]
fn a_name_too_long_for_its_row_is_cut_short_inside_the_dialog() {
    let mut window = plan();
    let friends = format!("{LONG} and friends");
    let people = vec![
        person("p1", "Alice Martin", Grade::Owner),
        person("p2", LONG, Grade::Editor),
        group(&friends, Grade::Reader),
    ];
    let shares = Shares { people, links: vec![link("l1", LONG)] };
    open(&mut window, Shared { shares, owner: true, links: true });
    let (dialog, ends) = written(&mut window);
    let edge = dialog.x + dialog.width;
    assert!(ends < edge, "the names and the link's label end at {ends}, in {dialog:?}");
    // What stands at the end of a row keeps its place.
    for name in [LONG, friends.as_str()] {
        let role = window.find(Role::Button, &format!("Role of {name}")).expect("a control for the role").bounds;
        assert!(role.x + role.width < edge, "{name}: {role:?} in {dialog:?}");
    }
    let revoke = button(&mut window, "Revoke", &format!("The link for {LONG}")).expect("a way to revoke it").bounds;
    assert!(revoke.x + revoke.width < edge, "{revoke:?} in {dialog:?}");

    // So it is in the log, of someone and of the link they came by.
    window.click(Role::Tab, "Activity");
    window.ui().take_requests();
    let via = Some(Via { id: "l1".into(), label: LONG.into() });
    window.ui().set_access_log(vec![Reach { link: via, ..reach(HOUR, by(LONG), Action::Edit, Grade::Editor) }]);
    let (dialog, ends) = written(&mut window);
    assert!(ends < dialog.x + dialog.width, "the log's letters end at {ends}, in {dialog:?}");
}

#[test]
fn a_group_is_added_by_its_name() {
    let added = |name: &str, role: Grade| {
        let mut people = people();
        people.push(group(name, role));
        people
    };
    let mut again = people();
    again[2].role = Grade::Editor;
    // What is typed, how it is added, and the people the shell is then asked to keep.
    let cases: [(&str, Does, Option<Vec<Grant>>); 6] = [
        ("Finance", |window| _ = window.key("Enter"), Some(added("Finance", Grade::Reader))),
        ("Finance", |window| _ = window.click(Role::Button, "Add reader"), Some(added("Finance", Grade::Reader))),
        (
            " Finance ",
            |window| {
                _ = window
                    .click(Role::Button, "Add with another role")
                    .click(Role::MenuItem, "Add commenter Reads and comments")
            },
            Some(added("Finance", Grade::Commenter)),
        ),
        // A group that is there already is given the role, and not listed twice.
        (
            "Family",
            |window| {
                _ = window
                    .click(Role::Button, "Add with another role")
                    .click(Role::MenuItem, "Add editor Reads, comments and edits")
            },
            Some(again),
        ),
        ("", |window| _ = window.key("Enter"), None),
        ("  ", |window| _ = window.click(Role::Button, "Add reader"), None),
    ];
    for (typed, add, asked) in cases {
        let mut window = plan();
        open(&mut window, shared(true, Vec::new()));
        window.click(Role::TextInput, "Add a group").type_text(typed);
        add(&mut window);
        let expected: Vec<Request> = asked.clone().into_iter().map(Request::SharePeople).collect();
        assert_eq!(window.ui().take_requests(), expected, "{typed:?}");
        if let Some(people) = asked {
            assert_eq!(rows(&mut window).len(), people.len(), "{typed:?}: listed at once");
            let field = window.find(Role::TextInput, "Add a group").expect("the field stays");
            assert_eq!(field.value.unwrap_or_default(), "", "{typed:?}: the field is ready for another");
        }
    }
}

/// The lines drawn round something in the window, with how wide and how round each is.
fn lines(window: &mut Harness) -> Vec<(Rect, f32, f32)> {
    let drawn = window.paint().items.into_iter().filter_map(|item| match item {
        Item::Stroke { rect, width, radius, .. } => Some((rect, width, radius)),
        _ => None,
    });
    drawn.collect()
}

/// The window as its pixels, one to a point.
fn shot(window: &mut Harness) -> Frame {
    let (list, images) = (window.paint(), window.ui().images().clone());
    let mut frame = Frame::new(WINDOW.width as u32, WINDOW.height as u32);
    CpuRenderer::new().render(&list, &images, 1.0, Color::rgb(255, 255, 255), &mut frame);
    frame
}

#[test]
fn a_group_is_added_from_one_control_in_one_frame() {
    let mut window = plan();
    open(&mut window, shared(true, Vec::new()));
    let part = |window: &mut Harness, role: Role, name: &str| window.find(role, name).expect("a part of it").bounds;
    let field = part(&mut window, Role::TextInput, "Add a group");
    let add = part(&mut window, Role::Button, "Add reader");
    let others = part(&mut window, Role::Button, "Add with another role");
    // Nothing stands between the parts but a line, and they are as tall as each other.
    for (left, right) in [(field, add), (add, others)] {
        let line = Rect::new(left.x + left.width, left.y, 1.0, left.height);
        let drawn =
            window.paint().items.into_iter().any(|item| matches!(item, Item::Fill { rect, .. } if rect == line));
        assert!(drawn && right.x == line.x + line.width, "{left:?} then {right:?}");
        assert_eq!((left.y, left.height), (right.y, right.height));
    }
    // One line runs round them all, close about them, and the control is as tall as any other.
    let touches = |rect: &Rect| rect.y >= field.y - 2.0 && rect.y + rect.height <= field.y + field.height + 2.0;
    let around: Vec<(Rect, f32, f32)> = lines(&mut window).into_iter().filter(|(rect, ..)| touches(rect)).collect();
    let [(frame, _, corner)] = around[..] else { panic!("one frame: {around:?}") };
    // The parts fill the frame, which is as wide as the people listed under it.
    assert_eq!((field.x - frame.x, frame.x + frame.width - others.x - others.width), (1.0, 1.0), "{frame:?}");
    let people = part(&mut window, Role::List, "Shared with");
    assert_eq!((frame.x, frame.width), (people.x, people.width));
    assert!((frame.height - part(&mut window, Role::Button, "Done").height).abs() < 0.5, "{frame:?}");

    assert!(corner > 0.0, "the frame is round");

    // A part is round only at the corners it has of the frame, and square where it is joined to the next.
    // The field's ring lies on the frame, and runs square down the line between the field and the button.
    window.click(Role::TextInput, "Add a group").type_text("Finance");
    let lit = shot(&mut window);
    let at = |x: f32, y: f32| lit.pixel(x as u32, y as u32);
    let (middle, foot) = (frame.y + frame.height / 2.0, frame.y + frame.height - 1.0);
    let ring = at(frame.x, middle);
    assert_ne!(ring, at(frame.x - 2.0, middle), "the ring shows on the frame's line");
    let line = field.x + field.width;
    assert_eq!([at(line, frame.y), at(line, middle), at(line, foot)], [ring; 3], "square down the line");
    assert!(at(frame.x, frame.y) != ring && at(frame.x, foot) != ring, "and round with the frame");
    // A button's ring keeps inside the button, and is round only where the button has the frame's corners.
    for (button, ends) in [(add, false), (others, true)] {
        window.key("Tab");
        let lit = shot(&mut window);
        let at = |x: f32, y: f32| lit.pixel(x as u32, y as u32);
        let (left, top) = (button.x + 1.0, button.y + 1.0);
        let (right, bottom) = (button.x + button.width - 2.0, button.y + button.height - 2.0);
        assert!(at(left, middle) == ring && at(button.x - 1.0, middle) != ring, "inside {button:?}");
        assert_eq!([at(left, top), at(left, bottom)], [ring; 2], "square where it is joined: {button:?}");
        assert_eq!([at(right, top) == ring, at(right, bottom) == ring], [!ends; 2], "{button:?}");
    }
    // So is what shows a button is under the pointer.
    window.hover_at(letters_render::Point { x: others.x + others.width / 2.0, y: middle });
    let lit = shot(&mut window);
    let at = |x: f32, y: f32| lit.pixel(x as u32, y as u32);
    let (right, under) = (others.x + others.width - 1.0, at(others.x + 3.0, others.y + 3.0));
    assert_ne!(under, at(add.x + 3.0, add.y + 3.0), "the button under the pointer shows it");
    assert_eq!([at(others.x, others.y), at(others.x, foot - 1.0)], [under; 2], "square beside the line");
    assert!(at(right, others.y) != under && at(right, foot - 1.0) != under, "round with the frame");
}

#[test]
fn the_band_is_set_apart_from_what_is_over_it_in_the_light_and_in_the_dark() {
    // Whether the eye tells two colours apart.
    let apart = |a: Color, b: Color| [(a.r, b.r), (a.g, b.g), (a.b, b.b)].into_iter().any(|(a, b)| a.abs_diff(b) >= 6);
    for dark in [false, true] {
        let mut window = plan();
        window.ui().set_dark(dark);
        open(&mut window, shared(true, Vec::new()));
        let dialog = window.find(Role::Dialog, "Share").expect("the dialog").bounds;
        let done = window.find(Role::Button, "Done").expect("the button").bounds;
        // The colours met going up from beside the button to over the band, and how many pixels of each. It
        // is looked at near the dialog's edge, where nothing else is drawn.
        let seen = shot(&mut window);
        let mut met: Vec<(Color, usize)> = Vec::new();
        for y in (done.y as u32 - 24..done.y as u32).rev() {
            let colour = seen.pixel(dialog.x as u32 + 8, y).expect("in the window");
            match met.last_mut() {
                Some((last, rows)) if *last == colour => *rows += 1,
                _ => met.push((colour, 1)),
            }
        }
        match (dark, &met[..]) {
            // In the light the band's tint is told from the surface, and nothing is drawn between them.
            (false, [(band, _), (surface, _)]) => assert!(apart(*band, *surface), "{met:?}"),
            // In the dark the tint is all but the surface's own, and a line one pixel thick is drawn over it.
            (true, [(band, _), (line, 1), (surface, _)]) => {
                assert!(apart(*line, *band) && apart(*line, *surface), "{met:?}")
            }
            _ => panic!("dark: {dark}: {met:?}"),
        }
    }
}

#[test]
fn what_ends_the_dialog_stands_on_a_band_across_its_foot() {
    // The band a button stands on, with room round it: it runs from side to side of the dialog, inside its
    // line, and down to its foot.
    let band = |window: &mut Harness, name: &str| {
        let dialog = window.find(Role::Dialog, "Share").expect("the dialog").bounds;
        let button = window.find(Role::Button, name).expect("the button").bounds;
        let (right, foot) = (dialog.x + dialog.width, dialog.y + dialog.height);
        window.paint().items.into_iter().find_map(|item| match item {
            Item::Fill { rect, .. } if rect.height < dialog.height / 2.0 => {
                let holds = rect.y < button.y && rect.y + rect.height > button.y + button.height;
                let across = rect.x > dialog.x && rect.x + rect.width < right && rect.width >= dialog.width - 2.0;
                let down = (rect.y + rect.height - foot).abs() <= 1.0;
                (holds && across && down && rect.x + rect.width > button.x + button.width).then_some(rect)
            }
            _ => None,
        })
    };
    let mut window = plan();
    open(&mut window, shared(true, vec![link("l1", "Builder")]));
    let under = band(&mut window, "Done").expect("Done stands on a band");
    // The card's round corners cut the band's.
    let cut = window.paint().items.into_iter().any(|item| match item {
        Item::PushClip { rect, radius } => {
            radius > 0.0 && rect.contains(letters_render::Point { x: under.x, y: under.y })
        }
        _ => false,
    });
    assert!(cut);
    // What ends a link's form stands there too.
    window.click(Role::Button, "New link");
    assert!(band(&mut window, "Make link").is_some());
}

#[test]
fn only_the_owner_changes_who_reaches_the_document_and_the_owner_stays() {
    let links = || vec![link("l1", "Builder")];
    // What changes the shares, by role and name, as the owner has it.
    let changes = [
        (Role::TextInput, "Add a group"),
        (Role::Button, "Add reader"),
        (Role::Button, "Role of Ben Price"),
        (Role::Button, "Role of Family"),
        (Role::Button, "New link"),
        (Role::Button, "Revoke"),
    ];
    for owner in [true, false] {
        let mut window = plan();
        open(&mut window, shared(owner, links()));
        for (role, name) in changes {
            assert_eq!(has(&mut window, role, name), owner, "owner {owner}: {name}");
        }
        // Everyone reads the list, and nobody changes the owner's row.
        assert_eq!(rows(&mut window).len(), 3, "owner {owner}");
        assert!(has(&mut window, Role::Group, "Link for Builder"), "owner {owner}");
        assert!(!has(&mut window, Role::Button, "Role of Alice Martin"), "owner {owner}");
    }

    // A server that makes no links offers none to make, and still lists those it has.
    let mut window = plan();
    open(&mut window, Shared { links: false, ..shared(true, links()) });
    assert!(!has(&mut window, Role::Button, "New link"));
    assert!(has(&mut window, Role::Button, "Revoke"));
}

#[test]
fn a_link_is_made_with_the_limits_the_form_sets() {
    let new = |role, days: Option<u64>, passphrase: Option<&str>, allows| NewLink {
        label: "Accountant".into(),
        role,
        expires: days.map(|days| NOON + days * DAY),
        passphrase: passphrase.map(str::to_owned),
        allows,
    };
    let takes = |download, copy, print| Allows { download, copy, print };
    // What is set on the form after the link is said who it is for, and the link that is asked for.
    let cases: [(&str, Does, NewLink); 4] = [
        ("as the form opens", |_| {}, new(Grade::Reader, Some(7), None, Allows::default())),
        (
            "a commenter for a month who may download",
            |window| {
                window.click(Role::RadioButton, "Commenter").click(Role::RadioButton, "30 days");
                window.click(Role::Button, "Download");
            },
            new(Grade::Commenter, Some(30), None, takes(true, false, false)),
        ),
        (
            "an editor for ever with a passphrase",
            |window| {
                window.click(Role::RadioButton, "Editor").click(Role::RadioButton, "No expiry");
                window.click(Role::TextInput, "Passphrase").type_text("north slope");
            },
            new(Grade::Editor, None, Some("north slope"), Allows::default()),
        ),
        (
            "what was allowed and then withheld again",
            |window| {
                window.click(Role::Button, "Print").click(Role::Button, "Download").click(Role::Button, "Download");
            },
            new(Grade::Reader, Some(7), None, takes(false, false, true)),
        ),
    ];
    for (case, set, asked) in cases {
        let mut window = plan();
        open(&mut window, shared(true, Vec::new()));
        window.click(Role::Button, "New link");
        let focused = window.focused().expect("the form takes the keyboard");
        assert_eq!((focused.role, focused.name.as_str()), (Role::TextInput, "Who the link is for"), "{case}");
        // A link that is for nobody is not made, by the button or from the field.
        assert!(window.find(Role::Button, "Make link").expect("the form's button").disabled, "{case}");
        window.key("Enter").type_text("Accountant ");
        set(&mut window);
        window.click(Role::Button, "Make link");
        assert_eq!(window.ui().take_requests(), [Request::MakeLink(asked)], "{case}");
        // The server is asked once, however long it takes and whatever is pressed meanwhile.
        assert!(window.find(Role::Button, "Make link").expect("the form's button").disabled, "{case}");
        window.click(Role::TextInput, "Who the link is for").key("Enter");
        assert_eq!(window.ui().take_requests(), [], "{case}");
    }

    // The way back makes nothing.
    let mut window = plan();
    open(&mut window, shared(true, Vec::new()));
    window.click(Role::Button, "New link").type_text("Accountant").click(Role::Button, "Cancel");
    assert_eq!(window.ui().take_requests(), []);
    assert_eq!(rows(&mut window).len(), 3, "the people are listed again");
}

#[test]
fn the_address_of_a_link_made_here_is_copied_while_the_window_has_it() {
    let mut window = plan();
    open(&mut window, shared(true, vec![link("l1", "Builder")]));
    assert!(!has(&mut window, Role::Button, "Copy link"), "the address of an older link is not kept anywhere");

    window.click(Role::Button, "New link").type_text("Accountant").key("Enter");
    window.ui().take_requests();
    let url = "https://letters.example.org/l/3f9c";
    window.ui().link_made(CreatedLink { link: link("l2", "Accountant"), url: url.into() });
    assert!(has(&mut window, Role::Group, "Link for Accountant"), "the new link is listed");
    assert!(has(&mut window, Role::Group, "Link for Builder"), "beside the one that was there");
    assert!(!has(&mut window, Role::Button, "Make link"), "the form is done");

    window.click(Role::Button, "Copy link");
    assert_eq!(window.platform.clipboard.as_ref().map(|clip| clip.text.as_str()), Some(url));
    assert!(has(&mut window, Role::Button, "Copied"), "the button says that it is there");
    assert_eq!(window.ui().take_requests(), [], "the address is the window's to hand over");

    // The server's next word of the shares does not take the address away.
    window.ui().set_shares(shared(true, vec![link("l2", "Accountant"), link("l1", "Builder")]));
    window.click(Role::Button, "Done").click(Role::Button, "Share");
    assert!(button(&mut window, "Copy link", "The link for Accountant").is_some());
    assert!(button(&mut window, "Copy link", "The link for Builder").is_none());
}

#[test]
fn a_link_says_its_limits_and_goes_when_it_is_revoked() {
    let takes = |download, copy, print| Allows { download, copy, print };
    // A link's limits, and how its card says each.
    let cases = [
        (link("l1", "Builder"), ["Reader", "No expiry", "No download, copy or print", "No passphrase"]),
        (
            Link { role: Grade::Editor, expires: Some(NOON + 12 * DAY), passphrase: true, ..link("l1", "Builder") },
            ["Editor", "Expires in 12 days", "No download, copy or print", "Passphrase"],
        ),
        (
            Link { allows: takes(true, false, true), expires: Some(NOON - DAY), ..link("l1", "Builder") },
            ["Reader", "Expired 14 Nov", "Download and print allowed", "No passphrase"],
        ),
    ];
    for (link, limits) in cases {
        let mut window = plan();
        open(&mut window, shared(false, vec![link.clone()]));
        assert!(has(&mut window, Role::Label, "Made 4 Nov"), "{link:?}");
        for limit in limits {
            assert!(has(&mut window, Role::Label, limit), "{limit}: {link:?}");
        }
    }

    // A link the owner revokes is gone at once, and one revoked before is not listed.
    let revoked = Link { revoked: Some(NOON - DAY), ..link("l3", "Plumber") };
    let mut window = plan();
    open(&mut window, shared(true, vec![link("l1", "Builder"), link("l2", "Accountant"), revoked]));
    assert!(!has(&mut window, Role::Group, "Link for Plumber"));
    let revoke = button(&mut window, "Revoke", "The link for Accountant").expect("each link has its own");
    window.click_at(revoke.center());
    assert_eq!(window.ui().take_requests(), [Request::RevokeLink("l2".into())]);
    assert!(!has(&mut window, Role::Group, "Link for Accountant"));
    assert!(has(&mut window, Role::Group, "Link for Builder"), "the other link stays");
}

#[test]
fn what_the_server_refuses_is_said_and_the_shares_are_asked_for_again() {
    // What was asked before the server refused, and whether the dialog showed it as done already.
    let cases: [(&str, Does, bool); 4] = [
        ("the shares", |_| {}, false),
        (
            "someone taken away",
            |window| _ = window.click(Role::Button, "Role of Ben Price").click(Role::MenuItem, "Remove"),
            true,
        ),
        ("a link revoked", |window| _ = window.click(Role::Button, "Revoke"), true),
        (
            "a link made",
            |window| _ = window.click(Role::Button, "New link").type_text("Accountant").key("Enter"),
            false,
        ),
    ];
    for (case, ask, shown) in cases {
        let mut window = plan();
        open(&mut window, shared(true, vec![link("l1", "Builder")]));
        ask(&mut window);
        window.ui().take_requests();
        window.ui().refuse_share(REFUSAL);
        window.assert_open(Role::Alert, REFUSAL).assert_open(Role::Dialog, "Share");
        // What the dialog showed may not be so. Nothing else is asked again, or a server out of reach
        // would be asked for ever.
        let again: &[Request] = if shown { &[Request::Shares] } else { &[] };
        assert_eq!(window.ui().take_requests(), again, "{case}");
        window.ui().refuse_share(REFUSAL);
        assert_eq!(window.ui().take_requests(), [], "{case}: a refusal of that is only said");

        window.ui().set_shares(shared(true, vec![link("l1", "Builder")]));
        if case == "a link made" {
            assert!(!window.find(Role::Button, "Make link").expect("the form stays").disabled, "it can be tried again");
            window.click(Role::Button, "Cancel");
        }
        assert_eq!(rows(&mut window).len(), 3, "{case}: the server's word stands");
        assert!(has(&mut window, Role::Group, "Link for Builder"), "{case}");
        // The sentence goes with the next thing that is done.
        window.click(Role::TextInput, "Add a group").type_text("Finance").key("Enter");
        window.assert_closed(Role::Alert, REFUSAL);
    }

    // A server that refuses the first question has nothing listed, and is not asked again.
    let mut window = plan();
    window.click(Role::Button, "Share");
    window.ui().take_requests();
    window.ui().refuse_share(REFUSAL);
    window.assert_open(Role::Alert, REFUSAL);
    assert_eq!(window.ui().take_requests(), []);
    assert!(rows(&mut window).is_empty());
}

#[test]
fn a_document_kept_on_this_device_says_that_sharing_needs_a_server() {
    let mut window = plan();
    window.ui().set_connection(None);
    window.click(Role::Button, "Share").assert_open(Role::Dialog, "Share");
    assert_eq!(window.ui().take_requests(), [], "there is no server to ask");
    let said = window
        .controls()
        .into_iter()
        .any(|found| found.name.starts_with("Sharing is for documents a Letters server keeps."));
    assert!(said, "the dialog says why it is empty");
    assert!(!has(&mut window, Role::TextInput, "Add a group"));
    window.click(Role::Button, "Done").assert_closed(Role::Dialog, "Share");
}

#[test]
fn sharing_is_for_someone_who_writes_the_document() {
    for access in [Access::Comments, Access::Reads] {
        let mut window = plan();
        open(&mut window, shared(true, Vec::new()));
        // The role can fall while the dialog is out.
        window.ui().set_access(access);
        window.assert_closed(Role::Dialog, "Share");
        assert!(!has(&mut window, Role::Button, "Share"), "{access:?}: the bar");
        window.key("Mod+Shift+P").type_text("share");
        let listed =
            window.controls().into_iter().any(|found| found.role == Role::ListBoxOption && found.name == "Share");
        assert!(!listed, "{access:?}: the commands");
        window.key("Enter").key("Escape").assert_closed(Role::Dialog, "Share");
        assert_eq!(window.ui().take_requests(), [], "{access:?}");

        window.ui().set_access(Access::Edits);
        window.click(Role::Button, "Share").assert_open(Role::Dialog, "Share");
    }
}

#[test]
fn the_owner_reads_who_reached_the_document_by_day() {
    let mut window = plan();
    open(&mut window, shared(true, vec![link("l1", "Builder")]));
    assert_eq!(shown(&mut window), ["Access"], "the dialog opens on who may reach the document");

    window.click(Role::Tab, "Activity");
    assert_eq!(window.ui().take_requests(), [Request::AccessLog]);
    assert_eq!(shown(&mut window), ["Activity"]);
    assert!(has(&mut window, Role::Label, ASKING), "the wait is said");
    assert!(rows(&mut window).is_empty(), "the people are on the other tab");
    assert!(!has(&mut window, Role::TextInput, "Add a group"));

    window.ui().set_access_log(log());
    assert!(!has(&mut window, Role::Label, ASKING));
    assert_eq!(listed(&mut window, Role::List), ["Today", "Yesterday", "Thursday", "26 Oct"]);
    let said = [
        "Ben Price started editing",
        "Guest was refused a download",
        "Carla Diaz wrote a comment",
        "Guest viewed the document",
        "Guest printed the document",
        "Ben Price downloaded a copy",
    ];
    assert_eq!(rows(&mut window), said, "newest first");
    // A row says the role, the link a guest came by, where from and when.
    let refused = window.find(Role::ListItem, "Guest was refused a download").expect("listed above");
    let about = refused.description.unwrap_or_default();
    for part in ["Reader", "Builder", "203.0.113.7", "Firefox on Linux", "09:00"] {
        assert!(about.contains(part), "{part}: {about}");
    }
    assert_eq!(window.ui().take_requests(), [], "the answer asks for nothing");

    // The other tab is as it was, and the log is asked for afresh each time it is turned to.
    window.click(Role::Tab, "Access");
    assert_eq!(rows(&mut window).len(), 3);
    assert_eq!(window.ui().take_requests(), []);
    window.click(Role::Tab, "Activity");
    assert_eq!(window.ui().take_requests(), [Request::AccessLog]);
    assert_eq!(rows(&mut window).len(), said.len(), "what was last said shows meanwhile");

    // The dialog opens on who may reach the document, whatever it was closed on.
    window.click(Role::Button, "Done").click(Role::Button, "Share");
    assert_eq!(shown(&mut window), ["Access"]);
    assert_eq!(window.ui().take_requests(), [Request::Shares]);
}

#[test]
fn the_log_tells_its_days_and_times_by_the_persons_clock() {
    let half = HOUR / 2;
    // How far the clock is ahead of UTC in minutes, and where two entries of half past eleven and half
    // past ten at night UTC on the 14th then fall for someone whose clock says the 15th.
    let cases = [
        (0, [("Yesterday", "23:30"), ("Yesterday", "22:30")]),
        (60, [("Today", "00:30"), ("Yesterday", "23:30")]),
        (-300, [("Yesterday", "18:30"), ("Yesterday", "17:30")]),
        (330, [("Today", "05:00"), ("Today", "04:00")]),
    ];
    for (zone, falls) in cases {
        let mut window = plan();
        window.ui().set_utc_offset(zone);
        let entry = |ago: u64| reach(ago, by("Ben Price"), Action::Open, Grade::Editor);
        activity(&mut window, vec![entry(12 * HOUR + half), entry(13 * HOUR + half)]);
        let mut days: Vec<&str> = falls.iter().map(|(day, _)| *day).collect();
        days.dedup();
        assert_eq!(listed(&mut window, Role::List), days, "{zone}");
        let rows: Vec<Found> = window.controls().into_iter().filter(|found| found.role == Role::ListItem).collect();
        for (row, (_, clock)) in rows.iter().zip(falls) {
            assert!(row.description.as_deref().unwrap_or_default().ends_with(clock), "{zone}: {clock}");
            assert!(has(&mut window, Role::Label, clock), "{zone}: the time is on the row");
        }
    }

    // The day it is now is the person's too: half an hour into the 16th by their clock, the 15th is yesterday.
    let mut window = plan();
    window.platform.wall = Some(NOON + 11 * HOUR + half);
    window.ui().set_utc_offset(60);
    activity(&mut window, vec![reach(HOUR, by("Ben Price"), Action::Open, Grade::Editor)]);
    assert_eq!(listed(&mut window, Role::List), ["Yesterday"]);
}

#[test]
fn a_log_with_nothing_in_it_says_so() {
    let mut window = plan();
    activity(&mut window, Vec::new());
    assert!(has(&mut window, Role::Label, "Nobody has reached this document through a share yet."));
    assert!(rows(&mut window).is_empty());
}

#[test]
fn earlier_entries_are_asked_for_a_page_at_a_time() {
    let entry = |at: u64| reach(at * HOUR, guest("Builder"), Action::Open, Grade::Reader);
    // A log shorter than the server's page is whole.
    let mut window = plan();
    activity(&mut window, (0..199).map(entry).collect());
    assert!(!has(&mut window, Role::Button, "Show earlier"));

    // A full page may have others before it: they are asked for from its last row on.
    let mut window = plan();
    activity(&mut window, (0..200).map(entry).collect());
    press(&mut window, "Show earlier");
    let before = || Request::AccessLogBefore(NOON - 199 * HOUR);
    assert_eq!(window.ui().take_requests(), [before()]);
    assert!(window.find(Role::Button, "Show earlier").is_some_and(|found| found.disabled), "it waits");
    press(&mut window, "Show earlier");
    assert_eq!(window.ui().take_requests(), [], "asked once, however long the answer takes");

    // What the server refuses can be asked for again.
    window.ui().refuse_share(REFUSAL);
    press(&mut window, "Show earlier");
    assert_eq!(window.ui().take_requests(), [before()]);

    window.ui().add_access_log((200..400).map(entry).collect());
    assert_eq!(rows(&mut window).len(), 400, "the earlier rows follow those that were listed");
    press(&mut window, "Show earlier");
    assert_eq!(window.ui().take_requests(), [Request::AccessLogBefore(NOON - 399 * HOUR)]);
    // A page that is not full is the last.
    window.ui().add_access_log((400..430).map(entry).collect());
    assert_eq!(rows(&mut window).len(), 430);
    assert!(!has(&mut window, Role::Button, "Show earlier"));

    // Rows nobody asked for come too late, as after the log was told afresh.
    window.ui().add_access_log((430..440).map(entry).collect());
    assert_eq!(rows(&mut window).len(), 430);
}

#[test]
fn a_row_says_how_long_an_edit_lasted_and_the_link_someone_came_by() {
    // How many seconds a session of edits has lasted, and what its row says.
    let cases = [
        (None, "Ben Price started editing"),
        (Some(59), "Ben Price started editing"),
        (Some(60), "Ben Price edited for 1 minute"),
        (Some(12 * 60 + 30), "Ben Price edited for 12 minutes"),
        (Some(HOUR), "Ben Price edited for 1 hour"),
        (Some(HOUR + 5 * 60), "Ben Price edited for 1 hour 5 minutes"),
        (Some(2 * HOUR + 60), "Ben Price edited for 2 hours 1 minute"),
    ];
    for (lasted, said) in cases {
        let mut window = plan();
        activity(&mut window, vec![Reach { lasted, ..reach(HOUR, by("Ben Price"), Action::Edit, Grade::Editor) }]);
        assert_eq!(rows(&mut window), [said], "{lasted:?}");
    }

    // Someone signed in who came by a link is named, and the link with them.
    let mut window = plan();
    let link = Some(Via { id: "l1".into(), label: "Accountant".into() });
    activity(&mut window, vec![Reach { link, ..reach(HOUR, by("Carla Diaz"), Action::Open, Grade::Reader) }]);
    let row = window.find(Role::ListItem, "Carla Diaz viewed the document").expect("named");
    assert!(row.description.unwrap_or_default().contains("by the link for Accountant"));
    assert!(has(&mut window, Role::Label, "Accountant"), "the link is on the row");
}

#[test]
fn the_log_says_how_long_the_server_keeps_it() {
    // The days a shell says the server keeps a row, and what the tab says over the log.
    let cases = [
        (None, "Everything that reached this document through a share, newest first."),
        (Some(0), "Everything that reached this document through a share, newest first."),
        (Some(1), "Everything that reached this document through a share in the last day, newest first."),
        (Some(365), "Everything that reached this document through a share in the last 365 days, newest first."),
    ];
    for (days, said) in cases {
        let mut window = plan();
        if let Some(days) = days {
            window.ui().set_log_days(days);
        }
        activity(&mut window, log());
        assert!(has(&mut window, Role::Label, said), "{days:?}");
    }
}

#[test]
fn who_reached_the_document_is_its_owners_to_read() {
    // Someone else has the one list, and so has a document no server keeps.
    let mut window = plan();
    open(&mut window, shared(false, Vec::new()));
    assert!(listed(&mut window, Role::Tab).is_empty(), "not the owner");
    let mut window = plan();
    window.ui().set_connection(None);
    window.click(Role::Button, "Share");
    assert!(listed(&mut window, Role::Tab).is_empty(), "kept on this device");

    // While a link is made its form stands alone.
    let mut window = plan();
    open(&mut window, shared(true, Vec::new()));
    assert_eq!(listed(&mut window, Role::Tab), ["Access", "Activity"]);
    window.click(Role::Button, "New link");
    assert!(listed(&mut window, Role::Tab).is_empty(), "the form");
    window.click(Role::Button, "Cancel");

    // An owner who stops being one while reading the log is shown the people again.
    window.click(Role::Tab, "Activity");
    window.ui().set_access_log(log());
    window.ui().set_shares(shared(false, Vec::new()));
    assert!(listed(&mut window, Role::Tab).is_empty());
    assert_eq!(rows(&mut window), ["Alice Martin, Owner", "Ben Price, Editor", "Family, Commenter"]);
}

#[test]
fn a_log_the_server_refuses_is_said_and_not_asked_for_again() {
    let mut window = plan();
    open(&mut window, shared(true, Vec::new()));
    window.click(Role::Tab, "Activity");
    window.ui().take_requests();
    window.ui().refuse_share(REFUSAL);
    window.assert_open(Role::Alert, REFUSAL).assert_open(Role::Dialog, "Share");
    assert_eq!(window.ui().take_requests(), [], "a server out of reach is not asked for ever");
    assert!(rows(&mut window).is_empty());
    assert!(!has(&mut window, Role::Label, ASKING), "nothing is waited for any more");
    assert_eq!(shown(&mut window), ["Activity"], "the tab stays");

    // The other tab is not held up by it, and turning back asks once more.
    window.click(Role::Tab, "Access").assert_closed(Role::Alert, REFUSAL);
    assert_eq!(rows(&mut window).len(), 3);
    window.click(Role::Tab, "Activity");
    assert_eq!(window.ui().take_requests(), [Request::AccessLog]);
    window.ui().set_access_log(log());
    assert_eq!(rows(&mut window).len(), log().len());
}

#[test]
fn the_tabs_are_turned_with_the_arrow_keys() {
    let at = |window: &mut Harness| window.focused().map(|found| (found.role, found.name));
    let mut window = plan();
    open(&mut window, shared(true, Vec::new()));
    // The tabs are one stop for the Tab key, after the way out.
    window.key("Tab");
    assert_eq!(at(&mut window), Some((Role::Tab, "Access".into())));
    window.key("Right");
    assert_eq!(at(&mut window), Some((Role::Tab, "Activity".into())));
    assert_eq!(shown(&mut window), ["Access"], "moving along them turns nothing");
    window.key("Enter");
    assert_eq!(shown(&mut window), ["Activity"]);
    assert_eq!(window.ui().take_requests(), [Request::AccessLog]);
    assert_eq!(at(&mut window), Some((Role::Tab, "Activity".into())), "the keyboard stays on the tab");
    window.key("Left").key("Enter");
    assert_eq!(shown(&mut window), ["Access"]);
    window.key("Tab");
    assert_ne!(at(&mut window).map(|(role, _)| role), Some(Role::Tab), "one press passes them both");
}
