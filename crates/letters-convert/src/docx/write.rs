//! Document to Word document.
//!
//! Every paragraph names a style and carries no formatting of its own, so changing a style in Word restyles
//! the document as it does here. Marks are the one thing written onto runs, as Word's own buttons would.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::ops::Range;

use letters_layout::Palette;
use letters_model::{Align, Block, BlockKind, Document, ListKind, Mark, NoteKind, Notes, Run, Style, TextStyle, Tone};
use letters_render::Color;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::math::equation;
use super::{CALLOUTS, DIVIDER, INLINE_CODE, LINK, LIST, SINGLE, TASK_DONE, TASK_OPEN, TWIPS, TYPES};
use crate::html::{Assets, href};

/// Exports are drawn on the light page, whatever theme the editor is in.
const PAGE: Palette = Palette::LIGHT;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const M: &str = "http://schemas.openxmlformats.org/officeDocument/2006/math";
const RELATIONSHIP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const DOCUMENT_TYPE: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml";
const HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

/// The lists every file defines, by the id paragraphs name them with. Numbered lists follow, one for each
/// place the page starts counting again.
const BULLETS: usize = 1;
const TASKS_OPEN: usize = 2;
const TASKS_DONE: usize = 3;
/// How far each level of a list sits in from the one above it, in twentieths of a point.
const LIST_INDENT: usize = 360;
/// The longest address Word follows. A longer one is written as its text alone.
const LONGEST_LINK: usize = 2000;
/// What starts a paragraph on a new page.
const BREAK: &str = "<w:pageBreakBefore/>";

/// Writes a `.docx` with `style` as its styles.
///
/// - Each element type becomes the Word style of the same name, and each block a paragraph of its style.
/// - Comments are written as Word comments on the same passages, with their replies. Import notes and
///   conflicts stay behind.
/// - `title` goes into the file's properties, and `assets` supplies the pictures.
/// - A formula is written as a Word equation, and as its source where it has a mistake in it.
/// - A link that would run script is written as its text alone, and an unknown block as its fallback text.
pub fn to_docx(doc: &Document, style: &Style, notes: &Notes, title: &str, assets: Assets) -> Vec<u8> {
    let page = &style.page;
    let mut writer = Writer {
        out: String::new(),
        links: Links::default(),
        media: Vec::new(),
        pictures: BTreeMap::new(),
        assets,
        numbered: 0,
        counters: Vec::new(),
        current: Vec::new(),
        anchors: BTreeMap::new(),
        pending: Vec::new(),
        measure: (page.width - page.margin_left - page.margin_right).max(1.0),
        breaks: false,
        lang: None,
    };
    let comments = writer.plan(doc, notes);
    writer.blocks(&doc.blocks, None);
    // Word wants a paragraph in every document, and comments on a block that wrote none need one to sit on.
    if doc.blocks.is_empty() || !writer.pending.is_empty() {
        writer.paragraph("", None, &[], Vec::new());
    }
    let twips = |points: f32| (points * TWIPS).round().max(0.0) as u32;
    let mut document = format!(
        "{HEAD}<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\" xmlns:m=\"{M}\"><w:body>{}<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/>\
         <w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"708\" w:footer=\"708\" \
         w:gutter=\"0\"/></w:sectPr></w:body></w:document>",
        writer.out,
        twips(page.width),
        twips(page.height),
        twips(page.margin_top),
        twips(page.margin_right),
        twips(page.margin_bottom),
        twips(page.margin_left),
    );
    document.push('\n');

    let body_links = std::mem::take(&mut writer.links);
    let comments_xml = (!comments.is_empty()).then(|| writer.comments(&comments));
    let comment_links = std::mem::take(&mut writer.links);

    // The parts beside the document, each with its type and the relationship that names it.
    let mut parts: Vec<(String, String, Vec<u8>)> = vec![
        ("styles.xml".into(), "styles".into(), styles(style, &doc.language).into_bytes()),
        ("numbering.xml".into(), "numbering".into(), numbering(writer.numbered).into_bytes()),
        ("settings.xml".into(), "settings".into(), settings(&doc.language).into_bytes()),
    ];
    if let Some((comments, threads)) = comments_xml {
        parts.push(("comments.xml".into(), "comments".into(), comments.into_bytes()));
        parts.push(("commentsExtended.xml".into(), "commentsExtended".into(), threads.into_bytes()));
    }

    let mut types = format!(
        "{HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>"
    );
    for (extension, kind) in [("png", "image/png"), ("jpg", "image/jpeg"), ("gif", "image/gif"), ("webp", "image/webp")]
    {
        if writer.media.iter().any(|(name, _)| name.ends_with(extension)) {
            let _ = write!(types, "<Default Extension=\"{extension}\" ContentType=\"{kind}\"/>");
        }
    }
    let _ =
        write!(types, "<Override PartName=\"/word/document.xml\" ContentType=\"{DOCUMENT_TYPE}.document.main+xml\"/>");
    let mut rels = String::new();
    for (index, (name, kind, _)) in parts.iter().enumerate() {
        let _ = write!(types, "<Override PartName=\"/word/{name}\" ContentType=\"{DOCUMENT_TYPE}.{kind}+xml\"/>");
        // Word's later additions name their relationships under another address.
        let family = if kind == "commentsExtended" {
            "http://schemas.microsoft.com/office/2011/relationships"
        } else {
            RELATIONSHIP
        };
        let _ = write!(rels, "<Relationship Id=\"rId{}\" Type=\"{family}/{kind}\" Target=\"{name}\"/>", index + 1);
    }
    types.push_str(concat!(
        "<Override PartName=\"/docProps/core.xml\" ",
        "ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
        "<Override PartName=\"/docProps/app.xml\" ",
        "ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/></Types>\n",
    ));

    let package = format!(
        "{HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" Type=\"{RELATIONSHIP}/officeDocument\" Target=\"word/document.xml\"/>\
         <Relationship Id=\"rId2\" \
         Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" \
         Target=\"docProps/core.xml\"/>\
         <Relationship Id=\"rId3\" Type=\"{RELATIONSHIP}/extended-properties\" Target=\"docProps/app.xml\"/>\
         </Relationships>\n"
    );
    let mut core = format!(
        "{HEAD}<cp:coreProperties \
         xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>"
    );
    escape(title, &mut core);
    core.push_str("</dc:title><dc:language>");
    escape(&doc.language, &mut core);
    core.push_str("</dc:language></cp:coreProperties>\n");
    let app = format!(
        "{HEAD}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">\
         <Application>Letters</Application></Properties>\n"
    );

    let mut entries: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), types.into_bytes()),
        ("_rels/.rels".into(), package.into_bytes()),
        ("docProps/core.xml".into(), core.into_bytes()),
        ("docProps/app.xml".into(), app.into_bytes()),
        ("word/document.xml".into(), document.into_bytes()),
        ("word/_rels/document.xml.rels".into(), relationships(&rels, &body_links).into_bytes()),
    ];
    if !comment_links.targets.is_empty() {
        entries.push(("word/_rels/comments.xml.rels".into(), relationships("", &comment_links).into_bytes()));
    }
    entries.extend(parts.into_iter().map(|(name, _, bytes)| (format!("word/{name}"), bytes)));
    entries.extend(writer.media.into_iter().map(|(name, bytes)| (format!("word/{name}"), bytes)));

    // A fixed timestamp keeps the output reproducible.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());
    let deflated = stored.compression_method(CompressionMethod::Deflated);
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        // Pictures are compressed already.
        let options = if name.starts_with("word/media/") { stored } else { deflated };
        zip.start_file(name, options).expect("an entry starts in memory");
        zip.write_all(&bytes).expect("an entry is written in memory");
    }
    zip.finish().expect("the file ends in memory").into_inner()
}

/// Writes text so that XML reads it as text, between tags or inside a quoted attribute. Characters that XML
/// cannot hold are left out, since Word refuses a file with one.
fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#9;"),
            '\n' => out.push_str("&#10;"),
            '\r' | '\u{fffe}' | '\u{ffff}' => {}
            c if c < ' ' => {}
            c => out.push(c),
        }
    }
}

fn escaped(text: &str) -> String {
    let mut out = String::new();
    escape(text, &mut out);
    out
}

/// A colour as Word writes one. Word has no translucent fills, so one is laid over the white page first.
fn hex(color: Color) -> String {
    let over = |channel: u8| {
        let alpha = u32::from(color.a);
        ((u32::from(channel) * alpha + 255 * (255 - alpha) + 127) / 255) as u8
    };
    format!("{:02X}{:02X}{:02X}", over(color.r), over(color.g), over(color.b))
}

/// The addresses one part links to, each with the number of its relationship.
#[derive(Default)]
struct Links {
    targets: Vec<(String, &'static str)>,
    known: BTreeMap<String, usize>,
}

impl Links {
    /// How far the ids of the part's fixed relationships run.
    const FIXED: usize = 8;

    fn id(&mut self, target: &str, kind: &'static str) -> String {
        let next = self.targets.len();
        let index = *self.known.entry(target.to_owned()).or_insert_with(|| {
            self.targets.push((target.to_owned(), kind));
            next
        });
        format!("rId{}", index + 1 + Self::FIXED)
    }
}

/// A part's relationships: those written out in `fixed`, then its links and pictures.
fn relationships(fixed: &str, links: &Links) -> String {
    let mut out =
        format!("{HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{fixed}");
    for (index, (target, kind)) in links.targets.iter().enumerate() {
        let mode = if *kind == "hyperlink" { " TargetMode=\"External\"" } else { "" };
        let _ = write!(
            out,
            "<Relationship Id=\"rId{}\" Type=\"{RELATIONSHIP}/{kind}\" Target=\"{}\"{mode}/>",
            index + 1 + Links::FIXED,
            escaped(target)
        );
    }
    out.push_str("</Relationships>\n");
    out
}

/// A link's address as Word will take it, or `None` for one to write as text alone.
///
/// Word refuses the whole file over an address it cannot parse, so anything outside the characters an address
/// is made of is percent-encoded.
fn address(url: &str) -> Option<String> {
    let url = href(url)?;
    let mut out = String::new();
    for byte in url.bytes() {
        let plain = byte.is_ascii_alphanumeric() || b"-._~:/?#@!$&'()*+,;=%".contains(&byte);
        if plain {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    (out.len() <= LONGEST_LINK).then_some(out)
}

/// One Word comment: a note, or a reply to the one at `parent`.
struct Comment<'n> {
    author: &'n str,
    created: &'n str,
    content: &'n [Run],
    parent: Option<usize>,
    done: bool,
}

/// A passage of a block and the comments that sit on it. No passage is the whole block.
type Passage = (Option<Range<usize>>, Range<usize>);

struct Writer<'d, 'a> {
    /// The document's body so far.
    out: String,
    links: Links,
    /// Pictures by the name of their part.
    media: Vec<(String, Vec<u8>)>,
    /// The relationship each asset already written goes by.
    pictures: BTreeMap<&'d str, Option<String>>,
    assets: Assets<'a>,
    /// How many numbered lists have started.
    numbered: usize,
    /// The numbers the page gives the list being written, level by level.
    counters: Vec<u32>,
    /// The Word list behind each level of it.
    current: Vec<usize>,
    /// The comments on each block.
    anchors: BTreeMap<&'d str, Vec<Passage>>,
    /// Comments on a whole block, waiting for its first paragraph.
    pending: Vec<Range<usize>>,
    /// The width of the text column in points, which no picture is drawn wider than.
    measure: f32,
    /// Whether the next paragraph starts a new page.
    breaks: bool,
    /// The language of the block being written, where it is not the document's.
    lang: Option<&'d str>,
}

impl<'d> Writer<'d, '_> {
    /// Lists the comments to write and notes where each sits. A reply follows the comment it answers.
    fn plan(&mut self, doc: &'d Document, notes: &'d Notes) -> Vec<Comment<'d>> {
        let mut comments = Vec::new();
        for note in notes.notes.iter().filter(|note| note.kind == NoteKind::Comment) {
            let Some(block) = doc.block(&note.anchor.block) else { continue };
            let first = comments.len();
            let comment =
                |author, created, content, parent| Comment { author, created, content, parent, done: note.resolved };
            comments.push(comment(&note.author, &note.created, &note.content, None));
            for reply in &note.replies {
                comments.push(comment(&reply.author, &reply.created, &reply.content, Some(first)));
            }
            let spot = (note.anchor.range.clone(), first..comments.len());
            self.anchors.entry(block.id.as_str()).or_default().push(spot);
        }
        comments
    }

    /// Writes sibling blocks. `within` is the style of the quote or callout around them.
    fn blocks(&mut self, blocks: &'d [Block], within: Option<&'static str>) {
        let around = self.lang;
        for block in blocks {
            // The first paragraph a block writes carries its page break, and all of them its language.
            self.breaks |= block.page_break;
            self.lang = block.lang.as_deref().or(around);
            let mut spans = Vec::new();
            for (range, comments) in self.anchors.remove(block.id.as_str()).unwrap_or_default() {
                // A passage of a formula's source is no passage of the equation it is written as.
                match range.filter(|_| !block.body.runs().is_empty() && block.kind != BlockKind::Formula) {
                    Some(range) => spans.push((range, comments)),
                    None => self.pending.push(comments),
                }
            }
            if !matches!(block.kind, BlockKind::ListItem { .. }) {
                self.counters.clear();
            }
            match &block.kind {
                BlockKind::Paragraph => self.paragraph(within.unwrap_or(""), None, block.body.runs(), spans),
                BlockKind::Heading { level } => {
                    let style = TYPES[usize::from((*level).clamp(1, 4))].1;
                    self.paragraph(style, None, block.body.runs(), spans);
                }
                BlockKind::ListItem { list, depth } => {
                    let depth = (*depth).min(8);
                    let list = self.list(*list, usize::from(depth));
                    self.paragraph(within.unwrap_or(LIST.0), Some((list, depth)), block.body.runs(), spans);
                }
                BlockKind::Code { .. } => self.paragraph("Code", None, block.body.runs(), spans),
                BlockKind::Formula => match equation(&block.plain_text(), true) {
                    Some(equation) => {
                        self.open(within.unwrap_or(""), None);
                        self.whole(&format!("<m:oMathPara>{equation}</m:oMathPara>"));
                        self.out.push_str("</w:p>");
                    }
                    // A source with a mistake in it goes out as it is, so that it is not lost.
                    None => self.paragraph("Code", None, block.body.runs(), spans),
                },
                BlockKind::Quote => self.container(block, "Quote"),
                BlockKind::Callout { tone } => {
                    let style = CALLOUTS.iter().find(|(known, ..)| known == tone).map_or("Quote", |(_, id, _)| *id);
                    self.container(block, style);
                }
                BlockKind::Divider => self.paragraph(DIVIDER.0, None, &[], Vec::new()),
                BlockKind::Figure { .. } => self.figure(block),
                // Until galleries and tables are written as such, what they hold is written in order.
                BlockKind::Caption => self.paragraph("Caption", None, block.body.runs(), spans),
                BlockKind::TableCell => self.paragraph(within.unwrap_or(""), None, block.body.runs(), spans),
                BlockKind::Gallery { .. } | BlockKind::Table { .. } | BlockKind::TableRow => {
                    self.blocks(block.body.blocks(), within);
                }
                // A reader that does not build figures yet hands one over as an unknown block, its fields beside it.
                BlockKind::Unknown { type_name } if type_name == "figure" => self.figure(block),
                BlockKind::Unknown { .. } => {
                    let fallback =
                        block.extra.get("fallback").and_then(|fallback| fallback.as_str()).unwrap_or_default();
                    if !fallback.is_empty() {
                        self.paragraph("Caption", None, &[Run::plain(fallback)], Vec::new());
                    }
                }
            }
        }
        self.lang = around;
    }

    /// Writes what a quote or a callout holds. Word has no block that holds others, so each paragraph inside
    /// takes the container's style, and Word draws neighbours of one style as one box.
    fn container(&mut self, block: &'d Block, style: &'static str) {
        let inner = block.body.blocks();
        if inner.is_empty() {
            self.paragraph(style, None, &[], Vec::new());
        }
        self.blocks(inner, Some(style));
        self.counters.clear();
    }

    /// The Word list an item belongs to. A numbered item starts a new one wherever the page counts from 1.
    fn list(&mut self, list: ListKind, depth: usize) -> usize {
        self.counters.resize(depth + 1, 0);
        self.current.resize(depth + 1, 0);
        match list {
            ListKind::Bullet => {
                self.counters[depth] = 0;
                BULLETS
            }
            ListKind::Task { checked } => {
                self.counters[depth] = 0;
                if checked { TASKS_DONE } else { TASKS_OPEN }
            }
            ListKind::Ordered => {
                self.counters[depth] += 1;
                if self.counters[depth] == 1 {
                    self.numbered += 1;
                    self.current[depth] = TASKS_DONE + self.numbered;
                }
                self.current[depth]
            }
        }
    }

    /// Starts a paragraph of a style, which takes the page break that waits for one.
    fn open(&mut self, style: &str, list: Option<(usize, u8)>) {
        self.out.push_str("<w:p>");
        let breaks = std::mem::take(&mut self.breaks);
        if !style.is_empty() || list.is_some() || breaks {
            self.out.push_str("<w:pPr>");
            if !style.is_empty() {
                let _ = write!(self.out, "<w:pStyle w:val=\"{style}\"/>");
            }
            if breaks {
                self.out.push_str(BREAK);
            }
            if let Some((list, depth)) = list {
                let _ = write!(self.out, "<w:numPr><w:ilvl w:val=\"{depth}\"/><w:numId w:val=\"{list}\"/></w:numPr>");
            }
            self.out.push_str("</w:pPr>");
        }
    }

    /// Writes one paragraph of a style, with the comments that sit on it.
    fn paragraph(
        &mut self,
        style: &str,
        list: Option<(usize, u8)>,
        runs: &[Run],
        mut spans: Vec<(Range<usize>, Range<usize>)>,
    ) {
        self.open(style, list);
        let len: usize = runs.iter().map(|run| run.text.len()).sum();
        spans.extend(self.pending.drain(..).map(|comments| (0..len, comments)));
        let mut out = std::mem::take(&mut self.out);
        inline(runs, &spans, self.lang, &mut self.links, &mut out);
        self.out = out;
        self.out.push_str("</w:p>");
    }

    /// Writes a picture as a paragraph of its own, and its caption as the paragraph after.
    fn figure(&mut self, block: &'d Block) {
        let field = |name: &str| block.extra.get(name);
        let text = |name: &str| field(name).and_then(|value| value.as_str());
        let size = |name: &str| field(name).and_then(|value| value.as_f64()).unwrap_or_default() as f32;
        let (asset, src, alt, width, height, caption) = match &block.kind {
            BlockKind::Figure { asset, src, alt, width, height, .. } => {
                (asset.as_deref(), src.as_deref(), alt.as_str(), *width, *height, block.body.runs().to_vec())
            }
            _ => {
                let caption = field("content").and_then(|runs| Run::list_from_value(runs.clone(), "content").ok());
                let alt = text("alt").unwrap_or_default();
                (text("asset"), text("src"), alt, size("width"), size("height"), caption.unwrap_or_default())
            }
        };
        let (mut width, mut height) = (width.max(1.0), height.max(1.0));
        if width > self.measure {
            (width, height) = (self.measure, height * self.measure / width);
        }

        let held = asset.and_then(|name| match self.pictures.get(name) {
            Some(known) => known.clone(),
            None => {
                let id = self.embed(name);
                self.pictures.insert(name, id.clone());
                id
            }
        });
        match (held, src.and_then(address)) {
            (Some(id), _) => {
                let number = self.out.matches("<wp:docPr ").count() + 1;
                let emu = |points: f32| (points * 12700.0).round() as u64;
                let (cx, cy) = (emu(width), emu(height));
                let alt = escaped(alt);
                let picture = format!(
                    "<w:r><w:drawing><wp:inline \
                     xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
                     distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/>\
                     <wp:docPr id=\"{number}\" name=\"Picture {number}\" descr=\"{alt}\"/>\
                     <a:graphic xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">\
                     <a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
                     <pic:pic xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\">\
                     <pic:nvPicPr><pic:cNvPr id=\"{number}\" name=\"Picture {number}\" descr=\"{alt}\"/><pic:cNvPicPr/>\
                     </pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{id}\"/><a:stretch><a:fillRect/></a:stretch>\
                     </pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
                     <a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData>\
                     </a:graphic></wp:inline></w:drawing></w:r>"
                );
                let breaks = if std::mem::take(&mut self.breaks) { BREAK } else { "" };
                let _ = write!(self.out, "<w:p><w:pPr><w:keepNext/>{breaks}</w:pPr>");
                self.whole(&picture);
                self.out.push_str("</w:p>");
            }
            // A picture that is still remote is a link to it, so that opening the file fetches nothing.
            (None, Some(url)) => {
                let label = if alt.is_empty() { url.as_str() } else { alt };
                let link = Run { link: Some(url.clone()), ..Run::plain(label) };
                self.paragraph("", None, &[link], Vec::new());
            }
            (None, None) if !alt.is_empty() => self.paragraph("Caption", None, &[Run::plain(alt)], Vec::new()),
            (None, None) => {}
        }
        if caption.iter().any(|run| !run.text.is_empty()) {
            self.paragraph("Caption", None, &caption, Vec::new());
        }
    }

    /// Writes content that is not text into the open paragraph, inside the comments that wait for one.
    fn whole(&mut self, content: &str) {
        let spans: Vec<_> = self.pending.drain(..).map(|comments| (0..0, comments)).collect();
        for id in spans.iter().flat_map(|(_, comments)| comments.clone()) {
            let _ = write!(self.out, "<w:commentRangeStart w:id=\"{id}\"/>");
        }
        self.out.push_str(content);
        for id in spans.iter().flat_map(|(_, comments)| comments.clone()) {
            close(id, &mut self.out);
        }
    }

    /// Stores an asset as a part of the file and returns its relationship. `None` for one that is missing or
    /// that Word cannot show.
    fn embed(&mut self, name: &str) -> Option<String> {
        let bytes = (self.assets)(name)?;
        let extension = match bytes.as_slice() {
            [0x89, b'P', b'N', b'G', ..] => "png",
            [0xff, 0xd8, 0xff, ..] => "jpg",
            [b'G', b'I', b'F', b'8', ..] => "gif",
            [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
            _ => return None,
        };
        let part = format!("media/image{}.{extension}", self.media.len() + 1);
        let id = self.links.id(&part, "image");
        self.media.push((part, bytes));
        Some(id)
    }

    /// Writes the comments part and the part that threads replies under them.
    fn comments(&mut self, comments: &[Comment]) -> (String, String) {
        let mut out = format!(
            "{HEAD}<w:comments xmlns:w=\"{W}\" xmlns:r=\"{R}\" xmlns:m=\"{M}\" \
             xmlns:w14=\"http://schemas.microsoft.com/office/word/2010/wordml\" \
             xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"w14\">"
        );
        let mut threads = format!(
            "{HEAD}<w15:commentsEx xmlns:w15=\"http://schemas.microsoft.com/office/word/2012/wordml\" \
             xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"w15\">"
        );
        // A thread names a comment by the id of its paragraph.
        let paragraph = |index: usize| format!("{:08X}", index + 1);
        for (index, comment) in comments.iter().enumerate() {
            let initials: String =
                comment.author.split_whitespace().filter_map(|word| word.chars().next()).take(3).collect();
            let _ = write!(
                out,
                "<w:comment w:id=\"{index}\" w:author=\"{}\" w:date=\"{}\" w:initials=\"{}\"><w:p w14:paraId=\"{}\">",
                escaped(comment.author),
                escaped(comment.created),
                escaped(&initials),
                paragraph(index),
            );
            inline(comment.content, &[], None, &mut self.links, &mut out);
            out.push_str("</w:p></w:comment>");
            let _ = write!(threads, "<w15:commentEx w15:paraId=\"{}\"", paragraph(index));
            if let Some(parent) = comment.parent {
                let _ = write!(threads, " w15:paraIdParent=\"{}\"", paragraph(parent));
            }
            let _ = write!(threads, " w15:done=\"{}\"/>", u8::from(comment.done));
        }
        out.push_str("</w:comments>\n");
        threads.push_str("</w15:commentsEx>\n");
        (out, threads)
    }
}

/// Ends a comment's passage and leaves its mark, which is where Word hangs the comment.
fn close(id: usize, out: &mut String) {
    let _ = write!(out, "<w:commentRangeEnd w:id=\"{id}\"/><w:r><w:commentReference w:id=\"{id}\"/></w:r>");
}

/// Writes runs, with each comment's passage marked around its text. Offsets are bytes into the runs' text.
/// `lang` is the language of the block around them, where that is not the document's.
fn inline(
    runs: &[Run],
    spans: &[(Range<usize>, Range<usize>)],
    lang: Option<&str>,
    links: &mut Links,
    out: &mut String,
) {
    let len: usize = runs.iter().map(|run| run.text.len()).sum();
    // Where passages start and end. One that ends at a point is written before another starts there.
    let mut marks: Vec<(usize, u8, usize)> = Vec::new();
    for (range, comments) in spans {
        let (start, end) = (range.start.min(len), range.end.min(len).max(range.start.min(len)));
        for id in comments.clone() {
            marks.push((start, 1, id));
            marks.push((end, if end > start { 0 } else { 2 }, id));
        }
    }
    marks.sort();
    let mut marks = marks.into_iter().peekable();
    let mut mark = |upto: usize, out: &mut String| {
        while let Some((_, kind, id)) = marks.next_if(|(at, ..)| *at <= upto) {
            if kind == 1 {
                let _ = write!(out, "<w:commentRangeStart w:id=\"{id}\"/>");
            } else {
                close(id, out);
            }
        }
    };

    let cuts: Vec<usize> = spans.iter().flat_map(|(range, _)| [range.start, range.end]).collect();
    let mut at = 0;
    let mut open: Option<&str> = None;
    for run in runs {
        let maths = run.marks.contains(&Mark::Math).then(|| equation(&run.text, false)).flatten();
        let mut rest = run.text.as_str();
        while !rest.is_empty() {
            // The run is cut where a passage starts or ends inside it, never inside a character or a formula.
            let cut = cuts.iter().map(|cut| cut.saturating_sub(at)).filter(|cut| *cut > 0 && *cut < rest.len());
            let mut cut = cut.min().filter(|_| maths.is_none()).unwrap_or(rest.len());
            while !rest.is_char_boundary(cut) {
                cut += 1;
            }
            let (piece, after) = rest.split_at(cut);
            let link = run.link.as_deref();
            if open != link && open.take().is_some() {
                out.push_str("</w:hyperlink>");
            }
            mark(at, out);
            if open.is_none()
                && let Some(target) = link.and_then(address)
            {
                let _ = write!(out, "<w:hyperlink r:id=\"{}\">", links.id(&target, "hyperlink"));
                open = link;
            }
            match &maths {
                Some(equation) => out.push_str(equation),
                None => text(piece, run, open.is_some(), run.lang.as_deref().or(lang), out),
            }
            at += piece.len();
            rest = after;
        }
    }
    if open.is_some() {
        out.push_str("</w:hyperlink>");
    }
    mark(usize::MAX, out);
}

/// Writes one run of text with its marks, and its language where that is not the document's. Marks are
/// direct formatting, as Word's own buttons make.
fn text(text: &str, run: &Run, linked: bool, lang: Option<&str>, out: &mut String) {
    out.push_str("<w:r>");
    let has = |mark: Mark| run.marks.contains(&mark);
    // A formula comes here only as its source, when it has a mistake in it, and source reads as code.
    let code = has(Mark::Code) || has(Mark::Math);
    let style = if code { Some(INLINE_CODE.0) } else { linked.then_some(LINK.0) };
    if style.is_some() || !run.marks.is_empty() || lang.is_some() {
        out.push_str("<w:rPr>");
        if let Some(style) = style {
            let _ = write!(out, "<w:rStyle w:val=\"{style}\"/>");
        }
        // In the order Word's schema asks for.
        let formats = [
            (Mark::Bold, "<w:b/><w:bCs/>"),
            (Mark::Italic, "<w:i/><w:iCs/>"),
            (Mark::Strike, "<w:strike/>"),
            (Mark::Highlight, "<w:highlight w:val=\"yellow\"/>"),
            (Mark::Underline, "<w:u w:val=\"single\"/>"),
        ];
        for (_, format) in formats.iter().filter(|(mark, _)| has(mark.clone())) {
            out.push_str(format);
        }
        if let Some(lang) = lang {
            let _ = write!(out, "<w:lang w:val=\"{}\"/>", escaped(lang));
        }
        out.push_str("</w:rPr>");
    }
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(['\n', '\t']).unwrap_or(rest.len());
        if end > 0 {
            out.push_str("<w:t xml:space=\"preserve\">");
            escape(&rest[..end], out);
            out.push_str("</w:t>");
        }
        match rest[end..].chars().next() {
            Some('\n') => out.push_str("<w:br/>"),
            Some(_) => out.push_str("<w:tab/>"),
            None => {}
        }
        rest = rest.get(end + 1..).unwrap_or_default();
    }
    out.push_str("</w:r>");
}

/// The paragraph properties an element type's style comes to. Word reads them in a set order, so what a
/// style adds goes in one of three places: `before` the spacing, `after` it, or `last`.
fn paragraph_look(kind: &TextStyle, before: &str, after: &str, last: &str) -> String {
    let twips = |points: f32| (points * TWIPS).round().max(0.0) as u32;
    let align = match kind.align {
        Align::Start => "left",
        Align::Center => "center",
        Align::End => "right",
        Align::Justify => "both",
    };
    // 240 is single spacing.
    let line = (kind.line_height / SINGLE * 240.0).round().max(1.0) as u32;
    format!(
        "<w:pPr>{before}<w:spacing w:before=\"{}\" w:after=\"{}\" w:line=\"{line}\" w:lineRule=\"auto\"/>{after}\
         <w:jc w:val=\"{align}\"/>{last}</w:pPr>",
        twips(kind.space_before),
        twips(kind.space_after),
    )
}

/// The run properties an element type's style comes to. Every field is written, so no style leans on another.
fn run_look(kind: &TextStyle, extra: &str) -> String {
    let family = escaped(&kind.family);
    let toggle = |on: bool| if on { "" } else { " w:val=\"0\"" };
    let (bold, italic) = (toggle(kind.weight >= 600), toggle(kind.italic));
    let color = kind.color.trim_start_matches('#').to_ascii_uppercase();
    let size = (kind.size * 2.0).round().max(1.0) as u32;
    format!(
        "<w:rPr><w:rFonts w:ascii=\"{family}\" w:hAnsi=\"{family}\" w:eastAsia=\"{family}\" w:cs=\"{family}\"/>\
         <w:b{bold}/><w:bCs{bold}/><w:i{italic}/><w:iCs{italic}/>{extra}<w:color w:val=\"{color}\"/>\
         <w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr>"
    )
}

/// One border of a paragraph: its side, its width in eighths of a point, the gap to the text in points, and
/// its colour.
fn border(side: &str, width: u32, gap: u32, color: &str) -> String {
    format!("<w:{side} w:val=\"single\" w:sz=\"{width}\" w:space=\"{gap}\" w:color=\"{color}\"/>")
}

/// The styles part: one Word style for each element type, and those the blocks without a type of their own need.
fn styles(style: &Style, language: &str) -> String {
    let fallback = TextStyle::default();
    let of = |key: &str| style.types.get(key).or_else(|| style.types.get("body")).unwrap_or(&fallback);
    let (body, code) = (of("body"), of("code"));
    let mut out = format!("{HEAD}<w:styles xmlns:w=\"{W}\"><w:docDefaults><w:rPrDefault>");
    out.push_str(
        &run_look(body, "").replace("</w:rPr>", &format!("<w:lang w:val=\"{}\"/></w:rPr>", escaped(language))),
    );
    out.push_str("</w:rPrDefault><w:pPrDefault/></w:docDefaults>");

    let mut paragraph = |id: &str, name: &str, based: bool, look: &str, runs: &str| {
        let default = if id == "Normal" { " w:default=\"1\"" } else { "" };
        let _ = write!(out, "<w:style w:type=\"paragraph\"{default} w:styleId=\"{id}\"><w:name w:val=\"{name}\"/>");
        if based {
            out.push_str("<w:basedOn w:val=\"Normal\"/>");
        }
        if id != "Normal" && id != LIST.0 {
            out.push_str("<w:next w:val=\"Normal\"/>");
        }
        let _ = write!(out, "<w:qFormat/>{look}{runs}</w:style>");
    };

    // A box is drawn with borders in its fill's colour, whose gaps to the text are its padding. Word draws
    // borders outside the text, so the paragraph is set in by as much.
    let boxed = |bar: &str, fill: &str| {
        let sides = ["top", "left", "bottom", "right"].map(|side| match side {
            "left" => border(side, 18, 12, bar),
            "top" | "bottom" => border(side, 4, 9, fill),
            _ => border(side, 4, 12, fill),
        });
        format!("<w:pBdr>{}</w:pBdr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill}\"/>", sides.concat())
    };
    let inset = "<w:ind w:left=\"285\" w:right=\"250\"/>";

    for (key, id, name) in TYPES {
        let kind = of(key);
        let keep = "<w:keepNext/><w:keepLines/>";
        let (before, after, last, runs) = match key {
            "title" => (keep.to_owned(), "", String::new(), ""),
            "quote" => {
                let bar = format!("<w:pBdr>{}</w:pBdr>", border("left", 18, 12, &hex(PAGE.line_strong)));
                (bar, "<w:ind w:left=\"285\"/>", String::new(), "")
            }
            // Code is not prose: Word should not check its spelling.
            "code" => (boxed(&hex(PAGE.code), &hex(PAGE.code)), inset, String::new(), "<w:noProof/>"),
            key if key.starts_with("heading") => {
                let level = key.trim_start_matches("heading").parse::<u8>().unwrap_or(1) - 1;
                (keep.to_owned(), "", format!("<w:outlineLvl w:val=\"{level}\"/>"), "")
            }
            _ => (String::new(), "", String::new(), ""),
        };
        paragraph(id, name, id != "Normal", &paragraph_look(kind, &before, after, &last), &run_look(kind, runs));
    }
    for (tone, id, name) in CALLOUTS {
        let look = match tone {
            Tone::Tip => PAGE.tip,
            Tone::Info => PAGE.info,
            Tone::Warn => PAGE.warn,
            Tone::Danger => PAGE.danger,
        };
        let frame = boxed(&hex(look.accent), &hex(look.fill));
        paragraph(id, name, true, &paragraph_look(body, &frame, inset, ""), &run_look(body, ""));
    }
    // Items of one list sit closer than paragraphs do.
    paragraph(LIST.0, LIST.1, true, &paragraph_look(body, "", "<w:contextualSpacing/>", ""), &run_look(body, ""));
    // A rule across the page is the bottom border of a paragraph too small to see.
    let rule = format!("<w:pBdr>{}</w:pBdr>", border("bottom", 6, 0, &hex(PAGE.line)));
    let gap = (body.space_after * 2.0 * TWIPS).round() as u32;
    let _ = write!(
        out,
        "<w:style w:type=\"paragraph\" w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:basedOn w:val=\"Normal\"/>\
         <w:next w:val=\"Normal\"/><w:qFormat/><w:pPr>{rule}<w:spacing w:before=\"{gap}\" w:after=\"{gap}\" \
         w:line=\"40\" w:lineRule=\"exact\"/></w:pPr><w:rPr><w:sz w:val=\"2\"/><w:szCs w:val=\"2\"/></w:rPr></w:style>",
        DIVIDER.0, DIVIDER.1
    );

    let mono = escaped(&code.family);
    let _ = writeln!(
        out,
        "<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\">\
         <w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/></w:style>\
         <w:style w:type=\"character\" w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:qFormat/><w:rPr>\
         <w:rFonts w:ascii=\"{mono}\" w:hAnsi=\"{mono}\" w:eastAsia=\"{mono}\" w:cs=\"{mono}\"/><w:noProof/>\
         <w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/></w:rPr></w:style>\
         <w:style w:type=\"character\" w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:rPr><w:color w:val=\"{}\"/>\
         <w:u w:val=\"single\"/></w:rPr></w:style>\
         <w:style w:type=\"numbering\" w:default=\"1\" w:styleId=\"NoList\"><w:name w:val=\"No List\"/>\
         <w:semiHidden/></w:style></w:styles>",
        INLINE_CODE.0,
        INLINE_CODE.1,
        hex(PAGE.inline_code),
        LINK.0,
        LINK.1,
        hex(PAGE.link),
    );
    out
}

/// The numbering part: bullets, the two kinds of task, and one list for each place numbering starts over.
fn numbering(numbered: usize) -> String {
    let marker = hex(PAGE.marker);
    let pattern = |id: usize, level: &dyn Fn(usize) -> (&'static str, String)| {
        let mut out = format!("<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>");
        for depth in 0..9 {
            let (format, text) = level(depth);
            let _ = write!(
                out,
                "<w:lvl w:ilvl=\"{depth}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/>\
                 <w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" \
                 w:hanging=\"{LIST_INDENT}\"/></w:pPr><w:rPr><w:color w:val=\"{marker}\"/></w:rPr></w:lvl>",
                LIST_INDENT * (depth + 1),
            );
        }
        out.push_str("</w:abstractNum>");
        out
    };
    let bullets = ['\u{2022}', '\u{25e6}', '\u{25aa}'];
    let mut out = format!("{HEAD}<w:numbering xmlns:w=\"{W}\">");
    out.push_str(&pattern(BULLETS, &|depth| ("bullet", bullets[depth % 3].to_string())));
    out.push_str(&pattern(TASKS_OPEN, &|_| ("bullet", TASK_OPEN.to_string())));
    out.push_str(&pattern(TASKS_DONE, &|_| ("bullet", TASK_DONE.to_string())));
    // The page numbers every level 1, 2, 3.
    let ordered = TASKS_DONE + 1;
    out.push_str(&pattern(ordered, &|depth| ("decimal", format!("%{}.", depth + 1))));
    for list in [BULLETS, TASKS_OPEN, TASKS_DONE] {
        let _ = write!(out, "<w:num w:numId=\"{list}\"><w:abstractNumId w:val=\"{list}\"/></w:num>");
    }
    for list in 0..numbered {
        // Each list counts from 1 again, where Word would carry on from the list before.
        let _ = write!(out, "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{ordered}\"/>", ordered + list);
        for depth in 0..9 {
            let _ = write!(out, "<w:lvlOverride w:ilvl=\"{depth}\"><w:startOverride w:val=\"1\"/></w:lvlOverride>");
        }
        out.push_str("</w:num>");
    }
    out.push_str("</w:numbering>\n");
    out
}

/// The settings part. Without its compatibility mode, Word opens the file as one from an older version.
fn settings(language: &str) -> String {
    format!(
        "{HEAD}<w:settings xmlns:w=\"{W}\"><w:compat><w:compatSetting w:name=\"compatibilityMode\" \
         w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat>\
         <w:themeFontLang w:val=\"{}\"/></w:settings>\n",
        escaped(language)
    )
}
