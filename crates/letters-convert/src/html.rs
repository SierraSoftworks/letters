//! HTML in both directions: a document to one HTML file, and the clipboard's HTML to and from blocks.
//!
//! The file stands alone: the document's style is written as CSS in its head, code is highlighted with spans,
//! images are written into it, and it holds no script and loads nothing. It reads the same offline, as an
//! attachment or on a web server.

mod read;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

pub use read::from_html;

use letters_layout::{Palette, TokenKind as Ink, figure_label, figure_numbers};
use letters_model::{Align, Block, BlockId, BlockKind, Document, ListKind, Mark, Run, Style, TextStyle, Tone};
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
/// - A numbered figure's caption opens with its label, and a reference to it reads as that label and leads to
///   the figure, which carries its block's id. A gallery is one figure of rows, tiled as the editor tiles them.
/// - Maths is set and written in as a drawing, in the colour of the text around it. Its TeX source is kept on
///   the element, of the class `math`, where [`from_html`] reads it back. Maths that cannot be set is
///   written as its source.
/// - A block or a passage in another language than the document's says so with `lang`, and a block that
///   starts a page with `data-page-break`, which the stylesheet turns into a break when the page is printed.
/// - Text that starts with a letter read from the right is set from the right, as on the page. A list item
///   that does says so with `dir`, and has its marker and its indent at the right, and so has a quote, of
///   the class `rtl`, whose first block does.
/// - A table of contents is a line for each heading it lists, which leads to the heading. A link preview
///   is a card that is one link, with its picture written in. Both are set as layout draws them.
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
    let figures = Some(Figures::of(doc, style));
    let contents = Some(Contents::of(doc));
    let mut writing = Writing { assets, maths: Some(sizes), window: page.window.is_some(), figures, contents };
    blocks(&doc.blocks, &mut writing, &mut out);
    out.push_str("</article>\n</body>\n</html>\n");
    out
}

/// Writes `blocks` as the elements a page's body would hold, with no page around them: what a copy offers to
/// other applications. Nothing is styled, no image is written in, and a reference to a figure is its text. Maths is written as its TeX source
/// between `\(` and `\)`, or `\[` and `\]` for a formula, for whatever takes the paste to set.
pub fn to_html_fragment(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut writing = Writing { assets: &mut |_| None, maths: None, window: false, figures: None, contents: None };
    self::blocks(blocks, &mut writing, &mut out);
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
    /// The document's numbered figures, on a page that spells their numbers out. `None` leaves a caption
    /// without a label and writes a reference as the text it carries.
    figures: Option<Figures>,
    /// The document's tables of contents, on a page whose lines can lead to their headings. `None` leaves a
    /// table out and writes a link preview as its link.
    contents: Option<Contents>,
}

impl Writing<'_> {
    /// What the text of a heading of `level` is written with, or other text for level 0.
    fn text(&self, level: u8) -> Line<'_> {
        let maths = self.maths.map(|sizes| sizes[usize::from(level.min(4))]);
        Line { maths, figures: self.figures.as_ref() }
    }
}

/// What a line of text is written with.
#[derive(Clone, Copy)]
struct Line<'a> {
    /// The size maths in the line is typeset at. `None` writes it as its source.
    maths: Option<f32>,
    figures: Option<&'a Figures>,
}

/// What the numbers of a document's figures read as. A document does not store them: they follow from the
/// order of its figures, so a page has to spell them out.
struct Figures {
    /// What a reference to each numbered figure reads as, such as "Figure 2". Its caption opens with the same
    /// and a colon.
    names: BTreeMap<BlockId, String>,
    /// What a reference reads as when its figure is gone or is no longer numbered.
    missing: String,
}

impl Figures {
    /// The figures of `doc`, numbered and named as layout does it, with the style's own word for a figure.
    fn of(doc: &Document, style: &Style) -> Self {
        let name = |number: u32| figure_label(style, number).trim_end_matches(':').to_owned();
        let any = name(0);
        let word = any.rsplit_once(' ').map_or(any.as_str(), |(word, _)| word);
        Self {
            missing: format!("{word} ?"),
            names: figure_numbers(doc).into_iter().map(|(id, number)| (id, name(number))).collect(),
        }
    }
}

/// What a document's tables of contents list. A document does not store their lines: they follow from its
/// headings, so a page has to spell them out.
#[derive(Default)]
struct Contents {
    /// The lines of each table.
    tables: BTreeMap<BlockId, Vec<Entry>>,
    /// The headings some table lists. Each carries its block's id, for a line to lead to.
    listed: BTreeSet<BlockId>,
}

/// A line of a table of contents.
struct Entry {
    heading: BlockId,
    level: u8,
    /// The heading's text, as a link to the heading.
    runs: Vec<Run>,
    /// Whether it reads from the right, and so is set in from there.
    rtl: bool,
}

impl Contents {
    fn of(doc: &Document) -> Self {
        let mut contents = Self::default();
        for table in doc.walk().filter(|block| matches!(block.kind, BlockKind::TableOfContents { .. })) {
            let line = |heading: &Block| match heading.kind {
                BlockKind::Heading { level } => {
                    let mut address = String::from("#");
                    fragment(&heading.id, &mut address);
                    // The whole line leads to the heading, so nothing in it leads anywhere of its own.
                    let leads = |run: &Run| Run { link: Some(address.clone()), ..run.clone() };
                    let runs = heading.body.runs().iter().map(leads).collect();
                    Some(Entry { heading: heading.id.clone(), level, runs, rtl: letters_layout::rtl(heading) })
                }
                _ => None,
            };
            let lines: Vec<_> = doc.contents(&table.id).into_iter().filter_map(line).collect();
            contents.listed.extend(lines.iter().map(|line| line.heading.clone()));
            contents.tables.insert(table.id.clone(), lines);
        }
        contents
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
    // Whether each open item reads from the right, outermost first.
    let mut sides: Vec<bool> = Vec::new();
    for block in blocks {
        if let BlockKind::ListItem { list, depth } = block.kind {
            let start = out.len();
            item(&mut lists, list, depth, block.body.runs(), page.text(0), out);
            // The item is the last one opened: text never holds a tag.
            let opened = out[start..].rfind("<li").map(|at| start + at);
            told(block, opened, page, out);
            // An item takes the side of the one it sits in, so it says its own only where they differ.
            sides.truncate(lists.len().saturating_sub(1));
            let (rtl, around) = (letters_layout::rtl(block), sides.last().copied().unwrap_or(false));
            if let (Some(opened), true) = (opened, rtl != around) {
                out.insert_str(opened + "<li".len(), if rtl { " dir=\"rtl\"" } else { " dir=\"ltr\"" });
            }
            sides.push(rtl);
            continue;
        }
        close(&mut lists, 0, out);
        let start = out.len();
        written(block, page, out);
        told(block, Some(start), page, out);
    }
    close(&mut lists, 0, out);
}

/// What a block says of itself that HTML has a word for and its element does not show: its language, where
/// that is not the document's, as ` lang="fr"`. Empty for a block that says none.
fn language(lang: Option<&str>) -> String {
    let mut said = String::new();
    if let Some(lang) = lang.filter(|lang| !lang.is_empty()) {
        said.push_str(" lang=\"");
        escape(lang, &mut said);
        said.push('"');
    }
    said
}

/// Says on the element written at `from` the block's language, that a page starts with it, which a browser
/// heeds when it prints, and the id of a heading that a table of contents leads to.
fn told(block: &Block, from: Option<usize>, page: &Writing, out: &mut String) {
    let mut said = language(block.lang.as_deref());
    if page.contents.as_ref().is_some_and(|contents| contents.listed.contains(&block.id)) {
        said.push_str(" id=\"");
        escape(block.id.as_str(), &mut said);
        said.push('"');
    }
    if block.page_break {
        said.push_str(" data-page-break");
    }
    // An attribute's value never holds this, so the first one ends the element's opening tag.
    let end = from.filter(|from| out[*from..].starts_with('<')).and_then(|from| Some(from + out[from..].find('>')?));
    if let (Some(end), false) = (end, said.is_empty()) {
        out.insert_str(end, &said);
    }
}

/// Writes one block that is not a list item.
fn written(block: &Block, page: &mut Writing, out: &mut String) {
    match &block.kind {
        BlockKind::Paragraph => element("p", block.body.runs(), page.text(0), out),
        BlockKind::Heading { level } => {
            let level = (*level).clamp(1, 4);
            element(&format!("h{level}"), block.body.runs(), page.text(level), out);
        }
        BlockKind::Code { language } => code(language.as_deref(), &block.plain_text(), out),
        BlockKind::Formula => {
            maths(&block.plain_text(), true, page.text(0).maths, out);
            out.push('\n');
        }
        BlockKind::Quote => {
            // Its bar stands where its first block starts, as layout draws it.
            let open =
                if letters_layout::leads(block.body.blocks()) { "<blockquote class=\"rtl\">" } else { "<blockquote>" };
            container(open, "</blockquote>", block.body.blocks(), page, out);
        }
        BlockKind::Callout { tone, .. } => {
            let open = callout(&block.kind, tone_name(*tone));
            container(&open, "</div>", block.body.blocks(), page, out);
        }
        BlockKind::Divider => out.push_str("<hr>\n"),
        BlockKind::Figure { asset, src, alt, width, height, .. } => {
            let image = Figure { asset: asset.as_deref(), src: src.as_deref(), alt, width: *width, height: *height };
            figure(&image, block.body.runs(), Some(&block.id), page, out);
        }
        BlockKind::Table { columns, header } => table(columns, *header, block.body.blocks(), page, out),
        // A merge can leave a row outside a table and a cell outside a row.
        BlockKind::TableRow => table(&[], false, std::slice::from_ref(block), page, out),
        // A merge can leave a caption outside a gallery too.
        BlockKind::Caption | BlockKind::TableCell => element("p", block.body.runs(), page.text(0), out),
        BlockKind::Gallery { .. } => gallery(block, page, out),
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
        BlockKind::LinkPreview { url, title, description, image } if page.contents.is_some() => {
            let card =
                Preview { url, title: title.as_deref(), description: description.as_deref(), image: image.as_deref() };
            preview(&card, page, out);
        }
        // What a copy offers is the link the preview holds, under its title.
        BlockKind::LinkPreview { url, title, .. } => {
            let link = Run { link: Some(url.clone()), ..Run::plain(title.as_deref().unwrap_or(url)) };
            element("p", &[link], page.text(0), out);
        }
        BlockKind::TableOfContents { .. } => contents(&block.id, page, out),
        // Written by `item`, which knows the list around it.
        BlockKind::ListItem { .. } => {}
    }
}

/// Writes a table of contents as layout draws it: a line for each heading it lists, in the heading's own
/// words, set in by the heading's depth, that leads to the heading. One that lists nothing says so in the
/// window alone, and one in a copy is left out, which has no page to lead into.
fn contents(table: &BlockId, page: &Writing, out: &mut String) {
    let Some(lines) = page.contents.as_ref().and_then(|contents| contents.tables.get(table)) else { return };
    if lines.is_empty() {
        if page.window {
            out.push_str("<nav class=\"contents\">\n<p class=\"none\">No headings yet</p>\n</nav>\n");
        }
        return;
    }
    let top = lines.iter().map(|line| line.level).min().unwrap_or(1);
    out.push_str("<nav class=\"contents\">\n");
    for line in lines {
        let _ =
            write!(out, "<p class=\"level-{}\"{}>", line.level - top + 1, if line.rtl { " dir=\"rtl\"" } else { "" });
        inline(&line.runs, page.text(0), out);
        out.push_str("</p>\n");
    }
    out.push_str("</nav>\n");
}

/// Writes a block's id as the end of an address that leads to it. An id may hold what an address cannot.
fn fragment(id: &BlockId, out: &mut String) {
    for byte in id.as_str().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => _ = write!(out, "%{byte:02X}"),
        }
    }
}

/// A link preview's fields.
struct Preview<'a> {
    url: &'a str,
    title: Option<&'a str>,
    description: Option<&'a str>,
    image: Option<&'a str>,
}

/// Writes a link preview as layout draws it, a card that is one link: its picture, then its title, its
/// description and the host it leads to. One that was never fetched shows its address alone. An address
/// that would run script makes a card that leads nowhere.
fn preview(card: &Preview, page: &mut Writing, out: &mut String) {
    let address = href(card.url);
    let tag = if address.is_some() { "a" } else { "div" };
    let _ = write!(out, "<{tag} class=\"preview\"");
    if let Some(address) = &address {
        out.push_str(" href=\"");
        escape(address, out);
        out.push('"');
    }
    out.push_str(">\n");
    if (card.title, card.description, card.image) == (None, None, None) {
        out.push_str("<span class=\"url\">");
        escape(card.url.trim(), out);
        let _ = writeln!(out, "</span>\n</{tag}>");
        return;
    }
    let held = card.image.and_then(|name| {
        let bytes = (page.assets)(name)?;
        Some((media_type(name, &bytes)?, bytes))
    });
    if let Some((media, bytes)) = held {
        // The title beside it says what the picture is of.
        let _ = writeln!(out, "<img src=\"data:{media};base64,{}\" alt=\"\">", base64(&bytes));
    }
    out.push_str("<span class=\"preview-text\">");
    let host = Some(host(card.url)).filter(|host| !host.is_empty()).unwrap_or(card.url);
    for (tag, said) in [("strong", card.title), ("span", card.description), ("small", Some(host))] {
        if let Some(said) = said {
            let _ = write!(out, "<{tag}>");
            escape(said.trim(), out);
            let _ = write!(out, "</{tag}>");
        }
    }
    let _ = writeln!(out, "</span>\n</{tag}>");
}

/// The part of an address that names where it leads: its host, or all of it where it names none.
pub(crate) fn host(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else { return url };
    let authority = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    // What stands before an `@` is a name to sign in with, and the host follows it.
    authority.rsplit_once('@').map_or(authority, |(_, host)| host)
}

/// The opening of a callout and the line that heads it. What was chosen over its tone is said in `data-`
/// attributes, which is what a paste reads, and the title line is there to be seen.
fn callout(kind: &BlockKind, tone: &str) -> String {
    let BlockKind::Callout { title, icon, color, .. } = kind else { return String::new() };
    let Some(look) = kind.callout_look() else { return String::new() };
    let mut open = format!("<div class=\"callout {tone}\" role=\"note\"");
    for (name, value) in [("data-title", title), ("data-icon", icon), ("data-color", color)] {
        if let Some(value) = value {
            let _ = write!(open, " {name}=\"");
            escape(value, &mut open);
            open.push('"');
        }
    }
    let mut ink = String::new();
    if let Some(color) = look.color {
        // The fill is the colour at the strength a tone's fill has.
        let _ = write!(open, " style=\"border-color: {color}; background: {color}1f;\"");
        ink = format!(" style=\"color: {color};\"");
    }
    open.push('>');
    if look.icon.is_none() && look.title.is_empty() {
        return open;
    }
    let alone = if look.title.is_empty() { " alone" } else { "" };
    let _ = write!(open, "\n<p class=\"callout-title{alone}\"{ink}>");
    if let Some(path) = look.icon.and_then(letters_layout::icon_path) {
        let _ = write!(
            open,
            "<svg width=\"18\" height=\"18\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" \
             stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\">\
             <path d=\"{path}\"/></svg>"
        );
    }
    escape(look.title, &mut open);
    open.push_str("</p>");
    open
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
fn element(tag: &str, runs: &[Run], line: Line, out: &mut String) {
    let _ = write!(out, "<{tag}>");
    if runs.iter().all(|run| run.text.is_empty()) {
        out.push_str("<br>");
    } else {
        inline(runs, line, out);
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
                    let _ = write!(out, "<{cell}{align}{}>", language(block.lang.as_deref()));
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
fn item(lists: &mut Vec<&'static str>, list: ListKind, depth: u8, runs: &[Run], line: Line, out: &mut String) {
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
    inline(runs, line, out);
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

/// Writes a figure and its caption. `id` is its block's, for a figure that can be numbered and referred to.
fn figure(image: &Figure, caption: &[Run], id: Option<&BlockId>, page: &mut Writing, out: &mut String) {
    open("figure", id, page, out);
    picture(image, None, page, out);
    self::caption(caption, id, page, out);
    out.push_str("</figure>\n");
}

/// Writes a gallery as one figure: its pictures in the rows layout tiles them in, and its caption under them.
fn gallery(block: &Block, page: &mut Writing, out: &mut String) {
    let children = block.body.blocks();
    let pictures = children.iter().filter_map(|child| match &child.kind {
        BlockKind::Figure { asset, src, alt, width, height, .. } => {
            Some(Figure { asset: asset.as_deref(), src: src.as_deref(), alt, width: *width, height: *height })
        }
        _ => None,
    });
    let pictures: Vec<Figure> = pictures.collect();
    open("figure class=\"gallery\"", Some(&block.id), page, out);
    let mut rest = &pictures[..];
    while !rest.is_empty() {
        // Up to three to a row, and the last picture is never left alone.
        let (row, after) = rest.split_at(if rest.len() == 4 { 2 } else { rest.len().min(3) });
        out.push_str("<div class=\"row\">\n");
        for image in row {
            // Each takes its share of the row by its shape, which gives the row one height.
            let shape = if image.width > 0.0 && image.height > 0.0 { image.width / image.height } else { 1.0 };
            picture(image, Some(shape), page, out);
        }
        out.push_str("</div>\n");
        rest = after;
    }
    let last = children.iter().rfind(|child| child.kind == BlockKind::Caption);
    caption(last.map_or(&[], |last| last.body.runs()), Some(&block.id), page, out);
    out.push_str("</figure>\n");
}

/// Opens a figure's element. One that is numbered carries its block's id, where a reference to it leads.
fn open(tag: &str, id: Option<&BlockId>, page: &Writing, out: &mut String) {
    let _ = write!(out, "<{tag}");
    let named = page.figures.as_ref().zip(id).filter(|(figures, id)| figures.names.contains_key(id));
    if let Some((_, id)) = named {
        out.push_str(" id=\"");
        escape(id.as_str(), out);
        out.push('"');
    }
    out.push_str(">\n");
}

/// Writes a picture. In a gallery's row, `tile` is its width over its height.
///
/// An image the file holds is written into the page. One that is still remote is not: the page must fetch
/// nothing, so it becomes a link. One that cannot be found leaves its description in its place.
fn picture(image: &Figure, tile: Option<f32>, page: &mut Writing, out: &mut String) {
    let held = image.asset.and_then(|name| {
        let bytes = (page.assets)(name)?;
        Some((media_type(name, &bytes)?, bytes))
    });
    let remote = image.src.and_then(href);
    let share = tile.map(|shape| format!(" style=\"flex: {}\"", number(shape))).unwrap_or_default();
    if let Some((media, bytes)) = held {
        let _ = write!(out, "<img src=\"data:{media};base64,{}\" alt=\"", base64(&bytes));
        escape(image.alt, out);
        // In CSS pixels, so the page keeps the image's place while it decodes.
        let pixels = |points: f32| (points * PIXELS).round().max(1.0);
        let _ = writeln!(out, "\" width=\"{}\" height=\"{}\"{share}>", pixels(image.width), pixels(image.height));
    } else if let Some(address) = remote {
        let _ = write!(out, "<p{share}><a href=\"");
        escape(&address, out);
        out.push_str("\">");
        escape(if image.alt.is_empty() { &address } else { image.alt }, out);
        out.push_str("</a></p>\n");
    } else if !image.alt.is_empty() {
        let _ = write!(out, "<p class=\"fallback\"{share}>");
        escape(image.alt, out);
        out.push_str("</p>\n");
    }
}

/// Writes the caption of the figure with `id`. A numbered figure's opens with its label, as on the page.
fn caption(runs: &[Run], id: Option<&BlockId>, page: &Writing, out: &mut String) {
    let label = page.figures.as_ref().zip(id).and_then(|(figures, id)| figures.names.get(id));
    let captioned = runs.iter().any(|run| !run.text.is_empty());
    if label.is_none() && !captioned {
        return;
    }
    out.push_str("<figcaption>");
    if let Some(label) = label {
        escape(label, out);
        out.push_str(if captioned { ": " } else { ":" });
    }
    inline(runs, page.text(0), out);
    out.push_str("</figcaption>\n");
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
fn inline(runs: &[Run], line: Line, out: &mut String) {
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
            // Words in another language than the text around them say so, for whatever reads the page aloud.
            let spoken = language(run.lang.as_deref());
            if !spoken.is_empty() {
                let _ = write!(out, "<span{spoken}>");
            }
            tags().for_each(|tag| _ = write!(out, "<{tag}>"));
            let target = run.marks.iter().find_map(|mark| match mark {
                Mark::Ref(target) => Some(target),
                _ => None,
            });
            match (target, line.figures) {
                // An anchor cannot hold another, so a reference inside a link is its label alone.
                (Some(target), Some(figures)) => reference(target, figures, address.is_none(), out),
                // A reference is not also a formula: its text is what is shown where it cannot be followed.
                (Some(_), None) => text(&run.text, out),
                (None, _) if run.marks.contains(&Mark::Math) => self::maths(&run.text, false, line.maths, out),
                (None, _) => text(&run.text, out),
            }
            tags().rev().for_each(|tag| _ = write!(out, "</{tag}>"));
            if !spoken.is_empty() {
                out.push_str("</span>");
            }
        }
        if address.is_some() {
            out.push_str("</a>");
        }
        rest = after;
    }
}

/// Writes a reference to the figure `target`: the figure's label, whatever text the reference carries, as a
/// link to the figure where `linked`. One whose figure is gone, or is no longer numbered, says so.
fn reference(target: &BlockId, figures: &Figures, linked: bool, out: &mut String) {
    match figures.names.get(target) {
        Some(name) if linked => {
            out.push_str("<a class=\"ref\" href=\"#");
            fragment(target, out);
            out.push_str("\">");
            escape(name, out);
            out.push_str("</a>");
        }
        Some(name) => escape(name, out),
        None => {
            out.push_str("<span class=\"ref missing\">");
            escape(&figures.missing, out);
            out.push_str("</span>");
        }
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
const TYPES: [(&str, &str); 12] = [
    ("body", "p, li"),
    ("body", "th, td"),
    ("body", ".preview"),
    ("heading1", "h1"),
    ("heading2", "h2"),
    ("heading3", "h3"),
    ("heading4", "h4"),
    ("quote", "blockquote p, blockquote li"),
    ("code", "pre"),
    ("caption", "figcaption, .fallback, .unknown"),
    ("caption", ".preview-text > span"),
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
         li::before, .unknown, .contents > .none, .preview-text > small {{ color: {marker}; }}\n\
         .preview > img {{ background: {code_fill}; }}\n\
         li.task > input {{ border-color: {line_strong}; }}\n\
         li.task > input:checked {{ border-color: {accent}; background: {accent} {tick}; }}\n\
         blockquote, .unknown {{ border-color: {line_strong}; }}\n\
         hr, th, td, .preview {{ border-color: {line}; }}\n\
         a {{ color: {link}; }}\n\
         a.ref, .contents a {{ color: inherit; }}\n\
         .preview > .url {{ color: {link}; }}\n\
         .ref.missing {{ color: {warning}; }}\n\
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
        warning = color(on.warn.accent),
        highlight = color(on.highlight),
    );
    for (tone, look) in [(Tone::Tip, on.tip), (Tone::Info, on.info), (Tone::Warn, on.warn), (Tone::Danger, on.danger)] {
        let (name, accent, fill) = (tone_name(tone), color(look.accent), color(look.fill));
        let _ = writeln!(css, ".callout.{name} {{ border-color: {accent}; background: {fill}; }}");
        let _ = writeln!(css, ".callout.{name} > .callout-title {{ color: {accent}; }}");
    }
    for kind in TokenKind::ALL {
        let _ = writeln!(css, ".tok-{} {{ color: {}; }}", kind.as_str(), color(on.syntax.color(ink(kind))));
    }
}

/// Writes what stands before each list item, where layout draws it: ending 8 pixels before the text, a
/// bullet or a box level with the middle of a lower-case letter. A browser's own markers sit a few pixels
/// off, and it greys a box that cannot be ticked. The measures are from the side the item starts at.
fn markers(text: f32, css: &mut String) {
    let dot = (text * 0.36).round().max(3.0);
    // An empty marker, where `none` would have some screen readers stop calling the list a list.
    let _ = write!(
        css,
        "li {{ list-style-type: \"\"; }}\n\
         li::before {{ display: inline-block; box-sizing: border-box; margin-inline-end: 8px; }}\n\
         ul > li::before {{ content: \"\"; width: {dot}px; height: {dot}px; margin-inline-start: -{reach}px; \
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
        // An item that reads the other way than the one it sits in has its whole indent at its own side.
        let inset = depth * 24;
        let _ = writeln!(css, "{}li[dir] {{ margin-inline: {inset}px -{inset}px; }}", "li ".repeat(depth));
    }
    css.push_str(concat!(
        "ol { counter-reset: item; }\n",
        "ol > li { counter-increment: item; }\n",
        "ol > li::before { content: counter(item) \".\"; width: 4em; margin-inline-start: calc(-4em - 8px); ",
        "text-align: end; }\n",
        "li.task::before { content: none; }\n",
        "li.task > input { appearance: none; -webkit-appearance: none; box-sizing: border-box; width: 14px; ",
        "height: 14px; margin: 0; margin-inline: -22px 8px; vertical-align: calc(0.34em - 7px); ",
        "border: 1.5px solid; ",
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
    type_rule(".preview", &body, false, css);
    type_rule(".preview-text > span, .preview-text > small", &of("caption"), false, css);
    space_rule("p", body.space_before, body.space_after, css);
    // A table of contents and a link preview keep the room a paragraph does.
    space_rule(".contents, .preview", body.space_before, body.space_after, css);
    for level in 1..=4 {
        let (heading, selector) = (of(&format!("heading{level}")), format!("h{level}"));
        type_rule(&selector, &heading, false, css);
        space_rule(&selector, heading.space_before, heading.space_after, css);
    }
    type_rule("blockquote p, blockquote li", &quote, false, css);
    space_rule("blockquote p", quote.space_before, quote.space_after, css);
    space_rule("blockquote .contents, blockquote .preview", quote.space_before, quote.space_after, css);
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
         ul, ol {{ margin: {before} 0 {after}; padding: 0; }}\n\
         li {{ padding-inline-start: 24px; }}\n\
         li + li, li > ul, li > ol {{ margin-top: {between}; }}\n\
         li > ul, li > ol {{ margin-bottom: 0; }}\n\
         :is(ul, ol) + :is(ul, ol) {{ margin-top: {beside}; }}\n\
         blockquote {{ margin: {after} 0; padding: 0 0 0 16px; border-left: 3px solid; }}\n\
         blockquote.rtl {{ padding: 0 16px 0 0; border-width: 0 3px 0 0; border-style: solid; }}\n\
         .callout {{ margin: {callout_gap} 0; padding: 12px 16px 12px 13px; border-left: 3px solid; \
         border-radius: 6px; }}\n\
         .callout > .callout-title {{ display: flex; align-items: center; gap: 8px; margin: 0 0 4px; \
         font-weight: 600; }}\n\
         .callout > .callout-title.alone {{ float: left; margin: 0 8px 0 0; }}\n\
         .callout > .callout-title > svg {{ flex: none; }}\n\
         [data-page-break] {{ break-before: page; }}\n",
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
         figure > figcaption {{ margin-top: 8px; }}\n\
         img {{ display: block; max-width: 100%; height: auto; }}\n\
         .gallery > .row {{ display: flex; align-items: flex-start; gap: 8px; }}\n\
         .gallery > .row + .row {{ margin-top: 8px; }}\n\
         .gallery > .row > * {{ flex: 1; min-width: 0; margin: 0; }}\n\
         a.ref {{ text-decoration: none; }}\n\
         p, h1, h2, h3, h4, th, td, figcaption {{ unicode-bidi: plaintext; }}\n\
         .contents > p {{ margin: 0; padding-inline-end: {page_number}; }}\n\
         .contents > p + p {{ margin-top: 3px; }}\n\
         .contents > .level-2 {{ padding-inline-start: 24px; }}\n\
         .contents > .level-3 {{ padding-inline-start: 48px; }}\n\
         .contents > .level-4 {{ padding-inline-start: 72px; }}\n\
         .contents a, .preview {{ text-decoration: none; }}\n\
         .preview {{ display: flex; align-items: flex-start; gap: 16px; padding: 11px 15px; border: 1px solid; \
         border-radius: 6px; container-type: inline-size; }}\n\
         .preview > img {{ flex: none; width: 120px; height: 80px; object-fit: cover; border-radius: 3px; }}\n\
         @container (max-width: 295px) {{ .preview > img {{ display: none; }} }}\n\
         .preview-text {{ flex: 1; min-width: 0; align-self: center; display: flex; flex-direction: column; \
         gap: 3px; }}\n\
         .preview-text > *, .preview > .url {{ display: -webkit-box; -webkit-box-orient: vertical; \
         overflow: hidden; unicode-bidi: plaintext; }}\n\
         .preview-text > strong, .preview > .url {{ -webkit-line-clamp: 2; line-clamp: 2; }}\n\
         .preview-text > span {{ -webkit-line-clamp: 3; line-clamp: 3; text-align: start; }}\n\
         .preview-text > small {{ -webkit-line-clamp: 1; line-clamp: 1; text-align: start; }}\n\
         .preview > .url {{ flex: 1; min-width: 0; padding-inline-end: 128px; }}\n\
         .math.display > svg {{ display: block; margin: 0 auto; max-width: 100%; height: auto; }}\n\
         article > :first-child, blockquote > :first-child, .callout > :first-child, \
         .callout > .callout-title + * {{ margin-top: 0; }}\n\
         article > :last-child, blockquote > :last-child, .callout > :last-child {{ margin-bottom: 0; }}\n\
         hr {{ margin: {rule_gap} 0; border: 0; border-top: 1px solid; }}\n\
         mark {{ color: inherit; }}\n\
         .tok-comment {{ font-style: italic; }}\n",
        after = px(unit),
        // The room a sheet sets a page number in, which a line keeps on screen too, so that it breaks alike.
        page_number = px((body.size * PIXELS * 2.6).round()),
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
