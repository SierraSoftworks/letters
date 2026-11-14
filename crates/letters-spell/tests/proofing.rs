//! Checking a document a block at a time, through the public interface.

use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Model, Op, Pos, ReplicaId, Run};
use letters_spell::{Checked, Dictionary, Proofing, Speller, language_at};

fn doc(language: &str, blocks: Vec<Block>) -> Document {
    Document { language: language.into(), blocks, ..Document::default() }
}

fn paragraph(id: &str, text: &str) -> Block {
    Block::text(id, BlockKind::Paragraph, text)
}

/// Three paragraphs in British English: `a` is clean, `b` has two typos and `c` has one.
fn letter() -> Document {
    doc(
        "en-GB",
        vec![
            paragraph("a", "Dear Ms Okafor,"),
            paragraph("b", "Thank you for yuor letter abuot the colour."),
            paragraph("c", "Yours sincerly,"),
        ],
    )
}

/// Steps until every block is up to date, and returns the ids of the blocks checked, in order.
fn settle(proofing: &mut Proofing, speller: &Speller, doc: &Document) -> Vec<String> {
    std::iter::from_fn(|| proofing.step(speller, doc)).map(|checked| checked.block.as_str().to_owned()).collect()
}

/// The words flagged in a block.
fn flagged(proofing: &Proofing, doc: &Document, id: &str) -> Vec<String> {
    let text = doc.block(&id.into()).map(Block::plain_text).unwrap_or_default();
    proofing.marks(&id.into()).iter().map(|range| text[range.clone()].to_owned()).collect()
}

/// A settled check of `doc` with the bundled English.
fn proofed(doc: &Document) -> (Proofing, Speller) {
    let (mut proofing, speller) = (Proofing::new(), Speller::new());
    settle(&mut proofing, &speller, doc);
    (proofing, speller)
}

#[test]
fn a_document_is_checked_one_block_per_step() {
    let (doc, mut speller, mut proofing) = (letter(), Speller::new(), Proofing::new());
    assert_eq!(proofing.step(&speller, &doc), Some(Checked { block: "a".into(), changed: true }));
    assert_eq!(flagged(&proofing, &doc, "a"), ["Okafor"]);
    assert_eq!(flagged(&proofing, &doc, "b"), [""; 0], "not reached yet");
    assert_eq!(proofing.step(&speller, &doc), Some(Checked { block: "b".into(), changed: true }));
    assert_eq!(flagged(&proofing, &doc, "b"), ["yuor", "abuot"]);
    assert_eq!(settle(&mut proofing, &speller, &doc), ["c"]);
    assert_eq!(flagged(&proofing, &doc, "c"), ["sincerly"]);
    assert_eq!(proofing.step(&speller, &doc), None);

    // A block checked again with the same result needs no repaint.
    speller.set_user_words(["zorbl"]);
    assert_eq!(proofing.step(&speller, &doc), Some(Checked { block: "a".into(), changed: false }));
}

#[test]
fn only_prose_is_checked() {
    let run = |text: &str, marks: &[Mark]| Run::marked(text, marks.iter().cloned());
    let text = |id: &str, kind: BlockKind, runs: Vec<Run>| Block { body: Body::Text(runs), ..Block::new(id, kind) };
    let cases = [
        (text("code", BlockKind::Code { language: None }, vec![run("fn mian() { retrun }", &[])]), vec![]),
        (text("inline", BlockKind::Paragraph, vec![run("Call ", &[]), run("retrun", &[Mark::Code])]), vec![]),
        // A word is one word however its letters are formatted.
        (
            text("split", BlockKind::Paragraph, vec![run("A wro", &[]), run("ngg", &[Mark::Bold]), run(" word", &[])]),
            vec!["wrongg"],
        ),
        (text("heading", BlockKind::Heading { level: 1 }, vec![run("Introductoin", &[])]), vec!["Introductoin"]),
        (text("newer", BlockKind::Unknown { type_name: "chart".into() }, vec![run("xlabl", &[])]), vec![]),
        (Block::new("rule", BlockKind::Divider), vec![]),
    ];
    for (block, expected) in cases {
        let id = block.id.as_str().to_owned();
        let doc = doc("en-GB", vec![block]);
        let (proofing, _) = proofed(&doc);
        assert_eq!(flagged(&proofing, &doc, &id), expected, "{id}");
    }
}

#[test]
fn nested_blocks_are_checked() {
    let quote = Block {
        body: Body::Blocks(vec![paragraph("inner", "A quoted mistaek.")]),
        ..Block::new("q", BlockKind::Quote)
    };
    let doc = doc("en-GB", vec![quote]);
    let (proofing, _) = proofed(&doc);
    assert_eq!(flagged(&proofing, &doc, "inner"), ["mistaek"]);
}

#[test]
fn an_edit_rechecks_only_its_block_and_first() {
    let mut model = Model::new(letter(), ReplicaId::from(1));
    let (mut proofing, speller) = proofed(model.doc());

    let change = model.apply([Op::DeleteText { block: "c".into(), range: 0..6 }]).unwrap();
    proofing.invalidate(&change);
    // The old offsets no longer fit the text, so they are gone until the block is checked again.
    assert_eq!(flagged(&proofing, model.doc(), "c"), [""; 0]);
    assert_eq!(flagged(&proofing, model.doc(), "b"), ["yuor", "abuot"]);
    assert_eq!(settle(&mut proofing, &speller, model.doc()), ["c"]);
    assert_eq!(flagged(&proofing, model.doc(), "c"), ["sincerly"]);

    // An edit made while the first pass is still running jumps the queue.
    let mut proofing = Proofing::new();
    proofing.step(&speller, model.doc());
    let change = model.apply([Op::InsertText { at: Pos::new("c", 0), text: "Yuors ".into() }]).unwrap();
    proofing.invalidate(&change);
    assert_eq!(settle(&mut proofing, &speller, model.doc()), ["c", "b"]);
    assert_eq!(flagged(&proofing, model.doc(), "c"), ["Yuors", "sincerly"]);
}

#[test]
fn undo_and_removed_blocks_are_followed() {
    let mut model = Model::new(letter(), ReplicaId::from(1));
    let (mut proofing, speller) = proofed(model.doc());

    let change = model.apply([Op::RemoveBlock { block: "b".into() }]).unwrap();
    proofing.invalidate(&change);
    assert_eq!(settle(&mut proofing, &speller, model.doc()), [""; 0]);
    assert_eq!(proofing.marks(&"b".into()), []);

    proofing.invalidate(&model.undo().unwrap());
    assert_eq!(settle(&mut proofing, &speller, model.doc()), ["b"]);
    assert_eq!(flagged(&proofing, model.doc(), "b"), ["yuor", "abuot"]);
}

#[test]
fn a_change_of_words_or_language_is_noticed_without_being_told() {
    type Alter = fn(&mut Document, &mut Speller);
    /// A few words and every letter, standing in for a dictionary the shell loads.
    const ENGLISH: &str = "9\nDear\nMs\nOkafor\nThank\nyou\nfor\nletter\nthe\nabcdefghijklmnopqrstuvwxyz\n";
    let cases: [(&str, Alter, &[&str]); 6] = [
        ("nothing", |_, _| (), &["Okafor", "yuor", "abuot", "sincerly"]),
        ("add to this document", |doc, _| _ = doc.dictionary.insert("Okafor".into()), &["yuor", "abuot", "sincerly"]),
        ("add to my dictionary", |_, speller| speller.set_user_words(["Okafor", "abuot"]), &["yuor", "sincerly"]),
        ("another English", |doc, _| doc.language = "en-US".into(), &["Okafor", "yuor", "abuot", "colour", "sincerly"]),
        ("a language with no dictionary", |doc, _| doc.language = "nl".into(), &[]),
        (
            "a dictionary arrives",
            |_, speller| speller.add_dictionary("en-GB", Dictionary::parse("", ENGLISH).unwrap()),
            &["yuor", "abuot", "colour", "Yours", "sincerly"],
        ),
    ];
    for (name, alter, expected) in cases {
        let mut doc = letter();
        let (mut proofing, mut speller) = proofed(&doc);
        alter(&mut doc, &mut speller);
        // The text is unchanged, so what was flagged stays in place until each block is checked again.
        assert_eq!(flagged(&proofing, &doc, "b"), ["yuor", "abuot"], "{name}");
        settle(&mut proofing, &speller, &doc);
        let all: Vec<String> = ["a", "b", "c"].iter().flat_map(|id| flagged(&proofing, &doc, id)).collect();
        assert_eq!(all, expected, "{name}");
    }
}

#[test]
fn a_block_or_run_can_be_in_another_language() {
    // Read from JSON, as the `lang` a document carries is: a model that names the field must keep this working.
    let doc = Document::from_json(
        r#"{ "format": 1, "language": "en-GB", "blocks": [
            { "id": "plain", "type": "paragraph", "content": [{ "text": "Colour and color." }] },
            { "id": "american", "type": "paragraph", "lang": "en-US", "content": [{ "text": "Colour and color." }] },
            { "id": "french", "type": "paragraph", "lang": "fr", "content": [{ "text": "Bonjour tout le monde." }] },
            { "id": "quote", "type": "quote", "lang": "en-US", "blocks": [
                { "id": "mixed", "type": "paragraph", "content": [
                    { "text": "Colour and color, " },
                    { "text": "colour and color", "lang": "en-GB" },
                    { "text": ", et voilà mes amis.", "lang": "fr" }] }] }
        ] }"#,
    )
    .unwrap();
    let (proofing, _) = proofed(&doc);
    let flags: [(&str, &[&str]); 4] =
        [("plain", &["color"]), ("american", &["Colour"]), ("french", &[]), ("mixed", &["Colour", "color"])];
    for (id, expected) in flags {
        assert_eq!(flagged(&proofing, &doc, id), expected, "{id}");
    }
    // Offset 18 is where the second run of "mixed" starts.
    let languages = [
        ("plain", 0, "en-GB"),
        ("american", 5, "en-US"),
        ("french", 0, "fr"),
        ("mixed", 0, "en-US"),
        ("mixed", 18, "en-GB"),
        ("mixed", 40, "fr"),
    ];
    for (id, offset, language) in languages {
        assert_eq!(language_at(&doc, &id.into(), offset), Some(language), "{id} at {offset}");
    }
    assert_eq!(language_at(&doc, &BlockId::new("missing"), 0), None);
}

#[test]
fn another_document_starts_afresh() {
    let (mut proofing, speller) = proofed(&letter());
    let other = doc("en-GB", vec![paragraph("b", "A diferent letter.")]);
    proofing.reset();
    assert_eq!(proofing.marks(&"b".into()), []);
    assert_eq!(settle(&mut proofing, &speller, &other), ["b"]);
    assert_eq!(flagged(&proofing, &other, "b"), ["diferent"]);
}
