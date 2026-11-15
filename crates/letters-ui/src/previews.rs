//! Link previews on the page: cards that lead to an address. A press on one opens the card that changes
//! it, and with Ctrl or Cmd held follows it as a link. One that shows its address alone has a control that
//! fetches what the page says of itself.
//!
//! Fetching is the shell's to do: it is asked, and says what it found or why it found nothing. No caret
//! stands in a preview, so the window asks this module what a press on one is on.

use std::collections::BTreeMap;

use accesskit::Role;
use letters_editor::{Editor, Picture, PreviewEdit};
use letters_model::{Block, BlockId, BlockKind, Document};
use letters_render::{DisplayList, Item, Point, Rect};

use crate::color::resolve;
use crate::element::Element;
use crate::icon;
use crate::safe::Safely;
use crate::style::{Align, Justify};
use crate::text::{TextShaper, TextStyle};
use crate::tokens::{self, Scheme};
use crate::widgets::{Button, Popover, TextField};
use crate::window::Message;

/// What a link preview is called as one control, which a press opens the card of.
pub(crate) const LABEL: &str = "Edit link preview";
/// What the control on a preview that shows its address alone says: before a fetch, during one, and after
/// one that found nothing.
pub(crate) const FETCH: &str = "Fetch preview";
const FETCHING: &str = "Fetching";
const AGAIN: &str = "Try again";
/// What is said of a page that was read and says nothing of itself.
const NOTHING: &str = "the page has no title, description or picture";

/// How wide the card is.
const WIDTH: f32 = 320.0;
/// The most room the control takes across, and how tall it is: yewi's small button.
pub(crate) const CONTROL: (f32, f32) = (112.0, 24.0);

/// What a shell found of the page a link preview leads to, in answer to a `Request::FetchPreview`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LinkPreviewFound {
    /// The address that was fetched, as the request wrote it. What was found goes to what waits for it.
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// The picture the page gives for itself, which the document then holds.
    pub picture: Option<Picture>,
}

impl LinkPreviewFound {
    /// Whether the page said anything a card can show.
    pub(crate) fn says(&self) -> bool {
        let said = |text: &Option<String>| text.as_deref().is_some_and(|text| !text.trim().is_empty());
        said(&self.title) || said(&self.description) || self.picture.is_some()
    }
}

/// What is asked of a link preview and of its card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Previewing {
    /// Asks for the address of a preview to put in, as the insert menu does.
    New,
    /// Shows the card of a preview, as a press on it does.
    Open(BlockId),
    /// What the field of the address holds, before it is taken.
    Address(String),
    /// Takes the address that was typed. A preview that is not in the document yet is put in with it.
    TakeAddress,
    /// Drops what was typed into the field of the address.
    Forget,
    /// Changes the preview whose card is out, as the editor does.
    Edit(PreviewEdit),
    /// Asks the shell for what the page of the preview whose card is out says of itself.
    Fetch,
    /// Asks the shell for what the page at the address of a link in the text says of itself.
    FetchLink(String),
    /// Escape or a press elsewhere.
    Close,
}

/// The link previews of the page, as the pointer, the card and the shell have them.
#[derive(Default)]
pub(crate) struct Previews {
    /// The preview whose card is out.
    pub open: Option<BlockId>,
    /// Whether the card that is out asks for the address of a preview to put in.
    adding: bool,
    /// What is being typed into the field of the address.
    address: Option<String>,
    /// The preview under the pointer, and whether the pointer is on its control.
    pub over: Option<(BlockId, bool)>,
    /// The pages that were asked for, by their address: why the fetch found nothing, in the shell's words,
    /// which is `None` while the shell fetches. A card and a link in the text that lead to one address
    /// wait for one page.
    fetches: BTreeMap<String, Option<String>>,
    /// Whether the shell cannot fetch what a page says of itself, so that nothing offers a fetch.
    unfetched: bool,
    /// What the page was last told to write on a preview: why its page was not fetched.
    noted: BTreeMap<BlockId, String>,
    /// Whether what the page writes on its previews may hold no more: a fetch began or ended, or the
    /// document changed.
    stale: bool,
}

/// The address of a link preview.
pub(crate) fn address(kind: &BlockKind) -> Option<&str> {
    match kind {
        BlockKind::LinkPreview { url, .. } => Some(url),
        _ => None,
    }
}

/// Where the control stands in the room a card keeps for it: in its middle.
pub(crate) fn control(room: Rect) -> Rect {
    let wide = CONTROL.0.min(room.width - 2.0 * tokens::SPACE_SM).max(0.0);
    let (x, y) = (room.x + (room.width - wide) / 2.0, room.y + (room.height - CONTROL.1) / 2.0);
    Rect::new(x.round(), y.round(), wide, CONTROL.1)
}

/// Whether an address is a web page's, which a shell can fetch.
pub(crate) fn web(url: &str) -> bool {
    let scheme = url.split_once("://").map(|(scheme, _)| scheme.to_ascii_lowercase());
    matches!(scheme.as_deref(), Some("http" | "https"))
}

/// Draws a button that fetches a page, at `at`, as yewi's small button: what it `says`, lit while the
/// pointer is `over` it. One whose page is being fetched (`waits`) is its words alone.
pub(crate) fn button(
    says: &str,
    (waits, over): (bool, bool),
    at: Rect,
    shaper: &dyn TextShaper,
    scheme: Scheme,
    out: &mut DisplayList,
) {
    let radius = tokens::RADIUS_CONTROL;
    let word = TextStyle::control().size(tokens::TEXT_SM).weight(500);
    let shaped = shaper.shape(says, &word, None);
    let origin = Point {
        x: at.x + ((at.width - shaped.size.width) / 2.0).round(),
        y: at.y + (at.height - shaped.size.height) / 2.0,
    };
    if waits {
        return shaped.paint(origin, resolve(tokens::FG_MUTED, scheme, 1.0), out);
    }
    let ground = if over { tokens::TONE_1 } else { tokens::SURFACE };
    out.push(Item::Fill { rect: at, color: resolve(ground, scheme, 1.0), radius });
    out.push(Item::Stroke { rect: at, color: resolve(tokens::LINE_STRONG, scheme, 1.0), width: 1.0, radius });
    shaped.paint(origin, resolve(tokens::FG, scheme, 1.0), out);
}

impl Previews {
    /// Whether a card is out: that of a preview, or the one that asks for the address of a new one.
    pub fn carded(&self) -> bool {
        self.open.is_some() || self.adding
    }

    /// Whether the card that is out asks for the address of a preview to put in.
    pub fn adding(&self) -> bool {
        self.adding
    }

    /// Shows the card that asks for the address of a preview to put in.
    pub fn add(&mut self) {
        (self.open, self.adding, self.address) = (None, true, None);
    }

    /// Shows the card of a preview.
    pub fn show(&mut self, block: BlockId) {
        (self.open, self.adding, self.address) = (Some(block), false, None);
    }

    pub fn close(&mut self) {
        (self.open, self.adding, self.address) = (None, false, None);
    }

    pub fn typed(&mut self, address: String) {
        if self.carded() {
            self.address = Some(address);
        }
    }

    pub fn forget(&mut self) {
        self.address = None;
    }

    /// Takes the address that was typed, with the preview it is for: `None` for one to put in. The card
    /// of a new preview goes with it, whether or not anything was typed.
    pub fn take(&mut self) -> Option<(Option<BlockId>, String)> {
        let typed = self.address.take().map(|typed| typed.trim().to_owned()).filter(|typed| !typed.is_empty());
        let block = self.open.clone();
        if self.adding {
            self.close();
        }
        typed.map(|typed| (block, typed))
    }

    /// Takes whether the shell can fetch what a page says of itself. Where it cannot, nothing waits for
    /// a page.
    pub fn offer(&mut self, offered: bool) {
        (self.unfetched, self.stale) = (!offered, true);
        if self.unfetched {
            self.fetches.clear();
        }
    }

    /// Whether a fetch is offered: the shell can fetch what a page says of itself.
    pub fn offered(&self) -> bool {
        !self.unfetched
    }

    /// Notes that the page at an address is waited for. Returns whether the shell has to be asked: it can
    /// fetch, and no fetch of that address runs already.
    pub fn fetch(&mut self, url: &str) -> bool {
        if self.unfetched || self.waits(url) {
            return false;
        }
        self.fetches.insert(url.to_owned(), None);
        self.stale = true;
        true
    }

    /// Takes the shell's answer for an address: why it found nothing, or `None` when it found the page.
    /// Returns whether the page was waited for.
    pub fn answered(&mut self, url: &str, failed: Option<&str>) -> bool {
        if !self.waits(url) {
            return false;
        }
        match failed {
            Some(why) => _ = self.fetches.insert(url.to_owned(), Some(why.trim().to_owned())),
            None => _ = self.fetches.remove(url),
        }
        self.stale = true;
        true
    }

    /// Notes that the document changed: a preview may have come, gone or been given another address.
    pub fn edited(&mut self) {
        self.stale = true;
    }

    /// What is said of a page that was read and says nothing a card can show.
    pub fn nothing() -> &'static str {
        NOTHING
    }

    /// Puts the card away once its preview is gone.
    pub fn follow(&mut self, editor: &Editor) {
        let leads = |block: &BlockId| editor.doc().block(block).and_then(|block| address(&block.kind));
        if self.open.as_ref().is_some_and(|open| leads(open).is_none()) {
            self.close();
        }
    }

    /// What a control that fetches the page at an address says now.
    pub fn says(&self, url: &str) -> &'static str {
        match self.fetches.get(url) {
            Some(None) => FETCHING,
            Some(Some(_)) => AGAIN,
            None => FETCH,
        }
    }

    /// What is said of an address whose last fetch found nothing: that, and why.
    pub fn failed(&self, url: &str) -> Option<String> {
        Some(format!("Not fetched: {}", self.fetches.get(url)?.as_deref()?))
    }

    /// What the page is to write on the previews of a document from now on. For each one whose line
    /// changes: what is said of the failed fetch of its address, or `None` where that holds no more.
    pub fn notes(&mut self, doc: &Document) -> Vec<(BlockId, Option<String>)> {
        if !std::mem::take(&mut self.stale) {
            return Vec::new();
        }
        let failing = self.fetches.values().any(Option::is_some);
        let said = |block: &Block| Some((block.id.clone(), self.failed(address(&block.kind)?)?));
        // The document is gone through only while there is something to say of one of its addresses.
        let now: BTreeMap<BlockId, String> =
            if failing { doc.walk().filter_map(said).collect() } else { BTreeMap::new() };
        let gone = self.noted.keys().filter(|block| !now.contains_key(*block)).map(|block| (block.clone(), None));
        let new = now.iter().filter(|(block, note)| self.noted.get(*block) != Some(*note));
        let changed = gone.chain(new.map(|(block, note)| (block.clone(), Some(note.clone())))).collect();
        self.noted = now;
        changed
    }

    /// Forgets what the page was told to write, and returns the previews it was written on. A sheet says
    /// nothing of a fetch, and the page is told again once it shows.
    pub fn unnoted(&mut self) -> Vec<BlockId> {
        self.stale = true;
        std::mem::take(&mut self.noted).into_keys().collect()
    }

    /// Whether the page at an address is being fetched, so that a press on a control for it asks for
    /// nothing.
    pub fn waits(&self, url: &str) -> bool {
        matches!(self.fetches.get(url), Some(None))
    }

    /// Draws the control of a preview that shows its address alone, at `at`. One whose page is being
    /// fetched says so and takes no press.
    pub fn paint(
        &self,
        (block, url): (&BlockId, &str),
        at: Rect,
        shaper: &dyn TextShaper,
        scheme: Scheme,
        out: &mut DisplayList,
    ) {
        let (waits, over) = (self.waits(url), self.over.as_ref() == Some(&(block.clone(), true)));
        button(self.says(url), (waits, over), at, shaper, scheme, out);
    }

    /// What a button that fetches the page at an address says in a card: as a control does, and that
    /// the page is fetched again where something of it shows already.
    pub fn asks(&self, url: &str, shown: bool) -> &'static str {
        match self.says(url) {
            FETCH if shown => "Fetch again",
            says => says,
        }
    }

    /// The card that asks for the address of a preview to put in, which hangs from the caret at `spot`.
    pub fn asking(&self, spot: Rect) -> Popover<Message> {
        let send = Message::Preview;
        let field = TextField::new("Address", self.address.as_deref().unwrap_or_default())
            .placeholder("Paste or type an address")
            .on_input(move |typed| send(Previewing::Address(typed)))
            .on_submit(send(Previewing::TakeAddress));
        let field = Element::from(field).key("preview-address").takes_focus().width(WIDTH);
        Popover::new(field)
            .label("Link preview")
            .max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
            .at(spot)
            .open(true)
            .on_close(send(Previewing::Close))
    }

    /// The card of a preview, which hangs under it at `spot`. `None` for a block that is no preview.
    pub fn card(&self, kind: &BlockKind, spot: Rect) -> Option<Popover<Message>> {
        let BlockKind::LinkPreview { url, title, description, image } = kind else { return None };
        let send = Message::Preview;
        let edit = move |edit: PreviewEdit| send(Previewing::Edit(edit));

        let typed = self.address.as_deref();
        let address = TextField::new("Address", typed.unwrap_or(url))
            .invalid(typed.is_some_and(|typed| typed.trim().is_empty()))
            .on_input(move |typed| send(Previewing::Address(typed)))
            .on_submit(send(Previewing::TakeAddress))
            .on_leave(send(Previewing::TakeAddress))
            .on_cancel(send(Previewing::Forget));
        let open = Button::icon_only(icon::EXTERNAL_LINK, "Open link")
            .subtle()
            .on_press(Message::Safe(Safely::Follow(typed.unwrap_or(url).trim().to_owned())));
        let address = Element::row()
            .align(Align::Center)
            .gap(tokens::SPACE_XS)
            .child(Element::from(address).grow(1.0))
            .child(open);

        let heading = TextField::new("Title", title.as_deref().unwrap_or_default())
            .placeholder("No title")
            .on_input(move |typed| edit(PreviewEdit::Title(Some(typed))))
            .on_submit(send(Previewing::Close));
        let about = TextField::new("Description", description.as_deref().unwrap_or_default())
            .placeholder("No description")
            .on_input(move |typed| edit(PreviewEdit::Description(Some(typed))))
            .on_submit(send(Previewing::Close));

        // The button says what the control on the card says. A fetch of a preview that shows something
        // already brings what stands over that, so it is asked for as another.
        let fetched = title.is_some() || description.is_some() || image.is_some();
        let fetch = Button::new(self.asks(url, fetched)).small().disabled(self.waits(url));
        let fetch = fetch.on_press(send(Previewing::Fetch));
        let mut buttons = Element::row().align(Align::Center).justify(Justify::Between);
        buttons = buttons.children(self.offered().then_some(fetch));
        if image.is_some() {
            let remove = Button::new("Remove picture").subtle().small();
            buttons = buttons.child(remove.on_press(edit(PreviewEdit::Picture(None))));
        }

        let mut card = Element::column().width(WIDTH).gap(tokens::SPACE_SM).child(address).child(heading).child(about);
        if let Some(failed) = self.failed(url) {
            let quiet = TextStyle::control().size(tokens::TEXT_SM);
            let said = Element::paragraph(failed, quiet).fg(tokens::DANGER).max_width(WIDTH);
            card = card.child(said.role(Role::Status));
        }
        let card = card.children((self.offered() || image.is_some()).then_some(buttons));
        let card = Popover::new(card)
            .label("Link preview")
            .max_width(WIDTH + 2.0 * tokens::SPACE_MD + 2.0)
            .at(spot)
            .open(true)
            .on_close(send(Previewing::Close));
        Some(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOF: &str = "https://example.org/roof";

    const WALL: &str = "https://example.org/wall";

    /// A document of two link previews that lead to the roof's page and one that leads to the wall's.
    fn cards() -> Document {
        let card = |id: &str, url: &str| {
            Block::new(id, BlockKind::LinkPreview { url: url.to_owned(), title: None, description: None, image: None })
        };
        Document { blocks: vec![card("card", ROOF), card("twin", ROOF), card("other", WALL)], ..Document::default() }
    }

    #[test]
    fn a_control_says_how_the_fetch_of_its_address_stands() {
        let mut previews = Previews::default();
        assert_eq!(previews.says(ROOF), FETCH);
        assert!(previews.fetch(ROOF), "the shell is asked");
        assert!(!previews.fetch(ROOF), "and not twice for one fetch");
        assert_eq!((previews.says(ROOF), previews.waits(ROOF)), (FETCHING, true));
        assert_eq!((previews.says(WALL), previews.waits(WALL)), (FETCH, false), "another address is not waited for");

        assert!(!previews.answered(WALL, None), "an answer nothing waits for");
        assert!(previews.answered(ROOF, Some(" the server answered 404 ")));
        assert_eq!((previews.says(ROOF), previews.waits(ROOF)), (AGAIN, false));
        assert_eq!(previews.failed(ROOF).as_deref(), Some("Not fetched: the server answered 404"));

        // Another try forgets why the last one failed, and a page that was found leaves nothing to say.
        assert!(previews.fetch(ROOF));
        assert_eq!(previews.failed(ROOF), None);
        assert!(previews.answered(ROOF, None));
        assert!(!previews.answered(ROOF, None), "an answer is taken once");
        assert_eq!((previews.says(ROOF), previews.waits(ROOF)), (FETCH, false));

        // A shell that cannot fetch is asked for nothing.
        previews.offer(false);
        assert!(!previews.fetch(ROOF));
        assert_eq!(previews.says(ROOF), FETCH);
    }

    #[test]
    fn the_page_is_told_once_what_to_write_on_a_preview_and_once_to_take_it_away() {
        let (mut previews, doc) = (Previews::default(), cards());
        let (block, twin) = (BlockId::new("card"), BlockId::new("twin"));
        let said = |code: u16| Some(format!("Not fetched: the server answered {code}"));
        previews.fetch(ROOF);
        assert_eq!(previews.notes(&doc), [], "nothing is said of a fetch that runs");
        previews.answered(ROOF, Some("the server answered 404"));
        let both = |code: u16| [(block.clone(), said(code)), (twin.clone(), said(code))];
        assert_eq!(previews.notes(&doc), both(404), "on every preview that leads to the address, and no other");
        assert_eq!(previews.notes(&doc), [], "what stands is not told again");

        // Another try takes the lines away, and one that fails for another reason writes that.
        previews.fetch(ROOF);
        assert_eq!(previews.notes(&doc), [(block.clone(), None), (twin.clone(), None)]);
        previews.answered(ROOF, Some("the server answered 500"));
        assert_eq!(previews.notes(&doc), both(500));
        previews.fetch(ROOF);
        previews.answered(ROOF, Some("the server answered 503"));
        assert_eq!(previews.notes(&doc), both(503), "though the page was not told of the try");

        // A preview that is gone, or leads elsewhere by now, has nothing written on it.
        let left = Document { blocks: doc.blocks[1..].to_vec(), ..Document::default() };
        assert_eq!(previews.notes(&left), [], "until the page hears that the document changed");
        previews.edited();
        assert_eq!(previews.notes(&left), [(block.clone(), None)]);
        // What a sheet has taken away is written again on the page.
        assert_eq!(previews.unnoted(), std::slice::from_ref(&twin));
        assert_eq!(previews.notes(&left), [(twin.clone(), said(503))]);
        // A shell that cannot fetch after all leaves nothing to say.
        previews.offer(false);
        assert_eq!(previews.notes(&left), [(twin, None)]);
    }

    #[test]
    fn a_page_says_something_with_any_one_of_a_title_a_description_and_a_picture() {
        let text = |text: &str| Some(text.to_owned());
        let picture = Picture { bytes: vec![1], mime: "image/png".into(), width: 1, height: 1 };
        // What was found, and whether a card can show any of it.
        let cases = [
            (LinkPreviewFound { title: text("Roofs"), ..Default::default() }, true),
            (LinkPreviewFound { description: text("Slate and lead."), ..Default::default() }, true),
            (LinkPreviewFound { picture: Some(picture), ..Default::default() }, true),
            (LinkPreviewFound { title: text(" "), description: text(""), ..Default::default() }, false),
            (LinkPreviewFound::default(), false),
        ];
        for (found, says) in cases {
            assert_eq!(found.says(), says, "{found:?}");
        }
    }

    #[test]
    fn an_address_is_taken_as_typed_without_the_space_round_it() {
        // Whether the card is of a new preview, what was typed, and what is taken.
        let cases = [
            (true, Some(" https://example.org/roof "), Some((None, "https://example.org/roof"))),
            (false, Some("example.org"), Some((Some("card"), "example.org"))),
            (true, Some("   "), None),
            (false, None, None),
        ];
        for (adding, typed, taken) in cases {
            let mut previews = Previews::default();
            match adding {
                true => previews.add(),
                false => previews.show(BlockId::new("card")),
            }
            if let Some(typed) = typed {
                previews.typed(typed.to_owned());
            }
            let taken =
                taken.map(|(block, address): (Option<&str>, &str)| (block.map(BlockId::new), address.to_owned()));
            assert_eq!(previews.take(), taken, "{typed:?}");
            assert_eq!(previews.carded(), !adding, "the card of a new preview goes, that of one stays");
        }
    }

    #[test]
    fn the_control_stands_in_the_middle_of_its_room_and_never_beyond_it() {
        for room in [Rect::new(400.0, 100.0, 128.0, 56.0), Rect::new(400.0, 100.0, 60.0, 56.0)] {
            let at = control(room);
            assert!(at.x >= room.x && at.x + at.width <= room.x + room.width, "{room:?}: {at:?}");
            assert!(at.y >= room.y && at.y + at.height <= room.y + room.height, "{room:?}: {at:?}");
        }
    }
}
