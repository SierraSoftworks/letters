//! The page the editor starts in, as a server sends it: the window as it will be, readable before any of
//! the editor has arrived, and the document as the editor reads it.
//!
//! What stands in for the window is the document's export where the canvas will draw the document, and a
//! [`Skeleton`] of how the window's bar starts, the document's name and how saving stands, so that nothing
//! moves when the editor's first frame takes their place. The bar's other controls come with the editor. The export sits in a frame of its own, so that its styles and
//! the page's cannot reach each other and nothing a document carries can run.
//!
//! A server calls [`served`] and sends what it answers. Beside the document such a page carries the
//! server's [`Opened`]: which document it is, who writes in it and what they may do.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::ops::RangeInclusive;

use accesskit::{Node, NodeId, Role};
use letters_api::{Opened, Trust};
use letters_editor::Editor;
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Model, Notes, Pos, ReplicaId, Selection, Style};
use letters_render::Size;
use letters_ui::{Ink, SafeMode, Scheme, Window, tokens};

/// The `id` of what stands in for the window until the editor has painted it.
pub const FIRST_PAINT: &str = "letters-first-paint";
/// The `id` of the script element that holds the document as `document.json` stores it.
pub const DOCUMENT: &str = "letters-document";
/// The `id` of the script element that holds what a server says of the document: its [`Opened`].
pub const OPENED: &str = "letters-opened";

/// Where each goes in a page's template.
const FIRST_PAINT_SLOT: &str = "<!--letters:first-paint-->";
const DOCUMENT_SLOT: &str = "<!--letters:document-->";
const PRELOAD_SLOT: &str = "<!--letters:preload-->";

/// What the Comment tool says of a document whose notes this build cannot read.
pub const NO_COMMENTS: &str = "Comments cannot be added here: this document's notes are in a form this version of \
                               Letters cannot read.";

/// The widths of window a [`Skeleton`] is laid out for. A narrower or wider one carries on from the nearest.
const WIDTHS: RangeInclusive<u16> = 240..=1600;

/// Where a control is in the window: its left, top, width and height.
pub type Place = [f32; 4];

/// The window's controls through a run of window widths in which each keeps to a line: it stays where it
/// is, or follows the window's far edge, or gives way a pixel for a pixel.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// The narrowest window the span is for. It holds up to the next span's.
    pub from: f32,
    /// Each control's place in the narrowest window, and how far each measure moves for a pixel more.
    pub controls: Vec<(Place, Place)>,
}

/// Joins what the window showed at each width into spans. `shown` runs from the narrowest window up.
pub fn spans(shown: &[(f32, Vec<Place>)]) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    // How many widths the last span holds. Its second says how its controls move.
    let mut held = 0;
    for (width, places) in shown {
        let fits = spans.last_mut().filter(|span| span.controls.len() == places.len()).is_some_and(|span| {
            let wider = width - span.from;
            if held == 1 {
                for ((at, moves), place) in span.controls.iter_mut().zip(places) {
                    *moves = std::array::from_fn(|i| (place[i] - at[i]) / wider);
                }
            }
            let on_its_line = |(at, moves): &(Place, Place), place: &Place| {
                (0..4).all(|i| (at[i] + moves[i] * wider - place[i]).abs() < 0.05)
            };
            span.controls.iter().zip(places).all(|(control, place)| on_its_line(control, place))
        });
        if fits {
            held += 1;
        } else {
            spans.push(Span { from: *width, controls: places.iter().map(|place| (*place, [0.0; 4])).collect() });
            held = 1;
        }
    }
    spans
}

/// The start of the window's bar as shapes with nothing in them, for every width of window: the document's
/// name and how saving stands. The controls after them are left out, so that less changes when they come.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    /// How much of the window's top its bar takes. The document has the rest.
    pub bar: f32,
    pub spans: Vec<Span>,
}

impl Skeleton {
    /// Lays the window out at every width a browser is likely to give it, and notes where its bar starts.
    pub fn of(window: &mut Window) -> Self {
        let mut bar = 0.0;
        let mut shown: Vec<(f32, Vec<(NodeId, Place)>)> = Vec::new();
        let mut lead: HashSet<NodeId> = HashSet::new();
        for width in WIDTHS {
            let width = f32::from(width);
            window.resize(Size { width, height: 2.0 * width }, 1.0);
            let update = window.access();
            let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, node)| (*id, node)).collect();
            let mut controls = Vec::new();
            if let Some(tree) = &update.tree {
                gather(&nodes, tree.root, &mut controls);
            }
            let document = nodes.values().find(|node| node.role() == Role::Document);
            bar = document.and_then(|document| document.bounds()).map_or(bar, |bounds| bounds.y0 as f32);
            // The bar starts with the document's name and then how saving stands, the one label it has.
            let ends = controls.iter().position(|(id, _)| nodes[id].role() == Role::Label).map_or(0, |at| at + 1);
            lead.extend(controls[..ends].iter().map(|(id, _)| *id));
            shown.push((width, controls));
        }
        // The controls at the bar's end follow the window's far edge, which the two widest windows tell
        // from the rest. The rest are cut off a gap before those begin, as the bar cuts them.
        let mut at_end: HashSet<NodeId> = HashSet::new();
        if let [.., (narrower, before), (wider, after)] = &shown[..] {
            let follows = |(id, place): &(NodeId, Place)| {
                let was = before.iter().find(|(other, _)| other == id);
                was.is_some_and(|(_, was)| (place[0] - was[0] - (wider - narrower)).abs() < 0.01)
            };
            at_end.extend(after.iter().filter(|control| follows(control)).map(|(id, _)| *id));
        }
        let shown: Vec<(f32, Vec<Place>)> = shown
            .into_iter()
            .map(|(width, controls)| {
                let ends = controls.iter().filter(|(id, _)| at_end.contains(id));
                let edge = ends.map(|(_, place)| place[0]).fold(width, f32::min) - tokens::SPACE_SM;
                let cut = |(id, [left, top, wide, high]): (NodeId, Place)| match at_end.contains(&id) {
                    true => Some([left, top, wide, high]),
                    // A sliver of a control would read as a mark of its own.
                    false => Some([left, top, wide.min(edge - left), high]).filter(|cut| cut[2] >= high / 2.0),
                };
                let mut places: Vec<Place> =
                    controls.into_iter().filter(|(id, _)| lead.contains(id)).filter_map(cut).collect();
                places.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
                (width, places)
            })
            .collect();
        Self { bar, spans: spans(&shown) }
    }

    /// The most controls any width of window shows.
    fn controls(&self) -> usize {
        self.spans.iter().map(|span| span.controls.len()).max().unwrap_or(0)
    }

    /// The rules that put each shape where its control will be, for the window's width.
    fn css(&self, css: &mut String) {
        for (i, span) in self.spans.iter().enumerate() {
            // The first span also serves a window narrower than any that was laid out, and the last a wider.
            let from = (i > 0).then(|| format!("(width >= {}px)", number(span.from)));
            let to = self.spans.get(i + 1).map(|next| format!("(width < {}px)", number(next.from)));
            let media = match (from, to) {
                (Some(from), Some(to)) => format!("{from} and {to}"),
                (Some(only), None) | (None, Some(only)) => only,
                (None, None) => "all".into(),
            };
            let _ = writeln!(css, "@media {media} {{");
            for (n, (at, moves)) in span.controls.iter().enumerate() {
                let measure = |i: usize| match moves[i].abs() < 1e-4 {
                    true => format!("{}px", number(at[i])),
                    false => {
                        format!("calc({}px + {}vw)", number(at[i] - moves[i] * span.from), number(moves[i] * 100.0))
                    }
                };
                let (left, top, width, height) = (measure(0), measure(1), measure(2), measure(3));
                let _ = writeln!(
                    css,
                    "#{FIRST_PAINT} i:nth-child({}) {{ display: block; left: {left}; top: {top}; width: {width}; \
                     height: {height}; }}",
                    n + 1
                );
            }
            css.push_str("}\n");
        }
    }
}

/// Adds the controls under `id` to `out`: the nodes that say where they are and hold no control of their
/// own. The document is not the window's to stand in for, and a run of text is its control's.
fn gather(nodes: &HashMap<NodeId, &Node>, id: NodeId, out: &mut Vec<(NodeId, Place)>) -> bool {
    let Some(node) = nodes.get(&id).filter(|node| !matches!(node.role(), Role::Document | Role::TextRun)) else {
        return false;
    };
    let mut holds = false;
    for child in node.children() {
        holds |= gather(nodes, *child, out);
    }
    if let Some(bounds) = node.bounds().filter(|_| !holds) {
        out.push((id, [bounds.x0 as f32, bounds.y0 as f32, bounds.width() as f32, bounds.height() as f32]));
        holds = true;
    }
    holds
}

/// A measure as CSS writes it: to a hundredth, with no noughts trailing.
fn number(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded == 0.0 { "0".into() } else { rounded.to_string() }
}

fn hex(ink: Ink, scheme: Scheme) -> String {
    let ink = ink.resolve(scheme);
    format!("#{:02x}{:02x}{:02x}", ink.r, ink.g, ink.b)
}

#[cfg(not(target_arch = "wasm32"))]
pub use server::{export, served};

/// What a server needs to make a page and the editor's own code does without.
#[cfg(not(target_arch = "wasm32"))]
mod server {
    use letters_api::Opened;
    use letters_convert::{Column, HtmlPage, WebFont};
    use letters_fonts::{DISPLAY, MONO, SANS};
    use letters_model::{Document, ReplicaId, Style};
    use letters_ui::{Scheme, palette};

    use super::{OPENED, Skeleton, compose, first_paint, preloads, script, served_window};

    /// The page every document is sent in.
    const TEMPLATE: &str = include_str!("../www/index.html");
    /// Where a page says where its files are, what it is called, and what the server says of its document.
    const BASE_SLOT: &str = "<!--letters:base-->";
    const TITLE: &str = "<title>Letters</title>";
    const OPENED_SLOT: &str = "<!--letters:opened-->";

    /// The whole page a server sends for `document`: it reads at once, and the editor opens it as `opened`
    /// says, with no request of its own. `base` is where the server keeps what the browser build made, a
    /// URL that ends in a slash.
    ///
    /// The page runs one script, a file under `base`, so a policy that lets no inline script run suits it.
    pub fn served(document: &Document, opened: &Opened, base: &str) -> String {
        // The window as the editor will open it.
        let mut window = served_window(document.clone(), opened, ReplicaId::new("page"));
        let style = window.style().clone();
        let html = export(document, &style, &opened.name, &format!("{base}fonts/"));
        let first = first_paint(&Skeleton::of(&mut window), &html);
        let said = serde_json::to_string(opened).expect("what a server says of a document is JSON");
        let text = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
        let template = TEMPLATE
            .replacen(BASE_SLOT, &format!("<base href=\"{}\" />", text(base)), 1)
            .replacen(TITLE, &format!("<title>{}</title>", text(&opened.name)), 1)
            .replacen(OPENED_SLOT, &script(OPENED, &said), 1);
        compose(&template, &preloads(document, base), &first, &document.to_json())
    }

    /// The window's own measures for the column a document is set in: the least room it keeps beside the
    /// column, and the room above and below it. The browser tests hold the two to each other.
    const GUTTER: f32 = 24.0;
    const TOP: f32 = 48.0;

    /// The fonts as a page names them: the family, its file among [`letters_fonts::FILES`], and whether it
    /// is the family's italic.
    const FACES: [(&str, &str, bool); 5] = [
        (SANS, "Inter.ttf", false),
        (SANS, "Inter-Italic.ttf", true),
        (MONO, "FiraCode.ttf", false),
        (DISPLAY, "Montserrat.ttf", false),
        (DISPLAY, "Montserrat-Italic.ttf", true),
    ];

    /// The document as HTML for the first paint to show: its export, set where the window sets the document,
    /// in the fonts the window draws with and in the window's colours. `fonts` is where the page fetches the
    /// fonts from, a URL that ends in a slash.
    ///
    /// A server makes it, so the editor's own code does not carry the export.
    pub fn export(document: &Document, style: &Style, title: &str, fonts: &str) -> String {
        let urls = FACES.map(|(_, file, _)| format!("{fonts}{file}"));
        let faces: Vec<WebFont> = FACES
            .iter()
            .zip(&urls)
            .map(|((family, _, italic), url)| WebFont { family, url, italic: *italic })
            .collect();
        let page = HtmlPage {
            dark: Some(&palette(Scheme::Dark)),
            fonts: &faces,
            window: Some(Column { gutter: GUTTER, top: TOP }),
            // The frame scrolls as the canvas will, with no bar of its own, and is as busy as the page around it.
            css: "html { scrollbar-width: none; cursor: progress; }\n",
        };
        letters_convert::to_html_with(document, style, title, &mut |_| None, &page)
    }
}

/// What stands in for the window until the editor has painted it: `skeleton` for how its bar starts, and
/// `export`, what [`export`] made of the document.
///
/// Nothing the document holds can end the frame early or run in it.
pub fn first_paint(skeleton: &Skeleton, export: &str) -> String {
    let (light, dark) = (Scheme::Light, Scheme::Dark);
    let bar = number(skeleton.bar);
    let mut css = format!(
        "#{FIRST_PAINT} {{ position: fixed; inset: 0; cursor: progress; --bar: {}; --line: {}; --shape: {}; }}\n\
         @media (prefers-color-scheme: dark) {{ #{FIRST_PAINT} {{ --bar: {}; --line: {}; --shape: {}; }} }}\n\
         #{FIRST_PAINT} div {{ position: absolute; inset: 0 0 auto; height: {bar}px; box-sizing: border-box; \
         overflow: hidden; background: var(--bar); border-bottom: 1px solid var(--line); }}\n\
         #{FIRST_PAINT} i {{ position: absolute; display: none; border-radius: {radius}px; \
         background: var(--shape); animation: {FIRST_PAINT} 0.9s ease-in-out infinite alternate; }}\n\
         @keyframes {FIRST_PAINT} {{ to {{ opacity: 0.4; }} }}\n\
         @media (prefers-reduced-motion: reduce) {{ #{FIRST_PAINT} i {{ animation: none; }} }}\n\
         #{FIRST_PAINT} iframe {{ position: absolute; left: 0; top: {bar}px; width: 100%; \
         height: calc(100% - {bar}px); border: 0; }}\n",
        hex(tokens::SURFACE, light),
        hex(tokens::LINE, light),
        hex(tokens::LINE, light),
        hex(tokens::SURFACE, dark),
        hex(tokens::LINE, dark),
        hex(tokens::LINE, dark),
        radius = number(tokens::RADIUS_CONTROL),
    );
    skeleton.css(&mut css);

    let mut out = format!("<div id=\"{FIRST_PAINT}\"><style>\n{css}</style>");
    let _ = write!(
        out,
        "<div role=\"status\" aria-label=\"Loading the editor\">{}</div>",
        "<i></i>".repeat(skeleton.controls())
    );
    // The frame may run nothing. It is the page's own all the same, so that it can fetch the page's
    // fonts and the editor can read how far it was scrolled.
    out.push_str("<iframe title=\"Document\" sandbox=\"allow-same-origin\" srcdoc=\"");
    for c in export.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out.push_str("\"></iframe></div>");
    out
}

/// Whether the first frame of `document` needs the maths faces: it holds a formula, or maths in a line.
pub fn has_maths(document: &Document) -> bool {
    let inline = |block: &Block| match &block.body {
        Body::Text(runs) => runs.iter().any(|run| run.marks.contains(&Mark::Math)),
        _ => false,
    };
    document.walk().any(|block| block.kind == BlockKind::Formula || inline(block))
}

/// What a page asks its browser to fetch early for `document`, beyond what every page does: the maths
/// faces, when its first frame waits for them. `base` is where the files are found, as the editor is given it.
pub fn preloads(document: &Document, base: &str) -> String {
    let files = letters_math::FONT_FILES.iter().filter(|_| has_maths(document));
    files.map(|file| format!("<link rel=\"preload\" as=\"fetch\" crossorigin href=\"{base}maths/{file}\" />")).collect()
}

/// Fills a page's template with a document: `preloads` and `first_paint` are what [`preloads`] and
/// [`first_paint`] made of it, and `json` its `document.json`, which cannot end its element early
/// whatever the document holds.
pub fn compose(template: &str, preloads: &str, first_paint: &str, json: &str) -> String {
    // The script goes in first: it holds no `<`, so it cannot be mistaken for another's place, while the
    // first paint may hold anything.
    let page = template.replacen(DOCUMENT_SLOT, &script(DOCUMENT, json), 1).replacen(PRELOAD_SLOT, preloads, 1);
    page.replacen(FIRST_PAINT_SLOT, first_paint, 1)
}

/// JSON for the editor to read from the page, in an element that nothing it holds can end early.
fn script(id: &str, json: &str) -> String {
    // Inside a script, `<` could begin the tag that ends it. JSON reads the escape as the same character.
    format!("<script type=\"application/json\" id=\"{id}\">{}</script>", json.replace('<', "\\u003c"))
}

/// The window the editor opens on `document`: the caret waits at its end. A page's [`Skeleton`] is taken
/// from the same window, so that it is the one the editor's first frame shows.
pub fn window(document: Document, replica: ReplicaId) -> Window {
    at_end(Window::new(Editor::new(document, replica)))
}

/// The window the editor opens on a document a server sent, as `opened` says of it: in the document's
/// style and under its name, with its notes, signing edits as the session does, and held in safe mode
/// until the person trusts it. [`served`] takes the page's [`Skeleton`] from the same window.
pub fn served_window(document: Document, opened: &Opened, replica: ReplicaId) -> Window {
    // The built-in style stands in for one this build cannot read, as it does on the desktop.
    let style = opened.style.as_ref().and_then(|style| Style::from_json(&style.to_string()).ok());
    let model = Model::new(document, replica).with_style(style.unwrap_or_default());
    let mut window = at_end(Window::new(Editor::with_model(model)));
    window.set_document_name(&opened.name);
    window.set_author(&opened.author);
    match opened.notes.as_ref().map(|notes| Notes::from_json(&notes.to_string())) {
        Some(Ok(notes)) => window.set_notes(notes),
        // A comment added to notes that cannot be read could not be kept.
        Some(Err(_)) => window.refuse_comments(Some(NO_COMMENTS)),
        None => {}
    }
    if opened.trust == Trust::Safe {
        window.set_safe_mode(Some(SafeMode { folder: None }));
    }
    window
}

/// Puts the caret at the document's end, where the person writes on.
fn at_end(mut window: Window) -> Window {
    let end = window.editor().doc().blocks.last().map(|block| Pos::new(block.id.clone(), usize::MAX));
    if let Some(end) = end {
        window.select(Selection::caret(end));
    }
    window
}

/// What a page shows when it was given no document of its own.
pub fn welcome(new_id: &mut dyn FnMut() -> String) -> Document {
    let mut block = |kind, text| Block::text(BlockId::new(new_id()), kind, text);
    Document {
        blocks: vec![
            block(BlockKind::Heading { level: 1 }, "Letters in the browser"),
            block(BlockKind::Paragraph, "The same engine as the desktop app, painted on a canvas. Type to try it."),
            block(BlockKind::Paragraph, ""),
        ],
        ..Document::default()
    }
}
