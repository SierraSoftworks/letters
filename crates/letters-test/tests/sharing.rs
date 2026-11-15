//! The Share dialog: who a document on a server is shared with, the links made for it, and what the window
//! asks of the shell.

use letters_api::{Allows, CreatedLink, Grant, Grantee, Link, NewLink, Role as Grade, Shares};
use letters_test::{Found, Harness, Role};
use letters_ui::{Access, Connection, Request, Shared};

/// Noon on 15 November 2026, in seconds since 1970.
const NOON: u64 = 1_794_744_000;
const DAY: u64 = 86_400;
const REFUSAL: &str = "Only the owner says who this document is shared with.";

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

/// Something a test does to the window.
type Does = fn(&mut Harness);

fn has(window: &mut Harness, role: Role, name: &str) -> bool {
    window.find(role, name).is_some()
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
