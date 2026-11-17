//! Fonts a writer can choose beside the bundled ones: the shell names the families it can find, the bar's
//! font menu finds one by name, and the window tells its shell which faces to bring.

use letters_model::{Block, BlockKind, Document};
use letters_render::{Font, Item};
use letters_test::{Harness, Role};
use letters_ui::{FontOffer, FontSource};

/// A family that stands for one a shell can find: a font of the formulas, which is not bundled.
const FAMILY: &str = "KaTeX_Main";

fn face() -> Font {
    Font::new(include_bytes!("../../letters-math/fonts/KaTeX_Main-Regular.ttf")[..].into(), 0)
}

/// A letter of one paragraph, with the caret in it.
fn letter() -> Harness {
    let blocks = vec![Block::text("a", BlockKind::Paragraph, "Rates rise in May.")];
    let mut window = Harness::open(Document { blocks, ..Document::default() });
    window.select("Rates");
    window
}

/// The letter in a window whose shell can find [`FAMILY`] on the device and Lora in a library.
fn offered() -> Harness {
    let mut window = letter();
    let offer = |family: &str, source| FontOffer { family: family.into(), source };
    window.ui().set_families(vec![offer(FAMILY, FontSource::Device), offer("Lora", FontSource::Library)]);
    window
}

/// The families of the faces the window asks its shell for after a frame.
fn asked(window: &mut Harness) -> Vec<String> {
    window.paint();
    window.ui().take_named().into_iter().map(|named| named.family).collect()
}

/// Whether a frame sets anything in [`face`].
fn set_in_it(window: &mut Harness, face: &Font) -> bool {
    let in_face = |item: &Item| matches!(item, Item::Glyphs { font, .. } if font.data.same(&face.data));
    window.paint().items.iter().any(in_face)
}

#[test]
fn a_shell_that_finds_no_fonts_offers_the_bundled_ones_alone() {
    let mut window = letter();
    window.click(Role::Button, "Font");
    assert!(window.find(Role::MenuItemCheckBox, "Inter").is_some(), "the bundled families are there");
    assert!(window.find(Role::MenuItem, "More fonts…").is_none(), "and nothing leads to more");
    assert_eq!(asked(&mut window), Vec::<String>::new(), "the shell is asked for no face of a bundled family");
}

#[test]
fn a_family_found_by_name_restyles_the_type_and_is_asked_of_the_shell() {
    let mut window = offered();
    window.click(Role::Button, "Font").click(Role::MenuItem, "More fonts…");
    window.type_text("katex").key("Enter");

    assert_eq!(window.find(Role::Button, "Font").unwrap().description.as_deref(), Some(FAMILY));
    let told = asked(&mut window);
    assert!(told.iter().any(|family| family == FAMILY), "the shell is asked for its face: {told:?}");
    assert_eq!(asked(&mut window), Vec::<String>::new(), "once");

    let face = face();
    assert!(!set_in_it(&mut window, &face), "until the face comes the text is set in a bundled one");
    window.ui().add_named_fonts(vec![face.clone()]);
    assert!(set_in_it(&mut window, &face), "and then in the face the shell found");

    window.click(Role::Button, "Font");
    assert!(window.find(Role::MenuItemCheckBox, FAMILY).is_some(), "the menu now names it for other types");
}

#[test]
fn escape_leaves_the_type_as_it_was() {
    let mut window = offered();
    window.click(Role::Button, "Font").click(Role::MenuItem, "More fonts…");
    window.type_text("lora").key("Escape");
    assert_eq!(window.find(Role::Button, "Font").unwrap().description.as_deref(), Some("Inter"));
}
