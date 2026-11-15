//! Copy, cut and paste.
//!
//! A copy carries the selected blocks as `document.json` does, inside the clipboard's HTML, so that pasting
//! into Letters loses nothing. Other applications read the HTML it wraps, or the plain text, which is
//! Markdown. What other applications copied is read from its HTML, or from its text as Markdown.

use std::collections::BTreeSet;

use letters_convert::{from_html, from_markdown, to_html_fragment, to_markdown};
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Op, Pos, Run};
use serde_json::Value;

use crate::{Clip, Editor, Platform, text, tree};

/// The HTML attribute that holds the copied blocks.
const CARRIER: &str = "data-letters-blocks";

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"").replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&")
}

/// The text of every block, one to a line.
fn lines(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .flat_map(|block| match &block.body {
            Body::Text(_) => vec![block.plain_text()],
            Body::Blocks(children) => lines(children),
            Body::None => Vec::new(),
        })
        .collect()
}

/// The clipboard's HTML for `blocks`: what other applications read, inside an element that carries the
/// blocks themselves.
fn to_html(blocks: &[Block]) -> String {
    let json = Value::Array(blocks.iter().map(Block::to_value).collect()).to_string();
    format!("<div {CARRIER}=\"{}\">{}</div>", escape(&json), to_html_fragment(blocks))
}

/// The blocks that HTML copied from Letters carries.
fn carried(html: &str) -> Option<Vec<Block>> {
    let (_, rest) = html.split_once(&format!("{CARRIER}=\""))?;
    let (json, _) = rest.split_once('"')?;
    let Value::Array(blocks) = serde_json::from_str(&unescape(json)).ok()? else { return None };
    blocks.into_iter().map(|block| Block::from_value(block, "clipboard").ok()).collect()
}

/// The clipboard's plain text for `blocks`: their Markdown.
fn to_text(blocks: &[Block]) -> String {
    let markdown = |blocks: &[Block]| {
        let doc = Document { blocks: blocks.to_vec(), ..Document::default() };
        to_markdown(&doc).trim_end_matches('\n').to_owned()
    };
    // A phrase keeps the spaces at its edges as they are, which Markdown would have to escape.
    if let [phrase] = blocks
        && phrase.kind == BlockKind::Paragraph
    {
        let text = phrase.plain_text();
        let start = text.len() - text.trim_start_matches(' ').len();
        let end = text.trim_end_matches(' ').len().max(start);
        let mut inner = phrase.clone();
        inner.body = Body::Text(text::slice(phrase.body.runs(), start..end));
        return format!("{}{}{}", &text[..start], markdown(&[inner]), &text[end..]);
    }
    markdown(blocks)
}

/// The blocks that text from another application stands for, read as Markdown.
fn from_text(text: &str, platform: &mut dyn Platform) -> Vec<Block> {
    let mut read = |markdown: &str| from_markdown(markdown, &mut || platform.new_id()).0.blocks;
    let line = text.trim_matches('\n');
    if line.contains('\n') {
        return read(text);
    }
    // A phrase keeps the spaces at its edges, which Markdown would drop or read as an indent.
    let phrase = line.trim();
    let mut blocks = read(phrase);
    if let [Block { kind: BlockKind::Paragraph, body: Body::Text(runs), .. }] = blocks.as_mut_slice() {
        let start = line.len() - line.trim_start().len();
        runs.insert(0, Run::plain(&line[..start]));
        runs.push(Run::plain(&line[start + phrase.len()..]));
    }
    blocks
}

fn is_address(text: &str) -> bool {
    let schemes = ["http://", "https://", "mailto:"];
    schemes.iter().any(|scheme| text.len() > scheme.len() && text.starts_with(scheme))
        && !text.contains(char::is_whitespace)
}

/// The last text block in `blocks`, at any depth.
fn last_text(blocks: &[Block]) -> Option<&Block> {
    blocks.iter().rev().find_map(|block| match &block.body {
        Body::Text(_) => Some(block),
        Body::Blocks(children) => last_text(children),
        Body::None => None,
    })
}

/// A copy of `block` with the body `leaf` gives each text block or divider. A leaf it gives nothing is left
/// out, and so is a container left with nothing in it.
fn keep(block: &Block, leaf: &dyn Fn(&Block) -> Option<Body>) -> Option<Block> {
    let body = match &block.body {
        Body::Blocks(children) => {
            let kept: Vec<Block> = children.iter().filter_map(|child| keep(child, leaf)).collect();
            (!kept.is_empty()).then_some(Body::Blocks(kept))?
        }
        _ => leaf(block)?,
    };
    let (id, kind, lang) = (block.id.clone(), block.kind.clone(), block.lang.clone());
    Some(Block { id, kind, body, lang, page_break: block.page_break, extra: block.extra.clone() })
}

/// Gives `block` and the blocks inside it new ids where theirs are `taken`. A block that was cut comes
/// back under its own id, so whatever refers to it still does, and a copy is a block of its own.
fn renew(block: &mut Block, taken: &mut BTreeSet<BlockId>, platform: &mut dyn Platform) {
    if !taken.insert(block.id.clone()) {
        block.id = BlockId::new(platform.new_id());
        taken.insert(block.id.clone());
    }
    if let Body::Blocks(children) = &mut block.body {
        children.iter_mut().for_each(|child| renew(child, taken, platform));
    }
}

impl Editor {
    /// The selection as blocks that stand on their own.
    fn fragment(&self) -> Vec<Block> {
        let Some(selection) = self.selection() else { return Vec::new() };
        let (start, end) = self.ordered(selection);
        let doc = self.doc();
        if start.block == end.block {
            let Some(block) = doc.block(&start.block) else { return Vec::new() };
            // Part of a block is only text. A whole block keeps its kind.
            let whole = start.offset == 0 && end.offset == text::len(block);
            let mut piece = block.clone();
            if !whole {
                piece.kind = BlockKind::Paragraph;
            }
            piece.body = Body::Text(text::slice(block.body.runs(), start.offset..end.offset));
            return vec![piece];
        }
        let order: Vec<&BlockId> = doc.walk().map(|block| &block.id).collect();
        let index = |id: &BlockId| order.iter().position(|other| *other == id).unwrap_or(usize::MAX);
        let (from, to) = (index(&start.block), index(&end.block));
        // The fragment starts at the deepest quote or callout that holds both ends, so text copied inside
        // one does not bring it along.
        let mut scope = &doc.blocks;
        let holds = |block: &&Block| tree::contains(block, &start.block) && tree::contains(block, &end.block);
        while let Some(Body::Blocks(children)) = scope.iter().find(holds).map(|block| &block.body) {
            scope = children;
        }
        let leaf = |block: &Block| {
            let at = index(&block.id);
            match &block.body {
                Body::Text(runs) if (from..=to).contains(&at) => {
                    let from_offset = if at == from { start.offset } else { 0 };
                    let to_offset = if at == to { end.offset } else { text::len(block) };
                    Some(Body::Text(text::slice(runs, from_offset..to_offset)))
                }
                Body::None if from < at && at < to => Some(Body::None),
                _ => None,
            }
        };
        scope.iter().filter_map(|block| keep(block, &leaf)).collect()
    }

    pub(crate) fn copy(&mut self, platform: &mut dyn Platform) {
        let blocks = self.fragment();
        if !blocks.is_empty() {
            let clip = Clip { text: to_text(&blocks), html: Some(to_html(&blocks)), ..Clip::default() };
            platform.clipboard_write(clip);
        }
    }

    pub(crate) fn cut(&mut self, platform: &mut dyn Platform) {
        self.copy(platform);
        self.delete_selection();
    }

    /// Pastes the clipboard over the selection. `plain` takes only its text.
    pub(crate) fn paste(&mut self, plain: bool, platform: &mut dyn Platform) {
        let Some(clip) = platform.clipboard_read() else { return };
        if !self.ensure_caret(platform) {
            return;
        }
        let Some(selection) = self.selection() else { return };
        // Source takes text as it is written: code, and a formula's TeX.
        let code = self.block(&selection.head.block).is_some_and(text::is_source) || self.formula().is_some();
        let ranged = !selection.is_caret();
        let address = clip.text.trim();
        if !plain && !code && ranged && is_address(address) {
            // A web address pasted over a selection links it.
            return self.set_link(Some(address.to_owned()));
        }
        let marks = self.caret.as_ref().map(|caret| caret.marks.clone()).unwrap_or_default();
        let html = clip.html.as_deref();
        let own = html.and_then(carried).filter(|blocks| !blocks.is_empty());
        let text = clip.text.replace("\r\n", "\n").replace('\r', "\n");
        if plain || code {
            // Only the words are wanted. For a copy made in Letters those are not the Markdown beside it.
            let text = own.map_or(text, |blocks| lines(&blocks).join("\n"));
            return self.insert_lines(&text, platform);
        }
        // Formatted text keeps its own formatting. Bare text takes what is in force at the caret.
        let none = BTreeSet::new();
        let read = |html: &str, platform: &mut dyn Platform| from_html(html, &mut || platform.new_id()).0.blocks;
        let formatted = own.or_else(|| html.map(|html| read(html, platform)).filter(|blocks| !blocks.is_empty()));
        match formatted.map_or_else(|| (from_text(&text, platform), &marks), |blocks| (blocks, &none)) {
            (blocks, _) if blocks.is_empty() => self.insert_lines(&text, platform),
            (blocks, marks) => self.insert_blocks(blocks, marks, platform),
        }
    }

    /// Replaces the selection with `blocks`. Text poured into the block at the caret also takes `marks`.
    fn insert_blocks(&mut self, mut blocks: Vec<Block>, marks: &BTreeSet<Mark>, platform: &mut dyn Platform) {
        self.delete_selection();
        let Some(at) = self.selection().map(|selection| selection.head.clone()) else { return };
        let mut taken: BTreeSet<BlockId> = self.doc().walk().map(|block| block.id.clone()).collect();
        blocks.iter_mut().for_each(|block| renew(block, &mut taken, platform));
        let doc = self.doc();
        let (Some(target), Some(place)) = (doc.block(&at.block), tree::place(doc, &at.block)) else { return };
        let (len, parent, index) = (text::len(target), place.parent_id(), place.index + 1);

        if let [only] = blocks.as_slice()
            && tree::is_text(only)
        {
            // A single block of text is poured into the block at the caret, which takes its kind if it was
            // an empty paragraph.
            let adopt = len == 0 && target.kind == BlockKind::Paragraph && only.kind != target.kind;
            self.insert_runs(&at, only, marks);
            if adopt {
                self.apply(vec![Op::SetKind { block: at.block, kind: only.kind.clone() }]);
            }
            return;
        }

        // The text after the caret is set aside, the blocks go in, and the text at each edge is joined on.
        let tail = (at.offset < len).then(|| BlockId::new(platform.new_id()));
        if let Some(tail) = &tail {
            self.apply(vec![Op::Split { at: at.clone(), new_id: tail.clone() }]);
        }
        let inserts = blocks.iter().enumerate().map(|(i, block)| Op::InsertBlock {
            parent: parent.clone(),
            index: index + i,
            block: block.clone(),
        });
        self.apply(inserts.collect());
        let (Some(first), Some(last)) = (blocks.first(), blocks.last()) else { return };
        if at.offset > 0 && tree::is_text(first) {
            self.apply(vec![Op::Join { block: first.id.clone() }]);
        } else if at.offset == 0 && last_text(&blocks).is_some() {
            // The block at the caret had nothing before the caret, so the pasted blocks take its place.
            self.apply(vec![Op::RemoveBlock { block: at.block.clone() }]);
        }
        let end = if tree::is_text(last) {
            let end = Pos::new(last.id.clone(), text::len(last));
            if let Some(tail) = tail {
                self.apply(vec![Op::Join { block: tail }]);
            }
            Some(end)
        } else if let Some(tail) = tail {
            Some(Pos::new(tail, 0))
        } else {
            last_text(&blocks).map(|block| Pos::new(block.id.clone(), text::len(block)))
        };
        // A pasted block that was joined onto the one at the caret no longer exists under its own id.
        let end = end.and_then(|pos| self.clamp(&pos));
        let end = end.or_else(|| self.clamp(&Pos::new(at.block.clone(), usize::MAX)));
        if let Some(end) = end {
            self.place(end);
        }
    }

    /// Inserts the text of `source` at `at` with the formatting each of its runs has, and `marks` besides.
    fn insert_runs(&mut self, at: &Pos, source: &Block, marks: &BTreeSet<Mark>) {
        let text = source.plain_text();
        if text.is_empty() {
            return;
        }
        self.apply(vec![Op::InsertText { at: at.clone(), text: text.clone() }]);
        let mut offset = at.offset;
        let mut ops = Vec::new();
        for run in source.body.runs() {
            let range = offset..offset + run.text.len();
            offset = range.end;
            let marks = run.marks.union(marks).cloned().collect();
            ops.extend(self.format(&at.block, range, &marks, &run.link));
        }
        self.apply(ops);
        self.place(Pos::new(at.block.clone(), offset));
    }
}
