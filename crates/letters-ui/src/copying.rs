//! Saving a copy of a document, or writing it in another format: one card over the page asks for the
//! copy's name, its format and its folder. Opened while the history shows an earlier version, the card
//! asks for a copy of that version. Under the formats it lists what the picked one will simplify, before
//! anything is written.
//!
//! The window only asks: the shell writes the copy, and answers with where it went or with why it could not.

use accesskit::Role;
use letters_convert::{Loss, Target, losses_with};
use letters_model::{Document, Notes, Revision};
use letters_render::{Rect, Size};

use crate::element::Element;
use crate::folders::{self, Browse, Tree};
use crate::opening::{Location, Request};
use crate::style::{Align, Dir, Edges};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, Popover, TextField};
use crate::window::Message;

const CARD_WIDTH: f32 = 544.0;
/// The most rows the tree shows before it scrolls.
const ROWS: f32 = 6.5;
/// What the list of a format's losses is called.
const LOSSES: &str = "Simplified in this copy";
/// The height of a line of the losses, for placing the card.
const LOSS_HEIGHT: f32 = 18.0;

/// What a copy of a document can be written as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CopyFormat {
    /// Another document, with this one's history.
    #[default]
    Letters,
    /// Another document with this one's text and pictures and nothing of how it came to be: no history
    /// and no notes. A file that ends as a Letters document does.
    Clean,
    Word,
    Pdf,
    /// One HTML file.
    WebPage,
    Markdown,
}

impl CopyFormat {
    pub const ALL: [Self; 6] = [Self::Letters, Self::Clean, Self::Word, Self::Pdf, Self::WebPage, Self::Markdown];

    /// The format's name, as the card offers it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Letters => "Letters",
            Self::Clean => "Clean copy",
            Self::Word => "Word",
            Self::Pdf => "PDF",
            Self::WebPage => "Web page",
            Self::Markdown => "Markdown",
        }
    }

    /// What the card says of a copy in this format. `earlier` is for a copy of an earlier version.
    fn about(self, earlier: bool) -> &'static str {
        match self {
            Self::Letters if earlier => "A document of its own that starts with this version\u{2019}s text.",
            Self::Letters => "A document of its own that starts with this one\u{2019}s text and history.",
            Self::Clean => "A document of its own that holds this one\u{2019}s text and nothing of how it came to be.",
            Self::Word => "A .docx file for Word and other editors.",
            Self::Pdf => "A .pdf file, set in pages as it would print.",
            Self::WebPage => "One .html file that holds its own pictures.",
            Self::Markdown => "A .md file. Its pictures are written beside it.",
        }
    }

    /// What the format is to the converters, which say what it will simplify. A copy with the history
    /// simplifies nothing.
    fn target(self) -> Option<Target> {
        match self {
            Self::Letters => None,
            Self::Clean => Some(Target::Clean),
            Self::Word => Some(Target::Docx),
            Self::Pdf => Some(Target::Pdf),
            Self::WebPage => Some(Target::Html),
            Self::Markdown => Some(Target::Markdown),
        }
    }
}

/// What the card sends.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Copying {
    /// Shows the card, set to a format.
    Open(CopyFormat),
    /// What the name field holds.
    Name(String),
    Format(CopyFormat),
    /// What is done among the folders: one is picked, opened or closed, or a new one asked for.
    Browse(Browse),
    /// Asks the shell for the copy, in the folder that is picked.
    Save,
    /// Asks the shell for the copy wherever the person then chooses.
    Elsewhere,
    Close,
}

/// A version the history shows that is not the document as it reads now, for a copy of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Earlier {
    pub revision: Revision,
    /// What the copy's name says it is a copy of: the version's name, or the day it was written.
    pub called: String,
}

/// How a message left the card.
pub(crate) enum Outcome {
    Open,
    /// Put away, with the copy asked of the shell, which may refuse.
    Asked,
    Closed,
}

/// The card while it shows: the copy's name and format, and the folder picked for it.
#[derive(Clone, Debug)]
pub(crate) struct Copy {
    name: String,
    /// Whether the name is the person's own. One the card suggested follows the format.
    typed: bool,
    format: CopyFormat,
    tree: Tree,
    /// Why the shell could not write the copy.
    notice: Option<String>,
    /// Whether the shell refused because a file is in the way, which the person may have it replace.
    replaceable: bool,
    /// The version the copy is of, where that is not the document as it reads now.
    version: Option<Earlier>,
    /// What the format will simplify, a line for each kind of loss.
    simplified: Vec<String>,
}

impl Copy {
    /// Starts with a name for the copy and the folder the document is in. `version` is the version to copy,
    /// for one the history shows that is not the document as it reads now.
    pub fn open(format: CopyFormat, location: &Location, title: &str, version: Option<Earlier>) -> Self {
        let mut tree = Tree::open(location);
        tree.picked = Some(location.folder.clone()).filter(|folder| !folder.is_empty());
        let name = suggested(format, title, version.as_ref());
        Self { name, typed: false, format, tree, notice: None, replaceable: false, version, simplified: Vec::new() }
    }

    /// The revision the copy is of, where that is not the document as it reads now.
    pub fn version(&self) -> Option<&Revision> {
        self.version.as_ref().map(|version| &version.revision)
    }

    /// Takes in what the copy is made of, to list what its format will simplify: the document, and the
    /// notes that are copied with it. `shown` says whether the window shows the picture linked from an
    /// address: a shell hands a PDF the linked pictures that show, so they are no loss there.
    pub fn weigh(&mut self, document: &Document, notes: &Notes, shown: &dyn Fn(&str) -> bool) {
        let lost = self.format.target().map(|to| losses_with(document, notes, to, shown)).unwrap_or_default();
        self.simplified = lost.iter().map(worded).collect();
    }

    /// Whether the copy is of an earlier version, which only the history has to show.
    pub fn earlier(&self) -> bool {
        self.version.is_some()
    }

    /// Whether the card offers a format. A copy of an earlier version is a clean one as it is.
    fn offers(&self, format: CopyFormat) -> bool {
        !self.earlier() || format != CopyFormat::Clean
    }

    /// What the card is called, and says over its name field.
    fn title(&self) -> &'static str {
        if self.earlier() { "Save a copy of this version" } else { "Save a copy" }
    }

    pub fn format(&self) -> CopyFormat {
        self.format
    }

    /// Comes back with why the shell could not write the copy. `replaceable` offers to replace the file
    /// that was in the way.
    pub fn refused(&mut self, notice: &str, replaceable: bool) {
        (self.notice, self.replaceable) = (Some(notice.to_owned()), replaceable);
    }

    pub fn update(
        &mut self,
        message: Copying,
        location: &Location,
        title: &str,
        requests: &mut Vec<Request>,
    ) -> Outcome {
        // Whatever changes the copy leaves the last refusal behind.
        if !matches!(message, Copying::Save | Copying::Browse(Browse::Unfold(..) | Browse::Folder(_))) {
            (self.notice, self.replaceable) = (None, false);
        }
        match message {
            Copying::Open(_) => {}
            Copying::Name(name) => (self.name, self.typed) = (name, true),
            Copying::Format(format) => {
                self.format = format;
                if !self.typed {
                    self.name = suggested(format, title, self.version.as_ref());
                }
            }
            Copying::Browse(message) => self.tree.update(message, location, requests),
            Copying::Save | Copying::Elsewhere => {
                let name = self.name.trim().to_owned();
                if name.is_empty() {
                    return Outcome::Open;
                }
                let folder = self.tree.picked.clone().filter(|_| message == Copying::Save);
                let replace = self.replaceable && message == Copying::Save;
                let format = self.format;
                requests.push(match self.version.clone() {
                    Some(Earlier { revision, .. }) => Request::SaveVersion { revision, name, format, folder, replace },
                    None => Request::SaveCopy { name, format, folder, replace },
                });
                (self.notice, self.replaceable) = (None, false);
                return Outcome::Asked;
            }
            Copying::Close => return Outcome::Closed,
        }
        Outcome::Open
    }

    /// Takes in what the shell now says of the folders.
    pub fn placed(&mut self, location: &Location) {
        self.tree.placed(location);
    }

    /// The card, over the middle of the page area.
    pub fn card(&self, location: &Location, room: Size) -> Popover<Message> {
        let send = |message: Copying| Message::Copy(message);
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let line = || Element::row().height(1.0).bg(tokens::LINE).decorative();
        let inset = Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS);
        let section = Edges { bottom: 0.0, ..Edges::all(tokens::SPACE_MD) };

        let name = TextField::new("Name of the copy", &self.name)
            .weight(600)
            .invalid(self.notice.is_some())
            .on_input(|name| Message::Copy(Copying::Name(name)))
            .on_submit(send(Copying::Save));
        let mut head = Element::column()
            .gap(tokens::SPACE_SM)
            .padding(Edges::all(tokens::SPACE_MD))
            .child(Element::text(self.title(), TextStyle::title()))
            .child(Element::from(name).key("copying").takes_focus());
        if let Some(notice) = &self.notice {
            head = head.child(Element::paragraph(notice, quiet).fg(tokens::DANGER).role(Role::Alert).label(notice));
        }

        let format = |format: CopyFormat| {
            let button =
                Button::new(format.name()).pressed(format == self.format).on_press(send(Copying::Format(format)));
            Element::from(button).grow(1.0)
        };
        let formats = Element::row()
            .gap(tokens::SPACE_XS)
            .role(Role::Group)
            .label("Format")
            .arrows(Dir::Row)
            .one_stop()
            .children(CopyFormat::ALL.into_iter().filter(|&offered| self.offers(offered)).map(format));
        let formats = Element::column()
            .gap(tokens::SPACE_SM)
            .padding(Edges { top: 0.0, ..Edges::all(tokens::SPACE_MD) })
            .child(formats)
            .child(Element::paragraph(self.format.about(self.earlier()), quiet).fg(tokens::FG_MUTED))
            .children(self.losses());

        let mut card = Element::column().width(CARD_WIDTH).child(head).child(formats);
        let mut rows = 0.0;
        if !location.folders.is_empty() {
            let title = Element::text(folders::title(location), quiet.weight(600)).fg(tokens::FG_MUTED);
            let tree = self.tree.list(location, "Folder", ROWS, browse);
            rows = (location.folders.len() as f32).min(ROWS) + 2.0;
            card = card
                .child(line())
                .child(Element::column().padding(section).child(title))
                .child(Element::column().padding(inset).child(tree))
                .child(line())
                .child(Element::column().padding(inset).child(folders::elsewhere(send(Copying::Elsewhere))));
        }
        card = card.child(line()).child(self.foot(location));

        // The card sits a little above the middle of the page area, as the landing card does.
        let lost = match self.simplified.len() {
            0 => 0.0,
            lines => (lines + 1) as f32 * LOSS_HEIGHT + tokens::SPACE_SM,
        };
        let height = 230.0 + lost + rows * folders::ROW_HEIGHT;
        let spot = Rect::new(room.width / 2.0, ((room.height - height) * 0.4).max(0.0), 0.0, 0.0);
        Popover::new(card)
            .label(self.title())
            .flush()
            .max_width(CARD_WIDTH + 2.0)
            .at(spot)
            .align(Align::Center)
            .gap(0.0)
            .open(true)
            .on_close(send(Copying::Close))
    }

    /// What the format will simplify, a line for each kind of loss. Nothing for a format that carries all
    /// the document holds.
    fn losses(&self) -> Option<Element<Message>> {
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let lines = self.simplified.iter().map(|line| {
            Element::row()
                .gap(tokens::SPACE_SM)
                .role(Role::ListItem)
                .label(line.as_str())
                .child(Element::text("\u{2022}", quiet).fg(tokens::FG_MUTED))
                .child(Element::paragraph(line.as_str(), quiet).grow(1.0))
        });
        let list = Element::column().role(Role::List).label(LOSSES).children(lines);
        let title = Element::text(LOSSES, quiet.weight(600)).fg(tokens::FG_MUTED);
        (!self.simplified.is_empty()).then(|| Element::column().child(title).child(list))
    }

    /// The card's last row: a new folder, and the copy itself.
    fn foot(&self, location: &Location) -> Element<Message> {
        let copy = self.format == CopyFormat::Letters;
        let said = match (self.replaceable, self.tree.picked_name(location)) {
            (true, _) => "Replace it".to_owned(),
            (_, Some(folder)) if copy => format!("Save copy in {folder}"),
            (_, Some(folder)) => format!("Export to {folder}"),
            (_, None) if copy => "Save copy".to_owned(),
            (_, None) => "Export".to_owned(),
        };
        let save = Button::new(said)
            .primary()
            .small()
            .disabled(self.name.trim().is_empty())
            .on_press(Message::Copy(Copying::Save));
        let mut foot = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .padding(Edges::xy(tokens::SPACE_SM, tokens::SPACE_SM));
        if !location.folders.is_empty() {
            foot = foot.child(self.tree.new_folder(location, browse));
        }
        foot.child(Element::row().grow(1.0)).child(save)
    }
}

/// The name a copy starts with. Another document cannot share its folder with this one under one name,
/// and a copy of an earlier version says which it is of.
fn suggested(format: CopyFormat, title: &str, version: Option<&Earlier>) -> String {
    match version {
        Some(version) => format!("{title}, {}", version.called),
        None if format == CopyFormat::Letters => format!("{title} copy"),
        None if format == CopyFormat::Clean => format!("{title} clean"),
        None => title.to_owned(),
    }
}

/// A loss as a line of the card, which starts with a capital as the card's other lines do.
fn worded(loss: &Loss) -> String {
    let said = loss.to_string();
    let mut letters = said.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => said,
    }
}

fn browse(message: Browse) -> Message {
    Message::Copy(Copying::Browse(message))
}
