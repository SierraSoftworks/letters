//! The notes that `notes.json` stores: comments, import notes and conflicts, each anchored to a passage.
//!
//! All three are one [`Note`] that differs only in its [`NoteKind`]. Reading is strict and writing is canonical,
//! as for `document.json`, and unknown fields and note types are kept and written back.

use std::collections::BTreeSet;
use std::ops::Range;

use serde_json::{Map, Value};

use crate::json::{Result, err, object, string, write_extra};
use crate::{Block, BlockId, Extra, Run};

/// The `notes.json` format version this build reads and writes.
pub const NOTES_FORMAT: u32 = 1;

/// Every note on a document, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notes {
    pub notes: Vec<Note>,
    pub extra: Extra,
}

/// Something anchored to a passage that waits in the margin for attention.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// Unique within the document.
    pub id: String,
    pub kind: NoteKind,
    pub anchor: Anchor,
    /// The name shown beside the note: a person, or `Letters import` and the like.
    pub author: String,
    /// RFC 3339.
    pub created: String,
    pub resolved: bool,
    /// What the note says, in the same runs as document text. Empty for a conflict.
    pub content: Vec<Run>,
    pub replies: Vec<Reply>,
    pub extra: Extra,
}

#[derive(Clone, Debug, PartialEq)]
pub enum NoteKind {
    /// A thread written by people.
    Comment,
    /// What an import simplified at this passage.
    Import,
    /// Two versions of one block that were changed apart. Picking one resolves the note.
    Conflict { versions: Box<[Version; 2]> },
    /// A type this build does not know. Its fields sit in [`Note::extra`] and are written back verbatim.
    Unknown { type_name: String },
}

/// One side of a conflict.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    pub author: String,
    /// When this side was last edited, as RFC 3339.
    pub edited: String,
    pub block: Block,
    pub extra: Extra,
}

/// The passage a note sits on.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub block: BlockId,
    /// UTF-8 bytes into the block's text. `None` anchors the whole block.
    pub range: Option<Range<usize>>,
    pub extra: Extra,
}

impl Anchor {
    pub fn block(block: impl Into<BlockId>) -> Self {
        Self { block: block.into(), range: None, extra: Extra::new() }
    }

    pub fn range(block: impl Into<BlockId>, range: Range<usize>) -> Self {
        Self { block: block.into(), range: Some(range), extra: Extra::new() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub author: String,
    /// RFC 3339.
    pub created: String,
    pub content: Vec<Run>,
    pub extra: Extra,
}

impl Notes {
    /// Reads a `notes.json`, refusing anything that breaks the schema.
    pub fn from_json(json: &str) -> Result<Self> {
        let value: Value = serde_json::from_str(json).or_else(|e| err("", format!("not valid JSON: {e}")))?;
        let mut map = object(value, "")?;
        match map.remove("format").as_ref().and_then(Value::as_u64) {
            Some(format) if format == u64::from(NOTES_FORMAT) => {}
            Some(format) if format > u64::from(NOTES_FORMAT) => {
                return err("format", format!("version {format} is newer than this build reads ({NOTES_FORMAT})"));
            }
            _ => return err("format", "expected a format version"),
        }

        let notes = list(map.remove("notes"), "notes", "notes", read_note)?;
        let mut seen = BTreeSet::new();
        if let Some(note) = notes.iter().find(|note| !seen.insert(&note.id)) {
            return err("notes", format!("the id \"{}\" is used by more than one note", note.id));
        }
        Ok(Self { notes, extra: map.into_iter().collect() })
    }

    /// Writes the canonical `notes.json`.
    pub fn to_json(&self) -> String {
        let mut map = Map::new();
        map.insert("format".into(), NOTES_FORMAT.into());
        map.insert("notes".into(), self.notes.iter().map(write_note).collect());
        write_extra(&mut map, &self.extra);
        let mut out = serde_json::to_string_pretty(&Value::Object(map)).expect("a JSON value always serialises");
        out.push('\n');
        out
    }
}

/// Reads an optional list, giving each item its path.
fn list<T>(value: Option<Value>, path: &str, what: &str, read: impl Fn(Value, &str) -> Result<T>) -> Result<Vec<T>> {
    match value {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => {
            items.into_iter().enumerate().map(|(i, item)| read(item, &format!("{path}[{i}]"))).collect()
        }
        Some(_) => err(path, format!("expected a list of {what}")),
    }
}

/// A string that must be present and not empty.
fn required(map: &mut Map<String, Value>, key: &str, path: &str, what: &str) -> Result<String> {
    match string(map, key, path)? {
        Some(text) if !text.is_empty() => Ok(text),
        _ => err(&format!("{path}.{key}"), format!("expected {what}")),
    }
}

fn content(map: &mut Map<String, Value>, path: &str) -> Result<Vec<Run>> {
    Run::list_from_value(map.remove("content").unwrap_or(Value::Array(Vec::new())), &format!("{path}.content"))
}

fn read_note(value: Value, path: &str) -> Result<Note> {
    let mut map = object(value, path)?;
    let id = required(&mut map, "id", path, "an id")?;
    let type_name = required(&mut map, "type", path, "a type")?;
    let kind = match type_name.as_str() {
        "comment" => NoteKind::Comment,
        "import" => NoteKind::Import,
        "conflict" => {
            let at = format!("{path}.versions");
            let versions: Result<[Version; 2]> = list(map.remove("versions"), &at, "versions", read_version)?
                .try_into()
                .or_else(|_| err(&at, "expected the two versions of the block"));
            NoteKind::Conflict { versions: Box::new(versions?) }
        }
        _ => NoteKind::Unknown { type_name },
    };
    let anchor = match map.remove("anchor") {
        Some(anchor) => read_anchor(anchor, &format!("{path}.anchor"))?,
        None => return err(&format!("{path}.anchor"), "every note needs an anchor"),
    };
    let author = required(&mut map, "author", path, "an author")?;
    let created = required(&mut map, "created", path, "a creation time")?;
    let resolved = match map.remove("resolved") {
        None => false,
        Some(Value::Bool(resolved)) => resolved,
        Some(_) => return err(&format!("{path}.resolved"), "expected true or false"),
    };
    let content = content(&mut map, path)?;
    let replies = list(map.remove("replies"), &format!("{path}.replies"), "replies", read_reply)?;
    Ok(Note { id, kind, anchor, author, created, resolved, content, replies, extra: map.into_iter().collect() })
}

fn read_anchor(value: Value, path: &str) -> Result<Anchor> {
    let mut map = object(value, path)?;
    let block = BlockId::new(required(&mut map, "block", path, "the id of a block")?);
    let offset = |map: &mut Map<String, Value>, key: &str| match map.remove(key) {
        None => Ok(None),
        Some(value) => match value.as_u64().and_then(|n| usize::try_from(n).ok()) {
            Some(offset) => Ok(Some(offset)),
            None => err(&format!("{path}.{key}"), "expected an offset in UTF-8 bytes"),
        },
    };
    let range = match (offset(&mut map, "start")?, offset(&mut map, "end")?) {
        (None, None) => None,
        (Some(start), Some(end)) if start <= end => Some(start..end),
        (Some(_), Some(_)) => return err(&format!("{path}.end"), "the end comes before the start"),
        _ => return err(path, "expected both start and end, or neither"),
    };
    Ok(Anchor { block, range, extra: map.into_iter().collect() })
}

fn read_version(value: Value, path: &str) -> Result<Version> {
    let mut map = object(value, path)?;
    let author = required(&mut map, "author", path, "an author")?;
    let edited = required(&mut map, "edited", path, "an edit time")?;
    let block = match map.remove("block") {
        Some(block) => Block::from_value(block, &format!("{path}.block"))?,
        None => return err(&format!("{path}.block"), "every version needs its block"),
    };
    Ok(Version { author, edited, block, extra: map.into_iter().collect() })
}

fn read_reply(value: Value, path: &str) -> Result<Reply> {
    let mut map = object(value, path)?;
    let author = required(&mut map, "author", path, "an author")?;
    let created = required(&mut map, "created", path, "a creation time")?;
    let content = content(&mut map, path)?;
    Ok(Reply { author, created, content, extra: map.into_iter().collect() })
}

fn write_note(note: &Note) -> Value {
    let mut map = Map::new();
    map.insert("id".into(), note.id.as_str().into());
    let type_name = match &note.kind {
        NoteKind::Comment => "comment",
        NoteKind::Import => "import",
        NoteKind::Conflict { .. } => "conflict",
        NoteKind::Unknown { type_name } => type_name,
    };
    map.insert("type".into(), type_name.into());

    let mut anchor = Map::new();
    anchor.insert("block".into(), note.anchor.block.as_str().into());
    if let Some(range) = &note.anchor.range {
        anchor.insert("start".into(), range.start.into());
        anchor.insert("end".into(), range.end.into());
    }
    write_extra(&mut anchor, &note.anchor.extra);
    map.insert("anchor".into(), anchor.into());
    map.insert("author".into(), note.author.as_str().into());
    map.insert("created".into(), note.created.as_str().into());
    map.insert("resolved".into(), note.resolved.into());
    map.insert("content".into(), Run::list_to_value(&note.content));
    if let NoteKind::Conflict { versions } = &note.kind {
        map.insert("versions".into(), versions.iter().map(write_version).collect());
    }
    if !note.replies.is_empty() {
        map.insert("replies".into(), note.replies.iter().map(write_reply).collect());
    }
    write_extra(&mut map, &note.extra);
    Value::Object(map)
}

fn write_version(version: &Version) -> Value {
    let mut map = Map::new();
    map.insert("author".into(), version.author.as_str().into());
    map.insert("edited".into(), version.edited.as_str().into());
    map.insert("block".into(), version.block.to_value());
    write_extra(&mut map, &version.extra);
    Value::Object(map)
}

fn write_reply(reply: &Reply) -> Value {
    let mut map = Map::new();
    map.insert("author".into(), reply.author.as_str().into());
    map.insert("created".into(), reply.created.as_str().into());
    map.insert("content".into(), Run::list_to_value(&reply.content));
    write_extra(&mut map, &reply.extra);
    Value::Object(map)
}
