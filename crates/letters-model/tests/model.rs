use letters_model::{Block, BlockId, BlockKind, Document, Mark, Model, Op, OpError, Pos};

/// One paragraph `p` reading "Hello world", then a divider `d`.
fn model() -> Model {
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "Hello world"), Block::new("d", BlockKind::Divider)];
    Model::new(Document { blocks, ..Document::default() })
}

fn id(id: &str) -> BlockId {
    id.into()
}

fn texts(model: &Model) -> Vec<String> {
    model.doc().walk().map(Block::plain_text).collect()
}

#[test]
fn text_edits_change_the_text() {
    let mut m = model();
    m.apply([Op::InsertText { at: Pos::new("p", 5), text: ",".into() }]).unwrap();
    m.apply([
        Op::DeleteText { block: id("p"), range: 0..1 },
        Op::InsertText { at: Pos::new("p", 0), text: "J".into() },
    ])
    .unwrap();
    assert_eq!(texts(&m)[0], "Jello, world");
}

#[test]
fn marks_cover_exactly_their_range() {
    let mut m = model();
    m.apply([Op::SetMark { block: id("p"), range: 6..11, mark: Mark::Bold, on: true }]).unwrap();
    m.apply([Op::SetMark { block: id("p"), range: 8..11, mark: Mark::Bold, on: false }]).unwrap();
    let bold: String = m.doc().blocks[0]
        .body
        .runs()
        .iter()
        .filter(|r| r.marks.contains(&Mark::Bold))
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(bold, "wo");
    assert_eq!(texts(&m)[0], "Hello world");
}

#[test]
fn inserted_text_takes_the_marks_before_it() {
    let mut m = model();
    m.apply([Op::SetMark { block: id("p"), range: 0..5, mark: Mark::Italic, on: true }]).unwrap();
    m.apply([Op::InsertText { at: Pos::new("p", 5), text: "!".into() }]).unwrap();
    let runs = m.doc().blocks[0].body.runs();
    assert_eq!((runs[0].text.as_str(), runs[0].marks.contains(&Mark::Italic)), ("Hello!", true));
}

#[test]
fn split_and_join_are_inverses() {
    let mut m = model();
    let before = m.doc().clone();
    let change = m.apply([Op::Split { at: Pos::new("p", 5), new_id: id("p2") }]).unwrap();
    assert_eq!(texts(&m), ["Hello", " world", ""]);
    assert!(change.structure && change.blocks.contains(&id("p2")));
    m.apply([Op::Join { block: id("p2") }]).unwrap();
    assert_eq!(m.doc(), &before);
}

#[test]
fn blocks_nest_inside_containers() {
    let mut m = model();
    m.apply([
        Op::InsertBlock { parent: None, index: 0, block: Block::new("q", BlockKind::Quote) },
        Op::InsertBlock {
            parent: Some(id("q")),
            index: 0,
            block: Block::text("inner", BlockKind::Paragraph, "Quoted"),
        },
        Op::SetKind { block: id("p"), kind: BlockKind::Heading { level: 2 } },
    ])
    .unwrap();
    assert_eq!(texts(&m), ["", "Quoted", "Hello world", ""]);
    assert_eq!(m.doc().block(&id("p")).unwrap().kind, BlockKind::Heading { level: 2 });
    m.apply([Op::RemoveBlock { block: id("q") }]).unwrap();
    assert!(m.doc().block(&id("inner")).is_none());
}

#[test]
fn undo_and_redo_step_through_whole_edits() {
    let mut m = model();
    let original = m.doc().clone();
    m.apply([Op::InsertText { at: Pos::new("p", 0), text: "A".into() }, Op::RemoveBlock { block: id("d") }]).unwrap();
    let edited = m.doc().clone();

    assert!(m.undo().unwrap().blocks.contains(&id("d")));
    assert_eq!(m.doc(), &original);
    m.redo().unwrap();
    assert_eq!(m.doc(), &edited);
    assert!(m.redo().is_none());
}

#[test]
fn a_refused_edit_changes_nothing() {
    let cases: Vec<(Vec<Op>, OpError)> = vec![
        (vec![Op::InsertText { at: Pos::new("nope", 0), text: "x".into() }], OpError::NoSuchBlock(id("nope"))),
        (vec![Op::InsertText { at: Pos::new("d", 0), text: "x".into() }], OpError::WrongBody(id("d"))),
        (vec![Op::DeleteText { block: id("p"), range: 3..99 }], OpError::BadOffset { block: id("p"), offset: 99 }),
        (vec![Op::Split { at: Pos::new("p", 1), new_id: id("d") }], OpError::DuplicateId(id("d"))),
        (vec![Op::Join { block: id("p") }], OpError::NothingToJoin(id("p"))),
        (vec![Op::SetKind { block: id("p"), kind: BlockKind::Quote }], OpError::WrongBody(id("p"))),
        (
            vec![Op::InsertBlock { parent: None, index: 9, block: Block::new("n", BlockKind::Divider) }],
            OpError::BadIndex { index: 9 },
        ),
        // The first operation is valid, and must be rolled back with the second.
        (
            vec![Op::InsertText { at: Pos::new("p", 0), text: "x".into() }, Op::RemoveBlock { block: id("nope") }],
            OpError::NoSuchBlock(id("nope")),
        ),
    ];
    for (ops, expected) in cases {
        let mut m = model();
        assert_eq!(m.apply(ops), Err(expected));
        assert_eq!(m.doc(), model().doc());
        assert!(m.undo().is_none());
    }
}

#[test]
fn offsets_inside_a_character_are_refused() {
    let mut m =
        Model::new(Document { blocks: vec![Block::text("p", BlockKind::Paragraph, "é")], ..Document::default() });
    let error = m.apply([Op::InsertText { at: Pos::new("p", 1), text: "x".into() }]);
    assert_eq!(error, Err(OpError::BadOffset { block: id("p"), offset: 1 }));
}
