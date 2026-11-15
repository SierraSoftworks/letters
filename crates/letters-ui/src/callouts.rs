//! Changing a callout where it stands: a press on its icon or its title opens a card that sets its type,
//! its title, its icon and its colour.
//!
//! The page is drawn on a canvas, so the window asks this module what a press is on. The card is about the
//! callout the caret is in, and each choice on it is an edit of its own, which shows on the page at once.

use accesskit::Role;
use letters_editor::{CalloutEdit, Editor};
use letters_layout::{Palette, icon_path};
use letters_model::{Block, BlockId, BlockKind, Body, BodyShape, Pos, Tone};
use letters_render::{DisplayList, Item, Rect};

use crate::color::{fade, resolve, same};
use crate::element::Element;
use crate::icon::Icon;
use crate::settings::{hex, read_hex};
use crate::style::{Align, Edges, Justify, Look, Ring};
use crate::text::TextStyle;
use crate::theme::accent;
use crate::tokens::{self, Scheme};
use crate::widgets::{Button, Popover, Segmented, Side, TextField};
use crate::window::Message;

/// What the icon and title of a callout are called as one control.
pub(crate) const LABEL: &str = "Edit callout";

/// How wide the card is.
const WIDTH: f32 = 304.0;
/// The size of a colour's dot.
const DOT: f32 = 18.0;
/// How far round a callout's icon and title a press still counts as on them, and the fill shows.
const REACH: f32 = 4.0;

/// The colours offered, as they show on a light page. The first four are the tones' own. Any other colour
/// can be typed.
const COLOURS: [(&str, [u8; 3]); 8] = [
    ("Blue", [0x2a, 0x63, 0xb5]),
    ("Green", [0x14, 0x6f, 0x43]),
    ("Amber", [0x85, 0x59, 0x00]),
    ("Red", [0xb8, 0x2c, 0x24]),
    ("Teal", [0x0b, 0x6a, 0x75]),
    ("Pink", [0xa8, 0x32, 0x7a]),
    ("Violet", [0x6b, 0x3f, 0xc4]),
    ("Graphite", [0x4a, 0x55, 0x60]),
];

/// What each icon is called, in the order of [`Tone::ICONS`].
const ICONS: [&str; 12] = [
    "Info",
    "Light bulb",
    "Warning triangle",
    "Stop sign",
    "Check",
    "Question",
    "Star",
    "Flame",
    "Bookmark",
    "Pencil",
    "Quote",
    "Bug",
];

/// What the card of a callout asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Calling {
    /// Shows the card of the callout the caret is in.
    Open,
    /// Changes that callout, as the editor does.
    Edit(CalloutEdit),
    /// What is typed into the colour's field, before it is taken.
    Hex(String),
    /// Takes the colour that was typed, if it is one.
    TakeHex,
    /// Drops what was typed into the colour's field.
    Forget,
    /// Escape or a press elsewhere.
    Close,
}

/// The callouts of the page, as the pointer and the card have them.
#[derive(Default)]
pub(crate) struct Callouts {
    /// The callout whose card shows.
    pub open: Option<BlockId>,
    /// What is being typed into the colour's field.
    hex: Option<String>,
    /// The callout whose icon and title the pointer rests on.
    pub over: Option<BlockId>,
}

/// The area round a callout's icon and title that takes a press.
pub(crate) fn target(head: Rect) -> Rect {
    Rect::new(head.x - REACH, head.y - REACH / 2.0, head.width + 2.0 * REACH, head.height + REACH)
}

/// Where the caret goes for a callout to be the one it is in: the start of the first text inside it that
/// no callout inside it holds.
pub(crate) fn inside(callout: &Block) -> Option<Pos> {
    fn first(blocks: &[Block]) -> Option<&Block> {
        blocks.iter().find_map(|block| match &block.body {
            _ if matches!(block.kind, BlockKind::Callout { .. }) => None,
            Body::Blocks(inner) => first(inner),
            _ => (block.kind.shape() == BodyShape::Text).then_some(block),
        })
    }
    first(callout.body.blocks()).map(|block| Pos::new(block.id.clone(), 0))
}

/// Whether a change would change the caret's callout, which is when a list of commands offers it.
pub(crate) fn changes(editor: &Editor, edit: &CalloutEdit) -> bool {
    let Some(BlockKind::Callout { tone, title, icon, color }) = editor.callout().map(|block| &block.kind) else {
        return false;
    };
    match edit {
        CalloutEdit::Tone(to) => to != tone,
        CalloutEdit::Reset => title.is_some() || icon.is_some() || color.is_some(),
        _ => true,
    }
}

/// The colour a tone has of its own, as it shows on a light page.
fn own(tone: Tone) -> [u8; 3] {
    let light = Palette::LIGHT;
    let of = match tone {
        Tone::Tip => light.tip,
        Tone::Info => light.info,
        Tone::Warn => light.warn,
        Tone::Danger => light.danger,
    };
    [of.accent.r, of.accent.g, of.accent.b]
}

impl Callouts {
    /// Takes what the card asked for that is the card's own to keep. Returns the edit to make, if any.
    pub fn update(&mut self, asked: Calling, editor: &Editor) -> Option<CalloutEdit> {
        match asked {
            Calling::Open => {
                (self.open, self.hex) = (editor.callout().map(|block| block.id.clone()), None);
                None
            }
            Calling::Edit(edit) => {
                // A colour that was pressed is the colour, whatever was being typed.
                if matches!(edit, CalloutEdit::Color(_) | CalloutEdit::Reset | CalloutEdit::Tone(_)) {
                    self.hex = None;
                }
                if edit == CalloutEdit::Remove {
                    self.open = None;
                }
                Some(edit)
            }
            Calling::Hex(typed) => {
                self.hex = Some(typed);
                None
            }
            Calling::TakeHex => {
                let rgb = self.hex.take().and_then(|typed| read_hex(&typed))?;
                let tone = editor.callout().and_then(|block| block.kind.callout_look()).map(|look| look.tone)?;
                // A tone's own colour is no choice at all, so that it follows the tone.
                Some(CalloutEdit::Color((rgb != own(tone)).then_some(hex(rgb))))
            }
            Calling::Forget => {
                self.hex = None;
                None
            }
            Calling::Close => {
                (self.open, self.hex) = (None, None);
                None
            }
        }
    }

    /// Puts the card away once the caret is in another callout, or in none.
    pub fn follow(&mut self, editor: &Editor) {
        if self.open.is_some() && self.open.as_ref() != editor.callout().map(|block| &block.id) {
            (self.open, self.hex) = (None, None);
        }
    }

    /// Shows that a press on a callout's icon and title does something, while the pointer rests there or
    /// its card is open.
    pub fn paint(&self, block: &BlockId, head: Rect, scheme: Scheme, out: &mut DisplayList) {
        if self.over.as_ref() == Some(block) || self.open.as_ref() == Some(block) {
            let (rect, radius) = (target(head), tokens::RADIUS_CONTROL);
            out.push(Item::Fill { rect, color: resolve(tokens::NEUTRAL_SOFT, scheme, 1.0), radius });
        }
    }

    /// The card of a callout, which hangs under its icon and title at `spot`.
    pub fn card(&self, kind: &BlockKind, spot: Rect) -> Option<Popover<Message>> {
        let BlockKind::Callout { title, icon, color, .. } = kind else { return None };
        let look = kind.callout_look()?;
        let edit = |edit: CalloutEdit| Message::Callout(Calling::Edit(edit));

        let mut tones = Segmented::new("Type").small();
        for tone in Tone::ALL {
            tones = tones.segment(tone.title(), tone == look.tone, edit(CalloutEdit::Tone(tone)));
        }

        let field = TextField::new("Title", look.title)
            .placeholder("No title")
            .on_input(move |typed| Message::Callout(Calling::Edit(CalloutEdit::Title(Some(typed)))))
            .on_submit(Message::Callout(Calling::Close));

        // Two rows of icons, and no icon at the end of the second.
        let mut buttons: Vec<Element<Message>> = Vec::new();
        for (name, label) in Tone::ICONS.into_iter().zip(ICONS) {
            let Some(path) = icon_path(name) else { continue };
            let button = Button::icon_only(Icon::new(path), label)
                .subtle()
                .small()
                .pressed(look.icon == Some(name))
                .on_press(edit(CalloutEdit::Icon(Some(name.to_owned()))));
            buttons.push(button.into());
        }
        let none = Button::new("None").subtle().small().pressed(look.icon.is_none());
        buttons.push(none.on_press(edit(CalloutEdit::Icon(Some("none".to_owned())))).into());
        let second = buttons.split_off(buttons.len().min(7));
        let line = |buttons: Vec<Element<Message>>| Element::row().gap(2.0).align(Align::Center).children(buttons);
        let icons = Element::column().gap(2.0).child(line(buttons)).child(line(second));

        let shown = look.color.and_then(read_hex).unwrap_or(own(look.tone));
        let mut dots = Element::row().gap(2.0).align(Align::Center);
        for (name, rgb) in COLOURS {
            let chosen = rgb == shown;
            let [r, g, b] = rgb;
            let dot = Element::row().width(DOT).height(DOT).radius(DOT / 2.0).bg(same(r, g, b));
            // The ring is always there, clear until its colour is the one chosen, so that no dot moves.
            let ring = if chosen { accent().base } else { fade(tokens::FG, 0.0) };
            let to = (rgb != own(look.tone)).then_some(hex(rgb));
            let swatch = Element::row()
                .padding(Edges::all(1.0))
                .border(Edges::all(2.0), ring)
                .radius(DOT / 2.0 + 3.0)
                .hover(Look::bg(tokens::NEUTRAL_SOFT))
                .ring(Ring::Outline(2.0))
                .role(Role::Button)
                .label(name)
                .pressed(chosen)
                .tooltip(name, Side::Top)
                .on_press(edit(CalloutEdit::Color(to)))
                .child(dot);
            dots = dots.child(swatch);
        }
        let typed = self.hex.as_ref();
        let hex_field = TextField::new("Colour", typed.cloned().unwrap_or(hex(shown)))
            .mono()
            .invalid(typed.is_some_and(|typed| read_hex(typed).is_none()))
            .on_input(|typed| Message::Callout(Calling::Hex(typed)))
            .on_submit(Message::Callout(Calling::TakeHex))
            .on_leave(Message::Callout(Calling::TakeHex))
            .on_cancel(Message::Callout(Calling::Forget));
        let colours = Element::row()
            .align(Align::Center)
            .justify(Justify::Between)
            .gap(tokens::SPACE_SM)
            .child(dots)
            .child(Element::from(hex_field).width(80.0));

        let chosen = title.is_some() || icon.is_some() || color.is_some();
        let reset = Button::new(format!("Use the defaults for {}", look.tone.title()))
            .subtle()
            .small()
            .disabled(!chosen)
            .on_press(edit(CalloutEdit::Reset));

        let name = |name: &str| Element::text(name, TextStyle::control().size(tokens::TEXT_SM)).fg(tokens::FG_MUTED);
        let card = Element::column()
            .width(WIDTH)
            .gap(tokens::SPACE_SM)
            .child(tones)
            .child(field)
            .child(Element::column().gap(tokens::SPACE_XS).child(name("Icon").decorative()).child(icons))
            .child(Element::column().gap(tokens::SPACE_XS).child(name("Colour").decorative()).child(colours))
            .child(Element::row().justify(Justify::End).child(reset));
        let card = Popover::new(card)
            .label("Callout")
            .max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
            .at(spot)
            .open(true)
            .on_close(Message::Callout(Calling::Close));
        Some(card)
    }
}
