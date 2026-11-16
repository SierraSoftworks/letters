//! The page a server sends: a template with the document in it twice, under the shape of the window's bar.

use std::collections::BTreeSet;
use std::convert::Infallible;

use accesskit::Node;
use letters_api::{Allows, Opened, Role, Trust};
use letters_editor::InputEvent;
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, ReplicaId, Style};
use letters_render::Size;
use letters_spell::{Dictionary, Speller, best_dictionary};
use letters_test::TestPlatform;
use letters_ui::Window;
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
        named: true,
        space: None,
        path: String::new(),
        role: Role::Editor,
        author: author.into(),
        socket: "/api/v1/documents/0190f3a2-7c1e-7b4a-9c55-3f2a1b0c9d8e/socket".into(),
        trust,
        allows: Allows { download: true, copy: true, print: true },
        by_link: false,
        last_seen: None,
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
        assert!(shown.contains("/assets/abc123/fonts/"), "{name}: the first paint has its fonts");
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
    let welcome = page::welcome(&mut || "b".into());
    let first = first_paint_of(&welcome);
    let shown = between(&first, "srcdoc=\"", "\"").replace("&quot;", "\"").replace("&amp;", "&");
    assert!(shown.contains("--column"), "the column is the window's");
    // The first font is the file the editor's first frame waits for, whole or its stand-in.
    for file in std::iter::once(page::first_font(&welcome)).chain(letters_fonts::FILES.into_iter().skip(1)) {
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
    // Every width, and only those that tell the lines apart: where one bends, and each side of a break.
    let told = [300.0, 344.0, 499.0, 500.0, 700.0].map(shown).to_vec();
    for (samples, most) in [(all.clone(), 4), (told, 3)] {
        let spans = page::spans(&samples);
        let widths = samples.len();
        assert!(spans.len() <= most, "from {widths} widths, a line is one span however many it holds: {}", spans.len());
        for (width, places) in &all {
            let at = placed(&spans, *width);
            assert_eq!(at.len(), places.len(), "from {widths} widths, at {width}");
            for (at, place) in at.iter().zip(places) {
                assert!(
                    (0..4).all(|i| (at[i] - place[i]).abs() < 0.1),
                    "from {widths} widths, at {width}: {at:?} for {place:?}"
                );
            }
        }
        // Between the widths that were laid out, and past the widest, each keeps to its line.
        assert_eq!(placed(&spans, 2000.0).last().map(|place| place[0]), Some(1960.0));
        assert_eq!(placed(&spans, 600.5).last().map(|place| place[0]), Some(560.5));
    }
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

/// Where the window has the button called `name` when it is `width` wide.
fn button(window: &mut Window, width: f32, name: &str) -> Place {
    window.resize(Size { width, height: 2.0 * width }, 1.0);
    let update = window.access();
    let named = |node: &&Node| node.role() == accesskit::Role::Button && node.label() == Some(name);
    let node = update.nodes.iter().map(|(_, node)| node).find(named).unwrap_or_else(|| panic!("at {width}: no {name}"));
    let bounds = node.bounds().expect("a button is somewhere");
    [bounds.x0 as f32, bounds.y0 as f32, bounds.width() as f32, bounds.height() as f32]
}

#[test]
fn the_skeleton_has_the_name_where_the_window_has_it_at_widths_it_was_not_laid_out_at() {
    let cases =
        [("Minutes", Trust::Trusted), ("The minutes of a meeting that went on for a long time indeed", Trust::Safe)];
    for (name, trust) in cases {
        let said = opened(name, "Ada", trust);
        let mut window = page::served_window(page::welcome(&mut || "b".into()), &said, ReplicaId::new("test"));
        let skeleton = Skeleton::of(&mut window);
        for width in (320..=1600).step_by(7).map(|width| width as f32) {
            let [left, top, wide, high] = button(&mut window, width, name);
            let shape = placed(&skeleton.spans, width)[0];
            let near = |a: f32, b: f32| (a - b).abs() < 0.1;
            assert!(
                near(shape[0], left) && near(shape[1], top) && near(shape[3], high),
                "{name} at {width}: {shape:?}"
            );
            // The bar cuts a name short where its tools begin.
            assert!(shape[2] <= wide + 0.1, "{name} at {width}: {shape:?} is no wider than the {wide} of the name");
        }
    }
}

#[test]
fn a_served_page_has_the_skeleton_of_the_window_on_all_of_a_long_document() {
    let text = |n: usize, kind| Block::text(BlockId::new(format!("b{n}")), kind, format!("What block {n} says."));
    // A name of its own or its first heading's, which is deep in the document, and the caret in each kind of last block.
    let cases = [
        (true, BlockKind::Paragraph),
        (false, BlockKind::Heading { level: 2 }),
        (false, BlockKind::Code { language: None }),
    ];
    for (named, last) in cases {
        let mut blocks: Vec<Block> = (0..200).map(|n| text(n, BlockKind::Paragraph)).collect();
        blocks[40] = text(40, BlockKind::Heading { level: 1 });
        blocks.push(text(200, last.clone()));
        let document = Document { blocks, ..Document::default() };
        let said = Opened { named, ..opened("Minutes", "Ada", Trust::Trusted) };

        let mut window = page::served_window(document.clone(), &said, ReplicaId::new("test"));
        let style = window.style().clone();
        let whole =
            page::first_paint(&Skeleton::of(&mut window), &page::export(&document, &style, &said.name, "/fonts/"));
        assert!(page::served(&document, &said, "/").contains(&whole), "named {named}, ending in {last:?}");
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
            preloads.matches("href=\"/static/maths/KaTeX_").count(),
            if maths { letters_math::FONT_FILES.len() } else { 0 },
            "{name}"
        );
        // Besides them it waits for one font, and for nothing else.
        assert_eq!(preloads.matches("href=\"/static/fonts/").count(), 1, "{name}");
        assert_eq!(preloads.matches("<link rel=\"preload\"").count(), preloads.matches("href=").count(), "{name}");
        assert_eq!(preloads.matches("href=").count(), 1 + preloads.matches("/maths/").count(), "{name}");
        let page = page::compose("<head><!--letters:preload--></head>", &preloads, "", "{}");
        assert_eq!(page, format!("<head>{preloads}</head>"), "{name}");
    }
}

#[test]
fn the_first_frame_waits_for_the_stand_in_only_where_it_writes_all_the_document_says() {
    let [stand_in, whole] = [letters_fonts::FIRST, letters_fonts::FILES[0]];
    let says = |text: &str| Block::text(BlockId::new("p"), BlockKind::Paragraph, text);
    let quoted = Block { body: Body::Blocks(vec![says("Ελληνικά")]), ..Block::new("q", BlockKind::Quote) };
    let cases = [
        ("nothing", vec![], stand_in),
        ("Latin", vec![says("Minutes of the café, 3 – 4 pm: “agreed” → done")], stand_in),
        ("Greek", vec![says("Minutes"), says("Ελληνικά")], whole),
        ("Greek in a quote", vec![says("Minutes"), quoted], whole),
        ("Chinese", vec![says("会议记录")], whole),
    ];
    for (name, blocks, font) in cases {
        let other = if font == stand_in { whole } else { stand_in };
        let document = Document { blocks, ..Document::default() };
        assert_eq!(page::first_font(&document), font, "{name}");

        // The page asks for that file early, and its first paint is set in it: one font is fetched, once.
        let page = page::served(&document, &opened("Minutes", "Ada", Trust::Trusted), "/assets/abc123/");
        let shown = between(&page, "srcdoc=\"", "\"");
        let around = page.replace(shown, "");
        assert!(around.contains(&format!("href=\"/assets/abc123/fonts/{font}\"")), "{name}");
        assert!(shown.contains(&format!("/assets/abc123/fonts/{font}")), "{name}");
        assert!(!page.contains(&format!("fonts/{other}")), "{name}: {other} is not asked for");
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

#[test]
fn the_languages_a_document_names_are_those_a_dictionary_is_fetched_for_and_never_english() {
    let passage = |lang: &str| json!({ "text": "x", "lang": lang });
    let block =
        json!({ "id": "b", "type": "paragraph", "lang": "fr", "content": [passage("pt_BR"), passage("en-GB")] });
    let cases = [("de-CH", vec!["de-CH", "fr", "pt-BR"]), ("en", vec!["fr", "pt-BR"])];
    for (language, named) in cases {
        let document = json!({ "format": 1, "language": language, "blocks": [block] });
        let document = Document::from_json(&document.to_string()).expect("the document reads");
        assert_eq!(page::languages(&document).into_iter().collect::<Vec<_>>(), named, "{language}");
    }
}

#[test]
fn text_is_checked_against_the_dictionary_its_server_answers_for_its_language() {
    // The dictionaries a server holds, and the languages written in: the document's first.
    let cases: [(&[&str], &[&str]); 4] = [
        (&["de-AT", "de-DE"], &["de-CH", "de-AT", "de"]),
        (&["de-AT", "de-CH"], &["de-DE", "de-CH"]),
        (&["de", "de-DE"], &["de-CH", "de-DE"]),
        (&["de-DE", "fr"], &["fr-CA", "de", "nl"]),
    ];
    // Each dictionary knows one word, which says whose it is.
    let word = |held: &str| held.replace('-', "").to_lowercase();
    for (held, written) in cases {
        let held: Vec<String> = held.iter().map(|tag| (*tag).to_owned()).collect();
        let passages: Vec<_> = written[1..].iter().map(|lang| json!({ "text": "x", "lang": lang })).collect();
        let block = json!({ "id": "b", "type": "paragraph", "content": passages });
        let document = json!({ "format": 1, "language": written[0], "blocks": [block] });
        let document = Document::from_json(&document.to_string()).expect("the document reads");
        let wanted = page::dictionaries(&document, &held);
        // They arrive in any order.
        for backwards in [false, true] {
            let mut arriving: Vec<_> = wanted.iter().collect();
            if backwards {
                arriving.reverse();
            }
            let mut speller = Speller::new();
            for (held, under) in arriving {
                let dictionary = Dictionary::parse("", &format!("1\n{}\n", word(held))).expect("it reads");
                speller.add_dictionary(under, dictionary);
            }
            for tag in written {
                let served = best_dictionary(held.iter().map(String::as_str), tag).map(|at| word(&held[at]));
                let words = BTreeSet::new();
                let accepted =
                    served.as_ref().map(|word| speller.checker(tag, &words).is_some_and(|by| by.check(word)));
                assert_ne!(accepted, Some(false), "{held:?}, {tag}, backwards: {backwards}");
                assert_eq!(speller.supports(tag), served.is_some(), "{held:?}, {tag}");
            }
        }
    }
}
