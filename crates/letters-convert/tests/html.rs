//! HTML export, through its one public function.

use letters_convert::{from_markdown, to_html};
use letters_model::{Block, BlockKind, Document, ListKind, Mark, Run, Style};

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
        } else if !["br", "hr", "input", "meta", "!doctype"].contains(&tag.as_str()) {
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
        ("> [!TIP]\n> Try it", "<div class=\"callout tip\" role=\"note\"> <p>Try it</p> </div>"),
        ("> [!CAUTION]\n> Hot", "<div class=\"callout danger\" role=\"note\"> <p>Hot</p> </div>"),
        ("a\n\n---\n\nb", "<p>a</p> <hr> <p>b</p>"),
        ("```\nx < 1 && y\n```", "<pre><code>x &lt; 1 &amp;&amp; y</code></pre>"),
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
        "line-height: 1.1;",
        "color: #aa3300;",
        "text-align: center;",
    ] {
        assert!(heading.contains(declaration), "h1 lacks {declaration:?}: {heading}");
    }
    assert!(css.contains("h1 { margin: 24pt 0 6.5pt; }"), "{css}");
    assert!(rule(css, "pre").contains("font-family: \"Fira Code\""), "{css}");
    // The text column is the page less its side margins, and paper takes the page setup.
    assert!(rule(css, "article").contains("max-width: 450pt;"), "{css}");
    assert!(rule(css, "@page").contains("size: 612pt 792pt;"), "{css}");

    // Every type of the built-in style has a rule, so the look can be reused on the web.
    let css = export(&Document::default());
    for selector in ["p, li", "h1", "h2", "h3", "h4", "blockquote p, blockquote li", "pre", "figcaption, .fallback"] {
        assert!(rule(stylesheet(&css), selector).contains("font-size:"), "{selector}");
    }
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
    assert!(!stylesheet(&html).contains("url(") && !stylesheet(&html).contains("@import"));
    assert!(html.contains("Content-Security-Policy"));
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
    let mut block = Block::new("f", BlockKind::Unknown { type_name: "figure".into() });
    for (name, value) in fields {
        block.extra.insert((*name).into(), (*value).into());
    }
    block.extra.insert("width".into(), 150.0.into());
    block.extra.insert("height".into(), 75.0.into());
    block.extra.insert("content".into(), Run::list_to_value(caption));
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
