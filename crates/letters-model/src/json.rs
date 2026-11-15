//! `document.json`: strict to read, canonical to write.
//!
//! Key order and layout are fixed, so one document always produces the same bytes. A block is written as
//! its id, its type and that type's fields, then its body, then every other field in key order. Unknown
//! fields and unknown block types are kept and written back.
//!
//! A gallery, a table and a row hold only their own parts, and a file that nests them otherwise is
//! refused. A row may hold any number of cells, since two people changing one table can leave it so.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::{Map, Value};

use crate::style::{ALIGNS, write_number};
use crate::{
    Block, BlockId, BlockKind, Body, BodyShape, Document, Extra, FORMAT, ListKind, Mark, Properties, PropertiesFormat,
    Run, Tone,
};

mod stream;

/// Why a `document.json` was refused, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// The offending value, such as `blocks[2].level`. Empty for the document itself.
    pub path: String,
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() { f.write_str(&self.message) } else { write!(f, "{}: {}", self.path, self.message) }
    }
}

impl std::error::Error for JsonError {}

pub(crate) type Result<T> = std::result::Result<T, JsonError>;

pub(crate) fn err<T>(path: &str, message: impl Into<String>) -> Result<T> {
    Err(JsonError { path: path.into(), message: message.into() })
}

impl Document {
    /// Reads a `document.json`, refusing anything that breaks the schema.
    pub fn from_json(json: &str) -> Result<Self> {
        // A sound file is read in one pass. Anything that pass declines is read again the slow way, which
        // can say what is wrong and where.
        if let Some(doc) = stream::document(json) {
            return Ok(doc);
        }
        let value: Value = serde_json::from_str(json).or_else(|e| err("", format!("not valid JSON: {e}")))?;
        document(object(value, "")?, None)
    }

    /// Writes the canonical `document.json`.
    pub fn to_json(&self) -> String {
        let mut map = Map::new();
        map.insert("format".into(), FORMAT.into());
        map.insert("language".into(), self.language.as_str().into());
        if !self.dictionary.is_empty() {
            map.insert("dictionary".into(), self.dictionary.iter().cloned().collect());
        }
        if let Some(properties) = &self.properties {
            map.insert("properties".into(), properties.to_value());
        }
        map.insert("blocks".into(), self.blocks.iter().map(write_block).collect());
        write_extra(&mut map, &self.extra);
        let mut out = serde_json::to_string_pretty(&Value::Object(map)).expect("a JSON value always serialises");
        out.push('\n');
        out
    }
}

impl Properties {
    /// Reads a document's properties as `document.json` stores them. `path` names them in errors.
    pub fn from_value(value: Value, path: &str) -> Result<Self> {
        let mut map = object(value, path)?;
        let format = match string(&mut map, "format", path)?.as_deref() {
            Some("yaml") => PropertiesFormat::Yaml,
            Some("toml") => PropertiesFormat::Toml,
            _ => return err(&format!("{path}.format"), "expected yaml or toml"),
        };
        let Some(source) = string(&mut map, "source", path)? else {
            return err(&format!("{path}.source"), "expected the properties as text");
        };
        Ok(Self { format, source, extra: map.into_iter().collect() })
    }

    /// The properties as `document.json` stores them.
    pub fn to_value(&self) -> Value {
        let mut map = Map::new();
        let format = match self.format {
            PropertiesFormat::Yaml => "yaml",
            PropertiesFormat::Toml => "toml",
        };
        map.insert("format".into(), format.into());
        map.insert("source".into(), self.source.as_str().into());
        write_extra(&mut map, &self.extra);
        Value::Object(map)
    }
}

impl Block {
    /// Reads one block as `document.json` stores it. `path` names the block in errors.
    pub fn from_value(value: Value, path: &str) -> Result<Self> {
        block(value, path)
    }

    /// The block as `document.json` stores it.
    pub fn to_value(&self) -> Value {
        write_block(self)
    }
}

impl Run {
    /// Reads a list of runs as a block's `content` stores it. `path` names the list in errors.
    pub fn list_from_value(value: Value, path: &str) -> Result<Vec<Self>> {
        runs(value, path)
    }

    /// A list of runs as a block's `content` stores it.
    pub fn list_to_value(runs: &[Self]) -> Value {
        runs.iter().map(write_run).collect()
    }
}

/// Builds a document from its fields. `read` holds the blocks when they were read on the way in.
fn document(mut map: Map<String, Value>, read: Option<Vec<Block>>) -> Result<Document> {
    match map.remove("format").as_ref().and_then(Value::as_u64) {
        Some(format) if format == u64::from(FORMAT) => {}
        Some(format) if format > u64::from(FORMAT) => {
            return err("format", format!("version {format} is newer than this build reads ({FORMAT})"));
        }
        _ => return err("format", "expected a format version"),
    }

    let language = match map.remove("language") {
        None => Document::default().language,
        Some(Value::String(tag)) => tag,
        Some(_) => return err("language", "expected a BCP 47 tag"),
    };
    let dictionary =
        strings(map.remove("dictionary"), "dictionary")?.unwrap_or_default().into_iter().collect::<BTreeSet<_>>();
    let properties = map.remove("properties").map(|value| Properties::from_value(value, "properties")).transpose()?;
    let blocks = match read {
        Some(blocks) => blocks,
        None => blocks(map.remove("blocks").unwrap_or(Value::Array(Vec::new())), "blocks")?,
    };

    let doc = Document { language, dictionary, properties, blocks, extra: map.into_iter().collect() };
    let mut seen = BTreeSet::new();
    for block in doc.walk() {
        if !seen.insert(&block.id) {
            return err("blocks", format!("the id \"{}\" is used by more than one block", block.id));
        }
    }
    Ok(doc)
}

pub(crate) fn object(value: Value, path: &str) -> Result<Map<String, Value>> {
    match value {
        Value::Object(map) => Ok(map),
        _ => err(path, "expected an object"),
    }
}

pub(crate) fn string(map: &mut Map<String, Value>, key: &str, path: &str) -> Result<Option<String>> {
    match map.remove(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => err(&format!("{path}.{key}"), "expected a string"),
    }
}

/// Takes a `lang` out of a block's or a run's fields.
pub(crate) fn lang(map: &mut Map<String, Value>, path: &str) -> Result<Option<String>> {
    match string(map, "lang", path)? {
        Some(tag) if tag.is_empty() => err(&format!("{path}.lang"), "expected a BCP 47 tag"),
        tag => Ok(tag),
    }
}

/// Takes a flag that is left out when it is off.
fn flag(map: &mut Map<String, Value>, key: &str, path: &str) -> Result<bool> {
    match map.remove(key) {
        None => Ok(false),
        Some(Value::Bool(on)) => Ok(on),
        Some(_) => err(&format!("{path}.{key}"), "expected true or false"),
    }
}

/// Takes one side of a figure's size.
fn size(map: &mut Map<String, Value>, key: &str, path: &str) -> Result<f32> {
    match map.remove(key).as_ref().and_then(Value::as_f64).map(|size| size as f32) {
        Some(size) if size.is_finite() && size > 0.0 => Ok(size),
        _ => err(&format!("{path}.{key}"), "expected a size above 0, in points"),
    }
}

fn strings(value: Option<Value>, path: &str) -> Result<Option<Vec<String>>> {
    let Some(value) = value else { return Ok(None) };
    let Value::Array(items) = value else { return err(path, "expected a list of strings") };
    items
        .into_iter()
        .map(|item| match item {
            Value::String(s) => Ok(s),
            _ => err(path, "expected a list of strings"),
        })
        .collect::<Result<_>>()
        .map(Some)
}

fn blocks(value: Value, path: &str) -> Result<Vec<Block>> {
    let Value::Array(items) = value else { return err(path, "expected a list of blocks") };
    items.into_iter().enumerate().map(|(i, item)| block(item, &format!("{path}[{i}]"))).collect()
}

fn block(value: Value, path: &str) -> Result<Block> {
    block_from(object(value, path)?, None, path)
}

/// The shape of body a block of this type holds, for the types this build knows.
pub(crate) fn shape(type_name: &str) -> Option<BodyShape> {
    match type_name {
        "paragraph" | "heading" | "list_item" | "code" | "formula" => Some(BodyShape::Text),
        "caption" | "figure" | "table_cell" => Some(BodyShape::Text),
        "quote" | "callout" | "gallery" | "table" | "table_row" => Some(BodyShape::Blocks),
        _ => None,
    }
}

/// Builds a block from its fields. `read` holds its body when that was read on the way in.
fn block_from(mut map: Map<String, Value>, read: Option<Body>, path: &str) -> Result<Block> {
    let id = match string(&mut map, "id", path)? {
        Some(id) if !id.is_empty() => BlockId::new(id),
        _ => return err(&format!("{path}.id"), "every block needs an id"),
    };
    let Some(type_name) = string(&mut map, "type", path)? else {
        return err(&format!("{path}.type"), "every block needs a type");
    };

    let kind = match type_name.as_str() {
        "paragraph" => BlockKind::Paragraph,
        "heading" => match map.remove("level").as_ref().and_then(Value::as_u64) {
            Some(level @ 1..=4) => BlockKind::Heading { level: level as u8 },
            _ => return err(&format!("{path}.level"), "expected a level from 1 to 4"),
        },
        "list_item" => {
            let checked = match map.remove("checked") {
                None => false,
                Some(Value::Bool(checked)) => checked,
                Some(_) => return err(&format!("{path}.checked"), "expected true or false"),
            };
            let list = match string(&mut map, "list", path)?.as_deref() {
                Some("bullet") => ListKind::Bullet,
                Some("ordered") => ListKind::Ordered,
                Some("task") => ListKind::Task { checked },
                _ => return err(&format!("{path}.list"), "expected bullet, ordered or task"),
            };
            let depth = match map.remove("depth") {
                None => 0,
                Some(depth) => match depth.as_u64().and_then(|d| u8::try_from(d).ok()) {
                    Some(depth) => depth,
                    None => return err(&format!("{path}.depth"), "expected a depth from 0 to 255"),
                },
            };
            BlockKind::ListItem { list, depth }
        }
        "code" => BlockKind::Code { language: string(&mut map, "language", path)? },
        "formula" => BlockKind::Formula,
        "quote" => BlockKind::Quote,
        "callout" => match string(&mut map, "tone", path)?.as_deref() {
            Some("tip") => BlockKind::Callout { tone: Tone::Tip },
            Some("info") => BlockKind::Callout { tone: Tone::Info },
            Some("warn") => BlockKind::Callout { tone: Tone::Warn },
            Some("danger") => BlockKind::Callout { tone: Tone::Danger },
            _ => return err(&format!("{path}.tone"), "expected tip, info, warn or danger"),
        },
        "divider" => BlockKind::Divider,
        "caption" => BlockKind::Caption,
        "figure" => {
            let (asset, src) = (string(&mut map, "asset", path)?, string(&mut map, "src", path)?);
            if asset.is_some() == src.is_some() {
                return err(path, "a figure names its picture as an asset or as a src, and not both");
            }
            let alt = string(&mut map, "alt", path)?.unwrap_or_default();
            let (width, height) = (size(&mut map, "width", path)?, size(&mut map, "height", path)?);
            BlockKind::Figure { asset, src, alt, width, height, numbered: flag(&mut map, "numbered", path)? }
        }
        "gallery" => BlockKind::Gallery { numbered: flag(&mut map, "numbered", path)? },
        "table" => {
            let at = format!("{path}.columns");
            let names = strings(map.remove("columns"), &at)?.unwrap_or_default();
            let align = |name: &String| ALIGNS.iter().find(|(known, _)| known == name).map(|(_, align)| *align);
            match names.iter().map(align).collect() {
                Some(columns) => BlockKind::Table { columns, header: flag(&mut map, "header", path)? },
                None => return err(&at, "expected start, center, end or justify for each column"),
            }
        }
        "table_row" => BlockKind::TableRow,
        "table_cell" => BlockKind::TableCell,
        _ => BlockKind::Unknown { type_name },
    };

    let body = match (kind.shape(), read) {
        (BodyShape::Text, Some(body @ Body::Text(_))) | (BodyShape::Blocks, Some(body @ Body::Blocks(_))) => body,
        (_, Some(_)) => return err(path, "the body does not fit the block's type"),
        (BodyShape::None, None) => Body::None,
        (BodyShape::Blocks, None) => {
            Body::Blocks(blocks(map.remove("blocks").unwrap_or(Value::Array(Vec::new())), &format!("{path}.blocks"))?)
        }
        (BodyShape::Text, None) => {
            Body::Text(runs(map.remove("content").unwrap_or(Value::Array(Vec::new())), &format!("{path}.content"))?)
        }
    };
    if let Some(stray) = body.blocks().iter().position(|child| !kind.admits(&child.kind)) {
        let holds = match kind {
            BlockKind::Gallery { .. } => "a gallery holds figures and captions",
            BlockKind::Table { .. } => "a table holds rows",
            _ => "a row holds cells",
        };
        return err(&format!("{path}.blocks[{stray}]"), holds);
    }
    let lang = lang(&mut map, path)?;
    let page_break = match map.remove("break") {
        None => false,
        Some(Value::String(at)) if at == "page" => true,
        Some(_) => return err(&format!("{path}.break"), "expected page"),
    };
    Ok(Block { id, kind, body, lang, page_break, extra: map.into_iter().collect() })
}

fn runs(value: Value, path: &str) -> Result<Vec<Run>> {
    let Value::Array(items) = value else { return err(path, "expected a list of runs") };
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| {
            let path = format!("{path}[{i}]");
            let mut map = object(item, &path)?;
            let Some(text) = string(&mut map, "text", &path)? else {
                return err(&format!("{path}.text"), "every run needs text");
            };
            let marks = strings(map.remove("marks"), &format!("{path}.marks"))?.unwrap_or_default();
            let mut marks: BTreeSet<Mark> = marks.into_iter().map(|name| read_mark(&name)).collect();
            let link = string(&mut map, "link", &path)?;
            let lang = lang(&mut map, &path)?;
            match string(&mut map, "ref", &path)? {
                Some(target) if target.is_empty() => return err(&format!("{path}.ref"), "expected a block id"),
                target => marks.extend(target.map(|target| Mark::Ref(BlockId::new(target)))),
            }
            Ok(Run { text, marks, link, lang, extra: map.into_iter().collect() })
        })
        .collect()
}

const MARKS: [(&str, Mark); 7] = [
    ("bold", Mark::Bold),
    ("italic", Mark::Italic),
    ("underline", Mark::Underline),
    ("strike", Mark::Strike),
    ("highlight", Mark::Highlight),
    ("code", Mark::Code),
    ("math", Mark::Math),
];

pub(crate) fn read_mark(name: &str) -> Mark {
    MARKS.iter().find(|(n, _)| *n == name).map_or_else(|| Mark::Other(name.into()), |(_, mark)| mark.clone())
}

/// The name a mark goes by in a run's `marks`. `None` for a reference, which is a field of its own.
pub(crate) fn mark_name(mark: &Mark) -> Option<&str> {
    match mark {
        Mark::Other(name) => Some(name),
        known => MARKS.iter().find(|(_, m)| m == known).map(|(name, _)| *name),
    }
}

fn write_block(block: &Block) -> Value {
    let mut map = Map::new();
    map.insert("id".into(), block.id.as_str().into());
    let type_name = match &block.kind {
        BlockKind::Paragraph => "paragraph",
        BlockKind::Heading { level } => {
            map.insert("level".into(), (*level).into());
            "heading"
        }
        BlockKind::ListItem { list, depth } => {
            let name = match list {
                ListKind::Bullet => "bullet",
                ListKind::Ordered => "ordered",
                ListKind::Task { .. } => "task",
            };
            map.insert("list".into(), name.into());
            if let ListKind::Task { checked } = list {
                map.insert("checked".into(), (*checked).into());
            }
            if *depth > 0 {
                map.insert("depth".into(), (*depth).into());
            }
            "list_item"
        }
        BlockKind::Code { language } => {
            if let Some(language) = language {
                map.insert("language".into(), language.as_str().into());
            }
            "code"
        }
        BlockKind::Formula => "formula",
        BlockKind::Quote => "quote",
        BlockKind::Callout { tone } => {
            let tone = match tone {
                Tone::Tip => "tip",
                Tone::Info => "info",
                Tone::Warn => "warn",
                Tone::Danger => "danger",
            };
            map.insert("tone".into(), tone.into());
            "callout"
        }
        BlockKind::Divider => "divider",
        BlockKind::Caption => "caption",
        BlockKind::Figure { asset, src, alt, width, height, numbered } => {
            for (key, name) in [("asset", asset), ("src", src)] {
                if let Some(name) = name {
                    map.insert(key.into(), name.as_str().into());
                }
            }
            map.insert("alt".into(), alt.as_str().into());
            map.insert("width".into(), write_number(*width));
            map.insert("height".into(), write_number(*height));
            write_flag(&mut map, "numbered", *numbered);
            "figure"
        }
        BlockKind::Gallery { numbered } => {
            write_flag(&mut map, "numbered", *numbered);
            "gallery"
        }
        BlockKind::Table { columns, header } => {
            let name = |align| ALIGNS.iter().find(|(_, known)| known == align).map(|(name, _)| *name);
            map.insert("columns".into(), columns.iter().map(name).collect());
            write_flag(&mut map, "header", *header);
            "table"
        }
        BlockKind::TableRow => "table_row",
        BlockKind::TableCell => "table_cell",
        BlockKind::Unknown { type_name } => type_name,
    };
    // `type` reads best straight after `id`, ahead of the fields that depend on it.
    map.shift_insert(1, "type".into(), type_name.into());

    match &block.body {
        Body::Text(runs) => {
            map.insert("content".into(), runs.iter().map(write_run).collect());
        }
        Body::Blocks(blocks) => {
            map.insert("blocks".into(), blocks.iter().map(write_block).collect());
        }
        Body::None => {}
    }
    let page_break = block.page_break.then_some(("break", "page"));
    write_rest(&mut map, [block.lang.as_deref().map(|lang| ("lang", lang)), page_break], &block.extra);
    Value::Object(map)
}

/// Writes a flag that is left out when it is off.
fn write_flag(map: &mut Map<String, Value>, key: &str, on: bool) {
    if on {
        map.insert(key.into(), true.into());
    }
}

fn write_run(run: &Run) -> Value {
    let mut map = Map::new();
    map.insert("text".into(), run.text.as_str().into());
    let marks: Vec<&str> = run.marks.iter().filter_map(mark_name).collect();
    if !marks.is_empty() {
        map.insert("marks".into(), marks.into());
    }
    if let Some(link) = &run.link {
        map.insert("link".into(), link.as_str().into());
    }
    let target = run.marks.iter().find_map(|mark| match mark {
        Mark::Ref(target) => Some(("ref", target.as_str())),
        _ => None,
    });
    write_rest(&mut map, [run.lang.as_deref().map(|lang| ("lang", lang)), target], &run.extra);
    Value::Object(map)
}

/// Appends the fields any block or run may carry, among its unknown fields and in key order with them. A
/// field of that name among the unknown ones gives way, and an empty one is left out.
fn write_rest(map: &mut Map<String, Value>, fields: [Option<(&str, &str)>; 2], extra: &Extra) {
    let fields = fields.into_iter().flatten().filter(|(_, value)| !value.is_empty());
    if fields.clone().next().is_none() {
        return write_extra(map, extra);
    }
    let mut all = extra.clone();
    all.extend(fields.map(|(key, value)| (key.to_owned(), value.into())));
    write_extra(map, &all);
}

/// Appends unknown fields in key order, with the keys of any nested object sorted too.
pub(crate) fn write_extra(map: &mut Map<String, Value>, extra: &Extra) {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                Value::Object(keys.into_iter().map(|k| (k.clone(), sorted(&map[k]))).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    for (key, value) in extra {
        map.insert(key.clone(), sorted(value));
    }
}
