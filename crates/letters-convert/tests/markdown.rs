//! Markdown import and export, through the two public functions.

mod common;

use letters_convert::{ImportNote, from_markdown, to_markdown};
use letters_model::{
    Align, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Properties, PropertiesFormat, Run, Tone,
};
use proptest::prelude::*;

use common::outline;

fn read(markdown: &str) -> (Document, Vec<ImportNote>) {
    let mut next = 0;
    from_markdown(markdown, &mut || {
        next += 1;
        format!("b{next}")
    })
}

fn doc(blocks: Vec<Block>) -> Document {
    Document { blocks, ..Document::default() }
}

fn with_text(kind: BlockKind, runs: Vec<Run>) -> Block {
    Block { body: Body::Text(runs), ..Block::new("", kind) }
}

fn holding(kind: BlockKind, blocks: Vec<Block>) -> Block {
    Block { body: Body::Blocks(blocks), ..Block::new("", kind) }
}

fn item(list: ListKind, depth: u8, text: &str) -> Block {
    with_text(BlockKind::ListItem { list, depth }, vec![Run::plain(text)])
}

/// A table of rows of cells, each cell holding its text.
fn table(columns: &[Align], header: bool, rows: &[&[&str]]) -> Block {
    let cell = |text: &&str| with_text(BlockKind::TableCell, vec![Run::plain(*text)]);
    let row = |cells: &&[&str]| holding(BlockKind::TableRow, cells.iter().map(cell).collect());
    holding(BlockKind::Table { columns: columns.to_vec(), header }, rows.iter().map(row).collect())
}

fn linked(text: &str, url: &str) -> Run {
    Run { link: Some(url.into()), ..Run::plain(text) }
}

/// A document with what a round trip may change set aside: block ids, emphasis on spaces, code on line breaks,
/// and the cells a table's short rows lack.
fn settled(doc: &Document) -> Document {
    fn settle_block(block: &Block) -> Block {
        let mut body = match &block.body {
            Body::Text(runs) if !matches!(block.kind, BlockKind::Code { .. } | BlockKind::Formula) => {
                Body::Text(settle(runs))
            }
            Body::Blocks(blocks) => Body::Blocks(blocks.iter().map(settle_block).collect()),
            other => other.clone(),
        };
        let mut kind = block.kind.clone();
        if let (BlockKind::Table { columns, .. }, Body::Blocks(rows)) = (&mut kind, &mut body) {
            let width = rows.iter().map(|row| row.body.blocks().len()).chain([columns.len(), 1]).max().unwrap_or(1);
            columns.resize(width, Align::Start);
            for row in rows {
                if let Body::Blocks(cells) = &mut row.body {
                    cells.resize(width, with_text(BlockKind::TableCell, Vec::new()));
                }
            }
        }
        Block { id: BlockId::new(""), kind, body, ..block.clone() }
    }
    fn settle(runs: &[Run]) -> Vec<Run> {
        let mut out: Vec<Run> = Vec::new();
        for run in runs {
            for c in run.text.chars() {
                let mut marks = run.marks.clone();
                if matches!(c, '\n' | '\r') {
                    marks.remove(&Mark::Code);
                }
                if c.is_whitespace() && !marks.contains(&Mark::Code) {
                    marks.retain(|mark| !matches!(mark, Mark::Bold | Mark::Italic | Mark::Strike));
                }
                match out.last_mut() {
                    Some(last) if last.marks == marks && last.link == run.link => last.text.push(c),
                    _ => out.push(Run { text: c.into(), marks, link: run.link.clone(), ..Run::default() }),
                }
            }
        }
        out
    }
    Document { blocks: doc.blocks.iter().map(settle_block).collect(), ..doc.clone() }
}

#[test]
fn reads_every_block() {
    let cases: &[(&str, &[&str])] = &[
        ("Plain text\nacross lines.", &[r#"p "Plain text across lines.""#]),
        ("# One\n\n#### Four", &[r#"h1 "One""#, r#"h4 "Four""#]),
        ("Title\n=====", &[r#"h1 "Title""#]),
        ("- a\n- b", &[r#"bullet/0 "a""#, r#"bullet/0 "b""#]),
        ("1. a\n2. b", &[r#"ordered/0 "a""#, r#"ordered/0 "b""#]),
        ("- [ ] a\n- [x] b", &[r#"todo/0 "a""#, r#"done/0 "b""#]),
        ("- a\n  1. b\n     - c\n- d", &[r#"bullet/0 "a""#, r#"ordered/1 "b""#, r#"bullet/2 "c""#, r#"bullet/0 "d""#]),
        ("```rust\nfn main() {}\n```", &[r#"code:rust "fn main() {}""#]),
        ("    indented\n    code", &[r#"code:- "indented\ncode""#]),
        ("> quoted\n>\n> - item", &["quote", r#"  p "quoted""#, r#"  bullet/0 "item""#]),
        ("> [!TIP]\n> Try this.", &["callout:tip", r#"  p "Try this.""#]),
        ("above\n\n---\n\nbelow", &[r#"p "above""#, "divider", r#"p "below""#]),
        ("first line\\\nsecond line", &[r#"p "first line\nsecond line""#]),
        ("- a\n\n  > q\n\n  - b", &[r#"bullet/0 "a""#, "quote", r#"  p "q""#, r#"bullet/0 "b""#]),
        ("", &[]),
    ];
    for (markdown, expected) in cases {
        assert_eq!(outline(&read(markdown).0), *expected, "reading {markdown:?}");
    }
}

#[test]
fn reads_every_mark() {
    let cases = [
        ("**bold**", "<bold|bold>"),
        ("*italic* and _italic_", "<italic|italic> and <italic|italic>"),
        ("~~struck~~", "<strike|struck>"),
        ("`code`", "<code|code>"),
        ("<u>under</u> and <ins>under</ins>", "<underline|under> and <underline|under>"),
        ("<mark>lit</mark>", "<highlight|lit>"),
        ("<b>b</b><i>i</i><s>s</s>", "<bold|b><italic|i><strike|s>"),
        ("***both***", "<bold,italic|both>"),
        ("**bold `code`**", "<bold|bold ><bold,code|code>"),
        ("[text](https://example.com)", "<@https://example.com|text>"),
        ("<https://example.com>", "<@https://example.com|https://example.com>"),
        ("[*a*](/b) c", "<italic,@/b|a> c"),
        ("[ref]\n\n[ref]: /target", "<@/target|ref>"),
        ("[[Page]] and [[Page|label]]", "<@Page|Page> and <@Page|label>"),
        ("a &amp; b \\* c", "a & b * c"),
    ];
    for (markdown, expected) in cases {
        assert_eq!(outline(&read(markdown).0), [format!("p {expected:?}")], "reading {markdown:?}");
    }
}

#[test]
fn reads_maths() {
    let cases: &[(&str, &[&str])] = &[
        (r"Euler: $e^{i\pi} + 1 = 0$.", &[r#"p "Euler: <math|e^{i\\pi} + 1 = 0>.""#]),
        ("**$x$** and [$y$](/z)", &[r#"p "<bold,math|x> and <math,@/z|y>""#]),
        ("$a\nb$", &[r#"p "<math|a b>""#]),
        ("$$x^2$$", &[r#"formula "x^2""#]),
        ("$$\na &= b \\\\\n  &= c\n$$", &[r#"formula "a &= b \\\\\n  &= c""#]),
        ("```math\nx^2\n```", &[r#"formula "x^2""#]),
        ("where\n$$\nx^2\n$$\nholds", &[r#"p "where""#, r#"formula "x^2""#, r#"p "holds""#]),
        ("- step\n  $$\n  x^2\n  $$\n- next", &[r#"bullet/0 "step""#, r#"formula "x^2""#, r#"bullet/0 "next""#]),
        ("> $$x^2$$", &["quote", r#"  formula "x^2""#]),
        ("# On $$x^2$$", &[r#"h1 "On <math|x^2>""#]),
    ];
    for (markdown, expected) in cases {
        assert_eq!(outline(&read(markdown).0), *expected, "reading {markdown:?}");
    }
}

#[test]
fn reads_a_table() {
    let cases: &[(&str, &[&str])] = &[
        (
            "| Year | Rate |\n|:-----|-----:|\n| 2026 | **4 %** |\n| 2027 |",
            &[
                "table:start,end headed",
                "  row",
                r#"    cell "Year""#,
                r#"    cell "Rate""#,
                "  row",
                r#"    cell "2026""#,
                r#"    cell "<bold|4 %>""#,
                "  row",
                r#"    cell "2027""#,
                r#"    cell """#,
            ],
        ),
        ("a|b\n:-:|-\n", &["table:center,start headed", "  row", r#"    cell "a""#, r#"    cell "b""#]),
        // A first row with nothing in it is how Markdown writes a table that has no header.
        (
            "|   |   |\n|---|---|\n| 1 | a<br>b \\| c |",
            &["table:start,start", "  row", r#"    cell "1""#, r#"    cell "a\nb | c""#],
        ),
        (
            "> | a |\n> |---|\n\n- item\n\n  | b |\n  |---|",
            &[
                "quote",
                "  table:start headed",
                "    row",
                r#"      cell "a""#,
                r#"bullet/0 "item""#,
                "table:start headed",
                "  row",
                r#"    cell "b""#,
            ],
        ),
    ];
    for (markdown, expected) in cases {
        assert_eq!(outline(&read(markdown).0), *expected, "reading {markdown:?}");
    }
}

#[test]
fn writes_a_table() {
    let (start, end) = (Align::Start, Align::End);
    let cases = [
        (
            table(&[start, end], true, &[&["Year", "Rate"], &["2026", "4 %"]]),
            "| Year | Rate |\n| --- | --: |\n| 2026 | 4 % |\n",
        ),
        (table(&[Align::Center, Align::Justify], true, &[&["a", "b"]]), "| a | b |\n| :-: | --- |\n"),
        // Markdown has no table without a header, so an empty one stands in.
        (table(&[start, start], false, &[&["1", "2"]]), "|  |  |\n| --- | --- |\n| 1 | 2 |\n"),
        // A row that a merge left short is filled, and one left long widens the table.
        (table(&[start], true, &[&["a", "b"], &[], &["c"]]), "| a | b |\n| --- | --- |\n|  |  |\n| c |  |\n"),
        (table(&[start], true, &[&["a|b\nc", " "]]), "| a\\|b<br>c | &#32; |\n| --- | --- |\n"),
        // A row that a merge left outside its table.
        (
            holding(BlockKind::TableRow, vec![with_text(BlockKind::TableCell, vec![Run::plain("a")])]),
            "|  |\n| --- |\n| a |\n",
        ),
    ];
    for (block, expected) in cases {
        assert_eq!(to_markdown(&doc(vec![block.clone()])), expected, "writing {block:?}");
    }
}

#[test]
fn reads_a_price_as_text() {
    let cases =
        ["costs $5", "between $5 and $10", "$5-$10 each", "US$5/kg and US$6", "$1.50, $2 or $3.25", "5$ and 10$"];
    for markdown in cases {
        assert_eq!(outline(&read(markdown).0), [format!("p {markdown:?}")], "reading {markdown:?}");
        let (back, _) = read(&to_markdown(&read(markdown).0));
        assert_eq!(outline(&back), [format!("p {markdown:?}")], "writing {markdown:?}");
    }
}

#[test]
fn writes_maths_between_dollar_signs() {
    let maths = |tex: &str| Run::marked(tex, [Mark::Math]);
    let formula = |tex: &str| with_text(BlockKind::Formula, vec![Run::plain(tex)]);
    let cases = [
        (
            with_text(BlockKind::Paragraph, vec![Run::plain("so "), maths("x^2"), Run::plain(" holds")]),
            "so $x^2$ holds\n",
        ),
        (with_text(BlockKind::Paragraph, vec![Run::marked("x", [Mark::Math, Mark::Bold])]), "**$x$**\n"),
        (with_text(BlockKind::Heading { level: 2 }, vec![Run::plain("On "), maths("a_1*b_2")]), "## On $a_1*b_2$\n"),
        (with_text(BlockKind::Paragraph, vec![maths("x"), Run::plain(" costs $5")]), "$x$ costs \\$5\n"),
        (formula("x^2"), "$$\nx^2\n$$\n"),
        (formula("a &= b \\\\\n&= c"), "$$\na &= b \\\\\n&= c\n$$\n"),
        (Block::new("", BlockKind::Formula), "$$\n$$\n"),
        (holding(BlockKind::Quote, vec![formula("x^2")]), "> $$\n> x^2\n> $$\n"),
    ];
    for (block, expected) in cases {
        assert_eq!(to_markdown(&doc(vec![block.clone()])), expected, "writing {block:?}");
    }
}

#[test]
fn maths_survives_a_round_trip() {
    let inline = [
        r"\frac{a}{b}",
        "a_1 + b_2^{n}",
        r"\text{cost: } \$5",
        "x < y & z > 0",
        "a|b",
        r"\{x\}",
        "a*b*c",
        "[x](y)",
        "f`x`",
        r"a \\ b",
        "#1 ~ !x",
        r"\text{$y$}",
        "2",
    ];
    let display =
        ["x^2", "a &= b \\\\\n  &= c", "a\n- b", "a\n\nb", "", " x ", "> a", "# a", "a\n===", "x $$ y", "a {", "```"];
    let around = |tex: &str| {
        let maths = Run::marked(tex, [Mark::Math]);
        vec![
            with_text(BlockKind::Paragraph, vec![maths.clone()]),
            with_text(BlockKind::Paragraph, vec![Run::plain("so "), maths.clone(), Run::plain(" and "), maths.clone()]),
            with_text(BlockKind::Paragraph, vec![Run::plain("a"), maths.clone(), Run::plain("2 $")]),
            with_text(BlockKind::Heading { level: 1 }, vec![Run::marked("On ", [Mark::Italic]), maths.clone()]),
            item(ListKind::Bullet, 0, "item"),
            with_text(
                BlockKind::ListItem { list: ListKind::Bullet, depth: 1 },
                vec![Run { link: Some("/a".into()), ..maths }],
            ),
        ]
    };
    let formula = |tex: &str| {
        let formula = with_text(
            BlockKind::Formula,
            Some(tex).filter(|tex| !tex.is_empty()).map(Run::plain).into_iter().collect(),
        );
        vec![
            with_text(BlockKind::Paragraph, vec![Run::plain("before")]),
            formula.clone(),
            item(ListKind::Ordered, 0, "item"),
            formula.clone(),
            holding(BlockKind::Callout { tone: Tone::Tip }, vec![formula]),
        ]
    };
    let cases = inline.iter().map(|tex| around(tex)).chain(display.iter().map(|tex| formula(tex)));
    for blocks in cases {
        let before = doc(blocks);
        let markdown = to_markdown(&before);
        let (after, notes) = read(&markdown);
        assert_eq!(settled(&after), settled(&before), "through {markdown:?}");
        assert_eq!(notes, [], "through {markdown:?}");
    }
}

#[test]
fn writes_maths_that_dollar_signs_cannot_hold_as_code() {
    let cases = [("a } b", "`a } b`"), ("x $ y", "`x $ y`"), (" x ", " $x$ "), ("a\nb", "$a$\\\n$b$")];
    for (tex, expected) in cases {
        let block = with_text(
            BlockKind::Paragraph,
            vec![Run::plain("so"), Run::marked(tex, [Mark::Math]), Run::plain("holds")],
        );
        assert_eq!(to_markdown(&doc(vec![block])), format!("so{expected}holds\n"), "writing {tex:?}");
    }
}

#[test]
fn reads_callout_tags_and_their_aliases() {
    let cases = [
        ("INFO", Tone::Info),
        ("TIP", Tone::Tip),
        ("WARN", Tone::Warn),
        ("DANGER", Tone::Danger),
        ("NOTE", Tone::Info),
        ("IMPORTANT", Tone::Info),
        ("WARNING", Tone::Warn),
        ("CAUTION", Tone::Danger),
        ("note", Tone::Info),
    ];
    for (tag, tone) in cases {
        for markdown in [format!("> [!{tag}]\n> Body"), format!("> [!{tag}]\n>\n> Body")] {
            let (doc, notes) = read(&markdown);
            assert_eq!(doc.blocks.len(), 1, "reading {markdown:?}");
            assert_eq!(doc.blocks[0].kind, BlockKind::Callout { tone }, "reading {markdown:?}");
            assert_eq!(outline(&doc)[1..], [r#"  p "Body""#], "reading {markdown:?}");
            assert_eq!(notes, [], "reading {markdown:?}");
        }
    }
}

#[test]
fn leaves_other_quotes_as_quotes() {
    let cases =
        ["> [!SPARKLE]\n> Body", "> \\[!NOTE\\]\n> Body", "> [!NOTE]Body", "> Body\n> [!NOTE]", "[!NOTE]\nBody"];
    for markdown in cases {
        let (doc, _) = read(markdown);
        let callouts = doc.walk().filter(|block| matches!(block.kind, BlockKind::Callout { .. })).count();
        assert_eq!(callouts, 0, "reading {markdown:?}");
        assert!(doc.walk().any(|block| block.plain_text().contains("Body")), "reading {markdown:?}");
    }
}

#[test]
fn reads_a_callout_title_as_its_first_line() {
    let (doc, _) = read("> [!info] Heads up\n> Body");
    assert_eq!(outline(&doc), ["callout:info", r#"  p "Heads up\nBody""#]);
}

#[test]
fn notes_what_an_import_simplified() {
    // Each case: the Markdown, a phrase from the note, and text the document must still hold.
    let cases = [
        ("##### Deep", "level 5 became level 4", "Deep"),
        ("3. three\n4. four", "started at 3", "three"),
        ("- one\n\n  two", "joined", "one\ntwo"),
        ("- item\n\n  ```\n  code\n  ```", "Moved out of a list item", "code"),
        ("<div class=\"x\">\nraw\n</div>", "HTML", "<div class=\"x\">"),
        ("text <span>inline</span>", "HTML", "<span>inline</span>"),
        ("# Title $$x^2$$", "Display maths", "x^2"),
        ("- item $$x^2$$", "Moved out of a list item", "x^2"),
        ("Claim[^1]\n\n[^1]: Source", "Footnote", "[^1]: Source"),
        ("![A cat](cat.png)", "Image", "A cat"),
        ("[a](/b \"Hover\")", "Hover", "a"),
        ("```rust ignore\nfn f() {}\n```", "ignore", "fn f() {}"),
    ];
    for (markdown, phrase, kept) in cases {
        let (doc, notes) = read(markdown);
        let noted: Vec<_> = notes.iter().filter(|note| note.message.contains(phrase)).collect();
        assert!(!noted.is_empty(), "notes for {markdown:?}: {notes:?}");
        assert!(noted.iter().all(|note| doc.block(&BlockId::new(&note.block)).is_some()), "{notes:?} sit on blocks");
        assert!(doc.walk().any(|block| block.plain_text().contains(kept)), "{kept:?} kept from {markdown:?}");
    }
}

#[test]
fn keeps_front_matter_as_the_documents_properties() {
    let cases = [
        (
            "---\ntitle: Notes\ntags: [a, b]\n---\n\n# Title\n",
            PropertiesFormat::Yaml,
            "title: Notes\ntags: [a, b]\n",
            1,
        ),
        ("+++\r\ntitle = \"Notes\"\r\n+++\r\nBody", PropertiesFormat::Toml, "title = \"Notes\"\n", 1),
        ("---\n# a comment\n\nkey: |\n  - text\n---\n", PropertiesFormat::Yaml, "# a comment\n\nkey: |\n  - text\n", 0),
        ("---\n---\nBody", PropertiesFormat::Yaml, "", 1),
    ];
    for (markdown, format, source, blocks) in cases {
        let (doc, notes) = read(markdown);
        let properties = doc.properties.as_ref().unwrap_or_else(|| panic!("{markdown:?} has front matter"));
        assert_eq!((properties.format, properties.source.as_str()), (format, source), "reading {markdown:?}");
        assert_eq!((doc.blocks.len(), notes.as_slice()), (blocks, [].as_slice()), "reading {markdown:?}");
        let (back, _) = read(&to_markdown(&doc));
        assert_eq!(settled(&back), settled(&doc), "through {:?}", to_markdown(&doc));
    }
}

#[test]
fn reads_front_matter_only_where_a_file_starts_with_it() {
    let cases = [
        "---\n\ntitle: a divider and text\n\n---\n",
        "Text\n\n---\ntitle: Notes\n---\n",
        "---\nnever closed\n",
        " ---\na: b\n---\n",
    ];
    for markdown in cases {
        let (doc, _) = read(markdown);
        assert_eq!(doc.properties, None, "reading {markdown:?}");
        assert!(!doc.blocks.is_empty(), "reading {markdown:?}");
    }
}

#[test]
fn writes_properties_that_front_matter_cannot_hold_as_code() {
    let properties =
        |source: &str| Properties { format: PropertiesFormat::Yaml, source: source.into(), extra: Default::default() };
    let cases = [
        ("title: Notes\n", "---\ntitle: Notes\n---\n\nBody\n"),
        ("title: Notes", "---\ntitle: Notes\n---\n\nBody\n"),
        ("", "---\n---\n\nBody\n"),
        ("a: 1\n---\nb: 2\n", "```yaml\na: 1\n---\nb: 2\n```\n\nBody\n"),
        ("\na: 1\n", "```yaml\n\na: 1\n```\n\nBody\n"),
    ];
    for (source, expected) in cases {
        let body = with_text(BlockKind::Paragraph, vec![Run::plain("Body")]);
        let doc = Document { properties: Some(properties(source)), ..doc(vec![body]) };
        assert_eq!(to_markdown(&doc), expected, "writing {source:?}");
    }
}

#[test]
fn imports_plain_markdown_without_notes() {
    let markdown = "# Title\n\nSome **bold** text with a [link](/a).\n\n- one\n- two\n\n> [!NOTE]\n> Careful.\n";
    assert_eq!(read(markdown).1, []);
}

#[test]
fn writes_every_block() {
    let plain = |kind, text: &str| with_text(kind, vec![Run::plain(text)]);
    let cases = [
        (plain(BlockKind::Paragraph, "Body"), "Body\n"),
        (plain(BlockKind::Paragraph, "one\ntwo"), "one\\\ntwo\n"),
        (plain(BlockKind::Heading { level: 3 }, "Title"), "### Title\n"),
        (item(ListKind::Bullet, 0, "item"), "- item\n"),
        (item(ListKind::Ordered, 0, "item"), "1. item\n"),
        (item(ListKind::Task { checked: false }, 0, "item"), "- [ ] item\n"),
        (item(ListKind::Task { checked: true }, 0, "item"), "- [x] item\n"),
        (plain(BlockKind::Code { language: Some("rust".into()) }, "fn main() {}"), "```rust\nfn main() {}\n```\n"),
        (plain(BlockKind::Code { language: None }, "a\n```\nb"), "````\na\n```\nb\n````\n"),
        (holding(BlockKind::Quote, vec![plain(BlockKind::Paragraph, "Said")]), "> Said\n"),
        (Block::new("", BlockKind::Divider), "---\n"),
    ];
    for (block, expected) in cases {
        assert_eq!(to_markdown(&doc(vec![block.clone()])), expected, "writing {:?}", block.kind);
    }
}

#[test]
fn writes_callouts_under_the_names_github_renders() {
    let cases = [(Tone::Info, "NOTE"), (Tone::Tip, "TIP"), (Tone::Warn, "WARNING"), (Tone::Danger, "CAUTION")];
    for (tone, name) in cases {
        let body = with_text(BlockKind::Paragraph, vec![Run::plain("Body")]);
        let markdown = to_markdown(&doc(vec![holding(BlockKind::Callout { tone }, vec![body])]));
        assert_eq!(markdown, format!("> [!{name}]\n> Body\n"));
    }
}

#[test]
fn writes_every_mark() {
    let cases = [
        (Run::marked("text", [Mark::Bold]), "**text**"),
        (Run::marked("text", [Mark::Italic]), "*text*"),
        (Run::marked("text", [Mark::Strike]), "~~text~~"),
        (Run::marked("text", [Mark::Code]), "`text`"),
        (Run::marked("text", [Mark::Underline]), "<u>text</u>"),
        (Run::marked("text", [Mark::Highlight]), "<mark>text</mark>"),
        (Run::marked("text", [Mark::Bold, Mark::Italic]), "***text***"),
        (linked("text", "https://example.com/a?b=1&c=2"), "[text](https://example.com/a?b=1&c=2)"),
        (linked("https://example.com", "https://example.com"), "<https://example.com>"),
    ];
    for (run, expected) in cases {
        let markdown =
            to_markdown(&doc(vec![with_text(BlockKind::Paragraph, vec![Run::plain("a "), run, Run::plain(" b")])]));
        assert_eq!(markdown, format!("a {expected} b\n"));
    }
}

#[test]
fn writes_nested_and_numbered_lists() {
    let blocks = vec![
        item(ListKind::Ordered, 0, "one"),
        item(ListKind::Bullet, 1, "inner"),
        item(ListKind::Ordered, 2, "deep"),
        item(ListKind::Ordered, 0, "two"),
        item(ListKind::Bullet, 0, "other"),
        with_text(BlockKind::Paragraph, vec![Run::plain("After")]),
    ];
    assert_eq!(to_markdown(&doc(blocks)), "1. one\n   - inner\n     1. deep\n2. two\n\n- other\n\nAfter\n");
}

#[test]
fn keeps_text_that_looks_like_markdown_as_text() {
    let cases = [
        "# not a heading",
        "- not a list",
        "1. not a list",
        "> not a quote",
        "---",
        "*not italic* and _nor this_",
        "snake_case and 2 * 3",
        "`not code` and ```",
        "[not](a link) and ![not](an image)",
        "<b>not bold</b> &amp; <!-- not a comment -->",
        "[!NOTE]",
        "costs $5 or $10, and $x$ is not maths",
        "a | b\n- | -",
        "Title\n===",
        "  spaced out  ",
        "trailing backslash\\",
        "[^1]: not a footnote",
        "~~~",
    ];
    let kinds = [
        BlockKind::Paragraph,
        BlockKind::Heading { level: 2 },
        BlockKind::ListItem { list: ListKind::Bullet, depth: 0 },
    ];
    for case in cases {
        for kind in &kinds {
            let before = doc(vec![with_text(kind.clone(), vec![Run::plain(case)])]);
            let markdown = to_markdown(&before);
            assert_eq!(settled(&read(&markdown).0), settled(&before), "{case:?} written as {markdown:?}");
        }
    }
}

#[test]
fn keeps_marks_where_delimiters_would_not_read_back() {
    let bold = |text: &str| Run::marked(text, [Mark::Bold]);
    let cases = [
        vec![bold("ends in a full stop."), Run::plain("then a letter")],
        vec![Run::plain("mid"), Run::marked("word", [Mark::Italic]), Run::plain("emphasis")],
        vec![bold("bold"), Run::marked("both", [Mark::Bold, Mark::Italic]), Run::marked("italic", [Mark::Italic])],
        vec![Run::marked("a", [Mark::Strike]), Run::marked("b", [Mark::Bold]), Run::marked("c", [Mark::Italic])],
        vec![Run::marked("\"quoted\"", [Mark::Italic]), Run::plain("s")],
        vec![bold("*"), Run::marked("_", [Mark::Italic]), Run::marked("~", [Mark::Strike])],
        vec![Run { link: Some(String::new()), ..Run::marked("]:", [Mark::Code]) }],
        vec![Run::plain("$5 at "), linked("the shop", "/pay?in=$")],
    ];
    for runs in cases {
        let before = doc(vec![with_text(BlockKind::Paragraph, runs)]);
        let markdown = to_markdown(&before);
        assert_eq!(settled(&read(&markdown).0), settled(&before), "written as {markdown:?}");
    }
}

#[test]
fn moves_spaces_out_of_emphasis() {
    let runs = vec![Run::plain("a"), Run::marked(" bold ", [Mark::Bold]), Run::plain("b")];
    assert_eq!(to_markdown(&doc(vec![with_text(BlockKind::Paragraph, runs)])), "a **bold** b\n");
}

#[test]
fn writes_what_markdown_cannot_say_as_best_it_can() {
    let mut hologram = Block::new("", BlockKind::Unknown { type_name: "hologram".into() });
    hologram.extra.insert("fallback".into(), "A hologram".into());
    let cases = [
        (vec![hologram], "A hologram\n"),
        (vec![with_text(BlockKind::Paragraph, vec![]), with_text(BlockKind::Paragraph, vec![Run::plain("a")])], "a\n"),
        (vec![item(ListKind::Bullet, 3, "orphan"), item(ListKind::Bullet, 5, "child")], "- orphan\n  - child\n"),
        (vec![with_text(BlockKind::Paragraph, vec![Run::marked("glow", [Mark::Other("sparkle".into())])])], "glow\n"),
        (vec![with_text(BlockKind::Heading { level: 9 }, vec![Run::plain("Deep")])], "###### Deep\n"),
    ];
    for (blocks, expected) in cases {
        assert_eq!(to_markdown(&doc(blocks)), expected);
    }
}

#[test]
fn round_trips_canonical_markdown_unchanged() {
    let cases = [
        "# Title\n\nA paragraph with **bold**, *italic*, ~~struck~~, `code`, <u>underline</u> and <mark>highlight</mark>.\n",
        "1. one\n2. two\n   - [ ] todo\n   - [x] done\n3. three\n",
        "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n",
        "> A quote\n>\n> > nested\n>\n> - with a list\n",
        "> [!WARNING]\n> Mind the gap.\n>\n> ```\n> code\n> ```\n",
        "> [!NOTE]\n>\n> - a list first\n",
        "See [the docs](https://example.com/docs) or <https://example.com>.\n",
        "Line one\\\nline two\n\n---\n\n-\n- after an empty item\n",
        ">\n\n#\n\n```\n```\n",
        "- text\n\n  -\n",
        "-\n  -\n",
        "- bullets\n\n1. then numbers\n",
        "- not a | table\n  - \\|\n",
        "Before\n\n| Name | Size |\n| --- | --: |\n| `a\\|b` | $x^2$ |\n| *c* |  |\n\nAfter\n",
        "> |  |  |\n> | :-: | --- |\n> | no | header |\n",
        "---\ntitle: Notes\n---\n\n# Notes\n\nSo $x^2$ holds:\n\n$$\nx^2 \\ge 0\n$$\n",
        "+++\ntitle = \"Notes\"\n+++\n",
    ];
    for markdown in cases {
        assert_eq!(to_markdown(&read(markdown).0), markdown);
    }
}

/// The conformance corpus ships each sample with the export a reader should expect.
#[test]
fn exports_the_conformance_samples() {
    let cases = [
        (include_str!("../../letters-format/corpus/document/report.json"), include_str!("corpus/report.md")),
        (include_str!("../../letters-format/corpus/document/notice.json"), include_str!("corpus/notice.md")),
    ];
    for (sample, expected) in cases {
        let sample = Document::from_json(sample).expect("the corpus holds a valid document");
        assert_eq!(to_markdown(&sample), expected);
    }
}

#[test]
fn bounds_how_deep_it_recurses() {
    let quotes = format!("{}deep", "> ".repeat(10_000));
    let (doc, notes) = read(&quotes);
    assert!(doc.walk().any(|block| block.plain_text().contains("deep")));
    assert_eq!(notes.len(), 1);

    let lists: String = (0..200).map(|depth| format!("{}- item\n", "  ".repeat(depth))).collect();
    let (doc, _) = read(&lists);
    assert!(doc.walk().count() > 30);
}

/// Text that leans on the characters Markdown gives meaning to.
fn any_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        3 => "[a-z]{1,4}",
        2 => Just(" ".to_owned()),
        4 => proptest::sample::select(vec![
            "*", "**", "_", "`", "~", "~~", "[", "]", "(", ")", "<", ">", "#", "-", "+", "=", "|", ":", "!", "$", "&",
            "\\", "1.", "\n", "\t", ";", "amp", "\"", "^", "/", "é", "中", "\u{a0}", "---", "[ ]", "<u>", "</b>", "<br>",
        ])
        .prop_map(str::to_owned),
    ];
    proptest::collection::vec(piece, 1..8).prop_map(|pieces| pieces.concat())
}

/// TeX that sits between dollar signs: its braces pair up, its dollars are escaped, and no space is at its edge.
fn any_tex() -> impl Strategy<Value = String> {
    let piece = proptest::sample::select(vec![
        "x",
        "2",
        "a_1",
        "^2",
        " + ",
        " ",
        "=",
        r"\frac{a}{b}",
        r"\alpha",
        r"\$",
        "{y}",
        "|",
        "*",
        "_",
        "<",
        ">",
        "&",
        "[",
        "]",
        "(",
        ")",
        "`",
        r"\\",
        "-",
        "!",
        "#",
        "~",
        "é",
    ]);
    proptest::collection::vec(piece, 1..6)
        .prop_map(|pieces| pieces.concat().trim().to_owned())
        .prop_filter("maths has a source", |tex| !tex.is_empty())
}

fn any_runs() -> impl Strategy<Value = Vec<Run>> {
    let marks = [Mark::Bold, Mark::Italic, Mark::Underline, Mark::Strike, Mark::Highlight, Mark::Code];
    let marks = prop_oneof![
        3 => Just(Vec::new()),
        3 => proptest::sample::subsequence(marks.to_vec(), 1..=2),
        1 => proptest::sample::subsequence(marks.to_vec(), 0..=6),
    ];
    let urls = vec![
        "https://example.com/a?b=1&c=2",
        "/relative path",
        "",
        "x(y)&amp;z\\",
        "<q>",
        "/pay?in=$",
        "https://a.b/$c",
    ];
    let link = proptest::option::weighted(0.2, proptest::sample::select(urls));
    let text = (any_text(), marks);
    let maths = (any_tex(), proptest::sample::subsequence(vec![Mark::Math, Mark::Bold, Mark::Underline], 1..=3))
        .prop_map(|(tex, marks)| (tex, [vec![Mark::Math], marks].concat()));
    let run = (prop_oneof![5 => text, 1 => maths], link)
        .prop_map(|((text, marks), link)| Run { link: link.map(str::to_owned), ..Run::marked(text, marks) });
    proptest::collection::vec(run, 1..4)
}

fn any_blocks() -> impl Strategy<Value = Vec<Block>> {
    let list = prop_oneof![
        Just(ListKind::Bullet),
        Just(ListKind::Ordered),
        any::<bool>().prop_map(|checked| ListKind::Task { checked }),
    ];
    let tone = proptest::sample::select(vec![Tone::Tip, Tone::Info, Tone::Warn, Tone::Danger]);
    let some_runs = || prop_oneof![4 => any_runs(), 1 => Just(Vec::new())];
    let leaf = prop_oneof![
        3 => any_runs().prop_map(|runs| with_text(BlockKind::Paragraph, runs)),
        1 => (1..=4u8, some_runs()).prop_map(|(level, runs)| with_text(BlockKind::Heading { level }, runs)),
        3 => (list, 0..3u8, some_runs()).prop_map(|(list, depth, runs)| with_text(BlockKind::ListItem { list, depth }, runs)),
        1 => (proptest::option::of("[a-z+#]{1,5}"), proptest::option::of(any_text())).prop_map(|(language, code)| {
            with_text(BlockKind::Code { language }, code.into_iter().map(Run::plain).collect())
        }),
        1 => Just(Block::new("", BlockKind::Divider)),
        1 => proptest::option::of(prop_oneof![any_tex(), any_text()])
            .prop_map(|tex| with_text(BlockKind::Formula, tex.into_iter().map(Run::plain).collect())),
        1 => any_table(),
    ];
    let block = leaf.prop_recursive(3, 16, 4, move |inner| {
        let children = proptest::collection::vec(inner, 0..4);
        prop_oneof![
            children.clone().prop_map(|blocks| holding(BlockKind::Quote, blocks)),
            (tone.clone(), children).prop_map(|(tone, blocks)| holding(BlockKind::Callout { tone }, blocks)),
        ]
    });
    proptest::collection::vec(block, 0..6).prop_map(nest_items)
}

/// A table as a merge may leave one: rows of any length under any number of columns. Its first row heads it
/// only where that row has text, since Markdown writes a table with no header as one with an empty first row.
fn any_table() -> impl Strategy<Value = Block> {
    let cell =
        prop_oneof![3 => any_runs(), 1 => Just(Vec::new())].prop_map(|runs| with_text(BlockKind::TableCell, runs));
    let row = proptest::collection::vec(cell, 0..4).prop_map(|cells| holding(BlockKind::TableRow, cells));
    let align = proptest::sample::select(vec![Align::Start, Align::Center, Align::End]);
    (proptest::collection::vec(align, 0..4), any::<bool>(), proptest::collection::vec(row, 1..4)).prop_map(
        |(columns, header, rows)| {
            let headed = rows[0].body.blocks().iter().any(|cell| !cell.plain_text().is_empty());
            holding(BlockKind::Table { columns, header: header && headed || rows.len() == 1 }, rows)
        },
    )
}

/// Markdown nests a list item under the one before it, so an item goes at most one level deeper than that.
fn nest_items(mut blocks: Vec<Block>) -> Vec<Block> {
    let mut deepest = 0;
    for block in &mut blocks {
        deepest = match &mut block.kind {
            BlockKind::ListItem { depth, .. } => {
                *depth = (*depth).min(deepest);
                *depth + 1
            }
            _ => 0,
        };
        if let Body::Blocks(children) = &mut block.body {
            *children = nest_items(std::mem::take(children));
        }
    }
    blocks
}

/// Front matter as a file holds it: lines of anything but a blank first line or the fence that would end it.
fn any_properties() -> impl Strategy<Value = Option<Properties>> {
    let line = proptest::sample::select(vec![
        "title: Notes",
        "tags: [a, b]",
        "# - *x* $",
        "  - nested",
        "",
        "+++",
        "---",
        "\t",
    ]);
    let format = proptest::sample::select(vec![(PropertiesFormat::Yaml, "---"), (PropertiesFormat::Toml, "+++")]);
    let properties = (format, proptest::collection::vec(line, 0..4)).prop_map(|((format, fence), lines)| {
        let lines: Vec<&str> = lines.into_iter().filter(|line| *line != fence).collect();
        let source: String =
            lines.iter().skip_while(|line| line.trim().is_empty()).map(|line| format!("{line}\n")).collect();
        Properties { format, source, extra: Default::default() }
    });
    proptest::option::of(properties)
}

/// Anything a Markdown file might hold, well formed or not.
fn any_markdown() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        4 => any_text(),
        4 => proptest::sample::select(vec![
            "\n", "\n\n", "# ", "## ", "###### ", "- ", "* ", "1. ", "7) ", "- [ ] ", "- [x] ", "> ", "> > ", "  ", "    ",
            "```\n", "~~~rust\n", "---\n", "===\n", "> [!NOTE]\n", "> [!warn] ", "[!TIP]", "| a | b |\n|---|---|\n", "<div>\n",
            "<!-- c -->", "<u>", "</u>", "<mark>", "<br>", "<span>", "$", "$$\n", "[^1]", "[^1]: ", "[a]: /b\n", "[a]",
            "[[w]]", "![i](s)", "[l](u \"t\")", "<https://e.com>", "&#32;", "&amp;", "\\\n", "  \n", "+++\n", "\r\n",
        ])
        .prop_map(str::to_owned),
    ];
    proptest::collection::vec(piece, 0..24).prop_map(|pieces| pieces.concat())
}

proptest! {
    #[test]
    fn a_document_survives_a_round_trip(blocks in any_blocks(), properties in any_properties()) {
        let before = Document { properties, ..doc(blocks) };
        let markdown = to_markdown(&before);
        let (after, notes) = read(&markdown);
        prop_assert_eq!(settled(&after), settled(&before), "through {:?}", markdown);
        prop_assert_eq!(notes, []);
    }

    #[test]
    fn imported_markdown_survives_a_round_trip(markdown in any_markdown()) {
        let (imported, notes) = read(&markdown);
        for note in &notes {
            prop_assert!(imported.block(&BlockId::new(&note.block)).is_some(), "{:?} has no block", note);
        }
        let written = to_markdown(&imported);
        let (again, _) = read(&written);
        prop_assert_eq!(settled(&again), settled(&imported), "through {:?}", written);
    }

    #[test]
    fn a_round_trip_settles_at_once(blocks in any_blocks()) {
        let once = to_markdown(&doc(blocks));
        let twice = to_markdown(&read(&once).0);
        prop_assert_eq!(twice, once);
    }
}
