//! Links to a document: who makes one, what is told and kept of it, whom it lets in and to what, and how
//! it ends.

mod common;

use std::io::Cursor;
use std::net::SocketAddr;

use axum::http::{Method, StatusCode};
use common::desk::{Desk, settle};
use common::{Browser, Client, Raw, START, Server, URL, follow, let_in, link, planned, texts};
use letters_api::{
    Allows, Code, CreatedLink, Entry, Error, Fetch, Listing, NewLink, Opened, Rename, Role, Seen, Shares, Tag, Trust,
    paths,
};
use letters_format::{Archive, HISTORY};
use letters_model::Op;
use letters_sync::Link::Live;

/// How many links the row of Ada's document says lead to it.
async fn counted(ada: &Client) -> Option<u32> {
    let own: Listing = ada.get(&paths::entries("home", "")).await.unwrap();
    own.entries[0].tags.iter().find_map(|tag| if let Tag::Links { count } = tag { Some(*count) } else { None })
}

/// Whether the document opens trusted in a browser.
async fn trusted(browser: &mut Browser, id: &str) -> Trust {
    browser.get(&paths::document(id)).await.json::<Opened>().trust
}

/// The seconds a refusal says to wait before asking again. None, when it does not say.
fn waits(refused: &Raw) -> u64 {
    refused.header("retry-after").parse().unwrap_or(0)
}

/// Follows a link that leads nowhere on the server at `at`, as a proxy in front of it would for `address`.
fn knock(at: SocketAddr, address: &str) -> u16 {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let asked = agent.get(&format!("http://{at}/l/nowhere")).header("X-Forwarded-For", address).call();
    asked.expect("the server answers").status().as_u16()
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

#[tokio::test]
async fn a_link_lets_whoever_follows_it_in_to_its_document_and_to_nothing_else() {
    let (server, ada, _, id) = planned("links-followed").await;
    let other = ada.start("home", "", "Diary").await.id.unwrap();
    let allows = Allows { download: true, ..Allows::default() };
    let new = NewLink { allows, ..link("Accountant", Role::Commenter) };
    let made: CreatedLink = ada.post(&paths::links(&id), &new).await.unwrap();

    // The page behind the address sends the browser on to the document, and says of itself as little as
    // it can: no referrer, nothing to index, and a cookie no script reads.
    let mut guest = server.visitor();
    let door = follow(&mut guest, &made).await;
    assert_eq!((door.status, door.location()), (StatusCode::SEE_OTHER, paths::editor_page(&id).as_str()));
    assert_eq!(door.header("referrer-policy"), "no-referrer");
    assert!(door.header("x-robots-tag").contains("noindex"), "{}", door.header("x-robots-tag"));
    assert!(door.cookies().iter().all(|cookie| cookie.contains("HttpOnly")), "{:?}", door.cookies());

    // The document opens as the link says, signed with the link's label, and held back until trusted.
    let opened: Opened = guest.get(&paths::document(&id)).await.json();
    let told = (opened.role, opened.space, opened.path.as_str(), opened.author.as_str(), opened.allows, opened.trust);
    assert_eq!(told, (Role::Commenter, None, "", "Accountant", allows, Trust::Safe));

    // Nothing else is there for them: they are nobody to the rest of the server.
    let elsewhere = [
        paths::SESSION.to_owned(),
        paths::SETTINGS.to_owned(),
        paths::DICTIONARY.to_owned(),
        paths::OWN_SOURCES.to_owned(),
        paths::SPACES.to_owned(),
        paths::RECENT.to_owned(),
        paths::SHARED.to_owned(),
        paths::PEOPLE.to_owned(),
        paths::entries("home", ""),
        paths::document(&other),
        paths::file(&other),
    ];
    for path in elsewhere {
        assert_eq!(guest.get(&path).await.status, StatusCode::UNAUTHORIZED, "{path}");
    }
    assert_eq!(guest.post(&paths::links(&id), &link("Mine", Role::Editor)).await.status, StatusCode::UNAUTHORIZED);
    // Who the document is shared with is asked by its page, and is said in words to be none of theirs.
    let shares = guest.get(&paths::shares(&id)).await;
    assert_eq!((shares.status, shares.json::<Error>().code), (StatusCode::FORBIDDEN, Code::Forbidden));
    assert!(guest.get(&paths::folder_page("home", "")).await.location().starts_with(paths::SIGN_IN_PAGE));
    // And a commenter changes nothing of the document's name.
    let renamed = guest.post(&paths::rename(&id), &Rename { name: "Mine".into() }).await;
    assert_eq!(renamed.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn someone_signed_in_follows_a_link_as_themselves() {
    let (server, ada, _, id) = planned("links-signed").await;
    let made: CreatedLink = ada.post(&paths::links(&id), &link("Readers", Role::Reader)).await.unwrap();
    // Carol reaches the document by the link alone. Ben's share lets him do more, and he keeps that.
    let cases = [("carol", StatusCode::NOT_FOUND, Role::Reader), ("ben", StatusCode::OK, Role::Editor)];
    for (username, before, role) in cases {
        let mut browser = server.reader(username, &[]);
        assert_eq!(browser.get(&paths::document(&id)).await.status, before, "{username}, before the link");
        assert_eq!(follow(&mut browser, &made).await.location(), paths::editor_page(&id));
        let opened: Opened = browser.get(&paths::document(&id)).await.json();
        assert_eq!((opened.role, opened.author.to_lowercase()), (role, username.to_owned()));
        assert_eq!(browser.get(paths::SPACES).await.status, StatusCode::OK, "what is theirs stays theirs");
    }
}

#[tokio::test]
async fn a_link_that_asks_a_passphrase_lets_in_whoever_knows_it_and_takes_few_guesses() {
    let (server, ada, _, id) = planned("links-asks").await;
    let new = NewLink { passphrase: Some("orchard".into()), ..link("Accountant", Role::Reader) };
    let made: CreatedLink = ada.post(&paths::links(&id), &new).await.unwrap();
    let at = made.url.strip_prefix(URL).unwrap();
    let (right, wrong) = ([("passphrase", "orchard")], [("passphrase", "apple")]);

    // The page asks, and sends what is typed back to the link's own address.
    let mut guest = server.visitor();
    let asked = follow(&mut guest, &made).await;
    assert_eq!(asked.status, StatusCode::OK);
    assert!(asked.text().contains(r#"name="passphrase""#) && asked.text().contains(at), "{}", asked.text());
    assert_eq!(guest.submit(at, &wrong, None).await.status, StatusCode::FORBIDDEN);
    assert_eq!(guest.get(&paths::document(&id)).await.status, StatusCode::UNAUTHORIZED, "a wrong one opens nothing");
    assert_eq!(guest.submit(at, &right, None).await.location(), paths::editor_page(&id));
    assert_eq!(guest.get(&paths::document(&id)).await.status, StatusCode::OK);

    // Guesses run out within the minute, for the right one too, and come back with the next.
    let (mut guesser, mut guesses) = (server.visitor(), 0);
    let slow = loop {
        let tried = guesser.submit(at, &wrong, None).await;
        if tried.status != StatusCode::FORBIDDEN || guesses == 10 {
            break tried;
        }
        guesses += 1;
    };
    assert_eq!(slow.status, StatusCode::TOO_MANY_REQUESTS);
    assert!((1..=60).contains(&waits(&slow)), "the card's answer says when to try again");
    assert!(guesses <= 5, "{guesses} guesses in a minute");
    assert_eq!(guesser.submit(at, &right, None).await.status, StatusCode::TOO_MANY_REQUESTS);
    server.wait(60);
    assert_eq!(guesser.submit(at, &right, None).await.location(), paths::editor_page(&id));
}

#[tokio::test]
async fn a_link_that_is_over_lets_nobody_in_and_whoever_it_let_in_goes() {
    let (server, ada, _, id) = planned("links-over").await;
    let ends = NewLink { expires: Some(START + 3600), ..link("Builder", Role::Reader) };
    let (mut early, short) = let_in(&server, &ada, &id, &ends).await;
    let (mut late, long) = let_in(&server, &ada, &id, &link("Accountant", Role::Editor)).await;
    let reached = paths::document(&id);

    server.wait(3600);
    assert_eq!(early.get(&reached).await.status, StatusCode::UNAUTHORIZED, "the link ran out");
    assert_eq!(follow(&mut server.visitor(), &short).await.status, StatusCode::NOT_FOUND);
    assert_eq!(late.get(&reached).await.status, StatusCode::OK);

    // A visit is over by the next day, and the link starts another.
    server.wait(24 * 3600);
    assert_eq!(late.get(&reached).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(follow(&mut late, &long).await.location(), paths::editor_page(&id));
    let mut desk = Desk::paged(&server, &late, &id).await.unwrap();
    settle([&mut desk]).await;
    assert_eq!(desk.line.link(), Live(letters_sync::Role::Editor));

    // Revoked, it ends the sessions it led in, opens nothing for the cookie it gave, and leads nowhere.
    // Ada's own sign-in on the app is a day old by now, so she signs in again.
    drop(ada);
    let ada = server.person("ada", &[]);
    assert_eq!(revoke(&ada, &id, &long.link.id).await.0, StatusCode::NO_CONTENT);
    settle([&mut desk]).await;
    assert!(!matches!(desk.line.link(), Live(_)), "{:?}", desk.line.link());
    assert_eq!(late.get(&reached).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(Desk::paged(&server, &late, &id).await.err().map(|refused| refused.code), Some(Code::SignedOut));
    let gone = follow(&mut server.visitor(), &long).await;
    assert_eq!((gone.status, gone.header("referrer-policy")), (StatusCode::NOT_FOUND, "no-referrer"));
}

#[tokio::test]
async fn a_link_withholds_what_it_does_not_let_its_reader_take_away() {
    let (server, ada, _, id) = planned("links-allows").await;
    let (all, none) = (Allows { download: true, copy: true, print: true }, Allows::default());
    let cases = [
        none,
        Allows { download: true, ..none },
        Allows { download: false, ..all },
        Allows { print: false, ..all },
        Allows { copy: false, ..all },
        all,
    ];
    for allows in cases {
        let (mut guest, _) = let_in(&server, &ada, &id, &NewLink { allows, ..link("Accountant", Role::Reader) }).await;
        let opened: Opened = guest.get(&paths::document(&id)).await.json();
        assert_eq!(opened.allows, allows);
        let file = guest.get(&paths::file(&id)).await.status;
        assert_eq!(file == StatusCode::OK, allows.download, "the file, with {allows:?}: {file}");
        // The page of its text is all that a reader's link shows. It offers the file, lets its text be
        // selected and goes to paper where the link lets its reader take that much away.
        let page = guest.get(&paths::read_page(&id)).await;
        assert_eq!(page.status, StatusCode::OK, "the page to read, with {allows:?}");
        let page = page.text();
        let offers = page.contains(&format!("href=\"{}\"", paths::file(&id)));
        let (selects, prints) = (!page.contains("user-select: none"), !page.contains("does not let its reader print"));
        assert_eq!((offers, selects, prints), (allows.download, allows.copy, allows.print), "{allows:?}");
    }
    // Someone signed in reads the page as it always was: what is theirs to take is on their own pages.
    let ben = server.reader("ben", &[]).get(&paths::read_page(&id)).await.text();
    assert!(!ben.contains(&paths::file(&id)) && !ben.contains("user-select: none"), "{ben}");
}

#[tokio::test]
async fn a_link_for_a_reader_shows_the_document_to_read_and_opens_no_line_to_it() {
    let (server, ada, _, id) = planned("links-read").await;
    let made: CreatedLink = ada.post(&paths::links(&id), &link("Readers", Role::Reader)).await.unwrap();
    let commenting: CreatedLink = ada.post(&paths::links(&id), &link("Reviewers", Role::Commenter)).await.unwrap();
    // Who follows which link, and whether the document is then theirs to read alone: a link for a reader,
    // where nothing else leads them to it. Ben's share lets him edit, whatever link he follows.
    let cases = [
        (None, &made, true),
        (Some("carol"), &made, true),
        (Some("ben"), &made, false),
        (None, &commenting, false),
        (Some("carol"), &commenting, false),
    ];
    for (username, by, reads) in cases {
        let mut browser = username.map_or_else(|| server.visitor(), |username| server.reader(username, &[]));
        follow(&mut browser, by).await;
        let said = format!("{username:?} by {}", by.link.label);
        // No line to the document's room is opened for a reader: its past would go down it.
        let line = Desk::paged(&server, &browser, &id).await.map(|_| ()).map_err(|refused| refused.code);
        assert_eq!(line, if reads { Err(Code::Forbidden) } else { Ok(()) }, "{said}");
        // The page of its text is there for either, and holds the document as it stands.
        let page = browser.get(&paths::read_page(&id)).await;
        assert_eq!(page.status, StatusCode::OK, "{said}");
        assert!(page.text().contains("<title>Plan</title>"), "{said}");
    }
}

/// What a file tells of how its document came to be: whether it holds a history, how many notes, and the
/// id it goes by.
fn past(file: Vec<u8>) -> (bool, usize, String) {
    let package = Archive::open(Cursor::new(file)).expect("a document").package().expect("it reads");
    let history = package.entries.keys().any(|name| name.starts_with(HISTORY));
    (history, package.notes().expect("its notes read").notes.len(), package.id)
}

#[tokio::test]
async fn a_link_hands_over_the_document_as_it_stands_and_nothing_of_its_past() {
    let (server, ada, _, id) = planned("links-clean").await;
    // Ada writes a line, takes it out again, writes another and leaves a comment.
    let mut desk = Desk::open(&server, &ada, &id).await.unwrap();
    settle([&mut desk]).await;
    let (draft, line) = ("Offer no more than 400,000. ", desk.model.as_ref().unwrap().doc().blocks[0].id.clone());
    desk.write(draft);
    desk.edit([Op::DeleteText { block: line, range: 0..draft.len() }]);
    desk.write("We would like to make an offer. ");
    desk.comment("Is 400,000 still the ceiling?");
    settle([&mut desk]).await;
    desk.leave().await;

    let taking = |role| NewLink { allows: Allows { download: true, ..Allows::default() }, ..link("Seller", role) };
    let made: CreatedLink = ada.post(&paths::links(&id), &taking(Role::Reader)).await.unwrap();
    // Who asks for the file, a link they followed first, and whether they get its past with it. Ben has
    // a share of his own, and a link takes nothing from that.
    let cases = [
        (Some("ada"), None, true),
        (Some("ben"), None, true),
        (Some("ben"), Some(Role::Reader), true),
        (Some("carol"), Some(Role::Reader), false),
        (None, Some(Role::Reader), false),
        (None, Some(Role::Editor), false),
    ];
    for (username, by, whole) in cases {
        let mut browser = username.map_or_else(|| server.visitor(), |username| server.reader(username, &[]));
        match by {
            Some(Role::Reader) => drop(follow(&mut browser, &made).await),
            Some(role) => {
                let made: CreatedLink = ada.post(&paths::links(&id), &taking(role)).await.unwrap();
                follow(&mut browser, &made).await;
            }
            None => {}
        }
        let file = browser.get(&paths::file(&id)).await;
        assert_eq!(file.status, StatusCode::OK, "{username:?} by {by:?}");
        let text = texts(file.body.clone()).join("\n");
        assert!(text.contains("We would like to make an offer."), "{username:?} by {by:?}: {text}");
        let (history, notes, named) = past(file.body);
        assert_eq!((history, notes > 0, named == id), (whole, whole, whole), "{username:?} by {by:?}");
        // An editor is told as much, so that it shows and saves no more than the file holds.
        let opened: Opened = browser.get(&paths::document(&id)).await.json();
        assert_eq!(opened.by_link, !whole, "{username:?} by {by:?}");
    }
}

#[tokio::test]
async fn where_a_person_read_up_to_is_kept_by_their_account_and_for_nobody_else() {
    let (server, ada, ben, id) = planned("links-seen").await;
    let (at, said) = (paths::document(&id), paths::seen(&id));
    let seen = |revision: &str| Seen { revision: revision.into() };
    // Nothing is known of a first visit. What Ada then says is hers on whatever device, and not Ben's.
    assert_eq!(ada.get::<Opened>(&at).await.unwrap().last_seen, None);
    for revision in ["r1", "r2"] {
        ada.put::<()>(&said, &seen(revision)).await.unwrap();
    }
    let elsewhere = server.browser("ada", &[]);
    assert_eq!(elsewhere.get::<Opened>(&at).await.unwrap().last_seen.as_deref(), Some("r2"));
    assert_eq!(ben.get::<Opened>(&at).await.unwrap().last_seen, None);

    // A link's own visitor has no account. What they say is taken, and kept for nobody.
    let (mut guest, _) = let_in(&server, &ada, &id, &link("Accountant", Role::Reader)).await;
    let body = serde_json::to_vec(&seen("r9")).unwrap();
    let taken = guest.send(Method::PUT, &said, letters_api::JSON, body, &[("origin", URL)]).await;
    assert_eq!(taken.status, StatusCode::NO_CONTENT);
    assert_eq!(guest.get(&at).await.json::<Opened>().last_seen, None);
    assert_eq!(ben.get::<Opened>(&at).await.unwrap().last_seen, None);

    // Whoever does not reach the document says nothing of it, and what is no revision is not kept.
    let carol = server.person("carol", &[]);
    assert_eq!(carol.put::<()>(&said, &seen("r1")).await.unwrap_err().code, Code::NotFound);
    for revision in [String::new(), "r".repeat(5000)] {
        assert_eq!(ada.put::<()>(&said, &seen(&revision)).await.unwrap_err().code, Code::Invalid);
    }
    assert_eq!(ada.get::<Opened>(&at).await.unwrap().last_seen.as_deref(), Some("r2"));
}

#[tokio::test]
async fn a_link_that_edits_writes_to_the_document() {
    let (server, ada, _, id) = planned("links-edits").await;
    let (mut editor, _) = let_in(&server, &ada, &id, &link("Accountant", Role::Editor)).await;
    let (mut reader, _) = let_in(&server, &ada, &id, &link("Builder", Role::Commenter)).await;
    let mut writes = Desk::paged(&server, &editor, &id).await.unwrap();
    let mut reads = Desk::paged(&server, &reader, &id).await.unwrap();
    settle([&mut writes, &mut reads]).await;
    let seated = (writes.line.link(), reads.line.link());
    assert_eq!(seated, (Live(letters_sync::Role::Editor), Live(letters_sync::Role::Commenter)));

    writes.write("From the accountant. ");
    settle([&mut writes, &mut reads]).await;
    assert!(reads.texts().join("\n").contains("From the accountant."), "{:?}", reads.texts());
    writes.leave().await;
    reads.leave().await;
    let file = ada.send(Method::GET, &paths::file(&id), "", Vec::new()).await.body;
    assert!(texts(file).join("\n").contains("From the accountant."), "the work is in its owner's file");

    // An editor names the document, from the server's own page and from no other site's.
    let named = serde_json::to_vec(&Rename { name: "Plans".into() }).unwrap();
    let foreign = [("origin", "https://elsewhere.example.test")];
    let forged = editor.send(Method::POST, &paths::rename(&id), letters_api::JSON, named, &foreign).await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert_eq!(reader.post(&paths::rename(&id), &Rename { name: "Mine".into() }).await.status, StatusCode::FORBIDDEN);
    let renamed: Entry = editor.post(&paths::rename(&id), &Rename { name: "Plans".into() }).await.json();
    assert_eq!((renamed.name.as_str(), renamed.space), ("Plans", None));
}

#[tokio::test]
async fn whoever_a_link_lets_in_trusts_the_document_for_their_own_visit() {
    let (server, ada, _, id) = planned("links-trust").await;
    let made: CreatedLink = ada.post(&paths::links(&id), &link("Readers", Role::Reader)).await.unwrap();
    let (mut one, mut next) = (server.visitor(), server.visitor());
    follow(&mut one, &made).await;
    follow(&mut next, &made).await;

    assert_eq!(one.post(&paths::trust(&id), &()).await.status, StatusCode::NO_CONTENT);
    let seen = (trusted(&mut one, &id).await, trusted(&mut next, &id).await);
    assert_eq!(seen, (Trust::Trusted, Trust::Safe), "one holder of a link does not decide for the next");
    // And for that visit alone: the link followed again is a document from a stranger again.
    follow(&mut one, &made).await;
    assert_eq!(trusted(&mut one, &id).await, Trust::Safe);
}

#[tokio::test]
async fn a_visit_asks_the_server_for_so_many_fetches_a_minute_and_no_more() {
    let (server, ada, ben, id) = planned("links-fetches").await;
    let (mut guest, _) = let_in(&server, &ada, &id, &link("Accountant", Role::Reader)).await;
    // A fetch the rules refuse at once is counted like any other.
    let fetch = Fetch { url: "http://localhost/sky.png".into(), asked: true };
    let mut fetched = 0;
    let slow = loop {
        let answer = guest.post(&paths::fetch_picture(&id), &fetch).await;
        if answer.status != StatusCode::UNPROCESSABLE_ENTITY || fetched == 200 {
            break answer;
        }
        fetched += 1;
    };
    assert_eq!(slow.status, StatusCode::TOO_MANY_REQUESTS, "after {fetched}");
    assert!(fetched >= 10, "a page with some pictures loads: {fetched}");
    assert!((1..=60).contains(&waits(&slow)), "the answer says when to ask again");

    // Someone signed in is not counted with them, and the next minute the visit asks again.
    let signed = ben.post::<()>(&paths::fetch_picture(&id), &fetch).await;
    assert_eq!(signed.unwrap_err().code, Code::Untrusted);
    server.wait(60);
    assert_eq!(guest.post(&paths::fetch_picture(&id), &fetch).await.status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn following_links_is_counted_by_the_address_a_named_proxy_tells() {
    // The proxies the settings name, and whether a second address then has looks of its own. The test
    // asks from this machine, which speaks for others unless the settings say that nobody does.
    let cases: [(&str, Option<&[&str]>, bool); 2] =
        [("links-proxied", None, true), ("links-unproxied", Some(&[]), false)];
    for (name, proxies, apart) in cases {
        let server = Server::behind(name, proxies);
        let at = server.listen().await;
        let asked = tokio::task::spawn_blocking(move || {
            let looks = (0..100).take_while(|_| knock(at, "203.0.113.1") == 404).count();
            (looks, knock(at, "203.0.113.2"))
        });
        let (looks, other) = asked.await.unwrap();
        assert!(looks < 100, "{name}: an address runs out of looks");
        assert_eq!(other == 404, apart, "{name}: the next address was answered {other}");
    }
}
