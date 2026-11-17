//! Word documents, through the two public functions: reading what Word and other tools write, and writing
//! what Word reads back.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read, Write};

use letters_convert::{DocxError, DocxImport, from_docx, from_markdown, to_docx};
use letters_model::{
    Align, Anchor, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Note, NoteKind, Notes, Preview, Reply,
    Run, Style, Tone,
};
use proptest::prelude::*;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use common::outline;

/// The time an import is told it happens at.
const NOW: &str = "2026-01-02T03:04:05Z";

/// The start of a PNG, which is all that tells a picture's format.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-pixels-";

/// A picture of four pixels by two, for where a picture's size is asked.
const PIXELS: &[u8] = b"\x89\x50\x4e\x47\x0d\x0a\x1a\x0a\x00\x00\x00\x0d\x49\x48\x44\x52\x00\x00\x00\x04\x00\x00\x00\x02\x08\x02\x00\x00\x00\xf0\xca\xea\x34\x00\x00\x00\x12\x49\x44\x41\x54\x78\xda\x63\x3c\x61\xa3\xc1\x00\x03\x4c\x0c\x48\x00\x00\x1d\x0e\x01\x30\x58\x04\x05\xe9\x00\x00\x00\x00\x49\x45\x4e\x44\xae\x42\x60\x82";

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
<w:style w:type="paragraph" w:styleId="Chapter"><w:name w:val="Chapter"/><w:basedOn w:val="H1"/>
  <w:pPr><w:pageBreakBefore/></w:pPr></w:style>
<w:style w:type="character" w:styleId="Mono"><w:name w:val="Verbatim Char"/>
  <w:rPr><w:rFonts w:ascii="Fira Code" w:hAnsi="Fira Code"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Link"><w:name w:val="Hyperlink"/>
  <w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Head"><w:name w:val="Table Heading"/><w:basedOn w:val="Std"/>
  <w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="table" w:styleId="Grid"><w:name w:val="Table Grid"/>
  <w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr></w:tblStylePr></w:style>
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
    /// What Word records of the file, as what sits inside the root element of that part.
    core: Option<String>,
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
        if let Some(core) = &self.core {
            let namespaces = r#"xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#;
            let part = format!("<cp:coreProperties {namespaces}>{core}</cp:coreProperties>");
            entries.push(("docProps/core.xml".to_owned(), part.into_bytes()));
        }
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

#[test]
fn a_file_says_when_it_was_made_and_last_changed() {
    let dated =
        |name: &str, date: &str| format!(r#"<dcterms:{name} xsi:type="dcterms:W3CDTF">{date}</dcterms:{name}>"#);
    // 4 May 2019 and 7 June 2021, 08:09:10, both in UTC.
    let (made, changed) = (Some(1_556_928_000), Some(1_623_053_350));
    let cases = [
        (Some(dated("created", "2019-05-04T00:00:00Z") + &dated("modified", "2021-06-07T08:09:10Z")), (made, changed)),
        (
            Some("<dc:title>Roof</dc:title>".to_owned() + &dated("modified", "2021-06-07T10:09:10+02:00")),
            (None, changed),
        ),
        (Some(dated("created", "2019-05-04")), (made, None)),
        // A file that says nothing, or nothing that can be read, is brought in all the same.
        (Some(dated("created", "last spring")), (None, None)),
        (Some("<broken".to_owned()), (None, None)),
        (None, (None, None)),
    ];
    for (core, expected) in cases {
        let file = Word { core: core.clone(), ..bare(&p("Words")) };
        let read = read(&file);
        assert_eq!((read.dates.created, read.dates.modified), expected, "{core:?}");
        assert_eq!(read.document.blocks.len(), 1, "{core:?}");
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

/// A run of an equation.
fn m(text: &str) -> String {
    format!("<m:r><m:t>{text}</m:t></m:r>")
}

/// A structure of an equation: its properties, then each part by name.
fn structure(name: &str, props: &str, parts: &[(&str, String)]) -> String {
    let parts: String = parts.iter().map(|(part, inner)| format!("<m:{part}>{inner}</m:{part}>")).collect();
    format!("<m:{name}><m:{name}Pr>{props}</m:{name}Pr>{parts}</m:{name}>")
}

/// The formulas of a document in order: whether each is a block of its own, and its source.
fn formulas(doc: &Document) -> Vec<(bool, String)> {
    let within = |block: &Block| {
        let maths = block.body.runs().iter().filter(|run| run.marks.contains(&Mark::Math));
        maths.map(|run| (false, run.text.clone())).collect::<Vec<_>>()
    };
    doc.walk()
        .flat_map(|block| match block.kind {
            BlockKind::Formula => vec![(true, block.plain_text())],
            _ => within(block),
        })
        .collect()
}

/// What a source is typeset as, which every way of typing one formula shares.
fn typeset(source: &str) -> String {
    letters_math::mathml(source, true).unwrap_or_else(|error| panic!("{source:?} is no formula: {error:?}"))
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

/// A field as Word writes one it has worked out: its instructions, then what they last came to.
fn field(code: &str, shown: &str) -> String {
    let mark = |kind: &str| format!(r#"<w:r><w:fldChar w:fldCharType="{kind}"/></w:r>"#);
    format!(
        "{}<w:r><w:instrText>{code}</w:instrText></w:r>{}{}{}",
        mark("begin"),
        mark("separate"),
        t(shown),
        mark("end")
    )
}

/// The same field as tools that write it in one piece do.
fn simple_field(code: &str, shown: &str) -> String {
    format!(r#"<w:fldSimple w:instr="{code}">{}</w:fldSimple>"#, t(shown))
}

/// The pictures of a document: what each shows, its alternative text and its caption.
fn pictures(doc: &Document) -> Vec<(String, String, String)> {
    let figure = |block: &Block| match &block.kind {
        BlockKind::Figure { asset, src, alt, .. } => {
            Some((asset.clone().or_else(|| src.clone())?, alt.clone(), block.plain_text()))
        }
        _ => None,
    };
    doc.walk().filter_map(figure).collect()
}

/// A document as `outline` writes it, with every asset under one name. An import names a picture after
/// what it holds, which no case could spell out.
fn shape(doc: &Document) -> Vec<String> {
    fn rename(blocks: &mut [Block]) {
        for block in blocks {
            if let BlockKind::Figure { asset: Some(name), .. } = &mut block.kind {
                *name = "asset".into();
            }
            if let Body::Blocks(inner) = &mut block.body {
                rename(inner);
            }
        }
    }
    let mut doc = doc.clone();
    rename(&mut doc.blocks);
    outline(&doc)
}

/// The line that heads each callout of a document, in order.
fn headed(doc: &Document) -> Vec<String> {
    doc.walk().filter_map(|block| block.kind.callout_look()).map(|look| look.title.to_owned()).collect()
}

/// The references of a document: what each reads as, and the caption of the figure it is to.
fn references(doc: &Document) -> Vec<(String, String)> {
    let caption = |id: &BlockId| {
        let figure = doc.block(id).expect("a reference names a block of the document");
        let under = figure.body.blocks().iter().find(|part| part.kind == BlockKind::Caption);
        under.unwrap_or(figure).plain_text()
    };
    let refers = |run: &Run| {
        let to = run.marks.iter().find_map(|mark| match mark {
            Mark::Ref(figure) => Some(caption(figure)),
            _ => None,
        });
        Some((run.text.clone(), to?))
    };
    doc.walk().flat_map(|block| block.body.runs()).filter_map(refers).collect()
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
        (styled("Head", "x"), r#"p "<bold|x>""#),
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

/// The text of each block that starts a new page.
fn turned(doc: &Document) -> Vec<String> {
    doc.walk().filter(|block| block.page_break).map(Block::plain_text).collect()
}

#[test]
fn reads_where_pages_start() {
    let page = r#"<w:r><w:br w:type="page"/></w:r>"#;
    let breaking = |props: &str, text: &str| para(&format!("{props}<w:pageBreakBefore/>"), &t(text));
    let ending = |section: &str, text: &str| para(&format!("<w:sectPr>{section}</w:sectPr>"), &t(text));
    let running = r#"<w:type w:val="continuous"/>"#;
    let cases: &[(String, &[&str])] = &[
        (p("a") + &breaking("", "b") + &p("c"), &["b"]),
        // A style can start each of its paragraphs on a page.
        (p("a") + &styled("Chapter", "b") + &styled("Chapter", "c"), &["b", "c"]),
        // A break in the text: in the empty paragraph Word's button makes, before a paragraph's text, and after it.
        (p("a") + &format!("<w:p>{page}</w:p>") + &p("b"), &["b"]),
        (p("a") + &format!("<w:p>{page}{}</w:p>", t("b")), &["b"]),
        (format!("<w:p>{}{page}</w:p>", t("a")) + &p("b") + &p("c"), &["b"]),
        // A section starts on a new page unless it says that it runs on. Each says so where it ends.
        (ending("", "a") + &p("b") + "<w:sectPr/>", &["b"]),
        (ending("", "a") + &ending(running, "b") + &p("c") + "<w:sectPr/>", &["c"]),
        (ending("", "a") + &p("b") + &format!("<w:sectPr>{running}</w:sectPr>"), &[]),
        // Inside a quote the break stays on its paragraph, and code that starts a page is a block of its own.
        (styled("Q", "a") + &breaking(r#"<w:pStyle w:val="Q"/>"#, "b"), &["b"]),
        (styled("Src", "a") + &breaking(r#"<w:pStyle w:val="Src"/>"#, "b") + &styled("Src", "c"), &["b\nc"]),
        // A column break is no page break, and neither is a line break.
        (p("a") + r#"<w:p><w:r><w:br w:type="column"/><w:br/></w:r></w:p>"# + &p("b"), &[]),
    ];
    for (body, expected) in cases {
        assert_eq!(turned(&read(&word(body)).document), *expected, "reading {body}");
    }
}

/// Each block that holds words or names a language: its language where it names one, then its text with
/// each run in a language of its own as `<fr|mot>`.
fn tongues(doc: &Document) -> Vec<String> {
    let run = |run: &Run| match &run.lang {
        Some(lang) => format!("<{lang}|{}>", run.text),
        None => run.text.clone(),
    };
    let line = |block: &Block| {
        let text: String = block.body.runs().iter().map(run).collect();
        let lang = block.lang.as_ref().map(|lang| format!("{lang}: "));
        (lang.is_some() || !text.is_empty()).then(|| lang.unwrap_or_default() + &text)
    };
    doc.walk().filter_map(line).collect()
}

#[test]
fn reads_the_language_text_is_in() {
    let lang = |tag: &str, text: &str| run(&format!(r#"<w:lang w:val="{tag}"/>"#), text);
    let quoted = |inner: &str| para(r#"<w:pStyle w:val="Q"/>"#, inner);
    // The shared styles start from British English.
    let cases: &[(String, &str, &[&str])] = &[
        (p("Hello") + &p("World"), "en-GB", &["Hello", "World"]),
        // A passage in another language is tagged, and a block all in one says so once.
        (format!("<w:p>{}{}</w:p>", t("The word "), lang("fr-FR", "mot")), "en-GB", &["The word <fr-FR|mot>"]),
        (
            p("Hello there, everyone")
                + &format!("<w:p>{}{}</w:p>", lang("fr-FR", "Bonjour "), lang("fr-FR", "à tous")),
            "en-GB",
            &["Hello there, everyone", "fr-FR: Bonjour à tous"],
        ),
        (
            p("Hello there, all") + &quoted(&lang("es", "Hola")) + &quoted(&lang("es", "Mundo")),
            "en-GB",
            &["Hello there, all", "es: ", "Hola", "Mundo"],
        ),
        // Word tags every run with the language it was typed in, whatever its styles start from. The
        // language of most of the text is the document's.
        (
            format!("<w:p>{}</w:p>", lang("de-DE", "Guten Morgen zusammen")) + &p("Hello") + &p("12"),
            "de-DE",
            &["Guten Morgen zusammen", "en-GB: Hello", "en-GB: 12"],
        ),
        // Text that reads from the right is in the language Word keeps for such text.
        (
            p("Hello there, everyone")
                + &format!("<w:p>{}</w:p>", run(r#"<w:rtl/><w:lang w:val="en-US" w:bidi="he-IL"/>"#, "שלום לכולם")),
            "en-GB",
            &["Hello there, everyone", "he-IL: שלום לכולם"],
        ),
        // Code is in no language.
        (
            format!("<w:p>{}{}</w:p>", lang("fr-FR", "Bonjour "), run(r#"<w:rStyle w:val="Mono"/>"#, "x"))
                + &p("Hello, all"),
            "en-GB",
            &["fr-FR: Bonjour x", "Hello, all"],
        ),
    ];
    for (body, language, expected) in cases {
        let document = read(&word(body)).document;
        assert_eq!(tongues(&document), *expected, "reading {body}");
        assert_eq!(document.language, *language, "reading {body}");
    }
    assert_eq!(read(&bare(&p("Hello"))).document.language, Document::default().language, "a file that names none");
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
fn reads_equations_as_the_source_a_person_would_type() {
    let val = |name: &str, value: &str| format!(r#"<m:{name} m:val="{value}"/>"#);
    let upright = |text: &str| format!("<m:r><m:rPr>{}</m:rPr><m:t>{text}</m:t></m:r>", val("sty", "p"));
    let e = |inner: String| ("e", inner);
    let matrix = structure(
        "m",
        "",
        &[
            ("mr", format!("<m:e>{}</m:e><m:e>{}</m:e>", m("a"), m("b"))),
            ("mr", format!("<m:e>{}</m:e><m:e>{}</m:e>", m("c"), m("d"))),
        ],
    );
    let rows = structure("eqArr", "", &[e(m("a&amp;=b")), e(m("c&amp;=d"))]);
    // Tools other than Word line equations up as a matrix whose columns meet.
    let column = |side: &str| format!("<m:mc><m:mcPr>{}</m:mcPr></m:mc>", val("mcJc", side));
    let met = structure(
        "m",
        &format!("<m:mcs>{}{}</m:mcs>", column("right"), column("left")),
        &[("mr", format!("<m:e>{}</m:e><m:e>{}</m:e>", m("a"), m("=b")))],
    );
    let cases: &[(String, &str)] = &[
        (m("x=1"), "x=1"),
        (m("α≤β×2"), r"\alpha\le\beta\times 2"),
        (structure("f", "", &[("num", m("a")), ("den", m("b"))]), r"\frac{a}{b}"),
        (structure("sSup", "", &[e(m("x")), ("sup", m("2"))]), "x^2"),
        (structure("sSubSup", "", &[e(m("x")), ("sub", m("ij")), ("sup", m("2"))]), "x_{ij}^2"),
        (structure("rad", &val("degHide", "1"), &[("deg", String::new()), e(m("x+1"))]), r"\sqrt{x+1}"),
        (structure("rad", "", &[("deg", m("3")), e(m("x"))]), r"\sqrt[3]{x}"),
        (structure("nary", &val("chr", "∑"), &[("sub", m("i=1")), ("sup", m("n")), e(m("i"))]), r"\sum_{i=1}^n i"),
        (structure("nary", &val("limLoc", "subSup"), &[("sub", m("0")), ("sup", m("1")), e(m("x"))]), r"\int_0^1 x"),
        (structure("d", "", &[e(m("a+b"))]), r"\left(a+b\right)"),
        (
            structure("d", &(val("begChr", "[") + &val("endChr", "]")), &[e(matrix)]),
            r"\begin{bmatrix} a & b \\ c & d \end{bmatrix}",
        ),
        (
            structure("d", &(val("begChr", "{") + &val("endChr", "")), &[e(rows.clone())]),
            r"\begin{cases} a&=b \\ c&=d \end{cases}",
        ),
        (rows, r"\begin{aligned} a&=b \\ c&=d \end{aligned}"),
        (met, r"\begin{aligned} a & =b \end{aligned}"),
        (structure("func", "", &[("fName", upright("sin")), e(m("x"))]), r"\sin x"),
        (structure("limLow", "", &[e(m("⋃")), ("lim", m("i"))]) + &m("A"), r"\bigcup_i A"),
        (structure("limLow", "", &[e(m("c+d")), ("lim", m("⏟"))]), r"\underbrace{c+d}"),
        (structure("limUpp", "", &[e(m("→")), ("lim", m("f"))]), r"\xrightarrow{f}"),
        (structure("limLow", "", &[e(upright("lim")), ("lim", m("n→∞"))]) + &m("a"), r"\lim_{n\to\infty} a"),
        (structure("acc", &val("chr", "\u{20d7}"), &[e(m("v"))]), r"\vec{v}"),
        (structure("bar", &val("pos", "top"), &[e(m("x"))]), r"\overline{x}"),
        (format!("<m:r><m:rPr>{}</m:rPr><m:t>R</m:t></m:r>", val("scr", "double-struck")), r"\mathbb{R}"),
        (format!("<m:r><m:rPr><m:nor/></m:rPr><m:t>if </m:t></m:r>{}", m("x")), r"\text{if }x"),
    ];
    for (equation, source) in cases {
        let import = read(&word(&format!("<w:p>{}<m:oMath>{equation}</m:oMath></w:p>", t("So "))));
        let read = formulas(&import.document);
        let [(false, read)] = read.as_slice() else { panic!("{equation} is one formula in its line, got {read:?}") };
        assert_eq!(typeset(read), typeset(source), "reading {equation} as {read:?}");
        assert_eq!(import.notes, [], "nothing of {equation} is simplified");
    }
}

#[test]
fn an_equation_alone_in_its_paragraph_is_a_block() {
    let (x, y) = (format!("<m:oMath>{}</m:oMath>", m("x")), format!("<m:oMath>{}</m:oMath>", m("y")));
    let list = r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#;
    // The body, the block it reads as, whether the formula is that block, and the formula.
    let cases: &[(String, &str, bool, &str)] = &[
        (format!("<w:p>{x}</w:p>"), "formula", true, "x"),
        (format!("<w:p><m:oMathPara>{x}</m:oMathPara></w:p>"), "formula", true, "x"),
        (
            format!("<w:p><m:oMathPara>{x}{y}</m:oMathPara></w:p>"),
            "formula",
            true,
            r"\begin{gathered} x \\ y \end{gathered}",
        ),
        (para(r#"<w:pStyle w:val="Q"/>"#, &x), "quote", true, "x"),
        (format!("<w:p>{}{x}</w:p>", t("So ")), "p", false, "x"),
        (format!("<w:p>{x}{}{y}</w:p>", t(" and ")), "p", false, "x"),
        (para(list, &x), "bullet/0", false, "x"),
        (para(r#"<w:pStyle w:val="H1"/>"#, &x), "h1", false, "x"),
        (format!(r#"<w:p><w:hyperlink r:id="site">{x}</w:hyperlink></w:p>"#), "p", false, "x"),
    ];
    for (body, kind, alone, source) in cases {
        let doc = read(&word(body).link("site", "https://example.com/")).document;
        let lines = outline(&doc);
        assert!(lines[0].split(' ').next() == Some(*kind), "{body} reads as a {kind}, got {lines:?}");
        let (block, read) = formulas(&doc).into_iter().next().unwrap_or_else(|| panic!("{body} holds a formula"));
        assert_eq!((block, typeset(&read)), (*alone, typeset(source)), "reading {body} as {read:?}");
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
    let chart = r#"<w:p><w:r><w:t>Chart: </w:t></w:r><w:r><w:drawing><wp:inline><a:graphic/></wp:inline></w:drawing></w:r></w:p>"#;
    // Roots inside roots, further down than a formula is typeset.
    let deep = (0..40).fold(m("x"), |inner, _| structure("rad", "", &[("deg", String::new()), ("e", inner)]));
    let deep_source = format!("So {}x{}", r"\sqrt{".repeat(40), "}".repeat(40));
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
        (format!("<w:p>{}<m:oMath>{deep}</m:oMath></w:p>", t("So ")), &[deep_source.as_str()], "equation"),
        (
            format!("<w:p>{}<m:oMath><w:del>{}</w:del><w:ins>{}</w:ins></m:oMath></w:p>", t("So "), m("y"), m("x")),
            &["So x"],
            "Tracked changes",
        ),
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
        (chart.to_owned(), &["Chart: "], "drawing"),
        (styled("T", "Report"), &["Report"], "Title"),
        (styled("H6", "Deep"), &["Deep"], "level 6"),
        (
            format!(r#"<w:p>{}<w:r><w:br w:type="page"/></w:r>{}</w:p>"#, t("Before "), t("after")),
            &["Before after"],
            "page break",
        ),
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
fn reads_a_table_of_contents_as_one_block() {
    let entry = |text: &str| styled("Toc1", text);
    let field = |code: &str| {
        format!(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>{code}</w:instrText></w:r>
            <w:r><w:fldChar w:fldCharType="separate"/></w:r>{}</w:p><w:p>{}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
            t("Introduction 1"),
            t("Method 2")
        )
    };
    let built = |inside: &str| {
        format!(
            r#"<w:sdt><w:sdtPr><w:docPartObj><w:docPartGallery w:val="Table of Contents"/></w:docPartObj></w:sdtPr>
            <w:sdtContent>{inside}</w:sdtContent></w:sdt>"#
        )
    };
    let unmade = r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> TOC \o "1-1" </w:instrText></w:r>
        <w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#;
    // A field makes the table, whether Word's own frame holds it or not, and says how deep it lists.
    let cases = [
        (field(r#" TOC \o "1-3" \h \z \u "#), "contents:3"),
        (field(r#" TOC \o "2-2" "#), "contents:2"),
        (field(r#" TOC \h "#), "contents:4"),
        (built(&field(r#" TOC \o "1-2" "#)), "contents:2"),
        (built(&(entry("Introduction 1") + &entry("Method 2"))), "contents:4"),
        (unmade.to_owned(), "contents:1"),
    ];
    for (contents, expected) in cases {
        let import = read(&word(&(p("Before") + &contents + &p("After"))));
        assert_eq!(outline(&import.document), [r#"p "Before""#, expected, r#"p "After""#], "reading {contents}");
        assert_eq!(import.notes, [], "reading {contents}");
    }

    // Lines that are only set as a table say nothing of what they list, so they are left out, with a note.
    let import = read(&word(&(p("Before") + &entry("Introduction 1") + &p("After"))));
    assert_eq!(outline(&import.document), [r#"p "Before""#, r#"p "After""#]);
    assert!(noted(&import, "After").iter().any(|note| note.contains("contents")), "{:?}", import.notes);
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
    assert_eq!(shape(&twice.document), [r#"p "Before after""#, r#"figure asset [] """#, r#"figure asset [] """#]);
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
fn reads_a_caption_as_its_pictures_text_and_its_label_as_a_number() {
    let shot = picture(r#"r:embed="png""#, "A chart");
    let alone = format!("<w:p>{shot}</w:p>");
    let caption = |inner: &str| para(r#"<w:pStyle w:val="Cap"/>"#, inner);
    let count = |number: &str| simple_field(r" SEQ Figure \* ARABIC ", number);
    let cases: &[(String, &[&str])] = &[
        (alone.clone() + &caption(&t("Sales by year")), &[r#"figure asset [A chart] "Sales by year""#]),
        // Word counts figures with a field, and the page counts them itself.
        (
            alone.clone() + &caption(&(t("Figure ") + &count("3") + &t(": Sales"))),
            &[r##"figure# asset [A chart] "Sales""##],
        ),
        (
            alone.clone()
                + &caption(&(t("Abbildung ") + &field(r" SEQ Abbildung \* ARABIC ", "2") + &t(" \u{2013} Umsatz"))),
            &[r##"figure# asset [A chart] "Umsatz""##],
        ),
        (alone.clone() + &caption(&(t("Figure ") + &count("1"))), &[r##"figure# asset [A chart] """##]),
        // A label typed by hand reads the same, and a sentence that opens like one does not.
        (
            alone.clone() + &caption(&(t("Figure 12. ") + &run("<w:i/>", "Sales"))),
            &[r##"figure# asset [A chart] "<italic|Sales>""##],
        ),
        (
            alone.clone() + &caption(&t("Figure 8 shows the lot")),
            &[r#"figure asset [A chart] "Figure 8 shows the lot""#],
        ),
        // Pictures that share a paragraph stand side by side over one caption.
        (
            format!("<w:p>{shot}{shot}</w:p>") + &caption(&(t("Figure ") + &count("1") + &t(": Both"))),
            &["gallery#", r#"  figure asset [A chart] """#, r#"  figure asset [A chart] """#, r#"  caption "Both""#],
        ),
        // A caption with no picture over it is a line of its own, as is a second one.
        (p("Words") + &caption(&t("Figure 1: Loose")), &[r#"p "Words""#, r#"caption "Figure 1: Loose""#]),
        (
            alone.clone() + &caption(&t("One")) + &caption(&t("Two")),
            &[r#"figure asset [A chart] "One""#, r#"caption "Two""#],
        ),
    ];
    for (body, expected) in cases {
        let import = read(&word(body).picture("png", PNG));
        assert_eq!(shape(&import.document), *expected, "reading {body}");
        assert_eq!(import.notes, [], "reading {body}");
    }
}

#[test]
fn reads_a_cross_reference_to_a_caption_as_a_reference_to_its_figure() {
    let label = simple_field(r" SEQ Figure \* ARABIC ", "1");
    let marked =
        format!(r#"<w:bookmarkStart w:id="0" w:name="_Ref42"/>{}{label}<w:bookmarkEnd w:id="0"/>"#, t("Figure "));
    let captioned = format!("<w:p>{}</w:p>", picture(r#"r:embed="png""#, ""))
        + &para(r#"<w:pStyle w:val="Cap"/>"#, &(marked + &t(": Sales")));
    let to_figure: &[(&str, &str)] = &[("Figure 1", "Sales")];
    let cases = [
        (simple_field(r" REF _Ref42 \h ", "Figure 1"), to_figure),
        (field(r" REF _Ref42 \h ", "Figure 1"), to_figure),
        // One to what is no figure, or one that shows a page, stays the words Word last showed.
        (simple_field(r" REF _Ref7 \h ", "Figure 1"), &[]),
        (field(r" PAGEREF _Ref42 \h ", "Figure 1"), &[]),
    ];
    for (reference, expected) in cases {
        // A reference can come before what it points at.
        let body = format!("<w:p>{}{reference}{}</w:p>{captioned}", t("See "), t(" for more."));
        let import = read(&word(&body).picture("png", PNG));
        let found = references(&import.document);
        let found: Vec<(&str, &str)> = found.iter().map(|(text, to)| (text.as_str(), to.as_str())).collect();
        assert_eq!(found, expected, "reading {reference}");
        assert_eq!(import.document.blocks[0].plain_text(), "See Figure 1 for more.", "reading {reference}");
    }
}

#[test]
fn reads_tables() {
    let cell = |props: &str, inner: &str| format!("<w:tc><w:tcPr>{props}</w:tcPr>{inner}</w:tc>");
    let plain = |text: &str| format!("<w:tc>{}</w:tc>", p(text));
    let set = |align: &str, text: &str| cell("", &para(&format!(r#"<w:jc w:val="{align}"/>"#), &t(text)));
    let row = |props: &str, cells: &str| format!("<w:tr><w:trPr>{props}</w:trPr>{cells}</w:tr>");
    let table = |props: &str, rows: &str| format!("<w:tbl><w:tblPr>{props}</w:tblPr>{rows}</w:tbl>");
    let people = |heads: &str| row(heads, &(plain("Name") + &plain("Age"))) + &row("", &(plain("Ada") + &plain("36")));
    let read_people =
        [r#"  row"#, r#"    cell "Name""#, r#"    cell "Age""#, r#"  row"#, r#"    cell "Ada""#, r#"    cell "36""#];
    let headed = |first: &'static str| [&[first][..], &read_people[..]].concat();
    let picture = format!("<w:p>{}</w:p>", picture(r#"r:embed="png""#, ""));
    // The body, the document it reads as, and a word of each note the import makes.
    let cases: &[(String, Vec<&str>, &[&str])] = &[
        (table("", &people("<w:tblHeader/>")), headed("table:start,start headed"), &[]),
        (table("", &people("")), headed("table:start,start"), &[]),
        // A style that sets a first row apart heads the tables that ask for that look.
        (
            table(r#"<w:tblStyle w:val="Grid"/><w:tblLook w:firstRow="1"/>"#, &people("")),
            headed("table:start,start headed"),
            &[],
        ),
        (
            table(r#"<w:tblStyle w:val="Grid"/><w:tblLook w:val="0400"/>"#, &people("")),
            headed("table:start,start"),
            &[],
        ),
        // The page sets a row that heads a table itself, so what the style of its cells adds is no mark.
        (
            table("", &row("<w:tblHeader/>", &(cell("", &styled("Head", "Name")) + &cell("", &styled("Head", "Age"))))),
            vec!["table:start,start headed", "  row", r#"    cell "Name""#, r#"    cell "Age""#],
            &[],
        ),
        // A column is set as the first of its cells to say so is.
        (
            table(
                "",
                &(row("", &(plain("a") + &plain("b") + &set("right", "c")))
                    + &row("", &(plain("d") + &set("center", "e") + &set("left", "f")))),
            ),
            vec![
                "table:start,center,end",
                "  row",
                r#"    cell "a""#,
                r#"    cell "b""#,
                r#"    cell "c""#,
                "  row",
                r#"    cell "d""#,
                r#"    cell "e""#,
                r#"    cell "f""#,
            ],
            &[],
        ),
        // A cell that spans columns is the first of them, and the rest are empty.
        (
            table(
                "",
                &(row("", &(cell(r#"<w:gridSpan w:val="2"/>"#, &p("Wide")) + &plain("c")))
                    + &row("", &(plain("a") + &plain("b") + &plain("c")))),
            ),
            vec![
                "table:start,start,start",
                "  row",
                r#"    cell "Wide""#,
                r#"    cell """#,
                r#"    cell "c""#,
                "  row",
                r#"    cell "a""#,
                r#"    cell "b""#,
                r#"    cell "c""#,
            ],
            &["Merged"],
        ),
        // A cell holds text alone: its paragraphs are lines of it, and so is whatever else it held.
        (
            table(
                "",
                &row(
                    "",
                    &(cell("", &(p("Ada") + &p("Lovelace")))
                        + &cell("", &(styled("H2", "Head") + &item(1, 0, "point")))),
                ),
            ),
            vec!["table:start,start", "  row", r#"    cell "Ada\nLovelace""#, r#"    cell "Head\npoint""#],
            &["Lists, headings and tables"],
        ),
        (
            table("", &row("", &(plain("a") + &cell("", &picture)))),
            vec!["table:start,start", "  row", r#"    cell "a""#, r#"    cell """#, r#"figure asset [] """#],
            &["out of its table"],
        ),
        // A table of one column only frames what it holds.
        (
            table("", &(row("", &plain("One")) + &row("", &plain("Two")))),
            vec![r#"p "One""#, r#"p "Two""#],
            &["one column"],
        ),
    ];
    for (body, expected, notes) in cases {
        let import = read(&word(body).picture("png", PNG));
        assert_eq!(shape(&import.document), *expected, "reading {body}");
        let said: Vec<&str> = import.notes.iter().map(|note| note.message.as_str()).collect();
        assert_eq!(said.len(), notes.len(), "reading {body}: {said:?}");
        for note in *notes {
            assert!(said.iter().any(|message| message.contains(note)), "no note of {note:?} reading {body}: {said:?}");
        }
    }
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

/// A document as a Word file, with the same picture for every asset it names.
fn export(doc: &Document) -> Vec<u8> {
    to_docx(doc, &Style::default(), &Notes::default(), "Report", &mut |_| Some(PNG.to_vec()))
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

/// A picture 120 points by 80 over a caption, which shows an asset or what is at an address.
fn figure(id: &str, shows: &str, alt: &str, caption: &str) -> Block {
    let (asset, src) = match shows.starts_with("https:") {
        true => (None, Some(shows.to_owned())),
        false => (Some(shows.to_owned()), None),
    };
    let kind = BlockKind::Figure { asset, src, alt: alt.into(), width: 120.0, height: 80.0, numbered: false };
    Block::text(id, kind, caption)
}

/// A block that holds others.
fn holding(id: &str, kind: BlockKind, blocks: Vec<Block>) -> Block {
    Block { body: Body::Blocks(blocks), ..Block::new(id, kind) }
}

/// A table of plain text, a slice of cells for each row.
fn table(id: &str, columns: &[Align], header: bool, rows: &[&[&str]]) -> Block {
    let row = |(index, cells): (usize, &&[&str])| {
        let cell = |(column, text): (usize, &&str)| {
            Block::text(format!("{id}-{index}-{column}").as_str(), BlockKind::TableCell, *text)
        };
        holding(&format!("{id}-{index}"), BlockKind::TableRow, cells.iter().enumerate().map(cell).collect())
    };
    let kind = BlockKind::Table { columns: columns.to_vec(), header };
    holding(id, kind, rows.iter().enumerate().map(row).collect())
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
fn page_breaks_and_languages_survive_a_round_trip() {
    let text = |kind: BlockKind, text: &str| Block::text("", kind, text);
    let turning = |block: Block| Block { page_break: true, ..block };
    let in_lang = |lang: &str, block: Block| Block { lang: Some(lang.into()), ..block };
    let item = BlockKind::ListItem { list: ListKind::Ordered, depth: 0 };
    let mixed = vec![Run::plain("The word "), Run { lang: Some("de".into()), ..Run::marked("Wort", [Mark::Bold]) }];
    let quote = Block {
        body: Body::Blocks(vec![text(BlockKind::Paragraph, "Hola"), turning(text(item.clone(), "Mundo"))]),
        ..in_lang("es", Block::new("", BlockKind::Quote))
    };
    let blocks = vec![
        text(BlockKind::Paragraph, "Hello there, everyone"),
        turning(text(BlockKind::Heading { level: 1 }, "Chapter two")),
        in_lang("fr-FR", text(BlockKind::Paragraph, "Bonjour")),
        in_lang("ar", text(BlockKind::Paragraph, "مرحبا بالعالم")),
        paragraph("", mixed),
        turning(quote),
        turning(in_lang("fr-FR", text(item, "Un"))),
        text(BlockKind::Code { language: None }, "one()"),
        turning(text(BlockKind::Code { language: None }, "two()")),
        turning(Block::new("", BlockKind::Divider)),
    ];
    let doc = Document { blocks, language: "en-AU".into(), ..Document::default() };
    let back = returned(&doc).document;

    assert_eq!(outline(&back), outline(&doc));
    assert_eq!(turned(&back), turned(&doc));
    assert_eq!(tongues(&back), tongues(&doc));
    assert_eq!(back.language, "en-AU");
}

/// Formulas of every structure Word has one for.
const FORMULAS: &[&str] = &[
    "x=1",
    r"E = mc^2",
    r"x_{i_j}^{2^k} + {}_a^b C",
    r"\frac{-b \pm \sqrt{b^2 - 4ac}}{2a}",
    r"\sqrt[3]{x} + \binom{n}{k}",
    r"\sum_{i=1}^n i = \frac{n(n+1)}{2}",
    r"\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}",
    r"\prod_{k=1}^n k \ne \bigcup_i A_i",
    r"\lim_{x \to 0} \frac{\sin x}{x} = 1",
    r"\sin^2\theta + \cos^2\theta",
    r"\left( \frac{a}{b} \right) + \left[ c \middle| d \right] + \left\langle e \right\rangle",
    r"\begin{pmatrix} a & b \\ c & d \end{pmatrix} \begin{vmatrix} 1 & 0 \\ 0 & 1 \end{vmatrix}",
    r"f(x) = \begin{cases} x & \text{if } x \ge 0 \\ -x & \text{otherwise} \end{cases}",
    r"\begin{aligned} a &= b + c \\ &= d \end{aligned}",
    r"\hat{x} + \vec{v} + \bar{z} + \dot{q} + \tilde{n} + \overline{AB} + \underline{c}",
    r"\overbrace{a+b}^{n} + \underbrace{c+d}_{m}",
    r"\mathbb{R} \subset \mathcal{C}, \mathbf{v} \in \mathfrak{g}, \mathrm{d}x",
    r"\alpha\beta\Gamma \le \infty \to \partial \nabla \cdot \times",
    r"a \quad b \; c",
    r"f'(x) + g''(x)",
    r"\boxed{x} + \cancel{y}",
    r"A \xrightarrow{f} B \xleftarrow[g]{h} C",
    r"\begin{array}{lr} a & b \\ c & d \end{array}",
    r"\left. \frac{df}{dx} \right|_{x=0} + \lfloor x \rfloor + \|v\|",
    r"\underbrace{1 + \cdots + 1}_{n \text{ times}} \textbf{ in all}",
    r"x \in \{1, 2\} \% \#",
];

#[test]
fn formulas_survive_a_round_trip() {
    for source in FORMULAS {
        let within = vec![Run::plain("Since "), Run::marked(*source, [Mark::Math]), Run::plain(", done.")];
        let blocks = vec![Block::text("", BlockKind::Formula, *source), paragraph("", within)];
        let back = returned(&Document { blocks, ..Document::default() });

        let read = formulas(&back.document);
        let [(true, alone), (false, within)] = read.as_slice() else {
            panic!("{source:?} returns as a block and in a line, got {read:?}")
        };
        assert_eq!(typeset(alone), typeset(source), "returning {source:?} as {alone:?}");
        assert_eq!(typeset(within), typeset(source), "returning {source:?} in a line as {within:?}");
        assert_eq!(back.document.blocks[1].plain_text(), format!("Since {within}, done."));
        assert_eq!(back.notes, [], "nothing of {source:?} is simplified");
    }
}

#[test]
fn formulas_keep_their_place_through_word() {
    let text = |kind: BlockKind, text: &str| Block::text("", kind, text);
    let maths = |source: &str| Run::marked(source, [Mark::Math]);
    let item = BlockKind::ListItem { list: ListKind::Bullet, depth: 0 };
    let linked = Run { link: Some("https://example.com/".into()), ..maths("c") };
    let quote = Block {
        body: Body::Blocks(vec![text(BlockKind::Paragraph, "Then"), text(BlockKind::Formula, "x^2")]),
        ..Block::new("", BlockKind::Quote)
    };
    let blocks = vec![
        text(BlockKind::Paragraph, "First"),
        Block { page_break: true, ..text(BlockKind::Formula, r"\frac{a}{b}") },
        Block { body: Body::Text(vec![maths("a")]), ..Block::new("", item) },
        quote,
        paragraph("", vec![linked]),
        paragraph("", vec![Run::marked("bold ", [Mark::Bold]), maths("b"), Run::plain(" plain")]),
    ];
    let doc = Document { blocks, ..Document::default() };
    let back = returned(&doc);

    assert_eq!(outline(&back.document), outline(&doc));
    assert_eq!(turned(&back.document), turned(&doc));
    assert_eq!(back.notes, []);
}

#[test]
fn a_formula_is_written_as_an_equation_and_one_with_a_mistake_as_its_source() {
    let within = |source: &str| paragraph("", vec![Run::plain("See "), Run::marked(source, [Mark::Math])]);
    let alone = |source: &str| Block::text("", BlockKind::Formula, source);
    let equations = |doc: &Document| {
        let xml = part(&export(doc), "word/document.xml");
        let tree = roxmltree::Document::parse(&xml).expect("the part is XML");
        let named = |name: &str| tree.descendants().filter(|node| node.tag_name().name() == name).count();
        (named("oMath"), named("oMathPara"), named("f"), xml.contains(r"\frac"))
    };

    let sound = Document { blocks: vec![within(r"\frac{a}{b}"), alone(r"\frac{a}{b}")], ..Document::default() };
    assert_eq!(equations(&sound), (2, 1, 2, false), "Word is given the fraction, not how it is typed");

    let mistaken = Document { blocks: vec![within(r"\frac{a"), alone(r"\frac{a")], ..Document::default() };
    assert_eq!(equations(&mistaken), (0, 0, 0, true));
    let texts: Vec<String> = returned(&mistaken).document.walk().map(Block::plain_text).collect();
    assert_eq!(texts, [r"See \frac{a", r"\frac{a"], "the source is not lost");
}

#[test]
fn a_table_of_contents_is_a_field_that_holds_the_lines_the_page_lists() {
    let heading = |id: &str, level: u8, text: &str| Block::text(id, BlockKind::Heading { level }, text);
    let blocks = vec![
        heading("title", 1, "Report"),
        Block::new("toc", BlockKind::TableOfContents { levels: 2 }),
        heading("a", 1, "Introduction"),
        heading("b", 2, "Scope & limits"),
        heading("c", 3, "A detail"),
        Block::text("p", BlockKind::Paragraph, "Text."),
        heading("d", 1, "Method"),
    ];
    let doc = Document { blocks, ..Document::default() };
    let body = part(&export(&doc), "word/document.xml");
    let xml = roxmltree::Document::parse(&body).expect("the body is XML");
    let named = |node: &roxmltree::Node, name: &str| node.tag_name().name() == name;
    let attribute = |node: &roxmltree::Node, name: &str| {
        node.attributes().find(|held| held.name() == name).map(|held| held.value().to_owned())
    };
    let said = |node: roxmltree::Node| -> String {
        node.descendants().filter(|held| named(held, "t")).filter_map(|held| held.text()).collect()
    };
    let styled = |para: &roxmltree::Node| {
        para.descendants().find(|held| named(held, "pStyle")).and_then(|style| attribute(&style, "val"))
    };
    let lines =
        xml.descendants().filter(|para| named(para, "p") && styled(para).is_some_and(|id| id.starts_with("TOC")));
    let lines: Vec<(String, String)> = lines.map(|para| (styled(&para).unwrap_or_default(), said(para))).collect();
    let marks: BTreeSet<String> = xml
        .descendants()
        .filter(|held| named(held, "bookmarkStart"))
        .filter_map(|mark| attribute(&mark, "name"))
        .collect();
    let leads: Vec<String> = xml
        .descendants()
        .filter(|held| named(held, "hyperlink"))
        .filter_map(|link| attribute(&link, "anchor"))
        .collect();
    let back = returned(&doc);

    // Word is asked for the same levels, and shows the page's lines until it renews them.
    assert!(body.contains(r#"TOC \o "1-2""#), "the field asks for two levels");
    let expected = [("TOC1", "Introduction"), ("TOC2", "Scope & limits"), ("TOC1", "Method")];
    assert_eq!(lines, expected.map(|(style, text)| (style.to_owned(), text.to_owned())));
    assert_eq!(leads.len(), 3, "each line leads somewhere");
    assert!(leads.iter().all(|lead| marks.contains(lead)), "{leads:?} lead to {marks:?}");
    assert_eq!(outline(&back.document), outline(&doc));
    assert_eq!(back.notes, []);
}

#[test]
fn a_link_preview_is_written_as_what_it_shows() {
    let preview = |title: Option<&str>, description: Option<&str>, image: Option<&str>| {
        let kind = BlockKind::LinkPreview {
            url: "https://example.com/post?id=1".into(),
            title: title.map(String::from),
            description: description.map(String::from),
            image: image.map(String::from),
        };
        Block::new("card", kind)
    };
    // What a preview holds, the lines it is written as, and whether its picture is.
    let cases = [
        (
            preview(Some("A post"), Some("What it is about."), Some("assets/card.png")),
            vec!["A post", "What it is about.", "example.com"],
            true,
        ),
        (preview(Some("A post"), None, None), vec!["A post", "example.com"], false),
        // One that was never fetched is its address.
        (preview(None, None, None), vec!["https://example.com/post?id=1"], false),
    ];
    for (card, lines, pictured) in cases {
        let doc = Document { blocks: vec![card], ..Document::default() };
        let file = to_docx(&doc, &Style::default(), &Notes::default(), "Report", &mut |_| Some(PIXELS.to_vec()));
        let back = import(&file, &Style::default()).expect("the export reads");
        let texts: Vec<String> = back.document.walk().map(Block::plain_text).filter(|text| !text.is_empty()).collect();
        let linked: Vec<&Run> =
            back.document.walk().flat_map(|block| block.body.runs()).filter(|run| run.link.is_some()).collect();

        assert_eq!(texts, lines);
        assert_eq!(pictures(&back.document).len(), usize::from(pictured), "{lines:?}");
        assert_eq!(linked.len(), 1, "{lines:?}");
        assert_eq!(
            (linked[0].text.as_str(), linked[0].link.as_deref()),
            (lines[0], Some("https://example.com/post?id=1"))
        );
        assert_eq!(back.notes, [], "{lines:?}");
    }
}

#[test]
fn a_link_in_text_says_where_it_leads_when_pointed_at() {
    let titled = |title: &str| Preview { title: Some(title.into()), ..Preview::default() };
    let long = "A long title. ".repeat(40);
    // The text of a link, what the document keeps of its page, and whether Word is given the title to show.
    let cases = [
        ("kept", Some(Preview { description: Some("What it is about.".into()), ..titled("A \"post\" & more") }), true),
        ("long", Some(titled(&long)), true),
        ("untitled", Some(Preview { description: Some("No title.".into()), ..Preview::default() }), false),
        ("unknown", None, false),
    ];
    let address = |text: &str| format!("https://example.com/{text}");
    let runs = cases.iter().map(|(text, ..)| Run { link: Some(address(text)), ..Run::plain(*text) }).collect();
    let previews = cases.iter().filter_map(|(text, preview, _)| Some((address(text), preview.clone()?))).collect();
    let doc = Document { previews, blocks: vec![paragraph("p", runs)], ..Document::default() };

    let body = part(&export(&doc), "word/document.xml");
    let xml = roxmltree::Document::parse(&body).expect("the body is XML");
    let links = xml.descendants().filter(|node| node.tag_name().name() == "hyperlink");
    let tips: BTreeMap<String, Option<&str>> = links
        .map(|link| {
            let text = link.descendants().filter(|node| node.tag_name().name() == "t").filter_map(|node| node.text());
            (text.collect(), link.attributes().find(|held| held.name() == "tooltip").map(|held| held.value()))
        })
        .collect();
    for (text, preview, shown) in &cases {
        let tip = tips[*text];
        assert_eq!(tip.is_some(), *shown, "{text}");
        let Some((tip, title)) = tip.zip(preview.as_ref().and_then(|preview| preview.title.as_deref())) else {
            continue;
        };
        // Word limits how long a ScreenTip may be, so a long title is cut short.
        let units = |text: &str| text.encode_utf16().count();
        assert!(units(tip) <= 255, "{text}: {tip:?}");
        let cut = title.starts_with(tip.trim_end_matches('\u{2026}'));
        assert!(if units(title) <= 255 { tip == title } else { cut }, "{text}: {tip:?}");
    }
    // The link itself is as it was.
    let back = returned(&doc);
    let runs = back.document.blocks[0].body.runs();
    let linked: Vec<(String, Option<String>)> = runs.iter().map(|run| (run.text.clone(), run.link.clone())).collect();
    let expected: Vec<(String, Option<String>)> =
        cases.iter().map(|(text, ..)| (text.to_string(), Some(address(text)))).collect();
    assert_eq!(linked, expected);
    assert_eq!(back.notes, []);
}

#[test]
fn text_that_reads_from_the_right_is_set_from_the_right() {
    // What a paragraph holds, whether Word sets it from the right, and the stretches Word is told read
    // from the right.
    let cases: &[(&str, bool, &[&str])] = &[
        ("Plain text, with 3 digits.", false, &[]),
        ("مرحبا بالعالم، 3 مرات.", true, &["مرحبا بالعالم، 3 مرات."]),
        ("The word مرحبا بالعالم means hello.", false, &["مرحبا بالعالم"]),
        ("שלום English שלום", true, &["שלום ", " שלום"]),
    ];
    for (text, turned, stretches) in cases {
        let doc = Document { blocks: vec![Block::text("p", BlockKind::Paragraph, *text)], ..Document::default() };
        let body = part(&export(&doc), "word/document.xml");
        let xml = roxmltree::Document::parse(&body).expect("the body is XML");
        let named = |node: &roxmltree::Node, name: &str| node.tag_name().name() == name;
        let rtl = xml.descendants().filter(|run| named(run, "r") && run.descendants().any(|held| named(&held, "rtl")));
        let said: Vec<String> = rtl
            .map(|run| run.descendants().filter(|held| named(held, "t")).filter_map(|held| held.text()).collect())
            .collect();
        let back = returned(&doc);

        assert_eq!(xml.descendants().any(|held| named(&held, "bidi")), *turned, "{text}");
        assert_eq!(said, *stretches, "{text}");
        assert_eq!(back.document.blocks[0].plain_text(), *text);
        assert_eq!(back.notes, [], "{text}");
    }
}

#[test]
fn a_callout_is_headed_as_on_the_page() {
    let callout = |id: &str, tone, title: Option<&str>| {
        let kind = BlockKind::Callout { tone, title: title.map(String::from), icon: Some("star".into()), color: None };
        holding(id, kind, vec![Block::text(format!("{id}-text").as_str(), BlockKind::Paragraph, "Mind the gap.")])
    };
    // A tone, the title a writer gave two callouts of it, and what heads each in Word and on the way back.
    let cases = [
        (Tone::Warn, None, vec!["Warning", "Warning"]),
        (Tone::Tip, Some("Before you start"), vec!["Before you start", "Before you start"]),
        // Two of one tone with no line to head them are still two.
        (Tone::Info, Some(""), vec!["", ""]),
    ];
    for (tone, title, heads) in cases {
        let blocks = vec![callout("a", tone, title), callout("b", tone, title)];
        let doc = Document { blocks, ..Document::default() };
        let body = part(&export(&doc), "word/document.xml");
        let back = returned(&doc);

        for head in heads.iter().filter(|head| !head.is_empty()) {
            assert_eq!(body.matches(&format!(">{head}<")).count(), 2, "{head:?} heads each callout in Word");
        }
        assert_eq!(headed(&back.document), heads, "{tone:?} {title:?}");
        assert_eq!(back.document.walk().filter(|block| block.plain_text() == "Mind the gap.").count(), 2);
        assert_eq!(back.notes, [], "{tone:?} {title:?}");
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
    for direct in ["<w:rFonts", "<w:sz ", "<w:color", "<w:jc", "<w:spacing"] {
        assert!(!body.contains(direct), "a paragraph carries {direct} of its own");
    }
    // Only an item of a list says how far in it sits, where a quote's frame would otherwise part from it.
    let set_in: Vec<&str> = body.split("</w:p>").filter(|paragraph| paragraph.contains("<w:ind")).collect();
    assert!(set_in.iter().all(|paragraph| paragraph.contains("<w:numPr>")), "{set_in:?}");
}

#[test]
fn the_parts_of_a_file_agree() {
    let mut doc = document(EVERYTHING);
    doc.blocks.push(figure("fig", "assets/chart.png", "A chart", "Sales"));
    doc.blocks.push(table("t", &[Align::Start, Align::End], true, &[&["Item", "Price"], &["Pens", "3.50"]]));
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
    doc.blocks.push(figure("held", "assets/chart.png", "A <chart>", "Sales & costs"));
    doc.blocks.push(figure("again", "assets/chart.png", "", ""));
    doc.blocks.push(figure("far", "https://example.com/p.png", "Far", ""));
    doc.blocks.push(figure("lost", "assets/missing.png", "Lost", "Its caption"));
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
fn figures_and_references_to_them_survive_a_round_trip() {
    let numbered = |mut block: Block| {
        if let BlockKind::Figure { numbered, .. } | BlockKind::Gallery { numbered } = &mut block.kind {
            *numbered = true;
        }
        block
    };
    let to = |text: &str, figure: &str| Run::marked(text, [Mark::Ref(BlockId::new(figure))]);
    let chart = "assets/chart.png";
    let pair = vec![
        figure("left", chart, "Left", ""),
        figure("right", chart, "Right", ""),
        Block::text("under", BlockKind::Caption, "Both years"),
    ];
    // A reference reads as the label of its figure, whatever its own text was when the figures last moved.
    let words = vec![
        Run::plain("See "),
        to("Figure 1", "one"),
        Run::plain(" and "),
        to("Figure 9", "pair"),
        Run::plain(", not "),
        to("Figure ?", "gone"),
        Run::plain(" or "),
        to("the plain one", "plain"),
        Run::plain("."),
    ];
    let blocks = vec![
        paragraph("words", words),
        numbered(figure("one", chart, "A chart", "Sales")),
        figure("plain", chart, "", "Not counted"),
        figure("bare", chart, "", ""),
        numbered(holding("pair", BlockKind::Gallery { numbered: false }, pair)),
        numbered(figure("last", chart, "", "")),
    ];
    let doc = Document { blocks, ..Document::default() };
    let on_caption = comment(Anchor::range(BlockId::new("one"), 0..5), "Ada", "Which year?");
    let notes = Notes { notes: vec![on_caption], ..Notes::default() };
    let file = to_docx(&doc, &Style::default(), &notes, "Report", &mut |_| Some(PNG.to_vec()));
    let back = import(&file, &Style::default()).expect("the export reads");

    let lines = shape(&back.document);
    assert_eq!(back.document.blocks[0].plain_text(), "See Figure 1 and Figure 2, not Figure ? or the plain one.");
    assert_eq!(
        lines[1..],
        [
            r#"figure# asset [A chart] "Sales""#,
            r#"figure asset [] "Not counted""#,
            r#"figure asset [] """#,
            "gallery#",
            r#"  figure asset [Left] """#,
            r#"  figure asset [Right] """#,
            r#"  caption "Both years""#,
            r#"figure# asset [] """#,
        ]
    );
    let read = references(&back.document);
    assert_eq!(read, [("Figure 1".to_owned(), "Sales".to_owned()), ("Figure 2".to_owned(), "Both years".to_owned())]);
    assert_eq!(anchored(&back.document, &back.comments[0].anchor), "Sales", "a comment stays on the caption's words");
    assert_eq!(back.notes, []);

    // Word keeps the numbers up itself: it counts each label with a field, and each reference names a label.
    let body = part(&file, "word/document.xml");
    let xml = roxmltree::Document::parse(&body).expect("the document is XML");
    let codes = xml.descendants().filter(|node| node.tag_name().name() == "instrText");
    let fields: Vec<&str> = codes.filter_map(|code| code.text()).map(str::trim).collect();
    let named = |name: &'static str| fields.iter().filter_map(move |code| code.strip_prefix(name));
    let labels: BTreeSet<String> = named("REF ").filter_map(|rest| rest.split(' ').next().map(str::to_owned)).collect();
    assert!(labels.len() == 2 && labels.is_subset(&values(&body, "bookmarkStart", "name")), "{fields:?}");
    let sequences: BTreeSet<&str> = named("SEQ ").collect();
    assert_eq!((named("SEQ ").count(), sequences.len()), (3, 1), "each figure is counted in one sequence: {fields:?}");
}

#[test]
fn a_gallery_is_written_in_the_rows_the_page_sets() {
    let page = Style::default().page;
    let measure = page.width - page.margin_left - page.margin_right;
    // How many pictures a gallery holds, and how many share a row with each of them.
    let cases: &[(usize, &[usize])] =
        &[(2, &[2, 2]), (3, &[3, 3, 3]), (4, &[2, 2, 2, 2]), (5, &[3, 3, 3, 2, 2]), (7, &[3, 3, 3, 2, 2, 2, 2])];
    for (count, expected) in cases {
        let tiles = (0..*count).map(|index| figure(&format!("f{index}"), "assets/chart.png", "", "")).collect();
        let gallery = holding("g", BlockKind::Gallery { numbered: false }, tiles);
        let back = returned(&Document { blocks: vec![gallery], ..Document::default() }).document;
        let beside = |block: &Block| match block.kind {
            BlockKind::Figure { width, .. } => Some((measure / width).round() as usize),
            _ => None,
        };
        assert_eq!(back.walk().filter_map(beside).collect::<Vec<_>>(), *expected, "{count} pictures");
        assert_eq!(outline(&back).first().map(String::as_str), Some("gallery"), "{count} pictures stay together");
    }
}

#[test]
fn tables_survive_a_round_trip() {
    let priced = &[&["Item", "Count", "Price"][..], &["Pens", "12", "3.50"], &["Ink"]];
    let turning = |block: Block| Block { page_break: true, ..block };
    let loose = [Block::text("x", BlockKind::TableCell, "A row"), Block::text("y", BlockKind::TableCell, "by itself")];
    let blocks = vec![
        Block::text("before", BlockKind::Paragraph, "Before"),
        table("t", &[Align::Start, Align::Center, Align::End], true, priced),
        // Two tables that touch stay two, and one can start a page.
        turning(table("u", &[Align::Justify, Align::Start], false, &[&["a", "b"], &["two\nlines", ""]])),
        // A row that a merge left outside its table is a table of one row.
        holding("r", BlockKind::TableRow, loose.to_vec()),
        Block::text("after", BlockKind::Paragraph, "After"),
    ];
    let doc = Document { blocks, ..Document::default() };
    let on_cell = comment(Anchor::range(BlockId::new("t-1-0"), 0..4), "Ada", "Which pens?");
    let notes = Notes { notes: vec![on_cell], ..Notes::default() };
    let file = to_docx(&doc, &Style::default(), &notes, "Report", &mut |_| None);
    let back = import(&file, &Style::default()).expect("the export reads");

    let cells = |texts: &[&str]| {
        let cells = texts.iter().map(|text| format!("    cell {text:?}"));
        std::iter::once("  row".to_owned()).chain(cells).collect::<Vec<_>>()
    };
    let expected = [
        vec![r#"p "Before""#.to_owned(), "table:start,center,end headed".to_owned()],
        cells(&["Item", "Count", "Price"]),
        cells(&["Pens", "12", "3.50"]),
        // A row short of cells ends in empty ones.
        cells(&["Ink", "", ""]),
        vec!["table:justify,start".to_owned()],
        cells(&["a", "b"]),
        cells(&["two\nlines", ""]),
        vec!["table:start,start".to_owned()],
        cells(&["A row", "by itself"]),
        vec![r#"p "After""#.to_owned()],
    ];
    assert_eq!(outline(&back.document), expected.concat());
    let turned: Vec<&Block> = back.document.walk().filter(|block| block.page_break).collect();
    assert!(matches!(turned[..], [Block { kind: BlockKind::Table { header: false, .. }, .. }]), "{turned:?}");
    assert_eq!(anchored(&back.document, &back.comments[0].anchor), "Pens", "a comment stays on its cell");
    assert_eq!(back.notes, []);
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
    doc.blocks.push(figure("f", "assets/chart.png", hostile, hostile));
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
            "<", ">", "&", "\"", "'", "&amp;", "]]>", "<w:t>", "</w:p>", "\n", "\t", "  ", "é", "中", "🙂", "\u{a0}", "[^1]", "1.", "שלום", "مرحبا",
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
    // A callout headed by its tone's title, by one of its own, or by none.
    let title = proptest::sample::select(vec![None, Some(""), Some("Take care")]);
    let boxed = move |tone, title: Option<&str>| BlockKind::Callout {
        tone,
        title: title.map(String::from),
        icon: None,
        color: None,
    };
    let holding = |kind: BlockKind, blocks: Vec<Block>| holding("", kind, blocks);
    let inside = proptest::collection::vec(text.clone(), 1..4).prop_map(nested);
    // A picture with a caption or none, and pictures side by side over one or none.
    let caption = || proptest::option::of(any_runs());
    let shot = || figure("", "assets/chart.png", "", "");
    let pictured = caption().prop_map(move |runs| Block { body: Body::Text(runs.unwrap_or_default()), ..shot() });
    let gallery = (2..6usize, caption()).prop_map(move |(count, runs)| {
        let under = runs.map(|runs| with(BlockKind::Caption, runs));
        holding(BlockKind::Gallery { numbered: false }, (0..count).map(|_| shot()).chain(under).collect())
    });
    // A table whose rows may fall short of its columns, which Word fills out.
    let align = proptest::sample::select(vec![Align::Start, Align::Center, Align::End, Align::Justify]);
    let cells = proptest::collection::vec(any_runs().prop_map(move |runs| with(BlockKind::TableCell, runs)), 1..4);
    let rows = proptest::collection::vec(cells.prop_map(move |cells| holding(BlockKind::TableRow, cells)), 1..4);
    let table = (proptest::collection::vec(align, 2..4), any::<bool>(), rows)
        .prop_map(move |(columns, header, rows)| holding(BlockKind::Table { columns, header }, rows));
    let block = prop_oneof![
        6 => text,
        1 => pictured,
        1 => gallery,
        1 => table,
        1 => (1..=4u8, any_runs()).prop_map(move |(level, runs)| with(BlockKind::Heading { level }, runs)),
        1 => any_text().prop_map(move |code| with(BlockKind::Code { language: None }, vec![Run::plain(code.trim_end_matches('\n'))])),
        1 => Just(Block::new("", BlockKind::Divider)),
        1 => (1..=4u8).prop_map(|levels| Block::new("", BlockKind::TableOfContents { levels })),
        1 => inside.clone().prop_map(move |blocks| holding(BlockKind::Quote, blocks)),
        1 => (tone, title, inside).prop_map(move |(tone, title, blocks)| holding(boxed(tone, title), blocks)),
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
        // A row short of cells is filled out, and a cell past the last column that says how it is set is
        // set from its start.
        if let (BlockKind::Table { columns, .. }, Body::Blocks(rows)) = (&mut block.kind, &mut block.body) {
            let across = rows.iter().map(|row| row.body.blocks().len()).max().unwrap_or(0).max(columns.len());
            columns.resize(across, Align::Start);
            for row in rows.iter_mut() {
                if let Body::Blocks(cells) = &mut row.body {
                    cells.resize_with(across, || Block::new("", BlockKind::TableCell));
                }
            }
        }
        out.push(block);
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
        prop_assert_eq!(shape(&back.document), shape(&expected));
        prop_assert_eq!(headed(&back.document), headed(&expected));
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
        // A gallery holds pictures, a table rows of cells, and a reference is to a figure: what a file can hold.
        for outer in import.document.walk() {
            let strays: Vec<&Block> = outer.body.blocks().iter().filter(|part| !outer.kind.admits(&part.kind)).collect();
            prop_assert!(strays.is_empty(), "{:?} holds {:?}", outer.kind, strays);
        }
        for mark in import.document.walk().flat_map(|block| block.body.runs()).flat_map(|run| &run.marks) {
            if let Mark::Ref(to) = mark {
                let kind = import.document.block(to).map(|figure| &figure.kind);
                let pictured = matches!(kind, Some(BlockKind::Figure { .. } | BlockKind::Gallery { .. }));
                prop_assert!(pictured, "{:?} refers to {:?}", to, kind);
            }
        }
        let stored = Document::from_json(&import.document.to_json());
        prop_assert_eq!(stored.as_ref(), Ok(&import.document), "a file holds the document as it was read");
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
        simple_field(r" SEQ Figure \* ARABIC ", "1"),
        r#"<w:r><w:instrText> SEQ Figure </w:instrText></w:r>"#.to_owned(),
        r#"<w:bookmarkStart w:id="5" w:name="mark"/>"#.to_owned(),
        simple_field(r" REF mark \h ", "Figure 1"),
        r#"<w:r><w:instrText> REF mark </w:instrText></w:r>"#.to_owned(),
        "<m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>".to_owned(),
        format!("<m:oMathPara><m:oMath>{}</m:oMath><m:oMath/></m:oMathPara>", m("a&amp;{b}^_")),
        format!(
            "<m:oMath>{}<w:r><w:t>w</w:t></w:r><m:box><m:e><m:oMath>{}</m:oMath></m:e></m:box></m:oMath>",
            structure("f", "", &[("num", structure("eqArr", "", &[("e", m("a&amp;b"))])), ("den", String::new())]),
            structure("d", r#"<m:begChr m:val="⟦"/><m:endChr/>"#, &[("e", structure("rad", "", &[("deg", m("]"))]))]),
        ),
        format!("<m:oMath>{}</m:oMath>", (0..70).fold(m("z"), |inner, _| structure("sSup", "", &[("e", inner)]))),
        "<m:oMath><m:nary/><m:acc/><m:func/><m:m><m:mr/></m:m><m:groupChr/><m:sPre/><m:limUpp/><m:unknown/></m:oMath>"
            .to_owned(),
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
            (rows, any::<bool>()).prop_map(|(rows, merged)| {
                // Cells merged across and down, in rows that head the table and start past its first column.
                let (wide, heading) = match merged {
                    true => (r#"<w:tcPr><w:gridSpan w:val="70"/><w:vMerge/></w:tcPr>"#, "<w:tblHeader/>"),
                    false => ("", ""),
                };
                let row = |cells: &Vec<String>| {
                    cells.iter().map(|cell| format!("<w:tc>{wide}{cell}</w:tc>")).collect::<String>()
                };
                let rows = rows.iter().map(|cells| {
                    format!(r#"<w:tr><w:trPr>{heading}<w:gridBefore w:val="2"/></w:trPr>{}</w:tr>"#, row(cells))
                });
                format!("<w:tbl>{}</w:tbl>", rows.collect::<String>())
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
