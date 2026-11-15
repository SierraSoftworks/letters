//! The page a server sends: a template with the document in it twice.

use letters_model::{Block, BlockId, BlockKind, Document};
use letters_web::page::{self, DOCUMENT, FIRST_PAINT};

const TEMPLATE: &str =
    "<body><canvas></canvas><!--letters:first-paint--><!--letters:document--><script>start()</script></body>";

/// What lies between `open` and the next `close` in `page`.
fn between<'a>(page: &'a str, open: &str, close: &str) -> &'a str {
    let start = page.find(open).unwrap_or_else(|| panic!("the page has no {open}")) + open.len();
    &page[start..start + page[start..].find(close).expect("what was opened is closed")]
}

#[test]
fn a_page_carries_its_document_to_read_and_to_open_whatever_it_holds() {
    let texts = [
        "A plain paragraph.",
        "It says \"quoted\" & <b>bold</b>.",
        "</script><script>alert(1)</script>",
        "\"></iframe><script>alert(1)</script>",
        "<!--letters:document-->",
    ];
    for text in texts {
        let block = Block::text(BlockId::new("b1"), BlockKind::Paragraph, text);
        let document = Document { blocks: vec![block], ..Document::default() };
        let html = format!("<!doctype html><title>\"T\" & more</title><article><p>{text}</p></article>");
        let page = page::compose(TEMPLATE, &html, &document.to_json());

        // A browser reads the attribute back to the HTML it was given, and the script's text to the JSON.
        let shown = between(&page, "srcdoc=\"", "\"");
        assert_eq!(shown.replace("&quot;", "\"").replace("&amp;", "&"), html, "{text}");
        let carried = between(&page, &format!("id=\"{DOCUMENT}\">"), "</script>");
        assert_eq!(Document::from_json(carried).expect("the page's document reads"), document, "{text}");

        // Nothing the document holds becomes part of the page around it.
        let frame = format!("<iframe id=\"{FIRST_PAINT}\" title=\"Document\" sandbox=\"\" srcdoc=\"\"></iframe>");
        let script = format!("<script type=\"application/json\" id=\"{DOCUMENT}\"></script>");
        let around = page.replace(shown, "").replace(carried, "");
        let expected =
            TEMPLATE.replace("<!--letters:first-paint-->", &frame).replace("<!--letters:document-->", &script);
        assert_eq!(around, expected, "{text}");
    }
}

#[test]
fn a_template_without_places_for_a_document_is_left_as_it_is() {
    assert_eq!(page::compose("<body></body>", "<p>text</p>", "{}"), "<body></body>");
}

#[test]
fn the_welcome_document_takes_its_ids_from_the_page() {
    let mut next = 0;
    let document = page::welcome(&mut || {
        next += 1;
        format!("id{next}")
    });
    let ids: Vec<&str> = document.blocks.iter().map(|block| block.id.as_str()).collect();
    assert_eq!(ids, ["id1", "id2", "id3"]);
    assert_eq!(Document::from_json(&document.to_json()).expect("it reads back"), document);
}
