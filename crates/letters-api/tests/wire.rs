//! What goes over the wire: each body reads back as it was written, an older reader lives with a newer
//! writer, and an address keeps each part in its place.

use letters_api::{
    Access, Action, Allows, Code, Entry, EntryKind, Error, Grant, Grantee, Limits, Link, Listing, NewLink, Opened,
    PROTOCOL, Person, Role, Server, Settings, Shares, Space, SpaceKind, Tag, Token, Trust, Who, paths,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn person() -> Person {
    Person { id: "sub-1".into(), username: "ben".into(), name: "Ben".into() }
}

fn entry() -> Entry {
    Entry {
        kind: EntryKind::Document,
        name: "Roof quote".into(),
        space: Some("home".into()),
        path: "House/Roof quote.let".into(),
        id: Some("doc-1".into()),
        created: Some("2026-11-14T09:00:00Z".into()),
        modified: 1_794_700_000,
        role: Role::Owner,
        contributors: vec!["Ben".into(), "Alice".into()],
        tags: vec![Tag::ConflictCopy { of: "House/Roof.let".into() }, Tag::ImportNotes { count: 3 }, Tag::Damaged],
    }
}

fn link() -> Link {
    Link {
        id: "l1".into(),
        label: "Accountant".into(),
        role: Role::Reader,
        expires: Some(1_800_000_000),
        passphrase: true,
        allows: Allows { download: true, copy: false, print: true },
        created: 1_794_700_000,
        revoked: None,
    }
}

/// Writes `value`, reads it back, and hands on what was written.
fn round_trip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> Value {
    let written = serde_json::to_value(value).expect("it serialises");
    let read: T = serde_json::from_value(written.clone()).expect("it reads back");
    assert_eq!(&read, value);
    written
}

#[test]
fn every_body_reads_back_as_written() {
    round_trip(&Server {
        product: Server::PRODUCT.into(),
        name: "Family".into(),
        version: "0.1.0".into(),
        protocols: vec![PROTOCOL],
        links: true,
        limits: Limits::default(),
    });
    round_trip(&Token { token: "t".into(), expires: 10, renew: "r".into(), ends: 20, person: person() });
    round_trip(&Settings {
        name: Some("Benjamin".into()),
        accent: Some([1, 2, 3]),
        dictionary: vec!["yewi".into()],
        default_style: Some(json!({"format": 1})),
        ..Settings::default()
    });
    let space = Space { id: "home".into(), name: "Home".into(), kind: SpaceKind::Personal, role: Role::Owner };
    round_trip(&Listing { space, path: "House".into(), entries: vec![entry()] });
    round_trip(&Opened {
        id: "doc-1".into(),
        created: "2026-11-14T09:00:00Z".into(),
        name: "Roof quote".into(),
        space: None,
        path: String::new(),
        role: Role::Commenter,
        author: "Ben".into(),
        socket: paths::socket("doc-1"),
        trust: Trust::Safe,
        allows: Allows::default(),
        style: Some(json!({"format": 1})),
        notes: None,
    });
    let people = vec![
        Grant { to: Grantee::Person { id: "sub-2".into(), name: "Alice".into() }, role: Role::Editor },
        Grant { to: Grantee::Group { name: "family".into() }, role: Role::Reader },
    ];
    round_trip(&Shares { people, links: vec![link()] });
    round_trip(&NewLink {
        label: "Accountant".into(),
        role: Role::Reader,
        expires: None,
        passphrase: Some("s3cret".into()),
        allows: Allows::default(),
    });
    round_trip(&Access {
        time: 5,
        who: Who::Link { id: "l1".into(), label: "Accountant".into() },
        action: Action::Download,
        role: Role::Reader,
        refused: true,
        address: "203.0.113.9".into(),
        browser: "Firefox".into(),
    });
    round_trip(&Error::new(Code::NameTaken, "A document of that name is already there."));
}

#[test]
fn a_reader_lives_with_a_newer_writer() {
    // A field, a code, a tag and an action that this build has never heard of.
    let server: Server = serde_json::from_value(json!({
        "product": "letters-server", "name": "Family", "version": "9.0.0", "protocols": [1, 2], "federation": true
    }))
    .expect("an unknown field is ignored, and one left out takes its default");
    assert!(server.speaks());
    assert_eq!(server.limits, Limits::default());

    let error: Error = serde_json::from_value(json!({"code": "quota", "message": "Full."})).expect("it reads");
    assert_eq!(error.code, Code::Unknown);
    assert_eq!(error.to_string(), "Full.");

    let tag: Tag = serde_json::from_value(json!({"kind": "pinned", "by": "Ben"})).expect("it reads");
    assert_eq!(tag, Tag::Unknown);
    let action: Action = serde_json::from_value(json!("exported")).expect("it reads");
    assert_eq!(action, Action::Other);
}

#[test]
fn a_server_of_another_kind_or_version_is_told_apart() {
    let server = |product: &str, protocols: Vec<u32>| Server {
        product: product.into(),
        name: String::new(),
        version: String::new(),
        protocols,
        links: false,
        limits: Limits::default(),
    };
    assert!(server(Server::PRODUCT, vec![PROTOCOL]).speaks());
    assert!(!server(Server::PRODUCT, vec![PROTOCOL + 1]).speaks());
    assert!(!server("something-else", vec![PROTOCOL]).speaks());
}

#[test]
fn what_is_left_out_says_nothing() {
    let folder = Entry {
        kind: EntryKind::Folder,
        name: "House".into(),
        space: Some("home".into()),
        path: "House".into(),
        id: None,
        created: None,
        modified: 1,
        role: Role::Editor,
        contributors: Vec::new(),
        tags: Vec::new(),
    };
    let written = round_trip(&folder);
    for absent in ["id", "created", "contributors", "tags"] {
        assert!(written.get(absent).is_none(), "{absent} is written though it says nothing");
    }
}

#[test]
fn each_role_does_what_the_one_before_it_does() {
    let roles = [Role::Reader, Role::Commenter, Role::Editor, Role::Owner];
    assert!(roles.is_sorted());
    let may: Vec<[bool; 3]> = roles.iter().map(|role| [role.comments(), role.edits(), role.shares()]).collect();
    assert_eq!(may, [[false, false, false], [true, false, false], [true, true, false], [true, true, true]]);
}

#[test]
fn a_refusal_has_the_status_of_its_code() {
    let cases = [
        (Code::Invalid, 400),
        (Code::SignedOut, 401),
        (Code::Expired, 401),
        (Code::Forbidden, 403),
        (Code::NotFound, 404),
        (Code::NameTaken, 409),
        (Code::TooLarge, 413),
        (Code::Slow, 429),
        (Code::Unavailable, 503),
    ];
    for (code, status) in cases {
        assert_eq!(code.status(), status, "{code:?}");
    }
}

#[test]
fn an_address_keeps_each_part_in_its_place() {
    // An id comes from a file and may hold anything.
    let cases = [
        (paths::document("a/b?c#d"), "/api/v1/documents/a%2Fb%3Fc%23d"),
        (paths::file("../../etc"), "/api/v1/documents/..%2F..%2Fetc/file"),
        (paths::asset("doc 1", "ab12.png"), "/api/v1/documents/doc%201/assets/ab12.png"),
        (paths::entries("home", ""), "/api/v1/spaces/home/entries"),
        (paths::entries("home", "House/2026 & on"), "/api/v1/spaces/home/entries?path=House%2F2026%20%26%20on"),
        (paths::folder_page("home", "House/Tax 100%"), "/s/home/House/Tax%20100%25"),
        (paths::link("d", "l/1"), "/api/v1/documents/d/links/l%2F1"),
        (paths::desktop_sign_in("abc-_", 4711, "x&y"), "/auth/desktop?challenge=abc-_&port=4711&state=x%26y"),
        (paths::editor_page("é"), "/d/%C3%A9"),
    ];
    for (address, expected) in cases {
        assert_eq!(address, expected);
    }
    assert!(paths::socket("d").starts_with(paths::API));
}
