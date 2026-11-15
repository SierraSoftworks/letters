//! The language a document is written in, and a passage of it that is written in another.
//!
//! A dialog shows both and has the editor say them. It offers what the settings offer: the kinds of English
//! that are built in, and the languages the shell has dictionaries for.

use letters_editor::Lang;

use crate::element::Element;
use crate::icon;
use crate::settings::{self, ENGLISH, NAMES, language_name, quiet, row};
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, Menu, MenuItem, Popover};
use crate::window::Message;

const WIDTH: f32 = 520.0;
/// What a passage's list calls leaving it to the document.
const SAME: &str = "Same as the document";

/// What the dialog sends.
#[derive(Clone, Debug, PartialEq)]
pub enum Writing {
    Show,
    Close,
    /// Says what the document is written in, as a BCP 47 tag.
    Document(String),
    /// Says what the text at the caret or under the selection is written in, or leaves it to the document
    /// again.
    Passage(Option<String>),
}

/// What the dialog shows of the document.
pub(crate) struct Told<'a> {
    /// The document's language, as it is written there.
    pub document: &'a str,
    /// The language in force at the caret or over the selection. `None` where nothing there can have one:
    /// code, a formula, a picture.
    pub passage: Option<Lang>,
    /// Whether text is selected, so that the passage is that text and not the caret's paragraph.
    pub selected: bool,
    /// The languages the shell has dictionaries for.
    pub shell: &'a [String],
}

/// The dialog over the middle of the window. It hangs from an element of no size, as the settings do.
pub(crate) fn view(told: &Told) -> Element<Message> {
    let send = |writing: Writing| Message::Written(writing);
    let head = Element::row()
        .align(Align::Center)
        .justify(Justify::Between)
        .child(Element::text("Language", TextStyle::title().size(tokens::TEXT_XL)))
        .child(Button::icon_only(icon::X, "Close").subtle().small().on_press(send(Writing::Close)));
    let width = WIDTH - 2.0 * tokens::SPACE_XL - NAMES - tokens::SPACE_LG;
    // A document that names none reads as new documents do.
    let document = Some(told.document).filter(|tag| !tag.is_empty()).unwrap_or(ENGLISH[0]);
    let own = match &told.passage {
        Some(Lang::Tag(tag)) => Some(tag.as_str()),
        _ => None,
    };
    let used: Vec<&str> = [document].into_iter().chain(own).collect();
    let offered = settings::offered(told.shell, &used);

    let mut menu = Menu::new();
    for tag in &offered {
        let item = MenuItem::new(language_name(tag)).checked(tag.eq_ignore_ascii_case(document));
        menu = menu.item(item.on_select(send(Writing::Document((*tag).to_owned()))));
    }
    let whole = settings::select("Language of the document", language_name(document), menu, width);
    let whole = row("Document", whole, Some("The document is spell-checked in it."));

    let name = if told.selected { "Selected text" } else { "This paragraph" };
    let part = match &told.passage {
        Some(lang) => {
            let same = MenuItem::new(SAME).checked(*lang == Lang::Document).on_select(send(Writing::Passage(None)));
            let mut menu = Menu::new().item(same).separator();
            for tag in &offered {
                let item =
                    MenuItem::new(language_name(tag)).checked(own.is_some_and(|own| own.eq_ignore_ascii_case(tag)));
                menu = menu.item(item.on_select(send(Writing::Passage(Some((*tag).to_owned())))));
            }
            let shown = match lang {
                Lang::Document => SAME.to_owned(),
                Lang::Tag(tag) => language_name(tag),
                Lang::Mixed => "More than one".to_owned(),
            };
            let label = format!("Language of {}", name.to_lowercase());
            let about = "For a passage that is written in another language than the document.";
            row(name, settings::select(&label, shown, menu, width), Some(about))
        }
        None => {
            let none = Element::text("Code, formulas and pictures have no language.", quiet()).fg(tokens::FG_MUTED);
            row(name, Element::row().align(Align::Center).height(tokens::CONTROL_HEIGHT).child(none), None)
        }
    };

    let content = Element::column()
        .width(WIDTH)
        .gap(tokens::SPACE_LG)
        .padding(Edges { top: tokens::SPACE_LG, ..Edges::all(tokens::SPACE_XL) })
        .child(head)
        .child(Element::column().gap(tokens::SPACE_MD).child(whole).child(part));
    let dialog = Popover::new(content)
        .flush()
        .label("Language")
        .max_width(WIDTH)
        .modal()
        .open(true)
        .on_close(send(Writing::Close));
    Element::row().overlay().popover(dialog)
}
