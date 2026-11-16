//! What the shell keeps for a person besides their settings: the folders and sources they trust, whether
//! they trust the document that is open, and the words of their own dictionary.
//!
//! The Settings dialog lists each once the shell has said it, and asks the shell to add to it or to forget.
//! Trust is the shell's to keep, so a list changes when the shell says it again.

use std::collections::BTreeSet;

use accesskit::Role;

use crate::element::Element;
use crate::icon::{self, Icon};
use crate::opening::Request;
use crate::settings::{Setting, quiet, row};
use crate::style::{Align, Edges};
use crate::text::TextStyle;
use crate::tokens;
use crate::widgets::{Button, SidePanel, TextField};
use crate::window::Message;

/// The tallest a list grows before it scrolls.
const TALLEST: f32 = 164.0;
/// What the field for a source is called, and what it shows while it is empty.
pub(crate) const SOURCE: &str = "Trust a source";
const EXAMPLE: &str = "img.example.org";
/// Why a source has nothing to remove it with.
const FIXED: &str = "Set by your administrator";

/// A folder whose documents the person trusts, as the shell says with `Window::set_trusted_folders`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedFolder {
    /// The shell's own path of it, which comes back unchanged in [`Request::ForgetFolder`].
    pub path: String,
    /// The folder as the person reads it, such as `~/Documents/Letters`.
    pub shown: String,
}

/// A place linked pictures load from when a trusted document opens, as the shell says with
/// `Window::set_trusted_sources`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedSource {
    /// The source as it is written, such as `https://img.example.org/charts/`. It comes back unchanged in
    /// [`Request::ForgetSource`].
    pub source: String,
    /// Whether an administrator set it for everyone. The person cannot remove it, and the list says why.
    pub fixed: bool,
}

/// What the shell said it keeps, and the source that is being typed.
#[derive(Default)]
pub(crate) struct Kept {
    /// `None` until the shell says, and the dialog then lists none.
    pub folders: Option<Vec<TrustedFolder>>,
    pub sources: Option<Vec<TrustedSource>>,
    /// Whether the person trusts the document that is open, where the shell says.
    pub document: Option<bool>,
    /// What is typed into the field for a source, whether the shell was asked to trust it and has not
    /// answered, and what it said if it would not.
    typed: String,
    asked: bool,
    refused: Option<String>,
}

impl Kept {
    /// Takes the sources the shell trusts now. That answers a source the person asked for, whose field
    /// empties. Returns whether one of them is new to a list that was said before.
    pub fn set_sources(&mut self, sources: Vec<TrustedSource>) -> bool {
        if std::mem::take(&mut self.asked) {
            self.typed.clear();
        }
        let before = self.sources.replace(sources);
        let now = self.sources.iter().flatten();
        before.is_some_and(|before| now.into_iter().any(|source| !before.contains(source)))
    }

    /// The shell does not trust the source that was asked for. What was typed stays, to be mended.
    pub fn refuse(&mut self, why: &str) {
        (self.refused, self.asked) = (Some(why.to_owned()), false);
    }

    /// Follows what the dialog sent. Returns what it asks of the shell.
    pub fn asks(&mut self, message: &Setting) -> Option<Request> {
        match message {
            Setting::TrustFolder => Some(Request::TrustFolderDialog),
            Setting::ForgetFolder(path) => Some(Request::ForgetFolder(path.clone())),
            Setting::ForgetSource(source) => Some(Request::ForgetSource(source.clone())),
            Setting::ForgetDocument => Some(Request::ForgetDocument),
            Setting::ForgetWord(word) => Some(Request::ForgetWord(word.clone())),
            Setting::Source(typed) => {
                (self.typed, self.refused) = (typed.clone(), None);
                None
            }
            Setting::TrustSource => {
                let source = self.typed.trim();
                // One question at a time: the answer empties the field, or says what is wrong with it.
                let ask = !source.is_empty() && !self.asked;
                ask.then(|| {
                    (self.asked, self.refused) = (true, None);
                    Request::TrustSource(source.to_owned())
                })
            }
            // What was typed and not asked for goes with the dialog.
            Setting::Close => {
                (self.typed, self.refused) = (String::new(), None);
                None
            }
            _ => None,
        }
    }

    /// The part of the dialog about trust, if the shell said anything of it.
    pub fn trust(&self) -> Option<Element<Message>> {
        let document = self.document.filter(|trusted| *trusted).map(|_| document());
        let folders = self.folders.as_deref().map(folders);
        let sources = self.sources.as_deref().map(|sources| self.sources(sources));
        let rows: Vec<_> = document.into_iter().chain(folders).chain(sources).collect();
        (!rows.is_empty()).then(|| section("Trust", rows))
    }

    /// The sources, and the field that adds one.
    fn sources(&self, sources: &[TrustedSource]) -> Element<Message> {
        let send = |setting: Setting| Message::Settings(setting);
        let rows = sources.iter().map(|source| {
            let said = source.source.as_str();
            match source.fixed {
                true => entry(None, said, Some(FIXED), None).label(format!("{said}, {}", FIXED.to_lowercase())),
                false => {
                    let stop = remove(format!("Stop trusting {said}"), Setting::ForgetSource(said.into()));
                    entry(None, said, None, Some(stop))
                }
            }
        });
        let field = TextField::new(SOURCE, self.typed.clone())
            .placeholder(EXAMPLE)
            .invalid(self.refused.is_some())
            .on_input(|typed| Message::Settings(Setting::Source(typed)))
            .on_submit(send(Setting::TrustSource));
        let idle = self.asked || self.typed.trim().is_empty();
        let add = Button::new("Trust").disabled(idle).on_press(send(Setting::TrustSource));
        let add = Element::row().gap(tokens::SPACE_SM).child(Element::from(field).grow(1.0)).child(add);
        let refused = self
            .refused
            .as_deref()
            .map(|why| Element::paragraph(why, quiet()).fg(tokens::DANGER).role(Role::Alert).label(why).shrink());
        let control = Element::column()
            .gap(tokens::SPACE_SM)
            .shrink()
            .child(list("Trusted sources", rows.collect(), "You trust no source yet."))
            .child(add)
            .children(refused);
        row("Sources", control, Some("In a document you trust, linked pictures from these load when it opens."))
    }
}

/// The part of the dialog about the person's own words, once the shell has said them.
pub(crate) fn dictionary(words: &BTreeSet<String>) -> Element<Message> {
    // As a person reads a list of words: a capital is no reason to come first.
    let mut words: Vec<&String> = words.iter().collect();
    words.sort_by_key(|word| word.to_lowercase());
    let rows = words.into_iter().map(|word| {
        let take = remove(format!("Remove {word}"), Setting::ForgetWord(word.clone()));
        entry(None, word, None, Some(take))
    });
    let about = "Words you added with \u{201c}Add to my dictionary\u{201d}. They are right in every document.";
    let words = row("Your words", list("Your words", rows.collect(), "You have added no words."), Some(about));
    section("Dictionary", vec![words])
}

/// The way out of the trust that the bar over a held document gives.
fn document() -> Element<Message> {
    let stop = Button::new("Stop trusting").on_press(Message::Settings(Setting::ForgetDocument));
    let stop = Element::from(stop).label("Stop trusting this document");
    let said = Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .grow(1.0)
        .child(Element::icon(icon::SHIELD, 14.0).fg(tokens::FG_MUTED).decorative())
        .child(Element::text("You trust this document", TextStyle::control()));
    let control = Element::row().align(Align::Center).gap(tokens::SPACE_SM).child(said).child(stop);
    row("This document", control, Some("Stop, and safe mode holds its linked pictures and links back again."))
}

/// The folders, and the button that lets the person choose another.
fn folders(folders: &[TrustedFolder]) -> Element<Message> {
    let rows = folders.iter().map(|folder| {
        let stop = remove(format!("Stop trusting {}", folder.shown), Setting::ForgetFolder(folder.path.clone()));
        entry(Some(icon::FOLDER), &folder.shown, None, Some(stop))
    });
    let add = Button::new("Add a folder").icon(icon::FOLDER_PLUS).on_press(Message::Settings(Setting::TrustFolder));
    let control = Element::column()
        .gap(tokens::SPACE_SM)
        .shrink()
        .child(list("Trusted folders", rows.collect(), "You trust no folder yet."))
        .child(Element::row().child(add));
    row("Folders", control, Some("Every document in these folders, and in the folders inside them, opens trusted."))
}

fn section(name: &str, rows: Vec<Element<Message>>) -> Element<Message> {
    Element::column().child(SidePanel::section(name)).child(Element::column().gap(tokens::SPACE_MD).children(rows))
}

/// A list in a frame, which scrolls once it is long, or a line that says it is empty. It is as wide as its
/// setting has room for, whatever it holds: see `entry`.
fn list(name: &str, rows: Vec<Element<Message>>, none: &str) -> Element<Message> {
    let frame = Element::column().border(Edges::all(1.0), tokens::LINE).radius(tokens::RADIUS_CONTROL).shrink();
    if rows.is_empty() {
        let none = Element::text(none, TextStyle::control()).fg(tokens::FG_MUTED);
        let none = Element::row().align(Align::Center).height(tokens::CONTROL_HEIGHT).child(none);
        return frame.padding(Edges::xy(tokens::SPACE_MD, 0.0)).child(none);
    }
    // A line parts each row from the one over it. Rows are told apart by their place, so that the keyboard
    // stays in the list when the row it was on goes: it is then on the row that took the place.
    let rows = rows.into_iter().enumerate().map(|(at, row)| match at {
        0 => row.key(at),
        _ => row.key(at).border(Edges::top(1.0), tokens::LINE),
    });
    frame.role(Role::List).label(name).max_height(TALLEST).scroll().children(rows)
}

/// One row of a list: what it holds, a word about it under that, and at its end the button that takes it out.
fn entry(mark: Option<Icon>, said: &str, about: Option<&str>, end: Option<Element<Message>>) -> Element<Message> {
    let mark = mark.map(|mark| Element::icon(mark, 14.0).fg(tokens::FG_MUTED).decorative());
    let lines = if about.is_some() { tokens::SPACE_XS } else { 0.0 };
    let about = about.map(|about| Element::text(about, quiet()).fg(tokens::FG_MUTED).shrink());
    // Something too long for the row is cut short: the whole of it is the row's name.
    let text = Element::text(said, TextStyle::control()).shrink();
    let text = Element::column().grow(1.0).shrink().child(text).children(about);
    Element::row()
        .align(Align::Center)
        .gap(tokens::SPACE_SM)
        .min_height(tokens::CONTROL_HEIGHT)
        .padding(Edges { left: tokens::SPACE_MD, right: tokens::SPACE_XS, top: lines, bottom: lines })
        .role(Role::ListItem)
        .label(said)
        .children(mark)
        .child(text)
        .children(end)
}

fn remove(does: String, setting: Setting) -> Element<Message> {
    let remove = Button::icon_only(icon::X, does).subtle().small().on_press(Message::Settings(setting));
    Element::from(remove).key("remove")
}
