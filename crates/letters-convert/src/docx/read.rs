//! Word document to document.

mod comments;
mod inline;
mod math;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use letters_model::{Align, Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Note, Run, Style};
use roxmltree::Node;

use super::numbering::Numbering;
use super::package::{Element, Package, Rels, parse};
use super::styles::{ParaProps, Role, RunProps, Styles, Theme, page};
use super::{DocxError, TASK_DONE, TASK_OPEN};
use crate::ImportNote;

/// Text boxes, table cells and content controls nested deeper than this are passed over, which bounds the
/// reader's recursion.
const MAX_DEPTH: usize = 32;

const CONTENTS: &str = "Table of contents left out";
const TITLE: &str = "Title became a level 1 heading";
const MERGED: &str = "Merged table cells were split";
const CELLS: &str = "Lists, headings and tables inside table cells became lines of text";
const FRAME: &str = "A table of one column became its paragraphs";
const PICTURE_MOVED: &str = "Picture moved out of its paragraph";
const PICTURE_OUT: &str = "Picture moved out of its table";
const DIRECT: &str = "Fonts, sizes, colours and alignment set on single passages were left out. The document's \
                      styles set them instead";
const HEADERS: &str = "Headers and footers were left out";
const HEADING_NUMBERS: &str = "Heading numbers were left out";

/// A Word document as Letters holds one.
#[derive(Clone, Debug, PartialEq)]
pub struct DocxImport {
    pub document: Document,
    /// The style the import was given, with Word's own styles and page laid over it.
    pub style: Style,
    /// What the import simplified, each on the block it happened to.
    pub notes: Vec<ImportNote>,
    /// Word's comments, with their replies.
    pub comments: Vec<Note>,
    /// The pictures the document shows, by the entry name its blocks use.
    pub assets: BTreeMap<String, Vec<u8>>,
}

/// Reads a `.docx`.
///
/// - `base` is the style to fall back on for whatever Word's styles do not define.
/// - `now` dates a comment Word left undated, as RFC 3339.
/// - `new_id` supplies an id for each block and each comment.
///
/// Text is never dropped: what the document cannot hold is flattened and named in a note.
pub fn from_docx(
    bytes: &[u8],
    base: &Style,
    now: &str,
    new_id: &mut dyn FnMut() -> String,
) -> Result<DocxImport, DocxError> {
    let mut package = Package::open(bytes)?;
    let main = match package.rels("")?.part("officeDocument") {
        Some(main) => main.to_owned(),
        None => "word/document.xml".to_owned(),
    };
    let text = package.text(&main)?.ok_or(DocxError::NotWord)?;
    let xml = parse(&main, &text)?;
    let body = xml.root_element().child("body").ok_or(DocxError::NotWord)?;
    let rels = package.rels(&main)?;

    // The parts beside the document, each with the relationships of its own.
    let mut side = |kind: &str| -> Result<Option<(String, String, Rels)>, DocxError> {
        let Some(name) = rels.part(kind) else { return Ok(None) };
        let Some(text) = package.text(name)? else { return Ok(None) };
        Ok(Some((name.to_owned(), text, package.rels(name)?)))
    };
    let (theme, styles, numbering) = (side("theme")?, side("styles")?, side("numbering")?);
    let (footnotes, endnotes) = (side("footnotes")?, side("endnotes")?);
    let (comments, threads) = (side("comments")?, side("commentsExtended")?);
    let mut furniture = Vec::new();
    for name in rels.parts("header").chain(rels.parts("footer")) {
        furniture.extend(package.text(name)?);
    }
    fn read(part: &Option<(String, String, Rels)>) -> Result<Option<roxmltree::Document<'_>>, DocxError> {
        part.as_ref().map(|(name, text, _)| parse(name, text)).transpose()
    }

    let theme = read(&theme)?.as_ref().map(Theme::read).unwrap_or_default();
    // A file with no styles of its own takes the look it is given.
    let seeded = styles.is_some();
    let styles = read(&styles)?.as_ref().map(|xml| Styles::read(xml, &theme)).unwrap_or_default();
    let numbering = read(&numbering)?.as_ref().map(Numbering::read).unwrap_or_default();
    let mut style = if seeded { styles.seed(base) } else { base.clone() };
    if let Some(page) = body.child("sectPr").and_then(|section| page(section, &base.page)) {
        style.page = page;
    }

    let mut reader = Reader {
        package,
        normal: styles.paragraph(None),
        styles,
        theme,
        numbering,
        new_id,
        notes: Vec::new(),
        whole: Vec::new(),
        fields: Vec::new(),
        depth: 0,
        detached: false,
        plain: false,
        paras: 0,
        placed: BTreeMap::new(),
        spans: BTreeMap::new(),
        cited: Vec::new(),
        numbers: BTreeMap::new(),
        counts: BTreeMap::new(),
        assets: BTreeMap::new(),
        pictures: BTreeMap::new(),
        turns: turns(body),
        cells: 0,
        bookmarks: BTreeMap::new(),
        refs: Vec::new(),
    };
    let mut blocks = reader.blocks(body, &rels);
    // The parts read from here on count their nodes anew.
    reader.turns.clear();

    // Notes follow the text, in the order it first cites them.
    let (footnotes_xml, endnotes_xml) = (read(&footnotes)?, read(&endnotes)?);
    let mut cited = 0;
    while let Some((endnote, id)) = reader.cited.get(cited).cloned() {
        cited += 1;
        let (story, element) = if endnote { (&endnotes, "endnote") } else { (&footnotes, "footnote") };
        let xml = if endnote { &endnotes_xml } else { &footnotes_xml };
        let (Some((.., rels)), Some(xml)) = (story, xml) else { continue };
        let Some(note) = xml.root_element().all(element).find(|note| note.attr("id") == Some(&id)) else { continue };
        let first = reader.paras;
        reader.plain = true;
        let mut inner = reader.blocks(note, rels);
        reader.plain = false;
        let label = format!("[^{cited}]:");
        match inner.first_mut() {
            Some(Block { id, kind: BlockKind::Paragraph, body: Body::Text(runs), .. }) => {
                // Word starts a note's text with a space after its number.
                let (was, text) = (runs[0].text.len(), runs[0].text.trim_start().to_owned());
                runs[0].text = text;
                let cut = was - runs[0].text.len();
                runs.insert(0, Run::plain(format!("{label} ")));
                merge(runs);
                for place in reader.placed.range_mut(first..).map(|(_, place)| place).filter(|place| place.block == *id)
                {
                    place.base += label.len() + 1;
                    place.cut = cut;
                    place.len = place.len.saturating_sub(cut);
                }
            }
            _ => inner.insert(0, Block::text((reader.new_id)().as_str(), BlockKind::Paragraph, label)),
        }
        blocks.append(&mut inner);
    }
    settle(&mut blocks);
    reader.refer(&mut blocks);
    let language = languages(&mut blocks, reader.styles.language());

    if furniture.iter().any(|part| shows_text(part)) {
        reader.whole.push(HEADERS.into());
    }
    let unknown: BTreeSet<&str> = Style::TYPES
        .iter()
        .filter_map(|name| style.types.get(*name))
        .map(|kind| kind.family.as_str())
        .filter(|family| ![letters_fonts::SANS, letters_fonts::MONO, letters_fonts::DISPLAY].contains(family))
        .collect();
    if seeded && !unknown.is_empty() {
        let names = unknown.into_iter().collect::<Vec<_>>().join(", ");
        reader.whole.push(format!(
            "The style names fonts that Letters does not carry ({names}). Their text shows in the nearest face \
             Letters has, and the names are kept for export"
        ));
    }
    // What concerns the whole document sits on its first block.
    if let Some(first) = blocks.first() {
        let whole = std::mem::take(&mut reader.whole);
        let notes = whole.into_iter().map(|message| ImportNote { block: first.id.to_string(), message });
        reader.notes.splice(0..0, notes);
    }

    let mut document = Document { blocks, ..Document::default() };
    if let Some(language) = language {
        document.language = language;
    }
    let notes = match (&comments, read(&comments)?) {
        (Some((.., rels)), Some(xml)) => {
            let threads = read(&threads)?;
            reader.comments(xml.root_element(), threads.as_ref().map(|xml| xml.root_element()), rels, &document, now)
        }
        _ => Vec::new(),
    };
    Ok(DocxImport { document, style, notes: reader.notes, comments: notes, assets: reader.assets })
}

/// The paragraphs after which a new page starts because a section ends there. A section says how it starts
/// in its own properties, which sit where it ends.
fn turns(body: Node) -> BTreeSet<usize> {
    let mut turns = BTreeSet::new();
    // The paragraph that ended the section before, until the next section says how it starts.
    let mut ended = None;
    let sections = body.descendants().filter(|node| node.is("sectPr"));
    for (section, parent) in sections.filter_map(|section| Some((section, section.parent()?))) {
        let paragraph = parent.parent().filter(|paragraph| parent.is("pPr") && paragraph.is("p"));
        if paragraph.is_none() && !parent.is("body") {
            continue;
        }
        let page = !matches!(section.value("type"), Some("continuous" | "nextColumn"));
        turns.extend(ended.take().filter(|_| page));
        ended = paragraph.map(|paragraph| paragraph.id().get_usize());
    }
    turns
}

/// Settles which language each text is in, and returns the document's.
///
/// Word names a language on nearly every run, often another than the one its styles start from. So the
/// language most of the text is in is the document's, and only text in another keeps a tag: on its block
/// where the whole block agrees, and on its runs otherwise.
fn languages(blocks: &mut [Block], declared: Option<&str>) -> Option<String> {
    fn count(blocks: &[Block], declared: Option<&str>, sizes: &mut BTreeMap<Option<String>, usize>) {
        for block in blocks.iter().filter(|block| worded(block)) {
            for run in block.body.runs().iter().filter(|run| spoken(run)) {
                let language = run.lang.as_deref().or(declared).map(str::to_owned);
                *sizes.entry(language).or_default() += run.text.len();
            }
            count(block.body.blocks(), declared, sizes);
        }
    }
    /// Tags what differs from `main`, and says whether the blocks hold any words.
    fn tag(blocks: &mut [Block], declared: Option<&str>, main: Option<&str>) -> bool {
        let mut any = false;
        for block in blocks.iter_mut().filter(|block| worded(block)) {
            let worded = match &mut block.body {
                Body::Text(runs) => {
                    for run in runs.iter_mut() {
                        let language = run.lang.take().or_else(|| declared.map(str::to_owned));
                        run.lang = language.filter(|language| spoken(run) && Some(language.as_str()) != main);
                    }
                    let mut words = runs.iter().filter(|run| spoken(run)).map(|run| run.lang.clone());
                    let first = words.next();
                    if let Some(Some(shared)) = first.clone().filter(|first| words.all(|other| other == *first)) {
                        runs.iter_mut().for_each(|run| run.lang = None);
                        block.lang = Some(shared);
                    }
                    merge(runs);
                    first.is_some()
                }
                Body::Blocks(inner) => {
                    let worded = tag(inner, declared, main);
                    // A quote all in one language says so once.
                    let mut inside = inner.iter().filter(|block| words(block)).map(|block| block.lang.clone());
                    let first = inside.next();
                    if let Some(Some(shared)) = first.clone().filter(|first| inside.all(|other| other == *first)) {
                        inner.iter_mut().for_each(|block| block.lang = None);
                        block.lang = Some(shared);
                    }
                    worded
                }
                Body::None => false,
            };
            any |= worded;
        }
        any
    }
    fn words(block: &Block) -> bool {
        worded(block) && (block.body.runs().iter().any(spoken) || block.body.blocks().iter().any(words))
    }

    let mut sizes = BTreeMap::new();
    count(blocks, declared, &mut sizes);
    // Most of the text decides, and what the styles say settles a tie.
    let declared_key = declared.map(str::to_owned);
    let most = sizes.iter().max_by_key(|(language, size)| (**size, **language == declared_key));
    let main = most.and_then(|(language, _)| language.clone()).or(declared_key);
    tag(blocks, declared, main.as_deref());
    main
}

/// Whether a block can hold words in some language: code and maths are in none.
fn worded(block: &Block) -> bool {
    !matches!(block.kind, BlockKind::Code { .. } | BlockKind::Formula)
}

/// Whether a run holds words in some language: code and maths are in none.
fn spoken(run: &Run) -> bool {
    !run.text.trim().is_empty() && !run.marks.contains(&Mark::Code) && !run.marks.contains(&Mark::Math)
}

/// Whether a header or footer shows any words or numbers.
fn shows_text(part: &str) -> bool {
    let Ok(xml) = roxmltree::Document::parse(part.trim_start_matches('\u{feff}')) else { return false };
    xml.descendants().filter(|node| node.is("t")).any(|node| text_of(node).chars().any(char::is_alphanumeric))
}

/// The text an element holds directly.
fn text_of(node: Node) -> String {
    node.children().filter_map(|child| child.text()).collect()
}

/// Joins neighbouring runs that share their formatting.
fn merge(runs: &mut Vec<Run>) {
    runs.dedup_by(|next, last| {
        let same = last.marks == next.marks && last.link == next.link && last.lang == next.lang;
        if same {
            last.text.push_str(&next.text);
        }
        same
    });
}

/// Tidies blocks once they are all read: list items nest one level at a time, and code ends on its last line.
fn settle(blocks: &mut [Block]) {
    // The levels Word set the items above this one at, outermost first. Items of one level stay siblings.
    let mut levels: Vec<u8> = Vec::new();
    for block in blocks {
        match &mut block.kind {
            BlockKind::ListItem { depth, .. } => {
                while levels.last().is_some_and(|level| *level > *depth) {
                    levels.pop();
                }
                if levels.last() != Some(depth) {
                    levels.push(*depth);
                }
                *depth = (levels.len() - 1) as u8;
            }
            _ => levels.clear(),
        }
        match &mut block.body {
            Body::Blocks(children) => settle(children),
            Body::Text(runs) if matches!(block.kind, BlockKind::Code { .. }) => {
                if let Some(run) = runs.last_mut() {
                    run.text.truncate(run.text.trim_end_matches('\n').len());
                }
            }
            _ => {}
        }
    }
}

/// A place in the text as it is read: a paragraph, and bytes into it.
#[derive(Clone, Copy, Debug)]
struct Spot {
    para: usize,
    at: usize,
}

/// Where a paragraph's text ended up.
#[derive(Clone, Debug)]
struct Place {
    block: BlockId,
    /// Bytes into the block's text at which the paragraph starts.
    base: usize,
    /// Bytes taken off the start of the paragraph on its way into the block.
    cut: usize,
    len: usize,
}

/// A field: instructions to Word, then what they last produced.
#[derive(Default)]
struct Field {
    code: String,
    /// Whether the instructions are over and what follows is shown.
    shown: bool,
    /// The address, for a field that is a link.
    link: Option<String>,
    /// Whether the field is a table of contents.
    contents: bool,
    /// Where what it shows starts.
    from: Option<Spot>,
}

/// What surrounds the inline content being read.
struct Inline<'r> {
    rels: &'r Rels,
    link: Option<String>,
    depth: usize,
}

impl Inline<'_> {
    fn deeper(&self, link: Option<String>) -> Self {
        Self { rels: self.rels, link: link.or_else(|| self.link.clone()), depth: self.depth + 1 }
    }
}

/// One paragraph as it is read.
struct Para {
    index: usize,
    role: Role,
    /// What text of this paragraph's style looks like in Word.
    styled: RunProps,
    /// What text of the block this becomes will look like, which marks are measured against.
    baseline: RunProps,
    runs: Vec<Run>,
    /// Bytes of text read so far.
    len: usize,
    notes: Vec<String>,
    /// Blocks that follow the paragraph: its pictures and the text of its text boxes.
    after: Vec<Block>,
    /// How many of `after` are pictures that sat in this paragraph.
    pictures: Vec<BlockId>,
    /// The state of a checkbox the paragraph opens with.
    task: Option<bool>,
    /// Whether Word drew a rule across the page here.
    ruled: bool,
    /// Whether any of it belongs to a table of contents.
    contents: bool,
    /// How deep a table of contents that starts in it lists, where a field says that it is one.
    listing: Option<u8>,
    /// Whether it lies in a table of contents that an earlier paragraph started.
    listed: bool,
    /// The language of the run being read.
    lang: Option<String>,
    /// Whether a page break comes before any of the text.
    breaks: bool,
    /// Whether a page break comes after some of the text.
    broken: bool,
    /// The bytes of the number Word counted this paragraph with, as it does a caption.
    counted: Option<Range<usize>>,
}

impl Para {
    fn push(&mut self, text: &str, marks: &BTreeSet<Mark>, link: Option<String>) {
        if text.is_empty() {
            return;
        }
        self.len += text.len();
        let lang = self.lang.clone();
        match self.runs.last_mut() {
            Some(last) if last.marks == *marks && last.link == link && last.lang == lang => {
                // Two formulas side by side stay two terms of one source.
                if marks.contains(&Mark::Math) {
                    last.text.push(' ');
                    self.len += 1;
                }
                last.text.push_str(text);
            }
            _ => self.runs.push(Run { text: text.into(), marks: marks.clone(), link, lang, ..Run::default() }),
        }
    }

    /// Notes something about the block this becomes, once however often it happens.
    fn note(&mut self, message: &str) {
        if !self.notes.iter().any(|held| held == message) {
            self.notes.push(message.into());
        }
    }

    fn is_blank(&self) -> bool {
        self.runs.iter().all(|run| run.text.trim().is_empty())
    }
}

/// What the last block of a list of blocks is still gathering paragraphs into.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Open {
    #[default]
    Nothing,
    Quote,
    Callout(letters_model::Tone),
    Code,
}

/// The blocks read so far among siblings.
#[derive(Default)]
struct Out {
    blocks: Vec<Block>,
    open: Open,
    /// Notes waiting for the next block, the nearest place to what they are about.
    pending: Vec<&'static str>,
    /// The numbers the page will give the list being read, level by level.
    counters: Vec<u32>,
    /// Whether the next block starts a new page.
    broken: bool,
}

struct Reader<'a, 'f> {
    package: Package<'a>,
    styles: Styles,
    theme: Theme,
    numbering: Numbering,
    /// What a paragraph of the default style looks like.
    normal: (ParaProps, RunProps),
    new_id: &'f mut dyn FnMut() -> String,
    notes: Vec<ImportNote>,
    /// Notes about the document as a whole.
    whole: Vec<String>,
    /// The fields open at this point, outermost first. A field can run across paragraphs.
    fields: Vec<Field>,
    /// How many text boxes, cells and controls enclose what is being read.
    depth: usize,
    /// Set while reading text that joins no block, such as the words of a comment.
    detached: bool,
    /// Set while reading text whose look in Word is beside the point, such as a footnote's.
    plain: bool,
    /// Paragraphs read so far.
    paras: usize,
    placed: BTreeMap<usize, Place>,
    /// Where each comment starts and ends, by its id.
    spans: BTreeMap<String, (Option<Spot>, Option<Spot>)>,
    /// The notes cited so far, in order: whether each is an endnote, and its id.
    cited: Vec<(bool, String)>,
    numbers: BTreeMap<(bool, String), usize>,
    /// The number Word will give the next item at each level of each list.
    counts: BTreeMap<String, Vec<Option<u32>>>,
    assets: BTreeMap<String, Vec<u8>>,
    /// The asset each picture part became, or why it became none, so that a part is read once.
    pictures: BTreeMap<String, Result<String, String>>,
    /// The paragraphs of the body after which a section starts a new page, by the number of their node.
    turns: BTreeSet<usize>,
    /// How many table cells enclose what is being read.
    cells: usize,
    /// The paragraph each bookmark starts in.
    bookmarks: BTreeMap<String, usize>,
    /// Each cross-reference: where what it shows starts and ends, and the bookmark it points at.
    refs: Vec<(Spot, usize, String)>,
}

impl Reader<'_, '_> {
    fn id(&mut self) -> BlockId {
        BlockId::new((self.new_id)())
    }

    fn whole(&mut self, message: &str) {
        if !self.whole.iter().any(|held| held == message) {
            self.whole.push(message.into());
        }
    }

    fn note(&mut self, block: &BlockId, message: &str) {
        if !self.detached {
            self.notes.push(ImportNote { block: block.to_string(), message: message.into() });
        }
    }

    /// Adds a block, with the notes and the page break that were waiting for one.
    fn push(&mut self, mut block: Block, out: &mut Out) {
        block.page_break |= std::mem::take(&mut out.broken);
        for message in std::mem::take(&mut out.pending) {
            self.note(&block.id, message);
        }
        out.blocks.push(block);
    }

    /// Reads the blocks inside a body, a text box, a table cell, a note or a comment.
    fn blocks(&mut self, parent: Node, rels: &Rels) -> Vec<Block> {
        let mut out = Out::default();
        self.depth += 1;
        if self.depth <= MAX_DEPTH {
            self.children(parent, rels, &mut out);
        }
        self.depth -= 1;
        // A note with no block after it goes on the last one.
        if let Some(last) = out.blocks.last().map(|block| block.id.clone()) {
            for message in std::mem::take(&mut out.pending) {
                self.note(&last, message);
            }
        }
        out.blocks
    }

    fn children(&mut self, parent: Node, rels: &Rels, out: &mut Out) {
        for node in parent.children() {
            match node.word() {
                Some("p") => self.paragraph(node, rels, out),
                Some("tbl") => self.table(node, rels, out),
                Some("sdt") => {
                    let gallery = node.child("sdtPr").and_then(|props| props.child("docPartObj"));
                    let contents = gallery.and_then(|part| part.value("docPartGallery")) == Some("Table of Contents");
                    if contents {
                        // Word's own table of contents, which holds the field that makes it.
                        let code =
                            node.descendants().filter(|part| part.is("instrText")).filter_map(|part| part.text());
                        let levels = inline::levels(&code.collect::<String>());
                        let table = Block::new(self.id(), BlockKind::TableOfContents { levels });
                        self.close(out);
                        self.push(table, out);
                    } else if let Some(content) = node.child("sdtContent") {
                        self.wrapped(content, rels, out);
                    }
                }
                Some("ins" | "moveTo" | "customXml" | "smartTag") => self.wrapped(node, rels, out),
                // A bookmark between paragraphs marks the one after it.
                Some("bookmarkStart") => {
                    if let Some(name) = node.attr("name") {
                        self.bookmarks.entry(name.to_owned()).or_insert(self.paras);
                    }
                }
                // A comment can start and end between paragraphs.
                Some("commentRangeStart") => self.span(node).0 = Some(Spot { para: self.paras, at: 0 }),
                Some("commentRangeEnd") => {
                    self.span(node).1 = Some(Spot { para: self.paras.saturating_sub(1), at: usize::MAX });
                }
                _ => {}
            }
        }
    }

    /// Reads blocks that something wraps without being a block itself.
    fn wrapped(&mut self, wrapper: Node, rels: &Rels, out: &mut Out) {
        self.depth += 1;
        if self.depth <= MAX_DEPTH {
            self.children(wrapper, rels, out);
        }
        self.depth -= 1;
    }

    fn span(&mut self, mark: Node) -> &mut (Option<Spot>, Option<Spot>) {
        self.spans.entry(mark.attr("id").unwrap_or_default().to_owned()).or_default()
    }

    fn paragraph(&mut self, node: Node, rels: &Rels, out: &mut Out) {
        let index = self.paras;
        self.paras += 1;
        let direct = node.child("pPr").map(ParaProps::read).unwrap_or_default();
        let (style, styled) = self.styles.paragraph(direct.style.as_deref());
        let props = direct.clone().over(&style);
        let role = match self.styles.role(direct.style.as_deref()) {
            // A style Word files under a heading level is a heading, whatever it is called.
            Role::Body => props.outline.filter(|level| *level < 9).map_or(Role::Body, |level| Role::Heading(level + 1)),
            role => role,
        };
        // Body text is measured against the default style, so what another style adds to it shows as marks.
        let (expected, baseline) = match role {
            Role::Body => (self.normal.0.align, self.normal.1.clone()),
            // The page sets the row that heads a table itself. Anywhere else the style's look is marks.
            Role::ColumnHead if self.cells == 0 => (self.normal.0.align, self.normal.1.clone()),
            _ => (style.align, styled.clone()),
        };
        let mut para = Para {
            index,
            role,
            styled,
            baseline,
            runs: Vec::new(),
            len: 0,
            notes: Vec::new(),
            after: Vec::new(),
            pictures: Vec::new(),
            task: None,
            ruled: direct.ruled == Some(true),
            contents: role == Role::Contents || self.fields.iter().any(|field| field.contents),
            listing: None,
            listed: self.fields.iter().any(|field| field.contents),
            lang: None,
            breaks: false,
            broken: false,
            counted: None,
        };
        self.inline(node, &Inline { rels, link: None, depth: 0 }, &mut para);
        // How a cell is aligned is its column's to say.
        let aligned = self.cells > 0 || props.align.unwrap_or(Align::Start) == expected.unwrap_or(Align::Start);
        if !para.is_blank() && !aligned {
            self.whole(DIRECT);
        }
        // A break before the text starts this block on a new page, and one after it the block that follows.
        out.broken |= para.breaks || props.page_break == Some(true);
        let later = para.broken || self.turns.contains(&node.id().get_usize());
        self.emit(para, &props, out);
        out.broken |= later;
    }

    /// Turns a paragraph into its block, or into more of the block before it.
    fn emit(&mut self, mut para: Para, props: &ParaProps, out: &mut Out) {
        if para.contents {
            if let Some(levels) = para.listing {
                // A field makes the table, so it is one block. Its lines are what Word last made of it,
                // and the page makes its own.
                let table = Block::new(self.id(), BlockKind::TableOfContents { levels });
                self.close(out);
                self.push(table, out);
            } else if !para.listed && !out.pending.contains(&CONTENTS) {
                // Lines set as a table of contents with no field behind them say nothing of what they list.
                out.pending.push(CONTENTS);
            }
            // What floats beside an entry is no part of the table.
            if !para.after.is_empty() {
                self.close(out);
            }
            for block in std::mem::take(&mut para.after) {
                self.push(block, out);
            }
            return;
        }
        let blank = para.is_blank();
        let level = props.list.as_ref().filter(|(list, _)| list != "0").map(|(list, depth)| {
            let level = self.numbering.level(list, *depth);
            // Word counts on through a list wherever its items sit.
            let counts = self.counts.entry(list.clone()).or_default();
            let at = usize::from(*depth);
            let reached = counts.get(at).copied().flatten().unwrap_or(level.start);
            // An item under this one starts its own count again.
            counts.resize(at + 1, None);
            counts[at] = Some(if blank { reached } else { reached.saturating_add(1) });
            (level, *depth, reached)
        });
        let named_depth = self.styles.depth(props.style.as_deref());
        let level = level.map(|(level, depth, reached)| (level, depth.max(named_depth), reached));
        let mut after = std::mem::take(&mut para.after);
        let mut runs = std::mem::take(&mut para.runs);
        // Tools without a checkbox of their own write a task as a bullet whose text opens with a box.
        let mut cut = 0;
        if para.task.is_none() && level.is_some_and(|(level, ..)| level.kind == Some(ListKind::Bullet)) {
            let boxes = [(TASK_OPEN, false), (TASK_DONE, true), ('\u{2612}', true)];
            let first = runs.first_mut().and_then(|run| {
                let (_, checked) = boxes.iter().find(|(glyph, _)| run.text.starts_with(*glyph))?;
                let rest = run.text[TASK_OPEN.len_utf8()..].strip_prefix([' ', '\u{a0}', '\t'])?.to_owned();
                cut = run.text.len() - rest.len();
                run.text = rest;
                Some(*checked)
            });
            para.task = first;
        }
        // The space between a checkbox and its text is not part of the text.
        if para.task.is_some() && cut == 0 {
            if let Some(run) = runs.first_mut() {
                let rest = run.text.trim_start_matches([' ', '\u{a0}', '\t']).to_owned();
                cut = run.text.len() - rest.len();
                run.text = rest;
            }
            runs.retain(|run| !run.text.is_empty());
        }
        // A caption under a picture is the picture's own text, and the page writes its number.
        let captioned = !blank && para.role == Role::Caption && captions(out.blocks.last()) && after.is_empty();
        let mut numbered = false;
        if captioned {
            let text: String = runs.iter().map(|run| run.text.as_str()).collect();
            cut = label(&text, para.counted.as_ref());
            numbered = cut > 0;
            skip(&mut runs, cut);
        }
        let place = |reader: &mut Self, block: &BlockId, base: usize| {
            if !reader.detached {
                reader.placed.insert(para.index, Place { block: block.clone(), base, cut, len: para.len - cut });
            }
        };

        let mut notes = std::mem::take(&mut para.notes);
        if blank && para.role == Role::Code && out.open == Open::Code {
            // An empty line of code is still a line.
            if let Some(Body::Text(code)) = out.blocks.last_mut().map(|block| &mut block.body) {
                code.push(Run::plain("\n"));
                merge(code);
            }
        } else if blank && (para.ruled || para.role == Role::Divider) {
            let block = Block::new(self.id(), BlockKind::Divider);
            place(self, &block.id, 0);
            self.close(out);
            self.push(block, out);
        } else if blank {
            // Word spaces text with empty paragraphs. Here the style does that. One that is not set as the
            // quote, the callout or the code above it is where that ends.
            let within = match para.role {
                Role::Quote => Open::Quote,
                Role::Callout(tone) | Role::CalloutTitle(tone) => Open::Callout(tone),
                Role::Code => Open::Code,
                _ => Open::Nothing,
            };
            if within != out.open {
                self.close(out);
            }
        } else if para.role == Role::Code {
            let text: String = runs.iter().map(|run| run.text.as_str()).collect();
            // Code that starts a new page is a block of its own.
            let open = out.blocks.last_mut().filter(|_| out.open == Open::Code && !out.broken);
            if let Some(Block { id, body: Body::Text(code), .. }) = open {
                let base = code.iter().map(|run| run.text.len()).sum::<usize>() + 1;
                code.push(Run::plain(format!("\n{text}")));
                merge(code);
                let id = id.clone();
                place(self, &id, base);
                for message in &notes {
                    if !self.notes.iter().any(|held| held.block == id.as_str() && held.message == *message) {
                        self.note(&id, message);
                    }
                }
            } else {
                self.close(out);
                let block = Block::text(self.id(), BlockKind::Code { language: None }, text);
                place(self, &block.id, 0);
                self.attach(&block.id, &notes);
                self.push(block, out);
                out.open = Open::Code;
            }
        } else if let Role::CalloutTitle(tone) = para.role {
            // The line that heads a callout starts one. It is the callout's title where it is not its tone's
            // own, and a comment on it moves to the text under it.
            let title = runs.iter().map(|run| run.text.as_str()).collect::<String>().replace(['\n', '\t'], " ");
            let title = (title != tone.title()).then_some(title);
            let mut callout = Block::new(self.id(), BlockKind::Callout { tone, title, icon: None, color: None });
            callout.body = Body::Blocks(Vec::new());
            self.attach(&callout.id, &notes);
            self.close(out);
            self.push(callout, out);
            out.open = Open::Callout(tone);
        } else if captioned {
            merge(&mut runs);
            let caption = Block { body: Body::Text(runs), ..Block::new(self.id(), BlockKind::Caption) };
            let pictured = out.blocks.last_mut().expect("a picture was just found");
            let id = match (&mut pictured.kind, &mut pictured.body) {
                (BlockKind::Gallery { numbered: counted }, Body::Blocks(inner)) => {
                    *counted = numbered;
                    inner.push(caption);
                    inner.last().map(|caption| caption.id.clone())
                }
                (BlockKind::Figure { numbered: counted, .. }, body) => {
                    *counted = numbered;
                    *body = caption.body;
                    Some(pictured.id.clone())
                }
                _ => None,
            };
            if let Some(id) = id {
                place(self, &id, 0);
                self.attach(&id, &notes);
            }
        } else {
            let list = match (para.task, level) {
                (Some(checked), level) => Some((ListKind::Task { checked }, level.map_or(0, |(_, depth, _)| depth))),
                (None, Some((level, depth, _))) => level.kind.map(|kind| (kind, depth)),
                (None, None) => None,
            };
            let item = list.map(|(list, depth)| BlockKind::ListItem { list, depth });
            // An equation with a paragraph to itself is set as a display.
            let alone = matches!(runs.as_slice(), [Run { marks, link: None, .. }] if marks.contains(&Mark::Math));
            let text = item.unwrap_or(match para.role {
                _ if alone => BlockKind::Formula,
                // A caption with no picture over it is still a line in the caption style.
                Role::Caption => BlockKind::Caption,
                _ => BlockKind::Paragraph,
            });
            let (kind, wrapper) = match para.role {
                Role::Heading(level) => {
                    if level > 4 {
                        notes.push(format!("Heading level {level} became level 4"));
                    }
                    (BlockKind::Heading { level: level.min(4) }, Open::Nothing)
                }
                Role::Title => {
                    notes.push(TITLE.into());
                    (BlockKind::Heading { level: 1 }, Open::Nothing)
                }
                Role::Quote => (text, Open::Quote),
                Role::Callout(tone) => (text, Open::Callout(tone)),
                _ => (text, Open::Nothing),
            };
            if kind == BlockKind::Formula {
                // A formula's text is its source, and no run of it is marked as one.
                runs.iter_mut().for_each(|run| run.marks.clear());
            }
            let numbered = level.is_some_and(|(level, ..)| level.kind == Some(ListKind::Ordered));
            if matches!(kind, BlockKind::Heading { .. }) && numbered {
                self.whole(HEADING_NUMBERS);
            }
            if wrapper != out.open {
                // A list does not run on from the page into a quote, or out of one.
                out.counters.clear();
                self.close(out);
            }

            // The page numbers a list from 1 wherever it starts, and Word may not.
            match &kind {
                BlockKind::ListItem { list, depth } => {
                    let depth = usize::from(*depth);
                    out.counters.resize(depth + 1, 0);
                    out.counters[depth] = if *list == ListKind::Ordered { out.counters[depth] + 1 } else { 0 };
                    let reached = level.map_or(1, |(.., reached)| reached);
                    if *list == ListKind::Ordered && out.counters[depth] == 1 && reached != 1 {
                        notes.push(format!("Numbering started at {reached} and now starts at 1"));
                    }
                }
                _ => out.counters.clear(),
            }

            let mut block = Block::new(self.id(), kind);
            block.body = Body::Text(runs);
            place(self, &block.id, 0);
            self.attach(&block.id, &notes);
            match wrapper {
                Open::Nothing => self.push(block, out),
                _ if out.open == wrapper => {
                    block.page_break = std::mem::take(&mut out.broken);
                    if let Some(Body::Blocks(inner)) = out.blocks.last_mut().map(|block| &mut block.body) {
                        inner.push(block);
                    }
                }
                _ => {
                    let kind = match wrapper {
                        // No line heads it in Word, so none does on the page.
                        Open::Callout(tone) => {
                            BlockKind::Callout { tone, title: Some(String::new()), icon: None, color: None }
                        }
                        _ => BlockKind::Quote,
                    };
                    let mut outer = Block::new(self.id(), kind);
                    outer.body = Body::Blocks(vec![block]);
                    self.push(outer, out);
                    out.open = wrapper;
                }
            }
        }

        let pictures = after.iter().all(|block| matches!(block.kind, BlockKind::Figure { .. }));
        if blank && after.len() > 1 && pictures {
            // Pictures that share a paragraph and nothing else stand side by side.
            self.close(out);
            let mut gallery = Block::new(self.id(), BlockKind::Gallery { numbered: false });
            gallery.body = Body::Blocks(std::mem::take(&mut after));
            self.push(gallery, out);
        }
        if !after.is_empty() {
            self.close(out);
            for block in after.drain(..) {
                if !blank && para.pictures.contains(&block.id) {
                    self.note(&block.id, PICTURE_MOVED);
                }
                self.push(block, out);
            }
        }
    }

    /// Ends whatever the last block was gathering.
    fn close(&mut self, out: &mut Out) {
        out.open = Open::Nothing;
    }

    fn attach(&mut self, block: &BlockId, notes: &[String]) {
        for message in notes {
            self.note(block, message);
        }
    }

    /// Reads a table. A cell holds text alone, so whatever else one held becomes lines of it.
    fn table(&mut self, table: Node, rels: &Rels, out: &mut Out) {
        self.close(out);
        let rows = parts(table, "tr", "tbl");
        if rows.iter().all(|row| parts(*row, "tc", "tr").len() < 2) {
            // A table of one column frames its paragraphs, which read on as they are.
            out.pending.push(FRAME);
            for cell in rows.iter().flat_map(|row| parts(*row, "tc", "tr")) {
                self.wrapped(cell, rels, out);
            }
            self.close(out);
            return;
        }

        let on =
            |switch: Option<Node>| switch.is_some_and(|switch| !matches!(switch.val(), Some("0" | "false" | "off")));
        let heads = rows.first().and_then(|row| row.child("trPr")).and_then(|props| props.child("tblHeader"));
        let style = table.child("tblPr").and_then(|props| props.value("tblStyle"));
        let header = on(heads) || first_row(table) && style.is_some_and(|style| self.styles.heads(style));
        let (mut merged, mut flattened) = (false, false);
        let mut columns: Vec<Option<Align>> = Vec::new();
        let mut lines = Vec::new();
        let mut after = Vec::new();
        self.cells += 1;
        for row in rows {
            let skipped = row.child("trPr").and_then(|props| props.number::<usize>("gridBefore")).unwrap_or(0);
            let mut line: Vec<Block> = (0..skipped.min(MAX_SPAN)).map(|_| self.cell(Vec::new())).collect();
            for cell in parts(row, "tc", "tr") {
                let props = cell.child("tcPr");
                let across = props.and_then(|props| props.number::<usize>("gridSpan")).unwrap_or(1).clamp(1, MAX_SPAN);
                merged |= across > 1 || props.is_some_and(|props| props.child("vMerge").is_some());
                let (from, noted) = (self.paras, self.notes.len());
                let blocks = self.blocks(cell, rels);
                // Paragraphs become lines and a formula stays one inside its line, which is no loss to name.
                let kept = |kind: &BlockKind| {
                    use BlockKind::{Figure, Formula, Gallery, Paragraph};
                    matches!(kind, Paragraph | Formula | Figure { .. } | Gallery { .. })
                };
                flattened |= blocks.iter().any(|block| !kept(&block.kind));
                let (mut runs, mut moved) = (Vec::new(), BTreeMap::new());
                flatten(blocks, &mut runs, &mut moved, &mut after);
                merge(&mut runs);
                let block = self.cell(runs);
                self.replace(from, noted, &moved, &block.id);
                columns.resize(columns.len().max(line.len() + across), None);
                // The first cell of a column to say how it is aligned says it for the column.
                let align = cell.children().find(|inner| inner.is("p")).and_then(|first| {
                    let direct = first.child("pPr").map(ParaProps::read).unwrap_or_default();
                    let (style, _) = self.styles.paragraph(direct.style.as_deref());
                    direct.over(&style).align
                });
                columns[line.len()] = columns[line.len()].or(align);
                line.push(block);
                line.extend((1..across).map(|_| self.cell(Vec::new())));
            }
            let mut row = Block::new(self.id(), BlockKind::TableRow);
            row.body = Body::Blocks(line);
            lines.push(row);
        }
        self.cells -= 1;

        let columns = columns.into_iter().map(|align| align.unwrap_or(Align::Start)).collect();
        let mut block = Block::new(self.id(), BlockKind::Table { columns, header });
        block.body = Body::Blocks(lines);
        let id = block.id.clone();
        self.push(block, out);
        for (happened, message) in [(merged, MERGED), (flattened, CELLS)] {
            if happened {
                self.note(&id, message);
            }
        }
        for block in after {
            self.note(&block.id, PICTURE_OUT);
            self.push(block, out);
        }
    }

    fn cell(&mut self, runs: Vec<Run>) -> Block {
        Block { body: Body::Text(runs), ..Block::new(self.id(), BlockKind::TableCell) }
    }

    /// Makes each cross-reference to a picture's caption a reference to the picture.
    fn refer(&mut self, blocks: &mut [Block]) {
        fn find<'b>(blocks: &'b mut [Block], id: &BlockId) -> Option<&'b mut Block> {
            blocks.iter_mut().find_map(|block| {
                if block.id == *id {
                    return Some(block);
                }
                match &mut block.body {
                    Body::Blocks(inner) => find(inner, id),
                    _ => None,
                }
            })
        }
        /// The picture or the pictures a caption belongs to.
        fn pictured(blocks: &[Block], caption: &BlockId) -> Option<BlockId> {
            blocks.iter().find_map(|block| match (&block.kind, &block.body) {
                (BlockKind::Figure { .. }, _) => (block.id == *caption).then(|| block.id.clone()),
                (BlockKind::Gallery { .. }, Body::Blocks(inner)) => {
                    inner.iter().any(|held| held.id == *caption).then(|| block.id.clone())
                }
                (_, Body::Blocks(inner)) => pictured(inner, caption),
                _ => None,
            })
        }
        for (from, to, bookmark) in std::mem::take(&mut self.refs) {
            let caption = self.bookmarks.get(&bookmark).and_then(|para| self.placed.get(para));
            let Some(target) = caption.and_then(|place| pictured(blocks, &place.block)) else { continue };
            let Some((place, start)) = self.locate(from) else { continue };
            let Some((_, end)) = self.locate(Spot { para: from.para, at: to }) else { continue };
            if let Some(Block { body: Body::Text(runs), .. }) = find(blocks, &place.block.clone()) {
                point(runs, start..end, &target);
            }
        }
    }

    /// Moves the places and notes of blocks that were flattened into `into`: those of the paragraphs read
    /// since `from`, and the notes made since there were `noted` of them.
    fn replace(&mut self, from: usize, noted: usize, moved: &BTreeMap<BlockId, usize>, into: &BlockId) {
        for (_, place) in self.placed.range_mut(from..) {
            if let Some(base) = moved.get(&place.block) {
                place.base += base;
                place.block = into.clone();
            }
        }
        let mut seen = BTreeSet::new();
        let mut recent = self.notes.split_off(noted.min(self.notes.len()));
        for note in &mut recent {
            if moved.contains_key(&BlockId::new(&note.block)) {
                note.block = into.to_string();
            }
        }
        recent.retain(|note| note.block != into.as_str() || seen.insert(note.message.clone()));
        self.notes.append(&mut recent);
    }
}

/// Whether the last block is a picture, or pictures side by side, with no caption yet.
fn captions(last: Option<&Block>) -> bool {
    last.is_some_and(|block| match &block.kind {
        BlockKind::Figure { .. } => block.body.runs().is_empty(),
        BlockKind::Gallery { .. } => !block.body.blocks().iter().any(|held| held.kind == BlockKind::Caption),
        _ => false,
    })
}

/// How many bytes at the start of a caption are a label and a number, such as "Figure 2: ", which the page
/// writes by itself. Word counts with a field after one word. A caption typed by hand reads the same only
/// where the number ends it or a mark follows, so that "Figure 2 shows the lot" stays as it is.
fn label(text: &str, counted: Option<&Range<usize>>) -> usize {
    let typed = counted.is_none();
    let end = match counted {
        Some(counted) => {
            let word = text.get(..counted.start).unwrap_or_default().trim_end();
            (!word.is_empty() && !word.contains(char::is_whitespace)).then_some(counted.end)
        }
        None => text.strip_prefix("Figure ").and_then(|rest| {
            let digits = rest.len() - rest.trim_start_matches(|digit: char| digit.is_ascii_digit()).len();
            (digits > 0).then_some("Figure ".len() + digits)
        }),
    };
    let Some(rest) = end.and_then(|end| text.get(end..)) else { return 0 };
    let rest = rest.trim_start();
    let marked = rest.strip_prefix([':', '.', '-', '\u{2013}', '\u{2014}']);
    if typed && marked.is_none() && !rest.is_empty() {
        return 0;
    }
    text.len() - marked.unwrap_or(rest).trim_start().len()
}

/// Takes bytes off the start of runs.
fn skip(runs: &mut Vec<Run>, mut bytes: usize) {
    for run in runs.iter_mut() {
        let cut = bytes.min(run.text.len());
        if run.text.is_char_boundary(cut) {
            run.text.drain(..cut);
        }
        bytes -= cut;
    }
    runs.retain(|run| !run.text.is_empty());
}

/// Makes a passage of runs a reference to a figure, cutting the runs it starts and ends in. A run refers to
/// one figure, so a reference inside another gives way to it.
fn point(runs: &mut Vec<Run>, passage: Range<usize>, figure: &BlockId) {
    if passage.is_empty() {
        return;
    }
    let mut at = 0;
    let mut cut = Vec::with_capacity(runs.len() + 2);
    for run in runs.drain(..) {
        let (start, end) = (at, at + run.text.len());
        at = end;
        // Where the passage starts and ends inside this run, if it does.
        let inside =
            |edge: usize| (edge > start && edge < end && run.text.is_char_boundary(edge - start)).then(|| edge - start);
        let mut edges = vec![0];
        edges.extend(inside(passage.start));
        edges.extend(inside(passage.end));
        edges.push(run.text.len());
        for pair in edges.windows(2).filter(|pair| pair[0] < pair[1]) {
            let mut piece = Run { text: run.text[pair[0]..pair[1]].to_owned(), ..run.clone() };
            if start + pair[0] >= passage.start && start + pair[1] <= passage.end {
                piece.marks.retain(|held| !matches!(held, Mark::Ref(_)));
                piece.marks.insert(Mark::Ref(figure.clone()));
            }
            cut.push(piece);
        }
    }
    *runs = cut;
}

/// The rows of a table or the cells of a row, whatever wraps them, and none of a table further in.
fn parts<'a, 'input>(of: Node<'a, 'input>, name: &str, outer: &str) -> Vec<Node<'a, 'input>> {
    let own = |node: &Node| node.ancestors().skip(1).find(|above| above.is(outer)) == Some(of);
    of.descendants().filter(|node| node.is(name) && own(node)).collect()
}

/// The most columns one cell is taken to span. A file can claim any number.
const MAX_SPAN: usize = 64;

/// Whether a table asks for the look its style gives a first row.
fn first_row(table: Node) -> bool {
    let Some(look) = table.child("tblPr").and_then(|props| props.child("tblLook")) else { return false };
    match look.attr("firstRow") {
        Some(first) => matches!(first, "1" | "true" | "on"),
        // Older files pack the switches into one number.
        None => look.val().and_then(|bits| u32::from_str_radix(bits, 16).ok()).is_some_and(|bits| bits & 0x20 != 0),
    }
}

/// Joins the text of blocks into one list of runs, a line for each block. Pictures cannot join text, so they
/// are set aside. `moved` records where each block's text starts in the result.
fn flatten(blocks: Vec<Block>, runs: &mut Vec<Run>, moved: &mut BTreeMap<BlockId, usize>, aside: &mut Vec<Block>) {
    for block in blocks {
        if matches!(block.kind, BlockKind::Figure { .. } | BlockKind::Gallery { .. }) {
            aside.push(block);
            continue;
        }
        match block.body {
            Body::Text(mut text) => {
                // A formula keeps being one inside the line.
                if block.kind == BlockKind::Formula {
                    text.iter_mut().for_each(|run| _ = run.marks.insert(Mark::Math));
                }
                let len: usize = runs.iter().map(|run| run.text.len()).sum();
                let fresh = runs.last().is_none_or(|last| last.text.ends_with(['\t', '\n']));
                if !fresh {
                    runs.push(Run::plain("\n"));
                }
                moved.insert(block.id, len + usize::from(!fresh));
                runs.extend(text);
            }
            Body::Blocks(inner) => {
                moved.insert(block.id, runs.iter().map(|run| run.text.len()).sum());
                flatten(inner, runs, moved, aside);
            }
            // What holds no text leaves none, and what was noted on it is noted where it stood.
            Body::None => _ = moved.insert(block.id, runs.iter().map(|run| run.text.len()).sum()),
        }
    }
}
