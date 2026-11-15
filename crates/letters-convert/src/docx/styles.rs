//! Word's styles as the reader needs them: what each resolves to, which element type it stands for, and the
//! document style they seed.

use std::collections::{BTreeMap, BTreeSet};

use letters_model::{Align, Page, Style, TextStyle, Tone};
use roxmltree::{Document, Node};

use super::package::Element;
use super::{CALLOUTS, DIVIDER, SINGLE, TWIPS, TYPES, monospace};

/// How deep a chain of styles is followed, which also ends one that loops.
const MAX_CHAIN: usize = 32;

/// The formatting Word sets on text. A field is `None` where a level of the cascade leaves it alone.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct RunProps {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub highlight: Option<bool>,
    /// A fill behind the text, which some tools write where Word would highlight.
    pub shaded: Option<bool>,
    /// Raised or lowered: superscript or subscript.
    pub shifted: Option<bool>,
    pub family: Option<String>,
    /// In points.
    pub size: Option<f32>,
    /// As `#rrggbb`.
    pub color: Option<String>,
    /// The character style named here.
    pub style: Option<String>,
    /// The language the text is in, as a BCP 47 tag.
    pub lang: Option<String>,
}

impl RunProps {
    /// These properties, with `base` showing through wherever they are silent.
    pub fn over(self, base: &Self) -> Self {
        Self {
            bold: self.bold.or(base.bold),
            italic: self.italic.or(base.italic),
            underline: self.underline.or(base.underline),
            strike: self.strike.or(base.strike),
            highlight: self.highlight.or(base.highlight),
            shaded: self.shaded.or(base.shaded),
            shifted: self.shifted.or(base.shifted),
            family: self.family.or_else(|| base.family.clone()),
            size: self.size.or(base.size),
            color: self.color.or_else(|| base.color.clone()),
            style: self.style.or_else(|| base.style.clone()),
            lang: self.lang.or_else(|| base.lang.clone()),
        }
    }

    /// Reads a `w:rPr`.
    pub fn read(props: Node, theme: &Theme) -> Self {
        let toggle = |name: &str| props.child(name).map(|node| node.on());
        let fonts = props.child("rFonts");
        let named = fonts.and_then(|fonts| fonts.attr("ascii").or_else(|| fonts.attr("hAnsi")));
        let themed = fonts.and_then(|fonts| fonts.attr("asciiTheme").or_else(|| fonts.attr("hAnsiTheme")));
        let family = named.map(str::to_owned).or_else(|| theme.family(themed?)).filter(|name| !name.trim().is_empty());
        let filled = props
            .child("shd")
            .and_then(|shade| shade.attr("fill"))
            .map(|fill| !fill.eq_ignore_ascii_case("auto") && !fill.eq_ignore_ascii_case("ffffff"));
        Self {
            bold: toggle("b"),
            italic: toggle("i"),
            underline: toggle("u"),
            strike: toggle("strike").or(toggle("dstrike")),
            highlight: toggle("highlight"),
            shaded: filled,
            shifted: props.value("vertAlign").map(|shift| shift != "baseline"),
            family,
            size: props.number::<f32>("sz").map(|half| half / 2.0).filter(|size| *size > 0.0),
            color: props.value("color").and_then(color),
            style: props.value("rStyle").map(str::to_owned),
            lang: props.value("lang").filter(|tag| is_language(tag)).map(str::to_owned),
        }
    }
}

/// Whether a tag is shaped as BCP 47 asks.
fn is_language(tag: &str) -> bool {
    !tag.is_empty() && tag.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// A colour as the style writes one. Word's `auto` is black on a white page.
fn color(value: &str) -> Option<String> {
    if value.eq_ignore_ascii_case("auto") {
        return Some("#000000".into());
    }
    (value.len() == 6 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("#{}", value.to_ascii_lowercase()))
}

/// The height Word gives a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Line {
    /// A multiple of single spacing.
    Multiple(f32),
    /// A height in points, fixed or as a least.
    Points(f32),
}

/// The formatting Word sets on a paragraph.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ParaProps {
    /// The paragraph style named here.
    pub style: Option<String>,
    pub align: Option<Align>,
    /// In points.
    pub before: Option<f32>,
    pub after: Option<f32>,
    pub line: Option<Line>,
    /// The list and level the paragraph is an item of. List `0` takes it out of any list.
    pub list: Option<(String, u8)>,
    /// The heading level this paragraph has in Word's outline, from 0.
    pub outline: Option<u8>,
    pub page_break: Option<bool>,
    /// Whether the only border is a line above or below, which is how Word draws a rule across the page.
    pub ruled: Option<bool>,
}

impl ParaProps {
    pub fn over(self, base: &Self) -> Self {
        Self {
            style: self.style.or_else(|| base.style.clone()),
            align: self.align.or(base.align),
            before: self.before.or(base.before),
            after: self.after.or(base.after),
            line: self.line.or(base.line),
            list: self.list.or_else(|| base.list.clone()),
            outline: self.outline.or(base.outline),
            page_break: self.page_break.or(base.page_break),
            ruled: self.ruled.or(base.ruled),
        }
    }

    /// Reads a `w:pPr`.
    pub fn read(props: Node) -> Self {
        let spacing = props.child("spacing");
        let twips = |name: &str| {
            let value: f32 = spacing?.attr(name)?.trim().parse().ok()?;
            Some(value / TWIPS)
        };
        let line = twips("line").filter(|line| *line > 0.0).map(|points| {
            match spacing.and_then(|spacing| spacing.attr("lineRule")) {
                Some("exact" | "atLeast") => Line::Points(points),
                // 240 twentieths is single spacing.
                _ => Line::Multiple(points / 12.0),
            }
        });
        let align = props.value("jc").map(|align| match align {
            "center" => Align::Center,
            "right" | "end" => Align::End,
            "both" | "distribute" => Align::Justify,
            _ => Align::Start,
        });
        let list = props.child("numPr").and_then(|list| {
            let level = list.number::<u8>("ilvl").unwrap_or(0);
            Some((list.value("numId")?.to_owned(), level))
        });
        let ruled = props.child("pBdr").map(|borders| {
            let drawn = |side: &str| borders.child(side).is_some_and(|side| side.on() && side.val() != Some("nil"));
            (drawn("top") || drawn("bottom")) && !drawn("left") && !drawn("right")
        });
        Self {
            style: props.value("pStyle").map(str::to_owned),
            align,
            before: twips("before").filter(|space| *space >= 0.0),
            after: twips("after").filter(|space| *space >= 0.0),
            line,
            list,
            outline: props.number::<u8>("outlineLvl"),
            page_break: props.child("pageBreakBefore").map(|node| node.on()),
            ruled,
        }
    }
}

/// The two fonts a theme names, which styles refer to in place of a family.
#[derive(Clone, Debug, Default)]
pub(super) struct Theme {
    major: Option<String>,
    minor: Option<String>,
}

impl Theme {
    pub fn read(xml: &Document) -> Self {
        let face = |group: &str| {
            let group = xml.descendants().find(|node| node.tag_name().name() == group)?;
            let latin = group.children().find(|node| node.tag_name().name() == "latin")?;
            latin.attribute("typeface").filter(|name| !name.is_empty()).map(str::to_owned)
        };
        Self { major: face("majorFont"), minor: face("minorFont") }
    }

    /// The family behind a name such as `minorHAnsi`.
    fn family(&self, name: &str) -> Option<String> {
        if name.starts_with("major") { self.major.clone() } else { self.minor.clone() }
    }
}

/// What a paragraph of some style becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    Body,
    Title,
    /// The level as Word counts it, from 1 to 9.
    Heading(u8),
    Quote,
    Callout(Tone),
    Code,
    Caption,
    Divider,
    /// An entry of a table of contents.
    Contents,
    /// The text of a cell in a row that heads a table.
    ColumnHead,
}

/// The names paragraph styles for code go by, in Word and in the tools that write `.docx`.
const CODE: [&str; 6] = ["code", "source code", "code block", "html preformatted", "preformatted text", "verbatim"];

/// The names character styles for code go by.
const CODE_INLINE: [&str; 7] =
    ["inline code", "verbatim char", "html code", "source text", "html typewriter", "html keyboard", "code char"];

/// The names quote styles go by.
const QUOTE: [&str; 4] = ["quote", "intense quote", "block text", "quotations"];

fn role(name: &str) -> Option<Role> {
    let name = name.trim().to_ascii_lowercase();
    if let Some(level) = name.strip_prefix("heading ").and_then(|level| level.parse::<u8>().ok()) {
        return Some(Role::Heading(level.clamp(1, 9)));
    }
    if let Some((tone, ..)) = CALLOUTS.iter().find(|(_, _, callout)| callout.eq_ignore_ascii_case(&name)) {
        return Some(Role::Callout(*tone));
    }
    Some(match name.as_str() {
        "title" => Role::Title,
        "table heading" => Role::ColumnHead,
        "caption" => Role::Caption,
        "toc heading" | "table of figures" => Role::Contents,
        "horizontal line" => Role::Divider,
        name if name.eq_ignore_ascii_case(DIVIDER.1) => Role::Divider,
        name if name.starts_with("toc ") => Role::Contents,
        name if QUOTE.contains(&name) => Role::Quote,
        name if CODE.contains(&name) => Role::Code,
        _ => return None,
    })
}

#[derive(Clone, Debug, Default)]
struct WordStyle {
    /// In lower case.
    name: String,
    paragraph: bool,
    based_on: Option<String>,
    para: ParaProps,
    run: RunProps,
}

/// A document's styles and the defaults beneath them.
#[derive(Clone, Debug, Default)]
pub(super) struct Styles {
    by_id: BTreeMap<String, WordStyle>,
    para: ParaProps,
    run: RunProps,
    /// The paragraph style of a paragraph that names none.
    normal: Option<String>,
    /// The table styles that set a first row apart.
    headed: BTreeSet<String>,
}

impl Styles {
    pub fn read(xml: &Document, theme: &Theme) -> Self {
        let root = xml.root_element();
        let mut styles = Self::default();
        if let Some(defaults) = root.child("docDefaults") {
            if let Some(props) = defaults.child("rPrDefault").and_then(|node| node.child("rPr")) {
                styles.run = RunProps::read(props, theme);
            }
            if let Some(props) = defaults.child("pPrDefault").and_then(|node| node.child("pPr")) {
                styles.para = ParaProps::read(props);
            }
        }
        for style in root.all("style") {
            let Some(id) = style.attr("styleId") else { continue };
            let paragraph = style.attr("type") == Some("paragraph");
            if paragraph && style.attr("default").is_some_and(|value| value == "1" || value == "true") {
                styles.normal = Some(id.to_owned());
            }
            let read = WordStyle {
                name: style.value("name").unwrap_or(id).to_ascii_lowercase(),
                paragraph,
                based_on: style.value("basedOn").map(str::to_owned),
                para: style.child("pPr").map(ParaProps::read).unwrap_or_default(),
                run: style.child("rPr").map(|props| RunProps::read(props, theme)).unwrap_or_default(),
            };
            styles.by_id.insert(id.to_owned(), read);
            let heads = |part: Node| part.is("tblStylePr") && part.attr("type") == Some("firstRow");
            if style.attr("type") == Some("table") && style.children().any(heads) {
                styles.headed.insert(id.to_owned());
            }
        }
        styles
    }

    /// Whether a table style gives a table's first row a look of its own, as it does a row of headings.
    pub fn heads(&self, id: &str) -> bool {
        self.headed.contains(id)
    }

    /// The language text is in where nothing says otherwise, when Word recorded one.
    pub fn language(&self) -> Option<&str> {
        self.run.lang.as_deref()
    }

    /// A style and those it is based on, nearest first.
    fn chain<'s>(&'s self, id: Option<&str>) -> impl Iterator<Item = &'s WordStyle> {
        let mut next = id.and_then(|id| self.by_id.get(id));
        std::iter::from_fn(move || {
            let style = next?;
            next = style.based_on.as_deref().and_then(|id| self.by_id.get(id));
            Some(style)
        })
        .take(MAX_CHAIN)
    }

    /// What a paragraph of this style looks like, with the defaults beneath it. `None` is the default style.
    pub fn paragraph(&self, id: Option<&str>) -> (ParaProps, RunProps) {
        let id = id.or(self.normal.as_deref());
        let (mut para, mut run) = (ParaProps::default(), RunProps::default());
        for style in self.chain(id) {
            para = para.over(&style.para);
            run = run.over(&style.run);
        }
        (para.over(&self.para), run.over(&self.run))
    }

    /// What a character style sets, with nothing beneath it.
    pub fn character(&self, id: &str) -> RunProps {
        self.chain(Some(id)).fold(RunProps::default(), |run, style| run.over(&style.run))
    }

    /// What a paragraph of this style becomes. The nearest style whose name is known decides.
    pub fn role(&self, id: Option<&str>) -> Role {
        self.chain(id.or(self.normal.as_deref())).find_map(|style| role(&style.name)).unwrap_or(Role::Body)
    }

    /// How deep a list style sets its items by name alone: Word's `List Bullet 2` is a second level, though
    /// it is the first level of a list of its own.
    pub fn depth(&self, id: Option<&str>) -> u8 {
        let named = self.chain(id).find_map(|style| {
            let level = style.name.strip_prefix("list ")?.rsplit(' ').next()?.parse::<u8>().ok()?;
            Some(level.clamp(1, 9) - 1)
        });
        named.unwrap_or(0)
    }

    /// Whether a character style marks code.
    pub fn code(&self, id: &str) -> bool {
        self.chain(Some(id)).any(|style| CODE_INLINE.contains(&style.name.as_str()))
    }

    /// The id of the paragraph style with this name.
    fn named(&self, name: &str) -> Option<&str> {
        let found = self.by_id.iter().find(|(_, style)| style.paragraph && style.name.eq_ignore_ascii_case(name));
        found.map(|(id, _)| id.as_str())
    }

    /// The id of the paragraph style that seeds an element type.
    fn source(&self, element: &str) -> Option<&str> {
        match element {
            "body" => self.normal.as_deref().or_else(|| self.named("normal")),
            "quote" => QUOTE.iter().find_map(|name| self.named(name)),
            "code" => CODE.iter().find_map(|name| self.named(name)),
            element => self.named(TYPES.iter().find(|(known, ..)| *known == element)?.2),
        }
    }

    /// The document style Word's styles amount to. A type Word does not define keeps `base`, moved onto the
    /// body's family and colour where it followed them.
    pub fn seed(&self, base: &Style) -> Style {
        let mut seeded = base.clone();
        let fallback = TextStyle::default();
        let old_body = base.types.get("body").unwrap_or(&fallback).clone();
        let body = self.text_style(self.source("body"), &old_body);
        for name in Style::TYPES {
            let old = base.types.get(name).unwrap_or(&old_body);
            let new = match self.source(name) {
                _ if name == "body" => body.clone(),
                Some(id) => self.text_style(Some(id), old),
                None => {
                    let mut kept = old.clone();
                    if old.family == old_body.family {
                        kept.family.clone_from(&body.family);
                    }
                    if old.color == old_body.color {
                        kept.color.clone_from(&body.color);
                    }
                    kept
                }
            };
            seeded.types.insert(name.to_owned(), new);
        }
        seeded
    }

    /// A Word style as an element type's style, keeping what Word has no say over from `base`.
    fn text_style(&self, id: Option<&str>, base: &TextStyle) -> TextStyle {
        let (para, run) = self.paragraph(id);
        let bold = run.bold.unwrap_or(false);
        // Word knows bold and not bold. A weight that already says the same is kept as it is.
        let weight = match (bold, base.weight >= 600) {
            (true, true) | (false, false) => base.weight,
            (true, false) => 700,
            (false, true) => 400,
        };
        // Word's own default, for a file that sets no size anywhere.
        let size = run.size.unwrap_or(10.0);
        let line_height = match para.line {
            Some(Line::Multiple(multiple)) => multiple * SINGLE,
            Some(Line::Points(points)) => points / size,
            None => SINGLE,
        };
        TextStyle {
            family: run.family.unwrap_or_else(|| base.family.clone()),
            weight,
            italic: run.italic.unwrap_or(false),
            size,
            color: run.color.unwrap_or_else(|| "#000000".into()),
            align: para.align.unwrap_or(Align::Start),
            // Kept to two places, so that a style reads as tidy numbers.
            line_height: ((line_height * 100.0).round() / 100.0).clamp(0.5, 5.0),
            space_before: para.before.unwrap_or(0.0),
            space_after: para.after.unwrap_or(0.0),
            extra: base.extra.clone(),
        }
    }
}

/// Reads the page a section is set on. `None` when its measures leave no room for text.
pub(super) fn page(section: Node, base: &Page) -> Option<Page> {
    let size = section.child("pgSz")?;
    let points = |node: Node, name: &str| Some(node.attr(name)?.trim().parse::<f32>().ok()? / TWIPS);
    let mut page = Page { width: points(size, "w")?, height: points(size, "h")?, ..base.clone() };
    if let Some(margins) = section.child("pgMar") {
        // Word allows a negative margin, to let the text run into the header.
        let margin = |name: &str, base: f32| points(margins, name).map_or(base, f32::abs);
        page.margin_top = margin("top", base.margin_top);
        page.margin_right = margin("right", base.margin_right);
        page.margin_bottom = margin("bottom", base.margin_bottom);
        page.margin_left = margin("left", base.margin_left);
    }
    let roomy = page.margin_left + page.margin_right < page.width && page.margin_top + page.margin_bottom < page.height;
    (page.width > 0.0 && page.height > 0.0 && roomy).then_some(page)
}

/// Whether text in this family reads as code.
pub(super) fn code_family(run: &RunProps) -> bool {
    run.family.as_deref().is_some_and(monospace)
}
