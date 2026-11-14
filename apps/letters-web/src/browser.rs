//! What the browser lends the editor: a clock, identifiers, the clipboard for as long as a clipboard event
//! lasts, and the fonts.

use js_sys::{Promise, Uint8Array};
use letters_editor::{Clip, Platform, block_id};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Crypto, Performance, Response, Window};

/// Fetches the font files from under `base`, a URL that ends in a slash, all at once. A page that preloads
/// them has them here by the time the wasm is running.
pub async fn fonts(window: &Window, base: &str) -> Result<Vec<Vec<u8>>, JsValue> {
    let requests: Vec<Promise> =
        letters_fonts::FILES.iter().map(|name| window.fetch_with_str(&format!("{base}{name}"))).collect();
    let mut fonts = Vec::new();
    for (request, name) in requests.into_iter().zip(letters_fonts::FILES) {
        let response: Response = JsFuture::from(request).await?.dyn_into()?;
        if !response.ok() {
            return Err(format!("the font {base}{name} could not be fetched: {}", response.status()).into());
        }
        let bytes = JsFuture::from(response.array_buffer()?).await?;
        fonts.push(Uint8Array::new(&bytes).to_vec());
    }
    Ok(fonts)
}

pub struct Browser {
    performance: Performance,
    crypto: Crypto,
    /// What a `paste` event brought, for the editor to read while it handles that event.
    pub pasted: Option<Clip>,
    /// What the editor copied, for the `copy` or `cut` event that asked for it to carry away.
    pub copied: Option<Clip>,
}

impl Browser {
    pub fn new(window: &Window) -> Result<Self, JsValue> {
        let performance = window.performance().ok_or("this browser has no clock")?;
        Ok(Self { performance, crypto: window.crypto()?, pasted: None, copied: None })
    }

    /// Milliseconds since the page started loading, to a fraction.
    pub fn now(&self) -> f64 {
        self.performance.now()
    }
}

impl Platform for Browser {
    fn now_ms(&self) -> u64 {
        self.now() as u64
    }

    // A page may read the clipboard only inside a paste event, so outside one there is nothing to give.
    fn clipboard_read(&mut self) -> Option<Clip> {
        self.pasted.clone()
    }

    fn clipboard_write(&mut self, clip: Clip) {
        self.copied = Some(clip);
    }

    fn new_id(&mut self) -> String {
        let mut bytes = [0; 10];
        self.crypto.get_random_values_with_u8_array(&mut bytes).expect("the browser has a source of randomness");
        block_id(bytes)
    }
}
