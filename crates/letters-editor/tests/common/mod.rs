//! Shared fixtures: a scripted editor, and a one-line-per-block notation for documents with a selection.
//!
//! Each line is a block: a kind (`p`, `h1` to `h4`, `ul`, `ol`, `todo`, `done`, `code`, `code:rust`, `hr`) and
//! its text. Two spaces before the kind nest a list item one level. `> ` puts the block in a quote, and
//! `tip> `, `info> `, `warn> ` or `danger> ` in a callout. In the text, `**`, `*`, `~~`, `` ` ``, `__` and `==`
//! switch bold, italic, strike, code, underline and highlight, `[text](address)` is a link, `⏎` is a line
//! break, `|` is the caret and `^` the other end of a selection. A backslash makes the next character text.
#![allow(dead_code)]

use std::collections::BTreeSet;

use letters_editor::{Clip, Command, Editor, InputEvent, Key, KeyEvent, Keymap, Lines, Marker, Modifiers, Platform};
use letters_model::{Block, BlockKind, Body, Change, Document, ListKind, Mark, Pos, ReplicaId, Run, Selection, Tone};

/// The marks the notation can write, with their switches. Longer switches come first so they are read first.
const SWITCHES: [(&str, Mark); 6] = [
    ("**", Mark::Bold),
    ("~~", Mark::Strike),
    ("__", Mark::Underline),
    ("==", Mark::Highlight),
    ("*", Mark::Italic),
    ("`", Mark::Code),
];
const SPECIAL: &str = "\\*~`_=[]|^⏎";
const TONES: [(&str, Tone); 4] =
    [("tip", Tone::Tip), ("info", Tone::Info), ("warn", Tone::Warn), ("danger", Tone::Danger)];

/// The clock, clipboard and ids a test controls, with an optional fake layout.
#[derive(Default)]
pub struct Fake {
    pub now: u64,
    pub clip: Option<Clip>,
    pub wrap: Option<Wrapped>,
    ids: u32,
}

impl Platform for Fake {
    fn now_ms(&self) -> u64 {
        self.now
    }
    fn clipboard_read(&mut self) -> Option<Clip> {
        self.clip.clone()
    }
    fn clipboard_write(&mut self, clip: Clip) {
        self.clip = Some(clip);
    }
    fn new_id(&mut self) -> String {
        self.ids += 1;
        format!("n{}", self.ids)
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
        let (doc, selection) = parse(state);
        let mut editor = Editor::new(doc, ReplicaId::from(1));
        editor.set_keymap(Keymap::standard());
        if let Some(selection) = selection {
            editor.select(selection);
        }
        Self { editor, platform: Fake::default() }
    }

    /// The same, laid out by a fake layout that wraps after `width` characters.
    pub fn wrapped(state: &str, width: usize) -> Self {
        let mut harness = Self::new(state);
        harness.platform.wrap = Some(Wrapped { width, doc: harness.editor.doc().clone() });
        harness
    }

    /// The document and selection in the notation.
    pub fn state(&self) -> String {
        render(self.editor.doc(), self.editor.selection())
    }

    /// The document alone.
    pub fn text(&self) -> String {
        render(self.editor.doc(), None)
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
    let (doc, selection) = parse(state);
    render(&doc, selection.as_ref())
}

/// Reads the notation. Blocks are named `b1`, `b2` and so on in document order, containers included.
pub fn parse(state: &str) -> (Document, Option<Selection>) {
    let mut blocks: Vec<Block> = Vec::new();
    let (mut anchor, mut head) = (None, None);
    let mut count = 0;
    let mut id = || {
        count += 1;
        format!("b{count}")
    };
    for line in state.lines() {
        let (container, line) = match line.split_once("> ") {
            Some(("", rest)) => (Some(BlockKind::Quote), rest),
            Some((tone, rest)) if TONES.iter().any(|(name, _)| *name == tone) => {
                let tone = TONES.iter().find(|(name, _)| *name == tone).expect("checked above").1;
                (Some(BlockKind::Callout { tone }), rest)
            }
            _ => (None, line),
        };
        let depth = (line.len() - line.trim_start_matches(' ').len()) / 2;
        let (kind, text) = line.trim_start_matches(' ').split_once(' ').unwrap_or((line.trim_start_matches(' '), ""));
        let list = |list| BlockKind::ListItem { list, depth: depth as u8 };
        let kind = match kind {
            "p" => BlockKind::Paragraph,
            "h1" | "h2" | "h3" | "h4" => BlockKind::Heading { level: kind[1..].parse().expect("a heading level") },
            "ul" => list(ListKind::Bullet),
            "ol" => list(ListKind::Ordered),
            "todo" => list(ListKind::Task { checked: false }),
            "done" => list(ListKind::Task { checked: true }),
            "hr" => BlockKind::Divider,
            "code" => BlockKind::Code { language: None },
            other => match other.strip_prefix("code:") {
                Some(language) => BlockKind::Code { language: Some(language.to_owned()) },
                None => panic!("unknown block kind {other:?} in {state:?}"),
            },
        };

        let mut block = Block::new(id().as_str(), kind);
        if matches!(block.body, Body::Text(_)) {
            let (runs, at_anchor, at_head) = inline(text);
            block.body = Body::Text(runs);
            anchor = at_anchor.map(|offset| Pos::new(block.id.clone(), offset)).or(anchor);
            head = at_head.map(|offset| Pos::new(block.id.clone(), offset)).or(head);
        }
        match (container, blocks.last_mut()) {
            (None, _) => blocks.push(block),
            // Consecutive lines with the same prefix share one quote or callout.
            (Some(kind), Some(Block { kind: open, body: Body::Blocks(children), .. })) if *open == kind => {
                children.push(block)
            }
            (Some(kind), _) => {
                let mut outer = Block::new(id().as_str(), kind);
                outer.body = Body::Blocks(vec![block]);
                blocks.push(outer);
            }
        }
    }
    let selection = head.map(|head| Selection { anchor: anchor.unwrap_or_else(|| head.clone()), head });
    (Document { blocks, ..Document::default() }, selection)
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
pub fn render(doc: &Document, selection: Option<&Selection>) -> String {
    let mut lines = Vec::new();
    for block in &doc.blocks {
        match (&block.kind, &block.body) {
            (BlockKind::Quote, Body::Blocks(children)) => {
                lines.extend(children.iter().map(|child| format!("> {}", line(child, selection))));
            }
            (BlockKind::Callout { tone }, Body::Blocks(children)) => {
                let name = TONES.iter().find(|(_, other)| other == tone).expect("every tone has a name").0;
                lines.extend(children.iter().map(|child| format!("{name}> {}", line(child, selection))));
            }
            _ => lines.push(line(block, selection)),
        }
    }
    lines.join("\n")
}

fn line(block: &Block, selection: Option<&Selection>) -> String {
    let (indent, kind) = match &block.kind {
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
