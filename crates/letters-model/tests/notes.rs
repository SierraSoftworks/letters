use letters_model::{Anchor, Block, BlockKind, Note, NoteKind, Notes, Reply, Run, Version};
use serde_json::{Value, json};

fn note(id: &str, kind: NoteKind) -> Note {
    Note {
        id: id.into(),
        kind,
        anchor: Anchor::range("b2", 16..25),
        author: "Ana".into(),
        created: "2026-11-14T20:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Is this the final date?")],
        replies: Vec::new(),
        extra: Default::default(),
    }
}

fn version(author: &str, text: &str) -> Version {
    Version {
        author: author.into(),
        edited: "2026-11-14T20:05:00Z".into(),
        block: Block::text("b2", BlockKind::Paragraph, text),
        extra: Default::default(),
    }
}

/// One note of every kind, with a reply and a resolved state among them.
fn notes() -> Notes {
    let reply = Reply {
        author: "Ben".into(),
        created: "2026-11-14T20:10:00Z".into(),
        content: vec![Run::marked("Yes", [letters_model::Mark::Bold])],
        extra: Default::default(),
    };
    let versions = Box::new([version("Ana", "Rates change in January."), version("Ben", "Rates change in March.")]);
    Notes {
        notes: vec![
            Note { replies: vec![reply], resolved: true, ..note("n1", NoteKind::Comment) },
            Note { anchor: Anchor::block("b1"), author: "Letters import".into(), ..note("n2", NoteKind::Import) },
            Note { content: Vec::new(), ..note("n3", NoteKind::Conflict { versions }) },
        ],
        extra: Default::default(),
    }
}

/// A `notes.json` holding one valid comment, changed by `edit`.
fn with(edit: impl Fn(&mut Value)) -> String {
    let mut note = json!({ "id": "n", "type": "comment", "anchor": { "block": "b" }, "author": "Ana",
        "created": "2026-11-14T20:00:00Z" });
    edit(&mut note);
    json!({ "format": 1, "notes": [note] }).to_string()
}

#[test]
fn every_kind_of_note_survives_the_round_trip() {
    let notes = notes();
    let written = notes.to_json();
    let reread = Notes::from_json(&written).unwrap();
    assert_eq!(reread, notes);
    assert_eq!(reread.to_json(), written);
}

#[test]
fn a_document_without_notes_reads_as_empty() {
    assert_eq!(Notes::from_json(r#"{"format":1}"#).unwrap(), Notes::default());
    assert_eq!(Notes::from_json(&Notes::default().to_json()).unwrap(), Notes::default());
}

#[test]
fn unknown_content_survives() {
    let json = with(|note| {
        note["type"] = "reaction".into();
        note["emoji"] = "tada".into();
        note["anchor"]["stable"] = json!([4, 2]);
    });
    let notes = Notes::from_json(&json).unwrap();
    assert_eq!(notes.notes[0].kind, NoteKind::Unknown { type_name: "reaction".into() });
    let written: Value = serde_json::from_str(&notes.to_json()).unwrap();
    assert_eq!(written["notes"][0]["type"], "reaction");
    assert_eq!(written["notes"][0]["emoji"], "tada");
    assert_eq!(written["notes"][0]["anchor"]["stable"], json!([4, 2]));
}

#[test]
fn schema_breaks_are_refused_with_their_path() {
    let remove = |key: &'static str| move |note: &mut Value| drop(note.as_object_mut().unwrap().remove(key));
    let set = |key: &'static str, value: Value| move |note: &mut Value| note[key] = value.clone();
    let one_version = json!([{ "author": "Ana", "edited": "2026", "block": { "id": "b", "type": "divider" } }]);
    let mut twice: Value = serde_json::from_str(&with(|_| ())).unwrap();
    let copy = twice["notes"][0].clone();
    twice["notes"].as_array_mut().unwrap().push(copy);
    let twice = twice.to_string();
    let cases = [
        ("[]".to_string(), ""),
        (r#"{"notes":[]}"#.to_string(), "format"),
        (r#"{"format":2}"#.to_string(), "format"),
        (r#"{"format":1,"notes":{}}"#.to_string(), "notes"),
        (with(remove("id")), "notes[0].id"),
        (with(remove("type")), "notes[0].type"),
        (with(remove("anchor")), "notes[0].anchor"),
        (with(remove("author")), "notes[0].author"),
        (with(remove("created")), "notes[0].created"),
        (with(set("resolved", "yes".into())), "notes[0].resolved"),
        (with(set("anchor", json!({ "start": 1, "end": 2 }))), "notes[0].anchor.block"),
        (with(set("anchor", json!({ "block": "b", "start": 1 }))), "notes[0].anchor"),
        (with(set("anchor", json!({ "block": "b", "start": 5, "end": 2 }))), "notes[0].anchor.end"),
        (with(set("anchor", json!({ "block": "b", "start": -1, "end": 2 }))), "notes[0].anchor.start"),
        (with(set("content", json!([{ "marks": ["bold"] }]))), "notes[0].content[0].text"),
        (with(set("replies", json!([{ "created": "2026" }]))), "notes[0].replies[0].author"),
        (with(set("type", "conflict".into())), "notes[0].versions"),
        (with(|note| (note["type"], note["versions"]) = ("conflict".into(), one_version.clone())), "notes[0].versions"),
        (
            with(|note| (note["type"], note["versions"]) = ("conflict".into(), json!([{ "author": "Ana" }, {}]))),
            "notes[0].versions[0].edited",
        ),
        (twice, "notes"),
    ];
    for (json, path) in cases {
        let error = Notes::from_json(&json).expect_err(&json);
        assert_eq!(error.path, path, "{json}: {error}");
    }
}
