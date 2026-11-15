//! The field a comment or a reply is written in.
//!
//! It is a small stretch of the page. An editor of its own writes it, so what is typed between Markdown's
//! marks is bold, italic, struck or code at once and an address is a link, and the layout that sets what
//! notes say sets it, with the marks shown where the caret is as the page shows them. What it holds when
//! it is sent is the note's text as the document keeps it.

use std::rc::Rc;

use accesskit::Role;
use letters_editor::{Clip, Command, Editor, InputEvent, Key, KeyEvent, Keymap, Lines, Platform};
use letters_layout::DocLayout;
use letters_model::{Block, BlockKind, Document, Mark, Pos, ReplicaId, Run, Selection};
use letters_render::{Color, DisplayList, Item, Point, Rect, Size};

use crate::color::{mix, resolve};
use crate::element::{Cursor, Element, Paint, Pointer};
use crate::notes::{self, Noting};
use crate::style::{Edges, Look, Ring};
use crate::text::{Shaped, TextShaper, TextStyle};
use crate::theme::accent;
use crate::threads::small;
use crate::tokens::{self, Scheme};
use crate::widgets::ControlSize;
use crate::window::{DocLines, Message};

/// The one block a field's text is in.
const ID: &str = "written";
/// The room between the field's edge and what is written in it: as a small text input has at its sides,
/// and what makes one line as tall as the button beside it.
const PADDING: (f32, f32) = (tokens::CONTROL_PADDING - 6.0, 3.5);

/// What the keyboard and the pointer do in the field of a comment or a reply.
#[derive(Clone, Debug, PartialEq)]
pub enum Writing {
    /// Text from the keyboard or an input method.
    Text(InputEvent),
    /// A key that moves in what is written, edits it or marks it.
    Key(KeyEvent),
    /// A press at a point of the field: with Shift held it extends the selection, and it is the first,
    /// second or third of `clicks` in quick succession.
    Press { at: Point, extend: bool, clicks: u8 },
    /// The pointer moved to a point of the field with its button held.
    Drag(Point),
}

/// How wide the text of a field is that shares a row `row` wide with the button that sends it.
pub(crate) fn width(row: f32) -> f32 {
    row - tokens::SPACE_SM - ControlSize::Small.height() - 2.0 * PADDING.0 - 2.0
}

/// The keys of a field: those of the page that move in a stretch of text, edit it or mark it. The key
/// that pastes puts in plain text, since what is written stays one stretch of text.
pub(crate) fn keys(page: &Keymap) -> Keymap {
    let fits = |command: &Command| match command {
        Command::ToggleMark(mark) => matches!(mark, Mark::Bold | Mark::Italic | Mark::Strike | Mark::Code),
        Command::Move { .. } | Command::Delete(_) | Command::Undo | Command::Redo => true,
        Command::SelectAll | Command::SelectWord | Command::Copy | Command::Cut | Command::PastePlain => true,
        _ => false,
    };
    let bound: Vec<(KeyEvent, Command)> =
        page.bindings().map(|(key, command)| (key.clone(), command.clone())).collect();
    let mut keys = page.clone();
    bound.iter().for_each(|(key, _)| keys.unbind(key));
    // Each goes before those bound so far, so the last goes first and the order stays the page's.
    for (key, command) in bound.into_iter().rev() {
        let command = if command == Command::Paste { Command::PastePlain } else { command };
        if fits(&command) {
            keys.also(key.key, key.mods, command);
        }
    }
    keys
}

/// What the field asks of the window to do what was asked of it.
pub(crate) struct Desk<'a> {
    /// The page's keys.
    pub keys: &'a Keymap,
    /// The layout that sets what notes say.
    pub layout: &'a mut DocLayout,
    /// How wide the text is set.
    pub width: f32,
}

/// What is written in a field so far.
#[derive(Default)]
pub(crate) struct Written {
    /// The editor that writes it, made when something is first done in the field: one that is only
    /// looked at needs none.
    editor: Option<Box<Editor>>,
}

impl Written {
    /// What is written, without its marks.
    pub fn plain(&self) -> String {
        self.block().map(Block::plain_text).unwrap_or_default()
    }

    /// Whether nothing is written but spaces.
    pub fn is_blank(&self) -> bool {
        self.plain().trim().is_empty()
    }

    /// What is written as a note keeps it: its runs without the space around them, and an address that
    /// was left as plain text as a link to itself.
    pub fn content(&self) -> Vec<Run> {
        let mut runs = self.block().map(|block| block.body.runs().to_vec()).unwrap_or_default();
        if let Some(first) = runs.first_mut() {
            first.text = first.text.trim_start().to_owned();
        }
        if let Some(last) = runs.last_mut() {
            last.text = last.text.trim_end().to_owned();
        }
        runs.retain(|run| !run.text.is_empty());
        notes::addressed(runs)
    }

    fn block(&self) -> Option<&Block> {
        self.editor.as_ref().and_then(|editor| editor.doc().blocks.first())
    }

    /// Does what the keyboard or the pointer asked in the field.
    pub fn does(&mut self, asked: Writing, desk: Desk, platform: &mut dyn Platform) {
        let editor = self.editor.get_or_insert_with(|| {
            let mut editor = Editor::new(blank(), ReplicaId::from(1));
            editor.set_inline_only(true);
            editor.set_keymap(keys(desk.keys));
            Box::new(editor)
        });
        // The layout holds whatever it set last, so it is set with this text before it is asked about it.
        set(desk.layout, Some(editor), desk.width);
        let place = |at: Point| {
            let within =
                Point { x: at.x.clamp(0.0, desk.width), y: at.y.clamp(0.0, (desk.layout.height() - 1.0).max(0.0)) };
            desk.layout.hit_sided(within)
        };
        match asked {
            Writing::Text(event) => {
                _ = editor.input(event, &mut Lent { platform, lines: DocLines::new(desk.layout, 0.0) })
            }
            Writing::Key(event) => {
                _ = editor.key(event, &mut Lent { platform, lines: DocLines::new(desk.layout, 0.0) })
            }
            Writing::Press { at, extend, clicks } => {
                let Some((head, before_wrap)) = place(at) else { return unset(desk.layout) };
                let anchor =
                    editor.selection().filter(|_| extend).map_or_else(|| head.clone(), |held| held.anchor.clone());
                // The later presses of a double or triple click are about where the first put the caret.
                if clicks == 1 || extend {
                    editor.select_at(Selection { anchor, head }, before_wrap);
                }
                match clicks {
                    1 => {}
                    2 => _ = editor.run(Command::SelectWord, platform),
                    _ => _ = editor.run(Command::SelectAll, platform),
                }
            }
            Writing::Drag(at) => {
                let Some((head, before_wrap)) = place(at) else { return unset(desk.layout) };
                let anchor = editor.selection().map_or_else(|| head.clone(), |held| held.anchor.clone());
                editor.select_at(Selection { anchor, head }, before_wrap);
            }
        }
        unset(desk.layout);
    }

    /// Sets what is written `width` wide, to show in the field. `selection` is the colour of selected
    /// text, and `hint` what an empty field says in its place.
    fn show(&self, layout: &mut DocLayout, width: f32, selection: Color, hint: Shaped) -> Shown {
        let editor = self.editor.as_deref();
        set(layout, editor, width);
        let height = layout.height().ceil();
        let mut list = DisplayList::default();
        let held = editor.and_then(Editor::selection);
        if let Some(ranged) = held.filter(|held| !held.is_caret()) {
            let lines = layout.selection(ranged).into_iter();
            list.items.extend(lines.map(|rect| Item::Fill { rect, color: selection, radius: 0.0 }));
        }
        layout.paint(Rect::new(0.0, 0.0, width, height), &mut list);
        let start = Pos::new(ID, 0);
        let caret = match (editor, held) {
            (Some(editor), Some(held)) => layout.caret_sided(&held.head, editor.before_wrap()),
            _ => layout.caret(&start),
        };
        unset(layout);
        let plain = self.plain();
        Shown { list, size: Size { width, height }, caret, hint: plain.is_empty().then_some(hint), plain }
    }
}

impl Written {
    /// The field as the window shows it, set as [`Desk`] says. `look` is the colour of selected text and
    /// what shapes the words of an empty field, `named` the field's name and those words, and Enter sends
    /// `submit`.
    pub fn field(
        &self,
        desk: Desk,
        (selection, shaper): (Color, &dyn TextShaper),
        named: (&str, &str),
        submit: Noting,
    ) -> Element<Message> {
        let shown = self.show(desk.layout, desk.width, selection, hint(named.1, shaper));
        field(shown, named, Rc::new(keys(desk.keys)), submit)
    }
}

/// The document of a field nothing is written in yet.
fn blank() -> Document {
    Document { blocks: vec![Block::text(ID, BlockKind::Paragraph, "")], ..Document::default() }
}

/// Sets what a field holds in the layout as the field shows it: in the small type of the interface,
/// with the marks that belong where the caret is.
fn set(layout: &mut DocLayout, editor: Option<&Editor>, width: f32) {
    let Some(editor) = editor else {
        layout.update(&blank(), &small(), width, None);
        layout.settle(Rect::new(0.0, 0.0, width, f32::MAX), 0);
        return;
    };
    layout.update(editor.doc(), &small(), width, None);
    let head = editor.selection().map(|held| held.head.clone());
    let markers = editor.markers();
    layout.reveal(head.as_ref().map(|head| (head, &markers)));
    layout.set_composition(editor.composition());
    layout.settle(Rect::new(0.0, 0.0, width, f32::MAX), 0);
}

/// Leaves the layout as what notes say is set in it: with no caret anywhere.
fn unset(layout: &mut DocLayout) {
    layout.reveal(None);
    layout.set_composition(None);
}

/// Lends the field's editor the shell's platform together with the lines of the field.
struct Lent<'a> {
    platform: &'a mut dyn Platform,
    lines: DocLines<'a>,
}

impl Platform for Lent<'_> {
    fn now_ms(&self) -> u64 {
        self.platform.now_ms()
    }

    fn wall_time(&self) -> Option<u64> {
        self.platform.wall_time()
    }

    fn clipboard_read(&mut self) -> Option<Clip> {
        self.platform.clipboard_read()
    }

    fn clipboard_write(&mut self, clip: Clip) {
        self.platform.clipboard_write(clip);
    }

    fn new_id(&mut self) -> String {
        self.platform.new_id()
    }

    fn lines(&self) -> Option<&dyn Lines> {
        Some(&self.lines)
    }
}

/// What a field shows, as it was last set.
pub(crate) struct Shown {
    list: DisplayList,
    size: Size,
    /// Where the caret is in it.
    caret: Option<Rect>,
    /// What it says while nothing is written in it.
    hint: Option<Shaped>,
    plain: String,
}

impl Paint for Shown {
    fn paint(&self, rect: Rect, _visible: Rect, scheme: Scheme, out: &mut DisplayList) {
        let first = out.items.len();
        out.items.extend(self.list.items.iter().cloned());
        out.translate(first, Point { x: rect.x, y: rect.y });
        if let Some(hint) = &self.hint {
            let down = (self.size.height - hint.size.height) / 2.0;
            hint.paint(Point { x: rect.x, y: rect.y + down }, resolve(tokens::FG_MUTED, scheme, 1.0), out);
        }
    }
}

/// How an empty field's hint is set.
fn hint(says: &str, shaper: &dyn TextShaper) -> Shaped {
    shaper.shape(says, &TextStyle::control().size(tokens::TEXT_SM), None)
}

/// The field as the window shows it: `shown` is what is written in it, `label` its name and `says` what
/// it says while it is empty. `keys` are the keys it uses, and Enter sends `submit`.
fn field(shown: Shown, (label, says): (&str, &str), keys: Rc<Keymap>, submit: Noting) -> Element<Message> {
    let wrote = |asked: Writing| Message::Note(Noting::Wrote(asked));
    let keyed = move |event: &KeyEvent| {
        let plain = !event.mods.primary && !event.mods.alt && !event.mods.shift;
        if event.key == Key::Enter && plain {
            return Some(Message::Note(submit.clone()));
        }
        keys.command(event).map(|_| wrote(Writing::Key(event.clone())))
    };
    let pointed = move |event: Pointer| match event {
        Pointer::Down { at, mods, clicks } => Some(wrote(Writing::Press { at, extend: mods.shift, clicks })),
        Pointer::Drag { at } => Some(wrote(Writing::Drag(at))),
        _ => None,
    };
    let (size, caret, plain, empty) = (shown.size, shown.caret, shown.plain.clone(), shown.hint.is_some());
    let mut field = Element::canvas(Rc::new(shown))
        .width(size.width + 2.0 * PADDING.0 + 2.0)
        .height(size.height + 2.0 * PADDING.1 + 2.0)
        .padding(Edges::xy(PADDING.0, PADDING.1))
        .border(Edges::all(1.0), tokens::LINE_STRONG)
        .radius(tokens::RADIUS_CONTROL)
        .bg(tokens::SURFACE)
        .hover(Look::NONE.with_border(mix(tokens::LINE_STRONG, tokens::FG, 0.3)))
        .focused(Look::NONE.with_border(accent().focus()))
        .ring(Ring::Field)
        .cursor(Cursor::Text)
        .role(Role::TextInput)
        .label(label)
        .field(plain)
        .caret(caret)
        .on_text(move |event| wrote(Writing::Text(event)))
        .on_key(keyed)
        .on_pointer(pointed);
    if empty {
        field = field.description(says);
    }
    field
}
