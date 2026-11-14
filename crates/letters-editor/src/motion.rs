//! Moving the caret and the selection.

use std::collections::BTreeSet;

use letters_model::{Document, Mark, Pos, Selection};

use crate::markers::{PAIRED, Sides};
use crate::{Caret, Editor, Motion, Platform, text, tree};

/// What movement between lines asks of layout. `letters_layout::DocLayout` can answer all of it.
pub trait Lines {
    /// The horizontal position of a caret at `pos`.
    fn caret_x(&self, pos: &Pos) -> Option<f32>;
    /// The position one visual line above or below `pos`, nearest to the horizontal position `x`.
    /// `None` at the first and last line of the document.
    fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos>;
    /// The start or the end of the visual line that holds `pos`.
    fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos>;
    /// How many lines Page Up and Page Down move by.
    fn page_lines(&self) -> usize {
        20
    }
}

/// Lines without a layout: each runs to the next line break, and `x` counts characters.
struct HardLines<'a>(&'a Document);

impl Lines for HardLines<'_> {
    fn caret_x(&self, pos: &Pos) -> Option<f32> {
        let text = self.0.block(&pos.block)?.plain_text();
        let offset = text::snap(&text, pos.offset);
        Some(text::column(&text, offset) as f32)
    }

    fn line_step(&self, pos: &Pos, x: f32, down: bool) -> Option<Pos> {
        let texts = tree::texts(self.0);
        let index = texts.iter().position(|block| block.id == pos.block)?;
        let text = texts[index].plain_text();
        let offset = text::snap(&text, pos.offset);
        let (index, text, line) = if down {
            match text::line_end(&text, offset) {
                end if end < text.len() => (index, text, end + 1),
                _ => (index + 1, texts.get(index + 1)?.plain_text(), 0),
            }
        } else {
            match text::line_start(&text, offset) {
                0 => {
                    let above = texts.get(index.checked_sub(1)?)?.plain_text();
                    let line = text::line_start(&above, above.len());
                    (index - 1, above, line)
                }
                start => {
                    let line = text::line_start(&text, start - 1);
                    (index, text, line)
                }
            }
        };
        Some(Pos::new(texts[index].id.clone(), text::at_column(&text, line, x as usize)))
    }

    fn line_edge(&self, pos: &Pos, end: bool) -> Option<Pos> {
        let text = self.0.block(&pos.block)?.plain_text();
        let offset = text::snap(&text, pos.offset);
        let edge = if end { text::line_end(&text, offset) } else { text::line_start(&text, offset) };
        Some(Pos::new(pos.block.clone(), edge))
    }
}

impl Editor {
    /// Where `motion` leads from `from`. `goal` is the horizontal position that vertical moves keep to.
    pub(crate) fn target(&self, from: &Pos, motion: Motion, goal: &mut Option<f32>, platform: &dyn Platform) -> Pos {
        let doc = self.doc();
        let texts = tree::texts(doc);
        let Some(index) = texts.iter().position(|block| block.id == from.block) else { return from.clone() };
        let text = texts[index].plain_text();
        let offset = text::snap(&text, from.offset);
        let here = |offset: usize| Pos::new(from.block.clone(), offset);
        let start_of = |index: usize| Pos::new(texts[index].id.clone(), 0);
        let end_of = |index: usize| Pos::new(texts[index].id.clone(), text::len(texts[index]));
        let (first, last) = (0, texts.len() - 1);

        let fallback = HardLines(doc);
        let laid = platform.lines();
        let lines: &dyn Lines = laid.unwrap_or(&fallback);
        let edge = |pos: &Pos, end: bool| {
            let laid = lines.line_edge(pos, end).and_then(|pos| self.clamp(&pos));
            laid.or_else(|| fallback.line_edge(pos, end)).unwrap_or_else(|| pos.clone())
        };

        match motion {
            Motion::Left | Motion::WordLeft if offset == 0 => {
                if index > first {
                    end_of(index - 1)
                } else {
                    here(0)
                }
            }
            Motion::Right | Motion::WordRight if offset == text.len() => {
                if index < last {
                    start_of(index + 1)
                } else {
                    here(offset)
                }
            }
            Motion::Left => here(text::prev(&text, offset)),
            Motion::Right => here(text::next(&text, offset)),
            Motion::WordLeft => here(text::word_left(&text, offset)),
            Motion::WordRight => here(text::word_right(&text, offset)),
            Motion::LineStart => edge(&here(offset), false),
            Motion::LineEnd => edge(&here(offset), true),
            Motion::BlockStart if offset == 0 && index > first => start_of(index - 1),
            Motion::BlockStart => here(0),
            Motion::BlockEnd if offset == text.len() && index < last => end_of(index + 1),
            Motion::BlockEnd => here(text.len()),
            Motion::DocStart => start_of(first),
            Motion::DocEnd => end_of(last),
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => {
                let down = matches!(motion, Motion::Down | Motion::PageDown);
                let paged = matches!(motion, Motion::PageUp | Motion::PageDown);
                let mut pos = here(offset);
                let x = goal.or_else(|| lines.caret_x(&pos)).unwrap_or(0.0);
                *goal = Some(x);
                for _ in 0..if paged { lines.page_lines().max(1) } else { 1 } {
                    match lines.line_step(&pos, x, down).and_then(|pos| self.clamp(&pos)) {
                        Some(next) => pos = next,
                        // Past the first or last line, the caret goes to the end of the line it is on.
                        None => return edge(&pos, down),
                    }
                }
                pos
            }
        }
    }

    pub(crate) fn move_to(&mut self, to: Motion, extend: bool, goal: Option<f32>, platform: &dyn Platform) {
        let Some(caret) = self.caret.clone() else { return };
        let selection = caret.selection.clone();
        let vertical = matches!(to, Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown);
        let mut goal = goal.filter(|_| vertical);
        if extend {
            let head = self.target(&selection.head, to, &mut goal, platform);
            self.set_selection(Selection { anchor: selection.anchor, head });
        } else if !selection.is_caret() {
            // A plain move collapses a selection towards the way it points.
            let (start, end) = self.ordered(&selection);
            let pos = match to {
                Motion::Left => start,
                Motion::Right => end,
                Motion::Up | Motion::PageUp => self.target(&start, to, &mut goal, platform),
                Motion::Down | Motion::PageDown => self.target(&end, to, &mut goal, platform),
                _ => self.target(&selection.head, to, &mut goal, platform),
            };
            self.place(pos);
        } else if matches!(to, Motion::Left | Motion::Right) {
            self.step(caret, to == Motion::Right, platform);
        } else {
            let pos = self.target(&selection.head, to, &mut goal, platform);
            self.place(pos);
        }
        self.goal = goal;
    }

    /// Moves a caret one stop sideways. A revealed marker is a stop of its own, so crossing one changes the
    /// marks typed text will take and leaves the caret where it is.
    fn step(&mut self, caret: Caret, right: bool, platform: &dyn Platform) {
        let pos = caret.selection.head.clone();
        let Some(block) = self.block(&pos.block) else { return };
        let sides = Sides::at(block, pos.offset);
        let stops = if right { sides.after(&caret.marks) } else { sides.before(&caret.marks) };
        if let Some((mark, opens)) = if right { stops.first() } else { stops.last() } {
            // Going right an opening marker leads into its span, and a closing one out of it.
            let mut marks = caret.marks.clone();
            if *opens == right {
                marks.insert(mark.clone());
            } else {
                marks.remove(mark);
            }
            return self.place_with(pos, marks);
        }

        let to = if right { Motion::Right } else { Motion::Left };
        let target = self.target(&pos, to, &mut None, platform);
        if target == pos {
            return;
        }
        // The caret lands on the near side of any marker at its new place: it keeps the paired marks of
        // the character it crossed, or none when it came from another block.
        let Some(block) = self.block(&target.block).filter(|block| !text::is_code(block)) else {
            return self.place_with(target, BTreeSet::new());
        };
        let paired = |mark: &&Mark| PAIRED.contains(mark);
        let mut marks: BTreeSet<Mark> = self.natural(&target).iter().filter(|mark| !paired(mark)).cloned().collect();
        if target.block == pos.block {
            let runs = block.body.runs();
            let crossed =
                if right { text::marks_before(runs, target.offset) } else { text::marks_after(runs, target.offset) };
            marks.extend(crossed.iter().filter(paired).cloned());
        }
        self.place_with(target, marks);
    }

    pub(crate) fn select_all(&mut self) {
        let texts = tree::texts(self.doc());
        let (Some(first), Some(last)) = (texts.first(), texts.last()) else { return };
        let selection =
            Selection { anchor: Pos::new(first.id.clone(), 0), head: Pos::new(last.id.clone(), text::len(last)) };
        self.set_selection(selection);
    }

    pub(crate) fn select_word(&mut self) {
        let Some(head) = self.selection().map(|selection| selection.head.clone()) else { return };
        let Some(block) = self.block(&head.block) else { return };
        let word = text::word_around(&block.plain_text(), head.offset);
        let at = |offset: usize| Pos::new(head.block.clone(), offset);
        self.set_selection(Selection { anchor: at(word.start), head: at(word.end) });
    }

    pub(crate) fn select_block(&mut self) {
        let Some(head) = self.selection().map(|selection| selection.head.clone()) else { return };
        let Some(len) = self.block(&head.block).map(text::len) else { return };
        let at = |offset: usize| Pos::new(head.block.clone(), offset);
        self.set_selection(Selection { anchor: at(0), head: at(len) });
    }
}
