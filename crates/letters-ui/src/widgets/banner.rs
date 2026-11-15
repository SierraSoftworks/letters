//! Banner: a strip across the window that says how things stand, with what can be done about it at its end.

use accesskit::Role;

use crate::color::fade;
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::style::{Align, Edges};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, Tone};

/// How tall a banner is. It keeps to one line, so what stands under it knows the room it has left.
pub const HEIGHT: f32 = tokens::CONTROL_HEIGHT_SM + 2.0 * tokens::SPACE_SM + 1.0;

const ICON: f32 = 16.0;

pub struct Banner<M> {
    message: String,
    tone: Tone,
    icon: Icon,
    actions: Vec<Element<M>>,
    on_dismiss: Option<(String, M)>,
}

impl<M> Banner<M> {
    /// A banner that says `message`: a sentence that leads with what matters, since a narrow window cuts
    /// its end short.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), tone: Tone::Default, icon: icon::INFO, actions: Vec::new(), on_dismiss: None }
    }

    /// The colour, and how urgently it is announced. The default is the colour of a plain notice.
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// A small button at the end of the strip.
    pub fn action(mut self, action: impl Into<Element<M>>) -> Self {
        self.actions.push(action.into());
        self
    }

    /// Adds a button named `label` after the actions that puts the banner away, and sends `message`. The
    /// banner does not go by itself: stop showing it.
    pub fn on_dismiss(mut self, label: impl Into<String>, message: M) -> Self {
        self.on_dismiss = Some((label.into(), message));
        self
    }
}

impl<M: Clone + 'static> From<Banner<M>> for Element<M> {
    fn from(banner: Banner<M>) -> Self {
        let (ink, ground, role) = match banner.tone {
            Tone::Default => (tokens::INFO, tokens::INFO_SOFT, Role::Status),
            Tone::Warn => (tokens::WARN, tokens::WARN_SOFT, Role::Alert),
            Tone::Danger => (tokens::DANGER, tokens::DANGER_SOFT, Role::Alert),
        };
        let said = Element::text(banner.message.clone(), TextStyle::control().size(tokens::TEXT_MD));
        let mut strip = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .height(HEIGHT)
            .padding(Edges::xy(tokens::SPACE_LG, 0.0))
            .bg(ground)
            .fg(tokens::FG)
            .border(Edges::bottom(1.0), fade(ink, 0.28))
            .role(role)
            .label(banner.message)
            .child(Element::icon(banner.icon, ICON).fg(ink))
            .child(said.grow(1.0).shrink())
            .children(banner.actions);
        if let Some((label, message)) = banner.on_dismiss {
            strip = strip.child(Button::icon_only(icon::X, label).subtle().small().on_press(message));
        }
        strip
    }
}
