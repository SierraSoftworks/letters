use letters_model::{BlockKind, Document};
use proptest::prelude::*;

const CORPUS: [&str; 2] = [include_str!("corpus/design-sample.json"), include_str!("corpus/every-block.json")];

#[test]
fn corpus_files_are_already_canonical() {
    for json in CORPUS {
        assert_eq!(Document::from_json(json).unwrap().to_json(), json);
    }
}

#[test]
fn key_order_and_spacing_do_not_change_the_bytes() {
    let shuffled = r#"{"blocks":[{"content":[{"marks":["italic","bold"],"text":"x"}],"zeta":{"b":1,"a":2},"level":1,
        "type":"heading","id":"h"}],"format":1}"#;
    let canonical = Document::from_json(shuffled).unwrap().to_json();
    assert_eq!(Document::from_json(&canonical).unwrap().to_json(), canonical);
    let order =
        ["\"id\"", "\"type\"", "\"level\"", "\"content\"", "\"zeta\"", "\"a\""].map(|k| canonical.find(k).unwrap());
    assert!(order.is_sorted(), "{canonical}");
}

#[test]
fn unknown_content_survives() {
    let doc = Document::from_json(CORPUS[1]).unwrap();
    let hologram = doc.block(&"f".into()).unwrap();
    assert_eq!(hologram.kind, BlockKind::Unknown { type_name: "hologram".into() });
    assert_eq!(hologram.extra["fallback"], "A hologram");
    assert_eq!(doc.block(&"g".into()).unwrap().extra["break"], "page");
}

#[test]
fn schema_breaks_are_refused_with_their_path() {
    let cases = [
        (r#"[]"#, ""),
        (r#"{"blocks":[]}"#, "format"),
        (r#"{"format":2,"blocks":[]}"#, "format"),
        (r#"{"format":1,"blocks":[{"type":"paragraph"}]}"#, "blocks[0].id"),
        (r#"{"format":1,"blocks":[{"id":"a"}]}"#, "blocks[0].type"),
        (r#"{"format":1,"blocks":[{"id":"a","type":"heading","level":5}]}"#, "blocks[0].level"),
        (r#"{"format":1,"blocks":[{"id":"a","type":"callout","tone":"loud"}]}"#, "blocks[0].tone"),
        (
            r#"{"format":1,"blocks":[{"id":"a","type":"paragraph","content":[{"marks":[]}]}]}"#,
            "blocks[0].content[0].text",
        ),
        (
            r#"{"format":1,"blocks":[{"id":"q","type":"quote","blocks":[{"id":"a","type":"list_item"}]}]}"#,
            "blocks[0].blocks[0].list",
        ),
        (r#"{"format":1,"blocks":[{"id":"a","type":"divider"},{"id":"a","type":"divider"}]}"#, "blocks"),
    ];
    for (json, path) in cases {
        let error = Document::from_json(json).expect_err(json);
        assert_eq!(error.path, path, "{json}: {error}");
    }
}

#[test]
fn a_repeated_key_keeps_its_last_value() {
    let cases = [
        r#"{"format":1,"blocks":[{"id":"x","type":"divider"}],"blocks":[{"id":"a","type":"paragraph","content":[{"text":"kept"}]}]}"#,
        r#"{"format":1,"blocks":[{"id":"a","type":"paragraph","content":[{"text":"lost"}],"content":[{"text":"kept"}]}]}"#,
        r#"{"format":1,"blocks":[{"id":"a","type":"paragraph","content":[{"text":"lost","text":"kept"}]}]}"#,
        r#"{"format":1,"blocks":[{"id":"a","type":"heading","type":"paragraph","content":[{"text":"kept"}]}]}"#,
    ];
    for json in cases {
        let doc = Document::from_json(json).expect(json);
        let block = doc.block(&"a".into()).expect(json);
        assert_eq!((&block.kind, block.plain_text().as_str()), (&BlockKind::Paragraph, "kept"), "{json}");
    }
}

/// The same JSON with the keys of every object in the opposite order.
fn reversed(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => map.iter().rev().map(|(key, value)| (key.clone(), reversed(value))).collect(),
        serde_json::Value::Array(items) => items.iter().map(reversed).collect(),
        other => other.clone(),
    }
}

fn json_value() -> impl Strategy<Value = serde_json::Value> {
    prop_oneof![any::<bool>().prop_map(Into::into), any::<i32>().prop_map(Into::into), ".{0,6}".prop_map(Into::into)]
}

/// Documents as loosely typed JSON, so unknown fields and types are exercised alongside the known ones.
fn document_json() -> impl Strategy<Value = String> {
    let run = (".{0,12}", prop::collection::vec("bold|italic|code|odd", 0..3), prop::option::of("https://[a-z]{1,8}"))
        .prop_map(|(text, marks, link)| serde_json::json!({ "text": text, "marks": marks, "link": link }));
    let block = (
        "paragraph|heading|list_item|code|divider|mystery",
        prop::collection::vec(run, 0..4),
        prop::collection::btree_map("[a-z]{4,8}_x", json_value(), 0..3),
    )
        .prop_map(|(kind, content, extra)| {
            let mut block = serde_json::json!({ "type": kind, "level": 3, "list": "task", "content": content });
            block.as_object_mut().unwrap().extend(extra);
            block
        });
    prop::collection::vec(block, 0..8).prop_map(|mut blocks| {
        for (i, block) in blocks.iter_mut().enumerate() {
            block["id"] = format!("b{i}").into();
        }
        serde_json::json!({ "format": 1, "blocks": blocks }).to_string()
    })
}

proptest! {
    #[test]
    fn writing_then_reading_gives_the_same_document(json in document_json()) {
        // A null link is not a string, so the generator's `None` links are dropped first.
        let json = json.replace(r#","link":null"#, "").replace(r#""link":null,"#, "");
        let doc = Document::from_json(&json).unwrap();
        let written = doc.to_json();
        let reread = Document::from_json(&written).unwrap();
        prop_assert_eq!(&reread, &doc);
        prop_assert_eq!(reread.to_json(), written);
    }

    #[test]
    fn the_order_of_keys_never_changes_what_is_read(json in document_json()) {
        let json = json.replace(r#","link":null"#, "").replace(r#""link":null,"#, "");
        let canonical = Document::from_json(&json).unwrap().to_json();
        let backwards = reversed(&serde_json::from_str(&canonical).unwrap()).to_string();
        prop_assert_eq!(Document::from_json(&backwards).unwrap(), Document::from_json(&canonical).unwrap());
    }
}
