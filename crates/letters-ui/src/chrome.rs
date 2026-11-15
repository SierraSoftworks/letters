//! The window's frame and its controls, where the bar is the window's title bar.
//!
//! The shell says what the desktop expects with a [`Chrome`], and does what the bar asks of it through
//! [`Request`]: the window only ever asks.

use accesskit::Role;
use letters_render::{Point, Size};

use crate::element::Element;
use crate::icon;
use crate::opening::Request;
use crate::style::{Align, Look};
use crate::tokens;
use crate::widgets::Button;
use crate::window::Message;

/// How far inside its edge a floating window can be taken hold of to resize it, and how far along an edge
/// a corner reaches.
const GRIP: f32 = 6.0;
const CORNER: f32 = 12.0;

/// One of the buttons a desktop puts on a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowButton {
    Minimise,
    /// Shown as Restore while the window is maximised.
    Maximise,
    Close,
}

/// Who draws the window's frame, and whether the bar is its title bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Frame {
    /// The system draws the frame and a title bar of its own, as it does around a browser's page.
    #[default]
    System,
    /// The system draws the frame and the bar is the title bar under it, as on macOS.
    Unified,
    /// Letters draws the frame of a window that floats: a 1 px border, with a grip inside it to resize by.
    Floating,
    /// Letters draws the frame of a window that fills its place, maximised, tiled or full screen: no border
    /// and no grip.
    Filled,
}

/// An edge or a corner of the window, by its compass point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

/// What the desktop expects of the window's frame and bar. The default is a window the system frames and
/// titles, which is also what a shell that never says gets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chrome {
    /// The buttons the bar shows, in the desktop's order. None where the system shows its own.
    pub buttons: Vec<WindowButton>,
    /// Whether the desktop keeps the buttons at the start of the bar.
    pub buttons_at_start: bool,
    /// The bar's height where the system sets one, as macOS does for a unified title bar.
    pub bar_height: Option<f32>,
    /// Room the bar leaves at its start for buttons the system draws over it, from the window's edge.
    pub inset_start: f32,
    pub frame: Frame,
    /// Whether the window is maximised, which turns Maximise into Restore.
    pub maximized: bool,
}

impl Chrome {
    /// Whether the bar is the window's title bar: it moves the window, and dims with it.
    pub(crate) fn titled(&self) -> bool {
        self.frame != Frame::System
    }

    /// The width of the border Letters draws round the window.
    pub(crate) fn border(&self) -> f32 {
        if self.frame == Frame::Floating { 1.0 } else { 0.0 }
    }

    /// The edge a point of a floating window's grip resizes from.
    pub(crate) fn edge(&self, at: Point, window: Size) -> Option<Edge> {
        if self.frame != Frame::Floating {
            return None;
        }
        let near =
            |reach: f32| (at.y < reach, at.y >= window.height - reach, at.x < reach, at.x >= window.width - reach);
        let ((north, south, west, east), corner) = (near(GRIP), near(CORNER));
        if !(north || south || west || east) {
            return None;
        }
        Some(match corner {
            (true, _, true, _) => Edge::NorthWest,
            (true, _, _, true) => Edge::NorthEast,
            (_, true, true, _) => Edge::SouthWest,
            (_, true, _, true) => Edge::SouthEast,
            _ if north => Edge::North,
            _ if south => Edge::South,
            _ if west => Edge::West,
            _ => Edge::East,
        })
    }

    /// The window's buttons as the bar shows them, if it shows any.
    pub(crate) fn buttons(&self) -> Option<Element<Message>> {
        let button = |button: &WindowButton| {
            let (icon, label, request) = match button {
                WindowButton::Minimise => (icon::MINUS, "Minimise", Request::Minimise),
                WindowButton::Maximise if self.maximized => (icon::COPY, "Restore", Request::ToggleMaximise),
                WindowButton::Maximise => (icon::SQUARE, "Maximise", Request::ToggleMaximise),
                WindowButton::Close => (icon::X, "Close window", Request::Close),
            };
            let shown = Element::from(Button::icon_only(icon, label).subtle().small().on_press(Message::Ask(request)));
            // Close is the one button with a colour of its own under the pointer.
            match button {
                WindowButton::Close => shown.hover(Look::bg(tokens::DANGER_SOFT).with_fg(tokens::DANGER)),
                _ => shown,
            }
        };
        let buttons = Element::row().align(Align::Center).gap(tokens::SPACE_2XS).role(Role::Group).label("Window");
        (!self.buttons.is_empty()).then(|| buttons.children(self.buttons.iter().map(button)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Size = Size { width: 800.0, height: 600.0 };

    #[test]
    fn a_floating_window_is_resized_from_just_inside_its_edge() {
        let chrome = Chrome { frame: Frame::Floating, ..Chrome::default() };
        let cases = [
            ((400.0, 2.0), Some(Edge::North)),
            ((400.0, 597.0), Some(Edge::South)),
            ((3.0, 300.0), Some(Edge::West)),
            ((798.0, 300.0), Some(Edge::East)),
            ((2.0, 2.0), Some(Edge::NorthWest)),
            ((10.0, 3.0), Some(Edge::NorthWest)),
            ((797.0, 10.0), Some(Edge::NorthEast)),
            ((3.0, 590.0), Some(Edge::SouthWest)),
            ((790.0, 598.0), Some(Edge::SouthEast)),
            ((10.0, 10.0), None),
            ((400.0, 300.0), None),
            ((400.0, 8.0), None),
        ];
        for ((x, y), expected) in cases {
            assert_eq!(chrome.edge(Point { x, y }, WINDOW), expected, "at {x}, {y}");
        }
    }

    #[test]
    fn only_a_floating_window_has_a_grip() {
        for frame in [Frame::System, Frame::Unified, Frame::Filled] {
            let chrome = Chrome { frame, ..Chrome::default() };
            assert_eq!(chrome.edge(Point { x: 1.0, y: 1.0 }, WINDOW), None, "{frame:?}");
        }
    }
}
