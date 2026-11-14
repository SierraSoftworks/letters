//! TopBar: the editor window's one bar. The document sits at the start, the format controls take the middle
//! and give way first when the window narrows, and the window's actions sit at the end.

use accesskit::Role;

use crate::element::Element;
use crate::style::{Align, Dir, Edges};
use crate::tokens;

/// The bar's height: yewi's control height with room to breathe.
pub const HEIGHT: f32 = 44.0;

pub struct TopBar<M> {
    label: String,
    start: Vec<Element<M>>,
    middle: Vec<Element<M>>,
    end: Vec<Element<M>>,
}

impl<M> Default for TopBar<M> {
    fn default() -> Self {
        Self { label: "Document".into(), start: Vec::new(), middle: Vec::new(), end: Vec::new() }
    }
}

impl<M> TopBar<M> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bar's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn start(mut self, child: impl Into<Element<M>>) -> Self {
        self.start.push(child.into());
        self
    }

    pub fn middle(mut self, child: impl Into<Element<M>>) -> Self {
        self.middle.push(child.into());
        self
    }

    pub fn end(mut self, child: impl Into<Element<M>>) -> Self {
        self.end.push(child.into());
        self
    }

    /// The short hairline between groups of controls.
    pub fn rule() -> Element<M> {
        let line = Element::row().width(1.0).height(tokens::SPACE_LG).bg(tokens::LINE);
        Element::row().padding(Edges::xy(tokens::SPACE_XS, 0.0)).child(line).decorative()
    }
}

impl<M> From<TopBar<M>> for Element<M> {
    fn from(bar: TopBar<M>) -> Self {
        let group = || Element::row().align(Align::Center).gap(tokens::SPACE_SM);
        Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .height(HEIGHT)
            .padding(Edges::xy(tokens::SPACE_MD, 0.0))
            .bg(tokens::SURFACE)
            .fg(tokens::FG)
            .border(Edges::bottom(1.0), tokens::LINE)
            .role(Role::Toolbar)
            .label(bar.label)
            .arrows(Dir::Row)
            .child(group().children(bar.start))
            .child(group().grow(1.0).shrink().clip().children(bar.middle))
            .child(group().children(bar.end))
    }
}
