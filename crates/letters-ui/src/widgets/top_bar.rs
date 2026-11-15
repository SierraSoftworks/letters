//! TopBar: the editor window's one bar. The document sits at the start, the format controls take the middle
//! and give way first when the window narrows, and the window's actions sit at the end.
//!
//! A bar can be given narrower forms of its start and middle to fold into before anything is cut short.

use accesskit::Role;
use letters_render::Point;

use crate::element::{Element, Fitted};
use crate::style::{Align, Dir, Edges, Justify};
use crate::tokens;

/// The bar's height: yewi's control height with room to breathe.
pub const HEIGHT: f32 = 44.0;

/// How much of itself a bar's content keeps while its window is behind another.
const DIMMED: f32 = 0.5;

pub struct TopBar<M> {
    label: String,
    height: f32,
    inset: f32,
    dim: bool,
    apart: bool,
    before: Vec<Element<M>>,
    forms: Vec<Form<M>>,
    shown: Option<Fitted>,
    end: Vec<Element<M>>,
    title: Option<Title<M>>,
}

/// What a bar that is its window's title bar sends: for a drag and for a double click on the room between
/// its controls, and for a secondary press there, with the point of the window it was at.
pub struct Title<M> {
    pub drag: M,
    pub double_click: M,
    pub menu: Box<dyn Fn(Point) -> M>,
}

/// The start and the middle of the bar, as they are for one width of window.
struct Form<M> {
    key: usize,
    start: Vec<Element<M>>,
    middle: Vec<Element<M>>,
}

impl<M> Default for TopBar<M> {
    fn default() -> Self {
        Self {
            label: "Document".into(),
            height: HEIGHT,
            inset: 0.0,
            dim: false,
            apart: false,
            before: Vec::new(),
            forms: Vec::new(),
            shown: None,
            end: Vec::new(),
            title: None,
        }
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

    /// The bar's height, where the system sets one for a title bar.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// Room kept at the very start of the bar, from the window's edge, for what the system draws there.
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }

    /// Dims what the bar holds, as a title bar does while its window is behind another.
    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    /// Sets the middle at the end of the room the start leaves, for a bar whose middle is its actions.
    pub fn apart(mut self) -> Self {
        self.apart = true;
        self
    }

    /// Makes the bar its window's title bar.
    pub fn title(mut self, title: Title<M>) -> Self {
        self.title = Some(title);
        self
    }

    /// Something ahead of the document, in every form of the bar: where a desktop that keeps its window
    /// buttons at the start has them.
    pub fn before(mut self, child: impl Into<Element<M>>) -> Self {
        self.before.push(child.into());
        self
    }

    pub fn start(mut self, child: impl Into<Element<M>>) -> Self {
        self.last().start.push(child.into());
        self
    }

    pub fn middle(mut self, child: impl Into<Element<M>>) -> Self {
        self.last().middle.push(child.into());
        self
    }

    /// Begins another form of the start and the middle, which the calls after this fill. The bar shows the
    /// first of its forms that fits, so give them from the widest to the narrowest. When not even the last
    /// fits, what may shrink at its start gives way, and then its middle is cut short.
    ///
    /// `key` is the form's place among all the bar's forms, which keeps its controls who they are when only
    /// some of the forms are built.
    pub fn form(mut self, key: usize) -> Self {
        self.forms.push(Form { key, start: Vec::new(), middle: Vec::new() });
        self
    }

    /// Tells `shown` which of the forms given fits, counted from the first.
    pub fn shown(mut self, shown: Fitted) -> Self {
        self.shown = Some(shown);
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

    fn last(&mut self) -> &mut Form<M> {
        if self.forms.is_empty() {
            self.forms.push(Form { key: 0, start: Vec::new(), middle: Vec::new() });
        }
        let last = self.forms.len() - 1;
        &mut self.forms[last]
    }
}

impl<M: 'static> From<TopBar<M>> for Element<M> {
    fn from(bar: TopBar<M>) -> Self {
        let opacity = if bar.dim { DIMMED } else { 1.0 };
        let group = || Element::row().align(Align::Center).gap(tokens::SPACE_SM);
        let justify = if bar.apart { Justify::Between } else { Justify::Start };
        let form = |form: Form<M>| {
            let start = group().shrink().children(form.start);
            let row = group().key(form.key).grow(1.0).shrink().justify(justify);
            row.child(start).child(group().children(form.middle))
        };
        let forms = Element::row().grow(1.0).shrink().clip().children(bar.forms.into_iter().map(form));
        let forms = match bar.shown {
            Some(shown) => forms.fit_into(shown),
            None => forms.fit(),
        };
        let padding = Edges::xy(tokens::SPACE_MD, 0.0);
        let row = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .height(bar.height)
            .padding(Edges { left: padding.left.max(bar.inset), ..padding })
            .bg(tokens::SURFACE)
            .fg(tokens::FG)
            .border(Edges::bottom(1.0), tokens::LINE)
            .role(Role::Toolbar)
            .label(bar.label)
            .arrows(Dir::Row)
            .children((!bar.before.is_empty()).then(|| group().opacity(opacity).children(bar.before)))
            .child(forms.opacity(opacity))
            .children((!bar.end.is_empty()).then(|| group().opacity(opacity).children(bar.end)));
        match bar.title {
            Some(title) => row.on_drag(title.drag).on_double_click(title.double_click).on_menu(title.menu),
            None => row,
        }
    }
}
