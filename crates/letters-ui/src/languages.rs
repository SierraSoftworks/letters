//! The language of a code block: the list under a fence while its name is typed, and the name in the corner
//! of the block's box, where it is changed.
//!
//! The languages on offer are those the highlighter colours, and code in none. A fence can still name any
//! other: the name is kept as written and shows in the corner, and the code is left plain.

use std::borrow::Cow;

use letters_editor::Editor;
use letters_model::{BlockId, BlockKind};
use letters_render::{Color, DisplayList, Item, Point, Rect};
use yewi_highlight::Language;

use crate::code_copy;
use crate::color::resolve;
use crate::element::Element;
use crate::style::Edges;
use crate::text::{Family, Shaped, TextStyle};
use crate::tokens::{self, Scheme};
use crate::widgets::{Choice, Options, Popover, TextField, options};
use crate::window::Message;

/// What opens a block of code, when typed on a line of its own.
const FENCE: &str = "```";
/// What code in no language is called.
const PLAIN: &str = "Plain text";
/// How wide the list under a fence is: as wide as the insert menu, which hangs where it does.
const WIDTH: f32 = 288.0;
/// How wide the list that changes a block's language is.
const CARD: f32 = 240.0;
/// How many languages a list shows before it scrolls.
const ROWS: usize = 8;
/// The room a block's language keeps to the left and right of its name.
const PADDING: f32 = 6.0;
/// How far it stands from the copy button.
const GAP: f32 = 2.0;

/// What the pointer and the keyboard ask of the list under a fence.
#[derive(Clone, Debug, PartialEq)]
pub enum Fencing {
    /// A language was pressed, counted from the first that shows.
    Take(usize),
    /// Escape or a press elsewhere, which leaves the line as it was typed.
    Close,
}

/// What the pointer and the keyboard ask of a code block's language.
#[derive(Clone, Debug, PartialEq)]
pub enum Languaging {
    /// Lists the languages the caret's code block can be in.
    Open,
    /// What the field over the list holds.
    Type(String),
    /// Moves the pick one language down the list, or up, round its ends.
    Step { down: bool },
    /// A language was pressed, counted from the first that shows.
    Take(usize),
    /// Enter, which takes the language that is picked.
    Enter,
    /// Escape or a press elsewhere.
    Close,
}

/// A language to write code in.
pub(crate) struct Offer {
    /// The language, or `None` for code in none.
    language: Option<Language>,
    /// The other names and file endings that find it. For a language they are those the highlighter reads
    /// in a fence.
    also: &'static str,
}

impl Offer {
    /// What a reader calls it.
    pub fn label(&self) -> &'static str {
        self.language.map_or(PLAIN, Language::label)
    }

    /// What a fence calls it. Code in no language has a fence with no name.
    pub fn name(&self) -> Option<&'static str> {
        self.language.map(Language::name)
    }

    /// How well what is typed, in small letters, finds it: a whole name before the start of what it is
    /// called, and that before the start of another name.
    fn found(&self, typed: &str) -> Option<u8> {
        let label = self.label().to_lowercase();
        let called = || self.name().into_iter().chain([label.as_str()]);
        let also = || self.also.split_whitespace();
        if called().chain(also()).any(|name| name == typed) {
            Some(0)
        } else if called().any(|name| name.starts_with(typed)) {
            Some(1)
        } else {
            also().any(|name| name.starts_with(typed)).then_some(2)
        }
    }
}

const fn offer(language: Language, also: &'static str) -> Offer {
    Offer { language: Some(language), also }
}

/// The languages by what a reader calls them, and code in none last.
pub(crate) static OFFERS: [Offer; 15] = [
    offer(Language::Css, "scss"),
    offer(Language::Diff, "patch"),
    offer(Language::Go, "golang"),
    offer(Language::Hcl, "nomad terraform tf"),
    offer(Language::Html, "htm xml svg"),
    offer(Language::JavaScript, "js jsx mjs cjs"),
    offer(Language::Json, "jsonc json5"),
    offer(Language::Python, "py"),
    offer(Language::Rust, "rs"),
    offer(Language::Bash, "sh shell zsh console"),
    offer(Language::Sql, ""),
    offer(Language::Toml, ""),
    offer(Language::TypeScript, "ts tsx"),
    offer(Language::Yaml, "yml"),
    Offer { language: None, also: "text txt none" },
];

/// The languages that what is typed finds, the nearest first: all of them when nothing is typed.
pub(crate) fn matching(typed: &str) -> Vec<&'static Offer> {
    let typed = typed.trim().to_lowercase();
    let mut found: Vec<(u8, &Offer)> = OFFERS.iter().filter_map(|offer| Some((offer.found(&typed)?, offer))).collect();
    if !typed.is_empty() {
        found.sort_by_key(|(near, _)| *near);
    }
    found.into_iter().map(|(_, offer)| offer).collect()
}

/// What the corner of a code block calls its language: what a reader calls it, or the name as the fence
/// wrote it when the highlighter does not know it.
pub(crate) fn named(language: Option<&str>) -> Cow<'_, str> {
    match language {
        None => PLAIN.into(),
        Some(name) => yewi_highlight::language_label(name).unwrap_or(name.into()),
    }
}

/// What assistive technology calls the language in the corner of a code block.
pub(crate) fn label(language: Option<&str>) -> String {
    format!("Language: {}", named(language))
}

/// The style a block's language is named in.
pub(crate) fn style() -> TextStyle {
    TextStyle::control().size(tokens::TEXT_XS)
}

/// Where a code block's language stands in its box, when its name is `wide`: beside the copy button.
pub(crate) fn place(code: Rect, wide: f32) -> Rect {
    let button = code_copy::button(code);
    let wide = (wide + 2.0 * PADDING).ceil();
    Rect::new(button.x - GAP - wide, button.y, wide, button.height)
}

/// A list of languages, of which those that find room show at once. The rows of one that scrolls end short
/// of the bar that shows how far.
fn list(options: Options<Message>, rows: usize) -> Element<Message> {
    let bar = if rows > ROWS { tokens::SPACE_SM } else { 0.0 };
    let list = Element::from(options).padding(Edges { right: bar, ..Edges::all(0.0) });
    list.max_height(ROWS as f32 * options::ROW + tokens::SPACE_XS).scroll()
}

/// One row of a list of languages: what it is called, and what a fence calls it.
fn choice(offer: &Offer, take: Message) -> Choice<Message> {
    let written = TextStyle::control().family(Family::Mono).size(tokens::TEXT_XS);
    let choice = Choice::new(offer.label()).on_take(take);
    match offer.name() {
        Some(name) => choice.aside(name, written),
        None => choice,
    }
}

/// The list under a fence that is being typed: the line it is open on, and which language is picked.
#[derive(Default)]
pub(crate) struct Fence {
    line: Option<BlockId>,
    picked: usize,
}

impl Fence {
    /// The caret's line and the name after its fence, when the line is one the editor would make code of
    /// and the caret is at its end: the only writing the list stays open over.
    fn fenced(editor: &Editor) -> Option<(&BlockId, String)> {
        let caret = editor.selection().filter(|selection| selection.is_caret())?;
        let line = editor.doc().block(&caret.head.block).filter(|line| line.kind == BlockKind::Paragraph)?;
        let text = line.plain_text();
        let name = text.strip_prefix(FENCE)?;
        let named = !name.contains(|c: char| c == '`' || c.is_whitespace()) && caret.head.offset == text.len();
        named.then(|| (&line.id, name.to_owned()))
    }

    /// Opens the list when a fence has just been typed.
    pub fn typed(&mut self, editor: &Editor) {
        if let Some((line, _)) = Self::fenced(editor).filter(|(_, name)| name.is_empty()) {
            (self.line, self.picked) = (Some(line.clone()), 0);
        }
    }

    /// Puts the list away once its line no longer reads as a fence, or the caret has left it.
    pub fn follow(&mut self, editor: &Editor) {
        let open = Self::fenced(editor).is_some_and(|(line, _)| Some(line) == self.line.as_ref());
        if !open {
            self.close();
        }
    }

    pub fn close(&mut self) {
        (self.line, self.picked) = (None, 0);
    }

    /// The languages the list shows, and how much of its line was typed to find them. `None` while it is
    /// put away or nothing matches.
    pub fn offers(&self, editor: &Editor) -> Option<(BlockId, usize, Vec<&'static Offer>)> {
        let (line, name) = Self::fenced(editor).filter(|(line, _)| Some(*line) == self.line.as_ref())?;
        let offers = matching(&name);
        (!offers.is_empty()).then(|| (line.clone(), FENCE.len() + name.len(), offers))
    }

    /// Moves the pick one language down or up a list of `shown`, round its ends.
    pub fn pick(&mut self, down: bool, shown: usize) {
        self.picked = Options::<Message>::step(self.picked, down, shown);
    }

    /// The language that is picked among `shown`.
    pub fn picked(&self, shown: usize) -> usize {
        self.picked.min(shown.saturating_sub(1))
    }

    /// The list over `offers`, to hang under the fence.
    pub fn list(&self, offers: &[&'static Offer]) -> Popover<Message> {
        let mut options = Options::new("Languages").picked(self.picked(offers.len()));
        for (index, offer) in offers.iter().enumerate() {
            options = options.choice(choice(offer, Message::Fence(Fencing::Take(index))));
        }
        let list = list(options, offers.len());
        let hint = options::hint("Matches names and file endings", &["Tab", "Esc"]);
        let card = Element::column().width(WIDTH).gap(tokens::SPACE_XS).padding(Edges::all(tokens::SPACE_XS));
        let card = card.child(list).child(hint);
        Popover::new(card).flush().beside().open(true).on_close(Message::Fence(Fencing::Close))
    }
}

/// The languages of the code blocks on the page, as the pointer and the list that changes one have them.
#[derive(Default)]
pub(crate) struct Languages {
    /// The code block whose language the pointer rests on.
    pub over: Option<BlockId>,
    /// The code block whose languages are listed.
    open: Option<BlockId>,
    /// What is typed to find one.
    typed: String,
    /// Which is picked, or none: the block is in a language that is not listed.
    picked: Option<usize>,
}

impl Languages {
    /// Lists the languages for a code block that is in `language` now, which is the one picked.
    pub fn open(&mut self, block: BlockId, language: Option<&str>) {
        // A name the highlighter does not know is none of those listed.
        let known = language.map_or(Some(None), |name| Language::from_name(name).map(Some));
        let picked = known.and_then(|known| OFFERS.iter().position(|offer| offer.language == known));
        *self = Self { over: self.over.take(), open: Some(block), typed: String::new(), picked };
    }

    /// The code block whose languages are listed.
    pub fn listed(&self) -> Option<&BlockId> {
        self.open.as_ref()
    }

    pub fn close(&mut self) {
        *self = Self { over: self.over.take(), ..Self::default() };
    }

    /// Puts the list away once the caret is in another block.
    pub fn follow(&mut self, editor: &Editor) {
        let inside = editor.selection().map(|selection| &selection.head.block);
        if self.open.is_some() && self.open.as_ref() != inside {
            self.close();
        }
    }

    /// Takes what the field holds now. The nearest language it finds is the one picked.
    pub fn typed(&mut self, typed: String) {
        (self.typed, self.picked) = (typed, Some(0));
    }

    /// Moves the pick one language down or up the list, round its ends. With none picked it goes to the
    /// first, or to the last.
    pub fn step(&mut self, down: bool) {
        let shown = matching(&self.typed).len();
        self.picked = Some(match self.picked {
            Some(picked) => Options::<Message>::step(picked, down, shown),
            None if down => 0,
            None => shown.saturating_sub(1),
        });
    }

    /// Puts the list away. Returns its block and what a fence calls the language at `index` of those that
    /// show, or the one that is picked: `None` when there is none there.
    pub fn take(&mut self, index: Option<usize>) -> Option<(BlockId, Option<&'static str>)> {
        let offers = matching(&self.typed);
        let offer = index.or(self.picked).and_then(|index| offers.get(index));
        let taken = self.open.take().zip(offer).map(|(block, offer)| (block, offer.name()));
        self.close();
        taken
    }

    /// Names a block's language at `at`, as [`place`] gave it. `behind` is what the box shows under it,
    /// bottom first, for a block whose first line reaches it: the end of that line is then covered.
    pub fn paint(
        &self,
        block: &BlockId,
        (at, said): (Rect, &Shaped),
        behind: Option<[Color; 2]>,
        scheme: Scheme,
        out: &mut DisplayList,
    ) {
        let radius = tokens::RADIUS_CONTROL;
        // Nothing of the line shows between the name and the copy button either.
        let covered = Rect { width: at.width + GAP, ..at };
        out.items.extend(behind.into_iter().flatten().map(|color| Item::Fill { rect: covered, color, radius }));
        let on = self.over.as_ref() == Some(block) || self.open.as_ref() == Some(block);
        if on {
            out.push(Item::Fill { rect: at, color: resolve(tokens::NEUTRAL_SOFT, scheme, 1.0), radius });
        }
        let ink = if on { tokens::FG } else { tokens::FG_MUTED };
        let origin =
            Point { x: at.x + (at.width - said.size.width) / 2.0, y: at.y + (at.height - said.size.height) / 2.0 };
        said.paint(origin, resolve(ink, scheme, 1.0), out);
    }

    /// The list of languages, which hangs under the block's language at `spot`.
    pub fn card(&self, spot: Rect) -> Popover<Message> {
        let send = Message::Language;
        let field = TextField::new("Language", self.typed.as_str())
            .placeholder("Find a language")
            .on_input(move |typed| send(Languaging::Type(typed)))
            .on_submit(send(Languaging::Enter))
            .on_cancel(send(Languaging::Close));
        let field = Element::from(field).key("language").takes_focus();
        let offers = matching(&self.typed);
        let list: Element<Message> = if offers.is_empty() {
            let quiet = TextStyle::control().size(tokens::TEXT_SM);
            let none = Element::text("No language matches.", quiet).fg(tokens::FG_MUTED).role(accesskit::Role::Status);
            Element::row().padding(Edges::xy(tokens::SPACE_SM, tokens::SPACE_XS)).child(none)
        } else {
            let mut options = Options::new("Languages").picked(self.picked.unwrap_or(usize::MAX));
            for (index, offer) in offers.iter().enumerate() {
                options = options.choice(choice(offer, send(Languaging::Take(index))));
            }
            list(options, offers.len())
        };
        // The field has the keys: those that walk a list are no use to one line of text.
        let keys = move |event: &letters_editor::KeyEvent| match event.key {
            _ if event.mods != letters_editor::Modifiers::default() => None,
            letters_editor::Key::Down => Some(send(Languaging::Step { down: true })),
            letters_editor::Key::Up => Some(send(Languaging::Step { down: false })),
            _ => None,
        };
        let card = Element::column().width(CARD).gap(tokens::SPACE_XS).padding(Edges::all(tokens::SPACE_XS));
        let card = card.child(field).child(list).on_key(keys);
        Popover::new(card)
            .label("Code language")
            .flush()
            .align(crate::style::Align::End)
            .at(spot)
            .open(true)
            .on_close(send(Languaging::Close))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(typed: &str) -> Vec<&'static str> {
        matching(typed).into_iter().map(Offer::label).collect()
    }

    #[test]
    fn what_is_typed_finds_languages_by_name_other_name_and_file_ending() {
        assert_eq!(labels("").len(), OFFERS.len(), "a fence alone offers everything");
        // What is typed, and the languages it finds, the nearest first.
        let cases: [(&str, &[&str]); 9] = [
            ("rust", &["Rust"]),
            ("RS", &["Rust"]),
            ("py", &["Python"]),
            // A whole name comes before the start of another.
            ("js", &["JavaScript", "JSON"]),
            ("sh", &["Shell"]),
            ("t", &["TOML", "TypeScript", "HCL", "Plain text"]),
            ("yml", &["YAML"]),
            ("plain", &["Plain text"]),
            ("klingon", &[]),
        ];
        for (typed, found) in cases {
            assert_eq!(labels(typed), found, "{typed}");
        }
    }

    #[test]
    fn every_language_the_highlighter_colours_is_offered_by_the_names_it_reads() {
        for language in Language::ALL {
            let offer = OFFERS.iter().find(|offer| offer.language == Some(language));
            let offer = offer.unwrap_or_else(|| panic!("{language:?} is not offered"));
            for name in offer.also.split_whitespace() {
                assert_eq!(Language::from_name(name), Some(language), "{name}");
            }
        }
        let plain = OFFERS.iter().find(|offer| offer.language.is_none()).expect("code in no language");
        for name in plain.also.split_whitespace() {
            assert_eq!(Language::from_name(name), None, "{name} is a language's own name");
        }
    }

    #[test]
    fn a_code_block_is_named_by_what_a_reader_calls_its_language() {
        // What the fence wrote, and what the corner says.
        let cases = [(None, "Plain text"), (Some("rs"), "Rust"), (Some("YAML"), "YAML"), (Some("mermaid"), "mermaid")];
        for (written, says) in cases {
            assert_eq!(named(written), says, "{written:?}");
        }
    }

    #[test]
    fn the_list_opens_on_the_language_the_block_is_in() {
        // What the block is in, and the language picked when its list opens.
        let cases = [(Some("ts"), Some("TypeScript")), (None, Some("Plain text")), (Some("mermaid"), None)];
        for (language, picked) in cases {
            let mut languages = Languages::default();
            languages.open(BlockId::new("a"), language);
            assert_eq!(languages.picked.map(|picked| OFFERS[picked].label()), picked, "{language:?}");
        }
    }
}
