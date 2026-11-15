//! How long layout takes on a 500-page document, against the budgets in the design.
//!
//! Run with `cargo bench -p letters-layout`. It exits with an error when a budget is broken. The budgets are
//! for the whole app on the reference machine, an M1 Pro, so layout should use a small part of each.

use std::time::{Duration, Instant};

use letters_layout::{DocLayout, Fonts};
use letters_model::{
    Block, BlockId, BlockKind, Body, Document, ListKind, Mark, Markers, Model, Op, Page, Pos, ReplicaId, Run, Style,
    Tone,
};
use letters_render::{DisplayList, Rect};

/// From launch to the first painted document.
const OPEN: Duration = Duration::from_millis(150);
/// From a key press to its paint.
const KEYSTROKE: Duration = Duration::from_millis(8);
/// One frame at 120 Hz.
const FRAME: Duration = Duration::from_micros(8_333);

const PAGES: usize = 500;
/// A page of dense prose holds about this many bytes.
const PAGE: usize = 3_000;
const WIDTH: f32 = 680.0;
const SCREEN: f32 = 900.0;
/// What the shell would settle in one idle slice.
const SLICE: usize = 8 * 1024;

const WORDS: &[&str] = &[
    "rates",
    "change",
    "on",
    "the",
    "first",
    "of",
    "January",
    "and",
    "every",
    "household",
    "scheme",
    "should",
    "check",
    "notice",
    "that",
    "arrives",
    "with",
    "December",
    "statement",
    "before",
    "new",
    "figures",
    "take",
    "effect",
    "a",
    "is",
    "to",
    "in",
    "payment",
    "quarterly",
    "assessment",
    "council",
    "property",
    "valuation",
    "appeal",
    "within",
    "twenty-eight",
    "days",
    "responsibility",
    "it",
    "for",
    "an",
    "increase",
    "résumé",
];

/// Formulas as writing has them, in a line or as a block.
const FORMULAS: &[&str] = &[
    "x^2",
    r"\alpha + \beta",
    r"\frac{a}{b}",
    r"e^{i\pi} + 1 = 0",
    r"\sqrt{x^2 + y^2}",
    r"\sum_{i=0}^{n} \frac{1}{i^2}",
    r"\int_0^\infty e^{-x^2} \, dx = \frac{\sqrt{\pi}}{2}",
];

/// A small repeatable generator, so every run lays out the same document.
struct Dice {
    state: u64,
    /// The bytes of text made so far.
    made: usize,
}

impl Dice {
    fn roll(&mut self, sides: usize) -> usize {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        (self.state % sides as u64) as usize
    }

    /// Between `least` and `least + spread` bytes of words.
    fn words(&mut self, least: usize, spread: usize) -> String {
        let bytes = least + self.roll(spread.max(1));
        let mut text = String::with_capacity(bytes + 16);
        while text.len() < bytes {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(WORDS[self.roll(WORDS.len())]);
        }
        self.made += text.len();
        text
    }

    /// Prose with a few marked stretches, as real writing has.
    fn runs(&mut self, least: usize, spread: usize) -> Body {
        let end = self.made + least + self.roll(spread);
        let mut runs = Vec::new();
        while self.made < end {
            runs.push(Run::plain(self.words(40, 200) + " "));
            let mark = [Mark::Bold, Mark::Italic, Mark::Code, Mark::Highlight][self.roll(4)].clone();
            runs.push(Run::marked(self.words(4, 20) + " ", [mark]));
            if self.roll(8) == 0 {
                runs.push(Run::marked(FORMULAS[self.roll(FORMULAS.len())], [Mark::Math]));
                runs.push(Run::plain(" "));
            }
        }
        Body::Text(runs)
    }
}

/// The document, and how many bytes of text it holds.
fn document() -> (Document, usize) {
    let mut dice = Dice { state: 0x9E37_79B9_7F4A_7C15, made: 0 };
    let (mut blocks, mut next) = (Vec::new(), 0);
    let mut id = || {
        next += 1;
        BlockId::new(format!("b{next}"))
    };
    while dice.made < PAGES * PAGE {
        let level = 1 + dice.roll(3) as u8;
        blocks.push(Block::text(id(), BlockKind::Heading { level }, dice.words(10, 30)));
        for _ in 0..3 + dice.roll(4) {
            blocks.push(Block { body: dice.runs(150, 600), ..Block::new(id(), BlockKind::Paragraph) });
        }
        match dice.roll(5) {
            0 => {
                let list = [ListKind::Bullet, ListKind::Ordered, ListKind::Task { checked: false }][dice.roll(3)];
                for _ in 0..3 + dice.roll(5) {
                    let kind = BlockKind::ListItem { list, depth: dice.roll(3) as u8 };
                    blocks.push(Block::text(id(), kind, dice.words(40, 120)));
                }
            }
            1 => {
                let lines: Vec<String> = (0..5 + dice.roll(12)).map(|_| dice.words(20, 50)).collect();
                blocks.push(Block::text(id(), BlockKind::Code { language: None }, lines.join("\n")));
            }
            2 => {
                let kind = if dice.roll(2) == 0 { BlockKind::Quote } else { BlockKind::callout(Tone::Info) };
                let inside = Block::text(id(), BlockKind::Paragraph, dice.words(200, 300));
                blocks.push(Block { body: Body::Blocks(vec![inside]), ..Block::new(id(), kind) });
            }
            3 => blocks.push(Block::new(id(), BlockKind::Divider)),
            _ => blocks.push(Block::text(id(), BlockKind::Formula, FORMULAS[dice.roll(FORMULAS.len())])),
        }
    }
    (Document { blocks, ..Document::default() }, dice.made)
}

/// Sorted timings of one measurement.
struct Times(Vec<Duration>);

impl Times {
    fn of(runs: usize, mut run: impl FnMut(usize) -> Duration) -> Self {
        let mut times: Vec<_> = (0..runs).map(&mut run).collect();
        times.sort_unstable();
        Self(times)
    }

    fn median(&self) -> Duration {
        self.0[self.0.len() / 2]
    }

    fn worst(&self) -> Duration {
        self.0[self.0.len() - 1]
    }

    fn total(&self) -> Duration {
        self.0.iter().sum()
    }
}

fn ms(time: Duration) -> f64 {
    time.as_secs_f64() * 1000.0
}

fn timed(run: impl FnOnce()) -> Duration {
    let start = Instant::now();
    run();
    start.elapsed()
}

fn screen(y: f32) -> Rect {
    Rect::new(0.0, y, WIDTH, SCREEN)
}

fn frame(layout: &mut DocLayout, y: f32, list: &mut DisplayList) {
    list.items.clear();
    layout.settle(screen(y), 0);
    layout.paint(screen(y), list);
}

fn opened(doc: &Document, style: &Style) -> DocLayout {
    opened_at(doc, style, WIDTH)
}

fn opened_at(doc: &Document, style: &Style, width: f32) -> DocLayout {
    let mut layout = DocLayout::new(Fonts { faces: letters_fonts::bundled() });
    layout.update(doc, style, width, None);
    layout
}

fn main() {
    let (doc, bytes) = document();
    let style = Style::default();
    let mut list = DisplayList::default();
    let mut broken = Vec::new();
    let mut report = |name: &str, times: &Times, measured: Duration, budget: Option<Duration>| {
        let budget = budget.map_or(String::new(), |budget| {
            if measured > budget {
                broken.push(name.to_owned());
            }
            format!("  budget {:.1} ms", ms(budget))
        });
        println!("{name:<15} median {:>8.3} ms  worst {:>8.3} ms{budget}", ms(times.median()), ms(times.worst()));
    };
    let megabytes = bytes as f64 / 1e6;
    println!("{PAGES} pages: {megabytes:.2} MB of text in {} blocks, {WIDTH} px wide", doc.walk().count());

    // Fonts, the first layout and the first screen. The worst run is the cold one.
    let open = Times::of(15, |_| {
        timed(|| {
            let mut layout = opened(&doc, &style);
            frame(&mut layout, 0.0, &mut list);
            assert!(list.items.len() > 10 && layout.height() > SCREEN);
        })
    });
    report("open", &open, open.worst(), Some(OPEN));

    // A new width drops every layout, as a window resize does on each frame.
    let mut layout = opened(&doc, &style);
    let resize = Times::of(30, |run| {
        timed(|| {
            layout.update(&doc, &style, WIDTH - 1.0 - run as f32, None);
            frame(&mut layout, 0.0, &mut list);
        })
    });
    report("resize", &resize, resize.worst(), Some(FRAME));

    // The far end of a document nothing has been laid out in.
    let jump = Times::of(15, |_| {
        let mut layout = opened(&doc, &style);
        let end = layout.height() - SCREEN;
        timed(|| frame(&mut layout, end, &mut list))
    });
    report("jump to end", &jump, jump.worst(), Some(FRAME));

    // Scrolling down a fresh document, 40 px a frame and then a screen a frame.
    for (name, step) in [("scroll", 40.0), ("page down", SCREEN)] {
        let mut layout = opened(&doc, &style);
        let frames = (((layout.height() - SCREEN) / step) as usize).min(1500);
        let scroll = Times::of(frames, |at| timed(|| frame(&mut layout, at as f32 * step, &mut list)));
        report(name, &scroll, scroll.worst(), Some(FRAME));
    }

    // Typing into a long paragraph on screen: the layout's part of a key press.
    let target = doc.blocks.iter().find(|block| block.plain_text().len() > 600).expect("a long paragraph");
    let (id, y) = (target.id.clone(), opened(&doc, &style).caret(&Pos::new(target.id.clone(), 0)).expect("a caret").y);
    let mut model = Model::new(doc.clone(), ReplicaId::from(1));
    let mut layout = opened(model.doc(), &style);
    frame(&mut layout, y, &mut list);
    let typing = Times::of(300, |at| {
        let change =
            model.apply([Op::InsertText { at: Pos::new(id.clone(), 40 + at), text: "x".into() }]).expect("typed");
        timed(|| {
            layout.update(model.doc(), &style, WIDTH, Some(&change));
            frame(&mut layout, y, &mut list);
            assert!(layout.caret(&Pos::new(id.clone(), 41 + at)).is_some());
        })
    });
    report("keystroke", &typing, typing.worst(), Some(KEYSTROKE));

    // A new paragraph changes the structure, so the column is rebuilt around what it keeps.
    let split = Times::of(100, |at| {
        let new_id = BlockId::new(format!("split{at}"));
        let change = model.apply([Op::Split { at: Pos::new(id.clone(), 20), new_id }]).expect("split");
        timed(|| {
            layout.update(model.doc(), &style, WIDTH, Some(&change));
            frame(&mut layout, y, &mut list);
        })
    });
    report("enter", &split, split.worst(), Some(KEYSTROKE));

    // Typing in the source of a formula in a line, which shows in its place while the caret is in it.
    let formula = |block: &&Block| block.body.runs().iter().any(|run| run.marks.contains(&Mark::Math));
    let target = doc.blocks.iter().find(formula).expect("a paragraph with a formula");
    let before = target.body.runs().iter().take_while(|run| !run.marks.contains(&Mark::Math));
    let (id, start) = (target.id.clone(), before.map(|run| run.text.len()).sum::<usize>());
    let y = opened(&doc, &style).caret(&Pos::new(id.clone(), start)).expect("a caret").y;
    let mut model = Model::new(doc.clone(), ReplicaId::from(1));
    let mut layout = opened(model.doc(), &style);
    layout.reveal(Some((&Pos::new(id.clone(), start + 1), &Markers::default())));
    frame(&mut layout, y, &mut list);
    let source = Times::of(200, |at| {
        let caret = Pos::new(id.clone(), start + 1 + at);
        let change = model.apply([Op::InsertText { at: caret.clone(), text: "x".into() }]).expect("typed");
        timed(|| {
            layout.update(model.doc(), &style, WIDTH, Some(&change));
            layout.reveal(Some((&Pos::new(id.clone(), caret.offset + 1), &Markers::default())));
            frame(&mut layout, y, &mut list);
        })
    });
    report("formula source", &source, source.worst(), Some(KEYSTROKE));

    // Typing in a formula that stays typeset, so each key sets a formula never seen before.
    let target = doc.blocks.iter().find(|block| block.kind == BlockKind::Formula).expect("a formula block");
    let (id, y) = (target.id.clone(), opened(&doc, &style).caret(&Pos::new(target.id.clone(), 0)).expect("a caret").y);
    let mut layout = opened(model.doc(), &style);
    frame(&mut layout, y, &mut list);
    let typeset = Times::of(200, |at| {
        let text = if at % 2 == 0 { "+" } else { "y" };
        let change = model.apply([Op::InsertText { at: Pos::new(id.clone(), at), text: text.into() }]).expect("typed");
        timed(|| {
            layout.update(model.doc(), &style, WIDTH, Some(&change));
            frame(&mut layout, y, &mut list);
        })
    });
    report("formula typeset", &typeset, typeset.worst(), Some(KEYSTROKE));

    // Replacing every estimate, in the slices the shell runs while idle.
    let mut layout = opened(&doc, &style);
    let estimate = layout.height();
    let mut slices = Vec::new();
    loop {
        let mut more = false;
        slices.push(timed(|| more = layout.settle(screen(0.0), SLICE)));
        if !more {
            break;
        }
    }
    let settle = Times::of(slices.len(), |at| slices[at]);
    report("settle slice", &settle, settle.worst(), Some(FRAME));
    println!("settle all     {:.0} ms in {} slices of {SLICE} bytes", ms(settle.total()), slices.len());
    println!("height         {:.0} px, first estimated as {estimate:.0} px", layout.height());

    // Print view: the column at the page's text width, split into sheets. The first split settles the
    // whole document, as the idle slices above would have by the time anyone asks for pages.
    let page = Page::default();
    let column = (page.width - page.margin_left - page.margin_right) * 96.0 / 72.0;
    let mut model = Model::new(doc.clone(), ReplicaId::from(1));
    let mut layout = opened_at(model.doc(), &style, column);
    let mut pages = layout.paginate(&page);
    let paginate = Times::of(50, |_| timed(|| pages = layout.paginate(&page)));
    report("paginate", &paginate, paginate.worst(), Some(FRAME));
    println!("sheets         {} of {} by {} pt", pages.sheets.len(), page.width, page.height);

    // A page break put before a block in the middle: the column is rebuilt and split again.
    let middle = model.doc().blocks.len() / 2;
    let breaking = Times::of(50, |at| {
        let mut block = model.doc().blocks[middle + at].clone();
        block.page_break = true;
        let ops =
            [Op::RemoveBlock { block: block.id.clone() }, Op::InsertBlock { parent: None, index: middle + at, block }];
        let change = model.apply(ops).expect("a page break");
        let sheets = pages.sheets.len();
        timed(|| {
            layout.update(model.doc(), &style, column, Some(&change));
            pages = layout.paginate(&page);
            assert!(pages.sheets.len() >= sheets);
        })
    });
    report("page break", &breaking, breaking.worst(), Some(KEYSTROKE));

    // One sheet a frame, as scrolling Print view or writing a PDF asks for them.
    let step = (pages.sheets.len() / 200).max(1);
    let sheet = Times::of(pages.sheets.len() / step, |at| {
        list.items.clear();
        timed(|| layout.paint_page(&pages, at * step, &mut list))
    });
    report("paint a sheet", &sheet, sheet.worst(), Some(FRAME));

    if !broken.is_empty() {
        eprintln!("over budget: {}", broken.join(", "));
        std::process::exit(1);
    }
}
