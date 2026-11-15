use letters_model::{Block, BlockId, BlockKind, Document, Mark, Model, Op, OpError, Pos, ReplicaId};

/// One paragraph `p` reading "Hello world", then a divider `d`.
fn model() -> Model {
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "Hello world"), Block::new("d", BlockKind::Divider)];
    Model::new(Document { blocks, ..Document::default() }, ReplicaId::from(1))
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
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "é")];
    let mut m = Model::new(Document { blocks, ..Document::default() }, ReplicaId::from(1));
    let error = m.apply([Op::InsertText { at: Pos::new("p", 1), text: "x".into() }]);
    assert_eq!(error, Err(OpError::BadOffset { block: id("p"), offset: 1 }));
}

#[test]
fn a_block_carries_a_page_break_and_a_language() {
    // What the block is given, and what it then carries.
    let cases = [
        (true, Some("fr"), (true, Some("fr"))),
        (true, None, (true, None)),
        (false, Some("de-CH"), (false, Some("de-CH"))),
        (false, Some(""), (false, None)),
    ];
    for (page_break, lang, carried) in cases {
        let mut m = model();
        let before = m.doc().clone();
        let op = Op::SetBlockProps { block: id("d"), page_break, lang: lang.map(str::to_owned) };

        let change = m.apply([op.clone()]).unwrap();
        let block = m.doc().block(&id("d")).unwrap();
        assert_eq!((block.page_break, block.lang.as_deref()), carried, "{op:?}");
        assert_eq!(change.blocks.contains(&id("d")), m.doc() != &before, "{op:?}: layout hears of it");
        assert!(!change.structure, "{op:?}");
        assert_eq!(m.apply([op.clone()]), Ok(Default::default()), "{op:?}: set again, it changes nothing");

        // A new kind leaves both as they are.
        m.apply([Op::SetKind { block: id("p"), kind: BlockKind::Heading { level: 1 } }]).unwrap();
        m.undo();
        m.undo();
        assert_eq!(m.doc(), &before, "{op:?}");
    }
}

#[test]
fn text_carries_a_language_that_typing_continues() {
    let mut m = model();
    let lang = |lang: Option<&str>| Op::SetLang { block: id("p"), range: 6..11, lang: lang.map(str::to_owned) };
    m.apply([lang(Some("fr"))]).unwrap();
    m.apply([Op::InsertText { at: Pos::new("p", 11), text: "!".into() }]).unwrap();
    let runs: Vec<(&str, Option<&str>)> =
        m.doc().blocks[0].body.runs().iter().map(|run| (run.text.as_str(), run.lang.as_deref())).collect();
    assert_eq!(runs, [("Hello ", None), ("world!", Some("fr"))]);

    assert_eq!(m.apply([lang(Some("fr"))]), Ok(Default::default()), "set again, it changes nothing");
    m.apply([lang(None)]).unwrap();
    let runs: Vec<(&str, Option<&str>)> =
        m.doc().blocks[0].body.runs().iter().map(|run| (run.text.as_str(), run.lang.as_deref())).collect();
    assert_eq!(runs, [("Hello world", None), ("!", Some("fr"))]);
}

#[test]
fn fields_once_kept_as_unknown_are_read_as_their_own() {
    // A build that did not know these fields kept them among the unknown ones.
    let mut block = Block::text("p", BlockKind::Paragraph, "Bonjour");
    block.extra.insert("break".into(), "page".into());
    block.extra.insert("lang".into(), "fr".into());
    let m = Model::new(Document { blocks: vec![block], ..Document::default() }, ReplicaId::from(1));
    let block = &m.doc().blocks[0];
    assert_eq!((block.page_break, block.lang.as_deref(), block.extra.is_empty()), (true, Some("fr"), true));
}
