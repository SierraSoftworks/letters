//! The fonts a type can be set in: the bundled families, those the shell says it can find, and the list
//! that finds one by name.
//!
//! A style keeps a family by its name alone. The shell finds its faces when text asks for them (see
//! `Window::take_named`), so the list only names what there is.

use std::cell::RefCell;

use letters_layout::Fonts;
use letters_render::Font;

use crate::element::Element;
use crate::style::Edges;
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Choice, Options, Popover, TextField, options};
use crate::window::{FAMILIES, Message};

/// How wide the list is.
const CARD: f32 = 288.0;
/// How many families the list shows before it scrolls.
const ROWS: usize = 8;
/// How many families the list holds at most. A library has thousands, and a name finds the rest.
const LISTED: usize = 48;

thread_local! {
    /// The faces shells have found for families that styles name. They are the same for every window, so
    /// one that opens later sets its text in them from the start.
    static NAMED: RefCell<Vec<Font>> = const { RefCell::new(Vec::new()) };
}

/// The fonts a document is laid out in: the bundled faces, then those found for the families styles name.
pub(crate) fn fonts() -> Fonts {
    let mut faces = letters_fonts::bundled();
    NAMED.with_borrow(|named| faces.extend(named.iter().cloned()));
    Fonts { faces }
}

/// Keeps faces a shell found for whatever is laid out later.
pub(crate) fn keep(faces: &[Font]) {
    NAMED.with_borrow_mut(|named| {
        for face in faces {
            if !named.iter().any(|known| known.index == face.index && known.data.same(&face.data)) {
                named.push(face.clone());
            }
        }
    });
}

/// Where a family's faces come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontSource {
    /// A font installed where the window runs. Another reader's device may lack it.
    Device,
    /// The Google Fonts library, whose faces any reader can fetch.
    Library,
}

/// A family the shell can find faces for, beside the bundled ones. See `Window::set_families`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FontOffer {
    pub family: String,
    pub source: FontSource,
}

/// What the pointer and the keyboard ask of the list of fonts.
#[derive(Clone, Debug, PartialEq)]
pub enum Fonting {
    /// Lists the fonts the caret's type can be set in.
    Open,
    /// What the field over the list holds.
    Type(String),
    /// Moves the pick one family down the list, or up, round its ends.
    Step { down: bool },
    /// A family was pressed, counted from the first that shows.
    Take(usize),
    /// Enter, which takes the family that is picked.
    Enter,
    /// Escape or a press elsewhere.
    Close,
}

/// A row of the list: a family, and where it is from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub family: String,
    pub from: &'static str,
}

/// The fonts on offer, and the list that finds one.
#[derive(Default)]
pub(crate) struct Families {
    /// What the shell can find, by name.
    offers: Vec<FontOffer>,
    open: bool,
    /// What is typed to find one.
    typed: String,
    picked: usize,
}

impl Families {
    /// Takes the families the shell can find. One it names twice is offered as the first says, and one of
    /// the bundled families is left out.
    pub fn set(&mut self, offers: Vec<FontOffer>) {
        let mut seen = std::collections::HashSet::new();
        let bundled = |family: &str| FAMILIES.iter().any(|(own, _)| own.eq_ignore_ascii_case(family));
        self.offers = offers;
        self.offers.retain(|offer| !bundled(&offer.family) && seen.insert(offer.family.to_lowercase()));
        self.offers.sort_by_cached_key(|offer| offer.family.to_lowercase());
    }

    /// Whether there is more to choose from than the bundled families.
    pub fn any(&self) -> bool {
        !self.offers.is_empty()
    }

    pub fn open(&mut self) {
        (self.open, self.picked) = (true, 0);
        self.typed.clear();
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Takes what the field holds now. The nearest family it finds is the one picked.
    pub fn typed(&mut self, typed: String) {
        (self.typed, self.picked) = (typed, 0);
    }

    /// Moves the pick one family down or up the list, round its ends.
    pub fn step(&mut self, down: bool, used: &[&str]) {
        self.picked = Options::<Message>::step(self.picked, down, self.rows(used).0.len());
    }

    /// Puts the list away. Returns the family at `index` of those that show, or the one that is picked.
    pub fn take(&mut self, index: Option<usize>, used: &[&str]) -> Option<String> {
        let mut rows = self.rows(used).0;
        let index = index.unwrap_or(self.picked);
        self.close();
        (index < rows.len()).then(|| rows.swap_remove(index).family)
    }

    /// The families that show, and how many more what is typed finds. With nothing typed: the bundled
    /// families, those the document's style names (`used`), then the rest by name. What is typed finds a
    /// family by the start of its name before the start of a later word, and that before its middle.
    pub fn rows(&self, used: &[&str]) -> (Vec<Row>, usize) {
        let typed = self.typed.trim().to_lowercase();
        let bundled = FAMILIES.iter().map(|(family, _)| (*family, "Built in"));
        let offered = |family: &str| self.offers.iter().find(|offer| offer.family.eq_ignore_ascii_case(family));
        let known = |family: &str| FAMILIES.iter().any(|(own, _)| own.eq_ignore_ascii_case(family));
        // A family the style names that nothing here can find is still listed, so it can be chosen again.
        let named = used.iter().filter(|family| !known(family)).map(|family| match offered(family) {
            Some(offer) => (offer.family.as_str(), from(offer.source)),
            None => (*family, "Not found"),
        });
        let rest = self.offers.iter().filter(|offer| !used.iter().any(|used| used.eq_ignore_ascii_case(&offer.family)));
        let rest = rest.map(|offer| (offer.family.as_str(), from(offer.source)));
        let mut found: Vec<(u8, &str, &'static str)> = bundled
            .chain(named)
            .chain(rest)
            .filter_map(|(family, from)| Some((near(family, &typed)?, family, from)))
            .collect();
        // The sort keeps the order above among families found alike.
        found.sort_by_key(|(near, ..)| *near);
        let more = found.len().saturating_sub(LISTED);
        found.truncate(LISTED);
        (found.into_iter().map(|(_, family, from)| Row { family: family.to_owned(), from }).collect(), more)
    }

    /// The list of fonts, which hangs under the bar's font.
    pub fn card(&self, used: &[&str]) -> Popover<Message> {
        let send = Message::Font;
        let field = TextField::new("Font", self.typed.as_str())
            .placeholder("Find a font")
            .on_input(move |typed| send(Fonting::Type(typed)))
            .on_submit(send(Fonting::Enter))
            .on_cancel(send(Fonting::Close));
        let field = Element::from(field).key("font").takes_focus();
        let (rows, more) = self.rows(used);
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let list: Element<Message> = if rows.is_empty() {
            let none = Element::text("No font matches.", quiet).fg(tokens::FG_MUTED).role(accesskit::Role::Status);
            Element::row().padding(Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS)).child(none)
        } else {
            let mut options = Options::new("Fonts").picked(self.picked.min(rows.len() - 1));
            let aside = TextStyle::control().size(tokens::TEXT_XS);
            for (index, row) in rows.iter().enumerate() {
                let choice = Choice::new(row.family.clone()).on_take(send(Fonting::Take(index)));
                options = options.choice(choice.aside(row.from, aside));
            }
            let bar = if rows.len() > ROWS { tokens::SPACE_SM } else { 0.0 };
            let list = Element::from(options).padding(Edges { right: bar, ..Edges::all(0.0) });
            list.max_height(ROWS as f32 * options::ROW + tokens::SPACE_XS).scroll()
        };
        // The field has the keys: those that walk a list are no use to one line of text.
        let keys = move |event: &letters_editor::KeyEvent| match event.key {
            _ if event.mods != letters_editor::Modifiers::default() => None,
            letters_editor::Key::Down => Some(send(Fonting::Step { down: true })),
            letters_editor::Key::Up => Some(send(Fonting::Step { down: false })),
            _ => None,
        };
        let card = Element::column().width(CARD).gap(tokens::SPACE_XS).padding(Edges::all(tokens::SPACE_XS));
        let mut card = card.child(field).child(list);
        if more > 0 {
            let said = format!("{more} more. Type a name to find one.");
            let said = Element::text(said, TextStyle::control().size(tokens::TEXT_XS)).fg(tokens::FG_MUTED);
            card = card.child(Element::row().padding(Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS)).child(said));
        }
        Popover::new(card.on_key(keys)).label("Fonts").flush().open(true).on_close(send(Fonting::Close))
    }
}

/// What the list says of where a family is from.
fn from(source: FontSource) -> &'static str {
    match source {
        FontSource::Device => "On this device",
        FontSource::Library => "Google Fonts",
    }
}

/// How well what is typed, in small letters, finds a family: `None` when it does not.
fn near(family: &str, typed: &str) -> Option<u8> {
    let name = family.to_lowercase();
    if name.starts_with(typed) {
        Some(0)
    } else if name.split_whitespace().any(|word| word.starts_with(typed)) {
        Some(1)
    } else {
        name.contains(typed).then_some(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn families() -> Families {
        let offer = |family: &str, source| FontOffer { family: family.into(), source };
        let mut families = Families::default();
        families.set(vec![
            offer("Lora", FontSource::Library),
            offer("Georgia", FontSource::Device),
            offer("Open Sans", FontSource::Library),
            offer("Lora", FontSource::Device),
            offer("inter", FontSource::Device),
        ]);
        families
    }

    fn found(families: &mut Families, typed: &str, used: &[&str]) -> Vec<(String, &'static str)> {
        families.typed(typed.into());
        families.rows(used).0.into_iter().map(|row| (row.family, row.from)).collect()
    }

    #[test]
    fn what_is_typed_finds_families_by_the_start_of_a_name_first() {
        let mut families = families();
        // What is typed, the families the style names, and the families found.
        let cases: [(&str, &[&str], &[&str]); 6] = [
            ("", &[], &["Inter", "Montserrat", "Fira Code", "Georgia", "Lora", "Open Sans"]),
            ("", &["Lora", "Inter"], &["Inter", "Montserrat", "Fira Code", "Lora", "Georgia", "Open Sans"]),
            ("o", &[], &["Open Sans", "Montserrat", "Fira Code", "Georgia", "Lora"]),
            ("SANS", &[], &["Open Sans"]),
            ("", &["Calibri"], &["Inter", "Montserrat", "Fira Code", "Calibri", "Georgia", "Lora", "Open Sans"]),
            ("klingon", &[], &[]),
        ];
        for (typed, used, families_found) in cases {
            let names: Vec<String> = found(&mut families, typed, used).into_iter().map(|(family, _)| family).collect();
            assert_eq!(names, families_found, "{typed:?} with {used:?}");
        }
    }

    #[test]
    fn a_family_says_where_it_is_from() {
        let mut families = families();
        let rows = found(&mut families, "", &["Calibri"]);
        let from = |family: &str| rows.iter().find(|(name, _)| name == family).map(|(_, from)| *from);
        assert_eq!(from("Inter"), Some("Built in"), "a system copy of a bundled family is not offered");
        assert_eq!(from("Lora"), Some("Google Fonts"), "a family named twice is offered as first said");
        assert_eq!(from("Georgia"), Some("On this device"));
        assert_eq!(from("Calibri"), Some("Not found"));
    }

    #[test]
    fn the_list_takes_the_family_pressed_or_the_one_picked() {
        let mut families = families();
        families.open();
        families.typed("o".into());
        families.step(true, &[]);
        assert_eq!(families.take(None, &[]).as_deref(), Some("Montserrat"), "Enter takes the pick");
        assert!(!families.is_open(), "and puts the list away");
        families.open();
        assert_eq!(families.take(Some(4), &[]).as_deref(), Some("Lora"), "a press takes its row");
        families.open();
        assert_eq!(families.take(Some(99), &[]), None);
    }
}
