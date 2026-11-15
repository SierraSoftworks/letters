use letters_model::{Align, Block, BlockKind, Document, Mark, PropertiesFormat};
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
}

#[test]
fn what_any_block_or_run_may_carry_is_read() {
    let doc = Document::from_json(CORPUS[1]).unwrap();
    let block = doc.block(&"g".into()).unwrap();
    assert_eq!((block.page_break, block.lang.as_deref()), (true, Some("fr")));
    let langs: Vec<Option<&str>> = block.body.runs().iter().map(|run| run.lang.as_deref()).collect();
    assert_eq!(langs, [None, Some("de"), None]);
    assert!(!block.extra.contains_key("break") && !block.extra.contains_key("lang"));

    let properties = doc.properties.as_ref().unwrap();
    assert_eq!(
        (properties.format, properties.source.as_str()),
        (PropertiesFormat::Yaml, "title: Every block\ntags: [a, b]\n")
    );
}

#[test]
fn maths_is_read_as_its_source() {
    let doc = Document::from_json(CORPUS[1]).unwrap();
    let formula = doc.block(&"m".into()).unwrap();
    assert_eq!((&formula.kind, formula.plain_text().as_str()), (&BlockKind::Formula, r"e^{i\pi} + 1 = 0"));
    let inline = doc.block(&"g".into()).unwrap().body.runs().last().unwrap();
    assert_eq!((inline.text.as_str(), inline.marks.contains(&Mark::Math)), ("x^2", true));
}

#[test]
fn pictures_are_read_with_their_captions() {
    let doc = Document::from_json(CORPUS[1]).unwrap();
    let figure = doc.block(&"i".into()).unwrap();
    let BlockKind::Figure { asset, src, alt, width, height, numbered } = &figure.kind else {
        panic!("{:?} is no figure", figure.kind)
    };
    assert_eq!((asset.as_deref(), src.as_deref()), (Some("assets/5e884898da28.png"), None));
    assert_eq!((alt.as_str(), *width, *height, *numbered), ("A harbour at dusk", 320.0, 213.5, true));
    assert_eq!(figure.plain_text(), "The harbour");

    let gallery = doc.block(&"j".into()).unwrap();
    assert_eq!(gallery.kind, BlockKind::Gallery { numbered: true });
    let parts: Vec<(bool, String)> =
        gallery.body.blocks().iter().map(|part| (part.kind == BlockKind::Caption, part.plain_text())).collect();
    assert_eq!(parts, [(false, String::new()), (false, String::new()), (true, "Boats and nets".into())]);

    let reference = &doc.block(&"h".into()).unwrap().body.runs()[1];
    assert_eq!(reference.text, "Figure 1");
    assert!(reference.marks.contains(&Mark::Ref("i".into())) && reference.extra.is_empty());
}

#[test]
fn a_table_is_read_as_rows_of_cells() {
    let doc = Document::from_json(CORPUS[1]).unwrap();
    let table = doc.block(&"t".into()).unwrap();
    assert_eq!(table.kind, BlockKind::Table { columns: vec![Align::Start, Align::End], header: true });
    let cells = |row: &Block| row.body.blocks().iter().map(Block::plain_text).collect::<Vec<_>>();
    let rows: Vec<Vec<String>> = table.body.blocks().iter().map(cells).collect();
    assert_eq!(rows, [["Boat", "Length"], ["Skiff", "4 m"]]);

    // Two people changing one table can leave a row short or long, so a file may hold one.
    let ragged = r#"{"format":1,"blocks":[{"id":"t","type":"table","columns":["start"],"blocks":[
        {"id":"r1","type":"table_row","blocks":[]},
        {"id":"r2","type":"table_row","blocks":[{"id":"a","type":"table_cell"},{"id":"b","type":"table_cell"}]}]}]}"#;
    let doc = Document::from_json(ragged).unwrap();
    let cells: Vec<usize> = doc.blocks[0].body.blocks().iter().map(|row| row.body.blocks().len()).collect();
    assert_eq!(cells, [0, 2]);
    assert_eq!(Document::from_json(&doc.to_json()).unwrap(), doc);
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
        (r#"{"format":1,"blocks":[{"id":"a","type":"divider","break":"column"}]}"#, "blocks[0].break"),
        (r#"{"format":1,"blocks":[{"id":"a","type":"divider","lang":""}]}"#, "blocks[0].lang"),
        (
            r#"{"format":1,"blocks":[{"id":"a","type":"paragraph","content":[{"text":"x","lang":5}]}]}"#,
            "blocks[0].content[0].lang",
        ),
        (
            r#"{"format":1,"blocks":[{"id":"a","type":"paragraph","content":[{"text":"x","ref":""}]}]}"#,
            "blocks[0].content[0].ref",
        ),
        (r#"{"format":1,"blocks":[{"id":"a","type":"figure","width":9,"height":9}]}"#, "blocks[0]"),
        (
            r#"{"format":1,"blocks":[{"id":"a","type":"figure","asset":"a.png","src":"https://x.test/a.png","width":9,"height":9}]}"#,
            "blocks[0]",
        ),
        (
            r#"{"format":1,"blocks":[{"id":"a","type":"figure","asset":"a.png","width":0,"height":9}]}"#,
            "blocks[0].width",
        ),
        (r#"{"format":1,"blocks":[{"id":"a","type":"figure","asset":"a.png","width":9}]}"#, "blocks[0].height"),
        (
            r#"{"format":1,"blocks":[{"id":"a","type":"figure","asset":"a.png","width":9,"height":9,"numbered":"yes"}]}"#,
            "blocks[0].numbered",
        ),
        (
            r#"{"format":1,"blocks":[{"id":"g","type":"gallery","blocks":[{"id":"a","type":"paragraph"}]}]}"#,
            "blocks[0].blocks[0]",
        ),
        (r#"{"format":1,"blocks":[{"id":"t","type":"table","columns":["left"]}]}"#, "blocks[0].columns"),
        (
            r#"{"format":1,"blocks":[{"id":"t","type":"table","columns":[],"blocks":[{"id":"a","type":"table_cell"}]}]}"#,
            "blocks[0].blocks[0]",
        ),
        (
            r#"{"format":1,"blocks":[{"id":"t","type":"table","columns":[],"blocks":[
                {"id":"r","type":"table_row","blocks":[{"id":"a","type":"table_row"}]}]}]}"#,
            "blocks[0].blocks[0].blocks[0]",
        ),
        (r#"{"format":1,"properties":{"format":"ini","source":""},"blocks":[]}"#, "properties.format"),
        (r#"{"format":1,"properties":{"format":"toml"},"blocks":[]}"#, "properties.source"),
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

/// The object with one more field, when there is a value for it.
fn with(mut object: serde_json::Value, key: &str, value: Option<String>) -> serde_json::Value {
    if let Some(value) = value {
        object[key] = value.into();
    }
    object
}

fn json_value() -> impl Strategy<Value = serde_json::Value> {
    prop_oneof![any::<bool>().prop_map(Into::into), any::<i32>().prop_map(Into::into), ".{0,6}".prop_map(Into::into)]
}

/// Documents as loosely typed JSON, so unknown fields and types are exercised alongside the known ones.
fn document_json() -> impl Strategy<Value = String> {
    let lang = || prop::option::of("en|fr|de-CH");
    let run =
        (".{0,12}", prop::collection::vec("bold|italic|code|math|odd", 0..3), prop::option::of("https://[a-z]{1,8}"))
            .prop_map(|(text, marks, link)| serde_json::json!({ "text": text, "marks": marks, "link": link }));
    let run = (run, lang(), prop::option::of("b0|gone"))
        .prop_map(|(run, lang, target)| with(with(run, "lang", lang), "ref", target));
    let block = (
        "paragraph|heading|list_item|code|formula|caption|figure|table_cell|divider|mystery",
        prop::collection::vec(run, 0..4),
        prop::collection::btree_map("[a-z]{4,8}_x", json_value(), 0..3),
    )
        .prop_map(|(kind, content, extra)| {
            let mut block = serde_json::json!({ "type": kind, "level": 3, "list": "task", "content": content });
            block["asset"] = "assets/a.png".into();
            (block["width"], block["height"]) = (120.into(), 80.25.into());
            block.as_object_mut().unwrap().extend(extra);
            block
        });
    let block = (block, lang(), prop::option::of(Just("page")))
        .prop_map(|(block, lang, at)| with(with(block, "lang", lang), "break", at.map(str::to_owned)));
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
