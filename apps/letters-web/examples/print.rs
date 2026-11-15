//! The PDF export as a module of its own, which the page fetches when the person first prints.
//!
//! The export brings a PDF writer, the picture decoders and font subsetting: half a megabyte that the
//! editor's first frame would otherwise wait for, and that most visits never use.

use js_sys::{Array, Map, Uint8Array};
use letters_convert::{PdfInfo, to_pdf};
use letters_layout::Fonts;
use letters_model::{Document, Style};
use wasm_bindgen::prelude::*;

fn files(files: &Array) -> Vec<Vec<u8>> {
    files.iter().map(|file| Uint8Array::new(&file).to_vec()).collect()
}

/// Hands over the fonts and the maths faces, each in the order its crate names its files. Called once,
/// before the first [`pdf`].
#[wasm_bindgen]
pub fn fonts(fonts: &Array, maths: &Array) {
    letters_fonts::provide(files(fonts));
    letters_math::provide_fonts(files(maths));
}

/// The PDF of a document, from its `document.json` and `style.json`. `assets` holds the pictures it
/// shows, by their names in it. `created` is in seconds since 1970 began.
#[wasm_bindgen]
pub fn pdf(
    document: &str,
    style: &str,
    title: &str,
    authors: Vec<String>,
    created: Option<f64>,
    assets: &Map,
) -> Result<Vec<u8>, JsError> {
    let (document, style) = (Document::from_json(document)?, Style::from_json(style)?);
    let info = PdfInfo { title: title.to_owned(), authors, created: created.map(|seconds| seconds as u64) };
    let mut asset =
        |name: &str| Some(Uint8Array::new(&assets.get(&name.into())).to_vec()).filter(|file| !file.is_empty());
    let fonts = Fonts { faces: letters_fonts::bundled() };
    Ok(to_pdf(&document, &style, fonts, &mut asset, &info)?)
}
