//! The Markdown markers revealed at the caret, with the markers the editor gives for it.

use letters_editor::{Clip, Command, Editor, InputEvent, Motion, Platform};
use letters_layout::{DocLayout, Palette};
use letters_model::{Block, BlockKind, Body, Document, Mark, Pos, ReplicaId, Run, Selection, Style, Tone};
use letters_render::{Color, Item, Point};
use proptest::prelude::*;

mod common;
use common::*;

/// Narrow enough that a sentence takes several lines.
const NARROW: f32 = 320.0;
const MUTED: Color = Palette::LIGHT.marker;

struct Desk;

impl Platform for Desk {
    fn now_ms(&self) -> u64 {
        0
    }
    fn clipboard_read(&mut self) -> Option<Clip> {
        None
    }
    fn clipboard_write(&mut self, _: Clip) {}
    fn new_id(&mut self) -> String {
        "new".into()
    }
}

/// A paragraph of runs, each plain or marked.
fn rich(id: &str, runs: &[(&str, &[Mark])]) -> Block {
    let runs = runs.iter().map(|(text, marks)| Run::marked(*text, marks.iter().cloned())).collect();
    Block { body: Body::Text(runs), ..paragraph(id, "") }
}

/// A sentence that wraps, with `word` marked on its second line.
fn sentence(marks: &[Mark]) -> Document {
    let before = "Rates change on the first of January, and every ";
    doc(vec![rich("a", &[(before, &[]), ("household", marks), (" on the scheme should check the notice.", &[])])])
}

/// The offset of the middle of the marked word in [`sentence`].
const INSIDE: usize = 52;

/// An editor and a layout of one document, as a shell holds them.
struct Page {
    editor: Editor,
    layout: DocLayout,
}

impl Page {
    fn new(doc: &Document, width: f32) -> Self {
        let mut layout = DocLayout::new(fonts());
        layout.update(doc, &Style::default(), width, None);
        Self { editor: Editor::new(doc.clone(), ReplicaId::from(1)), layout }
    }

    fn head(&self) -> Pos {
        self.editor.selection().expect("a caret").head.clone()
    }

    /// Shows the markers the editor gives for its caret. Returns whether any appear or go.
    fn reveal(&mut self) -> bool {
        self.layout.reveal(Some((&self.head(), &self.editor.markers())))
    }

    /// Puts the caret at a place and shows its markers.
    fn click(&mut self, block: &str, offset: usize) -> bool {
        self.editor.select(Selection::caret(Pos::new(block, offset)));
        self.reveal()
    }

    /// Steps the caret sideways and shows its markers.
    fn step(&mut self, to: Motion) -> bool {
        self.editor.run(Command::Move { to, extend: false }, &mut Desk);
        self.reveal()
    }

    fn painted(&self) -> Vec<Item> {
        painted(&self.layout, everything())
    }
}

/// The glyphs of the text, apart from those of markers.
fn text(items: &[Item]) -> Vec<(u32, f32, f32)> {
    let text: Vec<Item> = items.iter().filter(|item| !muted(item)).cloned().collect();
    placed(&text)
}

/// Whether an item is the glyphs of a marker.
fn muted(item: &Item) -> bool {
    matches!(item, Item::Glyphs { color, .. } if (color.r, color.g, color.b) == (MUTED.r, MUTED.g, MUTED.b))
}

/// The baseline and left edge of each marker painted.
fn markers(items: &[Item]) -> Vec<(f32, f32)> {
    let origin = |item: &Item| if let Item::Glyphs { origin, .. } = item { Some((origin.y, origin.x)) } else { None };
    items.iter().filter(|item| muted(item)).filter_map(origin).collect()
}

/// How far each glyph of the text sits right of where it was, with its baseline.
fn pushes(hidden: &[Item], shown: &[Item]) -> Vec<(f32, f32)> {
    let (hidden, shown) = (text(hidden), text(shown));
    assert_eq!(hidden.len(), shown.len(), "markers add no text");
    hidden.iter().zip(&shown).map(|(was, is)| (is.2, is.1 - was.1)).collect()
}

#[test]
fn a_pair_of_markers_pushes_the_rest_of_its_own_line() {
    let cases = [("bold", Mark::Bold), ("italic", Mark::Italic), ("strike", Mark::Strike), ("code", Mark::Code)];
    for (name, mark) in cases {
        let mut page = Page::new(&sentence(&[mark]), NARROW);
        let (hidden, height) = (page.painted(), page.layout.height());
        assert!(page.click("a", INSIDE), "{name}: markers appear");
        let shown = page.painted();

        let pair = markers(&shown);
        let [(line, open), (_, close)] = pair[..] else { panic!("{name} shows a pair of markers: {pair:?}") };
        assert_eq!(pair[0].0, pair[1].0, "{name}: on one line");
        assert!(open < close);
        assert_eq!(page.layout.height(), height, "{name}: nothing reflows");

        let (was, is) = (text(&hidden), text(&shown));
        assert_eq!(was.len(), is.len());
        let mut inside = 0.0;
        for ((id, x, y), (now_id, now_x, now_y)) in was.into_iter().zip(is) {
            assert_eq!((id, y), (now_id, now_y), "{name}: every glyph keeps its line");
            let pushed = now_x - x;
            match () {
                _ if y != line => assert_eq!(pushed, 0.0, "{name}: no other line moves"),
                _ if now_x < open => assert_eq!(pushed, 0.0, "{name}: text before the pair stays"),
                _ if now_x < close => {
                    assert!(pushed > 1.0, "{name}: the marked text makes room for the opening marker");
                    inside = pushed;
                }
                _ => assert!(pushed > inside + 1.0, "{name}: text after makes room for both"),
            }
        }
        assert!(inside > 0.0, "{name}: the marked text is on the markers' line");
    }
}

#[test]
fn the_caret_and_clicks_follow_the_pushed_text() {
    let doc = sentence(&[Mark::Bold]);
    let mut page = Page::new(&doc, NARROW);
    let plain = |offset| laid_at(&doc, NARROW).caret(&Pos::new("a", offset)).unwrap();
    page.click("a", INSIDE);
    let layout = &page.layout;
    let caret = |offset| layout.caret(&Pos::new("a", offset)).unwrap();

    let (start, end) = (48, 57);
    assert_eq!(caret(start - 2).x, plain(start - 2).x, "before the pair nothing moves");
    let opening = caret(INSIDE).x - plain(INSIDE).x;
    assert!(opening > 0.0, "the caret inside sits after the opening marker");
    assert!(caret(end + 2).x - plain(end + 2).x > opening, "and text after sits past both");
    assert_eq!(caret(0), plain(0), "other lines keep their carets");

    let text = doc.blocks[0].plain_text();
    for offset in (0..=text.len()).filter(|at| !text[*at..].starts_with(' ')) {
        let at = caret(offset);
        let middle = Point { x: at.x + 0.25, y: at.y + at.height / 2.0 };
        assert_eq!(layout.hit(middle), Some(Pos::new("a", offset)), "a click on the caret at {offset} finds it");
    }
    let [(line, open), _] = markers(&page.painted())[..] else { panic!("a pair") };
    assert_eq!(layout.hit(Point { x: open + 1.0, y: line - 4.0 }), Some(Pos::new("a", start)), "a click on a marker");
}

#[test]
fn stepping_over_a_marker_moves_the_caret_and_not_the_position() {
    let mut page = Page::new(&sentence(&[Mark::Bold]), NARROW);
    page.click("a", 56);
    let mut stops = vec![(page.head().offset, page.layout.caret(&page.head()).unwrap().x)];
    for _ in 0..2 {
        assert!(!page.step(Motion::Right), "the same markers stay up");
        stops.push((page.head().offset, page.layout.caret(&page.head()).unwrap().x));
    }
    let offsets: Vec<usize> = stops.iter().map(|stop| stop.0).collect();
    assert_eq!(offsets, [56, 57, 57], "the closing marker is a stop of its own");
    assert!(stops.windows(2).all(|pair| pair[0].1 < pair[1].1), "and each stop is further right: {stops:?}");
    assert!(page.step(Motion::Right), "past the pair its markers go");
}

#[test]
fn block_markers_sit_beside_the_text_and_move_nothing() {
    let callout = container("c", BlockKind::Callout { tone: Tone::Warn }, vec![paragraph("a", PROSE)]);
    let cases = [
        ("heading", doc(vec![paragraph("p", PROSE), Block::text("a", BlockKind::Heading { level: 2 }, "Rates")])),
        ("callout", doc(vec![paragraph("p", PROSE), callout])),
    ];
    for (name, doc) in cases {
        let mut page = Page::new(&doc, WIDTH);
        let hidden = page.painted();
        assert!(page.click("a", 2), "{name}: its marker appears");
        let shown = page.painted();
        assert_eq!(text(&shown), text(&hidden), "{name}: the text stays where it was");
        assert_eq!(page.layout.caret(&Pos::new("a", 2)), laid(&doc).caret(&Pos::new("a", 2)), "{name}");

        let first = page.layout.caret(&Pos::new("a", 0)).unwrap();
        let shown = markers(&shown);
        assert!(!shown.is_empty(), "{name} shows its marker");
        for (baseline, x) in shown {
            match name {
                "heading" => assert!(x < first.x && baseline > first.y, "it hangs left of the text edge"),
                _ => assert!(x >= first.x && baseline < first.y, "it sits above the first line, inside the box"),
            }
        }
        assert!(page.click("p", 2), "{name}: and goes with the caret");
        assert!(page.painted() == hidden, "{name}: the page is what it was");
    }
}

#[test]
fn a_heading_in_a_box_keeps_its_marker_inside() {
    let cases = [("callout", BlockKind::Callout { tone: Tone::Tip }), ("quote", BlockKind::Quote)];
    for (name, kind) in cases {
        let heading = Block::text("a", BlockKind::Heading { level: 3 }, "Rates change on the first of January");
        let doc = doc(vec![paragraph("p", PROSE), container("c", kind, vec![heading, paragraph("b", PROSE)])]);
        let mut page = Page::new(&doc, NARROW);
        let hidden = page.painted();
        let first = page.layout.caret(&Pos::new("a", 0)).unwrap();
        assert!(page.click("a", 2), "{name}: the heading's marker appears");
        let shown = page.painted();

        let [(line, left)] = markers(&shown)[..] else { panic!("{name}: one marker shows") };
        // The box starts at the page's left edge, and the bar down its side is 3 wide.
        assert!(left > 3.0, "{name}: the marker stays clear of the bar, not at {left}");
        assert!(page.layout.caret(&Pos::new("a", 0)).unwrap().x > first.x, "{name}: its line makes room");
        let moved: Vec<(f32, f32)> = pushes(&hidden, &shown).into_iter().filter(|push| push.1 != 0.0).collect();
        assert!(!moved.is_empty(), "{name}: the heading moves over");
        assert!(moved.iter().all(|push| push.0 == line && push.1 > 0.0), "{name}: and only its first line does");
        let glyphs = text(&shown);
        let heading = glyphs.iter().filter(|glyph| glyph.2 == line).map(|glyph| glyph.1).fold(f32::MAX, f32::min);
        assert!(left < heading, "{name}: the marker comes before the heading");
    }
}

#[test]
fn a_heading_makes_room_for_a_marker_its_margin_cannot_hold() {
    let doc = doc(vec![paragraph("p", PROSE), Block::text("a", BlockKind::Heading { level: 3 }, "Rates")]);
    // The room left of the text edge, and whether the marker fits in it.
    for (margin, fits) in [(200.0, true), (16.0, false), (0.0, false)] {
        let mut page = Page::new(&doc, WIDTH);
        page.layout.set_margin(margin);
        let first = page.layout.caret(&Pos::new("a", 0)).unwrap();
        assert!(page.click("a", 2), "the heading's marker appears");

        let [(_, left)] = markers(&page.painted())[..] else { panic!("one marker shows") };
        assert!(left >= -margin - 0.01, "the marker stays within {margin} px of the text edge, not at {left}");
        let pushed = page.layout.caret(&Pos::new("a", 0)).unwrap().x - first.x;
        assert_eq!(pushed > 0.0, !fits, "with {margin} px to hang in, the first line moves over by {pushed}");
    }
}

#[test]
fn progress_fades_markers_and_slides_the_line() {
    let mut page = Page::new(&sentence(&[Mark::Bold]), NARROW);
    let hidden = page.painted();
    assert!(page.click("a", INSIDE));
    let shown = page.painted();
    let slid = |page: &Page| pushes(&hidden, &page.painted()).iter().map(|push| push.1).fold(0.0, f32::max);
    let alpha = |page: &Page| page.painted().iter().filter(|item| muted(item)).filter_map(color_of).map(|c| c.a).max();
    let full = slid(&page);

    page.layout.set_reveal_progress(0.0);
    assert!(page.painted() == hidden, "at the start nothing shows");
    page.layout.set_reveal_progress(0.5);
    assert!((slid(&page) - full / 2.0).abs() < 0.01, "half way the line has slid half way");
    assert!(alpha(&page) < Some(MUTED.a) && alpha(&page) > Some(0), "and the markers are half faded");
    page.layout.set_reveal_progress(1.0);
    assert!(page.painted() == shown, "at the end they show in full");

    assert!(!page.reveal(), "the same markers again change nothing");
    assert!(page.layout.reveal(None), "hiding them is a change");
    page.layout.set_reveal_progress(0.0);
    assert!(page.painted() == shown, "they fade out from where they were");
    page.layout.set_reveal_progress(1.0);
    assert!(page.painted() == hidden, "and leave the page as it was");
}

#[test]
fn markers_follow_typing_without_fading_again() {
    let mut page = Page::new(&sentence(&[Mark::Bold]), NARROW);
    page.click("a", INSIDE);
    let before = markers(&page.painted());
    let change = page.editor.input(InputEvent::Insert("s".into()), &mut Desk);
    page.layout.update(page.editor.doc(), &Style::default(), NARROW, Some(&change));
    assert!(!page.reveal(), "the pair is the same pair");
    let after = markers(&page.painted());
    assert_eq!(before[0], after[0], "the opening marker stays");
    assert!(after[1].1 > before[1].1, "and the closing one makes room for the letter");
}

#[test]
fn composing_text_is_underlined() {
    let doc = prose(1);
    let mut layout = laid(&doc);
    let hidden = painted(&layout, everything());
    layout.set_composition(Some((&"p0".into(), 6..12)));
    let shown = painted(&layout, everything());
    let added: Vec<&Item> = shown.iter().filter(|item| !hidden.contains(item)).collect();
    let [Item::Fill { rect, .. }] = added[..] else { panic!("one underline is added, not {}", added.len()) };
    let (from, to) = (layout.caret(&Pos::new("p0", 6)).unwrap(), layout.caret(&Pos::new("p0", 12)).unwrap());
    assert!((rect.x - from.x).abs() < 1.0 && (rect.x + rect.width - to.x).abs() < 1.0, "under the composed text");
    assert!(rect.y > from.y + from.height / 2.0 && rect.y < from.y + from.height + 2.0, "below its letters");
    assert!(rect.height >= 1.0 && rect.height <= 3.0);

    layout.set_composition(None);
    assert!(painted(&layout, everything()) == hidden, "and goes when the composition ends");
}

#[test]
fn line_edges_stay_on_their_line() {
    let layout = laid_at(&prose(1), NARROW);
    let at = |offset| Pos::new("p0", offset);
    let caret = |pos: &Pos| layout.caret(pos).unwrap();
    let lines = (layout.height() / caret(&at(0)).height) as usize;
    assert!(lines >= 3, "the prose wraps");
    for offset in [0, 3, 60, 100, PROSE.len() - 2, PROSE.len()] {
        let here = caret(&at(offset));
        let (start, end) =
            (layout.line_edge(&at(offset), false).unwrap(), layout.line_edge(&at(offset), true).unwrap());
        assert_eq!((caret(&start).y, caret(&end).y), (here.y, here.y), "both edges of {offset} are on its line");
        assert!(caret(&start).x <= here.x && here.x <= caret(&end).x);
        assert!(start.offset <= offset && offset <= end.offset);
        assert_eq!(layout.line_edge(&end, true), Some(end.clone()), "End pressed twice stays put");
        assert_eq!(layout.line_edge(&start, false), Some(start), "and so does Home");
    }
    assert_eq!(layout.line_edge(&at(PROSE.len()), true), Some(at(PROSE.len())));
    assert_eq!(layout.line_edge(&Pos::new("missing", 0), true), None);
}

const WORDS: [&str; 8] = ["rates", "change", "on", "the", "first", "of", "January", "household"];
const MARKS: [Mark; 4] = [Mark::Bold, Mark::Italic, Mark::Strike, Mark::Code];

/// A few blocks of marked text: paragraphs and headings, some of them inside callouts.
fn documents() -> impl Strategy<Value = Document> {
    let words = prop::collection::vec(prop::sample::select(WORDS.to_vec()), 1..5);
    let run = (words, prop::sample::subsequence(MARKS.to_vec(), 0..=2))
        .prop_map(|(words, marks)| Run::marked(words.join(" ") + " ", marks));
    let block = (0..5u8, prop::collection::vec(run, 1..6));
    prop::collection::vec(block, 1..4).prop_map(|blocks| {
        let blocks = blocks.into_iter().enumerate().map(|(index, (kind, runs))| {
            let id = format!("b{index}");
            let text = |kind| Block { body: Body::Text(runs.clone()), ..Block::new(id.as_str(), kind) };
            let boxed = |inside| container(&format!("c{index}"), BlockKind::Callout { tone: Tone::Info }, vec![inside]);
            match kind {
                0 => text(BlockKind::Heading { level: 2 }),
                1 => boxed(text(BlockKind::Paragraph)),
                2 => boxed(text(BlockKind::Heading { level: 3 })),
                _ => text(BlockKind::Paragraph),
            }
        });
        doc(blocks.collect())
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Walking the caret through every stop of a document never moves a line break, and hiding the markers
    /// always gives back the same page.
    #[test]
    fn markers_never_change_the_layout(doc in documents(), width in 140.0f32..480.0, progress in 0.0f32..=1.0) {
        let mut page = Page::new(&doc, width);
        let (hidden, height) = (page.painted(), page.layout.height());
        let was = text(&hidden);
        let mut stop = None;
        for _ in 0..600 {
            if page.reveal() {
                page.layout.set_reveal_progress(progress);
            }
            let (head, shown) = (page.head(), page.painted());
            prop_assert_eq!(page.layout.height(), height);
            let held: Vec<f32> = markers(&shown).iter().map(|marker| marker.0).collect();
            let given = page.editor.markers();
            let due = given.inline.len() + given.block.iter().len();
            prop_assert!(held.len() >= due || progress == 0.0, "{} of {:?} show", held.len(), given);
            let is = text(&shown);
            prop_assert_eq!(was.len(), is.len());
            for ((id, x, y), (now_id, now_x, now_y)) in was.iter().zip(&is) {
                prop_assert_eq!((id, y), (now_id, now_y), "a glyph left its line with the caret at {:?}", head);
                prop_assert!(*now_x > x - 0.01, "text is only pushed outward: {} to {} at {:?}", x, now_x, head);
                prop_assert!(now_x == x || held.contains(y), "a line with no marker moved at {:?}", head);
            }
            let caret = page.layout.caret(&head).unwrap();
            let text = page.editor.doc().block(&head.block).unwrap().plain_text();
            if !text[head.offset..].starts_with(' ') {
                let middle = Point { x: caret.x + 0.25, y: caret.y + caret.height / 2.0 };
                prop_assert_eq!(page.layout.hit(middle), Some(head.clone()), "a click on the caret finds it");
            }

            let here = Some((head, page.editor.markers()));
            if std::mem::replace(&mut stop, here.clone()) == here {
                break;
            }
            page.editor.run(Command::Move { to: Motion::Right, extend: false }, &mut Desk);
        }
        prop_assert_eq!(stop.map(|stop| stop.0), page.layout.hit(Point { x: 1.0e6, y: 1.0e6 }), "the walk reached the end");
        page.layout.reveal(None);
        page.layout.set_reveal_progress(1.0);
        prop_assert!(page.painted() == hidden, "with the markers hidden the page is what it was");
    }
}
