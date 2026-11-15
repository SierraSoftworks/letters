//! What the formats share about pictures: how a picture's bytes become an asset, and how the numbers of
//! figures, which a document never stores, are written as text and read back from it.

use std::collections::BTreeMap;
use std::mem;

use letters_layout::figure_numbers;
use letters_model::{Block, BlockId, BlockKind, Body, Document, Mark, Run};
use letters_render::image::{MAX_PIXELS, MAX_SIDE};
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
/// or `None` for bytes that are not a picture Letters can show. The picture is not decoded: one whose file
/// starts well and is damaged further on is kept, and shows as an empty place.
pub(crate) fn asset(bytes: Vec<u8>, assets: &mut BTreeMap<String, Vec<u8>>) -> Option<(String, f32, f32)> {
    let extension = match bytes.as_slice() {
        _ if bytes.len() > LARGEST => return None,
        [0x89, b'P', b'N', b'G', ..] => "png",
        [0xff, 0xd8, 0xff, ..] => "jpg",
        [b'G', b'I', b'F', b'8', ..] => "gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "webp",
        _ => return None,
    };
    let (width, height) = size(&bytes)?;
    let (width, height) = fitted(width as f32, height as f32);
    let hash: String = Sha256::digest(&bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    let name = format!("assets/{hash}.{extension}");
    assets.insert(name.clone(), bytes);
    Some((name, width, height))
}

/// A whole number of `len` bytes at `at`, with its first byte the most significant where `big`.
fn uint(bytes: &[u8], at: usize, len: usize, big: bool) -> Option<u32> {
    let digits = bytes.get(at..)?.get(..len)?.iter();
    let join = |value: u32, digit: &u8| value << 8 | u32::from(*digit);
    Some(if big { digits.fold(0, join) } else { digits.rev().fold(0, join) })
}

/// A picture's size in pixels as it is shown, read from how its file starts, or `None` for a file that
/// does not say or a picture too large to draw. Nothing is decoded, so that a shell which draws its
/// pictures by other means, as a browser does, has no decoder to carry for the sake of two numbers.
fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    let (width, height) = match bytes {
        [0x89, b'P', b'N', b'G', ..] if bytes.get(12..16) == Some(b"IHDR") => {
            let (width, height) = (uint(bytes, 16, 4, true)?, uint(bytes, 20, 4, true)?);
            if on_its_side(exif_part(bytes, true)) { (height, width) } else { (width, height) }
        }
        [b'G', b'I', b'F', b'8', ..] => (uint(bytes, 6, 2, false)?, uint(bytes, 8, 2, false)?),
        // A WebP says its size where its first part does, by which of three kinds that part is.
        [b'R', b'I', b'F', b'F', ..] => match bytes.get(12..16)? {
            b"VP8 " if bytes.get(23..26) == Some(&[0x9d, 0x01, 0x2a]) => {
                (uint(bytes, 26, 2, false)? & 0x3fff, uint(bytes, 28, 2, false)? & 0x3fff)
            }
            b"VP8L" if bytes.get(20) == Some(&0x2f) => {
                let packed = uint(bytes, 21, 4, false)?;
                ((packed & 0x3fff) + 1, (packed >> 14 & 0x3fff) + 1)
            }
            b"VP8X" => {
                let (width, height) = (uint(bytes, 24, 3, false)? + 1, uint(bytes, 27, 3, false)? + 1);
                if on_its_side(exif_part(bytes, false)) { (height, width) } else { (width, height) }
            }
            _ => return None,
        },
        [0xff, 0xd8, ..] => jpeg_size(bytes)?,
        _ => return None,
    };
    let drawn = width > 0 && height > 0 && width <= MAX_SIDE && height <= MAX_SIDE;
    (drawn && u64::from(width) * u64::from(height) <= MAX_PIXELS).then_some((width, height))
}

/// A JPEG's size: that of its frame, with the two changed over where the camera says it was held on its
/// side, since the picture is shown upright.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let (mut at, mut on_its_side) = (2, false);
    loop {
        let [0xff, marker] = *bytes.get(at..)?.get(..2)? else { return None };
        match marker {
            // Padding before a marker.
            0xff => at += 1,
            // Markers that stand alone.
            0x01 | 0xd0..=0xd8 => at += 2,
            // The picture itself starts, or the file ends, and no frame was met.
            0xd9 | 0xda => return None,
            _ => {
                let length = uint(bytes, at + 2, 2, true)? as usize;
                let segment = bytes.get(at + 4..at + 2 + length)?;
                match marker {
                    // A frame header, which all of these are but the three that define tables.
                    0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                        let (height, width) = (uint(segment, 1, 2, true)?, uint(segment, 3, 2, true)?);
                        return Some(if on_its_side { (height, width) } else { (width, height) });
                    }
                    0xe1 if segment.starts_with(EXIF) => on_its_side = self::on_its_side(Some(segment)),
                    _ => {}
                }
                at += 2 + length;
            }
        }
    }
}

/// What EXIF opens with where it is not the whole of a part.
const EXIF: &[u8] = b"Exif\0\0";

/// The EXIF a PNG or a WebP holds in a part of its own, where the decoder looks for it. Both files are a
/// run of named parts, each with its length.
fn exif_part(bytes: &[u8], png: bool) -> Option<&[u8]> {
    let mut at: usize = if png { 8 } else { 12 };
    loop {
        let rest = bytes.get(at..)?;
        // A PNG writes the length first and its high byte first, and a WebP the name and the low byte.
        let (length, name) = if png { (0, 4) } else { (4, 0) };
        let length = uint(rest, length, 4, png)? as usize;
        let part = rest.get(8..)?.get(..length)?;
        match rest.get(name..name + 4)? {
            b"eXIf" | b"EXIF" => return Some(part),
            // A PNG's is heeded only where it comes before the pixels.
            b"IDAT" if png => return None,
            _ => {}
        }
        // A PNG's part ends in a checksum, and a WebP's of odd length in a byte of padding.
        at = at.checked_add(8 + if png { 4 } else { length & 1 })?.checked_add(length)?;
    }
}

/// Whether EXIF says the camera was held on its side, so that the picture is shown a quarter turned.
fn on_its_side(exif: Option<&[u8]>) -> bool {
    let turn = exif.and_then(|exif| orientation(exif.strip_prefix(EXIF).unwrap_or(exif)));
    turn.is_some_and(|turn| (5..=8).contains(&turn))
}

/// How EXIF says a picture is turned, from 1 for upright to 8.
fn orientation(exif: &[u8]) -> Option<u32> {
    let big = match exif.get(..4)? {
        b"MM\0*" => true,
        b"II*\0" => false,
        _ => return None,
    };
    let entries = uint(exif, 4, 4, big)? as usize;
    let count = uint(exif, entries, 2, big)? as usize;
    // An entry is 12 bytes: what it is, how it is written, how many, and the value.
    let turn = |entry: usize| {
        let at = entries.checked_add(2 + entry * 12)?;
        (uint(exif, at, 2, big)? == 0x0112).then(|| uint(exif, at.checked_add(8)?, 2, big)).flatten()
    };
    (0..count).find_map(turn)
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
