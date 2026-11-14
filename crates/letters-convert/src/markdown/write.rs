//! Document to Markdown.

use std::cmp::Reverse;
use std::ops::Range;

use letters_model::{Block, BlockKind, Document, ListKind, Mark, Run, Tone};

use super::{TAGS, TONES, from_markdown};

/// Writes Markdown. Reading the result back gives an equal document, apart from block ids and what Markdown
/// cannot say:
///
/// - Emphasis never starts or ends on a space, so spaces at the edge of a bold, italic or struck span leave it.
///   Inline code cannot hold a line break, so the break leaves it too.
/// - An empty paragraph is left out, and a list item sits at most one level below the item before it.
/// - An unknown block is written as its fallback text. Unknown marks and fields are left out.
pub fn to_markdown(doc: &Document) -> String {
    let mut out = blocks(&doc.blocks).join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// A table cell, for the reader to keep a table as source.
pub(super) fn cell(runs: &[Run]) -> String {
    text(runs, Context::Cell)
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
fn blocks(blocks: &[Block]) -> Vec<String> {
    let mut out = Vec::new();
    let mut levels: Vec<Level> = Vec::new();
    // The item before this one, while a list is being written.
    let mut last: Option<&Block> = None;
    for block in blocks.iter().filter(|block| is_written(block)) {
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
            out.extend(other(block));
            levels.clear();
            last = None;
        }
    }
    out
}

/// Whether a block has a Markdown form. An empty paragraph has none, nor has an unknown block with no fallback.
fn is_written(block: &Block) -> bool {
    match block.kind {
        BlockKind::Paragraph => !block.plain_text().is_empty(),
        BlockKind::Unknown { .. } => !fallback(block).is_empty(),
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
        BlockKind::Quote => quote(None, block.body.blocks()),
        BlockKind::Callout { tone } => quote(Some(*tone), block.body.blocks()),
        BlockKind::Divider => vec!["---".to_owned()],
        BlockKind::Unknown { .. } => lines(&text(&[Run::plain(fallback(block))], Context::Lines)),
        // A list item is written by `item`, which knows the list around it.
        BlockKind::ListItem { .. } => Vec::new(),
    }
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

fn quote(tone: Option<Tone>, children: &[Block]) -> Vec<String> {
    let inner = blocks(children);
    let mut out = Vec::new();
    if let Some(tone) = tone {
        let (name, _) = TONES.iter().find(|(_, named)| *named == tone).expect("every tone is named");
        out.push(format!("[!{name}]"));
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
/// as HTML tags where Markdown's rules about the characters beside a delimiter would lose it.
fn text(runs: &[Run], context: Context) -> String {
    let inline = Inline::new(runs);
    let written = inline.write(context, false);
    if inline.has(EMPHASIS) && !inline.reads_back(&written, context) { inline.write(context, true) } else { written }
}

const BOLD: u8 = 1;
const ITALIC: u8 = 2;
const STRIKE: u8 = 4;
const UNDERLINE: u8 = 8;
const HIGHLIGHT: u8 = 16;
const CODE: u8 = 32;
/// The marks written with delimiters, which Markdown only accepts next to the right characters.
const EMPHASIS: u8 = BOLD | ITALIC | STRIKE;

const MARKS: [(u8, Mark); 6] = [
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
    /// One dollar is a price. Two could hold maths between them, so they are escaped.
    maths: bool,
}

impl<'a> Inline<'a> {
    fn new(runs: &'a [Run]) -> Self {
        let mut chars = Vec::new();
        let mut formats = Vec::new();
        for run in runs {
            let bit = |mark| MARKS.iter().find(|(_, known)| known == mark).map_or(0, |(bit, _)| *bit);
            let marks = run.marks.iter().fold(0, |marks, mark| marks | bit(mark));
            for c in run.text.chars() {
                // A code span cannot hold a line break.
                let marks = if matches!(c, '\n' | '\r') { marks & !CODE } else { marks };
                chars.push(c);
                formats.push(Format { marks, link: run.link.as_deref() });
            }
        }

        // Markdown cannot open emphasis before a space or close it after one, so those spaces leave the span.
        let space = |i: usize, formats: &[Format]| chars[i].is_whitespace() && formats[i].marks & CODE == 0;
        for bit in [BOLD, ITALIC, STRIKE] {
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
        let maths = chars.iter().filter(|c| **c == '$').count() > 1;
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
                link: format.link.map(str::to_owned),
                ..Run::default()
            })
            .collect();
        // A heading stands in for a cell: both are one line of text.
        let source = if context == Context::Lines { written.to_owned() } else { format!("# {written}") };
        let (doc, _) = from_markdown(&source, &mut String::new);
        matches!(doc.blocks.as_slice(), [block] if block.body.runs() == runs)
    }

    /// Writes the text. With `tags`, emphasis is written as HTML, which reads back whatever sits beside it.
    fn write(&self, context: Context, tags: bool) -> String {
        let segments = self.segments();
        let mut out = String::new();
        let mut open: Vec<Span> = Vec::new();
        for (i, (range, format)) in segments.iter().enumerate() {
            let autolink = self.is_autolink(&segments, i);
            let mut spans: Vec<Span> = format.link.filter(|_| !autolink).map(Span::Link).into_iter().collect();
            spans.extend(
                MARKS.iter().filter(|(bit, _)| *bit != CODE && format.marks & bit != 0).map(|m| Span::Mark(m.0)),
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
            && format.marks & CODE == 0
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

fn start(span: Span, tags: bool) -> String {
    let delimiter = match span {
        Span::Link(_) => "[",
        Span::Mark(BOLD) if !tags => "**",
        Span::Mark(ITALIC) if !tags => "*",
        Span::Mark(STRIKE) if !tags => "~~",
        Span::Mark(bit) => return format!("<{}>", tag(bit)),
    };
    delimiter.to_owned()
}

fn close(span: Span, tags: bool) -> String {
    match span {
        Span::Link(url) => format!("]({})", destination(url)),
        Span::Mark(bit) if tags || bit & EMPHASIS == 0 => format!("</{}>", tag(bit)),
        Span::Mark(_) => start(span, tags),
    }
}

/// The HTML tag that stands for a mark.
fn tag(bit: u8) -> &'static str {
    let named = |mark: &Mark| TAGS.iter().find(|(_, tagged)| tagged == mark).map(|(name, _)| *name);
    MARKS.iter().find(|(known, _)| *known == bit).and_then(|(_, mark)| named(mark)).expect("every mark has a tag")
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
