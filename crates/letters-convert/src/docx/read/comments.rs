//! Word's comments as notes, each on the passage Word showed it beside.

use std::collections::{BTreeMap, BTreeSet};

use letters_model::{Anchor, BlockId, Document, Note, NoteKind, Reply, Run};
use roxmltree::Node;

use super::{MAX_DEPTH, Place, Reader, Spot, flatten, merge};
use crate::docx::package::{Element, Rels};

impl Reader<'_, '_> {
    /// Builds the comments, each on the passage Word showed it beside.
    pub(super) fn comments(
        &mut self,
        root: Node,
        threads: Option<Node>,
        rels: &Rels,
        document: &Document,
        now: &str,
    ) -> Vec<Note> {
        struct Read {
            id: String,
            author: String,
            created: String,
            content: Vec<Run>,
            /// The id Word gives the comment's last paragraph, which is how a thread names it.
            thread: Option<String>,
        }

        self.detached = true;
        let mut read = Vec::new();
        for comment in root.all("comment") {
            let mut content = Vec::new();
            flatten(self.blocks(comment, rels), &mut content, &mut BTreeMap::new(), &mut Vec::new());
            merge(&mut content);
            let last = comment.descendants().rfind(|node| node.is("p"));
            read.push(Read {
                id: comment.attr("id").unwrap_or_default().to_owned(),
                author: comment
                    .attr("author")
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or("Unknown")
                    .into(),
                created: created(comment.attr("date"), now),
                content,
                thread: last.and_then(|last| last.attr("paraId")).map(str::to_owned),
            });
        }
        self.detached = false;

        // A reply names the comment it answers, and a thread that is done says so on its first comment.
        let mut parents: BTreeMap<&str, &str> = BTreeMap::new();
        let mut done = BTreeSet::new();
        for thread in
            threads.iter().flat_map(|root| root.children()).filter(|node| node.tag_name().name() == "commentEx")
        {
            let Some(id) = thread.attr("paraId") else { continue };
            if let Some(parent) = thread.attr("paraIdParent") {
                parents.insert(id, parent);
            }
            if matches!(thread.attr("done"), Some("1" | "true")) {
                done.insert(id);
            }
        }
        let root_of = |thread: &str| {
            let mut at = thread;
            for _ in 0..MAX_DEPTH {
                match parents.get(at) {
                    Some(parent) => at = parent,
                    None => break,
                }
            }
            at.to_owned()
        };

        let known: BTreeSet<&BlockId> = document.walk().map(|block| &block.id).collect();
        let mut notes: Vec<Note> = Vec::new();
        let mut roots: BTreeMap<String, usize> = BTreeMap::new();
        for comment in &read {
            let thread = comment.thread.as_deref();
            if let Some(parent) = thread.filter(|id| parents.contains_key(id)).and_then(|id| roots.get(&root_of(id))) {
                notes[*parent].replies.push(Reply {
                    author: comment.author.clone(),
                    created: comment.created.clone(),
                    content: comment.content.clone(),
                    extra: Default::default(),
                });
                continue;
            }
            let anchor = self.anchor(&comment.id).filter(|anchor| known.contains(&anchor.block));
            // A passage that is all of its block is the block, as is one in words the block keeps beside its
            // text, such as a picture's caption.
            let anchor = anchor.map(|anchor| {
                let len = document.block(&anchor.block).map_or(0, |block| block.plain_text().len());
                match &anchor.range {
                    Some(range) if *range == (0..len) || range.end > len => Anchor::block(anchor.block),
                    _ => anchor,
                }
            });
            let Some(anchor) = anchor.or_else(|| document.blocks.first().map(|first| Anchor::block(first.id.clone())))
            else {
                continue;
            };
            if let Some(thread) = thread {
                roots.insert(thread.to_owned(), notes.len());
            }
            notes.push(Note {
                id: (self.new_id)(),
                kind: NoteKind::Comment,
                anchor,
                author: comment.author.clone(),
                created: comment.created.clone(),
                resolved: thread.is_some_and(|id| done.contains(id)),
                content: comment.content.clone(),
                replies: Vec::new(),
                extra: Default::default(),
            });
        }
        notes
    }

    /// The passage a comment sits on, from where Word marked its start and end.
    fn anchor(&self, comment: &str) -> Option<Anchor> {
        let (start, end) = self.spans.get(comment)?;
        let (start, end) = (start.or(*end)?, end.or(*start)?);
        let (first, from) = self.locate(start)?;
        let (last, to) = self.locate(end)?;
        // A passage that runs on into later blocks is marked as far as the end of its first.
        let to = if first.block == last.block { to } else { first.base + first.len };
        Some(if from < to { Anchor::range(first.block.clone(), from..to) } else { Anchor::block(first.block.clone()) })
    }

    /// The block a spot ended up in, and the bytes into its text.
    pub(super) fn locate(&self, spot: Spot) -> Option<(&Place, usize)> {
        if let Some(place) = self.placed.get(&spot.para) {
            return Some((place, place.base + spot.at.saturating_sub(place.cut).min(place.len)));
        }
        // The paragraph left no block, so the spot moves to the nearest text.
        if let Some((_, place)) = self.placed.range(spot.para..).next() {
            return Some((place, place.base));
        }
        self.placed.range(..spot.para).next_back().map(|(_, place)| (place, place.base + place.len))
    }
}

/// The time a comment was made. Word writes local time, and names no zone or wrongly calls it UTC.
fn created(date: Option<&str>, now: &str) -> String {
    let Some(date) = date.map(str::trim) else { return now.into() };
    let bytes = date.as_bytes();
    let shaped = bytes.len() >= 19
        && bytes.iter().take(19).enumerate().all(|(at, byte)| match at {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            _ => byte.is_ascii_digit(),
        });
    if !shaped {
        return now.into();
    }
    let zoned = date.ends_with('Z') || date[19..].contains(['+', '-']);
    if zoned { date.into() } else { format!("{date}Z") }
}
