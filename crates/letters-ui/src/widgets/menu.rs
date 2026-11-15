//! Menu: a column of actions. It draws no surface of its own: a popover around it does.

use accesskit::{HasPopup, Role};

use crate::color::fade;
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::style::{Align, Dir, Edges, Look};
use crate::text::{Family, TextStyle};
use crate::tokens;
use crate::widgets::Popover;

const ICON: f32 = 16.0;

/// How wide a note is set before it wraps.
const NOTE: f32 = 224.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Default,
    Warn,
    Danger,
}

pub struct MenuItem<M> {
    label: String,
    icon: Option<Icon>,
    aside: Option<String>,
    checked: Option<bool>,
    face: Family,
    tone: Tone,
    disabled: bool,
    on_select: Option<M>,
}

impl<M> MenuItem<M> {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            aside: None,
            checked: None,
            face: Family::Sans,
            tone: Tone::Default,
            disabled: false,
            on_select: None,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Quiet text at the end of the row, such as a shortcut.
    pub fn aside(mut self, aside: impl Into<String>) -> Self {
        self.aside = Some(aside.into());
        self
    }

    /// Makes the item checkable and says whether it is on.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// The family the label is set in, for a menu of fonts that shows each in its own face.
    pub fn face(mut self, face: Family) -> Self {
        self.face = face;
        self
    }

    pub fn danger(mut self) -> Self {
        self.tone = Tone::Danger;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sent when the item is chosen. The menu's popover closes with it.
    pub fn on_select(mut self, message: M) -> Self {
        self.on_select = Some(message);
        self
    }
}

enum Entry<M> {
    Item(MenuItem<M>),
    Heading(String),
    Note(String, Tone),
    Separator,
}

pub struct Menu<M> {
    label: Option<String>,
    entries: Vec<Entry<M>>,
    leads: bool,
}

impl<M> Default for Menu<M> {
    fn default() -> Self {
        Self { label: None, entries: Vec::new(), leads: false }
    }
}

impl<M> Menu<M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The menu's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn item(mut self, item: MenuItem<M>) -> Self {
        self.entries.push(Entry::Item(item));
        self
    }

    /// Starts a group under a small heading. A rule sets it apart from what came before.
    pub fn heading(mut self, heading: impl Into<String>) -> Self {
        self.entries.push(Entry::Heading(heading.into()));
        self
    }

    /// A few quiet words on how things stand. They wrap, and are not there to choose.
    pub fn note(mut self, note: impl Into<String>, tone: Tone) -> Self {
        self.entries.push(Entry::Note(note.into(), tone));
        self
    }

    /// Has the first item that can be chosen take the keyboard when the menu shows, for a menu that is
    /// not the first thing in its popover.
    pub fn leads(mut self) -> Self {
        self.leads = true;
        self
    }

    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator);
        self
    }

    /// The menu on a popover surface, ready to attach to its button.
    pub fn popover(self) -> Popover<M> {
        Popover::new(self).list(HasPopup::Menu)
    }
}

impl<M> From<Menu<M>> for Element<M> {
    fn from(menu: Menu<M>) -> Self {
        let mut element = Element::column()
            .min_width(176.0)
            .padding(Edges::xy(0.0, tokens::SPACE_XS))
            .fg(tokens::FG)
            .role(Role::Menu)
            .arrows(Dir::Column);
        if let Some(label) = menu.label {
            element = element.label(label);
        }

        let mut danger_before = None;
        let mut ruled = true;
        let mut leads = menu.leads;
        for entry in menu.entries {
            // yewi draws a rule wherever two kinds of thing meet, without being asked.
            let needs_rule = match &entry {
                Entry::Heading(_) => !ruled,
                Entry::Note(..) => false,
                Entry::Item(item) => !ruled && item.tone == Tone::Danger && danger_before == Some(false),
                // A separator with nothing above it, or right under another, has nothing to part.
                Entry::Separator => !ruled,
            };
            if needs_rule {
                element = element.child(rule());
            }
            ruled = matches!(entry, Entry::Separator);
            match entry {
                Entry::Separator => {}
                Entry::Heading(heading) => {
                    let style = TextStyle::body().size(tokens::TEXT_XS).weight(600);
                    let label = Element::text(heading, style).fg(tokens::FG_MUTED);
                    let padding =
                        Edges { top: tokens::SPACE_XS, bottom: tokens::SPACE_2XS, ..Edges::xy(tokens::SPACE_MD, 0.0) };
                    element = element.child(Element::row().padding(padding).child(label));
                }
                Entry::Note(note, tone) => {
                    let fg = match tone {
                        Tone::Default => tokens::FG_MUTED,
                        Tone::Warn => tokens::WARN,
                        Tone::Danger => tokens::DANGER,
                    };
                    let style = TextStyle::body().size(tokens::TEXT_XS);
                    let said = Element::paragraph(note, style).fg(fg).max_width(NOTE).role(Role::Status);
                    let padding = Edges::xy(tokens::SPACE_MD, tokens::SPACE_XS);
                    element = element.child(Element::column().padding(padding).child(said));
                }
                Entry::Item(item) => {
                    danger_before = Some(item.tone == Tone::Danger);
                    let first = leads && !item.disabled;
                    leads &= !first;
                    element = element.child(row(item, first));
                }
            }
        }
        element
    }
}

fn rule<M>() -> Element<M> {
    let line = Element::row().height(1.0).bg(tokens::LINE);
    Element::column().padding(Edges::xy(0.0, tokens::SPACE_XS)).child(line).decorative()
}

fn row<M>(item: MenuItem<M>, leads: bool) -> Element<M> {
    let (fg, active) = match item.tone {
        Tone::Default => (tokens::FG, tokens::NEUTRAL_SOFT),
        Tone::Warn => (tokens::WARN, tokens::NEUTRAL_SOFT),
        Tone::Danger => (tokens::DANGER, tokens::DANGER_SOFT),
    };
    let mut row = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .min_height(tokens::CONTROL_HEIGHT)
        .padding(Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS))
        .radius(tokens::RADIUS_SM)
        .fg(fg)
        .hover(Look::bg(active))
        .focused(Look::bg(active))
        .grow(1.0)
        .role(if item.checked.is_some() { Role::MenuItemCheckBox } else { Role::MenuItem })
        .focusable()
        .closes()
        .disabled(item.disabled);
    if item.disabled {
        row = row.opacity(tokens::DISABLED_OPACITY);
    }
    if let Some(icon) = item.icon {
        row = row.child(Element::icon(icon, ICON).fg(fade(fg, 0.7)));
    }
    row = row.child(Element::text(item.label, TextStyle::control().family(item.face)).grow(1.0));
    let trailing = Edges::left(tokens::SPACE_XS);
    if let Some(aside) = item.aside {
        let aside = Element::text(aside, TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED);
        row = row.child(Element::row().padding(trailing).child(aside));
    }
    if let Some(checked) = item.checked {
        // An unchecked item keeps the tick's room, so labels line up down the menu.
        let tick = if checked { Element::icon(icon::CHECK, ICON) } else { Element::row().width(ICON).height(ICON) };
        row = row.child(Element::row().padding(trailing).child(tick)).checked(checked);
    }
    if let Some(message) = item.on_select {
        row = row.on_press(message);
    }
    if leads {
        row = row.takes_focus();
    }
    // The gutter around a row is the menu's padding, which rules run through.
    Element::row().padding(Edges::xy(tokens::SPACE_XS, 0.0)).child(row)
}
