//! What an export says it will simplify, before it writes.

use letters_convert::{Loss, LossKind, Target, losses, losses_with};
use letters_model::{Anchor, Block, BlockKind, Body, Document, Mark, Note, NoteKind, Notes, Run, Tone, Version};

/// The formats a document is written in for another program to read.
const TARGETS: [Target; 4] = [Target::Markdown, Target::Html, Target::Docx, Target::Pdf];

fn figure(id: &str, asset: Option<&str>, src: Option<&str>) -> Block {
    let (asset, src) = (asset.map(String::from), src.map(String::from));
    Block::new(id, BlockKind::Figure { asset, src, alt: "A cat".into(), width: 80.0, height: 60.0, numbered: false })
}

fn holding(id: &str, kind: BlockKind, inside: Vec<Block>) -> Block {
    Block { body: Body::Blocks(inside), ..Block::new(id, kind) }
}

fn note(id: &str, kind: NoteKind) -> Note {
    Note {
        id: id.into(),
        kind,
        anchor: Anchor::range("p", 0..5),
        author: "Ada".into(),
        created: "2026-11-14T20:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain("Which rates?")],
        replies: Vec::new(),
        extra: Default::default(),
    }
}

/// A document with everything some format cannot carry, and its notes: two comments, an import note, and
/// two conflicts of which one is settled.
fn everything() -> (Document, Notes) {
    let mut odd = Block::text("odd", BlockKind::Paragraph, "");
    odd.body = Body::Text(vec![Run::marked("Sparkling", [Mark::Other("sparkle".into())])]);
    let mut french = Block::text("fr", BlockKind::Paragraph, "Bonjour");
    french.lang = Some("fr".into());
    let mut mixed = Block::text("mixed", BlockKind::Paragraph, "");
    mixed.body = Body::Text(vec![Run::plain("As they say, "), Run { lang: Some("de".into()), ..Run::plain("genau") }]);
    let mut later = Block::text("later", BlockKind::Heading { level: 2 }, "Later");
    later.page_break = true;
    let bright = BlockKind::Callout { tone: Tone::Tip, title: None, icon: Some("star".into()), color: None };
    let preview =
        BlockKind::LinkPreview { url: "https://example.com/".into(), title: None, description: None, image: None };
    let nested = vec![
        Block::text("q1", BlockKind::Paragraph, "Kept in the quote"),
        Block::text("q2", BlockKind::Code { language: Some("rust".into()) }, "let rate = 4;"),
        Block::text("q3", BlockKind::Heading { level: 3 }, "Inside"),
        // Word keeps this one in the quote, as it does the paragraph.
        Block::new("q4", preview.clone()),
    ];
    let blocks = vec![
        Block::text("p", BlockKind::Paragraph, "Rates change in January."),
        Block::new("chart", BlockKind::Unknown { type_name: "chart".into() }),
        odd,
        french,
        mixed,
        later,
        figure("held", Some("assets/cat.png"), None),
        figure("linked", None, Some("https://example.com/cat.png")),
        holding(
            "gallery",
            BlockKind::Gallery { numbered: false },
            vec![figure("g1", Some("assets/a.png"), None), figure("g2", Some("assets/b.png"), None)],
        ),
        holding("tip", bright, vec![Block::text("t1", BlockKind::Paragraph, "A tip")]),
        holding("plain", BlockKind::callout(Tone::Info), vec![Block::text("i1", BlockKind::Paragraph, "A note")]),
        holding("quote", BlockKind::Quote, nested),
        Block::text("bare", BlockKind::Code { language: None }, "plain text"),
        Block::new("toc", BlockKind::TableOfContents { levels: 3 }),
        Block::new("card", preview),
    ];
    let version = || Version {
        author: "Ada".into(),
        edited: "2026-11-14T20:00:00Z".into(),
        block: None,
        extra: Default::default(),
    };
    let conflict = |id: &str| note(id, NoteKind::Conflict { versions: Box::new([version(), version()]) });
    let settled = Note { resolved: true, ..conflict("n5") };
    let notes = vec![
        note("n1", NoteKind::Comment),
        note("n2", NoteKind::Comment),
        note("n3", NoteKind::Import),
        conflict("n4"),
        settled,
    ];
    (Document { blocks, ..Document::default() }, Notes { notes, ..Notes::default() })
}

#[test]
fn each_format_lists_what_it_cannot_carry() {
    use LossKind::*;
    let (doc, notes) = everything();
    let cases = [
        (
            Target::Markdown,
            vec![
                (UnknownBlocks, 1),
                (UnknownMarks, 1),
                (Comments, 2),
                (CalloutLooks, 1),
                (Galleries, 1),
                (PageBreaks, 1),
                (Languages, 2),
                // Its table of contents is written as the list it shows.
                (LinkPreviews, 2),
            ],
        ),
        (Target::Html, vec![(UnknownBlocks, 1), (UnknownMarks, 1), (Comments, 2), (LinkedPictures, 1)]),
        (
            Target::Docx,
            vec![
                (UnknownBlocks, 1),
                (UnknownMarks, 1),
                (LinkedPictures, 1),
                (NestedBlocks, 2),
                // Word shows no icon, so the callout that keeps its tone's loses it too.
                (CalloutLooks, 2),
                (CodeLanguages, 1),
            ],
        ),
        (Target::Pdf, vec![(UnknownBlocks, 1), (UnknownMarks, 1), (Comments, 2), (LinkedPictures, 1)]),
        // A clean copy is the document itself, without what is kept beside it.
        (Target::Clean, vec![(Comments, 2), (History, 1), (Conflicts, 1)]),
    ];
    for (to, expected) in cases {
        let mut found: Vec<(LossKind, usize)> =
            losses(&doc, &notes, to).iter().map(|loss| (loss.kind, loss.count)).collect();
        found.sort();
        assert_eq!(found, expected, "{to:?}");
    }
}

#[test]
fn a_document_every_format_carries_loses_nothing() {
    let blocks = vec![
        Block::text("h", BlockKind::Heading { level: 1 }, "Rates"),
        Block::text("p", BlockKind::Paragraph, "Rates change in January."),
        figure("held", Some("assets/cat.png"), None),
        holding("quote", BlockKind::Quote, vec![Block::text("q", BlockKind::Paragraph, "As was said.")]),
    ];
    let doc = Document { blocks, ..Document::default() };
    let notes = Notes { notes: vec![note("n", NoteKind::Import)], ..Notes::default() };
    for to in TARGETS {
        assert_eq!(losses(&doc, &notes, to), Vec::<Loss>::new(), "{to:?}");
    }
}

#[test]
fn a_loss_reads_as_a_line_that_says_how_many() {
    let (doc, notes) = everything();
    for to in TARGETS.into_iter().chain([Target::Clean]) {
        for loss in losses(&doc, &notes, to) {
            let line = loss.to_string();
            let counted = loss.kind != LossKind::History;
            assert!(!counted || line.contains(&loss.count.to_string()), "{to:?} {:?}: {line}", loss.kind);
            assert!(!line.ends_with('.') && !line.contains('\n'), "one line, for a list: {line}");
        }
    }
    // One and many are worded apart.
    let lines = |count| TARGETS.map(|to| Loss { kind: LossKind::Comments, count, to }.to_string());
    assert_ne!(lines(1).map(|line| line.replace('1', "")), lines(3).map(|line| line.replace('3', "")));
}

#[test]
fn a_linked_picture_the_shell_has_loaded_is_no_loss_in_a_pdf() {
    let (doc, notes) = everything();
    let loaded = |address: &str| address == "https://example.com/cat.png";
    for to in TARGETS {
        let before = losses(&doc, &notes, to);
        let after = losses_with(&doc, &notes, to, &loaded);
        let linked = |losses: &[Loss]| losses.iter().any(|loss| loss.kind == LossKind::LinkedPictures);
        assert_eq!(linked(&after), linked(&before) && to != Target::Pdf, "{to:?}: only a PDF shows it");
        assert_eq!(after.len(), before.len() - usize::from(to == Target::Pdf), "{to:?}: the rest stands");
    }
}
