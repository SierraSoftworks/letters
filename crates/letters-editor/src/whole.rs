//! Blocks that hold no text: a divider, a table of contents and a link preview. No caret stands in one, so
//! the caret steps over it, Backspace beside it removes it whole, and a change to one names it.

use letters_model::{Block, BlockId, BlockKind, Op, Pos};

use crate::{Editor, Picture, Platform, figures, text, tree};

/// How deep the headings go that a new table of contents lists.
const LEVELS: u8 = 3;

/// A change to a link preview. Text that is left empty is taken as none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewEdit {
    /// The address the card leads to. One with nothing in it is not taken.
    Url(String),
    /// The page's title, on one line.
    Title(Option<String>),
    Description(Option<String>),
    /// The card's picture, or none. The bytes go to [`Platform::store_asset`], and bytes that are no
    /// picture Letters can show are not taken.
    Picture(Option<Picture>),
    /// What a fetch of `url` found of the page, in place of the title, the description and the picture
    /// the card had. What was not found, the card no longer shows. A fetch takes time: when the card
    /// leads to another address by now, nothing of it is taken.
    Fetched {
        url: String,
        title: Option<String>,
        description: Option<String>,
        picture: Option<Picture>,
    },
}

impl PreviewEdit {
    /// The picture that comes with the edit.
    fn picture(&self) -> Option<&Picture> {
        match self {
            Self::Picture(picture) | Self::Fetched { picture, .. } => picture.as_ref(),
            _ => None,
        }
    }
}

/// An address as it is stored: on one line, with no space around it. `None` when nothing is left of it.
pub(crate) fn address(url: &str) -> Option<String> {
    let url: String = url.trim().chars().filter(|c| !matches!(c, '\n' | '\r')).collect();
    (!url.is_empty()).then_some(url)
}

/// A link preview after an edit. `None` for an edit that cannot be taken, or a block that is no preview.
fn edited(kind: &BlockKind, edit: &PreviewEdit) -> Option<BlockKind> {
    let BlockKind::LinkPreview { url, title, description, image } = kind else { return None };
    let (mut url, mut title, mut description, mut image) =
        (url.clone(), title.clone(), description.clone(), image.clone());
    let said = |text: String| (!text.is_empty()).then_some(text);
    // A page's own words come with the white space of its markup.
    let found = |text: &Option<String>| said(text.as_deref()?.split_whitespace().collect::<Vec<_>>().join(" "));
    match edit {
        PreviewEdit::Url(to) => url = address(to)?,
        // Text is typed a key at a time, so the spaces at its ends are left as typed.
        PreviewEdit::Title(to) => title = to.as_deref().map(|to| to.replace(['\n', '\r'], " ")).and_then(said),
        PreviewEdit::Description(to) => description = to.clone().and_then(said),
        PreviewEdit::Picture(None) => image = None,
        PreviewEdit::Picture(Some(picture)) => image = Some(figures::asset_name(&picture.bytes)?),
        PreviewEdit::Fetched { url: of, title: named, description: about, picture } => {
            if address(of).as_ref() != Some(&url) {
                return None;
            }
            (title, description) = (found(named), found(about));
            image = picture.as_ref().and_then(|picture| figures::asset_name(&picture.bytes));
        }
    }
    Some(BlockKind::LinkPreview { url, title, description, image })
}

impl Editor {
    /// Puts in a list of the document's headings.
    pub(crate) fn insert_contents(&mut self, platform: &mut dyn Platform) {
        self.insert_whole(BlockKind::TableOfContents { levels: LEVELS }, platform);
    }

    /// Puts in a card that leads to an address, and shows that alone until it is filled.
    pub(crate) fn insert_preview(&mut self, url: &str, platform: &mut dyn Platform) {
        if let Some(url) = address(url) {
            let kind = BlockKind::LinkPreview { url, title: None, description: None, image: None };
            self.insert_whole(kind, platform);
        }
    }

    /// Adds a block that holds no text after the caret's block, and leaves the caret in the text below it.
    /// An empty paragraph becomes that text, so the new block takes its place.
    pub(crate) fn insert_whole(&mut self, kind: BlockKind, platform: &mut dyn Platform) {
        let whole = Block::new(platform.new_id().as_str(), kind);
        let mut line = || {
            let id = BlockId::new(platform.new_id());
            (id.clone(), Block::new(id, BlockKind::Paragraph))
        };
        let Some(id) = self.selection().map(|selection| selection.head.block.clone()) else {
            // A document with no text yet starts with the block, and a line to write on below it.
            let (index, (below, block)) = (self.doc().blocks.len(), line());
            let ops = [whole, block].into_iter().enumerate();
            if self.apply(ops.map(|(n, block)| Op::InsertBlock { parent: None, index: index + n, block }).collect()) {
                self.place(Pos::new(below, 0));
            }
            return;
        };
        // After a part, the block follows the gallery or the table it is in.
        let unit = self.unit(&id);
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(&id), self.seat(&unit)) else { return };
        let parent = place.parent_id();
        if block.kind == BlockKind::Paragraph && text::len(block) == 0 {
            self.apply(vec![Op::InsertBlock { parent, index: place.index, block: whole }]);
            return;
        }
        let mut ops = vec![Op::InsertBlock { parent: parent.clone(), index: place.index + 1, block: whole }];
        let below = match place.siblings(doc).get(place.index + 1).filter(|below| tree::flows(below)) {
            Some(below) => below.id.clone(),
            None => {
                let (new, block) = line();
                ops.push(Op::InsertBlock { parent, index: place.index + 2, block });
                new
            }
        };
        self.apply(ops);
        self.place(Pos::new(below, 0));
    }

    /// Whether a table of contents can be told to list headings down to `levels`, and does not already.
    pub(crate) fn takes_levels(&self, table: &BlockId, levels: u8) -> bool {
        let listed = self.block(table).and_then(|block| match block.kind {
            BlockKind::TableOfContents { levels } => Some(levels),
            _ => None,
        });
        (1..=4).contains(&levels) && listed.is_some_and(|listed| listed != levels)
    }

    pub(crate) fn set_levels(&mut self, table: &BlockId, levels: u8) {
        if self.takes_levels(table, levels) {
            self.apply(vec![Op::SetKind { block: table.clone(), kind: BlockKind::TableOfContents { levels } }]);
        }
    }

    /// Whether an edit changes the link preview it names.
    pub(crate) fn takes_preview(&self, block: &BlockId, edit: &PreviewEdit) -> bool {
        self.block(block).is_some_and(|block| edited(&block.kind, edit).is_some_and(|kind| kind != block.kind))
    }

    pub(crate) fn preview_edit(&mut self, block: &BlockId, edit: &PreviewEdit, platform: &mut dyn Platform) {
        let Some(kind) = self.block(block).and_then(|block| edited(&block.kind, edit)) else { return };
        if let (BlockKind::LinkPreview { image: Some(name), .. }, Some(picture)) = (&kind, edit.picture()) {
            platform.store_asset(name, &picture.bytes);
        }
        self.apply(vec![Op::SetKind { block: block.clone(), kind }]);
    }
}
