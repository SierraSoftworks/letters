//! Notes on a passage: what people say about it, kept beside the page.
//!
//! Over a stretch of selected text float the two things that are done to a passage: linking it and
//! commenting on it. A comment is written in a field that hangs under its passage ([`crate::comment`]),
//! and is kept as a note that stays with its text. How notes show once they are kept is in
//! [`crate::threads`].

use std::collections::HashMap;

use accesskit::Role;
use letters_model::{Anchor, Block, BlockId, Document, Mark, Model, Note, NoteKind, Notes, Pos, Reply, Run, Selection};
use letters_render::{DisplayList, Item, Rect};

use crate::color::{fade, resolve};
use crate::comment::{Writing, Written};
use crate::element::Element;
use crate::icon;
use crate::style::{Align, Edges};
use crate::text::TextStyle;
use crate::threads::{Kind, Marker};
use crate::tokens::{self, Scheme};
use crate::widgets::{Button, Popover, Side};
use crate::window::Message;

/// How wide the field a comment is written in hangs under its passage, with the button that sends it.
pub(crate) const WIDTH: f32 = 320.0;
/// The room between a selection and the tools over it.
pub(crate) const TOOLS_GAP: f32 = 8.0;
/// The room the tools take above a selection, which they hang under when it is not there.
pub(crate) const TOOLS_ROOM: f32 = tokens::CONTROL_HEIGHT_SM + 2.0 * tokens::SPACE_XS + 2.0 + TOOLS_GAP;
/// How strong the tint of a passage is while its comment is being written or read.
const OPEN: f32 = 0.26;
/// What someone who only reads the document is told where a comment would be written, in the words the
/// server refuses one with.
const READS: &str = "You can read this document, but not comment on it.";

/// What the pointer and the keyboard ask about notes.
#[derive(Clone, Debug, PartialEq)]
pub enum Noting {
    /// Puts the tools over the selection away, until the selection changes.
    Away,
    /// Starts a comment on the selected passage.
    Write,
    /// What the keyboard or the pointer does in the field of the comment or the reply being written.
    Wrote(Writing),
    /// Keeps the comment being written.
    Send,
    /// Drops it.
    Drop,
    /// Opens a note in place, under the passage it is about.
    Open(String),
    /// A press on the marker beside a block: opens the first of its notes, or the one after the note
    /// that is open, and puts the last one away.
    Press(BlockId),
    /// Keeps that reply.
    Answer,
    /// Resolves the note that is open.
    Resolve,
    /// Resolves a note from its card beside the page.
    ResolveNote(String),
    /// Shuts the cards that lie over the page, or opens them again.
    Shut(bool),
    /// Puts the block of the conflict that is open back on the page, as the version that still has it.
    PutBack,
    /// Puts the note that is open away.
    Close,
    /// The kind of note the panel lists: all of them when `None`.
    Show(Option<Kind>),
    /// Resolves every open import note.
    ResolveImports,
}

/// The document's notes, and what the window is doing about them.
#[derive(Default)]
pub(crate) struct Margin {
    pub notes: Notes,
    /// Who writes here, as their comments are signed.
    pub author: String,
    /// The selection the tools were put away over. They stay away until it changes.
    away: Option<Selection>,
    /// The comment being written: the passage it is about, and what is written so far.
    draft: Option<(Anchor, Written)>,
    /// Why no comment can be written on this document, as the shell says.
    pub refused: Option<String>,
    /// Whether the person only reads the document, and so writes no comment on it either.
    pub reads: bool,
    /// The note that is open under its passage, and the reply written to it so far.
    thread: Option<(String, Written)>,
    /// Whether the caret opened the note that is open, by coming into its passage. Such a note leaves the
    /// keyboard with the page, and goes when the caret does.
    followed: bool,
    /// The note a press outside it just put away. A press on its own marker goes on from it.
    left: Option<String>,
    /// The markers beside the blocks that have open notes, in the order of the document.
    marks: Vec<Marker>,
    /// Where the open comments and import notes are among the notes, in the order of the page, as their
    /// cards stand beside it. Those whose text is gone come first.
    beside: Vec<usize>,
    /// The block whose marker the pointer rests on.
    pub over: Option<BlockId>,
    /// The time that notes are told against, as RFC 3339. Empty when the shell has no clock.
    pub now: String,
    /// The one kind of note the panel lists, when it does not list them all.
    pub filter: Option<Kind>,
}

impl Margin {
    /// Lets the tools show again once the selection is no longer the one they were put away over.
    pub fn follow(&mut self, selection: Option<&Selection>) {
        if self.away.as_ref() != selection {
            self.away = None;
        }
    }

    /// Whether the tools may show over the selection: not once they were put away over it.
    /// Why nothing can be added to the notes of this document, when nothing can: the shell said so, or
    /// the person only reads the document.
    pub fn refusal(&self) -> Option<&str> {
        self.refused.as_deref().or(self.reads.then_some(READS))
    }

    pub fn offers(&self) -> bool {
        self.away.is_none() && self.draft.is_none() && self.reading().is_none()
    }

    pub fn put_away(&mut self, selection: Option<&Selection>) {
        self.away = selection.cloned();
    }

    /// Takes the notes of the document, tied to the text they are about.
    pub fn take(&mut self, mut notes: Notes, model: &Model) {
        notes.pin(model);
        notes.follow(model);
        self.notes = notes;
        self.remark(model.doc());
    }

    /// Moves every note to where its text is now, after the document changed.
    pub fn edited(&mut self, model: &Model) {
        self.notes.follow(model);
        if let Some((anchor, _)) = &mut self.draft {
            anchor.follow(model);
        }
        self.remark(model.doc());
    }

    /// Starts a comment on what is selected. A selection with no text in it has nothing to comment on.
    pub fn write(&mut self, model: &Model, selection: Option<&Selection>) {
        let passage = selection.and_then(|selection| passage(model.doc(), selection));
        self.thread = None;
        self.draft = passage.map(|mut anchor| {
            // The passage stays with its text while the comment is written.
            anchor.pin(model);
            (anchor, Written::default())
        });
    }

    /// The field that is open, to write in: the comment's, or the reply's to the note that is read. With
    /// it comes how wide the row is that it shares with the button that sends it.
    /// A reply on a card beside the page has the row of the card, `card` wide.
    pub fn written(&mut self, card: Option<f32>) -> Option<(&mut Written, f32)> {
        let draft = self.draft.as_mut().map(|(_, draft)| (draft, WIDTH));
        draft.or(self.thread.as_mut().map(|(_, reply)| (reply, card.unwrap_or(crate::threads::WIDTH))))
    }

    /// The comment being written, as far as it is.
    pub fn drafted(&self) -> Option<&Written> {
        self.draft.as_ref().map(|(_, draft)| draft)
    }

    pub fn drop_draft(&mut self) {
        self.draft = None;
    }

    /// The passage the comment being written is about.
    pub fn writing(&self) -> Option<&Anchor> {
        self.draft.as_ref().map(|(anchor, _)| anchor)
    }

    /// Keeps the comment being written as a note, tied to the text it is about. Returns whether there was
    /// anything to keep: a comment that says nothing is dropped.
    pub fn send(&mut self, model: &Model, id: String, created: String) -> bool {
        let Some((mut anchor, written)) = self.draft.take() else { return false };
        let content = written.content();
        if content.is_empty() || self.refusal().is_some() {
            return false;
        }
        anchor.pin(model);
        self.notes.notes.push(Note {
            id,
            kind: NoteKind::Comment,
            anchor,
            author: self.author.clone(),
            created,
            resolved: false,
            content,
            replies: Vec::new(),
            extra: Default::default(),
        });
        true
    }

    /// The passages to mark on the page: those that open comments are about, the one of the note that is
    /// open and the one a comment is being written on. The last two come with `true`.
    pub fn commented(&self) -> impl Iterator<Item = (Selection, bool)> + '_ {
        let read = self.reading().map(|(note, _)| note.id.as_str());
        let shown = self.open().filter(move |(note, kind)| *kind == Kind::Comment || read == Some(note.id.as_str()));
        let kept = shown.filter_map(move |(note, _)| Some((stretch(&note.anchor)?, read == Some(note.id.as_str()))));
        kept.chain(self.writing().and_then(stretch).map(|passage| (passage, true)))
    }

    /// The notes that wait for attention, of the kinds this build knows, in the order they were added.
    pub fn open(&self) -> impl Iterator<Item = (&Note, Kind)> + '_ {
        let open = self.notes.notes.iter().filter(|note| !note.resolved);
        open.filter_map(|note| Some((note, Kind::of(note)?)))
    }

    /// The open notes an import left: how many, and the first in the order of the page, or as they were
    /// added when none of them has a marker.
    pub fn imports(&self) -> (usize, Option<&str>) {
        let left: Vec<&Note> = self.open().filter(|(_, kind)| *kind == Kind::Import).map(|(note, _)| note).collect();
        let mut marked = self.marks.iter().flat_map(|marker| &marker.notes);
        let first = marked.find_map(|id| left.iter().copied().find(|note| note.id == *id)).or(left.first().copied());
        (left.len(), first.map(|note| note.id.as_str()))
    }

    /// The markers beside the blocks that have open notes.
    pub fn marks(&self) -> &[Marker] {
        &self.marks
    }

    /// Works the markers out again, after the notes changed or the blocks they are about did. A note
    /// inside a table or a gallery is marked beside the whole of it.
    pub fn remark(&mut self, doc: &Document) {
        fn within<'a>(block: &'a Block, visit: &mut impl FnMut(&'a Block)) {
            visit(block);
            block.body.blocks().iter().for_each(|part| within(part, visit));
        }
        // Each open note with where it is among the notes.
        let open = self.notes.notes.iter().enumerate().filter(|(_, note)| !note.resolved);
        let open: Vec<(usize, &Note, Kind)> = open.filter_map(|(at, note)| Some((at, note, Kind::of(note)?))).collect();
        let mut about: HashMap<&BlockId, Vec<(usize, &Note, Kind)>> = HashMap::new();
        for (at, note, kind) in &open {
            about.entry(&note.anchor.block).or_default().push((*at, note, *kind));
        }
        let (mut marks, mut beside) = (Vec::new(), Vec::new());
        for top in doc.blocks.iter().filter(|_| !about.is_empty()) {
            let mut marker: Option<Marker> = None;
            within(top, &mut |block| {
                let Some(notes) = about.get(&block.id) else { return };
                for (_, note, kind) in notes {
                    marker.get_or_insert_with(|| Marker::beside(top, *kind)).add(note, *kind);
                }
                // Down a block the cards go as their passages do.
                let mut cards: Vec<&(usize, &Note, Kind)> = notes.iter().filter(|(.., kind)| kind.carded()).collect();
                cards.sort_by_key(|(_, note, _)| note.anchor.range.as_ref().map_or(0, |range| range.start));
                beside.extend(cards.into_iter().map(|(at, ..)| *at));
            });
            marks.extend(marker);
        }
        let mut placed = vec![false; self.notes.notes.len()];
        beside.iter().for_each(|at| placed[*at] = true);
        let lost = open.iter().filter(|(at, _, kind)| kind.carded() && !placed[*at]);
        let mut all: Vec<usize> = lost.map(|(at, ..)| *at).collect();
        all.append(&mut beside);
        (self.marks, self.beside) = (marks, all);
    }

    /// The open comments and import notes, in the order their cards stand beside the page.
    pub fn cards(&self) -> impl Iterator<Item = &Note> + '_ {
        let carded = |note: &&Note| self.carded(note);
        self.beside.iter().filter_map(|at| self.notes.notes.get(*at)).filter(carded)
    }

    /// Whether a note is one that has a card: an open comment or import note.
    pub fn carded(&self, note: &Note) -> bool {
        !note.resolved && Kind::of(note).is_some_and(Kind::carded)
    }

    /// Takes the card the caret opened as one the person turned to, since they pressed in it.
    pub fn turn(&mut self) {
        self.followed = false;
    }

    /// Leaves the card that is open, as a press on the page or Escape does. One a reply is written on
    /// stays, and so does one the caret is in the passage of. Returns whether a card was left.
    pub fn leave(&mut self, escaped: bool) -> bool {
        let leaves = |(_, reply): &(String, Written)| reply.is_blank() || escaped;
        if (self.followed && !escaped) || !self.thread.as_ref().is_some_and(leaves) {
            return false;
        }
        self.thread = None;
        true
    }

    /// The note that is open under its passage, and the reply written to it so far. A conflict whose
    /// versions show in the page is not read there: see [`Margin::reviewing`].
    pub fn reading(&self) -> Option<(&Note, &Written)> {
        let (id, reply) = self.thread.as_ref()?;
        let note = self.notes.notes.iter().find(|note| &note.id == id && !note.resolved)?;
        (!self.reviewed(note)).then_some((note, reply))
    }

    /// Whether a note is a conflict about a block that the page still has, so that both versions can be
    /// shown where the block is.
    fn reviewed(&self, note: &Note) -> bool {
        let conflict = matches!(note.kind, NoteKind::Conflict { .. });
        conflict && self.marks.iter().any(|marker| marker.notes.contains(&note.id))
    }

    /// The conflict that is open: the page shows both of its versions.
    pub fn reviewing(&self) -> Option<&Note> {
        let (id, _) = self.thread.as_ref()?;
        self.notes.notes.iter().find(|note| &note.id == id && !note.resolved).filter(|note| self.reviewed(note))
    }

    /// The open conflicts whose versions the page can show, in the order of the document.
    pub fn conflicts(&self) -> Vec<&str> {
        let conflict = |id: &&String| self.open().any(|(note, kind)| kind == Kind::Conflict && &note.id == *id);
        self.marks.iter().flat_map(|marker| marker.notes.iter().filter(conflict)).map(String::as_str).collect()
    }

    /// The conflict after the one that is open, or with `back` the one before it, round and round. With
    /// none open, the first or the last.
    pub fn step(&self, back: bool) -> Option<String> {
        let all = self.conflicts();
        let at = self.reviewing().and_then(|note| all.iter().position(|id| *id == note.id));
        let next = match (at, back) {
            (None, false) => 0,
            (None, true) => all.len().checked_sub(1)?,
            (Some(at), false) => (at + 1) % all.len(),
            (Some(at), true) => (at + all.len() - 1) % all.len(),
        };
        all.get(next).map(|id| (*id).to_owned())
    }

    /// The note the keyboard steps to, of those that have a marker, in the order of the document: the one
    /// after the note that is open, or with `back` the one before it, round and round. With none open,
    /// the first beside the block the caret is in or below it, or with `back` the last beside that block
    /// or above it. `None` where there is no other note to go to, and while a reply is being typed, which
    /// keeps its note open.
    pub fn next(&self, back: bool, doc: &Document, caret: Option<&BlockId>) -> Option<String> {
        fn holds(block: &Block, id: &BlockId) -> bool {
            block.id == *id || block.body.blocks().iter().any(|part| holds(part, id))
        }
        if self.thread.as_ref().is_some_and(|(_, reply)| !reply.is_blank()) {
            return None;
        }
        // Each note with where the block of its marker stands in the document.
        let mut all: Vec<(Option<usize>, &String)> = Vec::new();
        for marker in &self.marks {
            let at = doc.blocks.iter().position(|top| top.id == marker.block);
            all.extend(marker.notes.iter().map(|note| (at, note)));
        }
        let open = self.thread.as_ref().and_then(|(open, _)| all.iter().position(|(_, note)| *note == open));
        let next = match open {
            _ if all.is_empty() => return None,
            Some(_) if all.len() == 1 => return None,
            Some(at) if back => (at + all.len() - 1) % all.len(),
            Some(at) => (at + 1) % all.len(),
            None => {
                // The notes beside the blocks above the caret's, and with `back` those beside its own too.
                let own = caret.and_then(|caret| doc.blocks.iter().position(|top| holds(top, caret)));
                let before = |at: usize| own.is_some_and(|own| at < own || (back && at == own));
                let passed = all.iter().filter(|(at, _)| at.is_some_and(before)).count();
                if back { (passed + all.len() - 1) % all.len() } else { passed % all.len() }
            }
        };
        Some(all[next].1.clone())
    }

    /// Settles the conflict that is open, once one of its versions was chosen, and puts it away. Returns
    /// whether it did.
    pub fn settle(&mut self) -> bool {
        let open = self.reviewing().filter(|_| self.refusal().is_none()).map(|note| note.id.clone());
        let Some(id) = open else { return false };
        self.thread = None;
        self.notes.notes.iter_mut().filter(|note| note.id == id).for_each(|note| note.resolved = true);
        true
    }

    /// Opens a note under its passage. What was being written elsewhere is dropped.
    pub fn read(&mut self, id: &str) {
        (self.thread, self.draft, self.left) = (Some((id.to_owned(), Written::default())), None, None);
        self.followed = false;
    }

    /// The open comment whose passage a place in the text is in, either end of it included: the passages
    /// that are marked on the page. Where one lies in another, the shorter is the one meant.
    pub fn about(&self, at: &Pos) -> Option<&str> {
        let comments = self.open().filter(|(note, kind)| *kind == Kind::Comment && note.anchor.block == at.block);
        let around = comments.filter_map(|(note, _)| {
            let passage = note.anchor.range.as_ref().filter(|range| !range.is_empty())?;
            (passage.start..=passage.end).contains(&at.offset).then_some((passage.len(), note.id.as_str()))
        });
        around.min_by_key(|(long, _)| *long).map(|(_, id)| id)
    }

    /// Whether the note that is open is one the caret opened.
    pub fn follows(&self) -> bool {
        self.followed && self.reading().is_some()
    }

    /// Has the notes follow the caret, after the person did something. `was` is the comment whose passage
    /// the selection ended in before that, and `caret` is where the caret is now, when the selection is one.
    ///
    /// A comment whose passage the caret came into is opened, when the window is `free` to show it and no
    /// note is open or being written: nothing the person opened gives way to the caret. A note the caret
    /// opened goes once the caret has left its passage, unless a reply is typed there. Returns whether a
    /// note was opened.
    pub fn attend(&mut self, was: Option<&str>, caret: Option<&Pos>, free: bool) -> bool {
        let at = caret.and_then(|caret| self.about(caret)).map(str::to_owned);
        let left = |(open, reply): &(String, Written)| reply.is_blank() && Some(open) != at.as_ref();
        if self.followed && self.thread.as_ref().is_some_and(left) {
            self.thread = None;
        }
        let Some(at) = at.filter(|at| was != Some(at.as_str())) else { return false };
        let held = self.draft.is_some() || self.reading().is_some() || self.reviewing().is_some();
        if !free || held {
            return false;
        }
        (self.thread, self.followed, self.left) = (Some((at, Written::default())), true, None);
        true
    }

    /// Puts the open note away, as a press outside it or Escape does, and remembers which it was.
    pub fn close(&mut self) {
        self.left = self.thread.take().map(|(id, _)| id);
    }

    /// Puts the open note away because something else came to hang from the page.
    pub fn shut(&mut self) {
        (self.thread, self.left) = (None, None);
    }

    /// Forgets which note was last put away: the press that did it is over.
    pub fn forget(&mut self) {
        self.left = None;
    }

    /// Does what a press on the marker beside `block` asks: opens the first note of the block, or the one
    /// after the note that is open or was just put away by this press. After the last, none is open. A
    /// reply that is being typed keeps its note open.
    pub fn press(&mut self, block: &BlockId) {
        let Some(marker) = self.marks.iter().find(|marker| &marker.block == block) else { return };
        let at = self.thread.as_ref().map(|(id, _)| id).or(self.left.as_ref());
        let at = at.and_then(|id| marker.notes.iter().position(|note| note == id));
        if at.is_some() && self.thread.as_ref().is_some_and(|(_, reply)| !reply.is_blank()) {
            return;
        }
        let next = at.map_or(Some(0), |at| Some(at + 1)).and_then(|next| marker.notes.get(next)).cloned();
        match next {
            Some(id) => self.read(&id),
            None => self.shut(),
        }
    }

    /// Keeps the reply typed into the open note, signed and dated. Returns whether there was anything to
    /// keep.
    pub fn answer(&mut self, created: String) -> bool {
        let free = self.refusal().is_none();
        let Some((id, reply)) = &mut self.thread else { return false };
        let content = reply.content();
        let note = self.notes.notes.iter_mut().find(|note| &note.id == id);
        let Some(note) = note.filter(|_| !content.is_empty() && free) else { return false };
        note.replies.push(Reply { author: self.author.clone(), created, content, extra: Default::default() });
        *reply = Written::default();
        true
    }

    /// Resolves the note that is read and puts it away. Returns whether it did. A conflict whose versions
    /// show in the page is not read: choosing one of them settles it, see [`Margin::settle`].
    pub fn resolve(&mut self) -> bool {
        let read = self.reading().filter(|_| self.refusal().is_none()).map(|(note, _)| note.id.clone());
        let Some(id) = read else { return false };
        self.thread = None;
        self.notes.notes.iter_mut().filter(|note| note.id == id).for_each(|note| note.resolved = true);
        true
    }

    /// Resolves a note from its card. Returns whether it did.
    pub fn resolve_note(&mut self, id: &str) -> bool {
        if self.refusal().is_some() {
            return false;
        }
        if self.thread.as_ref().is_some_and(|(open, _)| open == id) {
            self.thread = None;
        }
        let open = self.notes.notes.iter_mut().find(|note| note.id == id && !note.resolved);
        open.map(|note| note.resolved = true).is_some()
    }

    /// Resolves every open import note. Returns whether there were any.
    pub fn resolve_imports(&mut self) -> bool {
        let free = self.refusal().is_none();
        let open = self.notes.notes.iter_mut().filter(|note| note.kind == NoteKind::Import && !note.resolved);
        let open: Vec<&mut Note> = open.filter(|_| free).collect();
        let any = !open.is_empty();
        open.into_iter().for_each(|note| note.resolved = true);
        any
    }

    /// The tools over a selection: a link and a comment, and nothing else. They hang over the first line of
    /// the selection, or under `under` when there is no room above it. `linked` is whether the selection
    /// is a link, for someone who can make it one: anyone else has the comment alone.
    pub fn tools(linked: Option<bool>, above: bool) -> Popover<Message> {
        let tool = |icon, label: &str| Button::icon_only(icon, label).subtle().small();
        let link = linked.map(|linked| tool(icon::LINK, "Link").pressed(linked).on_press(Message::EditLink));
        let comment = tool(icon::MESSAGE_SQUARE_PLUS, "Comment").on_press(Message::Note(Noting::Write));
        let tools = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_2XS)
            .padding(Edges::all(tokens::SPACE_XS))
            .role(Role::Toolbar)
            .label("Selection")
            .children(link)
            .child(comment);
        Popover::new(tools)
            .flush()
            .side(if above { Side::Top } else { Side::Bottom })
            .gap(TOOLS_GAP)
            .beside()
            .stays()
            .open(true)
            .on_close(Message::Note(Noting::Away))
    }

    /// The field the comment is written in, with the button that sends it, to hang under its passage.
    /// Where no comment can be written, the reason hangs there in place of `field`.
    pub fn field(&self, field: Element<Message>) -> Option<Popover<Message>> {
        let blank = self.drafted()?.is_blank();
        if let Some(why) = self.refusal() {
            let quiet = TextStyle::control().size(tokens::TEXT_SM);
            let said = Element::paragraph(why, quiet).fg(tokens::FG_MUTED).width(WIDTH).role(Role::Status);
            let said = Popover::new(said).label("New comment").max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0);
            return Some(said.open(true).on_close(Message::Note(Noting::Drop)));
        }
        let send = Button::icon_only(icon::SEND, "Send")
            .primary()
            .small()
            .disabled(blank)
            .on_press(Message::Note(Noting::Send));
        // A comment of several lines has the button beside its last.
        let row = Element::row().align(Align::End).gap(tokens::SPACE_SM).width(WIDTH).child(field).child(send);
        let field = Popover::new(row)
            .label("New comment")
            .max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
            .open(true)
            .on_close(Message::Note(Noting::Drop));
        // A stray press on the page does not cost what was typed: Escape drops it.
        Some(if blank { field } else { field.stays() })
    }
}

/// Marks the lines of a commented passage, under its text: a tint, and a line under it as a link has.
/// `open` is a passage whose comment is being written or read.
pub(crate) fn mark(lines: &[Rect], open: bool, scheme: Scheme, out: &mut DisplayList) {
    let tint = if open { fade(tokens::INFO, OPEN) } else { tokens::INFO_SOFT };
    for line in lines {
        out.push(Item::Fill { rect: *line, color: resolve(tint, scheme, 1.0), radius: 0.0 });
        let under = Rect::new(line.x, line.y + line.height - 1.0, line.width, 1.0);
        out.push(Item::Fill { rect: under, color: resolve(tokens::INFO, scheme, 1.0), radius: 0.0 });
    }
}

/// A time in seconds since 1970 as RFC 3339 writes it, in UTC, as notes are dated.
pub(crate) fn stamp(seconds: u64) -> String {
    // The last second of the year 9999, past which the form has no room.
    let seconds = seconds.min(253_402_300_799);
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = date(days);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3_600, rest / 60 % 60, rest % 60)
}

/// The year, month and day of the month that a day counted from 1970 is.
pub(crate) fn date(days: u64) -> (u64, u64, u64) {
    // Days are counted in eras of 400 years that start on 1 March, so that a leap day ends its year.
    let days = days + 719_468;
    let (era, day) = (days / 146_097, days % 146_097);
    let year = (day - day / 1_460 + day / 36_524 - day / 146_096) / 365;
    let day = day - (365 * year + year / 4 - year / 100);
    let month = (5 * day + 2) / 153;
    let day = day - (153 * month + 2) / 5 + 1;
    let month = if month < 10 { month + 3 } else { month - 9 };
    (year + era * 400 + u64::from(month <= 2), month, day)
}

/// An anchor's passage as a selection, when it is a stretch of text and not a whole block.
pub(crate) fn stretch(anchor: &Anchor) -> Option<Selection> {
    let range = anchor.range.as_ref().filter(|range| !range.is_empty())?;
    let at = |offset: usize| Pos::new(anchor.block.clone(), offset);
    Some(Selection { anchor: at(range.start), head: at(range.end) })
}

/// The passage a selection covers, as a note is anchored: the part of it in the first block it has text in.
pub(crate) fn passage(doc: &Document, selection: &Selection) -> Option<Anchor> {
    let place = |pos: &Pos| doc.walk().position(|block| block.id == pos.block);
    let (anchor, head) = (place(&selection.anchor)?, place(&selection.head)?);
    let forward = (anchor, selection.anchor.offset) <= (head, selection.head.offset);
    let ((first, start), (last, end)) = if forward {
        ((anchor, &selection.anchor), (head, &selection.head))
    } else {
        ((head, &selection.head), (anchor, &selection.anchor))
    };
    doc.walk().enumerate().skip(first).take(last - first + 1).find_map(|(at, block)| {
        let text = block.plain_text();
        let from = if at == first { start.offset } else { 0 };
        let to = if at == last { end.offset } else { text.len() };
        (from < to && to <= text.len()).then(|| Anchor::range(block.id.clone(), from..to))
    })
}

/// Makes each web address that is written out in plain text a link to itself, as a note keeps it: one that
/// ends a comment is never followed by the space that makes it a link while it is typed. What is code, or
/// a link already, stays as it is.
pub(crate) fn addressed(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    // The character before, which says whether a word starts.
    let mut before: Option<char> = None;
    for run in runs {
        if run.link.is_some() || run.marks.contains(&Mark::Code) {
            before = run.text.chars().last().or(before);
            out.push(run);
            continue;
        }
        let piece = |text: &str, link: Option<&str>| Run {
            text: text.to_owned(),
            link: link.map(str::to_owned),
            ..run.clone()
        };
        let (mut rest, mut plain) = (run.text.as_str(), String::new());
        while let Some(next) = rest.chars().next() {
            let starts = before.is_none_or(|before| !before.is_alphanumeric());
            if let Some((address, after)) = bare(rest).filter(|_| starts) {
                if !plain.is_empty() {
                    out.push(piece(&std::mem::take(&mut plain), None));
                }
                out.push(piece(address, Some(address)));
                (before, rest) = (address.chars().last(), after);
            } else {
                plain.push(next);
                (before, rest) = (Some(next), &rest[next.len_utf8()..]);
            }
        }
        if !plain.is_empty() {
            out.push(piece(&plain, None));
        }
    }
    out
}

/// What `text` opens with when that is a web address written out, without the stop that ends its sentence.
fn bare(text: &str) -> Option<(&str, &str)> {
    let scheme = ["https://", "http://"].into_iter().find(|scheme| text.starts_with(scheme))?;
    let end = text.find(char::is_whitespace).unwrap_or(text.len());
    let address = text[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
    (address.len() > scheme.len()).then(|| text.split_at(address.len()))
}

#[cfg(test)]
mod tests {
    use letters_model::Block;
    use letters_model::BlockKind::{Paragraph, Quote};

    use super::*;

    #[test]
    fn an_address_written_out_in_a_comment_is_a_link_when_the_comment_is_kept() {
        const A: &str = "https://a.example/q?x=1";
        let bold = |text: &str| Run::marked(text, [Mark::Bold]);
        let code = |text: &str| Run::marked(text, [Mark::Code]);
        let link = |run: Run, to: &str| Run { link: Some(to.to_owned()), ..run };
        // What was written, and what is kept.
        let cases = [
            (vec![Run::plain("Has anyone chased it?")], vec![Run::plain("Has anyone chased it?")]),
            (
                vec![Run::plain(format!("at {A}. Thanks"))],
                vec![Run::plain("at "), link(Run::plain(A), A), Run::plain(". Thanks")],
            ),
            (vec![Run::plain(format!("see {A}"))], vec![Run::plain("see "), link(Run::plain(A), A)]),
            // The marks of the text are the address's too.
            (vec![Run::plain("see "), bold(A)], vec![Run::plain("see "), link(bold(A), A)]),
            // What is a link already, what is code and what only holds an address stay as written.
            (vec![link(Run::plain(A), "https://b.example")], vec![link(Run::plain(A), "https://b.example")]),
            (vec![code(A)], vec![code(A)]),
            (vec![Run::plain(format!("x{A}"))], vec![Run::plain(format!("x{A}"))]),
            (vec![Run::plain("ab"), bold(A)], vec![Run::plain("ab"), bold(A)]),
        ];
        for (written, kept) in cases {
            assert_eq!(addressed(written.clone()), kept, "{written:?}");
        }
    }

    #[test]
    fn notes_inside_a_block_are_marked_beside_the_whole_of_it_in_the_order_of_the_page() {
        let part = |id: &str| Block::text(id, Paragraph, "One two");
        let quote = Block { body: letters_model::Body::Blocks(vec![part("q1"), part("q2")]), ..Block::new("q", Quote) };
        let doc = Document { blocks: vec![part("a"), quote, part("b")], ..Document::default() };
        let note = |id: &str, block: &str| Note {
            id: id.into(),
            kind: NoteKind::Comment,
            anchor: Anchor::range(block, 0..3),
            author: String::new(),
            created: String::new(),
            resolved: false,
            content: vec![Run::plain("Why?")],
            replies: Vec::new(),
            extra: Default::default(),
        };
        let notes = vec![note("1", "b"), note("2", "q2"), note("3", "q1"), note("4", "gone")];
        let mut margin = Margin { notes: Notes { notes, ..Notes::default() }, ..Margin::default() };
        margin.remark(&doc);
        let marks: Vec<(&str, Vec<&str>)> = margin
            .marks()
            .iter()
            .map(|marker| (marker.block.as_str(), marker.notes.iter().map(String::as_str).collect()))
            .collect();
        assert_eq!(marks, [("q", vec!["3", "2"]), ("b", vec!["1"])]);

        // A press on a marker goes through its notes, and then none is open.
        let q = BlockId::new("q");
        let mut read = Vec::new();
        for _ in 0..3 {
            margin.press(&q);
            read.push(margin.reading().map(|(note, _)| note.id.clone()));
        }
        assert_eq!(read, [Some("3".to_owned()), Some("2".to_owned()), None]);
    }

    #[test]
    fn a_note_is_dated_as_rfc_3339_writes_a_time() {
        // Seconds since 1970, and the date.
        let cases = [
            (0, "1970-01-01T00:00:00Z"),
            (951_782_399, "2000-02-28T23:59:59Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
            (1_794_709_230, "2026-11-15T02:20:30Z"),
        ];
        for (seconds, date) in cases {
            assert_eq!(stamp(seconds), date);
        }
    }

    #[test]
    fn a_note_is_anchored_to_the_selected_text_of_one_block() {
        let blocks = vec![Block::text("a", Paragraph, "One two"), Block::text("b", Paragraph, "three four")];
        let doc = Document { blocks, ..Document::default() };
        let select = |from: (&str, usize), to: (&str, usize)| Selection {
            anchor: Pos::new(from.0, from.1),
            head: Pos::new(to.0, to.1),
        };
        // A selection, and the block and range its note is anchored to.
        let cases = [
            (select(("a", 4), ("a", 7)), Some(("a", 4..7))),
            (select(("a", 7), ("a", 4)), Some(("a", 4..7))),
            // Over two blocks, the part in the first.
            (select(("a", 4), ("b", 5)), Some(("a", 4..7))),
            (select(("b", 5), ("a", 4)), Some(("a", 4..7))),
            // From the end of a block, the part in the next.
            (select(("a", 7), ("b", 5)), Some(("b", 0..5))),
            (select(("a", 4), ("a", 4)), None),
            (select(("a", 4), ("gone", 1)), None),
        ];
        for (selection, anchored) in cases {
            let anchor = passage(&doc, &selection).map(|anchor| (anchor.block, anchor.range));
            let anchored = anchored.map(|(block, range)| (block.into(), Some(range)));
            assert_eq!(anchor, anchored, "{selection:?}");
        }
    }
}
