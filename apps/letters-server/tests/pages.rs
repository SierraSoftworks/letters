//! The server's own pages: what a browser is shown, and what the forms on them do.

mod common;

use axum::http::StatusCode;
use common::{Browser, Server, URL, names};
use letters_api::{Entry, Listing, paths};
use letters_server::pages::{
    ACT_MOVE, ACT_NEW_DOCUMENT, ACT_NEW_FOLDER, ACT_PUT_BACK, ACT_RENAME, ACT_TRASH, RECENT_PAGE, TRASH_PAGE, asking,
};

/// What the folder at `path` of a space holds, as the interface tells the browser's own session.
async fn listed(browser: &Browser, space: &str, path: &str) -> Vec<Entry> {
    browser.pages().get::<Listing>(&paths::entries(space, path)).await.expect("the folder lists").entries
}

/// The page at `path`, which must be one.
async fn page(browser: &mut Browser, path: &str) -> String {
    let raw = browser.get(path).await;
    assert_eq!(raw.status, StatusCode::OK, "{path}: {}", raw.text());
    assert!(raw.header("content-type").starts_with("text/html"), "{path}");
    raw.text()
}

#[tokio::test]
async fn a_browser_with_no_session_is_shown_the_way_in() {
    let server = Server::new("way-in");
    let mut visitor = server.visitor();
    let front = page(&mut visitor, paths::HOME_PAGE).await;
    assert!(front.contains(&paths::sign_in_page(paths::HOME_PAGE)), "the front page offers to sign in");
    // A page deeper in comes back to itself once the person has signed in.
    for path in ["/s/family/House?ask=rename&id=x", RECENT_PAGE, TRASH_PAGE, "/d/some-id", "/d/some-id/read"] {
        let raw = visitor.get(path).await;
        assert_eq!(raw.location(), paths::sign_in_page(path), "{path}");
        assert!(!raw.text().contains("House"), "{path} tells nothing");
    }
    // A form has no page to come back to.
    let raw = visitor.fill(ACT_TRASH, &[("id", "x")]).await;
    assert_eq!(raw.location(), paths::sign_in_page(paths::HOME_PAGE));
}

#[tokio::test]
async fn the_front_page_leads_to_the_persons_first_space() {
    let server = Server::new("front");
    let mut ben = server.reader("ben", &["family"]);
    assert_eq!(ben.get(paths::HOME_PAGE).await.location(), paths::folder_page("home", ""));
    let own = page(&mut ben, &paths::folder_page("home", "")).await;
    for space in ["home", "family"] {
        assert!(own.contains(&format!("href=\"{}\"", paths::folder_page(space, ""))), "{space} is in the sidebar");
    }
    assert!(own.contains(paths::SIGN_OUT_PAGE), "the page offers to sign out");
}

#[tokio::test]
async fn a_folders_page_lists_it_and_offers_what_the_role_allows() {
    let server = Server::new("folder-page");
    let mut ben = server.reader("ben", &["family"]);
    ben.fill(ACT_NEW_FOLDER, &[("space", "family"), ("folder", ""), ("name", "R&D")]).await;
    let plan = ben.pages().start("family", "R&D", "Plan <b>").await;
    let (here, id) = (paths::folder_page("family", "R&D"), plan.id.clone().unwrap());

    let editors = page(&mut ben, &here).await;
    assert!(editors.contains("R&amp;D") && !editors.contains("R&D"), "names are written as text");
    assert!(editors.contains(&plan.name.replace('<', "&lt;").replace('>', "&gt;")), "{}", plan.name);
    assert!(editors.contains(&format!("href=\"{}\"", paths::editor_page(&id))), "the row opens the document");
    assert!(editors.contains(&format!("href=\"{}\"", paths::file(&id))), "the row offers the file");
    assert!(editors.contains(ACT_NEW_DOCUMENT) && editors.contains(ACT_NEW_FOLDER));

    // Each question opens over the folder, as a form that goes where the answer is taken.
    for (what, act) in [("rename", ACT_RENAME), ("move", ACT_MOVE), ("trash", ACT_TRASH)] {
        let ask = asking(&here, what, &id);
        assert!(editors.contains(&ask.replace('&', "&amp;")), "the row asks to {what}");
        assert!(!editors.contains(act), "no question is open until one is asked");
        assert!(page(&mut ben, &ask).await.contains(&format!("action=\"{act}\"")), "{what}");
    }

    let mut guest = server.reader("guest", &["friends"]);
    let readers = page(&mut guest, &here).await;
    assert!(readers.contains(&format!("href=\"{}\"", paths::read_page(&id))), "a reader can read it");
    for offer in [ACT_NEW_DOCUMENT, ACT_NEW_FOLDER, "ask=rename", "ask=trash"] {
        assert!(!readers.contains(offer), "a reader is not offered {offer}");
    }
    assert!(!page(&mut guest, &asking(&here, "trash", &id)).await.contains(ACT_TRASH), "nor by address");

    // What is out of reach is not there, and says nothing of what is.
    let mut stranger = server.reader("stranger", &[]);
    for path in [here.as_str(), "/s/family", "/s/nowhere"] {
        let raw = stranger.get(path).await;
        assert_eq!(raw.status, StatusCode::NOT_FOUND, "{path}");
        assert!(!raw.text().contains("Plan"), "{path}");
    }
}

#[tokio::test]
async fn the_forms_make_rename_move_and_throw_away() {
    let server = Server::new("forms");
    let mut ben = server.reader("ben", &["family"]);
    let top = paths::folder_page("family", "");

    let made = ben.fill(ACT_NEW_FOLDER, &[("space", "family"), ("folder", ""), ("name", "House")]).await;
    assert_eq!(made.location(), top, "the folder's page is shown again");
    let started = ben.fill(ACT_NEW_DOCUMENT, &[("space", "family"), ("folder", "House")]).await;
    let house = listed(&ben, "family", "House").await;
    let id = house[0].id.clone().expect("the document was made");
    assert_eq!(started.location(), paths::editor_page(&id), "a new document is opened");

    let here = paths::folder_page("family", "House");
    let renamed = ben.fill(ACT_RENAME, &[("id", &id), ("back", &here), ("name", "Roof")]).await;
    assert_eq!(renamed.location(), here);
    assert_eq!(names(&listed(&ben, "family", "House").await), ["Roof"]);

    // Where to is offered by the page, and taken as the page wrote it.
    let offered = page(&mut ben, &asking(&here, "move", &id)).await;
    assert!(offered.contains("value=\"home/\"") && offered.contains("value=\"family/\""), "each space's top");
    assert!(!offered.contains("value=\"family/House\""), "not the folder it is in");
    ben.fill(ACT_MOVE, &[("id", &id), ("back", &here), ("to", "family/")]).await;
    assert_eq!(names(&listed(&ben, "family", "").await), ["House", "Roof"]);

    ben.fill(ACT_TRASH, &[("id", &id), ("back", &top)]).await;
    assert_eq!(names(&listed(&ben, "family", "").await), ["House"]);
    assert!(page(&mut ben, TRASH_PAGE).await.contains("Roof"), "the trash lists it");
}

#[tokio::test]
async fn a_form_that_is_refused_says_so_on_its_page() {
    let server = Server::new("refused-form");
    let mut ben = server.reader("ben", &["family"]);
    let (plan, taken) = (ben.pages().start("family", "", "Plan").await, ben.pages().start("family", "", "Taken").await);
    let (here, id) = (paths::folder_page("family", ""), plan.id.unwrap());

    let refused = ben.fill(ACT_RENAME, &[("id", &id), ("back", &here), ("name", &taken.name)]).await;
    assert!(refused.location().starts_with(&format!("{here}?error=")), "{}", refused.location());
    assert!(page(&mut ben, refused.location()).await.contains("already there"), "the page says why");
    assert_eq!(names(&listed(&ben, "family", "").await), ["Plan", "Taken"]);

    // The page to show next is one of the server's own, whatever the form says.
    for elsewhere in ["https://elsewhere.example/", "//elsewhere.example/", "elsewhere"] {
        let raw = ben.fill(ACT_RENAME, &[("id", &id), ("back", elsewhere), ("name", "Plan B")]).await;
        assert_eq!(raw.location(), paths::HOME_PAGE, "{elsewhere}");
        ben.fill(ACT_RENAME, &[("id", &id), ("back", &here), ("name", "Plan")]).await;
    }
}

#[tokio::test]
async fn a_form_changes_nothing_unless_an_editor_sent_it_from_the_servers_pages() {
    let server = Server::new("foreign-form");
    let mut ben = server.reader("ben", &["family"]);
    let plan = ben.pages().start("family", "", "Plan").await;
    let (here, id) = (paths::folder_page("family", ""), plan.id.unwrap());
    let fields = [("id", id.as_str()), ("back", here.as_str())];

    for origin in [None, Some("https://elsewhere.example"), Some("null")] {
        let raw = ben.submit(ACT_TRASH, &fields, origin).await;
        assert_eq!(raw.status, StatusCode::FORBIDDEN, "{origin:?}");
    }
    let mut guest = server.reader("guest", &["friends"]);
    let refused = guest.submit(ACT_TRASH, &fields, Some(URL)).await;
    assert!(refused.location().contains("error=forbidden"), "{}", refused.location());
    assert_eq!(names(&listed(&ben, "family", "").await), ["Plan"]);
}

#[tokio::test]
async fn the_trash_puts_a_document_back_where_it_was() {
    // What happens to the folder meanwhile, and where the document is put back: its folder and its name.
    let cases = [("kept", "House", "Plan"), ("taken", "House", "Plan 2"), ("gone", "", "Plan")];
    for (meanwhile, folder, name) in cases {
        let server = Server::new(&format!("put-back-{meanwhile}"));
        let mut ben = server.reader("ben", &["family"]);
        ben.fill(ACT_NEW_FOLDER, &[("space", "family"), ("folder", ""), ("name", "House")]).await;
        let plan = ben.pages().start("family", "House", "Plan").await;
        ben.fill(ACT_TRASH, &[("id", plan.id.as_deref().unwrap()), ("back", "/")]).await;
        match meanwhile {
            "taken" => drop(ben.pages().start("family", "House", "Plan").await),
            "gone" => std::fs::remove_dir(server.scratch.0.join("family/House")).unwrap(),
            _ => {}
        }

        let trash = page(&mut ben, TRASH_PAGE).await;
        assert!(trash.contains("Plan") && trash.contains(ACT_PUT_BACK), "{meanwhile}: the trash offers it back");
        let mut guest = server.reader("guest", &["friends"]);
        assert!(!page(&mut guest, TRASH_PAGE).await.contains(ACT_PUT_BACK), "{meanwhile}: but not to a reader");

        let back = ben.fill(ACT_PUT_BACK, &[("space", "family"), ("file", "Plan.let")]).await;
        assert_eq!(back.location(), TRASH_PAGE, "{meanwhile}");
        let found = listed(&ben, "family", folder).await;
        let entry = found.iter().find(|entry| entry.name == name).unwrap_or_else(|| panic!("{meanwhile}: {found:?}"));
        assert_eq!(entry.id, plan.id, "{meanwhile}: it is the document that was thrown away");
        assert!(!page(&mut ben, TRASH_PAGE).await.contains(ACT_PUT_BACK), "{meanwhile}: the trash is empty again");
    }
}

#[tokio::test]
async fn a_document_reads_as_a_page_that_runs_nothing() {
    let server = Server::new("read");
    let mut ben = server.reader("ben", &["family"]);
    let plan = ben.pages().start("family", "", "Plan").await;
    let id = plan.id.unwrap();

    let raw = ben.get(&paths::read_page(&id)).await;
    assert_eq!(raw.status, StatusCode::OK);
    assert!(raw.text().contains("<title>Plan</title>"), "the page is the document, under its name");
    let policy = raw.header("content-security-policy");
    assert!(policy.contains("default-src 'none'") && !policy.contains("script-src"), "{policy}");
    assert_eq!(raw.header("cache-control"), "no-store");
    // Until the editor is part of the server, a document opens to read.
    assert_eq!(ben.get(&paths::editor_page(&id)).await.location(), paths::read_page(&id));
    assert!(page(&mut ben, RECENT_PAGE).await.contains("Plan"), "what was read is among the recent");

    let mut stranger = server.reader("stranger", &[]);
    let hidden = stranger.get(&paths::read_page(&id)).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert!(!hidden.text().contains("Plan"));
}

#[tokio::test]
async fn a_page_loads_only_the_servers_own_files() {
    let server = Server::new("statics");
    let mut ben = server.reader("ben", &["family"]);
    let raw = ben.get(&paths::folder_page("family", "")).await;
    let policy = raw.header("content-security-policy");
    for rule in ["default-src 'none'", "script-src 'self'", "form-action 'self'", "frame-ancestors 'none'"] {
        assert!(policy.contains(rule), "{rule} is in {policy}");
    }
    assert_eq!(raw.header("cache-control"), "no-store", "a page is for one person at one moment");
    // A browser told to name no referrer sends `Origin: null` with a form, which no form is taken with.
    assert_eq!(raw.header("referrer-policy"), "same-origin", "other sites are told nothing, the server its page");

    // Every file a page names is there, under an address a browser may keep for good.
    let text = raw.text();
    let named: Vec<&str> = text.split('"').filter(|part| part.starts_with("/static/")).collect();
    assert!(named.iter().any(|file| file.ends_with(".css")) && named.iter().any(|file| file.ends_with(".js")));
    for file in named {
        let raw = server.visitor().get(file).await;
        assert_eq!(raw.status, StatusCode::OK, "{file}");
        assert!(raw.header("cache-control").contains("immutable"), "{file}");
        assert!(!raw.body.is_empty(), "{file}");
        // Another build's address is another file's.
        let other = file.replacen("/static/", "/static/0", 1);
        assert_eq!(server.visitor().get(&other).await.status, StatusCode::NOT_FOUND, "{other}");
    }
}
