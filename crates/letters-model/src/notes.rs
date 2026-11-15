//! The notes that `notes.json` stores: comments, import notes and conflicts, each anchored to a passage.
//!
//! All three are one [`Note`] that differs only in its [`NoteKind`]. Reading is strict and writing is canonical,
//! as for `document.json`, and unknown fields and note types are kept and written back.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::json::{Result, err, object, string, write_extra};
use crate::{Block, BlockId, Extra, Hold, Model, Pos, Run, StablePos};

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
    /// Two versions of one block that were changed apart, the later edit last. Picking one resolves the note.
    Conflict { versions: Box<[Version; 2]> },
    /// A type this build does not know. Its fields sit in [`Note::extra`] and are written back verbatim.
    Unknown { type_name: String },
}

/// One side of a conflict.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    /// Empty when the writer gave no name.
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
    /// The two ends of the range as positions that stay with their text. The block and range above say where
    /// those are in the `document.json` beside them, for readers that know nothing of the history.
    pub stable: Option<(StablePos, StablePos)>,
    pub extra: Extra,
}

impl Note {
    /// The note a merge raises for a block that two writers changed in the same words. `versions` are the
    /// block as each left it, the later edit last.
    ///
    /// Everything in it, the id too, comes from the block and the two versions alone, so every device that
    /// merges the same pair raises the same note.
    pub fn conflict(block: impl Into<BlockId>, versions: [Version; 2]) -> Self {
        let block = block.into();
        let mut hash = Sha256::new();
        hash.update(block.as_str());
        for version in &versions {
            hash.update([0]);
            hash.update(write_version(version).to_string());
        }
        let id: String = hash.finalize()[..16].iter().map(|byte| format!("{byte:02x}")).collect();
        Self {
            id: format!("conflict-{id}"),
            created: versions[1].edited.clone(),
            kind: NoteKind::Conflict { versions: Box::new(versions) },
            anchor: Anchor::block(block),
            author: "Letters".into(),
            resolved: false,
            content: Vec::new(),
            replies: Vec::new(),
            extra: Extra::new(),
        }
    }
}

impl Anchor {
    pub fn block(block: impl Into<BlockId>) -> Self {
        Self { block: block.into(), range: None, stable: None, extra: Extra::new() }
    }

    pub fn range(block: impl Into<BlockId>, range: Range<usize>) -> Self {
        Self { block: block.into(), range: Some(range), stable: None, extra: Extra::new() }
    }

    /// Ties the range to the text it covers in `model`, so that it can [`follow`](Self::follow) that text.
    /// Leaves alone a whole block, a range that is already tied and one that is not in the document.
    pub fn pin(&mut self, model: &Model) {
        if let (None, Some(range)) = (&self.stable, &self.range) {
            let start = model.stable(&Pos::new(self.block.clone(), range.start), Hold::Next);
            let end = model.stable(&Pos::new(self.block.clone(), range.end), Hold::Prev);
            self.stable = start.zip(end);
        }
    }

    /// Moves the block and range to where the text is now, after edits or a merge. A passage that a split
    /// spread over several blocks keeps the part in its first block.
    pub fn follow(&mut self, model: &Model) {
        let Some((start, end)) = &self.stable else { return };
        let (Some(start), Some(end)) = (model.resolve(start), model.resolve(end)) else { return };
        let end = if end.block == start.block {
            end.offset.max(start.offset)
        } else {
            model.doc().block(&start.block).map_or(start.offset, |block| block.plain_text().len())
        };
        self.range = Some(start.offset..end);
        self.block = start.block;
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

impl Notes {
    /// Ties every anchor to the text it covers in `model`. See [`Anchor::pin`].
    pub fn pin(&mut self, model: &Model) {
        self.notes.iter_mut().for_each(|note| note.anchor.pin(model));
    }

    /// Moves every anchor to where its text is now. See [`Anchor::follow`].
    pub fn follow(&mut self, model: &Model) {
        self.notes.iter_mut().for_each(|note| note.anchor.follow(model));
    }

    /// Takes in the notes of another copy of the document. Returns whether anything here changed.
    ///
    /// The result depends only on the two sets of notes, never on which side merges or in what order, so every
    /// copy settles on the same `notes.json` and merging again changes nothing. A note is never lost: one held
    /// by both sides stays resolved if either resolved it and keeps the replies of both.
    pub fn merge(&mut self, other: &Notes) -> bool {
        let before = self.clone();
        let mut notes: BTreeMap<String, Note> = BTreeMap::new();
        for note in std::mem::take(&mut self.notes).into_iter().chain(other.notes.iter().cloned()) {
            let joined = match notes.remove(&note.id) {
                Some(held) => join(held, note),
                None => note,
            };
            notes.insert(joined.id.clone(), joined);
        }
        self.notes = notes.into_values().map(settle).collect();
        self.notes.sort_by(|a, b| (&a.created, &a.id).cmp(&(&b.created, &b.id)));
        for (key, value) in &other.extra {
            // Values have no order of their own, so their JSON text stands in for one.
            let (held, theirs) = (self.extra.get(key).map(Value::to_string), value.to_string());
            if held.is_none_or(|held| held < theirs) {
                self.extra.insert(key.clone(), value.clone());
            }
        }
        *self != before
    }
}

/// One note from two copies of it. Where they differ beyond replies and resolution there is no telling which
/// is newer, so the choice is arbitrary but the same on every device.
fn join(a: Note, b: Note) -> Note {
    let body = |note: &Note| write_note(&Note { resolved: false, replies: Vec::new(), ..note.clone() }).to_string();
    let resolved = a.resolved || b.resolved;
    let (mut kept, other) = if body(&a) >= body(&b) { (a, b) } else { (b, a) };
    kept.resolved = resolved;
    kept.replies.extend(other.replies);
    kept
}

/// Puts a note's replies in an order that depends only on what they are, dropping repeats.
fn settle(mut note: Note) -> Note {
    let key = |reply: &Reply| (reply.created.clone(), write_reply(reply).to_string());
    note.replies.sort_by_cached_key(key);
    note.replies.dedup();
    note
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
    let mut stable = |key: &str| match string(&mut map, key, path)?.map(|text| text.parse::<StablePos>()) {
        None => Ok(None),
        Some(Ok(stable)) => Ok(Some(stable)),
        Some(Err(_)) => err(&format!("{path}.{key}"), "expected a stable position"),
    };
    let stable = match (stable("stable_start")?, stable("stable_end")?) {
        (Some(start), Some(end)) if range.is_some() => Some((start, end)),
        (None, None) => None,
        _ => return err(path, "expected stable_start and stable_end beside a start and end, or neither"),
    };
    Ok(Anchor { block, range, stable, extra: map.into_iter().collect() })
}

fn read_version(value: Value, path: &str) -> Result<Version> {
    let mut map = object(value, path)?;
    // A writer who gave no name is still a side of the conflict.
    let Some(author) = string(&mut map, "author", path)? else {
        return err(&format!("{path}.author"), "expected an author");
    };
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
    if let Some((start, end)) = &note.anchor.stable {
        anchor.insert("stable_start".into(), start.to_string().into());
        anchor.insert("stable_end".into(), end.to_string().into());
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
