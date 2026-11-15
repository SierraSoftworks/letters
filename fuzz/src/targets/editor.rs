//! Two writers at their keyboards on one document, sharing what they wrote as they go. Whatever they type,
//! press, paste and drop in, the document stays sound and the selection stays on text, and they end on the
//! same document, also after each has taken back all of its own edits.

use std::collections::BTreeMap;

use letters_editor::{
    Clip, Command, Editor, InputEvent, Key, KeyEvent, Keymap, Modifiers, Motion, Picture, Platform, Spot, TableEdit,
};
use letters_model::{Align, Block, BlockId, BlockKind, Body, ListKind, Mark, Pos, Selection, Tone};

use crate::checks::sound;
use crate::edits::{sample, unique_ids};
use crate::input::Script;

const STEPS: usize = 64;
/// More than a script can make, so that taking everything back ends.
const UNDOS: usize = 4096;

const TYPED: [&str; 28] = [
    "a",
    "b",
    " ",
    " ",
    "é",
    "字",
    "👩‍👧",
    "*",
    "*",
    "`",
    "~",
    "$",
    "$",
    "#",
    "-",
    ">",
    "[",
    "]",
    "(",
    ")",
    "!",
    "=",
    "_",
    "1",
    ".",
    "|",
    "\n",
    "Figure 1",
];
const KEYS: [Key; 24] = [
    Key::Left,
    Key::Right,
    Key::Up,
    Key::Down,
    Key::Home,
    Key::End,
    Key::PageUp,
    Key::PageDown,
    Key::Backspace,
    Key::Backspace,
    Key::Delete,
    Key::Enter,
    Key::Enter,
    Key::Escape,
    Key::Tab,
    Key::Char('a'),
    Key::Char('b'),
    Key::Char('c'),
    Key::Char('e'),
    Key::Char('i'),
    Key::Char('u'),
    Key::Char('v'),
    Key::Char('x'),
    Key::Char('z'),
];
const MOTIONS: [Motion; 14] = [
    Motion::Left,
    Motion::Right,
    Motion::WordLeft,
    Motion::WordRight,
    Motion::LineStart,
    Motion::LineEnd,
    Motion::Up,
    Motion::Down,
    Motion::PageUp,
    Motion::PageDown,
    Motion::BlockStart,
    Motion::BlockEnd,
    Motion::DocStart,
    Motion::DocEnd,
];
const MARKS: [Mark; 7] =
    [Mark::Bold, Mark::Italic, Mark::Underline, Mark::Strike, Mark::Highlight, Mark::Code, Mark::Math];
const TONES: [Tone; 4] = [Tone::Tip, Tone::Info, Tone::Warn, Tone::Danger];
const ALIGNS: [Align; 4] = [Align::Start, Align::Center, Align::End, Align::Justify];

/// The clock, clipboard and ids of one writer's shell.
#[derive(Default)]
struct Desk {
    now: u64,
    clip: Option<Clip>,
    /// What this writer's new ids start with, so that two writers never make the same one.
    writer: usize,
    ids: u32,
    assets: BTreeMap<String, Vec<u8>>,
}

impl Platform for Desk {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn wall_time(&self) -> Option<u64> {
        Some(1_800_000_000 + self.now / 1000)
    }
    fn clipboard_read(&mut self) -> Option<Clip> {
        self.clip.clone()
    }
    fn clipboard_write(&mut self, clip: Clip) {
        self.clip = Some(clip);
    }
    fn new_id(&mut self) -> String {
        self.ids += 1;
        format!("w{}n{}", self.writer, self.ids)
    }
    fn store_asset(&mut self, name: &str, bytes: &[u8]) {
        self.assets.insert(name.to_owned(), bytes.to_vec());
    }
}

struct Writer {
    editor: Editor,
    desk: Desk,
}

impl Writer {
    fn new(mut editor: Editor, writer: usize, keymap: Keymap) -> Self {
        editor.set_keymap(keymap);
        editor.set_author(["Ana", "Ben"][writer]);
        Self { editor, desk: Desk { writer, ..Desk::default() } }
    }

    /// One of the document's blocks, counted in document order and wrapped into range.
    fn block(&self, script: &mut Script) -> Option<BlockId> {
        let ids: Vec<&BlockId> = self.editor.doc().walk().map(|block| &block.id).collect();
        (!ids.is_empty()).then(|| ids[script.pick(ids.len())].clone())
    }

    /// A spot by any block, since one that names nothing a picture can go by is ignored.
    fn spot(&self, script: &mut Script) -> Option<Spot> {
        let (id, after) = (self.block(script)?, script.flag());
        Some(if script.flag() { Spot::Beside { figure: id, after } } else { Spot::Line { block: id, below: after } })
    }

    fn kind(script: &mut Script) -> BlockKind {
        let depth = script.pick(4) as u8;
        match script.pick(10) {
            0 => BlockKind::Paragraph,
            1 => BlockKind::Heading { level: 1 + depth },
            2 => BlockKind::ListItem { list: ListKind::Bullet, depth },
            3 => BlockKind::ListItem { list: ListKind::Ordered, depth },
            4 => BlockKind::ListItem { list: ListKind::Task { checked: script.flag() }, depth },
            5 => BlockKind::Code { language: script.one(&[None, Some("rust")]).map(str::to_owned) },
            6 => BlockKind::Formula,
            7 => BlockKind::Quote,
            8 => BlockKind::callout(script.one(&TONES)),
            _ => BlockKind::Divider,
        }
    }

    fn command(&self, script: &mut Script) -> Option<Command> {
        Some(match script.pick(36) {
            0 | 1 => Command::Move { to: script.one(&MOTIONS), extend: script.flag() },
            2 => Command::Delete(script.one(&MOTIONS)),
            3 => Command::LineBelow,
            4 => Command::Split,
            5 => Command::LeaveFormula,
            6 => Command::LineBreak,
            7 => Command::Indent,
            8 => Command::Outdent,
            9 | 10 => Command::ToggleMark(script.one(&MARKS)),
            11 => Command::SetLink(
                script.one(&[None, Some("https://a.example"), Some("javascript:no")]).map(str::to_owned),
            ),
            12..=14 => Command::SetBlock(Self::kind(script)),
            15 => Command::ToggleChecked,
            16 => Command::InsertTable { rows: script.pick(4), columns: script.pick(4) },
            17..=19 => Command::Table(match script.pick(8) {
                0 => TableEdit::AddRow { below: script.flag() },
                1 => TableEdit::AddColumn { after: script.flag() },
                2 => TableEdit::RemoveRow,
                3 => TableEdit::RemoveColumn,
                4 => TableEdit::MoveRow { to: script.pick(4) },
                5 => TableEdit::MoveColumn { to: script.pick(4) },
                6 => TableEdit::Align(script.one(&ALIGNS)),
                _ => TableEdit::Header(script.flag()),
            }),
            20 => Command::MoveFigure(self.spot(script)?),
            21 => Command::ShiftFigure { right: script.flag() },
            22 => Command::ResizeFigure { width: f32::from(script.byte()) * 3.0, height: f32::from(script.byte()) },
            23 => Command::SetAlt(script.text(8)),
            24 => Command::SetNumbered(script.flag()),
            25 => Command::EditCaption,
            26 => Command::LeaveCaption,
            27 => Command::TakeReference(self.block(script)?),
            28 => Command::DismissReference,
            29 => Command::SelectAll,
            30 => Command::SelectWord,
            31 => Command::SelectBlock,
            32 => Command::SetLanguage(script.one(&["fr", "de-CH", ""]).to_owned()),
            33 => Command::AddWord(script.text(8)),
            34 => Command::RemoveWord(script.text(8)),
            _ => Command::PastePlain,
        })
    }

    /// One thing a writer or their shell does.
    fn step(&mut self, script: &mut Script) {
        match script.pick(20) {
            0..=4 => {
                // Typed a character at a time, as a keyboard does.
                for c in script.one(&TYPED).chars() {
                    self.editor.input(InputEvent::Insert(c.into()), &mut self.desk);
                }
            }
            5..=9 => {
                let bits = script.byte();
                let mods = Modifiers { primary: bits & 1 != 0, shift: bits & 2 != 0, alt: bits & 4 != 0 };
                self.editor.key(KeyEvent { key: script.one(&KEYS), mods }, &mut self.desk);
            }
            10 | 11 => {
                if let Some(command) = self.command(script) {
                    self.editor.run(command, &mut self.desk);
                }
            }
            12 | 13 => {
                let (Some(anchor), Some(head)) = (self.block(script), self.block(script)) else { return };
                let (from, to) = (script.pick(24), script.pick(24));
                self.editor.select(Selection { anchor: Pos::new(anchor, from), head: Pos::new(head, to) });
            }
            14 => {
                // An input method, which may be wrong about the text around the caret.
                let text = script.one(&["", "k", "か", "かな", "fix"]).to_owned();
                let (start, len) = (script.pick(12), script.pick(6));
                let event = match script.pick(4) {
                    0 => InputEvent::SetComposition { text, selection: script.flag().then_some(start..start) },
                    1 => InputEvent::Commit(text),
                    2 => InputEvent::ReplaceRange { range: start..start + len, text },
                    _ => InputEvent::DeleteSurrounding { before: start, after: len },
                };
                self.editor.input(event, &mut self.desk);
            }
            15 => self.desk.now += script.one(&[10, 2_000, 120_000]),
            16 => {
                let seed = self.desk.assets.len() as u8;
                let size = u32::from(script.byte()) * 8;
                // The last may be no picture at all.
                let picture = |n: u8| Picture {
                    bytes: vec![0x89, b'P', b'N', b'G', seed.wrapping_add(n)],
                    mime: if n == 3 { "text/plain" } else { "image/png" }.into(),
                    width: size,
                    height: 300,
                };
                let pictures = (0..script.pick(5) as u8).map(picture).collect();
                let at = if script.flag() { self.spot(script) } else { None };
                self.editor.run(Command::InsertFigures { pictures, at }, &mut self.desk);
            }
            17 => {
                if let Some(figure) = self.block(script) {
                    self.editor.select_picture(&figure);
                }
            }
            _ => {
                // Another application left something on the clipboard.
                let text = script.text(48);
                let html = script.flag().then(|| script.text(96));
                self.desk.clip = Some(Clip { text, html, ..Clip::default() });
                let command = if script.flag() { Command::Paste } else { Command::PastePlain };
                self.editor.run(command, &mut self.desk);
            }
        }
    }

    /// Takes in what `other` has written: only what is new here, or its whole history.
    fn merge(&mut self, other: &Self, whole: bool) {
        let theirs = other.editor.model();
        let history = if whole { theirs.save() } else { theirs.updates_since(&self.editor.model().revision()) };
        let merged = self.editor.merge(&history);
        assert!(merged.is_ok(), "one writer cannot read the other's history: {merged:?}");
    }

    /// Fails unless every block has its own id, the selection sits on characters of text and at no place
    /// inside a reference, and a picture is selected only where there is one.
    fn sound(&self, after: &str) {
        let doc = self.editor.doc();
        assert!(unique_ids(doc), "two blocks share an id after {after}");
        if let Some(selection) = self.editor.selection() {
            for pos in [&selection.anchor, &selection.head] {
                let block = doc.block(&pos.block).filter(|block| matches!(block.body, Body::Text(_)));
                let on_text = block.is_some_and(|block| block.plain_text().is_char_boundary(pos.offset));
                assert!(on_text, "the selection is off the text after {after}: {pos:?}");
                let inside = block.is_some_and(|block| referred(block, pos.offset));
                assert!(!inside, "the selection is inside a reference after {after}: {pos:?}");
            }
        }
        // In a gallery the caret is always on a picture, since the pictures there have no caption of their own.
        let head = self.editor.selection().filter(|selection| selection.is_caret()).map(|selection| &selection.head);
        let on =
            head.and_then(|head| doc.block(&head.block)).filter(|block| matches!(block.kind, BlockKind::Figure { .. }));
        let member = |id: &BlockId| {
            let holds = |gallery: &Block| gallery.body.blocks().iter().any(|part| part.id == *id);
            doc.walk().any(|block| matches!(block.kind, BlockKind::Gallery { .. }) && holds(block))
        };
        match self.editor.picture() {
            Some(picture) => {
                assert!(
                    on.is_some_and(|block| block.id == *picture),
                    "a picture that is no figure is selected after {after}"
                )
            }
            None => {
                assert!(!on.is_some_and(|block| member(&block.id)), "a caret in a picture of a gallery after {after}")
            }
        }
    }
}

/// Whether `offset` is between two characters of one reference.
fn referred(block: &Block, offset: usize) -> bool {
    let (mut end, mut before, mut after) = (0, None, None);
    for run in block.body.runs() {
        let start = end;
        end += run.text.len();
        let to = run.marks.iter().find(|mark| matches!(mark, Mark::Ref(_)));
        if start < offset && offset <= end {
            before = to;
        }
        if start <= offset && offset < end {
            after = to;
        }
    }
    before.is_some() && before == after
}

pub fn run(data: &[u8]) {
    let mut script = Script::new(data);
    let first = Editor::new(sample(), 1.into());
    let second = Editor::with_model(first.model().fork(2.into()));
    // One writer on each keymap.
    let mut writers = [Writer::new(first, 0, Keymap::standard()), Writer::new(second, 1, Keymap::mac())];

    for _ in 0..STEPS {
        if script.done() {
            break;
        }
        let turn = script.pick(2);
        writers[turn].step(&mut script);
        writers[turn].sound("a step");
        if script.flag() {
            let (writer, other) = pair(&mut writers, turn);
            other.merge(writer, script.flag());
            other.sound("a merge");
            // Storing a document is slow beside a keystroke, so it is tried as often as a save would.
            sound(other.editor.doc(), "a merge");
        }
    }

    // Each writer may also take back all of its own edits, whatever the other has done since.
    for turn in 0..2 {
        let (writer, other) = pair(&mut writers, turn);
        let mut undone = 0;
        while writer.editor.can_undo() && undone < UNDOS {
            writer.editor.run(Command::Undo, &mut writer.desk);
            writer.sound("an undo");
            undone += 1;
        }
        assert!(undone < UNDOS, "undo never runs out");
        sound(writer.editor.doc(), "taking everything back");
        other.merge(writer, true);
        other.sound("a merge");
    }
    let (first, second) = pair(&mut writers, 0);
    first.merge(second, true);
    sound(first.editor.doc(), "the last merge");
    assert_eq!(first.editor.doc(), second.editor.doc(), "two writers who have each other's edits differ");
}

/// The writer whose turn it is, and the other.
fn pair(writers: &mut [Writer; 2], turn: usize) -> (&mut Writer, &mut Writer) {
    let (first, second) = writers.split_at_mut(1);
    if turn == 0 { (&mut first[0], &mut second[0]) } else { (&mut second[0], &mut first[0]) }
}

pub fn seeds() -> Vec<Vec<u8>> {
    // Typing, keys with every modifier, commands, selections and a paste, shared now and then.
    let typed = (0..28u8).flat_map(|piece| [piece & 1, 0, piece, piece & 1]);
    let keys = (0..24u8).flat_map(|key| [0, 5, key % 8, key, key & 1]);
    let commands = (0..36u8).flat_map(|command| [1, 10, command, 1, 2, 3, 1]);
    let picked = (0..12u8).flat_map(|block| [0, 12, block, block + 3, 2, 9, 0, 5, 0, 8, 1]);
    let pasted: Vec<u8> = [&[0, 19, 9][..], b"one\n- two", &[1, 22], b"<p>one <b>two</b></p>", &[1, 1]].concat();
    let dropped = [1, 16, 50, 3, 0, 1, 0, 10, 20, 4, 1, 1, 0, 10, 23, 2, 65, 66, 1];
    vec![typed.collect(), keys.collect(), commands.collect(), picked.collect(), pasted, dropped.to_vec()]
}
