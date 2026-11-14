//! How a document is laid out in its Loro history, and the conversions in and out.
//!
//! - `body` is one rich-text sequence holding every block: a marker character, then the block's text.
//!   Splitting or joining a paragraph is one character, so it merges with concurrent typing.
//! - `blocks` and `depths` are maps from a marker character's own id to what its block is and how deep it
//!   sits. Kept apart from the text, a block can change kind while someone types beside its marker.
//! - `meta`, `dictionary` and `extra` are maps holding the document's own fields.
//!
//! The rest of the crate reads and writes the sequence as changes in which a marker carries its block, as
//! the [`BLOCK`] and [`DEPTH`] attributes. This module turns those to and from what Loro holds, and
//! nothing outside this crate sees Loro, so the library can be replaced without touching another crate.
//!
//! Loro draws a random id for every document it makes, before the replica's own id replaces it. Nothing is
//! written under it, but on wasm it is read from the browser's `crypto.getRandomValues` through
//! wasm-bindgen: the one thing the core takes from its host without being handed it.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::hash::BuildHasher;
use std::mem;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use loro::cursor::{Cursor, Side};
use loro::event::Diff;
use loro::{
    ContainerTrait, ExpandType, ID, LoroDoc, LoroMap, LoroText, LoroValue, StyleConfig, Subscription, TextDelta,
    ValueOrContainer,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::flat::{Attrs, BLOCK, DEPTH, Entry, MARKER, build, head, insert, order, transform};
use crate::history::read_id;
use crate::json::{mark_name, read_mark};
use crate::undo::Script;
use crate::{Block, BlockKind, Document, Run};

pub(crate) const BODY: &str = "body";
const BLOCKS: &str = "blocks";
const DEPTHS: &str = "depths";
const META: &str = "meta";
const DICTIONARY: &str = "dictionary";
const EXTRA: &str = "extra";
const LANGUAGE: &str = "language";

pub(crate) const LINK: &str = "link";
/// Prefix of a mark's attribute, whose value is `true`.
const MARK: &str = "m.";
/// Prefix of the attribute for a run field this build does not know, whose value is the field's JSON.
const FIELD: &str = "x.";

/// Typed text takes the formatting of the character before it.
pub(crate) fn configure(loro: &LoroDoc) {
    loro.config_default_text_style(Some(StyleConfig { expand: ExpandType::After }));
}

/// Text as the sequence can hold it. The marker character is the sequence's own, so text gives it up.
pub(crate) fn clean(text: &str) -> Cow<'_, str> {
    if text.contains(MARKER) { Cow::Owned(text.replace(MARKER, " ")) } else { Cow::Borrowed(text) }
}

/// An attribute key for `name`. Loro reads a colon in a key as a separator, so it is escaped.
fn key(prefix: &str, name: &str) -> String {
    format!("{prefix}{}", name.replace('%', "%25").replace(':', "%3A"))
}

/// The attribute that carries `mark`.
pub(crate) fn mark_attr(mark: &crate::Mark) -> String {
    key(MARK, mark_name(mark))
}

fn name(key: &str) -> String {
    key.replace("%3A", ":").replace("%25", "%")
}

fn text(value: &LoroValue) -> Option<&str> {
    match value {
        LoroValue::String(text) => Some(text.as_str()),
        _ => None,
    }
}

/// Changes a run's formatting as text attributes say. A null value clears, and unknown keys are left alone.
pub(crate) fn format<S: BuildHasher>(run: &mut Run, attrs: &Attrs<S>) {
    for (attr, value) in attrs {
        if attr == LINK {
            run.link = text(value).map(str::to_owned);
        } else if let Some(mark) = attr.strip_prefix(MARK) {
            let mark = read_mark(&name(mark));
            if matches!(value, LoroValue::Bool(true)) {
                run.marks.insert(mark);
            } else {
                run.marks.remove(&mark);
            }
        } else if let Some(field) = attr.strip_prefix(FIELD) {
            match text(value).and_then(|json| serde_json::from_str(json).ok()) {
                Some(value) => run.extra.insert(name(field), value),
                None => run.extra.remove(&name(field)),
            };
        }
    }
}

/// The attributes that give text the formatting of `run`, in a fixed order.
pub(crate) fn attrs(run: &Run) -> Vec<(String, LoroValue)> {
    let marks = run.marks.iter().map(|mark| (key(MARK, mark_name(mark)), true.into()));
    let link = run.link.iter().map(|link| (LINK.to_owned(), link.as_str().into()));
    let extra = run.extra.iter().map(|(field, value)| (key(FIELD, field), value.to_string().into()));
    marks.chain(link).chain(extra).collect()
}

/// What the history keeps for each marker, under the id Loro gave the marker character. No other
/// character can ever have that id, so nothing typed beside a marker can be taken for part of it.
struct Markers {
    blocks: LoroMap,
    depths: LoroMap,
}

impl Markers {
    fn of(loro: &LoroDoc) -> Self {
        Self { blocks: loro.get_map(BLOCKS), depths: loro.get_map(DEPTHS) }
    }

    fn key(id: ID) -> String {
        format!("{}@{}", id.counter, id.peer)
    }

    fn value(map: &LoroMap, key: &str) -> Option<LoroValue> {
        match map.get(key)? {
            ValueOrContainer::Value(value) => Some(value),
            ValueOrContainer::Container(_) => None,
        }
    }

    /// The attributes a marker is read by. A marker the maps do not know is a paragraph named by its key.
    fn attrs(&self, id: Option<ID>) -> Vec<(String, LoroValue)> {
        let key = id.map(Self::key).unwrap_or_default();
        let block = match Self::value(&self.blocks, &key) {
            Some(block @ LoroValue::String(_)) => block,
            _ => head(&Block::new(key.as_str(), BlockKind::Paragraph)).into(),
        };
        let depth = match Self::value(&self.depths, &key) {
            Some(depth @ LoroValue::I64(_)) => depth,
            _ => LoroValue::Null,
        };
        vec![(BLOCK.to_owned(), block), (DEPTH.to_owned(), depth)]
    }

    /// Keeps the attributes among `attrs` that say what a marker's block is.
    fn set<S: BuildHasher>(&self, id: ID, attrs: &Attrs<S>) -> Option<()> {
        let key = Self::key(id);
        if let Some(block @ LoroValue::String(_)) = attrs.get(BLOCK) {
            self.blocks.insert(&key, block.clone()).ok()?;
        }
        match attrs.get(DEPTH) {
            Some(depth @ LoroValue::I64(_)) => self.depths.insert(&key, depth.clone()).ok(),
            Some(_) => Self::forget(&self.depths, &key),
            None => Some(()),
        }
    }

    fn forget(map: &LoroMap, key: &str) -> Option<()> {
        if map.get(key).is_some() { map.delete(key).ok() } else { Some(()) }
    }
}

/// The id of the character at `at`.
fn id_at(body: &LoroText, at: usize) -> Option<ID> {
    body.get_cursor(at, Side::Middle)?.id
}

fn retain(retain: usize) -> TextDelta {
    TextDelta::Retain { retain, attributes: None }
}

/// Whether attributes say what a marker's block is, as none that Loro holds for the text do.
fn of_marker<S: BuildHasher>(attrs: &Attrs<S>) -> bool {
    attrs.contains_key(BLOCK) || attrs.contains_key(DEPTH)
}

/// Rewrites a change as Loro reports it, where a marker is only a character, into one whose markers carry
/// their blocks. The history must stand as the change left it. The markers the change put in are added to
/// `new`.
pub(crate) fn lift(loro: &LoroDoc, body: &LoroText, raw: &[TextDelta], new: &mut HashSet<ID>) -> Vec<TextDelta> {
    let markers = Markers::of(loro);
    // A history written by something else could hold these attributes on text. Here they mean a marker.
    let plain = |attrs: &Option<_>| attrs.clone().filter(|attrs| !of_marker(attrs));
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0;
    for item in raw {
        match item {
            TextDelta::Retain { retain, attributes } => {
                out.push(TextDelta::Retain { retain: *retain, attributes: plain(attributes) });
                at += retain;
            }
            TextDelta::Delete { .. } => out.push(item.clone()),
            TextDelta::Insert { insert: text, attributes } => {
                for (n, text) in text.split(MARKER).enumerate() {
                    if n > 0 {
                        let id = id_at(body, at);
                        new.extend(id);
                        out.push(insert(MARKER, markers.attrs(id)));
                        at += 1;
                    }
                    if !text.is_empty() {
                        out.push(TextDelta::Insert { insert: text.to_owned(), attributes: plain(attributes) });
                        at += text.chars().count();
                    }
                }
            }
        }
    }
    out
}

/// The change that gives markers what the history now holds for them: those named by `keys` that are in
/// the sequence, and not in `new`, which were read as they came in.
pub(crate) fn restyles(loro: &LoroDoc, body: &LoroText, keys: &[String], new: &HashSet<ID>) -> Vec<TextDelta> {
    let markers = Markers::of(loro);
    let mut places: Vec<(usize, ID)> = keys
        .iter()
        .filter_map(|key| {
            let id = read_id(key).filter(|id| !new.contains(id))?;
            // A marker that is gone, or not yet there at the revision shown, has nothing to change. Its
            // block is gone from the map with it, which saves Loro a search of the history for it.
            markers.blocks.get(key)?;
            let found = loro.get_cursor_pos(&Cursor::new(Some(id), body.id(), Side::Middle, 0)).ok()?;
            let there = found.update.is_none() && id_at(body, found.current.pos) == Some(id);
            there.then_some((found.current.pos, id))
        })
        .collect();
    places.sort_unstable_by_key(|(at, _)| *at);
    places.dedup();
    let mut delta = Vec::new();
    let mut at = 0;
    for (place, id) in places {
        if place > at {
            delta.push(retain(place - at));
        }
        delta.push(TextDelta::Retain { retain: 1, attributes: Some(markers.attrs(Some(id)).into_iter().collect()) });
        at = place + 1;
    }
    delta
}

/// Makes in the history a change the document already shows. `None` when Loro refused a part of it.
pub(crate) fn write(loro: &LoroDoc, body: &LoroText, delta: &[TextDelta]) -> Option<()> {
    let markers = Markers::of(loro);
    // Loro's own delta sets exactly the attributes named, whatever the text beside would lend.
    let put = |at: usize, item: TextDelta| {
        let delta = if at > 0 { vec![retain(at), item] } else { vec![item] };
        body.apply_delta(&delta).ok()
    };
    let mut at = 0;
    for item in delta {
        match item {
            TextDelta::Retain { retain, attributes } => {
                match attributes.as_ref().filter(|attrs| !attrs.is_empty()) {
                    Some(attrs) if of_marker(attrs) => {
                        for at in at..at + retain {
                            if body.char_at(at).ok().is_some_and(|c| MARKER.starts_with(c)) {
                                markers.set(id_at(body, at)?, attrs)?;
                            }
                        }
                    }
                    Some(_) => put(at, item.clone())?,
                    None => {}
                }
                at += retain;
            }
            TextDelta::Insert { insert: text, attributes } => {
                match attributes.as_ref().filter(|attrs| matches!(attrs.get(BLOCK), Some(LoroValue::String(_)))) {
                    Some(attrs) => {
                        for _ in text.chars() {
                            put(at, insert(MARKER, Vec::new()))?;
                            markers.set(id_at(body, at)?, attrs)?;
                            at += 1;
                        }
                    }
                    None => {
                        put(at, item.clone())?;
                        at += text.chars().count();
                    }
                }
            }
            TextDelta::Delete { delete } => {
                let gone = body.slice(at, at + delete).ok()?;
                for (n, _) in gone.chars().enumerate().filter(|(_, c)| MARKER.starts_with(*c)) {
                    let key = Markers::key(id_at(body, at + n)?);
                    Markers::forget(&markers.blocks, &key)?;
                    Markers::forget(&markers.depths, &key)?;
                }
                body.delete(at, *delete).ok()?;
            }
        }
    }
    Some(())
}

/// What one container's part of a change the history reported means to the model.
pub(crate) enum Event {
    /// The sequence changed, as Loro tells it.
    Body(Vec<TextDelta>),
    /// What these markers' blocks are changed, by key.
    Markers(Vec<String>),
    /// One of the document's own fields changed.
    Meta,
}

/// Collects what the history reports, for the model to catch up with.
pub(crate) fn watch(loro: &LoroDoc, body: &LoroText) -> (Arc<Mutex<Vec<Event>>>, Subscription) {
    let events: Arc<Mutex<Vec<Event>>> = Arc::default();
    let (queue, body) = (events.clone(), body.id());
    let markers = [loro.get_map(BLOCKS).id(), loro.get_map(DEPTHS).id()];
    let watch = loro.subscribe_root(Arc::new(move |event| {
        let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
        for diff in event.events {
            queue.push(match diff.diff {
                Diff::Text(delta) if *diff.target == body => Event::Body(delta),
                Diff::Map(map) if markers.contains(diff.target) => {
                    Event::Markers(map.updated.keys().map(|key| key.to_string()).collect())
                }
                _ => Event::Meta,
            });
        }
    }));
    (events, watch)
}

/// One write that puts a document into an empty history.
enum Call {
    /// Text put in at the start of the sequence.
    Text(String),
    /// What the block is whose marker now starts the sequence, and how deep it sits.
    Block(String, usize),
    /// An attribute over a byte range of the sequence as it then stands.
    Mark(Range<usize>, String, LoroValue),
    Set(&'static str, String, LoroValue),
}

/// The writes that store a document, in an order that depends on nothing but the document. It must be one
/// that [`crate::flat::normal`] gave, with its entries.
///
/// Each block is its own insert, followed by its own attributes. One insert for the whole text would be
/// cut in two by every attribute after it, and Loro takes quadratic time to replay that when it moves
/// through history. The last block is written first and every other one in front of it, where no
/// formatting of a neighbour can spread onto it.
fn plan(doc: &Document, index: &[Entry]) -> Vec<Call> {
    let blocks = order(&doc.blocks, index).into_iter().map(|(block, entry)| {
        let mut text = String::from(MARKER);
        let mut calls = vec![Call::Block(head(block), entry.level)];
        // One mark covers neighbouring runs that share an attribute.
        let mut open: BTreeMap<String, (LoroValue, Range<usize>)> = BTreeMap::new();
        for run in block.body.runs() {
            let range = text.len()..text.len() + run.text.len();
            text.push_str(&run.text);
            for (key, value) in attrs(run) {
                match open.get_mut(&key) {
                    Some((held, span)) if *held == value && span.end == range.start => span.end = range.end,
                    _ => {
                        if let Some((held, span)) = open.insert(key.clone(), (value, range.clone())) {
                            calls.push(Call::Mark(span, key, held));
                        }
                    }
                }
            }
        }
        calls.extend(open.into_iter().map(|(key, (value, span))| Call::Mark(span, key, value)));
        calls.insert(0, Call::Text(text));
        calls
    });
    let mut calls: Vec<Call> = blocks.rev().flatten().collect();
    calls.push(Call::Set(META, LANGUAGE.into(), doc.language.as_str().into()));
    calls.extend(doc.dictionary.iter().map(|word| Call::Set(DICTIONARY, word_key(word, ""), true.into())));
    calls.extend(doc.extra.iter().map(|(key, value)| Call::Set(EXTRA, key.clone(), value.to_string().into())));
    calls
}

/// The first eight bytes of a hash, as an id Loro accepts.
pub(crate) fn peer(hash: Sha256) -> u64 {
    let bytes: [u8; 8] = hash.finalize()[..8].try_into().expect("a SHA-256 digest has 32 bytes");
    u64::from_le_bytes(bytes).min(u64::MAX - 1)
}

/// The replica that writes a document's first change: a hash of that change.
///
/// Two devices that open the same document without a history therefore write the very same change, and their
/// copies still merge. The hash covers every write, so a build that stores documents differently can never
/// claim the same id for different content. Change the tag if the way the writes are carried out changes.
fn author(calls: &[Call]) -> u64 {
    fn feed(hash: &mut Sha256, bytes: &[u8]) {
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    fn feed_value(hash: &mut Sha256, value: &LoroValue) {
        match value {
            LoroValue::String(text) => feed(hash, text.as_bytes()),
            LoroValue::I64(number) => feed(hash, &number.to_le_bytes()),
            LoroValue::Bool(flag) => feed(hash, &[u8::from(*flag)]),
            other => feed(hash, format!("{other:?}").as_bytes()),
        }
    }

    let mut hash = Sha256::new();
    feed(&mut hash, b"letters first change 1");
    for call in calls {
        match call {
            Call::Text(text) => {
                feed(&mut hash, b"text");
                feed(&mut hash, text.as_bytes());
            }
            Call::Block(block, depth) => {
                feed(&mut hash, b"block");
                feed(&mut hash, block.as_bytes());
                feed(&mut hash, &(*depth as u64).to_le_bytes());
            }
            Call::Mark(range, key, value) => {
                feed(&mut hash, b"mark");
                feed(&mut hash, &(range.start as u64).to_le_bytes());
                feed(&mut hash, &(range.end as u64).to_le_bytes());
                feed(&mut hash, key.as_bytes());
                feed_value(&mut hash, value);
            }
            Call::Set(map, key, value) => {
                feed(&mut hash, map.as_bytes());
                feed(&mut hash, key.as_bytes());
                feed_value(&mut hash, value);
            }
        }
    }
    peer(hash)
}

/// Starts a history that holds a document as its first change. See [`plan`] for which documents.
pub(crate) fn create(doc: &Document, index: &[Entry]) -> LoroDoc {
    let loro = LoroDoc::new();
    configure(&loro);
    let calls = plan(doc, index);
    let (body, markers) = (loro.get_text(BODY), Markers::of(&loro));
    // None of these can fail: the peer is in range, the ranges lie inside the block just written, and no
    // key holds a colon. A failure would leave a shorter document, which the model then shows as it is.
    let _ = loro.set_peer_id(author(&calls));
    for call in calls {
        let _ = match call {
            Call::Text(text) => body.insert_utf8(0, &text),
            Call::Block(block, depth) => {
                let depth = if depth > 0 { (depth as i64).into() } else { LoroValue::Null };
                let attrs: Attrs<_> = [(BLOCK.to_owned(), block.into()), (DEPTH.to_owned(), depth)].into();
                id_at(&body, 0).and_then(|id| markers.set(id, &attrs));
                Ok(())
            }
            Call::Mark(range, key, value) => body.mark_utf8(range, &key, value),
            Call::Set(map, key, value) => loro.get_map(map).insert(&key, value),
        };
    }
    loro.commit();
    loro
}

/// Reads the whole document out of a history.
pub(crate) fn read(loro: &LoroDoc, body: &LoroText) -> (Document, Vec<Entry>) {
    let delta = lift(loro, body, &body.to_delta(), &mut HashSet::new());
    let read = transform(Vec::new(), &delta, &mut Script::default());
    let built = build(read.map(|read| read.blocks).unwrap_or_default());
    let mut doc = Document { blocks: built.blocks, ..Document::default() };
    read_meta(loro, &mut doc);
    (doc, built.index)
}

/// Reads the document's own fields, which are small enough to read whole after any change.
pub(crate) fn read_meta(loro: &LoroDoc, doc: &mut Document) {
    fn each(loro: &LoroDoc, map: &str, mut f: impl FnMut(&str, &LoroValue)) {
        loro.get_map(map).for_each(|key, value| {
            if let ValueOrContainer::Value(value) = value {
                f(key, &value);
            }
        });
    }

    let fresh = Document::default();
    doc.language = fresh.language;
    doc.dictionary.clear();
    doc.extra.clear();
    each(loro, META, |key, value| {
        if let (LANGUAGE, Some(language)) = (key, text(value)) {
            doc.language = language.to_owned();
        }
    });
    each(loro, DICTIONARY, |key, value| {
        if matches!(value, LoroValue::Bool(true)) {
            doc.dictionary.insert(word_of(key).to_owned());
        }
    });
    each(loro, EXTRA, |key, value| {
        if let Some(value) = text(value).and_then(|json| serde_json::from_str(json).ok()) {
            doc.extra.insert(key.to_owned(), value);
        }
    });
}

/// The key of one add of a word to the dictionary. Every add has a key of its own, so removing a word takes
/// out the adds it has seen and no others: a word added elsewhere at the same time stays.
fn word_key(word: &str, add: &str) -> String {
    format!("{}{MARKER}{add}", clean(word))
}

fn word_of(key: &str) -> &str {
    key.split(MARKER).next().unwrap_or(key)
}

/// Makes the document's own fields what a history gives back for them.
pub(crate) fn normal_meta(doc: &mut Document) {
    if doc.dictionary.iter().any(|word| word.contains(MARKER)) {
        doc.dictionary = mem::take(&mut doc.dictionary).iter().map(|word| clean(word).into_owned()).collect();
    }
    for value in doc.extra.values_mut() {
        if let Ok(read) = serde_json::from_str(&value.to_string()) {
            *value = read;
        }
    }
}

pub(crate) fn set_language(loro: &LoroDoc, language: &str) -> Option<()> {
    loro.get_map(META).insert(LANGUAGE, language).ok()
}

pub(crate) fn add_word(loro: &LoroDoc, word: &str) -> Option<()> {
    let peer = loro.peer_id();
    // The number of this replica's next edit, which no other add can have.
    let next = loro.oplog_vv().get(&peer).copied().unwrap_or(0);
    loro.get_map(DICTIONARY).insert(&word_key(word, &format!("{next}@{peer}")), true).ok()
}

pub(crate) fn remove_word(loro: &LoroDoc, word: &str) -> Option<()> {
    let map = loro.get_map(DICTIONARY);
    let mut adds = Vec::new();
    map.for_each(|key, _| {
        if word_of(key) == word {
            adds.push(key.to_owned());
        }
    });
    adds.iter().try_for_each(|key| map.delete(key).ok())
}

/// Sets one of the document's fields that this build does not know, or removes it.
pub(crate) fn set_extra(loro: &LoroDoc, key: &str, value: Option<&Value>) -> Option<()> {
    let map = loro.get_map(EXTRA);
    match value {
        Some(value) => map.insert(key, value.to_string()).ok(),
        None => Markers::forget(&map, key),
    }
}

/// Makes the document's own fields in the history those of `to`, where they differ from `from`.
pub(crate) fn write_meta(loro: &LoroDoc, from: &Document, to: &Document) -> Option<()> {
    if from.language != to.language {
        set_language(loro, &to.language)?;
    }
    from.dictionary.difference(&to.dictionary).try_for_each(|word| remove_word(loro, word))?;
    to.dictionary.difference(&from.dictionary).try_for_each(|word| add_word(loro, word))?;
    from.extra.keys().filter(|key| !to.extra.contains_key(*key)).try_for_each(|key| set_extra(loro, key, None))?;
    let changed = to.extra.iter().filter(|(key, value)| from.extra.get(*key) != Some(value));
    changed.into_iter().try_for_each(|(key, value)| set_extra(loro, key, Some(value)))
}
