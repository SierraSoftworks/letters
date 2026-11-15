//! Document to Markdown.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use letters_model::{
    Align, Block, BlockId, BlockKind, Body, BodyShape, Document, ListKind, Mark, Preview, Properties, PropertiesFormat,
    Run, Tone,
};

use super::read::{FENCES, front_matter};
use super::{TAGS, TONES, from_markdown};
use crate::figures;

/// Writes Markdown. Reading the result back gives an equal document, apart from block ids and what Markdown
/// cannot say:
///
/// - Emphasis and maths never start or end on a space, so spaces at the edge of a bold, italic or struck span
///   or of maths leave it. Inline code and maths cannot hold a line break, so the break leaves them too.
/// - Maths that dollar signs cannot hold, such as a brace with no partner, is written as code. A formula they
///   cannot hold is written as a code block in the language `math`, which reads back as a formula, as any
///   code block in that language does.
/// - An empty paragraph is left out, and a list item sits at most one level below the item before it.
/// - A table's first row always heads it, so a table with no header gets an empty first row, which reads back
///   as none. A header row with no text in it therefore reads back as no header. Short rows are filled with
///   empty cells, and a justified column reads back as aligned to the start.
/// - A figure is its picture alone in a paragraph, then its caption as an emphasised paragraph. A gallery is
///   its pictures in one paragraph, a line each, so a gallery of one reads back as a figure. A numbered
///   caption opens with its number as "Figure 2:", and a reference reads as "Figure 2". A picture is named by
///   its address, or by its entry in the document's file, which [`markdown_assets`] lists. Its size is left
///   out, and so are italics within a caption and the caption of a picture inside a gallery.
/// - A table of contents is the list it shows, each line a link to its heading at the anchor GitHub gives
///   one, which most renderers follow. It reads back as that list. `[TOC]` reads as a table.
/// - A link preview is its title as a link to the page, then on the next line what the page says of itself.
///   Its picture is left out. It reads back as that paragraph.
/// - A link to a page the document keeps a preview of carries the page's title as its own, which a reader
///   sees when the pointer rests on the link. The preview's description and picture are left out.
/// - An unknown block is written as its fallback text. Unknown marks and fields are left out, and so are the
///   languages of blocks and runs and the page breaks before blocks, which Markdown has no way to say.
pub fn to_markdown(doc: &Document) -> String {
    let mut doc = figures::labelled(doc);
    list_contents(&mut doc);
    title_links(&mut doc);
    let doc = &doc;
    let mut lines = doc.properties.as_ref().map(properties).unwrap_or_default();
    let body = blocks(&doc.blocks);
    if !lines.is_empty() && !body.is_empty() {
        lines.push(String::new());
    }
    lines.extend(body);
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// The entries of the document's file that its Markdown shows. [`to_markdown`] names each by its entry name,
/// as a path from the Markdown file's folder, so an export writes them there for the pictures to show.
pub fn markdown_assets(doc: &Document) -> BTreeSet<String> {
    let asset = |block: &Block| match &block.kind {
        BlockKind::Figure { asset, .. } => asset.clone(),
        _ => None,
    };
    doc.walk().filter_map(asset).collect()
}

/// Makes each table of contents the list it shows: an item to a heading, nested by the heading's level and
/// linked to the heading's anchor. Markdown has no word for the table that every renderer reads.
fn list_contents(doc: &mut Document) {
    fn swap(blocks: &mut Vec<Block>, lists: &mut BTreeMap<BlockId, Vec<Block>>) {
        let mut at = 0;
        while at < blocks.len() {
            let table = matches!(blocks[at].kind, BlockKind::TableOfContents { .. });
            match lists.remove(&blocks[at].id).filter(|_| table) {
                Some(list) => {
                    let lines = list.len();
                    blocks.splice(at..=at, list);
                    at += lines;
                }
                None => {
                    if let Body::Blocks(children) = &mut blocks[at].body {
                        swap(children, lists);
                    }
                    at += 1;
                }
            }
        }
    }
    let is_table = |block: &&Block| matches!(block.kind, BlockKind::TableOfContents { .. });
    let tables: Vec<BlockId> = doc.walk().filter(is_table).map(|table| table.id.clone()).collect();
    if tables.is_empty() {
        return;
    }
    let anchors = anchors(doc);
    let level = |heading: &Block| match heading.kind {
        BlockKind::Heading { level } => level,
        _ => 0,
    };
    let line = |heading: &Block, top: u8| {
        let link = anchors.get(&heading.id).map(|anchor| format!("#{anchor}"));
        let runs = heading.body.runs().iter().map(|run| Run { link: link.clone(), ..run.clone() });
        // A line sits as deep as its heading is below the highest one listed.
        let kind = BlockKind::ListItem { list: ListKind::Bullet, depth: level(heading) - top };
        Block { body: Body::Text(runs.collect()), ..Block::new(format!("{}-line", heading.id).as_str(), kind) }
    };
    let list = |table: BlockId| {
        let headings = doc.contents(&table);
        let top = headings.iter().map(|heading| level(heading)).min().unwrap_or(0);
        (table, headings.into_iter().map(|heading| line(heading, top)).collect())
    };
    let mut lists: BTreeMap<BlockId, Vec<Block>> = tables.into_iter().map(list).collect();
    swap(&mut doc.blocks, &mut lists);
}

/// Says after each link's address the title the document keeps of the page there, set apart by a line break,
/// which no address is written with. A link is then written with that title: see [`titled`].
fn title_links(doc: &mut Document) {
    fn title(blocks: &mut [Block], previews: &BTreeMap<String, Preview>) {
        for block in blocks {
            match &mut block.body {
                Body::Text(runs) => {
                    for link in runs.iter_mut().filter_map(|run| run.link.as_mut()) {
                        let kept = previews.get(link.as_str()).and_then(|preview| preview.title.as_deref());
                        *link = link.replace(['\n', '\r'], " ");
                        if let Some(kept) = kept.filter(|kept| !kept.is_empty() && !link.is_empty()) {
                            link.push('\n');
                            link.push_str(&kept.replace(['\n', '\r'], " "));
                        }
                    }
                }
                Body::Blocks(children) => title(children, previews),
                Body::None => {}
            }
        }
    }
    let previews = std::mem::take(&mut doc.previews);
    title(&mut doc.blocks, &previews);
}

/// A link's address, and the title [`title_links`] put after it.
fn titled(link: &str) -> (&str, Option<&str>) {
    link.split_once('\n').map_or((link, None), |(address, title)| (address, Some(title)))
}

/// The anchor of each heading as GitHub makes it: the heading's text in lower case, with a hyphen for each
/// space and without punctuation, and numbered from the second heading of the same text on.
fn anchors(doc: &Document) -> BTreeMap<BlockId, String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let headings = doc.walk().filter(|block| matches!(block.kind, BlockKind::Heading { .. }));
    let anchor = |heading: &Block| {
        let kept = |c: &char| c.is_alphanumeric() || c.is_whitespace() || matches!(c, '-' | '_');
        let text: String = heading.plain_text().trim().to_lowercase().chars().filter(kept).collect();
        let text: String = text.chars().map(|c| if c.is_whitespace() { '-' } else { c }).collect();
        let count = seen.entry(text.clone()).or_default();
        *count += 1;
        (heading.id.clone(), if *count > 1 { format!("{text}-{}", *count - 1) } else { text })
    };
    headings.map(anchor).collect()
}

/// Writes a document's properties as front matter, where they read back as it. Those that would not, such as
/// ones that hold a line like their own fence, are written as a code block, so that nothing is lost.
fn properties(properties: &Properties) -> Vec<String> {
    let (fence, format) =
        FENCES.iter().find(|(_, format)| *format == properties.format).expect("each format has a fence");
    let source = properties.source.strip_suffix('\n').unwrap_or(&properties.source);
    let mut written = vec![(*fence).to_owned()];
    written.extend(lines(source).into_iter().filter(|_| !properties.source.is_empty()));
    written.push((*fence).to_owned());
    let front = format!("{}\n", written.join("\n"));
    match front_matter(&front) {
        (Some(read), "") if read.source.strip_suffix('\n').unwrap_or(&read.source) == source => written,
        _ => code(if *format == PropertiesFormat::Yaml { "yaml" } else { "toml" }, source),
    }
}

/// One nesting level of the list being written.
struct Level {
    /// The column of this level's markers.
    column: usize,
    /// The column where the text of its latest item starts, which is where a nested list sits.
    content: usize,
    /// The number of its latest item, if that item was numbered.
    number: u64,
}

/// Writes sibling blocks as lines, with a blank line between blocks and none between the items of a list.
fn blocks<'a>(blocks: impl IntoIterator<Item = &'a Block>) -> Vec<String> {
    let mut out = Vec::new();
    let mut levels: Vec<Level> = Vec::new();
    // The item before this one, while a list is being written.
    let mut last: Option<&Block> = None;
    // Whether the block before this one ends in a picture with no caption, which would take an emphasised
    // paragraph under it for one.
    let mut bare = false;
    for block in blocks.into_iter().filter(|block| is_written(block)) {
        if let BlockKind::ListItem { list, depth } = block.kind {
            let after_text = last.is_some_and(|last| !last.plain_text().is_empty());
            let (lines, gap) = item(&mut levels, list, depth, block.body.runs(), after_text);
            if gap || (last.is_none() && !out.is_empty()) {
                out.push(String::new());
            }
            out.extend(lines);
            last = Some(block);
        } else {
            if !out.is_empty() {
                out.push(String::new());
            }
            match block.kind {
                BlockKind::Paragraph if bare && is_emphasised(block) => {
                    out.extend(lines(&written(block.body.runs(), Context::Lines, true)));
                }
                _ => out.extend(other(block)),
            }
            levels.clear();
            last = None;
        }
        bare = match block.kind {
            BlockKind::Figure { .. } => !is_shown(block),
            BlockKind::Gallery { .. } => block.body.blocks().iter().all(|part| is_picture(part) || !is_written(part)),
            _ => false,
        };
    }
    out
}

fn is_picture(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Figure { .. })
}

/// Whether a block has text that shows.
fn is_shown(block: &Block) -> bool {
    !block.plain_text().trim().is_empty()
}

/// Whether all of a block's text that shows is in italics, as a caption is written.
fn is_emphasised(block: &Block) -> bool {
    let shown = |run: &&Run| !run.text.trim().is_empty();
    is_shown(block) && block.body.runs().iter().filter(shown).all(|run| run.marks.contains(&Mark::Italic))
}

/// Whether a block has a Markdown form. An empty paragraph has none, nor has an unknown block with no fallback.
/// A table has one as soon as it has a row, since its cells are there to be filled.
fn is_written(block: &Block) -> bool {
    match block.kind {
        BlockKind::Paragraph | BlockKind::TableCell => !block.plain_text().is_empty(),
        BlockKind::Caption => is_shown(block),
        BlockKind::Gallery { .. } => block.body.blocks().iter().any(is_written),
        BlockKind::Table { .. } => {
            block.body.blocks().iter().any(|row| row.kind == BlockKind::TableRow || is_written(row))
        }
        BlockKind::TableRow => true,
        BlockKind::Unknown { .. } => !fallback(block).is_empty(),
        // One met here has no document to list the headings of.
        BlockKind::TableOfContents { .. } => false,
        _ => true,
    }
}

fn fallback(block: &Block) -> &str {
    block.extra.get("fallback").and_then(|fallback| fallback.as_str()).unwrap_or_default()
}

/// Writes a list item, and says whether a blank line must come before it.
fn item(levels: &mut Vec<Level>, list: ListKind, depth: u8, runs: &[Run], after_text: bool) -> (Vec<String>, bool) {
    // Markdown nests an item under the one before it, so it can sit at most one level deeper.
    let depth = usize::from(depth).min(levels.len());
    levels.truncate(depth + 1);
    let nested = levels.len() == depth;
    if nested {
        let column = levels.last().map_or(0, |parent| parent.content);
        levels.push(Level { column, content: column, number: 0 });
    }
    let level = &mut levels[depth];
    let ordered = list == ListKind::Ordered;
    // A change between bullets and numbers starts a new list beside the last one.
    let changed = !nested && ordered != (level.number > 0);
    level.number = if ordered { level.number + 1 } else { 0 };
    let (marker, checkbox) = match list {
        ListKind::Bullet => ("- ".to_owned(), ""),
        ListKind::Ordered => (format!("{}. ", level.number), ""),
        ListKind::Task { checked: false } => ("- ".to_owned(), "[ ] "),
        ListKind::Task { checked: true } => ("- ".to_owned(), "[x] "),
    };
    level.content = level.column + marker.len();

    let text = text(runs, Context::Lines);
    let (column, content) = (level.column, level.content);
    let mut lines = text.split('\n');
    let first = format!("{:column$}{marker}{checkbox}{}", "", lines.next().unwrap_or_default());
    let mut out = vec![first.trim_end_matches(' ').to_owned()];
    out.extend(lines.map(|line| format!("{:content$}{line}", "")));
    // A new list reads best set apart. An empty item needs it: straight after text, it cannot start a list.
    let empty_start = nested && after_text && text.is_empty() && checkbox.is_empty();
    (out, changed || empty_start)
}

/// Writes any block that is not a list item.
fn other(block: &Block) -> Vec<String> {
    let runs = block.body.runs();
    match &block.kind {
        BlockKind::Paragraph => lines(&text(runs, Context::Lines)),
        BlockKind::Heading { level } => {
            let marker = "#".repeat(usize::from(*level).clamp(1, 6));
            vec![format!("{marker} {}", text(runs, Context::Heading)).trim_end_matches(' ').to_owned()]
        }
        BlockKind::Code { language } => code(language.as_deref().unwrap_or_default(), &block.plain_text()),
        BlockKind::Formula => formula(&block.plain_text()),
        BlockKind::Quote => quote(None, block.body.blocks()),
        BlockKind::Callout { tone, title, .. } => {
            // Markdown has a place for a title, after the tag, and none for an icon or a colour.
            let title = title.as_deref().filter(|title| !title.is_empty());
            quote(Some((*tone, title)), block.body.blocks())
        }
        BlockKind::Divider => vec!["---".to_owned()],
        BlockKind::Table { columns, header } => table(columns, *header, block.body.blocks()),
        // A merge can leave a row outside a table and a cell outside a row.
        BlockKind::TableRow => table(&[], false, std::slice::from_ref(block)),
        BlockKind::TableCell => lines(&text(runs, Context::Lines)),
        BlockKind::Caption => emphasised(runs),
        BlockKind::Figure { .. } => {
            let mut out = vec![picture(block)];
            if is_shown(block) {
                out.push(String::new());
                out.extend(emphasised(runs));
            }
            out
        }
        // Its pictures share a paragraph, and what else it holds follows them.
        BlockKind::Gallery { .. } => {
            let parts = block.body.blocks();
            let mut out: Vec<String> = parts.iter().filter(|part| is_picture(part)).map(picture).collect();
            let rest = blocks(parts.iter().filter(|part| !is_picture(part)));
            if !out.is_empty() && !rest.is_empty() {
                out.push(String::new());
            }
            out.extend(rest);
            out
        }
        BlockKind::Unknown { .. } => lines(&text(&[Run::plain(fallback(block))], Context::Lines)),
        // What its card reads as: the title, which leads to the page, over what the page says of itself.
        BlockKind::LinkPreview { url, title, description, .. } => {
            let mut runs = vec![Run { link: Some(url.clone()), ..Run::plain(title.as_deref().unwrap_or(url)) }];
            runs.extend(description.iter().map(|description| Run::plain(format!("\n{description}"))));
            lines(&text(&runs, Context::Lines))
        }
        // A list item is written by `item`, which knows the list around it, and a table of contents as
        // the items `list_contents` made of it.
        BlockKind::ListItem { .. } | BlockKind::TableOfContents { .. } => Vec::new(),
    }
}

/// Writes a figure's picture, described by its alt text.
fn picture(figure: &Block) -> String {
    let BlockKind::Figure { asset, src, alt, .. } = &figure.kind else { return String::new() };
    let alt = [Run::plain(alt.replace(['\n', '\r'], " "))];
    // A gallery's pictures share a paragraph, where a dollar in each of two could pair up as maths.
    let alt = Inline { maths: true, ..Inline::new(&alt) }.write(Context::Heading, None);
    format!("![{alt}]({})", destination(asset.as_deref().or(src.as_deref()).unwrap_or_default()))
}

/// Writes a caption: its text, emphasised from end to end.
fn emphasised(runs: &[Run]) -> Vec<String> {
    let emphasise = |run: &Run| Run { marks: run.marks.iter().cloned().chain([Mark::Italic]).collect(), ..run.clone() };
    lines(&text(&runs.iter().map(emphasise).collect::<Vec<_>>(), Context::Lines))
}

fn lines(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_owned).collect()
}

fn code(language: &str, text: &str) -> Vec<String> {
    // The reader unescapes the info string, and a backtick fence cannot hold a backtick in it.
    let info = language.replace('\\', "\\\\").replace('&', "\\&");
    let tick = if info.contains('`') { '~' } else { '`' };
    let fence = tick.to_string().repeat(longest_run(text, tick).max(2) + 1);
    // A name that starts with the fence's own character would lengthen the fence.
    let gap = if info.starts_with(tick) { " " } else { "" };
    let mut out = vec![format!("{fence}{gap}{info}")];
    if !text.is_empty() {
        out.extend(lines(text));
    }
    out.push(fence);
    out
}

/// Writes display maths between dollar signs where it reads back from them. They sit in a paragraph, which a
/// blank line or a line that starts another block would end. A tab is as wide as where it sits, so inside a
/// quote it could start a block that it does not start here.
fn formula(tex: &str) -> Vec<String> {
    let dollars = if tex.is_empty() { "$$\n$$".to_owned() } else { format!("$$\n{tex}\n$$") };
    let (doc, _) = from_markdown(&dollars, &mut String::new);
    let reads_back =
        matches!(doc.blocks.as_slice(), [block] if block.kind == BlockKind::Formula && block.plain_text() == tex);
    if reads_back && !tex.contains('\t') { lines(&dollars) } else { code("math", tex) }
}

fn quote(tone: Option<(Tone, Option<&str>)>, children: &[Block]) -> Vec<String> {
    let inner = blocks(children);
    let mut out = Vec::new();
    if let Some((tone, title)) = tone {
        let (name, _) = TONES.iter().find(|(_, named)| *named == tone).expect("every tone is named");
        match title {
            Some(title) => out.push(format!("[!{name}] {}", text(&[Run::plain(title)], Context::Heading))),
            None => out.push(format!("[!{name}]")),
        }
        // A paragraph runs on from the tag, as GitHub expects. Any other block needs a gap to be read as itself.
        let first = children.iter().find(|child| is_written(child));
        if first.is_some_and(|first| first.kind != BlockKind::Paragraph) {
            out.push(String::new());
        }
    }
    out.extend(inner);
    if out.is_empty() {
        out.push(String::new());
    }
    out.into_iter().map(|line| if line.is_empty() { ">".to_owned() } else { format!("> {line}") }).collect()
}

/// Writes a table as GFM has them: a line to a row, with a rule under the first that aligns the columns. The
/// first row heads the table there, so a table with no header is given a row of empty cells for one. A row
/// that is short of cells is filled up. What a table holds that is not a row is written after it.
fn table(columns: &[Align], header: bool, children: &[Block]) -> Vec<String> {
    let is_row = |child: &&Block| child.kind == BlockKind::TableRow;
    let cell = |cell: &Block| match cell.kind {
        BlockKind::Unknown { .. } => text(&[Run::plain(fallback(cell))], Context::Cell),
        _ => text(cell.body.runs(), Context::Cell),
    };
    let mut rows: Vec<Vec<String>> =
        children.iter().filter(is_row).map(|row| row.body.blocks().iter().map(cell).collect()).collect();
    let mut out = Vec::new();
    if !rows.is_empty() {
        let width = rows.iter().map(Vec::len).chain([columns.len()]).max().unwrap_or(0).max(1);
        let rule = |column: usize| match columns.get(column) {
            Some(Align::Center) => ":-:",
            Some(Align::End) => "--:",
            _ => "---",
        };
        if !header {
            rows.insert(0, Vec::new());
        }
        rows.insert(1, (0..width).map(|column| rule(column).to_owned()).collect());
        for mut row in rows {
            row.resize(width, String::new());
            out.push(format!("| {} |", row.join(" | ")));
        }
    }
    let rest = blocks(children.iter().filter(|child| !is_row(child)));
    if !out.is_empty() && !rest.is_empty() {
        out.push(String::new());
    }
    out.extend(rest);
    out
}

fn longest_run(text: &str, of: char) -> usize {
    text.split(|c| c != of).map(str::len).max().unwrap_or(0)
}

/// Where a block's text is written, which decides how line breaks and block markers are handled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    /// A paragraph or list item: a break ends the line, and a line must not read as another block.
    Lines,
    /// One line that holds only text.
    Heading,
    /// One line, in which a pipe ends the cell.
    Cell,
}

/// Writes a block's text. Emphasis is written with Markdown's delimiters where they read back as written, and
/// as HTML tags where Markdown's rules about the characters beside a delimiter would lose it. Maths is written
/// between dollar signs where it reads back from them, and as code where it does not.
fn text(runs: &[Run], context: Context) -> String {
    written(runs, context, false)
}

/// Writes a block's text. With `set`, italics are written with the tag for text that is only set that way,
/// which tells a paragraph that is all in italics apart from the caption of the picture above it.
fn written(runs: &[Run], context: Context, set: bool) -> String {
    let mut inline = Inline::new(runs);
    loop {
        if !set {
            let written = inline.write(context, None);
            if !inline.has(EMPHASIS | MATH) || inline.reads_back(&written, context) {
                return written;
            }
        }
        let tagged = inline.write(context, Some(if set { "i" } else { "em" }));
        if !inline.has(MATH) || inline.reads_back(&tagged, context) {
            return tagged;
        }
        inline
            .formats
            .iter_mut()
            .filter(|format| format.marks & MATH != 0)
            .for_each(|format| format.marks ^= MATH | CODE);
    }
}

const BOLD: u8 = 1;
const ITALIC: u8 = 2;
const STRIKE: u8 = 4;
const UNDERLINE: u8 = 8;
const HIGHLIGHT: u8 = 16;
const CODE: u8 = 32;
const MATH: u8 = 64;
/// The marks written with delimiters, which Markdown only accepts next to the right characters.
const EMPHASIS: u8 = BOLD | ITALIC | STRIKE;

const MARKS: [(u8, Mark); 7] = [
    (MATH, Mark::Math),
    (BOLD, Mark::Bold),
    (ITALIC, Mark::Italic),
    (STRIKE, Mark::Strike),
    (UNDERLINE, Mark::Underline),
    (HIGHLIGHT, Mark::Highlight),
    (CODE, Mark::Code),
];

#[derive(Clone, Copy, PartialEq, Eq)]
struct Format<'a> {
    marks: u8,
    link: Option<&'a str>,
}

/// Syntax that wraps text and nests.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Span<'a> {
    Link(&'a str),
    Mark(u8),
}

impl Format<'_> {
    fn has(&self, span: Span) -> bool {
        match span {
            Span::Link(url) => self.link == Some(url),
            Span::Mark(bit) => self.marks & bit != 0,
        }
    }
}

/// A block's text with one format per character, which makes spans easy to trim and regroup.
struct Inline<'a> {
    chars: Vec<char>,
    formats: Vec<Format<'a>>,
    /// Whether a dollar in the text could be read as the edge of maths. One dollar alone is a price.
    maths: bool,
}

impl<'a> Inline<'a> {
    fn new(runs: &'a [Run]) -> Self {
        let mut chars = Vec::new();
        let mut formats = Vec::new();
        for run in runs {
            let bit = |mark| MARKS.iter().find(|(_, known)| known == mark).map_or(0, |(bit, _)| *bit);
            let marks = run.marks.iter().fold(0, |marks, mark| marks | bit(mark));
            // Maths is written as its source, which is not also code.
            let marks = if marks & MATH != 0 { marks & !CODE } else { marks };
            for c in run.text.chars() {
                // Neither a code span nor maths can hold a line break.
                let marks = if matches!(c, '\n' | '\r') { marks & !(CODE | MATH) } else { marks };
                chars.push(c);
                formats.push(Format { marks, link: run.link.as_deref() });
            }
        }

        // Markdown cannot open emphasis or maths before a space or close it after one, so those spaces leave the
        // span. Maths goes first: the spaces that leave it are then spaces of the emphasis around it.
        for bit in [MATH, BOLD, ITALIC, STRIKE] {
            let space = |i: usize, formats: &[Format]| match bit {
                MATH => chars[i].is_ascii_whitespace(),
                _ => chars[i].is_whitespace() && formats[i].marks & (CODE | MATH) == 0,
            };
            let mut start = 0;
            while start < chars.len() {
                let end = start + formats[start..].iter().take_while(|format| format.marks & bit != 0).count();
                let leading = (start..end).take_while(|i| space(*i, &formats)).count();
                let trailing = (start + leading..end).rev().take_while(|i| space(*i, &formats)).count();
                for i in (start..start + leading).chain(end - trailing..end) {
                    formats[i].marks &= !bit;
                }
                start = end.max(start + 1);
            }
        }
        let maths =
            chars.iter().filter(|c| **c == '$').count() > 1 || formats.iter().any(|format| format.marks & MATH != 0);
        Self { chars, formats, maths }
    }

    fn has(&self, marks: u8) -> bool {
        self.formats.iter().any(|format| format.marks & marks != 0)
    }

    /// The stretches of text that share one format.
    fn segments(&self) -> Vec<(Range<usize>, Format<'a>)> {
        let mut segments: Vec<(Range<usize>, Format)> = Vec::new();
        for (i, format) in self.formats.iter().enumerate() {
            match segments.last_mut() {
                Some((range, last)) if last == format => range.end = i + 1,
                _ => segments.push((i..i + 1, *format)),
            }
        }
        segments
    }

    /// Whether Markdown reads `written` back as this text with these marks.
    fn reads_back(&self, written: &str, context: Context) -> bool {
        let runs: Vec<Run> = self
            .segments()
            .into_iter()
            .map(|(range, format)| Run {
                text: self.chars[range].iter().collect(),
                marks: MARKS.iter().filter(|(bit, _)| format.marks & bit != 0).map(|(_, mark)| mark.clone()).collect(),
                link: format.link.map(|link| titled(link).0.to_owned()),
                ..Run::default()
            })
            .collect();
        let source = match context {
            Context::Lines => written.to_owned(),
            Context::Heading => format!("# {written}"),
            Context::Cell => format!("| {written} |\n| --- |"),
        };
        let (doc, _) = from_markdown(&source, &mut String::new);
        let read = match (context, doc.blocks.as_slice()) {
            (Context::Cell, [table]) => table.body.blocks().first().and_then(|row| row.body.blocks().first()),
            (_, [block]) => Some(block),
            _ => None,
        };
        read.is_some_and(|block| block.kind.shape() == BodyShape::Text && block.body.runs() == runs)
    }

    /// Writes the text. With `tags`, emphasis is written as HTML, which reads back whatever sits beside it,
    /// and italics as the tag it names.
    fn write(&self, context: Context, tags: Option<&str>) -> String {
        let segments = self.segments();
        let mut out = String::new();
        let mut open: Vec<Span> = Vec::new();
        for (i, (range, format)) in segments.iter().enumerate() {
            let autolink = self.is_autolink(&segments, i);
            let mut spans: Vec<Span> = format.link.filter(|_| !autolink).map(Span::Link).into_iter().collect();
            spans.extend(
                MARKS
                    .iter()
                    .filter(|(bit, _)| bit & (CODE | MATH) == 0 && format.marks & bit != 0)
                    .map(|m| Span::Mark(m.0)),
            );

            // Close what ends here, along with everything opened inside it.
            let keep = open.iter().position(|span| !spans.contains(span)).unwrap_or(open.len());
            for span in open.drain(keep..).rev() {
                out.push_str(&close(span, tags));
            }
            // Open the longest span first, so that the spans inside it close before it does.
            spans.retain(|span| !open.contains(span));
            spans.sort_by_key(|span| Reverse(segments[i..].iter().take_while(|(_, later)| later.has(*span)).count()));
            for span in spans {
                out.push_str(&start(span, tags));
                open.push(span);
            }

            let text: String = self.chars[range.clone()].iter().collect();
            if autolink {
                out.push_str(&format!("<{text}>"));
            } else if format.marks & CODE != 0 && format.link.is_some() && text.contains(']') {
                // Markdown looks for a link definition before it looks for code, and would end the link here.
                out.push_str("<code>");
                self.escape(range.clone(), context, &mut out);
                out.push_str("</code>");
            } else if format.marks & CODE != 0 {
                let text = if context == Context::Cell { text.replace('|', "\\|") } else { text };
                // A span is fenced by more backticks than it holds, and padded where a fence would touch one.
                let fence = "`".repeat(longest_run(&text, '`') + 1);
                let spaced = text.starts_with(' ') && text.ends_with(' ') && !text.trim_matches(' ').is_empty();
                let pad = if spaced || text.starts_with('`') || text.ends_with('`') { " " } else { "" };
                out.push_str(&format!("{fence}{pad}{text}{pad}{fence}"));
            } else if format.marks & MATH != 0 {
                let text = if context == Context::Cell { text.replace('|', "\\|") } else { text };
                out.push_str(&format!("${text}$"));
            } else {
                self.escape(range.clone(), context, &mut out);
            }
        }
        for span in open.drain(..).rev() {
            out.push_str(&close(span, tags));
        }
        out
    }

    /// Whether a segment is a link whose text is its own address, which Markdown writes as `<address>`.
    fn is_autolink(&self, segments: &[(Range<usize>, Format)], i: usize) -> bool {
        let (range, format) = &segments[i];
        let Some(url) = format.link else { return false };
        let Some((scheme, rest)) = url.split_once(':') else { return false };
        let alone = (i == 0 || segments[i - 1].1.link != format.link)
            && segments.get(i + 1).is_none_or(|(_, next)| next.link != format.link);
        alone
            && format.marks & (CODE | MATH) == 0
            && !url.contains('$')
            && self.chars[range.clone()].iter().copied().eq(url.chars())
            && (2..=32).contains(&scheme.len())
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
            && !rest.contains(|c: char| c.is_ascii_control() || " <>".contains(c))
    }

    /// Writes plain text so that Markdown reads it as text.
    fn escape(&self, range: Range<usize>, context: Context, out: &mut String) {
        // The character whose escape stops the current line from reading as another block.
        let mut marker = None;
        for i in range.clone() {
            let c = self.chars[i];
            // The neighbours on this line of text, and those of them in this segment.
            let before = i.checked_sub(1).map(|i| self.chars[i]).filter(|c| *c != '\n');
            let after = self.chars.get(i + 1).copied().filter(|c| *c != '\n');
            let previous = before.filter(|_| i > range.start);
            let next = after.filter(|_| i + 1 < range.end);

            if c == '\n' {
                out.push_str(match context {
                    Context::Lines if i + 1 < self.chars.len() => "\\\n",
                    // A break that ends the text has no line to end. Alone, the tag would read as an HTML block.
                    Context::Lines if out.is_empty() => "&#10;",
                    _ => "<br>",
                });
                continue;
            }
            if context == Context::Lines && before.is_none() && (out.is_empty() || out.ends_with('\n')) {
                let line = self.chars[i..].split(|c| *c == '\n').next().unwrap_or_default();
                marker = block_marker(line).map(|at| i + at);
            }

            let escaped = match c {
                // A digit straight after maths would make its dollar signs read as prices.
                '0'..='9' if i == range.start && i > 0 && self.formats[i - 1].marks & MATH != 0 => {
                    out.push_str(&format!("&#{};", u32::from(c)));
                    continue;
                }
                // Markdown trims a line, so the spaces at its ends are written as references.
                ' ' | '\t' | '\r' if c == '\r' || before.is_none() || after.is_none() => {
                    out.push_str(&format!("&#{};", u32::from(c)));
                    continue;
                }
                '\\' | '`' | '*' | '[' | ']' | '<' | '~' => true,
                // An underscore inside a word is only ever text.
                '_' => !(previous.is_some_and(char::is_alphanumeric) && next.is_some_and(char::is_alphanumeric)),
                '&' => is_reference(&self.chars[i + 1..range.end]),
                '$' => self.maths,
                // Before a link, this would make it an image.
                '!' => {
                    self.formats.get(i + 1).is_some_and(|next| next.link.is_some() && next.link != self.formats[i].link)
                }
                '|' => context == Context::Cell,
                // Hashes that end a heading are read as decoration.
                '#' => context == Context::Heading && i + 1 == self.chars.len(),
                _ => false,
            };
            if escaped || marker == Some(i) {
                out.push('\\');
            }
            out.push(c);
        }
    }
}

fn start(span: Span, tags: Option<&str>) -> String {
    let delimiter = match span {
        Span::Link(_) => "[",
        Span::Mark(BOLD) if tags.is_none() => "**",
        Span::Mark(ITALIC) if tags.is_none() => "*",
        Span::Mark(STRIKE) if tags.is_none() => "~~",
        Span::Mark(bit) => return format!("<{}>", tag(bit, tags)),
    };
    delimiter.to_owned()
}

fn close(span: Span, tags: Option<&str>) -> String {
    match span {
        Span::Link(link) => match titled(link) {
            (address, Some(title)) => format!("]({} \"{}\")", destination(address), quoted(title)),
            (address, None) => format!("]({})", destination(address)),
        },
        Span::Mark(bit) if tags.is_some() || bit & EMPHASIS == 0 => format!("</{}>", tag(bit, tags)),
        Span::Mark(_) => start(span, tags),
    }
}

/// The HTML tag that stands for a mark. `tags` names the one for italics.
fn tag(bit: u8, tags: Option<&str>) -> &str {
    let named = |mark: &Mark| TAGS.iter().find(|(_, tagged)| tagged == mark).map(|(name, _)| *name);
    let known = MARKS.iter().find(|(known, _)| *known == bit).and_then(|(_, mark)| named(mark));
    tags.filter(|_| bit == ITALIC).or(known).expect("every mark has a tag")
}

/// Writes a link's address. One with a space in it is wrapped in angle brackets, and none can hold a line break.
fn destination(url: &str) -> String {
    let chars: Vec<char> = url.chars().map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c }).collect();
    let pointed = chars.iter().any(|c| c.is_whitespace() || c.is_control());
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let escaped = match c {
            // A dollar here could pair with one in the text and read as maths.
            '\\' | '<' | '>' | '$' => true,
            '(' | ')' => !pointed,
            '&' => is_reference(&chars[i + 1..]),
            _ => false,
        };
        if escaped {
            out.push('\\');
        }
        out.push(*c);
    }
    if pointed { format!("<{out}>") } else { out }
}

/// Writes a link's title as its quotes hold it. What would end it is escaped, and so is what Markdown looks
/// for before it looks for links, which could pair with the same in the text.
fn quoted(title: &str) -> String {
    let chars: Vec<char> = title.chars().collect();
    let mut out = String::new();
    for (i, c) in chars.iter().enumerate() {
        let escaped = match c {
            '"' | '\\' | '`' | '<' | '$' | '|' => true,
            '&' => is_reference(&chars[i + 1..]),
            _ => false,
        };
        if escaped {
            out.push('\\');
        }
        out.push(*c);
    }
    out
}

/// Whether the text after an ampersand makes it a character reference such as `&amp;`.
fn is_reference(rest: &[char]) -> bool {
    let name = rest.iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '#').count();
    name > 0 && rest.get(name) == Some(&';')
}

/// The character to escape so that a line of text is not read as the start of another block.
fn block_marker(line: &[char]) -> Option<usize> {
    let ends = |i: usize| line.get(i).is_none_or(|c| matches!(c, ' ' | '\t'));
    let all = |allowed: &str| line.iter().all(|c| allowed.contains(*c));
    let hashes = line.iter().take_while(|c| **c == '#').count();
    let digits = line.iter().take_while(|c| c.is_ascii_digit()).count();
    match line.first()? {
        '>' => Some(0),
        '#' if hashes <= 6 && ends(hashes) => Some(0),
        '-' | '+' if ends(1) => Some(0),
        // The underline of a heading, a divider or the rule under a table's header. A bullet's own dash can
        // complete that rule, so a line of pipes and colons counts too.
        '=' if all("= \t") => Some(0),
        '-' | '|' | ':' if all("-|: \t") => Some(0),
        '0'..='9' if digits <= 9 && matches!(line.get(digits), Some('.' | ')')) && ends(digits + 1) => Some(digits),
        _ => None,
    }
}
