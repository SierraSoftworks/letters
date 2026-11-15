//! Link previews on the page: cards that lead to an address. A press on one opens the card that changes
//! it, and with Ctrl or Cmd held follows it as a link. One that shows its address alone has a control that
//! fetches what the page says of itself.
//!
//! Fetching is the shell's to do: it is asked, and says what it found or why it found nothing. No caret
//! stands in a preview, so the window asks this module what a press on one is on.

use std::collections::BTreeMap;

use accesskit::Role;
use letters_editor::{Editor, Picture, PreviewEdit};
use letters_model::{BlockId, BlockKind};
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
const CONTROL: (f32, f32) = (112.0, 24.0);

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
    /// The previews whose page was asked for: the address that was asked for, and why its fetch found
    /// nothing, in the shell's words, which is `None` while the shell fetches.
    fetches: BTreeMap<BlockId, (String, Option<String>)>,
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

    /// Notes that a preview waits for the page at its address. Returns whether the shell has to be asked:
    /// no fetch of that address runs already, for this preview or for another.
    pub fn fetch(&mut self, block: &BlockId, url: &str) -> bool {
        let asked = self.fetches.values().any(|(at, failed)| at == url && failed.is_none());
        self.fetches.insert(block.clone(), (url.to_owned(), None));
        !asked
    }

    /// Takes the shell's answer for an address: why it found nothing, or `None` when it found the page.
    /// Returns the previews that waited for it.
    pub fn answered(&mut self, url: &str, failed: Option<&str>) -> Vec<BlockId> {
        let waits = |(_, (at, failed)): &(&BlockId, &(String, Option<String>))| at == url && failed.is_none();
        let waited: Vec<BlockId> = self.fetches.iter().filter(waits).map(|(block, _)| block.clone()).collect();
        for block in &waited {
            match failed {
                Some(why) => _ = self.fetches.insert(block.clone(), (url.to_owned(), Some(why.trim().to_owned()))),
                None => _ = self.fetches.remove(block),
            }
        }
        waited
    }

    /// What is said of a page that was read and says nothing a card can show.
    pub fn nothing() -> &'static str {
        NOTHING
    }

    /// Puts the card away once its preview is gone, and forgets the fetch of an address that a preview
    /// leads to no more.
    pub fn follow(&mut self, editor: &Editor) {
        let leads = |block: &BlockId| editor.doc().block(block).and_then(|block| address(&block.kind));
        if self.open.as_ref().is_some_and(|open| leads(open).is_none()) {
            self.close();
        }
        self.fetches.retain(|block, (at, _)| leads(block) == Some(at.as_str()));
    }

    /// What the control of a preview says now.
    pub fn says(&self, block: &BlockId) -> &'static str {
        match self.fetches.get(block) {
            Some((_, None)) => FETCHING,
            Some((_, Some(_))) => AGAIN,
            None => FETCH,
        }
    }

    /// Why the last fetch of a preview found nothing.
    pub fn failed(&self, block: &BlockId) -> Option<&str> {
        self.fetches.get(block).and_then(|(_, failed)| failed.as_deref())
    }

    /// Whether a press on a preview's control asks for anything: not while its page is being fetched.
    pub fn waits(&self, block: &BlockId) -> bool {
        matches!(self.fetches.get(block), Some((_, None)))
    }

    /// Draws the control of a preview that shows its address alone, at `at`, as yewi's small button. One
    /// whose page is being fetched says so and takes no press.
    pub fn paint(&self, block: &BlockId, at: Rect, shaper: &dyn TextShaper, scheme: Scheme, out: &mut DisplayList) {
        let (waits, radius) = (self.waits(block), tokens::RADIUS_CONTROL);
        let over = !waits && self.over.as_ref() == Some(&(block.clone(), true));
        let word = TextStyle::control().size(tokens::TEXT_SM).weight(500);
        let shaped = shaper.shape(self.says(block), &word, None);
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
    pub fn card(&self, block: &BlockId, kind: &BlockKind, spot: Rect) -> Option<Popover<Message>> {
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
        let fetch = match self.says(block) {
            FETCH if fetched => "Fetch again",
            says => says,
        };
        let fetch = Button::new(fetch).small().disabled(self.waits(block)).on_press(send(Previewing::Fetch));
        let mut buttons = Element::row().align(Align::Center).justify(Justify::Between).child(fetch);
        if image.is_some() {
            let remove = Button::new("Remove picture").subtle().small();
            buttons = buttons.child(remove.on_press(edit(PreviewEdit::Picture(None))));
        }

        let mut card = Element::column().width(WIDTH).gap(tokens::SPACE_SM).child(address).child(heading).child(about);
        if let Some(why) = self.failed(block) {
            let quiet = TextStyle::control().size(tokens::TEXT_SM);
            let said = Element::paragraph(format!("Not fetched: {why}"), quiet).fg(tokens::DANGER).max_width(WIDTH);
            card = card.child(said.role(Role::Status));
        }
        let card = Popover::new(card.child(buttons))
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

    #[test]
    fn the_control_says_how_the_fetch_of_its_preview_stands() {
        let (mut previews, block, twin) = (Previews::default(), BlockId::new("card"), BlockId::new("twin"));
        assert_eq!(previews.says(&block), FETCH);
        assert!(previews.fetch(&block, ROOF), "the shell is asked");
        assert!(!previews.fetch(&block, ROOF), "and not twice for one fetch");
        assert!(!previews.fetch(&twin, ROOF), "nor for another preview of the address");
        assert_eq!((previews.says(&block), previews.waits(&block)), (FETCHING, true));

        assert_eq!(previews.answered("https://example.org/wall", None), [], "an answer for another address");
        assert_eq!(previews.answered(ROOF, Some(" the server answered 404 ")), [block.clone(), twin.clone()]);
        for block in [&block, &twin] {
            assert_eq!((previews.says(block), previews.waits(block)), (AGAIN, false));
            assert_eq!(previews.failed(block), Some("the server answered 404"));
        }

        // Another try forgets why the last one failed, and a page that was found leaves nothing to say.
        assert!(previews.fetch(&block, ROOF));
        assert_eq!(previews.failed(&block), None);
        assert_eq!(
            previews.answered(ROOF, None),
            std::slice::from_ref(&block),
            "one that does not wait is not answered"
        );
        assert_eq!((previews.says(&block), previews.waits(&block)), (FETCH, false));
        assert_eq!(previews.says(&twin), AGAIN);
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
