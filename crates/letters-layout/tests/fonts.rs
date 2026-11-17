//! Fonts that arrive after the first layout: text that was set short of its face is set again, and nothing
//! else is. Faces of the system back the fonts up where they lack a character.

use std::collections::BTreeSet;
use std::sync::{Arc, OnceLock};

use letters_fonts::{DISPLAY, MONO, SANS};
use letters_layout::{DocLayout, FontSpec, Fonts, Shaper};
use letters_model::{Align, Block, BlockKind, Document, Mark, Model, Op, Pos, ReplicaId, Style};
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
    items.iter().any(|item| matches!(item, Item::Glyphs { font, .. } if font.data.same(&system())))
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

/// The Latin part of a face, as a page starts with to show its first words early: a copy that maps no
/// character past Latin Extended-B. Its glyphs keep their numbers, so what it sets reads the same in the
/// whole face.
fn latin(font: &Font) -> Fonts {
    let mut data = font.data.to_vec();
    let long =
        |data: &[u8], at: usize| u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]) as usize;
    let short = |data: &[u8], at: usize| usize::from(u16::from_be_bytes([data[at], data[at + 1]]));
    let records = (0..short(&data, 4)).map(|record| 12 + 16 * record);
    let record = records.into_iter().find(|at| &data[*at..*at + 4] == b"cmap").expect("a face maps its characters");
    let maps = long(&data, record + 8);
    // Each of its tables of characters is listed with where it starts. The one of format 12 lists stretches
    // of characters in order, so it is cut after the last Latin one, and every listing is pointed at it.
    let listed: Vec<usize> = (0..short(&data, maps + 2)).map(|at| maps + 4 + 8 * at + 4).collect();
    let full = listed.iter().map(|at| long(&data, *at)).find(|table| short(&data, maps + table) == 12);
    let full = full.expect("a table of every character");
    let stretch = |data: &[u8], at: usize| long(data, maps + full + 16 + 12 * at + 4);
    let kept = (0..long(&data, maps + full + 12)).take_while(|at| stretch(&data, *at) < 0x250).count();
    data[maps + full + 12..maps + full + 16].copy_from_slice(&(kept as u32).to_be_bytes());
    for at in listed {
        data[at..at + 4].copy_from_slice(&(full as u32).to_be_bytes());
    }
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
fn the_whole_of_a_face_takes_the_place_of_the_part_a_page_started_with() {
    const GREEK: &str = "Τιμές";
    let whole = first();
    // A part cut here, and the one letters-fonts keeps for a shell that fetches its fonts.
    let kept = format!("{}/../letters-fonts/fonts/{}", env!("CARGO_MANIFEST_DIR"), letters_fonts::FIRST);
    let kept = Font::new(std::fs::read(kept).expect("the part letters-fonts names").into(), 0);
    let parts =
        [("a part cut here", latin(&fonts().faces[0])), ("the part letters-fonts keeps", Fonts { faces: vec![kept] })];
    let set_in = |items: &[Item], fonts: &Fonts| {
        let mut runs = glyph_runs(items).peekable();
        runs.peek().is_some()
            && runs.all(|item| matches!(item, Item::Glyphs { font, .. } if font.data.same(&fonts.faces[0].data)))
    };
    // What a paragraph says before the whole face arrives, and whether that is set short of a character.
    let cases = [("Latin first", "Rates", false), ("Greek first", GREEK, true)];
    for ((of, part), (first, early, short)) in parts.iter().flat_map(|part| cases.map(|case| (part, case))) {
        let name = format!("{of}, {first}");
        let mut model = Model::new(doc(vec![paragraph("p", PROSE), paragraph("x", early)]), ReplicaId::from(1));
        let mut layout = DocLayout::new(part.clone());
        layout.update(model.doc(), &Style::default(), WIDTH, None);
        settled(&mut layout);
        let before = painted(&layout, everything());
        assert!(set_in(&before, part), "{name}: the page starts in the part");
        assert_eq!(!layout.take_uncovered().is_empty(), short, "{name}: which lacks only what is not Latin");

        assert_eq!(layout.add_fonts(&whole), short, "{name}: only text that was short of a character is set again");
        settled(&mut layout);
        if !short {
            assert!(painted(&layout, everything()) == before, "{name}: and what was laid out stays as it was");
        }

        // What is written from then on is set in the whole face, which has every character.
        let change = model.apply(vec![Op::InsertText { at: Pos::new("x", 0), text: GREEK.into() }]).unwrap();
        layout.update(model.doc(), &Style::default(), WIDTH, Some(&change));
        settled(&mut layout);
        let after = painted(&layout, everything());
        let mut written = DocLayout::new(whole.clone());
        written.update(model.doc(), &Style::default(), WIDTH, None);
        assert_eq!(placed(&after), placed(&painted(&written, everything())), "{name}: the page reads as in the whole");
        assert!(placed(&after).iter().all(|(glyph, ..)| *glyph != 0), "{name}: with a glyph for every character");
        assert!(layout.take_uncovered().is_empty(), "{name}: and nothing is told as lacking");
        let line = layout.band(&"x".into()).map(|(top, height)| Rect::new(0.0, top, WIDTH, height)).unwrap();
        assert!(set_in(&painted(&layout, line), &whole), "{name}: the new words are in the whole face");
        assert!(!layout.add_fonts(part), "{name}: the part given again changes nothing");
    }

    let (late, wanted) = (Shaper::new(parts[0].1.clone()), Shaper::new(whole.clone()));
    let font = FontSpec::new(SANS, 14.0);
    assert!(late.shape(GREEK, &font, None) != wanted.shape(GREEK, &font, None), "a shaper lacks them too");
    late.add_fonts(&whole);
    assert!(late.shape(GREEK, &font, None) == wanted.shape(GREEK, &font, None), "until the whole face comes");
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

#[test]
fn a_face_a_style_names_is_told_once_and_sets_its_text_when_it_comes() {
    // The display family stands for one a writer chose, which the fonts at the start lack.
    let mut style = Style::default();
    style.types.get_mut("heading1").unwrap().family = DISPLAY.into();
    let chosen = Fonts { faces: fonts().faces[3..4].to_vec() };
    let model = Model::new(
        doc(vec![heading("h", 1, "Rates"), rich("p", &[("Rates ", &[]), ("change", &[Mark::Bold, Mark::Italic])])]),
        ReplicaId::from(1),
    );
    let mut layout = DocLayout::new(body());
    layout.update(model.doc(), &style, WIDTH, None);
    settled(&mut layout);

    let told = layout.take_named();
    let has = |family: &str, weight: u16, italic: bool| {
        told.iter().any(|named| named.family == family && named.weight == weight && named.italic == italic)
    };
    assert!(has(DISPLAY, 600, false), "the heading's face is told: {told:?}");
    assert!(!told.iter().any(|named| named.family == MONO), "a type no text is set in asks for no face: {told:?}");
    assert!(has(SANS, 400, false), "and the body's, which a shell knows it has");
    assert!(told.iter().any(|named| named.family == SANS && named.weight > 400 && named.italic), "and a mark's");
    assert!(layout.take_named().is_empty(), "each only once");

    let in_chosen = |layout: &DocLayout| {
        let items = painted(layout, everything());
        glyph_runs(&items)
            .any(|item| matches!(item, Item::Glyphs { font, .. } if font.data.same(&chosen.faces[0].data)))
    };
    assert!(!in_chosen(&layout), "until the face comes the heading is set in what there is");
    assert!(layout.add_fonts(&chosen), "the face is taken by its family's name");
    settled(&mut layout);
    assert!(in_chosen(&layout), "and the heading is set in it");
}

#[test]
fn a_style_tells_every_face_its_text_can_ask_for() {
    let mut style = Style::default();
    style.types.get_mut("heading1").unwrap().family = "Lora".into();
    let faces = letters_layout::faces(&style);
    let of = |family: &str| {
        let mut found: Vec<(u16, bool)> =
            faces.iter().filter(|face| face.family == family).map(|face| (face.weight, face.italic)).collect();
        found.sort_unstable();
        found
    };
    assert_eq!(of("Lora"), [(600, false), (600, true), (800, false), (800, true)], "a heading and bold in it");
    assert!(of(SANS).contains(&(400, true)) && of(SANS).contains(&(700, false)), "the body, slanted and bold");
    assert_eq!(of(MONO), [(400, false), (400, true), (700, false), (700, true)]);
}
