//! The language of a passage: said of a block or of a stretch of its text, and told back.

mod common;

use common::{Fake, Harness, parse, png};
use letters_editor::{Command, Editor, Lang};
use letters_model::{Body, Change, ReplicaId};

fn tag(lang: &str) -> Lang {
    Lang::Tag(lang.to_owned())
}

fn say(lang: &str) -> Command {
    Command::SetLang(Some(lang.to_owned()))
}

/// What each block of text says of its language, `-` for nothing, and after it each stretch of its text
/// that says its own.
fn said(editor: &Harness) -> Vec<String> {
    let text = editor.editor.doc().walk().filter(|block| matches!(block.body, Body::Text(_)));
    let stretch = |block: &letters_model::Block| -> String {
        let own = block.body.runs().iter().filter_map(|run| Some(format!(" {}={}", run.text, run.lang.as_ref()?)));
        own.collect()
    };
    text.map(|block| format!("{}{}", block.lang.as_deref().unwrap_or("-"), stretch(block))).collect()
}

/// An editor on `state`, in which the first block says it is written in `lang`.
fn within(state: &str, lang: &str) -> Harness {
    let (mut doc, selection) = parse(state);
    doc.blocks[0].lang = Some(lang.to_owned());
    let mut editor = Editor::new(doc, ReplicaId::from(1));
    selection.into_iter().for_each(|selection| editor.select(selection));
    Harness { editor, platform: Fake::default() }
}

#[test]
fn a_language_is_said_of_the_block_at_the_caret_and_of_the_text_that_is_selected() {
    let cases: [(&str, &[&str]); 7] = [
        // A caret says it of its block.
        ("p one|\np two", &["fr", "-"]),
        ("> p quo|ted\np two", &["fr", "-"]),
        ("fig# Boats| at sea", &["fr"]),
        // A selection says it of the text it holds, and of a block it holds whole.
        ("p one ^two| three", &["- two=fr"]),
        ("p ^one|\np two", &["fr", "-"]),
        ("p on^e\np two\np th|ree", &["- e=fr", "fr", "- th=fr"]),
        // Code has no language, and neither has a line with nothing selected in it.
        ("p ^one\ncode let a;\np\np |two", &["fr", "-", "-", "-"]),
    ];
    for (state, after) in cases {
        let mut editor = Harness::new(state);
        assert_eq!(editor.editor.lang(), Lang::Document, "{state:?} before");
        let shown = editor.state();

        let change = editor.run(say("fr"));
        assert_eq!(said(&editor), after, "{state:?}");
        assert_eq!(editor.editor.lang(), tag("fr"), "{state:?}");
        assert!(!change.blocks.is_empty(), "{state:?}: layout is told of the blocks");
        assert_eq!(editor.state(), shown, "{state:?}: the text and the selection stay");
        assert_eq!(editor.run(say("fr")), Change::default(), "{state:?}: said again, it changes nothing");

        assert!(editor.key("Mod+z"));
        assert!(said(&editor).iter().all(|block| block == "-"), "{state:?}: one step back takes all of it");
        assert!(!editor.editor.can_undo(), "{state:?} was one step");
    }
}

#[test]
fn a_block_that_takes_a_language_keeps_no_other_in_its_text() {
    let mut editor = Harness::new("p one ^two| three");
    editor.run(say("de"));
    assert_eq!(said(&editor), ["- two=de"]);

    editor.editor.select(parse("p one two th|ree").1.expect("a caret"));
    assert_eq!(editor.editor.lang(), Lang::Document);
    editor.run(say("fr"));
    assert_eq!(said(&editor), ["fr"]);

    // No tag, or an empty one, leaves it to the document again.
    for none in [None, Some("  ".to_owned())] {
        editor.run(say("fr"));
        editor.run(Command::SetLang(none.clone()));
        assert_eq!((said(&editor), editor.editor.lang()), (vec!["-".to_owned()], Lang::Document), "{none:?}");
    }
}

#[test]
fn the_language_in_force_is_told_for_the_caret_and_for_a_selection() {
    let mut editor = Harness::new("p one ^two| three\np four");
    editor.run(say("de"));
    let cases = [
        // A caret reads as text typed there would: after the character before it.
        ("p one two| three\np four", tag("de")),
        ("p one |two three\np four", Lang::Document),
        ("p one tw^o th|ree\np four", Lang::Mixed),
        ("p one two ^three\np fo|ur", Lang::Document),
        ("p ^one two three\np four|", Lang::Mixed),
    ];
    for (state, lang) in cases {
        editor.editor.select(parse(state).1.expect("a selection"));
        assert_eq!(editor.editor.lang(), lang, "{state:?}");
    }

    editor.editor.select(parse("p one two| three\np four").1.expect("a caret"));
    editor.type_text("x");
    assert_eq!(said(&editor), ["- twox=de", "-"], "what is typed takes the language it is typed in");
}

#[test]
fn a_block_reads_in_the_language_of_the_block_around_it() {
    let mut editor = within("> p quo|ted\n> p more\np after", "de");
    assert_eq!(editor.editor.lang(), tag("de"));

    editor.run(say("fr"));
    assert_eq!((said(&editor), editor.editor.lang()), (vec!["fr".to_owned(), "-".into(), "-".into()], tag("fr")));
    editor.run(Command::SetLang(None));
    assert_eq!(editor.editor.lang(), tag("de"), "with none of its own it is left to the quote's again");

    editor.editor.select(parse("> p quoted\n> p mo^re\np af|ter").1.expect("a selection"));
    assert_eq!(editor.editor.lang(), Lang::Mixed);
}

#[test]
fn a_language_is_said_only_where_there_is_text_to_speak_of() {
    let cases = [
        ("p one|", true),
        ("fig Boats| at sea", true),
        ("r1> td ce|ll", true),
        ("p ^one\ncode let a;|", true),
        ("code let a;|", false),
        ("formula a|", false),
        ("code ^let a;|", false),
        ("hr", false),
    ];
    for (state, takes) in cases {
        let mut editor = Harness::new(state);
        assert_eq!(editor.editor.applies(&say("fr")), takes, "{state:?}");
        let before = editor.editor.doc().clone();
        editor.run(say("fr"));
        assert_eq!(editor.editor.doc() != &before, takes, "{state:?}");
    }

    // A picture that is selected is no text, and one who only reads says nothing.
    let mut editor = Harness::new("p one|");
    editor.run(Command::InsertFigures { pictures: vec![png(1, 400, 300)], at: None });
    assert!(editor.editor.picture().is_some() && !editor.editor.applies(&say("fr")));
    let mut editor = Harness::new("p one|");
    editor.editor.set_read_only(true);
    assert!(!editor.editor.applies(&say("fr")));
}
