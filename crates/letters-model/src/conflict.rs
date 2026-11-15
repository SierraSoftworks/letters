//! Blocks that two writers changed in the same words while apart.
//!
//! Changes that do not touch merge by themselves, even inside one paragraph. Where both sides changed one
//! word, the merge keeps what each of them typed, and the block reads as neither meant it. The same holds
//! for text typed into a block that the other side took away. [`Model::conflicts`](crate::Model::conflicts)
//! finds those blocks, with both versions for a person to choose from, and
//! [`Model::settle`](crate::Model::settle) makes each read as the side that was edited last left it.
//!
//! # Settling
//!
//! Every device that merges the same two revisions settles them with the very same change, or the copies
//! of what it puts back would add up once the devices meet. So the change is not written where the device
//! stands, which can be further on. It is written at the point in history where the two revisions have
//! just met, and named after what it does as an outside change is. The same change written twice merges
//! as one.
//!
//! What it does is told by character. The later side's block stays: its characters that the earlier side
//! cut are put in again where they stood, and the characters the earlier side put into its own block are
//! taken out. Formatting and the kind of the block merge as they do anywhere, and a block that one side
//! joined to its neighbour stays joined.
//!
//! Two devices that merge different revisions, as when one of them wrote on before it merged, settle a
//! block by different changes. Each settling is noted against its block, so the next search finds a block
//! that both sides settled, and the later one stands.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::mem;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

use loro::{ExportMode, Frontiers, ID, JsonOpContent, JsonTextOp, LoroDoc, LoroText, TextDelta, VersionVector};
use unicode_segmentation::UnicodeSegmentation;

use crate::crdt::{self, Event};
use crate::flat::{self, Entry, MARKER};
use crate::model::unreadable;
use crate::notes::Version;
use crate::{Block, BlockId, BodyShape, Document, Extra, HistoryError, Revision};

/// A block that two revisions both changed, in the same words, since they last agreed.
#[derive(Clone, Debug, PartialEq)]
pub struct Conflict {
    pub block: BlockId,
    /// The two sides, in the order their revisions were given.
    pub sides: [Side; 2],
    /// Which of the sides was edited last. Every device finds the same one, whichever it names first.
    pub later: usize,
}

/// What one revision made of the block of a [`Conflict`].
#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    /// The block as this side left it. `None` when this side took it away.
    pub block: Option<Block>,
    /// Who last changed it on this side. Empty when that writer gave no name. Where a settled conflict was
    /// the last change, it is the writer of the version that stood, and that version's time.
    pub author: String,
    /// When, in seconds since 1970 and right to the minute. A writer who never told the time is taken to
    /// have written when the newest edit it built on was made, and at 0 when none of those told either.
    pub edited: u64,
}

impl Conflict {
    /// The two versions as a conflict note keeps them. `None` when a side took the block away.
    pub fn versions(&self) -> Option<[Version; 2]> {
        let whole = self.sides.iter().all(|side| side.block.is_some());
        whole.then(|| [self.sides[0].version(), self.sides[1].version()])
    }
}

impl Side {
    /// This side as a conflict note keeps it, with no block when this side took it away.
    pub fn version(&self) -> Version {
        Version {
            author: self.author.clone(),
            edited: rfc3339(self.edited),
            block: self.block.clone(),
            extra: Extra::new(),
        }
    }
}

/// A time in seconds since 1970 as RFC 3339 writes it, in UTC.
fn rfc3339(seconds: u64) -> String {
    // The last second of the year 9999, past which the form has no room.
    let seconds = seconds.min(253_402_300_799);
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    // Days are counted in eras of 400 years that start on 1 March, so that a leap day ends its year.
    let days = days + 719_468;
    let (era, day) = (days / 146_097, days % 146_097);
    let year = (day - day / 1_460 + day / 36_524 - day / 146_096) / 365;
    let day = day - (365 * year + year / 4 - year / 100);
    let month = (5 * day + 2) / 153;
    let day = day - (153 * month + 2) / 5 + 1;
    let month = if month < 10 { month + 3 } else { month - 9 };
    let year = year + era * 400 + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3_600, rest / 60 % 60, rest % 60)
}

/// Text that one side put into the sequence.
struct Put {
    /// The first and the last of the old places it can have stood at. Loro tells of what was cut at a
    /// place before what was put in there, whichever came first in the sequence, so text put in beside
    /// characters that were cut is known to no closer than their whole stretch.
    place: (usize, usize),
    /// Where it is in the side's own sequence, and how many characters.
    at: usize,
    chars: usize,
    /// Whether it runs on from the text before it, and is not all in a block of its own.
    runs: bool,
    /// Whether it holds a marker: a block of the side's own, or a block cut in two.
    row: bool,
    /// Its first and its last character.
    ends: (char, char),
}

/// What one side did to the sequence since the two last agreed, in the places of that time.
#[derive(Default)]
struct Edits {
    /// The characters it took out, in order.
    cut: Vec<Range<usize>>,
    /// All it put in, in order.
    put: Vec<Put>,
}

impl Edits {
    /// Reads them from the change as Loro tells it. A change of formatting alone is none: it merges.
    fn of(raw: &[TextDelta]) -> Self {
        // Where the change has come to, in the sequence it starts from and in the one it leads to.
        let (mut edits, mut at, mut to) = (Self::default(), 0, 0);
        // Since the last character that stayed: where the cuts began, what was put in, and whether a
        // marker was among it.
        let (mut from, mut first, mut own) = (None, 0, false);
        for item in raw {
            match item {
                TextDelta::Retain { retain, .. } => {
                    (at, to) = (at + retain, to + retain);
                    (from, first, own) = (None, edits.put.len(), false);
                }
                TextDelta::Delete { delete } => {
                    from.get_or_insert(at);
                    edits.cut.push(at..at + delete);
                    at += delete;
                    edits.put[first..].iter_mut().for_each(|put| put.place.1 = at);
                }
                TextDelta::Insert { insert, .. } => {
                    let Some(ends) = insert.chars().next().zip(insert.chars().next_back()) else { continue };
                    let (chars, row) = (insert.chars().count(), insert.contains(MARKER));
                    // Text behind a marker went into a block of its own, and none of it in here.
                    let runs = !own && !insert.starts_with(MARKER);
                    edits.put.push(Put { place: (*from.get_or_insert(at), at), at: to, chars, runs, row, ends });
                    (to, own) = (to + chars, own || row);
                }
            }
        }
        edits
    }

    /// The last of the characters in `range` that this side left in.
    fn last_kept(&self, range: Range<usize>) -> Option<usize> {
        let mut end = range.end;
        for cut in self.cut.iter().rev().skip_while(|cut| cut.start >= range.end) {
            if cut.end < end {
                break;
            }
            end = end.min(cut.start);
        }
        (end > range.start).then(|| end - 1)
    }

    /// Where the characters it took out right before `at` begin: `at` itself when the one before it stayed.
    fn cut_from(&self, at: usize) -> usize {
        let mut from = at;
        let before = self.cut.partition_point(|cut| cut.start < at);
        for cut in self.cut[..before].iter().rev().take_while(|cut| !cut.is_empty()) {
            if cut.end < from {
                break;
            }
            from = cut.start;
        }
        from
    }

    /// Whether this side took out the character at `at`.
    fn cuts(&self, at: usize) -> bool {
        let next = self.cut.partition_point(|cut| cut.end <= at);
        self.cut.get(next).is_some_and(|cut| cut.contains(&at))
    }

    /// What this side put in that can have stood anywhere from `from` to `to`.
    fn puts(&self, from: usize, to: usize) -> impl Iterator<Item = &Put> {
        let first = self.put.partition_point(|put| put.place.1 < from);
        self.put[first..].iter().take_while(move |put| put.place.0 <= to)
    }
}

/// What one side did to one word: the characters it cut from it, and whether it put text into it.
type Touch = (BTreeSet<usize>, bool);

/// Whether two characters that stand side by side read on as one word, with no word boundary between.
fn joins(before: char, after: char) -> bool {
    let pair: String = [before, after].into_iter().collect();
    pair.split_word_bounds().nth(1).is_none()
}

/// What one side did to the text of one block.
#[derive(Default)]
struct Done {
    /// The block left its place: its marker and all of its text were taken out. Whatever the side put
    /// there instead is in another block.
    gone: bool,
    changed: bool,
    /// By the word's place among the words of the block.
    words: BTreeMap<usize, Touch>,
}

impl Done {
    /// `text` is where the block's text starts in the sequence, `chars` are its characters, and `words`
    /// what [`words`] makes of them.
    fn of(edits: &Edits, text: usize, entry: &Entry, chars: &[char], words: &[(Range<usize>, bool)]) -> Self {
        let (start, end) = (text - usize::from(entry.marker), text + entry.chars);
        let mut done = Self::default();
        // Whether the marker went. A block with no marker of its own goes with its text.
        let (mut loose, mut cut_chars) = (!entry.marker && entry.chars > 0, 0);
        let first = edits.cut.partition_point(|cut| cut.end <= start);
        for cut in edits.cut[first..].iter().take_while(|cut| cut.start < end) {
            loose |= entry.marker && cut.contains(&start);
            let (from, to) = (cut.start.max(text), cut.end.min(end));
            if from >= to {
                continue;
            }
            (done.changed, cut_chars) = (true, cut_chars + to - from);
            let (from, to) = (from - text, to - text);
            let first = words.partition_point(|(word, _)| word.end <= from);
            for (nth, (word, _)) in words.iter().enumerate().skip(first).take_while(|(_, (word, _))| word.start < to) {
                done.words.entry(nth).or_default().0.extend(from.max(word.start)..to.min(word.end));
            }
        }
        done.gone = loose && cut_chars == entry.chars;
        // The word that holds a character of the block, by the character's place in it.
        let word = |at: usize| words.partition_point(|(word, _)| word.end <= at);
        let puts: Vec<&Put> = edits.puts(text, end).collect();
        // All a side put in at one place stands together, between the old characters around that place.
        for stretch in puts.chunk_by(|a, b| a.place == b.place).filter(|stretch| stretch[0].runs) {
            done.changed = true;
            let ((lo, hi), head, tail) = (stretch[0].place, stretch[0].ends.0, stretch[stretch.len() - 1].ends.1);
            let mut put = |nth: usize| done.words.entry(nth).or_default().1 = true;
            // Put in where characters were cut, it stands for them: in the first and the last of their words.
            let (from, to) = (lo.max(text), hi.min(end));
            if from < to {
                put(word(from - text));
                put(word(to - 1 - text));
            }
            // Text beside a word changes it only when it reads on from the word, or into it. A space or a
            // full stop between them leaves the word as it was, and what another writer did to it stands.
            let before = lo.checked_sub(text + 1).filter(|_| lo <= end);
            let after = hi.checked_sub(text).filter(|_| hi < end);
            for (at, forward) in before.map(|at| (at, false)).into_iter().chain(after.map(|at| (at, true))) {
                let nth = word(at);
                let Some(((range, true), old)) = words.get(nth).zip(chars.get(at)) else { continue };
                let inside = if forward { at > range.start } else { at + 1 < range.end };
                if inside || if forward { joins(tail, *old) } else { joins(*old, head) } {
                    put(nth);
                }
            }
        }
        done
    }

    /// Whether what two sides did to a block cannot both stand.
    fn clashes(&self, other: &Self) -> bool {
        match (self.gone, other.gone) {
            (true, true) => false,
            (true, false) => other.changed,
            (false, true) => self.changed,
            // Cutting the same characters is one cut once merged, and no harm.
            (false, false) => self.words.iter().any(|(word, (cut, put))| {
                other.words.get(word).is_some_and(|(theirs, also)| *put || *also || cut != theirs)
            }),
        }
    }
}

/// The words of a text and what lies between them, each as a range of characters, with whether it is a
/// word: more than space and punctuation.
fn words(chars: &[char]) -> Vec<(Range<usize>, bool)> {
    let text: String = chars.iter().collect();
    let mut at = 0;
    let word = |word: &str| {
        let start = at;
        at += word.chars().count();
        (start..at, word.chars().any(char::is_alphanumeric))
    };
    text.split_word_bounds().map(word).collect()
}

/// A block of a clash, as the two sides last agreed on it.
struct Old {
    block: BlockId,
    /// Whether it left its place on either side.
    gone: [bool; 2],
    /// Where it stood in the sequence: its marker, if it had one, and its text. Empty for a block that was
    /// not there, which both sides have since put in.
    place: Range<usize>,
    marker: bool,
}

/// How many of the blocks after a clash are tried as the place to put its block back in.
const NEAR: usize = 8;

/// The blocks of `old` in which the two sides' edits cannot both stand, and those among `again`, which
/// both sides settled apart. `near` takes the places of the markers that follow each.
fn clashes(
    old: &Document,
    index: &[Entry],
    edits: [&Edits; 2],
    again: &BTreeSet<String>,
    near: &mut Vec<Vec<usize>>,
) -> Vec<Old> {
    // Where each block starts in the sequence, and where its text does.
    let (mut starts, mut texts) = (Vec::with_capacity(index.len()), Vec::with_capacity(index.len()));
    let mut at = 0;
    for entry in index {
        starts.push(at);
        texts.push(at + usize::from(entry.marker));
        at += entry.len();
    }
    let touched = |edits: &Edits| {
        let holds = |at: usize| starts.partition_point(|start| *start <= at).saturating_sub(1);
        let mut blocks = BTreeSet::new();
        for cut in edits.cut.iter().filter(|cut| !cut.is_empty()) {
            blocks.extend(holds(cut.start)..=holds(cut.end - 1));
        }
        // Text put right before a marker ends the block before it.
        let ends = |at: usize| texts.partition_point(|text| *text <= at).checked_sub(1);
        let put = edits.put.iter().filter(|put| put.runs);
        blocks.extend(put.flat_map(|put| [put.place.0, put.place.1]).filter_map(ends));
        blocks
    };
    let (ours, theirs) = (touched(edits[0]), touched(edits[1]));
    let order = flat::order(&old.blocks, index);
    // Those among `again` that were there, and the rest.
    let mut absent: BTreeSet<&str> = again.iter().map(String::as_str).collect();
    let there = |(nth, (block, _)): (usize, &(&Block, &Entry))| absent.remove(block.id.as_str()).then_some(nth);
    let again: BTreeSet<usize> = order.iter().enumerate().filter_map(there).collect();
    let both: BTreeSet<usize> = ours.intersection(&theirs).chain(&again).copied().collect();
    let mut found = Vec::new();
    for nth in both.into_iter().filter(|nth| index[*nth].shape == BodyShape::Text) {
        let Some((block, entry)) = order.get(nth) else { continue };
        let chars: Vec<char> = block.plain_text().chars().collect();
        let words = words(&chars);
        let done = edits.map(|edits| Done::of(edits, texts[nth], entry, &chars, &words));
        if again.contains(&nth) || done[0].clashes(&done[1]) {
            let place = starts[nth]..starts[nth] + entry.len();
            found.push(Old {
                block: block.id.clone(),
                gone: [done[0].gone, done[1].gone],
                place,
                marker: entry.marker,
            });
            let after = index.iter().zip(&starts).skip(nth + 1).filter(|(entry, _)| entry.marker);
            near.push(after.take(NEAR).map(|(_, start)| *start).collect());
        }
    }
    // A block that was not there has no place, and is gone from a side that does not hold it.
    for block in absent {
        found.push(Old { block: BlockId::new(block), gone: [true, true], place: 0..0, marker: false });
        near.push(Vec::new());
    }
    found
}

/// Whether both sides changed one stretch of the sequence from a marker to the next, as a clash takes.
/// It is told from the sequence alone, which is read far sooner than the document it holds.
fn meet(sequence: &str, edits: [&Edits; 2]) -> bool {
    let markers: Vec<usize> =
        sequence.chars().enumerate().filter_map(|(at, c)| MARKER.starts_with(c).then_some(at)).collect();
    let touched = |edits: &Edits| {
        let holds = |at: usize| markers.partition_point(|marker| *marker <= at);
        let mut stretches = BTreeSet::new();
        for cut in edits.cut.iter().filter(|cut| !cut.is_empty()) {
            stretches.extend(holds(cut.start)..=holds(cut.end - 1));
        }
        // Text put in belongs with the character before it.
        let put = edits.put.iter().filter(|put| put.runs);
        stretches.extend(put.flat_map(|put| [put.place.0, put.place.1]).map(|at| holds(at.saturating_sub(1))));
        stretches
    };
    !touched(edits[0]).is_disjoint(&touched(edits[1]))
}

/// An edit's place in time, which is the same on every device: later by the clock first, then later in
/// the order of the history itself, which no two edits share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Stamp {
    time: i64,
    lamport: u32,
    replica: u64,
}

impl Stamp {
    fn of(loro: &LoroDoc, id: ID) -> Option<Self> {
        let change = loro.get_change(id)?;
        let lamport = change.lamport + u32::try_from(id.counter - change.id.counter).unwrap_or_default();
        Some(Self { time: change.timestamp, lamport, replica: id.peer })
    }
}

/// What one side holds of a block of a clash.
#[derive(Default)]
struct Held {
    /// `None` when it is not there.
    block: Option<Block>,
    /// The block as a change that puts it into the sequence, and with it the blocks the side cut from it:
    /// all it made of the block's old text.
    pieces: Vec<TextDelta>,
    /// The id of each character of that, in order.
    ids: Vec<ID>,
    /// The ids of whatever else the side put in between the block's old text.
    beside: Vec<ID>,
    /// The ids of the text it put in right before the block, which ends the block before.
    ahead: Vec<ID>,
    /// The id of the character that stands before the block.
    prior: Option<ID>,
}

/// What the history holds of each block at the revision it shows, which is that of the side whose `edits`
/// these are. `other` are those of the other side, and `olds` the ids each block had when the two last
/// agreed.
fn views(loro: &LoroDoc, body: &LoroText, blocks: &[Old], olds: &[Vec<ID>], edits: &Edits, other: &Edits) -> Vec<Held> {
    let (doc, index) = crdt::read(loro, body);
    let order = flat::order(&doc.blocks, &index);
    let view = |(old, ids): (&Old, &Vec<ID>)| {
        let text = old.place.start + usize::from(old.marker)..old.place.end;
        let mut held = Held::default();
        let mut own = 0..0;
        let found = order.iter().position(|(block, _)| block.id == old.block).map(|nth| {
            let start: usize = index[..nth].iter().map(Entry::len).sum();
            (nth, start, old.marker && crdt::id_at(body, start) == ids.first().copied())
        });
        // Another block can have taken the id on this side, while the block itself stands under another.
        // Which of the two the id then means is not to be told, and the block is left to merge.
        let stands = || ids.first().is_some_and(|marker| crdt::place(loro, body, *marker).is_some());
        if let Some((nth, start, same)) = found.filter(|(.., same)| *same || !old.marker || !stands()) {
            let mut end = start + index[nth].len();
            // A side that cut the block in two holds its old text in more blocks than one. They are its
            // version of the block together, up to the end of the one that holds the last of that text.
            let split = edits.puts(text.start, text.end).any(|put| put.row && put.place.0 < text.end);
            if split && same {
                let last = edits.last_kept(text.clone()).and_then(|last| ids.get(last - old.place.start));
                let last = last.and_then(|last| crdt::place(loro, body, *last));
                for entry in &index[nth + 1..] {
                    if last.is_none_or(|last| last < end) {
                        break;
                    }
                    end += entry.len();
                }
            }
            if let Some((pieces, ids)) = crdt::stretch(loro, body, start..end) {
                let prior = start.checked_sub(1).and_then(|at| crdt::id_at(body, at));
                let block = Some(order[nth].0.clone());
                (held, own) = (Held { block, pieces, ids, prior, ..Held::default() }, start..end);
            }
        }
        // What else it put in where the old text stood can come to stand in the block's way, so the
        // settling has to know of it. That goes for what it put before a marker it cut, and for what it
        // put after the text when the other side put something there too. Anything else it wrote before
        // or after the block is no edit of the block, and says nothing of when the block was last edited.
        if !old.place.is_empty() {
            let headless = !old.marker || edits.cuts(old.place.start);
            let from = if headless { old.place.start } else { text.start };
            let after = other.puts(text.end, text.end).next().is_some();
            let between = edits.puts(from, text.end).filter(|put| after || put.place.0 < text.end);
            let places = between.flat_map(|put| put.at..put.at + put.chars).filter(|at| !own.contains(at));
            held.beside = places.filter_map(|at| crdt::id_at(body, at)).collect();
            // Text it put in right before the block is no edit of the block. A merge can still leave it
            // standing behind what the other side made of the block. So can text it put in further back,
            // when the other side took out all that stood between.
            let reach = other.cut_from(old.place.start);
            let ahead = edits.puts(reach, old.place.start).filter(|put| !headless && put.runs);
            let places = ahead.flat_map(|put| put.at..put.at + put.chars);
            held.ahead = places.filter_map(|at| crdt::id_at(body, at)).collect();
        }
        held
    };
    blocks.iter().zip(olds).map(view).collect()
}

/// The characters among `wanted` that one side took out, between the revision `from` that both sides
/// share and its own, each against the earliest of the edits that cut it.
fn strikes(
    loro: &LoroDoc,
    body: &LoroText,
    from: &VersionVector,
    to: &VersionVector,
    wanted: &HashSet<ID>,
) -> Result<HashMap<ID, ID>, HistoryError> {
    let edits = loro.export_json_updates_without_peer_compression(from, to).map_err(unreadable)?;
    let sequence = loro::ContainerTrait::id(body);
    let mut struck: HashMap<ID, (u32, ID)> = HashMap::new();
    for change in &edits.changes {
        for op in change.ops.iter().filter(|op| op.container == sequence) {
            let JsonOpContent::Text(JsonTextOp::Delete { len, start_id, .. }) = &op.content else { continue };
            // A cut made backwards, as the backspace key makes it, takes its last character first.
            let count = len.abs();
            for nth in 0..count {
                let aim = if *len > 0 { nth } else { count - 1 - nth };
                let gone = ID::new(start_id.peer, start_id.counter + aim);
                if wanted.contains(&gone) {
                    let by = ID::new(change.id.peer, op.counter + nth);
                    let when = change.lamport + u32::try_from(by.counter - change.id.counter).unwrap_or_default();
                    let first = struck.entry(gone).or_insert((when, by));
                    if (when, by.peer) < (first.0, first.1.peer) {
                        *first = (when, by);
                    }
                }
            }
        }
    }
    Ok(struck.into_iter().map(|(gone, (_, by))| (gone, by)).collect())
}

/// A conflict, and what it takes to settle it.
pub(crate) struct Clash {
    conflict: Conflict,
    /// Whether a settling was the last to change the block on both sides. Two devices then settled it
    /// apart, each telling of the conflict it found, and no person has written in it since.
    settled: bool,
    /// Whether the block stood there when the sides last agreed. Those that did are in the order they
    /// stood in.
    old: bool,
    /// What the later side holds of the block, as a change that puts it into the sequence, and the id of
    /// each of its characters.
    keep: (Vec<TextDelta>, Vec<ID>),
    /// Those of them that the earlier side cut.
    lost: HashSet<ID>,
    /// The character that stands before the block on the later side.
    prior: Option<ID>,
    /// All the earlier side holds of the block.
    held: Vec<ID>,
    /// Those of them that it put in.
    drop: Vec<ID>,
    /// What else the earlier side put in where the block stood or right before it: text that runs on from
    /// the block before, then blocks of its own. This block goes between the two.
    beside: Vec<ID>,
    /// The markers of the blocks that followed the block when the sides last agreed, nearest first.
    near: Vec<ID>,
}

/// Finds the conflicts between two revisions. It reads the history at other revisions, and can leave it
/// showing one of them: the caller brings back the one its document shows.
pub(crate) fn find(
    history: &LoroDoc,
    body: &LoroText,
    events: &Mutex<Vec<Event>>,
    sides: [&Frontiers; 2],
) -> Result<Vec<Clash>, HistoryError> {
    let held = |at: &Frontiers| history.frontiers_to_vv(at).ok_or(HistoryError::UnknownRevision);
    let reach = [held(sides[0])?, held(sides[1])?];
    // A side that holds all of the other changed nothing apart from it.
    if reach[0].partial_cmp(&reach[1]).is_some() {
        return Ok(Vec::new());
    }
    let shared = reach[0].intersection(&reach[1]);
    // A step through the history costs Loro a pass over all the text that comes or goes with it. The
    // history behind a document shows what both sides wrote: it steps back from there, and returns.
    // Sides that wrote more apart than they share, as after a long paste, are read in a copy that shows
    // nothing at first. It reads what the side that wrote more did once, where the history itself
    // would pass over it up to four times.
    if size(&reach[0]) + size(&reach[1]) <= 3 * size(&shared) {
        return read(history, body, events, sides, reach, &shared, false);
    }
    let copy = LoroDoc::new();
    crdt::configure(&copy);
    // Detached, it takes the edits in and shows none of them.
    copy.detach();
    copy.import(&history.export(ExportMode::all_updates()).map_err(unreadable)?).map_err(unreadable)?;
    let body = copy.get_text(crdt::BODY);
    let (events, _watch) = crdt::watch(&copy, &body);
    read(&copy, &body, &events, sides, reach, &shared, true)
}

/// How much a revision holds, as Loro counts: an edit of a text counts once for each of its characters.
fn size(held: &VersionVector) -> i64 {
    held.values().map(|counter| i64::from(*counter)).sum()
}

/// Reads the conflicts between two revisions out of `loro`, which it shows at one revision after another.
/// `reach` is what each side holds and `shared` what both do. `light` says that this is little beside
/// what the sides wrote since: its document is then read at once, which costs less than a way back to it.
fn read(
    loro: &LoroDoc,
    body: &LoroText,
    events: &Mutex<Vec<Event>>,
    sides: [&Frontiers; 2],
    reach: [VersionVector; 2],
    shared: &VersionVector,
    light: bool,
) -> Result<Vec<Clash>, HistoryError> {
    let base = loro.vv_to_frontiers(shared);
    // Shows a revision, and gives the change to the sequence that led there with the blocks whose
    // conflicts were settled on the way.
    let show = |at: &Frontiers| {
        let queue = || events.lock().unwrap_or_else(PoisonError::into_inner);
        queue().clear();
        loro.checkout(at).map_err(|_| HistoryError::UnknownRevision)?;
        let (mut delta, mut settled) = (None, BTreeSet::new());
        for event in mem::take(&mut *queue()) {
            match event {
                Event::Body(body) => {
                    debug_assert!(delta.is_none(), "one step through the history told of the sequence twice");
                    delta = Some(body);
                }
                Event::Settled(blocks) => settled.extend(blocks),
                _ => {}
            }
        }
        Ok::<_, HistoryError>((Edits::of(&delta.unwrap_or_default()), settled))
    };
    show(&base)?;
    let sequence = body.to_string();
    let old = light.then(|| crdt::read(loro, body));
    // The step back between the sides is from the one read first: the one that wrote less.
    let lighter = usize::from(size(&reach[1]) < size(&reach[0]));
    let (one, settled) = show(sides[lighter])?;
    show(&base)?;
    let (other, also) = show(sides[1 - lighter])?;
    let (first, second) = if lighter == 0 { (one, other) } else { (other, one) };
    let again: BTreeSet<String> = settled.intersection(&also).cloned().collect();
    // Most copies that part change different blocks, and are done here.
    if again.is_empty() && !meet(&sequence, [&first, &second]) {
        return Ok(Vec::new());
    }
    let (old, index) = match old {
        Some(old) => old,
        None => {
            show(&base)?;
            crdt::read(loro, body)
        }
    };
    let mut near = Vec::new();
    let blocks = clashes(&old, &index, [&first, &second], &again, &mut near);
    if blocks.is_empty() {
        return Ok(Vec::new());
    }
    // The ids of the old characters are read where the sides last agreed.
    show(&base)?;
    let ids = |places: &mut dyn Iterator<Item = usize>| -> Vec<ID> {
        places.filter_map(|at| crdt::id_at(body, at)).collect()
    };
    let olds: Vec<Vec<ID>> = blocks.iter().map(|old| ids(&mut old.place.clone())).collect();
    let near: Vec<Vec<ID>> = near.into_iter().map(|near| ids(&mut near.into_iter())).collect();
    show(sides[1])?;
    let (theirs, their_names) = (views(loro, body, &blocks, &olds, &second, &first), crdt::names(loro));
    let their_settlings = crdt::settlings(loro);
    show(sides[0])?;
    let (ours, our_names) = (views(loro, body, &blocks, &olds, &first, &second), crdt::names(loro));
    let (names, settlings) = ([our_names, their_names], [crdt::settlings(loro), their_settlings]);
    let settlers = settlings.each_ref().map(|side| side.values().map(|by| by.0).collect::<HashSet<u64>>());

    // The characters either side may have cut: the old ones of the blocks and of what stands with them.
    let is_old = |id: &ID| shared.includes_id(*id);
    let held = ours.iter().chain(&theirs).flat_map(|held| &held.ids);
    let wanted: HashSet<ID> = olds.iter().flatten().chain(held).copied().filter(is_old).collect();
    let struck = [strikes(loro, body, shared, &reach[0], &wanted)?, strikes(loro, body, shared, &reach[1], &wanted)?];
    // The newest edit of each side at all, for a side whose edits to a block cannot be told.
    let last = |span: loro::IdSpan| Stamp::of(loro, ID::new(span.peer, span.counter.end - 1));
    let newest = [0, 1].map(|side| reach[side].sub_iter(shared).filter_map(last).max().unwrap_or_default());

    let mut found = Vec::new();
    let views = ours.into_iter().zip(theirs);
    for (((old, chars), near), (ours, theirs)) in blocks.into_iter().zip(olds).zip(near).zip(views) {
        // A side that holds no such block and did not take it away joined it to the block before, where
        // its words now are. That block is another, and this one has no version on that side to show.
        let there = [ours.block.is_some(), theirs.block.is_some()];
        if !(there[0] || old.gone[0]) || !(there[1] || old.gone[1]) || there == [false, false] {
            continue;
        }
        let held = [ours, theirs];
        // What each side did to the block: the characters it put into it or between its old ones, and its
        // cuts of the old ones.
        let all = chars.iter().chain(held.iter().flat_map(|held| &held.ids));
        let touched: HashSet<&ID> = all.filter(|id| is_old(id)).collect();
        let made = [0, 1].map(|side| {
            let put = held[side].ids.iter().filter(|id| !is_old(id)).chain(&held[side].beside);
            let cut = touched.iter().filter_map(|id| struck[side].get(*id));
            // Each replica's newest edit holds its earlier ones.
            let mut tips: BTreeMap<u64, i32> = BTreeMap::new();
            for id in put.chain(cut) {
                let tip = tips.entry(id.peer).or_insert(id.counter);
                *tip = (*tip).max(id.counter);
            }
            tips.into_iter().map(|(peer, counter)| ID::new(peer, counter)).collect::<Vec<ID>>()
        });
        let stamps =
            [0, 1].map(|side| made[side].iter().filter_map(|id| Stamp::of(loro, *id)).max().unwrap_or(newest[side]));
        let later = usize::from(stamps[1] > stamps[0]);
        let [ours, theirs] = held;
        let side = |nth: usize, held: &Held| {
            let stamp = stamps[nth];
            // A side that a settling changed last reads as the version that settling made stand.
            let kept = settlings[nth].get(old.block.as_str()).filter(|(replica, ..)| *replica == stamp.replica);
            let (author, edited) = match kept {
                Some((_, author, edited)) => (author.clone(), *edited),
                None => {
                    (names[nth].get(&stamp.replica).cloned().unwrap_or_default(), stamp.time.try_into().unwrap_or(0))
                }
            };
            Side { block: held.block.clone(), author, edited }
        };
        let sides = [side(0, &ours), side(1, &theirs)];
        let conflict = Conflict { block: old.block, sides, later };
        let settled = [0, 1].iter().all(|side| settlers[*side].contains(&stamps[*side].replica));
        let old = !old.place.is_empty();
        let (keep, drop) = if later == 0 { (ours, theirs) } else { (theirs, ours) };
        let lost = keep.ids.iter().filter(|id| struck[1 - later].contains_key(*id)).copied().collect();
        let prior = keep.prior;
        let beside = drop.ahead.into_iter().chain(drop.beside).collect();
        let held = drop.ids;
        let drop = held.iter().copied().filter(|id| !is_old(id)).collect();
        let keep = (keep.pieces, keep.ids);
        found.push(Clash { conflict, settled, old, keep, lost, prior, held, drop, beside, near });
    }
    Ok(found)
}

/// Where each of `ids` stands in the sequence as it is shown. They are given in the order of the sequence,
/// so most are found by reading on from the one before. Finding one from nothing reads the whole of it.
fn places(loro: &LoroDoc, body: &LoroText, ids: &[ID]) -> HashMap<ID, usize> {
    /// How far to read on past the last one found.
    const FAR: usize = 64;
    let wanted: HashSet<ID> = ids.iter().copied().collect();
    let (mut found, len) = (HashMap::new(), body.len_unicode());
    for id in ids {
        if found.contains_key(id) {
            continue;
        }
        let Some(mut at) = crdt::place(loro, body, *id) else { continue };
        found.insert(*id, at);
        let mut misses = 0;
        while misses < FAR && found.len() < wanted.len() && at + 1 < len {
            at += 1;
            match crdt::id_at(body, at).filter(|id| wanted.contains(id)) {
                Some(id) => {
                    found.insert(id, at);
                    misses = 0;
                }
                None => misses += 1,
            }
        }
    }
    found
}

/// The conflicts worth telling of. Where the two sides read alike there is nothing to choose, and where
/// both are settlings there is nothing new: each device told of its conflict when it settled.
pub(crate) fn told(clashes: Vec<Clash>) -> Vec<Conflict> {
    let differs = |conflict: &Conflict| conflict.sides[0].block != conflict.sides[1].block;
    clashes.into_iter().filter(|clash| !clash.settled).map(|clash| clash.conflict).filter(differs).collect()
}

/// What a settling does to the sequence, by place: the characters to take out, and the text to put in
/// before a place, for which conflict. Text for an earlier conflict goes in first.
#[derive(Default)]
struct Fixes {
    cut: BTreeSet<usize>,
    put: BTreeMap<(usize, usize), Vec<TextDelta>>,
    /// The conflict that mends each character: the first to hold it, on either side. Two conflicts can
    /// hold one, when a side cut one of their blocks from the other. Mended by both, it would be put back
    /// twice, moved by one from where the other counts on it, or kept by one and taken out by the other.
    mender: HashMap<ID, usize>,
}

impl Clash {
    /// Adds what makes the block read as the side that was edited last left it, in the sequence as the
    /// history shows it. It is the `nth` of the conflicts, and `next` is where the block of the one after
    /// it starts. Gives where its own block starts.
    fn mend(
        &self,
        loro: &LoroDoc,
        body: &LoroText,
        nth: usize,
        next: Option<usize>,
        fixes: &mut Fixes,
    ) -> Option<usize> {
        let (pieces, ids) = &self.keep;
        let kept: Vec<ID> = ids.iter().copied().filter(|id| !self.lost.contains(id)).collect();
        let mut stand = places(loro, body, &kept);
        let mine = |id: &ID| fixes.mender.get(id) == Some(&nth);
        let theirs = places(loro, body, &self.held);
        let dropped = self.drop.iter().filter(|id| mine(id)).filter_map(|id| theirs.get(id));
        fixes.cut.extend(dropped);
        // The merge can leave some of the block on the wrong side of what the earlier side put in beside
        // it: behind a marker, in a block of that side's own, or ahead of text that ends the block before.
        // Those characters are moved: taken out there, and put back in with what was lost.
        let beside = places(loro, body, &self.beside);
        let marks = |at: &&usize| body.char_at(**at).is_ok_and(|c| MARKER.starts_with(c));
        // What stands between the block's characters and is not the block's is the earlier side's too. A
        // block the later side joined to this one can hold a marker of that side's.
        let reach = stand.values().min().copied().zip(stand.values().max().copied());
        let (mut within, mut among) = (None, Vec::new());
        if let Some((first, last)) = reach {
            let own: HashSet<usize> = stand.values().chain(theirs.values()).copied().collect();
            let mut at = first;
            while let Ok(c) = body.char_at(at) {
                let (marker, behind) = (MARKER.starts_with(c), at > last);
                if fixes.cut.contains(&at) || (!behind && own.contains(&at)) {
                    at += 1;
                    continue;
                }
                match marker {
                    true if !behind => within = within.or(Some(at)),
                    // A block that was not there when the sides last agreed has no place to tell by what
                    // the earlier side put in beside it. Its text among the block's own is that, and so is
                    // text right behind the block, where the later side left none: it is the end of the
                    // block before, as the earlier side has it.
                    false if !self.old && !own.contains(&at) && !(behind && theirs.is_empty()) => among.push(at),
                    _ if behind => break,
                    _ => {}
                }
                at += 1;
            }
        }
        let gate = beside.values().filter(marks).copied().chain(within).min();
        let ahead = beside.values().chain(&among).filter(|at| gate.is_none_or(|gate| **at < gate));
        let lead = ahead.max().map(|at| at + 1);
        let astray =
            |id: &ID, at: usize| mine(id) && (lead.is_some_and(|lead| at < lead) || gate.is_some_and(|gate| at > gate));
        fixes.cut.extend(stand.iter().filter(|(id, at)| astray(id, **at)).map(|(_, at)| *at));
        stand.retain(|id, at| !astray(id, *at));
        // The characters of the block one by one, each with the piece that carries its attributes.
        let spelled = pieces.iter().enumerate().flat_map(|(piece, item)| match item {
            TextDelta::Insert { insert, .. } => insert.chars().map(|c| (piece, c)).collect(),
            _ => Vec::new(),
        });
        let mut put = |at: usize, run: &mut Vec<(usize, char)>| {
            let text = fixes.put.entry((at, nth)).or_default();
            for chars in mem::take(run).chunk_by(|(a, _), (b, _)| a == b) {
                let attributes = match &pieces[chars[0].0] {
                    TextDelta::Insert { attributes, .. } => attributes.clone(),
                    _ => None,
                };
                text.push(TextDelta::Insert { insert: chars.iter().map(|(_, c)| *c).collect(), attributes });
            }
        };
        // A run of lost characters goes back in after the last one before it that is still there.
        let (mut run, mut after, mut start) = (Vec::new(), None, None);
        for (id, spelled) in ids.iter().zip(spelled) {
            match stand.get(id) {
                Some(at) => {
                    if !run.is_empty() {
                        put(after.unwrap_or(*at), &mut run);
                    }
                    start.get_or_insert(after.unwrap_or(*at));
                    after = Some(at + 1);
                }
                None if fixes.mender.get(id) == Some(&nth) => run.push(spelled),
                None => {}
            }
        }
        if !run.is_empty() {
            // With nothing of the block left to go by, it goes between what the earlier side put in beside
            // it, and behind what stands before it on the later side. Or else before the nearest block
            // after it: one that still stands, or one put back.
            let prior = || self.prior.and_then(|id| crdt::place(loro, body, id)).map(|at| at + 1);
            let behind = || lead.max(prior().filter(|at| gate.is_none_or(|gate| *at <= gate)));
            let near = || self.near.iter().find_map(|id| crdt::place(loro, body, *id));
            let after = after.or_else(behind).or(gate).or_else(|| [near(), next].into_iter().flatten().min());
            let at = after.unwrap_or_else(|| body.len_unicode());
            put(at, &mut run);
            start.get_or_insert(at);
        }
        start
    }
}

/// Makes the block of each conflict between two revisions read as the side that was edited last left it,
/// by one change that every device writes alike. False when the history already holds the change, or
/// there is nothing to do.
///
/// It shows the history where the two revisions have just met, and leaves it there.
pub(crate) fn settle(loro: &LoroDoc, body: &LoroText, sides: [&Frontiers; 2], clashes: &[Clash]) -> bool {
    let met = Frontiers::from(sides[0].iter().chain(sides[1].iter()).collect::<Vec<ID>>());
    let met = loro.minimize_frontiers(&met).unwrap_or(met);
    if clashes.is_empty() || loro.checkout(&met).is_err() {
        return false;
    }
    // One change for all of them, read off one sequence: blocks that stood side by side are put back
    // side by side.
    let (mut fixes, mut next) = (Fixes::default(), None);
    for (nth, clash) in clashes.iter().enumerate() {
        for id in clash.keep.1.iter().chain(&clash.drop) {
            fixes.mender.entry(*id).or_insert(nth);
        }
    }
    for (nth, clash) in clashes.iter().enumerate().rev() {
        let start = clash.mend(loro, body, nth, next, &mut fixes);
        next = start.filter(|_| clash.old).or(next);
    }
    let mut put: BTreeMap<usize, Vec<TextDelta>> = BTreeMap::new();
    for ((at, _), text) in fixes.put {
        put.entry(at).or_default().extend(text);
    }
    let places: BTreeSet<usize> = fixes.cut.iter().chain(put.keys()).copied().collect();
    let (mut delta, mut at) = (Vec::new(), 0);
    for place in places {
        if place > at {
            delta.push(TextDelta::Retain { retain: place - at, attributes: None });
            at = place;
        }
        delta.extend(put.remove(&place).unwrap_or_default());
        if fixes.cut.contains(&place) {
            delta.push(TextDelta::Delete { delete: 1 });
            at += 1;
        }
    }
    if delta.is_empty() {
        return false;
    }
    let replica = crdt::outsider(&Revision(met).to_string(), &delta, "settled");
    if loro.oplog_vv().get(&replica).is_some_and(|held| *held > 0) || loro.set_peer_id(replica).is_err() {
        return false;
    }
    // A part Loro refused leaves the rest written, here as on every device.
    let _ = crdt::write_as_told(loro, body, &delta);
    for Conflict { block, sides, later } in clashes.iter().map(|clash| &clash.conflict) {
        crdt::settled(loro, block, &sides[*later].author, sides[*later].edited);
    }
    loro.commit();
    true
}

#[cfg(test)]
mod tests {
    use super::rfc3339;

    #[test]
    fn times_are_written_as_dates() {
        let cases = [
            (0, "1970-01-01T00:00:00Z"),
            (951_782_399, "2000-02-28T23:59:59Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (1_800_000_000, "2027-01-15T08:00:00Z"),
            (4_107_542_399, "2100-02-28T23:59:59Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"),
            (u64::MAX, "9999-12-31T23:59:59Z"),
        ];
        for (seconds, date) in cases {
            assert_eq!(rfc3339(seconds), date);
        }
    }
}
