//! Who is asking: what a session opens, how long it lasts, and what a browser's cookie may not do.

mod common;

use axum::http::{Method, StatusCode};
use common::Server;
use letters_api::{Code, NewFolder, Person, Renewal, Space, Token, paths};
use letters_server::session::Carrier;

const HOUR: u64 = 60 * 60;
const DAY: u64 = 24 * HOUR;

#[tokio::test]
async fn only_the_discovery_answers_a_stranger() {
    let server = Server::new("stranger");
    let stranger = server.stranger();
    let told: letters_api::Server = stranger.get(paths::SERVER).await.expect("anyone may ask what this is");
    assert!(told.speaks());
    assert_eq!(told.name, "Test");

    let closed = [
        paths::SESSION.to_owned(),
        paths::SPACES.to_owned(),
        paths::SETTINGS.to_owned(),
        paths::RECENT.to_owned(),
        paths::SOURCES.to_owned(),
        paths::entries("home", ""),
        paths::document("any"),
        paths::file("any"),
        paths::asset("any", "ab.png"),
    ];
    for path in closed {
        let refused = stranger.get::<serde_json::Value>(&path).await.expect_err(&path);
        assert_eq!(refused.code, Code::SignedOut, "{path}");
    }
}

#[tokio::test]
async fn a_session_tells_who_signed_in() {
    let server = Server::new("who");
    let ben = server.person("ben", &["family"]);
    let person: Person = ben.get(paths::SESSION).await.unwrap();
    assert_eq!((person.username.as_str(), person.name.as_str()), ("ben", "Ben"));
}

#[tokio::test]
async fn the_apps_token_runs_out_and_is_renewed_once() {
    let server = Server::new("renew");
    let ben = server.person("ben", &[]);
    server.wait(HOUR + 1);
    let stale = ben.get::<Person>(paths::SESSION).await.expect_err("the token has run out");
    assert_eq!(stale.code, Code::Expired);

    let renewal = Renewal { renew: ben.renew.clone() };
    let next: Token = server.stranger().post(paths::RENEW, &renewal).await.expect("the renewal trades for a token");
    let renewed = common::Client { token: Some(next.token.clone()), renew: next.renew.clone(), ..ben.clone() };
    assert_eq!(renewed.get::<Person>(paths::SESSION).await.unwrap().username, "ben");

    // The renewal worked once, and what it renewed is gone for good.
    let again = server.stranger().post::<Token>(paths::RENEW, &renewal).await.expect_err("a renewal works once");
    assert_eq!(again.code, Code::SignedOut);
    assert_eq!(ben.get::<Person>(paths::SESSION).await.expect_err("the old token").code, Code::SignedOut);
}

#[tokio::test]
async fn a_sign_in_ends_whatever_is_renewed() {
    let server = Server::new("ends");
    let (app, browser) = (server.person("ben", &[]), server.browser("ben", &[]));
    server.wait(6 * DAY);
    assert!(browser.get::<Person>(paths::SESSION).await.is_ok(), "a browser's session lasts the sign-in through");
    let next: Token = server.stranger().post(paths::RENEW, &Renewal { renew: app.renew.clone() }).await.unwrap();
    assert!(next.expires <= next.ends, "a token never outlasts its sign-in");

    server.wait(DAY + 1);
    assert_eq!(browser.get::<Person>(paths::SESSION).await.expect_err("the sign-in ended").code, Code::SignedOut);
    let late = server.stranger().post::<Token>(paths::RENEW, &Renewal { renew: next.renew }).await;
    assert_eq!(late.expect_err("the sign-in ended").code, Code::SignedOut);
}

#[tokio::test]
async fn signing_out_ends_the_session_and_its_renewal() {
    let server = Server::new("sign-out");
    let ben = server.person("ben", &[]);
    ben.post::<()>(paths::SIGN_OUT, &()).await.expect("signing out");
    assert_eq!(ben.get::<Person>(paths::SESSION).await.expect_err("signed out").code, Code::SignedOut);
    let renewal = server.stranger().post::<Token>(paths::RENEW, &Renewal { renew: ben.renew.clone() }).await;
    assert_eq!(renewal.expect_err("signed out").code, Code::SignedOut);
}

#[tokio::test]
async fn a_cookie_changes_nothing_for_another_sites_page() {
    let server = Server::new("origin");
    let browser = server.browser("ben", &[]);
    assert_eq!(browser.get::<Vec<Space>>(paths::SPACES).await.unwrap().len(), 1, "reading needs no origin");

    let folder = |name: &str| NewFolder { parent: String::new(), name: name.into() };
    let cases = [(Some("https://evil.example"), false), (None, false), (Some(common::URL), true)];
    for (n, (origin, allowed)) in cases.into_iter().enumerate() {
        let from = common::Client { origin: origin.map(String::from), ..browser.clone() };
        let made = from.post::<letters_api::Entry>(&paths::folders("home"), &folder(&format!("Folder {n}"))).await;
        match allowed {
            true => assert!(made.is_ok(), "the server's own page may ask"),
            false => assert_eq!(made.expect_err("another site's page").code, Code::Forbidden, "{origin:?}"),
        }
    }
    assert_eq!(server.scratch.names("home/ben"), ["Folder 2"], "only what the server's own page asked for was done");
}

#[tokio::test]
async fn a_token_is_good_only_the_way_it_was_handed_out() {
    let server = Server::new("carrier");
    let browser = server.browser("ben", &[]);
    // The cookie's value, copied into the app's header.
    let copied = common::Client { carrier: Carrier::Bearer, ..browser.clone() };
    assert_eq!(copied.get::<Person>(paths::SESSION).await.expect_err("a copied cookie").code, Code::SignedOut);
    let garbled = common::Client { token: Some("not-a-token".into()), ..browser };
    let raw = garbled.send(Method::GET, paths::SESSION, "", Vec::new()).await;
    assert_eq!(raw.status, StatusCode::UNAUTHORIZED);
}
