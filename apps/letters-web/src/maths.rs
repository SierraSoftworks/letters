//! The maths engine, which is a module of its own that the page fetches when a document first shows a
//! formula: `examples/maths.rs`. This is the editor's end of it.

use std::cell::RefCell;

use js_sys::{Array, Function, Reflect, Uint8Array};
use letters_math::{Engine, Error, Formula, Style};
use wasm_bindgen::prelude::*;

thread_local! {
    /// The module, once it has been fetched and has its faces.
    static MODULE: RefCell<Option<JsValue>> = const { RefCell::new(None) };
}

/// Calls a function of the module.
fn call(name: &str, arguments: &Array) -> Result<JsValue, JsValue> {
    MODULE.with_borrow(|module| {
        let module = module.as_ref().ok_or(JsValue::UNDEFINED)?;
        Reflect::get(module, &name.into())?.dyn_into::<Function>()?.apply(&JsValue::NULL, arguments)
    })
}

/// What the module says of a source it would not set, or what went wrong on the way there.
fn refusal(said: JsValue) -> Error {
    let part = |at: u32| Reflect::get_u32(&said, at).ok();
    let message = part(0).and_then(|message| message.as_string());
    let ends = |at: u32| part(at).and_then(|end| end.as_f64()).map(|end| end as usize);
    let span = ends(1).zip(ends(2)).map(|(start, end)| start..end);
    Error { message: message.unwrap_or_else(|| "The maths engine did not answer.".to_owned()), span }
}

fn typeset(source: &str, style: &Style, mapped: bool) -> Result<Formula, Error> {
    let Style { size, color, display, links } = *style;
    let color = u32::from_be_bytes([color.r, color.g, color.b, color.a]);
    let asked: [JsValue; 6] = [source.into(), size.into(), color.into(), display.into(), links.into(), mapped.into()];
    let set = call("typeset", &asked.into_iter().collect()).map_err(refusal)?;
    Formula::from_bytes(&Uint8Array::new(&set).to_vec()).ok_or_else(|| refusal(JsValue::UNDEFINED))
}

fn mathml(source: &str, display: bool) -> Result<String, Error> {
    let written = call("mathml", &Array::of2(&source.into(), &display.into())).map_err(refusal)?;
    written.as_string().ok_or_else(|| refusal(JsValue::UNDEFINED))
}

/// Takes the module, which is fetched and started, and has it set the editor's formulas from here on.
/// `faces` are the maths faces, which the editor holds too, and the body face is the first font the
/// editor has: a formula names its faces when it crosses, and both ends must have them.
pub fn provide(module: JsValue, faces: Vec<Vec<u8>>) -> Result<(), JsValue> {
    let body = letters_fonts::bundled().into_iter().next().map(|font| Uint8Array::from(&font.data[..]));
    let files: Array = faces.iter().map(|file| Uint8Array::from(&file[..])).collect();
    MODULE.set(Some(module));
    call("fonts", &Array::of2(&body.unwrap_or_else(|| Uint8Array::new_with_length(0)), &files))?;
    // The editor's own copy: it does nothing where the first frame waited for them.
    letters_math::provide_fonts(faces);
    letters_math::provide_engine(Engine { typeset, mathml });
    Ok(())
}

/// Gives the module the body face again, once the whole of it has taken the place of the part the page
/// started with. A formula with a letter only the whole face has, set between the two, stays without it.
pub fn rebody() {
    let Some(body) = letters_fonts::bundled().into_iter().next() else { return };
    let _ = call("body", &Array::of1(&Uint8Array::from(&body.data[..])));
}

/// Whether the module is here.
pub fn here() -> bool {
    MODULE.with_borrow(Option::is_some)
}
