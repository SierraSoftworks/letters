//! Link previews on the page: how one is put in, fetched, changed and followed, and what a link in text
//! shows of its page.

use accesskit::{Action, ActionRequest, TreeId};
use letters_editor::Picture;
use letters_model::{Block, BlockKind, Body, Document, Pos, Preview, Run, Selection};
use letters_render::Point;
use letters_test::{Found, Harness, Role};
use letters_ui::{Access, LinkPreviewFound, Request, SaveState};

const QUOTE: &str = "https://roofers.example.org/quotes/spring";
const GUIDE: &str = "https://slate.example.org/guides/welsh";
const EDIT: &str = "Edit link preview";
const FETCH: &str = "Fetch preview";
const CARD: &str = "Link preview";

fn preview(url: &str, title: Option<&str>, image: Option<&str>) -> BlockKind {
    let (title, image) = (title.map(str::to_owned), image.map(str::to_owned));
    BlockKind::LinkPreview { url: url.to_owned(), title, description: None, image }
}

/// A note with a link preview, `card`, between two lines, and the caret in the first line.
fn note(kind: BlockKind) -> Harness {
    notes(vec![Block::new("card", kind)])
}

/// A note with these blocks between two lines, and the caret in the first line.
fn notes(cards: Vec<Block>) -> Harness {
    let mut blocks = vec![Block::text("before", BlockKind::Paragraph, "The roofer sent a quote.")];
    blocks.extend(cards);
    blocks.push(Block::text("after", BlockKind::Paragraph, "Reply by Friday."));
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.paint();
    window
}

/// A note whose preview shows its address alone.
fn bare() -> Harness {
    note(preview(QUOTE, None, None))
}

/// A preview as the document has it: its address, its title, its description and its picture.
type Shown = (String, Option<String>, Option<String>, Option<String>);

/// The previews of the note as the document has them, from the top.
fn all_shown(window: &Harness) -> Vec<Shown> {
    let preview = |block: &Block| match &block.kind {
        BlockKind::LinkPreview { url, title, description, image } => {
            Some((url.clone(), title.clone(), description.clone(), image.clone()))
        }
        _ => None,
    };
    window.editor().doc().walk().filter_map(preview).collect()
}

/// The first preview of the note.
fn shown(window: &Harness) -> Shown {
    all_shown(window).into_iter().next().expect("the note has a preview")
}

/// The addresses the shell is asked to fetch, since this was last called.
fn fetches(window: &mut Harness) -> Vec<String> {
    let fetch = |request| match request {
        Request::FetchPreview { url } => Some(url),
        _ => None,
    };
    window.ui().take_requests().into_iter().filter_map(fetch).collect()
}

/// What a shell found of the page at an address.
fn found(url: &str, title: Option<&str>, description: Option<&str>) -> LinkPreviewFound {
    let (title, description) = (title.map(str::to_owned), description.map(str::to_owned));
    LinkPreviewFound { url: url.to_owned(), title, description, picture: None }
}

/// What the card of the preview says of the last fetch.
fn status(window: &mut Harness) -> Option<String> {
    let said = window.controls().into_iter().find(|found| found.role == Role::Status)?;
    said.value.or(Some(said.name))
}

/// The field of the card with a name, as it reads.
fn field(window: &mut Harness, name: &str) -> Option<String> {
    window.find(Role::TextInput, name).map(|field| field.value.unwrap_or_default())
}

/// A line that links to the guide, in a document that keeps this of the guide's page.
fn linked(kept: Option<Preview>) -> Harness {
    linked_to(GUIDE, kept)
}

/// A line with a link to an address, in a document that keeps this of the page there.
fn linked_to(url: &str, kept: Option<Preview>) -> Harness {
    let link = Run { link: Some(url.to_owned()), ..Run::plain("the quarries of Gwynedd") };
    let runs = vec![Run::plain("Slate from "), link, Run::plain(" lasts a century.")];
    let line = Block { body: Body::Text(runs), ..Block::new("line", BlockKind::Paragraph) };
    let previews = kept.into_iter().map(|kept| (url.to_owned(), kept)).collect();
    let mut window = Harness::open(Document { blocks: vec![line], previews, ..Document::default() });
    window.paint();
    window
}

/// What the guide's page says of itself.
fn guide() -> Preview {
    let (title, description) =
        (Some("Choosing Welsh slate".to_owned()), Some("How to tell the quarries apart.".to_owned()));
    Preview { title, description, image: Some("assets/quarry.png".to_owned()), ..Preview::default() }
}

/// A point on the link of [`linked`].
fn link(window: &mut Harness) -> Point {
    window.ui().select(Selection::caret(Pos::new("line", "Slate from ".len())));
    window.paint();
    let start = window.ui().caret_rect().expect("the page shows a caret");
    Point { x: start.x + 12.0, y: start.y + start.height / 2.0 }
}

/// The card that shows over the page once the pointer has rested where it is.
fn rested(window: &mut Harness) -> Option<Found> {
    window.advance(0).advance(1_000);
    window.controls().into_iter().find(|found| found.role == Role::Tooltip)
}

#[test]
fn a_preview_put_in_shows_its_address_and_asks_the_shell_for_its_page() {
    // How it is asked for on an empty line.
    type Ask = fn(&mut Harness);
    let ways: [(&str, Ask); 2] = [
        ("the insert menu", |window| _ = window.type_text("/link").key("Enter")),
        ("the commands", |window| _ = window.key("Mod+Shift+P").type_text("Link preview").key("Enter")),
    ];
    for (way, ask) in ways {
        let blocks = vec![Block::text("p", BlockKind::Paragraph, "")];
        let mut window = Harness::open(Document { blocks, ..Document::default() });
        window.ui().set_save_state(SaveState::Unsaved);
        ask(&mut window);
        window.assert_open(Role::Dialog, CARD);
        assert_eq!(window.editor().doc().blocks.len(), 1, "{way}: nothing is put in before there is an address");

        window.type_text(&format!(" {QUOTE} ")).key("Enter").assert_closed(Role::Dialog, CARD);
        assert_eq!(shown(&window), (QUOTE.to_owned(), None, None, None), "{way}");
        assert_eq!(fetches(&mut window), [QUOTE], "{way}: its page is asked for once");
        window.assert_open(Role::Button, "Fetching");
        window.type_text("Roof");
        assert_eq!(window.markdown().matches("Roof").count(), 1, "{way}: writing goes on in the line below");
    }

    // Enter with no address, or Escape, puts nothing in.
    for leave in ["Enter", "Escape"] {
        let blocks = vec![Block::text("p", BlockKind::Paragraph, "")];
        let mut window = Harness::open(Document { blocks, ..Document::default() });
        window.ui().set_save_state(SaveState::Unsaved);
        window.type_text("/link").key("Enter").key(leave).assert_closed(Role::Dialog, CARD);
        assert_eq!(window.editor().doc().blocks.len(), 1, "{leave}");
        assert_eq!(fetches(&mut window), [""; 0], "{leave}");
    }
}

#[test]
fn what_the_shell_finds_fills_the_preview_in_one_step() {
    let mut window = bare();
    window.click(Role::Button, FETCH);
    assert_eq!(fetches(&mut window), [QUOTE]);
    // A second press while the page is fetched asks for nothing more.
    window.click(Role::Button, "Fetching");
    assert_eq!(fetches(&mut window), [""; 0], "one fetch at a time");
    assert_eq!(window.editor().selection().map(|caret| caret.head.block.as_str()), Some("before"), "the caret stays");

    let picture = Picture {
        bytes: vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        mime: "image/png".into(),
        width: 400,
        height: 200,
    };
    let page = LinkPreviewFound {
        picture: Some(picture),
        ..found(QUOTE, Some("Spring quotes"), Some("  Slate and\n lead.  "))
    };
    window.fill_preview(page);
    let (url, title, description, image) = shown(&window);
    assert_eq!(
        (url.as_str(), title.as_deref(), description.as_deref()),
        (QUOTE, Some("Spring quotes"), Some("Slate and lead."))
    );
    assert!(image.is_some(), "the page's picture is kept in the document");
    window.assert_closed(Role::Button, FETCH).assert_closed(Role::Button, "Fetching");

    window.key("Mod+Z");
    assert_eq!(shown(&window), (QUOTE.to_owned(), None, None, None), "undo takes all of it back at once");
}

#[test]
fn what_is_found_leaves_the_page_where_it_is_read() {
    // A note far taller than its window, with the caret at its top and its preview at its end.
    let long = |id: &str| Block::text(id, BlockKind::Paragraph, "Estimates so far. ".repeat(80));
    let mut window = notes(vec![long("p1"), long("p2"), long("p3"), Block::new("card", preview(QUOTE, None, None))]);
    window.resize(900.0, 600.0);
    window.scroll_at(Point { x: 450.0, y: 300.0 }, 1e6).paint();
    window.click(Role::Button, FETCH).fill_preview(found(QUOTE, Some("Spring quotes"), None));
    window.paint();
    assert_eq!(shown(&window).1.as_deref(), Some("Spring quotes"));
    window.assert_open(Role::Button, EDIT);
}

#[test]
fn a_fetch_that_finds_nothing_says_why_and_offers_another_try() {
    // What the shell answers, and what is then said of the preview.
    type Answer = fn(&mut Harness);
    let cases: [(&str, Answer, &str); 2] = [
        (
            "the fetch failed",
            |window| window.ui().fail_preview(QUOTE, "the server answered 404"),
            "Not fetched: the server answered 404",
        ),
        (
            "the page says nothing of itself",
            |window| _ = window.fill_preview(found(QUOTE, None, Some("  "))),
            "Not fetched: the page has no title, description or picture",
        ),
    ];
    // How tall the preview stands on the page.
    let tall = |window: &mut Harness| window.find(Role::Button, EDIT).expect("the preview").bounds.height;
    for (case, answer, said) in cases {
        let mut window = bare();
        let alone = tall(&mut window);
        window.click(Role::Button, FETCH);
        fetches(&mut window);
        answer(&mut window);
        assert_eq!(shown(&window), (QUOTE.to_owned(), None, None, None), "{case}: the preview is as it was");
        assert!(tall(&mut window) > alone, "{case}: a line under its address says why");
        let again = window.find(Role::Button, "Try again").unwrap_or_else(|| panic!("{case}: another try is offered"));
        assert_eq!(again.description.as_deref(), Some(said), "{case}");
        // The preview says it too, to whoever does not see the line under it.
        let card = |window: &mut Harness| window.find(Role::Link, QUOTE).expect("the preview").description;
        assert_eq!(card(&mut window).as_deref(), Some(said), "{case}");
        // The card of the preview says it too.
        window.click(Role::Button, EDIT);
        assert_eq!(status(&mut window).as_deref(), Some(said), "{case}");
        window.key("Escape");

        window.click(Role::Button, "Try again");
        assert_eq!(fetches(&mut window), [QUOTE], "{case}");
        window.assert_open(Role::Button, "Fetching");
        assert_eq!(card(&mut window), None, "{case}: nothing is said of a try that is on its way");
        assert_eq!(tall(&mut window), alone, "{case}: the line goes for another try");
    }
}

#[test]
fn someone_listening_is_told_again_of_a_preview_whose_fetch_failed() {
    // What each update says with the preview, where it tells the preview at all.
    let said = |update: &accesskit::TreeUpdate| -> Vec<Option<String>> {
        let preview = |node: &accesskit::Node| node.role() == Role::Link && node.label() == Some(QUOTE);
        let told = update.nodes.iter().filter(|(_, node)| preview(node));
        told.map(|(_, node)| node.description().map(str::to_owned)).collect()
    };
    let mut window = bare();
    window.click(Role::Button, FETCH);
    assert_eq!(said(&window.ui().access()), [None], "nothing is said of a try that is on its way");
    window.ui().fail_preview(QUOTE, "the server answered 404");
    let why = "Not fetched: the server answered 404".to_owned();
    assert_eq!(said(&window.ui().access_changes()), [Some(why)], "the preview reads as it did, and says more");
}

#[test]
fn print_view_shows_a_preview_as_it_prints_whatever_is_said_of_its_fetch() {
    // A note in Print view after its preview was asked for, and what the shell answered if it did.
    let printing = |failed: Option<&str>| {
        let mut window = bare();
        window.click(Role::Button, FETCH);
        if let Some(why) = failed {
            window.ui().fail_preview(QUOTE, why);
        }
        let tall = window.find(Role::Button, EDIT).expect("the preview").bounds.height;
        window.key("Mod+P");
        (window, tall)
    };
    let ((mut waiting, alone), (mut failed, said)) = (printing(None), printing(Some("the server answered 404")));
    assert!(said > alone, "the page says why");
    assert!(failed.paint() == waiting.paint(), "a sheet says nothing of a fetch and keeps no room for it");

    failed.key("Escape");
    let back = failed.find(Role::Button, EDIT).expect("the preview").bounds.height;
    assert_eq!(back, said, "the page says it again");
}

#[test]
fn what_is_found_goes_to_every_preview_that_leads_to_the_address_and_to_no_other() {
    let bare = |id: &str, url: &str| Block::new(id, preview(url, None, None));
    let mut window = notes(vec![bare("card", QUOTE), bare("twin", QUOTE), bare("other", GUIDE)]);
    let controls = window.controls().into_iter().filter(|found| found.name == FETCH);
    for at in controls.map(|control| control.center()).collect::<Vec<_>>() {
        window.click_at(at);
    }
    assert_eq!(fetches(&mut window), [QUOTE, GUIDE], "an address is asked for once, however many wait for it");

    // The third preview is given another address while its page is fetched: that page is not for it any
    // more, and it offers a fetch of where it leads now.
    let third = window.controls().into_iter().filter(|found| found.name == EDIT).nth(2).expect("three previews");
    let elsewhere = "https://slate.example.org/guides/cornish";
    window.click_at(third.center()).click(Role::TextInput, "Address").key("Mod+A").type_text(elsewhere);
    window.key("Enter").key("Escape").click(Role::Button, FETCH);
    assert_eq!(fetches(&mut window), [elsewhere]);

    window.fill_preview(found(QUOTE, Some("Spring quotes"), None));
    window.fill_preview(found(GUIDE, Some("Welsh slate"), None));
    let titles: Vec<_> = all_shown(&window).into_iter().map(|(url, title, ..)| (url, title)).collect();
    let quote = (QUOTE.to_owned(), Some("Spring quotes".to_owned()));
    assert_eq!(titles, [quote.clone(), quote.clone(), (elsewhere.to_owned(), None)]);

    // An answer that nothing waits for any more is not taken.
    window.fill_preview(found(QUOTE, Some("Autumn quotes"), None));
    let titles: Vec<_> = all_shown(&window).into_iter().map(|(url, title, ..)| (url, title)).collect();
    assert_eq!(titles, [quote.clone(), quote, (elsewhere.to_owned(), None)]);
}

/// What the document keeps of the guide's page for the links in its text: the page's title.
fn kept(window: &Harness) -> Option<String> {
    window.editor().doc().previews.get(GUIDE).and_then(|kept| kept.title.clone())
}

#[test]
fn the_page_of_an_address_put_into_the_text_is_asked_for_at_once() {
    type Put = fn(&mut Harness);
    // How the address comes to be in the text, and whether it stands as a card of its own then.
    let ways: [(&str, Put, bool); 3] = [
        ("typed in a line of text", |window| _ = window.type_text(&format!("See {GUIDE} ")), false),
        (
            "set on a passage",
            |window| _ = window.type_text("See the guide").select("guide").key("Mod+K").type_text(GUIDE).key("Enter"),
            false,
        ),
        ("typed on a line of its own", |window| _ = window.type_text(GUIDE).key("Enter"), true),
    ];
    for (way, put, card) in ways {
        let blocks = vec![Block::text("p", BlockKind::Paragraph, "")];
        let mut window = Harness::open(Document { blocks, ..Document::default() });
        window.ui().set_save_state(SaveState::Unsaved);
        put(&mut window);
        assert_eq!(all_shown(&window).len(), usize::from(card), "{way}: a card stands for a link alone on its line");
        assert_eq!(fetches(&mut window), [GUIDE], "{way}: its page is asked for once");

        // What is found is the card's, or kept for the link, and one undo takes it back.
        let title = |window: &Harness| if card { shown(window).1 } else { kept(window) };
        window.fill_preview(found(GUIDE, Some("Choosing Welsh slate"), None));
        assert_eq!(title(&window).as_deref(), Some("Choosing Welsh slate"), "{way}");
        window.key("Escape").key("Mod+Z");
        assert_eq!(title(&window), None, "{way}: undo takes what was found back at once");
    }
}

#[test]
fn a_shell_that_cannot_fetch_is_asked_for_nothing_and_offers_no_fetch() {
    let blocks = vec![Block::text("p", BlockKind::Paragraph, "")];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.ui().set_save_state(SaveState::Unsaved);
    window.ui().set_preview_fetching(false);
    window.type_text("/link").key("Enter").type_text(QUOTE).key("Enter");
    assert_eq!(shown(&window), (QUOTE.to_owned(), None, None, None), "a preview is put in all the same");
    assert_eq!(fetches(&mut window), [""; 0]);

    // Nothing on the preview or in its card offers a fetch, and the rest of the card is there.
    let offers = |window: &mut Harness| {
        let fetch =
            |found: &Found| found.role == Role::Button && (found.name.contains("etch") || found.name == "Try again");
        window.controls().into_iter().filter(fetch).map(|found| found.name).collect::<Vec<_>>()
    };
    assert_eq!(offers(&mut window), [""; 0]);
    window.click(Role::Button, EDIT).assert_open(Role::Dialog, CARD);
    assert_eq!(offers(&mut window), [""; 0]);
    window.click(Role::TextInput, "Title").type_text("Spring quotes");
    assert_eq!(shown(&window).1.as_deref(), Some("Spring quotes"));

    // An address typed on a line of its own stays a link in the text: a card would have nothing to show.
    window.key("Escape").type_text(GUIDE).key("Enter");
    assert_eq!(all_shown(&window).len(), 1, "no card is made of it");
    assert!(window.markdown().contains(GUIDE), "it is written all the same");
    assert_eq!(fetches(&mut window), [""; 0]);
}

#[test]
fn a_press_on_a_preview_opens_a_card_that_changes_it() {
    let mut window = note(preview(GUIDE, Some("Welsh slate"), Some("assets/slate.png")));
    window.click(Role::Button, EDIT).assert_open(Role::Dialog, CARD);
    assert_eq!(field(&mut window, "Address").as_deref(), Some(GUIDE));
    assert_eq!(field(&mut window, "Title").as_deref(), Some("Welsh slate"));
    assert_eq!(fetches(&mut window), [""; 0], "opening the card fetches nothing");

    // What is typed for the title and the description shows on the page at once.
    window.click(Role::TextInput, "Title").key("Mod+A").type_text("Choosing slate");
    window.click(Role::TextInput, "Description").type_text("How to tell the quarries apart.");
    let (_, title, description, image) = shown(&window);
    assert_eq!(title.as_deref(), Some("Choosing slate"));
    assert_eq!(description.as_deref(), Some("How to tell the quarries apart."));
    assert_eq!(image.as_deref(), Some("assets/slate.png"));

    // The address is taken when it is done, with Enter or by going on to another field, and one with
    // nothing in it is not.
    window.click(Role::TextInput, "Address").key("Mod+A").type_text(QUOTE);
    assert_eq!(shown(&window).0, GUIDE, "not while it is typed");
    window.key("Enter");
    assert_eq!(shown(&window).0, QUOTE);
    window.click(Role::TextInput, "Address").key("Mod+A").type_text(GUIDE).click(Role::TextInput, "Title");
    assert_eq!(shown(&window).0, GUIDE, "going on to another field takes it too");
    window.click(Role::TextInput, "Address").key("Mod+A").key("Backspace").key("Enter");
    assert_eq!(shown(&window).0, GUIDE, "a preview always leads somewhere");

    window.click(Role::Button, "Remove picture");
    assert_eq!(shown(&window).3, None);
    window.assert_closed(Role::Button, "Remove picture").assert_open(Role::Dialog, CARD);

    // Another fetch is asked for by name, and is the shell's to do.
    window.click(Role::Button, "Fetch again");
    assert_eq!(fetches(&mut window), [GUIDE]);
    window.key("Escape").assert_closed(Role::Dialog, CARD);
}

#[test]
fn a_preview_is_followed_as_a_link_with_ctrl_or_cmd_held() {
    let mut window = bare();
    let card = window.find(Role::Button, EDIT).expect("the preview").center();
    window.mod_click_at(card);
    let asked = window.ui().take_requests();
    assert_eq!(asked, [Request::OpenLink { href: QUOTE.to_owned(), confirmed: false }]);
    window.assert_closed(Role::Dialog, CARD);
}

#[test]
fn the_card_goes_with_its_preview() {
    let mut window = bare();
    window.click(Role::Button, FETCH).click(Role::Button, EDIT).assert_open(Role::Dialog, CARD);
    window.key("Escape").key("Mod+A").key("Backspace");
    assert!(window.editor().doc().block(&"card".into()).is_none(), "the preview is gone with the rest");
    // What was found for a preview that is gone goes nowhere.
    window.fill_preview(found(QUOTE, Some("Spring quotes"), None));
    window.assert_closed(Role::Button, EDIT).assert_closed(Role::Dialog, CARD);
    assert!(!window.markdown().contains("Spring quotes"));
}

#[test]
fn assistive_technology_presses_a_preview_and_its_control() {
    // A link in the text whose card shows, with nothing kept of its page.
    let resting = || {
        let mut window = linked(None);
        let at = link(&mut window);
        window.hover_at(at);
        rested(&mut window).expect("the card of the link");
        window
    };
    // What is pressed, by name, and whether the page is then asked for.
    let cases = [(bare(), EDIT, false), (bare(), FETCH, true), (resting(), FETCH, true)];
    for (mut window, name, fetched) in cases {
        let tree = window.ui().access();
        let named = |node: &accesskit::Node| node.role() == Role::Button && node.label() == Some(name);
        let (target_node, node) = tree.nodes.iter().find(|(_, node)| named(node)).expect("it is told");
        assert!(node.supports_action(Action::Click), "{name} says that it can be pressed");
        let target_node = *target_node;
        window.ask(ActionRequest { action: Action::Click, target_tree: TreeId::ROOT, target_node, data: None });
        assert_eq!(fetches(&mut window).len(), usize::from(fetched), "{name}");
        assert_eq!(window.find(Role::Dialog, CARD).is_some(), !fetched, "{name}: the card is out");
    }
}

#[test]
fn a_link_in_text_shows_what_is_kept_of_its_page_while_the_pointer_rests_on_it() {
    let mut window = linked(Some(guide()));
    let at = link(&mut window);
    window.ui().take_requests();
    window.hover_at(at).advance(0);
    assert!(
        window.find(Role::Tooltip, "").is_none() && window.controls().iter().all(|found| found.role != Role::Tooltip)
    );

    let card = rested(&mut window).expect("a card, once the pointer has rested");
    for said in ["Choosing Welsh slate", "How to tell the quarries apart.", "slate.example.org"] {
        assert!(card.name.contains(said), "{said}: {:?}", card.name);
    }
    let beside = (card.bounds.y - at.y).abs().min((card.bounds.y + card.bounds.height - at.y).abs());
    assert!(beside < 24.0 && !card.bounds.contains(at), "beside the link, not over it: {:?}", card.bounds);
    // Showing it reads no page: its picture is the document's own, asked for as any picture of the page.
    let asked = window.ui().take_requests();
    assert!(asked.contains(&Request::Asset("assets/quarry.png".to_owned())), "{asked:?}");
    assert_eq!(asked.len(), 1, "and nothing else is asked for: {asked:?}");

    // The card goes with the pointer, and when the person writes.
    window.hover_at(Point { x: at.x, y: at.y + 200.0 });
    assert_eq!(rested(&mut window), None, "off the link");
    window.hover_at(at);
    assert!(rested(&mut window).is_some());
    window.type_text("x");
    assert_eq!(rested(&mut window), None, "while writing");
}

#[test]
fn the_card_of_a_link_offers_the_fetch_of_a_page_the_document_keeps_nothing_of() {
    for kept in [None, Some(Preview::default())] {
        let mut window = linked(kept.clone());
        let at = link(&mut window);
        window.hover_at(at);
        let card = rested(&mut window).expect("a card that says where the link leads");
        assert!(card.name.contains("slate.example.org"), "{kept:?}: {:?}", card.name);
        assert_eq!(fetches(&mut window), [""; 0], "{kept:?}: resting on a link fetches nothing");

        // The card stays while the pointer goes to its button, and only a press there asks for the page.
        let button = window.find(Role::Button, FETCH).expect("a button on the card");
        assert!(card.bounds.contains(button.center()), "{:?} on {:?}", button.bounds, card.bounds);
        window.click_at(Point { x: card.bounds.x + 3.0, y: card.bounds.y + 3.0 });
        assert_eq!(fetches(&mut window), [""; 0], "{kept:?}: a press beside the button");
        window.click_at(button.center());
        assert_eq!(fetches(&mut window), [GUIDE], "{kept:?}");
        assert_eq!(window.editor().selection().map(|caret| caret.head.offset), Some(11), "the caret stays");
        window.assert_closed(Role::Button, FETCH);
        assert!(rested(&mut window).is_some(), "{kept:?}: the card says that the page is fetched");

        // A fetch that found nothing says why, and can be tried again.
        window.ui().fail_preview(GUIDE, "the server answered 404");
        let card = rested(&mut window).expect("the card is still there");
        assert!(card.name.contains("Not fetched: the server answered 404"), "{kept:?}: {:?}", card.name);
        window.click(Role::Button, "Try again");
        assert_eq!(fetches(&mut window), [GUIDE], "{kept:?}");

        // What is found is kept for the link, and its card shows it from then on.
        window.fill_preview(found(GUIDE, Some("Choosing Welsh slate"), None));
        window.hover_at(Point { x: at.x, y: at.y + 200.0 }).hover_at(at);
        let card = rested(&mut window).expect("the card of the link");
        assert!(card.name.contains("Choosing Welsh slate"), "{kept:?}: {:?}", card.name);
        window.assert_closed(Role::Button, FETCH).assert_closed(Role::Button, "Try again");
    }
}

#[test]
fn a_link_has_no_card_where_nothing_is_kept_of_its_page_and_none_can_be_fetched() {
    // Where the link leads, and whether the shell fetches pages: only a web page is fetched.
    let cases = [(GUIDE, false), ("mailto:quarry@example.org", true), ("file:///srv/quarry/notes.html", true)];
    for (url, fetching) in cases {
        let mut window = linked_to(url, None);
        window.ui().set_preview_fetching(fetching);
        let at = link(&mut window);
        window.hover_at(at);
        assert_eq!(rested(&mut window), None, "{url}");
        window.key("Mod+K").assert_open(Role::Dialog, "Link").assert_closed(Role::Button, FETCH);
        assert_eq!(fetches(&mut window), [""; 0], "{url}");
    }
}

#[test]
fn the_card_that_edits_a_link_fetches_its_page() {
    let mut window = linked(None);
    link(&mut window);
    window.key("Mod+K").assert_open(Role::Dialog, "Link");
    window.click(Role::Button, FETCH);
    assert_eq!(fetches(&mut window), [GUIDE]);
    assert_eq!(window.find(Role::Button, "Fetching").map(|button| button.disabled), Some(true));

    window.ui().fail_preview(GUIDE, "the server answered 404");
    assert_eq!(status(&mut window).as_deref(), Some("Not fetched: the server answered 404"));
    window.click(Role::Button, "Try again");
    assert_eq!(fetches(&mut window), [GUIDE]);
    window.fill_preview(found(GUIDE, Some("Choosing Welsh slate"), None));
    assert_eq!(kept(&window).as_deref(), Some("Choosing Welsh slate"));

    // The card reads what was found, and offers it again. Another address in the field is not the link's
    // yet, so there is nothing of it to fetch.
    window.key("Mod+K").assert_open(Role::Label, "Choosing Welsh slate").assert_open(Role::Button, "Fetch again");
    window.type_text(QUOTE).assert_closed(Role::Button, "Fetch again").assert_closed(Role::Button, FETCH);
}

#[test]
fn the_card_of_a_link_reads_what_is_kept_of_its_page() {
    let mut window = linked(Some(guide()));
    let at = link(&mut window);
    window.key("Mod+K").assert_open(Role::Dialog, "Link");
    assert_eq!(field(&mut window, "Link").as_deref(), Some(GUIDE));
    window.assert_open(Role::Label, "Choosing Welsh slate").assert_open(Role::Label, "How to tell the quarries apart.");
    // The pointer raises no second card while this one hangs from the page.
    window.hover_at(at);
    assert_eq!(rested(&mut window), None);
    // What is kept is of the address in the field: another address has nothing to show yet.
    window.type_text("x").assert_closed(Role::Label, "Choosing Welsh slate");
}

const AS_CARD: &str = "Show link as card";
const AS_LINK: &str = "Show card as link";

/// A note whose middle line, `line`, is a link to the guide and nothing else.
fn lone() -> Harness {
    let link = Run { link: Some(GUIDE.to_owned()), ..Run::plain("the guide to Welsh slate") };
    notes(vec![Block { body: Body::Text(vec![link]), ..Block::new("line", BlockKind::Paragraph) }])
}

/// Puts the caret in a block of the note.
fn caret(window: &mut Harness, block: &str, offset: usize) {
    window.ui().select(Selection::caret(Pos::new(block, offset)));
    window.paint();
}

/// Which of the two commands that show a link as a card, or a card as a link, the palette lists.
fn offered(window: &mut Harness) -> Vec<String> {
    window.key("Mod+Shift+P").type_text("show as");
    let listed = window.controls().into_iter().filter(|found| found.role == Role::ListBoxOption);
    let names = listed.map(|found| found.name).collect();
    window.key("Escape");
    names
}

/// Takes a command from the palette by its name.
fn command(window: &mut Harness, name: &str) {
    window.key("Mod+Shift+P").type_text(name).key("Enter");
}

/// The addresses the links in the note's text lead to.
fn links(window: &Harness) -> Vec<String> {
    let runs = window.editor().doc().walk().flat_map(|block| block.body.runs());
    runs.filter_map(|run| run.link.clone()).collect()
}

#[test]
fn a_link_alone_on_its_line_is_shown_as_a_card_and_as_a_link_again_from_the_palette() {
    let mut window = lone();
    caret(&mut window, "line", 4);
    command(&mut window, AS_CARD);
    assert_eq!(all_shown(&window), [(GUIDE.to_owned(), None, None, None)], "the line is a card that leads there");
    assert_eq!(links(&window), [""; 0], "and no line of text any more");
    assert_eq!(fetches(&mut window), [GUIDE], "its page is asked for, as for a card that is put in");

    // The caret is left under the card, and the card above it is the one meant.
    command(&mut window, AS_LINK);
    assert_eq!(all_shown(&window), [], "the card is a line of text again");
    assert_eq!(links(&window), [GUIDE]);
    window.key("Mod+Z");
    assert_eq!(all_shown(&window).len(), 1, "one step back brings the card");
}

#[test]
fn the_preview_whose_card_is_out_is_the_one_shown_as_a_link() {
    // Two previews, and the caret right under the second of them.
    let cards = vec![
        Block::new("card", preview(QUOTE, Some("Spring quotes"), None)),
        Block::new("twin", preview(GUIDE, None, None)),
    ];
    let mut window = notes(cards);
    caret(&mut window, "after", 0);
    window.click(Role::Button, EDIT).assert_open(Role::Dialog, CARD);
    assert_eq!(field(&mut window, "Address").as_deref(), Some(QUOTE), "the card of the first is out");

    command(&mut window, AS_LINK);
    assert_eq!(links(&window), [QUOTE]);
    assert_eq!(all_shown(&window), [(GUIDE.to_owned(), None, None, None)], "the other preview stays a card");
    window.assert_closed(Role::Dialog, CARD);
    let kept = window.editor().doc().previews.get(QUOTE).and_then(|kept| kept.title.clone());
    assert_eq!(kept.as_deref(), Some("Spring quotes"), "what the card showed of its page is kept for the link");
}

#[test]
fn a_link_and_a_card_are_offered_their_other_shape_only_where_one_is_meant() {
    type Open = fn() -> Harness;
    // A note with the block its caret is in and how far along, and which of the two commands the palette
    // lists there.
    type Case = (&'static str, Open, (&'static str, usize), &'static [&'static str]);
    let carded: Open = || note(preview(GUIDE, None, None));
    let cases: [Case; 6] = [
        ("in a line that is one link", lone, ("line", 4), &[AS_CARD]),
        ("in a line with words round its link", || linked(None), ("line", 14), &[]),
        ("in a line with no link", lone, ("before", 4), &[]),
        ("at the start of the line under a card", carded, ("after", 0), &[AS_LINK]),
        ("further along the line under a card", carded, ("after", 5), &[]),
        ("at the start of the line over a card", carded, ("before", 0), &[]),
    ];
    for (what, open, (block, offset), lists) in cases {
        let mut window = open();
        caret(&mut window, block, offset);
        assert_eq!(offered(&mut window), lists, "{what}");
    }

    // Someone who does not write the document changes the shape of nothing.
    for (open, (block, offset)) in [(lone as Open, ("line", 4)), (carded, ("after", 0))] {
        let mut window = open();
        window.ui().set_access(Access::Reads);
        caret(&mut window, block, offset);
        assert_eq!(offered(&mut window), [""; 0]);
    }
}
