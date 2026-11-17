//! The server's own pages: what a browser is shown, and what the forms on them do.

mod common;

use axum::http::StatusCode;
use common::{Browser, START, Server, URL, follow, let_in, link, names, planned};
use letters_api::{CreatedLink, Entry, Listing, NewLink, Opened, Role, paths};
use letters_server::pages::{
    ACT_IMPORT, ACT_IMPORT_FILE, ACT_MERGE, ACT_MOVE, ACT_NEW_DOCUMENT, ACT_NEW_FOLDER, ACT_PUT_BACK, ACT_RENAME,
    ACT_REVOKE, ACT_SHARE, ACT_TRASH, ACT_UNSHARE, RECENT_PAGE, SHARED_PAGE, TRASH_PAGE, asking, asking_folder,
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
    let deeper =
        ["/s/family/House?ask=rename&id=x", RECENT_PAGE, SHARED_PAGE, TRASH_PAGE, "/d/some-id", "/d/some-id/read"];
    for path in deeper {
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
    for (what, act) in [("rename", ACT_RENAME), ("move", ACT_MOVE), ("trash", ACT_TRASH), ("share", ACT_SHARE)] {
        let ask = asking(&here, what, &id);
        assert!(editors.contains(&ask.replace('&', "&amp;")), "the row asks to {what}");
        assert!(!editors.contains(act), "no question is open until one is asked");
        assert!(page(&mut ben, &ask).await.contains(&format!("action=\"{act}\"")), "{what}");
    }

    let mut guest = server.reader("guest", &["friends"]);
    let readers = page(&mut guest, &here).await;
    assert!(readers.contains(&format!("href=\"{}\"", paths::read_page(&id))), "a reader can read it");
    for offer in [ACT_NEW_DOCUMENT, ACT_NEW_FOLDER, "ask=rename", "ask=trash", "ask=share"] {
        assert!(!readers.contains(offer), "a reader is not offered {offer}");
    }
    for (what, act) in [("trash", ACT_TRASH), ("share", ACT_SHARE)] {
        assert!(!page(&mut guest, &asking(&here, what, &id)).await.contains(act), "nor by address to {what}");
    }

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
async fn the_forms_rename_move_and_throw_away_a_folder_and_the_trash_puts_it_back() {
    let server = Server::new("folder-forms");
    // Ben is an owner of the space, and Carol edits in it.
    let (mut ben, mut carol) = (server.reader("ben", &["parents"]), server.reader("carol", &["family"]));
    let top = paths::folder_page("family", "");
    for name in ["House", "Garden"] {
        ben.fill(ACT_NEW_FOLDER, &[("space", "family"), ("folder", ""), ("name", name)]).await;
    }
    let id = ben.pages().start("family", "House", "Plan").await.id.unwrap();

    // A folder's row asks as a document's does. Whoever edits the space is offered all but to throw the
    // folder away, which is its owners', and a reader's row asks nothing.
    let (owners, editors) = (page(&mut ben, &top).await, page(&mut carol, &top).await);
    let readers = page(&mut server.reader("guest", &["friends"]), &top).await;
    for (what, act) in [("rename", ACT_RENAME), ("move", ACT_MOVE), ("trash", ACT_TRASH)] {
        let ask = asking_folder(&top, what, "House");
        assert!(owners.contains(&ask.replace('&', "&amp;")), "the row asks to {what}");
        assert!(page(&mut ben, &ask).await.contains(&format!("action=\"{act}\"")), "{what}");
        assert_eq!(editors.contains(&ask.replace('&', "&amp;")), what != "trash", "an editor, to {what}");
        assert!(!readers.contains(&format!("ask={what}")), "a reader is not offered to {what}");
    }

    let about = |folder: &'static str| [("space", "family"), ("folder", folder), ("back", top.as_str())];
    // Nor does the address of the question, or the form itself, throw a folder away for an editor.
    assert!(!page(&mut carol, &asking_folder(&top, "trash", "House")).await.contains(ACT_TRASH));
    carol.fill(ACT_TRASH, &about("House")).await;
    assert_eq!(names(&listed(&ben, "family", "").await), ["Garden", "House"], "the folder stays");
    let renamed = ben.fill(ACT_RENAME, &[&about("House")[..], &[("name", "Home")]].concat()).await;
    assert_eq!(renamed.location(), top);
    assert_eq!(names(&listed(&ben, "family", "").await), ["Garden", "Home"]);

    let offered = page(&mut ben, &asking_folder(&top, "move", "Home")).await;
    assert!(offered.contains("value=\"family/Garden\"") && offered.contains("value=\"home/\""), "other folders");
    assert!(!offered.contains("value=\"family/Home\""), "not itself");
    ben.fill(ACT_MOVE, &[&about("Home")[..], &[("to", "family/Garden")]].concat()).await;
    assert_eq!(names(&listed(&ben, "family", "Garden").await), ["Home"]);

    ben.fill(ACT_TRASH, &about("Garden/Home")).await;
    assert!(listed(&ben, "family", "Garden").await.is_empty());
    let trash = page(&mut ben, TRASH_PAGE).await;
    assert!(trash.contains("Home") && trash.contains(ACT_PUT_BACK), "the trash offers the folder back");

    // The folders were just looked through, for a document that is in the trash.
    assert!(ben.pages().get::<Opened>(&paths::document(&id)).await.is_err(), "in the trash");
    ben.fill(ACT_PUT_BACK, &[("space", "family"), ("file", "Home")]).await;
    // Its document is found by its id at once all the same, before anyone looks into the folder.
    let opened: Opened = ben.pages().get(&paths::document(&id)).await.expect("the document is back");
    assert_eq!(opened.path, "Garden/Home/Plan.let");
    assert!(!page(&mut ben, TRASH_PAGE).await.contains(ACT_PUT_BACK), "the trash is empty again");
}

#[tokio::test]
async fn the_share_dialog_shares_a_document_and_whoever_it_names_finds_it() {
    let server = Server::new("share-form");
    let (mut ada, mut ben) = (server.reader("ada", &[]), server.reader("ben", &[]));
    let id = ada.pages().start("home", "", "Plan").await.id.unwrap();
    let here = paths::folder_page("home", "");
    let ask = asking(&here, "share", &id);
    let about = [("id", id.as_str()), ("back", here.as_str())];

    // Who to is offered by the dialog, and taken as the dialog wrote it. It is shown again afterwards.
    assert!(page(&mut ada, &ask).await.contains("value=\"p:sub-ben\""), "whoever has signed in is offered");
    let shared = ada.fill(ACT_SHARE, &[about[0], about[1], ("to", "p:sub-ben"), ("role", "commenter")]).await;
    assert_eq!(shared.location(), ask);
    let dialog = page(&mut ada, &ask).await;
    assert!(dialog.contains(ACT_UNSHARE) && dialog.contains("Commenter"), "the dialog lists who it is shared with");

    // Ben finds it among what is shared with him, with whose it is, and is asked nothing about its place.
    let theirs = page(&mut ben, SHARED_PAGE).await;
    assert!(theirs.contains(&format!("href=\"{}\"", paths::editor_page(&id))) && theirs.contains("Ada"), "{theirs}");
    assert!(!theirs.contains("ask="), "a shared document is not his to move, throw away or share");
    let taken = ben.fill(ACT_UNSHARE, &[about[0], about[1], ("to", "p:sub-ben")]).await;
    assert!(taken.location().contains("error=forbidden"), "{}", taken.location());

    let taken = ada.fill(ACT_UNSHARE, &[about[0], about[1], ("to", "p:sub-ben")]).await;
    assert_eq!(taken.location(), ask);
    assert!(!page(&mut ben, SHARED_PAGE).await.contains("Plan"), "what is no longer shared is not listed");
}

#[tokio::test]
async fn the_share_dialog_lists_the_links_that_open_a_document_and_revokes_them() {
    let server = Server::linking("link-form");
    let (mut ada, mut ben) = (server.reader("ada", &[]), server.reader("ben", &[]));
    let id = ada.pages().start("home", "", "Plan").await.id.unwrap();
    let here = paths::folder_page("home", "");
    let ask = asking(&here, "share", &id);
    assert!(!page(&mut ada, &ask).await.contains(ACT_REVOKE), "a document with no link offers none to revoke");

    // One that is over is the access log's to remember: the dialog lists what still opens the document.
    let over = NewLink { expires: Some(START + 60), ..link("Builder", Role::Reader) };
    let news = [link("Accountant", Role::Commenter), over];
    let mut made = Vec::new();
    for new in &news {
        made.push(ada.pages().post::<CreatedLink>(&paths::links(&id), new).await.expect("her own document"));
    }
    server.wait(120);
    let dialog = page(&mut ada, &ask).await;
    assert!(dialog.contains("Accountant") && dialog.contains("Commenter"), "the link and what it lets its reader do");
    assert!(!dialog.contains("Builder"), "a link that is over is not listed");

    // Only whoever may share revokes, and then the link opens nothing and the dialog no longer has it.
    let about = [("id", id.as_str()), ("back", here.as_str()), ("link", made[0].link.id.as_str())];
    assert!(ben.fill(ACT_REVOKE, &about).await.location().contains("error="), "it is not Ben's to revoke");
    assert_eq!(follow(&mut server.visitor(), &made[0]).await.location(), paths::editor_page(&id));
    assert_eq!(ada.fill(ACT_REVOKE, &about).await.location(), ask);
    assert!(!page(&mut ada, &ask).await.contains("Accountant"), "a revoked link is not listed");
    assert_eq!(follow(&mut server.visitor(), &made[0]).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_share_dialog_tells_what_was_done_through_a_share_or_a_link() {
    let (server, ada, ben, id) = planned("activity-form").await;
    let mut owner = server.reader("ada", &[]);
    let ask = asking(&paths::folder_page("home", ""), "share", &id);
    let dialog = page(&mut owner, &ask).await;
    assert!(dialog.contains("Activity") && dialog.contains("Nobody has reached"), "a log with nothing in it says so");

    // Ben opens the document by his share. A link's reader opens it, and is refused its file.
    let _: Opened = ben.get(&paths::document(&id)).await.unwrap();
    server.wait(10);
    let (mut guest, _) = let_in(&server, &ada, &id, &link("Accountant", Role::Reader)).await;
    assert_eq!(guest.get(&paths::document(&id)).await.status, StatusCode::OK);
    server.wait(10);
    assert_eq!(guest.get(&paths::file(&id)).await.status, StatusCode::FORBIDDEN);

    // Each is told by who did what, the newest first, and a guest by the link they came by.
    let dialog = page(&mut owner, &ask).await;
    let told = ["Guest was refused a download", "Guest viewed the document", "Ben viewed the document"];
    let at: Vec<usize> =
        told.iter().map(|said| dialog.find(said).unwrap_or_else(|| panic!("{said}: {dialog}"))).collect();
    assert!(at.is_sorted(), "the newest comes first: {at:?}");
    assert!(dialog[at[0]..at[2]].contains("Accountant"), "a guest's row names their link");
    assert!(dialog.contains("Today"), "the rows are under their day");
    assert!(dialog.contains("Times are in "), "and a reader in another time zone is told whose times they are");
    assert!(!dialog.contains("before="), "a log this short is shown whole");
}

#[tokio::test]
async fn the_share_dialog_shows_a_long_log_a_page_at_a_time() {
    let (server, _, _, id) = planned("activity-pages").await;
    // Ben opens the document once more than a page holds, each long enough after the last to be a row of
    // its own. A session does not last that long, so each is a sign-in of its own.
    for _ in 0..=200 {
        let _: Opened = server.person("ben", &[]).get(&paths::document(&id)).await.unwrap();
        server.wait(600);
    }
    let mut owner = server.reader("ada", &[]);
    let rows = |dialog: &str| dialog.matches("Ben viewed the document").count();
    let ask = asking(&paths::folder_page("home", ""), "share", &id);
    let newest = page(&mut owner, &ask).await;
    let earlier = newest.split('"').find(|part| part.contains("before=")).expect("a full page offers what came before");
    let earlier = page(&mut owner, &earlier.replace("&amp;", "&")).await;
    assert_eq!(rows(&newest) + rows(&earlier), 201, "every row is on one page or the other");
    assert!(rows(&earlier) >= 1 && !earlier.contains("before="), "the last page offers nothing earlier");
    assert!(earlier.contains(&format!("href=\"{}\"", ask.replace('&', "&amp;"))), "and the way back to the newest");
}

#[tokio::test]
async fn a_conflict_copy_offers_its_merge_to_whoever_may_edit_and_nothing_else() {
    let server = Server::new("merge-form");
    let mut ben = server.reader("ben", &["family"]);
    let id = ben.pages().start("family", "", "Plan").await.id.unwrap();
    let (family, copy) = (server.scratch.0.join("family"), "Plan (conflicted copy 2026-11-15)");
    std::fs::copy(family.join("Plan.let"), family.join(format!("{copy}.let"))).unwrap();
    let here = paths::folder_page("family", "");

    let editors = page(&mut ben, &here).await;
    assert!(editors.contains(ACT_MERGE) && editors.contains("Merge conflicts"), "the copy's row offers the merge");
    // The copy has the id of its original, so a link or a menu on its row would act on the original.
    let opens = format!("href=\"{}\"", paths::editor_page(&id));
    // The original's row opens it by its name and twice from its menu.
    assert_eq!(editors.matches(&opens).count(), 3, "only the original's row opens the document");
    let mut guest = server.reader("guest", &["friends"]);
    let readers = page(&mut guest, &here).await;
    assert!(readers.contains(copy) && !readers.contains(ACT_MERGE), "a reader sees the copy and is offered nothing");

    let merged = ben.fill(ACT_MERGE, &[("id", &id), ("back", &here)]).await;
    assert_eq!(merged.location(), here);
    assert_eq!(names(&listed(&ben, "family", "").await), ["Plan"]);
    assert!(page(&mut ben, TRASH_PAGE).await.contains(copy), "the copy waits in the trash");
}

#[tokio::test]
async fn a_folders_page_takes_a_file_to_make_a_document_of_and_opens_it() {
    let server = Server::new("import-form");
    let mut ben = server.reader("ben", &["family"]);
    let (here, within) = (paths::folder_page("family", ""), [("space", "family"), ("folder", "")]);
    assert!(page(&mut ben, &here).await.contains(&format!("action=\"{ACT_IMPORT}\"")), "the page offers it");

    let made = ben.hand(ACT_IMPORT, &within, ("Roof.md", b"# Roof\n\nThe quote is due."), URL).await;
    let rows = listed(&ben, "family", "").await;
    assert_eq!(names(&rows), ["Roof"]);
    assert_eq!(made.location(), paths::editor_page(rows[0].id.as_deref().unwrap()), "the document is opened");

    // The page's script tells when the chosen file was last changed, and the document is as old.
    assert!(page(&mut ben, &here).await.contains("name=\"modified\""), "the form can say it");
    let dated = [("space", "family"), ("folder", ""), ("modified", "1623053350")];
    ben.hand(ACT_IMPORT, &dated, ("Old.md", b"Old words."), URL).await;
    let old = listed(&ben, "family", "").await.into_iter().find(|row| row.name == "Old").expect("it is listed");
    assert_eq!((old.created.as_deref(), old.modified), (Some("2021-06-07T08:09:10Z"), 1_623_053_350));

    // What is no file to import says so on the folder's page, and so does a form with no file.
    for (name, file) in [("Roof.pdf", &b"%PDF-1.7 \xff\xfe"[..]), ("", &b""[..])] {
        let refused = ben.hand(ACT_IMPORT, &within, (name, file), URL).await;
        assert!(refused.location().starts_with(&format!("{here}?error=")), "{name}: {}", refused.location());
    }
    // And a reader, or another site's page, brings nothing in.
    let mut guest = server.reader("guest", &["friends"]);
    assert!(!page(&mut guest, &here).await.contains(ACT_IMPORT), "a reader is not offered it");
    assert!(guest.hand(ACT_IMPORT, &within, ("Mine.md", b"Mine."), URL).await.location().contains("error=forbidden"));
    let forged = ben.hand(ACT_IMPORT, &within, ("Theirs.md", b"Theirs."), "https://elsewhere.example").await;
    assert_eq!(forged.status, StatusCode::FORBIDDEN);
    assert_eq!(names(&listed(&ben, "family", "").await), ["Old", "Roof"]);
}

#[tokio::test]
async fn a_folders_page_lists_a_file_that_is_no_document_yet_and_makes_one_of_it() {
    let server = Server::new("import-kept");
    let mut ben = server.reader("ben", &["family"]);
    ben.pages().start("family", "", "Plan").await;
    for name in ["Plan.md", "Roof.md"] {
        std::fs::write(server.scratch.0.join("family").join(name), "Words.").unwrap();
    }
    let here = paths::folder_page("family", "");
    let shown = page(&mut ben, &here).await;
    assert!(shown.contains("Also kept as Plan.md"), "the document's row names the file of its name");
    assert!(shown.contains("Roof.md") && !shown.contains("Also kept as Roof.md"), "the other has a row");
    // Only the one with no document is offered, and only to whoever writes there.
    assert_eq!(shown.matches(&format!("action=\"{ACT_IMPORT_FILE}\"")).count(), 1);
    let mut guest = server.reader("guest", &["friends"]);
    let read = page(&mut guest, &here).await;
    assert!(read.contains("Roof.md") && !read.contains(ACT_IMPORT_FILE), "a reader sees it and is not offered it");

    let made = ben.fill(ACT_IMPORT_FILE, &[("space", "family"), ("path", "Roof.md")]).await;
    let rows = listed(&ben, "family", "").await;
    let roof = rows.iter().find(|row| row.name == "Roof").expect("the document is made");
    assert_eq!(made.location(), paths::editor_page(roof.id.as_deref().unwrap()), "and opened");
    let refused = ben.fill(ACT_IMPORT_FILE, &[("space", "family"), ("path", "Gone.md")]).await;
    assert!(refused.location().starts_with(&format!("{here}?error=")), "{}", refused.location());
}

#[tokio::test]
async fn a_form_that_is_refused_says_so_on_its_page() {
    let server = Server::new("refused-form");
    let mut ben = server.reader("ben", &["family"]);
    let (plan, taken) = (ben.pages().start("family", "", "Plan").await, ben.pages().start("family", "", "Taken").await);
    let (here, id) = (paths::folder_page("family", ""), plan.id.unwrap());

    let refused = ben.fill(ACT_RENAME, &[("id", &id), ("back", &here), ("name", &taken.name)]).await;
    assert!(refused.location().starts_with(&format!("{here}?error=")), "{}", refused.location());
    assert!(page(&mut ben, refused.location()).await.contains("already exists"), "the page says why");
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
    // A server that was built with no editor opens a document to read.
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
    assert!(named.iter().any(|file| file.ends_with(".svg")), "the bar shows the icon");
    // The page has the icon of its tab and of a home screen, and the list of the icons a browser may show.
    assert!(policy.contains("manifest-src 'self'"), "{policy}");
    let linked = |rel: &str| {
        let link = text.split('<').find(|tag| tag.starts_with(&format!("link rel=\"{rel}\"")));
        link.and_then(|link| link.split('"').find(|part| part.starts_with("/static/")))
            .unwrap_or_else(|| panic!("{rel}"))
    };
    for (rel, kind) in [("icon", "image/x-icon"), ("apple-touch-icon", "image/png")] {
        assert_eq!(server.visitor().get(linked(rel)).await.header("content-type"), kind, "{rel}");
    }
    let manifest = linked("manifest");
    let list: serde_json::Value = serde_json::from_slice(&server.visitor().get(manifest).await.body).unwrap();
    let icons = list["icons"].as_array().expect("the list names icons");
    assert!(!icons.is_empty());
    for icon in icons {
        let file =
            manifest.rsplit_once('/').map(|(folder, _)| format!("{folder}/{}", icon["src"].as_str().unwrap())).unwrap();
        assert_eq!(server.visitor().get(&file).await.header("content-type"), "image/png", "{file}");
    }
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

#[tokio::test]
async fn a_folder_with_a_file_that_is_no_document_is_not_offered_to_the_trash() {
    let server = Server::new("kept-folder");
    let mut ben = server.reader("ben", &["parents"]);
    let top = paths::folder_page("family", "");
    ben.fill(ACT_NEW_FOLDER, &[("space", "family"), ("folder", ""), ("name", "House")]).await;
    std::fs::write(server.scratch.0.join("family/House/notes.txt"), b"kept").unwrap();

    // The question names the file and offers no way to throw the folder away, and neither does the form.
    let ask = asking_folder(&top, "trash", "House");
    let asked = page(&mut ben, &ask).await;
    assert!(asked.contains("notes.txt") && !asked.contains(&format!("action=\"{ACT_TRASH}\"")), "the file is named");
    let about = [("space", "family"), ("folder", "House"), ("back", top.as_str())];
    assert_eq!(ben.fill(ACT_TRASH, &about).await.location(), ask, "the form leads back to what keeps it");
    assert_eq!(names(&listed(&ben, "family", "").await), ["House"], "the folder stays");

    std::fs::remove_file(server.scratch.0.join("family/House/notes.txt")).unwrap();
    assert!(page(&mut ben, &ask).await.contains(&format!("action=\"{ACT_TRASH}\"")), "without it, it is asked");
}

#[tokio::test]
async fn a_row_has_a_menu_that_a_right_click_opens() {
    let server = Server::new("row-menu");
    let (mut ben, mut guest) = (server.reader("ben", &["family"]), server.reader("guest", &["friends"]));
    let top = paths::folder_page("family", "");
    ben.fill(ACT_NEW_FOLDER, &[("space", "family"), ("folder", ""), ("name", "House")]).await;
    let id = ben.pages().start("family", "", "Plan").await.id.unwrap();
    // Whoever only reads has the menu too: it opens the row.
    for reader in [&mut ben, &mut guest] {
        let shown = page(reader, &top).await;
        for (name, href) in [("House", paths::folder_page("family", "House")), ("Plan", paths::editor_page(&id))] {
            assert!(shown.contains(&format!("More for {name}")), "{name} has a menu");
            assert!(shown.matches(&format!("href=\"{href}\"")).count() >= 2, "{name}: the menu opens it");
        }
        // The script that opens the menu at the pointer is the server's own, and is served.
        let script =
            shown.split("src=\"").find_map(|rest| rest.split('"').next().filter(|src| src.ends_with("letters.js")));
        let script = reader.get(script.expect("the page loads the script")).await;
        assert_eq!(script.status, StatusCode::OK);
        assert!(script.text().contains("contextmenu"));
    }
}
