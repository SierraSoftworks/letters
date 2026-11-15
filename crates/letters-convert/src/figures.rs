//! What the formats share about pictures: how a picture's bytes become an asset, and how the numbers of
//! figures, which a document never stores, are written as text and read back from it.

use std::collections::BTreeMap;
use std::mem;

use letters_layout::figure_numbers;
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Run};
use letters_render::image::Image;
use sha2::{Digest, Sha256};

/// The word a numbered figure's caption opens with, and a reference to one reads as.
const LABEL: &str = "Figure";

/// The size, in points, a picture is shown at until its own size is known.
pub(crate) const UNSIZED: (f32, f32) = (320.0, 240.0);

/// A picture wider than this many points is scaled down to it, which fits between the margins of most paper.
const WIDEST: f32 = 480.0;

/// The largest picture an import keeps.
const LARGEST: usize = 64 << 20;

/// Keeps a picture as an asset named after its content. Returns the name and the size to show the picture at,
/// or `None` for bytes that are not a picture Letters can show.
pub(crate) fn asset(bytes: Vec<u8>, assets: &mut BTreeMap<String, Vec<u8>>) -> Option<(String, f32, f32)> {
    let extension = match bytes.as_slice() {
        _ if bytes.len() > LARGEST => return None,
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, 0xff, ..] => "jpg",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        _ => return None,
    };
    let image = Image::decode(&bytes).ok()?;
    let (width, height) = fitted(image.width() as f32, image.height() as f32);
    let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    let name = format!("assets/{hash}.{extension}");
    assets.insert(name.clone(), bytes);
    Some((name, width, height))
}

/// The size in points to show a picture of so many pixels at.
pub(crate) fn fitted(width: f32, height: f32) -> (f32, f32) {
    // A pixel is three quarters of a point, as on a web page.
    let (width, height) = (width * 0.75, height * 0.75);
    let (width, height) = if width > WIDEST { (WIDEST, height * WIDEST / width) } else { (width, height) };
    (width.max(1.0), height.max(1.0))
}

/// A copy of `doc` in which the numbers of its figures are text, for a format that cannot generate them: each
/// numbered caption opens with its label, and each reference reads as the label of its figure. The numbers
/// are the ones layout draws.
pub(crate) fn labelled(doc: &Document) -> Document {
    fn label(blocks: &mut [Block], numbers: &BTreeMap<BlockId, u32>) {
        for block in blocks {
            let pictured = matches!(block.kind, BlockKind::Figure { .. } | BlockKind::Gallery { .. });
            let opening = numbers.get(&block.id).filter(|_| pictured).map(|number| format!("{LABEL} {number}"));
            match &mut block.body {
                Body::Text(runs) => {
                    for run in runs.iter_mut() {
                        let target = run.marks.iter().find_map(|mark| match mark {
                            Mark::Ref(target) => numbers.get(target),
                            _ => None,
                        });
                        if let Some(number) = target {
                            run.text = format!("{LABEL} {number}");
                        }
                    }
                    if let Some(opening) = opening {
                        caption(runs, opening);
                    }
                }
                Body::Blocks(children) => {
                    label(children, numbers);
                    let Some(opening) = opening else { continue };
                    // A gallery's caption is the last it holds.
                    let last = children.iter().rposition(|child| child.kind == BlockKind::Caption);
                    let last = last.unwrap_or_else(|| {
                        children.push(Block::new(format!("{}-caption", block.id).as_str(), BlockKind::Caption));
                        children.len() - 1
                    });
                    if let Body::Text(runs) = &mut children[last].body {
                        caption(runs, opening);
                    }
                }
                Body::None => {}
            }
        }
    }
    fn caption(runs: &mut Vec<Run>, label: String) {
        let captioned = runs.iter().any(|run| !run.text.is_empty());
        runs.insert(0, Run::plain(if captioned { format!("{label}: ") } else { label }));
    }
    let mut doc = doc.clone();
    let numbers: BTreeMap<BlockId, u32> = figure_numbers(&doc).into_iter().collect();
    if !numbers.is_empty() {
        label(&mut doc.blocks, &numbers);
    }
    doc
}

/// Undoes [`labelled`] for a document read from such a format: a caption that opens with a label makes its
/// figure numbered, and text that reads as one of those labels becomes a reference to that figure.
pub(crate) fn relabel(doc: &mut Document) {
    let mut labels = BTreeMap::new();
    number(&mut doc.blocks, &mut labels);
    if !labels.is_empty() {
        link(&mut doc.blocks, &labels);
    }
}

/// Takes the label off the start of a caption, and returns the number it gave.
fn unlabel(runs: &mut Vec<Run>) -> Option<usize> {
    let alone = runs.len() == 1;
    let first = runs.first_mut()?;
    let rest = first.text.strip_prefix(LABEL)?.strip_prefix(' ')?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let number = rest[..digits].parse().ok()?;
    let caption = match rest[digits..].strip_prefix(':') {
        Some(caption) => caption.strip_prefix(' ').unwrap_or(caption),
        None if alone && digits == rest.len() => "",
        None => return None,
    };
    first.text = caption.to_owned();
    runs.retain(|run| !run.text.is_empty());
    Some(number)
}

/// Makes each figure whose caption opens with a label numbered, and lists the figures by the numbers written.
fn number(blocks: &mut [Block], labels: &mut BTreeMap<usize, BlockId>) {
    for block in blocks {
        let written = match (&block.kind, &mut block.body) {
            (BlockKind::Figure { .. }, Body::Text(runs)) => unlabel(runs),
            (BlockKind::Gallery { .. }, Body::Blocks(children)) => {
                let last = children.iter().rposition(|child| child.kind == BlockKind::Caption);
                let (written, emptied) = match last.map(|at| &mut children[at].body) {
                    Some(Body::Text(runs)) => (unlabel(runs), runs.is_empty()),
                    _ => (None, false),
                };
                // A caption that was its label alone was only there to carry it.
                if let (Some(at), Some(_), true) = (last, written, emptied) {
                    children.remove(at);
                }
                written
            }
            (_, Body::Blocks(children)) => {
                number(children, labels);
                None
            }
            _ => None,
        };
        if let Some(written) = written {
            if let BlockKind::Figure { numbered, .. } | BlockKind::Gallery { numbered } = &mut block.kind {
                *numbered = true;
            }
            labels.entry(written).or_insert_with(|| block.id.clone());
        }
    }
}

/// Makes text that reads as a figure's label a reference to that figure.
fn link(blocks: &mut [Block], labels: &BTreeMap<usize, BlockId>) {
    for block in blocks {
        match &mut block.body {
            Body::Text(runs) if !matches!(block.kind, BlockKind::Code { .. } | BlockKind::Formula) => {
                *runs = mem::take(runs).into_iter().flat_map(|run| refer(run, labels)).collect();
            }
            Body::Blocks(children) => link(children, labels),
            _ => {}
        }
    }
}

/// Splits a run around the labels in its text, each of which becomes a reference.
fn refer(run: Run, labels: &BTreeMap<usize, BlockId>) -> Vec<Run> {
    if run.marks.iter().any(|mark| matches!(mark, Mark::Code | Mark::Math | Mark::Ref(_))) {
        return vec![run];
    }
    let text = run.text.as_str();
    let part = |range: std::ops::Range<usize>| Run { text: text[range].to_owned(), ..run.clone() };
    let (mut out, mut from, mut search) = (Vec::new(), 0, 0);
    while let Some(at) = text[search..].find(LABEL).map(|found| search + found) {
        search = at + LABEL.len();
        let Some(rest) = text[search..].strip_prefix(' ') else { continue };
        let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let target = rest[..digits].parse::<usize>().ok().and_then(|number| labels.get(&number));
        let Some(target) = target.filter(|_| !text[..at].ends_with(char::is_alphanumeric)) else { continue };
        search += 1 + digits;
        if at > from {
            out.push(part(from..at));
        }
        let mut reference = part(at..search);
        reference.marks.insert(Mark::Ref(target.clone()));
        out.push(reference);
        from = search;
    }
    if from == 0 {
        return vec![run];
    }
    if from < text.len() {
        out.push(part(from..text.len()));
    }
    out
}
