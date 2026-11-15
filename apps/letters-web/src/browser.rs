//! What the browser lends the editor: a clock, identifiers, the clipboard for as long as a clipboard event
//! lasts, somewhere to keep the person's own words and default style, and the fonts.

use js_sys::{Promise, Uint8Array};
use letters_editor::{Clip, Platform, block_id};
use letters_model::Style;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Crypto, Performance, Response, Storage, Window};

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

/// Where this browser keeps the person's own words, a line each, as their dictionary file does elsewhere.
const WORDS: &str = "letters.words";

/// Where this browser keeps the person's default style, as a `style.json`, until an account keeps it.
const STYLE: &str = "letters.style";

pub struct Browser {
    performance: Performance,
    crypto: Crypto,
    /// `None` where the person has turned the page's storage off.
    storage: Option<Storage>,
    /// What a `paste` event brought, for the editor to read while it handles that event.
    pub pasted: Option<Clip>,
    /// What the editor copied, for the `copy` or `cut` event that asked for it to carry away.
    pub copied: Option<Clip>,
}

impl Browser {
    pub fn new(window: &Window) -> Result<Self, JsValue> {
        let performance = window.performance().ok_or("this browser has no clock")?;
        let storage = window.local_storage().ok().flatten();
        Ok(Self { performance, crypto: window.crypto()?, storage, pasted: None, copied: None })
    }

    /// The words the person has added to their own dictionary in this browser.
    pub fn user_words(&self) -> Vec<String> {
        let kept = self.storage.as_ref().and_then(|storage| storage.get_item(WORDS).ok().flatten());
        kept.unwrap_or_default().lines().map(str::to_owned).collect()
    }

    /// The style the person's new documents start with: the one they kept in this browser, else the
    /// built-in one. `None` where the browser keeps nothing, so there is no default to change.
    pub fn default_style(&self) -> Option<Style> {
        let kept = self.storage.as_ref()?.get_item(STYLE).ok().flatten();
        Some(kept.and_then(|json| Style::from_json(&json).ok()).unwrap_or_default())
    }

    /// Keeps `json`, a `style.json`, as the person's default style. `false` when it could not be kept.
    pub fn keep_default_style(&self, json: &str) -> bool {
        self.storage.as_ref().is_some_and(|storage| storage.set_item(STYLE, json).is_ok())
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

    fn learn_word(&mut self, word: &str) {
        let mut words = self.user_words();
        if words.iter().any(|known| known == word) {
            return;
        }
        words.push(word.to_owned());
        // Storage that is full or turned off keeps nothing, and the word still holds until the page closes.
        if let Some(storage) = &self.storage {
            let _ = storage.set_item(WORDS, &words.join("\n"));
        }
    }
}
