//! The table of contents and the link preview: blocks that hold fields and no text.

mod common;

use common::{id, sync};
use letters_model::{Block, BlockKind, Document, Model, Op, OpError, ReplicaId};

const URL: &str = "https://example.com/post";

fn contents(levels: u8) -> BlockKind {
    BlockKind::TableOfContents { levels }
}

fn preview(url: &str, title: Option<&str>, image: Option<&str>) -> BlockKind {
    let (title, image) = (title.map(str::to_owned), image.map(str::to_owned));
    BlockKind::LinkPreview { url: url.into(), title, description: None, image }
}

/// What a fetch of [`URL`] found.
fn fetched() -> BlockKind {
    let (title, description) = (Some("A post".to_owned()), Some("Two\nlines".to_owned()));
    BlockKind::LinkPreview { url: URL.into(), title, description, image: Some("assets/0a1b.jpg".into()) }
}

/// A document from one word a block: `toc` is a table of contents, `p:Text` a paragraph, and `2:Sea` a
/// heading of level 2 that reads "Sea".
fn written(lines: &[&str], levels: u8) -> Document {
    let block = |(nth, line): (usize, &&str)| {
        let name = format!("b{nth}");
        match line.split_once(':') {
            None => Block::new("toc", contents(levels)),
            Some(("p", text)) => Block::text(name.as_str(), BlockKind::Paragraph, text),
            Some((level, text)) => {
                let level = level.parse().expect("a heading's level");
                Block::text(name.as_str(), BlockKind::Heading { level }, text)
            }
        }
    };
    Document { blocks: lines.iter().enumerate().map(block).collect(), ..Document::default() }
}

/// A table of contents `toc`, a paragraph `p` and a preview `link` that was not fetched.
fn model() -> Model {
    let blocks = vec![
        Block::new("toc", contents(3)),
        Block::text("p", BlockKind::Paragraph, "Text"),
        Block::new("link", preview(URL, None, None)),
    ];
    Model::new(Document { blocks, ..Document::default() }, ReplicaId::from(1))
}

fn kinds(model: &Model) -> Vec<BlockKind> {
    model.doc().walk().map(|block| block.kind.clone()).collect()
}

fn set(block: &str, kind: BlockKind) -> Op {
    Op::SetKind { block: id(block), kind }
}

#[test]
fn a_table_of_contents_lists_the_headings_that_follow_it() {
    let cases: [(&str, u8, &[&str], &[&str]); 7] = [
        (
            "down to its levels",
            2,
            &["1:Title", "toc", "1:One", "2:Two", "3:Three", "p:Text", "1:Four"],
            &["One", "Two", "Four"],
        ),
        ("not the title or a heading over it", 4, &["1:Title", "2:Contents", "toc", "2:One"], &["One"]),
        ("not a heading without text", 4, &["toc", "2:", "2:  ", "2:One"], &["One"]),
        (
            "at the end, every heading but the title",
            2,
            &["1:Title", "2:One", "1:Two", "3:Deep", "toc"],
            &["One", "Two"],
        ),
        (
            "before text alone, every heading but the title",
            4,
            &["2:Before", "1:Title", "1:Next", "toc", "p:Text"],
            &["Before", "Next"],
        ),
        ("nothing, when what follows is deeper than it lists", 1, &["1:Title", "toc", "2:One"], &[]),
        ("nothing, in a document without headings", 4, &["p:Text", "toc"], &[]),
    ];
    for (name, levels, lines, expected) in cases {
        let doc = written(lines, levels);
        let listed: Vec<String> = doc.contents(&id("toc")).into_iter().map(Block::plain_text).collect();
        assert_eq!(listed, expected, "{name}");
    }
    // Only a table of contents lists anything.
    let doc = written(&["1:Title", "p:Text", "2:One"], 4);
    assert!(doc.contents(&id("b1")).is_empty() && doc.contents(&id("gone")).is_empty());
}

#[test]
fn both_are_changed_stored_and_taken_back_like_any_block() {
    let mut model = model();
    let before = kinds(&model);
    // What a fetch found goes in as one change of the block.
    model.apply([set("link", fetched()), set("toc", contents(2))]).unwrap();

    assert_eq!(kinds(&model), [contents(2), BlockKind::Paragraph, fetched()]);
    assert_eq!(Document::from_json(&model.doc().to_json()).as_ref(), Ok(model.doc()), "a file holds both");
    let opened = Model::load(&model.save(), ReplicaId::from(2)).unwrap();
    assert_eq!(opened.doc(), model.doc(), "a history holds both");

    assert!(model.undo().is_some());
    assert_eq!(kinds(&model), before);
}

#[test]
fn fields_no_file_would_hold_are_refused() {
    let cases = [
        ("a table that lists no level", "toc", contents(0)),
        ("a table that lists deeper than headings go", "toc", contents(5)),
        ("a preview with no address", "link", preview("", None, None)),
        ("an address of two lines", "link", preview("https://example.com/\npost", None, None)),
        ("a title of two lines", "link", preview(URL, Some("one\ntwo"), None)),
        ("a title that is empty, not left out", "link", preview(URL, Some(""), None)),
        ("a picture that is an address", "link", preview(URL, None, Some("https://example.com/a.png"))),
    ];
    for (name, block, kind) in cases {
        let mut model = model();
        let before = kinds(&model);
        assert!(matches!(model.apply([set(block, kind)]), Err(OpError::BadKind(_))), "{name}");
        assert_eq!(kinds(&model), before, "{name}");
    }
}

#[test]
fn a_preview_two_writers_changed_apart_is_one_of_theirs_whole() {
    let mut ana = model();
    let mut ben = ana.fork(ReplicaId::from(2));
    let (hers, his) = (preview(URL, Some("Ana's title"), None), preview(URL, None, Some("assets/ben.jpg")));
    ana.apply([set("link", hers.clone())]).unwrap();
    ben.apply([set("link", his.clone())]).unwrap();
    sync(&mut ana, &mut ben);

    assert_eq!(ana.doc(), ben.doc());
    let merged = ana.doc().block(&id("link")).map(|block| block.kind.clone());
    assert!([Some(hers), Some(his)].contains(&merged), "the fields of two writers were mixed: {merged:?}");
}
