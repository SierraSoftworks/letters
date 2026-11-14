//! The engine: RaTeX, and the only place that names it. Everything else sees a [`Formula`] or an [`Error`],
//! so another engine can take its place here.

use std::collections::HashMap;
use std::sync::Arc;

use letters_render::{Color, Glyph, Item, PathEl, Point, Rect, Size};
use ratex_font::FontId;
use ratex_layout::{FallbackGlyphMetrics, LayoutOptions};
use ratex_parser::ParseError;
use ratex_parser::parser::Parser;
use ratex_types::display_item::DisplayItem;
use ratex_types::math_style::MathStyle;
use ratex_types::path_command::PathCommand;

use crate::{Error, Formula, Style, fonts};

/// The width, in em, of a stroked line such as `\cancel`'s: the thickness of TeX's rules.
const STROKE: f32 = 0.04;

/// The thinnest a rule is drawn, in logical pixels. A fraction bar at body size would otherwise be a grey
/// blur, so KaTeX holds its rules to the same floor.
const HAIRLINE: f32 = 1.0;

/// The most dashes one dashed rule is cut into. A formula can ask for a rule of any length.
const MAX_DASHES: usize = 1024;

pub(crate) fn typeset(source: &str, style: &Style) -> Result<Formula, Error> {
    let nodes = Parser::new(source).parse().map_err(|error| parse_error(source, error))?;

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
        ratex_layout::layout(&nodes, &options)
    } else {
        ratex_layout::layout_with_fallback_metrics(&nodes, &options, Arc::new(body))
    };
    // The engine's guard against a small source that asks for a vast drawing.
    ratex_layout::validate_layout(&tree).map_err(|message| Error { message, span: None })?;
    let list = ratex_layout::to_display_list(&tree);

    let em = if style.size.is_finite() { style.size.max(0.0) } else { 0.0 };
    let px = |length: f64| (length * f64::from(em)) as f32;
    let size = Size { width: px(list.width), height: px(list.height + list.depth) };
    if !size.width.is_finite() || !size.height.is_finite() {
        return Err(Error { message: "The formula is too large".into(), span: None });
    }

    let mut items = Vec::new();
    for item in list.items {
        match item {
            DisplayItem::GlyphPath { x, y, scale, font, char_code, color } => {
                let face = FontId::parse(&font).unwrap_or(FontId::MainRegular);
                let character = ratex_font::katex_ttf_glyph_char(face, char_code);
                let Some((font, id)) = fonts::glyph(face.as_str(), character) else { continue };
                glyph(&mut items, font, id, em * scale as f32, color.into_ink(), Point { x: px(x), y: px(y) });
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
    }
    Ok(Formula { size, baseline: px(list.height), items })
}

/// The engine counts positions in bytes of the source, as the document does. A position that is not in the
/// source (the engine can report one from inside a macro) is left out.
fn parse_error(source: &str, error: ParseError) -> Error {
    let span = error.loc.map(|loc| loc.start..loc.end).filter(|span| {
        span.start <= span.end && source.is_char_boundary(span.start) && source.is_char_boundary(span.end)
    });
    Error { message: error.message.replace("'EOF'", "the end of the formula"), span }
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

/// Adds a glyph, to the run before it when that is in the same face, size and colour.
fn glyph(items: &mut Vec<Item>, font: letters_render::Font, id: u32, size: f32, color: Color, at: Point) {
    if let Some(Item::Glyphs { origin, font: run_font, size: run_size, color: run_color, glyphs, .. }) =
        items.last_mut()
        && *run_font == font
        && *run_size == size
        && *run_color == color
    {
        glyphs.push(Glyph { id, x: at.x - origin.x, y: at.y - origin.y });
        return;
    }
    let glyphs = vec![Glyph { id, x: 0.0, y: 0.0 }];
    items.push(Item::Glyphs { origin: at, font, size, color, skew: 0.0, embolden: false, glyphs });
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
