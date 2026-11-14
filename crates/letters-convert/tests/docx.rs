//! Word documents, through the two public functions: reading what Word and other tools write, and writing
//! what Word reads back.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use letters_convert::{DocxError, DocxImport, from_docx, from_markdown, to_docx};
use letters_model::{
    Align, Anchor, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Note, NoteKind, Notes, Reply, Run, Style,
    Tone,
};
use proptest::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use common::outline;

/// The time an import is told it happens at.
const NOW: &str = "2026-01-02T03:04:05Z";

/// The start of a PNG, which is all that tells a picture's format.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-pixels-";

/// Every namespace the cases write elements in.
const NAMESPACES: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" "#,
    r#"xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" "#,
    r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" "#,
    r#"xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
    r#"xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" "#,
    r#"xmlns:v="urn:schemas-microsoft-com:vml" "#,
    r#"xmlns:o="urn:schemas-microsoft-com:office:office""#,
);

const RELATIONSHIP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// Styles as Word writes them: named in English whatever their ids, each saying only how it differs from the
/// one it is based on.
const STYLES: &str = r#"
<w:docDefaults>
  <w:rPrDefault><w:rPr>
    <w:rFonts w:ascii="Inter" w:hAnsi="Inter"/><w:sz w:val="22"/><w:lang w:val="en-GB"/>
  </w:rPr></w:rPrDefault>
  <w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="264" w:lineRule="auto"/></w:pPr></w:pPrDefault>
</w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Std"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="T"><w:name w:val="Title"/><w:basedOn w:val="Std"/>
  <w:pPr><w:jc w:val="center"/></w:pPr><w:rPr><w:sz w:val="56"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="H1"><w:name w:val="heading 1"/><w:basedOn w:val="Std"/>
  <w:pPr><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="0"/></w:pPr>
  <w:rPr><w:rFonts w:ascii="Montserrat" w:hAnsi="Montserrat"/><w:b/><w:color w:val="2F5496"/><w:sz w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="H2"><w:name w:val="heading 2"/><w:basedOn w:val="Std"/>
  <w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="H6"><w:name w:val="heading 6"/><w:basedOn w:val="Std"/></w:style>
<w:style w:type="paragraph" w:styleId="Section"><w:name w:val="Section"/><w:basedOn w:val="H2"/></w:style>
<w:style w:type="paragraph" w:styleId="Outlined"><w:name w:val="Outlined"/><w:basedOn w:val="Std"/>
  <w:pPr><w:outlineLvl w:val="2"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Q"><w:name w:val="Quote"/><w:basedOn w:val="Std"/><w:rPr><w:i/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Src"><w:name w:val="Source Code"/><w:basedOn w:val="Std"/>
  <w:rPr><w:rFonts w:ascii="Fira Code" w:hAnsi="Fira Code"/><w:sz w:val="20"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Cap"><w:name w:val="caption"/><w:basedOn w:val="Std"/></w:style>
<w:style w:type="paragraph" w:styleId="Stressed"><w:name w:val="Stressed"/><w:basedOn w:val="Std"/>
  <w:rPr><w:i/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Bullet2"><w:name w:val="List Bullet 2"/><w:basedOn w:val="Std"/>
  <w:pPr><w:numPr><w:numId w:val="1"/></w:numPr></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Warn"><w:name w:val="Callout Warn"/><w:basedOn w:val="Std"/></w:style>
<w:style w:type="paragraph" w:styleId="Toc1"><w:name w:val="toc 1"/><w:basedOn w:val="Std"/></w:style>
<w:style w:type="character" w:styleId="Mono"><w:name w:val="Verbatim Char"/>
  <w:rPr><w:rFonts w:ascii="Fira Code" w:hAnsi="Fira Code"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Link"><w:name w:val="Hyperlink"/>
  <w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>
"#;

/// Lists by id: 1 bullets, 2 numbers, 3 numbers from 5, 4 open tasks, 5 ticked tasks, 6 an indent with no marker.
const NUMBERING: &str = concat!(
    r#"<w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="•"/></w:lvl>"#,
    r#"<w:lvl w:ilvl="1"><w:numFmt w:val="bullet"/><w:lvlText w:val="o"/></w:lvl>"#,
    r#"<w:lvl w:ilvl="2"><w:numFmt w:val="bullet"/><w:lvlText w:val="▪"/></w:lvl></w:abstractNum>"#,
    r#"<w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/></w:lvl>"#,
    r#"<w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/></w:lvl></w:abstractNum>"#,
    r#"<w:abstractNum w:abstractNumId="2"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="☐"/></w:lvl>"#,
    r#"</w:abstractNum>"#,
    r#"<w:abstractNum w:abstractNumId="3"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/><w:lvlText w:val="☒"/></w:lvl>"#,
    r#"</w:abstractNum>"#,
    r#"<w:abstractNum w:abstractNumId="4"><w:lvl w:ilvl="0"><w:numFmt w:val="none"/><w:lvlText w:val=""/></w:lvl>"#,
    r#"</w:abstractNum>"#,
    r#"<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>"#,
    r#"<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>"#,
    r#"<w:num w:numId="3"><w:abstractNumId w:val="1"/>"#,
    r#"<w:lvlOverride w:ilvl="0"><w:startOverride w:val="5"/></w:lvlOverride></w:num>"#,
    r#"<w:num w:numId="4"><w:abstractNumId w:val="2"/></w:num>"#,
    r#"<w:num w:numId="5"><w:abstractNumId w:val="3"/></w:num>"#,
    r#"<w:num w:numId="6"><w:abstractNumId w:val="4"/></w:num>"#,
);

/// A zip of these entries.
fn zipped(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).expect("an entry starts");
        zip.write_all(bytes).expect("an entry is written");
    }
    zip.finish().expect("the zip ends").into_inner()
}

/// The entries of a zip by name.
fn unzipped(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).expect("the file is a zip");
    let count = zip.len();
    let entry = |index: usize| {
        let mut file = zip.by_index(index).expect("an entry opens");
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).expect("an entry reads");
        (file.name().expect("an entry has a name").into_owned(), bytes)
    };
    (0..count).map(entry).collect()
}

/// A Word file made by hand: its body, and whichever parts beside it a case needs.
#[derive(Clone, Default)]
struct Word {
    body: String,
    /// Parts by the kind of relationship that names each, as what sits inside its root element.
    parts: Vec<(&'static str, String)>,
    /// Addresses outside the file, by the id of their relationship.
    links: Vec<(String, String)>,
    /// Pictures, by the id of their relationship.
    pictures: Vec<(String, Vec<u8>)>,
}

/// A file with this body and nothing beside it.
fn bare(body: &str) -> Word {
    Word { body: body.into(), ..Word::default() }
}

/// A file with this body and the styles and lists the cases share.
fn word(body: &str) -> Word {
    bare(body).part("styles", STYLES).part("numbering", NUMBERING)
}

impl Word {
    fn part(mut self, kind: &'static str, inner: &str) -> Self {
        self.parts.push((kind, inner.into()));
        self
    }

    fn link(mut self, id: &str, url: &str) -> Self {
        self.links.push((id.into(), url.into()));
        self
    }

    fn picture(mut self, id: &str, bytes: &[u8]) -> Self {
        self.pictures.push((id.into(), bytes.into()));
        self
    }

    fn bytes(&self) -> Vec<u8> {
        let rooted = |root: &str, inner: &str| format!("<{root} {NAMESPACES}>{inner}</{root}>").into_bytes();
        let relationships = |inner: &str| {
            let package = "http://schemas.openxmlformats.org/package/2006/relationships";
            format!(r#"<Relationships xmlns="{package}">{inner}</Relationships>"#).into_bytes()
        };
        let relationship = |id: &str, kind: &str, target: &str, mode: &str| {
            format!(r#"<Relationship Id="{id}" Type="{RELATIONSHIP}/{kind}" Target="{target}"{mode}/>"#)
        };
        let mut rels = String::new();
        let mut entries = vec![
            ("_rels/.rels".to_owned(), relationships(&relationship("rId1", "officeDocument", "word/document.xml", ""))),
            ("word/document.xml".to_owned(), rooted("w:document", &format!("<w:body>{}</w:body>", self.body))),
        ];
        for (index, (kind, inner)) in self.parts.iter().enumerate() {
            let root = match *kind {
                "commentsExtended" => "w15:commentsEx",
                "header" => "w:hdr",
                "footer" => "w:ftr",
                kind => &format!("w:{kind}"),
            };
            rels.push_str(&relationship(&format!("part{index}"), kind, &format!("{kind}{index}.xml"), ""));
            entries.push((format!("word/{kind}{index}.xml"), rooted(root, inner)));
        }
        for (id, url) in &self.links {
            rels.push_str(&relationship(id, "hyperlink", url, r#" TargetMode="External""#));
        }
        for (id, bytes) in &self.pictures {
            rels.push_str(&relationship(id, "image", &format!("media/{id}"), ""));
            entries.push((format!("word/media/{id}"), bytes.clone()));
        }
        entries.push(("word/_rels/document.xml.rels".to_owned(), relationships(&rels)));
        let entries: Vec<(&str, &[u8])> =
            entries.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())).collect();
        zipped(&entries)
    }
}

fn import(bytes: &[u8], base: &Style) -> Result<DocxImport, DocxError> {
    let mut next = 0;
    from_docx(bytes, base, NOW, &mut || {
        next += 1;
        format!("b{next}")
    })
}

fn read(file: &Word) -> DocxImport {
    import(&file.bytes(), &Style::default()).expect("the file reads")
}

/// A run of plain text.
fn t(text: &str) -> String {
    format!(r#"<w:r><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

/// A run with properties.
fn run(props: &str, text: &str) -> String {
    format!(r#"<w:r><w:rPr>{props}</w:rPr><w:t xml:space="preserve">{text}</w:t></w:r>"#)
}

/// A paragraph with properties around what it holds.
fn para(props: &str, inner: &str) -> String {
    format!("<w:p><w:pPr>{props}</w:pPr>{inner}</w:p>")
}

/// A paragraph of plain text.
fn p(text: &str) -> String {
    format!("<w:p>{}</w:p>", t(text))
}

/// A paragraph of a style.
fn styled(style: &str, text: &str) -> String {
    para(&format!(r#"<w:pStyle w:val="{style}"/>"#), &t(text))
}

/// An item of a list, at a depth.
fn item(list: u32, depth: u8, text: &str) -> String {
    para(&format!(r#"<w:numPr><w:ilvl w:val="{depth}"/><w:numId w:val="{list}"/></w:numPr>"#), &t(text))
}

/// A run that holds a picture, 100 points by 50.
fn picture(source: &str, alt: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline><wp:extent cx="1270000" cy="635000"/><wp:docPr id="1" name="P" descr="{alt}"/>
        <a:graphic><a:graphicData><pic:pic><pic:blipFill><a:blip {source}/></pic:blipFill></pic:pic></a:graphicData>
        </a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

/// The text of the one block a file reads as, as `outline` writes runs.
fn line(file: &Word) -> String {
    let lines = outline(&read(file).document);
    assert_eq!(lines.len(), 1, "{:?} reads as one block, got {lines:?}", file.body);
    lines[0].split_once(' ').map(|(_, text)| text.trim_matches('"').to_owned()).unwrap_or_default()
}

/// What the import noted on the block whose text holds `text`.
fn noted<'a>(import: &'a DocxImport, text: &str) -> Vec<&'a str> {
    let holds = |block: &&Block| block.plain_text().contains(text);
    let block = import.document.walk().find(holds).unwrap_or_else(|| panic!("no block holds {text:?}"));
    import.notes.iter().filter(|note| note.block == block.id.as_str()).map(|note| note.message.as_str()).collect()
}

/// The text a note sits on.
fn anchored(doc: &Document, anchor: &Anchor) -> String {
    let text = doc.block(&anchor.block).expect("the anchor names a block").plain_text();
    anchor.range.clone().map_or(text.clone(), |range| text[range].to_owned())
}

fn said(runs: &[Run]) -> String {
    runs.iter().map(|run| run.text.as_str()).collect()
}

/// The pictures of a document: what each shows, its alternative text and its caption.
///
/// The model has no figure yet, so one is an unknown block with its fields beside it.
fn pictures(doc: &Document) -> Vec<(String, String, String)> {
    let figure = |block: &Block| {
        let field = |name: &str| block.extra.get(name).and_then(|value| value.as_str()).map(str::to_owned);
        let caption =
            block.extra.get("content").cloned().map(|runs| Run::list_from_value(runs, "content").expect("runs"));
        let shown = field("asset").or_else(|| field("src"))?;
        Some((shown, field("alt").unwrap_or_default(), said(&caption.unwrap_or_default())))
    };
    doc.walk()
        .filter(|block| matches!(&block.kind, BlockKind::Unknown { type_name } if type_name == "figure"))
        .filter_map(figure)
        .collect()
}

#[test]
fn reads_paragraph_styles_as_blocks() {
    let ruled = r#"<w:pBdr><w:bottom w:val="single" w:sz="6" w:space="1" w:color="auto"/></w:pBdr>"#;
    let cases: &[(String, &[&str])] = &[
        (p("One") + &p("Two"), &[r#"p "One""#, r#"p "Two""#]),
        (styled("H1", "One") + &styled("H2", "Two"), &[r#"h1 "One""#, r#"h2 "Two""#]),
        (styled("T", "Report"), &[r#"h1 "Report""#]),
        (styled("H6", "Deep"), &[r#"h4 "Deep""#]),
        // A style is what the style it is based on is, and one Word files under a heading level is a heading.
        (styled("Section", "Based"), &[r#"h2 "Based""#]),
        (styled("Outlined", "Filed"), &[r#"h3 "Filed""#]),
        (para(r#"<w:outlineLvl w:val="1"/>"#, &t("Direct")), &[r#"h2 "Direct""#]),
        (
            styled("Q", "said") + &styled("Q", "twice") + &p("after"),
            &["quote", r#"  p "said""#, r#"  p "twice""#, r#"p "after""#],
        ),
        (styled("Warn", "Hot") + &item(1, 0, "plain"), &["callout:warn", r#"  p "Hot""#, r#"bullet/0 "plain""#]),
        // Word holds each line of code as a paragraph.
        (
            styled("Src", "fn main() {") + &styled("Src", "") + &styled("Src", "}") + &p("after"),
            &[r#"code:- "fn main() {\n\n}""#, r#"p "after""#],
        ),
        (p("above") + &para(ruled, "") + &p("below"), &[r#"p "above""#, "divider", r#"p "below""#]),
        (r#"<w:p><w:r><w:pict><v:rect o:hr="t"/></w:pict></w:r></w:p>"#.to_owned(), &["divider"]),
        // A line can be a shape that is drawn where a rule would be.
        (
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="5943600" cy="19050"/></wp:inline></w:drawing></w:r></w:p>"#
                .to_owned(),
            &["divider"],
        ),
        // Empty paragraphs are Word's way of spacing text.
        (p("one") + "<w:p/>" + &p("  ") + &p("two"), &[r#"p "one""#, r#"p "two""#]),
        (
            "<w:p><w:r><w:t>one</w:t><w:br/><w:t>two</w:t><w:tab/><w:t>three</w:t></w:r></w:p>".to_owned(),
            &[r#"p "one\ntwo\tthree""#],
        ),
        (r#"<w:p><w:r><w:br w:type="page"/><w:t>next</w:t></w:r></w:p>"#.to_owned(), &[r#"p "next""#]),
        (String::new(), &[]),
    ];
    for (body, expected) in cases {
        assert_eq!(outline(&read(&word(body)).document), *expected, "reading {body}");
    }
}

#[test]
fn reads_a_file_that_has_only_a_document() {
    let file = zipped(&[(
        "word/document.xml",
        format!("<w:document {NAMESPACES}><w:body>{}</w:body></w:document>", p("Alone")).as_bytes(),
    )]);
    let read = import(&file, &Style::default()).expect("the file reads");
    assert_eq!(outline(&read.document), [r#"p "Alone""#]);
    assert_eq!(read.style, Style::default(), "a file without styles keeps the style it was given");
}

#[test]
fn reads_formatting_as_marks() {
    let cases = [
        (run("<w:b/>", "x"), "<bold|x>"),
        (run("<w:i/>", "x"), "<italic|x>"),
        (run(r#"<w:u w:val="single"/>"#, "x"), "<underline|x>"),
        (run("<w:strike/>", "x") + &run("<w:dstrike/>", "y"), "<strike|xy>"),
        (run(r#"<w:highlight w:val="yellow"/>"#, "x"), "<highlight|x>"),
        (run(r#"<w:shd w:val="clear" w:color="auto" w:fill="FFFF00"/>"#, "x"), "<highlight|x>"),
        (run(r#"<w:rStyle w:val="Mono"/>"#, "x"), "<code|x>"),
        (run(r#"<w:rFonts w:ascii="Courier New" w:hAnsi="Courier New"/>"#, "x"), "<code|x>"),
        (run(r#"<w:rStyle w:val="Strong"/>"#, "x"), "<bold|x>"),
        (run("<w:b/><w:i/>", "x"), "<bold,italic|x>"),
        (t("a ") + &run("<w:b/>", "b") + &run("<w:b/>", "c") + &t(" d"), "a <bold|bc> d"),
        // A property can be there to switch itself off.
        (run(r#"<w:b w:val="0"/><w:i w:val="false"/><w:u w:val="none"/>"#, "x"), "x"),
    ];
    for (runs, expected) in cases {
        assert_eq!(line(&word(&format!("<w:p>{runs}</w:p>"))), expected, "reading {runs}");
    }
}

#[test]
fn a_style_shows_as_marks_only_where_it_differs_from_its_block() {
    let cases = [
        // The heading's style is bold and the quote's is italic: that is how the block looks, and no mark.
        (styled("H1", "x"), r#"h1 "x""#),
        (styled("Q", "x"), r#"  p "x""#),
        // A paragraph style of body text that slants it has nowhere else to say so.
        (styled("Stressed", "x"), r#"p "<italic|x>""#),
        // Emphasis a run asks for itself is kept, even where the style is as heavy.
        (para(r#"<w:pStyle w:val="H1"/>"#, &(t("a ") + &run("<w:b/>", "b"))), r#"h1 "a <bold|b>""#),
        (para(r#"<w:pStyle w:val="H1"/>"#, &run(r#"<w:b w:val="0"/><w:i/>"#, "x")), r#"h1 "<italic|x>""#),
    ];
    for (body, expected) in cases {
        let lines = outline(&read(&word(&body)).document);
        assert_eq!(lines.last().map(String::as_str), Some(expected), "reading {body}");
    }
}

#[test]
fn reads_lists() {
    let checkbox = |state: &str, text: &str| {
        format!(
            r#"<w:p><w:sdt><w:sdtPr><w14:checkbox><w14:checked w14:val="{state}"/></w14:checkbox></w:sdtPr>
            <w:sdtContent><w:r><w:t>☐</w:t></w:r></w:sdtContent></w:sdt>{}</w:p>"#,
            t(text)
        )
    };
    let cases: &[(String, &[&str])] = &[
        (
            item(1, 0, "a") + &item(1, 1, "b") + &item(1, 2, "c") + &item(1, 0, "d"),
            &[r#"bullet/0 "a""#, r#"bullet/1 "b""#, r#"bullet/2 "c""#, r#"bullet/0 "d""#],
        ),
        (
            item(2, 0, "a") + &item(2, 1, "b") + &item(2, 0, "c"),
            &[r#"ordered/0 "a""#, r#"ordered/1 "b""#, r#"ordered/0 "c""#],
        ),
        (item(2, 0, "a") + &item(1, 1, "b"), &[r#"ordered/0 "a""#, r#"bullet/1 "b""#]),
        // Tasks, as a list whose marker is a box, as a bullet whose text opens with one, and as Word's checkbox.
        (item(4, 0, "open") + &item(5, 0, "done"), &[r#"todo/0 "open""#, r#"done/0 "done""#]),
        (
            item(1, 0, "☐ open") + &item(1, 0, "☑ done") + &item(1, 0, "☒ crossed"),
            &[r#"todo/0 "open""#, r#"done/0 "done""#, r#"done/0 "crossed""#],
        ),
        (checkbox("0", " open") + &checkbox("1", " done"), &[r#"todo/0 "open""#, r#"done/0 "done""#]),
        // `List Bullet 2` is a second level by its name, though it is the first level of its own list.
        (item(1, 0, "a") + &styled("Bullet2", "b"), &[r#"bullet/0 "a""#, r#"bullet/1 "b""#]),
        // An item nests one level under the one before it, however far in Word set it.
        (item(1, 2, "deep") + &item(1, 2, "too"), &[r#"bullet/0 "deep""#, r#"bullet/0 "too""#]),
        (
            item(1, 0, "a") + &item(1, 2, "b") + &item(1, 2, "c"),
            &[r#"bullet/0 "a""#, r#"bullet/1 "b""#, r#"bullet/1 "c""#],
        ),
        // List 0 takes a paragraph out of its style's list, and a level without a marker is no list.
        (para(r#"<w:pStyle w:val="Bullet2"/><w:numPr><w:numId w:val="0"/></w:numPr>"#, &t("out")), &[r#"p "out""#]),
        (item(6, 0, "indented"), &[r#"p "indented""#]),
        // Word numbers a list it has no definition for.
        (item(99, 0, "lost"), &[r#"ordered/0 "lost""#]),
    ];
    for (body, expected) in cases {
        assert_eq!(outline(&read(&word(body)).document), *expected, "reading {body}");
    }
}

#[test]
fn notes_numbers_the_page_will_not_show() {
    // The body, the item to look at, and the number Word gave it where the page starts again at 1.
    let cases = [
        (item(2, 0, "one") + &item(2, 0, "two") + &item(2, 0, "three"), "one", None),
        (item(3, 0, "five") + &item(3, 0, "six"), "five", Some("5")),
        (item(3, 0, "five") + &item(3, 0, "six"), "six", None),
        (item(2, 0, "one") + &p("between") + &item(2, 0, "two"), "two", Some("2")),
        (item(2, 0, "one") + &item(2, 1, "inner") + &item(2, 0, "two") + &item(2, 1, "again"), "again", None),
    ];
    for (body, text, number) in cases {
        let import = read(&word(&body));
        let notes = noted(&import, text);
        match number {
            Some(number) => assert!(notes.iter().any(|note| note.contains(number)), "{text:?} in {body}: {notes:?}"),
            None => assert!(notes.is_empty(), "{text:?} in {body}: {notes:?}"),
        }
    }
}

#[test]
fn reads_links() {
    let field = |code: &str, shown: &str| {
        format!(
            r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve">{code}</w:instrText></w:r>
            <w:r><w:fldChar w:fldCharType="separate"/></w:r>{shown}<w:r><w:fldChar w:fldCharType="end"/></w:r>"#
        )
    };
    let look = r#"<w:rStyle w:val="Link"/>"#;
    let cases = [
        (
            format!(r#"<w:hyperlink r:id="site">{}</w:hyperlink>"#, run(look, "here")),
            "<@https://example.com/a?b=1&c=2|here>",
        ),
        (
            format!(r#"<w:hyperlink r:id="site" w:anchor="part">{}</w:hyperlink>"#, t("here")),
            "<@https://example.com/a?b=1&c=2#part|here>",
        ),
        (
            format!(r#"<w:hyperlink r:id="site">{}{}</w:hyperlink>"#, t("a "), run("<w:b/>", "b")),
            "<@https://example.com/a?b=1&c=2|a ><bold,@https://example.com/a?b=1&c=2|b>",
        ),
        (field(r#" HYPERLINK "https://example.com/f" "#, &run(look, "field")), "<@https://example.com/f|field>"),
        (
            field(r#" HYPERLINK "https://example.com/f" \l "part" \o "tip" "#, &t("field")),
            "<@https://example.com/f#part|field>",
        ),
        (
            format!(
                r#"<w:fldSimple w:instr=" HYPERLINK &quot;https://example.com/s&quot; ">{}</w:fldSimple>"#,
                t("simple")
            ),
            "<@https://example.com/s|simple>",
        ),
        // A field that is no link shows what it last came to.
        (field(" PAGE ", &t("7")), "7"),
        (format!(r#"<w:hyperlink r:id="nowhere">{}</w:hyperlink>"#, t("dangling")), "dangling"),
    ];
    for (runs, expected) in cases {
        let file = word(&format!("<w:p>{runs}</w:p>")).link("site", "https://example.com/a?b=1&amp;c=2");
        let import = read(&file);
        assert_eq!(line(&file), expected, "reading {runs}");
        assert!(import.notes.is_empty(), "a link's own look is no loss, reading {runs}: {:?}", import.notes);
    }
}

#[test]
fn keeps_the_text_of_what_it_simplifies_and_says_so() {
    let boxed = format!(
        r#"<w:p>{}<w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor><a:graphic><a:graphicData>
        <wps:wsp><wps:txbx><w:txbxContent>{}</w:txbxContent></wps:txbx></wps:wsp></a:graphicData></a:graphic></wp:anchor>
        </w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape><v:textbox><w:txbxContent>{}</w:txbxContent></v:textbox>
        </v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>"#,
        t("Beside"),
        p("In a box"),
        p("In a box"),
    );
    let table = format!(
        "<w:tbl><w:tr><w:tc>{}</w:tc><w:tc>{}</w:tc></w:tr><w:tr><w:tc>{}{}</w:tc><w:tc><w:p/></w:tc></w:tr></w:tbl>",
        p("Name"),
        p("Age"),
        p("Ada"),
        p("Lovelace"),
    );
    let chart = r#"<w:p><w:r><w:t>Chart: </w:t></w:r><w:r><w:drawing><wp:inline><a:graphic/></wp:inline></w:drawing></w:r></w:p>"#;
    // The body, the text each block must come to, and a word of the note on that block.
    let cases: &[(String, &[&str], &str)] = &[
        (
            format!(
                "<w:p>{}<w:del><w:r><w:delText>gone </w:delText></w:r></w:del><w:ins>{}</w:ins></w:p>",
                t("kept "),
                t("added")
            ),
            &["kept added"],
            "Tracked changes",
        ),
        (
            format!("<w:p>{}{}</w:p>", t("E = mc"), run(r#"<w:vertAlign w:val="superscript"/>"#, "2")),
            &["E = mc2"],
            "Superscript",
        ),
        (format!("<w:p>{}<m:oMath><m:r><m:t>x=1</m:t></m:r></m:oMath></w:p>", t("So ")), &["So x=1"], "Equation"),
        (
            r#"<w:p><w:r><w:t>Go </w:t><w:sym w:font="Wingdings" w:char="F0E0"/></w:r></w:p>"#.to_owned(),
            &["Go \u{fffd}"],
            "symbol",
        ),
        (
            format!(r#"<w:p><w:hyperlink w:anchor="part">{}</w:hyperlink></w:p>"#, t("See part 2")),
            &["See part 2"],
            "link",
        ),
        (boxed, &["Beside", "In a box"], "Text box"),
        (table, &["Name\tAge", "Ada\nLovelace\t"], "Table"),
        (chart.to_owned(), &["Chart: "], "drawing"),
        (styled("T", "Report"), &["Report"], "Title"),
        (styled("H6", "Deep"), &["Deep"], "level 6"),
    ];
    for (body, texts, note) in cases {
        let import = read(&word(body));
        let read: Vec<String> = import.document.walk().map(Block::plain_text).collect();
        assert_eq!(read, *texts, "reading {body}");
        let noted: Vec<&str> = texts.iter().flat_map(|text| noted(&import, text)).collect();
        assert!(
            noted.iter().any(|message| message.contains(note)),
            "no note of {note:?} reading {body}: {:?}",
            import.notes
        );
    }
}

#[test]
fn leaves_out_a_table_of_contents_and_says_where() {
    let entry = |text: &str| styled("Toc1", text);
    let built = format!(
        r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/></w:docPartObj></w:sdtPr>
        <w:sdtContent>{}{}</w:sdtContent></w:sdt>"#,
        entry("Introduction 1"),
        entry("Method 2")
    );
    let field = format!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> TOC \o "1-3" </w:instrText></w:r>
        <w:r><w:fldChar w:fldCharType="separate"/></w:r>{}</w:p><w:p>{}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        t("Introduction 1"),
        t("Method 2")
    );
    for contents in [built, field, entry("Introduction 1")] {
        let import = read(&word(&(p("Before") + &contents + &p("After"))));
        assert_eq!(outline(&import.document), [r#"p "Before""#, r#"p "After""#], "reading {contents}");
        assert!(
            noted(&import, "After").iter().any(|note| note.contains("contents")),
            "reading {contents}: {:?}",
            import.notes
        );
    }
}

#[test]
fn says_once_what_concerns_the_whole_document() {
    let sized = |text: &str| run(r#"<w:sz w:val="40"/><w:color w:val="FF0000"/>"#, text);
    let body = styled("H1", "Report")
        + &format!("<w:p>{}{}</w:p>", sized("Large"), sized(" and red"))
        + &para(r#"<w:jc w:val="center"/>"#, &t("Centred"))
        + &para(r#"<w:pStyle w:val="H1"/><w:numPr><w:numId w:val="2"/></w:numPr>"#, &t("Numbered"));
    let styles = STYLES.replace("Inter", "Calibri");
    let file = bare(&body)
        .part("styles", &styles)
        .part("numbering", NUMBERING)
        .part("header", &p("Confidential"))
        .part("footer", "<w:p/>");
    let import = read(&file);
    let first = noted(&import, "Report");
    for about in ["Fonts, sizes, colours", "Headers and footers", "Heading numbers", "Calibri"] {
        let told = import.notes.iter().filter(|note| note.message.contains(about)).count();
        assert_eq!(told, 1, "{about:?} is said once: {:?}", import.notes);
        assert!(first.iter().any(|note| note.contains(about)), "{about:?} is said on the first block: {first:?}");
    }

    let plain = read(&bare(&(p("One") + &p("Two"))));
    assert_eq!(plain.notes, [], "a plain file loses nothing");
}

#[test]
fn keeps_footnotes_as_text_after_the_document() {
    let cite = |kind: &str, id: u8| format!(r#"<w:r><w:{kind}Reference w:id="{id}"/></w:r>"#);
    let note = |kind: &str, id: i8, text: &str| {
        format!(r#"<w:{kind} w:id="{id}"><w:p><w:r><w:{kind}Ref/></w:r>{}</w:p></w:{kind}>"#, t(text))
    };
    let body = format!("<w:p>{}{}{}{}</w:p>", t("First"), cite("footnote", 2), t(" and second"), cite("endnote", 1))
        + &format!("<w:p>{}{}</w:p>", t("Again"), cite("footnote", 2));
    // Word keeps the rule above its notes as notes too.
    let footnotes = note("footnote", -1, "") + &note("footnote", 0, "") + &note("footnote", 2, " A source.");
    let file = word(&body).part("footnotes", &footnotes).part("endnotes", &note("endnote", 1, " An aside."));
    let import = read(&file);
    assert_eq!(
        outline(&import.document),
        [r#"p "First[^1] and second[^2]""#, r#"p "Again[^1]""#, r#"p "[^1]: A source.""#, r#"p "[^2]: An aside.""#]
    );
    assert!(noted(&import, "First").iter().any(|note| note.contains("Footnote")), "{:?}", import.notes);
}

#[test]
fn reads_comments_with_their_threads() {
    let mark = |id: u8| format!(r#"<w:r><w:commentReference w:id="{id}"/></w:r>"#);
    let body = format!(
        r#"<w:p>{}<w:commentRangeStart w:id="0"/>{}<w:commentRangeEnd w:id="0"/>{}{}{}</w:p><w:p>{}{}</w:p>"#,
        t("An "),
        run("<w:b/>", "important"),
        mark(0),
        mark(1),
        t(" point."),
        t("Second."),
        mark(2),
    );
    let comment = |id: u8, who: &str, thread: &str, text: &str| {
        format!(r#"<w:comment w:id="{id}" {who}><w:p w14:paraId="{thread}">{}</w:p></w:comment>"#, t(text))
    };
    let comments = comment(0, r#"w:author="Ada" w:date="2024-03-01T10:00:00Z""#, "0000000A", "Check this.")
        + &comment(1, r#"w:author="Grace" w:date="2024-03-02T09:30:00""#, "0000000B", "Checked.")
        + &comment(2, "", "0000000C", "Dealt with.");
    let threads = r#"<w15:commentEx w15:paraId="0000000A" w15:done="0"/>
        <w15:commentEx w15:paraId="0000000B" w15:paraIdParent="0000000A" w15:done="0"/>
        <w15:commentEx w15:paraId="0000000C" w15:done="1"/>"#;
    let import = read(&word(&body).part("comments", &comments).part("commentsExtended", threads));

    assert_eq!(import.notes, [], "a comment is not something the import simplified");
    let [first, second] = import.comments.as_slice() else { panic!("two threads, got {:?}", import.comments) };
    assert_eq!(first.kind, NoteKind::Comment);
    assert_eq!((first.author.as_str(), first.created.as_str(), first.resolved), ("Ada", "2024-03-01T10:00:00Z", false));
    assert_eq!(said(&first.content), "Check this.");
    assert_eq!(anchored(&import.document, &first.anchor), "important");
    let [reply] = first.replies.as_slice() else { panic!("one reply, got {:?}", first.replies) };
    assert_eq!((reply.author.as_str(), said(&reply.content).as_str()), ("Grace", "Checked."));
    assert!(reply.created.starts_with("2024-03-02T09:30:00"), "{:?}", reply.created);

    // Word can leave a comment without a name or a date.
    assert!(!second.author.is_empty());
    assert_eq!((second.created.as_str(), second.resolved), (NOW, true));
    assert_eq!(said(&second.content), "Dealt with.");
    assert_eq!(anchored(&import.document, &second.anchor), "Second.");
    assert_ne!(first.id, second.id);
}

#[test]
fn seeds_the_style_from_words_styles() {
    let section = r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>
        <w:pgMar w:top="1440" w:right="1134" w:bottom="1440" w:left="1134" w:header="708"/></w:sectPr>"#;
    let import = read(&word(&(p("Text") + section)));
    let style = &import.style;
    let base = Style::default();
    let of = |name: &str| &style.types[name];

    let body = of("body");
    assert_eq!((body.family.as_str(), body.size, body.space_after), ("Inter", 11.0, 8.0));
    assert!((body.line_height - 1.32).abs() < 0.01, "1.1 lines of Word's are {}", body.line_height);
    let heading = of("heading1");
    assert_eq!((heading.family.as_str(), heading.size, heading.color.as_str()), ("Montserrat", 16.0, "#2f5496"));
    assert_eq!((heading.space_before, heading.space_after), (12.0, 0.0));
    assert!(heading.weight >= 600 && of("body").weight < 600);
    assert_eq!((of("title").size, of("title").align), (28.0, Align::Center));
    assert!(of("quote").italic && !of("body").italic);
    assert_eq!((of("code").family.as_str(), of("code").size), ("Fira Code", 10.0));
    // A type Word does not define keeps its size, in the colour of the text around it.
    assert_eq!((of("heading4").color.as_str(), of("heading4").size), ("#000000", base.types["heading4"].size));

    assert!((style.page.width - 595.3).abs() < 0.1 && (style.page.height - 841.9).abs() < 0.1, "{:?}", style.page);
    assert_eq!((style.page.margin_top, style.page.margin_left), (72.0, 56.7));
    assert_eq!(import.document.language, "en-GB");
    assert_eq!(Style::from_json(&style.to_json()).as_ref(), Ok(style), "the seeded style is one a document can hold");
}

#[test]
fn reads_pictures_as_assets() {
    let embedded = r#"r:embed="png""#;
    let shown = |import: &DocxImport| pictures(&import.document);

    let captioned = read(
        &word(&(format!("<w:p>{}</w:p>", picture(embedded, "A chart")) + &styled("Cap", "Sales by year")))
            .picture("png", PNG),
    );
    let [(asset, alt, caption)] = shown(&captioned).try_into().expect("one picture");
    assert_eq!((alt.as_str(), caption.as_str()), ("A chart", "Sales by year"));
    assert!(asset.starts_with("assets/") && asset.ends_with(".png"), "{asset}");
    assert_eq!(captioned.assets.get(&asset).map(Vec::as_slice), Some(PNG));
    assert_eq!(captioned.document.walk().count(), 1, "the caption belongs to its picture");

    // A picture among words gets a block of its own, and one shown twice is kept once.
    let inline = format!("<w:p>{}{}{}</w:p>", t("Before "), picture(embedded, ""), t("after"));
    let twice = read(&word(&(inline + &format!("<w:p>{}</w:p>", picture(embedded, "")))).picture("png", PNG));
    assert_eq!(outline(&twice.document), [r#"p "Before after""#, "unknown:figure", "unknown:figure"]);
    assert_eq!(twice.assets.len(), 1);
    assert!(twice.notes.iter().any(|note| note.message.contains("Picture moved")), "{:?}", twice.notes);

    // What cannot be shown is left out and named, and the words around it stay.
    let cases = [
        (word("").picture("png", b"\x01\x00\x00\x00 an EMF"), embedded),
        (word("").link("far", "http://example.com/p.png"), r#"r:link="far""#),
        (word(""), r#"r:embed="missing""#),
    ];
    for (file, source) in cases {
        let body = format!("<w:p>{}{}</w:p>", t("Words"), picture(source, "Alt"));
        let import = read(&Word { body, ..file });
        assert_eq!((outline(&import.document), import.assets.len()), (vec![r#"p "Words""#.to_owned()], 0), "{source}");
        assert!(!noted(&import, "Words").is_empty(), "{source} is named");
    }

    let remote = read(
        &word(&format!("<w:p>{}</w:p>", picture(r#"r:link="far""#, "Far"))).link("far", "https://example.com/p.png"),
    );
    assert_eq!(shown(&remote), [("https://example.com/p.png".to_owned(), "Far".to_owned(), String::new())]);
    assert!(remote.assets.is_empty());
}

#[test]
fn refuses_what_is_not_a_word_document() {
    fn kind(error: &DocxError) -> String {
        match error {
            DocxError::Legacy => "legacy".into(),
            DocxError::Damaged(_) => "damaged".into(),
            DocxError::NotWord => "not word".into(),
            DocxError::Xml { part, .. } => format!("xml in {part}"),
            DocxError::TooLarge(part) => format!("too large: {part}"),
        }
    }
    let document = |xml: &str| zipped(&[("word/document.xml", xml.as_bytes())]);
    let styles = format!(
        r#"<Relationships xmlns="x"><Relationship Id="s" Type="{RELATIONSHIP}/styles" Target="styles.xml"/></Relationships>"#
    );
    let sound = format!("<w:document {NAMESPACES}><w:body/></w:document>");
    let cases = [
        (vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1, 0, 0], "legacy"),
        (b"Dear reader".to_vec(), "damaged"),
        (Vec::new(), "damaged"),
        (word("").bytes()[..200].to_vec(), "damaged"),
        (zipped(&[("mimetype", b"application/epub+zip")]), "not word"),
        (document(&format!("<w:document {NAMESPACES}/>")), "not word"),
        (document("<html><body>Hello</body></html>"), "not word"),
        (document("<w:document"), "xml in word/document.xml"),
        (zipped(&[("word/document.xml", &[b'<', 0xff, b'>'])]), "xml in word/document.xml"),
        (
            zipped(&[
                ("word/document.xml", sound.as_bytes()),
                ("word/_rels/document.xml.rels", styles.as_bytes()),
                ("word/styles.xml", b"<w:styles>"),
            ]),
            "xml in word/styles.xml",
        ),
    ];
    for (bytes, expected) in cases {
        let error = import(&bytes, &Style::default()).expect_err(expected);
        assert_eq!(kind(&error), expected, "{error}");
        assert!(!error.to_string().is_empty());
    }
}

fn document(markdown: &str) -> Document {
    let mut next = 0;
    let (doc, _) = from_markdown(markdown, &mut || {
        next += 1;
        format!("m{next}")
    });
    doc
}

fn export(doc: &Document) -> Vec<u8> {
    to_docx(doc, &Style::default(), &Notes::default(), "Report", &mut |_| None)
}

/// What a document comes back as from the Word file it is written to.
fn returned(doc: &Document) -> DocxImport {
    import(&export(doc), &Style::default()).expect("the export reads")
}

/// A part of a file as text.
fn part(file: &[u8], name: &str) -> String {
    let bytes = unzipped(file).remove(name).unwrap_or_else(|| panic!("the file has no {name}"));
    String::from_utf8(bytes).expect("the part is UTF-8")
}

/// The values of an attribute on every element of a name in an XML part.
fn values(xml: &str, element: &str, attribute: &str) -> BTreeSet<String> {
    let xml = roxmltree::Document::parse(xml).expect("the part is XML");
    let value = |node: roxmltree::Node| {
        node.attributes().find(|held| held.name() == attribute).map(|held| held.value().to_owned())
    };
    xml.descendants().filter(|node| node.tag_name().name() == element).filter_map(value).collect()
}

fn paragraph(id: &str, runs: Vec<Run>) -> Block {
    Block { body: Body::Text(runs), ..Block::new(id, BlockKind::Paragraph) }
}

fn figure(id: &str, fields: &[(&str, &str)], caption: &str) -> Block {
    let mut block = Block::new(id, BlockKind::Unknown { type_name: "figure".into() });
    block.extra.extend(fields.iter().map(|(name, value)| ((*name).to_owned(), (*value).into())));
    block.extra.insert("width".into(), 120.into());
    block.extra.insert("height".into(), 80.into());
    block.extra.insert("content".into(), Run::list_to_value(&[Run::plain(caption)]));
    block
}

fn comment(anchor: Anchor, author: &str, text: &str) -> Note {
    Note {
        id: format!("note-{author}-{text}"),
        kind: NoteKind::Comment,
        anchor,
        author: author.into(),
        created: "2024-03-01T10:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain(text)],
        replies: Vec::new(),
        extra: Default::default(),
    }
}

/// One of everything the document holds.
const EVERYTHING: &str = "# Report\n\nPlain **bold** *italic* ~~gone~~ <u>under</u> <mark>lit</mark> `code` and a \
                          [link](https://example.com/a?b=1&c=2).\n\n## Lists\n\n- one\n  - nested\n- two\n\n1. first\n\
                          2. second\n\n- [ ] open\n- [x] done\n\n> Quoted *words*.\n>\n> - and a list\n\n> [!WARNING]\n\
                          > Mind the gap.\n\n```rust\nfn main() {}\n```\n\n---\n\nLine one\\\nline two";

#[test]
fn a_document_survives_a_round_trip() {
    let cases = [
        EVERYTHING,
        "# One\n\n## Two\n\n### Three\n\n#### Four",
        "Plain **bold** *italic* ~~gone~~ <u>under</u> <mark>lit</mark> `code` ***both***",
        "A [link](https://example.com/a?b=1&c=2) and a [**bold** one](https://example.com/).",
        "- one\n  - nested\n    - deeper\n- two",
        "1. first\n   1. inner\n2. second\n\nBetween.\n\n1. again\n2. and again",
        "- [ ] open\n- [x] done",
        "> Quoted *words*.\n>\n> More of them.\n>\n> - and a list",
        "> [!TIP]\n> Try this.\n\n> [!NOTE]\n> Know this.\n\n> [!WARNING]\n> Mind this.\n\n> [!CAUTION]\n> Stop.",
        "```\nfn main() {\n\n    // indented\n}\n```",
        "Above\n\n---\n\nBelow",
        "one\\\ntwo",
        "# A **strong** heading\n\n> An *emphasised* quote.",
        "Tabs\tand   spaces, <tags> & \"quotes\", é 中 🙂",
        "",
    ];
    for markdown in cases {
        let doc = document(markdown);
        let back = returned(&doc);
        // Word has nowhere to say what language code is in.
        let expected: Vec<String> = outline(&doc).iter().map(|line| line.replace("code:rust", "code:-")).collect();
        assert_eq!(outline(&back.document), expected, "returning {markdown:?}");
        assert_eq!(back.notes, [], "nothing of {markdown:?} is simplified");
        assert_eq!(back.document.language, doc.language);
    }
}

#[test]
fn writes_outside_a_quote_what_word_cannot_keep_inside_one() {
    let back = returned(&document("> Before\n>\n> ## Heading\n>\n> After"));
    let texts: Vec<String> = back.document.walk().map(Block::plain_text).filter(|text| !text.is_empty()).collect();
    assert_eq!(texts, ["Before", "Heading", "After"]);
    assert!(outline(&back.document).contains(&r#"h2 "Heading""#.to_owned()), "{:?}", outline(&back.document));
}

#[test]
fn the_style_survives_a_round_trip() {
    let mut custom = Style::default();
    let body = custom.types.get_mut("body").expect("a style has a body");
    (body.family, body.size, body.color) = ("Georgia".into(), 11.5, "#223344".into());
    (body.align, body.line_height, body.space_after) = (Align::Justify, 1.45, 9.5);
    let heading = custom.types.get_mut("heading2").expect("a style has headings");
    (heading.italic, heading.space_before, heading.align) = (true, 20.0, Align::Center);
    custom.types.get_mut("code").expect("a style has code").size = 9.0;
    (custom.page.width, custom.page.height, custom.page.margin_left) = (612.0, 792.0, 90.0);

    // Each is read over the other, so that nothing comes from the style the import starts with.
    for (written, base) in [(&custom, &Style::default()), (&Style::default(), &custom)] {
        let file = to_docx(&document(EVERYTHING), written, &Notes::default(), "Report", &mut |_| None);
        let back = import(&file, base).expect("the export reads");
        for name in Style::TYPES {
            assert_eq!(back.style.types[name], written.types[name], "{name}");
        }
        assert_eq!(back.style.page, written.page);
    }
}

#[test]
fn the_style_is_written_as_word_styles_and_paragraphs_only_name_them() {
    let mut style = Style::default();
    let heading = style.types.get_mut("heading1").expect("a style has headings");
    (heading.family, heading.size, heading.color, heading.align) =
        ("Georgia".into(), 30.0, "#112233".into(), Align::Center);
    let file = to_docx(&document(EVERYTHING), &style, &Notes::default(), "Report", &mut |_| None);

    let styles = part(&file, "word/styles.xml");
    let xml = roxmltree::Document::parse(&styles).expect("the styles are XML");
    let named = |name: &str| {
        let is = |node: &roxmltree::Node| {
            node.tag_name().name() == "name" && node.attributes().any(|held| held.value() == name)
        };
        xml.descendants().find(is).and_then(|node| node.parent()).map(|style| &styles[style.range()])
    };
    // Word knows its own styles by these names, and shows them in its gallery under them.
    for name in ["Title", "heading 1", "heading 2", "heading 3", "heading 4", "Normal", "Quote", "caption"] {
        assert!(named(name).is_some(), "no style named {name:?}");
    }
    let look = named("heading 1").expect("a style for the first heading");
    for held in ["Georgia", r#"<w:sz w:val="60"/>"#, "112233", r#"<w:jc w:val="center"/>"#] {
        assert!(look.contains(held), "the heading's style lacks {held}: {look}");
    }
    let body = part(&file, "word/document.xml");
    for direct in ["<w:rFonts", "<w:sz ", "<w:color", "<w:jc", "<w:spacing", "<w:ind"] {
        assert!(!body.contains(direct), "a paragraph carries {direct} of its own");
    }
}

#[test]
fn the_parts_of_a_file_agree() {
    let mut doc = document(EVERYTHING);
    doc.blocks.push(figure("fig", &[("asset", "assets/chart.png"), ("alt", "A chart")], "Sales"));
    let first = doc.blocks[0].id.clone();
    let reply = Reply {
        author: "Grace".into(),
        created: NOW.into(),
        content: vec![Run::plain("Yes")],
        extra: Default::default(),
    };
    let linked = Run { link: Some("https://example.com/c".into()), ..Run::plain("See") };
    let notes = Notes {
        notes: vec![Note {
            replies: vec![reply],
            content: vec![linked],
            ..comment(Anchor::range(first, 0..3), "Ada", "")
        }],
        ..Notes::default()
    };
    let file = to_docx(&doc, &Style::default(), &notes, "Report", &mut |name| {
        (name == "assets/chart.png").then(|| PNG.to_vec())
    });
    let parts = unzipped(&file);
    let text = |name: &str| String::from_utf8(parts[name].clone()).expect("the part is UTF-8");

    let types = text("[Content_Types].xml");
    let (overrides, defaults) = (values(&types, "Override", "PartName"), values(&types, "Default", "Extension"));
    for name in parts.keys().filter(|name| *name != "[Content_Types].xml") {
        let extension = name.rsplit('.').next().unwrap_or_default();
        assert!(overrides.contains(&format!("/{name}")) || defaults.contains(extension), "{name} has no content type");
        if matches!(extension, "xml" | "rels") {
            let xml = text(name);
            assert!(roxmltree::Document::parse(&xml).is_ok(), "{name} is not XML: {xml}");
        }
    }

    // Every relationship leads to a part that is there, and every one a part uses is defined.
    for (rels, folder, owner) in [
        ("_rels/.rels", "", None),
        ("word/_rels/document.xml.rels", "word/", Some("word/document.xml")),
        ("word/_rels/comments.xml.rels", "word/", Some("word/comments.xml")),
    ] {
        let xml = text(rels);
        let tree = roxmltree::Document::parse(&xml).expect("relationships are XML");
        let inside = tree.descendants().filter(|node| {
            node.has_tag_name(("http://schemas.openxmlformats.org/package/2006/relationships", "Relationship"))
        });
        for rel in inside.filter(|rel| rel.attribute("TargetMode") != Some("External")) {
            let target = format!("{folder}{}", rel.attribute("Target").expect("a relationship has a target"));
            assert!(parts.contains_key(&target), "{rels} leads to {target}, which is not in the file");
        }
        if let Some(owner) = owner {
            let used: BTreeSet<String> = ["id", "embed"]
                .iter()
                .flat_map(|name| {
                    values(&text(owner), "hyperlink", name).into_iter().chain(values(&text(owner), "blip", name))
                })
                .collect();
            assert!(!used.is_empty() && used.is_subset(&values(&xml, "Relationship", "Id")), "{owner} uses {used:?}");
        }
    }
    assert_eq!(parts.get("word/media/image1.png").map(Vec::as_slice), Some(PNG));

    // Every style, list and comment the document names is defined.
    let body = text("word/document.xml");
    let used: BTreeSet<String> =
        values(&body, "pStyle", "val").into_iter().chain(values(&body, "rStyle", "val")).collect();
    assert!(used.len() > 8 && used.is_subset(&values(&text("word/styles.xml"), "style", "styleId")), "{used:?}");
    let lists = values(&body, "numId", "val");
    assert!(lists.len() > 3 && lists.is_subset(&values(&text("word/numbering.xml"), "num", "numId")), "{lists:?}");
    let comments = values(&text("word/comments.xml"), "comment", "id");
    assert_eq!(comments.len(), 2);
    for mark in ["commentRangeStart", "commentRangeEnd", "commentReference"] {
        assert_eq!(values(&body, mark, "id"), comments, "{mark}");
    }
}

#[test]
fn comments_survive_a_round_trip() {
    let doc = document("An important point.\n\nSecond.\n\n---");
    let [first, second, rule] = [0, 1, 2].map(|index| doc.blocks[index].id.clone());
    let reply = Reply {
        author: "Grace Hopper".into(),
        created: "2024-03-02T09:30:00Z".into(),
        content: vec![Run::marked("Checked.", [Mark::Bold])],
        extra: Default::default(),
    };
    let written = vec![
        Note { replies: vec![reply.clone()], ..comment(Anchor::range(first.clone(), 3..12), "Ada", "Check this.") },
        Note { resolved: true, ..comment(Anchor::block(second), "Ada", "Dealt with.") },
        // A block with no text still has a place in Word for its comment.
        comment(Anchor::block(rule), "Ada", "Why a rule?"),
        // What the import said and what is no longer there stay behind.
        Note { kind: NoteKind::Import, ..comment(Anchor::block(first), "Letters import", "Simplified.") },
        comment(Anchor::block(BlockId::new("gone")), "Ada", "Lost."),
    ];
    let file =
        to_docx(&doc, &Style::default(), &Notes { notes: written.clone(), ..Notes::default() }, "Report", &mut |_| {
            None
        });
    let back = import(&file, &Style::default()).expect("the export reads");

    assert_eq!(outline(&back.document), outline(&doc));
    assert_eq!(back.notes, []);
    assert_eq!(back.comments.len(), 3, "{:?}", back.comments);
    for (read, wrote) in back.comments.iter().zip(&written) {
        assert_eq!(read.kind, NoteKind::Comment);
        assert_eq!((&read.author, &read.created, read.resolved), (&wrote.author, &wrote.created, wrote.resolved));
        assert_eq!((&read.content, &read.replies), (&wrote.content, &wrote.replies));
        assert_eq!(anchored(&back.document, &read.anchor), anchored(&doc, &wrote.anchor));
        assert_eq!(read.anchor.range.is_some(), wrote.anchor.range.is_some());
    }
}

#[test]
fn pictures_are_written_into_the_file() {
    let mut doc = document("Before");
    doc.blocks.push(figure("held", &[("asset", "assets/chart.png"), ("alt", "A <chart>")], "Sales & costs"));
    doc.blocks.push(figure("again", &[("asset", "assets/chart.png"), ("alt", "")], ""));
    doc.blocks.push(figure("far", &[("src", "https://example.com/p.png"), ("alt", "Far")], ""));
    doc.blocks.push(figure("lost", &[("asset", "assets/missing.png"), ("alt", "Lost")], "Its caption"));
    let file = to_docx(&doc, &Style::default(), &Notes::default(), "Report", &mut |name| {
        (name == "assets/chart.png").then(|| PNG.to_vec())
    });
    let back = import(&file, &Style::default()).expect("the export reads");

    let media: Vec<String> = unzipped(&file).into_keys().filter(|name| name.starts_with("word/media/")).collect();
    assert_eq!(media.len(), 1, "a picture shown twice is stored once: {media:?}");
    let [(asset, alt, caption), (again, ..)] = pictures(&back.document).try_into().expect("two pictures");
    assert_eq!((alt.as_str(), caption.as_str()), ("A <chart>", "Sales & costs"));
    assert_eq!((back.assets.get(&asset).map(Vec::as_slice), &again), (Some(PNG), &asset));

    // A picture the file cannot hold is a link to it or its description, so nothing is fetched on opening.
    let lines = outline(&back.document);
    assert!(lines.contains(&r#"p "<@https://example.com/p.png|Far>""#.to_owned()), "{lines:?}");
    let texts: Vec<String> = back.document.walk().map(Block::plain_text).collect();
    assert!(texts.contains(&"Lost".to_owned()) && texts.contains(&"Its caption".to_owned()), "{texts:?}");
}

#[test]
fn links_that_would_run_script_are_written_as_text() {
    let cases = [
        "javascript:alert(1)",
        " JaVaScript:alert(1)",
        "java\tscript:alert(1)",
        "\u{1}vbscript:x",
        "data:text/html,<script>",
    ];
    for url in cases {
        let doc = Document {
            blocks: vec![paragraph("a", vec![Run { link: Some(url.into()), ..Run::plain("click") }])],
            ..Document::default()
        };
        let file = export(&doc);
        let rels = part(&file, "word/_rels/document.xml.rels");
        assert!(!rels.to_ascii_lowercase().contains("script"), "{url} is in {rels}");
        assert_eq!(outline(&returned(&doc).document), [r#"p "click""#], "{url}");
    }
}

#[test]
fn text_cannot_break_the_file() {
    let hostile = "</w:t></w:r></w:p> <w:p> & \" ' ]]> <!-- \u{1}\u{b}\u{ffff}\r end";
    let kept = "</w:t></w:r></w:p> <w:p> & \" ' ]]> <!--  end";
    let link = Run { link: Some("https://example.com/\"><x y=\"'&<>\u{1} é".into()), ..Run::plain(hostile) };
    let mut doc = Document { blocks: vec![paragraph("a", vec![Run::plain(hostile), link])], ..Document::default() };
    doc.blocks.push(figure("f", &[("asset", "assets/chart.png"), ("alt", hostile)], hostile));
    doc.language = "en\"><x".into();
    let note = Note { created: hostile.into(), ..comment(Anchor::block(BlockId::new("a")), hostile, hostile) };
    let notes = Notes { notes: vec![note], ..Notes::default() };
    let file = to_docx(&doc, &Style::default(), &notes, hostile, &mut |_| Some(PNG.to_vec()));

    for (name, bytes) in unzipped(&file).iter().filter(|(name, _)| name.ends_with(".xml") || name.ends_with(".rels")) {
        let xml = String::from_utf8(bytes.clone()).expect("the part is UTF-8");
        assert!(roxmltree::Document::parse(&xml).is_ok(), "{name} is not XML: {xml}");
    }
    let back = import(&file, &Style::default()).expect("the export reads");
    assert_eq!(back.document.blocks[0].plain_text(), format!("{kept}{kept}"));
    let [(_, alt, caption)] = pictures(&back.document).try_into().expect("one picture");
    assert_eq!((alt.as_str(), caption.as_str()), (kept, kept));
    assert_eq!((back.comments[0].author.as_str(), said(&back.comments[0].content).as_str()), (kept, kept));
    assert!(
        back.document.blocks[0]
            .body
            .runs()
            .iter()
            .any(|run| run.link.as_deref().is_some_and(|url| url.starts_with("https://example.com/")))
    );
}

#[test]
fn the_same_document_is_the_same_file() {
    let doc = document(EVERYTHING);
    assert_eq!(export(&doc), export(&doc));
    let empty = returned(&Document::default());
    assert_eq!((empty.document.blocks.len(), empty.notes.len()), (0, 0), "a file with nothing in it is still a file");
}

/// Text that leans on the characters XML and Word give meaning to.
fn any_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        4 => "[a-z]{1,4}",
        2 => Just(" ".to_owned()),
        3 => proptest::sample::select(vec![
            "<", ">", "&", "\"", "'", "&amp;", "]]>", "<w:t>", "</w:p>", "\n", "\t", "  ", "é", "中", "🙂", "\u{a0}", "[^1]", "1.",
        ])
        .prop_map(str::to_owned),
    ];
    // A block of nothing but space is one Word spaces text with, and is not read.
    (proptest::collection::vec(piece, 0..6), "[a-z]{1,3}", any::<bool>())
        .prop_map(|(pieces, word, first)| if first { word + &pieces.concat() } else { pieces.concat() + &word })
}

fn any_runs() -> impl Strategy<Value = Vec<Run>> {
    let marks = [Mark::Bold, Mark::Italic, Mark::Underline, Mark::Strike, Mark::Highlight, Mark::Code];
    let marks = prop_oneof![
        3 => Just(Vec::new()),
        3 => proptest::sample::subsequence(marks.to_vec(), 1..=2),
        1 => proptest::sample::subsequence(marks.to_vec(), 0..=6),
    ];
    let urls = vec!["https://example.com/a?b=1&c=2", "mailto:a@example.com", "/relative#part", "other.let"];
    let link = proptest::option::weighted(0.2, proptest::sample::select(urls));
    let run = (any_text(), marks, link)
        .prop_map(|(text, marks, link)| Run { link: link.map(str::to_owned), ..Run::marked(text, marks) });
    proptest::collection::vec(run, 1..4)
}

/// Blocks that Word can hold as they are: a quote or a callout holds paragraphs and list items alone.
fn any_blocks() -> impl Strategy<Value = Vec<Block>> {
    let with = |kind: BlockKind, runs: Vec<Run>| Block { body: Body::Text(runs), ..Block::new("", kind) };
    let list = prop_oneof![
        Just(ListKind::Bullet),
        Just(ListKind::Ordered),
        any::<bool>().prop_map(|checked| ListKind::Task { checked }),
    ];
    let text = prop_oneof![
        3 => any_runs().prop_map(move |runs| with(BlockKind::Paragraph, runs)),
        3 => (list, 0..3u8, any_runs()).prop_map(move |(list, depth, runs)| with(BlockKind::ListItem { list, depth }, runs)),
    ];
    let tone = proptest::sample::select(vec![Tone::Tip, Tone::Info, Tone::Warn, Tone::Danger]);
    let holding = |kind: BlockKind, blocks: Vec<Block>| Block { body: Body::Blocks(blocks), ..Block::new("", kind) };
    let inside = proptest::collection::vec(text.clone(), 1..4).prop_map(nested);
    let block = prop_oneof![
        6 => text,
        1 => (1..=4u8, any_runs()).prop_map(move |(level, runs)| with(BlockKind::Heading { level }, runs)),
        1 => any_text().prop_map(move |code| with(BlockKind::Code { language: None }, vec![Run::plain(code.trim_end_matches('\n'))])),
        1 => Just(Block::new("", BlockKind::Divider)),
        1 => inside.clone().prop_map(move |blocks| holding(BlockKind::Quote, blocks)),
        1 => (tone, inside).prop_map(move |(tone, blocks)| holding(BlockKind::Callout { tone }, blocks)),
    ];
    proptest::collection::vec(block, 0..8).prop_map(nested)
}

/// A document as Word holds it and hands it back, which is the document with what Word cannot tell apart
/// made the same.
fn settled(blocks: Vec<Block>) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for mut block in blocks {
        if let Body::Text(runs) = &mut block.body {
            runs.retain(|run| !run.text.is_empty());
            for run in runs.iter_mut() {
                // The page draws a link its own way, so Word's underline of one says nothing.
                if run.link.is_some() {
                    run.marks.remove(&Mark::Underline);
                }
            }
            runs.dedup_by(|next, last| {
                let same = last.marks == next.marks && last.link == next.link;
                if same {
                    last.text.push_str(&next.text);
                }
                same
            });
        }
        // Neighbouring paragraphs of one style are one quote, one callout or one block of code in Word.
        match (out.last_mut(), &block) {
            (Some(Block { kind, body: Body::Blocks(held), .. }), Block { body: Body::Blocks(more), .. })
                if *kind == block.kind =>
            {
                held.extend(more.iter().cloned());
            }
            (
                Some(Block { kind: BlockKind::Code { .. }, body: Body::Text(held), .. }),
                Block { kind: BlockKind::Code { .. }, .. },
            ) => {
                held[0].text = format!("{}\n{}", held[0].text, block.plain_text());
            }
            _ => out.push(block),
        }
    }
    for block in &mut out {
        if let Body::Blocks(inner) = &mut block.body {
            *inner = settled(std::mem::take(inner));
        }
    }
    out
}

/// Sets each list item at most one level under the one before it, which is as deep as the editor lets one go.
fn nested(mut blocks: Vec<Block>) -> Vec<Block> {
    let mut deepest = 0;
    for block in &mut blocks {
        deepest = match &mut block.kind {
            BlockKind::ListItem { depth, .. } => {
                *depth = (*depth).min(deepest);
                *depth + 1
            }
            _ => 0,
        };
    }
    blocks
}

proptest! {
    #[test]
    fn any_document_survives_a_round_trip(blocks in any_blocks()) {
        let doc = Document { blocks, ..Document::default() };
        let back = returned(&doc);
        let expected = Document { blocks: settled(doc.blocks.clone()), ..Document::default() };
        prop_assert_eq!(outline(&back.document), outline(&expected));
        prop_assert_eq!(back.notes, []);
    }

    /// Whatever a file nests, reading it ends, and what it says of the document is true of it.
    #[test]
    fn any_body_reads_into_a_sound_document(body in any_body()) {
        let comments = r#"<w:comment w:id="0" w:author="A"><w:p><w:r><w:t>c</w:t></w:r></w:p></w:comment>
            <w:comment w:id="1"><w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl></w:comment>"#;
        let footnotes = r#"<w:footnote w:id="1"><w:p><w:r><w:t> note</w:t></w:r></w:p></w:footnote>"#;
        let file = word(&body).part("comments", comments).part("footnotes", footnotes).picture("png", PNG);
        let import = read(&file.link("site", "https://example.com/"));

        let ids: Vec<&BlockId> = import.document.walk().map(|block| &block.id).collect();
        prop_assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len(), "ids are told apart");
        for note in &import.notes {
            prop_assert!(import.document.block(&BlockId::new(&note.block)).is_some(), "{:?} is on no block", note);
        }
        for comment in &import.comments {
            let text = import.document.block(&comment.anchor.block).map(Block::plain_text);
            prop_assert!(text.is_some(), "{:?} is on no block", comment.anchor);
            if let Some(range) = &comment.anchor.range {
                prop_assert!(text.unwrap_or_default().get(range.clone()).is_some(), "{:?} is not in its block", range);
            }
        }
        for name in pictures(&import.document).iter().map(|(shown, ..)| shown).filter(|shown| shown.starts_with("assets/")) {
            prop_assert!(import.assets.contains_key(name), "{} is not kept", name);
        }
    }
}

/// A body of whatever Word might nest: well formed, and otherwise as it falls. Fields and comments start and
/// end anywhere, as they do in files that have been edited for years.
fn any_body() -> impl Strategy<Value = String> {
    let field = |kind: &str| format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#);
    let comment = |mark: &str, id: u8| format!(r#"<w:comment{mark} w:id="{id}"/>"#);
    let inline = vec![
        t("\u{2610} "),
        "<w:r><w:tab/><w:br/></w:r>".to_owned(),
        run(r#"<w:b/><w:sz w:val="40"/>"#, "big"),
        run(r#"<w:rStyle w:val="Mono"/>"#, "code"),
        format!("<w:ins>{}</w:ins><w:del><w:r><w:delText>d</w:delText></w:r></w:del>", t("in")),
        format!(r#"<w:hyperlink w:anchor="a">{}</w:hyperlink>"#, t("inner")),
        format!(r#"<w:hyperlink r:id="site">{}</w:hyperlink>"#, t("outer")),
        field("begin"),
        field("separate"),
        field("end"),
        r#"<w:r><w:instrText> HYPERLINK "https://example.com/f" </w:instrText></w:r>"#.to_owned(),
        "<w:r><w:instrText> TOC </w:instrText></w:r>".to_owned(),
        comment("RangeStart", 0),
        comment("RangeEnd", 0),
        format!("<w:r>{}</w:r>", comment("Reference", 0)),
        comment("RangeEnd", 1),
        comment("RangeStart", 1),
        format!("<w:r>{}</w:r>", comment("Reference", 1)),
        r#"<w:r><w:footnoteReference w:id="1"/></w:r>"#.to_owned(),
        r#"<w:r><w:footnoteReference w:id="9"/></w:r>"#.to_owned(),
        picture(r#"r:embed="png""#, "alt"),
        "<m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>".to_owned(),
        r#"<w:sdt><w:sdtPr><w14:checkbox><w14:checked w14:val="1"/></w14:checkbox></w:sdtPr></w:sdt>"#.to_owned(),
    ];
    let style = |id: &str| format!(r#"<w:pPr><w:pStyle w:val="{id}"/></w:pPr>"#);
    let list = |id: u8, depth: u8| {
        format!(r#"<w:pPr><w:numPr><w:ilvl w:val="{depth}"/><w:numId w:val="{id}"/></w:numPr></w:pPr>"#)
    };
    let props = vec![
        String::new(),
        style("H1"),
        style("T"),
        style("Q"),
        style("Src"),
        style("Cap"),
        style("Toc1"),
        style("Bullet2"),
        list(1, 0),
        list(3, 1),
        list(2, 7),
    ];
    let inline = prop_oneof![2 => "[a-z ]{1,6}".prop_map(|text| t(&text)), 3 => proptest::sample::select(inline)];
    let paragraph = (proptest::sample::select(props), proptest::collection::vec(inline, 0..6))
        .prop_map(|(props, inline)| format!("<w:p>{props}{}</w:p>", inline.concat()));
    let block = paragraph.prop_recursive(4, 48, 4, |inner| {
        let blocks = proptest::collection::vec(inner, 0..4).prop_map(|blocks| blocks.concat());
        let wrapped = |open: &'static str, close: &'static str| {
            blocks.clone().prop_map(move |blocks| format!("{open}{blocks}{close}"))
        };
        let rows = proptest::collection::vec(proptest::collection::vec(blocks.clone(), 1..3), 1..3);
        prop_oneof![
            rows.prop_map(|rows| {
                let row =
                    |cells: &Vec<String>| cells.iter().map(|cell| format!("<w:tc>{cell}</w:tc>")).collect::<String>();
                format!(
                    "<w:tbl>{}</w:tbl>",
                    rows.iter().map(|cells| format!("<w:tr>{}</w:tr>", row(cells))).collect::<String>()
                )
            }),
            wrapped("<w:sdt><w:sdtContent>", "</w:sdtContent></w:sdt>"),
            wrapped("<w:ins>", "</w:ins>"),
            wrapped(
                "<w:p><w:r><w:t>beside</w:t><w:drawing><w:txbxContent>",
                "</w:txbxContent></w:drawing></w:r></w:p>"
            ),
        ]
    });
    proptest::collection::vec(block, 0..6).prop_map(|blocks| blocks.concat())
}
