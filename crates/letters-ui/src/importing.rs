//! Importing a file that is not a Letters document: one card over the page that says what the file brings
//! and lets the person choose what comes with it.
//!
//! The window only asks: the shell reads the file, offers it, and imports with what was chosen.

use accesskit::Role;
use letters_render::{Rect, Size};

use crate::element::Element;
use crate::icon;
use crate::opening::Request;
use crate::style::{Align, Dir, Edges, Justify, Look};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, Popover};
use crate::window::Message;

/// As wide as the landing card, which it stands in for.
const CARD_WIDTH: f32 = 544.0;
const ROW_HEIGHT: f32 = 38.0;
const TICK: f32 = 16.0;

/// A file the shell was asked to open that has to be imported, and what it found in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportOffer {
    /// The file's name, as the card shows it: `Trip notes.md`.
    pub name: String,
    pub kind: ImportKind,
    /// The folders the pictures it names are in. None when it brings its own, as a Word file does.
    pub folders: Vec<ImportFolder>,
    /// How many pictures it names that are not there. They stay links.
    pub missing: u32,
    /// How many comments it holds. The card offers to bring them when there are any.
    pub comments: u32,
    /// The folder the new document will be kept in, as the card shows it: `Family / House`.
    pub destination: String,
}

/// What kind of file is imported.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImportKind {
    #[default]
    Markdown,
    Word,
}

/// A folder an imported file takes pictures from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportFolder {
    /// The shell's own name for it, which it gets back unchanged in [`Request::Import`].
    pub path: String,
    /// The folder as the card shows it: `Notes / Trip`.
    pub shown: String,
    /// How many of the file's pictures are in it.
    pub pictures: u32,
    /// Whether the card starts with it ticked. A file can name any picture on the machine, so the shell
    /// ticks only the folders beside the file.
    pub chosen: bool,
}

/// What the import card sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Import {
    /// Ticks or clears the folder at this place in the offer.
    Folder(usize),
    /// Ticks or clears bringing the comments.
    Comments,
    /// Asks the shell to import with what is ticked.
    Apply,
    /// Puts the card away and tells the shell the file is not wanted.
    Cancel,
}

/// The offer on the card, and what the person has ticked.
#[derive(Clone, Debug)]
pub(crate) struct Importing {
    offer: ImportOffer,
    allowed: Vec<bool>,
    comments: bool,
    /// Whether the shell was asked to import, and has not yet answered by taking the card away.
    asked: bool,
}

impl Importing {
    pub fn new(offer: ImportOffer) -> Self {
        let allowed = offer.folders.iter().map(|folder| folder.chosen).collect();
        Self { comments: offer.comments > 0, offer, allowed, asked: false }
    }

    /// Takes a message from the card. Returns whether the card stays.
    pub fn update(&mut self, message: Import, requests: &mut Vec<Request>) -> bool {
        match message {
            Import::Cancel => {
                requests.push(Request::CancelImport);
                return false;
            }
            // The card waits for the shell as it is.
            _ if self.asked => {}
            Import::Folder(at) => _ = self.allowed.get_mut(at).map(|allowed| *allowed = !*allowed),
            Import::Comments => self.comments = !self.comments,
            Import::Apply => {
                let ticked = self.offer.folders.iter().zip(&self.allowed).filter(|(_, allowed)| **allowed);
                let folders = ticked.map(|(folder, _)| folder.path.clone()).collect();
                requests.push(Request::Import { folders, comments: self.comments });
                self.asked = true;
            }
        }
        true
    }

    /// The card, where the landing card stands: a little above the middle of the page area.
    pub fn card(&self, room: Size) -> Popover<Message> {
        let send = |message: Import| Message::Import(message);
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let line = || Element::row().height(1.0).bg(tokens::LINE).decorative();
        let offer = &self.offer;
        let kind = match offer.kind {
            ImportKind::Markdown => "Markdown file",
            ImportKind::Word => "Word document",
        };

        let named = Element::column()
            .shrink()
            .child(Element::text(format!("Import {}", offer.name), TextStyle::control().weight(600)).shrink())
            .child(Element::text(kind, quiet).fg(tokens::FG_MUTED).shrink());
        let head = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .child(Element::icon(icon::FILE_DOWN, 20.0).fg(tokens::FG_MUTED))
            .child(named);
        let mut card = Element::column().gap(tokens::SPACE_SM).width(CARD_WIDTH).child(head);
        let mut rows = 0;

        if !offer.folders.is_empty() {
            let folder = |(at, folder): (usize, &ImportFolder)| {
                let brings = counted(folder.pictures, "picture");
                self.choice(folder.shown.as_str(), self.allowed[at], send(Import::Folder(at)))
                    .key(folder.path.as_str())
                    .description(brings.as_str())
                    .child(Element::icon(icon::FOLDER, 16.0).fg(tokens::FG_MUTED))
                    .child(Element::text(folder.shown.as_str(), TextStyle::control().weight(500)).shrink())
                    .child(Element::row().grow(1.0))
                    .child(Element::text(brings.as_str(), quiet).fg(tokens::FG_MUTED))
            };
            let title = Element::text("Bring pictures from", quiet.weight(600));
            let list = Element::column()
                .role(Role::Group)
                .label("Folders to bring pictures from")
                .arrows(Dir::Column)
                .children(offer.folders.iter().enumerate().map(folder));
            rows += offer.folders.len();
            card = card
                .child(line())
                .child(title.fg(tokens::FG_MUTED).padding(Edges::xy(tokens::SPACE_SM, 0.0)))
                .child(list);
        }
        if offer.comments > 0 {
            let label = format!("Bring its {}", counted(offer.comments, "comment"));
            let comments = self
                .choice(&label, self.comments, send(Import::Comments))
                .child(Element::icon(icon::MESSAGE_SQUARE, 16.0).fg(tokens::FG_MUTED))
                .child(Element::text(label.as_str(), TextStyle::control().weight(500)).shrink());
            rows += 1;
            card = card.child(line()).child(comments);
        }
        // What happens whatever is ticked: where the document goes, and what cannot come.
        let mut notes = vec![match offer.destination.as_str() {
            "" => "The original file is left as it is.".to_owned(),
            folder => format!("Kept as a new document in {folder}. The original file is left as it is."),
        }];
        match offer.missing {
            0 => {}
            1 => notes.push("1 picture it names was not found and stays a link.".to_owned()),
            missing => notes.push(format!("{missing} pictures it names were not found and stay links.")),
        }
        let note = |note: &String| {
            Element::row()
                .gap(tokens::SPACE_SM)
                .role(Role::Note)
                .label(note.as_str())
                .child(Element::icon(icon::INFO, 14.0).fg(tokens::FG_MUTED))
                .child(Element::paragraph(note.as_str(), quiet).fg(tokens::FG_MUTED).grow(1.0).shrink())
        };
        let said = Element::column()
            .gap(tokens::SPACE_XS)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .children(notes.iter().map(note));
        card = card.child(line()).child(said);

        let apply = Button::new(if self.asked { "Importing" } else { "Import" })
            .icon(icon::FILE_DOWN)
            .primary()
            .disabled(self.asked)
            .on_press(send(Import::Apply));
        // Enter imports what the shell suggested, without a walk through the card first.
        let foot = Element::row()
            .align(Align::Center)
            .justify(Justify::End)
            .gap(tokens::SPACE_SM)
            .child(Button::new("Cancel").on_press(send(Import::Cancel)))
            .child(Element::from(apply).key("import").takes_focus());

        let height = rows as f32 * ROW_HEIGHT + notes.len() as f32 * 24.0 + 140.0;
        let spot = Rect::new(room.width / 2.0, ((room.height - height) * 0.4).max(0.0), 0.0, 0.0);
        Popover::new(card.child(line()).child(foot))
            .label(format!("Import {}", offer.name))
            .max_width(CARD_WIDTH + 2.0 * tokens::SPACE_MD)
            .at(spot)
            .align(Align::Center)
            .gap(0.0)
            .shows_focus()
            // A stray press on the page must not lose the file: Escape and Cancel put the card away.
            .stays()
            .open(true)
            .on_close(send(Import::Cancel))
    }

    /// A row that is ticked or not, with its box. What it is about is added by the caller.
    fn choice(&self, label: &str, on: bool, message: Message) -> Element<Message> {
        let tick = Element::row().width(TICK).height(TICK).radius(4.0).align(Align::Center).justify(Justify::Center);
        let tick = match on {
            true => tick.bg(tokens::ACCENT_SOLID).child(Element::icon(icon::CHECK, 12.0).fg(tokens::ON_SOLID)),
            false => tick.border(Edges::all(1.0), tokens::LINE_STRONG),
        };
        Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .height(ROW_HEIGHT)
            .padding(Edges::xy(tokens::SPACE_SM, 0.0))
            .radius(tokens::RADIUS_CONTROL)
            .hover(Look::bg(tokens::NEUTRAL_SOFT))
            .focused(Look::bg(tokens::NEUTRAL_SOFT))
            .role(Role::CheckBox)
            .label(label)
            .checked(on)
            .focusable()
            .disabled(self.asked)
            .on_press(message)
            .child(tick.decorative())
    }
}

/// A count with its noun: "1 picture", "3 pictures".
fn counted(count: u32, one: &str) -> String {
    format!("{count} {one}{}", if count == 1 { "" } else { "s" })
}
