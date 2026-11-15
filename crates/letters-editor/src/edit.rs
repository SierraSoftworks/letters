//! Edits to text and to the blocks that hold it: typing, deleting, splitting, joining and formatting.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use letters_model::{Block, BlockId, BlockKind, Body, BodyShape, ListKind, Mark, Op, Pos, Run, Selection};

use crate::markers::Sides;
use crate::{Caret, Editor, Kind, Motion, Platform, rules, text, tree};

/// Lists nest no deeper than this.
const MAX_DEPTH: u8 = 8;
/// What Tab steps a line of code in by.
const CODE_INDENT: &str = "    ";

impl Editor {
    /// Replaces the selection with `text` in the caret's marks, and leaves the caret after it.
    pub(crate) fn insert(&mut self, text: &str) {
        let Some(caret) = self.caret.clone() else { return };
        let (start, end) = self.ordered(&caret.selection);
        let (start, end) = if start.block == end.block {
            (start, end.offset)
        } else {
            // A selection across blocks is deleted first, which leaves a caret to type at.
            let at = self.remove(&start, &end);
            let offset = at.offset;
            (at, offset)
        };
        let Some(block) = self.block(&start.block) else { return };
        let marks = if text::is_source(block) { BTreeSet::new() } else { caret.marks.clone() };
        // Typed text joins a link only from inside it, so a link does not grow from its edges.
        let runs = block.body.runs();
        let before = start.offset.checked_sub(1).and_then(|at| text::run_at(runs, at)).and_then(|run| run.link.clone());
        let after = text::run_at(runs, end).and_then(|run| run.link.clone());
        let link = before.filter(|link| Some(link) == after.as_ref());
        self.replace(&start.block, start.offset..end, text, &marks, &link);
        self.place_with(Pos::new(start.block, start.offset + text.len()), caret.marks);
    }

    /// Replaces a range of one block with `text` that carries exactly `marks` and `link`.
    pub(crate) fn replace(
        &mut self,
        block: &BlockId,
        range: Range<usize>,
        text: &str,
        marks: &BTreeSet<Mark>,
        link: &Option<String>,
    ) {
        let mut ops = Vec::new();
        if !range.is_empty() {
            ops.push(Op::DeleteText { block: block.clone(), range: range.clone() });
        }
        if !text.is_empty() {
            ops.push(Op::InsertText { at: Pos::new(block.clone(), range.start), text: text.to_owned() });
        }
        self.apply(ops);
        // Inserted text takes after its neighbour. Whatever that gave it and should not have is put right.
        let fix = self.format(block, range.start..range.start + text.len(), marks, link);
        self.apply(fix);
    }

    /// The edits that leave `range` with exactly `marks` and `link`.
    pub(crate) fn format(
        &self,
        id: &BlockId,
        range: Range<usize>,
        marks: &BTreeSet<Mark>,
        link: &Option<String>,
    ) -> Vec<Op> {
        let Some(block) = self.block(id) else { return Vec::new() };
        let runs: Vec<&Run> = text::runs_in(block.body.runs(), range.clone()).collect();
        let present: BTreeSet<&Mark> = runs.iter().flat_map(|run| &run.marks).chain(marks).collect();
        let mut ops = Vec::new();
        for mark in present {
            let on = marks.contains(mark);
            if runs.iter().any(|run| run.marks.contains(mark) != on) {
                ops.push(Op::SetMark { block: id.clone(), range: range.clone(), mark: mark.clone(), on });
            }
        }
        if runs.iter().any(|run| &run.link != link) {
            ops.push(Op::SetLink { block: id.clone(), range, link: link.clone() });
        }
        ops
    }

    /// Deletes the selection if there is one, leaving a caret that keeps its marks.
    pub(crate) fn delete_selection(&mut self) -> bool {
        let Some(caret) = self.caret.clone().filter(|caret| !caret.selection.is_caret()) else { return false };
        let (start, end) = self.ordered(&caret.selection);
        let pos = self.remove(&start, &end);
        self.place_with(pos, caret.marks);
        true
    }

    /// Deletes everything from `start` to `end`, which is later in the document. Returns where that leaves
    /// a caret.
    pub(crate) fn remove(&mut self, start: &Pos, end: &Pos) -> Pos {
        if start.block == end.block {
            if start.offset < end.offset {
                self.apply(vec![Op::DeleteText { block: start.block.clone(), range: start.offset..end.offset }]);
            }
            return start.clone();
        }
        let walk: Vec<&Block> = self.doc().walk().collect();
        let index = |id: &BlockId| walk.iter().position(|block| &block.id == id);
        let (Some(from), Some(to)) = (index(&start.block), index(&end.block)) else { return start.clone() };
        let mut ops = Vec::new();
        let len = text::len(walk[from]);
        if start.offset < len {
            ops.push(Op::DeleteText { block: start.block.clone(), range: start.offset..len });
        }
        // Blocks wholly inside the range go. A container that also holds the last block stays.
        let mut i = from + 1;
        while i < to {
            if tree::contains(walk[i], &end.block) {
                i += 1;
            } else {
                ops.push(Op::RemoveBlock { block: walk[i].id.clone() });
                i += tree::size(walk[i]);
            }
        }
        if end.offset > 0 {
            ops.push(Op::DeleteText { block: end.block.clone(), range: 0..end.offset });
        }
        self.apply(ops);
        self.join(&start.block, &end.block)
    }

    /// Joins the text block `second` onto `first`, which comes before it. Returns where the two meet.
    pub(crate) fn join(&mut self, first: &BlockId, second: &BlockId) -> Pos {
        let doc = self.doc();
        let (Some(a), Some(b)) = (doc.block(first), doc.block(second)) else { return Pos::new(first.clone(), 0) };
        let seam = text::len(a);
        if seam == 0 && text::len(b) > 0 {
            // An empty block gives way, so the text keeps the kind of its own block.
            self.apply(vec![Op::RemoveBlock { block: tree::removal(doc, first) }]);
            return Pos::new(second.clone(), 0);
        }
        let (Some(at), Some(from)) = (tree::place(doc, first), tree::place(doc, second)) else {
            return Pos::new(first.clone(), seam);
        };
        let mut ops = Vec::new();
        if at.parent_id() != from.parent_id() || from.index != at.index + 1 {
            // The model joins a block onto the sibling before it, so the block moves there first.
            ops.push(Op::RemoveBlock { block: tree::removal(doc, second) });
            ops.push(Op::InsertBlock { parent: at.parent_id(), index: at.index + 1, block: b.clone() });
        }
        ops.push(Op::Join { block: second.clone() });
        self.apply(ops);
        Pos::new(first.clone(), seam)
    }

    /// Deletes the selection, or from the caret to where `motion` leads.
    pub(crate) fn delete(&mut self, motion: Motion, platform: &dyn Platform) {
        if self.delete_selection() {
            return;
        }
        let Some(caret) = self.caret.clone() else { return };
        let pos = caret.selection.head.clone();
        let Some(block) = self.block(&pos.block) else { return };
        if let Motion::Left | Motion::Right = motion {
            // A revealed marker beside the caret is deleted before any text is.
            let right = motion == Motion::Right;
            let sides = Sides::at(block, pos.offset);
            let stops = if right { sides.after(&caret.marks) } else { sides.before(&caret.marks) };
            if let Some((mark, _)) = if right { stops.first() } else { stops.last() } {
                return self.delete_marker(caret, mark, !right);
            }
            if !right && pos.offset == 0 {
                return self.delete_at_start(caret);
            }
            if right && pos.offset == text::len(block) {
                return self.delete_at_end(caret);
            }
        }
        let target = self.target(&pos, motion, &mut None, platform);
        let (start, end) = self.ordered(&Selection { anchor: pos, head: target });
        let pos = self.remove(&start, &end);
        self.place_with(pos, caret.marks);
    }

    /// Deleting either marker of a pair takes the mark off its span. `before` says which side of the caret
    /// the marker is on.
    fn delete_marker(&mut self, mut caret: Caret, mark: &Mark, before: bool) {
        let pos = caret.selection.head.clone();
        let Some(block) = self.block(&pos.block) else { return };
        let (runs, sides) = (block.body.runs(), Sides::at(block, pos.offset));
        let has = |run: &Run| run.marks.contains(mark);
        // The marker bounds the span on the left when the character there has the mark and the marker
        // closes it, and the span on the right otherwise.
        let (left, right) = (sides.left.contains(mark), sides.right.contains(mark));
        let range = match (before, left, right) {
            (true, true, _) | (false, true, false) => text::stretch_start(runs, pos.offset, has)..pos.offset,
            (true, false, true) | (false, _, true) => pos.offset..text::stretch_end(runs, pos.offset, has),
            // The pair holds no text yet, so only the caret carries the mark.
            _ => pos.offset..pos.offset,
        };
        if !range.is_empty() {
            self.apply(vec![Op::SetMark { block: pos.block.clone(), range, mark: mark.clone(), on: false }]);
        }
        caret.marks.remove(mark);
        self.caret = Some(caret);
    }

    /// Backspace at the start of a block takes away what makes the block more than a paragraph, one step
    /// at a time, and then joins it to the block before.
    fn delete_at_start(&mut self, caret: Caret) {
        let id = caret.selection.head.block.clone();
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(&id), tree::place(doc, &id)) else { return };
        let plainer = match &block.kind {
            BlockKind::Paragraph => None,
            BlockKind::Heading { level } if *level > 1 => Some(BlockKind::Heading { level: level - 1 }),
            BlockKind::ListItem { list, depth } if *depth > 0 => {
                Some(BlockKind::ListItem { list: *list, depth: depth - 1 })
            }
            _ => Some(BlockKind::Paragraph),
        };
        if let Some(kind) = plainer {
            self.apply(vec![Op::SetKind { block: id, kind }]);
            return;
        }
        if let Some(parent) = place.parent.filter(|_| place.index == 0) {
            // At the top of a callout the tag goes first, which leaves a quote. Then the block leaves the quote.
            if matches!(parent.kind, BlockKind::Callout { .. }) {
                self.apply(vec![Op::SetKind { block: parent.id.clone(), kind: BlockKind::Quote }]);
            } else {
                self.lift(&id, false);
            }
            return;
        }
        let leaves = tree::leaves(doc);
        let before = leaves.iter().position(|leaf| leaf.id == id).and_then(|index| index.checked_sub(1));
        let Some(before) = before.map(|index| leaves[index]) else { return };
        if tree::is_text(before) {
            let pos = self.join(&before.id.clone(), &id);
            self.place_with(pos, caret.marks);
        } else {
            // A divider, or anything else with no text to join, is removed whole.
            self.apply(vec![Op::RemoveBlock { block: tree::removal(doc, &before.id) }]);
        }
    }

    fn delete_at_end(&mut self, caret: Caret) {
        let id = caret.selection.head.block.clone();
        let doc = self.doc();
        let leaves = tree::leaves(doc);
        let after = leaves.iter().position(|leaf| leaf.id == id).and_then(|index| leaves.get(index + 1));
        let Some(after) = after else { return };
        if tree::is_text(after) {
            let pos = self.join(&id, &after.id.clone());
            self.place_with(pos, caret.marks);
        } else {
            self.apply(vec![Op::RemoveBlock { block: tree::removal(doc, &after.id) }]);
        }
    }

    /// Moves a block out of its quote or callout, to just before or just after it.
    fn lift(&mut self, id: &BlockId, after: bool) {
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(id), tree::place(doc, id)) else { return };
        let Some(parent) = place.parent else { return };
        let Some(outer) = tree::place(doc, &parent.id) else { return };
        let alone = parent.body.blocks().len() == 1;
        let mut ops = vec![Op::RemoveBlock { block: id.clone() }];
        if alone {
            ops.push(Op::RemoveBlock { block: parent.id.clone() });
        }
        let index = outer.index + usize::from(after && !alone);
        ops.push(Op::InsertBlock { parent: outer.parent_id(), index, block: block.clone() });
        self.apply(ops);
    }

    /// Makes a line for the caret outside what holds it in, below or above, when the document has no text
    /// there to move to. A code block is left on its own. A quote or callout is left from the last line in
    /// it, and so is anything that only blocks without text follow. Returns whether there was such a place.
    pub(crate) fn open(&mut self, down: bool, platform: &mut dyn Platform) -> bool {
        let Some(head) = self.selection().map(|selection| selection.head.clone()) else { return false };
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(&head.block), tree::place(doc, &head.block)) else {
            return false;
        };
        let code = text::is_source(block);
        let outermost = |place: &tree::Place| match down {
            true => place.index + 1 == place.siblings(doc).len(),
            false => place.index == 0,
        };
        let parent = place.parent.filter(|_| !code && outermost(&place));
        let around = parent.and_then(|parent| tree::place(doc, &parent.id));
        let from = around.as_ref().unwrap_or(&place);
        if !(code || parent.is_some() || !outermost(from)) {
            return false;
        }
        let (into, len) = (from.parent_id(), from.siblings(doc).len());
        let text = block.plain_text();

        if block.kind == BlockKind::Paragraph && text.is_empty() {
            // An empty line is already a line to type on, so it moves there itself.
            let alone = parent.is_some_and(|parent| parent.body.blocks().len() == 1);
            let mut ops = vec![Op::RemoveBlock { block: block.id.clone() }];
            ops.extend(parent.filter(|_| alone).map(|parent| Op::RemoveBlock { block: parent.id.clone() }));
            let gone = usize::from(parent.is_none() || alone);
            let index = if down { len - gone } else { 0 };
            ops.push(Op::InsertBlock { parent: into, index, block: block.clone() });
            self.apply(ops);
            self.place(head);
            return true;
        }
        let new = BlockId::new(platform.new_id());
        let mut ops = Vec::new();
        if code && down && head.offset == text.len() && text.ends_with('\n') {
            // The empty last line was the way out before there was one, so it is not left behind.
            ops.push(Op::DeleteText { block: block.id.clone(), range: text.len() - 1..text.len() });
        }
        let line = Block::new(new.clone(), BlockKind::Paragraph);
        ops.push(Op::InsertBlock { parent: into, index: if down { len } else { 0 }, block: line });
        self.apply(ops);
        self.place(Pos::new(new, 0));
        true
    }

    /// Puts the caret below everything: at the end of the text the document ends in, or on a line made
    /// after a last block that would hold the caret in or cannot hold one.
    pub(crate) fn line_below(&mut self, platform: &mut dyn Platform) {
        if let Some(last) = self.doc().blocks.last().filter(|last| tree::is_text(last) && !text::is_source(last)) {
            return self.place(Pos::new(last.id.clone(), text::len(last)));
        }
        let new = BlockId::new(platform.new_id());
        let line = Block::new(new.clone(), BlockKind::Paragraph);
        if self.apply(vec![Op::InsertBlock { parent: None, index: self.doc().blocks.len(), block: line }]) {
            self.place(Pos::new(new, 0));
        }
    }

    /// Steps out of the formula the caret is in: past inline maths, or to the line below a formula block.
    pub(crate) fn leave_formula(&mut self, platform: &mut dyn Platform) {
        let Some((id, source)) = self.formula().map(|(id, source)| (id.clone(), source)) else { return };
        let Some(mut marks) = self.caret.as_ref().map(|caret| caret.marks.clone()) else { return };
        let end = Pos::new(id, source.end);
        if !self.block(&end.block).is_some_and(text::is_source) {
            marks.remove(&Mark::Math);
            return self.place_with(end, marks);
        }
        self.place(end.clone());
        match self.target(&end, Motion::Right, &mut None, platform) {
            next if next == end => _ = self.open(true, platform),
            next => self.place(next),
        }
    }

    /// Breaks the block at the caret, as Enter does.
    pub(crate) fn split(&mut self, platform: &mut dyn Platform) -> Kind {
        self.delete_selection();
        let Some(caret) = self.caret.clone() else { return Kind::Other };
        if caret.marks.contains(&Mark::Math) && self.formula().is_some() {
            // A formula is not broken in two. Enter says it is done.
            self.leave_formula(platform);
            return Kind::Other;
        }
        let at = caret.selection.head.clone();
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(&at.block), tree::place(doc, &at.block)) else {
            return Kind::Other;
        };
        let (id, text) = (at.block.clone(), block.plain_text());
        let (at_start, at_end) = (at.offset == 0, at.offset == text.len());
        let paragraph = BlockKind::Paragraph;

        match &block.kind {
            BlockKind::Code { .. } | BlockKind::Formula if at_end && rules::closes(&block.kind, &text) => {
                // A closing fence ends code or a formula as it does in Markdown. An empty line does not,
                // because source has empty lines of its own.
                let new = BlockId::new(platform.new_id());
                let fence = text.rfind('\n').unwrap_or(0)..text.len();
                let after = Op::InsertBlock {
                    parent: place.parent_id(),
                    index: place.index + 1,
                    block: Block::new(new.clone(), paragraph),
                };
                self.apply(vec![Op::DeleteText { block: id, range: fence }, after]);
                self.place(Pos::new(new, 0));
                self.rule = true;
                return Kind::Rule;
            }
            BlockKind::Code { .. } | BlockKind::Formula => {
                self.insert("\n");
                return Kind::Other;
            }
            BlockKind::Paragraph if at_end => {
                // A fence opens code, and `$$` a formula.
                let code = rules::fence(&text).map(|language| BlockKind::Code { language });
                if let Some(kind) = code.or_else(|| (text == "$$").then_some(BlockKind::Formula)) {
                    let clear = Op::DeleteText { block: id.clone(), range: 0..text.len() };
                    self.apply(vec![clear, Op::SetKind { block: id.clone(), kind }]);
                    self.place(Pos::new(id, 0));
                    self.rule = true;
                    return Kind::Rule;
                }
            }
            _ => {}
        }

        if text.is_empty() {
            // Enter in an empty item or quoted line steps out of the list or the quote instead.
            let out = match &block.kind {
                BlockKind::ListItem { list, depth } if *depth > 0 => {
                    Some(BlockKind::ListItem { list: *list, depth: depth - 1 })
                }
                BlockKind::ListItem { .. } => Some(paragraph.clone()),
                _ => None,
            };
            if let Some(kind) = out {
                self.apply(vec![Op::SetKind { block: id, kind }]);
                return Kind::Other;
            }
            let last = place.parent.is_some_and(|parent| place.index + 1 == parent.body.blocks().len());
            if last && block.kind == paragraph {
                self.lift(&id, true);
                return Kind::Other;
            }
        }

        let new = BlockId::new(platform.new_id());
        let mut ops = vec![Op::Split { at, new_id: new.clone() }];
        match &block.kind {
            // A heading is followed by body text, and one pushed down leaves body text behind it.
            BlockKind::Heading { .. } if at_end => ops.push(Op::SetKind { block: new.clone(), kind: paragraph }),
            BlockKind::Heading { .. } if at_start => ops.push(Op::SetKind { block: id, kind: paragraph }),
            // The tick stays with the text it was given to.
            BlockKind::ListItem { list: ListKind::Task { checked: true }, depth } => {
                let kind = BlockKind::ListItem { list: ListKind::Task { checked: false }, depth: *depth };
                ops.push(Op::SetKind { block: if at_start { id } else { new.clone() }, kind });
            }
            _ => {}
        }
        self.apply(ops);
        self.place_with(Pos::new(new, 0), caret.marks);
        Kind::Other
    }

    pub(crate) fn indent(&mut self, deeper: bool) {
        let Some(caret) = self.caret.clone() else { return };
        let head = caret.selection.head.clone();
        let doc = self.doc();
        let Some(block) = doc.block(&head.block) else { return };
        if text::is_source(block) {
            if deeper {
                return self.insert(CODE_INDENT);
            }
            let text = block.plain_text();
            let line = text::line_start(&text, head.offset);
            let spaces = text[line..].bytes().take(CODE_INDENT.len()).take_while(|byte| *byte == b' ').count();
            if spaces > 0 {
                self.apply(vec![Op::DeleteText { block: head.block.clone(), range: line..line + spaces }]);
                self.place(Pos::new(head.block, head.offset.saturating_sub(spaces).max(line)));
            }
            return;
        }

        let selected: BTreeSet<BlockId> = self.touched(&caret.selection);
        let mut depths: BTreeMap<BlockId, u8> = BTreeMap::new();
        let mut ops = Vec::new();
        for block in tree::texts(doc).into_iter().filter(|block| selected.contains(&block.id)) {
            let BlockKind::ListItem { list, depth } = &block.kind else { continue };
            let to = if deeper {
                // An item nests at most one level below the item before it.
                let before = tree::place(doc, &block.id)
                    .and_then(|place| place.index.checked_sub(1).map(|index| &place.siblings(doc)[index]));
                let limit = match before {
                    Some(Block { id, kind: BlockKind::ListItem { depth, .. }, .. }) => {
                        depths.get(id).copied().unwrap_or(*depth) + 1
                    }
                    _ => 0,
                };
                (depth + 1).min(limit.max(*depth)).min(MAX_DEPTH)
            } else {
                depth.saturating_sub(1)
            };
            if to != *depth {
                depths.insert(block.id.clone(), to);
                let kind = BlockKind::ListItem { list: *list, depth: to };
                ops.push(Op::SetKind { block: block.id.clone(), kind });
            }
        }
        self.apply(ops);
    }

    /// The text blocks a selection touches, including one that only holds a caret.
    fn touched(&self, selection: &Selection) -> BTreeSet<BlockId> {
        let (start, end) = self.ordered(selection);
        let texts = tree::texts(self.doc());
        let index = |pos: &Pos| texts.iter().position(|block| block.id == pos.block);
        let (Some(from), Some(to)) = (index(&start), index(&end)) else { return BTreeSet::new() };
        texts[from..=to].iter().map(|block| block.id.clone()).collect()
    }

    pub(crate) fn toggle_mark(&mut self, mark: Mark) {
        let Some(mut caret) = self.caret.clone() else { return };
        if caret.selection.is_caret() {
            // Nothing is selected, so the mark waits for the text typed next.
            if !caret.marks.remove(&mark) {
                caret.marks.insert(mark);
            }
            self.caret = Some(caret);
            return;
        }
        let doc = self.doc();
        let prose = |id: &BlockId| doc.block(id).filter(|block| !text::is_source(block));
        let pieces: Vec<_> = self.pieces(&caret.selection).into_iter().filter(|(id, _)| prose(id).is_some()).collect();
        let marked = |(id, range): &(BlockId, Range<usize>)| {
            let runs = prose(id).map_or(&[][..], |block| block.body.runs());
            text::runs_in(runs, range.clone()).all(|run| run.marks.contains(&mark))
        };
        let on = !pieces.iter().all(marked);
        let ops = pieces.into_iter().map(|(block, range)| Op::SetMark { block, range, mark: mark.clone(), on });
        self.apply(ops.collect());
        self.set_selection(caret.selection);
    }

    pub(crate) fn set_link(&mut self, link: Option<String>) {
        let Some(caret) = self.caret.clone() else { return };
        let pieces = if caret.selection.is_caret() {
            // A caret stands for the whole of the link it is in.
            let pos = &caret.selection.head;
            let (Some(block), Some(current)) = (self.block(&pos.block), self.link()) else { return };
            let same = |run: &Run| run.link.as_deref() == Some(current);
            let runs = block.body.runs();
            let range = text::stretch_start(runs, pos.offset, same)..text::stretch_end(runs, pos.offset, same);
            vec![(pos.block.clone(), range)]
        } else {
            self.pieces(&caret.selection)
        };
        let ops = pieces.into_iter().map(|(block, range)| Op::SetLink { block, range, link: link.clone() });
        self.apply(ops.collect());
    }

    pub(crate) fn toggle_checked(&mut self) {
        let Some(id) = self.selection().map(|selection| selection.head.block.clone()) else { return };
        let Some(BlockKind::ListItem { list: ListKind::Task { checked }, depth }) = self.block(&id).map(|b| &b.kind)
        else {
            return;
        };
        let kind = BlockKind::ListItem { list: ListKind::Task { checked: !checked }, depth: *depth };
        self.apply(vec![Op::SetKind { block: id, kind }]);
    }

    pub(crate) fn set_block(&mut self, kind: BlockKind, platform: &mut dyn Platform) {
        match kind.shape() {
            BodyShape::Text => self.set_kind(kind),
            BodyShape::Blocks => self.wrap(kind, platform),
            BodyShape::None if kind == BlockKind::Divider => self.insert_divider(platform),
            BodyShape::None => {}
        }
    }

    /// Gives every selected text block `kind`, or makes them paragraphs when they all have it already.
    fn set_kind(&mut self, kind: BlockKind) {
        let Some(caret) = self.caret.clone() else { return };
        let selected = self.touched(&caret.selection);
        let blocks: Vec<&Block> =
            tree::texts(self.doc()).into_iter().filter(|block| selected.contains(&block.id)).collect();
        // List items are the same kind of thing whatever their depth or tick.
        let same = |block: &&Block| match (&block.kind, &kind) {
            (BlockKind::ListItem { list: a, .. }, BlockKind::ListItem { list: b, .. }) => {
                std::mem::discriminant(a) == std::mem::discriminant(b)
            }
            (a, b) => a == b,
        };
        let off = blocks.iter().all(same);
        let ops = blocks.iter().filter_map(|block| {
            let to = match (&block.kind, &kind) {
                _ if off => BlockKind::Paragraph,
                (BlockKind::ListItem { depth, .. }, BlockKind::ListItem { list, .. }) => {
                    BlockKind::ListItem { list: *list, depth: *depth }
                }
                _ => kind.clone(),
            };
            (to != block.kind).then(|| Op::SetKind { block: block.id.clone(), kind: to })
        });
        self.apply(ops.collect());
        // Marks mean nothing in code, so a caret entering or leaving it starts afresh.
        self.set_selection(caret.selection);
    }

    /// Puts the selected blocks in a quote or callout. Inside one already, changes it or takes it away.
    fn wrap(&mut self, kind: BlockKind, platform: &mut dyn Platform) {
        let Some(selection) = self.selection().cloned() else { return };
        let doc = self.doc();
        let Some(place) = tree::place(doc, &selection.head.block) else { return };
        if let Some(parent) = place.parent {
            let Some(outer) = tree::place(doc, &parent.id) else { return };
            if parent.kind != kind {
                self.apply(vec![Op::SetKind { block: parent.id.clone(), kind }]);
                return;
            }
            let mut ops = vec![Op::RemoveBlock { block: parent.id.clone() }];
            let children = parent.body.blocks().iter().enumerate();
            ops.extend(children.map(|(i, child)| Op::InsertBlock {
                parent: outer.parent_id(),
                index: outer.index + i,
                block: child.clone(),
            }));
            self.apply(ops);
            return;
        }
        // The blocks wrapped are the top-level ones from the anchor's to the head's.
        let other = tree::place(doc, &selection.anchor.block).filter(|place| place.parent.is_none());
        let other = other.map_or(place.index, |place| place.index);
        let (from, to) = (place.index.min(other), place.index.max(other));
        let children = doc.blocks[from..=to].to_vec();
        let mut ops: Vec<Op> = children.iter().map(|child| Op::RemoveBlock { block: child.id.clone() }).collect();
        let mut container = Block::new(platform.new_id().as_str(), kind);
        container.body = Body::Blocks(children);
        ops.push(Op::InsertBlock { parent: None, index: from, block: container });
        self.apply(ops);
    }

    /// Adds a divider after the caret's block and leaves the caret in the text below it. An empty paragraph
    /// becomes that text, so the divider takes its place.
    fn insert_divider(&mut self, platform: &mut dyn Platform) {
        let Some(id) = self.selection().map(|selection| selection.head.block.clone()) else { return };
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(&id), tree::place(doc, &id)) else { return };
        let divider = Block::new(platform.new_id().as_str(), BlockKind::Divider);
        let parent = place.parent_id();
        if block.kind == BlockKind::Paragraph && text::len(block) == 0 {
            self.apply(vec![Op::InsertBlock { parent, index: place.index, block: divider }]);
            return;
        }
        let mut ops = vec![Op::InsertBlock { parent: parent.clone(), index: place.index + 1, block: divider }];
        let below = match place.siblings(doc).get(place.index + 1).filter(|below| tree::is_text(below)) {
            Some(below) => below.id.clone(),
            None => {
                let new = BlockId::new(platform.new_id());
                let block = Block::new(new.clone(), BlockKind::Paragraph);
                ops.push(Op::InsertBlock { parent, index: place.index + 2, block });
                new
            }
        };
        self.apply(ops);
        self.place(Pos::new(below, 0));
    }
}
