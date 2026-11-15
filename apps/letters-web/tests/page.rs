//! The page a server sends: a template with the document in it twice, under the shape of the window's bar.

use std::convert::Infallible;

use letters_api::{Allows, Opened, Role, Trust};
use letters_editor::InputEvent;
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, ReplicaId, Style};
use letters_test::TestPlatform;
use letters_web::page::{self, DOCUMENT, FIRST_PAINT, OPENED, Place, Skeleton, Span};
use serde_json::json;

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
    let html = page::export(document, &style, "Title", "fonts/");
    page::first_paint(&Skeleton::of(&mut window), &html)
}

/// What a server says of a document called `name` that `author` may edit.
fn opened(name: &str, author: &str, trust: Trust) -> Opened {
    Opened {
        id: "0190f3a2-7c1e-7b4a-9c55-3f2a1b0c9d8e".into(),
        created: "2026-11-15T10:00:00Z".into(),
        name: name.into(),
        space: None,
        path: String::new(),
        role: Role::Editor,
        author: author.into(),
        socket: "/api/v1/documents/0190f3a2-7c1e-7b4a-9c55-3f2a1b0c9d8e/socket".into(),
        trust,
        allows: Allows { download: true, copy: true, print: true },
        style: None,
        notes: None,
    }
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
        let first = page::first_paint(&Skeleton::default(), &html);
        let page = page::compose(TEMPLATE, "", &first, &document.to_json());

        // A browser reads the attribute back to the HTML it was given, and the script's text to the JSON.
        let shown = between(&page, "srcdoc=\"", "\"");
        assert_eq!(shown.replace("&quot;", "\"").replace("&amp;", "&"), html, "{text}");
        let carried = between(&page, &format!("id=\"{DOCUMENT}\">"), "</script>");
        assert_eq!(Document::from_json(carried).expect("the page's document reads"), document, "{text}");

        // Nothing the document holds becomes part of the page around it, and nothing in the frame may run.
        let around = page.replace(shown, "").replace(carried, "");
        let empty = page::first_paint(&Skeleton::default(), "");
        let script = format!("<script type=\"application/json\" id=\"{DOCUMENT}\"></script>");
        let expected =
            TEMPLATE.replace("<!--letters:first-paint-->", &empty).replace("<!--letters:document-->", &script);
        assert_eq!(around, expected, "{text}");
        assert!(around.contains(&format!("<div id=\"{FIRST_PAINT}\">")), "{text}");
        assert!(around.contains("sandbox=\"allow-same-origin\""), "{text}");
    }
}

#[test]
fn a_served_page_carries_what_the_server_says_whatever_it_holds() {
    let names = ["Minutes", "A \"quoted\" & <b>bold</b> name", "</title></script><script>alert(1)</script>"];
    for name in names {
        let block = Block::text(BlockId::new("b1"), BlockKind::Paragraph, name);
        let document = Document { blocks: vec![block], ..Document::default() };
        let said = opened(name, name, Trust::Trusted);
        let page = page::served(&document, &said, "/assets/abc123/");

        let carried = between(&page, &format!("id=\"{OPENED}\">"), "</script>");
        assert_eq!(serde_json::from_str::<Opened>(carried).expect("what the server says reads"), said, "{name}");
        let carried = between(&page, &format!("id=\"{DOCUMENT}\">"), "</script>");
        assert_eq!(Document::from_json(carried).expect("the page's document reads"), document, "{name}");
        let title = between(&page, "<title>", "</title>");
        assert_eq!(title.replace("&lt;", "<").replace("&quot;", "\"").replace("&amp;", "&"), name, "{name}");

        // The page's files are the server's, and the one script that runs is among them: a policy that
        // lets no inline script run still starts the editor.
        assert!(page.contains("<base href=\"/assets/abc123/\" />"), "{name}");
        let shown = between(&page, "srcdoc=\"", "\"");
        assert!(shown.contains("/assets/abc123/fonts/Inter.ttf"), "{name}: the first paint has its fonts");
        let around = page.replace(shown, "");
        let kind = |(at, _): (usize, &str)| match &around[at..] {
            tag if tag.starts_with("<script type=\"application/json\"") => "data",
            tag if tag.starts_with("<script type=\"module\" src=\"") => "file",
            _ => "inline",
        };
        let scripts: Vec<&str> = around.match_indices("<script").map(kind).collect();
        assert_eq!(scripts, ["data", "data", "file"], "{name}");
    }
}

#[test]
fn the_window_on_a_served_document_is_in_its_style_and_signs_as_the_session_does() {
    let mut said = opened("Minutes", "Ada Lovelace", Trust::Trusted);
    said.style = Some(json!({ "format": 1, "page": { "margin_top": 99 } }));
    let mut window = page::served_window(page::welcome(&mut || "b".into()), &said, ReplicaId::new("test"));
    assert_eq!(window.style().page.margin_top, 99.0);

    let mut platform = TestPlatform::default();
    let written =
        window.change(|editor| Ok::<_, Infallible>(editor.input(InputEvent::Insert("x".into()), &mut platform)));
    assert!(written.is_ok());
    let authors = window.editor().model().authors();
    assert_eq!(authors.iter().map(String::as_str).collect::<Vec<_>>(), ["Ada Lovelace"]);
}

#[test]
fn a_served_document_opens_whatever_this_build_cannot_read_of_its_style_and_notes() {
    let cases = [(json!("nonsense"), json!([1])), (json!({ "format": 99 }), json!({ "format": 99 }))];
    for (style, notes) in cases {
        let mut said = opened("Minutes", "Ada Lovelace", Trust::Trusted);
        (said.style, said.notes) = (Some(style.clone()), Some(notes));
        let block = Block::text(BlockId::new("b1"), BlockKind::Paragraph, "Kept as it is.");
        let document = Document { blocks: vec![block], ..Document::default() };
        let window = page::served_window(document.clone(), &said, ReplicaId::new("test"));
        assert_eq!(window.editor().doc(), &document, "{style}");
        assert_eq!(window.style(), &Style::default(), "{style}: the built-in style stands in");
        assert!(page::served(&document, &said, "/").contains("srcdoc"), "{style}: and its page is still made");
    }
}

#[test]
fn a_served_page_in_safe_mode_leaves_room_for_the_bar_that_says_so() {
    let document = page::welcome(&mut || "b".into());
    let top = |trust| {
        let page = page::served(&document, &opened("Minutes", "Ada", trust), "/");
        let rule = between(&page, &format!("#{FIRST_PAINT} iframe {{"), "}").to_owned();
        between(&rule, "top: ", "px").parse::<f32>().expect("the frame's top is a measure")
    };
    assert!(top(Trust::Safe) > top(Trust::Trusted), "the document starts under the bar");
}

#[test]
fn a_template_without_places_for_a_document_is_left_as_it_is() {
    assert_eq!(page::compose("<body></body>", "<link>", "<p>text</p>", "{}"), "<body></body>");
}

#[test]
fn the_export_is_set_as_the_window_sets_the_document_in_the_windows_fonts() {
    let first = first_paint_of(&page::welcome(&mut || "b".into()));
    let shown = between(&first, "srcdoc=\"", "\"").replace("&quot;", "\"").replace("&amp;", "&");
    assert!(shown.contains("--column"), "the column is the window's");
    for file in letters_fonts::FILES {
        assert!(shown.contains(&format!("fonts/{file}")), "{file} can be fetched");
    }
    assert!(shown.contains("prefers-color-scheme: dark"), "a dark page is read in the window's dark colours");
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
fn only_a_document_with_maths_has_its_first_frame_wait_for_the_maths_faces() {
    let formula = Block::text(BlockId::new("f"), BlockKind::Formula, "x^2");
    let mut inline = Block::text(BlockId::new("p"), BlockKind::Paragraph, "x^2");
    if let Body::Text(runs) = &mut inline.body {
        runs[0].marks.insert(Mark::Math);
    }
    let quoted = Block { body: Body::Blocks(vec![inline.clone()]), ..Block::new("q", BlockKind::Quote) };
    let plain = Block::text(BlockId::new("p"), BlockKind::Paragraph, "x^2 is not maths until it is marked");
    let cases = [
        ("a formula", formula, true),
        ("maths in a line", inline, true),
        ("maths in a quote", quoted, true),
        ("none", plain, false),
    ];
    for (name, block, maths) in cases {
        let document = Document { blocks: vec![block], ..Document::default() };
        assert_eq!(page::has_maths(&document), maths, "{name}");
        let preloads = page::preloads(&document, "/static/");
        assert_eq!(
            preloads.matches("<link rel=\"preload\"").count(),
            if maths { letters_math::FONT_FILES.len() } else { 0 },
            "{name}"
        );
        assert_eq!(
            preloads.matches("href=\"/static/maths/KaTeX_").count(),
            preloads.matches("<link").count(),
            "{name}"
        );
        let page = page::compose("<head><!--letters:preload--></head>", &preloads, "", "{}");
        assert_eq!(page, format!("<head>{preloads}</head>"), "{name}");
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
