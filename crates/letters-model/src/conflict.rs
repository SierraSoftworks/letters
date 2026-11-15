//! Blocks that two writers changed in the same words while apart.
//!
//! Changes that do not touch merge by themselves, even inside one paragraph. Where both sides changed one
//! word, the merge keeps what each of them typed, and the block reads as neither meant it. The same holds
//! for text typed into a block that the other side took away. [`Model::conflicts`](crate::Model::conflicts)
//! finds those blocks, with both versions for a person to choose from.

use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::ops::Range;
use std::sync::{Mutex, PoisonError};

use loro::{Frontiers, ID, LoroDoc, LoroText, TextDelta, VersionVector};
use unicode_segmentation::UnicodeSegmentation;

use crate::crdt::{self, Event};
use crate::flat::{self, Entry, MARKER};
use crate::notes::Version;
use crate::{Block, BlockId, BodyShape, Document, Extra, HistoryError};

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
    /// Who last changed it on this side. Empty when that writer gave no name.
    pub author: String,
    /// When, in seconds since 1970 and right to the minute. A writer who never told the time is taken to
    /// have written when the newest edit it built on was made, and at 0 when none of those told either.
    pub edited: u64,
}

impl Conflict {
    /// The two versions as a conflict note keeps them. `None` when a side took the block away.
    pub fn versions(&self) -> Option<[Version; 2]> {
        let version = |side: &Side| {
            let (author, block) = (side.author.clone(), side.block.clone()?);
            Some(Version { author, edited: rfc3339(side.edited), block, extra: Extra::new() })
        };
        Some([version(&self.sides[0])?, version(&self.sides[1])?])
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

/// What one side did to the sequence since the two last agreed, in the places of that time.
#[derive(Default)]
struct Edits {
    /// The characters it took out, in order.
    cut: Vec<Range<usize>>,
    /// The places where it put text in that runs on from the text before, in order.
    put: Vec<usize>,
}

impl Edits {
    /// Reads them from the change as Loro tells it. A change of formatting alone is none: it merges.
    fn of(raw: &[TextDelta]) -> Self {
        let (mut edits, mut at) = (Self::default(), 0);
        for item in raw {
            match item {
                TextDelta::Retain { retain, .. } => at += retain,
                TextDelta::Delete { delete } => {
                    edits.cut.push(at..at + delete);
                    at += delete;
                }
                // Text that opens with a marker went into a block of its own, and none of it in here.
                TextDelta::Insert { insert, .. } if insert.is_empty() || insert.starts_with(MARKER) => {}
                TextDelta::Insert { .. } => edits.put.push(at),
            }
        }
        edits
    }
}

/// What one side did to one word: the characters it cut from it, and whether it put text in or beside it.
type Touch = (BTreeSet<usize>, bool);

/// What one side did to the text of one block.
#[derive(Default)]
struct Done {
    /// The block left its place: its marker and all of its text were taken out.
    gone: bool,
    changed: bool,
    /// By the word's place among the words of the block.
    words: BTreeMap<usize, Touch>,
}

impl Done {
    /// `text` is where the block's text starts in the sequence, and `words` what [`words`] makes of it.
    fn of(edits: &Edits, text: usize, entry: &Entry, words: &[(Range<usize>, bool)]) -> Self {
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
        let first = edits.put.partition_point(|put| *put < text);
        for put in edits.put[first..].iter().take_while(|put| **put <= end) {
            done.changed = true;
            let at = put - text;
            let next = words.partition_point(|(word, _)| word.end <= at);
            let inside = words.get(next).is_some_and(|(word, _)| word.start < at);
            // Text put between two words runs into each one that is more than space or punctuation.
            let before = next.checked_sub(1).filter(|_| !inside);
            for nth in [before, Some(next)].into_iter().flatten() {
                if words.get(nth).is_some_and(|(_, wordlike)| inside || *wordlike) {
                    done.words.entry(nth).or_default().1 = true;
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
fn words(text: &str) -> Vec<(Range<usize>, bool)> {
    let mut at = 0;
    let word = |word: &str| {
        let start = at;
        at += word.chars().count();
        (start..at, word.chars().any(char::is_alphanumeric))
    };
    text.split_word_bounds().map(word).collect()
}

/// The blocks of `old` in which the two sides' edits cannot both stand, each with whether it left its
/// place on either side.
fn clashes(old: &Document, index: &[Entry], edits: [&Edits; 2]) -> Vec<(BlockId, [bool; 2])> {
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
        blocks.extend(edits.put.iter().filter_map(|put| texts.partition_point(|text| text <= put).checked_sub(1)));
        blocks
    };
    let (ours, theirs) = (touched(edits[0]), touched(edits[1]));
    let order = flat::order(&old.blocks, index);
    let mut found = Vec::new();
    for nth in ours.intersection(&theirs).copied().filter(|nth| index[*nth].shape == BodyShape::Text) {
        let Some((block, entry)) = order.get(nth) else { continue };
        let words = words(&block.plain_text());
        let done = edits.map(|edits| Done::of(edits, texts[nth], entry, &words));
        if done[0].clashes(&done[1]) {
            found.push((block.id.clone(), [done[0].gone, done[1].gone]));
        }
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
        stretches.extend(edits.put.iter().map(|put| holds(put.saturating_sub(1))));
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

/// What the history holds of each block at the revision it shows, with the newest edit made to the block
/// since `shared` and who made it. A block whose edit left no text to ask is given the newest edit of
/// that side at all.
fn views(
    loro: &LoroDoc,
    body: &LoroText,
    blocks: &[(BlockId, [bool; 2])],
    shared: &VersionVector,
) -> Vec<(Side, Stamp)> {
    let (doc, index) = crdt::read(loro, body);
    let order = flat::order(&doc.blocks, &index);
    let last = |span: loro::IdSpan| Stamp::of(loro, ID::new(span.peer, span.counter.end - 1));
    let side = loro.state_vv().sub_iter(shared).filter_map(last).max().unwrap_or_default();
    let view = |id: &BlockId| {
        let Some(nth) = order.iter().position(|(block, _)| block.id == *id) else { return (None, side) };
        let start: usize = index[..nth].iter().map(Entry::len).sum();
        let new = |at: usize| crdt::id_at(body, at).filter(|id| !shared.includes_id(*id));
        let newest = (start..start + index[nth].len()).filter_map(new).filter_map(|id| Stamp::of(loro, id)).max();
        (Some(order[nth].0.clone()), newest.unwrap_or(side))
    };
    let told = |(id, _): &(BlockId, [bool; 2])| {
        let (block, stamp) = view(id);
        let (author, edited) = (crdt::author_of(loro, stamp.replica), u64::try_from(stamp.time).unwrap_or(0));
        (Side { block, author, edited }, stamp)
    };
    blocks.iter().map(told).collect()
}

/// Finds the conflicts between two revisions. It reads the history at other revisions, and leaves it
/// showing one of them: the caller brings back the one its document shows.
pub(crate) fn find(
    loro: &LoroDoc,
    body: &LoroText,
    events: &Mutex<Vec<Event>>,
    sides: [&Frontiers; 2],
) -> Result<Vec<Conflict>, HistoryError> {
    let held = |at: &Frontiers| loro.frontiers_to_vv(at).ok_or(HistoryError::UnknownRevision);
    let (ours, theirs) = (held(sides[0])?, held(sides[1])?);
    // A side that holds all of the other changed nothing apart from it.
    if ours.partial_cmp(&theirs).is_some() {
        return Ok(Vec::new());
    }
    let shared = ours.intersection(&theirs);
    let base = loro.vv_to_frontiers(&shared);
    // Shows a revision, and gives the change to the sequence that led there.
    let show = |at: &Frontiers| {
        let queue = || events.lock().unwrap_or_else(PoisonError::into_inner);
        queue().clear();
        loro.checkout(at).map_err(|_| HistoryError::UnknownRevision)?;
        let changes = mem::take(&mut *queue()).into_iter();
        let mut bodies = changes.filter_map(|event| if let Event::Body(delta) = event { Some(delta) } else { None });
        let delta = bodies.next().unwrap_or_default();
        debug_assert!(bodies.next().is_none(), "one step through the history told of the sequence twice");
        Ok::<_, HistoryError>(delta)
    };
    show(&base)?;
    let sequence = body.to_string();
    let first = Edits::of(&show(sides[0])?);
    show(&base)?;
    let second = Edits::of(&show(sides[1])?);
    // Most copies that part change different blocks, and are done here.
    if !meet(&sequence, [&first, &second]) {
        return Ok(Vec::new());
    }
    show(&base)?;
    let (old, index) = crdt::read(loro, body);
    let blocks = clashes(&old, &index, [&first, &second]);
    if blocks.is_empty() {
        return Ok(Vec::new());
    }
    show(sides[1])?;
    let theirs = views(loro, body, &blocks, &shared);
    show(sides[0])?;
    let ours = views(loro, body, &blocks, &shared);
    let mut found = Vec::new();
    for (((block, gone), (ours, first)), (theirs, second)) in blocks.into_iter().zip(ours).zip(theirs) {
        // A side that holds no such block and did not take it away joined it to the block before, where
        // its words now are. That block is another, and this one has no version on that side to show.
        if (ours.block.is_some() || gone[0]) && (theirs.block.is_some() || gone[1]) {
            found.push(Conflict { block, later: usize::from(second > first), sides: [ours, theirs] });
        }
    }
    Ok(found)
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
