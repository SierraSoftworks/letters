mod common;

use common::{Harness, canon};
use letters_editor::{Command, Query, Seek};
use letters_model::Selection;

fn plain(text: &str) -> Query {
    Query { text: text.into(), ..Query::default() }
}

fn regex(text: &str) -> Query {
    Query { text: text.into(), regex: true, ..Query::default() }
}

/// What each place a query was found at holds, in the order they were found.
fn found(editor: &Harness, query: &Query) -> Vec<String> {
    let doc = editor.editor.doc();
    let places = editor.editor.find(query).expect("the query can be looked for");
    let held = |at: &Selection| {
        assert_eq!(at.anchor.block, at.head.block, "nothing found runs from one block into the next");
        doc.block(&at.anchor.block).expect("its block is there").plain_text()[at.anchor.offset..at.head.offset]
            .to_owned()
    };
    places.iter().map(held).collect()
}

const PAGE: &str = "h1 The Cat\np a cat, a Cat and a concatenation\n> ul cat⏎dog\nr1> th cat\nr1> td 42 cats";

#[test]
fn a_query_finds_its_text_through_the_whole_document() {
    let editor = Harness::new(PAGE);
    let cases = [
        (plain("cat"), vec!["Cat", "cat", "Cat", "cat", "cat", "cat", "cat"]),
        (Query { case: true, ..plain("cat") }, vec!["cat", "cat", "cat", "cat", "cat"]),
        (Query { word: true, ..plain("cat") }, vec!["Cat", "cat", "Cat", "cat", "cat"]),
        (plain("a c"), vec!["a c", "a C", "a c"]),
        // The signs of a pattern are only characters until patterns are asked for.
        (plain("c.t"), vec![]),
        (regex("c.t"), vec!["Cat", "cat", "Cat", "cat", "cat", "cat", "cat"]),
        (regex(r"\d+ \w+"), vec!["42 cats"]),
        // Each line of a block has its own ends.
        (regex("^cat$"), vec!["cat", "cat"]),
        (regex("dog$"), vec!["dog"]),
        (plain("unicorn"), vec![]),
        (plain(""), vec![]),
        // A pattern that can hold nothing finds nothing where it would.
        (regex("x*"), vec![]),
    ];
    for (query, places) in cases {
        assert_eq!(found(&editor, &query), places, "{query:?}");
    }
}

#[test]
fn text_the_page_draws_something_else_for_is_not_found() {
    let editor = Harness::new("fig# sum of parts\np see ⟦sum 1⟧(b0) and $sum$ and sum\nformula sum");
    assert_eq!(found(&editor, &plain("sum")), ["sum", "sum"], "the caption and the words, not the source");
}

#[test]
fn a_pattern_that_does_not_read_is_refused_in_a_few_words() {
    let editor = Harness::new(PAGE);
    for pattern in ["(cat", "[a-", "a{2", r"\"] {
        let refused = editor.editor.find(&regex(pattern)).expect_err("the pattern does not read");
        assert!(!refused.0.is_empty() && !refused.0.contains('\n'), "{pattern:?} is refused as {:?}", refused.0);
        assert!(editor.editor.find(&plain(pattern)).is_ok(), "{pattern:?} is fine as characters");
    }
}

#[test]
fn seeking_goes_round_the_places_from_the_selection() {
    // The selection, and which of the three places is here, next and before.
    let cases = [
        ("p |one cat two cat\np cat", [0, 0, 2]),
        ("p one ^cat| two cat\np cat", [0, 1, 2]),
        ("p one c|at two cat\np cat", [1, 1, 2]),
        ("p one cat two ^cat|\np cat", [1, 2, 0]),
        ("p one cat two cat\np ^cat|", [2, 0, 1]),
        ("p one cat two cat\np cat|", [0, 0, 2]),
    ];
    for (state, [here, next, before]) in cases {
        let editor = Harness::new(state);
        let places = editor.editor.find(&plain("cat")).expect("the query can be looked for");
        let seek = |seek| editor.editor.seek(&places, seek);
        assert_eq!(seek(Seek::Here), Some(here), "here from {state:?}");
        assert_eq!(seek(Seek::Next), Some(next), "next from {state:?}");
        assert_eq!(seek(Seek::Previous), Some(before), "before from {state:?}");
    }
    let editor = Harness::new("p |nothing");
    assert_eq!(editor.editor.seek(&[], Seek::Next), None);
}

/// Replaces what `query` finds: at the place counted from the first, or with `None` at every place.
fn replace(state: &str, query: &Query, with: &str, only: Option<usize>) -> Harness {
    let mut editor = Harness::new(state);
    let places = editor.editor.find(query).expect("the query can be looked for");
    let only = only.map(|at| places[at].clone());
    editor.run(Command::Replace { query: query.clone(), with: with.into(), only });
    editor
}

#[test]
fn replacing_puts_words_in_place_of_what_was_found() {
    // The document, the query, what replaces, which place, and the document after.
    let cases = [
        ("p one |cat two cat\np cat", plain("cat"), "dog", None, "p one |dog two dog\np dog"),
        ("p one cat two cat\np cat|", plain("cat"), "", None, "p one  two \np |"),
        ("p a ^cat| b", plain("cat"), "lion", None, "p a ^lion| b"),
        ("p cats |and cats", plain("cat"), "x", None, "p xs |and xs"),
        ("p one cat two cat|", plain("cat"), "dog", Some(1), "p one cat two dog|"),
        ("p |one cat two cat", plain("cat"), "dog", Some(0), "p one dog| two cat"),
        // What is put in takes the formatting of what it replaces.
        ("p a **cat** and a [cat](x)|", plain("cat"), "dog", None, "p a **dog** and a [dog](x)|"),
        ("code let cat = 1;|", plain("cat"), "dog", None, "code let dog = 1;|"),
        // A pattern's groups are filled in, and a sign for one is only a sign without patterns.
        ("p Ada Lovelace|", regex(r"(\w+) (\w+)"), "$2, $1", None, "p Lovelace, Ada|"),
        ("p 12 and 7|", regex(r"(?<n>\d+)"), "${n}px", None, "p 12px and 7px|"),
        ("p cost|", plain("cost"), "$1", None, "p \\$1|"),
        ("p Cat cat|", Query { case: true, ..plain("cat") }, "dog", None, "p Cat dog|"),
    ];
    for (before, query, with, only, after) in cases {
        let editor = replace(before, &query, with, only);
        assert_eq!(editor.state(), canon(after), "{query:?} with {with:?} in {before:?}");
    }
}

#[test]
fn replacing_everywhere_is_one_step_to_undo() {
    let before = "p one |cat two cat\n> ul cat";
    let mut editor = replace(before, &plain("cat"), "dog", None);
    assert_eq!(editor.text(), canon("p one dog two dog\n> ul dog"));
    editor.run(Command::Undo);
    assert_eq!(editor.state(), canon(before));
    editor.run(Command::Redo);
    assert_eq!(editor.text(), canon("p one dog two dog\n> ul dog"));
}

#[test]
fn a_replacement_with_nothing_to_replace_does_nothing() {
    let state = "p one |cat";
    let mut editor = Harness::new(state);
    let elsewhere = editor.editor.selection().cloned();
    // A query that finds nothing, one that does not read, and a place that is not one it finds.
    let cases = [(plain("dog"), None), (regex("(cat"), None), (plain(""), None), (plain("cat"), elsewhere)];
    for (query, only) in cases {
        let command = Command::Replace { query, with: "x".into(), only };
        assert!(!editor.editor.applies(&command), "{command:?}");
        editor.run(command);
        assert_eq!(editor.state(), canon(state));
    }
    assert!(!editor.editor.can_undo());
}
