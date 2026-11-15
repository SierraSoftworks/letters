//! Shared fixtures: a scripted editor, and a one-line-per-block notation for documents with a selection.
//!
//! Each line is a block: a kind (`p`, `h1` to `h4`, `ul`, `ol`, `todo`, `done`, `code`, `code:rust`, `formula`,
//! `hr`, `cap`, `fig`) and its text. Two spaces before the kind nest a list item one level. `> ` puts the block
//! in a quote, and `tip> `, `info> `, `warn> ` or `danger> ` in a callout. In the text, `**`, `*`, `~~`, `` ` ``,
//! `__`, `==` and `$` switch bold, italic, strike, code, underline, highlight and maths, `[text](address)` is a
//! link, `⏎` is a line break, `|` is the caret and `^` the other end of a selection. A backslash makes the next
//! character text.
//!
//! A figure's text is its caption. `fig#` is numbered, and a `*` after the kind says its picture is selected.
//! `g> ` puts a figure or a caption in a gallery, and `g#> ` in a numbered one. A cell is `td`, or `th` in a
//! row that heads its table, after the row it is in: `r1> `, `r2> ` and so on. Consecutive rows are one table.
#![allow(dead_code)]

use std::collections::BTreeSet;

use letters_editor::{
    Clip, Command, Editor, InputEvent, Key, KeyEvent, Keymap, Lines, Marker, Modifiers, Platform, TableEdit,
};
use letters_model::{
    Align, Block, BlockId, BlockKind, Body, Change, Document, ListKind, Mark, Model, Pos, ReplicaId, Run, Selection,
    Tone,
};
use proptest::prelude::*;

/// The marks the notation can write, with their switches. Longer switches come first so they are read first.
const SWITCHES: [(&str, Mark); 7] = [
    ("**", Mark::Bold),
    ("~~", Mark::Strike),
    ("__", Mark::Underline),
    ("==", Mark::Highlight),
    ("*", Mark::Italic),
    ("`", Mark::Code),
    ("$", Mark::Math),
];
const SPECIAL: &str = "\\*~`_=$[]|^⏎";
const TONES: [(&str, Tone); 4] =
    [("tip", Tone::Tip), ("info", Tone::Info), ("warn", Tone::Warn), ("danger", Tone::Danger)];

/// The clock, clipboard and ids a test controls, with an optional fake layout.
#[derive(Default)]
pub struct Fake {
    pub now: u64,
    /// The time of day, for a writer whose edits are dated.
    pub clock: Option<u64>,
    pub clip: Option<Clip>,
    pub wrap: Option<Wrapped>,
    /// What this writer's new ids start with, so that two writers never make the same one.
    pub writer: String,
    ids: u32,
}

impl Platform for Fake {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn wall_time(&self) -> Option<u64> {
        self.clock
    }
    fn clipboard_read(&mut self) -> Option<Clip> {
        self.clip.clone()
    }
    fn clipboard_write(&mut self, clip: Clip) {
        self.clip = Some(clip);
    }
    fn new_id(&mut self) -> String {
        self.ids += 1;
        format!("{}n{}", self.writer, self.ids)
    }
    fn lines(&self) -> Option<&dyn Lines> {
        self.wrap.as_ref().map(|wrap| wrap as &dyn Lines)
    }
}

/// A layout that wraps every block after `width` bytes, for text of one byte per character.
pub struct Wrapped {
    pub width: usize,
    pub doc: Document,
}

impl Wrapped {
    fn len(&self, pos: &Pos) -> usize {
        self.doc.block(&pos.block).map_or(0, |block| block.plain_text().len())
    }

    /// The line an offset is on. The end of a full last line belongs to that line, not to one after it.
    fn line(&self, pos: &Pos) -> usize {
        let at_full_end = pos.offset > 0 && pos.offset == self.len(pos) && pos.offset.is_multiple_of(self.width);
        pos.offset / self.width - usize::from(at_full_end)
    }
}

impl Lines for Wrapped {
    fn caret_x(&self, pos: &Pos) -> Option<f32> {
        Some((pos.offset - self.line(pos) * self.width) as f32)
    }

    fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        let texts: Vec<&Block> = self.doc.walk().filter(|block| matches!(block.body, Body::Text(_))).collect();
        let index = texts.iter().position(|block| block.id == pos.block)?;
        let last_line = |block: &Block| block.plain_text().len().saturating_sub(1) / self.width;
        let (index, line) = match (down, self.line(pos)) {
            (true, line) if line < last_line(texts[index]) => (index, line + 1),
            (true, _) => (index + 1, 0),
            (false, 0) => (index.checked_sub(1)?, last_line(texts[index.checked_sub(1)?])),
            (false, line) => (index, line - 1),
        };
        let block = texts.get(index)?;
        let offset = (line * self.width + x as usize).min((line + 1) * self.width).min(block.plain_text().len());
        Some(Pos::new(block.id.clone(), offset))
    }

    fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos> {
        let start = self.line(pos) * self.width;
        let offset = if end { (start + self.width).min(self.len(pos)) } else { start };
        Some(Pos::new(pos.block.clone(), offset))
    }

    fn page_lines(&self) -> usize {
        3
    }
}

/// One step of a script: text to type a character at a time, or a key such as `Shift+Left` or `Mod+b`.
#[derive(Clone, Copy, Debug)]
pub enum Do {
    T(&'static str),
    K(&'static str),
}

/// An editor and the platform it is driven with.
pub struct Harness {
    pub editor: Editor,
    pub platform: Fake,
}

impl Harness {
    /// An editor on the document that `state` describes, with the standard keymap on every system.
    pub fn new(state: &str) -> Self {
        let (doc, selection, picture) = read(state);
        let mut editor = Editor::new(doc, ReplicaId::from(1));
        editor.set_keymap(Keymap::standard());
        if let Some(selection) = selection {
            editor.select(selection);
        }
        if let Some(picture) = picture {
            editor.select_picture(&picture);
        }
        Self { editor, platform: Fake::default() }
    }

    /// An editor on a model that comes with its history. `writer` tells its new ids from other writers'.
    pub fn on(model: Model, writer: u64) -> Self {
        let mut editor = Editor::with_model(model);
        editor.set_keymap(Keymap::standard());
        Self { editor, platform: Fake { writer: format!("w{writer}"), ..Fake::default() } }
    }

    /// A second writer on the same document, as it stands now.
    pub fn fork(&self, replica: u64) -> Self {
        Self::on(self.editor.model().fork(ReplicaId::from(replica)), replica)
    }

    /// Takes in everything `other` has written. Returns whether its history could be read.
    pub fn merge(&mut self, other: &Self) -> bool {
        self.editor.merge(&other.editor.model().save()).is_ok()
    }

    /// The same, laid out by a fake layout that wraps after `width` characters.
    pub fn wrapped(state: &str, width: usize) -> Self {
        let mut harness = Self::new(state);
        harness.platform.wrap = Some(Wrapped { width, doc: harness.editor.doc().clone() });
        harness
    }

    /// The document and selection in the notation.
    pub fn state(&self) -> String {
        render(self.editor.doc(), self.editor.selection(), self.editor.picture())
    }

    /// The document alone.
    pub fn text(&self) -> String {
        render(self.editor.doc(), None, None)
    }

    pub fn input(&mut self, event: InputEvent) -> Change {
        self.relayout();
        self.editor.input(event, &mut self.platform)
    }

    /// Types text one character at a time, as a keyboard does.
    pub fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.input(InputEvent::Insert(c.into()));
        }
    }

    /// Presses a key. Returns whether the editor had a use for it.
    pub fn key(&mut self, name: &str) -> bool {
        self.relayout();
        self.editor.key(key(name), &mut self.platform).is_some()
    }

    pub fn run(&mut self, command: Command) -> Change {
        self.relayout();
        self.editor.run(command, &mut self.platform)
    }

    pub fn play(&mut self, script: &[Do]) {
        for step in script {
            match step {
                Do::T(text) => self.type_text(text),
                Do::K(name) => _ = self.key(name),
            }
        }
    }

    pub fn marks(&self) -> BTreeSet<Mark> {
        self.editor.marks()
    }

    /// The caret's block as it is drawn while the caret is in it: its text with the revealed markers, and
    /// `|` for the caret.
    pub fn revealed(&self) -> String {
        let markers = self.editor.markers();
        let Some(head) = self.editor.selection().map(|selection| &selection.head) else { return String::new() };
        let text = self.editor.doc().block(&head.block).map(Block::plain_text).unwrap_or_default();
        // Markers come in drawing order, and the caret goes between two of them.
        let marker = |(i, marker): (usize, &Marker)| (marker.offset, 2 * i + 1, marker.text);
        let mut tokens: Vec<(usize, usize, &str)> = markers.inline.iter().enumerate().map(marker).collect();
        tokens.push((head.offset, 2 * markers.before_caret, "|"));
        tokens.sort();
        let mut tokens = tokens.into_iter().peekable();
        let mut out = String::new();
        for (offset, c) in text.char_indices().map(|(i, c)| (i, Some(c))).chain([(text.len(), None)]) {
            while let Some((_, _, token)) = tokens.next_if(|(at, _, _)| *at <= offset) {
                out.push_str(token);
            }
            out.extend(c);
        }
        out
    }

    fn relayout(&mut self) {
        if let Some(wrap) = &mut self.platform.wrap {
            wrap.doc = self.editor.doc().clone();
        }
    }
}

/// What another application leaves on the clipboard.
pub fn clip(text: &str, html: Option<&str>) -> Clip {
    Clip { text: text.into(), html: html.map(str::to_owned), ..Clip::default() }
}

/// `Mod+Shift+z`, `Alt+Left`, `Enter`: modifiers, then a named key or a character.
pub fn key(name: &str) -> KeyEvent {
    let mut parts: Vec<&str> = name.split('+').collect();
    let last = parts.pop().expect("a key has a name");
    let mods =
        Modifiers { primary: parts.contains(&"Mod"), shift: parts.contains(&"Shift"), alt: parts.contains(&"Alt") };
    let key = match last {
        "Enter" => Key::Enter,
        "Tab" => Key::Tab,
        "Backspace" => Key::Backspace,
        "Delete" => Key::Delete,
        "Escape" => Key::Escape,
        "Left" => Key::Left,
        "Right" => Key::Right,
        "Up" => Key::Up,
        "Down" => Key::Down,
        "Home" => Key::Home,
        "End" => Key::End,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        other => Key::Char(other.chars().next().expect("a key has a name")),
    };
    KeyEvent { key, mods }
}

/// The notation as the fixture itself writes it, so that a test may place the caret on either side of a
/// switch.
pub fn canon(state: &str) -> String {
    let (doc, selection, picture) = read(state);
    render(&doc, selection.as_ref(), picture.as_ref())
}

/// What a line of the notation is put in.
enum Outer {
    Holder(BlockKind),
    Row(usize),
}

/// The figure that the notation's `fig` stands for.
pub fn picture(id: &str, numbered: bool) -> BlockKind {
    let asset = Some(format!("assets/{id}.png"));
    BlockKind::Figure { asset, src: None, alt: String::new(), width: 240.0, height: 160.0, numbered }
}

fn holding(id: String, kind: BlockKind, parts: Vec<Block>) -> Block {
    Block { body: Body::Blocks(parts), ..Block::new(id.as_str(), kind) }
}

/// Reads the notation, without the picture it may say is selected.
pub fn parse(state: &str) -> (Document, Option<Selection>) {
    let (doc, selection, _) = read(state);
    (doc, selection)
}

/// Reads the notation. Blocks are named `b1`, `b2` and so on in the order they are met, a block before what
/// holds it.
pub fn read(state: &str) -> (Document, Option<Selection>, Option<BlockId>) {
    let mut blocks: Vec<Block> = Vec::new();
    let (mut anchor, mut head, mut selected) = (None, None, None);
    let mut count = 0;
    let mut id = || {
        count += 1;
        format!("b{count}")
    };
    // The number of the row that the line before was in.
    let mut row = 0;
    for line in state.lines() {
        let numbered = |row: &str| row.strip_prefix('r').and_then(|number| number.parse::<usize>().ok());
        let (container, line) = match line.split_once("> ") {
            Some(("", rest)) => (Some(Outer::Holder(BlockKind::Quote)), rest),
            Some(("g", rest)) => (Some(Outer::Holder(BlockKind::Gallery { numbered: false })), rest),
            Some(("g#", rest)) => (Some(Outer::Holder(BlockKind::Gallery { numbered: true })), rest),
            Some((tone, rest)) if TONES.iter().any(|(name, _)| *name == tone) => {
                let tone = TONES.iter().find(|(name, _)| *name == tone).expect("checked above").1;
                (Some(Outer::Holder(BlockKind::Callout { tone })), rest)
            }
            Some((name, rest)) if numbered(name).is_some() => (numbered(name).map(Outer::Row), rest),
            _ => (None, line),
        };
        let depth = (line.len() - line.trim_start_matches(' ').len()) / 2;
        let (kind, text) = line.trim_start_matches(' ').split_once(' ').unwrap_or((line.trim_start_matches(' '), ""));
        let list = |list| BlockKind::ListItem { list, depth: depth as u8 };
        let name = id();
        let (kind, on) = kind.strip_suffix('*').map_or((kind, false), |kind| (kind, true));
        let heads = kind == "th";
        let kind = match kind {
            "p" => BlockKind::Paragraph,
            "cap" => BlockKind::Caption,
            "fig" => picture(&name, false),
            "fig#" => picture(&name, true),
            "td" | "th" => BlockKind::TableCell,
            "h1" | "h2" | "h3" | "h4" => BlockKind::Heading { level: kind[1..].parse().expect("a heading level") },
            "ul" => list(ListKind::Bullet),
            "ol" => list(ListKind::Ordered),
            "todo" => list(ListKind::Task { checked: false }),
            "done" => list(ListKind::Task { checked: true }),
            "hr" => BlockKind::Divider,
            "formula" => BlockKind::Formula,
            "code" => BlockKind::Code { language: None },
            other => match other.strip_prefix("code:") {
                Some(language) => BlockKind::Code { language: Some(language.to_owned()) },
                None => panic!("unknown block kind {other:?} in {state:?}"),
            },
        };

        let mut block = Block::new(name.as_str(), kind);
        if matches!(block.body, Body::Text(_)) {
            let (runs, at_anchor, at_head) = inline(text);
            block.body = Body::Text(runs);
            anchor = at_anchor.map(|offset| Pos::new(block.id.clone(), offset)).or(anchor);
            head = at_head.map(|offset| Pos::new(block.id.clone(), offset)).or(head);
        }
        if on {
            selected = Some(block.id.clone());
        }
        let last = (row, std::mem::take(&mut row)).0;
        match (container, blocks.last_mut()) {
            (None, _) => blocks.push(block),
            (Some(Outer::Row(number)), open) => {
                row = number;
                match open {
                    // Consecutive rows share one table, and consecutive cells one row.
                    Some(Block { kind: BlockKind::Table { columns, header }, body: Body::Blocks(rows), .. })
                        if last > 0 && number >= last =>
                    {
                        *header |= heads;
                        match rows.last_mut() {
                            Some(Block { body: Body::Blocks(cells), .. }) if number == last => cells.push(block),
                            _ => rows.push(holding(id(), BlockKind::TableRow, vec![block])),
                        }
                        let width = rows.first().map_or(0, |row| row.body.blocks().len());
                        *columns = vec![Align::Start; width];
                    }
                    _ => {
                        let cells = holding(id(), BlockKind::TableRow, vec![block]);
                        let table = BlockKind::Table { columns: vec![Align::Start], header: heads };
                        blocks.push(holding(id(), table, vec![cells]));
                    }
                }
            }
            // Consecutive lines with the same prefix share one quote, callout or gallery.
            (Some(Outer::Holder(kind)), Some(Block { kind: open, body: Body::Blocks(children), .. }))
                if *open == kind =>
            {
                children.push(block)
            }
            (Some(Outer::Holder(kind)), _) => blocks.push(holding(id(), kind, vec![block])),
        }
    }
    let selection = head.map(|head| Selection { anchor: anchor.unwrap_or_else(|| head.clone()), head });
    (Document { blocks, ..Document::default() }, selection, selected)
}

/// The runs a line of text describes, and the offsets of `^` and `|` in it.
fn inline(text: &str) -> (Vec<Run>, Option<usize>, Option<usize>) {
    let mut runs: Vec<Run> = Vec::new();
    let mut marks: BTreeSet<Mark> = BTreeSet::new();
    let (mut anchor, mut head, mut linked) = (None, None, None);
    let len = |runs: &[Run]| runs.iter().map(|run| run.text.len()).sum::<usize>();
    // A character joins the run before it unless the marks changed or that run is a finished link.
    let push = |runs: &mut Vec<Run>, c: char, marks: &BTreeSet<Mark>| match runs
        .last_mut()
        .filter(|run| run.marks == *marks && run.link.is_none())
    {
        Some(run) => run.text.push(c),
        None => runs.push(Run::marked(c, marks.clone())),
    };
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some((switch, mark)) = SWITCHES.iter().find(|(switch, _)| rest.starts_with(switch)) {
            if !marks.remove(mark) {
                marks.insert(mark.clone());
            }
            rest = &rest[switch.len()..];
            continue;
        }
        rest = &rest[c.len_utf8()..];
        match c {
            '\\' => {
                let literal = rest.chars().next().expect("a backslash comes before a character");
                rest = &rest[literal.len_utf8()..];
                push(&mut runs, literal, &marks);
            }
            '⏎' => push(&mut runs, '\n', &marks),
            '^' => anchor = Some(len(&runs)),
            '|' => head = Some(len(&runs)),
            '[' => {
                // An empty run keeps the text before the link out of it.
                runs.push(Run::marked("", marks.clone()));
                linked = Some(runs.len() - 1);
            }
            ']' => {
                let (address, after) = rest[1..].split_once(')').expect("a link has an address");
                let from = linked.take().expect("a link opens before it closes");
                runs[from..].iter_mut().for_each(|run| run.link = Some(address.to_owned()));
                rest = after;
            }
            c => push(&mut runs, c, &marks),
        }
    }
    runs.retain(|run| !run.text.is_empty());
    (runs, anchor, head)
}

/// Writes the notation. Runs with the same formatting read as one, however the model split them.
pub fn render(doc: &Document, selection: Option<&Selection>, picture: Option<&BlockId>) -> String {
    let mut lines = Vec::new();
    let line = |block: &Block| line(block, selection, picture);
    for block in &doc.blocks {
        match (&block.kind, &block.body) {
            (BlockKind::Quote, Body::Blocks(children)) => {
                lines.extend(children.iter().map(|child| format!("> {}", line(child))));
            }
            (BlockKind::Callout { tone }, Body::Blocks(children)) => {
                let name = TONES.iter().find(|(_, other)| other == tone).expect("every tone has a name").0;
                lines.extend(children.iter().map(|child| format!("{name}> {}", line(child))));
            }
            (BlockKind::Gallery { numbered }, Body::Blocks(children)) => {
                let name = if *numbered { "g#" } else { "g" };
                lines.extend(children.iter().map(|child| format!("{name}> {}", line(child))));
            }
            (BlockKind::Table { header, .. }, Body::Blocks(rows)) => {
                for (i, row) in rows.iter().enumerate() {
                    let cell = |cell: &Block| match *header && i == 0 {
                        true => format!("r{}> {}", i + 1, line(cell).replacen("td", "th", 1)),
                        false => format!("r{}> {}", i + 1, line(cell)),
                    };
                    lines.extend(row.body.blocks().iter().map(cell));
                }
            }
            _ => lines.push(line(block)),
        }
    }
    lines.join("\n")
}

fn line(block: &Block, selection: Option<&Selection>, picture: Option<&BlockId>) -> String {
    // A selected picture is marked on its kind, and the caret that stands for it is not drawn.
    let on = picture == Some(&block.id);
    let selection = selection.filter(|_| !on);
    let (indent, kind) = match &block.kind {
        BlockKind::Caption => (0, "cap".to_owned()),
        BlockKind::Figure { numbered, .. } => {
            (0, format!("fig{}{}", if *numbered { "#" } else { "" }, if on { "*" } else { "" }))
        }
        BlockKind::TableCell => (0, "td".to_owned()),
        BlockKind::Paragraph => (0, "p".to_owned()),
        BlockKind::Heading { level } => (0, format!("h{level}")),
        BlockKind::ListItem { list, depth } => {
            let kind = match list {
                ListKind::Bullet => "ul",
                ListKind::Ordered => "ol",
                ListKind::Task { checked: false } => "todo",
                ListKind::Task { checked: true } => "done",
            };
            (usize::from(*depth), kind.to_owned())
        }
        BlockKind::Code { language: None } => (0, "code".to_owned()),
        BlockKind::Code { language: Some(language) } => (0, format!("code:{language}")),
        BlockKind::Divider => (0, "hr".to_owned()),
        BlockKind::Formula => (0, "formula".to_owned()),
        other => (0, format!("{other:?}")),
    };
    let at = |pos: &Pos| (pos.block == block.id).then_some(pos.offset);
    let head = selection.and_then(|selection| at(&selection.head));
    let anchor = selection.filter(|selection| !selection.is_caret()).and_then(|selection| at(&selection.anchor));

    // Each character with its formatting, then an unformatted end so that everything open is closed.
    let runs = block.body.runs().iter();
    let chars = runs.flat_map(|run| run.text.chars().map(move |c| (Some(c), &run.marks, &run.link)));
    let (plain, unlinked) = (BTreeSet::new(), None);
    let mut text = String::new();
    let (mut marks, mut link) = (&plain, &unlinked);
    let mut offset = 0;
    for (c, next_marks, next_link) in chars.chain([(None, &plain, &unlinked)]) {
        // The ends of the selection are written before anything else at their offset.
        if anchor == Some(offset) {
            text.push('^');
        }
        if head == Some(offset) {
            text.push('|');
        }
        if let Some(address) = link.as_ref().filter(|_| link != next_link) {
            text.push_str(&format!("]({address})"));
        }
        for (switch, mark) in &SWITCHES {
            if marks.contains(mark) != next_marks.contains(mark) {
                text.push_str(switch);
            }
        }
        if next_link.is_some() && link != next_link {
            text.push('[');
        }
        (marks, link) = (next_marks, next_link);
        match c {
            Some('\n') => text.push('⏎'),
            Some(c) if SPECIAL.contains(c) => text.extend(['\\', c]),
            Some(c) => text.push(c),
            None => {}
        }
        offset += c.map_or(0, char::len_utf8);
    }
    format!("{}{kind}{}{text}", "  ".repeat(indent), if text.is_empty() { "" } else { " " })
}

/// One thing a writer or a shell can do to the editor.
#[derive(Clone, Debug)]
pub enum Action {
    Type(String),
    Key(&'static str),
    Run(Command),
    /// Selects from one block and offset to another, both wrapped into range.
    Select(usize, usize, usize, usize),
    Input(InputEvent),
    Wait(u64),
}

pub fn action() -> impl Strategy<Value = Action> {
    let typed = prop::sample::select(vec![
        "a",
        "b",
        " ",
        " ",
        "é",
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
        "x",
        "1",
        ".",
        "|",
        "|",
        "\n",
    ]);
    let keys = prop::sample::select(vec![
        "Left",
        "Right",
        "Up",
        "Down",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Mod+Left",
        "Mod+Right",
        "Mod+Up",
        "Mod+Down",
        "Mod+Home",
        "Mod+End",
        "Shift+Left",
        "Shift+Right",
        "Shift+Up",
        "Shift+Down",
        "Shift+Mod+Left",
        "Shift+Mod+End",
        "Backspace",
        "Backspace",
        "Delete",
        "Mod+Backspace",
        "Mod+Delete",
        "Enter",
        "Enter",
        "Escape",
        "Shift+Enter",
        "Mod+Enter",
        "Tab",
        "Shift+Tab",
        "Mod+b",
        "Mod+i",
        "Mod+e",
        "Mod+Shift+x",
        "Mod+u",
        "Mod+z",
        "Mod+z",
        "Mod+Shift+z",
        "Mod+a",
        "Mod+c",
        "Mod+x",
        "Mod+v",
        "Mod+v",
        "Mod+Shift+v",
    ]);
    let kinds = prop::sample::select(vec![
        BlockKind::Paragraph,
        BlockKind::Heading { level: 2 },
        BlockKind::ListItem { list: ListKind::Bullet, depth: 0 },
        BlockKind::ListItem { list: ListKind::Task { checked: true }, depth: 0 },
        BlockKind::Code { language: None },
        BlockKind::Quote,
        BlockKind::Callout { tone: Tone::Tip },
        BlockKind::Divider,
    ]);
    let commands = prop_oneof![
        kinds.prop_map(Command::SetBlock),
        Just(Command::SetLink(Some("https://a.example".into()))),
        Just(Command::SetLink(None)),
        Just(Command::SelectWord),
        Just(Command::SelectBlock),
        Just(Command::ToggleMark(Mark::Highlight)),
        Just(Command::AddWord("mot".into())),
        Just(Command::RemoveWord("mot".into())),
        Just(Command::SetLanguage("fr".into())),
        Just(Command::InsertTable { rows: 2, columns: 2 }),
        prop::sample::select(vec![
            TableEdit::AddRow { below: true },
            TableEdit::AddRow { below: false },
            TableEdit::AddColumn { after: true },
            TableEdit::AddColumn { after: false },
            TableEdit::RemoveRow,
            TableEdit::RemoveColumn,
            TableEdit::MoveRow { to: 0 },
            TableEdit::MoveRow { to: 2 },
            TableEdit::MoveColumn { to: 0 },
            TableEdit::MoveColumn { to: 2 },
            TableEdit::Align(Align::Center),
            TableEdit::Header(false),
        ])
        .prop_map(Command::Table),
    ];
    let composed = prop::sample::select(vec!["", "k", "か", "かな"]);
    let inputs = prop_oneof![
        (composed.clone(), 0..8usize)
            .prop_map(|(text, at)| InputEvent::SetComposition { text: text.into(), selection: Some(at..at) }),
        composed.prop_map(|text| InputEvent::Commit(text.into())),
        (0..12usize, 0..6usize)
            .prop_map(|(start, len)| InputEvent::ReplaceRange { range: start..start + len, text: "fix".into() }),
        (0..4usize, 0..4usize).prop_map(|(before, after)| InputEvent::DeleteSurrounding { before, after }),
    ];
    prop_oneof![
        4 => typed.prop_map(|text| Action::Type(text.into())),
        4 => keys.prop_map(Action::Key),
        1 => commands.prop_map(Action::Run),
        2 => (0..40usize, 0..24usize, 0..40usize, 0..24usize).prop_map(|(a, b, c, d)| Action::Select(a, b, c, d)),
        1 => inputs.prop_map(Action::Input),
        1 => prop::sample::select(vec![10u64, 2_000]).prop_map(Action::Wait),
    ]
}

pub fn perform(editor: &mut Harness, action: &Action) {
    match action {
        Action::Type(text) => editor.type_text(text),
        Action::Key(name) => _ = editor.editor.key(key(name), &mut editor.platform),
        Action::Run(command) => _ = editor.run(command.clone()),
        Action::Input(event) => _ = editor.input(event.clone()),
        Action::Wait(ms) => editor.platform.now += ms,
        Action::Select(a, b, c, d) => {
            let ids: Vec<_> = editor.editor.doc().walk().map(|block| block.id.clone()).collect();
            if !ids.is_empty() {
                let pos = |block: &usize, offset: &usize| Pos::new(ids[block % ids.len()].clone(), *offset);
                editor.editor.select(Selection { anchor: pos(a, b), head: pos(c, d) });
            }
        }
    }
}

/// What must hold after anything one writer does: [`sound`], and the blocks made of parts are whole. Two
/// writers at once can leave a table ragged or a gallery with one picture, which the model allows.
pub fn check(editor: &Harness) -> Result<(), TestCaseError> {
    sound(editor)?;
    let doc = editor.editor.doc();
    for block in doc.walk() {
        let parts = block.body.blocks();
        match &block.kind {
            BlockKind::Table { columns, .. } => {
                let whole = |row: &Block| row.kind == BlockKind::TableRow && row.body.blocks().len() == columns.len();
                prop_assert!(!parts.is_empty() && !columns.is_empty() && parts.iter().all(whole), "a table is ragged");
            }
            BlockKind::Gallery { .. } => {
                let pictures = parts.iter().filter(|part| matches!(part.kind, BlockKind::Figure { .. })).count();
                prop_assert!(pictures >= 2, "a gallery has fewer than two pictures");
                let captions = parts.iter().filter(|part| part.kind == BlockKind::Caption).count();
                let last = parts.last().is_some_and(|part| part.kind == BlockKind::Caption);
                prop_assert!(captions == 0 || (captions == 1 && last), "a gallery's caption is not one at its end");
            }
            _ => {}
        }
    }
    Ok(())
}

/// What must hold after anything at all: every block has its own id, the document can be stored, the
/// selection sits on characters of text, and a picture is selected only where there is one.
pub fn sound(editor: &Harness) -> Result<(), TestCaseError> {
    let doc = editor.editor.doc();
    let ids: BTreeSet<_> = doc.walk().map(|block| &block.id).collect();
    prop_assert_eq!(ids.len(), doc.walk().count(), "two blocks share an id");
    if let Some(selection) = editor.editor.selection() {
        for pos in [&selection.anchor, &selection.head] {
            let block = doc.block(&pos.block).filter(|block| matches!(block.body, Body::Text(_)));
            let text = block.map(|block| block.plain_text());
            prop_assert!(text.is_some_and(|text| text.is_char_boundary(pos.offset)), "the selection is off the text");
        }
    }
    let stored = Document::from_json(&doc.to_json());
    prop_assert_eq!(stored.as_ref(), Ok(doc), "the document cannot be stored");
    let figure = |block: &&Block| matches!(block.kind, BlockKind::Figure { .. });
    // In a gallery the caret is always on a picture, since the pictures there have no caption of their own.
    let head = editor.editor.selection().filter(|selection| selection.is_caret()).map(|selection| &selection.head);
    let on = head.and_then(|head| doc.block(&head.block)).filter(figure);
    let member = |id: &BlockId| {
        doc.walk().any(|block| {
            matches!(block.kind, BlockKind::Gallery { .. }) && block.body.blocks().iter().any(|part| part.id == *id)
        })
    };
    match editor.editor.picture() {
        Some(picture) => prop_assert!(on.is_some_and(|block| block.id == *picture), "a picture that is no figure"),
        None => prop_assert!(!on.is_some_and(|block| member(&block.id)), "a caret in a picture of a gallery"),
    }
    Ok(())
}

/// A document with every kind of block and mark, for scripts to run on.
pub const SAMPLE: &str = "h1 A *title*|\np Some **bold** and [linked](https://a.example) text, ünïcödé 👩‍👧.\nul one\n  todo two\nhr\n> p quoted\n> p lines\ntip> p a tip\ncode:rust fn main() {⏎    body();⏎}\nfig Boats at sea\ng> fig\ng> fig\ng> cap Two more\nr1> th Boat\nr1> th Length\nr2> td Skiff\nr2> td 4 m\np";
