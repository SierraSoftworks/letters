//! Signing in at the identity provider: a browser, and the desktop app through a browser.

mod common;

use axum::http::StatusCode;
use common::idp::{Idp, Quirk, SECRET};
use common::{Browser, Client, Raw, Server, URL, query};
use letters_api::{Code, Exchange, Person, Role, Space, Token, paths};
use letters_server::ids::base64url;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// A server and the provider it signs people in at, with `oidc` as its other settings for it.
async fn pair(name: &str, oidc: &str) -> (Server, Idp) {
    let idp = Idp::start().await;
    let oidc = format!("client_secret = \"{SECRET}\"\n{oidc}");
    (Server::with_provider(name, &idp.issuer, &oidc), idp)
}

/// Ben, as the provider describes him: one of the family.
fn ben() -> Value {
    json!({ "sub": "u-ben", "preferred_username": "ben", "name": "Ben Example", "groups": ["family"] })
}

/// A person signs in: their browser goes to the provider, which agrees to `claims`, and comes back.
/// The answer to its coming back.
async fn sign_in(browser: &mut Browser, idp: &Idp, then: &str, claims: Value) -> Raw {
    let sent = browser.get(&paths::sign_in_page(then)).await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER, "{}", sent.text());
    let state = query(sent.location().split_once('?').unwrap().1)["state"].clone();
    let code = idp.approve(sent.location(), claims);
    browser.get(&format!("{}?code={code}&state={state}", paths::CALLBACK_PAGE)).await
}

/// Who the browser's session says it is. `None` when it has none.
async fn who(browser: &Browser) -> Option<Person> {
    browser.pages().get(paths::SESSION).await.ok()
}

/// The spaces a session reaches, and what it may do in each.
async fn reach(client: &Client) -> Vec<(String, Role)> {
    let spaces: Vec<Space> = client.get(paths::SPACES).await.unwrap();
    spaces.into_iter().map(|space| (space.id, space.role)).collect()
}

/// Whether a cookie is one only this server is sent, over TLS, out of reach of a page's scripts.
fn kept_safe(cookie: &str) -> bool {
    ["HttpOnly", "Secure", "SameSite=Lax"].iter().all(|rule| cookie.split("; ").any(|part| part == *rule))
}

#[tokio::test]
async fn a_browser_signs_in_at_the_provider_and_lands_where_it_was_going() {
    let (server, idp) = pair("signin", "").await;
    let mut browser = server.visitor();

    let sent = browser.get(&paths::sign_in_page("/s/family/Plans")).await;
    assert_eq!(sent.status, StatusCode::SEE_OTHER);
    let asked = query(sent.location().split_once('?').unwrap().1);
    assert_eq!(asked["code_challenge"].len(), 43, "the hash of a secret the server keeps");
    assert!(asked["state"].len() >= 43 && asked["nonce"].len() >= 43, "nobody guesses these");
    assert!(sent.cookies().iter().all(|cookie| kept_safe(cookie)), "{:?}", sent.cookies());

    let code = idp.approve(sent.location(), ben());
    let back = browser.get(&format!("{}?code={code}&state={}", paths::CALLBACK_PAGE, asked["state"])).await;
    assert_eq!(back.location(), "/s/family/Plans");
    assert!(back.cookies().iter().all(|cookie| kept_safe(cookie)), "{:?}", back.cookies());

    let person = who(&browser).await.expect("the browser is signed in");
    assert_eq!((person.id.as_str(), person.username.as_str(), person.name.as_str()), ("u-ben", "ben", "Ben Example"));
    let expected = [("home".to_owned(), Role::Owner), ("family".to_owned(), Role::Editor)];
    assert_eq!(reach(&browser.pages()).await, expected, "the provider's groups open the spaces");

    // Signed in, the browser has nothing to do at the provider.
    assert_eq!(browser.get(&paths::sign_in_page("/d/abc")).await.location(), "/d/abc");
}

#[tokio::test]
async fn a_person_is_read_from_the_claims_the_settings_name() {
    let renamed = "username_claim = \"username\"\ngroups_claim = \"roles\"";
    // The settings, what the provider does, what it says, and who signs in: username, name, in the family.
    let cases = [
        ("", Quirk::None, ben(), ("ben", "Ben Example", true)),
        ("", Quirk::None, json!({ "sub": "1", "preferred_username": "ada", "groups": "family" }), ("ada", "ada", true)),
        (
            "",
            Quirk::None,
            json!({ "sub": "2", "preferred_username": "linus", "name": "Linus" }),
            ("linus", "Linus", false),
        ),
        (
            renamed,
            Quirk::None,
            json!({ "sub": "3", "username": "grace", "preferred_username": "other", "name": "Grace", "roles": ["x", "family"] }),
            ("grace", "Grace", true),
        ),
        ("", Quirk::ClaimsOnRequest, ben(), ("ben", "Ben Example", true)),
        ("", Quirk::NameOnRequest, ben(), ("ben", "Ben Example", true)),
        ("", Quirk::SecretInForm, ben(), ("ben", "Ben Example", true)),
    ];
    for (i, (settings, quirk, claims, (username, name, family))) in cases.into_iter().enumerate() {
        let (server, idp) = pair(&format!("claims-{i}"), settings).await;
        idp.quirk(quirk);
        let mut browser = server.visitor();
        let back = sign_in(&mut browser, &idp, "/", claims).await;
        assert_eq!(back.location(), "/", "case {i}: {}", back.text());
        let person = who(&browser).await.expect("signed in");
        assert_eq!((person.username.as_str(), person.name.as_str()), (username, name), "case {i}");
        let reached = reach(&browser.pages()).await.iter().any(|(space, _)| space == "family");
        assert_eq!(reached, family, "case {i}");
    }
}

#[tokio::test]
async fn a_person_the_provider_does_not_name_is_not_signed_in() {
    let (server, idp) = pair("nameless", "").await;
    let mut browser = server.visitor();
    let back = sign_in(&mut browser, &idp, "/", json!({ "sub": "u-x", "name": "Nameless" })).await;
    assert_eq!(back.status, StatusCode::FORBIDDEN);
    assert!(back.text().contains("preferred_username"), "the page says what is missing: {}", back.text());
    assert_eq!(who(&browser).await, None);
}

#[tokio::test]
async fn a_token_that_is_not_the_providers_answer_to_this_sign_in_is_refused() {
    let quirks = [
        Quirk::OtherAudience,
        Quirk::OtherIssuer,
        Quirk::RunOut,
        Quirk::OtherNonce,
        Quirk::UnlistedKey,
        Quirk::SignedWithSecret,
        Quirk::OtherParty,
    ];
    let (server, idp) = pair("tokens", "").await;
    for quirk in quirks {
        idp.quirk(quirk);
        let mut browser = server.visitor();
        let back = sign_in(&mut browser, &idp, "/", ben()).await;
        assert_eq!(back.status, StatusCode::BAD_GATEWAY, "{quirk:?}: {}", back.text());
        assert_eq!(who(&browser).await, None, "{quirk:?}");
    }
    // The same provider, behaving, signs the same person in.
    idp.quirk(Quirk::None);
    let mut browser = server.visitor();
    assert_eq!(sign_in(&mut browser, &idp, "/", ben()).await.location(), "/");
}

#[tokio::test]
async fn the_provider_may_change_its_keys() {
    let (server, idp) = pair("keys", "").await;
    let mut before = server.visitor();
    assert_eq!(sign_in(&mut before, &idp, "/", ben()).await.location(), "/");

    idp.change_keys();
    server.wait(61);
    let mut after = server.visitor();
    let back = sign_in(&mut after, &idp, "/", ben()).await;
    assert_eq!(back.location(), "/", "{}", back.text());
    assert!(who(&before).await.is_some(), "and who was signed in stays so");
}

#[tokio::test]
async fn a_sign_in_ends_only_in_the_browser_that_started_it_and_once() {
    let (server, idp) = pair("states", "").await;
    let callback = |code: &str, state: &str| format!("{}?code={code}&state={state}", paths::CALLBACK_PAGE);

    let mut browser = server.visitor();
    let sent = browser.get(paths::SIGN_IN_PAGE).await;
    let state = query(sent.location().split_once('?').unwrap().1)["state"].clone();
    let code = idp.approve(sent.location(), ben());

    // Someone else's browser, handed the address the sign-in comes back to.
    let mut other = server.visitor();
    assert_eq!(other.get(&callback(&code, &state)).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(who(&other).await, None);
    // A sign-in of its own does not make the borrowed address work either.
    other.get(paths::SIGN_IN_PAGE).await;
    assert_eq!(other.get(&callback(&code, &state)).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(who(&other).await, None);

    // The browser that started it comes back, once.
    assert_eq!(browser.get(&callback(&code, &state)).await.location(), "/");
    assert_eq!(browser.get(&callback(&code, &state)).await.status, StatusCode::BAD_REQUEST);
    assert!(who(&browser).await.is_some(), "coming back twice does not end what the first time began");
}

#[tokio::test]
async fn a_sign_in_left_waiting_or_turned_down_signs_nobody_in() {
    let (server, idp) = pair("waiting", "").await;

    let mut slow = server.visitor();
    let sent = slow.get(paths::SIGN_IN_PAGE).await;
    let state = query(sent.location().split_once('?').unwrap().1)["state"].clone();
    let code = idp.approve(sent.location(), ben());
    server.wait(11 * 60);
    let late = slow.get(&format!("{}?code={code}&state={state}", paths::CALLBACK_PAGE)).await;
    assert_eq!(late.status, StatusCode::BAD_REQUEST);
    assert_eq!(who(&slow).await, None);

    let mut unwilling = server.visitor();
    let sent = unwilling.get(paths::SIGN_IN_PAGE).await;
    let state = query(sent.location().split_once('?').unwrap().1)["state"].clone();
    let denied = unwilling.get(&format!("{}?error=access_denied&state={state}", paths::CALLBACK_PAGE)).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert_eq!(who(&unwilling).await, None);
}

#[tokio::test]
async fn signing_in_leads_nowhere_but_this_server() {
    let (server, idp) = pair("then", "").await;
    let cases = [
        ("/d/abc?x=1", "/d/abc?x=1"),
        ("https://evil.example/", "/"),
        ("//evil.example", "/"),
        ("/\\evil.example", "/"),
        ("javascript:alert(1)", "/"),
        ("", "/"),
    ];
    for (then, lands) in cases {
        let mut browser = server.visitor();
        assert_eq!(sign_in(&mut browser, &idp, then, ben()).await.location(), lands, "{then:?}");
    }
}

#[tokio::test]
async fn a_provider_that_cannot_be_asked_is_said_so() {
    // Nothing listens there.
    let server = Server::with_provider("unreachable", "http://127.0.0.1:1", "");
    let mut browser = server.visitor();
    let page = browser.get(paths::SIGN_IN_PAGE).await;
    assert_eq!(page.status, StatusCode::BAD_GATEWAY);
    assert!(page.text().contains("identity provider"), "{}", page.text());
}

/// What the desktop app makes before it opens a browser: its secret, and the page to open.
fn app_asks(port: u16) -> (String, String, String) {
    let verifier = base64url(&[7; 32]);
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    let page = paths::desktop_sign_in(&challenge, port, "app-state");
    (verifier, challenge, page)
}

/// The person agrees in a signed-in browser, and the code their browser carries to the app.
async fn agree(browser: &mut Browser, challenge: &str, port: u16) -> String {
    let fields = [("challenge", challenge), ("port", &port.to_string()), ("state", "app-state")];
    let agreed = browser.submit(paths::DESKTOP_PAGE, &fields, Some(URL)).await;
    let (listener, carried) = agreed.location().split_once('?').expect("the browser goes to the app");
    assert_eq!(listener, format!("http://127.0.0.1:{port}/"));
    let carried = query(carried);
    assert_eq!(carried["state"], "app-state", "the app knows its own request by it");
    carried["code"].clone()
}

#[tokio::test]
async fn the_app_signs_in_through_the_browser() {
    let (server, idp) = pair("desktop", "").await;
    let (verifier, challenge, page) = app_asks(49152);
    let mut browser = server.visitor();

    // The browser is not signed in: the person signs in first, and comes back to agree.
    assert_eq!(browser.get(&page).await.location(), paths::sign_in_page(&page));
    assert_eq!(sign_in(&mut browser, &idp, &page, ben()).await.location(), page);
    let asks = browser.get(&page).await;
    assert_eq!(asks.status, StatusCode::OK);
    assert!(asks.text().contains("Ben Example"), "the page says who the app would be signed in as");

    // Another site's page cannot agree for the person.
    let fields = [("challenge", challenge.as_str()), ("port", "49152"), ("state", "app-state")];
    for origin in [None, Some("https://evil.example")] {
        let forged = browser.submit(paths::DESKTOP_PAGE, &fields, origin).await;
        assert_eq!((forged.status, forged.location()), (StatusCode::FORBIDDEN, ""), "{origin:?}");
    }

    let code = agree(&mut browser, &challenge, 49152).await;
    let app = server.stranger();
    let exchange = Exchange { code, verifier };
    let token: Token = app.post(paths::EXCHANGE, &exchange).await.expect("the app's code and secret are good");
    assert_eq!(token.person.username, "ben");
    let signed = Client { token: Some(token.token), ..app.clone() };
    assert!(reach(&signed).await.contains(&("family".to_owned(), Role::Editor)), "the app has the person's groups");

    // A code is traded once.
    assert_eq!(app.post::<Token>(paths::EXCHANGE, &exchange).await.unwrap_err().code, Code::SignedOut);
}

#[tokio::test]
async fn a_code_is_no_use_without_the_apps_secret_or_after_a_minute() {
    let (server, idp) = pair("codes", "").await;
    let (verifier, challenge, _) = app_asks(50000);
    let mut browser = server.visitor();
    sign_in(&mut browser, &idp, "/", ben()).await;
    let app = server.stranger();

    // Whoever overheard the code does not hold the secret, and their try spends the code.
    let code = agree(&mut browser, &challenge, 50000).await;
    let guess = Exchange { code: code.clone(), verifier: base64url(&[8; 32]) };
    assert_eq!(app.post::<Token>(paths::EXCHANGE, &guess).await.unwrap_err().code, Code::SignedOut);
    let real = Exchange { code, verifier: verifier.clone() };
    assert_eq!(app.post::<Token>(paths::EXCHANGE, &real).await.unwrap_err().code, Code::SignedOut);

    let code = agree(&mut browser, &challenge, 50000).await;
    server.wait(61);
    let late = Exchange { code, verifier };
    assert_eq!(app.post::<Token>(paths::EXCHANGE, &late).await.unwrap_err().code, Code::SignedOut);
}

#[tokio::test]
async fn the_app_is_only_reached_where_an_app_listens() {
    let (server, idp) = pair("listeners", "").await;
    let (_, challenge, _) = app_asks(50000);
    let mut browser = server.visitor();
    sign_in(&mut browser, &idp, "/", ben()).await;
    let cases = [
        // A port only the system's own services listen on.
        (challenge.as_str(), "80", "app-state"),
        // No hash of a secret.
        ("short", "50000", "app-state"),
        // A state that would not stay in its place in an address.
        (challenge.as_str(), "50000", "a&b=c"),
        (challenge.as_str(), "", "app-state"),
    ];
    for (challenge, port, state) in cases {
        let page = format!("{}?challenge={challenge}&port={port}&state={}", paths::DESKTOP_PAGE, paths::segment(state));
        let asks = browser.get(&page).await;
        assert_eq!((asks.status, asks.location()), (StatusCode::BAD_REQUEST, ""), "{page}");
        let fields = [("challenge", challenge), ("port", port), ("state", state)];
        let agreed = browser.submit(paths::DESKTOP_PAGE, &fields, Some(URL)).await;
        assert_eq!((agreed.status, agreed.location()), (StatusCode::BAD_REQUEST, ""), "{page}");
    }
}

#[tokio::test]
async fn signing_out_ends_the_browsers_session_and_takes_its_cookie() {
    let (server, idp) = pair("signout", "").await;
    let mut browser = server.visitor();
    sign_in(&mut browser, &idp, "/", ben()).await;
    let pages = browser.pages();

    // Another site cannot sign the person out.
    browser.submit(paths::SIGN_OUT_PAGE, &[], Some("https://evil.example")).await;
    assert!(who(&browser).await.is_some());

    let left = browser.submit(paths::SIGN_OUT_PAGE, &[], Some(URL)).await;
    assert_eq!(left.location(), "/");
    assert!(browser.cookies.is_empty(), "{:?}", browser.cookies);
    assert_eq!(pages.get::<Person>(paths::SESSION).await.unwrap_err().code, Code::SignedOut, "the token is spent");
}
