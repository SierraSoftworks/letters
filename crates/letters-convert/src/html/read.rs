//! HTML to document, for what browsers, Google Docs and Word put on the clipboard.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;

use html5gum::{DefaultEmitter, HtmlString, Token, Tokenizer};
use letters_model::{Align, Block, BlockKind, Body, Document, ListKind, Mark, Run, Tone};

use super::href;
use crate::ImportNote;
use crate::markdown::{TAGS, TONES, merge, nest, trim};

/// Elements nested deeper than this are read as their text, which bounds the reader's recursion.
const MAX_DEPTH: usize = 128;

/// A link's address longer than this is dropped, so that a page cannot make each run of a paste carry megabytes.
const MAX_ADDRESS: usize = 4096;

/// Elements that hold nothing and have no end tag.
const VOID: [&str; 13] =
    ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track", "wbr"];

/// Elements whose content is not text of the page. `o:p` is Word's, and holds at most a filler space.
const UNREAD: [&str; 13] = [
    "button", "head", "iframe", "math", "noscript", "o:p", "object", "script", "select", "style", "svg", "template",
    "title",
];

/// Elements that start a block of their own. Any other element is part of the text around it.
const BLOCKS: [&str; 42] = [
    "address",
    "article",
    "aside",
    "blockquote",
    "caption",
    "center",
    "dd",
    "details",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "dir",
    "menu",
];

/// Elements whose end tag may be left out, each with the elements it cannot reach past. One ends where the
/// next of its kind starts.
const SIBLINGS: [(&[&str], &[&str]); 3] =
    [(&["li"], &["ul", "ol"]), (&["td", "th"], &["tr", "table"]), (&["tr"], &["table"])];

/// The style properties the reader acts on.
const PROPERTIES: [&str; 8] = [
    "display",
    "font-style",
    "font-weight",
    "mso-list",
    "text-align",
    "text-decoration",
    "text-decoration-line",
    "white-space",
];

/// The empty cells one read adds where cells span rows or columns. Past it, spans are not filled in, so that
/// a page cannot make a small paste a huge table.
const MAX_SPANNED: usize = 10_000;

/// Reads the HTML another application put on the clipboard: a fragment or a whole page, with or without the
/// comments that mark what was copied. `new_id` supplies an id for each block.
///
/// - Elements are read for what they mean, and inline styles for the marks they stand for. Other styling is
///   dropped, and an unknown element is read through to its text.
/// - White space collapses as a browser shows it. A fragment with no block in it is one paragraph that keeps
///   the spaces at its edges, so that a phrase can be poured into a line.
/// - Maths is read where a page carries its TeX source, and left out where it carries only a drawing of it.
/// - Blocks with no text, scripts, style sheets and images are left out.
/// - A table keeps its rows and the text of its cells. A cell that spans several leaves empty cells where it
///   reaches. A table that only lays a page out, with one column or in that role, is read as what its cells
///   hold.
pub fn from_html(html: &str, new_id: &mut dyn FnMut() -> String) -> (Document, Vec<ImportNote>) {
    let mut nodes = parse(copied(html));
    // Excel marks the rows it copied, which leaves out the table around them.
    if nodes.iter().any(|node| matches!(node, Node::Element(element) if element.name == "tr")) {
        nodes = vec![Node::Element(Element { name: "table".into(), children: nodes, ..Element::default() })];
    }
    let mut reader = Reader {
        new_id,
        notes: Vec::new(),
        runs: Vec::new(),
        kind: BlockKind::Paragraph,
        gap: false,
        blocks: false,
        task: None,
        marker: None,
        cell: false,
        spare: MAX_SPANNED,
    };
    let mut blocks = Vec::new();
    reader.nodes(&nodes, &Style::default(), &mut blocks);
    reader.flush(&mut blocks);
    nest(&mut blocks);
    (Document { blocks, ..Document::default() }, reader.notes)
}

/// The part of the clipboard's HTML that was copied, where the application marks it.
fn copied(html: &str) -> &str {
    const START: &str = "<!--StartFragment-->";
    let from = html.find(START).map_or(0, |at| at + START.len());
    let to = html[from..].find("<!--EndFragment-->").map_or(html.len(), |at| from + at);
    &html[from..to]
}

enum Node {
    Text(String),
    Element(Element),
}

#[derive(Default)]
struct Element {
    name: String,
    attributes: BTreeMap<String, String>,
    /// The declarations of its style attribute that the reader acts on, in lower case.
    css: Vec<(&'static str, String)>,
    children: Vec<Node>,
}

impl Element {
    fn attribute(&self, name: &str) -> &str {
        self.attributes.get(name).map_or("", String::as_str)
    }

    fn classes(&self) -> impl Iterator<Item = &str> {
        self.attribute("class").split_ascii_whitespace()
    }

    fn css(&self, property: &str) -> Option<&str> {
        self.css.iter().rev().find(|(name, _)| *name == property).map(|(_, value)| value.as_str())
    }

    fn is_hidden(&self) -> bool {
        self.attributes.contains_key("hidden") || self.css("display") == Some("none")
    }

    /// The tone of a callout as Letters exports one, or as GitHub renders an alert.
    fn tone(&self) -> Option<Tone> {
        self.classes().find(|class| matches!(*class, "callout" | "markdown-alert"))?;
        self.classes().find_map(|class| {
            let name = class.strip_prefix("markdown-alert-").unwrap_or(class);
            TONES.iter().find(|(tag, _)| tag.eq_ignore_ascii_case(name)).map(|(_, tone)| *tone)
        })
    }

    /// The TeX source of the maths this element stands for, and whether it is set on a line of its own. The
    /// source is found where pages carry it: annotated on MathML, as KaTeX and Wikipedia write it, in the
    /// script MathJax leaves, or on an element of the class `math`: between delimiters as Pandoc writes it, or
    /// in `data-tex` as `to_html` does.
    fn maths(&self) -> Option<(String, bool)> {
        let text = |element: &Element| {
            let mut text = String::new();
            raw(&element.children, &mut text);
            text
        };
        let class = self.attribute("class");
        match self.name.as_str() {
            "math" => {
                let is_source = |element: &Element| {
                    element.name == "annotation" && element.attribute("encoding").to_ascii_lowercase().contains("tex")
                };
                let source = self.find(&is_source).map_or_else(|| self.attribute("alttext").to_owned(), text);
                Some((source, self.attribute("display") == "block"))
            }
            "script" => {
                let kind = self.attribute("type").strip_prefix("math/tex")?;
                Some((text(self), kind.contains("display")))
            }
            // Wikipedia shows a picture of the maths, described by its source.
            "img" if class.starts_with("mwe-math-fallback-image") => {
                Some((self.attribute("alt").to_owned(), class.ends_with("display")))
            }
            // A page of `to_html` shows a drawing of the maths, and keeps the source beside it.
            _ if self.classes().any(|class| class == "math") && !self.attribute("data-tex").is_empty() => {
                Some((self.attribute("data-tex").to_owned(), self.classes().any(|class| class == "display")))
            }
            _ if self.classes().any(|class| class == "math") => {
                let source = text(self);
                [(r"\(", r"\)", false), (r"\[", r"\]", true)].into_iter().find_map(|(open, close, display)| {
                    Some((source.trim().strip_prefix(open)?.strip_suffix(close)?.to_owned(), display))
                })
            }
            _ => None,
        }
    }

    /// The first element inside this one that `is` what is looked for.
    fn find(&self, is: &dyn Fn(&Element) -> bool) -> Option<&Element> {
        self.children.iter().find_map(|node| match node {
            Node::Element(child) if is(child) => Some(child),
            Node::Element(child) => child.find(is),
            Node::Text(_) => None,
        })
    }

    /// How the element says its text is aligned, as a table's cell does.
    fn align(&self) -> Option<Align> {
        let stated = self.css("text-align").map_or_else(|| self.attribute("align").to_ascii_lowercase(), str::to_owned);
        match stated.as_str() {
            "left" | "start" => Some(Align::Start),
            "center" => Some(Align::Center),
            "right" | "end" => Some(Align::End),
            "justify" => Some(Align::Justify),
            _ => None,
        }
    }

    /// How many rows or columns a cell reaches over, as the attribute `name` says.
    fn span(&self, name: &str) -> usize {
        self.attribute(name).trim().parse().unwrap_or(1).clamp(1, 1000)
    }

    /// How deep in its list a paragraph of Word's sits. Word writes list items as paragraphs.
    fn word_depth(&self) -> Option<u8> {
        let level = self.css("mso-list")?.split_whitespace().find_map(|word| word.strip_prefix("level"))?;
        Some(level.parse::<u8>().ok()?.saturating_sub(1))
    }
}

/// Reads tags into a tree the way a browser would, as far as clipboard HTML needs: end tags that are missing
/// or stray are settled, and nothing is moved.
fn parse(html: &str) -> Vec<Node> {
    let string = |text: &HtmlString| String::from_utf8_lossy(text).into_owned();
    let close = |open: &mut Vec<Element>| {
        if let (Some(element), Some(parent)) = (open.pop(), open.last_mut()) {
            parent.children.push(Node::Element(element));
        }
    };
    let mut emitter = DefaultEmitter::default();
    // So that the text of a script or a style sheet is not read as tags.
    emitter.naively_switch_states(true);
    let mut open = vec![Element::default()];
    // The elements open beyond the deepest one kept.
    let mut beyond = 0usize;
    for token in Tokenizer::new_with_emitter(html, emitter).flatten() {
        let top = open.len() - 1;
        match token {
            Token::String(text) => open[top].children.push(Node::Text(string(&text))),
            Token::StartTag(tag) => {
                let mut element = Element { name: string(&tag.name), ..Element::default() };
                for (name, value) in &tag.attributes {
                    match name.as_slice() {
                        b"style" => element.css = declarations(&string(value)),
                        _ => _ = element.attributes.insert(string(name), string(value)),
                    }
                }
                if tag.self_closing || VOID.contains(&element.name.as_str()) {
                    open[top].children.push(Node::Element(element));
                } else if open.len() > MAX_DEPTH {
                    beyond += 1;
                } else {
                    let name = element.name.as_str();
                    if let Some((kind, limits)) = SIBLINGS.iter().find(|(kind, _)| kind.contains(&name)) {
                        let reach = open.iter().rposition(|open| limits.contains(&open.name.as_str())).unwrap_or(0);
                        if let Some(at) = open[reach..].iter().rposition(|open| kind.contains(&open.name.as_str())) {
                            (reach + at..open.len()).for_each(|_| close(&mut open));
                        }
                    }
                    open.push(element);
                }
            }
            Token::EndTag(_) if beyond > 0 => beyond -= 1,
            Token::EndTag(tag) => {
                let name = string(&tag.name);
                if let Some(at) = open.iter().rposition(|open| open.name == name).filter(|at| *at > 0) {
                    (at..open.len()).for_each(|_| close(&mut open));
                }
            }
            _ => {}
        }
    }
    (1..open.len()).for_each(|_| close(&mut open));
    open.pop().map(|root| root.children).unwrap_or_default()
}

fn declarations(style: &str) -> Vec<(&'static str, String)> {
    let declaration = |declaration: &str| {
        let (property, value) = declaration.split_once(':')?;
        let property = PROPERTIES.iter().find(|name| name.eq_ignore_ascii_case(property.trim()))?;
        let value = value.split('!').next().unwrap_or_default().trim().to_ascii_lowercase();
        Some((*property, value))
    };
    style.split(';').filter_map(declaration).collect()
}

/// The text under `nodes` as it is written, with a line for each block and line break.
fn raw(nodes: &[Node], out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Element(element) if UNREAD.contains(&element.name.as_str()) => {}
            Node::Element(element) if element.name == "br" => out.push('\n'),
            Node::Element(element) => {
                let block = BLOCKS.contains(&element.name.as_str());
                if block && !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                raw(&element.children, out);
                if block && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
        }
    }
}

/// The text of `blocks` as the runs of one block, a line to each.
fn flatten(blocks: Vec<Block>, runs: &mut Vec<Run>) {
    for block in blocks {
        match block.body {
            Body::Text(text) if !text.is_empty() => {
                if !runs.is_empty() {
                    runs.push(Run::plain("\n"));
                }
                runs.extend(text);
            }
            Body::Blocks(children) => flatten(children, runs),
            _ => {}
        }
    }
}

/// What text takes from the elements around it.
#[derive(Clone, Default)]
struct Style {
    marks: BTreeSet<Mark>,
    link: Option<String>,
    /// Whether white space is kept as it is written.
    pre: bool,
    /// Whether the text is bold as it stands, as a heading's is. A weight it states is then not a mark.
    heavy: bool,
    /// The kind of the list around it, and how many lists that one sits in.
    list: Option<(ListKind, u8)>,
}

impl Style {
    /// The style inside `element`.
    fn inside(&self, element: &Element) -> Self {
        let mut style = self.clone();
        let name = element.name.as_str();
        style.heavy |= matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "th");
        match name {
            "kbd" | "samp" | "tt" => _ = style.marks.insert(Mark::Code),
            "a" => style.link = href(element.attribute("href")).filter(|address| address.len() <= MAX_ADDRESS),
            _ => style.marks.extend(TAGS.iter().filter(|(tag, _)| *tag == name).map(|(_, mark)| mark.clone())),
        }

        // A style overrules its element: Google Docs wraps what it copies in a `b` styled as normal.
        let bold = match element.css("font-weight") {
            Some("bold" | "bolder") => Some(true),
            Some("normal" | "lighter") => Some(false),
            Some(number) => number.parse::<f32>().ok().map(|weight| weight >= 600.0),
            None => None,
        };
        let italic = match element.css("font-style") {
            Some("normal") => Some(false),
            Some(slant) => (slant.starts_with("italic") || slant.starts_with("oblique")).then_some(true),
            None => None,
        };
        for (mark, on) in [(Mark::Bold, bold.filter(|_| !style.heavy)), (Mark::Italic, italic)] {
            match on {
                Some(true) => _ = style.marks.insert(mark),
                Some(false) => _ = style.marks.remove(&mark),
                None => {}
            }
        }
        for lines in ["text-decoration", "text-decoration-line"].iter().filter_map(|name| element.css(name)) {
            // A browser spells out the line it draws under a link, which is not a mark of the text.
            if lines.contains("underline") && style.link.is_none() {
                style.marks.insert(Mark::Underline);
            }
            if lines.contains("line-through") {
                style.marks.insert(Mark::Strike);
            }
        }
        if let Some(space) = element.css("white-space") {
            style.pre = matches!(space, "pre" | "pre-wrap" | "break-spaces");
        }
        style
    }
}

struct Reader<'f> {
    new_id: &'f mut dyn FnMut() -> String,
    notes: Vec<ImportNote>,
    /// The text read since the last block ended.
    runs: Vec<Run>,
    /// The kind of block that text will be.
    kind: BlockKind,
    /// Whether a space read now would follow a space or a line's start, and so not show.
    gap: bool,
    /// Whether any element that starts a block has been read.
    blocks: bool,
    /// The checkbox of the list item being read, once it has been seen.
    task: Option<bool>,
    /// The bullet or number that Word wrote before the list item being read.
    marker: Option<String>,
    /// Whether a table's cell is being read, where a table reads as its text.
    cell: bool,
    /// How many more empty cells may be added where cells span.
    spare: usize,
}

impl Reader<'_> {
    fn block(&mut self, kind: BlockKind) -> Block {
        Block::new((self.new_id)().as_str(), kind)
    }

    fn push(&mut self, text: &str, style: &Style) {
        match self.runs.last_mut() {
            Some(last) if last.marks == style.marks && last.link == style.link => last.text.push_str(text),
            _ if text.is_empty() => {}
            _ => self.runs.push(Run {
                text: text.to_owned(),
                marks: style.marks.clone(),
                link: style.link.clone(),
                ..Run::default()
            }),
        }
    }

    fn text(&mut self, text: &str, style: &Style) {
        let mut shown = String::with_capacity(text.len());
        for c in text.chars().filter(|c| *c != '\r') {
            let space = !style.pre && matches!(c, ' ' | '\t' | '\n' | '\x0c');
            if !space {
                shown.push(c);
            } else if !self.gap {
                shown.push(' ');
            }
            self.gap = space;
        }
        self.push(&shown, style);
    }

    fn line_break(&mut self, style: &Style) {
        // A space on either side of a line break does not show.
        if let Some(last) = self.runs.last_mut().filter(|last| self.gap && last.text.ends_with(' ')) {
            last.text.pop();
            if last.text.is_empty() {
                self.runs.pop();
            }
        }
        self.push("\n", style);
        self.gap = true;
    }

    /// Reads maths from its source: a formula between the blocks around it, or a run in the text.
    fn maths(&mut self, tex: &str, display: bool, style: &Style, out: &mut Vec<Block>) {
        let tex = tex.replace('\r', "");
        let tex = tex.trim();
        if tex.is_empty() {
            return;
        }
        if display {
            self.blocks = true;
            self.flush(out);
            let mut block = self.block(BlockKind::Formula);
            block.body = Body::Text(vec![Run::plain(tex)]);
            out.push(block);
        } else {
            let mut style = style.clone();
            style.marks.insert(Mark::Math);
            self.push(&tex.replace('\n', " "), &style);
            self.gap = false;
        }
    }

    /// Ends the block being read. One with nothing to show is left out.
    fn flush(&mut self, out: &mut Vec<Block>) {
        let mut runs = mem::take(&mut self.runs);
        // A phrase copied from the middle of a line keeps the spaces at its edges.
        if self.blocks {
            trim(&mut runs);
        }
        self.gap = self.blocks;
        if runs.iter().any(|run| !run.text.trim().is_empty()) {
            let mut block = self.block(self.kind.clone());
            block.body = Body::Text(runs);
            out.push(block);
        }
    }

    /// Reads `element`'s content as blocks whose text is of `kind`.
    fn within(&mut self, kind: BlockKind, element: &Element, style: &Style, out: &mut Vec<Block>) {
        let outer = mem::replace(&mut self.kind, kind);
        self.nodes(&element.children, style, out);
        self.flush(out);
        self.kind = outer;
    }

    fn nodes(&mut self, nodes: &[Node], style: &Style, out: &mut Vec<Block>) {
        for node in nodes {
            match node {
                Node::Text(text) => self.text(text, style),
                Node::Element(element) => self.element(element, style, out),
            }
        }
    }

    fn element(&mut self, element: &Element, outer: &Style, out: &mut Vec<Block>) {
        let name = element.name.as_str();
        // KaTeX writes maths twice: as MathML, which carries its source, and as the boxes it draws.
        if element.is_hidden() || element.classes().any(|class| matches!(class, "markdown-alert-title" | "katex-html"))
        {
            return;
        }
        if let Some((tex, display)) = element.maths() {
            return self.maths(&tex, display, outer, out);
        }
        if UNREAD.contains(&name) {
            return;
        }
        let mut style = outer.inside(element);
        if !BLOCKS.contains(&name) {
            match name {
                "br" => self.line_break(&style),
                "input" if element.attribute("type").eq_ignore_ascii_case("checkbox") => {
                    self.task = Some(element.attributes.contains_key("checked"));
                }
                _ if element.css("mso-list") == Some("ignore") => {
                    let marker = self.marker.insert(String::new());
                    raw(&element.children, marker);
                }
                _ => self.nodes(&element.children, &style, out),
            }
            return;
        }

        self.blocks = true;
        self.flush(out);
        match name {
            "hr" => out.push(self.block(BlockKind::Divider)),
            "pre" => self.code(element, out),
            "div" if element.css("white-space") == Some("pre") => self.code(element, out),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                let level = name[1..].parse::<u8>().unwrap_or(1);
                let from = out.len();
                self.within(BlockKind::Heading { level: level.min(4) }, element, &style, out);
                if let Some(heading) = out.get(from).filter(|_| level > 4) {
                    let message = format!("Heading level {level} became level 4");
                    self.notes.push(ImportNote { block: heading.id.to_string(), message });
                }
            }
            "ul" | "ol" => {
                let list = if name == "ol" { ListKind::Ordered } else { ListKind::Bullet };
                style.list = Some((list, outer.list.map_or(0, |(_, depth)| depth.saturating_add(1))));
                self.nodes(&element.children, &style, out);
            }
            "li" => self.item(element, &style, out),
            "blockquote" => self.container(BlockKind::Quote, element, &style, out),
            "table" => self.table(element, &style, out),
            "p" if element.word_depth().is_some() => {
                self.marker = None;
                self.nodes(&element.children, &style, out);
                let numbered = self.marker.take().is_some_and(|marker| marker.trim_end().ends_with(['.', ')']));
                let list = if numbered { ListKind::Ordered } else { ListKind::Bullet };
                let depth = element.word_depth().unwrap_or_default();
                let outer = mem::replace(&mut self.kind, BlockKind::ListItem { list, depth });
                self.flush(out);
                self.kind = outer;
            }
            _ => match element.tone() {
                Some(tone) => self.container(BlockKind::Callout { tone }, element, &style, out),
                None => self.nodes(&element.children, &style, out),
            },
        }
        self.flush(out);
    }

    fn code(&mut self, element: &Element, out: &mut Vec<Block>) {
        let inner = element.children.iter().filter_map(|node| match node {
            Node::Element(child) => Some(child),
            Node::Text(_) => None,
        });
        let language = [element].into_iter().chain(inner).flat_map(Element::classes).find_map(|class| {
            let name = class.strip_prefix("language-").or_else(|| class.strip_prefix("lang-"))?;
            (!name.is_empty()).then(|| name.to_owned())
        });
        let mut text = String::new();
        raw(&element.children, &mut text);
        let text = text.replace('\r', "");
        let text = text.trim_matches('\n');
        let mut block = self.block(BlockKind::Code { language });
        if !text.is_empty() {
            block.body = Body::Text(vec![Run::plain(text)]);
        }
        out.push(block);
    }

    /// Reads a list item. The text it opens with is the item, and anything else follows it.
    fn item(&mut self, element: &Element, style: &Style, out: &mut Vec<Block>) {
        let (list, depth) = style.list.unwrap_or((ListKind::Bullet, 0));
        let from = out.len();
        let around = self.task.take();
        self.within(BlockKind::Paragraph, element, style, out);
        // Google Docs writes a checklist item as a list item in the role of a checkbox.
        let role = (element.attribute("role") == "checkbox").then(|| element.attribute("aria-checked") == "true");
        let task = mem::replace(&mut self.task, around).or(role);
        let kind = BlockKind::ListItem { list: task.map_or(list, |checked| ListKind::Task { checked }), depth };
        match out.get_mut(from) {
            Some(first) if matches!(first.kind, BlockKind::Paragraph | BlockKind::Heading { .. }) => first.kind = kind,
            _ => out.insert(from, self.block(kind)),
        }

        let paragraphs = out[from + 1..].iter().take_while(|block| block.kind == BlockKind::Paragraph).count();
        let more: Vec<Block> = out.drain(from + 1..from + 1 + paragraphs).collect();
        if let (Body::Text(runs), false) = (&mut out[from].body, more.is_empty()) {
            for paragraph in more {
                runs.push(Run::plain("\n"));
                runs.extend(paragraph.body.runs().iter().cloned());
            }
            *runs = merge(mem::take(runs));
            self.notes.push(ImportNote {
                block: out[from].id.to_string(),
                message: "Paragraphs of this list item were joined".into(),
            });
        }
        // The outermost item names what moved, so that nothing is named twice.
        if depth == 0 {
            let moved = out[from + 1..].iter().filter(|block| !matches!(block.kind, BlockKind::ListItem { .. }));
            let notes = moved.map(|block| ImportNote { block: block.id.to_string(), message: MOVED.into() });
            self.notes.extend(notes);
        }
    }

    /// Reads a quote or a callout, whose content is blocks of its own.
    fn container(&mut self, kind: BlockKind, element: &Element, style: &Style, out: &mut Vec<Block>) {
        let mut children = Vec::new();
        self.within(BlockKind::Paragraph, element, &Style { list: None, ..style.clone() }, &mut children);
        nest(&mut children);
        if !children.is_empty() {
            let mut block = self.block(kind);
            block.body = Body::Blocks(children);
            out.push(block);
        }
    }

    /// Reads a table. One that only lays a page out, with one column or in that role, is read as the blocks in
    /// its cells, and so is one inside a cell, which holds text alone.
    fn table(&mut self, element: &Element, style: &Style, out: &mut Vec<Block>) {
        /// A row's cells, and whether it sits in the table's head.
        type Row<'a> = (bool, Vec<&'a Element>);
        fn collect<'a>(element: &'a Element, head: bool, rows: &mut Vec<Row<'a>>, rest: &mut Vec<&'a Node>) {
            for node in &element.children {
                match node {
                    Node::Element(child) => match child.name.as_str() {
                        "td" | "th" => match rows.last_mut() {
                            Some((_, row)) => row.push(child),
                            None => rows.push((head, vec![child])),
                        },
                        "tr" => {
                            rows.push((head, Vec::new()));
                            collect(child, head, rows, rest);
                        }
                        "thead" | "tbody" | "tfoot" => collect(child, child.name == "thead", rows, rest),
                        _ => rest.push(node),
                    },
                    Node::Text(_) => rest.push(node),
                }
            }
        }
        let (mut rows, mut rest) = (Vec::new(), Vec::new());
        collect(element, false, &mut rows, &mut rest);
        if self.cell || element.attribute("role") == "presentation" || rows.iter().all(|(_, row)| row.len() < 2) {
            return self.nodes(&element.children, style, out);
        }
        // A caption reads as the paragraph above its table.
        for node in rest {
            match node {
                Node::Text(text) => self.text(text, style),
                Node::Element(other) => self.element(other, style, out),
            }
        }
        self.flush(out);

        let header = rows
            .first()
            .is_some_and(|(head, row)| *head || !row.is_empty() && row.iter().all(|cell| cell.name == "th"));
        let (notes, kind) = (self.notes.len(), mem::replace(&mut self.kind, BlockKind::Paragraph));
        self.cell = true;
        let mut columns: Vec<Option<Align>> = Vec::new();
        // How many more rows each column is covered for by a cell above.
        let mut covered: Vec<usize> = Vec::new();
        let mut lines = Vec::new();
        for (_, row) in rows {
            let mut cells = Vec::new();
            let mut row = row.into_iter();
            loop {
                let column = cells.len();
                if covered.get(column).is_some_and(|rows| *rows > 0) && self.spare > 0 {
                    (covered[column], self.spare) = (covered[column] - 1, self.spare - 1);
                    cells.push(self.block(BlockKind::TableCell));
                    continue;
                }
                let Some(element) = row.next() else { break };
                let (mut blocks, mut runs) = (Vec::new(), Vec::new());
                let inside = Style { list: None, ..style.inside(element) };
                self.within(BlockKind::Paragraph, element, &inside, &mut blocks);
                flatten(blocks, &mut runs);
                let mut cell = self.block(BlockKind::TableCell);
                cell.body = Body::Text(merge(runs));
                cells.push(cell);

                let across = element.span("colspan").min(self.spare + 1);
                self.spare -= across - 1;
                covered.resize(covered.len().max(column + across), 0);
                columns.resize(columns.len().max(column + across), None);
                // Word says how a cell is aligned on the paragraph in it.
                let align = || element.align().or_else(|| element.find(&|inner| inner.name == "p")?.align());
                columns[column] = columns[column].or_else(align);
                covered[column..column + across].fill(element.span("rowspan") - 1);
                cells.extend((1..across).map(|_| self.block(BlockKind::TableCell)));
            }
            covered.iter_mut().skip(cells.len()).for_each(|rows| *rows = rows.saturating_sub(1));
            let mut line = self.block(BlockKind::TableRow);
            line.body = Body::Blocks(cells);
            lines.push(line);
        }
        // The blocks read inside the cells are gone, and so is what was noted on them.
        self.notes.truncate(notes);
        (self.kind, self.cell) = (kind, false);
        if lines.iter().any(|line| line.body.blocks().iter().any(|cell| !cell.plain_text().trim().is_empty())) {
            let columns = columns.into_iter().map(|align| align.unwrap_or(Align::Start)).collect();
            let mut block = self.block(BlockKind::Table { columns, header });
            block.body = Body::Blocks(lines);
            out.push(block);
        }
    }
}

const MOVED: &str = "Moved out of a list item";
