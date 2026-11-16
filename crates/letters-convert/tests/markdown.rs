//! Markdown import and export, through the two public functions.

mod common;

use letters_convert::{ImportNote, from_markdown, import_markdown, markdown_assets, to_markdown};
use std::collections::BTreeMap;

use letters_model::{
    Align, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Preview, Properties, PropertiesFormat, Run, Tone,
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

/// A document of `blocks`, each given an id where it has none, as every block of a document has one.
fn doc(mut blocks: Vec<Block>) -> Document {
    fn name(blocks: &mut [Block], next: &mut usize) {
        for block in blocks {
            *next += 1;
            if block.id.as_str().is_empty() {
                block.id = format!("n{next}").as_str().into();
            }
            if let Body::Blocks(children) = &mut block.body {
                name(children, next);
            }
        }
    }
    name(&mut blocks, &mut 0);
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

/// A figure whose picture is still at `src`, at the size one has until its own is known.
fn figure(src: &str, alt: &str, numbered: bool, caption: Vec<Run>) -> Block {
    let kind = BlockKind::Figure {
        asset: None,
        src: Some(src.into()),
        alt: alt.into(),
        width: 320.0,
        height: 240.0,
        numbered,
    };
    with_text(kind, caption)
}

/// A picture of 40 by 20 pixels, and one of 1200 by 600.
const SMALL: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x28\x00\x00\x00\x14\x01\x00\x00\x00\x00\xd7\x3d\x42\x16\x00\x00\x00\x0c\x49\x44\x41\x54\x78\xda\x63\x60\x18\x18\x00\x00\x00\x78\x00\x01\x35\x33\x6e\xa4\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";
const WIDE: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x04\xb0\x00\x00\x02\x58\x01\x00\x00\x00\x00\x5a\x9d\x22\xb7\x00\x00\x00\x6f\x49\x44\x41\x54\x78\xda\xed\xc1\x81\x00\x00\x00\x00\xc3\xa0\xf9\x53\x5f\xe1\x00\x55\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\xaf\x01\x61\xf7\x00\x01\x76\x1d\xb3\x81\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

fn linked(text: &str, url: &str) -> Run {
    Run { link: Some(url.into()), ..Run::plain(text) }
}

/// A document with what a round trip may change set aside: block ids, emphasis on spaces, code on line breaks,
/// the cells a table's short rows lack, and the previews of pages no link leads to.
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
        // A caption is written in italics from end to end, and one with nothing to show is not written.
        let numbered = match &mut kind {
            BlockKind::Figure { alt, numbered, .. } => {
                *alt = alt.replace(['\n', '\r'], " ");
                Some(*numbered)
            }
            BlockKind::Caption => Some(true),
            _ => None,
        };
        if let (Some(numbered), Body::Text(runs)) = (numbered, &mut body) {
            let upright = |run: &Run| Run {
                marks: run.marks.iter().filter(|mark| **mark != Mark::Italic).cloned().collect(),
                ..run.clone()
            };
            let shown = numbered || block.body.runs().iter().any(|run| !run.text.trim().is_empty());
            *runs = settle(&block.body.runs().iter().filter(|_| shown).map(upright).collect::<Vec<_>>());
        }
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
    let mut settled = Document { blocks: doc.blocks.iter().map(settle_block).collect(), ..doc.clone() };
    // A preview of a page shows on a link to it, so one that no link names is not written.
    settled.prune_previews();
    settled
}

/// What a document keeps of the pages at `titles`' addresses: for each, its title.
fn previews(titles: &[(&str, &str)]) -> BTreeMap<String, Preview> {
    let preview = |title: &str| Preview { title: Some(title.into()), ..Preview::default() };
    titles.iter().map(|(address, title)| ((*address).to_owned(), preview(title))).collect()
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
        // Markdown drops these at the edges of a paragraph, so the text beside a formula loses them too.
        ("where\n\u{b}$$\nx^2\n$$\n\u{c}holds\u{b}", &[r#"p "where""#, r#"formula "x^2""#, r#"p "holds""#]),
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
fn reads_pictures() {
    let (a, b) = ("https://example.com/a.png", "https://example.com/b.png");
    let cases: &[(String, &[&str])] = &[
        (format!("![A cat]({a})"), &[r#"figure https://example.com/a.png [A cat] """#]),
        // The caption is the emphasised paragraph under the picture, or the emphasised line.
        (
            format!("![A *fine* cat]({a})\n\n*It **sat** down.*\n\nNext"),
            &[r#"figure https://example.com/a.png [A fine cat] "It <bold|sat> down.""#, r#"p "Next""#],
        ),
        (format!("![a]({a})\n_Under it_"), &[r#"figure https://example.com/a.png [a] "Under it""#]),
        (format!("![a]({a} \"Its title\")"), &[r#"figure https://example.com/a.png [a] "Its title""#]),
        (
            format!("![a]({a})\n![b]({b})\n\n*Both*"),
            &[
                "gallery",
                r#"  figure https://example.com/a.png [a] """#,
                r#"  figure https://example.com/b.png [b] """#,
                r#"  caption "Both""#,
            ],
        ),
        // Only a paragraph that is emphasised from end to end is a caption.
        (
            format!("![a]({a})\n\n*Not* a caption"),
            &[r#"figure https://example.com/a.png [a] """#, r#"p "<italic|Not> a caption""#],
        ),
        (
            format!("![a]({a})\n\n<i>Aside</i>"),
            &[r#"figure https://example.com/a.png [a] """#, r#"p "<italic|Aside>""#],
        ),
        // A caption that opens with a number makes its figure numbered, and the text can refer to it.
        (
            format!("See Figure 2, not Figure 9.\n\n![a]({a})\n\n*Figure 2: Rates*"),
            &[r#"p "See <ref:b2|Figure 2>, not Figure 9.""#, r#"figure# https://example.com/a.png [a] "Rates""#],
        ),
        (
            format!("![a]({a})\n![b]({b})\n\n*Figure 1*\n\nAs Figure 1 shows."),
            &[
                "gallery#",
                r#"  figure https://example.com/a.png [a] """#,
                r#"  figure https://example.com/b.png [b] """#,
                r#"p "As <ref:b3|Figure 1> shows.""#,
            ],
        ),
    ];
    for (markdown, expected) in cases {
        let (doc, notes) = read(markdown);
        assert_eq!(outline(&doc), *expected, "reading {markdown:?}");
        assert_eq!(notes, [], "reading {markdown:?}");
    }
}

#[test]
fn reads_the_size_of_each_kind_of_picture_from_how_its_file_starts() {
    // Each file is no more than its opening: the size is read there, and nothing is decoded.
    let png = |width: u32, height: u32| {
        let mut file = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        file.extend([width.to_be_bytes(), height.to_be_bytes()].concat());
        file
    };
    let webp = |part: &[u8]| [b"RIFF\0\0\0\0WEBP".as_slice(), part].concat();
    // A JPEG of 64 by 48 that says, or not, that the camera was held on its side.
    let jpeg = |turned: bool| {
        let exif = b"\xff\xe1\0\x1eExif\0\0MM\0*\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0";
        let frame = b"\xff\xc0\0\x0b\x08\0\x30\0\x40\x01\x01\x11\0";
        [b"\xff\xd8".as_slice(), if turned { exif } else { b"" }, frame].concat()
    };
    // A file's name, how it starts, and its size in pixels where it is a picture.
    type Case = (&'static str, Vec<u8>, Option<(f32, f32)>);
    let cases: Vec<Case> = vec![
        ("a.png", png(40, 20), Some((40.0, 20.0))),
        ("a.gif", b"GIF89a\x1e\0\x32\0\0\0\0".to_vec(), Some((30.0, 50.0))),
        ("a.jpg", jpeg(false), Some((64.0, 48.0))),
        ("turned.jpg", jpeg(true), Some((48.0, 64.0))),
        ("lossy.webp", webp(b"VP8 \0\0\0\0\0\0\0\x9d\x01\x2a\x46\0\x21\0"), Some((70.0, 33.0))),
        ("lossless.webp", webp(b"VP8L\0\0\0\0\x2f\x14\x80\x18\0"), Some((21.0, 99.0))),
        ("extended.webp", webp(b"VP8X\x0a\0\0\0\0\0\0\0\x36\0\0\x2b\0\0"), Some((55.0, 44.0))),
        // What does not say its size, or is too large to draw, is no picture.
        ("cut.png", b"\x89PNG\r\n\x1a\n".to_vec(), None),
        ("cut.jpg", b"\xff\xd8\xff\xe0\0\x10JFIF".to_vec(), None),
        ("huge.png", png(70_000, 10), None),
        ("empty.png", png(0, 10), None),
    ];
    for (name, file, expected) in cases {
        let mut next = 0;
        let mut new_id = || {
            next += 1;
            format!("b{next}")
        };
        let import = import_markdown(&format!("![A picture]({name})"), &mut new_id, &mut |_| Some(file.clone()));
        let shown = import.document.blocks.iter().find_map(|block| match block.kind {
            BlockKind::Figure { width, height, .. } => Some((width, height)),
            _ => None,
        });
        // A pixel is three quarters of a point.
        assert_eq!(shown, expected.map(|(width, height)| (width * 0.75, height * 0.75)), "{name}");
        assert_eq!(import.assets.len(), usize::from(expected.is_some()), "{name}");
    }
}

#[test]
fn brings_in_the_pictures_a_file_names() {
    let files = |path: &str| match path {
        "img/small one.png" => Some(SMALL.to_vec()),
        "wide.png" => Some(WIDE.to_vec()),
        "notes.txt" => Some(b"not a picture".to_vec()),
        _ => None,
    };
    let markdown = concat!(
        "![Small](img/small%20one.png)\n\n![Wide](wide.png)\n\n![Again](<img/small one.png>)\n\n",
        "![Text](notes.txt)\n\n![Gone](gone.png)\n\n![Plain](http://example.com/plain.png)\n",
    );
    let mut next = 0;
    let mut new_id = || {
        next += 1;
        format!("b{next}")
    };
    let import = import_markdown(markdown, &mut new_id, &mut { files });

    let sizes: Vec<(Option<String>, f32, f32)> = import
        .document
        .blocks
        .iter()
        .filter_map(|block| match &block.kind {
            BlockKind::Figure { asset, width, height, .. } => Some((asset.clone(), *width, *height)),
            _ => None,
        })
        .collect();
    let [(Some(small), 30.0, 15.0), (Some(wide), 480.0, 240.0), (Some(again), 30.0, 15.0)] = sizes.as_slice() else {
        panic!("three figures at the sizes of their pictures, the wide one fitted to a page: {sizes:?}");
    };
    assert_eq!(small, again, "one picture is kept once");
    assert!(small.starts_with("assets/") && small.ends_with(".png"), "{small}");
    assert_eq!(import.assets.get(small).map(Vec::as_slice), Some(SMALL));
    assert_eq!(import.assets.get(wide).map(Vec::as_slice), Some(WIDE));
    assert_eq!(import.assets.len(), 2);

    // What cannot be shown stays, as a link to where it was.
    let kept: Vec<&Block> = import.document.blocks.iter().filter(|block| block.kind == BlockKind::Paragraph).collect();
    let links: Vec<_> = kept.iter().map(|block| block.body.runs()[0].link.as_deref()).collect();
    assert_eq!(links, [Some("notes.txt"), Some("gone.png"), Some("http://example.com/plain.png")]);
    assert_eq!(import.notes.len(), 3, "{:?}", import.notes);

    // An export names each picture by its entry, and reads back with those entries beside it.
    let exported = to_markdown(&import.document);
    assert!(exported.contains(&format!("![Small]({small})")), "{exported}");
    assert_eq!(
        markdown_assets(&import.document).into_iter().collect::<Vec<_>>(),
        import.assets.keys().cloned().collect::<Vec<_>>()
    );
    let again = import_markdown(&exported, &mut String::new, &mut |path| import.assets.get(path).cloned());
    assert_eq!(settled(&again.document), settled(&import.document));
}

#[test]
fn writes_pictures() {
    let (a, b) = ("https://example.com/a.png", "https://example.com/b.png");
    let plain = |text: &str| with_text(BlockKind::Paragraph, vec![Run::plain(text)]);
    let held = BlockKind::Figure {
        asset: Some("assets/0a1b.png".into()),
        src: None,
        alt: "Held".into(),
        width: 30.0,
        height: 15.0,
        numbered: false,
    };
    let both = |numbered, caption: Option<&str>| {
        let caption = caption.map(|text| with_text(BlockKind::Caption, vec![Run::plain(text)]));
        let parts = [figure(a, "a", false, vec![]), figure(b, "b", false, vec![])].into_iter().chain(caption);
        holding(BlockKind::Gallery { numbered }, parts.collect())
    };
    let cases = [
        (vec![figure(a, "A cat", false, vec![])], "![A cat](https://example.com/a.png)\n".to_owned()),
        (vec![with_text(held, vec![])], "![Held](assets/0a1b.png)\n".to_owned()),
        (
            vec![figure(
                a,
                "A [fine]\ncat",
                false,
                vec![Run::plain("It "), Run::marked("sat", [Mark::Bold]), Run::plain(" down.")],
            )],
            format!("![A \\[fine\\] cat]({a})\n\n*It **sat** down.*\n"),
        ),
        (vec![both(false, Some("Both"))], format!("![a]({a})\n![b]({b})\n\n*Both*\n")),
        // A numbered caption opens with its number, and a reference reads as it whatever text it holds.
        (
            vec![
                with_text(
                    BlockKind::Paragraph,
                    vec![Run::plain("See "), Run::marked("the chart", [Mark::Ref("f".into())]), Run::plain(".")],
                ),
                figure(a, "a", true, vec![Run::plain("Rates")]),
                Block { id: "f".into(), ..both(true, None) },
            ],
            format!("See Figure 2.\n\n![a]({a})\n\n*Figure 1: Rates*\n\n![a]({a})\n![b]({b})\n\n*Figure 2*\n"),
        ),
        // A paragraph in italics under a picture with no caption is not its caption.
        (
            vec![
                figure(a, "a", false, vec![]),
                with_text(BlockKind::Paragraph, vec![Run::marked("Aside", [Mark::Italic])]),
                plain("b"),
            ],
            format!("![a]({a})\n\n<i>Aside</i>\n\nb\n"),
        ),
    ];
    for (blocks, expected) in cases {
        assert_eq!(to_markdown(&doc(blocks)), expected);
    }
}

/// A link preview with what a page said of itself.
fn preview(url: &str, title: Option<&str>, description: Option<&str>) -> Block {
    let (title, description) = (title.map(String::from), description.map(String::from));
    Block::new(
        "",
        BlockKind::LinkPreview { url: url.into(), title, description, image: Some("assets/5e2f.jpg".into()) },
    )
}

#[test]
fn reads_the_line_that_stands_for_a_table_of_contents() {
    let contents: &[&str] = &["contents:4"];
    let cases: &[(&str, &[&str])] = &[
        ("[TOC]", contents),
        ("[toc]", contents),
        ("[[_TOC_]]", contents),
        ("# Guide\n\n[TOC]\n\nText", &[r#"h1 "Guide""#, "contents:4", r#"p "Text""#]),
        ("> [TOC]", &["quote", "  contents:4"]),
        // Only a line of its own is one, and only as written.
        ("\\[TOC\\]", &[r#"p "[TOC]""#]),
        ("See [TOC] above", &[r#"p "See [TOC] above""#]),
        ("[TOC]\n\n[TOC]: https://example.com", &[r#"p "<@https://example.com|TOC>""#]),
    ];
    for (markdown, expected) in cases {
        let (doc, notes) = read(markdown);
        assert_eq!(outline(&doc), *expected, "reading {markdown:?}");
        assert_eq!(notes, [], "reading {markdown:?}");
    }
}

#[test]
fn writes_a_table_of_contents_as_links_to_its_headings() {
    let heading = |level, text: &str| with_text(BlockKind::Heading { level }, vec![Run::plain(text)]);
    let contents = |levels| Block::new("", BlockKind::TableOfContents { levels });
    let written = |blocks: Vec<Block>| to_markdown(&doc(blocks));
    let headings = || {
        vec![
            heading(2, "Getting started"),
            with_text(BlockKind::Heading { level: 3 }, vec![Run::plain("What's "), Run::marked("new", [Mark::Bold])]),
            heading(4, "Too deep to list"),
            heading(2, "Getting started"),
        ]
    };

    let markdown = written([vec![heading(1, "Guide"), contents(3)], headings()].concat());
    let list: Vec<&str> = markdown.lines().skip_while(|line| !line.starts_with('-')).take(4).collect();
    // Each line leads to the anchor a renderer gives its heading, and a second heading of one text is told apart.
    let expected = [
        "- [Getting started](#getting-started)",
        "  - [What's **new**](#whats-new)",
        "- [Getting started](#getting-started-1)",
        "",
    ];
    assert_eq!(list, expected, "{markdown}");

    // A table lists what follows it, so one at the end of a document lists it all but the title.
    let last = written([vec![heading(1, "Guide")], headings(), vec![contents(2)]].concat());
    assert!(
        last.ends_with("- [Getting started](#getting-started)\n- [Getting started](#getting-started-1)\n"),
        "{last}"
    );

    // With no heading to list, it is not there.
    assert_eq!(written(vec![heading(1, "Guide"), contents(3)]), "# Guide\n");
}

#[test]
fn writes_a_link_preview_as_its_linked_title_and_description() {
    let cases = [
        (
            preview("https://example.com/fees", Some("Harbour fees"), Some("What a night costs.")),
            "[Harbour fees](https://example.com/fees)\\\nWhat a night costs.\n",
        ),
        (
            preview("https://example.com/fees", Some("Harbour [fees]"), None),
            "[Harbour \\[fees\\]](https://example.com/fees)\n",
        ),
        // One that has not been fetched is its address.
        (preview("https://example.com/fees", None, None), "<https://example.com/fees>\n"),
    ];
    for (block, expected) in cases {
        assert_eq!(to_markdown(&doc(vec![block.clone()])), expected, "writing {:?}", block.kind);
    }
}

#[test]
fn a_links_title_is_the_title_of_its_pages_preview() {
    let (fees, tides) = ("https://example.com/fees", "https://example.com/tides");
    let link =
        |text: &str, address: &str| with_text(BlockKind::Paragraph, vec![Run::plain("See "), linked(text, address)]);
    let cases = [
        (
            vec![link("the fees", fees)],
            previews(&[(fees, "Harbour fees")]),
            "See [the fees](https://example.com/fees \"Harbour fees\")\n",
        ),
        // The title is written as its quotes hold it, and a link that is its own address has a place for one too.
        (
            vec![link(fees, fees)],
            previews(&[(fees, "A \"night\" | $4 `each`")]),
            "See [https://example.com/fees](https://example.com/fees \"A \\\"night\\\" \\| \\$4 \\`each\\`\")\n",
        ),
        // Each link to a page says its title, and a link to another page says none.
        (
            vec![link("fees", fees), link("tides", tides), link("fees again", fees)],
            previews(&[(fees, "Harbour fees")]),
            "See [fees](https://example.com/fees \"Harbour fees\")\n\nSee [tides](https://example.com/tides)\n\n\
             See [fees again](https://example.com/fees \"Harbour fees\")\n",
        ),
    ];
    for (blocks, previews, markdown) in cases {
        let before = Document { previews, ..doc(blocks) };
        assert_eq!(to_markdown(&before), markdown);
        let (after, notes) = read(markdown);
        assert_eq!(after.previews, before.previews, "reading {markdown}");
        assert_eq!(notes, [], "reading {markdown}");
    }

    // What else a preview holds has no place in Markdown, and one with no title leaves its link as it is.
    let told = Preview {
        description: Some("What a night costs.".into()),
        image: Some("assets/5e2f.jpg".into()),
        ..Preview::default()
    };
    let before = Document { previews: [(fees.to_owned(), told)].into(), ..doc(vec![link("fees", fees)]) };
    assert_eq!(to_markdown(&before), "See [fees](https://example.com/fees)\n");

    // An address has one preview, so a second title for it is noted and dropped.
    let (twice, notes) = read("[a](/fees \"One\") and [b](/fees \"Two\") and [c](/fees 'One')");
    assert_eq!(twice.previews, previews(&[("/fees", "One")]));
    assert_eq!(notes.iter().map(|note| note.message.as_str()).collect::<Vec<_>>(), ["Link title dropped: Two"]);
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
            holding(BlockKind::callout(Tone::Tip), vec![formula]),
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
            assert_eq!(doc.blocks[0].kind, BlockKind::callout(tone), "reading {markdown:?}");
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
fn a_callout_title_follows_its_tag() {
    // Each case: the Markdown, the title read from it (`None` is the tone's own), and the first block's text.
    let cases = [
        ("> [!info] Heads up\n> Body", Some("Heads up"), "Body"),
        ("> [!WARNING] 3 * 4 [x]\n> Body", Some("3 * 4 [x]"), "Body"),
        ("> [!NOTE] Info\n> Body", None, "Body"),
        ("> [!NOTE]\n> Body", None, "Body"),
        // Spaces alone are no title, however they are written.
        ("> [!NOTE] &#32;\n> Body", None, "Body"),
        // A title that is more than plain text stays a line of the text.
        ("> [!NOTE] Heads **up**\n> Body", None, "Heads up\nBody"),
    ];
    for (markdown, title, text) in cases {
        let (doc, _) = read(markdown);
        let BlockKind::Callout { title: found, .. } = &doc.blocks[0].kind else { panic!("{markdown:?} is a callout") };
        assert_eq!(found.as_deref(), title, "reading {markdown:?}");
        assert_eq!(doc.blocks[0].body.blocks()[0].plain_text(), text, "reading {markdown:?}");
        // What was written reads back as the same callout.
        let (again, _) = read(&to_markdown(&doc));
        assert_eq!(again.blocks[0].kind, doc.blocks[0].kind, "writing {markdown:?}");
    }
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
        ("[a](/b \"Fees\") or [a](/b \"Hover\")", "Hover", "a"),
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
        let markdown = to_markdown(&doc(vec![holding(BlockKind::callout(tone), vec![body])]));
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
        "See Figure 1.\n\n![A cat](https://example.com/cat.png)\n\n*Figure 1: A **fine** cat.*\n",
        "![a](https://example.com/a.png)\n![b](<https://example.com/b c.png>)\n\n*Both.*\n\n*Not a caption.*\n",
        "> ![a](https://example.com/a.png)\n>\n> <i>An aside.</i>\n",
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
        (include_str!("../../letters-format/corpus/document/guide.json"), include_str!("corpus/guide.md")),
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
        1 => any_figure(prop_oneof![2 => any_runs(), 1 => Just(Vec::new())]),
        1 => any_gallery(),
    ];
    let block = leaf.prop_recursive(3, 16, 4, move |inner| {
        let children = proptest::collection::vec(inner, 0..4);
        prop_oneof![
            children.clone().prop_map(|blocks| holding(BlockKind::Quote, blocks)),
            (tone.clone(), children).prop_map(|(tone, blocks)| holding(BlockKind::callout(tone), blocks)),
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

/// A figure whose picture is still remote, which is what Markdown alone can name.
fn any_figure(caption: impl Strategy<Value = Vec<Run>>) -> impl Strategy<Value = Block> {
    let src = proptest::sample::select(vec!["https://example.com/a.png", "https://example.com/b (c).png?d=1&amp;e"]);
    (src, any_text(), any::<bool>(), caption)
        .prop_map(|(src, alt, numbered, caption)| figure(src, &alt, numbered, caption))
}

/// A gallery as Markdown can say one: several pictures with no captions of their own, then at most one caption.
fn any_gallery() -> impl Strategy<Value = Block> {
    let caption = any_runs()
        .prop_filter("a caption shows", |runs| runs.iter().any(|run| !run.text.trim().is_empty()))
        .prop_map(|runs| with_text(BlockKind::Caption, runs));
    let figures = proptest::collection::vec(any_figure(Just(Vec::new())), 2..4);
    (figures, proptest::option::of(caption), any::<bool>()).prop_map(|(mut parts, caption, numbered)| {
        for part in &mut parts {
            if let BlockKind::Figure { numbered, .. } = &mut part.kind {
                *numbered = false;
            }
        }
        parts.extend(caption);
        holding(BlockKind::Gallery { numbered }, parts)
    })
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

/// Previews of the pages that links lead to, as Markdown can say them: by their titles.
fn any_previews() -> impl Strategy<Value = BTreeMap<String, Preview>> {
    let addresses =
        vec!["https://example.com/a?b=1&c=2", "/relative path", "x(y)&amp;z\\", "<q>", "/pay?in=$", "/none"];
    let title = any_text()
        .prop_map(|title| title.replace('\n', " "))
        .prop_filter("a title says something", |title| !title.is_empty());
    proptest::collection::btree_map(proptest::sample::select(addresses).prop_map(str::to_owned), title, 0..3).prop_map(
        |titles| {
            titles
                .into_iter()
                .map(|(address, title)| (address, Preview { title: Some(title), ..Preview::default() }))
                .collect()
        },
    )
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
            "[[w]]", "![i](s)", "![i](https://e.com/s.png)\n", "*Figure 1: c*\n", "Figure 1", "[l](u \"t\")", "<https://e.com>", "&#32;", "&amp;", "\\\n", "  \n", "+++\n", "\r\n",
        ])
        .prop_map(str::to_owned),
    ];
    proptest::collection::vec(piece, 0..24).prop_map(|pieces| pieces.concat())
}

proptest! {
    #[test]
    fn a_document_survives_a_round_trip(
        blocks in any_blocks(),
        properties in any_properties(),
        previews in any_previews(),
    ) {
        let before = Document { properties, previews, ..doc(blocks) };
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

    /// What Markdown has no word for reads back as what it was written as, and stays that: a table of
    /// contents its list, a link preview its paragraph.
    #[test]
    fn what_is_written_as_something_else_settles_at_once(
        blocks in any_blocks(),
        levels in 1..=4u8,
        title in proptest::option::of(any_text()),
        description in proptest::option::of(any_text()),
    ) {
        let title = title.map(|title| title.replace('\n', " ")).filter(|title| !title.is_empty());
        let card = preview("https://example.com/a (b)?c=1&amp;d", title.as_deref(), description.as_deref().filter(|text| !text.is_empty()));
        let table = Block::new("", BlockKind::TableOfContents { levels });
        let once = to_markdown(&doc([vec![table], blocks, vec![card]].concat()));
        let twice = to_markdown(&read(&once).0);
        prop_assert_eq!(twice, once);
    }

    /// A table of contents read from a file lists whatever headings the file has, and its list settles too.
    #[test]
    fn the_list_of_an_imported_table_of_contents_settles_at_once(before in any_markdown(), after in any_markdown()) {
        let (imported, _) = read(&format!("{before}\n\n[TOC]\n\n{after}"));
        let once = to_markdown(&imported);
        let twice = to_markdown(&read(&once).0);
        prop_assert_eq!(twice, once);
    }
}
