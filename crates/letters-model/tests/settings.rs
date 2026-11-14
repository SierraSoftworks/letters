//! The style and the document's own fields: edits that are taken back, merged and kept like any other.

mod common;

use common::{doc, pair, sync, typed};
use letters_model::{
    Change, Document, Extra, Model, Op, OpError, Page, Properties, PropertiesFormat, ReplicaId, Style, TextStyle,
};

/// Sets the style of first headings to the built-in one with a change.
fn heading(change: impl FnOnce(&mut TextStyle)) -> Op {
    let mut style = Style::default().types["heading1"].clone();
    change(&mut style);
    Op::SetStyle { element: "heading1".into(), style }
}

fn page(change: impl FnOnce(&mut Page)) -> Op {
    let mut page = Page::default();
    change(&mut page);
    Op::SetPage { page }
}

/// A style that differs from the built-in one in its page and in a type.
fn narrow() -> Style {
    let mut style = Style { page: Page { width: 400.0, ..Page::default() }, ..Style::default() };
    style.types.get_mut("body").unwrap().size = 13.5;
    style
}

/// The front matter of a Markdown file.
fn front_matter() -> Properties {
    Properties { format: PropertiesFormat::Yaml, source: "title: Hello\n".into(), extra: Extra::new() }
}

/// Everything a model shows.
fn shown(model: &Model) -> (Document, Style) {
    (model.doc().clone(), model.style().clone())
}

/// A name, an edit, whether the model shows it, and whether it is to the style rather than the document.
type Setting = (&'static str, Op, fn(&Model) -> bool, bool);

#[test]
fn a_setting_is_an_edit_that_can_be_taken_back() {
    let aside = Op::SetStyle { element: "aside".into(), style: TextStyle { italic: true, ..TextStyle::default() } };
    let cases: Vec<Setting> = vec![
        ("a type's size", heading(|h| h.size = 30.0), |m| m.style().types["heading1"].size == 30.0, true),
        ("a type of its own", aside, |m| m.style().types.get("aside").is_some_and(|aside| aside.italic), true),
        ("the page", page(|page| page.width = 400.0), |m| m.style().page.width == 400.0, true),
        ("a whole style", Op::ReplaceStyle { style: narrow() }, |m| m.style() == &narrow(), true),
        ("the language", Op::SetLanguage { language: "fr".into() }, |m| m.doc().language == "fr", false),
        ("a word", Op::AddWord { word: "mot".into() }, |m| m.doc().dictionary.contains("mot"), false),
        ("no word", Op::RemoveWord { word: "Letters".into() }, |m| !m.doc().dictionary.contains("Letters"), false),
        (
            "properties",
            Op::SetProperties { properties: Some(front_matter()) },
            |m| m.doc().properties == Some(front_matter()),
            false,
        ),
    ];
    for (name, op, holds, style) in cases {
        let mut model = Model::new(doc(), ReplicaId::from(1));
        let before = shown(&model);

        let change = model.apply([op.clone()]).unwrap();
        assert!(holds(&model), "{name}");
        assert_eq!((change.style, change.document), (style, !style), "{name}");
        assert!(change.blocks.is_empty() && !change.structure, "{name}: no block is laid out again for it");
        assert_eq!(model.apply([op]), Ok(Change::default()), "{name}: set again, it changes nothing");

        assert_eq!(model.undo().map(|change| (change.style, change.document)), Some((style, !style)), "{name}");
        assert_eq!(shown(&model), before, "{name}");
        model.redo().unwrap();
        assert!(holds(&model), "{name}");
    }
}

#[test]
fn a_style_the_file_would_refuse_is_refused() {
    let cases = [
        ("no size", heading(|h| h.size = 0.0)),
        ("a size that is no number", heading(|h| h.size = f32::NAN)),
        ("no weight", heading(|h| h.weight = 0)),
        ("a colour by name", heading(|h| h.color = "red".into())),
        ("a blank family", heading(|h| h.family = " ".into())),
        ("margins that leave no room", page(|page| page.margin_left = page.width)),
    ];
    for (name, op) in cases {
        let mut model = Model::new(doc(), ReplicaId::from(1));
        let before = shown(&model);
        let refused = model.apply([typed("p", 0, "x"), op]);
        assert!(matches!(refused, Err(OpError::BadStyle(_))), "{name}: {refused:?}");
        assert_eq!(shown(&model), before, "{name}: nothing of the edit is kept");
        assert!(model.undo().is_none(), "{name}");
    }
}

#[test]
fn concurrent_style_changes_merge_field_by_field() {
    let (mut a, mut b) = pair();
    a.apply([heading(|h| h.size = 30.0)]).unwrap();
    b.apply([heading(|h| h.color = "#ff0000".into()), page(|page| page.width = 400.0)]).unwrap();

    let change = a.merge(&b.save()).unwrap();
    assert!(change.style && change.blocks.is_empty());
    b.merge(&a.save()).unwrap();
    assert_eq!(a.style(), b.style());
    let heading1 = &a.style().types["heading1"];
    assert_eq!((heading1.size, heading1.color.as_str(), a.style().page.width), (30.0, "#ff0000", 400.0));

    // Two values for one field cannot both stand, so every replica settles on the same one.
    a.apply([heading(|h| h.size = 31.0)]).unwrap();
    b.apply([heading(|h| h.size = 32.0)]).unwrap();
    sync(&mut a, &mut b);
    assert_eq!(a.style(), b.style());
    assert!([31.0, 32.0].contains(&a.style().types["heading1"].size));
}

#[test]
fn concurrent_changes_to_the_document_fields_merge() {
    let (mut a, mut b) = pair();
    // One replica removes a word and adds it again, while the other only removes it.
    a.apply([Op::RemoveWord { word: "Letters".into() }]).unwrap();
    a.apply([Op::AddWord { word: "Letters".into() }, Op::SetLanguage { language: "fr".into() }]).unwrap();
    b.apply([Op::RemoveWord { word: "Letters".into() }, Op::AddWord { word: "mot".into() }]).unwrap();

    let change = a.merge(&b.save()).unwrap();
    assert!(change.document && !change.style);
    b.merge(&a.save()).unwrap();
    assert_eq!(a.doc(), b.doc());
    let words: Vec<&str> = a.doc().dictionary.iter().map(String::as_str).collect();
    assert_eq!(words, ["Letters", "mot"], "a word added again outlasts a removal that never saw it");
    assert_eq!(a.doc().language, "fr");
}

#[test]
fn settings_are_kept_with_the_history() {
    let mut model = Model::new(doc(), ReplicaId::from(1)).with_style(narrow());
    assert_eq!(model.style(), &narrow());
    let first = model.revision();
    model.apply([heading(|h| h.size = 30.0), Op::SetLanguage { language: "fr".into() }]).unwrap();
    let edited = shown(&model);

    let loaded = Model::load(&model.save(), ReplicaId::from(2)).unwrap();
    assert_eq!(shown(&loaded), edited);

    let change = model.checkout(&first).unwrap();
    assert!(change.style && change.document);
    assert_eq!((model.style(), model.doc().language.as_str()), (&narrow(), "en-GB"));
    model.checkout_latest();
    assert_eq!(shown(&model), edited);
}

#[test]
fn a_file_opens_with_the_style_it_holds() {
    let history = Model::new(doc(), ReplicaId::from(1)).with_style(narrow()).save();
    let outside = Style { page: Page { height: 500.0, ..Page::default() }, ..narrow() };
    for (name, style) in [("the style of its history", narrow()), ("a style changed outside", outside)] {
        let open = || Model::new(doc(), ReplicaId::from(2)).with_style(style.clone());
        let mut model = open();
        assert_eq!(model.attach(open().prepare(Some(&history)).unwrap()), Change::default(), "{name}");
        assert_eq!(model.style(), &style, "{name}");

        // Whoever has the file's history gets the style from this one.
        let mut other = Model::load(&history, ReplicaId::from(3)).unwrap();
        other.merge(&model.save()).unwrap();
        assert_eq!(other.style(), &style, "{name}");
    }
}

#[test]
fn copies_of_a_file_under_different_styles_share_its_document() {
    let mut a = Model::new(doc(), ReplicaId::from(1)).with_style(narrow());
    let mut b = Model::new(doc(), ReplicaId::from(2));
    let before = b.doc().clone();
    sync(&mut a, &mut b);
    assert_eq!((a.doc(), b.doc()), (&before, &before), "the document is there once");
    assert_eq!(a.style(), b.style());
}
