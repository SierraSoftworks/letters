//! Figures and galleries: pictures coming in and moving about, and what is said of them.

use letters_model::{Block, BlockId, BlockKind, Body, Op, Pos};
use sha2::{Digest, Sha256};

use crate::{Editor, Picture, Platform, text, tree};

/// Where a picture goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spot {
    /// Beside a figure, before it or after it: into its gallery, or into one made of the two.
    Beside { figure: BlockId, after: bool },
    /// On a line of its own, above or below a block. A block inside a gallery or a table stands for the
    /// whole of it.
    Line { block: BlockId, below: bool },
}

/// A spot as the document has it.
enum Home {
    /// Among the pictures of a gallery.
    Gallery { gallery: BlockId, index: usize },
    /// With a figure that stands alone, in a gallery made of them.
    Pair { figure: Block, parent: Option<BlockId>, index: usize, after: bool },
    /// Among the blocks of `parent`, as a block of its own.
    Line { parent: Option<BlockId>, index: usize },
}

/// The file extension of a picture Letters can show, from how its bytes begin.
fn extension(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [0xff, 0xd8, 0xff, ..] => Some("jpg"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

/// The name a picture's bytes are kept under among the file's assets, which is the same for the same
/// bytes. `None` for bytes that are no picture Letters can show.
fn asset_name(bytes: &[u8]) -> Option<String> {
    let extension = extension(bytes)?;
    let hash: String = Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!("assets/{hash}.{extension}"))
}

/// The size in points to show a picture of so many pixels at: three quarters of a point to the pixel, as
/// on a web page, and no wider than `widest`.
fn fitted(width: u32, height: u32, widest: f32) -> (f32, f32) {
    let (width, height) = (width.max(1) as f32 * 0.75, height.max(1) as f32 * 0.75);
    let scale = if widest > 0.0 { (widest / width).min(1.0) } else { 1.0 };
    ((width * scale).max(1.0), (height * scale).max(1.0))
}

/// A figure as one of a gallery's pictures. The gallery has the caption and the number.
fn member(mut figure: Block) -> Block {
    if let BlockKind::Figure { numbered, .. } = &mut figure.kind {
        *numbered = false;
    }
    figure.body = Body::Text(Vec::new());
    figure
}

/// A gallery of `parts`.
fn gallery(parts: Vec<Block>, numbered: bool, platform: &mut dyn Platform) -> Block {
    let mut gallery = Block::new(platform.new_id().as_str(), BlockKind::Gallery { numbered });
    gallery.body = Body::Blocks(parts);
    gallery
}

/// Whether `typed`, the text from the start of a caption to the caret, is a label written out by hand,
/// such as "Figure 1: ". `word` is the style's word for a figure.
fn labels(typed: &str, word: &str) -> bool {
    let (typed, word) = (typed.to_lowercase(), word.to_lowercase());
    let rest = typed.strip_prefix(&word).and_then(|rest| rest.strip_prefix(' '));
    let number = rest.and_then(|rest| rest.strip_suffix(": ").or(rest.strip_suffix(". ")));
    number.is_some_and(|digits| (1..=4).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

impl Editor {
    /// The figure a command about one picture acts on: the one whose picture is selected, or in whose
    /// caption the caret is.
    pub(crate) fn subject(&self) -> Option<&Block> {
        self.block(&self.selection()?.head.block).filter(|block| tree::is_figure(block))
    }

    /// What carries the number that goes with a block: the gallery of a picture or a caption, or a
    /// figure that stands alone.
    pub(crate) fn numbered_by(&self, id: &BlockId) -> Option<&Block> {
        self.gallery(id).or_else(|| self.block(id).filter(|block| tree::is_figure(block)))
    }

    /// The picture that the caption the caret is in stands under: the figure's own, or the last of the
    /// gallery.
    pub(crate) fn captioned(&self) -> Option<BlockId> {
        let caret = self.caret.as_ref().filter(|caret| !caret.picture)?;
        let head = &caret.selection.head.block;
        match self.block(head)?.kind {
            BlockKind::Figure { .. } => Some(head.clone()),
            BlockKind::Caption => {
                let pictures = self.gallery(head)?.body.blocks().iter().rev();
                pictures.filter(|part| tree::is_figure(part)).map(|figure| figure.id.clone()).next()
            }
            _ => None,
        }
    }

    /// Esc in a caption goes back to the picture.
    pub(crate) fn leave_caption(&mut self) {
        if let Some(picture) = self.captioned() {
            self.place_picture(&picture);
        }
    }

    /// Turns the generated label of a figure or a gallery on or off.
    pub(crate) fn set_numbered(&mut self, holder: &BlockId, on: bool) {
        let Some(mut kind) = self.block(holder).map(|block| block.kind.clone()) else { return };
        if let BlockKind::Figure { numbered, .. } | BlockKind::Gallery { numbered } = &mut kind {
            *numbered = on;
            self.apply(vec![Op::SetKind { block: holder.clone(), kind }]);
        }
    }

    /// A label typed out at the start of a caption, such as "Figure 1: ", turns the generated one on and
    /// goes. Its number is not kept, since figures are numbered in the order they come.
    pub(crate) fn label_rule(&mut self, at: &Pos) -> bool {
        let word = letters_layout::figure_word(self.model.style());
        let typed = self.block(&at.block).map(|block| block.plain_text());
        let typed = typed.as_deref().and_then(|text| text.get(..at.offset)).is_some_and(|typed| labels(typed, word));
        let caption = self.block(&at.block).is_some_and(|block| !self.member(&block.id));
        let Some(holder) = self.numbered_by(&at.block).filter(|_| typed && caption).map(|holder| holder.id.clone())
        else {
            return false;
        };
        self.apply(vec![Op::DeleteText { block: at.block.clone(), range: 0..at.offset }]);
        self.set_numbered(&holder, true);
        self.place(Pos::new(at.block.clone(), 0));
        true
    }

    /// Changes the fields of the caret's figure.
    fn refigure(&mut self, change: impl FnOnce(&mut BlockKind)) {
        let Some(figure) = self.subject() else { return };
        let (id, mut kind) = (figure.id.clone(), figure.kind.clone());
        change(&mut kind);
        self.apply(vec![Op::SetKind { block: id, kind }]);
    }

    /// Sets the size the caret's figure is shown at, in points. A size that is no size changes nothing.
    pub(crate) fn resize_figure(&mut self, wide: f32, high: f32) {
        if [wide, high].iter().all(|size| size.is_finite() && *size > 0.0) {
            self.refigure(|kind| {
                if let BlockKind::Figure { width, height, .. } = kind {
                    (*width, *height) = (wide, high);
                }
            });
        }
    }

    pub(crate) fn set_alt(&mut self, words: String) {
        self.refigure(|kind| {
            if let BlockKind::Figure { alt, .. } = kind {
                *alt = words;
            }
        });
    }

    /// Whether a figure still shows its picture from an address, and `picture` can be kept in its place.
    pub(crate) fn embeddable(&self, figure: &BlockId, picture: &Picture) -> bool {
        let linked = |block: &Block| matches!(block.kind, BlockKind::Figure { asset: None, .. });
        self.block(figure).is_some_and(linked) && extension(&picture.bytes).is_some()
    }

    /// Keeps a linked picture's bytes in the file, and has its figure name them in place of the address.
    pub(crate) fn embed_figure(&mut self, figure: &BlockId, picture: &Picture, platform: &mut dyn Platform) {
        let Some(mut kind) = self.block(figure).map(|block| block.kind.clone()) else { return };
        let (BlockKind::Figure { asset: asset @ None, src, width, height, .. }, Some(name)) =
            (&mut kind, asset_name(&picture.bytes))
        else {
            return;
        };
        platform.store_asset(&name, &picture.bytes);
        (*asset, *src) = (Some(name), None);
        // The size stored with a link may be a guess. The picture's own shape is not, so it sets the height
        // for the width the figure is shown at.
        if picture.width > 0 && picture.height > 0 {
            *height = (*width * picture.height as f32 / picture.width as f32).max(1.0);
        }
        self.apply(vec![Op::SetKind { block: figure.clone(), kind }]);
    }

    /// Hands a picture's bytes to the shell under a name made from them, and makes the figure that shows
    /// it. `None` for bytes that are no picture Letters can show.
    fn figure(&self, picture: &Picture, platform: &mut dyn Platform) -> Option<Block> {
        let asset = asset_name(&picture.bytes)?;
        platform.store_asset(&asset, &picture.bytes);
        let page = &self.model.style().page;
        let (width, height) = fitted(picture.width, picture.height, page.width - page.margin_left - page.margin_right);
        let (asset, alt) = (Some(asset), String::new());
        let kind = BlockKind::Figure { asset, src: None, alt, width, height, numbered: false };
        Some(Block::new(platform.new_id().as_str(), kind))
    }

    /// Where a spot is in the document as it stands. `None` when it names nothing a picture can go by,
    /// or only the place `moved` is in already.
    fn home(&self, spot: &Spot, moved: Option<&BlockId>) -> Option<Home> {
        match spot {
            Spot::Beside { figure, after } => {
                let block = self.block(figure).filter(|block| tree::is_figure(block) && moved != Some(figure))?;
                let place = self.seat(figure)?;
                match place.parent.filter(|parent| tree::is_gallery(parent)) {
                    Some(gallery) => {
                        Some(Home::Gallery { gallery: gallery.id.clone(), index: place.index + usize::from(*after) })
                    }
                    None => {
                        let (figure, parent) = (block.clone(), place.parent_id());
                        Some(Home::Pair { figure, parent, index: place.index, after: *after })
                    }
                }
            }
            Spot::Line { block, below } => {
                let unit = self.unit(block);
                let place = self.seat(&unit).filter(|_| moved != Some(&unit))?;
                Some(Home::Line { parent: place.parent_id(), index: place.index + usize::from(*below) })
            }
        }
    }

    /// Where pictures go when no spot is named: beside the selected picture of a gallery, in place of an
    /// empty line, below the block the selection ends in, or at the end of a document with no text. With
    /// the home comes the empty line that gives way.
    fn home_at_caret(&self) -> (Option<Home>, Option<BlockId>) {
        let Some(caret) = &self.caret else {
            return (Some(Home::Line { parent: None, index: self.doc().blocks.len() }), None);
        };
        let end = self.ordered(&caret.selection).1.block;
        if caret.picture && self.member(&end) {
            return (self.home(&Spot::Beside { figure: end, after: true }, None), None);
        }
        let empty = self.block(&end).is_some_and(|block| block.kind == BlockKind::Paragraph && text::len(block) == 0);
        let home = self.home(&Spot::Line { block: end.clone(), below: !empty }, None);
        (home, empty.then_some(end))
    }

    /// Puts `figures` at a home, as one edit. `moved` is the one among them that the document has
    /// already, which leaves where it was, and `instead` an empty line that gives way. Returns whether
    /// the document changed.
    fn put(
        &mut self,
        figures: Vec<Block>,
        home: Home,
        moved: Option<&BlockId>,
        instead: Option<&BlockId>,
        platform: &mut dyn Platform,
    ) -> bool {
        let gone = moved.and_then(|id| self.seat(id)).map(|place| (place.parent_id(), place.index));
        // A block that left from before a place in the same holder brings that place up by one.
        let shifted = |parent: &Option<BlockId>, index: usize| {
            index - usize::from(gone.as_ref().is_some_and(|(from, at)| from == parent && *at < index))
        };
        let leave = |id: &BlockId| Op::RemoveBlock { block: id.clone() };
        let mut ops: Vec<Op> = moved.into_iter().chain(instead).map(leave).collect();
        match home {
            Home::Gallery { gallery, index } => {
                let parent = Some(gallery);
                let index = shifted(&parent, index);
                let join = |(n, block)| Op::InsertBlock { parent: parent.clone(), index: index + n, block };
                ops.extend(figures.into_iter().map(member).enumerate().map(join));
            }
            Home::Pair { figure, parent, index, after } => {
                let numbered = matches!(figure.kind, BlockKind::Figure { numbered: true, .. });
                // The caption the pictures share is the figure's, or else the first that came with the others.
                let captions = [&figure].into_iter().chain(&figures).map(|block| block.body.runs());
                let caption = captions.filter(|runs| !runs.is_empty()).map(<[_]>::to_vec).next().map(|runs| {
                    let mut caption = Block::new(platform.new_id().as_str(), BlockKind::Caption);
                    caption.body = Body::Text(runs);
                    caption
                });
                ops.push(leave(&figure.id));
                let mut parts: Vec<Block> = figures.into_iter().map(member).collect();
                parts.insert(if after { 0 } else { parts.len() }, member(figure));
                parts.extend(caption);
                let block = gallery(parts, numbered, platform);
                ops.push(Op::InsertBlock { index: shifted(&parent, index), parent, block });
            }
            Home::Line { parent, index } => {
                let index = shifted(&parent, index);
                let block = match <[Block; 1]>::try_from(figures) {
                    Ok([alone]) => alone,
                    Err(several) => gallery(several.into_iter().map(member).collect(), false, platform),
                };
                ops.push(Op::InsertBlock { parent, index, block });
            }
        }
        self.apply(ops)
    }

    /// Puts pictures into the document: one as a figure, several as a gallery. The last one is left
    /// selected.
    pub(crate) fn insert_figures(&mut self, pictures: &[Picture], at: Option<&Spot>, platform: &mut dyn Platform) {
        let (home, instead) = match at {
            Some(spot) => (self.home(spot, None), None),
            None => self.home_at_caret(),
        };
        let Some(home) = home else { return };
        let figures: Vec<Block> = pictures.iter().filter_map(|picture| self.figure(picture, platform)).collect();
        let Some(last) = figures.last().map(|figure| figure.id.clone()) else { return };
        if self.put(figures, home, None, instead.as_ref(), platform) {
            self.place_picture(&last);
        }
    }

    /// Moves the caret's figure to a spot, and leaves its picture selected. A gallery it leaves with one
    /// picture becomes a figure in the same step.
    pub(crate) fn move_figure(&mut self, spot: &Spot, platform: &mut dyn Platform) {
        let Some(figure) = self.subject().cloned() else { return };
        let id = figure.id.clone();
        let Some(home) = self.home(spot, Some(&id)) else { return };
        if self.put(vec![figure], home, Some(&id), None, platform) {
            self.settle();
            self.place_picture(&self.renamed(&id));
        }
    }

    /// Moves the selected picture one place along its gallery. At either end it stays.
    pub(crate) fn shift_figure(&mut self, right: bool, platform: &mut dyn Platform) {
        let Some(id) = self.picture().cloned() else { return };
        let Some(gallery) = self.gallery(&id) else { return };
        let pictures: Vec<&Block> = gallery.body.blocks().iter().filter(|part| tree::is_figure(part)).collect();
        let Some(at) = pictures.iter().position(|picture| picture.id == id) else { return };
        let beside = if right { pictures.get(at + 1) } else { at.checked_sub(1).and_then(|at| pictures.get(at)) };
        if let Some(beside) = beside {
            let spot = Spot::Beside { figure: beside.id.clone(), after: right };
            self.move_figure(&spot, platform);
        }
    }
}
