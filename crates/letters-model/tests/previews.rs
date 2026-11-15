//! What a document keeps of the pages its links lead to, which a link in the text shows on hover.

mod common;

use common::{sync, typed};
use letters_model::{Block, BlockKind, Body, Document, Model, Op, OpError, Preview, ReplicaId, Run};

const URL: &str = "https://example.com/post";
const OTHER: &str = "https://example.org/";

fn preview(title: Option<&str>, image: Option<&str>) -> Preview {
    Preview { title: title.map(str::to_owned), image: image.map(str::to_owned), ..Preview::default() }
}

/// What a fetch of [`URL`] found.
fn fetched() -> Preview {
    Preview { description: Some("Two\nlines".into()), ..preview(Some("A post"), Some("assets/0a1b.jpg")) }
}

fn set(url: &str, preview: Option<Preview>) -> Op {
    Op::SetPreview { url: url.into(), preview }
}

/// A paragraph `p` whose text links to each of `links`.
fn linking(links: &[&str]) -> Document {
    let run = |link: &&str| Run { link: Some((*link).to_owned()), ..Run::plain("link") };
    let p = Block { body: Body::Text(links.iter().map(run).collect()), ..Block::new("p", BlockKind::Paragraph) };
    Document { blocks: vec![p], ..Document::default() }
}

fn model() -> Model {
    Model::new(linking(&[URL]), ReplicaId::from(1))
}

#[test]
fn a_preview_is_kept_stored_and_taken_back_like_any_edit() {
    let mut model = model();
    let before = model.revision();
    let change = model.apply([set(URL, Some(fetched()))]).unwrap();

    assert!(change.document && change.blocks.is_empty(), "a preview changes the document and no block");
    assert_eq!(model.doc().previews.get(URL), Some(&fetched()));
    assert_eq!(Document::from_json(&model.doc().to_json()).as_ref(), Ok(model.doc()), "a file holds it");
    let opened = Model::load(&model.save(), ReplicaId::from(2)).unwrap();
    assert_eq!(opened.doc(), model.doc(), "a history holds it");

    model.apply([typed("p", 4, "!")]).unwrap();
    let after = model.revision();
    assert!(model.document_at(&before).unwrap().0.previews.is_empty(), "an older version had none");
    assert_eq!(model.document_at(&after).unwrap().0.previews.get(URL), Some(&fetched()));

    assert!(model.undo().is_some() && model.undo().is_some());
    assert!(model.doc().previews.is_empty());
    assert!(model.redo().is_some());
    assert_eq!(model.doc().previews.get(URL), Some(&fetched()));
}

#[test]
fn a_preview_is_taken_away_by_none_or_by_one_that_holds_nothing() {
    for gone in [None, Some(Preview::default())] {
        let mut model = model();
        model.apply([set(URL, Some(fetched())), set(OTHER, Some(preview(Some("Other"), None)))]).unwrap();
        model.apply([set(URL, gone)]).unwrap();
        assert_eq!(model.doc().previews.keys().collect::<Vec<_>>(), [OTHER]);
    }
}

#[test]
fn previews_no_file_would_hold_are_refused() {
    let cases = [
        ("no address", "", preview(Some("A post"), None)),
        ("an address of two lines", "https://example.com/\npost", preview(Some("A post"), None)),
        ("a title of two lines", URL, preview(Some("one\ntwo"), None)),
        ("a title that is empty, not left out", URL, preview(Some(""), None)),
        ("a picture that is an address", URL, preview(None, Some("https://example.com/a.png"))),
    ];
    for (name, url, preview) in cases {
        let mut model = model();
        let refused = model.apply([typed("p", 0, "x"), set(url, Some(preview))]);
        assert_eq!(refused, Err(OpError::BadPreview(url.into())), "{name}");
        assert!(model.doc().previews.is_empty() && model.doc().blocks[0].plain_text() == "link", "{name}");
    }
}

#[test]
fn previews_two_writers_set_apart_all_stay_and_one_address_is_one_of_theirs_whole() {
    let mut ana = model();
    let mut ben = ana.fork(ReplicaId::from(2));
    let (hers, his) = (preview(Some("Ana's title"), None), preview(None, Some("assets/ben.jpg")));
    ana.apply([set(URL, Some(hers.clone())), set(OTHER, Some(fetched()))]).unwrap();
    ben.apply([set(URL, Some(his.clone())), set("mailto:ask@example.com", Some(fetched()))]).unwrap();
    sync(&mut ana, &mut ben);

    assert_eq!(ana.doc(), ben.doc());
    let previews = &ana.doc().previews;
    assert_eq!(previews.len(), 3, "a preview of an address only one writer set was lost");
    let merged = previews.get(URL).cloned();
    assert!([Some(hers), Some(his)].contains(&merged), "the fields of two writers were mixed: {merged:?}");
}

#[test]
fn a_copy_without_a_history_carries_only_the_previews_a_link_names() {
    let mut doc = linking(&[URL, "https://example.com/bare"]);
    doc.blocks.push(Block::new(
        "card",
        BlockKind::LinkPreview {
            url: OTHER.into(),
            title: None,
            description: None,
            image: Some("assets/card.jpg".into()),
        },
    ));
    doc.previews.insert(URL.into(), fetched());
    // No link in the text leads here: a card is a preview of its own.
    doc.previews.insert(OTHER.into(), preview(Some("Other"), Some("assets/other.jpg")));

    let shown: Vec<&str> = doc.assets().into_iter().collect();
    assert_eq!(shown, ["assets/0a1b.jpg", "assets/card.jpg"]);
    doc.prune_previews();
    assert_eq!(doc.previews.keys().collect::<Vec<_>>(), [URL]);
}
