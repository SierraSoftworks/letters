use letters_model::{Align, Model, Op, ReplicaId, Style, StyleFont};
use proptest::prelude::*;

#[test]
fn the_built_in_style_survives_the_round_trip() {
    let style = Style::default();
    let json = style.to_json();
    assert_eq!(Style::from_json(&json).unwrap(), style);
    // Every type is written with every field, so a reader needs no defaults of its own.
    for name in Style::TYPES {
        assert!(json.contains(&format!("\"{name}\"")), "{name} is missing from {json}");
    }
}

#[test]
fn anything_missing_comes_from_the_built_in_style() {
    let style =
        Style::from_json(r##"{"format":1,"types":{"body":{"size":12.5,"color":"#AABBCC","align":"justify"}}}"##)
            .unwrap();
    let built_in = Style::default();
    let body = &style.types["body"];
    assert_eq!((body.size, body.color.as_str(), body.align), (12.5, "#aabbcc", Align::Justify));
    assert_eq!(body.family, built_in.types["body"].family);
    assert_eq!(style.types["title"], built_in.types["title"]);
    assert_eq!(style.page, built_in.page);
}

#[test]
fn unknown_content_survives() {
    let json = r#"{"format":1,"theme":"dusk","types":{"body":{"tracking":0.5},"margin_note":{"size":8}},
        "page":{"columns":2}}"#;
    let written = Style::from_json(json).unwrap().to_json();
    let reread: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(reread["theme"], "dusk");
    assert_eq!(reread["types"]["body"]["tracking"], 0.5);
    assert_eq!(reread["types"]["margin_note"]["size"], 8);
    assert_eq!(reread["page"]["columns"], 2);
}

#[test]
fn schema_breaks_are_refused_with_their_path() {
    let cases = [
        (r#"[]"#, ""),
        (r#"{"types":{}}"#, "format"),
        (r#"{"format":2}"#, "format"),
        (r#"{"format":1,"types":{"body":{"weight":0}}}"#, "types.body.weight"),
        (r#"{"format":1,"types":{"body":{"size":-1}}}"#, "types.body.size"),
        (r#"{"format":1,"types":{"body":{"color":"red"}}}"#, "types.body.color"),
        (r#"{"format":1,"types":{"title":{"align":"left"}}}"#, "types.title.align"),
        (r#"{"format":1,"types":{"title":"big"}}"#, "types.title"),
        (r#"{"format":1,"page":{"width":0}}"#, "page.width"),
        (r#"{"format":1,"page":{"margin_left":300,"margin_right":300}}"#, "page"),
    ];
    for (json, path) in cases {
        let error = Style::from_json(json).expect_err(json);
        assert_eq!(error.path, path, "{json}: {error}");
    }
}

proptest! {
    #[test]
    fn writing_then_reading_gives_the_same_style(
        sizes in prop::collection::vec(0.01f32..500.0, 9),
        weight in 1u16..=1000,
        italic: bool,
        color in "#[0-9a-f]{6}",
        margin in 0f32..200.0,
    ) {
        let mut style = Style::default();
        for (text, size) in style.types.values_mut().zip(sizes) {
            (text.size, text.line_height, text.space_after) = (size, size / 7.0, size * 3.0);
            (text.weight, text.italic, text.color) = (weight, italic, color.clone());
        }
        style.page.margin_left = margin;
        let written = style.to_json();
        let reread = Style::from_json(&written).unwrap();
        prop_assert_eq!(&reread, &style);
        prop_assert_eq!(reread.to_json(), written);
    }
}

#[test]
fn the_faces_a_document_holds_are_listed_and_named_as_its_assets() {
    let face = |weight, italic, asset: &str| StyleFont { family: "Lora".into(), weight, italic, asset: asset.into() };
    let fonts = [face(700, false, "assets/b.ttf"), face(400, true, "assets/c.ttf"), face(400, false, "assets/a.ttf")];
    let mut style = Style::default();
    assert!(style.fonts().is_empty() && style.assets().is_empty(), "a style lists none until told");

    style.set_fonts(&fonts);
    let reread = Style::from_json(&style.to_json()).unwrap();
    assert_eq!(reread, style, "the list is written and read back");
    assert_eq!(reread.fonts().len(), 3);
    assert_eq!(reread.assets().into_iter().collect::<Vec<_>>(), ["assets/a.ttf", "assets/b.ttf", "assets/c.ttf"]);

    let mut other = Style::default();
    other.set_fonts(&[fonts[2].clone(), fonts[0].clone(), fonts[1].clone(), fonts[1].clone()]);
    assert_eq!(other, style, "whatever order they are given in, and each once");

    style.set_fonts(&[]);
    assert_eq!(style, Style::default(), "and a style that holds none says nothing of fonts");
}

#[test]
fn the_list_of_faces_travels_with_the_history() {
    let mut style = Style::default();
    style.set_fonts(&[StyleFont { family: "Lora".into(), weight: 400, italic: false, asset: "assets/a.ttf".into() }]);
    let mut model = Model::new(Default::default(), ReplicaId::from(1));
    model.apply(vec![Op::ReplaceStyle { style: style.clone() }]).unwrap();
    let copy = model.fork(ReplicaId::from(2));
    assert_eq!(copy.style().fonts(), style.fonts());
    // What a reader makes of an entry that is no face.
    let odd = r#"{"format":1,"fonts":[{"family":"Lora"},"x",{"family":"Lora","weight":400,"italic":false,"asset":"assets/a.ttf"}]}"#;
    assert_eq!(Style::from_json(odd).unwrap().fonts(), style.fonts(), "only what is a face is listed");
}
