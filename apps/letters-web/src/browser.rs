//! What the browser lends the editor: a clock, identifiers, the clipboard for as long as a clipboard event
//! lasts, somewhere to keep the person's own words, default style and settings, and the files beside the wasm.

use js_sys::{Promise, Reflect, Uint8Array};
use letters_editor::{Clip, Platform, block_id};
use letters_model::Style;
use letters_ui::{Appearance, Settings};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Crypto, Performance, Response, Storage, Url, Window};

/// Fetches the files at `urls`, all at once. A page that preloads them has them here by the time the wasm
/// is running.
pub async fn files(window: &Window, urls: &[String]) -> Result<Vec<Vec<u8>>, JsValue> {
    let requests: Vec<Promise> = urls.iter().map(|url| window.fetch_with_str(url)).collect();
    let mut files = Vec::new();
    for (request, url) in requests.into_iter().zip(urls) {
        let response: Response = JsFuture::from(request).await?.dyn_into()?;
        if !response.ok() {
            return Err(format!("{url} could not be fetched: {}", response.status()).into());
        }
        let bytes = JsFuture::from(response.array_buffer()?).await?;
        files.push(Uint8Array::new(&bytes).to_vec());
    }
    Ok(files)
}

/// Where this browser keeps the person's own words, a line each, as their dictionary file does elsewhere.
const WORDS: &str = "letters.words";

/// Where this browser keeps the person's default style, as a `style.json`, until an account keeps it.
const STYLE: &str = "letters.style";

/// Where this browser keeps what the person set for themselves, a key to each setting, until an account
/// keeps them. A setting that is not set has no key.
const THEME: &str = "letters.theme";
const ACCENT: &str = "letters.accent";
const NAME: &str = "letters.name";
const LANGUAGE: &str = "letters.language";

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

    /// Whether the browser had `url` from an earlier visit when the page asked for it, as its preload did.
    /// What came from the cache crossed no network, or only as much of one as asking whether it had changed.
    pub fn kept(&self, window: &Window, url: &str) -> bool {
        let page = window.location().href().unwrap_or_default();
        let Ok(resolved) = Url::new_with_base(url, &page) else { return false };
        let Some(entry) = self.performance.get_entries_by_name(&resolved.href()).iter().last() else { return false };
        let size = |name: &str| Reflect::get(&entry, &name.into()).ok().and_then(|size| size.as_f64());
        // A browser that does not say is taken to have fetched it.
        matches!((size("transferSize"), size("encodedBodySize")), (Some(sent), Some(held)) if sent < held)
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

    /// What the person set for themselves in this browser. A key that holds what makes no sense for it is
    /// as if it had never been set.
    pub fn settings(&self) -> Settings {
        let kept = |key| {
            let kept = self.storage.as_ref()?.get_item(key).ok().flatten()?;
            Some(kept.trim().to_owned()).filter(|kept| !kept.is_empty())
        };
        Settings {
            appearance: match kept(THEME).as_deref() {
                Some("light") => Appearance::Light,
                Some("dark") => Appearance::Dark,
                _ => Appearance::System,
            },
            accent: kept(ACCENT).and_then(|accent| colour(&accent)),
            name: kept(NAME),
            language: kept(LANGUAGE),
        }
    }

    /// Keeps `settings` for the next page this browser loads, and for its other tabs. Storage that is full
    /// or turned off keeps nothing, and the settings still hold until the page closes.
    pub fn keep_settings(&self, settings: &Settings) {
        let Some(storage) = &self.storage else { return };
        let theme = match settings.appearance {
            Appearance::System => None,
            Appearance::Light => Some("light".to_owned()),
            Appearance::Dark => Some("dark".to_owned()),
        };
        let accent = settings.accent.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"));
        let keys =
            [(THEME, theme), (ACCENT, accent), (NAME, settings.name.clone()), (LANGUAGE, settings.language.clone())];
        for (key, set) in keys {
            let _ = match set {
                Some(set) => storage.set_item(key, &set),
                None => storage.remove_item(key),
            };
        }
    }

    /// Milliseconds since the page started loading, to a fraction.
    pub fn now(&self) -> f64 {
        self.performance.now()
    }
}

/// Reads `#rrggbb`.
fn colour(hex: &str) -> Option<[u8; 3]> {
    let digits = hex.strip_prefix('#')?;
    // A sign would parse as a number, and is no part of a colour.
    if digits.len() != 6 || !digits.bytes().all(|digit| digit.is_ascii_hexdigit()) {
        return None;
    }
    let rgb = u32::from_str_radix(digits, 16).ok()?;
    Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

impl Platform for Browser {
    fn now_ms(&self) -> u64 {
        self.now() as u64
    }

    // Edits are dated by the browser's clock.
    fn wall_time(&self) -> Option<u64> {
        Some((js_sys::Date::now() / 1000.0) as u64)
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
