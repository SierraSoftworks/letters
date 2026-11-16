//! Finding entries by what a person types: letters in order anywhere in the path, the better stand
//! first, and filters leave out what does not pass them.

use letters_api::{Entry, EntryKind, Query, Role, Tag};

/// 1 November 2026, in seconds since 1970.
const NOVEMBER: u64 = 1_793_491_200;

fn entry(kind: EntryKind, path: &str) -> Entry {
    Entry {
        kind,
        name: path.rsplit('/').next().unwrap_or(path).trim_end_matches(".let").into(),
        space: Some("home".into()),
        path: path.into(),
        id: None,
        created: None,
        modified: NOVEMBER,
        role: Role::Editor,
        contributors: Vec::new(),
        tags: Vec::new(),
    }
}

fn document(path: &str) -> Entry {
    entry(EntryKind::Document, path)
}

/// A house's papers: who wrote each, what marks it, and how many days into November it was written.
fn papers() -> Vec<Entry> {
    let by = |path: &str, people: &[&str], tags: Vec<Tag>, day: u64| Entry {
        contributors: people.iter().map(|&name| name.to_owned()).collect(),
        tags,
        modified: NOVEMBER + day * 86_400,
        ..document(path)
    };
    vec![
        entry(EntryKind::Folder, "House"),
        entry(EntryKind::Folder, "House/Roof"),
        by("House/Roof/Roof quote.let", &["Sam Okafor", "Ada"], vec![Tag::Shared { with: 2 }], 0),
        by("House/Roof/Scaffold plan.let", &["Ada"], vec![Tag::Editing { people: vec!["Ada".into()] }], 8),
        by("House/Garden notes.let", &["Sam Okafor"], vec![Tag::ConflictCopy { of: "House/Garden.let".into() }], 14),
        by("Recipes/Proof of bread.let", &[], vec![Tag::Shared { with: 0 }], 20),
    ]
}

fn found(typed: &str, entries: &[Entry]) -> Vec<String> {
    Query::parse(typed).rank(entries).into_iter().map(|entry| entry.path.clone()).collect()
}

#[test]
fn letters_in_order_find_an_entry_anywhere_in_its_path() {
    let cases = [
        ("rq", true),
        ("hsrfqt", true),
        ("ROOF", true),
        ("house quote", true),
        ("quote house", true),
        // Out of order, or not there.
        ("qr", false),
        ("roofs", false),
        ("roof zebra", false),
        // The file's ending is not part of what is shown.
        (".let", false),
    ];
    let quote = [document("House/Roof/Roof quote.let")];
    for (typed, finds) in cases {
        assert_eq!(!found(typed, &quote).is_empty(), finds, "{typed:?}");
    }
}

#[test]
fn the_better_stand_is_listed_first() {
    // Each: what is typed, the path that comes first, and one that is found after it.
    let cases = [
        // Letters that run together, before letters apart.
        ("roof", "Roof.let", "Report of fees.let"),
        // The start of a word, before the middle of one.
        ("plan", "Plan.let", "Explanation.let"),
        ("sp", "Scaffold plan.let", "Wasps.let"),
        // The entry's own name, before a folder on its way.
        ("roof", "House/Roof.let", "Roof/House.let"),
        // Of two that stand as well, the nearer.
        ("roof", "Roof.let", "Archive/2019/Roof.let"),
    ];
    for (typed, first, second) in cases {
        for entries in [[document(first), document(second)], [document(second), document(first)]] {
            assert_eq!(found(typed, &entries), [first, second], "{typed:?}");
        }
    }
}

#[test]
fn a_filter_leaves_out_what_does_not_pass_it() {
    let cases: [(&str, &[&str]); 13] = [
        ("author:sam", &["House/Roof/Roof quote.let", "House/Garden notes.let"]),
        ("by:ADA", &["House/Roof/Roof quote.let", "House/Roof/Scaffold plan.let"]),
        ("author:\"sam okafor\" author:ada", &["House/Roof/Roof quote.let"]),
        ("is:folder", &["House", "House/Roof"]),
        ("is:shared", &["House/Roof/Roof quote.let"]),
        ("is:conflict", &["House/Garden notes.let"]),
        ("is:editing", &["House/Roof/Scaffold plan.let"]),
        ("after:2026-11-10", &["House/Garden notes.let", "Recipes/Proof of bread.let"]),
        ("before:2026-11-10 is:document", &["House/Roof/Roof quote.let", "House/Roof/Scaffold plan.let"]),
        ("after:2026-11-10 before:2026-11-11", &[]),
        // Beside words, a filter narrows what they find.
        ("roof author:ada is:document", &["House/Roof/Roof quote.let", "House/Roof/Scaffold plan.let"]),
        ("plan author:sam", &[]),
        ("author:nobody", &[]),
    ];
    let papers = papers();
    for (typed, wanted) in cases {
        let mut got = found(typed, &papers);
        got.sort_by_key(|path| papers.iter().position(|entry| entry.path == *path));
        assert_eq!(got, wanted, "{typed:?}");
    }
}

#[test]
fn what_is_no_filter_is_a_word() {
    let cases = [
        ("re:", "Re: the roof.let"),
        ("note:roof", "Note: roof.let"),
        ("is:leaking", "This: leaking.let"),
        ("before:long", "Before: long.let"),
        ("author:", "Author: unknown.let"),
    ];
    for (typed, path) in cases {
        assert_eq!(found(typed, &[document(path), document("Other.let")]), [path], "{typed:?}");
    }
}

#[test]
fn nothing_typed_finds_everything_as_it_came() {
    let papers = papers();
    for typed in ["", "   ", "\""] {
        let query = Query::parse(typed);
        assert!(query.is_empty(), "{typed:?}");
        assert_eq!(query.rank(&papers).len(), papers.len());
    }
    assert!(!Query::parse("author:sam").is_empty());
}
