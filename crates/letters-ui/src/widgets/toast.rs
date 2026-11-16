//! Toast: a short notice that lies over the foot of a room for a moment, with at most one thing to do about
//! it. It takes no room itself, and what it lies over stays the pointer's wherever the toast is not.

use accesskit::Role;
use letters_render::Size;

use crate::element::Element;
use crate::icon::{self, Icon};
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::Button;

/// As wide as a toast gets, which is yewi's width for one.
const WIDTH: f32 = 384.0;
const ICON: f32 = 16.0;

pub struct Toast<M> {
    message: String,
    icon: Icon,
    action: Option<Element<M>>,
    on_dismiss: Option<M>,
    wraps: bool,
}

impl<M> Toast<M> {
    /// A toast that says `message`, in a few words: it keeps to one line.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), icon: icon::INFO, action: None, on_dismiss: None, wraps: false }
    }

    /// For a message that is a sentence: it runs over as many lines as it needs, and what can be done about
    /// it stands under it.
    pub fn wraps(mut self) -> Self {
        self.wraps = true;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// A small button after the message: the one thing to do about it.
    pub fn action(mut self, action: impl Into<Element<M>>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Adds a button that puts the toast away, and sends `message`. The toast does not go by itself: stop
    /// showing it.
    pub fn on_dismiss(mut self, message: M) -> Self {
        self.on_dismiss = Some(message);
        self
    }
}

impl<M: Clone + 'static> Toast<M> {
    /// The toast at the bottom end of a room of this size, as an overlay: it goes among the children of
    /// what fills the room, after whatever it lies over.
    pub fn over(self, room: Size) -> Element<M> {
        let style = TextStyle::control().size(tokens::TEXT_MD);
        // What can be done stands a little apart from what is said.
        let apart = Edges { right: if self.action.is_some() { tokens::SPACE_SM } else { 0.0 }, ..Edges::ZERO };
        let (mark, said, action) = match self.wraps {
            false => {
                let said = Element::text(self.message.clone(), style).shrink();
                (Element::icon(self.icon, ICON), Element::row().padding(apart).shrink().child(said), self.action)
            }
            true => {
                // The first line stands level with the button that puts the toast away.
                let level = |tall: f32| Edges::top(((tokens::CONTROL_HEIGHT_SM - tall) / 2.0).max(0.0));
                let said = Element::paragraph(self.message.clone(), style).grow(1.0).shrink();
                let under = self.action.map(|action| Element::row().child(action));
                let said = Element::column()
                    .grow(1.0)
                    .shrink()
                    .gap(tokens::SPACE_SM)
                    .padding(level((style.size * style.leading).round()))
                    .child(said)
                    .children(under);
                (Element::row().padding(level(ICON)).child(Element::icon(self.icon, ICON)), said, None)
            }
        };
        let padding =
            Edges { top: tokens::SPACE_SM, right: tokens::SPACE_SM, bottom: tokens::SPACE_SM, left: tokens::SPACE_MD };
        let mut toast = Element::row()
            .align(if self.wraps { Align::Start } else { Align::Center })
            .gap(tokens::SPACE_SM)
            .max_width(WIDTH)
            .padding(padding)
            .bg(tokens::SURFACE_RAISED)
            .fg(tokens::FG)
            .border(Edges::all(1.0), tokens::CARD_LINE)
            .radius(tokens::RADIUS_CARD)
            .shadow(tokens::SHADOW_OVERLAY)
            .role(Role::Status)
            .label(self.message)
            .child(mark.fg(tokens::FG_MUTED))
            .child(said)
            .children(action);
        if let Some(message) = self.on_dismiss {
            toast = toast.child(Button::icon_only(icon::X, "Dismiss").subtle().small().on_press(message));
        }
        Element::column()
            .overlay()
            .hollow()
            .width(room.width)
            .height(room.height)
            .padding(Edges::all(tokens::SPACE_LG))
            .align(Align::End)
            .justify(Justify::End)
            .child(toast)
    }
}
