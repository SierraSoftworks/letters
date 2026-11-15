//! Fonts that arrive after the first layout: text that was set short of its face is set again, and nothing
//! else is. Faces of the system back the fonts up where they lack a character.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use letters_fonts::{DISPLAY, MONO, SANS};
use letters_layout::{DocLayout, FontSpec, Fonts, Shaper};
use letters_model::{Align, Block, BlockKind, Document, Mark, Model, Op, ReplicaId, Style};
use letters_render::{Font, Item, Rect};

mod common;
use common::*;

/// The one face a page starts with: the upright of the body's family.
fn first() -> Fonts {
    Fonts { faces: fonts().faces[..1].to_vec() }
}

/// The faces that come later: the italics, the code face and the display face.
fn rest() -> Fonts {
    Fonts { faces: fonts().faces[1..].to_vec() }
}

/// The body's family alone, upright and italic: fonts that lack what the faces below have.
fn body() -> Fonts {
    Fonts { faces: fonts().faces[..2].to_vec() }
}

/// A font file as a system keeps the faces of several families: a collection of the display face, then
/// the code face. Of the two only the code face draws boxes and card suits, which the body's lacks too.
fn system() -> Arc<[u8]> {
    static FILE: OnceLock<Arc<[u8]>> = OnceLock::new();
    let collected = || {
        let files = [fonts().faces[3].data.clone(), fonts().faces[2].data.clone()];
        let mut out = [&b"ttcf"[..], &[0, 1, 0, 0], &(files.len() as u32).to_be_bytes()].concat();
        let mut start = out.len() + 4 * files.len();
        for file in &files {
            out.extend((start as u32).to_be_bytes());
            start += file.len();
        }
        for file in &files {
            // Each table of a face says where it is from the start of the whole file.
            let (base, mut face) = (out.len() as u32, file.to_vec());
            for record in 0..usize::from(u16::from_be_bytes([face[4], face[5]])) {
                let at = 12 + 16 * record + 8;
                let offset = u32::from_be_bytes([face[at], face[at + 1], face[at + 2], face[at + 3]]) + base;
                face[at..at + 4].copy_from_slice(&offset.to_be_bytes());
            }
            out.extend(face);
        }
        out.into()
    };
    FILE.get_or_init(collected).clone()
}

const DISPLAY_FACE: u32 = 0;
const CODE_FACE: u32 = 1;

/// One face of [`system`], as a shell hands over the face it found for a character.
fn backup(index: u32) -> Fonts {
    Fonts { faces: vec![Font::new(system(), index)] }
}

/// Whether anything painted is set in a face of [`system`].
fn backed(items: &[Item]) -> bool {
    items.iter().any(|item| matches!(item, Item::Glyphs { font, .. } if Arc::ptr_eq(&font.data, &system())))
}

/// A copy of a face that calls itself bold, as a system may keep a family of the fonts in files of its own.
fn emboldened(font: &Font) -> Fonts {
    let mut data = font.data.to_vec();
    let records = (0..usize::from(u16::from_be_bytes([data[4], data[5]]))).map(|record| 12 + 16 * record);
    let record = records.into_iter().find(|at| &data[*at..*at + 4] == b"OS/2").expect("a face says its weight");
    let table = u32::from_be_bytes([data[record + 8], data[record + 9], data[record + 10], data[record + 11]]);
    data[table as usize + 4..table as usize + 6].copy_from_slice(&700u16.to_be_bytes());
    Fonts { faces: vec![Font::new(data.into(), 0)] }
}

/// The default style with its top headings in the display family.
fn style() -> Style {
    let mut style = Style::default();
    style.types.get_mut("heading1").expect("a style has headings").family = DISPLAY.into();
    style
}

/// A table whose first cell is written as code, which makes it another width in the code face.
fn coded_cell() -> Block {
    let name = "household_rate_band";
    let table = table("t", &[Align::Start, Align::Start], false, &[&[name, "Band C"]]);
    let mut model = Model::new(doc(vec![table]), ReplicaId::from(1));
    model.apply(vec![Op::SetMark { block: "t00".into(), range: 0..name.len(), mark: Mark::Code, on: true }]).unwrap();
    model.doc().blocks[0].clone()
}

#[test]
fn text_set_short_of_its_face_is_set_again_when_the_face_arrives() {
    let heading = |level: u8| Block::text("x", BlockKind::Heading { level }, "Household rates");
    let marked = |mark: Mark| rich("x", &[("Rates ", &[]), ("change", &[mark]), (" in January.", &[])]);
    // A block, and whether it asks for a face that comes late.
    let cases = [
        ("plain prose", paragraph("x", PROSE), false),
        ("a bold word", marked(Mark::Bold), false),
        ("a heading in the body's family", heading(2), false),
        ("an italic word", marked(Mark::Italic), true),
        ("a quote, which the style slants", container("x", BlockKind::Quote, vec![paragraph("q", PROSE)]), true),
        ("code in a line", marked(Mark::Code), true),
        ("a code block", Block::text("x", BlockKind::Code { language: None }, "let rate = 0.125;"), true),
        ("a heading in the display family", heading(1), true),
        ("code in a cell", coded_cell(), true),
        ("characters only a late face has", paragraph("x", "┌──┐ Rates change."), true),
    ];
    for (name, block, late) in cases {
        let page = doc(vec![paragraph("p", PROSE), block, paragraph("z", PROSE)]);
        let set = |fonts: Fonts| {
            let mut layout = DocLayout::new(fonts);
            layout.update(&page, &style(), WIDTH, None);
            settled(&mut layout);
            layout
        };
        let (mut layout, whole) = (set(first()), set(fonts()));
        let wanted = painted(&whole, everything());
        assert_eq!(painted(&layout, everything()) != wanted, late, "{name}: only a late face is stood in for");

        assert_eq!(layout.add_fonts(&rest()), late, "{name}: and only text that asked for one is set again");
        settled(&mut layout);
        assert!(painted(&layout, everything()) == wanted, "{name}: the page is as if every font had been there");
        assert_eq!(layout.height(), whole.height(), "{name}: at the same height");
        assert!(!layout.add_fonts(&fonts()), "{name}: fonts that are there already change nothing");
    }
}

#[test]
fn a_shaper_takes_late_fonts_too() {
    let (late, whole) = (Shaper::new(first()), Shaper::new(fonts()));
    let font = FontSpec::new(MONO, 14.0);
    let wanted = whole.shape("rate = 0.125", &font, None);
    assert!(late.shape("rate = 0.125", &font, None) != wanted, "a family that is not there yet is stood in for");
    assert!(late.add_fonts(&rest()), "its face arrives");
    assert!(late.shape("rate = 0.125", &font, None) == wanted, "and the string is set in it");
    assert!(!late.add_fonts(&rest()), "once");
}

#[test]
fn characters_no_face_has_are_told_once_with_what_a_face_for_them_is_chosen_by() {
    // A text, and its characters that no bundled face has: each with whether it is to show as an emoji.
    let cases = [
        ("prose the fonts cover", "Café prices: “rates” rose 12 % – naïve…", vec![]),
        (
            "what needs no glyph",
            "soft\u{ad}hyphens, joiners\u{200d}, no\u{a0}break,\ttabs and a\u{3000}wide space",
            vec![],
        ),
        ("Han", "漢字漢字", vec![('漢', false), ('字', false)]),
        ("an emoji", "Done 😀", vec![('😀', true)]),
        ("a flag and a hand in a skin tone", "🇯🇵 👍🏽", vec![('🇯', true), ('🇵', true), ('👍', true), ('🏽', true)]),
        ("a symbol as text", "By air ✈", vec![('✈', false)]),
        ("the symbol asked for as an emoji", "By air ✈\u{fe0f}", vec![('✈', true)]),
        ("a keycap", "Press 1\u{fe0f}\u{20e3}", vec![('\u{20e3}', true)]),
    ];
    for (name, text, lacking) in cases {
        let page = Document { language: "ja".into(), ..doc(vec![paragraph("p", PROSE), paragraph("x", text)]) };
        let mut layout = DocLayout::new(fonts());
        layout.update(&page, &Style::default(), WIDTH, None);
        settled(&mut layout);
        let told = layout.take_uncovered();
        let found: BTreeSet<(char, bool)> = told.iter().map(|missing| (missing.ch, missing.emoji)).collect();
        assert_eq!(found, lacking.iter().copied().collect::<BTreeSet<_>>(), "{name}");
        assert_eq!(told.len(), lacking.len(), "{name}: each is told once");
        let language = |missing: &letters_layout::Uncovered| missing.lang.as_deref() == Some("ja");
        assert!(told.iter().all(language), "{name}: with the language of its text, which says which Han to fetch");

        let again = [page.blocks.clone(), vec![paragraph("y", text)]].concat();
        layout.update(&Document { blocks: again, ..page }, &Style::default(), WIDTH, None);
        settled(&mut layout);
        assert!(layout.take_uncovered().is_empty(), "{name}: and not again where it is written once more");
    }
}

#[test]
fn a_backup_face_sets_only_what_the_fonts_lack() {
    let coded = rich("x", &[("Rates ", &[]), ("change", &[Mark::Code]), (" in January.", &[])]);
    // A block, and whether it has characters that the fonts lack and the backup has.
    let cases = [
        ("box drawing", paragraph("x", "┌──┐ Rates change."), true),
        ("a card suit in a cell", table("x", &[Align::Start], false, &[&["♠ Spades"]]), true),
        ("prose the fonts cover", paragraph("x", PROSE), false),
        ("code, whose family the backup is", coded, false),
        ("Han, which the backup lacks too", paragraph("x", "漢字"), false),
    ];
    for (name, block, helped) in cases {
        let page = doc(vec![paragraph("p", PROSE), block, paragraph("z", PROSE)]);
        let set = |fonts: &[Fonts]| {
            let mut layout = DocLayout::new(body());
            let taken = fonts.iter().any(|fonts| layout.add_fallback_fonts(fonts));
            layout.update(&page, &Style::default(), WIDTH, None);
            settled(&mut layout);
            (layout, taken)
        };
        let first = |layout: &DocLayout| {
            let (top, bottom) = layout.band(&"p".into()).expect("the first paragraph is laid out");
            painted(layout, Rect::new(0.0, top, WIDTH, bottom - top))
        };
        let (mut layout, _) = set(&[]);
        let (before, prose) = (painted(&layout, everything()), first(&layout));

        let other = layout.add_fallback_fonts(&backup(DISPLAY_FACE));
        assert!(!other, "{name}: of a file of several families, only that of the face handed over is used");
        assert_eq!(layout.add_fallback_fonts(&backup(CODE_FACE)), helped, "{name}: a face that has what is lacking");
        settled(&mut layout);
        let after = painted(&layout, everything());
        assert_eq!(backed(&after), helped, "{name}: it sets what the fonts lack");
        assert_eq!(after != before, helped, "{name}: and nothing else changes");
        assert!(first(&layout) == prose, "{name}: what the fonts cover is set as it was");
        assert!(!layout.add_fallback_fonts(&backup(CODE_FACE)), "{name}: a backup that is there changes nothing");

        let (ready, early) = set(&[backup(DISPLAY_FACE), backup(CODE_FACE)]);
        assert!(!early, "{name}: before any text is met, nothing is lacking");
        assert!(painted(&ready, everything()) == after, "{name}: a page that starts with the backup is set alike");
    }
}

#[test]
fn a_backup_never_stands_for_a_family_of_the_fonts() {
    let page = doc(vec![rich("x", &[("Rates ", &[]), ("change", &[Mark::Bold]), (" in January.", &[])])]);
    let set = |backup: Option<Fonts>| {
        let mut layout = DocLayout::new(fonts());
        let taken = backup.is_some_and(|backup| layout.add_fallback_fonts(&backup));
        layout.update(&page, &Style::default(), WIDTH, None);
        settled(&mut layout);
        (taken, painted(&layout, everything()))
    };
    let (taken, with) = set(Some(emboldened(&fonts().faces[0])));
    assert!(!taken, "a file of a family the fonts have is left out");
    assert!(with == set(None).1, "so bold text is set in the fonts' own face, whatever a system has by that name");
}

#[test]
fn a_shaper_tells_what_it_lacks_and_takes_backups() {
    let (shaper, plain) = (Shaper::new(body()), Shaper::new(body()));
    let font = FontSpec::new(SANS, 14.0);
    let before = shaper.shape("♠ Spades", &font, None);
    let told: Vec<_> =
        shaper.take_uncovered().into_iter().map(|missing| (missing.ch, missing.emoji, missing.lang)).collect();
    assert_eq!(told, [('♠', false, None)], "a plain string has no language");
    assert!(shaper.add_fallback_fonts(&backup(CODE_FACE)), "a face for it arrives");
    assert!(shaper.shape("♠ Spades", &font, None) != before, "and the string is set with it");
    assert!(shaper.shape("Spades", &font, None) == plain.shape("Spades", &font, None), "the rest as it was");
    assert!(shaper.take_uncovered().is_empty(), "nothing is lacking now");
    assert!(!shaper.add_fallback_fonts(&backup(CODE_FACE)), "and the face is taken once");
}
