//! How a document is laid out in its Loro history, and the conversions in and out.
//!
//! - `body` is one rich-text sequence holding every block: a marker character, then the block's text.
//!   Splitting or joining a paragraph is one character, so it merges with concurrent typing.
//! - `blocks` and `depths` are maps from a marker character's own id to what its block is and how deep it
//!   sits. Kept apart from the text, a block can change kind while someone types beside its marker.
//! - `meta`, `dictionary` and `extra` are maps holding the document's own fields.
//! - `style` is a map with one entry per field of the style, so two replicas that change different fields
//!   of one element type both keep their change.
//!
//! The rest of the crate reads and writes the sequence as changes in which a marker carries its block, as
//! the [`BLOCK`] and [`DEPTH`] attributes. This module turns those to and from what Loro holds, and
//! nothing outside this crate sees Loro, so the library can be replaced without touching another crate.
//!
//! Loro draws a random id for every document it makes, before the replica's own id replaces it. Nothing is
//! written under it, but on wasm it is read from the browser's `crypto.getRandomValues` through
//! wasm-bindgen: the one thing the core takes from its host without being handed it.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::BuildHasher;
use std::mem;
use std::ops::Range;
use std::sync::{Arc, Mutex, PoisonError};

use loro::cursor::{Cursor, PosType, Side};
use loro::event::Diff;
use loro::{
    ContainerTrait, ExpandType, ID, LoroDoc, LoroMap, LoroText, LoroValue, StyleConfig, Subscription, TextDelta,
    ValueOrContainer,
};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::flat::{Attrs, BLOCK, DEPTH, Entry, MARKER, build, head, insert, order, transform};
use crate::history::read_id;
use crate::json::{mark_name, read_mark};
use crate::undo::Script;
use crate::{Block, BlockId, BlockKind, Document, Mark, Properties, Run, STYLE_FORMAT, Style};

pub(crate) const BODY: &str = "body";
const BLOCKS: &str = "blocks";
const DEPTHS: &str = "depths";
const META: &str = "meta";
const DICTIONARY: &str = "dictionary";
const EXTRA: &str = "extra";
const STYLE: &str = "style";
/// The writers' names, each against the replica its writer used.
const AUTHORS: &str = "authors";
/// The blocks whose conflict was settled, each against its [`Settling`].
const SETTLED: &str = "settled";
const LANGUAGE: &str = "language";
const PROPERTIES: &str = "properties";

pub(crate) const LINK: &str = "link";
/// The attribute for a run's language. It is the field `lang`, as a build that did not know it kept it.
pub(crate) const LANG: &str = "x.lang";
/// The attribute for the block a run refers to. Like [`LANG`], it is the field `ref` as an older build kept it.
pub(crate) const REF: &str = "x.ref";
/// Prefix of a mark's attribute, whose value is `true`.
const MARK: &str = "m.";
/// Prefix of the attribute for a run field this build does not know, whose value is the field's JSON.
const FIELD: &str = "x.";

/// How long one change may run on, in seconds, and so how closely an edit's time is known. A change
/// costs the history about 5 bytes: a minute adds 7% to an hour of slow typing, ten seconds 50%.
const CHANGE: i64 = 60;

/// Typed text takes the formatting of the character before it.
pub(crate) fn configure(loro: &LoroDoc) {
    loro.config_default_text_style(Some(StyleConfig { expand: ExpandType::After }));
    loro.set_change_merge_interval(CHANGE);
}

/// Writes the name of the replica's writer into the change that is open.
pub(crate) fn sign(loro: &LoroDoc, author: &str) {
    let (authors, replica) = (loro.get_map(AUTHORS), loro.peer_id().to_string());
    let held = authors.get(&replica).and_then(|held| held.into_value().ok());
    if held.as_ref().and_then(text) != Some(author) {
        // Loro refuses no string under a string.
        let _ = authors.insert(&replica, author);
    }
}

/// The name each replica's writer gave, as the revision shown holds them. A replica that gave none is not
/// among them, as one that made an outside change never is.
pub(crate) fn names(loro: &LoroDoc) -> HashMap<u64, String> {
    let mut names = HashMap::new();
    loro.get_map(AUTHORS).for_each(|replica, held| {
        if let ValueOrContainer::Value(held) = held
            && let Some(name) = text(&held).filter(|name| !name.is_empty())
            && let Ok(replica) = replica.parse()
        {
            names.insert(replica, name.to_owned());
        }
    });
    names
}

pub(crate) fn authors(loro: &LoroDoc) -> BTreeSet<String> {
    names(loro).into_values().collect()
}

/// Text as the sequence can hold it. The marker character is the sequence's own, so text gives it up.
pub(crate) fn clean(text: &str) -> Cow<'_, str> {
    if text.contains(MARKER) { Cow::Owned(text.replace(MARKER, " ")) } else { Cow::Borrowed(text) }
}

/// An attribute key for `name`. Loro reads a colon in a key as a separator, so it is escaped.
fn key(prefix: &str, name: &str) -> String {
    format!("{prefix}{}", name.replace('%', "%25").replace(':', "%3A"))
}

/// The attribute that carries `mark`, and the value that sets it.
pub(crate) fn mark_attr(mark: &Mark) -> (String, LoroValue) {
    match mark_name(mark) {
        Some(name) => (key(MARK, name), true.into()),
        None => (REF.to_owned(), field(mark_target(mark).unwrap_or_default())),
    }
}

/// The id of the block a reference names.
fn mark_target(mark: &Mark) -> Option<&str> {
    match mark {
        Mark::Ref(target) => Some(target.as_str()),
        _ => None,
    }
}

/// A text field as the attribute for an unknown field holds it: its JSON.
fn field(text: &str) -> LoroValue {
    Value::from(text).to_string().into()
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

/// What the [`LANG`] attribute holds for a language, or for none.
pub(crate) fn lang_value(lang: Option<&str>) -> LoroValue {
    lang.map_or(LoroValue::Null, field)
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
        } else if attr == LANG || attr == REF {
            let field = text(value).and_then(|json| serde_json::from_str::<String>(json).ok());
            let field = field.filter(|field| !field.is_empty());
            if attr == LANG {
                run.lang = field;
            } else {
                run.marks.retain(|mark| mark_target(mark).is_none());
                run.marks.extend(field.map(|target| Mark::Ref(BlockId::new(target))));
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
    let marks = run.marks.iter().filter(|mark| mark_target(mark).is_none()).map(mark_attr);
    let link = run.link.iter().map(|link| (LINK.to_owned(), link.as_str().into()));
    let extra = run.extra.iter().map(|(field, value)| (key(FIELD, field), value.to_string().into()));
    // Last, so that each stands over a field of the same name among the unknown ones.
    let lang = run.lang.iter().filter(|lang| !lang.is_empty()).map(|lang| (LANG.to_owned(), field(lang)));
    let target = run.marks.iter().filter(|mark| mark_target(mark).is_some_and(|id| !id.is_empty())).map(mark_attr);
    marks.chain(link).chain(extra).chain(lang).chain(target).collect()
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
pub(crate) fn id_at(body: &LoroText, at: usize) -> Option<ID> {
    body.get_cursor(at, Side::Middle)?.id
}

/// Where the character `id` stands in the sequence as it is shown. `None` when it is not in it. Loro
/// reads the sequence from its start to tell.
pub(crate) fn place(loro: &LoroDoc, body: &LoroText, id: ID) -> Option<usize> {
    let found = loro.get_cursor_pos(&Cursor::new(Some(id), body.id(), Side::Middle, 0)).ok()?;
    let there = found.update.is_none() && id_at(body, found.current.pos) == Some(id);
    there.then_some(found.current.pos)
}

/// A stretch of the sequence as a change that puts it in, its markers carrying their blocks, and the id
/// of each of its characters.
pub(crate) fn stretch(loro: &LoroDoc, body: &LoroText, range: Range<usize>) -> Option<(Vec<TextDelta>, Vec<ID>)> {
    let mut raw = vec![retain(range.start)];
    raw.extend(body.slice_delta(range.start, range.end, PosType::Unicode).ok()?);
    let mut pieces = lift(loro, body, &raw, &mut HashSet::new());
    pieces.remove(0);
    let ids = range.map(|at| id_at(body, at)).collect::<Option<Vec<ID>>>()?;
    Some((pieces, ids))
}

/// The settling of a block's conflict: the replica its change is written as, then who wrote the version it
/// made stand, and when.
pub(crate) type Settling = (u64, String, u64);

/// Notes in the change that is open that it settles the conflict over `block`, for a later search to
/// tell two settlings of one block apart, and to tell whose version the block then reads as.
pub(crate) fn settled(loro: &LoroDoc, block: &BlockId, author: &str, edited: u64) {
    // The replica is the settling's own, and names it.
    let held: Vec<LoroValue> = vec![loro.peer_id().cast_signed().into(), author.into(), edited.cast_signed().into()];
    // Loro refuses no list of plain values under a string.
    let _ = loro.get_map(SETTLED).insert(block.as_str(), held);
}

/// The last settling of each block, as the revision shown holds them. No person writes as its replica.
pub(crate) fn settlings(loro: &LoroDoc) -> HashMap<String, Settling> {
    let mut settlings = HashMap::new();
    loro.get_map(SETTLED).for_each(|block, held| {
        if let ValueOrContainer::Value(LoroValue::List(held)) = held
            && let [LoroValue::I64(replica), author, LoroValue::I64(edited)] = held.as_slice()
        {
            let author = text(author).unwrap_or_default().to_owned();
            settlings.insert(block.to_owned(), (replica.cast_unsigned(), author, edited.cast_unsigned()));
        }
    });
    settlings
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
            Some((place(loro, body, id)?, id))
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

/// Sets attributes over a stretch of the sequence one at a time, in the order of their names. Loro's own
/// delta sets several in an order that differs from one machine to the next, and a change made outside
/// Letters has to be written the same way on all of them.
fn mark<'a>(
    body: &LoroText,
    range: Range<usize>,
    attrs: impl Iterator<Item = (&'a String, &'a LoroValue)>,
) -> Option<()> {
    let attrs: BTreeMap<_, _> = attrs.collect();
    attrs.into_iter().try_for_each(|(key, value)| body.mark(range.clone(), key, value.clone()).ok())
}

/// Puts text in with exactly the attributes named, whatever the text beside it would lend.
fn put<S: BuildHasher>(body: &LoroText, at: usize, text: &str, attrs: Option<&Attrs<S>>) -> Option<()> {
    body.insert(at, text).ok()?;
    let wanted = |key: &String| attrs.and_then(|attrs| attrs.get(key)).filter(|value| !value.is_null());
    let mut from = at;
    for piece in body.slice_delta(at, at + text.chars().count(), PosType::Unicode).ok()? {
        let TextDelta::Insert { insert, attributes } = piece else { continue };
        let lent = attributes.unwrap_or_default();
        let to = from + insert.chars().count();
        let clear = lent.keys().filter(|key| wanted(key).is_none()).map(|key| (key, &LoroValue::Null));
        let set = lent.keys().chain(attrs.into_iter().flat_map(|attrs| attrs.keys()));
        let set = set.filter_map(|key| Some((key, wanted(key).filter(|value| lent.get(key) != Some(value))?)));
        mark(body, from..to, clear.chain(set))?;
        from = to;
    }
    Some(())
}

/// Makes in the history a change the document already shows. `None` when Loro refused a part of it.
///
/// Text that takes the place of text is put in behind the first character it replaces, which then goes
/// with the rest. Put in before that character, it would share its place with whatever another writer
/// typed there meanwhile, and the two would merge in an order that only their replica ids decide: a word
/// typed before a word that was replaced could come to stand behind the new one, or inside it.
pub(crate) fn write(loro: &LoroDoc, body: &LoroText, delta: &[TextDelta]) -> Option<()> {
    apply(loro, body, delta, true)
}

/// As [`write`], with all text put in exactly where the change says, for a change whose places were
/// worked out one by one.
pub(crate) fn write_as_told(loro: &LoroDoc, body: &LoroText, delta: &[TextDelta]) -> Option<()> {
    apply(loro, body, delta, false)
}

fn apply(loro: &LoroDoc, body: &LoroText, delta: &[TextDelta], replacing: bool) -> Option<()> {
    let markers = Markers::of(loro);
    let mut at = 0;
    let mut delta = delta.iter().peekable();
    while let Some(item) = delta.next() {
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
                    Some(attrs) => mark(body, at..at + retain, attrs.iter())?,
                    None => {}
                }
                at += retain;
            }
            TextDelta::Insert { insert: text, attributes } => {
                match attributes.as_ref().filter(|attrs| matches!(attrs.get(BLOCK), Some(LoroValue::String(_)))) {
                    Some(attrs) => {
                        for _ in text.chars() {
                            put(body, at, MARKER, None::<&Attrs<std::hash::RandomState>>)?;
                            markers.set(id_at(body, at)?, attrs)?;
                            at += 1;
                        }
                    }
                    None => {
                        put(body, at, text, attributes.as_ref())?;
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
                let mut put_in = 0;
                while replacing
                    && let Some(TextDelta::Insert { insert: text, attributes }) = delta.peek()
                    && !attributes.as_ref().is_some_and(|attrs| attrs.contains_key(BLOCK))
                {
                    put(body, at + 1 + put_in, text, attributes.as_ref())?;
                    put_in += text.chars().count();
                    delta.next();
                }
                if put_in == 0 {
                    body.delete(at, *delete).ok()?;
                } else {
                    body.delete(at, 1).ok()?;
                    at += put_in;
                    if *delete > 1 {
                        body.delete(at, delete - 1).ok()?;
                    }
                }
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
    Style,
    /// The conflicts over these blocks were settled, by id.
    Settled(Vec<String>),
}

/// Collects what the history reports, for the model to catch up with.
pub(crate) fn watch(loro: &LoroDoc, body: &LoroText) -> (Arc<Mutex<Vec<Event>>>, Subscription) {
    let events: Arc<Mutex<Vec<Event>>> = Arc::default();
    let (queue, body) = (events.clone(), body.id());
    let markers = [loro.get_map(BLOCKS).id(), loro.get_map(DEPTHS).id()];
    let (style, settled) = (loro.get_map(STYLE).id(), loro.get_map(SETTLED).id());
    let watch = loro.subscribe_root(Arc::new(move |event| {
        let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
        for diff in event.events {
            let keys = |map: loro::event::MapDelta| map.updated.keys().map(|key| key.to_string()).collect();
            queue.push(match diff.diff {
                Diff::Text(delta) if *diff.target == body => Event::Body(delta),
                Diff::Map(map) if markers.contains(diff.target) => Event::Markers(keys(map)),
                Diff::Map(map) if *diff.target == settled => Event::Settled(keys(map)),
                _ if *diff.target == style => Event::Style,
                _ => Event::Meta,
            });
        }
    }));
    (events, watch)
}

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

/// The replica a change made outside Letters is written as: a hash of the revision it starts from, of
/// what it does to the sequence, and of the fields or the style it leads to.
///
/// Two replicas that take in the same outside change therefore write the very same change, and it merges
/// as one. The hash covers the change, so a build that tells two documents apart differently can never
/// claim the same id for different content. Change the tag if the way [`write`] or [`assign`] carries a
/// change out changes.
pub(crate) fn outsider(from: &str, delta: &[TextDelta], to: &str) -> u64 {
    let mut hash = Sha256::new();
    feed(&mut hash, b"letters outside change 1");
    feed(&mut hash, from.as_bytes());
    for item in delta {
        let (kind, count, text, attributes) = match item {
            TextDelta::Retain { retain, attributes } => (b"retain", *retain, "", attributes.as_ref()),
            TextDelta::Insert { insert, attributes } => (b"insert", 0, insert.as_str(), attributes.as_ref()),
            TextDelta::Delete { delete } => (b"delete", *delete, "", None),
        };
        feed(&mut hash, kind);
        feed(&mut hash, &(count as u64).to_le_bytes());
        feed(&mut hash, text.as_bytes());
        let attributes: BTreeMap<_, _> = attributes.into_iter().flatten().collect();
        feed(&mut hash, &(attributes.len() as u64).to_le_bytes());
        for (key, value) in attributes {
            feed(&mut hash, key.as_bytes());
            feed_value(&mut hash, value);
        }
    }
    feed(&mut hash, b"to");
    feed(&mut hash, to.as_bytes());
    peer(hash)
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
    let properties = doc.properties.iter().map(|properties| properties.to_value().to_string());
    calls.extend(properties.map(|json| Call::Set(META, PROPERTIES.into(), json.into())));
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
fn author(tag: &[u8], calls: &[Call]) -> u64 {
    let mut hash = Sha256::new();
    feed(&mut hash, tag);
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

/// Starts a history that holds a document as its first change and its style as its second. See [`plan`]
/// for which documents.
///
/// The style is a change of its own, written by a replica named after the document's change as well. Two
/// devices that open the same document under different styles then still share the document.
pub(crate) fn create(doc: &Document, index: &[Entry], style: &Style) -> LoroDoc {
    let loro = LoroDoc::new();
    configure(&loro);
    let (body, markers) = (loro.get_text(BODY), Markers::of(&loro));
    let calls = plan(doc, index);
    let first = author(b"letters first change 1", &calls);
    let fields = style_fields(style).into_iter().map(|(key, value)| Call::Set(STYLE, key, value.into()));
    let fields: Vec<Call> = fields.collect();
    let second = author(&[b"letters first style 1".as_slice(), &first.to_le_bytes()].concat(), &fields);
    for (peer, calls) in [(first, calls), (second, fields)] {
        // None of these can fail: the peer is in range, the ranges lie inside the block just written, and
        // no key holds a colon. A failure would leave a shorter document, which the model then shows as
        // it is.
        let _ = loro.set_peer_id(peer);
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
    }
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
    doc.properties = None;
    each(loro, META, |key, value| match (key, text(value)) {
        (LANGUAGE, Some(language)) => language.clone_into(&mut doc.language),
        (PROPERTIES, Some(json)) => {
            let read = serde_json::from_str(json).ok().map(|value| Properties::from_value(value, ""));
            doc.properties = read.and_then(Result::ok);
        }
        _ => {}
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
    if let Some(properties) = &mut doc.properties
        && let Ok(read) = Properties::from_value(properties.to_value(), "")
    {
        *properties = read;
    }
}

/// One value in one of the history's maps, as an edit sets it. `None` takes the key out.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Assign {
    map: &'static str,
    key: String,
    value: Option<LoroValue>,
}

/// Makes assignments in the history, and gives the ones that put back what they replaced.
pub(crate) fn assign(loro: &LoroDoc, assigns: &[Assign]) -> Option<Vec<Assign>> {
    let mut back = Vec::with_capacity(assigns.len());
    for Assign { map, key, value } in assigns {
        let held = loro.get_map(*map);
        let was = Markers::value(&held, key);
        if was != *value {
            match value {
                Some(value) => held.insert(key, value.clone()).ok()?,
                None => held.delete(key).ok()?,
            }
            back.push(Assign { map, key: key.clone(), value: was });
        }
    }
    back.reverse();
    Some(back)
}

/// The assignments that make the document's own fields in the history those of `to`, where they differ
/// from `from`.
pub(crate) fn meta_assigns(loro: &LoroDoc, from: &Document, to: &Document) -> Vec<Assign> {
    let set = |map, key: &str, value: Option<LoroValue>| Assign { map, key: key.to_owned(), value };
    let mut assigns = Vec::new();
    if from.language != to.language {
        assigns.push(set(META, LANGUAGE, Some(to.language.as_str().into())));
    }
    if from.properties != to.properties {
        let json = to.properties.as_ref().map(|properties| properties.to_value().to_string().into());
        assigns.push(set(META, PROPERTIES, json));
    }
    // A word goes with every add of it this replica has seen. The history lists them in no order, and an
    // outside change must be written in one.
    let mut gone = BTreeSet::new();
    loro.get_map(DICTIONARY).for_each(|key, _| {
        if from.dictionary.contains(word_of(key)) && !to.dictionary.contains(word_of(key)) {
            gone.insert(key.to_owned());
        }
    });
    assigns.extend(gone.iter().map(|key| set(DICTIONARY, key, None)));
    // The number of this replica's next edit, which no other add can have.
    let peer = loro.peer_id();
    let add = format!("{}@{peer}", loro.oplog_vv().get(&peer).copied().unwrap_or(0));
    let added = to.dictionary.difference(&from.dictionary);
    assigns.extend(added.map(|word| set(DICTIONARY, &word_key(word, &add), Some(true.into()))));
    let gone = from.extra.keys().filter(|key| !to.extra.contains_key(*key));
    assigns.extend(gone.map(|key| set(EXTRA, key, None)));
    let changed = to.extra.iter().filter(|(key, value)| from.extra.get(*key) != Some(value));
    assigns.extend(changed.map(|(key, value)| set(EXTRA, key, Some(value.to_string().into()))));
    assigns
}

/// A style as the history holds it: each field under a key of its own, as the JSON `style.json` gives it.
fn style_fields(style: &Style) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let Ok(Value::Object(root)) = serde_json::from_str(&style.to_json()) else { return fields };
    let key = |parts: &[&str]| parts.iter().map(|part| clean(part)).collect::<Vec<_>>().join(MARKER);
    for (name, value) in root {
        match (name.as_str(), value) {
            ("format", _) => {}
            ("types", Value::Object(types)) => {
                for (element, style) in types {
                    let Value::Object(style) = style else { continue };
                    fields.extend(
                        style.iter().map(|(field, value)| (key(&["types", &element, field]), value.to_string())),
                    );
                }
            }
            ("page", Value::Object(page)) => {
                fields.extend(page.iter().map(|(field, value)| (key(&["page", field]), value.to_string())));
            }
            (_, value) => drop(fields.insert(key(&["extra", &name]), value.to_string())),
        }
    }
    fields
}

/// The style that fields give. A value that breaks the schema is left out, as two changes that each made
/// sense can do once merged, and the built-in value stands in for it.
fn style_of(fields: impl IntoIterator<Item = (String, String)>) -> Style {
    fn object<'a>(map: &'a mut Map<String, Value>, key: &str) -> Option<&'a mut Map<String, Value>> {
        map.entry(key).or_insert_with(|| Value::Object(Map::new())).as_object_mut()
    }
    /// Takes out the value an error names. False when there is no telling which it is.
    fn leave_out(root: &mut Map<String, Value>, path: &str) -> bool {
        let Some((part, rest)) = path.split_once('.') else { return root.remove(path).is_some() };
        let Some(Value::Object(inside)) = root.get_mut(part) else { return false };
        // The name of an element type may hold a dot of its own.
        let element =
            inside.keys().find(|name| rest.strip_prefix(name.as_str()).is_some_and(|field| field.starts_with('.')));
        match element.cloned() {
            Some(name) => match inside.get_mut(&name) {
                Some(Value::Object(style)) => {
                    style.remove(&rest[name.len() + 1..]).is_some() || inside.remove(&name).is_some()
                }
                _ => inside.remove(&name).is_some(),
            },
            None => inside.remove(rest).is_some() || root.remove(part).is_some(),
        }
    }

    let (mut root, mut types, mut page) = (Map::new(), Map::new(), Map::new());
    for (key, json) in fields {
        let Ok(value) = serde_json::from_str::<Value>(&json) else { continue };
        let mut parts = key.split(MARKER);
        match (parts.next(), parts.next(), parts.next()) {
            (Some("types"), Some(element), Some(field)) => {
                object(&mut types, element).map(|style| style.insert(field.to_owned(), value));
            }
            (Some("page"), Some(field), None) => drop(page.insert(field.to_owned(), value)),
            (Some("extra"), Some(name), None) => drop(root.insert(name.to_owned(), value)),
            _ => {}
        }
    }
    root.insert("format".into(), STYLE_FORMAT.into());
    root.insert("types".into(), types.into());
    root.insert("page".into(), page.into());
    loop {
        match Style::from_json(&Value::Object(root.clone()).to_string()) {
            Ok(style) => return style,
            Err(error) if leave_out(&mut root, &error.path) => {}
            Err(_) => return Style::default(),
        }
    }
}

/// A style as a history gives it back.
pub(crate) fn normal_style(style: &Style) -> Style {
    style_of(style_fields(style))
}

pub(crate) fn read_style(loro: &LoroDoc) -> Style {
    let mut fields = Vec::new();
    loro.get_map(STYLE).for_each(|key, value| {
        if let ValueOrContainer::Value(LoroValue::String(json)) = value {
            fields.push((key.to_owned(), json.to_string()));
        }
    });
    style_of(fields)
}

/// The assignments that make the style in the history `to`, field by field. They are told from what the
/// history holds, which may be more than the style it reads as: see [`style_of`].
pub(crate) fn style_assigns(loro: &LoroDoc, to: &Style) -> Vec<Assign> {
    let (mut old, new) = (BTreeMap::new(), style_fields(to));
    loro.get_map(STYLE).for_each(|key, value| {
        if let ValueOrContainer::Value(LoroValue::String(json)) = value {
            old.insert(key.to_owned(), json.to_string());
        }
    });
    let gone = old.keys().filter(|key| !new.contains_key(*key)).map(|key| (key.clone(), None));
    let set = new.iter().filter(|(key, json)| old.get(*key) != Some(json));
    let set = set.map(|(key, json)| (key.clone(), Some(json.as_str().into())));
    gone.chain(set).map(|(key, value)| Assign { map: STYLE, key, value }).collect()
}
