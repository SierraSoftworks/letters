//! HTML export, through its one public function.

use letters_convert::{
    Column, HtmlPage, WebFont, from_html, from_markdown, to_html, to_html_fragment, to_html_themed, to_html_with,
    to_markdown,
};
use letters_layout::{Palette, Scheme};
use letters_model::{Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Run, Style, Tone};
use letters_render::Color;

fn document(markdown: &str) -> Document {
    let mut next = 0;
    let (doc, _) = from_markdown(markdown, &mut || {
        next += 1;
        format!("b{next}")
    });
    doc
}

fn export(doc: &Document) -> String {
    to_html(doc, &Style::default(), "Report", &mut |_| None)
}

/// What sits between the tags of the page's article, with one space for each run of white space.
fn article(html: &str) -> String {
    let (_, rest) = html.split_once("<article>").expect("the page has an article");
    let (article, _) = rest.split_once("</article>").expect("the article ends");
    article.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The page's stylesheet.
fn stylesheet(html: &str) -> &str {
    let (_, rest) = html.split_once("<style>").expect("the page has a stylesheet");
    rest.split_once("</style>").expect("the stylesheet ends").0
}

/// The body of the first rule for `selector`.
fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
    let start = css.find(&format!("{selector} {{")).unwrap_or_else(|| panic!("no rule for {selector}"));
    let body = &css[start + selector.len() + 2..];
    &body[..body.find('}').expect("the rule ends")]
}

/// The names of the tags in `html`, in order, with a slash before each closing one.
fn tags(html: &str) -> Vec<String> {
    let name = |tag: &str| tag.split([' ', '>']).next().unwrap_or_default().to_ascii_lowercase();
    html.split('<').skip(1).map(name).collect()
}

/// Whether every element that opens is closed, innermost first.
fn nests(html: &str) -> bool {
    let mut open = Vec::new();
    for tag in tags(html) {
        if let Some(closed) = tag.strip_prefix('/') {
            if open.pop().as_deref() != Some(closed) {
                return false;
            }
        } else if !["br", "hr", "img", "input", "meta", "!doctype", "path", "rect"].contains(&tag.as_str()) {
            open.push(tag);
        }
    }
    open.is_empty()
}

fn link(text: &str, url: &str) -> Document {
    let run = Run { link: Some(url.into()), ..Run::plain(text) };
    Document {
        blocks: vec![Block { body: letters_model::Body::Text(vec![run]), ..Block::new("a", BlockKind::Paragraph) }],
        ..Document::default()
    }
}

#[test]
fn blocks_and_marks_become_their_elements() {
    let cases = [
        ("# Title\n\n#### Small", "<h1>Title</h1> <h4>Small</h4>"),
        ("Plain **bold** *italic* ~~gone~~", "<p>Plain <strong>bold</strong> <em>italic</em> <s>gone</s></p>"),
        ("<u>under</u> <mark>lit</mark> `x < y`", "<p><u>under</u> <mark>lit</mark> <code>x &lt; y</code></p>"),
        ("***both***", "<p><strong><em>both</em></strong></p>"),
        ("one\\\ntwo", "<p>one<br> two</p>"),
        (
            "[a **b**](https://example.com/?q=1&r=2)",
            "<p><a href=\"https://example.com/?q=1&amp;r=2\">a <strong>b</strong></a></p>",
        ),
        ("- a\n- b", "<ul> <li>a</li> <li>b</li> </ul>"),
        ("1. a\n2. b", "<ol> <li>a</li> <li>b</li> </ol>"),
        ("- a\n  1. b\n  2. c\n- d", "<ul> <li>a <ol> <li>b</li> <li>c</li> </ol> </li> <li>d</li> </ul>"),
        ("- a\n\n1. b", "<ul> <li>a</li> </ul> <ol> <li>b</li> </ol>"),
        (
            "- [x] done\n- [ ] open",
            "<ul> <li class=\"task\"><input type=\"checkbox\" disabled checked>done</li> \
             <li class=\"task\"><input type=\"checkbox\" disabled>open</li> </ul>",
        ),
        ("> quoted\n>\n> - item", "<blockquote> <p>quoted</p> <ul> <li>item</li> </ul> </blockquote>"),
        ("a\n\n---\n\nb", "<p>a</p> <hr> <p>b</p>"),
        ("```\nx < 1 && y\n```", "<pre><code>x &lt; 1 &amp;&amp; y</code></pre>"),
        (
            "| a | b |\n|---|--:|\n| 1 | **2** |",
            "<table> <thead> <tr><th>a</th><th style=\"text-align: end\">b</th></tr> </thead> <tbody> \
             <tr><td>1</td><td style=\"text-align: end\"><strong>2</strong></td></tr> </tbody> </table>",
        ),
        ("|   |   |\n|---|---|\n| 1 | 2 |", "<table> <tbody> <tr><td>1</td><td>2</td></tr> </tbody> </table>"),
    ];
    for (markdown, expected) in cases {
        let html = export(&document(markdown));
        assert_eq!(article(&html), expected, "{markdown:?}");
        assert!(nests(&html), "{markdown:?} gave elements that do not nest:\n{html}");
    }
}

#[test]
fn list_items_nest_whatever_their_depths() {
    let item = |depth| Block::text("", BlockKind::ListItem { list: ListKind::Bullet, depth }, "item");
    for depths in [vec![3], vec![0, 2, 1], vec![0, 1, 2, 0], vec![1, 0, 3, 3, 1]] {
        let doc = Document { blocks: depths.iter().map(|depth| item(*depth)).collect(), ..Document::default() };
        let html = export(&doc);
        assert!(nests(&html), "{depths:?} gave elements that do not nest:\n{html}");
        assert_eq!(html.matches("<li>").count(), depths.len(), "{depths:?}");
    }
}

#[test]
fn code_is_highlighted_and_keeps_its_text() {
    let source = "// sum\nfn main() {\n    let total = 40 + 2; // <ok>\n}";
    let html = export(&document(&format!("```rust\n{source}\n```")));
    let block = article(&html);
    assert!(block.starts_with("<pre><code class=\"language-rust\">"), "{block}");
    for (class, text) in [("tok-keyword", "fn"), ("tok-comment", "// sum"), ("tok-number", "40")] {
        assert!(block.contains(&format!("<span class=\"{class}\">{text}</span>")), "{class} around {text}: {block}");
        assert!(stylesheet(&html).contains(&format!(".{class} {{ color: #")), "no colour for {class}");
    }

    // With the tags taken out, the code reads as it was written.
    let (_, code) = html.split_once("<pre><code class=\"language-rust\">").expect("the page has the block");
    let code = code.split_once("</code></pre>").expect("the block ends").0;
    let text: String = code.split('<').map(|part| part.split_once('>').map_or(part, |(_, text)| text)).collect();
    assert_eq!(text.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&"), source);
}

#[test]
fn an_unknown_language_is_written_plain() {
    let html = export(&document("```klingon ignore\nnuqneH <qaStaH>\n```"));
    assert_eq!(article(&html), "<pre><code class=\"language-klingon\">nuqneH &lt;qaStaH&gt;</code></pre>");
}

#[test]
fn the_style_is_written_as_css() {
    let mut style = Style::default();
    {
        let heading = style.types.get_mut("heading1").expect("the style has headings");
        (heading.family, heading.size, heading.weight) = ("Montserrat".into(), 30.0, 800);
        (heading.color, heading.italic, heading.line_height) = ("#aa3300".into(), true, 1.1);
        (heading.space_before, heading.space_after) = (24.0, 6.5);
        heading.align = letters_model::Align::Center;
    }
    style.types.get_mut("code").expect("the style has code").family = "Fira Code".into();
    (style.page.width, style.page.height) = (612.0, 792.0);
    (style.page.margin_left, style.page.margin_right) = (72.0, 90.0);

    let html = to_html(&document("# Title"), &style, "Report", &mut |_| None);
    let css = stylesheet(&html);
    let heading = rule(css, "h1");
    for declaration in [
        "font-family: \"Montserrat\"",
        "font-size: 30pt;",
        "font-weight: 800;",
        "font-style: italic;",
        // In the whole pixels the editor sets lines by: 30 points are 40 pixels.
        "line-height: 44px;",
        "text-align: center;",
    ] {
        assert!(heading.contains(declaration), "h1 lacks {declaration:?}: {heading}");
    }
    assert!(css.contains("h1 { color: #aa3300; }"), "{css}");
    assert!(css.contains("h1 { margin: 32px 0 9px; }"), "{css}");
    assert!(rule(css, "pre").contains("font-family: \"Fira Code\""), "{css}");
    // The text column is the page less its side margins, and paper takes the page setup.
    assert!(rule(css, "article").contains("max-width: 450pt;"), "{css}");
    assert!(rule(css, "@page").contains("size: 612pt 792pt;"), "{css}");

    // Every type of the built-in style has a rule, so the look can be reused on the web.
    let css = export(&Document::default());
    for selector in
        ["p, li", "h1", "h2", "h3", "h4", "blockquote p, blockquote li", "pre", "figcaption, .fallback, .unknown"]
    {
        assert!(rule(stylesheet(&css), selector).contains("font-size:"), "{selector}");
    }
}

#[test]
fn a_themed_page_goes_dark_where_its_reader_does() {
    let hex = |color: Color| format!("#{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    let dark = Palette {
        scheme: Scheme::Dark,
        page: Color::rgb(0x15, 0x17, 0x1a),
        text: Color::rgb(0xe6, 0xe8, 0xeb),
        link: Color::rgb(0x6c, 0xb0, 0xff),
        ..Palette::LIGHT
    };
    let mut style = Style::default();
    style.types.get_mut("heading1").expect("the style has headings").color = "#aa3300".into();
    let doc = document(include_str!("corpus/report.md"));

    let light = to_html(&doc, &style, "Report", &mut |_| None);
    let themed = to_html_themed(&doc, &style, "Report", &mut |_| None, &dark);
    assert!(light.contains("<meta name=\"color-scheme\" content=\"light\">"), "the plain page is light alone");
    assert!(!light.contains("prefers-color-scheme"), "the plain page is light alone");
    assert!(themed.contains("<meta name=\"color-scheme\" content=\"light dark\">"));
    assert_eq!(article(&themed), article(&light), "a theme changes no content");

    let (day, rest) = stylesheet(&themed).split_once("@media screen and (prefers-color-scheme: dark) {\n").unwrap();
    let night = rest.split_once("\n}\n").expect("the dark rules end").0;
    assert!(day.contains("h1 { color: #aa3300; }"), "the light page keeps the style's colours: {day}");

    // Nothing stays as it was on white: each rule that colours the light page has one for the dark.
    let coloured: Vec<&str> = day.lines().filter(|rule| rule.contains(": #")).collect();
    assert!(coloured.len() > 20, "{coloured:?}");
    for light in coloured {
        let selector = light.split_once(" {").expect("a rule has a selector").0;
        assert!(night.lines().any(|dark| dark.split_once(" {").is_some_and(|rule| rule.0 == selector)), "{selector}");
    }
    assert_eq!(rule(night, "html"), " background: #15171a; ");
    assert_eq!(rule(night, "a"), " color: #6cb0ff; ");
    // The style's colours turn over as they do on the editor's dark page: ink to the theme's text, hues kept.
    assert_eq!(rule(night, "p, li"), " color: #e6e8eb; ");
    let heading = dark.recolor(Color::rgb(0xaa, 0x33, 0x00));
    assert_eq!(rule(night, "h1"), format!(" color: {}; ", hex(heading)));
    assert_ne!(hex(heading), "#aa3300");
}

#[test]
fn maths_is_drawn_and_reads_back() {
    let doc = document("Mass is $E = mc^2$, where $a < b$.\n\n```math\n\\frac{a}{b}\n```\n\n- So $x^2$ holds.");
    let html = export(&doc);
    let body = article(&html);
    for (element, tex) in [
        ("span class=\"math inline\"", "E = mc^2"),
        ("span class=\"math inline\"", "a &lt; b"),
        ("p class=\"math display\"", "\\frac{a}{b}"),
        ("span class=\"math inline\"", "x^2"),
    ] {
        assert!(body.contains(&format!("<{element} data-tex=\"{tex}\"><svg ")), "{tex} is not drawn: {body}");
    }
    // Each is a drawing in outlines, so the page needs no font for it, and the source is not shown beside it.
    assert!(nests(&html), "the drawings gave elements that do not nest");
    assert_eq!(body.matches("</svg>").count(), 4);
    assert!(tags(&body).iter().any(|name| name == "path"), "a drawing has no outlines");
    let shown: String = body.split('<').map(|part| part.split_once('>').map_or(part, |(_, text)| text)).collect();
    assert!(shown.contains("Mass is , where .") && !shown.contains("mc^2"), "{shown}");

    // The page gives the document back, maths and all.
    let mut next = 0;
    let (back, _) = from_html(&html, &mut || {
        next += 1;
        format!("r{next}")
    });
    assert_eq!(to_markdown(&back), to_markdown(&doc));
}

#[test]
fn maths_that_cannot_be_set_is_written_as_its_source() {
    let doc = document("Mass is $\\frac{a}$ here.\n\n```math\n\\notacommand{x}\n```");
    let body = article(&export(&doc));
    assert_eq!(
        body,
        "<p>Mass is <span class=\"math inline\">\\(\\frac{a}\\)</span> here.</p> \
         <p class=\"math display\">\\[\\notacommand{x}\\]</p>"
    );
}

#[test]
fn the_file_stands_alone() {
    let markdown = include_str!("corpus/report.md");
    let html = export(&document(markdown));
    assert!(html.starts_with("<!doctype html>\n<html lang=\"en\">"), "{}", &html[..60]);
    assert!(html.contains("<meta charset=\"utf-8\">") && html.contains("<title>Report</title>"));
    assert!(nests(&html), "the corpus report gave elements that do not nest");
    // No script, and nothing fetched: the only addresses are the ones a reader may follow.
    let names = tags(&html);
    for banned in ["script", "link", "img", "iframe", "object", "embed", "base", "form"] {
        assert!(!names.iter().any(|name| name == banned), "the page holds a <{banned}>");
    }
    let css = stylesheet(&html);
    assert!(!css.contains("@import") && !css.contains("@font-face"), "{css}");
    assert_eq!(css.matches("url(").count(), css.matches("url(\"data:").count(), "the stylesheet fetches something");
    assert!(html.contains("Content-Security-Policy") && !html.contains("font-src"));
}

#[test]
fn a_page_loads_the_fonts_it_is_given_and_takes_the_callers_rules() {
    let fonts = [
        WebFont { family: "Inter", url: "/fonts/Inter.ttf", italic: false },
        WebFont { family: "Inter", url: "/fonts/Inter-Italic.ttf?v=\"2\"</style>", italic: true },
    ];
    let page =
        HtmlPage { fonts: &fonts, css: "html { cursor: progress; }\n/* </style><script> */", ..HtmlPage::default() };
    let html = to_html_with(&document("# Title"), &Style::default(), "Report", &mut |_| None, &page);
    let css = stylesheet(&html);

    // The page may fetch fonts from where it came from, and still nothing else.
    assert!(html.contains("default-src 'none'; font-src 'self';"), "{}", &html[..400]);
    assert!(
        css.contains(
            "font-family: \"Inter\"; src: url(\"/fonts/Inter.ttf\"); font-weight: 1 1000; font-style: normal;"
        )
    );
    assert_eq!(css.matches("@font-face").count(), 2);
    assert!(css.contains("font-style: italic; font-display: swap;"), "{css}");

    // The caller's rules come last, where they win, and neither they nor an address can end the stylesheet.
    assert!(css.trim_end().ends_with("html { cursor: progress; }\n/* <\\/style><script> */"), "{css}");
    assert_eq!(html.matches("</style>").count(), 1);
}

#[test]
fn a_page_for_a_window_is_set_as_the_window_sets_it() {
    let mut poll = Block::new("poll", BlockKind::Unknown { type_name: "poll<b>".into() });
    poll.extra.insert("fallback".into(), "Which day suits?".into());
    let doc = Document { blocks: vec![poll], ..Document::default() };
    let style = Style::default();
    let window = HtmlPage { window: Some(Column { gutter: 24.0, top: 48.0 }), ..HtmlPage::default() };

    let own = to_html(&doc, &style, "Report", &mut |_| None);
    let shown = to_html_with(&doc, &style, "Report", &mut |_| None, &window);

    // A page of its own is as wide as the style's paper and gives its margins up on a narrow screen. A
    // window's column is the text's measure, or the window less its gutters, and keeps its room above.
    assert!(rule(stylesheet(&own), "article").contains("max-width: 453pt;"));
    assert!(stylesheet(&own).contains("@media (max-width: 40em)"));
    let column = rule(stylesheet(&shown), "article");
    assert!(column.contains("min(604px, 100% - 48px)") && column.contains("padding: 48px 0;"), "{column}");
    assert!(!stylesheet(&shown).contains("@media (max-width: 40em)"));

    // A reader elsewhere gets what the block says. The window's stand-in shows what the window will.
    assert_eq!(article(&own), "<p class=\"fallback\">Which day suits?</p>");
    assert_eq!(article(&shown), "<p class=\"unknown\">Unsupported block: poll&lt;b&gt;</p>");
}

#[test]
fn text_cannot_break_out_of_the_page() {
    let hostile = "</title></style><script>alert(1)</script>\"'&";
    let mut doc = document("para");
    doc.language = hostile.into();
    doc.blocks.push(Block::text("h", BlockKind::Heading { level: 2 }, hostile));
    doc.blocks.push(Block::text("c", BlockKind::Code { language: Some(hostile.into()) }, hostile));
    doc.blocks.push(Block::text("m", BlockKind::Paragraph, ""));
    doc.blocks[3].body =
        letters_model::Body::Text(vec![Run::marked(hostile, [Mark::Code, Mark::Other(hostile.into())])]);
    doc.blocks.push(link(hostile, &format!("https://example.com/{hostile}")).blocks.remove(0));
    let mut unknown = Block::new("u", BlockKind::Unknown { type_name: hostile.into() });
    unknown.extra.insert("fallback".into(), hostile.into());
    doc.blocks.push(unknown);
    let mut style = Style::default();
    for of in style.types.values_mut() {
        (of.family, of.color) = (hostile.into(), hostile.into());
    }

    let html = to_html(&doc, &style, hostile, &mut |_| None);
    assert!(!html.contains("<script"), "{html}");
    assert_eq!(html.matches("</style>").count(), 1, "{html}");
    assert_eq!(html.matches("</title>").count(), 1, "{html}");
    assert!(nests(&html), "{html}");
    assert!(
        html.contains("<h2>&lt;/title&gt;&lt;/style&gt;&lt;script&gt;alert(1)&lt;/script&gt;&quot;'&amp;</h2>"),
        "{html}"
    );
    assert!(html.contains("<p class=\"fallback\">&lt;/title&gt;"), "{html}");
}

/// A figure as a file stores one, with `fields` beside its caption.
fn figure(fields: &[(&str, &str)], caption: &[Run]) -> Document {
    let field = |name: &str| fields.iter().find(|(field, _)| *field == name).map(|(_, value)| (*value).to_owned());
    let (asset, src, alt) = (field("asset"), field("src"), field("alt").unwrap_or_default());
    let kind = BlockKind::Figure { asset, src, alt, width: 150.0, height: 75.0, numbered: false };
    let mut block = Block::new("f", kind);
    block.body = Body::Text(caption.to_vec());
    Document { blocks: vec![block], ..Document::default() }
}

#[test]
fn images_are_written_into_the_page() {
    // An image's first bytes, and the base64 a browser must read them back from.
    let cases: [(&str, &[u8], &str); 4] = [
        ("assets/a.png", &[0x89, b'P', b'N', b'G'], "data:image/png;base64,iVBORw=="),
        ("assets/b.png", &[0x89, b'P', b'N', b'G', 1], "data:image/png;base64,iVBORwE="),
        ("assets/c", b"GIF89a\xff\xfe\xfd", "data:image/gif;base64,R0lGODlh//79"),
        ("assets/d.SVG", b"<svg/>", "data:image/svg+xml;base64,PHN2Zy8+"),
    ];
    for (name, bytes, address) in cases {
        let doc = figure(
            &[("asset", name), ("alt", "A \"cat\"")],
            &[Run::plain("Fig. 1: "), Run::marked("a cat", [Mark::Bold])],
        );
        let html = to_html(&doc, &Style::default(), "Report", &mut |asked| (asked == name).then(|| bytes.to_vec()));
        let expected = format!(
            "<figure> <img src=\"{address}\" alt=\"A &quot;cat&quot;\" width=\"200\" height=\"100\"> \
             <figcaption>Fig. 1: <strong>a cat</strong></figcaption> </figure>"
        );
        assert_eq!(article(&html), expected, "{name}");
    }
}

#[test]
fn an_image_the_page_cannot_hold_keeps_its_place() {
    let remote = [("src", "https://example.com/cat.png"), ("alt", "A cat")];
    let cases: [(&[(&str, &str)], &str); 5] = [
        // A remote image is linked, so that opening the page fetches nothing.
        (&remote, "<figure> <p><a href=\"https://example.com/cat.png\">A cat</a></p> </figure>"),
        (
            &remote[..1],
            "<figure> <p><a href=\"https://example.com/cat.png\">https://example.com/cat.png</a></p> </figure>",
        ),
        (&[("src", "javascript:alert(1)"), ("alt", "A cat")], "<figure> <p class=\"fallback\">A cat</p> </figure>"),
        // The file does not hold it, or holds something that is not an image.
        (&[("asset", "assets/missing.png"), ("alt", "A cat")], "<figure> <p class=\"fallback\">A cat</p> </figure>"),
        (&[("asset", "assets/notes.txt"), ("alt", "")], "<figure> </figure>"),
    ];
    for (fields, expected) in cases {
        let html = to_html(&figure(fields, &[]), &Style::default(), "Report", &mut |name| {
            (name == "assets/notes.txt").then(|| b"plain text".to_vec())
        });
        assert_eq!(article(&html), expected, "{fields:?}");
        assert!(!tags(&html).iter().any(|tag| tag == "img"), "{fields:?}");
    }
}

/// A picture the file holds, 60 points tall, with its id as its description.
fn picture(id: &str, width: f32, numbered: bool, caption: &str) -> Block {
    let asset = Some(format!("assets/{id}.png"));
    let kind = BlockKind::Figure { asset, src: None, alt: id.into(), width, height: 60.0, numbered };
    Block::text(id, kind, caption)
}

/// A paragraph that refers to each of `figures`, each reference carrying text that has gone stale.
fn referring(figures: &[&str]) -> Block {
    let refer = |id: &&str| Run::marked("Figure 9", [Mark::Ref(BlockId::new(id))]);
    let mut block = Block::new("p", BlockKind::Paragraph);
    block.body = Body::Text(figures.iter().flat_map(|id| [Run::plain("see "), refer(id)]).collect());
    block
}

/// A page of `doc`, whose file holds every picture it asks for.
fn pictured(doc: &Document, style: &Style) -> String {
    article(&to_html(doc, style, "Report", &mut |_| Some(vec![0x89, b'P', b'N', b'G'])))
}

#[test]
fn figures_are_numbered_and_references_lead_to_them() {
    let mut worded = Style::default();
    worded.types.get_mut("caption").expect("a style has captions").extra.insert("label".into(), "Abbildung".into());
    for (style, word) in [(Style::default(), "Figure"), (worded, "Abbildung")] {
        let blocks = vec![
            referring(&["cat", "bare", "gone"]),
            picture("plain", 80.0, false, "Not counted"),
            picture("cat", 80.0, true, "A cat"),
            picture("bare", 80.0, true, ""),
        ];
        let html = pictured(&Document { blocks, ..Document::default() }, &style);

        // A numbered figure's caption opens with its label, and the figure can be led to.
        assert!(html.contains(&format!("<figcaption>{word} 1: A cat</figcaption>")), "{html}");
        assert!(html.contains(&format!("<figcaption>{word} 2:</figcaption>")), "{html}");
        assert!(html.contains("<figure id=\"cat\">") && html.contains("<figure id=\"bare\">"), "{html}");
        assert!(html.contains("<figcaption>Not counted</figcaption>"), "{html}");
        // A reference reads as its figure's label now, not as the text it was written with.
        assert!(html.contains(&format!("href=\"#cat\">{word} 1</a>")), "{html}");
        assert!(html.contains(&format!("href=\"#bare\">{word} 2</a>")), "{html}");
        assert!(!html.contains("Figure 9"), "{html}");
        // One to a figure that is gone says so, and leads nowhere.
        assert!(html.contains(&format!("{word} ?")) && !html.contains("#gone"), "{html}");
    }
}

#[test]
fn a_reference_leads_to_a_figure_whatever_its_id_holds() {
    let id = "a b\"<c>/\u{e9}";
    let doc = Document { blocks: vec![referring(&[id]), picture(id, 80.0, true, "")], ..Document::default() };
    let html = pictured(&doc, &Style::default());
    assert!(html.contains("id=\"a b&quot;&lt;c&gt;/\u{e9}\""), "{html}");
    assert!(html.contains("href=\"#a%20b%22%3Cc%3E%2F%C3%A9\""), "{html}");
}

#[test]
fn a_gallery_is_one_figure_of_rows() {
    // How many pictures, and how many of them each row holds: up to three, and the last is never alone.
    let cases: [(usize, &[usize]); 5] = [(1, &[1]), (3, &[3]), (4, &[2, 2]), (5, &[3, 2]), (7, &[3, 2, 2])];
    for (count, rows) in cases {
        let mut gallery = Block::new("g", BlockKind::Gallery { numbered: true });
        let mut held: Vec<Block> = (0..count).map(|index| picture(&format!("p{index}"), 90.0, false, "")).collect();
        held.push(Block::text("c", BlockKind::Caption, "Cats"));
        gallery.body = Body::Blocks(held);
        let doc = Document { blocks: vec![referring(&["g"]), gallery], ..Document::default() };
        let html = pictured(&doc, &Style::default());

        let (_, figure) = html.split_once("<figure").expect("the gallery is a figure");
        assert_eq!(figure.matches("<figure").count(), 0, "{count}: one figure holds every picture");
        let found: Vec<usize> = figure.split("<div").skip(1).map(|row| row.matches("<img").count()).collect();
        assert_eq!(found, rows, "{count} pictures");
        assert!(figure.contains("<figcaption>Figure 1: Cats</figcaption>"), "{count}: {figure}");
        assert!(figure.contains("id=\"g\"") && html.contains("href=\"#g\">Figure 1</a>"), "{count}: {html}");
        assert!(nests(&html), "{count}: {html}");
    }
}

#[test]
fn a_reference_read_back_from_a_page_is_its_label() {
    let doc = Document { blocks: vec![referring(&["cat"]), picture("cat", 80.0, true, "")], ..Document::default() };
    let html = to_html(&doc, &Style::default(), "Report", &mut |_| None);
    let (read, _) = from_html(&html, &mut || "new".into());
    let runs = read.blocks[0].body.runs();
    assert_eq!(read.blocks[0].plain_text(), "see Figure 1");
    assert!(runs.iter().all(|run| run.link.is_none()), "a link into the page it came from leads nowhere: {runs:?}");
}

#[test]
fn a_copied_reference_is_its_text() {
    let html = to_html_fragment(&[referring(&["cat"])]);
    assert!(html.contains("Figure 9") && !html.contains("<a"), "{html}");
}

#[test]
fn links_that_would_run_script_are_written_as_text() {
    let unsafe_urls = [
        "javascript:alert(1)",
        " JavaScript:alert(1)",
        "java\tscript:alert(1)",
        "\u{1}vbscript:x",
        "data:text/html,<script>",
    ];
    for url in unsafe_urls {
        let html = export(&link("press", url));
        assert_eq!(article(&html), "<p>press</p>", "{url:?}");
    }
    let safe_urls = [
        ("https://example.com/a", "https://example.com/a"),
        ("mailto:ben@example.com", "mailto:ben@example.com"),
        ("notes/other.let#part", "notes/other.let#part"),
        ("#part", "#part"),
        ("/wiki/javascript:void", "/wiki/javascript:void"),
        ("obsidian://open?vault=a", "obsidian://open?vault=a"),
        (" https://example.com/\n", "https://example.com/"),
    ];
    for (url, written) in safe_urls {
        let html = export(&link("press", url));
        assert_eq!(article(&html), format!("<p><a href=\"{written}\">press</a></p>"), "{url:?}");
    }
}

#[test]
fn what_has_no_text_keeps_its_place() {
    let mut doc = document("above\n\nbelow");
    doc.blocks.insert(1, Block::text("e", BlockKind::Paragraph, ""));
    doc.blocks.push(Block::new("q", BlockKind::Quote));
    doc.blocks.push(Block::new("c", BlockKind::Code { language: None }));
    // An unknown block with no fallback has nothing to show.
    doc.blocks.push(Block::new("u", BlockKind::Unknown { type_name: "chart".into() }));
    let html = export(&doc);
    assert_eq!(
        article(&html),
        "<p>above</p> <p><br></p> <p>below</p> <blockquote> <p><br></p> </blockquote> <pre><code> </code></pre>"
    );
}

#[test]
fn a_callout_is_headed_by_its_title_and_keeps_what_was_chosen_for_it() {
    let cases = [
        ("> [!TIP]\n> Try it", "callout tip", "Tip</p> <p>Try it</p>"),
        ("> [!CAUTION]\n> Hot", "callout danger", "Danger</p> <p>Hot</p>"),
        ("> [!WARNING] Mind the gap\n> Hot", "callout warn", "Mind the gap</p> <p>Hot</p>"),
    ];
    for (markdown, class, text) in cases {
        let html = export(&document(markdown));
        let article = article(&html);
        assert!(article.contains(class) && article.contains(text), "{markdown:?} gave {article}");
        assert!(nests(&html), "{markdown:?} gave elements that do not nest:\n{html}");
    }

    // A title, an icon and a colour of the writer's own are read back from the page, and no title is added
    // to the text for being shown.
    let chosen = [
        BlockKind::Callout { tone: Tone::Info, title: Some("A <b> & c".into()), icon: None, color: None },
        BlockKind::Callout { tone: Tone::Warn, title: Some(String::new()), icon: Some("none".into()), color: None },
        BlockKind::Callout { tone: Tone::Tip, title: None, icon: Some("flame".into()), color: Some("#aa3366".into()) },
    ];
    for kind in chosen {
        let mut doc = document("> [!NOTE]\n> Inside");
        doc.blocks[0].kind = kind.clone();
        let html = export(&doc);
        assert!(nests(&html), "{kind:?} gave elements that do not nest:\n{html}");
        let (read, _) = from_html(&html, &mut || "r".to_owned());
        let callout = read.blocks.iter().find(|block| matches!(block.kind, BlockKind::Callout { .. }));
        let callout = callout.unwrap_or_else(|| panic!("{kind:?} reads back as a callout"));
        assert_eq!(callout.kind, kind);
        assert_eq!(callout.body.blocks()[0].plain_text(), "Inside", "{kind:?}");
    }
}
