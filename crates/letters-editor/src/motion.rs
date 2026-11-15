//! Moving the caret and the selection.

use std::collections::BTreeSet;

use letters_model::{Block, BlockId, Document, Mark, Pos, Selection};

use crate::markers::{PAIRED, Sides};
use crate::{Caret, Editor, Motion, Platform, parts, refs, text, tree};

/// A position on a laid-out line. Where a line wraps with no space to stand before, one offset is the end of
/// that line and the start of the next. A caret there is at the start of the next, unless `before_wrap` says
/// it is at the end of the first. Anywhere else `before_wrap` means nothing, and layout answers it unset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinePos {
    pub pos: Pos,
    pub before_wrap: bool,
}

impl From<Pos> for LinePos {
    fn from(pos: Pos) -> Self {
        Self { pos, before_wrap: false }
    }
}

/// What movement between lines asks of layout. `letters_layout::DocLayout` can answer all of it.
pub trait Lines {
    /// The horizontal position of a caret at `at`.
    fn caret_x(&self, at: &LinePos) -> Option<f32>;
    /// The position one visual line above or below `at`, nearest to the horizontal position `x`.
    /// `None` at the first and last line of the document.
    fn line_step(&self, at: &LinePos, x: f32, down: bool) -> Option<LinePos>;
    /// The start or the end of the visual line that holds `at`.
    fn line_edge(&self, at: &LinePos, end: bool) -> Option<LinePos>;
    /// How many lines Page Up and Page Down move by.
    fn page_lines(&self) -> usize {
        20
    }
}

/// Lines without a layout: each runs to the next line break, and `x` counts characters.
struct HardLines<'a>(&'a Document);

impl Lines for HardLines<'_> {
    fn caret_x(&self, LinePos { pos, .. }: &LinePos) -> Option<f32> {
        let text = self.0.block(&pos.block)?.plain_text();
        let offset = text::snap(&text, pos.offset);
        Some(text::column(&text, offset) as f32)
    }

    fn line_step(&self, LinePos { pos, .. }: &LinePos, x: f32, down: bool) -> Option<LinePos> {
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
        Some(Pos::new(texts[index].id.clone(), text::at_column(&text, line, x as usize)).into())
    }

    fn line_edge(&self, LinePos { pos, .. }: &LinePos, end: bool) -> Option<LinePos> {
        let text = self.0.block(&pos.block)?.plain_text();
        let offset = text::snap(&text, pos.offset);
        let edge = if end { text::line_end(&text, offset) } else { text::line_start(&text, offset) };
        Some(Pos::new(pos.block.clone(), edge).into())
    }
}

impl Editor {
    /// Where `motion` leads from `from`. `goal` is the horizontal position that vertical moves keep to.
    pub(crate) fn target(&self, from: &Pos, motion: Motion, goal: &mut Option<f32>, platform: &dyn Platform) -> Pos {
        self.aim(&self.stand(from), motion, goal, platform).pos
    }

    /// A position with the line it is on: the caret's own head is on the line the caret says.
    fn stand(&self, pos: &Pos) -> LinePos {
        let caret = self.caret.as_ref().filter(|caret| caret.selection.head == *pos);
        LinePos { pos: pos.clone(), before_wrap: caret.is_some_and(|caret| caret.before_wrap) }
    }

    /// A position that layout answered, on a character the caret can stand at. One that had to move for
    /// that is no longer where layout said a line wraps.
    fn near(&self, at: LinePos) -> Option<LinePos> {
        let pos = self.clamp(&at.pos)?;
        Some(LinePos { before_wrap: at.before_wrap && pos == at.pos, pos })
    }

    /// Says which line the caret is on, after it was put where `at` is.
    pub(crate) fn side(&mut self, at: &LinePos) {
        if let Some(caret) = &mut self.caret {
            caret.before_wrap = at.before_wrap && !caret.picture && caret.selection.head == at.pos;
        }
    }

    /// As [`Editor::target`], between positions that say which line they are on.
    fn aim(&self, at: &LinePos, motion: Motion, goal: &mut Option<f32>, platform: &dyn Platform) -> LinePos {
        let from = &at.pos;
        let doc = self.doc();
        let texts = tree::texts(doc);
        let Some(index) = texts.iter().position(|block| block.id == from.block) else { return at.clone() };
        // A picture in a gallery shows no text to move through.
        let text = if self.member(&from.block) { String::new() } else { texts[index].plain_text() };
        let offset = text::snap(&text, from.offset);
        let here = |offset: usize| Pos::new(from.block.clone(), offset);
        // The place to move from, which is still at a wrap only if it was on a character.
        let on = LinePos { pos: here(offset), before_wrap: at.before_wrap && offset == from.offset };
        let start_of = |index: usize| Pos::new(texts[index].id.clone(), 0);
        let end_of = |index: usize| Pos::new(texts[index].id.clone(), self.span(texts[index]));
        let (first, last) = (0, texts.len() - 1);

        let fallback = HardLines(doc);
        let laid = platform.lines();
        let lines: &dyn Lines = laid.unwrap_or(&fallback);
        let edge = |at: &LinePos, end: bool| {
            let laid = lines.line_edge(at, end).and_then(|to| self.near(to));
            laid.or_else(|| fallback.line_edge(at, end)).unwrap_or_else(|| at.clone())
        };

        let to = match motion {
            Motion::LineStart => return edge(&on, false),
            Motion::LineEnd => return edge(&on, true),
            Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown => {
                let down = matches!(motion, Motion::Down | Motion::PageDown);
                let paged = matches!(motion, Motion::PageUp | Motion::PageDown);
                let mut at = on;
                let x = goal.or_else(|| lines.caret_x(&at)).unwrap_or(0.0);
                *goal = Some(x);
                for _ in 0..if paged { lines.page_lines().max(1) } else { 1 } {
                    match lines.line_step(&at, x, down).and_then(|to| self.near(to)) {
                        Some(next) => at = next,
                        // Past the first or last line, the caret goes to the end of the line it is on.
                        None => return edge(&at, down),
                    }
                }
                return at;
            }
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
            Motion::BlockStart if offset == 0 && index > first => start_of(index - 1),
            Motion::BlockStart => here(0),
            Motion::BlockEnd if offset == text.len() && index < last => end_of(index + 1),
            Motion::BlockEnd => here(text.len()),
            Motion::DocStart => start_of(first),
            Motion::DocEnd => end_of(last),
        };
        // A reference is passed in one step, whatever the step was to.
        let back = matches!(motion, Motion::Left | Motion::WordLeft);
        let forth = matches!(motion, Motion::Right | Motion::WordRight);
        let inside = self.block(&to.block).and_then(|block| refs::around(block, to.offset));
        match inside.filter(|_| back || forth) {
            Some(range) => Pos::new(to.block, if back { range.start } else { range.end }).into(),
            None => to.into(),
        }
    }

    /// Where a caret goes one line up or down from a place in text or, with the flag set, from a picture.
    /// `None` when the document has no line there.
    ///
    /// A figure is one stop above its caption, the pictures of a gallery stand side by side above theirs,
    /// and a cell is left for the cell above or below it. None of that depends on how the lines are laid out.
    fn line(&self, (at, picture): &(LinePos, bool), x: f32, down: bool, lines: &dyn Lines) -> Option<(LinePos, bool)> {
        let doc = self.doc();
        let pos = &at.pos;
        let block = self.block(&pos.block)?;
        let parent = self.seat(&pos.block)?.parent;
        let step = || lines.line_step(at, x, down).and_then(|to| self.near(to));
        let leaves = || step().is_none_or(|to| to.pos.block != pos.block);
        // A place among the parts of a block is where no line wraps.
        let part = |(pos, picture): (Pos, bool)| (LinePos::from(pos), picture);
        // The text before or after everything in a gallery or a table.
        let beyond = |unit: &Block| {
            let texts = tree::texts(doc);
            let edge = if down { tree::last_text(unit) } else { tree::first_text(unit) }?;
            let at = texts.iter().position(|block| block.id == edge.id)?;
            let next = if down { texts.get(at + 1) } else { at.checked_sub(1).and_then(|at| texts.get(at)) }?;
            Some(part(self.enter(Pos::new(next.id.clone(), if down { 0 } else { self.span(next) }), down)))
        };
        let figure = tree::is_figure(block);
        let on = |figure: &Block| Some(part((Pos::new(figure.id.clone(), 0), true)));
        if let Some(gallery) = parent.filter(|parent| tree::is_gallery(parent)) {
            if figure {
                let caption = parts::caption(gallery).filter(|_| down);
                let caption = caption.map(|caption| part((Pos::new(caption.id.clone(), 0), false)));
                return caption.or_else(|| beyond(gallery));
            }
            let last = gallery.body.blocks().iter().rev().find(|part| tree::is_figure(part));
            if let Some(last) = last.filter(|_| !down && leaves()) {
                return on(last);
            }
        } else if figure && *picture && down {
            return Some(part((pos.clone(), false)));
        } else if figure && !*picture && !down && leaves() {
            return on(block);
        } else if let Some((table, row, column)) = self.grid(&pos.block).filter(|_| leaves()) {
            let Some(next) = parts::row_beside(table, row, down) else { return beyond(table) };
            let cells = parts::cells(next);
            let cell = cells.get(column).or(cells.last())?;
            return Some(part((Pos::new(cell.id.clone(), text::snap(&cell.plain_text(), pos.offset)), false)));
        }
        let to = step()?;
        // No figure is stepped over, wherever the layout has its lines.
        let texts = tree::texts(doc);
        let index = |id: &BlockId| texts.iter().position(|block| block.id == *id);
        let (from, at) = (index(&pos.block)?, index(&to.pos.block)?);
        let over = |i: &usize| tree::is_figure(texts[*i]);
        let between = if down { (from + 1..at).find(over) } else { (at + 1..from).rev().find(over) };
        if let Some(i) = between {
            let edge = Pos::new(texts[i].id.clone(), if down { 0 } else { self.span(texts[i]) });
            return Some(part(self.enter(edge, down)));
        }
        let (pos, picture) = self.enter(to.pos.clone(), down);
        Some((LinePos { before_wrap: to.before_wrap && !picture && pos == to.pos, pos }, picture))
    }

    /// Where a caret that comes to `pos` from the line above or below stands. A figure is come to by its
    /// picture from above, and from below by its caption when it has one.
    fn enter(&self, pos: Pos, down: bool) -> (Pos, bool) {
        match self.block(&pos.block).filter(|block| tree::is_figure(block)) {
            Some(figure) if down || self.span(figure) == 0 => (Pos::new(pos.block, 0), true),
            _ => (pos, false),
        }
    }

    /// Moves a caret up or down by a line or a page.
    fn rise(&mut self, caret: Caret, to: Motion, goal: &mut Option<f32>, platform: &mut dyn Platform) {
        let down = matches!(to, Motion::Down | Motion::PageDown);
        let paged = matches!(to, Motion::PageUp | Motion::PageDown);
        let head = LinePos { pos: caret.selection.head.clone(), before_wrap: caret.before_wrap };
        let (at, moved, ended) = {
            let fallback = HardLines(self.doc());
            let lines = platform.lines().unwrap_or(&fallback);
            let x = goal.or_else(|| lines.caret_x(&head)).unwrap_or(0.0);
            *goal = Some(x);
            let (mut at, mut moved, mut ended) = ((head, caret.picture), false, false);
            for _ in 0..if paged { lines.page_lines().max(1) } else { 1 } {
                match self.line(&at, x, down, lines) {
                    Some(next) => (at, moved) = (next, true),
                    None => {
                        ended = true;
                        break;
                    }
                }
            }
            (at, moved, ended)
        };
        // An arrow that has no line to go to makes one, where there is something to leave.
        if !moved && !paged && self.open(down, platform) {
            return;
        }
        let (at, picture) = at;
        if ended && !picture {
            // Past the first or last line, the caret goes to the end of the line it is on.
            let edge = if down { Motion::LineEnd } else { Motion::LineStart };
            let to = self.aim(&at, edge, &mut None, platform);
            return self.stop(to);
        }
        self.land((at.pos.clone(), picture));
        self.side(&at);
    }

    /// Puts the caret at a place on a line.
    fn stop(&mut self, at: LinePos) {
        self.place(at.pos.clone());
        self.side(&at);
    }

    pub(crate) fn move_to(&mut self, to: Motion, extend: bool, goal: Option<f32>, platform: &mut dyn Platform) {
        let Some(caret) = self.caret.clone() else { return };
        let selection = caret.selection.clone();
        let vertical = matches!(to, Motion::Up | Motion::Down | Motion::PageUp | Motion::PageDown);
        let mut goal = goal.filter(|_| vertical);
        let head = LinePos { pos: selection.head.clone(), before_wrap: caret.before_wrap };
        if extend {
            let head = self.aim(&head, to, &mut goal, platform);
            self.set_selection(Selection { anchor: selection.anchor, head: head.pos.clone() });
            self.side(&head);
        } else if !selection.is_caret() {
            // A plain move collapses a selection towards the way it points. A figure that the selection
            // took whole at that end stays selected.
            let (start, end) = self.ordered(&selection);
            let taken = self.taken(&start, &end);
            let at = match to {
                Motion::Left => self.stand(&start),
                Motion::Right => self.stand(&end),
                Motion::Up | Motion::PageUp => self.aim(&self.stand(&start), to, &mut goal, platform),
                Motion::Down | Motion::PageDown => self.aim(&self.stand(&end), to, &mut goal, platform),
                _ => self.aim(&head, to, &mut goal, platform),
            };
            if matches!(to, Motion::Left | Motion::Right) && taken.contains(&at.pos.block) {
                self.place_picture(&at.pos.block);
            } else {
                self.stop(at);
            }
        } else if vertical {
            self.rise(caret, to, &mut goal, platform);
        } else if matches!(to, Motion::Left | Motion::Right) {
            self.step(caret, to == Motion::Right, platform);
        } else {
            let at = self.aim(&head, to, &mut goal, platform);
            if at.pos.block != selection.head.block || to == Motion::DocStart {
                self.land(self.arrive(at.pos));
            } else if at.pos != selection.head || !caret.picture {
                self.stop(at);
            }
        }
        self.goal = goal;
    }

    /// Moves a caret one stop sideways. A revealed marker is a stop of its own, so crossing one changes the
    /// marks typed text will take and leaves the caret where it is. A picture is a stop before its caption.
    fn step(&mut self, caret: Caret, right: bool, platform: &mut dyn Platform) {
        let pos = caret.selection.head.clone();
        let Some(block) = self.block(&pos.block) else { return };
        let to = if right { Motion::Right } else { Motion::Left };
        if caret.picture {
            // Right of a picture is its caption, when it has one. Otherwise the caret leaves the figure.
            let span = self.span(block);
            if right && span > 0 {
                return self.place(pos);
            }
            let from = Pos::new(pos.block, if right { span } else { 0 });
            match self.target(&from, to, &mut None, platform) {
                target if target == from => _ = self.open(right, platform),
                target => self.land(self.arrive(target)),
            }
            return;
        }
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
        if !right && pos.offset == 0 && tree::is_figure(block) {
            return self.place_picture(&pos.block);
        }

        let target = self.target(&pos, to, &mut None, platform);
        if target == pos {
            // The document ends here, so the caret leaves what holds it in by a line made for it.
            self.open(right, platform);
            return;
        }
        if target.block != pos.block
            && let (on, true) = self.arrive(target.clone())
        {
            return self.place_picture(&on.block);
        }
        // The caret lands on the near side of any marker at its new place: it keeps the paired marks of
        // the character it crossed, or none when it came from another block.
        let Some(block) = self.block(&target.block).filter(|block| !text::is_source(block)) else {
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
            Selection { anchor: Pos::new(first.id.clone(), 0), head: Pos::new(last.id.clone(), self.span(last)) };
        self.set_selection(selection);
    }

    pub(crate) fn select_word(&mut self) {
        let Some(caret) = &self.caret else { return };
        let (head, before_wrap) = (caret.selection.head.clone(), caret.before_wrap);
        let Some(block) = self.block(&head.block) else { return };
        let text = block.plain_text();
        // At the end of a line that wraps with no space, the word is the one that line ends with, and not
        // the one the next line starts with.
        let inside = if before_wrap { text::prev(&text, head.offset) } else { head.offset };
        let word = text::word_around(&text, inside);
        let at = |offset: usize| Pos::new(head.block.clone(), offset);
        self.set_selection(Selection { anchor: at(word.start), head: at(word.end) });
        // A word that ends with the line ends on it.
        self.side(&LinePos { pos: at(word.end), before_wrap: before_wrap && word.end == head.offset });
    }

    pub(crate) fn select_block(&mut self) {
        let Some(head) = self.selection().map(|selection| selection.head.clone()) else { return };
        let Some(len) = self.block(&head.block).map(|block| self.span(block)) else { return };
        let at = |offset: usize| Pos::new(head.block.clone(), offset);
        self.set_selection(Selection { anchor: at(0), head: at(len) });
    }
}
