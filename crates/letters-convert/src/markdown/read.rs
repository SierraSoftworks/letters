//! Markdown to document.

use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{Block, BlockKind, Body, Document, ListKind, Mark, Run, Tone};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use super::{TAGS, TONES, write};
use crate::ImportNote;

/// Quotes and lists nested deeper than this are kept as source, which bounds the reader's recursion.
const MAX_DEPTH: usize = 32;

const MATHS: &str = "Maths kept as TeX source";
const FOOTNOTE: &str = "Footnote kept as text";

/// Reads CommonMark and GFM. `new_id` supplies an id for each block.
///
/// Nothing is dropped: what the document cannot hold is kept as text or as a code block and named in a note.
pub fn from_markdown(markdown: &str, new_id: &mut dyn FnMut() -> String) -> (Document, Vec<ImportNote>) {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
        | Options::ENABLE_WIKILINKS;
    let (front, source) = front_matter(markdown);
    let events = Parser::new_ext(source, options).into_offset_iter().collect();
    let mut reader = Reader { source, events, at: 0, depth: 0, new_id, notes: Vec::new(), task: None };
    let mut blocks = Vec::new();
    if let Some((language, text)) = front {
        reader.placeholder(language, &unix_lines(text), "Front matter kept as a code block", &mut blocks);
    }
    blocks.extend(reader.blocks());
    (Document { blocks, ..Document::default() }, reader.notes)
}

/// Splits front matter off the start of a file: the lines between a first line of `---` or `+++` and the next
/// such line. Returns its language and text, then the rest of the file.
fn front_matter(markdown: &str) -> (Option<(&str, &str)>, &str) {
    for (fence, language) in [("---", "yaml"), ("+++", "toml")] {
        let Some(rest) = markdown.strip_prefix(fence) else { continue };
        let Some(body) = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')) else { continue };
        // A blank line straight after the fence makes it a divider.
        if body.trim_start_matches([' ', '\t']).starts_with(['\n', '\r']) {
            continue;
        }
        let mut end = 0;
        for line in body.split_inclusive('\n') {
            if line.trim_end() == fence {
                return (Some((language, body[..end].trim())), &body[end + line.len()..]);
            }
            end += line.len();
        }
    }
    (None, markdown)
}

struct Reader<'a, 'f> {
    source: &'a str,
    events: Vec<(Event<'a>, Range<usize>)>,
    /// The next event to read.
    at: usize,
    /// How many quotes and lists enclose the block being read.
    depth: usize,
    new_id: &'f mut dyn FnMut() -> String,
    notes: Vec<ImportNote>,
    /// The checkbox of the list item being read, once its marker has been seen.
    task: Option<bool>,
}

impl<'a> Reader<'a, '_> {
    fn next(&mut self) -> Option<(Event<'a>, Range<usize>)> {
        let event = self.events.get(self.at).cloned()?;
        self.at += 1;
        Some(event)
    }

    fn peek(&self, ahead: usize) -> Option<&Event<'a>> {
        self.events.get(self.at + ahead).map(|(event, _)| event)
    }

    fn block(&mut self, kind: BlockKind) -> Block {
        Block::new((self.new_id)().as_str(), kind)
    }

    fn code(&mut self, language: Option<&str>, text: &str) -> Block {
        let mut block = self.block(BlockKind::Code { language: language.map(str::to_owned) });
        if !text.is_empty() {
            block.body = Body::Text(vec![Run::plain(text)]);
        }
        block
    }

    /// Keeps source the document cannot hold as a code block, with a note that says so.
    fn placeholder(&mut self, language: &str, text: &str, message: &str, out: &mut Vec<Block>) {
        let block = self.code(Some(language), text);
        self.note(&block, message);
        out.push(block);
    }

    /// Adds a note to a block, once however often its cause repeats.
    fn note(&mut self, block: &Block, message: impl Into<String>) {
        let note = ImportNote { block: block.id.to_string(), message: message.into() };
        if !self.notes.iter().rev().take_while(|other| other.block == note.block).any(|other| *other == note) {
            self.notes.push(note);
        }
    }

    /// Reads blocks up to the end of the container they sit in.
    fn blocks(&mut self) -> Vec<Block> {
        let mut out = Vec::new();
        while let Some((event, range)) = self.next() {
            match event {
                Event::Start(tag) => self.read(tag, range, 0, &mut out),
                Event::Rule => out.push(self.block(BlockKind::Divider)),
                Event::End(_) => break,
                _ => {}
            }
        }
        nest(&mut out);
        out
    }

    /// Reads the block that `tag` opens. A list read here starts at `list_depth`.
    fn read(&mut self, tag: Tag<'a>, range: Range<usize>, list_depth: u8, out: &mut Vec<Block>) {
        match tag {
            Tag::Paragraph => {
                if let (Some(Event::DisplayMath(tex)), Some(Event::End(_))) = (self.peek(0), self.peek(1)) {
                    let tex = unix_lines(tex.trim());
                    self.at += 2;
                    return self.placeholder("math", &tex, MATHS, out);
                }
                let mut block = self.block(BlockKind::Paragraph);
                let runs = self.inline(&block);
                self.at += 1;
                if !runs.is_empty() {
                    block.body = Body::Text(runs);
                    out.push(block);
                }
            }
            Tag::Heading { level, .. } => {
                let level = level as u8;
                let mut block = self.block(BlockKind::Heading { level: level.min(4) });
                if level > 4 {
                    self.note(&block, format!("Heading level {level} became level 4"));
                }
                block.body = Body::Text(self.inline(&block));
                self.at += 1;
                out.push(block);
            }
            Tag::BlockQuote(_) | Tag::List(_) | Tag::FootnoteDefinition(_) if self.depth == MAX_DEPTH => {
                let mut open = 1;
                while open > 0
                    && let Some((event, _)) = self.next()
                {
                    match event {
                        Event::Start(_) => open += 1,
                        Event::End(_) => open -= 1,
                        _ => {}
                    }
                }
                let source = self.source;
                self.placeholder("markdown", source[range].trim_end(), "Deep nesting kept as Markdown", out);
            }
            Tag::BlockQuote(_) => {
                let kind = self.callout().map_or(BlockKind::Quote, |tone| BlockKind::Callout { tone });
                let mut block = self.block(kind);
                self.depth += 1;
                block.body = Body::Blocks(self.blocks());
                self.depth -= 1;
                out.push(block);
            }
            Tag::List(start) => {
                self.depth += 1;
                self.list(start, list_depth, out);
                self.depth -= 1;
            }
            Tag::CodeBlock(kind) => {
                let info = match &kind {
                    CodeBlockKind::Fenced(info) => info.as_ref(),
                    CodeBlockKind::Indented => "",
                };
                let (language, options) = info.trim().split_once(char::is_whitespace).unwrap_or((info.trim(), ""));
                let text = self.raw();
                let block =
                    self.code(Some(language).filter(|name| !name.is_empty()), text.strip_suffix('\n').unwrap_or(&text));
                if !options.is_empty() {
                    self.note(&block, format!("Code block options dropped: {}", options.trim()));
                }
                out.push(block);
            }
            Tag::HtmlBlock => {
                let html = self.raw();
                self.placeholder("html", html.trim_end_matches('\n'), "HTML kept as a code block", out);
            }
            Tag::Table(alignments) => {
                let mut block = self.block(BlockKind::Code { language: Some("markdown".into()) });
                block.body = Body::Text(vec![Run::plain(self.table(&block, &alignments))]);
                self.note(&block, "Table kept as Markdown in a code block");
                out.push(block);
            }
            Tag::FootnoteDefinition(label) => {
                self.depth += 1;
                let mut inner = self.blocks();
                self.depth -= 1;
                let marker = format!("[^{label}]:");
                match inner.first_mut() {
                    Some(Block { kind: BlockKind::Paragraph, body: Body::Text(runs), .. }) => {
                        runs.insert(0, Run::plain(marker + " "));
                        *runs = merge(std::mem::take(runs));
                    }
                    _ => {
                        let mut first = self.block(BlockKind::Paragraph);
                        first.body = Body::Text(vec![Run::plain(marker)]);
                        inner.insert(0, first);
                    }
                }
                self.note(&inner[0], FOOTNOTE);
                out.extend(inner);
            }
            // A container this reader does not know: its content is read in place.
            _ => out.extend(self.blocks()),
        }
    }

    /// The text of a block that holds nothing else, up to its end.
    fn raw(&mut self) -> String {
        let mut text = String::new();
        while let Some((event, _)) = self.next() {
            match event {
                Event::Text(part) | Event::Html(part) => text.push_str(&part),
                Event::End(_) => break,
                _ => {}
            }
        }
        text
    }

    /// Reads a callout tag such as `[!NOTE]` off the start of a quote and takes it out of the quote's text.
    fn callout(&mut self) -> Option<Tone> {
        let Some((Event::Start(Tag::Paragraph), range)) = self.events.get(self.at) else { return None };
        // The source is checked, not the events, so that an escaped `\[!NOTE\]` stays text.
        let (name, rest) = self.source[range.clone()].strip_prefix("[!")?.split_once(']')?;
        let (_, tone) = TONES.iter().find(|(tag, _)| tag.eq_ignore_ascii_case(name))?;
        if !rest.is_empty() && !rest.starts_with([' ', '\t', '\n', '\r']) {
            return None;
        }

        let end = range.start + name.len() + 3;
        let first = self.at + 1;
        while matches!(self.events.get(first), Some((Event::Text(_), text)) if text.end <= end) {
            self.events.remove(first);
        }
        if let Some((Event::Text(title), _)) = self.events.get_mut(first) {
            *title = title.trim_start().to_owned().into();
            if title.is_empty() {
                self.events.remove(first);
            }
        }
        let is_line_end =
            |event: &Event| matches!(event, Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph));
        if let Some(line_end) = (first..self.events.len()).find(|i| is_line_end(&self.events[*i].0)) {
            match &self.events[line_end].0 {
                // Text after the tag on its line is a title, which keeps its own line.
                Event::SoftBreak if line_end > first => self.events[line_end].0 = Event::HardBreak,
                Event::End(_) if line_end == first => drop(self.events.drain(first - 1..=first)),
                Event::SoftBreak | Event::HardBreak if line_end == first => drop(self.events.remove(first)),
                _ => {}
            }
        }
        Some(*tone)
    }

    /// Reads a list as flat items. `depth` is how far the list is nested in other lists.
    fn list(&mut self, start: Option<u64>, depth: u8, out: &mut Vec<Block>) {
        let mut renumbered = start.filter(|first| *first != 1);
        while let Some((Event::Start(Tag::Item), _)) = self.next() {
            let mut block = self.block(BlockKind::ListItem { list: ListKind::Bullet, depth });
            if let Some(first) = renumbered.take() {
                self.note(&block, format!("Numbering started at {first} and now starts at 1"));
            }

            self.task = None;
            let mut runs = self.inline(&block);
            let mut task = self.task.take();
            let mut after = Vec::new();
            while let Some((event, range)) = self.next() {
                match event {
                    Event::End(_) => break,
                    // The paragraphs an item opens with are its text. Anything else follows the item.
                    Event::Start(Tag::Paragraph) if after.is_empty() => {
                        let more = self.inline(&block);
                        self.at += 1;
                        task = task.or(self.task.take());
                        if !runs.is_empty() && !more.is_empty() {
                            runs.push(Run::plain("\n"));
                            self.note(&block, "Paragraphs of this list item were joined");
                        }
                        runs.extend(more);
                    }
                    Event::Start(tag) => {
                        let nested = matches!(tag, Tag::List(_));
                        let from = after.len();
                        self.read(tag, range, depth.saturating_add(1), &mut after);
                        for moved in after[from..].iter().filter(|_| !nested) {
                            self.note(moved, "Moved out of a list item");
                        }
                    }
                    Event::Rule => {
                        let rule = self.block(BlockKind::Divider);
                        self.note(&rule, "Moved out of a list item");
                        after.push(rule);
                    }
                    _ => {}
                }
            }

            let list = match task {
                Some(checked) => ListKind::Task { checked },
                None if start.is_some() => ListKind::Ordered,
                None => ListKind::Bullet,
            };
            block.kind = BlockKind::ListItem { list, depth };
            block.body = Body::Text(merge(runs));
            out.push(block);
            out.append(&mut after);
        }
    }

    /// Reads a table back into Markdown, since the document has no tables yet.
    fn table(&mut self, block: &Block, alignments: &[Alignment]) -> String {
        let mut rows: Vec<Vec<String>> = Vec::new();
        while let Some((event, _)) = self.next() {
            match event {
                Event::Start(Tag::TableHead | Tag::TableRow) => rows.push(Vec::new()),
                Event::Start(Tag::TableCell) => {
                    let cell = write::cell(&self.inline(block));
                    self.at += 1;
                    rows.last_mut().expect("a cell sits in a row").push(cell);
                }
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }
        let rule = |alignment: &Alignment| match alignment {
            Alignment::None => "---",
            Alignment::Left => ":--",
            Alignment::Center => ":-:",
            Alignment::Right => "--:",
        };
        table(rows, alignments.iter().map(|alignment| rule(alignment).to_owned()).collect())
    }

    /// Reads inline content up to the next block boundary, which is left for the caller.
    fn inline(&mut self, block: &Block) -> Vec<Run> {
        let mut text = Text::default();
        while let Some(event) = self.peek(0).cloned() {
            match event {
                Event::Text(part) => text.push(&part, None),
                Event::Code(part) => text.push(&part, Some(Mark::Code)),
                Event::SoftBreak => text.push(" ", None),
                Event::HardBreak => text.push("\n", None),
                Event::InlineHtml(html) => {
                    if !text.tag(&html) {
                        text.push(&html, None);
                        self.note(block, "HTML tag kept as text");
                    }
                }
                Event::InlineMath(tex) => {
                    text.push(&format!("${}$", unix_lines(&tex)), Some(Mark::Code));
                    self.note(block, MATHS);
                }
                Event::DisplayMath(tex) => {
                    text.push(&format!("$${}$$", unix_lines(tex.trim())), Some(Mark::Code));
                    self.note(block, MATHS);
                }
                Event::FootnoteReference(label) => {
                    text.push(&format!("[^{label}]"), None);
                    self.note(block, FOOTNOTE);
                }
                Event::TaskListMarker(checked) => self.task = Some(checked),
                Event::Start(Tag::Emphasis) => text.marks.push(Mark::Italic),
                Event::Start(Tag::Strong) => text.marks.push(Mark::Bold),
                Event::Start(Tag::Strikethrough) => text.marks.push(Mark::Strike),
                Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => drop(text.marks.pop()),
                Event::Start(Tag::Link { dest_url, title, .. }) => {
                    if !title.is_empty() {
                        self.note(block, format!("Link title dropped: {title}"));
                    }
                    text.links.push((address(&dest_url), text.len));
                }
                Event::Start(Tag::Image { dest_url, .. }) => {
                    self.note(block, "Image kept as a link");
                    text.links.push((address(&dest_url), text.len));
                }
                Event::End(TagEnd::Link) => drop(text.links.pop()),
                Event::End(TagEnd::Image) => {
                    // An image with no description is named by its address.
                    if let Some((url, len)) = text.links.last().cloned()
                        && len == text.len
                    {
                        text.push(&url, None);
                    }
                    text.links.pop();
                }
                _ => break,
            }
            self.at += 1;
        }
        text.runs
    }
}

/// The runs of one block as they are read, with the formatting open at this point in it.
#[derive(Default)]
struct Text {
    runs: Vec<Run>,
    /// Bytes pushed so far.
    len: usize,
    /// Marks opened by Markdown syntax, innermost last.
    marks: Vec<Mark>,
    /// Marks opened by HTML tags.
    tags: Vec<Mark>,
    /// Open links, each with the `len` at which it opened.
    links: Vec<(String, usize)>,
}

impl Text {
    fn push(&mut self, text: &str, mark: Option<Mark>) {
        if text.is_empty() {
            return;
        }
        self.len += text.len();
        let marks: BTreeSet<Mark> = self.marks.iter().chain(&self.tags).chain(&mark).cloned().collect();
        let link = self.links.last().map(|(url, _)| url.clone());
        match self.runs.last_mut() {
            Some(last) if last.marks == marks && last.link == link => last.text.push_str(text),
            _ => self.runs.push(Run { text: text.into(), marks, link, ..Run::default() }),
        }
    }

    /// Applies an HTML tag that stands for a mark or a line break. False for any other HTML.
    fn tag(&mut self, html: &str) -> bool {
        let Some(inner) = html.strip_prefix('<').and_then(|rest| rest.strip_suffix('>')) else { return false };
        if inner.trim_end_matches('/').trim().eq_ignore_ascii_case("br") {
            self.push("\n", None);
            return true;
        }
        let (name, closes) = inner.strip_prefix('/').map_or((inner, false), |name| (name, true));
        let Some((_, mark)) = TAGS.iter().find(|(tag, _)| tag.eq_ignore_ascii_case(name)) else { return false };
        if !closes {
            self.tags.push(mark.clone());
        } else if let Some(open) = self.tags.iter().rposition(|open| open == mark) {
            self.tags.remove(open);
        } else {
            return false;
        }
        true
    }
}

/// Maths is the one text the parser hands over with the file's own line endings.
fn unix_lines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// A link's address. A wiki link can wrap, and an address cannot hold the line break.
fn address(url: &str) -> String {
    url.replace(['\n', '\r'], " ")
}

/// Lets each list item sit at most one level below the item before it. A block that is not an item splits the
/// list, so the items after it start over.
pub(crate) fn nest(blocks: &mut [Block]) {
    let mut deepest = 0;
    for block in blocks {
        deepest = match &mut block.kind {
            BlockKind::ListItem { depth, .. } => {
                *depth = (*depth).min(deepest);
                depth.saturating_add(1)
            }
            _ => 0,
        };
    }
}

/// Lays rows of cells out as a Markdown table, with `rules` under its first row.
pub(crate) fn table(mut rows: Vec<Vec<String>>, rules: Vec<String>) -> String {
    rows.insert(rows.len().min(1), rules);
    rows.iter().map(|row| format!("| {} |", row.join(" | "))).collect::<Vec<_>>().join("\n")
}

/// Joins neighbouring runs that share their formatting.
pub(crate) fn merge(mut runs: Vec<Run>) -> Vec<Run> {
    runs.dedup_by(|next, last| {
        let same = last.marks == next.marks && last.link == next.link;
        if same {
            last.text.push_str(&next.text);
        }
        same
    });
    runs
}
