//! Figures, galleries, tables and references: blocks made of parts, and text that names a block.

mod common;

use common::{id, sync, typed};
use letters_model::{Align, Block, BlockId, BlockKind, Body, Document, Mark, Model, Op, OpError, ReplicaId};

fn picture(asset: &str, width: f32, height: f32) -> BlockKind {
    BlockKind::Figure { asset: Some(asset.into()), src: None, alt: String::new(), width, height, numbered: false }
}

fn figure(id: &str, caption: &str) -> Block {
    Block::text(id, picture(&format!("assets/{id}.png"), 240.0, 160.0), caption)
}

fn cell(id: &str, text: &str) -> Block {
    Block::text(id, BlockKind::TableCell, text)
}

fn holding(id: &str, kind: BlockKind, parts: Vec<Block>) -> Block {
    Block { body: Body::Blocks(parts), ..Block::new(id, kind) }
}

fn row(id: &str, cells: Vec<Block>) -> Block {
    holding(id, BlockKind::TableRow, cells)
}

fn columns(columns: &[Align]) -> BlockKind {
    BlockKind::Table { columns: columns.to_vec(), header: true }
}

fn insert(parent: Option<&str>, index: usize, block: Block) -> Op {
    Op::InsertBlock { parent: parent.map(id), index, block }
}

fn remove(block: &str) -> Op {
    Op::RemoveBlock { block: id(block) }
}

/// A paragraph `p`, a gallery `g` of two figures and a caption, and a table `t` of two rows of two cells.
fn doc() -> Document {
    let gallery = vec![figure("f1", "Boats"), figure("f2", ""), Block::text("cap", BlockKind::Caption, "At sea")];
    let rows = vec![
        row("r1", vec![cell("c1", "Boat"), cell("c2", "Length")]),
        row("r2", vec![cell("c3", "Skiff"), cell("c4", "4 m")]),
    ];
    let blocks = vec![
        Block::text("p", BlockKind::Paragraph, "See Figure 1."),
        holding("g", BlockKind::Gallery { numbered: true }, gallery),
        holding("t", columns(&[Align::Start, Align::End]), rows),
    ];
    Document { blocks, ..Document::default() }
}

const OUTLINE: &str = "p g(f1 f2 cap) t(r1(c1 c2) r2(c3 c4))";

fn model() -> Model {
    Model::new(doc(), ReplicaId::from(1))
}

/// The ids of a document's blocks, with what each holds in brackets after it.
fn outline(doc: &Document) -> String {
    fn walk(blocks: &[Block]) -> String {
        let one = |block: &Block| match block.body.blocks() {
            [] => block.id.to_string(),
            parts => format!("{}({})", block.id, walk(parts)),
        };
        blocks.iter().map(one).collect::<Vec<_>>().join(" ")
    }
    walk(&doc.blocks)
}

/// Whether the document can go into a file and come back, as only one that nests its parts rightly can.
fn stored(doc: &Document) -> bool {
    Document::from_json(&doc.to_json()).as_ref() == Ok(doc)
}

#[test]
fn a_table_grows_by_rows_and_columns() {
    let mut m = model();
    m.apply([insert(Some("t"), 2, row("r3", vec![cell("c5", "Dory"), cell("c6", "5 m")]))]).unwrap();
    // A column is one more cell in every row and one more alignment, in one edit.
    let wider = columns(&[Align::Start, Align::End, Align::Center]);
    let cells = ["r1", "r2", "r3"].iter().enumerate().map(|(n, row)| insert(Some(row), 2, cell(&format!("x{n}"), "")));
    let change = m.apply(cells.chain([Op::SetKind { block: id("t"), kind: wider.clone() }])).unwrap();

    assert_eq!(outline(m.doc()), "p g(f1 f2 cap) t(r1(c1 c2 x0) r2(c3 c4 x1) r3(c5 c6 x2))");
    assert_eq!(m.doc().block(&id("t")).unwrap().kind, wider);
    assert!(change.structure && change.blocks.contains(&id("t")));
    m.apply([typed("x1", 0, "Oak")]).unwrap();
    assert_eq!(m.doc().block(&id("x1")).unwrap().plain_text(), "Oak");
    assert!(stored(m.doc()));

    for _ in 0..3 {
        m.undo().unwrap();
    }
    assert_eq!(m.doc(), model().doc());
}

#[test]
fn a_block_is_refused_where_it_cannot_sit_or_as_it_cannot_be() {
    let paragraph = || Block::text("x", BlockKind::Paragraph, "Stray");
    let both = BlockKind::Figure {
        asset: Some("assets/a.png".into()),
        src: Some("https://example.com/a.png".into()),
        alt: String::new(),
        width: 240.0,
        height: 160.0,
        numbered: false,
    };
    let neither =
        BlockKind::Figure { asset: None, src: None, alt: String::new(), width: 240.0, height: 160.0, numbered: false };
    let quote = holding("q", BlockKind::Quote, vec![paragraph()]);
    let kind = |block: &str, kind: BlockKind| Op::SetKind { block: id(block), kind };
    let cases: Vec<(&str, Vec<Op>, OpError)> = vec![
        ("a paragraph in a gallery", vec![insert(Some("g"), 1, paragraph())], OpError::WrongBody(id("g"))),
        ("a cell straight in a table", vec![insert(Some("t"), 0, cell("x", ""))], OpError::WrongBody(id("t"))),
        ("a row in a row", vec![insert(Some("r1"), 0, row("x", vec![]))], OpError::WrongBody(id("r1"))),
        (
            "a row made with a paragraph",
            vec![insert(None, 0, row("r", vec![paragraph()]))],
            OpError::WrongBody(id("r")),
        ),
        ("a gallery's figure made a paragraph", vec![kind("f1", BlockKind::Paragraph)], OpError::WrongBody(id("f1"))),
        (
            "a quote of text made a gallery",
            vec![insert(None, 0, quote), kind("q", BlockKind::Gallery { numbered: false })],
            OpError::WrongBody(id("q")),
        ),
        ("a figure with no picture", vec![kind("f1", neither)], OpError::BadKind(id("f1"))),
        ("a figure with two pictures", vec![insert(None, 0, Block::new("x", both))], OpError::BadKind(id("x"))),
        ("a figure with no width", vec![kind("f1", picture("assets/a.png", 0.0, 160.0))], OpError::BadKind(id("f1"))),
        (
            "a figure with no number for a width",
            vec![kind("f1", picture("assets/a.png", f32::NAN, 160.0))],
            OpError::BadKind(id("f1")),
        ),
    ];
    for (name, ops, expected) in cases {
        let mut m = model();
        assert_eq!(m.apply(ops), Err(expected), "{name}");
        assert_eq!(m.doc(), model().doc(), "{name}");
    }
}

#[test]
fn a_figure_is_resized_and_its_picture_swapped() {
    for kind in [picture("assets/f1.png", 120.0, 80.5), picture("assets/other.jpg", 240.0, 160.0)] {
        let mut m = model();
        let change = m.apply([Op::SetKind { block: id("f1"), kind: kind.clone() }]).unwrap();
        let figure = m.doc().block(&id("f1")).unwrap();
        assert_eq!((&figure.kind, figure.plain_text().as_str()), (&kind, "Boats"));
        assert!(change.blocks.contains(&id("f1")) && !change.structure);
        assert_eq!(outline(m.doc()), OUTLINE);

        m.undo().unwrap();
        assert_eq!(m.doc(), model().doc());
    }
}

#[test]
fn a_block_keeps_its_id_when_it_moves() {
    let mut m = model();
    let named = Op::SetMark { block: id("p"), range: 4..12, mark: Mark::Ref(id("f1")), on: true };
    m.apply([named, remove("f2"), remove("cap")]).unwrap();
    let figure = m.doc().block(&id("f1")).unwrap().clone();

    // The last picture leaves its gallery, and the gallery goes, in one edit.
    m.apply([remove("g"), insert(None, 1, figure.clone())]).unwrap();
    assert_eq!(outline(m.doc()), "p f1 t(r1(c1 c2) r2(c3 c4))");
    assert_eq!(m.doc().block(&id("f1")), Some(&figure));

    // Cut and pasted, it is the same block still. A copy of it needs a name of its own.
    m.apply([remove("f1")]).unwrap();
    m.apply([insert(Some("r1"), 0, cell("x", "")), insert(None, 2, figure.clone())]).unwrap();
    assert_eq!(outline(m.doc()), "p t(r1(x c1 c2) r2(c3 c4)) f1");
    assert_eq!(m.apply([insert(None, 0, figure)]), Err(OpError::DuplicateId(id("f1"))));
    let refers = |run: &letters_model::Run| run.marks.contains(&Mark::Ref(id("f1")));
    assert!(m.doc().blocks[0].body.runs().iter().any(refers), "what named the block names it still");
}

#[test]
fn a_reference_names_one_block_and_ends_with_its_text() {
    let refer = |target: &str, on: bool| Op::SetMark { block: id("p"), range: 4..12, mark: Mark::Ref(id(target)), on };
    let runs = |m: &Model| -> Vec<(String, Vec<BlockId>)> {
        let target = |mark: &Mark| match mark {
            Mark::Ref(target) => Some(target.clone()),
            _ => None,
        };
        let run = |run: &letters_model::Run| (run.text.clone(), run.marks.iter().filter_map(target).collect());
        m.doc().blocks[0].body.runs().iter().map(run).collect()
    };
    let mut m = model();
    m.apply([refer("f1", true)]).unwrap();
    assert_eq!(runs(&m), [("See ".into(), vec![]), ("Figure 1".into(), vec![id("f1")]), (".".into(), vec![])]);

    // Text typed at either end of it stands beside it. Text typed inside is part of it.
    m.apply([typed("p", 12, "b")]).unwrap();
    m.apply([typed("p", 4, "a")]).unwrap();
    m.apply([typed("p", 8, "-")]).unwrap();
    assert_eq!(runs(&m), [("See a".into(), vec![]), ("Fig-ure 1".into(), vec![id("f1")]), ("b.".into(), vec![])]);

    // The block may go: what named it is left for the reader to show as broken.
    m.apply([remove("f1")]).unwrap();
    assert_eq!(runs(&m)[1].1, [id("f1")]);
    assert!(stored(m.doc()));

    let range = 5..14;
    m.apply([Op::SetMark { block: id("p"), range: range.clone(), mark: Mark::Ref(id("f2")), on: true }]).unwrap();
    assert_eq!(runs(&m)[1].1, [id("f2")], "a second name takes the place of the first");
    m.apply([Op::SetMark { block: id("p"), range, mark: Mark::Ref(id("f2")), on: false }]).unwrap();
    assert_eq!(runs(&m), [("See aFig-ure 1b.".into(), vec![])]);
}

#[test]
fn what_a_container_cannot_hold_sits_after_it() {
    // A tree made by hand may nest anything. The model reads it as a merge that led there would be read.
    let gallery = vec![figure("f1", ""), Block::text("x", BlockKind::Paragraph, "Stray"), figure("f2", "")];
    let table = vec![row("r1", vec![cell("c1", "")]), cell("c2", "Loose"), row("r2", vec![])];
    let blocks = vec![
        holding("g", BlockKind::Gallery { numbered: false }, gallery),
        holding("t", columns(&[Align::Start]), table),
    ];
    let m = Model::new(Document { blocks, ..Document::default() }, ReplicaId::from(1));
    assert_eq!(outline(m.doc()), "g(f1) x f2 t(r1(c1)) c2 r2");
    assert!(stored(m.doc()));
    assert_eq!(Model::load(&m.save(), ReplicaId::from(2)).unwrap().doc(), m.doc());
}

#[test]
fn a_type_made_as_an_unknown_block_keeps_what_it_holds() {
    // A reader that does not build figures yet hands one over as an unknown block, with its fields beside it.
    let mut block = Block::new("f", BlockKind::Unknown { type_name: "figure".into() });
    let fields = serde_json::json!({
        "asset": "assets/a.png", "alt": "A boat", "width": 320, "height": 240.5,
        "content": [{ "text": "Caption" }], "fallback": "Picture",
    });
    block.extra.extend(fields.as_object().unwrap().clone());
    let m = Model::new(Document { blocks: vec![block], ..Document::default() }, ReplicaId::from(1));

    let figure = &m.doc().blocks[0];
    let BlockKind::Figure { asset, alt, width, height, .. } = &figure.kind else { panic!("{:?}", figure.kind) };
    assert_eq!((asset.as_deref(), alt.as_str(), *width, *height), (Some("assets/a.png"), "A boat", 320.0, 240.5));
    assert_eq!((figure.plain_text().as_str(), &figure.extra["fallback"]), ("Caption", &"Picture".into()));
    assert_eq!(Model::load(&m.save(), ReplicaId::from(2)).unwrap().doc(), m.doc());
}

#[test]
fn two_people_changing_one_block_of_parts_both_keep_their_work() {
    let column = |rows: &[&str]| -> Vec<Op> {
        let cells = rows.iter().map(|row| insert(Some(row), 2, cell(&format!("x{row}"), "")));
        let wider = columns(&[Align::Start, Align::End, Align::Center]);
        cells.chain([Op::SetKind { block: id("t"), kind: wider }]).collect()
    };
    // A name, what each of two replicas does unseen by the other, and the outline once merged.
    let cases: Vec<(&str, Vec<Op>, Vec<Op>, &str)> = vec![
        (
            "a column and a row, which is left a cell short",
            column(&["r1", "r2"]),
            vec![insert(Some("t"), 2, row("r3", vec![cell("c5", ""), cell("c6", "")]))],
            "p g(f1 f2 cap) t(r1(c1 c2 xr1) r2(c3 c4 xr2) r3(c5 c6))",
        ),
        (
            "typing in a cell whose column is removed, which is left a cell",
            vec![remove("c1"), remove("c3")],
            vec![typed("c1", 4, "s")],
            "p g(f1 f2 cap) t(r1(r1~ c2) r2(c4))",
        ),
        (
            "typing in a caption whose figure is removed, which is left a caption",
            vec![remove("f1")],
            vec![typed("f1", 5, "!")],
            "p g(g~ f2 cap) t(r1(c1 c2) r2(c3 c4))",
        ),
        (
            "a row put in a table that is removed, which is left without one",
            vec![remove("t")],
            vec![insert(Some("t"), 1, row("r3", vec![cell("c5", "Dory")]))],
            "p g(f1 f2 cap) r3(c5)",
        ),
        (
            "a second caption",
            vec![insert(Some("g"), 3, Block::text("cap2", BlockKind::Caption, "Ours"))],
            vec![insert(Some("g"), 3, Block::text("cap3", BlockKind::Caption, "Theirs"))],
            "p g(f1 f2 cap cap2 cap3) t(r1(c1 c2) r2(c3 c4))",
        ),
    ];
    for (name, ours, theirs, merged) in cases {
        let (mut a, mut b) = (model(), Model::new(doc(), ReplicaId::from(2)));
        a.apply(ours).unwrap();
        b.apply(theirs).unwrap();
        sync(&mut a, &mut b);
        assert_eq!(a.doc(), b.doc(), "{name}");
        assert_eq!(outline(a.doc()), merged, "{name}");
        assert!(stored(a.doc()), "{name}");
    }
}

#[test]
fn a_block_merged_into_a_container_that_cannot_hold_it_sits_after_it() {
    let pictures = vec![figure("f3", ""), figure("f4", "")];
    let blocks = vec![holding("q", BlockKind::Quote, pictures)];
    let start = || Document { blocks: blocks.clone(), ..Document::default() };
    let (mut a, mut b) = (Model::new(start(), ReplicaId::from(1)), Model::new(start(), ReplicaId::from(2)));
    a.apply([Op::SetKind { block: id("q"), kind: BlockKind::Gallery { numbered: false } }]).unwrap();
    b.apply([insert(Some("q"), 1, Block::text("x", BlockKind::Paragraph, "Aside"))]).unwrap();
    sync(&mut a, &mut b);

    assert_eq!(a.doc(), b.doc());
    assert_eq!(outline(a.doc()), "q(f3) x f4");
    assert!(stored(a.doc()));
    // Each can still be edited where it now sits.
    a.apply([typed("x", 5, "!"), insert(Some("q"), 1, figure("f5", ""))]).unwrap();
    assert_eq!(outline(a.doc()), "q(f3 f5) x f4");
}
