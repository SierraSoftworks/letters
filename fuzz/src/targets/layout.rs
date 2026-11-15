//! Any document on the screen and on paper: it is laid out at any width, painted, paged and hit tested, and
//! every place layout names is a place in the text.

use letters_layout::{DocLayout, Fonts};
use letters_model::{Body, Document, Pos, Selection};
use letters_render::{DisplayList, Point, Rect};

use crate::edits::{written, written_seeds};

/// Window widths in logical pixels: none, a sliver, a phone, a page and a wide screen.
const WIDTHS: [f32; 6] = [0.0, 1.0, 37.5, 320.0, 720.0, 2400.0];
/// How many times idle layout is asked to go on. Far more than a bounded input needs.
const IDLE: usize = 4096;

/// Fails unless `pos` is between two characters of a block of text.
fn on_text(doc: &Document, pos: &Pos, from: &str) {
    let block = doc.block(&pos.block).filter(|block| matches!(block.body, Body::Text(_)));
    let on = block.is_some_and(|block| block.plain_text().is_char_boundary(pos.offset));
    assert!(on, "{from} names {pos:?}, which is no place in the text");
}

pub fn run(data: &[u8]) {
    let Some((&shape, rest)) = data.split_first() else { return };
    let Some((doc, style)) = written(rest) else { return };
    let width = WIDTHS[usize::from(shape) % WIDTHS.len()];
    let mut layout = DocLayout::new(Fonts { faces: letters_fonts::bundled() });
    layout.update(&doc, &style, width, None);

    // The first screen is painted before the rest has settled, and again once it has.
    let screen = Rect::new(0.0, 0.0, width, 600.0);
    layout.settle(screen, 0);
    layout.paint(screen, &mut DisplayList::default());
    let mut idle = 0;
    while layout.settle(screen, 4096) {
        idle += 1;
        assert!(idle < IDLE, "layout never settles");
    }
    let height = layout.height();
    assert!(height.is_finite() && height >= 0.0, "the document is {height} high");
    layout.paint(Rect::new(0.0, (height - 600.0).max(0.0), width, 600.0), &mut DisplayList::default());

    // A click anywhere lands in the text, and the caret can be drawn and moved from there.
    for step in 0..12u8 {
        let across = f32::from(step.wrapping_mul(shape | 1) % 13) / 12.0;
        let point = Point { x: width * across * 1.2 - 8.0, y: height * f32::from(step) / 11.0 };
        let Some(pos) = layout.hit(point) else { continue };
        on_text(&doc, &pos, "a click");
        let caret = layout.caret(&pos).unwrap_or_else(|| panic!("no caret can be drawn at {pos:?}"));
        assert!([caret.x, caret.y, caret.height].iter().all(|side| side.is_finite()), "a caret is at {caret:?}");
        for down in [false, true] {
            if let Some(stepped) = layout.line_step(&pos, caret.x, down) {
                on_text(&doc, &stepped, "a step to the next line");
            }
            if let Some(edge) = layout.line_edge(&pos, down) {
                on_text(&doc, &edge, "the edge of a line");
            }
        }
        let first = doc.walk().find(|block| matches!(block.body, Body::Text(_)));
        if let Some(first) = first {
            layout.selection(&Selection { anchor: Pos::new(first.id.clone(), 0), head: pos });
        }
    }

    // On paper every sheet is painted, and the links and headings it lists are on a sheet.
    let pages = layout.paginate(&style.page);
    assert!(!pages.sheets.is_empty(), "a document takes no sheet of paper");
    for index in 0..pages.sheets.len() {
        layout.paint_page(&pages, index, &mut DisplayList::default());
        for link in layout.page_links(&pages, index) {
            assert!(link.place.is_none_or(|(sheet, _)| sheet < pages.sheets.len()), "a reference leads off the paper");
        }
    }
    for heading in layout.outline(&pages) {
        assert!(heading.page < pages.sheets.len(), "a heading is on no sheet");
    }
}

pub fn seeds() -> Vec<Vec<u8>> {
    let widths = 0..WIDTHS.len() as u8;
    widths.flat_map(|shape| written_seeds().into_iter().map(move |seed| [&[shape][..], &seed].concat())).collect()
}
