//! Characters the bundled fonts lack: the window tells its shell of each once, wherever it shows, and sets
//! it in the faces of the system that the shell hands over.

use std::sync::{Arc, OnceLock};

use letters_editor::Editor;
use letters_model::{Anchor, Block, BlockKind, Document, Note, NoteKind, Notes, ReplicaId, Run, Version};
use letters_render::{Font, Item};
use letters_test::{Harness, Role};
use letters_ui::{Location, Window};

/// The sign for a mho, an ohm turned over, which no bundled face has.
const MHO: &str = "\u{2127}";

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

/// Shows the mho somewhere in a letter that does not have it on its page: in a note that is opened,
/// in the document's name, in the version the history shows, or in the version of a conflict that is
/// held back beside the page.
fn show(window: &mut Harness, place: &str) {
    match place {
        "a note" => {
            window.ui().set_notes(noted(NoteKind::Comment, Anchor::range("a", 0..5), &format!("{MHO} or F?")));
            window.click(Role::Button, "1 comment");
        }
        "a conflict" => {
            let theirs = version("Alice Moreau", &format!("{MHO} rates rise in May."));
            let versions = Box::new([theirs, version("Sam Okafor", "Rates rise in May.")]);
            window.ui().set_notes(noted(NoteKind::Conflict { versions }, Anchor::block("a"), ""));
            window.click(Role::Button, "1 conflict");
        }
        "the name" => {
            let name = Some(format!("{MHO} rates"));
            window.ui().set_location(Location { name, ..Location::default() });
        }
        "a version" => {
            window.select("Rates").type_text(MHO);
            window.click(Role::Button, "History");
        }
        _ => {}
    }
}

#[test]
fn a_character_no_bundled_face_has_is_told_once_wherever_it_shows() {
    for place in ["the page", "a note", "the name", "a version", "a conflict"] {
        let on_page = place == "the page";
        let mut window = letter(if on_page { "\u{2127} rates rise in May." } else { "Rates rise in May." });
        assert_eq!(lacking(&mut window), if on_page { vec!['\u{2127}'] } else { Vec::new() }, "{place}");
        show(&mut window, place);
        assert_eq!(lacking(&mut window), if on_page { Vec::new() } else { vec!['\u{2127}'] }, "{place}");

        // Asked again there is nothing new, nor when the same character shows somewhere else.
        assert_eq!(lacking(&mut window), Vec::<char>::new(), "{place}: once");
        // The history has the window to itself, so there is nowhere else to look from it.
        for elsewhere in ["the name", "a note"].into_iter().filter(|_| place != "a version") {
            show(&mut window, elsewhere);
            assert_eq!(lacking(&mut window), Vec::<char>::new(), "{place}, then {elsewhere}");
        }
    }
}

#[test]
fn a_character_only_an_earlier_version_has_is_told_when_that_version_shows() {
    // The mho was written at nine and taken out again an hour later.
    let mut written = letter("Rates rise in May.");
    written.select("Rates").type_text(MHO);
    written.platform.wall = written.platform.wall.map(|nine| nine + 3_600);
    written.select(MHO).type_text("Rates");
    // The letter as another window opens it has never had the mho on its page.
    let copy = written.editor().model().fork(ReplicaId::from(2));
    let mut window = Harness::with(Window::new(Editor::with_model(copy)));
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
    for text in ["Rates rise in May.", "\u{2127} rates rise in May."] {
        let mut window = letter(text);
        window.paint();
        assert!(!window.ui().needs_paint(), "{text}: the frame is shown");
        window.ui().take_uncovered();
        assert!(!window.ui().needs_paint(), "{text}: a shell may ask after every frame");
    }
}

#[test]
fn a_face_of_the_system_sets_what_the_bundled_fonts_lack() {
    // Where the mho shows, when the shell hands the face over.
    for place in ["the page", "a note", "the name", "a version", "a conflict"] {
        let on_page = place == "the page";
        let mut window = letter(if on_page { "\u{2127} rates rise in May." } else { "Rates rise in May." });
        show(&mut window, place);
        assert!(!backed(&mut window), "{place}: no face of the system yet");
        assert!(window.ui().add_fallback_fonts(vec![system()]), "{place}: what lacked a glyph has one");
        assert!(backed(&mut window), "{place}: set in the face of the system");
    }
}

#[test]
fn a_face_of_the_system_is_kept_for_what_is_laid_out_later() {
    for place in ["a note", "the name", "a version", "a conflict"] {
        let mut window = letter("Rates rise in May.");
        window.paint();
        // Nothing lacks a glyph yet, so nothing changes, and what the fonts have stays in them.
        assert!(!window.ui().add_fallback_fonts(vec![system()]), "{place}");
        assert!(!backed(&mut window), "{place}");
        show(&mut window, place);
        assert!(backed(&mut window), "{place}: set in the face that came before it showed");
        assert_eq!(lacking(&mut window), Vec::<char>::new(), "{place}: and the shell is not asked for it");
    }
}
