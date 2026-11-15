//! Links to a document: who makes one, what is told and kept of it, and that a revoked one stays listed.

mod common;

use axum::http::{Method, StatusCode};
use common::{Client, START, Server, URL};
use letters_api::{Allows, Code, CreatedLink, Grant, Grantee, Listing, NewLink, Role, Shares, Tag, paths};

fn link(label: &str, role: Role) -> NewLink {
    NewLink { label: label.into(), role, expires: None, passphrase: None, allows: Allows::default() }
}

/// A server that makes links, where Ada has "Plan" in her own space and shares it with Ben to edit: the
/// server, Ada, Ben, and the document's id.
async fn planned(name: &str) -> (Server, Client, Client, String) {
    let server = Server::linking(name);
    let (ada, ben) = (server.person("ada", &[]), server.person("ben", &[]));
    let id = ada.start("home", "", "Plan").await.id.expect("a document has an id");
    let to = Grantee::Person { id: "sub-ben".into(), name: String::new() };
    let people = vec![Grant { to, role: Role::Editor }];
    ada.put::<Shares>(&paths::shares(&id), &Shares { people, links: Vec::new() }).await.unwrap();
    (server, ada, ben, id)
}

/// How many links the row of Ada's document says lead to it.
async fn counted(ada: &Client) -> Option<u32> {
    let own: Listing = ada.get(&paths::entries("home", "")).await.unwrap();
    own.entries[0].tags.iter().find_map(|tag| if let Tag::Links { count } = tag { Some(*count) } else { None })
}

/// Asks to revoke a link of the document: the status, and how much was said with it.
async fn revoke(who: &Client, id: &str, link: &str) -> (StatusCode, usize) {
    let raw = who.send(Method::DELETE, &paths::link(id, link), "", Vec::new()).await;
    (raw.status, raw.body.len())
}

#[tokio::test]
async fn a_link_is_made_by_whoever_shares_where_the_server_makes_links() {
    let (_server, ada, ben, id) = planned("links-made").await;
    let new = NewLink {
        expires: Some(START + 3600),
        passphrase: Some("orchard".into()),
        allows: Allows { download: true, ..Allows::default() },
        ..link(" Accountant ", Role::Commenter)
    };
    let made: CreatedLink = ada.post(&paths::links(&id), &new).await.unwrap();
    // Its address is this server's, and ends in a secret nobody guesses.
    let secret = made.url.strip_prefix(&format!("{URL}/l/")).unwrap_or_else(|| panic!("{}", made.url));
    assert!(secret.len() >= 43, "256 bits: {secret}");
    let told = (made.link.label.as_str(), made.link.role, made.link.expires, made.link.passphrase, made.link.allows);
    assert_eq!(told, ("Accountant", Role::Commenter, new.expires, true, new.allows));

    // It is listed with the document's shares from then on, without its address, and its row counts it.
    let shares: Shares = ada.get(&paths::shares(&id)).await.unwrap();
    assert_eq!(shares.links, [made.link]);
    let said = serde_json::to_string(&shares).unwrap();
    assert!(!said.contains(secret) && !said.contains("orchard"), "{said}");
    assert_eq!(counted(&ada).await, Some(1));

    // Someone a share led in makes none, and nobody does on a server that makes none.
    let lent = ben.post::<CreatedLink>(&paths::links(&id), &link("Mine", Role::Reader)).await;
    assert_eq!(lent.unwrap_err().code, Code::Forbidden);
    let plain = Server::new("links-off");
    let eve = plain.person("eve", &[]);
    let other = eve.start("home", "", "Plan").await.id.unwrap();
    let off = eve.post::<CreatedLink>(&paths::links(&other), &link("Mine", Role::Reader)).await;
    assert_eq!(off.unwrap_err().code, Code::Forbidden);
}

#[tokio::test]
async fn a_link_says_who_it_is_for_and_what_it_lets_them_do() {
    let (_server, ada, _, id) = planned("links-sound").await;
    let cases = [
        (link("  ", Role::Reader), "no label"),
        (link("Accountant", Role::Owner), "an owner"),
        (NewLink { expires: Some(START), ..link("Accountant", Role::Reader) }, "an end that has come"),
        (NewLink { passphrase: Some(String::new()), ..link("Accountant", Role::Reader) }, "an empty passphrase"),
    ];
    for (new, what) in cases {
        let refused = ada.post::<CreatedLink>(&paths::links(&id), &new).await;
        assert_eq!(refused.unwrap_err().code, Code::Invalid, "{what}");
    }
    assert_eq!(counted(&ada).await, None);
}

#[tokio::test]
async fn a_revoked_link_stays_listed_and_one_that_is_over_is_counted_no_more() {
    let (server, ada, ben, id) = planned("links-revoked").await;
    let short = NewLink { expires: Some(START + 60), ..link("Builder", Role::Reader) };
    let short: CreatedLink = ada.post(&paths::links(&id), &short).await.unwrap();
    let long: CreatedLink = ada.post(&paths::links(&id), &link("Accountant", Role::Editor)).await.unwrap();
    assert_eq!(counted(&ada).await, Some(2));

    server.wait(120);
    assert_eq!(counted(&ada).await, Some(1), "the one that ran out");
    // Only whoever shares revokes, and only a link there is.
    assert_eq!(revoke(&ben, &id, &long.link.id).await.0, StatusCode::FORBIDDEN);
    assert_eq!(revoke(&ada, &id, "no-such-link").await.0, StatusCode::NOT_FOUND);
    assert_eq!(revoke(&ada, &id, &long.link.id).await, (StatusCode::NO_CONTENT, 0));
    assert_eq!(counted(&ada).await, None);

    // Both are still listed, in the order they were made, and the revoked one says since when.
    server.wait(60);
    assert_eq!(revoke(&ada, &id, &long.link.id).await.0, StatusCode::NO_CONTENT, "revoking again changes nothing");
    let shares: Shares = ada.get(&paths::shares(&id)).await.unwrap();
    let listed: Vec<_> = shares.links.iter().map(|link| (link.id.as_str(), link.revoked)).collect();
    assert_eq!(listed, [(short.link.id.as_str(), None), (long.link.id.as_str(), Some(START + 120))]);
}
