//! The page a server sends: a template with the document in it twice, under the shape of the window's bar.

use letters_model::{Block, BlockId, BlockKind, Document, ReplicaId, Style};
use letters_ui::{Scheme, palette};
use letters_web::page::{self, DOCUMENT, FIRST_PAINT, Place, Skeleton, Span};

const TEMPLATE: &str =
    "<body><canvas></canvas><!--letters:first-paint--><!--letters:document--><script>start()</script></body>";

/// What lies between `open` and the next `close` in `page`.
fn between<'a>(page: &'a str, open: &str, close: &str) -> &'a str {
    let start = page.find(open).unwrap_or_else(|| panic!("the page has no {open}")) + open.len();
    &page[start..start + page[start..].find(close).expect("what was opened is closed")]
}

/// The whole first paint a page would carry for `document`, as the harness page is built.
fn first_paint_of(document: &Document) -> String {
    let mut window = page::window(document.clone(), ReplicaId::new("test"));
    let style = window.style().clone();
    let html = letters_convert::to_html_themed(document, &style, "Title", &mut |_| None, &palette(Scheme::Dark));
    let skeleton = Skeleton::of(&mut window);
    page::first_paint(&skeleton, &style, "fonts/", &html)
}

/// Where the spans put each control in a window of `width`.
fn placed(spans: &[Span], width: f32) -> Vec<Place> {
    let span = spans.iter().rev().find(|span| span.from <= width).expect("a span holds the width");
    let at = |(at, moves): &(Place, Place)| std::array::from_fn(|i| at[i] + moves[i] * (width - span.from));
    span.controls.iter().map(at).collect()
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
        let first = page::first_paint(&Skeleton::default(), &Style::default(), "fonts/", &html);
        let page = page::compose(TEMPLATE, &first, &document.to_json());

        // A browser reads the attribute back to the HTML it was given, and the script's text to the JSON.
        let shown = between(&page, "srcdoc=\"", "\"");
        assert_eq!(shown.replace("&quot;", "\"").replace("&amp;", "&"), html, "{text}");
        let carried = between(&page, &format!("id=\"{DOCUMENT}\">"), "</script>");
        assert_eq!(Document::from_json(carried).expect("the page's document reads"), document, "{text}");

        // Nothing the document holds becomes part of the page around it, and nothing in the frame may run.
        let around = page.replace(shown, "").replace(carried, "");
        let empty = page::first_paint(&Skeleton::default(), &Style::default(), "fonts/", "");
        let script = format!("<script type=\"application/json\" id=\"{DOCUMENT}\"></script>");
        let expected =
            TEMPLATE.replace("<!--letters:first-paint-->", &empty).replace("<!--letters:document-->", &script);
        assert_eq!(around, expected, "{text}");
        assert!(around.contains(&format!("<div id=\"{FIRST_PAINT}\">")), "{text}");
        assert!(around.contains("sandbox=\"allow-same-origin\""), "{text}");
    }
}

#[test]
fn a_template_without_places_for_a_document_is_left_as_it_is() {
    assert_eq!(page::compose("<body></body>", "<p>text</p>", "{}"), "<body></body>");
}

#[test]
fn the_export_is_set_as_the_window_sets_the_document_in_the_windows_fonts() {
    let first = first_paint_of(&page::welcome(&mut || "b".into()));
    let shown = between(&first, "srcdoc=\"", "\"").replace("&quot;", "\"").replace("&amp;", "&");
    // The export's own rules come first, so the page's are the ones that hold.
    let rules = shown.rfind("<style>").expect("the page's rules are in the frame");
    assert!(shown[rules..].contains("article {"), "the column is set");
    for file in letters_fonts::FILES {
        assert!(shown[rules..].contains(&format!("url(\"fonts/{file}\")")), "{file} can be fetched");
    }
    assert!(shown.contains("font-src 'self'"), "the frame may fetch the page's fonts");
    assert!(!shown.contains("script-src"), "and still runs nothing");
}

#[test]
fn spans_put_every_control_where_the_window_showed_it() {
    // One control stays, one follows the far edge, one gives way as the window narrows, and one is only
    // there from 500 up.
    let shown = |width: f32| {
        let mut places = vec![[10.0, 8.0, 28.0, 28.0], [48.0, 8.0, (width - 200.0).min(144.0), 28.0]];
        if width >= 500.0 {
            places.push([300.0, 8.0, 28.0, 28.0]);
        }
        places.push([width - 40.0, 8.0, 28.0, 28.0]);
        (width, places)
    };
    let all: Vec<(f32, Vec<Place>)> = (300..=700).map(|width| shown(width as f32)).collect();
    let spans = page::spans(&all);
    assert!(spans.len() <= 4, "a line is one span, however many widths it holds: {}", spans.len());
    for (width, places) in &all {
        let at = placed(&spans, *width);
        assert_eq!(at.len(), places.len(), "at {width}");
        for (at, place) in at.iter().zip(places) {
            assert!((0..4).all(|i| (at[i] - place[i]).abs() < 0.1), "at {width}: {at:?} for {place:?}");
        }
    }
    // Between the widths that were laid out, and past the widest, each keeps to its line.
    assert_eq!(placed(&spans, 2000.0).last().map(|place| place[0]), Some(1960.0));
    assert_eq!(placed(&spans, 600.5).last().map(|place| place[0]), Some(560.5));
}

#[test]
fn the_skeleton_is_the_start_of_the_windows_bar_and_none_of_its_tools() {
    let mut window = page::window(page::welcome(&mut || "b".into()), ReplicaId::new("test"));
    let skeleton = Skeleton::of(&mut window);
    assert!(skeleton.bar > 0.0, "the window has a bar");
    assert!(skeleton.spans.len() < 64, "the bar has a few forms, not one for every width");
    for width in [320.0, 390.0, 768.0, 1024.0, 1400.0, 2560.0] {
        let places = placed(&skeleton.spans, width);
        assert!(matches!(places.len(), 1 | 2), "at {width}: the name, and how saving stands: {places:?}");
        let name = places[0];
        assert!(name[0] < 64.0 && name[2] >= 24.0, "at {width}: the name starts the bar: {name:?}");
        for [left, top, wide, high] in &places {
            assert!(*left >= 0.0 && left + wide <= width, "at {width}: inside the window");
            assert!(*top >= 0.0 && top + high <= skeleton.bar, "at {width}: inside the bar");
        }
        for pair in places.windows(2) {
            assert!(pair[0][0] + pair[0][2] <= pair[1][0] + 0.1, "at {width}: {pair:?} overlap");
        }
    }
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
