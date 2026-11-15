mod common;

use common::{Do, Harness, canon, clip, png};
use letters_editor::{Command, Picture};
use letters_model::Preview;

const PAGE: &str = "https://a.example";
const LINK: &str = "[https://a.example](https://a.example)";
const FILLED: &str = "card https://a.example ~ A page ~ About it ~ assets/old.png";

fn said(text: &str) -> Option<String> {
    Some(text.to_owned())
}

/// What a fetch of an address found: a title, and maybe a picture.
fn fetched(url: &str, title: Option<&str>, picture: Option<Picture>) -> Command {
    Command::Fetched { url: url.into(), title: title.map(str::to_owned), description: None, picture }
}

/// An editor with an address on its clipboard, as another application leaves it.
fn copied(state: &str, text: &str) -> Harness {
    let mut editor = Harness::new(state);
    editor.platform.clip = Some(clip(text, None));
    editor
}

/// What the document keeps of the page at an address, for a link in the text to show.
fn kept<'a>(editor: &'a Harness, url: &str) -> Option<&'a Preview> {
    editor.editor.doc().previews.get(url)
}

#[test]
fn the_address_of_a_page_put_on_a_line_of_its_own_becomes_a_card() {
    let typed = [Do::T(PAGE), Do::K("Enter")];
    let pasted = [Do::K("Mod+v")];
    // The document, what is on the clipboard, what is done, and the document after it.
    let cases = [
        ("p one\np |", PAGE, &pasted[..], "p one\ncard https://a.example\np |"),
        ("p one\np |\np two", " https://a.example\n", &pasted, "p one\ncard https://a.example\np |\np two"),
        ("", PAGE, &pasted, "card https://a.example\np |"),
        ("> p |", PAGE, &pasted, "> card https://a.example\n> p |"),
        ("p one\np |\np two", "", &typed, "p one\ncard https://a.example\np |\np two"),
        ("p |", "", &typed, "card https://a.example\np |"),
    ];
    for (before, clipboard, script, after) in cases {
        let mut editor = copied(before, clipboard);
        editor.play(script);
        assert_eq!(editor.state(), canon(after), "{script:?} in {before:?}");
        // The card shows nothing of its page yet, so it is named once for the shell to fetch.
        assert_eq!(editor.editor.take_unfetched(), [PAGE], "{script:?} in {before:?}");
        assert!(editor.editor.take_unfetched().is_empty());
        editor.key("Mod+z");
        assert!(!editor.text().contains("card"), "undoing {script:?} in {before:?} left {}", editor.state());
    }
}

#[test]
fn an_address_put_among_text_is_a_link_there() {
    let (typed, spaced) = ([Do::T(PAGE), Do::K("Enter")], [Do::T("https://a.example. ")]);
    let (pasted, plain, enter) = ([Do::K("Mod+v")], [Do::K("Mod+Shift+v")], [Do::K("Enter")]);
    // The document, the clipboard, what is done, the document after it, and whether the page wants a fetch.
    let cases = [
        // Pasted among text, it is a link there.
        ("p see |", PAGE, &pasted[..], format!("p see {LINK}|"), true),
        ("ul |", PAGE, &pasted, format!("ul {LINK}|"), true),
        // Typed, it is one once Enter or a space follows it, without the stop that ends a sentence.
        ("p see |", "", &typed, format!("p see {LINK}\np |"), true),
        ("ul |", "", &typed, format!("ul {LINK}\nul |"), true),
        ("p see |", "", &spaced, format!("p see {LINK}. |"), true),
        // An address that leads to no page is a link on any line, with nothing to fetch.
        ("p |", "mailto:a@b.example", &pasted, "p [mailto:a@b.example](mailto:a@b.example)|".to_owned(), false),
        // Where text is taken as it is written, an address is only that.
        ("code |", PAGE, &pasted, format!("code {PAGE}|"), false),
        ("code |", "", &typed, format!("code {PAGE}⏎|"), false),
        ("p `co|de`", PAGE, &pasted, format!("p `co{PAGE}|de`"), false),
        ("p |", PAGE, &plain, format!("p {PAGE}|"), false),
        ("p |", "https://a.example\nnext", &plain, format!("p {PAGE}\np next|"), false),
        // A line that holds a link is as its writer made it, and Enter leaves it so.
        ("p [https://a.example|](https://a.example)", "", &enter, format!("p {LINK}\np |"), false),
    ];
    for (before, clipboard, script, after, fetch) in cases {
        let mut editor = copied(before, clipboard);
        editor.play(script);
        assert_eq!(editor.state(), canon(&after), "{script:?} in {before:?}");
        let named = editor.editor.take_unfetched();
        assert_eq!(named, if fetch { vec![PAGE] } else { Vec::new() }, "{script:?} in {before:?}");
    }

    // Backspace straight after gives back the address as it was typed.
    let mut editor = Harness::new("p see |");
    editor.play(&[Do::T("https://a.example "), Do::K("Backspace")]);
    assert_eq!(editor.state(), canon("p see https://a.example |"));
}

#[test]
fn where_no_page_can_be_fetched_an_address_stays_a_link_and_none_is_named() {
    let (typed, pasted) = ([Do::T(PAGE), Do::K("Enter")], [Do::K("Mod+v")]);
    for (script, after) in [(&typed[..], format!("p {LINK}\np |")), (&pasted, format!("p {LINK}|"))] {
        let mut editor = copied("p |", PAGE);
        editor.editor.set_preview_fetching(false);
        editor.play(script);
        assert_eq!(editor.state(), canon(&after), "{script:?}");
        // A card that is asked for is made all the same, and shows its address.
        editor.run(Command::InsertPreview(PAGE.into()));
        assert!(editor.text().contains("card https://a.example"), "{}", editor.state());
        assert!(editor.editor.take_unfetched().is_empty(), "{script:?}");
    }
}

#[test]
fn only_what_the_writer_puts_in_is_named_for_a_fetch() {
    // A link made by the link card or by shorthand is, and so is a card that is asked for.
    let mut editor = Harness::new("p see ^this| page");
    editor.run(Command::SetLink(said(PAGE)));
    editor.play(&[Do::K("End"), Do::T(" [docs](https://b.example)"), Do::K("Enter")]);
    editor.run(Command::InsertPreview("https://c.example".into()));
    assert_eq!(editor.editor.take_unfetched(), [PAGE, "https://b.example", "https://c.example"]);

    // An undo and a redo bring nothing new, and neither does what another writer put in.
    editor.play(&[Do::K("Mod+z"), Do::K("Mod+Shift+z")]);
    let mut other = editor.fork(2);
    other.play(&[Do::K("Mod+a")]);
    other.run(Command::SetLink(said("https://d.example")));
    assert_eq!(other.editor.take_unfetched(), ["https://d.example"]);
    assert!(editor.merge(&other) && editor.text().contains("https://d.example"));
    assert!(editor.editor.take_unfetched().is_empty());

    // Nor do the links inside text that is pasted, or a page that the document keeps something of.
    let mut editor = copied("p [kept](https://a.example)\np |", "see [docs](https://b.example) and more");
    editor.run(fetched(PAGE, Some("A page"), None));
    editor.key("Mod+v");
    editor.play(&[Do::T(" https://a.example ")]);
    assert_eq!(
        editor.state(),
        canon(&format!("p [kept](https://a.example)\np see [docs](https://b.example) and more {LINK} |"))
    );
    assert!(editor.editor.take_unfetched().is_empty());
}

#[test]
fn a_link_in_markdown_brings_the_title_of_its_page_and_takes_it_along() {
    let mut editor = copied("p |", "[docs](https://a.example \"A page\") and [more](https://b.example \"B\")");
    editor.key("Mod+v");
    assert_eq!(editor.state(), canon("p [docs](https://a.example) and [more](https://b.example)|"));
    assert_eq!(kept(&editor, PAGE).and_then(|kept| kept.title.clone()), said("A page"));

    // What the document keeps of a page already is not replaced by what pasted text says of it.
    editor.platform.clip = Some(clip("[again](https://a.example \"Another\")", None));
    editor.key("Mod+v");
    assert_eq!(kept(&editor, PAGE).and_then(|kept| kept.title.clone()), said("A page"));

    // A copy says the title where Markdown has a place for it.
    editor.play(&[Do::K("Mod+a"), Do::K("Mod+c")]);
    let copied = editor.platform.clip.as_ref().map(|clip| clip.text.as_str()).unwrap_or_default();
    assert!(copied.contains("(https://a.example \"A page\")"), "{copied}");
}

#[test]
fn backspace_after_a_card_turns_it_back_into_its_link() {
    // A card just made, and one that has been filled since.
    let mut editor = copied("p one\np |", PAGE);
    editor.key("Mod+v");
    editor.key("Backspace");
    assert_eq!(editor.state(), canon(&format!("p one\np {LINK}\np |")));
    assert_eq!(kept(&editor, PAGE), None, "the card showed nothing to keep");

    // The link is text from here on: Backspace comes to its line, and Enter does not make a card of it.
    editor.play(&[Do::K("Backspace"), Do::K("Enter")]);
    assert_eq!(editor.state(), canon(&format!("p one\np {LINK}\np |")));
    assert!(!editor.text().contains("card"));

    let before = format!("p one\n{FILLED}\np |two");
    let mut editor = Harness::new(&before);
    editor.key("Backspace");
    assert_eq!(editor.state(), canon(&format!("p one\np {LINK}\np |two")));
    // What the card showed is kept for the link, with no new fetch.
    let (title, description, image) = (said("A page"), said("About it"), said("assets/old.png"));
    assert_eq!(kept(&editor, PAGE), Some(&Preview { title, description, image, ..Preview::default() }));
    editor.key("Mod+z");
    assert_eq!(editor.state(), canon(&before), "one undo step");
    assert_eq!(kept(&editor, PAGE), None);
}

#[test]
fn a_link_alone_on_its_line_and_a_card_are_each_shown_as_the_other_on_request() {
    // The line, and the document once its link is a card: `None` where the line holds more than a link.
    let cases = [
        ("p one\np [docs|](https://a.example)\np two", Some("p one\ncard https://a.example\np |two")),
        ("p [do|cs](https://a.example)", Some("card https://a.example\np |")),
        ("> p [docs|](https://a.example)\nhr", Some("> card https://a.example\n> p |\nhr")),
        ("p see [docs|](https://a.example)", None),
        ("p [one](https://a.example)[two|](https://b.example)", None),
        ("ul [docs|](https://a.example)", None),
        ("p plain|", None),
    ];
    for (before, after) in cases {
        let mut editor = Harness::new(before);
        assert_eq!(editor.editor.applies(&Command::LinkAsCard), after.is_some(), "{before:?}");
        editor.run(Command::LinkAsCard);
        assert_eq!(editor.state(), canon(after.unwrap_or(before)), "{before:?}");
        editor.key("Mod+z");
        assert_eq!(editor.state(), canon(before), "undoing it in {before:?}");
    }

    // A card goes back and forth with all it shows.
    let before = format!("p one|\n{FILLED}");
    let mut editor = Harness::new(&before);
    for block in ["b1", "gone"] {
        assert!(!editor.editor.applies(&Command::CardAsLink(block.into())), "{block}");
    }
    editor.run(Command::CardAsLink("b2".into()));
    assert_eq!(editor.state(), canon(&format!("p one|\np {LINK}")));
    editor.play(&[Do::K("Down")]);
    editor.run(Command::LinkAsCard);
    assert_eq!(editor.state(), canon(&format!("p one\n{FILLED}\np |")));
}

#[test]
fn a_fetch_fills_all_that_the_document_shows_of_an_address() {
    let before = "p see [this](https://a.example) and [that](https://b.example)|\ncard https://a.example\ncard https://b.example ~ B";
    let mut editor = Harness::new(before);
    let picture = png(3, 400, 300);
    editor.run(fetched(PAGE, Some(" A\n  page "), Some(picture.clone())));

    // The card that leads there shows it, and the document keeps it for the link in the text.
    let found = kept(&editor, PAGE).cloned().expect("the document keeps what was found");
    let image = found.image.clone().expect("with the page's picture");
    assert_eq!(found.title, said("A page"));
    assert_eq!(editor.platform.assets.get(&image), Some(&picture.bytes), "the shell was handed the bytes");
    let after = format!(
        "p see [this](https://a.example) and [that](https://b.example)|\ncard https://a.example ~ A page ~ - ~ {image}\ncard https://b.example ~ B"
    );
    assert_eq!(editor.state(), canon(&after));
    assert_eq!(kept(&editor, "https://b.example"), None, "another address is not touched");

    // Not taken: the same again, and what was found at an address nothing leads to.
    for command in [fetched(PAGE, Some("A page"), Some(picture)), fetched("https://c.example", Some("C"), None)] {
        assert!(!editor.editor.applies(&command), "{command:?}");
    }

    // What a later fetch did not find is shown no longer, and it is one undo step.
    editor.run(fetched(PAGE, None, None));
    assert_eq!((editor.state(), kept(&editor, PAGE)), (canon(before), None));
    editor.key("Mod+z");
    assert_eq!((editor.state(), kept(&editor, PAGE)), (canon(&after), Some(&found)));
}

#[test]
fn a_preview_outlives_its_link_and_fills_the_card_that_is_made_for_its_address() {
    let mut editor = copied("p see [this](https://a.example)\np |", PAGE);
    editor.run(fetched(PAGE, Some("A page"), None));
    // The card shows what the document keeps of the page, so there is nothing to fetch.
    editor.key("Mod+v");
    assert_eq!(editor.state(), canon("p see [this](https://a.example)\ncard https://a.example ~ A page\np |"));
    assert!(editor.editor.take_unfetched().is_empty());

    // Taking the link away leaves what was kept, which undo may want again. A fetch for it is too late.
    let mut editor = Harness::new("p see [th|is](https://a.example)");
    editor.run(fetched(PAGE, Some("A page"), None));
    editor.run(Command::SetLink(None));
    assert_eq!(kept(&editor, PAGE).and_then(|kept| kept.title.clone()), said("A page"));
    assert!(!editor.editor.applies(&fetched(PAGE, Some("Another"), None)));
}
