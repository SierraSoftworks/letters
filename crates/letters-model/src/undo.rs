//! This session's undo steps.
//!
//! A step is kept as the changes that take one edit back. When another replica's edit arrives, every
//! change to the sequence is rewritten to act on the same text as before. Undoing is then one more edit
//! and costs what an edit costs: the history is never replayed, however long it is.

use std::collections::VecDeque;

use loro::TextDelta;

use crate::crdt::Assign;

/// A change to the sequence, counted in characters.
pub(crate) type Delta = Vec<TextDelta>;

/// One of the changes that take an edit back.
#[derive(Clone, Debug)]
pub(crate) enum Edit {
    Text(Delta),
    /// Values put back into the document's own fields or its style.
    Set(Vec<Assign>),
}

impl Edit {
    fn is_empty(&self) -> bool {
        match self {
            Self::Text(delta) => delta.is_empty(),
            Self::Set(assigns) => assigns.is_empty(),
        }
    }
}

/// The changes that take one edit back, in the order to make them.
pub(crate) type Step = Vec<Edit>;

/// Steps kept. The oldest is forgotten first.
const MAX_STEPS: usize = 1000;

#[derive(Default)]
pub(crate) struct Steps {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
}

impl Steps {
    /// Notes how to take back an edit just made. With `join`, it is taken back together with the last.
    pub fn record(&mut self, mut step: Step, join: bool) {
        step.retain(|edit| !edit.is_empty());
        if step.is_empty() {
            return;
        }
        self.redo.clear();
        match self.undo.back_mut().filter(|_| join) {
            // The newest edit is the first to take back.
            Some(last) => {
                step.append(last);
                *last = step;
            }
            None => self.undo.push_back(step),
        }
        if self.undo.len() > MAX_STEPS {
            self.undo.pop_front();
        }
    }

    /// The step to make next, backwards or forwards.
    pub fn take(&mut self, back: bool) -> Option<Step> {
        if back { self.undo.pop_back() } else { self.redo.pop() }
    }

    /// Notes how to reverse a step just made, which was taken with the same `back`.
    pub fn stepped(&mut self, reverse: Step, back: bool) {
        if back {
            self.redo.push(reverse);
        } else {
            self.undo.push_back(reverse);
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Rewrites every step to follow a change to the sequence that did not come from this session. A step
    /// that the change leaves nothing of keeps its place, so whoever counts its edits can still step
    /// back as many.
    pub fn rebase(&mut self, theirs: &[TextDelta]) {
        fn stack<'a>(steps: impl DoubleEndedIterator<Item = &'a mut Step>, theirs: &[TextDelta]) {
            let mut theirs = theirs.to_vec();
            // The newest step acts on the sequence as it stands, and each older one on what that leaves.
            for step in steps.rev() {
                if theirs.is_empty() {
                    break;
                }
                for edit in step.iter_mut() {
                    let Edit::Text(ours) = edit else { continue };
                    let moved = rebase(ours, &theirs, true);
                    theirs = rebase(&theirs, ours, false);
                    *ours = moved;
                }
                step.retain(|edit| !edit.is_empty());
            }
        }
        stack(self.undo.iter_mut(), theirs);
        stack(self.redo.iter_mut(), theirs);
    }
}

/// Whether an item passes over characters without changing them.
pub(crate) fn is_skip(item: &TextDelta) -> bool {
    matches!(item, TextDelta::Retain { attributes, .. } if attributes.as_ref().is_none_or(|attrs| attrs.is_empty()))
}

/// Writes a change item by item, in as few items as say the same.
#[derive(Default)]
pub(crate) struct Script(Delta);

impl Script {
    pub fn push(&mut self, mut item: TextDelta) {
        if let TextDelta::Retain { attributes, .. } = &mut item
            && attributes.as_ref().is_some_and(|attrs| attrs.is_empty())
        {
            *attributes = None;
        }
        match (self.0.last_mut(), &item) {
            (_, TextDelta::Retain { retain: 0, .. } | TextDelta::Delete { delete: 0 }) => {}
            (_, TextDelta::Insert { insert, .. }) if insert.is_empty() => {}
            (Some(TextDelta::Delete { delete }), TextDelta::Delete { delete: more }) => *delete += more,
            (Some(TextDelta::Retain { retain, attributes }), TextDelta::Retain { retain: more, attributes: other })
                if attributes == other =>
            {
                *retain += more;
            }
            _ => self.0.push(item),
        }
    }

    pub fn retain(&mut self, retain: usize) {
        self.push(TextDelta::Retain { retain, attributes: None });
    }

    pub fn delete(&mut self, delete: usize) {
        self.push(TextDelta::Delete { delete });
    }

    /// The change, without the characters it passes over at its end.
    pub fn finish(mut self) -> Delta {
        while self.0.last().is_some_and(is_skip) {
            self.0.pop();
        }
        self.0
    }
}

/// Reads a change item by item, part of an item at a time.
struct Items<'a> {
    items: &'a [TextDelta],
    /// Characters of the first item already read.
    used: usize,
}

impl<'a> Items<'a> {
    fn peek(&self) -> Option<&'a TextDelta> {
        self.items.first()
    }

    /// Characters of the sequence the next item still covers. A change passes over everything after its end.
    fn left(&self) -> usize {
        match self.peek() {
            Some(TextDelta::Retain { retain: len, .. } | TextDelta::Delete { delete: len }) => len - self.used,
            Some(TextDelta::Insert { .. }) => 0,
            None => usize::MAX,
        }
    }

    fn skip(&mut self, chars: usize) {
        if chars < self.left() {
            self.used += chars;
        } else if !self.items.is_empty() {
            (self.items, self.used) = (&self.items[1..], 0);
        }
    }
}

/// `ours` rewritten to follow `theirs`, when both were made for the same sequence.
///
/// `later` tells the two apart where they meet: the later one's text goes after text put in at the same
/// place, and its formatting stands where both set the same attribute. Rebasing each on the other, one of
/// them as the later, gives two orders of the same two changes that end in the same sequence.
fn rebase(ours: &[TextDelta], theirs: &[TextDelta], later: bool) -> Delta {
    let (mut a, mut b) = (Items { items: ours, used: 0 }, Items { items: theirs, used: 0 });
    let mut out = Script::default();
    loop {
        match (a.peek(), b.peek()) {
            (Some(insert @ TextDelta::Insert { .. }), theirs)
                if !(later && matches!(theirs, Some(TextDelta::Insert { .. }))) =>
            {
                out.push(insert.clone());
                a.skip(0);
            }
            (_, Some(TextDelta::Insert { insert, .. })) => {
                out.retain(insert.chars().count());
                b.skip(0);
            }
            (None, _) => break,
            (Some(ours), theirs) => {
                let chars = a.left().min(b.left());
                match (ours, theirs) {
                    // What they deleted is no longer there to change.
                    (_, Some(TextDelta::Delete { .. })) => {}
                    (TextDelta::Retain { attributes, .. }, theirs) => {
                        let mut attributes = attributes.clone();
                        if let (false, Some(attrs), Some(TextDelta::Retain { attributes: Some(set), .. })) =
                            (later, &mut attributes, theirs)
                        {
                            attrs.retain(|key, _| !set.contains_key(key));
                        }
                        out.push(TextDelta::Retain { retain: chars, attributes });
                    }
                    _ => out.delete(chars),
                }
                a.skip(chars);
                b.skip(chars);
            }
        }
    }
    out.finish()
}
