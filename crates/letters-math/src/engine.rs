//! The engine: RaTeX, and the only place that names it. Everything else sees a [`Formula`] or an [`Error`],
//! so another engine can take its place here.

use std::collections::HashMap;
use std::mem;
use std::ops::Range;
use std::sync::Arc;

use letters_render::{Color, Glyph, Item, PathEl, Point, Rect, Size};
use ratex_font::FontId;
use ratex_layout::{FallbackGlyphMetrics, LayoutOptions};
use ratex_parser::ParseError;
use ratex_parser::parse_node::{ArrayTag, ParseNode, ProofBranch};
use ratex_parser::parser::Parser;
use ratex_types::display_item::{DisplayItem, DisplayList};
use ratex_types::math_style::MathStyle;
use ratex_types::path_command::PathCommand;

use crate::{Error, Formula, Part, Style, fonts, parts};

/// The width, in em, of a stroked line such as `\cancel`'s: the thickness of TeX's rules.
const STROKE: f32 = 0.04;

/// The thinnest a rule is drawn, in logical pixels. A fraction bar at body size would otherwise be a grey
/// blur, so KaTeX holds its rules to the same floor.
const HAIRLINE: f32 = 1.0;

/// The most dashes one dashed rule is cut into. A formula can ask for a rule of any length.
const MAX_DASHES: usize = 1024;

pub(crate) fn typeset(source: &str, style: &Style) -> Result<Formula, Error> {
    let nodes = parse(source, style)?;
    draw(set(source, &nodes, style)?, style, &[])
}

/// As [`typeset`], with the parts of the formula that can be traced to their source.
pub(crate) fn typeset_mapped(source: &str, style: &Style) -> Result<Formula, Error> {
    let nodes = parse(source, style)?;
    let list = set(source, &nodes, style)?;
    let sources = sources(source, nodes, &list, style).unwrap_or_default();
    draw(list, style, &sources)
}

fn parse(source: &str, style: &Style) -> Result<Vec<ParseNode>, Error> {
    let mut nodes = Parser::new(source).parse().map_err(|error| parse_error(source, error))?;
    if !style.links {
        unlink(&mut nodes);
    }
    Ok(nodes)
}

/// Lays a parsed formula out, as the list of what the engine draws.
fn set(source: &str, nodes: &[ParseNode], style: &Style) -> Result<DisplayList, Error> {
    let options = LayoutOptions {
        style: if style.display { MathStyle::Display } else { MathStyle::Text },
        color: ratex_types::Color::new(
            f32::from(style.color.r) / 255.0,
            f32::from(style.color.g) / 255.0,
            f32::from(style.color.b) / 255.0,
            f32::from(style.color.a) / 255.0,
        ),
        ..LayoutOptions::default()
    };
    let body = body_metrics(source);
    let tree = if body.is_empty() {
        ratex_layout::layout(nodes, &options)
    } else {
        ratex_layout::layout_with_fallback_metrics(nodes, &options, Arc::new(body))
    };
    // The engine's guard against a small source that asks for a vast drawing.
    ratex_layout::validate_layout(&tree).map_err(|message| Error { message, span: None })?;
    Ok(ratex_layout::to_display_list(&tree))
}

/// For each thing in `list`, the bytes of the source it was set from, where that can be told. `None` when
/// nothing can.
///
/// The engine's drawing keeps no positions, so the formula is laid out again with each part in a colour of
/// its own, and a part is whatever comes out in its colour.
fn sources(
    source: &str,
    mut nodes: Vec<ParseNode>,
    list: &DisplayList,
    style: &Style,
) -> Option<Vec<Option<Range<usize>>>> {
    // The same source, one byte along. A position that moves with it is a place in the source. One that
    // stays is a place in a macro of the engine's own, which only looks like one.
    let mut moved = parse(&format!(" {source}"), style).ok()?;
    let parts = parts::mark(source, &mut nodes, &mut moved)?;
    let marked = set(source, &moved, style).ok()?;
    // A mark must change nothing but colours, or its drawing is another formula's.
    let alike = marked.items.len() == list.items.len() && marked.items.iter().zip(&list.items).all(|(a, b)| same(a, b));
    alike.then(|| marked.items.iter().zip(&list.items).map(|(marked, real)| parts.source(marked, real)).collect())
}

/// Whether two things are drawn alike but for their colour.
fn same(a: &DisplayItem, b: &DisplayItem) -> bool {
    use DisplayItem as D;
    match (a, b) {
        (
            D::GlyphPath { x, y, scale, font, char_code, color: _ },
            D::GlyphPath { x: x2, y: y2, scale: scale2, font: font2, char_code: code2, color: _ },
        ) => (x, y, scale, font, char_code) == (x2, y2, scale2, font2, code2),
        (
            D::Line { x, y, width, thickness, dashed, color: _ },
            D::Line { x: x2, y: y2, width: width2, thickness: thickness2, dashed: dashed2, color: _ },
        ) => (x, y, width, thickness, dashed) == (x2, y2, width2, thickness2, dashed2),
        (
            D::Rect { x, y, width, height, color: _ },
            D::Rect { x: x2, y: y2, width: width2, height: height2, color: _ },
        ) => (x, y, width, height) == (x2, y2, width2, height2),
        (
            D::Path { x, y, commands, fill, color: _ },
            D::Path { x: x2, y: y2, commands: commands2, fill: fill2, color: _ },
        ) => (x, y, commands, fill) == (x2, y2, commands2, fill2),
        _ => false,
    }
}

/// Turns what the engine draws into a formula. `sources` says where in the source each thing in `list` is
/// from, and is empty for a formula that is not traced.
fn draw(list: DisplayList, style: &Style, sources: &[Option<Range<usize>>]) -> Result<Formula, Error> {
    let em = if style.size.is_finite() { style.size.max(0.0) } else { 0.0 };
    let px = |length: f64| (length * f64::from(em)) as f32;
    let size = Size { width: px(list.width), height: px(list.height + list.depth) };
    if !size.width.is_finite() || !size.height.is_finite() {
        return Err(Error { message: "The formula is too large".into(), span: None });
    }

    let (mut items, mut parts) = (Vec::new(), Vec::<Part>::new());
    let mut before = &None;
    for (index, item) in list.items.into_iter().enumerate() {
        let source = sources.get(index).unwrap_or(&None);
        let from = items.len();
        match item {
            DisplayItem::GlyphPath { x, y, scale, font, char_code, color } => {
                let face = FontId::parse(&font).unwrap_or(FontId::MainRegular);
                let character = ratex_font::katex_ttf_glyph_char(face, char_code);
                let Some((font, id)) = fonts::glyph(face.as_str(), character) else { continue };
                // A run holds one part, so that a part is whole items.
                let run = if source == before { items.last_mut() } else { None };
                let glyph = (font, id, em * scale as f32, color.into_ink(), Point { x: px(x), y: px(y) });
                if let Some(new) = glyph_in(run, glyph) {
                    items.push(new);
                }
            }
            // A line is placed by its vertical centre.
            DisplayItem::Line { x, y, width, thickness, color, dashed } => {
                let rule = Rect::new(px(x), px(y - thickness / 2.0), px(width), px(thickness));
                if dashed {
                    dashes(&mut items, rule, color.into_ink());
                } else {
                    fill(&mut items, rule, color.into_ink());
                }
            }
            DisplayItem::Rect { x, y, width, height, color } => {
                fill(&mut items, Rect::new(px(x), px(y), px(width), px(height)), color.into_ink());
            }
            DisplayItem::Path { x, y, commands, fill, color } => {
                let origin = Point { x: px(x), y: px(y) };
                let color = color.into_ink();
                if fill {
                    // Each outline is filled alone: the engine's shapes overlap in places (the shaft and
                    // the head of an arrow), which even-odd filling would otherwise punch out.
                    for outline in commands.chunk_by(|_, next| !matches!(next, PathCommand::MoveTo { .. })) {
                        items.push(Item::Path { origin, scale: em, path: path(outline), color, stroke: None });
                    }
                } else if em > 0.0 {
                    let stroke = Some(STROKE.max(HAIRLINE / em));
                    items.push(Item::Path { origin, scale: em, path: path(&commands), color, stroke });
                }
            }
        }
        before = source;
        let Some(source) = source.as_ref().filter(|_| items.len() > from) else { continue };
        match parts.last_mut() {
            Some(part) if part.source == *source && part.items.end == from => part.items.end = items.len(),
            _ => parts.push(Part { source: source.clone(), items: from..items.len() }),
        }
    }
    Ok(Formula { size, baseline: px(list.height), items, parts })
}

/// The engine counts positions in bytes of the source, as the document does. One from inside a macro of the
/// engine's own is a position in that macro, which only looks like one in the source, and is left out.
pub(crate) fn parse_error(source: &str, error: ParseError) -> Error {
    let span = match &error.loc {
        Some(loc) => Some(loc.start..loc.end).filter(|span| written(source, span)),
        // What is missing is missing at the end.
        None => error.message.ends_with("'EOF'").then_some(source.len()..source.len()),
    };
    Error { message: error.message.replace("'EOF'", "the end of the formula"), span }
}

/// Whether the place of an error is a place in the source: only such a place moves when the source does.
fn written(source: &str, span: &Range<usize>) -> bool {
    let moved = Parser::new(&format!(" {source}")).parse().err().and_then(|error| error.loc);
    source.get(span.clone()).is_some()
        && moved.is_some_and(|moved| (moved.start, moved.end) == (span.start + 1, span.end + 1))
}

/// Sets every link as its plain argument: the text of `\href` and the address of `\url`, drawn as what is
/// around them is.
fn unlink(nodes: &mut [ParseNode]) {
    // A stack, since the source decides how deep the tree goes.
    let mut pending: Vec<&mut ParseNode> = nodes.iter_mut().collect();
    while let Some(node) = pending.pop() {
        if let ParseNode::Href { mode, body, loc, .. } = node {
            *node = ParseNode::OrdGroup { mode: *mode, body: mem::take(body), semisimple: None, loc: loc.take() };
        }
        children(node, &mut pending);
    }
}

/// Adds the nodes directly inside `node`. Every kind of node is named, so that a kind new to the engine
/// cannot hide a link: it does not compile until it has a place here.
pub(crate) fn children<'a>(node: &'a mut ParseNode, out: &mut Vec<&'a mut ParseNode>) {
    use ParseNode as N;
    match node {
        N::Atom { .. }
        | N::MathOrd { .. }
        | N::TextOrd { .. }
        | N::OpToken { .. }
        | N::AccentToken { .. }
        | N::SpacingNode { .. }
        | N::ColorToken { .. }
        | N::Size { .. }
        | N::DelimSizing { .. }
        | N::LeftRightRight { .. }
        | N::Middle { .. }
        | N::Rule { .. }
        | N::Kern { .. }
        | N::Cr { .. }
        | N::Infix { .. }
        | N::Internal { .. }
        | N::Verb { .. }
        | N::Url { .. }
        | N::Raw { .. }
        | N::NoNumber { .. }
        | N::IncludeGraphics { .. } => {}
        N::OrdGroup { body, .. }
        | N::OperatorName { body, .. }
        | N::Text { body, .. }
        | N::Color { body, .. }
        | N::Styling { body, .. }
        | N::Sizing { body, .. }
        | N::LeftRight { body, .. }
        | N::Phantom { body, .. }
        | N::MClass { body, .. }
        | N::Href { body, .. }
        | N::HBox { body, .. }
        | N::Pmb { body, .. }
        | N::Html { body, .. } => out.extend(body),
        N::Op { body, .. } => out.extend(body.iter_mut().flatten()),
        N::Font { body, .. }
        | N::Overline { body, .. }
        | N::Underline { body, .. }
        | N::VPhantom { body, .. }
        | N::Smash { body, .. }
        | N::Enclose { body, .. }
        | N::Lap { body, .. }
        | N::RaiseBox { body, .. }
        | N::VCenter { body, .. }
        | N::Accent { base: body, .. }
        | N::AccentUnder { base: body, .. }
        | N::HorizBrace { base: body, .. }
        | N::Environment { name_group: body, .. }
        | N::CdLabel { label: body, .. }
        | N::CdLabelParent { fragment: body, .. } => out.push(body),
        N::GenFrac { numer, denom, .. } => out.extend([&mut **numer, &mut **denom]),
        // In the order they are written: the one in square brackets comes first.
        N::Sqrt { body, index: other, .. } | N::XArrow { body, below: other, .. } => {
            out.extend(other.as_deref_mut());
            out.push(body);
        }
        N::SupSub { base, sup, sub, .. } => out.extend([base, sup, sub].into_iter().filter_map(Option::as_deref_mut)),
        N::CdArrow { label_above, label_below, .. } => {
            out.extend([label_above, label_below].into_iter().filter_map(Option::as_deref_mut));
        }
        N::MathChoice { display, text, script, scriptscript, .. } => {
            out.extend([display, text, script, scriptscript].into_iter().flatten());
        }
        N::Tag { body: first, tag: second, .. } | N::HtmlMathMl { html: first, mathml: second, .. } => {
            out.extend([first, second].into_iter().flatten());
        }
        N::Array { body, tags, .. } => {
            out.extend(body.iter_mut().flatten());
            for tag in tags.iter_mut().flatten() {
                match tag {
                    ArrayTag::Explicit(nodes) => out.extend(nodes),
                    ArrayTag::Auto(_) => {}
                }
            }
        }
        N::ProofTree { tree, .. } => {
            let mut branches = vec![tree];
            while let Some(ProofBranch { conclusion, premises, left_label, right_label, .. }) = branches.pop() {
                out.extend(conclusion);
                out.extend([left_label, right_label].into_iter().flatten().flatten());
                branches.extend(premises);
            }
        }
    }
}

/// Measurements for the characters of the source that only the body face draws.
fn body_metrics(source: &str) -> HashMap<char, FallbackGlyphMetrics> {
    let foreign = source.chars().filter(|character| !character.is_ascii());
    foreign
        .filter_map(|character| {
            let [width, height, depth] = fonts::body_metrics(character)?;
            Some((character, FallbackGlyphMetrics { width, height, depth }))
        })
        .collect()
}

/// Adds a glyph to `run` when that is a run in the same face, size and colour. Otherwise gives a run of
/// its own for it.
fn glyph_in(
    run: Option<&mut Item>,
    (font, id, size, color, at): (letters_render::Font, u32, f32, Color, Point),
) -> Option<Item> {
    if let Some(Item::Glyphs { origin, font: run_font, size: run_size, color: run_color, glyphs, .. }) = run
        && *run_font == font
        && *run_size == size
        && *run_color == color
    {
        glyphs.push(Glyph { id, x: at.x - origin.x, y: at.y - origin.y });
        return None;
    }
    let glyphs = vec![Glyph { id, x: 0.0, y: 0.0 }];
    Some(Item::Glyphs { origin: at, font, size, color, skew: 0.0, embolden: false, glyphs, text: None })
}

/// Adds a filled rectangle. A side thinner than a hairline grows to one about its centre.
fn fill(items: &mut Vec<Item>, rect: Rect, color: Color) {
    // Written so that a side that is not a number is dropped too.
    let visible = rect.width > 0.0 && rect.height > 0.0;
    if !visible {
        return;
    }
    let (width, height) = (rect.width.max(HAIRLINE), rect.height.max(HAIRLINE));
    let rect = Rect::new(rect.x - (width - rect.width) / 2.0, rect.y - (height - rect.height) / 2.0, width, height);
    items.push(Item::Fill { rect, color, radius: 0.0 });
}

/// Adds a horizontal rule as dashes four times as long as it is thick, with gaps to match.
fn dashes(items: &mut Vec<Item>, rule: Rect, color: Color) {
    let dash = rule.height.max(HAIRLINE) * 4.0;
    let count = (rule.width / (dash * 2.0)).ceil().clamp(0.0, MAX_DASHES as f32) as usize;
    for index in 0..count {
        let start = index as f32 * dash * 2.0;
        let width = dash.min(rule.width - start);
        fill(items, Rect::new(rule.x + start, rule.y, width, rule.height), color);
    }
}

fn path(commands: &[PathCommand]) -> Arc<[PathEl]> {
    let point = |x: f64, y: f64| Point { x: x as f32, y: y as f32 };
    let element = |command: &PathCommand| match *command {
        PathCommand::MoveTo { x, y } => PathEl::MoveTo(point(x, y)),
        PathCommand::LineTo { x, y } => PathEl::LineTo(point(x, y)),
        PathCommand::QuadTo { x1, y1, x, y } => PathEl::QuadTo(point(x1, y1), point(x, y)),
        PathCommand::CubicTo { x1, y1, x2, y2, x, y } => PathEl::CurveTo(point(x1, y1), point(x2, y2), point(x, y)),
        PathCommand::Close => PathEl::Close,
    };
    commands.iter().map(element).collect()
}

trait IntoInk {
    fn into_ink(self) -> Color;
}

impl IntoInk for ratex_types::Color {
    fn into_ink(self) -> Color {
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color::rgba(channel(self.r), channel(self.g), channel(self.b), channel(self.a))
    }
}
