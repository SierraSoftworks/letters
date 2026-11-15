//! Safe mode: the bar over a document from outside, which is held back until the person trusts it, and the
//! card about a link that the shell does not open on a press alone.
//!
//! The window only shows and asks. The shell knows what the person trusts, says when a document is held
//! back, and judges every link the person asks to open.

use accesskit::Role;
use letters_layout::DocLayout;
use letters_model::{BlockId, Document, Pos, Selection};
use letters_render::{Point, Rect};

use crate::element::Element;
use crate::icon;
use crate::style::{Align, Edges, Justify};
use crate::text::{Family, TextStyle};
use crate::tokens;
use crate::widgets::{Banner, Button, Popover, Side, banner};
use crate::window::Message;

/// How wide the card about a link is, inside its padding.
const CARD_WIDTH: f32 = 400.0;
/// How many lines of an address the card shows before it leaves the middle out.
const LINES: usize = 8;
/// What the bar says, with what matters first: a narrow window cuts its end short.
const HELD: &str = "Safe mode: linked pictures and links are held back until you trust this document.";

/// How a document in safe mode can be trusted, as the shell says with `Window::set_safe_mode`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SafeMode {
    /// The name of the folder the document is in, when trusting that folder would trust the document. The
    /// bar then offers "Trust this folder" and says which. `None` for a document that no folder vouches
    /// for, such as one that was imported.
    pub folder: Option<String>,
}

/// What the pointer and the keyboard ask about safe mode and links.
#[derive(Clone, Debug, PartialEq)]
pub enum Safely {
    TrustDocument,
    TrustFolder,
    /// Puts the bar away, to a button on the top bar.
    Hide,
    /// Brings the bar back.
    Show,
    /// Opens the link with this address, if the shell lets it.
    Follow(String),
    /// The person has read the address on the card and wants it opened.
    Confirm,
    /// Puts the card away.
    Leave,
}

/// What the shell said of a link it did not open.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Verdict {
    Ask,
    Refused(String),
}

#[derive(Default)]
pub(crate) struct Safe {
    mode: Option<SafeMode>,
    hidden: bool,
    /// The link the card is about.
    card: Option<(String, Verdict)>,
}

impl Safe {
    /// Takes what the shell says. Returns whether the document has just become trusted.
    pub fn set(&mut self, mode: Option<SafeMode>) -> bool {
        let trusted = self.mode.is_some() && mode.is_none();
        // A bar that was put away comes back with the next document that is held.
        if mode.is_none() {
            self.hidden = false;
        }
        self.mode = mode;
        trusted
    }

    pub fn on(&self) -> bool {
        self.mode.is_some()
    }

    /// Whether the shell named a folder that can be trusted with the document.
    pub fn folder(&self) -> bool {
        self.mode.as_ref().is_some_and(|mode| mode.folder.is_some())
    }

    pub fn hide(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    /// The room the bar takes above the page.
    pub fn height(&self) -> f32 {
        if self.on() && !self.hidden { banner::HEIGHT } else { 0.0 }
    }

    /// The bar under the top bar, while the document is held and the bar was not put away.
    pub fn bar(&self) -> Option<Element<Message>> {
        let mode = self.mode.as_ref().filter(|_| !self.hidden)?;
        let send = |asked: Safely| Message::Safe(asked);
        let mut bar = Banner::new(HELD)
            .icon(icon::SHIELD)
            .action(Button::new("Trust this document").small().on_press(send(Safely::TrustDocument)));
        if let Some(folder) = &mode.folder {
            let all = match folder.as_str() {
                "" => "Trust every document in its folder".to_owned(),
                folder => format!("Trust every document in {folder}"),
            };
            let trust = Button::new("Trust this folder").small().on_press(send(Safely::TrustFolder));
            bar = bar.action(Element::from(trust).tooltip(all, Side::Bottom));
        }
        Some(bar.on_dismiss("Hide", send(Safely::Hide)).into())
    }

    /// The button on the top bar that stands for a bar that was put away, and brings it back.
    pub fn badge(&self) -> Option<Button<Message>> {
        let hidden = self.on() && self.hidden;
        hidden.then(|| {
            Button::icon_only(icon::SHIELD, "Safe mode").subtle().small().on_press(Message::Safe(Safely::Show))
        })
    }

    /// The shell wants the person to see where a link goes before it is opened.
    pub fn ask(&mut self, href: &str) {
        self.card = Some((href.to_owned(), Verdict::Ask));
    }

    /// The shell will not open a link, and says why.
    pub fn refuse(&mut self, href: &str, why: &str) {
        self.card = Some((href.to_owned(), Verdict::Refused(why.to_owned())));
    }

    pub fn leave(&mut self) {
        self.card = None;
    }

    /// Puts the card away. Returns the address on it, if the card asked whether to open it.
    pub fn confirm(&mut self) -> Option<String> {
        let (href, verdict) = self.card.take()?;
        (verdict == Verdict::Ask).then_some(href)
    }

    /// The card over the middle of the window, while the shell has something to say about a link. It
    /// hangs from an element of no size, which the window puts after everything else.
    pub fn card(&self) -> Option<Element<Message>> {
        let (href, verdict) = self.card.as_ref()?;
        let send = |asked: Safely| Message::Safe(asked);
        let (title, sign) = match verdict {
            Verdict::Ask => ("Open this link?", icon::EXTERNAL_LINK),
            Verdict::Refused(_) => ("This link was not opened", icon::SHIELD),
        };
        let head = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_SM)
            .child(Element::icon(sign, 20.0).fg(tokens::FG_MUTED))
            .child(Element::text(title, TextStyle::control().weight(600)));
        // The whole address, in a face that tells its letters apart, on a ground that sets it off.
        let mono = TextStyle::control().size(tokens::TEXT_SM).family(Family::Mono);
        let fits = (CARD_WIDTH - 2.0 * tokens::SPACE_SM) / (tokens::TEXT_SM * MONO_ADVANCE);
        let lines = lines(&plain(href), fits as usize, LINES);
        let address = Element::column()
            .padding(Edges::all(tokens::SPACE_SM))
            .bg(tokens::NEUTRAL_SOFT)
            .radius(tokens::RADIUS_SM)
            .role(Role::Note)
            .label(plain(href))
            .children(lines.into_iter().map(|line| Element::text(line, mono)));
        let mut card = Element::column().gap(tokens::SPACE_MD).width(CARD_WIDTH).child(head).child(address);
        let foot = Element::row().align(Align::Center).justify(Justify::End).gap(tokens::SPACE_SM);
        let foot = match verdict {
            Verdict::Ask => {
                let open = Button::new("Open link").icon(icon::EXTERNAL_LINK).primary().on_press(send(Safely::Confirm));
                foot.child(Button::new("Cancel").on_press(send(Safely::Leave)))
                    .child(Element::from(open).key("open").takes_focus())
            }
            Verdict::Refused(why) => {
                let quiet = TextStyle::control().size(tokens::TEXT_SM);
                card = card.child(Element::paragraph(why.as_str(), quiet).fg(tokens::FG_MUTED));
                foot.child(Element::from(Button::new("Close").on_press(send(Safely::Leave))).key("close").takes_focus())
            }
        };
        let card = Popover::new(card.child(foot))
            .label(title)
            .max_width(CARD_WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
            .modal()
            .open(true)
            .on_close(send(Safely::Leave));
        Some(Element::row().overlay().popover(card))
    }
}

/// How wide a letter of the address is, as a share of its size. A little more than the face's own, so
/// that a line cut to fit does fit.
const MONO_ADVANCE: f32 = 0.62;

/// An address with nothing in it that hides: a character that is not drawn, or that turns the text round,
/// is written as its number.
fn plain(href: &str) -> String {
    let hides = |c: char| {
        c.is_control()
            || c.is_whitespace() && c != ' '
            || matches!(c, '\u{ad}' | '\u{61c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}')
            || matches!(c, '\u{2060}'..='\u{2069}' | '\u{feff}')
    };
    let mut plain = String::with_capacity(href.len());
    for c in href.chars() {
        match hides(c) {
            true => plain.push_str(&format!("\\u{{{:x}}}", u32::from(c))),
            false => plain.push(c),
        }
    }
    plain
}

/// An address cut into lines of `fits` characters, since it has no spaces to wrap at. One that would
/// take more than `most` lines keeps its start and its end, with a line of dots for what is left out.
fn lines(address: &str, fits: usize, most: usize) -> Vec<String> {
    let letters: Vec<char> = address.chars().collect();
    let mut lines: Vec<String> = letters.chunks(fits.max(1)).map(|line| line.iter().collect()).collect();
    if lines.len() > most.max(3) {
        let end = lines.split_off(lines.len() - 1);
        lines.truncate(most.max(3) - 2);
        lines.push("\u{2026}".to_owned());
        lines.extend(end);
    }
    lines
}

/// The address of the link whose text is drawn at a point of the layout.
pub(crate) fn link_at(doc: &Document, layout: &DocLayout, at: Point) -> Option<String> {
    link_line_at(doc, layout, at).map(|(_, href, _)| href)
}

/// The link whose text is drawn at a point of the layout: the block it is written in, its address, and the
/// line of it that is there.
pub(crate) fn link_line_at(doc: &Document, layout: &DocLayout, at: Point) -> Option<(BlockId, String, Rect)> {
    let pos = layout.hit(at)?;
    let block = doc.block(&pos.block)?;
    let mut start = 0;
    for run in block.body.runs() {
        let range = start..start + run.text.len();
        start = range.end;
        // A point on a run is nearest to one of its two ends, so most runs need no measuring.
        let near = !range.is_empty() && (range.start..=range.end).contains(&pos.offset);
        let Some(href) = run.link.as_deref().filter(|_| near) else { continue };
        let end = |offset| Pos::new(block.id.clone(), offset);
        let drawn = layout.selection(&Selection { anchor: end(range.start), head: end(range.end) });
        if let Some(line) = drawn.into_iter().find(|line| line.contains(at)) {
            return Some((block.id.clone(), href.to_owned(), line));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_shows_what_would_hide_in_it() {
        let cases = [
            ("https://a.example/page?q=1", "https://a.example/page?q=1"),
            ("https://a.example/\u{202e}gpj.exe", "https://a.example/\\u{202e}gpj.exe"),
            ("https://a\u{200b}.example", "https://a\\u{200b}.example"),
            ("java\tscript:run()", "java\\u{9}script:run()"),
        ];
        for (href, shown) in cases {
            assert_eq!(plain(href), shown);
        }
    }

    #[test]
    fn a_long_address_keeps_its_start_and_its_end() {
        assert_eq!(lines("abcdefgh", 3, 8), ["abc", "def", "gh"]);
        let long: String = ('a'..='z').collect();
        let cut = lines(&long, 2, 5);
        assert_eq!(cut, ["ab", "cd", "ef", "\u{2026}", "yz"]);
        assert_eq!(lines("", 10, 8), Vec::<String>::new());
    }
}
