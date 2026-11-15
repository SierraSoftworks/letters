//! Merging other files of the document into it: the bar about conflict copies, the card that names the
//! files first, and the two commands. The merge itself is the shell's.

use letters_test::{Found, Harness, Role};
use letters_ui::{MergeAsk, MergeFile, Request};

/// A file beside the letter, as a shell names it.
fn file(name: &str) -> MergeFile {
    MergeFile {
        path: format!("/home/sam/House/{name}.let"),
        name: name.to_owned(),
        folder: "Family / House".to_owned(),
    }
}

/// The conflict copies a sync tool left, the first `count` of them.
fn copies(count: usize) -> Vec<MergeFile> {
    ["Roof quote (conflict 1)", "Roof quote (conflict 2)", "Roof quote (conflict 3)"][..count]
        .iter()
        .map(|name| file(name))
        .collect()
}

/// A letter in a file, in a shell that merges: it has said that there are no conflict copies.
fn letter() -> Harness {
    let mut window = Harness::doc("Roof first, then the gutters.");
    window.ui().set_conflict_copies(Vec::new());
    window
}

/// The bar that speaks of merging, whatever it says.
fn bar(window: &mut Harness) -> Option<Found> {
    let says = |found: &Found| found.name.contains("conflict cop") || found.role == Role::Alert;
    window.controls().into_iter().find(|found| matches!(found.role, Role::Status | Role::Alert) && says(found))
}

fn page(window: &mut Harness) -> Found {
    window.controls().into_iter().find(|found| found.role == Role::Document).expect("a page")
}

/// Opens the palette, types and takes what is picked.
fn run(window: &mut Harness, typed: &str) {
    window.key("Mod+Shift+P").type_text(typed).key("Enter");
}

/// Whether the palette offers a command by this name.
fn offered(window: &mut Harness, name: &str) -> bool {
    window.key("Mod+Shift+P").type_text(name);
    let listed = window.find(Role::ListBoxOption, name).is_some();
    window.key("Escape");
    listed
}

const CONFLICTS: Request = Request::MergeConflicts { confirmed: false };

#[test]
fn conflict_copies_show_one_button_that_asks_the_shell_to_merge_them() {
    // How many copies the shell tells of, and what the bar names them by.
    let cases = [(1, "Roof quote (conflict 1)"), (3, "3 conflict copies")];
    for (count, names) in cases {
        let mut window = letter();
        assert!(bar(&mut window).is_none(), "a document without copies has no bar");
        let whole = page(&mut window).bounds;

        window.ui().set_conflict_copies(copies(count));
        let told = bar(&mut window).expect("a bar");
        assert!(told.name.contains(names), "{count}: {}", told.name);
        let under = page(&mut window).bounds;
        assert!(under.y >= told.bounds.y + told.bounds.height, "{count}: the page starts under the bar");
        assert_eq!(under.y + under.height, whole.y + whole.height, "{count}: and ends where it did");

        window.click(Role::Button, "Merge conflicts");
        assert_eq!(window.ui().take_requests(), [CONFLICTS], "{count}");
        // Nothing is merged twice while the shell is at it.
        window.click(Role::Button, "Merge conflicts");
        assert_eq!(window.ui().take_requests(), [], "{count}: the button waits for the shell's answer");
        assert!(!offered(&mut window, "Merge conflicts"), "{count}: and so does the palette");

        window.ui().set_conflict_copies(Vec::new());
        assert!(bar(&mut window).is_none(), "{count}: the copies are merged");
        assert_eq!(page(&mut window).bounds, whole, "{count}: the page has its room back");
    }
}

#[test]
fn the_shell_has_the_files_named_before_it_merges_them_and_a_yes_asks_again() {
    // What the shell asks about, the button that answers, and what the shell is asked then.
    let cases = [
        (true, 2, "Merge", vec![Request::MergeConflicts { confirmed: true }]),
        (true, 2, "Cancel", vec![]),
        (
            false,
            1,
            "Merge",
            vec![Request::MergeFrom { path: Some(file("Roof quote (conflict 1)").path), confirmed: true }],
        ),
        (false, 1, "Cancel", vec![]),
    ];
    for (conflicts, count, answer, asked) in cases {
        let mut window = letter();
        window.ui().set_conflict_copies(copies(if conflicts { count } else { 0 }));
        window.ui().ask_merge(MergeAsk { files: copies(count), conflicts });
        for copy in copies(count) {
            let named = window
                .controls()
                .into_iter()
                .find(|found| found.role == Role::ListItem && found.name.contains(&copy.name));
            let named = named.unwrap_or_else(|| panic!("{conflicts}: the card names {}", copy.name));
            assert!(named.name.contains(&copy.folder), "and says where it lies: {}", named.name);
        }
        let focused = window.focused().expect("something focused");
        assert_eq!((focused.role, focused.name.as_str()), (Role::Button, "Merge"), "Enter says yes");
        // The card is answered before another merge is asked for.
        for command in ["Merge conflicts", "Merge from file"] {
            assert!(!offered(&mut window, command), "{conflicts}: {command} while the card shows");
        }

        window.click(Role::Button, answer);
        assert_eq!(window.ui().take_requests(), asked, "{conflicts}, {answer}");
        assert!(window.find(Role::Button, "Cancel").is_none(), "{conflicts}, {answer}: the card is gone");
        // Copies that are being merged are not merged again from the bar. A no leaves them to be.
        let waits = window.find(Role::Button, "Merge conflicts").map(|button| button.disabled);
        assert_eq!(waits, conflicts.then_some(answer == "Merge"), "{conflicts}, {answer}");
    }
}

#[test]
fn a_card_with_no_file_to_name_does_not_show() {
    let mut window = letter();
    window.ui().ask_merge(MergeAsk::default());
    assert!(window.find(Role::Button, "Merge").is_none());
}

#[test]
fn the_palette_merges_only_what_the_shell_can() {
    // Before the shell has said which copies there are, it is not known to merge at all.
    let mut window = Harness::doc("Roof first");
    for command in ["Merge from file", "Merge conflicts"] {
        assert!(!offered(&mut window, command), "{command}");
    }

    window.ui().set_conflict_copies(Vec::new());
    assert!(!offered(&mut window, "Merge conflicts"), "there are none to merge");
    run(&mut window, "Merge from file");
    assert_eq!(window.ui().take_requests(), [Request::MergeFrom { path: None, confirmed: false }]);

    window.ui().set_conflict_copies(copies(1));
    run(&mut window, "Merge conflicts");
    assert_eq!(window.ui().take_requests(), [CONFLICTS]);
}

#[test]
fn a_merge_that_cannot_be_done_says_why_until_it_is_put_away() {
    const WHY: &str = "Roof quote (conflict 1) shares no history with this document, so it was not merged.";
    // How many copies there are when the shell refuses.
    for count in [0, 1] {
        let mut window = letter();
        window.ui().set_conflict_copies(copies(count));
        if count > 0 {
            window.click(Role::Button, "Merge conflicts");
        }
        window.ui().refuse_merge(WHY);
        let said = bar(&mut window).expect("a bar");
        assert_eq!((said.role, said.name.as_str()), (Role::Alert, WHY), "{count}");
        // The copies can be tried again from where it says so.
        let again = window.find(Role::Button, "Merge conflicts");
        assert_eq!(again.map(|button| button.disabled), (count > 0).then_some(false), "{count}");

        window.click(Role::Button, "Hide");
        assert!(bar(&mut window).is_none(), "{count}");
    }
}

#[test]
fn a_bar_that_was_put_away_stays_away_until_there_is_another_copy() {
    let mut window = letter();
    window.ui().set_conflict_copies(copies(1));
    window.click(Role::Button, "Hide");
    assert!(bar(&mut window).is_none());
    // A shell tells of the copies again with every save.
    window.ui().set_conflict_copies(copies(1));
    assert!(bar(&mut window).is_none(), "the same copy is no news");
    assert!(offered(&mut window, "Merge conflicts"), "and can still be merged from the palette");

    window.ui().set_conflict_copies(copies(2));
    assert!(bar(&mut window).is_some(), "another copy is");
}

#[test]
fn the_end_of_a_long_document_shows_whole_under_the_bar() {
    let mut window = Harness::doc(&"A line of the letter.\n\n".repeat(80));
    window.ui().set_conflict_copies(copies(1));
    window.key("Mod+End").paint();
    let (caret, page) = (window.ui().caret_rect().expect("the page shows a caret"), page(&mut window).bounds);
    assert!(caret.y + caret.height <= page.y + page.height, "the last line is in view: {caret:?} in {page:?}");
}

const INTO: Request = Request::MergeIntoOriginal { confirmed: false };

#[test]
fn a_document_that_is_a_conflict_copy_offers_to_merge_into_its_original() {
    let mut window = letter();
    assert!(!offered(&mut window, "Merge into the original"), "this document is no copy");

    window.ui().set_conflict_original(Some(file("Roof quote")));
    let told = bar(&mut window).expect("a bar");
    assert!(told.name.contains("Roof quote"), "it names the original: {}", told.name);
    window.click(Role::Button, "Merge into the original");
    assert_eq!(window.ui().take_requests(), [INTO]);
    // Nothing is merged twice while the shell is at it.
    window.click(Role::Button, "Merge into the original");
    assert_eq!(window.ui().take_requests(), [], "the button waits for the shell's answer");
    assert!(!offered(&mut window, "Merge into the original"), "and so does the palette");

    // The shell has the original named first, and a yes asks again.
    window.ui().ask_merge_into(file("Roof quote"));
    let named = window.controls().into_iter().find(|found| found.role == Role::ListItem);
    let named = named.expect("the card names the original").name;
    assert!(named.contains("Roof quote") && named.contains("Family / House"), "{named}");
    run(&mut window, "Merge into the original");
    assert_eq!(window.ui().take_requests(), [], "the card is answered first");
    window.click(Role::Button, "Merge");
    assert_eq!(window.ui().take_requests(), [Request::MergeIntoOriginal { confirmed: true }]);
    let waits = window.find(Role::Button, "Merge into the original").map(|button| button.disabled);
    assert_eq!(waits, Some(true), "the button waits again while the shell merges");

    // Put away, the bar is back when the shell says which document this is a copy of.
    window.click(Role::Button, "Hide");
    window.ui().set_conflict_original(Some(file("Roof quote")));
    assert!(bar(&mut window).is_none(), "the same original is no news");
    window.ui().set_conflict_original(Some(file("Roof quote, first draft")));
    assert!(bar(&mut window).is_some(), "another is");

    window.ui().set_conflict_original(None);
    assert!(bar(&mut window).is_none(), "the window holds the original now");
}

#[test]
fn a_conflict_copy_with_copies_of_its_own_is_merged_into_its_original_first() {
    let mut window = letter();
    window.ui().set_conflict_copies(copies(1));
    window.ui().set_conflict_original(Some(file("Roof quote")));
    assert!(window.find(Role::Button, "Merge conflicts").is_none());
    window.click(Role::Button, "Merge into the original");
    assert_eq!(window.ui().take_requests(), [INTO]);

    window.ui().set_conflict_original(None);
    window.click(Role::Button, "Merge conflicts");
    assert_eq!(window.ui().take_requests(), [CONFLICTS], "its copies come after");
}

#[test]
fn the_bar_says_what_was_merged_for_a_few_seconds() {
    // The names the shell says, and what the bar says of them.
    let cases: [(&[&str], Option<&str>); 3] = [
        (&["Roof quote (conflict 1)"], Some("Merged Roof quote (conflict 1)")),
        (&["a", "b", "c"], Some("Merged 3 files")),
        (&[], None),
    ];
    for (names, said) in cases {
        let mut window = letter();
        window.ui().set_merged(names.iter().map(|name| (*name).to_owned()).collect());
        let merged = window.controls().into_iter().find(|found| found.name.starts_with("Merged"));
        assert_eq!(merged.map(|found| found.name).as_deref(), said, "{names:?}");
        window.advance(0).advance(10_000);
        assert!(window.controls().iter().all(|found| !found.name.starts_with("Merged")), "{names:?}: it goes");
    }
}
