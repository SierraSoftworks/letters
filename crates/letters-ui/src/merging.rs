//! Merging another file of this document into it: the bar over a document a sync tool left conflict copies
//! of, or that is such a copy itself, and the card that names the files before they are merged.
//!
//! The window only shows and asks. The shell finds the copies, judges each file and does the merge, whose
//! edits arrive as another writer's do.

use accesskit::Role;

use crate::element::Element;
use crate::icon;
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Banner, Button, Popover, Side, Tone, banner};
use crate::window::Message;

/// How wide the card is, inside its padding.
const CARD_WIDTH: f32 = 400.0;

/// A file that can be merged into the document, as the shell names it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeFile {
    /// What the shell knows the file by. It comes back in a [`Request::MergeFrom`](crate::Request).
    pub path: String,
    /// The file's name, as the person is to read it.
    pub name: String,
    /// The folder it is in, as the bar shows places: `Family / House`.
    pub folder: String,
}

/// The files the shell wants the person to see before it merges them, as it says with `Window::ask_merge`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MergeAsk {
    /// Every conflict copy, or the one file the person chose.
    pub files: Vec<MergeFile>,
    /// Whether these are the document's conflict copies, which go to the trash once merged.
    pub conflicts: bool,
}

/// What the pointer and the keyboard ask about merging.
#[derive(Clone, Debug, PartialEq)]
pub enum Merging {
    /// Merges every conflict copy the shell told of.
    Conflicts,
    /// Lets the person choose a file to merge.
    FromFile,
    /// Merges this document, a conflict copy, into the document it is a copy of.
    IntoOriginal,
    /// The person has read the files on the card and wants them merged.
    Confirm,
    /// Puts the card away.
    Leave,
    /// Puts the bar away until there is something new to say on it.
    Hide,
}

/// What a yes on the card asks the shell for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Confirmed {
    Conflicts,
    File(String),
    Original,
}

/// What the card asks about.
enum Card {
    /// Files to merge into this document.
    Files(MergeAsk),
    /// The document to merge this one into.
    Original(MergeFile),
}

#[derive(Default)]
pub(crate) struct Merges {
    /// The conflict copies the shell last told of. `None` until it has: a shell that never does cannot
    /// merge, and is not asked to.
    copies: Option<Vec<MergeFile>>,
    /// The document this one is a conflict copy of, when it is one.
    original: Option<MergeFile>,
    hidden: bool,
    /// Whether the shell was asked for the merge the bar offers and has not answered.
    asked: bool,
    card: Option<Card>,
    /// Why the last merge could not be done.
    refused: Option<String>,
}

impl Merges {
    /// Takes the copies there are now. A bar that was put away comes back when they are others.
    pub fn set(&mut self, copies: Vec<MergeFile>) {
        if self.copies.as_ref() != Some(&copies) {
            self.hidden = false;
        }
        (self.copies, self.asked) = (Some(copies), false);
    }

    /// Takes the document this one is a conflict copy of, or that it is one no longer.
    pub fn set_original(&mut self, original: Option<MergeFile>) {
        if self.original != original {
            self.hidden = false;
        }
        (self.original, self.asked) = (original, false);
    }

    /// Whether nothing is asked of the shell or of the person: a card is answered before another merge.
    fn idle(&self) -> bool {
        !self.asked && self.card.is_none()
    }

    /// Whether this document is a conflict copy, and no merge into its original is under way.
    pub fn copied(&self) -> bool {
        self.idle() && self.original.is_some()
    }

    /// Whether the shell merges files at all, and a file can be chosen now.
    pub fn can(&self) -> bool {
        self.copies.is_some() && self.card.is_none()
    }

    /// Whether there are conflict copies to merge, and no merge of them under way.
    pub fn waiting(&self) -> bool {
        self.idle() && self.copies.as_ref().is_some_and(|copies| !copies.is_empty())
    }

    /// Notes that the shell was asked for the merge the bar offers. What was said of the last merge is past.
    pub fn asked(&mut self) {
        (self.asked, self.refused) = (true, None);
    }

    /// The shell wants the person to see what is merged first.
    pub fn ask(&mut self, ask: MergeAsk) {
        (self.card, self.asked) = ((!ask.files.is_empty()).then_some(Card::Files(ask)), false);
    }

    /// The shell wants the person to see what this document is merged into first.
    pub fn ask_into(&mut self, original: MergeFile) {
        (self.card, self.asked) = (Some(Card::Original(original)), false);
    }

    /// The shell could not merge, and says why. The bar says it, put away or not.
    pub fn refuse(&mut self, why: &str) {
        (self.refused, self.asked) = (Some(why.to_owned()), false);
    }

    pub fn leave(&mut self) {
        self.card = None;
    }

    /// Puts the bar away, with what it said of the last merge.
    pub fn hide(&mut self) {
        (self.hidden, self.refused) = (true, None);
    }

    /// Puts the card away. Returns what it asked about.
    pub fn confirm(&mut self) -> Option<Confirmed> {
        let ask = match self.card.take()? {
            Card::Files(ask) => ask,
            Card::Original(_) => {
                self.asked = true;
                return Some(Confirmed::Original);
            }
        };
        if ask.conflicts {
            self.asked = true;
            return Some(Confirmed::Conflicts);
        }
        ask.files.into_iter().next().map(|file| Confirmed::File(file.path))
    }

    fn shows(&self) -> bool {
        let copies = self.copies.as_ref().is_some_and(|copies| !copies.is_empty());
        self.refused.is_some() || ((copies || self.original.is_some()) && !self.hidden)
    }

    /// The room the bar takes above the page.
    pub fn height(&self) -> f32 {
        if self.shows() { banner::HEIGHT } else { 0.0 }
    }

    /// The bar under the top bar, while there is something to merge or a merge that could not be done. A
    /// document that is a conflict copy says that first: its own copies wait for it to be the original.
    pub fn bar(&self) -> Option<Element<Message>> {
        if !self.shows() {
            return None;
        }
        let copies = self.copies.as_deref().unwrap_or_default();
        let mut bar = match (&self.refused, &self.original, copies) {
            (Some(why), ..) => Banner::new(why.as_str()).tone(Tone::Warn),
            (None, Some(original), _) => Banner::new(format!("This is a conflict copy of {}.", original.name)),
            (None, None, [copy]) => {
                Banner::new(format!("A conflict copy has edits this document lacks: {}", copy.name))
            }
            (None, None, copies) => {
                Banner::new(format!("{} conflict copies have edits this document lacks.", copies.len()))
            }
        };
        if let Some(original) = &self.original {
            let does = format!("Brings this copy's edits into {} and moves this copy to the trash", original.name);
            let merge = Button::new("Merge into the original").small().disabled(self.asked);
            let merge = merge.on_press(Message::Merge(Merging::IntoOriginal));
            bar = bar.action(Element::from(merge).tooltip(does, Side::Bottom));
        } else if !copies.is_empty() {
            let does = match copies.len() {
                1 => "Brings its edits into this document and moves the copy to the trash",
                _ => "Brings their edits into this document and moves the copies to the trash",
            };
            let merge = Button::new("Merge conflicts").small().disabled(self.asked);
            let merge = merge.on_press(Message::Merge(Merging::Conflicts));
            bar = bar.action(Element::from(merge).tooltip(does, Side::Bottom));
        }
        Some(bar.icon(icon::GIT_MERGE).on_dismiss("Hide", Message::Merge(Merging::Hide)).into())
    }

    /// The card over the middle of the window, while the shell asks about files to merge. It hangs from
    /// an element of no size, which the window puts after everything else.
    pub fn card(&self) -> Option<Element<Message>> {
        let send = |asked: Merging| Message::Merge(asked);
        let (files, title, does) = match self.card.as_ref()? {
            Card::Original(original) => (
                std::slice::from_ref(original),
                "Merge into the original?",
                "This copy's edits go into it, and this copy moves to the trash. The window goes on with the original.",
            ),
            Card::Files(ask) => {
                let (title, does) = match (ask.conflicts, ask.files.len()) {
                    (true, 1) => (
                        "Merge this conflict copy?",
                        "Its edits come into this document, and the copy moves to the trash.",
                    ),
                    (true, _) => (
                        "Merge these conflict copies?",
                        "Their edits come into this document, and the copies move to the trash.",
                    ),
                    (false, _) => {
                        ("Merge this file?", "Its edits come into this document. The file stays where it is.")
                    }
                };
                (&ask.files[..], title, does)
            }
        };
        let head = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .child(Element::icon(icon::GIT_MERGE, 20.0).fg(tokens::FG_MUTED))
            .child(Element::text(title, TextStyle::control().weight(600)));
        // Each file by its name, with where it lies under it.
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let files = files.iter().map(|file| {
            Element::column()
                .role(Role::ListItem)
                .label(format!("{}, in {}", file.name, file.folder))
                .child(Element::paragraph(file.name.as_str(), TextStyle::control().weight(500)))
                .child(Element::paragraph(file.folder.as_str(), quiet).fg(tokens::FG_MUTED))
        });
        let files = Element::column()
            .gap(tokens::SPACE_SM)
            .padding(Edges::all(tokens::SPACE_SM))
            .bg(tokens::NEUTRAL_SOFT)
            .radius(tokens::RADIUS_SM)
            .role(Role::List)
            .label("Files to merge")
            .children(files);
        let merge = Button::new("Merge").icon(icon::GIT_MERGE).primary().on_press(send(Merging::Confirm));
        let foot = Element::row()
            .align(Align::Center)
            .justify(Justify::End)
            .gap(tokens::SPACE_SM)
            .child(Button::new("Cancel").on_press(send(Merging::Leave)))
            .child(Element::from(merge).key("merge").takes_focus());
        let card = Element::column()
            .gap(tokens::SPACE_MD)
            .width(CARD_WIDTH)
            .child(head)
            .child(files)
            .child(Element::paragraph(does, quiet).fg(tokens::FG_MUTED))
            .child(foot);
        let card = Popover::new(card)
            .label(title)
            .max_width(CARD_WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
            .modal()
            .open(true)
            .on_close(send(Merging::Leave));
        Some(Element::row().overlay().popover(card))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(name: &str) -> MergeFile {
        MergeFile { path: format!("/home/sam/{name}"), name: name.to_owned(), folder: "Home".to_owned() }
    }

    #[test]
    fn a_bar_that_was_put_away_comes_back_only_for_other_copies() {
        // What the shell tells next, and whether the bar shows then.
        let cases = [(vec![copy("a")], false), (vec![copy("a"), copy("b")], true), (vec![], false)];
        for (told, shows) in cases {
            let mut merges = Merges::default();
            merges.set(vec![copy("a")]);
            merges.hide();
            merges.set(told.clone());
            assert_eq!(merges.height() > 0.0, shows, "{told:?}");
        }
    }

    #[test]
    fn a_yes_on_the_card_asks_for_what_the_card_was_about() {
        // What the shell asked about, and what a yes asks for.
        let cases = [
            (MergeAsk { files: vec![copy("a"), copy("b")], conflicts: true }, Some(Confirmed::Conflicts)),
            (MergeAsk { files: vec![copy("a")], conflicts: false }, Some(Confirmed::File("/home/sam/a".to_owned()))),
            (MergeAsk { files: vec![], conflicts: false }, None),
        ];
        for (ask, confirmed) in cases {
            let mut merges = Merges::default();
            merges.ask(ask.clone());
            assert_eq!(merges.confirm(), confirmed, "{ask:?}");
            assert_eq!(merges.confirm(), None, "the card is gone");
        }
        let mut merges = Merges::default();
        merges.ask_into(copy("original"));
        assert_eq!(merges.confirm(), Some(Confirmed::Original));
    }
}
