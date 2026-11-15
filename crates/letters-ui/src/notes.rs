//! Notes on a passage: what people say about it, kept beside the page.
//!
//! Over a stretch of selected text float the two things that are done to a passage: linking it and
//! commenting on it. A comment is written in a field that hangs under its passage, in Markdown's bold,
//! italic and links, and is kept as a note that stays with its text.

use std::collections::BTreeSet;

use accesskit::Role;
use letters_model::{Anchor, Document, Mark, Model, Note, NoteKind, Notes, Pos, Run, Selection};
use letters_render::{DisplayList, Item, Rect};

use crate::color::{fade, resolve};
use crate::element::Element;
use crate::icon;
use crate::style::{Align, Edges};
use crate::tokens::{self, Scheme};
use crate::widgets::{Button, ControlSize, Popover, Side, TextField};
use crate::window::Message;

/// How wide the field a comment is written in hangs under its passage.
const WIDTH: f32 = 320.0;
/// The room between a selection and the tools over it.
const TOOLS_GAP: f32 = 8.0;
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
        self.away.is_none() && self.draft.is_none()
    }

    pub fn put_away(&mut self, selection: Option<&Selection>) {
        self.away = selection.cloned();
    }

    /// Takes the notes of the document, tied to the text they are about.
    pub fn take(&mut self, mut notes: Notes, model: &Model) {
        notes.pin(model);
        notes.follow(model);
        self.notes = notes;
    }

    /// Moves every note to where its text is now, after the document changed.
    pub fn edited(&mut self, model: &Model) {
        self.notes.follow(model);
        if let Some((anchor, _)) = &mut self.draft {
            anchor.follow(model);
        }
    }

    /// Starts a comment on what is selected. A selection with no text in it has nothing to comment on.
    pub fn write(&mut self, model: &Model, selection: Option<&Selection>) {
        let passage = selection.and_then(|selection| passage(model.doc(), selection));
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
        if content.is_empty() {
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

    /// The passages to mark on the page: those that open comments are about, and the one a comment is
    /// being written on, which comes with `true`.
    pub fn commented(&self) -> impl Iterator<Item = (Selection, bool)> + '_ {
        let open = self.notes.notes.iter().filter(|note| note.kind == NoteKind::Comment && !note.resolved);
        let kept = open.filter_map(|note| Some((stretch(&note.anchor)?, false)));
        kept.chain(self.writing().and_then(stretch).map(|passage| (passage, true)))
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

    /// The field the comment is written in, to hang under its passage.
    pub fn field(&self) -> Option<Popover<Message>> {
        let (_, text) = self.draft.as_ref()?;
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

/// Reads a comment as it was typed: Markdown's bold, italic and links, a bare address as a link to itself,
/// and everything else as it stands.
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
            marked.insert(mark);
            read(inner, &marked, link, out);
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

/// What `text` opens with when that is bold or italic text between its marks: the mark, the text inside
/// and what follows. An underscore marks only whole words, so that a `name_like_this` stays as typed.
fn fenced(text: &str, starts: bool) -> Option<(Mark, &str, &str)> {
    let fences = [("**", Mark::Bold), ("*", Mark::Italic), ("_", Mark::Italic)];
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
    use letters_model::BlockKind::Paragraph;

    use super::*;

    #[test]
    fn a_comment_is_read_with_markdowns_bold_italic_and_links() {
        const B: u8 = 1;
        const I: u8 = 2;
        // What is typed, and its runs: the text, its marks and its link.
        type Runs<'a> = &'a [(&'a str, u8, &'a str)];
        let cases: [(&str, Runs); 12] = [
            ("Has anyone chased it? \u{26a1}", &[("Has anyone chased it? \u{26a1}", 0, "")]),
            ("due **this Friday**, see", &[("due ", 0, ""), ("this Friday", B, ""), (", see", 0, "")]),
            ("tell *Harland* and _Sam_", &[("tell ", 0, ""), ("Harland", I, ""), (" and ", 0, ""), ("Sam", I, "")]),
            ("*a **b** c*", &[("a ", I, ""), ("b", B | I, ""), (" c", I, "")]),
            (
                "see [their email](https://a.example/m)!",
                &[("see ", 0, ""), ("their email", 0, "https://a.example/m"), ("!", 0, "")],
            ),
            ("[a **b**](x:y)", &[("a ", 0, "x:y"), ("b", B, "x:y")]),
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
                    let marks =
                        u8::from(run.marks.contains(&Mark::Bold)) * B + u8::from(run.marks.contains(&Mark::Italic)) * I;
                    (run.text, marks, run.link.unwrap_or_default())
                })
                .collect();
            let runs: Vec<(String, u8, String)> =
                runs.iter().map(|(text, marks, link)| ((*text).to_owned(), *marks, (*link).to_owned())).collect();
            assert_eq!(read, runs, "{typed:?}");
        }
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
