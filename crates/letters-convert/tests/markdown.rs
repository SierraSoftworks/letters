//! Markdown import and export, through the two public functions.

use letters_convert::{ImportNote, from_markdown, to_markdown};
use letters_model::{Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Run, Tone};
use proptest::prelude::*;

fn read(markdown: &str) -> (Document, Vec<ImportNote>) {
    let mut next = 0;
    from_markdown(markdown, &mut || {
        next += 1;
        format!("b{next}")
    })
}

/// A document as indented lines such as `h1 "Title"`, with marked runs as `<bold,italic|text>` and linked ones
/// as `<@url|text>`, so that a case can state the document it expects.
fn outline(doc: &Document) -> Vec<String> {
    fn runs(block: &Block) -> String {
        let run = |run: &Run| {
            let mut format: Vec<String> = run.marks.iter().map(|mark| format!("{mark:?}").to_lowercase()).collect();
            format.extend(run.link.iter().map(|url| format!("@{url}")));
            if format.is_empty() { run.text.clone() } else { format!("<{}|{}>", format.join(","), run.text) }
        };
        format!("{:?}", block.body.runs().iter().map(run).collect::<String>())
    }
    fn push(blocks: &[Block], indent: usize, out: &mut Vec<String>) {
        for block in blocks {
            let line = match &block.kind {
                BlockKind::Paragraph => format!("p {}", runs(block)),
                BlockKind::Heading { level } => format!("h{level} {}", runs(block)),
                BlockKind::ListItem { list: ListKind::Bullet, depth } => format!("bullet/{depth} {}", runs(block)),
                BlockKind::ListItem { list: ListKind::Ordered, depth } => format!("ordered/{depth} {}", runs(block)),
                BlockKind::ListItem { list: ListKind::Task { checked: true }, depth } => {
                    format!("done/{depth} {}", runs(block))
                }
                BlockKind::ListItem { list: ListKind::Task { checked: false }, depth } => {
                    format!("todo/{depth} {}", runs(block))
                }
                BlockKind::Code { language } => format!("code:{} {}", language.as_deref().unwrap_or("-"), runs(block)),
                BlockKind::Quote => "quote".to_owned(),
                BlockKind::Callout { tone } => format!("callout:{tone:?}").to_lowercase(),
                BlockKind::Divider => "divider".to_owned(),
                BlockKind::Unknown { type_name } => format!("unknown:{type_name}"),
            };
            out.push(format!("{:indent$}{line}", ""));
            push(block.body.blocks(), indent + 2, out);
        }
    }
    let mut out = Vec::new();
    push(&doc.blocks, 0, &mut out);
    out
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

fn linked(text: &str, url: &str) -> Run {
    Run { link: Some(url.into()), ..Run::plain(text) }
}

/// A document with what a round trip may change set aside: block ids, emphasis on spaces and code on line breaks.
fn settled(doc: &Document) -> Document {
    fn settle_block(block: &Block) -> Block {
        let body = match &block.body {
            Body::Text(runs) if !matches!(block.kind, BlockKind::Code { .. }) => Body::Text(settle(runs)),
            Body::Blocks(blocks) => Body::Blocks(blocks.iter().map(settle_block).collect()),
            other => other.clone(),
        };
        Block { id: BlockId::new(""), body, ..block.clone() }
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
        ("| a | b |\n|---|--:|\n| 1 | 2 |", "Table", "| 1 | 2 |"),
        ("<div class=\"x\">\nraw\n</div>", "HTML", "<div class=\"x\">"),
        ("text <span>inline</span>", "HTML", "<span>inline</span>"),
        ("---\ntitle: Notes\n---\n\nBody", "Front matter", "title: Notes"),
        ("+++\r\ntitle = \"Notes\"\r\nlang = \"en\"\r\n+++\r\nBody", "Front matter", "title = \"Notes\"\nlang"),
        ("Euler: $e^{i\\pi} + 1 = 0$", "Maths", "$e^{i\\pi} + 1 = 0$"),
        ("$$\nx^2\n$$", "Maths", "x^2"),
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
    ];
    for markdown in cases {
        assert_eq!(to_markdown(&read(markdown).0), markdown);
    }
}

/// The conformance corpus ships each sample with the export a reader should expect.
#[test]
fn exports_the_conformance_report() {
    let report = include_str!("../../letters-format/corpus/document/report.json");
    let report = Document::from_json(report).expect("the corpus holds a valid document");
    assert_eq!(to_markdown(&report), include_str!("corpus/report.md"));
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
    let run = (any_text(), marks, link)
        .prop_map(|(text, marks, link)| Run { link: link.map(str::to_owned), ..Run::marked(text, marks) });
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
    fn a_document_survives_a_round_trip(blocks in any_blocks()) {
        let before = doc(blocks);
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
