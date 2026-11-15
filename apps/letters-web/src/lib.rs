//! The Letters browser shell: the shared engine painting on a canvas.
//!
//! - [`shell`] owns the canvas, the hidden input beside it and the frame loop.
//! - [`input`] turns the hidden input's events into the editor's input, and [`browser`] lends the editor
//!   the browser's clock, identifiers and clipboard.
//! - [`faces`] says which face backs up the fonts for a character they lack.
//! - [`mirror`] keeps the window readable as HTML beside the canvas, for assistive technology.
//! - [`page`] is the page a server sends: the document as HTML to read at once, and for the editor to open.
//! - [`Letters`] is what a page holds: `await Letters.start(canvas, base)` starts the editor.
//!
//! Frames are rasterised on the CPU, so the page needs no WebGPU. The fonts are fetched beside the wasm and
//! are no part of it, so a new build of the code leaves them in the browser's cache. The dictionaries, and
//! the maths faces of a document without maths, are fetched after the first frame, which has no use for them.

pub mod browser;
pub mod faces;
pub mod input;
pub mod mirror;
pub mod page;
pub mod shell;

use std::cell::RefCell;
use std::rc::Rc;

use letters_model::Document;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::browser::Browser;
use crate::shell::{Later, Shell};

/// The editor on a page.
#[wasm_bindgen]
pub struct Letters {
    shell: Rc<RefCell<Shell>>,
}

#[wasm_bindgen]
impl Letters {
    /// Starts the editor in `canvas`, whose size the page's CSS decides. The files it fetches are found
    /// under `base`, a URL that is empty or ends in a slash: `fonts/`, `maths/` and `dictionaries/`.
    ///
    /// The first frame waits for the first font, which sets most text, and for the maths faces only when
    /// the page's document holds maths. The other fonts are set in as they arrive, and the rest is fetched
    /// once that frame is on the canvas. A browser that kept the fonts from an earlier visit has them all
    /// at once, and its first frame is drawn with all of them.
    pub async fn start(canvas: HtmlCanvasElement, base: String) -> Result<Letters, JsValue> {
        std::panic::set_hook(Box::new(|panic| web_sys::console::error_1(&panic.to_string().into())));
        let window = web_sys::window().ok_or("there is no window")?;
        let mut platform = Browser::new(&window)?;
        let opened = shell::carried(&window, &mut platform);
        let server = shell::served(&window);
        let maths = page::has_maths(&opened);
        let fonts = shell::urls(&base, "fonts", &letters_fonts::FILES);
        let first = if platform.kept(&window, &fonts[0]) { fonts.len() } else { 1 };
        let mut urls = fonts[..first].to_vec();
        if maths {
            urls.extend(shell::urls(&base, "maths", &letters_math::FONT_FILES));
        }
        let mut files = browser::files(&window, &urls).await?;
        let faces = files.split_off(first);
        letters_fonts::provide(files);
        if maths {
            letters_math::provide_fonts(faces);
        }
        let later = Later { base, fonts: fonts[first..].to_vec(), maths: !maths };
        Ok(Self { shell: Shell::start(canvas, platform, opened, server, later)? })
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

    /// Whether the dictionaries have arrived, so that misspellings are marked.
    #[wasm_bindgen(getter)]
    pub fn spelling(&self) -> bool {
        self.shell.borrow().stats().spelling
    }

    /// Whether every font has arrived, so that no text stands in another's face.
    #[wasm_bindgen(getter)]
    pub fn fonts(&self) -> bool {
        self.shell.borrow().stats().fonts
    }

    /// How long the mirror last took to bring up to date, in milliseconds.
    #[wasm_bindgen(getter, js_name = mirrorMs)]
    pub fn mirror_ms(&self) -> f64 {
        self.shell.borrow().stats().mirror_ms
    }
}
