//! Notes on a passage: what people say about it, kept beside the page.
//!
//! Over a stretch of selected text float the two things that are done to a passage: linking it and
//! commenting on it. A comment is written in a field that hangs under its passage, in Markdown's bold,
//! italic, struck text, code and links, and is kept as a note that stays with its text. How notes show
//! once they are kept is in [`crate::threads`].

use std::collections::{BTreeSet, HashMap};

use accesskit::Role;
use letters_model::{Anchor, Block, BlockId, Document, Mark, Model, Note, NoteKind, Notes, Pos, Reply, Run, Selection};
use letters_render::{DisplayList, Item, Rect};

use crate::color::{fade, resolve};
use crate::element::Element;
use crate::icon;
use crate::style::{Align, Edges};
use crate::text::TextStyle;
use crate::threads::{Kind, Marker};
use crate::tokens::{self, Scheme};
use crate::widgets::{Button, ControlSize, Popover, Side, TextField};
use crate::window::Message;

/// How wide the field a comment is written in hangs under its passage.
const WIDTH: f32 = 320.0;
/// The room between a selection and the tools over it.
pub(crate) const TOOLS_GAP: f32 = 8.0;
/// The room the tools take above a selection, which they hang under when it is not there.
pub(crate) const TOOLS_ROOM: f32 = tokens::CONTROL_HEIGHT_SM + 2.0 * tokens::SPACE_XS + 2.0 + TOOLS_GAP;
/// How strong the tint of a passage is while its comment is being written or read.
const OPEN: f32 = 0.26;

/// What the pointer and the keyboard ask about notes.
#[derive(Clone, Debug, PartialEq)]
pub enum Noting {
    /// Puts the tools over the selection away, until the selection changes.
    Away,
    /// Starts a comment on the selected passage.
    Write,
    /// What is typed into the comment being written.
    Draft(String),
    /// Keeps the comment being written.
    Send,
    /// Drops it.
    Drop,
    /// Opens a note in place, under the passage it is about.
    Open(String),
    /// A press on the marker beside a block: opens the first of its notes, or the one after the note
    /// that is open, and puts the last one away.
    Press(BlockId),
    /// What is typed into the reply to the note that is open.
    Reply(String),
    /// Keeps that reply.
    Answer,
    /// Resolves the note that is open.
    Resolve,
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
    /// The comment being written: the passage it is about, and what is typed so far.
    draft: Option<(Anchor, String)>,
    /// Why no comment can be written on this document, as the shell says.
    pub refused: Option<String>,
    /// The note that is open under its passage, and the reply typed into it so far.
    thread: Option<(String, String)>,
    /// Whether the caret opened the note that is open, by coming into its passage. Such a note leaves the
    /// keyboard with the page, and goes when the caret does.
    followed: bool,
    /// The note a press outside it just put away. A press on its own marker goes on from it.
    left: Option<String>,
    /// The markers beside the blocks that have open notes, in the order of the document.
    marks: Vec<Marker>,
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
            (anchor, String::new())
        });
    }

    pub fn typed(&mut self, text: String) {
        if let Some((_, draft)) = &mut self.draft {
            *draft = text;
        }
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
        let Some((mut anchor, text)) = self.draft.take() else { return false };
        let content = inline(text.trim());
        if content.is_empty() || self.refused.is_some() {
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
        let mut about: HashMap<&BlockId, Vec<(&Note, Kind)>> = HashMap::new();
        for (note, kind) in self.open() {
            about.entry(&note.anchor.block).or_default().push((note, kind));
        }
        let mut marks = Vec::new();
        for top in doc.blocks.iter().filter(|_| !about.is_empty()) {
            let mut marker: Option<Marker> = None;
            within(top, &mut |block| {
                for (note, kind) in about.get(&block.id).into_iter().flatten() {
                    marker.get_or_insert_with(|| Marker::beside(top, *kind)).add(note, *kind);
                }
            });
            marks.extend(marker);
        }
        self.marks = marks;
    }

    /// The note that is open under its passage, and the reply typed into it so far. A conflict whose
    /// versions show in the page is not read there: see [`Margin::reviewing`].
    pub fn reading(&self) -> Option<(&Note, &str)> {
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

    /// Settles the conflict that is open, once one of its versions was chosen, and puts it away. Returns
    /// whether it did.
    pub fn settle(&mut self) -> bool {
        let open = self.reviewing().filter(|_| self.refused.is_none()).map(|note| note.id.clone());
        let Some(id) = open else { return false };
        self.thread = None;
        self.notes.notes.iter_mut().filter(|note| note.id == id).for_each(|note| note.resolved = true);
        true
    }

    /// Opens a note under its passage. What was being written elsewhere is dropped.
    pub fn read(&mut self, id: &str) {
        (self.thread, self.draft, self.left) = (Some((id.to_owned(), String::new())), None, None);
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
        let left = |(open, reply): &(String, String)| reply.trim().is_empty() && Some(open) != at.as_ref();
        if self.followed && self.thread.as_ref().is_some_and(left) {
            self.thread = None;
        }
        let Some(at) = at.filter(|at| was != Some(at.as_str())) else { return false };
        let held = self.draft.is_some() || self.reading().is_some() || self.reviewing().is_some();
        if !free || held {
            return false;
        }
        (self.thread, self.followed, self.left) = (Some((at, String::new())), true, None);
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
        if at.is_some() && self.thread.as_ref().is_some_and(|(_, reply)| !reply.trim().is_empty()) {
            return;
        }
        let next = at.map_or(Some(0), |at| Some(at + 1)).and_then(|next| marker.notes.get(next)).cloned();
        match next {
            Some(id) => self.read(&id),
            None => self.shut(),
        }
    }

    pub fn reply(&mut self, text: String) {
        if let Some((_, reply)) = &mut self.thread {
            *reply = text;
        }
    }

    /// Keeps the reply typed into the open note, signed and dated. Returns whether there was anything to
    /// keep.
    pub fn answer(&mut self, created: String) -> bool {
        let Some((id, reply)) = &mut self.thread else { return false };
        let content = inline(reply.trim());
        let note = self.notes.notes.iter_mut().find(|note| &note.id == id);
        let Some(note) = note.filter(|_| !content.is_empty() && self.refused.is_none()) else { return false };
        note.replies.push(Reply { author: self.author.clone(), created, content, extra: Default::default() });
        reply.clear();
        true
    }

    /// Resolves the note that is read and puts it away. Returns whether it did. A conflict whose versions
    /// show in the page is not read: choosing one of them settles it, see [`Margin::settle`].
    pub fn resolve(&mut self) -> bool {
        let read = self.reading().filter(|_| self.refused.is_none()).map(|(note, _)| note.id.clone());
        let Some(id) = read else { return false };
        self.thread = None;
        self.notes.notes.iter_mut().filter(|note| note.id == id).for_each(|note| note.resolved = true);
        true
    }

    /// Resolves every open import note. Returns whether there were any.
    pub fn resolve_imports(&mut self) -> bool {
        let open = self.notes.notes.iter_mut().filter(|note| note.kind == NoteKind::Import && !note.resolved);
        let open: Vec<&mut Note> = open.filter(|_| self.refused.is_none()).collect();
        let any = !open.is_empty();
        open.into_iter().for_each(|note| note.resolved = true);
        any
    }

    /// The tools over a selection: a link and a comment, and nothing else. They hang over the first line of
    /// the selection, or under `under` when there is no room above it.
    pub fn tools(linked: bool, above: bool) -> Popover<Message> {
        let tool = |icon, label: &str| Button::icon_only(icon, label).subtle().small();
        let link = tool(icon::LINK, "Link").pressed(linked).on_press(Message::EditLink);
        let comment = tool(icon::MESSAGE_SQUARE_PLUS, "Comment").on_press(Message::Note(Noting::Write));
        let tools = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_2XS)
            .padding(Edges::all(tokens::SPACE_XS))
            .role(Role::Toolbar)
            .label("Selection")
            .child(link)
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

    /// The field the comment is written in, to hang under its passage. Where no comment can be written,
    /// the reason hangs there in its place.
    pub fn field(&self) -> Option<Popover<Message>> {
        let (_, text) = self.draft.as_ref()?;
        if let Some(why) = &self.refused {
            let quiet = TextStyle::control().size(tokens::TEXT_SM);
            let said = Element::paragraph(why.as_str(), quiet).fg(tokens::FG_MUTED).width(WIDTH).role(Role::Status);
            let said = Popover::new(said).label("New comment").max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0);
            return Some(said.open(true).on_close(Message::Note(Noting::Drop)));
        }
        let field = TextField::new("Comment", text)
            .placeholder("Comment on this")
            .size(ControlSize::Small)
            .on_input(|text| Message::Note(Noting::Draft(text)))
            .on_submit(Message::Note(Noting::Send))
            .on_cancel(Message::Note(Noting::Drop));
        let send = Button::icon_only(icon::SEND, "Send")
            .primary()
            .small()
            .disabled(text.trim().is_empty())
            .on_press(Message::Note(Noting::Send));
        let row = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .width(WIDTH)
            .child(Element::from(field).grow(1.0))
            .child(send);
        let field = Popover::new(row)
            .label("New comment")
            .max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
            .open(true)
            .on_close(Message::Note(Noting::Drop));
        // A stray press on the page does not cost what was typed: Escape drops it.
        Some(if text.trim().is_empty() { field } else { field.stays() })
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

/// Reads a comment as it was typed: Markdown's bold, italic, struck text, code and links, a bare address
/// as a link to itself, and everything else as it stands.
pub(crate) fn inline(text: &str) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    read(text, &BTreeSet::new(), None, &mut runs);
    runs
}

/// What a stretch of a comment is set in.
type Format<'a> = (&'a BTreeSet<Mark>, Option<&'a str>);

fn push(text: &str, (marks, link): Format, out: &mut Vec<Run>) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(last) if &last.marks == marks && last.link.as_deref() == link => last.text.push_str(text),
        _ => out.push(Run { link: link.map(str::to_owned), ..Run::marked(text, marks.iter().cloned()) }),
    }
}

fn read(text: &str, marks: &BTreeSet<Mark>, link: Option<&str>, out: &mut Vec<Run>) {
    let mut rest = text;
    // The character before `rest`, which says whether a word starts there.
    let mut before: Option<char> = None;
    while let Some(next) = rest.chars().next() {
        let starts = before.is_none_or(|before| !before.is_alphanumeric());
        if let Some((mark, inner, after)) = fenced(rest, starts) {
            let mut marked = marks.clone();
            let code = mark == Mark::Code;
            marked.insert(mark);
            // Code is kept as typed: nothing in it is a mark.
            match code {
                true => push(inner, (&marked, link), out),
                false => read(inner, &marked, link, out),
            }
            (before, rest) = (inner.chars().last(), after);
        } else if let Some((label, address, after)) = linked(rest).filter(|_| link.is_none()) {
            read(label, marks, Some(address), out);
            (before, rest) = (Some(')'), after);
        } else if let Some((address, after)) = bare(rest).filter(|_| starts && link.is_none()) {
            push(address, (marks, Some(address)), out);
            (before, rest) = (address.chars().last(), after);
        } else {
            let (one, after) = rest.split_at(next.len_utf8());
            push(one, (marks, link), out);
            (before, rest) = (Some(next), after);
        }
    }
}

/// What `text` opens with when that is text between a pair of marks: the mark, the text inside and what
/// follows. An underscore marks only whole words, so that a `name_like_this` stays as typed.
fn fenced(text: &str, starts: bool) -> Option<(Mark, &str, &str)> {
    let fences =
        [("**", Mark::Bold), ("~~", Mark::Strike), ("*", Mark::Italic), ("_", Mark::Italic), ("`", Mark::Code)];
    fences.into_iter().find_map(|(fence, mark)| {
        let body = text.strip_prefix(fence).filter(|_| fence != "_" || starts)?;
        let closes = |at: &usize| {
            let (inner, after) = (&body[..*at], &body[at + fence.len()..]);
            // A single star is not one of a pair of them.
            let lone = fence != "*" || !(inner.ends_with('*') || after.starts_with('*'));
            let worded = fence != "_" || !after.starts_with(char::is_alphanumeric);
            lone && worded && !inner.is_empty() && !inner.starts_with(char::is_whitespace) && !inner.ends_with(' ')
        };
        let close = body.match_indices(fence).map(|(at, _)| at).find(closes)?;
        Some((mark, &body[..close], &body[close + fence.len()..]))
    })
}

/// What `text` opens with when that is a link as Markdown writes it: its words, its address and what follows.
fn linked(text: &str) -> Option<(&str, &str, &str)> {
    let (label, rest) = text.strip_prefix('[')?.split_once("](")?;
    let (address, after) = rest.split_once(')')?;
    let whole = !label.is_empty() && !label.contains(['[', '\n']) && !address.is_empty();
    (whole && !address.contains(char::is_whitespace)).then_some((label, address, after))
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
    fn a_comment_is_read_with_markdowns_bold_italic_and_links() {
        const B: u8 = 1;
        const I: u8 = 2;
        const S: u8 = 4;
        const C: u8 = 8;
        // What is typed, and its runs: the text, its marks and its link.
        type Runs<'a> = &'a [(&'a str, u8, &'a str)];
        let cases: [(&str, Runs); 14] = [
            ("Has anyone chased it? \u{26a1}", &[("Has anyone chased it? \u{26a1}", 0, "")]),
            ("due **this Friday**, see", &[("due ", 0, ""), ("this Friday", B, ""), (", see", 0, "")]),
            ("tell *Harland* and _Sam_", &[("tell ", 0, ""), ("Harland", I, ""), (" and ", 0, ""), ("Sam", I, "")]),
            ("*a **b** c*", &[("a ", I, ""), ("b", B | I, ""), (" c", I, "")]),
            (
                "see [their email](https://a.example/m)!",
                &[("see ", 0, ""), ("their email", 0, "https://a.example/m"), ("!", 0, "")],
            ),
            ("[a **b**](x:y)", &[("a ", 0, "x:y"), ("b", B, "x:y")]),
            ("~~Monday~~ **Tuesday**", &[("Monday", S, ""), (" ", 0, ""), ("Tuesday", B, "")]),
            // Code is kept as typed.
            ("run `a *b* c` twice", &[("run ", 0, ""), ("a *b* c", C, ""), (" twice", 0, "")]),
            (
                "at https://a.example/q?x=1. Thanks",
                &[("at ", 0, ""), ("https://a.example/q?x=1", 0, "https://a.example/q?x=1"), (". Thanks", 0, "")],
            ),
            // What only looks like a mark stays as typed.
            ("2 * 3 * 4", &[("2 * 3 * 4", 0, "")]),
            ("the roof_plan_v2 file", &[("the roof_plan_v2 file", 0, "")]),
            ("**open", &[("**open", 0, "")]),
            ("[not a link] (here)", &[("[not a link] (here)", 0, "")]),
            ("", &[]),
        ];
        for (typed, runs) in cases {
            let read: Vec<(String, u8, String)> = inline(typed)
                .into_iter()
                .map(|run| {
                    let has = |mark: Mark, bit: u8| u8::from(run.marks.contains(&mark)) * bit;
                    let marks = has(Mark::Bold, B) + has(Mark::Italic, I) + has(Mark::Strike, S) + has(Mark::Code, C);
                    (run.text, marks, run.link.unwrap_or_default())
                })
                .collect();
            let runs: Vec<(String, u8, String)> =
                runs.iter().map(|(text, marks, link)| ((*text).to_owned(), *marks, (*link).to_owned())).collect();
            assert_eq!(read, runs, "{typed:?}");
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
