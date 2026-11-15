//! Document to Word document.
//!
//! Every paragraph names a style and carries no formatting of its own, so changing a style in Word restyles
//! the document as it does here. Marks are the one thing written onto runs, as Word's own buttons would,
//! beside the side text reads from, which Word wants said where the page works it out.
//! The exceptions are what Word has no style for: how a column of a table is aligned, which each of its
//! cells says, where a picture stands on its line, how far in an item of a list sits inside a quote, and
//! the side a quote's bar is at where its text reads from the right.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Cursor, Write as _};
use std::ops::Range;

use letters_layout::Palette;
use letters_model::{
    Align, Block, BlockId, BlockKind, Document, ListKind, Mark, NoteKind, Notes, Preview, Run, Style, TextStyle, Tone,
};
use letters_render::Color;
use letters_render::image::Image;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::math::equation;
use super::{
    BOOKMARK, BOX_SPACE, CALLOUT_TITLE, CALLOUTS, CELL, CONTENTS, DIVIDER, HEAD_CELL, HEADING_MARK, INLINE_CODE, LINK,
    LIST, PREVIEW, SEQUENCE, SINGLE, TABLE, TABLE_SPACE, TASK_DONE, TASK_OPEN, TWIPS, TYPES,
};
use crate::figures::fitted;
use crate::html::{Assets, host, href};

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
/// How far the text of a quote, a callout or code sits in from the margin, which is where Word draws the
/// frame's left side.
const BOX_INSET: usize = 285;
/// The longest address Word follows. A longer one is written as its text alone.
const LONGEST_LINK: usize = 2000;
/// What starts a paragraph on a new page.
const BREAK: &str = "<w:pageBreakBefore/>";
/// What sets a paragraph from the right.
const BIDI: &str = "<w:bidi/>";
/// The gap between two pictures of a gallery, in points, which is the page's. A space too small to see
/// stands there, spread to a little under the gap, so that a row never runs over its line.
const TILE_GAP: f32 = 8.0;
const TILE_SPACE: &str = "<w:r><w:rPr><w:spacing w:val=\"150\"/><w:sz w:val=\"2\"/><w:szCs w:val=\"2\"/></w:rPr>\
                          <w:t xml:space=\"preserve\"> </w:t></w:r>";
/// The widest a link's preview shows its picture, in points.
const PREVIEW_WIDE: f32 = 240.0;
/// The ids of the bookmarks at headings start here, clear of those at figures, which are their numbers.
const HEADING_IDS: usize = 1_000_000;
/// The padding inside a table cell, across and down, in twentieths of a point. The page's own.
const CELL_PAD: (u32, u32) = (200, 120);

/// Writes a `.docx` with `style` as its styles.
///
/// - Each element type becomes the Word style of the same name, and each block a paragraph of its style.
/// - Comments are written as Word comments on the same passages, with their replies. Import notes and
///   conflicts stay behind.
/// - `title` goes into the file's properties, and `assets` supplies the pictures.
/// - A formula is written as a Word equation, and as its source where it has a mistake in it.
/// - A numbered figure's caption opens with a label that Word counts, and a reference to the figure is a
///   cross-reference to that label. The pictures of a gallery share a paragraph, in the rows the page sets.
/// - A table is a Word table, its header row repeated on each page it runs to.
/// - A callout opens with the line that heads it on the page, in a style of its own. Its icon is left out,
///   and so is a colour of its own.
/// - A block that reads from the right on the page is set from the right, and so is each stretch of text
///   in a line that does.
/// - A table of contents is the field Word makes one from, holding the lines the page lists until Word is
///   asked to renew them. A link's preview is what it shows: its picture, its title as the link, and its
///   description.
/// - A link that would run script is written as its text alone, and an unknown block as its fallback text.
pub fn to_docx(doc: &Document, style: &Style, notes: &Notes, title: &str, assets: Assets) -> Vec<u8> {
    let page = &style.page;
    let labels = letters_layout::figure_numbers(doc).into_iter().map(|(id, number)| {
        // The page's label, cut around its number, which Word writes.
        let (label, digits) = (letters_layout::figure_label(style, number), number.to_string());
        let (word, rest) = label.rsplit_once(&digits).unwrap_or((&label, ""));
        (id, Label { number, word: word.to_owned(), rest: rest.to_owned() })
    });
    // As on the page, only a caption set to the middle or the end moves its picture.
    let pictured = match style.types.get("caption").or_else(|| style.types.get("body")).map(|caption| caption.align) {
        Some(align @ (Align::Center | Align::End)) => align,
        _ => Align::Start,
    };
    // The lines of each table of contents, and a number for every heading that one of them lists.
    let tables = doc.walk().filter(|block| matches!(block.kind, BlockKind::TableOfContents { .. }));
    let tables: BTreeMap<&str, Vec<&Block>> =
        tables.map(|table| (table.id.as_str(), doc.contents(&table.id))).collect();
    let mut listed = BTreeMap::new();
    for heading in tables.values().flatten() {
        let next = listed.len() + 1;
        listed.entry(heading.id.as_str()).or_insert(next);
    }
    let mut writer = Writer {
        out: String::new(),
        links: Links::new(&doc.previews),
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
        rtl: false,
        turn: false,
        labels: labels.collect(),
        tables,
        listed,
        pictured,
        drawn: 0,
        keep: false,
        align: None,
        lead: String::new(),
        last: String::new(),
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

    let body_links = writer.links.part();
    let comments_xml = (!comments.is_empty()).then(|| writer.comments(&comments));
    let comment_links = writer.links.part();

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
struct Links<'d> {
    targets: Vec<(String, &'static str)>,
    known: BTreeMap<String, usize>,
    /// What the document keeps of the pages its links lead to, by address as a link writes it.
    previews: &'d BTreeMap<String, Preview>,
}

impl<'d> Links<'d> {
    /// How far the ids of the part's fixed relationships run.
    const FIXED: usize = 8;
    /// The longest ScreenTip written, in UTF-16 units. Word limits how long one may be.
    const TIP: usize = 255;

    fn new(previews: &'d BTreeMap<String, Preview>) -> Self {
        Self { targets: Vec::new(), known: BTreeMap::new(), previews }
    }

    /// Hands over the links of the part just written and starts the next part's.
    fn part(&mut self) -> Self {
        std::mem::replace(self, Self::new(self.previews))
    }

    /// What Word shows over a link that is pointed at: the title of its page, where the document keeps
    /// one. Word has no place there for the description and the picture the page shows with it.
    fn tip(&self, link: &str) -> Option<String> {
        let title = self.previews.get(link)?.title.as_deref()?.trim();
        let mut room = Self::TIP;
        let fits = |c: &char| room.checked_sub(c.len_utf16()).map(|left| room = left).is_some();
        let mut tip: String = title.chars().take_while(fits).collect();
        if tip.len() < title.len() {
            tip.pop();
            tip.push('\u{2026}');
        }
        (!tip.is_empty()).then_some(tip)
    }

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
fn relationships(fixed: &str, links: &Links<'_>) -> String {
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

/// The label of a numbered figure, such as "Figure 2:", in the parts around its number.
struct Label {
    number: u32,
    word: String,
    rest: String,
}

impl Label {
    /// What a reference to the figure reads as.
    fn shown(&self) -> String {
        format!("{}{}", self.word, self.number)
    }
}

struct Writer<'d, 'a> {
    /// The document's body so far.
    out: String,
    links: Links<'d>,
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
    /// Whether the block being written reads from the right, as the page finds.
    rtl: bool,
    /// Whether the next paragraph is set from the right.
    turn: bool,
    /// The label of each numbered figure and gallery.
    labels: BTreeMap<BlockId, Label>,
    /// The headings each table of contents lists, by the table's id.
    tables: BTreeMap<&'d str, Vec<&'d Block>>,
    /// The number of the bookmark at each heading that a table of contents lists.
    listed: BTreeMap<&'d str, usize>,
    /// Where a picture stands on its line, which is where the caption's style sets text.
    pictured: Align,
    /// How many pictures have been drawn. Word wants a number on each.
    drawn: usize,
    /// Whether the next paragraph stays on the page of the one after it.
    keep: bool,
    /// How the next paragraph is set, where that is not its style's to say.
    align: Option<Align>,
    /// What the next paragraph opens with before its text, as a caption does with its label.
    lead: String,
    /// The style of the last paragraph written.
    last: String,
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
        let (around, turned) = (self.lang, self.rtl);
        for block in blocks {
            // The first paragraph a block writes carries its page break, and all of them its language and
            // the side it reads from.
            self.breaks |= block.page_break;
            self.lang = block.lang.as_deref().or(around);
            self.rtl = letters_layout::rtl(block);
            let spans = self.noted(block);
            if !matches!(block.kind, BlockKind::ListItem { .. }) {
                self.counters.clear();
            }
            match &block.kind {
                BlockKind::Paragraph => self.paragraph(within.unwrap_or(""), None, block.body.runs(), spans),
                BlockKind::Heading { level } => {
                    let style = TYPES[usize::from((*level).clamp(1, 4))].1;
                    if let Some(number) = self.listed.get(block.id.as_str()) {
                        // Where the line of a table of contents leads.
                        let id = HEADING_IDS + number;
                        let _ = write!(
                            self.lead,
                            "<w:bookmarkStart w:id=\"{id}\" w:name=\"{HEADING_MARK}{number}\"/><w:bookmarkEnd w:id=\"{id}\"/>"
                        );
                    }
                    self.paragraph(style, None, block.body.runs(), spans);
                }
                BlockKind::ListItem { list, depth } => {
                    let depth = (*depth).min(8);
                    let list = self.list(*list, usize::from(depth));
                    self.paragraph(within.unwrap_or(LIST.0), Some((list, depth)), block.body.runs(), spans);
                }
                BlockKind::Code { .. } => {
                    self.part("Code");
                    self.paragraph("Code", None, block.body.runs(), spans);
                }
                BlockKind::Formula => match equation(&block.plain_text(), true) {
                    Some(equation) => {
                        self.open(within.unwrap_or(""), None);
                        self.whole(&format!("<m:oMathPara>{equation}</m:oMathPara>"));
                        self.out.push_str("</w:p>");
                    }
                    // A source with a mistake in it goes out as it is, so that it is not lost.
                    None => {
                        self.part("Code");
                        self.paragraph("Code", None, block.body.runs(), spans);
                    }
                },
                BlockKind::Quote => self.container(block, "Quote", within),
                BlockKind::Callout { tone, .. } => {
                    let style = CALLOUTS.iter().find(|(known, ..)| known == tone).map_or("Quote", |(_, id, _)| *id);
                    self.container(block, style, within);
                }
                BlockKind::Divider => self.paragraph(DIVIDER.0, None, &[], Vec::new()),
                BlockKind::Figure { .. } => self.figure(block, spans),
                BlockKind::Gallery { .. } => self.gallery(block, within),
                BlockKind::Caption => self.paragraph("Caption", None, block.body.runs(), spans),
                BlockKind::Table { columns, header } => {
                    let is_row = |part: &&Block| part.kind == BlockKind::TableRow;
                    let (rows, rest): (Vec<_>, Vec<_>) = block.body.blocks().iter().partition(is_row);
                    self.table(&rows, columns, *header);
                    // What a merge left in a table that is no row of it follows it, as on the page.
                    for stray in rest {
                        self.blocks(std::slice::from_ref(stray), within);
                    }
                }
                // A merge can leave a row outside a table, which is a table of one row, and a cell outside
                // a row, which is a line of text.
                BlockKind::TableRow => self.table(&[block], &[], false),
                BlockKind::TableCell => self.paragraph(within.unwrap_or(""), None, block.body.runs(), spans),
                BlockKind::Unknown { .. } => {
                    let fallback =
                        block.extra.get("fallback").and_then(|fallback| fallback.as_str()).unwrap_or_default();
                    if !fallback.is_empty() {
                        self.paragraph("Caption", None, &[Run::plain(fallback)], Vec::new());
                    }
                }
                BlockKind::LinkPreview { .. } => self.preview(block, within),
                BlockKind::TableOfContents { levels } => self.contents(block, *levels),
            }
        }
        (self.lang, self.rtl) = (around, turned);
    }

    /// Takes the comments on a block. Those on a passage of its text are returned, and those on the whole of
    /// it wait for the first paragraph it writes.
    fn noted(&mut self, block: &'d Block) -> Vec<(Range<usize>, Range<usize>)> {
        let mut spans = Vec::new();
        for (range, comments) in self.anchors.remove(block.id.as_str()).unwrap_or_default() {
            // A passage of a formula's source is no passage of the equation it is written as.
            match range.filter(|_| !block.body.runs().is_empty() && block.kind != BlockKind::Formula) {
                Some(range) => spans.push((range, comments)),
                None => self.pending.push(comments),
            }
        }
        spans
    }

    /// Makes the comments on a block that writes no text of its own wait for the next paragraph.
    fn defer(&mut self, block: &'d Block) {
        let spans = self.noted(block);
        self.pending.extend(spans.into_iter().map(|(_, comments)| comments));
    }

    /// Writes what a quote or a callout holds. Word has no block that holds others, so each paragraph inside
    /// takes the container's style, and Word draws neighbours of one style as one box.
    fn container(&mut self, block: &'d Block, style: &'static str, within: Option<&'static str>) {
        let inner = block.body.blocks();
        // One inside another of its kind is more of that one, which is all Word can show of it.
        if within != Some(style) {
            self.part(style);
        }
        // The line that heads a callout on the page heads it here, in a style of its own that shares the
        // box. Word has no place for an icon, or for a colour beside the tone's.
        let title = block.kind.callout_look().map_or("", |look| look.title);
        if !title.is_empty() {
            // A comment on the whole callout sits on its text, where there is any.
            let waiting = if inner.is_empty() { Vec::new() } else { std::mem::take(&mut self.pending) };
            self.paragraph(&format!("{style}{}", CALLOUT_TITLE.0), None, &[Run::plain(title)], Vec::new());
            self.pending = waiting;
        } else if inner.is_empty() {
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

    /// Starts a paragraph of a style, which takes the page break that waits for one, and what else was set
    /// for the next paragraph alone.
    fn open(&mut self, style: &str, list: Option<(usize, u8)>) {
        self.out.push_str("<w:p>");
        self.last.clear();
        self.last.push_str(style);
        let (breaks, keep, align) =
            (std::mem::take(&mut self.breaks), std::mem::take(&mut self.keep), self.align.take());
        let turn = std::mem::take(&mut self.turn);
        if !style.is_empty() || list.is_some() || breaks || keep || align.is_some() || turn {
            // In the order Word's schema asks for.
            self.out.push_str("<w:pPr>");
            if !style.is_empty() {
                let _ = write!(self.out, "<w:pStyle w:val=\"{style}\"/>");
            }
            if keep {
                self.out.push_str("<w:keepNext/>");
            }
            if breaks {
                self.out.push_str(BREAK);
            }
            if let Some((list, depth)) = list {
                let _ = write!(self.out, "<w:numPr><w:ilvl w:val=\"{depth}\"/><w:numId w:val=\"{list}\"/></w:numPr>");
            }
            if turn && style == "Quote" {
                // A frame's sides are the page's and not the text's, so the bar is moved to where the text
                // starts, as the page draws it.
                let bar = border("right", 18, 12, &hex(PAGE.line_strong));
                let _ = write!(self.out, "<w:pBdr><w:left w:val=\"nil\"/>{bar}</w:pBdr>");
            }
            if turn {
                // Word then counts a style's left inset from the side the text starts at, as the page does.
                self.out.push_str(BIDI);
            }
            if let Some((_, depth)) = list
                && !style.is_empty()
                && style != LIST.0
            {
                // An item in a quote or a callout. Word draws a frame where a paragraph's first line starts,
                // so every marker stands at the frame's inset, and the text as deep as its level.
                let hang = LIST_INDENT * (usize::from(depth) + 1);
                let _ = write!(self.out, "<w:ind w:left=\"{}\" w:hanging=\"{hang}\"/>", BOX_INSET + hang);
            }
            if let Some(align) = align {
                let _ = write!(self.out, "<w:jc w:val=\"{}\"/>", justification(align));
            }
            self.out.push_str("</w:pPr>");
        }
    }

    /// Parts a box from one of its kind right above it, which Word would draw as one with it. `style` is
    /// what the paragraphs of the box are set in.
    fn part(&mut self, style: &str) {
        let above = self.last.strip_suffix(CALLOUT_TITLE.0).filter(|callout| *callout == style);
        if self.last == style || above.is_some() {
            self.empty(BOX_SPACE.0);
        }
    }

    /// Writes a paragraph that holds nothing and takes no comment, which Word needs here and there.
    fn empty(&mut self, style: &str) {
        self.open(style, None);
        self.out.push_str("</w:p>");
    }

    /// Writes one paragraph of a style, with the comments that sit on it.
    fn paragraph(
        &mut self,
        style: &str,
        list: Option<(usize, u8)>,
        runs: &[Run],
        mut spans: Vec<(Range<usize>, Range<usize>)>,
    ) {
        self.turn = self.rtl;
        self.open(style, list);
        let lead = std::mem::take(&mut self.lead);
        self.out.push_str(&lead);
        let len: usize = runs.iter().map(|run| run.text.len()).sum();
        spans.extend(self.pending.drain(..).map(|comments| (0..len, comments)));
        let mut out = std::mem::take(&mut self.out);
        inline(runs, &spans, self.lang, self.rtl, &self.labels, &mut self.links, &mut out);
        self.out = out;
        self.out.push_str("</w:p>");
    }

    /// Writes a picture as a paragraph of its own, and its caption as the paragraph after.
    fn figure(&mut self, block: &'d Block, spans: Vec<(Range<usize>, Range<usize>)>) {
        let BlockKind::Figure { alt, width, height, .. } = &block.kind else { return };
        let captioned = self.labels.contains_key(&block.id) || block.body.runs().iter().any(|run| !run.text.is_empty());
        match self.held(block) {
            Some(id) => {
                let (mut width, mut height) = (width.max(1.0), height.max(1.0));
                if width > self.measure {
                    (width, height) = (self.measure, height * self.measure / width);
                }
                let picture = self.drawing(&id, alt, width, height);
                // A picture goes to the next page with its caption.
                (self.keep, self.align) = (captioned, Some(self.pictured));
                self.open("", None);
                self.whole(&picture);
                self.out.push_str("</w:p>");
            }
            None => self.stand_in(block),
        }
        if captioned {
            self.label(&block.id, block.body.runs());
            self.paragraph("Caption", None, block.body.runs(), spans);
        }
    }

    /// Writes pictures side by side as one paragraph, a line for each row the page sets them in, and their
    /// caption as the paragraph after.
    fn gallery(&mut self, block: &'d Block, within: Option<&'static str>) {
        let inside = block.body.blocks();
        let is_figure = |part: &&Block| matches!(part.kind, BlockKind::Figure { .. });
        // The page shows the last caption under the pictures, and whatever else a merge left here after it.
        let caption = inside.iter().rfind(|part| part.kind == BlockKind::Caption);
        let shown = |part: &&Block| is_figure(part) || caption.is_some_and(|caption| caption.id == part.id);
        let numbered = self.labels.contains_key(&block.id);

        let (mut tiles, mut missing) = (Vec::new(), Vec::new());
        for figure in inside.iter().filter(is_figure) {
            // A picture here has no text of its own to hang a comment on.
            self.defer(figure);
            let BlockKind::Figure { alt, width, height, .. } = &figure.kind else { continue };
            match self.held(figure) {
                Some(id) => tiles.push((id, alt.as_str(), width.max(1.0) / height.max(1.0))),
                None => missing.push(figure),
            }
        }
        let (mut rows, mut at) = (String::new(), 0);
        while at < tiles.len() {
            // Up to three to a row, and the last picture is never left alone.
            let left = tiles.len() - at;
            let take = if left == 4 { 2 } else { left.min(3) };
            let row = &tiles[at..at + take];
            let room = (self.measure - TILE_GAP * (take - 1) as f32).max(take as f32);
            let tall = (room / row.iter().map(|(.., shape)| shape).sum::<f32>()).max(1.0);
            if at > 0 {
                rows.push_str("<w:r><w:br/></w:r>");
            }
            for (index, (id, alt, shape)) in row.iter().enumerate() {
                if index > 0 {
                    rows.push_str(TILE_SPACE);
                }
                rows.push_str(&self.drawing(id, alt, (shape * tall).max(1.0), tall));
            }
            at += take;
        }
        if !rows.is_empty() {
            (self.keep, self.align) = (numbered || caption.is_some(), Some(self.pictured));
            self.open("", None);
            self.whole(&rows);
            self.out.push_str("</w:p>");
        }
        for figure in missing {
            self.stand_in(figure);
        }
        self.label(&block.id, caption.map_or(&[], |caption| caption.body.runs()));
        match caption {
            Some(caption) => self.blocks(std::slice::from_ref(caption), within),
            None if numbered => self.paragraph("Caption", None, &[], Vec::new()),
            None => {}
        }
        for stray in inside.iter().filter(|part| !shown(part)) {
            self.blocks(std::slice::from_ref(stray), within);
        }
    }

    /// Readies the label that opens the caption of a numbered figure, for the paragraph written next.
    fn label(&mut self, figure: &BlockId, caption: &[Run]) {
        let Some(Label { number, word, rest }) = self.labels.get(figure) else { return };
        // A reference to the figure names the bookmark, and shows what it holds: the label up to its number.
        let _ = write!(
            self.lead,
            "<w:bookmarkStart w:id=\"{number}\" w:name=\"{BOOKMARK}{number}\"/><w:r><w:t xml:space=\"preserve\">{}</w:t>\
             </w:r>{}<w:bookmarkEnd w:id=\"{number}\"/>",
            escaped(word),
            field(&format!("SEQ {SEQUENCE} \\* ARABIC"), &format!("<w:r><w:t>{number}</w:t></w:r>")),
        );
        if caption.iter().any(|run| !run.text.is_empty()) {
            let _ = write!(self.lead, "<w:r><w:t xml:space=\"preserve\">{} </w:t></w:r>", escaped(rest));
        }
    }

    /// The relationship of a figure's picture, for one the file can hold.
    fn held(&mut self, figure: &'d Block) -> Option<String> {
        let BlockKind::Figure { asset: Some(name), .. } = &figure.kind else { return None };
        self.stored(name)
    }

    /// The relationship of an asset, for one the file can hold. Each is stored once.
    fn stored(&mut self, name: &'d str) -> Option<String> {
        match self.pictures.get(name) {
            Some(known) => known.clone(),
            None => {
                let id = self.embed(name);
                self.pictures.insert(name, id.clone());
                id
            }
        }
    }

    /// Writes a table of contents as the field Word makes one from. What the field last came to is the
    /// lines the page lists, each leading to its heading, which Word shows until it is asked to renew
    /// them. It then sets its own, with the numbers of its pages.
    fn contents(&mut self, block: &'d Block, levels: u8) {
        let lines = self.tables.remove(block.id.as_str()).unwrap_or_default();
        let code = format!("TOC \\o \"1-{}\" \\h", levels.clamp(1, 4));
        let mut opening = field_start(&code);
        if lines.is_empty() {
            self.open(CONTENTS[0].0, None);
            self.whole(&(opening + &field_mark("end")));
            self.out.push_str("</w:p>");
            return;
        }
        for (index, heading) in lines.iter().enumerate() {
            let BlockKind::Heading { level } = heading.kind else { continue };
            let (rtl, lang) = (letters_layout::rtl(heading), heading.lang.as_deref().or(self.lang));
            // A field starts in its first line and ends in its last.
            let mut line = std::mem::take(&mut opening);
            if let Some(number) = self.listed.get(heading.id.as_str()) {
                let _ = write!(line, "<w:hyperlink w:anchor=\"{HEADING_MARK}{number}\">");
            }
            let text = Run::plain(heading.plain_text().replace('\n', " "));
            inline(&[text], &[], lang, rtl, &self.labels, &mut self.links, &mut line);
            if self.listed.contains_key(heading.id.as_str()) {
                line.push_str("</w:hyperlink>");
            }
            if index + 1 == lines.len() {
                line.push_str(&field_mark("end"));
            }
            self.turn = rtl;
            self.open(CONTENTS[usize::from(level.clamp(1, 4)) - 1].0, None);
            self.whole(&line);
            self.out.push_str("</w:p>");
        }
    }

    /// Writes a link's preview as what it shows, in a frame that holds it together: its picture, its title
    /// as the link, its description and where it leads. Word has no card that would fetch these again.
    fn preview(&mut self, block: &'d Block, within: Option<&'static str>) {
        let BlockKind::LinkPreview { url, title, description, image } = &block.kind else { return };
        let style = within.unwrap_or(PREVIEW.0);
        if within.is_none() {
            self.part(style);
        }
        let turned = |text: &str| text.chars().find_map(letters_layout::reads).unwrap_or(false);
        let given = |text: &'d Option<String>| text.as_deref().filter(|text| !text.trim().is_empty());
        let (title, description) = (given(title), given(description));
        let shot = image.as_deref().and_then(|name| {
            let image = Image::decode(&(self.assets)(name)?).ok()?;
            let (width, height) = fitted(image.width() as f32, image.height() as f32);
            let wide = width.min(PREVIEW_WIDE).min(self.measure);
            Some((self.stored(name)?, wide, (height * wide / width).max(1.0)))
        });
        if let Some((id, width, height)) = shot {
            // The title under it says what the picture is of.
            let picture = self.drawing(&id, "", width, height);
            self.keep = true;
            self.open(style, None);
            self.whole(&picture);
            self.out.push_str("</w:p>");
        }
        // One that was never fetched shows its address alone, and a title says where it leads under it.
        let link = Run { link: Some(url.clone()), ..Run::marked(title.unwrap_or(url), [Mark::Bold]) };
        let under = description.into_iter().chain(title.map(|_| host(url)));
        let lines: Vec<Vec<Run>> =
            std::iter::once(vec![link]).chain(under.map(|text| vec![Run::plain(text)])).collect();
        for (index, line) in lines.iter().enumerate() {
            let said: String = line.iter().map(|run| run.text.as_str()).collect();
            (self.keep, self.rtl) = (index + 1 < lines.len(), turned(&said));
            self.paragraph(style, None, line, Vec::new());
        }
    }

    /// Writes what stands for a picture the file cannot hold. One that is still remote is a link to it, so
    /// that opening the file fetches nothing, and one that is missing is its description.
    fn stand_in(&mut self, figure: &'d Block) {
        let BlockKind::Figure { src, alt, .. } = &figure.kind else { return };
        match src.as_deref().and_then(address) {
            Some(url) => {
                let label = if alt.is_empty() { url.as_str() } else { alt };
                let link = Run { link: Some(url.clone()), ..Run::plain(label) };
                self.paragraph("", None, &[link], Vec::new());
            }
            None if !alt.is_empty() => self.paragraph("Caption", None, &[Run::plain(alt.as_str())], Vec::new()),
            None => {}
        }
    }

    /// A picture as it sits in a line, at a size in points.
    fn drawing(&mut self, id: &str, alt: &str, width: f32, height: f32) -> String {
        self.drawn += 1;
        let number = self.drawn;
        let emu = |points: f32| (points * 12700.0).round() as u64;
        let (cx, cy) = (emu(width), emu(height));
        let alt = escaped(alt);
        format!(
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
        )
    }

    /// Writes a table with a column for each cell of its longest row. A row short of cells ends in empty ones.
    fn table(&mut self, rows: &[&'d Block], columns: &[Align], header: bool) {
        let cells = |row: &'d Block| row.body.blocks().iter().filter(|cell| cell.kind == BlockKind::TableCell);
        if rows.is_empty() {
            return;
        }
        if self.breaks {
            // Word starts no new page inside a table, so an empty paragraph ahead of it starts one.
            self.empty(TABLE_SPACE.0);
        }
        let across = rows.iter().map(|&row| cells(row).count()).max().unwrap_or(0).max(columns.len()).max(1);
        // Word shares the width out by what the columns hold, as the page does. These are where it starts.
        let wide = ((self.measure * TWIPS).round() as usize / across).max(1);
        let (look, first) = if header { ("0620", 1) } else { ("0600", 0) };
        let _ = write!(
            self.out,
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"{}\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblLook w:val=\"{look}\" \
             w:firstRow=\"{first}\" w:lastRow=\"0\" w:firstColumn=\"0\" w:lastColumn=\"0\" w:noHBand=\"1\" \
             w:noVBand=\"1\"/></w:tblPr><w:tblGrid>{}</w:tblGrid>",
            TABLE.0,
            format!("<w:gridCol w:w=\"{wide}\"/>").repeat(across),
        );
        for (index, &row) in rows.iter().enumerate() {
            self.defer(row);
            self.out.push_str("<w:tr>");
            if header && index == 0 {
                self.out.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            // The cells of a row that heads the table are set by a style of their own, which any reader shows.
            let style = if header && index == 0 { HEAD_CELL.0 } else { CELL.0 };
            let mut held = cells(row);
            for column in 0..across {
                let _ = write!(self.out, "<w:tc><w:tcPr><w:tcW w:w=\"{wide}\" w:type=\"dxa\"/></w:tcPr>");
                self.align = columns.get(column).copied().filter(|align| *align != Align::Start);
                match held.next() {
                    Some(cell) => self.blocks(std::slice::from_ref(cell), Some(style)),
                    None => self.empty(style),
                }
                self.out.push_str("</w:tc>");
            }
            self.out.push_str("</w:tr>");
        }
        self.out.push_str("</w:tbl>");
        // Word joins tables that touch and sets one against the text under it, so a paragraph keeps it apart.
        self.empty(TABLE_SPACE.0);
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
            // A comment reads from the side its first letter does, as a block does.
            let rtl = comment.content.iter().flat_map(|run| run.text.chars()).find_map(letters_layout::reads);
            let rtl = rtl.unwrap_or(false);
            let _ = write!(
                out,
                "<w:comment w:id=\"{index}\" w:author=\"{}\" w:date=\"{}\" w:initials=\"{}\"><w:p w14:paraId=\"{}\">{}",
                escaped(comment.author),
                escaped(comment.created),
                escaped(&initials),
                paragraph(index),
                if rtl { format!("<w:pPr>{BIDI}</w:pPr>") } else { String::new() },
            );
            inline(comment.content, &[], None, rtl, &self.labels, &mut self.links, &mut out);
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

/// A field: the instructions Word follows, between marks, and then what they last came to, which is what
/// a reader that follows none shows.
fn field(code: &str, shown: &str) -> String {
    [field_start(code), shown.to_owned(), field_mark("end")].concat()
}

/// One of the marks a field's parts stand between.
fn field_mark(kind: &str) -> String {
    format!("<w:r><w:fldChar w:fldCharType=\"{kind}\"/></w:r>")
}

/// A field up to where what it last came to starts.
fn field_start(code: &str) -> String {
    let code = format!("<w:r><w:instrText xml:space=\"preserve\"> {code} </w:instrText></w:r>");
    [field_mark("begin"), code, field_mark("separate")].concat()
}

/// Ends a comment's passage and leaves its mark, which is where Word hangs the comment.
fn close(id: usize, out: &mut String) {
    let _ = write!(out, "<w:commentRangeEnd w:id=\"{id}\"/><w:r><w:commentReference w:id=\"{id}\"/></w:r>");
}

/// Where a paragraph's text turns to read the other way, in bytes, and the way it reads from each turn on,
/// `true` for from the right. It starts as the paragraph reads, which `rtl` says.
///
/// A letter says for itself. What has no side of its own, as spaces, digits and signs have none, reads as
/// the letters on both sides of it do where they agree, and as the paragraph does where they do not. Word
/// asks to be told, where the page works it out.
fn turns(runs: &[Run], rtl: bool) -> Vec<(usize, bool)> {
    let mut turns = vec![(0, rtl)];
    let mut set = |at: usize, side: bool| match turns.last_mut() {
        Some((from, last)) if *from == at => *last = side,
        Some((_, last)) if *last == side => {}
        _ => turns.push((at, side)),
    };
    let (mut last, mut open, mut at) = (rtl, None, 0);
    for run in runs {
        // A formula stands in its line as one thing, whatever its source is written in.
        let formula = run.marks.contains(&Mark::Math);
        for (offset, letter) in run.text.char_indices() {
            match (!formula).then_some(letter).and_then(letters_layout::reads) {
                Some(side) => {
                    if let Some(start) = open.take() {
                        set(start, if last == side { side } else { rtl });
                    }
                    set(at + offset, side);
                    last = side;
                }
                None => _ = open.get_or_insert(at + offset),
            }
        }
        at += run.text.len();
    }
    if let Some(start) = open {
        set(start, rtl);
    }
    turns
}

/// Writes runs, with each comment's passage marked around its text. Offsets are bytes into the runs' text.
/// `lang` is the language of the block around them, where that is not the document's, `rtl` says that
/// their paragraph reads from the right, and `labels` are those of the figures a run can refer to.
fn inline(
    runs: &[Run],
    spans: &[(Range<usize>, Range<usize>)],
    lang: Option<&str>,
    rtl: bool,
    labels: &BTreeMap<BlockId, Label>,
    links: &mut Links<'_>,
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

    let turns = turns(runs, rtl);
    let turned = |at: usize| turns.iter().rev().find(|(from, _)| *from <= at).is_some_and(|(_, side)| *side);
    let mut cuts: Vec<usize> = spans.iter().flat_map(|(range, _)| [range.start, range.end]).collect();
    cuts.extend(turns.iter().map(|(from, _)| *from));
    let mut at = 0;
    let mut open: Option<&str> = None;
    for run in runs {
        let own = run.lang.as_deref().or(lang);
        let figure = run.marks.iter().find_map(|mark| match mark {
            Mark::Ref(figure) => labels.get(figure),
            _ => None,
        });
        // A formula is written as an equation, and a reference to a numbered figure as a field that shows
        // the figure's label, which Word keeps up as figures come and go. Neither is text to cut.
        let whole = match figure {
            Some(label) => {
                let (named, mut shown) = (label.shown(), String::new());
                let side = named.chars().find_map(letters_layout::reads).unwrap_or(false);
                text(&named, run, false, own, side, &mut shown);
                Some(field(&format!("REF {BOOKMARK}{} \\h", label.number), &shown))
            }
            None => run.marks.contains(&Mark::Math).then(|| equation(&run.text, false)).flatten(),
        };
        let mut rest = run.text.as_str();
        while !rest.is_empty() {
            // The run is cut where a passage starts or ends inside it, never inside a character or a formula.
            let cut = cuts.iter().map(|cut| cut.saturating_sub(at)).filter(|cut| *cut > 0 && *cut < rest.len());
            let mut cut = cut.min().filter(|_| whole.is_none()).unwrap_or(rest.len());
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
                let _ = write!(out, "<w:hyperlink r:id=\"{}\"", links.id(&target, "hyperlink"));
                if let Some(tip) = link.and_then(|link| links.tip(link)) {
                    let _ = write!(out, " w:tooltip=\"{}\"", escaped(&tip));
                }
                out.push('>');
                open = link;
            }
            match &whole {
                Some(whole) => out.push_str(whole),
                None => text(piece, run, open.is_some(), own, turned(at), out),
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

/// Writes one run of text with its marks, its language where that is not the document's, and that it
/// reads from the right where `rtl` says so. Marks are direct formatting, as Word's own buttons make.
fn text(text: &str, run: &Run, linked: bool, lang: Option<&str>, rtl: bool, out: &mut String) {
    out.push_str("<w:r>");
    let has = |mark: Mark| run.marks.contains(&mark);
    // A formula comes here only as its source, when it has a mistake in it, and source reads as code.
    let code = has(Mark::Code) || has(Mark::Math);
    let style = if code { Some(INLINE_CODE.0) } else { linked.then_some(LINK.0) };
    if style.is_some() || !run.marks.is_empty() || lang.is_some() || rtl {
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
        if rtl {
            out.push_str("<w:rtl/>");
        }
        if let Some(lang) = lang.map(escaped) {
            // Word keeps the language of text that reads from the right apart from that of the rest.
            let turned = if rtl { format!(" w:bidi=\"{lang}\"") } else { String::new() };
            let _ = write!(out, "<w:lang w:val=\"{lang}\"{turned}/>");
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
    let align = justification(kind.align);
    // 240 is single spacing.
    let line = (kind.line_height / SINGLE * 240.0).round().max(1.0) as u32;
    format!(
        "<w:pPr>{before}<w:spacing w:before=\"{}\" w:after=\"{}\" w:line=\"{line}\" w:lineRule=\"auto\"/>{after}\
         <w:jc w:val=\"{align}\"/>{last}</w:pPr>",
        twips(kind.space_before),
        twips(kind.space_after),
    )
}

/// How text is set across its line, as Word names it.
fn justification(align: Align) -> &'static str {
    match align {
        Align::Start => "left",
        Align::Center => "center",
        Align::End => "right",
        Align::Justify => "both",
    }
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
        if !["Normal", LIST.0, CELL.0, HEAD_CELL.0].contains(&id) {
            out.push_str("<w:next w:val=\"Normal\"/>");
        }
        let _ = write!(out, "<w:qFormat/>{look}{runs}</w:style>");
    };

    // A box is drawn with borders in its fill's colour, whose gaps to the text are its padding. Word draws
    // borders outside the text, so the paragraph is set in by as much. Both sides are as far from the text
    // as each other: Word turns the insets of a paragraph that reads from the right over and leaves its
    // borders where they are, and the box must still be one.
    let boxed = |bar: &str, fill: &str| {
        let sides = ["top", "left", "bottom", "right"].map(|side| match side {
            "left" => border(side, 18, 12, bar),
            "top" | "bottom" => border(side, 4, 9, fill),
            _ => border(side, 2, 14, fill),
        });
        format!("<w:pBdr>{}</w:pBdr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill}\"/>", sides.concat())
    };
    let inset = format!("<w:ind w:left=\"{BOX_INSET}\" w:right=\"{BOX_INSET}\"/>");
    let inset = inset.as_str();

    let quoted = format!("<w:ind w:left=\"{BOX_INSET}\"/>");
    for (key, id, name) in TYPES {
        let kind = of(key);
        let keep = "<w:keepNext/><w:keepLines/>";
        let (before, after, last, runs) = match key {
            "title" => (keep.to_owned(), "", String::new(), ""),
            "quote" => {
                let bar = format!("<w:pBdr>{}</w:pBdr>", border("left", 18, 12, &hex(PAGE.line_strong)));
                (bar, quoted.as_str(), String::new(), "")
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
        // The line that heads the box is in its accent, and stays with the text under it. Its frame is the
        // box's own, which is what makes Word draw the two as one.
        let title = TextStyle { weight: 600, color: format!("#{}", hex(look.accent)), ..body.clone() };
        let (id, name) = (format!("{id}{}", CALLOUT_TITLE.0), format!("{name}{}", CALLOUT_TITLE.1));
        let kept = format!("<w:keepNext/>{frame}");
        paragraph(&id, &name, true, &paragraph_look(&title, &kept, inset, ""), &run_look(&title, ""));
    }
    // The lines of a table of contents sit closer than paragraphs do, each level further in than the last.
    let line = TextStyle { space_after: body.space_after / 2.0, align: Align::Start, ..body.clone() };
    for (level, (id, name)) in CONTENTS.iter().enumerate() {
        let inset = format!("<w:ind w:left=\"{}\"/>", 240 * level);
        paragraph(id, name, true, &paragraph_look(&line, "", &inset, ""), &run_look(&line, ""));
    }
    // What a link's preview shows is held in a thin frame, as a card is.
    let frame = ["top", "left", "bottom", "right"]
        .map(|side| border(side, 4, if side == "top" || side == "bottom" { 9 } else { 12 }, &hex(PAGE.line)));
    let card = format!("<w:pBdr>{}</w:pBdr>", frame.concat());
    let look = paragraph_look(body, &card, "<w:ind w:left=\"250\" w:right=\"250\"/>", "");
    paragraph(PREVIEW.0, PREVIEW.1, true, &look, &run_look(body, ""));
    // Items of one list sit closer than paragraphs do.
    paragraph(LIST.0, LIST.1, true, &paragraph_look(body, "", "<w:contextualSpacing/>", ""), &run_look(body, ""));
    // The text of a cell is body text with no space around it, and bold in a row that heads the table.
    let cell = TextStyle { space_before: 0.0, space_after: 0.0, align: Align::Start, ..body.clone() };
    paragraph(CELL.0, CELL.1, true, &paragraph_look(&cell, "", "", ""), "");
    paragraph(HEAD_CELL.0, HEAD_CELL.1, true, &paragraph_look(&cell, "", "", ""), "<w:rPr><w:b/><w:bCs/></w:rPr>");
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

    // A paragraph too small to see, which parts two boxes that are framed alike.
    let _ = write!(
        out,
        "<w:style w:type=\"paragraph\" w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:basedOn w:val=\"Normal\"/>\
         <w:next w:val=\"Normal\"/><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"20\" \
         w:lineRule=\"exact\"/></w:pPr><w:rPr><w:sz w:val=\"2\"/><w:szCs w:val=\"2\"/></w:rPr></w:style>",
        BOX_SPACE.0, BOX_SPACE.1,
    );
    // A paragraph too small to see, which leaves under a table the space a paragraph leaves under itself.
    let _ = write!(
        out,
        "<w:style w:type=\"paragraph\" w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:basedOn w:val=\"Normal\"/>\
         <w:next w:val=\"Normal\"/><w:pPr><w:spacing w:before=\"{}\" w:after=\"0\" w:line=\"20\" \
         w:lineRule=\"exact\"/></w:pPr><w:rPr><w:sz w:val=\"2\"/><w:szCs w:val=\"2\"/></w:rPr></w:style>",
        TABLE_SPACE.0,
        TABLE_SPACE.1,
        (body.space_after * TWIPS).round().max(0.0) as u32,
    );
    // A table is ruled under each row, as on the page, and more firmly under a row that heads it.
    let (pad_x, pad_y) = CELL_PAD;
    let _ = write!(
        out,
        "<w:style w:type=\"table\" w:styleId=\"{}\"><w:name w:val=\"{}\"/><w:qFormat/><w:tblPr><w:tblBorders>{}{}\
         </w:tblBorders><w:tblCellMar><w:top w:w=\"{pad_y}\" w:type=\"dxa\"/><w:left w:w=\"{pad_x}\" w:type=\"dxa\"/>\
         <w:bottom w:w=\"{pad_y}\" w:type=\"dxa\"/><w:right w:w=\"{pad_x}\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr>\
         <w:tblStylePr w:type=\"firstRow\"><w:tcPr><w:tcBorders>{}</w:tcBorders></w:tcPr></w:tblStylePr></w:style>",
        TABLE.0,
        TABLE.1,
        border("bottom", 6, 0, &hex(PAGE.line)),
        border("insideH", 6, 0, &hex(PAGE.line)),
        border("bottom", 6, 0, &hex(PAGE.line_strong)),
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
