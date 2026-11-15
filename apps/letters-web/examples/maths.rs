//! The maths engine as a module of its own, which the page fetches when a document first shows a formula.
//!
//! It is a quarter of a megabyte that the editor's first frame would otherwise wait for, and that a
//! document with no maths never uses. A formula crosses back to the editor as bytes, since the two share
//! no memory: see `letters_math::Formula::to_bytes`.

use js_sys::{Array, Uint8Array};
use letters_math::{Error, Style, built_in};
use letters_render::Color;
use wasm_bindgen::prelude::*;

/// Hands over the maths faces, in the order `letters_math::FONT_FILES` names them, and the body face,
/// which draws the letters they lack. Called once, before the first formula is asked for.
#[wasm_bindgen]
pub fn fonts(body: Vec<u8>, maths: &Array) {
    letters_fonts::provide_first(body);
    letters_math::provide_fonts(maths.iter().map(|file| Uint8Array::new(&file).to_vec()).collect());
}

/// Takes the whole body face, where the editor started with a part of it.
#[wasm_bindgen]
pub fn body(face: Vec<u8>) {
    letters_fonts::provide(vec![face]);
}

/// Why a source could not be set, as the editor reads it back: what is wrong, and the bytes at fault where
/// the engine can tell.
fn refusal(error: Error) -> JsValue {
    let span = error.span.map_or([JsValue::UNDEFINED, JsValue::UNDEFINED], |span| [span.start.into(), span.end.into()]);
    Array::of3(&error.message.into(), &span[0], &span[1]).into()
}

/// Typesets `source` as `letters_math::typeset` does, or as `typeset_mapped` where `mapped` is set, and
/// gives the formula as bytes. `color` is the style's as `0xRRGGBBAA`.
#[wasm_bindgen]
pub fn typeset(
    source: &str,
    size: f32,
    color: u32,
    display: bool,
    links: bool,
    mapped: bool,
) -> Result<Vec<u8>, JsValue> {
    let [r, g, b, a] = color.to_be_bytes();
    let style = Style { size, color: Color { r, g, b, a }, display, links };
    (built_in().typeset)(source, &style, mapped).map(|formula| formula.to_bytes()).map_err(refusal)
}

/// Writes `source` as MathML, as `letters_math::mathml` does.
#[wasm_bindgen]
pub fn mathml(source: &str, display: bool) -> Result<String, JsValue> {
    (built_in().mathml)(source, display).map_err(refusal)
}
