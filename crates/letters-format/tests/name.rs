use letters_format::suggested_name;
use letters_model::{Block, BlockKind, Document, Tone};

fn heading(level: u8, text: &str) -> Block {
    Block::text("h", BlockKind::Heading { level }, text)
}

fn document(blocks: impl IntoIterator<Item = Block>) -> Document {
    Document { blocks: blocks.into_iter().collect(), ..Document::default() }
}

fn named(title: &str) -> Option<String> {
    suggested_name(&document([heading(1, title)]))
}

#[test]
fn a_document_is_named_after_its_first_heading_1() {
    let paragraph = Block::text("p", BlockKind::Paragraph, "Not a title");
    let cases = [
        (vec![heading(1, "Cottage renovation")], Some("Cottage renovation")),
        (vec![paragraph.clone(), heading(2, "Budget"), heading(1, "Plan"), heading(1, "Later")], Some("Plan")),
        (vec![paragraph.clone(), heading(2, "Only a section")], None),
        (vec![], None),
    ];
    for (blocks, name) in cases {
        assert_eq!(suggested_name(&document(blocks.clone())).as_deref(), name, "{blocks:?}");
    }
}

#[test]
fn a_heading_inside_a_callout_counts() {
    let mut callout = Block::new("c", BlockKind::Callout { tone: Tone::Info });
    callout.body = letters_model::Body::Blocks(vec![heading(1, "Inside")]);
    assert_eq!(suggested_name(&document([callout])).as_deref(), Some("Inside"));
}

#[test]
fn a_name_is_safe_on_every_file_system() {
    let cases = [
        ("  Rates:  2026/27  ", Some("Rates 2026 27")),
        ("What now?", Some("What now")),
        ("a\\b*c\"d<e>f|g", Some("a b c d e f g")),
        ("Line one\nline\ttwo", Some("Line one line two")),
        (".hidden", Some("hidden")),
        ("Ends with dots...", Some("Ends with dots")),
        ("Größe und Maße", Some("Größe und Maße")),
        ("日本語の題名", Some("日本語の題名")),
        ("///", None),
        (" . ", None),
        ("", None),
    ];
    for (title, name) in cases {
        assert_eq!(named(title).as_deref(), name, "{title:?}");
    }
}

#[test]
fn a_long_heading_is_cut_at_a_word_and_fits_a_file_name() {
    let cases = ["word ".repeat(100), "x".repeat(500), "語".repeat(200), format!("{} tail", "y".repeat(119))];
    for title in cases {
        let name = named(&title).expect("something is left");
        assert!(name.len() <= 120, "{} bytes", name.len());
        assert!(title.starts_with(&name), "{name:?} is the start of the heading");
        assert_eq!(name, name.trim(), "no space is left at the cut");
    }
    assert_eq!(named(&"word ".repeat(100)).unwrap().len() % 5, 4, "the cut falls between words");
}
