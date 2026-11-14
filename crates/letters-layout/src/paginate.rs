//! Splits the column into the pages of Print view and export.
//!
//! Pagination is a pass over leaves that are already laid out. Every line of a leaf is as tall as the next, so
//! the pass shapes no text: it only decides before which lines a page starts. A page then shows a band of
//! the column, cut between lines.

use std::ops::Range;

use letters_model::BlockId;
use letters_render::{Point, Rect};

use crate::flow::{Frame, Leaf};
use crate::theme;

/// A document split into pages. See [`crate::DocLayout::paginate`].
///
/// It describes the layout as it was when the pages were made, so paginate again after an update.
#[derive(Clone, Debug, PartialEq)]
pub struct Pages {
    /// The size of a sheet of paper, in logical pixels.
    pub width: f32,
    pub height: f32,
    /// The part of a sheet that text is set in, from the sheet's top left.
    pub area: Rect,
    /// The pages in order. There is always at least one.
    pub sheets: Vec<Sheet>,
}

impl Pages {
    /// Where sheet `index` shows the column's top left. Add it to a place in the column, such as a caret, to
    /// draw that on the sheet. Take it from a point on the sheet to hit test there.
    pub fn origin(&self, index: usize) -> Option<Point> {
        let sheet = self.sheets.get(index)?;
        Some(Point { x: self.area.x, y: self.area.y - sheet.band.0 })
    }
}

/// One page of a paginated document.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    /// The blocks on the page in order, each with the lines of it that the page holds. A block that runs
    /// over the end of a page is on the next one too, with the rest of its lines.
    pub blocks: Vec<(BlockId, Range<usize>)>,
    /// Where the page starts and ends in the column.
    pub(crate) band: (f32, f32),
    /// The leaf and the line that the page starts with.
    pub(crate) first: (usize, usize),
}

/// A page never ends with fewer lines of a block than this, and never starts with fewer.
const TOGETHER: usize = 2;

/// Room for the rounding of positions that are whole pixels in sum.
const SLACK: f32 = 0.01;

/// Where a page starts: before a line of a leaf, at a height in the column.
#[derive(Clone, Copy)]
struct Cut {
    leaf: usize,
    line: usize,
    y: f32,
}

/// Splits settled leaves into pages that hold `room` of the column each.
///
/// A leaf marked to break starts a page. A heading stays with the block after it. A block splits only where
/// two of its lines stay and two go, and never inside a line of code. A leaf that no rule lets onto the page
/// goes to the next one, unless it is all the page would hold: then the page takes what fits.
pub(crate) fn sheets(leaves: &[Leaf], frames: &[Frame], room: f32) -> Vec<Sheet> {
    if leaves.is_empty() {
        return vec![Sheet { blocks: Vec::new(), band: (0.0, 0.0), first: (0, 0) }];
    }
    // What a leaf shows above its text and takes with it to a new page: its own box, and the top of each
    // quote or callout that it opens.
    let mut heads: Vec<f32> = leaves.iter().map(|leaf| if leaf.boxed() { theme::BOX_PAD_Y } else { 0.0 }).collect();
    for frame in frames {
        heads[frame.first] = heads[frame.first].max(leaves[frame.first].top - frame.lead);
    }
    let top = |at: usize| leaves[at].y - heads[at];

    let mut cuts = vec![Cut { leaf: 0, line: 0, y: top(0) }];
    // The leaf to place, and how many of its lines earlier pages took.
    let (mut at, mut from) = (0, 0);
    while at < leaves.len() {
        let (leaf, page) = (&leaves[at], cuts[cuts.len() - 1]);
        let opens = (page.leaf, page.line) == (at, from);
        if leaf.breaks && from == 0 && !opens {
            cuts.push(Cut { leaf: at, line: 0, y: top(at) });
            continue;
        }
        let limit = page.y + room + SLACK;
        if leaf.y + leaf.height + leaf.bottom <= limit {
            (at, from) = (at + 1, 0);
            continue;
        }

        let count = leaf.lines.max(1) as usize;
        let line = leaf.height / count as f32;
        let fits = (((limit - leaf.y) / line).floor().max(0.0) as usize).min(count);
        let free = |before: &usize| leaf.joined.binary_search(&(*before as u32)).is_err();
        let cut = |before: usize| Cut { leaf: at, line: before, y: leaf.y + before as f32 * line };
        let stay = from + if from == 0 { TOGETHER } else { 1 };
        if let Some(before) = (stay..=fits.min(count.saturating_sub(TOGETHER))).rev().find(free) {
            cuts.push(cut(before));
            from = before;
        } else if !opens {
            // It goes to the next page whole, and takes along the headings it follows.
            let mut first = at;
            while first > page.leaf + 1 && leaves[first - 1].role.is_heading() {
                first -= 1;
            }
            cuts.push(Cut { leaf: first, line: 0, y: top(first) });
            (at, from) = (first, 0);
        } else {
            // It opens the page and the rules leave nowhere to split it, so the page takes what it can hold.
            // A piece taller than a page runs over.
            let held = (from + 1..=fits.min(count - 1)).rev().find(free);
            match held.or_else(|| (fits.max(from) + 1..count).find(free)) {
                Some(before) => {
                    cuts.push(cut(before));
                    from = before;
                }
                None => (at, from) = (at + 1, 0),
            }
        }
    }

    let end = leaves.last().map_or(0.0, |last| last.y + last.height + last.bottom);
    let sheet = |(index, page): (usize, &Cut)| {
        let next = cuts.get(index + 1);
        // Up to the leaf and the line that the next page starts with.
        let (last, upto) = next.map_or((leaves.len(), 0), |next| (next.leaf, next.line));
        let mut blocks = Vec::new();
        for (at, leaf) in leaves.iter().enumerate().take(last + 1).skip(page.leaf) {
            let start = if at == page.leaf { page.line } else { 0 };
            let end = if at == last { upto } else { leaf.lines.max(1) as usize };
            if let (Some(id), true) = (&leaf.id, start < end) {
                blocks.push((id.clone(), start..end));
            }
        }
        let bottom = next.map_or(end, |next| next.y).min(page.y + room);
        Sheet { blocks, band: (page.y, bottom), first: (page.leaf, page.line) }
    };
    cuts.iter().enumerate().map(sheet).collect()
}
