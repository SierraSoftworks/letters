//! Options: a short list to pick from while the keyboard stays where the person is typing.
//!
//! Focus never enters the list. Its owner hears the keys and says which choice is picked, and the list shows
//! that one and tells assistive technology of it. A press on a choice takes it outright.

use accesskit::Role;

use crate::element::Element;
use crate::style::{Align, Edges, Look};
use crate::text::TextStyle;
use crate::tokens;

/// The least height of a choice's row.
const ROW: f32 = 28.0;

pub struct Choice<M> {
    label: String,
    style: TextStyle,
    lead: Option<Box<Element<M>>>,
    aside: Option<(String, TextStyle)>,
    on_take: Option<M>,
}

impl<M> Choice<M> {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), style: TextStyle::control(), lead: None, aside: None, on_take: None }
    }

    /// The style the label is set in.
    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// What stands before the label: an icon, or a picture of what the choice gives.
    pub fn lead(mut self, lead: impl Into<Element<M>>) -> Self {
        self.lead = Some(Box::new(lead.into()));
        self
    }

    /// Quiet text at the end of the row, which assistive technology reads as the choice's description.
    pub fn aside(mut self, aside: impl Into<String>, style: TextStyle) -> Self {
        self.aside = Some((aside.into(), style));
        self
    }

    /// Sent when the choice is pressed.
    pub fn on_take(mut self, message: M) -> Self {
        self.on_take = Some(message);
        self
    }
}

enum Entry<M> {
    Choice(Choice<M>),
    Heading(String),
}

pub struct Options<M> {
    label: String,
    entries: Vec<Entry<M>>,
    picked: usize,
}

impl<M> Options<M> {
    /// A list with a name for assistive technology.
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), entries: Vec::new(), picked: 0 }
    }

    pub fn choice(mut self, choice: Choice<M>) -> Self {
        self.entries.push(Entry::Choice(choice));
        self
    }

    /// Starts a group under a small heading.
    pub fn heading(mut self, heading: impl Into<String>) -> Self {
        self.entries.push(Entry::Heading(heading.into()));
        self
    }

    /// Which choice is picked, counted from the first and past any headings.
    pub fn picked(mut self, picked: usize) -> Self {
        self.picked = picked;
        self
    }

    /// The pick one step down or up a list of `count` choices, round its ends.
    pub fn step(picked: usize, down: bool, count: usize) -> usize {
        let last = count.saturating_sub(1);
        (picked.min(last) + if down { 1 } else { last }) % count.max(1)
    }
}

impl<M> From<Options<M>> for Element<M> {
    fn from(options: Options<M>) -> Self {
        let mut list = Element::column().fg(tokens::FG).role(Role::ListBox).label(options.label);
        let mut index = 0;
        for entry in options.entries {
            match entry {
                Entry::Heading(heading) => {
                    let style = TextStyle::body().size(tokens::TEXT_XS).weight(600);
                    let padding =
                        Edges { top: tokens::SPACE_SM, bottom: tokens::SPACE_2XS, ..Edges::xy(tokens::SPACE_SM, 0.0) };
                    let label = Element::text(heading, style).fg(tokens::FG_MUTED);
                    list = list.child(Element::row().padding(padding).child(label));
                }
                Entry::Choice(choice) => {
                    list = list.child(row(choice, index == options.picked));
                    index += 1;
                }
            }
        }
        list
    }
}

fn row<M>(choice: Choice<M>, picked: bool) -> Element<M> {
    let mut row = Element::row()
        .key(choice.label.as_str())
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .min_height(ROW)
        .padding(Edges::xy(tokens::SPACE_SM, 3.0))
        .radius(tokens::RADIUS_CONTROL)
        .hover(Look::bg(tokens::NEUTRAL_SOFT))
        .role(Role::ListBoxOption)
        .label(choice.label.as_str())
        .selected(picked)
        .children(choice.lead.map(|lead| *lead))
        .child(Element::text(choice.label, choice.style));
    if let Some((aside, style)) = choice.aside {
        row = row
            .description(aside.as_str())
            .child(Element::row().grow(1.0).min_width(tokens::SPACE_SM))
            .child(Element::text(aside, style).fg(tokens::FG_MUTED));
    }
    if let Some(message) = choice.on_take {
        row = row.on_press(message);
    }
    if picked { row.bg(tokens::ACCENT_SOFT) } else { row }
}
