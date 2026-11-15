//! Markdown to document.

use std::collections::BTreeSet;
use std::mem;
use std::ops::Range;

use letters_model::{Block, BlockKind, Body, Document, ListKind, Mark, Properties, PropertiesFormat, Run, Tone};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use super::{TAGS, TONES, write};
use crate::ImportNote;

/// Quotes and lists nested deeper than this are kept as source, which bounds the reader's recursion.
const MAX_DEPTH: usize = 32;

const MOVED: &str = "Moved out of a list item";
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
    let (properties, source) = front_matter(markdown);
    let events = Parser::new_ext(source, options).into_offset_iter().collect();
    let mut reader = Reader { source, events, at: 0, depth: 0, new_id, notes: Vec::new(), task: None };
    let blocks = reader.blocks();
    (Document { properties, blocks, ..Document::default() }, reader.notes)
}

/// The fence that opens and closes front matter in each format.
pub(super) const FENCES: [(&str, PropertiesFormat); 2] =
    [("---", PropertiesFormat::Yaml), ("+++", PropertiesFormat::Toml)];

/// Splits front matter off the start of a file: the lines between a first line that is a fence and the next
/// such line. Returns it as the document's properties, then the rest of the file.
pub(super) fn front_matter(markdown: &str) -> (Option<Properties>, &str) {
    for (fence, format) in FENCES {
        let Some(rest) = markdown.strip_prefix(fence) else { continue };
        let Some(body) = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n')) else { continue };
        // A blank line straight after the fence makes it a divider.
        if body.trim_start_matches([' ', '\t']).starts_with(['\n', '\r']) {
            continue;
        }
        let mut end = 0;
        for line in body.split_inclusive('\n') {
            if line.trim_end() == fence {
                let properties = Properties { format, source: unix_lines(&body[..end]), extra: Default::default() };
                return (Some(properties), &body[end + line.len()..]);
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

    fn formula(&mut self, tex: &str) -> Block {
        let mut block = self.block(BlockKind::Formula);
        if !tex.is_empty() {
            block.body = Body::Text(vec![Run::plain(tex)]);
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
                let mut block = self.block(BlockKind::Paragraph);
                let (runs, formulas) = self.flow(&block);
                self.at += 1;
                if !runs.is_empty() {
                    block.body = Body::Text(runs);
                    out.push(block);
                }
                out.extend(formulas);
            }
            Tag::Heading { level, .. } => {
                let level = level as u8;
                let mut block = self.block(BlockKind::Heading { level: level.min(4) });
                if level > 4 {
                    self.note(&block, format!("Heading level {level} became level 4"));
                }
                block.body = Body::Text(self.line(&block));
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
                let text = text.strip_suffix('\n').unwrap_or(&text);
                // GitHub shows a code block in this language as maths.
                if language == "math" && options.is_empty() {
                    return out.push(self.formula(text));
                }
                let block = self.code(Some(language).filter(|name| !name.is_empty()), text);
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
            let (mut runs, mut after) = self.flow(&block);
            let mut task = self.task.take();
            after.iter().for_each(|moved| self.note(moved, MOVED));
            while let Some((event, range)) = self.next() {
                match event {
                    Event::End(_) => break,
                    // The paragraphs an item opens with are its text. Anything else follows the item.
                    Event::Start(Tag::Paragraph) if after.is_empty() => {
                        let (more, formulas) = self.flow(&block);
                        self.at += 1;
                        task = task.or(self.task.take());
                        if !runs.is_empty() && !more.is_empty() {
                            runs.push(Run::plain("\n"));
                            self.note(&block, "Paragraphs of this list item were joined");
                        }
                        runs.extend(more);
                        formulas.iter().for_each(|moved| self.note(moved, MOVED));
                        after.extend(formulas);
                    }
                    Event::Start(tag) => {
                        let nested = matches!(tag, Tag::List(_));
                        let from = after.len();
                        self.read(tag, range, depth.saturating_add(1), &mut after);
                        for moved in after[from..].iter().filter(|_| !nested) {
                            self.note(moved, MOVED);
                        }
                    }
                    Event::Rule => {
                        let rule = self.block(BlockKind::Divider);
                        self.note(&rule, MOVED);
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
                    let cell = write::cell(&self.line(block));
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

    /// Reads text up to the next block boundary, which is left for the caller. Display maths in it is a formula
    /// of its own: the text before the first is returned for `block`, then each formula and the text after it
    /// as the blocks that follow.
    fn flow(&mut self, block: &Block) -> (Vec<Run>, Vec<Block>) {
        let mut text = Text::default();
        self.inline(block, &mut text, true);
        let mut first = mem::take(&mut text.runs);
        let mut rest = Vec::new();
        while let Some(Event::DisplayMath(tex)) = self.peek(0).cloned() {
            self.at += 1;
            rest.push(self.formula(&unix_lines(tex.trim())));
            let mut after = self.block(BlockKind::Paragraph);
            self.inline(&after, &mut text, true);
            let mut runs = mem::take(&mut text.runs);
            trim(&mut runs);
            if !runs.is_empty() {
                after.body = Body::Text(runs);
                rest.push(after);
            }
        }
        if !rest.is_empty() {
            trim(&mut first);
        }
        (first, rest)
    }

    /// Reads the text of a block that is one line, where display maths can only sit in the line.
    fn line(&mut self, block: &Block) -> Vec<Run> {
        let mut text = Text::default();
        self.inline(block, &mut text, false);
        text.runs
    }

    /// Reads inline content into `text`, up to the next block boundary, which is left for the caller. With
    /// `formulas`, display maths is such a boundary.
    fn inline(&mut self, block: &Block, text: &mut Text, formulas: bool) {
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
                    // A price is not maths. As Pandoc reads them, the dollar that ends maths has no digit after it.
                    let end = self.events[self.at].1.end;
                    if self.source.as_bytes().get(end).is_some_and(u8::is_ascii_digit) {
                        text.push(&format!("${}$", one_line(&tex)), None);
                    } else {
                        text.push(&one_line(&tex), Some(Mark::Math));
                    }
                }
                Event::DisplayMath(_) if formulas => break,
                Event::DisplayMath(tex) => {
                    // Not all that two dollar signs hold can sit between single ones, and so be written back.
                    let tex = one_line(tex.trim());
                    let inline = format!("${tex}$");
                    let mut events = Parser::new_ext(&inline, Options::ENABLE_MATH).skip(1);
                    let fits = matches!(events.next(), Some(Event::InlineMath(read)) if *read == *tex)
                        && matches!(events.next(), Some(Event::End(_)));
                    text.push(&tex, Some(if fits { Mark::Math } else { Mark::Code }));
                    self.note(block, "Display maths set in its line");
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

/// Maths that sits in a line of text, where a line break in its source is only a space.
fn one_line(tex: &str) -> String {
    unix_lines(tex).replace('\n', " ")
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

/// Drops the spaces and line breaks at the edges of a block's text.
pub(crate) fn trim(runs: &mut Vec<Run>) {
    const EDGE: [char; 3] = [' ', '\t', '\n'];
    let blank = |run: &Run| run.text.trim_matches(EDGE).is_empty();
    runs.truncate(runs.len() - runs.iter().rev().take_while(|run| blank(run)).count());
    runs.drain(..runs.iter().take_while(|run| blank(run)).count());
    if let Some(last) = runs.last_mut() {
        last.text.truncate(last.text.trim_end_matches(EDGE).len());
    }
    if let Some(first) = runs.first_mut() {
        first.text.drain(..first.text.len() - first.text.trim_start_matches(EDGE).len());
    }
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
