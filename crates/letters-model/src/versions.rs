//! The steps a history was written in, and the versions marked on it.
//!
//! A step is one change of the history: what one replica wrote without a pause, as far as its history
//! tells one stretch of writing from the next. A version is a revision someone, or the shell, marked to
//! be listed and kept. The marks live in the map `versions`, so they travel and merge with the history.

use std::collections::{HashMap, HashSet};

use loro::{
    ChangeMeta, ContainerTrait, Frontiers, ID, JsonOpContent, JsonTextOp, LoroDoc, ValueOrContainer, VersionVector,
};
use serde_json::{Map, Value};

use crate::flat::MARKER;
use crate::{Revision, crdt};

const VERSIONS: &str = "versions";
/// The message of a change that only marks or unmarks a version. Loro keeps a change with a message
/// apart from the changes beside it, so such a change never holds an edit.
const MARKING: &str = "version";

/// One step of a history, oldest first in [`Model::timeline`](crate::Model::timeline).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Point {
    /// The revision the step starts from: every step listed before it, and none after.
    pub before: Revision,
    /// The revision it leads to.
    pub after: Revision,
    /// Who wrote it, each with how many of its edits are theirs. Empty for a step no one put a name to:
    /// a file changed outside Letters, a settled conflict, a writer the shell did not name.
    pub authors: Vec<(String, usize)>,
    /// When it was begun, in seconds since 1970. 0 when the model was never told the time.
    pub time: u64,
    /// How many edits it holds.
    pub edits: usize,
    /// How much it changed: about the characters put in and taken out.
    pub size: usize,
    /// The characters of text it put in, and those it took out. A step that changed only formatting, the
    /// kind of a block or where blocks part has neither.
    pub added: usize,
    pub removed: usize,
    /// The versions marked at this step: those it is the first to hold all of. A version marked on one
    /// device while another wrote can lack some of what `after` holds, so show a version at its own
    /// revision.
    pub tags: Vec<Tag>,
}

/// Why a version was marked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TagKind {
    /// A person marked it.
    Named,
    /// A session ended there.
    Session,
    /// Writing paused there for long.
    Pause,
    /// An import ended there.
    Import,
    /// Two copies were merged there.
    Merge,
    /// An older version was restored there.
    Restore,
}

const KINDS: [(TagKind, &str); 6] = [
    (TagKind::Named, "named"),
    (TagKind::Session, "session"),
    (TagKind::Pause, "pause"),
    (TagKind::Import, "import"),
    (TagKind::Merge, "merge"),
    (TagKind::Restore, "restore"),
];

/// A version: a revision marked to be listed in the history, and kept however the history is thinned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub at: Revision,
    pub kind: TagKind,
    /// What it is called, when it was given a name.
    pub name: Option<String>,
    /// Who marked it, when the shell named the writer.
    pub author: Option<String>,
    /// When it was marked, in seconds since 1970. 0 when the model was never told the time.
    pub time: u64,
    /// The version a restore brought back.
    pub restored: Option<Revision>,
}

/// Every step of the history, oldest first, with the versions marked at each. The history must show its
/// newest revision, which is where the writers' names and the marks are all known.
///
/// Steps written apart have no order of their own. They are listed by Loro's logical clock and then by
/// replica, which every device does alike, and which never lists a step before one it was written on.
pub(crate) fn timeline(loro: &LoroDoc) -> Vec<Point> {
    steps(loro).0
}

/// The steps of [`timeline`] that a revision lacks the end of, where `held` is all the revision holds.
/// A step written apart from the revision is among them wherever it is listed.
pub(crate) fn since(loro: &LoroDoc, held: &VersionVector) -> Vec<Point> {
    let (points, ends) = steps(loro);
    points.into_iter().zip(ends).filter(|(_, end)| !held.includes_id(*end)).map(|(point, _)| point).collect()
}

/// The steps as [`timeline`] lists them, and the last edit of each.
fn steps(loro: &LoroDoc) -> (Vec<Point>, Vec<ID>) {
    let names = crdt::names(loro);
    let mut heads = Frontiers::default();
    let (mut points, mut ends): (Vec<Point>, Vec<ID>) = (Vec::new(), Vec::new());
    // Where each replica's changes start, with the step that is the first to hold each.
    let mut firsts: HashMap<u64, Vec<(i32, usize)>> = HashMap::new();
    for change in changes(loro) {
        let before = heads.clone();
        heads.update_frontiers_on_new_change(last(&change), &change.deps);
        let marking = change.message.as_deref() == Some(MARKING);
        // A change that only marks a version is no step, and goes with the step before it.
        let step = if marking { points.len().saturating_sub(1) } else { points.len() };
        firsts.entry(change.id.peer).or_default().push((change.id.counter, step));
        if marking {
            continue;
        }
        ends.push(last(&change));
        points.push(Point {
            before: Revision(before),
            after: Revision(heads.clone()),
            authors: names.get(&change.id.peer).map(|name| (name.clone(), 1)).into_iter().collect(),
            time: u64::try_from(change.timestamp).unwrap_or_default(),
            edits: 1,
            size: change.len,
            added: 0,
            removed: 0,
            tags: Vec::new(),
        });
    }
    let step = |id: ID| {
        let starts = firsts.get(&id.peer)?;
        let held = starts.partition_point(|(counter, _)| *counter <= id.counter).checked_sub(1)?;
        Some(starts[held].1)
    };
    count(loro, &mut points, step);
    for tag in tags(loro) {
        // A revision holds all that its newest changes were written on, so the last of them to be listed
        // is the step that completes it. A mark on a revision this history lacks is left out.
        let at: Option<Vec<usize>> = tag.at.0.iter().map(step).collect();
        if let Some(point) = at.and_then(|at| at.into_iter().max()).and_then(|at| points.get_mut(at)) {
            point.tags.push(tag);
        }
    }
    (points, ends)
}

/// The revisions either side of each step, as [`timeline`] lists them: all a reader of old revisions
/// needs of it.
pub(crate) fn revisions(loro: &LoroDoc) -> Vec<(Frontiers, Frontiers)> {
    let (mut heads, mut steps) = (Frontiers::default(), Vec::new());
    for change in changes(loro) {
        let before = heads.clone();
        heads.update_frontiers_on_new_change(last(&change), &change.deps);
        if change.message.as_deref() != Some(MARKING) {
            steps.push((before, heads.clone()));
        }
    }
    steps
}

/// The changes of a history in the order every device lists them.
///
/// Loro stores what a writer wrote at once in parts of about 4 KB, each written on the one before. They
/// are told here as the one change they were written as, so that a long paste is one step.
fn changes(loro: &LoroDoc) -> Vec<ChangeMeta> {
    let mut changes: Vec<ChangeMeta> = Vec::new();
    for (peer, end) in loro.oplog_vv().iter() {
        let (first, mut counter) = (changes.len(), 0);
        while counter < *end {
            let Some(part) = loro.get_change(ID::new(*peer, counter)) else { break };
            counter = part.id.counter + i32::try_from(part.len.max(1)).unwrap_or(i32::MAX);
            match changes[first..].last_mut() {
                Some(change)
                    if part.timestamp == change.timestamp
                        && part.message == change.message
                        && part.deps.as_single() == Some(last(change)) =>
                {
                    change.len += part.len;
                }
                _ => changes.push(part),
            }
        }
    }
    changes.sort_by_key(|change| (change.lamport, change.id.peer));
    changes
}

/// The last edit of a change.
fn last(change: &ChangeMeta) -> ID {
    ID::new(change.id.peer, change.id.counter + i32::try_from(change.len.max(1) - 1).unwrap_or(0))
}

/// Counts the characters of text each step put in and took out. `step` tells the step that holds an edit.
fn count(loro: &LoroDoc, points: &mut [Point], step: impl Fn(ID) -> Option<usize>) {
    let Ok(edits) = loro.export_json_updates_without_peer_compression(&VersionVector::default(), &loro.oplog_vv())
    else {
        return;
    };
    let sequence = &loro.get_text(crdt::BODY).id();
    let texts = || {
        edits.changes.iter().flat_map(move |change| {
            let ops = change.ops.iter().filter(move |op| op.container == *sequence);
            ops.map(move |op| (ID::new(change.id.peer, op.counter), &op.content))
        })
    };
    // A marker starts a block and is no character of text, put in or taken out.
    let mut markers = HashSet::new();
    for (id, edit) in texts() {
        if let JsonOpContent::Text(JsonTextOp::Insert { text, .. }) = edit {
            let marks = text.chars().zip(id.counter..).filter(|(c, _)| MARKER.starts_with(*c));
            markers.extend(marks.map(|(_, counter)| ID::new(id.peer, counter)));
        }
    }
    for (id, edit) in texts() {
        let Some(point) = step(id).and_then(|at| points.get_mut(at)) else { continue };
        match edit {
            JsonOpContent::Text(JsonTextOp::Insert { text, .. }) => {
                point.added += text.chars().filter(|c| !MARKER.starts_with(*c)).count();
            }
            JsonOpContent::Text(JsonTextOp::Delete { len, start_id, .. }) => {
                let gone = (0..len.abs()).map(|nth| ID::new(start_id.peer, start_id.counter + nth));
                point.removed += gone.filter(|id| !markers.contains(id)).count();
            }
            _ => {}
        }
    }
}

/// Marks a version in the change that is open, over any mark its revision already has.
pub(crate) fn mark(loro: &LoroDoc, tag: &Tag) {
    let mut fields = Map::new();
    let kind = KINDS.iter().find(|(kind, _)| *kind == tag.kind).map_or("named", |(_, name)| name);
    fields.insert("kind".into(), kind.into());
    let restored = tag.restored.as_ref().map(Revision::to_string);
    for (key, value) in [("name", &tag.name), ("author", &tag.author), ("restored", &restored)] {
        if let Some(value) = value {
            fields.insert(key.into(), value.as_str().into());
        }
    }
    if tag.time > 0 {
        fields.insert("time".into(), tag.time.into());
    }
    loro.set_next_commit_message(MARKING);
    // Loro refuses no string under a string.
    let _ = loro.get_map(VERSIONS).insert(&tag.at.to_string(), Value::Object(fields).to_string());
}

/// Takes the mark off a revision, in the change that is open. False when it had none.
pub(crate) fn unmark(loro: &LoroDoc, at: &Revision) -> bool {
    let (versions, at) = (loro.get_map(VERSIONS), at.to_string());
    let marked = versions.get(&at).is_some();
    if marked {
        loro.set_next_commit_message(MARKING);
        let _ = versions.delete(&at);
    }
    marked
}

/// The versions marked, as the revision shown holds them, oldest first. A mark this build cannot read is
/// left out, and stays in the history.
pub(crate) fn tags(loro: &LoroDoc) -> Vec<Tag> {
    let mut tags = Vec::new();
    loro.get_map(VERSIONS).for_each(|at, held| {
        let ValueOrContainer::Value(held) = held else { return };
        let fields = held.as_string().and_then(|json| serde_json::from_str::<Map<String, Value>>(json).ok());
        let text = |fields: &Map<String, Value>, key| fields.get(key).and_then(Value::as_str).map(str::to_owned);
        if let Some(fields) = fields
            && let Ok(at) = at.parse()
            && let Some((kind, _)) =
                KINDS.iter().find(|(_, name)| fields.get("kind").and_then(Value::as_str) == Some(name))
        {
            let time = fields.get("time").and_then(Value::as_u64).unwrap_or_default();
            let restored = text(&fields, "restored").and_then(|restored| restored.parse().ok());
            let (name, author) = (text(&fields, "name"), text(&fields, "author"));
            tags.push(Tag { at, kind: *kind, name, author, time, restored });
        }
    });
    tags.sort_by_cached_key(|tag| (tag.time, tag.at.to_string()));
    tags
}
