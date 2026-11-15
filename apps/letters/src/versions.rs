//! When a window marks a version in its document's history.
//!
//! The history lists a marked step as a version of its own, and keeps it however it is thinned. The model
//! names the kinds of mark, and a shell knows when they happen: a session's writing ends, writing rests
//! for long, an import lands, two copies meet in a save.
//!
//! Marks are written late on purpose. A mark is part of the history, so one made as a document opens
//! would change a file that was only read. Where the last session ended, and where writing rested, is
//! marked when the next edit comes: an edit is saved anyway.

use std::time::{Duration, Instant};

use letters_model::{Revision, TagKind};

/// How long writing rests before the place it stopped is marked, and how far apart merges are marked.
/// The history view starts a new version after a rest of the same length.
pub const PAUSE: Duration = Duration::from_secs(600);

/// Where a window's document rested, as far as marking versions goes.
#[derive(Debug, Default)]
pub struct Rests {
    /// The revision this session found, until the session writes anything.
    found: Option<Revision>,
    /// The revision the last edit led to, and when it was made.
    last: Option<(Revision, Instant)>,
    /// When a merge was last marked.
    merged: Option<Instant>,
}

impl Rests {
    /// The document was read from its file, where it stood at `found`.
    pub fn opened(&mut self, found: Revision) {
        self.found = Some(found);
    }

    /// An edit was made at `now`. Returns the revision before it and why it is worth a mark: the last
    /// session ended there, or writing rested there for [`PAUSE`] or longer.
    pub fn edited(&mut self, now: Instant) -> Option<(Revision, TagKind)> {
        let found = self.found.take().map(|found| (found, TagKind::Session));
        let rested = |(_, at): &(Revision, Instant)| now.saturating_duration_since(*at) >= PAUSE;
        let paused = self.last.take_if(|last| rested(last)).map(|(last, _)| (last, TagKind::Pause));
        found.or(paused)
    }

    /// The document reads as `at` once the edit at `now`, and any mark made for it, is in.
    pub fn rest(&mut self, at: Revision, now: Instant) {
        self.last = Some((at, now));
    }

    /// A save at `now` met another writer's edits and took them in. Returns whether that is worth a
    /// mark: copies kept in step through a shared folder meet every few seconds, and the history is not
    /// to list each time.
    pub fn merged(&mut self, now: Instant) -> bool {
        let due = self.merged.is_none_or(|last| now.saturating_duration_since(last) >= PAUSE);
        if due {
            self.merged = Some(now);
        }
        due
    }
}
