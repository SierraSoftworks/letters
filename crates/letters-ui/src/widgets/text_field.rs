//! TextField: yewi's one-line text input.

use std::rc::Rc;

use accesskit::Role;

use crate::color::mix;
use crate::element::{Element, Input};
use crate::style::{Edges, Look, Ring};
use crate::text::{Family, TextStyle};
use crate::theme::accent;
use crate::tokens;
use crate::widgets::ControlSize;

pub struct TextField<M> {
    value: String,
    label: String,
    placeholder: String,
    size: ControlSize,
    mono: bool,
    weight: Option<u16>,
    invalid: bool,
    disabled: bool,
    on_input: Option<Rc<dyn Fn(String) -> M>>,
    on_submit: Option<M>,
    on_leave: Option<M>,
    on_cancel: Option<M>,
}

impl<M> TextField<M> {
    /// A field showing `value`. `label` is its accessible name: every field needs one.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            placeholder: String::new(),
            size: ControlSize::Medium,
            mono: false,
            weight: None,
            invalid: false,
            disabled: false,
            on_input: None,
            on_submit: None,
            on_leave: None,
            on_cancel: None,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn size(mut self, size: ControlSize) -> Self {
        self.size = size;
        self
    }

    /// Sets the value in the code face, for addresses and identifiers.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The weight the value is set in, for a field that stands where a title did.
    pub fn weight(mut self, weight: u16) -> Self {
        self.weight = Some(weight);
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the whole new value on every edit. The field shows what the app then passes back as
    /// `value`, and without this it is read-only.
    pub fn on_input(mut self, handler: impl Fn(String) -> M + 'static) -> Self {
        self.on_input = Some(Rc::new(handler));
        self
    }

    /// Sent when Enter is pressed in the field.
    pub fn on_submit(mut self, message: M) -> Self {
        self.on_submit = Some(message);
        self
    }

    /// Sent when focus leaves the field, or the popover it is in closes under it: the moment to take or drop
    /// what was typed.
    pub fn on_leave(mut self, message: M) -> Self {
        self.on_leave = Some(message);
        self
    }

    /// Sent when Escape is pressed in the field, before anything else Escape does: the moment to drop what
    /// was typed. Focus then goes back to where it rests, unless Escape closed a popover.
    pub fn on_cancel(mut self, message: M) -> Self {
        self.on_cancel = Some(message);
        self
    }
}

impl<M> From<TextField<M>> for Element<M> {
    fn from(field: TextField<M>) -> Self {
        let padding = match field.size {
            ControlSize::Small => tokens::CONTROL_PADDING - 6.0,
            ControlSize::Medium => tokens::CONTROL_PADDING - 4.0,
            ControlSize::Large => tokens::CONTROL_PADDING,
        };
        let mut style = TextStyle::body().size(field.size.text());
        if field.mono {
            style = style.family(Family::Mono).size(tokens::TEXT_SM);
        }
        if let Some(weight) = field.weight {
            style = style.weight(weight);
        }
        let read_only = field.on_input.is_none();
        let (border, accent) =
            if field.invalid { (tokens::DANGER, tokens::DANGER) } else { (tokens::LINE_STRONG, accent().focus()) };
        // A field that cannot be used steps back with a tonal fill, not by fading.
        let (bg, fg) = match (field.disabled, read_only) {
            (true, _) => (tokens::TONE_1, tokens::FG_MUTED),
            (false, true) => (tokens::TONE_2, tokens::FG),
            (false, false) => (tokens::SURFACE, tokens::FG),
        };
        let mut element = Element::input(Input {
            value: field.value,
            placeholder: field.placeholder,
            style,
            on_input: field.on_input,
            on_submit: field.on_submit,
            on_leave: field.on_leave,
            on_cancel: field.on_cancel,
        })
        .height(field.size.height())
        .padding(Edges::xy(padding, 0.0))
        .border(Edges::all(1.0), border)
        .radius(tokens::RADIUS_CONTROL)
        .bg(bg)
        .fg(fg)
        .focused(Look::NONE.with_border(accent))
        .ring(Ring::Field)
        .role(Role::TextInput)
        .label(field.label)
        .disabled(field.disabled);
        if !field.disabled && !read_only && !field.invalid {
            element = element.hover(Look::NONE.with_border(mix(tokens::LINE_STRONG, tokens::FG, 0.3)));
        }
        element
    }
}
