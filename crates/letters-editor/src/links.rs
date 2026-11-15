//! A link and the page it leads to. The address of a web page becomes a link where it is pasted or typed
//! among text, and a card, a [`BlockKind::LinkPreview`], where it is put on a line of its own. Backspace
//! after a card makes it a link in a line of text again. For a link in the text the document keeps what a
//! fetch found of its page, by its address, for the link to show.

use std::collections::BTreeMap;
use std::ops::Range;

use letters_model::{Block, BlockId, BlockKind, Body, Mark, Op, Pos, Preview, Run};

use crate::{Editor, Picture, Platform, figures, text, tree, whole};

/// Whether `text` is an address that starts as one of `schemes` does, and nothing else.
fn under(text: &str, schemes: &[&str]) -> bool {
    schemes.iter().any(|scheme| text.len() > scheme.len() && text.starts_with(scheme))
        && !text.contains(char::is_whitespace)
}

/// Whether `text` is an address that text can lead to.
pub(crate) fn is_address(text: &str) -> bool {
    under(text, &["http://", "https://", "mailto:"])
}

/// Whether `text` is the address of a web page, which a card can show.
pub(crate) fn is_page(text: &str) -> bool {
    under(text, &["http://", "https://"])
}

/// Whether a line is the address of a web page as it was typed: with no formatting, and no link yet. A
/// line that holds a link is left as its writer made it, which is what keeps a card that was turned back
/// from becoming one again.
fn typed(line: &Block) -> bool {
    let bare = |run: &Run| run.link.is_none() && run.marks.is_empty();
    line.kind == BlockKind::Paragraph && is_page(&line.plain_text()) && line.body.runs().iter().all(bare)
}

/// Where the address of a web page sits that ends `typed`: the last word of it, without the punctuation
/// that ends a sentence.
fn ended(typed: &str) -> Option<Range<usize>> {
    let start = typed.char_indices().rev().find(|(_, c)| c.is_whitespace()).map_or(0, |(at, c)| at + c.len_utf8());
    let word = typed[start..].trim_end_matches(['.', ',', ';', ':', '!', '?']);
    is_page(word).then(|| start..start + word.len())
}

/// The address a line leads to, when it holds one link and nothing else.
fn lone(line: &Block) -> Option<&str> {
    let mut runs = line.body.runs().iter().filter(|run| !run.text.is_empty());
    let link = runs.next()?.link.as_deref()?;
    (line.kind == BlockKind::Paragraph && runs.all(|run| run.link.as_deref() == Some(link))).then_some(link)
}

/// A block in the place of `of`, which keeps what its place says of it.
fn instead(of: &Block, id: String, kind: BlockKind) -> Block {
    Block { lang: of.lang.clone(), page_break: of.page_break, ..Block::new(id.as_str(), kind) }
}

impl Editor {
    /// The card that leads where `link` does, with what the document keeps of the page. `None` when
    /// nothing is left of the address.
    pub(crate) fn card(&self, link: &str) -> Option<BlockKind> {
        let url = whole::address(link)?;
        let previews = &self.doc().previews;
        let kept = previews.get(link).or_else(|| previews.get(&url)).cloned().unwrap_or_default();
        Some(BlockKind::LinkPreview { url, title: kept.title, description: kept.description, image: kept.image })
    }

    /// The line the caret is on and the address it leads to, when it holds one link and nothing else.
    fn lone_link(&self) -> Option<(&Block, &str)> {
        let line = self.block(&self.selection()?.head.block)?;
        lone(line).filter(|link| whole::address(link).is_some()).map(|link| (line, link))
    }

    /// Whether the caret's line can be shown as a card.
    pub(crate) fn takes_card(&self) -> bool {
        self.lone_link().is_some()
    }

    /// Notes that the writer has just put in a link or a card that leads to `address`, for
    /// [`Editor::take_unfetched`]: the address of a web page that the document keeps nothing of yet.
    pub(crate) fn put_in(&mut self, address: &str) {
        let wanted = self.fetching && is_page(address) && !self.doc().previews.contains_key(address);
        if wanted && !self.unfetched.iter().any(|noted| noted == address) {
            self.unfetched.push(address.to_owned());
        }
    }

    /// Makes a link of the address of a web page that was typed in a block up to `end`, as a space or
    /// Enter after it asks. Code is left as it is written, and so is what is a link already. Returns
    /// whether it did.
    pub(crate) fn link_typed(&mut self, id: &BlockId, end: usize) -> bool {
        let Some(block) = self.block(id).filter(|block| !text::is_source(block)) else { return false };
        let text = block.plain_text();
        let Some(range) = text.get(..end).and_then(ended) else { return false };
        let bare = |run: &Run| {
            let literal = |mark: &Mark| matches!(mark, Mark::Code | Mark::Math | Mark::Ref(_));
            run.link.is_none() && !run.marks.iter().any(literal)
        };
        if !text::runs_in(block.body.runs(), range.clone()).all(bare) {
            return false;
        }
        let address = text[range.clone()].to_owned();
        let linked = self.apply(vec![Op::SetLink { block: id.clone(), range, link: Some(address.clone()) }]);
        if linked {
            self.put_in(&address);
        }
        linked
    }

    /// Keeps what pasted text said of the pages it links to, where the document keeps nothing of a page.
    pub(crate) fn keep_told(&mut self, told: BTreeMap<String, Preview>) {
        let previews = &self.doc().previews;
        let line = |url: &str| !url.is_empty() && !url.contains(['\n', '\r']);
        let new = |(url, preview): &(String, Preview)| {
            line(url) && !previews.contains_key(url) && preview.sound() && !preview.is_empty()
        };
        let keep = |(url, preview)| Op::SetPreview { url, preview: Some(preview) };
        let ops = told.into_iter().filter(new).map(keep).collect();
        self.apply(ops);
    }

    /// Shows the link that the caret's line holds alone as a card in the line's place. The caret goes to
    /// the text below it, on a line made when there is none.
    pub(crate) fn link_as_card(&mut self, platform: &mut dyn Platform) {
        let doc = self.doc();
        let Some((line, link)) = self.lone_link() else { return };
        let (Some(place), Some(kind)) = (self.seat(&line.id), self.card(link)) else { return };
        let (parent, url) = (place.parent_id(), whole::address(link));
        // The card goes in before the line goes, so that what holds them is never left empty.
        let card = instead(line, platform.new_id(), kind);
        let mut ops = vec![
            Op::InsertBlock { parent: parent.clone(), index: place.index, block: card },
            Op::RemoveBlock { block: line.id.clone() },
        ];
        let below = match place.siblings(doc).get(place.index + 1).filter(|below| tree::flows(below)) {
            Some(below) => below.id.clone(),
            None => {
                let new = BlockId::new(platform.new_id());
                let block = Block::new(new.clone(), BlockKind::Paragraph);
                ops.push(Op::InsertBlock { parent, index: place.index + 1, block });
                new
            }
        };
        if self.apply(ops) {
            url.iter().for_each(|url| self.put_in(url));
        }
        self.place(Pos::new(below, 0));
    }

    /// Shows a card as the link it leads to: a line of text in its place that holds the link. What the
    /// card showed of its page is kept for the link, so that it shows the same with no new fetch.
    pub(crate) fn card_as_link(&mut self, card: &BlockId, platform: &mut dyn Platform) {
        let doc = self.doc();
        let (Some(block), Some(place)) = (doc.block(card), self.seat(card)) else { return };
        let BlockKind::LinkPreview { url, title, description, image } = &block.kind else { return };
        let link = Run { link: Some(url.clone()), ..Run::plain(url.as_str()) };
        let line = Block { body: Body::Text(vec![link]), ..instead(block, platform.new_id(), BlockKind::Paragraph) };
        let mut ops = vec![
            Op::InsertBlock { parent: place.parent_id(), index: place.index, block: line },
            Op::RemoveBlock { block: card.clone() },
        ];
        if title.is_some() || description.is_some() || image.is_some() {
            let extra = doc.previews.get(url).map(|kept| kept.extra.clone()).unwrap_or_default();
            let told = Preview { title: title.clone(), description: description.clone(), image: image.clone(), extra };
            ops.push(Op::SetPreview { url: url.clone(), preview: Some(told) });
        }
        self.apply(ops);
    }

    /// Whether a line is an address that Enter after it makes a card of: one typed alone on its line,
    /// where a card has a page to show.
    pub(crate) fn typed_card(&self, line: &BlockId) -> bool {
        self.fetching && self.block(line).is_some_and(typed)
    }

    /// Whether the address of a web page pasted at the caret becomes a card: on an empty line of body
    /// text, where a card has a page to show.
    pub(crate) fn pasted_card(&self, address: &str, line: &BlockId) -> bool {
        let empty = |line: &Block| line.kind == BlockKind::Paragraph && text::len(line) == 0;
        self.fetching && is_page(address) && self.block(line).is_some_and(empty)
    }

    /// Shows the address of a web page as a card above the line it was put on alone, which is left empty
    /// for the writing to go on.
    pub(crate) fn address_as_card(&mut self, line: &BlockId, address: &str, platform: &mut dyn Platform) {
        let (Some(block), Some(place)) = (self.block(line), self.seat(line)) else { return };
        let Some(kind) = self.card(address) else { return };
        let card = Block::new(platform.new_id().as_str(), kind);
        let typed = (text::len(block) > 0).then(|| Op::DeleteText { block: line.clone(), range: 0..text::len(block) });
        let above = Op::InsertBlock { parent: place.parent_id(), index: place.index, block: card };
        if self.apply(typed.into_iter().chain([above]).collect()) {
            self.put_in(address);
            self.place(Pos::new(self.renamed(line), 0));
        }
    }

    /// The edits that make all the document shows of the page at `url` what a fetch found of it. None
    /// when it shows nothing of the page, or shows just that already.
    fn fills(
        &self,
        url: &str,
        title: &Option<String>,
        description: &Option<String>,
        picture: Option<&Picture>,
    ) -> Vec<Op> {
        let doc = self.doc();
        let (title, description) = (whole::folded(title), whole::folded(description));
        let image = picture.and_then(|picture| figures::asset_name(&picture.bytes));
        // Every card that leads there shows it.
        let address = whole::address(url);
        let card = |block: &Block| {
            let BlockKind::LinkPreview { url, .. } = &block.kind else { return None };
            let (url, title, description, image) = (url.clone(), title.clone(), description.clone(), image.clone());
            let kind = BlockKind::LinkPreview { url, title, description, image };
            let leads = matches!(&block.kind, BlockKind::LinkPreview { url, .. } if Some(url) == address.as_ref());
            (leads && kind != block.kind).then(|| Op::SetKind { block: block.id.clone(), kind })
        };
        let mut ops: Vec<Op> = doc.walk().filter_map(card).collect();
        // For a link in the text that leads there, the document keeps it by the address as the link has it.
        let leads = |block: &Block| block.body.runs().iter().any(|run| run.link.as_deref() == Some(url));
        if !url.is_empty() && !url.contains(['\n', '\r']) && doc.walk().any(leads) {
            let kept = doc.previews.get(url);
            let extra = kept.map(|kept| kept.extra.clone()).unwrap_or_default();
            let found = Some(Preview { title, description, image, extra }).filter(|found| !found.is_empty());
            if found.as_ref() != kept {
                ops.push(Op::SetPreview { url: url.to_owned(), preview: found });
            }
        }
        ops
    }

    /// Whether a fetch says anything new of a page that the document shows.
    pub(crate) fn takes_fetched(
        &self,
        url: &str,
        title: &Option<String>,
        description: &Option<String>,
        picture: Option<&Picture>,
    ) -> bool {
        !self.fills(url, title, description, picture).is_empty()
    }

    /// Takes what a fetch found of the page at `url`, in place of what the document showed of it.
    pub(crate) fn fetched(
        &mut self,
        url: &str,
        title: &Option<String>,
        description: &Option<String>,
        picture: Option<&Picture>,
        platform: &mut dyn Platform,
    ) {
        let ops = self.fills(url, title, description, picture);
        let shown = picture.and_then(|picture| Some((figures::asset_name(&picture.bytes)?, picture)));
        if let Some((name, picture)) = shown.filter(|_| !ops.is_empty()) {
            platform.store_asset(&name, &picture.bytes);
        }
        self.apply(ops);
    }
}
