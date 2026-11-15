//! Finding text, and replacing what is found.
//!
//! A query is looked for in each block's text by itself, so nothing that is found runs from one block into
//! the next. Text that the page draws something else for is left out: the source of a formula, and the
//! words of a reference.

use std::collections::HashMap;
use std::ops::Range;

use letters_model::{Block, BlockId, BlockKind, Mark, Pos, Selection};
use regex::{Regex, RegexBuilder};

use crate::{Editor, text, tree};

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    /// Whether `text` is a regular expression, and not the characters to find. A replacement then takes
    /// what its groups found: `$1`, or `${name}`.
    pub regex: bool,
    /// Whether capitals and small letters are told apart.
    pub case: bool,
    /// Whether what is found must be whole words.
    pub word: bool,
}

/// Why a query cannot be looked for: what is wrong with its regular expression, in a few words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadQuery(pub String);

/// Which of the places a query was found at to go to from the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seek {
    /// The first that starts at or after the start of the selection: where the selection stays while more
    /// of what it holds is typed.
    Here,
    /// The first after the selection, or the document's first after its last.
    Next,
    /// The last before the selection, or the document's last before its first.
    Previous,
}

impl Query {
    /// The expression that finds the query, or `None` for one that asks for nothing.
    fn compile(&self) -> Result<Option<Regex>, BadQuery> {
        if self.text.is_empty() {
            return Ok(None);
        }
        let pattern = if self.regex { self.text.clone() } else { regex::escape(&self.text) };
        let pattern = if self.word { format!(r"\b(?:{pattern})\b") } else { pattern };
        // A block may hold several lines, and `^` and `$` mean the ends of each.
        let built = RegexBuilder::new(&pattern).case_insensitive(!self.case).multi_line(true).build();
        built.map(Some).map_err(|error| BadQuery(reason(&error)))
    }
}

/// What is wrong with an expression, without the drawing of where.
fn reason(error: &regex::Error) -> String {
    match error {
        regex::Error::CompiledTooBig(_) => "the pattern is too large".to_owned(),
        error => {
            let said = error.to_string();
            let last = said.lines().last().unwrap_or_default();
            last.trim_start_matches("error: ").to_owned()
        }
    }
}

/// A place a query was found at, and what a replacement puts there.
struct Found {
    block: BlockId,
    range: Range<usize>,
    with: String,
}

impl Found {
    fn selection(&self) -> Selection {
        Selection {
            anchor: Pos::new(self.block.clone(), self.range.start),
            head: Pos::new(self.block.clone(), self.range.end),
        }
    }
}

/// The parts of a block's text that the page draws something else for.
fn drawn_over(block: &Block) -> Vec<Range<usize>> {
    let mut at = 0;
    let mut ranges = Vec::new();
    for run in block.body.runs() {
        let end = at + run.text.len();
        if run.marks.iter().any(|mark| matches!(mark, Mark::Math | Mark::Ref(_))) {
            ranges.push(at..end);
        }
        at = end;
    }
    ranges
}

impl Editor {
    /// Every place a query is found at, in the order of the document, each a selection of what was found
    /// from its start to its end. A query with no text finds nothing.
    pub fn find(&self, query: &Query) -> Result<Vec<Selection>, BadQuery> {
        let Some(regex) = query.compile()? else { return Ok(Vec::new()) };
        Ok(self.places(&regex, None).iter().map(Found::selection).collect())
    }

    /// Which of `found`, the places [`Editor::find`] gave, to go to from the selection. `None` when
    /// nothing was found.
    pub fn seek(&self, found: &[Selection], seek: Seek) -> Option<usize> {
        let last = found.len().checked_sub(1)?;
        let Some((start, end)) = self.selection().map(|selection| self.ordered(selection)) else {
            return Some(if seek == Seek::Previous { last } else { 0 });
        };
        let texts = tree::texts(self.doc());
        let order: HashMap<&BlockId, usize> = texts.iter().enumerate().map(|(i, block)| (&block.id, i)).collect();
        let key = |pos: &Pos| (order.get(&pos.block).copied().unwrap_or(usize::MAX), pos.offset);
        Some(
            match seek {
                Seek::Here => Some(found.partition_point(|at| key(&at.anchor) < key(&start))).filter(|at| *at <= last),
                Seek::Next => Some(found.partition_point(|at| key(&at.anchor) < key(&end))).filter(|at| *at <= last),
                Seek::Previous => found.partition_point(|at| key(&at.head) <= key(&start)).checked_sub(1),
            }
            .unwrap_or(if seek == Seek::Previous { last } else { 0 }),
        )
    }

    /// Whether a replacement has anything to replace: the place `only` names, or with `None` any place.
    pub(crate) fn replaceable(&self, query: &Query, only: Option<&Selection>) -> bool {
        let Ok(Some(regex)) = query.compile() else { return false };
        let found = self.places(&regex, None);
        only.map_or(!found.is_empty(), |only| found.iter().any(|found| found.selection() == *only))
    }

    /// Puts `with` in place of what a query finds: at the place `only` names, or with `None` at every
    /// place. What is put in takes the formatting of the first character it replaces. After one
    /// replacement the caret stands behind it.
    pub(crate) fn replace_found(&mut self, query: &Query, with: &str, only: Option<&Selection>) {
        let Ok(Some(regex)) = query.compile() else { return };
        let mut found = self.places(&regex, Some((with, query.regex)));
        if let Some(only) = only {
            found.retain(|found| found.selection() == *only);
        }
        // Where each end of the selection is once the text before it has changed length.
        let moved = |pos: &Pos| {
            let mut offset = pos.offset;
            for found in found.iter().filter(|found| found.block == pos.block && found.range.start < pos.offset) {
                if found.range.end <= pos.offset {
                    offset = offset + found.with.len() - found.range.len();
                } else {
                    offset = offset - (pos.offset - found.range.start) + found.with.len();
                }
            }
            Pos::new(pos.block.clone(), offset)
        };
        let kept = self.caret.as_ref().map(|caret| {
            let selection = &caret.selection;
            (Selection { anchor: moved(&selection.anchor), head: moved(&selection.head) }, caret.picture)
        });
        // The last goes first, so that the places before it are still where they were found.
        for found in found.iter().rev() {
            let Some(first) =
                self.block(&found.block).and_then(|block| text::run_at(block.body.runs(), found.range.start))
            else {
                continue;
            };
            let (marks, link) = (first.marks.clone(), first.link.clone());
            self.replace(&found.block, found.range.clone(), &found.with, &marks, &link);
        }
        match (only, found.first(), kept) {
            (Some(_), Some(one), _) => self.place(Pos::new(one.block.clone(), one.range.start + one.with.len())),
            (None, _, Some((selection, false))) => self.set_selection(selection),
            _ => {}
        }
    }

    /// The places an expression is found at, in the order of the document. With `with`, each comes with
    /// what replaces it: the words as they are, or with the groups of what was found filled in.
    fn places(&self, regex: &Regex, with: Option<(&str, bool)>) -> Vec<Found> {
        let mut found = Vec::new();
        for block in tree::texts(self.doc()) {
            if block.kind == BlockKind::Formula {
                continue;
            }
            let text = block.plain_text();
            let over = drawn_over(block);
            let shows = |range: &Range<usize>| {
                !range.is_empty() && !over.iter().any(|over| over.start < range.end && range.start < over.end)
            };
            match with {
                Some((with, true)) => {
                    for groups in regex.captures_iter(&text) {
                        let range = groups.get(0).map_or(0..0, |all| all.range());
                        if shows(&range) {
                            let mut put = String::new();
                            groups.expand(with, &mut put);
                            found.push(Found { block: block.id.clone(), range, with: put });
                        }
                    }
                }
                _ => {
                    let with = with.map_or("", |(with, _)| with);
                    let ranges = regex.find_iter(&text).map(|at| at.range()).filter(shows);
                    found.extend(ranges.map(|range| Found { block: block.id.clone(), range, with: with.to_owned() }));
                }
            }
        }
        found
    }
}
