//! Documents shared with people and groups: who a share leads to a document, what it lets them do there,
//! and that only those who edit the document's space say so.

mod common;

use axum::http::Method;
use common::desk::{Desk, settle};
use common::{Client, Server, texts};
use letters_api::{Code, Entry, Grant, Grantee, Listing, Move, Opened, Rename, Role, Shares, Tag, Trust, paths};
use letters_sync::Link;

/// A row for whoever signs in as `username`, by the id the tests' people have.
fn person(username: &str, role: Role) -> Grant {
    Grant { to: Grantee::Person { id: format!("sub-{username}"), name: String::new() }, role }
}

fn group(name: &str, role: Role) -> Grant {
    Grant { to: Grantee::Group { name: name.into() }, role }
}

fn with(people: &[Grant]) -> Shares {
    Shares { people: people.to_vec(), links: Vec::new() }
}

fn named(to: &Grantee) -> &str {
    match to {
        Grantee::Person { name, .. } | Grantee::Group { name } => name,
    }
}

/// Who the shares name and what each may do, people by their names.
fn rows(shares: &Shares) -> Vec<(&str, Role)> {
    shares.people.iter().map(|grant| (named(&grant.to), grant.role)).collect()
}

/// A server where Ada has the document "Plan" in her own space and shares it with `people`: the server,
/// Ada, and the document's id. Ben, Carol and Eve have signed in before, in no group.
async fn lent(name: &str, people: &[Grant]) -> (Server, Client, String) {
    let server = Server::new(name);
    let ada = server.person("ada", &[]);
    for other in ["ben", "carol", "eve"] {
        server.person(other, &[]);
    }
    let id = ada.start("home", "", "Plan").await.id.expect("a document has an id");
    ada.put::<Shares>(&paths::shares(&id), &with(people)).await.expect("its owner shares it");
    (server, ada, id)
}

async fn opened(who: &Client, id: &str) -> Result<Opened, letters_api::Error> {
    who.get(&paths::document(id)).await
}

#[tokio::test]
async fn a_share_leads_whoever_it_names_to_the_document_and_nobody_else() {
    let people = [person("ben", Role::Editor), group("friends", Role::Reader), group("club", Role::Commenter)];
    let (server, ada, id) = lent("shares-reach", &people).await;
    let cases: [(&str, &[&str], Option<Role>); 5] = [
        ("ben", &[], Some(Role::Editor)),
        ("carol", &["friends"], Some(Role::Reader)),
        ("carol", &["friends", "club"], Some(Role::Commenter)),
        ("ben", &["friends"], Some(Role::Editor)),
        ("eve", &["family"], None),
    ];
    for (username, groups, role) in cases {
        let who = server.person(username, groups);
        let listed: Vec<Entry> = who.get(paths::SHARED).await.unwrap();
        let seen: Vec<_> = listed.iter().map(|entry| (entry.name.as_str(), entry.id.as_ref(), entry.role)).collect();
        assert_eq!(seen, Vec::from_iter(role.map(|role| ("Plan", Some(&id), role))), "{username} in {groups:?}");
        match (opened(&who, &id).await, role) {
            (Ok(opened), Some(role)) => {
                assert_eq!(opened.role, role, "{username} in {groups:?}");
                // Where it is kept is its owner's business.
                assert_eq!((opened.space, opened.path.as_str()), (None, ""));
                assert_eq!((listed[0].space.as_ref(), listed[0].path.as_str()), (None, ""));
            }
            (Err(refused), None) => assert_eq!(refused.code, Code::NotFound, "it is not there for {username}"),
            (opened, _) => panic!("{username} in {groups:?}: {opened:?}"),
        }
    }
    // Its owner has it in their own space, marked as shared, and not among what is shared with them.
    assert_eq!(ada.get::<Vec<Entry>>(paths::SHARED).await.unwrap(), []);
    let own: Listing = ada.get(&paths::entries("home", "")).await.unwrap();
    assert!(own.entries[0].tags.contains(&Tag::Shared { with: 3 }), "{:?}", own.entries[0].tags);
}

#[tokio::test]
async fn a_shared_document_is_in_safe_mode_until_the_person_trusts_it() {
    let (server, ada, id) = lent("shares-trust", &[person("ben", Role::Reader)]).await;
    let ben = server.person("ben", &[]);
    assert_eq!(opened(&ada, &id).await.unwrap().trust, Trust::Trusted, "its owner made it");
    assert_eq!(opened(&ben, &id).await.unwrap().trust, Trust::Safe);

    ben.post::<()>(&paths::trust(&id), &()).await.unwrap();

    assert_eq!(opened(&ben, &id).await.unwrap().trust, Trust::Trusted);
}

#[tokio::test]
async fn whoever_a_share_leads_in_does_not_say_who_else_it_is_shared_with() {
    let people = [person("ben", Role::Editor), person("carol", Role::Reader)];
    let (server, ada, id) = lent("shares-owner", &people).await;
    let (ben, carol) = (server.person("ben", &[]), server.person("carol", &[]));
    // An editor edits, and is not asked who else may.
    let asked = ben.get::<Shares>(&paths::shares(&id)).await;
    let taken = ben.put::<Shares>(&paths::shares(&id), &with(&[person("ben", Role::Editor)])).await;
    assert_eq!(asked.unwrap_err().code, Code::Forbidden);
    assert_eq!(taken.unwrap_err().code, Code::Forbidden);

    // Ben keeps a copy of the file in his own space. It has the document's id, and there it is his.
    let file = ben.send(Method::GET, &paths::file(&id), "", Vec::new()).await.body;
    let own = server.scratch.0.join("home/ben");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("Mine.let"), file).unwrap();
    let his: Listing = ben.get(&paths::entries("home", "")).await.unwrap();
    assert_eq!(his.entries[0].id.as_ref(), Some(&id));
    let lent: Shares = ben.put(&paths::shares(&id), &with(&[person("carol", Role::Editor)])).await.unwrap();
    assert_eq!(rows(&lent), [("Ben", Role::Owner), ("Carol", Role::Editor)]);

    // What he says of his copy says nothing of Ada's document, and his copy does not stand in for it.
    let hers: Shares = ada.get(&paths::shares(&id)).await.unwrap();
    assert_eq!(rows(&hers), [("Ada", Role::Owner), ("Ben", Role::Editor), ("Carol", Role::Reader)]);
    assert_eq!(opened(&carol, &id).await.unwrap().role, Role::Reader, "Carol is led to the one shared first");
}

#[tokio::test]
async fn everyone_who_edits_a_space_says_who_its_documents_are_shared_with() {
    let server = Server::new("shares-space");
    let (dana, grace) = (server.person("dana", &["family"]), server.person("grace", &["family"]));
    let (carol, eve) = (server.person("carol", &["friends"]), server.person("eve", &["reviewers"]));
    server.person("ben", &[]);
    let id = dana.start("family", "", "Trip").await.id.expect("a document has an id");
    let people = [person("ben", Role::Reader), person("carol", Role::Editor)];
    let lent: Shares = dana.put(&paths::shares(&id), &with(&people)).await.unwrap();
    // Nobody owns what a space of several keeps.
    assert_eq!(rows(&lent), [("Ben", Role::Reader), ("Carol", Role::Editor)]);

    // Someone else who edits the space is told the same, and takes Ben out.
    assert_eq!(grace.get::<Shares>(&paths::shares(&id)).await.unwrap(), lent);
    let left: Shares = grace.put(&paths::shares(&id), &with(&people[1..])).await.unwrap();
    assert_eq!(rows(&left), [("Carol", Role::Editor)]);
    let hers: Listing = grace.get(&paths::entries("family", "")).await.unwrap();
    assert!(hers.entries[0].tags.contains(&Tag::Shared { with: 1 }), "{:?}", hers.entries[0].tags);

    // Whoever the space lets read or comment is not asked, though a share lets one of them edit this.
    for (who, role) in [(&carol, Role::Editor), (&eve, Role::Commenter)] {
        let asked = who.get::<Shares>(&paths::shares(&id)).await;
        assert_eq!(asked.unwrap_err().code, Code::Forbidden, "{role:?}");
        assert_eq!(opened(who, &id).await.unwrap().role, role);
        let theirs: Listing = who.get(&paths::entries("family", "")).await.unwrap();
        assert_eq!((theirs.entries[0].role, theirs.entries[0].tags.as_slice()), (role, [].as_slice()));
    }
}

#[tokio::test]
async fn a_share_is_of_the_document_and_not_of_its_place() {
    let (server, ada, id) = lent("shares-place", &[person("ben", Role::Editor)]).await;
    let ben = server.person("ben", &[]);
    let mut desk = Desk::open(&server, &ben, &id).await.unwrap();
    settle([&mut desk]).await;
    assert_eq!(desk.line.link(), Link::Live(letters_sync::Role::Editor));
    desk.write("Written by Ben. ");
    settle([&mut desk]).await;
    desk.leave().await;
    let file = ada.send(Method::GET, &paths::file(&id), "", Vec::new()).await.body;
    assert!(texts(file).join("\n").contains("Written by Ben."), "an editor's work is in its owner's file");

    // An editor names the document. Where it is kept, and whether it is kept, is its owner's to say.
    let renamed: Entry = ben.post(&paths::rename(&id), &Rename { name: "Plans".into() }).await.unwrap();
    assert_eq!((renamed.name.as_str(), renamed.space), ("Plans", None));
    let moved = ben.post::<Entry>(&paths::relocate(&id), &Move { space: "home".into(), folder: String::new() }).await;
    let thrown = ben.post::<()>(&paths::trash(&id), &()).await;
    assert_eq!(moved.unwrap_err().code, Code::Forbidden);
    assert_eq!(thrown.unwrap_err().code, Code::Forbidden);
    assert_eq!(opened(&ada, &id).await.unwrap().name, "Plans");
}

#[tokio::test]
async fn a_share_that_changes_seats_whoever_it_led_in_anew() {
    let people = [person("ben", Role::Editor), person("carol", Role::Editor)];
    let (server, ada, id) = lent("shares-reseat", &people).await;
    let (ben, carol) = (server.person("ben", &[]), server.person("carol", &[]));
    let mut a = Desk::open(&server, &ada, &id).await.unwrap();
    let mut b = Desk::open(&server, &ben, &id).await.unwrap();
    let mut c = Desk::open(&server, &carol, &id).await.unwrap();
    settle([&mut a, &mut b, &mut c]).await;

    ada.put::<Shares>(&paths::shares(&id), &with(&[person("ben", Role::Reader)])).await.unwrap();
    settle([&mut a, &mut b, &mut c]).await;

    assert_eq!(a.line.link(), Link::Live(letters_sync::Role::Editor), "its owner's session goes on");
    assert!(!matches!(b.line.link(), Link::Live(_)) && !matches!(c.line.link(), Link::Live(_)));
    let mut again = Desk::open(&server, &ben, &id).await.unwrap();
    settle([&mut again]).await;
    assert_eq!(again.line.link(), Link::Live(letters_sync::Role::Reader), "Ben is back, to read");
    let gone = Desk::open(&server, &carol, &id).await.err().map(|refused| refused.code);
    assert_eq!(gone, Some(Code::NotFound), "and for Carol it is not there any more");
}

#[tokio::test]
async fn a_share_names_someone_the_server_knows_and_what_they_may_do() {
    let (_server, ada, id) = lent("shares-named", &[]).await;
    let cases = [
        (person("nobody", Role::Reader), "someone who never signed in"),
        (person("ben", Role::Owner), "a second owner"),
        (group(" ", Role::Reader), "a group with no name"),
    ];
    for (grant, what) in cases {
        let refused = ada.put::<Shares>(&paths::shares(&id), &with(&[grant])).await;
        assert_eq!(refused.unwrap_err().code, Code::Invalid, "{what}");
    }

    // Its owner's own row is not theirs to give away, and whoever is named twice has what was said last.
    let people = [person("ada", Role::Reader), person("ben", Role::Reader), person("ben", Role::Commenter)];
    let shares: Shares = ada.put(&paths::shares(&id), &with(&people)).await.unwrap();
    assert_eq!(rows(&shares), [("Ada", Role::Owner), ("Ben", Role::Commenter)]);

    // Whoever shares is offered everyone else who has signed in, and the groups the server has heard of.
    let offered: Vec<Grantee> = ada.get(paths::PEOPLE).await.unwrap();
    let names: Vec<&str> = offered.iter().map(named).collect();
    assert_eq!(names[..3], ["Ben", "Carol", "Eve"]);
    assert!(["family", "friends", "reviewers"].iter().all(|group| names[3..].contains(group)), "{names:?}");
}
