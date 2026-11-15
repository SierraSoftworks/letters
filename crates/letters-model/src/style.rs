//! The document style that `style.json` stores: one [`TextStyle`] per element type, plus page setup.
//!
//! A `.letstyle` stylesheet is the same JSON in a file of its own. Reading is strict and fills anything missing
//! from the built-in style, and writing is complete and canonical, so a document looks the same everywhere.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::Extra;
use crate::json::{Result, err, object, write_extra};

/// The `style.json` format version this build reads and writes.
pub const STYLE_FORMAT: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    /// Keyed by element type: `title`, `heading1` to `heading4`, `body`, `quote`, `code`, `caption`.
    pub types: BTreeMap<String, TextStyle>,
    pub page: Page,
    pub extra: Extra,
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
    pub extra: Extra,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Justify,
}

pub(crate) const ALIGNS: [(&str, Align); 4] =
    [("start", Align::Start), ("center", Align::Center), ("end", Align::End), ("justify", Align::Justify)];

/// Paper size and margins, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub margin_top: f32,
    pub margin_right: f32,
    pub margin_bottom: f32,
    pub margin_left: f32,
    pub extra: Extra,
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
            extra: Extra::new(),
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
            extra: Extra::new(),
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
            ("code", TextStyle { family: "Fira Code".into(), size: 10.0, ..body.clone() }),
            ("caption", TextStyle { size: 9.0, ..body.clone() }),
        ];
        Self {
            types: types.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            page: Page::default(),
            extra: Extra::new(),
        }
    }
}

impl Style {
    /// The element types every style defines, in the order they are listed and written.
    pub const TYPES: [&str; 9] =
        ["title", "heading1", "heading2", "heading3", "heading4", "body", "quote", "code", "caption"];

    /// Reads a `style.json` or a `.letstyle` file, refusing anything that breaks the schema.
    ///
    /// A missing type or field takes its value from the built-in style, so the result is always complete.
    pub fn from_json(json: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(json).or_else(|e| err("", format!("not valid JSON: {e}")))?;
        let mut map = object(value, "")?;
        match map.remove("format").as_ref().and_then(Value::as_u64) {
            Some(format) if format == u64::from(STYLE_FORMAT) => {}
            Some(format) if format > u64::from(STYLE_FORMAT) => {
                return err("format", format!("version {format} is newer than this build reads ({STYLE_FORMAT})"));
            }
            _ => return err("format", "expected a format version"),
        }

        let mut style = Self::default();
        if let Some(types) = map.remove("types") {
            for (name, value) in object(types, "types")? {
                let base = style.types.get(&name).unwrap_or(&style.types["body"]).clone();
                let read = read_text_style(value, base, &format!("types.{name}"))?;
                style.types.insert(name, read);
            }
        }
        if let Some(page) = map.remove("page") {
            style.page = read_page(page)?;
        }
        style.extra = map.into_iter().collect();
        Ok(style)
    }

    /// Writes the canonical `style.json`: every type with every field.
    pub fn to_json(&self) -> String {
        let known = Self::TYPES.iter().filter_map(|name| Some((*name, self.types.get(*name)?)));
        let other = self.types.iter().filter(|(name, _)| !Self::TYPES.contains(&name.as_str()));
        let types: Map<String, Value> = known
            .chain(other.map(|(name, style)| (name.as_str(), style)))
            .map(|(name, style)| (name.to_owned(), write_text_style(style)))
            .collect();

        let mut map = Map::new();
        map.insert("format".into(), STYLE_FORMAT.into());
        map.insert("types".into(), types.into());
        map.insert("page".into(), write_page(&self.page));
        write_extra(&mut map, &self.extra);
        let mut out = serde_json::to_string_pretty(&Value::Object(map)).expect("a JSON value always serialises");
        out.push('\n');
        out
    }
}

/// Takes `key` out of `map` as a number in `allowed`, leaving `into` alone when the key is absent.
fn number(
    map: &mut Map<String, Value>,
    key: &str,
    path: &str,
    allowed: impl Fn(f32) -> bool,
    expected: &str,
    into: &mut f32,
) -> Result<()> {
    let Some(value) = map.remove(key) else { return Ok(()) };
    match value.as_f64().map(|n| n as f32) {
        Some(n) if n.is_finite() && allowed(n) => *into = n,
        _ => return err(&format!("{path}.{key}"), format!("expected {expected}")),
    }
    Ok(())
}

fn positive(n: f32) -> bool {
    n > 0.0
}

fn not_negative(n: f32) -> bool {
    n >= 0.0
}

fn read_text_style(value: Value, mut style: TextStyle, path: &str) -> Result<TextStyle> {
    let mut map = object(value, path)?;
    let at = |key: &str| format!("{path}.{key}");
    match map.remove("family") {
        None => {}
        Some(Value::String(family)) if !family.trim().is_empty() => style.family = family,
        Some(_) => return err(&at("family"), "expected a font family name"),
    }
    match map.remove("weight") {
        None => {}
        Some(weight) => match weight.as_u64() {
            Some(weight @ 1..=1000) => style.weight = weight as u16,
            _ => return err(&at("weight"), "expected a weight from 1 to 1000"),
        },
    }
    match map.remove("italic") {
        None => {}
        Some(Value::Bool(italic)) => style.italic = italic,
        Some(_) => return err(&at("italic"), "expected true or false"),
    }
    match map.remove("color") {
        None => {}
        Some(Value::String(color)) if is_color(&color) => style.color = color.to_ascii_lowercase(),
        Some(_) => return err(&at("color"), "expected a colour as #rrggbb"),
    }
    match map.remove("align") {
        None => {}
        Some(align) => match ALIGNS.iter().find(|(name, _)| align.as_str() == Some(name)) {
            Some((_, align)) => style.align = *align,
            None => return err(&at("align"), "expected start, center, end or justify"),
        },
    }
    number(&mut map, "size", path, positive, "a size above 0, in points", &mut style.size)?;
    number(&mut map, "line_height", path, positive, "a multiple of the font size above 0", &mut style.line_height)?;
    number(&mut map, "space_before", path, not_negative, "a space of 0 or more, in points", &mut style.space_before)?;
    number(&mut map, "space_after", path, not_negative, "a space of 0 or more, in points", &mut style.space_after)?;
    style.extra = map.into_iter().collect();
    Ok(style)
}

fn is_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_page(value: Value) -> Result<Page> {
    let mut map = object(value, "page")?;
    let mut page = Page::default();
    number(&mut map, "width", "page", positive, "a width above 0, in points", &mut page.width)?;
    number(&mut map, "height", "page", positive, "a height above 0, in points", &mut page.height)?;
    let margin = "a margin of 0 or more, in points";
    number(&mut map, "margin_top", "page", not_negative, margin, &mut page.margin_top)?;
    number(&mut map, "margin_right", "page", not_negative, margin, &mut page.margin_right)?;
    number(&mut map, "margin_bottom", "page", not_negative, margin, &mut page.margin_bottom)?;
    number(&mut map, "margin_left", "page", not_negative, margin, &mut page.margin_left)?;
    if page.margin_left + page.margin_right >= page.width || page.margin_top + page.margin_bottom >= page.height {
        return err("page", "the margins leave no room for text");
    }
    page.extra = map.into_iter().collect();
    Ok(page)
}

/// Whole numbers are written without a fraction, and the rest as the shortest decimal that reads back the same.
pub(crate) fn write_number(n: f32) -> Value {
    if n.fract() == 0.0 && n.abs() < 16_777_216.0 {
        Value::from(n as i64)
    } else {
        Value::from(n.to_string().parse::<f64>().unwrap_or_default())
    }
}

fn write_text_style(style: &TextStyle) -> Value {
    let align = ALIGNS.iter().find(|(_, align)| *align == style.align).map(|(name, _)| *name);
    let mut map = Map::new();
    map.insert("family".into(), style.family.as_str().into());
    map.insert("weight".into(), style.weight.into());
    map.insert("italic".into(), style.italic.into());
    map.insert("size".into(), write_number(style.size));
    map.insert("color".into(), style.color.as_str().into());
    map.insert("align".into(), align.expect("every alignment is named").into());
    map.insert("line_height".into(), write_number(style.line_height));
    map.insert("space_before".into(), write_number(style.space_before));
    map.insert("space_after".into(), write_number(style.space_after));
    write_extra(&mut map, &style.extra);
    Value::Object(map)
}

fn write_page(page: &Page) -> Value {
    let mut map = Map::new();
    map.insert("width".into(), write_number(page.width));
    map.insert("height".into(), write_number(page.height));
    map.insert("margin_top".into(), write_number(page.margin_top));
    map.insert("margin_right".into(), write_number(page.margin_right));
    map.insert("margin_bottom".into(), write_number(page.margin_bottom));
    map.insert("margin_left".into(), write_number(page.margin_left));
    write_extra(&mut map, &page.extra);
    Value::Object(map)
}
