//! What others wrote since the person last had the document open.
//!
//! The shell keeps the revision a person last saw of a document, outside the file, and says it when the
//! document opens. Where others wrote since, a toast names them and offers the history, with the document
//! as it reads now against the version last seen.

use std::cmp::Reverse;

use letters_model::{Point as Step, Revision};
use letters_render::Size;

use crate::element::Element;
use crate::icon;
use crate::threads;
use crate::widgets::{Button, Side, Toast};
use crate::window::Message;

/// What is asked about the last visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seeing {
    /// Shows what changed since: the history, with the document as it reads now against the version
    /// last seen.
    Show,
    /// Puts the toast away.
    Hide,
}

/// What the person last saw of the document, as the shell said it.
pub(crate) struct Seen {
    /// The revision the document read at.
    pub revision: Revision,
    /// Who else wrote since, whoever wrote most first.
    others: Vec<String>,
    /// Whether the toast still waits: it goes for good when it is answered, and when the person writes.
    noted: bool,
}

impl Seen {
    /// What was seen at `revision`, since when the document took `steps`. `me` is who writes here, whose
    /// own steps from elsewhere are no news.
    pub fn new(revision: Revision, steps: &[Step], me: &str) -> Self {
        let mut others: Vec<(&str, usize)> = Vec::new();
        for (name, edits) in steps.iter().flat_map(|step| &step.authors).filter(|(name, _)| name != me) {
            match others.iter_mut().find(|(known, _)| known == name) {
                Some((_, count)) => *count += edits,
                None => others.push((name, *edits)),
            }
        }
        others.sort_by_key(|(_, edits)| Reverse(*edits));
        let others = others.into_iter().map(|(name, _)| threads::named(name).to_owned()).collect();
        Self { revision, others, noted: true }
    }

    /// Puts the toast away for good.
    pub fn noted(&mut self) {
        self.noted = false;
    }

    /// The toast over the foot of a page of this size: who wrote, and a way to what they changed. `None`
    /// where no one else wrote, and once it was put away.
    pub fn toast(&self, room: Size) -> Option<Element<Message>> {
        // A toast keeps to one line, so it names one person and counts more.
        let (said, who) = match self.others.as_slice() {
            _ if !self.noted => return None,
            [] => return None,
            [one] => (format!("Edited by {one}"), one.clone()),
            [one, other] => ("Edited by 2 people".to_owned(), format!("{one} and {other}")),
            [one, rest @ ..] => {
                (format!("Edited by {} people", rest.len() + 1), format!("{one} and {} others", rest.len()))
            }
        };
        let does = format!("Shows what {who} changed since your last visit");
        let show = Button::new("See changes").small().on_press(Message::Seen(Seeing::Show));
        let show = Element::from(show).tooltip(does, Side::Top);
        let toast = Toast::new(said).icon(icon::HISTORY).action(show).on_dismiss(Message::Seen(Seeing::Hide));
        Some(toast.over(room))
    }
}
