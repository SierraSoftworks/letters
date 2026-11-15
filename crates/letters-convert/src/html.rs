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
use letters_render::Color;
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

/// Reads an entry of the document's file by name, such as `assets/<hash>.png`. `None` when the file lacks it.
pub type Assets<'a> = &'a mut dyn FnMut(&str) -> Option<Vec<u8>>;

/// Writes a whole HTML file with `style` as its stylesheet. `title` names the page, as the file's name does
/// in the editor, and `assets` supplies the images to write into it.
///
/// - The style is used as written: each element type becomes a rule for its HTML element, in points.
/// - An image held by the file is written in as a `data:` address. One that is still remote becomes a link
///   to it, so that opening the page fetches nothing.
/// - A link that would run script is written as its text alone.
/// - Maths is written as its TeX source between `\(` and `\)`, or `\[` and `\]` for a formula, in an element
///   of the class `math`. Nothing in the file sets it, since nothing in the file runs.
/// - An unknown block is written as its fallback text. Unknown marks and fields are left out.
pub fn to_html(doc: &Document, style: &Style, title: &str, assets: Assets) -> String {
    page(doc, style, title, assets, None)
}

/// As [`to_html`], for a page that follows its reader's theme: light as `to_html` writes it, and in `dark`
/// where the reader asks for a dark page. Paper stays light.
///
/// The style's colours are shown on the dark page as layout shows them, through [`Palette::recolor`].
pub fn to_html_themed(doc: &Document, style: &Style, title: &str, assets: Assets, dark: &Palette) -> String {
    page(doc, style, title, assets, Some(dark))
}

fn page(doc: &Document, style: &Style, title: &str, assets: Assets, dark: Option<&Palette>) -> String {
    let mut out = String::from("<!doctype html>\n<html lang=\"");
    escape(&doc.language, &mut out);
    out.push_str(concat!(
        "\">\n<head>\n<meta charset=\"utf-8\">\n",
        // Nothing in the file may run or load, whatever a document manages to carry into it.
        "<meta http-equiv=\"Content-Security-Policy\" ",
        "content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:\">\n",
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
        // Says which themes the page has, so the browser's own parts follow: the canvas, scroll bars, tick boxes.
        "<meta name=\"color-scheme\" content=\"",
    ));
    out.push_str(if dark.is_some() { "light dark" } else { "light" });
    out.push_str("\">\n<meta name=\"generator\" content=\"Letters\">\n<title>");
    escape(title, &mut out);
    out.push_str("</title>\n<style>\n");
    css(style, dark, &mut out);
    out.push_str("</style>\n</head>\n<body>\n<article>\n");
    blocks(&doc.blocks, assets, &mut out);
    out.push_str("</article>\n</body>\n</html>\n");
    out
}

/// Writes `blocks` as the elements a page's body would hold, with no page around them: what a copy offers to
/// other applications. Nothing is styled, and no image is written in.
pub fn to_html_fragment(blocks: &[Block]) -> String {
    let mut out = String::new();
    self::blocks(blocks, &mut |_| None, &mut out);
    out
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
fn blocks(blocks: &[Block], assets: Assets, out: &mut String) {
    // The lists that are open, outermost first, each with its latest item still open.
    let mut lists: Vec<&str> = Vec::new();
    for block in blocks {
        if let BlockKind::ListItem { list, depth } = block.kind {
            item(&mut lists, list, depth, block.body.runs(), out);
            continue;
        }
        close(&mut lists, 0, out);
        match &block.kind {
            BlockKind::Paragraph => element("p", block.body.runs(), out),
            BlockKind::Heading { level } => element(&format!("h{}", (*level).clamp(1, 4)), block.body.runs(), out),
            BlockKind::Code { language } => code(language.as_deref(), &block.plain_text(), out),
            // Maths goes out as its source between the delimiters MathJax and Pandoc use, for a page or a paste
            // that can set it.
            BlockKind::Formula => {
                out.push_str("<p class=\"math display\">\\[");
                escape(&block.plain_text(), out);
                out.push_str("\\]</p>\n");
            }
            BlockKind::Quote => container("<blockquote>", "</blockquote>", block.body.blocks(), assets, out),
            BlockKind::Callout { tone } => {
                let open = format!("<div class=\"callout {}\" role=\"note\">", tone_name(*tone));
                container(&open, "</div>", block.body.blocks(), assets, out);
            }
            BlockKind::Divider => out.push_str("<hr>\n"),
            BlockKind::Figure { asset, src, alt, width, height, .. } => {
                let image =
                    Figure { asset: asset.as_deref(), src: src.as_deref(), alt, width: *width, height: *height };
                figure(&image, block.body.runs(), assets, out);
            }
            BlockKind::Table { columns, header } => table(columns, *header, block.body.blocks(), assets, out),
            // A merge can leave a row outside a table and a cell outside a row.
            BlockKind::TableRow => table(&[], false, std::slice::from_ref(block), assets, out),
            // Until galleries are written as such, what they hold is written in order.
            BlockKind::Caption | BlockKind::TableCell => element("p", block.body.runs(), out),
            BlockKind::Gallery { .. } => self::blocks(block.body.blocks(), assets, out),
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
                figure(&image, &caption.unwrap_or_default(), assets, out);
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
fn element(tag: &str, runs: &[Run], out: &mut String) {
    let _ = write!(out, "<{tag}>");
    if runs.iter().all(|run| run.text.is_empty()) {
        out.push_str("<br>");
    } else {
        inline(runs, out);
    }
    let _ = writeln!(out, "</{tag}>");
}

fn container(open: &str, end: &str, children: &[Block], assets: Assets, out: &mut String) {
    let _ = writeln!(out, "{open}");
    let start = out.len();
    blocks(children, assets, out);
    if out.len() == start {
        out.push_str("<p><br></p>\n");
    }
    let _ = writeln!(out, "{end}");
}

/// Writes a table, its first row in a head where that row heads it. A cell says how its column is aligned
/// where that is not to the start. What the table holds that is not a row is written after it.
fn table(columns: &[Align], header: bool, children: &[Block], assets: Assets, out: &mut String) {
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
                    inline(block.body.runs(), out);
                    let _ = write!(out, "</{cell}>");
                }
                out.push_str("</tr>\n");
            }
            let _ = writeln!(out, "</{section}>");
        }
        out.push_str("</table>\n");
    }
    for other in children.iter().filter(|child| !is_row(child)) {
        blocks(std::slice::from_ref(other), assets, out);
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
fn item(lists: &mut Vec<&'static str>, list: ListKind, depth: u8, runs: &[Run], out: &mut String) {
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
    inline(runs, out);
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
fn figure(image: &Figure, caption: &[Run], assets: Assets, out: &mut String) {
    out.push_str("<figure>\n");
    let held = image.asset.and_then(|name| {
        let bytes = assets(name)?;
        Some((media_type(name, &bytes)?, bytes))
    });
    let remote = image.src.and_then(href);
    if let Some((media, bytes)) = held {
        let _ = write!(out, "<img src=\"data:{media};base64,{}\" alt=\"", base64(&bytes));
        escape(image.alt, out);
        // In CSS pixels, so the page keeps the image's place while it decodes.
        let pixels = |points: f32| (points * 96.0 / 72.0).round().max(1.0);
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
        inline(caption, out);
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

/// Writes a block's runs. Neighbouring runs of one link share its anchor.
fn inline(runs: &[Run], out: &mut String) {
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
                out.push_str("<span class=\"math inline\">\\(");
                escape(&run.text, out);
                out.push_str("\\)</span>");
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
        line = if of.line_height.is_finite() { of.line_height } else { 1.5 },
    );
}

/// Writes the space above and below an element type.
fn space_rule(selector: &str, before: f32, after: f32, css: &mut String) {
    let _ = writeln!(css, "{selector} {{ margin: {} 0 {}; }}", pt(before), pt(after));
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
    ("caption", "figcaption, .fallback"),
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
         li::marker {{ color: {marker}; }}\n\
         li.task > input {{ accent-color: {accent}; }}\n\
         blockquote {{ border-color: {line_strong}; }}\n\
         hr, th, td {{ border-color: {line}; }}\n\
         a {{ color: {link}; }}\n\
         mark {{ background: {highlight}; }}\n",
        inline_code = color(on.inline_code),
        code_fill = color(on.code),
        marker = color(on.marker),
        accent = color(on.accent),
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

/// Writes the stylesheet: the page, each element type of the style, the furniture layout draws around them,
/// then the colours of it all. A page that has a dark theme repeats the colours in `dark`.
fn css(style: &Style, dark: Option<&Palette>, css: &mut String) {
    let of = |key: &str| style.types.get(key).or_else(|| style.types.get("body")).cloned().unwrap_or_default();
    let (body, quote, code) = (of("body"), of("quote"), of("code"));
    // The space after body text is the unit the gaps around lists, quotes and dividers are measured in.
    let unit = body.space_after;
    let page = &style.page;

    let _ = write!(
        css,
        "body {{ margin: 0; -webkit-text-size-adjust: 100%; text-size-adjust: 100%; }}\n\
         article {{ max-width: {measure}; margin: 0 auto; padding: {top} {right} {bottom} {left}; \
         overflow-wrap: break-word; }}\n",
        measure = pt(page.width - page.margin_left - page.margin_right),
        top = pt(page.margin_top),
        right = pt(page.margin_right),
        bottom = pt(page.margin_bottom),
        left = pt(page.margin_left),
    );

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
    for (key, selector) in [("caption", "figcaption, .fallback"), ("title", ".title")] {
        type_rule(selector, &of(key), false, css);
        space_rule(selector, of(key).space_before, of(key).space_after, css);
    }

    // The measures in pixels are layout's own, so the file looks as the page does.
    let _ = write!(
        css,
        "code {{ font-family: {mono}, ui-monospace, monospace; }}\n\
         :not(pre) > code {{ font-size: 0.9em; padding: 0.1em 0.25em; border-radius: 3px; }}\n\
         pre {{ padding: 12px 16px; border-radius: 6px; white-space: pre-wrap; \
         overflow-wrap: anywhere; tab-size: 4; }}\n\
         ul, ol {{ margin: {before} 0 {after}; padding-left: 24px; }}\n\
         li + li, li > ul, li > ol {{ margin-top: {between}; }}\n\
         li > ul, li > ol {{ margin-bottom: 0; }}\n\
         li.task {{ list-style: none; }}\n\
         li.task > input {{ width: 14px; height: 14px; margin: 0 8px 0 -22px; vertical-align: -2px; }}\n\
         blockquote {{ margin: {after} 0; padding: 0 0 0 16px; border-left: 3px solid; }}\n\
         .callout {{ margin: {callout_gap} 0; padding: 12px 16px; border-left: 3px solid; border-radius: 6px; }}\n",
        mono = quoted(&code.family),
        before = pt(body.space_before),
        after = pt(unit),
        between = pt(unit * 0.25),
        callout_gap = pt(unit * 1.5),
    );
    let _ = write!(
        css,
        "table {{ margin: {after} 0; border-collapse: collapse; }}\n\
         th, td {{ padding: 4px 8px; border: 1px solid; vertical-align: top; }}\n\
         th {{ font-weight: 600; }}\n\
         figure {{ margin: {after} 0; }}\n\
         figure > * {{ margin: 0; }}\n\
         figure > figcaption {{ margin-top: {between}; }}\n\
         img {{ display: block; max-width: 100%; height: auto; }}\n\
         article > :first-child, blockquote > :first-child, .callout > :first-child {{ margin-top: 0; }}\n\
         article > :last-child, blockquote > :last-child, .callout > :last-child {{ margin-bottom: 0; }}\n\
         hr {{ margin: {rule_gap} 0; border: 0; border-top: 1px solid; }}\n\
         mark {{ color: inherit; }}\n\
         .tok-comment {{ font-style: italic; }}\n",
        after = pt(unit),
        between = pt(unit * 0.5),
        rule_gap = pt(unit * 2.0),
    );
    colours(style, &PAGE, css);
    if let Some(dark) = dark {
        // Only a screen goes dark: paper is white whatever the reader's theme.
        css.push_str("@media screen and (prefers-color-scheme: dark) {\n");
        colours(style, dark, css);
        css.push_str("}\n");
    }
    // A narrow screen gives the margins back to the text, and paper takes them from the page setup.
    let _ = write!(
        css,
        "@media (max-width: 40em) {{ article {{ padding: 24px 16px; }} }}\n\
         @page {{ size: {width} {height}; margin: {top} {right} {bottom} {left}; }}\n\
         @media print {{ article {{ max-width: none; padding: 0; }} }}\n",
        width = pt(page.width),
        height = pt(page.height),
        top = pt(page.margin_top),
        right = pt(page.margin_right),
        bottom = pt(page.margin_bottom),
        left = pt(page.margin_left),
    );
}
