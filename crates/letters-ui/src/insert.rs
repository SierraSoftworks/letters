//! The insert menu: what a `/` typed on an empty line offers to put there.
//!
//! The menu hangs under the slash while the caret stays on the page. What is typed after the slash narrows
//! it, and each offer shows the Markdown shorthand that makes the same thing without the menu.

use letters_editor::Editor;
use letters_model::{BlockId, BlockKind, ListKind, Tone};

use crate::element::Element;
use crate::icon::{self, Icon};
use crate::style::Edges;
use crate::text::{Family, TextStyle};
use crate::tokens;
use crate::widgets::{Choice, Options, Popover};
use crate::window::Message;

/// How wide the menu is.
const WIDTH: f32 = 288.0;

/// What the pointer and the keyboard ask of the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Inserting {
    /// An offer was pressed, counted from the first that shows.
    Take(usize),
    /// Escape or a press elsewhere, which leaves the line as it was typed.
    Close,
}

/// What taking an offer does to its line.
#[derive(Clone)]
pub(crate) enum Put {
    /// Makes the line a block of this kind.
    Kind(BlockKind),
    /// Puts a table in the line's place.
    Table,
    /// Asks for pictures to put in the line's place.
    Pictures,
}

/// Something the menu puts on the line.
pub(crate) struct Offer {
    pub name: &'static str,
    group: &'static str,
    icon: Icon,
    /// What typed at the start of a line makes the same thing.
    shorthand: &'static str,
    pub put: Put,
}

const fn offer(group: &'static str, name: &'static str, icon: Icon, shorthand: &'static str, put: Put) -> Offer {
    Offer { name, group, icon, shorthand, put }
}

const fn kind(kind: BlockKind) -> Put {
    Put::Kind(kind)
}

const fn list(list: ListKind) -> Put {
    Put::Kind(BlockKind::ListItem { list, depth: 0 })
}

pub(crate) static OFFERS: [Offer; 13] = [
    offer("Text", "Heading 1", icon::HEADING_1, "#", kind(BlockKind::Heading { level: 1 })),
    offer("Text", "Heading 2", icon::HEADING_2, "##", kind(BlockKind::Heading { level: 2 })),
    offer("Text", "Heading 3", icon::HEADING_3, "###", kind(BlockKind::Heading { level: 3 })),
    offer("Text", "Bulleted list", icon::LIST, "-", list(ListKind::Bullet)),
    offer("Text", "Numbered list", icon::LIST_ORDERED, "1.", list(ListKind::Ordered)),
    offer("Text", "Task list", icon::LIST_CHECKS, "[ ]", list(ListKind::Task { checked: false })),
    offer("Text", "Quote", icon::TEXT_QUOTE, ">", kind(BlockKind::Quote)),
    offer("Text", "Code block", icon::CODE_XML, "```", kind(BlockKind::Code { language: None })),
    offer("Elements", "Table", icon::TABLE, "| a | b |", Put::Table),
    offer("Elements", "Image", icon::IMAGE, "![]()", Put::Pictures),
    offer("Elements", "Callout", icon::INFO, "> [!INFO]", kind(BlockKind::Callout { tone: Tone::Info })),
    offer("Elements", "Formula", icon::SIGMA, "$$", kind(BlockKind::Formula)),
    offer("Elements", "Divider", icon::SEPARATOR_HORIZONTAL, "---", kind(BlockKind::Divider)),
];

/// The offers that match what is typed after the slash: those with a word that begins with it, in the
/// menu's order. All of them when nothing is typed yet.
pub(crate) fn matching(typed: &str) -> Vec<&'static Offer> {
    let typed = typed.to_lowercase();
    let begins = |word: &str| word.to_lowercase().starts_with(&typed);
    OFFERS.iter().filter(|offer| begins(offer.name) || offer.name.split(' ').any(begins)).collect()
}

/// The menu's own state: the line it is open on, and which offer is picked.
#[derive(Default)]
pub(crate) struct Insert {
    /// The line whose slash opened the menu.
    line: Option<BlockId>,
    picked: usize,
}

impl Insert {
    /// The caret's line and what it holds, when that is a slash and what was typed after it with the caret
    /// at its end: the only writing the menu stays open over.
    fn slashed(editor: &Editor) -> Option<(&BlockId, String)> {
        let caret = editor.selection().filter(|selection| selection.is_caret())?;
        let line = editor.doc().block(&caret.head.block).filter(|line| line.kind == BlockKind::Paragraph)?;
        let text = line.plain_text();
        let worded = text.starts_with('/') && !text.contains(char::is_whitespace) && caret.head.offset == text.len();
        worded.then_some((&line.id, text))
    }

    /// Opens the menu when a slash has just been typed onto an empty line.
    pub fn typed(&mut self, editor: &Editor) {
        if let Some((line, _)) = Self::slashed(editor).filter(|(_, text)| text == "/") {
            (self.line, self.picked) = (Some(line.clone()), 0);
        }
    }

    /// Puts the menu away once its line no longer reads as a slash and a word, or the caret has left it.
    pub fn follow(&mut self, editor: &Editor) {
        let open = Self::slashed(editor).is_some_and(|(line, _)| Some(line) == self.line.as_ref());
        if !open {
            self.close();
        }
    }

    pub fn close(&mut self) {
        (self.line, self.picked) = (None, 0);
    }

    /// The offers the menu shows, and how much of its line was typed to find them. `None` while it is put
    /// away or nothing matches.
    pub fn offers(&self, editor: &Editor) -> Option<(BlockId, usize, Vec<&'static Offer>)> {
        let (line, text) = Self::slashed(editor).filter(|(line, _)| Some(*line) == self.line.as_ref())?;
        let offers = matching(&text[1..]);
        (!offers.is_empty()).then(|| (line.clone(), text.len(), offers))
    }

    /// Moves the pick one offer down or up a menu of `shown`, round its ends.
    pub fn pick(&mut self, down: bool, shown: usize) {
        self.picked = Options::<Message>::step(self.picked, down, shown);
    }

    /// The offer that is picked among `shown`.
    pub fn picked(&self, shown: usize) -> usize {
        self.picked.min(shown.saturating_sub(1))
    }

    /// The menu over `offers`, to hang under the slash.
    pub fn menu(&self, offers: &[&'static Offer]) -> Popover<Message> {
        let shorthand = TextStyle::control().family(Family::Mono).size(tokens::TEXT_XS);
        let mut options = Options::new("Insert").picked(self.picked(offers.len()));
        let mut group = "";
        for (index, offer) in offers.iter().enumerate() {
            if offer.group != group {
                group = offer.group;
                options = options.heading(group);
            }
            let choice = Choice::new(offer.name)
                .lead(Element::icon(offer.icon, 16.0).fg(tokens::FG_MUTED))
                .aside(offer.shorthand, shorthand)
                .on_take(Message::Insert(Inserting::Take(index)));
            options = options.choice(choice);
        }
        let list = Element::from(options).width(WIDTH).padding(Edges::all(tokens::SPACE_XS));
        Popover::new(list).flush().beside().open(true).on_close(Message::Insert(Inserting::Close))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_typed_after_the_slash_narrows_the_offers_by_the_words_of_their_names() {
        let names = |typed: &str| matching(typed).into_iter().map(|offer| offer.name).collect::<Vec<_>>();
        assert_eq!(names("").len(), OFFERS.len(), "a slash alone offers everything");
        // What is typed, and the offers left.
        let cases: [(&str, &[&str]); 5] = [
            ("head", &["Heading 1", "Heading 2", "Heading 3"]),
            ("LIST", &["Bulleted list", "Numbered list", "Task list"]),
            ("block", &["Code block"]),
            ("q", &["Quote"]),
            ("zz", &[]),
        ];
        for (typed, left) in cases {
            assert_eq!(names(typed), left, "{typed}");
        }
    }
}
