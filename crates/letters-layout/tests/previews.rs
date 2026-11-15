//! Link previews: the card, what it shows of the page it leads to, and the one link it is on a sheet.

use letters_layout::{DocLayout, Palette, PreviewBox};
use letters_model::{Document, Style};
use letters_render::{Item, Rect};

mod common;
use common::*;

const URL: &str = "https://example.org/2026/rates?ref=notice";
const TITLE: &str = "Rates notice";
const SAID: &str = "What every household pays from the first of January.";
const PICTURE: &str = "assets/notice.jpg";

/// What was fetched of a page: its title, its description and its picture.
type Fetched<'a> = [Option<&'a str>; 3];

/// A preview between two paragraphs.
fn carded(fetched: Fetched) -> Document {
    doc(vec![paragraph("a", PROSE), preview("v", URL, fetched), paragraph("z", PROSE)])
}

/// The one link preview a layout shows.
fn card(layout: &DocLayout) -> PreviewBox {
    let mut cards = layout.previews(everything());
    assert_eq!(cards.len(), 1, "the document has one link preview");
    cards.remove(0)
}

/// The slot a card shows its picture in and where the picture is drawn, as painted.
fn pictured(items: &[Item]) -> Option<(Rect, Rect)> {
    let at = items.iter().position(|item| matches!(item, Item::Image { asset, .. } if asset == PICTURE))?;
    match (&items[at - 1], &items[at]) {
        (Item::PushClip { rect: slot, .. }, Item::Image { rect, .. }) => Some((*slot, *rect)),
        other => panic!("a card's picture is cut to its slot: {other:?}"),
    }
}

fn within(inner: Rect, outer: Rect) -> bool {
    let slack = 0.5;
    inner.x >= outer.x - slack
        && inner.y >= outer.y - slack
        && inner.x + inner.width <= outer.x + outer.width + slack
        && inner.y + inner.height <= outer.y + outer.height + slack
}

#[test]
fn a_card_shows_what_was_fetched_of_its_page() {
    let cases: [(&str, Fetched, &[&str]); 5] = [
        ("everything", [Some(TITLE), Some(SAID), Some(PICTURE)], &[TITLE, SAID, "example.org"]),
        ("no picture", [Some(TITLE), Some(SAID), None], &[TITLE, SAID, "example.org"]),
        ("a title alone", [Some(TITLE), None, None], &[TITLE, "example.org"]),
        ("no title", [None, Some(SAID), Some(PICTURE)], &[SAID, "example.org"]),
        ("a picture alone", [None, None, Some(PICTURE)], &["example.org"]),
    ];
    for (name, fetched, says) in cases {
        let layout = laid(&carded(fetched));
        let (card, items) = (card(&layout), painted(&layout, everything()));
        let column = Rect::new(0.0, card.rect.y, WIDTH, card.rect.height);
        assert!(within(card.rect, column) && card.rect.height > 0.0, "{name}: the card is in the column");
        assert!(items.iter().any(|item| matches!(item, Item::Stroke { rect, .. } if *rect == card.rect)), "{name}");
        assert_eq!(card.control, None, "{name}: a preview that was fetched keeps no room for fetching it");

        let slot = pictured(&items).map(|(slot, _)| slot);
        assert_eq!(slot.is_some(), fetched[2].is_some(), "{name}: the picture shows when there is one");
        assert!(slot.is_none_or(|slot| within(slot, card.rect)), "{name}: inside the card");
        let text = reach(&items, card.rect).expect("a card says something");
        let beside = slot.is_none_or(|slot| text.0 >= slot.x + slot.width);
        assert!(beside && text.1 <= card.rect.x + card.rect.width, "{name}: its text stands beside the picture");

        let spelt = spelt(&layout);
        assert!(says.iter().all(|part| spelt.contains(part)), "{name}: it says {says:?} in {spelt:?}");
        assert!(!spelt.contains("2026/rates"), "{name}: of the address only the host shows");
        let (above, below) = (layout.band(&"a".into()).unwrap(), layout.band(&"z".into()).unwrap());
        assert!(above.1 <= card.rect.y && card.rect.y + card.rect.height <= below.1, "{name}: among the blocks");
    }
}

#[test]
fn a_long_description_is_cut_to_a_few_lines() {
    let tall = |said: &str| {
        let layout = laid(&doc(vec![preview("v", URL, [Some(TITLE), Some(said), None])]));
        (card(&layout).rect.height, spelt(&layout))
    };
    let (short, whole) = tall(SAID);
    let (long, cut) = tall(&PROSE.repeat(4));
    let (longer, _) = tall(&PROSE.repeat(40));
    assert!(short < long && long == longer, "a card stops growing with what it says: {short}, {long}, {longer}");
    assert!(cut.contains('\u{2026}') && !whole.contains('\u{2026}'), "and what is cut short says so");
    let starts = cut.matches(&PROSE[..40]).count();
    assert!((1..4).contains(&starts), "it is cut at its end, and no more than it must be: {cut:?}");
}

#[test]
fn a_card_fills_its_slot_with_the_picture_at_its_own_shape() {
    let sizes = [(1200, 630), (400, 900), (300, 300), (1, 5000)];
    let mut slots = Vec::new();
    for size in sizes {
        let mut layout = laid(&carded([Some(TITLE), Some(SAID), Some(PICTURE)]));
        let (slot, unsized_) = pictured(&painted(&layout, everything())).expect("the card has a picture");
        assert_eq!(slot, unsized_, "a picture of unknown size is drawn in its slot");
        assert!(!layout.set_loaded("assets/other.jpg", Some(size)), "another picture's size changes nothing");
        assert!(layout.set_loaded(PICTURE, Some(size)), "its own size is a reason to paint again");

        let (same, drawn) = pictured(&painted(&layout, everything())).unwrap();
        assert_eq!(same, slot, "the slot stays as it is");
        assert!(within(slot, drawn), "the picture covers its slot: {drawn:?} over {slot:?} for {size:?}");
        let (shape, own) = (drawn.width / drawn.height, size.0 as f32 / size.1 as f32);
        assert!((shape / own - 1.0).abs() < 0.01, "at its own shape: {drawn:?} for {size:?}");
        let snug = (drawn.width - slot.width).abs() < 0.5 || (drawn.height - slot.height).abs() < 0.5;
        assert!(snug, "and no larger than that takes: {drawn:?} for {size:?}");
        slots.push(slot);
    }
    assert!(slots.windows(2).all(|pair| pair[0] == pair[1]), "the slot is one size whatever the picture's");
}

#[test]
fn a_preview_with_only_its_address_keeps_room_for_a_control() {
    let layout = laid(&carded([None, None, None]));
    let (card, items) = (card(&layout), painted(&layout, everything()));
    let control = card.control.expect("a preview that was never fetched has room for fetching it");
    let at_end = within(control, card.rect) && control.x + control.width > card.rect.x + card.rect.width - 24.0;
    assert!(at_end && control.width > 40.0, "the room is at the end of the card: {control:?} in {:?}", card.rect);
    let text = reach(&items, card.rect).expect("the address shows");
    assert!(text.1 < control.x, "the address stops short of it: {text:?} against {control:?}");
    assert!(spelt(&layout).contains(URL), "the whole address shows");
    assert!(pictured(&items).is_none());

    let long = format!("https://example.org/{}", "a-long-path/".repeat(60));
    let run_on = laid(&doc(vec![preview("v", &long, [None, None, None])]));
    let grown = self::card(&run_on).rect.height;
    assert!(grown < card.rect.height * 2.5, "an address that runs on is cut to a line or two: {grown}");
}

#[test]
fn a_note_is_written_under_an_address_alone() {
    const NOTE: &str = "Not fetched: the server answered 404";
    let (id, muted, unfetched) = ("v".into(), Palette::LIGHT.marker, carded([None, None, None]));
    let below = |layout: &DocLayout| layout.band(&"z".into()).expect("the paragraph under the card").0;
    let mut layout = laid(&unfetched);
    let (bare, page, under) = (card(&layout), painted(&layout, everything()), below(&layout));
    assert!(inked(&page, muted).is_empty(), "nothing on the page is muted before");

    assert!(layout.set_preview_note(&id, Some(NOTE)), "the card is laid out anew for a note");
    assert!(!layout.set_preview_note(&id, Some(NOTE)), "and not for the same note again");
    let (noted, items) = (card(&layout), painted(&layout, everything()));
    let grown = noted.rect.height - bare.rect.height;
    assert!(grown > 8.0, "the card grows by the note's line: {grown}");
    assert!((below(&layout) - under - grown).abs() < 0.5, "and what is under it moves down as far");
    assert!(!inked(&items, muted).is_empty(), "the note is written, muted");
    let (was, control) = (bare.control.unwrap(), noted.control.expect("the control keeps its room"));
    assert_eq!((control.x, control.width), (was.x, was.width), "where it was");
    let text = reach(&items, noted.rect).expect("the card's text");
    assert!(control.height > was.height && text.1 < control.x, "beside the note too: {text:?}, {control:?}");
    assert!(!spelt(&layout).contains("Not fetched"), "a sheet shows the document");

    // A layout that starts over, as at another width, still has it.
    layout.update(&unfetched, &Style::default(), WIDTH - 120.0, None);
    assert!(!inked(&painted(&layout, everything()), muted).is_empty(), "the note outlives a new layout");
    layout.update(&unfetched, &Style::default(), WIDTH, None);
    assert!(layout.set_preview_note(&id, None), "the card is laid out anew when the note goes");
    assert!(painted(&layout, everything()) == page, "and the page is as it was");

    let mut filled = laid(&carded([Some(TITLE), Some(SAID), None]));
    let page = painted(&filled, everything());
    assert!(!filled.set_preview_note(&id, Some(NOTE)), "a card that shows its page takes no note");
    assert!(painted(&filled, everything()) == page, "and shows none");
}

#[test]
fn a_card_is_one_link_on_a_sheet() {
    // Prose enough above the card that it would straddle two small sheets.
    let blocks = vec![
        paragraph("a", &PROSE.repeat(3)),
        preview("v", URL, [Some(TITLE), Some(&PROSE.repeat(3)), Some(PICTURE)]),
        paragraph("z", PROSE),
    ];
    let layout = laid_at(&doc(blocks), 400.0);
    let pages = layout.paginate(&paper(400.0, 260.0));
    assert!(pages.sheets.len() > 1, "the document runs over sheets");
    let mut found = Vec::new();
    for index in 0..pages.sheets.len() {
        let (items, origin) = (sheet(&layout, &pages, index), pages.origin(index).unwrap());
        // A picture is decoration: it is part of nothing the sheet tells of.
        let mut open = 0;
        for item in &items {
            match item {
                Item::Open { .. } => open += 1,
                Item::Close => open -= 1,
                Item::Image { .. } => assert_eq!(open, 0, "the card's picture is part of nothing"),
                _ => {}
            }
        }
        for link in layout.page_links(&pages, index) {
            let card = card(&layout).rect;
            let on_sheet = Rect { x: card.x + origin.x, y: card.y + origin.y, ..card };
            assert_eq!(link.rect, on_sheet, "the link is the whole card");
            let (top, bottom) = (pages.area.y, pages.area.y + pages.area.height);
            assert!(link.rect.y >= top - 0.5 && link.rect.y + link.rect.height <= bottom + 0.5, "whole on its sheet");
            assert_eq!((link.target.as_str(), link.place), (URL, None), "it leads to the address");
            let spelt = links(&items);
            let part = spelt.iter().find(|(id, _)| Some(*id) == link.part).expect("the link names its part");
            assert!(part.1.contains(TITLE) && part.1.contains("example.org"), "which is all the card says: {part:?}");
            found.push(index);
        }
    }
    assert_eq!(found.len(), 1, "the card is one link, on one sheet: {found:?}");
}
