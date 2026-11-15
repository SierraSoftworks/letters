//! The names a document's history is addressed by: who wrote, which point in time, and where in the text.

use std::fmt;
use std::str::FromStr;

use std::cmp::Ordering;

use loro::{Frontiers, ID, LoroDoc, VersionVector};
use sha2::{Digest, Sha256};

/// One writer of a document.
///
/// Every editing session takes a fresh id from the shell and never stores it in the file. Two writers must
/// never share one: their edits would be taken for the same edits, and the copies could no longer merge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReplicaId(pub(crate) u64);

impl ReplicaId {
    /// From an identifier that is unique across devices, as the shell's `new_id` gives.
    pub fn new(id: &str) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"letters replica");
        hash.update(id.as_bytes());
        Self(crate::crdt::peer(hash))
    }
}

impl From<u64> for ReplicaId {
    /// For a shell that numbers its own sessions, and for tests.
    fn from(id: u64) -> Self {
        // Loro keeps the largest number for itself.
        Self(id.min(u64::MAX - 1))
    }
}

/// A point in a document's history. As text it is short enough to keep in a file, as a version marker is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision(pub(crate) Frontiers);

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut ids = self.0.to_vec();
        ids.sort();
        let ids: Vec<String> = ids.iter().map(|id| format!("{}@{}", id.counter, id.peer)).collect();
        f.write_str(&ids.join(","))
    }
}

impl FromStr for Revision {
    type Err = HistoryError;

    fn from_str(text: &str) -> Result<Self, HistoryError> {
        let ids = text.split(',').filter(|id| !id.is_empty()).map(read_id).collect::<Option<Vec<ID>>>();
        ids.map(|ids| Self(Frontiers::from(ids))).ok_or_else(|| HistoryError::Unreadable("not a revision".into()))
    }
}

pub(crate) fn read_id(text: &str) -> Option<ID> {
    let (counter, peer) = text.split_once('@')?;
    let counter: i32 = counter.parse().ok()?;
    (counter >= 0).then_some(ID::new(peer.parse().ok()?, counter))
}

/// How far a history has read each of its writers. Two replicas trade these, and each then sends the other
/// only what it lacks: see [`Model::extent`](crate::Model::extent).
///
/// Extents order as the histories they tell of do: `Greater` holds everything the other does and more,
/// and two that each hold what the other lacks do not order. As bytes one grows with the number of
/// sessions that ever wrote, since each session writes under an id of its own.
#[derive(Clone, Debug, Default)]
pub struct Extent(pub(crate) VersionVector);

impl Extent {
    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.encode()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, HistoryError> {
        let read = VersionVector::decode(bytes).map_err(|_| HistoryError::Unreadable("not an extent".into()))?;
        Ok(Self(read))
    }

    /// How far the edits in a saved history or an update reach, for each writer with edits in it.
    pub fn of(history: &[u8]) -> Result<Self, HistoryError> {
        let read = LoroDoc::decode_import_blob_meta(history, true);
        read.map(|meta| Self(meta.partial_end_vv)).map_err(|error| HistoryError::Unreadable(error.to_string()))
    }

    /// Takes in what `other` tells of, as a history does that merges the other.
    pub fn merge(&mut self, other: &Extent) {
        self.0.merge(&other.0);
    }

    /// Whether the two tell of an edit in common, as copies that began as one document do.
    pub fn shares(&self, other: &Extent) -> bool {
        self.0.iter().any(|(writer, end)| *end > 0 && other.0.get(writer).is_some_and(|held| *held > 0))
    }

    /// Whether it tells of no edit at all, as a copy does that is yet to be filled.
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|(_, end)| *end <= 0)
    }
}

impl PartialEq for Extent {
    fn eq(&self, other: &Self) -> bool {
        self.partial_cmp(other) == Some(Ordering::Equal)
    }
}

impl PartialOrd for Extent {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

/// Why a history was refused. The model is left as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryError {
    /// The bytes are not a history, or are damaged.
    Unreadable(String),
    /// The history does not reach back, or forward, to this revision.
    UnknownRevision,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "the history cannot be read: {why}"),
            Self::UnknownRevision => f.write_str("the history does not hold this revision"),
        }
    }
}

impl std::error::Error for HistoryError {}

/// Which neighbour a [`StablePos`] holds on to, and so where text typed exactly there lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hold {
    /// The character after it, as the start of a passage does. Text typed there lands before it.
    Next,
    /// The character before it, as the end of a passage does. Text typed there lands after it.
    Prev,
}

/// A position that stays with its text through edits and merges, where a [`Pos`](crate::Pos) would drift.
///
/// As text it is short enough to keep in a file, as a note's anchor is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StablePos {
    /// The character held on to. `None` is the end of the document.
    pub(crate) id: Option<ID>,
    /// Whether the position sits after that character.
    pub(crate) after: bool,
}

impl fmt::Display for StablePos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.id {
            None => f.write_str("end"),
            Some(id) => write!(f, "{}@{}{}", id.counter, id.peer, if self.after { "+" } else { "" }),
        }
    }
}

impl FromStr for StablePos {
    type Err = HistoryError;

    fn from_str(text: &str) -> Result<Self, HistoryError> {
        if text == "end" {
            return Ok(Self { id: None, after: false });
        }
        let (id, after) = text.strip_suffix('+').map_or((text, false), |id| (id, true));
        let id = read_id(id).ok_or_else(|| HistoryError::Unreadable("not a stable position".into()))?;
        Ok(Self { id: Some(id), after })
    }
}
