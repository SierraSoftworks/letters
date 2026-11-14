//! The document style that `style.json` stores: one [`TextStyle`] per element type, plus page setup.
//!
//! This is the first cut of the shape. Reading and writing `style.json` is still to come.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// Keyed by element type: `title`, `heading1` to `heading4`, `body`, `quote`, `code`, `caption`.
    pub types: BTreeMap<String, TextStyle>,
    pub page: Page,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub family: String,
    pub weight: u16,
    pub italic: bool,
    /// In points.
    pub size: f32,
    /// As `#rrggbb`.
    pub color: String,
    pub align: Align,
    /// A multiple of the font size.
    pub line_height: f32,
    /// In points.
    pub space_before: f32,
    /// In points.
    pub space_after: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Justify,
}

/// Paper size and margins, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            family: "Inter".into(),
            weight: 400,
            italic: false,
            size: 11.0,
            color: "#1f2328".into(),
            align: Align::Start,
            line_height: 1.5,
            space_before: 0.0,
            space_after: 8.0,
        }
    }
}

impl Default for Page {
    /// A4 with 25 mm margins.
    fn default() -> Self {
        Self {
            width: 595.0,
            height: 842.0,
            margin_top: 71.0,
            margin_right: 71.0,
            margin_bottom: 71.0,
            margin_left: 71.0,
        }
    }
}

impl Default for Style {
    fn default() -> Self {
        let body = TextStyle::default();
        let heading =
            |size: f32| TextStyle { weight: 600, size, line_height: 1.25, space_before: size, ..body.clone() };
        let types = [
            ("title", TextStyle { weight: 700, size: 28.0, line_height: 1.2, ..body.clone() }),
            ("heading1", heading(22.0)),
            ("heading2", heading(17.0)),
            ("heading3", heading(14.0)),
            ("heading4", heading(12.0)),
            ("body", body.clone()),
            ("quote", TextStyle { italic: true, ..body.clone() }),
            ("code", TextStyle { family: "JetBrains Mono".into(), size: 10.0, ..body.clone() }),
            ("caption", TextStyle { size: 9.0, ..body.clone() }),
        ];
        Self { types: types.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), page: Page::default() }
    }
}
