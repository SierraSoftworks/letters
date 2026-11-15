//! Fonts that arrive after the first layout: text that was set short of its face is set again, and nothing
//! else is.

use letters_fonts::{DISPLAY, MONO};
use letters_layout::{DocLayout, FontSpec, Fonts, Shaper};
use letters_model::{Align, Block, BlockKind, Mark, Model, Op, ReplicaId, Style};

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
