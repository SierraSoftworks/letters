//! The Servers dialog: where the person is signed in, signing in to another server through the system's
//! browser, and what the window asks of the shell.

use letters_test::{Found, Harness, Role};
use letters_ui::{Access, Request, Server, SignIn};

const HOME: &str = "https://letters.example.org";
const WORK: &str = "https://docs.example.net";
const PAGE: &str = "https://id.example.org/authorize?client=letters";
const ASKING: &str = "Asking the server how to sign in.";
const FAILED: &str = "letters.example.org did not answer. Check the address and that you are online.";

fn server(address: &str, account: &str) -> Server {
    Server { address: address.into(), account: account.into() }
}

/// A window whose shell signs in to servers, and has signed in to these.
fn signed_in(servers: Vec<Server>) -> Harness {
    let mut window = Harness::doc("# Roof plan\n\nThe roofer needs the scaffold up by 3 March.");
    window.ui().set_servers(servers);
    window
}

fn open(window: &mut Harness) {
    window.click(Role::Button, "More").click(Role::MenuItem, "Servers").assert_open(Role::Dialog, "Servers");
}

/// The browser shows the sign-in page, as a shell says it.
fn waiting() -> Option<SignIn> {
    Some(SignIn::Waiting { name: "letters.example.org".into(), page: PAGE.into() })
}

/// The servers the dialog lists, as a screen reader says them.
fn rows(window: &mut Harness) -> Vec<String> {
    window.controls().into_iter().filter(|found| found.role == Role::ListItem).map(|found| found.name).collect()
}

fn has(window: &mut Harness, role: Role, name: &str) -> bool {
    window.find(role, name).is_some()
}

/// The field for a server's address and the button beside it.
fn asks(window: &mut Harness) -> (Found, Found) {
    let field = window.find(Role::TextInput, "Server address").expect("the field");
    (field, window.find(Role::Button, "Sign in").expect("the button"))
}

/// Something a test does to the window.
type Does = fn(&mut Harness);

#[test]
fn the_dialog_lists_where_the_person_is_signed_in_and_signs_them_out() {
    let mut window = signed_in(vec![server(HOME, "Sam Okafor"), server(WORK, "S. Okafor")]);
    open(&mut window);
    let listed = ["letters.example.org, signed in as Sam Okafor", "docs.example.net, signed in as S. Okafor"];
    assert_eq!(rows(&mut window), listed);
    assert_eq!(window.ui().take_requests(), [], "the dialog shows what the shell has said");

    // Each server has its own way out, and is gone at once.
    let about = |found: &Found| found.role == Role::Button && found.description.as_deref() == Some("docs.example.net");
    let out = window.controls().into_iter().find(about).expect("a Sign out for each");
    assert_eq!(out.name, "Sign out");
    window.click_at(out.center());
    assert_eq!(window.ui().take_requests(), [Request::SignOut { server: WORK.into() }]);
    assert_eq!(rows(&mut window), listed[..1]);

    // What the shell then says stands.
    window.ui().set_servers(Vec::new());
    assert!(rows(&mut window).is_empty());
    for close in ["Done", "Close"] {
        window.click(Role::Button, close).assert_closed(Role::Dialog, "Servers");
        open(&mut window);
    }
    window.key("Escape").assert_closed(Role::Dialog, "Servers");
}

#[test]
fn the_keyboard_stays_in_the_dialog_when_the_row_it_was_on_goes() {
    let mut window = signed_in(vec![server(HOME, "Sam Okafor")]);
    open(&mut window);
    for _ in 0..6 {
        if window.focused().is_some_and(|found| found.name == "Sign out") {
            break;
        }
        window.key("Tab");
    }
    window.key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::SignOut { server: HOME.into() }]);

    // What is typed next reaches neither the page under the dialog nor a button nobody chose.
    window.type_text("x").key("Enter").key("Backspace");
    window.assert_markdown("# Roof plan\n\nThe roofer needs the scaffold up by 3 March.");
    let dialog = window.find(Role::Dialog, "Servers").expect("the dialog stays");
    let next = window.key("Tab").focused().expect("Tab goes on in the dialog");
    assert!(dialog.bounds.contains(next.center()), "{next:?}");
}

#[test]
fn a_server_is_signed_in_to_through_the_browser() {
    // What is typed, how it is sent, and the server the shell is asked to sign in to.
    let cases: [(&str, Does, Option<&str>); 4] = [
        ("letters.example.org", |window| _ = window.key("Enter"), Some("letters.example.org")),
        (" https://letters.example.org ", |window| _ = window.click(Role::Button, "Sign in"), Some(HOME)),
        ("", |window| _ = window.key("Enter"), None),
        ("  ", |window| _ = window.key("Enter"), None),
    ];
    for (typed, send, asked) in cases {
        let mut window = signed_in(Vec::new());
        open(&mut window);
        let focused = window.focused().expect("the dialog takes the keyboard");
        assert_eq!((focused.role, focused.name.as_str()), (Role::TextInput, "Server address"), "{typed:?}");
        assert!(asks(&mut window).1.disabled, "{typed:?}: there is no server to ask yet");
        window.type_text(typed);
        send(&mut window);
        let expected: Vec<Request> =
            asked.map(|server| Request::SignIn { server: server.into() }).into_iter().collect();
        assert_eq!(window.ui().take_requests(), expected, "{typed:?}");
        assert_eq!(has(&mut window, Role::Status, ASKING), asked.is_some(), "{typed:?}: the wait is said at once");
    }

    let mut window = signed_in(Vec::new());
    open(&mut window);
    window.type_text("letters.example.org").key("Enter");
    window.ui().take_requests();
    // The server is asked once, however long it takes and whatever is pressed meanwhile.
    let (field, button) = asks(&mut window);
    assert!(field.disabled && button.disabled, "nothing else is asked while the sign-in is under way");
    window.click_at(button.center()).key("Enter");
    assert_eq!(window.ui().take_requests(), []);

    // The browser has the sign-in page. Its address can be carried to a browser that did not open.
    window.ui().set_sign_in(waiting());
    assert!(!has(&mut window, Role::Status, ASKING));
    let said = window.controls().into_iter().find(|found| found.role == Role::Status).expect("the wait is said");
    assert!(said.name.contains("letters.example.org") && said.name.contains("browser"), "{}", said.name);
    window.click(Role::Button, "Copy the page\u{2019}s address");
    assert_eq!(window.platform.clipboard.as_ref().map(|clip| clip.text.as_str()), Some(PAGE));
    assert!(has(&mut window, Role::Button, "Copied"), "the button says that it is there");
    assert_eq!(window.ui().take_requests(), [], "the address is the window's to hand over");

    // Signed in: the server is listed, and the field is ready for another.
    window.ui().set_servers(vec![server(HOME, "Sam Okafor")]);
    window.ui().set_sign_in(None);
    window.assert_open(Role::Dialog, "Servers");
    assert_eq!(rows(&mut window), ["letters.example.org, signed in as Sam Okafor"]);
    let (field, _) = asks(&mut window);
    assert_eq!((field.value.unwrap_or_default().as_str(), field.disabled), ("", false));
    assert!(!window.controls().into_iter().any(|found| found.role == Role::Status), "nothing is waited for");

    // The next sign-in page has not been copied.
    window.click(Role::TextInput, "Server address").type_text("docs.example.net").key("Enter");
    window.ui().set_sign_in(waiting());
    assert!(has(&mut window, Role::Button, "Copy the page\u{2019}s address"));
}

#[test]
fn a_sign_in_under_way_can_be_given_up() {
    // How far the sign-in has come when it is given up.
    for state in [None, waiting()] {
        let mut window = signed_in(Vec::new());
        open(&mut window);
        assert!(!has(&mut window, Role::Button, "Cancel"), "there is nothing to give up yet");
        window.type_text("letters.example.org").key("Enter");
        if state.is_some() {
            window.ui().set_sign_in(state.clone());
        }
        window.ui().take_requests();
        window.click(Role::Button, "Cancel");
        assert_eq!(window.ui().take_requests(), [Request::CancelSignIn], "{state:?}");
        assert!(!window.controls().into_iter().any(|found| found.role == Role::Status), "{state:?}");
        assert!(!has(&mut window, Role::Button, "Cancel"), "{state:?}");
        // The address stays for another try, and the dialog stays for the person who opened it.
        let (field, button) = asks(&mut window);
        assert_eq!(field.value.as_deref(), Some("letters.example.org"), "{state:?}");
        assert!(!field.disabled && !button.disabled, "{state:?}");
        // The shell's word that nothing is under way changes nothing more.
        window.ui().set_sign_in(None);
        assert_eq!(asks(&mut window).0.value.as_deref(), Some("letters.example.org"), "{state:?}");
        window.click(Role::TextInput, "Server address").key("Enter");
        assert_eq!(window.ui().take_requests(), [Request::SignIn { server: "letters.example.org".into() }]);
    }
}

#[test]
fn a_sign_in_that_failed_says_why_until_something_else_is_typed() {
    let mut window = signed_in(Vec::new());
    open(&mut window);
    window.type_text("letters.example.org").key("Enter");
    window.ui().take_requests();
    window.ui().set_sign_in(Some(SignIn::Failed(FAILED.into())));
    window.assert_open(Role::Alert, FAILED).assert_open(Role::Dialog, "Servers");
    assert!(!has(&mut window, Role::Button, "Cancel"), "nothing is under way");
    let (field, button) = asks(&mut window);
    assert_eq!(field.value.as_deref(), Some("letters.example.org"), "the address stays to be put right");
    assert!(!field.disabled && !button.disabled);
    assert_eq!(window.ui().take_requests(), [], "the window does not try again by itself");

    // The keyboard is back in the field.
    let focused = window.focused().expect("the field takes the keyboard back");
    assert_eq!((focused.role, focused.name.as_str()), (Role::TextInput, "Server address"));
    window.key("End").type_text("g");
    window.assert_closed(Role::Alert, FAILED);
    window.key("Enter");
    assert_eq!(window.ui().take_requests(), [Request::SignIn { server: "letters.example.orgg".into() }]);
}

#[test]
fn a_sign_in_the_shell_began_shows_while_it_lasts() {
    // How the sign-in ends, and whether the dialog that came out for it then stays.
    let ends: [(&str, Does, bool); 3] = [
        ("done", |window| window.ui().set_sign_in(None), false),
        ("given up", |window| _ = window.click(Role::Button, "Cancel"), false),
        ("failed", |window| window.ui().set_sign_in(Some(SignIn::Failed(FAILED.into()))), true),
    ];
    for (how, end, stays) in ends {
        let mut window = signed_in(vec![server(HOME, "Sam Okafor")]);
        // A session that ended is begun again by the shell, with nobody at the dialog.
        window.ui().set_sign_in(waiting());
        window.assert_open(Role::Dialog, "Servers");
        assert!(has(&mut window, Role::Button, "Cancel"), "{how}");
        end(&mut window);
        assert_eq!(has(&mut window, Role::Dialog, "Servers"), stays, "{how}");
        assert_eq!(has(&mut window, Role::Alert, FAILED), stays, "{how}");
    }

    // Someone who put the dialog away while it waits is not shown it again as the sign-in goes on.
    let mut window = signed_in(Vec::new());
    window.ui().set_sign_in(Some(SignIn::Finding));
    window.click(Role::Button, "Done").assert_closed(Role::Dialog, "Servers");
    window.ui().set_sign_in(waiting());
    window.assert_closed(Role::Dialog, "Servers");
    assert_eq!(window.ui().take_requests(), [], "putting it away gives nothing up");
    // It is there to look at, and stays for whoever opened it.
    open(&mut window);
    assert!(has(&mut window, Role::Button, "Cancel"));
    window.ui().set_sign_in(None);
    window.assert_open(Role::Dialog, "Servers");
}

#[test]
fn only_a_shell_that_signs_in_to_servers_offers_the_dialog() {
    let offered = |window: &mut Harness| {
        window.click(Role::Button, "More");
        let menu = has(window, Role::MenuItem, "Servers");
        window.key("Escape").key("Mod+Shift+P").type_text("servers");
        let listed =
            window.controls().into_iter().any(|found| found.role == Role::ListBoxOption && found.name == "Servers");
        window.key("Escape");
        (menu, listed)
    };
    // A browser's window is its server's, and says nothing of servers.
    let mut window = Harness::doc("# Roof plan");
    assert_eq!(offered(&mut window), (false, false));
    window.key("Mod+Shift+P").type_text("servers").key("Enter").assert_closed(Role::Dialog, "Servers");

    // Signed in nowhere is still a shell that signs in, and someone who only reads signs in as well.
    for access in [Access::Edits, Access::Comments, Access::Reads] {
        let mut window = signed_in(Vec::new());
        window.ui().set_access(access);
        assert_eq!(offered(&mut window), (true, true), "{access:?}");
        window.key("Mod+Shift+P").type_text("servers").key("Enter").assert_open(Role::Dialog, "Servers");
        assert!(rows(&mut window).is_empty(), "{access:?}");
    }
}

#[test]
fn a_dialog_that_is_asked_for_takes_the_place_of_another() {
    let dialogs = ["Servers", "Settings", "Share"];
    let pairs = dialogs.iter().flat_map(|first| dialogs.iter().map(move |second| (*first, *second)));
    for (first, second) in pairs.filter(|(first, second)| first != second) {
        let mut window = signed_in(Vec::new());
        // The commands are reached by their key from under a dialog too.
        for dialog in [first, second] {
            window.key("Mod+Shift+P").type_text(&dialog.to_lowercase()).key("Enter");
        }
        let out: Vec<&str> = dialogs.into_iter().filter(|dialog| has(&mut window, Role::Dialog, dialog)).collect();
        assert_eq!(out, [second], "{first}, then {second}");
    }

    // Print view and the history take the window, and a dialog does not stay over them.
    for view in ["Print view", "History"] {
        let mut window = signed_in(Vec::new());
        open(&mut window);
        window.key("Mod+Shift+P").type_text(&view.to_lowercase()).key("Enter");
        window.assert_closed(Role::Dialog, "Servers");
    }
}
