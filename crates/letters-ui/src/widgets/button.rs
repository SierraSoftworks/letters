//! Button, and its square icon-only form.

use accesskit::Role;

use crate::color::{blend, fade, mix};
use crate::element::Element;
use crate::icon::Icon;
use crate::style::{Align, Edges, Justify, Look, Ring};
use crate::text::TextStyle;
use crate::theme::accent;
use crate::tokens;
use crate::widgets::{Menu, Popover};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    /// An outlined button on the surface.
    #[default]
    Default,
    /// The one filled action of a view.
    Primary,
    /// No outline or fill until hovered, for toolbars.
    Subtle,
    Danger,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ControlSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ControlSize {
    pub(crate) fn height(self) -> f32 {
        match self {
            ControlSize::Small => tokens::CONTROL_HEIGHT_SM,
            ControlSize::Medium => tokens::CONTROL_HEIGHT,
            ControlSize::Large => tokens::CONTROL_HEIGHT_LG,
        }
    }

    pub(crate) fn text(self) -> f32 {
        if self == ControlSize::Small { tokens::TEXT_SM } else { tokens::TEXT_MD }
    }
}

pub struct Button<M> {
    label: String,
    icon: Option<Icon>,
    mark: Option<(Icon, tokens::Color)>,
    icon_only: bool,
    variant: Variant,
    size: ControlSize,
    pressed: Option<bool>,
    disabled: bool,
    shrink: bool,
    on_press: Option<M>,
    popover: Option<Popover<M>>,
}

impl<M> Button<M> {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            mark: None,
            icon_only: false,
            variant: Variant::Default,
            size: ControlSize::Medium,
            pressed: None,
            disabled: false,
            shrink: false,
            on_press: None,
            popover: None,
        }
    }

    /// A square button that shows only an icon. `label` is its accessible name.
    pub fn icon_only(icon: Icon, label: impl Into<String>) -> Self {
        Self { icon: Some(icon), icon_only: true, ..Self::new(label) }
    }

    /// An icon before the label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A second icon drawn over the first in a colour of its own, as the plus on a button that adds.
    pub fn mark(mut self, mark: Icon, color: tokens::Color) -> Self {
        self.mark = Some((mark, color));
        self
    }

    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }

    pub fn primary(self) -> Self {
        self.variant(Variant::Primary)
    }

    pub fn subtle(self) -> Self {
        self.variant(Variant::Subtle)
    }

    pub fn danger(self) -> Self {
        self.variant(Variant::Danger)
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    pub fn small(self) -> Self {
        self.size(ControlSize::Small)
    }

    /// Makes this a toggle button and says whether it is on.
    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = Some(pressed);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Lets the button give up the end of its label, for an ellipsis, where its row has too little room.
    pub fn shrink(mut self) -> Self {
        self.shrink = true;
        self
    }

    pub fn on_press(mut self, message: M) -> Self {
        self.on_press = Some(message);
        self
    }

    /// Opens a popover when pressed.
    pub fn popover(mut self, popover: Popover<M>) -> Self {
        self.popover = Some(popover);
        self
    }

    /// Opens a menu when pressed.
    pub fn menu(self, menu: Menu<M>) -> Self {
        self.popover(menu.popover())
    }
}

impl<M> From<Button<M>> for Element<M> {
    fn from(button: Button<M>) -> Self {
        // The component tokens of yewi's button.scss, per variant.
        let accent = accent();
        let (bg, bg_hover, fg, border) = match (button.pressed, button.variant) {
            (Some(true), variant) if variant != Variant::Primary => (
                Some(accent.soft),
                fade(accent.base, 0.18),
                accent.text,
                Some(blend(tokens::SURFACE, accent.base, 0.45)),
            ),
            (_, Variant::Default) => (Some(tokens::SURFACE), tokens::TONE_1, tokens::FG, Some(tokens::LINE_STRONG)),
            (_, Variant::Primary) => (Some(accent.solid), accent.solid_hover, tokens::ON_SOLID, None),
            (_, Variant::Subtle) => (None, tokens::NEUTRAL_SOFT, tokens::FG_MUTED, None),
            (_, Variant::Danger) => {
                (Some(tokens::SURFACE), tokens::DANGER_SOFT, tokens::DANGER, Some(fade(tokens::DANGER, 0.45)))
            }
        };
        let mut hover = Look::bg(bg_hover);
        if button.variant == Variant::Subtle && button.pressed != Some(true) {
            hover = hover.with_fg(tokens::FG);
        }

        let height = button.size.height();
        let text = button.size.text();
        let padding = match (button.icon_only, button.size) {
            (true, _) => 0.0,
            (_, ControlSize::Small) => tokens::CONTROL_PADDING - 4.0,
            (_, ControlSize::Medium) => tokens::CONTROL_PADDING,
            (_, ControlSize::Large) => tokens::CONTROL_PADDING + 4.0,
        };

        let mut element = Element::row()
            .align(Align::Center)
            .justify(Justify::Center)
            .gap((text * 0.4).round())
            .height(height)
            .padding(Edges::xy(padding, 0.0))
            .radius(tokens::RADIUS_CONTROL)
            .fg(fg)
            .hover(hover)
            .active(Look::bg(mix(bg_hover, tokens::FG, 0.06)))
            .ring(Ring::Outline(2.0))
            .role(Role::Button)
            .disabled(button.disabled);
        // The border is always there, so every variant measures the same.
        element.style.border = Edges::all(1.0);
        element.style.border_color = border;
        element.style.bg = bg;
        if button.icon_only {
            element = element.width(height).label(button.label.clone());
        }
        if button.disabled {
            element = element.opacity(tokens::DISABLED_OPACITY);
        }
        if let Some(pressed) = button.pressed {
            element = element.pressed(pressed);
        }
        if let Some(icon) = button.icon {
            let size = (text * 1.15).round();
            let mark = button.mark.map(|(mark, color)| Element::icon(mark, size).fg(color).overlay());
            element = element.child(Element::row().child(Element::icon(icon, size)).children(mark));
        }
        if !button.icon_only {
            let label = Element::text(button.label, TextStyle::control().size(text).weight(500));
            element = element.child(if button.shrink { label.shrink() } else { label });
        }
        if button.shrink {
            element = element.shrink();
        }
        if let Some(message) = button.on_press {
            element = element.on_press(message);
        }
        if let Some(popover) = button.popover {
            element = element.popover(popover);
        }
        element
    }
}
