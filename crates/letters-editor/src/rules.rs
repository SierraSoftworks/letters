//! Markdown input rules: shorthand that turns into formatting as it is typed.
//!
//! A rule fires on the character that completes it and lands as its own undo step, so Backspace or Undo
//! straight after gives back the characters as typed.

use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{Block, BlockId, BlockKind, Body, ListKind, Mark, Op, Pos, Run, Tone};

use crate::markers::Sides;
use crate::{Caret, Editor, Platform, text};

/// Whether a block is a quote or a callout, which a tag at its top makes a callout.
fn quoted(block: &Block) -> bool {
    matches!(block.kind, BlockKind::Quote | BlockKind::Callout { .. })
}

/// What shorthand at the start of a block turns the block into.
#[derive(Debug, PartialEq)]
enum BlockRule {
    Kind(BlockKind),
    Quote,
    Divider,
    Callout(Tone),
}

/// The block shorthand completed by `typed`, the text from the start of a block to the caret. `rest` is the
/// text after the caret.
fn block_rule(typed: &str, rest: &str, kind: &BlockKind) -> Option<BlockRule> {
    let item = |list, depth| Some(BlockRule::Kind(BlockKind::ListItem { list, depth }));
    let task = |marker: &str, depth| match marker {
        "[ ]" | "[]" => item(ListKind::Task { checked: false }, depth),
        "[x]" | "[X]" => item(ListKind::Task { checked: true }, depth),
        _ => None,
    };
    if let Some(tone) = callout(typed) {
        return Some(BlockRule::Callout(tone));
    }
    match (kind, typed.strip_suffix(' ')) {
        (BlockKind::Paragraph, None) => (typed == "---" && rest.is_empty()).then_some(BlockRule::Divider),
        (BlockKind::Paragraph, Some(marker)) => match marker {
            "#" | "##" | "###" | "####" => Some(BlockRule::Kind(BlockKind::Heading { level: marker.len() as u8 })),
            "-" | "*" | "+" => item(ListKind::Bullet, 0),
            ">" => Some(BlockRule::Quote),
            _ if ordered(marker) => item(ListKind::Ordered, 0),
            _ => task(marker, 0),
        },
        // A bullet followed by a checkbox is a task.
        (BlockKind::ListItem { list: ListKind::Bullet, depth }, Some(marker)) => task(marker, *depth),
        _ => None,
    }
}

/// `1.` and the like.
fn ordered(marker: &str) -> bool {
    let digits = marker.strip_suffix('.').unwrap_or_default();
    (1..=9).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// The tone a callout tag such as `[!INFO]` names. GitHub's alert names are read as aliases.
fn callout(tag: &str) -> Option<Tone> {
    let name = tag.strip_prefix("[!")?.strip_suffix(']')?;
    Some(match name.to_ascii_uppercase().as_str() {
        "TIP" => Tone::Tip,
        "INFO" | "NOTE" | "IMPORTANT" => Tone::Info,
        "WARN" | "WARNING" => Tone::Warn,
        "DANGER" | "CAUTION" => Tone::Danger,
        _ => return None,
    })
}

/// The language of a code fence such as ```` ```rust ````, when `text` is one. `Some(None)` is a fence that
/// names no language.
pub(crate) fn fence(text: &str) -> Option<Option<String>> {
    let language = text.strip_prefix("```")?;
    let named = !language.contains(|c: char| c == '`' || c.is_whitespace());
    named.then(|| (!language.is_empty()).then(|| language.to_owned()))
}

/// Whether the last line of a block of source is the fence that closes it: of code, or of a formula.
pub(crate) fn closes(kind: &BlockKind, text: &str) -> bool {
    let fence = if *kind == BlockKind::Formula { "$$" } else { "```" };
    text.rsplit('\n').next() == Some(fence)
}

/// `typed` with every escaped dollar sign hidden. `\$` is a dollar sign in TeX and in prose, never a marker.
fn unescaped(typed: &str) -> String {
    typed.replace("\\$", "\\\u{1}")
}

/// The formula that the `$` ending `typed` closes: the opening and closing markers of `$x$` or `$$x$$`.
fn maths(typed: &str) -> Option<(Range<usize>, Range<usize>)> {
    let (open, close) = pair(&unescaped(typed), '$')?;
    let content = &typed[open.end..close.start];
    let edged = content.starts_with(char::is_whitespace) || content.ends_with(char::is_whitespace);
    // A sum of money is no formula, nor is the first of two as in `$5-$10`.
    let sum = content.trim_end_matches(['-', '–']);
    let money = sum.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ','));
    (close.len() <= 2 && !edged && !money).then_some((open, close))
}

/// Whether `typed` ends inside a formula that is still being written: after a `$` with TeX straight after
/// it. Shorthand typed there is TeX, and stays as typed.
fn in_maths(typed: &str) -> bool {
    let typed = unescaped(typed);
    let after = typed.rfind('$').map(|at| &typed[at + 1..]);
    after.is_some_and(|after| after.starts_with(|c: char| !c.is_whitespace() && !c.is_ascii_digit()))
}

/// A span of text between a pair of typed markers.
#[derive(Debug, PartialEq)]
struct Span {
    open: Range<usize>,
    close: Range<usize>,
    style: Style,
}

#[derive(Debug, PartialEq)]
enum Style {
    Marks(&'static [Mark]),
    Link(String),
}

/// The inline shorthand completed by `typed`, the text from the start of a block to the caret.
fn inline_rule(typed: &str) -> Option<Span> {
    let last = typed.chars().next_back()?;
    if last == ')' {
        return link(typed);
    }
    let (open, close) = pair(typed, last)?;
    let content = &typed[open.end..close.start];
    let edged = content.starts_with(char::is_whitespace) || content.ends_with(char::is_whitespace);
    let marks: &[Mark] = match (last, close.len()) {
        ('*', 1) if !edged => &[Mark::Italic],
        ('*', 2) if !edged => &[Mark::Bold],
        ('*', 3) if !edged => &[Mark::Bold, Mark::Italic],
        ('~', 2) if !edged => &[Mark::Strike],
        ('`', 1) if !content.trim().is_empty() => &[Mark::Code],
        _ => return None,
    };
    Some(Span { open, close, style: Style::Marks(marks) })
}

/// The closing run of `marker` characters that ends `typed`, and the opening run of the same length before
/// it. Runs of another length are not a pair, which is what keeps `**bold*` from reading as italic.
fn pair(typed: &str, marker: char) -> Option<(Range<usize>, Range<usize>)> {
    let close = typed.trim_end_matches(marker).len()..typed.len();
    let before = &typed[..close.start];
    let open_end = before.rfind(marker)? + marker.len_utf8();
    let open = before[..open_end].trim_end_matches(marker).len()..open_end;
    (open.len() == close.len() && open.end < close.start).then_some((open, close))
}

/// `[text](address)`, but not the image `![alt](address)`.
fn link(typed: &str) -> Option<Span> {
    let middle = typed.strip_suffix(')')?.rfind("](")?;
    let open = typed[..middle].rfind('[')?;
    let address = &typed[middle + 2..typed.len() - 1];
    let valid = open + 1 < middle
        && !address.is_empty()
        && !address.contains(char::is_whitespace)
        && !typed[..open].ends_with('!');
    valid.then(|| Span { open: open..open + 1, close: middle..typed.len(), style: Style::Link(address.to_owned()) })
}

impl Editor {
    /// Applies the shorthand that the text just typed completes. Returns whether any did.
    pub(crate) fn apply_rules(&mut self, platform: &mut dyn Platform) -> bool {
        let at = |caret: &Caret| caret.selection.is_caret() && !caret.picture;
        let Some(caret) = self.caret.clone().filter(at) else { return false };
        let pos = caret.selection.head;
        // A stretch of text on its own has no figure to label or to refer to, and stays the block it is.
        let page = !self.inline_only;
        if page && (self.label_rule(&pos) || self.reference_rule(&pos, &caret.marks)) {
            return true;
        }
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(&pos.block), self.seat(&pos.block)) else {
            return false;
        };
        if text::is_source(block) {
            return false;
        }
        let (id, text) = (pos.block.clone(), block.plain_text());
        let (typed, rest) = text.split_at(pos.offset);
        let clear = Op::DeleteText { block: id.clone(), range: 0..pos.offset };

        match block_rule(typed, rest, &block.kind).filter(|_| page) {
            Some(BlockRule::Kind(kind)) => {
                self.apply(vec![clear, Op::SetKind { block: id.clone(), kind }]);
            }
            // Quotes do not nest by shorthand, which keeps `>` typed inside one as text.
            Some(BlockRule::Quote) if place.parent.is_none() => {
                let mut inner = block.clone();
                inner.body = Body::Text(text::slice(block.body.runs(), pos.offset..text.len()));
                let mut quote = Block::new(platform.new_id().as_str(), BlockKind::Quote);
                quote.body = Body::Blocks(vec![inner]);
                let insert = Op::InsertBlock { parent: None, index: place.index, block: quote };
                self.apply(vec![Op::RemoveBlock { block: id.clone() }, insert]);
            }
            Some(BlockRule::Divider) => {
                // The divider goes in above, and the emptied paragraph is the text below it.
                let divider = Block::new(platform.new_id().as_str(), BlockKind::Divider);
                let insert = Op::InsertBlock { parent: place.parent_id(), index: place.index, block: divider };
                self.apply(vec![clear, insert]);
            }
            // The tag belongs at the top of a quote, and retyping it in a callout changes the tone.
            Some(BlockRule::Callout(tone)) if place.index == 0 && place.parent.is_some_and(quoted) => {
                let parent = place.parent_id().expect("the block has a parent");
                let kind = self
                    .doc()
                    .block(&parent)
                    .map_or(BlockKind::callout(tone), |held| crate::callout::toned(&held.kind, tone));
                self.apply(vec![clear, Op::SetKind { block: parent, kind }]);
            }
            _ => return self.apply_inline_rule(&id, typed, &caret.marks, platform),
        }
        self.place(Pos::new(self.renamed(&id), 0));
        true
    }

    /// Turns `$x$` into inline maths, and a paragraph that is all `$$x$$` into a formula block.
    fn apply_maths_rule(
        &mut self,
        id: &BlockId,
        typed: &str,
        marks: &BTreeSet<Mark>,
        platform: &mut dyn Platform,
    ) -> bool {
        let Some((open, close)) = maths(typed) else { return false };
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(id), self.seat(id)) else { return false };
        if text::marks_after(block.body.runs(), open.start).contains(&Mark::Code) {
            return false;
        }
        let content = open.start..close.start - open.len();
        let delete = |range: &Range<usize>| Op::DeleteText { block: id.clone(), range: range.clone() };
        let whole = open == (0..2) && close.end == text::len(block) && block.kind == BlockKind::Paragraph;
        let below = (place.parent_id(), place.index + 1);
        self.apply(vec![delete(&close), delete(&open)]);
        if whole {
            // A display formula stands alone, and the writing goes on below it.
            let new = BlockId::new(platform.new_id());
            let line = Block::new(new.clone(), BlockKind::Paragraph);
            let mut ops = self.format(id, content, &BTreeSet::new(), &None);
            ops.push(Op::SetKind { block: id.clone(), kind: BlockKind::Formula });
            ops.push(Op::InsertBlock { parent: below.0, index: below.1, block: line });
            self.apply(ops);
            self.place(Pos::new(new, 0));
        } else {
            // TeX is plain text, whatever it was typed in.
            let ops = self.format(id, content.clone(), &BTreeSet::from([Mark::Math]), &None);
            self.apply(ops);
            // The caret stays outside the formula it has just closed.
            self.place_with(Pos::new(id.clone(), content.end), marks.clone());
        }
        true
    }

    fn apply_inline_rule(
        &mut self,
        id: &BlockId,
        typed: &str,
        marks: &BTreeSet<Mark>,
        platform: &mut dyn Platform,
    ) -> bool {
        // Inside code and maths the markers are meant as written.
        if marks.contains(&Mark::Code) || marks.contains(&Mark::Math) {
            return false;
        }
        // Where no maths is written a dollar sign marks nothing, and holds no shorthand back.
        let maths = !self.inline_only;
        if maths && typed.ends_with('$') {
            return self.apply_maths_rule(id, typed, marks, platform);
        }
        let in_maths = maths && in_maths(typed);
        if let Some(before) = typed.strip_suffix(' ') {
            // A space after the address of a web page makes a link of it.
            return !in_maths && self.link_typed(id, before.len());
        }
        let Some(span) = inline_rule(typed).filter(|_| !in_maths) else { return false };
        let Some(runs) = self.block(id).map(|block| block.body.runs()) else { return false };
        if text::marks_after(runs, span.open.start).contains(&Mark::Code) {
            return false;
        }
        let content = span.open.start..span.close.start - span.open.len();
        let delete = |range: &Range<usize>| Op::DeleteText { block: id.clone(), range: range.clone() };
        let mut ops = vec![delete(&span.close), delete(&span.open)];
        let mut put_in = None;
        match span.style {
            Style::Marks(marks) => ops.extend(marks.iter().map(|mark| Op::SetMark {
                block: id.clone(),
                range: content.clone(),
                mark: mark.clone(),
                on: true,
            })),
            Style::Link(link) => {
                ops.push(Op::SetLink { block: id.clone(), range: content.clone(), link: Some(link.clone()) });
                put_in = Some(link);
            }
        }
        self.apply(ops);
        if let Some(link) = put_in {
            self.put_in(&link);
        }
        // The caret stays outside the pair it has just closed.
        self.place_with(Pos::new(id.clone(), content.end), marks.clone());
        true
    }

    /// Typing into a revealed marker edits the formatting, not the text: `#` at the start of a heading deepens
    /// it, and `*` beside an italic pair makes it bold. Returns whether `text` was taken that way.
    pub(crate) fn type_marker(&mut self, text: &str) -> bool {
        let Some(mut caret) = self.caret.clone().filter(|caret| caret.selection.is_caret()) else { return false };
        let pos = caret.selection.head.clone();
        let Some(block) = self.block(&pos.block) else { return false };
        match (text, &block.kind) {
            ("#", BlockKind::Heading { level }) if pos.offset == 0 => {
                // There is no fifth level, so a fifth `#` is dropped.
                if *level < 4 {
                    let kind = BlockKind::Heading { level: level + 1 };
                    self.apply(vec![Op::SetKind { block: pos.block, kind }]);
                }
                true
            }
            ("*", _) if !text::is_source(block) && !caret.marks.contains(&Mark::Math) => {
                let sides = Sides::at(block, pos.offset);
                let mut stops = sides.before(&caret.marks);
                stops.extend(sides.after(&caret.marks));
                let touches = |mark: &Mark| stops.iter().any(|(stop, _)| stop == mark);
                // `*` and `*` make `**`, and `**` and `*` make `***`.
                let (from, to) = match (touches(&Mark::Italic), touches(&Mark::Bold)) {
                    (true, false) => (Mark::Italic, vec![(Mark::Italic, false), (Mark::Bold, true)]),
                    (false, true) => (Mark::Bold, vec![(Mark::Italic, true)]),
                    _ => return false,
                };
                let runs = block.body.runs();
                let has = |run: &Run| run.marks.contains(&from);
                let start =
                    if sides.left.contains(&from) { text::stretch_start(runs, pos.offset, has) } else { pos.offset };
                let end =
                    if sides.right.contains(&from) { text::stretch_end(runs, pos.offset, has) } else { pos.offset };
                let carried = caret.marks.contains(&from);
                let mut ops = Vec::new();
                for (mark, on) in to {
                    if start < end {
                        ops.push(Op::SetMark { block: pos.block.clone(), range: start..end, mark: mark.clone(), on });
                    }
                    if carried && on {
                        caret.marks.insert(mark);
                    } else if carried {
                        caret.marks.remove(&mark);
                    }
                }
                self.apply(ops);
                self.caret = Some(caret);
                true
            }
            _ => false,
        }
    }
}
