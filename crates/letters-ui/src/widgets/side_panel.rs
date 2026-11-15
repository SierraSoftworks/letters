//! SidePanel: the one panel beside the page, for styles, notes or history.

use accesskit::Role;

use crate::element::Element;
use crate::icon;
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, top_bar};

pub const WIDTH: f32 = 344.0;

pub struct SidePanel<M> {
    title: String,
    aside: Option<String>,
    on_close: Option<M>,
    children: Vec<Element<M>>,
}

impl<M> SidePanel<M> {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), aside: None, on_close: None, children: Vec::new() }
    }

    /// A few quiet words at the end of the head, such as how much the panel lists.
    pub fn aside(mut self, aside: impl Into<String>) -> Self {
        self.aside = Some(aside.into());
        self
    }

    /// Adds a close button to the head that sends `message`.
    pub fn on_close(mut self, message: M) -> Self {
        self.on_close = Some(message);
        self
    }

    pub fn child(mut self, child: impl Into<Element<M>>) -> Self {
        self.children.push(child.into());
        self
    }

    /// A small heading over a group of the panel's content.
    pub fn section(label: &str) -> Element<M> {
        let style = TextStyle::control().size(tokens::TEXT_XS).weight(600);
        let padding = Edges { top: tokens::SPACE_LG, bottom: tokens::SPACE_SM, ..Edges::ZERO };
        Element::row().padding(padding).child(Element::text(label.to_uppercase(), style).fg(tokens::FG_MUTED))
    }
}

impl<M> From<SidePanel<M>> for Element<M> {
    fn from(panel: SidePanel<M>) -> Self {
        let mut head = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .height(top_bar::HEIGHT)
            .padding(Edges::xy(tokens::SPACE_LG, 0.0))
            .border(Edges::bottom(1.0), tokens::LINE)
            .child(Element::text(panel.title.clone(), TextStyle::title()));
        if let Some(aside) = panel.aside {
            head = head.child(Element::text(aside, TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED));
        }
        if let Some(message) = panel.on_close {
            head = head.child(Button::icon_only(icon::X, "Close").subtle().small().on_press(message));
        }
        let body = Element::column()
            .grow(1.0)
            .scroll()
            .padding(Edges { top: 0.0, ..Edges::all(tokens::SPACE_LG) })
            .children(panel.children);
        Element::column()
            .width(WIDTH)
            .bg(tokens::SURFACE)
            .fg(tokens::FG)
            .border(Edges::left(1.0), tokens::LINE)
            .role(Role::Complementary)
            .label(panel.title)
            .child(head)
            .child(body)
    }
}
