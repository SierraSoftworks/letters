//! HTML in both directions: a document to one HTML file, and the clipboard's HTML to and from blocks.
//!
//! The file stands alone: the document's style is written as CSS in its head, code is highlighted with spans,
//! images are written into it, and it holds no script and loads nothing. It reads the same offline, as an
//! attachment or on a web server.

mod read;

use std::fmt::Write as _;

pub use read::from_html;

use letters_layout::{Palette, TokenKind as Ink};
use letters_model::{Align, Block, BlockKind, Document, ListKind, Mark, Run, Style, TextStyle, Tone};
use letters_render::{Color, Size};
use yewi_highlight::{TokenKind, tokenize};

/// Exports are drawn on the light page, whatever theme the editor is in. One page can add a dark theme to it.
const PAGE: Palette = Palette::LIGHT;

/// The element each mark is written as, outermost first.
const MARKS: [(Mark, &str); 6] = [
    (Mark::Bold, "strong"),
    (Mark::Italic, "em"),
    (Mark::Underline, "u"),
    (Mark::Strike, "s"),
    (Mark::Highlight, "mark"),
    (Mark::Code, "code"),
];

/// The colour maths is typeset in. The drawing leaves it to the text around it, so that a theme reaches it.
const INK: Color = PAGE.text;

/// Logical pixels in a point: 96 to the inch against 72.
const PIXELS: f32 = 96.0 / 72.0;

/// Reads an entry of the document's file by name, such as `assets/<hash>.png`. `None` when the file lacks it.
pub type Assets<'a> = &'a mut dyn FnMut(&str) -> Option<Vec<u8>>;

/// Writes a whole HTML file with `style` as its stylesheet. `title` names the page, as the file's name does
/// in the editor, and `assets` supplies the images to write into it.
///
/// - The style is used as written: each element type becomes a rule for its HTML element. Sizes are in
///   points. Line heights and gaps are in the whole pixels the editor rounds them to, so that lines fall
///   where they do there.
/// - An image held by the file is written in as a `data:` address. One that is still remote becomes a link
///   to it, so that opening the page fetches nothing.
/// - A link that would run script is written as its text alone.
/// - Maths is set and written in as a drawing, in the colour of the text around it. Its TeX source is kept on
///   the element, of the class `math`, where [`from_html`] reads it back. Maths that cannot be set is
///   written as its source.
/// - An unknown block is written as its fallback text. Unknown marks and fields are left out.
pub fn to_html(doc: &Document, style: &Style, title: &str, assets: Assets) -> String {
    to_html_with(doc, style, title, assets, &HtmlPage::default())
}

/// As [`to_html`], for a page that follows its reader's theme: light as `to_html` writes it, and in `dark`
/// where the reader asks for a dark page. Paper stays light.
///
/// The style's colours are shown on the dark page as layout shows them, through [`Palette::recolor`].
pub fn to_html_themed(doc: &Document, style: &Style, title: &str, assets: Assets, dark: &Palette) -> String {
    to_html_with(doc, style, title, assets, &HtmlPage { dark: Some(dark), ..HtmlPage::default() })
}

/// How a page differs from the file [`to_html`] writes. The default changes nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct HtmlPage<'a> {
    /// The colours of the page where its reader asks for a dark one. See [`to_html_themed`].
    pub dark: Option<&'a Palette>,
    /// The font files the page loads, for a server that holds the document's fonts. With none, the page
    /// loads nothing and is set in the reader's own copies of the fonts, or in what stands in for them.
    pub fonts: &'a [WebFont<'a>],
    /// Sets the document as the editor's window does, and not as a page of its own.
    pub window: Option<Column>,
    /// Rules written last in the stylesheet, where they win. For what only the caller knows of its page,
    /// such as a cursor. The caller vouches for them: they are written as they are.
    pub css: &'a str,
}

/// A font file for a page to load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebFont<'a> {
    /// The family, as the style names it.
    pub family: &'a str,
    /// Where the file is. The page may load fonts from its own origin alone.
    pub url: &'a str,
    /// The file holds the family's italic. Each file is taken to hold every weight.
    pub italic: bool,
}

/// How the editor's window sets a document, for a page that stands in for the window until it has painted:
/// what the page shows is where the window will draw it.
///
/// The text column is as wide as the style's page less its margins, or as the page less its gutters when
/// that is narrower, and starts on a whole pixel. A block the editor cannot show is drawn as the window
/// draws it, in a box that names its type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    /// The least room on each side of the column, in logical pixels.
    pub gutter: f32,
    /// The room above the document's first line and below its last.
    pub top: f32,
}

/// As [`to_html`], with the changes `page` asks for.
pub fn to_html_with(doc: &Document, style: &Style, title: &str, assets: Assets, page: &HtmlPage) -> String {
    let mut out = String::from("<!doctype html>\n<html lang=\"");
    escape(&doc.language, &mut out);
    out.push_str(concat!(
        "\">\n<head>\n<meta charset=\"utf-8\">\n",
        // Nothing in the file may run or load, whatever a document manages to carry into it.
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; ",
    ));
    // But the fonts its own server holds, where the caller names some.
    out.push_str(if page.fonts.is_empty() { "" } else { "font-src 'self'; " });
    out.push_str(concat!(
        "style-src 'unsafe-inline'; img-src data:\">\n",
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
        // Says which themes the page has, so the browser's own parts follow: the canvas, scroll bars, tick boxes.
        "<meta name=\"color-scheme\" content=\"",
    ));
    out.push_str(if page.dark.is_some() { "light dark" } else { "light" });
    out.push_str("\">\n<meta name=\"generator\" content=\"Letters\">\n<title>");
    escape(title, &mut out);
    out.push_str("</title>\n<style>\n");
    css(style, page, &mut out);
    out.push_str("</style>\n</head>\n<body>\n<article>\n");
    let size = |key: &str| {
        let points = style.types.get(key).or_else(|| style.types.get("body")).map_or(0.0, |of| of.size);
        if points.is_finite() && points > 0.0 { points * PIXELS } else { TextStyle::default().size * PIXELS }
    };
    let sizes = [size("body"), size("heading1"), size("heading2"), size("heading3"), size("heading4")];
    blocks(&doc.blocks, &mut Writing { assets, maths: Some(sizes), window: page.window.is_some() }, &mut out);
    out.push_str("</article>\n</body>\n</html>\n");
    out
}

/// Writes `blocks` as the elements a page's body would hold, with no page around them: what a copy offers to
/// other applications. Nothing is styled, and no image is written in. Maths is written as its TeX source
/// between `\(` and `\)`, or `\[` and `\]` for a formula, for whatever takes the paste to set.
pub fn to_html_fragment(blocks: &[Block]) -> String {
    let mut out = String::new();
    self::blocks(blocks, &mut Writing { assets: &mut |_| None, maths: None, window: false }, &mut out);
    out
}

/// What blocks are written for.
struct Writing<'a> {
    assets: Assets<'a>,
    /// On a page that draws its maths, the size of its text in logical pixels: the body's, then each level
    /// of heading's. Maths is typeset at the size it is shown, so that its hairlines come out as thin as
    /// they should. `None` writes maths as its source.
    maths: Option<[f32; 5]>,
    /// Blocks are drawn as the editor's window draws them, where that is not how a page of its own reads best.
    window: bool,
}

impl Writing<'_> {
    /// The size maths is typeset at in a heading of `level`, or in other text for level 0.
    fn text(&self, level: u8) -> Option<f32> {
        self.maths.map(|sizes| sizes[usize::from(level.min(4))])
    }
}

/// Writes text so that HTML reads it as text, between tags or inside a quoted attribute.
fn escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

/// Writes sibling blocks, one to a line. Consecutive list items become nested lists.
fn blocks(blocks: &[Block], page: &mut Writing, out: &mut String) {
    // The lists that are open, outermost first, each with its latest item still open.
    let mut lists: Vec<&str> = Vec::new();
    for block in blocks {
        if let BlockKind::ListItem { list, depth } = block.kind {
            item(&mut lists, list, depth, block.body.runs(), page.text(0), out);
            continue;
        }
        close(&mut lists, 0, out);
        match &block.kind {
            BlockKind::Paragraph => element("p", block.body.runs(), page.text(0), out),
            BlockKind::Heading { level } => {
                let level = (*level).clamp(1, 4);
                element(&format!("h{level}"), block.body.runs(), page.text(level), out);
            }
            BlockKind::Code { language } => code(language.as_deref(), &block.plain_text(), out),
            BlockKind::Formula => {
                maths(&block.plain_text(), true, page.text(0), out);
                out.push('\n');
            }
            BlockKind::Quote => container("<blockquote>", "</blockquote>", block.body.blocks(), page, out),
            BlockKind::Callout { tone } => {
                let open = format!("<div class=\"callout {}\" role=\"note\">", tone_name(*tone));
                container(&open, "</div>", block.body.blocks(), page, out);
            }
            BlockKind::Divider => out.push_str("<hr>\n"),
            BlockKind::Figure { asset, src, alt, width, height, .. } => {
                let image =
                    Figure { asset: asset.as_deref(), src: src.as_deref(), alt, width: *width, height: *height };
                figure(&image, block.body.runs(), page, out);
            }
            BlockKind::Table { columns, header } => table(columns, *header, block.body.blocks(), page, out),
            // A merge can leave a row outside a table and a cell outside a row.
            BlockKind::TableRow => table(&[], false, std::slice::from_ref(block), page, out),
            // Until galleries are written as such, what they hold is written in order.
            BlockKind::Caption | BlockKind::TableCell => element("p", block.body.runs(), page.text(0), out),
            BlockKind::Gallery { .. } => self::blocks(block.body.blocks(), page, out),
            // A reader that does not build figures yet hands one over as an unknown block, its fields beside it.
            BlockKind::Unknown { type_name } if type_name == "figure" => {
                let field = |name: &str| block.extra.get(name);
                let text = |name: &str| field(name).and_then(|value| value.as_str());
                let size = |name: &str| field(name).and_then(|value| value.as_f64()).unwrap_or_default() as f32;
                let caption = field("content").and_then(|runs| Run::list_from_value(runs.clone(), "content").ok());
                let image = Figure {
                    asset: text("asset"),
                    src: text("src"),
                    alt: text("alt").unwrap_or_default(),
                    width: size("width"),
                    height: size("height"),
                };
                figure(&image, &caption.unwrap_or_default(), page, out);
            }
            BlockKind::Unknown { type_name } if page.window => {
                out.push_str("<p class=\"unknown\">Unsupported block: ");
                escape(type_name, out);
                out.push_str("</p>\n");
            }
            BlockKind::Unknown { .. } => {
                let fallback = block.extra.get("fallback").and_then(|fallback| fallback.as_str()).unwrap_or_default();
                if !fallback.is_empty() {
                    out.push_str("<p class=\"fallback\">");
                    text(fallback, out);
                    out.push_str("</p>\n");
                }
            }
            // Written by `item`, which knows the list around it.
            BlockKind::ListItem { .. } => {}
        }
    }
    close(&mut lists, 0, out);
}

fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Tip => "tip",
        Tone::Info => "info",
        Tone::Warn => "warn",
        Tone::Danger => "danger",
    }
}

/// Writes a text block. One with no text keeps its line, as it does on the page.
fn element(tag: &str, runs: &[Run], maths: Option<f32>, out: &mut String) {
    let _ = write!(out, "<{tag}>");
    if runs.iter().all(|run| run.text.is_empty()) {
        out.push_str("<br>");
    } else {
        inline(runs, maths, out);
    }
    let _ = writeln!(out, "</{tag}>");
}

fn container(open: &str, end: &str, children: &[Block], page: &mut Writing, out: &mut String) {
    let _ = writeln!(out, "{open}");
    let start = out.len();
    blocks(children, page, out);
    if out.len() == start {
        out.push_str("<p><br></p>\n");
    }
    let _ = writeln!(out, "{end}");
}

/// Writes a table, its first row in a head where that row heads it. A cell says how its column is aligned
/// where that is not to the start. What the table holds that is not a row is written after it.
fn table(columns: &[Align], header: bool, children: &[Block], page: &mut Writing, out: &mut String) {
    let is_row = |child: &&Block| child.kind == BlockKind::TableRow;
    let rows: Vec<&Block> = children.iter().filter(is_row).collect();
    if !rows.is_empty() {
        out.push_str("<table>\n");
        let (head, body) = rows.split_at(usize::from(header));
        for (section, cell, rows) in [("thead", "th", head), ("tbody", "td", body)] {
            if rows.is_empty() {
                continue;
            }
            let _ = writeln!(out, "<{section}>");
            for row in rows {
                out.push_str("<tr>");
                for (column, block) in row.body.blocks().iter().enumerate() {
                    let align = match columns.get(column) {
                        Some(Align::Center) => " style=\"text-align: center\"",
                        Some(Align::End) => " style=\"text-align: end\"",
                        Some(Align::Justify) => " style=\"text-align: justify\"",
                        Some(Align::Start) | None => "",
                    };
                    let _ = write!(out, "<{cell}{align}>");
                    inline(block.body.runs(), page.text(0), out);
                    let _ = write!(out, "</{cell}>");
                }
                out.push_str("</tr>\n");
            }
            let _ = writeln!(out, "</{section}>");
        }
        out.push_str("</table>\n");
    }
    for other in children.iter().filter(|child| !is_row(child)) {
        blocks(std::slice::from_ref(other), page, out);
    }
}

/// Closes the innermost lists until `keep` are open.
fn close(lists: &mut Vec<&str>, keep: usize, out: &mut String) {
    while lists.len() > keep {
        let tag = lists.pop().unwrap_or_default();
        let _ = writeln!(out, "</li>\n</{tag}>");
    }
}

/// Writes a list item, opening and closing lists around it to reach its depth.
fn item(lists: &mut Vec<&'static str>, list: ListKind, depth: u8, runs: &[Run], maths: Option<f32>, out: &mut String) {
    let tag = if list == ListKind::Ordered { "ol" } else { "ul" };
    // An item nests inside the one before it, so it can sit at most one level deeper.
    let depth = usize::from(depth).min(lists.len());
    close(lists, depth + 1, out);
    if lists.len() == depth + 1 {
        out.push_str("</li>\n");
        // A change between bullets and numbers starts a new list beside the last one.
        if lists[depth] != tag {
            let _ = writeln!(out, "</{}>", lists.pop().unwrap_or_default());
        }
    } else if depth > 0 {
        out.push('\n');
    }
    if lists.len() == depth {
        let _ = writeln!(out, "<{tag}>");
        lists.push(tag);
    }
    out.push_str(match list {
        ListKind::Task { checked: true } => "<li class=\"task\"><input type=\"checkbox\" disabled checked>",
        ListKind::Task { checked: false } => "<li class=\"task\"><input type=\"checkbox\" disabled>",
        ListKind::Bullet | ListKind::Ordered => "<li>",
    });
    inline(runs, maths, out);
}

/// Writes a code block, with a span around each token the highlighter colours.
fn code(language: Option<&str>, source: &str, out: &mut String) {
    let name = lexer_name(language);
    out.push_str("<pre><code");
    if !name.is_empty() {
        out.push_str(" class=\"language-");
        escape(name, out);
        out.push('"');
    }
    out.push('>');
    for token in tokenize(source, name) {
        match token.kind {
            Some(kind) => {
                let _ = write!(out, "<span class=\"tok-{}\">", kind.as_str());
                escape(token.text, out);
                out.push_str("</span>");
            }
            None => escape(token.text, out),
        }
    }
    // A line break keeps an empty block one line tall.
    out.push_str(if source.is_empty() { "\n</code></pre>\n" } else { "</code></pre>\n" });
}

/// An image and where it comes from: an entry of the document's file, or an address it is still at.
struct Figure<'a> {
    asset: Option<&'a str>,
    src: Option<&'a str>,
    alt: &'a str,
    /// The size it is shown at, in points.
    width: f32,
    height: f32,
}

/// Writes a figure and its caption.
///
/// An image the file holds is written into the page. One that is still remote is not: the page must fetch
/// nothing, so it becomes a link. One that cannot be found leaves its description in its place.
fn figure(image: &Figure, caption: &[Run], page: &mut Writing, out: &mut String) {
    out.push_str("<figure>\n");
    let held = image.asset.and_then(|name| {
        let bytes = (page.assets)(name)?;
        Some((media_type(name, &bytes)?, bytes))
    });
    let remote = image.src.and_then(href);
    if let Some((media, bytes)) = held {
        let _ = write!(out, "<img src=\"data:{media};base64,{}\" alt=\"", base64(&bytes));
        escape(image.alt, out);
        // In CSS pixels, so the page keeps the image's place while it decodes.
        let pixels = |points: f32| (points * PIXELS).round().max(1.0);
        let _ = writeln!(out, "\" width=\"{}\" height=\"{}\">", pixels(image.width), pixels(image.height));
    } else if let Some(address) = remote {
        out.push_str("<p><a href=\"");
        escape(&address, out);
        out.push_str("\">");
        escape(if image.alt.is_empty() { &address } else { image.alt }, out);
        out.push_str("</a></p>\n");
    } else if !image.alt.is_empty() {
        out.push_str("<p class=\"fallback\">");
        escape(image.alt, out);
        out.push_str("</p>\n");
    }
    if caption.iter().any(|run| !run.text.is_empty()) {
        out.push_str("<figcaption>");
        inline(caption, page.text(0), out);
        out.push_str("</figcaption>\n");
    }
    out.push_str("</figure>\n");
}

/// The media type of an image a browser can show, told from its first bytes. SVG is text, so its name tells.
fn media_type(name: &str, bytes: &[u8]) -> Option<&'static str> {
    let svg = name.rsplit_once('.').is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("svg"));
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ if svg => Some("image/svg+xml"),
        _ => None,
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().fold(0u32, |group, byte| group << 8 | u32::from(*byte)) << (8 * (3 - chunk.len()));
        for index in 0..4 {
            let digit = ALPHABET[(group >> (18 - 6 * index) & 63) as usize] as char;
            out.push(if index <= chunk.len() { digit } else { '=' });
        }
    }
    out
}

/// Writes maths: a drawing of it on a page that sets maths at `text`, and elsewhere its source between the
/// delimiters MathJax and Pandoc use.
///
/// The drawing is sized in em, so it follows the text it sits in, and stands on that text's baseline. The
/// source is kept beside it twice over: for `from_html`, and as what a screen reader says.
fn maths(tex: &str, display: bool, text: Option<f32>, out: &mut String) {
    let (tag, class, open, close) =
        if display { ("p", "display", r"\[", r"\]") } else { ("span", "inline", r"\(", r"\)") };
    let set = text.and_then(|text| {
        let style = letters_math::Style { size: text * letters_math::TEXT_RATIO, color: INK, display, links: false };
        let formula = letters_math::typeset(tex, &style).ok()?;
        (formula.size.width > 0.0 && formula.size.height > 0.0).then_some((formula, text))
    });
    let _ = write!(out, "<{tag} class=\"math {class}\"");
    if let Some((formula, text)) = set {
        let Size { width, height } = formula.size;
        let em = |pixels: f32| (pixels / text * 1000.0).round() / 1000.0;
        out.push_str(" data-tex=\"");
        escape(tex, out);
        let _ = write!(
            out,
            "\"><svg viewBox=\"0 0 {width} {height}\" width=\"{}em\" height=\"{}em\" overflow=\"visible\" \
             style=\"vertical-align: {}em\" role=\"img\" aria-label=\"",
            em(width),
            em(height),
            em(formula.baseline - height),
        );
        escape(tex, out);
        out.push_str("\">");
        out.push_str(&formula.svg(INK));
        out.push_str("</svg>");
    } else {
        out.push('>');
        out.push_str(open);
        escape(tex, out);
        out.push_str(close);
    }
    let _ = write!(out, "</{tag}>");
}

/// Writes a block's runs. Neighbouring runs of one link share its anchor.
fn inline(runs: &[Run], maths: Option<f32>, out: &mut String) {
    let mut rest = runs;
    while let Some(first) = rest.first() {
        let (linked, after) = rest.split_at(rest.iter().take_while(|run| run.link == first.link).count());
        let address = first.link.as_deref().and_then(href);
        if let Some(address) = &address {
            out.push_str("<a href=\"");
            escape(address, out);
            out.push_str("\">");
        }
        for run in linked.iter().filter(|run| !run.text.is_empty()) {
            let tags = || MARKS.iter().filter(|(mark, _)| run.marks.contains(mark)).map(|(_, tag)| *tag);
            tags().for_each(|tag| _ = write!(out, "<{tag}>"));
            if run.marks.contains(&Mark::Math) {
                self::maths(&run.text, false, maths, out);
            } else {
                text(&run.text, out);
            }
            tags().rev().for_each(|tag| _ = write!(out, "</{tag}>"));
        }
        if address.is_some() {
            out.push_str("</a>");
        }
        rest = after;
    }
}

/// Writes text that may hold line breaks.
fn text(text: &str, out: &mut String) {
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push_str("<br>\n");
        }
        escape(line, out);
    }
}

/// The name to look a code block's lexer up by: the first word of its language. A fence may carry more after it.
pub(crate) fn lexer_name(language: Option<&str>) -> &str {
    language.and_then(|language| language.split_whitespace().next()).unwrap_or_default()
}

/// A link's address as a browser will read it, or `None` for one that would run script when followed.
pub(crate) fn href(url: &str) -> Option<String> {
    // A browser drops these before it looks for the scheme, so they cannot be left to hide one.
    let url: String = url.chars().filter(|c| !c.is_control()).collect();
    let url = url.trim();
    let scheme = url.split_once(':').map(|(scheme, _)| scheme).filter(|scheme| !scheme.contains(['/', '?', '#']));
    let runs = scheme
        .is_some_and(|scheme| ["javascript", "vbscript", "data"].iter().any(|bad| scheme.eq_ignore_ascii_case(bad)));
    (!url.is_empty() && !runs).then(|| url.to_owned())
}

/// The kind layout colours a token as, so the page and the export agree.
pub(crate) fn ink(kind: TokenKind) -> Ink {
    match kind {
        TokenKind::Comment => Ink::Comment,
        TokenKind::Keyword => Ink::Keyword,
        TokenKind::Literal => Ink::Literal,
        TokenKind::Number => Ink::Number,
        TokenKind::String => Ink::String,
        TokenKind::Type => Ink::Type,
        TokenKind::Function => Ink::Function,
        TokenKind::Property => Ink::Property,
        TokenKind::Tag => Ink::Tag,
        TokenKind::Variable => Ink::Variable,
        TokenKind::Meta => Ink::Meta,
        TokenKind::Inserted => Ink::Inserted,
        TokenKind::Deleted => Ink::Deleted,
    }
}

/// A length in points, to two places.
fn pt(points: f32) -> String {
    let rounded = if points.is_finite() { (points * 100.0).round() / 100.0 } else { 0.0 };
    if rounded == 0.0 { "0".to_owned() } else { format!("{rounded}pt") }
}

/// A number as CSS takes it, to two places.
fn number(value: f32) -> f32 {
    if value.is_finite() { (value * 100.0).round() / 100.0 } else { 0.0 }
}

/// A length in whole logical pixels, which is how layout measures the height of a line and the gaps between
/// blocks. Written the same, the page puts each line where the editor does.
fn px(pixels: f32) -> String {
    let whole = if pixels.is_finite() { pixels.round() } else { 0.0 };
    if whole == 0.0 { "0".to_owned() } else { format!("{whole}px") }
}

fn color(color: Color) -> String {
    let Color { r, g, b, a } = color;
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// A CSS string. Nothing in it can end the string, or the style element around it.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars().filter(|c| !c.is_control()) {
        match c {
            '"' | '\\' | '<' | '>' | '&' => _ = write!(out, "\\{:x} ", u32::from(c)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Writes how an element type's text is set, but for its colour, which [`colours`] writes.
fn type_rule(selector: &str, of: &TextStyle, mono: bool, css: &mut String) {
    let generic = if mono { "ui-monospace, monospace" } else { "system-ui, sans-serif" };
    let align = match of.align {
        Align::Start => "start",
        Align::Center => "center",
        Align::End => "end",
        Align::Justify => "justify",
    };
    let _ = writeln!(
        css,
        "{selector} {{\n  font-family: {family}, {generic};\n  font-size: {size};\n  font-weight: {weight};\n  \
         font-style: {slant};\n  line-height: {line};\n  text-align: {align};\n}}",
        family = quoted(&of.family),
        size = pt(of.size),
        weight = of.weight,
        slant = if of.italic { "italic" } else { "normal" },
        line = px((of.size * PIXELS * of.line_height).max(1.0)),
    );
}

/// Writes the space above and below an element type.
fn space_rule(selector: &str, before: f32, after: f32, css: &mut String) {
    let _ = writeln!(css, "{selector} {{ margin: {} 0 {}; }}", px(before * PIXELS), px(after * PIXELS));
}

/// The element types that set text, each with the elements written in it.
const TYPES: [(&str, &str); 10] = [
    ("body", "p, li"),
    ("body", "th, td"),
    ("heading1", "h1"),
    ("heading2", "h2"),
    ("heading3", "h3"),
    ("heading4", "h4"),
    ("quote", "blockquote p, blockquote li"),
    ("code", "pre"),
    ("caption", "figcaption, .fallback, .unknown"),
    ("title", ".title"),
];

/// Writes every colour of the page as `on` has it, and nothing else, so that a second palette can follow the
/// first and change the colours alone.
fn colours(style: &Style, on: &Palette, css: &mut String) {
    let _ = writeln!(css, "html {{ background: {}; }}", color(on.page));
    for (key, selector) in TYPES {
        let of = style.types.get(key).or_else(|| style.types.get("body"));
        let digits = of.and_then(|of| of.color.strip_prefix('#')).filter(|digits| digits.len() == 6);
        // A colour that cannot be read falls back to the page's text colour, as it does on the page.
        let ink = match digits.map(|digits| u32::from_str_radix(digits, 16)) {
            Some(Ok(rgb)) => on.recolor(Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)),
            _ => on.text,
        };
        let _ = writeln!(css, "{selector} {{ color: {}; }}", color(ink));
    }
    let _ = write!(
        css,
        ":not(pre) > code {{ background: {inline_code}; }}\n\
         pre {{ background: {code_fill}; }}\n\
         li::before, .unknown {{ color: {marker}; }}\n\
         li.task > input {{ border-color: {line_strong}; }}\n\
         li.task > input:checked {{ border-color: {accent}; background: {accent} {tick}; }}\n\
         blockquote, .unknown {{ border-color: {line_strong}; }}\n\
         hr, th, td {{ border-color: {line}; }}\n\
         a {{ color: {link}; }}\n\
         mark {{ background: {highlight}; }}\n",
        inline_code = color(on.inline_code),
        code_fill = color(on.code),
        marker = color(on.marker),
        accent = color(on.accent),
        // A tick, drawn in the page itself. The `#` of its colour is written so that it does not end the address.
        tick = format_args!(
            "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 14 14'%3E%3Cpath \
             d='M3.4 7.4l2.4 2.4 4.8-5.4' fill='none' stroke='%23{}' stroke-width='1.6' stroke-linecap='round' \
             stroke-linejoin='round'/%3E%3C/svg%3E\")",
            &color(on.on_accent)[1..],
        ),
        line_strong = color(on.line_strong),
        line = color(on.line),
        link = color(on.link),
        highlight = color(on.highlight),
    );
    for (tone, look) in [(Tone::Tip, on.tip), (Tone::Info, on.info), (Tone::Warn, on.warn), (Tone::Danger, on.danger)] {
        let (name, accent, fill) = (tone_name(tone), color(look.accent), color(look.fill));
        let _ = writeln!(css, ".callout.{name} {{ border-color: {accent}; background: {fill}; }}");
    }
    for kind in TokenKind::ALL {
        let _ = writeln!(css, ".tok-{} {{ color: {}; }}", kind.as_str(), color(on.syntax.color(ink(kind))));
    }
}

/// Writes what stands before each list item, where layout draws it: ending 8 pixels before the text, a
/// bullet or a box level with the middle of a lower-case letter. A browser's own markers sit a few pixels
/// off, and it greys a box that cannot be ticked.
fn markers(text: f32, css: &mut String) {
    let dot = (text * 0.36).round().max(3.0);
    // An empty marker, where `none` would have some screen readers stop calling the list a list.
    let _ = write!(
        css,
        "li {{ list-style-type: \"\"; }}\n\
         li::before {{ display: inline-block; box-sizing: border-box; margin-right: 8px; }}\n\
         ul > li::before {{ content: \"\"; width: {dot}px; height: {dot}px; margin-left: -{reach}px; \
         vertical-align: calc(0.34em - {half}px); border-radius: 50%; background: currentColor; }}\n",
        reach = dot + 8.0,
        half = dot / 2.0,
    );
    // A list inside a list changes its bullet: a ring, a square, then round again.
    for depth in 1..=8 {
        let shape = match depth % 3 {
            0 => "border: 0; border-radius: 50%; background: currentColor;",
            1 => "border: 1px solid; border-radius: 50%; background: none;",
            _ => "border: 0; border-radius: 0; background: currentColor;",
        };
        let _ = writeln!(css, "{}ul > li::before {{ {shape} }}", "li ".repeat(depth));
    }
    css.push_str(concat!(
        "ol { counter-reset: item; }\n",
        "ol > li { counter-increment: item; }\n",
        "ol > li::before { content: counter(item) \".\"; width: 4em; margin-left: calc(-4em - 8px); ",
        "text-align: right; }\n",
        "li.task::before { content: none; }\n",
        "li.task > input { appearance: none; -webkit-appearance: none; box-sizing: border-box; width: 14px; ",
        "height: 14px; margin: 0 8px 0 -22px; vertical-align: calc(0.34em - 7px); border: 1.5px solid; ",
        "border-radius: 3px; }\n",
    ));
}

/// Writes the stylesheet: the page, each element type of the style, the furniture layout draws around them,
/// then the colours of it all. A page that has a dark theme repeats the colours in `dark`.
fn css(style: &Style, with: &HtmlPage, css: &mut String) {
    let of = |key: &str| style.types.get(key).or_else(|| style.types.get("body")).cloned().unwrap_or_default();
    let (body, quote, code) = (of("body"), of("quote"), of("code"));
    // The space after body text is the unit the gaps around lists, quotes and dividers are measured in.
    let unit = (body.space_after * PIXELS).round();
    let page = &style.page;

    for font in with.fonts {
        let _ = writeln!(
            css,
            "@font-face {{ font-family: {}; src: url({}); font-weight: 1 1000; font-style: {}; font-display: swap; }}",
            quoted(font.family),
            quoted(font.url),
            if font.italic { "italic" } else { "normal" },
        );
    }
    // Paper keeps the fills behind code and callouts, which a browser would otherwise leave off it.
    css.push_str(concat!(
        "body { margin: 0; -webkit-text-size-adjust: 100%; text-size-adjust: 100%; ",
        "-webkit-print-color-adjust: exact; print-color-adjust: exact; }\n",
    ));
    let measure = page.width - page.margin_left - page.margin_right;
    let _ = match with.window {
        // The window centres its column and starts it on a whole pixel, which `margin: auto` would not.
        Some(Column { gutter, top }) => writeln!(
            css,
            "article {{ --column: min({column}px, 100% - {gutters}px); width: var(--column); padding: {top} 0; \
             margin-left: round(calc((100% - var(--column)) / 2), 1px); overflow-wrap: break-word; }}",
            column = number(measure * PIXELS),
            gutters = number(2.0 * gutter),
            top = px(top),
        ),
        None => writeln!(
            css,
            "article {{ max-width: {measure}; margin: 0 auto; padding: {top} {right} {bottom} {left}; \
             overflow-wrap: break-word; }}",
            measure = pt(measure),
            top = pt(page.margin_top),
            right = pt(page.margin_right),
            bottom = pt(page.margin_bottom),
            left = pt(page.margin_left),
        ),
    };

    type_rule("p, li", &body, false, css);
    type_rule("th, td", &body, false, css);
    space_rule("p", body.space_before, body.space_after, css);
    for level in 1..=4 {
        let (heading, selector) = (of(&format!("heading{level}")), format!("h{level}"));
        type_rule(&selector, &heading, false, css);
        space_rule(&selector, heading.space_before, heading.space_after, css);
    }
    type_rule("blockquote p, blockquote li", &quote, false, css);
    space_rule("blockquote p", quote.space_before, quote.space_after, css);
    type_rule("pre", &code, true, css);
    space_rule("pre", code.space_before.max(code.space_after), code.space_after, css);
    for (key, selector) in [("caption", "figcaption, .fallback, .unknown"), ("title", ".title")] {
        type_rule(selector, &of(key), false, css);
        space_rule(selector, of(key).space_before, of(key).space_after, css);
    }
    // A block the window cannot show stands in a box, with the room around it that a code block has.
    let label = of("caption");
    space_rule(".unknown", label.space_before.max(label.space_after), label.space_after, css);

    // The measures in pixels are layout's own, so the file looks as the page does.
    let _ = write!(
        css,
        "code {{ font-family: {mono}, ui-monospace, monospace; }}\n\
         :not(pre) > code {{ font-size: 0.9em; }}\n\
         :not(pre) > code, mark {{ padding: 0 2px; margin: 0 -2px; border-radius: 3px; }}\n\
         .unknown {{ padding: 11px 15px; border: 1px solid; border-radius: 6px; }}\n\
         pre {{ padding: 12px 16px; border-radius: 6px; white-space: pre-wrap; \
         overflow-wrap: anywhere; tab-size: 4; }}\n\
         ul, ol {{ margin: {before} 0 {after}; padding-left: 24px; }}\n\
         li + li, li > ul, li > ol {{ margin-top: {between}; }}\n\
         li > ul, li > ol {{ margin-bottom: 0; }}\n\
         :is(ul, ol) + :is(ul, ol) {{ margin-top: {beside}; }}\n\
         blockquote {{ margin: {after} 0; padding: 0 0 0 16px; border-left: 3px solid; }}\n\
         .callout {{ margin: {callout_gap} 0; padding: 12px 16px 12px 13px; border-left: 3px solid; \
         border-radius: 6px; }}\n",
        mono = quoted(&code.family),
        before = px(body.space_before * PIXELS),
        after = px(unit),
        between = px(unit * 0.25),
        // Bullets after numbers are two lists here and one run of items to layout, which sets them as close
        // as the items of one list. The margin is taken off the gap that the list above leaves.
        beside = px((unit * 0.25).round() - unit.max((body.space_before * PIXELS).round())),
        callout_gap = px(unit * 1.5),
    );
    markers(body.size * PIXELS, css);
    let _ = write!(
        css,
        "table {{ margin: {after} 0; border-collapse: collapse; }}\n\
         th, td {{ padding: 4px 8px; border: 1px solid; vertical-align: top; }}\n\
         th {{ font-weight: 600; }}\n\
         figure {{ margin: {after} 0; }}\n\
         figure > * {{ margin: 0; }}\n\
         figure > figcaption {{ margin-top: {between}; }}\n\
         img {{ display: block; max-width: 100%; height: auto; }}\n\
         .math.display > svg {{ display: block; margin: 0 auto; max-width: 100%; height: auto; }}\n\
         article > :first-child, blockquote > :first-child, .callout > :first-child {{ margin-top: 0; }}\n\
         article > :last-child, blockquote > :last-child, .callout > :last-child {{ margin-bottom: 0; }}\n\
         hr {{ margin: {rule_gap} 0; border: 0; border-top: 1px solid; }}\n\
         mark {{ color: inherit; }}\n\
         .tok-comment {{ font-style: italic; }}\n",
        after = px(unit),
        between = px(unit * 0.5),
        rule_gap = px(unit * 2.0),
    );
    colours(style, &PAGE, css);
    if let Some(dark) = with.dark {
        // Only a screen goes dark: paper is white whatever the reader's theme.
        css.push_str("@media screen and (prefers-color-scheme: dark) {\n");
        colours(style, dark, css);
        css.push_str("}\n");
    }
    // A narrow screen gives the margins back to the text. A window has gutters for that.
    if with.window.is_none() {
        css.push_str("@media (max-width: 40em) { article { padding: 24px 16px; } }\n");
    }
    // Paper takes its margins from the page setup.
    let _ = write!(
        css,
        "@page {{ size: {width} {height}; margin: {top} {right} {bottom} {left}; }}\n\
         @media print {{ article {{ width: auto; max-width: none; margin: 0; padding: 0; }} }}\n",
        width = pt(page.width),
        height = pt(page.height),
        top = pt(page.margin_top),
        right = pt(page.margin_right),
        bottom = pt(page.margin_bottom),
        left = pt(page.margin_left),
    );
    // Nothing in the caller's rules can end the stylesheet: CSS reads the escape as the same character.
    css.push_str(&with.css.replace("</", "<\\/"));
    if !with.css.is_empty() && !with.css.ends_with('\n') {
        css.push('\n');
    }
}
