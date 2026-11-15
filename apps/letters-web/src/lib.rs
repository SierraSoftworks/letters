//! The Letters browser shell: the shared engine painting on a canvas.
//!
//! - [`shell`] owns the canvas, the hidden input beside it and the frame loop.
//! - [`input`] turns the hidden input's events into the editor's input, and [`browser`] lends the editor
//!   the browser's clock, identifiers and clipboard.
//! - [`mirror`] keeps the window readable as HTML beside the canvas, for assistive technology.
//! - [`page`] is the page a server sends: the document as HTML to read at once, and for the editor to open.
//! - [`Letters`] is what a page holds: `await Letters.start(canvas, "fonts/")` starts the editor.
//!
//! Frames are rasterised on the CPU, so the page needs no WebGPU. The fonts are fetched beside the wasm and
//! are no part of it, so a new build of the code leaves them in the browser's cache.

pub mod browser;
pub mod input;
pub mod mirror;
pub mod page;
pub mod shell;

use std::cell::RefCell;
use std::rc::Rc;

use letters_model::Document;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::shell::Shell;

/// The editor on a page.
#[wasm_bindgen]
pub struct Letters {
    shell: Rc<RefCell<Shell>>,
}

#[wasm_bindgen]
impl Letters {
    /// Starts the editor in `canvas`, whose size the page's CSS decides, with the fonts found under
    /// `fonts`, a URL that ends in a slash.
    pub async fn start(canvas: HtmlCanvasElement, fonts: String) -> Result<Letters, JsValue> {
        std::panic::set_hook(Box::new(|panic| web_sys::console::error_1(&panic.to_string().into())));
        let window = web_sys::window().ok_or("there is no window")?;
        letters_fonts::provide(browser::fonts(&window, &fonts).await?);
        Ok(Self { shell: Shell::start(canvas)? })
    }

    /// The document, as `document.json` stores it.
    pub fn json(&self) -> String {
        self.shell.borrow().document().to_json()
    }

    /// Shows a document, given as `document.json` stores it, in place of the one on screen.
    pub fn open(&self, json: &str) -> Result<(), JsValue> {
        let document = Document::from_json(json).map_err(|error| JsError::new(&error.to_string()))?;
        self.shell.borrow_mut().open(document);
        Ok(())
    }

    /// Everything this page knows of the document, for another replica of it to merge.
    pub fn history(&self) -> Vec<u8> {
        self.shell.borrow().history()
    }

    /// Takes in what another replica of the document wrote, as its `history()` gives it.
    pub fn merge(&self, history: &[u8]) -> Result<(), JsValue> {
        self.shell.borrow_mut().merge(history).map_err(|error| JsError::new(&error.to_string()).into())
    }

    /// How many frames have been painted.
    #[wasm_bindgen(getter)]
    pub fn frames(&self) -> u32 {
        self.shell.borrow().stats().frames
    }

    /// When the first frame was on the canvas, in milliseconds since the page started loading.
    #[wasm_bindgen(getter, js_name = firstFrameMs)]
    pub fn first_frame_ms(&self) -> Option<f64> {
        self.shell.borrow().stats().first_frame_ms
    }

    /// How long the latest frame took to paint, in milliseconds.
    #[wasm_bindgen(getter, js_name = frameMs)]
    pub fn frame_ms(&self) -> f64 {
        self.shell.borrow().stats().frame_ms
    }

    /// How long the mirror last took to bring up to date, in milliseconds.
    #[wasm_bindgen(getter, js_name = mirrorMs)]
    pub fn mirror_ms(&self) -> f64 {
        self.shell.borrow().stats().mirror_ms
    }
}
