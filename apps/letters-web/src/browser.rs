//! What the browser lends the editor: a clock, identifiers, the clipboard for as long as a clipboard event
//! lasts, somewhere to keep the person's own words, default style and settings, the files beside the wasm, and
//! its decoders for pictures.

use std::collections::HashMap;

use js_sys::{Array, Promise, Reflect, Uint8Array};
use letters_editor::{Clip, Picture, Platform, block_id};
use letters_model::Style;
use letters_render::image::Image;
use letters_ui::{Appearance, Settings};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, BlobPropertyBag, CanvasRenderingContext2d, Crypto, HtmlCanvasElement, ImageBitmap, Performance, Response,
    Storage, Url, Window,
};

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

/// The longest side a picture is drawn from, in pixels. A photo straight from a camera is several times
/// what any screen shows of it, and its pixels would be held twice over while they are handed across.
const LONGEST: f64 = 4096.0;

/// The media type of a picture in one of the formats a document keeps, told by its first bytes.
fn mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("image/webp"),
        _ => None,
    }
}

/// Has the browser decode a picture: upright, as its EXIF orientation says.
async fn bitmap(window: &Window, picture: &Blob) -> Option<ImageBitmap> {
    let decoding = window.create_image_bitmap_with_blob(picture).ok()?;
    JsFuture::from(decoding).await.ok()?.dyn_into().ok()
}

/// A picture from its file, as the person pasted, dropped or chose it. `None` for what is no picture
/// Letters can show.
pub async fn picture(window: &Window, file: &Blob) -> Option<Picture> {
    let bytes = Uint8Array::new(&JsFuture::from(file.array_buffer()).await.ok()?).to_vec();
    let mime = mime(&bytes)?.to_owned();
    // The size it is shown at is its upright one, which only decoding it tells.
    let decoded = bitmap(window, file).await?;
    let (width, height) = (decoded.width(), decoded.height());
    decoded.close();
    Some(Picture { bytes, mime, width, height })
}

/// The pixels of a picture the document keeps, decoded by the browser, which has the decoders already and
/// runs them off this thread.
pub async fn decode(window: &Window, bytes: &[u8]) -> Option<Image> {
    let kind = BlobPropertyBag::new();
    kind.set_type(mime(bytes)?);
    let file = Blob::new_with_u8_array_sequence_and_options(&Array::of1(&Uint8Array::from(bytes)), &kind).ok()?;
    let decoded = bitmap(window, &file).await?;
    let (width, height) = (f64::from(decoded.width()), f64::from(decoded.height()));
    let scale = (LONGEST / width.max(height)).min(1.0);
    let (width, height) = ((width * scale).round().max(1.0), (height * scale).round().max(1.0));
    // A canvas is the one place a page can read decoded pixels from.
    let canvas: HtmlCanvasElement = window.document()?.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(width as u32);
    canvas.set_height(height as u32);
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    context.draw_image_with_image_bitmap_and_dw_and_dh(&decoded, 0.0, 0.0, width, height).ok()?;
    decoded.close();
    let pixels = context.get_image_data(0.0, 0.0, width, height).ok()?;
    Image::from_rgba8(pixels.width(), pixels.height(), pixels.data().0).ok()
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
    /// The pictures put into the document here, by the names it has for them. They are kept until there
    /// is a server to keep them.
    pub assets: HashMap<String, Vec<u8>>,
}

/// Puts text on the clipboard outside a clipboard event, as a press on a copy button asks. The browser
/// allows it only in the wake of a press or a key, and not at all on a page served over plain HTTP.
pub fn copy(window: &Window, text: &str) -> Result<(), JsValue> {
    // Read by name: the binding to the async clipboard is not in every release of `web-sys`.
    let clipboard = Reflect::get(&window.navigator(), &"clipboard".into())?;
    if clipboard.is_undefined() {
        return Err("this page may not write to the clipboard".into());
    }
    let write: js_sys::Function = Reflect::get(&clipboard, &"writeText".into())?.dyn_into()?;
    let written: Promise = write.call1(&clipboard, &text.into())?.dyn_into()?;
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(refused) = JsFuture::from(written).await {
            web_sys::console::error_1(&refused);
        }
    });
    Ok(())
}

impl Browser {
    pub fn new(window: &Window) -> Result<Self, JsValue> {
        let performance = window.performance().ok_or("this browser has no clock")?;
        let storage = window.local_storage().ok().flatten();
        Ok(Self { performance, crypto: window.crypto()?, storage, pasted: None, copied: None, assets: HashMap::new() })
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

    fn store_asset(&mut self, name: &str, bytes: &[u8]) {
        self.assets.entry(name.to_owned()).or_insert_with(|| bytes.to_vec());
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
