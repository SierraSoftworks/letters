//! A formula as presentation MathML: what it is made of, for the formats and tools that take maths as a
//! structure and not as a drawing.
//!
//! The tree follows the source: a fraction is an `mfrac`, `\left(` and `\right)` an `mrow` between two
//! fences. What only nudges the drawing, such as `\smash`, is kept where MathML has a word for it and
//! dropped where it has none.

use std::fmt::Write as _;
use std::ops::Range;

use ratex_font::symbols::{self, get_symbol};
use ratex_parser::parse_node::{AlignType, Measurement, Mode, ParseNode, StyleStr};
use ratex_parser::parser::Parser;

use crate::Error;
use crate::engine::parse_error;

/// The character MathML puts between a function's name and what it is applied to.
const APPLY: &str = "\u{2061}";

/// The mark each accent command sets over or under its base.
const ACCENTS: &[(&str, char)] = &[
    (r"\hat", '^'),
    (r"\widehat", '^'),
    (r"\tilde", '~'),
    (r"\widetilde", '~'),
    (r"\utilde", '~'),
    (r"\bar", '\u{af}'),
    (r"\vec", '\u{2192}'),
    (r"\dot", '\u{2d9}'),
    (r"\ddot", '\u{a8}'),
    (r"\acute", '\u{b4}'),
    (r"\grave", '`'),
    (r"\check", '\u{2c7}'),
    (r"\widecheck", '\u{2c7}'),
    (r"\breve", '\u{2d8}'),
    (r"\mathring", '\u{2da}'),
    (r"\overrightarrow", '\u{2192}'),
    (r"\underrightarrow", '\u{2192}'),
    (r"\overleftarrow", '\u{2190}'),
    (r"\underleftarrow", '\u{2190}'),
    (r"\overleftrightarrow", '\u{2194}'),
    (r"\underleftrightarrow", '\u{2194}'),
    (r"\Overrightarrow", '\u{21d2}'),
    (r"\overrightharpoon", '\u{21c0}'),
    (r"\overleftharpoon", '\u{21bc}'),
    (r"\overgroup", '\u{23e0}'),
    (r"\undergroup", '\u{23e1}'),
    (r"\overlinesegment", '\u{203e}'),
    (r"\underlinesegment", '\u{203e}'),
    (r"\overbrace", '\u{23de}'),
    (r"\underbrace", '\u{23df}'),
    (r"\overbracket", '\u{23b4}'),
    (r"\underbracket", '\u{23b5}'),
    // The accents of text.
    (r"\'", '\u{b4}'),
    (r"\`", '`'),
    (r"\^", '^'),
    (r"\~", '~'),
    (r"\=", '\u{af}'),
    (r"\.", '\u{2d9}'),
    ("\\\"", '\u{a8}'),
    (r"\u", '\u{2d8}'),
    (r"\v", '\u{2c7}'),
    (r"\H", '\u{2dd}'),
    (r"\r", '\u{2da}'),
];

/// The arrow each stretching arrow command draws.
const ARROWS: &[(&str, char)] = &[
    (r"\xrightarrow", '\u{2192}'),
    (r"\xleftarrow", '\u{2190}'),
    (r"\xleftrightarrow", '\u{2194}'),
    (r"\xRightarrow", '\u{21d2}'),
    (r"\xLeftarrow", '\u{21d0}'),
    (r"\xLeftrightarrow", '\u{21d4}'),
    (r"\xhookrightarrow", '\u{21aa}'),
    (r"\xhookleftarrow", '\u{21a9}'),
    (r"\xmapsto", '\u{21a6}'),
    (r"\xrightharpoonup", '\u{21c0}'),
    (r"\xrightharpoondown", '\u{21c1}'),
    (r"\xleftharpoonup", '\u{21bc}'),
    (r"\xleftharpoondown", '\u{21bd}'),
    (r"\xrightleftharpoons", '\u{21cc}'),
    (r"\xleftrightharpoons", '\u{21cb}'),
    (r"\xrightequilibrium", '\u{21cc}'),
    (r"\xleftequilibrium", '\u{21cb}'),
    (r"\xrightleftarrows", '\u{21c4}'),
    (r"\xtofrom", '\u{21c4}'),
    (r"\xtwoheadrightarrow", '\u{21a0}'),
    (r"\xtwoheadleftarrow", '\u{219e}'),
    (r"\xlongequal", '='),
];

/// The `mathvariant` each font command asks for. One that MathML has no variant for keeps the letters plain.
const VARIANTS: &[(&str, &str)] = &[
    ("mathrm", "normal"),
    ("mathit", "italic"),
    ("mathnormal", "italic"),
    ("mathbf", "bold"),
    ("boldsymbol", "bold-italic"),
    ("mathbb", "double-struck"),
    ("mathcal", "script"),
    ("mathscr", "script"),
    ("mathfrak", "fraktur"),
    ("mathsf", "sans-serif"),
    ("mathsfit", "sans-serif-italic"),
    ("mathtt", "monospace"),
    (r"\textbf", "bold"),
    (r"\textit", "italic"),
    (r"\textsl", "italic"),
    (r"\emph", "italic"),
    (r"\textsf", "sans-serif"),
    (r"\texttt", "monospace"),
];

/// How `menclose` names what each command draws around its argument.
const ENCLOSURES: &[(&str, &str)] = &[
    (r"\fbox", "box"),
    (r"\boxed", "box"),
    (r"\fcolorbox", "box"),
    (r"\colorbox", "box"),
    (r"\cancel", "updiagonalstrike"),
    (r"\bcancel", "downdiagonalstrike"),
    (r"\xcancel", "updiagonalstrike downdiagonalstrike"),
    (r"\sout", "horizontalstrike"),
    (r"\angl", "actuarial"),
    (r"\phase", "phasorangle"),
];

fn lookup<V: Copy>(table: &[(&str, V)], name: &str) -> Option<V> {
    table.iter().find(|(known, _)| *known == name).map(|(_, value)| *value)
}

/// One MathML element: a token with its text, or a layout with what it arranges.
struct Element {
    name: &'static str,
    attributes: Vec<(&'static str, String)>,
    text: String,
    children: Vec<Element>,
}

impl Element {
    fn token(name: &'static str, text: impl Into<String>) -> Self {
        Self { name, attributes: Vec::new(), text: text.into(), children: Vec::new() }
    }

    fn layout(name: &'static str, children: Vec<Element>) -> Self {
        Self { name, attributes: Vec::new(), text: String::new(), children }
    }

    fn with(mut self, attribute: &'static str, value: impl Into<String>) -> Self {
        self.attributes.push((attribute, value.into()));
        self
    }

    fn write(&self, out: &mut String) {
        let _ = write!(out, "<{}", self.name);
        for (name, value) in &self.attributes {
            let _ = write!(out, " {name}=\"");
            escape(value, out);
            out.push('"');
        }
        if self.text.is_empty() && self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        escape(&self.text, out);
        self.children.iter().for_each(|child| child.write(out));
        let _ = write!(out, "</{}>", self.name);
    }
}

fn escape(text: &str, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // XML has no way to hold the control characters.
            character if character < ' ' => {}
            character => out.push(character),
        }
    }
}

pub(crate) fn write(source: &str, display: bool) -> Result<String, Error> {
    let nodes = Parser::new(source).parse().map_err(|error| parse_error(source, error))?;
    let mut writer = Writer { source, display, variant: None };
    let mut math = Element::layout("math", writer.row(&nodes)?).with("xmlns", "http://www.w3.org/1998/Math/MathML");
    if display {
        math = math.with("display", "block");
    }
    let mut out = String::new();
    math.write(&mut out);
    Ok(out)
}

struct Writer<'s> {
    source: &'s str,
    /// Whether what is being written is set as a display: limits go above and below their operators.
    display: bool,
    /// The `mathvariant` a font command around what is being written asks for.
    variant: Option<&'static str>,
}

impl Writer<'_> {
    /// Writes nodes that follow one another.
    fn row(&mut self, nodes: &[ParseNode]) -> Result<Vec<Element>, Error> {
        let mut out = Vec::new();
        for node in nodes {
            self.node(node, &mut out)?;
        }
        Ok(out)
    }

    /// Writes nodes as the one element an argument has to be.
    fn group(&mut self, nodes: &[ParseNode]) -> Result<Element, Error> {
        let mut row = self.row(nodes)?;
        Ok(if row.len() == 1 { row.remove(0) } else { Element::layout("mrow", row) })
    }

    fn one(&mut self, node: &ParseNode) -> Result<Element, Error> {
        match node {
            ParseNode::OrdGroup { body, .. } => self.group(body),
            node => self.group(std::slice::from_ref(node)),
        }
    }

    /// Writes with one setting changed, and puts it back.
    fn set<T>(&mut self, display: bool, write: impl FnOnce(&mut Self) -> Result<T, Error>) -> Result<T, Error> {
        let outer = std::mem::replace(&mut self.display, display);
        let written = write(self);
        self.display = outer;
        written
    }

    /// A letter or a number, in the face the commands around it ask for.
    fn ordinary(&self, name: &'static str, text: String, upright: bool) -> Element {
        let token = Element::token(name, text);
        match self.variant.or(upright.then_some("normal")) {
            Some(variant) => token.with("mathvariant", variant),
            None => token,
        }
    }

    fn node(&mut self, node: &ParseNode, out: &mut Vec<Element>) -> Result<(), Error> {
        use ParseNode as N;
        match node {
            N::MathOrd { mode, text, .. } => self.letters(character(text, *mode), false, out),
            N::TextOrd { mode, text, .. } => {
                let text = upright(character(text, *mode));
                let digits = text.bytes().all(|byte| byte.is_ascii_digit());
                // The digits of one number, and the point among them, are one token.
                match out.last_mut().filter(|last| last.name == "mn" && (digits || text == ".")) {
                    Some(number) => number.text.push_str(&text),
                    None if digits => out.push(self.ordinary("mn", text, false)),
                    // A prime, a bar and the signs on a keyboard are operators, and the rest are names.
                    None if text
                        .chars()
                        .all(|sign| sign.is_ascii_punctuation() || "\u{2032}\u{2016}".contains(sign)) =>
                    {
                        out.push(Element::token("mo", text));
                    }
                    None => self.letters(text, true, out),
                }
            }
            N::Atom { mode, text, .. } => out.push(Element::token("mo", character(text, *mode))),
            N::SpacingNode { text, .. } => {
                if matches!(text.as_str(), r"\ " | "~" | " " | r"\space" | r"\nobreakspace") {
                    out.push(Element::token("mtext", "\u{a0}"));
                }
            }
            N::Kern { dimension, .. } => out.extend(em(dimension).map(|width| space().with("width", width))),
            N::OrdGroup { body, .. } => out.push(self.group(body)?),
            N::SupSub { base, sup, sub, .. } => self.scripts(base.as_deref(), sub.as_deref(), sup.as_deref(), out)?,
            N::GenFrac { numer, denom, has_bar_line, left_delim, right_delim, .. } => {
                let parts = self.set(false, |writer| Ok(vec![writer.one(numer)?, writer.one(denom)?]))?;
                let mut fraction = Element::layout("mfrac", parts);
                if !has_bar_line {
                    fraction = fraction.with("linethickness", "0");
                }
                out.push(match (left_delim, right_delim) {
                    (None, None) => fraction,
                    (left, right) => {
                        let side = |delim: &Option<String>| delim.as_deref().unwrap_or(".").to_owned();
                        fenced(&side(left), vec![fraction], &side(right))
                    }
                });
            }
            N::Sqrt { body, index, .. } => {
                let body = self.one(body)?;
                out.push(match index {
                    Some(index) => Element::layout("mroot", vec![body, self.set(false, |writer| writer.one(index))?]),
                    None => Element::layout("msqrt", vec![body]),
                });
            }
            N::Accent { label, base, .. } => out.push(self.marked("mover", "accent", base, label)?),
            N::AccentUnder { label, base, .. } => out.push(self.marked("munder", "accentunder", base, label)?),
            N::HorizBrace { label, is_over, base, .. } => {
                let (name, accent) = if *is_over { ("mover", "accent") } else { ("munder", "accentunder") };
                out.push(self.marked(name, accent, base, label)?);
            }
            N::Overline { body, .. } => out.push(self.marked("mover", "accent", body, r"\overlinesegment")?),
            N::Underline { body, .. } => out.push(self.marked("munder", "accentunder", body, r"\underlinesegment")?),
            N::Op { .. } | N::OperatorName { .. } => {
                let (operator, named) = self.operator(node)?;
                out.push(operator);
                out.extend(named.then(|| Element::token("mo", APPLY)));
            }
            N::Font { font, body, .. } => {
                let outer = self.variant;
                self.variant = lookup(VARIANTS, font).or(outer);
                let written = self.one(body);
                self.variant = outer;
                out.push(written?);
            }
            N::Pmb { body, .. } => {
                let outer = self.variant.replace("bold");
                let written = self.group(body);
                self.variant = outer;
                out.push(written?);
            }
            N::Text { body, font, .. } => self.text(body, font.as_deref(), out)?,
            N::Verb { body, .. } => out.push(Element::token("mtext", body.as_str()).with("mathvariant", "monospace")),
            N::Url { url, .. } => out.push(Element::token("mtext", url.as_str())),
            N::Color { color, body, .. } => {
                out.push(Element::layout("mstyle", self.row(body)?).with("mathcolor", color.as_str()));
            }
            N::Styling { style, body, .. } => {
                let display = *style == StyleStr::Display;
                let level = match style {
                    StyleStr::Script => Some("1"),
                    StyleStr::Scriptscript => Some("2"),
                    StyleStr::Display | StyleStr::Text => None,
                };
                let inner = self.set(display, |writer| writer.row(body))?;
                match level {
                    // A style that is already in force says nothing.
                    None if display == self.display => out.extend(inner),
                    None => out.push(Element::layout("mstyle", inner).with("displaystyle", display.to_string())),
                    Some(level) => out.push(
                        Element::layout("mstyle", inner).with("displaystyle", "false").with("scriptlevel", level),
                    ),
                }
            }
            N::MathChoice { display, text, .. } => {
                let chosen = if self.display { display } else { text };
                out.extend(self.row(chosen)?);
            }
            N::MClass { mclass, body, .. } => {
                // A sign made into an operator or a relation is one.
                let sign = match (mclass.as_str(), body.as_slice()) {
                    ("mord", _) => None,
                    (
                        _,
                        [N::TextOrd { mode, text, .. } | N::MathOrd { mode, text, .. } | N::Atom { mode, text, .. }],
                    ) => Some(character(text, *mode)),
                    _ => None,
                };
                out.push(match sign {
                    Some(sign) => Element::token("mo", sign),
                    None => self.group(body)?,
                });
            }
            N::Sizing { body, .. } | N::HBox { body, .. } | N::Html { body, .. } | N::Href { body, .. } => {
                out.extend(self.row(body)?);
            }
            N::HtmlMathMl { mathml, .. } => out.extend(self.row(mathml)?),
            N::DelimSizing { delim, size, .. } => {
                let height = format!("{}em", [1.2, 1.8, 2.4, 3.0][usize::from((*size).clamp(1, 4)) - 1]);
                let delimiter = Element::token("mo", delimiter(delim)).with("minsize", height.clone());
                out.push(delimiter.with("maxsize", height));
            }
            N::LeftRight { body, left, right, .. } => out.push(fenced(left, self.row(body)?, right)),
            N::Middle { delim, .. } => out.push(fence(delim, "infix")),
            N::Rule { width, height, .. } => {
                let mut rule = space().with("mathbackground", "currentColor");
                for (name, measure) in [("width", width), ("height", height)] {
                    rule = match em(measure) {
                        Some(length) => rule.with(name, length),
                        None => rule,
                    };
                }
                out.push(rule);
            }
            N::Phantom { body, .. } => out.push(Element::layout("mphantom", self.row(body)?)),
            N::VPhantom { body, .. } => {
                let hidden = Element::layout("mphantom", vec![self.one(body)?]);
                out.push(Element::layout("mpadded", vec![hidden]).with("width", "0"));
            }
            N::Smash { body, smash_height, smash_depth, .. } => {
                let mut smashed = Element::layout("mpadded", vec![self.one(body)?]);
                for (name, _) in [("height", *smash_height), ("depth", *smash_depth)].into_iter().filter(|(_, on)| *on)
                {
                    smashed = smashed.with(name, "0");
                }
                out.push(smashed);
            }
            N::Lap { body, alignment, .. } => {
                let lapped = Element::layout("mpadded", vec![self.one(body)?]).with("width", "0");
                out.push(match alignment.as_str() {
                    "llap" => lapped.with("lspace", "-1width"),
                    "clap" => lapped.with("lspace", "-0.5width"),
                    _ => lapped,
                });
            }
            N::RaiseBox { body, dy, .. } => {
                let raised = Element::layout("mpadded", vec![self.one(body)?]);
                out.push(match em(dy) {
                    Some(lift) => raised.with("voffset", lift),
                    None => raised,
                });
            }
            N::VCenter { body, .. } => out.push(self.one(body)?),
            N::Enclose { label, body, .. } => {
                let inner = self.one(body)?;
                out.push(match lookup(ENCLOSURES, label) {
                    Some(notation) => Element::layout("menclose", vec![inner]).with("notation", notation),
                    None => inner,
                });
            }
            N::XArrow { label, body, below, .. } => {
                let arrow = lookup(ARROWS, label).unwrap_or('\u{2192}');
                let arrow = Element::token("mo", arrow).with("stretchy", "true");
                let (over, under) = self.set(false, |writer| {
                    Ok((writer.one(body)?, below.as_deref().map(|below| writer.one(below)).transpose()?))
                })?;
                out.push(match under {
                    Some(under) => Element::layout("munderover", vec![arrow, under, over]),
                    None => Element::layout("mover", vec![arrow, over]),
                });
            }
            N::Array { body, cols, col_separation_type, .. } => {
                let mut displayed = false;
                let mut rows = Vec::new();
                for row in body {
                    let mut cells = Vec::new();
                    for cell in row {
                        // Each cell comes in the style its environment sets it in.
                        let (display, inner) = match cell {
                            N::Styling { style, body, .. } => (*style == StyleStr::Display, body.as_slice()),
                            cell => (false, std::slice::from_ref(cell)),
                        };
                        displayed |= display;
                        cells.push(Element::layout("mtd", self.set(display, |writer| writer.row(inner))?));
                    }
                    rows.push(Element::layout("mtr", cells));
                }
                let mut table = Element::layout("mtable", rows);
                let sides = cols.iter().flatten().filter(|col| matches!(col.align_type, AlignType::Align));
                let sides: Vec<&str> = sides
                    .map(|col| match col.align.as_deref() {
                        Some("l") => "left",
                        Some("r") => "right",
                        _ => "center",
                    })
                    .collect();
                if sides.iter().any(|side| *side != "center") {
                    table = table.with("columnalign", sides.join(" "));
                }
                // The two sides of an aligned equation meet, and each pair stands clear of the next.
                if matches!(col_separation_type.as_deref(), Some("align" | "alignat")) && sides.len() > 1 {
                    let gaps: Vec<&str> =
                        (1..sides.len()).map(|gap| if gap % 2 == 1 { "0em" } else { "2em" }).collect();
                    table = table.with("columnspacing", gaps.join(" "));
                }
                if displayed {
                    table = table.with("displaystyle", "true");
                }
                out.push(table);
            }
            N::Tag { body, tag, .. } => {
                out.extend(self.row(body)?);
                out.push(space().with("width", "2em"));
                self.text(tag, None, out)?;
            }
            N::Cr { .. } => out.push(space().with("linebreak", "newline")),
            // What steers the engine and shows nothing.
            N::OpToken { .. }
            | N::AccentToken { .. }
            | N::ColorToken { .. }
            | N::Size { .. }
            | N::LeftRightRight { .. }
            | N::Environment { .. }
            | N::Infix { .. }
            | N::Internal { .. }
            | N::Raw { .. }
            | N::NoNumber { .. } => {}
            N::IncludeGraphics { loc, .. } => {
                return Err(self.unheld("A picture", loc.as_ref().map(|at| at.start..at.end)));
            }
            N::CdLabel { loc, .. } | N::CdLabelParent { loc, .. } | N::CdArrow { loc, .. } => {
                return Err(self.unheld("A commutative diagram", loc.as_ref().map(|at| at.start..at.end)));
            }
            N::ProofTree { loc, .. } => {
                return Err(self.unheld("A proof tree", loc.as_ref().map(|at| at.start..at.end)));
            }
        }
        Ok(())
    }

    /// The error for something MathML has no element for. As with any error, a place that is not in the
    /// source is left out.
    fn unheld(&self, what: &str, span: Option<Range<usize>>) -> Error {
        let inside = |span: &Range<usize>| {
            span.start <= span.end && self.source.is_char_boundary(span.start) && self.source.is_char_boundary(span.end)
        };
        Error { message: format!("{what} cannot be written as MathML"), span: span.filter(inside) }
    }

    /// Adds an identifier. Upright letters that follow one another are one name.
    fn letters(&self, text: String, upright: bool, out: &mut Vec<Element>) {
        let token = self.ordinary("mi", text, upright);
        let variant = token.attributes.first().map(|(_, variant)| variant.as_str());
        let named = variant.is_some_and(|variant| !variant.contains("italic")) && self.variant.is_some();
        match out.last_mut().filter(|last| named && last.name == "mi" && last.attributes == token.attributes) {
            Some(name) => name.text.push_str(&token.text),
            None => out.push(token),
        }
    }

    /// Writes a base with one or both of its scripts.
    fn scripts(
        &mut self,
        base: Option<&ParseNode>,
        sub: Option<&ParseNode>,
        sup: Option<&ParseNode>,
        out: &mut Vec<Element>,
    ) -> Result<(), Error> {
        use ParseNode as N;
        // Limits sit above and below an operator that takes them, and what a brace or an arrow is labelled
        // with sits there always.
        let stacked = match base {
            Some(N::Op { limits, always_handle_sup_sub, .. }) => {
                *limits && (self.display || *always_handle_sup_sub == Some(true))
            }
            Some(N::OperatorName { limits, always_handle_sup_sub, .. }) => {
                *limits && (self.display || *always_handle_sup_sub)
            }
            Some(N::HorizBrace { .. } | N::XArrow { .. }) => true,
            _ => false,
        };
        let (base, named) = match base {
            Some(base @ (N::Op { .. } | N::OperatorName { .. })) => self.operator(base)?,
            Some(base) => (self.one(base)?, false),
            None => (Element::layout("mrow", Vec::new()), false),
        };
        let (sub, sup) = self.set(false, |writer| {
            Ok((sub.map(|sub| writer.one(sub)).transpose()?, sup.map(|sup| writer.one(sup)).transpose()?))
        })?;
        let name = match (stacked, sub.is_some(), sup.is_some()) {
            (false, true, true) => "msubsup",
            (false, true, false) => "msub",
            (false, false, _) => "msup",
            (true, true, true) => "munderover",
            (true, true, false) => "munder",
            (true, false, _) => "mover",
        };
        out.push(Element::layout(name, [Some(base), sub, sup].into_iter().flatten().collect()));
        out.extend(named.then(|| Element::token("mo", APPLY)));
        Ok(())
    }

    /// Writes an operator, and says whether it is a named one, which is applied to what follows it.
    fn operator(&mut self, node: &ParseNode) -> Result<(Element, bool), Error> {
        let name = |name: String| Element::token("mi", name).with("mathvariant", "normal");
        Ok(match node {
            ParseNode::Op { symbol: true, name: Some(sign), mode, .. } => {
                (Element::token("mo", character(sign, *mode)), false)
            }
            ParseNode::Op { name: Some(named), .. } => (name(named.trim_start_matches('\\').to_owned()), true),
            ParseNode::Op { body: Some(body), .. } => (self.group(body)?, false),
            ParseNode::OperatorName { body, .. } => match plain(body) {
                Some(named) => (name(named), true),
                None => {
                    let outer = self.variant.replace("normal");
                    let written = self.group(body);
                    self.variant = outer;
                    (written?, true)
                }
            },
            _ => (Element::layout("mrow", Vec::new()), false),
        })
    }

    /// Writes a base with the mark a command sets over or under it.
    fn marked(
        &mut self,
        name: &'static str,
        accent: &'static str,
        base: &ParseNode,
        label: &str,
    ) -> Result<Element, Error> {
        let mark = match lookup(ACCENTS, label) {
            Some(mark) => mark.to_string(),
            None => label.trim_start_matches('\\').to_owned(),
        };
        let mark = Element::token("mo", mark).with("stretchy", "true");
        Ok(Element::layout(name, vec![self.one(base)?, mark]).with(accent, "true"))
    }

    /// Writes what `\text` holds: its words as text, and whatever maths sits among them as maths.
    fn text(&mut self, body: &[ParseNode], font: Option<&str>, out: &mut Vec<Element>) -> Result<(), Error> {
        let variant = font.and_then(|font| lookup(VARIANTS, font));
        let flush = |words: &mut String, out: &mut Vec<Element>| {
            if !words.is_empty() {
                // A space at either end would not survive XML, so those are the kind that does.
                let inner = words.trim_matches(' ');
                let start = words.len() - words.trim_start_matches(' ').len();
                let end = if inner.is_empty() { 0 } else { words.len() - start - inner.len() };
                let kept = format!("{}{inner}{}", "\u{a0}".repeat(start), "\u{a0}".repeat(end));
                words.clear();
                let text = Element::token("mtext", kept);
                out.push(match variant {
                    Some(variant) => text.with("mathvariant", variant),
                    None => text,
                });
            }
        };
        let mut words = String::new();
        for node in body {
            match node {
                ParseNode::TextOrd { mode, text, .. }
                | ParseNode::MathOrd { mode, text, .. }
                | ParseNode::Atom { mode, text, .. } => words.push_str(&character(text, *mode)),
                ParseNode::SpacingNode { .. } => words.push(' '),
                ParseNode::Text { body, font: inner, .. }
                    if inner.as_deref().and_then(|f| lookup(VARIANTS, f)) == variant =>
                {
                    flush(&mut words, out);
                    self.text(body, font, out)?;
                }
                node => {
                    flush(&mut words, out);
                    self.node(node, out)?;
                }
            }
        }
        flush(&mut words, out);
        Ok(())
    }
}

fn space() -> Element {
    Element::layout("mspace", Vec::new())
}

/// The characters of nodes that are nothing but characters.
fn plain(nodes: &[ParseNode]) -> Option<String> {
    let mut text = String::new();
    for node in nodes {
        match node {
            ParseNode::MathOrd { mode, text: name, .. }
            | ParseNode::TextOrd { mode, text: name, .. }
            | ParseNode::Atom { mode, text: name, .. } => text.push_str(&character(name, *mode)),
            ParseNode::SpacingNode { .. } | ParseNode::Kern { .. } => text.push('\u{2009}'),
            _ => return None,
        }
    }
    Some(text)
}

/// The character a symbol's name stands for. A name the engine has no character for is written as it is.
fn character(name: &str, mode: Mode) -> String {
    let mode = match mode {
        Mode::Math => symbols::Mode::Math,
        Mode::Text => symbols::Mode::Text,
    };
    // The engine keeps a few glyphs of its own where Unicode has no character.
    let known = get_symbol(name, mode).and_then(|symbol| symbol.codepoint);
    match known.filter(|character| !('\u{e000}'..='\u{f8ff}').contains(character)) {
        Some(character) => character.to_string(),
        None => name.to_owned(),
    }
}

/// A bar typed as a bar is one: the engine draws `|` with the sign for "divides".
fn upright(text: String) -> String {
    match text.as_str() {
        "\u{2223}" => "|".into(),
        "\u{2225}" => "\u{2016}".into(),
        _ => text,
    }
}

/// The character of a delimiter. `.` is the delimiter that is not there.
fn delimiter(name: &str) -> String {
    match name {
        "." => String::new(),
        "<" | r"\lt" => "\u{27e8}".into(),
        ">" | r"\gt" => "\u{27e9}".into(),
        name => upright(character(name, Mode::Math)),
    }
}

fn fence(name: &str, form: &'static str) -> Element {
    Element::token("mo", delimiter(name)).with("fence", "true").with("form", form).with("stretchy", "true")
}

/// What `\left` and `\right` hold, between its two delimiters.
fn fenced(left: &str, mut inner: Vec<Element>, right: &str) -> Element {
    inner.insert(0, fence(left, "prefix"));
    inner.push(fence(right, "postfix"));
    Element::layout("mrow", inner)
}

/// A length in em, as MathML writes one. `None` for a unit that has no fixed size in em.
fn em(measure: &Measurement) -> Option<String> {
    let per_em = match measure.unit.as_str() {
        "em" => 1.0,
        "mu" => 1.0 / 18.0,
        "ex" => 0.431,
        "pt" => 0.1,
        _ => return None,
    };
    let length = (measure.number * per_em * 1000.0).round() / 1000.0;
    length.is_finite().then(|| format!("{length}em"))
}
