//! The clipboard's HTML: reading what other applications copy, and writing what a copy offers them.

mod common;

use letters_convert::{ImportNote, from_html, from_markdown, to_html_fragment};
use letters_model::{Block, BlockId, BlockKind, Body, Document};
use proptest::prelude::*;

use common::outline;

fn read(html: &str) -> (Document, Vec<ImportNote>) {
    let mut next = 0;
    from_html(html, &mut || {
        next += 1;
        format!("b{next}")
    })
}

/// The first block's text as `outline` writes runs.
fn line(html: &str) -> String {
    let lines = outline(&read(html).0);
    assert_eq!(lines.len(), 1, "{html:?} reads as one block, got {lines:?}");
    lines[0].split_once(' ').map(|(_, text)| text.trim_matches('"').to_owned()).unwrap_or_default()
}

/// `blocks` with their ids set aside, which a copy does not carry.
fn unnamed(blocks: &[Block]) -> Vec<Block> {
    let unnamed = |block: &Block| {
        let body = match &block.body {
            Body::Blocks(children) => Body::Blocks(unnamed(children)),
            other => other.clone(),
        };
        Block { id: BlockId::new(""), body, ..block.clone() }
    };
    blocks.iter().map(unnamed).collect()
}

#[test]
fn reads_every_block() {
    let cases: &[(&str, &[&str])] = &[
        ("<p>One</p><p>Two</p>", &[r#"p "One""#, r#"p "Two""#]),
        ("<h1>One</h1><h4>Four</h4><h6>Six</h6>", &[r#"h1 "One""#, r#"h4 "Four""#, r#"h4 "Six""#]),
        ("<ul><li>a</li><li>b</li></ul>", &[r#"bullet/0 "a""#, r#"bullet/0 "b""#]),
        ("<ol><li>a</li><li>b</li></ol>", &[r#"ordered/0 "a""#, r#"ordered/0 "b""#]),
        (
            "<ul><li>a<ol><li>b<ul><li>c</li></ul></li></ol></li><li>d</li></ul>",
            &[r#"bullet/0 "a""#, r#"ordered/1 "b""#, r#"bullet/2 "c""#, r#"bullet/0 "d""#],
        ),
        (
            r#"<ul><li><input type="checkbox" disabled> a</li><li><input type="checkbox" checked> b</li></ul>"#,
            &[r#"todo/0 "a""#, r#"done/0 "b""#],
        ),
        (
            r#"<pre><code class="language-rust">fn main() {}
</code></pre>"#,
            &[r#"code:rust "fn main() {}""#],
        ),
        ("<pre>two\n  lines</pre>", &[r#"code:- "two\n  lines""#]),
        (
            "<blockquote><p>quoted</p><ul><li>item</li></ul></blockquote>",
            &["quote", r#"  p "quoted""#, r#"  bullet/0 "item""#],
        ),
        (r#"<div class="callout tip" role="note"><p>Try this.</p></div>"#, &["callout:tip", r#"  p "Try this.""#]),
        (
            r#"<div class="markdown-alert markdown-alert-warning"><p class="markdown-alert-title">Warning</p><p>Hot.</p></div>"#,
            &["callout:warn", r#"  p "Hot.""#],
        ),
        ("<p>above</p><hr><p>below</p>", &[r#"p "above""#, "divider", r#"p "below""#]),
        ("<p>first line<br>second line</p>", &[r#"p "first line\nsecond line""#]),
        ("<div>loose<p>inner</p>after</div>", &[r#"p "loose""#, r#"p "inner""#, r#"p "after""#]),
        ("<section><x-card>read <q>through</q></x-card></section>", &[r#"p "read through""#]),
        ("", &[]),
    ];
    for (html, expected) in cases {
        assert_eq!(outline(&read(html).0), *expected, "reading {html:?}");
    }
}

#[test]
fn reads_every_mark() {
    let cases = [
        ("<b>b</b><strong>s</strong>", "<bold|bs>"),
        ("<i>i</i> <em>e</em>", "<italic|i> <italic|e>"),
        ("<u>u</u><ins>n</ins>", "<underline|un>"),
        ("<s>s</s><del>d</del><strike>k</strike>", "<strike|sdk>"),
        ("<mark>lit</mark>", "<highlight|lit>"),
        ("<code>c</code> <kbd>k</kbd>", "<code|c> <code|k>"),
        ("<b><i>both</i></b>", "<bold,italic|both>"),
        (
            r#"<a href="https://example.com">a <b>link</b></a>"#,
            "<@https://example.com|a ><bold,@https://example.com|link>",
        ),
        ("<a>no address</a>", "no address"),
        (r#"<a href="javascript:alert(1)">no script</a>"#, "no script"),
        (
            r#"<span style="font-weight: 700">heavy</span> <span style="font-weight:bold">bold</span>"#,
            "<bold|heavy> <bold|bold>",
        ),
        (r#"<span style="FONT-STYLE: Italic">slanted</span>"#, "<italic|slanted>"),
        (r#"<span style="text-decoration: underline line-through">both</span>"#, "<underline,strike|both>"),
        (r#"<span style="text-decoration-line: line-through">struck</span>"#, "<strike|struck>"),
        // Google Docs wraps a copy in a `b` that is not bold.
        (r#"<b style="font-weight:normal"><span style="font-weight:400">plain</span></b>"#, "plain"),
        (r#"<b>bold <span style="font-weight: normal; font-style: normal">not</span></b>"#, "<bold|bold >not"),
    ];
    for (html, expected) in cases {
        assert_eq!(line(html), expected, "reading {html:?}");
    }
}

#[test]
fn leaves_out_the_styling_a_browser_spells_out() {
    // A browser writes each element's computed style into what it copies.
    let cases = [
        (r#"<h2 style="font-weight: 600">Title</h2>"#, "Title"),
        (r#"<h2><span style="font-weight: 700">Title</span></h2>"#, "Title"),
        (r#"<a href="https://example.com" style="text-decoration: underline">link</a>"#, "<@https://example.com|link>"),
        (r#"<p style="font-weight: 400; font-style: normal; text-decoration: none">plain</p>"#, "plain"),
        (r#"<p style="color: red; background: url(data:image/png;base64,AAAA); font-size: 20px">plain</p>"#, "plain"),
    ];
    for (html, expected) in cases {
        assert_eq!(line(html), expected, "reading {html:?}");
    }
}

#[test]
fn collapses_white_space_as_a_browser_shows_it() {
    let cases = [
        ("<p>\n  one\n  two\t three  </p>", "one two three"),
        ("<p>one <b> two </b> three</p>", "one <bold|two >three"),
        ("<p>one<b> </b> <i> </i>two</p>", "one<bold| >two"),
        ("<p>one <br> two</p>", r"one\ntwo"),
        ("<p><br>one<br><br></p>", "one"),
        ("<p>non&nbsp;breaking &amp; &lt;escaped&gt; &copy;</p>", r"non\u{a0}breaking & <escaped> ©"),
        (r#"<p><span style="white-space: pre-wrap">kept   as  written</span></p>"#, "kept   as  written"),
        (r#"<p style="white-space: pre-wrap">two\nlines</p>"#, r"two\nlines"),
    ];
    for (html, expected) in cases {
        assert_eq!(line(&html.replace(r"\n", "\n")), expected, "reading {html:?}");
    }
}

#[test]
fn keeps_the_spaces_at_the_edges_of_a_phrase() {
    // A phrase copied out of a line goes into a line, where its spaces matter.
    let cases = [
        ("<meta charset='utf-8'><span> a phrase </span>", " a phrase "),
        ("word ", "word "),
        (" <b>bold</b> and  plain", " <bold|bold> and plain"),
        ("<html><body><!--StartFragment--><span>marked </span><!--EndFragment-->\n</body>\n</html>", "marked "),
    ];
    for (html, expected) in cases {
        assert_eq!(line(html), expected, "reading {html:?}");
    }
}

#[test]
fn leaves_out_what_is_not_text() {
    let cases = [
        "<head><title>Page</title><style>p { color: red }</style></head><p>text</p>",
        "<script>if (a < b) { document.write('<p>no</p>') }</script><p>text</p>",
        "<!-- a comment --><p>text<!-- another --></p>",
        "<p>text</p><p hidden>no</p><div style='display: none'>no</div>",
        "<p>text</p><p> </p><p>&nbsp;</p><div><br></div><blockquote></blockquote>",
        "<p>text<img src='https://example.com/cat.png' alt='A cat'><svg><text>no</text></svg></p>",
        "<p>text<button>Copy</button></p>",
        // A picture the clipboard gives no lasting address for, and the speck a page counts its readers with.
        "<p>text</p><p><img src='data:image/png;base64,iVBORw0KGgo='><img src='file:///C:/Temp/clip_image001.png'></p>",
        "<p>text</p><img src='http://example.com/plain.png'><img src='https://example.com/count.gif' width=1 height=1>",
    ];
    for html in cases {
        assert_eq!(outline(&read(html).0), [r#"p "text""#], "reading {html:?}");
    }
}

#[test]
fn reads_what_applications_copy() {
    let chrome = concat!(
        r#"<meta charset='utf-8'><h2 style="box-sizing: border-box; font-weight: 600; color: rgb(31, 35, 40); "#,
        r#"white-space: normal; text-decoration-style: initial;">Install</h2><p style="font-weight: 400; "#,
        r#"white-space: normal;">Run <code style="font-family: monospace;">cargo add</code> or read "#,
        r#"<a href="https://example.com/guide" style="color: rgb(9, 105, 218); text-decoration: underline;">"#,
        r#"the guide</a>.</p>"#,
    );
    let docs = concat!(
        r#"<meta charset='utf-8'><meta charset="utf-8"><b style="font-weight:normal;" id="docs-internal-guid-1">"#,
        r#"<h1 dir="ltr" style="line-height:1.38;"><span style="font-size:20pt;font-weight:400;font-style:normal;"#,
        r#"text-decoration:none;white-space:pre-wrap;">Plan</span></h1><p dir="ltr"><span style="font-weight:400;"#,
        r#"white-space:pre-wrap;">Ship </span><span style="font-weight:700;font-style:italic;white-space:pre-wrap;">"#,
        r#"soon</span></p><ul style="margin-top:0;"><li dir="ltr" aria-level="1"><p dir="ltr" role="presentation">"#,
        r#"<span style="white-space:pre-wrap;">First</span></p></li><ul><li aria-level="2"><p role="presentation">"#,
        r#"<span>Nested</span></p></li></ul><li role="checkbox" aria-checked="true"><p><span>Done</span></p></li>"#,
        r#"</ul><br /></b>"#,
    );
    let word = concat!(
        "Version:1.0\r\nStartHTML:0000000105\r\n<html xmlns:o=\"urn:schemas-microsoft-com:office:office\">\r\n",
        "<head><meta charset=utf-8><style><!-- p.MsoNormal {margin:0in;} --></style></head>\r\n",
        "<body lang=EN-US style='tab-interval:.5in'><!--StartFragment-->\r\n",
        "<h1>Minutes<o:p></o:p></h1>\r\n",
        "<p class=MsoNormal>Agreed <b>two</b>\r\n<i style='mso-bidi-font-style:normal'>things</i>:<o:p></o:p></p>\r\n",
        "<p class=MsoListParagraphCxSpFirst style='text-indent:-.25in;mso-list:l0 level1 lfo1'>",
        "<![if !supportLists]><span style='font-family:Symbol;mso-list:Ignore'>·<span style='font:7.0pt ",
        "\"Times New Roman\"'>&nbsp;&nbsp;&nbsp;&nbsp; </span></span><![endif]>Ship it<o:p></o:p></p>\r\n",
        "<p class=MsoListParagraphCxSpLast style='margin-left:1.0in;mso-list:l1 level2 lfo2'>",
        "<![if !supportLists]><span style='mso-list:Ignore'>a.<span style='font:7.0pt \"Times New Roman\"'>",
        "&nbsp;&nbsp; </span></span><![endif]>On Friday<o:p></o:p></p>\r\n",
        "<p class=MsoNormal><o:p>&nbsp;</o:p></p>\r\n<!--EndFragment--></body>\r\n</html>",
    );
    let editor = concat!(
        r#"<meta charset='utf-8'><div style="color: #d4d4d4;background-color: #1e1e1e;font-family: Menlo, "#,
        r#"monospace;font-weight: normal;white-space: pre;"><div><span style="color: #569cd6;">fn</span>"#,
        r#"<span style="color: #d4d4d4;"> main() {</span></div><div><span>    run();</span></div><br>"#,
        r#"<div><span>}</span></div></div>"#,
    );
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "a browser",
            chrome,
            &[r#"h2 "Install""#, r#"p "Run <code|cargo add> or read <@https://example.com/guide|the guide>.""#],
        ),
        (
            "Google Docs",
            docs,
            &[
                r#"h1 "Plan""#,
                r#"p "Ship <bold,italic|soon>""#,
                r#"bullet/0 "First""#,
                r#"bullet/1 "Nested""#,
                r#"done/0 "Done""#,
            ],
        ),
        (
            "Word",
            word,
            &[
                r#"h1 "Minutes""#,
                r#"p "Agreed <bold|two> <italic|things>:""#,
                r#"bullet/0 "Ship it""#,
                r#"ordered/1 "On Friday""#,
            ],
        ),
        ("a code editor", editor, &[r#"code:- "fn main() {\n    run();\n\n}""#]),
    ];
    for (application, html, expected) in cases {
        assert_eq!(outline(&read(html).0), *expected, "reading a copy from {application}");
    }
}

#[test]
fn reads_maths_where_a_page_carries_its_source() {
    let katex = |display: &str| {
        format!(
            concat!(
                r#"<span class="katex"><span class="katex-mathml"><math xmlns="http://www.w3.org/1998/Math/MathML"{}>"#,
                r#"<semantics><mrow><msup><mi>x</mi><mn>2</mn></msup></mrow>"#,
                r#"<annotation encoding="application/x-tex">x^2</annotation></semantics></math></span>"#,
                r#"<span class="katex-html" aria-hidden="true"><span class="base"><span class="mord mathnormal">x</span>"#,
                r#"<span class="mord mtight">2</span></span></span></span>"#,
            ),
            display
        )
    };
    let wikipedia = concat!(
        r#"<span class="mwe-math-element"><span class="mwe-math-mathml-inline" style="display: none;">"#,
        r#"<math alttext="{\displaystyle x^{2}}"><semantics><mi>x</mi></semantics></math></span>"#,
        r#"<img src="https://example.org/x.svg" class="mwe-math-fallback-image-inline" alt="{\displaystyle x^{2}}"></span>"#,
    );
    let cases: &[(String, &[&str])] = &[
        (format!("<p>so {} holds</p>", katex("")), &[r#"p "so <math|x^2> holds""#]),
        (
            format!("<p>so</p><span class=\"katex-display\">{}</span>", katex(r#" display="block""#)),
            &[r#"p "so""#, r#"formula "x^2""#],
        ),
        (format!("<p>so {wikipedia} holds</p>"), &[r#"p "so <math|{\\displaystyle x^{2}}> holds""#]),
        (r#"<p>so <script type="math/tex">x < 2</script> holds</p>"#.into(), &[r#"p "so <math|x < 2> holds""#]),
        (r#"<script type="math/tex; mode=display">x^2</script>"#.into(), &[r#"formula "x^2""#]),
        (r#"<p>so <span class="math inline">\(x^2\)</span> holds</p>"#.into(), &[r#"p "so <math|x^2> holds""#]),
        (
            r#"<p>so <span class="math display">\[x^2\]</span> holds</p>"#.into(),
            &[r#"p "so""#, r#"formula "x^2""#, r#"p "holds""#],
        ),
        (
            r#"<ul><li>step <math display="block" alttext="x^2"><mi>x</mi></math></li></ul>"#.into(),
            &[r#"bullet/0 "step""#, r#"formula "x^2""#],
        ),
        // Maths with no source is a drawing, and a `math` class alone does not make text maths.
        ("<p>so <math><mi>x</mi></math> holds</p>".into(), &[r#"p "so holds""#]),
        (r#"<p>so <span class="math">x</span> holds</p>"#.into(), &[r#"p "so x holds""#]),
    ];
    for (html, expected) in cases {
        assert_eq!(outline(&read(html).0), *expected, "reading {html:?}");
    }
}

#[test]
fn reads_pictures() {
    let (a, b) = ("https://example.com/a.png", "https://example.com/b.png");
    let cases: &[(String, &[&str])] = &[
        (
            format!("<p><a href='/full'><img src='{a}' alt='A cat'></a></p>"),
            &[r#"figure https://example.com/a.png [A cat] """#],
        ),
        (
            format!(
                "<figure><img src='{a}' alt='A cat'><figcaption>It <b>sat</b> down.</figcaption></figure><p>Next</p>"
            ),
            &[r#"figure https://example.com/a.png [A cat] "It <bold|sat> down.""#, r#"p "Next""#],
        ),
        // Several pictures in one figure are a gallery, as Ghost writes one.
        (
            format!(
                "<figure class='kg-gallery-card'><div class='kg-gallery-row'><div><img src='{a}' alt='a'></div>\
                 <div><img src='{b}'></div></div><figcaption>Both</figcaption></figure>"
            ),
            &[
                "gallery",
                r#"  figure https://example.com/a.png [a] """#,
                r#"  figure https://example.com/b.png [] """#,
                r#"  caption "Both""#,
            ],
        ),
        (
            format!("<p><img src='{a}'> beside it</p>"),
            &[r#"figure https://example.com/a.png [] """#, r#"p "beside it""#],
        ),
        // A figure need not hold a picture.
        (
            "<figure><blockquote>Said</blockquote><figcaption>Someone</figcaption></figure>".to_owned(),
            &["quote", r#"  p "Said""#, r#"p "Someone""#],
        ),
    ];
    for (html, expected) in cases {
        let (doc, notes) = read(html);
        assert_eq!(outline(&doc), *expected, "reading {html:?}");
        assert_eq!(notes, [], "reading {html:?}");
    }
}

#[test]
fn shows_a_picture_at_the_size_its_page_did() {
    // Each case: what the element says of its size in pixels, and the size in points.
    let cases = [
        ("width='400' height='200'", (300.0, 150.0)),
        ("width='1280' height='320'", (480.0, 120.0)),
        ("", (320.0, 240.0)),
    ];
    for (stated, expected) in cases {
        let (doc, _) = read(&format!("<img src='https://example.com/a.png' {stated}>"));
        let [Block { kind: BlockKind::Figure { width, height, .. }, .. }] = doc.blocks.as_slice() else {
            panic!("one figure from {stated:?}: {:?}", doc.blocks);
        };
        assert_eq!((*width, *height), expected, "from {stated:?}");
    }
}

#[test]
fn reads_a_table() {
    let cases: &[(&str, &[&str])] = &[
        (
            concat!(
                "<table><caption>Sizes</caption><thead><tr><th>Name</th><th align=\"right\">Size</th></tr></thead>",
                "<tbody><tr><td><b>a</b></td><td align=\"right\"><p>1</p><p>2</p></td></tr>",
                "<tr><td>b|c</td><td></td></tr></tbody></table>",
            ),
            &[
                r#"p "Sizes""#,
                "table:start,end headed",
                "  row",
                r#"    cell "Name""#,
                r#"    cell "Size""#,
                "  row",
                r#"    cell "<bold|a>""#,
                r#"    cell "1\n2""#,
                "  row",
                r#"    cell "b|c""#,
                r#"    cell """#,
            ],
        ),
        // Word wraps what a cell holds in a paragraph, and says there how it is aligned.
        (
            concat!(
                "<table class=MsoTableGrid border=1 cellspacing=0><tr><td width=100 valign=top>",
                "<p class=MsoNormal>Item<o:p></o:p></p></td><td width=100 valign=top>",
                "<p class=MsoNormal align=center style='text-align:center'><i>Cost</i><o:p></o:p></p></td></tr></table>",
            ),
            &["table:start,center", "  row", r#"    cell "Item""#, r#"    cell "<italic|Cost>""#],
        ),
        // Excel marks the rows it copied, inside their table.
        (
            concat!(
                "<html><body><table border=0><col width=64 span=2><!--StartFragment--><tr height=20>",
                "<td height=20 align=right>1</td><td>a</td></tr><tr><td align=right>2</td><td>b</td></tr>",
                "<!--EndFragment--></table></body></html>",
            ),
            &[
                "table:end,start",
                "  row",
                r#"    cell "1""#,
                r#"    cell "a""#,
                "  row",
                r#"    cell "2""#,
                r#"    cell "b""#,
            ],
        ),
        // A cell that spans rows or columns leaves empty cells where it reaches, so that columns stay in line.
        (
            "<table><tr><td rowspan=2>a</td><td colspan=2>b</td></tr><tr><td>c</td><td>d</td></tr></table>",
            &[
                "table:start,start,start",
                "  row",
                r#"    cell "a""#,
                r#"    cell "b""#,
                r#"    cell """#,
                "  row",
                r#"    cell """#,
                r#"    cell "c""#,
                r#"    cell "d""#,
            ],
        ),
        // A cell holds text alone, so a table inside one is read as its text.
        (
            "<table><tr><td>a</td><td><table><tr><td>b</td><td>c</td></tr></table></td></tr></table>",
            &["table:start,start", "  row", r#"    cell "a""#, r#"    cell "b\nc""#],
        ),
    ];
    for (html, expected) in cases {
        let (doc, notes) = read(html);
        assert_eq!(outline(&doc), *expected, "reading {html:?}");
        assert_eq!(notes, [], "reading {html:?}");
    }
}

#[test]
fn bounds_the_cells_that_spans_add() {
    let row = "<tr><td colspan=1000 rowspan=1000>a</td><td>b</td></tr>";
    let (doc, _) = read(&format!("<table>{}</table>", row.repeat(2_000)));
    let cells = doc.walk().filter(|block| block.kind == BlockKind::TableCell).count();
    assert!((4_000..20_000).contains(&cells), "{cells} cells from 4,000 written");
}

#[test]
fn reads_a_table_that_lays_a_page_out_as_its_cells() {
    let cases = [
        "<table><tr><td><p>One</p><p>Two</p></td></tr><tr><td>Three</td></tr></table>",
        r#"<table role="presentation"><tr><td>One</td><td>Two</td></tr><tr><td>Three</td></tr></table>"#,
    ];
    for html in cases {
        let (doc, notes) = read(html);
        assert_eq!(outline(&doc), [r#"p "One""#, r#"p "Two""#, r#"p "Three""#], "reading {html:?}");
        assert_eq!(notes, [], "reading {html:?}");
    }
}

#[test]
fn notes_what_a_read_simplified() {
    // Each case names the blocks that should carry a note, by their place in the outline.
    let cases: &[(&str, &[&str], &[usize])] = &[
        ("<h5>Deep</h5>", &[r#"h4 "Deep""#], &[0]),
        ("<ul><li><p>a</p><p>b</p></li></ul>", &[r#"bullet/0 "a\nb""#], &[0]),
        (
            "<ul><li>a<pre>code</pre><ul><li>b<blockquote>q</blockquote></li></ul></li></ul>",
            &[r#"bullet/0 "a""#, r#"code:- "code""#, r#"bullet/0 "b""#, "quote", r#"  p "q""#],
            &[1, 3],
        ),
        (
            "<h1>Plain</h1><p>text</p><ul><li>item</li></ul>",
            &[r#"h1 "Plain""#, r#"p "text""#, r#"bullet/0 "item""#],
            &[],
        ),
    ];
    for (html, expected, noted) in cases {
        let (doc, notes) = read(html);
        assert_eq!(outline(&doc), *expected, "reading {html:?}");
        let ids: Vec<&str> = noted.iter().map(|at| doc.blocks[*at].id.as_str()).collect();
        assert_eq!(
            notes.iter().map(|note| note.block.as_str()).collect::<Vec<_>>(),
            ids,
            "notes on {html:?}: {notes:?}"
        );
    }
}

#[test]
fn reads_tags_that_do_not_match() {
    let cases: &[(&str, &[&str])] = &[
        (
            "<ul><li>a<li>b<ul><li>c</ul><li>d</ul>",
            &[r#"bullet/0 "a""#, r#"bullet/0 "b""#, r#"bullet/1 "c""#, r#"bullet/0 "d""#],
        ),
        ("<p>one<p>two", &[r#"p "one""#, r#"p "two""#]),
        ("<p>one</div></span></p>two", &[r#"p "one""#, r#"p "two""#]),
        ("<p><b>bold<i>both</b>plain</p>", &[r#"p "<bold|bold><bold,italic|both>plain""#]),
        ("<li>alone</li>", &[r#"bullet/0 "alone""#]),
        ("<ul><ul><li>deep</li></ul></ul>", &[r#"bullet/0 "deep""#]),
        ("<p>cut off <b>mid", &[r#"p "cut off <bold|mid>""#]),
        ("<p>a < b &amp c</p", &[r#"p "a < b & c""#]),
        ("<br/><hr/><p/>text", &["divider", r#"p "text""#]),
    ];
    for (html, expected) in cases {
        assert_eq!(outline(&read(html).0), *expected, "reading {html:?}");
    }
}

#[test]
fn bounds_how_deep_it_recurses() {
    for tag in ["div", "blockquote", "ul><li", "b", "table><tr><td", "table><tr><td>a<td"] {
        let html = format!("{}deep", format!("<{tag}>").repeat(50_000));
        let (doc, _) = read(&html);
        let text: String = outline(&doc).concat();
        assert!(text.contains("deep"), "the text under 50,000 of <{tag}> is kept");
    }
}

#[test]
fn writes_a_fragment_with_no_page_around_it() {
    let (doc, _) = from_markdown("# Title\n\nSome **bold** text.\n\n- item", &mut || String::new());
    let html = to_html_fragment(&doc.blocks);
    for part in ["<h1>Title</h1>", "<strong>bold</strong>", "<li>item"] {
        assert!(html.contains(part), "{part} in {html}");
    }
    for part in ["<html", "<head", "<style", "<body", "<article"] {
        assert!(!html.contains(part), "no {part} in {html}");
    }
}

#[test]
fn reads_back_the_fragment_a_copy_writes() {
    let markdown = concat!(
        "# Title\n\n#### Small\n\nText with **bold**, *italic*, ~~struck~~, <u>under</u>, <mark>lit</mark>, `code`\n",
        "and [a **link**](https://example.com), with $x < 2$ in it.\n\nTwo\\\nlines.\n\n$$\na &= b \\\\\n  &< c\n$$\n\n",
        "- a\n  1. b\n     - [x] c\n- [ ] d\n\n",
        "| Name | Size |\n| --- | :-: |\n| **a** | 1<br>2 |\n|  | `x` |\n\n|  |  |\n| --- | --: |\n| no | header |\n\n",
        "```rust\nfn main() {\n    let a = 1 < 2;\n}\n```\n\n```\n```\n\n> quoted\n>\n> - item\n\n> [!WARNING]\n> Hot.\n\n---\n\nEnd.\n",
    );
    let (doc, notes) = from_markdown(markdown, &mut || String::new());
    assert_eq!(notes, [], "the fixture holds only what a document can");
    let (back, notes) = read(&to_html_fragment(&doc.blocks));
    assert_eq!(unnamed(&back.blocks), unnamed(&doc.blocks));
    assert_eq!(notes, []);
}

/// Each list of blocks that sit side by side in a document: its own, and those of every quote and callout.
fn siblings(blocks: &[Block]) -> Vec<&[Block]> {
    let inner = blocks.iter().filter(|block| !block.body.blocks().is_empty());
    [blocks].into_iter().chain(inner.flat_map(|block| siblings(block.body.blocks()))).collect()
}

/// HTML as it might arrive: tags that open, close and nest in any order around text.
fn any_html() -> impl Strategy<Value = String> {
    let tag = prop::sample::select(vec![
        "p",
        "div",
        "span",
        "b",
        "i",
        "a href='x'",
        "ul",
        "ol",
        "li",
        "blockquote",
        "pre",
        "h1",
        "h6",
        "table",
        "tr",
        "td",
        "th",
        "br",
        "hr",
        "input type=checkbox",
        "script",
        "style",
        "code",
        "span style='font-weight:700'",
        "p style='mso-list:l0 level3 lfo1'",
        "span style='mso-list:Ignore'",
        "div class='callout tip'",
        "div style='white-space:pre'",
        "figure",
        "figcaption",
        "img src='https://e.com/a.png'",
        "img src='https://e.com/b.png' width=9 height=0",
    ]);
    let piece = prop_oneof![
        tag.clone().prop_map(|tag| format!("<{tag}>")),
        tag.prop_map(|tag| format!("</{}>", tag.split(' ').next().unwrap_or_default())),
        "[a-z <>&\n]{0,6}",
        Just("&nbsp;".to_owned()),
    ];
    prop::collection::vec(piece, 0..40).prop_map(|pieces| pieces.concat())
}

proptest! {
    #[test]
    fn reads_any_html_into_a_sound_document(html in any_html()) {
        // A phrase alone keeps the spaces at its edges, so a block goes before it.
        let (doc, notes) = read(&format!("<p>first</p>{html}"));
        let mut ids = Vec::new();
        for blocks in siblings(&doc.blocks) {
            let mut deepest = 0;
            for block in blocks {
                ids.push(block.id.as_str());
                let text = block.plain_text();
                deepest = match &block.kind {
                    BlockKind::ListItem { depth, .. } => {
                        prop_assert!(*depth <= deepest, "an item sits at most one level below the one before it");
                        depth + 1
                    }
                    _ => 0,
                };
                let children = block.body.blocks();
                prop_assert!(children.iter().all(|child| block.kind.admits(&child.kind)), "a block holds what it may");
                match &block.kind {
                    BlockKind::Paragraph | BlockKind::Heading { .. } => {
                        prop_assert!(!text.trim().is_empty(), "a block of text has text");
                        prop_assert_eq!(text.trim_matches([' ', '\n']), &text, "and no space at its edges");
                    }
                    BlockKind::Quote | BlockKind::Callout { .. } => {
                        prop_assert!(!block.body.blocks().is_empty(), "a quote holds blocks");
                    }
                    BlockKind::Figure { asset, src, width, height, .. } => {
                        prop_assert!(asset.is_some() != src.is_some(), "a figure has one picture");
                        prop_assert!(*width > 0.0 && *height > 0.0, "and a size");
                    }
                    BlockKind::Gallery { .. } => {
                        prop_assert!(children.iter().filter(|part| part.kind != BlockKind::Caption).count() > 1);
                        prop_assert!(children.iter().filter(|part| part.kind == BlockKind::Caption).count() <= 1);
                    }
                    _ => {}
                }
                prop_assert!(block.body.runs().iter().all(|run| !run.text.is_empty()), "no run is empty");
            }
        }
        for note in &notes {
            prop_assert!(ids.contains(&note.block.as_str()), "{:?} is on a block of the document", note);
        }
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        prop_assert_eq!(ids.len(), count, "each block has an id of its own");
    }

    #[test]
    fn reads_any_text_without_failing(html in "\\PC{0,200}") {
        read(&html);
    }
}
