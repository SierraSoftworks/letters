//! Segmented: a few choices side by side on one track, with the one in force raised on it. yewi's segmented
//! control.

use accesskit::Role;

use crate::element::Element;
use crate::style::{Align, Edges, Justify, Look, Ring};
use crate::text::TextStyle;
use crate::tokens::{self, Color as Ink};
use crate::widgets::ControlSize;

/// The gap between the track's edge and its segments.
const INSET: f32 = 2.0;

pub struct Segmented<M> {
    label: String,
    size: ControlSize,
    segments: Vec<(String, bool, M)>,
}

impl<M> Segmented<M> {
    /// A control with a name for assistive technology.
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), size: ControlSize::default(), segments: Vec::new() }
    }

    pub fn small(mut self) -> Self {
        self.size = ControlSize::Small;
        self
    }

    /// A choice, whether it is the one in force, and what a press on it sends.
    pub fn segment(mut self, label: impl Into<String>, chosen: bool, message: M) -> Self {
        self.segments.push((label.into(), chosen, message));
        self
    }
}

impl<M> From<Segmented<M>> for Element<M> {
    fn from(control: Segmented<M>) -> Self {
        let height = control.size.height();
        let style = TextStyle::control().size(control.size.text()).weight(500);
        // A well, darker than the chip of the choice in force.
        let well = Ink { light: tokens::TONE_1.light, dark: tokens::BG.dark };
        let mut track = Element::row()
            .align(Align::Center)
            .gap(INSET)
            .height(height)
            .padding(Edges::all(INSET))
            .border(Edges::all(1.0), tokens::LINE)
            .radius(tokens::RADIUS_CONTROL)
            .bg(well)
            .role(Role::RadioGroup)
            .label(control.label);
        for (label, chosen, message) in control.segments {
            let mut segment = Element::row()
                .align(Align::Center)
                .justify(Justify::Center)
                .height(height - 2.0 * (INSET + 1.0))
                .padding(Edges::xy(tokens::SPACE_MD - INSET, 0.0))
                .radius(tokens::RADIUS_CONTROL - INSET)
                .fg(if chosen { tokens::FG } else { tokens::FG_MUTED })
                .hover(Look::NONE.with_fg(tokens::FG))
                .ring(Ring::Outline(1.0))
                .role(Role::RadioButton)
                .label(label.as_str())
                .checked(chosen)
                .child(Element::text(label, style).decorative())
                .on_press(message);
            if chosen {
                segment = segment.bg(tokens::SURFACE_RAISED).shadow(tokens::SHADOW_CARD);
            }
            track = track.child(segment);
        }
        track
    }
}
