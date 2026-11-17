//! What a document may need that the window does not start with: a face for characters the bundled fonts
//! lack, and a word list for a script that is written without spaces. The window tells its shell of each
//! once, wherever it shows, and uses what the shell hands over there and in whatever is laid out later.

use std::sync::{Arc, OnceLock};

use letters_editor::Editor;
use letters_model::{Anchor, Block, BlockKind, Document, Note, NoteKind, Notes, ReplicaId, Run, Version};
use letters_render::{Font, Item};
use letters_test::{Harness, Role};
use letters_ui::{Location, Window, WordList};

/// The places of [`show`] that the history shows, which has the window to itself: nothing else shows with it.
const HISTORY: [&str; 2] = ["a version", "a passage"];

/// The sign for a mho, an ohm turned over, which no bundled face has.
const MHO: &str = "\u{2127}";

/// Thai as it is written, without spaces: words a word list keeps whole, in an order that does not come
/// round within a line, so that lines which end wherever they fill cannot all end between words.
fn thai() -> String {
    const WORDS: [&str; 9] = ["ภาษา", "ไทย", "เป็น", "ภาษา", "ราชการ", "ของ", "ประเทศ", "ไทย", "และ"];
    (0..18).flat_map(|from| &WORDS[from % WORDS.len()..]).copied().collect()
}

/// A face that stands for one of the system: a font of the formulas, which has the mho and is not
/// among the bundled fonts.
fn system() -> Font {
    static FILE: OnceLock<Arc<[u8]>> = OnceLock::new();
    let file = || Arc::from(&include_bytes!("../../letters-math/fonts/KaTeX_AMS-Regular.ttf")[..]);
    Font::new(FILE.get_or_init(file).clone(), 0)
}

/// A letter of one paragraph `a`, written at nine on a Sunday morning.
fn letter(text: &str) -> Harness {
    let blocks = vec![Block::text("a", BlockKind::Paragraph, text)];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.platform.wall = Some(1_794_733_200);
    window
}

/// The letter with paragraph `a` written over hour by hour from nine, each `old` with its `new`, as a window
/// opens it that never had the earlier versions on its page.
fn rewritten(edits: &[(&str, &str)]) -> Harness {
    let mut written = letter("Rates rise in May.");
    for (old, new) in edits {
        written.select(old).type_text(new);
        written.platform.wall = written.platform.wall.map(|then| then + 3_600);
    }
    let copy = written.editor().model().fork(ReplicaId::from(2));
    Harness::with(Window::new(Editor::with_model(copy)))
}

/// A note on paragraph `a` that says `says`.
fn noted(kind: NoteKind, anchor: Anchor, says: &str) -> Notes {
    let note = Note {
        id: "n".into(),
        kind,
        anchor,
        author: "Alice Moreau".into(),
        created: "2026-11-10T08:00:00Z".into(),
        resolved: false,
        content: vec![Run::plain(says)],
        replies: Vec::new(),
        extra: Default::default(),
    };
    Notes { notes: vec![note], ..Notes::default() }
}

/// How one writer left paragraph `a`, for a conflict about it.
fn version(author: &str, text: &str) -> Version {
    let block = Some(Block::text("a", BlockKind::Paragraph, text));
    Version { author: author.into(), edited: "2026-11-14T10:00:00Z".into(), block, extra: Default::default() }
}

/// The characters a frame showed without a glyph, as the shell is told of them after it.
fn lacking(window: &mut Harness) -> Vec<char> {
    window.paint();
    window.ui().take_uncovered().into_iter().map(|lacking| lacking.ch).collect()
}

/// Whether a frame sets anything in the face of the system.
fn backed(window: &mut Harness) -> bool {
    let face = system();
    let in_face = |item: &Item| matches!(item, Item::Glyphs { font, .. } if font.data.same(&face.data));
    window.paint().items.iter().any(in_face)
}

/// The scripts a frame set without their word list, as the shell is told of them after it.
fn unlisted(window: &mut Harness) -> Vec<WordList> {
    window.paint();
    window.ui().take_unlisted()
}

/// Where a frame sets its text: each run of glyphs and how many it holds. Lines that end elsewhere move them.
fn set(window: &mut Harness) -> Vec<(f32, f32, usize)> {
    let run = |item: &Item| match item {
        Item::Glyphs { origin, glyphs, .. } => Some((origin.x, origin.y, glyphs.len())),
        _ => None,
    };
    window.paint().items.iter().filter_map(run).collect()
}

/// Shows `what` somewhere in a letter that does not have it on its page: in a note that is opened, in the
/// document's name, in a label that wraps (the card that says why a link is not opened), in the version
/// the history shows, in what the history of a selection says a version did, or in the version of a
/// conflict that is held back beside the page.
fn show(window: &mut Harness, place: &str, what: &str) {
    match place {
        "a label" => window.ui().refuse_link("https://example.org/", what),
        "a note" => {
            // The comment shows on its card beside the page.
            window.ui().set_notes(noted(NoteKind::Comment, Anchor::range("a", 0..5), &format!("{what} or F?")));
            window.paint();
        }
        "a conflict" => {
            let theirs = version("Alice Moreau", &format!("{what} rates rise in May."));
            let versions = Box::new([theirs, version("Sam Okafor", "Rates rise in May.")]);
            window.ui().set_notes(noted(NoteKind::Conflict { versions }, Anchor::block("a"), ""));
            window.click(Role::Button, "1 conflict");
        }
        "the name" => {
            let name = Some(format!("{what} rates"));
            window.ui().set_location(Location { name, ..Location::default() });
        }
        "a version" => {
            window.select("Rates").type_text(what);
            window.click(Role::Button, "History");
        }
        "a passage" => {
            // Written at nine and taken out at ten: the page of the history has it no more than the
            // letter's own, and the panel shows the paragraph as it read with it.
            window.select("Rates").type_text(what);
            window.platform.wall = window.platform.wall.map(|nine| nine + 3_600);
            window.select(what).type_text("Rates");
            window.select("Rates").key("Mod+Shift+P").type_text("History of selection").key("Enter");
        }
        _ => {}
    }
}

#[test]
fn a_character_no_bundled_face_has_is_told_once_wherever_it_shows() {
    for place in ["the page", "a note", "the name", "a version", "a passage", "a conflict"] {
        let on_page = place == "the page";
        let mut window = letter(if on_page { "\u{2127} rates rise in May." } else { "Rates rise in May." });
        assert_eq!(lacking(&mut window), if on_page { vec!['\u{2127}'] } else { Vec::new() }, "{place}");
        show(&mut window, place, MHO);
        assert_eq!(lacking(&mut window), if on_page { Vec::new() } else { vec!['\u{2127}'] }, "{place}");

        // Asked again there is nothing new, nor when the same character shows somewhere else.
        assert_eq!(lacking(&mut window), Vec::<char>::new(), "{place}: once");
        for elsewhere in ["the name", "a note"].into_iter().filter(|_| !HISTORY.contains(&place)) {
            show(&mut window, elsewhere, MHO);
            assert_eq!(lacking(&mut window), Vec::<char>::new(), "{place}, then {elsewhere}");
        }
    }
}

#[test]
fn a_character_only_an_earlier_version_has_is_told_when_that_version_shows() {
    // The mho was written at nine and taken out again an hour later.
    let mut window = rewritten(&[("Rates", MHO), (MHO, "Rates")]);
    assert_eq!(lacking(&mut window), Vec::<char>::new(), "the page");
    // The newest version shows what the one before it had and it took out.
    window.click(Role::Button, "History");
    assert_eq!(lacking(&mut window), ['\u{2127}'], "the history");
    assert_eq!(lacking(&mut window), Vec::<char>::new(), "once");
}

#[test]
fn what_is_told_says_how_the_character_is_written() {
    // What is written, and what a face is chosen by: the character, and whether it is to show as an emoji.
    let cases = [("By air \u{2708}", '\u{2708}', false), ("By air \u{2708}\u{fe0f}", '\u{2708}', true)];
    for (text, ch, emoji) in cases {
        let mut window = letter(text);
        window.paint();
        let told: Vec<(char, bool)> =
            window.ui().take_uncovered().into_iter().map(|lacking| (lacking.ch, lacking.emoji)).collect();
        assert_eq!(told, [(ch, emoji)], "{text}");
    }
}

#[test]
fn asking_what_is_lacking_asks_for_no_frame() {
    for text in ["Rates rise in May.", "\u{2127} rates rise in May.", &thai()] {
        let mut window = letter(text);
        window.paint();
        assert!(!window.ui().needs_paint(), "{text}: the frame is shown");
        window.ui().take_uncovered();
        window.ui().take_unlisted();
        assert!(!window.ui().needs_paint(), "{text}: a shell may ask after every frame");
    }
}

#[test]
fn a_face_of_the_system_sets_what_the_bundled_fonts_lack() {
    // Where the mho shows, when the shell hands the face over.
    for place in ["the page", "a note", "the name", "a version", "a passage", "a conflict"] {
        let on_page = place == "the page";
        let mut window = letter(if on_page { "\u{2127} rates rise in May." } else { "Rates rise in May." });
        show(&mut window, place, MHO);
        assert!(!backed(&mut window), "{place}: no face of the system yet");
        assert!(window.ui().add_fallback_fonts(vec![system()]), "{place}: what lacked a glyph has one");
        assert!(backed(&mut window), "{place}: set in the face of the system");
    }
}

#[test]
fn a_face_of_the_system_is_kept_for_what_is_laid_out_later() {
    for place in ["a note", "the name", "a version", "a passage", "a conflict"] {
        let mut window = letter("Rates rise in May.");
        window.paint();
        // Nothing lacks a glyph yet, so nothing changes, and what the fonts have stays in them.
        assert!(!window.ui().add_fallback_fonts(vec![system()]), "{place}");
        assert!(!backed(&mut window), "{place}");
        show(&mut window, place, MHO);
        assert!(backed(&mut window), "{place}: set in the face that came before it showed");
        assert_eq!(lacking(&mut window), Vec::<char>::new(), "{place}: and the shell is not asked for it");
    }
}

/// The scripts a build is told of: none where every word list is built in.
fn asked(shows: bool) -> Vec<WordList> {
    if shows && !WordList::BUILT_IN { vec![WordList::Thai] } else { Vec::new() }
}

#[test]
fn a_script_without_its_word_list_is_told_once_wherever_it_shows() {
    let thai = thai();
    for place in ["the page", "a note", "the name", "a label", "a version", "a passage", "a conflict"] {
        let on_page = place == "the page";
        let mut window = letter(if on_page { &thai } else { "Rates rise in May." });
        assert_eq!(unlisted(&mut window), asked(on_page), "{place}");
        show(&mut window, place, &thai);
        assert_eq!(unlisted(&mut window), asked(!on_page), "{place}");

        // Asked again there is nothing new, nor when the same script shows somewhere else. The history and
        // a card over the window leave no way to the notes.
        assert_eq!(unlisted(&mut window), [], "{place}: once");
        if !HISTORY.contains(&place) && place != "a label" {
            show(&mut window, "a note", &thai);
            assert_eq!(unlisted(&mut window), [], "{place}, then a note");
        }
    }
}

#[test]
fn a_word_list_ends_lines_between_words_wherever_its_script_shows() {
    let (thai, list) = (thai(), WordList::Thai.data());
    for place in ["the page", "a note", "a label", "a version", "a passage", "a conflict"] {
        // The list comes before the text shows, or once the shell was told that the text needs it.
        let shown = |ahead: bool| {
            let mut window = letter(if place == "the page" { &thai } else { "Rates rise in May." });
            if ahead {
                assert!(!window.ui().add_word_list(WordList::Thai, &list), "{place}: nothing is set in Thai yet");
            }
            show(&mut window, place, &thai);
            window
        };
        let (mut ahead, mut late) = (shown(true), shown(false));
        let without = set(&mut late);
        let taken = late.ui().add_word_list(WordList::Thai, &list);
        assert_eq!(taken, !WordList::BUILT_IN, "{place}: lines end elsewhere only where the list was missing");
        let with = set(&mut late);
        assert_eq!(with != without, !WordList::BUILT_IN, "{place}: a line no longer ends wherever it fills");
        assert_eq!(with, set(&mut ahead), "{place}: as if the list had been here before the text");
        assert_eq!(unlisted(&mut ahead), [], "{place}: and the shell is not asked for a list that is here");
    }
}

#[test]
fn a_file_that_is_not_the_word_list_changes_nothing_and_the_list_is_taken_after_it() {
    let (thai, list) = (thai(), WordList::Thai.data());
    // How a letter is set that was handed the list alone: its page, and a note that is opened later.
    let mut whole = letter(&thai);
    whole.ui().add_word_list(WordList::Thai, &list);
    show(&mut whole, "a note", &thai);
    let whole = set(&mut whole);
    let damaged: [(&str, Vec<u8>); 3] = [
        ("nothing", Vec::new()),
        ("a file cut short", list[..list.len() / 2].to_vec()),
        ("another script's list", WordList::Lao.data()),
    ];
    for (name, data) in damaged {
        let mut window = letter(&thai);
        let before = set(&mut window);
        assert!(!window.ui().add_word_list(WordList::Thai, &data), "{name}");
        assert_eq!(set(&mut window), before, "{name}");
        let taken = window.ui().add_word_list(WordList::Thai, &list);
        assert_eq!(taken, !WordList::BUILT_IN, "{name}, then the list");
        show(&mut window, "a note", &thai);
        assert_eq!(set(&mut window), whole, "{name}, then the list: as if the list alone had come");
    }
}

#[test]
fn a_word_list_reaches_the_versions_the_history_steps_to() {
    let (thai, list) = (thai(), WordList::Thai.data());
    // Thai was written at nine and taken out at ten, and more was written at eleven: the history opens
    // on a version without Thai, and the one before it shows how the paragraph read with it.
    let edits = [("Rates", &*thai), (&*thai, "Rates"), ("May", "May or June")];
    // The list comes before the history opens, while the newest version shows again, or last of all.
    let stepped = |comes: &str| {
        let mut window = rewritten(&edits);
        let hand = |now: &str, window: &mut Harness| {
            if now == comes {
                window.ui().add_word_list(WordList::Thai, &list);
            }
        };
        hand("first", &mut window);
        window.click(Role::Button, "History").key("Left").paint();
        window.key("Right").paint();
        hand("between", &mut window);
        window.key("Left").paint();
        hand("last", &mut window);
        window
    };
    let mut never = stepped("never");
    assert_eq!(unlisted(&mut never), asked(true), "the version that had Thai tells of it");
    let (without, with) = (set(&mut never), set(&mut stepped("last")));
    assert_eq!(with != without, !WordList::BUILT_IN, "a line no longer ends wherever it fills");
    for comes in ["first", "between"] {
        assert_eq!(set(&mut stepped(comes)), with, "{comes}: as when the list comes last");
    }
}

#[test]
fn what_only_the_history_of_a_selection_shows_is_told_of_too() {
    // Written at nine and taken out at ten, in a letter that a window opens afterwards: the panel alone
    // shows it, where it says how the paragraph read.
    let passage = |what: &str| {
        let mut window = rewritten(&[("Rates", what), (what, "Rates")]);
        window.select("Rates").key("Mod+Shift+P").type_text("History of selection").key("Enter");
        window
    };
    assert_eq!(lacking(&mut passage(MHO)), ['\u{2127}'], "a character no bundled face has");
    assert_eq!(unlisted(&mut passage(&thai())), asked(true), "a script without its word list");
}
