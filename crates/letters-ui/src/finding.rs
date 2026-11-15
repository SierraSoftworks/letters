//! Find and replace: a bar that hangs under the top bar, at the page's end.
//!
//! The bar holds what is looked for and how, and counts the places it was found at. The page marks every
//! place, and the one that was gone to is selected, so it reads as the selection does. Enter in the field
//! goes to the next place and Shift with Enter to the one before. A second line replaces: the place that
//! is selected, or all of them in one step. The page under the bar is written on as ever, and what is
//! found follows what is written.

use accesskit::Role;
use letters_editor::{Command, Editor, Query, Seek};
use letters_model::{BlockId, Selection};
use letters_render::{DisplayList, Item, Rect};

use crate::color::{fade, resolve};
use crate::element::Element;
use crate::icon::{self, Icon};
use crate::style::{Align, Edges, Justify};
use crate::text::TextStyle;
use crate::tokens::{self, Scheme};
use crate::widgets::{Button, ControlSize, Popover, Side, TextField};
use crate::window::Message;

/// What the bar is called.
pub(crate) const LABEL: &str = "Find and replace";
/// How wide the bar is, where the page has the room.
const WIDTH: f32 = 500.0;
/// The width under which the ways of looking go to a line of their own.
const NARROW: f32 = 400.0;
/// The room kept between the bar and the edges of the page area.
const MARGIN: f32 = 12.0;
/// What the bar's surface takes round what it holds: its padding and its border, on both sides.
const AROUND: f32 = 2.0 * tokens::SPACE_SM + 2.0;
/// The room the count of places is given, so that the field keeps its width as the count changes.
const COUNT: f32 = 76.0;
/// The longest selection that is taken as what to look for when the bar opens, in bytes.
const TAKEN: usize = 200;
/// How strong the tint over a place is, as a share of the colour it is made from.
const TINT: f32 = 0.34;

/// One of the ways of looking that the bar switches on and off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    /// Capitals and small letters are told apart.
    Case,
    /// Only whole words are found.
    Word,
    /// What is typed is a regular expression.
    Regex,
}

/// What the keyboard and the pointer ask of the bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Finding {
    /// Shows the bar with the keyboard in its field, and with `replace` the line that replaces too.
    Open {
        replace: bool,
    },
    /// Escape, or the button that puts the bar away.
    Close,
    /// What the field holds that is looked for.
    Type(String),
    /// What the field holds that is put in its place.
    With(String),
    /// Switches a way of looking on, or off.
    Toggle(Way),
    /// Shows the line that replaces, or hides it.
    Expand,
    /// Goes to the next place, or with `back` to the one before.
    Step {
        back: bool,
    },
    /// Replaces at the place that is selected and goes on to the next.
    Replace,
    ReplaceAll,
}

/// What the window does for the bar, which has neither the page nor the editor to change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Does {
    /// Selects a place and brings it into view.
    Go(Selection),
    /// Runs a replacement. After one of a single place, the next place is gone to.
    Run(Command),
}

/// The bar's own state: what is looked for, and where it was found.
#[derive(Default)]
pub(crate) struct Find {
    open: bool,
    replacing: bool,
    query: Query,
    with: String,
    /// The places the query is found at, in the order of the document.
    found: Vec<Selection>,
    /// What is wrong with the query, when it is a pattern that does not read.
    bad: Option<String>,
    /// The place the bar last had selected.
    gone: Option<Selection>,
    /// How many times the bar was asked for. Its field is named by this, so that it takes the keyboard
    /// each time.
    asked: u32,
}

impl Find {
    pub fn open(&self) -> bool {
        self.open
    }

    /// Whether there is a place to go to.
    pub fn steps(&self) -> bool {
        self.open && !self.found.is_empty()
    }

    /// How much of the page's top the bar covers, with the room over it.
    pub fn cover(&self) -> f32 {
        if !self.open {
            return 0.0;
        }
        let lines = if self.replacing { 2.0 } else { 1.0 };
        MARGIN + 2.0 * tokens::SPACE_SM + lines * tokens::CONTROL_HEIGHT_SM + (lines - 1.0) * tokens::SPACE_XS
    }

    /// Looks again, after the query or the document changed.
    pub fn refresh(&mut self, editor: &Editor) {
        if !self.open {
            return;
        }
        (self.found, self.bad) = match editor.find(&self.query) {
            Ok(found) => (found, None),
            Err(bad) => (Vec::new(), Some(bad.0)),
        };
    }

    /// Which of the places is the selection.
    pub fn current(&self, selection: Option<&Selection>) -> Option<usize> {
        let selection = selection.filter(|selection| !selection.is_caret())?;
        self.found.iter().position(|found| found == selection)
    }

    /// The place to go to from the selection.
    pub fn seek(&self, editor: &Editor, seek: Seek) -> Option<Selection> {
        editor.seek(&self.found, seek).map(|at| self.found[at].clone())
    }

    /// The places in the part of the page that shows, but for the one that is selected. `band` is the
    /// stretch of the column a block takes up, and `view` the stretch in view.
    pub fn shown<'a>(
        &'a self,
        selection: Option<&'a Selection>,
        band: impl Fn(&BlockId) -> Option<(f32, f32)> + Copy + 'a,
        view: (f32, f32),
    ) -> impl Iterator<Item = &'a Selection> + 'a {
        // The places are in the order of the page, so those in view are found without asking about the rest.
        let found = if self.open { &self.found[..] } else { &[] };
        let above = |found: &Selection| band(&found.anchor.block).is_none_or(|(_, bottom)| bottom <= view.0);
        let first = found.partition_point(above);
        let reached = move |found: &&Selection| band(&found.anchor.block).is_none_or(|(top, _)| top < view.1);
        found[first..].iter().take_while(reached).filter(move |found| Some(*found) != selection)
    }

    /// Takes what was asked of the bar. Returns what is left for the window to do.
    pub fn update(&mut self, asked: Finding, editor: &Editor) -> Option<Does> {
        // Where what is typed is found from here on. Once it is found nowhere, the place that was selected
        // for it is let go of.
        let here = |find: &Self| {
            let left = || find.gone.as_ref().filter(|gone| editor.selection() == Some(*gone));
            let to = find.seek(editor, Seek::Here).or_else(|| left().map(|gone| Selection::caret(gone.anchor.clone())));
            to.map(Does::Go)
        };
        let does = match asked {
            Finding::Open { replace } => {
                self.open = true;
                self.replacing |= replace;
                self.asked += 1;
                // What is selected is what is looked for, and stays selected as the place that was found.
                if let Some(text) = selected(editor) {
                    self.query.text = text;
                }
                self.refresh(editor);
                None
            }
            Finding::Close => {
                (self.open, self.found, self.bad) = (false, Vec::new(), None);
                None
            }
            Finding::Type(text) => {
                self.query.text = text;
                self.refresh(editor);
                here(self)
            }
            Finding::With(with) => {
                self.with = with;
                None
            }
            Finding::Toggle(way) => {
                let on = match way {
                    Way::Case => &mut self.query.case,
                    Way::Word => &mut self.query.word,
                    Way::Regex => &mut self.query.regex,
                };
                *on = !*on;
                self.refresh(editor);
                here(self)
            }
            Finding::Expand => {
                self.replacing = !self.replacing;
                None
            }
            Finding::Step { back } => self.seek(editor, if back { Seek::Previous } else { Seek::Next }).map(Does::Go),
            Finding::Replace => match self.current(editor.selection()) {
                Some(at) => {
                    let (query, with) = (self.query.clone(), self.with.clone());
                    Some(Does::Run(Command::Replace { query, with, only: Some(self.found[at].clone()) }))
                }
                // The first press shows what the second replaces.
                None => here(self),
            },
            Finding::ReplaceAll => {
                let (query, with) = (self.query.clone(), self.with.clone());
                (!self.found.is_empty()).then_some(Does::Run(Command::Replace { query, with, only: None }))
            }
        };
        if let Some(Does::Go(to)) = &does {
            self.gone = Some(to.clone());
        }
        does
    }

    /// What the bar says of what was found.
    fn count(&self, current: Option<usize>) -> String {
        match (self.found.len(), current) {
            _ if self.query.text.is_empty() || self.bad.is_some() => String::new(),
            (0, _) => "No results".to_owned(),
            (all, Some(at)) => format!("{} of {all}", at + 1),
            (1, None) => "1 found".to_owned(),
            (all, None) => format!("{all} found"),
        }
    }

    /// The bar, hung `top` down the window with its end at `end`, in a page area `room` wide. `current` is
    /// the place that is selected. It hangs from an element of no size that the window puts before
    /// everything else, so that whatever else opens lies over it.
    pub fn view(&self, current: Option<usize>, top: f32, end: f32, room: f32) -> Element<Message> {
        let send = Message::Find;
        let width = WIDTH.min(room - 2.0 * MARGIN - AROUND).max(0.0);
        let narrow = width < NARROW;
        let none = self.found.is_empty();
        let tool = |icon: Icon, label: &str, tip: &str, message: Finding, off: bool| {
            let button = Button::icon_only(icon, label).subtle().small().disabled(off).on_press(send(message));
            Element::from(button).tooltip(tip, Side::Bottom)
        };
        let way = |icon: Icon, label: &str, way: Way, on: bool| {
            let button = Button::icon_only(icon, label).subtle().small().pressed(on);
            Element::from(button.on_press(send(Finding::Toggle(way)))).tooltip(label, Side::Bottom)
        };

        let field = TextField::new("Find", self.query.text.as_str())
            .placeholder(if self.query.regex { "Find a pattern" } else { "Find" })
            .size(ControlSize::Small)
            .invalid(self.bad.is_some())
            .on_input(move |typed| send(Finding::Type(typed)))
            .on_submit(send(Finding::Step { back: false }))
            .on_submit_back(send(Finding::Step { back: true }));
        let field = Element::from(field).key(("find", self.asked)).takes_focus().grow(1.0).shrink();
        let quiet = TextStyle::control().size(tokens::TEXT_SM);
        let said = self.count(current);
        let tone = if none { tokens::DANGER } else { tokens::FG_MUTED };
        let count = Element::row()
            .justify(Justify::End)
            .min_width(if narrow { 0.0 } else { COUNT })
            .child(Element::text(said, quiet).fg(tone).role(Role::Status));
        let (expand, told) = match self.replacing {
            true => (icon::CHEVRON_DOWN, "Hide replace"),
            false => (icon::CHEVRON_RIGHT, "Show replace"),
        };
        let expand = Button::icon_only(expand, told).subtle().small().on_press(send(Finding::Expand));
        let expand = Element::from(expand).expanded(self.replacing).tooltip(told, Side::Bottom);
        let ways = Element::row()
            .gap(1.0)
            .child(way(icon::CASE_SENSITIVE, "Match case", Way::Case, self.query.case))
            .child(way(icon::WHOLE_WORD, "Whole words", Way::Word, self.query.word))
            .child(way(icon::REGEX, "Regular expression", Way::Regex, self.query.regex));
        let steps = Element::row()
            .gap(1.0)
            .child(tool(icon::CHEVRON_UP, "Previous", "Previous (Shift+Enter)", Finding::Step { back: true }, none))
            .child(tool(icon::CHEVRON_DOWN, "Next", "Next (Enter)", Finding::Step { back: false }, none))
            .child(tool(icon::X, "Close", "Close (Esc)", Finding::Close, false));

        let line = || Element::row().align(Align::Center).gap(tokens::SPACE_XS);
        // What stands under the field starts where the field does.
        let indent = || Element::row().width(tokens::CONTROL_HEIGHT_SM).decorative();
        let mut lines = Element::column().gap(tokens::SPACE_XS).width(width);
        lines = match narrow {
            false => lines.child(line().child(expand).child(field).child(count).child(ways).child(steps)),
            true => lines
                .child(line().child(expand).child(field).child(steps))
                .child(line().child(indent()).child(ways).child(Element::spacer()).child(count)),
        };
        if let Some(bad) = &self.bad {
            let why = Element::paragraph(format!("Not a pattern: {bad}"), quiet).fg(tokens::DANGER).role(Role::Status);
            lines = lines.child(line().child(indent()).child(why.grow(1.0)));
        }
        if self.replacing {
            let with = TextField::new("Replace with", self.with.as_str())
                .placeholder("Replace with")
                .size(ControlSize::Small)
                .on_input(move |typed| send(Finding::With(typed)))
                .on_submit(send(Finding::Replace));
            let one = Button::new("Replace").small().disabled(none).on_press(send(Finding::Replace));
            let all = Button::new("Replace all").small().disabled(none).on_press(send(Finding::ReplaceAll));
            let with = Element::from(with).grow(1.0).shrink();
            lines = match narrow {
                false => lines.child(line().child(indent()).child(with).child(one).child(all)),
                true => {
                    lines.child(line().child(indent()).child(with)).child(line().child(indent()).child(one).child(all))
                }
            };
        }
        let bar = Popover::new(Element::column().padding(Edges::all(tokens::SPACE_SM)).child(lines))
            .label(LABEL)
            .flush()
            .max_width(width + AROUND)
            .at(Rect::new(end - MARGIN, top + MARGIN / 2.0, 0.0, 0.0))
            .align(Align::End)
            .gap(0.0)
            .stays()
            .open(true)
            .on_close(send(Finding::Close));
        Element::row().overlay().popover(bar)
    }
}

/// The selection as something to look for: text of one block and one line, and not too much of it.
fn selected(editor: &Editor) -> Option<String> {
    let selection = editor.selection().filter(|selection| selection.anchor.block == selection.head.block)?;
    let (from, to) =
        (selection.anchor.offset.min(selection.head.offset), selection.anchor.offset.max(selection.head.offset));
    let text = editor.doc().block(&selection.head.block)?.plain_text();
    let text = text.get(from..to).filter(|text| !text.is_empty() && text.len() <= TAKEN && !text.contains('\n'))?;
    Some(text.to_owned())
}

/// Marks the lines of a place the query was found at, under its text.
pub(crate) fn mark(lines: &[Rect], scheme: Scheme, out: &mut DisplayList) {
    let tint = resolve(fade(tokens::WARN, TINT), scheme, 1.0);
    for line in lines.iter().filter(|line| line.width > 0.0) {
        out.push(Item::Fill { rect: *line, color: tint, radius: 2.0 });
    }
}
