//! What a shell has the window say about something that happened once, such as unsaved changes that could
//! not be brought back: a sentence over the foot of the page with one button for what can be done about it.
//! It stays until the person presses the button or puts it away, and the page under it stays theirs.

use letters_render::Size;

use crate::element::Element;
use crate::widgets::{Button, Toast};
use crate::window::Message;

/// Something that happened once, for [`Window::notify`](crate::Window::notify).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notice {
    /// What happened, in a sentence that stands by itself.
    pub sentence: String,
    /// What the button about it is called: "Show file".
    pub button: String,
    /// What the shell knows the notice by. It comes back with the press, in
    /// [`Request::NoticePressed`](crate::Request::NoticePressed).
    pub key: String,
}

/// What is done about a notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Noticing {
    /// Its button was pressed.
    Press,
    /// Puts it away.
    Hide,
}

/// The notice over the foot of a page of this size.
pub(crate) fn toast(notice: &Notice, room: Size) -> Element<Message> {
    let does = Button::new(notice.button.as_str()).small().on_press(Message::Notice(Noticing::Press));
    Toast::new(notice.sentence.as_str()).wraps().action(does).on_dismiss(Message::Notice(Noticing::Hide)).over(room)
}
