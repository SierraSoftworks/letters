//! `document.json`: strict to read, canonical to write.
//!
//! Key order and layout are fixed, so one document always produces the same bytes. Unknown fields and unknown
//! block types are kept and written back.

use std::collections::BTreeSet;
use std::fmt;

use serde_json::{Map, Value};

use crate::{Block, BlockId, BlockKind, Body, BodyShape, Document, Extra, FORMAT, ListKind, Mark, Run, Tone};

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
        map.insert("blocks".into(), self.blocks.iter().map(write_block).collect());
        write_extra(&mut map, &self.extra);
        let mut out = serde_json::to_string_pretty(&Value::Object(map)).expect("a JSON value always serialises");
        out.push('\n');
        out
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
    let blocks = match read {
        Some(blocks) => blocks,
        None => blocks(map.remove("blocks").unwrap_or(Value::Array(Vec::new())), "blocks")?,
    };

    let doc = Document { language, dictionary, blocks, extra: map.into_iter().collect() };
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
fn shape(type_name: &str) -> Option<BodyShape> {
    match type_name {
        "paragraph" | "heading" | "list_item" | "code" => Some(BodyShape::Text),
        "quote" | "callout" => Some(BodyShape::Blocks),
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
        "quote" => BlockKind::Quote,
        "callout" => match string(&mut map, "tone", path)?.as_deref() {
            Some("tip") => BlockKind::Callout { tone: Tone::Tip },
            Some("info") => BlockKind::Callout { tone: Tone::Info },
            Some("warn") => BlockKind::Callout { tone: Tone::Warn },
            Some("danger") => BlockKind::Callout { tone: Tone::Danger },
            _ => return err(&format!("{path}.tone"), "expected tip, info, warn or danger"),
        },
        "divider" => BlockKind::Divider,
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
    Ok(Block { id, kind, body, extra: map.into_iter().collect() })
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
            let marks = marks.into_iter().map(|name| read_mark(&name)).collect();
            let link = string(&mut map, "link", &path)?;
            Ok(Run { text, marks, link, extra: map.into_iter().collect() })
        })
        .collect()
}

const MARKS: [(&str, Mark); 7] = [
    ("math", Mark::Math),
    ("bold", Mark::Bold),
    ("italic", Mark::Italic),
    ("underline", Mark::Underline),
    ("strike", Mark::Strike),
    ("highlight", Mark::Highlight),
    ("code", Mark::Code),
];

pub(crate) fn read_mark(name: &str) -> Mark {
    MARKS.iter().find(|(n, _)| *n == name).map_or_else(|| Mark::Other(name.into()), |(_, mark)| mark.clone())
}

pub(crate) fn mark_name(mark: &Mark) -> &str {
    match mark {
        Mark::Other(name) => name,
        known => MARKS.iter().find(|(_, m)| m == known).map(|(name, _)| *name).expect("every known mark is named"),
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
        BlockKind::Unknown { type_name } => type_name,
        BlockKind::Figure { .. } => "figure",
        BlockKind::Gallery { .. } => "gallery",
        BlockKind::Caption => "caption",
        BlockKind::Formula => "formula",
        BlockKind::Table { .. } => "table",
        BlockKind::TableRow => "table_row",
        BlockKind::TableCell => "table_cell",
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
    write_extra(&mut map, &block.extra);
    Value::Object(map)
}

fn write_run(run: &Run) -> Value {
    let mut map = Map::new();
    map.insert("text".into(), run.text.as_str().into());
    if !run.marks.is_empty() {
        map.insert("marks".into(), run.marks.iter().map(mark_name).collect());
    }
    if let Some(link) = &run.link {
        map.insert("link".into(), link.as_str().into());
    }
    write_extra(&mut map, &run.extra);
    Value::Object(map)
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
