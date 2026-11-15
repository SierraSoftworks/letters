//! Edits a script makes to whatever a document holds when their turn comes, and the documents it starts on.

use std::collections::BTreeSet;
use std::ops::Range;

use letters_model::{
    Align, Block, BlockId, BlockKind, BodyShape, Document, ListKind, Mark, Model, Op, Page, Pos, Properties,
    PropertiesFormat, Run, Style, Tone,
};

use crate::input::Script;

/// A document with every kind of block, as letters-format's corpus holds it.
pub fn sample() -> Document {
    let json = include_str!("../../crates/letters-format/corpus/document/notice.json");
    Document::from_json(json).expect("the corpus holds a document")
}

/// Everything a replica shows.
pub type Shown = (Document, Style);

pub fn shown(model: &Model) -> Shown {
    (model.doc().clone(), model.style().clone())
}

/// What a replica that only ever read this one's history shows.
pub fn reread(model: &Model) -> Shown {
    shown(&Model::load(&model.save(), 99.into()).expect("a saved history loads"))
}

pub fn unique_ids(doc: &Document) -> bool {
    let mut ids = BTreeSet::new();
    doc.walk().all(|block| ids.insert(&block.id))
}

/// Short text with characters of every width, and the one the history marks blocks with.
fn word(script: &mut Script) -> String {
    const PIECES: [&str; 12] = ["a", "b", " ", "é", "字", "🙂", "\n", "\u{1e}", "$", "*", "Figure 1", "👩‍👧"];
    (0..script.pick(4)).map(|_| script.one(&PIECES)).collect()
}

fn lang(script: &mut Script) -> Option<String> {
    script.one(&[None, Some("fr"), Some("de-CH")]).map(str::to_owned)
}

fn picture(asset: Option<&str>, src: Option<&str>, width: f32) -> BlockKind {
    let (numbered, asset, src) = (src.is_some(), asset.map(str::to_owned), src.map(str::to_owned));
    BlockKind::Figure { asset, src, alt: "é".into(), width, height: 90.5, numbered }
}

fn kinds(shape: BodyShape) -> Vec<BlockKind> {
    match shape {
        BodyShape::Text => vec![
            BlockKind::Paragraph,
            BlockKind::Heading { level: 2 },
            BlockKind::ListItem { list: ListKind::Task { checked: true }, depth: 1 },
            BlockKind::ListItem { list: ListKind::Bullet, depth: 0 },
            BlockKind::Code { language: Some("rust".into()) },
            BlockKind::Formula,
            BlockKind::Caption,
            BlockKind::TableCell,
            picture(Some("assets/c.jpg"), None, 200.0),
            picture(None, Some("https://example.com/d.png"), 33.25),
            // No file would hold this one, so no edit may make it.
            picture(None, None, 10.0),
        ],
        BodyShape::Blocks => vec![
            BlockKind::Quote,
            BlockKind::callout(Tone::Warn),
            BlockKind::Gallery { numbered: true },
            BlockKind::Table { columns: vec![Align::Center], header: false },
            BlockKind::TableRow,
        ],
        BodyShape::None => vec![BlockKind::Divider],
    }
}

/// A stretch of up to `max` characters of a block's text, in bytes.
fn span(script: &mut Script, block: &Block, max: usize) -> Range<usize> {
    let text = block.plain_text();
    let mut starts: Vec<usize> = text.char_indices().map(|(at, _)| at).collect();
    starts.push(text.len());
    let start = script.pick(starts.len());
    starts[start]..starts[(start + script.pick(max + 1)).min(starts.len() - 1)]
}

fn at(script: &mut Script, block: &Block) -> Pos {
    Pos::new(block.id.clone(), span(script, block, 0).start)
}

/// The next edit of a script, fitted to what `doc` holds. `None` when there is nothing for it to act on.
/// New blocks draw on few enough ids that two writers sometimes pick the same one.
pub fn op(script: &mut Script, (doc, style): &Shown) -> Option<Op> {
    let all: Vec<&Block> = doc.walk().collect();
    let texts: Vec<&Block> = all.iter().copied().filter(|block| block.kind.shape() == BodyShape::Text).collect();
    let pick = |script: &mut Script, from: &[&Block]| (!from.is_empty()).then(|| from[script.pick(from.len())].clone());
    let new = |script: &mut Script| BlockId::new(format!("n{}", script.pick(24)));
    Some(match script.pick(32) {
        0..=7 => {
            let block = pick(script, &texts)?;
            Op::InsertText { at: at(script, &block), text: word(script) }
        }
        8..=11 => {
            let block = pick(script, &texts)?;
            Op::DeleteText { range: span(script, &block, 3), block: block.id }
        }
        12..=13 => {
            let block = pick(script, &texts)?;
            let marks = [
                Mark::Bold,
                Mark::Italic,
                Mark::Math,
                Mark::Code,
                Mark::Other("x:y%".into()),
                Mark::Ref(all[script.pick(all.len())].id.clone()),
                Mark::Ref("gone".into()),
            ];
            Op::SetMark { range: span(script, &block, 5), block: block.id, mark: script.one(&marks), on: script.flag() }
        }
        14 => {
            let block = pick(script, &texts)?;
            let link = script.one(&[None, Some("https://example.com/a")]).map(str::to_owned);
            Op::SetLink { range: span(script, &block, 5), block: block.id, link }
        }
        15 => {
            let block = pick(script, &texts)?;
            Op::SetLang { range: span(script, &block, 5), block: block.id, lang: lang(script) }
        }
        16 => Op::SetBlockProps { block: pick(script, &all)?.id, page_break: script.flag(), lang: lang(script) },
        17..=19 => {
            let block = pick(script, &texts)?;
            Op::Split { at: at(script, &block), new_id: new(script) }
        }
        20..=22 => Op::Join { block: pick(script, &texts)?.id },
        23 => {
            let block = pick(script, &all)?;
            Op::SetKind { block: block.id, kind: script.one(&kinds(block.kind.shape())) }
        }
        24..=25 => {
            let containers: Vec<&Block> =
                all.iter().copied().filter(|block| block.kind.shape() == BodyShape::Blocks).collect();
            let parent = if script.flag() { pick(script, &containers) } else { None };
            // Mostly a kind its parent takes, so that tables and galleries fill up. Now and then any kind.
            let mut kinds: Vec<BlockKind> = [BodyShape::Text, BodyShape::Blocks, BodyShape::None].map(kinds).concat();
            if script.pick(8) > 0 {
                kinds.retain(|kind| parent.as_ref().is_none_or(|parent| parent.kind.admits(kind)));
            }
            let mut block = Block::new(new(script), script.one(&kinds));
            if let letters_model::Body::Text(runs) = &mut block.body {
                runs.push(Run::marked(word(script), [Mark::Italic]));
            }
            let siblings = parent.as_ref().map_or(doc.blocks.len(), |parent| parent.body.blocks().len());
            Op::InsertBlock { parent: parent.map(|parent| parent.id), index: script.pick(siblings + 1), block }
        }
        26..=27 => Op::RemoveBlock { block: pick(script, &all)?.id },
        28 => {
            let element = script.one(&["body", "heading1", "aside"]);
            let mut text = style.types.get(element).cloned().unwrap_or_default();
            match script.pick(3) {
                // The last size is one no style may hold.
                0 => text.size = script.one(&[9.0, 12.5, 30.0, 0.0]),
                1 => text.weight = script.one(&[400, 700]),
                _ => text.color = script.one(&["#000000", "#ff0000"]).into(),
            }
            Op::SetStyle { element: element.into(), style: text }
        }
        // Either is a page on its own. A narrow page with a wide margin, as a merge can give, is none.
        29 => Op::SetPage {
            page: match script.flag() {
                true => Page { width: script.one(&[300.0, 595.0]), ..style.page.clone() },
                false => Page { margin_left: script.one(&[71.0, 250.0]), ..style.page.clone() },
            },
        },
        30 => match script.pick(3) {
            0 => Op::SetLanguage { language: script.one(&["en", "fr", "de"]).into() },
            1 => Op::AddWord { word: script.one(&["mot", "word", "字\u{1e}x"]).into() },
            _ => Op::RemoveWord { word: script.one(&["mot", "word", "字\u{1e}x"]).into() },
        },
        _ => {
            let source = script.one(&["title: a\n", "title: b\n"]).to_owned();
            let properties = Properties { format: PropertiesFormat::Toml, source, extra: Default::default() };
            Op::SetProperties { properties: script.flag().then_some(properties) }
        }
    })
}

/// The next few edits of a script, each fitted to the document as it stands. Together they may ask for what
/// the first of them made impossible, which a model refuses whole.
pub fn ops(script: &mut Script, shown: &Shown, most: usize) -> Vec<Op> {
    (0..=script.pick(most)).filter_map(|_| op(script, shown)).collect()
}

/// A document and its style from whatever an input holds. The first byte says how the rest is read: as
/// Markdown, as HTML, as `document.json`, or as edits to the sample, which also reach its style and paper.
/// `None` for JSON that is no document.
pub fn written(data: &[u8]) -> Option<Shown> {
    let mut script = Script::new(data);
    let mut ids = crate::checks::ids();
    Some(match script.pick(4) {
        0 => (letters_convert::from_markdown(&crate::input::text(script.rest()), &mut ids).0, Style::default()),
        1 => (letters_convert::from_html(&crate::input::text(script.rest()), &mut ids).0, Style::default()),
        2 => (Document::from_json(&crate::input::text(script.rest())).ok()?, Style::default()),
        _ => {
            let mut model = Model::new(sample(), 1.into());
            while !script.done() {
                // A refused batch changes nothing, and the script goes on from what stands.
                let _ = model.apply(ops(&mut script, &shown(&model), 2));
            }
            shown(&model)
        }
    })
}

/// Inputs for [`written`]: one of each reading.
pub fn written_seeds() -> Vec<Vec<u8>> {
    let markdown = include_str!("../../crates/letters-convert/tests/corpus/report.md");
    let (doc, _) = letters_convert::from_markdown(markdown, &mut crate::checks::ids());
    let html = letters_convert::to_html_fragment(&doc.blocks);
    let json = include_str!("../../crates/letters-format/corpus/document/notice.json");
    let edits: Vec<u8> = (0..96u8).flat_map(|step| [step, 1, step / 3, 2, 3]).collect();
    let tagged = |tag: u8, rest: &[u8]| [&[tag], rest].concat();
    vec![tagged(0, markdown.as_bytes()), tagged(1, html.as_bytes()), tagged(2, json.as_bytes()), tagged(3, &edits)]
}
