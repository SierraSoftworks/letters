use letters_model::{
    Anchor, Block, BlockKind, Document, Model, Note, NoteKind, Notes, Op, Pos, ReplicaId, Reply, Run, Version,
};
use proptest::prelude::*;
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
        (
            with(set(
                "anchor",
                json!({ "block": "b", "start": 1, "end": 2, "stable_start": "soon", "stable_end": "end" }),
            )),
            "notes[0].anchor.stable_start",
        ),
        (with(set("anchor", json!({ "block": "b", "start": 1, "end": 2, "stable_end": "end" }))), "notes[0].anchor"),
        (with(set("anchor", json!({ "block": "b", "stable_start": "end", "stable_end": "end" }))), "notes[0].anchor"),
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

#[test]
fn merging_keeps_every_note_reply_and_resolution() {
    let reply = |author: &str, minute: u32| Reply {
        author: author.into(),
        created: format!("2026-11-14T20:{minute}:00Z"),
        content: vec![Run::plain("Agreed")],
        extra: Default::default(),
    };
    let shared = note("n1", NoteKind::Comment);
    let mut ours = Notes {
        notes: vec![
            Note { replies: vec![reply("Ana", 20)], ..shared.clone() },
            Note { created: "2026-11-14T21:00:00Z".into(), ..note("ours", NoteKind::Import) },
        ],
        ..Notes::default()
    };
    let theirs = Notes {
        notes: vec![
            Note { created: "2026-11-14T19:00:00Z".into(), ..note("theirs", NoteKind::Comment) },
            Note { replies: vec![reply("Ben", 10), reply("Ana", 20)], resolved: true, ..shared },
        ],
        ..Notes::default()
    };

    assert!(ours.merge(&theirs));
    let ids: Vec<&str> = ours.notes.iter().map(|note| note.id.as_str()).collect();
    assert_eq!(ids, ["theirs", "n1", "ours"], "notes read in the order they were written");
    let n1 = &ours.notes[1];
    assert!(n1.resolved);
    let authors: Vec<&str> = n1.replies.iter().map(|reply| reply.author.as_str()).collect();
    assert_eq!(authors, ["Ben", "Ana"]);
    assert!(!ours.merge(&theirs), "a merge that adds nothing changes nothing");
}

/// A model of one paragraph `p` reading "Hello world", and a note pinned on "world".
fn pinned(replica: u64) -> (Model, Notes) {
    let doc = Document { blocks: vec![Block::text("p", BlockKind::Paragraph, "Hello world")], ..Document::default() };
    let model = Model::new(doc, ReplicaId::from(replica));
    let mut notes = Notes {
        notes: vec![Note { anchor: Anchor::range("p", 6..11), ..note("n", NoteKind::Comment) }],
        ..Notes::default()
    };
    notes.pin(&model);
    (model, notes)
}

/// The text an anchor covers now.
fn covered(model: &Model, anchor: &Anchor) -> String {
    let text = model.doc().block(&anchor.block).unwrap().plain_text();
    text[anchor.range.clone().unwrap()].to_owned()
}

#[test]
fn an_anchor_follows_its_text_through_edits() {
    let typed = |offset: usize, text: &str| Op::InsertText { at: Pos::new("p", offset), text: text.into() };
    let cases = [
        ("typing before it", vec![typed(0, "Oh. ")], "world"),
        ("typing inside it", vec![typed(8, "---")], "wo---rld"),
        ("typing at its ends", vec![typed(11, "!"), typed(6, "big ")], "world"),
        ("a split before it", vec![Op::Split { at: Pos::new("p", 5), new_id: "q".into() }], "world"),
        ("a split inside it", vec![Op::Split { at: Pos::new("p", 8), new_id: "q".into() }], "wo"),
        ("deleting part of it", vec![Op::DeleteText { block: "p".into(), range: 4..8 }], "rld"),
    ];
    for (why, ops, expected) in cases {
        let (mut model, mut notes) = pinned(1);
        model.apply(ops).unwrap();
        notes.follow(&model);
        assert_eq!(covered(&model, &notes.notes[0].anchor), expected, "{why}");
    }
}

#[test]
fn an_anchor_follows_its_text_through_a_merge_and_a_save() {
    let (mut ours, mut notes) = pinned(1);
    let (mut theirs, _) = pinned(2);
    theirs.apply([Op::InsertText { at: Pos::new("p", 0), text: "Well, hello. ".into() }]).unwrap();
    ours.merge(&theirs.save()).unwrap();

    // The stable ends travel in notes.json, so a copy read back from a file follows the text as well.
    let mut reread = Notes::from_json(&notes.to_json()).unwrap();
    assert_eq!(reread, notes);
    reread.follow(&ours);
    notes.follow(&ours);
    assert_eq!(reread, notes);
    assert_eq!(covered(&ours, &notes.notes[0].anchor), "world");
}

/// A few notes drawn from a small pool of ids, so that two sets often hold different copies of one note.
fn some_notes() -> impl Strategy<Value = Notes> {
    let reply = (0..3u8, 0..3u8).prop_map(|(author, minute)| Reply {
        author: format!("author{author}"),
        created: format!("2026-11-14T20:0{minute}:00Z"),
        content: Vec::new(),
        extra: Default::default(),
    });
    let one = (0..4u8, 0..3u8, any::<bool>(), "[a-c]{0,2}", prop::collection::vec(reply, 0..3)).prop_map(
        |(id, minute, resolved, text, replies)| Note {
            created: format!("2026-11-14T19:0{minute}:00Z"),
            resolved,
            content: vec![Run::plain(text)],
            replies,
            ..note(&format!("n{id}"), NoteKind::Comment)
        },
    );
    prop::collection::btree_map(0..4u8, one, 0..4).prop_map(|notes| {
        let notes = notes.into_iter().map(|(id, note)| Note { id: format!("n{id}"), ..note }).collect();
        Notes { notes, ..Notes::default() }
    })
}

fn merged(mut a: Notes, b: &Notes) -> Notes {
    a.merge(b);
    a
}

proptest! {
    #[test]
    fn every_copy_settles_on_the_same_notes(a in some_notes(), b in some_notes(), c in some_notes()) {
        let ab = merged(a.clone(), &b);
        prop_assert_eq!(&ab, &merged(b.clone(), &a), "whichever side merges");
        prop_assert_eq!(&merged(ab.clone(), &c), &merged(a.clone(), &merged(b.clone(), &c)), "in whatever order");
        prop_assert!(!ab.clone().merge(&b) && !ab.clone().merge(&a), "and merging again changes nothing");
        prop_assert_eq!(Notes::from_json(&ab.to_json()).unwrap(), ab, "the result is a valid notes.json");
    }
}
