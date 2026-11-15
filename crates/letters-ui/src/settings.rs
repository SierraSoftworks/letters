//! Settings: what a person sets for themselves, whatever document is open.
//!
//! The dialog shows and asks. The shell keeps what is set, and follows the parts only it can. How a document
//! looks is not in here: that is the document's own, in the Styles panel.

use accesskit::{HasPopup, Role};

use crate::color::{fade, same};
use crate::element::Element;
use crate::icon;
use crate::style::{Align, Edges, Justify, Look, Ring};
use crate::text::TextStyle;
use crate::theme::accent;
use crate::tokens::{self, Color as Ink};
use crate::widgets::{Button, Menu, MenuItem, Popover, Side, SidePanel, TextField};
use crate::window::Message;

/// The width of the dialog, and of the names down its left side.
const WIDTH: f32 = 560.0;
const NAMES: f32 = 116.0;
/// The size of an accent's dot.
const DOT: f32 = 20.0;
/// The tallest the list of languages grows before it scrolls.
const LANGUAGES: f32 = 296.0;

/// Whether the window is light or dark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// As the person's system is.
    #[default]
    System,
    Light,
    Dark,
}

/// What a person sets for themselves. Everything is optional: a default `Settings` is a person who has set
/// nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub appearance: Appearance,
    /// The accent as sRGB. `None` is yewi's own blue.
    pub accent: Option<[u8; 3]>,
    /// The name the person's edits are signed with. `None` leaves it to the shell, which knows the account.
    pub name: Option<String>,
    /// The language new documents are written in, as a BCP 47 tag such as `en-GB`. `None` is English as
    /// the United States spell it.
    pub language: Option<String>,
}

/// What the dialog sends.
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Show,
    Close,
    Appearance(Appearance),
    Accent(Option<[u8; 3]>),
    /// What is typed into the accent's field, before it is taken.
    Hex(String),
    /// Takes the colour that was typed, or drops it if it is none.
    TakeHex,
    /// What is typed into the name field, before it is taken.
    Name(String),
    TakeName,
    /// Drops what was typed into either field.
    Forget,
    Language(String),
}

/// The accents offered. Any other colour can be typed.
const ACCENTS: [(&str, Option<[u8; 3]>); 8] = [
    ("Blue", None),
    ("Teal", Some([0x0e, 0x8a, 0x8f])),
    ("Green", Some([0x1f, 0x8a, 0x5b])),
    ("Amber", Some([0xb9, 0x85, 0x2f])),
    ("Red", Some([0xc4, 0x37, 0x2e])),
    ("Pink", Some([0xc2, 0x41, 0x7f])),
    ("Violet", Some([0x6d, 0x4a, 0xff])),
    ("Graphite", Some([0x5a, 0x64, 0x70])),
];

/// The kinds of English that are built in, which every shell can offer.
const ENGLISH: [&str; 4] = ["en-US", "en-GB", "en-AU", "en-CA"];

/// What the dialog keeps between frames.
#[derive(Default)]
pub(crate) struct Dialog {
    pub open: bool,
    /// What is being typed into the name field, and into the accent's.
    name: Option<String>,
    hex: Option<String>,
    /// The languages the shell has dictionaries for, besides English.
    pub languages: Vec<String>,
    /// The name edits carry while the person has set none.
    pub account: Option<String>,
}

impl Dialog {
    /// Does what the dialog asked. Returns whether `settings` changed, which the shell is then told.
    pub fn update(&mut self, message: Setting, settings: &mut Settings) -> bool {
        let before = settings.clone();
        match message {
            Setting::Show => self.open = true,
            Setting::Close => (self.open, self.name, self.hex) = (false, None, None),
            Setting::Appearance(appearance) => settings.appearance = appearance,
            Setting::Accent(accent) => (settings.accent, self.hex) = (accent, None),
            Setting::Hex(typed) => self.hex = Some(typed),
            Setting::TakeHex => {
                if let Some(rgb) = self.hex.take().and_then(|typed| read_hex(&typed)) {
                    // yewi's own blue is no choice at all, so that it follows yewi if yewi changes it.
                    settings.accent = Some(rgb).filter(|rgb| *rgb != rgb_of(tokens::ACCENT));
                }
            }
            Setting::Name(typed) => self.name = Some(typed),
            Setting::TakeName => {
                if let Some(typed) = self.name.take() {
                    settings.name = Some(typed.trim().to_owned()).filter(|name| !name.is_empty());
                }
            }
            Setting::Forget => (self.name, self.hex) = (None, None),
            Setting::Language(language) => settings.language = Some(language),
        }
        *settings != before
    }

    /// The dialog over the middle of the window, while it is open. It hangs from an element of no size,
    /// which the window puts after everything else so that the dialog lies over it all.
    pub fn view(&self, settings: &Settings) -> Option<Element<Message>> {
        if !self.open {
            return None;
        }
        let send = |setting: Setting| Message::Settings(setting);
        let head = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .child(Element::text("Settings", TextStyle::title().size(tokens::TEXT_XL)))
            .child(Button::icon_only(icon::X, "Close").subtle().small().on_press(send(Setting::Close)));
        // Said first, so that nobody looks here for what the Styles panel holds.
        let note = "These settings influence every document you open. Document-level styles can be modified in \
                    the document itself.";
        let note = Element::paragraph(note, quiet()).fg(tokens::FG_MUTED).padding(Edges::top(tokens::SPACE_XS));
        let content = Element::column()
            .width(WIDTH)
            .padding(Edges { top: tokens::SPACE_LG, ..Edges::all(tokens::SPACE_XL) })
            .child(head)
            .child(note)
            .child(SidePanel::section("Appearance"))
            .child(Element::column().gap(tokens::SPACE_MD).child(appearance(settings)).child(self.accent(settings)))
            .child(SidePanel::section("Writing"))
            .child(Element::column().gap(tokens::SPACE_MD).child(self.name(settings)).child(self.language(settings)));
        let dialog = Popover::new(content)
            .flush()
            .label("Settings")
            .max_width(WIDTH)
            .modal()
            .open(true)
            .on_close(send(Setting::Close));
        Some(Element::row().overlay().popover(dialog))
    }

    /// The accents on offer, and a field for any other colour.
    fn accent(&self, settings: &Settings) -> Element<Message> {
        let mut dots = Element::row().gap(tokens::SPACE_XS).align(Align::Center);
        for (name, rgb) in ACCENTS {
            let chosen = settings.accent == rgb;
            let [r, g, b] = rgb.unwrap_or(rgb_of(tokens::ACCENT));
            let dot = Element::row().width(DOT).height(DOT).radius(DOT / 2.0).bg(same(r, g, b));
            // The ring is always there, clear until its colour is the one chosen, so that no dot moves.
            let ring = if chosen { accent().base } else { fade(tokens::FG, 0.0) };
            let swatch = Element::row()
                .padding(Edges::all(2.0))
                .border(Edges::all(2.0), ring)
                .radius(DOT / 2.0 + 4.0)
                .hover(Look::bg(tokens::NEUTRAL_SOFT))
                .ring(Ring::Outline(2.0))
                .role(Role::Button)
                .label(name)
                .pressed(chosen)
                .tooltip(name, Side::Top)
                .on_press(Message::Settings(Setting::Accent(rgb)))
                .child(dot);
            dots = dots.child(swatch);
        }
        let typed = self.hex.as_ref();
        let shown = hex(settings.accent.unwrap_or(rgb_of(tokens::ACCENT)));
        let field = TextField::new("Accent colour", typed.cloned().unwrap_or(shown))
            .mono()
            .invalid(typed.is_some_and(|typed| read_hex(typed).is_none()))
            .on_input(|typed| Message::Settings(Setting::Hex(typed)))
            .on_submit(Message::Settings(Setting::TakeHex))
            .on_leave(Message::Settings(Setting::TakeHex))
            .on_cancel(Message::Settings(Setting::Forget));
        let control = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .gap(tokens::SPACE_MD)
            .child(dots)
            .child(Element::from(field).width(92.0));
        row("Accent", control, None)
    }

    /// The name edits are signed with. Left empty, the field shows the name they carry anyway.
    fn name(&self, settings: &Settings) -> Element<Message> {
        let shown = self.name.clone().or(settings.name.clone()).unwrap_or_default();
        let field = TextField::new("Your name", shown)
            .placeholder(self.account.as_deref().unwrap_or("Your name"))
            .on_input(|typed| Message::Settings(Setting::Name(typed)))
            .on_submit(Message::Settings(Setting::TakeName))
            .on_leave(Message::Settings(Setting::TakeName))
            .on_cancel(Message::Settings(Setting::Forget));
        row("Your name", field.into(), Some("Signs your edits in each document's history."))
    }

    /// The language of new documents: the kinds of English built in, then the system's dictionaries.
    fn language(&self, settings: &Settings) -> Element<Message> {
        let chosen = settings.language.as_deref().unwrap_or(ENGLISH[0]);
        let mut offered: Vec<&str> = ENGLISH.to_vec();
        let others = self.languages.iter().map(String::as_str).chain([chosen]);
        offered.extend(others.filter(|tag| !ENGLISH.iter().any(|english| english.eq_ignore_ascii_case(tag))));
        offered[ENGLISH.len()..].sort_by_key(|tag| language_name(tag));
        offered.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

        let mut menu = Menu::new().label("Language");
        for tag in offered {
            let pick = Message::Settings(Setting::Language(tag.to_owned()));
            let item = MenuItem::new(language_name(tag)).checked(tag.eq_ignore_ascii_case(chosen));
            menu = menu.item(item.on_select(pick));
        }
        // The list is as wide as the control it drops from, as a select's is.
        let list = Element::from(menu).min_width(WIDTH - 2.0 * tokens::SPACE_XL - NAMES - tokens::SPACE_LG);
        let list = Popover::new(list.max_height(LANGUAGES).scroll()).max_width(WIDTH).list(HasPopup::Menu).gap(4.0);
        let name = language_name(chosen);
        let select = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .height(tokens::CONTROL_HEIGHT)
            .padding(Edges::xy(tokens::CONTROL_PADDING - 4.0, 0.0))
            .border(Edges::all(1.0), tokens::LINE_STRONG)
            .radius(tokens::RADIUS_CONTROL)
            .bg(tokens::SURFACE)
            .hover(Look::bg(tokens::TONE_1))
            .ring(Ring::Outline(2.0))
            .role(Role::Button)
            .label("Language")
            .description(name.clone())
            .child(Element::text(name, TextStyle::control()).grow(1.0))
            .child(Element::icon(icon::CHEVRON_DOWN, 14.0).fg(tokens::FG_MUTED))
            .popover(list);
        row("Language", select, Some("New documents are written and spell-checked in it."))
    }
}

/// Light, dark, or as the system is: three buttons of which one is down.
fn appearance(settings: &Settings) -> Element<Message> {
    let choices = [("System", Appearance::System), ("Light", Appearance::Light), ("Dark", Appearance::Dark)];
    let buttons = choices.map(|(name, appearance)| {
        let button = Button::new(name).pressed(settings.appearance == appearance);
        Element::from(button.on_press(Message::Settings(Setting::Appearance(appearance)))).min_width(76.0)
    });
    row("Theme", Element::row().gap(tokens::SPACE_XS).children(buttons), None)
}

/// One setting: its name, the control that sets it, and perhaps a word about what it does.
fn row(name: &str, control: Element<Message>, about: Option<&str>) -> Element<Message> {
    // The control names itself to assistive technology, so the name beside it is for the eye.
    let name = Element::row()
        .align(Align::Center)
        .width(NAMES)
        .height(tokens::CONTROL_HEIGHT)
        .child(Element::text(name, TextStyle::control().weight(500)))
        .decorative();
    let about = about.map(|about| Element::paragraph(about, quiet()).fg(tokens::FG_MUTED));
    let set = Element::column().gap(tokens::SPACE_XS).grow(1.0).child(control).children(about);
    Element::row().align(Align::Start).gap(tokens::SPACE_LG).child(name).child(set)
}

fn quiet() -> TextStyle {
    TextStyle::body().size(tokens::TEXT_SM)
}

fn rgb_of(ink: Ink) -> [u8; 3] {
    [ink.light.r, ink.light.g, ink.light.b]
}

pub(crate) fn hex([r, g, b]: [u8; 3]) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// Reads a colour as people type it: six hex digits, with or without the `#`.
pub(crate) fn read_hex(typed: &str) -> Option<[u8; 3]> {
    let digits = typed.trim();
    let digits = digits.strip_prefix('#').unwrap_or(digits);
    // A sign would parse as a number, and is no part of a colour.
    let hex = digits.len() == 6 && digits.bytes().all(|digit| digit.is_ascii_hexdigit());
    let rgb = u32::from_str_radix(digits, 16).ok().filter(|_| hex)?;
    Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

/// A language tag as people say it: `de-CH` is "German (Switzerland)". What the tables do not know is said
/// as it is written.
pub(crate) fn language_name(tag: &str) -> String {
    let mut parts = tag.split(['-', '_']);
    let code = parts.next().unwrap_or_default().to_ascii_lowercase();
    let Some((_, language)) = LANGUAGE_NAMES.iter().find(|(known, _)| *known == code) else { return tag.to_owned() };
    // The region is the part of two letters, after a script if there is one.
    let region = parts.find(|part| part.len() == 2).map(str::to_ascii_uppercase);
    match region {
        Some(region) => {
            let place =
                REGION_NAMES.iter().find(|(known, _)| *known == region).map_or(region.as_str(), |found| found.1);
            format!("{language} ({place})")
        }
        None => (*language).to_owned(),
    }
}

/// The languages Hunspell dictionaries are commonly installed for.
const LANGUAGE_NAMES: [(&str, &str); 48] = [
    ("af", "Afrikaans"),
    ("ar", "Arabic"),
    ("bg", "Bulgarian"),
    ("bn", "Bengali"),
    ("ca", "Catalan"),
    ("cs", "Czech"),
    ("cy", "Welsh"),
    ("da", "Danish"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("eo", "Esperanto"),
    ("es", "Spanish"),
    ("et", "Estonian"),
    ("eu", "Basque"),
    ("fa", "Persian"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("ga", "Irish"),
    ("gl", "Galician"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hr", "Croatian"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("is", "Icelandic"),
    ("it", "Italian"),
    ("ko", "Korean"),
    ("la", "Latin"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("nb", "Norwegian Bokmål"),
    ("nl", "Dutch"),
    ("nn", "Norwegian Nynorsk"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("sr", "Serbian"),
    ("sv", "Swedish"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
    ("xh", "Xhosa"),
    ("zu", "Zulu"),
];

/// The regions those dictionaries are for.
const REGION_NAMES: [(&str, &str); 50] = [
    ("AR", "Argentina"),
    ("AT", "Austria"),
    ("AU", "Australia"),
    ("BE", "Belgium"),
    ("BG", "Bulgaria"),
    ("BR", "Brazil"),
    ("CA", "Canada"),
    ("CH", "Switzerland"),
    ("CL", "Chile"),
    ("CO", "Colombia"),
    ("CZ", "Czechia"),
    ("DE", "Germany"),
    ("DK", "Denmark"),
    ("EE", "Estonia"),
    ("ES", "Spain"),
    ("FI", "Finland"),
    ("FR", "France"),
    ("GB", "United Kingdom"),
    ("GR", "Greece"),
    ("HR", "Croatia"),
    ("HU", "Hungary"),
    ("ID", "Indonesia"),
    ("IE", "Ireland"),
    ("IL", "Israel"),
    ("IN", "India"),
    ("IS", "Iceland"),
    ("IT", "Italy"),
    ("KR", "South Korea"),
    ("LT", "Lithuania"),
    ("LU", "Luxembourg"),
    ("LV", "Latvia"),
    ("MX", "Mexico"),
    ("NL", "Netherlands"),
    ("NO", "Norway"),
    ("NZ", "New Zealand"),
    ("PH", "Philippines"),
    ("PL", "Poland"),
    ("PT", "Portugal"),
    ("RO", "Romania"),
    ("RS", "Serbia"),
    ("RU", "Russia"),
    ("SE", "Sweden"),
    ("SI", "Slovenia"),
    ("SK", "Slovakia"),
    ("TH", "Thailand"),
    ("TR", "Türkiye"),
    ("UA", "Ukraine"),
    ("US", "United States"),
    ("VN", "Vietnam"),
    ("ZA", "South Africa"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_read_as_people_type_it() {
        let cases = [
            ("#156a4f", Some([0x15, 0x6a, 0x4f])),
            ("156A4F", Some([0x15, 0x6a, 0x4f])),
            (" #ffffff ", Some([0xff, 0xff, 0xff])),
            ("#fff", None),
            ("green", None),
            ("#156a4f00", None),
            ("+156a4", None),
            ("", None),
        ];
        for (typed, expected) in cases {
            assert_eq!(read_hex(typed), expected, "{typed:?}");
        }
    }

    #[test]
    fn a_language_is_named_as_people_say_it() {
        let cases = [
            ("en-GB", "English (United Kingdom)"),
            ("de_CH", "German (Switzerland)"),
            ("fr", "French"),
            ("sr-Latn-RS", "Serbian (Serbia)"),
            ("pt-AO", "Portuguese (AO)"),
            ("tlh", "tlh"),
        ];
        for (tag, expected) in cases {
            assert_eq!(language_name(tag), expected, "{tag}");
        }
    }

    #[test]
    fn what_is_typed_changes_nothing_until_it_is_taken() {
        let (mut dialog, mut settings) = (Dialog::default(), Settings::default());
        // Each step: what the dialog sends, and whether the shell has something new to keep.
        let steps = [
            (Setting::Show, false),
            (Setting::Name(" Ben ".into()), false),
            (Setting::TakeName, true),
            (Setting::TakeName, false),
            (Setting::Hex("#156a".into()), false),
            (Setting::TakeHex, false),
            (Setting::Hex("156a4f".into()), false),
            (Setting::TakeHex, true),
            (Setting::Name("Someone else".into()), false),
            (Setting::Forget, false),
            (Setting::Appearance(Appearance::Dark), true),
            (Setting::Language("en-GB".into()), true),
            (Setting::Language("en-GB".into()), false),
            (Setting::Close, false),
        ];
        for (step, changes) in steps {
            assert_eq!(dialog.update(step.clone(), &mut settings), changes, "{step:?}");
        }
        let expected = Settings {
            appearance: Appearance::Dark,
            accent: Some([0x15, 0x6a, 0x4f]),
            name: Some("Ben".into()),
            language: Some("en-GB".into()),
        };
        assert_eq!(settings, expected);
    }

    #[test]
    fn clearing_a_setting_gives_it_back_to_its_default() {
        let (mut dialog, mut settings) = (Dialog::default(), Settings::default());
        settings.name = Some("Ben".into());
        settings.accent = Some([1, 2, 3]);
        dialog.update(Setting::Name("  ".into()), &mut settings);
        dialog.update(Setting::TakeName, &mut settings);
        dialog.update(Setting::Hex(hex(rgb_of(tokens::ACCENT))), &mut settings);
        dialog.update(Setting::TakeHex, &mut settings);
        assert_eq!(settings, Settings::default());
    }
}
